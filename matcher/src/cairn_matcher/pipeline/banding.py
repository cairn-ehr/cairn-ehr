"""Band a match score (gated by the db/016 veto findings) and shape the proposal payload.

This module owns the conservative auto-link threshold B1 deliberately did NOT (B1 returns
a raw score; the decision to act lives here, on the advisory side). It is pure: no DB.

Banding rule (priority order), honouring db/016's "never auto-link, never auto-reject":
  * total >= auto AND no veto findings (any severity)        -> AUTO_CANDIDATE
  * total >= review (incl. a high score capped by any veto)  -> REVIEW
  * total <  review                                          -> None  (persist nothing)

The thresholds here are SHIPPED DEFAULTS — illustrative magnitudes. Learning real ones
from local adjudication data is B3. Note the provenance_factor 0.5 floor (scoring.py)
halves every field at unknown provenance, so defaults are chosen with that in mind.
"""

import hashlib
from collections.abc import Sequence
from dataclasses import dataclass
from enum import Enum

from cairn_matcher import __version__
from cairn_matcher.scoring import DEFAULT_WEIGHTS, MatchScore, Weights


class Band(Enum):
    """The advisory disposition of a scored pair. Persisted as the string value."""

    AUTO_CANDIDATE = "auto_candidate"
    REVIEW = "review"


@dataclass(frozen=True)
class VetoFinding:
    """One row returned by the in-DB cairn_match_veto floor (carried verbatim)."""

    veto_kind: str
    severity: str
    subject: str
    detail: str


@dataclass(frozen=True)
class Thresholds:
    """The two conservative score cut-offs. review < auto. Defaults below; B3 learns."""

    review: float
    auto: float


DEFAULT_THRESHOLDS = Thresholds(review=3.0, auto=8.0)


def band(
    score: MatchScore,
    vetoes: Sequence[VetoFinding],
    thresholds: Thresholds = DEFAULT_THRESHOLDS,
) -> Band | None:
    """Classify a scored pair into AUTO_CANDIDATE / REVIEW / None (no proposal).

    ANY veto finding (hard_veto or degrade_hold) forbids AUTO_CANDIDATE and caps the band
    at REVIEW — never an auto-link, never an auto-reject. A pair below the review
    threshold yields None regardless of vetoes (no positive signal to act on).
    """
    if score.total < thresholds.review:
        return None
    if score.total >= thresholds.auto and not vetoes:
        return Band.AUTO_CANDIDATE
    return Band.REVIEW


def matcher_version(weights: Weights = DEFAULT_WEIGHTS) -> str:
    """A version-pin string for a proposal: package version + a digest of the weights.

    ADR-0014 makes the matcher a config-version-pinned actor. This is the lightweight
    slice of that: a proposal records WHICH matcher config produced it, so a re-run with
    different weights is distinguishable. Full §7.5 actor registration/signing is B3.
    """
    items = sorted(
        (field, level.name, w)
        for field, fw in weights.per_field.items()
        for level, w in fw.weights.items()
    )
    digest = hashlib.sha256(repr(items).encode()).hexdigest()[:12]
    return f"{__version__}+{digest}"


@dataclass(frozen=True)
class ProposalPayload:
    """Everything db.upsert_proposal needs, already JSON-serializable for the JSONB cols."""

    score_total: float
    band: Band
    veto_findings: tuple[dict, ...]
    evidence: tuple[dict, ...]
    matcher_version: str


def build_payload(
    score: MatchScore,
    vetoes: Sequence[VetoFinding],
    band_value: Band,
    weights: Weights = DEFAULT_WEIGHTS,
) -> ProposalPayload:
    """Shape a self-explaining proposal payload: the band, the score, and WHY (evidence
    breakdown + veto findings), plus the matcher version that produced it."""
    evidence = tuple(
        {
            "field": e.field,
            "level": e.level.name,
            "provenance_rank": e.provenance_rank,
            "weight_contribution": e.weight_contribution,
        }
        for e in score.fields
    )
    findings = tuple(
        {"veto_kind": v.veto_kind, "severity": v.severity, "subject": v.subject, "detail": v.detail}
        for v in vetoes
    )
    return ProposalPayload(
        score_total=score.total,
        band=band_value,
        veto_findings=findings,
        evidence=evidence,
        matcher_version=matcher_version(weights),
    )
