# DR slice 2d's six unwritten design tests — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:test-driven-development. This is a
> TEST-ONLY slice over behaviour that already shipped, so "watch it fail" means: every test here is
> run against a **deliberately mutated** build and seen to go red for the stated reason before it is
> trusted. A pin that only ever passed proves nothing.

**Goal:** Close [#593](https://github.com/cairn-ehr/cairn-ehr/issues/593) — the six tests DR slice
2d's design §7 lists and PR #566 merged without: **7, 14, 16, 17, 19, 22**. (Test 23 was already
written by PR #574; HANDOVER's list of owed tests was stale on that one.)

**Architecture:** No production code is planned. Shared restore fixtures move out of
`restore_cli_surface.rs` (752 lines) into `tests/common/restore_kit.rs`, included by `#[path]` —
the convention `clinic_kit.rs` set for #567. Four new test files, each under 500 lines:

| File | Tests | Level |
|---|---|---|
| `restore_pen_is_uncapped.rs` | 7 | library (`apply_clinical_plane`) |
| `restore_one_event_id_one_body.rs` | 16 | library, through the real derivation |
| `restore_cli_applies_nothing_untrusted.rs` | 14, 17 | the `cairn-node restore` binary |
| `restore_cli_survives_its_own_failure.rs` | 19, 22 | the `cairn-node restore` binary |

If a test finds a defect, the fix lands in this slice (or is filed, house rule 5) and this plan is
amended — the test is never weakened to match the code.

**Spec:** `docs/superpowers/specs/2026-09-09-dr-slice-2d-restore-reads-the-clinical-plane-design.md`
§7. No decision is added here.

Paper-parity: not clinical-surface — test-only pins over an operator recovery command whose §1.2
standing (#512) is unchanged; no workflow step is added, removed or reordered.

## Global constraints

- AGPL-3.0; no new dependency.
- House rule 6: no literal key material; `lineage`/`variant`, never `salt`/`nonce`/`iv`.
- DB-gated tests read `CAIRN_TEST_PG` and take `db::test_serial_guard`. A fault-injection trigger
  is scoped to one event and dropped **before** any assertion, so a failing run cannot poison later
  suites (`attachment_reference_shape.rs`'s pattern).
- Commits use `test(#593):` (the parenthesis breaks GitHub's closing-keyword adjacency).
- `cargo fmt --check` and `RUSTDOCFLAGS=-D warnings cargo doc` in every commit step, not only the
  final gate (the #567 lesson).

## The tests, and the mutation each must kill

**Test 7 — the pen is not capped, at volume.** 10 001 records that each carry custody, applied with
no custody key installed, must ALL be penned under `(restore)` with their `dek_wrapped` intact, and
the report must say the ordinary quota was exceeded. *Mutation:* `pen()` passes
`ORDINARY_QUOTA_ROWS`/`ORDINARY_QUOTA_BYTES` instead of `NULL, NULL` — the 10 001st pen raises and
the run fails. A handful of records passes against that mutation, which is why the volume is the test.

**Test 16 — one event id, one body.** Through `clinical_plane_accounting` over a real captured medium
plus an appended, correctly chained segment:
(a) the same event re-captured WITHOUT its key at the same `source_seq` (a straddle) is a no-op —
one `event_log` row, `already_present = 1`, nothing penned, the body still opens;
(b) a DIFFERENT body signed under the same `event_id` is refused as a substitution, penned with the
door's text, and the original body is what the chart reads.
*Mutations:* (a) `plane_records` collapses by `source_seq` alone — the straddle vanishes; (b) db/020's
substitution guard removed — the forged body counts as `applied`.

**Test 14 — nothing past `verified_through`, through `restore`.** Two captures; the second clinical
segment's chain link is broken. The first chart's body opens, the second chart's event is in neither
`event_log` nor the pen, and the untrusted warning prints on stderr AND in the stdout summary.
*Mutation:* `plane_records_with_accounting` ignores `verified_through`.

**Test 17 — named outcomes.** (a) a signed CAIRNB1/B2 medium restores its federation plane and says
it predates the clinical plane — never the CAIRNB3 "carries NO clinical records" line;
(b) a CAIRNB3 medium with an appended `Plane::Unknown` segment of two validly signed events notes
"2 record(s) in a plane this build does not recognise", and neither event enters `event_log`.
*Mutations:* delete the `Legacy` summary arm; delete the unknown-plane note.

**Test 19 — the order is load-bearing, behaviourally.** A temporary trigger raises `disk_full`
(53100, not a door verdict) on one sealed event's insert. The first restore fails, and `local_node`
is EMPTY. The trigger is dropped; the SAME restore into the SAME database completes, the body opens,
and exactly one `local_node` row exists. *Mutation:* `finalize_identity` moved above the clinical
apply (the "minimal reordering" design §3 rejects) — the first run leaves an identity behind.

**Test 22 — the summary survives the failure, counted by reason.** A medium whose newest segment
carries one record whose `dek_wrapped` will not open and one record signed by an unenrolled key: the
restore exits non-zero, prints BOTH reason lines with a count of 1 each, and still prints
`new node` / `supersedes` / `re-peer with` before exiting. *Mutations:* the pen bail moved above
the summary; the per-reason loop deleted.
