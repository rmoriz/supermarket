//! `Api::V1Controller#init_params` and `#item_limit`, including their quirks:
//! `"abc".to_i == 0`, an empty `items` means 0, and the upper limit comes from the
//! `API_ITEM_LIMIT` environment variable with a hard 100 fallback.

use crate::error::ApiError;
use crate::locales::t;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Order {
    RecentlyUpdated,
    RecentlyAdded,
    MostDownloaded,
    MostFollowed,
    ByName,
}

impl Order {
    /// Exact `Cookbook::ORDER_OPTIONS` values.
    pub fn cookbook_sql(self) -> &'static str {
        match self {
            Order::RecentlyUpdated => "cookbooks.deprecated, cookbooks.updated_at DESC",
            Order::RecentlyAdded => "cookbooks.deprecated, cookbooks.id DESC",
            Order::MostDownloaded => {
                "cookbooks.deprecated, (cookbooks.web_download_count + cookbooks.api_download_count) DESC, cookbooks.id ASC"
            }
            Order::MostFollowed => {
                "cookbooks.deprecated, cookbook_followers_count DESC, cookbooks.id ASC"
            }
            Order::ByName => "cookbooks.deprecated, cookbooks.name ASC",
        }
    }

    /// `Tool.ordered_by`.
    pub fn tool_sql(self) -> &'static str {
        match self {
            Order::RecentlyAdded => "id DESC",
            _ => "name ASC",
        }
    }

    /// `ORDER_OPTIONS.fetch(option, ...)`: unknown strings fall back to `by_name`.
    pub fn parse(raw: &str) -> Order {
        match raw {
            "recently_updated" => Order::RecentlyUpdated,
            "recently_added" => Order::RecentlyAdded,
            "most_downloaded" => Order::MostDownloaded,
            "most_followed" => Order::MostFollowed,
            _ => Order::ByName,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ListParams {
    pub start: i64,
    pub items: i64,
    pub order: Order,
}

/// Ruby's `String#to_i`: leading integer, otherwise 0.
pub fn ruby_to_i(raw: &str) -> i64 {
    let trimmed = raw.trim_start();
    let mut value: i64 = 0;
    let mut negative = false;
    let mut chars = trimmed.chars().peekable();
    if let Some(sign @ ('-' | '+')) = chars.peek() {
        negative = *sign == '-';
        chars.next();
    }
    let mut any = false;
    for ch in chars {
        let Some(digit) = ch.to_digit(10) else { break };
        any = true;
        value = value.saturating_mul(10).saturating_add(digit as i64);
    }
    if !any {
        return 0;
    }
    if negative {
        -value
    } else {
        value
    }
}

/// `item_limit`: `API_ITEM_LIMIT` when it parses to something positive, else 100.
pub fn item_limit(configured: Option<i32>) -> i64 {
    match configured {
        Some(limit) if limit > 0 => limit as i64,
        _ => 100,
    }
}

pub fn parse(
    start: Option<&str>,
    items: Option<&str>,
    order: Option<&str>,
    api_item_limit: Option<i32>,
) -> Result<ListParams, ApiError> {
    let start_value = ruby_to_i(start.unwrap_or("0"));
    let items_value = ruby_to_i(items.unwrap_or("10")).min(item_limit(api_item_limit));

    if start_value < 0 || items_value < 0 {
        return Err(ApiError::NegativeParameter {
            start: start.unwrap_or("not provided").to_owned(),
            items: items.unwrap_or("not provided").to_owned(),
        });
    }

    Ok(ListParams {
        start: start_value,
        items: items_value,
        order: Order::parse(order.unwrap_or("name ASC")),
    })
}

/// Unused-message guard: the negative-parameter text lives in the locale table.
pub fn negative_parameter_message() -> String {
    t("api.error_messages.negative_parameter", &[])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_zero_and_ten() {
        let params = parse(None, None, None, None).unwrap();
        assert_eq!(params.start, 0);
        assert_eq!(params.items, 10);
        assert_eq!(params.order, Order::ByName);
    }

    #[test]
    fn items_clamp_to_the_default_limit() {
        assert_eq!(parse(None, Some("500"), None, None).unwrap().items, 100);
    }

    #[test]
    fn items_clamp_to_the_configured_limit() {
        assert_eq!(parse(None, Some("500"), None, Some(7)).unwrap().items, 7);
        assert_eq!(parse(None, Some("5"), None, Some(0)).unwrap().items, 5);
        assert_eq!(parse(None, Some("500"), None, Some(0)).unwrap().items, 100);
    }

    #[test]
    fn negative_start_is_rejected_with_the_raw_values() {
        let error = parse(Some("-1"), Some("10"), None, None).unwrap_err();
        assert_eq!(
            error.messages(),
            vec![
                "Start and item parameters must be positive numbers but they are start: -1 and items: 10."
            ]
        );
    }

    #[test]
    fn absent_negative_params_say_not_provided() {
        let error = parse(None, Some("-2"), None, None).unwrap_err();
        assert_eq!(
            error.messages(),
            vec![
                "Start and item parameters must be positive numbers but they are start: not provided and items: -2."
            ]
        );
    }

    #[test]
    fn non_numeric_and_empty_params_become_zero() {
        assert_eq!(ruby_to_i("abc"), 0);
        assert_eq!(ruby_to_i("12abc"), 12);
        assert_eq!(ruby_to_i(""), 0);
        assert_eq!(parse(None, Some(""), None, None).unwrap().items, 0);
    }

    #[test]
    fn unknown_order_falls_back_to_name() {
        assert_eq!(
            parse(None, None, Some("nope"), None).unwrap().order,
            Order::ByName
        );
    }
}
