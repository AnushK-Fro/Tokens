-- What was actually paid: subscriptions, credit top-ups and API invoices, one row per charge, as
-- listed on each provider's billing page. The rest of the dashboard prices usage at API rates;
-- this is the money that really left the account.
--
-- Rows come from `tokens import-payments <csv>` (see README "Spending"); the data is personal and
-- is never committed.
--
-- `provider` is the billing account (`chatgpt` and `openai-api` are separate bills from OpenAI).
-- `agent` is the dashboard agent the payment funds, or NULL when it funds none of them.
-- `kind` is `subscription` (a flat plan) or `usage` (credits, top-ups, metered invoices).
-- The unique key makes a re-imported row a no-op.

CREATE TABLE IF NOT EXISTS payments (
    id         BIGINT UNSIGNED NOT NULL AUTO_INCREMENT,
    paid_on    DATE            NOT NULL,
    provider   VARCHAR(32)     NOT NULL,
    product    VARCHAR(160)    NOT NULL,
    kind       VARCHAR(16)     NOT NULL,
    agent      VARCHAR(32)     NULL,
    amount_usd DECIMAL(10, 2)  NOT NULL,
    -- Invoice, receipt or order number where the bill shows one. Kept here, never served.
    reference  VARCHAR(128)    NOT NULL DEFAULT '',
    note       VARCHAR(255)    NOT NULL DEFAULT '',
    PRIMARY KEY (id),
    UNIQUE KEY payments_charge (provider, paid_on, amount_usd, reference),
    KEY payments_day (paid_on)
) ENGINE = InnoDB DEFAULT CHARSET = utf8mb4 COLLATE = utf8mb4_bin;
