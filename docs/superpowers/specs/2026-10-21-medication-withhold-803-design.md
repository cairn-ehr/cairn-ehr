# Medication withhold ("W") — design (#803)

- **Date:** 2026-10-21
- **Issue:** [#803](https://github.com/cairn-ehr/cairn-ehr/issues/803); feeds [#804](https://github.com/cairn-ehr/cairn-ehr/issues/804)
- **Status:** design, from a brainstorm with the maintainer (EM physician) on 2026-10-21
- **Needs:** a new ADR (**ADR-0081**). This is a new clinical state and a new pair of event types, not a UI feature.

## 1. Intent: the paper "W"

On a paper inpatient drug chart, which is both the prescription and the administration record, a drug in
doubt is not deleted. Someone writes **W** (withheld) with a reason, and the drug is not given while the W stands.
The maintainer's examples:

- **The patient's condition makes the drug unsafe for now.** Metformin in acute illness, loop diuretics with
  hyponatraemia, antihypertensives in post-anaesthetic hypotension, insulin with hypoglycaemia on the pre-dose
  check.
- **Doubt about the indication.** On admission, a regular medication looks against the evidence and the
  original prescriber can't be reached. The admitting doctor withholds it *pro tem* until the indication has
  been discussed.

Who writes the W, and when (all three happen):

1. **A doctor, in advance**, into future slots: "W tonight's and tomorrow morning's antihypertensive".
2. **A nurse, at the bedside.** A pre-dose check (BP, glucose) shows it's unsafe, so she writes W in *this*
   slot, does not give the dose, and then tells the prescriber (usually verbally). The prescriber **upholds**
   (often implicitly, by doing nothing, or by extending it) or **rescinds**. The known failure: on a busy ward
   the prescriber isn't told and finds out only by reading the chart.
3. **A doctor, pro tem**: held until reviewed. No end time is known in advance.

**How a W ends on paper.** It is valid for the slots it was written into; afterwards the drug is simply given
again, and the status before the W is restored *without anyone acting*. The default is **resume**. The
pro-tem kind is the exception: it ends with an explicit act after the discussion, either continue or cease.

**The maintainer's sign-off rule:** *"If I sign off a drug chart, I take responsibility for that chart as it is
at the time I sign it off."* A W on the chart at that moment is part of what is signed. Signing it off
upholds it.

## 2. Goals, non-goals, anti-goals

**Goals**
- A clinician (doctor or nurse) can mark a current medication line **withheld** with a **reason** and a **time
  span**: until a stated time, or **until reviewed** (an explicit open end).
- When a stated span ends, the line reads **as it did before the W** (active), with no act by anyone.
- A clinician can **release** a hold early, which serves both the prescriber rescinding a nurse's W and the end
  of a pro-tem hold. Ceasing the drug instead remains the existing cessation.
- The list (CLI and window) shows a withheld line as **"W — withheld until <time | reviewed>: <reason>"** with
  who withheld it. Readers see it without any pop-up, exactly as on paper.
- Sign-off covers withheld lines like any other line. A W written after a sign-off makes that line's sign-off
  stale, so its signer sees an unsigned change.
- The clinical word **"withheld"** means only this hold. Where a clinician-facing text uses "withheld" to mean
  *hidden from view*, it is reworded to **"not shown"** (maintainer's decision; see §6).

**Non-goals (this design)**
- **No administration record (MAR) or dose slots.** Cairn has neither a drug-round grid nor structured
  frequency. A **time span** stands in for "the slots the W was written into" (maintainer: "span of time would
  do").
- **No forced acknowledgement** of a nurse's W by the prescriber. Paper needs none, and a required step would be
  `M > N`.
- **No ward-level "new withholds" list across patients.** It would beat paper's "found out by reading", but it
  needs reader identity and per-reader "seen" state (§5.11, unbuilt). Deferred and filed.
- **No line-level naming of a colleague's concurrent W on re-read** (the #797 rule applied to lines). That
  belongs to #804's before/after list comparison.
- **No role restriction** on who may withhold. The record keeps who did it; restricting it is local policy
  (principle 9, policy-neutral infrastructure; ADR-0021 hard policy is role-gated above the floor).
- Showing a **cessation's** reason (today it is shown nowhere) is a separate gap. Filed, not built here.

**Anti-goals (a failure even if it works)**
- A hold that **stays held by default** after its stated end, the "ACE inhibitor still held at discharge"
  harm. Only an explicit *until reviewed* may outlive time.
- A clinician's W **editing or suppressing** someone else's prescription. A W is the withholder's own
  **additive** act on the thread (ADR-0043: cross-human suppression is refused). The original line stays its
  author's.
- A release that ends a hold its author **never saw** (a W written on another node that reached this node after
  the release was decided).
- Interaction or allergy checks that **ignore** a withheld drug. It is paused, not gone, and usually resumes.
- Any text where "withheld" could mean either *held* or *hidden*.

## 3. Constraints (from the house and the record)

- Append-only, signed, born-sealed bodies (ADR-0052); per-write human authorship (ADR-0053); additive-only
  record format (ADR-0012): **two new event types, no change to existing ones**.
- Sync apply never refuses on a clinical-state veto (a node-local veto would fork the event set). Refusals live
  at the **local** door only; read-time folding must cope with any arrival order.
- Principle 4: "until reviewed" is an **explicit value**, never a missing field standing in for it. A clinician
  who does not know *when* a hold should end says so with "until reviewed"; they are never asked to invent an
  end. (So `from`/`until` are exact instants, unlike the `{value, precision}` start/stop times elsewhere in the
  medication stream: a hold is compared against *now*, and a day-precision "until" would have no defined
  instant of expiry. The ADR records this departure. *Builder-check call.*)
- `t_effective ≤ t_recorded` is enforced at the submit door, so a **future** start (a doctor's advance W for
  tonight) cannot ride on the envelope's `t_effective`. It goes in the payload.

## 4. Decided design

### 4.1 Two new event types

**`clinical.medication-withhold.asserted`** (schema `clinical.medication-withhold/1`), payload:

| field | meaning |
|---|---|
| `medication_id` | the thread withheld (uuid, required) |
| `reason` | non-empty text, required at the floor (unlike cessation's optional reason) |
| `from` | ISO-8601 timestamp with offset, **optional**. Absent means the event's effective time. **May be in the future** (an advance W). |
| `until` | **required**: either an ISO-8601 timestamp with offset, or the literal token `"reviewed"` (held until a release or a cessation). Must be later than `from` when both are times. |

Twin: `Withheld medication from <from> until <until | reviewed> — <reason>`. Like the cessation's twin, it
does **not** repeat the drug name: the name lives on the thread's assertion, and a copy here could disagree
with a later correction or coding of that assertion. *(Builder-check call: follows the cessation precedent,
`crates/cairn-event/src/medication/cessation.rs`.)*

**`clinical.medication-withhold-release.asserted`** (schema `clinical.medication-withhold-release/1`), payload:

| field | meaning |
|---|---|
| `medication_id` | the thread |
| `releases` | non-empty array of the **content addresses of the withhold events this release ends**: the holds the releaser saw on the line. A release never ends a hold it does not name (the anti-goal above; the same "name what was displayed" pattern as ADR-0061's attested search). |
| `reason` | optional text ("discussed with GP — continue", "BP recovered") |

A release ends the named holds **from the release's effective time onward**. A named hold whose span had
already ended is unaffected (a no-op, not an error). A named hold that **has not started yet** (an advance W
whose `from` lies after the release) is **cancelled**: it never comes into force. A prescriber who writes "W from
20:00" and releases it at 14:00 means the dose to be given. *(Builder-check call; an earlier draft said the
opposite, which would have withheld a dose against the prescriber's intent.)*

### 4.2 Read-time status (the fold)

A line's status, evaluated at read time against the node's clock `now`:

1. **Ceased** if a cessation stands (unchanged; cessation is terminal and wins over any hold).
2. Otherwise **Withheld** if any withhold on the thread is *in force at `now`*: `from ≤ now`, `until` is
   `reviewed` or `> now`, and no release naming it has an effective time `≤ now`. (So a release dated before a
   hold's `from` cancels it, per §4.1.)
3. Otherwise **Active**.

Several holds can be in force at once (a nurse's W, then the doctor's longer W). The line shows them all,
newest first, never just one. A hold that has not started yet (a future `from`) is shown as **"W from <time>"**
on an active line, so an advance W is visible before it bites.

Expiry needs **no event**: when a span ends, the line reads active again. That makes the read model
time-dependent. Views compute status against `now()`; nothing stores "withheld" as a fact. Limitation: a node
with a badly wrong clock shows expiry early or late. The window always prints the `until` time, so a human can
judge (principle 4; clock grade, ADR-0027).

### 4.2a Records and lines (reconciled groups)

The list shows **lines**, and a line can be several duplicate records of one drug linked by reconciliation
(ADR-0047; `MedicationGroup.members`). A clinician withholds the **drug on the line**, not a record.
*(Builder-check call, all of this subsection.)*
- A withhold gesture writes **one** withhold event on the line's **representative thread** (the one the line
  already shows as its own, ADR-0047's min-UUID collapse).
- A line is **Withheld** if a hold is in force on **any** member thread, and it shows every such hold.
- The local door's "ceased" refusal tests the **line's** status (db/035's `medication_group_status`), not the
  single record's, so a ceased duplicate member never blocks withholding a current line.
- Release over holds spread across several members writes **one release event per member thread**, each naming
  that thread's holds, in one transaction.
- A **separation** (un-linking duplicates) moves nothing: each hold stays on its own thread, and shows on
  whichever line that thread now belongs to (never erase, always overlay).
- **A withheld line stays on the current list.** Where db/033/db/035 compute a line status that the current
  list filters on (`WHERE st.status = 'active'`), Withheld must be a property *beside* that status, or the
  filter must admit it. A withheld drug must never drop off the current list (anti-goal: a hold is a pause).

### 4.3 Sign-off

- A withheld line is **on the list** and is signed like any other. Signing it off upholds every W in force on
  it (the maintainer's rule).
- Both new event types join the **sign-off commitment** (db/034's three enumerated `IN` lists: the commitment
  function, the readable count and the partial index, which must be **recreated**, not edited, because it is
  `CREATE INDEX IF NOT EXISTS`; plus the Rust mirror `crates/cairn-node/src/medication/attestation.rs`
  `thread_commitment_on`, which must stay identical to the SQL). So a W or a release written after a sign-off makes that line's sign-off stale.
  Existing threads with no withhold events keep their commitment, so no mass staleness.
- A hold **expiring** changes no event, so it does not make a sign-off stale. The signer took responsibility
  for "W until 20:00", which includes what happens at 20:00.
- "Sign off N" counts withheld lines like active ones.

### 4.4 Local-door refusals (never at sync apply)

The local door refuses, with the usual legible refusal:
- a withhold on a **ceased** thread (nothing to hold);
- a withhold with an empty reason, a malformed time, `until` not after `from`, or `until` missing;
- a release with an empty `releases`, or naming an event that is not a withhold **on the same thread**.

At sync apply, all of these are admitted as recorded (the floor's structural checks still run; a payload that
fails them is refused at both doors like any other malformed event). The fold is defined for every arrival
order.

### 4.5 Displays

- **CLI `medication-list`:** `W  <drug> …  withheld until <time | reviewed>: <reason> (<withholder>)`; an
  advance hold as `W from <time>`.
- **Window:** the line carries the same text in its status cell, and keeps its other line actions.
- **New CLI verbs:** `medication-withhold --medication-id --reason --until <ts|reviewed> [--from <ts>]` and
  `medication-release --medication-id --withhold <event-id>…`, printing their event ids (house pattern since
  #826/#828).
- **New window gestures:** on a current line, **Withhold** opens an inline reason field and an "until" choice
  (a time, or "until reviewed"); on a withheld line, **Release** ends the holds the window is showing.

### 4.6 Wording: "withheld" means held, "not shown" means hidden

The clinician-facing uses of "withheld" that mean *hidden* are few (the window's own messages already avoid
the word):
- CLI `crates/cairn-node/src/medication/list_text.rs` (~:182): "…are withheld from sign-off" → "…are left
  out of sign-off".
- CLI patient search `crates/cairn-node/src/patient/search_person.rs` (~:134): "(name withheld)" → "(name not
  shown)".
- Sensitivity report `crates/cairn-node/src/main.rs` (~:4547), `sensitivity/render.rs` (~:384): "nothing is
  withheld" → "nothing is hidden".
Internal identifiers (`withheld_rows`, `WithheldLine`, `chart-withheld`, …) are **not renamed** in this work. A
rename is churn with no reader. A comment at the medication-view `withheld_*` definitions says they mean
*hidden*, not the clinical hold.

## 5. Where it touches the code (inventory for the plan)

From the 2026-10-21 code map. Every place a new medication event type must be registered:

1. `event_type_class` rows; `crates/cairn-node/tests/twin_registry.rs` exhaustive list.
2. `cairn_event_twin_check` rows + floor check(s); `db/tests/034_twin_registry_test.sql` count (24 → 26).
3. `cairn_projection_apply` rows; `db/tests/039_projection_registry_test.sql` (29) and
   `crates/cairn-node/tests/projection_registry.rs` (27). The two files pin different totals (memory note).
4. Projection tables (`medication_withhold`, `medication_withhold_release`) keyed by `content_address`, apply
   functions calling `cairn_guard_medication_patient`; an orphan arm in `cairn_medication_thread_patient`
   (db/031:153) if a withhold can arrive before its assert.
5. db/034 commitment lists (above).
6. db/037 shred: one `DELETE … WHERE content_address = v_ca` arm per new table.
7. db/048 `cairn_event_thread` union arms + content_address indexes;
   `crates/cairn-node/tests/thread_resolution_cost.rs` `MEDICATION_EVENT_TYPES` (6 → 8).
8. `crates/cairn-node/src/safety.rs` test table of real type names.
9. Truncation lists: `tests/common/mod.rs` (~:809), `tests/common/restore_kit.rs` (~:374).
10. Read views: status in `patient_medication*` (db/031) and the group status in db/033/db/035
    (`medication_group_status`) gain the Withheld state. Readers that mean "active" as "not ceased" (db/033,
    db/035, db/042 coding worklist, db/054 person charts) **stay "not ceased"**: a withheld drug is still on the
    list for coding, duplicates and the person's charts.
11. `db/059_medication_withhold.sql` listed in `crates/cairn-node/src/db.rs`; `SCHEMA_GENERATION` 58 → 59.
12. Rust: `cairn-event::medication::{withhold, release}` pure builders + twins; `cairn-node::medication`
    orchestrators through `seal_sign_submit` (mind the cairn-sync `clinical_pull.rs` call-site note if a shared
    signature changes); `MedicationStatus { Active, Withheld(..), Ceased }` in `cairn-medication-view::row`;
    `medication/read.rs` reads holds; `list_text.rs` renders them; the window's `row_view.rs`, `main.js`.
13. Every new DB read wrapped in `LocalDbFault` bumps `db_errors_stay_legible.rs`; every new commit goes through
    `commit_failure` (the #814/#817 commit inventory guard).

**Mixed-version fleet.** A node still on schema generation 58 admits the new events uninterpreted (ADR-0056) and
shows a withheld drug as **active**. Acceptable pre-clinically (no node runs clinical care yet); the ADR names it.

## 6. Slicing

- **Slice A, below the surface (record + CLI):** ADR-0081; the two event types, floor, projections, fold,
  commitment, shred/thread/registry arms, schema generation 59; `MedicationStatus::Withheld`; the CLI verbs and
  `medication-list` rendering; the §4.6 rewording. Paper-parity: the CLI is an operator surface, so the plan
  takes the benchmark section with the gesture's counts as the target for slice B.
- **Slice B, the window:** the W display on the line, the Withhold and Release gestures, the gesture-timing rows
  (db/044's `gesture_kind` CHECK widened), and the RUNBOOK section that owes the human timing.

## 7. Paper-parity benchmark (§1.2), the target for slice B

- **Paper counterpart:** writing W in the administration slot(s) with initials, and the reason in the slot code
  or a short note. Resuming: give the next dose, no extra act.
- **Steps:** paper *N* = 2 (write W + initials; reason). Architecture-forced *M* = 2 (choose Withhold on the line
  and give the reason; "until" defaults to *until reviewed* or a chosen time inside the same gesture; authorship
  comes from possession, ADR-0053). UI bundling target *K* = 2. **Resume:** paper 0, Cairn 0 (expiry needs no
  event). **Rescind:** paper 1 (a note), Cairn 1 (Release).
- **Time + cognitive load:** budget ≤ the paper W, about 10 s from the chart open to recorded. Measurement is
  owed by slice B (RUNBOOK).

## 8. Left to the builder

- Exact SQL shape of the fold (a view with `now()` or a function taking `now` for testability; the latter is
  preferred so tests can pin expiry without sleeping).
- Whether the two projection tables are one table with a kind column.
- Exact CLI flag names and the window's "until" control (a time field plus an "until reviewed" choice is enough).

## 9. To file when the ADR lands

- A cessation's reason is shown nowhere (CLI or window).
- Ward-level list of new withholds across patients (needs §5.11 reader identity).
