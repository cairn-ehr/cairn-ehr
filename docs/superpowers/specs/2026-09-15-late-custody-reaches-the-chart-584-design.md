# Design — a late key reaches the chart (#584)

- **Issue:** [#584](https://github.com/cairn-ehr/cairn-ehr/issues/584) — custody that lands after its
  event was admitted never reaches the chart.
- **Date:** 2026-09-15. **Branch:** `feat/584-late-custody-reaches-the-chart`.
- **Maintainer decisions taken in the brainstorm (2026-09-15):** option (a) narrowed — the door
  re-dispatches heal-safe projections; ONE helper called by BOTH doors; the `heal_safe = false`
  residual is made **unrepresentable**, not reported.
- **Records:** a new **ADR-0070** (spec v0.71 → v0.72), refining ADR-0057 (where projections are
  dispatched) and ADR-0052 (the custody plane). No new migration file, no `SCHEMA_GENERATION` bump.

## 1. Why this piece exists

Projections are dispatched by ONE `AFTER INSERT` trigger on `event_log`
(`cairn_projection_dispatch_trg`, db/005). Both write doors write custody — `event_dek` and the clear
shadow `event_clear` — in their step 9, **before** their `event_log` INSERT, so the trigger can read
the clear view in the same transaction. Every one of those inserts is `ON CONFLICT DO NOTHING`.

So when a sealed event is first admitted **without** custody and its key arrives **later**, the second
apply writes `event_clear`, its `event_log` INSERT is a no-op, and the trigger never fires again. The
body opens; the medication list stays empty. Every medication apply fn returned early the first time
(`cairn_clear_payload(e)` was NULL) and nothing ever runs it again.

Four entrances reach that state — three named in the issue, one found while designing:

| # | Entrance | Signal today |
|---|---|---|
| 1 | `cairn-sync pull --full` re-offers events a peer first served without custody | `decide_custody`'s operator line names a second step, `cairn_reproject()` |
| 2 | `cairn-sync requeue` lands a retained pen row's key on an already-admitted event | `reproject_owed`, exit 3, **reported by that one run only** |
| 3 | `cairn-node restore`, keyless copy of an event before its keyed copy (trap 9's second entrance) | **none** — exit 0, report identical to the harmless order |
| 4 | `submit_event` re-submitting the bytes of an event already admitted without custody, with its DEK | none — the strict door's step 9 has the identical shape (db/005) |

Entrance 3 is why the fix cannot live in the callers: `restore` has no fact from which to detect it.
The door is the only place that **knows** custody landed late.

## 2. What the audit established (every registered apply fn, 2026-09-15)

Two read-only audits walked the LIVE definition of every row in `cairn_projection_apply` (the highest
-numbered `db/NNN` that defines it), and the load-bearing claims were re-read by hand.

### 2.1 Every medication fn is custody-gated and writes nothing without custody

`medication_statement_apply` (db/031), `medication_cessation_apply` (db/031), `medication_dose_seed_initial`
(db/032), `medication_dose_change_apply` (db/032), `medication_dose_correction_apply` (live db/035),
`medication_reconciliation_apply` (db/033), `medication_attestation_apply` (db/034),
`medication_coding_apply` and `medication_coding_correction_apply` (db/042) all open with
`p jsonb := cairn_clear_payload(e); … IF p IS NULL THEN RETURN;` — no row, no flag, no placeholder.
All are `heal_safe = TRUE`.

**Cross-event lookups go through projection tables only** (`cairn_medication_thread_patient`,
`medication_reconciliation`, the read-time views). None keys on `event_log` or `event_clear`
existence, so *"the target has no custody here"* and *"the target is absent"* are ALREADY handled
identically — the out-of-order case the set-union design had to solve anyway. Re-running only the
late event's own heal-safe fns is therefore enough for every projection.

### 2.2 Every non-medication fn ignores custody by design

ADR-0052 §2's seal-robustness rule (db/005 `submit_event` step 7 comment; db/002 `patient_chart_apply`
line 94): only `clinical.*` bodies are lawfully sealed, and every demographic/identity/patient/sensitivity
apply fn reads `e.body` directly behind `IF e.sealed THEN RETURN` — or, for
`sensitivity_assertion_apply` (db/048), projects a deliberately `'unreadable'` MAX-ranked row. None
reads `event_clear`. `event_log.sealed` never changes, so **a late key cannot change anything these fns
read**, and re-running them is an idempotent no-op.

This includes the ONLY `heal_safe = false` registration in the tree, `note.added → patient_chart_apply`
(the `note_count` counter): it sits behind the sealed guard, so a late key owes it nothing. **The
residual #584 asked to keep reporting is empty today by construction** — which is what licenses
decision 3 below.

### 2.3 Four placement hazards the helper call must respect

1. **After the substitution guard.** Step 9 writes `event_clear` before the guard compares content
   addresses. Dispatching earlier would run apply fns over a RIVAL body filed under an existing
   `event_id`; the later RAISE rolls it back, but the refusal a caller reads could become whatever an
   apply fn raised first instead of `substitution refused` — the reason `restore` pens and tests assert.
2. **While `cairn.remote_apply` is still `on` (lenient door).** db/020 clears the marker right after
   its INSERT. Dispatching after that line reaches three RAISE arms — `cairn_guard_medication_patient`
   (db/031), the local reconciliation refusal and the oversize-group check (db/033) — so a late key
   would be REFUSED and could never land. At the strict door the marker is off, and stays off: a late
   landing there is judged in the strict posture, exactly as a first arrival there would be.
3. **Only when `cairn_replay_eligible`.** A row carrying an `event_deferred` marker (admitted
   uninterpreted, or failed re-adjudication — whose marker is permanent) must never project;
   `cairn_reproject` already filters on this seam. (db/043's gate 4 deliberately runs apply fns on a
   STILL-marked row as its promotion proof — so the eligibility filter belongs at the door call, never
   inside the shared dispatch.)
4. **After step 8's clear-view floor.** The custody-less first admission skipped the per-type floor;
   the keyed re-apply runs it on the clear view (db/020 line ~395) before anything is written. The call
   sits after the INSERT, so this holds by position.

### 2.4 What the healed state is — and is not

After the fix the chart equals **"the event arrived at the moment its key landed"**, not *"at its
first admission"*. Three projections carry arrival-order residue — which event's `content_address` a
`medication_patient_conflict_flag` names, the `patient_id` snapshot on a `medication_coding` row
written before its statement was readable (`coalesce(thread patient, e.patient_id)`, db/042 — it can
differ from the statement's patient only when the coding event's envelope names a DIFFERENT patient,
which is #192's cross-patient contradiction and is flagged in either order), and the reconciliation
oversize clamp measured at apply time. **Every one is the SAME residue the ordinary out-of-order case
already has**, and none is widened here, so no issue is filed for them. Out of scope; stated in the
ADR so nobody reads the heal as time travel.

## 3. The decisions (ADR-0070)

1. **Custody arriving is a projection-relevant event.** When a door newly writes `event_clear` for an
   event whose `event_log` row already exists, it runs that event's **heal-safe** registered apply fns
   once, over the stored row — after its substitution guard, in the door's own posture, and only when
   the row is replay-eligible.
2. **One expression, called from both doors.** The "run this row's heal-safe fns" loop exists once in
   SQL and is shared with db/043's gate 4, which today carries its own copy.
3. **A projection that reads custody must be heal-safe** — enforced by a catalog guard. With that
   invariant, a late key can never leave a debt a door did not pay, so `requeue`'s `reproject_owed`
   (its field, its message, its exit-3 arm) is **retired**, not narrowed. A signal that is zero by
   construction reads as a measurement; deleting it is the honest form.
4. **The healed state is arrival at custody time** (§2.4), which is what set-union already guarantees
   for these projections; nothing stronger is promised.

**Rejected:** an `AFTER INSERT` trigger on `event_clear` (automatic for every writer, but it fires
inside step 9, before the substitution guard, so both doors' guards would have to move ahead of their
custody writes first); db/020 inline only (leaves entrance 4 open); a durable debt ledger (machinery
for a case §2.2 shows cannot occur); a narrow owner-granted heal door called by callers (option (b) —
entrance 3 has no caller-visible signal to call it on).

## 4. The shape

### 4.1 db/005 — two small functions beside the dispatcher

- **`cairn_projection_dispatch_heal_safe(e event_log) RETURNS void`** — the loop db/043 gate 4 holds
  today, verbatim: `FOR apply_fn IN SELECT … WHERE event_type = e.event_type AND heal_safe ORDER BY
  run_order, apply_fn LOOP EXECUTE format('SELECT %I($1)', …) USING e`. No eligibility filter (§2.3.3).
  `SET search_path = public, pg_temp` (dynamic `%I` EXECUTE, same argument as the dispatcher; #426).
  `REVOKE EXECUTE … FROM PUBLIC` — it writes projections, the applier posture (#382).
- **`cairn_project_late_custody(p_event_id uuid) RETURNS void`** — loads the stored `event_log` row
  (the row as FIRST admitted, carrying the attestation columns that admission stored — never a row
  re-synthesised from this call's arguments); returns if there is none (defensive: both callers reach
  it only after their INSERT) or if `NOT cairn_replay_eligible(row)`; otherwise calls the dispatch
  above. Same `search_path` pin, same REVOKE. Tests observe its effect on the projections, not a return
  value.

Both are plain PL/pgSQL, not definers: they are called from inside the SECURITY DEFINER doors (and the
owner-only db/043), so they already run with the owner's rights — the dispatcher's posture.

### 4.2 The two doors

Each door gains ONE local, `v_clear_written boolean`, set by `GET DIAGNOSTICS … ROW_COUNT` straight
after its `INSERT INTO event_clear` (false on the arms that write no custody). The call:

```sql
IF v_rows = 0 AND v_clear_written THEN
    PERFORM cairn_project_late_custody(v_event_id);
END IF;
```

- **db/020 `apply_remote_event`:** the substitution guard moves UP to sit between `GET DIAGNOSTICS
  v_rows` and the `set_config('cairn.remote_apply', '', true)` clear, and the call follows the guard —
  so the marker is still `on` (§2.3.2). Moving the guard across the `set_config` changes nothing it
  checks: the marker is transaction-local and a RAISE aborts the transaction either way. The comment
  "Capture the insert outcome BEFORE the set_config below" stays true.
- **db/005 `submit_event`:** the call goes straight after its existing post-INSERT substitution guard.
  No marker (strict posture, §2.3.2).
- **db/043 gate 4:** its loop becomes `PERFORM cairn_projection_dispatch_heal_safe(r.el_row);` — the
  surrounding subtransaction and its comment are unchanged.

Cost on the ordinary path: one `GET DIAGNOSTICS` per sealed write. The dispatch runs only on a late
landing, and then runs exactly the fns a first arrival would have.

### 4.3 Guards (Rust, `crates/cairn-node/tests/`)

- **`late_custody_reads_are_heal_safe.rs`** (catalog, DB-gated): every `apply_fn` in
  `cairn_projection_apply` whose `pg_proc.prosrc` mentions `cairn_clear_payload` or `event_clear` has
  `heal_safe = TRUE` on every row naming it. **Positive control:** the set of custody-reading appliers
  is non-empty and contains `medication_statement_apply` (a guard that sees nothing passes vacuously —
  the #586 lesson). **Documented residual:** a custody read hidden inside a helper the applier calls is
  not seen; the header says so.
- **`event_clear_writers_project_late_custody.rs`** (source, no DB): every `INSERT INTO event_clear` in
  the shipping `db/*.sql` sits in a function body that also calls `cairn_project_late_custody`, and the
  count of writers is pinned (2) so a third is a decision, not a drift. Scans whole files — no stopping
  at a first test module (#586).
- Existing guards the new functions must satisfy (not new work, but checked): `search_path_pg_temp.rs`
  (its pinned floors may need +2), `floor_execute_grants.rs`, `db_gate_actually_ran.rs`.

### 4.4 cairn-sync `requeue` — retire `reproject_owed`

Delete `chart_rebuild_owed`, `chart_rebuild_message`, the `reproject_owed` field of `RequeueCounts`
(and its JSON key, its summary clause, its `incomplete()` arm, its interrupted-message clause), and
`EXIT_INCOMPLETE`'s *"a chart owed a heal is reported by ONE run only … is #584"* paragraph. Exit 3
keeps its two remaining causes (rows retained, rows still refused). If the pre-apply custody read
exists only to feed `chart_rebuild_owed`, it goes too; if `keyed_row_verdict` uses it, it stays.

### 4.5 Operator text that names the retired step

- `decide_custody`'s recovery clause (cairn-sync `main.rs`): **one** step, `pull --full`; the unit test
  that asserts the line contains `cairn_reproject` inverts to assert it does NOT, keeping `pull --full`.
  Its doc comment's *"Why the recovery clause names TWO steps"* is rewritten to say why it names one,
  and where the second step went (this ADR).
- `restore/clinical.rs` `CustodyDidNotLand`: drop *"its projection ran without the key … names the
  `cairn-node reproject` heal"*; the remedy is `cairn-sync requeue`, full stop.
- `restore_kit.rs` doc (line ~229) and `restore_cli_surface.rs` comment (line ~140) stop citing #584 as
  open.
- db/005's seal-robustness comment lists `db/045` and `db/048` as seal-robust too (audit nit).

## 5. Testing (TDD — each written failing first)

### 5.1 The door, DB-gated (`crates/cairn-node/tests/late_custody_reaches_the_chart.rs`)

Fixtures come from the existing production orchestrators (a born-sealed, human-authored
`clinical.medication.asserted`), never hand-built bytes.

1. **Headline.** Admit through `apply_remote_event` WITHOUT the DEK → `medication_statement` 0 and the
   dose seed 0; re-apply WITH the DEK → body opens, statement 1, dose seed 1. *Both* fns of the type.
2. **Idempotent third apply.** A further keyed re-apply writes no second custody row, dispatches nothing
   (`v_clear_written` false), and leaves every medication table and `medication_patient_conflict_flag`
   count unchanged.
3. **Lenient posture held.** A late landing whose statement contradicts the thread's standing patient
   FLAGS (a `medication_patient_conflict_flag` row) rather than raising — pins §2.3.2.
4. **Deferred rows never project.** An event with an `event_deferred` marker whose key lands late:
   custody lands, the chart stays empty, and the marker survives. Then re-adjudication (gate 4, through
   the shared dispatch) projects it.
5. **A rival with a DEK.** A different body under an existing custody-less `event_id`, carrying a DEK →
   the door refuses with `substitution refused`, no `event_clear` row survives, and the chart is empty.
6. **A shredded target.** Key re-offered after a shred → no custody, no dispatch, chart empty.
7. **heal_safe = false is not re-run.** A test-scoped counting applier (`cairn_test_*`, fault-injection
   without residue: registered and dropped at test start and before asserting; `pg_proc` checked clean)
   registered `heal_safe = false` alongside one registered `heal_safe = true` → after a late landing the
   safe one has run once more and the unsafe one has not.
8. **The strict door's entrance.** Admit custody-less through `apply_remote_event`, then `submit_event`
   the same bytes with the DEK and its attestation → the chart is 1.

### 5.2 Pins that invert (each already names its inversion)

- `restore_one_event_id_one_body.rs::a_keyless_copy_first_leaves_the_chart_unprojected_until_584` →
  renamed `a_keyless_copy_first_still_reaches_the_chart`, asserts `medication_rows == 1`, pin prose
  retired.
- `requeue_retains_unlanded_custody.rs` arm 1, phase two: chart **1** straight after the release, **no**
  `reproject_owed` key, exit **0**; phase three (the owner-run heal) deleted — the chart assertion moves
  up and remains "THE ASSERTION THAT MATTERS".
- `clinical_pull.rs::an_admitted_peer_recovers_the_bodies_it_pulled_without_custody`: statement **1**
  after `pull --full` alone; step 3 deleted; doc rewritten (one step).
- `requeue.rs` / `main.rs` unit tests that construct or print `reproject_owed` are rewritten against the
  smaller struct.

### 5.3 Mutations to run before calling the tests meaningful

Each must turn a named test red **at the assertion that names its claim** (the #593 lesson):

| Mutation | Must fail |
|---|---|
| delete the helper call in db/020 | 5.1.1, the three inverted pins |
| delete the helper call in db/005 | 5.1.8 |
| drop the `cairn_replay_eligible` check | 5.1.4 |
| drop `AND heal_safe` from the shared dispatch | 5.1.7 |
| place the db/020 call AFTER the marker clear | 5.1.3 |
| place the db/020 call BEFORE the substitution guard | 5.1.5 (reason no longer `substitution refused`, or a projection survives) — **if it does not fail, say so and record why position is still required** |
| call on `v_rows = 0` without `v_clear_written` | 5.1.2 |
| set a medication applier `heal_safe = false` | `late_custody_reads_are_heal_safe.rs` |
| add a third `INSERT INTO event_clear` without the call | `event_clear_writers_project_late_custody.rs` |

## 6. What is still broken when this merges

- **Charts already missing a late-custody record on an existing database are not healed by upgrading**
  (no generation bump). `cairn-node reproject` still heals them. Pre-clinical posture: no deployment
  holds such a chart; stated in the ADR's consequences.
- **#594** (restore's exit code for records past a chain break / in an unknown plane / a torn tail —
  decided 2026-09-15, not built here) · **#596** · **#597** (its wording is still false; it no longer
  hides an empty chart) · **#598** · **#599** · **#585** (nothing reads the doors' WARNINGs) · the
  arrival-order residue of §2.4 (pre-existing and flagged where it matters; deliberately not filed).
- **The custody-read guard's residual:** an applier that reads custody through a helper is invisible
  to a `prosrc` scan.

## 7. Documentation owed by the branch

ADR-0070 + `decisions/README.md` row · `docs/spec/index.md` → **0.72** · `language-substrate.md`'s
*"`AFTER INSERT` only"* bullet (custody arrival is now a second, decided maintenance path) · HANDOVER
(trap 9 retires into history; a new trap: *every `event_clear` writer calls the helper, and a
custody-reading applier is heal-safe*) · ROADMAP (#584 closed; the Slice 66 *"repair is TWO steps"*
line) · the implementation plan with its review ledger.

## Paper-parity benchmark (§1.2)

**Paper counterpart:** a page that reached the ward before the chart it belongs in — held at the
nurses' station, then filed once the chart turns up. Recovery is **N = 1** act (file the page).

**Architecture-forced, before → after this slice** (counting the acts on the recovery path once the
underlying fault — an unregistered key, an unadmitted peer — is fixed; that fault repair is a
provisioning act with no paper counterpart, already owned where the fault is: ADR-0066 and #512 for
restore's keys, pairing for a peer):

| Entrance | Before | After |
|---|---|---|
| `requeue` | M = 2 (requeue, then an owner-privileged `cairn-node reproject`) | **M = 1** (requeue) |
| `pull --full` | M = 2 (pull --full, then `cairn_reproject()` as DB owner) | **M = 1** |
| `restore`, keyless copy first | the record is silently missing — worse than any M | **M = 0** extra |
| `submit_event` re-submit | silently missing | **M = 0** extra |

**UI bundling target K = 1.** `M = N` on every entrance; the slice removes an act rather than adding
one. **Time budget:** a late landing costs no more than the same event's first-arrival projection — it
runs the identical registered fns once — plus one `GET DIAGNOSTICS` on every sealed write. No new
runnable surface is exposed, so no measurement is owed by this slice; the ordinary sealed-write cost is
already measured (median 222 ms node-tier, Slice 61) and gains no query.
