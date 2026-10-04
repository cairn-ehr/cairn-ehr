"""R4 Task 5: the drain — newest first, a poison chart held, a big backlog swept once."""

import itertools
import uuid

from cairn_matcher.pipeline import queue_db, worker
from cairn_matcher.pipeline.banding import matcher_version
from cairn_matcher.pipeline.worker import Settings, drain
from cairn_matcher.pipeline.worker_plan import RetryBook
from tests.conftest import cairn_test_dsn, seed_patient

A, B, C = (str(uuid.UUID(int=i)) for i in (21, 22, 23))


def _count(conn, sql, *args):
    with conn.cursor() as cur:
        cur.execute(sql, args)
        n = cur.fetchone()[0]
    conn.rollback()
    return n


def _queued(conn):
    with conn.cursor() as cur:
        cur.execute("SELECT DISTINCT patient_id::text FROM match_pending WHERE reason = 'config'")
        got = {p for (p,) in cur.fetchall()}
    conn.rollback()
    return got


def _state(conn):
    with conn.cursor() as cur:
        cur.execute("INSERT INTO match_worker_state (matcher_version) VALUES ('v')")
    conn.commit()


def _drain(conn, settings):
    return drain(conn, settings, RetryBook(300.0), clock=lambda: 0.0, sleep=lambda s: None)


def test_the_newest_change_is_checked_first(pg_conn):
    for p in (A, B, C):                       # C is seeded last: its notice is newest
        seed_patient(pg_conn, p, names=[(f"Name {p[-2:]}", 20)])
    assert queue_db.next_charts(pg_conn, 10, []) == [C, B, A]
    pg_conn.rollback()


def test_a_poison_chart_is_held_while_the_rest_drain(pg_conn, monkeypatch):
    _state(pg_conn)
    for p in (A, B):
        seed_patient(pg_conn, p, names=[(f"Name {p[-2:]}", 20)])
    real = worker.check_chart

    def check(conn, patient, settings):
        if patient == A:
            raise RuntimeError("poison")
        return real(conn, patient, settings)

    monkeypatch.setattr(worker, "check_chart", check)
    book = RetryBook(backoff_s=300.0)
    report = drain(pg_conn, Settings(), book, clock=lambda: 0.0, sleep=lambda s: None)
    assert (report.checked, report.failed) == (1, 1)
    assert book.held(0.0) == [A]
    assert _count(pg_conn, "SELECT count(*) FROM match_pending WHERE patient_id = %s", A) > 0
    assert _count(pg_conn, "SELECT count(*) FROM match_pending WHERE patient_id = %s", B) == 0


def _backlog(conn, base, n=6):
    ids = [str(uuid.UUID(int=base + i)) for i in range(n)]
    for p in ids:
        seed_patient(conn, p, names=[("Mary Smith", 20)])
    return ids


def test_a_big_backlog_is_swept_once_and_clears_exactly_the_notices_it_read(pg_conn):
    _state(pg_conn)
    ids = _backlog(pg_conn, 100)
    with pg_conn.cursor() as cur:            # A and B linked: the sweep must skip them
        cur.execute("INSERT INTO person_member (patient_id, person_id) VALUES (%s,%s),(%s,%s)",
                    (ids[0], ids[0], ids[1], ids[0]))
    pg_conn.commit()
    report = _drain(pg_conn, Settings(bulk_threshold=3))
    assert report.swept
    assert _count(pg_conn, "SELECT count(*) FROM match_pending") == 0
    lo, hi = sorted(ids[:2])
    assert _count(pg_conn, "SELECT count(*) FROM match_proposal "
                           "WHERE patient_low = %s AND patient_high = %s", lo, hi) == 0


def test_a_notice_committed_during_the_sweep_survives_and_is_checked(pg_conn, monkeypatch):
    import psycopg

    from cairn_matcher.pipeline import sweep as sweep_mod

    _state(pg_conn)
    _backlog(pg_conn, 300)
    late = str(uuid.UUID(int=399))
    real_propose = sweep_mod.propose
    fired = []

    def propose(conn, a, b, **kw):
        if not fired:                          # a second session commits a NEW chart mid-sweep
            fired.append(True)
            with psycopg.connect(cairn_test_dsn()) as other:
                seed_patient(other, late, names=[("Late Arrival", 20)])
        return real_propose(conn, a, b, **kw)

    monkeypatch.setattr("cairn_matcher.pipeline.sweep.propose", propose)
    real_check = worker.check_chart
    seen = []

    def check(conn, patient, settings):
        seen.append(patient)
        return real_check(conn, patient, settings)

    monkeypatch.setattr(worker, "check_chart", check)
    report = _drain(pg_conn, Settings(bulk_threshold=3))
    assert report.swept and fired
    assert late in seen
    assert _count(pg_conn, "SELECT count(*) FROM match_pending") == 0


def test_a_sweep_pair_that_errors_keeps_its_charts_notices(pg_conn, monkeypatch):
    _state(pg_conn)
    ids = [str(uuid.UUID(int=200 + i)) for i in range(5)]
    for p in ids:
        seed_patient(pg_conn, p, names=[("Mary Smith", 20)])
    from cairn_matcher.pipeline import runner
    real = runner.propose

    def propose(conn, a, b, **kw):
        if ids[0] in (str(a), str(b)):
            raise RuntimeError("unscoreable")
        return real(conn, a, b, **kw)

    monkeypatch.setattr("cairn_matcher.pipeline.sweep.propose", propose)
    # A per-chart check would re-check the kept chart and clear it; make every check fail (the
    # chart is held, its notices stay) so only the sweep's own keep rule is under test.
    def failing(conn, patient, settings):
        raise RuntimeError("held")

    monkeypatch.setattr(worker, "check_chart", failing)
    _drain(pg_conn, Settings(bulk_threshold=2))
    assert _count(pg_conn, "SELECT count(*) FROM match_pending WHERE patient_id = %s",
                  ids[0]) > 0


def test_a_new_version_or_a_first_run_queues_every_chart(pg_conn):
    for p in (A, B):
        seed_patient(pg_conn, p, names=[("Mary Smith", 20)])
    with pg_conn.cursor() as cur:
        cur.execute("TRUNCATE match_pending")    # as on a node that existed before db/056
    pg_conn.commit()
    v = matcher_version()
    assert queue_db.ensure_version(pg_conn, v) is True
    # Containment, not equality: a shared dev DB may hold a stray patient_chart row.
    queued = _queued(pg_conn)
    assert {A, B} <= queued
    assert queue_db.ensure_version(pg_conn, v) is False
    assert _queued(pg_conn) == queued


def test_bulk_mode_never_reproposes_a_judged_pair_in_reconciliation(pg_conn, monkeypatch):
    from cairn_matcher.pipeline import runner
    from cairn_matcher.pipeline import sweep as sweep_mod

    _state(pg_conn)
    for p in (A, B):
        seed_patient(pg_conn, p, dob=("1950-01-07", 60, "day"), names=[("Mary Smith", 60)])
    runner.propose(pg_conn, A, B)             # a PENDING proposal for the pair
    pg_conn.commit()
    with pg_conn.cursor() as cur:             # ... then a human links them: now judged
        cur.execute("INSERT INTO person_member (patient_id, person_id) VALUES (%s,%s),(%s,%s)",
                    (A, A, B, A))
    pg_conn.commit()
    before = _count(pg_conn, "SELECT count(*) FROM match_proposal WHERE status = 'pending'")
    assert before == 1
    real, called = sweep_mod.propose, []

    def spy(conn, a, b, **kw):
        called.append((str(a), str(b)))
        return real(conn, a, b, **kw)

    monkeypatch.setattr("cairn_matcher.pipeline.sweep.propose", spy)
    worker.run_bulk(pg_conn, Settings())
    assert called == []
    assert _count(pg_conn, "SELECT count(*) FROM match_proposal WHERE status = 'pending'") == 1


def _node_sees_progress() -> bool:
    """What another session (the node's status read) sees: has a progress stamp been COMMITTED?"""
    import psycopg

    with psycopg.connect(cairn_test_dsn()) as other:
        return other.execute(
            "SELECT last_drained_at IS NOT NULL FROM match_worker_state").fetchone()[0]


def test_a_bulk_drain_stamps_progress_as_sweep_pairs_complete(pg_conn, monkeypatch):
    # R4 rulings R15/R17: the node reads "behind" when a change has waited five minutes AND the
    # worker completed no work in that time. A sweep can run for minutes, so run_bulk stamps
    # progress (and COMMITS it, so the node can see it), throttled, as pairs are SUCCESSFULLY
    # scored -- never before any work.
    from cairn_matcher.pipeline import sweep as sweep_mod

    _state(pg_conn)                                  # last_drained_at starts NULL
    _backlog(pg_conn, 500)
    events, seen_by_node = [], []
    real_stamp, real_propose = queue_db.stamp_drained, sweep_mod.propose

    def stamp(conn):
        events.append("stamp")
        real_stamp(conn)

    def propose(conn, a, b, **kw):
        events.append("propose")
        if len(seen_by_node) < 2:                    # before the 1st and the 2nd pair
            seen_by_node.append(_node_sees_progress())
        return real_propose(conn, a, b, **kw)

    monkeypatch.setattr(queue_db, "stamp_drained", stamp)
    monkeypatch.setattr(sweep_mod, "propose", propose)
    ticks = itertools.count(0, 31)                   # every call 31 s later: every stamp is due
    worker.run_bulk(pg_conn, Settings(), clock=lambda: float(next(ticks)))
    assert events[:2] == ["propose", "stamp"]        # the first stamp follows the first pair ...
    assert seen_by_node == [False, True]             # ... committed, so the node sees it
    first, last = events.index("propose"), len(events) - 1 - events[::-1].index("propose")
    assert events[first:last + 1].count("stamp") > 1  # ... and progress keeps being stamped


def test_a_sweep_whose_blocking_raises_stamps_no_progress(pg_conn, monkeypatch):
    # R4 ruling R15 (the R8 crash-loop): a data-triggered error in the sweep's blocking aborts the
    # sweep before any pair completes. Every retry (watch's backoff, a supervisor restart) must
    # leave the progress stamp untouched, so after five quiet minutes the node reads "behind"
    # instead of "running" forever while nothing is checked.
    import pytest

    from cairn_matcher.pipeline import db

    _state(pg_conn)                                  # last_drained_at starts NULL
    _backlog(pg_conn, 600)

    def broken(conn, **kw):
        raise RuntimeError("bad data in the blocking SQL")

    monkeypatch.setattr(db, "generate_candidate_pairs", broken)
    with pytest.raises(RuntimeError):
        worker.run_bulk(pg_conn, Settings(), clock=lambda: 0.0)
    pg_conn.rollback()
    assert _node_sees_progress() is False


def test_a_sweep_in_which_every_pair_fails_stamps_no_progress(pg_conn, monkeypatch):
    # R4 review N3: a failed pair is not progress. If every propose() raises for a systematic
    # reason (a missing grant after a schema change, a persist timeout), the sweep keeps the failed
    # charts' notices, the next round sweeps again -- and a stamp per attempt would read "running"
    # forever while nothing is checked. Neither during nor after such a sweep may the stamp move.
    from cairn_matcher.pipeline import sweep as sweep_mod

    _state(pg_conn)                                  # last_drained_at starts NULL
    _backlog(pg_conn, 700)
    seen_by_node = []

    def failing(conn, a, b, **kw):
        if len(seen_by_node) < 3:
            seen_by_node.append(_node_sees_progress())
        raise RuntimeError("permission denied for table match_proposal")

    monkeypatch.setattr(sweep_mod, "propose", failing)
    ticks = itertools.count(0, 31)                   # every stamp would be due
    result = worker.run_bulk(pg_conn, Settings(), clock=lambda: float(next(ticks)))
    assert result.generated > 0 and len(result.errors) == result.generated
    assert seen_by_node == [False, False, False]     # nothing stamped during the sweep ...
    assert _node_sees_progress() is False            # ... nor after it
