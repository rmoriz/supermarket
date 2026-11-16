//! Strangler proxy for the cutover: one public listener in front of Rails and the
//! Rust port, flipping one route group at a time by configuration.
//!
//! ```text
//! PORT=3002 RAILS_URL=http://127.0.0.1:3001 RUST_URL=http://127.0.0.1:3005 \
//!   STRANGLER_GROUPS=chat,tools cargo run --bin strangler
//! ```
//!
//! Every request is classified into a route group. Groups listed in
//! `STRANGLER_GROUPS` go to Rust, everything else stays on Rails, so a bad flip
//! is reverted by editing the list and restarting — no deploy. Groups listed in
//! `STRANGLER_SHADOW` are served by Rails while the Rust answer is fetched and
//! compared in the background (used for `universe` and search before serving
//! them).

use std::collections::HashSet;

use axum::body::Body;
use axum::extract::State;
use axum::http::{header, HeaderMap, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::Router;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum Group {
    Chat,
    Tools,
    Feeds,
    Reads,
    Uploads,
    Ui,
}

impl Group {
    fn name(self) -> &'static str {
        match self {
            Group::Chat => "chat",
            Group::Tools => "tools",
            Group::Feeds => "feeds",
            Group::Reads => "reads",
            Group::Uploads => "uploads",
            Group::Ui => "ui",
        }
    }

    fn parse(name: &str) -> Option<Self> {
        Some(match name.trim() {
            "chat" => Group::Chat,
            "tools" => Group::Tools,
            "feeds" => Group::Feeds,
            "reads" => Group::Reads,
            "uploads" => Group::Uploads,
            "ui" => Group::Ui,
            _ => return None,
        })
    }

    /// Reverse blast-radius order from the spec: cheap, isolated surfaces flip
    /// first and the signed upload path flips last.
    fn order(self) -> u8 {
        match self {
            Group::Chat => 0,
            Group::Tools => 1,
            Group::Feeds => 2,
            Group::Ui => 3,
            Group::Reads => 4,
            Group::Uploads => 5,
        }
    }
}

/// Classifies a request into the group whose switch owns it.
fn classify(method: &Method, path: &str) -> Group {
    let is_write = !matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS);
    if path == "/chat" || path.starts_with("/chat/") {
        return Group::Chat;
    }
    if path == "/tools-directory"
        || path == "/tools"
        || path.starts_with("/tools/")
        || path == "/tools.atom"
    {
        return Group::Tools;
    }
    if path.starts_with("/robots") || path == "/universe" || path == "/api/v1/universe" {
        return Group::Feeds;
    }
    if path.starts_with("/api/") || path == "/fieri/jobs" || path.starts_with("/fieri/") {
        // Signed writes and the fieri intake carry the most risk.
        if is_write {
            return Group::Uploads;
        }
        return Group::Reads;
    }
    Group::Ui
}

#[derive(Clone)]
struct Config {
    rails: String,
    rust: String,
    flipped: HashSet<Group>,
    shadow: HashSet<Group>,
}

impl Config {
    fn from_env() -> Self {
        let set = |name: &str| -> HashSet<Group> {
            std::env::var(name)
                .unwrap_or_default()
                .split(',')
                .filter_map(Group::parse)
                .collect()
        };
        Config {
            rails: std::env::var("RAILS_URL").unwrap_or_else(|_| "http://127.0.0.1:3001".into()),
            rust: std::env::var("RUST_URL").unwrap_or_else(|_| "http://127.0.0.1:3005".into()),
            flipped: set("STRANGLER_GROUPS"),
            shadow: set("STRANGLER_SHADOW"),
        }
    }
}

const HOP_HEADERS: &[&str] = &[
    "host",
    "connection",
    "content-length",
    "transfer-encoding",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailer",
    "upgrade",
];

fn backend_for(cfg: &Config, group: Group) -> &str {
    if cfg.flipped.contains(&group) {
        &cfg.rust
    } else {
        &cfg.rails
    }
}

use axum::body::Bytes;

async fn forward(
    client: &reqwest::Client,
    base: &str,
    method: Method,
    uri: &Uri,
    headers: &HeaderMap,
    body: Bytes,
) -> Result<Response, reqwest::Error> {
    let target = format!(
        "{}{}",
        base.trim_end_matches('/'),
        uri.path_and_query().map_or("/", |value| value.as_str())
    );
    let mut request = client.request(method, target).body(body);
    for (name, value) in headers {
        if HOP_HEADERS.contains(&name.as_str()) {
            continue;
        }
        request = request.header(name, value);
    }
    let response = request.send().await?;
    let status = response.status();
    let headers = response.headers().clone();
    let body = response.bytes().await?;
    let mut out = Response::new(Body::from(body));
    *out.status_mut() = status;
    for (name, value) in &headers {
        if HOP_HEADERS.contains(&name.as_str()) {
            continue;
        }
        out.headers_mut().append(name.clone(), value.clone());
    }
    Ok(out)
}

fn proxy_client() -> reqwest::Client {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap_or_default()
}

async fn handle(
    State(cfg): State<Config>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let group = classify(&method, uri.path());
    // The proxy mirrors status codes: a backend 3xx must reach the client
    // instead of being followed here.
    let client = proxy_client();
    let shadowed = cfg.shadow.contains(&group);
    let target = if shadowed {
        &cfg.rails
    } else {
        backend_for(&cfg, group)
    };
    let served = match forward(
        &client,
        target,
        method.clone(),
        &uri,
        &headers,
        body.clone(),
    )
    .await
    {
        Ok(response) => response,
        Err(error) => {
            tracing::error!(group = group.name(), path = uri.path(), %error, "upstream failed");
            return (
                StatusCode::BAD_GATEWAY,
                [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
                "bad gateway\n",
            )
                .into_response();
        }
    };
    if shadowed {
        let hit = if cfg.flipped.contains(&group) {
            cfg.rails.clone()
        } else {
            cfg.rust.clone()
        };
        let path = uri.path().to_owned();
        let status = served.status();
        let content_type = served.headers().get(header::CONTENT_TYPE).cloned();
        let etag = served.headers().get(header::ETAG).cloned();
        tokio::spawn(async move {
            let mirror_client = proxy_client();
            match forward(&mirror_client, &hit, method, &uri, &headers, body).await {
                Ok(mirror) => {
                    if mirror.status() != status {
                        tracing::warn!(
                            group = group.name(),
                            path = %path,
                            served = %status,
                            mirror = %mirror.status(),
                            "shadow status mismatch"
                        );
                        return;
                    }
                    if mirror.headers().get(header::CONTENT_TYPE).cloned() != content_type {
                        tracing::warn!(group = group.name(), path = %path, "shadow content-type mismatch");
                        return;
                    }
                    if mirror.headers().get(header::ETAG).cloned() != etag {
                        tracing::warn!(group = group.name(), path = %path, "shadow etag mismatch");
                    }
                }
                Err(error) => {
                    tracing::warn!(group = group.name(), path = %path, %error, "shadow mirror failed")
                }
            }
        });
    }
    served
}

async fn health() -> Response {
    (StatusCode::OK, "ok\n").into_response()
}

fn app(cfg: Config) -> Router {
    Router::new()
        .route("/_strangler/health", axum::routing::get(health))
        .fallback(handle)
        .with_state(cfg)
}

#[tokio::main]
async fn main() {
    let _ = dotenvy::dotenv();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "strangler=info".into()),
        )
        .init();
    let cfg = Config::from_env();
    let mut flipped: Vec<&str> = cfg.flipped.iter().map(|g| g.name()).collect();
    flipped.sort_by_key(|name| Group::parse(name).map(Group::order));
    tracing::info!(rails = %cfg.rails, rust = %cfg.rust, flipped = ?flipped, "strangler up");
    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(3002);
    let listener = tokio::net::TcpListener::bind(("0.0.0.0", port))
        .await
        .expect("bind");
    axum::serve(listener, app(cfg)).await.expect("serve");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(flipped: &[Group], shadow: &[Group]) -> Config {
        Config {
            rails: "http://rails.test".into(),
            rust: "http://rust.test".into(),
            flipped: flipped.iter().copied().collect(),
            shadow: shadow.iter().copied().collect(),
        }
    }

    #[test]
    fn chat_flips_first_and_uploads_last() {
        assert!(Group::Chat.order() < Group::Ui.order());
        assert!(Group::Ui.order() < Group::Uploads.order());
    }

    #[test]
    fn groups_cover_the_surfaces_the_spec_lists() {
        assert_eq!(classify(&Method::GET, "/chat/chef/2014-09-24"), Group::Chat);
        assert_eq!(classify(&Method::GET, "/tools-directory"), Group::Tools);
        assert_eq!(classify(&Method::GET, "/tools.atom"), Group::Tools);
        assert_eq!(classify(&Method::GET, "/robots.txt"), Group::Feeds);
        assert_eq!(classify(&Method::GET, "/universe"), Group::Feeds);
        assert_eq!(
            classify(&Method::GET, "/users/ada/followed_cookbook_activity"),
            Group::Ui
        );
        assert_eq!(classify(&Method::GET, "/api/v1/cookbooks"), Group::Reads);
        assert_eq!(classify(&Method::POST, "/api/v1/cookbooks"), Group::Uploads);
        assert_eq!(
            classify(&Method::DELETE, "/api/v1/cookbooks/redis"),
            Group::Uploads
        );
        assert_eq!(classify(&Method::POST, "/fieri/jobs"), Group::Uploads);
        assert_eq!(classify(&Method::GET, "/cookbooks/redis"), Group::Ui);
    }

    #[test]
    fn an_unflipped_group_stays_on_rails() {
        let cfg = cfg(&[Group::Chat], &[]);
        assert_eq!(backend_for(&cfg, Group::Chat), "http://rust.test");
        assert_eq!(backend_for(&cfg, Group::Ui), "http://rails.test");
    }

    #[test]
    fn flavours_parse_from_the_switch_list() {
        let cfg = cfg(&[Group::Tools, Group::Feeds], &[Group::Reads]);
        assert!(cfg.flipped.contains(&Group::Tools));
        assert!(cfg.shadow.contains(&Group::Reads));
        assert!(!cfg.flipped.contains(&Group::Ui));
    }
}
