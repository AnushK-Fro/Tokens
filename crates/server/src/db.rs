use std::time::Duration;

use anyhow::{Context, Result};
use sqlx::mysql::{MySqlConnectOptions, MySqlPoolOptions};
use sqlx::MySqlPool;

pub async fn connect(database_url: &str) -> Result<MySqlPool> {
    let options: MySqlConnectOptions = database_url.parse().context("DATABASE_URL is not a valid mysql:// URL")?;
    let options = options.charset("utf8mb4").timezone(Some("+00:00".to_owned()));

    let pool = MySqlPoolOptions::new()
        .max_connections(8)
        .acquire_timeout(Duration::from_secs(10))
        .connect_with(options)
        .await
        .context("connecting to MariaDB")?;

    sqlx::migrate!("./migrations").run(&pool).await.context("running migrations")?;

    Ok(pool)
}
