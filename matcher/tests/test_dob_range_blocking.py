"""Integration tests for the anchored birth-year-range blocking passes (§5.4).

A chart carrying a `year-range` dob (value "<min>/<max>", the clinician-observed
estimated-age window from slice B) anchors a block of every chart whose birth-year
window overlaps its own. Pairs are anchor-x-member ONLY (see pipeline/blocking.py for
why all-pairing would manufacture noise). Gated on CAIRN_TEST_PG.
"""

from cairn_matcher.pipeline.runner import canonical_pair
from tests.conftest import seed_patient

PA = "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa"   # the John Doe / range chart (anchor)
PB = "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb"
PC = "cccccccc-cccc-cccc-cccc-cccccccccccc"
PD = "dddddddd-dddd-dddd-dddd-dddddddddddd"


def _gen(conn, **kw):
    from cairn_matcher.pipeline.db import generate_candidate_pairs
    return generate_candidate_pairs(conn, **kw)


def _pairs(conn, **kw):
    pairs, _skipped = _gen(conn, **kw)
    return pairs


def test_point_dob_inside_window_pairs_with_the_range_chart(pg_conn):
    # The core §5.4 case: John Doe estimated ~40±5 (window 1981-1991); the prior chart
    # was born 1985. No shared name (callsign only, excluded), no shared identifier --
    # ONLY the range pass can surface this pair.
    seed_patient(pg_conn, PA, dob=("1981/1991", 30, "year-range"),
                 callsign_names=[("Unknown-ed-bay3-2026-07-04-aaaaaaaa", 10)])
    seed_patient(pg_conn, PB, dob=("1985-06-15", 20), names=[("Alex Smith", 20)])
    assert canonical_pair(PA, PB) in _pairs(pg_conn)


def test_point_dob_outside_window_does_not_pair(pg_conn):
    seed_patient(pg_conn, PA, dob=("1981/1991", 30, "year-range"))
    seed_patient(pg_conn, PB, dob=("1995-01-01", 20), names=[("Alex Smith", 20)])
    assert canonical_pair(PA, PB) not in _pairs(pg_conn)


def test_window_boundary_is_inclusive(pg_conn):
    # Mirrors compare_dob's inclusive interval semantics: born exactly at the window
    # max (1991) still groups.
    seed_patient(pg_conn, PA, dob=("1981/1991", 30, "year-range"))
    seed_patient(pg_conn, PB, dob=("1991-12-31", 20))
    assert canonical_pair(PA, PB) in _pairs(pg_conn)


def test_overlapping_ranges_pair_disjoint_do_not(pg_conn):
    # Two John Does at two sites (the two-callsigns case): overlapping windows are the
    # ONLY key such a pair can ever share. A third, disjoint window must not join.
    seed_patient(pg_conn, PA, dob=("1980/1990", 30, "year-range"))
    seed_patient(pg_conn, PB, dob=("1988/1995", 30, "year-range"))
    seed_patient(pg_conn, PC, dob=("2000/2005", 30, "year-range"))
    pairs = _pairs(pg_conn)
    assert canonical_pair(PA, PB) in pairs
    assert canonical_pair(PA, PC) not in pairs
    assert canonical_pair(PB, PC) not in pairs


def test_members_are_never_paired_with_each_other(pg_conn):
    # PB and PC are both inside PA's window but share nothing else. The anchored pass
    # must pair each with PA and NOT with each other (the all-pairs noise the design
    # rejects: being born within the same decade is not a signal).
    seed_patient(pg_conn, PA, dob=("1981/1991", 30, "year-range"))
    seed_patient(pg_conn, PB, dob=("1983-01-01", 20), names=[("Alex Smith", 20)])
    seed_patient(pg_conn, PC, dob=("1989-01-01", 20), names=[("Robin Jones", 20)])
    pairs = _pairs(pg_conn)
    assert canonical_pair(PA, PB) in pairs
    assert canonical_pair(PA, PC) in pairs
    assert canonical_pair(PB, PC) not in pairs


def test_malformed_range_values_degrade_silently(pg_conn):
    # Inverted ("1991/1981") and non-numeric ("about-forty") year-range values are
    # EXCLUDED (safe degrade mirroring parse_dob): no crash, no pairs, no false group.
    seed_patient(pg_conn, PA, dob=("1991/1981", 30, "year-range"))
    seed_patient(pg_conn, PB, dob=("about-forty", 30, "year-range"))
    seed_patient(pg_conn, PC, dob=("1985-06-15", 20))
    pairs = _pairs(pg_conn)
    assert canonical_pair(PA, PC) not in pairs
    assert canonical_pair(PB, PC) not in pairs


def test_oversized_range_block_is_skipped_and_reported(pg_conn):
    # cap=3: anchor + 3 in-window members = block size 4 > 3 -> the whole anchored
    # block is skipped and reported under the anchor's uuid, pairs withheld (the hub
    # sweep is the declared backstop). Size counts the ANCHOR TOO: it is the block.
    seed_patient(pg_conn, PA, dob=("1981/1991", 30, "year-range"))
    for p in (PB, PC, PD):
        seed_patient(pg_conn, p, dob=("1985-06-15", 20))
    pairs, skipped = _gen(pg_conn, max_block_size=3)
    assert canonical_pair(PA, PB) not in pairs
    assert any(pn == "dob-range" and key == PA and sz == 4 for pn, key, sz in skipped)


def test_toggle_disables_the_range_pass(pg_conn):
    seed_patient(pg_conn, PA, dob=("1981/1991", 30, "year-range"))
    seed_patient(pg_conn, PB, dob=("1985-06-15", 20))
    assert canonical_pair(PA, PB) in _pairs(pg_conn)
    off = _pairs(pg_conn, enabled_passes={"identifier", "dob", "name", "name+year"})
    assert canonical_pair(PA, PB) not in off


# ---------------------------------------------------------------------------
# The 'dob-range+sex' RESCUE arm: window overlap AND a shared blocking-sex value
# (union of sex-at-birth + administrative-sex). Fires within the cap where the plain
# window block is oversized -- mirroring the name / name+year rescue pattern.
# ---------------------------------------------------------------------------


def test_sex_rescues_pair_from_oversized_window_block(pg_conn):
    # cap=3. PA's window catches PB, PC, PD -> plain 'dob-range' block size 4 is
    # skipped (and reported). Only PB shares PA's observed sex, so the 'dob-range+sex'
    # sub-block is size 2 -- within cap -> the true candidate is rescued.
    seed_patient(pg_conn, PA, dob=("1981/1991", 30, "year-range"),
                 admin_sex=("male", 30))
    seed_patient(pg_conn, PB, dob=("1985-06-15", 20), sex=("male", 40))
    seed_patient(pg_conn, PC, dob=("1983-01-01", 20), sex=("female", 40))
    seed_patient(pg_conn, PD, dob=("1989-01-01", 20))          # no sex row at all
    pairs, skipped = _gen(pg_conn, max_block_size=3)
    assert any(pn == "dob-range" and key == PA and sz == 4 for pn, key, sz in skipped)
    assert canonical_pair(PA, PB) in pairs
    assert canonical_pair(PA, PC) not in pairs
    assert canonical_pair(PA, PD) not in pairs


def test_union_sex_groups_via_administrative_sex_only(pg_conn):
    # The trans case the like-for-like key would drop: the member's sex-at-birth
    # ('female') differs from the observation, but their administrative-sex ('male')
    # matches -- the UNION of both fields still groups the pair. cap=3 forces the
    # rescue arm to be the only path (plain window block: PA+PB+PC+PD = 4 > 3).
    seed_patient(pg_conn, PA, dob=("1981/1991", 30, "year-range"),
                 admin_sex=("male", 30))
    seed_patient(pg_conn, PB, dob=("1985-06-15", 20), sex=("female", 40),
                 admin_sex=("male", 30))
    seed_patient(pg_conn, PC, dob=("1983-01-01", 20), sex=("female", 40))
    seed_patient(pg_conn, PD, dob=("1989-01-01", 20))
    pairs, skipped = _gen(pg_conn, max_block_size=3)
    assert any(pn == "dob-range" and key == PA and sz == 4 for pn, key, sz in skipped)
    assert canonical_pair(PA, PB) in pairs


def test_sex_mismatch_never_suppresses_the_plain_pass(pg_conn):
    # Additive-only guarantee: with NO cap pressure, a pair whose sexes differ still
    # comes through the plain 'dob-range' pass -- the sex arm can only ADD, never veto.
    seed_patient(pg_conn, PA, dob=("1981/1991", 30, "year-range"),
                 admin_sex=("male", 30))
    seed_patient(pg_conn, PB, dob=("1985-06-15", 20), sex=("female", 40))
    assert canonical_pair(PA, PB) in _pairs(pg_conn)


def test_sex_arm_toggles_independently(pg_conn):
    # cap=3 as above: with 'dob-range+sex' disabled the rescue disappears; the plain
    # pass alone is still capped out -> no pair. Proves the two arms are independently
    # measurable (the A/B use case).
    seed_patient(pg_conn, PA, dob=("1981/1991", 30, "year-range"),
                 admin_sex=("male", 30))
    seed_patient(pg_conn, PB, dob=("1985-06-15", 20), sex=("male", 40))
    seed_patient(pg_conn, PC, dob=("1983-01-01", 20))
    seed_patient(pg_conn, PD, dob=("1989-01-01", 20))
    on_pairs, _ = _gen(pg_conn, max_block_size=3)
    off_pairs, _ = _gen(
        pg_conn, max_block_size=3,
        enabled_passes={"identifier", "dob", "name", "name+year", "dob-range"},
    )
    assert canonical_pair(PA, PB) in on_pairs
    assert canonical_pair(PA, PB) not in off_pairs


def test_unknown_sex_does_not_rescue_via_blocking_sex(pg_conn):
    # Principle 4 (no-data-is-never-agreement), mirroring adapter._VALUE_SENTINELS and the
    # identifier pass's `system <> 'unknown'`: two charts BOTH recording sex 'unknown' must
    # NOT share a blocking_sex row -- that would key the 'dob-range+sex' rescue on mutual
    # ignorance, not a real signal. cap=3 forces the plain 'dob-range' block (size 4) to be
    # skipped, so the rescue arm is the only path -- and it must NOT fire on 'unknown'.
    seed_patient(pg_conn, PA, dob=("1981/1991", 30, "year-range"),
                 admin_sex=("unknown", 30))
    seed_patient(pg_conn, PB, dob=("1985-06-15", 20), sex=("unknown", 40))
    seed_patient(pg_conn, PC, dob=("1983-01-01", 20))
    seed_patient(pg_conn, PD, dob=("1989-01-01", 20))
    pairs, skipped = _gen(pg_conn, max_block_size=3)
    assert any(pn == "dob-range" and key == PA and sz == 4 for pn, key, sz in skipped)
    assert canonical_pair(PA, PB) not in pairs


def test_name_year_never_keys_on_a_range_window_min(pg_conn):
    # Honesty fix: "1981/1991" used to leak its first 4-digit run (1981) into the
    # 'name+year' pass as if it were a birth year -- a false key (the window MIN is not
    # a birth year; principle 4). Setup: PA (range dob) and PB (born 1981) share the
    # name token "zed"; PC oversizes the single-token 'zed' block at cap=2, so ONLY
    # 'name+year' could pair PA-PB among the name passes. The range passes are toggled
    # OFF to isolate the assertion (they legitimately pair PA-PB via the window).
    seed_patient(pg_conn, PA, dob=("1981/1991", 30, "year-range"), names=[("Zed", 20)])
    seed_patient(pg_conn, PB, dob=("1981-05-05", 20), names=[("Zed", 20)])
    seed_patient(pg_conn, PC, dob=("2000-01-01", 20), names=[("Zed", 20)])
    pairs, skipped = _gen(
        pg_conn, max_block_size=2,
        enabled_passes={"identifier", "dob", "name", "name+year"},
    )
    assert any(pn == "name" and sz == 3 for pn, _key, sz in skipped)
    assert canonical_pair(PA, PB) not in pairs
