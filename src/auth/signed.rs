//! Port of the `mixlib-authentication` 3.0.10 signed-header protocol, which is
//! what knife/clients use to prove possession of the RSA key stored
//! on the account. Verified against Ruby-generated vectors in
//! `harness/fixtures/signed_requests.json` (regenerate with
//! `harness/bin/signed_vectors`).
//!
//! Check order is upstream's exactly: mandatory headers, signature, timestamp
//! skew, content hash. Only a missing header is distinguished from a bad one.

use axum::http::{header, HeaderMap};
use base64::Engine;
use chrono::{DateTime, Utc};
use num_bigint::BigUint;
use rsa::pkcs1::DecodeRsaPublicKey;
use rsa::pkcs8::DecodePublicKey;
use rsa::traits::PublicKeyParts;
use rsa::RsaPublicKey;
use sha1::{Digest as _, Sha1};
use sha2::Sha256;

/// Upstream's mandatory headers, in its own order and naming.
const MANDATORY: [(&str, &str); 5] = [
    ("x-ops-sign", "X_OPS_SIGN"),
    ("x-ops-userid", "X_OPS_USERID"),
    ("x-ops-timestamp", "X_OPS_TIMESTAMP"),
    ("host", "HOST"),
    ("x-ops-content-hash", "X_OPS_CONTENT_HASH"),
];

/// `Mixlib::Authentication::DEFAULT_SERVER_API_VERSION`.
const DEFAULT_SERVER_API_VERSION: &str = "0";

pub const TIME_SKEW_SECONDS: i64 = 15 * 60;

#[derive(Debug, PartialEq, Eq)]
pub enum SignedError {
    /// `MissingAuthenticationHeader`: the controller answers
    /// `authentication_request_error`.
    MissingHeaders(String),
    /// Anything that makes the request fail verification: the controller
    /// answers `authentication_key_error`.
    Invalid,
}

/// Everything verification needs, borrowed from the live request.
pub struct SignedRequest<'a> {
    pub method: &'a str,
    pub path: &'a str,
    pub body: &'a [u8],
    /// The `tarball` part, when the body is multipart. The hash is over this,
    /// not over the multipart envelope.
    pub file: Option<&'a [u8]>,
    pub headers: &'a HeaderMap,
    pub public_key: &'a str,
    pub now: DateTime<Utc>,
}

fn header_value<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|value| value.to_str().ok())
}

fn hash_hex_free(algorithm: DigestKind, bytes: &[u8]) -> String {
    let digest = match algorithm {
        DigestKind::Sha1 => {
            let mut hasher = Sha1::new();
            hasher.update(bytes);
            hasher.finalize().to_vec()
        }
        DigestKind::Sha256 => {
            let mut hasher = Sha256::new();
            hasher.update(bytes);
            hasher.finalize().to_vec()
        }
    };
    base64::engine::general_purpose::STANDARD.encode(digest)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DigestKind {
    Sha1,
    Sha256,
}

impl DigestKind {
    fn for_version(version: &str) -> Option<Self> {
        match version {
            "1.0" | "1.1" => Some(Self::Sha1),
            "1.3" => Some(Self::Sha256),
            _ => None,
        }
    }
}

/// `override_path.gsub(%r{/+}, "/")`, then drop a trailing slash unless the
/// path is just `/`.
fn canonical_path(path: &str) -> String {
    let mut collapsed = String::with_capacity(path.len());
    let mut last_slash = false;
    for ch in path.chars() {
        if ch == '/' {
            if last_slash {
                continue;
            }
            last_slash = true;
        } else {
            last_slash = false;
        }
        collapsed.push(ch);
    }
    if collapsed.len() > 1 {
        while collapsed.ends_with('/') {
            collapsed.pop();
        }
    }
    collapsed
}

/// `canonical_time`: `Time.parse(timestamp).utc.iso8601`.
fn canonical_time(timestamp: &str) -> Option<String> {
    let parsed = DateTime::parse_from_rfc3339(timestamp)
        .map(|value| value.with_timezone(&Utc))
        .or_else(|_| {
            chrono::NaiveDateTime::parse_from_str(timestamp, "%Y-%m-%d %H:%M:%S %Z")
                .or_else(|_| chrono::NaiveDateTime::parse_from_str(timestamp, "%Y-%m-%d %H:%M:%S"))
                .map(|value| value.and_utc())
        })
        .ok()?;
    Some(parsed.format("%Y-%m-%dT%H:%M:%SZ").to_string())
}

fn parse_public_key(pem: &str) -> Option<RsaPublicKey> {
    RsaPublicKey::from_public_key_pem(pem)
        .or_else(|_| RsaPublicKey::from_pkcs1_pem(pem))
        .ok()
}

/// RSA `public_decrypt` with PKCS#1 v1.5 type-1 padding, which is what
/// `private_encrypt` on the client produced for versions 1.0 and 1.1.
fn public_decrypt(key: &RsaPublicKey, signature: &[u8]) -> Option<Vec<u8>> {
    // `rsa` uses its own BigUint flavour; go through bytes to keep `num-bigint`.
    let modulus = BigUint::from_bytes_be(&key.n().to_bytes_be());
    let exponent = BigUint::from_bytes_be(&key.e().to_bytes_be());
    let modulus_len = modulus.bits().div_ceil(8) as usize;
    if signature.len() != modulus_len {
        return None;
    }
    let message = BigUint::from_bytes_be(signature).modpow(&exponent, &modulus);
    let mut bytes = message.to_bytes_be();
    if bytes.len() < modulus_len {
        let mut padded = vec![0u8; modulus_len - bytes.len()];
        padded.extend_from_slice(&bytes);
        bytes = padded;
    }
    if bytes.first() != Some(&0) || bytes.get(1) != Some(&1) {
        return None;
    }
    let separator = bytes.iter().skip(2).position(|byte| *byte == 0)? + 2;
    Some(bytes[separator + 1..].to_vec())
}

/// Canonical block for the given version. Mirrors
/// `SignedHeaderAuth#canonicalize_request`.
#[allow(clippy::too_many_arguments)]
fn canonical_block(
    method: &str,
    path: &str,
    content_hash: &str,
    timestamp: &str,
    user_id: &str,
    server_api_version: &str,
    version: &str,
    digest: DigestKind,
    body_hash: &str,
) -> String {
    let path = canonical_path(path);
    let timestamp = canonical_time(timestamp).unwrap_or_default();
    if version == "1.3" {
        format!(
            "Method:{}\nPath:{}\nX-Ops-Content-Hash:{}\nX-Ops-Sign:version={}\n\
             X-Ops-Timestamp:{}\nX-Ops-UserId:{}\nX-Ops-Server-API-Version:{}",
            method.to_uppercase(),
            path,
            content_hash,
            version,
            timestamp,
            user_id,
            server_api_version,
        )
    } else {
        let user = if version == "1.1" {
            hash_hex_free(digest, user_id.as_bytes())
        } else {
            user_id.to_owned()
        };
        format!(
            "Method:{}\nHashed Path:{}\nX-Ops-Content-Hash:{}\nX-Ops-Timestamp:{}\nX-Ops-UserId:{}",
            method.to_uppercase(),
            hash_hex_free(digest, path.as_bytes()),
            body_hash,
            timestamp,
            user,
        )
    }
}

/// Reassembles `X-Ops-Authorization-N` numerically and Base64-decodes it.
fn request_signature(headers: &HeaderMap) -> Option<Vec<u8>> {
    let mut lines: Vec<(u32, &str)> = Vec::new();
    for (name, value) in headers {
        let name = name.as_str();
        let Some(index) = name.strip_prefix("x-ops-authorization-") else {
            continue;
        };
        let index: u32 = index.parse().ok()?;
        lines.push((index, value.to_str().ok()?));
    }
    lines.sort_by_key(|(index, _)| *index);
    let joined = lines
        .into_iter()
        .map(|(_, value)| value)
        .collect::<Vec<_>>()
        .join("\n");
    // `Base64.decode64` ignores the newlines between the header lines.
    let compact: String = joined.split_whitespace().collect();
    base64::engine::general_purpose::STANDARD
        .decode(compact)
        .ok()
}

/// Parses `algorithm=sha1;version=1.0;`.
fn signing_description(headers: &HeaderMap) -> Option<(Option<String>, String)> {
    let description = header_value(headers, "x-ops-sign")?;
    let mut algorithm = None;
    let mut version = None;
    for part in description.trim().split(';') {
        let mut fields = part.split('=');
        let name = fields.next().unwrap_or("").trim();
        let Some(value) = fields.next() else { continue };
        let value = value.trim();
        match name {
            "algorithm" => algorithm = Some(value.to_owned()),
            "version" => version = Some(value.to_owned()),
            _ => {}
        }
    }
    Some((algorithm, version?))
}

/// Verifies the request. `Ok(())` means signature, timestamp and body hash all
/// matched.
pub fn verify(request: &SignedRequest<'_>) -> Result<(), SignedError> {
    let missing: Vec<&str> = MANDATORY
        .iter()
        .filter(|(name, _)| header_value(request.headers, name).is_none())
        .map(|(_, upper)| *upper)
        .collect();
    if !missing.is_empty() {
        return Err(SignedError::MissingHeaders(format!(
            "missing required authentication header(s) '{}'",
            missing.join("', '")
        )));
    }

    // All headers are present now; parse cannot fail on presence.
    let (algorithm, version) = signing_description(request.headers).ok_or(SignedError::Invalid)?;
    let algorithm = algorithm.unwrap_or_else(|| "sha1".to_owned());

    let valid_signature = verify_signature(request, &algorithm, &version).unwrap_or(false);
    let valid_timestamp = verify_timestamp(request).unwrap_or(false);
    let valid_content_hash = verify_content_hash(request, &version).unwrap_or(false);

    if valid_signature && valid_timestamp && valid_content_hash {
        Ok(())
    } else {
        Err(SignedError::Invalid)
    }
}

fn digest_for(algorithm: &str, version: &str) -> Option<DigestKind> {
    if !matches!(version, "1.0" | "1.1" | "1.3") {
        return None;
    }
    let expected = if version == "1.3" { "sha256" } else { "sha1" };
    if algorithm == expected {
        DigestKind::for_version(version)
    } else {
        None
    }
}

fn hashed_body(request: &SignedRequest<'_>, digest: DigestKind) -> String {
    let bytes = request.file.unwrap_or(request.body);
    hash_hex_free(digest, bytes)
}

fn verify_signature(request: &SignedRequest<'_>, algorithm: &str, version: &str) -> Option<bool> {
    let digest = digest_for(algorithm, version)?;
    let key = parse_public_key(request.public_key)?;
    let user_id = header_value(request.headers, "x-ops-userid")?.trim();
    let content_hash = header_value(request.headers, "x-ops-content-hash")?.trim();
    let timestamp = header_value(request.headers, "x-ops-timestamp")?.trim();
    let server_api_version = header_value(request.headers, "x-ops-server-api-version")
        .unwrap_or(DEFAULT_SERVER_API_VERSION)
        .trim()
        .to_owned();

    let body_hash = hashed_body(request, digest);
    let block = canonical_block(
        request.method,
        request.path,
        content_hash,
        timestamp,
        user_id,
        &server_api_version,
        version,
        digest,
        &body_hash,
    );
    let signature = request_signature(request.headers)?;

    if version == "1.3" {
        let verifying = rsa::pkcs1v15::VerifyingKey::<Sha256>::new(key);
        use rsa::signature::Verifier;
        let parsed = rsa::pkcs1v15::Signature::try_from(signature.as_slice()).ok()?;
        // `Verifier::verify` hashes the message itself, matching
        // `OpenSSL::PKey::RSA#verify(digest, sig, data)`.
        Some(verifying.verify(block.as_bytes(), &parsed).is_ok())
    } else {
        Some(
            public_decrypt(&key, &signature)
                .map(|decrypted| decrypted == block.as_bytes())
                .unwrap_or(false),
        )
    }
}

fn verify_timestamp(request: &SignedRequest<'_>) -> Option<bool> {
    let timestamp = header_value(request.headers, "x-ops-timestamp")?.trim();
    let parsed = DateTime::parse_from_rfc3339(timestamp)
        .map(|value| value.with_timezone(&Utc))
        .or_else(|_| {
            chrono::NaiveDateTime::parse_from_str(timestamp, "%Y-%m-%d %H:%M:%S %Z")
                .or_else(|_| chrono::NaiveDateTime::parse_from_str(timestamp, "%Y-%m-%d %H:%M:%S"))
                .map(|value| value.and_utc())
        })
        .ok()?;
    Some((request.now - parsed).num_seconds().abs() < TIME_SKEW_SECONDS)
}

fn verify_content_hash(request: &SignedRequest<'_>, version: &str) -> Option<bool> {
    let header = header_value(request.headers, "x-ops-content-hash")?.trim();
    let digest = DigestKind::for_version(version)?;
    Some(header == hashed_body(request, digest))
}

#[allow(dead_code)]
fn _assert_header_names(_: &header::HeaderName) {}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;
    use serde_json::Value;

    struct Case {
        name: String,
        expect: String,
        key: String,
        method: String,
        path: String,
        body: Vec<u8>,
        file: Option<Vec<u8>>,
        now: DateTime<Utc>,
        headers: HeaderMap,
    }

    fn cases() -> Vec<Case> {
        let raw = std::fs::read_to_string("harness/fixtures/signed_requests.json").unwrap();
        let document: Value = serde_json::from_str(&raw).unwrap();
        document["cases"]
            .as_array()
            .unwrap()
            .iter()
            .map(|case| {
                let mut headers = HeaderMap::new();
                for (name, value) in case["headers"].as_object().unwrap() {
                    headers.insert(
                        header::HeaderName::from_bytes(name.to_lowercase().as_bytes()).unwrap(),
                        HeaderValue::from_str(value.as_str().unwrap()).unwrap(),
                    );
                }
                // The fixtures were generated "now"; pin the clock to the
                // generation-time timestamp so the skew cases stay meaningful.
                Case {
                    name: case["name"].as_str().unwrap().to_owned(),
                    expect: case["actual"].as_str().unwrap().to_owned(),
                    key: case["public_key"].as_str().unwrap().to_owned(),
                    method: case["method"].as_str().unwrap().to_owned(),
                    path: case["path"].as_str().unwrap().to_owned(),
                    body: case["body"].as_str().unwrap().as_bytes().to_vec(),
                    file: case["file"].as_str().map(|value| value.as_bytes().to_vec()),
                    // Pinned to generation time so the skew cases stay meaningful.
                    now: DateTime::parse_from_rfc3339(case["now"].as_str().unwrap())
                        .unwrap()
                        .with_timezone(&Utc),
                    headers,
                }
            })
            .collect()
    }

    #[test]
    fn vectors_match_upstream() {
        for case in cases() {
            let result = verify(&SignedRequest {
                method: &case.method,
                path: &case.path,
                body: &case.body,
                file: case.file.as_deref(),
                headers: &case.headers,
                public_key: &case.key,
                now: case.now,
            });
            let actual = match &result {
                Ok(()) => "ok".to_owned(),
                Err(SignedError::Invalid) => "invalid".to_owned(),
                Err(SignedError::MissingHeaders(message)) => {
                    format!("Mixlib::Authentication::MissingAuthenticationHeader: {message}")
                }
            };
            if actual != case.expect {
                let request = SignedRequest {
                    method: &case.method,
                    path: &case.path,
                    body: &case.body,
                    file: None,
                    headers: &case.headers,
                    public_key: &case.key,
                    now: case.now,
                };
                panic!(
                    "case {}: actual={} desc={:?} sig={:?} ts={:?} ch={:?}",
                    case.name,
                    actual,
                    signing_description(&case.headers),
                    verify_signature(&request, "sha1", "1.0"),
                    verify_timestamp(&request),
                    verify_content_hash(&request, "1.0"),
                );
            }
        }
    }

    #[test]
    fn duplicate_and_trailing_slashes_collapse() {
        assert_eq!(canonical_path("//api//v1/cookbooks/"), "/api/v1/cookbooks");
        assert_eq!(canonical_path("/"), "/");
    }
}
