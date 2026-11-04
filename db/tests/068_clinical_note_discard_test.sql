\set ON_ERROR_STOP on
\ir _scratch_database_guard.sql
-- db/068 — the in-DB mirror for the discard stream's floor and projection
-- (`clinical.note-discard.asserted`, ADR-0089 decision 15; sign-in slice 3b-2b).
--
-- WHY THIS MIRROR EXISTS: cairn_check_note_discard is the gate between a raw-SQL client and a
-- signed audit record whose narratives a clinician is later shown. Each floor rule gets one body
-- it refuses (asserting WHICH rule refused it, by message fragment). The Rust suite
-- (crates/cairn-node/tests/note_discard_floor.rs) drives the real seal -> sign -> submit path.
--
-- Tests, in order (one DO block or SELECT group each):
--   registration_and_the_type_is_not_threadless
--   a_well_formed_discard_is_admitted
--   one refusal per floor rule (missing payload, schema tag, parts, part, writer, narrative,
--     markup, dangling anchor, recording times, order, stated_time, t_effective)
--   a_remote_newer_profile_is_carried_not_projected
--   a_well_formed_discard_is_projected
--   a_shred_scrubs_the_discard_row
--
-- Wrapped in a transaction that ROLLS BACK: nothing here commits.

\o /dev/null
BEGIN;

-- A valid discard body with `over` merged into its first part; two parts so ordering is covered.
CREATE FUNCTION pg_temp.part(over jsonb DEFAULT '{}') RETURNS jsonb LANGUAGE sql AS $$
    SELECT jsonb_build_object(
        'writer',            'discarder',
        'narrative',         'chest clear',
        'stated_time',       NULL,
        'recording_started', '2026-11-04T09:00:00+10:00',
        'recording_ended',   '2026-11-04T09:05:00+10:00') || over;
$$;
CREATE FUNCTION pg_temp.body(parts jsonb DEFAULT NULL) RETURNS jsonb LANGUAGE sql AS $$
    SELECT jsonb_build_object(
        'schema_version', 'clinical.note-discard/1',
        't_effective',    '2026-11-04T09:10:00+10:00',
        'plaintext_twin', 'Draft discarded; it was never entered in the record.',
        'payload', jsonb_build_object('parts',
            COALESCE(parts, jsonb_build_array(pg_temp.part(), pg_temp.part('{"writer":"not-known"}')))));
$$;
CREATE FUNCTION pg_temp.one(over jsonb) RETURNS jsonb LANGUAGE sql AS $$
    SELECT pg_temp.body(jsonb_build_array(pg_temp.part(over)));
$$;

CREATE FUNCTION pg_temp.accepts(b jsonb) RETURNS void LANGUAGE plpgsql AS $$
BEGIN
    PERFORM cairn_check_note_discard('clinical.note-discard.asserted', b);
EXCEPTION WHEN raise_exception THEN
    RAISE EXCEPTION 'FAIL: refused a body it must accept (%): %', SQLERRM, b->'payload';
END $$;

-- Refused with a message containing `frag`; refusing for a DIFFERENT reason is a failure too
-- (the rule under test never ran). `own` = the message must be the discard floor's own.
CREATE FUNCTION pg_temp.refuses(b jsonb, frag text, own boolean DEFAULT true) RETURNS void LANGUAGE plpgsql AS $$
BEGIN
    BEGIN
        PERFORM cairn_check_note_discard('clinical.note-discard.asserted', b);
    EXCEPTION WHEN raise_exception THEN
        IF position(frag IN SQLERRM) = 0 THEN
            RAISE EXCEPTION 'FAIL: refused for the wrong reason: % (wanted "%")', SQLERRM, frag;
        END IF;
        IF own AND SQLERRM NOT LIKE 'note discard: %' THEN
            RAISE EXCEPTION 'FAIL: refusal is not the discard floor''s own: %', SQLERRM;
        END IF;
        RETURN;
    END;
    RAISE EXCEPTION 'FAIL: accepted a body it must refuse ("%"): %', frag, b->'payload';
END $$;

-- registration_and_the_type_is_not_threadless
DO $$
BEGIN
    ASSERT (SELECT mode FROM event_type_class WHERE event_type = 'clinical.note-discard.asserted') = 'additive',
        'the discard type is registered additive';
    ASSERT NOT cairn_event_type_has_no_thread('clinical.note-discard.asserted'),
        'the_type_is_not_threadless: it stays off db/048''s no-thread list, as notes do';
    ASSERT (SELECT check_fn FROM cairn_event_twin_check WHERE event_type = 'clinical.note-discard.asserted')
           = 'cairn_check_note_discard', 'the floor is registered';
    ASSERT (SELECT twin_required_msg FROM cairn_event_twin_check WHERE event_type = 'clinical.note-discard.asserted')
           = 'note discard requires a non-empty authored twin (§3.13/§3.19)', 'the twin message';
    ASSERT (SELECT heal_safe AND run_order = 20 FROM cairn_projection_apply
             WHERE event_type = 'clinical.note-discard.asserted' AND apply_fn = 'clinical_note_discard_apply'),
        'the projection is registered heal-safe, run_order 20';
END $$;

-- a_well_formed_discard_is_admitted: plain, escaped markup, a stated time, an Offset spelling.
SELECT pg_temp.accepts(pg_temp.body());
SELECT pg_temp.accepts(pg_temp.one(jsonb_build_object('narrative', t))) FROM unnest(ARRAY[
    'BP \<90', 'see [x]\(http://e.org)', 'ends with \\', 'BP <90']) AS t;
SELECT pg_temp.accepts(pg_temp.one('{"stated_time":"2026-11-04T06:00:00Z"}'));

-- Rule: the payload exists, the schema tag is /1 (authoring door).
SELECT pg_temp.refuses(pg_temp.body() - 'payload', 'missing payload');
SELECT pg_temp.refuses(pg_temp.body() || '{"schema_version":"clinical.note-discard/2"}', 'schema_version');

-- Rule: parts is a non-empty array of objects.
SELECT pg_temp.refuses(pg_temp.body('[]'), 'non-empty array');
SELECT pg_temp.refuses(pg_temp.body() #- '{payload,parts}', 'non-empty array');
SELECT pg_temp.refuses(pg_temp.body() || '{"payload":{"parts":{}}}', 'non-empty array');
SELECT pg_temp.refuses(pg_temp.body('["text"]'), 'must be an object');

-- Rule: the writer is a closed word.
SELECT pg_temp.refuses(pg_temp.one(jsonb_build_object('writer', w)), 'writer')
FROM unnest(ARRAY['Dr B', 'discarder ', 'nurse-key-7']) AS w;
SELECT pg_temp.refuses(pg_temp.one('{"writer":42}'), 'writer');
SELECT pg_temp.refuses(pg_temp.body(jsonb_build_array(pg_temp.part() - 'writer')), 'writer');

-- Rule: a narrative that is a string and not blank.
SELECT pg_temp.refuses(pg_temp.one('{"narrative":42}'), 'narrative');
SELECT pg_temp.refuses(pg_temp.one(jsonb_build_object('narrative', E'  \n\t ')), 'narrative');
SELECT pg_temp.refuses(pg_temp.body(jsonb_build_array(pg_temp.part() - 'narrative')), 'narrative');
-- ... a later part is judged too, not just the first.
SELECT pg_temp.refuses(pg_temp.body(jsonb_build_array(pg_temp.part(), pg_temp.part('{"narrative":" "}'))), 'narrative');

-- Rule: db/064's markup check (raw HTML, external link, reference definition) on EVERY part.
-- Messages are db/064's own ('note assertion: ...'), proving the shared function ran.
SELECT pg_temp.refuses(pg_temp.one(jsonb_build_object('narrative', t)), 'note assertion:', false)
FROM unnest(ARRAY['pt said <b>', 'see [x](http://e.org)', '[ref]: http://e.org']) AS t;
SELECT pg_temp.refuses(pg_temp.body(jsonb_build_array(pg_temp.part(), pg_temp.part('{"narrative":"<script>"}'))),
                       'raw HTML', false);

-- Rule: no media exists, so every anchor dangles.
SELECT pg_temp.refuses(pg_temp.one('{"narrative":"![wound, L shin](cairn:att/m1)"}'), 'names no media entry', false);

-- Rule: recording times are explicit-offset instants, in order.
SELECT pg_temp.refuses(pg_temp.body(jsonb_build_array(pg_temp.part() - 'recording_started')), 'recording_');
SELECT pg_temp.refuses(pg_temp.one(jsonb_build_object('recording_ended', t)), 'recording_')
FROM unnest(ARRAY['yesterday', 'now', '2026-11-04 09:05', '2026-11-04T09:05:00']) AS t;
SELECT pg_temp.refuses(pg_temp.one('{"recording_ended":"2026-11-04T08:59:00+10:00"}'), 'after');

-- Rule: stated_time is JSON null or an instant.
SELECT pg_temp.refuses(pg_temp.body(jsonb_build_array(pg_temp.part() - 'stated_time')), 'stated_time');
SELECT pg_temp.refuses(pg_temp.one(jsonb_build_object('stated_time', t)), 'stated_time')
FROM unnest(ARRAY['yesterday', 'now', '2026-11-04T09:05:00', '']) AS t;
SELECT pg_temp.refuses(pg_temp.one('{"stated_time":42}'), 'stated_time');
SELECT pg_temp.refuses(pg_temp.one('{"stated_time":true}'), 'stated_time');

-- Rule: the envelope t_effective (the discard instant) is set.
SELECT pg_temp.refuses(pg_temp.body() - 't_effective', 't_effective');
SELECT pg_temp.refuses(pg_temp.body() || '{"t_effective":null}', 't_effective');

-- a_remote_newer_profile_is_carried_not_projected (ruling R8): at the remote door a /2 body that
-- breaks /1's rules is admitted; the authoring door still refuses it; the applier projects /1 only.
SET LOCAL cairn.remote_apply = 'on';
SELECT pg_temp.accepts(pg_temp.one('{"narrative":"pt said <b>"}') || '{"schema_version":"clinical.note-discard/2"}');
-- Every other rule still applies to a /1 body at the remote door.
SELECT pg_temp.refuses(pg_temp.one('{"narrative":"pt said <b>"}'), 'raw HTML', false);
SET LOCAL cairn.remote_apply = '';
SELECT pg_temp.refuses(pg_temp.one('{"narrative":"x"}') || '{"schema_version":"clinical.note-discard/2"}', 'schema_version');

-- The applier, through the real dispatcher: unsealed rows inserted as the owner (the floor is
-- not under test here; cairn_clear_payload reads an unsealed row's body as its clear payload).
INSERT INTO event_log (event_id, patient_id, event_type, schema_version, hlc_wall,
                       hlc_counter, node_origin, t_effective, signed_bytes, content_address,
                       body, contributors, signer_key_id, plaintext_twin)
SELECT id::uuid, '68686868-0000-7868-8868-686868686868'::uuid, 'clinical.note-discard.asserted', v, 2, n, 'n1',
       '2026-11-04T09:10:00+10:00', sb, '\x1220'::bytea || digest(sb, 'sha256'),
       pg_temp.body() -> 'payload', '[]'::jsonb, 'deadbeef', 'Draft discarded; it was never entered in the record.'
FROM (VALUES ('68686868-0001-7868-8868-686868686868', 'clinical.note-discard/1', 1, '\x6801'::bytea),
             ('68686868-0002-7868-8868-686868686868', 'clinical.note-discard/2', 2, '\x6802'::bytea))
     AS r(id, v, n, sb);
DO $$
BEGIN
    ASSERT (SELECT jsonb_array_length(parts) = 2 AND parts = pg_temp.body() -> 'payload' -> 'parts'
              AND t_effective = '2026-11-04T09:10:00+10:00'::timestamptz
              AND patient_id = '68686868-0000-7868-8868-686868686868'
              FROM clinical_note_discard WHERE event_id = '68686868-0001-7868-8868-686868686868'),
        'a_well_formed_discard_is_projected: the parts as is, the envelope instant';
    ASSERT NOT EXISTS (SELECT 1 FROM clinical_note_discard WHERE event_id = '68686868-0002-7868-8868-686868686868'),
        'a_remote_newer_profile_is_carried_not_projected';
    ASSERT NOT EXISTS (SELECT 1 FROM clinical_note WHERE patient_id = '68686868-0000-7868-8868-686868686868'),
        'a discard is never a note';
END $$;

-- a_shred_scrubs_the_discard_row: a discard's parts ARE clinical plaintext (db/037).
SELECT cairn_execute_shred('68686868-0001-7868-8868-686868686868', gen_random_uuid(), 'test');
DO $$
BEGIN
    ASSERT NOT EXISTS (SELECT 1 FROM clinical_note_discard WHERE event_id = '68686868-0001-7868-8868-686868686868'),
        'a_shred_scrubs_the_discard_row';
END $$;

ROLLBACK;
\o
