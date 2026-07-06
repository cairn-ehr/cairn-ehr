"""Supervised Fellegi–Sunter weight + threshold learning from a labelled dataset.

Pure, deterministic, no I/O. The scorer (scoring.score) sums per-field log2(m/u)
log-weights; this module estimates those weights the canonical F-S way — by counting
agreement levels across labelled match / non-match pairs — then (Task 2/3) derives the two
banding thresholds that go with them. Same math as scoring, run backwards from ground truth.

PoC (design docs/superpowers/specs/2026-07-06-b3-weight-learning-design.md): ships the
mechanism, not the shipped defaults. Production weights come from real local adjudication
(§5.13 / ADR-0014); synthetic-learned weights reflect the generator's corruption model.
"""

import math
from collections.abc import Sequence
from dataclasses import dataclass

from cairn_matcher.agreement import AgreementLevel
from cairn_matcher.eval.dataset import (
    LabelledDataset,
    all_pairs,
    record_to_candidate,
    truth_pairs,
)
from cairn_matcher.orchestrator import DEFAULT_CONFIG, ComparatorConfig, field_comparisons
from cairn_matcher.pipeline.banding import Thresholds
from cairn_matcher.records import FieldComparison
from cairn_matcher.scoring import FieldWeights, Weights, score

LabelledPair = tuple[bool, list[FieldComparison]]


def labelled_comparisons(
    ds: LabelledDataset, config: ComparatorConfig = DEFAULT_CONFIG
) -> list[LabelledPair]:
    """(is_match, field comparisons) for every record pair via the REAL scoring path.

    Reuses the production adapter (record_to_candidate) + orchestrator (field_comparisons)
    so the learner sees exactly what ships; candidates are built once per record, not once
    per pair. The ground-truth label is membership in the dataset's within-cluster pairs.
    """
    candidates = {r.record_id: record_to_candidate(r) for r in ds.all_records()}
    truth = truth_pairs(ds)
    out: list[LabelledPair] = []
    for low, high in all_pairs(ds):
        comps = field_comparisons(candidates[low], candidates[high], config)
        out.append(((low, high) in truth, comps))
    return out


def estimate_weights(labelled: Sequence[LabelledPair], *, alpha: float = 0.5) -> Weights:
    """Learn a Weights table by counting agreement levels across labelled pairs.

    For each field f and each level L it prices (INSUFFICIENT_DATA excluded — a missing
    field is zero evidence, never a weight, §3.7), estimate
        m = P(L | match, f comparable),  u = P(L | non-match, f comparable)
    with additive (Laplace) smoothing (alpha) over the field's observed level set, so a
    zero-count cell yields a bounded weight, never ±inf. weight = log2(m / u).

    Every field seen in ANY comparison gets an entry (empty if it was never comparable in
    training) so score() never raises on held-out data for a known field; an empty entry
    scores 0.0 at every level — the honest 'no learned evidence' degrade.
    """
    if alpha <= 0.0:
        raise ValueError(f"alpha (Laplace smoothing) must be > 0, got {alpha}")
    if not labelled:
        raise ValueError("estimate_weights needs at least one labelled pair")

    # counts[field] = {"match": {level: n}, "nonmatch": {level: n}}; seen_fields tracks
    # every field name, including those only ever INSUFFICIENT_DATA, for the entry guarantee.
    counts: dict[str, dict[str, dict[AgreementLevel, int]]] = {}
    seen_fields: set[str] = set()
    for is_match, comps in labelled:
        cls = "match" if is_match else "nonmatch"
        for comp in comps:
            seen_fields.add(comp.field)
            if comp.level is AgreementLevel.INSUFFICIENT_DATA:
                continue
            bucket = counts.setdefault(comp.field, {"match": {}, "nonmatch": {}})
            bucket[cls][comp.level] = bucket[cls].get(comp.level, 0) + 1

    per_field: dict[str, FieldWeights] = {}
    for f in seen_fields:
        bucket = counts.get(f, {"match": {}, "nonmatch": {}})
        levels = sorted(set(bucket["match"]) | set(bucket["nonmatch"]))
        if not levels:
            per_field[f] = FieldWeights({})  # only ever INSUFFICIENT_DATA -> prices nothing
            continue
        k = len(levels)
        m_total = sum(bucket["match"].values())
        u_total = sum(bucket["nonmatch"].values())
        weights: dict[AgreementLevel, float] = {}
        for level in levels:
            m = (bucket["match"].get(level, 0) + alpha) / (m_total + alpha * k)
            u = (bucket["nonmatch"].get(level, 0) + alpha) / (u_total + alpha * k)
            weights[level] = math.log2(m / u)
        per_field[f] = FieldWeights(weights)
    return Weights(per_field=per_field)


def derive_thresholds(
    scored: Sequence[tuple[bool, float]],
    *,
    recall_target: float = 0.99,
    margin: float = 0.5,
) -> tuple[Thresholds, bool]:
    """Pick (review, auto) safety-first from labelled scores. Returns (thresholds, collided).

    Both thresholds anchor to the BEST-SCORING non-match (the strongest impostor the data
    contains) — that anchoring is what makes the placement safety-first and keeps the
    band() invariant review <= auto true by construction:

      * auto = max(non-match score) + margin -> ZERO false auto-links: no non-match reaches
        auto (an auto-link is an un-attested, if recallable, link; false-auto is the
        matcher's stated dangerous rate).
      * review = max(non-match score) -> surface any pair that OUT-SCORES the best impostor,
        never below it (a review below the top non-match would flood the worklist with
        impostor-grade pairs — the opposite of safety-first). Strictly below auto whenever
        margin > 0, so review <= auto always holds — no inversion, no clamp, however well or
        poorly the classes separate. (No non-matches at all -> fall back to the match range:
        review = min(match), auto = max(match) + margin.)

    margin must be > 0: a non-positive margin collapses (margin == 0) or inverts
    (margin < 0) the review < auto safety gap, which can produce a false auto-link.

    recall_target (default 0.99) is a DIAGNOSTIC, not a lever on review. With review fixed at
    the safe placement, 'collided' is True when that placement fails to surface
    recall_target of the true matches (achieved_recall < recall_target) — i.e. some true
    matches are entangled BELOW the best impostor, so safety-first placement and the recall
    floor genuinely conflict on this data. The learner flags, never compromises: it will not
    drag review into the impostor range to chase recall.
    """
    if not 0.0 < recall_target <= 1.0:
        raise ValueError(f"recall_target must be in (0, 1], got {recall_target}")
    if margin <= 0.0:
        raise ValueError(
            f"margin must be > 0 (a non-positive margin collapses or inverts the "
            f"review<auto safety gap and can produce a false auto-link), got {margin}"
        )
    match_scores = [s for is_m, s in scored if is_m]
    nonmatch_scores = [s for is_m, s in scored if not is_m]
    if not match_scores:
        raise ValueError("derive_thresholds needs at least one true-match pair")

    if nonmatch_scores:
        review = max(nonmatch_scores)
    else:
        review = min(match_scores)
    auto = review + margin

    surfaced = sum(1 for s in match_scores if s >= review)
    achieved_recall = surfaced / len(match_scores)
    collided = achieved_recall < recall_target

    return Thresholds(review=review, auto=auto), collided


@dataclass(frozen=True)
class LearnMetadata:
    """Provenance of a learned model: the knobs + training-set size + collision flag."""

    alpha: float
    recall_target: float
    margin: float
    train_pairs: int
    train_matches: int
    review_auto_collided: bool


@dataclass(frozen=True)
class LearnedModel:
    """A learned (weights, thresholds) pair plus the metadata that produced it.

    weights/thresholds are the exact production types (scoring.Weights,
    banding.Thresholds), so a caller can drop them straight into score()/band().
    """

    weights: Weights
    thresholds: Thresholds
    metadata: LearnMetadata


def learn_model(
    ds: LabelledDataset,
    *,
    config: ComparatorConfig = DEFAULT_CONFIG,
    alpha: float = 0.5,
    recall_target: float = 0.99,
    margin: float = 0.5,
) -> LearnedModel:
    """Estimate weights, then derive the thresholds that go with them, from one dataset.

    Coupled by design: learned weights rescale the total score, so the shipped
    (review, auto) defaults are meaningless afterward — thresholds are re-derived on the
    SAME training pairs, scored with the freshly-learned weights (provenance applied via
    the real score()).
    """
    labelled = labelled_comparisons(ds, config)
    weights = estimate_weights(labelled, alpha=alpha)
    scored = [(is_m, score(comps, weights).total) for is_m, comps in labelled]
    thresholds, collided = derive_thresholds(
        scored, recall_target=recall_target, margin=margin
    )
    metadata = LearnMetadata(
        alpha=alpha,
        recall_target=recall_target,
        margin=margin,
        train_pairs=len(labelled),
        train_matches=sum(1 for is_m, _ in labelled if is_m),
        review_auto_collided=collided,
    )
    return LearnedModel(weights=weights, thresholds=thresholds, metadata=metadata)
