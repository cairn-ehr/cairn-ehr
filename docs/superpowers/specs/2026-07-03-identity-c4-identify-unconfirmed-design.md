# C4 — `identify` + the *unconfirmed* trust state

**Date:** 2026-07-03 · **Spec home:** §5.7 (identity event algebra) / §5.4 (unidentified registration —
John Doe) · **Principle:** 2 (never merge, always overlay), 4 (acknowledged uncertainty — the chart tells
the clinician *how much* to trust the identity behind it, incl. "we don't yet know who this is") ·
**Blast radius:** safety-critical (in-DB / Rust) — a defect could either fail to flag an identity-pending
chart (a John Doe silently reading as a fully-known patient) or mis-rank it against an open dispute.

## Why this slice

C3 built the §5.7 **chart trust-state contract** as a projection (`chart_trust`) and delivered two of its
three states: **confirmed** (the default — no row) and **under-review** (driven by an open `dispute`). It
deliberately shaped the VIEW so later slices *add a source branch*, and it named the missing third state
as the next compose:

> `[unconfirmed  ⟵ identity-pending registration]   (C4/C5, §5.4 — future)` *(db/023 chart_trust header)*

This slice supplies that third state, **unconfirmed**, and the §5.7 `identify` event that clears it:

- **identity-pending** (§5.4): an unconscious / unknown patient ("John Doe") gets a UUID immediately and
  care proceeds; the chart renders in **unconfirmed** trust mode ("no history available; allergies
  unknown"). That is a *known-benign* workflow state — the data on the chart genuinely belongs to this
  (as-yet-unnamed) person; we simply have not established *who* they are.
- **`identify`** (§5.7): "identity-pending → confirmed. Human; method recorded." Establishing who the
  patient is (driver's licence, family confirmation, biometric match, …) flips the chart to **confirmed**.
  Linking to a *prior* chart, when one exists, is a **separate ordinary `link` assertion** (C1, already
  built) — `identify` records only that this chart's own identity is now established, and by what method.

Building the unconfirmed state here completes the trust-state contract (all three states now exist and
compose), so the remaining algebra — `reattribute` (adds another *under-review* source: a pending move)
and the §5.2 coherence check (adds *under-review* from a demoted link) — each still just **compose one
more branch**, with no rewrite.

**Deliverable boundary.** This slice delivers: two additive event types (`identity.pending.asserted` /
`identity.identify.asserted`), their structural floor, a standing `chart_identity_state` overlay keyed by
subject, and the reworked `chart_trust` projection that composes **under-review (dispute) ⊔ unconfirmed
(pending)** by highest severity. It does **not** deliver the full §5.4 John-Doe registration subsystem
(system-generated callsign, clinician-observed evidence assertions — age/marks/belongings, matcher re-run
on new evidence), nor the "prior history now available" push alert on link, nor registration-class
partitioning of the create funnel (§5.3/§5.8). Those are §5.4 *workflow* pieces above the foundation line;
this slice builds only the **trust-state primitive** — the event pair that opens and closes the
unconfirmed state — exactly as C3 built the dispute primitive, not a queue subsystem.

## Architecture & components

Two new event types flow through the **existing** `submit_event` door (db/005) — unchanged. New types
register in `event_type_class` and add a branch to the `cairn_event_twin` hook, exactly as demographics
(db/010), C1 (db/018), and C3 (db/023) did. `cairn-sync`/federation get them for free: ordinary signed
events that sync set-union.

| Layer | File | What |
|---|---|---|
| Rust builders (pure) | `crates/cairn-event/src/identity.rs` | add `PendingAssertion` / `IdentifyAssertion` body builders + twin renderers. Pure, no I/O — mirrors the C1 `LinkAssertion` and C3 `DisputeAssertion` builders in the same file. |
| In-DB floor + projection | `db/024_identity_identify.sql` | event-type registration, `cairn_check_identity_state_assertion` structural floor, `cairn_event_twin` branch, `chart_identity_state` overlay table + AFTER-INSERT trigger, and the **reworked** `chart_trust` VIEW (severity-max compose). `db/023` is left **untouched** (this migration CREATE-OR-REPLACEs the shared view/hook from a later step, never edits the earlier file — the established slice pattern). |
| Migration wiring | `crates/cairn-node/src/db.rs` | one `SCHEMA` list entry (`024_identity_identify`). |
| Tests | `crates/cairn-node/tests/identity_identify.rs` + `cairn-event` unit tests | TDD, red-first. |

No `submit_event` re-declaration; additive DDL only. No SCHEMA-version bump (the migration list is the
loader). The safety-critical write door stays single-source.

## Event model & structural floor

**Keyed by `subject`, not a separate id — the deliberate contrast with `dispute`.** A dispute has its own
`dispute_id` because one chart can carry *several* concurrent, independently-resolvable disputes.
Identity-pending is a **single per-chart lifecycle state** — a chart is either identity-pending or it is
not — so the natural key is the **subject** itself. The standing state (pending ⇄ identified) overlays by
HLC per subject. This makes C4 *simpler* than C3 (no per-dispute id, and — because the key *is* the
subject — no subject-consistency guard is possible or needed).

**`identity.pending.asserted`** — opens the unconfirmed state (the §5.4 John-Doe front door):

```json
{
  "subject": "<patient uuid registered identity-pending>",
  "basis":   "<required, non-empty; value-open — 'unconscious ED arrival, no ID', 'John Doe', 'unknown'>"
}
```

**`identity.identify.asserted`** — establishes identity → confirmed (§5.7 "who, method"):

```json
{
  "subject": "<patient uuid now identified>",
  "method":  "<required, non-empty; value-open — 'driver's licence', 'family confirmation', 'biometric match', ...>"
}
```

Both register as **`additive`, `targets_other_author = FALSE`**.

**Lifecycle is full overlay (never merge, always overlay).** pending → identify ⇒ confirmed; a *later*
pending (higher HLC) re-opens unconfirmed (a mis-identification retracted, chart re-registered
identity-pending). Out-of-order arrival converges to the highest-HLC assertion, arrival-order-independent,
exactly as C1's link/unlink and C3's dispute/resolve.

**Why additive / no mandatory attestation, and how §5.7's "Human; method recorded" is still honoured.**
An identify neither suppresses, moves, nor erases any event — it adds a trust annotation (and structurally
records the *method*). So it flows through the existing `submit_event` gate with the same low ceremony as
C1/C3:

- The **method is structurally required** (non-empty `method`) — that is "method recorded" enforced at the
  floor, for every identify, regardless of policy.
- The **"Human" vouching composes via the existing attestation gate**, not a new floor rule: an identify
  authored with a responsibility-bearing contributor trips the db/005 gate, which already forces a valid
  human attestation on it. *Whether* a given deployment mandates that is workflow-tier policy — the §5.5
  discipline of "granularity/mechanism in the primitive, risk control in the workflow tier." Forcing
  attestation at the event-type level would make `identify` the first identity event to diverge from the
  uniform additive pattern for a policy concern. **Considered and rejected** on that basis; the mechanism
  to require a human already exists and is expressible without a floor special-case.

**Structural floor** (`cairn_check_identity_state_assertion(p_type, b)`, culture-neutral, in-DB — the
`cairn_check_link_assertion` / `cairn_check_dispute_assertion` pattern). Each violation is a distinct
legible exception:

- `subject` present, a valid UUID string.
- The descriptive field required for this type — `basis` for `pending`, `method` for `identify` — present
  and a non-empty string (§4.1: honest, value-open; *"unknown"* is acceptable, an empty string is not — no
  required field satisfiable only by fabrication, principle 4).
- **No** cross-existence check on `subject`: an identity-pending marker (or an identify) may legitimately
  arrive before or independently of the chart it names (offline-first, set-union — the safety signal must
  exist even if the body has not synced yet, mirroring §5.9 and C3). The projection carries the subject
  either way.

The authored §4.5 legibility twin is **HARD-required non-empty** (same rule as demographics, C1 link, and
C3 dispute — identity events must stay legible without their schema). Rendered e.g.
`identity pending: <subject> — <basis>` / `identity confirmed: <subject> via <method>`.

## Projections & the trust-state contract

### `chart_identity_state` — standing identity-status overlay (trigger-maintained TABLE)

The append-only truth is `event_log`; this is the standing-state projection, same role as C1's
`patient_link` and C3's `chart_dispute`.

```
chart_identity_state(
    subject     UUID PRIMARY KEY,       -- the chart's own uuid IS the key (one status per chart)
    state       TEXT   NOT NULL,        -- 'pending' | 'identified'
    detail      TEXT,                   -- the winning assertion's descriptive text: basis (pending) or method (identified)
    hlc_wall    BIGINT NOT NULL,
    hlc_counter INT    NOT NULL,
    origin      TEXT   NOT NULL,
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
)
-- index the standing-pending set: the trust VIEW's unconfirmed source + the "still-John-Doe" worklist.
CREATE INDEX chart_identity_state_pending_idx ON chart_identity_state (subject) WHERE state = 'pending';
```

An AFTER-INSERT trigger on `event_log` (fired only for the two types) upserts the row keyed by `subject`
and **overlays by HLC** — the latest `(hlc_wall, hlc_counter, origin)` wins the `state`, identical
tiebreak to db/002 / C1 / C3. `pending` then a later `identify` ⇒ `state='identified'`; an `identify`
arriving before the older `pending` ⇒ stays `identified`. *Never merge, always overlay.*

Unlike C1's `patient_link`, this needs **no** connected-component recompute and **no** oversize guard — a
per-chart status is a single standing row, so the trigger is a plain HLC-guarded upsert (cheaper than C1,
same as C3). And unlike C3's `chart_dispute`, there is **no subject-consistency guard**: the key *is* the
subject, so a "rebind to a different subject" is structurally impossible.

### `chart_trust` — the effective trust-state projection (VIEW), reworked to compose by severity

The §5.7 contract is *confirmed / unconfirmed / under-review*. C3 delivered under-review against the
confirmed default from one source. C4 adds **unconfirmed** from a second source and turns the VIEW into a
**highest-severity-wins** overlay (the same "effective grade is the highest standing assertion" discipline
as the §5.9 sensitivity projection). Column contract is unchanged (`patient_id uuid`, `trust_state text`),
so `CREATE OR REPLACE VIEW` stays reload-idempotent and `person_chart_trust` (C3) is untouched.

```sql
CREATE OR REPLACE VIEW chart_trust AS
WITH trust_source(patient_id, severity) AS (
    -- under-review (2): any standing OPEN dispute            (C3, §5.5(b))
    SELECT subject, 2 FROM chart_dispute       WHERE state = 'open'
    UNION ALL
    -- unconfirmed  (1): a standing identity-pending chart    (C4, §5.4)   <-- THIS slice
    SELECT subject, 1 FROM chart_identity_state WHERE state = 'pending'
    -- future sources ADD a branch here, never rewrite:
    --   under-review (2) ⟵ pending reattribution             (§5.5 — future)
    --   under-review (2) ⟵ coherence-check demoted link      (§5.2 feedback — future)
)
SELECT patient_id,
       (CASE max(severity) WHEN 2 THEN 'under-review'
                           WHEN 1 THEN 'unconfirmed' END)::text AS trust_state
FROM trust_source
GROUP BY patient_id;
```

**Precedence: under-review (2) > unconfirmed (1) > confirmed (default).** When a chart is *both*
identity-pending *and* has an open dispute, the single displayed state is **under-review**. Rationale
(documented, because a projection must pick one state and this is a safety call):

- *unconfirmed* means "we don't yet know **who** this is" — the data present genuinely belongs to this
  (unnamed) person; the caution is about *absent* history.
- *under-review* means "the **attribution** of events on this chart is actively challenged" (open dispute,
  and later pending-reattribution / coherence failure) — the data *present* may not belong here at all.

The sharper, more actively-dangerous caution is under-review (present data possibly wrong-patient), so it
wins the display. `max(severity)` encodes exactly that, and it is the mechanism future *under-review*
sources plug into (they emit severity 2 and need no CASE change; a source introducing a *new* label adds
both its `SELECT … n` branch **and** its `WHEN n` arm — the one invariant a future editor must hold).

A subject in the default (confirmed) state has **no row** here (it appears in neither source) — the read
coalesces a missing row to `'confirmed'`, keeping the VIEW tiny and the "still-John-Doe" worklist a
trivial `SELECT subject FROM chart_identity_state WHERE state='pending'`.

### Surfacing on the unified read — unchanged

`person_chart_trust` (C3) already composes trust onto C1's `person_chart` by
`LEFT JOIN chart_trust … COALESCE(…, 'confirmed')`. Because C4 only widens what `chart_trust` *emits*
(now also `unconfirmed`), that view needs **no change** — a member chart in the unconfirmed state now
surfaces `trust_state='unconfirmed'` through the same join, for free. (Like `person_chart`, it lists a
subject only once its `patient_chart` row exists; a pending marker arriving before the body still reports
unconfirmed via `chart_trust` queried directly — the authoritative pre-sync safety signal, exactly as C3.)

## Error handling & convergence edge cases (pinned by tests)

- **Pending marks unconfirmed** (`chart_trust` + `person_chart_trust` both report `unconfirmed`).
- **Identify → confirmed** (later-HLC identify wins → no row → confirmed).
- **Out-of-order:** identify@200 lands before pending@100 → row stays `identified`; the older pending does
  not re-open it. (Offline-first convergence, same as C1/C3.)
- **Re-pending lifecycle:** a *newer* pending@300 after identify@200 re-opens unconfirmed — proves the
  overlay is a full lifecycle, not one-way.
- **Precedence / compose:** a chart with *both* an open dispute *and* a pending marker reads
  **under-review**; resolving the dispute leaves it **unconfirmed** (pending still standing); a later
  identify returns it to **confirmed**. This is the C3⊔C4 composition proof.
- **Idempotent re-assert:** `submit_event` dedups by content-address; the trigger upsert is idempotent —
  re-applying the same pending is a no-op and does not create a second `chart_identity_state` row.
- **Pending before chart:** a pending naming a subject with no `patient_chart` row still reports
  unconfirmed via `chart_trust`; `person_chart_trust` lists it only once the chart arrives (parity with
  C3's dispute-before-chart).
- **No identity events:** a standard-registration chart reads `'confirmed'`.
- **Floor rejections**, each a distinct legible exception: missing/invalid `subject`, empty `basis`
  (pending) / empty `method` (identify), empty authored twin.

## Testing (TDD, red-first)

- **Rust unit** (`cairn-event`): pending body shape (`subject`/`basis`, no `method`), identify body shape
  (`subject`/`method`, no `basis`), twin rendering distinguishes pending vs confirmed and includes subject
  + descriptive text.
- **DB integration** (`crates/cairn-node/tests/identity_identify.rs`, gated on `$CAIRN_TEST_PG`,
  serialized via `db::test_serial_guard`): valid pending accepted; pending → `chart_trust` unconfirmed +
  `person_chart_trust.trust_state='unconfirmed'`; identify → confirmed; out-of-order identify wins;
  re-pending re-opens unconfirmed; **dispute+pending precedence → under-review, resolve→unconfirmed,
  identify→confirmed**; idempotent re-assert is one row; pending-before-chart still reports unconfirmed;
  no-identity chart reads `'confirmed'`; floor rejections (bad subject, missing subject, empty basis,
  empty method, missing twin).

Every test drives a specific behaviour before the code exists.

## Out of scope for C4 (deferred, recorded)

- The full §5.4 John-Doe **registration subsystem**: system-generated callsign (never plausible fake
  names; matcher excludes placeholder names from its feature space), clinician-observed evidence
  assertions (estimated age + basis, observed sex, photo, distinguishing marks, belongings, EMS context),
  and the matcher re-run on each new evidence assertion. C4 builds only the trust-state *primitive*.
- The **"prior history now available"** push alert emitted when an identified John Doe is linked to an
  existing chart (§5.4) — a §5.12 notification-economy concern.
- Registration-class partitioning of the search-before-create funnel (§5.3/§5.8) — a workflow/UI concern.
- `reattribute` (§5.5 event-granular strike-through + tiered adjudication) and `repudiate` (alias pool +
  suppressing semantics) — later slices; each *composes* one more source into `chart_trust`.
- Person-level trust aggregation (does an unconfirmed / under-review member taint the person view?) —
  read-surface / API-UI tier, deliberately out of scope (trust attaches to the `patient_id` the marker
  names, not the aggregated `person_id`), same call as C3.
- No spec/ADR change: this implements the settled §5.7 algebra and the §5.7 projection-side contract; it
  refines nothing that needs a new decision.
