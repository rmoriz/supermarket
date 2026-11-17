# Known gaps against the Rails reference

Audit date: 2026-11-17, against `supermarket-ruby/` at the vendored commit.

The port answers all 119 routes of `harness/routes.tsv`, so none of these are
missing paths. They are missing behaviour behind paths the differential corpus
does not exercise: the oracle runs with every feature on, and several of these
need an external service, so replaying the corpus cannot catch them.

Order is by impact, not effort. What is closed is listed at the end; the open
items are the ones that need a new subsystem.

## 1. Evaluation itself

- No `MetricsRunner`, and none of the workers it fans out to:
  `CookstyleWorker`, `NoBinariesWorker`, `CollaboratorWorker`,
  `ContributingFileWorker`, `TestingFileWorker`, `VersionTagWorker`, or the
  `CookbookArtifact` download-and-unpack helper (`engines/fieri/app/models/`).
  `POST /fieri/jobs` only enqueues; the unknown-class path in
  `src/worker.rs:1034-1038` hands the job back to Ruby. Without a fieri service
  at `FIERI_URL`, no metric is ever produced.
- No `RunQualityMetrics` service, and none of its tasks:
  `quality_metrics:run:all_the_latest|on_latest|on_version`, nor the visibility
  tasks `quality_metrics:run:all_public|all_admin_only|public|admin_only|list`.
  `supermarket-worker` has one subcommand, `spdx`.
- No `cookbook:grant_partner_badge` and no `user:make_admin`, so granting a
  partner badge or an admin role needs SQL.

## 2. S3 storage: config parsed, storage layer absent

Upstream moves every tarball write, URL and delete to S3 whenever `S3_BUCKET`
and `S3_REGION` are set (`app/lib/supermarket/s3_config.rb:15-25`,
`config/initializers/paperclip.rb:16-27`). The port parses
`S3_BUCKET`/`S3_REGION`/`S3_URLS_EXPIRE` into `Config.s3` (`src/config.rs:134-151`)
and reads none of it. There is no S3 module, no `aws-sdk-*` crate in
`Cargo.lock`, `write_tarball` (`src/api/v1/cookbook_uploads.rs:818`) always writes
`public/system/...`, `artifact_url` (`src/api/v1/cookbook_versions.rs:116`)
always returns a local path and `routes.rs:482` always serves `/system`.

Upstream contract, for reference:

| Variable | Effect |
| --- | --- |
| `S3_BUCKET`, `S3_REGION` | Both required, any-but-not-all `IncompleteConfig` at boot with its own message (`s3_config.rb:19-21`). |
| `S3_ACCESS_KEY_ID`, `S3_SECRET_ACCESS_KEY` | Static keys; both required, partial pair raises its own `IncompleteConfig` (`s3_config.rb:86-93`). Absent means the SDK default chain: IAM role, ECS task role, IMDS. |
| `S3_PATH` | Prefix prepended to the paperclip path (`s3_config.rb:37-39`). |
| `S3_PRIVATE_OBJECTS=true` | Object ACL `private`; otherwise paperclip's default `public-read` (`s3_config.rb:41-43`, `storage/s3.rb` `set_permissions`). |
| `S3_ENCRYPTION` | `x-amz-server-side-encryption` on PUT, symbolised (`s3_config.rb:45-47`). |
| `S3_ENDPOINT` | Custom endpoint, host name for URLs, and `force_path_style` for the client (`s3_config.rb:54-61`). |
| `S3_DOMAIN_STYLE` | URL template, default `:s3_domain_url`, e.g. `:s3_path_url` (`s3_config.rb:34`). |
| `CDN_URL` | Forces `:s3_alias_url` against that host, overriding `S3_DOMAIN_STYLE` (`s3_config.rb:49-52`). |
| `PROTOCOL` | Scheme for generated S3 URLs; empty yields protocol-relative `//host/key`. |
| `S3_URLS_EXPIRE` | Seconds; `cookbook_artifact_url` then returns a presigned GET instead of a plain URL (`app/models/cookbook_version.rb:84-89`). |

Object key is paperclip's `:class/:attachment/:compatible_id/:style/:basename.:extension`
with `compatible_id` = `legacy_id || id` (`initializers/paperclip.rb:2-4,16`),
i.e. `[S3_PATH/]cookbook_versions/tarballs/<legacy_id||id>/original/<basename>.tgz`.
Writes carry the upload content type, the ACL above and SSE when set; `NoSuchBucket`
creates the bucket and retries, `SlowDown` retries with backoff (`storage/s3.rb`
`flush_writes`). `flush_deletes` deletes the object and swallows `ServiceError`,
so destroying a version or a cookbook (cascade at `app/models/cookbook.rb:100`)
drops the object too. `tarball.url` still appends the `?<updated_at epoch>`
cache buster, and `artifact_url` feeds FIERI
(`app/workers/fieri_notify_worker.rb:27`), so the presign branch matters beyond
the browser redirects.

Port work, in order:

1. Extend `Config.s3` with the rest of the matrix, refuse partial credential
   pairs with upstream's message, and stop parsing S3 when the pair is absent.
2. New async storage module: SigV4-presigned PUT/DELETE and presigned GET with
   expiry, ACL/SSE headers, path/domain/alias URL shapes, bucket-create retry.
   Either `aws-sdk-s3` (upstream's own dependency; brings the credential chain
   including IAM roles) or hand-rolled SigV4 on the existing `hmac`/`sha2`/`hex`
   plus `reqwest` (no new dependency, but IMDS/ECS credentials must be written
   by hand).
3. `write_tarball` becomes `async`, passes `legacy_id`, and PUTs instead of
   writing when S3 is on; `artifact_url` branches on storage and presigns when
   `S3_URLS_EXPIRE` is set; `fieri_notify` posts `artifact_url` rather than the
   `/download` endpoint (wrong even locally today).
4. Both destroy paths (`cookbook_uploads.rs:828` and `:1042`) delete the
   objects they own — currently even filesystem tarballs are left behind.
5. `legacy_id` is also missing from `artifact_url`, so migrated rows already
   resolve to `<new id>` on both backends instead of `<legacy_id>`.
6. Harness: minio service, S3 env for both sides, `X-Amz-*`/expiry
   normalization in `harness/normalize.rb`, plus a corpus case per storage mode.

Consequences today:

- `S3_ACCESS_KEY_ID` / `S3_SECRET_ACCESS_KEY` are unsupported, as are
  `S3_PATH`, `S3_PRIVATE_OBJECTS`, `S3_ENCRYPTION`, `S3_ENDPOINT`,
  `S3_DOMAIN_STYLE` and `CDN_URL`.
- `S3_URLS_EXPIRE` does nothing.
- A deployment that sets `S3_BUCKET` has Rust write to the filesystem while
  Rails writes to the bucket, so artifacts are neither shared nor reachable
  during a strangle rollout. Without S3 configured both apps use the same
  `public/system` path.
- Deletes leave every tarball behind, on both backends.

## 3. Observability libraries

- `SENTRY_URL`: no Sentry reporter (upstream `config/initializers/raven.rb`).
- `DATADOG_TRACER_ENABLED` / `DATADOG_APP_NAME` / `DATADOG_ENVIRONMENT`: no
  tracer (upstream `config/initializers/datadog_tracer.rb`).
- `OPENSSL_FIPS=1`: no FIPS mode (`app/lib/supermarket/fips.rb`). The port
  starts anyway instead of failing fast when FIPS cannot be verified.

## 4. Small ones

- Two upstream data migrations have no equivalent:
  `mark_bad_readme_imports_for_reimport` and
  `remove_duplicate_cookbook_dependencies` (`app/lib/supermarket/migration/`).
- The development-only `letter_opener` mount is absent, noted at
  `src/routes/table.rs:21`.
- `CLA_REPORT_EMAIL`, `COOKSTYLE_COPS`, `CURRY_CLA_LOCATION`,
  `CURRY_SUCCESS_LABEL`, `GITHUB_ACCESS_TOKEN`, `GITHUB_ENTERPRISE_URL`,
  `PUBSUBHUBBUB_SECRET`, `ICLA_VERSION` and `CCLA_VERSION` are read by upstream
  code the port does not have (CLA reporting, fieri workers, hub pings), so they
  are still inert.

## Not gaps

- `POST /api/v1/quality_metrics/publish_evaluation`,
  `POST /api/v1/quality_metrics/supported_platforms_evaluation` and
  `POST /api/v1/cookbook-versions/publish_evaluation` answer
  `ActionNotFound`; upstream `QualityMetricsController` has no such methods.
- `PATCH /profile/change_password` is a 404 upstream too;
  `ProfileController` defines no such action.
- `cla` and `join_ccla` are listed as features but gate nothing upstream.

## Closed on branch `gap-fixes`

**Route-level feature constraints.** `/tools*`, `/tools-directory`,
`/users/{id}/tools`, `/fieri/*` and every `*_evaluation` route now answer the
catch-all 404 while their feature is off, as upstream's route constraints make
them disappear. The guard runs before the CSRF check, matching route matching
happening before controller callbacks. `/api/v1/tools` and
`/api/v1/tools-search` stay ungated, as upstream leaves them outside the
constraint. Implemented in `src/middleware.rs` (`require_feature`).

**Analytics partial.** `templates/application/_analytics.html` renders the
OneTrust, gtag and Segment snippets from `ENABLE_ONETRUST`/`ONETRUST_ID`,
`ENABLE_GTAG`/`GOOGLE_ANALYTICS_ID` and `SEGMENT_WRITE_KEY`. `security_policy`
now generates the CSP nonce before the handler and passes it on as an internal
request header, so the inline tags carry the same nonce as the policy.

**Air-gapped fonts.** `Shell::fonts` follows `AIR_GAPPED`, so the Google Fonts
stylesheet is dropped like upstream's `unless air_gapped?`. The analytics
partial is suppressed under `AIR_GAPPED` for the same reason.

**Host-header allowlist.** `DISABLE_HOST_HEADER_ATTACK` (truthy) plus
`ALLOWED_HOSTS` builds the allowlist from `production.rb`; a foreign, absent or
`X-Forwarded-Host`-spoofed host answers 403 with Rails' empty body.

**`CHEF_OAUTH2_VERIFY_SSL=false`.** The OAuth token and userinfo calls now
accept an invalid certificate for the id server.

**StatsD port.** `STATSD_URL` and `STATSD_PORT` are both required, as in
`config/initializers/statsd.rb`, and the port is used instead of assuming 8125.

**Follow counters.** `cookbook.followed` and `cookbook.unfollowed` are
incremented like `CookbooksController`, including the duplicate-follow case.

**`REDIS_JOBQ_URL`.** Jobs, the worker, cron, `/fieri/status` and the health
sidekiq/redis counters use a separate client for `REDIS_JOBQ_URL` (falling back
to `REDIS_URL`); the universe cache stays on `REDIS_URL`.
