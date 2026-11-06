\set ON_ERROR_STOP on
\ir _scratch_database_guard.sql
-- db/071 — the in-DB mirror for the vouch stream's floor and projection
-- (`clinical.note-attestation.asserted`, ADR-0091; sign-in slice 3c-2).
--
-- WHY THIS MIRROR EXISTS: cairn_check_note_attestation is the gate between a raw-SQL client and a
-- recorded signature. Each structural rule gets one body it refuses (asserting WHICH rule refused
-- it, by message fragment), at the strict door and, where the rule is shared, at the remote door.
-- The part-relative rules (addendum by the part's own writer, co-signature of someone else's
-- signature, re-keyed writer) need enrolled actors and the real door: the Rust suite
-- (crates/cairn-node/tests/note_attestation_floor.rs) drives those through seal -> sign -> submit.
--
-- Tests, in order (one DO block or SELECT group each):
--   registration_and_the_type_is_not_threadless
--   a_student_may_release_a_key_for_clinical_signing   (the "(student)" label needs no vault change)
--   the_gesture_timing_kind_note_sign_off_is_accepted
--   a_well_formed_vouch_is_structurally_admitted
--   one refusal per structural rule (payload, schema tag, responsibility count, role, signer,
--     target, target_address, act, addendum carries attested)
--   contributors_are_exactly_one_at_the_strict_door_only
--   a_remote_newer_profile_is_carried_not_projected
--   the strict door's part rules that need no actor (not held, not a note, wrong chart, stale
--     address) and its admission of a well-formed attested sign-off
--   a_well_formed_vouch_is_projected, a_malformed_one_is_skipped
--   a_shred_scrubs_the_vouch_row
--
-- Wrapped in a transaction that ROLLS BACK: nothing here commits.

\o /dev/null
BEGIN;

-- The held note the strict-door cases name (inserted below), its chart, and the signer's key.
CREATE FUNCTION pg_temp.note_id() RETURNS text LANGUAGE sql AS $$ SELECT '71717171-0001-7871-8871-717171717171' $$;
CREATE FUNCTION pg_temp.chart() RETURNS text LANGUAGE sql AS $$ SELECT '71717171-0000-7871-8871-717171717171' $$;
CREATE FUNCTION pg_temp.note_ca() RETURNS bytea LANGUAGE sql AS $$
    SELECT '\x1220'::bytea || digest('\x7101'::bytea, 'sha256');
$$;
CREATE FUNCTION pg_temp.kid() RETURNS text LANGUAGE sql AS $$ SELECT repeat('b7', 32) $$;

-- A valid vouch body: a sign-off, attested, naming the held note by id and address; `over` is
-- merged into its payload.
CREATE FUNCTION pg_temp.body(over jsonb DEFAULT '{}') RETURNS jsonb LANGUAGE sql AS $$
    SELECT jsonb_build_object(
        'schema_version', 'clinical.note-attestation/1',
        'patient_id',     pg_temp.chart(),
        'signer_key_id',  pg_temp.kid(),
        'plaintext_twin', 'Vouches for note part ' || pg_temp.note_id() || ': the note signed off (attested).',
        'contributors',   jsonb_build_array(jsonb_build_object(
                              'actor_id', pg_temp.kid(), 'role', 'attested',
                              'responsibility', jsonb_build_object('held_by', pg_temp.kid()))),
        'payload', jsonb_build_object(
            'target',         pg_temp.note_id(),
            'target_address', encode(pg_temp.note_ca(), 'hex'),
            'act',            'sign-off') || over);
$$;
-- The body with its contributors replaced.
CREATE FUNCTION pg_temp.with_contributors(c jsonb) RETURNS jsonb LANGUAGE sql AS $$
    SELECT pg_temp.body() || jsonb_build_object('contributors', c);
$$;
CREATE FUNCTION pg_temp.bearing(kid text, role text) RETURNS jsonb LANGUAGE sql AS $$
    SELECT jsonb_build_object('actor_id', kid, 'role', role, 'responsibility', jsonb_build_object('held_by', kid));
$$;

CREATE FUNCTION pg_temp.accepts(b jsonb) RETURNS void LANGUAGE plpgsql AS $$
BEGIN
    PERFORM cairn_check_note_attestation('clinical.note-attestation.asserted', b);
EXCEPTION WHEN raise_exception THEN
    RAISE EXCEPTION 'FAIL: refused a body it must accept (%): %', SQLERRM, b;
END $$;

-- Refused with a message containing `frag` and in the floor's own words; refusing for a DIFFERENT
-- reason is a failure too (the rule under test never ran).
CREATE FUNCTION pg_temp.refuses(b jsonb, frag text) RETURNS void LANGUAGE plpgsql AS $$
BEGIN
    BEGIN
        PERFORM cairn_check_note_attestation('clinical.note-attestation.asserted', b);
    EXCEPTION WHEN raise_exception THEN
        IF position(frag IN SQLERRM) = 0 THEN
            RAISE EXCEPTION 'FAIL: refused for the wrong reason: % (wanted "%")', SQLERRM, frag;
        END IF;
        IF SQLERRM NOT LIKE 'note attestation: %' THEN
            RAISE EXCEPTION 'FAIL: refusal is not the vouch floor''s own: %', SQLERRM;
        END IF;
        RETURN;
    END;
    RAISE EXCEPTION 'FAIL: accepted a body it must refuse ("%"): %', frag, b;
END $$;

-- registration_and_the_type_is_not_threadless
DO $$
BEGIN
    ASSERT (SELECT mode FROM event_type_class WHERE event_type = 'clinical.note-attestation.asserted') = 'additive',
        'the vouch type is registered additive';
    ASSERT NOT cairn_event_type_has_no_thread('clinical.note-attestation.asserted'),
        'the_type_is_not_threadless: it stays off db/048''s no-thread list, as notes do';
    ASSERT (SELECT check_fn FROM cairn_event_twin_check WHERE event_type = 'clinical.note-attestation.asserted')
           = 'cairn_check_note_attestation', 'the floor is registered';
    ASSERT (SELECT twin_required_msg FROM cairn_event_twin_check WHERE event_type = 'clinical.note-attestation.asserted')
           = 'note attestation requires a non-empty authored twin (§3.13/§3.19)', 'the twin message';
    ASSERT (SELECT heal_safe AND run_order = 20 FROM cairn_projection_apply
             WHERE event_type = 'clinical.note-attestation.asserted' AND apply_fn = 'clinical_note_attestation_apply'),
        'the projection is registered heal-safe, run_order 20';
END $$;

-- a_student_may_release_a_key_for_clinical_signing: the window's new "student" enrolment role
-- needs no vault change, because every role but superuser/admin may release for 'clinical'.
DO $$
BEGIN
    ASSERT cairn_vault_purpose_allowed('student', false, 'clinical'), 'a student signs clinically';
END $$;

-- the_gesture_timing_kind_note_sign_off_is_accepted (db/044's widened CHECK)
INSERT INTO ui_gesture_timing (gesture_kind, size_bucket) VALUES ('note-sign-off', '1-3')
ON CONFLICT DO NOTHING;

-- a_well_formed_vouch_is_structurally_admitted: at the remote door (structure only), each act
-- and role, including a co-signed sign-off and an upper-case hex address.
SET LOCAL cairn.remote_apply = 'on';
SELECT pg_temp.accepts(pg_temp.body());
SELECT pg_temp.accepts(pg_temp.body('{"act":"addendum"}'));
SELECT pg_temp.accepts(pg_temp.with_contributors(jsonb_build_array(pg_temp.bearing(pg_temp.kid(), 'co-signed'))));
SELECT pg_temp.accepts(pg_temp.body(jsonb_build_object('target_address', upper(encode(pg_temp.note_ca(), 'hex')))));

-- The structural rules hold at BOTH doors: each refusal is run at the remote door, then the strict.
CREATE FUNCTION pg_temp.refuses_at_both(b jsonb, frag text) RETURNS void LANGUAGE plpgsql AS $$
BEGIN
    PERFORM set_config('cairn.remote_apply', 'on', true);
    PERFORM pg_temp.refuses(b, frag);
    PERFORM set_config('cairn.remote_apply', '', true);
    PERFORM pg_temp.refuses(b, frag);
    PERFORM set_config('cairn.remote_apply', 'on', true);
END $$;

-- Rule: the payload exists and the schema tag is /1 (a /1 body: the remote door's R8 skip is for
-- other profiles only).
SELECT pg_temp.refuses_at_both(pg_temp.body() - 'payload', 'missing payload');

-- Rule: exactly one contributor carries responsibility, attested or co-signed, and it is the signer.
SELECT pg_temp.refuses_at_both(pg_temp.with_contributors('[]'), 'exactly one contributor');
SELECT pg_temp.refuses_at_both(pg_temp.body() - 'contributors', 'exactly one contributor');
SELECT pg_temp.refuses_at_both(pg_temp.with_contributors('"x"'), 'exactly one contributor');
SELECT pg_temp.refuses_at_both(pg_temp.with_contributors(jsonb_build_array(
    jsonb_build_object('actor_id', pg_temp.kid(), 'role', 'drafted'))), 'exactly one contributor');
SELECT pg_temp.refuses_at_both(pg_temp.with_contributors(jsonb_build_array(
    pg_temp.bearing(pg_temp.kid(), 'attested'), pg_temp.bearing(repeat('c8', 32), 'co-signed'))),
    'exactly one contributor');
SELECT pg_temp.refuses_at_both(pg_temp.with_contributors(jsonb_build_array(pg_temp.bearing(pg_temp.kid(), r))),
    'must be attested or co-signed')
FROM unnest(ARRAY['authored', 'witnessed', 'ordered']) AS r;
SELECT pg_temp.refuses_at_both(pg_temp.with_contributors(jsonb_build_array(pg_temp.bearing(repeat('c8', 32), 'attested'))),
    'must be the event''s signer');

-- Rule: target is a uuid string.
SELECT pg_temp.refuses_at_both(pg_temp.body('{"target":42}'), 'target must be a uuid string');
SELECT pg_temp.refuses_at_both(pg_temp.body('{"target":"not-a-uuid"}'), 'target must be a valid uuid');
SELECT pg_temp.refuses_at_both(pg_temp.body() #- '{payload,target}', 'target must be a uuid string');

-- Rule: target_address is non-empty, even-length hex.
SELECT pg_temp.refuses_at_both(pg_temp.body(jsonb_build_object('target_address', a)), 'target_address must be')
FROM unnest(ARRAY['', 'abc', '12zz', '\x1220', '12 20']) AS a;
SELECT pg_temp.refuses_at_both(pg_temp.body('{"target_address":1220}'), 'target_address must be');
SELECT pg_temp.refuses_at_both(pg_temp.body() #- '{payload,target_address}', 'target_address must be');

-- Rule: act is a closed word, and an addendum carries attested.
SELECT pg_temp.refuses_at_both(pg_temp.body(jsonb_build_object('act', a)), 'act must be')
FROM unnest(ARRAY['signoff', 'Sign-off', 'addendum ', 'co-sign']) AS a;
SELECT pg_temp.refuses_at_both(pg_temp.body() #- '{payload,act}', 'act must be');
SELECT pg_temp.refuses_at_both(
    pg_temp.with_contributors(jsonb_build_array(pg_temp.bearing(pg_temp.kid(), 'co-signed')))
        || jsonb_build_object('payload', pg_temp.body('{"act":"addendum"}') -> 'payload'),
    'an addendum act carries attested');

-- contributors_are_exactly_one_at_the_strict_door_only: an extra contributory entry is tolerated
-- at the remote door (set-union losslessness, ADR-0051) and refused at the strict one.
SET LOCAL cairn.remote_apply = 'on';
SELECT pg_temp.accepts(pg_temp.with_contributors(jsonb_build_array(
    pg_temp.bearing(pg_temp.kid(), 'attested'), jsonb_build_object('actor_id', repeat('d9', 32), 'role', 'recorded'))));

-- a_remote_newer_profile_is_carried_not_projected (ruling R8): at the remote door a /2 body that
-- breaks /1's rules is admitted; the authoring door refuses it for its schema tag.
SELECT pg_temp.accepts(pg_temp.body('{"act":"vouch-later"}') || '{"schema_version":"clinical.note-attestation/2"}');
SET LOCAL cairn.remote_apply = '';
SELECT pg_temp.refuses(pg_temp.body() || '{"schema_version":"clinical.note-attestation/2"}', 'schema_version');
SELECT pg_temp.refuses(pg_temp.with_contributors(jsonb_build_array(
    pg_temp.bearing(pg_temp.kid(), 'attested'), jsonb_build_object('actor_id', repeat('d9', 32), 'role', 'recorded'))),
    'exactly the one vouching human');

-- The strict door's part rules. First the target is not held at all.
SELECT pg_temp.refuses(pg_temp.body(), 'is not a note held on this node');

-- Hold the note (inserted as the owner; the floor is not under test for it). Its profile is /2 so
-- clinical_note_apply carries it unprojected (the vouch floor reads event_log, not clinical_note).
-- A node-signed part: an attested sign-off needs no actor, so these cases need no enrolment.
INSERT INTO event_log (event_id, patient_id, event_type, schema_version, hlc_wall,
                       hlc_counter, node_origin, t_effective, signed_bytes, content_address,
                       body, contributors, signer_key_id, plaintext_twin)
VALUES (pg_temp.note_id()::uuid, pg_temp.chart()::uuid, 'clinical.note.asserted', 'clinical.note/2', 1, 1, 'n1',
        '2026-11-06T09:00:00+10:00', '\x7101'::bytea, pg_temp.note_ca(),
        '{}'::jsonb, jsonb_build_array(jsonb_build_object('actor_id', repeat('a1', 32), 'role', 'recorded')),
        repeat('a1', 32), 'a note');
-- ... and an event that is not a note, on the same chart.
INSERT INTO event_log (event_id, patient_id, event_type, schema_version, hlc_wall,
                       hlc_counter, node_origin, t_effective, signed_bytes, content_address,
                       body, contributors, signer_key_id, plaintext_twin)
VALUES ('71717171-0009-7871-8871-717171717171', pg_temp.chart()::uuid, 'clinical.note-discard.asserted',
        'clinical.note-discard/2', 1, 2, 'n1', '2026-11-06T09:00:00+10:00', '\x7109'::bytea,
        '\x1220'::bytea || digest('\x7109'::bytea, 'sha256'), '{}'::jsonb, '[]'::jsonb, repeat('a1', 32), 'x');

SELECT pg_temp.accepts(pg_temp.body());
SELECT pg_temp.refuses(pg_temp.body(jsonb_build_object(
    'target', '71717171-0009-7871-8871-717171717171',
    'target_address', encode('\x1220'::bytea || digest('\x7109'::bytea, 'sha256'), 'hex'))),
    'is not a note held on this node');
SELECT pg_temp.refuses(pg_temp.body() || '{"patient_id":"71717171-0000-7871-8871-000000000002"}', 'lives on chart');
SELECT pg_temp.refuses(pg_temp.body(jsonb_build_object(
    'target_address', encode('\x1220'::bytea || digest('\x7102'::bytea, 'sha256'), 'hex'))),
    'is not the content address');
-- An address that differs only in case is the same bytes.
SELECT pg_temp.accepts(pg_temp.body(jsonb_build_object('target_address', upper(encode(pg_temp.note_ca(), 'hex')))));
-- A node-signed part: never a co-signature, and never the signer's own draft for an addendum.
SELECT pg_temp.refuses(pg_temp.with_contributors(jsonb_build_array(pg_temp.bearing(pg_temp.kid(), 'co-signed'))),
    'was not signed by a human');
SELECT pg_temp.refuses(pg_temp.body('{"act":"addendum"}'), 'drafted by someone else');

-- The applier, through the real dispatcher: unsealed rows inserted as the owner
-- (cairn_clear_payload reads an unsealed row's body as its clear payload).
INSERT INTO event_log (event_id, patient_id, event_type, schema_version, hlc_wall,
                       hlc_counter, node_origin, t_effective, signed_bytes, content_address,
                       body, contributors, signer_key_id, plaintext_twin)
SELECT id::uuid, pg_temp.chart()::uuid, 'clinical.note-attestation.asserted', v, 3, n, 'n1',
       NULL, sb, '\x1220'::bytea || digest(sb, 'sha256'),
       pg_temp.body() -> 'payload', c, pg_temp.kid(), 'a vouch'
FROM (VALUES ('71717171-0011-7871-8871-717171717171', 'clinical.note-attestation/1', 1, '\x7111'::bytea,
              pg_temp.body() -> 'contributors'),
             ('71717171-0012-7871-8871-717171717171', 'clinical.note-attestation/2', 2, '\x7112'::bytea,
              pg_temp.body() -> 'contributors'),
             -- A /1 body the floor never saw (a sealed vouch admitted remotely without its DEK):
             -- no responsibility-bearing contributor, so nothing to read as a signature.
             ('71717171-0013-7871-8871-717171717171', 'clinical.note-attestation/1', 3, '\x7113'::bytea,
              '[]'::jsonb))
     AS r(id, v, n, sb, c);
DO $$
BEGIN
    ASSERT (SELECT target = pg_temp.note_id()::uuid AND target_address = pg_temp.note_ca()
              AND act = 'sign-off' AND role = 'attested' AND signer_key_id = pg_temp.kid()
              AND patient_id = pg_temp.chart()::uuid AND t_effective IS NULL
              AND content_address = '\x1220'::bytea || digest('\x7111'::bytea, 'sha256')
              FROM clinical_note_attestation WHERE event_id = '71717171-0011-7871-8871-717171717171'),
        'a_well_formed_vouch_is_projected';
    ASSERT NOT EXISTS (SELECT 1 FROM clinical_note_attestation WHERE event_id = '71717171-0012-7871-8871-717171717171'),
        'a_remote_newer_profile_is_carried_not_projected';
    ASSERT NOT EXISTS (SELECT 1 FROM clinical_note_attestation WHERE event_id = '71717171-0013-7871-8871-717171717171'),
        'a_malformed_one_is_skipped, never raised';
END $$;

-- a_shred_scrubs_the_vouch_row (db/037).
SELECT cairn_execute_shred('71717171-0011-7871-8871-717171717171', gen_random_uuid(), 'test');
DO $$
BEGIN
    ASSERT NOT EXISTS (SELECT 1 FROM clinical_note_attestation WHERE event_id = '71717171-0011-7871-8871-717171717171'),
        'a_shred_scrubs_the_vouch_row';
END $$;

ROLLBACK;
\o
