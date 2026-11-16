# Phase 3 — Web UI, read paths

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans.

**Goal:** Every public read page renders the same HTML as Rails: layout, header/footer, welcome, dashboard, directory and filters, cookbook/version pages, user and group pages, search, error pages, robots and the atom feed. Mutations stay stubs.

**Architecture:** `askama` templates under `templates/`, ported one-for-one from `app/views` (same partial decomposition, same whitespace). A `render` module assembles the shared context (current user, flash, search, feature flags, URL helpers) exactly as `ApplicationController` and `ApplicationHelper` do. Asset URLs come from the precompiled manifest copied out of the oracle.

**Spec:** `docs/superpowers/specs/2026-11-16-supermarket-rs-port-design.md`

## Global constraints

- Byte parity after normalization: copy changes from `docs/copy-changes.md` are applied to the Rails side by the harness, and nowhere else.
- Rails' `content_for` fields (title, description) become explicit template variables; the layout applies the same join/default.
- Asset URLs are the oracle's digest paths, served from `static/assets` with Rails' cache headers.
- Session/flash/CSRF come from the existing cookie and session modules; no page is ported without its corpus entries in the same commit.
- Feature flags (`tools`, `announcement`, `no_crawl`) gate the same blocks as upstream.

## Tasks

### Task 1: Template plumbing and layout ✅
- [x] Add `askama`, a `render` context module, and the `templates/` tree.
- [x] Port `layouts/application`, `application/_appheader`, `_flash`, `_search`, `_announcement_banner`, `_analytics`.
- [x] Serve `static/assets` with the precompiled manifest and Rails' cache headers.
- [x] Corpus: `ui_read.yml` covers the layout on `/` for guest and signed-in user.

### Task 2: Error pages and robots ✅
- [x] Port `exceptions/401|404|422|500` and `pages/robots`.
- [x] Fallback and `not_found!` render the exceptions page in the negotiated format.
- [x] Corpus: unknown path (HTML + JSON), unmatched method, robots.

### Task 3: Welcome, dashboard, documentation ✅
- [x] Port `pages/welcome`, `pages/dashboard`, `pages/documentation`, `resources/_community_stats`.
- [x] Signed-in welcome redirects to the dashboard; dashboard requires a session.
- [x] Corpus: guest and signed-in variants, counts, activity feed.

### Task 4: Cookbooks directory, index, show, version ✅
- [x] Port `cookbooks/directory`, `index`, `show`, `available_for_adoption` and their partials (cookbook card, sidebar, main, installs, metrics, manage box, follow buttons).
- [x] Every filter (`q`, `platforms[]`, `badges[]`, `deprecated`), sort (`order`) and page param behaves as upstream.
- [x] Corpus: each filter combination, sort, page boundary, unknown cookbook, version download page.

### Task 5: Users, groups, search ✅
- [x] Port `users/show|tools|groups`, `groups/index|show`, the sidebar/heading/member partials.
- [x] Port the shared search results page used by `cookbooks#index` and `tools#index`.
- [x] Corpus: user tabs, group pages, search by name, search by badge/platform, empty results.

### Task 6: Atom feed and ledger ✅
- [x] Port `users/followed_cookbook_activity.atom.builder` semantics.
- [x] Green run recorded in `docs/ledger/phase-3.md`.

## Definition of Done

`harness/corpus/ui_read.yml` is 100% MATCH, `cargo test`/`clippy` are clean, and the ported page specs (welcome, directory, cookbook, user, group, search) pass against the Rust host.
