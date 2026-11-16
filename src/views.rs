//! HTML fragments for error pages. Replaced by askama templates in phase 3; until
//! then they exist so the not-found path already answers `text/html`.

use axum::http::StatusCode;

const SHELL: &str = concat!(
    "<!DOCTYPE html>\n<html lang=\"en\"><head><meta charset=\"utf-8\">",
    "<title>%TITLE%</title></head><body>",
    "<h1>%TITLE%</h1><p>%BODY%</p></body></html>\n"
);

pub fn error_page(status: StatusCode) -> String {
    let (title, body) = match status {
        StatusCode::NOT_FOUND => (
            "The page you were looking for doesn't exist.",
            "You may have mistyped the address or the page may have moved.",
        ),
        StatusCode::INTERNAL_SERVER_ERROR => (
            "We're sorry, but something went wrong.",
            "If you are the application owner check the logs for more information.",
        ),
        StatusCode::UNPROCESSABLE_ENTITY => (
            "The change you wanted was rejected.",
            "Maybe you tried to change something you didn't have access to.",
        ),
        _ => ("An error occurred.", "Please try again later."),
    };
    SHELL.replace("%TITLE%", title).replace("%BODY%", body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn not_found_page_matches_upstream_wording() {
        let page = error_page(StatusCode::NOT_FOUND);
        assert!(page.contains("The page you were looking for doesn't exist."));
    }
}
