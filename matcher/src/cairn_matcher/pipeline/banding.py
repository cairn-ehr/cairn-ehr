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
from cairn_matcher.agreement import AgreementLevel
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


def _has_shared_identifier(score: MatchScore) -> bool:
    """Does the evidence carry a SHARED STRONG IDENTIFIER (identifier field EXACT)?

    A shared national/system identifier is the rare, high-value signal a veto must never
    silently bury: two charts sharing an identifier yet flagged (e.g. a verified-DOB
    clash) is the classic wrong-chart / mistyped-identifier contamination case.
    """
    return any(
        e.field == "identifier" and e.level is AgreementLevel.EXACT for e in score.fields
    )


def _corroborated_positive(score: MatchScore) -> bool:
    """≥2 fields contributing positive weight, and NO disagreeing field.

    The structural flood-control gate of the §5.4 unconfirmed-chart rule (design
    2026-07-05 §4): age-window overlap + shared sex (or overlap + a shared identifier
    off belongings) qualifies; a bare 11-year window overlap — which a sizeable DB
    satisfies for a double-digit share of charts — does not. Structural (a field count)
    rather than a score floor so it does not drift as provenance factors or weights
    change. The paper counterpart searches the registry on age AND sex, never age alone.
    """
    positives = sum(1 for e in score.fields if e.weight_contribution > 0)
    disagrees = any(e.level is AgreementLevel.DISAGREE for e in score.fields)
    return positives >= 2 and not disagrees


def band(
    score: MatchScore,
    vetoes: Sequence[VetoFinding],
    thresholds: Thresholds = DEFAULT_THRESHOLDS,
    *,
    has_known_alias: bool = False,
    unconfirmed: bool = False,
) -> Band | None:
    """Classify a scored pair into AUTO_CANDIDATE / REVIEW / None (no proposal).

    ANY veto finding (hard_veto or degrade_hold) forbids AUTO_CANDIDATE and caps the band
    at REVIEW — never an auto-link, never an auto-reject. A pair below the review
    threshold normally yields None (no positive signal to act on).

    Exception (ADR-0014 §6, never auto-reject): a veto coexisting with a SHARED STRONG
    IDENTIFIER is forced to REVIEW even below threshold. Otherwise the veto's own subject
    (e.g. a verified-DOB clash carrying a large negative weight) could drag the score
    sub-threshold and silently suppress the very anomaly it flags — an auto-reject in
    effect. The rescue is scoped to a shared identifier (not any veto) so it surfaces the
    contamination signal without flooding the worklist with common-name coincidences.

    Exception (§5.5(a) known alias): when the pair's name agreement is driven by a name a
    chart has REPUDIATED as known-false (`has_known_alias`), the band is REVIEW — never
    None (the "recognised returning persona" signal is never dropped below threshold) and
    never AUTO_CANDIDATE (two charts are never auto-linked on the strength of a name one of
    them declared false — §5.7 reserves that call for a Human). Like the shared-identifier
    rescue it can only ever SURFACE the pair, never auto-reject it.

    Deliberately UNSCOPED, unlike that rescue. The shared-identifier rescue is gated (fires
    only with a veto AND a shared identifier) precisely to avoid flooding the worklist with
    common-name coincidences. The known-alias forcing CANNOT borrow that discipline: the
    matcher cannot tell a returning fabricated persona from a real, different bearer of the
    same false name — from the name alone they are identical — so any score/veto gate would
    risk suppressing the very §5.5(a) recognition the alias pool exists to serve. So it
    flags every corroborated known-alias match and defers to a Human. Do NOT "fix" this into
    a scoped rule: that would silently drop returning-persona pairs. Worklist VOLUME is
    managed downstream instead — every such proposal carries a `known_alias` evidence entry
    (build_payload), so a hub worklist can filter/prioritise them without any suppression here.

    Exception (§5.4 unconfirmed-chart forcing): when an unconfirmed chart (still-to-be-
    identified) has corroborated matching evidence (≥2 positive fields, no disagreeing
    fields; the structural flood-control gate), the band is REVIEW — unconfirmed NEEDS human
    identification effort, so corroborated candidates must reach the worklist. Never
    AUTO_CANDIDATE (a human must make the link). The no-veto gate is near-vacuous for an
    unconfirmed Doe since a veto requires verified values, and the veto+identifier case is
    already owned by the rescue. `'under-review'` is a dispute state and deliberately does
    NOT trigger this.
    """
    if has_known_alias:
        return Band.REVIEW
    if score.total < thresholds.review:
        if vetoes and _has_shared_identifier(score):
            return Band.REVIEW
        if unconfirmed and not vetoes and _corroborated_positive(score):
            return Band.REVIEW
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
    alias_evidence: Sequence[dict] = (),
    trust_evidence: Sequence[dict] = (),
) -> ProposalPayload:
    """Shape a self-explaining proposal payload: the band, the score, and WHY (evidence
    breakdown + veto findings), plus the matcher version that produced it.

    `alias_evidence` (the §5.5(a) `known_alias` entries, if any) is appended after the
    field breakdown so a reviewer sees that the match involves a repudiated known alias —
    the paper-registry "known alias" flag, restored to the worklist.

    `trust_evidence` (the `identity_pending` marker and other trust/identity attestations,
    if any) is appended after the alias entries so a hub worklist can group an unconfirmed
    chart's candidate list — pure surfacing, never suppression.
    """
    evidence = tuple(
        {
            "field": e.field,
            "level": e.level.name,
            "provenance_rank": e.provenance_rank,
            "weight_contribution": e.weight_contribution,
        }
        for e in score.fields
    ) + tuple(alias_evidence) + tuple(trust_evidence)
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
