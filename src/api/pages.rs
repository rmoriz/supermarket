//! `PagesController#robots` — `app/views/pages/robots.text.erb`.

use axum::extract::State;
use axum::http::header;
use axum::response::{IntoResponse, Response};

use crate::state::AppState;

pub async fn robots(State(state): State<AppState>) -> Response {
    let mut body = String::from("User-agent: *\n");
    if let Some(allow) = &state.cfg.robots_allow {
        body.push_str(&format!("Allow: {allow}\n"));
    }
    if let Some(disallow) = &state.cfg.robots_disallow {
        body.push_str(&format!("Disallow: {disallow}\n"));
    }
    body.push_str(&format!(
        "Sitemap: {}/sitemap.xml.gz\n",
        state.cfg.base_url()
    ));
    ([(header::CONTENT_TYPE, "text/plain")], body).into_response()
}

#[cfg(test)]
mod tests {
    use crate::config::Config;
    use crate::state::AppState;

    fn state(robots: Option<&str>) -> AppState {
        let robots = robots.map(str::to_owned);
        AppState::for_tests(
            Config::from_env_with(&move |var| match var {
                "DATABASE_URL" => Some("postgres://x/y".into()),
                "REDIS_URL" => Some("redis://localhost:56379/0".into()),
                "SECRET_KEY_BASE" => Some("s".into()),
                "FQDN" => Some("localhost".into()),
                "PORT" => Some("3005".into()),
                "ROBOTS_ALLOW" => robots.clone(),
                _ => None,
            })
            .unwrap(),
        )
    }

    #[tokio::test]
    async fn body_matches_the_template() {
        let response = super::robots(axum::extract::State(state(Some("/")))).await;
        let body = axum::body::to_bytes(response.into_body(), 4096)
            .await
            .unwrap();
        assert_eq!(
            std::str::from_utf8(&body).unwrap(),
            "User-agent: *\nAllow: /\nSitemap: http://localhost:3005/sitemap.xml.gz\n"
        );
    }
}
