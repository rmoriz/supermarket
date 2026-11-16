//! `ToolsController` read paths: the tool index/search, tool page and the
//! new/edit forms.

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
use crate::web::pages;
use crate::web::{self, Page, Shell};

pub const TYPES: &[&str] = &[
    "knife_plugin",
    "ohai_plugin",
    "chef_infra_tool",
    "chef_infra_handler",
    "kitchen_driver",
    "powershell_module",
    "dsc_resource",
    "compliance_profile",
];

/// `pretty_type`: titleize, with DSC resources spelled out.
pub fn pretty_type(kind: &str) -> String {
    if kind == "dsc_resource" {
        return "DSC Resource".into();
    }
    kind.split('_')
        .map(|part| {
            let mut chars = part.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().chain(chars).collect::<String>(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

pub struct ToolType {
    pub value: String,
    pub label: String,
}

pub fn types() -> Vec<ToolType> {
    TYPES
        .iter()
        .map(|kind| ToolType {
            value: (*kind).to_owned(),
            label: pretty_type(kind),
        })
        .collect()
}

pub struct Tool {
    pub id: i32,
    pub name: String,
    pub slug: String,
    pub kind: String,
    pub type_label: String,
    pub description: String,
    pub description_raw: String,
    pub instructions: String,
    pub instructions_raw: String,
    pub source_url: Option<String>,
    pub owner_username: String,
    pub owner_name: String,
    pub owner_avatar: String,
    pub owner_avatar_large: String,
    pub maintainer: String,
    pub has_body: bool,
    pub editable: bool,
    pub deletable: bool,
    pub adoptable: bool,
    pub confirm_deletion: String,
    pub updated_at: String,
    pub created_at: String,
    pub up_for_adoption: bool,
    pub name_value: Option<String>,
    pub slug_value: Option<String>,
}

#[derive(sqlx::FromRow)]
struct ToolRow {
    id: i32,
    name: String,
    slug: String,
    kind: String,
    description: Option<String>,
    instructions: Option<String>,
    source_url: Option<String>,
    owner_username: String,
    owner_name: String,
    owner_email: String,
    owner_id: i32,
    updated_at: NaiveDateTime,
    created_at: NaiveDateTime,
    up_for_adoption: Option<bool>,
}

const TOOL_SELECT: &str = "SELECT t.id, t.name, t.slug, t.type AS kind,
       t.description, t.instructions, t.source_url,
       a.username AS owner_username,
       COALESCE(NULLIF(TRIM(COALESCE(u.first_name,'') || ' ' || COALESCE(u.last_name,'')), ''), a.username) AS owner_name,
       u.email AS owner_email, u.id AS owner_id,
       t.updated_at, t.created_at, t.up_for_adoption
  FROM tools t
  JOIN users u ON u.id = t.user_id
  JOIN accounts a ON a.user_id = u.id AND a.provider = 'chef_oauth2'";

fn build(row: ToolRow, viewer: Option<&web::Viewer>) -> Tool {
    let owner = viewer.map(|v| v.id == row.owner_id).unwrap_or(false);
    let admin = viewer.map(|v| v.admin).unwrap_or(false);
    let has_body = row.description.is_some() || row.instructions.is_some();
    let owner_username = row.owner_username.clone();
    Tool {
        name: row.name.clone(),
        confirm_deletion: t("tool.confirm_deletion", &[("name", &row.name)]),
        slug: row.slug.clone(),
        kind: row.kind.clone(),
        type_label: pretty_type(&row.kind),
        description: simple_format(
            row.description.as_deref().unwrap_or(""),
            Some("description"),
        ),
        description_raw: row.description.unwrap_or_default(),
        instructions: crate::web::markdown::render(row.instructions.as_deref().unwrap_or("")),
        instructions_raw: row.instructions.unwrap_or_default(),
        source_url: row.source_url.filter(|v| !v.is_empty()),
        owner_username: row.owner_username,
        owner_avatar: web::gravatar(row.owner_id, &row.owner_name, &row.owner_email, 128),
        owner_avatar_large: web::gravatar(row.owner_id, &row.owner_name, &row.owner_email, 180),
        maintainer: owner_username,
        owner_name: row.owner_name,
        has_body,
        editable: owner || admin,
        deletable: owner || admin,
        adoptable: owner || admin,
        updated_at: web::longish_naive(&row.updated_at),
        created_at: web::longish_naive(&row.created_at),
        up_for_adoption: row.up_for_adoption.unwrap_or(false),
        name_value: Some(row.name.clone()),
        slug_value: Some(row.slug.clone()),
        id: row.id,
    }
}

/// Rails' `simple_format`: paragraphs split on blank lines, newlines to `<br />`.
fn simple_format(text: &str, itemprop: Option<&str>) -> String {
    if text.is_empty() {
        return String::new();
    }
    let attr = itemprop
        .map(|name| format!(" itemprop=\"{name}\""))
        .unwrap_or_default();
    text.split("\n\n")
        .map(|paragraph| {
            format!(
                "<p{attr}>{}</p>",
                web::html_escape(paragraph.trim_end()).replace('\n', "<br />\n")
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

pub async fn tools_of(db: &PgPool, user_id: i32, viewer_id: Option<i32>, offset: i64) -> Vec<Tool> {
    let rows = sqlx::query_as::<_, ToolRow>(&format!(
        "{TOOL_SELECT} WHERE t.user_id = $2 ORDER BY t.name LIMIT 20 OFFSET $3"
    ))
    .bind(viewer_id)
    .bind(user_id)
    .bind(offset)
    .fetch_all(db)
    .await
    .unwrap_or_default();
    let viewer = match viewer_id {
        Some(id) => web::load_viewer(db, id).await,
        None => None,
    };
    rows.into_iter()
        .map(|row| build(row, viewer.as_ref()))
        .collect()
}

/// The dashboard's five most recent tools.
pub async fn simple_tools(db: &PgPool, user_id: i32) -> Vec<Tool> {
    let rows = sqlx::query_as::<_, ToolRow>(&format!(
        "{TOOL_SELECT} WHERE t.user_id = $1 ORDER BY t.name LIMIT 5"
    ))
    .bind(user_id)
    .fetch_all(db)
    .await
    .unwrap_or_default();
    rows.into_iter().map(|row| build(row, None)).collect()
}

#[derive(Debug, Default, Clone, Deserialize)]
pub struct IndexParams {
    pub q: Option<String>,
    pub order: Option<String>,
    pub r#type: Option<String>,
    pub page: Option<i64>,
}

#[derive(Template)]
#[template(path = "tools/index.html")]
struct Index<'a> {
    shell: Shell<'a>,
    tools: Vec<Tool>,
    pagination: String,
    rss_url: String,
    type_label: String,
    type_links: Vec<(String, String)>,
}

pub async fn index(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(params): Query<IndexParams>,
) -> Response {
    let viewer = web::viewer(&state, &headers).await;
    let viewer_id = viewer.as_ref().map(|v| v.id);
    let mut conditions: Vec<String> = Vec::new();
    let mut binds: Vec<String> = Vec::new();
    if let Some(q) = params.q.as_deref().filter(|q| !q.is_empty()) {
        binds.push(format!("%{q}%"));
        // $1 is the viewer id in `TOOL_SELECT`; binds start at $2.
        conditions.push(format!(
            "(t.name ILIKE ${0} OR t.description ILIKE ${0})",
            binds.len() + 1
        ));
    }
    if let Some(kind) = params.r#type.as_deref().filter(|k| TYPES.contains(k)) {
        binds.push(kind.to_owned());
        conditions.push(format!("t.type = ${}", binds.len() + 1));
    }
    let where_sql = if conditions.is_empty() {
        String::new()
    } else {
        format!(" WHERE {}", conditions.join(" AND "))
    };
    let order_sql = if params.order.as_deref() == Some("created_at") {
        " ORDER BY t.created_at"
    } else {
        " ORDER BY t.name"
    };
    let page_number = params.page.unwrap_or(1).max(1);
    let offset = (page_number - 1) * 20;

    let count: i64 = {
        let sql = format!("SELECT COUNT(*) FROM tools t{where_sql}");
        let mut query = sqlx::query_scalar::<_, i64>(&sql).bind(viewer_id);
        for bind in &binds {
            query = query.bind(bind);
        }
        query.fetch_one(&state.db).await.unwrap_or(0)
    };
    let sql = format!("{TOOL_SELECT}{where_sql}{order_sql} LIMIT 20 OFFSET {offset}");
    let mut query = sqlx::query_as::<_, ToolRow>(&sql).bind(viewer_id);
    for bind in &binds {
        query = query.bind(bind);
    }
    let rows = query.fetch_all(&state.db).await.unwrap_or_default();
    let tools: Vec<Tool> = rows
        .into_iter()
        .map(|row| build(row, viewer.as_ref()))
        .collect();

    let mut rss = params.clone();
    rss.order = Some("created_at".into());
    // `tools_path(format: :atom, ...)` → `.atom` in the path, no `format` param.
    let query = index_query(&rss, None);
    let type_label = params
        .r#type
        .as_deref()
        .filter(|kind| TYPES.contains(kind))
        .map(pretty_type)
        .unwrap_or_else(|| "All".into());
    let shell = pages::shell(&state, viewer.as_ref(), &headers)
        .title("Tools & Plugins")
        .description("Browse and search Chef tools and plugins.")
        .search("/tools", "Tools", params.q.clone())
        .params(Vec::new(), Vec::new(), params.order.clone(), false);
    let page = Index {
        shell,
        tools,
        pagination: web::pagination::render_with(
            page_number,
            ((count as f64) / 20.0).ceil() as i64,
            |page| format!("/tools{}", index_query(&params, Some(page))),
        ),
        rss_url: format!("/tools.atom{query}"),
        type_label: type_label.clone(),
        type_links: types()
            .iter()
            .map(|kind| {
                let mut next = params.clone();
                next.r#type = Some(kind.value.clone());
                (
                    kind.label.clone(),
                    format!("/tools{}", index_query(&next, None)),
                )
            })
            .collect(),
    };
    Page::ok(page).into_response()
}

/// Rails' `Hash#to_query` sorts keys, so `order` precedes `q` and `type`.
/// `page` is kept last, matching the pagination helper.
fn index_query(params: &IndexParams, page: Option<i64>) -> String {
    let mut pairs: Vec<(String, String)> = Vec::new();
    if let Some(order) = params.order.as_deref() {
        pairs.push(("order".into(), urlencode(order)));
    }
    if let Some(page) = page {
        pairs.push(("page".into(), page.to_string()));
    }
    if let Some(q) = params.q.as_deref().filter(|q| !q.is_empty()) {
        pairs.push(("q".into(), urlencode(q)));
    }
    if let Some(kind) = params.r#type.as_deref().filter(|k| TYPES.contains(k)) {
        pairs.push(("type".into(), urlencode(kind)));
    }
    // `Hash#to_query` order: order, page, q, type — page is sorted too.
    pairs.sort_by(|a, b| a.0.cmp(&b.0));
    if pairs.is_empty() {
        String::new()
    } else {
        let joined: Vec<String> = pairs.iter().map(|(k, v)| format!("{k}={v}")).collect();
        format!("?{}", joined.join("&amp;"))
    }
}

fn urlencode(value: &str) -> String {
    form_urlencoded::byte_serialize(value.as_bytes()).collect()
}

/// Rack's path-segment escaping: everything but the unreserved set is escaped.
fn encode_segment(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

pub struct ToolCollaborator {
    id: i32,
    user_id: i32,
    username: String,
    name: String,
    avatar: String,
    removal_text: &'static str,
    destroy: bool,
}

#[derive(Template)]
#[template(path = "tools/show.html")]
struct Show<'a> {
    shell: Shell<'a>,
    tool: Tool,
    other_tools: Vec<Tool>,
    collaborators: Vec<ToolCollaborator>,
    can_add_collaborator: bool,
    ineligible_url: String,
}

pub async fn show(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
) -> Response {
    let viewer = web::viewer(&state, &headers).await;
    let row = sqlx::query_as::<_, ToolRow>(&format!("{TOOL_SELECT} WHERE t.slug = $2"))
        .bind(viewer.as_ref().map(|v| v.id))
        .bind(&slug)
        .fetch_optional(&state.db)
        .await
        .ok()
        .flatten();
    let Some(row) = row else {
        // `assign_tool` raises before `override_search` runs, so the layout
        // still points at the cookbook search.
        return pages::error_page_as_q(
            &state,
            &headers,
            StatusCode::NOT_FOUND,
            false,
            "tools",
            None,
            Some(("/cookbooks", "Cookbooks")),
        )
        .await;
    };
    let owner_id = row.owner_id;
    let tool = build(row, viewer.as_ref());
    let others = sqlx::query_as::<_, ToolRow>(&format!(
        "{TOOL_SELECT} WHERE t.user_id = (SELECT user_id FROM tools WHERE slug = $1) AND t.slug <> $1 ORDER BY t.name"
    ))
    .bind(&slug)
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();
    let other_tools: Vec<Tool> = others
        .into_iter()
        .map(|row| build(row, viewer.as_ref()))
        .collect();
    let collaborators: Vec<ToolCollaborator> = sqlx::query_as::<
        _,
        (i32, i32, String, String, String, String),
    >(
        "SELECT c.id, u.id, a.username,
                COALESCE(NULLIF(TRIM(COALESCE(u.first_name,'') || ' ' || COALESCE(u.last_name,'')), ''), a.username),
                u.email, a.username
           FROM collaborators c
           JOIN users u ON u.id = c.user_id
           JOIN accounts a ON a.user_id = u.id AND a.provider = 'chef_oauth2'
          WHERE c.resourceable_type = 'Tool' AND c.resourceable_id = $1 AND c.group_id IS NULL
          ORDER BY c.id",
    )
    .bind(tool.id)
    .fetch_all(&state.db)
    .await
    .unwrap_or_default()
    .into_iter()
    .map(|(id, user_id, username, name, email, _)| {
        let is_owner = viewer.as_ref().map(|v| v.id == owner_id).unwrap_or(false);
        let admin = viewer.as_ref().map(|v| v.admin).unwrap_or(false);
        let is_self = viewer.as_ref().map(|v| v.id == user_id).unwrap_or(false);
        ToolCollaborator {
            id,
            user_id,
            removal_text: if is_owner {
                "Remove Collaborator"
            } else {
                "Remove Myself"
            },
            avatar: web::gravatar(user_id, &name, &email, 80),
            name,
            username,
            destroy: is_owner || admin || is_self,
        }
    })
    .collect();
    let mut ineligible: Vec<i32> = collaborators.iter().map(|c| c.user_id).collect();
    ineligible.push(owner_id);
    let ineligible_url = format!(
        "/collaborators?{}",
        ineligible
            .iter()
            .map(|id| format!("ineligible_user_ids%5B%5D={id}"))
            .collect::<Vec<_>>()
            .join("&amp;")
    );
    // `policy(tool).create_collaborator?` is owner-or-admin? No: owner only.
    let can_add_collaborator = viewer.as_ref().map(|v| v.id == owner_id).unwrap_or(false);
    let shell = pages::shell(&state, viewer.as_ref(), &headers)
        .title(tool.name.clone())
        .description(tool.name.clone())
        .search("/tools", "Tools", None);
    let page = Show {
        shell,
        tool,
        other_tools,
        collaborators,
        can_add_collaborator,
        ineligible_url,
    };
    Page::ok(page).into_response()
}

#[derive(Template)]
#[template(path = "tools/new.html")]
struct New<'a> {
    shell: Shell<'a>,
    tool: Tool,
    err_name: bool,
    err_slug: bool,
    err_type: bool,
    err_source_url: bool,
    types: Vec<ToolType>,
    errors: Vec<String>,
    form_action: String,
    editing: bool,
    user: crate::web::users::UserView,
    title: String,
    active: &'static str,
    own_profile: bool,
    show_email: bool,
    make_admin: bool,
    revoke_admin: bool,
    profile_controller: bool,
    chef_icon: String,
    tools_enabled: bool,
    collaborator_groups: bool,
}

pub async fn new(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let viewer = web::viewer(&state, &headers).await;
    let Some(viewer) = viewer else {
        // `before_action :store_location!` runs first, then `authenticate_user!`.
        return web::redirect_to_sign_in(&state, &headers, "/tools/new");
    };
    let user = crate::web::users::find_user_view(&state.db, &viewer.username).await;
    let Some(user) = user else {
        return pages::error_page_as(&state, &headers, StatusCode::NOT_FOUND, false, "tools").await;
    };
    let tool = Tool {
        id: 0,
        name: String::new(),
        slug: String::new(),
        kind: String::new(),
        type_label: pretty_type("knife_plugin"),
        description: String::new(),
        description_raw: String::new(),
        instructions: String::new(),
        instructions_raw: String::new(),
        source_url: None,
        owner_username: viewer.username.clone(),
        owner_name: viewer.name.clone(),
        owner_avatar: String::new(),
        owner_avatar_large: String::new(),
        maintainer: viewer.username.clone(),
        has_body: false,
        editable: true,
        deletable: false,
        adoptable: false,
        confirm_deletion: String::new(),
        updated_at: String::new(),
        created_at: String::new(),
        up_for_adoption: false,
        name_value: None,
        slug_value: None,
    };
    let title = format!("{} Tools", user.possessive);
    let shell = pages::shell(&state, Some(&viewer), &headers)
        .title("Add new Tool")
        .search("/tools", "Tools", None);
    let page = New {
        shell,
        chef_icon: pages::chef_icon(),
        err_name: false,
        err_slug: false,
        err_type: false,
        err_source_url: false,
        tool,
        types: types(),
        errors: Vec::new(),
        form_action: "/tools".into(),
        editing: false,
        user,
        title,
        active: "add",
        own_profile: true,
        show_email: true,
        make_admin: false,
        revoke_admin: false,
        profile_controller: false,
        tools_enabled: true,
        collaborator_groups: state.cfg.feature_active("collaborator_groups"),
    };
    Page::ok(page).into_response()
}

#[derive(Template)]
#[template(path = "tools/edit.html")]
struct Edit<'a> {
    shell: Shell<'a>,
    view_action: String,
    tool: Tool,
    err_name: bool,
    err_slug: bool,
    err_type: bool,
    err_source_url: bool,
    types: Vec<ToolType>,
    errors: Vec<String>,
    form_action: String,
    editing: bool,
    user: crate::web::users::UserView,
    title: String,
    active: &'static str,
    own_profile: bool,
    show_email: bool,
    make_admin: bool,
    revoke_admin: bool,
    profile_controller: bool,
    chef_icon: String,
    tools_enabled: bool,
    collaborator_groups: bool,
}

pub async fn edit(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
) -> Response {
    let viewer = web::viewer(&state, &headers).await;
    let Some(viewer) = viewer else {
        return pages::error_page_as(&state, &headers, StatusCode::UNAUTHORIZED, false, "tools")
            .await;
    };
    let row = sqlx::query_as::<_, ToolRow>(&format!("{TOOL_SELECT} WHERE t.slug = $2"))
        .bind(Some(viewer.id))
        .bind(&slug)
        .fetch_optional(&state.db)
        .await
        .ok()
        .flatten();
    let Some(row) = row else {
        return pages::error_page_as(&state, &headers, StatusCode::NOT_FOUND, false, "tools").await;
    };
    let tool = build(row, Some(&viewer));
    if !tool.editable {
        return pages::error_page_as(&state, &headers, StatusCode::NOT_FOUND, false, "tools").await;
    }
    let Some(user) = crate::web::users::find_user_view(&state.db, &viewer.username).await else {
        return pages::error_page_as(&state, &headers, StatusCode::NOT_FOUND, false, "tools").await;
    };
    let title = format!("{} Tools", user.possessive);
    let name = tool.name.clone();
    let shell = pages::shell(&state, Some(&viewer), &headers)
        .title(format!("Edit {name}"))
        .search("/tools", "Tools", None);
    let page = Edit {
        shell,
        chef_icon: pages::chef_icon(),
        err_name: false,
        err_slug: false,
        err_type: false,
        err_source_url: false,
        view_action: format!("/tools/{}", encode_segment(&slug)),
        form_action: format!("/tools/{}", encode_segment(&slug)),
        editing: true,
        tool,
        types: types(),
        errors: Vec::new(),
        user,
        title,
        active: "maintains",
        own_profile: true,
        show_email: true,
        make_admin: false,
        revoke_admin: false,
        profile_controller: false,
        tools_enabled: true,
        collaborator_groups: state.cfg.feature_active("collaborator_groups"),
    };
    Page::ok(page).into_response()
}

#[derive(Debug, Default, Deserialize)]
pub struct ToolFields {
    pub name: Option<String>,
    pub slug: Option<String>,
    pub r#type: Option<String>,
    pub description: Option<String>,
    pub source_url: Option<String>,
    pub instructions: Option<String>,
    pub up_for_adoption: Option<String>,
}

fn tool_fields(pairs: &[(String, String)]) -> Option<ToolFields> {
    if !pairs.iter().any(|(key, _)| key.starts_with("tool[")) {
        return None;
    }
    let mut value = serde_json::Map::new();
    for (key, item) in pairs {
        if let Some(field) = key.strip_prefix("tool[") {
            if let Some(field) = field.strip_suffix(']') {
                value.insert(field.to_owned(), serde_json::Value::String(item.clone()));
            }
        }
    }
    serde_json::from_value(serde_json::Value::Object(value)).ok()
}

/// `name.parameterize`, the fallback `ensure_slug` uses.
fn parameterize(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut last_dash = false;
    for ch in value.chars() {
        let lower = ch.to_ascii_lowercase();
        if lower.is_ascii_alphanumeric() || lower == '_' || lower == '-' {
            out.push(lower);
            last_dash = lower == '-';
        } else if !last_dash {
            out.push('-');
            last_dash = true;
        }
    }
    out.trim_matches('-').to_owned()
}

/// `validate_url`'s accepted shapes: an http(s) scheme and no whitespace.
fn valid_url(value: &str) -> bool {
    if value.trim() != value || value.chars().any(char::is_whitespace) {
        return false;
    }
    value.starts_with("http://") || value.starts_with("https://")
}

/// The model validations in declaration order, callbacks first.
async fn validate_tool(
    db: &PgPool,
    fields: &ToolFields,
    exclude_id: Option<i32>,
) -> (String, String, Vec<String>) {
    let name = fields.name.clone().unwrap_or_default().trim().to_owned();
    let mut slug = fields.slug.clone().unwrap_or_default().to_owned();
    if slug.trim().is_empty() {
        slug = parameterize(&name);
    }
    let slug = slug.to_lowercase();
    let lowercase_name = name.to_lowercase();
    let label = fields.r#type.clone().unwrap_or_default();

    let mut errors = Vec::new();
    if name.is_empty() {
        errors.push("Name can't be blank".to_owned());
    } else if sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (SELECT 1 FROM tools WHERE lower(lowercase_name) = $1 AND id <> $2)",
    )
    .bind(&lowercase_name)
    .bind(exclude_id.unwrap_or(0))
    .fetch_one(db)
    .await
    .unwrap_or(false)
    {
        errors.push("Name has already been taken".to_owned());
    }
    if lowercase_name.is_empty() {
        errors.push("Lowercase name can't be blank".to_owned());
    } else if sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (SELECT 1 FROM tools WHERE lowercase_name = $1 AND id <> $2)",
    )
    .bind(&lowercase_name)
    .bind(exclude_id.unwrap_or(0))
    .fetch_one(db)
    .await
    .unwrap_or(false)
    {
        errors.push("Lowercase name has already been taken".to_owned());
    }
    if !TYPES.contains(&label.as_str()) {
        errors.push("Type is not included in the list".to_owned());
    }
    if slug.is_empty() {
        errors.push("Slug can't be blank".to_owned());
    } else if sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (SELECT 1 FROM tools WHERE lower(slug) = $1 AND id <> $2)",
    )
    .bind(&slug)
    .bind(exclude_id.unwrap_or(0))
    .fetch_one(db)
    .await
    .unwrap_or(false)
    {
        errors.push("Slug has already been taken".to_owned());
    }
    if slug.is_empty()
        || !slug
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        errors.push("Slug is invalid".to_owned());
    }
    if let Some(source_url) = fields.source_url.as_deref() {
        if !source_url.is_empty() && !valid_url(source_url) {
            errors.push("Source url is not a valid URL".to_owned());
        }
    }

    (name, slug, errors)
}

fn form_tool(fields: &ToolFields, name: &str, slug: &str, owner: &web::Viewer) -> Tool {
    let kind = fields.r#type.clone().unwrap_or_default();
    Tool {
        id: 0,
        name: name.to_owned(),
        slug: slug.to_owned(),
        type_label: pretty_type(&kind),
        kind,
        description: simple_format(
            fields.description.as_deref().unwrap_or(""),
            Some("description"),
        ),
        description_raw: fields.description.clone().unwrap_or_default(),
        instructions: web::markdown::render(fields.instructions.as_deref().unwrap_or("")),
        instructions_raw: fields.instructions.clone().unwrap_or_default(),
        source_url: fields.source_url.clone().filter(|value| !value.is_empty()),
        owner_username: owner.username.clone(),
        owner_name: owner.name.clone(),
        owner_avatar: String::new(),
        owner_avatar_large: String::new(),
        maintainer: owner.username.clone(),
        has_body: fields.description.is_some() || fields.instructions.is_some(),
        editable: true,
        deletable: false,
        adoptable: false,
        confirm_deletion: String::new(),
        updated_at: String::new(),
        created_at: String::new(),
        up_for_adoption: fields.up_for_adoption.as_deref() == Some("true"),
        name_value: fields.name.clone(),
        slug_value: fields.slug.clone(),
    }
}

#[derive(Template)]
#[template(path = "tools/edit.html")]
struct EditForm<'a> {
    shell: Shell<'a>,
    view_action: String,
    tool: Tool,
    err_name: bool,
    err_slug: bool,
    err_type: bool,
    err_source_url: bool,
    types: Vec<ToolType>,
    errors: Vec<String>,
    form_action: String,
    editing: bool,
    user: crate::web::users::UserView,
    title: String,
    active: &'static str,
    own_profile: bool,
    show_email: bool,
    make_admin: bool,
    revoke_admin: bool,
    profile_controller: bool,
    chef_icon: String,
    tools_enabled: bool,
    collaborator_groups: bool,
}

/// The new/edit page with validation errors, rendered after a failed save.
async fn render_form(
    state: &AppState,
    headers: &HeaderMap,
    viewer: &web::Viewer,
    tool: Tool,
    errors: Vec<String>,
    editing: bool,
) -> Response {
    let Some(user) = crate::web::users::find_user_view(&state.db, &viewer.username).await else {
        return pages::error_page_as(state, headers, StatusCode::NOT_FOUND, false, "tools").await;
    };
    let title = format!("{} Tools", user.possessive);
    let name = tool.name.clone();
    let shell = pages::shell(state, Some(viewer), headers)
        .title(if editing {
            format!("Edit {name}")
        } else {
            "Add new Tool".to_owned()
        })
        .search("/tools", "Tools", None);
    let form_action = if editing {
        format!("/tools/{}", encode_segment(&tool.slug))
    } else {
        "/tools".to_owned()
    };
    let active = if editing { "maintains" } else { "add" };
    let err_name = errors.iter().any(|error| error.starts_with("Name "));
    let err_slug = errors.iter().any(|error| error.starts_with("Slug "));
    let err_type = errors.iter().any(|error| error.starts_with("Type "));
    let err_source_url = errors.iter().any(|error| error.starts_with("Source url "));
    if editing {
        let page = EditForm {
            shell,
            chef_icon: pages::chef_icon(),
            err_name,
            err_slug,
            err_type,
            err_source_url,
            view_action: form_action.clone(),
            tool,
            types: types(),
            errors,
            form_action,
            editing,
            user,
            title,
            active,
            own_profile: true,
            show_email: true,
            make_admin: false,
            revoke_admin: false,
            profile_controller: false,
            tools_enabled: true,
            collaborator_groups: state.cfg.feature_active("collaborator_groups"),
        };
        (StatusCode::OK, Page::ok(page).into_response()).into_response()
    } else {
        let page = New {
            shell,
            chef_icon: pages::chef_icon(),
            err_name,
            err_slug,
            err_type,
            err_source_url,
            tool,
            types: types(),
            errors,
            form_action,
            editing,
            user,
            title,
            active,
            own_profile: true,
            show_email: true,
            make_admin: false,
            revoke_admin: false,
            profile_controller: false,
            tools_enabled: true,
            collaborator_groups: state.cfg.feature_active("collaborator_groups"),
        };
        (StatusCode::OK, Page::ok(page).into_response()).into_response()
    }
}

/// `POST /tools`.
pub async fn create(
    State(state): State<AppState>,
    headers: HeaderMap,
    axum::extract::Form(pairs): axum::extract::Form<Vec<(String, String)>>,
) -> Response {
    let Some(viewer) = web::viewer(&state, &headers).await else {
        return web::redirect_to_sign_in(&state, &headers, "/tools");
    };
    let Some(fields) = tool_fields(&pairs) else {
        return pages::error_page_as(&state, &headers, StatusCode::BAD_REQUEST, false, "tools")
            .await;
    };
    let (name, slug, errors) = validate_tool(&state.db, &fields, None).await;
    if !errors.is_empty() {
        let tool = form_tool(&fields, &name, &slug, &viewer);
        return render_form(&state, &headers, &viewer, tool, errors, false).await;
    }
    let inserted = sqlx::query_scalar::<_, i32>(
        "INSERT INTO tools (user_id, name, lowercase_name, slug, type, description,
                            instructions, source_url, created_at, updated_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, now(), now()) RETURNING id",
    )
    .bind(viewer.id)
    .bind(&name)
    .bind(name.to_lowercase())
    .bind(&slug)
    .bind(fields.r#type.clone().unwrap_or_default())
    .bind(fields.description.clone())
    .bind(fields.instructions.clone())
    .bind(fields.source_url.clone().filter(|value| !value.is_empty()))
    .fetch_optional(&state.db)
    .await
    .ok()
    .flatten();
    if inserted.is_none() {
        let tool = form_tool(&fields, &name, &slug, &viewer);
        return render_form(
            &state,
            &headers,
            &viewer,
            tool,
            vec!["Name has already been taken".into()],
            false,
        )
        .await;
    }
    let notice = t("tool.created", &[("name", &name)]);
    web::redirect_with_flash(
        &state,
        &headers,
        &format!("/users/{}/tools", viewer.username),
        "notice",
        &notice,
    )
}

async fn find_tool_row(db: &PgPool, slug: &str, viewer_id: Option<i32>) -> Option<ToolRow> {
    sqlx::query_as::<_, ToolRow>(&format!("{TOOL_SELECT} WHERE t.slug = $2"))
        .bind(viewer_id)
        .bind(slug)
        .fetch_optional(db)
        .await
        .ok()
        .flatten()
}

/// `manager?`: owner, collaborator or admin. `tool.destroy?` is owner or admin.
async fn may_manage(db: &PgPool, viewer: &web::Viewer, row: &ToolRow) -> (bool, bool) {
    if viewer.admin || viewer.id == row.owner_id {
        return (true, true);
    }
    let collab: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM collaborators
          WHERE resourceable_type = 'Tool' AND resourceable_id = $1 AND user_id = $2)",
    )
    .bind(row.id)
    .bind(viewer.id)
    .fetch_one(db)
    .await
    .unwrap_or(false);
    (collab, false)
}

/// `PATCH /tools/:id`, `PUT /tools/:id`.
pub async fn update(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
    axum::extract::Form(pairs): axum::extract::Form<Vec<(String, String)>>,
) -> Response {
    let Some(viewer) = web::viewer(&state, &headers).await else {
        return web::redirect_to_sign_in(&state, &headers, &format!("/tools/{slug}"));
    };
    let Some(row) = find_tool_row(&state.db, &slug, Some(viewer.id)).await else {
        return pages::error_page_as(&state, &headers, StatusCode::NOT_FOUND, false, "tools").await;
    };
    let (manage, _) = may_manage(&state.db, &viewer, &row).await;
    if !manage {
        return pages::error_page_as(&state, &headers, StatusCode::NOT_FOUND, false, "tools").await;
    }
    let Some(fields) = tool_fields(&pairs) else {
        return pages::error_page_as(&state, &headers, StatusCode::BAD_REQUEST, false, "tools")
            .await;
    };
    let (name, new_slug, errors) = validate_tool(&state.db, &fields, Some(row.id)).await;
    let current_slug = row.slug.clone();
    if !errors.is_empty() {
        let mut tool = build(row, Some(&viewer));
        tool.name = name.clone();
        tool.slug = new_slug.clone();
        tool.name_value = Some(name);
        tool.slug_value = Some(new_slug);
        tool.kind = fields.r#type.clone().unwrap_or_default();
        tool.type_label = pretty_type(&tool.kind);
        tool.source_url = fields.source_url.clone();
        // `update` only assigns the keys that were submitted.
        if let Some(description) = fields.description.clone() {
            tool.description_raw = description;
        }
        if let Some(instructions) = fields.instructions.clone() {
            tool.instructions_raw = instructions;
        }
        return render_form(&state, &headers, &viewer, tool, errors, true).await;
    }
    let up_for_adoption = fields.up_for_adoption.as_deref();
    let _ = sqlx::query(
        "UPDATE tools SET user_id = user_id, name = $2, lowercase_name = $3, slug = $4,
                type = $5, description = $6, instructions = $7, source_url = $8,
                up_for_adoption = CASE WHEN $9::text IS NULL THEN up_for_adoption
                                       ELSE $9::boolean END,
                updated_at = now()
          WHERE id = $1",
    )
    .bind(row.id)
    .bind(&name)
    .bind(name.to_lowercase())
    .bind(&new_slug)
    .bind(fields.r#type.clone().unwrap_or_default())
    .bind(fields.description.clone())
    .bind(fields.instructions.clone())
    .bind(fields.source_url.clone().filter(|value| !value.is_empty()))
    .bind(up_for_adoption)
    .execute(&state.db)
    .await;
    let notice = match up_for_adoption {
        Some("true") => t("adoption.up", &[("name", &name)]),
        Some(_) => t("adoption.down", &[("name", &name)]),
        None => t("tool.updated", &[("name", &name)]),
    };
    web::redirect_with_flash(
        &state,
        &headers,
        &format!("/tools/{current_slug}"),
        "notice",
        &notice,
    )
}

/// `DELETE /tools/:id`.
pub async fn destroy(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
) -> Response {
    let Some(viewer) = web::viewer(&state, &headers).await else {
        return web::redirect_to_sign_in(&state, &headers, &format!("/tools/{slug}"));
    };
    let Some(row) = find_tool_row(&state.db, &slug, Some(viewer.id)).await else {
        return pages::error_page_as(&state, &headers, StatusCode::NOT_FOUND, false, "tools").await;
    };
    let (_, deletable) = may_manage(&state.db, &viewer, &row).await;
    if !deletable {
        return pages::error_page_as(&state, &headers, StatusCode::NOT_FOUND, false, "tools").await;
    }
    let name = row.name.clone();
    let _ = sqlx::query(
        "DELETE FROM collaborators WHERE resourceable_type = 'Tool' AND resourceable_id = $1",
    )
    .bind(row.id)
    .execute(&state.db)
    .await;
    let _ = sqlx::query("DELETE FROM tools WHERE id = $1")
        .bind(row.id)
        .execute(&state.db)
        .await;
    let notice = t("tool.deleted", &[("name", &name)]);
    web::redirect_with_flash(
        &state,
        &headers,
        &format!("/users/{}/tools", row.owner_username),
        "notice",
        &notice,
    )
}

/// `POST /tools/:id/adoption`.
pub async fn adoption(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(slug): Path<String>,
) -> Response {
    let Some(row) = find_tool_row(&state.db, &slug, None).await else {
        return pages::error_page_as(&state, &headers, StatusCode::NOT_FOUND, false, "tools").await;
    };
    let Some(viewer) = web::viewer(&state, &headers).await else {
        return web::redirect_to_sign_in(&state, &headers, &format!("/tools/{slug}"));
    };
    let _ = crate::jobs::perform_mailer(
        &state,
        "AdoptionMailer",
        "interest_email",
        vec![
            crate::jobs::MailerArg::Int(row.id.into()),
            crate::jobs::MailerArg::Value("Tool"),
            crate::jobs::MailerArg::Int(viewer.id.into()),
        ],
    )
    .await;
    let notice = t("adoption.email_sent", &[("cookbook_or_tool", &row.name)]);
    web::redirect_with_flash(
        &state,
        &headers,
        &format!("/tools/{slug}"),
        "notice",
        &notice,
    )
}

#[derive(Template)]
#[template(path = "tools/directory.html")]
struct Directory<'a> {
    shell: Shell<'a>,
    recent: Vec<Tool>,
    type_links: Vec<(String, String)>,
}

/// `GET /tools-directory`.
pub async fn directory(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let viewer = web::viewer(&state, &headers).await;
    let rows = sqlx::query_as::<_, ToolRow>(&format!("{TOOL_SELECT} ORDER BY t.id DESC LIMIT 5"))
        .bind(viewer.as_ref().map(|v| v.id))
        .fetch_all(&state.db)
        .await
        .unwrap_or_default();
    let recent = rows
        .into_iter()
        .map(|row| build(row, viewer.as_ref()))
        .collect();
    let shell = pages::shell(&state, viewer.as_ref(), &headers)
        .title("Tools Directory")
        .description("Browse and search Chef tools and plugins.")
        .search("/tools", "Tools", None);
    let page = Directory {
        shell,
        recent,
        type_links: types()
            .iter()
            .map(|kind| (kind.label.clone(), format!("/tools?type={}", kind.value)))
            .collect(),
    };
    Page::ok(page).into_response()
}

#[derive(Template)]
#[template(path = "tools/index.atom", escape = "none")]
struct IndexAtom {
    host: String,
    base: String,
    self_url: String,
    updated: String,
    entries: Vec<IndexAtomEntry>,
}

struct IndexAtomEntry {
    id: i32,
    name: String,
    published: String,
    updated: String,
    slug: String,
    content: String,
    maintainer: String,
    owner: String,
}

fn iso(value: &NaiveDateTime) -> String {
    value.format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

/// `GET /tools.atom`, the `format.atom` arm of `index`.
pub async fn index_atom(
    State(state): State<AppState>,
    headers: HeaderMap,
    uri: axum::http::Uri,
    Query(params): Query<IndexParams>,
) -> Response {
    let viewer = web::viewer(&state, &headers).await;
    let viewer_id = viewer.as_ref().map(|v| v.id);
    let mut conditions: Vec<String> = Vec::new();
    let mut binds: Vec<String> = Vec::new();
    if let Some(q) = params.q.as_deref().filter(|q| !q.is_empty()) {
        binds.push(format!("%{q}%"));
        conditions.push(format!(
            "(t.name ILIKE ${0} OR t.description ILIKE ${0})",
            binds.len() + 1
        ));
    }
    if let Some(kind) = params.r#type.as_deref().filter(|k| TYPES.contains(k)) {
        binds.push(kind.to_owned());
        conditions.push(format!("t.type = ${}", binds.len() + 1));
    }
    let where_sql = if conditions.is_empty() {
        String::new()
    } else {
        format!(" WHERE {}", conditions.join(" AND "))
    };
    let order_sql = if params.order.as_deref() == Some("created_at") {
        " ORDER BY t.created_at"
    } else {
        " ORDER BY t.name"
    };
    let page_number = params.page.unwrap_or(1).max(1);
    let offset = (page_number - 1) * 20;
    let sql = format!("{TOOL_SELECT}{where_sql}{order_sql} LIMIT 20 OFFSET {offset}");
    let mut query = sqlx::query_as::<_, ToolRow>(&sql).bind(viewer_id);
    for bind in &binds {
        query = query.bind(bind);
    }
    let rows = query.fetch_all(&state.db).await.unwrap_or_default();
    // The feed's self link is the request URL, which Rails rebuilds verbatim.
    let self_url = match uri.query() {
        Some(query) if !query.is_empty() => {
            format!("/tools.atom?{}", query.replace('&', "&amp;"))
        }
        _ => "/tools.atom".to_owned(),
    };
    let feed = IndexAtom {
        host: crate::web::cookbooks::host_of(&headers),
        base: state.cfg.base_url(),
        self_url,
        updated: chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string(),
        entries: rows
            .into_iter()
            .map(|row| IndexAtomEntry {
                id: row.id,
                name: row.name,
                published: iso(&row.created_at),
                updated: iso(&row.updated_at),
                slug: row.slug.clone(),
                content: web::html_escape(row.description.as_deref().unwrap_or("")),
                maintainer: row.owner_username.clone(),
                owner: row.owner_username,
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
