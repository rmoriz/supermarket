//! Response middleware reproducing the Rails default headers, security policy,
//! caching headers, conditional GET, request ids, compression, a 60 s timeout
//! and a 25 MB body limit.

use axum::extract::{DefaultBodyLimit, State};
use axum::http::{header, HeaderName, HeaderValue, Method, Request, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Router;
use base64::Engine;
use cadence::StatsdClient;
use tower_http::compression::CompressionLayer;
use tower_http::limit::RequestBodyLimitLayer;
use tower_http::request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer};
use tower_http::set_header::SetResponseHeaderLayer;
use tower_http::timeout::TimeoutLayer;

use crate::session;

/// 25 MB, matching the `client_max_body_size` in front of the Rails app.
pub const BODY_LIMIT: usize = 25 * 1024 * 1024;
pub const TIMEOUT_SECONDS: u64 = 60;

/// Headers Rails sets on every response (`action_dispatch.default_headers`).
pub const DEFAULT_HEADERS: &[(&str, &str)] = &[
    ("x-frame-options", "SAMEORIGIN"),
    ("x-xss-protection", "0"),
    ("x-content-type-options", "nosniff"),
    ("x-permitted-cross-domain-policies", "none"),
    ("referrer-policy", "strict-origin-when-cross-origin"),
];

/// `config/initializers/content_security_policy.rb`, with `%NONCE%` for the
/// per-request script nonce (Rails appends the same directive).
pub const CONTENT_SECURITY_POLICY: &str = concat!(
    "default-src 'self' https:; font-src 'self' https: data: http://fonts.gstatic.com; ",
    "img-src 'self' https: data: www.googletagmanager.com; connect-src 'self' https: ",
    "wss://ws.hotjar.com; script-src 'self' blob: https: https://www.googletagmanager.com ",
    "https://www.google-analytics.com http://cdn.segment.com 'nonce-%NONCE%'; ",
    "object-src 'none'; style-src 'self' 'unsafe-inline' https: http://fonts.googleapis.com"
);

/// Rails' ETag digest: `W/"<md5 of the body>"`.
pub fn etag_for(body: &[u8]) -> String {
    // `md5::compute` returns a `Digest` whose Debug is the lowercase hex digest.
    format!("W/\"{:x}\"", md5::compute(body))
}

fn nonce() -> String {
    let bytes: [u8; 16] = rand::random();
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// Response marker for a route-level redirect (`redirect(...)` in `routes.rb`).
/// Those never enter a controller, so they lack the default security headers.
#[derive(Debug, Clone, Copy)]
pub struct RouterRedirect;

/// Response marker for an OmniAuth request-phase redirect (`POST /auth/:provider`).
/// OmniAuth's middleware runs before the controller stack, so the response has
/// no default security headers and no content type, but the outer stack still
/// adds the CSP and the session cookie.
#[derive(Clone, Copy)]
pub struct OmniAuthRedirect;

/// Response marker for a page Rails renders from `config.exceptions_app`
/// (`InvalidAuthenticityToken`, an unhandled 500). Those responses are produced
/// outside the normal middleware stack, so they carry neither CSP nor
/// `cache-control` and leave the session cookie alone.
#[derive(Clone, Copy)]
pub struct ExceptionResponse;

tokio::task_local! {
    /// Set as soon as a handler reads the session. Rails' cookie store only
    /// writes the session back when the request touched it (`warden` loading a
    /// user, flash, CSRF), never for a plain read-through.
    static SESSION_TOUCHED: std::sync::Arc<std::sync::atomic::AtomicBool>;
}

/// Records that this request read the session, so the cookie is written back.
pub fn mark_session_touched() {
    let _ = SESSION_TOUCHED.try_with(|flag| flag.store(true, std::sync::atomic::Ordering::Relaxed));
}

/// Rails marks 304s, errors and empty 200s `no-cache`; other bodies are private
/// and revalidated.
fn cache_control(status: StatusCode, empty_body: bool) -> &'static str {
    if status == StatusCode::OK && empty_body || !status.is_success() {
        "no-cache"
    } else {
        "max-age=0, private, must-revalidate"
    }
}

/// Appends `; charset=utf-8` to the textual content type, as Rails does when it
/// actually renders a body (an empty `head 200` keeps the bare type).
fn add_charset(headers: &mut axum::http::HeaderMap) {
    if let Some(value) = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
        .and_then(|value| charset_for(&value))
    {
        if let Ok(value) = HeaderValue::from_str(&value) {
            headers.insert(header::CONTENT_TYPE, value);
        }
    }
}

/// Rails adds `; charset=utf-8` to textual bodies.
fn charset_for(content_type: &str) -> Option<String> {
    if content_type.contains("charset") {
        return None;
    }
    if content_type.starts_with("text/html")
        || content_type.starts_with("text/plain")
        || content_type.starts_with("application/json")
    {
        Some(format!("{content_type}; charset=utf-8"))
    } else {
        None
    }
}

/// Rails' `Rack::ETag` sets an ETag on cacheable success bodies (including
/// 201s and deletes), and answers 304 for a matching `If-None-Match` on GET.
/// Bodies here are small, so they are buffered to hash them.
pub async fn parity_headers(req: Request<axum::body::Body>, next: Next) -> Response {
    let is_get = req.method() == Method::GET;
    let if_none_match = req
        .headers()
        .get(header::IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);

    let mut response = next.run(req).await;
    let forgery = response.extensions().get::<ExceptionResponse>().is_some();
    let omniauth = response.extensions().get::<OmniAuthRedirect>().is_some();
    let status = response.status();
    let headers = response.headers_mut();

    if status.is_redirection() && !omniauth {
        headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("text/html; charset=utf-8"),
        );
    }

    if status.is_success() && status != StatusCode::NO_CONTENT {
        let (parts, body) = response.into_parts();
        let bytes = axum::body::to_bytes(body, BODY_LIMIT)
            .await
            .unwrap_or_default();
        let tag = etag_for(&bytes);
        let empty = bytes.is_empty();
        let mut response = Response::from_parts(parts, axum::body::Body::from(bytes));
        if !empty {
            add_charset(response.headers_mut());
        }
        if !forgery {
            if let Ok(value) = HeaderValue::from_str(cache_control(status, empty)) {
                response.headers_mut().insert(header::CACHE_CONTROL, value);
            }
        }
        if empty {
            return response;
        }
        if let Ok(value) = HeaderValue::from_str(&tag) {
            response.headers_mut().insert(header::ETAG, value);
        }
        if is_get && if_none_match.as_deref() == Some(tag.as_str()) {
            let mut not_modified = StatusCode::NOT_MODIFIED.into_response();
            let target = not_modified.headers_mut();
            for name in [header::ETAG, header::CACHE_CONTROL] {
                if let Some(value) = response.headers().get(&name) {
                    target.insert(name, value.clone());
                }
            }
            return not_modified;
        }
        response
    } else {
        add_charset(response.headers_mut());
        if !forgery {
            if let Ok(value) = HeaderValue::from_str(cache_control(status, false)) {
                response.headers_mut().insert(header::CACHE_CONTROL, value);
            }
        }
        response
    }
}

/// Ensures every visitor carries a session (Rails' cookie store creates one on
/// first request so CSRF protection always has a token), injecting the cookie
/// into the request and setting it on the response.
pub async fn session_cookie(mut req: Request<axum::body::Body>, next: Next) -> Response {
    let cookie_header = req
        .headers()
        .get(header::COOKIE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_owned();
    let supplied = cookie_header.split(';').find_map(|piece| {
        let (name, value) = piece.trim().split_once('=')?;
        (name == session::COOKIE_NAME).then(|| value.to_owned())
    });
    let touched = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let was_touched =
        |flag: &std::sync::atomic::AtomicBool| flag.load(std::sync::atomic::Ordering::Relaxed);

    // A visitor who already has a session gets it written back when the
    // request read it (`warden` reloads the user, flash and CSRF touch the
    // store). Handlers that changed the session set their own cookie.
    // A cookie that no longer decrypts (rotated secret, truncated value) is
    // discarded exactly like a missing one.
    let supplied = supplied.filter(|value| {
        let decoded: String = form_urlencoded::parse(format!("x={value}").as_bytes())
            .map(|(_, value)| value.into_owned())
            .next()
            .unwrap_or_else(|| value.clone());
        let secret = std::env::var("SECRET_KEY_BASE").unwrap_or_default();
        session::decode(&decoded, &secret).is_ok()
    });
    if let Some(value) = supplied {
        let mut response = SESSION_TOUCHED.scope(touched.clone(), next.run(req)).await;
        let forgery = response.extensions().get::<ExceptionResponse>().is_some();
        if session_written(&response)
            && was_touched(&touched)
            && !forgery
            && !response.headers().contains_key(header::SET_COOKIE)
        {
            if let Ok(header_value) = HeaderValue::from_str(&format!(
                "{}={value}; path=/; HttpOnly; SameSite=Lax",
                session::COOKIE_NAME
            )) {
                response
                    .headers_mut()
                    .append(header::SET_COOKIE, header_value);
            }
        }
        return response;
    }

    let session = session::Session::new(random_token());
    let secret = std::env::var("SECRET_KEY_BASE").unwrap_or_default();
    let raw = session::encode(&session, &secret);
    let encoded: String = form_urlencoded::byte_serialize(raw.as_bytes()).collect();
    // Drop any stale value for our own cookie so `session_from` reads the new
    // one; Rails' jar rewrites the value in place.
    let remaining: Vec<&str> = cookie_header
        .split(';')
        .map(str::trim)
        .filter(|piece| {
            !piece.is_empty() && !piece.starts_with(&format!("{}=", session::COOKIE_NAME))
        })
        .collect();
    let merged = if remaining.is_empty() {
        format!("{}={encoded}", session::COOKIE_NAME)
    } else {
        format!(
            "{}; {}={encoded}",
            remaining.join("; "),
            session::COOKIE_NAME
        )
    };
    if let Ok(value) = HeaderValue::from_str(&merged) {
        req.headers_mut().insert(header::COOKIE, value);
    }
    let mut response = SESSION_TOUCHED.scope(touched.clone(), next.run(req)).await;
    // Rails' cookie store is lazy: a session is only written when the request
    // touched it. HTML pages always read the CSRF token, plain bodies (robots,
    // feeds) and redirects do not, and the exception page renders outside the
    // layout.
    let forgery = response.extensions().get::<ExceptionResponse>().is_some();
    if session_written(&response)
        && was_touched(&touched)
        && !forgery
        && !response.status().is_redirection()
    {
        if let Ok(value) = HeaderValue::from_str(&format!(
            "{}={encoded}; path=/; HttpOnly; SameSite=Lax",
            session::COOKIE_NAME
        )) {
            response.headers_mut().append(header::SET_COOKIE, value);
        }
    }
    response
}

/// Rails' cookie store writes the session back whenever the request touched it,
/// except for atom feeds: the feed renderers never read the store.
fn session_written(response: &Response) -> bool {
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    !content_type.starts_with("application/atom+xml")
}

fn random_token() -> String {
    let bytes: [u8; 24] = rand::random();
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// Paths exempt from forgery protection: the JSON API and the OAuth callbacks
/// `skip_before_action :verify_authenticity_token`, plus `collaborators#destroy`.
fn forgery_exempt(method: &Method, path: &str) -> bool {
    // `SessionsController` skips the check only for the callback action; the
    // request phase (`POST /auth/:provider`) still needs a token.
    if path.starts_with("/api/") || path.ends_with("/callback") || path == "/fieri/jobs" {
        return true;
    }
    // `/logout` and `/signout` are router-level redirects: no controller runs,
    // so no callback verifies the token.
    if *method == Method::DELETE && (path == "/logout" || path == "/signout") {
        return true;
    }
    // `PATCH /profile/change_password` has no action: dispatch raises
    // `ActionNotFound` before the CSRF callback ever runs.
    if *method == Method::PATCH && path == "/profile/change_password" {
        return true;
    }
    if *method == Method::DELETE && path.starts_with("/collaborators/") {
        let rest = path.trim_start_matches("/collaborators/");
        return !rest.is_empty() && !rest.contains('/');
    }
    false
}

/// `protect_from_forgery with: :exception`: unsafe verbs need a valid masked
/// authenticity token from the session, otherwise a 422 exception page.
pub async fn verify_forgery(
    State(state): State<crate::state::AppState>,
    req: Request<axum::body::Body>,
    next: Next,
) -> Response {
    let method = req.method().clone();
    let path = req.uri().path().to_owned();
    if method == Method::GET || method == Method::HEAD || forgery_exempt(&method, &path) {
        return next.run(req).await;
    }

    let headers = req.headers().clone();
    let (parts, body) = req.into_parts();
    let bytes = axum::body::to_bytes(body, BODY_LIMIT)
        .await
        .unwrap_or_default();
    let token = headers
        .get(header::HeaderName::from_static("x-csrf-token"))
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
        .or_else(|| {
            form_urlencoded::parse(&bytes)
                .find(|(name, _)| name == "authenticity_token")
                .map(|(_, value)| value.into_owned())
        });
    let session = crate::web::session_from(&state, &headers);
    let valid = match (&session, &token) {
        (Some(session), Some(token)) => crate::session::unmask_token(token)
            .map(|raw| raw == crate::session::real_csrf(session))
            .unwrap_or(false),
        _ => false,
    };
    if !valid {
        let mut response = crate::web::pages::error_page_as(
            &state,
            &headers,
            StatusCode::UNPROCESSABLE_ENTITY,
            false,
            "exceptions",
        )
        .await;
        response.extensions_mut().insert(ExceptionResponse);
        return response;
    }
    let request = Request::from_parts(parts, axum::body::Body::from(bytes));
    next.run(request).await
}

pub fn stack<S>(router: Router<S>) -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    let mut router = router
        .layer(axum::middleware::from_fn(security_policy))
        .layer(axum::middleware::from_fn(parity_headers))
        .layer(PropagateRequestIdLayer::x_request_id())
        .layer(SetRequestIdLayer::x_request_id(MakeRequestUuid))
        .layer(TimeoutLayer::with_status_code(
            StatusCode::SERVICE_UNAVAILABLE,
            std::time::Duration::from_secs(TIMEOUT_SECONDS),
        ))
        .layer(DefaultBodyLimit::max(BODY_LIMIT))
        .layer(RequestBodyLimitLayer::new(BODY_LIMIT))
        .layer(CompressionLayer::new())
        .layer(axum::middleware::from_fn(session_cookie));

    for (name, value) in DEFAULT_HEADERS {
        router = router.layer(SetResponseHeaderLayer::overriding(
            HeaderName::from_static(name),
            HeaderValue::from_static(value),
        ));
    }
    router.layer(axum::middleware::from_fn(router_redirect_headers))
}

/// Statsd client for `STATSD_URL` (`host:port`).
pub fn statsd_client(host_port: &str) -> StatsdClient {
    use cadence::UdpMetricSink;
    use std::net::UdpSocket;

    let host_port = if host_port.contains(':') {
        host_port.to_owned()
    } else {
        format!("{host_port}:8125")
    };
    let Ok(socket) = UdpSocket::bind("0.0.0.0:0") else {
        return StatsdClient::from_sink("supermarket", cadence::NopMetricSink);
    };
    match UdpMetricSink::from(host_port.as_str(), socket) {
        Ok(sink) => StatsdClient::from_sink("supermarket", sink),
        Err(_) => StatsdClient::from_sink("supermarket", cadence::NopMetricSink),
    }
}

/// CSP needs a fresh nonce per response, so it is set in a layer after the body
/// is final (the nonce is only used by HTML script tags, which phase 3 renders).
/// Route-level redirects are served by the router, outside `ActionDispatch`'s
/// controller stack: no default headers, and `UTF-8` spelled like Rack does.
pub async fn router_redirect_headers(req: Request<axum::body::Body>, next: Next) -> Response {
    let mut response = next.run(req).await;
    let omniauth = response.extensions().get::<OmniAuthRedirect>().is_some();
    if response.extensions().get::<RouterRedirect>().is_some() || omniauth {
        for (name, _) in DEFAULT_HEADERS {
            response.headers_mut().remove(*name);
        }
    }
    if response.extensions().get::<RouterRedirect>().is_some() {
        response.headers_mut().insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("text/html; charset=UTF-8"),
        );
    }
    response
}

pub async fn security_policy(req: Request<axum::body::Body>, next: Next) -> Response {
    let mut response = next.run(req).await;
    if response.extensions().get::<ExceptionResponse>().is_some() {
        return response;
    }
    let policy = CONTENT_SECURITY_POLICY.replace("%NONCE%", &nonce());
    if let Ok(value) = HeaderValue::from_str(&policy) {
        response
            .headers_mut()
            .insert(header::CONTENT_SECURITY_POLICY, value);
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::routing::get;
    use tower::ServiceExt;

    fn app() -> Router {
        stack(
            Router::new()
                .route(
                    "/",
                    get(|body: axum::body::Bytes| async move { body.len().to_string() }),
                )
                .route(
                    "/json",
                    get(|| async { ([(header::CONTENT_TYPE, "application/json")], r#"{"a":1}"#) }),
                ),
        )
    }

    #[tokio::test]
    async fn default_headers_are_exact() {
        let response = app()
            .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
            .await
            .unwrap();
        for (name, value) in DEFAULT_HEADERS {
            assert_eq!(
                response.headers().get(*name).and_then(|v| v.to_str().ok()),
                Some(*value),
                "header {name}"
            );
        }
    }

    #[tokio::test]
    async fn request_id_is_generated_and_echoed() {
        let response = app()
            .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert!(response.headers().get("x-request-id").is_some());
    }

    #[tokio::test]
    async fn oversized_body_is_rejected() {
        let body = vec![b'a'; BODY_LIMIT + 1];
        let response = app()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    }

    #[tokio::test]
    async fn json_gets_a_charset_and_an_etag() {
        let response = app()
            .oneshot(Request::builder().uri("/json").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(
            response.headers().get(header::CONTENT_TYPE).unwrap(),
            "application/json; charset=utf-8"
        );
        assert_eq!(
            response.headers().get(header::ETAG).unwrap(),
            etag_for(br#"{"a":1}"#).as_str()
        );
        assert_eq!(
            response.headers().get(header::CACHE_CONTROL).unwrap(),
            "max-age=0, private, must-revalidate"
        );
    }

    #[tokio::test]
    async fn matching_if_none_match_answers_304() {
        let tag = etag_for(br#"{"a":1}"#);
        let response = app()
            .oneshot(
                Request::builder()
                    .uri("/json")
                    .header(header::IF_NONE_MATCH, tag)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_MODIFIED);
    }

    #[tokio::test]
    async fn errors_are_no_cache() {
        let response = stack(Router::new().route("/boom", get(|| async { StatusCode::NOT_FOUND })))
            .oneshot(Request::builder().uri("/boom").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(
            response.headers().get(header::CACHE_CONTROL).unwrap(),
            "no-cache"
        );
    }
}
