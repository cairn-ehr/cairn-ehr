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
    result = check_chart(pg_conn, B, Settings())
    assert result.proposed == 1
    assert _count(pg_conn, "SELECT count(*) FROM match_proposal") == 1
    assert _count(pg_conn, "SELECT count(*) FROM match_pending WHERE patient_id = %s", B) == 0
    assert _count(pg_conn, "SELECT count(*) FROM match_worker_state "
                           "WHERE last_drained_at IS NOT NULL") == 1


def test_a_strong_pair_is_proposed_and_never_linked(pg_conn):
    # The design's test list: "an auto-band pair leaves patient_link empty". The pair must
    # really reach the AUTO_CANDIDATE band, or the test proves nothing about the strongest case:
    # the near-duplicates above only reach REVIEW (Smith/Smyth disagree, no sex on file), so
    # these two agree on name, DOB, sex and identifier (runner.assess bands them auto_candidate).
    for p in (A, B):
        seed_patient(pg_conn, p, dob=("1950-01-07", 60, "day"), sex=("female", 60),
                     names=[("Mary Smith", 60)], identifiers=[("mrn:a", "77", "77")])
    events_before = _count(pg_conn, "SELECT count(*) FROM event_log")
    check_chart(pg_conn, B, Settings())
    assert _count(pg_conn, "SELECT count(*) FROM match_proposal") == 1
    assert _count(pg_conn, "SELECT band FROM match_proposal") == "auto_candidate"
    assert _count(pg_conn, "SELECT count(*) FROM patient_link") == 0
    # ... and it authored nothing: no event (a link is an event), so nothing can sync onward.
    assert _count(pg_conn, "SELECT count(*) FROM event_log") == events_before


def test_a_chart_id_spelled_differently_is_checked_in_full(pg_conn):
    # A is the LOW side of (A, B). Checked under a braced, upper-case id, the chart must still
    # recognise its own side of the pair; read as "the other side", A would count as judged (a
    # chart is always its own partner) and the pair would be dropped unchecked.
    _near_duplicates(pg_conn)
    result = check_chart(pg_conn, "{" + A.upper() + "}", Settings())
    assert result.proposed == 1
    assert result.patient == A
    assert _count(pg_conn, "SELECT count(*) FROM match_pending WHERE patient_id = %s", A) == 0


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
    check_chart(pg_conn, B, Settings())
    ids = _count(pg_conn, "SELECT array_agg(id) FROM match_pending WHERE patient_id = %s", B)
    assert ids is not None and all(i > upto for i in ids)


def test_a_crash_before_commit_loses_nothing(pg_conn, monkeypatch):
    _near_duplicates(pg_conn)

    def boom(*_a, **_k):
        raise RuntimeError("crash between scoring and the delete")

    monkeypatch.setattr(queue_db, "clear_notices", boom)
    with pytest.raises(RuntimeError):
        check_chart(pg_conn, B, Settings())
    pg_conn.rollback()
    assert _count(pg_conn, "SELECT count(*) FROM match_proposal") == 0
    assert _count(pg_conn, "SELECT count(*) FROM match_pending WHERE patient_id = %s", B) > 0


def test_a_linked_pair_is_never_proposed(pg_conn):
    _near_duplicates(pg_conn)
    with pg_conn.cursor() as cur:
        cur.execute("INSERT INTO person_member (patient_id, person_id) VALUES (%s,%s),(%s,%s)",
                    (A, A, B, A))
    pg_conn.commit()
    assert check_chart(pg_conn, B, Settings()).proposed == 0
    assert _count(pg_conn, "SELECT count(*) FROM match_proposal") == 0


def test_a_pair_only_an_unattested_unlink_stands_on_is_proposed(pg_conn):
    # #741: an agent's unconfirmed "different people" is not a human judgement. The pair must
    # still reach a human (banner + worklist), with the dispute shown there.
    import hashlib
    _near_duplicates(pg_conn)
    lo, hi = sorted([A, B])
    with pg_conn.cursor() as cur:
        cur.execute(
            "INSERT INTO patient_link (low, high, state, hlc_wall, hlc_counter, origin, "
            "provenance, content_address, attested) "
            "VALUES (%s,%s,'unlink',1,0,'seed','test:agent',%s,false)",
            (lo, hi, b"\x12\x20" + hashlib.sha256(f"{lo}{hi}".encode()).digest()))
    pg_conn.commit()
    assert check_chart(pg_conn, B, Settings()).proposed == 1


def _seed_auto_applied(conn, lo, hi):
    """A proposal auto_apply.rs already linked: status 'auto_applied' with the link event's id
    (db/019: applied_event_id is set exactly on applied/auto_applied rows)."""
    with conn.cursor() as cur:
        cur.execute(
            "INSERT INTO match_proposal (patient_low, patient_high, score_total, band, "
            "veto_findings, evidence, matcher_version, status, applied_event_id) "
            "VALUES (%s,%s,9,'auto_candidate','[]','[]','v','auto_applied',%s)",
            (lo, hi, str(uuid.uuid4())))
    conn.commit()


def _seed_standing_link(conn, lo, hi, state):
    """The pair's standing patient_link row, written by an UN-attested writer (the matcher or an
    ADR-0030 agent)."""
    import hashlib
    with conn.cursor() as cur:
        cur.execute(
            "INSERT INTO patient_link (low, high, state, hlc_wall, hlc_counter, origin, "
            "provenance, content_address, attested) "
            "VALUES (%s,%s,%s,1,0,'seed','test:machine',%s,false)",
            (lo, hi, state, b"\x12\x20" + hashlib.sha256(f"{lo}{hi}".encode()).digest()))
    conn.commit()


def test_an_auto_applied_pair_an_agent_unlinked_reopens_for_a_human(pg_conn):
    # The matcher auto-linked A–B; an agent's later un-attested unlink won the overlay, so the
    # two are separate records again and no human has judged them. A row left 'auto_applied'
    # is not open (db/057), so neither the banner nor the worklist would ever show the pair —
    # #741's hazard through the auto_applied door. The re-check must reopen it as 'pending'
    # (clearing applied_event_id, db/019), where the dispute is shown.
    _near_duplicates(pg_conn)
    lo, hi = sorted([A, B])
    _seed_auto_applied(pg_conn, lo, hi)
    _seed_standing_link(pg_conn, lo, hi, "unlink")
    assert check_chart(pg_conn, B, Settings()).proposed == 1
    assert _count(pg_conn, "SELECT status FROM match_proposal") == "pending"
    assert _count(pg_conn, "SELECT count(*) FROM match_proposal "
                           "WHERE applied_event_id IS NOT NULL") == 0
    assert _count(pg_conn, "SELECT count(*) FROM match_proposal_open") == 1


def test_an_auto_applied_pair_whose_link_still_stands_stays_auto_applied(pg_conn):
    # The matcher's own un-attested link is still the standing row: nothing disputes it, so the
    # re-check refreshes the score and leaves the verdict alone. (person_member is not seeded,
    # so the pair is not "judged" and does reach upsert_proposal — the case under test.)
    _near_duplicates(pg_conn)
    lo, hi = sorted([A, B])
    _seed_auto_applied(pg_conn, lo, hi)
    _seed_standing_link(pg_conn, lo, hi, "link")
    check_chart(pg_conn, B, Settings())
    assert _count(pg_conn, "SELECT status FROM match_proposal") == "auto_applied"
    assert _count(pg_conn, "SELECT count(*) FROM match_proposal "
                           "WHERE applied_event_id IS NOT NULL") == 1


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
    result = check_chart(pg_conn, A, Settings())
    assert result.retracted == 1
    assert _count(pg_conn, "SELECT status FROM match_proposal") == "retracted"


def test_a_stale_review_row_no_longer_blocked_is_retracted(pg_conn):
    # #743 part 1: auto_apply moved this pair to status 'review' (a veto appeared), then the
    # facts changed and the matcher no longer proposes it. It must be withdrawn, not left asking
    # a human to judge a pair the matcher has dropped.
    c = str(uuid.UUID(int=13))
    seed_patient(pg_conn, A, names=[("Mary Smith", 20)])
    seed_patient(pg_conn, c, names=[("Zed Quux", 20)])
    lo, hi = sorted([A, c])
    with pg_conn.cursor() as cur:
        cur.execute(
            "INSERT INTO match_proposal (patient_low, patient_high, score_total, band, "
            "veto_findings, evidence, matcher_version, status) "
            "VALUES (%s,%s,1,'auto_candidate','[]','[]','v','review')",
            (lo, hi))
    pg_conn.commit()
    result = check_chart(pg_conn, A, Settings())
    assert result.retracted == 1
    assert _count(pg_conn, "SELECT status FROM match_proposal") == "retracted"


def test_a_review_row_still_above_the_floor_stays_review(pg_conn):
    # #743 part 1 lets the matcher re-assess 'review' rows. One it still proposes must keep
    # auto_apply's kick: moving it back to 'pending' would let the next auto-apply run try the
    # pair again, so a veto's "a human decides this" would not be durable.
    _near_duplicates(pg_conn)
    lo, hi = sorted([A, B])
    with pg_conn.cursor() as cur:
        cur.execute(
            "INSERT INTO match_proposal (patient_low, patient_high, score_total, band, "
            "veto_findings, evidence, matcher_version, status) "
            "VALUES (%s,%s,1,'auto_candidate','[]','[]','v','review')",
            (lo, hi))
    pg_conn.commit()
    result = check_chart(pg_conn, B, Settings())
    assert result.proposed == 1 and result.retracted == 0
    assert _count(pg_conn, "SELECT status FROM match_proposal") == "review"


def test_the_worker_role_suffices(pg_conn, monkeypatch):
    _near_duplicates(pg_conn)
    seen = {}

    def as_worker(name, real):
        """Wrap a phase so it records current_user, proving the role survived check_chart's
        rollbacks (an UNCOMMITTED SET ROLE is rolled back and would silently revert)."""
        def wrapper(conn, *args, **kw):
            with conn.cursor() as cur:
                cur.execute("SELECT current_user")
                seen[name] = cur.fetchone()[0]
            return real(conn, *args, **kw)
        return wrapper

    monkeypatch.setattr(runner, "assess", as_worker("assess", runner.assess))
    monkeypatch.setattr(runner, "persist", as_worker("persist", runner.persist))
    monkeypatch.setattr(queue_db, "clear_notices", as_worker("clear", queue_db.clear_notices))
    with pg_conn.cursor() as cur:
        cur.execute("SET ROLE cairn_agent")
    pg_conn.commit()          # a committed session SET survives the rollbacks inside check_chart
    check_chart(pg_conn, B, Settings())
    with pg_conn.cursor() as cur:
        cur.execute("RESET ROLE")
    pg_conn.commit()
    assert seen == {"assess": "cairn_agent", "persist": "cairn_agent", "clear": "cairn_agent"}
    assert _count(pg_conn, "SELECT count(*) FROM match_proposal") == 1


def test_a_slow_transaction_with_a_lower_id_is_not_deleted_by_the_check(pg_conn, monkeypatch):
    """ids are assigned at INSERT, not commit: T1 holds id N (uncommitted), T2's N+1 commits,
    the check reads only N+1; T1 commits mid-check. Its notice (id < the max read) must survive."""
    import psycopg

    from tests.conftest import cairn_test_dsn

    _near_duplicates(pg_conn)
    slow = psycopg.connect(cairn_test_dsn())
    slow.execute("INSERT INTO match_pending (patient_id, reason) VALUES (%s,'change')", (B,))
    # A later, committed notice gets a HIGHER id than the slow one.
    with psycopg.connect(cairn_test_dsn()) as fast:
        fast.execute("INSERT INTO match_pending (patient_id, reason) VALUES (%s,'change')", (B,))
    real_assess = runner.assess
    committed = []

    def assess_then_slow_commits(conn, a, b, **kw):
        if not committed:
            slow.commit()
            committed.append(True)
        return real_assess(conn, a, b, **kw)

    monkeypatch.setattr(runner, "assess", assess_then_slow_commits)
    try:
        check_chart(pg_conn, B, Settings())
    finally:
        slow.close()
    assert committed
    assert _count(pg_conn, "SELECT count(*) FROM match_pending WHERE patient_id = %s", B) == 1


def test_no_lock_is_held_across_an_assessment(pg_conn, monkeypatch):
    import psycopg

    from tests.conftest import cairn_test_dsn

    _near_duplicates(pg_conn)
    # A third near-duplicate gives the chart TWO pairs, so the second assess starts after the
    # first one's reads: any lock the first left behind is still there when the probe runs.
    seed_patient(pg_conn, str(uuid.UUID(int=14)), dob=("1950-01-07", 60, "day"),
                 names=[("Mary Smith", 60)], identifiers=[("mrn:a", "77", "77")])
    real_assess = runner.assess
    probed = []

    def assess_after_probing_for_locks(conn, a, b, **kw):
        with psycopg.connect(cairn_test_dsn(), autocommit=True) as other:
            other.execute("BEGIN")
            for table in ("patient_name", "patient_demographic", "patient_identifier",
                          "patient_chart"):
                other.execute(f"LOCK TABLE {table} IN ACCESS EXCLUSIVE MODE NOWAIT")
            other.execute("ROLLBACK")
        probed.append(True)
        return real_assess(conn, a, b, **kw)

    monkeypatch.setattr(runner, "assess", assess_after_probing_for_locks)
    check_chart(pg_conn, B, Settings())
    assert len(probed) == 2


def test_a_since_linked_pending_proposal_is_not_refreshed(pg_conn):
    _near_duplicates(pg_conn)
    lo, hi = sorted([A, B])
    with pg_conn.cursor() as cur:
        cur.execute(
            "INSERT INTO match_proposal (patient_low, patient_high, score_total, band, "
            "veto_findings, evidence, matcher_version) VALUES (%s,%s,1,'review','[]','[]','old')",
            (lo, hi))
        cur.execute("INSERT INTO person_member (patient_id, person_id) VALUES (%s,%s),(%s,%s)",
                    (A, A, B, A))
    pg_conn.commit()
    result = check_chart(pg_conn, B, Settings())
    assert result.proposed == 0 and result.retracted == 0
    assert _count(pg_conn, "SELECT matcher_version FROM match_proposal") == "old"
