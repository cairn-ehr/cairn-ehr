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
