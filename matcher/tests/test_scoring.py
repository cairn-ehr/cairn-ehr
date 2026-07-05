import pytest

from cairn_matcher.agreement import AgreementLevel
from cairn_matcher.records import FieldComparison
from cairn_matcher.scoring import (
    DEFAULT_WEIGHTS,
    FieldWeights,
    MatchScore,
    Weights,
    provenance_factor,
    score,
)


def test_provenance_factor_floor_and_ceiling():
    assert provenance_factor(0) == pytest.approx(0.5)
    assert provenance_factor(70) == pytest.approx(1.0)
    assert provenance_factor(35) == pytest.approx(0.75)
    assert provenance_factor(999) == pytest.approx(1.0)  # clamped
    assert provenance_factor(-5) == pytest.approx(0.5)   # clamped


WEIGHTS = Weights(per_field={
    "dob": FieldWeights({AgreementLevel.EXACT: 8.0, AgreementLevel.DISAGREE: -4.0}),
})


def test_exact_agreement_scaled_by_provenance():
    # rank 70 -> factor 1.0 -> 8.0 * 1.0
    comps = [FieldComparison("dob", AgreementLevel.EXACT, 70)]
    assert score(comps, WEIGHTS).total == pytest.approx(8.0)
    # rank 0 -> factor 0.5 -> 8.0 * 0.5
    comps = [FieldComparison("dob", AgreementLevel.EXACT, 0)]
    assert score(comps, WEIGHTS).total == pytest.approx(4.0)


def test_disagree_contributes_negative():
    comps = [FieldComparison("dob", AgreementLevel.DISAGREE, 70)]
    assert score(comps, WEIGHTS).total == pytest.approx(-4.0)


def test_insufficient_data_contributes_zero():
    comps = [FieldComparison("dob", AgreementLevel.INSUFFICIENT_DATA, 70)]
    s = score(comps, WEIGHTS)
    assert s.total == pytest.approx(0.0)
    assert s.fields[0].weight_contribution == pytest.approx(0.0)


def test_unknown_level_contributes_zero():
    # A field IN the table but at a level the table doesn't price is legitimate zero
    # evidence (e.g. identifier defines only EXACT — its comparator is positive-only).
    comps = [FieldComparison("dob", AgreementLevel.PARTIAL, 70)]
    assert score(comps, WEIGHTS).total == pytest.approx(0.0)


def test_unknown_field_raises_instead_of_silently_zeroing():
    # A field the table has NO entry for is a config mismatch (e.g. a stale weights
    # table keyed 'sex-at-birth' after the rename to 'sex'): silently zeroing a
    # verified DISAGREE would be a precise untruth, so score() must fail loudly.
    comps = [FieldComparison("unmapped", AgreementLevel.EXACT, 70)]
    with pytest.raises(ValueError, match="unmapped"):
        score(comps, WEIGHTS)


def test_unknown_field_with_insufficient_data_does_not_raise():
    # INSUFFICIENT_DATA never consults the table (it is zero by principle §3.7), so an
    # unweighted field that carries no evidence must not fail the whole comparison.
    comps = [FieldComparison("unmapped", AgreementLevel.INSUFFICIENT_DATA, 70)]
    assert score(comps, WEIGHTS).total == pytest.approx(0.0)


def test_per_field_contributions_sum_to_total():
    comps = [
        FieldComparison("dob", AgreementLevel.EXACT, 70),
        FieldComparison("dob", AgreementLevel.DISAGREE, 70),
    ]
    s = score(comps, WEIGHTS)
    assert s.total == pytest.approx(sum(f.weight_contribution for f in s.fields))


def test_default_weights_cover_the_default_fields():
    for fld in ("dob", "sex", "name", "identifier"):
        assert fld in DEFAULT_WEIGHTS.per_field


def test_default_weights_carry_sex_key_not_sex_at_birth():
    # The weight/FieldSpec key renamed to "sex" (composite field); 'sex-at-birth'
    # remains ONLY as the projection field name in SQL/seeding, never a weight key.
    assert "sex" in DEFAULT_WEIGHTS.per_field
    assert "sex-at-birth" not in DEFAULT_WEIGHTS.per_field


def test_match_score_is_returned():
    assert isinstance(score([], WEIGHTS), MatchScore)
    assert score([], WEIGHTS).total == 0.0


def test_default_weights_tables_are_immutable():
    # DEFAULT_WEIGHTS is a process-wide singleton. frozen=True only blocks attribute
    # rebinding, not mutation of the wrapped dicts — so without an immutable wrapper a
    # caller (e.g. a B3 locale tweak) could silently poison scoring for every other
    # record/tenant in the process. Both layers must reject mutation.
    with pytest.raises(TypeError):
        DEFAULT_WEIGHTS.per_field["dob"] = None  # outer table
    with pytest.raises(TypeError):
        DEFAULT_WEIGHTS.per_field["dob"].weights[AgreementLevel.EXACT] = 999.0  # inner

    # A freshly constructed Weights/FieldWeights from a plain dict is frozen too.
    fw = FieldWeights({AgreementLevel.EXACT: 1.0})
    with pytest.raises(TypeError):
        fw.weights[AgreementLevel.EXACT] = 2.0
