# Token usage API

Base URL: `{{BASE_URL}}`

Daily token usage and API-equivalent cost for coding agents (Claude Code, Codex, OpenCode, Kimi),
collected from `ccusage` reports. Stored usage is **append-only and monotonic**: a value can rise as
a day fills in, but no report can lower or delete it. When ccusage forgets old days (Claude Code
purges old transcripts), what was already recorded stays.

All responses are JSON unless noted. Dates are `YYYY-MM-DD`; timestamps are RFC 3339 UTC.

---

## GET /api

This document, as Markdown.

## GET /api/health

`200 {"ok": true}` when the database answers, `503 {"ok": false}` otherwise.

## GET /api/summary

Public, no key. Every day × agent × model ever recorded, summed across sources.

```json
{
  "generated_at": "2026-11-10T02:30:00Z",
  "timezone": "UTC",
  "today": "2026-11-10",
  "source_count": 3,
  "last_sync_at": "2026-11-10T02:29:41Z",
  "retention": {
    "preserved_rows": 412,
    "preserved_cost_usd": 1830.55,
    "protected_rows": 0
  },
  "daily": [
    {
      "day": "2026-11-09",
      "agent": "claude",
      "model": "claude-opus-5-5",
      "input_tokens": 12034,
      "output_tokens": 98012,
      "cache_read_tokens": 41200331,
      "cache_creation_tokens": 1820044,
      "reasoning_tokens": 0,
      "total_tokens": 43130421,
      "cost_usd": 61.42
    }
  ]
}
```

Field notes:

- `model` drops any `provider/` or `org/` prefix (`deepseek/deepseek-v4.1-flash` →
  `deepseek-v4.1-flash`), and rows that then share a name are summed. Storage keeps the name as
  reported.
- `today` is in the server's `timezone`; use it rather than the client clock for "today" figures.
- `input_tokens` is **uncached** input for every agent. Older Codex reports counted cached reads
  inside input; those are separated here so cached reads appear only in `cache_read_tokens`.
- `reasoning_tokens` is a subset of `output_tokens`, not an extra category.
- `total_tokens` is ccusage's own total for the row.
- `cost_usd` is ccusage's API-price estimate, not a subscription charge.
- `retention.preserved_rows` / `preserved_cost_usd`: stored rows older than anything in the latest
  report from their source, i.e. history ccusage no longer has and only this database keeps.
- `retention.protected_rows`: rows in the latest reports where ccusage gave a lower figure than
  the stored one; the stored figure was kept.
- Source names and projects are private and never returned.

Useful derived numbers: cache hit rate = `cache_read / (input + cache_read + cache_creation)`;
cost per million = `cost_usd / total_tokens * 1e6`.

## POST /api/ingest

Requires `Authorization: Bearer <INGEST_TOKEN>` (the key lives in the server's `.env`).

Body: the unmodified output of a ccusage daily report, for example

```sh
npx ccusage@latest claude daily --json     # Claude Code
npx ccusage@latest codex daily --json      # Codex
npx ccusage@latest opencode daily --json   # OpenCode
npx ccusage@latest kimi daily --json       # Kimi CLI
```

Send **one agent per request**. Plain `ccusage daily --json` (v20+) merges every agent into one
report (`"agent": "all"`) and its rows can't be split back apart; it is rejected with `422`.

Headers:

| Header                 | Required | Notes                                                             |
|------------------------|----------|-------------------------------------------------------------------|
| `Authorization`        | yes      | `Bearer <INGEST_TOKEN>`                                           |
| `Content-Type`         | yes      | `application/json`                                                |
| `X-Usage-Source-Id`    | yes      | Stable id for this machine/login, `[A-Za-z0-9._:@+-]`, ≤ 128 chars |
| `X-Usage-Agent`        | yes      | `claude`, `codex`, `opencode`, `kimi`, … ≤ 32 chars                |
| `X-Usage-Source-Name`  | no       | Label, private                                                    |
| `X-Usage-Project`      | no       | Label, private                                                    |
| `X-Usage-Scope`        | no       | Accepted and ignored. `full` no longer clears anything.           |

Response:

```json
{ "ok": true, "rows": 120, "inserted": 1, "increased": 2, "unchanged": 117, "protected": 0, "observed_at": "2026-11-10T02:29:41Z" }
```

- `inserted`: new day/model rows. `increased`: rows where at least one column went up.
  `unchanged`: everything else (includes `protected` rows).
- `protected`: rows where the report was lower than what is stored; the stored values were kept.

Each row is merged by taking the maximum of every column (tokens and cost) against what is stored.
Rows or days missing from the report are left alone. Re-sending the same report is a no-op, so
syncing often (e.g. every few minutes) is safe.

Errors: `401` bad or missing key, `413` body over 10 MB, `422` invalid JSON, headers, or not a
daily report (`{"error": "..."}`).

Example:

```sh
npx ccusage@latest claude daily --json | curl -fsS -X POST '{{BASE_URL}}/api/ingest' \
  -H "Authorization: Bearer $INGEST_TOKEN" \
  -H 'Content-Type: application/json' \
  -H 'X-Usage-Source-Id: laptop-claude' \
  -H 'X-Usage-Agent: claude' \
  --data-binary @-
```

## POST /api/events

Per-request usage from `tokens-cli`, kept for analysis (the dashboard reads the daily rows). Same
`Authorization` and `X-Usage-*` headers as `/api/ingest`.

Body: `{"events": [...]}`, at most 5000 per request.

```json
{
  "events": [
    {
      "id": "msg_011Cgt…:req_011Cgt…",
      "timestamp": "2026-11-10T04:55:38.854Z",
      "model": "claude-opus-5-5",
      "provider": "anthropic",
      "session_id": "09123004-bc99-400d-8693-5f1b4dd8faf6",
      "project": "/workspaces/AdvinServers-Panels/Convoy",
      "input_tokens": 2,
      "output_tokens": 767,
      "cache_read_tokens": 132547,
      "cache_creation_tokens": 808,
      "cache_creation_5m_tokens": 0,
      "cache_creation_1h_tokens": 808,
      "reasoning_tokens": 0,
      "total_tokens": 134124,
      "cost_usd": 0.0476,
      "cost_source": "calculated",
      "details": { "version": "2.1.322", "usage": { "service_tier": "standard" } }
    }
  ]
}
```

- `id` is unique per agent (Claude: message id + request id; Codex: response id; Kimi and older
  Codex: file and line). The same request sent again, from any source, is one row.
- `cost_source`: `log` (the agent wrote the cost), `calculated` (LiteLLM prices) or `none`.
- `details` is everything else the log line had about the request, never prompts or replies.
- Rows are stored in `usage_events` and merge like daily rows: per-column maximum, never deleted.

Response: `{"ok": true, "events": 1000, "written": 12}` (`written` is MariaDB's affected-row
count: 1 per new row, 2 per raised row).

## Legacy

The PHP version's entry points still work on `/`, `/index.php` and `/tokens.php`: a `POST` with
`?api=ingest` or no query at all is an ingest, and `?api=health|summary` answer as above, so old
sync scripts need no change. `?api=summary` returns the shape above, not the PHP one.
