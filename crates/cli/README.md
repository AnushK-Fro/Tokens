# tokens-cli

Reads Claude Code, Codex, Kimi and OpenCode usage from their local logs and prints it per day,
like ccusage. `push` sends it to a Tokens server.

Download a build from the repository's Actions or Releases page (Linux and Windows), or build it:

```sh
cargo build --release -p tokens-cli   # target/release/tokens-cli
```

## Use

```sh
tokens-cli                         # daily table, all agents
tokens-cli -a claude -b            # one agent, a row per model
tokens-cli --since 2026-11-01 --json

tokens-cli push --url https://tokens.example.com --token <INGEST_TOKEN>
tokens-cli push --watch 60         # keep pushing every minute
```

`push` reads `TOKENS_URL` and `TOKENS_TOKEN` if the flags are left out. It is safe to run as often
as you like: the server only keeps the highest numbers it has seen.

Prices come from LiteLLM's public list (cached for a day). The table groups days in this
machine's timezone; `push` uses UTC unless `--timezone` says otherwise.
