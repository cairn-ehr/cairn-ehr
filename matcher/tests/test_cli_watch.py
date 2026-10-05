"""R4 Task 5: `cairn-matcher watch --once` drains and exits; nothing links."""

import uuid

import pytest

from cairn_matcher.cli import main
from tests.conftest import cairn_test_dsn, seed_patient

A, B = (str(uuid.UUID(int=i)) for i in (31, 32))


def test_watch_once_checks_every_chart_and_exits_zero(pg_conn):
    for p in (A, B):
        seed_patient(pg_conn, p, dob=("1950-01-07", 60, "day"), names=[("Mary Smith", 60)])
    assert main(["watch", "--once", "--dsn", cairn_test_dsn()]) == 0
    with pg_conn.cursor() as cur:
        cur.execute("SELECT count(*) FROM match_pending")
        assert cur.fetchone()[0] == 0
        cur.execute("SELECT count(*) FROM match_proposal")
        assert cur.fetchone()[0] == 1
        cur.execute("SELECT count(*) FROM patient_link")
        assert cur.fetchone()[0] == 0
        cur.execute("SELECT last_drained_at IS NOT NULL FROM match_worker_state")
        assert cur.fetchone()[0] is True


def test_watch_once_exits_two_when_the_database_is_unreachable(caplog):
    # Needs psycopg (to fail a connect) but no database: skipped only where the extra is absent.
    pytest.importorskip("psycopg")
    assert main(["watch", "--once", "--dsn", "host=127.0.0.1 port=1 connect_timeout=2"]) == 2
    assert "OperationalError" in caplog.text, caplog.text   # the error class is named
