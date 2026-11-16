# Phase 0 — skeleton, router, harness

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax.

**Goal:** A Rust binary that owns every route from `config/routes.rb` (all returning `501`), speaks Rails session cookies and the API error envelope correctly, and a differential harness that can replay any request against Rails and Rust and diff the results.

**Architecture:** `axum` binary with a complete router, `sqlx` pool, Redis manager, locale table compiled from `config/locales/en.yml`, and a Ruby harness under `harness/` that seeds two databases and diffs both hosts.

**Tech Stack:** Rust 1.85+, `axum` 0.8, `tokio`, `sqlx` 0.8 (postgres, runtime-tokio), `rustls`, `aes-gcm`, `pbkdf2`, `sha1`, `serde`/`serde_json`, `askama` 0.14, `aws-sdk-s3`, `redis`, `tracing`, `cadence` (statsd), `reqwest`; Ruby 3.3 harness with `rspec`-free plain scripts.

**Spec:** `docs/superpowers/specs/2026-11-16-supermarket-rs-port-design.md`

## Global constraints

Inherited from the master plan. Additionally for this phase: no business SQL yet (only `SELECT 1`), no templates beyond a placeholder, and every stub answers the correct content type for its route.

## Review Focus

- A route that exists in `routes.rb` but not in the router (clients get 404 instead of 501, and nobody notices) — pinned by the manifest test.
- Cookie crypto that round-trips Rust↔Rust but not Rails↔Rust (silent logout on cutover) — pinned by Ruby-generated vectors both ways.
- Error envelope drift (wrong `error_code`, missing key order) — pinned by literal-body assertions.
- Env parsing that invents defaults Rails does not have (`API_ITEM_LIMIT`, `FEATURES`, `FQDN`) — pinned by a parity table.
- Harness normalization that hides real differences — pinned by tests where a deliberate mismatch must still fail.

---

### Task 1: Workspace, config, locale table

**Files:**
- Create: `Cargo.toml`, `rust-toolchain.toml`, `src/main.rs`, `src/lib.rs`, `src/config.rs`, `src/locales.rs`, `build.rs`, `.env.example`
- Test: `src/config.rs` (`#[cfg(test)]`), `src/locales.rs` (`#[cfg(test)]`)

**Interfaces:**
- Produces: `config::Config::from_env() -> Result<Config, ConfigError>` with fields `database_url: String`, `redis_url: String`, `secret_key_base: String`, `fqdn: String`, `port: Option<String>`, `api_item_limit: Option<i32>`, `features: Vec<String>`, `s3: Option<S3Config>`, `fieri_url: Option<String>`; `ConfigError` naming the missing var. `locales::t(key: &str, vars: &[(&str, &str)]) -> String` interpolating `%{name}`; `locales::has_key(&str) -> bool`.

- [x] **Step 1: Write the failing config test**

```rust
#[test]
fn required_vars_are_required() {
    let err = Config::from_env_with(&|_| None).unwrap_err();
    assert_eq!(err.var, "DATABASE_URL");
}

#[test]
fn empty_api_item_limit_and_features_are_none_and_empty() {
    let cfg = Config::from_env_with(&fake_env(&[("DATABASE_URL", "pg"), ("SECRET_KEY_BASE", "s"), ("REDIS_URL", "r"), ("FQDN", "x.test"), ("API_ITEM_LIMIT", ""), ("FEATURES", "")]))
        .unwrap();
    assert_eq!(cfg.api_item_limit, None);
    assert!(cfg.features.is_empty());
}
```

- [x] **Step 2: Run `cargo test config` — expect `unresolved import config`.**

- [x] **Step 3: Implement `Config`** with the exact var names from `supermarket-ruby/src/supermarket/.env.example`; `from_env()` delegates to `from_env_with(&std::env::var_os)`. No default values that Rails does not have.

- [x] **Step 4: Write the failing locale test** — `assert_eq!(locales::t("api.error_codes.not_found", &[]), "NOT_FOUND")` and a test that `has_key` is true for every key in a generated list of ≥300 keys (fail count printed).

- [x] **Step 5: Implement `build.rs`** parsing `supermarket-ruby/src/supermarket/config/locales/en.yml` (flat `key.path` → string, `%{x}` kept) into `OUT_DIR/locales.rs`; `locales::t` panics on unknown key, interpolates, leaves unknown `%{x}` intact.

- [x] **Step 6: `cargo test` — all green; commit `feat: config and locale table`.**

### Task 2: Error envelope and content types

**Files:**
- Create: `src/error.rs`, `src/content_type.rs`
- Test: `src/error.rs`

**Interfaces:**
- Produces: `enum ApiError { NotFound, InvalidData(Vec<String>), NegativeParameter{start:String,items:String}, AuthenticationFailed(Vec<String>), Unauthorized(Vec<String>), Forbidden(Vec<String>), Conflict(Vec<String>) }`; `ApiError::status() -> StatusCode`; `ApiError::into_response()` emitting `{"error_code":…,"error_messages":[…]}` with keys in that order.

- [x] **Step 1: Failing test** asserting exact bytes for `NotFound`:

```rust
#[test]
fn not_found_body_is_byte_exact() {
    let body = axum::body::to_bytes(ApiError::NotFound.into_response().into_body(), usize::MAX).await.unwrap();
    assert_eq!(&*body, br#"{"error_messages":["Resource does not exist."],"error_code":"NOT_FOUND"}"#);
}
```

(Key order is asserted literally; it matches `app/views/api/v1_controller.rb#render_404`.)

- [x] **Step 2: Run it — expect failure on missing module.**
- [x] **Step 3: Implement**, pulling message text through `locales::t` so wording never duplicates.
- [x] **Step 4: Green; commit `feat: API error envelope`.**

### Task 3: Full router with 501 stubs

**Files:**
- Create: `src/routes.rs`, `harness/routes.json`
- Test: `src/routes.rs`

**Interfaces:**
- Produces: `routes::router(state: AppState) -> axum::Router`; `routes::stub_handler` returning `501` with `Content-Type: application/json`.
- Consumes: `AppState` from Task 5.

- [x] **Step 1: Generate `harness/routes.json`** from the reference app (`bin/rails routes --expanded` in a booted `supermarket-ruby` dev container, or transcribed from `config/routes.rb` where Rails is unavailable), as `[{"verb":"GET","path":"/api/v1/cookbooks/:cookbook","action":"api/v1/cookbooks#show","format":"json"}]`. All 144 route lines, including redirects and the `/fieri` mount.
- [x] **Step 2: Failing test** `every_manifest_route_is_registered`: for each manifest entry, assert the router matches it (no `404`) — a `tower::ServiceExt::oneshot` sweep that fails listing missing routes.
- [x] **Step 3: Implement the router** in `routes.rb` order. Redirects use `axum::routing` handlers with `Redirect::to`/`StatusCode::FOUND`.
- [x] **Step 4: Second failing test** `every_registered_route_exists_upstream` — the reverse sweep, so no invented routes.
- [x] **Step 5: Green; commit `feat: complete route skeleton`.**

### Task 4: Rails cookie crypto

**Files:**
- Create: `src/session.rs`, `harness/cookie_vectors.rb`, `harness/fixtures/cookies.json`
- Test: `src/session.rs`

**Interfaces:**
- Produces: `session::decode(raw: &str, secret_key_base: &str) -> Result<Session, SessionError>`; `session::encode(session: &Session, secret_key_base: &str) -> String`; `Session { user_id: Option<i32>, csrf: String, session_id: String, return_to: Option<String> }`; cookie name const `_supermarket_session`.
- Crypto: `pbkdf2_hmac::<Sha1>(secret_key_base, b"authenticated encrypted cookie", 1000, 32)` for the key, `aes_256_gcm`, value format `b64(ct)--b64(iv)--b64(tag)` (`base64::engine::general_purpose::STANDARD`), payload JSON, `purpose`/metadata ignored on read.

- [x] **Step 1: Write `harness/cookie_vectors.rb`** that boots nothing, loads only `actionpack`, sets `secret_key_base = "test-secret-…"` and `cookies_serializer = :json`, and writes 20 `{session: {...}, cookie: "<rails cookie value>"}` pairs plus 10 negative cases (bad tag, truncated, wrong secret, `--`-in-payload) to `harness/fixtures/cookies.json`.
- [x] **Step 2: Failing test** `decodes_every_rails_vector` and `rust_encode_is_decodable_by_rails` (shell out to the same Ruby script with `--decode <cookie>`), both against the fixture file.
- [x] **Step 3: Implement `session.rs`** until both directions pass, including the negatives returning `SessionError`.
- [x] **Step 4: Green; commit `feat: Rails-compatible session cookie crypto`.**

### Task 5: App state, Postgres, Redis

**Files:**
- Create: `src/state.rs`, `src/db.rs`, `src/cache.rs`
- Test: `src/db.rs`, `src/cache.rs` (integration, `#[sqlx::test]` and a Redis container from the harness `docker-compose.test.yml`)

**Interfaces:**
- Produces: `struct AppState { db: PgPool, redis: redis::aio::ConnectionManager, cfg: Arc<Config>, http: reqwest::Client, statsd: cadence::StatsdClient }`; `db::connect(&Config) -> Result<PgPool>`; `cache::get_json/set_json/del(pool, key)` matching `Rails.cache` semantics.

- [x] **Step 1: Failing test** `pool_connects_and_selects_one` and `cache_roundtrip_and_delete`.
- [x] **Step 2: Implement** with pool size from `DATABASE_POOL` (default 5).
- [x] **Step 3: Green; commit `feat: app state, db pool, cache`.**

### Task 6: Middleware stack

**Files:**
- Create: `src/middleware.rs`
- Test: `src/middleware.rs`

**Interfaces:**
- Produces: `middleware::stack(router: Router) -> Router` adding request id (`X-Request-Id` echoed), `X-Frame-Options: SAMEORIGIN`, `X-Content-Type-Options: nosniff`, `Referer`-policy header as upstream, gzip compression, 60s timeout, 25 MB body limit, and statsd timing per route.

- [x] **Step 1: Failing tests** for each header and for the body limit (26 MB request → 413).
- [x] **Step 2: Implement; assert header *presence and exact value* against a captured Rails response in `harness/fixtures/headers.json`.**
- [x] **Step 3: Green; commit `feat: response middleware`.**

### Task 7: Harness

**Files:**
- Create: `harness/bin/seed`, `harness/bin/up`, `harness/bin/record`, `harness/bin/replay`, `harness/bin/diff`, `harness/normalize.rb`, `harness/corpus/*.yml`, `harness/docker-compose.test.yml`
- Test: `harness/spec/normalize_spec.rb`, `harness/spec/harness_spec.rb`

**Interfaces:**
- Produces: `replay` taking a corpus file and emitting `{request, rails:{status,headers,body}, rust:{…}, verdict}` JSONL; `diff` reducing to `MATCH`/`DIFF` per entry with the differing region; exit code 1 on any `DIFF`.
- Normalization only: timestamps, integer ids, CSRF tokens, ETag, `Set-Cookie` value, S3 signature params, `docs/copy-changes.md` substitutions, HTML attribute order.

- [x] **Step 1: Failing normalize test** — two bodies differing only in `updated_at` normalize equal; two differing in a cookbook name normalize *unequal*; a body containing `Chef Supermarket` and one containing `Supermarket` normalize equal.
- [x] **Step 2: Implement `normalize.rb`; green.**
- [x] **Step 3: Failing harness test** — boot both hosts with a fixture corpus, get `MATCH` for `/api/v1/health`, `DIFF` for `/api/v1/cookbooks` (Rust still 501); assert exit code 1.
- [x] **Step 4: Implement `up`/`replay`/`diff`; green; commit `feat: differential harness`.**

### Task 8: CI and task runner

**Files:**
- Create: `Makefile`, `.github/workflows/ci.yml`
- Test: none (verified by running the targets)

- [x] **Step 1: `make check`** = `cargo fmt --check && cargo clippy -D warnings && cargo test && harness/bin/replay --corpus harness/corpus/smoke.yml` with services Postgres+Redis from `docker-compose.test.yml`.
- [x] **Step 2: Run it on a clean checkout; it must fail only on the expected 501 diffs, and CI must reproduce that.**
- [x] **Step 3: Commit `chore: CI and task runner`.**

## Phase exit check

`harness/bin/replay --corpus harness/corpus/routes.yml` produces one `DIFF` per stub and zero harness errors; `cargo test` is green including both cookie directions.
