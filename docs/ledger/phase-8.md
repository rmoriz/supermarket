# Phase 8 ledger — cutover

Date: 2026-11-16

## Build

- `cargo build --bin strangler` — clean.
- `cargo clippy --all-targets -- -D warnings` — clean.
- `cargo fmt --check` — clean.

## Proxy flip smoke (local, port 3002)

| Step | Request | Expected | Observed |
|------|---------|----------|----------|
| baseline, `STRANGLER_GROUPS=chat` | `GET /chat` | 200 from Rust | 200 |
| baseline | `GET /api/v1/cookbooks` | 200 from Rails | 200 |
| Rust stopped | `GET /chat` | 502 (upstream down) | 502 |
| Rust stopped | `GET /api/v1/cookbooks` | 200 (Rails still serves) | 200 |
| Rust restarted | `GET /chat` | 200 again | 200 |
| health | `GET /_strangler/health` | `ok` | `ok` |

The group classification and backend selection are correct: only the flipped
group changes backend, and it reverts by config alone.

## Shadow smoke (`STRANGLER_SHADOW=feeds`)

- `/universe` is served by Rails while the Rust answer is fetched in the
  background; the proxy logs the mirror attempt (and `shadow mirror failed`
  when Rust is stopped).
- The oracle image's universe cache can hold a value written by a different
  Ruby Marshal version (`format version 4.8 required; 123.34 given`), which
  makes Rails answer 500. The seed clears both `*-universe` keys, so a fresh
  seed makes Rails regenerate and both sides agree.

## Corpus (full suite, `RUST_URL=http://127.0.0.1:3005`, direct)

| Corpus | Result |
|--------|--------|
| smoke | 8/8 |
| api_read | 54/54 |
| api_delete | 10/10 |
| api_upload | 17/17 |
| routes | 119/119 |
| ui_read | 42/42 |
| ui_identity | 43/43 |
| ui_mutations | 57/57 |
| phase7 | 47/47 |
| phase8 | 12/12 |

## Corpus through the proxy

All groups flipped, release binaries, oracle on 3001 behind nothing, Rust on
3005 behind the proxy on 3002, `RUST_URL=http://127.0.0.1:3002`:

| Corpus | Result |
|--------|--------|
| smoke | 8/8 |
| api_read | 54/54 |
| api_delete | 10/10 |
| api_upload | 17/17 |
| routes | 119/119 |
| ui_read | 42/42 |
| ui_identity | 43/43 |
| ui_mutations | 57/57 |
| phase7 | 47/47 |
| phase8 | 12/12 |

The proxy must not follow backend redirects, or a 302 turns into whatever the
`Location` resolves to (and breaks on `localhost` targets that do not serve the
artifact files). `proxy_client()` builds the HTTP client with
`redirect::Policy::none()`.

## Unit and integration tests

`cargo fmt --check` clean, `cargo clippy --all-targets -D warnings` clean,
`cargo test` green (88 lib + 19 integration tests).

Ordering note: `/api/v1/cookbooks/:cookbook` `metrics.downloads.versions` and
the dashboard's "Your Cookbooks"/"You Collaborate On" lists come from unordered
Associations upstream. Both queries now omit `ORDER BY` so Rails and Rust read
the same Postgres row order. Because the seed used to bump
`cookbook_followers_count` with an `UPDATE` (which moves the row to the end of
the heap), the counter is now inserted inline so heap order stays equal to id
order.

## Last unwired routes (phase-8 addendum)

- `GET /cookbooks/:cookbook/versions/:version/download` (web): bumps the web
  download counters and redirects to the artifact, resolving `latest` through
  the sorted version list like `get_version!`.
- The quality-metric routes with no Rails action (`supported_platforms`,
  `contributing_file`) are served by their real handlers instead of a stub.
- `POST /auth/:provider` (OmniAuth request phase): `SecureRandom.hex(20)` state,
  `Redirecting to <url>...` body, no default security headers or content type,
  CSP and cookie still applied. Unknown provider renders the exception 404 with
  no CSP, no `cache-control` and no session write-back.
- `every_table_route_has_a_handler` now proves no route is left on the 501
  placeholder (the `/auth/{provider}` and `/fieri` expansions excepted).


## Flips

Local only. Production flips follow `docs/cutover-runbook.md`: chat → tools →
feeds → ui → reads → uploads, with `universe`/search shadowed beforehand and
uploads inside an announced write window. Rollback is a config edit.
