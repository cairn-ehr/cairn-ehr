# matcher/src/cairn_matcher/pipeline/watch.py
"""`watch`: the commit-time worker's outer loop (repair path R4).

LISTEN for db/056's NOTIFY, re-queue every chart when the matcher version changed, drain, wait,
repeat. It lives apart from worker.py (the per-chart/bulk/drain logic) so each file stays small.
The worker never links anyone: it only writes match_proposal, match_pending, match_worker_state.
"""

import logging
import time

from cairn_matcher.pipeline import queue_db
from cairn_matcher.pipeline.banding import matcher_version
from cairn_matcher.pipeline.worker import Settings, drain
from cairn_matcher.pipeline.worker_plan import RetryBook

log = logging.getLogger("cairn_matcher.worker")


def watch(dsn: str, settings: Settings, *, once: bool = False) -> int:
    """Run the worker and return a process exit code.

    LISTEN is issued BEFORE the first drain, so a notice committed between a drain and the wait
    still wakes us. A poll (settings.poll_s) backs NOTIFY up across a reconnect. With `once`,
    drain a single time and return 0 (1 if any chart failed; 2 if the database is unreachable).
    Otherwise a lost connection is retried with a backoff capped at 60 s.
    """
    import psycopg

    version = matcher_version(settings.weights, settings.thresholds, settings.config)
    book = RetryBook(settings.retry_after_s)
    backoff = 1.0
    while True:
        try:
            # `listen` is autocommit (a LISTEN must not sit in a transaction); `work` is the
            # ordinary transactional connection the drain uses.
            with psycopg.connect(dsn, autocommit=True) as listen, psycopg.connect(dsn) as work:
                listen.execute("LISTEN cairn_match_pending")
                if queue_db.ensure_version(work, version):
                    log.info("matcher %s: every chart queued for a re-check", version)
                while True:
                    report = drain(work, settings, book)
                    # Reset the reconnect backoff only once a drain has really completed: an
                    # error raised AFTER connecting (statement timeout, deadlock) is deterministic
                    # and must keep backing off, not retry (and re-sweep) every second.
                    backoff = 1.0
                    if once:
                        return 0 if report.failed == 0 else 1
                    # Sleep until a notice arrives or the poll interval passes.
                    for _ in listen.notifies(timeout=settings.poll_s, stop_after=1):
                        pass
        except psycopg.OperationalError as exc:
            if once:
                log.error("cannot reach the database: %s", exc)
                return 2
            log.warning("database connection lost (%s); retrying in %.0fs", exc, backoff)
            time.sleep(backoff)
            backoff = min(backoff * 2, 60.0)
