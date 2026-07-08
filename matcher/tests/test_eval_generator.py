"""Tests for the synthetic blocking-eval dataset generator (pure, stdlib-only)."""

import copy
import itertools
import random

from cairn_matcher.eval.dataset import load_dataset, truth_pairs
from cairn_matcher.eval.generator import (
    GenSpec,
    corrupt_dob_format,
    corrupt_dob_typo,
    corrupt_identifier,
    corrupt_name,
    generate_dataset,
    name_tokens,
    shares_blocking_key,
    synth_seed,
)


def test_name_tokens_lowercases_and_splits_all_names():
    rec = {"names": [{"value": "Alex Nguyen"}, {"value": "NGUYEN Van Alex"}]}
    assert name_tokens(rec) == {"alex", "nguyen", "van"}


def test_name_tokens_empty_when_no_names():
    assert name_tokens({"record_id": "r"}) == set()


def test_name_tokens_excludes_placeholder_use_names():
    # §5.4 / issue #124: the real name_tokens CTE excludes placeholder-use (callsign) names
    # (`use_key <> ALL(PLACEHOLDER_NAME_USES)`); this pure mirror must agree, or the eval's
    # "recoverable by blocking" guarantee would over-claim for a callsign-carrying record.
    # A name carrying a placeholder `use`/`use_key` contributes ZERO tokens.
    rec = {"names": [{"value": "Unknown-ed-site1-2026-07-03-00ab", "use_key": "callsign"}]}
    assert name_tokens(rec) == set()
    # ... but a real name on the SAME record still tokenizes (exclusion is placeholder-only).
    rec2 = {
        "names": [
            {"value": "Alex Nguyen"},
            {"value": "Unknown-ed-site1-2026-07-03-00ab", "use": "callsign"},
        ]
    }
    assert name_tokens(rec2) == {"alex", "nguyen"}


def test_shares_key_via_exact_dob():
    a = {"dob": {"value": "1990-05-12"}, "names": [{"value": "Ann"}]}
    b = {"dob": {"value": "1990-05-12"}, "names": [{"value": "Bob"}]}
    assert shares_blocking_key(a, b) is True


def test_shares_key_via_name_token():
    a = {"dob": {"value": "1990-05-12"}, "names": [{"value": "Alex Nguyen"}]}
    b = {"dob": {"value": "1985-01-01"}, "names": [{"value": "Sam Nguyen"}]}
    assert shares_blocking_key(a, b) is True


def test_shares_key_via_identifier_but_not_unknown():
    a = {"identifiers": [{"system": "au-medicare", "match_key": "111"}]}
    b = {"identifiers": [{"system": "au-medicare", "match_key": "111"}]}
    assert shares_blocking_key(a, b) is True
    a_unk = {"identifiers": [{"system": "unknown", "match_key": "111"}]}
    b_unk = {"identifiers": [{"system": "unknown", "match_key": "111"}]}
    assert shares_blocking_key(a_unk, b_unk) is False


def test_no_shared_key_is_false():
    a = {"dob": {"value": "1990-05-12"}, "names": [{"value": "Alex Nguyen"}],
         "identifiers": [{"system": "au-medicare", "match_key": "111"}]}
    b = {"dob": {"value": "12/05/1990"}, "names": [{"value": "Sam Smith"}],
         "identifiers": [{"system": "au-medicare", "match_key": "222"}]}
    assert shares_blocking_key(a, b) is False


def _seed_rec():
    return {
        "record_id": "e0-seed",
        "dob": {"value": "1990-05-12", "precision": "day", "provenance_rank": 40},
        "names": [{"value": "Alex Nguyen", "provenance_rank": 30}],
        "identifiers": [{"system": "au-medicare", "match_key": "12345", "value": "12345"}],
    }


def test_dob_format_keeps_birth_year_changes_value():
    rec = _seed_rec()
    before = copy.deepcopy(rec)
    out = corrupt_dob_format(rec, random.Random(1))
    assert rec == before                       # input unmutated (pure)
    assert out["dob"]["value"] != "1990-05-12" # exact value changed
    assert "1990" in out["dob"]["value"]       # birth-year preserved


def test_dob_typo_changes_value():
    out = corrupt_dob_typo(_seed_rec(), random.Random(2))
    assert out["dob"]["value"] != "1990-05-12"


def test_name_corruption_changes_a_name_value():
    out = corrupt_name(_seed_rec(), random.Random(3))
    assert [n["value"] for n in out["names"]] != ["Alex Nguyen"]


def test_identifier_corruption_drops_or_mistypes():
    out = corrupt_identifier(_seed_rec(), random.Random(4))
    ids = out["identifiers"]
    # either dropped (fewer) or the match_key changed
    assert ids == [] or ids[0]["match_key"] != "12345"


def test_operators_are_noops_when_field_absent():
    bare = {"record_id": "x", "names": [{"value": "Sam"}]}
    r = random.Random(5)
    assert corrupt_dob_format(bare, r) == bare
    assert corrupt_dob_typo(bare, r) == bare
    assert corrupt_identifier(bare, r) == bare
    no_names = {
        "record_id": "y",
        "dob": {"value": "1990-01-01", "precision": "day", "provenance_rank": 30},
    }
    assert corrupt_name(no_names, r) == no_names


def test_synth_seed_is_deterministic_for_same_rng_stream():
    a = synth_seed(random.Random(7), 0)
    b = synth_seed(random.Random(7), 0)
    assert a == b


def test_synth_seed_has_required_shape():
    rec = synth_seed(random.Random(8), 3)
    assert rec["record_id"] == "e3-seed"
    assert rec["names"] and rec["names"][0]["value"].strip()
    assert rec["dob"]["value"].count("-") == 2          # full ISO
    assert rec["dob"]["precision"] == "day"


def test_synth_seed_spans_multiple_name_shapes_across_indices():
    shapes = {len(synth_seed(random.Random(i), i)["names"][0]["value"].split())
              for i in range(40)}
    assert 1 in shapes            # at least one mononym
    assert any(s >= 2 for s in shapes)   # and multi-token names


def test_generate_is_deterministic_for_same_seed():
    spec = GenSpec(seed=42, n_entities=25)
    assert generate_dataset(spec) == generate_dataset(spec)


def test_generate_differs_for_different_seed():
    assert generate_dataset(GenSpec(seed=1, n_entities=25)) != \
           generate_dataset(GenSpec(seed=2, n_entities=25))


def test_output_round_trips_through_real_loader():
    ds = load_dataset(generate_dataset(GenSpec(seed=3, n_entities=30)))
    assert len(ds.entities) == 30
    assert all(len(e.records) == 2 for e in ds.entities)   # seed + one clone
    ids = [r.record_id for e in ds.entities for r in e.records]
    assert len(ids) == len(set(ids))                       # unique record_ids
    assert len(truth_pairs(ds)) == 30                      # one true pair per entity


def test_every_true_pair_shares_a_blocking_key():
    # The recoverability invariant: every within-cluster (seed, clone) pair is blockable.
    ds_dict = generate_dataset(GenSpec(seed=4, n_entities=50))
    for ent in ds_dict["entities"]:
        for a, b in itertools.combinations(ent["records"], 2):
            assert shares_blocking_key(a, b), f"unrecoverable pair in {ent['entity_id']}"
