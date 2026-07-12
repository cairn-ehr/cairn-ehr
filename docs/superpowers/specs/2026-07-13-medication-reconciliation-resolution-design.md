# Design — Medication reconciliation resolution (slice 3 of the `clinical.medication` surface)

**Date:** 2026-07-13 · **Branch:** `feat/medication-reconciliation-slice-3` · **Status:** design, pending
implementation plan.

**Scope of change:** additive Rust (`cairn-event::medication::reconciliation`, `cairn-node::medication`
orchestrators + CLI) + one new DB migration (`db/033_medication_reconciliation.sql`: floor + grouping
projection) + a new **[ADR-0047](../../spec/decisions/0047-medication-reconciliation-resolution.md)**
recording the *why* + a spec version bump (v0.47 → v0.48, `index.md` only). Adds two verbs to the
medication surface opened by [slice 1](2026-07-11-medication-recording-design.md) and extended by
[slice 2](2026-07-12-medication-dose-overlay-design.md). **No new founding principle, no new envelope
field, no floor bypass, no wire change** — this graduates the slice-1/slice-2 §8 deferral
("cross-thread reconciliation resolution") into product code, reusing the settled event-overlay +
connected-component (`person_member`) + §3.9 additive-vs-suppressing primitives.

---

## 1. What this is, and why now — the slice-1 wart

Slice 1 records each medication as an immortal `medication_id` thread and surfaces an **advisory**
`patient_medication_reconciliation_flag` when two *active* threads for one patient share a duplicate key
(`coalesce(inn_code, normalized term)`). It named exactly one "resolution": **cease the redundant
thread.** That is clinically wrong.

Cessation (`clinical.medication-cessation.asserted`) means *the patient stopped taking the drug*. When
two threads are simply the **same ongoing medication recorded twice** — two nodes (hospital + GP), an
encounter re-entry, a brand recorded alongside its generic — ceasing one to silence the flag **fabricates
a stop event**: the drug drops off the current list and the audit log now asserts a discontinuation that
never happened. That is the "slice-1 wart" this slice removes.

The record already has the right shape one level up. The **identity** subsystem never merges two patient
records — it *links* them (`identity.link.asserted`), derives a golden identity by connected component,
and unlinks cleanly (principle 2, §5.7). Two medication threads that turn out to be the same drug are the
identical problem. Slice 3 reuses that pattern verbatim, one level down.

**Blast radius → substrate ([§9](../../spec/language-substrate.md)).** The write path can silently corrupt
the clinical record (a mis-collapse hides a real second medication, or a fabricated cessation asserts a
false stop), so it is **safety-critical**: pure Rust builders + an **in-DB floor** + a projection. The
advisory reconciliation *detection* (the flag) stays where slices 1/2 put it; slice 3 adds the
*resolution*. Structure mirrors slices 1/2 exactly.

## 2. Event vocabulary — two verbs over a thread *pair*

Unlike slices 1/2 (verbs about *one* thread), reconciliation is about *two* threads, so it mirrors the
identity link/unlink algebra. Both preserve the `clinical.medication-<noun>.asserted` naming of the
surface:

| Event type | schema_version | state | Body carries |
|---|---|---|---|
| `clinical.medication-reconciliation.asserted` | `clinical.medication-reconciliation/1` | `reconciled` | `subject_a`, `subject_b` (two `medication_id`s), `provenance`, `reason?` |
| `clinical.medication-separation.asserted` | `clinical.medication-separation/1` | `separated` | same shape — the never-erase reversal (*"these are actually two different drugs"*) |

- Both are **append-only overlays** over a canonical `(low, high)` thread pair, never a mutation of
  either thread or of each other. The latest-HLC assertion for a pair wins its `state` (link then a
  later separation ⇒ edge gone), exactly like `patient_link`.
- Both are `additive`, `targets_other_author=FALSE` (like the identity link and like dose-correction): a
  reconciliation forecloses nothing — **both threads' full histories, dose timelines, and cessations
  survive verbatim in `event_log` and their projections**. So [ADR-0043](../../spec/decisions/0043-suppression-self-only-disagreement-is-additive.md)'s
  suppression owner-gate **does not apply**, and **cross-author reconciliation is allowed** (clinician B
  reconciles threads authored by A and C — normal clinical practice).
- **Reconciliation is allowed between *any* two threads, not only flagged ones.** This is a feature: a
  human can manually reconcile brand↔generic (`Lipitor` ↔ `atorvastatin`), which the deterministic
  `dup_key` flag deliberately cannot detect (that is the deferred Tier-A drug-dictionary case). The floor
  validates *structure only*.

## 3. The signed body (day-one shape)

The two subjects and provenance ride the **signed event**, so their shape is the can't-cheaply-retrofit
piece (the [ADR-0042](../../spec/decisions/0042-concrete-attachment-reference-shape.md) lesson). Fixed
day-one, mirroring the identity link body:

```
clinical.medication-reconciliation.asserted  (and -separation.asserted, identical shape):
  subject_a   : UUID          # a medication_id thread (NOT minted here)
  subject_b   : UUID          # a medication_id thread, distinct from subject_a
  patient     : UUID          # envelope patient_id (all three should match; see §9 tension 1)
  provenance  : string        # REQUIRED, non-empty (§4.1 ladder; "clinician-judgment" / "same-INN" / ...)
  reason      : string | omitted   # free-text ("brand vs generic", "duplicate entry on transfer")
```

- **`subject_a` / `subject_b` are plain UUIDs and are NOT required to exist locally.** In set-union sync a
  reconciliation may replicate before one of the threads it links, or a thread may live on another node.
  Requiring local presence would break the availability floor (AP,
  [ADR-0001](../../spec/decisions/0001-fat-postgres-thin-daemon.md)) — the same reasoning that lets a
  cessation precede its assert in slice 1 and a dose-correction precede its target in slice 2. The
  grouping projection is out-of-order convergent: an edge whose threads are not yet local still stands and
  lights up when they arrive.
- **Deliberately NOT the built-in `target_event_id` field** (whose `submit_event` path forces the target
  to exist locally and is tied to `targets_other_author=TRUE`). A reconciliation targets no single event —
  it relates two *threads* — and must stay offline-first and additive.

**Legibility twin (§3.13).** Every event carries the mandatory mechanically-derived plaintext twin —
reconciliation → *"Reconciled as the same medication as thread &lt;short-b&gt; — brand vs generic
[clinician-judgment]"*; separation → *"Separated from thread &lt;short-b&gt; — recorded as distinct
medications"*. Always non-empty; there is exactly one event and the prose is a rendering *of* it.

## 4. The in-DB floor (the safety-critical seam)

Maximally permissive on clinical judgment (principle 4 / paper-parity — **never block a reconciliation a
clinician asserts**), strict on structure. One shared `cairn_check_medication_reconciliation(p_type, b)`
(mirrors slice-1's one-function-two-verbs pattern and `cairn_check_link_assertion`):

- `subject_a` / `subject_b`: present, string, **valid UUIDs**, **distinct** (a self-reconcile is
  meaningless and would corrupt the component walk — refused, exactly like the identity self-link).
- `patient`: the envelope carries it; well-formed UUID.
- `provenance`: present, non-empty (§4.1 ladder; value-open, `"unknown"` is honest).
- Nothing clinical is forced. No check that the subjects share a `dup_key` (brand↔generic must be
  reconcilable); no existence check on the subjects (offline-first).

**Classification — both `additive`, `targets_other_author=FALSE`** (registered in `event_type_class`).
See §2: a reconciliation is not a suppression; the ADR-0043 owner-gate does not apply.

**Twin dispatch.** Two more branches are added to the `cairn_event_twin` dispatch per the existing
verbatim-branch idiom. This grows the [#173](https://github.com/cairn-ehr/cairn-ehr/issues/173)
dispatch-fragility already filed; slice 3 follows the idiom rather than expanding scope into that refactor
(the call slices 1/2 made).

## 5. The projection — a thread-grouping component (`db/033_medication_reconciliation.sql`)

Mirrors the identity `patient_link` → `person_member` machinery verbatim, over `medication_id` threads
instead of patient UUIDs.

- **`medication_reconciliation`** — the standing-edge HLC overlay. One row per canonical `(low, high)`
  medication-id pair; `state IN ('reconciled', 'separated')`; the latest-HLC assertion wins the state with
  the `content_address` tiebreak ([#115](https://github.com/cairn-ehr/cairn-ehr/issues/115)). Same shape
  and index discipline (an index on the `high` side) as `patient_link`.
- **`medication_group_member`** (`medication_id` → `group_id`) — the golden-medication projection.
  `group_id` = the **minimum `medication_id`** in the connected component of standing `reconciled` edges
  (a derived canonical representative — the "group" is a projection, never a stored id; principle 2). A
  thread never touched by a reconciliation event has no row and collapses to itself. Fed by
  `cairn_recompute_medication_group(seed)`: a **bounded BFS** over standing `reconciled` edges (cost
  bounded by the touched component, not the table — the ADR-0001/Bet-B incremental discipline), an
  **oversize guard** (`cairn_max_medication_group_size()` GUC, default 10000) that **fails loud on local
  authoring and clamps-and-flags on remote apply** (a `medication_projection_flag` row is the alarm +
  worklist — the `identity_projection_flag` pattern, since the cap is a node-local GUC and vetoing a
  validly-signed replicated event would fork the event set between honest nodes). A
  `pg_advisory_xact_lock` on a fixed project constant serializes recomputes (the `patient_link` race fix).
- **Trigger** on both event types folds the one new edge into `medication_reconciliation` (HLC-overlay
  upsert) then recomputes the component around both endpoints.

### 5.1 Group status & current dose — the collapsed row (the two clinical rulings)

A `medication_group_current` view derives, **per group**, one collapsed answer. Winner/ordering tiebreaks
over TEXT keys use `COLLATE "C"` ([ADR-0045](../../spec/decisions/0045-collation-independent-projection-tiebreaks.md))
so two nodes pick the same collapsed row.

**Status (ruling: latest-effective wins on disagreement).** Partition the group's member threads into
*active* (no cessation row) and *ceased* (a cessation row exists — the slice-1 per-thread fact, unchanged):

1. all active → **active**;
2. all ceased → **ceased**;
3. **mixed → latest-*effective* wins**: compare the maximum effective date across the ceased members'
   cessations (`stopped_value`, null → recording time) against the maximum effective date across the
   active members' current dose points. The later one decides — a cessation effective last month **loses**
   to a dose-change effective yesterday (still on it); a dose-change effective last month **loses** to a
   cessation yesterday (stopped). Ties break by `(hlc_wall, hlc_counter, origin, content_address)` under
   `COLLATE "C"`, fully convergent.

> **Single-thread groups can never be "mixed"** (one member is wholly active or wholly ceased), so this
> rule reduces **exactly** to slice-1/2 semantics for a lone thread. Shipped single-thread behavior is
> provably unchanged; the latest-effective machinery is **inert** until a genuine multi-thread
> disagreement exists. Pinned by regression tests.

**Current dose (when active).** The latest-*effective* dose point across the group's **active** members —
the group-level extension of slice-2's per-thread `medication_current_dose` (same bitemporal ISO-lexical
`COLLATE "C"` sort key; correction overlay applied per point). Built **on top of** slice-2's per-thread
view, which stays intact so `resolve_correction_target` (slice-2's "default the correction target to the
current dose point of a thread") keeps working. For a singleton group the group dose = the thread's own
`medication_current_dose` — provably identical, regression-pinned.

### 5.2 The collapsed current/past views + the flag

- **`patient_medication_current` / `patient_medication_past`** are reworked to emit **one row per group**
  (keyed by `group_id`, surfaced in the existing `medication_id` column). **CRITICAL: keep the exact same
  column set as db/032** — `connect_and_load_schema` replays db/031 → db/032 → db/033 on every connect, so
  widening these views would make db/032's narrower `CREATE OR REPLACE` fail on the next reconnect with
  "cannot drop columns from view" (the recorded `migration-replay-no-view-widening` hazard). Only the
  *rows* (collapsed by group) and the *dose/status source* change; column names/types/order are byte-for-
  byte identical to db/032. A thread never reconciled is a group of one → its `medication_id` and behavior
  are unchanged (self-healing, no data migration). Display term/substance for a group comes from the
  canonical (min-UUID) member — a documented approximation (§9 tension 2).
- **`patient_medication_reconciliation_flag`** is reworked (`CREATE OR REPLACE`, replacing db/031's
  definition) to fire only when active threads sharing a `dup_key` span **more than one distinct
  `group_id`**. So reconciling two same-key threads (they land in one group) **clears the flag**; a later
  separation (they split back into two groups) **re-fires it** (if they still share a key). No false
  cessation anywhere in the loop.

## 6. Node orchestrators & CLI (`cairn-node`)

Mirrors slices 1/2; **device-additive** throughout (`recorded` contributor, no responsibility attestation
— consistent with slices 1/2, since human-attested clinical responsibility is not built yet). Because the
verbs are `additive`+`targets_other_author=FALSE`, a future attested reconciliation adds a
responsibility-bearing contributor with **zero floor change** (identity's exact pattern).

- **`crates/cairn-event/src/medication/reconciliation.rs`** — pure `reconciliation_body` /
  `separation_body` (or one builder parameterized by state) + `render_*_twin`. A new module in the
  existing `medication/` directory (already split `assert`/`cessation`/`dose` in slice 2), keeping every
  file under the 500-line guideline.
- **`crates/cairn-node/src/medication.rs`** — async `reconcile_medications(...)` /
  `separate_medications(...)` orchestrators (mint `event_id`, stamp HLC, sign, 1-arg `submit_event`).
  Watch the file's line count; extract a `reconciliation` submodule if it approaches 500 lines.
- **CLI:** `medication reconcile <thread_a> <thread_b> [--provenance --reason]`;
  `medication separate <thread_a> <thread_b> [--provenance --reason]`. Two explicit thread ids (a
  flag-driven "reconcile these N" picker is UI-tier, out of scope). **`--provenance` defaults to
  `"clinician-judgment"`** in the orchestrator when omitted, so the floor's required-non-empty
  `provenance` (§4) is always satisfied without forcing the clinician to type it — the common bedside
  case is a human judgment call.

## 7. Testing (TDD, failing-test-first)

**Pure builder tests (no DB):** bodies carry distinct subjects + non-empty provenance; self-reconcile
rejected by the Rust guard mirroring the floor; twins non-empty and read naturally; state field correct
per verb.

**DB-gated integration tests:**
- reconcile two active same-key threads → `patient_medication_current` shows **one** row; the flag
  **clears**;
- **separate** re-splits → two rows again, flag **re-fires**;
- reconcile **brand↔generic** (no shared `dup_key`, never flagged) → collapses to one row anyway;
- group **current dose** = latest-effective dose point across members;
- **mixed active/ceased** disagreement → latest-effective wins (both directions: later dose ⇒ active;
  later cessation ⇒ ceased);
- **single-thread regression**: a lone active thread and a lone ceased thread render exactly as slices 1/2
  (the reduction proof, guarding against silent shipped-behavior change);
- **out-of-order convergence**: a reconciliation (or separation) applied before one/both subject threads
  are local converges when they arrive (offline-first);
- **cross-author** reconcile allowed (no owner-gate);
- **min-UUID canonical** is the group representative; a 3-thread transitive component (A–B, B–C) collapses
  to one group; separating B–C re-splits cleanly;
- **self-reconcile** refused at the floor; positive control.

Crypto material in tests is **runtime-derived**, never literal (house rule 6 / issue #146).

## 8. Deliverables checklist

- `db/033_medication_reconciliation.sql` (floor + edge overlay + grouping projection + reworked
  collapsed views + reworked flag).
- `crates/cairn-event/src/medication/reconciliation.rs` + `mod.rs` re-exports.
- `crates/cairn-node/src/medication.rs` orchestrators + CLI wiring.
- `crates/cairn-node/tests/medication_reconciliation.rs` (DB-gated) + pure builder tests.
- **[ADR-0047](../../spec/decisions/0047-medication-reconciliation-resolution.md)** (the *why*) + spec
  version bump v0.47 → v0.48 in `index.md` + the ADR index row in HANDOVER/decisions README.
- HANDOVER.md + ROADMAP.md updated (slice 32).

## 9. Design tensions recorded

1. **Cross-patient reconciliation** (linking patient X's and Y's threads) is a pathology the offline-first
   floor cannot cheaply reject (both threads' patients may be non-local at submit time). The collapse view
   groups per patient, so a bad edge is low-stakes, reversible, and auditable. A hard guard (refuse/flag a
   cross-patient component in the recompute) is **deferred**, filed as a GitHub issue (house rule 5) — not
   left implicit. (Identity has no analogue because its link subjects *are* patient UUIDs; here the
   subjects are threads and the patient is a separate field.)
2. **Group display term** = the canonical (min-UUID) member's term — a documented approximation. For a
   reconciled brand↔generic pair, *which* name shows is arbitrary (min-UUID, not "prefer the INN"); a
   term-preference rule is **deferred**.
3. **#173 twin-dispatch** grows by two branches (following the idiom, not the refactor).
4. **Current dose / status stay projection conveniences, not truth claims** (principle 4), inherited from
   slices 1/2. Active-review / last-confirmed staleness remains deferred.
5. **Migration replay:** the three reworked views (`patient_medication_current`, `_past`,
   `patient_medication_reconciliation_flag`) MUST keep byte-identical column sets to their db/031/db/032
   definitions (the recorded replay hazard) — collapse changes rows and dose/status source, never columns.

## 10. Explicitly out of scope for slice 3

Deferred, each its own later slice:

- **A hard cross-patient reconciliation guard** (tension 1) — filed as an issue.
- **A term-preference (prefer-INN) display rule** for reconciled groups (tension 2).
- **Human-attested clinical responsibility** on a reconciliation event (device-additive throughout, like
  slices 1/2; the floor already supports it additively).
- **Fuzzy / automatic reconciliation proposals** (brand↔generic detection, typos, salts) — waits on the
  Tier-A drug dictionary; slice 3 provides only the human-driven *resolution*, not automated *detection*.
- **The #173 twin-dispatch registry refactor** — slice 3 adds two branches the old way.
- **Extending the [#157](https://github.com/cairn-ehr/cairn-ehr/issues/157) HLC-collision advisory** onto
  the new reconciliation/grouping projections — the consistency follow-on deferred with the medication
  projections generally.
