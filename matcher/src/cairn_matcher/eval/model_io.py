"""JSON serialization for a LearnedModel (pure value transforms + a thin file edge).

Lets a learned model be written to disk and reloaded into the exact production types
(scoring.Weights, banding.Thresholds) so a future deployment could adopt it. No pipeline
code reads these files yet — this is an advisory desk artifact. Malformed input raises
ModelIOError loudly rather than silently defaulting (house rule #5).
"""

import json
from collections.abc import Mapping

from cairn_matcher.agreement import AgreementLevel
from cairn_matcher.eval.learner import LearnedModel, LearnMetadata
from cairn_matcher.pipeline.banding import Thresholds
from cairn_matcher.scoring import FieldWeights, Weights

_META_FIELDS = (
    "alpha", "recall_target", "margin", "train_pairs", "train_matches", "review_auto_collided",
)


class ModelIOError(ValueError):
    """The model JSON is structurally invalid (bad shape, unknown level, missing key)."""


def _weights_to_json(weights: Weights) -> dict:
    """{field: {LEVEL_NAME: weight}} — agreement levels keyed by their stable enum NAME."""
    return {
        field: {level.name: w for level, w in fw.weights.items()}
        for field, fw in weights.per_field.items()
    }


def _weights_from_json(obj: Mapping) -> Weights:
    """Inverse of _weights_to_json; rejects any unknown agreement-level name."""
    per_field: dict[str, FieldWeights] = {}
    for field, levels in obj.items():
        table: dict[AgreementLevel, float] = {}
        for name, w in levels.items():
            try:
                level = AgreementLevel[name]
            except KeyError as exc:
                raise ModelIOError(
                    f"unknown agreement level {name!r} for field {field!r}"
                ) from exc
            table[level] = float(w)
        per_field[field] = FieldWeights(table)
    return Weights(per_field=per_field)


def model_to_json(model: LearnedModel) -> dict:
    """Serialize a LearnedModel to a plain JSON-ready dict (weights/thresholds/metadata)."""
    return {
        "weights": _weights_to_json(model.weights),
        "thresholds": {"review": model.thresholds.review, "auto": model.thresholds.auto},
        "metadata": {f: getattr(model.metadata, f) for f in _META_FIELDS},
    }


def model_from_json(obj: Mapping) -> LearnedModel:
    """Reconstruct a LearnedModel from a decoded JSON mapping; raise on any missing key."""
    for key in ("weights", "thresholds", "metadata"):
        if key not in obj:
            raise ModelIOError(f"model JSON missing top-level key {key!r}")
    thr = obj["thresholds"]
    meta = obj["metadata"]
    try:
        thresholds = Thresholds(review=float(thr["review"]), auto=float(thr["auto"]))
        metadata = LearnMetadata(**{f: meta[f] for f in _META_FIELDS})
    except (KeyError, TypeError) as exc:
        raise ModelIOError(f"malformed thresholds/metadata: {exc}") from exc
    return LearnedModel(
        weights=_weights_from_json(obj["weights"]),
        thresholds=thresholds,
        metadata=metadata,
    )


def write_model(model: LearnedModel, path) -> None:
    """Write a LearnedModel to `path` as UTF-8 JSON (sorted keys, deterministic)."""
    with open(path, "w", encoding="utf-8") as fh:
        json.dump(model_to_json(model), fh, ensure_ascii=False, indent=2, sort_keys=True)


def read_model(path) -> LearnedModel:
    """Read and reconstruct a LearnedModel from a JSON file at `path`."""
    with open(path, encoding="utf-8") as fh:
        return model_from_json(json.load(fh))
