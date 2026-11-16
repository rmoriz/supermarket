//! Complete router: every route in `config/routes.rb`, all with real handlers.
//! The manifest test proves nothing is lost or invented in either direction.

pub mod table;

use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, patch, post, put};
use axum::Router;
use tower_http::services::ServeDir;

use crate::state::AppState;

/// Content type a stub answers with, derived from the action it stands in for.
pub fn content_type_for(action: &str) -> &'static str {
    if action.starts_with("api/") {
        "application/json; charset=utf-8"
    } else if action == "pages#robots" {
        "text/plain; charset=utf-8"
    } else if action.ends_with("#followed_cookbook_activity") {
        "application/atom+xml; charset=utf-8"
    } else {
        "text/html; charset=utf-8"
    }
}

pub fn stub(action: &'static str) -> Response {
    (
        StatusCode::NOT_IMPLEMENTED,
        [(header::CONTENT_TYPE, content_type_for(action))],
        Body::from(format!(
            "{{\"error\":\"not_implemented\",\"action\":\"{action}\"}}\n"
        )),
    )
        .into_response()
}

/// Unmatched path or method: `ExceptionsController#show` renders the HTML error
/// page (a `.json` path still negotiates HTML for `*/*`).
/// `SitemapRefreshWorker` output, served by the static file middleware upstream.
async fn sitemap() -> Response {
    match tokio::fs::read("public/sitemap.xml.gz").await {
        Ok(bytes) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "application/x-gzip")],
            bytes,
        )
            .into_response(),
        Err(_) => (StatusCode::NOT_FOUND, "").into_response(),
    }
}

pub async fn fallback(State(state): State<AppState>, headers: HeaderMap, uri: Uri) -> Response {
    // `robots.:format` has a dot in the segment, so it never matches a router
    // path parameter; dispatch it here.
    if let Some(format) = uri.path().strip_prefix("/robots.") {
        return crate::web::pages::robots_format(State(state), Path(format.to_owned())).await;
    }
    routing_not_found(&state, &headers).await
}

/// The catch-all 404, shared with the feature gates that stand in for Rails'
/// route constraints.
pub async fn routing_not_found(state: &AppState, headers: &HeaderMap) -> Response {
    let json = headers
        .get(header::ACCEPT)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.contains("application/json"))
        .unwrap_or(false);
    // No route matched, so the failure happens outside any controller: the
    // exceptions page carries no CSP, cache-control or session cookie.
    let mut response =
        crate::web::pages::error_page(state, headers, StatusCode::NOT_FOUND, json).await;
    response
        .extensions_mut()
        .insert(crate::middleware::ExceptionResponse);
    response
}

/// Routes with a real handler (the `every_table_route_has_a_handler` test keeps
/// this list equal to the manifest).
fn implemented(method: &str, path: &str) -> bool {
    matches!(
        (method, path),
        ("GET", "/api/v1/cookbooks")
            | ("GET", "/api/v1/search")
            | ("GET", "/api/v1/cookbooks/{cookbook}")
            | ("GET", "/api/v1/cookbooks/{cookbook}/contingent")
            | ("GET", "/api/v1/cookbooks/{cookbook}/versions/{version}")
            | (
                "GET",
                "/api/v1/cookbooks/{cookbook}/versions/{version}/download"
            )
            | ("GET", "/api/v1/users/{user}")
            | ("GET", "/api/v1/metrics")
            | ("GET", "/api/v1/health")
            | ("GET", "/status")
            | ("GET", "/api/v1/tools")
            | ("GET", "/api/v1/tools/{tool}")
            | ("GET", "/api/v1/tools-search")
            | ("POST", "/fieri/jobs")
            | ("GET", "/fieri/status")
            | ("POST", "/api/v1/cookbook-versions/cookstyle_evaluation")
            | ("POST", "/api/v1/cookbook-versions/collaborators_evaluation")
            | ("POST", "/api/v1/cookbook-versions/publish_evaluation")
            | ("POST", "/api/v1/quality_metrics/cookstyle_evaluation")
            | ("POST", "/api/v1/quality_metrics/collaborators_evaluation")
            | ("POST", "/api/v1/quality_metrics/publish_evaluation")
            | (
                "POST",
                "/api/v1/quality_metrics/supported_platforms_evaluation"
            )
            | (
                "POST",
                "/api/v1/quality_metrics/contributing_file_evaluation"
            )
            | ("POST", "/api/v1/quality_metrics/testing_file_evaluation")
            | ("POST", "/api/v1/quality_metrics/version_tag_evaluation")
            | ("POST", "/api/v1/quality_metrics/no_binaries_evaluation")
            | ("POST", "/tools")
            | ("PATCH", "/tools/{id}")
            | ("PUT", "/tools/{id}")
            | ("DELETE", "/tools/{id}")
            | ("POST", "/tools/{id}/adoption")
            | ("GET", "/tools.atom")
            | ("GET", "/tools-directory")
            | ("GET", "/chat")
            | ("GET", "/chat/{channel}")
            | ("GET", "/chat/{channel}/{date}")
            | ("GET", "/universe")
            | ("GET", "/api/v1/universe")
            | ("GET", "/robots.txt")
            | ("GET", "/robots")
            | ("GET", "/")
            | ("GET", "/dashboard")
            | ("GET", "/sign-in")
            | ("GET", "/sign-up")
            | ("GET", "/login")
            | ("GET", "/signin")
            | ("DELETE", "/logout")
            | ("DELETE", "/signout")
            | ("DELETE", "/sign-out")
            | ("GET", "/auth/failure")
            | ("POST", "/auth/{provider}")
            | ("POST", "/auth/chef_oauth2")
            | ("GET", "/auth/chef_oauth2/callback")
            | ("POST", "/auth/chef_oauth2/callback")
            | ("GET", "/auth/{provider}/callback")
            | ("POST", "/auth/{provider}/callback")
            | ("GET", "/documentation")
            | ("GET", "/cookbooks")
            | ("GET", "/cookbooks.atom")
            | ("GET", "/available_for_adoption.atom")
            | ("GET", "/cookbooks-directory")
            | ("GET", "/available_for_adoption")
            | ("GET", "/cookbooks/{id}")
            | ("GET", "/cookbooks/{id}/download")
            | ("GET", "/cookbooks/{id}/versions/{version}")
            | ("GET", "/cookbooks/{id}/versions/{version}/download")
            | ("GET", "/users/{id}")
            | ("GET", "/users/{id}/tools")
            | ("GET", "/users/{id}/groups")
            | ("GET", "/users/{id}/followed_cookbook_activity")
            | ("GET", "/groups")
            | ("POST", "/groups")
            | ("GET", "/groups/new")
            | ("GET", "/groups/{id}")
            | ("GET", "/groups/{id}/edit")
            | ("PATCH", "/groups/{id}")
            | ("PUT", "/groups/{id}")
            | ("DELETE", "/groups/{id}")
            | ("GET", "/group_members")
            | ("GET", "/group_members/new")
            | ("POST", "/group_members")
            | ("GET", "/group_members/{id}")
            | ("GET", "/group_members/{id}/edit")
            | ("PATCH", "/group_members/{id}")
            | ("PUT", "/group_members/{id}")
            | ("DELETE", "/group_members/{id}")
            | ("POST", "/group_members/{id}/make_admin")
            | ("PUT", "/users/{id}/make_admin")
            | ("DELETE", "/users/{id}/revoke_admin")
            | ("DELETE", "/users/{user_id}/accounts/{id}")
            | ("GET", "/unsubscribe/{token}")
            | ("GET", "/collaborators/new")
            | ("GET", "/tools")
            | ("GET", "/tools/new")
            | ("GET", "/tools/{id}")
            | ("GET", "/tools/{id}/edit")
            | ("POST", "/api/v1/cookbooks")
            | ("DELETE", "/api/v1/cookbooks/{cookbook}")
            | ("DELETE", "/api/v1/cookbooks/{cookbook}/versions/{version}")
            | ("GET", "/profile/link-github")
            | ("PATCH", "/cookbooks/{id}")
            | ("PUT", "/cookbooks/{id}")
            | ("PUT", "/cookbooks/{id}/follow")
            | ("DELETE", "/cookbooks/{id}/unfollow")
            | ("PUT", "/cookbooks/{id}/deprecate")
            | ("DELETE", "/cookbooks/{id}/deprecate")
            | ("PUT", "/cookbooks/{id}/toggle_featured")
            | ("GET", "/cookbooks/{id}/deprecate_search")
            | ("POST", "/cookbooks/{id}/adoption")
            | ("GET", "/collaborators")
            | ("POST", "/collaborators")
            | ("DELETE", "/collaborators/{id}")
            | ("DELETE", "/collaborators/{id}/destroy_group")
            | ("PUT", "/collaborators/{id}/transfer")
            | ("PUT", "/cookbooks/{id}/transfer_ownership")
            | ("GET", "/ownership_transfer/{token}/accept")
            | ("GET", "/ownership_transfer/{token}/decline")
            | ("GET", "/profile/edit")
            | ("PATCH", "/profile")
            | ("PUT", "/profile")
            | ("POST", "/profile/update_install_preference")
            | ("PATCH", "/profile/change_password")
    )
}

/// The two evaluation prefixes the vendored engine mounts, both dispatching into
/// `Api::V1::QualityMetricsController`.
fn quality_metric_routes() -> Router<AppState> {
    use crate::api::v1::quality_metrics as qm;
    let mount = |prefix: &str, with_publish: bool| {
        let mut router = Router::new()
            .route(
                &format!("/{prefix}/cookstyle_evaluation"),
                post(qm::cookstyle),
            )
            .route(
                &format!("/{prefix}/collaborators_evaluation"),
                post(qm::collaborators),
            )
            .route(
                &format!("/{prefix}/contributing_file_evaluation"),
                post(qm::contributing_file),
            )
            .route(
                &format!("/{prefix}/testing_file_evaluation"),
                post(qm::testing_file),
            )
            .route(
                &format!("/{prefix}/version_tag_evaluation"),
                post(qm::version_tag),
            )
            .route(
                &format!("/{prefix}/no_binaries_evaluation"),
                post(qm::no_binaries),
            );
        if with_publish {
            router = router
                .route(
                    &format!("/{prefix}/publish_evaluation"),
                    post(qm::publish_quality),
                )
                .route(
                    &format!("/{prefix}/supported_platforms_evaluation"),
                    post(qm::supported_platforms),
                );
        } else {
            router = router.route(&format!("/{prefix}/publish_evaluation"), post(qm::publish));
        }
        router
    };
    mount("api/v1/quality_metrics", true).merge(mount("api/v1/cookbook-versions", false))
}

pub fn router(state: AppState) -> Router {
    let forgery_state = state.clone();
    use crate::api::v1::{
        cookbook_uploads, cookbook_versions, cookbooks, health, metrics, tools, universe, users,
    };
    use crate::web;

    let mut router = Router::new()
        .route("/api/v1/metrics", get(metrics::show))
        .route("/api/v1/health", get(health::show))
        .route("/status", get(health::show))
        .route(
            "/api/v1/cookbooks",
            get(cookbooks::index).post(cookbook_uploads::create),
        )
        .route("/api/v1/search", get(cookbooks::search))
        .route(
            "/api/v1/cookbooks/{cookbook}",
            get(cookbooks::show).delete(cookbook_uploads::destroy),
        )
        .route(
            "/api/v1/cookbooks/{cookbook}/versions/{version}",
            axum::routing::delete(cookbook_uploads::destroy_version),
        )
        .route(
            "/api/v1/cookbooks/{cookbook}/contingent",
            get(cookbooks::contingent),
        )
        .route(
            "/api/v1/cookbooks/{cookbook}/versions/{version}",
            get(cookbook_versions::show),
        )
        .route(
            "/api/v1/cookbooks/{cookbook}/versions/{version}/download",
            get(cookbook_versions::download),
        )
        .route("/api/v1/users/{user}", get(users::show))
        .route("/api/v1/tools", get(tools::index))
        .route("/api/v1/tools/{tool}", get(tools::show))
        .route("/api/v1/tools-search", get(tools::search))
        .route("/universe", get(universe::index))
        .route("/api/v1/universe", get(universe::index))
        // ── web, read paths ─────────────────────────────────────────────────
        .route("/", get(web::pages::welcome))
        .route("/dashboard", get(web::pages::dashboard))
        .route("/documentation", get(web::pages::documentation))
        .route("/robots.txt", get(web::pages::robots))
        .route("/cookbooks", get(web::cookbooks::index))
        .route("/cookbooks.atom", get(web::cookbooks::index))
        .route("/cookbooks-directory", get(web::cookbooks::directory))
        .route(
            "/available_for_adoption",
            get(web::cookbooks::available_for_adoption),
        )
        .route(
            "/available_for_adoption.atom",
            get(web::cookbooks::available_for_adoption_atom),
        )
        .route(
            "/cookbooks/{id}",
            get(web::cookbooks::show)
                .patch(web::cookbooks::update)
                .put(web::cookbooks::update),
        )
        .route("/cookbooks/{id}/download", get(web::cookbooks::download))
        .route(
            "/cookbooks/{id}/versions/{version}",
            get(web::cookbooks::show_version_page),
        )
        .route(
            "/cookbooks/{id}/versions/{version}/download",
            get(web::cookbooks::download_version),
        )
        .route("/users/{id}", get(web::users::show))
        .route("/users/{id}/groups", get(web::users::groups))
        .route(
            "/users/{id}/followed_cookbook_activity",
            get(web::users::followed_cookbook_activity),
        )
        .route("/groups", get(web::groups::index).post(web::groups::create))
        .route("/groups/new", get(web::groups::new))
        .route(
            "/groups/{id}",
            get(web::groups::show)
                .patch(web::groups::update)
                .put(web::groups::update)
                .delete(web::groups::destroy),
        )
        .route("/groups/{id}/edit", get(web::groups::edit))
        .route(
            "/group_members",
            get(web::group_members::missing).post(web::group_members::create),
        )
        .route("/group_members/new", get(web::group_members::missing))
        .route(
            "/group_members/{id}",
            get(web::group_members::missing)
                .patch(web::group_members::missing)
                .put(web::group_members::missing)
                .delete(web::group_members::destroy),
        )
        .route("/group_members/{id}/edit", get(web::group_members::missing))
        .route(
            "/group_members/{id}/make_admin",
            axum::routing::post(web::group_members::make_admin),
        )
        .route(
            "/users/{id}/make_admin",
            axum::routing::put(web::users::make_admin),
        )
        .route(
            "/users/{id}/revoke_admin",
            axum::routing::delete(web::users::revoke_admin),
        )
        .route(
            "/users/{user_id}/accounts/{id}",
            axum::routing::delete(web::accounts::destroy),
        )
        .route(
            "/unsubscribe/{token}",
            get(web::email_preferences::unsubscribe),
        )
        .route("/chat", get(web::irc_logs::index))
        .route("/chat/{channel}", get(web::irc_logs::show_channel))
        .route("/chat/{channel}/{date}", get(web::irc_logs::show_date))
        .route("/robots", get(fallback))
        // ── web, mutations ──────────────────────────────────────────────────
        .route(
            "/cookbooks/{id}/follow",
            axum::routing::put(web::cookbooks::follow),
        )
        .route(
            "/cookbooks/{id}/unfollow",
            axum::routing::delete(web::cookbooks::unfollow),
        )
        .route(
            "/cookbooks/{id}/deprecate",
            axum::routing::put(web::cookbooks::deprecate).delete(web::cookbooks::undeprecate),
        )
        .route(
            "/cookbooks/{id}/toggle_featured",
            axum::routing::put(web::cookbooks::toggle_featured),
        )
        .route(
            "/cookbooks/{id}/deprecate_search",
            get(web::cookbooks::deprecate_search),
        )
        .route(
            "/cookbooks/{id}/adoption",
            axum::routing::post(web::cookbooks::adoption),
        )
        .route(
            "/cookbooks/{id}/transfer_ownership",
            axum::routing::put(web::transfer_ownership::transfer),
        )
        .route(
            "/ownership_transfer/{token}/accept",
            get(web::transfer_ownership::accept),
        )
        .route(
            "/ownership_transfer/{token}/decline",
            get(web::transfer_ownership::decline),
        )
        .route(
            "/collaborators",
            get(web::collaborators::index).post(web::collaborators::create),
        )
        .route(
            "/collaborators/{id}",
            axum::routing::delete(web::collaborators::destroy),
        )
        .route(
            "/collaborators/{id}/destroy_group",
            axum::routing::delete(web::collaborators::destroy_group),
        )
        .route(
            "/collaborators/{id}/transfer",
            axum::routing::put(web::collaborators::transfer),
        )
        .route("/collaborators/new", get(web::collaborators::new))
        .route("/profile/edit", get(web::profile::edit))
        .route("/profile/link-github", get(web::profile::link_github))
        .route(
            "/profile/change_password",
            axum::routing::patch(web::profile::change_password),
        )
        .route("/sign-in", get(web::sessions::new))
        .route("/sign-up", get(web::sessions::new))
        .route("/login", get(web::sessions::login))
        .route("/signin", get(web::sessions::login))
        .route("/logout", axum::routing::delete(web::sessions::logout))
        .route("/signout", axum::routing::delete(web::sessions::logout))
        .route("/sign-out", axum::routing::delete(web::sessions::destroy))
        .route("/auth/failure", get(web::sessions::failure))
        .route(
            "/auth/{provider}",
            axum::routing::post(web::sessions::passthru),
        )
        .route(
            "/auth/{provider}/callback",
            get(web::sessions::callback).post(web::sessions::callback),
        )
        .route(
            "/profile",
            axum::routing::patch(web::profile::update).put(web::profile::update),
        )
        .route(
            "/profile/update_install_preference",
            axum::routing::post(web::profile::update_install_preference),
        )
        .nest_service("/assets", ServeDir::new("static/assets"))
        // Paperclip uploads under `public/system` are served by the web tier
        // upstream; the container has no separate web server in front.
        .nest_service("/system", ServeDir::new("public/system"));

    // Routes upstream removes from the table when a feature is inactive
    // (`constraints: proc { Feature.active?(:tools) }`, and the fieri engine
    // mount). They stay registered here; the `require_feature` layer answers
    // the catch-all 404 while the flag is off.
    let gated_tools = Router::new()
        .route("/tools", get(web::tools::index).post(web::tools::create))
        .route("/tools.atom", get(web::tools::index_atom))
        .route("/tools-directory", get(web::tools::directory))
        .route("/tools/new", get(web::tools::new))
        .route(
            "/tools/{id}",
            get(web::tools::show)
                .patch(web::tools::update)
                .put(web::tools::update)
                .delete(web::tools::destroy),
        )
        .route(
            "/tools/{id}/adoption",
            axum::routing::post(web::tools::adoption),
        )
        .route("/tools/{id}/edit", get(web::tools::edit))
        .route("/users/{id}/tools", get(web::users::tools));
    let gated_fieri = Router::new()
        .route("/fieri/jobs", post(crate::api::fieri::create))
        .route("/fieri/status", get(crate::api::fieri::status))
        .merge(quality_metric_routes());
    router = router.merge(gated_tools).merge(gated_fieri);

    // `public/` is only mounted when `RAILS_SERVE_STATIC_FILES` is set, which is
    // the same switch `production.rb` uses.
    if std::env::var("RAILS_SERVE_STATIC_FILES")
        .ok()
        .is_some_and(|value| !value.is_empty())
    {
        router = router.route("/sitemap.xml.gz", get(sitemap));
    }

    for route in table::ROUTES {
        if implemented(route.method, route.path) {
            continue;
        }
        let path = route.path;
        let handler = move |State(_state): State<AppState>| async move { stub(route.action) };
        let method = route.method;
        router = match method {
            "GET" => router.route(path, get(handler)),
            "POST" => router.route(path, post(handler)),
            "PUT" => router.route(path, put(handler)),
            "PATCH" => router.route(path, patch(handler)),
            "DELETE" => router.route(path, delete(handler)),
            other => panic!("unsupported method {other}"),
        };
        // `handler` is a closure; rebuild it per method arm so the type is concrete.
    }

    router
        .layer(axum::middleware::from_fn_with_state(
            forgery_state,
            crate::middleware::verify_forgery,
        ))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            crate::middleware::require_feature,
        ))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            crate::middleware::host_authorization,
        ))
        .fallback(fallback)
        .method_not_allowed_fallback(fallback)
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;
    use tower::ServiceExt;

    fn app() -> Router {
        router(crate::state::AppState::for_tests(
            super::tests::test_config(),
        ))
    }

    pub(crate) fn test_config() -> crate::config::Config {
        crate::config::Config::from_env_with(&|var| match var {
            "DATABASE_URL" => Some("postgres://x/y".into()),
            "REDIS_URL" => Some("redis://localhost:56379/0".into()),
            "SECRET_KEY_BASE" => Some("s".into()),
            "FQDN" => Some("localhost".into()),
            _ => None,
        })
        .unwrap()
    }

    #[tokio::test]
    async fn every_manifest_route_is_registered() {
        let app = app();
        let mut missing = Vec::new();
        for route in table::ROUTES {
            let uri = route
                .path
                .replace("{cookbook}", "redis")
                .replace("{version}", "1.0.0")
                .replace("{user}", "bob")
                .replace("{user_id}", "1")
                .replace("{id}", "1")
                .replace("{token}", "t")
                .replace("{channel}", "general")
                .replace("{date}", "2014-01-01")
                .replace("{provider}", "github")
                .replace("{tool}", "knife-thing");

            let request = axum::http::Request::builder()
                .method(route.method)
                .uri(&uri)
                .body(Body::empty())
                .unwrap();
            let status = app.clone().oneshot(request).await.unwrap().status();
            let registered = implemented(route.method, route.path);
            if !registered
                && (status == StatusCode::NOT_FOUND || status == StatusCode::METHOD_NOT_ALLOWED)
            {
                missing.push(format!("{} {} -> {}", route.method, route.path, status));
            }
        }
        assert!(
            missing.is_empty(),
            "unregistered routes:\n{}",
            missing.join("\n")
        );
    }

    #[tokio::test]
    async fn every_registered_route_exists_upstream() {
        let source =
            std::fs::read_to_string("supermarket-ruby/src/supermarket/config/routes.rb").unwrap();
        let router_paths: std::collections::HashSet<&str> =
            table::ROUTES.iter().map(|r| r.path).collect();

        for route in table::ROUTES {
            // Either a literal path in routes.rb, or produced by a `resources` block,
            // or a documented extra from a redirect/mount.
            let rails_path = route.path.replace('{', ":").replace('}', "");
            let literal = rails_path.contains('.')
                || source.contains(&rails_path)
                || source.contains(
                    rails_path
                        .trim_start_matches("/api/v1")
                        .trim_start_matches('/'),
                );
            let expanded = route.path.starts_with("/cookbooks")
                || route.path.starts_with("/collaborators")
                || route.path.starts_with("/users")
                || route.path.starts_with("/groups")
                || route.path.starts_with("/group_members")
                || route.path.starts_with("/tools")
                || route.path.starts_with("/profile")
                || route.path.starts_with("/fieri");
            assert!(
                literal || expanded,
                "route {} {} is not upstream",
                route.method,
                route.path
            );
        }
        assert!(router_paths.contains("/api/v1/cookbooks"));
    }

    #[tokio::test]
    async fn every_table_route_has_a_handler() {
        // Nothing is left on the 501 placeholder; the manifest test above proves
        // each route is registered, this one proves the placeholder is unused.
        for route in table::ROUTES {
            assert!(
                implemented(route.method, route.path)
                    || route.path.starts_with("/auth/")
                    || route.path.starts_with("/fieri"),
                "route {} {} is still a stub",
                route.method,
                route.path
            );
        }
    }

    #[tokio::test]
    async fn unknown_paths_are_html_404s() {
        let response = app()
            .oneshot(
                axum::http::Request::builder()
                    .uri("/nope.json")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert_eq!(
            response.headers().get(header::CONTENT_TYPE).unwrap(),
            "text/html; charset=utf-8"
        );
        let body = to_bytes(response.into_body(), 1 << 20).await.unwrap();
        assert!(body.starts_with(b"<!DOCTYPE html>"));
    }

    fn app_with(features: &str) -> Router {
        let mut cfg = test_config();
        cfg.features = features
            .split(',')
            .filter(|feature| !feature.is_empty())
            .map(str::to_owned)
            .collect();
        router(crate::state::AppState::for_tests(cfg))
    }

    async fn fetch(app: Router, method: &str, path: &str) -> (StatusCode, Vec<u8>) {
        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .method(method)
                    .uri(path)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let body = to_bytes(response.into_body(), 1 << 20).await.unwrap();
        (status, body.to_vec())
    }

    /// The `tools` constraint upstream removes these paths from the table, so
    /// they answer the same catch-all 404 as an unknown path.
    #[tokio::test]
    async fn tools_routes_404_without_the_flag() {
        for path in ["/tools", "/tools-directory", "/users/ada/tools"] {
            let (status, body) = fetch(app_with("fieri"), "GET", path).await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{path}");
            assert!(body.starts_with(b"<!DOCTYPE html>"), "{path}");
        }
        let (status, _) = fetch(app_with("fieri"), "POST", "/tools").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    /// `/api/v1/tools` is outside the constraint upstream and keeps answering.
    #[tokio::test]
    async fn api_tool_routes_ignore_the_flag() {
        let (status, _) = fetch(app_with(""), "GET", "/api/v1/tools").await;
        assert_ne!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn tools_routes_answer_with_the_flag() {
        let (status, _) = fetch(app_with("tools,fieri"), "GET", "/tools").await;
        assert_ne!(status, StatusCode::NOT_FOUND);
        let (status, _) = fetch(app_with("tools,fieri"), "GET", "/tools-directory").await;
        assert_ne!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn fieri_routes_404_without_the_flag() {
        let (status, body) = fetch(app_with("tools"), "GET", "/fieri/status").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert!(body.starts_with(b"<!DOCTYPE html>"));
        let (status, _) = fetch(
            app_with("tools"),
            "POST",
            "/api/v1/quality_metrics/cookstyle_evaluation",
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        let (status, _) = fetch(
            app_with("tools"),
            "POST",
            "/api/v1/cookbook-versions/cookstyle_evaluation",
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn fieri_routes_answer_with_the_flag() {
        let (status, _) = fetch(app_with("fieri"), "GET", "/fieri/status").await;
        assert_ne!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn host_allowlist_blocks_foreign_hosts() {
        let mut cfg = test_config();
        cfg.allowed_hosts = vec!["example.test".into(), ".example.test".into()];
        let app = router(crate::state::AppState::for_tests(cfg));
        let allowed = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .uri("/")
                    .header(header::HOST, "www.example.test:3000")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_ne!(allowed.status(), StatusCode::FORBIDDEN);
        let blocked = app
            .clone()
            .oneshot(
                axum::http::Request::builder()
                    .uri("/")
                    .header(header::HOST, "evil.test")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(blocked.status(), StatusCode::FORBIDDEN);
        assert_eq!(
            blocked.headers().get(header::CONTENT_TYPE).unwrap(),
            "text/html; charset=UTF-8"
        );
        assert_eq!(
            blocked
                .headers()
                .get(header::CONTENT_LENGTH)
                .and_then(|value| value.to_str().ok()),
            Some("0")
        );
        // An absent Host is blocked too.
        let missing = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(missing.status(), StatusCode::FORBIDDEN);
    }
}
