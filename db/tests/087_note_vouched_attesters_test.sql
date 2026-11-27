\set ON_ERROR_STOP on
\ir _scratch_database_guard.sql
-- db/087 — the set-returning attester door (#1021).
--
-- WHY: the note list and the person menu used to ask db/065's one-event door about every part,
-- one function call each (about a fifth of a ward-year's menu read). db/087's door answers for
-- a whole array of events in one call. It must answer EXACTLY what the one-event door answers
-- for each event, or a note would grade "signed by" someone differently in the two reads that
-- use the doors; this test pins that agreement on db/065's three fixtures, plus the guards
-- (definer-run, pinned search_path, the window's and nobody else's). Nothing commits.

BEGIN;

-- db/065's fixtures, verbatim: no attester, an attester with no marker, an attester with a
-- carried-not-vouched marker.
SET LOCAL session_replication_role = replica;
INSERT INTO event_log (event_id, patient_id, event_type, schema_version, hlc_wall, hlc_counter,
                       node_origin, signed_bytes, content_address, body, contributors,
                       signer_key_id, plaintext_twin, attester_key)
SELECT ('aaaaaaaa-0000-7000-8000-0000000000c' || i)::uuid, 'bbbbbbbb-0000-7000-8000-000000000001',
       'note.added', '1', 10 + i, 1, 'n1', ('\xc' || i)::bytea,
       '\x1220'::bytea || digest(('\xc' || i)::bytea, 'sha256'),
       '{}'::jsonb, '[]'::jsonb, 'deadbeef', 'attester fixture',
       CASE WHEN i = 1 THEN NULL ELSE digest('attester-' || i, 'sha256') END
  FROM generate_series(1, 3) AS i;
INSERT INTO event_attestation_unvouched (event_id) VALUES ('aaaaaaaa-0000-7000-8000-0000000000c3');
SET LOCAL session_replication_role = origin;

DO $$
DECLARE
    ids uuid[] := ARRAY['aaaaaaaa-0000-7000-8000-0000000000c1', 'aaaaaaaa-0000-7000-8000-0000000000c2',
                        'aaaaaaaa-0000-7000-8000-0000000000c3', 'aaaaaaaa-0000-7000-8000-0000000000ff']::uuid[];
    id uuid;
BEGIN
    -- Only the vouched attester comes back: no row for no attester, a marked one, or an unknown event.
    ASSERT (SELECT array_agg(event_id) FROM cairn_note_vouched_attesters(ids))
           = ARRAY['aaaaaaaa-0000-7000-8000-0000000000c2']::uuid[],
        'only the vouched attester has a row';
    -- Event by event, the same answer as db/065's door (a missing row reads as its NULL).
    FOREACH id IN ARRAY ids LOOP
        ASSERT (SELECT a.attester FROM cairn_note_vouched_attesters(ids) a WHERE a.event_id = id)
               IS NOT DISTINCT FROM cairn_note_vouched_attester(id),
            'the set door and the one-event door disagree on ' || id;
    END LOOP;
    -- An empty or NULL array (a chart with no notes: `array_agg` over nothing is NULL) answers nothing.
    ASSERT NOT EXISTS (SELECT 1 FROM cairn_note_vouched_attesters(NULL)), 'NULL array: no rows';
    ASSERT NOT EXISTS (SELECT 1 FROM cairn_note_vouched_attesters('{}')), 'empty array: no rows';
    ASSERT (SELECT prosecdef AND proconfig @> ARRAY['search_path=public, pg_temp'] FROM pg_proc
             WHERE oid = 'cairn_note_vouched_attesters(uuid[])'::regprocedure),
        'the door is definer-run with a pinned search_path';
    ASSERT NOT has_function_privilege('public', 'cairn_note_vouched_attesters(uuid[])', 'EXECUTE'),
        'the door must not be PUBLIC';
    ASSERT has_function_privilege('cairn_window', 'cairn_note_vouched_attesters(uuid[])', 'EXECUTE'),
        'the door is the window''s';
END $$;

ROLLBACK;
