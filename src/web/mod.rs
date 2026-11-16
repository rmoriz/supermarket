//! HTML rendering: viewer/session extraction, asset manifest, URL and format
//! helpers shared by every page, and the response wrapper.

use std::collections::HashMap;
use std::sync::OnceLock;

use askama::Template;
use axum::body::Body;
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use sqlx::PgPool;

use crate::config::Config;
use crate::session;
use crate::state::AppState;

pub mod email_preferences;
pub mod escape;

/// Template filters: askama's own, plus the Rails helpers templates call.
pub mod filters {
    pub use askama::filters::*;

    /// An `Option<String>` rendered as the empty string when absent (Rails'
    /// `value: nil` producing `value=""`).
    #[askama::filter_fn]
    pub fn or_empty<T: std::fmt::Display>(
        value: &Option<T>,
        _: &dyn askama::Values,
    ) -> askama::Result<String> {
        Ok(value.as_ref().map(ToString::to_string).unwrap_or_default())
    }

    /// `flash_message_class_for`.
    #[askama::filter_fn]
    pub fn flash_class<T: AsRef<str>>(
        name: T,
        _: &dyn askama::Values,
    ) -> askama::Result<&'static str> {
        Ok(match name.as_ref() {
            "notice" => "success",
            "alert" => "alert",
            "warning" => "warning",
            _ => "alert",
        })
    }
}

pub mod accounts;
pub mod collaborators;
pub mod cookbooks;
pub mod group_members;
pub mod groups;
pub mod irc_logs;
pub mod markdown;
pub mod pages;
pub mod profile;
pub mod sessions;
pub mod tools;
pub mod transfer_ownership;
pub mod users;

/// The signed-in user, loaded once per request from the session cookie.
#[derive(Debug, Clone)]
pub struct Viewer {
    pub id: i32,
    pub username: String,
    pub name: String,
    pub email: String,
    pub admin: bool,
    pub install_preference: Option<String>,
}

impl Viewer {
    pub fn initial(&self) -> String {
        self.name.chars().next().unwrap_or('?').to_string()
    }

    pub fn gravatar(&self) -> String {
        gravatar(self.id, &self.name, &self.email, 80)
    }

    pub fn possessive(&self) -> String {
        Shell::posessivize(&self.name)
    }
}

/// Decodes the session cookie from the request and loads the user it names.
pub async fn viewer(state: &AppState, headers: &HeaderMap) -> Option<Viewer> {
    let user_id = session_from(state, headers).and_then(|session| session.user_id())?;
    load_viewer(&state.db, user_id).await
}

#[allow(clippy::type_complexity)]
pub async fn load_viewer(db: &PgPool, user_id: i32) -> Option<Viewer> {
    let row: Option<(
        i32,
        String,
        Option<String>,
        Option<String>,
        String,
        i32,
        Option<String>,
    )> = sqlx::query_as(
        "SELECT u.id, COALESCE(a.username, ''), u.first_name, u.last_name, u.email,
                    COALESCE(u.roles_mask, 0), u.install_preference
               FROM users u
               LEFT JOIN accounts a ON a.user_id = u.id AND a.provider = 'chef_oauth2'
              WHERE u.id = $1
              ORDER BY a.id LIMIT 1",
    )
    .bind(user_id)
    .fetch_optional(db)
    .await
    .ok()?;
    let (id, username, first, last, email, roles, install_preference) = row?;
    let name = match (
        first.filter(|v| !v.is_empty()),
        last.filter(|v| !v.is_empty()),
    ) {
        (Some(first), Some(last)) => format!("{first} {last}"),
        (Some(first), None) => first,
        (None, Some(last)) => last,
        (None, None) => username.clone(),
    };
    Some(Viewer {
        id,
        username,
        name,
        email,
        admin: roles & 1 == 1,
        install_preference,
    })
}

pub fn session_from(state: &AppState, headers: &HeaderMap) -> Option<session::Session> {
    let cookie = headers.get(header::COOKIE)?.to_str().ok()?;
    let raw = cookie.split(';').find_map(|piece| {
        let piece = piece.trim();
        piece
            .strip_prefix(session::COOKIE_NAME)
            .and_then(|rest| rest.strip_prefix('='))
    })?;
    // Rack percent-decodes cookie values; the ciphertext contains `+`, `/` and `=`.
    let decoded: String = form_urlencoded::parse(format!("x={raw}").as_bytes())
        .map(|(_, value)| value.into_owned())
        .next()
        .unwrap_or_else(|| raw.to_owned());
    let session = session::decode(&decoded, &state.cfg.secret_key_base).ok()?;
    crate::middleware::mark_session_touched();
    Some(session)
}

/// Redirect after writing a flash message into the session (Rails' `flash[...]`
/// followed by `redirect_to`).
pub fn redirect_with_flash(
    state: &AppState,
    headers: &HeaderMap,
    path: &str,
    kind: &str,
    message: &str,
) -> Response {
    let mut response = redirect(state, path);
    let mut session =
        session_from(state, headers).unwrap_or_else(|| session::Session::new(String::new()));
    session.set_flash(kind, message);
    write_session(&mut response, &session, state);
    response
}

/// Redirect to `path` after letting `edit` change the session.
pub fn redirect_with_session(
    state: &AppState,
    headers: &HeaderMap,
    path: &str,
    edit: impl FnOnce(&mut session::Session),
) -> Response {
    let mut response = redirect(state, path);
    let mut session =
        session_from(state, headers).unwrap_or_else(|| session::Session::new(String::new()));
    edit(&mut session);
    write_session(&mut response, &session, state);
    response
}

/// Redirect with both a session edit and a flash message.
pub fn redirect_with_session_and_flash(
    state: &AppState,
    headers: &HeaderMap,
    path: &str,
    kind: &str,
    message: &str,
    edit: impl FnOnce(&mut session::Session),
) -> Response {
    let mut response = redirect(state, path);
    let mut session =
        session_from(state, headers).unwrap_or_else(|| session::Session::new(String::new()));
    edit(&mut session);
    session.set_flash(kind, message);
    write_session(&mut response, &session, state);
    response
}

/// `authenticate_user!` with the location stored first: guests get a redirect
/// to `/sign-in`, the return path and the notice in their session.
pub fn redirect_to_sign_in(state: &AppState, headers: &HeaderMap, return_to: &str) -> Response {
    let mut response = redirect(state, "/sign-in");
    let mut session =
        session_from(state, headers).unwrap_or_else(|| session::Session::new(String::new()));
    session.set_return_to(return_to);
    session.set_flash("notice", &crate::locales::t("user.must_be_signed_in", &[]));
    write_session(&mut response, &session, state);
    response
}

/// Re-encodes `session` into the response's `Set-Cookie` header.
pub fn write_session(response: &mut Response, session: &session::Session, state: &AppState) {
    let raw = session::encode(session, &state.cfg.secret_key_base);
    let encoded: String = form_urlencoded::byte_serialize(raw.as_bytes()).collect();
    if let Ok(value) = format!(
        "{}={encoded}; path=/; HttpOnly; SameSite=Lax",
        session::COOKIE_NAME
    )
    .parse()
    {
        response.headers_mut().append(header::SET_COOKIE, value);
    }
}

/// Flash messages carried in the session, in insertion order.
pub fn flash(state: &AppState, headers: &HeaderMap) -> Vec<(String, String)> {
    let Some(session) = session_from(state, headers) else {
        return Vec::new();
    };
    let Some(map) = session.data().get("flash").and_then(|v| v.as_object()) else {
        return Vec::new();
    };
    map.iter()
        .filter_map(|(name, value)| {
            value
                .as_str()
                .map(|message| (name.clone(), message.to_owned()))
        })
        .collect()
}

pub fn csrf_token(state: &AppState, headers: &HeaderMap) -> String {
    session_from(state, headers)
        .map(|session| session::mask_token(&session::real_csrf(&session)))
        .unwrap_or_default()
}

/// Turns bracket-encoded form pairs (`user[email_preferences_attributes][0][id]`)
/// into a JSON tree so nested structs deserialize the way Rack's `params` does.
pub fn pairs_to_value(pairs: Vec<(String, String)>) -> serde_json::Value {
    fn segments(name: &str) -> Vec<&str> {
        name.trim_end_matches(']').split('[').collect()
    }

    fn insert(parent: &mut serde_json::Value, key: &str, rest: &[&str], value: String) {
        let child = match parent {
            serde_json::Value::Object(map) => {
                map.entry(key.to_owned()).or_insert(serde_json::Value::Null)
            }
            serde_json::Value::Array(items) => {
                let position = if key.is_empty() {
                    items.push(serde_json::Value::Null);
                    items.len() - 1
                } else if key.chars().all(|c| c.is_ascii_digit()) {
                    let index = key.parse().unwrap_or(items.len());
                    while items.len() <= index {
                        items.push(serde_json::Value::Null);
                    }
                    index
                } else {
                    // `a[][b]` and friends: extend the element being built.
                    if !matches!(items.last(), Some(serde_json::Value::Object(_))) {
                        items.push(serde_json::Value::Object(serde_json::Map::new()));
                    }
                    items.len() - 1
                };
                &mut items[position]
            }
            other => {
                *other = serde_json::Value::Object(serde_json::Map::new());
                let serde_json::Value::Object(map) = other else {
                    unreachable!()
                };
                map.entry(key.to_owned()).or_insert(serde_json::Value::Null)
            }
        };
        match rest.split_first() {
            None => *child = serde_json::Value::String(value),
            Some((next, tail)) => {
                if child.is_null() {
                    *child = if next.is_empty() || next.chars().all(|c| c.is_ascii_digit()) {
                        serde_json::Value::Array(Vec::new())
                    } else {
                        serde_json::Value::Object(serde_json::Map::new())
                    };
                }
                insert(child, next, tail, value);
            }
        }
    }

    let mut root = serde_json::Map::new();
    for (name, value) in pairs {
        let path = segments(&name);
        if let Some((head, rest)) = path.split_first() {
            insert(
                &mut serde_json::Value::Object(root.clone()),
                "",
                &[],
                String::new(),
            );
            let mut wrapper = serde_json::Value::Object(root);
            insert(&mut wrapper, head, rest, value);
            let serde_json::Value::Object(next) = wrapper else {
                unreachable!()
            };
            root = next;
        }
    }
    serde_json::Value::Object(root)
}

/// Deserializes bracket-encoded form pairs into `T`; `None` when they do not
/// fit (Rails would answer 400 there).
pub fn from_pairs<T: serde::de::DeserializeOwned>(pairs: Vec<(String, String)>) -> Option<T> {
    serde_json::from_value(pairs_to_value(pairs)).ok()
}

/// Rails' `flash_message_class_for`.
pub fn flash_class(name: &str) -> &'static str {
    match name {
        "notice" => "success",
        "alert" => "alert",
        "warning" => "warning",
        _ => "alert",
    }
}

// ---------------------------------------------------------------- assets

static MANIFEST: OnceLock<HashMap<String, String>> = OnceLock::new();

fn manifest() -> &'static HashMap<String, String> {
    MANIFEST.get_or_init(|| {
        let mut map = HashMap::new();
        let Ok(entries) = std::fs::read_dir("static/assets") else {
            return map;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.ends_with(".gz") {
                continue;
            }
            // `logical-<64 hex>.<ext>`; keep the first digest deterministically.
            let Some((stem, ext)) = name.rsplit_once('.') else {
                continue;
            };
            let Some((logical, digest)) = stem.rsplit_once('-') else {
                continue;
            };
            if digest.len() < 16 || !digest.bytes().all(|b| b.is_ascii_hexdigit()) {
                continue;
            }
            map.entry(format!("{logical}.{ext}")).or_insert(name);
        }
        map
    })
}

/// Root-relative asset path, or the plain name when the manifest has no entry.
pub fn asset_path(name: &str) -> String {
    match manifest().get(name) {
        Some(hashed) => format!("/assets/{hashed}"),
        None => format!("/assets/{name}"),
    }
}

pub fn image_url(cfg: &Config, name: &str) -> String {
    format!("{}{}", cfg.base_url(), asset_path(name))
}

// ---------------------------------------------------------------- helpers

pub fn pluralize(count: i64, singular: &str) -> String {
    if count == 1 {
        format!("1 {singular}")
    } else {
        format!("{count} {singular}s")
    }
}

/// Rails' `pluralize` with `number_with_delimiter`, as the welcome page uses it.
pub fn pluralize_commas(count: i64, singular: &str) -> String {
    let word = if count == 1 {
        singular.to_owned()
    } else {
        format!("{singular}s")
    };
    format!("{} {word}", number_with_delimiter(count))
}

pub fn number_with_delimiter(value: i64) -> String {
    let text = value.to_string();
    let mut out = String::new();
    for (index, ch) in text.chars().enumerate() {
        if index > 0 && (text.len() - index) % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// Rails' `to_fs(:longish)`: `%B %-d, %Y %H:%M`.
pub fn longish(value: &DateTime<Utc>) -> String {
    value.format("%B %-d, %Y").to_string()
}

pub fn longish_naive(value: &chrono::NaiveDateTime) -> String {
    value.format("%B %-d, %Y").to_string()
}

/// `Feature.active?(:gravatar)`, latched once at startup.
static GRAVATAR: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

pub fn set_gravatar(enabled: bool) {
    GRAVATAR.store(enabled, std::sync::atomic::Ordering::Relaxed);
}

/// `gravatar_for`: the Gravatar image, or the bundled icon when the feature is
/// inactive.
pub fn gravatar(user_id: i32, name: &str, email: &str, size: u32) -> String {
    if !GRAVATAR.load(std::sync::atomic::Ordering::Relaxed) {
        return format!(
            r#"<img alt="{}" class="gravatar" src="{}" />"#,
            html_escape(name),
            asset_path("apple-touch-icon.png")
        );
    }
    let digest = format!("{:x}", md5::compute(email.to_lowercase()));
    format!(
        r#"<img alt="{name}" class="gravatar" src="https://secure.gravatar.com/avatar/{digest}?s={size}" />"#,
        name = html_escape(name)
    )
    .replace("data-id", &user_id.to_string())
}

pub fn html_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Rails' `distance_of_time_in_words` (the `time_ago_in_words` helper),
/// including its leap-day discount.
pub fn time_ago_in_words(value: &chrono::NaiveDateTime) -> String {
    use chrono::{Datelike, Utc};

    const MINUTES_IN_YEAR: i64 = 525_600;
    const MINUTES_IN_QUARTER_YEAR: i64 = 131_400;
    const MINUTES_IN_THREE_QUARTERS_YEAR: i64 = 394_200;

    let now = Utc::now().naive_utc();
    let seconds = (now - *value).num_seconds().max(0);
    let minutes = (seconds + 30) / 60;
    let plural =
        |count: i64, unit: &str| format!("{count} {unit}{}", if count == 1 { "" } else { "s" });

    match minutes {
        0..=1 => plural(1, "minute"),
        2..=44 => plural(minutes, "minute"),
        45..=89 => "about 1 hour".into(),
        90..=1439 => format!("about {} hours", (minutes as f64 / 60.0).round() as i64),
        1440..=2519 => "1 day".into(),
        2520..=43199 => plural((minutes as f64 / 1440.0).round() as i64, "day"),
        43200..=86399 => format!("about {} months", (minutes as f64 / 43200.0).round() as i64),
        86400..=525_599 => plural((minutes as f64 / 43200.0).round() as i64, "month"),
        _ => {
            let mut from_year = value.year();
            if value.month() >= 3 {
                from_year += 1;
            }
            let mut to_year = now.year();
            if now.month() < 3 {
                to_year -= 1;
            }
            let leap_years = if from_year > to_year {
                0
            } else {
                (from_year..=to_year)
                    .filter(|year| chrono::NaiveDate::from_ymd_opt(*year, 2, 29).is_some())
                    .count() as i64
            };
            let minutes_with_offset = minutes - leap_years * 1440;
            let remainder = minutes_with_offset % MINUTES_IN_YEAR;
            let years = minutes_with_offset.div_euclid(MINUTES_IN_YEAR);
            if remainder < MINUTES_IN_QUARTER_YEAR {
                plural(years, "year")
            } else if remainder < MINUTES_IN_THREE_QUARTERS_YEAR {
                format!("over {} year{}", years, if years == 1 { "" } else { "s" })
            } else {
                format!("almost {} years", years + 1)
            }
        }
    }
}

/// A rendered page with its status.
/// Rails' `redirect_to`: 302 with an absolute Location.
pub fn redirect(state: &AppState, path: &str) -> Response {
    let location = format!("{}{path}", state.cfg.base_url());
    let mut response = StatusCode::FOUND.into_response();
    if let Ok(value) = location.parse() {
        response.headers_mut().insert(header::LOCATION, value);
    }
    response
}

pub struct Page {
    pub status: StatusCode,
    pub body: String,
}

impl Page {
    pub fn ok<T: Template>(template: T) -> Self {
        Self {
            status: StatusCode::OK,
            body: template.render().unwrap_or_default(),
        }
    }
}

impl IntoResponse for Page {
    fn into_response(self) -> Response {
        // `preload_links_header`: the layout's stylesheet/script/font hints.
        let link = format!(
            "<{}>; rel=preload; as=style; nopush,<//fonts.googleapis.com/css?family=Montserrat:400,700>; rel=preload; as=style; nopush,<{}>; rel=preload; as=script; nopush",
            asset_path("application.css"),
            asset_path("application.js")
        );
        (
            self.status,
            [
                (header::CONTENT_TYPE, "text/html; charset=utf-8".to_owned()),
                (header::LINK, link),
            ],
            Body::from(self.body),
        )
            .into_response()
    }
}

/// The shared layout context.
/// `Badgeable::BADGES` and the platform list `_search.html.erb` offers.
pub const BADGE_OPTIONS: &[&str] = &["partner"];

pub const PLATFORM_OPTIONS: &[&str] = &[
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

pub struct Shell<'a> {
    pub title: Option<String>,
    pub description: Option<String>,
    pub viewer: Option<&'a Viewer>,
    pub flash: Vec<(String, String)>,
    pub search_path: String,
    pub search_name: &'static str,
    pub q: Option<String>,
    pub order: Option<String>,
    pub badges: Vec<String>,
    pub platforms: Vec<String>,
    pub badge_options: &'static [&'static str],
    pub platform_options: &'static [&'static str],
    pub deprecated: bool,
    pub csrf: String,
    pub tools: bool,
    pub fieri: bool,
    pub announcement: bool,
    pub collaborator_groups: bool,
    pub no_crawl: bool,
    pub announcement_text: Option<String>,
    pub announcement_banner: bool,
    pub year: i32,
    pub favicon: String,
    pub apple_touch_icon: String,
    pub stylesheet: String,
    pub script: String,
    pub fonts: bool,
    pub analytics: String,
    pub air_gapped: bool,
}

impl<'a> Shell<'a> {
    pub fn badge_checked(&self, badge: &str) -> bool {
        self.badges.iter().any(|b| b == badge)
    }

    pub fn platform_checked(&self, platform: &str) -> bool {
        self.platforms.iter().any(|p| p == platform)
    }

    pub fn new(state: &AppState, viewer: Option<&'a Viewer>, headers: &HeaderMap) -> Self {
        let cfg = &state.cfg;
        let air_gapped = std::env::var("AIR_GAPPED").as_deref() == Ok("true");
        Shell {
            title: None,
            description: None,
            viewer,
            flash: flash(state, headers),
            search_path: "/cookbooks".into(),
            search_name: "Cookbooks",
            q: None,
            order: None,
            badges: Vec::new(),
            platforms: Vec::new(),
            badge_options: BADGE_OPTIONS,
            platform_options: PLATFORM_OPTIONS,
            deprecated: false,
            csrf: csrf_token(state, headers),
            tools: cfg.feature_active("tools"),
            fieri: cfg.feature_active("fieri"),
            announcement: cfg.feature_active("announcement"),
            collaborator_groups: cfg.feature_active("collaborator_groups"),
            no_crawl: cfg.feature_active("no_crawl"),
            announcement_text: std::env::var("ANNOUNCEMENT_TEXT")
                .ok()
                .filter(|v| !v.is_empty()),
            announcement_banner: std::env::var("ANNOUNCEMENT_BANNER").as_deref() == Ok("true"),
            year: Utc::now().format("%Y").to_string().parse().unwrap_or(2026),
            favicon: image_url(cfg, "favicon.ico"),
            apple_touch_icon: image_url(cfg, "apple-touch-icon.png"),
            stylesheet: asset_path("application.css"),
            script: asset_path("application.js"),
            fonts: true,
            analytics: String::new(),
            air_gapped,
        }
    }

    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    pub fn search(mut self, path: &str, name: &'static str, q: Option<String>) -> Self {
        self.search_path = path.to_owned();
        self.search_name = name;
        self.q = q;
        self
    }

    pub fn params(
        mut self,
        badges: Vec<String>,
        platforms: Vec<String>,
        order: Option<String>,
        deprecated: bool,
    ) -> Self {
        self.badges = badges;
        self.platforms = platforms;
        self.order = order;
        self.deprecated = deprecated;
        self
    }

    /// Rails' custom URL helpers, resolved once per page.
    pub fn chef_domain() -> String {
        std::env::var("CHEF_DOMAIN").unwrap_or_else(|_| "chef.io".into())
    }

    pub fn progress_domain() -> String {
        std::env::var("PROGRESS_DOMAIN").unwrap_or_else(|_| "progress.com".into())
    }

    pub fn chef_www_url() -> String {
        std::env::var("CHEF_WWW_URL")
            .unwrap_or_else(|_| format!("https://www.{}", Self::chef_domain()))
    }

    pub fn progress_www_url(extra: Option<&str>) -> String {
        let base = std::env::var("PROGRESS_WWW_URL")
            .unwrap_or_else(|_| format!("https://www.{}", Self::progress_domain()));
        dispatch(&base, extra)
    }

    pub fn chef_docs_url(extra: Option<&str>) -> String {
        let base = std::env::var("CHEF_DOCS_URL")
            .unwrap_or_else(|_| format!("https://docs.{}", Self::chef_domain()));
        dispatch(&base, extra)
    }

    pub fn chef_downloads_url(extra: Option<&str>) -> String {
        let base = std::env::var("CHEF_DOWNLOADS_URL")
            .unwrap_or_else(|_| format!("https://www.{}/downloads", Self::chef_domain()));
        dispatch(&base, extra)
    }

    pub fn chef_blog_url(extra: Option<&str>) -> String {
        let base =
            std::env::var("CHEF_BLOG_URL").unwrap_or_else(|_| "https://chef.io/blog/".into());
        dispatch(&base, extra)
    }

    pub fn chef_sign_up_url() -> String {
        let manage = std::env::var("CHEF_MANAGE_URL")
            .or_else(|_| std::env::var("CHEF_SERVER_URL"))
            .unwrap_or_else(|_| "https://api.chef.io".into());
        let base = std::env::var("CHEF_SIGN_UP_URL")
            .unwrap_or_else(|_| format!("{manage}/signup?ref=community"));
        dispatch(&base, None)
    }

    pub fn chef_status_url() -> String {
        std::env::var("CHEF_STATUS_URL")
            .unwrap_or_else(|_| format!("https://status.{}", Self::chef_domain()))
    }

    pub fn learn_chef_url() -> String {
        std::env::var("LEARN_CHEF_URL")
            .unwrap_or_else(|_| format!("https://learn.{}", Self::chef_domain()))
    }

    pub fn chef_training_url() -> String {
        std::env::var("CHEF_TRAINING_URL")
            .unwrap_or_else(|_| format!("{}/training", Self::chef_www_url()))
    }

    pub fn chef_identity_url() -> String {
        let server =
            std::env::var("CHEF_SERVER_URL").unwrap_or_else(|_| "https://api.chef.io".into());
        std::env::var("CHEF_IDENTITY_URL").unwrap_or_else(|_| format!("{server}/id"))
    }

    pub fn github_profile_url(username: &str) -> String {
        let mut path = std::env::var("GITHUB_URL").unwrap_or_else(|_| "https://github.com/".into());
        if !path.ends_with('/') {
            path.push('/');
        }
        format!("{path}{username}")
    }

    pub fn posessivize(name: &str) -> String {
        if name.is_empty() {
            return String::new();
        }
        if name.ends_with('s') {
            format!("{name}'")
        } else {
            format!("{name}'s")
        }
    }
}

fn dispatch(url: &str, extra: Option<&str>) -> String {
    match extra {
        None => url.to_owned(),
        Some(extra) => format!("{url}/{extra}"),
    }
}

/// Kaminari's `paginate` output, with its default window (4), markup and locale
/// strings, as `app/views/kaminari/_paginator.html.erb` renders it.
pub mod pagination {
    const WINDOW: i64 = 4;

    fn page_url(base: &str, page: i64) -> String {
        if base.is_empty() {
            return format!("?page={page}");
        }
        if base.contains('?') {
            format!("{base}&amp;page={page}")
        } else {
            format!("{base}?page={page}")
        }
    }

    pub fn render(current: i64, total: i64, base: &str) -> String {
        render_with(current, total, |page| page_url(base, page))
    }

    /// Same, with the URL of each page supplied by the caller (used where the
    /// query string has to stay sorted around the `page` parameter).
    pub fn render_with<F>(current: i64, total: i64, url: F) -> String
    where
        F: Fn(i64) -> String,
    {
        if total <= 1 {
            return String::new();
        }
        let current = current.clamp(1, total);
        let mut out = String::from("<ul class=\"pagination\">");
        if current > 1 {
            out.push_str(&format!(
                "<li><a href=\"{}\">&laquo; First</a></li>",
                url(1)
            ));
            out.push_str(&format!(
                "<li><a rel=\"prev\" href=\"{}\">&lsaquo; Prev</a></li>",
                url(current - 1)
            ));
        }
        let mut previous: Option<i64> = None;
        for page in 1..=total {
            let visible = page == 1 || page == total || (page - current).abs() <= WINDOW;
            if !visible {
                if previous.is_some() {
                    out.push_str("<li class=\"unavailable\"><a href=\"#\">&hellip;</a></li>");
                    previous = None;
                }
                continue;
            }
            let class = if page == current {
                " class=\"current\""
            } else {
                ""
            };
            let rel = if page == current + 1 {
                " rel=\"next\""
            } else if page == current - 1 {
                " rel=\"prev\""
            } else {
                ""
            };
            out.push_str(&format!(
                "<li{class}><a{rel} href=\"{}\">{page}</a></li>",
                url(page)
            ));
            previous = Some(page);
        }
        if current < total {
            out.push_str(&format!(
                "<li><a rel=\"next\" href=\"{}\">Next &rsaquo;</a></li>",
                url(current + 1)
            ));
            out.push_str(&format!(
                "<li><a href=\"{}\">Last &raquo;</a></li>",
                url(total)
            ));
        }
        out.push_str("</ul>");
        out
    }
}
