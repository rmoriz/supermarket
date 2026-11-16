# Vendor chef/supermarket as `supermarket-ruby` — implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Import https://github.com/chef/supermarket at commit `80d517b2` into `supermarket-ruby/` with full upstream history via `git subtree`.

**Architecture:** A base commit establishes the repo (README, `.gitignore`, `docs/`), then a subtree merge imports the pinned upstream tree untouched, then `UPSTREAM.md` records provenance and the update/pull recipe.

**Tech Stack:** git 2.56, `git subtree`, GitHub HTTPS remote.

**Spec:** `docs/superpowers/specs/2026-11-16-vendor-supermarket-ruby-design.md`

## Global Constraints

- Upstream commit: `80d517b228fac3290fc83b06d3a873117d13b090`.
- Prefix: `supermarket-ruby/`. Remote name: `upstream-supermarket`. Branch that ships: `main`.
- Import with **no** `--squash`; history must survive.
- Never edit files inside `supermarket-ruby/` except our own `UPSTREAM.md`.
- Upstream is Apache-2.0: its `LICENSE` and headers stay untouched.

## Review Focus

- `git subtree pull --prefix=supermarket-ruby upstream-supermarket main` must
  still work after our `UPSTREAM.md` exists (our file must not sit where upstream
  has one).
- The import must be a merge commit whose parents include `80d517b2`, not a
  plain copy, so blame resolves.
- A fresh clone must need no extra steps to see the reference tree.
- Repo-level `.gitignore` must not swallow any upstream file.

---

### Task 1: Base commit

**Files:**
- Create: `README.md`, `.gitignore`
- Existing: `docs/superpowers/specs/2026-11-16-vendor-supermarket-ruby-design.md`, `docs/superpowers/plans/2026-11-16-vendor-supermarket-ruby.md`

**Interfaces:**
- Produces: `main` at a root commit, committed history that `git subtree add` can merge into; reference remote name `upstream-supermarket` and path `supermarket-ruby/` used by Tasks 2–3.

- [x] **Step 1: Write `.gitignore`**

Contents: a single line, `.DS_Store`. Nothing Ruby-specific — upstream has its own.

- [x] **Step 2: Write `README.md`**

Must state: this repo is a Rust port workspace for Chef Supermarket; `supermarket-ruby/` is upstream's Rails code, vendored read-only at a pinned commit and never to be edited in place; pointers to `supermarket-ruby/UPSTREAM.md`, the design doc, and the plan; the update recipe from Step 4; a Licensing section saying the root repo (once Rust code exists) is the port's own work, while `supermarket-ruby/` stays Apache-2.0 per its `LICENSE`.

- [x] **Step 3: Add the upstream remote**

Run: `git remote add upstream-supermarket https://github.com/chef/supermarket`
Expected: no output; `git remote -v` lists it.

- [x] **Step 4: Commit**

```bash
git add .gitignore README.md docs
git commit -m "chore: start supermarket-rs port workspace"
```

Expected: root commit on `main`; `git log --oneline` shows exactly one commit.

### Task 2: Subtree import

**Files:**
- Create: `supermarket-ruby/**` (upstream tree, untouched)

**Interfaces:**
- Consumes: branch `main` with at least one commit; remote `upstream-supermarket`.
- Produces: `supermarket-ruby/` populated at `80d517b2`, reachable history for that SHA.

- [x] **Step 1: Fetch upstream with full history**

Run: `git fetch --no-tags upstream-supermarket`
Expected: refs downloaded, including `upstream-supermarket/main`; exit 0.

- [x] **Step 2: Confirm the pinned commit is present**

Run: `git cat-file -t 80d517b228fac3290fc83b06d3a873117d13b090`
Expected: `commit`

- [x] **Step 3: Import as a subtree merge**

Run:
```bash
git subtree add --prefix=supermarket-ruby 80d517b228fac3290fc83b06d3a873117d13b090
```
Expected: a merge commit is created ("Add 'supermarket-ruby/' from commit …").

- [x] **Step 4: Verify the import**

Run:
```bash
git log --oneline -1
git rev-list --parents -1 HEAD
git diff --quiet 80d517b228fac3290fc83b06d3a873117d13b090^{tree} HEAD:supermarket-ruby && echo tree-identical
ls supermarket-ruby | head -30
test -f supermarket-ruby/src/supermarket/Gemfile && test -f supermarket-ruby/LICENSE && echo tree-ok
git status --porcelain
```
Expected: newest commit merges `80d517b2…`; `rev-list --parents` shows two parents: the base commit and `80d517b2…`; `tree-identical` (imported content byte-equal to upstream); tree shows the Rails app; `tree-ok`; clean status.

- [x] **Step 5: Fix `.gitignore` only if the status in Step 4 is dirty**

If any upstream file is ignored, add the smallest negation for it and `git add` it; commit as `fix: keep upstream files unignored`. Otherwise skip.

### Task 3: Provenance and pull check

**Files:**
- Create: `supermarket-ruby/UPSTREAM.md`

**Interfaces:**
- Consumes: imported tree and history from Task 2.
- Produces: reproducible update recipe validated against the real remote.

- [x] **Step 1: Write `supermarket-ruby/UPSTREAM.md`**

Must contain: source URL, pinned SHA, import date `2026-11-16`, Apache-2.0 note pointing at the same directory's `LICENSE`, "do not edit upstream files here", and the update recipe:

```bash
git fetch upstream-supermarket
git subtree pull --prefix=supermarket-ruby upstream-supermarket main
```

- [x] **Step 2: Commit**

```bash
git add supermarket-ruby/UPSTREAM.md
git commit -m "docs: record upstream provenance and update recipe"
```

- [x] **Step 3: Prove the pull path works**

Run: `git fetch upstream-supermarket && git subtree pull --prefix=supermarket-ruby upstream-supermarket main`
Expected: either "Subtree is already up to date", or a fast-forward/merge that leaves `git status` clean and `UPSTREAM.md` present (`ls supermarket-ruby/UPSTREAM.md`).

- [x] **Step 4: Verify a fresh clone sees the tree**

Run: `git clone . /tmp/sm-clone-check && test -f /tmp/sm-clone-check/supermarket-ruby/src/supermarket/Gemfile && echo clone-ok; rm -rf /tmp/sm-clone-check`
Expected: `clone-ok`

- [x] **Step 5: Re-sync UPSTREAM.md if Step 3 moved the pinned SHA**

If the pull advanced the tree, update the SHA and date in `UPSTREAM.md`, then `git commit -am "docs: bump pinned upstream commit"`.
