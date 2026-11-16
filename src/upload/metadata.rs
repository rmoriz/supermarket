//! Port of `CookbookUpload::Metadata`, the slice of `metadata.json` that is used
//! and stored. Unknown keys are ignored; missing ones take their defaults.

use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Metadata {
    pub name: String,
    pub version: String,
    pub description: String,
    pub license: String,
    pub platforms: BTreeMap<String, String>,
    pub dependencies: BTreeMap<String, String>,
    pub source_url: String,
    pub issues_url: String,
    pub privacy: bool,
    pub chef_versions: Value,
    pub ohai_versions: Value,
}

impl Default for Metadata {
    fn default() -> Self {
        Self {
            name: String::new(),
            version: String::new(),
            description: String::new(),
            license: String::new(),
            platforms: BTreeMap::new(),
            dependencies: BTreeMap::new(),
            source_url: String::new(),
            issues_url: String::new(),
            privacy: false,
            // `JSON::Array.default([])`, not null.
            chef_versions: Value::Array(Vec::new()),
            ohai_versions: Value::Array(Vec::new()),
        }
    }
}

/// Raised where `dry-struct` would raise: the root is not an object, or a
/// attribute cannot be coerced.
#[derive(Debug, PartialEq, Eq)]
pub struct InvalidMetadata;

fn string(value: &Value) -> Option<String> {
    Some(match value {
        Value::String(text) => text.clone(),
        Value::Number(number) => number.to_string(),
        Value::Bool(flag) => flag.to_string(),
        Value::Null => String::new(),
        other => serde_json::to_string(other).ok()?,
    })
}

fn map(value: &Value) -> Option<BTreeMap<String, String>> {
    let object = value.as_object()?;
    object
        .iter()
        .map(|(key, value)| Some((key.clone(), string(value)?)))
        .collect()
}

fn boolean(value: &Value) -> Option<bool> {
    Some(match value {
        Value::Bool(flag) => *flag,
        Value::Number(number) => number.as_f64()? != 0.0,
        Value::String(text) => match text.trim().to_ascii_lowercase().as_str() {
            "true" | "t" | "yes" | "y" | "on" | "1" => true,
            "false" | "f" | "no" | "n" | "off" | "0" | "" => false,
            _ => return None,
        },
        _ => return None,
    })
}

fn json_array(value: &Value) -> Option<Value> {
    match value {
        Value::Array(_) => Some(value.clone()),
        Value::String(text) => {
            let parsed: Value = serde_json::from_str(text).ok()?;
            parsed.is_array().then_some(parsed)
        }
        _ => None,
    }
}

impl Metadata {
    pub fn from_json(value: &Value) -> Result<Self, InvalidMetadata> {
        // `Dry::Struct.new([])` (and `nil`) treats an empty argument list as "all
        // defaults"; a populated array cannot be converted to a Hash.
        let empty = matches!(value, Value::Null)
            || matches!(value, Value::Array(items) if items.is_empty());
        if empty {
            return Ok(Metadata::default());
        }
        let object = value.as_object().ok_or(InvalidMetadata)?;

        // `Coercible::String.default("")`
        let text = |key: &str| -> Result<String, InvalidMetadata> {
            match object.get(key) {
                None => Ok(String::new()),
                Some(value) => string(value).ok_or(InvalidMetadata),
            }
        };
        let hash = |key: &str| -> Result<BTreeMap<String, String>, InvalidMetadata> {
            match object.get(key) {
                None => Ok(BTreeMap::new()),
                Some(value) => map(value).ok_or(InvalidMetadata),
            }
        };

        let mut metadata = Metadata {
            name: text("name")?,
            version: text("version")?,
            description: text("description")?,
            license: text("license")?,
            platforms: hash("platforms")?,
            dependencies: hash("dependencies")?,
            source_url: text("source_url")?,
            issues_url: text("issues_url")?,
            privacy: match object.get("privacy") {
                None => false,
                Some(value) => boolean(value).ok_or(InvalidMetadata)?,
            },
            chef_versions: match object.get("chef_versions") {
                None => Value::Array(Vec::new()),
                Some(value) => json_array(value).ok_or(InvalidMetadata)?,
            },
            ohai_versions: match object.get("ohai_versions") {
                None => Value::Array(Vec::new()),
                Some(value) => json_array(value).ok_or(InvalidMetadata)?,
            },
        };

        // Upstream transparently drops a self-dependency instead of erroring.
        metadata.dependencies.remove(&metadata.name);
        Ok(metadata)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_empty() {
        for document in [
            serde_json::json!({}),
            serde_json::json!([]),
            serde_json::json!(null),
        ] {
            let metadata = Metadata::from_json(&document).unwrap();
            assert_eq!(metadata, Metadata::default());
            assert!(!metadata.privacy);
        }
    }

    #[test]
    fn values_are_coerced() {
        let metadata = Metadata::from_json(&serde_json::json!({
            "name": "redis",
            "version": "1.0.0",
            "privacy": "true",
            "platforms": { "debian": ">= 0.0.0" },
            "unknown": 1,
        }))
        .unwrap();
        assert_eq!(metadata.name, "redis");
        assert!(metadata.privacy);
        assert_eq!(metadata.platforms["debian"], ">= 0.0.0");
    }

    #[test]
    fn self_dependencies_are_dropped() {
        let metadata = Metadata::from_json(&serde_json::json!({
            "name": "redis",
            "dependencies": { "redis": ">= 0.0.0", "apt": "~> 1.0" },
        }))
        .unwrap();
        assert_eq!(metadata.dependencies.len(), 1);
        assert!(metadata.dependencies.contains_key("apt"));
    }

    #[test]
    fn wrong_shapes_are_invalid() {
        for document in [
            serde_json::json!([1]),
            serde_json::json!("text"),
            serde_json::json!({ "platforms": "debian" }),
            serde_json::json!({ "privacy": "maybe" }),
            serde_json::json!({ "chef_versions": {} }),
        ] {
            assert_eq!(Metadata::from_json(&document), Err(InvalidMetadata));
        }
    }
}
