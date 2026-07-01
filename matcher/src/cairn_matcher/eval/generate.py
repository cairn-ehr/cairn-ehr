"""`python -m cairn_matcher.eval.generate` — emit a synthetic blocking-eval dataset JSON.

The disk/CLI edge for generator.py (which stays pure/filesystem-free). Write to --out, or
stdout if omitted. Feed the result to the existing eval CLI:

    python -m cairn_matcher.eval.generate --entities 200 --seed 1 --out synth.json
    CAIRN_TEST_PG="host=... port=5532 ..." python -m cairn_matcher.eval synth.json
"""

import argparse
import json
import sys

from cairn_matcher.eval.generator import GenSpec, generate_dataset


def write_dataset(path, mapping):
    """Write a dataset mapping to `path` as UTF-8 JSON (non-ASCII preserved for legibility)."""
    with open(path, "w", encoding="utf-8") as fh:
        json.dump(mapping, fh, ensure_ascii=False, indent=2, sort_keys=True)


def main(argv=None):
    """Parse args, generate the dataset, write it. Returns a process exit code."""
    parser = argparse.ArgumentParser(prog="cairn_matcher.eval.generate", description=__doc__)
    parser.add_argument("--entities", type=int, default=200, help="number of entities (true pairs)")
    parser.add_argument("--seed", type=int, default=0, help="PRNG seed (reproducibility)")
    parser.add_argument("--out", help="output path; stdout if omitted")
    args = parser.parse_args(argv)

    dataset = generate_dataset(GenSpec(seed=args.seed, n_entities=args.entities))
    if args.out:
        write_dataset(args.out, dataset)
    else:
        json.dump(dataset, sys.stdout, ensure_ascii=False, indent=2, sort_keys=True)
        sys.stdout.write("\n")
    return 0


if __name__ == "__main__":  # pragma: no cover
    raise SystemExit(main())
