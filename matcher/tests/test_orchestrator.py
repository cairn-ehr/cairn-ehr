import functools

import pytest

from cairn_matcher.agreement import AgreementLevel
from cairn_matcher.comparators import compare_dob
from cairn_matcher.orchestrator import FieldSpec, field_comparisons
from cairn_matcher.records import CandidateRecord, DateValue, FieldValue, Name


def n(given, family):
    return Name(tokens={"given": tuple(given), "family": tuple(family)})


def _rec():
    return CandidateRecord(
        dob=FieldValue(DateValue(1980, 3, 15), provenance_rank=60),
        sex_at_birth=FieldValue("female", provenance_rank=60),
        names=FieldValue(frozenset({n(["alex"], ["kim"])}), provenance_rank=20),
        identifiers={"au-medicare": frozenset({"2951"})},
    )


def test_identical_records_produce_field_agreements():
    comps = {c.field: c for c in field_comparisons(_rec(), _rec())}
    assert comps["dob"].level is AgreementLevel.EXACT
    assert comps["sex"].level is AgreementLevel.EXACT
    assert comps["name"].level is AgreementLevel.EXACT
    assert comps["identifier"].level is AgreementLevel.EXACT


def test_provenance_is_the_weaker_of_the_two_sides():
    strong = CandidateRecord(dob=FieldValue(DateValue(1980, 3, 15), provenance_rank=60))
    weak = CandidateRecord(dob=FieldValue(DateValue(1980, 3, 15), provenance_rank=10))
    comps = {c.field: c for c in field_comparisons(strong, weak)}
    assert comps["dob"].provenance_rank == 10


def test_absent_field_grades_insufficient_data():
    empty = CandidateRecord()
    comps = {c.field: c for c in field_comparisons(empty, _rec())}
    assert comps["dob"].level is AgreementLevel.INSUFFICIENT_DATA
    assert comps["name"].level is AgreementLevel.INSUFFICIENT_DATA


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


# --- FieldSpec refuses unnameable callables (issue #100 review) ---------------------------
# matcher_version fingerprints a FieldSpec's comparator AND extractor by module-qualified
# name — the ADR-0011/0029 recall key. A lambda, closure, or functools.partial has no
# stable name (every module-level lambda is "<lambda>"; closures from one factory share a
# qualname; partial has none at all), so two DIFFERENT configs could pin identically — a
# silent false merge of the recall key — or crash at payload-build time deep inside a
# sweep. The spec therefore refuses them at CONSTRUCTION, where the config author sees it
# (the Thresholds #211 pattern).


def _named_extractor(rec):
    return (None, 0)


def test_field_spec_refuses_a_lambda_extractor():
    with pytest.raises(ValueError, match="stable"):
        FieldSpec("dob", compare_dob, lambda r: (None, 0))


def test_field_spec_refuses_a_partial_comparator():
    with pytest.raises(ValueError, match="stable"):
        FieldSpec("dob", functools.partial(compare_dob), _named_extractor)


def test_field_spec_refuses_a_closure_comparator():
    def _make():
        def _cmp(a, b, ctx):
            return AgreementLevel.EXACT
        return _cmp

    with pytest.raises(ValueError, match="stable"):
        FieldSpec("dob", _make(), _named_extractor)


def test_field_spec_accepts_module_level_named_functions():
    # The shipped configuration style: plain module-level functions carry stable names.
    spec = FieldSpec("dob", compare_dob, _named_extractor)
    assert spec.field == "dob"
