mod config;
mod db;
mod error;
mod import;
mod import_cursor;
mod import_payments;
mod ingest;
mod routes;
mod state;
mod store;
mod usage;

use std::path::PathBuf;

use anyhow::{bail, Context};
use tracing_subscriber::EnvFilter;

use crate::config::Config;
use crate::state::AppState;

const USAGE: &str = "\
usage: tokens [command]

commands:
  serve                  run the web app (default)
  migrate                apply database migrations and exit
  import-sqlite <path>   merge a token-usage.sqlite from the PHP version into MariaDB
                         (monotonic: never lowers or removes anything already stored)
  import-cursor-csv <path> [--source <id>]
                         merge a Cursor usage export (usage-events-*.csv) as agent `cursor`,
                         summed per day and model; same monotonic merge, safe to re-run.
                         --source defaults to `cursor-usage-export`; use one id per account
  import-payments <path> merge a payments CSV (what was actually paid; see README \"Spending\");
                         safe to re-run, never deletes";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let _ = dotenvy::dotenv();
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("tokens=info,tower_http=info")))
        .init();

    let mut args = std::env::args().skip(1);
    let command = args.next().unwrap_or_else(|| "serve".to_owned());

    if matches!(command.as_str(), "-h" | "--help" | "help") {
        println!("{USAGE}");
        return Ok(());
    }

    let config = Config::from_env()?;
    let pool = db::connect(&config.database_url).await?;

    match command.as_str() {
        "serve" => serve(config, pool).await,
        "migrate" => {
            tracing::info!("migrations are up to date");
            Ok(())
        }
        "import-sqlite" => {
            let path = args.next().map(PathBuf::from).context("import-sqlite needs a path\n\n".to_owned() + USAGE)?;
            let summary = import::from_sqlite(&pool, &path).await?;
            println!("{summary}");
            Ok(())
        }
        "import-cursor-csv" => {
            let mut path = None;
            let mut source_id = import_cursor::DEFAULT_SOURCE_ID.to_owned();
            while let Some(arg) = args.next() {
                match arg.as_str() {
                    "--source" => source_id = args.next().context("--source needs an id")?,
                    _ if path.is_none() => path = Some(PathBuf::from(arg)),
                    other => bail!("unexpected argument `{other}`\n\n{USAGE}"),
                }
            }
            let path = path.context("import-cursor-csv needs a path\n\n".to_owned() + USAGE)?;
            let summary = import_cursor::from_csv(&pool, &path, &source_id, config.timezone).await?;
            println!("{summary}");
            Ok(())
        }
        "import-payments" => {
            let path = args.next().map(PathBuf::from).context("import-payments needs a path\n\n".to_owned() + USAGE)?;
            let summary = import_payments::from_csv(&pool, &path).await?;
            println!("{summary}");
            Ok(())
        }
        other => bail!("unknown command `{other}`\n\n{USAGE}"),
    }
}

async fn serve(config: Config, pool: sqlx::MySqlPool) -> anyhow::Result<()> {
    let bind = config.bind;

    if config.ingest_token.is_none() {
        tracing::warn!("INGEST_TOKEN is not set; ingest is disabled");
    }

    let app = routes::router(AppState::new(pool, config));
    let listener = tokio::net::TcpListener::bind(bind).await.with_context(|| format!("binding {bind}"))?;
    tracing::info!("listening on http://{bind}");

    axum::serve(listener, app).with_graceful_shutdown(shutdown_signal()).await?;
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };

    #[cfg(unix)]
    let terminate = async {
        if let Ok(mut signal) = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            signal.recv().await;
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }
}
