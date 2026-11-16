//! `TransferOwnershipController`: immediate or emailed cookbook ownership
//! transfer, plus the accept/decline links from the email.

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use serde::Deserialize;

use crate::locales::t;
use crate::state::AppState;
use crate::web;
use crate::web::pages;

#[derive(Debug, Default, Deserialize)]
pub struct TransferParams {
    pub cookbook: Option<TransferFields>,
}

#[derive(Debug, Default, Deserialize)]
pub struct TransferFields {
    pub user_id: Option<String>,
    pub add_owner_as_collaborator: Option<String>,
}

/// `PUT /cookbooks/:id/transfer_ownership`: owner/admin only. A recipient who
/// is already a collaborator (or an admin initiator) takes over immediately;
/// anyone else gets an emailed request instead.
pub async fn transfer(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(name): Path<String>,
    axum::extract::Form(pairs): axum::extract::Form<Vec<(String, String)>>,
) -> Response {
    let cookbook: Option<(i32, String, i32)> =
        sqlx::query_as("SELECT id, name, user_id FROM cookbooks WHERE lowercase_name = $1")
            .bind(name.to_lowercase())
            .fetch_optional(&state.db)
            .await
            .ok()
            .flatten();
    let Some((cookbook_id, cookbook_name, owner_id)) = cookbook else {
        return pages::error_page_as(
            &state,
            &headers,
            StatusCode::NOT_FOUND,
            false,
            "transfer_ownership",
        )
        .await;
    };
    let viewer = web::viewer(&state, &headers).await;
    let authorized = viewer
        .as_ref()
        .map(|v| v.id == owner_id || v.admin)
        .unwrap_or(false);
    if !authorized {
        return pages::error_page_as(
            &state,
            &headers,
            StatusCode::NOT_FOUND,
            false,
            "transfer_ownership",
        )
        .await;
    }
    let viewer = viewer.unwrap_or_else(|| unreachable!());
    if viewer.id != owner_id && !viewer.admin {
        return pages::error_page_as(
            &state,
            &headers,
            StatusCode::NOT_FOUND,
            false,
            "transfer_ownership",
        )
        .await;
    }
    let fields = web::from_pairs::<TransferParams>(pairs)
        .and_then(|params| params.cookbook)
        .unwrap_or_default();
    let Some(recipient) = fields
        .user_id
        .as_deref()
        .and_then(|id| id.parse::<i32>().ok())
    else {
        return pages::error_page_as(
            &state,
            &headers,
            StatusCode::NOT_FOUND,
            false,
            "transfer_ownership",
        )
        .await;
    };
    let recipient_row: Option<(i32, String)> = sqlx::query_as(
        "SELECT u.id, a.username FROM users u
           JOIN accounts a ON a.user_id = u.id AND a.provider = 'chef_oauth2'
          WHERE u.id = $1",
    )
    .bind(recipient)
    .fetch_optional(&state.db)
    .await
    .ok()
    .flatten();
    let Some((recipient_id, recipient_username)) = recipient_row else {
        return pages::error_page_as(
            &state,
            &headers,
            StatusCode::NOT_FOUND,
            false,
            "transfer_ownership",
        )
        .await;
    };
    let add_owner = fields.add_owner_as_collaborator.as_deref() == Some("1");

    let is_collaborator: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM collaborators
          WHERE resourceable_type = 'Cookbook' AND resourceable_id = $1 AND user_id = $2)",
    )
    .bind(cookbook_id)
    .bind(recipient_id)
    .fetch_one(&state.db)
    .await
    .unwrap_or(false);

    let key = if viewer.admin || is_collaborator {
        let _ = sqlx::query("UPDATE cookbooks SET user_id = $2 WHERE id = $1")
            .bind(cookbook_id)
            .bind(recipient_id)
            .execute(&state.db)
            .await;
        if add_owner {
            let _ = sqlx::query(
                "INSERT INTO collaborators
                    (resourceable_id, resourceable_type, user_id, created_at, updated_at)
                 VALUES ($1, 'Cookbook', $2, NOW(), NOW())",
            )
            .bind(cookbook_id)
            .bind(owner_id)
            .execute(&state.db)
            .await;
        }
        let _ = sqlx::query(
            "DELETE FROM collaborators
              WHERE resourceable_type = 'Cookbook' AND resourceable_id = $1 AND user_id = $2",
        )
        .bind(cookbook_id)
        .bind(recipient_id)
        .execute(&state.db)
        .await;
        "cookbook.ownership_transfer.done"
    } else {
        use rand::RngCore;
        let mut bytes = [0u8; 16];
        rand::thread_rng().fill_bytes(&mut bytes);
        let token = hex::encode(bytes);
        let request_id: Option<i32> = sqlx::query_scalar(
            "INSERT INTO ownership_transfer_requests
                (cookbook_id, recipient_id, sender_id, token, add_owner_as_collaborator,
                 created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, NOW(), NOW()) RETURNING id",
        )
        .bind(cookbook_id)
        .bind(recipient_id)
        .bind(viewer.id)
        .bind(&token)
        .bind(add_owner)
        .fetch_optional(&state.db)
        .await
        .ok()
        .flatten();
        if let Some(request_id) = request_id {
            let _ = crate::jobs::perform_mailer(
                &state,
                "CookbookMailer",
                "transfer_ownership_email",
                vec![crate::jobs::MailerArg::Int(request_id.into())],
            )
            .await;
        }
        "cookbook.ownership_transfer.email_sent"
    };
    let notice = t(
        key,
        &[("cookbook", &cookbook_name), ("user", &recipient_username)],
    );
    web::redirect_with_flash(
        &state,
        &headers,
        &format!("/cookbooks/{cookbook_name}"),
        "notice",
        &notice,
    )
}

#[derive(sqlx::FromRow)]
struct TransferRequestRow {
    id: i32,
    cookbook_id: i32,
    recipient_id: i32,
    add_owner_as_collaborator: Option<bool>,
}

async fn find_request(db: &sqlx::PgPool, token: &str) -> Option<TransferRequestRow> {
    sqlx::query_as(
        "SELECT id, cookbook_id, recipient_id, add_owner_as_collaborator
           FROM ownership_transfer_requests WHERE token = $1 AND accepted IS NULL",
    )
    .bind(token)
    .fetch_optional(db)
    .await
    .ok()
    .flatten()
}

async fn cookbook_name_and_path(db: &sqlx::PgPool, id: i32) -> (String, String) {
    let name: Option<String> = sqlx::query_scalar("SELECT name FROM cookbooks WHERE id = $1")
        .bind(id)
        .fetch_optional(db)
        .await
        .ok()
        .flatten();
    let name = name.unwrap_or_default();
    (format!("/cookbooks/{name}"), name)
}

/// `GET /ownership_transfer/:token/accept`.
pub async fn accept(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(token): Path<String>,
) -> Response {
    let Some(request) = find_request(&state.db, &token).await else {
        return pages::error_page_as(
            &state,
            &headers,
            StatusCode::NOT_FOUND,
            false,
            "transfer_ownership",
        )
        .await;
    };
    let _ = sqlx::query("UPDATE ownership_transfer_requests SET accepted = TRUE WHERE id = $1")
        .bind(request.id)
        .execute(&state.db)
        .await;
    let owner_id: Option<i32> = sqlx::query_scalar("SELECT user_id FROM cookbooks WHERE id = $1")
        .bind(request.cookbook_id)
        .fetch_optional(&state.db)
        .await
        .ok()
        .flatten();
    if request.add_owner_as_collaborator.unwrap_or(false) {
        if let Some(owner_id) = owner_id {
            let _ = sqlx::query(
                "INSERT INTO collaborators
                    (resourceable_id, resourceable_type, user_id, created_at, updated_at)
                 VALUES ($1, 'Cookbook', $2, NOW(), NOW())",
            )
            .bind(request.cookbook_id)
            .bind(owner_id)
            .execute(&state.db)
            .await;
        }
    }
    let _ = sqlx::query("UPDATE cookbooks SET user_id = $2 WHERE id = $1")
        .bind(request.cookbook_id)
        .bind(request.recipient_id)
        .execute(&state.db)
        .await;
    if state.cfg.feature_active("fieri") && state.cfg.fieri_url.is_some() {
        let version_id: Option<i32> = sqlx::query_scalar(
            "SELECT id FROM cookbook_versions WHERE cookbook_id = $1 ORDER BY id DESC LIMIT 1",
        )
        .bind(request.cookbook_id)
        .fetch_optional(&state.db)
        .await
        .ok()
        .flatten();
        if let Some(version_id) = version_id {
            let _ = crate::jobs::perform_async(
                &state,
                "FieriNotifyWorker",
                vec![serde_json::json!(version_id)],
            )
            .await;
        }
    }
    let (path, name) = cookbook_name_and_path(&state.db, request.cookbook_id).await;
    let notice = t(
        "cookbook.ownership_transfer.invite_accepted",
        &[("cookbook", &name)],
    );
    web::redirect_with_flash(&state, &headers, &path, "notice", &notice)
}

/// `GET /ownership_transfer/:token/decline`.
pub async fn decline(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(token): Path<String>,
) -> Response {
    let Some(request) = find_request(&state.db, &token).await else {
        return pages::error_page_as(
            &state,
            &headers,
            StatusCode::NOT_FOUND,
            false,
            "transfer_ownership",
        )
        .await;
    };
    let _ = sqlx::query("UPDATE ownership_transfer_requests SET accepted = FALSE WHERE id = $1")
        .bind(request.id)
        .execute(&state.db)
        .await;
    let (path, name) = cookbook_name_and_path(&state.db, request.cookbook_id).await;
    let notice = t(
        "cookbook.ownership_transfer.invite_declined",
        &[("cookbook", &name)],
    );
    web::redirect_with_flash(&state, &headers, &path, "notice", &notice)
}
