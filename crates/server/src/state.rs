use std::sync::Arc;

use sqlx::MySqlPool;

use crate::config::Config;

#[derive(Clone)]
pub struct AppState {
    pub pool: MySqlPool,
    pub config: Arc<Config>,
}

impl AppState {
    pub fn new(pool: MySqlPool, config: Config) -> Self {
        Self { pool, config: Arc::new(config) }
    }
}
