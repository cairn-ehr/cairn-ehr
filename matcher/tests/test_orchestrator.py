from cairn_matcher.agreement import AgreementLevel
from cairn_matcher.orchestrator import DEFAULT_CONFIG, field_comparisons
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
    assert comps["sex-at-birth"].level is AgreementLevel.EXACT
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
