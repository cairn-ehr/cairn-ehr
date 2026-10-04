"""R4 Task 2: assess() reads and decides; persist() writes without committing.

The worker needs a chart's proposals AND the delete of its notices in ONE transaction (a crash
re-checks the chart, never loses it), which propose() — committing per pair — cannot give.
propose() itself must behave exactly as before; its existing suites guard that.
"""

from cairn_matcher.pipeline.banding import Band
from cairn_matcher.pipeline.runner import assess, persist
from tests.conftest import seed_patient

PA = "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa"
PB = "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb"


def _proposals(conn):
    with conn.cursor() as cur:
        cur.execute("SELECT count(*) FROM match_proposal")
        return cur.fetchone()[0]


def test_assess_decides_without_writing(pg_conn):
    for p in (PA, PB):
        seed_patient(pg_conn, p, names=[("Alex Smith", 20)],
                     identifiers=[("mrn:hospital-a", "12345", "12345")])
    a = assess(pg_conn, PB, PA)
    assert (a.low, a.high) == (PA, PB)           # canonical order whatever the call order
    assert a.band is Band.REVIEW and a.payload is not None
    assert _proposals(pg_conn) == 0


def test_persist_writes_but_never_commits(pg_conn):
    for p in (PA, PB):
        seed_patient(pg_conn, p, names=[("Alex Smith", 20)],
                     identifiers=[("mrn:hospital-a", "12345", "12345")])
    assert persist(pg_conn, assess(pg_conn, PA, PB)) is True
    assert _proposals(pg_conn) == 1
    pg_conn.rollback()                            # the caller owns the transaction
    assert _proposals(pg_conn) == 0


def test_persist_of_a_below_floor_assessment_retracts_a_pending_row(pg_conn):
    for p in (PA, PB):
        seed_patient(pg_conn, p, sex=("female", 0))
    with pg_conn.cursor() as cur:
        cur.execute(
            "INSERT INTO match_proposal (patient_low, patient_high, score_total, band, "
            "veto_findings, evidence, matcher_version) VALUES (%s,%s,1,'review','[]','[]','v')",
            (PA, PB))
    pg_conn.commit()
    a = assess(pg_conn, PA, PB)
    assert a.band is None and a.payload is None
    assert persist(pg_conn, a) is True
    pg_conn.commit()
    with pg_conn.cursor() as cur:
        cur.execute("SELECT status FROM match_proposal")
        assert cur.fetchone()[0] == "retracted"
