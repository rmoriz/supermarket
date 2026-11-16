//! Query-string extractor that keeps Rails' array semantics: `platforms[]=x` is
//! the key `platforms[]`, and repeated keys accumulate.

use axum::extract::FromRequestParts;
use axum::http::request::Parts;

#[derive(Debug, Default, Clone)]
pub struct QueryMap(Vec<(String, String)>);

impl QueryMap {
    pub fn parse(raw: &str) -> Self {
        QueryMap(
            form_urlencoded::parse(raw.as_bytes())
                .map(|(k, v)| (k.into_owned(), v.into_owned()))
                .collect(),
        )
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.0
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    pub fn all(&self, key: &str) -> Vec<&str> {
        self.0
            .iter()
            .filter(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
            .collect()
    }

    /// A key that may arrive as `key[]=a&key[]=b` or `key[]=a,b`.
    pub fn array(&self, key: &str) -> Vec<String> {
        let bare = key.trim_end_matches("[]");
        // Rails also accepts the bare key for a single value, comma-splittable.
        let single = self
            .get(bare)
            .into_iter()
            .flat_map(|value| value.split(','));
        let mut values: Vec<String> = self
            .all(key)
            .into_iter()
            .flat_map(|value| value.split(','))
            .chain(single)
            .map(str::to_owned)
            .collect();
        values.retain(|v| !v.is_empty());
        values
    }
}

impl<S: Send + Sync> FromRequestParts<S> for QueryMap {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        Ok(QueryMap::parse(parts.uri.query().unwrap_or("")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bracket_keys_and_repeats_are_preserved() {
        let query = QueryMap::parse("platforms%5B%5D=debian&platforms%5B%5D=ubuntu&start=3");
        assert_eq!(query.array("platforms[]"), vec!["debian", "ubuntu"]);
        assert_eq!(query.get("start"), Some("3"));
    }

    #[test]
    fn bare_and_comma_forms_are_accepted() {
        assert_eq!(
            QueryMap::parse("platforms=debian,ubuntu").array("platforms[]"),
            vec!["debian", "ubuntu"]
        );
    }
}
