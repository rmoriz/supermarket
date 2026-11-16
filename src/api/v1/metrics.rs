//! `Api::V1::MetricsController#show` — public counters.

use axum::extract::State;
use axum::Json;
use serde_json::json;

use crate::api::v1::cookbooks::internal;
use crate::error::ApiError;
use crate::state::AppState;

pub async fn show(State(state): State<AppState>) -> Result<Json<serde_json::Value>, ApiError> {
    let (downloads, versions, cookbooks, follows, users, hits): (i64, i64, i64, i64, i64, Option<i32>) =
        sqlx::query_as(
            "SELECT
               (SELECT coalesce(sum(web_download_count + api_download_count), 0)::bigint FROM cookbooks),
               (SELECT count(*) FROM cookbook_versions),
               (SELECT count(*) FROM cookbooks),
               (SELECT count(*) FROM cookbook_followers),
               (SELECT count(*) FROM users),
               (SELECT total FROM hits WHERE label = 'universe')",
        )
        .fetch_one(&state.db)
        .await
        .map_err(internal)?;

    Ok(Json(json!({
        "total_cookbook_downloads": downloads,
        "total_cookbook_versions": versions,
        "total_cookbooks": cookbooks,
        "total_follows": follows,
        "total_users": users,
        "total_hits": { "/universe": hits.unwrap_or(0) },
    })))
}
