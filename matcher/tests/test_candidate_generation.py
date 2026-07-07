# matcher/tests/test_candidate_generation.py
"""Integration tests for db.generate_candidate_pairs (blocking).

Seed patient_* projection rows directly, then assert which canonical pairs the four
blocking passes (identifier / exact-DOB / name-token / name-token+birth-year) generate.
Gated on CAIRN_TEST_PG.
"""

import uuid

from cairn_matcher.pipeline.runner import canonical_pair
from tests.conftest import seed_patient

PA = "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa"
PB = "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb"
PC = "cccccccc-cccc-cccc-cccc-cccccccccccc"


def _pairs(conn, **kw):
    from cairn_matcher.pipeline.db import generate_candidate_pairs
    pairs, _skipped = generate_candidate_pairs(conn, **kw)
    return pairs


def _gen(conn, **kw):
    from cairn_matcher.pipeline.db import generate_candidate_pairs
    return generate_candidate_pairs(conn, **kw)


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
        assert uuid.UUID(low) < uuid.UUID(high)


PD = "dddddddd-dddd-dddd-dddd-dddddddddddd"


def test_oversized_block_is_skipped_and_reported(pg_conn):
    # cap=2: three patients share one DOB -> group size 3 > 2 -> skipped, no pairs from it.
    for p in (PA, PB, PC):
        seed_patient(pg_conn, p, dob=("1980-07-15", 20))
    pairs, skipped = _gen(pg_conn, max_block_size=2)
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
    pairs, skipped = _gen(pg_conn, max_block_size=2)
    assert canonical_pair(PA, PD) in pairs
    assert any(pn == "dob" and sz == 3 for pn, _key, sz in skipped)


def test_name_year_rescues_pair_from_oversized_name_block(pg_conn):
    # Three patients share the name token "smith" -> the single-token 'name' block is
    # size 3. At cap=2 that block is oversized and skipped today, dropping every pair in
    # it. PA and PB also share a birth-year (1980) but NOT an exact DOB, so only the new
    # 'name+year' compound pass can rescue their pair.
    seed_patient(pg_conn, PA, dob=("1980-01-01", 20), names=[("Smith", 20)])
    seed_patient(pg_conn, PB, dob=("1980-06-06", 20), names=[("Smith", 20)])
    seed_patient(pg_conn, PC, dob=("1991-01-01", 20), names=[("Smith", 20)])
    pairs, skipped = _gen(pg_conn, max_block_size=2)
    # The oversized single-token block is still reported as skipped...
    assert any(pn == "name" and sz == 3 for pn, _key, sz in skipped)
    # ...but the same-year sub-block (smith|1980) survives and yields PA-PB.
    assert canonical_pair(PA, PB) in pairs
    # The different-year patient (PC, 1991) is alone in its sub-block -> no pair with it.
    assert canonical_pair(PA, PC) not in pairs
    assert canonical_pair(PB, PC) not in pairs


def test_name_year_honest_degrade_no_recall_regression(pg_conn):
    # PB has no DOB, so it cannot join the 'name+year' pass. The shared "jones" token must
    # still group PA-PB via the single-token 'name' pass -> coverage never regresses for a
    # record with a missing DOB. (A value with no 4-digit run, e.g. a 2-digit year
    # "07/15/80", fails the `[0-9]{4}` guard identically and degrades the same way.)
    seed_patient(pg_conn, PA, dob=("1985-03-03", 20), names=[("Jones", 20)])
    seed_patient(pg_conn, PB, names=[("Jones", 20)], identifiers=[("mrn:a", "2", "2")])
    assert canonical_pair(PA, PB) in _pairs(pg_conn)


def test_name_year_rescues_precision_mismatched_dob(pg_conn):
    # Year-precision "1990" vs day-precision "1990-05-12": the first 4-digit run is "1990"
    # for both, so they share the 'name|1990' sub-block -- though exact-DOB never groups them.
    # A different-year decoy (PC) oversizes the single "garcia" token block at cap=2, so only
    # the compound pass can produce PA-PB.
    seed_patient(pg_conn, PA, dob=("1990", 20, "year"), names=[("Garcia", 20)])
    seed_patient(pg_conn, PB, dob=("1990-05-12", 20, "day"), names=[("Garcia", 20)])
    seed_patient(pg_conn, PC, dob=("2000-01-01", 20), names=[("Garcia", 20)])
    pairs, skipped = _gen(pg_conn, max_block_size=2)
    assert any(pn == "name" and sz == 3 for pn, _key, sz in skipped)
    assert canonical_pair(PA, PB) in pairs


def test_name_year_rescues_cross_format_dob(pg_conn):
    # The same person stored two ways: ISO "1990-05-12" (Cairn-native) and day-first
    # "12/05/1990" (a FHIR/legacy import). The year (1990) is NOT leading in the second
    # value, so the old `left(value,4)` + `^[0-9]{4}` guard gave them different keys and
    # never grouped them. Extracting the first 4-digit RUN yields "1990" for both, so the
    # 'name+year' compound pass rescues PA-PB. A different-year decoy (PC) oversizes the
    # single "okafor" token block at cap=2, so only the compound pass can produce the pair.
    seed_patient(pg_conn, PA, dob=("1990-05-12", 20), names=[("Okafor", 20)])
    seed_patient(pg_conn, PB, dob=("12/05/1990", 20), names=[("Okafor", 20)])
    seed_patient(pg_conn, PC, dob=("2000-01-01", 20), names=[("Okafor", 20)])
    pairs, skipped = _gen(pg_conn, max_block_size=2)
    assert any(pn == "name" and sz == 3 for pn, _key, sz in skipped)
    assert canonical_pair(PA, PB) in pairs


def test_name_and_name_year_pair_is_emitted_once(pg_conn):
    # PA and PB share BOTH a name token and a birth-year, so the 'name' and 'name+year'
    # passes both surface the pair. After canonical-pair dedup it appears exactly once.
    seed_patient(pg_conn, PA, dob=("1975-08-08", 20), names=[("Patel", 20)])
    seed_patient(pg_conn, PB, dob=("1975-08-08", 20), names=[("Patel", 20)])
    pairs = _pairs(pg_conn)
    assert pairs.count(canonical_pair(PA, PB)) == 1


# ---------------------------------------------------------------------------
# A/B pass-toggle (enabled_passes): measurement tooling for pass changes.
# ---------------------------------------------------------------------------


def test_toggle_disabling_dob_removes_only_that_pass(pg_conn):
    # PA-PB share an exact DOB; PA-PC share an identifier. Disabling 'dob' must drop
    # the DOB pair while the identifier pair still comes through -- the toggle selects
    # passes, it does not short-circuit the run.
    seed_patient(pg_conn, PA, dob=("1980-07-15", 20), identifiers=[("mrn:a", "7", "7")])
    seed_patient(pg_conn, PB, dob=("1980-07-15", 20))
    seed_patient(pg_conn, PC, identifiers=[("mrn:a", "7", "7")])
    pairs = _pairs(pg_conn, enabled_passes={"identifier", "name", "name+year"})
    assert canonical_pair(PA, PC) in pairs
    assert canonical_pair(PA, PB) not in pairs


def test_toggle_default_none_equals_all_passes(pg_conn):
    # Regression pin: passing nothing and passing every pass name are the same run.
    from cairn_matcher.pipeline.blocking import ALL_PASSES
    seed_patient(pg_conn, PA, dob=("1980-07-15", 20), names=[("Alex Smith", 20)])
    seed_patient(pg_conn, PB, dob=("1980-07-15", 20), names=[("Alex Jones", 20)])
    assert _pairs(pg_conn) == _pairs(pg_conn, enabled_passes=set(ALL_PASSES))


def test_toggle_unknown_pass_name_raises(pg_conn):
    import pytest
    from cairn_matcher.pipeline.db import generate_candidate_pairs
    with pytest.raises(ValueError, match="no-such-pass"):
        generate_candidate_pairs(pg_conn, enabled_passes={"no-such-pass"})


def test_dob_first_initial_rescues_pair_with_no_shared_name_token(pg_conn):
    # PA "Jon" and PB "John" share NO full name token (distinct tokens) and NO exact DOB,
    # but share birth-year 1990 and first initial 'j'. Only dob+first-initial can group
    # them. A different-year decoy (PC, also 'j') keeps this honest: PC shares the initial
    # but not the year, so it must NOT pair with PA/PB via this pass.
    seed_patient(pg_conn, PA, dob=("1990-01-01", 20), names=[("Jon", 20)])
    seed_patient(pg_conn, PB, dob=("1990-12-31", 20), names=[("John", 20)])
    seed_patient(pg_conn, PC, dob=("1970-01-01", 20), names=[("Jane", 20)])
    # Without the new pass (name/name+year/dob need a shared token or exact dob): no pair.
    without = _pairs(pg_conn, enabled_passes={"dob", "name", "name+year"})
    assert canonical_pair(PA, PB) not in without
    # With it: PA-PB rescued; the different-year PC is not pulled in.
    with_pass = _pairs(pg_conn, enabled_passes={"dob", "name", "name+year", "dob+first-initial"})
    assert canonical_pair(PA, PB) in with_pass
    assert canonical_pair(PA, PC) not in with_pass
    assert canonical_pair(PB, PC) not in with_pass


def test_dob_first_initial_excludes_year_range_dob(pg_conn):
    # A §5.4 estimated-age chart (year-range precision) must NOT contribute a birth-YEAR to
    # this point-year pass -- the anchored dob-range passes own ranges. PA (range 1988/1992)
    # and PB (point 1990) share initial 'k' and overlapping years, but dob+first-initial is
    # a POINT-year pass: PA has no point year, so this pass does not pair them.
    seed_patient(pg_conn, PA, dob=("1988/1992", 20, "year-range"), names=[("Kim", 20)])
    seed_patient(pg_conn, PB, dob=("1990-05-05", 20), names=[("Kayla", 20)])
    pairs = _pairs(pg_conn, enabled_passes={"dob+first-initial"})
    assert canonical_pair(PA, PB) not in pairs


def test_name_sex_rescues_oversized_unisex_name_block(pg_conn):
    # A heavily unisex token "sasha" is shared by three charts -> the single-token 'name'
    # block is size 3 and, at cap=2, is skipped wholesale (every pair in it dropped). PA and
    # PB share administrative-sex 'female'; PC is 'male'. Only name+sex can rescue PA-PB by
    # splitting the block on sex. (This is the capped-only benefit the uncapped aggregate
    # blocking-recall metric cannot show -- hence a direct test.)
    seed_patient(pg_conn, PA, admin_sex=("female", 20), names=[("Sasha", 20)])
    seed_patient(pg_conn, PB, admin_sex=("female", 20), names=[("Sasha", 20)])
    seed_patient(pg_conn, PC, admin_sex=("male", 20), names=[("Sasha", 20)])
    pairs, skipped = _gen(pg_conn, max_block_size=2)
    # The oversized single-token 'name' block is still reported skipped...
    assert any(pn == "name" and sz == 3 for pn, _key, sz in skipped)
    # ...but the same-sex sub-block (sasha|female) survives and yields PA-PB.
    assert canonical_pair(PA, PB) in pairs
    # The opposite-sex PC is alone in its sub-block -> no pair with it via name+sex.
    assert canonical_pair(PA, PC) not in pairs
    assert canonical_pair(PB, PC) not in pairs


def test_name_sex_uses_union_of_both_sex_facets(pg_conn):
    # blocking_sex is the UNION of sex-at-birth and administrative-sex (recall-first): a
    # chart recording only sex-at-birth and one recording only administrative-sex still
    # share a sex value. PA (sex-at-birth 'female') and PB (administrative-sex 'female')
    # share token "ari"; a male decoy PC oversizes the 'name' block at cap=2.
    seed_patient(pg_conn, PA, sex=("female", 20), names=[("Ari", 20)])
    seed_patient(pg_conn, PB, admin_sex=("female", 20), names=[("Ari", 20)])
    seed_patient(pg_conn, PC, admin_sex=("male", 20), names=[("Ari", 20)])
    pairs, skipped = _gen(pg_conn, max_block_size=2)
    assert any(pn == "name" and sz == 3 for pn, _key, sz in skipped)
    assert canonical_pair(PA, PB) in pairs


def test_name_sex_ignores_unknown_sentinel_sex(pg_conn):
    # principle 4: two charts that BOTH merely recorded sex 'unknown' must NOT be grouped by
    # mutual ignorance. "unknown" is in adapter.VALUE_SENTINELS, so the sasha|unknown
    # sub-block never forms; with the single-token 'name' block skipped at cap=2, no pair.
    seed_patient(pg_conn, PA, admin_sex=("unknown", 20), names=[("Sasha", 20)])
    seed_patient(pg_conn, PB, admin_sex=("unknown", 20), names=[("Sasha", 20)])
    seed_patient(pg_conn, PC, admin_sex=("male", 20), names=[("Sasha", 20)])
    pairs, _skipped = _gen(pg_conn, max_block_size=2)
    assert canonical_pair(PA, PB) not in pairs
