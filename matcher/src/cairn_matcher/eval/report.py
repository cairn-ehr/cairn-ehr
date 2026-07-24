"""Render metric bundles to a plain-text report.

Pure string formatting — no scoring, no DB. Kept separate from the metric computation so
the numbers can be consumed programmatically (weight-learning) without the prose, and the
prose can change without touching the math.
"""

from cairn_matcher.eval.metrics import OperatingPoint, ScorerMetrics

_CAVEAT = (
    "NOTE: on a small hand-authored set these numbers are a regression/tuning "
    "instrument, not a statistical accuracy claim."
)


def _op_line(op: OperatingPoint) -> str:
    """One operating-point row: precision / recall / F1 to three decimals."""
    return (f"  {op.name:<8} precision={op.precision:.3f} "
            f"recall={op.recall:.3f} f1={op.f1:.3f}")


def _repaired_line(metrics: ScorerMetrics) -> str:
    """The #290 honesty line: how many true pairs a synthetic repair made artificially easy.

    N of M where M = total true pairs (all confusion match cells). When N > 0 it appends the
    optimism this implies (recall/f1 above are inflated by up to N EXACT-graded true pairs); at
    N == 0 (every real dataset) the bare 'N of M' reads as the honest 'nothing to discount'.
    """
    c = metrics.confusion
    true_total = c.match_auto + c.match_review + c.match_none
    line = (
        f"  repaired match pairs (synthetically eased): "
        f"{metrics.repaired_match_pairs} of {true_total}"
    )
    if metrics.repaired_match_pairs:
        line += (
            f"  — recall/f1 above optimistic by up to "
            f"{metrics.repaired_match_pairs} true pair(s)"
        )
    return line


def format_scorer(metrics: ScorerMetrics, *, dataset_name: str = "") -> str:
    """Render scorer metrics: confusion, both operating points, the danger rates, spread."""
    c = metrics.confusion
    title = f"Scorer eval — {dataset_name}" if dataset_name else "Scorer eval"
    lines = [
        title,
        f"  pairs evaluated: {metrics.pair_count}",
        "  confusion (truth x band):",
        f"    match    : auto={c.match_auto} review={c.match_review} none={c.match_none}",
        f"    non-match: auto={c.nonmatch_auto} review={c.nonmatch_review} none={c.nonmatch_none}",
        _op_line(metrics.strict),
        _op_line(metrics.lenient),
        f"  auto_false_link_rate={metrics.auto_false_link_rate:.3f}  "
        f"missed_match_rate={metrics.missed_match_rate:.3f}",
        f"  match scores    : n={metrics.match_scores.count} "
        f"min={metrics.match_scores.minimum:.2f} med={metrics.match_scores.median:.2f} "
        f"max={metrics.match_scores.maximum:.2f}",
        f"  non-match scores: n={metrics.nonmatch_scores.count} "
        f"min={metrics.nonmatch_scores.minimum:.2f} med={metrics.nonmatch_scores.median:.2f} "
        f"max={metrics.nonmatch_scores.maximum:.2f}",
        _repaired_line(metrics),
        _CAVEAT,
    ]
    return "\n".join(lines)


def format_blocking(metrics) -> str:
    """Render blocking metrics. Duck-typed on the BlockingMetrics fields (Task 8) so this
    pure module never imports the psycopg-adjacent blocking layer."""
    lines = [
        "Blocking eval",
        f"  pair_completeness={metrics.pair_completeness:.3f} (blocking recall ceiling)",
        f"  reduction_ratio={metrics.reduction_ratio:.3f}",
        f"  generated_pairs={metrics.generated_pairs} of {metrics.total_pairs} possible",
        f"  skipped oversized blocks: {len(metrics.skipped_blocks)} "
        f"(dropped_pair_estimate={metrics.dropped_pair_estimate})",
        f"  dropped TRUE matches (blocking misses): {len(metrics.dropped_true_matches)}",
    ]
    for low, high in metrics.dropped_true_matches:
        lines.append(f"    - {low} / {high}")
    return "\n".join(lines)
