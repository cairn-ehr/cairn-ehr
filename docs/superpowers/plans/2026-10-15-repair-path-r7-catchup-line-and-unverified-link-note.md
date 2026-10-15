# Repair path R7: an honest banner while catching up, and a note on an unverified link (#742, #743 part 2)

**Goal:** close the two places R5a's review found where the window says less than the node knows.
(1) A checked chart's banner says "not yet known" while a `change` notice waits (#742). (2) A link
whose pair carries a `degrade_hold` identifier finding is marked unverified in the links list,
without holding anything (#743 part 2).

**Design:** the design page's section *R7 — designed 2026-10-15*
(`docs/superpowers/specs/2026-09-27-duplicate-repair-path-679-680-681-design.md`). Both decisions
are the maintainer's (2026-10-15).

**Architecture:** in-DB reads (db/056, db/054, edited in place; ADR-0080's hash gate replays them;
`SCHEMA_GENERATION` stays 58) → `cairn-node` read functions (`duplicate_check.rs`,
`patient/edges.rs`) → `cairn-gui-tauri` pure view functions (`duplicates/view.rs::check_lines`,
`link/record_links.rs::links_section`). No event type, no wire change, no ADR.

**House rules in force:** TDD (red first, for every behaviour); commit messages say `Refs #742` /
`Refs #743` or `fix(#742):`, never a closing keyword, and `scripts/check_closing_keywords.py` runs
before each commit; a new `LocalDbFault`-wrapped read in `edges.rs` bumps
`db_errors_stay_legible.rs`'s per-file count; DB-suite runs are checked with `--nocapture` for
`skipped:` lines; the full gate is `scripts/run-db-gated-tests.sh` plus the `cairn-gui` tree's.

## Task 1: db/056 reports whether a `change` notice waits (#742)

- **Test first** (`crates/cairn-node/tests/match_pending.rs`): with a worker row and one `config`
  notice, `charts_changing = 0`; add a `change` notice for another chart, `charts_changing = 1`; two
  notices for one chart count once. Heal test: a database carrying the pre-R7 shape (with
  `quiet_age_s` but no `charts_changing`) is healed on connect.
- **Build:** `OUT charts_changing bigint` = `count(DISTINCT patient_id) FILTER (WHERE reason =
  'change')`; the DO-block drops any existing `cairn_duplicate_check_status()` lacking
  `charts_changing` among its `proargnames`.

## Task 2: `CheckState::CatchingUp` carries `change_waiting` (#742)

- **Test first** (`duplicate_check.rs` unit tests): `classify` sets `change_waiting` from
  `charts_changing > 0`; `status_line`'s goldens are byte-identical.
- **Build:** `QueueSnapshot::charts_changing`, read by `read_snapshot`; the new variant field.

## Task 3: the banner explains a waiting change on a checked chart (#742)

- **Test first** (`cairn-gui-tauri/src/duplicates/view_tests.rs`): a checked record on a
  `CatchingUp` node with `change_waiting` shows the status line; with only a `config` re-check
  (`change_waiting: false`) shows nothing; `Current` still never appends it; a pending line still
  brings it, as before.
- **Build:** `check_lines`'s `explains` arm for `CatchingUp`.

## Task 4: db/054 lists un-attested links over a `degrade_hold` pair (#743 part 2)

- **Test first** (new `crates/cairn-node/tests/unverified_link_note.rs`): two charts whose identifiers
  in one system differ with no `normalized` form on one side, joined by an un-attested standing
  link → one row naming the system; the same link attested → none; a `hard_veto` pair → the function
  may list it (Rust drops it beside the clash), and the R1b hold stays FALSE for the `degrade_hold`
  set (`record_is_doubted`), so the note cannot become a hold. `record_unverified_links` reads it.
- **Build:** `cairn_chart_set_unverified_links(uuid[])`, SECURITY DEFINER, `search_path` pinned,
  EXECUTE revoked from PUBLIC and granted to `cairn_agent`, as its db/054 siblings;
  `patient::edges::{UnverifiedLink, record_unverified_links}`.

## Task 5: the links list appends the note (#743 part 2)

- **Test first** (`record_links.rs` unit tests): an un-attested link with an unverified row gets the
  note naming the system; an attested link does not; a clash line keeps only its clash note; a note
  naming no standing edge is dropped and the list still draws; an undoubted, un-noted line is
  byte-identical to before.
- **Build:** `RecordLinks::unverified`, read in `read_record_links`'s one snapshot; `links_section`.

## Task 6: docs and gates

Design page "R7 — as built"; HANDOVER and ROADMAP (pruned); the full DB gate and the `cairn-gui`
gate in CI's order after the final edit; a draft PR.

## Paper-parity benchmark (§1.2)

- **Paper counterpart:** the records clerk's tray note "duplicate check still being done" clipped to
  the front of a folder; and a sticky note on two clipped folders saying "their hospital numbers
  differ, and I can't tell whether it is only how they were written".
- **Steps:** reading either line adds no act: paper N = 0 (glance at the note) → architecture-forced
  M = 0 → UI target K = 0. Acting on the unverified note is R6's existing "Review this link…" → confirm
  or unlink (N = 2 → M = 2 → K = 2, budget ≤ 15 s, RUNBOOK §13). `M ≤ N` throughout.
- **Time + cognitive load:** the status line rides the status read the banner already makes (no new
  statement, so the chart open stays ≤ the single-chart open); the unverified note rides the links
  list's existing read-only snapshot (one more statement in the same transaction, a linked record
  only). The load is one sentence per line, shown only when it is true. Measured with RUNBOOK §11 and
  §13, a human act owed.
