"""The comparator contract and the graded-agreement vocabulary (ADR-0014 §Decision 2).

A comparator is a PURE, field-typed function returning a graded AgreementLevel, never a
boolean — because Fellegi–Sunter weighs each level of agreement differently. The levels
are ordinal (higher = stronger agreement) so that name-set matching can pick the best
agreement across a cross-product with a plain max().

PHONETIC and NICKNAME exist in the vocabulary as the reserved plug points for locale
packs (a later slice). NO comparator in this core emits them — shipping a phonetic
encoder (Soundex is anglo) or a nickname lexicon (cultural) in the core would be the
"cultural capture" ADR-0014 forbids.
"""

from collections.abc import Callable
from dataclasses import dataclass
from enum import IntEnum
from typing import Any


class AgreementLevel(IntEnum):
    """Graded agreement between two field values. Ordinal: higher == stronger.

    Persisted BY NAME only (proposal evidence, the config fingerprint, eval/model_io) — never
    by number — so a level may be inserted in order. tests/test_agreement.py checks the
    persisted forms behaviourally (evidence and fingerprint carry names; a model round-trips
    "NEAR").
    """

    INSUFFICIENT_DATA = 0  # a side is absent/unknown -> ZERO evidence (not a penalty, §3.7)
    DISAGREE = 1           # both present, no agreement at any level
    NEAR = 2               # both present and differing by ONE keying error (DOB only) —
                           # consistent with one person and a typo AND with two people (§3.7)
    PARTIAL = 3            # precision-coarsened / weak (e.g. year-only DOB vs full)
    EDIT_DISTANCE = 4      # agree within an edit-distance band
    PHONETIC = 5           # reserved for locale packs — not emitted by this core
    NICKNAME = 6           # reserved for locale packs — not emitted by this core
    EXACT = 7              # exact agreement


@dataclass(frozen=True)
class Context:
    """Per-comparison facets a comparator may need. Never carries I/O handles.

    edit_distance_threshold is the Jaro–Winkler similarity at or above which
    compare_edit_distance grades EDIT_DISTANCE rather than DISAGREE.
    """

    edit_distance_threshold: float = 0.90


# A comparator: pure, field-typed, returns a graded agreement level.
Comparator = Callable[[Any, Any, Context], AgreementLevel]
