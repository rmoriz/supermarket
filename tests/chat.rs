//! The `chat` switch: the redirect routes answer the standard 404 unless
//! `FEATURES` names `chat`, and stay byte-identical when it does.
//!
//! The suite seeds nothing of its own: `tests/api.rs` truncates and reseeds the
//! shared database in another process, and a second seeder would race it.

use axum::body::Body;
use axum::http::HeaderMap;
use axum::Router;
use supermarket_rs::{config::Config, routes, state::AppState};
use tower::ServiceExt;

fn config(features: &str) -> Config {
    Config::from_env_with(&|var| match var {
        "DATABASE_URL" => Some(std::env::var("DATABASE_URL").unwrap_or_else(|_| {
            "postgres://supermarket:supermarket@localhost:55432/supermarket_test".into()
        })),
        "REDIS_URL" => {
            Some(std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://localhost:56379/2".into()))
        }
        "SECRET_KEY_BASE" => Some("test-secret".into()),
        "FQDN" => Some("localhost".into()),
        "PORT" => Some("3000".into()),
        "FEATURES" => Some(features.into()),
        _ => None,
    })
    .unwrap()
}

fn app(features: &str) -> Router {
    routes::router(AppState::for_tests(config(features)))
}

async fn get(features: &str, path: &str) -> (u16, HeaderMap, String) {
    let response = app(features)
        .oneshot(
            axum::http::Request::builder()
                .uri(path)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status().as_u16();
    let headers = response.headers().clone();
    let body = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .unwrap();
    (status, headers, String::from_utf8(body.to_vec()).unwrap())
}

fn location(headers: &HeaderMap) -> &str {
    headers
        .get(axum::http::header::LOCATION)
        .unwrap()
        .to_str()
        .unwrap()
}

#[tokio::test]
async fn default_off_answers_404_everywhere() {
    for path in ["/chat", "/chat/chef", "/chat/chef/2013-08-07"] {
        let (status, headers, body) = get("tools,fieri", path).await;
        assert_eq!(status, 404, "{path}");
        assert!(
            headers.get(axum::http::header::LOCATION).is_none(),
            "{path}"
        );
        assert!(
            body.contains("The page you were looking for does not exist."),
            "{path}"
        );
    }
}

#[tokio::test]
async fn enabled_restores_the_exact_redirects() {
    let (status, headers, _) = get("tools,fieri,chat", "/chat").await;
    assert_eq!(status, 302);
    assert_eq!(location(&headers), "https://botbot.me/freenode/chef/");

    let (status, headers, _) = get("tools,fieri,chat", "/chat/chef").await;
    assert_eq!(status, 302);
    assert_eq!(location(&headers), "https://botbot.me/freenode/chef");

    let (status, headers, _) = get("tools,fieri,chat", "/chat/chef/2013-08-07").await;
    assert_eq!(status, 302);
    assert_eq!(
        location(&headers),
        "https://github.com/chef/irc_log_archives"
    );

    let (status, headers, _) = get("tools,fieri,chat", "/chat/chef/2014-09-24").await;
    assert_eq!(status, 302);
    assert_eq!(
        location(&headers),
        "https://botbot.me/freenode/chef/2014-09-24"
    );

    let (status, headers, body) = get("tools,fieri,chat", "/chat/chef/nope").await;
    assert_eq!(status, 404);
    assert!(headers.get(axum::http::header::LOCATION).is_none());
    assert!(body.contains("The page you were looking for does not exist."));
}

#[tokio::test]
async fn health_reports_chat_last_and_false_by_default() {
    let (status, _, body) = get("tools,fieri", "/api/v1/health").await;
    assert_eq!(status, 200);
    assert!(body.ends_with(r#""chat":false}}"#), "{body}");

    let (_, _, body) = get("tools,fieri,chat", "/api/v1/health").await;
    assert!(body.ends_with(r#""chat":true}}"#), "{body}");
}
