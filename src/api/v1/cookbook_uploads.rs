//! `Api::V1::CookbookUploadsController`: publish, delete a cookbook, delete a
//! version.
//!
//! Check order is upstream's exactly: required parts, authentication (unknown
//! user, missing key, signature), authorization, then parameter validation.
//! Each stage has its own error body, several of them odd on purpose.

use axum::body::Bytes;
use axum::extract::{OriginalUri, Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::{json, Value};
use sqlx::PgPool;

use cadence::prelude::*;

use crate::api::v1::cookbooks::{cookbook_url, format_timestamp, internal, version_url};
use crate::auth::signed::{self, SignedError, SignedRequest};
use crate::error::ApiError;
use crate::jobs;
use crate::locales::t;
use crate::state::AppState;
use crate::upload::{Metadata, Parameters};

/// One multipart part, already split from the envelope.
struct Part {
    name: String,
    file_name: Option<String>,
    bytes: Vec<u8>,
}

#[derive(Debug)]
struct UploadParts {
    cookbook: Vec<u8>,
    tarball: (Vec<u8>, bool),
}

/// Minimal `multipart/form-data` reader. Rack keeps a string part as a `String`
/// and a part with a filename as an `UploadedFile`; `is_file` carries that
/// distinction into `Archive`. A non-multipart body is read as
/// `application/x-www-form-urlencoded`, which is what Rack would do.
fn multipart_parts(body: &[u8], content_type: &str) -> Result<Vec<Part>, ApiError> {
    let boundary = content_type
        .split(';')
        .map(str::trim)
        .find_map(|piece| piece.strip_prefix("boundary="))
        .map(|value| value.trim_matches('"').to_owned());
    let Some(boundary) = boundary else {
        return Ok(form_urlencoded(body));
    };
    let delimiter = format!("--{boundary}").into_bytes();

    let mut parts = Vec::new();
    let mut position = 0usize;
    while let Some(start) = find_from(body, &delimiter, position) {
        let mut cursor = start + delimiter.len();
        if body.get(cursor..cursor + 2) == Some(b"--") {
            break;
        }
        // Skip CRLF (or a bare LF) after the delimiter.
        if body.get(cursor..cursor + 2) == Some(b"\r\n") {
            cursor += 2;
        } else if body.get(cursor..cursor + 1) == Some(b"\n") {
            cursor += 1;
        }

        let Some(headers_end) = find_from(body, b"\r\n\r\n", cursor) else {
            break;
        };
        let raw_headers = &body[cursor..headers_end];
        let data_start = headers_end + 4;
        let Some(next) = find_from(body, &delimiter, data_start) else {
            break;
        };
        // The CRLF before the next delimiter belongs to the boundary.
        let mut data_end = next;
        if data_end >= 2 && &body[data_end - 2..data_end] == b"\r\n" {
            data_end -= 2;
        }

        let headers = String::from_utf8_lossy(raw_headers);
        let mut name = None;
        let mut file_name = None;
        for line in headers.split("\r\n") {
            let Some((key, value)) = line.split_once(':') else {
                continue;
            };
            if !key.trim().eq_ignore_ascii_case("content-disposition") {
                continue;
            }
            for field in value.split(';') {
                let field = field.trim();
                if let Some(rest) = field.strip_prefix("name=") {
                    name = Some(rest.trim_matches('"').to_owned());
                } else if let Some(rest) = field.strip_prefix("filename=") {
                    file_name = Some(rest.trim_matches('"').to_owned());
                }
            }
        }

        if let Some(name) = name {
            parts.push(Part {
                name,
                file_name,
                bytes: body[data_start..data_end].to_vec(),
            });
        }
        position = next;
    }
    Ok(parts)
}

fn form_urlencoded(body: &[u8]) -> Vec<Part> {
    let text = String::from_utf8_lossy(body);
    text.split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| {
            let (name, value) = pair.split_once('=').unwrap_or((pair, ""));
            Part {
                name: percent_decode(name),
                file_name: None,
                bytes: percent_decode(value).into_bytes(),
            }
        })
        .collect()
}

fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'%' if index + 2 < bytes.len() => {
                match u8::from_str_radix(&value[index + 1..index + 3], 16) {
                    Ok(byte) => {
                        out.push(byte);
                        index += 3;
                    }
                    Err(_) => {
                        out.push(b'%');
                        index += 1;
                    }
                }
            }
            b'+' => {
                out.push(b' ');
                index += 1;
            }
            byte => {
                out.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn find_from(haystack: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if from > haystack.len() {
        return None;
    }
    haystack[from..]
        .windows(needle.len())
        .position(|window| window == needle)
        .map(|offset| from + offset)
}

/// `upload_params`: `params.require(:cookbook)` then `params.require(:tarball)`.
fn upload_parts(parts: Vec<Part>) -> Result<UploadParts, ApiError> {
    let mut cookbook = None;
    let mut tarball = None;
    for part in parts {
        match part.name.as_str() {
            "cookbook" => cookbook = Some(part.bytes),
            "tarball" => tarball = Some((part.bytes, part.file_name.is_some())),
            _ => {}
        }
    }

    let Some(cookbook) = cookbook else {
        return Err(ApiError::InvalidData(vec![t(
            "api.error_messages.missing_cookbook",
            &[],
        )]));
    };
    let Some(tarball) = tarball else {
        return Err(ApiError::InvalidData(vec![t(
            "api.error_messages.missing_tarball",
            &[],
        )]));
    };
    Ok(UploadParts { cookbook, tarball })
}

struct CurrentUser {
    id: i32,
}

fn header_str<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|value| value.to_str().ok())
}

fn base_url(state: &AppState) -> String {
    state.cfg.base_url()
}

/// `authenticate_user!`: unknown user, missing key, then signature.
async fn authenticate(
    state: &AppState,
    headers: &HeaderMap,
    method: &str,
    path: &str,
    body: &[u8],
    file: Option<&[u8]>,
) -> Result<CurrentUser, ApiError> {
    let username = header_str(headers, "x-ops-userid").unwrap_or("");

    let user: Option<(i32, Option<String>)> = sqlx::query_as(
        "SELECT u.id, u.public_key FROM accounts a JOIN users u ON u.id = a.user_id
          WHERE a.provider = 'chef_oauth2' AND a.username = $1",
    )
    .bind(username)
    .fetch_optional(&state.db)
    .await
    .map_err(internal)?;

    let Some((id, public_key)) = user else {
        return Err(ApiError::AuthenticationFailed(vec![t(
            "api.error_messages.invalid_username",
            &[("username", username)],
        )]));
    };

    let Some(public_key) = public_key else {
        return Err(ApiError::AuthenticationFailed(vec![t(
            "api.error_messages.missing_public_key_error",
            &[("current_host", &base_url(state))],
        )]));
    };

    let now = chrono::Utc::now();
    match signed::verify(&SignedRequest {
        method,
        path,
        body,
        file,
        headers,
        public_key: &public_key,
        now,
    }) {
        Ok(()) => Ok(CurrentUser { id }),
        Err(SignedError::MissingHeaders(_)) => Err(ApiError::AuthenticationFailed(vec![t(
            "api.error_messages.authentication_request_error",
            &[],
        )])),
        Err(SignedError::Invalid) => Err(ApiError::AuthenticationFailed(vec![t(
            "api.error_messages.authentication_key_error",
            &[],
        )])),
    }
}

struct CookbookRecord {
    id: i32,
    name: String,
    owner_id: i32,
}

async fn find_cookbook(db: &PgPool, name: &str) -> Result<Option<CookbookRecord>, ApiError> {
    let row: Option<(i32, String, i32)> =
        sqlx::query_as("SELECT id, name, user_id FROM cookbooks WHERE lowercase_name = $1")
            .bind(name.to_lowercase())
            .fetch_optional(db)
            .await
            .map_err(internal)?;
    Ok(row.map(|(id, name, owner_id)| CookbookRecord { id, name, owner_id }))
}

async fn collaborates(db: &PgPool, cookbook_id: i32, user_id: i32) -> Result<bool, ApiError> {
    let count: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM collaborators
          WHERE resourceable_type = 'Cookbook' AND resourceable_id = $1 AND user_id = $2",
    )
    .bind(cookbook_id)
    .bind(user_id)
    .fetch_one(db)
    .await
    .map_err(internal)?;
    Ok(count > 0)
}

async fn can_create(
    state: &AppState,
    existing: Option<&CookbookRecord>,
    user_id: i32,
) -> Result<bool, ApiError> {
    match existing {
        None => Ok(true),
        Some(record) => {
            if record.owner_id == user_id {
                Ok(true)
            } else {
                collaborates(&state.db, record.id, user_id).await
            }
        }
    }
}

fn is_admin(roles_mask: i32) -> bool {
    roles_mask & 1 != 0
}

async fn roles_mask(db: &PgPool, user_id: i32) -> Result<i32, ApiError> {
    let mask: Option<i32> = sqlx::query_scalar("SELECT roles_mask FROM users WHERE id = $1")
        .bind(user_id)
        .fetch_optional(db)
        .await
        .map_err(internal)?;
    Ok(mask.unwrap_or(0))
}

pub async fn create(
    State(state): State<AppState>,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let content_type = header_str(&headers, "content-type").unwrap_or("");
    let parts = multipart_parts(&body, content_type)?;
    let parts = upload_parts(parts)?;

    let (tarball, is_file) = parts.tarball;
    let user = authenticate(
        &state,
        &headers,
        "POST",
        uri.path(),
        &body,
        is_file.then_some(tarball.as_slice()),
    )
    .await?;

    let params = Parameters::new(&parts.cookbook, &tarball, is_file);
    let metadata = params.metadata();

    let existing = find_cookbook(&state.db, &metadata.name).await?;
    if !can_create(&state, existing.as_ref(), user.id).await? {
        return Err(ApiError::Unauthorized(vec![t(
            "api.error_messages.unauthorized_upload_error",
            &[],
        )]));
    }

    // `CookbookUpload#errors`: parameter errors first, then the category.
    let mut errors = params.errors();
    let category_name = params.category_name();
    let category: Option<i32> = if category_name.is_empty() {
        None
    } else {
        sqlx::query_scalar("SELECT id FROM categories WHERE name = $1")
            .bind(titleize(&category_name))
            .fetch_optional(&state.db)
            .await
            .map_err(internal)?
    };
    if category.is_none() && !category_name.is_empty() {
        errors.push(t(
            "api.error_messages.non_existent_category",
            &[("category_name", &category_name)],
        ));
    }

    if !errors.is_empty() {
        return Err(ApiError::InvalidData(errors));
    }

    let name = metadata.name.clone();
    publish(
        &state, user.id, existing, category, &metadata, &params, &tarball, is_file,
    )
    .await?;

    let body = json!({ "uri": cookbook_url(&base_url(&state), &name) });
    Ok((StatusCode::CREATED, Json(body)).into_response())
}

/// Rails' `titleize`, close enough for category names.
fn titleize(value: &str) -> String {
    value
        .replace('_', " ")
        .split_whitespace()
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => {
                    first.to_uppercase().collect::<String>() + &chars.as_str().to_lowercase()
                }
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn humanize(attribute: &str) -> String {
    let spaced = attribute.replace('_', " ");
    let mut chars = spaced.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

struct Validation {
    attribute: &'static str,
    message: String,
}

impl Validation {
    fn new(attribute: &'static str, message: impl Into<String>) -> Self {
        Self {
            attribute,
            message: message.into(),
        }
    }

    fn full(&self) -> String {
        format!("{} {}", humanize(self.attribute), self.message)
    }
}

fn present(value: &str, attribute: &'static str, out: &mut Vec<Validation>) {
    if value.trim().is_empty() {
        out.push(Validation::new(attribute, "can't be blank"));
    }
}

/// `Chef::VersionConstraint#parse` acceptance, for validator parity.
fn valid_version_constraint(value: &str) -> bool {
    let stripped = value; // upstream does not strip
    if !stripped.contains(' ') && stripped.starts_with(|c: char| c.is_ascii_digit()) {
        return crate::chef_version::Version::parse(stripped).is_ok();
    }
    let Some((op, rest)) = split_operator(stripped) else {
        return false;
    };
    if !rest.starts_with(|c: char| c.is_ascii_digit()) {
        return false;
    }
    let _ = op;
    crate::chef_version::Version::parse(rest).is_ok()
}

fn split_operator(value: &str) -> Option<(&'static str, &str)> {
    let (op, rest) = match value.split_once(' ') {
        Some((op, rest)) => (op, rest),
        None => return None,
    };
    let op = match op {
        "<" => "<",
        ">" => ">",
        "<=" => "<=",
        ">=" => ">=",
        "=" => "=",
        "~>" => "~>",
        _ => return None,
    };
    Some((op, rest))
}

/// `URI.parse` plus the allowed-scheme check of `validates :url`.
fn valid_url(value: &str) -> bool {
    if value.contains(char::is_whitespace) {
        return false;
    }
    let Some((scheme, rest)) = value.split_once("://") else {
        return false;
    };
    if !matches!(scheme, "http" | "https") {
        return false;
    }
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    !authority.is_empty()
}

fn constraint_validation(constraint: &str) -> Validation {
    let message = format!(
        "is not a valid Chef version constraint. Chef::Exceptions::InvalidVersionConstraint: '{}'",
        constraint
    );
    Validation::new("version_constraint", message)
}

fn version_validation(version: &str) -> Validation {
    Validation::new(
        "version",
        format!(
            "is not a valid Chef version. Chef::Exceptions::InvalidCookbookVersion: '{}' does not match 'x.y.z' or 'x.y'",
            version
        ),
    )
}

#[allow(clippy::too_many_arguments)]
async fn publish(
    state: &AppState,
    user_id: i32,
    existing: Option<CookbookRecord>,
    category: Option<i32>,
    metadata: &Metadata,
    params: &Parameters<'_>,
    tarball: &[u8],
    is_file: bool,
) -> Result<(), ApiError> {
    use std::env;
    if metadata.privacy && env::var("ENFORCE_PRIVACY").ok().as_deref() == Some("true") {
        return Err(ApiError::InvalidData(vec![t(
            "api.error_messages.privacy_violation",
            &[],
        )]));
    }

    let readme = params.readme();
    let changelog = params.changelog();

    // Dependency cookbook links, resolved before the transaction like upstream.
    let dependency_names: Vec<String> = metadata.dependencies.keys().cloned().collect();
    let mut dependency_cookbooks: Vec<(String, i32)> = Vec::new();
    if !dependency_names.is_empty() {
        let lowercase: Vec<String> = dependency_names.iter().map(|n| n.to_lowercase()).collect();
        let rows: Vec<(String, i32)> = sqlx::query_as(
            "SELECT lowercase_name, id FROM cookbooks WHERE lowercase_name = ANY($1)",
        )
        .bind(&lowercase)
        .fetch_all(&state.db)
        .await
        .map_err(internal)?;
        dependency_cookbooks = rows;
    }

    let mut tx = state.db.begin().await.map_err(internal)?;

    // Cookbook validations, then the version's, at the point Rails autosaves.
    let mut cookbook_errors: Vec<Validation> = Vec::new();
    present(&metadata.name, "name", &mut cookbook_errors);
    if !metadata.name.is_empty() && !valid_cookbook_name(&metadata.name) {
        cookbook_errors.push(Validation::new("name", "is invalid"));
    }
    if let Some(record) = &existing {
        if record.name.to_lowercase() != metadata.name.to_lowercase() {
            // Unreachable in practice (the lookup is by lowercase name).
        }
    } else {
        let taken: Option<i32> =
            sqlx::query_scalar("SELECT id FROM cookbooks WHERE lowercase_name = $1")
                .bind(metadata.name.to_lowercase())
                .fetch_optional(&mut *tx)
                .await
                .map_err(internal)?;
        if taken.is_some() {
            cookbook_errors.push(Validation::new("name", "has already been taken"));
        }
    }
    if !metadata.source_url.is_empty() && !valid_url(&metadata.source_url) {
        cookbook_errors.push(Validation::new("source_url", "is not a valid URL"));
    }
    if !metadata.issues_url.is_empty() && !valid_url(&metadata.issues_url) {
        cookbook_errors.push(Validation::new("issues_url", "is not a valid URL"));
    }
    if !cookbook_errors.is_empty() {
        tx.rollback().await.ok();
        return Err(ApiError::InvalidData(
            cookbook_errors.iter().map(Validation::full).collect(),
        ));
    }

    let mut version_errors: Vec<Validation> = Vec::new();
    present(&metadata.license, "license", &mut version_errors);
    if metadata.license.chars().count() > 255 {
        version_errors.push(Validation::new(
            "license",
            "is too long (maximum is 255 characters)",
        ));
    }
    present(&metadata.description, "description", &mut version_errors);
    let readme_contents = readme.contents.clone().unwrap_or_default();
    present(&readme_contents, "readme", &mut version_errors);
    present(&metadata.version, "version", &mut version_errors);
    if !metadata.version.is_empty() {
        let duplicate: Option<i32> = sqlx::query_scalar(
            "SELECT id FROM cookbook_versions
              WHERE cookbook_id = $1 AND lower(version) = lower($2)",
        )
        .bind(existing.as_ref().map(|record| record.id))
        .bind(&metadata.version)
        .fetch_optional(&mut *tx)
        .await
        .map_err(internal)?;
        if duplicate.is_some() {
            version_errors.push(Validation::new("version", "has already been taken"));
        }
        if crate::chef_version::Version::parse(&metadata.version).is_err() {
            version_errors.push(version_validation(&metadata.version));
        }
    }
    if !version_errors.is_empty() {
        tx.rollback().await.ok();
        return Err(ApiError::InvalidData(
            version_errors.iter().map(Validation::full).collect(),
        ));
    }

    // Platform validation happens as each platform is saved.
    for (name, constraint) in &metadata.platforms {
        let mut errors = Vec::new();
        present(name, "name", &mut errors);
        if !valid_version_constraint(constraint) {
            errors.push(constraint_validation(constraint));
        }
        if !errors.is_empty() {
            tx.rollback().await.ok();
            return Err(ApiError::InvalidData(
                errors.iter().map(Validation::full).collect(),
            ));
        }
    }
    for (name, constraint) in &metadata.dependencies {
        let mut errors = Vec::new();
        present(name, "name", &mut errors);
        if !valid_version_constraint(constraint) {
            errors.push(constraint_validation(constraint));
        }
        if !errors.is_empty() {
            tx.rollback().await.ok();
            return Err(ApiError::InvalidData(
                errors.iter().map(Validation::full).collect(),
            ));
        }
    }

    let now = chrono::Local::now().naive_utc();
    let cookbook_id = match &existing {
        Some(record) => {
            sqlx::query(
                "UPDATE cookbooks
                    SET name = $2, updated_at = $3, privacy = $4,
                        source_url = COALESCE($5, source_url),
                        issues_url = COALESCE($6, issues_url)
                  WHERE id = $1",
            )
            .bind(record.id)
            .bind(&metadata.name)
            .bind(now)
            .bind(metadata.privacy)
            .bind((!metadata.source_url.is_empty()).then_some(metadata.source_url.clone()))
            .bind((!metadata.issues_url.is_empty()).then_some(metadata.issues_url.clone()))
            .execute(&mut *tx)
            .await
            .map_err(internal)?;
            record.id
        }
        None => {
            let id: i32 = sqlx::query_scalar(
                "INSERT INTO cookbooks
                   (name, lowercase_name, user_id, category_id, privacy, source_url, issues_url,
                    created_at, updated_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $8)
                 RETURNING id",
            )
            .bind(&metadata.name)
            .bind(metadata.name.to_lowercase())
            .bind(user_id)
            .bind(category)
            .bind(metadata.privacy)
            .bind((!metadata.source_url.is_empty()).then_some(metadata.source_url.clone()))
            .bind((!metadata.issues_url.is_empty()).then_some(metadata.issues_url.clone()))
            .bind(now)
            .fetch_one(&mut *tx)
            .await
            .map_err(internal)?;
            id
        }
    };

    let (version_id, legacy_id): (i32, Option<i32>) = sqlx::query_as(
        "INSERT INTO cookbook_versions
           (cookbook_id, user_id, license, version, description, readme, readme_extension,
            changelog, changelog_extension, chef_versions, ohai_versions,
            tarball_file_name, tarball_content_type, tarball_file_size, tarball_updated_at,
            created_at, updated_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $16)
         RETURNING id, legacy_id",
    )
    .bind(cookbook_id)
    .bind(user_id)
    .bind(&metadata.license)
    .bind(&metadata.version)
    .bind(&metadata.description)
    .bind(&readme_contents)
    .bind(&readme.extension)
    .bind(changelog.contents)
    .bind(&changelog.extension)
    .bind(&metadata.chef_versions)
    .bind(&metadata.ohai_versions)
    .bind(tarball_file_name(
        &metadata.name,
        &metadata.version,
        is_file,
    ))
    .bind("application/gzip")
    .bind(tarball.len() as i64)
    .bind(now)
    .bind(now)
    .fetch_one(&mut *tx)
    .await
    .map_err(internal)?;

    for (name, constraint) in &metadata.platforms {
        let platform_id: i32 = sqlx::query_scalar(
            "INSERT INTO supported_platforms (name, version_constraint, created_at, updated_at)
             VALUES ($1, $2, $3, $3)
             ON CONFLICT (name, version_constraint) DO UPDATE SET name = EXCLUDED.name
             RETURNING id",
        )
        .bind(name)
        .bind(constraint)
        .bind(now)
        .fetch_one(&mut *tx)
        .await
        .map_err(internal)?;
        sqlx::query(
            "INSERT INTO cookbook_version_platforms (cookbook_version_id, supported_platform_id)
             VALUES ($1, $2)",
        )
        .bind(version_id)
        .bind(platform_id)
        .execute(&mut *tx)
        .await
        .map_err(internal)?;
    }

    for (name, constraint) in &metadata.dependencies {
        let linked = dependency_cookbooks
            .iter()
            .find(|(lowercase, _)| *lowercase == name.to_lowercase())
            .map(|(_, id)| *id);
        sqlx::query(
            "INSERT INTO cookbook_dependencies
               (name, version_constraint, cookbook_version_id, cookbook_id, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $5)",
        )
        .bind(name)
        .bind(constraint)
        .bind(version_id)
        .bind(linked)
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(internal)?;
    }

    tx.commit().await.map_err(internal)?;

    if let Some(file_name) = tarball_file_name(&metadata.name, &metadata.version, is_file) {
        write_tarball(state, version_id, legacy_id, tarball, &file_name).await?;
    }

    jobs::perform_async(state, "CookbookNotifyWorker", vec![json!(version_id)])
        .await
        .ok();
    if state.cfg.feature_active("fieri") && state.cfg.fieri_url.is_some() {
        jobs::perform_async(state, "FieriNotifyWorker", vec![json!(version_id)])
            .await
            .ok();
    }
    let _ = state.statsd.count("cookbook.version.published", 1);
    jobs::flush_universe(state).await;
    jobs::perform_async(
        state,
        "SpdxLicenseUpdateWorker",
        vec![spdx_license_json(), json!(version_id)],
    )
    .await
    .ok();

    Ok(())
}

fn valid_cookbook_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// Paperclip's `:basename.:extension`; the archive must have been named.
fn tarball_file_name(name: &str, version: &str, is_file: bool) -> Option<String> {
    is_file.then(|| format!("{name}-{version}.tgz"))
}

/// `FetchSpdxLicenseJson.spdx_license_json`: the bundled SPDX list, verbatim.
pub fn spdx_license_json() -> Value {
    static LICENSES: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
    LICENSES
        .get_or_init(|| {
            let raw = include_str!(
                "../../../supermarket-ruby/src/supermarket/app/assets/data/licenses.json"
            );
            let document: Value = serde_json::from_str(raw).expect("bundled SPDX list");
            document["licenses"].clone()
        })
        .clone()
}

/// Paperclip storage: `public/system/...` by default,
/// `<bucket>/[S3_PATH/]cookbook_versions/tarballs/<legacy_id||id>/original/` when
/// S3 is configured (`config/initializers/paperclip.rb`).
async fn write_tarball(
    state: &AppState,
    version_id: i32,
    legacy_id: Option<i32>,
    bytes: &[u8],
    file_name: &str,
) -> Result<(), ApiError> {
    let id = legacy_id.unwrap_or(version_id);
    match state.s3.as_deref() {
        Some(s3) => s3
            .put(
                &s3.tarball_key(&id.to_string(), file_name),
                bytes.to_vec(),
                "application/gzip",
            )
            .await
            .map_err(internal),
        None => {
            let directory = std::path::Path::new("public/system/cookbook_versions/tarballs")
                .join(id.to_string())
                .join("original");
            if std::fs::create_dir_all(&directory).is_ok() {
                let _ = std::fs::write(directory.join(file_name), bytes);
            }
            Ok(())
        }
    }
}

/// Paperclip's `flush_deletes`: drop the stored artifact. Missing objects and
/// transport errors are ignored, exactly like the S3 branch upstream.
async fn delete_tarball(state: &AppState, id: i32, file_name: Option<&str>) {
    let Some(file_name) = file_name else {
        return;
    };
    match state.s3.as_deref() {
        Some(s3) => s3.delete(&s3.tarball_key(&id.to_string(), file_name)).await,
        None => {
            let _ = std::fs::remove_file(
                std::path::Path::new("public/system/cookbook_versions/tarballs")
                    .join(id.to_string())
                    .join("original")
                    .join(file_name),
            );
            let _ = std::fs::remove_dir_all(
                std::path::Path::new("public/system/cookbook_versions/tarballs")
                    .join(id.to_string()),
            );
        }
    }
}

pub async fn destroy(
    State(state): State<AppState>,
    OriginalUri(uri): OriginalUri,
    Path(cookbook): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let Some(record) = find_cookbook(&state.db, &cookbook).await? else {
        return Err(ApiError::NotFound);
    };
    let user = authenticate(&state, &headers, "DELETE", uri.path(), &body, None)
        .await?
        .id;

    // `destroy?`: admins only, unless owners keep the right.
    let admin = is_admin(roles_mask(&state.db, user).await?);
    let owner_allowed = std::env::var("OWNERS_CAN_REMOVE_ARTIFACTS").ok().as_deref()
        == Some("true")
        && record.owner_id == user;
    if !(admin || owner_allowed) {
        return Err(ApiError::Forbidden(vec![t(
            "api.error_messages.unauthorized_destroy_error",
            &[],
        )]));
    }

    // `@latest_cookbook_version_url` is read before the rows disappear; the
    // view body is built from the still-live records.
    let body = cookbook_body(&state, record.id).await?;
    let snapshot = cookbook_as_json(&state, record.id).await?;
    // `dependent: :destroy` on `cookbook_versions` runs paperclip's delete hooks;
    // collect the keys before the rows disappear.
    let artifacts: Vec<(i32, Option<String>)> = sqlx::query_as(
        "SELECT id, tarball_file_name FROM cookbook_versions WHERE cookbook_id = $1",
    )
    .bind(record.id)
    .fetch_all(&state.db)
    .await
    .map_err(internal)?;

    delete_cookbook(&state, record.id).await?;
    for (id, file_name) in &artifacts {
        delete_tarball(&state, *id, file_name.as_deref()).await;
    }

    jobs::perform_async(&state, "CookbookDeletionWorker", vec![snapshot])
        .await
        .ok();
    let _ = state.statsd.count("cookbook.deleted", 1);
    jobs::flush_universe(&state).await;

    Ok(Json(body).into_response())
}

/// `@cookbook.destroy`: dependent rows first, then the cookbook itself.
async fn delete_cookbook(state: &AppState, id: i32) -> Result<(), ApiError> {
    let mut tx = state.db.begin().await.map_err(internal)?;
    let dependent: [&str; 6] = [
        "DELETE FROM metric_results WHERE cookbook_version_id IN
           (SELECT id FROM cookbook_versions WHERE cookbook_id = $1)",
        "DELETE FROM cookbook_dependencies WHERE cookbook_version_id IN
           (SELECT id FROM cookbook_versions WHERE cookbook_id = $1)",
        "DELETE FROM cookbook_version_platforms WHERE cookbook_version_id IN
           (SELECT id FROM cookbook_versions WHERE cookbook_id = $1)",
        "DELETE FROM cookbook_versions WHERE cookbook_id = $1",
        "DELETE FROM cookbook_followers WHERE cookbook_id = $1",
        "DELETE FROM group_resources WHERE resourceable_type = 'Cookbook' AND resourceable_id = $1",
    ];
    for statement in dependent {
        sqlx::query(statement)
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(internal)?;
    }
    sqlx::query(
        "DELETE FROM collaborators WHERE resourceable_type = 'Cookbook' AND resourceable_id = $1",
    )
    .bind(id)
    .execute(&mut *tx)
    .await
    .map_err(internal)?;
    sqlx::query("UPDATE cookbooks SET replacement_id = NULL WHERE replacement_id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(internal)?;
    sqlx::query("DELETE FROM cookbooks WHERE id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    Ok(())
}

/// `_cookbook.json.jbuilder`, used as the deletion response.
#[derive(sqlx::FromRow)]
struct DeletionView {
    name: String,
    maintainer: String,
    category: Option<String>,
    source_url: Option<String>,
    issues_url: Option<String>,
    description: Option<String>,
    created_at: chrono::NaiveDateTime,
    updated_at: chrono::NaiveDateTime,
    up_for_adoption: Option<bool>,
    latest: Option<String>,
}

async fn cookbook_body(state: &AppState, cookbook_id: i32) -> Result<Value, ApiError> {
    let row: Option<DeletionView> = sqlx::query_as(
        "SELECT c.name AS name, a.username AS maintainer, cat.name AS category,
                c.source_url AS source_url, c.issues_url AS issues_url,
                v.description AS description, c.created_at AS created_at,
                c.updated_at AS updated_at, c.up_for_adoption AS up_for_adoption,
                (SELECT cv.version FROM cookbook_versions cv
                  WHERE cv.cookbook_id = c.id ORDER BY cv.id DESC LIMIT 1) AS latest
           FROM cookbooks c
           JOIN users u ON u.id = c.user_id
           JOIN accounts a ON a.user_id = u.id AND a.provider = 'chef_oauth2'
           LEFT JOIN categories cat ON cat.id = c.category_id
           LEFT JOIN LATERAL (
                SELECT cv.description FROM cookbook_versions cv
                 WHERE cv.cookbook_id = c.id ORDER BY cv.id DESC LIMIT 1
           ) v ON true
          WHERE c.id = $1",
    )
    .bind(cookbook_id)
    .fetch_optional(&state.db)
    .await
    .map_err(internal)?;

    let Some(row) = row else {
        return Err(ApiError::NotFound);
    };
    let base = base_url(state);
    Ok(json!({
        "name": row.name,
        "maintainer": row.maintainer,
        "description": row.description,
        "category": row.category,
        "latest_version": row.latest.map(|version| version_url(&base, &row.name, &version)),
        "external_url": row.source_url,
        "source_url": row.source_url,
        "issues_url": row.issues_url,
        "average_rating": Value::Null,
        "created_at": format_timestamp(row.created_at),
        "updated_at": format_timestamp(row.updated_at),
        "up_for_adoption": row.up_for_adoption,
    }))
}

async fn latest_version(state: &AppState, cookbook_id: i32) -> Result<Option<String>, ApiError> {
    let versions: Vec<String> =
        sqlx::query_scalar("SELECT version FROM cookbook_versions WHERE cookbook_id = $1")
            .bind(cookbook_id)
            .fetch_all(&state.db)
            .await
            .map_err(internal)?;
    crate::chef_version::latest(&versions)
        .map_err(|error| ApiError::InvalidData(vec![error.to_string()]))
}

/// `@cookbook.as_json`, the payload `CookbookDeletionWorker` receives.
#[derive(sqlx::FromRow)]
struct CookbookSnapshot {
    name: String,
    created_at: chrono::NaiveDateTime,
    updated_at: chrono::NaiveDateTime,
    source_url: Option<String>,
    deprecated: bool,
    category_id: Option<i32>,
    lowercase_name: Option<String>,
    issues_url: Option<String>,
    cookbook_followers_count: i32,
    user_id: Option<i32>,
    replacement_id: Option<i32>,
    web_download_count: i32,
    api_download_count: i32,
    featured: bool,
    up_for_adoption: Option<bool>,
    privacy: Option<bool>,
    badges_mask: i32,
    deprecation_reason: Option<String>,
}

async fn cookbook_as_json(state: &AppState, id: i32) -> Result<Value, ApiError> {
    let row: Option<CookbookSnapshot> = sqlx::query_as(
        "SELECT name, created_at, updated_at, source_url, deprecated, category_id,
                lowercase_name, issues_url, cookbook_followers_count, user_id,
                replacement_id, web_download_count, api_download_count, featured,
                up_for_adoption, privacy, badges_mask, deprecation_reason
           FROM cookbooks WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await
    .map_err(internal)?;

    let Some(row) = row else {
        return Err(ApiError::NotFound);
    };

    Ok(json!({
        "id": id,
        "name": row.name,
        "created_at": format_timestamp(row.created_at),
        "updated_at": format_timestamp(row.updated_at),
        "source_url": row.source_url,
        "deprecated": row.deprecated,
        "category_id": row.category_id,
        "lowercase_name": row.lowercase_name,
        "issues_url": row.issues_url,
        "cookbook_followers_count": row.cookbook_followers_count,
        "user_id": row.user_id,
        "replacement_id": row.replacement_id,
        "web_download_count": row.web_download_count,
        "api_download_count": row.api_download_count,
        "featured": row.featured,
        "up_for_adoption": row.up_for_adoption,
        "privacy": row.privacy,
        "badges_mask": row.badges_mask,
        "deprecation_reason": row.deprecation_reason,
    }))
}

pub async fn destroy_version(
    State(state): State<AppState>,
    OriginalUri(uri): OriginalUri,
    Path((cookbook, version)): Path<(String, String)>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response, ApiError> {
    let Some(record) = find_cookbook(&state.db, &cookbook).await? else {
        return Err(ApiError::NotFound);
    };
    let normalised = version.replace('_', ".");
    let wanted = if normalised == "latest" {
        latest_version(&state, record.id).await?
    } else {
        Some(normalised)
    };
    let found: Option<(i32, String, Option<String>)> = sqlx::query_as(
        "SELECT id, version, tarball_file_name FROM cookbook_versions
          WHERE cookbook_id = $1 AND version = $2",
    )
    .bind(record.id)
    .bind(wanted)
    .fetch_optional(&state.db)
    .await
    .map_err(internal)?;
    let Some((version_id, version, tarball_file_name)) = found else {
        return Err(ApiError::NotFound);
    };

    // Authorization is checked before the only-version conflict.
    let user = authenticate(&state, &headers, "DELETE", uri.path(), &body, None)
        .await?
        .id;
    let admin = is_admin(roles_mask(&state.db, user).await?);
    let owner_allowed = std::env::var("OWNERS_CAN_REMOVE_ARTIFACTS").ok().as_deref()
        == Some("true")
        && record.owner_id == user;
    if !(admin || owner_allowed) {
        // A bare `{}` here, unlike the cookbook deletion's message.
        return Ok((StatusCode::FORBIDDEN, Json(json!({}))).into_response());
    }

    let total: i64 =
        sqlx::query_scalar("SELECT count(*) FROM cookbook_versions WHERE cookbook_id = $1")
            .bind(record.id)
            .fetch_one(&state.db)
            .await
            .map_err(internal)?;
    if total == 1 {
        let message = t("api.error_messages.only_cookbook_version", &[]);
        return Ok((
            StatusCode::CONFLICT,
            Json(json!({
                "error_code": t("api.error_codes.conflict", &[]),
                "error_messages": [message],
                "error": message,
            })),
        )
            .into_response());
    }

    let body = version_body(&state, record.id, version_id, &record.name, &version).await?;
    let mut tx = state.db.begin().await.map_err(internal)?;
    sqlx::query("DELETE FROM metric_results WHERE cookbook_version_id = $1")
        .bind(version_id)
        .execute(&mut *tx)
        .await
        .map_err(internal)?;
    sqlx::query("DELETE FROM cookbook_dependencies WHERE cookbook_version_id = $1")
        .bind(version_id)
        .execute(&mut *tx)
        .await
        .map_err(internal)?;
    sqlx::query("DELETE FROM cookbook_version_platforms WHERE cookbook_version_id = $1")
        .bind(version_id)
        .execute(&mut *tx)
        .await
        .map_err(internal)?;
    sqlx::query("DELETE FROM cookbook_versions WHERE id = $1")
        .bind(version_id)
        .execute(&mut *tx)
        .await
        .map_err(internal)?;
    tx.commit().await.map_err(internal)?;
    delete_tarball(&state, version_id, tarball_file_name.as_deref()).await;
    jobs::flush_universe(&state).await;

    Ok(Json(body).into_response())
}

async fn version_body(
    state: &AppState,
    cookbook_id: i32,
    version_id: i32,
    cookbook: &str,
    version: &str,
) -> Result<Value, ApiError> {
    let row: (String, Option<i32>, chrono::NaiveDateTime) = sqlx::query_as(
        "SELECT license, tarball_file_size, created_at FROM cookbook_versions WHERE id = $1",
    )
    .bind(version_id)
    .fetch_one(&state.db)
    .await
    .map_err(internal)?;

    let base = base_url(state);
    let mut body = json!({
        "license": row.0,
        "tarball_file_size": row.1,
        "version": version,
        "published_at": row.2.and_utc().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        "average_rating": Value::Null,
        "cookbook": cookbook_url(&base, cookbook),
        "file": format!("{}/download", version_url(&base, cookbook, version)),
    });

    if state.cfg.feature_active("fieri") {
        let metrics: Vec<(String, bool, Option<String>)> = sqlx::query_as(
            "SELECT qm.name, mr.failure, mr.feedback
               FROM metric_results mr JOIN quality_metrics qm ON qm.id = mr.quality_metric_id
              WHERE mr.cookbook_version_id = $1 ORDER BY mr.id",
        )
        .bind(version_id)
        .fetch_all(&state.db)
        .await
        .map_err(internal)?;
        body.as_object_mut().unwrap().insert(
            "quality_metrics".into(),
            Value::Array(
                metrics
                    .into_iter()
                    .map(|(name, failed, feedback)| {
                        json!({"name": name, "failed": failed, "feedback": feedback})
                    })
                    .collect(),
            ),
        );
    }

    // `dependent: :destroy` runs before the render, so both are empty.
    body.as_object_mut()
        .unwrap()
        .insert("supports".into(), json!({}));
    body.as_object_mut()
        .unwrap()
        .insert("dependencies".into(), json!({}));
    let _ = cookbook_id;
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn multipart_parts_parse_names_and_files() {
        let body = b"--X\r\nContent-Disposition: form-data; name=\"cookbook\"\r\n\r\n{}\r\n--X\r\nContent-Disposition: form-data; name=\"tarball\"; filename=\"x.tgz\"\r\n\r\ndata\r\n--X--\r\n";
        let parts = multipart_parts(body, "multipart/form-data; boundary=X").unwrap();
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0].name, "cookbook");
        assert_eq!(parts[0].bytes, b"{}");
        assert_eq!(parts[0].file_name, None);
        assert_eq!(parts[1].file_name.as_deref(), Some("x.tgz"));
        assert_eq!(parts[1].bytes, b"data");
    }

    #[test]
    fn missing_parts_are_reported_in_order() {
        let parts = multipart_parts(b"--X--\r\n", "multipart/form-data; boundary=X").unwrap();
        let error = upload_parts(parts).unwrap_err();
        assert_eq!(
            error.messages(),
            vec!["Multipart POST must include a part named 'cookbook'.".to_owned()]
        );
    }

    #[test]
    fn titleize_matches_rails() {
        assert_eq!(titleize("database"), "Database");
        assert_eq!(titleize("web servers"), "Web Servers");
    }

    #[test]
    fn constraints_accept_the_same_grammar() {
        assert!(valid_version_constraint(">= 0.0.0"));
        assert!(valid_version_constraint("~> 1.2"));
        assert!(valid_version_constraint("1.2.3"));
        assert!(!valid_version_constraint("abc"));
        assert!(!valid_version_constraint(">= abc"));
        assert!(!valid_version_constraint(""));
    }
}
