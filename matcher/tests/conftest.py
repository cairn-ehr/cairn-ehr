# matcher/tests/conftest.py
"""Shared fixtures for the gated integration tests.

These tests need a real PostgreSQL >= 18 with the cairn_pgx extension installed (the same
substrate the Rust DB-gated tests use). They are SKIPPED cleanly when CAIRN_TEST_PG is
unset, so `uv run pytest` stays green on a machine with no database.

The conftest applies the node schema itself (the same db/*.sql files, in the same order,
the cairn-node loader applies on connect — all idempotent) so the Python suite is
self-sufficient given a PG+cairn_pgx cluster.
"""

import os
from pathlib import Path

import pytest

CAIRN_TEST_PG = os.environ.get("CAIRN_TEST_PG")

# Mirror crates/cairn-node/src/db.rs SCHEMA order. 008 is intentionally skipped (spike-only).
_SCHEMA_FILES = [
    "001_envelope", "002_projection", "003_blobs", "004_actors", "005_submit",
    "006_recall", "007_node_federation", "009_node_supersede_and_restore",
    "010_demographics", "011_demographics_fields", "012_demographics_names",
    "013_demographics_sex_gender", "014_demographics_address", "015_globalise_twin",
    "016_match_veto", "017_match_proposal", "018_identity_linkage", "019_apply_proposal",
    "020_apply_remote_event", "021_sync_quarantine", "022_node_event_quarantine",
    "023_identity_dispute", "024_identity_identify", "025_identity_repudiate",
]

_DB_DIR = Path(__file__).resolve().parents[2] / "db"

# Projection tables a test seeds / the fixture truncates between tests. name_repudiation
# (db/025) backs the patient_alias_pool view the known-alias matcher reads.
_PROJECTION_TABLES = [
    "match_proposal", "patient_identifier", "patient_demographic", "patient_name",
    "name_repudiation",
]


def _apply_schema(conn) -> None:
    """Apply every SCHEMA file in order (idempotent; CREATE IF NOT EXISTS / OR REPLACE)."""
    with conn.cursor() as cur:
        for name in _SCHEMA_FILES:
            cur.execute((_DB_DIR / f"{name}.sql").read_text())
    conn.commit()


@pytest.fixture
def pg_conn():
    """A connection with schema applied and projection tables truncated; skip if no DB."""
    if not CAIRN_TEST_PG:
        pytest.skip("CAIRN_TEST_PG not set — skipping DB-gated integration test")
    import psycopg

    conn = psycopg.connect(CAIRN_TEST_PG, autocommit=False)
    try:
        _apply_schema(conn)
        with conn.cursor() as cur:
            cur.execute(f"TRUNCATE {', '.join(_PROJECTION_TABLES)}")
        conn.commit()
        yield conn
    finally:
        conn.rollback()
        conn.close()


def seed_patient(
    conn, patient_id, *, dob=None, sex=None, names=(), identifiers=(), callsign_names=()
):
    """Insert projection rows for one patient directly (bypassing submit_event).

    dob/sex: (value, provenance_rank[, precision]) tuples or None.
    names: iterable of (value, provenance_rank) — seeded under use_key='legal'.
    callsign_names: iterable of (value, provenance_rank) — seeded under use_key='callsign',
        the §5.4 placeholder use the matcher excludes from its feature space.
    identifiers: iterable of (system, match_key, value).
    """
    import json

    with conn.cursor() as cur:
        if dob is not None:
            value, rank, *rest = dob
            precision = rest[0] if rest else "day"
            cur.execute(
                "INSERT INTO patient_demographic (patient_id, field, value, facets, "
                "provenance, provenance_rank, asserted_hlc_wall, asserted_hlc_count, asserted_origin) "
                "VALUES (%s,'dob',%s,%s,'seed',%s,0,0,'seed')",
                (patient_id, value, json.dumps({"precision": precision}), rank),
            )
        if sex is not None:
            value, rank = sex
            cur.execute(
                "INSERT INTO patient_demographic (patient_id, field, value, facets, "
                "provenance, provenance_rank, asserted_hlc_wall, asserted_hlc_count, asserted_origin) "
                "VALUES (%s,'sex-at-birth',%s,NULL,'seed',%s,0,0,'seed')",
                (patient_id, value, rank),
            )
        for value, rank in names:
            cur.execute(
                "INSERT INTO patient_name (patient_id, use_key, value, use_raw, provenance, "
                "provenance_rank, last_hlc_wall, last_hlc_count, asserted_origin) "
                "VALUES (%s,'legal',%s,'legal','seed',%s,0,0,'seed') ON CONFLICT DO NOTHING",
                (patient_id, value, rank),
            )
        for value, rank in callsign_names:
            cur.execute(
                "INSERT INTO patient_name (patient_id, use_key, value, use_raw, provenance, "
                "provenance_rank, last_hlc_wall, last_hlc_count, asserted_origin) "
                "VALUES (%s,'callsign',%s,'callsign','seed',%s,0,0,'seed') ON CONFLICT DO NOTHING",
                (patient_id, value, rank),
            )
        for system, match_key, value in identifiers:
            cur.execute(
                "INSERT INTO patient_identifier (patient_id, system, match_key, value, normalized, "
                "profile, use_type, provenance, asserted_hlc_wall, asserted_hlc_count, asserted_origin) "
                "VALUES (%s,%s,%s,%s,%s,NULL,NULL,'seed',0,0,'seed') ON CONFLICT DO NOTHING",
                (patient_id, system, match_key, value, match_key),
            )
    conn.commit()


def seed_repudiation(conn, subject, value, *, reason="test fabricated persona"):
    """Insert a name_repudiation row directly, backing patient_alias_pool for a chart.

    Bypasses the C5 suppressing-event floor (submit_event + human attestation) on purpose:
    that floor is proven in crates/cairn-node/tests/identity_repudiate.rs. These matcher
    tests exercise CONSUMPTION of the resulting projection, not how it is written.
    """
    with conn.cursor() as cur:
        cur.execute(
            "INSERT INTO name_repudiation (subject, value, reason, hlc_wall, hlc_counter, origin) "
            "VALUES (%s,%s,%s,0,0,'seed') ON CONFLICT DO NOTHING",
            (subject, value, reason),
        )
    conn.commit()
