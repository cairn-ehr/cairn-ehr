# Administrative-Sex Scoring + Unconfirmed-Chart REVIEW Rule Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close [#130](https://github.com/cairn-ehr/cairn-ehr/issues/130): score sex using the
`administrative-sex` field (composite, positive-only fallback) and force an identity-pending
chart's corroborated below-threshold pairs to REVIEW, so the pure-age §5.4 John Doe pair is
blocked → scored → **surfaced** end-to-end.

**Architecture:** Advisory Python only (`matcher/` uv project) — no `db/` migration, no SCHEMA
bump, no ADR/spec edit. A new pure `compare_sex` comparator over a `SexValue` composite replaces
the `sex-at-birth` FieldSpec; `banding.band()` gains a scoped forcing rule (≥2 positive fields,
zero DISAGREE, no veto, one chart `unconfirmed` in `chart_trust`) mirroring the known-alias
precedent; `runner`/`sweep` wire trust states through the same batch-preload pattern as aliases.
Design: `docs/superpowers/specs/2026-07-05-admin-sex-scoring-design.md` (approved).

**Tech Stack:** Python 3.11+ (uv, never venv/pip), pytest, psycopg only behind the `pipeline`
extra, PG18 + cairn_pgx rig for DB-gated tests.

## Global Constraints

- AGPL-3.0; **zero new runtime dependencies** (the pure core stays dependency-free).
- TDD: write the failing test first, run it red, implement, run it green, commit.
- Inline documentation legible to a junior developer (why + how it fits, not what the next line does).
- Pure functions in `records/comparators/orchestrator/scoring/banding` — **no psycopg import
  outside `pipeline/db.py`**; `pipeline` modules import db lazily inside functions.
- `'sex-at-birth'` as the **projection field name** (SQL strings, conftest seeding,
  `eval/blocking_eval.py`) is the DB contract — MUST NOT change. Only the **weight/FieldSpec
  key** renames to `"sex"`.
- Pure suite: `cd matcher && uv run pytest` (must stay green with no psycopg installed).
- DB-gated suite: `cd matcher && CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" uv run --extra pipeline pytest`.
- Lint: `uvx ruff check matcher/src matcher/tests` must be clean.
- Commits on branch `claude/admin-sex-scoring`; conventional-commit messages.

---

### Task 1: Pure core — `SexValue` + `compare_sex`

**Files:**
- Modify: `matcher/src/cairn_matcher/records.py` (add `SexValue` after `DateValue`)
- Modify: `matcher/src/cairn_matcher/comparators.py` (add `compare_sex` after `compare_exact`)
- Test: `matcher/tests/test_comparator_sex.py` (new)

**Interfaces:**
- Consumes: `AgreementLevel`, `Context` (`cairn_matcher.agreement`), `MatcherTypeError`
  (`cairn_matcher.records`), `_require_str_or_none` (already in `comparators.py`).
- Produces: `records.SexValue(sex_at_birth: str | None = None, administrative: str | None = None)`
  frozen dataclass; `comparators.compare_sex(a: SexValue | None, b: SexValue | None, ctx: Context)
  -> AgreementLevel`. Task 2's orchestrator imports both.

- [ ] **Step 1: Write the failing tests**

Create `matcher/tests/test_comparator_sex.py`:

```python
"""compare_sex: the composite sex comparator (§5.4 slice, design 2026-07-05).

Branch 1 (both charts carry sex-at-birth) preserves the old compare_exact semantics —
a birth-fact clash stays honest negative evidence. Branch 2 (anything else) is the
positive-only union fallback: intersection -> EXACT, disjoint -> INSUFFICIENT_DATA,
NEVER DISAGREE — clinician-observed evidence may support but never suppress a match.
"""
import pytest

from cairn_matcher.agreement import AgreementLevel, Context
from cairn_matcher.comparators import compare_sex
from cairn_matcher.records import MatcherTypeError, SexValue

CTX = Context()


def test_absent_side_is_insufficient_data():
    assert compare_sex(None, None, CTX) == AgreementLevel.INSUFFICIENT_DATA
    assert compare_sex(SexValue(sex_at_birth="male"), None, CTX) == AgreementLevel.INSUFFICIENT_DATA


def test_both_sex_at_birth_exact():
    a = SexValue(sex_at_birth="male")
    b = SexValue(sex_at_birth="male")
    assert compare_sex(a, b, CTX) == AgreementLevel.EXACT


def test_both_sex_at_birth_disagree_is_preserved():
    # The birth-fact clash stays real negative evidence (aligned with the db/016 veto).
    a = SexValue(sex_at_birth="male")
    b = SexValue(sex_at_birth="female")
    assert compare_sex(a, b, CTX) == AgreementLevel.DISAGREE


def test_both_sab_branch_wins_even_when_admin_would_agree():
    # Branch 1 fires whenever BOTH sides carry sex-at-birth; administrative values do
    # not soften a birth-fact clash (no double-counting, no suppression of the signal).
    a = SexValue(sex_at_birth="male", administrative="female")
    b = SexValue(sex_at_birth="female", administrative="female")
    assert compare_sex(a, b, CTX) == AgreementLevel.DISAGREE


def test_admin_only_vs_sab_intersects_exact():
    # The §5.4 headline shape: Doe has observed administrative-sex only; the prior
    # chart has a real sex-at-birth. Union fallback intersects -> EXACT.
    doe = SexValue(administrative="male")
    prior = SexValue(sex_at_birth="male")
    assert compare_sex(doe, prior, CTX) == AgreementLevel.EXACT


def test_admin_only_vs_admin_only_intersects_exact():
    assert compare_sex(
        SexValue(administrative="male"), SexValue(administrative="male"), CTX
    ) == AgreementLevel.EXACT


def test_fallback_disjoint_is_never_disagree():
    # An apparent-sex misjudgement must not penalise the true pair (positive-only,
    # compare_identifier_sets precedent).
    doe = SexValue(administrative="male")
    prior = SexValue(sex_at_birth="female")
    assert compare_sex(doe, prior, CTX) == AgreementLevel.INSUFFICIENT_DATA


def test_trans_true_match_no_disagree_via_fallback():
    # Chart A: sex-at-birth male + administrative female. Chart B (same person,
    # other site): administrative female only. Branch 2 (B has no sab); A's union
    # {male, female} intersects B's {female} -> EXACT, not DISAGREE.
    a = SexValue(sex_at_birth="male", administrative="female")
    b = SexValue(administrative="female")
    assert compare_sex(a, b, CTX) == AgreementLevel.EXACT


def test_whitespace_trims_and_empty_degrades():
    # Trim-only (no casefold — culture-touching, locale-pack territory, same
    # discipline as compare_exact). An all-whitespace value is absence.
    assert compare_sex(
        SexValue(sex_at_birth=" male "), SexValue(sex_at_birth="male"), CTX
    ) == AgreementLevel.EXACT
    assert compare_sex(
        SexValue(sex_at_birth="   "), SexValue(sex_at_birth="male"), CTX
    ) == AgreementLevel.INSUFFICIENT_DATA


def test_wrong_type_raises():
    with pytest.raises(MatcherTypeError):
        compare_sex("male", SexValue(sex_at_birth="male"), CTX)  # type: ignore[arg-type]
    with pytest.raises(MatcherTypeError):
        compare_sex(SexValue(sex_at_birth=123), SexValue(sex_at_birth="male"), CTX)  # type: ignore[arg-type]
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cd matcher && uv run pytest tests/test_comparator_sex.py -v`
Expected: FAIL — `ImportError: cannot import name 'SexValue'`.

- [ ] **Step 3: Implement `SexValue` and `compare_sex`**

In `matcher/src/cairn_matcher/records.py`, after the `DateValue` class:

```python
@dataclass(frozen=True)
class SexValue:
    """The composite value the `sex` comparator sees: both of one chart's sex facets.

    sex_at_birth is the §4.2 birth fact (the db/016 veto's subject); administrative is
    the apparent/phenotypic field a §5.4 clinician-observed sex lands on (slice B chose
    it deliberately — a clinician cannot know the birth fact). Either may be None; a
    chart with neither field never constructs a SexValue at all (the orchestrator's
    extractor returns None instead, which grades INSUFFICIENT_DATA).
    """

    sex_at_birth: str | None = None
    administrative: str | None = None
```

In `matcher/src/cairn_matcher/comparators.py`, import `SexValue` from
`cairn_matcher.records` (extend the existing import line) and add after `compare_exact`:

```python
def compare_sex(a: "SexValue | None", b: "SexValue | None", ctx: Context) -> AgreementLevel:
    """Composite sex agreement over sex-at-birth + administrative-sex (design 2026-07-05).

    Two branches, in priority order:
      1. BOTH charts carry sex-at-birth -> exact-compare those two values (trim-only, no
         casefold — compare_exact's discipline). EXACT / DISAGREE: a birth-fact clash is
         honest negative evidence, aligned with the db/016 veto's subject. Administrative
         values never soften it (they are heavily correlated with the birth fact; scoring
         them separately would double-count and break Fellegi–Sunter independence).
      2. Otherwise the POSITIVE-ONLY union fallback (mirrors the blocking pass's
         blocking_sex union): each side's set of present values; either empty ->
         INSUFFICIENT_DATA; intersection -> EXACT; disjoint -> INSUFFICIENT_DATA — NEVER
         DISAGREE. A clinician-observed apparent sex may *support* but never *suppress* a
         match (slice B's rule; an androgynous patient misjudged at the bedside must not
         penalise their true pair).
    """
    if a is None or b is None:
        return AgreementLevel.INSUFFICIENT_DATA
    if not isinstance(a, SexValue) or not isinstance(b, SexValue):
        raise MatcherTypeError(
            f"compare_sex expected SexValue or None, got {type(a)!r} / {type(b)!r}"
        )
    sab_a = _clean_sex(a.sex_at_birth, "compare_sex.a.sex_at_birth")
    sab_b = _clean_sex(b.sex_at_birth, "compare_sex.b.sex_at_birth")
    if sab_a is not None and sab_b is not None:
        return AgreementLevel.EXACT if sab_a == sab_b else AgreementLevel.DISAGREE
    set_a = {v for v in (sab_a, _clean_sex(a.administrative, "compare_sex.a.administrative")) if v}
    set_b = {v for v in (sab_b, _clean_sex(b.administrative, "compare_sex.b.administrative")) if v}
    if not set_a or not set_b:
        return AgreementLevel.INSUFFICIENT_DATA
    return AgreementLevel.EXACT if set_a & set_b else AgreementLevel.INSUFFICIENT_DATA


def _clean_sex(value: object, field_name: str) -> str | None:
    """Trim a sex facet; an all-whitespace value degrades to absence (None)."""
    cleaned = _require_str_or_none(value, field_name)
    return cleaned if cleaned else None
```

Also extend the `records` import at the top of `comparators.py` to include `SexValue`.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cd matcher && uv run pytest tests/test_comparator_sex.py -v`
Expected: all PASS.

- [ ] **Step 5: Run the whole pure suite (regression) and commit**

Run: `cd matcher && uv run pytest`
Expected: all pass (200 pure + the new ones).

```bash
git add matcher/src/cairn_matcher/records.py matcher/src/cairn_matcher/comparators.py matcher/tests/test_comparator_sex.py
git commit -m "feat(matcher): SexValue composite + positive-only compare_sex comparator (#130)"
```

---

### Task 2: Orchestrator composite extraction + `"sex"` weight key

**Files:**
- Modify: `matcher/src/cairn_matcher/records.py` (add `administrative_sex` to `CandidateRecord`)
- Modify: `matcher/src/cairn_matcher/orchestrator.py` (`_sex_composite`, `FieldSpec("sex", …)`)
- Modify: `matcher/src/cairn_matcher/scoring.py` (`DEFAULT_WEIGHTS` key rename)
- Test: `matcher/tests/test_orchestrator.py` (update + add), `matcher/tests/test_scoring.py` (update)

**Interfaces:**
- Consumes: `SexValue` + `compare_sex` (Task 1).
- Produces: `CandidateRecord.administrative_sex: FieldValue | None = None` (keyword field,
  placed directly after `sex_at_birth`; all existing constructions are keyword-based — verified);
  the orchestrator emits a `FieldComparison(field="sex", …)` instead of `"sex-at-birth"`;
  `DEFAULT_WEIGHTS.per_field["sex"]` = `{EXACT: 1.0, DISAGREE: -2.0}`. Tasks 3–6 rely on the
  `"sex"` field key in evidence.

- [ ] **Step 1: Update/add failing tests**

In `matcher/tests/test_orchestrator.py`: change every expectation of a `"sex-at-birth"`
field comparison to `"sex"` (the record-side attribute stays `sex_at_birth`), and add:

```python
def test_sex_composite_uses_admin_sex_when_sab_absent():
    # The §5.4 headline shape: Doe carries administrative-sex only (clinician-observed,
    # rank 30); prior chart carries sex-at-birth (rank 60). Fallback EXACT at the
    # weaker side's rank (min(30, 60) = 30).
    doe = CandidateRecord(administrative_sex=FieldValue("male", provenance_rank=30))
    prior = CandidateRecord(sex_at_birth=FieldValue("male", provenance_rank=60))
    comp = next(c for c in field_comparisons(doe, prior) if c.field == "sex")
    assert comp.level == AgreementLevel.EXACT
    assert comp.provenance_rank == 30


def test_sex_rank_prefers_sex_at_birth_when_present():
    # Side rank rule: sex-at-birth's rank when present, else administrative's
    # (documented second-order approximation, design §2).
    a = CandidateRecord(
        sex_at_birth=FieldValue("male", provenance_rank=60),
        administrative_sex=FieldValue("male", provenance_rank=30),
    )
    b = CandidateRecord(sex_at_birth=FieldValue("male", provenance_rank=50))
    comp = next(c for c in field_comparisons(a, b) if c.field == "sex")
    assert comp.provenance_rank == 50  # min(60, 50): admin's 30 is not the side rank


def test_sex_absent_both_facets_is_insufficient_data():
    comp = next(
        c for c in field_comparisons(CandidateRecord(), CandidateRecord())
        if c.field == "sex"
    )
    assert comp.level == AgreementLevel.INSUFFICIENT_DATA
```

In `matcher/tests/test_scoring.py`: change `"sex-at-birth"` weight-key references to `"sex"`
(FieldComparison fixtures and DEFAULT_WEIGHTS assertions — the projection field name does not
appear in this file), and add the explicit key test:

```python
def test_default_weights_carry_sex_key_not_sex_at_birth():
    # The weight/FieldSpec key renamed to "sex" (composite field); 'sex-at-birth'
    # remains ONLY as the projection field name in SQL/seeding, never a weight key.
    assert "sex" in DEFAULT_WEIGHTS.per_field
    assert "sex-at-birth" not in DEFAULT_WEIGHTS.per_field
```

- [ ] **Step 2: Run to verify failures**

Run: `cd matcher && uv run pytest tests/test_orchestrator.py tests/test_scoring.py -v`
Expected: new tests FAIL (`administrative_sex` unknown kwarg / field `"sex"` absent); renamed
expectations FAIL against the current `"sex-at-birth"` key.

- [ ] **Step 3: Implement**

`records.py` — in `CandidateRecord`, directly after `sex_at_birth`:

```python
    administrative_sex: FieldValue | None = None  # §5.4 apparent/phenotypic sex facet
```

`orchestrator.py` — import `SexValue` and `compare_sex`; replace the `sex-at-birth` FieldSpec
with `FieldSpec("sex", compare_sex, _sex_composite)` and add:

```python
def _sex_composite(rec: CandidateRecord) -> tuple[Any, int]:
    """Build the SexValue composite + this side's provenance rank.

    Rank rule: sex-at-birth's rank when that facet is present, else administrative-sex's.
    In the edge case where the union fallback intersects on the OTHER facet than the one
    whose rank we report, the rank is a second-order approximation — bounded by the
    [0.5, 1.0] provenance factor on a weight of 1.0 (design 2026-07-05 §2); revisit only
    if B3 provenance-sensitive tuning makes it observable. The orchestrator's existing
    min(rank_a, rank_b) then reduces to the weaker side, as for every field.
    """
    sab, admin = rec.sex_at_birth, rec.administrative_sex
    if sab is None and admin is None:
        return (None, 0)
    value = SexValue(
        sex_at_birth=None if sab is None else sab.value,
        administrative=None if admin is None else admin.value,
    )
    rank = sab.provenance_rank if sab is not None else admin.provenance_rank
    return (value, rank)
```

`scoring.py` — in `DEFAULT_WEIGHTS`, rename the `"sex-at-birth"` key to `"sex"` (same weights,
`EXACT: 1.0, DISAGREE: -2.0`) and update the adjacent comment. `matcher_version()` digests the
weight table, so the version pin changes automatically — proposals from this config are
distinguishable from prior runs (per-epoch matcher actor, C2b). No code change needed for that.

- [ ] **Step 4: Run to verify green**

Run: `cd matcher && uv run pytest`
Expected: whole pure suite passes.

- [ ] **Step 5: Commit**

```bash
git add matcher/src/cairn_matcher/records.py matcher/src/cairn_matcher/orchestrator.py matcher/src/cairn_matcher/scoring.py matcher/tests/test_orchestrator.py matcher/tests/test_scoring.py
git commit -m "feat(matcher): score sex via composite field — admin-sex fallback, 'sex' weight key (#130)"
```

---

### Task 3: Adapter + `load_candidate` read administrative-sex

**Files:**
- Modify: `matcher/src/cairn_matcher/pipeline/adapter.py` (`candidate_from_rows` gains `admin_sex_row`)
- Modify: `matcher/src/cairn_matcher/pipeline/db.py` (`load_candidate` widens the sex SELECT)
- Test: `matcher/tests/test_adapter_record.py` (pure additions only — the DB-gated
  `load_candidate` administrative-sex assertion lands in Task 5's new
  `test_identity_pending_pipeline.py`, which is created there)

**Interfaces:**
- Consumes: `CandidateRecord.administrative_sex` (Task 2), `single_field` (existing).
- Produces: `candidate_from_rows(*, dob_row, sex_row, name_rows, identifier_rows,
  admin_sex_row=None)`; `load_candidate` returns records with `administrative_sex` populated.

- [ ] **Step 1: Write failing pure tests**

Add to `matcher/tests/test_adapter_record.py`:

```python
def test_admin_sex_row_shapes_into_administrative_sex():
    rec = candidate_from_rows(
        dob_row=None, sex_row=None, name_rows=(), identifier_rows=(),
        admin_sex_row={"value": "male", "provenance_rank": 30},
    )
    assert rec.administrative_sex == FieldValue("male", provenance_rank=30)
    assert rec.sex_at_birth is None


def test_admin_sex_unknown_sentinel_degrades_to_absence():
    # `unknown` is a legitimate recorded value but ZERO matching evidence (principle 4)
    # — same single_field discipline as sex-at-birth.
    rec = candidate_from_rows(
        dob_row=None, sex_row=None, name_rows=(), identifier_rows=(),
        admin_sex_row={"value": "unknown", "provenance_rank": 30},
    )
    assert rec.administrative_sex is None


def test_admin_sex_defaults_to_none():
    rec = candidate_from_rows(dob_row=None, sex_row=None, name_rows=(), identifier_rows=())
    assert rec.administrative_sex is None
```

- [ ] **Step 2: Run to verify failure**

Run: `cd matcher && uv run pytest tests/test_adapter_record.py -v`
Expected: FAIL — unexpected keyword `admin_sex_row`.

- [ ] **Step 3: Implement**

`adapter.py` — `candidate_from_rows` gains keyword-only `admin_sex_row: Mapping | None = None`
and passes `administrative_sex=single_field(admin_sex_row)` (update its docstring: the
administrative-sex facet rides the same sentinel discipline as sex-at-birth).

`db.py` — in `load_candidate`, replace the single sex-at-birth SELECT with ONE query for both
facets and split the rows in Python:

```python
        # One query for BOTH sex facets (§4.2 sex-at-birth: the birth fact; §5.4
        # administrative-sex: the apparent/phenotypic facet a clinician-observed sex
        # lands on). Split by field name here — 'sex-at-birth'/'administrative-sex'
        # are the projection contract, NOT the scorer's weight key (that is "sex").
        cur.execute("SELECT field, value, provenance_rank FROM patient_demographic "
                    "WHERE patient_id=%s AND field IN ('sex-at-birth','administrative-sex')",
                    (patient_id,))
        sex_rows = cur.fetchall()
        sex_row = next((r for r in sex_rows if r["field"] == "sex-at-birth"), None)
        admin_sex_row = next((r for r in sex_rows if r["field"] == "administrative-sex"), None)
```

and pass `admin_sex_row=admin_sex_row` to `candidate_from_rows`. Update `load_candidate`'s
docstring ("winner rows (dob, both sex facets)").

- [ ] **Step 4: Run pure suite green**

Run: `cd matcher && uv run pytest`
Expected: all pass.

- [ ] **Step 5: Run DB-gated suite (regression on the widened SELECT)**

Run: `cd matcher && CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" uv run --extra pipeline pytest`
Expected: all pass (existing e2e tests exercise `load_candidate` against live rows).

- [ ] **Step 6: Commit**

```bash
git add matcher/src/cairn_matcher/pipeline/adapter.py matcher/src/cairn_matcher/pipeline/db.py matcher/tests/test_adapter_record.py
git commit -m "feat(matcher): load_candidate reads administrative-sex into the composite (#130)"
```

---

### Task 4: Banding — the unconfirmed-chart forcing rule + evidence marker

**Files:**
- Modify: `matcher/src/cairn_matcher/pipeline/banding.py` (`band()` gains `unconfirmed`;
  `_corroborated_positive`; `build_payload` gains `trust_evidence`)
- Test: `matcher/tests/test_banding.py` (additions)

**Interfaces:**
- Consumes: `MatchScore`/`FieldEvidence` (existing), `AgreementLevel`.
- Produces: `band(score, vetoes, thresholds=…, *, has_known_alias=False, unconfirmed=False)`;
  `build_payload(score, vetoes, band_value, weights=…, alias_evidence=(), trust_evidence=())`.
  Task 5's runner calls both with the new arguments.

- [ ] **Step 1: Write failing tests**

Add to `matcher/tests/test_banding.py` (reuse the file's existing MatchScore/FieldEvidence
construction helpers; the snippets below build scores explicitly for clarity):

```python
def _evidence(field, level, contribution, rank=30):
    return FieldEvidence(field, level, rank, contribution)


def _headline_score():
    # The §5.4 pure-age pair: dob PARTIAL + sex EXACT, everything else absent — ≈1.79,
    # below review=3.0.
    return MatchScore(total=1.79, fields=(
        _evidence("dob", AgreementLevel.PARTIAL, 1.07),
        _evidence("sex", AgreementLevel.EXACT, 0.71),
        _evidence("name", AgreementLevel.INSUFFICIENT_DATA, 0.0),
        _evidence("identifier", AgreementLevel.INSUFFICIENT_DATA, 0.0),
    ))


def test_unconfirmed_rule_forces_review_on_corroborated_pair():
    assert band(_headline_score(), vetoes=(), unconfirmed=True) is Band.REVIEW


def test_unconfirmed_rule_needs_two_positive_fields():
    # A bare window overlap (one positive field) must NOT flood the worklist.
    one_field = MatchScore(total=1.07, fields=(
        _evidence("dob", AgreementLevel.PARTIAL, 1.07),
    ))
    assert band(one_field, vetoes=(), unconfirmed=True) is None


def test_unconfirmed_rule_blocked_by_any_disagree():
    # Disagreeing evidence -> the normal scoring path decides; forcing is only for
    # thin-but-uncontradicted evidence.
    contradicted = MatchScore(total=0.4, fields=(
        _evidence("dob", AgreementLevel.PARTIAL, 1.07),
        _evidence("sex", AgreementLevel.EXACT, 0.71),
        _evidence("name", AgreementLevel.DISAGREE, -1.4),
    ))
    assert band(contradicted, vetoes=(), unconfirmed=True) is None


def test_unconfirmed_rule_blocked_by_a_veto():
    veto = VetoFinding("dob_clash", "hard_veto", "dob", "verified clash")
    assert band(_headline_score(), vetoes=(veto,), unconfirmed=True) is None


def test_unconfirmed_rule_inert_when_flag_false():
    assert band(_headline_score(), vetoes=(), unconfirmed=False) is None


def test_unconfirmed_rule_never_upgrades_to_auto():
    # Above-auto scores follow the NORMAL path; the forcing rule only ever acts below
    # review and only ever yields REVIEW.
    big = MatchScore(total=9.0, fields=(
        _evidence("identifier", AgreementLevel.EXACT, 8.0),
        _evidence("dob", AgreementLevel.PARTIAL, 1.0),
    ))
    assert band(big, vetoes=(), unconfirmed=True) is Band.AUTO_CANDIDATE  # unchanged path


def test_build_payload_appends_trust_evidence_after_alias_evidence():
    score = _headline_score()
    marker = {"rule": "identity_pending", "unconfirmed": ["11111111-1111-1111-1111-111111111111"]}
    payload = build_payload(score, (), Band.REVIEW, trust_evidence=(marker,))
    assert payload.evidence[-1] == marker
```

- [ ] **Step 2: Run to verify failures**

Run: `cd matcher && uv run pytest tests/test_banding.py -v`
Expected: new tests FAIL — unexpected keyword `unconfirmed` / `trust_evidence`.

- [ ] **Step 3: Implement**

`banding.py` — make `has_known_alias` and the new flag keyword-only (it already is), add after
`_has_shared_identifier`:

```python
def _corroborated_positive(score: MatchScore) -> bool:
    """≥2 fields contributing positive weight, and NO disagreeing field.

    The structural flood-control gate of the §5.4 unconfirmed-chart rule (design
    2026-07-05 §4): age-window overlap + shared sex (or overlap + a shared identifier
    off belongings) qualifies; a bare 11-year window overlap — which a sizeable DB
    satisfies for a double-digit share of charts — does not. Structural (a field count)
    rather than a score floor so it does not drift as provenance factors or weights
    change. The paper counterpart searches the registry on age AND sex, never age alone.
    """
    positives = sum(1 for e in score.fields if e.weight_contribution > 0)
    disagrees = any(e.level is AgreementLevel.DISAGREE for e in score.fields)
    return positives >= 2 and not disagrees
```

`band()` — add the keyword-only parameter `unconfirmed: bool = False`, extend the docstring
(the §5.4 identity-pending forcing: an *unconfirmed* chart NEEDS human identification effort,
so its corroborated candidates must reach the worklist — the known-alias precedent; REVIEW
only, never AUTO — §5.7 reserves that call for a human; no-veto gate is near-vacuous for a
real Doe since a db/016 veto needs verified values on BOTH sides, and the veto+identifier
case is owned by the existing rescue; `'under-review'` is a dispute state and deliberately
does NOT trigger this), and change the below-review branch to:

```python
    if score.total < thresholds.review:
        if vetoes and _has_shared_identifier(score):
            return Band.REVIEW
        if unconfirmed and not vetoes and _corroborated_positive(score):
            return Band.REVIEW
        return None
```

`build_payload()` — add parameter `trust_evidence: Sequence[dict] = ()` and append it after
the alias entries:

```python
    ) + tuple(alias_evidence) + tuple(trust_evidence)
```

(update its docstring: the `identity_pending` marker rides behind the alias entries so a hub
worklist can group a Doe's candidate list — pure surfacing, never suppression).

- [ ] **Step 4: Run green**

Run: `cd matcher && uv run pytest tests/test_banding.py -v && uv run pytest`
Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add matcher/src/cairn_matcher/pipeline/banding.py matcher/tests/test_banding.py
git commit -m "feat(matcher): unconfirmed-chart REVIEW forcing rule + identity_pending marker (#130)"
```

---

### Task 5: Trust loading + runner/sweep wiring + conftest seam

**Files:**
- Modify: `matcher/src/cairn_matcher/pipeline/db.py` (`load_trust`, `load_trust_for`)
- Modify: `matcher/src/cairn_matcher/pipeline/runner.py` (`propose` gains `trust`)
- Modify: `matcher/src/cairn_matcher/pipeline/sweep.py` (pre-load trust like aliases)
- Modify: `matcher/tests/conftest.py` (`chart_identity_state` truncation + `seed_identity_pending`)
- Test: `matcher/tests/test_identity_pending_pipeline.py` (new, DB-gated)

**Interfaces:**
- Consumes: `band(…, unconfirmed=…)` + `build_payload(…, trust_evidence=…)` (Task 4);
  `chart_trust` view (db/024: columns `patient_id`, `trust_state`; rows ONLY for flagged
  charts; `cairn_agent` holds SELECT); `chart_identity_state` table (db/024: `subject` PK,
  `state` in ('pending','identified'), `detail`, `hlc_wall`, `hlc_counter`, `origin`).
- Produces: `db.load_trust(conn, patient_id) -> str | None`;
  `db.load_trust_for(conn, patient_ids) -> dict[str, str]` (canonical lowercase uuid text
  keys, absent key = confirmed); `propose(conn, a, b, *, thresholds=…, weights=…,
  aliases=None, trust=None)`; `conftest.seed_identity_pending(conn, subject, *, basis=…)`.

- [ ] **Step 1: Write failing DB-gated tests**

Create `matcher/tests/test_identity_pending_pipeline.py`:

```python
"""§5.4 identity-pending trust plumbing + the #130 end-to-end (DB-gated).

Gated on CAIRN_TEST_PG via the pg_conn fixture's internal skip (house convention).
psycopg-touching imports stay INSIDE test functions so the pure `uv run pytest` run
(no psycopg installed) still collects this module cleanly.
"""
import uuid

from tests.conftest import seed_identity_pending, seed_patient


def _uid() -> str:
    return str(uuid.uuid4())


def test_load_trust_for_reads_pending_as_unconfirmed(pg_conn):
    from cairn_matcher.pipeline.db import load_trust, load_trust_for

    doe, ordinary = _uid(), _uid()
    seed_patient(pg_conn, doe, dob=("1981/1991", 30, "year-range"))
    seed_patient(pg_conn, ordinary, dob=("1985-03-12", 60, "day"))
    seed_identity_pending(pg_conn, doe)

    trust = load_trust_for(pg_conn, [doe, ordinary])
    assert trust.get(doe) == "unconfirmed"
    assert ordinary not in trust  # absent row = confirmed default
    assert load_trust(pg_conn, doe) == "unconfirmed"
    assert load_trust(pg_conn, ordinary) is None


def test_load_trust_for_empty_set_is_empty(pg_conn):
    from cairn_matcher.pipeline.db import load_trust_for

    assert load_trust_for(pg_conn, []) == {}


def test_load_candidate_populates_administrative_sex(pg_conn):
    # The Task-3 widened SELECT, proven against live projection rows (spec §6 DB-gated).
    from cairn_matcher.pipeline.db import load_candidate

    doe = _uid()
    seed_patient(pg_conn, doe, admin_sex=("male", 30))
    rec = load_candidate(pg_conn, doe)
    assert rec.administrative_sex is not None
    assert rec.administrative_sex.value == "male"
    assert rec.administrative_sex.provenance_rank == 30
    assert rec.sex_at_birth is None
```

- [ ] **Step 2: Run to verify failures**

Run: `cd matcher && CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" uv run --extra pipeline pytest tests/test_identity_pending_pipeline.py -v`
Expected: FAIL — `seed_identity_pending` / `load_trust_for` not defined.

- [ ] **Step 3: Implement**

`conftest.py` — add `"chart_identity_state"` to `_PROJECTION_TABLES` (it must truncate between
tests like every other seeded projection) and add after `seed_repudiation`:

```python
def seed_identity_pending(conn, subject, *, basis="unidentified at registration"):
    """Mark a chart identity-pending directly (chart_identity_state), bypassing the C4 floor.

    The floor (submit_event + authored twin + the identity-state assertion gate) is proven
    in crates/cairn-node/tests; these matcher tests exercise CONSUMPTION of the chart_trust
    projection, not how it is written — the same rationale as seed_repudiation above.
    """
    with conn.cursor() as cur:
        cur.execute(
            "INSERT INTO chart_identity_state (subject, state, detail, hlc_wall, hlc_counter, origin) "
            "VALUES (%s,'pending',%s,0,0,'seed') "
            "ON CONFLICT (subject) DO UPDATE SET state='pending', detail=EXCLUDED.detail",
            (subject, basis),
        )
    conn.commit()
```

`db.py` — add after `load_aliases_for`:

```python
def load_trust(conn, patient_id) -> str | None:
    """One chart's §5.7 trust state from the chart_trust view; None = confirmed.

    The view (db/024) carries rows ONLY for flagged charts (unconfirmed / under-review),
    so an absent row IS the confirmed default — mirrored by person_chart_trust's COALESCE.
    Single-pair path only; a batch driver uses load_trust_for (one query for the set).
    """
    with conn.cursor() as cur:
        cur.execute("SELECT trust_state FROM chart_trust WHERE patient_id=%s", (patient_id,))
        row = cur.fetchone()
        return None if row is None else row[0]


def load_trust_for(conn, patient_ids) -> dict[str, str]:
    """Trust states for a whole candidate set in ONE query (the load_aliases_for pattern).

    Absent key = confirmed. Keys are canonical lowercase uuid text, matching str(patient_id)
    at the call site. Scoped to the given ids so this stays an index probe, never a scan of
    every flagged chart in the fleet.
    """
    ids = [str(p) for p in patient_ids]
    if not ids:
        return {}
    with conn.cursor() as cur:
        cur.execute(
            "SELECT patient_id, trust_state FROM chart_trust WHERE patient_id = ANY(%s::uuid[])",
            (ids,),
        )
        return {str(pid): state for pid, state in cur.fetchall()}
```

`runner.py` — `propose` gains keyword `trust: Mapping[str, "str"] | None = None` (document: the
preloaded batch map, the `aliases` pattern; None = per-pair on-demand loads). After the alias
block and before `band(…)`:

```python
    # §5.4 identity-pending trust: an *unconfirmed* chart (a standing John Doe) needs human
    # identification effort, so banding may force its corroborated pairs to REVIEW (design
    # 2026-07-05 §4). Trust states come from the caller's preloaded map when batching, else
    # per-pair on-demand loads (same seam as aliases).
    if trust is None:
        trust_a = db.load_trust(conn, a)
        trust_b = db.load_trust(conn, b)
    else:
        trust_a = trust.get(str(a))
        trust_b = trust.get(str(b))
    unconfirmed_ids = sorted(
        str(p) for p, t in ((a, trust_a), (b, trust_b)) if t == "unconfirmed"
    )
```

change the `band(...)` call to pass `unconfirmed=bool(unconfirmed_ids)`, and build the marker
before `build_payload`:

```python
    trust_evidence = (
        ({"rule": "identity_pending", "unconfirmed": unconfirmed_ids},)
        if unconfirmed_ids else ()
    )
    payload = build_payload(
        match_score, vetoes, band_value, weights, alias_evidence, trust_evidence
    )
```

(The marker is emitted on EVERY persisted proposal involving an unconfirmed chart — also
above-threshold ones — so a hub worklist can group a Doe's whole candidate list.)

`sweep.py` — next to the alias preload:

```python
    # Pre-load the §5.4 trust states for the candidate set in the same ONE-query style;
    # propose() then reads trust from this map, not the DB (see the aliases preload above).
    trust = db.load_trust_for(conn, candidate_patients)
```

and pass `trust=trust` in the `propose(...)` call.

- [ ] **Step 4: Run green**

Run: `cd matcher && CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" uv run --extra pipeline pytest tests/test_identity_pending_pipeline.py -v`
Expected: PASS.

Run: `cd matcher && uv run pytest`
Expected: pure suite still green (no psycopg needed to collect).

- [ ] **Step 5: Commit**

```bash
git add matcher/src/cairn_matcher/pipeline/db.py matcher/src/cairn_matcher/pipeline/runner.py matcher/src/cairn_matcher/pipeline/sweep.py matcher/tests/conftest.py matcher/tests/test_identity_pending_pipeline.py
git commit -m "feat(matcher): chart_trust plumbing — batch preload + propose/sweep wiring (#130)"
```

---

### Task 6: The #130 end-to-end + full-suite gate

**Files:**
- Modify: `matcher/tests/test_identity_pending_pipeline.py` (add the e2e tests)

**Interfaces:**
- Consumes: everything above; `sweep` (existing), `canonical_pair`
  (`cairn_matcher.pipeline.runner` re-export), `match_proposal` table (db/017: columns
  `patient_low, patient_high, band, evidence`).
- Produces: the regression test that IS #130's close-out evidence.

- [ ] **Step 1: Write the failing e2e tests**

Append to `matcher/tests/test_identity_pending_pipeline.py`:

```python
def test_pure_age_john_doe_pair_surfaces_as_review_end_to_end(pg_conn):
    # THE #130 headline: a callsign-only John Doe with clinician-observed evidence
    # (estimated-age range + observed administrative-sex, both rank 30) vs their prior
    # chart. Blocks via the anchored dob-range passes; scores ≈1.79 (below review=3.0);
    # the unconfirmed-chart rule forces REVIEW. Blocked -> scored -> SURFACED.
    import json

    from cairn_matcher.pipeline.runner import canonical_pair
    from cairn_matcher.pipeline.sweep import sweep

    doe, prior = _uid(), _uid()
    seed_patient(
        pg_conn, doe,
        dob=("1981/1991", 30, "year-range"),
        admin_sex=("male", 30),
        callsign_names=[("Unknown-ED-XX-20260705-abcd1234", 30)],
    )
    seed_patient(
        pg_conn, prior,
        dob=("1985-03-12", 60, "day"),
        sex=("male", 60),
        names=[("Robert Menzies", 60)],
    )
    seed_identity_pending(pg_conn, doe)

    result = sweep(pg_conn)

    assert result.errors == []
    low, high = canonical_pair(doe, prior)
    with pg_conn.cursor() as cur:
        cur.execute(
            "SELECT band, evidence FROM match_proposal "
            "WHERE patient_low=%s AND patient_high=%s",
            (low, high),
        )
        row = cur.fetchone()
    assert row is not None, "the pure-age Doe pair must persist a proposal"
    band_value, evidence = row
    assert band_value == "review"
    entries = evidence if isinstance(evidence, list) else json.loads(evidence)
    marker = next(e for e in entries if e.get("rule") == "identity_pending")
    assert marker["unconfirmed"] == [str(uuid.UUID(doe))]


def test_without_pending_state_the_same_pair_stays_below_threshold(pg_conn):
    # The control: identical evidence, no identity-pending state -> the pair still
    # blocks and scores ≈1.79 but persists NOTHING. Proves the forcing rule (not the
    # sex scoring alone) is what surfaces the headline pair.
    from cairn_matcher.pipeline.runner import canonical_pair
    from cairn_matcher.pipeline.sweep import sweep

    doe, prior = _uid(), _uid()
    seed_patient(
        pg_conn, doe,
        dob=("1981/1991", 30, "year-range"),
        admin_sex=("male", 30),
        callsign_names=[("Unknown-ED-XX-20260705-abcd1234", 30)],
    )
    seed_patient(
        pg_conn, prior,
        dob=("1985-03-12", 60, "day"),
        sex=("male", 60),
        names=[("Robert Menzies", 60)],
    )

    sweep(pg_conn)

    low, high = canonical_pair(doe, prior)
    with pg_conn.cursor() as cur:
        cur.execute(
            "SELECT 1 FROM match_proposal WHERE patient_low=%s AND patient_high=%s",
            (low, high),
        )
        assert cur.fetchone() is None
```

- [ ] **Step 2: Run to verify the e2e result**

Run: `cd matcher && CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" uv run --extra pipeline pytest tests/test_identity_pending_pipeline.py -v`
Expected: both new tests PASS immediately IF Tasks 1–5 are correct — this task is the
integration gate, not new implementation. If the first FAILS, debug the pipeline (most likely
suspects: the marker not persisted through `upsert_proposal`'s JSON path, or the sweep not
passing `trust=`). The control test must pass unchanged.

- [ ] **Step 3: Run BOTH full suites + lint**

Run: `cd matcher && uv run pytest`
Expected: pure suite green (was 200, now more).

Run: `cd matcher && CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" uv run --extra pipeline pytest`
Expected: DB-gated suite green (was 264, now more).

Run: `uvx ruff check matcher/src matcher/tests`
Expected: clean.

- [ ] **Step 4: Commit**

```bash
git add matcher/tests/test_identity_pending_pipeline.py
git commit -m "test(matcher): #130 end-to-end — pure-age John Doe pair surfaces as REVIEW"
```

---

## Post-plan session work (NOT tasks for implementer subagents)

Final whole-branch review; update `docs/HANDOVER.md` + `docs/ROADMAP.md` (prune, keep <500
lines); push branch `claude/admin-sex-scoring`; open the PR to `main` linking issue #130
("Closes #130" — the headline pair now surfaces; note the remaining deferred items from the
design §7 stay recorded on the issue/HANDOVER as appropriate).
