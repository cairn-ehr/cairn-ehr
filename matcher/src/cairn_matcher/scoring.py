"""The Fellegi–Sunter combiner: agreement vector + weights -> explainable match score.

Classic Fellegi–Sunter assigns each field, at each agreement level, a log-weight
log2(m/u) — positive when agreement is more likely under a match than a non-match,
negative for disagreement. The total match score is the sum of per-field log-weights: a
log-likelihood ratio. Two Cairn-specific properties:

  * INSUFFICIENT_DATA contributes EXACTLY ZERO — a missing field is never a penalty
    (§3.7, the no-data-is-never-disagreement principle).
  * each weight is scaled by provenance_factor(rank) — a *verified* clash or agreement
    weighs more than an *imported/unknown* one (§4.2, provenance-aware).

This module owns NO threshold and makes NO decision. It returns a score with a per-field
breakdown; banding it against the conservative auto-link threshold is the in-DB floor's
job. The m/u weights here are shipped DEFAULTS; learning them from local adjudication
data is a later slice (B3).
"""

from collections.abc import Mapping
from dataclasses import dataclass
from types import MappingProxyType

from cairn_matcher.agreement import AgreementLevel
from cairn_matcher.records import FieldComparison

_PROVENANCE_CEILING = 70  # cairn_provenance_rank's top tier (fact-proven), db/011


def provenance_factor(rank: int) -> float:
    """Map a provenance rank to an evidence-strength multiplier in [0.5, 1.0].

    Unknown provenance (rank 0) still contributes — it IS data — but at half strength;
    a fully verified value (rank >= 70) contributes at full strength. Monotonic.
    """
    clamped = max(0, min(rank, _PROVENANCE_CEILING))
    return 0.5 + 0.5 * (clamped / _PROVENANCE_CEILING)


@dataclass(frozen=True)
class FieldWeights:
    """log2(m/u) per agreement level for one field. Missing level -> 0.0 (no evidence)."""

    weights: Mapping[AgreementLevel, float]

    def __post_init__(self) -> None:
        """Wrap a plain dict in MappingProxyType so the table is truly immutable.

        frozen=True only blocks rebinding the `weights` attribute, not mutating the dict
        it points at. Since DEFAULT_WEIGHTS is a process-wide singleton, an unwrapped
        dict would let any caller silently poison scoring for every other record/tenant
        in the process. Same idiom as records.Name.
        """
        if isinstance(self.weights, dict):
            object.__setattr__(self, "weights", MappingProxyType(self.weights))

    def weight_for(self, level: AgreementLevel) -> float:
        return self.weights.get(level, 0.0)


@dataclass(frozen=True)
class Weights:
    """The deployment's per-field weight table (its locale tuning). Learning is B3."""

    per_field: Mapping[str, FieldWeights]

    def __post_init__(self) -> None:
        """Wrap the outer per-field dict immutably too (see FieldWeights.__post_init__)."""
        if isinstance(self.per_field, dict):
            object.__setattr__(self, "per_field", MappingProxyType(self.per_field))


@dataclass(frozen=True)
class FieldEvidence:
    """One field's contribution to the score — the explainability unit."""

    field: str
    level: AgreementLevel
    provenance_rank: int
    weight_contribution: float


@dataclass(frozen=True)
class MatchScore:
    """A match score (log-likelihood ratio) plus its per-field breakdown.

    sum(f.weight_contribution for f in fields) == total, always.
    """

    total: float
    fields: tuple[FieldEvidence, ...]


# Shipped default weights. Illustrative log2(m/u) magnitudes — B3 learns real ones from
# local data. Stronger, rarer agreements (a shared identifier, an exact DOB) weigh most;
# low-cardinality fields (sex, the composite sex-at-birth/administrative-sex field) weigh
# least; disagreements are negative.
DEFAULT_WEIGHTS = Weights(per_field={
    "dob": FieldWeights({
        AgreementLevel.EXACT: 6.0,
        AgreementLevel.PARTIAL: 1.5,
        AgreementLevel.DISAGREE: -4.0,
    }),
    "sex": FieldWeights({
        AgreementLevel.EXACT: 1.0,
        AgreementLevel.DISAGREE: -2.0,
    }),
    "name": FieldWeights({
        AgreementLevel.EXACT: 5.0,
        AgreementLevel.EDIT_DISTANCE: 2.5,
        AgreementLevel.DISAGREE: -2.0,
    }),
    "identifier": FieldWeights({
        AgreementLevel.EXACT: 8.0,  # positive-only (the comparator never emits DISAGREE)
    }),
})


def score(comparisons: list[FieldComparison], weights: Weights = DEFAULT_WEIGHTS) -> MatchScore:
    """Combine per-field agreements into a match score with a per-field breakdown."""
    evidence: list[FieldEvidence] = []
    for comp in comparisons:
        if comp.level is AgreementLevel.INSUFFICIENT_DATA:
            contribution = 0.0
        else:
            field_weights = weights.per_field.get(comp.field)
            base = field_weights.weight_for(comp.level) if field_weights else 0.0
            contribution = base * provenance_factor(comp.provenance_rank)
        evidence.append(
            FieldEvidence(comp.field, comp.level, comp.provenance_rank, contribution)
        )
    total = sum(e.weight_contribution for e in evidence)
    return MatchScore(total=total, fields=tuple(evidence))
