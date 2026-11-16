//! `AccountsController#destroy`: unlink a connected OAuth account from the
//! signed-in user.

use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};

use crate::locales::t;
use crate::state::AppState;
use crate::web::pages;
use crate::web::{self, Page};

pub async fn destroy(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((_user_id, id)): Path<(i32, i32)>,
) -> Response {
    let Some(viewer) = web::viewer(&state, &headers).await else {
        return web::redirect_to_sign_in(&state, &headers, "/profile/edit");
    };
    // `current_user.accounts.find` raises `RecordNotFound` for foreign rows.
    let found: Option<i32> =
        sqlx::query_scalar("SELECT id FROM accounts WHERE id = $1 AND user_id = $2")
            .bind(id)
            .bind(viewer.id)
            .fetch_optional(&state.db)
            .await
            .ok()
            .flatten();
    if found.is_none() {
        return pages::error_page_as(&state, &headers, StatusCode::NOT_FOUND, false, "accounts")
            .await;
    }
    let _ = sqlx::query("DELETE FROM accounts WHERE id = $1")
        .bind(id)
        .execute(&state.db)
        .await;
    // `redirect_back ... fallback_location: edit_profile_path`: an absolute
    // referer is used verbatim, a local path gets the host prefix.
    let back = headers
        .get(header::REFERER)
        .and_then(|value| value.to_str().ok())
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    let mut response = match back {
        Some(back) if back.starts_with("http") => {
            let path = back
                .strip_prefix(&state.cfg.base_url())
                .unwrap_or(&back)
                .to_owned();
            if back.starts_with(&state.cfg.base_url()) {
                web::redirect_with_flash(
                    &state,
                    &headers,
                    &path,
                    "notice",
                    &t("account.disconnected", &[]),
                )
            } else {
                let mut response = web::redirect_with_flash(
                    &state,
                    &headers,
                    &path,
                    "notice",
                    &t("account.disconnected", &[]),
                );
                if let Ok(value) = back.parse() {
                    response.headers_mut().insert(header::LOCATION, value);
                }
                response
            }
        }
        _ => web::redirect_with_flash(
            &state,
            &headers,
            "/profile/edit",
            "notice",
            &t("account.disconnected", &[]),
        ),
    };
    let _ = &mut response;
    response
}

/// `head :no_content`-style helpers are not used here; keep `Page` reachable.
#[allow(dead_code)]
fn _unused(_page: Page) -> Response {
    StatusCode::OK.into_response()
}
