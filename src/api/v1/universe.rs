//! `/universe` and `/api/v1/universe`, ported from `app/models/universe.rb`.
//!
//! The cache value is JSON under the upstream key (`<protocol>-universe`). Upstream
//! stores a Marshal-dumped cache entry; a Marshal codec lands in the cutover phase,
//! and until then a Rails-written entry simply reads as a miss (JSON parse fails)
//! and is regenerated.

use axum::extract::State;
use axum::http::HeaderMap;
use axum::Json;
use serde_json::{json, Map, Value};

use crate::api::v1::cookbooks::internal;
use crate::cache;
use crate::error::ApiError;
use crate::state::AppState;

/// `UniverseCache#protocol`: `request.protocol` minus its trailing `://`.
fn protocol(headers: &HeaderMap) -> String {
    match headers
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        .filter(|v| !v.is_empty())
    {
        Some(proto) => proto.split(',').next().unwrap_or(proto).trim().to_owned(),
        None => "http".to_owned(),
    }
}

pub fn cache_key(protocol: &str) -> String {
    format!("{protocol}-universe")
}

async fn generate(state: &AppState, proto: &str) -> Result<Value, ApiError> {
    let base = match &state.cfg.port {
        Some(port) if !port.is_empty() => format!("{proto}://{}:{port}", state.cfg.fqdn),
        _ => format!("{proto}://{}", state.cfg.fqdn),
    };
    let location_path = format!("{base}/api/v1");

    let rows: Vec<(String, String, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT cookbook_versions.version, cookbooks.name,
                cookbook_dependencies.name, cookbook_dependencies.version_constraint
           FROM cookbook_versions
           INNER JOIN cookbooks ON cookbooks.id = cookbook_versions.cookbook_id
           LEFT JOIN cookbook_dependencies
             ON cookbook_dependencies.cookbook_version_id = cookbook_versions.id
            AND cookbook_dependencies.name != cookbooks.name",
    )
    .fetch_all(&state.db)
    .await
    .map_err(internal)?;

    let mut cookbooks = Map::new();
    for (version, name, dependency, constraint) in rows {
        let entry = cookbooks
            .entry(name.clone())
            .or_insert_with(|| Value::Object(Map::new()));
        let versions = entry.as_object_mut().unwrap();
        let version_entry = versions.entry(version.clone()).or_insert_with(|| {
            json!({
                "location_type": "opscode",
                "location_path": location_path,
                "download_url": format!("{base}/api/v1/cookbooks/{name}/versions/{version}/download"),
                "dependencies": {},
            })
        });
        if let (Some(dependency), Some(constraint)) = (dependency, constraint) {
            version_entry
                .as_object_mut()
                .unwrap()
                .get_mut("dependencies")
                .unwrap()
                .as_object_mut()
                .unwrap()
                .insert(dependency, json!(constraint));
        }
    }

    Ok(Value::Object(cookbooks))
}

async fn track_hit(state: &AppState) -> Result<(), ApiError> {
    sqlx::query(
        "WITH upd AS (
           UPDATE hits SET total = total + 1 WHERE label = 'universe' RETURNING *
         )
         INSERT INTO hits (label, total)
         SELECT 'universe', 1 WHERE NOT EXISTS (SELECT * FROM upd)",
    )
    .execute(&state.db)
    .await
    .map_err(internal)?;
    Ok(())
}

pub async fn index(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Value>, ApiError> {
    let proto = protocol(&headers);
    let key = cache_key(&proto);

    let universe = match cache::get_raw(&state, &key).await.ok().flatten() {
        Some(bytes) => match serde_json::from_slice::<Value>(&bytes) {
            Ok(value) => value,
            Err(_) => {
                let generated = generate(&state, &proto).await?;
                let _ = cache::set_raw(&state, &key, generated.to_string().as_bytes()).await;
                generated
            }
        },
        None => {
            let generated = generate(&state, &proto).await?;
            let _ = cache::set_raw(&state, &key, generated.to_string().as_bytes()).await;
            generated
        }
    };

    track_hit(&state).await?;
    Ok(Json(universe))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_keys_match_upstream() {
        assert_eq!(cache_key("http"), "http-universe");
        assert_eq!(cache_key("https"), "https-universe");
    }
}
