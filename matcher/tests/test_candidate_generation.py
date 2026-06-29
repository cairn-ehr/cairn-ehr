# matcher/tests/test_candidate_generation.py
"""Integration tests for db.generate_candidate_pairs (blocking).

Seed patient_* projection rows directly, then assert which canonical pairs the three
blocking passes (identifier / exact-DOB / name-token) generate. Gated on CAIRN_TEST_PG.
"""

from cairn_matcher.pipeline.runner import canonical_pair
from tests.conftest import seed_patient

PA = "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa"
PB = "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb"
PC = "cccccccc-cccc-cccc-cccc-cccccccccccc"


def _pairs(conn, **kw):
    from cairn_matcher.pipeline.db import generate_candidate_pairs
    pairs, _skipped = generate_candidate_pairs(conn, **kw)
    return pairs


def test_shared_identifier_generates_the_pair(pg_conn):
    seed_patient(pg_conn, PA, identifiers=[("mrn:a", "111", "111")])
    seed_patient(pg_conn, PB, identifiers=[("mrn:a", "111", "111")])
    assert canonical_pair(PA, PB) in _pairs(pg_conn)


def test_shared_name_token_generates_the_pair(pg_conn):
    # Only a shared token "alex"; distinct identifiers, no DOB.
    seed_patient(pg_conn, PA, names=[("Alex Smith", 20)], identifiers=[("mrn:a", "1", "1")])
    seed_patient(pg_conn, PB, names=[("Alex Jones", 20)], identifiers=[("mrn:a", "2", "2")])
    assert canonical_pair(PA, PB) in _pairs(pg_conn)


def test_shared_exact_dob_generates_the_pair(pg_conn):
    seed_patient(pg_conn, PA, dob=("1980-07-15", 20))
    seed_patient(pg_conn, PB, dob=("1980-07-15", 20))
    assert canonical_pair(PA, PB) in _pairs(pg_conn)


def test_no_shared_block_does_not_generate(pg_conn):
    seed_patient(pg_conn, PA, dob=("1980-07-15", 20), names=[("Alex Smith", 20)],
                 identifiers=[("mrn:a", "1", "1")])
    seed_patient(pg_conn, PB, dob=("1991-02-02", 20), names=[("Robin Jones", 20)],
                 identifiers=[("mrn:a", "2", "2")])
    assert canonical_pair(PA, PB) not in _pairs(pg_conn)


def test_pair_sharing_two_keys_is_emitted_once(pg_conn):
    # Same identifier AND same DOB -> two passes hit -> still one row after DISTINCT.
    for p in (PA, PB):
        seed_patient(pg_conn, p, dob=("1980-07-15", 20), identifiers=[("mrn:a", "9", "9")])
    pairs = _pairs(pg_conn)
    assert pairs.count(canonical_pair(PA, PB)) == 1


def test_unknown_system_never_blocks(pg_conn):
    seed_patient(pg_conn, PA, identifiers=[("unknown", "x", "x")])
    seed_patient(pg_conn, PB, identifiers=[("unknown", "x", "x")])
    assert canonical_pair(PA, PB) not in _pairs(pg_conn)


def test_pairs_are_canonical_and_self_excluded(pg_conn):
    # Three patients all sharing one identifier -> C(3,2)=3 pairs, all low<high, none self.
    for p in (PA, PB, PC):
        seed_patient(pg_conn, p, identifiers=[("mrn:a", "7", "7")])
    pairs = _pairs(pg_conn)
    assert len(pairs) == 3
    for low, high in pairs:
        assert low < high


PD = "dddddddd-dddd-dddd-dddd-dddddddddddd"


def test_oversized_block_is_skipped_and_reported(pg_conn):
    # cap=2: three patients share one DOB -> group size 3 > 2 -> skipped, no pairs from it.
    for p in (PA, PB, PC):
        seed_patient(pg_conn, p, dob=("1980-07-15", 20))
    pairs, skipped = __import__(
        "cairn_matcher.pipeline.db", fromlist=["generate_candidate_pairs"]
    ).generate_candidate_pairs(pg_conn, max_block_size=2)
    assert pairs == []
    assert any(pn == "dob" and sz == 3 for pn, _key, sz in skipped)


def test_cap_is_per_group_not_global(pg_conn):
    # An oversized DOB block (PA,PB,PC) is skipped, but an in-cap identifier block
    # (PA,PD) in the SAME run is still generated.
    for p in (PA, PB, PC):
        seed_patient(pg_conn, p, dob=("1980-07-15", 20))
    seed_patient(pg_conn, PD)
    with pg_conn.cursor() as cur:
        cur.execute("INSERT INTO patient_identifier (patient_id, system, match_key, value, "
                    "normalized, profile, use_type, provenance, asserted_hlc_wall, "
                    "asserted_hlc_count, asserted_origin) VALUES "
                    "(%s,'mrn:a','55','55','55',NULL,NULL,'seed',0,0,'seed'),"
                    "(%s,'mrn:a','55','55','55',NULL,NULL,'seed',0,0,'seed')", (PA, PD))
    pg_conn.commit()
    pairs, skipped = __import__(
        "cairn_matcher.pipeline.db", fromlist=["generate_candidate_pairs"]
    ).generate_candidate_pairs(pg_conn, max_block_size=2)
    assert canonical_pair(PA, PD) in pairs
    assert any(pn == "dob" and sz == 3 for pn, _key, sz in skipped)
