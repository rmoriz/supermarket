# Phase 4 — Web UI mutations

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans. Steps use `- [ ]` checkboxes.

**Goal:** Port every write action reachable from the signed-in UI, with Rails-identical status codes, redirects, flash copy, cookies and DOM fragments, and a corpus that pins them.

**Spec:** `docs/superpowers/specs/2026-11-16-supermarket-rs-port-design.md`
**Upstream sources:** `supermarket-ruby/src/supermarket/app/controllers/{cookbooks,profile,collaborators,transfer_ownership}_controller.rb`, `app/models/{cookbook,collaborator}.rb`, `app/authorizers/*`, `app/views/{cookbooks,resources,collaborators,profile}/*`.

## Scope

Routes (all from `src/routes/table.rs`):

| Method | Path | Action |
|---|---|---|
| PUT | `/cookbooks/{id}/follow` | `cookbooks#follow` (inline partial) |
| DELETE | `/cookbooks/{id}/unfollow` | `cookbooks#unfollow` (inline partial) |
| PATCH | `/cookbooks/{id}` | `cookbooks#update` (source/issues/adoption URLs) |
| PUT | `/cookbooks/{id}/deprecate` | `cookbooks#deprecate` |
| DELETE | `/cookbooks/{id}/deprecate` | `cookbooks#undeprecate` |
| PUT | `/cookbooks/{id}/toggle_featured` | `cookbooks#toggle_featured` |
| GET | `/cookbooks/{id}/deprecate_search` | `cookbooks#deprecate_search` (JSON) |
| POST | `/cookbooks/{id}/adoption` | `cookbooks#adoption` |
| GET | `/collaborators` | `collaborators#index` (JSON) |
| POST | `/collaborators` | `collaborators#create` |
| DELETE | `/collaborators/{id}` | `collaborators#destroy` (head 200) |
| DELETE | `/collaborators/{id}/destroy_group` | `collaborators#destroy_group` |
| PUT | `/collaborators/{id}/transfer` | `collaborators#transfer` |
| PUT | `/cookbooks/{id}/transfer_ownership` | `transfer_ownership#transfer` |
| GET | `/ownership_transfer/{token}/accept` | `transfer_ownership#accept` |
| GET | `/ownership_transfer/{token}/decline` | `transfer_ownership#decline` |
| GET | `/profile/edit` | `profile#edit` |
| PATCH/PUT | `/profile` | `profile#update` |
| POST | `/profile/update_install_preference` | `profile#update_install_preference` |

## Rails semantics to reproduce

- `assign_cookbook` uses `Cookbook.with_name(...).first!`, so an unknown name is
  a 404 before anything else; `follow`/`unfollow`/`adoption` then run
  `store_location_then_authenticate_user!` (store `/cookbooks/<name>`, then
  redirect to `/sign-in` with `user.must_be_signed_in`).
- `authorize!` failures are `Pundit::NotAuthorizedError` → 404 page.
- `follow` creates a follower and increments `cookbook_followers_count` through
  the counter cache; `unfollow` deletes with `first!` (404 if absent). Both
  re-render `follow_button_list` when `list=true`, else `follow_button_show`.
- `update` authorizes `manage_cookbook_urls?`, updates only
  `source_url`/`issues_url`/`up_for_adoption`, and redirects with
  `cookbook.updated` / `adoption.up` / `adoption.down`.
- `deprecate` validates the replacement with `Cookbook#deprecate`: a deprecated
  replacement fails with `cookbook.deprecate_with_deprecated_failure`; success
  redirects with `cookbook.deprecated`. `undeprecate` clears all three fields.
- `toggle_featured` is admin-only, flips the flag, redirects with
  `cookbook.featured` and the `featured`/`unfeatured` state word.
- `deprecate_search` returns `{ "results": [...] }` with only non-deprecated
  cookbooks other than the current one, `name ILIKE '%q%'`, ordered by search
  rank (name).
- `collaborators#index` returns `{ "collaborators": [...] }` excluding
  `ineligible_user_ids`, `LIMIT 20`, optional search.
- `collaborators#create` resolves Cookbook/Tool, adds each eligible user,
  redirects with `collaborator.added`.
- `collaborators#destroy` removes one and returns an empty `200`.
- `destroy_group` removes group collaborators, drops the `GroupResource`,
  appends `collaborator.group_removed` plus the still-collaborating notices,
  then redirects to the resource.
- `collaborators#transfer` demotes the owner to collaborator and promotes the
  collaborator, redirecting with `collaborator.owner_changed`.
- `transfer_ownership#transfer` either moves ownership immediately (admin or
  recipient already a collaborator, message `...done`) or mints a request token
  and mails it (`...email_sent`).
- `accept`/`decline` load only requests with `accepted: nil` (404 otherwise).
- `profile#update` writes the permitted user fields; `update_install_preference`
  requires `preference`, writes it and returns a bare 200 (404 when signed out).

## Steps

- [x] Plan written (this file).
- [x] `Src` mutations: handlers + helpers in `src/web/cookbooks.rs`,
      `src/web/collaborators.rs`, `src/web/profile.rs`; register routes.
- [x] Templates: `policy`-equivalent flags on the sidebar, collaborator add
      dialog, profile edit page with Rails markup.
- [x] Corpus `harness/corpus/ui_mutations.yml` covering success, 404, guest
      redirect, authorizer denial, and every flash branch, with `reset: true`
      and job observers where Rails enqueues.
- [x] Replay to 100% MATCH; `cargo fmt --check`, `cargo clippy --all-targets --
      -D warnings`, `cargo test`.
- [x] Commit and update the master ledger.
