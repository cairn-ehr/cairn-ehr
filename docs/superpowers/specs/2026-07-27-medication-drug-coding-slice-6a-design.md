# Design — `clinical.medication` slice 6a: the inline `substance.coding` shape

- **Date:** 2026-07-27
- **Implements:** [ADR-0059](../../spec/decisions/0059-medication-drug-coding-drugref-moiety-anchor.md)
  (decisions 1, 2, 4-partial, 5, 7) — the first half of the code slice that ADR unblocks.
- **Status:** design agreed; implementation plan to follow.
- **Scope:** working scaffolding, not canonical. The spec (`docs/spec/`) and the ADR log win on any
  disagreement.

## Why this slice exists

ADR-0059 fixed the **wire-content shape** of a medication's drug coding before any code carried it,
because that shape is expensive to retrofit onto an append-only clinical stream. This slice writes the
first half of that shape into product code: the **inline** coding on `clinical.medication.asserted`, the
retirement of the reserved `substance.inn_code` slot, and the two concrete reconciliation gaps the ADR
cites (the dup-key blind spot and the arbitrary reconciled-group display).

It deliberately stops short of the coding **overlay** event types. Those are slice 6b.

## Scope

**In:**

1. `substance.coding {system, code, display}` on the medication assertion — builder, payload, legibility
   twin (`cairn-event`).
2. `substance.inn_code` retired: removed from the builder, refused at the strict door, ignored at the
   apply door; the `medication_statement.inn_code` **column stays, deprecated in place** (ADR-0059
   decision 2 — a DROP is the non-additive move principle 11 forbids).
3. A coding-system **vocabulary registry** + the in-DB floor check for the coding object
   (new `db/041_medication_coding.sql`), strict-submit / lenient-apply.
4. A `medication_coding` **projection table** fed by the assertion's inline coding, and the read views
   widened to expose the coding.
5. The E1 dup-key widened to the `(system, code)` **pair**, and `medication_group_display` reworked to
   prefer a coded member; a new advisory `medication_group_coding_conflict` view.
6. The CLI surface (`--coding-system/--coding-code/--coding-display`) and every `inn_code` call site.

**Out (with the reason each is out):**

| Deferred | Why |
|---|---|
| `clinical.medication-coding.asserted` + `-correction.asserted` (ADR-0059 decision 3) | Slice 6b. Purely additive on top of this slice's projection table — no rework of anything signed. |
| Any drugref code (term→moiety lookup, type-ahead, DDI) | §9 advisory tier, and the cross-service connection model is a design decision in its own right. Its absence is this slice's honest-degradation proof (see below). |
| The §5.9 safety-class capture (ADR-0059 decision 4) | The safety projection does not exist yet ([#232](https://github.com/cairn-ehr/cairn-ehr/issues/232)). See "An ADR tension worth recording". |
| Fuzzy / term→anchor matching, coded↔uncoded dup detection | The drug-matcher slice. ADR-0059 decision 5 is explicit that the key alone does not close this. |

## An ADR tension worth recording

ADR-0059's follow-on section lists as a **first-class test obligation** for the code slice that *"the §5.9
safety projection must fire on that node from the captured class."* Decision 4 of the same ADR says the
class field *"belongs to the safety-projection shape … and is owed by that slice."* Both cannot be true of
this slice: there is no safety projection to fire. The buildable half of the obligation **is** met here —
a coded medication reads, lists and reconciles with no drugref anywhere in the code path — and the
unbuildable half becomes a filed issue against #232 carrying the ADR-0059 decision-4 constraint verbatim,
so the safety-projection slice inherits it rather than rediscovering it.

## 1. Wire shape (`cairn-event`)

```rust
/// A drug-identity coding claim captured at coding time (ADR-0059).
/// All three fields travel: `display` is the honest-degradation label a node
/// without drugref still shows, so it is never optional within the object.
pub struct SubstanceCoding<'a> {
    pub system: &'a str,   // "drugref-moiety" today
    pub code: &'a str,     // the immortal moiety_uuid
    pub display: &'a str,  // INN-preferred label captured at coding time
}
```

`MedicationAssertion.inn_code: Option<&str>` is **replaced** by `coding: Option<SubstanceCoding<'a>>`.
Present ⇒ `substance.coding = {system, code, display}`; absent ⇒ the key is omitted entirely, never
serialized as null, so an uncoded assertion's content address is byte-identical to today's.

**Twin rule.** `render_medication_twin` appends ` [display]` **only when `display` differs from `term`**
under a case-folded compare:

```
Lipitor 40 mg tablet — one BD (patient-reported), started 2024 [atorvastatin]
atorvastatin 40 mg tablet — one BD (patient-reported), started 2024
little white pill (patient-reported)
```

Pure and deterministic — the same event renders the same twin on every node, which is what makes the twin
a legibility guarantee rather than a local convenience.

**Retired-slot discipline.** The builder can no longer emit `substance.inn_code`. The **strict door
refuses** a payload that still carries it, with a message naming `substance.coding`; the **apply door
ignores** it (same door detection as §2's registry tier). Fail loud at the source, never refuse a
verifiable peer event — the ADR-0051 posture, and the ADR-0056 reason (a refusal on the remote door
freezes a watermark).

## 2. Floor and vocabulary registry — new `db/041_medication_coding.sql`

```sql
CREATE TABLE IF NOT EXISTS medication_coding_system (
    system      TEXT PRIMARY KEY,
    code_format TEXT NOT NULL,   -- 'uuid' | 'opaque'
    note        TEXT NOT NULL
);
```

Seeded with `drugref-moiety` (`uuid`) and the two reserved levels `drugref-clinical-drug` /
`drugref-product`, converging on replay via the `ON CONFLICT … DO UPDATE … WHERE IS DISTINCT FROM` arm
(#214 — a stale seed row must heal, and a converged one must not rewrite).

`cairn_check_medication_coding(p jsonb)`: when `substance.coding` is absent, return (uncoded is
first-class, principle 4). When present, the checks fall into **two tiers**, because the per-type floor
runs at **both** doors — `cairn_event_twin` is called by `submit_event` *and* by `apply_remote_event`
(db/020 §8), deliberately, to close the M8 asymmetry:

| Tier | Check | Local door | Remote door |
|---|---|---|---|
| **Structural** | `system`, `code`, `display` each a non-empty string | refuse | refuse |
| **Registry-derived** | `system` names a registered row | refuse | admit |
| **Registry-derived** | under `code_format = 'uuid'`, `code` parses as a UUID | refuse | admit |

Structural refusal at both doors matches how `substance.term` is already treated: a malformed coding
object is a broken event, not a difference of opinion. The registry-derived tier is where ADR-0051's
strict-submit / lenient-apply belongs — a peer may legitimately run a newer or locally-extended registry,
so its event is admitted and projected verbatim (it simply never matches a dup-key). `display` is
structural, not optional, because it is the honest-degradation label: a coding without it gives a
drugref-less reader nothing beyond `term`, the exact failure ADR-0059 decision 4 exists to prevent.

The registry signature `check_fn(text, jsonb)` is fixed, so the door is detected the way db/031 already
detects it — `current_setting('cairn.remote_apply', true)`, the same idiom
`cairn_guard_medication_patient` uses.

The registry is the mechanism-not-policy expression of ADR-0059 decision 7 (*"a deployment may plug a
different authority"*): substituting an authority is a row, not a patch. It follows the register-by-row
pattern already used by `event_type_class`, `cairn_event_twin_check` and `cairn_projection_apply`.

**Why a new file rather than an edit to db/031.** `SCHEMA_GENERATION` is derived from the newest `db/`
prefix, and this is a **floor** change: #188 exists so an older binary cannot `CREATE OR REPLACE` a newer
safety check back down. 40 → 41, plus the entry in cairn-node's `SCHEMA` list. cairn-sync's list carries
no medication files at all and legitimately lags ([#284](https://github.com/cairn-ehr/cairn-ehr/issues/284)).

db/031's `cairn_check_medication_assertion` calls the new function. plpgsql resolves the call at first
execution, not at `CREATE` time, so the later file is fine — the same late-binding db/031 already relies
on for `cairn_medication_thread_patient`. A load-order test asserts both the function and the seeded
registry exist after a connect, so a missing db/041 fails as a test, not as a first-write surprise.

## 3. Projection plane

### 3.1 A separate `medication_coding` table

```sql
CREATE TABLE IF NOT EXISTS medication_coding (
    medication_id   UUID PRIMARY KEY,
    patient_id      UUID NOT NULL,
    coding_system   TEXT NOT NULL,
    coding_code     TEXT NOT NULL,
    coding_display  TEXT NOT NULL,
    hlc_wall        BIGINT  NOT NULL,
    hlc_counter     INTEGER NOT NULL,
    origin          TEXT    NOT NULL,
    content_address BYTEA   NOT NULL,
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
```

Lives in **db/031** (a view created there must be able to reference it) and is written by
`medication_statement_apply` **only when coding is present**, under the standard
`cairn_hlc_overlay_wins` overlay arm. `medication_coding` joins the fn's `projection_tables` inventory.

Not columns on `medication_statement`, for two reasons:

- **One home for one fact.** Slice 6b's overlay events write the effective coding; had the inline coding
  also lived on the statement row, every reader would need a precedence rule between two homes.
- **6b becomes purely additive.** The overlay apply fns write rows into this same table under the same
  winner rule — no view bodies rewritten, no column sets touched.

A later uncoded re-assertion writes **no** row, so it cannot silently clear a coding; retraction is 6b's
correction event. `heal_safe` stays `TRUE` — this is a DO-UPDATE overlay, not the DO-NOTHING shape
[#277](https://github.com/cairn-ehr/cairn-ehr/issues/277) warns about. Note for 6b: once the overlay types
also write here, `medication_coding` becomes a multi-type table, and db/039 already refuses a narrow
`cairn_reproject` prefix over one.

### 3.2 Widened read views

`patient_medication`, `patient_medication_current` and `patient_medication_past` gain
`coding_system, coding_code, coding_display` **appended at the end**, via a LEFT JOIN to
`medication_coding`. The identical trailing column list must appear in **every file that creates each
view** — db/031, db/032 and db/033 — because the loader replays all of them on every connect and db/031
runs first: a narrower definition downstream fails with *"cannot drop columns from view"* on the next
connect (#207). The failure is loud and immediate (every DB-gated test connects), so the risk is detection
cost, not silent drift.

`medication_statement.inn_code` and the views' `inn_code` column both remain, deprecated in place and
read by nothing.

### 3.3 Reconciliation

Dup-key, in db/031 and db/033 (same column name, type and position — only the expression changes):

```sql
coalesce('code:' || coding_system || '|' || coding_code,
         'term:' || lower(btrim(term) COLLATE "C")) AS dup_key
```

The **pair**, never a bare code — ADR-0059 decision 5: once the reserved finer levels exist, a bare code
would re-split the same substance cross-node. Each branch is prefixed so a free-text term can never
collide with a code key. This closes **coded↔coded** (including `Lipitor`↔`atorvastatin` once both are
coded) and, by construction of `coalesce`, does **not** close coded↔uncoded. The tests assert both halves,
including the negative — an overstated claim here is exactly what the ADR's review caught.

`medication_group_display` (db/033) gains the same three columns and a **prefer-coded** winner ordering:
coded members first, then `(coding_system, coding_code)`, then `medication_id`, all `COLLATE "C"`
(ADR-0045). New advisory view `medication_group_coding_conflict`: reconciled groups whose members carry
**two different anchors** — a possible-mis-reconciliation signal, surfaced and never resolved.

## 4. Node surface

`cairn-node`'s medication-assert input and CLI replace `--inn-code` with
`--coding-system / --coding-code / --coding-display`, validated **all-or-nothing** in the orchestrator
(a clear error naming the missing flag, never a partial coding reaching the door). Roughly 25
`inn_code: None` call sites across `cairn-node`, `cairn-sync` and `cairn-event` tests are updated —
a full-workspace `cargo test` is the gate, since per-crate runs miss cross-crate arity breaks.

## 5. Test obligations (RED first)

**Floor** — a valid coding is accepted; each of the three fields missing or empty is refused **at both
doors** (the structural tier); an unregistered `system` is refused at submit **and admitted + projected**
at remote apply; likewise a non-UUID code under a `uuid`-format system (the registry-derived tier); a
payload still carrying `substance.inn_code` is refused at submit and ignored at apply; an assertion with
no coding at all passes unchanged at both doors.

**Projection** — the coding row is written; a replayed/duplicate assert converges by overlay winner; an
uncoded assert writes no row; the widened views expose the coding; the deprecated `inn_code` column is
still present and NULL.

**Reconciliation** — two coded threads sharing an anchor raise exactly one flag; a coded and an uncoded
thread for the same substance still raise none (the negative the ADR insists on); the group display picks
the coded member; two anchors in one group produce a conflict row.

**Honest degradation** — proven by construction: a source guard asserting no `db/` file and no crate
references drugref. With no drugref code in the tree, drugref-absent is the *only* configuration, which is
a stronger proof than a mocked absence.

**Cross-node** — a coded assertion syncs and converges to the identical coding row and dup-key on a second
node (the existing multi-node harness).

## Paper-parity benchmark (§1.2)

- **Paper counterpart:** writing a drug name on a paper medication list — the clinician writes
  *"atorvastatin"*, *"Lipitor"*, or *"little white pill"*. **N = 1** human act; nothing on paper forces a
  code.
- **Architecture-forced steps M = 1.** Coding adds **zero** forced acts: `substance.coding` is optional at
  every layer — builder, floor, projection — and an uncoded term stays a first-class recordable value.
  Pinned by a test (an assert with no coding passes the floor unchanged), so a later slice cannot quietly
  make coding required. Any design that makes coding a required field to save a medication is an
  architecture defect to file (house rule 5).
- **UI bundling target K = 1** — the coding attaches invisibly when a clinician picks a type-ahead
  suggestion.
- **Time / cognitive-load budget:** owed by the slice that first exposes a coding UI (the ADR-0059
  follow-on, in the [#288](https://github.com/cairn-ehr/cairn-ehr/issues/288) med-list neighbourhood).
  This slice exposes only a CLI test/ops surface, which is not the clinician surface the budget measures.

## Risks

- **The three-file view coherence** (#207) is the sharpest edge. Mitigation: one task that edits db/031,
  db/032 and db/033 together, and a reconnect in the test suite immediately after.
- **`inn_code` call-site churn** spans three crates; a per-crate test run hides the break. Mitigation:
  full-workspace `cargo test` before any commit.
- **A stale dev DB** carrying pre-slice `medication_statement` rows: nothing to backfill (no event has ever
  carried a coding), so the generation-change heal is a no-op here rather than load-bearing.
