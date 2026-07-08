"""Drift canary: pin the generator's recoverability predicate to the real blocking SQL.

`generator.shares_blocking_key` is a hand-maintained mirror of the base blocking passes in
`pipeline/db.py`'s `_GROUPS_SQL` / `_RANGE_GROUPS_SQL` — the two are coupled only by a comment.
The coupling is *asymmetric*: if a future edit WIDENS the SQL (adds a pass) the predicate merely
over-repairs (still safe); but if an edit NARROWS or renames a base pass the predicate keeps
claiming those pairs are recoverable, so `_repair` skips them and the DB silently drops true
matches — a break that only the DB-gated volume test would catch, and only when a database is
configured.

This test gives the FAST (no-DB) suite that missing signal: it asserts every base pass the
predicate leans on is still present in the SQL text. It needs psycopg only to import the SQL
constant (no connection), so it degrades cleanly to a skip where the extra is absent.
"""

import pytest

# The SQL lives in the psycopg-touching module; import the constant only, no connection.
pytest.importorskip(
    "psycopg", reason="pipeline extra (psycopg) absent — cannot read the blocking SQL"
)

from cairn_matcher.pipeline.db import _GROUPS_SQL, _RANGE_GROUPS_SQL  # noqa: E402


def _ws(text: str) -> str:
    """Collapse ALL whitespace so the canary pins SQL *content*, not formatting.

    A purely cosmetic reindent/re-wrap of db.py must not trip a tripwire whose failure
    message asserts a semantic narrowing — false alarms teach maintainers to update
    fragments reflexively, which is exactly how a real narrowing later slips through.
    """
    return " ".join(text.split())


# Each entry: the recoverability assumption in shares_blocking_key -> the SQL fragment
# that must survive for it to hold, in the statement that owns it. Narrowing/renaming
# any of these breaks the "recoverable by construction" guarantee, so tripping this
# test points straight at the mismatch.
_MIRRORED_PASSES = [
    ("exact-DOB pass (shares_blocking_key dob branch)",
     _GROUPS_SQL, "FROM patient_demographic WHERE field = 'dob'"),
    ("identifier pass excluding 'unknown' (_identifier_keys)",
     _GROUPS_SQL, "FROM patient_identifier WHERE system <> 'unknown'"),
    ("name-token pass: NFC + lower + whitespace split (name_tokens)",
     _GROUPS_SQL, "regexp_split_to_table(lower(normalize(value, NFC)), '\\s+')"),
    # §5.4: the placeholder-use exclusion the name_tokens mirror
    # (generator._is_placeholder_name) depends on.
    ("placeholder-use exclusion in name_tokens (§5.4)",
     _GROUPS_SQL, "use_key <> ALL(%s)"),
    # §5.4 slice 25: the dob+first-initial arm. shares_blocking_key's first-initial
    # clause (shared first initial + shared POINT year) claims these pairs recoverable,
    # so the SQL arm that extracts the initial must survive. `substring(nt.token FROM 1
    # FOR 1)` appears only in this arm's SELECT+GROUP BY, so it pins it specifically.
    # (name+sex needs NO pin: the mirror has no clause for it — it is subsumed by the
    # already-pinned name-token fragment.)
    ("dob+first-initial arm: birth-year + first initial (shares_blocking_key first-initial clause)",
     _GROUPS_SQL, "substring(nt.token FROM 1 FOR 1)"),
    # The exact-'dob' arm must keep EXCLUDING year-range rows: shares_blocking_key's
    # exact branch mirrors this exclusion, and without it two identical range strings
    # would be grouped by the SQL but not by the mirror (under-claim, safe) — while
    # DROPPING the guard from the mirror side would over-claim. Pin the SQL side.
    # Two clauses to pin the exact-dob arm specifically: the birth_year CTE carries the
    # same IS DISTINCT FROM guard, but with `AND value ~ '[0-9]{4}'` between these
    # clauses, so this fragment can only match the exact-dob arm.
    ("exact-dob arm excludes year-range rows (shares_blocking_key exact branch)",
     _GROUPS_SQL,
     "FROM patient_demographic WHERE field = 'dob' "
     "AND (facets ->> 'precision') IS DISTINCT FROM 'year-range'"),
    # The anchored range mirror (_birth_window + the overlap branch). Any of these
    # fragments disappearing means the range passes changed shape under the mirror.
    ("range rows keyed on precision 'year-range' (_birth_window range branch)",
     _RANGE_GROUPS_SQL, "facets ->> 'precision' = 'year-range'"),
    ("range min extracted as ^([0-9]{4})/ (_YEAR_RANGE_RE)",
     _RANGE_GROUPS_SQL, "substring(value FROM '^([0-9]{4})/')"),
    ("window overlap join (shares_blocking_key range branch)",
     _RANGE_GROUPS_SQL, "AND m.y_min <= a.y_max"),
    ("window overlap join, other bound (shares_blocking_key range branch)",
     _RANGE_GROUPS_SQL, "AND a.y_min <= m.y_max"),
    ("anchored on range charts only (point-point never keys)",
     _RANGE_GROUPS_SQL, "WHERE a.is_range"),
    ("blocking_sex unions both sex facets (dob-range+sex subset claim)",
     _RANGE_GROUPS_SQL, "field IN ('sex-at-birth', 'administrative-sex')"),
]


@pytest.mark.parametrize("assumption, sql_text, fragment", _MIRRORED_PASSES)
def test_shares_blocking_key_mirrors_the_blocking_sql(assumption, sql_text, fragment):
    assert _ws(fragment) in _ws(sql_text), (
        f"the blocking SQL no longer contains the pass fragment that "
        f"shares_blocking_key mirrors: {assumption}. Update "
        f"generator.shares_blocking_key/_birth_window to match — otherwise the "
        f"synthetic generator's recoverability guarantee is silently false."
    )
