# Tokens

Token usage dashboard for Claude Code, Codex, OpenCode and Kimi, fed by `ccusage`. Rust (axum +
sqlx) on MariaDB; the frontend is plain ES modules compiled into the binary.

## Nothing is ever lost

Claude Code purges old transcripts, so `ccusage` forgets old days and can report them lower or
not at all. This app treats stored usage as **append-only and monotonic**:

- Each (source, day, agent, model) row is merged with a per-column `GREATEST()`. Tokens and cost
  can rise as a day fills in, but no report can lower them. Rows or days missing from a report
  are left alone. `X-Usage-Scope: full` no longer clears anything.
- MariaDB triggers hold the same rule for anything else that touches the table: an `UPDATE`
  cannot lower a value, and `DELETE` on `daily_usage` / `usage_revisions` is refused.
- Every insert or increase is written to `usage_revisions`, and every sync to `ingests` (with
  how many rows ccusage reported lower than what is stored: `rows_protected`).
- The dashboard's **Kept history** tile counts the rows that only this database still has.

## Layout

```
src/
  main.rs            CLI: serve | migrate | import-sqlite <path> | import-cursor-csv <path>
  config.rs          env config (.env)
  db.rs              pool + embedded migrations
  usage.rs           Counts and the monotonic merge rule (unit-tested)
  ingest/report.rs   ccusage JSON → per-day, per-model rows (unit-tested)
  store/ingest.rs    the only writer: record_report()
  store/summary.rs   read-only queries for the dashboard
  routes/            /api, /api/summary, /api/ingest, /api/health, static assets
  import.rs          one-off import of the PHP version's SQLite file
  import_cursor.rs   import of a Cursor usage-events CSV export (unit-tested)
migrations/          schema + triggers
docs/API.md          served at GET /api (Markdown, for agents)
web/                 index.html, css/, js/ (views/ per panel), fonts/ (Geist, Geist Mono; OFL)
```

## Run

```sh
cp .env.example .env      # set DATABASE_URL and INGEST_TOKEN
cargo run --release       # creates tables on start, serves on BIND (default 127.0.0.1:8787)
```

MariaDB setup:

```sql
CREATE DATABASE tokens CHARACTER SET utf8mb4;
CREATE USER 'tokens'@'localhost' IDENTIFIED BY '...';
GRANT ALL ON tokens.* TO 'tokens'@'localhost';
```

If binary logging is on, creating the triggers needs `SET GLOBAL log_bin_trust_function_creators = 1`
(or run the first start as a user with `SUPER`).

## Deploying with Docker

`docker-compose.yml` runs MariaDB, the app, and Caddy (automatic HTTPS for `TOKENS_HOST`):

```sh
cp .env.example .env      # set TOKENS_HOST (e.g. tokens.anush.me), DB_PASSWORD, INGEST_TOKEN
docker compose up -d --build
```

Point `TOKENS_HOST`'s DNS at the machine and open ports 80 and 443 (Caddy needs both for the
certificate). If they're taken, set `HTTP_PORT` / `HTTPS_PORT`. Sync clients then send to
`https://<TOKENS_HOST>/api/ingest` with the same `INGEST_TOKEN`.

**Seed.** `seed/tokens.sql` is a full dump (schema, triggers, every row) that is built into the
`db` image and loaded on the first start with an empty `db` volume, so a new deployment starts
with all usage recorded so far. It is not committed (it names machines and projects): create it
with `scripts/refresh-seed.sh`, which dumps the database in `.env`, or copy it from an existing
deployment. Without it the database starts empty. Later starts keep what the volume holds. To re-seed an existing deployment,
`docker compose down -v` removes the volume first; that drops anything recorded only there.

Imports run the binary in a one-off container, e.g. a Cursor export:

```sh
docker compose run --rm -v "$PWD/usage-events.csv:/import.csv:ro" tokens import-cursor-csv /import.csv --source cursor-usage-export
```

## Moving from tokens.php

```sh
cargo run --release -- import-sqlite /path/to/token-usage.sqlite
```

It goes through the same merge, so it is safe before or after syncs start arriving, and re-running
it changes nothing. Old sync clients keep working: a `POST` to `/`, `/index.php` or `/tokens.php`
(with `?api=ingest` or no query) is still accepted as an ingest.

## Importing Cursor usage

Cursor isn't covered by `ccusage`. Export usage from cursor.com/dashboard (Usage → Export CSV) and:

```sh
cargo run --release -- import-cursor-csv "usage-events-2026-11-10.csv" [--source <id>]
```

Requests are summed per day (in `TOKEN_USAGE_TIMEZONE`) and model under agent `cursor`, then go
through the same monotonic merge, so overlapping or repeated exports are safe and a newer export
only raises the days it extends. `Input (w/ Cache Write)` is stored as cache-creation tokens,
`Input (w/o Cache Write)` as input. Plan usage (`Included` / `Free`) has no dollar figure, so each
request is priced at Cursor's published API rates (`rate()` in `src/import_cursor.rs`, from
cursor.com/docs/models): Grok 4.7 prompts over 256k tokens bill the whole request at 2×; Grok Bot,
which has no published rate, is estimated at Grok 4.7; `auto` stays at $0. Costs only ever rise, so
re-importing after a rate change can raise a stored cost but never lower it. Requests marked
`Errored, No Charge` / `Aborted, Not Charged` keep their tokens at $0.

Use one `--source` id per Cursor account and keep it for every later export of that account:
sources are summed, so accounts stack, while the same account under two ids would count twice.
In use: `cursor-usage-export` (default) and `cursor-usage-export-2`.

## Syncing

The devcontainer's `.devcontainer/token-usage-sync.py` is the client: it runs
`ccusage daily --json --by-agent`, splits the days per agent and POSTs each to `TOKEN_USAGE_URL`
every `TOKEN_USAGE_INTERVAL` seconds. It needs no changes.

Each report must cover one agent. Plain `ccusage daily --json` in v20+ merges every agent into
one report (`"agent": "all"`) that can't be split back apart, so the server rejects it with `422`.
Keep one source id per machine and agent: totals are summed across sources, so the same usage
sent under two ids counts twice. Syncing is idempotent, and the more often it runs, the less a
purge can take before it has been recorded.

## Tests

```sh
cargo test
```
