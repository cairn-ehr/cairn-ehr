"""The labelled-dataset format the harness measures, plus its loader.

Ground truth is expressed as ENTITY CLUSTERS: records grouped by the real person they
describe. Within-cluster record pairs are true matches; cross-cluster pairs are true
non-matches. That avoids hand-labelling O(n^2) pairs.

A dataset record deliberately mirrors the projection-row SHAPE the matcher already
operates over, so the pure scorer eval and the DB blocking eval both derive from one
shape with no parallel construction logic (see record_to_candidate / blocking_eval).
"""

import itertools
from collections.abc import Mapping, Sequence
from dataclasses import dataclass

from cairn_matcher.pipeline.adapter import candidate_from_rows
from cairn_matcher.records import CandidateRecord


class DatasetError(ValueError):
    """The dataset JSON is structurally invalid (missing/duplicate ids, wrong shape).

    Raised loudly rather than silently tolerated (house rule #5): a malformed eval set
    would otherwise produce quietly-wrong metrics.
    """


@dataclass(frozen=True)
class DatasetRecord:
    """One patient record as projection-shaped field dicts. Every field is optional
    except record_id; absence is a safe, gradeable absence (principle 4), not an error.

    dob: {"value": ISO str, "precision": "year"|"month"|"day", "provenance_rank": int}
         — or a §5.4 estimated-age window: {"value": "<yyyy>/<yyyy>",
         "precision": "year-range", "provenance_rank": int}
    sex_at_birth: {"value": str, "provenance_rank": int}
    administrative_sex: {"value": str, "provenance_rank": int} — the apparent/phenotypic
         facet a clinician-observed sex lands on (slice D's composite-sex fallback input)
    names: tuple of {"value": display str, "provenance_rank": int}
    identifiers: tuple of {"system": str, "match_key": str, "value": str}
    """

    record_id: str
    dob: Mapping | None = None
    sex_at_birth: Mapping | None = None
    administrative_sex: Mapping | None = None
    names: tuple[Mapping, ...] = ()
    identifiers: tuple[Mapping, ...] = ()


@dataclass(frozen=True)
class EntityCluster:
    """All records that describe ONE real person — the ground-truth grouping."""

    entity_id: str
    records: tuple[DatasetRecord, ...]


@dataclass(frozen=True)
class LabelledDataset:
    """A named set of entity clusters: the unit the harness evaluates."""

    name: str
    entities: tuple[EntityCluster, ...]
    description: str = ""

    def all_records(self) -> tuple[DatasetRecord, ...]:
        """Every record across all clusters, in cluster-then-record declaration order."""
        return tuple(r for e in self.entities for r in e.records)


def _require_keys(items: Sequence, keys: tuple[str, ...], *, what: str, record_id: str) -> None:
    """Raise a located DatasetError if any item is missing a key its consumers dereference.

    record_to_candidate (and blocking_eval seeding) index these keys directly, so a missing
    one would otherwise surface as an opaque KeyError deep in the pipeline rather than a
    loud, located error at load time (house rule #5).
    """
    for item in items:
        missing = [k for k in keys if k not in item]
        if missing:
            raise DatasetError(
                f"record {record_id!r}: each {what} needs {keys}, missing {missing} in {item!r}"
            )


def _record_from(obj: Mapping) -> DatasetRecord:
    """Shape one record dict into a DatasetRecord; require a non-empty record_id."""
    record_id = obj.get("record_id")
    if not isinstance(record_id, str) or not record_id:
        raise DatasetError(f"each record needs a non-empty string record_id, got {obj!r}")
    names = tuple(obj.get("names", ()))
    identifiers = tuple(obj.get("identifiers", ()))
    _require_keys(names, ("value",), what="name", record_id=record_id)
    _require_keys(
        identifiers, ("system", "match_key", "value"), what="identifier", record_id=record_id
    )
    return DatasetRecord(
        record_id=record_id,
        dob=obj.get("dob"),
        sex_at_birth=obj.get("sex_at_birth"),
        administrative_sex=obj.get("administrative_sex"),
        names=names,
        identifiers=identifiers,
    )


def load_dataset(obj: Mapping) -> LabelledDataset:
    """Parse an in-memory dataset mapping (already JSON-decoded) into typed clusters.

    Validates the two invariants the harness depends on: there is an `entities` list,
    and every record_id is unique across the whole dataset (pairs are keyed by id).
    """
    entities_raw = obj.get("entities")
    if not isinstance(entities_raw, Sequence) or isinstance(entities_raw, (str, bytes)):
        raise DatasetError("dataset needs an 'entities' list")

    seen_ids: set[str] = set()
    entities: list[EntityCluster] = []
    for ent in entities_raw:
        entity_id = ent.get("entity_id")
        if not isinstance(entity_id, str) or not entity_id:
            raise DatasetError(f"each entity needs a non-empty entity_id, got {ent!r}")
        records = tuple(_record_from(r) for r in ent.get("records", ()))
        for r in records:
            if r.record_id in seen_ids:
                raise DatasetError(f"duplicate record_id across dataset: {r.record_id!r}")
            seen_ids.add(r.record_id)
        entities.append(EntityCluster(entity_id=entity_id, records=records))

    return LabelledDataset(
        name=str(obj.get("name", "")),
        description=str(obj.get("description", "")),
        entities=tuple(entities),
    )


def record_to_candidate(rec: DatasetRecord) -> CandidateRecord:
    """Map a dataset record to a B1 CandidateRecord via the REAL projection adapter.

    The eval scores the same path production does: the only transform here is reshaping
    the dataset's flat dob dict into the projection's {value, facets:{precision}, ...}
    row shape candidate_from_rows expects. Everything else (DOB precision-gating, name
    token-bagging, identifier keying, safe degrade on absence) is the adapter's, reused
    verbatim so the eval can never drift from the production mapping.
    """
    dob_row = None
    if rec.dob is not None:
        dob_row = {
            "value": rec.dob.get("value"),
            "facets": {"precision": rec.dob.get("precision")},
            "provenance_rank": rec.dob.get("provenance_rank", 0),
        }
    sex_row = None
    if rec.sex_at_birth is not None:
        sex_row = {
            "value": rec.sex_at_birth.get("value"),
            "provenance_rank": rec.sex_at_birth.get("provenance_rank", 0),
        }
    admin_sex_row = None
    if rec.administrative_sex is not None:
        admin_sex_row = {
            "value": rec.administrative_sex.get("value"),
            "provenance_rank": rec.administrative_sex.get("provenance_rank", 0),
        }
    name_rows = [
        {"value": n["value"], "provenance_rank": n.get("provenance_rank", 0)} for n in rec.names
    ]
    identifier_rows = [
        {"system": i["system"], "match_key": i["match_key"]} for i in rec.identifiers
    ]
    return candidate_from_rows(
        dob_row=dob_row, sex_row=sex_row, name_rows=name_rows, identifier_rows=identifier_rows,
        admin_sex_row=admin_sex_row,
    )


def canonical_label_pair(a: str, b: str) -> tuple[str, str]:
    """Order two record_id labels (low, high) so a pair has one identity regardless of
    argument order. Lexical order on labels — the blocking layer maps to uuid order and
    reverse-maps back, so the two spaces never need to agree on ordering."""
    return (a, b) if a < b else (b, a)


def truth_pairs(ds: LabelledDataset) -> frozenset[tuple[str, str]]:
    """Every true-match pair: all within-cluster unordered record pairs, canonicalised.

    Cross-cluster pairs are, by construction, the non-matches; we never enumerate them
    here (the universe is all_pairs; non-matches = all_pairs - truth_pairs).
    """
    out: set[tuple[str, str]] = set()
    for ent in ds.entities:
        ids = [r.record_id for r in ent.records]
        for a, b in itertools.combinations(ids, 2):
            out.add(canonical_label_pair(a, b))
    return frozenset(out)


def all_pairs(ds: LabelledDataset) -> list[tuple[str, str]]:
    """The full comparison universe: every unordered record pair, canonicalised."""
    ids = [r.record_id for r in ds.all_records()]
    return [canonical_label_pair(a, b) for a, b in itertools.combinations(ids, 2)]
