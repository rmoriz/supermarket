//! Renders every mailer for the harness and prints a JSON map of the bodies.
//!
//! `harness/bin/mailers` runs the same scenarios inside the oracle and diffs
//! the two maps; nothing is delivered.

use std::sync::Arc;

use sqlx::PgPool;
use supermarket_rs::{config::Config, db, mail};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _ = dotenvy::from_filename(".env");
    let env_name = std::env::var("RAILS_ENV").unwrap_or_else(|_| "test".into());
    let _ = dotenvy::from_filename(format!(".env.{env_name}"));

    let cfg = Arc::new(Config::from_env()?);
    let pool = db::connect(&cfg).await?;
    let base = cfg.base_url();
    let mut out = serde_json::Map::new();

    // Scenarios mirror `harness/bin/mailer_fixtures.rb`.
    let version: (i32, String, String) = sqlx::query_as(
        "SELECT v.id, v.version, c.name FROM cookbook_versions v
           JOIN cookbooks c ON c.id = v.cookbook_id
          WHERE c.name = 'redis' ORDER BY v.created_at DESC LIMIT 1",
    )
    .fetch_one(&pool)
    .await?;
    let ada: String = sqlx::query_scalar("SELECT email FROM users WHERE id = 1")
        .fetch_one(&pool)
        .await?;
    let unsubscribe: String = sqlx::query_scalar(
        "SELECT ep.token FROM email_preferences ep
           JOIN system_emails se ON se.id = ep.system_email_id
          WHERE ep.user_id = 1 AND se.name = 'New cookbook version'",
    )
    .fetch_one(&pool)
    .await?;

    let message = mail::follower_notification(
        &cfg,
        &ada,
        &version.2,
        &version.1,
        &format!("{base}/cookbooks/{}/versions/{}", version.2, version.1),
        None,
        &format!(
            "{base}/cookbooks/{}/versions/{}#changelog",
            version.2, version.1
        ),
        &format!("{base}/unsubscribe/{unsubscribe}"),
    );
    out.insert("follower_notification_email".into(), render(&message));

    let deleted_token: String = sqlx::query_scalar(
        "SELECT ep.token FROM email_preferences ep
           JOIN system_emails se ON se.id = ep.system_email_id
          WHERE ep.user_id = 1 AND se.name = 'Cookbook deleted'",
    )
    .fetch_one(&pool)
    .await?;
    let message = mail::cookbook_deleted(
        &cfg,
        &ada,
        "redis",
        &format!("{base}/unsubscribe/{deleted_token}"),
    );
    out.insert("cookbook_deleted_email".into(), render(&message));

    let deprecated_token: String = sqlx::query_scalar(
        "SELECT ep.token FROM email_preferences ep
           JOIN system_emails se ON se.id = ep.system_email_id
          WHERE ep.user_id = 1 AND se.name = 'Cookbook deprecated'",
    )
    .fetch_one(&pool)
    .await?;
    let reason = default_reason(&cfg, &pool, "redis").await?;
    let message = mail::cookbook_deprecated(
        &cfg,
        &ada,
        "redis",
        &format!("{base}/cookbooks/redis"),
        &reason,
        None,
        &format!("{base}/unsubscribe/{deprecated_token}"),
    );
    out.insert("cookbook_deprecated_email".into(), render(&message));

    let reason = default_reason(&cfg, &pool, "oldredis").await?;
    let message = mail::cookbook_deprecated(
        &cfg,
        &ada,
        "oldredis",
        &format!("{base}/cookbooks/oldredis"),
        &reason,
        Some(("redis", &format!("{base}/cookbooks/redis"))),
        &format!("{base}/unsubscribe/{deprecated_token}"),
    );
    out.insert("deprecated_with_replacement".into(), render(&message));

    let transfer: (String, String, String, String) = sqlx::query_as(
        "SELECT u.first_name, u.last_name, c.name, r.email
           FROM ownership_transfer_requests t
           JOIN users u ON u.id = t.sender_id
           JOIN cookbooks c ON c.id = t.cookbook_id
           JOIN users r ON r.id = t.recipient_id
          ORDER BY t.id LIMIT 1",
    )
    .fetch_one(&pool)
    .await?;
    let token: String = sqlx::query_scalar("SELECT token FROM ownership_transfer_requests LIMIT 1")
        .fetch_one(&pool)
        .await?;
    let message = mail::transfer_ownership(
        &cfg,
        &transfer.3,
        &format!("{} {}", transfer.0, transfer.1),
        &transfer.2,
        &format!("{base}/ownership_transfer/{token}/accept"),
        &format!("{base}/ownership_transfer/{token}/decline"),
    );
    out.insert("transfer_ownership_email".into(), render(&message));

    let collaborator: (String, String) = sqlx::query_as(
        "SELECT u.email, c.resourceable_type FROM collaborators c
           JOIN users u ON u.id = c.user_id LIMIT 1",
    )
    .fetch_one(&pool)
    .await?;
    let grace: String = sqlx::query_scalar("SELECT email FROM users WHERE id = 2")
        .fetch_one(&pool)
        .await?;
    let message = mail::collaborator_added(
        &cfg,
        &collaborator.0,
        "redis",
        &collaborator.1,
        &format!("{base}/cookbooks/redis"),
    );
    out.insert("added_email".into(), render(&message));

    let message = mail::adoption_interest(
        &cfg,
        &ada,
        "cookbook",
        "redis",
        &format!("{base}/cookbooks/redis"),
        "grace",
        &grace,
    );
    out.insert("interest_email".into(), render(&message));

    let message = mail::adoption_follower(
        &cfg,
        &grace,
        "cookbook",
        "redis",
        &format!("{base}/cookbooks/redis"),
    );
    out.insert("follower_email".into(), render(&message));

    println!("{}", serde_json::to_string_pretty(&out)?);
    Ok(())
}

fn render(message: &mail::Message) -> serde_json::Value {
    serde_json::json!({
        "to": message.to,
        "subject": message.subject,
        "text": message.text,
        "html": message.html,
    })
}

async fn default_reason(cfg: &Config, pool: &PgPool, name: &str) -> Result<String, sqlx::Error> {
    let stored: String = sqlx::query_scalar(
        "SELECT COALESCE(deprecation_reason, '') FROM cookbooks WHERE name = $1",
    )
    .bind(name)
    .fetch_one(pool)
    .await?;
    let _ = cfg;
    Ok(if stored.trim().is_empty() {
        format!(
            "The {name} cookbook has been deprecated and is no longer being maintained by its authors.  Use of the {name} cookbook is no longer recommended."
        )
    } else {
        stored
    })
}
