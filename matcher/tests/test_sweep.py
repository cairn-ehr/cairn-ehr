# matcher/tests/test_sweep.py
"""Integration tests for sweep(): generate candidates -> propose() each -> SweepResult.

Gated on CAIRN_TEST_PG (skips cleanly without a database).
"""

from cairn_matcher.pipeline.runner import canonical_pair
from tests.conftest import seed_patient, seed_repudiation

PA = "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa"
PB = "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb"
PC = "cccccccc-cccc-cccc-cccc-cccccccccccc"
PD = "dddddddd-dddd-dddd-dddd-dddddddddddd"


def _proposal_status(conn, low, high):
    with conn.cursor() as cur:
        cur.execute("SELECT status FROM match_proposal WHERE patient_low=%s AND patient_high=%s",
                    (low, high))
        row = cur.fetchone()
        return row[0] if row else None


def test_sweep_proposes_a_strong_candidate(pg_conn):
    from cairn_matcher.pipeline.sweep import sweep
    for p in (PA, PB):
        seed_patient(pg_conn, p, names=[("Alex Smith", 20)],
                     identifiers=[("mrn:a", "12345", "12345")])
    result = sweep(pg_conn)
    assert result.generated >= 1
    assert result.review >= 1                      # the strong pair lands in REVIEW
    low, high = canonical_pair(PA, PB)
    assert _proposal_status(pg_conn, low, high) == "pending"


def test_sweep_writes_nothing_for_a_no_signal_population(pg_conn):
    from cairn_matcher.pipeline.sweep import sweep
    # No shared blocking key at all -> no candidates -> no proposals.
    seed_patient(pg_conn, PA, names=[("Alex Smith", 20)], identifiers=[("mrn:a", "1", "1")])
    seed_patient(pg_conn, PB, names=[("Robin Jones", 20)], identifiers=[("mrn:a", "2", "2")])
    result = sweep(pg_conn)
    assert result.generated == 0
    assert result.review == 0 and result.auto_candidate == 0
    assert result.below_threshold == 0 and result.errors == []


def test_sweep_is_idempotent_and_preserves_human_status(pg_conn):
    from cairn_matcher.pipeline.sweep import sweep
    for p in (PA, PB):
        seed_patient(pg_conn, p, names=[("Alex Smith", 20)],
                     identifiers=[("mrn:a", "12345", "12345")])
    sweep(pg_conn)
    low, high = canonical_pair(PA, PB)
    with pg_conn.cursor() as cur:                  # a reviewer accepts it
        cur.execute("UPDATE match_proposal SET status='accepted' WHERE patient_low=%s", (low,))
    pg_conn.commit()
    sweep(pg_conn)                                  # re-run
    assert _proposal_status(pg_conn, low, high) == "accepted"


def test_sweep_reports_oversized_blocks(pg_conn):
    from cairn_matcher.pipeline.sweep import sweep
    for p in (PA, PB, PC):
        seed_patient(pg_conn, p, dob=("1980-07-15", 20))
    result = sweep(pg_conn, max_block_size=2)
    assert any(sb.pass_name == "dob" and sb.size == 3 for sb in result.skipped_blocks)


def test_sweep_skips_and_reports_a_failing_pair(pg_conn, monkeypatch):
    # Two independent strong pairs. propose() is forced to raise on ONE; the sweep must
    # record the error, recover the connection, and still score the other pair.
    from cairn_matcher.pipeline import sweep as sweep_mod
    from cairn_matcher.pipeline.runner import propose as real_propose

    for p in (PA, PB):
        seed_patient(pg_conn, p, identifiers=[("mrn:a", "111", "111")])
    for p in (PC, PD):
        seed_patient(pg_conn, p, identifiers=[("mrn:b", "222", "222")])

    failing = canonical_pair(PA, PB)

    def flaky(conn, a, b, **kw):
        if canonical_pair(a, b) == failing:
            raise RuntimeError("boom")
        return real_propose(conn, a, b, **kw)

    monkeypatch.setattr(sweep_mod, "propose", flaky)
    result = sweep_mod.sweep(pg_conn)
    assert len(result.errors) == 1
    assert result.errors[0].pair == failing
    assert "boom" in result.errors[0].message
    # the other pair was still scored and persisted
    assert _proposal_status(pg_conn, *canonical_pair(PC, PD)) == "pending"


def test_sweep_tags_a_returning_known_alias_pair(pg_conn):
    # End-to-end over the batch alias PRELOAD path: sweep() reads the candidate set's
    # aliases once (load_aliases_for) and threads the map into propose(), so a returning
    # persona still lands a REVIEW proposal carrying the known_alias flag.
    from cairn_matcher.pipeline.sweep import sweep

    false_name = "John Fakename"
    seed_patient(pg_conn, PA, names=[("Jane Realname", 20), (false_name, 20)])
    seed_repudiation(pg_conn, PA, false_name)
    seed_patient(pg_conn, PB, names=[(false_name, 20)])

    result = sweep(pg_conn)
    assert result.review >= 1
    low, high = canonical_pair(PA, PB)
    with pg_conn.cursor() as cur:
        cur.execute(
            "SELECT band, evidence FROM match_proposal WHERE patient_low=%s AND patient_high=%s",
            (low, high),
        )
        band_value, evidence = cur.fetchone()
    assert band_value == "review"
    assert any(e.get("kind") == "known_alias" and e["alias_of"] == PA for e in evidence)
