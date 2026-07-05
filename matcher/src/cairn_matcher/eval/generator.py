"""Synthetic blocking-eval dataset generator (pure, stdlib-only).

Emits the eval dataset dict shape (see dataset.py) at volume: clean seed identities
plus one corrupted near-duplicate ("clone") per person. Ground truth is the entity
grouping, so no pair-labelling is needed. Deterministic given a seed.

This module is PURE: stdlib random/dataclasses/unicodedata only, no I/O, no psycopg.
The disk/CLI edge lives in generate.py (the dataset.py <-> loader.py split).
"""

import copy
import random
import re
import unicodedata
from collections.abc import Mapping
from dataclasses import dataclass

from cairn_matcher.placeholder_uses import PLACEHOLDER_NAME_USES


def _is_placeholder_name(n: Mapping) -> bool:
    """True iff this name record carries a placeholder `use` (a callsign), mirroring the SQL
    `use_key <> ALL(PLACEHOLDER_NAME_USES)` exclusion. Accepts either `use_key` (the db/012
    projection column) or `use` (the raw facet); a name with neither is a real name and never
    a placeholder. Compared lower-cased to match the folded `use_key`."""
    raw = n.get("use_key") or n.get("use")
    return raw is not None and str(raw).lower() in PLACEHOLDER_NAME_USES


def name_tokens(record: Mapping) -> set[str]:
    """NFC-normalised, lower-cased whitespace tokens across a record's NON-placeholder names.

    Mirrors the SQL 'name' blocking pass (lower(normalize(value, NFC)) split on
    whitespace) so this predicate agrees with what generate_candidate_pairs actually
    blocks on — including the NFC fold that lets NFD/NFC variants of a name co-block, and
    the §5.4 exclusion of placeholder-use (callsign) names (`use_key <> ALL(...)`). The
    placeholder set is imported from the shared `cairn_matcher.placeholder_uses` (the single
    source of truth), so this mirror can never drift from what `pipeline/db.py` excludes.

    Today's generator emits no `use`/`use_key` field, so this is a no-op for the current
    synthetic data — but it keeps the "recoverable by blocking" guarantee honest the moment a
    dataset carries a callsign, instead of silently over-claiming recovery (issue #124).
    """
    tokens: set[str] = set()
    for n in record.get("names", ()):
        if _is_placeholder_name(n):
            continue
        tokens.update(unicodedata.normalize("NFC", str(n["value"])).lower().split())
    return tokens


def _identifier_keys(record: Mapping) -> set[tuple[str, str]]:
    """(system, match_key) pairs excluding the 'unknown' sentinel — the identifier pass."""
    return {
        (i["system"], i["match_key"])
        for i in record.get("identifiers", ())
        if i["system"] != "unknown"
    }


# Mirrors of _RANGE_GROUPS_SQL's birth_window guards (pipeline/db.py): a range value
# must be exactly '<yyyy>/<yyyy>' with min <= max; a point value contributes its FIRST
# 4-digit run. Kept as module constants so the two branches below can't drift apart.
_YEAR_RANGE_RE = re.compile(r"^([0-9]{4})/([0-9]{4})$")
_FIRST_YEAR_RE = re.compile(r"[0-9]{4}")


def _birth_window(record: Mapping):
    """(y_min, y_max, is_range) for one record's dob, or None — the birth_window CTE.

    Mirrors _RANGE_GROUPS_SQL exactly, in the safe direction: a malformed or inverted
    year-range value yields NO window (the SQL excludes the row), never a guess; a
    point value needs a 4-digit run (the SQL's `value ~ '[0-9]{4}'`) and contributes
    its first run as a degenerate [y, y] window. A year-range row can never enter the
    point branch (the SQL's IS DISTINCT FROM guard), so a range can't double-enter as
    a false point [min, min].
    """
    dob = record.get("dob")
    if not dob or not isinstance(dob.get("value"), str):
        return None
    value = dob["value"]
    if dob.get("precision") == "year-range":
        m = _YEAR_RANGE_RE.match(value)
        if not m:
            return None
        lo, hi = int(m.group(1)), int(m.group(2))
        if lo > hi:
            return None
        return (lo, hi, True)
    m = _FIRST_YEAR_RE.search(value)
    if m is None:
        return None
    year = int(m.group(0))
    return (year, year, False)


def shares_blocking_key(a: Mapping, b: Mapping) -> bool:
    """True iff records a and b would co-occur in >=1 blocking pass.

    The symmetric keys (pipeline/db.py _GROUPS_SQL): shared non-unknown identifier,
    equal exact-DOB value (excluding year-range rows, mirroring the SQL's
    IS DISTINCT FROM 'year-range' — two identical range strings must NOT fake an
    exact key the SQL never groups), or a shared name token. The 'name+year' pass is
    subsumed by the name-token check (it requires a shared token).

    The ANCHORED range passes (_RANGE_GROUPS_SQL): windows overlap AND at least one
    side is a real year-range (window_overlap requires a.is_range — two point DOBs
    merely sharing a year are never a range key). 'dob-range+sex' is a subset of
    'dob-range''s pair set (same overlap join, intersected with a shared sex), so
    recoverability needs only the plain overlap branch; like every branch here, the
    block-size cap is deliberately not modelled (evaluate_blocking reports skips).
    """
    if _identifier_keys(a) & _identifier_keys(b):
        return True
    da, db_ = a.get("dob"), b.get("dob")
    if (
        da
        and db_
        and da.get("precision") != "year-range"
        and db_.get("precision") != "year-range"
        and da.get("value") is not None
        and da.get("value") == db_.get("value")
    ):
        return True
    wa, wb = _birth_window(a), _birth_window(b)
    if wa and wb and (wa[2] or wb[2]) and wb[0] <= wa[1] and wa[0] <= wb[1]:
        return True
    return bool(name_tokens(a) & name_tokens(b))


def _clone(record):
    """A deep copy so an operator can never mutate its input (pure discipline)."""
    return copy.deepcopy(dict(record))


def corrupt_dob_format(record, rng):
    """Re-express the same birth-year in a different exact form: day-first restring
    ("1990-05-12" -> "12/05/1990") or precision downgrade to year-only ("1990").

    Exact-DOB blocking then MISSES the pair while name+year still CATCHES it. No-op if
    the record has no ISO 'YYYY-MM-DD' dob value (safe degrade).
    """
    out = _clone(record)
    dob = out.get("dob")
    if not dob or not isinstance(dob.get("value"), str):
        return out
    parts = dob["value"].split("-")
    if len(parts) != 3:
        return out  # not full ISO -> leave it
    y, m, d = parts
    if rng.random() < 0.5:
        dob["value"] = f"{d}/{m}/{y}"          # day-first re-import; year still present
    else:
        dob["value"] = y                        # precision downgrade
        dob["precision"] = "year"
    return out


def _perturb_digit(text, rng):
    """Transpose two adjacent digits, or bump one digit by 1 (mod 10). Pure given rng."""
    positions = [i for i, c in enumerate(text) if c.isdigit()]
    if not positions:
        return text
    chars = list(text)
    adj = [i for i in positions if i + 1 in positions]
    if adj and rng.random() < 0.5:
        i = rng.choice(adj)
        chars[i], chars[i + 1] = chars[i + 1], chars[i]
    else:
        i = rng.choice(positions)
        chars[i] = str((int(chars[i]) + 1) % 10)
    return "".join(chars)


def corrupt_dob_typo(record, rng):
    """Fat-finger the DOB: transpose or bump a digit. May change the birth-year (then the
    pair honestly degrades off name+year; another key must carry it). No-op if no dob."""
    out = _clone(record)
    dob = out.get("dob")
    if not dob or not isinstance(dob.get("value"), str):
        return out
    dob["value"] = _perturb_digit(dob["value"], rng)
    return out


def _strip_diacritics(text):
    """NFD-decompose and drop combining marks: 'Jón' -> 'Jon'. Culture-neutral."""
    return "".join(c for c in unicodedata.normalize("NFD", text)
                   if not unicodedata.combining(c))


def corrupt_name(record, rng):
    """Corrupt ONE of the record's names: strip diacritics, transpose two letters, or drop
    a token (when the name has >1 token). Breaks the exact shared-name-token block for the
    affected token. No-op if the record has no names."""
    out = _clone(record)
    names = out.get("names", [])
    if not names:
        return out
    idx = rng.randrange(len(names))
    value = str(names[idx]["value"])
    mode = rng.choice(("diacritic", "transpose", "drop"))
    if mode == "diacritic":
        value = _strip_diacritics(value)
    elif mode == "transpose" and len(value) >= 2:
        i = rng.randrange(len(value) - 1)
        chars = list(value)
        chars[i], chars[i + 1] = chars[i + 1], chars[i]
        value = "".join(chars)
    else:  # drop a token when the name has >1 token; a single-token (mononym) name is left unchanged this round
        tokens = value.split()
        if len(tokens) > 1:
            del tokens[rng.randrange(len(tokens))]
            value = " ".join(tokens)
    names[idx] = {**names[idx], "value": value}
    return out


def corrupt_identifier(record, rng):
    """Drop the shared identifier, or mistype its match_key/value. Identifier blocking then
    misses; the pair must fall through to DOB/name. No-op if the record has no identifiers."""
    out = _clone(record)
    ids = out.get("identifiers", [])
    if not ids:
        return out
    idx = rng.randrange(len(ids))
    if rng.random() < 0.5:
        del ids[idx]                            # drop it entirely
    else:
        mistyped = _perturb_digit(str(ids[idx]["match_key"]), rng)
        ids[idx] = {**ids[idx], "match_key": mistyped, "value": mistyped}
    return out


def corrupt_dob_estimate(record, rng):
    """Rewrite the clone as a §5.4 estimated-age registration of the same person.

    The dob becomes an inclusive birth-year window CONTAINING the current value's
    first 4-digit run (an honest interval, never a false-precise midpoint —
    principle 4; tol 2..5 gives the 5–11-year widths slice B's evidence produces),
    at provenance 30 (clinician-observed). Sex moves to the OBSERVED facet: a
    clinician observes apparent sex but cannot know the birth fact (slice B), so
    sex_at_birth is dropped and administrative_sex carries the seed's value when
    present (a correct observation) or a random draw when the seed recorded none.
    Runs LAST in _OPERATORS: an estimated-age record supersedes format/typo dob
    corruption wholesale; a typo'd year windows around the typo (honest corruption —
    the window may or may not still overlap the seed's). No-op without a 4-digit
    run (safe degrade, like every operator).
    """
    out = _clone(record)
    dob = out.get("dob")
    if not dob or not isinstance(dob.get("value"), str):
        return out
    m = _FIRST_YEAR_RE.search(dob["value"])
    if m is None:
        return out
    year = int(m.group(0))
    tol = rng.randint(2, 5)
    out["dob"] = {
        "value": f"{year - tol:04d}/{year + tol:04d}",
        "precision": "year-range",
        "provenance_rank": 30,
    }
    sab = out.pop("sex_at_birth", None)
    observed = sab["value"] if sab else rng.choice(("male", "female"))
    out["administrative_sex"] = {"value": observed, "provenance_rank": 30}
    return out


# Curated, culture-plural pools. Deliberately small and hand-written (no faker: a dep
# and Western bias would both violate the mission). Blocking keys on tokens/years, not
# name rarity, so a small pool is sufficient and makes tokens recur (realistic collisions).
_MONONYMS = ("Suharto", "Sukarno", "Madonna", "Ronaldinho", "Teresa")
_GIVEN = ("Alex", "Sam", "Mira", "Jon", "Ana", "Wei", "Omar", "Fatima", "Ivan", "Lena")
_FAMILY = ("Nguyen", "Einarsson", "Garcia", "Okafor", "Kowalski", "Haddad", "Silva", "Ali")
_PATRONYMIC = (("Jón", "Einarsson"), ("Ólafur", "Bjarnason"), ("Freyr", "Þórsson"))
_ID_SYSTEMS = ("au-medicare", "national-id", "kennitala", "mrn-local")


def _synth_name(rng):
    """Draw one display name across three culture shapes: mononym, patronymic+diacritic,
    or multi-token given+family. Returns the display string."""
    shape = rng.choice(("mono", "patronymic", "given_family"))
    if shape == "mono":
        return rng.choice(_MONONYMS)
    if shape == "patronymic":
        g, p = rng.choice(_PATRONYMIC)
        return f"{g} {p}"
    return f"{rng.choice(_GIVEN)} {rng.choice(_FAMILY)}"


def _synth_dob(rng):
    """A plausible ISO 'YYYY-MM-DD' at day precision."""
    year = rng.randint(1935, 2015)
    month = rng.randint(1, 12)
    day = rng.randint(1, 28)   # 28 avoids month-length edge cases (not needed for blocking)
    return {"value": f"{year:04d}-{month:02d}-{day:02d}", "precision": "day",
            "provenance_rank": rng.choice((20, 30, 40))}


def synth_seed(rng, index):
    """Build one clean seed record for entity `index`. Always has a name and an ISO dob;
    ~70% carry an identifier, ~50% a sex_at_birth (both inert for blocking but realistic)."""
    rec = {
        "record_id": f"e{index}-seed",
        "dob": _synth_dob(rng),
        "names": [{"value": _synth_name(rng), "provenance_rank": rng.choice((20, 30))}],
    }
    if rng.random() < 0.7:
        key = f"{rng.randint(10000, 99999)}"
        rec["identifiers"] = [{"system": rng.choice(_ID_SYSTEMS),
                               "match_key": key, "value": key}]
    if rng.random() < 0.5:
        rec["sex_at_birth"] = {"value": rng.choice(("male", "female")),
                               "provenance_rank": 40}
    return rec


@dataclass(frozen=True)
class GenSpec:
    """Knobs for one synthetic dataset. Deterministic: (seed, fields) reproduce byte-for-byte.

    Cluster size is fixed at 2 (seed + one clone) this slice, so each entity yields exactly
    one seed<->clone true pair and the recoverability invariant is exactly the all-pairs one.

    Adding an operator changes RNG consumption, so a given seed's output differs
    across versions of this module: "deterministic given a seed" is a
    reproducibility contract within one version, not a cross-version stability one.
    """
    seed: int = 0
    n_entities: int = 100
    p_dob_format: float = 0.45
    p_dob_typo: float = 0.2
    p_name: float = 0.5
    p_identifier: float = 0.5
    p_dob_estimate: float = 0.15


_OPERATORS = (
    ("p_dob_format", corrupt_dob_format),
    ("p_dob_typo", corrupt_dob_typo),
    ("p_name", corrupt_name),
    ("p_identifier", corrupt_identifier),
    ("p_dob_estimate", corrupt_dob_estimate),
)


def _repair(seed, clone):
    """Guarantee the seed<->clone pair stays blockable: if corruptions destroyed every base
    key, append the seed's primary name (verbatim) to the clone's retained names, restoring a
    shared name token. Every seed has >=1 name, so this always succeeds. Pure (returns new)."""
    if shares_blocking_key(seed, clone):
        return clone
    out = _clone(clone)
    out.setdefault("names", [])
    out["names"].append(dict(seed["names"][0]))
    return out


def _make_clone(seed, spec, rng, index):
    """One corrupted near-duplicate of `seed`: apply each enabled operator with its
    probability, then repair to satisfy the recoverability invariant."""
    clone = _clone(seed)
    clone["record_id"] = f"e{index}-dup"
    for prob_field, op in _OPERATORS:
        if rng.random() < getattr(spec, prob_field):
            clone = op(clone, rng)
    return _repair(seed, clone)


def generate_dataset(spec):
    """Build the full dataset dict: n_entities clusters, each a seed + one corrupted clone.

    Returns a JSON-shaped mapping that round-trips through eval.dataset.load_dataset. Ground
    truth is the entity grouping; truth_pairs derives the one true pair per cluster for free.
    """
    rng = random.Random(spec.seed)
    entities = []
    for i in range(spec.n_entities):
        seed = synth_seed(rng, i)
        clone = _make_clone(seed, spec, rng, i)
        entities.append({"entity_id": f"e{i}", "records": [seed, clone]})
    return {
        "name": f"synthetic_s{spec.seed}_n{spec.n_entities}",
        "description": (
            "Synthetic blocking-eval set: seed + one corrupted clone per entity. "
            "Every true pair is recoverable by >=1 base blocking key (by construction); "
            "a regression/tuning instrument, not a statistical accuracy claim."
        ),
        "entities": entities,
    }
