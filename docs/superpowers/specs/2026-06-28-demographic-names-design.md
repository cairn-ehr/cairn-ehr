# Design — Demographic names: retained set + display-winner (slice 3)

**Date:** 2026-06-28 · **Spec home:** demographics §4.2 · **ADRs:** new **0036** (names recency-first display), [0034](../../spec/decisions/0034-demographic-legibility-twin.md) (twin), [0010](../../spec/decisions/0010-additive-vs-suppressing-classification.md) (additive registry), [0012](../../spec/decisions/0012-schema-evolution-event-format-and-legibility-across-time.md) (additive evolution / legibility) · **Substrate:** `cairn-node`

## Purpose

Slice 3 of the demographics subsystem. Slice 1 ([identifiers](2026-06-27-demographic-identifier-assertion-design.md)) built the **pure set-union** projection; slice 2 ([DOB + sex-at-birth](2026-06-27-demographic-dob-sex-at-birth-design.md)) built the **winner-only** provenance-precedence projection. Names is the first field that needs **both at once** (§4.2 line 15: *"All retained; display = highest-provenance recent legal name"*):

- a **retained set** — every name kept as matching evidence (legal, maiden, alias, transliteration), like `patient_identifier`;
- a **single display-winner** selected from that set, like `patient_demographic` but chosen, not collapsed.

This is the genuinely new projection shape neither prior slice has. The slice deliberately keeps the *value* simple (a scalar display string) so the only new mechanic is the projection.

### Scope boundaries (what this slice is NOT)

- **Scalar display string only.** A name's value is one authored display string (`"田中 太郎"`, a mononym, a patronymic — culture-neutral as-authored). **Structured parts** (given/family + a content-addressed locale profile, the §4.3 address pattern) are a **later additive slice**, not this one.
- **No event-type or floor change.** A name is `demographic.field.asserted` with `field="name"` — the slice-2 generic event + generic floor already accept it (see below). Nothing in db/005/010/011 or `submit_event` is touched.
- **Matching is out of scope.** "Weak evidence" name comparison lives in the identity §5.2 matcher (later). This slice carries names and projects a current display; it computes no link evidence.
- **No value validation.** The floor never parses or normalises a name. Transliteration/script comparison is advisory matcher work (above the floor).
- **No CLI verb.** Authoring is exercised by the test harness (and the future product CLI), as in slices 1–2.
- **No preferred-name / "a.k.a." display logic.** The projection's display-winner is the **legal-preferred reference point**. Surfacing a patient's preferred/chosen name as an "a.k.a." over the legal name is **UI soft-policy above the floor** (principle 12); the retained set already carries the alias/preferred members the UI needs.

## The headline: the slice-2 spine already accepts names

A name event reuses **the generic `demographic.field.asserted` event verbatim**:

- **Floor:** `cairn_check_demographic_field` already requires `field`/`provenance`/`value` non-empty and only adds *extra* checks for `field='dob'`. `field='name'` passes the generic checks unchanged — **no floor change.**
- **Cross-field isolation:** the slice-2 projection trigger gates `IF fld NOT IN ('dob','sex-at-birth') RETURN NULL`, so a name event **never touches `patient_demographic`**. The federation-forward "unknown field carried-not-projected" design (ADR-0012) does exactly its job — a node that never learns the names projection still stores+legibilises name events.

So the entire slice is **additive**: a Rust builder, a new `db/012` projection (table + trigger + view), tests, an ADR, and a §4.2 wording refinement.

**Rejected alternative:** a dedicated `demographic.name.asserted` event type. Cleaner trigger `WHEN` clause, but costs a new event-type registration + a floor-dispatch branch — needless surface against the generic-field design built for exactly this.

## The new mechanic: retained set + display-winner-as-a-view

Two objects in `db/012_demographics_names.sql`.

### (a) `patient_name` — the retained set (evidence)

One row per distinct `(patient_id, use_key, value)` name the patient has ever been asserted to have.

```
PRIMARY KEY (patient_id, use_key, value)
  use_key := coalesce(NULLIF(trim(facets->>'use'), ''), 'unspecified')   -- mirrors patient_identifier.match_key
columns:
  value              TEXT    NOT NULL    -- the authored display string
  use_raw            TEXT                -- the original use facet (NULL when absent)
  provenance         TEXT    NOT NULL
  provenance_rank    INT     NOT NULL    -- cached cairn_provenance_rank(provenance)
  last_hlc_wall      BIGINT  NOT NULL
  last_hlc_count     INTEGER NOT NULL
  asserted_origin    TEXT    NOT NULL
  updated_at         TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
```

The trigger keeps, per member, the **most-recent assertion** as its representative:

```sql
ON CONFLICT (patient_id, use_key, value) DO UPDATE SET … 
WHERE (EXCLUDED.last_hlc_wall, EXCLUDED.last_hlc_count,
       EXCLUDED.provenance_rank, EXCLUDED.asserted_origin)
    > (pn.last_hlc_wall, pn.last_hlc_count,
       pn.provenance_rank, pn.asserted_origin);
```

The tuple is **recency-first** `(hlc_wall, hlc_count, provenance_rank, origin)` — matching the display rule — and is a deterministic, apply-order-independent function of the member's assertion set, so every node converges to the same representative. (Tradeoff, noted: a member's *shown* provenance is that of its most-recent assertion, not necessarily its strongest; full provenance history remains in `event_log`. Acceptable — this is a current-display + matching projection, not the evidence-of-record.)

`provenance_rank` reuses the existing `cairn_provenance_rank()` (db/011) — no second ladder.

### (b) `patient_name_current` — the display-winner, as a VIEW (no stored pointer)

```sql
CREATE VIEW patient_name_current AS
SELECT DISTINCT ON (patient_id) *
FROM patient_name
ORDER BY patient_id,
         (use_key = 'legal') DESC,                   -- tier 1: prefer legal
         last_hlc_wall DESC, last_hlc_count DESC,     -- recency-first
         provenance_rank DESC, asserted_origin DESC;  -- provenance/origin tiebreak
```

That single `ORDER BY` is the whole chosen rule:

- a `legal` member always outranks any non-legal (a 2010 legal beats a 2024 alias);
- newest-legal wins among legals (**recency beats provenance for names** — the key divergence from DOB, which provenance-locks);
- when **no** legal member exists, the newest any-use member wins — the **unidentified-patient fallback** (a triage alias / "Unknown Male ~40" still displays; paper-parity holds).

The winner is a **pure deterministic function of the set** — no winner-pointer to maintain or drift. Name sets are tiny and indexed by `patient_id`, so the per-read `DISTINCT ON` is free.

`GRANT SELECT` on both `patient_name` and `patient_name_current` to `cairn_agent`.

## Rust builders (`cairn-event/src/demographics.rs`, pure)

```rust
/// One §4.2 name assertion. `value` is the authored display string (verbatim).
/// `use_` is the recommended-but-open category (legal/maiden/alias/transliteration/…),
/// omitted from the body when None. Structured parts are a later additive slice.
pub fn name_assertion_body(value: &str, use_: Option<&str>, provenance: &str) -> Value;
    // → demographic_field_body("name", value, facets {"use": use_} when Some, provenance)

/// §4.5 legibility twin. Matches the spec example "Name (legal): 田中 太郎":
/// use in parens when present; falls back to provenance when use is absent
/// ("Name (patient-stated): Mary") so the parenthetical is never empty.
pub fn render_name_twin(value: &str, use_: Option<&str>, provenance: &str) -> String;
```

## Tests (TDD, red-first)

**`cairn-event` units:** body shape (`field`/`value`/`facets.use`/`provenance`); `use` omitted when `None`; twin with-use and without-use.

**`cairn-node` integration (PG18 + cairn_pgx):**

1. **Happy path** — assert a legal name → present in `patient_name`; `patient_name_current` shows it.
2. **Retained set** — legal + maiden + alias all asserted → all three rows kept; current = legal.
3. **Recency-first within legal** (the DOB divergence) — Smith `document-verified`/old-HLC + Jones `patient-stated`/new-HLC, both legal → current = **Jones**.
4. **No-legal fallback** — only an alias asserted → current = the alias.
5. **Legal takes over fallback** — alias + a legal (newer or older) → current = the legal.
6. **Set-union idempotency** — re-apply the same name event → still one member, projection unchanged.
7. **Cross-field isolation** — a `name` event creates no `patient_demographic` row; a `dob`/`sex-at-birth` event creates no `patient_name` row.
8. **Floor reuse** — a `name` event with empty `value` is rejected by the existing generic floor; authored-twin enforcement still applies.

## Spec + decision log

- **New ADR-0036** — *demographic name display is recency-first*: captures **why names diverge from DOB's provenance-lock** (names are a volatile, legitimately-changing identity field; provenance-first pins stale married names and deadnames — a dignity *and* safety failure). Records the two-tier rule (legal-preferred, recency-first; any-use fallback) and the legal-as-reference-point / UI-"a.k.a." layering. Immutable home of the *why* so it is not reverted to provenance-first.
- **§4.2 refinement** — the names row reworded from *"display = highest-provenance recent legal name"* to *"all retained; display = **most-recent legal name (recency-first; provenance and origin break ties)**, falling back to the most-recent name of any `use` when no legal name exists"*, plus a one-line volatility-rationale note pointing at ADR-0036.
- **HANDOVER / ROADMAP** updated at session end (Phase 4 names → done; next-slice menu trimmed).

## File-size / house-rules check

`db/012` is a single focused migration (~120 lines, under the 500-line target). `demographics.rs` gains two small pure functions (well under 500 lines total). Every non-trivial function carries junior-readable intent comments. No new dependency (no licensing surface). TDD throughout; full workspace suite + clippy must be green before commit.
