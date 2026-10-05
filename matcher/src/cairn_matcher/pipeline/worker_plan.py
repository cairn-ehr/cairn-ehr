# matcher/src/cairn_matcher/pipeline/worker_plan.py
"""The commit-time worker's pure decisions (repair path R4): which mode, and which charts to hold.

Pure and psycopg-free, so the policy is testable with no database.
"""

from dataclasses import dataclass, field
from enum import Enum


class Mode(Enum):
    """How the next round drains the queue."""

    PER_CHART = "per-chart"   # check each waiting chart on its own, newest change first
    SWEEP = "sweep"           # a backlog too big for per-chart checks: one full sweep instead


def choose_mode(charts_waiting: int, bulk_threshold: int) -> Mode:
    """Sweep when MORE than `bulk_threshold` charts wait.

    Each per-chart check still pays a population-wide grouping (the symmetric statement; the
    range statement is anchored since #725), so N waiting charts cost ~N of them; one sweep costs
    one pass plus its pairs. Above the threshold (set from the measured break-even, see
    Settings.bulk_threshold) the sweep is cheaper.
    """
    return Mode.SWEEP if charts_waiting > bulk_threshold else Mode.PER_CHART


@dataclass
class RetryBook:
    """Charts whose check raised, held back for `backoff_s` so the worker never spins on one.

    In memory on purpose: a restarted worker retries at once. A held chart keeps its notices, so
    the node keeps reading it as "not yet checked" — the hold is never a silent skip.
    """

    backoff_s: float
    _until: dict[str, float] = field(default_factory=dict)

    def failed(self, patient: str, now: float) -> None:
        self._until[patient] = now + self.backoff_s

    def succeeded(self, patient: str) -> None:
        self._until.pop(patient, None)

    def held(self, now: float) -> list[str]:
        return sorted(p for p, until in self._until.items() if until > now)


@dataclass
class Throttle:
    """Say yes at most once per `interval_s` — the worker's progress stamp during a sweep.

    The node reads the duplicate check as "behind" once quiet time -- seconds since the oldest
    waiting notice or the worker's last completed work, whichever is later -- passes five minutes
    (db/056, rulings R13/R17). A sweep scores thousands of pairs; stamping after every successful
    one would be a write per pair, so the worker stamps only when the throttle is due. The FIRST
    call is always due (the sweep's first successfully scored pair stamps); after that, `due` is
    True once a full interval has passed since the last True. `now` is injected (a monotonic
    clock in production, a fake one in tests), so this stays pure.
    """

    interval_s: float
    _last: float | None = None

    def due(self, now: float) -> bool:
        if self._last is not None and now - self._last < self.interval_s:
            return False
        self._last = now
        return True


def sweep_completed_work(scored: int, failed: int) -> bool:
    """Did a sweep do work that may be stamped as progress (R4 review N3)?

    `scored` counts the pairs whose propose() succeeded (main loop and reconciliation), `failed`
    the pairs whose propose() raised. A sweep with nothing to score at all is completed work: the
    queue was handled and its notices are cleared. A sweep in which every attempted pair failed
    is NOT: it keeps the failed charts' notices and, under a systematic failure (a missing grant,
    a schema mismatch), would sweep and fail again every round; stamping it would make the node
    read "running" forever while nothing is checked. Any successful pair is progress.
    """
    return scored > 0 or failed == 0
