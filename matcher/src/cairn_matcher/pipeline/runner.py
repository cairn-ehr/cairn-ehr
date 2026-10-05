# matcher/src/cairn_matcher/pipeline/runner.py
"""Orchestrate one pairwise proposal: load -> score -> veto -> band -> persist.

This is the only place IO (pipeline.db) and the pure core (orchestrator/scoring/banding)
meet. It computes a verdict for a single given pair; finding WHICH pairs to score
(blocking) is B2b. A pair below the review threshold persists nothing — the B3 hub
duplicate-sweep is the declared backstop for any signal missed at the noise floor.
"""

from collections.abc import Mapping
from dataclasses import dataclass
from uuid import UUID

from cairn_matcher.orchestrator import DEFAULT_CONFIG, ComparatorConfig, field_comparisons
from cairn_matcher.pipeline.alias import known_alias_evidence
from cairn_matcher.pipeline.banding import (
    DEFAULT_THRESHOLDS,
    Band,
    ProposalPayload,
    Thresholds,
    band,
    build_payload,
)

# canonical_pair moved to the pure blocking module (one definition of pair identity for
# every pass shape); re-exported here because runner is its historical import path.
from cairn_matcher.pipeline.blocking import canonical_pair
from cairn_matcher.scoring import DEFAULT_WEIGHTS, Weights, score

__all__ = ["Assessment", "assess", "canonical_pair", "persist", "propose"]


@dataclass(frozen=True)
class Assessment:
    """One pair's verdict, decided but not yet written.

    `band` None means the pair is below the review floor: persisting it retracts a still-pending
    proposal (#135) and otherwise writes nothing. `payload` is None exactly when `band` is.
    `low`/`high` are the pair in canonical (low, high) lowercase-uuid-text order.
    """

    low: str
    high: str
    band: Band | None
    payload: ProposalPayload | None


def assess(
    conn,
    a,
    b,
    *,
    thresholds: Thresholds = DEFAULT_THRESHOLDS,
    weights: Weights = DEFAULT_WEIGHTS,
    config: ComparatorConfig = DEFAULT_CONFIG,
    aliases: Mapping[str, "frozenset[str]"] | None = None,
    trust: Mapping[str, str] | None = None,
) -> Assessment:
    """Score the pair, gate on the in-DB veto, band it — and write NOTHING.

    Everything propose() used to do before persisting, unchanged (see the comments carried over
    below). Reads only, so a caller can assess many pairs and then persist them all, plus its own
    bookkeeping, in one transaction it controls (the R4 worker).

    `config` is the per-field comparator wiring handed to field_comparisons (the ADR-0014
    locale-pack seam). thresholds/weights/config together are the pair's effective matcher
    configuration, and ALL of it flows into build_payload so the persisted matcher_version
    pins what actually scored and banded this pair (issue #100) — the ADR-0011/0029
    contamination-recall handle.

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
    # without the optional `pipeline` extra; only an actual assess() call needs psycopg.
    from cairn_matcher.pipeline import db

    rec_a = db.load_candidate(conn, a)
    rec_b = db.load_candidate(conn, b)
    comparisons = field_comparisons(rec_a, rec_b, config)
    match_score = score(comparisons, weights)
    vetoes = db.match_veto(conn, a, b)

    # The alias/trust map keys AND the persisted alias/trust markers all use canonical
    # lowercase uuid text, whatever id type/casing the caller passed (uuid.UUID round-trip =
    # the canonical_pair rule). load_aliases_for / load_trust_for key their maps this way, so a
    # non-canonical caller (an uppercase or braced uuid) must be canonicalized HERE or it
    # silently misses every map entry — for aliases that meant the §5.5(a) known-alias REVIEW
    # forcing quietly did not fire (issue #211 gap 1). Computed once for both lookups + labels.
    key_a, key_b = (str(UUID(str(p))) for p in (a, b))

    # §5.5(a) known-alias recognition: does the pair match (partly) on a name a chart has
    # REPUDIATED as known-false? This is advisory evidence for the human reviewer — never a
    # suppression and never an auto-link (banding forces REVIEW when present). Aliases come
    # from the caller's preloaded map when batching, else a single-pair on-demand load. The
    # map lookup uses the canonical key; the single-pair load passes `a`/`b` straight to the DB
    # (Postgres canonicalizes the uuid parameter itself, so no pre-canonicalization is needed).
    if aliases is None:
        aliases_a = db.load_aliases(conn, a)
        aliases_b = db.load_aliases(conn, b)
    else:
        aliases_a = aliases.get(key_a, frozenset())
        aliases_b = aliases.get(key_b, frozenset())
    alias_evidence = known_alias_evidence(
        key_a, rec_a.names.value if rec_a.names else None, aliases_a,
        key_b, rec_b.names.value if rec_b.names else None, aliases_b,
    )
    # §5.4 identity-pending trust: an *unconfirmed* chart (a standing John Doe) needs human
    # identification effort, so banding may force its corroborated pairs to REVIEW (design
    # 2026-07-05 §4). Trust states come from the caller's preloaded map when batching, else
    # per-pair on-demand loads (same seam as aliases).
    if trust is None:
        trust = db.load_trust_for(conn, (a, b))
    # key_a/key_b (the canonical lowercase uuid text) were computed above the alias block and
    # are reused here — the trust map is keyed canonically exactly like the alias map.
    trust_a = trust.get(key_a)
    trust_b = trust.get(key_b)
    unconfirmed_ids = sorted(
        k for k, t in ((key_a, trust_a), (key_b, trust_b)) if t == "unconfirmed"
    )
    band_value = band(
        match_score, vetoes, thresholds,
        has_known_alias=bool(alias_evidence), unconfirmed=bool(unconfirmed_ids),
    )
    low, high = canonical_pair(a, b)
    if band_value is None:
        # Nothing new to persist — but a PENDING proposal from an earlier sweep may survive
        # for this pair (e.g. the §5.4 forcing rule surfaced it while a chart was
        # 'unconfirmed', and it has since been identified — issue #135). persist() retracts
        # that stale row so a worklist stops grouping a resolved chart under a nonexistent
        # Doe; assess() only reports the below-floor verdict.
        return Assessment(low, high, None, None)
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
        match_score, vetoes, band_value, weights, alias_evidence, trust_evidence,
        thresholds=thresholds, config=config,
    )
    return Assessment(low, high, band_value, payload)


def persist(conn, assessment: Assessment) -> bool:
    """Write an assessment; NEVER commit. Returns True when a row was written or retracted.

    A banded pair is upserted (a human's status is preserved — db.upsert_proposal). A below-floor
    pair retracts a still-pending proposal, if any (#135). The caller owns the commit, so a
    caller can make these writes atomic with its own bookkeeping.
    """
    from cairn_matcher.pipeline import db

    if assessment.band is None:
        return bool(db.retract_pending_proposal(conn, assessment.low, assessment.high))
    db.upsert_proposal(conn, assessment.low, assessment.high, assessment.payload)
    return True


def propose(
    conn,
    a,
    b,
    *,
    thresholds: Thresholds = DEFAULT_THRESHOLDS,
    weights: Weights = DEFAULT_WEIGHTS,
    config: ComparatorConfig = DEFAULT_CONFIG,
    aliases: Mapping[str, "frozenset[str]"] | None = None,
    trust: Mapping[str, str] | None = None,
) -> Band | None:
    """Score the pair (a, b), gate on the in-DB veto, and persist a proposal if warranted.

    Returns the Band (AUTO_CANDIDATE | REVIEW) when a proposal is written, or None when
    the pair is below the review threshold (nothing persisted). The pair is stored in
    canonical (low, high) order so the row is symmetric in a and b.

    This is assess() + persist() + the commit boundary; see assess() for the meaning of
    `thresholds`, `weights`, `config`, `aliases` and `trust` (batch callers preload the
    alias/trust maps so no per-pair SELECT is issued).
    """
    verdict = assess(
        conn, a, b, thresholds=thresholds, weights=weights, config=config,
        aliases=aliases, trust=trust,
    )
    # Commit boundary owned here: a batch caller wrapping propose() is not silently
    # committed mid-transaction by a helper function it doesn't control. A write is made
    # durable; a pure read (nothing retracted, nothing banded) still ends its transaction
    # (rollback closes the read snapshot) so a batch driver does not pin the xmin horizon
    # across the whole run.
    if persist(conn, verdict):
        conn.commit()
    else:
        conn.rollback()
    return verdict.band
