# Vendoring chef/supermarket as `supermarket-ruby` — design

**Date:** 2026-11-16
**Status:** approved

## Intent

`supermarket-rs` is being prepared as a Rust port workspace. The Ruby original
(https://github.com/chef/supermarket, Apache-2.0) is needed in-tree as a
readable, greppable, blame-able reference.

## Goals

- Upstream source available locally under `supermarket-ruby/`, unmodified.
- Upstream history and blame preserved, so porting decisions can be traced.
- The exact imported commit is recorded and the update path is a documented
  one-liner.

## Non-goals

- Running the imported app (no `bundle install`, no DB, no Ruby version pin).
- Changing anything inside the imported tree.
- Continuous upstream tracking (no automation, no CI).

## Constraints

- Nothing may be renamed or restructured inside `supermarket-ruby/`; upstream
  history must stay intact.
- Upstream's Apache-2.0 `LICENSE` and all upstream files stay as-is, including
  their copyright headers. Only files at the repo root and
  `supermarket-ruby/UPSTREAM.md` are ours.
- Import pinned to a fixed commit: `80d517b228fac3290fc83b06d3a873117d13b090`
  (upstream `main`, 2026-07-29). Not `main` head-of-branch, so porting works
  against a stable target.

## Approaches considered

1. **`git subtree` at a pinned commit — chosen.** Full history in one repo,
   `git subtree pull` for updates, no empty folders after clone. Costs ~14 MB of
   objects and merge noise in `git log`.
2. **Plain snapshot, no history.** Smallest and cleanest, but no blame/diff on
   upstream code and updates become manual re-snapshots.
3. **Submodule.** One pointer file, but a fresh clone shows an empty directory
   until `git submodule update --init`, which breaks the "reference is right
   here" purpose.

## How it is built

- Initial commit on `main` with `README.md`, `.gitignore`, and the `docs/` tree,
  so `main` has a base before a merge-based import.
- `git remote add upstream-supermarket https://github.com/chef/supermarket`,
  full fetch, then `git subtree add --prefix=supermarket-ruby <sha>` with no
  `--squash`.
- `supermarket-ruby/UPSTREAM.md` records source URL, SHA, import date, license,
  and the update recipe; the root README repeats the recipe.

## Left to the builder

Commit message wording and README prose.
