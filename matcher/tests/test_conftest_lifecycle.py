# matcher/tests/test_conftest_lifecycle.py
"""Guards the integration-test connection lifecycle (issue #84 pt1).

The bug: `seed_patient` (and its siblings) COMMIT, but the pg_conn fixture only
TRUNCATEd projection tables at *setup*; teardown was `rollback()`, which cannot undo a
commit. So the LAST integration test's committed rows persisted in the test database and
any external consumer of the same DB (e.g. the eval harness) inherited dirty state.

The fix routes the fixture through `managed_pg_conn`, a context manager that truncates the
projection tables on EXIT as well, guaranteeing a clean database no matter what the test
committed. This test drives that context manager directly and asserts the guarantee.
"""

import os

import pytest

from tests import conftest
from tests.conftest import _PROJECTION_TABLES, managed_pg_conn, seed_patient

CAIRN_TEST_PG = os.environ.get("CAIRN_TEST_PG")


def _row_counts(dsn):
    """Row counts for every projection table, read on a fresh independent connection."""
    import psycopg

    with psycopg.connect(dsn, autocommit=True) as conn, conn.cursor() as cur:
        counts = {}
        for table in _PROJECTION_TABLES:
            cur.execute(f"SELECT count(*) FROM {table}")
            counts[table] = cur.fetchone()[0]
    return counts


def test_managed_pg_conn_truncates_committed_rows_on_exit():
    """A committed row (what a leaking test leaves behind) must not survive teardown."""
    if not CAIRN_TEST_PG:
        pytest.skip("CAIRN_TEST_PG not set — skipping DB-gated integration test")

    # Commit a projection row inside the managed connection, exactly as a real
    # integration test does via seed_patient's trailing conn.commit().
    with managed_pg_conn(CAIRN_TEST_PG) as conn:
        seed_patient(conn, "11111111-1111-1111-1111-111111111111", names=[("Leaky", 0)])

    # A fresh connection (not the managed one) must see zero rows in every projection
    # table: the managed connection truncated them on exit.
    assert _row_counts(CAIRN_TEST_PG) == dict.fromkeys(_PROJECTION_TABLES, 0)


class _FakeConn:
    """Minimal stand-in for a psycopg connection: records that close() ran."""

    def __init__(self):
        self.closed = False

    def close(self):
        self.closed = True


def test_exit_truncation_error_does_not_mask_the_test_failure(monkeypatch):
    """A cleanup error on exit must NEVER replace the body's own exception (issue #84 pt1).

    A generator context manager re-raises whatever its finally block raises, so an
    un-swallowed exit truncation error (e.g. the test crashed the connection) would surface
    INSTEAD of the real assertion failure. This drives that exact race — DB-independent, so
    it also guards the contract in the pure `uv run pytest` run — and asserts the body's
    sentinel propagates while the connection is still closed.
    """
    fake = _FakeConn()

    # Stub the DB touchpoints so no real cluster (and no psycopg install) is needed: a fake
    # `psycopg` module whose connect() yields the fake connection, schema application a
    # no-op, and truncation that succeeds on entry but blows up on exit. Injecting psycopg
    # via sys.modules (rather than monkeypatching the real one) keeps this test runnable in
    # the pure `uv run pytest` suite, where psycopg is not installed (pipeline extra only).
    import sys
    import types

    monkeypatch.setitem(
        sys.modules, "psycopg", types.SimpleNamespace(connect=lambda *a, **k: fake)
    )
    monkeypatch.setattr(conftest, "_apply_schema", lambda conn: None)
    calls = {"n": 0}

    def _flaky_truncate(conn):
        calls["n"] += 1
        if calls["n"] >= 2:  # entry succeeds; the exit (teardown) call raises
            raise RuntimeError("cleanup blew up")

    monkeypatch.setattr(conftest, "_truncate_projections", _flaky_truncate)

    class _Sentinel(Exception):
        pass

    # The body raises _Sentinel; the exit truncation raises RuntimeError. The caller must
    # see _Sentinel (the real failure), not the swallowed cleanup RuntimeError.
    try:
        with pytest.raises(_Sentinel):
            with managed_pg_conn("dummy-dsn"):
                raise _Sentinel
    finally:
        assert fake.closed, "the connection must still be closed even when exit truncation fails"
