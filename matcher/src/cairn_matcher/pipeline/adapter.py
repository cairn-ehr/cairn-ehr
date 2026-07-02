"""Pure mappers from a node's patient_* projection rows into B1 CandidateRecords.

No I/O, no psycopg. Callers (pipeline.db) hand these functions plain dict rows; these
functions shape them into the value types B1 scores over. Every field degrades safely
on absence or malformed input (principle 4: absence is never disagreement); a
structurally wrong row raises MatcherTypeError elsewhere in this module (house rule #5).
"""

import unicodedata
from collections.abc import Mapping, Sequence

from cairn_matcher.records import CandidateRecord, DateValue, FieldValue, MatcherTypeError, Name

# The ISO field counts we can extract per declared precision. precision -> how many of
# (year, month, day) the value must supply. We never parse a locale date string; we only
# read the dash-separated ISO fields the cairn-event writer already emits.
_PRECISION_PARTS = {"year": 1, "month": 2, "day": 3}

# Uncertainty sentinels that are legitimate recorded VALUES (principle 4: `unknown` is
# first-class), but must NOT be compared as ordinary strings: "unknown" == "unknown"
# would fabricate EXACT agreement, and "unknown" vs "male" a DISAGREE penalty — both
# from acknowledged ignorance. Treated as field ABSENCE for matching.
_VALUE_SENTINELS = {"unknown"}


def _normalize_token(token: str) -> str:
    """Culture-neutral canonicalisation for a name token: NFC then casefold.

    Different upstream systems disagree on Unicode normalisation form (NFC precomposed
    vs NFD decomposed) for the SAME name — "Jón" can arrive either way. Without this,
    the two forms are different code points end to end: they grade DISAGREE and produce
    different blocking tokens, so a true duplicate is never even generated. NFC + casefold
    is culture-NEUTRAL (not the anglo transliteration ADR-0014 forbids); the SQL blocking
    tokenizer applies the matching `lower(normalize(value,'NFC'))`.
    """
    return unicodedata.normalize("NFC", token).casefold()


def parse_dob(value: str | None, precision: str | None) -> DateValue | None:
    """Extract a DateValue from an ISO dob value at the projection's declared precision.

    Returns None (a safe, gradeable absence) when the value is missing, the precision is
    missing or unknown, or the value is not ISO-shaped to at least the declared precision.
    We never coerce a locale string or guess month/day order — that is a B3/locale-pack
    concern; here, an unreadable value simply has no DOB to compare.
    """
    if not value or precision not in _PRECISION_PARTS:
        return None
    parts = value.split("-")
    needed = _PRECISION_PARTS[precision]
    if len(parts) < needed:
        return None  # value is coarser than the precision it claims
    # The year field must be a full 4-digit run, mirroring the SQL blocking discipline
    # (db.py requires `[0-9]{4}`). A 2-digit legacy year ("80") would otherwise parse to
    # DateValue(year=80) and fabricate a DISAGREE clash against the same person's 1980
    # record — a wrong DateValue from a formatting artefact, which principle 4 forbids.
    if len(parts[0]) != 4 or not parts[0].isdigit():
        return None
    try:
        nums = [int(p) for p in parts[:needed]]
    except ValueError:
        return None  # non-numeric field -> not ISO -> safe degrade
    year = nums[0]
    month = nums[1] if needed >= 2 else None
    day = nums[2] if needed >= 3 else None
    # Numeric but out-of-range fields (e.g. month 13, day 45) are not a real date; degrade
    # rather than emit a wrong DateValue. We range-check only — calendar validity per month
    # (e.g. 30 Feb) is a B3/locale-pack concern, not this precision-gated field extractor's.
    if month is not None and not 1 <= month <= 12:
        return None
    if day is not None and not 1 <= day <= 31:
        return None
    return DateValue(year=year, month=month, day=day)


def _name_bag(display: object) -> Name:
    """Turn one opaque display string into an untagged token-bag Name.

    patient_name projects only the authored display string — no given/family roles — so
    we put all whitespace-split, lower-cased tokens under a single 'unspecified' role.
    compare_name_set compares bags per role, so a shared single role reduces to a
    whole-string token-bag comparison (culture-neutral; no schema change). A non-string
    value is a structural bug, not mere absence -> raise (house rule #5).
    """
    if not isinstance(display, str):
        raise MatcherTypeError(f"name value must be str, got {type(display).__name__}")
    tokens = tuple(sorted(_normalize_token(t) for t in display.split()))
    return Name(tokens={"unspecified": tokens})


def build_names(rows: Sequence[Mapping]) -> FieldValue | None:
    """Collect every asserted name into a frozenset[Name]; provenance = max over rows.

    The name FIELD's provenance is the strongest evidence behind any of the patient's
    retained names; the orchestrator separately reduces cross-record comparisons to the
    weaker side. Empty set -> None (absence -> INSUFFICIENT_DATA downstream).
    """
    if not rows:
        return None
    names = frozenset(_name_bag(r["value"]) for r in rows)
    rank = max(int(r["provenance_rank"]) for r in rows)
    return FieldValue(value=names, provenance_rank=rank)


def build_identifiers(rows: Sequence[Mapping]) -> dict[str, frozenset[str]]:
    """Group identifier match_keys by system, skipping the 'unknown' sentinel.

    match_key == coalesce(normalized, value) — the same key the db/016 veto floor uses,
    so the advisory positive-evidence comparison and the hard veto align on identity.
    """
    out: dict[str, set[str]] = {}
    for r in rows:
        system = r["system"]
        if system == "unknown":
            continue
        out.setdefault(system, set()).add(r["match_key"])
    return {system: frozenset(keys) for system, keys in out.items()}


def single_field(row: Mapping | None) -> FieldValue | None:
    """Map one patient_demographic winner row to a FieldValue, or None when absent.

    A recorded uncertainty sentinel (`unknown`) is a legitimate value but ZERO matching
    evidence (principle 4): it maps to None so it neither fabricates EXACT agreement with
    another `unknown` nor a DISAGREE penalty against a real value.
    """
    if row is None:
        return None
    value = row["value"]
    if isinstance(value, str) and value.strip().casefold() in _VALUE_SENTINELS:
        return None
    return FieldValue(value=value, provenance_rank=int(row["provenance_rank"]))


def candidate_from_rows(
    *,
    dob_row: Mapping | None,
    sex_row: Mapping | None,
    name_rows: Sequence[Mapping],
    identifier_rows: Sequence[Mapping],
) -> CandidateRecord:
    """Assemble a CandidateRecord from one patient's projection rows.

    dob is special: its value is parsed via parse_dob at the row's declared precision; an
    unparseable value drops the whole dob field to None (safe degrade), never a guess.
    """
    dob = None
    if dob_row is not None:
        precision = (dob_row.get("facets") or {}).get("precision")
        parsed = parse_dob(dob_row["value"], precision)
        if parsed is not None:
            dob = FieldValue(value=parsed, provenance_rank=int(dob_row["provenance_rank"]))
    return CandidateRecord(
        dob=dob,
        sex_at_birth=single_field(sex_row),
        names=build_names(name_rows),
        identifiers=build_identifiers(identifier_rows),
    )
