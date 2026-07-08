"""Whole-pipeline property tests: the principle-bearing invariants, end to end."""

import random

import pytest

import cairn_matcher as cm
from cairn_matcher.orchestrator import field_comparisons
from cairn_matcher.records import CandidateRecord, DateValue, FieldValue, Name
from cairn_matcher.scoring import score


def n(given, family):
    return Name(tokens={"given": tuple(given), "family": tuple(family)})


def _full_record():
    return CandidateRecord(
        dob=FieldValue(DateValue(1980, 3, 15), provenance_rank=60),
        sex_at_birth=FieldValue("female", provenance_rank=60),
        names=FieldValue(frozenset({n(["alex"], ["kim"])}), provenance_rank=20),
        identifiers={"au-medicare": frozenset({"2951"})},
    )


def _score_of(a, b):
    return score(field_comparisons(a, b)).total


def _record_with_names(given_tokens, family_tokens="smith"):
    """Build a minimal record with the given name token bag, same other fields."""
    return CandidateRecord(
        dob=FieldValue(DateValue(1980, 3, 15), provenance_rank=60),
        sex_at_birth=FieldValue("male", provenance_rank=60),
        names=FieldValue(frozenset({n(given_tokens, [family_tokens])}), provenance_rank=20),
    )


def test_score_is_symmetric():
    # Regression: identical records (the original trivial case) must still be symmetric.
    a, b = _full_record(), _full_record()
    assert _score_of(a, b) == _score_of(b, a)


def test_score_is_symmetric_heterogeneous_names():
    # C1 regression: heterogeneous name token bags that expose the greedy-pairing
    # asymmetry.  A has ("jon","jonn"), B has ("john","jon").  Because the greedy
    # algorithm pairs from A's perspective first, score(A,B) != score(B,A) unless the
    # implementation evaluates both traversal directions and takes the best.
    a = _record_with_names(["jon", "jonn"])
    b = _record_with_names(["john", "jon"])
    assert _score_of(a, b) == pytest.approx(_score_of(b, a))


def test_score_is_symmetric_randomized():
    # Sweep 200 random record pairs built from a small token pool.  Seeded for
    # determinism — this is a pure unit test with no I/O.  Any asymmetry here is a
    # sign-flip bug in the greedy pairing.
    TOKEN_POOL = ["jon", "john", "jonn", "jane", "jan", "smith", "smyth", "jones"]
    rng = random.Random(42)

    def random_record():
        # Pick 2 distinct tokens from the pool to form a 2-token given name.
        tokens = rng.sample(TOKEN_POOL, 2)
        family = rng.choice(TOKEN_POOL)
        return _record_with_names(tokens, family)

    for _ in range(200):
        a = random_record()
        b = random_record()
        assert _score_of(a, b) == pytest.approx(_score_of(b, a)), (
            f"Asymmetry: score(a,b)={_score_of(a, b)}, score(b,a)={_score_of(b, a)}\n"
            f"  a.names={a.names}, b.names={b.names}"
        )


def test_identical_strong_records_score_clearly_positive():
    a, b = _full_record(), _full_record()
    assert _score_of(a, b) > 0.0


def test_adding_a_missing_field_never_lowers_the_score():
    # The §3.7 invariant, end to end: a record that is absent a field must not be
    # penalised versus a record that simply never had that field considered.
    a = _full_record()
    full = _full_record()
    # b is identical to `full` but with DOB removed entirely (absent, not different).
    b_absent = CandidateRecord(
        sex_at_birth=full.sex_at_birth, names=full.names, identifiers=full.identifiers
    )
    score_with_absent_dob = _score_of(a, b_absent)
    score_full = _score_of(a, full)
    assert score_with_absent_dob <= score_full
    # And an absent field never makes the total go negative on an otherwise-matching pair.
    assert score_with_absent_dob > 0.0


def test_public_api_surface_is_importable():
    for name in (
        "CandidateRecord", "DateValue", "Name", "FieldValue",
        "AgreementLevel", "field_comparisons", "score", "MatchScore",
    ):
        assert hasattr(cm, name), name
