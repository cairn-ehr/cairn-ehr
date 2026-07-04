# matcher/src/cairn_matcher/pipeline/sweep.py
"""Batch driver (piece B2b): generate candidate pairs, score each via propose().

This is the front end B2 lacked — it decides WHICH pairs to score (db.generate_candidate_pairs,
the blocking passes) and feeds each through the existing pairwise propose(). Pure
orchestration over the db + runner seam; no scoring/banding logic lives here.

Two phases. Phase 1 generates the candidates, then closes the read snapshot BEFORE the
write loop so a long sweep does not pin the xmin horizon (the hazard runner.propose
already guards on its own sub-threshold path). Phase 2 loops propose() per pair: each is
its own transaction and idempotent (human status preserved on re-run), so the sweep is
resumable, and a failing pair is recorded and skipped (house rule #5) rather than aborting
the batch.

Requires the optional `pipeline` extra (psycopg) at CALL time, because it drives db/runner.
"""

from dataclasses import dataclass, field

from cairn_matcher.pipeline.banding import DEFAULT_THRESHOLDS, Band, Thresholds
from cairn_matcher.pipeline.runner import propose
from cairn_matcher.scoring import DEFAULT_WEIGHTS, Weights


@dataclass(frozen=True)
class SkippedBlock:
    """A blocking-value group excluded from pair generation for exceeding the cap."""

    pass_name: str   # 'identifier' | 'dob' | 'name' | 'name+year' | 'dob-range' | 'dob-range+sex'
    key: str         # the human-readable blocking value (system:match_key, dob, token, token|year,
                      # or -- for the two anchored range passes -- the anchor patient's uuid)
    size: int        # number of patients sharing it (the whole block, INCLUDING the anchor for
                      # the two anchored passes); a symmetric block of size k drops C(k,2) pairs,
                      # an anchored block of size k drops k-1 (anchor x each withheld member)


@dataclass(frozen=True)
class SweepError:
    """One candidate pair whose propose() raised — recorded, never silently dropped."""

    pair: tuple[str, str]
    message: str


@dataclass(frozen=True)
class SweepResult:
    """Summary of one sweep: the observability surface and the 'log what was dropped' record."""

    generated: int                                   # candidate pairs attempted (scored + errored)
    auto_candidate: int                              # proposals written in the AUTO_CANDIDATE band
    review: int                                      # proposals written in the REVIEW band
    below_threshold: int                             # pairs that persisted nothing
    skipped_blocks: list[SkippedBlock] = field(default_factory=list)
    errors: list[SweepError] = field(default_factory=list)


def sweep(
    conn,
    *,
    max_block_size: int = 100,
    thresholds: Thresholds = DEFAULT_THRESHOLDS,
    weights: Weights = DEFAULT_WEIGHTS,
) -> SweepResult:
    """Score every blocking candidate pair and return a SweepResult summary.

    Generates candidates (closing the read snapshot before writing), then proposes on each
    surviving pair. A pair whose propose() raises is recorded in `errors` and skipped; the
    connection is rolled back so it stays usable for the next pair.
    """
    # Imported lazily so this module is importable without the optional `pipeline` extra;
    # only an actual sweep() call needs psycopg (mirrors runner.propose's lazy db import).
    from cairn_matcher.pipeline import db

    pairs, skipped_raw = db.generate_candidate_pairs(conn, max_block_size=max_block_size)
    # Pre-load the §5.5(a) known-aliases for the whole candidate-patient set in ONE query,
    # still inside the generate read snapshot. This replaces two per-pair alias SELECTs in
    # propose() (which would re-fetch a chart's aliases once per pair it appears in, and be
    # two empty SELECTs per pair in the common no-repudiation case) with a single scoped,
    # PK-indexed read; propose() then reads aliases from this map, not the DB.
    candidate_patients = {pid for pair in pairs for pid in pair}
    aliases = db.load_aliases_for(conn, candidate_patients)
    # Close the read transaction the SELECTs opened before the per-pair write loop.
    conn.rollback()

    skipped_blocks = [SkippedBlock(*s) for s in skipped_raw]
    auto = review = below = 0
    errors: list[SweepError] = []
    for low, high in pairs:
        try:
            result = propose(
                conn, low, high, thresholds=thresholds, weights=weights, aliases=aliases
            )
        except Exception as exc:  # noqa: BLE001 — batch must survive one bad pair (house rule #5)
            # Clear the aborted transaction so the connection is usable for the next pair.
            conn.rollback()
            errors.append(SweepError((low, high), f"{type(exc).__name__}: {exc}"))
            continue
        if result is Band.AUTO_CANDIDATE:
            auto += 1
        elif result is Band.REVIEW:
            review += 1
        else:
            below += 1
    return SweepResult(
        generated=len(pairs),
        auto_candidate=auto,
        review=review,
        below_threshold=below,
        skipped_blocks=skipped_blocks,
        errors=errors,
    )
