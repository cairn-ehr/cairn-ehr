"""§5.4 identity-pending trust plumbing + the #130 end-to-end (DB-gated).

Gated on CAIRN_TEST_PG via the pg_conn fixture's internal skip (house convention).
psycopg-touching imports stay INSIDE test functions so the pure `uv run pytest` run
(no psycopg installed) still collects this module cleanly.
"""
import uuid

from tests.conftest import seed_identity_pending, seed_patient


def _uid() -> str:
    return str(uuid.uuid4())


def test_load_trust_for_reads_pending_as_unconfirmed(pg_conn):
    from cairn_matcher.pipeline.db import load_trust, load_trust_for

    doe, ordinary = _uid(), _uid()
    seed_patient(pg_conn, doe, dob=("1981/1991", 30, "year-range"))
    seed_patient(pg_conn, ordinary, dob=("1985-03-12", 60, "day"))
    seed_identity_pending(pg_conn, doe)

    trust = load_trust_for(pg_conn, [doe, ordinary])
    assert trust.get(doe) == "unconfirmed"
    assert ordinary not in trust  # absent row = confirmed default
    assert load_trust(pg_conn, doe) == "unconfirmed"
    assert load_trust(pg_conn, ordinary) is None


def test_load_trust_for_empty_set_is_empty(pg_conn):
    from cairn_matcher.pipeline.db import load_trust_for

    assert load_trust_for(pg_conn, []) == {}


def test_load_candidate_populates_administrative_sex(pg_conn):
    # The Task-3 widened SELECT, proven against live projection rows (spec §6 DB-gated).
    from cairn_matcher.pipeline.db import load_candidate

    doe = _uid()
    seed_patient(pg_conn, doe, admin_sex=("male", 30))
    rec = load_candidate(pg_conn, doe)
    assert rec.administrative_sex is not None
    assert rec.administrative_sex.value == "male"
    assert rec.administrative_sex.provenance_rank == 30
    assert rec.sex_at_birth is None
