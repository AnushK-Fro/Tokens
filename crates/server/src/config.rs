use std::net::SocketAddr;

use anyhow::{Context, Result};
use chrono_tz::Tz;

#[derive(Clone)]
pub struct Config {
    pub database_url: String,
    pub bind: SocketAddr,
    /// Bearer token sync clients send to `/api/ingest`. Ingest is refused when unset.
    pub ingest_token: Option<String>,
    /// Zone stored days are counted in, and the dashboard's zone when a request names none.
    pub timezone: Tz,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let database_url = std::env::var("DATABASE_URL").context("DATABASE_URL is required, e.g. mysql://tokens:secret@localhost:3306/tokens")?;

        let bind = std::env::var("BIND")
            .unwrap_or_else(|_| "127.0.0.1:8787".to_owned())
            .parse()
            .context("BIND must be an address like 127.0.0.1:8787")?;

        let ingest_token = std::env::var("INGEST_TOKEN").ok().map(|token| token.trim().to_owned()).filter(|token| !token.is_empty());

        let timezone_name = std::env::var("TOKEN_USAGE_TIMEZONE").unwrap_or_else(|_| "UTC".to_owned());
        let timezone = timezone_name.parse::<Tz>().unwrap_or_else(|_| {
            tracing::warn!("unknown TOKEN_USAGE_TIMEZONE `{timezone_name}`, using UTC");
            Tz::UTC
        });

        Ok(Self { database_url, bind, ingest_token, timezone })
    }
}
