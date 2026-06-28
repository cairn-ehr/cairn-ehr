# Design — Demographics slice 5: §4.3 address (the three-facet value)

- **Date:** 2026-06-28
- **Status:** Approved (brainstorm) → ready for implementation plan
- **Spec home:** [demographics §4.3](../../spec/demographics.md#43-address-the-three-facet-value),
  [ADR-0032](../../spec/decisions/0032-culture-neutral-address-representation.md) (representation, locked)
- **New ADR:** 0038 (per-use recency-first address winner; refines 0032, follows
  [ADR-0036](../../spec/decisions/0036-demographic-name-display-recency-first.md))
- **Slice of:** the demographics subsystem on `cairn-node` (slices 1–4 landed: identifiers, DOB +
  sex-at-birth, names, administrative-sex + gender-identity).

## Purpose

Graduate the §4.3 culture-neutral address representation into product code on the existing demographics
spine. The **representation** is already locked by ADR-0032 (three facets: mandatory `display`, optional
`geo`, optional `structured`); this slice adds the *projection* (how addresses are stored, deduplicated,
and surfaced as "current") and the pure `cairn-event` builders. It settles the one open per-field policy
question ADR-0032 deliberately left out — the **display-winner** — and resolves a stale line in the §4.3
summary table.

## The one design decision: per-use recency-first

ADR-0032 fixed address *representation* but called the thin "recency wins" treatment a *matching*
statement, not a projection one — leaving the winner policy open. The §4.3 summary table (written
2026-06-27) says the per-use current address is *"highest-provenance most-recent"* (provenance-first,
DOB-style lock). That predates the names slice ([ADR-0036](../../spec/decisions/0036-demographic-name-display-recency-first.md),
2026-06-28), which established that **volatile, legitimately-changing fields must be recency-first** — or a
stale verified value pins over the current truth.

**Address is the archetypal volatile field — people move.** A document-verified address from 2019 pinning
over a patient-stated "I moved last month" is the stale-married-name / deadname failure ADR-0036 rejected,
applied to *where you would send an ambulance or a letter*. Clinically the latest claim about where the
patient physically is must win, even when unverified.

**Decision:** per `use`, the current address is **recency-first** (newest assertion wins;
`provenance_rank` then `asserted_origin` break exact-recency ties). This mirrors names exactly and warrants
ADR-0038 + a §4.3 prose/table fix, just as names got ADR-0036. All addresses are retained as evidence
regardless; provenance still feeds the later §5.2 matcher.

**One current address per `use`.** Residential, postal, and work are independently "current" — the winner
VIEW selects one row per `(patient, use)`. No legal-tier preference (unlike names), no cross-use fallback;
the UI surfaces past/other addresses from the retained set when needed.

## Event shape (no new event type)

Reuses the slice-2 generic `demographic.field.asserted` event with `field:"address"`, exactly as names
reused it with `field:"name"`. **`value` carries the mandatory `display` string** (the value-core), so the
generic floor's existing non-empty-`value` check already enforces "display non-empty" with no new code.

```
{ field:"address",
  provenance:"…",                                   // §4.1 ladder term (required, value-open)
  value:"<display>",                                // mandatory — the complete human-readable address
  facets:{ use?:"residential",                      // recommended-but-open use vocabulary
           geo?:{ lat, lon, accuracy_m, basis },    // optional, precision-aware (principle 4 in space)
           structured?:{ profile:"namespace@hash",  // content-addressed locale bundle reference
                         parts:{ … } } } }          // open bag, values opaque text to the core
```

## Component 1 — floor (`db/014_demographics_address.sql`)

Extends `cairn_check_demographic_field` via `CREATE OR REPLACE`, **carrying forward the existing `dob`
branch** (same supersession pattern db/013 used for the projection; latest-loaded wins). Adds a
`field='address'` branch — **structural only, culture-neutral, never holds a profile, never rejects on
validation** (principle 12 / 4):

- `structured` present ⇒ `structured.profile` is a non-empty string (the §4.3 `structured ⇒ profile`
  invariant); every `structured.parts` value is text (opaque to the core; no part-name vocabulary).
- `geo` present ⇒ `lat` and `lon` are numbers, `accuracy_m` is a **non-negative** number, `basis` is
  non-empty text. *(Type/shape only — a negative radius is structurally meaningless, not "uncertain".)*

**Explicitly above the floor (advisory, deferred):** lat/lon range bounds, `display == formatter(parts)`
cross-facet consistency, profile re-derivation. Keeping these out of the floor is what keeps it
culture-neutral and lets a profile-less node accept any address and degrade honestly.

`display` non-empty needs **no new check** — it is the generic `value` check already in the function.

## Component 2 — projection (`db/014`, retained set + per-use VIEW)

Mirrors the names slice (db/012).

**`patient_address`** — retained set, `PRIMARY KEY (patient_id, use_key, display)`:
- `use_key` folded identically to names: `lower(coalesce(NULLIF(trim(use),''),'unspecified') COLLATE "C")`
  — `use` is open vocabulary, so casing variants collapse to one member and the fold stays convergent
  across the fleet (a locale `lower()` is collation-dependent); `use_raw` keeps the authored casing.
- `display` is the member discriminant (the value-core), exactly as `value` is for names. Two assertions
  of the same `display` under the same `use` are the **same member**; `geo`/`structured` travel as the
  member's representative facets, with the **most-recent** assertion as representative.
- Columns: `geo jsonb`, `structured jsonb`, `provenance`, `provenance_rank` (cached
  `cairn_provenance_rank`), `last_hlc_wall/count`, `asserted_origin`, `updated_at`.
- `patient_address_apply()` trigger on `demographic.field.asserted`, gated to `field='address'`
  (ignores dob/sex/name/unknown). `ON CONFLICT … DO UPDATE … WHERE` advances only on a strictly-greater
  recency tuple → set-union idempotent, apply-order-independent, convergent.

**`patient_address_current`** — `DISTINCT ON (patient_id, use_key)` VIEW, the winner rule *is* the
`ORDER BY`: `patient_id, use_key, last_hlc_wall DESC, last_hlc_count DESC, provenance_rank DESC,
asserted_origin DESC`. Recency-first within each use; deterministic tiebreaks → one current address per
use, identical on every node.

`GRANT SELECT ON patient_address, patient_address_current TO cairn_agent`.

Register `("014_demographics_address", …)` in the `crates/cairn-node/src/db.rs` SCHEMA array.

## Component 3 — `cairn-event::demographics` (pure, unit-tested)

```rust
pub struct Geo<'a> { lat: f64, lon: f64, accuracy_m: f64, basis: &'a str }
pub struct StructuredAddress<'a> { profile: &'a str, parts: serde_json::Value } // parts = json object
pub struct AddressAssertion<'a> {
    display: &'a str, provenance: &'a str,
    use_: Option<&'a str>, geo: Option<Geo<'a>>, structured: Option<StructuredAddress<'a>>,
}
pub fn address_assertion_body(a: &AddressAssertion) -> Value      // field=address, value=display, facets{…}
pub fn render_address_twin(a: &AddressAssertion) -> String        // "Address (<use|provenance>): <display>"
```

Optional facets are **omitted entirely when absent, never serialized null** (the established rule, so the
floor's key-presence checks see exactly what was asserted). The twin mirrors `render_name_twin`: `use` in
the parens when present, else provenance. `geo`/`structured` do not enter the twin — `display` is by
definition the complete human-readable address.

## Component 4 — spec / ADR / currency

- **ADR-0038** — per-use recency-first address winner. Refines ADR-0032 (representation locked, winner was
  open); follows ADR-0036's volatile-field logic. Records the retained-set + per-use VIEW, and the deferred
  explicit supersession.
- **§4.3 demographics** — prose note on the winner policy; fix the line-20 summary-table cell
  ("highest-provenance most-recent" → "per use: most-recent within the use; full history retained").
- Spec version 0.38 → 0.39; ADR index row; HANDOVER.md + ROADMAP.md currency.

## Test plan (TDD — failing test first)

**`cairn-event` unit** (in `demographics.rs`): body carries field/value(display)/provenance + all facets
when present; body omits absent `use`/`geo`/`structured` (never null); geo sub-object shape; twin uses
`use` when present else provenance; structured `parts` passthrough.

**`cairn-node` integration** (`tests/demographics_address.rs`, PG18 + cairn_pgx):
1. Happy path — display-only address projects; current view shows it.
2. `geo` + `structured` carried through and stored in the retained set.
3. Per-use independence — a residential and a postal address are *both* current simultaneously.
4. Recency-beats-provenance within a use — a newer patient-stated address displaces an older
   document-verified one in the current view; both remain in the retained set.
5. Set-union idempotency/convergence — re-asserting (and out-of-order apply) yields one stable member;
   winner identical regardless of apply order.
6. Floor rejections (each isolated, triple-gated: error + empty `event_log` + empty projection):
   `structured` without `profile`; a non-text `parts` value; malformed `geo` (non-number lat,
   negative `accuracy_m`, empty `basis`).
7. Regression — slices 1–4 stay green; an unknown field and a legacy event are unaffected.

## Deferred (YAGNI — out of scope for this slice)

- Explicit address supersession / unlink events — the append-only retained set + recency handles "moved"
  (names deferred this identically).
- §5.2 matcher comparator using the address `profile` (its own later slice).
- Advisory validators: lat/lon range bounds, `display == formatter(parts)` drift detection, profile
  re-derivation (the advisory layer, not the floor).
- Reverse geocoding (a *new* `geocoded_from_text` assertion; UI/advisory concern, never a mutation).

## Files

- `db/014_demographics_address.sql` — new (floor extension + projection).
- `crates/cairn-event/src/demographics.rs` — append builders + unit tests.
- `crates/cairn-node/src/db.rs` — register migration 014.
- `crates/cairn-node/tests/demographics_address.rs` — new integration suite.
- `docs/spec/decisions/0038-demographic-address-winner-per-use-recency.md` — new ADR.
- `docs/spec/demographics.md`, `docs/spec/index.md`, `docs/HANDOVER.md`, `docs/ROADMAP.md` — currency.
