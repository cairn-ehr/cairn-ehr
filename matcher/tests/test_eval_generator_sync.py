"""Drift canary: pin the generator's recoverability predicate to the real blocking SQL.

`generator.shares_blocking_key` is a hand-maintained mirror of the base blocking passes in
`pipeline/db.py`'s `_GROUPS_SQL` — the two are coupled only by a comment. The coupling is
*asymmetric*: if a future edit WIDENS the SQL (adds a pass) the predicate merely over-repairs
(still safe); but if an edit NARROWS or renames a base pass the predicate keeps claiming those
pairs are recoverable, so `_repair` skips them and the DB silently drops true matches — a break
that only the DB-gated volume test would catch, and only when a database is configured.

This test gives the FAST (no-DB) suite that missing signal: it asserts every base pass the
predicate leans on is still present in the SQL text. It needs psycopg only to import the SQL
constant (no connection), so it degrades cleanly to a skip where the extra is absent.
"""

import pytest

# The SQL lives in the psycopg-touching module; import the constant only, no connection.
pytest.importorskip("psycopg", reason="pipeline extra (psycopg) absent — cannot read the blocking SQL")

from cairn_matcher.pipeline.db import _GROUPS_SQL  # noqa: E402


# Each entry: the recoverability assumption in shares_blocking_key -> the SQL fragment that
# must survive for it to hold. Narrowing/renaming any of these breaks the "recoverable by
# construction" guarantee, so tripping this test points straight at the mismatch.
_MIRRORED_PASSES = [
    ("exact-DOB pass (shares_blocking_key dob branch)", "FROM patient_demographic WHERE field = 'dob'"),
    ("identifier pass excluding 'unknown' (_identifier_keys)", "FROM patient_identifier WHERE system <> 'unknown'"),
    ("name-token pass: NFC + lower + whitespace split (name_tokens)", "regexp_split_to_table(lower(normalize(value, NFC)), '\\s+')"),
    # §5.4: the placeholder-use exclusion the name_tokens mirror (generator._is_placeholder_name)
    # depends on. If the SQL drops it, the SQL blocks on callsigns while the mirror excludes
    # them — the two disagree, and the eval mis-reports blocking recall.
    ("placeholder-use exclusion in name_tokens (§5.4)", "use_key <> ALL(%s)"),
]


@pytest.mark.parametrize("assumption, fragment", _MIRRORED_PASSES)
def test_shares_blocking_key_mirrors_the_blocking_sql(assumption, fragment):
    assert fragment in _GROUPS_SQL, (
        f"_GROUPS_SQL no longer contains the base pass that shares_blocking_key mirrors: "
        f"{assumption}. Update generator.shares_blocking_key to match — otherwise the "
        f"synthetic generator's recoverability guarantee is silently false."
    )
