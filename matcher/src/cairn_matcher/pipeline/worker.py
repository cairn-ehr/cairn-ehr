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

    me = str(patient).lower()
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
    return ChartResult(patient, proposed, retracted, skipped)
