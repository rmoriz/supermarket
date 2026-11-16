//! `UsersController` read paths: profile, a user's tools and groups, and the
//! followed-cookbook atom feed.

use crate::web::filters;
use askama::Template;
use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use chrono::NaiveDateTime;
use serde::Deserialize;
use sqlx::PgPool;

use crate::locales::t;
use crate::state::AppState;
use crate::web::cookbooks::Card;
use crate::web::pages;
use crate::web::{self, Page, Shell};

#[derive(Debug, Default, Deserialize)]
pub struct TabParams {
    pub tab: Option<String>,
    pub page: Option<i64>,
}

pub struct UserView {
    pub id: i32,
    pub username: String,
    pub name: String,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub public_key: Option<String>,
    pub email: String,
    pub avatar_large: String,
    pub company: Option<String>,
    pub twitter_username: Option<String>,
    pub slack_username: Option<String>,
    pub github_accounts: Vec<String>,
    pub joined: String,
    pub owned_stats: String,
    pub collaborated_stats: String,
    pub followed_stats: String,
    pub possessive: String,
}

pub struct GroupRow {
    pub id: i32,
    pub name: String,
}

#[allow(clippy::type_complexity)]
async fn find_user(db: &PgPool, username: &str) -> Option<UserView> {
    let row: Option<(
        i32,
        String,
        Option<String>,
        Option<String>,
        String,
        Option<String>,
        Option<String>,
        Option<String>,
        NaiveDateTime,
        Option<String>,
    )> = sqlx::query_as(
        "SELECT u.id, a.username, u.first_name, u.last_name, u.email, u.company,
                    u.twitter_username, u.slack_username, u.created_at, u.public_key
               FROM users u JOIN accounts a ON a.user_id = u.id AND a.provider = 'chef_oauth2'
              WHERE a.username = $1
              ORDER BY a.id LIMIT 1",
    )
    .bind(username)
    .fetch_optional(db)
    .await
    .ok()?;
    let (id, username, first, last, email, company, twitter, slack, created_at, public_key) = row?;
    let name = pages::display_name(
        first.as_deref().unwrap_or(""),
        last.as_deref().unwrap_or(""),
        &username,
    );
    let github: Vec<String> = sqlx::query_scalar(
        "SELECT username FROM accounts WHERE user_id = $1 AND provider = 'github' ORDER BY id",
    )
    .bind(id)
    .fetch_all(db)
    .await
    .unwrap_or_default();
    let owned: i64 = count(db, "SELECT COUNT(*) FROM cookbooks WHERE user_id = $1", id).await;
    let collaborated: i64 = count(
        db,
        "SELECT COUNT(*) FROM collaborators WHERE resourceable_type = 'Cookbook' AND user_id = $1",
        id,
    )
    .await;
    let followed: i64 = count(
        db,
        "SELECT COUNT(*) FROM cookbook_followers WHERE user_id = $1",
        id,
    )
    .await;
    Some(UserView {
        id,
        username: username.clone(),
        name: name.clone(),
        first_name: first,
        last_name: last,
        public_key: public_key.filter(|v| !v.is_empty()),
        email,
        avatar_large: web::gravatar(id, &name, &username, 180),
        company: company.filter(|v| !v.is_empty()),
        twitter_username: twitter.filter(|v| !v.is_empty()),
        slack_username: slack.filter(|v| !v.is_empty()),
        github_accounts: github,
        joined: web::time_ago_in_words(&created_at),
        owned_stats: pluralized_stats(owned, "Cookbook"),
        collaborated_stats: pluralized_stats(collaborated, "Collaboration"),
        followed_stats: pluralized_stats(followed, "Follow"),
        possessive: Shell::posessivize(&name),
    })
}

async fn count(db: &PgPool, sql: &str, bind: i32) -> i64 {
    sqlx::query_scalar(sql)
        .bind(bind)
        .fetch_one(db)
        .await
        .unwrap_or(0)
}

pub async fn find_user_view(db: &PgPool, username: &str) -> Option<UserView> {
    find_user(db, username).await
}

fn pluralized_stats(value: i64, thing: &str) -> String {
    let words = web::pluralize(value, thing);
    match words.split_once(' ') {
        Some((number, word)) => format!("{number} <span>{word}</span>"),
        None => words,
    }
}

#[derive(Template)]
#[template(path = "users/show.html")]
struct Show<'a> {
    shell: Shell<'a>,
    user: UserView,
    title: String,
    active: &'static str,
    tab: Option<String>,
    cookbooks: Vec<Card>,
    pagination: String,
    own_profile: bool,
    empty_current: String,
    empty_other: String,
    show_email: bool,
    make_admin: bool,
    revoke_admin: bool,
    profile_controller: bool,
    chef_icon: String,
    fieri: bool,
    quality_tip: String,
    tools_enabled: bool,
    collaborator_groups: bool,
    partner_badge: String,
}

pub async fn show(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(username): Path<String>,
    Query(params): Query<TabParams>,
) -> Response {
    let viewer = web::viewer(&state, &headers).await;
    let Some(user) = find_user(&state.db, &username).await else {
        return pages::error_page_as(&state, &headers, StatusCode::NOT_FOUND, false, "users").await;
    };
    let tab = params
        .tab
        .clone()
        .filter(|tab| tab == "collaborates" || tab == "follows");
    let viewer_id = viewer.as_ref().map(|v| v.id);
    let (sql, bind) = match tab.as_deref() {
        Some("collaborates") => (
            "SELECT c.name FROM cookbooks c JOIN collaborators col
               ON col.resourceable_type = 'Cookbook' AND col.resourceable_id = c.id
              WHERE col.user_id = $2 ORDER BY c.name LIMIT 20 OFFSET $3",
            user.id,
        ),
        Some("follows") => (
            "SELECT c.name FROM cookbooks c JOIN cookbook_followers f ON f.cookbook_id = c.id
              WHERE f.user_id = $2 ORDER BY c.name LIMIT 20 OFFSET $3",
            user.id,
        ),
        _ => (
            "SELECT c.name FROM cookbooks c WHERE c.user_id = $2 ORDER BY c.name LIMIT 20 OFFSET $3",
            user.id,
        ),
    };
    let page_number = params.page.unwrap_or(1).max(1);
    let offset = (page_number - 1) * 20;
    let names: Vec<String> = sqlx::query_scalar(sql)
        .bind(viewer_id)
        .bind(bind)
        .bind(offset)
        .fetch_all(&state.db)
        .await
        .unwrap_or_default();
    let total = match tab.as_deref() {
        Some("collaborates") => user.collaborated_stats_count(),
        Some("follows") => user.followed_stats_count(),
        _ => user.owned_stats_count(),
    };
    let cookbooks = crate::web::cookbooks::cards_by_name(&state.db, &names, viewer_id).await;
    let own_profile = viewer_id == Some(user.id);
    let empty_current = t(
        &format!(
            "profile.current_user_no_cookbooks.{}",
            tab.as_deref().unwrap_or("owns")
        ),
        &[],
    );
    let empty_other = t(
        &format!("profile.no_cookbooks.{}", tab.as_deref().unwrap_or("owns")),
        &[("username", &user.username)],
    );
    let make_admin =
        viewer.as_ref().map(|v| v.admin).unwrap_or(false) && !is_admin(&state.db, user.id).await;
    let revoke_admin =
        viewer.as_ref().map(|v| v.admin).unwrap_or(false) && viewer_id != Some(user.id);
    let title = format!("{} Cookbooks", user.possessive);
    let shell = pages::shell(&state, viewer.as_ref(), &headers)
        .title(user.username.clone())
        .description(format!("The Supermarket profile for {}.", user.username));
    let page = Show {
        shell,
        chef_icon: pages::chef_icon(),
        fieri: state.cfg.feature_active("fieri"),
        quality_tip: pages::quality_tip(),
        tools_enabled: state.cfg.feature_active("tools"),
        collaborator_groups: state.cfg.feature_active("collaborator_groups"),
        partner_badge: pages::partner_badge(),
        user,
        title,
        active: "cookbooks",
        tab: tab.clone(),
        cookbooks,
        pagination: web::pagination::render(
            page_number,
            ((total as f64) / 20.0).ceil() as i64,
            &format!("/users/{username}"),
        ),
        own_profile,
        empty_current,
        empty_other,
        show_email: viewer.as_ref().map(|v| v.admin).unwrap_or(false),
        make_admin,
        revoke_admin,
        profile_controller: false,
    };
    Page::ok(page).into_response()
}

/// `find_user` for an `assign_user`-style before action: unknown accounts 404.
async fn assigned_user(
    state: &AppState,
    headers: &HeaderMap,
    username: &str,
) -> Result<UserView, Box<Response>> {
    match find_user(&state.db, username).await {
        Some(user) => Ok(user),
        None => Err(Box::new(
            pages::error_page_as(state, headers, StatusCode::NOT_FOUND, false, "users").await,
        )),
    }
}

/// `PUT /users/:id/make_admin`: admins promote other users.
pub async fn make_admin(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(username): Path<String>,
) -> Response {
    let viewer = web::viewer(&state, &headers).await;
    let user = match assigned_user(&state, &headers, &username).await {
        Ok(user) => user,
        Err(response) => return *response,
    };
    let allowed =
        viewer.as_ref().map(|v| v.admin).unwrap_or(false) && !is_admin(&state.db, user.id).await;
    if !allowed {
        return pages::error_page_as(&state, &headers, StatusCode::NOT_FOUND, false, "users").await;
    }
    set_admin(&state, user.id, true).await;
    web::redirect_with_flash(
        &state,
        &headers,
        &format!("/users/{}", user.username),
        "notice",
        &t("user.made_admin", &[("name", &user.username)]),
    )
}

/// `DELETE /users/:id/revoke_admin`: admins demote everyone but themselves.
pub async fn revoke_admin(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(username): Path<String>,
) -> Response {
    let viewer = web::viewer(&state, &headers).await;
    let user = match assigned_user(&state, &headers, &username).await {
        Ok(user) => user,
        Err(response) => return *response,
    };
    let allowed = viewer.as_ref().map(|v| v.admin).unwrap_or(false)
        && viewer.as_ref().map(|v| v.id) != Some(user.id);
    if !allowed {
        return pages::error_page_as(&state, &headers, StatusCode::NOT_FOUND, false, "users").await;
    }
    set_admin(&state, user.id, false).await;
    web::redirect_with_flash(
        &state,
        &headers,
        &format!("/users/{}", user.username),
        "notice",
        &t("user.revoked_admin", &[("name", &user.username)]),
    )
}

/// `@user.roles = @user.roles +/- ["admin"]`: role bit 0 of `roles_mask`.
async fn set_admin(state: &AppState, user_id: i32, admin: bool) {
    let mask = if admin {
        "COALESCE(roles_mask, 0) | 1"
    } else {
        "COALESCE(roles_mask, 0) & ~1"
    };
    let _ = sqlx::query(&format!(
        "UPDATE users SET roles_mask = {mask}, updated_at = NOW() WHERE id = $1"
    ))
    .bind(user_id)
    .execute(&state.db)
    .await;
}

async fn is_admin(db: &PgPool, user_id: i32) -> bool {
    let mask: Option<i32> = sqlx::query_scalar("SELECT roles_mask FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_optional(db)
        .await
        .ok()
        .flatten();
    mask.unwrap_or(0) & 1 == 1
}

impl UserView {
    fn owned_stats_count(&self) -> i64 {
        stats_count(&self.owned_stats)
    }
    fn collaborated_stats_count(&self) -> i64 {
        stats_count(&self.collaborated_stats)
    }
    fn followed_stats_count(&self) -> i64 {
        stats_count(&self.followed_stats)
    }
}

fn stats_count(html: &str) -> i64 {
    html.split(' ')
        .next()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0)
}

#[derive(Template)]
#[template(path = "users/tools.html")]
struct Tools<'a> {
    shell: Shell<'a>,
    user: UserView,
    title: String,
    active: &'static str,
    tools: Vec<crate::web::tools::Tool>,
    pagination: String,
    own_profile: bool,
    empty_current: String,
    empty_other: String,
    show_email: bool,
    make_admin: bool,
    revoke_admin: bool,
    profile_controller: bool,
    chef_icon: String,
    tools_enabled: bool,
    collaborator_groups: bool,
}

pub async fn tools(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(username): Path<String>,
    Query(params): Query<TabParams>,
) -> Response {
    let viewer = web::viewer(&state, &headers).await;
    let Some(user) = find_user(&state.db, &username).await else {
        return pages::error_page_as(&state, &headers, StatusCode::NOT_FOUND, false, "users").await;
    };
    let viewer_id = viewer.as_ref().map(|v| v.id);
    let page_number = params.page.unwrap_or(1).max(1);
    let offset = (page_number - 1) * 20;
    let tools = crate::web::tools::tools_of(&state.db, user.id, viewer_id, offset).await;
    let total: i64 = count(
        &state.db,
        "SELECT COUNT(*) FROM tools WHERE user_id = $1",
        user.id,
    )
    .await;
    let own_profile = viewer_id == Some(user.id);
    let make_admin =
        viewer.as_ref().map(|v| v.admin).unwrap_or(false) && !is_admin(&state.db, user.id).await;
    let revoke_admin =
        viewer.as_ref().map(|v| v.admin).unwrap_or(false) && viewer_id != Some(user.id);
    let title = format!("{} Tools", user.possessive);
    let shell = pages::shell(&state, viewer.as_ref(), &headers)
        .title(user.username.clone())
        .description(format!("{}'s Chef tools and plugins.", user.username))
        .search("/tools", "Tools", None);
    let page = Tools {
        shell,
        chef_icon: pages::chef_icon(),
        title,
        active: "maintains",
        tools,
        pagination: web::pagination::render(
            page_number,
            ((total as f64) / 20.0).ceil() as i64,
            &format!("/users/{username}/tools"),
        ),
        own_profile,
        empty_current: t("profile.current_user_no_tools", &[]),
        empty_other: t("profile.no_tools", &[("username", &user.username)]),
        show_email: viewer.as_ref().map(|v| v.admin).unwrap_or(false),
        make_admin,
        revoke_admin,
        profile_controller: false,
        user,
        tools_enabled: true,
        collaborator_groups: state.cfg.feature_active("collaborator_groups"),
    };
    Page::ok(page).into_response()
}

#[derive(Template)]
#[template(path = "users/groups.html")]
struct Groups<'a> {
    shell: Shell<'a>,
    user: UserView,
    own_profile: bool,
    title: String,
    active: &'static str,
    groups: Vec<GroupRow>,
    show_email: bool,
    make_admin: bool,
    revoke_admin: bool,
    profile_controller: bool,
    chef_icon: String,
    tools_enabled: bool,
    collaborator_groups: bool,
}

pub async fn groups(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(username): Path<String>,
) -> Response {
    let viewer = web::viewer(&state, &headers).await;
    let Some(user) = find_user(&state.db, &username).await else {
        return pages::error_page_as(&state, &headers, StatusCode::NOT_FOUND, false, "users").await;
    };
    let viewer_id = viewer.as_ref().map(|v| v.id);
    let rows: Vec<(i32, String)> = sqlx::query_as(
        "SELECT g.id, g.name FROM groups g JOIN group_members gm ON gm.group_id = g.id
          WHERE gm.user_id = $1 ORDER BY g.name",
    )
    .bind(user.id)
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();
    let make_admin =
        viewer.as_ref().map(|v| v.admin).unwrap_or(false) && !is_admin(&state.db, user.id).await;
    let revoke_admin =
        viewer.as_ref().map(|v| v.admin).unwrap_or(false) && viewer_id != Some(user.id);
    let title = format!("{} Groups", user.possessive);
    let shell = pages::shell(&state, viewer.as_ref(), &headers)
        .title(user.username.clone())
        .description(format!("{}'s Groups.", user.username));
    let page = Groups {
        shell,
        chef_icon: pages::chef_icon(),
        title,
        active: "groups",
        groups: rows
            .into_iter()
            .map(|(id, name)| GroupRow { id, name })
            .collect(),
        own_profile: viewer_id == Some(user.id),
        show_email: viewer.as_ref().map(|v| v.admin).unwrap_or(false),
        make_admin,
        revoke_admin,
        profile_controller: false,
        user,
        tools_enabled: true,
        collaborator_groups: state.cfg.feature_active("collaborator_groups"),
    };
    Page::ok(page).into_response()
}

#[derive(Template)]
#[template(path = "users/followed_cookbook_activity.atom", escape = "none")]
struct Feed {
    host: String,
    base: String,
    username: String,
    updated: String,
    entries: Vec<FeedEntry>,
}

pub struct FeedEntry {
    pub id: String,
    pub url: String,
    pub title: String,
    pub content: String,
    pub author: String,
    pub author_url: String,
    pub updated: String,
}

#[allow(clippy::type_complexity)]
pub async fn followed_cookbook_activity(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(username): Path<String>,
) -> Response {
    let Some(user) = find_user(&state.db, &username).await else {
        return pages::error_page_as(&state, &headers, StatusCode::NOT_FOUND, false, "users").await;
    };
    let rows: Vec<(String, String, i32, String, String, String, String, NaiveDateTime, String, String)> =
        sqlx::query_as(
            "SELECT c.name, v.version, v.id, a.username, v.description, u.first_name, u.last_name,
                    v.created_at, COALESCE(pu.first_name, ou.first_name, ''), COALESCE(pu.last_name, ou.last_name, '')
               FROM cookbook_versions v
               JOIN cookbooks c ON c.id = v.cookbook_id
               JOIN cookbook_followers f ON f.cookbook_id = c.id AND f.user_id = $1
               JOIN users u ON u.id = c.user_id
               JOIN accounts a ON a.user_id = u.id AND a.provider = 'chef_oauth2'
               JOIN users ou ON ou.id = c.user_id
               LEFT JOIN users pu ON pu.id = v.user_id
              ORDER BY v.created_at DESC LIMIT 50",
        )
        .bind(user.id)
        .fetch_all(&state.db)
        .await
        .unwrap_or_default();
    let mut entries = Vec::new();
    for (name, version, id, username_owner, description, first, last, created_at, pfirst, plast) in
        rows
    {
        let owner_name = pages::display_name(&first, &last, &username_owner);
        let publisher = pages::display_name(&pfirst, &plast, &username_owner);
        entries.push(FeedEntry {
            id: format!("tag:supermarket,2005:{}", id),
            url: format!("/cookbooks/{name}/versions/{version}"),
            title: t(
                "cookbook.activity",
                &[
                    ("maintainer", &owner_name),
                    ("version", &version),
                    ("cookbook", &name),
                    ("publisher", &publisher),
                ],
            ),
            content: format!("<p>{}</p>", web::html_escape(&description)),
            author: publisher,
            author_url: format!("/users/{username_owner}"),
            updated: created_at.format("%Y-%m-%dT%H:%M:%SZ").to_string(),
        });
    }
    // `safe_updated_at` falls back to the current time for an empty feed.
    let updated = entries
        .first()
        .map(|entry| entry.updated.clone())
        .unwrap_or_else(|| chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string());
    let feed = Feed {
        host: crate::web::cookbooks::host_of(&headers),
        base: state.cfg.base_url(),
        username: user.username,
        updated,
        entries,
    };
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/atom+xml; charset=utf-8")],
        feed.render().unwrap_or_default() + "\n",
    )
        .into_response()
}
