"""R4 Task 4: one chart's check — proposals and the notice delete in ONE transaction.

Seeds go through conftest.seed_patient, whose committed projection rows fire db/056's hook, so
every seeded chart has real notices to drain.
"""

import uuid

import pytest

from cairn_matcher.pipeline import queue_db, runner
from cairn_matcher.pipeline.worker import Settings, check_chart
from tests.conftest import seed_patient

A, B = (str(uuid.UUID(int=i)) for i in (11, 12))


def _upto(conn, p):
    with conn.cursor() as cur:
        cur.execute("SELECT max(id) FROM match_pending WHERE patient_id = %s", (p,))
        upto = cur.fetchone()[0]
    conn.rollback()
    return upto


def _count(conn, sql, *args):
    with conn.cursor() as cur:
        cur.execute(sql, args)
        return cur.fetchone()[0]


def _near_duplicates(conn):
    seed_patient(conn, A, dob=("1950-01-07", 60, "day"), names=[("Mary Smith", 60)],
                 identifiers=[("mrn:a", "77", "77")])
    seed_patient(conn, B, dob=("1950-01-07", 60, "day"), names=[("Mary Smyth", 60)],
                 identifiers=[("mrn:a", "77", "77")])


def test_a_near_duplicate_becomes_a_proposal_and_the_notices_are_cleared(pg_conn):
    with pg_conn.cursor() as cur:            # stamp_drained UPDATEs the worker's state row
        cur.execute("INSERT INTO match_worker_state (matcher_version) VALUES ('v')")
    pg_conn.commit()
    _near_duplicates(pg_conn)
    result = check_chart(pg_conn, B, _upto(pg_conn, B), Settings())
    assert result.proposed == 1
    assert _count(pg_conn, "SELECT count(*) FROM match_proposal") == 1
    assert _count(pg_conn, "SELECT count(*) FROM match_pending WHERE patient_id = %s", B) == 0
    assert _count(pg_conn, "SELECT count(*) FROM match_worker_state "
                           "WHERE last_drained_at IS NOT NULL") == 1


def test_a_strong_pair_is_proposed_and_never_linked(pg_conn):
    _near_duplicates(pg_conn)
    check_chart(pg_conn, B, _upto(pg_conn, B), Settings())
    assert _count(pg_conn, "SELECT count(*) FROM match_proposal") == 1
    assert _count(pg_conn, "SELECT count(*) FROM patient_link") == 0


def test_a_change_landing_mid_check_survives_and_is_checked_again(pg_conn, monkeypatch):
    import psycopg

    from tests.conftest import cairn_test_dsn

    _near_duplicates(pg_conn)
    upto = _upto(pg_conn, B)
    real_assess = runner.assess

    def assess_then_a_colleague_corrects_the_name(conn, a, b, **kw):
        with psycopg.connect(cairn_test_dsn()) as other:
            other.execute("INSERT INTO match_pending (patient_id, reason) VALUES (%s,'change')",
                          (B,))
        return real_assess(conn, a, b, **kw)

    monkeypatch.setattr(runner, "assess", assess_then_a_colleague_corrects_the_name)
    check_chart(pg_conn, B, upto, Settings())
    ids = _count(pg_conn, "SELECT array_agg(id) FROM match_pending WHERE patient_id = %s", B)
    assert ids is not None and all(i > upto for i in ids)


def test_a_crash_before_commit_loses_nothing(pg_conn, monkeypatch):
    _near_duplicates(pg_conn)
    upto = _upto(pg_conn, B)

    def boom(*_a, **_k):
        raise RuntimeError("crash between scoring and the delete")

    monkeypatch.setattr(queue_db, "clear_chart", boom)
    with pytest.raises(RuntimeError):
        check_chart(pg_conn, B, upto, Settings())
    pg_conn.rollback()
    assert _count(pg_conn, "SELECT count(*) FROM match_proposal") == 0
    assert _count(pg_conn, "SELECT count(*) FROM match_pending WHERE patient_id = %s", B) > 0


def test_a_linked_pair_is_never_proposed(pg_conn):
    _near_duplicates(pg_conn)
    with pg_conn.cursor() as cur:
        cur.execute("INSERT INTO person_member (patient_id, person_id) VALUES (%s,%s),(%s,%s)",
                    (A, A, B, A))
    pg_conn.commit()
    assert check_chart(pg_conn, B, _upto(pg_conn, B), Settings()).proposed == 0
    assert _count(pg_conn, "SELECT count(*) FROM match_proposal") == 0


def test_a_stale_pending_proposal_no_longer_blocked_is_reassessed(pg_conn):
    # A pending proposal for (A, C) where C now shares nothing with A: the per-chart #210
    # reconciliation re-assesses it, and below the floor it is retracted.
    c = str(uuid.UUID(int=13))
    seed_patient(pg_conn, A, names=[("Mary Smith", 20)])
    seed_patient(pg_conn, c, names=[("Zed Quux", 20)])
    lo, hi = sorted([A, c])
    with pg_conn.cursor() as cur:
        cur.execute(
            "INSERT INTO match_proposal (patient_low, patient_high, score_total, band, "
            "veto_findings, evidence, matcher_version) VALUES (%s,%s,1,'review','[]','[]','v')",
            (lo, hi))
    pg_conn.commit()
    result = check_chart(pg_conn, A, _upto(pg_conn, A), Settings())
    assert result.retracted == 1
    assert _count(pg_conn, "SELECT status FROM match_proposal") == "retracted"


def test_the_worker_role_suffices(pg_conn):
    _near_duplicates(pg_conn)
    upto = _upto(pg_conn, B)
    with pg_conn.cursor() as cur:
        cur.execute("SET ROLE cairn_agent")
    check_chart(pg_conn, B, upto, Settings())
    with pg_conn.cursor() as cur:
        cur.execute("RESET ROLE")
    pg_conn.commit()
    assert _count(pg_conn, "SELECT count(*) FROM match_proposal") == 1
