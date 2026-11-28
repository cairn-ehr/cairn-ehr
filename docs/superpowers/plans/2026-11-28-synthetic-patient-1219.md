# #1219 — a patient is marked synthetic at registration — Implementation Plan

**Goal:** A registrar, a trainer or an importer can register an invented patient; the chart then says so
on every surface that names it; no link ever joins it to a chart of a real person; and it synchronises
like any other chart.

**Decision:** [ADR-0115](../../spec/decisions/0115-a-patient-is-marked-synthetic-at-registration.md)
(maintainer, 2026-11-28), which supersedes ADR-0114. Read its "Not guaranteed" section first.

**Architecture:** One additive field on `identity.registration.asserted`. The registration projection
(db/045) gains a column; one SQL function answers "is this chart synthetic, real, or not yet known
here", and one more answers "do these two charts straddle the line". Everything else reads those two.
No clinical event, no node event and no wire format changes.

**Tech stack:** PostgreSQL 18 SQL (db/088 new; db/018, db/045 and db/054 in place, as ADR-0080 allows),
Rust (`cairn-event`, `cairn-patient-search`, `cairn-node`, `cairn-gui`), Python (the matcher's pipeline,
the cohort importer).

## Premises, and how each was established

ADR-0114 was accepted on a premise read from the code and never run (#1220). So each thing this plan
rests on is listed with how it is known. **A row marked READ is a task before it is a foundation.**

| # | Premise | How known |
|---|---|---|
| 1 | Both doors admit a registration carrying a payload field the build does not know, register the chart, and keep the field in the stored body | **RUN**: `tests/registration_tolerates_an_unknown_field.rs`, written for this plan, green against PG 18 on `main` |
| 2 | The floor does not compare a registration's twin with a derivation of its own, so a later build may word it differently | **RUN**: the same test submits a reworded twin |
| 3 | A link arriving from a peer is ADMITTED whatever local state says, and doubt is expressed by withholding its effect at read time | **RUN** (Task 0): `link_veto_floor.rs::remote_vetoed_link_admitted_flagged_and_under_review`, already in the tree, green |
| 4 | A human-attested link passes the existing hard veto, so ADR-0115's refusal needs a rule an attestation does NOT lift | **RUN** (Task 0): `link_veto_floor.rs::human_attested_link_with_hard_veto_still_passes` and `agent_link_with_hard_veto_refused_at_local_door`, green |
| 5 | `patient_registration_current` picks one registration per chart, the EARLIEST, whatever the arrival order | **RUN** (Task 0): `patient_registration.rs::the_projection_keeps_every_registration_and_the_view_picks_the_earliest`, green. Two registrations that disagree about the mark cannot be tested until the field exists; that is Task 1's |
| 6 | The search door returns chart ids only, and a candidate's details are read separately, so a new candidate fact needs no change to a door a window already calls | **CHECKED** (Task 0) against `db/window_door_surface.txt`, which `window_door_surface.rs` holds equal to the live catalogue: `cairn_search_candidates(text[],text,jsonb) -> TABLE(patient_id uuid, matched_pass text)` |
| 7 | The window's registration reaches the node through `cairn_registration_submit`, whose argument list is fixed, so carrying the mark needs an additive door (ADR-0111) | **CHECKED** (Task 0): the snapshot lists `cairn_registration_submit(uuid,text,jsonb,jsonb,text) -> text`. Task 4 decides the new door's shape |

## Global constraints

- **Three answers, never two** (principle 4). A chart is `synthetic`, `real`, or `unknown` here: its
  registration has not arrived on this node (a peer's clinical event legitimately precedes it). No code
  treats `unknown` as `real`. Where a surface must show one thing, `unknown` shows as "registration not
  yet received", which is true.
- **One home each.** `cairn_chart_synthetic(uuid)` and `cairn_charts_straddle_synthetic_line(uuid, uuid)`
  are the only places SQL reads the mark. No door, view or Rust function reads the column for itself
  (the mirror-list defect class: #182, #404, #441).
- **The remote door never refuses on the mark.** A node cannot know a peer's verdict, and a refusal
  would fork the event set. It admits, and the effect is withheld
  ([ADR-0064](../../spec/decisions/0064-admit-the-claim-withhold-the-power.md)).
- **The local door refuses a cross-line link whoever signs it.** An attestation lifts the §5.2 veto; it
  does not lift this.
- **Absent means not marked.** A registration without the field is of a real person, and real
  registrations stay byte-identical to today's: the field is omitted, never `false`.
- **The wrong mark cannot yet be unmade** (ADR-0115, "Not guaranteed"). Every surface that sets the mark
  is designed so that setting it by accident takes more than one slip, and none uses a confirmation
  dialog to get there.
- A refusal is `P0001` through the existing helpers (#621). House rule 6 for key material in tests.
  Files stay under 500 lines. `SCHEMA_GENERATION` becomes **88** with its floor-ledger entry.
- The band and every mark wrap; they never clip.

## Task 0 — run the premises marked READ

> [!NOTE]
> **Done 2026-11-28; all five hold, and no new test was needed.** Each was already pinned by a test in the
> tree, which ran green in a full `cairn-node` run against PG 18 on `main`. The table above names them.
> **One thing Task 0 found that the plan had not said:** `db/window_door_surface.txt` lists the COLUMNS of
> `patient_registration` and `patient_registration_current` as window-readable. So Task 1's new column is a change
> to the window's surface (an additive one), and that snapshot and its header are regenerated in the same commit.

Nothing is built here. Each becomes a test that passes on `main`, kept as a pin.

- [ ] Premise 3 and 4, in `tests/identity_linkage.rs` or beside it: an un-attested link over a
      hard-veto pair is refused locally and admitted remotely; an attested one is admitted locally.
      Record the exact refusal text.
- [ ] Premise 5: two registrations for one chart at different HLCs; assert which one
      `patient_registration_current` returns, and that the answer is the same in either arrival order.
- [ ] Premise 6: `cairn_search_candidates`' signature is in `db/window_door_surface.txt` unchanged, and
      `candidate_read.rs` is the only reader of candidate details.
- [ ] If any of 3 to 6 is false, **stop and correct this plan before Task 1**.

> [!NOTE]
> **Tasks 1 and 2 are built** (PR #1222). Where the build differs from the text below, the code and its
> comments win: a malformed mark from a peer is read as MARKED (the visible direction) and not reported
> through a separate function; the two re-declared functions live in db/088 with a guard test holding each
> equal to its original plus one addition; and `db/tests/088_…sql` was not written, the Rust suite
> `synthetic_patient.rs` covering it.

## Task 1 — the mark in the event and the projection (`cairn-event`, db/088, db/045)

Tests first: unit tests in `cairn-event::registration`; `db/tests/088_synthetic_patient_test.sql`;
`tests/synthetic_registration.rs` (DB-gated).

- [ ] `RegistrationAssertion.synthetic: bool`. `registration_assertion_body` writes `"synthetic": true`
      only when set. A golden pins that a real registration's payload is byte-identical to today's.
- [ ] `render_registration_twin` begins `SYNTHETIC patient registered (…)` when set, so the mark is in
      the one line that is legible on any build, forever (principle 11).
- [ ] db/045 floor: `synthetic`, when present, must be the JSON boolean `true`. `false`, a string or
      null is refused by name: an explicit `false` would be a second spelling of "real".
- [ ] `patient_registration.synthetic BOOLEAN NOT NULL DEFAULT false`, filled by the apply function;
      `patient_registration_current` carries it. Existing rows read `false` with no backfill.
- [ ] db/088: `cairn_chart_synthetic(uuid) RETURNS text` giving `'synthetic'`, `'real'` or `'unknown'`
      (no registration held), `STABLE`, granted to `cairn_agent`.
- [ ] Two registrations of one chart that disagree: the earliest decides, as for every other fact of
      the birth act, and `cairn_chart_synthetic_conflict(uuid)` reports the chart so a surface can say
      so. Not a refusal at the remote door.
- [ ] Reprojection (db/039) rebuilds the column from the log; a registration with the field admitted
      by an OLDER build (premise 1) becomes marked when that node upgrades and reprojects. Tested by
      inserting through the door, dropping the column's value, and reprojecting.
- [ ] `SCHEMA_GENERATION` 88 and its ledger entry, **with the window floor left at 85**. No window can
      show the mark yet, so raising the floor here would lock every terminal out with no newer window
      to move to. The entry's `why` says that the floor is owed a raise by Task 5.

## Task 2 — no link across the line (db/088, db/018, db/054, db/016)

Tests first: `tests/synthetic_link_line.rs` (DB-gated), the matrix below, each cell a named test.

- [ ] `cairn_charts_straddle_synthetic_line(a, b) RETURNS boolean`: true only when one is `synthetic`
      and the other `real`. `unknown` on either side is false here and handled by the read (below):
      a door must not refuse on what it does not know.
- [ ] db/018 local door: refuse a `link` over a straddling pair, attested or not, with a message that
      says no signature lifts it and names the remedy (register correctly, then reattribute). An
      `unlink` over such a pair is always admitted.
- [ ] db/018 remote door: admit, and raise `link_veto_flag` for the pair as for a hard veto.
- [ ] db/054 `cairn_person_charts`: a link over a straddling pair never joins the set, evaluated at
      READ time, so a registration that arrives after the link still cuts it (the #220 shape). This is
      the floor under the two doors: whatever was admitted, the combined read never mixes them.
- [ ] `cairn_match_veto` gains a `synthetic_line` row at `hard_veto` severity, so every existing
      reader of `cairn_has_hard_veto` (auto-apply, the duplicate check, ReportTriage's resolver)
      refuses the pair without learning anything new.
- [ ] The matrix:

      | Chart A | Chart B | Local link | Remote link | Combined read |
      |---|---|---|---|---|
      | synthetic | real | refused, attested or not | admitted, flagged | never joined |
      | synthetic | synthetic | as today | as today | as today |
      | real | real | as today | as today | as today |
      | synthetic | unknown | admitted | admitted | not joined until B's registration arrives; then joined only if B is synthetic |
      | real | unknown | as today | as today | as today, and cut if B turns out synthetic |

- [ ] The last two rows change what an existing link over an unregistered chart reads as. Run the
      whole identity suite before and after and list every test whose outcome moved, in the PR.

## Task 3 — the matcher never proposes one (`matcher/`, db/017, `duplicate_check`)

Tests first, in `matcher/tests` and `tests/match_proposal*.rs`.

- [ ] Candidate-pair generation excludes straddling pairs, in the one SQL it runs; a proposal for one
      submitted by another client is refused at db/017.
- [ ] Two synthetic charts are still proposed and still linkable: duplicate-repair training needs it.
      A test holds this, because the easy implementation excludes anything synthetic.
- [ ] The duplicate sweep's yield (its miss-rate metric, ADR-0014) is reported with and without
      synthetic pairs (ADR-0115 decision 8).

## Task 4 — the doors that set the mark (CLI, the window's funnel, the terminal service)

Tests first. The page decides nothing; the wording and the mark are built in Rust.

- [x] `patient-register --synthetic` (built). **Differs from this line as first written:** the first
      two output lines are unchanged, because the RUNBOOK's `sed` and the cohort importer read the chart
      id from them; the mark is a THIRD line, `SYNTHETIC patient: not a real person. This cannot be
      changed.` It is a function of its own beside the ordinary registration, not a `bool` argument,
      so no call site can pass the wrong one.
- [ ] `register-john-doe --synthetic`. Not built.
- [ ] The window: a separate act, **Register a training patient**, not a tick box beside Register. A
      tick box is one mis-click from marking a real patient, and that mark cannot be unmade. The form
      shows the band from the moment it opens.
- [ ] The request to the terminal service carries the mark through an ADDITIVE door (premise 7): a new
      function beside `cairn_registration_submit`, never a changed argument list, with its row in
      `db/window_door_surface.txt` and the window-floor ledger (ADR-0111). An older window keeps
      registering real patients and cannot register a synthetic one.
- [ ] The search that precedes a registration shows candidates with their marks (Task 5), and a
      synthetic registration attests the displayed list like any other.

## Task 5 — it is shown wherever the patient is shown (`cairn-patient-search`, `cairn-gui`, CLI text)

Tests first: pure tests on the text and row builders; golden tests where a column layout is pinned.

- [ ] `Candidate.synthetic: Synthetic` (`Yes`, `No`, `Unknown`), read in `candidate_read.rs`.
- [ ] `candidate_text.rs`: a leading mark in the NAME cell, inside the ellipsized width, so no column
      moves and the mark is never the part that is cut.
- [ ] The chart: a permanent band, `SYNTHETIC PATIENT — not a real person`, above everything else on
      every tab. It cannot be dismissed. A chart whose registration is not held shows
      `Registration not yet received` in the same place, never nothing.
- [ ] Every other row that names a patient carries the mark. Enumerate them by search, not from
      memory, and list them in the PR: the person menu (ADR-0095), the funnel's prompts, the
      medication list's combined-read header, `medication-list`, `patient-sensitivity`,
      `patient-safety`, the duplicate-review worklist.
- [ ] **Raise the window floor** (ADR-0111) in the commit that gives the window the band, with
      `old_window_misses: Some("which patients are synthetic")`. An older window shows a training
      patient as a real one, which is exactly the misbehaviour the floor exists to stop: below the
      floor a terminal is told it needs its update and may open no further chart. This is the one
      place ADR-0115's "an older build shows it as real" CAN be closed, and only for the window; a
      node on an older build is still as ADR-0115 says.
- [ ] A source-level guard, in the manner of the existing inventory tests, that fails when a new
      builder of a patient row does not read `Candidate.synthetic` or `cairn_chart_synthetic`. Without
      it decision 7 holds only until the next surface is added.

## Task 6 — the loader and the sibling (`scripts/import_synthetic_cohort.py`, ReportTriage)

- [x] The importer registers with `--synthetic` and refuses a `cairn-node` too old to know the flag,
      naming the remedy. Its guard against a node holding other charts, and that guard's tests, are
      deleted together. It also checks that the node SAID the patient is synthetic and stops that
      patient if it did not.
- [x] A cohort loaded before this is built is unmarked. The importer detects such charts (they carry
      the cohort's record-number system and `cairn_chart_synthetic` says `real`), refuses to add to
      them, and says the database must be loaded again.
- [ ] An issue on ReportTriage: its `--synthetic` mode can check each chart, and its rule that a
      synthetic record is never read beside a real feed gets a per-chart fact.

## Not in this plan

- Reattribution ([§5.5](../../spec/identity.md#55-reattribution-one-primitive-tiered-workflows)), the
  only repair for a wrong mark. It is its own slice and should follow this one closely.
- Any change to what an older build shows (ADR-0115, "Not guaranteed").
- Partitioning identifiers between synthetic and real charts.
- Converting charts registered before this is built.

## Paper-parity benchmark (§1.2)

- **Paper counterpart:** a dummy chart with TRAINING stamped across its cover, in the same trolley as
  the real ones.
- **Steps:**
  - Making a training chart: paper N = 2 (take a blank chart, stamp it) → architecture M = 1 (one
    registration event carrying the mark) → UI K = 1 act, *Register a training patient*, in place of
    *Register*, not in addition to it.
  - Seeing that a chart is a training chart: N = 0 (the stamp is on the cover) → M = 0 → K = 0 (the band
    and the row mark).
  - Picking a patient from a list that holds both: N = 0 extra (the stamp shows on the spine) → K = 0
    extra (the mark is in the name cell).
  - Mis-stamping a real chart: on paper N = 1 to undo (a new cover). Here it cannot be undone until
    reattribution is built. **This is worse than paper and is filed as a defect of the architecture as
    it stands**, per house rule 5, against the reattribution slice; it is why Task 4 makes the mark a
    separate act.
- **Time + cognitive load:** the band is readable in under 1 s with no click and asks nothing. Marking
  adds no time to a registration. Budget: no measurable change to any clinical act on either kind of
  chart. Measured by a RUNBOOK section (a human registers a training patient, finds it among real
  ones at the narrowest supported window, and tries and fails to link it to a real chart), owed by
  Task 5, the first task that exposes the mark in the window.

## Order and size

Task 0 alone, first, and reported before anything else is touched. Tasks 1 and 2 are one PR: the mark
and the line it draws are not safe apart. Task 3 with them or directly after. Task 4's CLI half and
Task 6 make a second PR, which is the first point at which the cohort can be loaded marked. Task 4's
window half and Task 5 are the third, and need the window's rig.
