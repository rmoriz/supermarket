//! `IrcLogsController`: the chat log redirector.
//!
//! Every action is an external redirect to botbot.me or the old GitHub archive
//! repository; an unparsable date is a 404. Both are route-level redirects.

use axum::extract::{Path, State};

use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};

use crate::state::AppState;
use crate::web::pages;

const BOTBOT_BASE: &str = "https://botbot.me/freenode/";
const GITHUB_REPO: &str = "https://github.com/chef/irc_log_archives";

/// A controller `redirect_to`, not a route redirect: the default headers and
/// CSP stay, the content type is added by the parity layer.
fn external(url: &str) -> Response {
    let mut response = StatusCode::FOUND.into_response();
    if let Ok(value) = url.parse() {
        response.headers_mut().insert(header::LOCATION, value);
    }
    response
}

/// `GET /chat`.
pub async fn index() -> Response {
    external("https://botbot.me/freenode/chef/")
}

/// `Date.parse`: enough of Ruby's lenient reader for the accepted shapes.
fn parse_date(raw: &str) -> Option<(i32, u32, u32)> {
    let text = raw.trim();
    if text.is_empty() {
        return None;
    }
    // Time suffixes do not change the date (`"2014-09-24 12:00"`), while a
    // month name introduces a space-separated date of its own.
    let mut text = text;
    if let Some((date, _)) = text.split_once('T') {
        text = date;
    } else if let Some((date, rest)) = text.split_once(' ') {
        let iso_first = date.len() >= 4
            && date.as_bytes()[..4].iter().all(u8::is_ascii_digit)
            && date.contains('-');
        if rest.contains(':') || iso_first {
            text = date;
        }
    }

    let months = [
        ("jan", 1),
        ("feb", 2),
        ("mar", 3),
        ("apr", 4),
        ("may", 5),
        ("jun", 6),
        ("jul", 7),
        ("aug", 8),
        ("sep", 9),
        ("oct", 10),
        ("nov", 11),
        ("dec", 12),
    ];
    let month = |name: &str| {
        let lower = name.to_ascii_lowercase();
        months
            .iter()
            .find(|(prefix, _)| lower.starts_with(prefix))
            .map(|(_, value)| *value)
    };

    let numeric = |value: &str| -> Option<u32> { value.parse().ok() };

    if let Some((first, second, third)) = split3(text) {
        if let (Some(year), Some(month), Some(day)) =
            (numeric(first), numeric(second), numeric(third))
        {
            if first.len() == 4 {
                // ISO: year first.
                if (1..=12).contains(&month) && (1..=31).contains(&day) {
                    return Some((year as i32, month, day));
                }
            }
        }
        if let (Some(day), Some(month), Some(year)) =
            (numeric(first), month(second), numeric(third))
        {
            if (1..=31).contains(&day) {
                return Some((year as i32, month, day));
            }
        }
        if let (Some(month), Some(day), Some(year)) =
            (month(first), numeric(second), numeric(third))
        {
            if (1..=31).contains(&day) {
                return Some((year as i32, month, day));
            }
        }
    }

    if text.len() == 8 && text.chars().all(|c| c.is_ascii_digit()) {
        let year = text[0..4].parse().ok()?;
        let month = text[4..6].parse().ok()?;
        let day = text[6..8].parse().ok()?;
        if (1..=12).contains(&month) && (1..=31).contains(&day) {
            return Some((year, month, day));
        }
    }
    None
}

fn split3(text: &str) -> Option<(&str, &str, &str)> {
    let parts: Vec<&str> = if text.contains(['-', '/']) {
        text.split(['-', '/']).collect()
    } else {
        text.split_whitespace().collect()
    };
    if parts.len() != 3 || parts.iter().any(|part| part.is_empty()) {
        return None;
    }
    Some((parts[0], parts[1], parts[2]))
}

/// `GET /chat/:channel` and `GET /chat/:channel/:date`.
pub async fn show_channel(Path(channel): Path<String>) -> Response {
    external(&format!("{BOTBOT_BASE}{channel}"))
}

pub async fn show_date(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((channel, date)): Path<(String, String)>,
) -> Response {
    let Some((year, month, day)) = parse_date(&date) else {
        return not_found(&state, &headers).await;
    };
    // botbot.me logged from 2013-08-08 onwards; earlier days live on GitHub.
    if (year, month, day) > (2013, 8, 8) {
        external(&format!("{BOTBOT_BASE}{channel}/{date}"))
    } else {
        external(GITHUB_REPO)
    }
}

/// `not_found!` renders `exceptions/404` from inside the controller, so the
/// normal header stack applies; only the body class is the controller's.
async fn not_found(state: &AppState, headers: &HeaderMap) -> Response {
    pages::error_page_as(state, headers, StatusCode::NOT_FOUND, false, "irc_logs").await
}

#[cfg(test)]
mod tests {
    use super::parse_date;

    #[test]
    fn parses_the_shapes_date_parse_accepts() {
        assert_eq!(parse_date("2014-09-24"), Some((2014, 9, 24)));
        assert_eq!(parse_date("2014-9-24"), Some((2014, 9, 24)));
        assert_eq!(parse_date("20140924"), Some((2014, 9, 24)));
        assert_eq!(parse_date("24 Sep 2014"), Some((2014, 9, 24)));
        assert_eq!(parse_date("Sep 24 2014"), Some((2014, 9, 24)));
        assert_eq!(parse_date("2014-09-24 12:00"), Some((2014, 9, 24)));
    }

    #[test]
    fn rejects_garbage_and_out_of_range_months() {
        assert_eq!(parse_date("2014-13-01"), None);
        assert_eq!(parse_date("garbage"), None);
        assert_eq!(parse_date("2014_09_24"), None);
        assert_eq!(parse_date("24.09.2014"), None);
    }
}
