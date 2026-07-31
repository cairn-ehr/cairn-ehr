# matcher/tests/test_match_proposal_band_check.py
"""DB-gated: `match_proposal.band` is constrained to the `banding.Band` enum (issue #79).

The B2 review deferred a DB-level `CHECK (band IN (…))` as "defence in depth" — the values
are owned by the Python `Band` enum, and while several writers touch the table (see db/017's
header), only that pipeline writes the `band` COLUMN. So the CHECK guards against a writer
that ISN'T that pipeline (a psql session, a future service, a migration script) storing a
band no reader can interpret.

Defence in depth has a cost, though: the accepted set now lives in TWO places — this
`db/017` CHECK and `banding.Band`. That is precisely the two-place-mapping failure mode of
issue #119, so the constraint does not land bare. The first test below drives the DB with
EVERY enum member, so adding a third `Band` without the matching migration fails loudly
here rather than at a production INSERT; the second proves the CHECK actually rejects a
non-member (i.e. that it exists at all, and that the first test is not vacuous).

Gated on CAIRN_TEST_PG via the shared `pg_conn` fixture's own skip (house convention).
"""

import pytest

from cairn_matcher.pipeline.banding import Band

# A distinct canonical-ordered pair per inserted row. db/017 PRIMARY KEYs (low, high) and
# CHECKs low < high, so each case needs its own pair; the `high` half is held constant and
# the `low` half varies by index, which keeps low < high for every index used here.
_HIGH = "ffffffff-ffff-4fff-8fff-ffffffffffff"


def _pair(index: int) -> tuple[str, str]:
    """The index-th distinct (low, high) pair, canonically ordered (low < high)."""
    return (f"{index:08d}-0000-4000-8000-000000000000", _HIGH)


def _insert(conn, low: str, high: str, band_value: str) -> None:
    """Insert one minimally-populated advisory proposal row with the given band string.

    Bypasses `db.upsert_proposal` on purpose: the whole point of the CHECK is to constrain
    a writer that is NOT the Python pipeline, so the test must write raw SQL to exercise it.
    """
    with conn.cursor() as cur:
        cur.execute(
            "INSERT INTO match_proposal "
            "(patient_low, patient_high, score_total, band, "
            "veto_findings, evidence, matcher_version) "
            "VALUES (%s,%s,%s,%s,'[]'::jsonb,'[]'::jsonb,'test')",
            (low, high, 1.0, band_value),
        )


def test_db_check_accepts_every_band_enum_value(pg_conn):
    """Every `Band` member must be storable — the anti-drift half of the guard.

    If a future slice adds a third band to the Python enum and forgets the paired db/017
    migration, this fails on that member's INSERT: the loud, in-test signal that the two
    places have diverged.
    """
    for index, member in enumerate(Band):
        low, high = _pair(index)
        _insert(pg_conn, low, high, member.value)
    pg_conn.commit()

    with pg_conn.cursor() as cur:
        cur.execute("SELECT count(*) FROM match_proposal")
        assert cur.fetchone()[0] == len(Band)


def test_db_check_rejects_a_band_outside_the_enum(pg_conn):
    """A band string no reader can interpret must be refused by the database itself.

    Without this, the test above would pass just as happily against a table carrying no
    CHECK at all.
    """
    import psycopg

    low, high = _pair(0)
    with pytest.raises(psycopg.errors.CheckViolation):
        _insert(pg_conn, low, high, "not_a_real_band")
    # No rollback needed: the failed statement leaves the transaction aborted, and
    # `_truncate_projections` rolls back before it truncates precisely so teardown survives
    # a test that ended that way (see its docstring in conftest).
