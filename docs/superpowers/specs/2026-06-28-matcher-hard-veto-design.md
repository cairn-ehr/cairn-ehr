# Design — §4.4/§5.2 in-DB hard-veto + coherence-check

**Date:** 2026-06-28 · **Spec home:** [identity §5.2](../../spec/identity.md#52-matching-pipeline-safety-asymmetric-false-merge-worse-than-false-split),
[identity §5.13](../../spec/identity.md#513-locale-pluggable-comparators-the-matcher-extension-point),
[demographics §4.4](../../spec/demographics.md#44-identifiers-representation),
[demographics §4.2](../../spec/demographics.md#42-per-field-projection-policy) ·
**Refines:** [ADR-0014](../../spec/decisions/0014-locale-pluggable-matcher-comparators.md),
[ADR-0033](../../spec/decisions/0033-patient-identifier-representation.md) · **No new ADR** (implements settled spec).

## 1. Purpose & the safety boundary

The §5.2 matching pipeline decomposes (see HANDOVER) into three pieces with a hard dependency order:

| Piece | Layer | Status |
|---|---|---|
| **A. Hard-veto + coherence check** — same-system identifier mismatch · verified-DOB clash · verified-sex-at-birth clash | **In-DB, safety-critical (§9)** | **this slice** |
| B. Advisory probabilistic matcher (blocking + Fellegi–Sunter + comparators) → ranked candidate worklist | Python, fit-for-purpose (advisory) | deferred |
| C. Proposal → `link` apply seam | In-DB, safety-critical | deferred — needs §5.7 identity algebra (unbuilt) |

This slice builds **piece A only**: a pure, read-only, in-database function that, given two patient candidates,
returns the closed set of **hard vetoes** between them. It is the safety floor every future matcher proposal (B)
must pass. A hard veto **forces a human decision — never an auto-link, and never an auto-reject** (an auto-reject is
itself a silent false split — [§5.13](../../spec/identity.md#513-locale-pluggable-comparators-the-matcher-extension-point)).

The matcher is **advisory** (it only *proposes*); applying the closed event algebra is authoritative in-DB logic;
the seam between them is the database boundary ([§9.3–§9.4](../../spec/language-substrate.md#93-integration-boundary)).
This function sits on the **safety-critical** side of that seam: it is deterministic, parses nothing culture-specific,
and keeps the §9 safety surface small.

No event-format change, no `submit_event` change — purely additive SQL over projections that already exist
(`patient_identifier` db/010, `patient_demographic` db/011). It reuses `cairn_provenance_rank` (db/011). It does not
write, link, demote, or queue anything.

## 2. Interface

```sql
cairn_match_veto(patient_a uuid, patient_b uuid)
  RETURNS TABLE(veto_kind text, severity text, subject text, detail text)
```

- One row per finding; **empty set = no veto** (clear to auto-link, subject to the matcher's own conservative
  threshold — not this function's concern).
- `veto_kind ∈ {identifier, dob, sex-at-birth}`.
- `severity ∈ {hard_veto, degrade_hold}` (§3).
- `subject` — the identifier `system` namespace, or the demographic field name.
- `detail` — a legible, human-readable reason (reviewer-legibility, house rule).

Scalar convenience:

```sql
cairn_has_hard_veto(patient_a uuid, patient_b uuid) RETURNS boolean
```

= "any `hard_veto`-severity row exists" — the matcher's auto-link gate. (A `degrade_hold` alone does **not** trip
this gate, but the caller still surfaces the pair to a human; see §3.)

**Invariants:** symmetric (`cairn_match_veto(a,b)` ≡ `cairn_match_veto(b,a)` as a set), deterministic, and
`a = b` → empty (a patient never vetoes itself).

## 3. The two verdict levels (the honest-degradation nuance)

§4.4 is precise: a node fires a *true* hard veto only on a basis it can trust.

- **HARD_VETO** — a trustworthy clash. Blocks auto-link **and** (once linking exists, piece C) can demote an
  *existing* link to under-review. Triggers:
  - identifier: same `system`, both sides carry a `normalized` form, and they differ;
  - dob / sex-at-birth: both winners verified, same precision (dob), values differ.
- **DEGRADE_HOLD** — an *untrustworthy basis* for a clash. Blocks auto-link and surfaces to a human, but **cannot**
  demote an existing link. Trigger:
  - identifier: same `system`, but `normalized` is absent on at least one side (a profile-less node), and the raw
    `value` strings differ — the difference may be pure formatting noise (`9434765919` vs `943 476 5919`), so the
    node "holds for human review rather than firing the veto or demoting an existing link" ([§4.4](../../spec/demographics.md#44-identifiers-representation)).

Both keep us on the safe side of the false-merge ≫ false-split asymmetry: neither ever auto-rejects.

## 4. Composition — pure helpers (house rule: pure, reusable functions)

- **`cairn_identifier_veto(a, b)` → TABLE** — the §4.4 logic over `patient_identifier`. A patient may legitimately
  hold **multiple** identifiers in one `system` (the projection PK is `(patient, system, match_key)`), so the
  comparison is **set-based per system**, not value-to-value. Consider only systems **present on both** patients,
  **excluding the `unknown` sentinel** (which never participates in a veto). For each such shared `system`, a clash
  exists only when the two patients share **no common** identifier — sharing even one value is evidence *for* a link,
  never a veto. Concretely, per shared `system`:
  - the two sides have **no common `normalized` value** *and* both sides have at least one non-null `normalized`
    → `HARD_VETO` (a trustworthy disjoint set);
  - the two sides have **no common `value` string** *and* at least one side has a null `normalized` (profile-less)
    → `DEGRADE_HOLD` (the difference may be formatting noise — cannot be trusted as a true mismatch);
  - the sides share any `normalized` (or, when degraded, any `value`) → **no finding** (positive evidence, not a
    mismatch — the `9434765919` vs `943 476 5919` case shares one `normalized`).
  - Note: `patient_identifier.match_key = COALESCE(normalized, value)`; the veto reads the explicit `normalized` and
    `value` columns (not `match_key`) to make the trustworthy-vs-degraded distinction.
- **`cairn_field_clash(a, b, field)` → TABLE** — over `patient_demographic` (one winner row per `(patient, field)`).
  Fire `HARD_VETO` iff: both patients have a winner for `field`, **both** winners are verified
  (`provenance_rank ≥ 60` — `document-verified` or `fact-proven`; the "verified value locks" property of the db/011
  projection means a node's winner already reflects its verified value when one exists), the winners carry the **same**
  `facets.precision`, and the `value` strings differ. Otherwise no finding. Reused verbatim for `dob` and
  `sex-at-birth`.
  - For `sex-at-birth`, `precision` is absent on both sides (no precision facet) — "same precision" is trivially
    satisfied (both null), so the rule reduces to "both verified + values differ".
- **`cairn_match_veto(a, b)`** — `UNION ALL` of `cairn_identifier_veto` + `cairn_field_clash(_, _, 'dob')` +
  `cairn_field_clash(_, _, 'sex-at-birth')`.

## 5. Why the DOB clash is precision-gated and parses no dates

`patient_demographic.value` for `dob` is an **open string** (`1980`, `1980-03-15`, `circa 1980`) and the floor
never parses it ([db/011](../../db/011_demographics_fields.sql)). Date parsing is locale-specific, profile-dependent
logic that belongs in the advisory Python matcher (piece B), **not** the safety-critical floor. So the in-DB veto
decides a DOB clash without parsing:

- **Same precision + verified both + value differs → HARD_VETO** (e.g. day-precision `1980-03-15` vs `1980-03-16`).
- **Different precision → no finding.** `1980` (year) vs `1980-03-15` (day) are *consistent* (a coarsening); the floor
  cannot prove otherwise without parsing, and [principle 4](../../spec/index.md#founding-principles-the-lens-for-every-decision)
  says imprecision is partial agreement, never disagreement. The advisory matcher does the compatible-vs-contradictory
  judgment later.

**Known conservative residual:** same precision but different *format/coding* (`15/03/1980` vs `1980-03-15`, or sex
`M` vs `male`) fires a false HARD_VETO. This is on the **safe side** — it only routes the pair to human review, never
auto-rejects, never auto-merges — is rare within one node's own data (consistent entry format), and is resolved by the
advisory matcher's locale comparators. Documented, not engineered away in the floor.

## 6. Deceased-status conflict — explicitly deferred

The §5.13 closed veto set includes deceased-status conflict, but **no deceased field is projected** anywhere
(`patient_demographic` projects only `dob` and `sex-at-birth`). It is recorded as a commented stub in
`db/016_match_veto.sql` and a HANDOVER note — **not silently dropped** — and slots into `cairn_match_veto` as a fourth
branch once that field is projected.

## 7. Placement, grants, schema

- New file `db/016_match_veto.sql`; bump the `cairn-node` `SCHEMA` array 14 → 15.
- Functions are `SECURITY INVOKER`, read-only; reads `patient_identifier` + `patient_demographic`, already
  `GRANT SELECT`-ed to `cairn_agent`. `GRANT EXECUTE` on the public entry points (`cairn_match_veto`,
  `cairn_has_hard_veto`) to the matcher role.
- No `cairn-event` change, no `submit_event` change, no new projection table.

## 8. Tests (TDD on PG18 + cairn_pgx — the demographics-slice pattern)

cairn-node integration tests that submit demographic events through the real door, then assert verdicts:

1. **No veto** — two patients, no shared identifier system, compatible/absent DOB → empty set.
2. **Identifier HARD_VETO** — same `system`, both `normalized` present, differ.
3. **Identifier DEGRADE_HOLD** — same `system`, `normalized` absent (profile-less), `value`s differ.
4. **Identifier same-normalized = no veto** — same `system`, `value`s formatted differently but `normalized` equal
   (the `9434765919 == 943 476 5919` case) → empty.
4b. **Multi-valued shared system, one common value = no veto** — patient A holds `{X, Y}`, B holds `{Y}` in one
   `system` → they share `Y` → empty (set-disjointness, not value-to-value).
5. **`unknown` system never vetoes** — same `system: unknown`, different values → empty.
6. **DOB HARD_VETO** — both verified, same precision, differ.
7. **DOB no-veto (different precision)** — year vs day → empty.
8. **DOB no-veto (not both verified)** — one verified, one patient-stated → empty.
9. **sex-at-birth HARD_VETO** — both verified, differ.
10. **Multiple findings** — identifier clash *and* DOB clash in one call → two rows.
11. **Symmetry** — `cairn_match_veto(a,b)` ≡ `cairn_match_veto(b,a)`.
12. **Scalar gate** — `cairn_has_hard_veto` true on a HARD_VETO, false on a lone DEGRADE_HOLD / no finding.

## 9. Out of scope (clearly-scoped follow-ons)

- The **advisory probabilistic matcher** (piece B — Python; blocking, Fellegi–Sunter, comparator API, weight config).
- The **proposal → `link` apply seam** and the **coherence-check demotion trigger** (piece C — needs the §5.7
  identity event algebra).
- A **candidate / possible-duplicate worklist table** (a future destination for B's proposals).
- The **deceased-status** veto branch (needs a projected deceased field).
