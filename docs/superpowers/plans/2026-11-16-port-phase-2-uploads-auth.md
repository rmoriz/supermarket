# Phase 2 — uploads and API auth

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans.

**Goal:** `POST /api/v1/cookbooks`, `DELETE /api/v1/cookbooks/:cookbook` and `DELETE /api/v1/cookbooks/:cookbook/versions/:version` behave byte-identically to Rails, including the signed-header protocol, every multipart failure mode, the metadata validation matrix and the authorization outcomes.

**Architecture:** `src/auth/signed.rs` is a direct port of `mixlib-authentication` 3.0.10 (the version in `Gemfile.lock`), verified against Ruby-generated vectors. `src/upload/` mirrors `CookbookUpload` and `CookbookUpload::Parameters` one module per upstream file. Handlers stay thin: authenticate → authorize → parse → publish.

**Tech Stack:** `axum` multipart, `flate2` + `tar`, `rsa` (PKCS#1 v1.5 verify and raw `public_decrypt`), `sha1`/`sha2`.

**Spec:** `docs/superpowers/specs/2026-11-16-supermarket-rs-port-design.md`

## Global constraints

- Three request signing versions all verify: 1.0/1.1 (public decrypt, SHA-1 content hash) and 1.3 (RSA verify over the canonical block, SHA-256). Unknown versions/algorithms fail.
- Order of checks is upstream's: mandatory headers → signature → timestamp (15 min skew) → content hash. A missing header is `AUTHENTICATION_FAILED` / `authentication_request_error`, not a 400.
- The unknown-username check happens before the signature check, and the missing-key check before it too, each with its own message.
- Authorization is `CookbookAuthorizer`: `create?` is owner-or-collaborator; `destroy?` is admin unless `OWNERS_CAN_REMOVE_ARTIFACTS=true`; `destroy_version` reuses `destroy?` and adds the "only version" 409 **before** the authorization failure is rescued.
- Extraction order inside `Parameters#errors` is a `Set`: cookbook JSON, metadata, readme — duplicates collapse.
- Jobs (`CookbookNotifyWorker`, `FieriNotifyWorker`, `SpdxLicenseUpdateWorker`, `CookbookDeletionWorker`) are enqueued onto the `default` Sidekiq queue with identical class names and payloads; the corpus pins the payloads.
- Nothing ports without its corpus entry in the same commit.

## Review Focus

- Timestamp `Time.parse` acceptance, skew boundary at exactly 900 s, and a signature over the *tarball file* rather than the multipart body.
- `X-Ops-Authorization-*` reassembly is sorted numerically, joined with `"\n"`, and Base64-decoded — not in header order.
- `canonical_path` collapses `//`, drops a trailing slash; `version 1.1` hashes the user id.
- Both the cookbook JSON and the tarball are required, and a missing `cookbook` or `tarball` part is `INVALID_DATA` with the `missing_*` message.
- Self-dependency removal, `./`-prefixed entries, case-insensitive `README`/`CHANGELOG` matching and the exact `tarball_corrupt` text.
- `privacy=true` with `ENFORCE_PRIVACY=true` refuses the upload; `privacy` is still stored otherwise.
- Deleting the only version is a 409 whose body carries an `error` key *and* `error_messages`.

---

### Task 1: Signed-request verifier ✅

**Files:**
- Create: `src/auth/mod.rs`, `src/auth/signed.rs`
- Test: `src/auth/signed.rs`, vectors `harness/fixtures/signed_requests.json`

- [x] **Step 1** Generate vectors from the oracle's `mixlib-authentication`: canonical blocks and full header sets for 1.0, 1.1 and 1.3, valid and each single failure (bad signature, stale timestamp, wrong body hash, missing header, unsupported version).
- [x] **Step 2** Failing tests over the matrix.
- [x] **Step 3** Implement the verifier with a `Verified { user_id }` result.
- [x] **Step 4** Green; commit.

### Task 2: Multipart and archive parsing ✅

**Files:**
- Create: `src/upload/mod.rs`, `src/upload/archive.rs`, `src/upload/metadata.rs`, `src/upload/parameters.rs`
- Test: `src/upload/*`, fixture tarballs built in-test

- [x] **Step 1** Failing tests: not-gzipped, missing path, corrupt archive, missing/empty README, missing metadata, non-JSON metadata, wrong metadata shape, `./` prefixes, embedded path separators.
- [x] **Step 2** Implement `Archive` (find/read) and `Parameters` (errors as an insertion-ordered set; `messages` deduplicated).
- [x] **Step 3** Green; commit.

### Task 3: `POST /api/v1/cookbooks` ✅

**Files:**
- Create: `src/api/v1/cookbook_uploads.rs`
- Test: `tests/api.rs`, corpus `harness/corpus/api_upload.yml`

- [x] **Step 1** Corpus: valid new cookbook, new version of an existing one, duplicate version, missing parts, each parse failure, unauthorized user, missing key, bad signature, stale timestamp, privacy violation, category that does not exist.
- [x] **Step 2** Implement the transaction in upstream's statement order, including the self-dependency strip and platform/dependency import.
- [x] **Step 3** Green; commit.

### Task 4: Deletions ✅

**Files:**
- Modify: `src/api/v1/cookbook_uploads.rs`
- Test: corpus `harness/corpus/api_delete.yml`

- [x] **Step 1** Corpus: owner delete, non-owner delete, admin delete, `OWNERS_CAN_REMOVE_ARTIFACTS=true`, missing cookbook/version, single-version 409.
- [x] **Step 2** Implement both handlers with their distinct error bodies (403 `{}` for versions, 403 `forbidden` for cookbooks).
- [x] **Step 3** Green; commit.

### Task 5: Jobs and cache side effects ✅

**Files:**
- Modify: `src/jobs.rs`, both handlers

- [x] **Step 1** Corpus asserts the exact Sidekiq payloads and that `*-universe` keys are flushed.
- [x] **Step 2** Green; commit.
