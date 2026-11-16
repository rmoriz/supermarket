//! `SessionsController` and the OAuth handshake, including account linking via
//! `AccountsController`.

use askama::Template;

#[allow(unused_imports)]
use crate::locales::t;
use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::Value;

use crate::session;
use crate::state::AppState;
#[allow(unused_imports)]
use crate::web::filters;
use crate::web::pages;
use crate::web::{self, Page, Shell};

#[derive(Template)]
#[template(path = "sessions/new.html")]
#[allow(dead_code)]
struct New<'a> {
    shell: Shell<'a>,
}

pub async fn new(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let viewer = web::viewer(&state, &headers).await;
    let shell = pages::shell(&state, viewer.as_ref(), &headers);
    Page::ok(New { shell }).into_response()
}

#[derive(Template)]
#[template(path = "sessions/failure.html")]
#[allow(dead_code)]
struct Failure<'a> {
    shell: Shell<'a>,
}

pub async fn failure(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let viewer = web::viewer(&state, &headers).await;
    let shell = pages::shell(&state, viewer.as_ref(), &headers);
    Page::ok(Failure { shell }).into_response()
}

/// `GET /login`, `GET /signin`.
pub async fn login(State(state): State<AppState>) -> Response {
    route_redirect(&state, "/sign-in")
}

/// `DELETE /logout`, `DELETE /signout`.
pub async fn logout(State(state): State<AppState>) -> Response {
    route_redirect(&state, "/sign-out")
}

/// `SecureRandom.hex`: sixteen random bytes as thirty-two hex characters.
fn random_token() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

fn route_redirect(state: &AppState, path: &str) -> Response {
    let mut response = web::redirect(state, path);
    response
        .extensions_mut()
        .insert(crate::middleware::RouterRedirect);
    response
}

struct Provider {
    site: String,
    authorize_url: String,
    token_url: String,
    client_id: String,
    client_secret: String,
    user_info: &'static str,
}

fn provider(state: &AppState, name: &str) -> Option<Provider> {
    match name {
        "chef_oauth2" => Some(Provider {
            site: state
                .cfg
                .chef_oauth2_url
                .clone()
                .unwrap_or_else(|| "https://id.chef.io".into()),
            authorize_url: "/id/oauth/authorize".into(),
            token_url: "/id/oauth/token".into(),
            client_id: state.cfg.chef_oauth2_app_id.clone().unwrap_or_default(),
            client_secret: state.cfg.chef_oauth2_secret.clone().unwrap_or_default(),
            user_info: "/id/v1/me",
        }),
        "github" => Some(Provider {
            site: state
                .cfg
                .github_site
                .clone()
                .unwrap_or_else(|| "https://api.github.com".into()),
            authorize_url: state
                .cfg
                .github_authorize_url
                .clone()
                .unwrap_or_else(|| "https://github.com/login/oauth/authorize".into()),
            token_url: state
                .cfg
                .github_access_token_url
                .clone()
                .unwrap_or_else(|| "https://github.com/login/oauth/access_token".into()),
            client_id: state.cfg.github_key.clone().unwrap_or_default(),
            client_secret: state.cfg.github_secret.clone().unwrap_or_default(),
            user_info: "user",
        }),
        _ => None,
    }
}

/// `POST /auth/:provider`: OmniAuth's request phase, a redirect to the provider.
pub async fn passthru(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(name): Path<String>,
) -> Response {
    let Some(provider) = provider(&state, &name) else {
        // OmniAuth raises outside the controller stack: the 404 page carries no
        // CSP, no `cache-control` and no session write-back.
        let mut response = pages::error_page(&state, &headers, StatusCode::NOT_FOUND, false).await;
        response
            .extensions_mut()
            .insert(crate::middleware::ExceptionResponse);
        return response;
    };
    // OmniAuth's `SecureRandom.hex(20)`: forty hex characters.
    let state_token: String = {
        use rand::RngCore;
        let mut bytes = [0u8; 20];
        rand::thread_rng().fill_bytes(&mut bytes);
        hex::encode(bytes)
    };
    let redirect_uri = format!("{}/auth/{name}/callback", state.cfg.base_url());
    let authorization = if provider.authorize_url.starts_with("http") {
        provider.authorize_url.clone()
    } else {
        format!("{}{}", provider.site, provider.authorize_url)
    };
    let mut response = web::redirect_with_session(&state, &headers, &authorization, |session| {
        session.set_oauth_state(&state_token)
    });
    response
        .extensions_mut()
        .insert(crate::middleware::OmniAuthRedirect);
    let location = format!(
        "{authorization}?client_id={}&redirect_uri={}&response_type=code&state={state_token}",
        urlencoding(&provider.client_id),
        urlencoding(&redirect_uri),
    );
    if let Ok(value) = location.parse() {
        response.headers_mut().insert(header::LOCATION, value);
    }
    // OmniAuth's own Rack response body, kept verbatim.
    *response.body_mut() = axum::body::Body::from(format!("Redirecting to {location}..."));
    response
}

fn urlencoding(value: &str) -> String {
    form_urlencoded::byte_serialize(value.as_bytes()).collect()
}

#[derive(Debug, Deserialize)]
pub struct CallbackParams {
    pub code: Option<String>,
    pub state: Option<String>,
}

/// `GET|POST /auth/chef_oauth2/callback` and `/auth/:provider/callback`.
pub async fn callback(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(name): Path<String>,
    Query(params): Query<CallbackParams>,
) -> Response {
    let Some(provider) = provider(&state, &name) else {
        // OmniAuth raises outside the controller stack: the 404 page carries no
        // CSP, no `cache-control` and no session write-back.
        let mut response = pages::error_page(&state, &headers, StatusCode::NOT_FOUND, false).await;
        response
            .extensions_mut()
            .insert(crate::middleware::ExceptionResponse);
        return response;
    };
    // OmniAuth's `callback_phase` raises when the provider sends no code.
    let Some(code) = params.code.as_deref() else {
        return pages::error_page(&state, &headers, StatusCode::INTERNAL_SERVER_ERROR, false).await;
    };
    let client = reqwest::Client::builder()
        .user_agent("supermarket")
        .build()
        .unwrap_or_default();
    let token_url = if provider.token_url.starts_with("http") {
        provider.token_url.clone()
    } else {
        format!("{}{}", provider.site, provider.token_url)
    };
    let redirect_uri = format!("{}/auth/{name}/callback", state.cfg.base_url());
    let token: Option<Value> = match client
        .post(token_url)
        .header(header::ACCEPT, "application/json")
        .form(&[
            ("client_id", provider.client_id.as_str()),
            ("client_secret", provider.client_secret.as_str()),
            ("code", code),
            ("redirect_uri", redirect_uri.as_str()),
            ("grant_type", "authorization_code"),
        ])
        .send()
        .await
    {
        Ok(response) => response.json().await.ok(),
        Err(_) => None,
    };

    let access_token = token
        .as_ref()
        .and_then(|value| value.get("access_token"))
        .and_then(Value::as_str);
    let Some(access_token) = access_token else {
        return web::redirect_with_flash(
            &state,
            &headers,
            "/auth/failure?message=invalid_credentials",
            "alert",
            &t("requires_linked_github", &[]),
        );
    };
    let info_url = if provider.user_info.starts_with("http") {
        provider.user_info.to_owned()
    } else {
        format!(
            "{}/{}",
            provider.site.trim_end_matches('/'),
            provider.user_info.trim_start_matches('/')
        )
    };
    let info: Option<Value> = match client
        .get(info_url)
        .bearer_auth(access_token)
        .header(header::ACCEPT, "application/json")
        .send()
        .await
    {
        Ok(response) => response.json().await.ok(),
        Err(_) => None,
    };
    let Some(info) = info else {
        return web::redirect_with_flash(
            &state,
            &headers,
            "/auth/failure?message=invalid_credentials",
            "alert",
            &t("requires_linked_github", &[]),
        );
    };

    if name == "chef_oauth2" {
        sign_in(
            &state,
            &headers,
            &info,
            token.as_ref().unwrap_or(&Value::Null),
        )
        .await
    } else {
        link(&state, &headers, &name, &info, access_token).await
    }
}

/// `SessionsController#create`: find or create the account and sign in.
async fn sign_in(state: &AppState, headers: &HeaderMap, info: &Value, token: &Value) -> Response {
    let text = |key: &str| {
        info.get(key)
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
    };
    let username = text("username");
    let Some(username) = username else {
        return web::redirect_with_flash(
            state,
            headers,
            "/auth/failure?message=invalid_credentials",
            "alert",
            &t("requires_linked_github", &[]),
        );
    };
    let credential = |key: &str| {
        token
            .get(key)
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
    };
    let expires = token
        .get("expires_at")
        .and_then(Value::as_i64)
        .and_then(|seconds| chrono::DateTime::from_timestamp(seconds, 0))
        .map(|stamp| stamp.naive_utc());

    let existing: Option<i32> = sqlx::query_scalar(
        "SELECT user_id FROM accounts WHERE provider = 'chef_oauth2' AND uid = $1",
    )
    .bind(&username)
    .fetch_optional(&state.db)
    .await
    .ok()
    .flatten();

    let user_id = match existing {
        Some(user_id) => {
            let _ = sqlx::query(
                "UPDATE accounts SET username = $1, oauth_token = $2, oauth_refresh_token = $3,
                        oauth_expires = $4, updated_at = NOW()
                  WHERE provider = 'chef_oauth2' AND uid = $1",
            )
            .bind(&username)
            .bind(credential("access_token"))
            .bind(credential("refresh_token"))
            .bind(expires)
            .execute(&state.db)
            .await;
            let _ = sqlx::query(
                "UPDATE users SET public_key = COALESCE($2, public_key), first_name = $3,
                        last_name = $4, email = COALESCE($5, email), updated_at = NOW()
                  WHERE id = $1",
            )
            .bind(user_id)
            .bind(text("public_key"))
            .bind(text("first_name"))
            .bind(text("last_name"))
            .bind(text("email"))
            .execute(&state.db)
            .await;
            user_id
        }
        None => {
            let inserted: Option<i32> = sqlx::query_scalar(
                "INSERT INTO users (first_name, last_name, email, public_key, roles_mask,
                                    created_at, updated_at)
                 VALUES ($1, $2, $3, $4, 0, NOW(), NOW()) RETURNING id",
            )
            .bind(text("first_name"))
            .bind(text("last_name"))
            .bind(text("email"))
            .bind(text("public_key"))
            .fetch_optional(&state.db)
            .await
            .ok()
            .flatten();
            let Some(user_id) = inserted else {
                return pages::error_page(state, headers, StatusCode::INTERNAL_SERVER_ERROR, false)
                    .await;
            };
            let _ = sqlx::query(
                "INSERT INTO accounts (user_id, provider, uid, username, oauth_token,
                                       oauth_refresh_token, oauth_expires, created_at, updated_at)
                 VALUES ($1, 'chef_oauth2', $2, $2, $3, $4, $5, NOW(), NOW())",
            )
            .bind(user_id)
            .bind(&username)
            .bind(credential("access_token"))
            .bind(credential("refresh_token"))
            .bind(expires)
            .execute(&state.db)
            .await;
            // `User#after_create` -> `default_email_preferences`: a new account
            // starts subscribed to every system email, each with its own token.
            let system_emails: Vec<i32> =
                sqlx::query_scalar("SELECT id FROM system_emails ORDER BY id")
                    .fetch_all(&state.db)
                    .await
                    .unwrap_or_default();
            for system_email_id in system_emails {
                let _ = sqlx::query(
                    "INSERT INTO email_preferences
                        (user_id, system_email_id, token, created_at, updated_at)
                     VALUES ($1, $2, $3, NOW(), NOW())
                     ON CONFLICT (user_id, system_email_id) DO NOTHING",
                )
                .bind(user_id)
                .bind(system_email_id)
                .bind(random_token())
                .execute(&state.db)
                .await;
            }
            user_id
        }
    };

    let name = match (text("first_name"), text("last_name")) {
        (Some(first), Some(last)) => format!("{first} {last}"),
        (Some(first), None) => first,
        (None, Some(last)) => last,
        (None, None) => username.clone(),
    };
    let notice = t("user.signed_in", &[("name", &name)]);
    let return_to = web::session_from(state, headers)
        .and_then(|session| session.return_to().map(str::to_owned))
        .unwrap_or_else(|| "/".into());
    web::redirect_with_session_and_flash(state, headers, &return_to, "notice", &notice, |session| {
        session.set_user_id(user_id);
        session.take_return_to();
    })
}

/// `AccountsController#create`: attach the OAuth identity to the current user.
async fn link(
    state: &AppState,
    headers: &HeaderMap,
    provider_name: &str,
    info: &Value,
    access_token: &str,
) -> Response {
    let Some(viewer) = web::viewer(state, headers).await else {
        return web::redirect_to_sign_in(state, headers, "/profile/edit");
    };
    let uid = match provider_name {
        "github" => info
            .get("id")
            .map(Value::to_string)
            .map(|value| value.trim_matches('"').to_owned())
            .or_else(|| info.get("login").and_then(Value::as_str).map(str::to_owned)),
        _ => info.get("uid").and_then(Value::as_str).map(str::to_owned),
    };
    let username = match provider_name {
        "github" => info.get("login").and_then(Value::as_str),
        _ => info.get("username").and_then(Value::as_str),
    };
    let (Some(uid), Some(username)) = (uid, username) else {
        return web::redirect_with_flash(
            state,
            headers,
            "/profile/edit",
            "alert",
            "is not a valid extractor!",
        );
    };
    let existing: Option<i32> =
        sqlx::query_scalar("SELECT id FROM accounts WHERE provider = $1 AND uid = $2")
            .bind(provider_name)
            .bind(&uid)
            .fetch_optional(&state.db)
            .await
            .ok()
            .flatten();
    if existing.is_some() {
        return web::redirect_with_flash(
            state,
            headers,
            "/profile/edit",
            "alert",
            &t(
                "account.already_connected",
                &[("provider", provider_name), ("username", username)],
            ),
        );
    }
    let _ = sqlx::query(
        "INSERT INTO accounts (user_id, provider, uid, username, oauth_token,
                               created_at, updated_at)
         VALUES ($1, $2, $3, $4, $5, NOW(), NOW())",
    )
    .bind(viewer.id)
    .bind(provider_name)
    .bind(&uid)
    .bind(username)
    .bind(access_token)
    .execute(&state.db)
    .await;
    let return_to = web::session_from(state, headers)
        .and_then(|session| session.return_to().map(str::to_owned))
        .unwrap_or_else(|| "/profile/edit".into());
    let notice = format!("Successfully connected {provider_name}.");
    web::redirect_with_session_and_flash(state, headers, &return_to, "notice", &notice, |session| {
        session.take_return_to();
    })
}

/// `DELETE /sign-out`: reset the session and redirect home.
pub async fn destroy(State(state): State<AppState>, _headers: HeaderMap) -> Response {
    let mut session = session::Session::new(String::new());
    session.set_flash("signout", "true");
    session.set_flash("notice", &t("user.signed_out", &[]));
    let mut response = web::redirect(&state, "/");
    web::write_session(&mut response, &session, &state);
    response
}
