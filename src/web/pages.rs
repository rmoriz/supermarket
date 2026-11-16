//! `PagesController`: welcome, dashboard, documentation, robots, and the error
//! page rendered by the exceptions app.

use crate::web::filters;
use askama::Template;
use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use sqlx::PgPool;

use crate::locales::t;
use crate::state::AppState;
use crate::web::{self, Page, Shell};

pub const BADGES: &[&str] = &["partner"];
pub const PLATFORMS: &[&str] = &[
    "aix",
    "amazon",
    "centos",
    "debian",
    "fedora",
    "freebsd",
    "mac_os_x",
    "opensuseleap",
    "oracle",
    "redhat",
    "ubuntu",
    "scientific",
    "smartos",
    "solaris",
    "suse",
    "windows",
];

pub fn shell<'a>(
    state: &AppState,
    viewer: Option<&'a web::Viewer>,
    headers: &HeaderMap,
) -> Shell<'a> {
    Shell::new(state, viewer, headers)
}

pub fn quality_tip() -> String {
    t("cookbook.quality_metric_pass_rate_tip", &[])
}

pub fn partner_badge() -> String {
    web::asset_path("badge-cookbook-partner.svg")
}

pub fn chef_icon() -> String {
    web::asset_path("chef-icon.png")
}

pub fn platform_icon(name: &str) -> String {
    let icon = match name {
        "aix" => "aix",
        "almalinux" => "almalinux",
        "amazon" => "aws",
        "centos" => "centos",
        "debian" => "debian",
        "fedora" => "fedora",
        "freebsd" => "freebsd",
        "linuxmint" => "linuxmint",
        "mac_os_x" | "mac_os_x_server" => "macosx",
        "oracle" => "oracle",
        "opensuse" | "opensuseleap" => "suse",
        "redhat" => "redhat",
        "rocky" => "rocky",
        "scientific" => "scientific",
        "smartos" => "smartos",
        "suse" => "suse",
        "ubuntu" => "ubuntu",
        "windows" => "windows",
        "zlinux" => "zlinux",
        _ => "generic",
    };
    web::asset_path(&format!("platform_{icon}.svg"))
}

/// A route whose action (and template) is missing: Rails raises
/// `ActionNotFound` and serves it from `config.exceptions_app`.
pub async fn action_not_found(state: &AppState, headers: &HeaderMap) -> Response {
    let mut response = error_page(state, headers, StatusCode::NOT_FOUND, false).await;
    response
        .extensions_mut()
        .insert(crate::middleware::ExceptionResponse);
    response
}

/// Renders an exception page through the layout, like `ExceptionsController#show`.
pub async fn error_page(
    state: &AppState,
    headers: &HeaderMap,
    status: StatusCode,
    json: bool,
) -> Response {
    error_page_as(state, headers, status, json, "exceptions").await
}

/// Same, with the controller-specific body class Rails' layout uses (the
/// exception is rendered by the controller that raised it).
pub async fn error_page_as(
    state: &AppState,
    headers: &HeaderMap,
    status: StatusCode,
    json: bool,
    class: &str,
) -> Response {
    error_page_as_q(state, headers, status, json, class, None, None).await
}

/// Same, also carrying the `q` search term the layout prefills.
pub async fn error_page_as_q(
    state: &AppState,
    headers: &HeaderMap,
    status: StatusCode,
    json: bool,
    class: &str,
    q: Option<&str>,
    search: Option<(&'static str, &'static str)>,
) -> Response {
    let viewer = web::viewer(state, headers).await;
    if json {
        let message = match status {
            StatusCode::NOT_FOUND => "Not Found",
            _ => status.canonical_reason().unwrap_or("Error"),
        };
        return (
            status,
            [(header::CONTENT_TYPE, "application/json; charset=utf-8")],
            format!(
                r#"{{"message":{}}}"#,
                serde_json::to_string(message).unwrap()
            ),
        )
            .into_response();
    }
    let mut shell = shell(state, viewer.as_ref(), headers);
    shell.air_gapped = true;
    // The layout's search form targets the current controller's index, unless
    // the request raised before the `override_search` callback ran.
    if let Some((path, name)) = search {
        shell.search_path = path.into();
        shell.search_name = name;
    } else if class == "tools" {
        shell.search_path = "/tools".into();
        shell.search_name = "Tools";
    }
    shell.q = q.map(str::to_owned);
    let body = match status {
        StatusCode::UNAUTHORIZED => render_401(&shell, class),
        StatusCode::UNPROCESSABLE_ENTITY => render_422(&shell, class),
        StatusCode::INTERNAL_SERVER_ERROR => render_500(&shell, class),
        _ => render_404(&shell, class),
    };
    let mut response = Page { status, body }.into_response();
    // Raised exceptions are rendered by `config.exceptions_app`, outside the
    // stack that adds CSP, caching headers and the session cookie.
    if status.is_server_error() {
        response
            .extensions_mut()
            .insert(crate::middleware::ExceptionResponse);
    }
    response
}

#[derive(Template)]
#[template(path = "exceptions/401.html", escape = "none")]
struct Exceptions401<'a> {
    shell: &'a Shell<'a>,
    body_class: &'a str,
}
fn render_401(shell: &Shell<'_>, body_class: &str) -> String {
    Exceptions401 { shell, body_class }
        .render()
        .unwrap_or_default()
}

#[derive(Template)]
#[template(path = "exceptions/404.html", escape = "none")]
struct Exceptions404<'a> {
    shell: &'a Shell<'a>,
    body_class: &'a str,
}
fn render_404(shell: &Shell<'_>, body_class: &str) -> String {
    Exceptions404 { shell, body_class }
        .render()
        .unwrap_or_default()
}

#[derive(Template)]
#[template(path = "exceptions/422.html", escape = "none")]
struct Exceptions422<'a> {
    shell: &'a Shell<'a>,
    body_class: &'a str,
}
fn render_422(shell: &Shell<'_>, body_class: &str) -> String {
    Exceptions422 { shell, body_class }
        .render()
        .unwrap_or_default()
}

#[derive(Template)]
#[template(path = "exceptions/500.html", escape = "none")]
struct Exceptions500<'a> {
    shell: &'a Shell<'a>,
    body_class: &'a str,
}
fn render_500(shell: &Shell<'_>, body_class: &str) -> String {
    Exceptions500 { shell, body_class }
        .render()
        .unwrap_or_default()
}

#[derive(Template)]
#[template(path = "pages/welcome.html")]
struct Welcome<'a> {
    shell: Shell<'a>,
    signout: bool,
    chef_identity_url: String,
    chef_sign_up_url: String,
    cookbook_count: String,
    user_count: String,
}

pub async fn welcome(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let viewer = web::viewer(&state, &headers).await;
    if viewer.is_some() {
        return web::redirect(&state, "/dashboard");
    }
    let cookbook_count: i64 = count(&state.db, "cookbooks").await;
    let user_count: i64 = count(&state.db, "users").await;
    let shell = shell(&state, None, &headers).title("Welcome - The resource for Chef cookbooks")
        .description("Supermarket is Chef's open-source community platform. Find, explore and view Chef cookbooks for all of your ops needs.");
    let page = Welcome {
        shell,
        signout: web::flash(&state, &headers)
            .iter()
            .any(|(name, _)| name == "signout"),
        chef_identity_url: Shell::chef_identity_url(),
        chef_sign_up_url: Shell::chef_sign_up_url(),
        cookbook_count: web::pluralize_commas(cookbook_count, "Cookbook"),
        user_count: web::pluralize_commas(user_count, "Chef"),
    };
    Page::ok(page).into_response()
}

async fn count(db: &PgPool, table: &str) -> i64 {
    sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table}"))
        .fetch_one(db)
        .await
        .unwrap_or(0)
}

#[derive(Template)]
#[template(path = "pages/documentation.html")]
struct Documentation<'a> {
    shell: Shell<'a>,
}

pub async fn documentation(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let viewer = web::viewer(&state, &headers).await;
    let shell = shell(&state, viewer.as_ref(), &headers)
        .title("Documentation")
        .description("View and browse guides, documentation, and tutorials.");
    Page::ok(Documentation { shell }).into_response()
}

#[derive(Template)]
#[template(path = "pages/robots.txt", escape = "none")]
struct Robots {
    robots_allow: Option<String>,
    robots_disallow: Option<String>,
    base: String,
}

/// `GET /robots.:format`: `respond_to :text` answers `text`, every other format
/// is an `UnknownFormat` handled by the exceptions app (html → 404 page, json →
/// `{}`, anything else → the plain 500 page).
pub async fn robots_format(State(state): State<AppState>, Path(format): Path<String>) -> Response {
    let format = format.trim_start_matches('.');
    const NOT_FOUND_BODY: &str = "<div class=\"page withspace\">\n  <div class=\"dialog\">\n    <h1>The page you were looking for does not exist.</h1>\n    <p>You may have mistyped the address or the page may have moved.</p>\n  </div>\n</div>\n";
    const ERROR_BODY: &str = "500 Internal Server Error\nIf you are the administrator of this website, then please read this web application's log file and/or the web server's log file to find out what went wrong.";
    match format {
        "txt" | "text" => robots(State(state)).await,
        "json" => (
            StatusCode::NOT_FOUND,
            [(header::CONTENT_TYPE, "application/json; charset=utf-8")],
            "{}",
        )
            .into_response(),
        "html" => (
            StatusCode::NOT_FOUND,
            [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
            NOT_FOUND_BODY,
        )
            .into_response(),
        _ => {
            let mut response = (
                StatusCode::INTERNAL_SERVER_ERROR,
                [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
                ERROR_BODY,
            )
                .into_response();
            response
                .extensions_mut()
                .insert(crate::middleware::ExceptionResponse);
            response
        }
    }
}

pub async fn robots(State(state): State<AppState>) -> Response {
    let page = Robots {
        robots_allow: state.cfg.robots_allow.clone(),
        robots_disallow: state.cfg.robots_disallow.clone(),
        base: state.cfg.base_url(),
    };
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        page.render().unwrap_or_default() + "\n",
    )
        .into_response()
}

pub struct ActivityItem {
    pub owner_gravatar: String,
    pub text: String,
    pub publisher_gravatar: String,
    pub created_at: String,
}

pub struct SimpleCookbook {
    pub name: String,
    pub latest_version: String,
    pub owner_username: String,
    pub owner_avatar: String,
    pub followers_text: String,
}

#[derive(Template)]
#[template(path = "pages/dashboard.html")]
struct Dashboard<'a> {
    shell: Shell<'a>,
    viewer_username: String,
    activity: Vec<ActivityItem>,
    followed_count: String,
    cookbooks: Vec<SimpleCookbook>,
    collaborated: Vec<SimpleCookbook>,
    tools: Vec<crate::web::tools::Tool>,
    tools_enabled: bool,
}

pub async fn dashboard(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let Some(viewer) = web::viewer(&state, &headers).await else {
        return web::redirect_with_flash(
            &state,
            &headers,
            "/sign-in",
            "notice",
            &crate::locales::t("user.must_be_signed_in", &[]),
        );
    };
    let activity = activity(&state.db, viewer.id).await;
    let followed: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM cookbook_followers WHERE user_id = $1")
            .bind(viewer.id)
            .fetch_one(&state.db)
            .await
            .unwrap_or(0);
    let cookbooks = simple_cookbooks(
        &state.db,
        "SELECT c.name, v.version, a.username, u.email, u.id, u.first_name, u.last_name, c.cookbook_followers_count
           FROM cookbooks c JOIN users u ON u.id = c.user_id
           JOIN accounts a ON a.user_id = u.id AND a.provider = 'chef_oauth2'
           JOIN LATERAL (SELECT version FROM cookbook_versions WHERE cookbook_id = c.id ORDER BY id DESC LIMIT 1) v ON true
          WHERE c.user_id = $1 LIMIT 5",
        viewer.id,
    )
    .await;
    let collaborated = simple_cookbooks(
        &state.db,
        "SELECT c.name, v.version, a.username, u.email, u.id, u.first_name, u.last_name, c.cookbook_followers_count
           FROM cookbooks c JOIN users u ON u.id = c.user_id
           JOIN accounts a ON a.user_id = u.id AND a.provider = 'chef_oauth2'
           JOIN collaborators col ON col.resourceable_id = c.id AND col.resourceable_type = 'Cookbook' AND col.user_id = $1
           JOIN LATERAL (SELECT version FROM cookbook_versions WHERE cookbook_id = c.id ORDER BY id DESC LIMIT 1) v ON true
          LIMIT 5",
        viewer.id,
    )
    .await;
    let tools = crate::web::tools::simple_tools(&state.db, viewer.id).await;
    let shell = shell(&state, Some(&viewer), &headers).title("Dashboard");
    let page = Dashboard {
        shell,
        viewer_username: viewer.username.clone(),
        activity,
        followed_count: web::pluralize(followed, "Cookbook"),
        cookbooks,
        collaborated,
        tools,
        tools_enabled: state.cfg.feature_active("tools"),
    };
    Page::ok(page).into_response()
}

#[allow(clippy::type_complexity)]
async fn activity(db: &PgPool, user_id: i32) -> Vec<ActivityItem> {
    let rows: Vec<(
        String,
        i32,
        String,
        String,
        String,
        String,
        String,
        String,
        chrono::NaiveDateTime,
    )> = sqlx::query_as(
        "SELECT c.name, v.version, a.username, ou.email, ou.id, ou.first_name, ou.last_name,
                    COALESCE(pu.email, ou.email), v.created_at
               FROM cookbook_versions v
               JOIN cookbooks c ON c.id = v.cookbook_id
               JOIN cookbook_followers f ON f.cookbook_id = c.id AND f.user_id = $1
               JOIN users ou ON ou.id = c.user_id
               JOIN accounts a ON a.user_id = ou.id AND a.provider = 'chef_oauth2'
               LEFT JOIN users pu ON pu.id = v.user_id
              ORDER BY v.created_at DESC LIMIT 50",
    )
    .bind(user_id)
    .fetch_all(db)
    .await
    .unwrap_or_default();
    rows.into_iter()
        .map(|(name, version, username, owner_email, _owner_id, first, last, publisher_email, created_at)| {
            let owner_name = display_name(&first, &last, &username);
            let publisher = first.clone();
            let _ = publisher;
            let publisher_name = owner_name.clone();
            ActivityItem {
                owner_gravatar: web::gravatar(0, &owner_name, &owner_email, 48),
                text: format!(
                    r#"<a href="/users/{username}">{owner_name}</a>: <a href="/cookbooks/{name}">{name}</a> v{version} released by <a href="/users/{username}">{publisher_name}</a>"#
                ),
                publisher_gravatar: web::gravatar(0, &publisher_name, &publisher_email, 48),
                created_at: web::longish_naive(&created_at),
            }
        })
        .collect()
}

pub fn display_name(first: &str, last: &str, username: &str) -> String {
    match (first.is_empty(), last.is_empty()) {
        (false, false) => format!("{first} {last}"),
        (false, true) => first.to_owned(),
        (true, false) => last.to_owned(),
        (true, true) => username.to_owned(),
    }
}

#[allow(clippy::type_complexity)]
async fn simple_cookbooks(db: &PgPool, sql: &str, bind: i32) -> Vec<SimpleCookbook> {
    let rows: Vec<(String, String, String, String, i32, String, String, i32)> = sqlx::query_as(sql)
        .bind(bind)
        .fetch_all(db)
        .await
        .unwrap_or_default();
    rows.into_iter()
        .map(
            |(name, version, username, email, _id, first, last, followers)| {
                let owner_name = display_name(&first, &last, &username);
                SimpleCookbook {
                    name,
                    latest_version: version,
                    owner_username: username,
                    owner_avatar: web::gravatar(0, &owner_name, &email, 128),
                    followers_text: web::pluralize(followers as i64, "Follower"),
                }
            },
        )
        .collect()
}

pub fn error_message(status: StatusCode) -> &'static str {
    match status {
        StatusCode::NOT_FOUND => "Not Found",
        _ => "Internal Server Error",
    }
}
