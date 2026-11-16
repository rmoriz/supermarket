//! `ProfileController`: the account edit screen, its update, and the install
//! method preference persisted from the cookbook page.

use askama::Template;
use axum::extract::State;
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;

use crate::locales::t;
use crate::state::AppState;
use crate::web::users::{find_user_view, UserView};
use crate::web::{self, Page, Shell};
use crate::web::{filters, pages};

struct EmailPreference {
    name: String,
    checked: bool,
    id: Option<i32>,
    system_email_id: i32,
}

#[derive(Template)]
#[template(path = "profile/edit.html")]
struct Edit<'a> {
    shell: Shell<'a>,
    user: UserView,
    email_preferences: Vec<EmailPreference>,
    errors: Vec<String>,
    chef_profile_url: String,
    github: bool,
    github_account_type: &'static str,
    chef_oauth2_url: String,
    public_key_signature: Option<String>,
    github_accounts: Vec<String>,
    collaborations: Vec<(String, String, String, i32)>,
    show_email: bool,
    make_admin: bool,
    revoke_admin: bool,
    own_profile: bool,
    profile_controller: bool,
    chef_icon: String,
}

/// `SecureRandom.hex`: sixteen random bytes as thirty-two hex characters.
fn token() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

fn chef_profile_url() -> String {
    std::env::var("CHEF_PROFILE_URL")
        .or_else(|_| std::env::var("CHEF_MANAGE_URL"))
        .or_else(|_| std::env::var("CHEF_SERVER_URL"))
        .unwrap_or_else(|_| "https://api.chef.io".into())
}

fn chef_oauth2_url() -> String {
    std::env::var("CHEF_OAUTH2_URL")
        .or_else(|_| std::env::var("CHEF_SERVER_URL"))
        .unwrap_or_else(|_| "https://api.chef.io".into())
}

async fn email_preferences(state: &AppState, user_id: i32) -> Vec<EmailPreference> {
    let emails: Vec<(i32, String)> =
        sqlx::query_as("SELECT id, name FROM system_emails ORDER BY name")
            .fetch_all(&state.db)
            .await
            .unwrap_or_default();
    let existing: Vec<(i32, i32)> =
        sqlx::query_as("SELECT id, system_email_id FROM email_preferences WHERE user_id = $1")
            .bind(user_id)
            .fetch_all(&state.db)
            .await
            .unwrap_or_default();
    emails
        .into_iter()
        .map(|(system_email_id, name)| {
            let found = existing
                .iter()
                .find(|(_, system)| *system == system_email_id);
            EmailPreference {
                name,
                checked: found.is_some(),
                id: found.map(|(id, _)| *id),
                system_email_id,
            }
        })
        .collect()
}

async fn github_accounts(state: &AppState, user_id: i32) -> Vec<String> {
    sqlx::query_scalar(
        "SELECT username FROM accounts WHERE user_id = $1 AND provider = 'github' ORDER BY id",
    )
    .bind(user_id)
    .fetch_all(&state.db)
    .await
    .unwrap_or_default()
}

async fn collaborations(state: &AppState, user_id: i32) -> Vec<(String, String, String, i32)> {
    sqlx::query_as(
        "SELECT COALESCE(c.name, t.name),
                COALESCE('/cookbooks/' || c.name, '/tools/' || t.slug),
                col.resourceable_type, col.id
           FROM collaborators col
           LEFT JOIN cookbooks c ON col.resourceable_type = 'Cookbook' AND c.id = col.resourceable_id
           LEFT JOIN tools t ON col.resourceable_type = 'Tool' AND t.id = col.resourceable_id
          WHERE col.user_id = $1 AND col.group_id IS NULL
          ORDER BY col.id",
    )
    .bind(user_id)
    .fetch_all(&state.db)
    .await
    .unwrap_or_default()
}

/// `User#public_key_signature`: MD5 over the PKCS#1 DER of the stored public
/// key, printed as colon-separated octets so the user can compare it with
/// `openssl rsa ... | openssl md5 -c`.
async fn public_key_signature(state: &AppState, user_id: i32) -> Option<String> {
    let pem: Option<String> = sqlx::query_scalar("SELECT public_key FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_optional(&state.db)
        .await
        .ok()
        .flatten();
    let pem = pem.filter(|pem| !pem.trim().is_empty())?;
    use rsa::pkcs8::{DecodePublicKey, EncodePublicKey};
    let key = rsa::RsaPublicKey::from_public_key_pem(&pem).ok()?;
    let der = key.to_public_key_der().ok()?;
    let hex = format!("{:x}", md5::compute(der.as_bytes()));
    Some(
        hex.as_bytes()
            .chunks(2)
            .map(|pair| String::from_utf8_lossy(pair).into_owned())
            .collect::<Vec<_>>()
            .join(":"),
    )
}

async fn render(
    state: &AppState,
    headers: &HeaderMap,
    errors: Vec<String>,
    submitted: Option<&UpdateFields>,
) -> Response {
    let viewer = web::viewer(state, headers).await;
    let Some(viewer) = viewer else {
        return web::redirect_to_sign_in(state, headers, "/profile/edit");
    };
    let Some(mut user) = find_user_view(&state.db, &viewer.username).await else {
        return pages::error_page_as(state, headers, StatusCode::NOT_FOUND, false, "profile").await;
    };
    if let Some(fields) = submitted {
        if let Some(company) = &fields.company {
            user.company = Some(company.clone()).filter(|v| !v.is_empty());
        }
        if let Some(twitter) = &fields.twitter_username {
            user.twitter_username = Some(twitter.clone()).filter(|v| !v.is_empty());
        }
        if let Some(slack) = &fields.slack_username {
            user.slack_username = Some(slack.clone()).filter(|v| !v.is_empty());
        }
    }
    let shell = pages::shell(state, Some(&viewer), headers).title("Manage Profile");
    let page = Edit {
        shell,
        email_preferences: email_preferences(state, viewer.id).await,
        user,
        errors,
        chef_profile_url: chef_profile_url(),
        github: state.cfg.feature_active("github"),
        github_account_type: "GitHub",
        chef_oauth2_url: chef_oauth2_url(),
        public_key_signature: public_key_signature(state, viewer.id).await,
        github_accounts: github_accounts(state, viewer.id).await,
        collaborations: collaborations(state, viewer.id).await,
        show_email: viewer.admin,
        make_admin: false,
        revoke_admin: false,
        own_profile: true,
        profile_controller: true,
        chef_icon: pages::chef_icon(),
    };
    Page::ok(page).into_response()
}

/// `GET /profile/edit`.
pub async fn edit(State(state): State<AppState>, headers: HeaderMap) -> Response {
    render(&state, &headers, Vec::new(), None).await
}

#[derive(Debug, Default, Deserialize)]
pub struct UpdateParams {
    pub user: Option<UpdateFields>,
}

#[derive(Debug, Default, Deserialize)]
pub struct UpdateFields {
    pub email: Option<String>,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub company: Option<String>,
    pub twitter_username: Option<String>,
    pub slack_username: Option<String>,
    pub jira_username: Option<String>,
    #[serde(default)]
    pub email_preferences_attributes: Vec<EmailPreferenceFields>,
}

#[derive(Debug, Default, Deserialize)]
pub struct EmailPreferenceFields {
    pub id: Option<String>,
    pub system_email_id: Option<String>,
    pub _destroy: Option<String>,
}

#[derive(Template)]
#[template(path = "profile/link_github.html")]
#[allow(dead_code)]
struct LinkGithub<'a> {
    shell: Shell<'a>,
}

/// `GET /profile/link-github`: the action has no method, so Rails renders the
/// template directly.
pub async fn link_github(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let viewer = web::viewer(&state, &headers).await;
    let Some(viewer) = viewer else {
        return web::redirect_to_sign_in(&state, &headers, "/profile/link-github");
    };
    let shell = pages::shell(&state, Some(&viewer), &headers).title("Link GitHub Account");
    Page::ok(LinkGithub { shell }).into_response()
}

/// `PATCH /profile/change_password`: routed but has neither method nor template.
pub async fn change_password(State(state): State<AppState>, headers: HeaderMap) -> Response {
    pages::action_not_found(&state, &headers).await
}

/// `PATCH /profile`.
pub async fn update(
    State(state): State<AppState>,
    headers: HeaderMap,
    axum::extract::Form(pairs): axum::extract::Form<Vec<(String, String)>>,
) -> Response {
    let Some(viewer) = web::viewer(&state, &headers).await else {
        return web::redirect_to_sign_in(&state, &headers, "/profile/edit");
    };
    let fields = web::from_pairs::<UpdateParams>(pairs)
        .and_then(|params| params.user)
        .unwrap_or_default();
    // `validates :email, presence: true`: a blank email fails the update, which
    // falls into `render "edit"` — where `@user` was never assigned, so the page
    // raises. The raised page is what users see (a 500), not the form.
    if fields.email.as_deref() == Some("") {
        return pages::error_page_as(
            &state,
            &headers,
            StatusCode::INTERNAL_SERVER_ERROR,
            false,
            "exceptions",
        )
        .await;
    }
    if let Some(email) = fields.email.as_deref() {
        let _ = sqlx::query("UPDATE users SET email = $2 WHERE id = $1")
            .bind(viewer.id)
            .bind(email)
            .execute(&state.db)
            .await;
    }
    for (column, value) in [
        ("first_name", fields.first_name),
        ("last_name", fields.last_name),
        ("company", fields.company),
        ("twitter_username", fields.twitter_username),
        ("slack_username", fields.slack_username),
        ("jira_username", fields.jira_username),
    ] {
        if let Some(value) = value {
            let _ = sqlx::query(&format!("UPDATE users SET {column} = $2 WHERE id = $1"))
                .bind(viewer.id)
                .bind(value)
                .execute(&state.db)
                .await;
        }
    }
    for preference in &fields.email_preferences_attributes {
        let Some(system_email_id) = preference
            .system_email_id
            .as_deref()
            .and_then(|id| id.parse::<i32>().ok())
        else {
            continue;
        };
        if preference._destroy.as_deref() == Some("1") {
            let _ = sqlx::query(
                "DELETE FROM email_preferences
                  WHERE user_id = $1 AND system_email_id = $2",
            )
            .bind(viewer.id)
            .bind(system_email_id)
            .execute(&state.db)
            .await;
        } else {
            // `EmailPreference` mints its token in `before_validation`.
            let _ = sqlx::query(
                "INSERT INTO email_preferences
                    (user_id, system_email_id, token, created_at, updated_at)
                 VALUES ($1, $2, $3, NOW(), NOW())
                 ON CONFLICT (user_id, system_email_id) DO NOTHING",
            )
            .bind(viewer.id)
            .bind(system_email_id)
            .bind(token())
            .execute(&state.db)
            .await;
        }
    }
    let notice = t("profile.updated", &[]);
    web::redirect_with_flash(
        &state,
        &headers,
        &format!("/users/{}", viewer.username),
        "notice",
        &notice,
    )
}

#[derive(Debug, Default, Deserialize)]
pub struct PreferenceParams {
    pub preference: Option<String>,
}

/// `POST /profile/update_install_preference`: bare 200, 404 when signed out.
pub async fn update_install_preference(
    State(state): State<AppState>,
    headers: HeaderMap,
    axum::extract::Form(params): axum::extract::Form<PreferenceParams>,
) -> Response {
    let Some(viewer) = web::viewer(&state, &headers).await else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Some(preference) = params.preference else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    let _ = sqlx::query("UPDATE users SET install_preference = $2 WHERE id = $1")
        .bind(viewer.id)
        .bind(&preference)
        .execute(&state.db)
        .await;
    // `head(200)`: empty body, Rails' default content type.
    (StatusCode::OK, [(header::CONTENT_TYPE, "text/html")], "").into_response()
}
