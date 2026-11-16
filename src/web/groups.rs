//! `GroupsController` read paths: the JSON index, the group page and the new
//! group form.

use crate::web::filters;
use askama::Template;
use axum::extract::{Form, Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::json;

use crate::state::AppState;
use crate::web::pages;
use crate::web::{self, Page, Shell};

#[derive(Debug, Default, Deserialize)]
pub struct IndexParams {
    pub q: Option<String>,
}

#[derive(Deserialize, sqlx::FromRow)]
struct GroupRow {
    id: i32,
    name: String,
}

const FEATURE_MESSAGE: &str = "You must activate the collaborator_groups feature to create a group";

fn feature_off(state: &AppState, headers: &HeaderMap) -> Option<Response> {
    (!state.cfg.feature_active("collaborator_groups")).then(|| {
        web::redirect_with_flash(state, headers, "/groups/new", "warning", FEATURE_MESSAGE)
    })
}

pub async fn index(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(params): Query<IndexParams>,
) -> Response {
    if let Some(response) = feature_off(&state, &headers) {
        return response;
    }
    let rows: Vec<GroupRow> = if let Some(q) = params.q.as_deref().filter(|q| !q.is_empty()) {
        sqlx::query_as("SELECT id, name FROM groups WHERE name ILIKE $1 ORDER BY id")
            .bind(format!("%{q}%"))
            .fetch_all(&state.db)
            .await
            .unwrap_or_default()
    } else {
        sqlx::query_as("SELECT id, name FROM groups ORDER BY id")
            .fetch_all(&state.db)
            .await
            .unwrap_or_default()
    };
    let body = json!(rows
        .iter()
        .map(|row| json!({ "id": row.id, "name": row.name }))
        .collect::<Vec<_>>())
    .to_string();
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json; charset=utf-8")],
        body,
    )
        .into_response()
}

pub struct Member {
    pub id: i32,
    pub username: String,
    pub admin: bool,
}

pub struct Resource {
    pub name: String,
    pub path: String,
}

pub struct Group {
    pub id: i32,
    pub name: String,
}

#[derive(Template)]
#[template(path = "groups/show.html")]
struct Show<'a> {
    shell: Shell<'a>,
    group: Group,
    admin_members: Vec<Member>,
    members: Vec<Member>,
    resources: Vec<Resource>,
    viewer_is_admin: bool,
    admin_count: usize,
}

pub async fn show(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<i32>,
) -> Response {
    // `before_action :collaborator_groups_feature_check` runs before the lookup.
    if let Some(response) = feature_off(&state, &headers) {
        return response;
    }
    let row: Option<(i32, String)> = sqlx::query_as("SELECT id, name FROM groups WHERE id = $1")
        .bind(id)
        .fetch_optional(&state.db)
        .await
        .ok()
        .flatten();
    let Some((id, name)) = row else {
        return pages::error_page_as(&state, &headers, StatusCode::NOT_FOUND, false, "groups")
            .await;
    };
    let viewer = web::viewer(&state, &headers).await;
    let rows: Vec<(i32, String, Option<bool>, i32)> = sqlx::query_as(
        "SELECT gm.id, a.username, gm.admin, u.id
           FROM group_members gm JOIN users u ON u.id = gm.user_id
           JOIN accounts a ON a.user_id = u.id AND a.provider = 'chef_oauth2'
          WHERE gm.group_id = $1 ORDER BY gm.id",
    )
    .bind(id)
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();
    let mut admin_members = Vec::new();
    let mut members = Vec::new();
    let mut viewer_is_admin = false;
    for (member_id, username, admin, user_id) in rows {
        let is_admin = admin.unwrap_or(false);
        if is_admin && viewer.as_ref().map(|v| v.id) == Some(user_id) {
            viewer_is_admin = true;
        }
        let member = Member {
            id: member_id,
            username,
            admin: is_admin,
        };
        if is_admin {
            admin_members.push(member);
        } else {
            members.push(member);
        }
    }
    let resources: Vec<(String, String, i32)> = sqlx::query_as(
        "SELECT resourceable_type, '', resourceable_id FROM group_resources WHERE group_id = $1",
    )
    .bind(id)
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();
    let mut resource_list = Vec::new();
    for (kind, _, resource_id) in resources {
        if kind == "Cookbook" {
            if let Some(name) =
                sqlx::query_scalar::<_, String>("SELECT name FROM cookbooks WHERE id = $1")
                    .bind(resource_id)
                    .fetch_optional(&state.db)
                    .await
                    .ok()
                    .flatten()
            {
                resource_list.push(Resource {
                    path: format!("/cookbooks/{name}"),
                    name,
                });
            }
        } else if let Some(name) =
            sqlx::query_scalar::<_, String>("SELECT name FROM tools WHERE id = $1")
                .bind(resource_id)
                .fetch_optional(&state.db)
                .await
                .ok()
                .flatten()
        {
            resource_list.push(Resource {
                path: format!("/tools/{name}"),
                name,
            });
        }
    }
    let admin_count = admin_members.len();
    let shell = pages::shell(&state, viewer.as_ref(), &headers)
        .title(name.clone())
        .description(name.clone());
    let page = Show {
        shell,
        group: Group { id, name },
        admin_members,
        members,
        resources: resource_list,
        viewer_is_admin,
        admin_count,
    };
    Page::ok(page).into_response()
}

#[derive(Template)]
#[template(path = "groups/new.html")]
struct New<'a> {
    shell: Shell<'a>,
}

pub async fn new(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Some(response) = feature_off(&state, &headers) {
        return response;
    }
    let viewer = web::viewer(&state, &headers).await;
    let shell = pages::shell(&state, viewer.as_ref(), &headers).title("New Group");
    Page::ok(New { shell }).into_response()
}

#[derive(Debug, Default, Deserialize)]
pub struct CreateParams {
    pub name: Option<String>,
}

/// `POST /groups`: the creator also becomes the group's first admin.
pub async fn create(
    State(state): State<AppState>,
    headers: HeaderMap,
    Form(pairs): Form<Vec<(String, String)>>,
) -> Response {
    if let Some(response) = feature_off(&state, &headers) {
        return response;
    }
    let Some(viewer) = web::viewer(&state, &headers).await else {
        return web::redirect_to_sign_in(&state, &headers, "/groups/new");
    };
    let params = CreateParams {
        name: pairs
            .into_iter()
            .find(|(key, _)| key == "group[name]")
            .map(|(_, value)| value),
    };
    let name = params.name.unwrap_or_default();
    let inserted: Option<i32> = sqlx::query_scalar(
        "INSERT INTO groups (name, created_at, updated_at) VALUES ($1, NOW(), NOW()) RETURNING id",
    )
    .bind(&name)
    .fetch_optional(&state.db)
    .await
    .ok()
    .flatten();
    let Some(group_id) = inserted else {
        // `Group.new` fails on a blank name; validations surface the messages.
        let message = if name.trim().is_empty() {
            "An error has occurred Name can't be blank"
        } else {
            "An error has occurred"
        };
        return web::redirect_with_flash(&state, &headers, "/groups/new", "warning", message);
    };
    let _ = sqlx::query(
        "INSERT INTO group_members (group_id, user_id, admin, created_at, updated_at)
         VALUES ($1, $2, TRUE, NOW(), NOW())",
    )
    .bind(group_id)
    .bind(viewer.id)
    .execute(&state.db)
    .await;
    web::redirect_with_flash(
        &state,
        &headers,
        &format!("/groups/{group_id}"),
        "notice",
        "Group successfully created!",
    )
}

/// `GET /groups/:id/edit`: routed, but the controller defines no action and
/// there is no template.
pub async fn edit(State(state): State<AppState>, headers: HeaderMap) -> Response {
    pages::action_not_found(&state, &headers).await
}

/// `PATCH|PUT /groups/:id`
pub async fn update(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(_id): Path<i32>,
) -> Response {
    pages::action_not_found(&state, &headers).await
}

/// `DELETE /groups/:id`
pub async fn destroy(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(_id): Path<i32>,
) -> Response {
    pages::action_not_found(&state, &headers).await
}
