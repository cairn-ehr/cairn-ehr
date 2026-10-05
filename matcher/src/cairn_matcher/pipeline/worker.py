# matcher/src/cairn_matcher/pipeline/worker.py
"""The commit-time duplicate check (repair path R4, #679, ADR-0076 decision 7).

db/056 appends a notice whenever a chart's identity evidence changes. This worker drains them:
for each chart it finds the candidate pairs (targeted.py), drops pairs already judged
(judged.py), assesses each (runner.assess) and writes the outcomes plus the delete of the
chart's notices in ONE short transaction. It writes match_proposal and nothing else that
matters: it NEVER links — a hit is a proposal a human resolves (R2's panel, R5's worklist).

Requires the optional `pipeline` extra (psycopg).
"""

import logging
import time
import uuid
from dataclasses import dataclass, field

from cairn_matcher.orchestrator import DEFAULT_CONFIG, ComparatorConfig
from cairn_matcher.pipeline import judged, queue_db, runner, targeted
from cairn_matcher.pipeline.banding import DEFAULT_THRESHOLDS, Thresholds
from cairn_matcher.pipeline.sweep import SweepResult, sweep
from cairn_matcher.pipeline.worker_plan import (
    Mode,
    RetryBook,
    Throttle,
    choose_mode,
    sweep_completed_work,
)
from cairn_matcher.scoring import DEFAULT_WEIGHTS, Weights

log = logging.getLogger("cairn_matcher.worker")

# How often a sweep stamps progress (at most). The node calls the check "behind" after five quiet
# minutes (crates/cairn-node/src/duplicate_check.rs STALLED_AFTER_SECS); stamping about every 30 s
# while pairs are being scored keeps a healthy sweep far inside that, at the cost of one tiny
# UPDATE + commit per 30 s, not one per pair. Only a SUCCESSFULLY scored pair stamps (ruling R15,
# review N3): a pair whose propose() raised is not progress.
PROGRESS_EVERY_S = 30.0


@dataclass(frozen=True)
class Settings:
    """The worker's knobs; the R4 as-built note on the design page explains the defaults."""

    # The per-chart check keeps blocks up to DEFAULT_TARGETED_CAP (1000) members; a bulk sweep
    # keeps only up to sweep_block_size (100). Skipped blocks are logged, never silently dropped.
    max_block_size: int = targeted.DEFAULT_TARGETED_CAP
    sweep_block_size: int = 100        # the sweep's own all-pairs cap, unchanged
    # Charts waiting above which one sweep beats checking each (ruling R11). Measured 2026-10-04
    # (Apple M3 Max 128 GB, PostgreSQL 18.1): break-even ~83 charts at 2 000 records (sweep 37.6 s
    # / per-chart p50 453 ms) and ~15 at 10 000 (sweep 129.9 s / p50 8 829 ms). It falls as the
    # population grows because the per-chart check is superlinear (its blocking SQL groups the
    # whole population for every chart; a follow-up issue anchors it on the chart). 30 sits
    # between the two: the worst wrong choice costs ~2.7x at 2 000 (31 charts swept, 37.6 s,
    # instead of ~14 s per chart) and ~2x at 10 000 (30 charts checked singly, ~265 s, instead
    # of one 130 s sweep), where the old 500 cost ~30x there (500 x 8.8 s = 73 min vs ~2 min).
    # Operators override with `cairn-matcher watch --bulk-threshold`; re-measure once the
    # per-chart blocking is fixed.
    bulk_threshold: int = 30
    # Charts fetched per queue read. 1 = re-read the queue after EVERY chart, so a change that
    # arrives while a chart is being checked is checked next, ahead of older waiting changes.
    # A larger batch would make it wait behind the whole batch (~9 s a chart at 10 000 records,
    # #725). The re-read is cheap (one indexed GROUP BY over the waiting notices) next to a
    # check that takes seconds; a backlog big enough to make it costly is swept first.
    batch: int = 1
    retry_after_s: float = 300.0
    poll_s: float = 60.0
    pace_ms: int = 0
    thresholds: Thresholds = DEFAULT_THRESHOLDS
    weights: Weights = DEFAULT_WEIGHTS
    config: ComparatorConfig = DEFAULT_CONFIG


@dataclass(frozen=True)
class ChartResult:
    patient: str
    proposed: int
    retracted: int
    skipped_blocks: list = field(default_factory=list)


def check_chart(conn, patient: str, settings: Settings) -> ChartResult:
    """Check one chart and clear exactly the notices it read; COMMITS.

    1. Capture the chart's notice ids FIRST (queue_db.notice_ids): a bigserial is assigned at
       INSERT, not commit, so a slow transaction's lower-id notice can commit mid-check; deleting
       `id <= max` would remove a change this check never saw. Only the captured ids are deleted.
    2. Read (pairs, skip rule, stale pending proposals, aliases/trust), then assess each pair.
       The read transaction is ENDED (rollback) after the prep and after every assess call, so
       no ACCESS SHARE lock or xmin pin is held across the (slow) assessments: the node's loader
       re-runs `ALTER TABLE ... ADD COLUMN IF NOT EXISTS` on every connect, which takes ACCESS
       EXCLUSIVE and would otherwise queue behind this worker, and every clinical write behind it.
    3. In one short final transaction: persist every outcome, delete the captured notices,
       stamp the drain time, commit. If anything raises, the caller rolls back: no proposal
       lands and the notices stay (a crash loses nothing).
    """
    from cairn_matcher.pipeline import db

    # ONE canonical id (lowercase, hyphenated) used for every step below: the pair helpers compare
    # ids as text, and a braced or unhyphenated id would make the chart miss its own side.
    me = str(uuid.UUID(str(patient)))
    ids = queue_db.notice_ids(conn, me)
    pairs, skipped = targeted.candidate_pairs_for(
        conn, me, max_block_size=settings.max_block_size)
    partners = judged.judged_partners(conn, me)
    pairs = judged.drop_judged(pairs, me, partners)
    # #210 per chart: a PENDING proposal involving this chart that blocking no longer produces
    # (a Doe identified since) is re-assessed, so a stale row is retracted rather than left.
    # Judged/linked pairs are skipped here too, or a since-linked pair would be re-upserted.
    generated = set(pairs)
    stale = judged.drop_judged(
        [p for p in queue_db.pending_pairs_involving(conn, me) if p not in generated],
        me, partners)
    todo = pairs + stale
    everyone = {pid for pair in todo for pid in pair}
    aliases = db.load_aliases_for(conn, everyone)
    trust = db.load_trust_for(conn, everyone)
    conn.rollback()
    verdicts = []
    for low, high in todo:
        verdicts.append(runner.assess(conn, low, high, thresholds=settings.thresholds,
                                      weights=settings.weights, config=settings.config,
                                      aliases=aliases, trust=trust))
        conn.rollback()
    proposed = retracted = 0
    for v in verdicts:
        wrote = runner.persist(conn, v)
        if v.band is not None:
            proposed += 1
        elif wrote:
            retracted += 1
    queue_db.clear_notices(conn, ids)
    queue_db.stamp_drained(conn)
    conn.commit()
    return ChartResult(me, proposed, retracted, skipped)


@dataclass(frozen=True)
class DrainReport:
    checked: int      # charts whose per-chart check committed
    failed: int       # charts whose check raised (held by the RetryBook, notices kept)
    swept: bool       # True when a bulk sweep ran first


def run_bulk(conn, settings: Settings, clock=time.monotonic) -> SweepResult:
    """A backlog too big for per-chart checks: ONE sweep, then clear the notices read; COMMITS.

    The notice ids are captured BEFORE the sweep (and before judged_pairs), and exactly those
    ids are deleted afterwards. A notice committed while the sweep runs is not in the set, so it
    survives and gets a per-chart check (never `id <= watermark`: a bigserial is assigned at
    INSERT, not commit, so a lower id can commit late). Charts in a pair the sweep failed to
    score keep their notices (`keep`), so they still read "not yet checked". The sweep keeps its
    own all-pairs cap: a block it skips is reported in the result, as it always has been.

    Progress (rulings R13, R15, R17, review N3): the node reads the check as "behind" when a
    change has waited five minutes and the worker completed no work in that time (quiet time runs
    from the oldest waiting notice or the last completed work, whichever is later), and a sweep can
    run longer than that. So the worker stamps `last_drained_at` and commits when a pair has been
    SUCCESSFULLY scored and PROGRESS_EVERY_S has passed since the last stamp (a Throttle on
    `clock`; the first successful pair always stamps), and once more at the end unless every
    attempted pair failed (worker_plan.sweep_completed_work; a sweep with nothing to score is
    completed work). Only completed work is progress -- never the start of a sweep, its blocking
    phase, or a failed pair. Otherwise a sweep that fails every time (its blocking raises: the R8
    crash-loop; or every propose() raises: a missing grant, a schema mismatch) would refresh the
    stamp on each retry and the node would read "running" forever while nothing is checked. A
    long blocking phase therefore honestly reads "behind" once quiet time passes five minutes.
    sweep() calls back only after propose() has committed its own transaction, so the stamp's
    commit never holds a lock across a pair's work, and never commits anything of the sweep's
    own.
    """
    ids = queue_db.notice_ids(conn)
    skip = judged.judged_pairs(conn)
    conn.rollback()          # no read transaction (lock, xmin pin) held across the sweep
    throttle = Throttle(PROGRESS_EVERY_S)

    def progress() -> None:
        if throttle.due(clock()):
            queue_db.stamp_drained(conn)
            conn.commit()

    result = sweep(conn, max_block_size=settings.sweep_block_size,
                   thresholds=settings.thresholds, weights=settings.weights,
                   config=settings.config, skip_pairs=skip, on_progress=progress)
    keep = sorted({pid for e in result.errors for pid in e.pair})
    queue_db.clear_notices(conn, ids, keep)
    scored = (result.auto_candidate + result.review + result.below_threshold
              + result.reconciled)
    if sweep_completed_work(scored, len(result.errors)):
        queue_db.stamp_drained(conn)
    conn.commit()
    return result


def describe_skipped(blocks) -> str:
    """One log-ready phrase for a chart's oversized blocks (pure): "<pass> <value> size <n>; ...".

    Each block is (pass_name, blocking value, member count) as targeted.candidate_pairs_for
    returns it. The value is a non-discriminating one by definition (hundreds of charts share
    it), or, for the anchored age-window passes, the anchor chart's id.
    """
    return "; ".join(f"{pass_name} {value!r} size {size}" for pass_name, value, size in blocks)


def drain(conn, settings: Settings, book: RetryBook, clock=time.monotonic,
          sleep=time.sleep) -> DrainReport:
    """Drain the queue until nothing is left but held charts.

    One sweep first when the backlog is over the threshold; then per-chart checks, newest change
    first, skipping charts the RetryBook holds. A chart whose check raises is rolled back, logged
    and held; its notices stay. `clock`/`sleep` are injectable for tests. `check_chart` is looked
    up as a module global on purpose, so tests can substitute it.
    """
    swept = False
    waiting = queue_db.charts_waiting(conn)
    conn.rollback()
    if choose_mode(waiting, settings.bulk_threshold) is Mode.SWEEP:
        result = run_bulk(conn, settings, clock)
        swept = True
        log.info("swept a backlog of %d charts: %d pairs, %d errors, %d blocks skipped",
                 waiting, result.generated, len(result.errors), len(result.skipped_blocks))
    checked = failed = 0
    while True:
        batch = queue_db.next_charts(conn, settings.batch, book.held(clock()))
        conn.rollback()
        if not batch:
            break
        for patient in batch:
            try:
                result = check_chart(conn, patient, settings)
                book.succeeded(patient)
                checked += 1
                if result.skipped_blocks:
                    # Checked, but not against the members of these blocks: say so.
                    log.warning("duplicate check for %s skipped %d block(s) over %d charts: %s",
                                patient, len(result.skipped_blocks), settings.max_block_size,
                                describe_skipped(result.skipped_blocks))
            except Exception as exc:  # noqa: BLE001 — one bad chart must not stop the drain
                conn.rollback()
                book.failed(patient, clock())
                failed += 1
                log.warning("duplicate check failed for %s (held %.0fs): %s: %s",
                            patient, settings.retry_after_s, type(exc).__name__, exc)
            if settings.pace_ms:
                sleep(settings.pace_ms / 1000)
    if waiting == 0:
        queue_db.stamp_drained(conn)       # an empty round still says "last ran HH:MM"
        conn.commit()
    return DrainReport(checked, failed, swept)
