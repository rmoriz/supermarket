# supermarket-rs port — master plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans. Each phase has its own plan file; steps use `- [ ]` checkboxes.

**Goal:** Replace the Rails app in `supermarket-ruby/src/supermarket` with a wire-compatible Rust service, phase by phase, each phase ending in a green differential corpus for its slice.

**Architecture:** One `axum`/`sqlx` binary mirroring `config/routes.rb` route-for-route against the unchanged Postgres schema, with a Ruby harness that replays requests against Rails and Rust and diffs them.

**Spec:** `docs/superpowers/specs/2026-11-16-supermarket-rs-port-design.md`

## Global constraints (apply to every phase)

- Schema frozen: `db/schema.rb` is law. No migrations until cutover.
- Response bodies byte-equal after normalization; JSON key order is part of the contract.
- Public copy drops the brand word "Chef" per spec *Copy and naming*; `docs/copy-changes.md` records every swap and the harness applies it to the Rails side before diffing.
- Ported i18n strings live in `config/locales` and are compiled into the binary verbatim; API error codes/messages are never reworded.
- Same env var names, same Redis cache keys (`http-universe`, `https-universe`), same statsd metric names, same Sidekiq queue/job payloads.
- Every route has a `501` stub from phase 0, so the router is complete and the corpus can measure progress.
- No behaviour is ported without its corpus entries landing in the same commit.

## Phase index and gates

| # | Phase | Plan | Gate to next phase |
|---|-------|------|--------------------|
| 0 | Skeleton, router, harness | `2026-11-16-port-phase-0-skeleton-harness.md` | corpus runs end to end; cookie crypto passes Ruby vectors |
| 1 | Public read API | `2026-11-16-port-phase-1-read-api.md` | 100% corpus match on every route in the plan |
| 2 | Uploads and API auth | `2026-11-16-port-phase-2-uploads-auth.md` | signed-request vectors + spec failure matrix green |
| 3 | Web UI, read paths | `2026-11-16-port-phase-3-ui-read.md` | DOM assertions of ported feature specs pass |
| 4 | Web UI, mutations | `2026-11-16-port-phase-4-ui-mutations.md` | follow/deprecate/adopt/transfer corpus green |
| 5 | Identity, CLAs, admin | delivered in `ui_identity` + `phase7` corpora | OAuth round-trip + CLA/groups corpus green |
| 6 | Background work | delivered in `api_upload` + `phase7` corpora | Ruby verifies Rust-enqueued jobs; mailer bodies diff clean |
| 7 | Remaining surfaces | `2026-11-16-port-phase-7-surfaces.md` | fieri/tools/chat/robots/feeds corpus green |
| 8 | Cutover | `2026-11-16-port-phase-8-cutover.md` | shadow diffs clean, then proxy flips, then proxy removal |

**Status 2026-11-16: all phases 0–8 delivered.** Phase ledgers live in `docs/ledger`.

Phases 0–2 must not be reordered. 3 and 4 are sequential (same templates). 5 can start once 3 lands. 6 needs 2 and 5. 7 needs 3. 8 needs all.

## Phase sketches (detail is written per phase, when the phase starts)

- **2 — Uploads/auth:** `mixlib-authentication` verifier (v1.0/1.1/1.3 canonicalization, SHA-1/SHA-256, timestamp skew, canonical path), multipart limits, gzip+tar parsing, `metadata.json` schema, dependency/platform import, category lookup, ownership authorizer, unshare/version delete, job hand-offs, `X-Ops-Userid` → `Account.for("chef_oauth2")` lookup.
- **3 — UI read paths:** layout, CSS/JS as served today, welcome/dashboard, directory with every filter/sort/pagination param, cookbook and version pages, user/group pages, search, error pages, atom/robots, CSRF token issuance.
- **4 — UI mutations:** follow/unfollow, deprecate/undeprecate with replacement validation, adoption, featured toggle, ownership transfer + token accept/decline, collaborators, profile update, install preference.
- **5 — Identity/CLA/admin:** oc-id and GitHub OAuth with linking, unlink, organizations, contributor requests, invitations, ICLA/CCLA signatures, role changes, email preferences, unsubscribe tokens.
- **6 — Background:** notify/deletion/deprecation mailers, OAuth token refresh + cron schedule, sitemap refresh, SPDX fetch, and the Sidekiq compatibility check.
- **7 — Remaining:** fieri engine + quality metric endpoints, chat/IRC logs, tools directory, robots, atom feeds, feature flags.
- **8 — Cutover:** proxy, per-group flips, shadowed `universe`/search, write window, S3 grace period, proxy removal, runbook.

## Global Definition of Done

A phase is done when: every corpus entry for its routes matches Rails byte-for-byte after normalization; the ported feature specs for its pages pass against the Rust host; statsd names and job payloads are verified; `docs/copy-changes.md` covers its copy changes; and the phase's ledger records the green run.
