"""Synthetic blocking-eval dataset generator (pure, stdlib-only).

Emits the eval dataset dict shape (see dataset.py) at volume: clean seed identities
plus one corrupted near-duplicate ("clone") per person. Ground truth is the entity
grouping, so no pair-labelling is needed. Deterministic given a seed.

This module is PURE: stdlib random/dataclasses/unicodedata only, no I/O, no psycopg.
The disk/CLI edge lives in generate.py (the dataset.py <-> loader.py split).
"""

import copy
import unicodedata
from collections.abc import Mapping


def name_tokens(record: Mapping) -> set[str]:
    """Lower-cased whitespace tokens across ALL of a record's names.

    Mirrors the SQL 'name' blocking pass (lower(value) split on whitespace) so this
    predicate agrees with what generate_candidate_pairs actually blocks on.
    """
    tokens: set[str] = set()
    for n in record.get("names", ()):
        tokens.update(str(n["value"]).lower().split())
    return tokens


def _identifier_keys(record: Mapping) -> set[tuple[str, str]]:
    """(system, match_key) pairs excluding the 'unknown' sentinel — the identifier pass."""
    return {
        (i["system"], i["match_key"])
        for i in record.get("identifiers", ())
        if i["system"] != "unknown"
    }


def shares_blocking_key(a: Mapping, b: Mapping) -> bool:
    """True iff records a and b would co-occur in >=1 base blocking pass.

    The three BASE keys (pipeline/db.py _GROUPS_SQL): shared non-unknown identifier,
    equal exact-DOB value, or a shared name token. The fourth pass 'name+year' is
    subsumed by the name-token check (it requires a shared token), so it is not tested
    separately: if name tokens intersect, the plain 'name' pass already groups them.
    """
    if _identifier_keys(a) & _identifier_keys(b):
        return True
    da, db_ = a.get("dob"), b.get("dob")
    if da and db_ and da.get("value") is not None and da.get("value") == db_.get("value"):
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
    else:  # drop a token when possible, else fall back to transpose handled above
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
