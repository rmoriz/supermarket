use std::sync::Arc;

use supermarket_rs::{config::Config, db, middleware, routes, state::AppState};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // `.env` then `.env.<RAILS_ENV|RACK_ENV>` — same load order as the Rails app.
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
    supermarket_rs::web::set_gravatar(cfg.feature_active("gravatar"));
    let state = AppState::new(cfg.clone(), pool, redis);

    let bind = match &cfg.port {
        Some(port) if !port.is_empty() => format!("0.0.0.0:{port}"),
        _ => "0.0.0.0:3000".to_owned(),
    };

    let app = middleware::stack(routes::router(state));
    let listener = tokio::net::TcpListener::bind(&bind).await?;
    tracing::info!(%bind, "listening");
    axum::serve(listener, app.into_make_service()).await?;
    Ok(())
}
