//! `GroupMembersController`: adding and removing group members, promoting them
//! to group admins, and the route stubs whose actions the application leaves
//! undefined.

use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::response::Response;

use crate::state::AppState;
use crate::web;
use crate::web::collaborators;
use crate::web::pages;

/// Actions with a route but no action method raise `ActionNotFound`; Rails
/// turns that into a 404 exception page.
pub async fn missing(State(state): State<AppState>, headers: HeaderMap) -> Response {
    pages::action_not_found(&state, &headers).await
}

#[derive(Debug, Default)]
struct MemberForm {
    group_id: Option<i32>,
    user_ids: Option<String>,
}

fn parse_form(body: &[u8]) -> MemberForm {
    let mut form = MemberForm::default();
    for (key, value) in form_urlencoded::parse(body) {
        match key.as_ref() {
            "group_member[group_id]" => form.group_id = value.parse().ok(),
            "group_member[user_ids]" => form.user_ids = Some(value.into_owned()),
            _ => {}
        }
    }
    form
}

/// `POST /group_members`
pub async fn create(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let Some(_viewer) = web::viewer(&state, &headers).await else {
        return web::redirect_to_sign_in(&state, &headers, "/group_members");
    };
    let form = parse_form(&body);
    let group_id = form.group_id;
    let Some(group_path) = group_path(&state, group_id).await else {
        return pages::action_not_found(&state, &headers).await;
    };
    let user_ids = form
        .user_ids
        .as_deref()
        .map(str::trim)
        .unwrap_or_default()
        .to_owned();
    if user_ids.is_empty() {
        return web::redirect_with_flash(
            &state,
            &headers,
            &group_path,
            "warning",
            "At least one user must be added!",
        );
    }

    let resources: Vec<(String, i32)> = sqlx::query_as(
        "SELECT resourceable_type, resourceable_id FROM group_resources WHERE group_id = $1",
    )
    .bind(group_id)
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();

    let mut warning = String::new();
    for piece in user_ids.split(',') {
        let Ok(user_id) = piece.trim().parse::<i32>() else {
            continue;
        };
        let inserted: Option<i32> = sqlx::query_scalar(
            "INSERT INTO group_members (user_id, group_id, admin, created_at, updated_at)
             VALUES ($1, $2, NULL, NOW(), NOW()) RETURNING id",
        )
        .bind(user_id)
        .bind(group_id)
        .fetch_optional(&state.db)
        .await
        .ok()
        .flatten();
        match inserted {
            Some(_) => {
                for (kind, resource_id) in &resources {
                    collaborators::add_user_as_collaborator(
                        &state,
                        kind,
                        *resource_id,
                        &user_id.to_string(),
                        Some(group_id.unwrap_or_default()),
                    )
                    .await;
                }
            }
            None => {
                warning.push_str("User can't be blank");
                break;
            }
        }
    }
    let (kind, message) = if warning.is_empty() {
        ("notice", "Members successfully added!".to_owned())
    } else {
        ("warning", warning)
    };
    web::redirect_with_flash(&state, &headers, &group_path, kind, &message)
}

/// `DELETE /group_members/:id`
pub async fn destroy(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<i32>,
) -> Response {
    let Some(_viewer) = web::viewer(&state, &headers).await else {
        return web::redirect_to_sign_in(&state, &headers, "/group_members");
    };
    let Some(member) = find_member(&state, id).await else {
        return pages::action_not_found(&state, &headers).await;
    };
    let path = format!("/groups/{}", member.group_id);
    // `check_admin_member_present`: a group keeps at least one admin.
    if member.admin {
        let admins: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM group_members WHERE group_id = $1 AND admin")
                .bind(member.group_id)
                .fetch_one(&state.db)
                .await
                .unwrap_or(0);
        if admins <= 1 {
            return web::redirect_with_flash(
                &state,
                &headers,
                &path,
                "warning",
                "Member could not be removed because a group must have at least one admin member",
            );
        }
    }
    if let Err(_error) = sqlx::query("DELETE FROM group_members WHERE id = $1")
        .bind(id)
        .execute(&state.db)
        .await
    {
        return web::redirect_with_flash(
            &state,
            &headers,
            &path,
            "warning",
            "An error has occurred",
        );
    }
    let resources: Vec<(String, i32)> = sqlx::query_as(
        "SELECT resourceable_type, resourceable_id FROM group_resources WHERE group_id = $1",
    )
    .bind(member.group_id)
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();
    for (kind, resource_id) in resources {
        collaborators::remove_user_from_resource(&state, &kind, resource_id, member.user_id).await;
    }
    web::redirect_with_flash(
        &state,
        &headers,
        &path,
        "notice",
        "Member successfully removed",
    )
}

/// `POST /group_members/:id/make_admin`
pub async fn make_admin(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<i32>,
) -> Response {
    let Some(viewer) = web::viewer(&state, &headers).await else {
        return web::redirect_to_sign_in(&state, &headers, "/group_members");
    };
    let Some(member) = find_member(&state, id).await else {
        return pages::action_not_found(&state, &headers).await;
    };
    let path = format!("/groups/{}", member.group_id);
    let caller_admin: Option<i32> = sqlx::query_scalar(
        "SELECT id FROM group_members WHERE group_id = $1 AND user_id = $2 AND admin",
    )
    .bind(member.group_id)
    .bind(viewer.id)
    .fetch_optional(&state.db)
    .await
    .ok()
    .flatten();
    if caller_admin.is_none() {
        return web::redirect_with_flash(
            &state,
            &headers,
            &path,
            "error",
            "You must be an admin member of the group to do that.",
        );
    }
    let _ = sqlx::query("UPDATE group_members SET admin = TRUE, updated_at = NOW() WHERE id = $1")
        .bind(id)
        .execute(&state.db)
        .await;
    web::redirect_with_flash(
        &state,
        &headers,
        &path,
        "notice",
        "Member has successfully been made an admin!",
    )
}

struct Member {
    group_id: i32,
    user_id: i32,
    admin: bool,
}

async fn find_member(state: &AppState, id: i32) -> Option<Member> {
    let row: Option<(i32, i32, Option<bool>)> =
        sqlx::query_as("SELECT group_id, user_id, admin FROM group_members WHERE id = $1")
            .bind(id)
            .fetch_optional(&state.db)
            .await
            .ok()
            .flatten();
    row.map(|(group_id, user_id, admin)| Member {
        group_id,
        user_id,
        admin: admin.unwrap_or(false),
    })
}

async fn group_path(state: &AppState, group_id: Option<i32>) -> Option<String> {
    let group_id = group_id?;
    let found: Option<i32> = sqlx::query_scalar("SELECT id FROM groups WHERE id = $1")
        .bind(group_id)
        .fetch_optional(&state.db)
        .await
        .ok()
        .flatten();
    found.map(|id| format!("/groups/{id}"))
}
