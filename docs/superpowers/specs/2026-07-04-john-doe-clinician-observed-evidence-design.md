# Design — §5.4 clinician-observed evidence: estimated-age range + observed sex (full loop)

**Date:** 2026-07-04
**Spec home:** §5.4 (Unidentified registration — John Doe), lines 42–43.
**Status:** design approved; ready for implementation plan.

## 1. What this slice is

The next sub-slice of the §5.4 John-Doe subsystem, after slice A (callsign minting +
matcher placeholder exclusion, built 2026-07-03). Spec §5.4 line 42:

> Identity evidence captured as **clinician-observed assertions**: estimated age with
> basis, observed sex, photo, distinguishing marks, belongings, EMS pickup context —
> honest data, full matcher features.

Line 43: *"matcher re-runs on every new evidence assertion."*

This slice implements the two evidence kinds that **reuse the existing demographic fields**:
**estimated age → dob** and **observed sex**. It is a **full loop**: the evidence is not
just recorded honestly, it becomes a **positive match signal** in the advisory matcher, so a
John Doe's estimated age can help find their prior chart.

Explicitly **out of scope** (deferred, recorded): photo / distinguishing marks / belongings /
EMS-context evidence (need a new field home, and photo pulls in the attachment tier — a larger
separate slice); the "prior history now available" push-alert on link (needs a notification
tier, §5.12, not built); fuzzy age-range comparison beyond overlap.

## 2. The load-bearing finding: the floor already carries it

`db/011_demographics_fields.sql` already:
- accepts a generic `demographic.field.asserted` event with `value` + `provenance` + optional
  `facets`;
- for the `dob` field, **requires** a non-empty `facets.precision` (principle 4 — no unqualified
  exact date) and **accepts** an optional non-empty `facets.basis`; it **parses no date** (a
  half-recalled value must record);
- ranks `clinician-observed` at **30** — below `patient-stated` (50) and `document-verified`
  (60), so a clinician's estimate is correctly **displaceable** the moment real identity
  documents arrive, with no special handling.

Therefore: **no `db/` floor change, no schema change, no new event type, no ADR/spec bump.**
The slice is pure logic + a node authoring path + advisory matcher range-awareness, all on top
of the existing floor and projection (`patient_demographic`).

## 3. Principle-4 core: why a *range*, and why *birth year*

An estimated age is (a) inherently a **range** ("about 40" ≈ 40 ± 5) and (b) **relative to the
observation date** (age 40 in 2026 is age 44 in 2030). Two honest-representation rules follow:

1. **Store the derived birth window, never the raw age.** Birth year is time-invariant; age
   drifts. Storing "age 40" would silently age the record. We convert at assertion time.
2. **Store an explicit range, not a single midpoint.** A single midpoint year (`1986`) is a
   *precise untruth*: the matcher would `DISAGREE` it against a true `1983` DOB — a false split
   on soft estimate data. An explicit range (`1981/1991`) refuses that false precision
   (principle 4 — an imprecise near-truth beats a precise untruth).

Consequence, confirmed against the matcher code: a range value is **unparseable by today's
`parse_dob`** (splits on `-`), so today it safely degrades to `None` → `INSUFFICIENT_DATA` (no
penalty, no help). To turn it into a **positive** match signal we add range-aware dob comparison
— that is the "full loop" half of this slice.

## 4. Layer 1 — pure core (`cairn-event`)

New pure functions (co-located with the demographic builders; keep the file < 500 lines, split a
new `evidence.rs` module if `demographics.rs` would grow too large):

- `birth_year_range_from_age(age_years: u32, tolerance_years: u32, observed_year: i32) -> (i32, i32)`
  Pure. `birth_year = observed_year - age`; returns
  `(observed_year - age - tolerance, observed_year - age + tolerance)`.
  Example: `observed_year=2026, age=40, tol=5 → (1981, 1991)`.
- `estimated_dob_body(min_year, max_year, basis, provenance) -> serde_json::Value`
  Builds a `demographic.field.asserted` payload reusing `dob_assertion_body` with:
  - **`value = "<min>/<max>"`** (e.g. `"1981/1991"`), the ISO-8601-interval-style `/` separator;
  - **`facets.precision = "year-range"`**;
  - `facets.basis = basis` (the clinician's free text, e.g.
    `"apparent age ~40±5: dentition, greying"`);
  - `provenance` (normally `"clinician-observed"`, but the parameter is open).
- Observed sex reuses the **existing** `administrative_sex_assertion_body(value, provenance)`
  with `provenance = "clinician-observed"`. **Not `sex-at-birth`**: a clinician observing an
  unconscious stranger sees *apparent/phenotypic* sex and cannot honestly claim the birth fact
  (principle 4); keeping it off `sex-at-birth` also guarantees it can never masquerade as a
  *verified* birth-sex feeding db/016's veto.

**Twin:** the existing `render_dob_twin` already produces a legible, profile-independent twin —
*"Date of birth (clinician-observed): 1981/1991 (year-range)"* — so **no new twin code**. The
observed-sex twin reuses `render_administrative_sex_twin`.

Value-encoding note: `"1981/1991"` does not collide with `parse_dob`'s `-` splitter (it fails
`int()` today → safe `None`); the matcher's new range branch keys on `precision == "year-range"`.

## 5. Layer 2 — node authoring (`cairn-node`)

A standalone **`assert-observed-evidence`** CLI subcommand (new `evidence.rs` in `cairn-node`),
authoring one or both `demographic.field.asserted` events on an **existing** patient UUID:

```
cairn-node assert-observed-evidence <patient-uuid> \
    [--age N --tol M --age-basis "dentition, greying"] \
    [--sex X --sex-basis "external genitalia"]
```

- Reuses the shared `db::next_hlc` HLC-tick helper and the actor-ensure path already used by
  `register_john_doe`; authors through the reused `submit_event` door in **one transaction** when
  both are supplied.
- The **observation year** for the age→range conversion is taken from the effective time of the
  assertion (the node's current wall-clock year by default; overridable is a later refinement —
  YAGNI now).
- **Standalone, not folded into `register-john-doe`** (which stays unchanged): §5.4 line 43 says
  evidence accrues over time, and a standalone command works both at registration and later. It
  also works on any chart, not only John Does (a clinician may add an observed estimate to a
  poorly-documented chart).

No floor change: the events are ordinary `demographic.field.asserted` assertions the existing
floor + `patient_demographic` projection already handle.

## 6. Layer 3 — advisory matcher (`matcher/`, Python)

Three edits, advisory-only (no `db/` floor, no SCHEMA bump):

- **`records.py`** — extend `DateValue` (or add a parallel field pair) to carry an optional
  birth-year **interval** `(year_min, year_max)`. Point dates keep their `year/month/day` shape
  unchanged.
- **`adapter.py` `parse_dob`** — when `precision == "year-range"`, parse `"<min>/<max>"` into the
  interval form; range-check the two years; an unparseable value still returns `None` (safe
  degrade, unchanged contract).
- **`comparators.py` `compare_dob`** — range-aware and **positive-only**:
  - point-vs-point: unchanged (existing shared-parts logic);
  - point-vs-range: point inside `[min, max]` → `PARTIAL`; point outside → `INSUFFICIENT_DATA`;
  - range-vs-range: intervals overlap → `PARTIAL`; disjoint → `INSUFFICIENT_DATA`.
  - **Never `DISAGREE`** for a range comparison. A soft visual estimate (provenance rank 30) may
    *support* a match but must never *suppress* one — mirroring `compare_identifier_sets`'
    positive-only contract and §5.4's recognition goal. The human adjudicates the worklist and
    judges age themselves.

## 7. Testing (TDD, failing test first)

- **`cairn-event` pure unit tests**: `birth_year_range_from_age` (midpoint, tolerance, an
  observation-year boundary); `estimated_dob_body` (value = `"min/max"`, precision =
  `"year-range"`, basis present/absent-omitted-never-null, provenance carried); observed-sex
  reuse; twin string renders legibly.
- **`cairn-node` DB-gated integration tests** (`tests/observed_evidence.rs`): an age assertion
  lands in `patient_demographic` as `clinician-observed` with the range value + facets; a later
  `document-verified` exact dob **displaces** it (provenance ladder); an observed-sex assertion
  lands on `administrative-sex`; both-in-one-transaction; asserting on a John-Doe chart composes
  with slice A (chart stays *unconfirmed* until `identify`).
- **matcher pure tests** (`test_compare_dob_range.py` / adapter tests): `parse_dob` of a
  `"year-range"` value → interval; unparseable range → `None`; point-inside-range → `PARTIAL`;
  point-outside → `INSUFFICIENT_DATA`; overlapping ranges → `PARTIAL`; disjoint → `INSUFFICIENT_DATA`;
  point-vs-point regression unchanged.
- **matcher DB-gated e2e**: a John Doe with an estimated-age range and a candidate chart whose
  real DOB falls inside the range → the pair is proposed/scored with a positive dob signal (not
  dropped); a candidate outside the range → the pair is neither penalized nor suppressed by dob.
- **Full suites green** before commit: `cargo test --workspace` + workspace clippy; matcher
  `uv run pytest` (pure) and `CAIRN_TEST_PG=… uv run --extra pipeline pytest` (DB); ruff clean.

## 8. What this slice deliberately does NOT change

- No `db/` floor file, no SCHEMA version bump, no new event type, no ADR, no spec edit — it
  implements settled §5.4/§4.2 on the existing spine.
- `register-john-doe` unchanged; the callsign / matcher-placeholder exclusion (slice A)
  unchanged.
- db/016 hard-veto untouched: `clinician-observed` is not *verified*, so the verified-DOB /
  verified-sex-at-birth vetoes never fire on this evidence — correct by construction.

## 9. Deferred (recorded, not lost)

- Photo / distinguishing marks / belongings / EMS-context evidence (new field home + attachment
  tier) — larger separate slice.
- The "prior history now available — N allergies, M meds" push-alert on link (§5.12, no
  notification tier yet).
- Fuzzy / graded age-range comparison beyond binary overlap (e.g. near-miss softening).
- **A birth-year-*range* blocking pass.** This slice makes the estimated range a positive
  *scoring* signal (`compare_dob`), which fires once a pair is already blocked together by
  another key (a shared identifier off belongings, a later refined name, or the hub duplicate
  sweep). It does **not** add a blocking key: `"1981/1991"` yields first-4-digit `1981`, which
  only blocks against a 1981-born candidate, not one born 1985 *inside* the range; and a John
  Doe's only name is an excluded callsign. So pure-age evidence alone won't auto-surface the
  prior chart via blocking — honest and safe (never a false merge/split), and consistent with
  §5.4's accreting-evidence workflow. A range-vs-point blocking pass (analogous to the existing
  name+birth-year compound pass) is a clean future slice.
- An explicit `--observed-year` override on the CLI (default = node wall-clock year now).
- Folding evidence capture into `register-john-doe` as a convenience (composition, not new
  mechanism).
