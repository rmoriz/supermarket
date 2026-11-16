//! The mounted `Fieri::Engine`: job intake and the status probe.
//!
//! `Fieri::ApplicationController` inherits the whole public controller stack, so
//! both actions carry the normal JSON content type and `no-cache`. Job intake
//! skips forgery protection; the status probe never writes.

use axum::extract::State;
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use redis::AsyncCommands;
use serde_json::json;

use crate::state::AppState;

/// The error envelope `Fieri::ApplicationController#param_missing_error` writes.
fn param_missing(param: &str) -> Response {
    (
        StatusCode::BAD_REQUEST,
        [(header::CONTENT_TYPE, "application/json; charset=utf-8")],
        json!({
            "status": "error",
            "message": format!("param is missing or the value is empty: {param}"),
        })
        .to_string(),
    )
        .into_response()
}

fn unauthorized() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        [(header::CONTENT_TYPE, "application/json; charset=utf-8")],
        json!({
            "error_code": crate::locales::t("api.error_codes.unauthorized", &[]),
            "error_messages": [crate::locales::t("api.error_messages.unauthorized_post_error", &[])],
        })
        .to_string(),
    )
        .into_response()
}

fn present(pairs: &[(String, String)], name: &str) -> bool {
    pairs
        .iter()
        .any(|(key, value)| key == name && !value.is_empty())
}

/// `POST /fieri/jobs`: `params.require(:fieri_key)`, then authorize, then
/// `params.require(:cookbook).permit(:name, :version, :artifact_url)`.
pub async fn create(State(state): State<AppState>, body: String) -> Response {
    let pairs: Vec<(String, String)> = form_urlencoded::parse(body.as_bytes())
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect();

    if !present(&pairs, "fieri_key") {
        return param_missing("fieri_key");
    }
    let key = pairs
        .iter()
        .find(|(name, _)| name == "fieri_key")
        .map(|(_, value)| value.as_str())
        .unwrap_or_default();
    if state.cfg.fieri_key.as_deref() != Some(key) {
        return unauthorized();
    }
    // `params.require(:cookbook)` only needs one `cookbook[...]` key to yield the
    // nested hash; the individual fields are then required in order.
    if !pairs.iter().any(|(name, _)| name.starts_with("cookbook[")) {
        return param_missing("cookbook");
    }

    let mut name = None;
    let mut version = None;
    let mut artifact_url = None;
    for (field, value) in &pairs {
        match field.as_str() {
            "cookbook[name]" => name = Some(value.clone()).filter(|v| !v.is_empty()),
            "cookbook[version]" => version = Some(value.clone()).filter(|v| !v.is_empty()),
            "cookbook[artifact_url]" => {
                artifact_url = Some(value.clone()).filter(|v| !v.is_empty())
            }
            _ => {}
        }
    }
    let Some(name) = name else {
        return param_missing("name");
    };
    let Some(version) = version else {
        return param_missing("version");
    };
    let Some(artifact_url) = artifact_url else {
        return param_missing("artifact_url");
    };

    let _ = crate::jobs::perform_async(
        &state,
        "MetricsRunner",
        vec![json!({
            "name": name,
            "version": version,
            "artifact_url": artifact_url,
        })],
    )
    .await;

    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json; charset=utf-8")],
        json!({ "status": "ok" }).to_string(),
    )
        .into_response()
}

fn status_map(status: &str) -> serde_json::Map<String, serde_json::Value> {
    let mut map = serde_json::Map::new();
    map.insert("status".into(), json!(status));
    map
}

/// `GET /fieri/status`: the `Sidekiq::Queue`/`Workers`/`DeadSet`/`RetrySet`/
/// `Stats` numbers plus the Redis INFO counters, with `UNKNOWN` on a timeout
/// and `UNREACHABLE` when Redis cannot be reached at all.
pub async fn status(State(state): State<AppState>, _headers: HeaderMap) -> Response {
    let mut sidekiq = status_map("REACHABLE");
    let mut redis_health = status_map("REACHABLE");

    let mut connection = match state.redis_jobs.get_multiplexed_async_connection().await {
        Ok(connection) => connection,
        Err(_) => {
            sidekiq = status_map("UNREACHABLE");
            redis_health = status_map("UNREACHABLE");
            return render((sidekiq, redis_health));
        }
    };

    let result: redis::RedisResult<()> = async {
        let latency = if let Some(first) = connection
            .lindex::<_, Option<String>>("queue:default", 0)
            .await?
        {
            match serde_json::from_str::<serde_json::Value>(&first)
                .ok()
                .and_then(|value| value.get("enqueued_at").and_then(|v| v.as_f64()))
            {
                Some(enqueued) => {
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|d| d.as_secs_f64())
                        .unwrap_or_default();
                    json!(now - enqueued)
                }
                None => json!(0),
            }
        } else {
            // `Sidekiq::Queue#latency` yields the Integer 0 on an empty queue.
            json!(0)
        };
        let queued: i64 = connection.llen("queue:default").await?;
        let workers: i64 = connection.hlen("workers").await?;
        let dead: i64 = connection.zcard("dead").await?;
        let retryable: i64 = connection.zcard("retry").await?;
        let processed: i64 = connection.get("stat:processed").await.unwrap_or(0);
        let failed: i64 = connection.get("stat:failed").await.unwrap_or(0);

        sidekiq.insert("latency".into(), latency);
        sidekiq.insert("queued_jobs".into(), json!(queued));
        sidekiq.insert("active_workers".into(), json!(workers));
        sidekiq.insert("dead_jobs".into(), json!(dead));
        sidekiq.insert("retryable_jobs".into(), json!(retryable));
        sidekiq.insert("total_processed".into(), json!(processed));
        sidekiq.insert("total_failed".into(), json!(failed));
        Ok(())
    }
    .await;

    if result.is_err() {
        sidekiq = status_map("UNREACHABLE");
        redis_health = status_map("UNREACHABLE");
        return render((sidekiq, redis_health));
    }

    let info: Option<String> = redis::cmd("INFO").query_async(&mut connection).await.ok();
    let Some(info) = info else {
        sidekiq = status_map("UNREACHABLE");
        redis_health = status_map("UNREACHABLE");
        return render((sidekiq, redis_health));
    };
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
        redis_health.insert(key.into(), json!(value));
    }

    render((sidekiq, redis_health))
}

fn render(
    (sidekiq, redis_health): (
        serde_json::Map<String, serde_json::Value>,
        serde_json::Map<String, serde_json::Value>,
    ),
) -> Response {
    let ok = sidekiq.get("status").and_then(|v| v.as_str()) == Some("REACHABLE")
        && redis_health.get("status").and_then(|v| v.as_str()) == Some("REACHABLE");
    let body = json!({
        "status": if ok { "ok" } else { "not ok" },
        "sidekiq": sidekiq,
        "redis": redis_health,
    });
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json; charset=utf-8")],
        body.to_string(),
    )
        .into_response()
}
