//! Shared application state handed to every handler.

use std::sync::Arc;

use cadence::{NopMetricSink, StatsdClient};
use sqlx::PgPool;

use crate::config::Config;

#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    /// Redis client; connections are opened per use so construction needs no I/O.
    pub redis: redis::Client,
    pub cfg: Arc<Config>,
    pub http: reqwest::Client,
    pub statsd: Arc<StatsdClient>,
}

impl AppState {
    pub fn new(cfg: Arc<Config>, db: PgPool, redis: redis::Client) -> Self {
        let statsd = match &cfg.statsd_host {
            Some(host) => crate::middleware::statsd_client(host),
            None => StatsdClient::from_sink("supermarket", NopMetricSink),
        };
        AppState {
            db,
            redis,
            cfg,
            http: reqwest::Client::builder().build().expect("reqwest client"),
            statsd: Arc::new(statsd),
        }
    }

    /// State that never touches the network: lazy pool, parsed but unconnected Redis.
    pub fn for_tests(cfg: Config) -> Self {
        let db = sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            .connect_lazy(&cfg.database_url)
            .expect("lazy pool");
        let redis = redis::Client::open(cfg.redis_url.clone()).expect("redis client");
        Self::new(Arc::new(cfg), db, redis)
    }
}
