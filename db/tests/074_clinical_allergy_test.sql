\set ON_ERROR_STOP on
\ir _scratch_database_guard.sql
-- db/074 — the in-DB mirror for the allergy list's two floors (allergy slice 1):
-- cairn_check_allergy_assertion (`clinical.allergy.asserted`) and
-- cairn_check_allergy_none_assertion (`clinical.allergy-none.asserted`).
--
-- WHY THIS MIRROR EXISTS: the floor has TWO TIERS (ADR-0051, the db/041 idiom), and the split
-- is the whole point. A STRUCTURAL rule (a key missing, the wrong JSON type, a blank term) must
-- refuse at BOTH doors — no node can read such a body. A CLOSED-VOCABULARY rule (an enumerated
-- word, an onset's shape, an onset in the future, a filled reserved slot) refuses at the
-- AUTHORING door only, and the SAME body must be ADMITTED at the remote door
-- (`cairn.remote_apply = 'on'`), so a newer peer's word never wedges sync (ADR-0056). Each rule
-- below is pinned at the door(s) it belongs to, by message fragment, so a refusal for the wrong
-- reason fails too. The Rust suite (crates/cairn-node/tests/allergy_floor.rs) drives the real
-- seal → sign → submit → project path; this pins the rules one at a time.
--
-- The floors are called DIRECTLY (as the schema owner) with hand-built bodies. Everything
-- ROLLS BACK; result tables go to /dev/null, and a failure is an ERROR on stderr.

\o /dev/null
BEGIN;

-- A valid entry body, with `over` merged into its payload (a nested override replaces the
-- whole sub-object, so pass it complete).
CREATE FUNCTION pg_temp.entry(over jsonb DEFAULT '{}') RETURNS jsonb LANGUAGE sql AS $$
    SELECT jsonb_build_object(
        'schema_version', 'clinical.allergy/1',
        'plaintext_twin', 'Allergy: penicillin → rash',
        'payload', jsonb_build_object(
            'allergen_kind', 'drug',
            'substance',     '{"term":"penicillin","coding":null}'::jsonb,
            'kind',          'allergy',
            'reaction',      '{"category":"rash","text":null}'::jsonb,
            'onset',         '{"precision":"not-known","value":null}'::jsonb,
            'source',        'seen-documented',
            'refs',          '[]'::jsonb) || over);
$$;

-- A valid "none known" body.
CREATE FUNCTION pg_temp.none_body(over jsonb DEFAULT '{}') RETURNS jsonb LANGUAGE sql AS $$
    SELECT jsonb_build_object(
        'schema_version', 'clinical.allergy-none/1',
        'plaintext_twin', 'No known allergies',
        'payload', jsonb_build_object('scope', 'all') || over);
$$;

-- An onset sub-object, for brevity.
CREATE FUNCTION pg_temp.onset(p text, v text) RETURNS jsonb LANGUAGE sql AS $$
    SELECT jsonb_build_object('onset', jsonb_build_object('precision', p, 'value', v));
$$;

-- Run the floor that owns `b` (by its schema tag's family) at the CURRENT door.
CREATE FUNCTION pg_temp.run(b jsonb) RETURNS void LANGUAGE plpgsql AS $$
BEGIN
    IF b ->> 'schema_version' LIKE 'clinical.allergy-none/%' THEN
        PERFORM cairn_check_allergy_none_assertion('clinical.allergy-none.asserted', b);
    ELSE
        PERFORM cairn_check_allergy_assertion('clinical.allergy.asserted', b);
    END IF;
END $$;

-- Set the door for the rest of this transaction: TRUE = the remote (sync-apply) door.
CREATE FUNCTION pg_temp.door(remote boolean) RETURNS void LANGUAGE sql AS $$
    SELECT set_config('cairn.remote_apply', CASE WHEN remote THEN 'on' ELSE '' END, true);
$$;

-- The floor accepts `b` at the current door, or this test fails naming the body.
CREATE FUNCTION pg_temp.accepts(b jsonb) RETURNS void LANGUAGE plpgsql AS $$
BEGIN
    PERFORM pg_temp.run(b);
EXCEPTION WHEN raise_exception THEN
    RAISE EXCEPTION 'FAIL: refused a body it must accept (%) at remote_apply=%: %',
        SQLERRM, current_setting('cairn.remote_apply', true), b;
END $$;

-- The floor refuses `b` at the current door with a message containing `frag`, and the message
-- is the allergy floor's own — refusing for a DIFFERENT reason means the rule never ran.
CREATE FUNCTION pg_temp.refuses(b jsonb, frag text) RETURNS void LANGUAGE plpgsql AS $$
BEGIN
    BEGIN
        PERFORM pg_temp.run(b);
    EXCEPTION WHEN raise_exception THEN
        IF position(frag IN SQLERRM) = 0 THEN
            RAISE EXCEPTION 'FAIL: refused for the wrong reason: % (wanted "%")', SQLERRM, frag;
        END IF;
        IF SQLERRM NOT LIKE 'allergy assertion: %' AND SQLERRM NOT LIKE 'allergy-none assertion: %' THEN
            RAISE EXCEPTION 'FAIL: refusal is not the allergy floor''s own: %', SQLERRM;
        END IF;
        RETURN;
    END;
    RAISE EXCEPTION 'FAIL: accepted a body it must refuse ("%") at remote_apply=%: %',
        frag, current_setting('cairn.remote_apply', true), b;
END $$;

-- A STRUCTURAL rule: refused at the authoring door AND at the remote door.
CREATE FUNCTION pg_temp.refuses_both(b jsonb, frag text) RETURNS void LANGUAGE plpgsql AS $$
BEGIN
    PERFORM pg_temp.door(false);
    PERFORM pg_temp.refuses(b, frag);
    PERFORM pg_temp.door(true);
    PERFORM pg_temp.refuses(b, frag);
    PERFORM pg_temp.door(false);
END $$;

-- A CLOSED-VOCABULARY rule: refused at the authoring door, the same body ADMITTED remotely.
CREATE FUNCTION pg_temp.authoring_only(b jsonb, frag text) RETURNS void LANGUAGE plpgsql AS $$
BEGIN
    PERFORM pg_temp.door(false);
    PERFORM pg_temp.refuses(b, frag);
    PERFORM pg_temp.door(true);
    PERFORM pg_temp.accepts(b);
    PERFORM pg_temp.door(false);
END $$;

-- 0. Registration: both types are known, additive, keep the conservative sensitivity bound
--    (off db/048's no-thread list), and their floors and projections are wired.
DO $$
DECLARE t text;
BEGIN
    FOREACH t IN ARRAY ARRAY['clinical.allergy.asserted', 'clinical.allergy-none.asserted'] LOOP
        ASSERT (SELECT mode FROM event_type_class WHERE event_type = t) = 'additive',
            t || ' is registered additive';
        ASSERT (SELECT NOT targets_other_author FROM event_type_class WHERE event_type = t),
            t || ' targets no other author';
        ASSERT NOT cairn_event_type_has_no_thread(t),
            t || ' keeps the conservative sensitivity bound (stays off db/048''s no-thread list)';
    END LOOP;
    ASSERT (SELECT check_fn FROM cairn_event_twin_check WHERE event_type = 'clinical.allergy.asserted')
           = 'cairn_check_allergy_assertion', 'the entry floor is registered';
    ASSERT (SELECT check_fn FROM cairn_event_twin_check WHERE event_type = 'clinical.allergy-none.asserted')
           = 'cairn_check_allergy_none_assertion', 'the none floor is registered';
    ASSERT (SELECT heal_safe AND run_order = 20 FROM cairn_projection_apply
             WHERE event_type = 'clinical.allergy.asserted' AND apply_fn = 'clinical_allergy_apply'),
        'the entry projection is registered heal-safe, order 20';
    ASSERT (SELECT heal_safe AND run_order = 20 FROM cairn_projection_apply
             WHERE event_type = 'clinical.allergy-none.asserted' AND apply_fn = 'clinical_allergy_none_apply'),
        'the none projection is registered heal-safe, order 20';
END $$;

-- 1. Accepted at the authoring door: the plain bodies, every precision, a reaction text, and
--    an onset of today (the recording day itself is not the future).
SELECT pg_temp.door(false);
SELECT pg_temp.accepts(pg_temp.entry());
SELECT pg_temp.accepts(pg_temp.entry(pg_temp.onset('day', '2019-02-28')));
SELECT pg_temp.accepts(pg_temp.entry(pg_temp.onset('month', '2019-02')));
SELECT pg_temp.accepts(pg_temp.entry(pg_temp.onset('year', '2019')));
SELECT pg_temp.accepts(pg_temp.entry(pg_temp.onset('day', to_char(now() AT TIME ZONE 'UTC', 'YYYY-MM-DD'))));
-- Decision 7: "today" is today ANYWHERE on earth (UTC+14), so at 06:00 in Brisbane the local day
-- the window accepts is never refused as the future; the day after that is.
SELECT pg_temp.accepts(pg_temp.entry(pg_temp.onset('day',
    to_char(now() AT TIME ZONE 'UTC' + interval '14 hours', 'YYYY-MM-DD'))));
SELECT pg_temp.authoring_only(pg_temp.entry(pg_temp.onset('day',
    to_char(now() AT TIME ZONE 'UTC' + interval '14 hours' + interval '1 day', 'YYYY-MM-DD'))), 'future');
SELECT pg_temp.accepts(pg_temp.entry(
    '{"kind":"adverse-effect","allergen_kind":"other","source":"patient-stated",
      "reaction":{"category":"other","text":"felt dizzy"}}'));
SELECT pg_temp.accepts(pg_temp.none_body());
SELECT pg_temp.accepts(pg_temp.none_body('{"scope":"drug"}'));

-- 2. Structural rules: refused at BOTH doors.
SELECT pg_temp.refuses_both(pg_temp.entry() - 'payload', 'missing payload');
SELECT pg_temp.refuses_both(pg_temp.entry() #- '{payload,allergen_kind}', 'allergen_kind');
SELECT pg_temp.refuses_both(pg_temp.entry('{"allergen_kind":42}'), 'allergen_kind');
SELECT pg_temp.refuses_both(pg_temp.entry('{"substance":"penicillin"}'), 'substance');
SELECT pg_temp.refuses_both(pg_temp.entry('{"substance":{"coding":null}}'), 'substance.term');
SELECT pg_temp.refuses_both(pg_temp.entry('{"substance":{"term":7,"coding":null}}'), 'substance.term');
SELECT pg_temp.refuses_both(pg_temp.entry(jsonb_build_object('substance',
    jsonb_build_object('term', E'  \t\n ', 'coding', NULL))), 'substance.term');
SELECT pg_temp.refuses_both(pg_temp.entry('{"substance":{"term":"penicillin"}}'), 'substance.coding');
SELECT pg_temp.refuses_both(pg_temp.entry('{"substance":{"term":"penicillin","coding":"J01CE"}}'),
                            'substance.coding');
SELECT pg_temp.refuses_both(pg_temp.entry(
    '{"substance":{"term":"penicillin","coding":{"system":"drugref-moiety","code":"x","display":" "}}}'),
    'substance.coding.display');
SELECT pg_temp.refuses_both(pg_temp.entry() #- '{payload,kind}', 'kind');
SELECT pg_temp.refuses_both(pg_temp.entry('{"kind":["allergy"]}'), 'kind');
SELECT pg_temp.refuses_both(pg_temp.entry('{"reaction":"rash"}'), 'reaction');
SELECT pg_temp.refuses_both(pg_temp.entry('{"reaction":{"text":null}}'), 'reaction.category');
SELECT pg_temp.refuses_both(pg_temp.entry('{"reaction":{"category":"rash"}}'), 'reaction.text');
SELECT pg_temp.refuses_both(pg_temp.entry('{"reaction":{"category":"rash","text":5}}'), 'reaction.text');
SELECT pg_temp.refuses_both(pg_temp.entry('{"onset":"2019"}'), 'onset');
SELECT pg_temp.refuses_both(pg_temp.entry('{"onset":{"value":null}}'), 'onset.precision');
SELECT pg_temp.refuses_both(pg_temp.entry('{"onset":{"precision":"year"}}'), 'onset.value');
SELECT pg_temp.refuses_both(pg_temp.entry('{"onset":{"precision":"year","value":2019}}'), 'onset.value');
SELECT pg_temp.refuses_both(pg_temp.entry(pg_temp.onset('year', NULL)), 'onset.value');
SELECT pg_temp.refuses_both(pg_temp.entry(pg_temp.onset('not-known', '2019')), 'onset.value');
SELECT pg_temp.refuses_both(pg_temp.entry() #- '{payload,source}', 'source');
SELECT pg_temp.refuses_both(pg_temp.entry('{"source":true}'), 'source');
SELECT pg_temp.refuses_both(pg_temp.entry() #- '{payload,refs}', 'refs');
SELECT pg_temp.refuses_both(pg_temp.entry('{"refs":{}}'), 'refs');
SELECT pg_temp.refuses_both(pg_temp.none_body() - 'payload', 'missing payload');
SELECT pg_temp.refuses_both(pg_temp.none_body() #- '{payload,scope}', 'scope');
SELECT pg_temp.refuses_both(pg_temp.none_body('{"scope":1}'), 'scope');

-- 3. Closed vocabularies: refused at the authoring door, the same body admitted remotely.
SELECT pg_temp.authoring_only(pg_temp.entry() || '{"schema_version":"clinical.allergy/2"}', 'schema_version');
SELECT pg_temp.authoring_only(pg_temp.entry('{"allergen_kind":"food"}'), 'allergen_kind');
SELECT pg_temp.authoring_only(pg_temp.entry('{"kind":"intolerance"}'), 'kind');
SELECT pg_temp.authoring_only(pg_temp.entry('{"reaction":{"category":"hives","text":null}}'), 'reaction.category');
SELECT pg_temp.authoring_only(pg_temp.entry(pg_temp.onset('season', '2019-winter')), 'onset.precision');
SELECT pg_temp.authoring_only(pg_temp.entry('{"source":"hearsay"}'), 'source');
-- An onset value that does not read as its precision's ISO form, or is no real date.
SELECT pg_temp.authoring_only(pg_temp.entry(pg_temp.onset('month', '2019-13')), 'onset.value');
SELECT pg_temp.authoring_only(pg_temp.entry(pg_temp.onset('day', '2019-02-31')), 'onset.value');
SELECT pg_temp.authoring_only(pg_temp.entry(pg_temp.onset('day', '2019-02')), 'onset.value');
SELECT pg_temp.authoring_only(pg_temp.entry(pg_temp.onset('year', '19')), 'onset.value');
-- No calendar has a year 0000 (day precision is already no real date to pg_input_is_valid).
SELECT pg_temp.authoring_only(pg_temp.entry(pg_temp.onset('year', '0000')), 'year 0000');
SELECT pg_temp.authoring_only(pg_temp.entry(pg_temp.onset('month', '0000-03')), 'year 0000');
SELECT pg_temp.authoring_only(pg_temp.entry(pg_temp.onset('day', '0000-03-14')), 'onset.value');
-- An onset after the recording day: the year after the latest calendar year anywhere on earth.
SELECT pg_temp.authoring_only(pg_temp.entry(pg_temp.onset('year',
    (extract(year FROM now() AT TIME ZONE 'UTC' + interval '14 hours')::int + 1)::text)), 'future');
-- The reserved slots, opened by later slices: a coding (slice 3) and a correction ref (slice 2).
SELECT pg_temp.authoring_only(pg_temp.entry(
    '{"substance":{"term":"amoxicillin","coding":{"system":"drugref-moiety",
      "code":"01900000-0000-7000-8000-000000000001","display":"amoxicillin"}}}'), 'coding must be null');
SELECT pg_temp.authoring_only(pg_temp.entry(
    '{"refs":[{"rel":"correction-of","event":"01900000-0000-7000-8000-000000000002"}]}'), 'refs must be empty');
SELECT pg_temp.authoring_only(pg_temp.none_body('{"scope":"food"}'), 'scope');
SELECT pg_temp.authoring_only(pg_temp.none_body() || '{"schema_version":"clinical.allergy-none/2"}',
                              'schema_version');

-- the_gesture_timing_kinds_allergy_record_and_allergy_none_are_accepted (db/044's widened CHECK)
INSERT INTO ui_gesture_timing (gesture_kind, size_bucket) VALUES ('allergy-record', '1-3')
ON CONFLICT DO NOTHING;
INSERT INTO ui_gesture_timing (gesture_kind, size_bucket) VALUES ('allergy-none', '1-3')
ON CONFLICT DO NOTHING;

-- 4. A peer's newer word is admitted AND projects: the vocabulary columns carry no CHECK.
--    The rows are a NEWER schema_version (/2): the appliers project every version they can read
--    (decision 8, unlike db/064's /1-only filter), so a newer peer's allergy is never withheld.
--    Unsealed rows inserted as the owner (cairn_clear_payload reads an unsealed row's body as
--    its clear payload); the dispatcher runs the appliers.
INSERT INTO event_log (event_id, patient_id, event_type, schema_version, hlc_wall,
                       hlc_counter, node_origin, signed_bytes, content_address,
                       body, contributors, signer_key_id, plaintext_twin)
SELECT id::uuid, gen_random_uuid(), t, v, 3, n, 'n1', sb,
       '\x1220'::bytea || digest(sb, 'sha256'), body, '[]'::jsonb, 'deadbeef', 'mirror fixture'
FROM (VALUES
    ('74747474-0001-7474-8474-747474747474', 'clinical.allergy.asserted', 'clinical.allergy/2', 1,
     '\x7401'::bytea, pg_temp.entry('{"allergen_kind":"food","kind":"intolerance"}') -> 'payload'),
    ('74747474-0002-7474-8474-747474747474', 'clinical.allergy-none.asserted', 'clinical.allergy-none/2', 2,
     '\x7402'::bytea, '{"scope":"food"}'::jsonb)) AS r(id, t, v, n, sb, body);
DO $$
BEGIN
    ASSERT (SELECT allergen_kind = 'food' AND kind = 'intolerance' AND term = 'penicillin'
                   AND coding IS NULL AND onset_value IS NULL AND refs = '[]'::jsonb
              FROM clinical_allergy WHERE event_id = '74747474-0001-7474-8474-747474747474'),
        'a newer peer''s words project as themselves';
    ASSERT (SELECT scope = 'food' FROM clinical_allergy_none
             WHERE event_id = '74747474-0002-7474-8474-747474747474'),
        'a newer peer''s scope projects as itself';
END $$;

ROLLBACK;
\o
