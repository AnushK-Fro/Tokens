-- Usage is append-only and monotonic: a stored value may grow, never shrink, and no row is ever
-- deleted. ccusage only sees what is still on disk, and Claude Code purges old transcripts, so a
-- later report can be smaller than an earlier one. The application merges with GREATEST() per
-- column; the triggers below hold the same line for anything else that touches the table.

CREATE TABLE IF NOT EXISTS sources (
    id            VARCHAR(128) NOT NULL,
    name          VARCHAR(160) NOT NULL,
    agent         VARCHAR(32)  NOT NULL,
    project       VARCHAR(160) NOT NULL DEFAULT '',
    first_seen_at DATETIME(3)  NOT NULL,
    last_seen_at  DATETIME(3)  NOT NULL,
    PRIMARY KEY (id)
) ENGINE = InnoDB DEFAULT CHARSET = utf8mb4 COLLATE = utf8mb4_bin;

CREATE TABLE IF NOT EXISTS daily_usage (
    source_id             VARCHAR(128)    NOT NULL,
    day                   DATE            NOT NULL,
    agent                 VARCHAR(32)     NOT NULL,
    model                 VARCHAR(160)    NOT NULL,
    input_tokens          BIGINT UNSIGNED NOT NULL DEFAULT 0,
    output_tokens         BIGINT UNSIGNED NOT NULL DEFAULT 0,
    cache_read_tokens     BIGINT UNSIGNED NOT NULL DEFAULT 0,
    cache_creation_tokens BIGINT UNSIGNED NOT NULL DEFAULT 0,
    reasoning_tokens      BIGINT UNSIGNED NOT NULL DEFAULT 0,
    total_tokens          BIGINT UNSIGNED NOT NULL DEFAULT 0,
    cost_usd              DOUBLE          NOT NULL DEFAULT 0,
    first_observed_at     DATETIME(3)     NOT NULL,
    last_changed_at       DATETIME(3)     NOT NULL,
    last_observed_at      DATETIME(3)     NOT NULL,
    PRIMARY KEY (source_id, day, agent, model),
    KEY daily_usage_day (day),
    CONSTRAINT daily_usage_source FOREIGN KEY (source_id) REFERENCES sources (id) ON DELETE RESTRICT
) ENGINE = InnoDB DEFAULT CHARSET = utf8mb4 COLLATE = utf8mb4_bin;

-- One row per accepted report, so it is visible when a sync arrived and what it did.
CREATE TABLE IF NOT EXISTS ingests (
    id             BIGINT UNSIGNED NOT NULL AUTO_INCREMENT,
    source_id      VARCHAR(128)    NOT NULL,
    agent          VARCHAR(32)     NOT NULL,
    received_at    DATETIME(3)     NOT NULL,
    rows_received  INT UNSIGNED    NOT NULL,
    rows_inserted  INT UNSIGNED    NOT NULL,
    rows_increased INT UNSIGNED    NOT NULL,
    rows_unchanged INT UNSIGNED    NOT NULL,
    -- Rows where the report was lower than what is stored in at least one column; the stored
    -- value was kept. A steady stream of these is ccusage forgetting purged history.
    rows_protected INT UNSIGNED    NOT NULL,
    -- Oldest and newest day in the report. Stored days before first_day are ones ccusage no
    -- longer knows about, which is exactly what this database exists to keep.
    first_day      DATE            NULL,
    last_day       DATE            NULL,
    PRIMARY KEY (id),
    KEY ingests_received (received_at),
    KEY ingests_source (source_id, agent, id)
) ENGINE = InnoDB DEFAULT CHARSET = utf8mb4 COLLATE = utf8mb4_bin;

-- Every insert or increase of a daily_usage row, as it stood afterwards. The full history of a
-- row is its revisions in id order, so any value can be traced back to the sync that set it.
CREATE TABLE IF NOT EXISTS usage_revisions (
    id                    BIGINT UNSIGNED NOT NULL AUTO_INCREMENT,
    ingest_id             BIGINT UNSIGNED NULL,
    source_id             VARCHAR(128)    NOT NULL,
    day                   DATE            NOT NULL,
    agent                 VARCHAR(32)     NOT NULL,
    model                 VARCHAR(160)    NOT NULL,
    input_tokens          BIGINT UNSIGNED NOT NULL,
    output_tokens         BIGINT UNSIGNED NOT NULL,
    cache_read_tokens     BIGINT UNSIGNED NOT NULL,
    cache_creation_tokens BIGINT UNSIGNED NOT NULL,
    reasoning_tokens      BIGINT UNSIGNED NOT NULL,
    total_tokens          BIGINT UNSIGNED NOT NULL,
    cost_usd              DOUBLE          NOT NULL,
    recorded_at           DATETIME(3)     NOT NULL,
    PRIMARY KEY (id),
    KEY usage_revisions_row (source_id, day, agent, model)
) ENGINE = InnoDB DEFAULT CHARSET = utf8mb4 COLLATE = utf8mb4_bin;

CREATE TRIGGER daily_usage_never_decreases BEFORE UPDATE ON daily_usage FOR EACH ROW
SET NEW.input_tokens          = GREATEST(NEW.input_tokens, OLD.input_tokens),
    NEW.output_tokens         = GREATEST(NEW.output_tokens, OLD.output_tokens),
    NEW.cache_read_tokens     = GREATEST(NEW.cache_read_tokens, OLD.cache_read_tokens),
    NEW.cache_creation_tokens = GREATEST(NEW.cache_creation_tokens, OLD.cache_creation_tokens),
    NEW.reasoning_tokens      = GREATEST(NEW.reasoning_tokens, OLD.reasoning_tokens),
    NEW.total_tokens          = GREATEST(NEW.total_tokens, OLD.total_tokens),
    NEW.cost_usd              = GREATEST(NEW.cost_usd, OLD.cost_usd),
    NEW.first_observed_at     = OLD.first_observed_at;

CREATE TRIGGER daily_usage_never_deleted BEFORE DELETE ON daily_usage FOR EACH ROW
SIGNAL SQLSTATE '45000' SET MESSAGE_TEXT = 'daily_usage is append-only; rows are never deleted';

CREATE TRIGGER usage_revisions_never_deleted BEFORE DELETE ON usage_revisions FOR EACH ROW
SIGNAL SQLSTATE '45000' SET MESSAGE_TEXT = 'usage_revisions is append-only; rows are never deleted';
