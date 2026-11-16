# Phase 1 — public read API

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans.

**Goal:** Every unauthenticated read route answers byte-identically to the Rails app, including its quirks, ordering and error bodies.

**Architecture:** One module per controller under `src/api/v1/`, each with its own SQL (no ORM, no shared query builder beyond pagination/ordering helpers). Version ordering is a port of `Chef::Version`, not a hand-rolled comparator.

**Tech Stack:** as phase 0, plus `serde_json` with `preserve_order` and explicit key emission order.

**Spec:** `docs/superpowers/specs/2026-11-16-supermarket-rs-port-design.md`

## Global constraints

- JSON key order is asserted literally in tests, matching the jbuilder declaration order.
- `total` in list endpoints is `Cookbook.count` over **all** cookbooks even when `user`/`platforms` filters apply — a real quirk, pinned.
- Nothing here may "fix" upstream: `location_type` stays `"opscode"`, `average_rating` stays `null`, `start`/`items` stay string-parsed.
- All corpus entries land in the same commit as the handler.

## Review Focus

- `latest` / `_`-separated versions (`1_0_0`) / unknown versions resolving wrong or 500-ing — pinned per route.
- Negative `start`/`items`, `items` above `API_ITEM_LIMIT`, non-integer values — pinned with exact 400 bodies.
- Deprecated cookbooks sorting first, ties broken as upstream does — pinned with seeded ties.
- `/universe` cache key per protocol and the `hits` increment happening even on cache hits — pinned.
- Download counters incrementing per protocol (api vs web) and the redirect target (S3 vs local) both — pinned.

---

### Task 1: Pagination and ordering helpers

**Files:**
- Create: `src/api/v1/params.rs`
- Test: `src/api/v1/params.rs`

**Interfaces:**
- Produces: `struct ListParams { start: i64, items: i64, order: Order }`; `ListParams::parse(query: &Query, api_item_limit: Option<i32>) -> Result<ListParams, ApiError>`; `enum Order { RecentlyUpdated, RecentlyAdded, MostDownloaded, MostFollowed, ByName }` (default `ByName`) with `Order::sql(&self) -> &'static str` returning the exact `ORDER_OPTIONS` strings (`cookbooks.deprecated, cookbooks.updated_at DESC`, etc.).

- [x] **Step 1: Failing tests**: `items` defaults to 10; `items=500` clamps to 100; `items=500` with `API_ITEM_LIMIT=7` clamps to 7; `API_ITEM_LIMIT=0` falls back to 100; `start=-1` → 400 with `error_code: "INVALID_DATA"` and the `negative_parameter` message verbatim (`locale: api.error_messages.negative_parameter`, `start`/`items` filled, "not provided" when absent); unknown `order` falls back to `by_name`.
- [x] **Step 2: Run — fail.**
- [x] **Step 3: Implement** matching `Api::V1Controller#init_params` exactly (including that `params.fetch(:start, 0).to_i` makes `start=abc` a `0`).
- [x] **Step 4: Green; commit `feat: list params parity`.**

### Task 2: `Chef::Version` ordering

**Files:**
- Create: `src/chef_version.rs`
- Test: `src/chef_version.rs`, fixtures `harness/fixtures/versions.json`

**Interfaces:**
- Produces: `chef_version::Version` with `Ord`/`PartialOrd` matching Ruby `Chef::Version`; `chef_version::sort_desc(&[String]) -> Vec<String>`; `chef_version::latest(&[String]) -> Option<&String>`.

- [x] **Step 1: Generate fixtures** with `supermarket-ruby`'s `chef` gem: 200 version strings (semver, prerelease, `_`-separated, `1.0`, `1.0.0.0`) plus their `sort_by`-descending order and `Chef::Version#<=>` matrix; write to `harness/fixtures/versions.json`.
- [x] **Step 2: Failing tests** replaying the order and the comparison matrix.
- [x] **Step 3: Implement** the segment/qualifier rules until identical, including invalid-version behaviour.
- [x] **Step 4: Green; commit `feat: Chef::Version ordering`.**

### Task 3: `GET /api/v1/cookbooks`

**Files:**
- Create: `src/api/v1/cookbooks.rs`
- Test: `src/api/v1/cookbooks.rs`, corpus `harness/corpus/api_cookbooks_index.yml`

**Interfaces:**
- Produces: `cookbooks::index(State<AppState>, Query<HashMap<String,String>>) -> Result<Json<Vec<u8>>, ApiError>`; body `{"start":…,"total":…,"items":[{"cookbook_name":…,"cookbook_maintainer":…,"cookbook_description":…,"cookbook":url}]}` in that key order, `total` from `SELECT count(*) FROM cookbooks`, `cookbook` = `{base}/api/v1/cookbooks/{name}`.
- `maintainer` = owner's `accounts.username` where `provider = 'chef_oauth2'`; `description` = latest version's description.

- [x] **Step 1: Corpus** — default, `start`/`items` paging, each order value, `user=`, `platforms[]=`, unknown user, negatives, `items` over limit, `Accept: application/json` and bare.
- [x] **Step 2: Failing replay — every entry `DIFF`.**
- [x] **Step 3: Implement** with one SQL statement: join `cookbooks → users → accounts(chef_oauth2)`, lateral join to the semver-latest `cookbook_versions` for the description, optional `user` and platform-exists filters, `ORDER BY` from `Order::sql()`, plus a separate `total` count. Semver-latest is computed in Rust from the joined versions when the SQL `max(version)` disagrees (pinned by a test with `1.10.0` vs `1.9.0`).
- [x] **Step 4: Green; commit `feat: cookbook index API`.**

### Task 4: `GET /api/v1/search`

**Files:**
- Modify: `src/api/v1/cookbooks.rs`
- Test: corpus `harness/corpus/api_search.yml`

**Interfaces:**
- Produces: `cookbooks::search(...)` with the same body shape as Task 3, ordered by the `pg_search` ranking.

- [x] **Step 1: Corpus** — `q=redis`, `q=` (no param), `q` with no matches, `q` with pagination, `q` matching a maintainer username and one matching only a description, a name with a typo (trigram path).
- [x] **Step 2: Failing replay.**
- [x] **Step 3: Capture the SQL `pg_search` emits** by enabling `ActiveRecord::Base.logger` on the Rails side for one search, then reproduce it verbatim in `src/api/v1/search.sql` (A-weighted trigram on `name`, B on username, C on description, `ranked_by ":trigram + (0.5 * :tsearch)"`, `order_within_rank cookbooks.name`). Keep the file under review so a later "tidy" does not reorder results.
- [x] **Step 4: Green including ordering assertions; commit `feat: search API with pg_search parity`.**

### Task 5: `GET /api/v1/cookbooks/:cookbook`

**Files:**
- Modify: `src/api/v1/cookbooks.rs`
- Test: corpus `harness/corpus/api_cookbook_show.yml`

**Interfaces:**
- Produces: body = cookbook partial keys in order `name, maintainer, description, category, latest_version, external_url, source_url, issues_url, average_rating(null), created_at, updated_at, up_for_adoption`, then `deprecated`, optional `replacement`, `versions` (absolute URLs, semver-descending), `metrics: {downloads: {total, versions: {ver: count}}, followers, collaborators}`.

- [x] **Step 1: Corpus** — normal cookbook; case-insensitive name (`/API/V1/COOKBOOKS/Redis`); deprecated cookbook with replacement; deprecated without replacement; unknown → 404 JSON; `latest_version` URL.
- [x] **Step 2: Failing replay.**
- [x] **Step 3: Implement**, timestamp format `2026-11-16T10:00:00.000Z` (ActiveSupport JSON), category nullable, counts from `cookbook_followers`/`collaborators`.
- [x] **Step 4: Green; commit `feat: cookbook show API`.**

### Task 6: `GET /api/v1/cookbooks/:cookbook/contingent`

**Files:**
- Modify: `src/api/v1/cookbooks.rs`
- Test: corpus `harness/corpus/api_contingent.yml`

- [x] **Step 1: Corpus** — cookbook with two contingents (asserting sort by dependent cookbook name then semver), cookbook with none (`{"contingents":[]}`), unknown cookbook (404).
- [x] **Step 2: Failing replay.**
- [x] **Step 3: Implement**: `cookbook_dependencies WHERE cookbook_id = $1` joined to versions/cookbooks, sorted in Rust per `Cookbook#contingents`.
- [x] **Step 4: Green; commit `feat: contingent API`.**

### Task 7: `GET /api/v1/cookbooks/:cookbook/versions/:version`

**Files:**
- Create: `src/api/v1/cookbook_versions.rs`
- Test: corpus `harness/corpus/api_version_show.yml`

**Interfaces:**
- Produces: keys in order `license, tarball_file_size, version, published_at (ISO8601 with no millis), average_rating(null), cookbook, file`, optional `quality_metrics` (only when the `fieri` feature is on), `supports {platform: constraint}`, `dependencies {name: constraint}`.

- [x] **Step 1: Corpus** — exact version, `latest`, underscore form `1_0_0`, unknown version (404), unknown cookbook (404), version with platforms and dependencies (ordering of `supports`/`dependencies` as upstream: insertion order by id).
- [x] **Step 2: Failing replay.**
- [x] **Step 3: Implement**; `published_at` via `chrono` `to_rfc3339_opts(SecondsFormat::Secs, true)`.
- [x] **Step 4: Green; commit `feat: version show API`.**

### Task 8: `GET …/versions/:version/download`

**Files:**
- Modify: `src/api/v1/cookbook_versions.rs`
- Test: corpus `harness/corpus/api_download.yml`

- [x] **Step 1: Corpus** — download of an existing version (`302`, `Location` header), `latest`, unknown version, plus counter assertions read from Postgres after replay (`api_download_count` +1 on version **and** cookbook, `web_download_count` untouched).
- [x] **Step 2: Failing replay.**
- [x] **Step 3: Implement** counters in one transaction, then `Redirect::to(artifact_url)`; `artifact_url` from the storage trait: S3 presigned with `S3_URLS_EXPIRE` when set, otherwise `{FQDN}/…` local path. Statsd `cookbook.downloads.api`.
- [x] **Step 4: Green; commit `feat: download redirect with counters`.**

### Task 9: `/universe` and `/api/v1/universe`

**Files:**
- Create: `src/api/v1/universe.rs`, `src/universe_cache.rs`
- Test: corpus `harness/corpus/api_universe.yml`

**Interfaces:**
- Produces: `universe::index` returning the exact hash, keyed cookbook → version → `{location_type:"opscode", location_path, download_url, dependencies{}}`; Redis key `{protocol}-universe`; `hits` upsert.

- [x] **Step 1: Corpus** — plain HTTP, `X-Forwarded-Proto: https` (different cache key and URLs), cookbook whose only dependency is itself (excluded), cookbook with two versions and one dependency; assert the `hits` row increments 1→2 across two calls *including* a cache hit.
- [x] **Step 2: Failing replay.**
- [x] **Step 3: Implement** the SQL from `app/models/universe.rb` verbatim (including the self-dependency exclusion), hand-emit JSON in the source's key order (`location_type`, `location_path`, `download_url`, `dependencies`), and the single-statement hit upsert (`WITH upd AS (UPDATE …) INSERT … WHERE NOT EXISTS`).
- [x] **Step 4: Green; commit `feat: universe API with cache and hit counter`.**

### Task 10: `GET /api/v1/users/:user`

**Files:**
- Create: `src/api/v1/users.rs`
- Test: corpus `harness/corpus/api_users.yml`

- [x] **Step 1: Corpus** — user owning/following/collaborating on cookbooks, user with GitHub accounts (sorted) and a tool, unknown user (404), user whose only account is `github` (not found).
- [x] **Step 2: Failing replay.**
- [x] **Step 3: Implement**: lookup by `accounts.username` **restricted to `provider='chef_oauth2'`**, `name` falling back to username, map-shaped `cookbooks.owns/collaborates/follows` and `tools.owns/collaborates` keyed by resource name in `name ASC`.
- [x] **Step 4: Green; commit `feat: user API`.**

### Task 11: `GET /api/v1/metrics`, `/api/v1/health`, `/status`

**Files:**
- Create: `src/api/v1/metrics.rs`, `src/api/v1/health.rs`
- Test: corpus `harness/corpus/api_metrics_health.yml`

- [x] **Step 1: Corpus** — metrics with seeded counts; health with all services up (exact key order `status, supermarket, sidekiq, postgresql, redis, features`); health with Redis stopped (status `not ok`, `redis.status: "unreachable"`) launched by a harness hook; `/status` alias.
- [x] **Step 2: Failing replay.**
- [x] **Step 3: Implement**: metrics counters in one SQL round trip; health reads `pg_stat_activity` (`connections`, `waiting_on_lock` via `wait_event IS NOT NULL`), Redis `INFO` fields (`uptime_in_seconds`, `connected_clients`, `used_memory`, `used_memory_peak`, `-1` fallback), Sidekiq sets/lists (`queue:default` latency+size, `schedule`, `retry`, `dead`, `stat:processed`, `stat:failed`, workers), `FEATURES` flags, `supermarket.expired_ocid_tokens`.
- [x] **Step 4: Green; commit `feat: metrics and health APIs`.**

### Task 12: Tools

**Files:**
- Create: `src/api/v1/tools.rs`
- Test: corpus `harness/corpus/api_tools.yml`

- [x] **Step 1: Corpus** — index with `start`/`items`/`order`, search by `q`/`type`/`order`, tool show by slug, unknown slug (404), `ALLOWED_TYPES` filter ignoring bogus types, and `total` for search being the *filtered* count (unlike cookbooks).
- [x] **Step 2: Failing replay.**
- [x] **Step 3: Implement** with the `tools` counterpart of the search SQL.
- [x] **Step 4: Green; commit `feat: tools API`.**

### Task 13: Remove all phase-1 stubs and verify the phase

- [x] **Step 1: `harness/bin/replay --corpus harness/corpus/api_*.yml` → zero `DIFF`.**
- [x] **Step 2: Kill the still-`501` stub routes for these paths only; the manifest test proves no route was dropped.**
- [x] **Step 3: Load test `/universe` and `/api/v1/search` at 50 rps for 2 minutes; record p95 in the ledger.**
- [x] **Step 4: Commit `phase 1 complete`.**

## Phase exit check

Full phase-1 corpus green, load numbers recorded, no stub left on any phase-1 path.
