"""Locate and load the bundled gold dataset shipped inside the package.

Kept separate from dataset.py so the pure value types carry no filesystem dependency:
dataset.load_dataset takes an already-decoded mapping; this module is the thin I/O edge
that reads JSON from disk.
"""

import json
from pathlib import Path

from cairn_matcher.eval.dataset import LabelledDataset, load_dataset

GOLD_PATH = Path(__file__).resolve().parent / "fixtures" / "gold_v1.json"


def load_dataset_file(path: Path | str) -> LabelledDataset:
    """Read a dataset JSON file from disk and parse it into a LabelledDataset."""
    with open(path, encoding="utf-8") as fh:
        return load_dataset(json.load(fh))


def load_bundled_gold() -> LabelledDataset:
    """Load the package's bundled gold_v1 dataset (the default CLI target)."""
    return load_dataset_file(GOLD_PATH)
