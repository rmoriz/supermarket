use std::sync::Arc;

use serde_json::json;
use supermarket_rs::{config::Config, db, jobs, locales::t, state::AppState};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let _ = dotenvy::from_filename(".env");
    let env_name = std::env::var("RAILS_ENV").unwrap_or_else(|_| "development".into());
    let _ = dotenvy::from_filename(format!(".env.{env_name}"));

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let cfg = Arc::new(Config::from_env()?);
    let pool = db::connect(&cfg).await?;
    let redis = redis::Client::open(cfg.redis_url.clone())?;
    let state = AppState::new(cfg, pool, redis);

    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        // `rake update_spdx_license:run:*`, the only upstream task that matters
        // at runtime; the rest is dev tooling.
        Some("spdx") => spdx(&state, &args[1..]).await,
        _ => {
            tracing::info!("consuming queue:default");
            supermarket_rs::worker::run_forever(state).await;
        }
    }
    Ok(())
}

/// Prints the task result the way the rake task does (`OK:`/`ERROR:`).
async fn spdx(state: &AppState, args: &[String]) {
    let outcome = match args.first().map(String::as_str) {
        Some("all") => all_latest(state).await,
        Some("latest") => match args.get(1) {
            Some(name) => on_latest(state, name).await,
            None => {
                println!("ERROR: Nothing to do without a cookbook name. e.g. spdx latest NAME");
                std::process::exit(1);
            }
        },
        Some("version") => match (args.get(1), args.get(2)) {
            (Some(name), Some(version)) => on_version(state, name, version).await,
            _ => {
                println!(
                    "ERROR: Nothing to do without a cookbook name and version. e.g. spdx version NAME VERSION"
                );
                std::process::exit(1);
            }
        },
        _ => {
            println!("ERROR: Nothing to do. e.g. spdx all | spdx latest NAME | spdx version NAME VERSION");
            std::process::exit(1);
        }
    };
    match outcome {
        Ok(message) => println!("OK: {message}"),
        Err(message) => println!("ERROR: {message}"),
    }
}

/// `UpdateSpdxLicenseUrl.all_latest_cookbook_versions`: the newest version of
/// every cookbook, by `created_at` like upstream.
async fn all_latest(state: &AppState) -> Result<String, String> {
    let ids: Vec<i32> = sqlx::query_scalar(
        "SELECT id FROM cookbook_versions WHERE created_at = (
           SELECT MAX(latest.created_at) FROM cookbook_versions AS latest
            WHERE latest.cookbook_id = cookbook_versions.cookbook_id)",
    )
    .fetch_all(&state.db)
    .await
    .map_err(|error| error.to_string())?;
    enqueue_all(state, &ids).await;
    Ok(t(
        "spdx_license.scheduled.multiple",
        &[("count", &ids.len().to_string())],
    ))
}

async fn on_latest(state: &AppState, name: &str) -> Result<String, String> {
    let cookbook: Option<(i32, String)> =
        sqlx::query_as("SELECT id, name FROM cookbooks WHERE name = $1")
            .bind(name)
            .fetch_optional(&state.db)
            .await
            .map_err(|error| error.to_string())?;
    let Some((cookbook_id, cookbook_name)) = cookbook else {
        return Err(t("cookbook.not_found", &[("name", name)]));
    };
    let version: Option<i32> = sqlx::query_scalar(
        "SELECT id FROM cookbook_versions WHERE cookbook_id = $1
          ORDER BY created_at DESC LIMIT 1",
    )
    .bind(cookbook_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|error| error.to_string())?;
    let Some(version_id) = version else {
        return Err(t(
            "cookbook.version_not_found",
            &[("name", name), ("version", "latest")],
        ));
    };
    enqueue_all(state, &[version_id]).await;
    Ok(t(
        "spdx_license.scheduled.latest",
        &[("name", &cookbook_name)],
    ))
}

async fn on_version(state: &AppState, name: &str, version: &str) -> Result<String, String> {
    let cookbook: Option<(i32, String)> =
        sqlx::query_as("SELECT id, name FROM cookbooks WHERE name = $1")
            .bind(name)
            .fetch_optional(&state.db)
            .await
            .map_err(|error| error.to_string())?;
    let Some((cookbook_id, cookbook_name)) = cookbook else {
        return Err(t("cookbook.not_found", &[("name", name)]));
    };
    let version_id: Option<i32> = sqlx::query_scalar(
        "SELECT id FROM cookbook_versions WHERE cookbook_id = $1 AND version = $2",
    )
    .bind(cookbook_id)
    .bind(version)
    .fetch_optional(&state.db)
    .await
    .map_err(|error| error.to_string())?;
    let Some(version_id) = version_id else {
        return Err(t(
            "cookbook.version_not_found",
            &[("version", version), ("name", name)],
        ));
    };
    enqueue_all(state, &[version_id]).await;
    // Upstream interpolates the *version id* into this message.
    Ok(t(
        "spdx_license.scheduled.single",
        &[
            ("name", &cookbook_name),
            ("version", &version_id.to_string()),
        ],
    ))
}

/// Queues one `SpdxLicenseUpdateWorker` per version with the bundled licence
/// list, the payload the upload path writes too.
async fn enqueue_all(state: &AppState, version_ids: &[i32]) {
    for version_id in version_ids {
        let _ = jobs::perform_async(
            state,
            "SpdxLicenseUpdateWorker",
            vec![
                supermarket_rs::api::v1::cookbook_uploads::spdx_license_json(),
                json!(version_id),
            ],
        )
        .await;
    }
}
