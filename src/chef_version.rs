//! Port of `Chef::Version` (`lib/chef/version_class.rb` in the chef gem).
//!
//! Only `x.y.z` and `x.y` are accepted; anything else raises and surfaces as a
//! 500 upstream, so the error type is preserved rather than "fixed".

use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
}

#[derive(Debug, PartialEq, Eq)]
pub struct InvalidVersion(pub String);

impl fmt::Display for InvalidVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "'{}' does not match 'x.y.z' or 'x.y'", self.0)
    }
}

impl std::error::Error for InvalidVersion {}

impl Version {
    pub fn parse(raw: &str) -> Result<Version, InvalidVersion> {
        let parts: Vec<&str> = raw.split('.').collect();
        match parts.as_slice() {
            [major, minor, patch] if all_digits(&[major, minor, patch]) => Ok(Version {
                major: major.parse().unwrap(),
                minor: minor.parse().unwrap(),
                patch: patch.parse().unwrap(),
            }),
            [major, minor] if all_digits(&[major, minor]) => Ok(Version {
                major: major.parse().unwrap(),
                minor: minor.parse().unwrap(),
                patch: 0,
            }),
            _ => Err(InvalidVersion(raw.to_owned())),
        }
    }
}

fn all_digits(parts: &[&str]) -> bool {
    parts
        .iter()
        .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// `cookbook_versions.sort_by { Chef::Version.new(v) }.reverse`: newest first.
/// Unparseable versions propagate the upstream error.
pub fn sort_desc(versions: &[String]) -> Result<Vec<String>, InvalidVersion> {
    let mut parsed: Vec<(Version, String)> = versions
        .iter()
        .map(|raw| Ok((Version::parse(raw)?, raw.clone())))
        .collect::<Result<_, InvalidVersion>>()?;
    parsed.sort_by_key(|entry| std::cmp::Reverse(entry.0));
    Ok(parsed.into_iter().map(|(_, raw)| raw).collect())
}

/// The `latest` idiom: the semver-highest version.
pub fn latest(versions: &[String]) -> Result<Option<String>, InvalidVersion> {
    Ok(sort_desc(versions)?.into_iter().next())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn orders_semantically_not_lexically() {
        let versions = ["1.9.0", "1.10.0", "1.2", "2.0.0", "0.9.9"];
        assert_eq!(
            sort_desc(&versions.map(str::to_owned)).unwrap(),
            vec!["2.0.0", "1.10.0", "1.9.0", "1.2", "0.9.9"]
        );
    }

    #[test]
    fn x_y_pads_the_patch() {
        assert_eq!(
            Version::parse("1.2").unwrap(),
            Version::parse("1.2.0").unwrap()
        );
    }

    #[test]
    fn invalid_versions_carry_the_upstream_message() {
        let error = Version::parse("1.0.0-alpha").unwrap_err();
        assert_eq!(
            error.to_string(),
            "'1.0.0-alpha' does not match 'x.y.z' or 'x.y'"
        );
    }

    #[test]
    fn latest_picks_the_highest() {
        let versions = ["1.0.0".to_owned(), "1.0.10".to_owned()];
        assert_eq!(latest(&versions).unwrap().as_deref(), Some("1.0.10"));
    }
}
