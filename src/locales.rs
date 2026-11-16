//! Locale strings compiled from `config/locales/*.yml` by `build.rs`.

include!(concat!(env!("OUT_DIR"), "/locales.rs"));

/// Looks a message up by dotted key. Panics on unknown keys: a missing string is a
/// programming error, not a runtime condition.
pub fn t(key: &str, vars: &[(&str, &str)]) -> String {
    let template = lookup(key).unwrap_or_else(|| panic!("missing locale key: {key}"));
    interpolate(template, vars)
}

pub fn has_key(key: &str) -> bool {
    lookup(key).is_some()
}

pub fn lookup(key: &str) -> Option<&'static str> {
    MESSAGES
        .binary_search_by(|(k, _)| (*k).cmp(key))
        .ok()
        .map(|i| MESSAGES[i].1)
}

pub fn interpolate(template: &str, vars: &[(&str, &str)]) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(start) = rest.find("%{") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        match after.find('}') {
            Some(end) => {
                let name = &after[..end];
                match vars.iter().find(|(k, _)| *k == name) {
                    Some((_, value)) => out.push_str(value),
                    None => out.push_str(&rest[start..start + 2 + end + 1]),
                }
                rest = &after[end + 1..];
            }
            None => {
                out.push_str(&rest[start..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_error_code_is_verbatim() {
        assert_eq!(t("api.error_codes.not_found", &[]), "NOT_FOUND");
    }

    #[test]
    fn known_keys_exist() {
        for key in [
            "api.error_messages.not_found",
            "api.error_messages.negative_parameter",
            "api.error_messages.authentication_key_error",
            "user.signed_in",
            "cookbook.deprecated",
        ] {
            assert!(has_key(key), "{key} missing");
        }
    }

    #[test]
    fn interpolation_matches_rails_placeholders() {
        let msg = t(
            "api.error_messages.negative_parameter",
            &[("start", "-1"), ("items", "not provided")],
        );
        assert_eq!(
            msg,
            "Start and item parameters must be positive numbers but they are start: -1 and items: not provided."
        );
    }

    #[test]
    fn unknown_placeholders_survive() {
        assert_eq!(interpolate("a %{x} b", &[]), "a %{x} b");
    }

    #[test]
    #[should_panic(expected = "missing locale key")]
    fn unknown_key_panics() {
        t("nope.nope", &[]);
    }
}
