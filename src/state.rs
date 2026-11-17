//! Shared application state handed to every handler.

use std::sync::Arc;

use cadence::{NopMetricSink, StatsdClient};
use sqlx::PgPool;

use crate::config::Config;
use crate::storage::S3;

#[derive(Clone)]
pub struct AppState {
    pub db: PgPool,
    /// Redis client; connections are opened per use so construction needs no I/O.
    pub redis: redis::Client,
    /// The Sidekiq queue, which may live on its own server (`REDIS_JOBQ_URL`).
    pub redis_jobs: redis::Client,
    pub cfg: Arc<Config>,
    /// S3 storage when `S3_BUCKET`/`S3_REGION` are set; the filesystem otherwise.
    pub s3: Option<Arc<S3>>,
    pub http: reqwest::Client,
    pub statsd: Arc<StatsdClient>,
}

impl AppState {
    pub fn new(cfg: Arc<Config>, db: PgPool, redis: redis::Client) -> Self {
        let statsd = match &cfg.statsd_host {
            Some(host) => crate::middleware::statsd_client(host),
            None => StatsdClient::from_sink("supermarket", NopMetricSink),
        };
        let redis_jobs =
            redis::Client::open(cfg.redis_jobq_url.clone()).expect("job queue redis client");
        let s3 = cfg
            .s3
            .as_ref()
            .map(|config| Arc::new(S3::from_config(config, &cfg.protocol)));
        AppState {
            db,
            redis,
            redis_jobs,
            cfg,
            s3,
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
