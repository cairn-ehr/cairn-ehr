# Design — two §5.4 finishers: node-local John-Doe ordinal + `--observed-year`

**Date:** 2026-07-11 · **Scope:** PR #1 of the §5.4-finishers thread · **Spec/ADR impact:** none
(no new event type, no floor/wire/SCHEMA change, no ADR, no spec bump)

## Context

Three small self-contained §5.4 finishers are open (HANDOVER "Open threads"):

1. a **readable callsign suffix** (HANDOVER worded it "partition-safe per-day count");
2. an **`--observed-year` CLI override** for clinician-observed evidence;
3. **`identify`→optional-link** wired into one resolution flow.

Finisher 3 is materially larger — `identify` has event builders + the `db/024` floor + test
helpers but **no node authoring function and no CLI**, and its optional `link` trips the `db/005`
attestation gate, for which **no CLI mints a human attestation token yet**. It therefore gets its own
brainstorm→spec→plan (a later PR). **This spec covers finishers 1 and 2 only.**

### The callsign tension (why finisher 1 is not "a per-day counter")

The existing front door derives the callsign's disambiguating suffix from the freshly-minted patient
UUID (last 8 hex chars), and [`crates/cairn-node/src/john_doe.rs`](../../../crates/cairn-node/src/john_doe.rs)
**deliberately rejected a per-day counter**:

> "This is what keeps two John Does registered at the same site on the same day distinct *without a
> per-day counter that a partition would race on*."

A naive per-day sequential counter is **not partition-safe**: two nodes registering John Does while
partitioned from each other both mint `Unknown-ED-north-2026-07-11-001` for **different** patients.
Identical bedside headers on two live unidentified charts is exactly the wrong-chart hazard
(paper-parity, principle 3) the UUID suffix was chosen to avoid.

**Resolution (decided in brainstorming):** do **not** touch the callsign identity string. Keep the
UUID suffix (partition-safety unchanged) and add a **separate, node-local, non-replicated friendly
ordinal** as a bedside display aid — "this node's John Doe #47" — that is never signed, never on the
wire, and never part of any identity. It is a display convenience in the same family as the node-local
surrogate keys (ADR-0031) and the backup-health sidecar. The ordinal is **all-time per-node** (no daily
reset) — the simplest form that delivers the value (a short, stable, human-sayable handle) with no
timezone semantics to get wrong.

## Finisher 1 — node-local all-time John-Doe ordinal

### Mechanism: a read-only projection VIEW

A new migration `db/030_john_doe_local_ordinal.sql` adds one VIEW. It reads the existing `event_log`
(no new table, no write-path change):

```sql
CREATE OR REPLACE VIEW john_doe_local_ordinal AS
SELECT patient_id,
       body->>'value' AS callsign,
       row_number() OVER (ORDER BY hlc_wall, hlc_counter, content_address) AS ordinal
FROM event_log
WHERE event_type = 'demographic.field.asserted'
  AND body->>'field' = 'name'
  AND body->'facets'->>'use' = 'callsign'
  AND body->>'provenance' = 'system:john-doe-registration'
  AND node_origin = (SELECT encode(node_id,'hex') FROM local_node WHERE id);
```

Design properties:

- **Node-local by construction.** The `node_origin = <local node hex>` filter means each node numbers
  only the John Does *it* first recorded. The callsign events themselves replicate everywhere (they are
  ordinary clinical events), but a John Doe registered on another node has a different `node_origin` and
  never perturbs this node's sequence. On an un-provisioned node the subquery is NULL, so the VIEW is
  empty — honest.
- **Selects exactly John-Doe registrations.** The four body predicates match the callsign name authored
  by `register_john_doe` (`field=name`, `facets.use=callsign`, `provenance=system:john-doe-registration`)
  — not an ordinary name, not the pending marker.
- **Stable & deterministic.** Ordered by `(hlc_wall, hlc_counter, content_address)` — the same
  collation-free tiebreak spine as #115/#69 (`content_address` is a BYTEA multihash, byte-ordered,
  identical on every node). On a single authoring node the HLC is monotonic, so ranks never renumber;
  `content_address` breaks any degenerate tie deterministically.
- **No new wire / floor / event / SCHEMA surface.** Pure read-side projection. Because the callsign
  identity string is untouched, this cannot regress partition-safety.

### Surfacing it at registration

`register_john_doe` currently returns `(Uuid, String)` = `(patient_id, callsign)`. It gains a query of
`john_doe_local_ordinal` for the just-registered `patient_id`, **inside its existing transaction** (the
callsign row it just inserted is visible in-txn, so the ordinal already counts it), and returns
`(Uuid, String, i64)` = `(patient_id, callsign, ordinal)`.

The CLI (`Cmd::RegisterJohnDoe` handler) prints:

```
registered John Doe <uuid>
callsign <Unknown-…>
local ref: John Doe #47 (this node)
```

The VIEW is also independently queryable for any later lookup/worklist use (no dedicated lookup CLI in
this slice — the VIEW is the deliverable; a lookup command can be added later if a real workflow asks).

### TDD

- **DB-gated** (`crates/cairn-node/tests/john_doe.rs` or a sibling): register two John Does on the node
  → ordinals `1` then `2`; assert `register_john_doe` returns the same ordinal the VIEW reports; inject a
  callsign-shaped `event_log` row with a **foreign** `node_origin` → this node's ordinals are unchanged
  (node-local proof); a non-callsign name event on this node does not count.

## Finisher 2 — `--observed-year` override

### Mechanism

Today the observed-evidence handler ([`main.rs`](../../../crates/cairn-node/src/main.rs) `AssertObservedEvidence`)
always derives `observed_year` from the DB `current_date`. This finisher lets a clinician recording
evidence about a past observation state the year the estimate was made.

- New optional CLI flag `#[arg(long)] observed_year: Option<i32>` on `Cmd::AssertObservedEvidence`.
  Omitted ⇒ current behaviour (today's year).
- A **pure, unit-tested** helper in `crates/cairn-node/src/evidence.rs`:

  ```rust
  /// Resolve the year an age estimate was observed. `None` ⇒ default to `current_year`
  /// (today). A supplied year must be plausible: not in the future (you cannot have
  /// observed a patient in a year that has not happened) and not absurdly historical
  /// (keeps the `observed_year − age` birth-range arithmetic sane). Honest reject rather
  /// than compute a garbage range (principle 4).
  pub fn resolve_observed_year(provided: Option<i32>, current_year: i32) -> anyhow::Result<i32>
  ```

  Bounds: **`1900 ≤ y ≤ current_year`**.
- The resolved year flows into the existing `birth_year_range_from_age(age, tol, observed_year)`. The
  handler resolves `current_year` from the DB (as it already does), passes it plus the CLI flag to the
  helper, and uses the result.

### Deliberate scope boundary

`--observed-year` parameterizes only the **computed DOB range** (`observed_year − age ± tol`). It does
**not** set the event's `t_effective`. Backdating the effective time is a separate bitemporal concern
(§3.6, ADR-0003) and is out of scope for this finisher — the evidence event keeps `t_effective: None`.

### TDD

- **Pure** (`evidence.rs` unit tests): `resolve_observed_year(None, 2026) == 2026`;
  `resolve_observed_year(Some(2010), 2026) == 2010`; `Some(2027)` (future) ⇒ Err;
  `Some(1899)` ⇒ Err; boundary `Some(1900)` and `Some(2026)` ⇒ Ok.
- **DB-gated**: assert observed evidence with `observed_year = 2000`, age 40 ± 5 → the projected
  `dob` value is the year range `1955/1965` (`2000 − 40 = 1960`, ±5).

## Non-goals / boundaries (both finishers)

- No new event type; no change to `submit_event`/`apply_remote_event` floors; no wire/SCHEMA change.
- No ADR, no spec version bump — finisher 1 is a node-local display projection; finisher 2 is a
  CLI-surface + input-validation refinement of the existing additive observed-evidence path.
- Finisher 3 (`identify`→optional-link) is explicitly deferred to its own spec/PR.

## Files touched

- `db/030_john_doe_local_ordinal.sql` — new VIEW (finisher 1).
- `crates/cairn-node/src/db.rs` — register `db/030` in the migration list.
- `crates/cairn-node/src/john_doe.rs` — `register_john_doe` returns the ordinal (query the VIEW in-txn).
- `crates/cairn-node/src/evidence.rs` — pure `resolve_observed_year` + tests; thread the resolved year.
- `crates/cairn-node/src/main.rs` — `--observed-year` flag; print the `local ref` line at registration.
- `crates/cairn-node/tests/…` — DB-gated coverage for both.

## Governing-principle check

- **Principle 1 (append-only).** No mutation; the ordinal is derived read-side from the immutable log.
- **Principle 2 (identity is a claim).** The callsign identity string is untouched; the ordinal is a
  display aid, never an identity or a merge key.
- **Principle 3 (paper-parity).** A short sayable "John Doe #47" restores the paper affordance of a
  numbered unidentified folder without reintroducing the partition-unsafe shared counter.
- **Principle 4 (acknowledged uncertainty).** `--observed-year` refuses implausible input rather than
  fabricating a birth-year range; the observed year remains an honest estimate parameter.
- **§9 defect blast radius.** Finisher 1 is a read-only projection; finisher 2 validates input to an
  advisory-tier evidence path. Neither touches the safety-critical write floor.
