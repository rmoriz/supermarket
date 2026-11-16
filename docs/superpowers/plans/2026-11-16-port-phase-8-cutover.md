# Phase 8 — cutover

> **For agentic workers:** REQUIRED SUB-SKILL: use `subagent-driven-development`
> or `executing-plans`. Steps use `- [ ]` checkboxes.

**Goal:** Put a switchable proxy in front of Rails and the port, prove every
route group flips and flips back, shadow the two cache-heavy reads, and hand
over a runbook for the production window. Schema stays frozen; Rails keeps
serving writes until the upload window.

**Spec:** `docs/superpowers/specs/2026-11-16-supermarket-rs-port-design.md`,
sections *Cutover, rollback* and *Out of scope*.

**Runbook:** `docs/cutover-runbook.md` is the operator-facing copy of this plan.

## Gate to done

- `cargo build --bin strangler` and `cargo clippy --all-targets -D warnings`
  clean.
- Proxy smoke test: a flipped group answers from Rust, an unflipped group still
  answers from Rails, and both revert by config.
- Shadow mode serves Rails while the mirror answers and logs mismatches.
- Corpus and unit tests green with the proxy in front of the Rust binary.
- Runbook reviewed; the phase-8 ledger records the flips.

## Tasks

- [x] **1. Proxy binary.** `src/bin/strangler.rs`: classify every request into a
      route group, forward to the backend selected by `STRANGLER_GROUPS`, mirror
      `STRANGLER_SHADOW` groups to the other backend, health endpoint, group
      unit tests.
- [x] **2. Build and lint.** `cargo build --bin strangler`,
      `cargo clippy --all-targets -D warnings`; fix the `bytes`/`HeaderMap`
      iteration mistakes and the borrow in the shadow task.
- [x] **3. Flip smoke test.** Run on port 3002 with `STRANGLER_GROUPS=chat`:
      `/chat` must fail 502 when Rust is stopped and `/api/v1/cookbooks` must
      stay 200, both revert when Rust restarts.
- [x] **4. Shadow smoke test.** `STRANGLER_SHADOW=feeds`: `/universe` is served
      by Rails while Rust is fetched; the log records the mirror attempt.
- [x] **5. Runbook.** Write `docs/cutover-runbook.md`: flip order, shadow
      protocol, write window, rollback by config, 30-day S3 grace, proxy
      removal, ledger obligations.
- [x] **6. Full verification.** `cargo fmt --check`, clippy, `cargo test`, and
      the whole corpus with `RUST_URL=http://127.0.0.1:3005`.
- [x] **7. Ledger.** Append the phase-8 entry with the smoke results and the
      final corpus table.

## Notes

- The oracle in this environment fails `/universe` with an incompatible
  marshal file (Ruby marshalling version skew in the local image). Rust answers
  200. The shadow paths still exercise correctly; note the expected mismatch in
  the ledger.
- Groups are read at process start only, so the switch is a strangle restart,
  never a deploy. That is deliberate: a rollback must not depend on a build.
