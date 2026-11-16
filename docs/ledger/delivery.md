# Delivery ledger — all phases

Date: 2026-11-16

Head at final verification: the single port commit on top of the vendored
import (working tree clean).

Final state: the Rust port answers every route of the reference application
(`harness/routes.tsv`, 119/119), with the unchanged Postgres schema and the
upstream wire format. The differential corpus is green direct and green through
the cutover proxy.

## Phase table

The history was squashed for publication: the port is one commit on top of the
import, so per-phase commit hashes below are the pre-squash ones.

| Phase | Commit | Evidence |
|-------|--------|----------|
| 0 Skeleton, router, harness | first commits | corpus runs end to end; cookie vectors match Ruby both ways |
| 1 Public read API | phase-1 commits | api_read 54/54 |
| 2 Uploads and API auth | phase-2 commits | signed-request vectors, api_upload 17/17, api_delete 10/10 |
| 3 Web UI, read paths | phase-3 commits | ui_read 42/42, atom/robots/error pages |
| 4 Web UI, mutations | phase-4 commits | ui_mutations 57/57 |
| 5 Identity, CLAs, admin | phase-5 commits | ui_identity 43/43, OAuth request phase and callback |
| 6 Background work | phase-6 commits | job observers green; `supermarket-worker` drains the same queues |
| 7 Remaining surfaces | squashed | phase7 47/47 (fieri, quality metrics, tools, chat, robots, feeds) |
| 8 Cutover | squashed | smoke 8/8, routes 119/119, phase8 12/12, proxy flip and shadow smoke |

## Final verification

- `cargo fmt --check` clean.
- `cargo clippy --all-targets -D warnings` clean.
- `cargo test` green (88 lib + 27 integration tests).
Run order for the final record: `cargo test` (which now touches only Redis DB
2), then the corpus direct, then the corpus through the proxy.

- Full corpus, direct (`RUST_URL=http://127.0.0.1:3005`): smoke 8/8, api_read
  54/54, api_delete 10/10, api_upload 17/17, routes 119/119, ui_read 42/42,
  ui_identity 43/43, ui_mutations 57/57, phase7 47/47, phase8 12/12.
- Full corpus through the proxy with every group flipped
  (`RUST_URL=http://127.0.0.1:3002`): the same counts, all green.
- `cargo build --release --bins` clean; both release binaries start.
- SPDX ops task (`rake update_spdx_license:run:*`): `supermarket-worker spdx
  all|latest NAME|version NAME VERSION` prints the same OK/ERROR lines
  (including upstream's `licese` typo and the version-id interpolation) and
  queues the same payloads.
- Worker end-to-end: a queued `SpdxLicenseUpdateWorker` payload is drained by
  `supermarket-worker`, the version's `spdx_license_url` is written, and the
  queue is empty afterwards. `WORKER_ONCE=1` gives a one-shot drain.
- Model callbacks with side effects are reproduced, not just the plain rows:
  `User#after_create` → `default_email_preferences` subscribes a new account to
  every `system_email` with a `SecureRandom.hex` token, and a preference row
  added from the profile form carries one too (`EmailPreference` is
  `Tokenable`). Verified by SQL: three rows for three system emails, each token
  32 hex characters, and a repeat insert is deduped by the unique index.
  `Category#before_save → set_slug` has no write path in the app (categories are
  seeded), so it needs no handler.
- `Tool#copy_name_to_lowercase_name` was found missing on the create path (the
  insert bound the slug into `lowercase_name`); it now binds
  `name.to_lowercase()`, matching the update path and the model callback.

## Brand and copy removal

Upstream's brand words, logos and trademark sentences are gone from every public
surface: page copy, mail bodies and subjects, page metadata, the footer, README
and the docs. `docs/copy-changes.md` lists every swap; the same list lives in
`harness/normalize.rb` (`COPY_SWAPS`), so the parity suite holds the port to it.

- The five brand image files (`supermarket-chef-logo*`, `chef-icon`, `favicon`,
  `apple-touch-icon`, `logo_email`) now carry neutral `Supermarket` artwork under
  their upstream filenames and digests, so CSS, HTML and mail markup stay
  byte-identical.
- Kept on purpose: `Chef` as a person in the adoption mail and the "Chefs"
  count label, `Chef::Exceptions` messages, `is not a valid Chef version`,
  `provider=chef_oauth2`, and anything inside a URL.
- Full corpus green with these swaps, direct and through the proxy (same counts
  as in *Final verification*).

## Chat feature switch

The `/chat` redirects are opt-in: without `chat` in `FEATURES` all three routes
answer the standard 404 page. The chat entries in `harness/corpus/phase7.yml`
and `harness/corpus/routes.yml` assume the switch is on, because the Rails
oracle always redirects; replay them with `chat` in `FEATURES`.

## Test isolation

`tests/api.rs` shares one seeded schema across parallel cases. A download shifts
the download counters and Postgres rewrites the updated heap tuple, which
reorders the unordered `metrics.versions` map (the Rails jbuilder view has the
same non-determinism). The download cases now hold a lock, re-establish the
seeded counters before reading and put them back before asserting, and
`show_body_is_exact` keeps the stable body bytes exact but compares the
volatile version map by key instead of by order.

## Known local-oracle quirks (not port bugs)

- Rails' universe cache is a Marshal entry. A JSON value under the same key from
  another process makes the oracle answer 500, so the tests default to Redis DB 2
  (DB 0 belongs to the oracle) and the seed clears both `*-universe` keys.
- Ruby marshalling skew in the image is why a stale entry surfaces as the 500
  above rather than a miss.

## Remaining production-only work

The flips themselves are operational and follow `docs/cutover-runbook.md`.
Everything the code needs for the switch (proxy, shadow mode, per-group config,
rollback) is in `src/bin/strangler.rs` and smoke-verified.
