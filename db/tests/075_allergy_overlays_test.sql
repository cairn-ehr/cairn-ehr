\set ON_ERROR_STOP on
\ir _scratch_database_guard.sql
-- db/075 (+ db/074 step 3, in place) — the in-DB mirror for the allergy overlays (allergy slice 2):
-- a CORRECTION (an ordinary `clinical.allergy.asserted` whose `refs` names the line it replaces),
-- a DE-LABEL (`clinical.allergy-challenge.asserted`) and an ENTERED-IN-ERROR withdrawal
-- (`clinical.allergy-retraction.asserted`).
--
-- WHY THIS MIRROR EXISTS: the overlay floors keep db/074's TWO TIERS (ADR-0051, the db/041
-- idiom). A STRUCTURAL rule (a type, a uuid target, a blank tolerated term, a date value that
-- contradicts its precision) refuses at BOTH doors. A CLOSED-VOCABULARY or REFERENCE rule (the
-- schema tag, the date's words and shape, a filled coding, what the target IS and which chart it
-- sits on) refuses at the AUTHORING door only, and the same body is ADMITTED at the remote door
-- (`cairn.remote_apply = 'on'`): a peer may hold a target this node has not received yet, and a
-- newer peer's word must never wedge sync (ADR-0056; the plan's D6). Each rule is pinned at its
-- door(s) by message fragment, so a refusal for the wrong reason fails too. The Rust suite
-- (crates/cairn-node/tests/allergy_overlay_floor.rs) drives the real seal → sign → submit path.
--
-- The floors are called DIRECTLY (as the schema owner) with hand-built bodies; the targets they
-- look up are unsealed event_log rows inserted as the owner. Everything ROLLS BACK.

\o /dev/null
BEGIN;

-- Two charts, and the fixture events the overlays aim at (inserted below).
CREATE FUNCTION pg_temp.chart() RETURNS text LANGUAGE sql AS $$ SELECT '75757575-0000-7575-8575-757575757575' $$;
CREATE FUNCTION pg_temp.other_chart() RETURNS text LANGUAGE sql AS $$ SELECT '75757575-0000-7575-8575-757575757576' $$;
CREATE FUNCTION pg_temp.entry_id() RETURNS text LANGUAGE sql AS $$ SELECT '75757575-0001-7575-8575-757575757575' $$;
CREATE FUNCTION pg_temp.none_id() RETURNS text LANGUAGE sql AS $$ SELECT '75757575-0002-7575-8575-757575757575' $$;
CREATE FUNCTION pg_temp.challenge_id() RETURNS text LANGUAGE sql AS $$ SELECT '75757575-0003-7575-8575-757575757575' $$;
CREATE FUNCTION pg_temp.retraction_id() RETURNS text LANGUAGE sql AS $$ SELECT '75757575-0004-7575-8575-757575757575' $$;
CREATE FUNCTION pg_temp.foreign_entry_id() RETURNS text LANGUAGE sql AS $$ SELECT '75757575-0005-7575-8575-757575757575' $$;

-- Put one unsealed event on a chart (the dispatcher projects it, as the owner).
CREATE FUNCTION pg_temp.put(id text, chart text, t text, v text, payload jsonb) RETURNS void
LANGUAGE sql AS $$
    INSERT INTO event_log (event_id, patient_id, event_type, schema_version, hlc_wall,
                           hlc_counter, node_origin, signed_bytes, content_address,
                           body, contributors, signer_key_id, plaintext_twin)
    VALUES (id::uuid, chart::uuid, t, v, 3, 0, 'n1', convert_to(id, 'UTF8'),
            '\x1220'::bytea || digest(convert_to(id, 'UTF8'), 'sha256'),
            payload, '[]'::jsonb, 'deadbeef', 'mirror fixture');
$$;

-- An entry payload (db/074's), with `over` merged in.
CREATE FUNCTION pg_temp.entry_payload(over jsonb DEFAULT '{}') RETURNS jsonb LANGUAGE sql AS $$
    SELECT jsonb_build_object(
        'allergen_kind', 'drug',
        'substance',     '{"term":"amoxicillin","coding":null}'::jsonb,
        'kind',          'allergy',
        'reaction',      '{"category":"rash","text":null}'::jsonb,
        'onset',         '{"precision":"not-known","value":null}'::jsonb,
        'source',        'seen-documented',
        'refs',          '[]'::jsonb) || over;
$$;

-- A whole body for chart(), its schema tag and payload given.
CREATE FUNCTION pg_temp.body(v text, payload jsonb) RETURNS jsonb LANGUAGE sql AS $$
    SELECT jsonb_build_object('schema_version', v, 'patient_id', pg_temp.chart(),
                              'plaintext_twin', 'mirror twin', 'payload', payload);
$$;

-- A correction of `target` (a /1 entry whose refs names it).
CREATE FUNCTION pg_temp.correction(target text) RETURNS jsonb LANGUAGE sql AS $$
    SELECT pg_temp.body('clinical.allergy/1', pg_temp.entry_payload(jsonb_build_object(
        'refs', jsonb_build_array(jsonb_build_object('rel', 'correction-of', 'event', target)))));
$$;

-- An entry whose refs are exactly `refs`.
CREATE FUNCTION pg_temp.entry_refs(refs jsonb) RETURNS jsonb LANGUAGE sql AS $$
    SELECT pg_temp.body('clinical.allergy/1', pg_temp.entry_payload(jsonb_build_object('refs', refs)));
$$;

-- A valid de-label of the fixture entry, with `over` merged into its payload.
CREATE FUNCTION pg_temp.challenge(over jsonb DEFAULT '{}') RETURNS jsonb LANGUAGE sql AS $$
    SELECT pg_temp.body('clinical.allergy-challenge/1', jsonb_build_object(
        'target',    pg_temp.entry_id(),
        'tolerated', '{"term":"amoxicillin","coding":null}'::jsonb,
        'date',      '{"precision":"not-known","value":null}'::jsonb,
        'text',      NULL) || over);
$$;

-- A de-label dated `p` / `v`.
CREATE FUNCTION pg_temp.dated(p text, v text) RETURNS jsonb LANGUAGE sql AS $$
    SELECT pg_temp.challenge(jsonb_build_object('date', jsonb_build_object('precision', p, 'value', v)));
$$;

-- A valid withdrawal of `target`.
CREATE FUNCTION pg_temp.retraction(target text) RETURNS jsonb LANGUAGE sql AS $$
    SELECT pg_temp.body('clinical.allergy-retraction/1',
                        jsonb_build_object('target', target, 'text', 'wrong patient'));
$$;

-- Run the floor that owns `b` (by its schema tag's family) at the CURRENT door.
CREATE FUNCTION pg_temp.run(b jsonb) RETURNS void LANGUAGE plpgsql AS $$
BEGIN
    IF b ->> 'schema_version' LIKE 'clinical.allergy-challenge/%' THEN
        PERFORM cairn_check_allergy_challenge('clinical.allergy-challenge.asserted', b);
    ELSIF b ->> 'schema_version' LIKE 'clinical.allergy-retraction/%' THEN
        PERFORM cairn_check_allergy_retraction('clinical.allergy-retraction.asserted', b);
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
-- is an allergy floor's own — refusing for a DIFFERENT reason means the rule never ran.
CREATE FUNCTION pg_temp.refuses(b jsonb, frag text) RETURNS void LANGUAGE plpgsql AS $$
BEGIN
    BEGIN
        PERFORM pg_temp.run(b);
    EXCEPTION WHEN raise_exception THEN
        IF position(frag IN SQLERRM) = 0 THEN
            RAISE EXCEPTION 'FAIL: refused for the wrong reason: % (wanted "%")', SQLERRM, frag;
        END IF;
        IF SQLERRM NOT LIKE 'allergy assertion: %' AND SQLERRM NOT LIKE 'allergy challenge: %'
           AND SQLERRM NOT LIKE 'allergy retraction: %' THEN
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

-- A CLOSED-VOCABULARY or REFERENCE rule: refused at the authoring door, admitted remotely.
CREATE FUNCTION pg_temp.authoring_only(b jsonb, frag text) RETURNS void LANGUAGE plpgsql AS $$
BEGIN
    PERFORM pg_temp.door(false);
    PERFORM pg_temp.refuses(b, frag);
    PERFORM pg_temp.door(true);
    PERFORM pg_temp.accepts(b);
    PERFORM pg_temp.door(false);
END $$;

-- The fixtures: on chart(), an entry, a "none known", a de-label and a withdrawal; on the other
-- chart, an entry.
SELECT pg_temp.put(pg_temp.entry_id(), pg_temp.chart(), 'clinical.allergy.asserted',
                   'clinical.allergy/1', pg_temp.entry_payload());
SELECT pg_temp.put(pg_temp.none_id(), pg_temp.chart(), 'clinical.allergy-none.asserted',
                   'clinical.allergy-none/1', '{"scope":"drug"}');
SELECT pg_temp.put(pg_temp.challenge_id(), pg_temp.chart(), 'clinical.allergy-challenge.asserted',
                   'clinical.allergy-challenge/1',
                   pg_temp.dated('month', '2022-03') -> 'payload' || '{"text":"oral challenge"}');
SELECT pg_temp.put(pg_temp.retraction_id(), pg_temp.chart(), 'clinical.allergy-retraction.asserted',
                   'clinical.allergy-retraction/1', pg_temp.retraction(pg_temp.challenge_id()) -> 'payload');
SELECT pg_temp.put(pg_temp.foreign_entry_id(), pg_temp.other_chart(), 'clinical.allergy.asserted',
                   'clinical.allergy/1', pg_temp.entry_payload());

-- 0. Registration: both new types are known, additive, aim at no other author (a nurse may
--    strike a doctor's line, as on paper), keep the conservative sensitivity bound (off db/048's
--    no-thread list), and their floors and projections are wired.
DO $$
DECLARE t text;
BEGIN
    FOREACH t IN ARRAY ARRAY['clinical.allergy-challenge.asserted', 'clinical.allergy-retraction.asserted'] LOOP
        ASSERT (SELECT mode FROM event_type_class WHERE event_type = t) = 'additive',
            t || ' is registered additive';
        ASSERT (SELECT NOT targets_other_author FROM event_type_class WHERE event_type = t),
            t || ' targets no other author';
        ASSERT NOT cairn_event_type_has_no_thread(t),
            t || ' keeps the conservative sensitivity bound (stays off db/048''s no-thread list)';
    END LOOP;
    ASSERT (SELECT check_fn FROM cairn_event_twin_check WHERE event_type = 'clinical.allergy-challenge.asserted')
           = 'cairn_check_allergy_challenge', 'the de-label floor is registered';
    ASSERT (SELECT check_fn FROM cairn_event_twin_check WHERE event_type = 'clinical.allergy-retraction.asserted')
           = 'cairn_check_allergy_retraction', 'the withdrawal floor is registered';
    ASSERT (SELECT heal_safe AND run_order = 20 FROM cairn_projection_apply
             WHERE event_type = 'clinical.allergy-challenge.asserted'
               AND apply_fn = 'clinical_allergy_challenge_apply'),
        'the de-label projection is registered heal-safe, order 20';
    ASSERT (SELECT heal_safe AND run_order = 20 FROM cairn_projection_apply
             WHERE event_type = 'clinical.allergy-retraction.asserted'
               AND apply_fn = 'clinical_allergy_retraction_apply'),
        'the withdrawal projection is registered heal-safe, order 20';
    -- The fixtures projected as signed.
    ASSERT (SELECT target = pg_temp.entry_id()::uuid AND tolerated_term = 'amoxicillin'
                   AND tolerated_coding IS NULL AND date_precision = 'month'
                   AND date_value = '2022-03' AND text = 'oral challenge'
                   AND patient_id = pg_temp.chart()::uuid
              FROM clinical_allergy_challenge WHERE event_id = pg_temp.challenge_id()::uuid),
        'a de-label projects as signed';
    ASSERT (SELECT target = pg_temp.challenge_id()::uuid AND text = 'wrong patient'
              FROM clinical_allergy_retraction WHERE event_id = pg_temp.retraction_id()::uuid),
        'a withdrawal projects as signed';
END $$;

-- the_three_overlay_gesture_kinds_are_accepted (db/044's widened CHECK)
INSERT INTO ui_gesture_timing (gesture_kind, size_bucket) VALUES
    ('allergy-correct', '1-3'), ('allergy-challenge', '1-3'), ('allergy-retract', '1-3')
ON CONFLICT DO NOTHING;

-- 1. Accepted at the authoring door: a correction of an entry; a de-label at every date
--    precision (today included); a withdrawal of an entry, of a none statement, of a de-label.
SELECT pg_temp.door(false);
SELECT pg_temp.accepts(pg_temp.correction(pg_temp.entry_id()));
SELECT pg_temp.accepts(pg_temp.entry_refs('[]'));
SELECT pg_temp.accepts(pg_temp.challenge());
SELECT pg_temp.accepts(pg_temp.dated('day', '2022-03-14'));
SELECT pg_temp.accepts(pg_temp.dated('day', to_char(now() AT TIME ZONE 'UTC', 'YYYY-MM-DD')));
SELECT pg_temp.accepts(pg_temp.dated('month', '2022-03'));
SELECT pg_temp.accepts(pg_temp.dated('year', '2022'));
SELECT pg_temp.accepts(pg_temp.challenge('{"text":"oral challenge, no reaction"}'));
-- A raw-SQL client's blank text is still a string (the builders write null; the floor need not care).
SELECT pg_temp.accepts(pg_temp.challenge('{"text":"  "}'));
SELECT pg_temp.accepts(pg_temp.retraction(pg_temp.entry_id()));
SELECT pg_temp.accepts(pg_temp.retraction(pg_temp.none_id()));
SELECT pg_temp.accepts(pg_temp.retraction(pg_temp.challenge_id()));
SELECT pg_temp.accepts(jsonb_set(pg_temp.retraction(pg_temp.entry_id()), '{payload,text}', 'null'));

-- 2. Refused at the authoring door only (references and closed vocabularies), the same body
--    admitted remotely.
--    The correction ref: exactly one {rel: correction-of, event}, whose event is an entry here.
SELECT pg_temp.authoring_only(pg_temp.entry_refs(jsonb_build_array(
    jsonb_build_object('rel', 'correction-of', 'event', pg_temp.entry_id()),
    jsonb_build_object('rel', 'correction-of', 'event', pg_temp.entry_id()))),
    'refs must be [] or one {rel: correction-of, event}');
SELECT pg_temp.authoring_only(pg_temp.entry_refs(jsonb_build_array(
    jsonb_build_object('rel', 'supersedes', 'event', pg_temp.entry_id()))),
    'refs must be [] or one {rel: correction-of, event}');
SELECT pg_temp.authoring_only(pg_temp.entry_refs(jsonb_build_array(
    jsonb_build_object('rel', 'correction-of', 'event', pg_temp.entry_id(), 'why', 'typo'))),
    'refs must be [] or one {rel: correction-of, event}');
SELECT pg_temp.authoring_only(pg_temp.entry_refs('[{"rel":"correction-of"}]'),
    'refs must be [] or one {rel: correction-of, event}');
SELECT pg_temp.authoring_only(pg_temp.entry_refs('["correction-of"]'),
    'refs must be [] or one {rel: correction-of, event}');
SELECT pg_temp.authoring_only(pg_temp.correction(''), 'refs must be [] or one {rel: correction-of, event}');
SELECT pg_temp.authoring_only(pg_temp.correction('E1'), 'refs must be [] or one {rel: correction-of, event}');
SELECT pg_temp.authoring_only(pg_temp.entry_refs(jsonb_build_array(
    jsonb_build_object('rel', 'correction-of', 'event', 7))),
    'refs must be [] or one {rel: correction-of, event}');
SELECT pg_temp.authoring_only(pg_temp.correction(pg_temp.none_id()), 'is not an allergy entry on this chart');
SELECT pg_temp.authoring_only(pg_temp.correction(pg_temp.challenge_id()), 'is not an allergy entry on this chart');
SELECT pg_temp.authoring_only(pg_temp.correction(pg_temp.foreign_entry_id()), 'is not an allergy entry on this chart');
SELECT pg_temp.authoring_only(pg_temp.correction(gen_random_uuid()::text), 'is not an allergy entry on this chart');
--    A de-label's target is an entry on this chart.
SELECT pg_temp.authoring_only(pg_temp.challenge(jsonb_build_object('target', pg_temp.none_id())),
    'is not an allergy entry on this chart');
SELECT pg_temp.authoring_only(pg_temp.challenge(jsonb_build_object('target', pg_temp.challenge_id())),
    'is not an allergy entry on this chart');
SELECT pg_temp.authoring_only(pg_temp.challenge(jsonb_build_object('target', pg_temp.foreign_entry_id())),
    'is not an allergy entry on this chart');
--    A withdrawal's target is an entry, a none statement or a de-label on this chart; never a
--    withdrawal (D3: a wrong withdrawal is repaired by recording the entry again).
SELECT pg_temp.authoring_only(pg_temp.retraction(pg_temp.retraction_id()), 'a withdrawal is never itself withdrawn');
SELECT pg_temp.authoring_only(pg_temp.retraction(pg_temp.foreign_entry_id()),
    'is not an allergy entry, none statement or de-label on this chart');
SELECT pg_temp.authoring_only(pg_temp.retraction(gen_random_uuid()::text),
    'is not an allergy entry, none statement or de-label on this chart');
--    The de-label's closed vocabularies: its schema tag, its date's words, shape and window,
--    and the coding's system (#1053).
SELECT pg_temp.authoring_only(pg_temp.dated('year',
    (extract(year FROM now() AT TIME ZONE 'UTC' + interval '14 hours')::int + 1)::text),
    'date.value "');
SELECT pg_temp.authoring_only(pg_temp.dated('day',
    to_char(now() AT TIME ZONE 'UTC' + interval '14 hours' + interval '1 day', 'YYYY-MM-DD')), 'future');
SELECT pg_temp.authoring_only(pg_temp.dated('month', '2022-13'), 'date.value "2022-13" is not a valid month');
SELECT pg_temp.authoring_only(pg_temp.dated('year', '0000'), 'date.value "0000" has year 0000');
SELECT pg_temp.authoring_only(pg_temp.dated('season', '2022-winter'), 'date.precision');
SELECT pg_temp.authoring_only(pg_temp.challenge() || '{"schema_version":"clinical.allergy-challenge/2"}',
    'schema_version');
SELECT pg_temp.authoring_only(pg_temp.retraction(pg_temp.entry_id())
    || '{"schema_version":"clinical.allergy-retraction/2"}', 'schema_version');
--    #1053: a moiety coding is admitted at both doors; a drug GROUP is refused at the authoring
--    door only (admitted remotely); a malformed coding is refused at both.
SELECT pg_temp.door(false);
SELECT pg_temp.accepts(pg_temp.challenge('{"tolerated":{"term":"amoxicillin","coding":{"system":"drugref-moiety",
      "code":"01900000-0000-7000-8000-000000000001","display":"amoxicillin"}}}'));
SELECT pg_temp.authoring_only(pg_temp.challenge('{"tolerated":{"term":"amoxicillin","coding":{"system":"drugref-class",
      "code":"01900000-0000-7000-8000-000000000001","display":"penicillins"}}}'),
    'tolerated.coding.system must be drugref-moiety');
SELECT pg_temp.refuses_both(pg_temp.challenge('{"tolerated":{"term":"amoxicillin","coding":{"system":"drugref-moiety",
      "code":"01900000-0000-7000-8000-000000000001"}}}'), 'tolerated.coding');
SELECT pg_temp.door(true);
SELECT pg_temp.accepts(pg_temp.challenge('{"tolerated":{"term":"amoxicillin","coding":{"system":"drugref-clinical-drug",
      "code":"01900000-0000-7000-8000-000000000001","display":"amoxicillin 500 mg"}}}'));
SELECT pg_temp.door(false);
--    db/074's onset messages keep their field name after the in-place split.
SELECT pg_temp.authoring_only(pg_temp.body('clinical.allergy/1', pg_temp.entry_payload(
    '{"onset":{"precision":"month","value":"2019-13"}}')), 'allergy assertion: onset.value "2019-13" is not a valid month');

-- 3. Structural rules: refused at BOTH doors.
SELECT pg_temp.refuses_both(pg_temp.challenge() - 'payload', 'missing payload');
SELECT pg_temp.refuses_both(pg_temp.challenge('{"target":"E1"}'), 'target must be a uuid string');
SELECT pg_temp.refuses_both(pg_temp.challenge('{"target":""}'), 'target must be a uuid string');
SELECT pg_temp.refuses_both(pg_temp.challenge('{"target":7}'), 'target must be a uuid string');
SELECT pg_temp.refuses_both(pg_temp.challenge() #- '{payload,target}', 'target must be a uuid string');
SELECT pg_temp.refuses_both(pg_temp.challenge('{"tolerated":"amoxicillin"}'), 'tolerated');
SELECT pg_temp.refuses_both(pg_temp.challenge('{"tolerated":{"term":"  ","coding":null}}'), 'tolerated.term');
SELECT pg_temp.refuses_both(pg_temp.challenge('{"tolerated":{"term":5,"coding":null}}'), 'tolerated.term');
SELECT pg_temp.refuses_both(pg_temp.challenge('{"tolerated":{"coding":null}}'), 'tolerated.term');
SELECT pg_temp.refuses_both(pg_temp.challenge('{"tolerated":{"term":"amoxicillin"}}'), 'tolerated.coding');
SELECT pg_temp.refuses_both(pg_temp.challenge('{"tolerated":{"term":"amoxicillin","coding":"J01CA04"}}'),
    'tolerated.coding');
SELECT pg_temp.refuses_both(pg_temp.challenge('{"date":"2022"}'), 'date must be an object');
SELECT pg_temp.refuses_both(pg_temp.challenge('{"date":{"value":null}}'), 'date.precision');
SELECT pg_temp.refuses_both(pg_temp.dated('not-known', '2022'), 'date.value must be null exactly when');
SELECT pg_temp.refuses_both(pg_temp.dated('year', NULL), 'date.value must be null exactly when');
SELECT pg_temp.refuses_both(pg_temp.challenge('{"date":{"precision":"year","value":2022}}'),
    'date.value must be null exactly when');
SELECT pg_temp.refuses_both(pg_temp.challenge() #- '{payload,text}', 'text must be present');
SELECT pg_temp.refuses_both(pg_temp.challenge('{"text":3}'), 'text must be present');
SELECT pg_temp.refuses_both(pg_temp.retraction(pg_temp.entry_id()) - 'payload', 'missing payload');
SELECT pg_temp.refuses_both(pg_temp.retraction('not-a-uuid'), 'target must be a uuid string');
SELECT pg_temp.refuses_both(pg_temp.retraction(pg_temp.entry_id()) #- '{payload,target}', 'target must be a uuid string');
SELECT pg_temp.refuses_both(pg_temp.retraction(pg_temp.entry_id()) #- '{payload,text}', 'text must be present');
SELECT pg_temp.refuses_both(jsonb_set(pg_temp.retraction(pg_temp.entry_id()), '{payload,text}', '[]'),
    'text must be present');
--    The correction's refs is still an array at both doors (slice 1's rule).
SELECT pg_temp.refuses_both(pg_temp.entry_refs('{}'), 'refs must be an array');

-- 4. Admitted at the remote door: a newer word in date.precision, and a peer's refs of another
--    rel (D6: a ref the reader cannot parse is ignored, so the entry reads as uncorrected).
SELECT pg_temp.door(true);
SELECT pg_temp.accepts(pg_temp.dated('season', '2022-winter'));
SELECT pg_temp.accepts(pg_temp.entry_refs(jsonb_build_array(
    jsonb_build_object('rel', 'supersedes', 'event', pg_temp.entry_id()))));
SELECT pg_temp.door(false);

-- 5. A shred scrubs a de-label's row and a withdrawal's row (db/037): the tolerated term and the
--    withdrawal's reason are clinical plaintext.
SELECT cairn_execute_shred(pg_temp.challenge_id()::uuid, gen_random_uuid(), 'test');
SELECT cairn_execute_shred(pg_temp.retraction_id()::uuid, gen_random_uuid(), 'test');
DO $$
BEGIN
    ASSERT NOT EXISTS (SELECT 1 FROM clinical_allergy_challenge WHERE event_id = pg_temp.challenge_id()::uuid),
        'a_shred_scrubs_the_delabel_row';
    ASSERT NOT EXISTS (SELECT 1 FROM clinical_allergy_retraction WHERE event_id = pg_temp.retraction_id()::uuid),
        'a_shred_scrubs_the_withdrawal_row';
END $$;

-- 6. Bodies the floor never saw (a sealed peer overlay admitted remotely without its DEK, projected
--    when custody lands) that the read could not represent: none may raise (the heal would fail)
--    and none may be projected (a NULL tolerated term or date precision would fail the whole
--    list's read). Held but unprojected, each is counted unreadable and leaves its target live.
SELECT pg_temp.put(r.id, pg_temp.chart(), r.t, r.v, r.pl)
FROM (VALUES
    ('75757575-0011-7575-8575-757575757575', 'clinical.allergy-challenge.asserted', 'clinical.allergy-challenge/1',
     pg_temp.challenge() -> 'payload' #- '{tolerated,term}'),
    ('75757575-0012-7575-8575-757575757575', 'clinical.allergy-challenge.asserted', 'clinical.allergy-challenge/1',
     pg_temp.challenge() -> 'payload' #- '{date,precision}'),
    ('75757575-0013-7575-8575-757575757575', 'clinical.allergy-challenge.asserted', 'clinical.allergy-challenge/1',
     pg_temp.challenge('{"target":"E1"}') -> 'payload'),
    ('75757575-0014-7575-8575-757575757575', 'clinical.allergy-retraction.asserted', 'clinical.allergy-retraction/1',
     pg_temp.retraction('E1') -> 'payload'),
    ('75757575-0015-7575-8575-757575757575', 'clinical.allergy-retraction.asserted', 'clinical.allergy-retraction/1',
     jsonb_set(pg_temp.retraction(pg_temp.entry_id()) -> 'payload', '{target}', '7')))
  AS r(id, t, v, pl);
DO $$
BEGIN
    ASSERT NOT EXISTS (SELECT 1 FROM clinical_allergy_challenge
                        WHERE event_id IN ('75757575-0011-7575-8575-757575757575',
                                           '75757575-0012-7575-8575-757575757575',
                                           '75757575-0013-7575-8575-757575757575')),
        'a_delabel_the_read_could_not_represent_is_skipped_never_raised';
    ASSERT NOT EXISTS (SELECT 1 FROM clinical_allergy_retraction
                        WHERE event_id IN ('75757575-0014-7575-8575-757575757575',
                                           '75757575-0015-7575-8575-757575757575')),
        'a_withdrawal_whose_target_is_not_a_uuid_is_skipped_never_raised';
END $$;

ROLLBACK;
\o
