# Gap fixes ledger

Date: 2026-11-17

Closes the items of [`docs/known-gaps.md`](../known-gaps.md) that needed no new
subsystem. S3 storage and the fieri evaluation workers stay open.

## Verification

| Check | Command | Result |
|-------|---------|--------|
| Build | `cargo build --release` | clean |
| Lint | `cargo clippy --all-targets -- -D warnings` | clean |
| Format | `cargo fmt --check` | clean |
| Unit + integration | `cargo test` | 100 lib, 26 integration, 0 failed |
| Corpus, direct | all ten `harness/corpus/*.yml` against :3005 | every case matches (369) |
| Corpus, proxy | same files through `PORT=3002 STRANGLER_GROUPS=chat,tools,feeds,ui,reads,uploads` | every case matches (369) |

## Live checks

Feature gating, `FEATURES=chat` (tools and fieri absent):

```
/tools 404  /tools-directory 404  /users/ada/tools 404
/fieri/jobs 404  /fieri/status 404
/api/v1/quality_metrics/cookstyle_evaluation 404
```

Same with `FEATURES=tools,fieri,announcement,chat`: all 200/400 as before, and
`/api/v1/tools` plus `/api/v1/tools-search` stay 200 with the flags off (they are
outside the upstream constraint).

Analytics with `ENABLE_GTAG=true GOOGLE_ANALYTICS_ID=G-TEST123
SEGMENT_WRITE_KEY=segkey`: the page carries the gtag and Segment tags, each
`nonce` equal to the `'nonce-…'` in the CSP. With `AIR_GAPPED=true` both the
tags and the `fonts.googleapis.com` link disappear. `ENABLE_ONETRUST=true
ONETRUST_ID=abc-123` renders the OneTrust loader.

Host allowlist, `DISABLE_HOST_HEADER_ATTACK=true ALLOWED_HOSTS=127.0.0.1,localhost`:
`Host: 127.0.0.1` is 200, `Host: evil.test` and `X-Forwarded-Host: evil.test`
are 403 with `content-type: text/html; charset=UTF-8` and `content-length: 0`.

Job queue, `REDIS_JOBQ_URL=redis://127.0.0.1:56379/3` on the worker: the
`cron_job:*` and `queues` keys land in database 3, database 1 stays empty. A
job pushed to database 3 is reported by `/fieri/status` (`queued_jobs: 1`).

## Notes

- The route gate is a path check in an outer layer, not a per-route
  `route_layer`, because Rails route matching runs before the CSRF callback: a
  gated-off `POST /tools` must answer 404, not 422.
- The CSP nonce reaches the renderer as the internal request header
  `x-csp-nonce`; `security_policy` generates it before the handler.
- `routing_not_found` is now shared by the fallback and the gate so both render
  the identical exceptions page.
