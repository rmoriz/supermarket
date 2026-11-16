//! Thin Redis helpers matching `Rails.cache` call sites.
//!
//! Rails stores `Marshal`-dumped entries under a URL-derived namespace, so during the
//! dual-run window the raw codec matters; `get_raw`/`set_raw` are the seam the
//! marshal codec plugs into (phase 1, `/universe`).

use redis::AsyncCommands;

use crate::state::AppState;

pub async fn get_raw(state: &AppState, key: &str) -> redis::RedisResult<Option<Vec<u8>>> {
    let mut conn = state.redis.get_multiplexed_async_connection().await?;
    conn.get(key).await
}

pub async fn set_raw(state: &AppState, key: &str, value: &[u8]) -> redis::RedisResult<()> {
    let mut conn = state.redis.get_multiplexed_async_connection().await?;
    let _: () = conn.set(key, value).await?;
    Ok(())
}

pub async fn del(state: &AppState, key: &str) -> redis::RedisResult<()> {
    let mut conn = state.redis.get_multiplexed_async_connection().await?;
    let _: () = conn.del(key).await?;
    Ok(())
}

/// Increments a counter and returns the new value.
pub async fn incr(state: &AppState, key: &str) -> redis::RedisResult<i64> {
    let mut conn = state.redis.get_multiplexed_async_connection().await?;
    conn.incr(key, 1).await
}

/// Pings the server, used by `/api/v1/health`.
pub async fn ping(state: &AppState) -> bool {
    match state.redis.get_multiplexed_async_connection().await {
        Ok(mut conn) => {
            let pong: redis::RedisResult<String> = redis::cmd("PING").query_async(&mut conn).await;
            pong.is_ok()
        }
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    fn test_state() -> AppState {
        AppState::for_tests(
            Config::from_env_with(&|var| match var {
                "DATABASE_URL" => Some("postgres://x/y".into()),
                "REDIS_URL" => Some(
                    std::env::var("REDIS_URL")
                        .unwrap_or_else(|_| "redis://localhost:56379/2".into()),
                ),
                "SECRET_KEY_BASE" => Some("s".into()),
                "FQDN" => Some("localhost".into()),
                _ => None,
            })
            .unwrap(),
        )
    }

    #[tokio::test]
    async fn cache_roundtrip_and_delete() {
        let state = test_state();
        let key = "supermarket-rs:test:roundtrip";
        set_raw(&state, key, b"value").await.expect("set");
        assert_eq!(get_raw(&state, key).await.unwrap(), Some(b"value".to_vec()));
        del(&state, key).await.expect("del");
        assert_eq!(get_raw(&state, key).await.unwrap(), None);
    }

    #[tokio::test]
    async fn ping_reports_reachable() {
        assert!(ping(&test_state()).await);
    }
}
