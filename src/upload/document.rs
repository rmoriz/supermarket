//! Port of `CookbookUpload::Document`: a README or CHANGELOG entry.

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Document {
    /// `Coercible::String.default(nil)`: nil when the document is absent.
    pub contents: Option<String>,
    pub extension: String,
}
