//! `Api::V1::CookbookVersionsController` — version detail and artifact redirect.

use axum::extract::{Path, State};
use axum::response::{IntoResponse, Response};
use axum::Json;
use cadence::CountedExt;
use serde_json::{json, Value};

use crate::api::v1::cookbooks::{cookbook_url, find_cookbook, internal, version_url, CookbookRow};
use crate::chef_version;
use crate::error::ApiError;
use crate::state::AppState;

struct VersionRow {
    license: String,
    size: Option<i32>,
    version: String,
    created_at: chrono::NaiveDateTime,
    file_name: Option<String>,
    tarball_updated_at: Option<chrono::NaiveDateTime>,
    legacy_id: Option<i32>,
    /// Insertion order, exactly as the association loads them.
    platforms: Vec<(String, String)>,
    dependencies: Vec<(String, String)>,
}

async fn load_version(
    state: &AppState,
    cookbook: &CookbookRow,
    requested: &str,
) -> Result<VersionRow, ApiError> {
    let normalised = requested.replace('_', ".");

    if normalised == "latest" {
        let versions: Vec<String> =
            sqlx::query_scalar("SELECT version FROM cookbook_versions WHERE cookbook_id = $1")
                .bind(cookbook.id)
                .fetch_all(&state.db)
                .await
                .map_err(internal)?;
        let Some(latest) = chef_version::latest(&versions)
            .map_err(|error| ApiError::InvalidData(vec![error.to_string()]))?
        else {
            return Err(ApiError::NotFound);
        };
        return load_exact(state, cookbook, &latest).await;
    }

    load_exact(state, cookbook, &normalised).await
}

/// version id, license, tarball size, created_at, tarball file name,
/// tarball updated_at, legacy id
type VersionColumns = (
    i32,
    String,
    Option<i32>,
    chrono::NaiveDateTime,
    Option<String>,
    Option<chrono::NaiveDateTime>,
    Option<i32>,
);

async fn load_exact(
    state: &AppState,
    cookbook: &CookbookRow,
    version: &str,
) -> Result<VersionRow, ApiError> {
    let row: Option<VersionColumns> = sqlx::query_as(
        "SELECT id, license, tarball_file_size, created_at, tarball_file_name,
                tarball_updated_at, legacy_id
           FROM cookbook_versions
          WHERE cookbook_id = $1 AND version = $2",
    )
    .bind(cookbook.id)
    .bind(version)
    .fetch_optional(&state.db)
    .await
    .map_err(internal)?;

    let Some((id, license, size, created_at, file_name, tarball_updated_at, legacy_id)) = row
    else {
        return Err(ApiError::NotFound);
    };

    let platforms: Vec<(String, String)> = sqlx::query_as(
        "SELECT sp.name, sp.version_constraint
           FROM cookbook_version_platforms cvp
           JOIN supported_platforms sp ON sp.id = cvp.supported_platform_id
          WHERE cvp.cookbook_version_id = $1
          ORDER BY cvp.id",
    )
    .bind(id)
    .fetch_all(&state.db)
    .await
    .map_err(internal)?;

    let dependencies: Vec<(String, String)> = sqlx::query_as(
        "SELECT name, version_constraint FROM cookbook_dependencies
          WHERE cookbook_version_id = $1 ORDER BY id",
    )
    .bind(id)
    .fetch_all(&state.db)
    .await
    .map_err(internal)?;

    Ok(VersionRow {
        license,
        size,
        version: version.to_owned(),
        created_at,
        file_name,
        tarball_updated_at,
        legacy_id,
        platforms,
        dependencies,
    })
}

/// `CookbookVersion#cookbook_artifact_url`: the paperclip URL, a presigned one
/// when `S3_URLS_EXPIRE` is set. Filesystem URLs carry the `?<mtime epoch>`
/// cache buster paperclip appends; presigned URLs never do.
pub async fn artifact_url(
    state: &AppState,
    version_id: i32,
    legacy_id: Option<i32>,
    file_name: &str,
    updated_at: Option<chrono::NaiveDateTime>,
) -> String {
    let id = legacy_id.unwrap_or(version_id);
    if let Some(s3) = state.s3.as_deref() {
        let key = s3.tarball_key(&id.to_string(), file_name);
        if let Some(expires_in) = state.cfg.s3.as_ref().and_then(|config| config.url_expiry) {
            return s3
                .presigned_get(&key, expires_in)
                .await
                .unwrap_or_else(|_| s3.url(&key));
        }
        let cache_buster = updated_at
            .map(|value| format!("?{}", value.and_utc().timestamp()))
            .unwrap_or_default();
        return format!("{}{cache_buster}", s3.url(&key));
    }
    let base = state.cfg.base_url();
    let cache_buster = updated_at
        .map(|value| format!("?{}", value.and_utc().timestamp()))
        .unwrap_or_default();
    format!("{base}/system/cookbook_versions/tarballs/{id}/original/{file_name}{cache_buster}")
}

pub async fn show(
    State(state): State<AppState>,
    Path((cookbook, version)): Path<(String, String)>,
) -> Result<Json<Value>, ApiError> {
    let cookbook = find_cookbook(&state.db, &cookbook)
        .await?
        .ok_or(ApiError::NotFound)?;
    let found = load_version(&state, &cookbook, &version).await?;
    let base = state.cfg.base_url();

    let mut body = json!({
        "license": found.license,
        "tarball_file_size": found.size,
        "version": found.version,
        "published_at": found.created_at.and_utc().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        "average_rating": Value::Null,
        "cookbook": cookbook_url(&base, &cookbook.name),
        "file": format!("{}/download", version_url(&base, &cookbook.name, &found.version)),
    });

    if state.cfg.feature_active("fieri") {
        let version_id: i32 = sqlx::query_scalar(
            "SELECT id FROM cookbook_versions WHERE cookbook_id = $1 AND version = $2",
        )
        .bind(cookbook.id)
        .bind(&found.version)
        .fetch_one(&state.db)
        .await
        .map_err(internal)?;
        let metrics: Vec<(String, bool, Option<String>)> = sqlx::query_as(
            "SELECT qm.name, mr.failure, mr.feedback
               FROM metric_results mr
               JOIN quality_metrics qm ON qm.id = mr.quality_metric_id
              WHERE mr.cookbook_version_id = $1
              ORDER BY mr.id",
        )
        .bind(version_id)
        .fetch_all(&state.db)
        .await
        .map_err(internal)?;
        // jbuilder always emits the key; an empty relation becomes [].
        body.as_object_mut().unwrap().insert(
            "quality_metrics".into(),
            Value::Array(
                metrics
                    .into_iter()
                    .map(|(name, failed, feedback)| {
                        json!({ "name": name, "failed": failed, "feedback": feedback })
                    })
                    .collect(),
            ),
        );
    }

    let supports: serde_json::Map<String, Value> = found
        .platforms
        .iter()
        .map(|(name, constraint)| (name.clone(), json!(constraint)))
        .collect();
    let dependencies: serde_json::Map<String, Value> = found
        .dependencies
        .iter()
        .map(|(name, constraint)| (name.clone(), json!(constraint)))
        .collect();
    let map = body.as_object_mut().unwrap();
    if !supports.is_empty() {
        map.insert("supports".into(), Value::Object(supports));
    }
    if !dependencies.is_empty() {
        map.insert("dependencies".into(), Value::Object(dependencies));
    }

    Ok(Json(body))
}

/// `GET .../download`: counters first, then a 302 to the artifact.
pub async fn download(
    State(state): State<AppState>,
    Path((cookbook, version)): Path<(String, String)>,
) -> Result<Response, ApiError> {
    let cookbook = find_cookbook(&state.db, &cookbook)
        .await?
        .ok_or(ApiError::NotFound)?;
    let found = load_version(&state, &cookbook, &version).await?;

    let version_id: i32 = sqlx::query_scalar(
        "SELECT id FROM cookbook_versions WHERE cookbook_id = $1 AND version = $2",
    )
    .bind(cookbook.id)
    .bind(&found.version)
    .fetch_one(&state.db)
    .await
    .map_err(internal)?;

    let mut tx = state.db.begin().await.map_err(internal)?;
    sqlx::query(
        "UPDATE cookbook_versions SET api_download_count = api_download_count + 1 WHERE id = $1",
    )
    .bind(version_id)
    .execute(&mut *tx)
    .await
    .map_err(internal)?;
    sqlx::query("UPDATE cookbooks SET api_download_count = api_download_count + 1 WHERE id = $1")
        .bind(cookbook.id)
        .execute(&mut *tx)
        .await
        .map_err(internal)?;
    tx.commit().await.map_err(internal)?;

    let _ = state.statsd.incr("cookbook.downloads.api");

    let file_name = found.file_name.unwrap_or_else(|| "tarball.tgz".to_owned());
    let target = artifact_url(
        &state,
        version_id,
        found.legacy_id,
        &file_name,
        found.tarball_updated_at,
    )
    .await;
    // Rails `redirect_to` answers 302, not the 303 axum's Redirect emits.
    Ok((
        axum::http::StatusCode::FOUND,
        [(axum::http::header::LOCATION, target)],
    )
        .into_response())
}
