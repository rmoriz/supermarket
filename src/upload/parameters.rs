//! Port of `CookbookUpload::Parameters`: validates the multipart parts and
//! extracts the category, metadata, README and CHANGELOG.
//!
//! Error order and wording are upstream's, including the odd bits: the README
//! extractor reports a damaged archive as "must be GZipped" (it rescues the
//! whole `Archive::Error` family first), and duplicates collapse because the
//! messages live in a `Set`.

use crate::locales;
use crate::upload::archive::{Archive, ArchiveError};
use crate::upload::document::Document;
use crate::upload::metadata::Metadata;

pub struct Parameters<'a> {
    cookbook_data: &'a [u8],
    archive: Archive<'a>,
}

impl<'a> Parameters<'a> {
    /// `params.fetch(:cookbook)` / `params.fetch(:tarball)`; a missing part is
    /// the controller's business, not ours.
    pub fn new(cookbook_data: &'a [u8], tarball: &'a [u8], tarball_is_file: bool) -> Self {
        Self {
            cookbook_data,
            archive: Archive::new(tarball, tarball_is_file),
        }
    }

    /// The `category` from the cookbook JSON. Invalid JSON yields `""`.
    pub fn category_name(&self) -> String {
        match self.cookbook_json() {
            Some(document) => document.get("category").map(stringify).unwrap_or_default(),
            None => String::new(),
        }
    }

    /// The parsed `metadata.json`, or empty defaults when the tarball has
    /// problems.
    pub fn metadata(&self) -> Metadata {
        let (errors, metadata) = self.tarball_metadata();
        if errors.is_empty() {
            metadata
        } else {
            Metadata::default()
        }
    }

    /// The cookbook's README, or an empty document.
    pub fn readme(&self) -> Document {
        let (errors, readme) = self.tarball_readme(&self.metadata().name);
        if errors.is_empty() {
            readme.unwrap_or_default()
        } else {
            Document::default()
        }
    }

    /// The cookbook's CHANGELOG, or an empty document.
    pub fn changelog(&self) -> Document {
        let (errors, changelog) = self.tarball_changelog(&self.metadata().name);
        if errors.is_empty() {
            changelog
        } else {
            Document::default()
        }
    }

    /// Every distinct error, in upstream's collection order.
    pub fn errors(&self) -> Vec<String> {
        let mut messages: Vec<String> = Vec::new();
        let mut push = |message: String| {
            if !messages.contains(&message) {
                messages.push(message);
            }
        };

        if self.cookbook_json().is_none() {
            push(locales::t("api.error_messages.cookbook_not_json", &[]));
        }

        let (metadata_errors, metadata) = self.tarball_metadata();
        metadata_errors.into_iter().for_each(&mut push);

        let (readme_errors, _) = self.tarball_readme(&metadata.name);
        readme_errors.into_iter().for_each(&mut push);

        messages
    }

    fn cookbook_json(&self) -> Option<serde_json::Value> {
        serde_json::from_slice(self.cookbook_data).ok()
    }

    fn tarball_metadata(&self) -> (Vec<String>, Metadata) {
        let pattern = regex::Regex::new(r"\A(\./)?[^/]+/metadata\.json\z").unwrap();
        let path = match self.archive.find(&pattern) {
            Ok(paths) => paths.into_iter().next(),
            Err(error) => return (vec![metadata_error(&error)], Metadata::default()),
        };

        let Some(path) = path else {
            return (
                vec![locales::t("api.error_messages.missing_metadata", &[])],
                Metadata::default(),
            );
        };

        let contents = match self.archive.read(&path) {
            Ok(contents) => contents.unwrap_or_default(),
            Err(error) => return (vec![metadata_error(&error)], Metadata::default()),
        };

        let document: serde_json::Value = match serde_json::from_str(&contents) {
            Ok(document) => document,
            Err(_) => {
                return (
                    vec![locales::t("api.error_messages.metadata_not_json", &[])],
                    Metadata::default(),
                )
            }
        };

        match Metadata::from_json(&document) {
            Ok(metadata) => (Vec::new(), metadata),
            Err(_) => (
                vec![locales::t("api.error_messages.invalid_metadata", &[])],
                Metadata::default(),
            ),
        }
    }

    fn tarball_readme(&self, cookbook: &str) -> (Vec<String>, Option<Document>) {
        let path = match self.find_document(cookbook, "readme") {
            Ok(Some(path)) => path,
            Ok(None) => {
                return (
                    vec![locales::t("api.error_messages.missing_readme", &[])],
                    None,
                )
            }
            Err(errors) => return (errors, None),
        };

        let contents = match self.archive.read(&path) {
            Ok(contents) => contents.unwrap_or_default(),
            Err(error) => return (vec![readme_error(&error)], None),
        };

        if contents.trim().is_empty() {
            return (
                vec![locales::t("api.error_messages.missing_readme", &[])],
                None,
            );
        }

        (
            Vec::new(),
            Some(Document {
                contents: Some(contents),
                extension: extension(&path),
            }),
        )
    }

    fn tarball_changelog(&self, cookbook: &str) -> (Vec<String>, Document) {
        let path = match self.find_document(cookbook, "changelog") {
            Ok(Some(path)) => path,
            Ok(None) => return (Vec::new(), Document::default()),
            Err(errors) => return (errors, Document::default()),
        };

        let contents = match self.archive.read(&path) {
            Ok(contents) => contents.unwrap_or_default(),
            Err(error) => return (vec![readme_error(&error)], Document::default()),
        };

        (
            Vec::new(),
            Document {
                contents: Some(contents),
                extension: extension(&path),
            },
        )
    }

    /// The first entry named `<name>/readme` (case-insensitive, optional
    /// extension, optional `./`). The name is interpolated raw, as upstream
    /// does, so regex metacharacters in cookbook names behave identically.
    fn find_document(&self, cookbook: &str, kind: &str) -> Result<Option<String>, Vec<String>> {
        let pattern = format!(r"\A(\./)?{cookbook}/{kind}(\.\w+)?\z");
        let Ok(pattern) = regex::RegexBuilder::new(&pattern)
            .case_insensitive(true)
            .build()
        else {
            return Ok(None);
        };
        match self.archive.find(&pattern) {
            Ok(mut paths) => Ok(paths.drain(..).next()),
            Err(error) => Err(vec![readme_error(&error)]),
        }
    }
}

fn stringify(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

/// `File.extname(path)[1..-1].to_s`.
fn extension(path: &str) -> String {
    let base = path.rsplit('/').next().unwrap_or("");
    match base.rfind('.') {
        None | Some(0) => String::new(),
        Some(index) => base[index + 1..].to_owned(),
    }
}

fn metadata_error(error: &ArchiveError) -> String {
    match error {
        ArchiveError::NoPath => locales::t("api.error_messages.tarball_has_no_path", &[]),
        ArchiveError::NotGzipped => locales::t("api.error_messages.tarball_not_gzipped", &[]),
        ArchiveError::CorruptTarball | ArchiveError::Other(_) => locales::t(
            "api.error_messages.tarball_corrupt",
            &[(
                "error",
                "#<CookbookUpload::Archive::CorruptTarball: Damaged tar archive>",
            )],
        ),
    }
}

/// The README/CHANGELOG extractors rescue `Archive::Error` first, which covers
/// a damaged archive too.
fn readme_error(error: &ArchiveError) -> String {
    match error {
        ArchiveError::NoPath => locales::t("api.error_messages.tarball_has_no_path", &[]),
        _ => locales::t("api.error_messages.tarball_not_gzipped", &[]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::upload::archive::fixtures::tarball;

    fn errors(cookbook: &[u8], tarball: &[u8], is_file: bool) -> Vec<String> {
        Parameters::new(cookbook, tarball, is_file).errors()
    }

    #[test]
    fn a_healthy_upload_is_valid() {
        let tarball = tarball(&[
            (
                "redis/metadata.json",
                br#"{"name":"redis","version":"1.0.0"}"#,
            ),
            ("redis/README.md", b"# hi"),
            ("redis/CHANGELOG.md", b"changes"),
        ]);
        let params = Parameters::new(br#"{"category":"Cool"}"#, &tarball, true);
        assert_eq!(params.errors(), Vec::<String>::new());
        assert_eq!(params.category_name(), "Cool");
        assert_eq!(params.metadata().name, "redis");
        assert_eq!(params.readme().contents.as_deref(), Some("# hi"));
        assert_eq!(params.readme().extension, "md");
        assert_eq!(params.changelog().contents.as_deref(), Some("changes"));
    }

    #[test]
    fn an_absent_category_is_an_empty_string() {
        let tarball = tarball(&[("redis/metadata.json", b"{}")]);
        let params = Parameters::new(b"{}", &tarball, true);
        assert_eq!(params.category_name(), "");
    }

    #[test]
    fn invalid_cookbook_json_is_reported() {
        let tarball = tarball(&[("redis/README", b"x")]);
        let errors = errors(b"ack!", &tarball, true);
        assert!(errors
            .contains(&"Multipart POST part 'cookbook' must be a valid JSON string.".to_owned()));
    }

    #[test]
    fn a_non_file_tarball_is_not_a_path_but_still_parses_json() {
        let errors = errors(b"ack!", b"tarball!", false);
        assert_eq!(
            errors,
            vec![
                "Multipart POST part 'cookbook' must be a valid JSON string.".to_owned(),
                "Multipart POST part 'tarball' must be a file.".to_owned(),
            ]
        );
    }

    #[test]
    fn a_plain_string_is_not_gzipped() {
        let errors = errors(b"{}", b"tarball!", true);
        assert_eq!(
            errors,
            vec!["Multipart POST part 'tarball' must be GZipped.".to_owned()]
        );
    }

    #[test]
    fn missing_metadata_and_readme_are_distinct() {
        let errors = errors(b"{}", &tarball(&[]), true);
        assert_eq!(
            errors,
            vec![
                "Multipart POST part 'tarball' must contain a metadata.json entry.".to_owned(),
                "Multipart POST part 'tarball' must contain a non-empty README.".to_owned(),
            ]
        );
    }

    #[test]
    fn a_blank_readme_counts_as_missing() {
        let tarball = tarball(&[("redis/metadata.json", b"{}"), ("redis/README.md", b"  \n")]);
        assert_eq!(
            errors(b"{}", &tarball, true),
            vec!["Multipart POST part 'tarball' must contain a non-empty README.".to_owned()]
        );
    }

    #[test]
    fn metadata_must_be_json_of_the_right_shape() {
        // Without usable metadata the name is unknown, so the README can never
        // match either; upstream reports both.
        let not_json = tarball(&[("redis/metadata.json", b"nope")]);
        assert_eq!(
            errors(b"{}", &not_json, true),
            vec![
                "Tarball entry named 'metadata.json' could not be parsed as JSON.".to_owned(),
                "Multipart POST part 'tarball' must contain a non-empty README.".to_owned(),
            ]
        );

        let wrong_shape = tarball(&[("redis/metadata.json", b"[1]")]);
        assert_eq!(
            errors(b"{}", &wrong_shape, true),
            vec![
                "The metadata is formatted incorrectly.".to_owned(),
                "Multipart POST part 'tarball' must contain a non-empty README.".to_owned(),
            ]
        );
    }

    #[test]
    fn a_damaged_archive_is_corrupt_for_metadata_and_not_gzipped_for_the_readme() {
        let mut data = tarball(&[("redis/metadata.json", b"{}"), ("redis/README", b"x")]);
        data.truncate(data.len() / 2);
        assert_eq!(
            errors(b"{}", &data, true),
            vec![
                "Multipart POST part 'tarball' is corrupt: #<CookbookUpload::Archive::CorruptTarball: Damaged tar archive>".to_owned(),
                "Multipart POST part 'tarball' must be GZipped.".to_owned(),
            ]
        );
    }

    #[test]
    fn readme_and_changelog_match_case_insensitively_and_ignore_dot_slash() {
        let tarball = tarball(&[
            ("./redis/metadata.json", br#"{"name":"redis"}"#),
            ("./redis/ReadMe.markdown", b"hi"),
            ("./redis/ChangeLog", b"log"),
        ]);
        let params = Parameters::new(b"{}", &tarball, true);
        assert_eq!(params.errors(), Vec::<String>::new());
        assert_eq!(params.readme().extension, "markdown");
        assert_eq!(params.changelog().extension, "");
    }

    #[test]
    fn extensions_follow_file_extname() {
        assert_eq!(extension("redis/README.md"), "md");
        assert_eq!(extension("redis/README"), "");
        assert_eq!(extension("redis/.README"), "");
        assert_eq!(extension("redis/README."), "");
    }
}
