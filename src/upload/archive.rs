//! Port of `CookbookUpload::Archive`: locate and read entries of an uploaded
//! gzipped tarball.

use std::io::Read;

#[derive(Debug, PartialEq, Eq)]
pub enum ArchiveError {
    /// The part was not a file.
    NoPath,
    /// Not gzip at all.
    NotGzipped,
    /// Gzipped, but the tar stream is damaged. Carries libarchive's wording.
    CorruptTarball,
    /// Any other libarchive error. Upstream lets these escape as 500s.
    Other(String),
}

pub struct Archive<'a> {
    data: &'a [u8],
    is_file: bool,
}

impl<'a> Archive<'a> {
    /// `is_file` mirrors `source.respond_to?(:path)`: Rack hands a `File` for a
    /// file part and a `String` otherwise.
    pub fn new(data: &'a [u8], is_file: bool) -> Self {
        Self { data, is_file }
    }

    fn gzipped(&self) -> bool {
        self.data.starts_with(&[0x1f, 0x8b])
    }

    /// Every entry, in stream order. `libarchive` would raise on the first
    /// unreadable byte, so this buffers the whole archive.
    fn entries(&self) -> Result<Vec<(String, Vec<u8>)>, ArchiveError> {
        if !self.is_file {
            return Err(ArchiveError::NoPath);
        }
        if !self.gzipped() {
            return Err(ArchiveError::NotGzipped);
        }

        let mut decoded = Vec::new();
        flate2::read::GzDecoder::new(self.data)
            .read_to_end(&mut decoded)
            .map_err(|_| ArchiveError::CorruptTarball)?;

        let mut archive = tar::Archive::new(decoded.as_slice());
        let mut entries = Vec::new();
        let raw = archive
            .entries()
            .map_err(|_| ArchiveError::CorruptTarball)?;
        for entry in raw {
            let mut entry = entry.map_err(|_| ArchiveError::CorruptTarball)?;
            let path = entry
                .path()
                .map(|path| path.to_string_lossy().into_owned())
                .unwrap_or_default();
            let mut contents = Vec::new();
            entry
                .read_to_end(&mut contents)
                .map_err(|_| ArchiveError::CorruptTarball)?;
            entries.push((path, contents));
        }
        Ok(entries)
    }

    /// Paths matching `pattern`, in stream order.
    pub fn find(&self, pattern: &regex::Regex) -> Result<Vec<String>, ArchiveError> {
        Ok(self
            .entries()?
            .into_iter()
            .filter(|(path, _)| pattern.is_match(path))
            .map(|(path, _)| path)
            .collect())
    }

    /// Contents of the first entry whose path is exactly `path`.
    pub fn read(&self, path: &str) -> Result<Option<String>, ArchiveError> {
        Ok(self
            .entries()?
            .into_iter()
            .find(|(candidate, _)| candidate == path)
            .map(|(_, contents)| String::from_utf8_lossy(&contents).into_owned()))
    }
}

#[cfg(test)]
pub mod fixtures {
    use std::io::Write;

    /// Builds a gzipped tar from `(path, contents)` pairs.
    pub fn tarball(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut tar = tar::Builder::new(Vec::new());
        for (path, contents) in entries {
            let mut header = tar::Header::new_gnu();
            header.set_size(contents.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            tar.append_data(&mut header, path, *contents).unwrap();
        }
        let raw = tar.into_inner().unwrap();
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        encoder.write_all(&raw).unwrap();
        encoder.finish().unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn metadata_pattern() -> regex::Regex {
        regex::Regex::new(r"\A(\./)?[^/]+/metadata\.json\z").unwrap()
    }

    #[test]
    fn entries_are_found_and_read() {
        let data = fixtures::tarball(&[
            ("./redis/metadata.json", b"{}"),
            ("redis/README.md", b"# hi"),
        ]);
        let archive = Archive::new(&data, true);
        // The `tar` crate normalizes the leading `./` away; a knife archive
        // reads the same either way.
        assert_eq!(
            archive.find(&metadata_pattern()).unwrap(),
            vec!["redis/metadata.json"]
        );
        assert_eq!(
            archive.read("redis/metadata.json").unwrap().as_deref(),
            Some("{}")
        );
        assert_eq!(
            archive.read("redis/README.md").unwrap().as_deref(),
            Some("# hi")
        );
        assert_eq!(archive.read("nginx/README.md").unwrap(), None);
    }

    #[test]
    fn not_a_file_is_no_path() {
        let data = fixtures::tarball(&[("redis/README.md", b"x")]);
        assert_eq!(
            Archive::new(&data, false).find(&metadata_pattern()),
            Err(ArchiveError::NoPath)
        );
    }

    #[test]
    fn a_plain_string_is_not_gzipped() {
        assert_eq!(
            Archive::new(b"tarball!", true).find(&metadata_pattern()),
            Err(ArchiveError::NotGzipped)
        );
    }

    #[test]
    fn damaged_gzip_is_corrupt() {
        let mut data = fixtures::tarball(&[("redis/README.md", b"x")]);
        data.truncate(data.len() / 2);
        assert!(matches!(
            Archive::new(&data, true).find(&metadata_pattern()),
            Err(ArchiveError::CorruptTarball)
        ));
    }
}
