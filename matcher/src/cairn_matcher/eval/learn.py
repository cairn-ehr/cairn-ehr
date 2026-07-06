"""`python -m cairn_matcher.eval.learn [dataset.json]` — learn matcher weights + thresholds.

The disk/CLI edge for the pure learner (learner.py / crossval.py stay filesystem-free).
Prints a k-fold held-out before/after lift report; with --out, ALSO learns a model on the
full dataset and writes it as JSON. Defaults to the bundled gold set.

    python -m cairn_matcher.eval.learn --folds 5
    python -m cairn_matcher.eval.generate --entities 400 --seed 1 --out synth.json
    python -m cairn_matcher.eval.learn synth.json --folds 5 --out learned.json

PoC: ships the mechanism, not the shipped defaults (§5.13 / ADR-0014). See
docs/superpowers/specs/2026-07-06-b3-weight-learning-design.md for the honest limits.
"""

import argparse
import sys

from cairn_matcher.eval.crossval import format_lift, kfold_lift
from cairn_matcher.eval.dataset import DatasetError
from cairn_matcher.eval.learner import learn_model
from cairn_matcher.eval.loader import load_bundled_gold, load_dataset_file
from cairn_matcher.eval.model_io import write_model


def main(argv: list[str] | None = None) -> int:
    """Parse args, run the k-fold lift, print it, optionally write a full-data model.

    Returns a process exit code: 0 on success, 2 if the dataset could not be loaded.
    """
    parser = argparse.ArgumentParser(prog="cairn_matcher.eval.learn", description=__doc__)
    parser.add_argument(
        "dataset", nargs="?",
        help="path to a dataset JSON file; default: the bundled gold_v1 set",
    )
    parser.add_argument("--folds", type=int, default=5, help="k-fold count (>= 2)")
    parser.add_argument("--recall-target", type=float, default=0.99,
                        help="fraction of true matches the review threshold must surface")
    parser.add_argument("--margin", type=float, default=0.5,
                        help="added above max non-match score for the auto threshold")
    parser.add_argument("--alpha", type=float, default=0.5,
                        help="Laplace smoothing pseudo-count (> 0)")
    parser.add_argument("--out", help="write a full-dataset learned model to this JSON path")
    args = parser.parse_args(argv)

    try:
        ds = load_dataset_file(args.dataset) if args.dataset else load_bundled_gold()
    except (DatasetError, OSError, ValueError) as exc:
        print(f"error: could not load dataset: {exc}", file=sys.stderr)
        return 2

    report = kfold_lift(
        ds, folds=args.folds, alpha=args.alpha,
        recall_target=args.recall_target, margin=args.margin,
    )
    print(format_lift(report, dataset_name=ds.name))

    if args.out:
        model = learn_model(
            ds, alpha=args.alpha, recall_target=args.recall_target, margin=args.margin,
        )
        write_model(model, args.out)
        print(f"\nwrote full-dataset learned model to {args.out}")
    return 0


if __name__ == "__main__":  # pragma: no cover
    raise SystemExit(main())
