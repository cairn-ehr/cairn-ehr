"""Drive the configured comparator over each field of two records.

This is the registry seam ADR-0014's locale packs will extend: DEFAULT_CONFIG wires the
culture-neutral comparators to fields; a locale pack swaps in phonetic/nickname variants
without touching the combiner. Pure — no I/O.
"""

from collections.abc import Callable
from dataclasses import dataclass
from dataclasses import field as dataclass_field
from typing import Any

from cairn_matcher.agreement import Comparator, Context
from cairn_matcher.comparators import (
    compare_dob,
    compare_identifier_sets,
    compare_name_set,
    compare_sex,
)
from cairn_matcher.records import CandidateRecord, FieldComparison, SexValue


def callable_identity(fn: Callable) -> str:
    """The module-qualified name of a CONFIGURED callable — its version-pin identity.

    matcher_version (pipeline/banding.py) fingerprints a FieldSpec's comparator and
    extractor by this name: the wiring is configuration (the ADR-0014 locale-pack seam),
    while the function bodies are code, pinned by the package-version component. That only
    works for callables whose name IS stable — a lambda ("<lambda>", shared by every
    module-level lambda), a closure ("...<locals>...", shared by every closure from the
    same factory), or a functools.partial (no __qualname__ at all) would either crash the
    pin or let two DIFFERENT configs pin identically, silently merging the
    ADR-0011/0029 contamination-recall key. Refuse those here, loudly; a parameterised
    comparator belongs behind a named wrapper function (or, later, the ADR-0014
    `namespace@content-hash` comparator-profile tag) so its parameters are visible config.
    """
    module = getattr(fn, "__module__", None)
    qualname = getattr(fn, "__qualname__", None)
    if not module or not qualname or "<lambda>" in qualname or "<locals>" in qualname:
        raise ValueError(
            f"FieldSpec callables need a stable module-qualified name for the "
            f"matcher_version fingerprint; got {fn!r}. Use a module-level named "
            f"function instead of a lambda/closure/partial."
        )
    return f"{module}.{qualname}"


def _field_value(rec: CandidateRecord, attr: str) -> tuple[Any, int]:
    """Pull (value, provenance_rank) for a single-valued field; (None, 0) if absent."""
    fv = getattr(rec, attr)
    return (None, 0) if fv is None else (fv.value, fv.provenance_rank)


def _dob(rec: CandidateRecord) -> tuple[Any, int]:
    """Named (fingerprint-stable) extractor for the dob field — see callable_identity."""
    return _field_value(rec, "dob")


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
    """One field's comparison recipe: which comparator, and how to extract its inputs.

    Every field here is part of the matcher_version fingerprint (issue #100): the spec IS
    configuration, and configuration is the ADR-0011/0029 recall key. Adding a field?
    banding._config_fingerprint picks it up automatically (it iterates dataclasses.fields),
    and test_banding's completeness guard fails if it somehow doesn't.
    """

    field: str
    comparator: Comparator
    get: Callable[[CandidateRecord], tuple[Any, int]]
    context: Context = dataclass_field(default_factory=Context)

    def __post_init__(self) -> None:
        """Refuse callables the version fingerprint cannot stably name (issue #100).

        Validated at CONSTRUCTION — where the config author is looking — rather than
        failing per-pair inside a sweep's build_payload (where sweep()'s per-pair
        exception handling would reduce it to N errors and zero proposals). Same
        construct-time-invariant pattern as Thresholds' review<=auto check (#211).
        """
        callable_identity(self.comparator)
        callable_identity(self.get)


ComparatorConfig = tuple[FieldSpec, ...]


# The shipped culture-neutral configuration. A locale pack (B3) ships its own.
DEFAULT_CONFIG: ComparatorConfig = (
    FieldSpec("dob", compare_dob, _dob),
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
