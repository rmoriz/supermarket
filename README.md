# supermarket-rs

A Rust port of [Supermarket](https://github.com/chef/supermarket), the community
site for cookbooks. Upstream branding is stripped from everything visitors see —
see [`docs/copy-changes.md`](docs/copy-changes.md) for the full inventory.

`supermarket-ruby/` holds the upstream Ruby on Rails application, imported
verbatim at a pinned commit. It is a **read-only reference**: browse it, grep
it, blame it, but never edit files inside it here. Changes belong upstream or in
the Rust port.

- Provenance, pinned commit, and licence: [`supermarket-ruby/UPSTREAM.md`](supermarket-ruby/UPSTREAM.md)
- Design: [`docs/superpowers/specs/2026-11-16-vendor-supermarket-ruby-design.md`](docs/superpowers/specs/2026-11-16-vendor-supermarket-ruby-design.md)
- Plan: [`docs/superpowers/plans/2026-11-16-vendor-supermarket-ruby.md`](docs/superpowers/plans/2026-11-16-vendor-supermarket-ruby.md)
- Port design: [`docs/superpowers/specs/2026-11-16-supermarket-rs-port-design.md`](docs/superpowers/specs/2026-11-16-supermarket-rs-port-design.md)

## Updating the vendored source

The import is a `git subtree`, so it can be advanced without losing history:

```bash
git remote add upstream-supermarket https://github.com/chef/supermarket  # once per clone
git fetch --no-tags upstream-supermarket
git subtree pull --prefix=supermarket-ruby upstream-supermarket main
```

Then update the pinned commit in `supermarket-ruby/UPSTREAM.md`.

Upstream tags are not fetched (`remote.upstream-supermarket.tagOpt --no-tags`) and
the imported ones were dropped, so tags are the port’s to use.

## Running

The binary needs the Postgres and Redis URLs from the upstream env file:

```bash
cargo build --release
FQDN=localhost PORT=3000 \
DATABASE_URL=postgres://supermarket:supermarket@localhost:55432/supermarket_test \
REDIS_URL=redis://localhost:56379/1 \
SECRET_KEY_BASE=<32+ bytes> \
./target/release/supermarket-rs
```

`/api/v1/health` and `/status` answer without auth.

The chat log redirects under `/chat` are opt-in: name `chat` in the comma list
`FEATURES` to serve them. With it absent — the default — all three routes answer
the normal 404 page.

Background jobs live in `target/release/supermarket-worker` (Sidekiq-compatible
`queue:default`). It also carries the only ops task the app needs,
`supermarket-worker spdx all|latest NAME|version NAME VERSION`, replacing
`rake update_spdx_license:run:*`. Uploads go to `public/system` and need that
directory writable; `S3_BUCKET`/`S3_REGION` are parsed but S3 storage is not
implemented yet (see [`docs/known-gaps.md`](docs/known-gaps.md)).

`REDIS_JOBQ_URL` puts the Sidekiq queue on its own Redis (falls back to
`REDIS_URL`); the universe cache stays on `REDIS_URL`. `STATSD_URL` plus
`STATSD_PORT` enable the counters. `ENABLE_GTAG`/`GOOGLE_ANALYTICS_ID`,
`ENABLE_ONETRUST`/`ONETRUST_ID` and `SEGMENT_WRITE_KEY` add the analytics tags,
and `AIR_GAPPED=true` drops them plus the Google Fonts link. `ALLOWED_HOSTS`
with `DISABLE_HOST_HEADER_ATTACK=true` turns on the host allowlist, and
`CHEF_OAUTH2_VERIFY_SSL=false` accepts a self-signed id server certificate.

## Parity harness

`harness/` holds a differential harness that replays a request corpus against a
running Rails oracle and this binary and diffs status, headers, body and
observed side effects after normalization.

```bash
harness/bin/seed                                   # fixed fixture graph, truncates
RAILS_URL=http://127.0.0.1:3001 RUST_URL=http://127.0.0.1:3002 \
  harness/bin/replay harness/corpus/api_read.yml > /tmp/replay.jsonl
harness/bin/diff /tmp/replay.jsonl
```

`harness/corpus/*.yml` is the whole suite (smoke, api_read, api_delete,
api_upload, routes, ui_read, ui_identity, ui_mutations, phase7, phase8). The
ledger under `docs/ledger/` records a green run per phase.

## Cutover

`src/bin/strangler.rs` is a route-group proxy that switches Rails and the port
per group without a deploy; `docs/cutover-runbook.md` is the operator procedure
and rollback.

Container evaluation stack: [`docs/containers.md`](docs/containers.md).

## Licensing

The port in this repository is licensed under the GNU Affero General Public
License, version 3 only — see [`LICENSE`](LICENSE).

`supermarket-ruby/` is upstream's code, unchanged apart from `UPSTREAM.md`, and
stays under the Apache License 2.0 in
[`supermarket-ruby/LICENSE`](supermarket-ruby/LICENSE). Apache-2.0 and
AGPL-3.0 are compatible in this direction, so the combined work ships under
AGPL-3.0.
