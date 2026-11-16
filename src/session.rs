//! Rails-compatible encrypted session cookie.
//!
//! Rails 7.1 (`load_defaults 7.1`) uses authenticated cookie encryption:
//! key = PBKDF2-HMAC-SHA1(secret_key_base, "authenticated encrypted cookie", 1000, 32),
//! AES-256-GCM with an empty AAD, value layout `base64(ct)--base64(iv)--base64(tag)`,
//! plaintext `{"_rails":{"pur":"cookie.<name>"},"data":<session>}`.
use std::collections::BTreeMap;

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use pbkdf2::pbkdf2_hmac;
use rand::RngCore;
use serde_json::{Map, Value};
use sha2::Sha256;

pub const COOKIE_NAME: &str = "_supermarket_session";
/// Rails' `AUTHENTICITY_TOKEN_LENGTH`.
pub const CSRF_BYTES: usize = 32;
const SALT: &[u8] = b"authenticated encrypted cookie";
const ITERATIONS: u32 = 1000;
const KEY_LEN: usize = 32;

#[derive(Debug, PartialEq, Eq)]
pub struct SessionError(&'static str);

impl std::fmt::Display for SessionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}

impl std::error::Error for SessionError {}

#[derive(Debug, Clone, Default)]
pub struct Session {
    data: Map<String, Value>,
}

impl Session {
    pub fn new(csrf: String) -> Self {
        let mut data = Map::new();
        data.insert("csrf".into(), Value::String(csrf));
        Session { data }
    }

    pub fn from_map(data: Map<String, Value>) -> Self {
        Session { data }
    }

    pub fn user_id(&self) -> Option<i32> {
        self.data
            .get("user_id")
            .and_then(Value::as_i64)
            .and_then(|id| i32::try_from(id).ok())
    }

    pub fn set_user_id(&mut self, id: i32) {
        self.data.insert("user_id".into(), Value::from(id));
    }

    pub fn remove_user_id(&mut self) {
        self.data.remove("user_id");
    }

    pub fn set_flash(&mut self, kind: &str, message: &str) {
        let entry = self
            .data
            .entry("flash")
            .or_insert_with(|| Value::Object(Map::new()));
        if let Value::Object(map) = entry {
            map.insert(kind.into(), Value::String(message.into()));
        }
    }

    pub fn csrf(&self) -> &str {
        self.data.get("csrf").and_then(Value::as_str).unwrap_or("")
    }

    pub fn return_to(&self) -> Option<&str> {
        self.data.get("return_to").and_then(Value::as_str)
    }

    pub fn set_return_to(&mut self, path: &str) {
        self.data
            .insert("return_to".into(), Value::String(path.into()));
    }

    pub fn set_oauth_state(&mut self, value: &str) {
        self.data
            .insert("omniauth.state".into(), Value::String(value.to_owned()));
    }

    pub fn take_return_to(&mut self) -> Option<String> {
        self.data
            .remove("return_to")
            .and_then(|v| v.as_str().map(str::to_owned))
    }

    pub fn data(&self) -> &Map<String, Value> {
        &self.data
    }

    /// Session hash in Rails' JSON shape, `BTreeMap` so output is deterministic.
    pub fn to_json_map(&self) -> BTreeMap<&str, &Value> {
        self.data.iter().map(|(k, v)| (k.as_str(), v)).collect()
    }
}

pub fn derive_key(secret_key_base: &str) -> [u8; KEY_LEN] {
    let mut key = [0u8; KEY_LEN];
    pbkdf2_hmac::<Sha256>(secret_key_base.as_bytes(), SALT, ITERATIONS, &mut key);
    key
}

pub fn purpose(cookie_name: &str) -> String {
    format!("cookie.{cookie_name}")
}

fn cipher(secret_key_base: &str) -> Aes256Gcm {
    Aes256Gcm::new_from_slice(&derive_key(secret_key_base)).expect("32-byte key")
}

/// The real CSRF token derived from the session (Rails keeps one random value
/// per session; ours is stored as an arbitrary string, so it is hashed to the
/// 32 bytes the masking format requires).
pub fn real_csrf(session: &Session) -> Vec<u8> {
    if let Some(encoded) = session.data.get("_csrf_token").and_then(Value::as_str) {
        if let Ok(raw) = B64.decode(encoded) {
            return raw;
        }
    }
    use sha2::{Digest, Sha256};
    Sha256::digest(session.csrf().as_bytes()).to_vec()
}

/// `Base64.strict_encode64(pad ^ token)` — Rails' masked authenticity token.
pub fn mask_token(token: &[u8]) -> String {
    let mut pad = vec![0u8; token.len()];
    rand::thread_rng().fill_bytes(&mut pad);
    // `one_time_pad + xor(one_time_pad, token)`, split over two strings so the
    // unmasked tail is byte-for-byte the raw token.
    let mut out = pad.clone();
    out.extend(pad.iter().zip(token).map(|(byte, key)| byte ^ key));
    B64.encode(out)
}

/// Unmasks a token minted by `mask_token`; `None` on any malformed input.
pub fn unmask_token(encoded: &str) -> Option<Vec<u8>> {
    let masked = B64.decode(encoded).ok()?;
    if masked.len() != CSRF_BYTES * 2 {
        return None;
    }
    let (pad, token) = masked.split_at(CSRF_BYTES);
    Some(pad.iter().zip(token).map(|(a, b)| a ^ b).collect())
}

pub fn encode(session: &Session, secret_key_base: &str) -> String {
    encode_named(session, COOKIE_NAME, secret_key_base)
}

pub fn encode_named(session: &Session, cookie_name: &str, secret_key_base: &str) -> String {
    // Rails 7.1 serializes message metadata with the message serializer:
    // {"_rails":{"message":b64(json),"exp":null,"pur":...}}
    let inner = serde_json::to_string(&session.data).expect("session json");
    let envelope = format!(
        r#"{{"_rails":{{"message":"{}","exp":null,"pur":"{}"}}}}"#,
        B64.encode(inner),
        purpose(cookie_name)
    );
    let plaintext = envelope;

    let mut iv = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut iv);
    let sealed = cipher(secret_key_base)
        .encrypt(
            Nonce::from_slice(&iv),
            Payload {
                msg: plaintext.as_bytes(),
                aad: b"",
            },
        )
        .expect("encrypt");
    let (ct, tag) = sealed.split_at(sealed.len() - 16);
    // aes-gcm appends the 16-byte tag; Rails splits ct and tag into separate parts.
    let tag: [u8; 16] = tag.try_into().expect("16-byte tag");
    format!(
        "{}--{}--{}",
        B64.encode(ct),
        B64.encode(iv),
        B64.encode(tag)
    )
}

pub fn decode(raw: &str, secret_key_base: &str) -> Result<Session, SessionError> {
    decode_named(raw, COOKIE_NAME, secret_key_base)
}

pub fn decode_named(
    raw: &str,
    cookie_name: &str,
    secret_key_base: &str,
) -> Result<Session, SessionError> {
    let mut parts = raw.split("--");
    let (Some(ct), Some(iv), Some(tag), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(SessionError("invalid_message_format"));
    };

    let ct = B64
        .decode(ct)
        .map_err(|_| SessionError("invalid_message_format"))?;
    let iv = B64
        .decode(iv)
        .map_err(|_| SessionError("invalid_message_format"))?;
    let tag = B64
        .decode(tag)
        .map_err(|_| SessionError("invalid_message_format"))?;
    if iv.len() != 12 || tag.len() != 16 || ct.is_empty() {
        return Err(SessionError("invalid_message_format"));
    }

    let mut sealed = ct;
    sealed.extend_from_slice(&tag);
    let plaintext = cipher(secret_key_base)
        .decrypt(
            Nonce::from_slice(&iv),
            Payload {
                msg: &sealed,
                aad: b"",
            },
        )
        .map_err(|_| SessionError("invalid_message_verification"))?;

    let envelope: Value = serde_json::from_slice(&plaintext)
        .map_err(|_| SessionError("invalid_message_serialization"))?;

    let wanted = purpose(cookie_name);
    let rails = envelope
        .get("_rails")
        .ok_or(SessionError("missing metadata"))?;
    if rails.get("pur").and_then(Value::as_str).unwrap_or("") != wanted {
        return Err(SessionError("mismatched purpose"));
    }

    // Expiry, when present, is enforced the same way Rails does.
    if let Some(exp) = rails.get("exp").and_then(Value::as_str) {
        if let Ok(expiry) = chrono::DateTime::parse_from_rfc3339(exp) {
            if chrono::Utc::now() >= expiry.with_timezone(&chrono::Utc) {
                return Err(SessionError("expired"));
            }
        }
    }

    // Rails 7.1+: base64 JSON under `_rails.message`; older formats put the
    // payload straight into `data`.
    let data = match rails.get("message").and_then(Value::as_str) {
        Some(message) => {
            let decoded = B64
                .decode(message)
                .map_err(|_| SessionError("invalid_message_serialization"))?;
            serde_json::from_slice::<Value>(&decoded)
                .map_err(|_| SessionError("invalid_message_serialization"))?
        }
        None => envelope.get("data").cloned().unwrap_or_default(),
    };
    let data = data.as_object().cloned().unwrap_or_default();
    Ok(Session::from_map(data))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    struct Vectors {
        secret: String,
        vectors: Vec<(Value, String)>,
        negatives: Vec<String>,
    }

    fn vectors() -> Vectors {
        let raw = std::fs::read_to_string("harness/fixtures/cookies.json").expect("fixtures");
        let doc: Value = serde_json::from_str(&raw).unwrap();
        Vectors {
            secret: doc["secret_key_base"].as_str().unwrap().to_owned(),
            vectors: doc["vectors"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| {
                    (
                        v["session"].clone(),
                        v["cookie"].as_str().unwrap().to_owned(),
                    )
                })
                .collect(),
            negatives: doc["negatives"]
                .as_array()
                .unwrap()
                .iter()
                .map(|n| n["cookie"].as_str().unwrap().to_owned())
                .collect(),
        }
    }

    #[test]
    fn decodes_every_rails_vector() {
        let v = vectors();
        for (session, cookie) in &v.vectors {
            let decoded = decode(cookie, &v.secret).expect("decodes");
            assert_eq!(serde_json::to_value(decoded.data()).unwrap(), *session);
        }
    }

    #[test]
    fn rejects_every_negative_vector() {
        let v = vectors();
        for cookie in &v.negatives {
            assert!(decode(cookie, &v.secret).is_err(), "accepted {cookie}");
        }
    }

    #[test]
    fn roundtrips_own_output() {
        let v = vectors();
        let mut session = Session::new("token".into());
        session.set_user_id(9);
        let cookie = encode(&session, &v.secret);
        let decoded = decode(&cookie, &v.secret).unwrap();
        assert_eq!(decoded.user_id(), Some(9));
        assert_eq!(decoded.csrf(), "token");
    }

    #[test]
    fn masks_and_unmasks_a_token_like_rails() {
        let token = [7u8; CSRF_BYTES];
        let masked = mask_token(&token);
        // `one_time_pad + xor(one_time_pad, token)` — first half is the pad.
        let raw = B64.decode(&masked).unwrap();
        assert_eq!(
            &raw[..CSRF_BYTES],
            &raw[CSRF_BYTES..].iter().map(|b| b ^ 7).collect::<Vec<_>>()[..]
        );
        assert_eq!(unmask_token(&masked).unwrap(), token);
        assert!(unmask_token("").is_none());
        assert!(unmask_token("c2hvcnQ=").is_none());
    }

    #[test]
    fn wrong_purpose_is_rejected() {
        let v = vectors();
        let cookie = encode_named(&Session::new("t".into()), "_other", &v.secret);
        assert!(decode(&cookie, &v.secret).is_err());
    }

    #[test]
    fn session_json_shape_matches_rails() {
        let mut session = Session::from_map(serde_json::from_value(json!({"user_id": 3})).unwrap());
        session.set_return_to("/x");
        assert_eq!(
            serde_json::to_string(&session.to_json_map()).unwrap(),
            r#"{"return_to":"/x","user_id":3}"#
        );
    }
}
