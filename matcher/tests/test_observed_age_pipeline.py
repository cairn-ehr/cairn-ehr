"""§5.4 e2e: a clinician-observed estimated-age range scores as a positive dob signal.

Exercises the real projection -> load_candidate -> field_comparisons loop on live DB rows.

Gated on CAIRN_TEST_PG (skips cleanly without a database) via the `pg_conn` fixture's own
internal pytest.skip -- the house convention every other DB-gated test in this suite follows
(test_pipeline_smoke.py, test_candidate_generation.py, ...), rather than a duplicate
module-level pytestmark. `cairn_matcher.pipeline.db` is the one psycopg-touching module, so
`load_candidate` is imported LAZILY inside `_dob_level` (not at module top): a top-level
import would pull psycopg in at COLLECTION time and break the pure `uv run pytest` run,
which has no psycopg installed (see test_alias_pipeline.py / test_john_doe_exclusion.py for
the same pattern).
"""
import uuid

from cairn_matcher.agreement import AgreementLevel
from cairn_matcher.orchestrator import field_comparisons
from tests.conftest import seed_patient


def _uid() -> str:
    return str(uuid.uuid4())


def _dob_level(conn, a: str, b: str) -> AgreementLevel:
    """The dob AgreementLevel for the pair, straight from the real scoring path."""
    from cairn_matcher.pipeline.db import load_candidate

    rec_a = load_candidate(conn, a)
    rec_b = load_candidate(conn, b)
    comparisons = field_comparisons(rec_a, rec_b)
    return next(fc.level for fc in comparisons if fc.field == "dob")


def test_candidate_dob_inside_the_estimated_range_is_a_positive_dob_signal(pg_conn):
    john_doe, candidate = _uid(), _uid()
    # John Doe: an estimated birth-year range (clinician-observed, rank 30).
    seed_patient(pg_conn, john_doe, dob=("1981/1991", 30, "year-range"), sex=("male", 30))
    # Candidate: a real document dob INSIDE the range.
    seed_patient(pg_conn, candidate, dob=("1985-03-12", 60, "day"), sex=("male", 60))

    assert _dob_level(pg_conn, john_doe, candidate) == AgreementLevel.PARTIAL


def test_candidate_dob_outside_the_range_is_neither_penalty_nor_veto(pg_conn):
    john_doe, candidate = _uid(), _uid()
    seed_patient(pg_conn, john_doe, dob=("1981/1991", 30, "year-range"))
    seed_patient(pg_conn, candidate, dob=("1950-01-01", 60, "day"))

    # Outside the range -> INSUFFICIENT_DATA (positive-only; never a DISAGREE penalty).
    assert _dob_level(pg_conn, john_doe, candidate) == AgreementLevel.INSUFFICIENT_DATA
