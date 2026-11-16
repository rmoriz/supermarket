//! Sidekiq 6.5 client-side enqueue, byte-compatible with `perform_async`.
//!
//! Jobs land on the `default` queue exactly as Sidekiq writes them: a JSON
//! payload with `retry`, `queue`, `class`, `args`, `jid`, `created_at` and
//! `enqueued_at`, `SADD queues` and `LPUSH queue:default`. Nothing here runs the
//! job; the existing Sidekiq workers keep draining the queue across the cutover.

use redis::AsyncCommands;
use serde_json::{json, Value};

use crate::state::AppState;

const QUEUE: &str = "default";

/// `SecureRandom.hex(12)`.
fn jid() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut bytes);
    hex::encode(bytes)
}

fn now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or_default()
}

/// Queues `class` with `args`. Key order follows Sidekiq's own construction
/// (defaults first, then the item, then jid/created_at, then enqueued_at).
pub async fn perform_async(
    state: &AppState,
    class: &str,
    args: Vec<Value>,
) -> redis::RedisResult<()> {
    push(state, class, args, None).await
}

/// Pushes `class` with `args`, adding `display_class` when Sidekiq would.
async fn push(
    state: &AppState,
    class: &str,
    args: Vec<Value>,
    display_class: Option<&str>,
) -> redis::RedisResult<()> {
    let created_at = now();
    let mut payload = json!({
        "retry": true,
        "queue": QUEUE,
        "class": class,
        "args": args,
    });
    if let Some(display_class) = display_class {
        payload["display_class"] = json!(display_class);
    }
    payload["jid"] = json!(jid());
    payload["created_at"] = json!(created_at);
    payload["enqueued_at"] = json!(now());
    let encoded = payload.to_string();

    let mut conn = state.redis_jobs.get_multiplexed_async_connection().await?;
    redis::pipe()
        .sadd("queues", QUEUE)
        .lpush(format!("queue:{QUEUE}"), encoded)
        .query_async::<()>(&mut conn)
        .await
}

/// A delayed-mailer argument, YAML-encoded the way Sidekiq's `DelayedMailer`
/// proxy serializes `[mailer, :method, args]`.
pub enum MailerArg {
    Int(i64),
    Value(&'static str),
    Record(RecordArg),
}

/// An ActiveRecord object as Psych's `encode_with` writes it (`concise_attributes`
/// for a persisted row, then the version markers).
pub struct RecordArg {
    pub class: &'static str,
    pub attributes: Vec<(&'static str, RecordValue)>,
}

pub enum RecordValue {
    Null,
    Int(i32),
    Text(String),
    /// `YYYY-MM-DD HH:MM:SS.NNNNNNNNN Z`, rendered with the shared anchors
    /// Psych emits for a repeated time object.
    Timestamp(String),
}

fn render_args(out: &mut String, args: &[MailerArg]) {
    let mut timestamp = 0usize;
    for (index, arg) in args.iter().enumerate() {
        // The third element is a nested argument sequence: `- - first` then
        // `  - second`.
        out.push_str(if index == 0 { "- - " } else { "  - " });
        match arg {
            MailerArg::Int(value) => out.push_str(&value.to_string()),
            MailerArg::Value(value) => out.push_str(value),
            MailerArg::Record(record) => {
                out.push_str(&format!("!ruby/object:{}\n", record.class));
                out.push_str("    concise_attributes:\n");
                for (name, value) in &record.attributes {
                    out.push_str("    - !ruby/object:ActiveModel::Attribute::FromDatabase\n");
                    out.push_str(&format!("      name: {name}\n"));
                    match value {
                        RecordValue::Null => {}
                        RecordValue::Int(value) => {
                            out.push_str(&format!("      value_before_type_cast: {value}\n"))
                        }
                        RecordValue::Text(value) => {
                            out.push_str(&format!("      value_before_type_cast: {value}\n"))
                        }
                        RecordValue::Timestamp(value) => {
                            timestamp += 1;
                            out.push_str("      value_before_type_cast: !ruby/object:ActiveSupport::TimeWithZone\n");
                            if timestamp == 1 {
                                out.push_str(&format!("        utc: &1 {value}\n"));
                                out.push_str(
                                    "        zone: &2 !ruby/object:ActiveSupport::TimeZone\n",
                                );
                                out.push_str("          name: Etc/UTC\n");
                            } else {
                                out.push_str("        utc: *1\n");
                                out.push_str("        zone: *2\n");
                            }
                            out.push_str(&format!("        time: {value}\n"));
                        }
                    }
                }
                out.push_str("    new_record: false\n");
                out.push_str("    active_record_yaml_version: 2");
            }
        }
        out.push('\n');
    }
}

/// `Mailer.method(*args).deliver_later`: Sidekiq's `DelayedMailer` payload.
pub async fn perform_mailer(
    state: &AppState,
    mailer: &str,
    method: &str,
    args: Vec<MailerArg>,
) -> redis::RedisResult<()> {
    let mut yaml = format!("---\n- !ruby/class '{mailer}'\n- :{method}\n");
    render_args(&mut yaml, &args);
    push(
        state,
        "Sidekiq::Extensions::DelayedMailer",
        vec![json!(yaml)],
        Some(&format!("{mailer}.{method}")),
    )
    .await
}

/// `UniverseCache.flush`: both protocol keys, on the cache database.
pub async fn flush_universe(state: &AppState) {
    let mut conn = match state.redis.get_multiplexed_async_connection().await {
        Ok(conn) => conn,
        Err(_) => return,
    };
    let _: redis::RedisResult<()> = conn
        .del(&["http-universe".to_owned(), "https-universe".to_owned()])
        .await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jids_are_24_hex_characters() {
        let value = jid();
        assert_eq!(value.len(), 24);
        assert!(value.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(value, jid());
    }
}
