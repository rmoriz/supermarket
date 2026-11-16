//! `/api/v1/health` and `/status`, ported from `app/lib/supermarket/health.rb`.
//!
//! The HTTP status is always 200; only the body describes component health.

use axum::extract::State;
use axum::Json;
use redis::AsyncCommands;
use serde_json::{json, Map, Value};

use crate::state::AppState;

const REACHABLE: &str = "reachable";
const UNKNOWN: &str = "unknown";
const UNREACHABLE: &str = "unreachable";
const ALL_FEATURES: [&str; 6] = [
    "tools",
    "fieri",
    "announcement",
    "github",
    "no_crawl",
    "chat",
];

#[derive(Default)]
struct Report {
    status: String,
    supermarket: Map<String, Value>,
    sidekiq: Map<String, Value>,
    postgresql: Map<String, Value>,
    redis: Map<String, Value>,
    features: Map<String, Value>,
}

pub async fn show(State(state): State<AppState>) -> Json<Value> {
    Json(check(&state).await)
}

async fn check(state: &AppState) -> Value {
    let mut report = Report {
        status: String::new(),
        supermarket: Map::new(),
        sidekiq: status_map(REACHABLE),
        postgresql: status_map(REACHABLE),
        redis: status_map(REACHABLE),
        features: Map::new(),
    };

    // expired oc-id tokens; a dead database leaves the counter absent, like upstream.
    match sqlx::query_scalar::<_, i64>(
        "SELECT count(*) FROM accounts WHERE provider = 'chef_oauth2' AND oauth_expires < now()",
    )
    .fetch_one(&state.db)
    .await
    {
        Ok(count) => {
            report
                .supermarket
                .insert("expired_ocid_tokens".into(), json!(count));
        }
        Err(error) => {
            report.postgresql = status_map(&classify_pg(&error));
        }
    }

    if report.postgresql.get("status").and_then(Value::as_str) != Some(UNREACHABLE) {
        match sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM pg_stat_activity WHERE wait_event IS NOT NULL",
        )
        .fetch_one(&state.db)
        .await
        {
            Ok(waiting) => {
                report
                    .postgresql
                    .insert("waiting_on_lock".into(), json!(waiting));
            }
            Err(error) => report.postgresql = status_map(&classify_pg(&error)),
        }
        match sqlx::query_scalar::<_, i64>("SELECT count(*) FROM pg_stat_activity")
            .fetch_one(&state.db)
            .await
        {
            Ok(connections) => {
                report
                    .postgresql
                    .insert("connections".into(), json!(connections));
            }
            Err(error) => report.postgresql = status_map(&classify_pg(&error)),
        }
    }

    let redis = state.redis.get_multiplexed_async_connection().await;
    match redis {
        Err(_) => {
            report.sidekiq = status_map(UNREACHABLE);
            report.redis = status_map(UNREACHABLE);
        }
        Ok(mut connection) => {
            sidekiq_health(&mut connection, &mut report).await;
            redis_health(&mut connection, &mut report).await;
        }
    }

    for feature in ALL_FEATURES {
        report
            .features
            .insert(feature.into(), json!(state.cfg.feature_active(feature)));
    }

    let ok = [&report.sidekiq, &report.postgresql, &report.redis]
        .iter()
        .all(|map| map.get("status").and_then(Value::as_str) == Some(REACHABLE));
    report.status = if ok { "ok" } else { "not ok" }.to_owned();

    json!({
        "status": report.status,
        "supermarket": report.supermarket,
        "sidekiq": report.sidekiq,
        "postgresql": report.postgresql,
        "redis": report.redis,
        "features": report.features,
    })
}

fn status_map(status: &str) -> Map<String, Value> {
    let mut map = Map::new();
    map.insert("status".into(), json!(status));
    map
}

fn classify_pg(error: &sqlx::Error) -> String {
    match error {
        sqlx::Error::PoolTimedOut => UNKNOWN.to_owned(),
        _ => UNREACHABLE.to_owned(),
    }
}

async fn sidekiq_health(connection: &mut redis::aio::MultiplexedConnection, report: &mut Report) {
    let result: redis::RedisResult<()> = async {
        let latency = queue_latency(connection).await.unwrap_or(0.0);
        let queued: i64 = connection.llen("queue:default").await?;
        let scheduled: i64 = connection.zcard("schedule").await?;
        let retryable: i64 = connection.zcard("retry").await?;
        let dead: i64 = connection.zcard("dead").await?;
        let processed: i64 = connection.get("stat:processed").await.unwrap_or(0);
        let failed: i64 = connection.get("stat:failed").await.unwrap_or(0);
        let workers: i64 = connection.hlen("workers").await?;

        // Sidekiq reports whole seconds; keep the JSON number integral when it is.
        report.sidekiq.insert(
            "latency".into(),
            if latency.fract() == 0.0 {
                json!(latency as i64)
            } else {
                json!(latency)
            },
        );
        report.sidekiq.insert("queued_jobs".into(), json!(queued));
        report
            .sidekiq
            .insert("scheduled_jobs".into(), json!(scheduled));
        report
            .sidekiq
            .insert("retryable_jobs".into(), json!(retryable));
        report.sidekiq.insert("dead_jobs".into(), json!(dead));
        report
            .sidekiq
            .insert("total_processed".into(), json!(processed));
        report.sidekiq.insert("total_failed".into(), json!(failed));
        report
            .sidekiq
            .insert("active_workers".into(), json!(workers));
        Ok(())
    }
    .await;

    if result.is_err() {
        report.sidekiq = status_map(UNREACHABLE);
        report.redis = status_map(UNREACHABLE);
    }
}

async fn queue_latency(connection: &mut redis::aio::MultiplexedConnection) -> Option<f64> {
    let first: Option<String> = connection.lindex("queue:default", 0).await.ok()?;
    let first = first?;
    let parsed: Value = serde_json::from_str(&first).ok()?;
    let enqueued_at = parsed.get("enqueued_at")?.as_f64()?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_secs_f64();
    Some(((now - enqueued_at) * 100.0).round() / 100.0)
}

async fn redis_health(connection: &mut redis::aio::MultiplexedConnection, report: &mut Report) {
    let info: Option<String> = match redis::cmd("INFO").query_async(connection).await {
        Ok(info) => Some(info),
        Err(_) => {
            report.sidekiq = status_map(UNREACHABLE);
            report.redis = status_map(UNREACHABLE);
            None
        }
    };

    if let Some(info) = info {
        for key in [
            "uptime_in_seconds",
            "connected_clients",
            "used_memory",
            "used_memory_peak",
        ] {
            let value = info
                .lines()
                .find_map(|line| line.strip_prefix(&format!("{key}:")))
                .and_then(|raw| raw.trim().parse::<i64>().ok())
                .unwrap_or(-1);
            report.redis.insert(key.into(), json!(value));
        }
    }
}
