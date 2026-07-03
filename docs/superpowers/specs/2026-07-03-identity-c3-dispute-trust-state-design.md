# C3 — `dispute` + the chart trust-state projection

**Date:** 2026-07-03 · **Spec home:** §5.7 (identity event algebra) / §5.5(b) (identity theft) ·
**Principle:** 2 (never merge, always overlay), 4 (acknowledged uncertainty — the chart tells the
clinician how much to trust the identity behind it) · **Blast radius:** safety-critical (in-DB / Rust) —
a defect could either mis-flag a good chart or, worse, *fail to flag* a disputed one.

## Why this slice

The remaining §5.7 algebra after C1 (`link`/`unlink`) and C2/C2b (the proposal→apply seam) is
`identify` / `repudiate` / `reattribute` / `dispute`. Each of these drives or reads a **chart
trust-state contract** (*confirmed / unconfirmed / under-review*) that the spec calls the
"projection-side contract" — and which **does not exist yet**. C1 explicitly deferred it:

> Chart *trust states* (confirmed / unconfirmed / under-review, §5.7 projection-side contract) — a
> later read-side concern. *(db/018 C1 design, "Out of scope")*

So C3 builds that keystone, driven by the simplest, most self-contained event in the set:

- **`dispute`** (§5.7): patient-initiated "I was never there in March" → the named chart renders
  *under-review* and enters a triage worklist; resolution clears it. It is the §5.5(b) identity-theft
  **front door**, and — unlike `reattribute` (event-granular strike-through + tiered adjudication) or
  `repudiate` (alias pool + suppressing semantics) — it needs **no** new machinery beyond the trust
  projection itself. It is an *additive* assertion (it annotates trust; it never erases, moves, or
  blocks anything), so it flows through the **existing** `submit_event` door with the same low ceremony
  as `link`.

Building the trust projection here, driven by `dispute`, means `identify` (C4/C5, adds *unconfirmed*),
`reattribute` (adds *under-review* from a pending move), and the §5.2 coherence check (adds
*under-review* from a demoted link) each **compose one more source** into the projection later, with no
rewrite.

**Deliverable boundary.** This slice delivers: the two dispute event types, their structural floor, a
standing `chart_dispute` edge overlay, and the `chart_trust` effective-state projection surfaced on the
`person_chart` read. It does **not** deliver a separate queue subsystem (the open-dispute set *is* the
triage worklist — a query, not a new table), notification/contamination cascade (that is a
`reattribute`-tier concern, §5.5), or the *unconfirmed* trust state (needs registration-class /
identity-pending, C4/C5).

## Architecture & components

Two new event types flow through the **existing** `submit_event` door (db/005) — unchanged. New types
register in `event_type_class` and add a branch to the `cairn_event_twin` hook, exactly as the
demographics slices (db/010) and C1 (db/018) did. `cairn-sync`/federation get dispute for free: ordinary
signed events that sync set-union.

| Layer | File | What |
|---|---|---|
| Rust builders (pure) | `crates/cairn-event/src/identity.rs` | add `DisputeAssertion` / `DisputeResolution` body builders + twin renderers. Pure functions, no I/O — mirrors the C1 `LinkAssertion` builders in the same file. |
| In-DB floor + projection | `db/023_identity_dispute.sql` | event-type registration, `cairn_check_dispute_assertion` structural floor, `cairn_event_twin` dispute branch, `chart_dispute` overlay table + AFTER-INSERT trigger, `chart_trust` effective-state VIEW, `person_chart` extended with a `trust_state` column. |
| Migration wiring | `crates/cairn-node/src/db.rs` | one `SCHEMA` list entry (`023_identity_dispute`). |
| Tests | `crates/cairn-node/tests/identity_dispute.rs` + `cairn-event` unit tests | TDD, red-first. |

No `submit_event` re-declaration; additive DDL only. No SCHEMA-version bump (there is no numeric version
gate; the migration list is the loader). The safety-critical write door stays single-source.

## Event model & structural floor

A dispute has its **own identity** (`dispute_id`), because one chart can carry several concurrent,
independently-resolvable disputes ("I was never there in March" *and* "someone else's labs are on my
chart"). The standing state of each dispute (open → resolved) overlays by HLC per `dispute_id`, exactly
the shape C1 uses for the link edge per `(low, high)` pair.

**`identity.dispute.asserted`** — opens a dispute:

```json
{
  "dispute_id": "<uuid>",
  "subject": "<patient uuid under dispute>",
  "reason": "<required, non-empty; value-open — 'patient states never attended', 'suspected identity theft', 'unknown'>"
}
```

**`identity.dispute.resolved`** — closes a specific dispute:

```json
{
  "dispute_id": "<uuid — the dispute being closed>",
  "subject": "<patient uuid — carried on both, see below>",
  "resolution": "<required, non-empty; value-open — 'dismissed, no evidence', 'upheld → reattribution filed', ...>"
}
```

Both register as **`additive`, `targets_other_author = FALSE`**.

**Why `subject` is carried on the resolution too (not just the open).** Offline-first requires
convergence under out-of-order arrival (pinned by C1's tests). If a `resolved` can land *before* the
`asserted` it closes (a real sync ordering), the overlay row must still know which chart the dispute is
about. Carrying `subject` on both makes every assertion self-describing: the overlay keys on
`dispute_id`, and the highest-HLC assertion sets both `state` and `subject`. Since `subject` is
immutable for a given dispute in honest use, open and resolve agree on it; the projection never depends
on arrival order.

**Why additive / no mandatory attestation, and why that is safe.** Opening a dispute does not suppress,
move, or erase any event — it adds a trust annotation and queues a human. So the existing `submit_event`
gate handles both authoring paths with no new logic, identical to C1's link:

- A **patient-portal / clerk actor** files a dispute with no responsibility-bearing contributor → no
  attestation required (it is advisory: it flags for review, it does not act).
- A **clinician who takes responsibility** for the dispute includes a responsibility-bearing contributor
  → the db/005 gate *already* forces a valid human attestation on it.

Safety does not come from a write-time block on the dispute; it comes from the fact that under-review is
a **non-blocking trust annotation** (the chart still reads — principle 3, never block care) and every
dispute is **fully attributable and reversible** (a resolution clears it). *Considered and rejected: a
"denial-of-trust" flood — anyone enrolled opening spurious disputes to mark charts under-review.* On
paper anyone may raise a query against a record; the mitigation is that every dispute names its author in
the audit log and is one resolve-event away from cleared, and rate-limiting is an API-tier policy
concern, not a floor concern. Refusing to record an honest dispute would itself fail paper-parity.

**Structural floor** (`cairn_check_dispute_assertion(p_type, b)`, culture-neutral, in-DB — the
`cairn_check_identifier_assertion` / `cairn_check_link_assertion` pattern). Each violation is a distinct
legible exception:

- `dispute_id` present, a valid UUID string.
- `subject` present, a valid UUID string.
- The descriptive field required for this type — `reason` for `asserted`, `resolution` for `resolved` —
  present and a non-empty string (§4.1: honest, value-open; *"unknown"* is acceptable, an empty string
  is not — no required field satisfiable only by fabrication, principle 4).
- **No** cross-existence check on `subject`: a dispute may legitimately arrive before or independently of
  the chart it names (offline-first, set-union — the safety signal must exist even if the body has not
  synced yet, mirroring §5.9). The projection carries the subject either way.

The authored §4.5 legibility twin is **HARD-required non-empty** (same rule as demographics and C1
link) — identity events must stay legible without their schema. Rendered e.g.
`dispute opened: <subject> — <reason> (dispute <dispute_id>)` /
`dispute resolved: <subject> — <resolution> (dispute <dispute_id>)`.

## Projections & the trust-state contract

### `chart_dispute` — standing-dispute overlay (trigger-maintained TABLE)

The append-only truth is `event_log`; this is the standing-state projection, same role as C1's
`patient_link`.

```
chart_dispute(
    dispute_id  UUID PRIMARY KEY,          -- the dispute's own identity
    subject     UUID   NOT NULL,           -- the patient chart under dispute
    state       TEXT   NOT NULL,           -- 'open' | 'resolved'
    reason      TEXT,                       -- carried from the winning assertion (reason or resolution)
    hlc_wall    BIGINT NOT NULL,
    hlc_counter INT    NOT NULL,
    origin      TEXT   NOT NULL,
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
)
-- index the standing-open set by subject: the trust VIEW's hot lookup + the triage worklist.
CREATE INDEX chart_dispute_open_subject_idx ON chart_dispute (subject) WHERE state = 'open';
```

An AFTER-INSERT trigger on `event_log` (fired only for the two dispute types) upserts the row keyed by
`dispute_id` and **overlays by HLC** — the latest `(hlc_wall, hlc_counter, origin)` wins the `state`,
identical tiebreak to db/002 / C1. So `asserted` then a later `resolved` ⇒ `state='resolved'`;
`resolved` arriving before the older `asserted` ⇒ stays `resolved`. *Never merge, always overlay.*

Unlike C1's `patient_link`, this needs **no** connected-component recompute — a dispute is a single-row
standing fact, so the trigger is a plain overlay upsert (cheaper than C1). No oversize guard is required
(there is no BFS to blow up).

### `chart_trust` — the effective trust-state projection (VIEW)

The §5.7 contract is *confirmed / unconfirmed / under-review*. This slice introduces the
**under-review** state (driven by an open dispute) against the **confirmed** default. It is delivered as
a **thin VIEW** — consistent with C1's `person_chart` being a VIEW, and with the ADR-0001/Bet-B
discipline: the trust of a chart is a *bounded, indexed* `EXISTS` over `chart_dispute`, not a
full-projection recompute (the thing Bet B replaced the poc VIEWs to avoid). One source now; the VIEW is
shaped so future sources compose as additional precedence branches.

```sql
-- Effective trust state per disputed/known subject. Precedence (highest-severity wins),
-- built so later slices ADD a branch, never rewrite:
--   under-review  ⟵ any standing OPEN dispute            (THIS slice)
--   [under-review ⟵ pending reattribution]               (C4, §5.5 — future)
--   [under-review ⟵ coherence-check demoted link]        (§5.2 feedback loop — future)
--   [unconfirmed  ⟵ identity-pending registration]       (C4/C5, §5.4 — future)
--   confirmed     ⟵ default
CREATE VIEW chart_trust AS
    SELECT subject AS patient_id, 'under-review'::text AS trust_state
    FROM chart_dispute WHERE state = 'open'
    GROUP BY subject;
```

`chart_trust` returns a row only for a subject that is *not* in the default state — a subject with no
open dispute has no row (the read below coalesces a missing row to `'confirmed'`). This keeps the VIEW
tiny and makes "the triage worklist" a trivial `SELECT * FROM chart_dispute WHERE state='open'`.

### Surfacing on the unified read

`person_chart` (C1) is extended with a `trust_state` column via `CREATE OR REPLACE VIEW` (append-only —
allowed, and additive by principle 11). Every member row reports its **own** chart's trust state,
coalescing to `'confirmed'` when unknown to `chart_trust`:

```sql
CREATE OR REPLACE VIEW person_chart AS
    SELECT COALESCE(pm.person_id, pc.patient_id) AS person_id,
           pc.*,
           COALESCE(ct.trust_state, 'confirmed') AS trust_state
    FROM patient_chart pc
    LEFT JOIN person_member pm ON pm.patient_id = pc.patient_id
    LEFT JOIN chart_trust   ct ON ct.patient_id = pc.patient_id;
```

*Trust attaches to the `patient_id` (the chart the dispute names), not the aggregated `person_id`.* A
dispute is against a specific registration; whether an under-review member taints the whole
person-level view is a read-surface (API/UI) judgment above the foundation line, deliberately out of
scope. Per-row trust is honest and composes cleanly.

## Error handling & convergence edge cases (pinned by tests)

- **Open → resolve converges** (later-HLC resolve wins → confirmed).
- **Out-of-order:** resolve@200 lands before open@100 → row stays `resolved`; the older open does not
  reopen it. (The offline-first convergence requirement, same as C1's link/unlink test.)
- **Multiple disputes on one subject:** resolving one leaves the chart under-review while another stays
  open; resolving all → confirmed.
- **Idempotent re-assert:** `submit_event` dedups by content-address; the trigger upsert is idempotent —
  re-applying the same dispute is a no-op (and does not create a second `chart_dispute` row).
- **Dispute before chart:** a dispute naming a subject with no `patient_chart` row still reports
  under-review for that subject via `chart_trust` (safety signal without the body; `person_chart` only
  lists it once the chart arrives, which is correct for a *chart* read).
- **Floor rejections**, each a distinct legible exception: missing/invalid `dispute_id`, missing/invalid
  `subject`, empty `reason` (asserted) / empty `resolution` (resolved), empty authored twin.

## Testing (TDD, red-first)

- **Rust unit** (`cairn-event`): dispute body shape (`dispute_id`/`subject`/`reason`), resolution body
  shape (`resolution`), twin rendering distinguishes opened vs resolved and includes subject +
  dispute_id.
- **DB integration** (`crates/cairn-node/tests/identity_dispute.rs`, gated on `$CAIRN_TEST_PG`,
  serialized via `db::test_serial_guard`): valid dispute accepted; opens → `chart_trust`
  under-review + `person_chart.trust_state='under-review'`; resolve → confirmed; out-of-order resolve
  wins; two disputes, resolve-one stays under-review, resolve-all confirmed; idempotent re-assert is one
  row; dispute-before-chart still reports under-review; no-dispute chart reads `'confirmed'`; floor
  rejections (bad dispute_id, missing subject, empty reason, empty resolution, missing twin).

Every test drives a specific behaviour before the code exists.

## Out of scope for C3 (deferred, recorded)

- The *unconfirmed* (identity-pending) trust state + registration classes / John Doe (§5.4) — C4/C5,
  where `identify` moves unconfirmed → confirmed.
- `reattribute` (§5.5 event-granular strike-through + tiered adjudication) and `repudiate` (alias pool +
  suppressing semantics) — later slices; each *composes* one more source into `chart_trust`.
- The §5.2 coherence-check feedback loop demoting a link to under-review — a matcher-side concern that
  will add its own source branch to `chart_trust`.
- Notification / contamination cascade on dispute (§5.5) and the disclosure-scope query — reattribution
  concerns; a dispute here only flags + worklists.
- Person-level trust aggregation (does an under-review member taint the person view?) — read-surface /
  API-UI tier.
- No spec/ADR change: this implements the settled §5.7 algebra and the §5.7 projection-side contract; it
  refines nothing that needs a new decision.
