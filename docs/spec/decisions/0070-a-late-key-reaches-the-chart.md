# ADR-0070 — A late key reaches the chart

- **Status:** Accepted
- **Date:** 2026-09-15
- **Refines:** [ADR-0057](0057-generic-reprojection-registered-apply-dispatch.md) (where projections are
  dispatched), [ADR-0052](0052-born-sealed-clinical-bodies.md) (the custody plane)

---

## Context

[#584](https://github.com/cairn-ehr/cairn-ehr/issues/584): custody that lands after its event was admitted
never reached the chart.

ADR-0057 decision 1 dispatches every projection from **one** `AFTER INSERT` trigger on `event_log`
(`cairn_projection_dispatch_trg`, `db/005`). Under ADR-0052 decision 5 both write doors write custody — the
wrapped DEK in `event_dek` and the clear shadow in `event_clear` — in their step 9, **before** their
`event_log` INSERT, so that the trigger can read the clear view in the same transaction. Every one of those
inserts is `ON CONFLICT DO NOTHING`, and ADR-0052 decision 4 lets a node without custody admit a sealed row
it cannot read.

Put together, a sealed event first admitted **without** its key, whose key arrives **later**, never reaches
the chart. The second apply writes `event_clear`, its `event_log` INSERT is a no-op, and the trigger does not
fire again. The body opens; the medication list stays empty. Every medication applier returned early on the
first arrival, because `cairn_clear_payload(e)` was NULL, and nothing ever ran it again.

### Four entrances

Three were named in the issue; the fourth was found while designing.

| # | Entrance | Signal before this decision |
|---|---|---|
| 1 | `cairn-sync pull --full` re-offers events a peer first served without custody | `decide_custody`'s operator line named a second step, `cairn_reproject()` |
| 2 | `cairn-sync requeue` lands a retained pen row's key on an already-admitted event | `reproject_owed`, exit 3, **reported by that one run only** |
| 3 | `cairn-node restore` applies the keyless copy of an event before its keyed copy | **none** — exit 0, and a report identical to the harmless order |
| 4 | `submit_event` re-submits the bytes of an event already admitted without custody, with its DEK | **none** — the strict door's step 9 has the identical shape |

**Entrance 3 is why the remedy lives in the doors and not in the callers.** `restore` holds no fact from
which it could tell that a copy it applied earlier was keyless, so it has nothing to act on. A door is the
only place that **knows**, at the moment it happens, that the custody it just wrote belongs to an event
already in the log.

### What the audit established

Two read-only audits walked the live definition (the highest-numbered `db/NNN` that defines it) of every
applier registered in `cairn_projection_apply`, and the load-bearing claims were re-read by hand
(2026-09-15).

1. **Every medication applier is custody-gated and writes nothing without custody.**
   `medication_statement_apply` and `medication_cessation_apply` (`db/031`), `medication_dose_seed_initial`
   and `medication_dose_change_apply` (`db/032`), `medication_dose_correction_apply` (live in `db/035`),
   `medication_reconciliation_apply` (`db/033`), `medication_attestation_apply` (`db/034`), and
   `medication_coding_apply` and `medication_coding_correction_apply` (`db/042`) all open with
   `p jsonb := cairn_clear_payload(e); … IF p IS NULL THEN RETURN;` — no row, no flag, no placeholder. All
   are `heal_safe = TRUE`. **Their cross-event lookups go through projection tables only**
   (`cairn_medication_thread_patient`, `medication_reconciliation`, the read-time views); none keys on the
   existence of an `event_log` or `event_clear` row. So *"the target has no custody here"* and *"the target
   is absent"* were already handled identically — the out-of-order case set-union sync had to solve anyway —
   and re-running only the late event's own heal-safe appliers is enough for every projection.
2. **Every non-medication applier ignores custody by design.** ADR-0052 decision 2 lawfully seals only
   `clinical.*` bodies (the seal-robustness comment in `db/005`'s `submit_event`; `db/002`'s
   `patient_chart_apply`), so every demographic, identity, patient and sensitivity applier reads `e.body`
   behind `IF e.sealed THEN RETURN` — or, for `sensitivity_assertion_apply` (`db/048`), projects a
   deliberately `'unreadable'` MAX-ranked row. None reads `event_clear`, and `event_log.sealed` never
   changes, so **a late key cannot change anything these appliers read** and re-running them is an
   idempotent no-op. That includes **the only `heal_safe = false` registration in the tree**,
   `note.added → patient_chart_apply` (the `note_count` counter): it sits behind the sealed guard, so a late
   key owes it nothing. The residual #584 asked to keep reporting — a custody-dependent projection that a
   heal could not re-run — **is empty by construction**, which is what licenses decision 3.

### Rejected alternatives

- **An `AFTER INSERT` trigger on `event_clear`.** It would cover every `event_clear` insert automatically,
  but it fires inside step 9, before the substitution guard, so both doors would first have to move their
  guards ahead of their custody writes.
- **A call in `apply_remote_event` (`db/020`) only.** It leaves entrance 4 open.
- **A durable ledger of owed projections.** Machinery for a case the audit's second finding shows cannot
  occur.
- **A narrow owner-granted heal door that callers invoke.** Entrance 3 has no caller-visible signal to
  invoke it on.

---

## Decision

### 1. Custody arriving is a projection-relevant event

When a door newly writes `event_clear` for an event whose `event_log` row already exists, it runs that
event's **heal-safe** registered appliers once, over the stored row — after its substitution guard, in the
door's own posture, and only when the row is replay-eligible.

A door detects the case from its own statement counts: step 9's `event_clear` insert wrote a row, and the
`event_log` insert did not. The stored row is the row as **first admitted**, carrying the attestation
columns that admission stored — never a row rebuilt from this call's arguments. Each qualifier is
load-bearing:

- **After the substitution guard.** Step 9 writes `event_clear` before the guard compares content
  addresses. Dispatching earlier would run appliers over a **rival** body filed under an existing
  `event_id`. The guard's later RAISE rolls that back, but the refusal a caller reads could become whatever
  an applier raised first instead of `substitution refused` — the reason `restore` pens and tests assert.
- **In the door's own posture.** At `apply_remote_event` the call runs while `cairn.remote_apply` is still
  `on`; the substitution guard moved above the marker clear to make room for it. Three projection checks
  read that marker, and each admits a first arrival on the remote path: `cairn_guard_medication_patient`
  (`db/031`) writes a `medication_patient_conflict_flag`, the oversize-group check (`db/033`) writes a
  `medication_projection_flag`, and the cross-patient reconciliation refusal (`db/033`) is skipped, its
  contradiction surfaced at read time by the `medication_group_cross_patient` view. After the clear, all
  three RAISE, so a late key dispatched there would be refused and could never land. At `submit_event` the
  marker is off and stays off: a late landing there is judged in the strict posture, exactly as a first
  arrival there would be.
- **Only when replay-eligible.** A row carrying an `event_deferred` marker — admitted uninterpreted, or
  failed re-adjudication, whose marker stays until a later pass promotes the event — must never project
  ([ADR-0056](0056-unknown-event-types-admitted-uninterpreted.md) decision 4, through ADR-0057 decision 3's
  `cairn_replay_eligible` seam).

By position the call also comes after step 8's clear-view floor: the custody-less first admission skipped
the per-type floor, and the keyed re-apply runs it on the clear view before anything is written.

### 2. One expression, called from both doors

The *"run this row's heal-safe appliers"* loop exists **once** in SQL, beside the dispatcher in `db/005`:
`cairn_projection_dispatch_heal_safe(event_log)`. Re-adjudication's gate 4 (`db/043`), which carried its own
copy, now calls it too. It holds **no** eligibility filter, deliberately: gate 4 runs appliers over a row
whose marker is still present, as its proof that promotion is safe. The filter lives in
`cairn_project_late_custody(uuid)`, which loads the stored row, returns unless the row exists and is
replay-eligible, and dispatches. Both doors call that one function. Both functions take the dispatcher's
shape — non-definer PL/pgSQL with `search_path` pinned, since every caller already runs as the owner — and
the appliers' grant posture, `EXECUTE` revoked from `PUBLIC`, because they write projections.

### 3. A projection that reads custody must be heal-safe

Enforced by a catalog guard, `crates/cairn-node/tests/late_custody_guards.rs`: every registered applier
whose body mentions `cairn_clear_payload` or `event_clear` is registered `heal_safe = TRUE`. With that
invariant a late key can never leave a debt a door did not pay, so `requeue`'s `reproject_owed` — its
field, its message and its exit-3 arm — is **retired, not narrowed**. A signal that is zero by construction
reads as a measurement; deleting it is the honest form.

### 4. The healed state is arrival at custody time

After a late landing the chart equals *"the event arrived at the moment its key landed"*, not *"at its first
admission"*. That is what set-union already guarantees for these projections; nothing stronger is promised.
The residue this leaves is named under Consequences.

---

## Paper-parity benchmark (§1.2)

**Paper counterpart:** a page that reached the ward before the chart it belongs in — held at the nurses'
station, then filed once the chart turns up. Recovery is **N = 1** act: file the page.

Counted on the recovery path once the underlying fault is repaired — an unregistered key, an unadmitted
peer. That repair is a provisioning act with no paper counterpart, owned where the fault is:
[ADR-0066](0066-identity-dies-with-the-disk-custody-must-not.md) and
[#512](https://github.com/cairn-ehr/cairn-ehr/issues/512) for restore's keys, pairing for a peer.

| Entrance | Architecture-forced *M*, before | After |
|---|---|---|
| `requeue` | 2 — requeue, then an owner-privileged `cairn-node reproject` | **1** |
| `pull --full` | 2 — `pull --full`, then `cairn_reproject()` as the database owner | **1** |
| `restore`, keyless copy first | the record silently missing — worse than any *M* | **0 extra** |
| `submit_event` re-submit | the record silently missing | **0 extra** |

**UI bundling target K = 1.** `M = N` on every entrance: the decision removes an act rather than adding one.
**Time budget:** a late landing costs what the same event's first arrival costs — the identical registered
appliers, run once — plus one `GET DIAGNOSTICS` on every sealed write. No new runnable surface is exposed,
so no measurement is owed; the ordinary sealed-write cost is already measured (median 222 ms node-tier,
Slice 61) and gains no query.

---

## Consequences

- **ADR-0057's single `AFTER INSERT` dispatcher gains a second, decided dispatch site.** It runs registered
  appliers only, so *"a projection lives only in its registered apply function"* still holds: the
  late-custody path adds a moment at which appliers run, never a place where projection logic lives.
  [§9.4](../language-substrate.md#94-merge-projection-boundary-fat-postgres-thin-rust-daemon)'s
  projection bullet, which said *"`AFTER INSERT` only"*, now names this path.
- **ADR-0052's custody plane gains its missing half.** A node without custody still admits a sealed row it
  cannot read; when the key arrives, the row now reaches the chart with no operator act.
- **The operator signals that described the debt are gone.** `requeue`'s `reproject_owed`, its message and
  its exit-3 cause are retired; exit 3 now means only that rows were retained in the pen or are still
  refused by the door. **`pull --full` is one step:** `decide_custody`'s operator line names only it, and no
  longer names `cairn_reproject()`. `restore`'s `CustodyDidNotLand` remedy ends at `cairn-sync requeue`.
- **Three tests that pinned the defect now pin the heal:** a keyless-first `restore` reaches the chart
  (`restore_one_event_id_one_body.rs`); `pull --full` alone brings the chart back (`clinical_pull.rs`);
  `requeue` arm 1 exits 0 with the chart populated (`requeue_retains_unlanded_custody.rs`). The doors
  themselves are pinned by `late_custody_reaches_the_chart.rs`, the two functions by
  `heal_safe_dispatch.rs`.
- **The healed state is arrival at custody time, and three projections carry arrival-order residue:** which
  event's `content_address` a `medication_patient_conflict_flag` names; the `patient_id` snapshot on a
  `medication_coding` row written before its statement was readable (`coalesce(thread patient,
  e.patient_id)`, `db/042` — it differs from the statement's patient only when the coding event's envelope
  names a different patient, which is [#192](https://github.com/cairn-ehr/cairn-ehr/issues/192)'s
  cross-patient contradiction and is flagged in either order); and the reconciliation oversize clamp,
  measured at apply time. **Each is the same residue the ordinary out-of-order case already has**, none is
  widened here, and none is filed. The heal is not time travel.
- **A second invariant is pinned beside decision 3:** the PL/pgSQL and SQL functions that
  `INSERT INTO event_clear` are exactly the two doors, and each calls `cairn_project_late_custody`. A third
  such function is a decision, not a drift — without the call it would reopen #584 through its own entrance.
  The shred's `DELETE FROM event_clear` (`cairn_execute_shred`, `db/037`) is outside the rule: it removes
  custody, so it owes no projection.
- **The guard has a residual.** Both of its rules read a function's own body (`pg_proc.prosrc`), so a
  custody read reached only through a helper the applier calls is invisible. Every custody reader today
  calls `cairn_clear_payload` directly, and a positive control asserts the guard sees them, so it cannot
  pass vacuously. Its comment stripper is per-line and literal-blind: a call written after a `--` inside a
  string literal on the same line would be missed.
- **Charts already missing a record on an existing database are not healed by upgrading.** There is no
  migration and no `SCHEMA_GENERATION` bump, so the loader's generation-change heal does not run;
  `cairn-node reproject` still heals such a chart. Under the pre-clinical posture no deployment holds one.
- **No wire-format change.** The event core is untouched.
- **A neighbouring weakness in the same marker is tracked separately:**
  [#602](https://github.com/cairn-ehr/cairn-ehr/issues/602), any client can set `cairn.remote_apply` before
  calling `submit_event`. It predates this decision, affects a first arrival at that door exactly as it
  affects a late landing, and is neither widened nor narrowed here.
- **How we would know the bet failed:** `late_custody_guards.rs` fails — an applier reads custody without
  being heal-safe, or a function that inserts into `event_clear` does not call
  `cairn_project_late_custody` — or a chart is found empty after `requeue` exits 0.

The design and the implementation plan are
`docs/superpowers/specs/2026-09-15-late-custody-reaches-the-chart-584-design.md` and
`docs/superpowers/plans/2026-09-15-late-custody-reaches-the-chart-584.md` (working scaffolding, excluded
from the published site).
