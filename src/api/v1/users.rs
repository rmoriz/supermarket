//! `Api::V1::UsersController#show`.

use axum::extract::{Path, State};
use axum::Json;
use serde_json::{json, Map, Value};

use crate::api::v1::cookbooks::{cookbook_url, internal};
use crate::error::ApiError;
use crate::state::AppState;

/// user id, first name, last name, company, twitter, slack
type UserColumns = (
    i32,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
);

pub async fn show(
    State(state): State<AppState>,
    Path(username): Path<String>,
) -> Result<Json<Value>, ApiError> {
    // `Account.for("chef_oauth2").with_username(...)`: a GitHub-only account is a 404.
    let user: Option<UserColumns> = sqlx::query_as(
        "SELECT u.id, u.first_name, u.last_name, u.company, u.twitter_username, u.slack_username
               FROM accounts a JOIN users u ON u.id = a.user_id
              WHERE a.provider = 'chef_oauth2' AND a.username = $1",
    )
    .bind(&username)
    .fetch_optional(&state.db)
    .await
    .map_err(internal)?;

    let Some((user_id, first_name, last_name, company, twitter, slack)) = user else {
        return Err(ApiError::NotFound);
    };

    let name = match (first_name.as_deref(), last_name.as_deref()) {
        (Some(first), Some(last)) => format!("{first} {last}").trim().to_owned(),
        (Some(first), None) => first.to_owned(),
        (None, Some(last)) => last.to_owned(),
        (None, None) => username.clone(),
    };

    let github: Vec<String> = sqlx::query_scalar(
        "SELECT username FROM accounts WHERE user_id = $1 AND provider = 'github' AND username IS NOT NULL",
    )
    .bind(user_id)
    .fetch_all(&state.db)
    .await
    .map_err(internal)?;
    let mut github = github;
    github.sort();

    async fn map_of(
        state: &AppState,
        sql: &str,
        user_id: i32,
        base: &str,
    ) -> Result<Map<String, Value>, ApiError> {
        let rows: Vec<String> = sqlx::query_scalar(sql)
            .bind(user_id)
            .fetch_all(&state.db)
            .await
            .map_err(internal)?;
        Ok(rows
            .into_iter()
            .map(|name| {
                (
                    name.clone(),
                    json!(format!("{base}/api/v1/cookbooks/{name}")),
                )
            })
            .collect())
    }

    let base = state.cfg.base_url();
    let owned = map_of(
        &state,
        "SELECT name FROM cookbooks WHERE user_id = $1 ORDER BY name ASC",
        user_id,
        &base,
    )
    .await?;
    let collaborated = map_of(
        &state,
        "SELECT c.name FROM cookbooks c
           JOIN collaborators co ON co.resourceable_id = c.id AND co.resourceable_type = 'Cookbook'
          WHERE co.user_id = $1 ORDER BY c.name ASC",
        user_id,
        &base,
    )
    .await?;
    let followed = map_of(
        &state,
        "SELECT c.name FROM cookbooks c
           JOIN cookbook_followers f ON f.cookbook_id = c.id
          WHERE f.user_id = $1 ORDER BY c.name ASC",
        user_id,
        &base,
    )
    .await?;

    let owned_tools: Vec<(String, String)> =
        sqlx::query_as("SELECT name, slug FROM tools WHERE user_id = $1 ORDER BY name ASC")
            .bind(user_id)
            .fetch_all(&state.db)
            .await
            .map_err(internal)?;
    let collaborated_tools: Vec<(String, String)> = sqlx::query_as(
        "SELECT t.name, t.slug FROM tools t
           JOIN collaborators co ON co.resourceable_id = t.id AND co.resourceable_type = 'Tool'
          WHERE co.user_id = $1 ORDER BY t.name ASC",
    )
    .bind(user_id)
    .fetch_all(&state.db)
    .await
    .map_err(internal)?;

    let tool_map = |rows: Vec<(String, String)>| -> Map<String, Value> {
        rows.into_iter()
            .map(|(name, slug)| (name, json!(format!("{base}/api/v1/tools/{slug}"))))
            .collect()
    };

    // jbuilder drops a key whose block emitted nothing, so empty maps vanish.
    fn with(map: Map<String, Value>, key: &str, value: Map<String, Value>) -> Map<String, Value> {
        let mut map = map;
        if !value.is_empty() {
            map.insert(key.to_owned(), Value::Object(value));
        }
        map
    }

    let mut cookbooks = with(Map::new(), "owns", owned);
    cookbooks = with(cookbooks, "collaborates", collaborated);
    cookbooks = with(cookbooks, "follows", followed);
    let mut tools = with(Map::new(), "owns", tool_map(owned_tools));
    tools = with(tools, "collaborates", tool_map(collaborated_tools));

    let mut body = Map::new();
    body.insert("username".into(), json!(username));
    body.insert("name".into(), json!(name));
    body.insert("company".into(), json!(company));
    body.insert("github".into(), json!(github));
    body.insert("twitter".into(), json!(twitter));
    body.insert("slack".into(), json!(slack));
    if !cookbooks.is_empty() {
        body.insert("cookbooks".into(), Value::Object(cookbooks));
    }
    if !tools.is_empty() {
        body.insert("tools".into(), Value::Object(tools));
    }
    Ok(Json(Value::Object(body)))
}

#[allow(dead_code)]
fn unused(base: &str, name: &str) -> String {
    cookbook_url(base, name)
}
