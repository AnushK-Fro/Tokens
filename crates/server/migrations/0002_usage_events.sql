-- One row per model request, as sent by tokens-cli: the finest grain the agents' logs have.
-- The dashboard reads daily_usage; this table keeps everything else for later analysis
-- (timestamps, sessions, projects, cache tiers, the agent's raw usage object). Prompts and
-- replies are never sent.
--
-- Same rules as daily_usage: rows are never deleted and counts never go down. An event can be
-- sent again (every push overlaps the last), and Claude Code logs a message several times while
-- it streams, so a repeat merges by GREATEST() per column.
--
-- event_id is unique per agent, not per source: the same request sent under two source ids is
-- still one row, so nothing here is double counted.

CREATE TABLE IF NOT EXISTS usage_events (
    agent                    VARCHAR(32)     NOT NULL,
    event_id                 VARCHAR(255)    NOT NULL,
    source_id                VARCHAR(128)    NOT NULL,
    occurred_at              DATETIME(3)     NOT NULL,
    model                    VARCHAR(160)    NOT NULL,
    provider                 VARCHAR(64)     NOT NULL DEFAULT '',
    session_id               VARCHAR(255)    NOT NULL DEFAULT '',
    project                  VARCHAR(512)    NOT NULL DEFAULT '',
    input_tokens             BIGINT UNSIGNED NOT NULL DEFAULT 0,
    output_tokens            BIGINT UNSIGNED NOT NULL DEFAULT 0,
    cache_read_tokens        BIGINT UNSIGNED NOT NULL DEFAULT 0,
    cache_creation_tokens    BIGINT UNSIGNED NOT NULL DEFAULT 0,
    cache_creation_5m_tokens BIGINT UNSIGNED NOT NULL DEFAULT 0,
    cache_creation_1h_tokens BIGINT UNSIGNED NOT NULL DEFAULT 0,
    reasoning_tokens         BIGINT UNSIGNED NOT NULL DEFAULT 0,
    total_tokens             BIGINT UNSIGNED NOT NULL DEFAULT 0,
    cost_usd                 DOUBLE          NOT NULL DEFAULT 0,
    -- 'log' (the agent wrote the cost), 'calculated' (LiteLLM prices) or 'none' (no price known).
    cost_source              VARCHAR(16)     NOT NULL DEFAULT '',
    -- Everything else the log line had about the request, as JSON.
    details                  LONGTEXT        NULL,
    first_received_at        DATETIME(3)     NOT NULL,
    last_received_at         DATETIME(3)     NOT NULL,
    PRIMARY KEY (agent, event_id),
    KEY usage_events_time (occurred_at),
    KEY usage_events_source (source_id, occurred_at),
    KEY usage_events_session (session_id),
    KEY usage_events_model (model, occurred_at)
) ENGINE = InnoDB DEFAULT CHARSET = utf8mb4 COLLATE = utf8mb4_bin;

CREATE TRIGGER usage_events_never_decrease BEFORE UPDATE ON usage_events FOR EACH ROW
SET NEW.input_tokens             = GREATEST(NEW.input_tokens, OLD.input_tokens),
    NEW.output_tokens            = GREATEST(NEW.output_tokens, OLD.output_tokens),
    NEW.cache_read_tokens        = GREATEST(NEW.cache_read_tokens, OLD.cache_read_tokens),
    NEW.cache_creation_tokens    = GREATEST(NEW.cache_creation_tokens, OLD.cache_creation_tokens),
    NEW.cache_creation_5m_tokens = GREATEST(NEW.cache_creation_5m_tokens, OLD.cache_creation_5m_tokens),
    NEW.cache_creation_1h_tokens = GREATEST(NEW.cache_creation_1h_tokens, OLD.cache_creation_1h_tokens),
    NEW.reasoning_tokens         = GREATEST(NEW.reasoning_tokens, OLD.reasoning_tokens),
    NEW.total_tokens             = GREATEST(NEW.total_tokens, OLD.total_tokens),
    NEW.cost_usd                 = GREATEST(NEW.cost_usd, OLD.cost_usd),
    NEW.first_received_at        = OLD.first_received_at;

CREATE TRIGGER usage_events_never_deleted BEFORE DELETE ON usage_events FOR EACH ROW
SIGNAL SQLSTATE '45000' SET MESSAGE_TEXT = 'usage_events is append-only; rows are never deleted';
