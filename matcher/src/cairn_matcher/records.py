"""Input/output value types the scoring core operates over.

These are plain frozen dataclasses. A later slice (B2) populates a CandidateRecord
from the patient_* projections; this core builds them by hand in tests. Keeping the
types here — separate from the comparison logic — means the comparators and the
combiner depend only on data shapes, not on where the data came from.
"""

from collections.abc import Mapping
from dataclasses import dataclass, field
from types import MappingProxyType
from typing import Any

from cairn_matcher.agreement import AgreementLevel


class MatcherTypeError(TypeError):
    """A value was structurally the wrong type (an adapter bug), not merely absent.

    Absence is normal and safe — it grades INSUFFICIENT_DATA. This error is for the
    different failure: a str where a DateValue is required, etc. We raise loudly rather
    than fail silently (house rule #5).
    """


@dataclass(frozen=True)
class DateValue:
    """A canonical, already-parsed date. Precision is implied by which parts are present.

    Two shapes, never mixed:
      * a POINT date — some prefix of (year, month, day) present, the rest None;
      * a birth-year RANGE — year_min..year_max present (inclusive), all point parts None.
        A range is how a clinician-observed *estimated age* is carried (§5.4): honest
        imprecision, never a false-precise midpoint (principle 4).

    The core never parses a locale date STRING into this — that is locale-specific and
    belongs to B2/locale packs. compare_dob operates only on the parts present here.
    """

    year: int | None = None
    month: int | None = None
    day: int | None = None
    year_min: int | None = None
    year_max: int | None = None

    @property
    def is_range(self) -> bool:
        """True iff this is a birth-year interval rather than a point date."""
        return self.year_min is not None and self.year_max is not None


@dataclass(frozen=True)
class SexValue:
    """The composite value the `sex` comparator sees: both of one chart's sex facets.

    sex_at_birth is the §4.2 birth fact (the db/016 veto's subject); administrative is
    the apparent/phenotypic field a §5.4 clinician-observed sex lands on (slice B chose
    it deliberately — a clinician cannot know the birth fact). Either may be None; a
    chart with neither field never constructs a SexValue at all (the orchestrator's
    extractor returns None instead, which grades INSUFFICIENT_DATA).
    """

    sex_at_birth: str | None = None
    administrative: str | None = None


@dataclass(frozen=True)
class Name:
    """One asserted name as role-tagged token bags, e.g. {"given": ("alex",), ...}.

    A patient carries a SET of these (the §4.2 retained name history). Comparison is
    order- and role-tolerant: tokens are compared as bags per role, not positionally.
    """

    tokens: Mapping[str, tuple[str, ...]]

    def __post_init__(self) -> None:
        """Convert plain dicts to immutable MappingProxyType so Name can be hashable.

        Dataclasses with frozen=True compute a hash only from hashable fields.
        A plain dict is unhashable; MappingProxyType wraps it immutably. We convert
        on construction so instances can go into frozensets (used in tests and by
        CandidateRecord).
        """
        if isinstance(self.tokens, dict):
            object.__setattr__(self, "tokens", MappingProxyType(self.tokens))

    def __hash__(self) -> int:
        """Hash a Name by its frozen tokens structure.

        Convert the mapping to a frozenset of (key, value) items for hashing.
        """
        return hash(frozenset(self.tokens.items()))

    def __eq__(self, other: Any) -> bool:
        """Compare Names by their tokens."""
        if not isinstance(other, Name):
            return NotImplemented
        return self.tokens == other.tokens


@dataclass(frozen=True)
class FieldValue:
    """A single demographic field's value plus the provenance rank behind it.

    provenance_rank is the cached patient_demographic.provenance_rank (the §4.1 ladder
    as an int; 0 = unrecognized). The combiner scales evidence by it.
    """

    value: Any
    provenance_rank: int = 0


@dataclass(frozen=True)
class CandidateRecord:
    """Everything one patient contributes to a comparison. Additive: more fields later."""

    dob: FieldValue | None = None
    sex_at_birth: FieldValue | None = None
    administrative_sex: FieldValue | None = None  # §5.4 apparent/phenotypic sex facet
    names: FieldValue | None = None  # value is a frozenset[Name] (the history set)
    identifiers: Mapping[str, frozenset[str]] = field(default_factory=dict)


@dataclass(frozen=True)
class FieldComparison:
    """The graded outcome for one field, with the (weaker-side) provenance behind it."""

    field: str
    level: AgreementLevel
    provenance_rank: int
