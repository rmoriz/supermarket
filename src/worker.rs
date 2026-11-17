//! Sidekiq-compatible job consumer.
//!
//! Jobs are popped reliably (`BRPOPLPUSH` into `queue:default:inprogress`),
//! matched on `class` and executed. The payload shapes are byte-compatible with
//! the Rails producers, so either side can drain the queue during the cutover.

use std::time::Duration;

use redis::AsyncCommands;
use serde_json::{json, Value};

use crate::jobs::{flush_universe, perform_async, MailerArg, RecordArg, RecordValue};
use crate::mail;
use crate::state::AppState;
use crate::web;

const QUEUE: &str = "default";

/// Current time as RFC 3339 with a `+00:00` offset, like `Time#iso8601`.
fn iso8601(timestamp: chrono::NaiveDateTime) -> String {
    timestamp.format("%Y-%m-%dT%H:%M:%S+00:00").to_string()
}

/// `CookbookNotifyWorker`: tell followers about a newly published version when
/// they are subscribed and their oc-id token is not `imported`.
async fn cookbook_notify(state: &AppState, version_id: i32) {
    let row: Option<(String, String, i32)> = sqlx::query_as(
        "SELECT v.version, c.name, c.id FROM cookbook_versions v
           JOIN cookbooks c ON c.id = v.cookbook_id WHERE v.id = $1",
    )
    .bind(version_id)
    .fetch_optional(&state.db)
    .await
    .ok()
    .flatten();
    let Some((version, name, cookbook_id)) = row else {
        return;
    };
    let followers: Vec<(i32, String, String)> = sqlx::query_as(
        "SELECT u.id, u.email, a.username FROM cookbook_followers f
           JOIN users u ON u.id = f.user_id
           JOIN accounts a ON a.user_id = u.id AND a.provider = 'chef_oauth2'
          WHERE f.cookbook_id = $1 AND a.oauth_token <> 'imported'
            AND EXISTS (SELECT 1 FROM email_preferences ep
                          JOIN system_emails se ON se.id = ep.system_email_id
                         WHERE ep.user_id = u.id AND se.name = 'New cookbook version')",
    )
    .bind(cookbook_id)
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();

    for (user_id, email, _username) in followers {
        let Some(preference) = email_preference(state, user_id, "New cookbook version").await
        else {
            continue;
        };
        let version_url = format!(
            "{}/cookbooks/{name}/versions/{version}",
            state.cfg.base_url()
        );
        let changelog: Option<String> = sqlx::query_scalar(
            "SELECT ch.changelog FROM html_changelogs ch
               JOIN cookbook_versions v ON v.id = ch.cookbook_version_id WHERE v.id = $1",
        )
        .bind(version_id)
        .fetch_optional(&state.db)
        .await
        .ok()
        .flatten();
        let html = changelog.as_deref().map(web::markdown::render);
        let unsubscribe = format!("{}/unsubscribe/{}", state.cfg.base_url(), preference);
        let message = mail::follower_notification(
            &state.cfg,
            &email,
            &name,
            &version,
            &version_url,
            html.as_deref(),
            &format!("{version_url}#changelog"),
            &unsubscribe,
        );
        mail::deliver(&state.cfg, &message).await;
    }
}

async fn name_of(state: &AppState, cookbook_id: i32) -> String {
    sqlx::query_scalar("SELECT name FROM cookbooks WHERE id = $1")
        .bind(cookbook_id)
        .fetch_optional(&state.db)
        .await
        .ok()
        .flatten()
        .unwrap_or_default()
}

async fn email_preference(state: &AppState, user_id: i32, name: &str) -> Option<String> {
    sqlx::query_scalar(
        "SELECT ep.token FROM email_preferences ep
           JOIN system_emails se ON se.id = ep.system_email_id
          WHERE ep.user_id = $1 AND se.name = $2",
    )
    .bind(user_id)
    .bind(name)
    .fetch_optional(&state.db)
    .await
    .ok()
    .flatten()
}

/// `CookbookDeletionWorker`: notify followers and collaborators, then drop
/// their rows.
async fn cookbook_deleted(state: &AppState, cookbook: &Value) {
    let Some(id) = cookbook.get("id").and_then(Value::as_i64) else {
        return;
    };
    let id = id as i32;
    let Some(name) = cookbook
        .get("name")
        .and_then(Value::as_str)
        .map(str::to_owned)
    else {
        return;
    };
    let users: Vec<(i32, String)> = sqlx::query_as(
        "SELECT u.id, u.email FROM users u
          WHERE u.id IN (
            SELECT user_id FROM cookbook_followers WHERE cookbook_id = $1
            UNION SELECT user_id FROM collaborators
                   WHERE resourceable_type = 'Cookbook' AND resourceable_id = $1)
            AND EXISTS (SELECT 1 FROM email_preferences ep
                          JOIN system_emails se ON se.id = ep.system_email_id
                         WHERE ep.user_id = u.id AND se.name = 'Cookbook deleted')",
    )
    .bind(id)
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();
    for (user_id, email) in users {
        let Some(preference) = email_preference(state, user_id, "Cookbook deleted").await else {
            continue;
        };
        let unsubscribe = format!("{}/unsubscribe/{}", state.cfg.base_url(), preference);
        let message = mail::cookbook_deleted(&state.cfg, &email, &name, &unsubscribe);
        mail::deliver(&state.cfg, &message).await;
    }
    let _ = sqlx::query("DELETE FROM cookbook_followers WHERE cookbook_id = $1")
        .bind(id)
        .execute(&state.db)
        .await;
    let _ = sqlx::query(
        "DELETE FROM collaborators WHERE resourceable_type = 'Cookbook' AND resourceable_id = $1",
    )
    .bind(id)
    .execute(&state.db)
    .await;
}

/// `CookbookDeprecatedNotifier`: owner, subscribers among collaborators and
/// followers.
async fn cookbook_deprecated(state: &AppState, cookbook_id: i32) {
    let row: Option<(String, Option<i32>, String, Option<String>)> = sqlx::query_as(
        "SELECT c.name, c.replacement_id, COALESCE(c.deprecation_reason, ''), u.email
           FROM cookbooks c LEFT JOIN users u ON u.id = c.user_id WHERE c.id = $1",
    )
    .bind(cookbook_id)
    .fetch_optional(&state.db)
    .await
    .ok()
    .flatten();
    let Some((name, replacement_id, stored_reason, owner_email)) = row else {
        return;
    };
    let reason = if stored_reason.trim().is_empty() {
        format!(
            "The {name} cookbook has been deprecated and is no longer being maintained by its \
             authors.  Use of the {name} cookbook is no longer recommended."
        )
    } else {
        stored_reason
    };
    let replacement: Option<(String, String)> = match replacement_id {
        Some(id) => {
            let replacement = name_of(state, id).await;
            Some((
                replacement.clone(),
                format!("{}/cookbooks/{replacement}", state.cfg.base_url()),
            ))
        }
        None => None,
    };
    let subscribers: Vec<(i32, String)> = sqlx::query_as(
        "SELECT u.id, u.email FROM users u
          WHERE u.id IN (
            SELECT user_id FROM collaborators
             WHERE resourceable_type = 'Cookbook' AND resourceable_id = $1
            UNION SELECT user_id FROM cookbook_followers WHERE cookbook_id = $1)
            AND EXISTS (SELECT 1 FROM email_preferences ep
                          JOIN system_emails se ON se.id = ep.system_email_id
                         WHERE ep.user_id = u.id AND se.name = 'Cookbook deprecated')",
    )
    .bind(cookbook_id)
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();
    let mut recipients: Vec<(i32, String)> = Vec::new();
    for user in subscribers {
        if !recipients.iter().any(|(id, _)| *id == user.0) {
            recipients.push(user);
        }
    }
    if let (Some(owner_id), Some(email)) = (
        sqlx::query_scalar::<_, Option<i32>>("SELECT user_id FROM cookbooks WHERE id = $1")
            .bind(cookbook_id)
            .fetch_optional(&state.db)
            .await
            .ok()
            .flatten()
            .flatten(),
        owner_email,
    ) {
        if !recipients.iter().any(|(id, _)| *id == owner_id) {
            recipients.push((owner_id, email));
        }
    }
    let cookbook_url = format!("{}/cookbooks/{}", state.cfg.base_url(), name);
    for (user_id, email) in recipients {
        let Some(preference) = email_preference(state, user_id, "Cookbook deprecated").await else {
            continue;
        };
        let unsubscribe = format!("{}/unsubscribe/{}", state.cfg.base_url(), preference);
        let message = mail::cookbook_deprecated(
            &state.cfg,
            &email,
            &name,
            &cookbook_url,
            &reason,
            replacement
                .as_ref()
                .map(|(replacement, url)| (replacement.as_str(), url.as_str())),
            &unsubscribe,
        );
        mail::deliver(&state.cfg, &message).await;
    }
}

/// cookbook name, version, tarball file name, legacy id, tarball updated_at
type NotifyRow = (
    String,
    String,
    Option<String>,
    Option<i32>,
    Option<chrono::NaiveDateTime>,
);

/// `FieriNotifyWorker`.
async fn fieri_notify(state: &AppState, version_id: i32) {
    let (Some(url), Some(key)) = (
        state.cfg.fieri_url.as_deref(),
        state.cfg.fieri_key.as_deref(),
    ) else {
        return;
    };
    let row: Option<NotifyRow> = sqlx::query_as(
        "SELECT c.name, v.version, v.tarball_file_name, v.legacy_id, v.tarball_updated_at
               FROM cookbook_versions v JOIN cookbooks c ON c.id = v.cookbook_id
              WHERE v.id = $1",
    )
    .bind(version_id)
    .fetch_optional(&state.db)
    .await
    .ok()
    .flatten();
    let Some((name, version, file_name, legacy_id, updated_at)) = row else {
        return;
    };
    // `CookbookVersion#cookbook_artifact_url`, not the /download endpoint.
    let artifact = crate::api::v1::cookbook_versions::artifact_url(
        state,
        version_id,
        legacy_id,
        file_name.as_deref().unwrap_or("tarball.tgz"),
        updated_at,
    )
    .await;
    let body = json!({
        "fieri_key": key,
        "cookbook": { "name": name, "version": version, "artifact_url": artifact },
    });
    let _ = reqwest::Client::new()
        .post(url)
        .header("Content-Type", "application/json")
        .body(body.to_string())
        .send()
        .await;
}

/// `OauthTokenRefreshScheduleWorker`: `Account.tokens_expiring_soon(now)`.
async fn oauth_refresh_schedule(state: &AppState) {
    let ids: Vec<i32> = sqlx::query_scalar(
        "SELECT id FROM accounts
          WHERE oauth_expires BETWEEN NOW() AND NOW() + INTERVAL '25 minutes 1 second'",
    )
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();
    for id in ids {
        let _ = perform_async(state, "OauthTokenRefreshWorker", vec![json!(id)]).await;
    }
}

/// `OauthTokenRefreshWorker#perform`.
async fn oauth_refresh(state: &AppState, account_id: i32) {
    let row: Option<(String, Option<String>, Option<chrono::NaiveDateTime>)> = sqlx::query_as(
        "SELECT oauth_token, oauth_refresh_token, oauth_expires FROM accounts WHERE id = $1",
    )
    .bind(account_id)
    .fetch_optional(&state.db)
    .await
    .ok()
    .flatten();
    let Some((token, refresh_token, _expires)) = row else {
        return;
    };
    let Some(refresh_token) = refresh_token else {
        return;
    };
    let Some(client_id) = state.cfg.chef_oauth2_app_id.as_deref() else {
        return;
    };
    let Some(client_secret) = state.cfg.chef_oauth2_secret.as_deref() else {
        return;
    };
    let mut endpoint = state
        .cfg
        .chef_oauth2_url
        .clone()
        .unwrap_or_else(|| "https://id.chef.io/id/oauth".to_owned());
    endpoint.push_str("/token");
    let form = [
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh_token.as_str()),
        ("client_id", client_id),
        ("client_secret", client_secret),
    ];
    let Ok(response) = reqwest::Client::new()
        .post(&endpoint)
        .form(&form)
        .send()
        .await
    else {
        return;
    };
    let status = response.status();
    let Ok(payload) = response.json::<Value>().await else {
        return;
    };
    if !status.is_success() {
        // 5xx is retried by Sidekiq; 4xx is logged and dropped.
        return;
    }
    let new_token = payload.get("access_token").and_then(Value::as_str);
    let new_refresh = payload.get("refresh_token").and_then(Value::as_str);
    let expires_in = payload.get("expires_in").and_then(Value::as_i64);
    if let Some(new_token) = new_token {
        let _ = sqlx::query(
            "UPDATE accounts SET oauth_token = $2,
                    oauth_refresh_token = COALESCE($3, oauth_refresh_token),
                    oauth_expires = NOW() + ($4 || ' seconds')::interval,
                    updated_at = NOW()
              WHERE id = $1",
        )
        .bind(account_id)
        .bind(new_token)
        .bind(new_refresh.unwrap_or(&token))
        .bind(expires_in.unwrap_or(3600).to_string())
        .execute(&state.db)
        .await;
    }
}

/// `SitemapRefreshWorker`: rewrite `public/sitemap.xml.gz`, then ping the
/// search engines.
async fn sitemap_refresh(state: &AppState) {
    let body = sitemap_body(state).await;

    use flate2::write::GzEncoder;
    use flate2::Compression;
    use std::io::Write;

    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    if encoder.write_all(body.as_bytes()).is_err() {
        return;
    }
    let Ok(compressed) = encoder.finish() else {
        return;
    };
    let _ = std::fs::create_dir_all("public");
    let _ = std::fs::write("public/sitemap.xml.gz", compressed);

    let target = format!("{}/sitemap.xml.gz", state.cfg.base_url());
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
    {
        Ok(client) => client,
        Err(_) => return,
    };
    for endpoint in [
        format!("http://www.google.com/webmasters/tools/ping?sitemap={target}"),
        format!("http://www.bing.com/webmaster/ping.aspx?siteMap={target}"),
    ] {
        let _ = client.get(endpoint).send().await;
    }
}

/// The `config/sitemap.rb` document: root, directory, cookbooks, versions and
/// every user (`user_path(user)`, so an account without a username maps to
/// `/users/`).
pub async fn sitemap_body(state: &AppState) -> String {
    let base = state.cfg.base_url();
    let now = chrono::Utc::now()
        .format("%Y-%m-%dT%H:%M:%S+00:00")
        .to_string();
    let mut body = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?><urlset xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" \
         xsi:schemaLocation=\"http://www.sitemaps.org/schemas/sitemap/0.9 http://www.sitemaps.org/schemas/sitemap/0.9/sitemap.xsd\" \
         xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\" \
         xmlns:image=\"http://www.google.com/schemas/sitemap-image/1.1\" \
         xmlns:video=\"http://www.google.com/schemas/sitemap-video/1.1\" \
         xmlns:news=\"http://www.google.com/schemas/sitemap-news/0.9\" \
         xmlns:mobile=\"http://www.google.com/schemas/sitemap-mobile/1.0\" \
         xmlns:pagemap=\"http://www.google.com/schemas/sitemap-pagemap/1.0\" \
         xmlns:xhtml=\"http://www.w3.org/1999/xhtml\">",
    );
    let entry = |loc: String, lastmod: String, priority: &str| {
        format!(
            "<url><loc>{loc}</loc><lastmod>{lastmod}</lastmod><changefreq>weekly</changefreq><priority>{priority}</priority></url>"
        )
    };
    body.push_str(&entry(base.clone(), now.clone(), "1.0"));
    body.push_str(&entry(format!("{base}/cookbooks-directory"), now, "0.5"));
    let cookbooks: Vec<(String, chrono::NaiveDateTime)> =
        sqlx::query_as("SELECT name, updated_at FROM cookbooks ORDER BY id")
            .fetch_all(&state.db)
            .await
            .unwrap_or_default();
    for (name, updated_at) in &cookbooks {
        body.push_str(&entry(
            format!("{base}/cookbooks/{name}"),
            iso8601(*updated_at),
            "0.8",
        ));
    }
    let versions: Vec<(String, String, chrono::NaiveDateTime)> = sqlx::query_as(
        "SELECT c.name, v.version, v.updated_at FROM cookbook_versions v
           JOIN cookbooks c ON c.id = v.cookbook_id ORDER BY v.id",
    )
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();
    for (cookbook, version, updated_at) in &versions {
        body.push_str(&entry(
            format!("{base}/cookbooks/{cookbook}/versions/{version}"),
            iso8601(*updated_at),
            "0.5",
        ));
    }
    let users: Vec<(String, chrono::NaiveDateTime)> = sqlx::query_as(
        "SELECT COALESCE(a.username, ''), u.updated_at FROM users u
           LEFT JOIN accounts a ON a.user_id = u.id AND a.provider = 'chef_oauth2'
          ORDER BY u.id",
    )
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();
    for (username, updated_at) in &users {
        body.push_str(&entry(
            format!("{base}/users/{username}"),
            iso8601(*updated_at),
            "0.5",
        ));
    }
    body.push_str("</urlset>");
    body
}

/// `SpdxLicenseUpdateWorker`.
async fn spdx_update(state: &AppState, licenses: &Value, version_id: i64) {
    let license: Option<String> =
        sqlx::query_scalar("SELECT license FROM cookbook_versions WHERE id = $1")
            .bind(version_id as i32)
            .fetch_optional(&state.db)
            .await
            .ok()
            .flatten();
    let Some(license) = license else {
        return;
    };
    let Some(records) = licenses.as_array() else {
        return;
    };
    let found = records
        .iter()
        .find(|record| record.get("licenseId").and_then(Value::as_str) == Some(license.as_str()));
    let Some(found) = found else {
        return;
    };
    let url = found
        .get("detailsUrl")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let _ = sqlx::query(
        "UPDATE cookbook_versions SET spdx_license_url = $2, updated_at = NOW() WHERE id = $1",
    )
    .bind(version_id as i32)
    .bind(url)
    .execute(&state.db)
    .await;
}

// ------------------------------------------------------------- delayed mail

/// Parsed `Sidekiq::Extensions::DelayedMailer` argument.
struct DelayedMailer {
    mailer: String,
    method: String,
    args: Vec<Value>,
}

/// Reads the constrained YAML `Sidekiq::Extensions::DelayedMailer` writes.
///
/// The document is `---`, a `!ruby/class`, `:method` and one `- - <arg>`
/// slice per argument; records are `!ruby/object:<Model>` followed by
/// `concise_attributes` entries with `name`/`value_before_type_cast` pairs.
fn parse_delayed_mailer(yaml: &str) -> Option<DelayedMailer> {
    let mut mailer = None;
    let mut method = None;
    let mut args: Vec<Value> = Vec::new();
    let mut record: Option<usize> = None;
    let mut attribute: Option<(String, Option<String>)> = None;

    for line in yaml.lines() {
        if let Some(rest) = line.strip_prefix("- !ruby/class '") {
            mailer = Some(rest.trim_end_matches('\'').to_owned());
            continue;
        }
        if let Some(rest) = line.strip_prefix("- :") {
            method = Some(rest.trim().to_owned());
            continue;
        }
        // Argument slice start: `- - 42` or `- - !ruby/object:Model`.
        if let Some(rest) = line.strip_prefix("- - ") {
            let rest = rest.trim();
            if let Some(class) = rest.strip_prefix("!ruby/object:") {
                args.push(json!({ "class": class.trim(), "attrs": [] }));
                record = Some(args.len() - 1);
            } else {
                args.push(Value::String(rest.to_owned()));
                record = None;
            }
            attribute = None;
            continue;
        }
        // Further arguments in the same slice start at two spaces.
        if let Some(rest) = line.strip_prefix("  - ") {
            let rest = rest.trim();
            if let Some(class) = rest.strip_prefix("!ruby/object:") {
                args.push(json!({ "class": class.trim(), "attrs": [] }));
                record = Some(args.len() - 1);
            } else {
                args.push(Value::String(rest.to_owned()));
                record = None;
            }
            attribute = None;
            continue;
        }
        let trimmed = line.trim_start();
        if trimmed.starts_with("- !ruby/object:ActiveModel::Attribute") {
            attribute = Some((String::new(), None));
            continue;
        }
        if let Some(name) = trimmed.strip_prefix("name: ") {
            if let Some((slot, _)) = attribute.as_mut() {
                *slot = name.trim().to_owned();
            }
            continue;
        }
        if let Some(value) = trimmed
            .strip_prefix("value_before_type_cast: ")
            .or_else(|| trimmed.strip_prefix("value: "))
        {
            if let (Some(index), Some((name, _))) = (record, attribute.as_ref()) {
                if !name.is_empty() {
                    if let Some(attrs) = args[index].get_mut("attrs").and_then(Value::as_array_mut)
                    {
                        attrs.push(json!({ "name": name, "value": value.trim() }));
                    }
                }
            }
            attribute = None;
        }
    }
    Some(DelayedMailer {
        mailer: mailer?,
        method: method?,
        args,
    })
}

fn record_id(record: &Value) -> Option<i32> {
    record
        .get("attrs")?
        .as_array()?
        .iter()
        .find(|attr| attr.get("name").and_then(Value::as_str) == Some("id"))?
        .get("value")?
        .as_str()?
        .parse()
        .ok()
}

#[allow(dead_code)]
fn record_attr<'a>(record: &'a Value, name: &str) -> Option<&'a str> {
    record
        .get("attrs")?
        .as_array()?
        .iter()
        .find(|attr| attr.get("name").and_then(Value::as_str) == Some(name))?
        .get("value")?
        .as_str()
}

/// Runs a queued `DelayedMailer`: loads the referenced records and renders.
async fn delayed_mailer(state: &AppState, yaml: &str) {
    let Some(parsed) = parse_delayed_mailer(yaml) else {
        return;
    };
    match (parsed.mailer.as_str(), parsed.method.as_str()) {
        ("CollaboratorMailer", "added_email") => {
            let Some(record) = parsed.args.first() else {
                return;
            };
            let Some(id) = record_id(record) else {
                return;
            };
            let row: Option<(i32, String, String, String)> = sqlx::query_as(
                "SELECT c.id, COALESCE(k.name, t.name, ''), c.resourceable_type, u.email
                   FROM collaborators c
                   LEFT JOIN cookbooks k ON k.id = c.resourceable_id AND c.resourceable_type = 'Cookbook'
                   LEFT JOIN tools t ON t.id = c.resourceable_id AND c.resourceable_type = 'Tool'
                   JOIN users u ON u.id = c.user_id
                  WHERE c.id = $1",
            )
            .bind(id)
            .fetch_optional(&state.db)
            .await
            .ok()
            .flatten();
            let Some((_, name, class_name, email)) = row else {
                return;
            };
            let url = format!("{}/{}/{}", state.cfg.base_url(), plural(&class_name), name);
            let message = mail::collaborator_added(&state.cfg, &email, &name, &class_name, &url);
            mail::deliver(&state.cfg, &message).await;
        }
        ("CookbookMailer", "cookbook_deleted_email") => {
            let Some(name) = parsed.args.first().and_then(Value::as_str) else {
                return;
            };
            let Some(user) = parsed.args.get(1) else {
                return;
            };
            let Some(user_id) = record_id(user) else {
                return;
            };
            let _ = cookbook_deleted_email_for(state, user_id, name).await;
        }
        ("CookbookMailer", "follower_notification_email") => {
            // Not enqueued by the app (workers use deliver_now) but kept for
            // payloads written by older Rails nodes.
        }
        ("CookbookMailer", "transfer_ownership_email") => {
            let Some(id) = parsed.args.first().and_then(|value| match value {
                Value::String(text) => text.parse::<i32>().ok(),
                _ => value.as_i64().map(|id| id as i32),
            }) else {
                return;
            };
            let row: Option<(String, String, String, String)> = sqlx::query_as(
                "SELECT u.first_name, u.last_name, c.name, r.email
                   FROM ownership_transfer_requests t
                   JOIN users u ON u.id = t.sender_id
                   JOIN cookbooks c ON c.id = t.cookbook_id
                   JOIN users r ON r.id = t.recipient_id
                  WHERE t.id = $1",
            )
            .bind(id)
            .fetch_optional(&state.db)
            .await
            .ok()
            .flatten();
            let Some((first, last, cookbook, email)) = row else {
                return;
            };
            let token: Option<String> =
                sqlx::query_scalar("SELECT token FROM ownership_transfer_requests WHERE id = $1")
                    .bind(id)
                    .fetch_optional(&state.db)
                    .await
                    .ok()
                    .flatten();
            let Some(token) = token else {
                return;
            };
            let message = mail::transfer_ownership(
                &state.cfg,
                &email,
                &format!("{first} {last}"),
                &cookbook,
                &format!("{}/ownership_transfer/{token}/accept", state.cfg.base_url()),
                &format!(
                    "{}/ownership_transfer/{token}/decline",
                    state.cfg.base_url()
                ),
            );
            mail::deliver(&state.cfg, &message).await;
        }
        ("AdoptionMailer", "interest_email") => {
            let Some(id) = parsed.args.first().and_then(|value| match value {
                Value::String(text) => text.parse::<i32>().ok(),
                _ => value.as_i64().map(|id| id as i32),
            }) else {
                return;
            };
            let Some(class_name) = parsed.args.get(1).and_then(Value::as_str) else {
                return;
            };
            let Some(user_id) = parsed.args.get(2).and_then(|value| match value {
                Value::String(text) => text.parse::<i32>().ok(),
                _ => value.as_i64().map(|id| id as i32),
            }) else {
                return;
            };
            let _ = adoption_interest(state, id, class_name, user_id).await;
        }
        ("AdoptionMailer", "follower_email") => {
            let Some(id) = parsed.args.first().and_then(|value| match value {
                Value::String(text) => text.parse::<i32>().ok(),
                _ => value.as_i64().map(|id| id as i32),
            }) else {
                return;
            };
            let Some(class_name) = parsed.args.get(1).and_then(Value::as_str) else {
                return;
            };
            let _ = adoption_follower(state, id, class_name).await;
        }
        _ => {}
    }
}

fn plural(class_name: &str) -> &'static str {
    match class_name {
        "Tool" => "tools",
        _ => "cookbooks",
    }
}

async fn cookbook_deleted_email_for(state: &AppState, user_id: i32, name: &str) {
    let email: Option<String> = sqlx::query_scalar("SELECT email FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_optional(&state.db)
        .await
        .ok()
        .flatten();
    let Some(email) = email else { return };
    let Some(preference) = email_preference(state, user_id, "Cookbook deleted").await else {
        return;
    };
    let message = mail::cookbook_deleted(
        &state.cfg,
        &email,
        name,
        &format!("{}/unsubscribe/{preference}", state.cfg.base_url()),
    );
    mail::deliver(&state.cfg, &message).await;
}

async fn adoption_interest(state: &AppState, id: i32, class_name: &str, user_id: i32) {
    let table = if class_name == "Tool" {
        "tools"
    } else {
        "cookbooks"
    };
    let row: Option<(String, String, String)> = sqlx::query_as(&format!(
        "SELECT r.name, u.email, o.email FROM {table} r
           JOIN users u ON u.id = $1 JOIN users o ON o.id = r.user_id WHERE r.id = $2"
    ))
    .bind(user_id)
    .bind(id)
    .fetch_optional(&state.db)
    .await
    .ok()
    .flatten();
    let Some((name, adopting_email, owner_email)) = row else {
        return;
    };
    let username: Option<String> = sqlx::query_scalar(
        "SELECT username FROM accounts WHERE user_id = $1 AND provider = 'chef_oauth2'",
    )
    .bind(user_id)
    .fetch_optional(&state.db)
    .await
    .ok()
    .flatten();
    let thing = class_name.to_lowercase();
    let message = mail::adoption_interest(
        &state.cfg,
        &owner_email,
        &thing,
        &name,
        &format!("{}/{}/{name}", state.cfg.base_url(), plural(class_name)),
        &username.unwrap_or_default(),
        &adopting_email,
    );
    mail::deliver(&state.cfg, &message).await;
}

async fn adoption_follower(state: &AppState, id: i32, class_name: &str) {
    let table = if class_name == "Tool" {
        "tools"
    } else {
        "cookbooks"
    };
    let name: Option<String> =
        sqlx::query_scalar(&format!("SELECT name FROM {table} WHERE id = $1"))
            .bind(id)
            .fetch_optional(&state.db)
            .await
            .ok()
            .flatten();
    let Some(name) = name else { return };
    let followers: Vec<String> = sqlx::query_scalar(
        "SELECT u.email FROM cookbook_followers f JOIN users u ON u.id = f.user_id
          WHERE f.cookbook_id = $1",
    )
    .bind(id)
    .fetch_all(&state.db)
    .await
    .unwrap_or_default();
    let thing = class_name.to_lowercase();
    for email in followers {
        let message = mail::adoption_follower(
            &state.cfg,
            &email,
            &thing,
            &name,
            &format!("{}/{}/{name}", state.cfg.base_url(), plural(class_name)),
        );
        mail::deliver(&state.cfg, &message).await;
    }
}

// ------------------------------------------------------------------ dispatch

/// Runs one job payload. Returns `true` when it was handled.
pub async fn execute(state: &AppState, payload: &str) -> bool {
    let Ok(job) = serde_json::from_str::<Value>(payload) else {
        return false;
    };
    let class = job.get("class").and_then(Value::as_str).unwrap_or_default();
    let args = job
        .get("args")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let int = |index: usize| args.get(index).and_then(Value::as_i64).map(|v| v as i32);
    match class {
        "CookbookNotifyWorker" => match int(0) {
            Some(id) => cookbook_notify(state, id).await,
            None => return false,
        },
        "CookbookDeletionWorker" => match args.first() {
            Some(value) => cookbook_deleted(state, value).await,
            None => return false,
        },
        "CookbookDeprecatedNotifier" => match int(0) {
            Some(id) => cookbook_deprecated(state, id).await,
            None => return false,
        },
        "FieriNotifyWorker" => match int(0) {
            Some(id) => fieri_notify(state, id).await,
            None => return false,
        },
        "OauthTokenRefreshScheduleWorker" => oauth_refresh_schedule(state).await,
        "OauthTokenRefreshWorker" => match int(0) {
            Some(id) => oauth_refresh(state, id).await,
            None => return false,
        },
        "SitemapRefreshWorker" => sitemap_refresh(state).await,
        "SpdxLicenseUpdateWorker" => {
            let (Some(licenses), Some(id)) = (args.first(), args.get(1).and_then(Value::as_i64))
            else {
                return false;
            };
            spdx_update(state, licenses, id).await;
        }
        "Sidekiq::Extensions::DelayedMailer" => match args.first().and_then(Value::as_str) {
            Some(yaml) => delayed_mailer(state, yaml).await,
            None => return false,
        },
        _ => return false,
    }
    true
}

/// Writes the two sidekiq-cron entries the initializer loads, in sidekiq-cron's
/// own Redis representation.
pub async fn install_cron(state: &AppState) {
    let mut conn = match state.redis_jobs.get_multiplexed_async_connection().await {
        Ok(conn) => conn,
        Err(_) => return,
    };
    for (name, class, cron) in [
        (
            "Daily refresh of the sitemap",
            "SitemapRefreshWorker",
            "@daily",
        ),
        (
            "Schedule refresh of expiring tokens",
            "OauthTokenRefreshScheduleWorker",
            "*/5 * * * *",
        ),
    ] {
        let key = format!("cron_job:{name}");
        let existing: bool = conn.exists(&key).await.unwrap_or(false);
        if existing {
            continue;
        }
        let message = json!({ "class": class, "args": [] }).to_string();
        let _: redis::RedisResult<()> = redis::pipe()
            .sadd("cron_jobs", &key)
            .hset_multiple(
                &key,
                &[
                    ("name", name),
                    ("klass", class),
                    ("cron", cron),
                    ("args", "[]"),
                    ("message", message.as_str()),
                    ("status", "enabled"),
                    ("date_as_argument", "false"),
                ],
            )
            .query_async(&mut conn)
            .await;
    }
    // `job_enqueued_key` guard so the first poll does not fire immediately.
    let now = chrono::Utc::now().to_rfc3339();
    for name in [
        "Daily refresh of the sitemap",
        "Schedule refresh of expiring tokens",
    ] {
        let key = format!("cron_job:{name}:enqueued");
        let exists: bool = conn.exists(&key).await.unwrap_or(false);
        if !exists {
            let _: redis::RedisResult<()> = conn.zadd(&key, &now, &now).await;
        }
    }
}

/// The scheduling loop: `@daily` fires at midnight UTC, `*/5` every five
/// minutes, guarded by the `last_enqueue_time` field like sidekiq-cron.
pub async fn schedule_loop(state: AppState) {
    // Sidekiq polls every ~30 seconds; a 5-second tick keeps the minute
    // boundaries tight without spinning.
    let mut last_minute = 0i64;
    loop {
        tokio::time::sleep(Duration::from_secs(5)).await;
        let now = chrono::Utc::now();
        let minute = now.timestamp() / 60;
        if minute == last_minute {
            continue;
        }
        last_minute = minute;
        let slots: [(&str, &str, bool); 2] = [
            (
                "Daily refresh of the sitemap",
                "SitemapRefreshWorker",
                now.format("%H:%M").to_string() == "00:00",
            ),
            (
                "Schedule refresh of expiring tokens",
                "OauthTokenRefreshScheduleWorker",
                now.timestamp() % 300 < 60,
            ),
        ];
        for (name, class, due) in slots {
            if !due {
                continue;
            }
            let Ok(mut conn) = state.redis_jobs.get_multiplexed_async_connection().await else {
                continue;
            };
            let key = format!("cron_job:{name}");
            let last: Option<String> = conn.hget(&key, "last_enqueue_time").await.ok().flatten();
            let stamp = now.format("%Y-%m-%d %H:%M:%S +0000").to_string();
            if last.as_deref() == Some(stamp.as_str()) {
                continue;
            }
            let _: redis::RedisResult<()> = conn.hset(&key, "last_enqueue_time", &stamp).await;
            let _ = perform_async(&state, class, Vec::new()).await;
        }
    }
}

/// Consumes the queue forever.
pub async fn run_forever(state: AppState) {
    install_cron(&state).await;
    tokio::spawn(schedule_loop(state.clone()));
    let Ok(mut conn) = state.redis_jobs.get_multiplexed_async_connection().await else {
        return;
    };
    let queue = format!("queue:{QUEUE}");
    let inprogress = format!("queue:{QUEUE}:inprogress");
    loop {
        let popped = match conn
            .brpoplpush::<_, _, Option<String>>(&queue, &inprogress, 5.0)
            .await
        {
            Ok(value) => value,
            Err(error) => {
                tracing::warn!("brpoplpush: {error}");
                None
            }
        };
        let Some(payload) = popped else {
            if std::env::var("WORKER_ONCE").is_ok() {
                return;
            }
            continue;
        };
        let handled = execute(&state, &payload).await;
        let _: redis::RedisResult<()> = conn.lrem(&inprogress, 1, &payload).await;
        if !handled {
            // Unknown class: put it back on the queue so a Ruby worker that
            // knows it can take it over.
            let _: redis::RedisResult<()> = conn.lpush(format!("queue:{QUEUE}"), &payload).await;
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        let _ = flush_universe(&state).await;
    }
}

/// Re-export so callers can build the same mailer payloads.
pub use crate::jobs::perform_mailer as enqueue_mailer;

/// Convenience for tests: an empty argument list of the `MailerArg` type.
pub fn no_args() -> Vec<MailerArg> {
    Vec::new()
}

/// Builds a `MailerArg` record from an attribute list.
pub fn record(class: &'static str, attributes: Vec<(&'static str, RecordValue)>) -> MailerArg {
    MailerArg::Record(RecordArg { class, attributes })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Once;

    static SEED: Once = Once::new();

    fn state() -> AppState {
        let url = std::env::var("DATABASE_URL").unwrap_or_else(|_| {
            "postgres://supermarket:supermarket@localhost:55432/supermarket_test".into()
        });
        let cfg = crate::config::Config::from_env_with(&|var| match var {
            "DATABASE_URL" => Some(url.clone()),
            "REDIS_URL" => Some("redis://localhost:56379/0".into()),
            "SECRET_KEY_BASE" => Some("test".into()),
            "FQDN" => Some("localhost".into()),
            _ => None,
        })
        .unwrap();
        SEED.call_once(|| {
            std::process::Command::new("harness/bin/seed")
                .env("DATABASE_URL", &url)
                .output()
                .expect("seed runs");
        });
        AppState::for_tests(cfg)
    }

    #[tokio::test]
    async fn sitemap_lists_root_cookbooks_versions_and_users() {
        let state = state();
        let body = sitemap_body(&state).await;
        let base = state.cfg.base_url();
        let root = format!("<loc>{base}</loc><lastmod>");
        let directory = format!("<loc>{base}/cookbooks-directory</loc>");
        let cookbook = format!("<loc>{base}/cookbooks/redis</loc>");
        let version = format!("<loc>{base}/cookbooks/redis/versions/1.10.0</loc>");
        let user = format!("<loc>{base}/users/grace</loc>");
        // alan has no chef account but `user_path` still adds him as `/users/`.
        let accountless = format!("<loc>{base}/users/</loc>");

        assert!(body.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?><urlset "));
        assert!(body.ends_with("</urlset>"));
        let root_at = body.find(&root).expect("root entry");
        let dir_at = body.find(&directory).expect("directory entry");
        let cookbook_at = body.find(&cookbook).expect("cookbook entry");
        assert!(root_at < dir_at && dir_at < cookbook_at);
        assert!(body.contains(&version));
        assert!(body.contains(&user));
        assert!(body.contains(&accountless));
        // cookbooks carry 0.8, everything below them 0.5.
        assert!(body.contains(&format!("{cookbook}<lastmod>")));
        assert!(body
            .split(&cookbook)
            .nth(1)
            .unwrap()
            .contains("<priority>0.8</priority>"));
        assert!(body.contains(&format!("{version}<lastmod>")));
    }

    #[tokio::test]
    async fn spdx_update_stores_the_details_url() {
        let state = state();
        let licenses = json!([
            { "licenseId": "MIT", "detailsUrl": "https://spdx.test/MIT" },
            { "licenseId": "Apache-2.0", "detailsUrl": "https://spdx.test/Apache-2.0" },
        ]);
        spdx_update(&state, &licenses, 1).await;
        let stored: String =
            sqlx::query_scalar("SELECT spdx_license_url FROM cookbook_versions WHERE id = 1")
                .fetch_one(&state.db)
                .await
                .unwrap();
        assert_eq!(stored, "https://spdx.test/Apache-2.0");
    }

    #[tokio::test]
    async fn spdx_update_leaves_unknown_licences_alone() {
        let state = state();
        // A different row than the SPDX success test, so the two can run in
        // parallel against the shared database.
        let _ = sqlx::query("UPDATE cookbook_versions SET spdx_license_url = NULL WHERE id = 2")
            .execute(&state.db)
            .await;
        let licenses = json!([{ "licenseId": "MIT", "detailsUrl": "https://spdx.test/MIT" }]);
        spdx_update(&state, &licenses, 2).await;
        let stored: String = sqlx::query_scalar(
            "SELECT COALESCE(spdx_license_url, '') FROM cookbook_versions WHERE id = 2",
        )
        .fetch_one(&state.db)
        .await
        .unwrap();
        assert_eq!(stored, "");
    }

    #[test]
    fn delayed_mailer_yaml_yields_mailer_method_and_args() {
        let yaml = "\
---
- !ruby/class 'CollaboratorMailer'
- :added_email
- - !ruby/object:Collaborator
    concise_attributes:
    - !ruby/object:ActiveModel::Attribute::FromDatabase
      name: id
      value_before_type_cast: 1
    - !ruby/object:ActiveModel::Attribute::FromDatabase
      name: resourceable_id
      value_before_type_cast: 9
    new_record: false
";
        let parsed = parse_delayed_mailer(yaml).expect("parses");
        assert_eq!(parsed.mailer, "CollaboratorMailer");
        assert_eq!(parsed.method, "added_email");
        assert_eq!(record_id(&parsed.args[0]), Some(1));
    }

    #[test]
    fn delayed_mailer_yaml_keeps_scalar_arguments_separate() {
        let yaml = "\
---
- !ruby/class 'AdoptionMailer'
- :interest_email
- - 7
  - Cookbook
  - 2
";
        let parsed = parse_delayed_mailer(yaml).expect("parses");
        assert_eq!(parsed.args.len(), 3);
        assert_eq!(parsed.args[0], json!("7"));
        assert_eq!(parsed.args[1], json!("Cookbook"));
        assert_eq!(parsed.args[2], json!("2"));
    }
}
