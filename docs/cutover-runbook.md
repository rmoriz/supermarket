# Cutover runbook — Rails → supermarket-rs

Short, reversible rollout of the Rust port in front of the Rails app. Nothing
here changes the database schema, so every step has a lossless rollback: flip
the group back and Rails serves it again. The only irreversible act is deleting
the strangler, and that happens last.

**Roles:** one release engineer drives the flips; a second person watches the
error budget and the shadow diff log and can call a rollback by flipping config.

## 0. Preconditions

- Rails is still the writer for the schema. Rust has run read-only against the
  production database for at least 48 h with an empty error log.
- `supermarket-rs` is deployed next to Rails, reachable only from the proxy,
  with `ENFORCE_PRIVACY` and the same feature flags as Rails.
- The strangler (`target/release/strangler`) is deployed in front of both, with
  `RAILS_URL` and `RUST_URL` pointing at the two backends and **no** groups
  flipped yet. All public traffic now traverses it and is served by Rails.
- `/_strangler/health` returns `ok` and the backend pair answers as today.
- The differential corpus and the Ruby feature specs are green against the
  current build. Record the run in the phase-8 ledger.

## 1. Flip order

Groups flip in reverse blast radius (spec, *Cutover/rollback*). After each flip,
watch the error rate and the Rust log for 15 minutes:

```sh
# 3002 is the strangleport; the list is read at start-up only.
PORT=3002 RAILS_URL=… RUST_URL=… STRANGLER_GROUPS=chat <strangler>
```

1. `chat` — pure redirects, no writes.
2. `tools` — tool CRUD; the signed-in tool mutations are the risk, watch the
   create/update/destroy paths in the Rust log.
3. `feeds` — `/robots*`, `/universe`, `/api/v1/universe`. `universe` and search
   are **shadowed first** (step 2) and only then flipped.
4. `ui` — the whole HTML surface. Biggest surface, still read-mostly mistakes.
5. `reads` — the GET API.
6. `uploads` — `POST/DELETE /api/v1/cookbooks*`, fieri intake, quality metrics.

`STRANGLER_GROUPS` accumulates; there is no per-group state on disk.

## 2. Shadow before serving `universe` and search

Keep Rails serving, mirror to Rust, diff:

```sh
STRANGLER_SHADOW=feeds,reads <strangler>
```

The proxy answers from Rails and fetches the Rust answer in the background. It
logs `shadow status mismatch`, `shadow content-type mismatch` and `shadow etag
mismatch` with the path. Serve the shadowed groups only after a full day
without a mismatch, or after each mismatch is understood and recorded in the
ledger.

Clear `STRANGLER_SHADOW` entries once a group is flipped; a shadowed group is
never served from Rust.

## 3. Write window

Uploads and the signed API flip last, in a short announced window:

1. Stop Rails' write paths: point the Rails job workers at a paused queue and
   put the Rails app in read-only mode (or stop the Rails web dynos for web
   writes only, keeping workers up to drain).
2. Flip `uploads`. Both apps share one database, so no data migration runs.
3. Smoke the API by hand: `POST /api/v1/cookbooks` with a signed artifact, one
   `DELETE`, one fieri job, one quality-metric POST. Confirm rows, stored
   artifacts and jobs land.
4. Unpause the workers; they now read jobs written by Rust, which use the same
   payload classes and keys.

If a smoke fails, flip `uploads` back, resume Rails writes, rerun the corpus
against the failure, and repeat the window another day.

## 4. Rollback

Always one config edit and a strangle restart, no deploy:

- Bad group → remove it from `STRANGLER_GROUPS`, restart the strangle. Rails
  serves it again within seconds.
- Bad build → restart the strangle with an empty group list; everything is
  Rails again.
- Keep the Ruby dynos and workers deployed until step 5 completes. Never scale
  Rails below one instance during the rollout.

Because the schema never diverged, a rollback needs no data repair. Uploaded
artifacts stay where the database says they are and rows stay in Postgres; both
apps read the same storage. Without `S3_BUCKET`/`S3_REGION` that is
`public/system`, paperclip's own filesystem path, so the two share a directory
and each reads what the other wrote. With the pair set, both use the same bucket,
key layout, ACL and URL shape, so a rollback keeps resolving every artifact.
`S3_ACCESS_KEY_ID`/`S3_SECRET_ACCESS_KEY` must be set on both sides or on
neither; otherwise one side talks to the bucket with static keys and the other
through its instance role.

## 5. S3 grace period

Both backends delete the object when a version or a cookbook is destroyed
(paperclip's `flush_deletes`, and the port's `delete_tarball` on both destroy
paths), so a deleted version loses its artifact on either side. For the window
to survive a rollback, turn bucket versioning on and keep noncurrent versions
for 30 days with a lifecycle rule before switching traffic: the new backend's
delete then only writes a delete marker and the old one's objects are still
recoverable. Without versioning a delete is final, exactly as it is on the
filesystem — there the grace period is only "do not prune `public/system`".

## 6. Proxy removal

After the full traffic mix has run on Rust for 30 days with the error budget
green:

1. Point DNS/ingress at Rust directly; leave the strangle deployed and idle for
   one more day as the rollback path.
2. Remove the strangle and scale Rails down to zero web dynos, keeping the
   Ruby workers until the job backlog is drained.
3. Move `db/schema.rb` ownership to `sqlx` migrations; from here on schema
   changes are Rust-first.
4. Unmount the `supermarket-ruby/` subtree once the 30-day object grace ends.

## 7. Ledger

Record for every flip: timestamp, group, amount of shadow diff data (if any),
mismatches understood, error-rate delta, and who pulled the trigger. A phase-8
ledger entry is only closed with the final "Rails web removed" line.
