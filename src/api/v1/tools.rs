//! `Api::V1::ToolsController`.

use axum::extract::{Path, State};
use axum::Json;
use serde_json::{json, Value};

use crate::api::v1::cookbooks::internal;
use crate::api::v1::params;
use crate::error::ApiError;
use crate::query::QueryMap;
use crate::state::AppState;

const ALLOWED_TYPES: [&str; 8] = [
    "knife_plugin",
    "ohai_plugin",
    "chef_infra_tool",
    "chef_infra_handler",
    "kitchen_driver",
    "powershell_module",
    "dsc_resource",
    "compliance_profile",
];

#[derive(sqlx::FromRow)]
struct ToolRow {
    name: String,
    slug: String,
    #[sqlx(rename = "type")]
    kind: String,
    source_url: Option<String>,
    description: Option<String>,
    owner: String,
}

const LIST_SELECT: &str = "SELECT t.name, t.slug, t.type, t.source_url, t.description,
                                  a.username AS owner
                             FROM tools t
                             JOIN users u ON u.id = t.user_id
                             JOIN accounts a ON a.user_id = u.id AND a.provider = 'chef_oauth2'";

fn item(state: &AppState, tool: &ToolRow) -> Value {
    json!({
        "tool_name": tool.name,
        "tool_type": tool.kind,
        "tool_source_url": tool.source_url,
        "tool_description": tool.description,
        "tool_owner": tool.owner,
        "tool": format!("{}/api/v1/tools/{}", state.cfg.base_url(), tool.slug),
    })
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
    let total: i64 = sqlx::query_scalar("SELECT count(*) FROM tools")
        .fetch_one(&state.db)
        .await
        .map_err(internal)?;

    let sql = format!(
        "{LIST_SELECT} ORDER BY {} LIMIT $1 OFFSET $2",
        params.order.tool_sql()
    );
    let rows: Vec<ToolRow> = sqlx::query_as(&sql)
        .bind(params.items)
        .bind(params.start)
        .fetch_all(&state.db)
        .await
        .map_err(internal)?;

    Ok(Json(json!({
        "start": params.start,
        "total": total,
        "items": rows.iter().map(|row| item(&state, row)).collect::<Vec<_>>(),
    })))
}

/// name, slug, type, source_url, description, instructions, up_for_adoption, owner
type ToolColumns = (
    String,
    String,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<bool>,
    String,
);

pub async fn show(
    State(state): State<AppState>,
    Path(slug): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let row: Option<ToolColumns> = sqlx::query_as(
        "SELECT t.name, t.slug, t.type, t.source_url, t.description, t.instructions,
                    t.up_for_adoption, a.username
               FROM tools t
               JOIN users u ON u.id = t.user_id
               JOIN accounts a ON a.user_id = u.id AND a.provider = 'chef_oauth2'
              WHERE t.slug = $1",
    )
    .bind(&slug)
    .fetch_optional(&state.db)
    .await
    .map_err(internal)?;

    let Some((name, slug, kind, source_url, description, instructions, up_for_adoption, owner)) =
        row
    else {
        return Err(ApiError::NotFound);
    };

    Ok(Json(json!({
        "name": name,
        "slug": slug,
        "type": kind,
        "source_url": source_url,
        "description": description,
        "instructions": instructions,
        "owner": owner,
        "up_for_adoption": up_for_adoption,
    })))
}

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

    let term = query.get("q").unwrap_or("").to_owned();
    let kind = query
        .get("type")
        .filter(|value| ALLOWED_TYPES.contains(value))
        .map(str::to_owned);

    let filter = "($1::text IS NULL OR t.type = $1)";
    let matching_sql = if term.is_empty() {
        format!("SELECT count(*) FROM tools t WHERE {filter}")
    } else {
        format!(
            "SELECT count(*) FROM tools t
               JOIN users u ON u.id = t.user_id
               JOIN accounts a ON a.user_id = u.id AND a.provider = 'chef_oauth2'
              WHERE {filter}
                AND (t.name % $2 OR (setweight(to_tsvector('english', coalesce(a.username, '')), 'B')
                     || setweight(to_tsvector('english', coalesce(t.description, '')), 'C'))
                    @@ $3::tsquery)"
        )
    };
    let mut count_query = sqlx::query_scalar::<_, i64>(&matching_sql).bind(&kind);
    if !term.is_empty() {
        count_query = count_query.bind(&term).bind(tsquery(&term));
    }
    let matching: i64 = count_query.fetch_one(&state.db).await.map_err(internal)?;

    // Upstream counts the already offset+limited relation, so `total` is the
    // number of rows actually returned.
    let total = matching.saturating_sub(params.start).min(params.items);

    let rows: Vec<ToolRow> = if term.is_empty() {
        let sql = format!(
            "{LIST_SELECT} WHERE {filter} ORDER BY {} LIMIT $2 OFFSET $3",
            params.order.tool_sql()
        );
        sqlx::query_as(&sql)
            .bind(&kind)
            .bind(params.items)
            .bind(params.start)
            .fetch_all(&state.db)
            .await
            .map_err(internal)?
    } else {
        let sql = format!(
            "SELECT name, slug, type, source_url, description, owner FROM (
               SELECT t.name, t.slug, t.type, t.source_url, t.description,
                      a.username AS owner,
                      similarity(t.name, $2)
                      + 0.5 * ts_rank(
                          setweight(to_tsvector('english', coalesce(a.username, '')), 'B')
                          || setweight(to_tsvector('english', coalesce(t.description, '')), 'C'),
                          $3::tsquery) AS rank
                 FROM tools t
                 JOIN users u ON u.id = t.user_id
                 JOIN accounts a ON a.user_id = u.id AND a.provider = 'chef_oauth2'
                WHERE {filter}
                  AND (t.name % $2 OR (setweight(to_tsvector('english', coalesce(a.username, '')), 'B')
                       || setweight(to_tsvector('english', coalesce(t.description, '')), 'C'))
                      @@ $3::tsquery)
             ) ranked
             ORDER BY rank DESC, name ASC LIMIT $4 OFFSET $5"
        );
        sqlx::query_as(&sql)
            .bind(&kind)
            .bind(&term)
            .bind(tsquery(&term))
            .bind(params.items)
            .bind(params.start)
            .fetch_all(&state.db)
            .await
            .map_err(internal)?
    };

    Ok(Json(json!({
        "start": params.start,
        "total": total,
        "items": rows.iter().map(|row| item(&state, row)).collect::<Vec<_>>(),
    })))
}
