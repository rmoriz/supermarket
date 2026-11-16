//! `EmailPreferencesController#unsubscribe`: a token in a mail unsubscribes the
//! recipient from one system email.

use askama::Template;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};

#[allow(unused_imports)]
use crate::web::filters;

use crate::state::AppState;
use crate::web::pages;
use crate::web::{self, Page, Shell};

#[derive(Template)]
#[template(path = "email_preferences/unsubscribe.html")]
#[allow(dead_code)]
struct Unsubscribe<'a> {
    shell: Shell<'a>,
    system_email: String,
}

pub async fn unsubscribe(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(token): Path<String>,
) -> Response {
    let row: Option<(i32, String)> = sqlx::query_as(
        "SELECT ep.id, se.name FROM email_preferences ep
           JOIN system_emails se ON se.id = ep.system_email_id
          WHERE ep.token = $1",
    )
    .bind(&token)
    .fetch_optional(&state.db)
    .await
    .ok()
    .flatten();
    let Some((id, system_email)) = row else {
        return pages::error_page_as(
            &state,
            &headers,
            StatusCode::NOT_FOUND,
            false,
            "email_preferences",
        )
        .await;
    };
    let _ = sqlx::query("DELETE FROM email_preferences WHERE id = $1")
        .bind(id)
        .execute(&state.db)
        .await;
    let viewer = web::viewer(&state, &headers).await;
    let shell = pages::shell(&state, viewer.as_ref(), &headers);
    Page::ok(Unsubscribe {
        shell,
        system_email,
    })
    .into_response()
}
