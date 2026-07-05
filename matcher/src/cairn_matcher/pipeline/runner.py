# matcher/src/cairn_matcher/pipeline/runner.py
"""Orchestrate one pairwise proposal: load -> score -> veto -> band -> persist.

This is the only place IO (pipeline.db) and the pure core (orchestrator/scoring/banding)
meet. It computes a verdict for a single given pair; finding WHICH pairs to score
(blocking) is B2b. A pair below the review threshold persists nothing — the B3 hub
duplicate-sweep is the declared backstop for any signal missed at the noise floor.
"""

from collections.abc import Mapping
from uuid import UUID

from cairn_matcher.orchestrator import field_comparisons
from cairn_matcher.pipeline.alias import known_alias_evidence
from cairn_matcher.pipeline.banding import (
    DEFAULT_THRESHOLDS,
    Band,
    Thresholds,
    band,
    build_payload,
)

# canonical_pair moved to the pure blocking module (one definition of pair identity for
# every pass shape); re-exported here because runner is its historical import path.
from cairn_matcher.pipeline.blocking import canonical_pair
from cairn_matcher.scoring import DEFAULT_WEIGHTS, Weights, score

__all__ = ["canonical_pair", "propose"]


def propose(
    conn,
    a,
    b,
    *,
    thresholds: Thresholds = DEFAULT_THRESHOLDS,
    weights: Weights = DEFAULT_WEIGHTS,
    aliases: Mapping[str, "frozenset[str]"] | None = None,
    trust: Mapping[str, str] | None = None,
) -> Band | None:
    """Score the pair (a, b), gate on the in-DB veto, and persist a proposal if warranted.

    Returns the Band (AUTO_CANDIDATE | REVIEW) when a proposal is written, or None when
    the pair is below the review threshold (nothing persisted). The pair is stored in
    canonical (low, high) order so the row is symmetric in a and b.

    `aliases` is an optional preloaded {patient_id_text: known-aliases} lookup a BATCH
    caller (sweep) supplies so this function issues no per-pair alias SELECT — it reads
    both charts' aliases from the map instead. A direct single-pair call leaves it None
    and each chart's aliases are loaded on demand (`db.load_aliases`).

    `trust` is the analogous preloaded batch map of {patient_id_text: trust_state} a BATCH
    caller (sweep) supplies so this function issues no per-pair trust SELECT. A direct
    single-pair call leaves it None and the pair's trust states are loaded on demand in
    one query (`db.load_trust_for`) — the same seam as aliases.
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
    # suppression and never an auto-link (banding forces REVIEW when present). Aliases come
    # from the caller's preloaded map when batching, else a single-pair on-demand load.
    if aliases is None:
        aliases_a = db.load_aliases(conn, a)
        aliases_b = db.load_aliases(conn, b)
    else:
        aliases_a = aliases.get(str(a), frozenset())
        aliases_b = aliases.get(str(b), frozenset())
    alias_evidence = known_alias_evidence(
        str(a), rec_a.names.value if rec_a.names else None, aliases_a,
        str(b), rec_b.names.value if rec_b.names else None, aliases_b,
    )
    # §5.4 identity-pending trust: an *unconfirmed* chart (a standing John Doe) needs human
    # identification effort, so banding may force its corroborated pairs to REVIEW (design
    # 2026-07-05 §4). Trust states come from the caller's preloaded map when batching, else
    # per-pair on-demand loads (same seam as aliases).
    if trust is None:
        trust = db.load_trust_for(conn, (a, b))
    # The map keys AND the persisted marker use canonical lowercase uuid text, whatever
    # id type/casing the caller passed (uuid.UUID round-trip = the canonical_pair rule).
    key_a, key_b = (str(UUID(str(p))) for p in (a, b))
    trust_a = trust.get(key_a)
    trust_b = trust.get(key_b)
    unconfirmed_ids = sorted(
        k for k, t in ((key_a, trust_a), (key_b, trust_b)) if t == "unconfirmed"
    )
    band_value = band(
        match_score, vetoes, thresholds,
        has_known_alias=bool(alias_evidence), unconfirmed=bool(unconfirmed_ids),
    )
    if band_value is None:
        # Nothing to persist — but the load/veto SELECTs opened a read transaction. Close
        # it so a batch driver iterating many sub-threshold pairs does not pin the xmin
        # horizon (hold back vacuum) by leaving one snapshot open across the whole run.
        conn.rollback()
        return None
    low, high = canonical_pair(a, b)
    # The marker is emitted on EVERY persisted proposal involving an unconfirmed chart —
    # also above-threshold ones — so a hub worklist can group a Doe's whole candidate list.
    # "kind" is the one discriminator key for every non-field evidence entry (the
    # known_alias convention) — evidence JSONB is immutable, so a second key style
    # would burden every future consumer forever.
    trust_evidence = (
        ({"kind": "identity_pending", "unconfirmed": unconfirmed_ids},)
        if unconfirmed_ids else ()
    )
    payload = build_payload(
        match_score, vetoes, band_value, weights, alias_evidence, trust_evidence
    )
    db.upsert_proposal(conn, low, high, payload)
    # Commit boundary owned here: a batch caller wrapping propose() is not silently
    # committed mid-transaction by a helper function it doesn't control.
    conn.commit()
    return band_value
