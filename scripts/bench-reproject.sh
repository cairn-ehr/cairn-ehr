#!/usr/bin/env bash
# scripts/bench-reproject.sh — the #208/ADR-0057 measured numbers, reproducibly.
#
# Loads the current db/*.sql into a THROWAWAY database (same safety rules as
# run-db-sql-tests.sh), bulk-generates ~2,004,000 synthetic events (the Bet-B
# volume) as the table owner — the projection/replay path is what is being
# measured, so door-grade signatures are unnecessary; the content-address CHECK
# is satisfied by computing the digest in SQL — then measures:
#   1. per-insert dispatcher write-path latency (p50/p95 over 2,000 single-row
#      inserts against the full log — the B1-shaped number), and
#   2. one full heal replay: SELECT count(*) FROM cairn_reproject('').
# Run on the Mac dev box now; re-run unchanged on the Pi5 rig for the
# authoritative number (the follow-on issue filed at PR time).
set -euo pipefail
cd "$(dirname "$0")/.."

DBNAME="${1:-cairn_reproject_bench}"
case "$DBNAME" in
    cairn_test*)
        echo "refusing: cairn_test* databases belong to the test suites" >&2; exit 2;;
esac

echo "== recreating throwaway database ${DBNAME}"
dropdb --if-exists "$DBNAME"
createdb "$DBNAME"
psql -d "$DBNAME" -v ON_ERROR_STOP=1 -q -c "CREATE EXTENSION cairn_pgx;"

echo "== loading db/*.sql (product set — skipping spike-only 008, like the loaders)"
for f in db/[0-9]*.sql; do
    [ "$(basename "$f")" = "008_surrogate_projection.sql" ] && continue
    psql -d "$DBNAME" -v ON_ERROR_STOP=1 -q -f "$f"
done

echo "== generating ~2,004,000 events (Bet-B volume; 200 patients like B1)"
psql -d "$DBNAME" -v ON_ERROR_STOP=1 -q <<'SQL'
-- Deterministic synthetic corpus: 200 patient.created + per-patient note.added
-- filler + a demographic.field.asserted stream, so replay exercises both the
-- patient_chart path and the winner-table path. Direct owner INSERT: the
-- dispatcher trigger (the measured path) fires exactly as it would at a door.
INSERT INTO event_log (event_id, patient_id, event_type, schema_version,
    hlc_wall, hlc_counter, node_origin, signed_bytes, content_address,
    body, contributors, signer_key_id, plaintext_twin)
SELECT uuidv7(), pid, ty, 'bench-1', 1700000000000 + gs, 0, 'bench-node', sb,
       '\x1220'::bytea || digest(sb, 'sha256'),
       CASE WHEN ty = 'demographic.field.asserted'
            THEN jsonb_build_object('field', 'dob', 'value', '1970-01-01',
                                    'provenance', 'patient-reported')
            ELSE jsonb_build_object('name', 'Bench P' || (gs % 200), 'note', gs) END,
       '[]'::jsonb, 'bench-key', 'bench twin ' || gs
FROM (
    SELECT gs,
           ('00000000-0000-7000-8000-' || lpad(to_hex(gs % 200), 12, '0'))::uuid AS pid,
           CASE WHEN gs <= 200 THEN 'patient.created'
                WHEN gs % 10 = 0 THEN 'demographic.field.asserted'
                ELSE 'note.added' END AS ty,
           ('bench-' || gs)::bytea AS sb
    FROM generate_series(1, 2004000) AS gs
) g;
ANALYZE event_log;
SQL

echo "== (1) dispatcher write-path latency: 2,000 single-row inserts at full log"
psql -d "$DBNAME" -v ON_ERROR_STOP=1 <<'SQL'
DO $$
DECLARE
    t0 timestamptz; deltas double precision[] := '{}'; i int;
    pid uuid := '00000000-0000-7000-8000-000000000001';
    sb bytea;
BEGIN
    FOR i IN 1..2000 LOOP
        sb := ('bench-tail-' || i)::bytea;
        t0 := clock_timestamp();
        INSERT INTO event_log (event_id, patient_id, event_type, schema_version,
            hlc_wall, hlc_counter, node_origin, signed_bytes, content_address,
            body, contributors, signer_key_id, plaintext_twin)
        VALUES (uuidv7(), pid, 'note.added', 'bench-1', 1800000000000 + i, 0,
            'bench-node', sb, '\x1220'::bytea || digest(sb, 'sha256'),
            jsonb_build_object('note', i), '[]'::jsonb, 'bench-key', 'tail');
        deltas := deltas || extract(epoch FROM clock_timestamp() - t0) * 1000.0;
    END LOOP;
    RAISE NOTICE 'write-path ms  p50=%  p95=%  max=%',
        (SELECT percentile_cont(0.5)  WITHIN GROUP (ORDER BY d) FROM unnest(deltas) d),
        (SELECT percentile_cont(0.95) WITHIN GROUP (ORDER BY d) FROM unnest(deltas) d),
        (SELECT max(d) FROM unnest(deltas) d);
END $$;
SQL

echo "== (2) full heal replay at Bet-B volume"
psql -d "$DBNAME" -v ON_ERROR_STOP=1 -c "\\timing on" \
     -c "SELECT * FROM cairn_reproject('', false, 'manual');" \
     -c "SELECT prefix, events_seen, elapsed_ms, skipped_fns FROM reproject_log ORDER BY id DESC LIMIT 1;"

echo "== done — record BOTH numbers (env: $(uname -srm), PG $(psql -d "$DBNAME" -tAc 'show server_version'))"
