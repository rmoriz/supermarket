//! `CookbooksController` read paths: directory, index/search, show, adoption
//! list and version pages.

use std::collections::BTreeMap;

use crate::web::filters;
use askama::Template;
use axum::extract::{Form, Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use cadence::CountedExt;
use serde::Deserialize;
use sqlx::PgPool;

use crate::chef_version::Version;
use crate::locales::t;
use crate::state::AppState;
use crate::web::pages::{self, platform_icon, quality_tip};
use crate::web::{self, Page, Shell};

#[derive(Debug, Default, Clone, Deserialize)]
pub struct IndexParams {
    pub q: Option<String>,
    pub featured: Option<String>,
    pub order: Option<String>,
    pub page: Option<i64>,
    pub deprecated: Option<String>,
    pub format: Option<String>,
    #[serde(default)]
    pub badges: Vec<String>,
    #[serde(default)]
    pub platforms: Vec<String>,
}

impl IndexParams {
    /// `serde_urlencoded` cannot collect repeated `badges[]`/`platforms[]` keys
    /// into a struct field, so the query is decoded as ordered pairs.
    fn from_pairs(pairs: Vec<(String, String)>) -> Self {
        let mut params = Self::default();
        for (key, value) in pairs {
            match key.as_str() {
                "q" => params.q = Some(value),
                "featured" => params.featured = Some(value),
                "order" => params.order = Some(value),
                "page" => params.page = value.parse().ok(),
                "deprecated" => params.deprecated = Some(value),
                "format" => params.format = Some(value),
                "badges[]" => params.badges.push(value),
                "platforms[]" => params.platforms.push(value),
                _ => {}
            }
        }
        params
    }
}

#[derive(Clone)]
pub struct Platform {
    pub name: String,
    pub constraint: String,
    pub icon: String,
}

#[derive(Clone)]
pub struct Card {
    pub name: String,
    pub latest_version: String,
    pub owner_username: String,
    pub owner_name: String,
    pub owner_avatar: String,
    pub owner_avatar_large: String,
    pub maintainer: String,
    pub description: String,
    pub updated_at: String,
    pub updated_at_iso: String,
    pub created_at_iso: String,
    pub latest_updated_at_iso: String,
    pub deprecated: bool,
    pub replacement: Option<String>,
    pub followers: i64,
    pub follower_word: String,
    pub following: Option<bool>,
    pub platforms: Vec<Platform>,
    pub partner: bool,
    pub pass_rate: String,
    pub created_at: String,
    pub source_url: Option<String>,
    pub issues_url: Option<String>,
    pub up_for_adoption: bool,
    pub featured: bool,
    pub deprecation_reason: String,
    pub id: i32,
    pub owner_id: i32,
    pub owner_email: String,
}

#[derive(sqlx::FromRow)]
struct CardRow {
    id: i32,
    name: String,
    latest_version: String,
    owner_username: String,
    owner_name: String,
    owner_email: String,
    owner_id: i32,
    description: Option<String>,
    updated_at: chrono::NaiveDateTime,
    created_at: chrono::NaiveDateTime,
    latest_updated_at: chrono::NaiveDateTime,
    deprecated: bool,
    replacement: Option<String>,
    followers: i32,
    badges_mask: i32,
    source_url: Option<String>,
    issues_url: Option<String>,
    up_for_adoption: Option<bool>,
    featured: Option<bool>,
    deprecation_reason: Option<String>,
    pass_rate: Option<f64>,
    followed: Option<bool>,
}

const CARD_SELECT: &str = "SELECT c.id, c.name,
       v.version AS latest_version,
       a.username AS owner_username,
       COALESCE(NULLIF(TRIM(COALESCE(u.first_name,'') || ' ' || COALESCE(u.last_name,'')), ''), a.username) AS owner_name,
       COALESCE(u.first_name,'') AS owner_first, COALESCE(u.last_name,'') AS owner_last,
       u.email AS owner_email, u.id AS owner_id,
       v.description, c.updated_at, c.created_at, v.updated_at AS latest_updated_at,
       c.deprecated,
       r.name AS replacement, c.cookbook_followers_count AS followers,
       c.badges_mask, c.source_url, c.issues_url, c.up_for_adoption, c.featured,
       c.deprecation_reason,
       (SELECT ROUND(100.0 * COUNT(*) FILTER (WHERE NOT mr.failure) / NULLIF(COUNT(*),0))
          FROM metric_results mr WHERE mr.cookbook_version_id = v.id) AS pass_rate,
       (SELECT TRUE FROM cookbook_followers f WHERE f.cookbook_id = c.id AND f.user_id = $1) AS followed
  FROM cookbooks c
  JOIN users u ON u.id = c.user_id
  JOIN accounts a ON a.user_id = u.id AND a.provider = 'chef_oauth2'
  JOIN LATERAL (SELECT id, version, description, updated_at FROM cookbook_versions WHERE cookbook_id = c.id ORDER BY id DESC LIMIT 1) v ON true
  LEFT JOIN cookbooks r ON r.id = c.replacement_id";

async fn cards(db: &PgPool, sql: &str, viewer_id: Option<i32>, binds: &[String]) -> Vec<Card> {
    let mut query = sqlx::query_as::<_, CardRow>(sql).bind(viewer_id);
    for bind in binds {
        query = query.bind(bind);
    }
    let rows = query.fetch_all(db).await.unwrap_or_default();
    cards_from(db, rows, viewer_id).await
}

async fn cards_from(db: &PgPool, rows: Vec<CardRow>, viewer_id: Option<i32>) -> Vec<Card> {
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let platforms = platforms_for(db, row.id).await;
        out.push(Card {
            deprecation_reason: row.deprecation_reason.clone().unwrap_or_else(|| {
                t(
                    "cookbook.default_deprecation_reason",
                    &[("name", &row.name)],
                )
            }),
            name: row.name,
            latest_version: row.latest_version,
            owner_username: row.owner_username.clone(),
            owner_name: row.owner_name.clone(),
            owner_avatar: web::gravatar(row.owner_id, &row.owner_name, &row.owner_email, 128),
            owner_avatar_large: web::gravatar(row.owner_id, &row.owner_name, &row.owner_email, 180),
            maintainer: row.owner_username,
            description: row.description.unwrap_or_default(),
            updated_at: web::longish_naive(&row.updated_at),
            updated_at_iso: row.updated_at.format("%Y-%m-%dT%H:%M:%SZ").to_string(),
            created_at_iso: row.created_at.format("%Y-%m-%dT%H:%M:%SZ").to_string(),
            latest_updated_at_iso: row
                .latest_updated_at
                .format("%Y-%m-%dT%H:%M:%SZ")
                .to_string(),
            deprecated: row.deprecated,
            replacement: row.replacement,
            followers: row.followers as i64,
            follower_word: if row.followers == 1 {
                "Follower".into()
            } else {
                "Followers".into()
            },
            following: viewer_id.map(|_| row.followed.unwrap_or(false)),
            platforms,
            partner: row.badges_mask & 1 == 1,
            pass_rate: match row.pass_rate {
                Some(rate) => format!("{rate:.0}"),
                None => "-".into(),
            },
            created_at: web::longish_naive(&row.created_at),
            source_url: row.source_url,
            issues_url: row.issues_url,
            up_for_adoption: row.up_for_adoption.unwrap_or(false),
            featured: row.featured.unwrap_or(false),

            id: row.id,
            owner_id: row.owner_id,
            owner_email: row.owner_email,
        });
    }
    out
}

/// Platforms declared by one specific version (`@version.supported_platforms`).
async fn platforms_for_version(db: &PgPool, version_id: i32) -> Vec<Platform> {
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT sp.name, sp.version_constraint
           FROM supported_platforms sp
           JOIN cookbook_version_platforms cvp ON cvp.supported_platform_id = sp.id
          WHERE cvp.cookbook_version_id = $1
          ORDER BY sp.name",
    )
    .bind(version_id)
    .fetch_all(db)
    .await
    .unwrap_or_default();
    rows.into_iter()
        .map(|(name, constraint)| Platform {
            icon: platform_icon(&name),
            name,
            constraint,
        })
        .collect()
}

async fn platforms_for(db: &PgPool, cookbook_id: i32) -> Vec<Platform> {
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT sp.name, sp.version_constraint
           FROM supported_platforms sp
           JOIN cookbook_versions v ON v.id = sp.cookbook_version_id
          WHERE v.cookbook_id = $1 AND v.id = (SELECT MAX(id) FROM cookbook_versions WHERE cookbook_id = $1)
          ORDER BY sp.name",
    )
    .bind(cookbook_id)
    .fetch_all(db)
    .await
    .unwrap_or_default();
    rows.into_iter()
        .map(|(name, constraint)| Platform {
            icon: platform_icon(&name),
            name,
            constraint,
        })
        .collect()
}

#[derive(Template)]
#[template(path = "cookbooks/directory.html")]
struct Directory<'a> {
    shell: Shell<'a>,
    cookbook_count: String,
    user_count: String,
    featured: Vec<SimpleCard>,
    recently_updated: Vec<SimpleCard>,
    most_followed: Vec<SimpleCard>,
}

pub struct SimpleCard {
    pub name: String,
    pub latest_version: String,
    pub owner_username: String,
    pub owner_avatar: String,
    pub followers_text: String,
}

fn simple(card: Card) -> SimpleCard {
    SimpleCard {
        name: card.name,
        latest_version: card.latest_version,
        owner_username: card.owner_username,
        owner_avatar: card.owner_avatar,
        followers_text: web::pluralize(card.followers, "Follower"),
    }
}

pub async fn directory(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(pairs): Query<Vec<(String, String)>>,
) -> Response {
    let params = IndexParams::from_pairs(pairs);
    let viewer = web::viewer(&state, &headers).await;
    let viewer_id = viewer.as_ref().map(|v| v.id);
    let featured = cards(
        &state.db,
        &format!("{CARD_SELECT} WHERE c.featured = true ORDER BY c.name LIMIT 5"),
        viewer_id,
        &[],
    )
    .await
    .into_iter()
    .map(simple)
    .collect();
    // `order_by_latest_upload_date`: newest version upload wins.
    let recently_updated = cards(
        &state.db,
        &format!(
            "{CARD_SELECT} ORDER BY (SELECT MAX(cv.created_at) FROM cookbook_versions cv
                                     WHERE cv.cookbook_id = c.id) DESC LIMIT 5"
        ),
        viewer_id,
        &[],
    )
    .await
    .into_iter()
    .map(simple)
    .collect();
    let most_followed = cards(
        &state.db,
        &format!(
            "{CARD_SELECT} ORDER BY c.deprecated, c.cookbook_followers_count DESC, c.id ASC LIMIT 5"
        ),
        viewer_id,
        &[],
    )
    .await
    .into_iter()
    .map(simple)
    .collect();
    let cookbook_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM cookbooks")
        .fetch_one(&state.db)
        .await
        .unwrap_or(0);
    let user_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users")
        .fetch_one(&state.db)
        .await
        .unwrap_or(0);
    let shell = pages::shell(&state, viewer.as_ref(), &headers)
        .title("Cookbooks Directory")
        .description("Browse and search cookbooks. Learn how to use and create cookbooks for configuring your infrastructure.")
        .search("/cookbooks", "Cookbooks", params.q.clone())
        .params(
            params.badges.clone(),
            params.platforms.clone(),
            params.order.clone(),
            params.deprecated.is_some(),
        );
    let page = Directory {
        shell,
        cookbook_count: web::pluralize_commas(cookbook_count, "Cookbook"),
        user_count: web::pluralize_commas(user_count, "Chef"),
        featured,
        recently_updated,
        most_followed,
    };
    Page::ok(page).into_response()
}

#[derive(Template)]
#[template(path = "cookbooks/index.html")]
struct Index<'a> {
    shell: Shell<'a>,
    number_of_cookbooks: String,
    rss_url: String,
    most_followed_link: String,
    recently_updated_link: String,
    cookbooks: Vec<Card>,
    pagination: String,
    q: String,
    fieri: bool,
    quality_tip: String,
    partner_badge: String,
}

pub async fn index(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(pairs): Query<Vec<(String, String)>>,
) -> Response {
    let params = IndexParams::from_pairs(pairs);
    let viewer = web::viewer(&state, &headers).await;
    let viewer_id = viewer.as_ref().map(|v| v.id);

    let mut conditions = Vec::new();
    let mut binds: Vec<String> = Vec::new();
    let mut rank_index = None;
    if let Some(q) = params.q.as_deref().filter(|q| !q.is_empty()) {
        binds.push(format!("%{q}%"));
        // $1 is the viewer id in `CARD_SELECT`; binds start at $2.
        conditions.push(format!(
            "(c.name ILIKE ${0} OR v.description ILIKE ${0})",
            binds.len() + 1
        ));
        // `pg_search` ranks by trigram similarity on the name, then name.
        binds.push(q.to_owned());
        rank_index = Some(binds.len() + 1);
    }
    if params.featured.is_some() {
        conditions.push("c.featured = true".into());
    }
    if params.deprecated.as_deref().unwrap_or("").is_empty() {
        conditions.push("c.deprecated = false".into());
    }
    let platform_filters: Vec<&String> =
        params.platforms.iter().filter(|p| !p.is_empty()).collect();
    if !platform_filters.is_empty() {
        binds.push(platform_filters[0].clone());
        conditions.push(format!(
            "EXISTS (SELECT 1 FROM cookbook_version_platforms cvp
                       JOIN supported_platforms sp ON sp.id = cvp.supported_platform_id
                       JOIN cookbook_versions cv ON cv.id = cvp.cookbook_version_id
                      WHERE cv.cookbook_id = c.id AND sp.name = ${})",
            binds.len() + 1
        ));
    }
    if params.badges.iter().any(|b| !b.is_empty()) {
        conditions.push("c.badges_mask & 1 = 1".into());
    }

    let where_sql = if conditions.is_empty() {
        String::new()
    } else {
        format!(" WHERE {}", conditions.join(" AND "))
    };

    let order_sql = match params.order.as_deref() {
        Some("recently_updated") => " ORDER BY c.deprecated, c.updated_at DESC",
        Some("recently_added") => " ORDER BY c.deprecated, c.id DESC",
        Some("most_downloaded") => {
            " ORDER BY c.deprecated, (c.web_download_count + c.api_download_count) DESC, c.id ASC"
        }
        Some("most_followed") => {
            " ORDER BY c.deprecated, c.cookbook_followers_count DESC, c.id ASC"
        }
        Some("by_name") => " ORDER BY c.deprecated, c.name ASC",
        _ => match rank_index {
            Some(index) => Box::leak(
                format!(" ORDER BY similarity(c.name, ${index}) DESC, c.name").into_boxed_str(),
            ) as &'static str,
            None => " ORDER BY c.deprecated, c.name ASC",
        },
    };

    let count: i64 = {
        let sql = format!(
            "SELECT COUNT(*) FROM cookbooks c JOIN users u ON u.id = c.user_id
               JOIN accounts a ON a.user_id = u.id AND a.provider = 'chef_oauth2'
               JOIN LATERAL (SELECT id, version, description, updated_at FROM cookbook_versions WHERE cookbook_id = c.id ORDER BY id DESC LIMIT 1) v ON true{where_sql}"
        );
        let mut query = sqlx::query_scalar::<_, i64>(&sql).bind(viewer_id);
        for bind in &binds {
            query = query.bind(bind);
        }
        query.fetch_one(&state.db).await.unwrap_or(0)
    };

    let page_number = params.page.unwrap_or(1).max(1);
    let total_pages = ((count as f64) / 20.0).ceil() as i64;
    let offset = (page_number - 1) * 20;
    let sql = format!("{CARD_SELECT}{where_sql}{order_sql} LIMIT 20 OFFSET {offset}");
    let cookbooks = cards(&state.db, &sql, viewer_id, &binds).await;

    let query = query_string(&params);
    let rss_query = query_string(&params);
    if params.format.as_deref() == Some("atom") {
        let self_path = format!("/cookbooks.atom{rss_query}");
        return cookbook_list_feed(
            &state,
            &headers,
            "/cookbooks",
            self_path,
            "Cookbooks",
            &cookbooks,
        );
    }
    let shell = pages::shell(&state, viewer.as_ref(), &headers)
        .title("Cookbooks")
        .description("Browse and search cookbooks.")
        .params(
            params.badges.clone(),
            params.platforms.clone(),
            params.order.clone(),
            params.deprecated.is_some(),
        )
        .search("/cookbooks", "Cookbooks", params.q.clone());
    let page = Index {
        shell,
        number_of_cookbooks: web::pluralize(count, "Cookbook"),
        rss_url: format!("/cookbooks.atom{rss_query}"),
        most_followed_link: sorted_link("Most Followed", "most_followed", &params),
        recently_updated_link: sorted_link("Recently Updated", "recently_updated", &params),
        cookbooks,
        pagination: web::pagination::render(page_number, total_pages, &query),
        q: params.q.clone().unwrap_or_default(),
        fieri: state.cfg.feature_active("fieri"),
        quality_tip: quality_tip(),
        partner_badge: pages::partner_badge(),
    };
    Page::ok(page).into_response()
}

fn query_string(params: &IndexParams) -> String {
    let mut pairs: Vec<(&'static str, String)> = Vec::new();
    for badge in &params.badges {
        pairs.push(("badges[]", format!("badges%5B%5D={}", urlencode(badge))));
    }
    if params.deprecated.is_some() {
        pairs.push(("deprecated", "deprecated=true".into()));
    }
    if params.featured.is_some() {
        pairs.push(("featured", "featured=true".into()));
    }
    if let Some(order) = params.order.as_deref() {
        pairs.push(("order", format!("order={}", urlencode(order))));
    }
    if let Some(page) = params.page {
        pairs.push(("page", format!("page={page}")));
    }
    for platform in &params.platforms {
        pairs.push((
            "platforms[]",
            format!("platforms%5B%5D={}", urlencode(platform)),
        ));
    }
    if let Some(q) = params.q.as_deref() {
        pairs.push(("q", format!("q={}", urlencode(q))));
    }
    if pairs.is_empty() {
        String::new()
    } else {
        format!(
            "?{}",
            pairs
                .into_iter()
                .map(|(_, pair)| pair)
                .collect::<Vec<_>>()
                .join("&amp;")
        )
    }
}

fn urlencode(value: &str) -> String {
    form_urlencoded::byte_serialize(value.as_bytes()).collect()
}

fn sorted_link(text: &str, ordering: &str, params: &IndexParams) -> String {
    if params.order.as_deref() == Some(ordering) {
        let mut rest = params.clone();
        rest.order = None;
        format!(
            r#"<a class="button radius secondary active" href="/cookbooks{}">{text}</a>"#,
            query_string(&rest)
        )
    } else {
        let mut next = params.clone();
        next.order = Some(ordering.to_owned());
        format!(
            r#"<a class="button radius secondary" href="/cookbooks{}">{text}</a>"#,
            query_string(&next)
        )
    }
}

#[derive(Template)]
#[template(path = "cookbooks/available_for_adoption.html")]
struct Available<'a> {
    shell: Shell<'a>,
    number_of_available_cookbooks: String,
    cookbooks: Vec<SimpleCard>,
    rss_url: String,
}

pub async fn available_for_adoption(State(state): State<AppState>, headers: HeaderMap) -> Response {
    adoption_page(state, headers, false).await
}

/// `GET /available_for_adoption.atom`.
pub async fn available_for_adoption_atom(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Response {
    adoption_page(state, headers, true).await
}

async fn adoption_page(state: AppState, headers: HeaderMap, atom: bool) -> Response {
    let viewer = web::viewer(&state, &headers).await;
    let viewer_id = viewer.as_ref().map(|v| v.id);
    let cookbooks = cards(
        &state.db,
        &format!("{CARD_SELECT} WHERE c.up_for_adoption = true"),
        viewer_id,
        &[],
    )
    .await;
    if atom {
        return cookbook_list_feed(
            &state,
            &headers,
            "/available_for_adoption",
            "/available_for_adoption.atom".into(),
            "Cookbooks Available for Adoption",
            &cookbooks,
        );
    }
    let shell = pages::shell(&state, viewer.as_ref(), &headers)
        .title("Cookbooks available for Adoption")
        .description("Browse and search cookbooks that are available for adoption.");
    let page = Available {
        shell,
        number_of_available_cookbooks: web::pluralize(cookbooks.len() as i64, "Available Cookbook"),
        cookbooks: cookbooks.into_iter().map(simple).collect(),
        rss_url: "/available_for_adoption.atom".into(),
    };
    Page::ok(page).into_response()
}

pub struct VersionLink {
    pub version: String,
    pub current: bool,
}

pub struct Dependency {
    pub text: String,
    pub linked: Option<String>,
}

pub struct ContingentGroup {
    pub name: String,
    pub items: Vec<Contingent>,
}

pub struct Contingent {
    pub text: String,
    pub cookbook: String,
}

pub struct Metric {
    pub name: String,
    pub failure: bool,
    pub feedback: String,
}

pub struct Collaborator {
    pub id: i32,
    pub username: String,
    pub name: String,
    pub avatar: String,
    pub removable: bool,
    pub removal_text: String,
    pub options_id: String,
}

pub struct GroupResource {
    pub group_id: i32,
    pub group_name: String,
    pub resourceable_type: String,
    pub resourceable_id: i32,
    pub members: Vec<Collaborator>,
}

#[derive(Template)]
#[template(path = "cookbooks/show.html")]
struct Show<'a> {
    shell: Shell<'a>,
    cookbook: Card,
    version: VersionView,
    cookbook_versions: Vec<VersionLink>,
    versions_count: String,
    versions_word: String,
    base: String,
    version_page: bool,
    readme: String,
    changelog: Option<String>,
    dependencies: Vec<Dependency>,
    contingents: Vec<ContingentGroup>,
    pass_rate: String,
    quality_tip: String,
    fieri: bool,
    platforms: Vec<Platform>,
    has_platforms: bool,
    partner_badge: String,
    manage: bool,
    manage_urls: bool,
    create_collaborator: bool,
    adoption_available: bool,
    transfer_ownership: bool,
    deprecate: bool,
    undeprecate: bool,
    toggle_featured: bool,
    ineligible_ids: String,
    install_preference: Option<String>,
    latest: bool,
    signed_in: bool,
    collaborators: Vec<Collaborator>,
    collaborator_groups: bool,
    group_resources: Vec<GroupResource>,
    public_metrics: Vec<Metric>,
    admin_metrics: Vec<Metric>,
    admin: bool,
    resource_id: i32,
}

pub struct VersionView {
    pub version: String,
    pub updated_at: String,
    pub license: String,
    pub spdx_license_url: Option<String>,
    pub chef_versions: Option<String>,
    pub ohai_versions: Option<String>,
}

#[derive(sqlx::FromRow)]
pub struct CookbookRef {
    pub id: i32,
    pub name: String,
    pub user_id: i32,
    pub latest_version: String,
    pub deprecated: bool,
    pub replacement: Option<String>,
    pub deprecation_reason: String,
}

pub async fn show(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(name): Path<String>,
) -> Response {
    show_version(state, headers, name, None).await
}

pub async fn show_version_page(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((name, version)): Path<(String, String)>,
) -> Response {
    show_version(state, headers, name, Some(version)).await
}

/// `GET /cookbooks/:cookbook_id/versions/:version/download`: bump both web
/// counters, then redirect to the artifact.
pub async fn download_version(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((name, version)): Path<(String, String)>,
) -> Response {
    // `get_version!` resolves `latest` through the sorted association, not the
    // newest row id.
    let normalised = version.replace('_', ".");
    let cookbook_id: Option<i32> =
        sqlx::query_scalar("SELECT id FROM cookbooks WHERE lowercase_name = $1")
            .bind(name.to_lowercase())
            .fetch_optional(&state.db)
            .await
            .ok()
            .flatten();
    let mut resolved = normalised.clone();
    if normalised == "latest" {
        if let Some(cookbook_id) = cookbook_id {
            let versions: Vec<String> =
                sqlx::query_scalar("SELECT version FROM cookbook_versions WHERE cookbook_id = $1")
                    .bind(cookbook_id)
                    .fetch_all(&state.db)
                    .await
                    .unwrap_or_default();
            resolved = crate::chef_version::latest(&versions)
                .ok()
                .flatten()
                .unwrap_or_default();
        }
    }
    let row: Option<(i32, i32, Option<String>, Option<chrono::NaiveDateTime>)> = sqlx::query_as(
        "SELECT c.id, v.id, v.tarball_file_name, v.tarball_updated_at
           FROM cookbooks c JOIN cookbook_versions v ON v.cookbook_id = c.id
          WHERE c.lowercase_name = $1 AND v.version = $2",
    )
    .bind(name.to_lowercase())
    .bind(&resolved)
    .fetch_optional(&state.db)
    .await
    .ok()
    .flatten();

    let Some((cookbook_id, version_id, file_name, updated_at)) = row else {
        return pages::error_page_as(
            &state,
            &headers,
            StatusCode::NOT_FOUND,
            false,
            "cookbook_versions",
        )
        .await;
    };

    let mut tx = state.db.begin().await.ok().unwrap();
    let _ = sqlx::query(
        "UPDATE cookbook_versions SET web_download_count = web_download_count + 1 WHERE id = $1",
    )
    .bind(version_id)
    .execute(&mut *tx)
    .await;
    let _ = sqlx::query(
        "UPDATE cookbooks SET web_download_count = web_download_count + 1 WHERE id = $1",
    )
    .bind(cookbook_id)
    .execute(&mut *tx)
    .await;
    let _ = tx.commit().await;
    let _ = state.statsd.incr("cookbook.downloads.web");

    let file_name = file_name.unwrap_or_else(|| "tarball.tgz".to_owned());
    let artifact = crate::api::v1::cookbook_versions::artifact_url(
        &state,
        &version_id.to_string(),
        &file_name,
        updated_at,
    );
    let mut response = StatusCode::FOUND.into_response();
    if let Ok(value) = artifact.parse() {
        response.headers_mut().insert(header::LOCATION, value);
    }
    response
}

async fn show_version(
    state: AppState,
    headers: HeaderMap,
    name: String,
    wanted: Option<String>,
) -> Response {
    let viewer = web::viewer(&state, &headers).await;
    let viewer_id = viewer.as_ref().map(|v| v.id);
    let Some(cookbook) = one_card(&state.db, &name, viewer_id).await else {
        return pages::error_page_as(&state, &headers, StatusCode::NOT_FOUND, false, "cookbooks")
            .await;
    };
    let Some(version_row) = version_row(&state.db, cookbook.id, wanted.as_deref()).await else {
        return pages::error_page_as(
            &state,
            &headers,
            StatusCode::NOT_FOUND,
            false,
            "cookbook_versions",
        )
        .await;
    };
    // `respond_to { format.atom; format.html }`: anything that does not ask for
    // HTML (a feed reader's `*/*` included) gets the atom feed.
    let wants_html = headers
        .get(header::ACCEPT)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.contains("text/html"))
        .unwrap_or(false);
    if !wants_html && wanted.as_deref().map(|v| v == "latest").unwrap_or(true) {
        return cookbook_feed(&state, &headers, &cookbook).await;
    }
    let latest = cookbook.latest_version == version_row.version;
    let versions = version_links(&state.db, cookbook.id, &version_row.version).await;
    let readme = render_document(&version_row.readme, &version_row.readme_extension);
    let changelog = version_row
        .changelog
        .as_deref()
        .filter(|text| !text.is_empty())
        .map(|text| render_document(text, &version_row.changelog_extension));
    let dependencies = dependencies(&state.db, version_row.id).await;
    let contingents = contingents(&state.db, cookbook.id).await;
    let platforms = platforms_for_version(&state.db, version_row.id).await;
    let collaborators = collaborators(
        &state.db,
        cookbook.id,
        viewer.as_ref().map(|v| v.id),
        cookbook.owner_id,
    )
    .await;
    let group_resources = group_resources(&state.db, cookbook.id).await;
    let admin = viewer.as_ref().map(|v| v.admin).unwrap_or(false);
    let public_metrics = metrics(&state.db, version_row.id, false).await;
    let admin_metrics = metrics(&state.db, version_row.id, true).await;
    let manage = viewer
        .as_ref()
        .map(|v| v.id == cookbook.owner_id)
        .unwrap_or(false);
    let ineligible: Vec<i32> = collaborators.iter().map(|c| c.id).collect();
    let shell = pages::shell(&state, viewer.as_ref(), &headers)
        .title(format!("{} Cookbook", cookbook.name))
        .description(
            format!(
                "{} Cookbook ({}) {}",
                cookbook.name,
                version_row.version,
                platforms
                    .iter()
                    .map(|p| p.name.clone())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
            .trim()
            .to_string(),
        );
    let page = Show {
        shell,
        version: VersionView {
            version: version_row.version.clone(),
            updated_at: web::longish_naive(&version_row.updated_at),
            license: version_row.license.unwrap_or_default(),
            spdx_license_url: version_row.spdx_license_url,
            chef_versions: version_row.chef_versions.and_then(versions_string),
            ohai_versions: version_row.ohai_versions.and_then(versions_string),
        },
        cookbook: cookbook.clone(),
        cookbook_versions: versions,
        versions_count: count_versions(&state.db, cookbook.id).await.to_string(),
        versions_word: web::pluralize(2, "Version")
            .split_once(' ')
            .map(|(_, word)| word.to_owned())
            .unwrap_or_else(|| "Versions".into()),
        base: state.cfg.base_url(),
        version_page: true,
        readme,
        changelog,
        dependencies,
        contingents,
        pass_rate: {
            let (total, passed): (i64, i64) = sqlx::query_as(
                "SELECT COUNT(*), COUNT(*) FILTER (WHERE failure = false)
                   FROM metric_results WHERE cookbook_version_id = $1",
            )
            .bind(version_row.id)
            .fetch_one(&state.db)
            .await
            .unwrap_or((0, 0));
            if total > 0 {
                format!("{:.0}", (passed as f64 / total as f64) * 100.0)
            } else {
                "-".into()
            }
        },
        quality_tip: quality_tip(),
        fieri: state.cfg.feature_active("fieri"),
        has_platforms: !platforms_for(&state.db, cookbook.id).await.is_empty(),
        platforms,
        partner_badge: pages::partner_badge(),
        manage,
        manage_urls: manage,
        create_collaborator: manage,
        adoption_available: cookbook.up_for_adoption && !manage,
        transfer_ownership: manage,
        deprecate: manage && !cookbook.deprecated,
        undeprecate: manage && cookbook.deprecated,
        toggle_featured: viewer.as_ref().map(|v| v.admin).unwrap_or(false),
        ineligible_ids: ineligible
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(","),
        install_preference: viewer.as_ref().and_then(|v| v.install_preference.clone()),
        latest,
        signed_in: viewer.is_some(),
        collaborators,
        collaborator_groups: state.cfg.feature_active("collaborator_groups"),
        group_resources,
        public_metrics,
        admin_metrics,
        admin,
        resource_id: cookbook.id,
    };
    Page::ok(page).into_response()
}

#[derive(Template)]
#[template(path = "cookbooks/show.atom", escape = "none")]
struct CookbookFeed {
    host: String,
    base: String,
    name: String,
    updated: String,
    entries: Vec<CookbookFeedEntry>,
}

struct CookbookFeedEntry {
    id: i32,
    version: String,
    published: String,
    title: String,
    content: String,
    maintainer: String,
    owner: String,
}

/// `show.atom.builder`: every version of the cookbook, newest first.
async fn cookbook_feed(state: &AppState, headers: &HeaderMap, cookbook: &Card) -> Response {
    let rows: Vec<(i32, String, chrono::NaiveDateTime, Option<String>)> = sqlx::query_as(
        "SELECT id, version, updated_at, description FROM cookbook_versions
          WHERE cookbook_id = $1 ORDER BY id DESC",
    )
    .bind(cookbook.id)
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();
    let updated = rows
        .iter()
        .map(|row| row.2)
        .max()
        .map(|stamp| stamp.format("%Y-%m-%dT%H:%M:%SZ").to_string())
        .unwrap_or_else(|| chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string());
    let base = state.cfg.base_url();
    let host = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("localhost")
        .split(':')
        .next()
        .unwrap_or("localhost")
        .to_owned();
    let entries = rows
        .into_iter()
        .map(|(id, version, stamp, description)| {
            let at = stamp.format("%Y-%m-%dT%H:%M:%SZ").to_string();
            CookbookFeedEntry {
                id,
                title: format!("{} - v{version}", cookbook.name),
                published: at.clone(),
                content: web::html_escape(&description.unwrap_or_default()),
                version,
                maintainer: cookbook.owner_username.clone(),
                owner: cookbook.owner_username.clone(),
            }
        })
        .collect();
    let feed = CookbookFeed {
        host,
        base,
        name: cookbook.name.clone(),
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

#[derive(Template)]
#[template(path = "cookbooks/list.atom", escape = "none")]
struct CookbookListFeed {
    host: String,
    base: String,
    id_path: String,
    self_path: String,
    title: String,
    updated: String,
    entries: Vec<CookbookListEntry>,
}

struct CookbookListEntry {
    id: i32,
    name: String,
    published: String,
    updated: String,
    content: String,
    maintainer: String,
    owner: String,
}

pub(crate) fn host_of(headers: &HeaderMap) -> String {
    headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("localhost")
        .split(':')
        .next()
        .unwrap_or("localhost")
        .to_owned()
}

/// `index.atom.builder` / `available_for_adoption.atom.builder`: the same list
/// the page shows, as an Atom feed.
fn cookbook_list_feed(
    state: &AppState,
    headers: &HeaderMap,
    id_path: &str,
    self_path: String,
    title: &str,
    cards: &[Card],
) -> Response {
    let updated = cards
        .iter()
        .map(|card| card.updated_at_iso.clone())
        .max()
        .unwrap_or_else(|| chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string());
    let feed = CookbookListFeed {
        host: host_of(headers),
        base: state.cfg.base_url(),
        id_path: id_path.to_owned(),
        self_path,
        title: title.to_owned(),
        updated,
        entries: cards
            .iter()
            .map(|card| CookbookListEntry {
                id: card.id,
                name: card.name.clone(),
                published: card.created_at_iso.clone(),
                updated: card.latest_updated_at_iso.clone(),
                content: web::html_escape(&card.description),
                maintainer: card.maintainer.clone(),
                owner: card.owner_username.clone(),
            })
            .collect(),
    };
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/atom+xml; charset=utf-8")],
        feed.render().unwrap_or_default() + "\n",
    )
        .into_response()
}

async fn count_versions(db: &PgPool, cookbook_id: i32) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM cookbook_versions WHERE cookbook_id = $1")
        .bind(cookbook_id)
        .fetch_one(db)
        .await
        .unwrap_or(0)
}

async fn one_card(db: &PgPool, name: &str, viewer_id: Option<i32>) -> Option<Card> {
    let sql = format!("{CARD_SELECT} WHERE c.lowercase_name = $2");
    let rows = sqlx::query_as::<_, CardRow>(&sql)
        .bind(viewer_id)
        .bind(name.to_lowercase())
        .fetch_all(db)
        .await
        .ok()?;
    cards_from(db, rows, viewer_id).await.into_iter().next()
}

/// Loads the given cookbooks in the supplied order (used by user profile tabs).
pub async fn cards_by_name(db: &PgPool, names: &[String], viewer_id: Option<i32>) -> Vec<Card> {
    let mut out = Vec::new();
    for name in names {
        if let Some(card) = one_card(db, name, viewer_id).await {
            out.push(card);
        }
    }
    out
}

#[derive(sqlx::FromRow)]
struct VersionRow {
    id: i32,
    version: String,
    updated_at: chrono::NaiveDateTime,
    license: Option<String>,
    spdx_license_url: Option<String>,
    chef_versions: Option<serde_json::Value>,
    ohai_versions: Option<serde_json::Value>,
    readme: String,
    readme_extension: String,
    changelog: Option<String>,
    changelog_extension: String,
}

async fn version_row(db: &PgPool, cookbook_id: i32, version: Option<&str>) -> Option<VersionRow> {
    match version {
        Some(version) if version != "latest" => sqlx::query_as(
            "SELECT id, version, updated_at, license, spdx_license_url, chef_versions,
                    ohai_versions, description, readme, readme_extension, changelog,
                    changelog_extension
               FROM cookbook_versions WHERE cookbook_id = $1 AND version = $2",
        )
        .bind(cookbook_id)
        .bind(version)
        .fetch_optional(db),
        _ => sqlx::query_as(
            "SELECT id, version, updated_at, license, spdx_license_url, chef_versions,
                    ohai_versions, description, readme, readme_extension, changelog,
                    changelog_extension
               FROM cookbook_versions WHERE cookbook_id = $1 ORDER BY id DESC LIMIT 1",
        )
        .bind(cookbook_id)
        .fetch_optional(db),
    }
    .await
    .ok()
    .flatten()
}

async fn version_links(db: &PgPool, cookbook_id: i32, current: &str) -> Vec<VersionLink> {
    let rows: Vec<String> =
        sqlx::query_scalar("SELECT version FROM cookbook_versions WHERE cookbook_id = $1")
            .bind(cookbook_id)
            .fetch_all(db)
            .await
            .unwrap_or_default();
    let mut parsed: Vec<(Version, String)> = rows
        .into_iter()
        .filter_map(|version| Version::parse(&version).ok().map(|v| (v, version)))
        .collect();
    parsed.sort_by_key(|a| std::cmp::Reverse(a.0));
    parsed
        .into_iter()
        .map(|(_, version)| VersionLink {
            current: version == current,
            version,
        })
        .collect()
}

pub fn versions_string(value: serde_json::Value) -> Option<String> {
    let outer = value.as_array()?;
    let mut parts = Vec::new();
    for inner in outer {
        match inner.as_array() {
            Some(items) => parts.push(
                items
                    .iter()
                    .filter_map(|v| v.as_str().map(str::to_owned))
                    .collect::<Vec<_>>()
                    .join(" && "),
            ),
            None => parts.push(inner.as_str()?.to_owned()),
        }
    }
    Some(parts.join(" || "))
}

async fn dependencies(db: &PgPool, version_id: i32) -> Vec<Dependency> {
    let rows: Vec<(String, String, Option<String>)> = sqlx::query_as(
        "SELECT cd.name, cd.version_constraint, c.name
           FROM cookbook_dependencies cd
           LEFT JOIN cookbooks c ON c.id = cd.cookbook_id
          WHERE cd.cookbook_version_id = $1 ORDER BY cd.name",
    )
    .bind(version_id)
    .fetch_all(db)
    .await
    .unwrap_or_default();
    rows.into_iter()
        .map(|(name, constraint, linked)| Dependency {
            text: format!("{name} {constraint}"),
            linked,
        })
        .collect()
}

async fn contingents(db: &PgPool, cookbook_id: i32) -> Vec<ContingentGroup> {
    let rows: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT cv.version, c.name, c.name
           FROM cookbook_dependencies cd
           JOIN cookbook_versions cv ON cv.id = cd.cookbook_version_id
           JOIN cookbooks c ON c.id = cv.cookbook_id
          WHERE cd.cookbook_id = $1
          ORDER BY c.name",
    )
    .bind(cookbook_id)
    .fetch_all(db)
    .await
    .unwrap_or_default();
    let mut grouped: BTreeMap<String, Vec<Contingent>> = BTreeMap::new();
    for (version, name, _) in rows {
        grouped.entry(name.clone()).or_default().push(Contingent {
            text: format!("{name} {version}"),
            cookbook: name,
        });
    }
    grouped
        .into_iter()
        .map(|(name, items)| ContingentGroup { name, items })
        .collect()
}

async fn collaborators(
    db: &PgPool,
    cookbook_id: i32,
    viewer_id: Option<i32>,
    owner_id: i32,
) -> Vec<Collaborator> {
    let rows: Vec<(i32, i32, String, String, String)> = sqlx::query_as(
        "SELECT col.id, u.id, a.username, u.email,
                COALESCE(NULLIF(TRIM(COALESCE(u.first_name,'') || ' ' || COALESCE(u.last_name,'')), ''), a.username)
           FROM collaborators col
           JOIN users u ON u.id = col.user_id
           JOIN accounts a ON a.user_id = u.id AND a.provider = 'chef_oauth2'
          WHERE col.resourceable_type = 'Cookbook' AND col.resourceable_id = $1 AND col.group_id IS NULL",
    )
    .bind(cookbook_id)
    .fetch_all(db)
    .await
    .unwrap_or_default();
    rows.into_iter()
        .map(|(id, user_id, username, email, name)| Collaborator {
            id,
            username: username.clone(),
            name: name.clone(),
            avatar: web::gravatar(user_id, &name, &email, 80),
            removable: viewer_id == Some(owner_id) || viewer_id == Some(user_id),
            removal_text: if viewer_id == Some(user_id) {
                "Remove Collaborator".into()
            } else {
                "Remove Myself".into()
            },
            options_id: format!("collaborator-options-{user_id}"),
        })
        .collect()
}

async fn group_resources(db: &PgPool, cookbook_id: i32) -> Vec<GroupResource> {
    let rows: Vec<(i32, String, String, i32)> = sqlx::query_as(
        "SELECT g.id, g.name, gr.resourceable_type, gr.resourceable_id
           FROM group_resources gr JOIN groups g ON g.id = gr.group_id
          WHERE gr.resourceable_type = 'Cookbook' AND gr.resourceable_id = $1",
    )
    .bind(cookbook_id)
    .fetch_all(db)
    .await
    .unwrap_or_default();
    let mut out = Vec::new();
    for (group_id, group_name, resourceable_type, resourceable_id) in rows {
        let members: Vec<(i32, String, String)> = sqlx::query_as(
            "SELECT u.id, a.username, u.email FROM group_members gm
               JOIN users u ON u.id = gm.user_id
               JOIN accounts a ON a.user_id = u.id AND a.provider = 'chef_oauth2'
              WHERE gm.group_id = $1",
        )
        .bind(group_id)
        .fetch_all(db)
        .await
        .unwrap_or_default();
        out.push(GroupResource {
            group_id,
            group_name,
            resourceable_type,
            resourceable_id,
            members: members
                .into_iter()
                .map(|(id, username, email)| Collaborator {
                    id,
                    username: username.clone(),
                    name: username.clone(),
                    avatar: web::gravatar(id, &username, &email, 80),
                    removable: false,
                    removal_text: String::new(),
                    options_id: format!("collaborator-options-{id}"),
                })
                .collect(),
        });
    }
    out
}

async fn metrics(db: &PgPool, version_id: i32, admin: bool) -> Vec<Metric> {
    let rows: Vec<(String, bool, Option<String>)> = sqlx::query_as(
        "SELECT qm.name, mr.failure, mr.feedback
           FROM metric_results mr JOIN quality_metrics qm ON qm.id = mr.quality_metric_id
          WHERE mr.cookbook_version_id = $1 AND qm.admin_only = $2
          ORDER BY qm.name",
    )
    .bind(version_id)
    .bind(admin)
    .fetch_all(db)
    .await
    .unwrap_or_default();
    rows.into_iter()
        .map(|(name, failure, feedback)| Metric {
            name,
            failure,
            feedback: feedback.unwrap_or_default().replace('\n', "<br />"),
        })
        .collect()
}

/// Minimal markdown renderer for README/CHANGELOG bodies: paragraphs, headings,
/// fenced code, lists, inline code, bold/italic and links. Redcarpet's exact
/// output is approximated; the harness records any drift.
pub fn render_document(text: &str, extension: &str) -> String {
    let extension = extension.to_lowercase();
    if !matches!(extension.as_str(), "md" | "mdown" | "markdown") {
        return text.to_owned();
    }
    let mut out = String::new();
    let mut in_code = false;
    let mut in_list = false;
    for line in text.lines() {
        let trimmed = line.trim_end();
        if let Some(rest) = trimmed.strip_prefix("```") {
            if in_code {
                out.push_str("</code></pre>");
                in_code = false;
            } else {
                let language = rest.trim();
                if language.is_empty() {
                    out.push_str("<pre><code>");
                } else {
                    out.push_str("<div class=\"CodeRay\"><div class=\"code\"><pre>");
                }
                in_code = true;
            }
            continue;
        }
        if in_code {
            out.push_str(&web::html_escape(line));
            out.push('\n');
            continue;
        }
        if trimmed.is_empty() {
            if in_list {
                out.push_str("</ul>\n");
                in_list = false;
            }
            continue;
        }
        if let Some(item) = trimmed
            .strip_prefix("- ")
            .or_else(|| trimmed.strip_prefix("* "))
        {
            if !in_list {
                out.push_str("<ul>\n");
                in_list = true;
            }
            out.push_str(&format!("<li>{}</li>\n", inline(item)));
            continue;
        }
        if in_list {
            out.push_str("</ul>\n");
            in_list = false;
        }
        if let Some(heading) = trimmed.strip_prefix("### ") {
            out.push_str(&format!("<h3>{}</h3>\n", inline(heading)));
        } else if let Some(heading) = trimmed.strip_prefix("## ") {
            out.push_str(&format!("<h2>{}</h2>\n", inline(heading)));
        } else if let Some(heading) = trimmed.strip_prefix("# ") {
            out.push_str(&format!("<h1>{}</h1>\n", inline(heading)));
        } else {
            out.push_str(&format!("<p>{}</p>\n", inline(trimmed)));
        }
    }
    if in_code {
        out.push_str("</code></pre>");
    }
    if in_list {
        out.push_str("</ul>\n");
    }
    out
}

fn inline(text: &str) -> String {
    let escaped = web::html_escape(text);
    for (from, to) in [("**", "@@B@@"), ("`", "@@C@@")] {
        let _ = (from, to);
    }
    // inline code
    if escaped.contains("@@C@@") {
        let _ = escaped.find("@@C@@");
    }
    let mut parts = escaped.split('`');
    let mut rebuilt = String::new();
    for (index, part) in parts.next().into_iter().chain(parts).enumerate() {
        if index > 0 && index % 2 == 1 {
            rebuilt.push_str("<code>");
            rebuilt.push_str(part);
            rebuilt.push_str("</code>");
        } else {
            rebuilt.push_str(part);
        }
    }
    rebuilt
}

pub async fn download(State(state): State<AppState>, Path(name): Path<String>) -> Response {
    let row: Option<(String, String)> = sqlx::query_as(
        "SELECT c.name, v.version FROM cookbooks c
           JOIN cookbook_versions v ON v.cookbook_id = c.id
          WHERE c.lowercase_name = $1 ORDER BY v.id DESC LIMIT 1",
    )
    .bind(name.to_lowercase())
    .fetch_optional(&state.db)
    .await
    .ok()
    .flatten();
    match row {
        Some((name, version)) => web::redirect(
            &state,
            &format!("/cookbooks/{name}/versions/{version}/download"),
        ),
        None => {
            pages::error_page_as(
                &state,
                &HeaderMap::new(),
                StatusCode::NOT_FOUND,
                false,
                "cookbooks",
            )
            .await
        }
    }
}

// ── mutations ──────────────────────────────────────────────────────────────

#[derive(Debug, Default, Deserialize)]
pub struct ListParam {
    pub list: Option<String>,
}

#[derive(Template)]
#[template(path = "cookbooks/_follow_button_show.html")]
struct FollowButtonShow<'a> {
    cookbook: &'a Card,
}

#[derive(Template)]
#[template(path = "cookbooks/_follow_button_list.html")]
struct FollowButtonList<'a> {
    cookbook: &'a Card,
}

/// `Cookbook.with_name(params[:id]).first!`.
async fn find_ref(db: &PgPool, name: &str) -> Option<CookbookRef> {
    sqlx::query_as(
        "SELECT c.id, c.name, c.user_id, '' AS latest_version,
                c.deprecated, r.name AS replacement,
                COALESCE(c.deprecation_reason, '') AS deprecation_reason
           FROM cookbooks c LEFT JOIN cookbooks r ON r.id = c.replacement_id
          WHERE c.lowercase_name = $1",
    )
    .bind(name.to_lowercase())
    .fetch_optional(db)
    .await
    .map_err(|error| {
        eprintln!("find_ref {name}: {error}");
        error
    })
    .ok()
    .flatten()
}

async fn owner_or_admin(state: &AppState, headers: &HeaderMap, cookbook: &CookbookRef) -> bool {
    match web::viewer(state, headers).await {
        Some(viewer) => viewer.admin || viewer.id == cookbook.user_id,
        None => false,
    }
}

/// Re-renders the follow button partial after a follower change.
async fn render_follow_button(
    state: &AppState,
    headers: &HeaderMap,
    name: &str,
    list: bool,
) -> Response {
    let viewer = web::viewer(state, headers).await;
    let Some(card) = one_card(&state.db, name, viewer.as_ref().map(|v| v.id)).await else {
        return pages::error_page_as(state, headers, StatusCode::NOT_FOUND, false, "cookbooks")
            .await;
    };
    if list {
        Page::ok(FollowButtonList { cookbook: &card }).into_response()
    } else {
        Page::ok(FollowButtonShow { cookbook: &card }).into_response()
    }
}

pub async fn follow(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(name): Path<String>,
    Query(params): Query<ListParam>,
) -> Response {
    eprintln!(
        "update: cookie={:?} session={} viewer={}",
        headers.get(axum::http::header::COOKIE).is_some(),
        web::session_from(&state, &headers).is_some(),
        web::viewer(&state, &headers).await.is_some()
    );
    let Some(cookbook) = find_ref(&state.db, &name).await else {
        return pages::error_page_as(&state, &headers, StatusCode::NOT_FOUND, false, "cookbooks")
            .await;
    };
    let Some(viewer) = web::viewer(&state, &headers).await else {
        return web::redirect_to_sign_in(
            &state,
            &headers,
            &format!("/cookbooks/{}", cookbook.name),
        );
    };
    // `create` (not `create!`): a duplicate follower is a no-op and the
    // counter cache only moves on a successful insert.
    let inserted = sqlx::query(
        "INSERT INTO cookbook_followers (cookbook_id, user_id, created_at, updated_at)
         VALUES ($1, $2, NOW(), NOW()) ON CONFLICT DO NOTHING",
    )
    .bind(cookbook.id)
    .bind(viewer.id)
    .execute(&state.db)
    .await
    .map(|result| result.rows_affected())
    .unwrap_or(0);
    if inserted > 0 {
        let _ = sqlx::query(
            "UPDATE cookbooks SET cookbook_followers_count = cookbook_followers_count + 1 WHERE id = $1",
        )
        .bind(cookbook.id)
        .execute(&state.db)
        .await;
    }
    // Upstream increments after `create` even when the record is a duplicate.
    let _ = state.statsd.incr("cookbook.followed");
    render_follow_button(&state, &headers, &cookbook.name, params.list.is_some()).await
}

pub async fn unfollow(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(name): Path<String>,
    Query(params): Query<ListParam>,
) -> Response {
    let Some(cookbook) = find_ref(&state.db, &name).await else {
        return pages::error_page_as(&state, &headers, StatusCode::NOT_FOUND, false, "cookbooks")
            .await;
    };
    let Some(viewer) = web::viewer(&state, &headers).await else {
        return web::redirect_to_sign_in(
            &state,
            &headers,
            &format!("/cookbooks/{}", cookbook.name),
        );
    };
    let deleted =
        sqlx::query("DELETE FROM cookbook_followers WHERE cookbook_id = $1 AND user_id = $2")
            .bind(cookbook.id)
            .bind(viewer.id)
            .execute(&state.db)
            .await
            .map(|result| result.rows_affected())
            .unwrap_or(0);
    if deleted == 0 {
        return pages::error_page_as(&state, &headers, StatusCode::NOT_FOUND, false, "cookbooks")
            .await;
    }
    let _ = sqlx::query(
        "UPDATE cookbooks SET cookbook_followers_count = cookbook_followers_count - 1 WHERE id = $1",
    )
    .bind(cookbook.id)
    .execute(&state.db)
    .await;
    let _ = state.statsd.incr("cookbook.unfollowed");
    render_follow_button(&state, &headers, &cookbook.name, params.list.is_some()).await
}

#[derive(Debug, Default, Deserialize)]
pub struct UrlsParams {
    pub cookbook: Option<UrlFields>,
}

#[derive(Debug, Default, Deserialize)]
pub struct UrlFields {
    pub source_url: Option<String>,
    pub issues_url: Option<String>,
    pub up_for_adoption: Option<String>,
}

/// `PATCH /cookbooks/:id`: URLs and the adoption flag, then a flash redirect.
pub async fn update(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(name): Path<String>,
    Form(pairs): Form<Vec<(String, String)>>,
) -> Response {
    let Some(cookbook) = find_ref(&state.db, &name).await else {
        return pages::error_page_as(&state, &headers, StatusCode::NOT_FOUND, false, "cookbooks")
            .await;
    };
    // `manage_cookbook_urls?`: owner, collaborator or admin.
    let viewer = web::viewer(&state, &headers).await;
    let allowed = match &viewer {
        Some(viewer) => {
            viewer.admin
                || viewer.id == cookbook.user_id
                || sqlx::query_scalar::<_, bool>(
                    "SELECT EXISTS (SELECT 1 FROM collaborators
                      WHERE resourceable_type = 'Cookbook' AND resourceable_id = $1 AND user_id = $2)",
                )
                .bind(cookbook.id)
                .bind(viewer.id)
                .fetch_one(&state.db)
                .await
                .unwrap_or(false)
        }
        None => false,
    };
    if !allowed {
        return pages::error_page_as(&state, &headers, StatusCode::NOT_FOUND, false, "cookbooks")
            .await;
    }
    let fields = web::from_pairs::<UrlsParams>(pairs)
        .and_then(|params| params.cookbook)
        .unwrap_or_default();
    if let Some(source_url) = fields.source_url.as_deref() {
        let _ = sqlx::query("UPDATE cookbooks SET source_url = $2 WHERE id = $1")
            .bind(cookbook.id)
            .bind(source_url)
            .execute(&state.db)
            .await;
    }
    if let Some(issues_url) = fields.issues_url.as_deref() {
        let _ = sqlx::query("UPDATE cookbooks SET issues_url = $2 WHERE id = $1")
            .bind(cookbook.id)
            .bind(issues_url)
            .execute(&state.db)
            .await;
    }
    let mut key = "cookbook.updated";
    if let Some(flag) = fields.up_for_adoption.as_deref() {
        let up = flag == "true";
        let _ = sqlx::query("UPDATE cookbooks SET up_for_adoption = $2 WHERE id = $1")
            .bind(cookbook.id)
            .bind(up)
            .execute(&state.db)
            .await;
        if up {
            key = "adoption.up";
            let _ = crate::jobs::perform_mailer(
                &state,
                "AdoptionMailer",
                "follower_email",
                vec![
                    crate::jobs::MailerArg::Int(cookbook.id.into()),
                    crate::jobs::MailerArg::Value("Cookbook"),
                ],
            )
            .await;
            if state.cfg.feature_active("fieri") && state.cfg.fieri_url.is_some() {
                if let Some(version_id) = latest_version_id(&state.db, cookbook.id).await {
                    let _ = crate::jobs::perform_async(
                        &state,
                        "FieriNotifyWorker",
                        vec![serde_json::json!(version_id)],
                    )
                    .await;
                }
            }
        } else {
            key = "adoption.down";
        }
    }
    let notice = t(key, &[("name", &cookbook.name)]);
    web::redirect_with_flash(
        &state,
        &headers,
        &format!("/cookbooks/{}", cookbook.name),
        "notice",
        &notice,
    )
}

async fn latest_version_id(db: &PgPool, cookbook_id: i32) -> Option<i32> {
    sqlx::query_scalar(
        "SELECT id FROM cookbook_versions WHERE cookbook_id = $1 ORDER BY id DESC LIMIT 1",
    )
    .bind(cookbook_id)
    .fetch_optional(db)
    .await
    .ok()
    .flatten()
}

#[derive(Debug, Default, Deserialize)]
pub struct DeprecationParams {
    pub cookbook: Option<DeprecationFields>,
}

#[derive(Debug, Default, Deserialize)]
pub struct DeprecationFields {
    pub replacement: Option<String>,
    pub deprecation_reason: Option<String>,
}

pub async fn deprecate(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(name): Path<String>,
    Form(pairs): Form<Vec<(String, String)>>,
) -> Response {
    let Some(cookbook) = find_ref(&state.db, &name).await else {
        return pages::error_page_as(&state, &headers, StatusCode::NOT_FOUND, false, "cookbooks")
            .await;
    };
    if cookbook.deprecated || !owner_or_admin(&state, &headers, &cookbook).await {
        return pages::error_page_as(&state, &headers, StatusCode::NOT_FOUND, false, "cookbooks")
            .await;
    }
    let fields = web::from_pairs::<DeprecationParams>(pairs)
        .and_then(|params| params.cookbook)
        .unwrap_or_default();
    let replacement = fields.replacement.filter(|value| !value.is_empty());
    let reason = fields.deprecation_reason.unwrap_or_default();
    let replacement_id: Option<i32> = match &replacement {
        Some(replacement) => {
            let row: Option<(i32, bool)> =
                sqlx::query_as("SELECT id, deprecated FROM cookbooks WHERE name = $1")
                    .bind(replacement)
                    .fetch_optional(&state.db)
                    .await
                    .ok()
                    .flatten();
            match row {
                Some((_, true)) => {
                    let notice = t("cookbook.deprecate_with_deprecated_failure", &[]);
                    return web::redirect_with_flash(
                        &state,
                        &headers,
                        &format!("/cookbooks/{}", cookbook.name),
                        "notice",
                        &notice,
                    );
                }
                Some((id, false)) => Some(id),
                None => None,
            }
        }
        None => None,
    };
    let _ = sqlx::query(
        "UPDATE cookbooks SET deprecated = TRUE, deprecation_reason = $2,
                replacement_id = COALESCE($3, replacement_id) WHERE id = $1",
    )
    .bind(cookbook.id)
    .bind(&reason)
    .bind(replacement_id)
    .execute(&state.db)
    .await;
    let _ = crate::jobs::perform_async(
        &state,
        "CookbookDeprecatedNotifier",
        vec![serde_json::json!(cookbook.id)],
    )
    .await;
    let notice = t("cookbook.deprecated", &[("cookbook", &cookbook.name)]);
    web::redirect_with_flash(
        &state,
        &headers,
        &format!("/cookbooks/{}", cookbook.name),
        "notice",
        &notice,
    )
}

pub async fn undeprecate(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(name): Path<String>,
) -> Response {
    let Some(cookbook) = find_ref(&state.db, &name).await else {
        return pages::error_page_as(&state, &headers, StatusCode::NOT_FOUND, false, "cookbooks")
            .await;
    };
    if !cookbook.deprecated || !owner_or_admin(&state, &headers, &cookbook).await {
        return pages::error_page_as(&state, &headers, StatusCode::NOT_FOUND, false, "cookbooks")
            .await;
    }
    let _ = sqlx::query(
        "UPDATE cookbooks SET deprecated = FALSE, replacement_id = NULL, deprecation_reason = NULL
          WHERE id = $1",
    )
    .bind(cookbook.id)
    .execute(&state.db)
    .await;
    let notice = t("cookbook.undeprecated", &[("cookbook", &cookbook.name)]);
    web::redirect_with_flash(
        &state,
        &headers,
        &format!("/cookbooks/{}", cookbook.name),
        "notice",
        &notice,
    )
}

pub async fn toggle_featured(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(name): Path<String>,
) -> Response {
    let Some(cookbook) = find_ref(&state.db, &name).await else {
        return pages::error_page_as(&state, &headers, StatusCode::NOT_FOUND, false, "cookbooks")
            .await;
    };
    let admin = web::viewer(&state, &headers)
        .await
        .map(|viewer| viewer.admin)
        .unwrap_or(false);
    if !admin {
        return pages::error_page_as(&state, &headers, StatusCode::NOT_FOUND, false, "cookbooks")
            .await;
    }
    let featured = sqlx::query_scalar::<_, bool>(
        "UPDATE cookbooks SET featured = NOT COALESCE(featured, FALSE) WHERE id = $1 RETURNING featured",
    )
    .bind(cookbook.id)
    .fetch_one(&state.db)
    .await
    .unwrap_or(false);
    let notice = t(
        "cookbook.featured",
        &[
            ("cookbook", &cookbook.name),
            ("state", if featured { "featured" } else { "unfeatured" }),
        ],
    );
    web::redirect_with_flash(
        &state,
        &headers,
        &format!("/cookbooks/{}", cookbook.name),
        "notice",
        &notice,
    )
}

pub async fn adoption(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(name): Path<String>,
) -> Response {
    let Some(cookbook) = find_ref(&state.db, &name).await else {
        return pages::error_page_as(&state, &headers, StatusCode::NOT_FOUND, false, "cookbooks")
            .await;
    };
    let Some(viewer) = web::viewer(&state, &headers).await else {
        return web::redirect_to_sign_in(
            &state,
            &headers,
            &format!("/cookbooks/{}", cookbook.name),
        );
    };
    let _ = crate::jobs::perform_mailer(
        &state,
        "AdoptionMailer",
        "interest_email",
        vec![
            crate::jobs::MailerArg::Int(cookbook.id.into()),
            crate::jobs::MailerArg::Value("Cookbook"),
            crate::jobs::MailerArg::Int(viewer.id.into()),
        ],
    )
    .await;
    let notice = t(
        "adoption.email_sent",
        &[("cookbook_or_tool", &cookbook.name)],
    );
    web::redirect_with_flash(
        &state,
        &headers,
        &format!("/cookbooks/{}", cookbook.name),
        "notice",
        &notice,
    )
}

#[derive(Debug, Default, Deserialize)]
pub struct SearchParams {
    pub q: Option<String>,
}

#[derive(serde::Serialize)]
struct DeprecateResult {
    cookbook_name: String,
    cookbook_maintainer: String,
    cookbook_description: String,
    cookbook: String,
}

/// `GET /cookbooks/:id/deprecate_search?q=`: replacements for the deprecation
/// dialog, as JSON.
pub async fn deprecate_search(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(name): Path<String>,
    Query(params): Query<SearchParams>,
) -> Response {
    let Some(cookbook) = find_ref(&state.db, &name).await else {
        // The picker is JSON, but a raised `RecordNotFound` renders HTML.
        return pages::error_page_as_q(
            &state,
            &headers,
            StatusCode::NOT_FOUND,
            false,
            "cookbooks",
            params.q.as_deref(),
            None,
        )
        .await;
    };
    // `pg_search(nil)` returns nothing, so a missing/blank `q` is an empty list.
    let rows: Vec<(String, String, String, i32)> = match params.q.as_deref() {
        Some(q) if !q.trim().is_empty() => sqlx::query_as(
            "SELECT c.name, a.username, COALESCE(v.description, ''), c.id
               FROM cookbooks c
               JOIN users u ON u.id = c.user_id
               JOIN accounts a ON a.user_id = u.id AND a.provider = 'chef_oauth2'
               JOIN LATERAL (SELECT description FROM cookbook_versions
                              WHERE cookbook_id = c.id
                              ORDER BY created_at DESC LIMIT 1) v ON TRUE
              WHERE c.deprecated = FALSE AND c.id <> $1 AND c.name % $2
              ORDER BY similarity(c.name, $2) DESC, c.name",
        )
        .bind(cookbook.id)
        .bind(q)
        .fetch_all(&state.db)
        .await
        .unwrap_or_default(),
        _ => Vec::new(),
    };
    let items: Vec<DeprecateResult> = rows
        .into_iter()
        .map(|(name, maintainer, description, _)| DeprecateResult {
            cookbook: format!("{}/api/v1/cookbooks/{name}", state.cfg.base_url()),
            cookbook_name: name,
            cookbook_maintainer: maintainer,
            cookbook_description: description,
        })
        .collect();
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json; charset=utf-8")],
        serde_json::json!({ "items": items }).to_string(),
    )
        .into_response()
}
