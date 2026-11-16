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

## 2. S3 storage: config exists, code does not

`Config.s3` is parsed from `S3_BUCKET` / `S3_REGION` / `S3_URLS_EXPIRE`
(`src/config.rs`) and never read. There is no `.s3` consumer and no S3 crate in
`Cargo.lock`. `write_tarball` always writes
`public/system/cookbook_versions/tarballs/...` and `artifact_url()` always
returns a local path.

Consequences:

- `S3_ACCESS_KEY_ID` / `S3_SECRET_ACCESS_KEY` are unsupported.
- `S3_URLS_EXPIRE` does nothing.
- A deployment that sets `S3_BUCKET` has Rust write to the filesystem while
  Rails writes to the bucket, so artifacts are not shared during a strangle
  rollout. Without S3 configured both apps use the same
  `public/system` path.
- Deletes never touch a bucket because they cannot.

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
