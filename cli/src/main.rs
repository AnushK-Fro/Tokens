mod agents;
mod event;
mod pricing;
mod push;
mod report;

use std::time::Duration;

use anyhow::{bail, Context, Result};
use chrono::NaiveDate;
use chrono_tz::Tz;
use clap::{Args, Parser, Subcommand};
use rayon::prelude::*;

use event::{Event, AGENTS};

/// Token usage for Claude Code, Codex, Kimi and OpenCode, read from their local logs.
#[derive(Parser)]
#[command(version, about)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
    #[command(flatten)]
    daily: DailyArgs,
}

#[derive(Subcommand)]
enum Command {
    /// Print usage per day (the default).
    Daily(DailyArgs),
    /// Send usage to a Tokens server.
    Push(PushArgs),
}

#[derive(Args, Clone, Default)]
struct Common {
    /// Only these agents (claude, codex, kimi, opencode); repeatable.
    #[arg(short, long = "agent", value_name = "AGENT", global = true)]
    agents: Vec<String>,
    /// Use the cached price list without checking for a newer one.
    #[arg(long, global = true)]
    offline: bool,
}

#[derive(Args, Clone, Default)]
struct DailyArgs {
    #[command(flatten)]
    common: Common,
    /// First day to show (YYYY-MM-DD or YYYYMMDD).
    #[arg(short, long)]
    since: Option<String>,
    /// Last day to show (inclusive).
    #[arg(short, long)]
    until: Option<String>,
    /// Print JSON instead of a table.
    #[arg(short, long)]
    json: bool,
    /// Add a row per model under each day.
    #[arg(short, long)]
    breakdown: bool,
    /// Timezone for grouping days (default: this machine's).
    #[arg(short = 'z', long)]
    timezone: Option<String>,
}

#[derive(Args)]
struct PushArgs {
    #[command(flatten)]
    common: Common,
    /// Server URL [env: TOKENS_URL or TOKEN_USAGE_URL]
    #[arg(long)]
    url: Option<String>,
    /// Ingest token [env: TOKENS_TOKEN or TOKEN_USAGE_API_KEY]
    #[arg(long)]
    token: Option<String>,
    /// Label for this machine, shown as <name>@<hostname>:<agent> [env: TOKEN_USAGE_SOURCE_NAME]
    #[arg(long)]
    name: Option<String>,
    /// Project label [env: TOKEN_USAGE_PROJECT]
    #[arg(long)]
    project: Option<String>,
    /// Timezone for grouping days; should match the server's [env: TOKEN_USAGE_TIMEZONE, default UTC]
    #[arg(short = 'z', long)]
    timezone: Option<String>,
    /// Send every request again, not just those since the last push.
    #[arg(long)]
    all: bool,
    /// Keep running and push every this many seconds.
    #[arg(short, long, value_name = "SECONDS")]
    watch: Option<u64>,
    /// Show what would be sent without sending it.
    #[arg(long)]
    dry_run: bool,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Some(Command::Push(args)) => push(args),
        Some(Command::Daily(args)) => daily(args),
        None => daily(cli.daily),
    }
}

fn daily(args: DailyArgs) -> Result<()> {
    let timezone = match &args.timezone {
        Some(name) => parse_timezone(name)?,
        None => iana_time_zone::get_timezone().ok().and_then(|name| name.parse().ok()).unwrap_or(Tz::UTC),
    };
    let since = args.since.as_deref().map(parse_day).transpose()?;
    let until = args.until.as_deref().map(parse_day).transpose()?;

    let events = load(&agents(&args.common)?, args.common.offline);
    let mut daily = report::daily(&events, timezone);
    daily.retain(|day, _| since.is_none_or(|s| *day >= s) && until.is_none_or(|u| *day <= u));

    if args.json {
        println!("{}", serde_json::to_string_pretty(&report::to_json(&daily))?);
    } else if daily.is_empty() {
        println!("No usage found.");
    } else {
        report::print_table(&daily, args.breakdown);
    }
    Ok(())
}

fn push(args: PushArgs) -> Result<()> {
    let url = args.url.or_else(|| env(&["TOKENS_URL", "TOKEN_USAGE_URL"])).context("no server URL: pass --url or set TOKENS_URL")?;
    let token = args.token.or_else(|| env(&["TOKENS_TOKEN", "TOKEN_USAGE_API_KEY"])).unwrap_or_default();
    let name = args.name.or_else(|| env(&["TOKEN_USAGE_SOURCE_NAME"]));
    let project = args.project.or_else(|| env(&["TOKEN_USAGE_PROJECT"]));
    let target = push::Target::new(&url, token, name, project)?;
    let timezone = parse_timezone(&args.timezone.or_else(|| env(&["TOKEN_USAGE_TIMEZONE"])).unwrap_or_else(|| "UTC".to_owned()))?;
    let agents = agents(&args.common)?;

    loop {
        let events = load(&agents, args.common.offline);
        let daily = report::daily(&events, timezone);
        let result = push::push(&target, &events, &daily, &agents, args.all, args.dry_run);

        match (args.watch, result) {
            (None, result) => return result,
            (Some(seconds), result) => {
                if let Err(error) = result {
                    eprintln!("error: {error:#}");
                }
                std::thread::sleep(Duration::from_secs(seconds.max(15)));
            }
        }
    }
}

/// Reads every agent's logs (in parallel) and prices each request.
fn load(agents: &[&'static str], offline: bool) -> Vec<Event> {
    let mut prices = pricing::Prices::load(offline);
    let mut events: Vec<Event> = agents.par_iter().flat_map(|agent| agents::load(agent)).collect();
    prices.apply(&mut events);
    events
}

fn agents(common: &Common) -> Result<Vec<&'static str>> {
    if common.agents.is_empty() {
        return Ok(AGENTS.to_vec());
    }
    common
        .agents
        .iter()
        .map(|name| {
            let name = name.trim().to_ascii_lowercase();
            AGENTS.iter().copied().find(|agent| *agent == name).with_context(|| format!("unknown agent `{name}` (use claude, codex, kimi or opencode)"))
        })
        .collect()
}

fn env(names: &[&str]) -> Option<String> {
    names.iter().find_map(|name| std::env::var(name).ok().map(|v| v.trim().to_owned()).filter(|v| !v.is_empty()))
}

fn parse_timezone(name: &str) -> Result<Tz> {
    match name.parse::<Tz>() {
        Ok(tz) => Ok(tz),
        Err(_) => bail!("unknown timezone `{name}` (use an IANA name such as UTC or Europe/London)"),
    }
}

fn parse_day(value: &str) -> Result<NaiveDate> {
    NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .or_else(|_| NaiveDate::parse_from_str(value, "%Y%m%d"))
        .with_context(|| format!("`{value}` is not a date (YYYY-MM-DD or YYYYMMDD)"))
}
