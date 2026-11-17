# Gap fixes ledger

Date: 2026-11-17

Closes the items of [`docs/known-gaps.md`](../known-gaps.md) that needed no new
subsystem, plus S3 artifact storage. The fieri evaluation workers stay open.

## Verification

| Check | Command | Result |
|-------|---------|--------|
| Build | `cargo build --release` | clean |
| Lint | `cargo clippy --all-targets -- -D warnings` | clean |
| Format | `cargo fmt --check` | clean |
| Unit + integration | `cargo test` | 111 lib, 26 integration, 0 failed |
| Corpus, S3 on | all ten `harness/corpus/*.yml` against :3005, garage storage | every case matches (410) |
| Corpus, S3 off | same files, both sides on `public/system` | every case matches (410) |
| Normalizer spec | `ruby harness/spec/normalize_test.rb` | all checks pass |

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

## S3 storage

`src/storage.rs` implements the paperclip contract with `aws-sdk-s3`: key
`[S3_PATH/]cookbook_versions/tarballs/<legacy_id||id>/original/<file>`, ACL
`public-read` unless `S3_PRIVATE_OBJECTS=true`, SSE from `S3_ENCRYPTION`,
bucket creation on `NoSuchBucket` (one retry), `SlowDown` backoff ×5, and
`flush_deletes`-style deletes that swallow errors. `S3_URLS_EXPIRE` presigns the
artifact URL, which is also what `fieri_notify` now posts; plain URLs keep the
`?<mtime>` buster. `S3_DOMAIN_STYLE`, `CDN_URL`, `S3_ENDPOINT` and `PROTOCOL`
build the URL, and a partial bucket/region or key pair refuses to boot with
upstream's `IncompleteConfig` text. The client sits in a `OnceCell`, so a
filesystem deployment never enters the credential chain. `tests/…` and seven
unit tests cover the key shape, every URL style and a presign round trip against
a fake endpoint.

Live proof, `S3_PATH=harness`, `S3_PRIVATE_OBJECTS=true`, `S3_URLS_EXPIRE=300`,
garage on 39000:

```
publish brand-new.tgz        -> 201; object at harness/cookbook_versions/tarballs/7/original/brandnew-0.1.0.tgz
GET  .../versions/0.1.0/download -> 302, presigned location (X-Amz-Expires=300, X-Amz-Signature)
signed GET of that location  -> 200, 229 bytes, sha256 equal to the fixture
unsigned GET of the key      -> 403 (S3_PRIVATE_OBJECTS)
DELETE .../versions/2.0.0    -> 200; only tarballs/7/…redis-2.0.0.tgz disappears
DELETE .../cookbooks/brandnew -> 200; only tarballs/6/…brandnew-0.1.0.tgz disappears, 7/… stays
```

## Notes

- The route gate is a path check in an outer layer, not a per-route
  `route_layer`, because Rails route matching runs before the CSRF callback: a
  gated-off `POST /tools` must answer 404, not 422.
- The CSP nonce reaches the renderer as the internal request header
  `x-csp-nonce`; `security_policy` generates it before the handler.
- `routing_not_found` is now shared by the fallback and the gate so both render
  the identical exceptions page.
- `/chat` is no longer gated: upstream routes it unconditionally and the corpus
  showed the 404 against a live Rails answering 302. The `chat` key now only
  appears in the health report.
- The deletion response needed explicit column aliases (`a.username AS
  maintainer`, `cat.name AS category`) and a `latest` alias; without them
  sqlx decoded into the wrong fields and a category-less delete answered 400.
  The corpus gained `an admin deletes a cookbook that has no category`.
- Garage ignores object ACLs and 403s anonymous GETs even in website mode, and
  its `GetObjectAcl` is unimplemented, so the harness presigns
  (`S3_URLS_EXPIRE=300`) and the normalizer drops `X-Amz-*`, `x-id=GetObject`
  and rewrites the host endpoint `127.0.0.1:39000` to `garage:3900`.
