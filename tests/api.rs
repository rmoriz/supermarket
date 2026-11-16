//! Wire-level tests for the public read API, run against the harness database.
//!
//! `harness/bin/seed` is invoked once; assertions are on exact bytes wherever the
//! data is stable, so a key reorder or wording change fails loudly.

use std::sync::Arc;
use std::sync::Once;

use axum::body::Body;
use axum::Router;
use supermarket_rs::{config::Config, routes, state::AppState};
use tower::ServiceExt;

static SEED: Once = Once::new();

fn config() -> Config {
    // Tests default to Redis DB 2: DB 0 belongs to the Rails oracle, and a JSON
    // universe entry written by the tests would poison the oracle's Marshal
    // cache and turn its next `/universe` into a 500.
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
        "FEATURES" => Some("tools,fieri".into()),
        _ => None,
    })
    .unwrap()
}

fn app() -> Router {
    SEED.call_once(|| {
        let output = std::process::Command::new("harness/bin/seed")
            .env("DATABASE_URL", config().database_url)
            .output()
            .expect("run seed");
        assert!(
            output.status.success(),
            "seed failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    });
    let state = AppState::for_tests(config());
    routes::router(state)
}

async fn get(path: &str) -> (u16, String) {
    let response = app()
        .oneshot(
            axum::http::Request::builder()
                .uri(path)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status().as_u16();
    let body = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .unwrap();
    (status, String::from_utf8(body.to_vec()).unwrap())
}

#[tokio::test]
async fn index_body_is_exact_and_orders_deprecated_last() {
    let (status, body) = get("/api/v1/cookbooks?items=2").await;
    assert_eq!(status, 200);
    assert_eq!(
        body,
        r#"{"start":0,"total":4,"items":[{"cookbook_name":"orphan","cookbook_maintainer":"grace","cookbook_description":"Connects through redisio.","cookbook":"http://localhost:3000/api/v1/cookbooks/orphan"},{"cookbook_name":"redis","cookbook_maintainer":"ada","cookbook_description":"Installs redis, newer.","cookbook":"http://localhost:3000/api/v1/cookbooks/redis"}]}"#
    );
}

#[tokio::test]
async fn index_orders_and_filters() {
    let (_, by_name) = get("/api/v1/cookbooks?items=10").await;
    assert!(by_name.contains(r#""cookbook_name":"oldredis""#));
    let last = by_name.rfind("\"cookbook_name\":\"").unwrap();
    assert!(by_name[last..].starts_with(r#""cookbook_name":"oldredis""#));

    let (_, added) = get("/api/v1/cookbooks?order=recently_added&items=1").await;
    assert!(added.contains("\"cookbook_name\":\"orphan\""));

    let (_, owned) = get("/api/v1/cookbooks?user=ada&items=10").await;
    assert!(owned.contains("\"cookbook_name\":\"redis\""));
    assert!(!owned.contains("orphan"));

    let (_, platform) = get("/api/v1/cookbooks?platforms[]=debian&items=10").await;
    assert!(platform.contains("\"cookbook_name\":\"redis\""));
    assert!(!platform.contains("redisio"));
}

#[tokio::test]
async fn index_errors_are_byte_exact() {
    let (status, body) = get("/api/v1/cookbooks?start=-1").await;
    assert_eq!(status, 400);
    assert_eq!(
        body,
        r#"{"error_code":"INVALID_DATA","error_messages":["Start and item parameters must be positive numbers but they are start: -1 and items: not provided."]}"#
    );
}

#[tokio::test]
async fn show_body_is_exact() {
    let (status, body) = get("/api/v1/cookbooks/redis").await;
    assert_eq!(status, 200);
    assert_eq!(
        body,
        concat!(
            r#"{"name":"redis","maintainer":"ada","description":"Installs redis, newer.","category":"Database","#,
            r#""latest_version":"http://localhost:3000/api/v1/cookbooks/redis/versions/1.10.0","#,
            r#""external_url":"https://example.test/redis","source_url":"https://example.test/redis","#,
            r#""issues_url":"https://example.test/redis/issues","average_rating":null,"#,
            r#""created_at":"2020-02-01T00:00:00.000Z","updated_at":"2020-02-02T00:00:00.000Z","#,
            r#""up_for_adoption":null,"deprecated":false,"#,
            r#""versions":["http://localhost:3000/api/v1/cookbooks/redis/versions/1.10.0","http://localhost:3000/api/v1/cookbooks/redis/versions/1.0.0"],"#,
            r#""metrics":{"downloads":{"total":5,"versions":{"1.0.0":2,"1.10.0":2}},"followers":1,"collaborators":1}}"#
        )
    );
}

#[tokio::test]
async fn show_is_case_insensitive_and_deprecated_cookbooks_carry_a_replacement() {
    let (status, body) = get("/api/v1/cookbooks/OLDRedis").await;
    assert_eq!(status, 200);
    assert!(body.contains(
        r#""deprecated":true,"replacement":"http://localhost:3000/api/v1/cookbooks/redis""#
    ));
}

#[tokio::test]
async fn unknown_cookbook_is_the_not_found_envelope() {
    let (status, body) = get("/api/v1/cookbooks/nope").await;
    assert_eq!(status, 404);
    assert_eq!(
        body,
        r#"{"error_messages":["Resource does not exist."],"error_code":"NOT_FOUND"}"#
    );
}

#[tokio::test]
async fn contingent_orders_by_dependent_name() {
    let (status, body) = get("/api/v1/cookbooks/redisio/contingent").await;
    assert_eq!(status, 200);
    assert_eq!(
        body,
        r#"{"contingents":[{"name":"orphan","version":"2.0.0","url":"http://localhost:3000/api/v1/cookbooks/orphan/versions/2.0.0"}]}"#
    );

    let (_, empty) = get("/api/v1/cookbooks/redis/contingent").await;
    assert_eq!(empty, r#"{"contingents":[]}"#);
}

#[tokio::test]
async fn version_show_covers_latest_and_underscore_forms() {
    for version in ["latest", "1_0_0"] {
        let (status, body) = get(&format!("/api/v1/cookbooks/redis/versions/{version}")).await;
        assert_eq!(status, 200, "{version}");
        let expected_version = if version == "latest" {
            "1.10.0"
        } else {
            "1.0.0"
        };
        assert!(body.starts_with(&format!(
            r#"{{"license":"Apache-2.0","tarball_file_size":100,"version":"{expected_version}","published_at":"#
        )));
        if version == "1_0_0" {
            assert!(body.contains(r#""supports":{"debian":">= 0.0.0"}"#));
        }
    }

    let (status, body) = get("/api/v1/cookbooks/redis/versions/9.9.9").await;
    assert_eq!(status, 404);
    assert_eq!(
        body,
        r#"{"error_messages":["Resource does not exist."],"error_code":"NOT_FOUND"}"#
    );
}

#[tokio::test]
async fn download_redirects_and_counts_per_protocol() {
    let before = get("/api/v1/cookbooks/redis").await.1;
    let response = app()
        .oneshot(
            axum::http::Request::builder()
                .uri("/api/v1/cookbooks/redis/versions/1.0.0/download")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 302);
    assert_eq!(
        response.headers().get("location").unwrap(),
        "http://localhost:3000/system/cookbook_versions/tarballs/1/original/redis-1.0.0.tgz?1580515200"
    );
    let after = get("/api/v1/cookbooks/redis").await.1;
    assert_ne!(before, after);
    assert!(after.contains(r#""total":6"#));
}

#[tokio::test]
async fn search_is_ranked_and_total_stays_unfiltered() {
    let (status, body) = get("/api/v1/search?q=redis").await;
    assert_eq!(status, 200);
    assert!(body.starts_with(r#"{"start":0,"total":4,"items":[{"cookbook_name":"redis""#));

    let (_, nothing) = get("/api/v1/search?q=zzzznotfound").await;
    assert_eq!(nothing, r#"{"start":0,"total":4,"items":[]}"#);

    let (_, empty_query) = get("/api/v1/search").await;
    assert_eq!(empty_query, r#"{"start":0,"total":4,"items":[]}"#);
}

#[tokio::test]
async fn user_show_maps_cookbooks_and_tools() {
    let (status, body) = get("/api/v1/users/ada").await;
    assert_eq!(status, 200);
    assert_eq!(
        body,
        concat!(
            r#"{"username":"ada","name":"Ada Lovelace","company":null,"github":[],"twitter":null,"slack":null,"#,
            r#""cookbooks":{"owns":{"redis":"http://localhost:3000/api/v1/cookbooks/redis","redisio":"http://localhost:3000/api/v1/cookbooks/redisio"}},"#,
            r#""tools":{"owns":{"knife-thing":"http://localhost:3000/api/v1/tools/knife-thing"}}}"#
        )
    );
}

#[tokio::test]
async fn github_only_users_are_not_found() {
    let (status, _) = get("/api/v1/users/alan-gh").await;
    assert_eq!(status, 404);
}

#[tokio::test]
async fn metrics_and_health_keys_are_in_upstream_order() {
    let (status, body) = get("/api/v1/metrics").await;
    assert_eq!(status, 200);
    assert!(body.starts_with(r#"{"total_cookbook_downloads":"#));
    assert!(body.contains(r#""total_hits":{"/universe":"#));

    let (status, body) = get("/api/v1/health").await;
    assert_eq!(status, 200);
    assert!(body.starts_with(r#"{"status":"#));
    let order: Vec<&str> = [
        "status",
        "supermarket",
        "sidekiq",
        "postgresql",
        "redis",
        "features",
    ]
    .into_iter()
    .collect();
    let mut cursor = 0;
    for key in order {
        let found = body[cursor..].find(&format!("\"{key}\"")).expect(key);
        cursor += found + key.len();
    }
    assert!(body.contains(r#""features":{"tools":true,"fieri":true"#));

    let (status, alias) = get("/status").await;
    assert_eq!(status, 200);
    assert!(alias.contains(r#""status":"ok""#));
}

#[tokio::test]
async fn tools_list_search_and_show() {
    let (_, index) = get("/api/v1/tools").await;
    assert!(index.contains(r#""tool_name":"knife-thing""#));
    assert!(index.starts_with(r#"{"start":0,"total":1,"#));

    let (_, search) = get("/api/v1/tools-search?q=knife").await;
    assert!(search.contains(r#""total":1"#));
    let (_, bogus_type) = get("/api/v1/tools-search?type=bogus").await;
    assert!(bogus_type.contains("knife-thing"));

    let (status, show) = get("/api/v1/tools/knife-thing").await;
    assert_eq!(status, 200);
    assert!(show.starts_with(r#"{"name":"knife-thing","slug":"knife-thing""#));

    let (status, _) = get("/api/v1/tools/nope").await;
    assert_eq!(status, 404);
}

#[tokio::test]
async fn universe_cache_keys_follow_the_protocol_and_hits_always_increment() {
    let first = app()
        .oneshot(
            axum::http::Request::builder()
                .uri("/api/v1/universe")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = axum::body::to_bytes(first.into_body(), 1 << 20)
        .await
        .unwrap();
    let parsed: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(parsed["redis"]["1.0.0"]["location_type"], "opscode");

    let (_, metrics_before) = get("/api/v1/metrics").await;
    let before: serde_json::Value = serde_json::from_str(&metrics_before).unwrap();
    get("/api/v1/universe").await;
    let (_, metrics_after) = get("/api/v1/metrics").await;
    let after: serde_json::Value = serde_json::from_str(&metrics_after).unwrap();
    assert_eq!(
        after["total_hits"]["/universe"].as_i64().unwrap(),
        before["total_hits"]["/universe"].as_i64().unwrap() + 1
    );

    // The https variant is a different cache key and yields https URLs.
    let secure = app()
        .oneshot(
            axum::http::Request::builder()
                .uri("/api/v1/universe")
                .header("x-forwarded-proto", "https")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = axum::body::to_bytes(secure.into_body(), 1 << 20)
        .await
        .unwrap();
    let parsed: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(parsed["redis"]["1.0.0"]["location_path"]
        .as_str()
        .unwrap()
        .starts_with("https://"));
}

#[tokio::test]
async fn unimplemented_routes_still_answer_501() {
    let (status, _) = get("/api/v1/cookbook-versions/cookstyle_evaluation").await;
    // POST-only route: GET is a 405 from axum, the method matters.
    assert!(
        status == 404 || status == 405 || status == 501,
        "got {status}"
    );

    // Phase 7 landed the quality-metric intake: it now answers its own 400.
    let response = app()
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/api/v1/quality_metrics/cookstyle_evaluation")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 400);

    // The signed web download redirects now that phase 7/8 covered it.
    let response = app()
        .oneshot(
            axum::http::Request::builder()
                .uri("/cookbooks/redis/versions/1.0.0/download")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status().as_u16(), 302);
}

#[allow(dead_code)]
fn state() -> Arc<AppState> {
    Arc::new(AppState::for_tests(config()))
}
