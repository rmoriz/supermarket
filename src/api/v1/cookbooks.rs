//! `Api::V1::CookbooksController`.

use axum::extract::State;
use axum::Json;
use serde_json::{json, Value};

use crate::api::v1::params::{self, ListParams};
use crate::chef_version;
use crate::error::ApiError;
use crate::query::QueryMap;
use crate::state::AppState;

pub struct CookbookRow {
    pub id: i32,
    pub name: String,
    pub deprecated: bool,
    pub description: Option<String>,
    pub latest_version: Option<String>,
    pub category: Option<String>,
    pub source_url: Option<String>,
    pub issues_url: Option<String>,
    pub created_at: chrono::NaiveDateTime,
    pub updated_at: chrono::NaiveDateTime,
    pub up_for_adoption: Option<bool>,
    pub replacement: Option<String>,
    pub maintainer: String,
    pub web_download_count: i32,
    pub api_download_count: i32,
    pub followers: i64,
    pub collaborators: i64,
}

#[derive(sqlx::FromRow)]
struct IndexRow {
    id: i32,
    name: String,
    maintainer: String,
}

pub fn format_timestamp(value: chrono::NaiveDateTime) -> String {
    value
        .and_utc()
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

pub fn cookbook_url(base: &str, name: &str) -> String {
    format!("{base}/api/v1/cookbooks/{name}")
}

pub fn version_url(base: &str, cookbook: &str, version: &str) -> String {
    format!("{base}/api/v1/cookbooks/{cookbook}/versions/{version}")
}

/// Versions of a cookbook, semver-descending (the Chef::Version order).
pub async fn sorted_versions(
    db: &sqlx::PgPool,
    cookbook_id: i32,
) -> Result<Vec<(String, i32, i32)>, ApiError> {
    let mut rows: Vec<(String, i32, i32)> = sqlx::query_as(
        "SELECT version, web_download_count, api_download_count
           FROM cookbook_versions WHERE cookbook_id = $1",
    )
    .bind(cookbook_id)
    .fetch_all(db)
    .await
    .map_err(internal)?;

    let names: Vec<String> = rows.iter().map(|r| r.0.clone()).collect();
    let order = chef_version::sort_desc(&names)
        .map_err(|error| ApiError::InvalidData(vec![error.to_string()]))?;
    let mut sorted = Vec::with_capacity(rows.len());
    for version in order {
        if let Some(index) = rows.iter().position(|r| r.0 == version) {
            sorted.push(rows.remove(index));
        }
    }
    Ok(sorted)
}

pub fn internal(error: impl std::fmt::Display) -> ApiError {
    tracing::error!(%error, "database error");
    ApiError::InvalidData(vec![error.to_string()])
}

#[derive(sqlx::FromRow)]
struct FindRow {
    id: i32,
    name: String,
    deprecated: bool,
    category: Option<String>,
    source_url: Option<String>,
    issues_url: Option<String>,
    replacement: Option<String>,
    created_at: chrono::NaiveDateTime,
    updated_at: chrono::NaiveDateTime,
    up_for_adoption: Option<bool>,
    maintainer: String,
    web_download_count: i32,
    api_download_count: i32,
    followers: i64,
    collaborators: i64,
}

pub async fn find_cookbook(db: &sqlx::PgPool, name: &str) -> Result<Option<CookbookRow>, ApiError> {
    let row: Option<FindRow> = sqlx::query_as(
        "SELECT c.id, c.name, c.deprecated, cat.name AS category, c.source_url, c.issues_url,
                rep.name AS replacement, c.created_at, c.updated_at,
                c.up_for_adoption, a.username AS maintainer, c.web_download_count,
                c.api_download_count,
                (SELECT count(*) FROM cookbook_followers f WHERE f.cookbook_id = c.id) AS followers,
                (SELECT count(*) FROM collaborators co
                   WHERE co.resourceable_type = 'Cookbook' AND co.resourceable_id = c.id) AS collaborators
           FROM cookbooks c
           JOIN users u ON u.id = c.user_id
           JOIN accounts a ON a.user_id = u.id AND a.provider = 'chef_oauth2'
           LEFT JOIN categories cat ON cat.id = c.category_id
           LEFT JOIN cookbooks rep ON rep.id = c.replacement_id
          WHERE c.lowercase_name = $1",
    )
    .bind(name.to_lowercase())
    .fetch_optional(db)
    .await
    .map_err(internal)?;

    let Some(row) = row else {
        return Ok(None);
    };

    let versions = sorted_versions(db, row.id).await?;
    let latest_version = versions.first().map(|v| v.0.clone());
    let description = match &latest_version {
        Some(version) => sqlx::query_scalar::<_, Option<String>>(
            "SELECT description FROM cookbook_versions WHERE cookbook_id = $1 AND version = $2",
        )
        .bind(row.id)
        .bind(version)
        .fetch_optional(db)
        .await
        .map_err(internal)?
        .flatten(),
        None => None,
    };

    Ok(Some(CookbookRow {
        id: row.id,
        name: row.name,
        deprecated: row.deprecated,
        description,
        latest_version,
        category: row.category,
        source_url: row.source_url,
        issues_url: row.issues_url,
        created_at: row.created_at,
        updated_at: row.updated_at,
        up_for_adoption: row.up_for_adoption,
        replacement: row.replacement,
        maintainer: row.maintainer,
        web_download_count: row.web_download_count,
        api_download_count: row.api_download_count,
        followers: row.followers,
        collaborators: row.collaborators,
    }))
}

pub async fn total_cookbooks(db: &sqlx::PgPool) -> Result<i64, ApiError> {
    sqlx::query_scalar("SELECT count(*) FROM cookbooks")
        .fetch_one(db)
        .await
        .map_err(internal)
}

/// Shared body of `index` and `search`.
fn list_body(params: ListParams, total: i64, rows: Vec<Value>) -> Json<Value> {
    Json(json!({
        "start": params.start,
        "total": total,
        "items": rows,
    }))
}

pub async fn index(
    State(state): State<AppState>,
    query: QueryMap,
) -> Result<Json<Value>, ApiError> {
    let params = params::parse(
        query.get("start"),
        query.get("items"),
        query.get("order"),
        state.cfg.api_item_limit,
    )?;
    let total = total_cookbooks(&state.db).await?;

    let user = query.get("user");
    let platforms = query.array("platforms[]");

    // `ORDER_OPTIONS` strings are static, so they are safe to splat.
    let sql = format!(
        "SELECT cookbooks.id, cookbooks.name, a.username AS maintainer
           FROM cookbooks
           JOIN users u ON u.id = cookbooks.user_id
           JOIN accounts a ON a.user_id = u.id AND a.provider = 'chef_oauth2'
          WHERE ($1::text IS NULL OR a.username = $1)
            AND (cardinality($2::text[]) = 0 OR EXISTS (
                  SELECT 1
                    FROM cookbook_versions cv
                    JOIN cookbook_version_platforms cvp ON cvp.cookbook_version_id = cv.id
                    JOIN supported_platforms sp ON sp.id = cvp.supported_platform_id
                   WHERE cv.cookbook_id = cookbooks.id AND sp.name = ANY($2)))
          ORDER BY {}
          LIMIT $3 OFFSET $4",
        params.order.cookbook_sql()
    );

    let rows: Vec<IndexRow> = sqlx::query_as(&sql)
        .bind(user)
        .bind(&platforms)
        .bind(params.items)
        .bind(params.start)
        .fetch_all(&state.db)
        .await
        .map_err(internal)?;

    let base = state.cfg.base_url();
    let mut items = Vec::with_capacity(rows.len());
    for row in rows {
        let cookbook = sqlx::query_as::<_, (Option<String>,)>(
            "SELECT description FROM cookbook_versions WHERE cookbook_id = $1 ORDER BY created_at DESC LIMIT 1",
        )
        .bind(row.id)
        .fetch_optional(&state.db)
        .await
        .map_err(internal)?;
        items.push(json!({
            "cookbook_name": row.name,
            "cookbook_maintainer": row.maintainer,
            "cookbook_description": cookbook.and_then(|c| c.0),
            "cookbook": cookbook_url(&base, &row.name),
        }));
    }

    Ok(list_body(params, total, items))
}

/// pg_search's `prefix: true` query: every word becomes a quoted prefix term.
/// The result is passed as `$2::tsquery`.
fn tsquery(term: &str) -> String {
    term.split_whitespace()
        .map(|word| format!("'{}':*", word.replace('\'', "''")))
        .collect::<Vec<_>>()
        .join(" && ")
}

pub async fn search(
    State(state): State<AppState>,
    query: QueryMap,
) -> Result<Json<Value>, ApiError> {
    let params = params::parse(
        query.get("start"),
        query.get("items"),
        query.get("order"),
        state.cfg.api_item_limit,
    )?;
    let total = total_cookbooks(&state.db).await?;
    let term = query.get("q").unwrap_or("").to_owned();
    let tsquery = tsquery(&term);

    // Mirrors the pg_search scope exactly: trigram on the name only, prefix
    // tsearch on maintainer and latest description, ranked
    // `trigram + 0.5 * tsearch`, ties by name.
    let rows: Vec<IndexRow> = sqlx::query_as(
        "SELECT id, name, maintainer, rank FROM (
           SELECT c.id, c.name, a.username AS maintainer,
                  similarity(c.name, $1)
                  + 0.5 * ts_rank(
                      setweight(to_tsvector('english', coalesce(a.username, '')), 'B')
                      || setweight(to_tsvector('english', coalesce(d.description, '')), 'C'),
                      $2::tsquery) AS rank
             FROM cookbooks c
             JOIN users u ON u.id = c.user_id
             JOIN accounts a ON a.user_id = u.id AND a.provider = 'chef_oauth2'
             LEFT JOIN LATERAL (
               SELECT description FROM cookbook_versions v
                WHERE v.cookbook_id = c.id
                ORDER BY v.created_at DESC LIMIT 1
             ) d ON TRUE
            WHERE $1 <> '' AND (
                  c.name % $1
               OR (setweight(to_tsvector('english', coalesce(a.username, '')), 'B')
                   || setweight(to_tsvector('english', coalesce(d.description, '')), 'C'))
                  @@ $2::tsquery)
         ) matched
        ORDER BY rank DESC, name ASC
        LIMIT $3 OFFSET $4",
    )
    .bind(&term)
    .bind(&tsquery)
    .bind(params.items)
    .bind(params.start)
    .fetch_all(&state.db)
    .await
    .map_err(internal)?;

    let base = state.cfg.base_url();
    let mut items = Vec::with_capacity(rows.len());
    for row in rows {
        let description: Option<(Option<String>,)> = sqlx::query_as(
            "SELECT description FROM cookbook_versions WHERE cookbook_id = $1 ORDER BY created_at DESC LIMIT 1",
        )
        .bind(row.id)
        .fetch_optional(&state.db)
        .await
        .map_err(internal)?;
        items.push(json!({
            "cookbook_name": row.name,
            "cookbook_maintainer": row.maintainer,
            "cookbook_description": description.and_then(|d| d.0),
            "cookbook": cookbook_url(&base, &row.name),
        }));
    }

    Ok(list_body(params, total, items))
}

pub async fn show(
    State(state): State<AppState>,
    axum::extract::Path(cookbook): axum::extract::Path<String>,
) -> Result<Json<Value>, ApiError> {
    let cookbook = find_cookbook(&state.db, &cookbook)
        .await?
        .ok_or(ApiError::NotFound)?;
    let base = state.cfg.base_url();

    let versions = sorted_versions(&state.db, cookbook.id).await?;
    let latest = cookbook
        .latest_version
        .as_deref()
        .map(|version| version_url(&base, &cookbook.name, version));

    let mut body = json!({
        "name": cookbook.name,
        "maintainer": cookbook.maintainer,
        "description": cookbook.description,
        "category": cookbook.category,
        "latest_version": latest,
        "external_url": cookbook.source_url,
        "source_url": cookbook.source_url,
        "issues_url": cookbook.issues_url,
        "average_rating": Value::Null,
        "created_at": format_timestamp(cookbook.created_at),
        "updated_at": format_timestamp(cookbook.updated_at),
        "up_for_adoption": cookbook.up_for_adoption,
    });

    let map = body.as_object_mut().unwrap();
    map.insert("deprecated".into(), json!(cookbook.deprecated));
    if cookbook.deprecated {
        if let Some(replacement) = &cookbook.replacement {
            map.insert(
                "replacement".into(),
                json!(cookbook_url(&base, replacement)),
            );
        }
    }

    let urls: Vec<String> = versions
        .iter()
        .map(|(version, _, _)| version_url(&base, &cookbook.name, version))
        .collect();
    map.insert("versions".into(), json!(urls));

    // The jbuilder view iterates the unordered association, so the hash keys
    // come back in Postgres' physical row order — no ORDER BY here either.
    let per_version_rows: Vec<(String, i32, i32)> = sqlx::query_as(
        "SELECT version, web_download_count, api_download_count
           FROM cookbook_versions WHERE cookbook_id = $1",
    )
    .bind(cookbook.id)
    .fetch_all(&state.db)
    .await
    .map_err(internal)?;
    let mut per_version = serde_json::Map::new();
    for (version, web, api) in per_version_rows {
        per_version.insert(version, json!(web + api));
    }
    map.insert(
        "metrics".into(),
        json!({
            "downloads": {
                "total": cookbook.web_download_count + cookbook.api_download_count,
                "versions": per_version,
            },
            "followers": cookbook.followers,
            "collaborators": cookbook.collaborators,
        }),
    );

    Ok(Json(body))
}

pub async fn contingent(
    State(state): State<AppState>,
    axum::extract::Path(cookbook): axum::extract::Path<String>,
) -> Result<Json<Value>, ApiError> {
    let found = find_cookbook(&state.db, &cookbook)
        .await?
        .ok_or(ApiError::NotFound)?;
    let base = state.cfg.base_url();

    // `Cookbook#contingents` sorts by dependent cookbook name, then version.
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT c.name, v.version
           FROM cookbook_dependencies d
           JOIN cookbook_versions v ON v.id = d.cookbook_version_id
           JOIN cookbooks c ON c.id = v.cookbook_id
          WHERE d.cookbook_id = $1",
    )
    .bind(found.id)
    .fetch_all(&state.db)
    .await
    .map_err(internal)?;

    let mut parsed: Vec<(String, chef_version::Version, String)> = rows
        .into_iter()
        .map(|(name, version)| {
            let parsed = chef_version::Version::parse(&version)
                .map_err(|e| ApiError::InvalidData(vec![e.to_string()]))?;
            Ok((name, parsed, version))
        })
        .collect::<Result<_, ApiError>>()?;
    parsed.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.cmp(&b.1)));

    let items: Vec<Value> = parsed
        .into_iter()
        .map(|(name, _, version)| {
            json!({
                "name": name,
                "version": version,
                "url": version_url(&base, &name, &version),
            })
        })
        .collect();

    Ok(Json(json!({ "contingents": items })))
}
