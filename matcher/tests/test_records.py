from cairn_matcher.agreement import AgreementLevel
from cairn_matcher.records import (
    CandidateRecord,
    DateValue,
    FieldComparison,
    FieldValue,
    MatcherTypeError,
    Name,
)


def test_records_construct_and_are_frozen():
    dob = FieldValue(DateValue(1980, 3, 15), provenance_rank=60)
    name = Name(tokens={"given": ("alex",), "family": ("kim",)})
    rec = CandidateRecord(
        dob=dob,
        names=FieldValue(frozenset({name})),
        identifiers={"au-medicare": frozenset({"2951"})},
    )
    assert rec.dob.value == DateValue(1980, 3, 15)
    assert rec.dob.provenance_rank == 60
    assert next(iter(rec.names.value)).tokens["family"] == ("kim",)


def test_field_value_defaults_provenance_zero_and_identifiers_default_empty():
    assert FieldValue("x").provenance_rank == 0
    assert CandidateRecord().identifiers == {}
    assert CandidateRecord().dob is None


def test_field_comparison_carries_level_and_rank():
    fc = FieldComparison(field="dob", level=AgreementLevel.EXACT, provenance_rank=60)
    assert fc.field == "dob"
    assert fc.level is AgreementLevel.EXACT


def test_matcher_type_error_is_a_type_error():
    assert issubclass(MatcherTypeError, TypeError)
