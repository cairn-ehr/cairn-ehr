# matcher/src/cairn_matcher/pipeline/runner.py
"""Orchestrate one pairwise proposal: load -> score -> veto -> band -> persist.

This is the only place IO (pipeline.db) and the pure core (orchestrator/scoring/banding)
meet. It computes a verdict for a single given pair; finding WHICH pairs to score
(blocking) is B2b. A pair below the review threshold persists nothing — the B3 hub
duplicate-sweep is the declared backstop for any signal missed at the noise floor.
"""

import uuid

from cairn_matcher.orchestrator import field_comparisons
from cairn_matcher.pipeline.alias import known_alias_evidence
from cairn_matcher.pipeline.banding import (
    DEFAULT_THRESHOLDS,
    Band,
    Thresholds,
    band,
    build_payload,
)
from cairn_matcher.scoring import DEFAULT_WEIGHTS, Weights, score


def canonical_pair(a, b) -> tuple[str, str]:
    """Order a patient-id pair canonically by uuid VALUE, emitted as lowercase text.

    The match_proposal CHECK (patient_low < patient_high) compares normalized `uuid`
    values, not their text form. Text-sorting the canonical string happens to agree for
    lowercase UUIDs but diverges for upper/mixed case — flipping the pair would violate
    the CHECK or, worse, store propose(a,b) and propose(b,a) as two mirror rows. We
    compare uuid.UUID objects (128-bit integer order = Postgres byte order) so any input
    case yields one stable row identity. Accepts str or uuid.UUID; this is pure (no DB),
    which is why it lives here rather than in db.py.
    """
    ua, ub = uuid.UUID(str(a)), uuid.UUID(str(b))
    return (str(ua), str(ub)) if ua < ub else (str(ub), str(ua))


def propose(
    conn,
    a,
    b,
    *,
    thresholds: Thresholds = DEFAULT_THRESHOLDS,
    weights: Weights = DEFAULT_WEIGHTS,
) -> Band | None:
    """Score the pair (a, b), gate on the in-DB veto, and persist a proposal if warranted.

    Returns the Band (AUTO_CANDIDATE | REVIEW) when a proposal is written, or None when
    the pair is below the review threshold (nothing persisted). The pair is stored in
    canonical (low, high) order so the row is symmetric in a and b.
    """
    # Imported lazily so `runner` (and its pure helper canonical_pair) is importable
    # without the optional `pipeline` extra; only an actual propose() call needs psycopg.
    from cairn_matcher.pipeline import db

    rec_a = db.load_candidate(conn, a)
    rec_b = db.load_candidate(conn, b)
    comparisons = field_comparisons(rec_a, rec_b)
    match_score = score(comparisons, weights)
    vetoes = db.match_veto(conn, a, b)

    # §5.5(a) known-alias recognition: does the pair match (partly) on a name a chart has
    # REPUDIATED as known-false? This is advisory evidence for the human reviewer — never a
    # suppression and never an auto-link (banding forces REVIEW when present).
    alias_evidence = known_alias_evidence(
        str(a), rec_a.names.value if rec_a.names else None, db.load_aliases(conn, a),
        str(b), rec_b.names.value if rec_b.names else None, db.load_aliases(conn, b),
    )
    band_value = band(
        match_score, vetoes, thresholds, has_known_alias=bool(alias_evidence)
    )
    if band_value is None:
        # Nothing to persist — but the load/veto SELECTs opened a read transaction. Close
        # it so a batch driver iterating many sub-threshold pairs does not pin the xmin
        # horizon (hold back vacuum) by leaving one snapshot open across the whole run.
        conn.rollback()
        return None
    low, high = canonical_pair(a, b)
    payload = build_payload(match_score, vetoes, band_value, weights, alias_evidence)
    db.upsert_proposal(conn, low, high, payload)
    # Commit boundary owned here: a batch caller wrapping propose() is not silently
    # committed mid-transaction by a helper function it doesn't control.
    conn.commit()
    return band_value
