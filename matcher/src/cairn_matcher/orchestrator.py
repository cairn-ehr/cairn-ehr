"""Drive the configured comparator over each field of two records.

This is the registry seam ADR-0014's locale packs will extend: DEFAULT_CONFIG wires the
culture-neutral comparators to fields; a locale pack swaps in phonetic/nickname variants
without touching the combiner. Pure — no I/O.
"""

from collections.abc import Callable
from dataclasses import dataclass, field as dataclass_field
from typing import Any

from cairn_matcher.agreement import Comparator, Context
from cairn_matcher.comparators import (
    compare_dob,
    compare_identifier_sets,
    compare_name_set,
    compare_sex,
)
from cairn_matcher.records import CandidateRecord, FieldComparison, SexValue


def _field_value(rec: CandidateRecord, attr: str) -> tuple[Any, int]:
    """Pull (value, provenance_rank) for a single-valued field; (None, 0) if absent."""
    fv = getattr(rec, attr)
    return (None, 0) if fv is None else (fv.value, fv.provenance_rank)


def _names(rec: CandidateRecord) -> tuple[Any, int]:
    fv = rec.names
    return (None, 0) if fv is None else (fv.value, fv.provenance_rank)


def _identifiers(rec: CandidateRecord) -> tuple[Any, int]:
    # Identifier match is positive-only and not provenance-tracked in B1 -> rank 0.
    return (rec.identifiers, 0)


def _sex_composite(rec: CandidateRecord) -> tuple[Any, int]:
    """Build the SexValue composite + this side's provenance rank.

    Rank rule: sex-at-birth's rank when that facet is present, else administrative-sex's.
    In the edge case where the union fallback intersects on the OTHER facet than the one
    whose rank we report, the rank is a second-order approximation — bounded by the
    [0.5, 1.0] provenance factor on a weight of 1.0 (design 2026-07-05 §2); revisit only
    if B3 provenance-sensitive tuning makes it observable. The orchestrator's existing
    min(rank_a, rank_b) then reduces to the weaker side, as for every field.
    """
    sab, admin = rec.sex_at_birth, rec.administrative_sex
    if sab is None and admin is None:
        return (None, 0)
    value = SexValue(
        sex_at_birth=None if sab is None else sab.value,
        administrative=None if admin is None else admin.value,
    )
    rank = sab.provenance_rank if sab is not None else admin.provenance_rank
    return (value, rank)


@dataclass(frozen=True)
class FieldSpec:
    """One field's comparison recipe: which comparator, and how to extract its inputs."""

    field: str
    comparator: Comparator
    get: Callable[[CandidateRecord], tuple[Any, int]]
    context: Context = dataclass_field(default_factory=Context)


ComparatorConfig = tuple[FieldSpec, ...]


# The shipped culture-neutral configuration. A locale pack (B3) ships its own.
DEFAULT_CONFIG: ComparatorConfig = (
    FieldSpec("dob", compare_dob, lambda r: _field_value(r, "dob")),
    FieldSpec("sex", compare_sex, _sex_composite),
    FieldSpec("name", compare_name_set, _names),
    FieldSpec("identifier", compare_identifier_sets, _identifiers),
)


def field_comparisons(
    a: CandidateRecord, b: CandidateRecord, config: ComparatorConfig = DEFAULT_CONFIG
) -> list[FieldComparison]:
    """Run each field's comparator and record its graded outcome.

    The provenance recorded is min(rank_a, rank_b): evidence about a field is only as
    trustworthy as its WEAKER-provenance side (a verified value compared against an
    unverified one is, jointly, unverified-grade).
    """
    out: list[FieldComparison] = []
    for spec in config:
        value_a, rank_a = spec.get(a)
        value_b, rank_b = spec.get(b)
        level = spec.comparator(value_a, value_b, spec.context)
        out.append(FieldComparison(spec.field, level, min(rank_a, rank_b)))
    return out
