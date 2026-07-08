# matcher/tests/test_pipeline_smoke.py
"""Smoke test: the gated fixture applies schema, seeds, and reads back. Proves the
integration substrate works (or skips cleanly with no DB) before the real pipeline tests.
"""

from tests.conftest import seed_patient

PA = "11111111-1111-1111-1111-111111111111"


def test_fixture_seeds_and_reads_back(pg_conn):
    seed_patient(pg_conn, PA, dob=("1980-07-15", 60, "day"), names=[("Alex Smith", 20)])
    with pg_conn.cursor() as cur:
        cur.execute(
            "SELECT value FROM patient_demographic WHERE patient_id = %s AND field='dob'",
            (PA,),
        )
        assert cur.fetchone()[0] == "1980-07-15"
        cur.execute("SELECT count(*) FROM patient_name WHERE patient_id = %s", (PA,))
        assert cur.fetchone()[0] == 1
