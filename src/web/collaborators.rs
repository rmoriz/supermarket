//! `CollaboratorsController`: the collaborator picker JSON plus add, remove,
//! group removal and ownership transfer between collaborators.

use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::locales::t;
use crate::state::AppState;
use crate::web;
use crate::web::pages;

/// `?q=&ineligible_user_ids[]=`. The bracket suffix is decoded as a single
/// string by `serde_urlencoded`, so the pairs are collected first.
#[derive(Debug, Default)]
pub struct IndexParams {
    pub ineligible: Vec<String>,
    pub q: Option<String>,
}

impl IndexParams {
    fn from_pairs(pairs: Vec<(String, String)>) -> Self {
        let mut params = IndexParams::default();
        for (name, value) in pairs {
            match name.as_str() {
                "ineligible_user_ids[]" | "ineligible_user_ids" => params.ineligible.push(value),
                "q" => params.q = Some(value),
                _ => {}
            }
        }
        params
    }
}

#[derive(Serialize, sqlx::FromRow)]
struct Candidate {
    id: i32,
    first_name: Option<String>,
    last_name: Option<String>,
    username: String,
}

/// `GET /collaborators?q=&ineligible_user_ids[]=`: people who could still be
/// added, as JSON.
pub async fn index(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(pairs): Query<Vec<(String, String)>>,
) -> Response {
    let params = IndexParams::from_pairs(pairs);
    if web::viewer(&state, &headers).await.is_none() {
        return web::redirect_with_flash(
            &state,
            &headers,
            "/sign-in",
            "notice",
            &t("user.must_be_signed_in", &[]),
        );
    }
    let ineligible: Vec<i32> = params
        .ineligible
        .iter()
        .filter_map(|id| id.parse().ok())
        .collect();
    // `includes(:chef_account)`: users without one still appear (blank username).
    // `where.not(id: [])` is a no-op, so an empty list must not exclude anybody.
    let rows: Vec<Candidate> = match params.q.as_deref() {
        Some(q) if !q.is_empty() => sqlx::query_as(
            "SELECT u.id, u.first_name, u.last_name, COALESCE(a.username, '') AS username
               FROM users u LEFT JOIN accounts a ON a.user_id = u.id AND a.provider = 'chef_oauth2'
              WHERE (cardinality($1::int[]) = 0 OR NOT (u.id = ANY($1)))
                AND a.username ILIKE $2
              ORDER BY a.username LIMIT 20",
        )
        .bind(&ineligible)
        .bind(format!("%{q}%"))
        .fetch_all(&state.db)
        .await
        .unwrap_or_default(),
        _ => sqlx::query_as(
            "SELECT u.id, u.first_name, u.last_name, COALESCE(a.username, '') AS username
               FROM users u LEFT JOIN accounts a ON a.user_id = u.id AND a.provider = 'chef_oauth2'
              WHERE cardinality($1::int[]) = 0 OR NOT (u.id = ANY($1))
              ORDER BY u.id LIMIT 20",
        )
        .bind(&ineligible)
        .fetch_all(&state.db)
        .await
        .unwrap_or_default(),
    };
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json; charset=utf-8")],
        json!({ "collaborators": rows }).to_string(),
    )
        .into_response()
}

#[derive(Debug, Default, Deserialize)]
pub struct CreateParams {
    pub collaborator: Option<CreateFields>,
}

#[derive(Debug, Default, Deserialize)]
pub struct CreateFields {
    pub resourceable_type: Option<String>,
    pub resourceable_id: Option<String>,
    pub user_ids: Option<String>,
    pub group_ids: Option<String>,
}

struct Resource {
    id: i32,
    path: String,
}

/// Resolves the resource a collaborator form points at (`Cookbook`/`Tool`).
async fn resource(
    state: &AppState,
    kind: Option<&str>,
    id: Option<i32>,
) -> Option<(String, Resource)> {
    let id = id?;
    match kind? {
        "Cookbook" => {
            let name: String = sqlx::query_scalar("SELECT name FROM cookbooks WHERE id = $1")
                .bind(id)
                .fetch_optional(&state.db)
                .await
                .ok()
                .flatten()?;
            Some((
                "Cookbook".into(),
                Resource {
                    id,
                    path: format!("/cookbooks/{name}"),
                },
            ))
        }
        "Tool" => {
            let slug: String = sqlx::query_scalar("SELECT slug FROM tools WHERE id = $1")
                .bind(id)
                .fetch_optional(&state.db)
                .await
                .ok()
                .flatten()?;
            Some((
                "Tool".into(),
                Resource {
                    id,
                    path: format!("/tools/{slug}"),
                },
            ))
        }
        _ => None,
    }
}

/// Columns the added-email job serializes for the fresh collaborator row.
type CollaboratorRecord = (
    i32,
    Option<i32>,
    Option<i32>,
    Option<chrono::NaiveDateTime>,
    Option<chrono::NaiveDateTime>,
    Option<String>,
    Option<i32>,
);

/// Users already on the resource (owner included) cannot be added again.
async fn ineligible_ids(state: &AppState, kind: &str, id: i32) -> Vec<i32> {
    let owner_table = if kind == "Cookbook" {
        "cookbooks"
    } else {
        "tools"
    };
    sqlx::query_scalar(&format!(
        "SELECT user_id FROM {owner_table} WHERE id = $1
         UNION SELECT user_id FROM collaborators
          WHERE resourceable_type = $2 AND resourceable_id = $1 AND group_id IS NULL"
    ))
    .bind(id)
    .bind(kind)
    .fetch_all(&state.db)
    .await
    .unwrap_or_default()
}

/// `add_users_as_collaborators`: skips ineligible users, inserts the rest and
/// queues the added-email job.
pub async fn add_users(
    state: &AppState,
    kind: &str,
    id: i32,
    user_ids: &str,
    group_id: Option<i32>,
) {
    let ineligible = if group_id.is_none() {
        ineligible_ids(state, kind, id).await
    } else {
        Vec::new()
    };
    for piece in user_ids.split(',') {
        let Ok(user_id) = piece.trim().parse::<i32>() else {
            continue;
        };
        if ineligible.contains(&user_id) {
            continue;
        }
        let inserted: Option<i32> = sqlx::query_scalar(
            "INSERT INTO collaborators
                (resourceable_id, resourceable_type, user_id, group_id, created_at, updated_at)
             VALUES ($1, $2, $3, $4, NOW(), NOW()) RETURNING id",
        )
        .bind(id)
        .bind(kind)
        .bind(user_id)
        .bind(group_id)
        .fetch_optional(&state.db)
        .await
        .ok()
        .flatten();
        if let Some(collaborator_id) = inserted {
            let record: Option<CollaboratorRecord> = sqlx::query_as(
                "SELECT id, resourceable_id, user_id, created_at, updated_at,
                        resourceable_type, group_id
                   FROM collaborators WHERE id = $1",
            )
            .bind(collaborator_id)
            .fetch_optional(&state.db)
            .await
            .ok()
            .flatten();
            if let Some((id, resource_id, user_id, created_at, updated_at, kind, group_id)) = record
            {
                use crate::jobs::{MailerArg, RecordArg, RecordValue};
                let timestamp = |value: Option<chrono::NaiveDateTime>| {
                    value
                        .map(|value| {
                            RecordValue::Timestamp(
                                value.format("%Y-%m-%d %H:%M:%S%.9f Z").to_string(),
                            )
                        })
                        .unwrap_or(RecordValue::Null)
                };
                let _ = crate::jobs::perform_mailer(
                    state,
                    "CollaboratorMailer",
                    "added_email",
                    vec![MailerArg::Record(RecordArg {
                        class: "Collaborator",
                        attributes: vec![
                            ("id", RecordValue::Int(id)),
                            (
                                "resourceable_id",
                                resource_id
                                    .map(RecordValue::Int)
                                    .unwrap_or(RecordValue::Null),
                            ),
                            (
                                "user_id",
                                user_id.map(RecordValue::Int).unwrap_or(RecordValue::Null),
                            ),
                            ("created_at", timestamp(created_at)),
                            ("updated_at", timestamp(updated_at)),
                            (
                                "resourceable_type",
                                kind.map(RecordValue::Text).unwrap_or(RecordValue::Null),
                            ),
                            (
                                "group_id",
                                group_id.map(RecordValue::Int).unwrap_or(RecordValue::Null),
                            ),
                        ],
                    })],
                )
                .await;
            }
        }
    }
}

/// `add_users_as_collaborators(resource, user_id, group_id)` for one user.
pub async fn add_user_as_collaborator(
    state: &AppState,
    kind: &str,
    id: i32,
    user_id: &str,
    group_id: Option<i32>,
) {
    add_users(state, kind, id, user_id, group_id).await;
}

/// `remove_collaborator`: drop the row and re-run the quality check for
/// cookbooks.
pub async fn remove_user_from_resource(state: &AppState, kind: &str, id: i32, user_id: i32) {
    let collaborator_id: Option<i32> = sqlx::query_scalar(
        "SELECT id FROM collaborators
          WHERE resourceable_type = $1 AND resourceable_id = $2 AND user_id = $3
          LIMIT 1",
    )
    .bind(kind)
    .bind(id)
    .bind(user_id)
    .fetch_optional(&state.db)
    .await
    .ok()
    .flatten();
    let Some(collaborator_id) = collaborator_id else {
        return;
    };
    let _ = sqlx::query("DELETE FROM collaborators WHERE id = $1")
        .bind(collaborator_id)
        .execute(&state.db)
        .await;
    if kind == "Cookbook" && state.cfg.feature_active("fieri") && state.cfg.fieri_url.is_some() {
        if let Some(version_id) = latest_version_id(state, id).await {
            let _ = crate::jobs::perform_async(state, "FieriNotifyWorker", vec![json!(version_id)])
                .await;
        }
    }
}

/// `GET /collaborators/new`: routed, but only the JSON template exists.
pub async fn new(State(state): State<AppState>, headers: HeaderMap) -> Response {
    pages::action_not_found(&state, &headers).await
}

/// `POST /collaborators`: add users and groups to a cookbook or tool.
pub async fn create(
    State(state): State<AppState>,
    headers: HeaderMap,
    axum::extract::Form(pairs): axum::extract::Form<Vec<(String, String)>>,
) -> Response {
    let viewer = web::viewer(&state, &headers).await;
    let Some(viewer) = viewer else {
        return web::redirect_with_flash(
            &state,
            &headers,
            "/sign-in",
            "notice",
            &t("user.must_be_signed_in", &[]),
        );
    };
    let fields = web::from_pairs::<CreateParams>(pairs)
        .and_then(|params| params.collaborator)
        .unwrap_or_default();
    let resourceable_id = fields
        .resourceable_id
        .as_deref()
        .and_then(|id| id.parse().ok());
    let Some((kind, target)) =
        resource(&state, fields.resourceable_type.as_deref(), resourceable_id).await
    else {
        return pages::error_page_as(
            &state,
            &headers,
            StatusCode::NOT_FOUND,
            false,
            "collaborators",
        )
        .await;
    };
    let owner_id: Option<i32> = if kind == "Cookbook" {
        sqlx::query_scalar("SELECT user_id FROM cookbooks WHERE id = $1")
            .bind(target.id)
            .fetch_optional(&state.db)
            .await
            .ok()
            .flatten()
    } else {
        sqlx::query_scalar("SELECT user_id FROM tools WHERE id = $1")
            .bind(target.id)
            .fetch_optional(&state.db)
            .await
            .ok()
            .flatten()
    };
    if owner_id != Some(viewer.id) && !viewer.admin {
        return pages::error_page_as(
            &state,
            &headers,
            StatusCode::NOT_FOUND,
            false,
            "collaborators",
        )
        .await;
    }
    if let Some(user_ids) = fields.user_ids.as_deref().filter(|ids| !ids.is_empty()) {
        add_users(&state, &kind, target.id, user_ids, None).await;
    }
    if let Some(group_ids) = fields.group_ids.as_deref().filter(|ids| !ids.is_empty()) {
        for group_id in group_ids.split(',') {
            let Ok(group_id) = group_id.trim().parse::<i32>() else {
                continue;
            };
            let members: Vec<i32> =
                sqlx::query_scalar("SELECT user_id FROM group_members WHERE group_id = $1")
                    .bind(group_id)
                    .fetch_all(&state.db)
                    .await
                    .unwrap_or_default();
            let joined = members
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(",");
            add_users(&state, &kind, target.id, &joined, Some(group_id)).await;
            let _ = sqlx::query(
                "INSERT INTO group_resources
                    (group_id, resourceable_id, resourceable_type, created_at, updated_at)
                 VALUES ($1, $2, $3, NOW(), NOW())",
            )
            .bind(group_id)
            .bind(target.id)
            .bind(&kind)
            .execute(&state.db)
            .await;
        }
    }
    if kind == "Cookbook" && state.cfg.feature_active("fieri") && state.cfg.fieri_url.is_some() {
        if let Some(version_id) = latest_version_id(&state, target.id).await {
            let _ =
                crate::jobs::perform_async(&state, "FieriNotifyWorker", vec![json!(version_id)])
                    .await;
        }
    }
    let notice = t("collaborator.added", &[]);
    web::redirect_with_flash(&state, &headers, &target.path, "notice", &notice)
}

async fn latest_version_id(state: &AppState, cookbook_id: i32) -> Option<i32> {
    sqlx::query_scalar(
        "SELECT id FROM cookbook_versions WHERE cookbook_id = $1 ORDER BY id DESC LIMIT 1",
    )
    .bind(cookbook_id)
    .fetch_optional(&state.db)
    .await
    .ok()
    .flatten()
}

#[derive(sqlx::FromRow)]
struct CollaboratorRow {
    resourceable_type: Option<String>,
    resourceable_id: Option<i32>,
    user_id: Option<i32>,
}

async fn find_collaborator(state: &AppState, id: i32) -> Option<CollaboratorRow> {
    sqlx::query_as(
        "SELECT resourceable_type, resourceable_id, user_id FROM collaborators WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await
    .ok()
    .flatten()
}

/// `DELETE /collaborators/:id`: remove one collaborator (owner/admin, or the
/// collaborator themself).
pub async fn destroy(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<i32>,
) -> Response {
    let Some(viewer) = web::viewer(&state, &headers).await else {
        return web::redirect_with_flash(
            &state,
            &headers,
            "/sign-in",
            "notice",
            &t("user.must_be_signed_in", &[]),
        );
    };
    let Some(collaborator) = find_collaborator(&state, id).await else {
        return pages::error_page_as(
            &state,
            &headers,
            StatusCode::NOT_FOUND,
            false,
            "collaborators",
        )
        .await;
    };
    let kind = collaborator.resourceable_type.clone().unwrap_or_default();
    let resource_id = collaborator.resourceable_id.unwrap_or_default();
    let owner_table = if kind == "Cookbook" {
        "cookbooks"
    } else {
        "tools"
    };
    let owner_id: Option<i32> =
        sqlx::query_scalar(&format!("SELECT user_id FROM {owner_table} WHERE id = $1"))
            .bind(resource_id)
            .fetch_optional(&state.db)
            .await
            .ok()
            .flatten();
    let allowed =
        viewer.admin || owner_id == Some(viewer.id) || collaborator.user_id == Some(viewer.id);
    if !allowed {
        return pages::error_page_as(
            &state,
            &headers,
            StatusCode::NOT_FOUND,
            false,
            "collaborators",
        )
        .await;
    }
    let _ = sqlx::query("DELETE FROM collaborators WHERE id = $1")
        .bind(id)
        .execute(&state.db)
        .await;
    if kind == "Cookbook" && state.cfg.feature_active("fieri") && state.cfg.fieri_url.is_some() {
        if let Some(version_id) = latest_version_id(&state, resource_id).await {
            let _ =
                crate::jobs::perform_async(&state, "FieriNotifyWorker", vec![json!(version_id)])
                    .await;
        }
    }
    // `head 200` inside `format.js`: an empty body with the format's type.
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/javascript")],
        "",
    )
        .into_response()
}

#[derive(Debug, Default, Deserialize)]
pub struct GroupParams {
    pub resourceable_type: Option<String>,
    pub resourceable_id: Option<String>,
}

/// `DELETE /collaborators/:id/destroy_group`: drop a whole group.
pub async fn destroy_group(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(group_id): Path<i32>,
    Query(params): Query<GroupParams>,
) -> Response {
    let Some(viewer) = web::viewer(&state, &headers).await else {
        return web::redirect_with_flash(
            &state,
            &headers,
            "/sign-in",
            "notice",
            &t("user.must_be_signed_in", &[]),
        );
    };
    let group: Option<(i32, String)> = sqlx::query_as("SELECT id, name FROM groups WHERE id = $1")
        .bind(group_id)
        .fetch_optional(&state.db)
        .await
        .ok()
        .flatten();
    let Some((group_id, group_name)) = group else {
        return pages::error_page_as(
            &state,
            &headers,
            StatusCode::NOT_FOUND,
            false,
            "collaborators",
        )
        .await;
    };
    let resourceable_id = params
        .resourceable_id
        .as_deref()
        .and_then(|id| id.parse().ok());
    let Some((kind, target)) =
        resource(&state, params.resourceable_type.as_deref(), resourceable_id).await
    else {
        return pages::error_page_as(
            &state,
            &headers,
            StatusCode::NOT_FOUND,
            false,
            "collaborators",
        )
        .await;
    };
    let _ = viewer;
    let group_members: Vec<i32> = sqlx::query_scalar(
        "SELECT user_id FROM collaborators
          WHERE resourceable_type = $1 AND resourceable_id = $2 AND group_id = $3",
    )
    .bind(&kind)
    .bind(target.id)
    .bind(group_id)
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();
    let _ = sqlx::query(
        "DELETE FROM collaborators
          WHERE resourceable_type = $1 AND resourceable_id = $2 AND group_id = $3",
    )
    .bind(&kind)
    .bind(target.id)
    .bind(group_id)
    .execute(&state.db)
    .await;
    let _ = sqlx::query(
        "DELETE FROM group_resources
          WHERE group_id = $1 AND resourceable_type = $2 AND resourceable_id = $3",
    )
    .bind(group_id)
    .bind(&kind)
    .bind(target.id)
    .execute(&state.db)
    .await;

    let mut notice = format!(
        "{} ",
        t("collaborator.group_removed", &[("name", &group_name)])
    );
    for user_id in group_members {
        let still: Vec<(Option<i32>, Option<String>)> = sqlx::query_as(
            "SELECT col.group_id, g.name FROM collaborators col
               LEFT JOIN groups g ON g.id = col.group_id
              WHERE col.resourceable_type = $1 AND col.resourceable_id = $2 AND col.user_id = $3",
        )
        .bind(&kind)
        .bind(target.id)
        .bind(user_id)
        .fetch_all(&state.db)
        .await
        .unwrap_or_default();
        let username: Option<String> = sqlx::query_scalar(
            "SELECT username FROM accounts WHERE user_id = $1 AND provider = 'chef_oauth2' LIMIT 1",
        )
        .bind(user_id)
        .fetch_optional(&state.db)
        .await
        .ok()
        .flatten();
        let username = username.unwrap_or_default();
        for (group, name) in still {
            match (group, name) {
                (Some(_), Some(name)) => {
                    notice.push_str(&format!(
                        "{username} is still a collaborator associated with {name} "
                    ));
                }
                _ => notice.push_str(&format!(
                    "{username} is still a collaborator on this {kind} "
                )),
            }
        }
    }
    web::redirect_with_flash(&state, &headers, &target.path, "notice", &notice)
}

#[derive(sqlx::FromRow)]
struct TransferRow {
    resourceable_type: Option<String>,
    resourceable_id: Option<i32>,
    user_id: Option<i32>,
    resource_name: Option<String>,
    resource_path: Option<String>,
    username: Option<String>,
}

async fn find_transfer(state: &AppState, id: i32) -> Option<TransferRow> {
    sqlx::query_as(
        "SELECT col.resourceable_type, col.resourceable_id, col.user_id,
                COALESCE(c.name, t.name) AS resource_name,
                COALESCE('/cookbooks/' || c.name, '/tools/' || t.slug) AS resource_path,
                a.username
           FROM collaborators col
           LEFT JOIN cookbooks c ON col.resourceable_type = 'Cookbook' AND c.id = col.resourceable_id
           LEFT JOIN tools t ON col.resourceable_type = 'Tool' AND t.id = col.resourceable_id
           LEFT JOIN accounts a ON a.user_id = col.user_id AND a.provider = 'chef_oauth2'
          WHERE col.id = $1",
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await
    .ok()
    .flatten()
}

/// `PUT /collaborators/:id/transfer`: the collaborator becomes owner.
pub async fn transfer(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<i32>,
) -> Response {
    let Some(viewer) = web::viewer(&state, &headers).await else {
        return web::redirect_with_flash(
            &state,
            &headers,
            "/sign-in",
            "notice",
            &t("user.must_be_signed_in", &[]),
        );
    };
    let Some(collaborator) = find_transfer(&state, id).await else {
        return pages::error_page_as(
            &state,
            &headers,
            StatusCode::NOT_FOUND,
            false,
            "collaborators",
        )
        .await;
    };
    let kind = collaborator.resourceable_type.clone().unwrap_or_default();
    let resource_id = collaborator.resourceable_id.unwrap_or_default();
    let owner_table = if kind == "Cookbook" {
        "cookbooks"
    } else {
        "tools"
    };
    let owner_id: Option<i32> =
        sqlx::query_scalar(&format!("SELECT user_id FROM {owner_table} WHERE id = $1"))
            .bind(resource_id)
            .fetch_optional(&state.db)
            .await
            .ok()
            .flatten();
    if owner_id != Some(viewer.id) && !viewer.admin {
        return pages::error_page_as(
            &state,
            &headers,
            StatusCode::NOT_FOUND,
            false,
            "collaborators",
        )
        .await;
    }
    if let Some(previous_owner) = owner_id {
        let _ = sqlx::query(
            "INSERT INTO collaborators
                (resourceable_id, resourceable_type, user_id, created_at, updated_at)
             VALUES ($1, $2, $3, NOW(), NOW())",
        )
        .bind(resource_id)
        .bind(&kind)
        .bind(previous_owner)
        .execute(&state.db)
        .await;
    }
    let _ = sqlx::query(&format!(
        "UPDATE {owner_table} SET user_id = $2 WHERE id = $1"
    ))
    .bind(resource_id)
    .bind(collaborator.user_id)
    .execute(&state.db)
    .await;
    let _ = sqlx::query("DELETE FROM collaborators WHERE id = $1")
        .bind(id)
        .execute(&state.db)
        .await;
    let notice = t(
        "collaborator.owner_changed",
        &[
            (
                "resource",
                collaborator.resource_name.as_deref().unwrap_or(""),
            ),
            ("user", collaborator.username.as_deref().unwrap_or("")),
        ],
    );
    let path = collaborator.resource_path.unwrap_or_else(|| "/".into());
    web::redirect_with_flash(&state, &headers, &path, "notice", &notice)
}

#[cfg(test)]
mod tests {
    #[test]
    fn notice_shape() {
        let value = crate::locales::t("collaborator.group_removed", &[("name", "operators")]);
        assert_eq!(value, "Collaborator group operators successfully removed");
    }
}
