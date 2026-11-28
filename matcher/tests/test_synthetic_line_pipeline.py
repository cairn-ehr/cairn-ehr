# matcher/tests/test_synthetic_line_pipeline.py
"""The matcher never proposes a pair across the line between synthetic patients and real ones,
and still proposes two synthetic charts to each other (ADR-0115, #1219 Task 3).

DB-gated on CAIRN_TEST_PG. Charts are seeded directly; `seed_registration` is what gives a
chart a kind. A chart with no registration is of UNKNOWN kind and pairs with everything, which
is why every other suite here (none of which registers its charts) is unaffected.
"""

from cairn_matcher.pipeline.banding import Band
from cairn_matcher.pipeline.runner import assess, canonical_pair, persist, propose
from cairn_matcher.pipeline.sweep import sweep
from cairn_matcher.pipeline.synthetic_line import SYNTHETIC_SIDE_LABEL
from tests.conftest import seed_patient, seed_registration

REAL_A = "11111111-1111-1111-1111-111111111111"
REAL_B = "22222222-2222-2222-2222-222222222222"
SYN_A = "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa"
SYN_B = "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb"
UNKNOWN = "eeeeeeee-eeee-eeee-eeee-eeeeeeeeeeee"


def _same_person(conn, patient, *, synthetic=None):
    """Seed a chart that matches every other chart seeded by this helper (same name and MRN)."""
    seed_patient(conn, patient, names=[("Alex Smith", 20)], dob=("1980-07-15", 20),
                 identifiers=[("mrn:hospital-a", "12345", "12345")])
    if synthetic is not None:
        seed_registration(conn, patient, synthetic=synthetic)


def _generated(conn, **kw):
    from cairn_matcher.pipeline.db import generate_candidate_pairs
    return generate_candidate_pairs(conn, **kw)


def _status(conn, a, b):
    low, high = canonical_pair(a, b)
    with conn.cursor() as cur:
        cur.execute("SELECT status FROM match_proposal WHERE patient_low=%s AND patient_high=%s",
                    (low, high))
        row = cur.fetchone()
    conn.rollback()
    return row[0] if row else None


# --- the floor's answer, as the matcher reads it -------------------------------------------

def test_the_kinds_are_read_from_the_floor_and_an_unregistered_chart_is_unknown(pg_conn):
    from cairn_matcher.pipeline.db import load_kinds_for
    _same_person(pg_conn, REAL_A, synthetic=False)
    _same_person(pg_conn, SYN_A, synthetic=True)
    _same_person(pg_conn, UNKNOWN)
    assert load_kinds_for(pg_conn, [REAL_A, SYN_A.upper(), UNKNOWN]) == {
        REAL_A: "real", SYN_A: "synthetic", UNKNOWN: "unknown"}
    assert load_kinds_for(pg_conn, []) == {}


# --- pair generation -----------------------------------------------------------------------

def test_the_sweep_generates_no_pair_across_the_line(pg_conn):
    _same_person(pg_conn, REAL_A, synthetic=False)
    _same_person(pg_conn, SYN_A, synthetic=True)
    pairs, _ = _generated(pg_conn)
    assert pairs == []


def test_two_synthetic_charts_are_still_generated_as_a_pair(pg_conn):
    for p in (REAL_A, REAL_B):
        _same_person(pg_conn, p, synthetic=False)
    for p in (SYN_A, SYN_B):
        _same_person(pg_conn, p, synthetic=True)
    pairs, _ = _generated(pg_conn)
    assert pairs == sorted([canonical_pair(REAL_A, REAL_B), canonical_pair(SYN_A, SYN_B)])


def test_a_chart_of_unknown_kind_is_paired_with_both_sides(pg_conn):
    _same_person(pg_conn, REAL_A, synthetic=False)
    _same_person(pg_conn, SYN_A, synthetic=True)
    _same_person(pg_conn, UNKNOWN)
    pairs, _ = _generated(pg_conn)
    assert pairs == sorted([canonical_pair(REAL_A, UNKNOWN), canonical_pair(SYN_A, UNKNOWN)])


def test_synthetic_charts_do_not_push_a_real_block_over_the_cap(pg_conn):
    # Two real charts and three synthetic ones share every blocking value. With a cap of 2, a
    # cap applied to the whole group of five would drop the real pair. The real half is 2: kept.
    for p in (REAL_A, REAL_B):
        _same_person(pg_conn, p, synthetic=False)
    for p in (SYN_A, SYN_B, "cccccccc-cccc-cccc-cccc-cccccccccccc"):
        _same_person(pg_conn, p, synthetic=True)
    pairs, skipped = _generated(pg_conn, max_block_size=2)
    assert pairs == [canonical_pair(REAL_A, REAL_B)]
    # Every skipped block is the synthetic half, named as such, at its own size.
    assert skipped
    assert all(key.endswith(SYNTHETIC_SIDE_LABEL) and size == 3 for _p, key, size in skipped)


def test_the_per_chart_check_generates_no_pair_across_the_line(pg_conn):
    from cairn_matcher.pipeline.targeted import candidate_pairs_for
    _same_person(pg_conn, REAL_A, synthetic=False)
    _same_person(pg_conn, REAL_B, synthetic=False)
    _same_person(pg_conn, SYN_A, synthetic=True)
    _same_person(pg_conn, SYN_B, synthetic=True)
    pairs, _ = candidate_pairs_for(pg_conn, SYN_A)
    assert pairs == [canonical_pair(SYN_A, SYN_B)]
    pairs, _ = candidate_pairs_for(pg_conn, REAL_A)
    assert pairs == [canonical_pair(REAL_A, REAL_B)]


def test_an_anchored_age_window_holds_no_member_across_the_line(pg_conn):
    # An estimated-age chart (a John Doe's year range) is the ANCHOR of a window block.
    from cairn_matcher.pipeline.db import generate_candidate_pairs
    seed_patient(pg_conn, REAL_A, dob=("1975/1985", 30, "year-range"))
    seed_registration(pg_conn, REAL_A, synthetic=False)
    seed_patient(pg_conn, REAL_B, dob=("1980-07-15", 20))
    seed_registration(pg_conn, REAL_B, synthetic=False)
    seed_patient(pg_conn, SYN_A, dob=("1981-03-02", 20))
    seed_registration(pg_conn, SYN_A, synthetic=True)
    pairs, _ = generate_candidate_pairs(pg_conn, enabled_passes={"dob-range"})
    assert pairs == [canonical_pair(REAL_A, REAL_B)]


# --- the verdict ---------------------------------------------------------------------------

def test_a_pair_across_the_line_is_never_banded_whatever_it_scores(pg_conn):
    # Handed straight to assess(), as a client that skipped pair generation would.
    _same_person(pg_conn, REAL_A, synthetic=False)
    _same_person(pg_conn, SYN_A, synthetic=True)
    verdict = assess(pg_conn, REAL_A, SYN_A)
    assert verdict.band is None and verdict.payload is None
    assert propose(pg_conn, REAL_A, SYN_A) is None
    assert _status(pg_conn, REAL_A, SYN_A) is None


def test_two_synthetic_charts_are_proposed_like_any_other_pair(pg_conn):
    _same_person(pg_conn, SYN_A, synthetic=True)
    _same_person(pg_conn, SYN_B, synthetic=True)
    assert propose(pg_conn, SYN_A, SYN_B) in (Band.AUTO_CANDIDATE, Band.REVIEW)
    assert _status(pg_conn, SYN_A, SYN_B) == "pending"


def test_a_proposal_written_before_the_registration_arrived_is_retracted(pg_conn):
    # Offline-first: the other chart's registration is still in transit, so its kind is unknown
    # and the pair is proposed. When the registration arrives and says "synthetic", the next
    # sweep must take the proposal back: no human can ever accept it (db/018).
    _same_person(pg_conn, REAL_A, synthetic=False)
    _same_person(pg_conn, SYN_A)
    assert propose(pg_conn, REAL_A, SYN_A) is not None
    assert _status(pg_conn, REAL_A, SYN_A) == "pending"

    seed_registration(pg_conn, SYN_A, synthetic=True)
    result = sweep(pg_conn)
    assert _status(pg_conn, REAL_A, SYN_A) == "retracted"
    assert result.reconciled_retracted == 1


def test_persist_of_a_crossing_verdict_writes_no_row(pg_conn):
    _same_person(pg_conn, REAL_A, synthetic=False)
    _same_person(pg_conn, SYN_A, synthetic=True)
    assert persist(pg_conn, assess(pg_conn, REAL_A, SYN_A)) is False
    pg_conn.rollback()


# --- the yield, with and without the training cohort (ADR-0115 decision 8) -----------------

def test_the_sweep_says_how_much_of_its_yield_is_between_synthetic_charts(pg_conn):
    for p in (REAL_A, REAL_B):
        _same_person(pg_conn, p, synthetic=False)
    for p in (SYN_A, SYN_B):
        _same_person(pg_conn, p, synthetic=True)
    result = sweep(pg_conn)
    assert result.generated == 2
    assert result.synthetic_generated == 1
    proposed = result.auto_candidate + result.review
    assert proposed == 2
    assert result.synthetic_auto_candidate + result.synthetic_review == 1
    # What a reader of the miss-rate metric wants: the yield among real patients alone.
    assert result.real_yield() == 1


def test_a_sweep_with_no_synthetic_chart_reports_none(pg_conn):
    for p in (REAL_A, REAL_B):
        _same_person(pg_conn, p, synthetic=False)
    result = sweep(pg_conn)
    assert (result.synthetic_generated, result.synthetic_auto_candidate,
            result.synthetic_review) == (0, 0, 0)
    assert result.real_yield() == result.auto_candidate + result.review == 1
