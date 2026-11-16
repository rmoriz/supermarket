//! `Api::V1::QualityMetricsController`: evaluation result intake from Fieri.
//!
//! Rails renders error bodies with `Api::V1Controller#error` (code first, but
//! messages first for `render_404`), and `head 200` as a bare, empty success.

use axum::body::Body;
use axum::extract::State;
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde_json::json;
use sqlx::PgPool;

use crate::state::AppState;

/// `render_404`: messages before code, `NOT_FOUND`.
fn not_found() -> Response {
    (
        StatusCode::NOT_FOUND,
        [(header::CONTENT_TYPE, "application/json; charset=utf-8")],
        json!({
            "error_messages": [crate::locales::t("api.error_messages.not_found", &[])],
            "error_code": crate::locales::t("api.error_codes.not_found", &[]),
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

/// `rescue_from ActionController::ParameterMissing` → `missing_<param>`.
/// Unknown keys keep Rails' `Translation missing: …` fallback string.
fn missing_param(param: &str) -> Response {
    let key = format!("api.error_messages.missing_{param}");
    let message = crate::locales::lookup(&key)
        .map(str::to_owned)
        .unwrap_or_else(|| format!("Translation missing: en.{key}"));
    (
        StatusCode::BAD_REQUEST,
        [(header::CONTENT_TYPE, "application/json; charset=utf-8")],
        json!({
            "error_code": crate::locales::t("api.error_codes.invalid_data", &[]),
            "error_messages": [message],
        })
        .to_string(),
    )
        .into_response()
}

/// `ActionNotFound`, rendered by the exceptions app in the requested format.
fn action_not_found(action: &str) -> Response {
    let mut response = (
        StatusCode::NOT_FOUND,
        [(header::CONTENT_TYPE, "application/json; charset=utf-8")],
        json!({
            "message": format!(
                "The action '{action}' could not be found for Api::V1::QualityMetricsController"
            ),
        })
        .to_string(),
    )
        .into_response();
    response
        .extensions_mut()
        .insert(crate::middleware::ExceptionResponse);
    response
}

/// `NoMethodError` on the nil cookbook, rendered by the exceptions app.
fn nil_latest() -> Response {
    let mut response = (
        StatusCode::INTERNAL_SERVER_ERROR,
        [(header::CONTENT_TYPE, "application/json; charset=utf-8")],
        json!({ "message": "undefined method `latest_cookbook_version' for nil:NilClass" })
            .to_string(),
    )
        .into_response();
    response
        .extensions_mut()
        .insert(crate::middleware::ExceptionResponse);
    response
}

fn head_ok() -> Response {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        Body::empty(),
    )
        .into_response()
}

fn param<'a>(pairs: &'a [(String, String)], name: &str) -> Option<&'a str> {
    pairs
        .iter()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.as_str())
}

fn present(pairs: &[(String, String)], name: &str) -> bool {
    param(pairs, name).is_some_and(|value| !value.is_empty())
}

/// Runs the shared `before_action` chain. `Some(response)` wins.
#[allow(clippy::result_large_err)]
async fn guards(state: &AppState, pairs: &[(String, String)]) -> Result<Option<i32>, Response> {
    if !present(pairs, "cookbook_name") {
        return Err(missing_param("cookbook_name"));
    }
    if state.cfg.fieri_key.as_deref() != param(pairs, "fieri_key") {
        return Err(unauthorized());
    }
    let name = param(pairs, "cookbook_name").unwrap_or_default();
    let cookbook: Option<i32> =
        sqlx::query_scalar("SELECT id FROM cookbooks WHERE lowercase_name = $1 LIMIT 1")
            .bind(name.to_lowercase())
            .fetch_optional(&state.db)
            .await
            .ok()
            .flatten();
    let version = param(pairs, "cookbook_version").map(|version| version.replace('_', "."));
    let Some(cookbook) = cookbook else {
        // Without a version the controller calls `.first.latest_cookbook_version`
        // and dies on nil (500); with one, `first!` raises RecordNotFound (404).
        return Err(match version {
            Some(_) => not_found(),
            None => nil_latest(),
        });
    };

    // `get_version!` underscores to dots and treats `latest` specially; a
    // missing version falls back to the latest one.
    let id: Option<i32> = match version.as_deref() {
        Some("latest") | None => sqlx::query_scalar(
            "SELECT id FROM cookbook_versions WHERE cookbook_id = $1 ORDER BY id DESC LIMIT 1",
        )
        .bind(cookbook)
        .fetch_optional(&state.db)
        .await
        .ok()
        .flatten(),
        Some(version) => sqlx::query_scalar(
            "SELECT id FROM cookbook_versions WHERE cookbook_id = $1 AND version = $2 LIMIT 1",
        )
        .bind(cookbook)
        .bind(version)
        .fetch_optional(&state.db)
        .await
        .ok()
        .flatten(),
    };
    match id {
        Some(id) => Ok(Some(id)),
        None => Err(not_found()),
    }
}

async fn metric_id(db: &PgPool, name: &str) -> Option<i32> {
    let existing: Option<i32> =
        sqlx::query_scalar("SELECT id FROM quality_metrics WHERE name = $1")
            .bind(name)
            .fetch_optional(db)
            .await
            .ok()
            .flatten();
    if let Some(id) = existing {
        return Some(id);
    }
    sqlx::query_scalar(
        "INSERT INTO quality_metrics (name, created_at, updated_at, admin_only)
         VALUES ($1, now(), now(), false)
         ON CONFLICT (name) DO UPDATE SET name = EXCLUDED.name
         RETURNING id",
    )
    .bind(name)
    .fetch_optional(db)
    .await
    .ok()
    .flatten()
}

/// `create_metric`: insert the new result, then drop the older ones for the
/// same version and metric.
async fn create_metric(
    db: &PgPool,
    version_id: i32,
    metric: Option<i32>,
    failure: &str,
    feedback: &str,
) {
    let Some(metric) = metric else { return };
    let inserted: Option<i32> = sqlx::query_scalar(
        "INSERT INTO metric_results
           (cookbook_version_id, quality_metric_id, failure, feedback, created_at, updated_at)
         VALUES ($1, $2, $3::boolean, $4, now(), now()) RETURNING id",
    )
    .bind(version_id)
    .bind(metric)
    .bind(failure)
    .bind(feedback)
    .fetch_optional(db)
    .await
    .ok()
    .flatten();
    let Some(inserted) = inserted else { return };
    let _ = sqlx::query(
        "DELETE FROM metric_results
          WHERE cookbook_version_id = $1 AND quality_metric_id = $2 AND id != $3",
    )
    .bind(version_id)
    .bind(metric)
    .bind(inserted)
    .execute(db)
    .await;
}

fn bool_param(pairs: &[(String, String)], name: &str) -> String {
    match param(pairs, name) {
        Some("true") => "true".into(),
        Some("false") => "false".into(),
        _ => String::new(),
    }
}

fn body_of(body: &str) -> Vec<(String, String)> {
    form_urlencoded::parse(body.as_bytes())
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect()
}

/// `POST /api/v1/quality_metrics/cookstyle_evaluation`
pub async fn cookstyle(State(state): State<AppState>, body: String) -> Response {
    let pairs = body_of(&body);
    let version = match guards(&state, &pairs).await {
        Ok(version) => version,
        Err(response) => return response,
    };
    for required in ["cookbook_version", "cookstyle_failure"] {
        if !present(&pairs, required) {
            return missing_param(required);
        }
    }
    let Some(version) = version else {
        return not_found();
    };
    create_metric(
        &state.db,
        version,
        metric_id(&state.db, "Cookstyle").await,
        &bool_param(&pairs, "cookstyle_failure"),
        param(&pairs, "cookstyle_feedback").unwrap_or_default(),
    )
    .await;
    // Foodcritic is superseded by cookstyle; its results (and license's) go.
    for name in ["Foodcritic", "License"] {
        if let Some(metric) = metric_id(&state.db, name).await {
            let _ = sqlx::query(
                "DELETE FROM metric_results WHERE cookbook_version_id = $1 AND quality_metric_id = $2",
            )
            .bind(version)
            .bind(metric)
            .execute(&state.db)
            .await;
        }
    }
    head_ok()
}

/// The six simple actions share a shape: name, metric, required params.
async fn simple(state: &AppState, body: &str, metric: &str, required: &[&str]) -> Response {
    let pairs = body_of(body);
    let version = match guards(state, &pairs).await {
        Ok(version) => version,
        Err(response) => return response,
    };
    for name in required {
        if !present(&pairs, name) {
            return missing_param(name);
        }
    }
    let Some(version) = version else {
        return not_found();
    };
    let Some(failure) = required.first() else {
        return head_ok();
    };
    create_metric(
        &state.db,
        version,
        metric_id(&state.db, metric).await,
        &bool_param(&pairs, failure),
        param(&pairs, required[1]).unwrap_or_default(),
    )
    .await;
    head_ok()
}

pub async fn collaborators(State(state): State<AppState>, body: String) -> Response {
    simple(
        &state,
        &body,
        "Collaborator Number",
        &["collaborator_failure", "collaborator_feedback"],
    )
    .await
}

pub async fn contributing_file(State(state): State<AppState>, body: String) -> Response {
    simple(
        &state,
        &body,
        "Contributing File",
        &["contributing_file_failure", "contributing_file_feedback"],
    )
    .await
}

pub async fn testing_file(State(state): State<AppState>, body: String) -> Response {
    simple(
        &state,
        &body,
        "Testing File",
        &["testing_file_failure", "testing_file_feedback"],
    )
    .await
}

pub async fn version_tag(State(state): State<AppState>, body: String) -> Response {
    let pairs = body_of(&body);
    if let Ok(version) = guards(&state, &pairs).await {
        if version.is_some() && !present(&pairs, "cookbook_version") {
            return missing_param("cookbook_version");
        }
    }
    simple(
        &state,
        &body,
        "Version Tag",
        &["version_tag_failure", "version_tag_feedback"],
    )
    .await
}

pub async fn no_binaries(State(state): State<AppState>, body: String) -> Response {
    simple(
        &state,
        &body,
        "No Binaries",
        &["no_binaries_failure", "no_binaries_feedback"],
    )
    .await
}

/// `POST /api/v1/cookbook-versions/publish_evaluation` (no action exists).
pub async fn publish(State(_state): State<AppState>, _headers: HeaderMap) -> Response {
    action_not_found("publish_evaluation")
}

/// `POST /api/v1/quality_metrics/publish_evaluation` (no action exists).
pub async fn publish_quality(State(_state): State<AppState>) -> Response {
    action_not_found("publish_evaluation")
}

/// `POST /api/v1/quality_metrics/supported_platforms_evaluation`.
pub async fn supported_platforms(State(_state): State<AppState>) -> Response {
    action_not_found("supported_platforms_evaluation")
}
