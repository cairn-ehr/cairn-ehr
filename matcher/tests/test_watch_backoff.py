"""R4 Task 5 fix: watch's reconnect backoff resets only after a drain SUCCEEDS."""

import pytest

from cairn_matcher.pipeline import watch as watch_mod
from cairn_matcher.pipeline.worker import DrainReport, Settings
from tests.conftest import cairn_test_dsn


class _Stop(BaseException):
    """Ends the otherwise endless watch loop."""


def test_a_failure_after_connect_keeps_backing_off(monkeypatch):
    import psycopg

    dsn = cairn_test_dsn()
    if not dsn:
        pytest.skip("CAIRN_TEST_PG not set — skipping DB-gated integration test")
    # fail, fail, succeed, fail, stop -> delays 1, 2, then back to 1 (reset only by the success)
    script = iter(["boom", "boom", "ok", "boom", "stop"])

    def drain(conn, settings, book):
        step = next(script)
        if step == "boom":
            raise psycopg.OperationalError("statement timeout")
        if step == "stop":
            raise _Stop
        return DrainReport(0, 0, False)

    delays = []
    monkeypatch.setattr(watch_mod, "drain", drain)
    monkeypatch.setattr(watch_mod.time, "sleep", delays.append)
    with pytest.raises(_Stop):
        watch_mod.watch(dsn, Settings(poll_s=0.01))
    assert delays == [1.0, 2.0, 1.0]



class _FakeListen:
    """The LISTEN connection, faked: `notifies` hands out at most `stop_after` queued
    notifications per call, as separate packets would arrive (psycopg may return MORE than
    stop_after when several share one packet, which is why a real-database test cannot force
    this case)."""

    def __init__(self):
        self.pending = []

    def __enter__(self):
        return self

    def __exit__(self, *exc):
        return False

    def execute(self, sql):
        pass

    def notifies(self, *, timeout=None, stop_after=None):
        n = 0
        while self.pending and (stop_after is None or n < stop_after):
            yield self.pending.pop(0)
            n += 1


def test_notifications_piled_up_during_a_drain_cost_one_more_drain_not_one_each(monkeypatch):
    # A long drain (a sweep) lets many NOTIFYs pile up -- one per committing clinical write. ONE
    # further drain covers all of them; waking once per notification would re-drain the queue
    # again and again for nothing.
    import psycopg

    listen = _FakeListen()
    left_at_drain = []

    def drain(conn, settings, book):
        left_at_drain.append(len(listen.pending))
        if len(left_at_drain) == 1:
            listen.pending.extend(["notify"] * 5)    # five commits land during the first drain
        if len(left_at_drain) == 2:
            raise _Stop
        return DrainReport(0, 0, False)

    monkeypatch.setattr(psycopg, "connect", lambda dsn, autocommit=False: (
        listen if autocommit else _FakeListen()))
    monkeypatch.setattr(watch_mod.queue_db, "ensure_version", lambda conn, version: False)
    monkeypatch.setattr(watch_mod, "drain", drain)
    with pytest.raises(_Stop):
        watch_mod.watch("unused", Settings(poll_s=0.01))
    assert left_at_drain == [0, 0]       # the second drain starts with the pile already read


def test_once_names_the_database_error_rather_than_calling_it_unreachable(monkeypatch, caplog):
    # A statement timeout (QueryCanceled) is an OperationalError too, but the database answered:
    # "cannot reach the database" would send the operator to the network instead of the query.
    import logging

    import psycopg

    def drain(conn, settings, book):
        raise psycopg.errors.QueryCanceled("canceling statement due to statement timeout")

    monkeypatch.setattr(psycopg, "connect", lambda dsn, autocommit=False: _FakeListen())
    monkeypatch.setattr(watch_mod.queue_db, "ensure_version", lambda conn, version: False)
    monkeypatch.setattr(watch_mod, "drain", drain)
    with caplog.at_level(logging.ERROR, logger="cairn_matcher.worker"):
        assert watch_mod.watch("unused", Settings(), once=True) == 2
    text = caplog.text
    assert "QueryCanceled" in text and "statement timeout" in text, text
    assert "cannot reach" not in text, text
