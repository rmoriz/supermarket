//! Postgres pool.

use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;

use crate::config::Config;

/// Pool size follows `DATABASE_POOL` (upstream default 5).
pub fn pool_size() -> u32 {
    std::env::var("DATABASE_POOL")
        .ok()
        .and_then(|v| v.parse().ok())
        .filter(|n: &u32| *n > 0)
        .unwrap_or(5)
}

pub async fn connect(cfg: &Config) -> Result<PgPool, sqlx::Error> {
    PgPoolOptions::new()
        .max_connections(pool_size())
        .connect(&cfg.database_url)
        .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn pool_connects_and_selects_one() {
        let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
            "postgres://supermarket:supermarket@localhost:55432/supermarket_test".into()
        });
        let cfg = Config::from_env_with(&|var| match var {
            "DATABASE_URL" => Some(url.clone()),
            "REDIS_URL" => Some("redis://localhost:56379/0".into()),
            "SECRET_KEY_BASE" => Some("s".into()),
            "FQDN" => Some("localhost".into()),
            _ => None,
        })
        .unwrap();
        let pool = connect(&cfg).await.expect("connects");
        let one: i32 = sqlx::query_scalar("SELECT 1")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(one, 1);
    }
}
