# `requeue` releases custody, and something proves it — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:test-driven-development. This is a
> TEST-ONLY slice, so "the failing test first" is the whole slice: every test here must be watched
> failing against a deliberately broken build before it is trusted.

**Goal:** Close [#568](https://github.com/cairn-ehr/cairn-ehr/issues/568). `do_requeue`'s
custody-carrying arm — the remedy every restore-penned reason advertises — has no test: every call
site in the suite passes `None` for the unwrap secret, and `cmd_requeue`'s `--key` plumbing has none
at all. Give both a DB-gated test whose assertion is that **a sealed body OPENS**, not that a row
exists.

**Architecture:** One new integration-test file, `crates/cairn-sync/tests/requeue_releases_custody.rs`,
driving the **real `cairn-sync` binary**. No production code changes are planned; if a test finds a
defect, the fix lands here and the plan is amended rather than the test weakened.

**Tech Stack:** Rust, `postgres`/`tokio-postgres`, `tempfile`, PostgreSQL 18 + `cairn_pgx`. No new
dependency.

**Spec:** None. This slice writes down behaviour that ADR-0067 already decided and
`db/052_restore_doors.sql` already implements; it adds no decision of its own.

Paper-parity: not clinical-surface — this slice ships no clinician-reachable workflow. It adds tests
for an operator recovery command (`cairn-sync requeue`) whose own §1.2 standing is unchanged by
testing it, and it changes no step count on any path a clinician walks.

## Global Constraints

- **AGPL-3.0.** No dependency is added. Every crate used is already a dev-dependency of `cairn-sync`.
- **TDD, without exception.** Each test is watched failing for the *stated* reason before the
  assertion is trusted. For a test-only slice the discipline is inverted and stricter: a test that
  passes the first time proves nothing until a deliberate mutation has been shown to make it fail.
  **Every test here names the mutation it kills, and that mutation is actually run.**
- **Inline documentation for a junior developer.** Every fixture carries *why it exists and how it
  fits*. The file header carries the failure scenario in full — an operator's disk is gone, the pen
  is the only copy, and a release without custody is permanent.
- **Files stay under 500 lines where feasible.** The new file is the reason this work does NOT go
  into `crates/cairn-sync/src/main.rs`, which is 13 484 lines and already has two open issues asking
  for it to be decomposed (**#531**, **#329**).
- **Never hard-code cryptographic material, and never give a non-cryptographic value a
  cryptographic name** (house rule 6). Every key here comes from `generate_key` /
  `generate_unwrap_secret` at runtime. The words `salt`, `nonce`, `iv` are reserved for real
  constructions; a discriminator is a `lineage` or a `variant`.
- **DB-gated:** reads `CAIRN_TEST_PG`, takes `cairn_node::db::test_serial_guard(&base)` — advisory
  locks are per-database, not cluster-wide (#476). A DB-free `cargo test` needs
  `CAIRN_ALLOW_DB_SKIP=1` since #450.
- **The gate is the FULL workspace.** `cargo test --workspace`, never piped through `tail` (that
  masks cargo's exit code).
- **Commit messages** use the `test(#568):` form — the parenthesis breaks GitHub's closing-keyword
  adjacency — and end with
  `Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>`.

---

## Why the CLI route, and what it does and does not cover

`do_requeue` is private to a **binary** crate, so a direct-call test can only live inside
`main.rs`'s `#[cfg(test)]` module, which has no sealed-event fixture at all — it would mean building
one (seal, register custody, enroll, chart-registration precedence) from scratch inside the largest
file in the tree.

Driving `cairn-sync requeue --key` instead reaches the same arm **through the shipped surface**, and
covers in one pass both halves #568 asks for: the custody arm inside `do_requeue`, and
`cmd_requeue`'s key-selection plumbing, *"where a wrong default would live"*. That plumbing cannot be
reached from a direct call at all.

**What this route cannot isolate,** stated so nobody later reads more into a green suite: it cannot
distinguish a fault inside `do_requeue` from one inside `cmd_requeue`'s resolution — only that the
composed command is right. Task 4 narrows that by driving the two failure arms separately.

## One correction to #568's own text, carried into the code

#568 says *"counting an `event_dek` row is not enough; that is exactly the assertion that would pass
under a double-wrap."* Against **this** door it would not. `db/020`'s step 8 (`IF v_sealed AND p_dek IS NOT NULL`) unseals with `p_dek`
first, and a double-wrapped value fails that unseal, which sets `v_inner := NULL` and skips the whole
custody block — so a double-wrap leaves **no `event_dek` row and no `event_clear` row**, and a count
would catch it.

The instruction is still right, for a better reason: the twin assertion states the **property**
(*the body opens*) rather than a side effect of it, and it survives a future door that writes custody
before proving it can be used. Task 3 asserts both, twin first.

---

## File Structure

**Created:**
- `crates/cairn-sync/tests/requeue_releases_custody.rs` — the whole slice.

**Modified:** none expected.

---

## Tasks

### Task 1 — The fixture: a restored node whose pen is the only copy

- [ ] Provision one node against `CAIRN_TEST_PG`: schema, serial guard, a runtime-generated signing
      key written hex into a `TempDir`, its DERIVED unwrap public key registered via
      `cairn_register_unwrap_key` (derived, not generated — `resolve_at_startup`'s fallback is what
      lets `--key` alone resolve custody, and `clinical_pull.rs` explains why cairn-sync's fixtures
      derive where cairn-node's provision).
- [ ] Register a chart (§5.3/§5.8 precedence, #345) and author ONE real born-sealed
      `clinical.medication.asserted` through `cairn_node::medication::assert_medication`.
- [ ] Read back the four things the medium would carry: `event_log.signed_bytes`, its content
      digest, `event_dek.dek_wrapped`, and `event_clear.twin`. **Assert the twin is non-empty here**
      — if the source node cannot read its own chart the rest of the file is measuring nothing.
- [ ] Wipe the clinical tier (`event_log`, `event_dek`, `event_clear`, `patient_chart`, …), leaving
      the node enrolled and its custody registered: the state a restore reaches.
- [ ] Pen the event through the real door, `cairn_quarantine_event(...)`, with its `dek_wrapped`,
      the restore peer sentinel and a `restore:` reason — never a raw INSERT.

### Task 2 — RED: watch the headline fail for the right reason

- [ ] Write `a_penned_sealed_record_releases_with_its_custody_and_the_body_opens`, run it
      against a build where `do_requeue`'s unwrap is mutated to pass `wrapped` through unchanged
      (the double-wrap), and confirm it fails on the TWIN assertion, not on a count or a panic.
- [ ] Revert the mutation. Record the observed failure text in the test's doc comment.

### Task 3 — GREEN: the headline

- [ ] Run the real binary: `cairn-sync requeue --conn <base> --key <path> --metrics`.
- [ ] Assert, in this order: exit status 0 · `released == 1`, `still_quarantined == 0` · the pen row
      is gone · `event_clear.twin` equals the twin the dead node held · `event_dek.dek_wrapped`
      opens with the node's unwrap secret. Twin first: it is the property, the rest are its traces.

### Task 4 — The two degradation arms, separately

- [ ] `without_resolvable_custody_the_record_still_releases_but_stays_sealed`: run with a
      `--key` naming a DIFFERENT node's key file. `resolve_at_startup` refuses (the derived key does
      not match the registered one), `cmd_requeue` degrades **best-effort** and warns, and the event
      still releases at exit 0 with NO `event_clear` row. This is the anti-vacuity twin of Task 3 —
      without it, a suite that never opened anything would still be green.
- [ ] `a_penned_dek_from_another_node_releases_the_record_and_says_custody_was_lost`: pen the DEK
      re-wrapped for a STRANGER's unwrap public key, run with this node's REAL key. Custody
      resolves, the unwrap fails, and `do_requeue`'s `Err(_)` arm fires — the only one of the three
      arms the other two tests never reach. Assert the warning names the digest.

### Task 5 — The plumbing guard

- [ ] `requeue_refuses_a_missing_key_file_rather_than_minting_one`: `--key` at a path that does not
      exist. Assert the command still releases the event (it is a recovery command; aborting before
      releasing anything is worse), that it warns, and — the point — that **no key file was created
      at that path**. `cmd_requeue`'s doc names this as the reason it calls `load_existing_key`
      rather than `load_or_create_key`; nothing checked it.

### Task 6 — The gate and the docs

- [ ] `cargo fmt --check`, `cargo clippy -- -D warnings`, then the full workspace gate via
      `scripts/run-db-gated-tests.sh`.
- [ ] HANDOVER + ROADMAP record the slice; #568 is named as closed by the PR body, never by a
      commit-message keyword adjacent to the number.

---

## Amendment (2026-09-12, after review)

The plan said *"if a test finds a defect, the fix lands here and the plan is amended rather than the
test weakened."* The review found defects in the **tests**, not via them, and this is what changed.

### A fifth test, because the arms were indexed by the wrong thing

The plan enumerated `do_requeue`'s custody decision as **three outcomes**. That is true of the
outcome set and false of the input set: the match's `_` arm absorbs three distinct
`(unwrap_secret, penned_dek)` pairs into one silent `None`, and only `(None, Some)` was tested.
`(Some, None)` — a keyless pen row while custody resolves — is the shape **every plaintext pull
creates**, so the untested pair was the modal one outside disaster recovery.

Added `a_keyless_pen_row_releases_quietly_and_raises_no_custody_alarm`. Its load-bearing assertion
is negative: **no custody warning is printed**, because nothing went wrong. An operator trained by
false alarms stops reading the message on the run where it is real. `pen_with_custody` became
`pen(…, Option<&[u8]>)` to express a keyless row through the same real door.

### Every degraded arm now asks the log, not the tool

Arms 2, 3 and 4 each claimed *"a recovery command must still recover the EVENT"* and proved it only
with `released` — a counter **inside the function under test** — while their sole database
assertion was the negative `twin == None`, which is equally consistent with the event never having
been applied. `event_survived` (an `EXISTS` over `event_log`) and `pen_rows` now back every arm.

### Two mutations survived a first draft, not one

The plan anticipated this discipline and it earned its keep twice:

* **Mutation 4** (recorded in the original file header): the missing-key test named a path inside a
  directory that did not exist either, so the minting loader failed on the absent parent rather
  than being refused.
* **Mutation 7** (new): written first as *"move the `DELETE` ahead of `apply_signed` and swallow the
  error"*, it **could not fail** — every arm feeds the door bytes it accepts, so the apply succeeds
  and the reordering changes nothing observable. Modelling the claim required skipping the door
  entirely. The ordering defect it failed to model is therefore still uncovered here, and that is
  stated in the file header rather than papered over.

### Coverage boundaries moved into the file header

`--unwrap-key` and the whole `FileOutcome::Loaded` path remain untested (every arm resolves through
the derived fallback), every arm pens exactly one record, and the CLI route cannot isolate a fault
inside `do_requeue` from one inside the resolution. All three were known and stated only here, in
the disposable artifact. They are now in the durable one.

### Four production defects filed, none fixed here

The slice stays test-only. **#578** (requeue deletes the pen row when custody did not land —
silently, exit 0; the guard exists one module over as `custody_landed`), **#579** (the metrics
object cannot express custody loss), **#580** (the warning promises the pen holds both halves, then
empties it), **#581** (a comment claims `do_requeue` skips acked rows, and the unwrap error's
malformed-vs-foreign distinction is discarded).
