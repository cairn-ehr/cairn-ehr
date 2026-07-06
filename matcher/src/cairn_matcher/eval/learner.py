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

from cairn_matcher.agreement import AgreementLevel
from cairn_matcher.eval.dataset import (
    LabelledDataset,
    all_pairs,
    record_to_candidate,
    truth_pairs,
)
from cairn_matcher.orchestrator import DEFAULT_CONFIG, ComparatorConfig, field_comparisons
from cairn_matcher.records import FieldComparison
from cairn_matcher.scoring import FieldWeights, Weights

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
