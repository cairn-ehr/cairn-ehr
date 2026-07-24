"""Pure tests for the eval dataset value types and loader."""

import pytest

from cairn_matcher.eval.dataset import (
    DatasetError,
    DatasetRecord,
    EntityCluster,
    LabelledDataset,
    load_dataset,
    record_to_candidate,
)

_MINIMAL = {
    "name": "tiny",
    "entities": [
        {"entity_id": "e1", "records": [
            {
                "record_id": "r1",
                "dob": {"value": "1990-05-12", "precision": "day", "provenance_rank": 70},
            },
            {"record_id": "r2", "names": [{"value": "Alex Nguyen", "provenance_rank": 30}]},
        ]},
        {"entity_id": "e2", "records": [{"record_id": "r3"}]},
    ],
}


def test_load_dataset_builds_typed_tree():
    ds = load_dataset(_MINIMAL)
    assert isinstance(ds, LabelledDataset)
    assert ds.name == "tiny"
    assert len(ds.entities) == 2
    assert isinstance(ds.entities[0], EntityCluster)
    assert isinstance(ds.entities[0].records[0], DatasetRecord)
    assert ds.entities[0].records[0].record_id == "r1"
    assert ds.entities[0].records[0].dob == {
        "value": "1990-05-12",
        "precision": "day",
        "provenance_rank": 70,
    }


def test_all_records_flattens_in_order():
    ds = load_dataset(_MINIMAL)
    assert [r.record_id for r in ds.all_records()] == ["r1", "r2", "r3"]


def test_missing_record_id_raises():
    bad = {"name": "x", "entities": [{"entity_id": "e", "records": [{"dob": {}}]}]}
    with pytest.raises(DatasetError):
        load_dataset(bad)


def test_duplicate_record_id_raises():
    bad = {"name": "x", "entities": [
        {"entity_id": "e1", "records": [{"record_id": "dup"}]},
        {"entity_id": "e2", "records": [{"record_id": "dup"}]},
    ]}
    with pytest.raises(DatasetError):
        load_dataset(bad)


def test_missing_entities_key_raises():
    with pytest.raises(DatasetError):
        load_dataset({"name": "x"})


def test_name_without_value_raises_located_dataset_error():
    # A name dict missing "value" must fail loudly at load time (record_to_candidate /
    # the seeder index it directly) rather than as an opaque KeyError downstream.
    bad = {"name": "x", "entities": [{"entity_id": "e", "records": [
        {"record_id": "r1", "names": [{"provenance_rank": 30}]}]}]}
    with pytest.raises(DatasetError, match="r1"):
        load_dataset(bad)


def test_identifier_without_required_keys_raises():
    bad = {"name": "x", "entities": [{"entity_id": "e", "records": [
        {"record_id": "r1", "identifiers": [{"system": "mrn"}]}]}]}
    with pytest.raises(DatasetError, match="identifier"):
        load_dataset(bad)


def test_administrative_sex_loads_and_reaches_the_candidate():
    """The composite-sex fallback rides the REAL adapter: an admin-sex-only record
    must land on CandidateRecord.administrative_sex (slice D's field), not sex_at_birth."""
    ds = load_dataset({
        "entities": [{"entity_id": "e1", "records": [
            {"record_id": "r1",
             "administrative_sex": {"value": "male", "provenance_rank": 30}},
        ]}],
    })
    rec = ds.entities[0].records[0]
    assert rec.administrative_sex == {"value": "male", "provenance_rank": 30}
    cand = record_to_candidate(rec)
    assert cand.administrative_sex is not None
    assert cand.administrative_sex.value == "male"
    assert cand.administrative_sex.provenance_rank == 30
    assert cand.sex_at_birth is None


def test_admin_sex_absent_stays_none():
    ds = load_dataset({
        "entities": [{"entity_id": "e1", "records": [{"record_id": "r1"}]}],
    })
    assert ds.entities[0].records[0].administrative_sex is None
    assert record_to_candidate(ds.entities[0].records[0]).administrative_sex is None


def test_sab_vs_admin_pair_grades_sex_via_the_composite_fallback():
    """One chart carries only sex-at-birth, the other only administrative-sex — the
    §5.4 slice-D union fallback must produce a graded 'sex' comparison (EXACT here),
    proving the eval path exercises the composite the shipped scorer uses."""
    from cairn_matcher.agreement import AgreementLevel
    from cairn_matcher.orchestrator import DEFAULT_CONFIG, field_comparisons

    ds = load_dataset({
        "entities": [{"entity_id": "e1", "records": [
            {"record_id": "a", "sex_at_birth": {"value": "male", "provenance_rank": 40}},
            {"record_id": "b",
             "administrative_sex": {"value": "male", "provenance_rank": 30}},
        ]}],
    })
    a, b = (record_to_candidate(r) for r in ds.entities[0].records)
    by_field = {c.field: c for c in field_comparisons(a, b, DEFAULT_CONFIG)}
    assert by_field["sex"].level is AgreementLevel.EXACT


# --- repaired marker round-trip + quantify helper (issue #211 gap 4) ---------------------
# The generator's _repair injects a verbatim EXACT name into the hardest clones to keep them
# blockable. DatasetRecord must carry that marker through the loader (a bare dict key would be
# silently dropped by the fixed-field dataclass), and a pure helper must let eval consumers
# COUNT the repair-influenced records so held-out lift can be reported honestly.
from cairn_matcher.eval import dataset as _dataset  # noqa: E402


def test_dataset_record_repaired_defaults_false():
    # A record with no marker is not repair-influenced — the default must be a plain False.
    assert DatasetRecord(record_id="r").repaired is False


def test_load_dataset_preserves_repaired_marker():
    ds = load_dataset({
        "entities": [{"entity_id": "e1", "records": [
            {"record_id": "seed"},
            {"record_id": "clone", "repaired": True},
        ]}],
    })
    by_id = {r.record_id: r for e in ds.entities for r in e.records}
    assert by_id["clone"].repaired is True     # marker survived the loader
    assert by_id["seed"].repaired is False     # unmarked record stays False


def test_repaired_record_ids_enumerates_only_marked_records():
    ds = load_dataset({
        "entities": [
            {"entity_id": "e1", "records": [
                {"record_id": "s1"}, {"record_id": "c1", "repaired": True}]},
            {"entity_id": "e2", "records": [
                {"record_id": "s2"}, {"record_id": "c2"}]},
        ],
    })
    assert _dataset.repaired_record_ids(ds) == frozenset({"c1"})
