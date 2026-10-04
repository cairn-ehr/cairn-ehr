# matcher/src/cairn_matcher/pipeline/worker.py
"""The commit-time duplicate check (repair path R4, #679, ADR-0076 decision 7).

db/056 appends a notice whenever a chart's identity evidence changes. This worker drains them:
for each chart it finds the candidate pairs (targeted.py), drops pairs already judged
(judged.py), assesses each (runner.assess) and writes the outcomes plus the delete of the
chart's notices in ONE short transaction. It writes match_proposal and nothing else that
matters: it NEVER links — a hit is a proposal a human resolves (R2's panel, R5's worklist).

Requires the optional `pipeline` extra (psycopg).
"""

from dataclasses import dataclass, field

from cairn_matcher.orchestrator import DEFAULT_CONFIG, ComparatorConfig
from cairn_matcher.pipeline import judged, queue_db, runner, targeted
from cairn_matcher.pipeline.banding import DEFAULT_THRESHOLDS, Thresholds
from cairn_matcher.scoring import DEFAULT_WEIGHTS, Weights


@dataclass(frozen=True)
class Settings:
    """The worker's knobs. Defaults are set from Task 7's measurement (design page note)."""

    max_block_size: int = targeted.DEFAULT_TARGETED_CAP
    sweep_block_size: int = 100        # the sweep's own all-pairs cap, unchanged
    bulk_threshold: int = 500
    batch: int = 50
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


def check_chart(conn, patient: str, upto_id: int, settings: Settings) -> ChartResult:
    """Check one chart and clear its notices up to `upto_id`, in ONE transaction; COMMITS.

    Reads first (pairs, skip rule, stale pending proposals, the assessments), then writes every
    outcome, deletes the notices it read, stamps the drain time and commits. Nothing is locked
    until the writes, so a clinical write never waits on this. If anything raises, the caller
    rolls back: no proposal lands and the notices stay, so the chart is checked again (a crash
    loses nothing).
    """
    from cairn_matcher.pipeline import db

    pairs, skipped = targeted.candidate_pairs_for(
        conn, patient, max_block_size=settings.max_block_size)
    pairs = judged.drop_judged(pairs, patient, judged.judged_partners(conn, patient))
    # #210 per chart: a PENDING proposal involving this chart that blocking no longer produces
    # (a Doe identified since) is re-assessed, so a stale row is retracted rather than left.
    generated = set(pairs)
    stale = [p for p in queue_db.pending_pairs_involving(conn, patient) if p not in generated]
    everyone = {pid for pair in pairs + stale for pid in pair}
    aliases = db.load_aliases_for(conn, everyone)
    trust = db.load_trust_for(conn, everyone)
    verdicts = [
        runner.assess(conn, low, high, thresholds=settings.thresholds,
                      weights=settings.weights, config=settings.config,
                      aliases=aliases, trust=trust)
        for low, high in pairs + stale
    ]
    proposed = retracted = 0
    for v in verdicts:
        wrote = runner.persist(conn, v)
        if v.band is not None:
            proposed += 1
        elif wrote:
            retracted += 1
    queue_db.clear_chart(conn, patient, upto_id)
    queue_db.stamp_drained(conn)
    conn.commit()
    return ChartResult(patient, proposed, retracted, skipped)
