# matcher/tests/conftest.py
"""Shared fixtures for the gated integration tests.

These tests need a real PostgreSQL >= 18 with the cairn_pgx extension installed (the same
substrate the Rust DB-gated tests use). They are SKIPPED cleanly when CAIRN_TEST_PG is
unset, so `uv run pytest` stays green on a machine with no database.

The conftest applies the node schema itself (the same db/*.sql files, in the same order,
the cairn-node loader applies on connect — all idempotent) so the Python suite is
self-sufficient given a PG+cairn_pgx cluster.
"""

import hashlib
import os
from contextlib import contextmanager
from pathlib import Path

import pytest

CAIRN_TEST_PG = os.environ.get("CAIRN_TEST_PG")


def _seed_content_address(*parts: str) -> bytes:
    """A synthetic, multihash-shaped content_address for a directly-seeded projection row.

    The state overlays (name_repudiation, chart_identity_state, …) carry the winning event's
    `content_address` as the #115 deterministic overlay tiebreaker, and store it NOT NULL. The
    seeds below inject projection rows directly (bypassing the event floor on purpose — see
    their docstrings), so they must supply this column themselves. A real content_address is
    `\\x1220` + sha256(signed_bytes); we mirror that shape with a deterministic digest of the
    row's identifying fields so each seed row gets a distinct, stable value. It is inert for
    these consumption tests (any real event carries hlc_wall > 0 and so outranks the seed's
    (0, 0, 'seed') triple before the tiebreaker is ever consulted). Not key material.
    """
    return b"\x12\x20" + hashlib.sha256("|".join(parts).encode()).digest()

_DB_DIR = Path(__file__).resolve().parents[2] / "db"

# Mirror crates/cairn-node/src/db.rs SCHEMA: every top-level db/*.sql in filename order,
# minus the deliberate exclusions below. DERIVED from disk rather than hand-written
# (issue #212: the previous hand copy silently stalled at 025 while the loader grew to
# 038), so a new migration is picked up the moment it lands. The Rust loader is pinned
# to the same disk set by the #188 fs-derived guards (cairn-event's schema_generation
# test + cairn-node's completeness test), which is what keeps this derivation and
# db.rs's explicit list from drifting apart.
_SKIPPED_SCHEMA_FILES = {
    "008_surrogate_projection",  # spike-only; db.rs deliberately does not load it either
}
_SCHEMA_FILES = sorted(
    p.stem
    for p in _DB_DIR.glob("[0-9][0-9][0-9]_*.sql")
    if p.stem not in _SKIPPED_SCHEMA_FILES
)

# Projection tables a test seeds / the fixture truncates between tests. name_repudiation
# (db/025) backs the patient_alias_pool view the known-alias matcher reads.
_PROJECTION_TABLES = [
    "match_proposal", "patient_identifier", "patient_demographic", "patient_name",
    "name_repudiation", "chart_identity_state",
]


def _apply_schema(conn) -> None:
    """Apply every SCHEMA file in order (idempotent; CREATE IF NOT EXISTS / OR REPLACE)."""
    with conn.cursor() as cur:
        for name in _SCHEMA_FILES:
            cur.execute((_DB_DIR / f"{name}.sql").read_text())
    conn.commit()


def _truncate_projections(conn) -> None:
    """TRUNCATE + commit every projection table, on its own clean transaction.

    Rolls back first so it runs even if the caller left an aborted transaction behind
    (a failed test), then commits so the empty state is durable for the NEXT connection —
    which is the whole point: an external consumer (e.g. the eval harness) must never
    inherit committed rows. Best-effort by design (see managed_pg_conn): teardown must
    never mask the test's own failure with a cleanup error.
    """
    conn.rollback()
    with conn.cursor() as cur:
        cur.execute(f"TRUNCATE {', '.join(_PROJECTION_TABLES)}")
    conn.commit()


@contextmanager
def managed_pg_conn(dsn):
    """Yield a schema-applied connection with projection tables empty on entry AND exit.

    The lifecycle every DB-gated matcher test shares, factored out so the exit contract is
    directly testable (issue #84 pt1): because seed helpers COMMIT, a rollback-only
    teardown could not undo the last test's writes and they leaked into the database. Here
    the projection tables are truncated on entry (clean start) and again on exit (clean
    exit), so no committed row survives regardless of what the test did.
    """
    import psycopg

    conn = psycopg.connect(dsn, autocommit=False)
    try:
        _apply_schema(conn)
        _truncate_projections(conn)
        yield conn
    finally:
        # Best-effort exit truncation: swallow any cleanup error so it can never REPLACE the
        # test's own exception as the surfaced failure. A generator context manager
        # re-raises whatever its finally raises, so an un-caught truncation error (e.g. the
        # test crashed the connection) would mask the real assertion failure. Safe to
        # swallow — the NEXT connection truncates on entry regardless, and the lifecycle
        # test asserts the exit guarantee on a healthy connection.
        try:
            _truncate_projections(conn)
        except Exception as e:
            print(f"managed_pg_conn cleanup warning: failed to truncate projections: {e}")
        finally:
            conn.close()


@pytest.fixture
def pg_conn():
    """A connection with schema applied and projection tables truncated; skip if no DB."""
    if not CAIRN_TEST_PG:
        pytest.skip("CAIRN_TEST_PG not set — skipping DB-gated integration test")
    with managed_pg_conn(CAIRN_TEST_PG) as conn:
        yield conn


def seed_patient(
    conn, patient_id, *, dob=None, sex=None, admin_sex=None, names=(), identifiers=(),
    callsign_names=()
):
    """Insert projection rows for one patient directly (bypassing submit_event).

    dob: (value, provenance_rank[, precision]) tuple or None.
    sex/admin_sex: (value, provenance_rank) tuples or None — sex seeds
    field='sex-at-birth'; admin_sex seeds field='administrative-sex' (the
    apparent/phenotypic field a §5.4 clinician-observed sex lands on).
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
                "provenance, provenance_rank, asserted_hlc_wall, "
                "asserted_hlc_count, asserted_origin) "
                "VALUES (%s,'dob',%s,%s,'seed',%s,0,0,'seed')",
                (patient_id, value, json.dumps({"precision": precision}), rank),
            )
        if sex is not None:
            value, rank = sex
            cur.execute(
                "INSERT INTO patient_demographic (patient_id, field, value, facets, "
                "provenance, provenance_rank, asserted_hlc_wall, "
                "asserted_hlc_count, asserted_origin) "
                "VALUES (%s,'sex-at-birth',%s,NULL,'seed',%s,0,0,'seed')",
                (patient_id, value, rank),
            )
        if admin_sex is not None:
            value, rank = admin_sex
            cur.execute(
                "INSERT INTO patient_demographic (patient_id, field, value, facets, "
                "provenance, provenance_rank, asserted_hlc_wall, "
                "asserted_hlc_count, asserted_origin) "
                "VALUES (%s,'administrative-sex',%s,NULL,'seed',%s,0,0,'seed')",
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
                "profile, use_type, provenance, asserted_hlc_wall, "
                "asserted_hlc_count, asserted_origin) "
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
            "INSERT INTO name_repudiation "
            "(subject, value, reason, hlc_wall, hlc_counter, origin, content_address) "
            "VALUES (%s,%s,%s,0,0,'seed',%s) ON CONFLICT DO NOTHING",
            (subject, value, reason, _seed_content_address(subject, value, "repudiation")),
        )
    conn.commit()


def seed_identity_pending(conn, subject, *, basis="unidentified at registration"):
    """Mark a chart identity-pending directly (chart_identity_state), bypassing the C4 floor.

    The floor (submit_event + authored twin + the identity-state assertion gate) is proven
    in crates/cairn-node/tests; these matcher tests exercise CONSUMPTION of the chart_trust
    projection, not how it is written — the same rationale as seed_repudiation above.
    """
    with conn.cursor() as cur:
        cur.execute(
            "INSERT INTO chart_identity_state "
            "(subject, state, detail, hlc_wall, hlc_counter, origin, content_address) "
            "VALUES (%s,'pending',%s,0,0,'seed',%s) "
            "ON CONFLICT (subject) DO UPDATE SET state='pending', detail=EXCLUDED.detail",
            (subject, basis, _seed_content_address(subject, "pending")),
        )
    conn.commit()
