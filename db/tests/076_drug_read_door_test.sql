\set ON_ERROR_STOP on
\ir _scratch_database_guard.sql
-- db/076 (+ db/074, db/041, db/042 in place) — the in-DB mirror for allergy slice 3's database
-- half: the drug read door (the ONE place Cairn reads drugref, ADR-0098 M1) and the floor that
-- opens the allergy entry's reserved coding slot.
--
-- WHY THIS MIRROR EXISTS. drugref is optional, so the door has two lives: on a node without it
-- (or with one it cannot read, or one whose allergy_check has another shape) every door answers
-- "nothing", with no error — the medication list must still draw; on a node with it, the door hands each (allergen, medication) pair to
-- drugref.allergy_check and returns drugref's answers unchanged (M4). Both lives are pinned here,
-- rule by rule. The Rust mirror (crates/cairn-node/tests/drug_read_door.rs) runs the same sections
-- through tokio-postgres and the real seal → sign → submit pipeline.
--
-- THE STUB. drugref is simulated by crates/cairn-node/tests/fixtures/drug_stub.sql: the three
-- tables the door reads, and an allergy_check of drugref's exact shape answering from canned rows.
-- It is loaded ONLY inside a transaction that ends in ROLLBACK — one per section — so no other
-- mirror ever sees a `drugref` schema. Section 0 runs first, with none.
--
-- A failure is an ERROR on stderr (ON_ERROR_STOP); result tables go to /dev/null.

\o /dev/null

-- 0. No drugref: every door answers nothing, and only the window may call them.
BEGIN;
DO $$
DECLARE
    f text;
BEGIN
    ASSERT to_regnamespace('drugref') IS NULL, 'this mirror needs a database with no drugref schema';
    ASSERT cairn_drug_available() IS FALSE, 'no drugref schema: not available';
    ASSERT NOT EXISTS (SELECT 1 FROM cairn_drug_search('amoxicillin', 20)), 'no drugref: no options';
    ASSERT NOT EXISTS (SELECT 1 FROM cairn_drug_facts(ARRAY[gen_random_uuid()], ARRAY[gen_random_uuid()])),
        'no drugref: no facts';
    ASSERT NOT EXISTS (SELECT 1 FROM cairn_drug_group_status(ARRAY[gen_random_uuid()])),
        'no drugref: no group status';
    -- The doors are the window's alone (REVOKE … FROM PUBLIC, GRANT … TO cairn_window).
    FOREACH f IN ARRAY ARRAY['cairn_drug_available()', 'cairn_drug_search(text, integer)',
                             'cairn_drug_facts(uuid[], uuid[])', 'cairn_drug_group_status(uuid[])'] LOOP
        ASSERT NOT has_function_privilege('public', f, 'EXECUTE'), format('PUBLIC must not execute %s', f);
        ASSERT has_function_privilege('cairn_window', f, 'EXECUTE'), format('the window must execute %s', f);
    END LOOP;
    -- Each is a definer door with the pinned path (pg_temp last, #426), so the window reads
    -- drugref through it and never directly.
    ASSERT (SELECT count(*) = 4 AND bool_and(p.prosecdef AND p.proconfig = ARRAY['search_path=pg_catalog, pg_temp'])
              FROM pg_proc p
             WHERE p.proname IN ('cairn_drug_available', 'cairn_drug_search', 'cairn_drug_facts',
                                 'cairn_drug_group_status')),
        'every door is SECURITY DEFINER with search_path = pg_catalog, pg_temp';
    -- The facts door's shape is drugref's answer, not Cairn's fold (M4). Pinned so a replay over
    -- the older shape is seen to converge.
    ASSERT pg_get_function_result('cairn_drug_facts(uuid[], uuid[])'::regprocedure)
           = 'TABLE(allergen uuid, med uuid, outcome text, match_kind text, group_name text)',
        'the facts door has the delegated shape';
END $$;
ROLLBACK;

-- 1. search_text_is_literal_and_short_text_finds_nothing (Review Focus 1), and M5's group filter.
BEGIN;
\ir ../../crates/cairn-node/tests/fixtures/drug_stub.sql
-- The options for `t`, as "system:display" in the door's own order.
CREATE FUNCTION pg_temp.options(t text, n int DEFAULT 20) RETURNS text[] LANGUAGE sql AS $$
    SELECT coalesce(array_agg(s.system || ':' || s.display), '{}') FROM cairn_drug_search(t, n) s;
$$;
DO $$
BEGIN
    ASSERT cairn_drug_available(), 'the stub is a readable drugref';
    -- Moieties match from the start of their name, or of a word in it.
    ASSERT pg_temp.options('amox') = ARRAY['drugref-moiety:amoxicillin'],
        format('"amox" finds amoxicillin: %s', pg_temp.options('amox'));
    ASSERT pg_temp.options('AMOX') = pg_temp.options('amox'), 'case does not matter';
    ASSERT pg_temp.options('  amox  ') = pg_temp.options('amox'), 'the text is trimmed';
    ASSERT pg_temp.options('moxi') = '{}', 'the middle of a drug name does not';
    -- Groups are drugref's allergy groups only (M5), matched anywhere in the name; a MED-RT class
    -- is never offered. The code is the class_uuid, the display its name (M2).
    ASSERT pg_temp.options('penicillin') = ARRAY['drugref-class:Penicillins'],
        format('"penicillin" finds the allergy group, never the EPC class: %s', pg_temp.options('penicillin'));
    ASSERT pg_temp.options('cillins') = ARRAY['drugref-class:Penicillins'], 'a group matches mid-name';
    ASSERT pg_temp.options('antibacterial') = '{}', 'a MED-RT class is never offered (M5)';
    ASSERT (SELECT code FROM cairn_drug_search('penicillins', 20))
           = (SELECT class_uuid::text FROM drugref.substance_class WHERE class_name = 'Penicillins'),
        'a group is coded by its class_uuid';
    -- Fewer than three characters: nothing, never the whole table.
    ASSERT pg_temp.options('am') = '{}', 'two letters find nothing';
    ASSERT pg_temp.options('  am ') = '{}', 'two letters padded with spaces find nothing';
    ASSERT pg_temp.options('') = '{}' AND pg_temp.options(NULL) = '{}', 'no text finds nothing';
    -- LIKE's metacharacters match literally.
    ASSERT pg_temp.options('am%x') = '{}', '% is a literal';
    ASSERT pg_temp.options('%%%') = '{}', '%%% is three literal percent signs, not "everything"';
    ASSERT pg_temp.options('amo_') = '{}', '_ is a literal';
    ASSERT pg_temp.options('___') = '{}', '___ is three literal underscores, not "every name"';
    ASSERT pg_temp.options('amo\') = '{}', 'a trailing backslash is a literal and raises nothing';
    ASSERT pg_temp.options('pen_') = '{}', '_ is a literal in a group name too';
END $$;
-- The limit is capped at 20, and the order is drugs first, then shorter names, then byte order:
-- a group named "zzlimit" is shorter than every drug, so it sorts first if the tiers were lost.
INSERT INTO drugref.substance_moiety (moiety_uuid, display_name)
SELECT md5('stub limit ' || i)::uuid, 'zzlimit ' || i FROM generate_series(1, 25) i;
INSERT INTO drugref.substance_class VALUES (md5('stub limit group')::uuid, 'zzlimit', 'AG');
INSERT INTO drugref.allergy_group VALUES (md5('stub limit group')::uuid, 'structural-seed');
DO $$
BEGIN
    ASSERT cardinality(pg_temp.options('zzlimit', 100)) = 20, 'the limit is capped at 20';
    ASSERT pg_temp.options('zzlimit', 5) = ARRAY['drugref-moiety:zzlimit 1', 'drugref-moiety:zzlimit 2',
                                                 'drugref-moiety:zzlimit 3', 'drugref-moiety:zzlimit 4',
                                                 'drugref-moiety:zzlimit 5'],
        format('drugs first, then length, then byte order: %s', pg_temp.options('zzlimit', 5));
    ASSERT cardinality(pg_temp.options('zzlimit', NULL)) = 20, 'no limit means the cap';
    ASSERT cardinality(pg_temp.options('zzlimit', -1)) = 0, 'a negative limit asks for nothing, and raises nothing';
END $$;
ROLLBACK;

-- 2. Facts: every row drugref.allergy_check gives for every pair, and nothing else (M4); and the
--    groups' review status (M7).
BEGIN;
\ir ../../crates/cairn-node/tests/fixtures/drug_stub.sql
CREATE FUNCTION pg_temp.id(n text) RETURNS uuid LANGUAGE sql AS $$
    SELECT moiety_uuid FROM drugref.substance_moiety WHERE display_name = n
    UNION ALL SELECT class_uuid FROM drugref.substance_class WHERE class_name = n;
$$;
CREATE FUNCTION pg_temp.name(u uuid) RETURNS text LANGUAGE sql AS $$
    SELECT display_name FROM drugref.substance_moiety WHERE moiety_uuid = u
    UNION ALL SELECT class_name FROM drugref.substance_class WHERE class_uuid = u;
$$;
DO $$
DECLARE
    got text[];
BEGIN
    SELECT array_agg(l ORDER BY l COLLATE "C") INTO got
      FROM (SELECT concat_ws(' / ', pg_temp.name(f.allergen), pg_temp.name(f.med), f.outcome,
                             f.match_kind, f.group_name) AS l
              FROM cairn_drug_facts(
          ARRAY[pg_temp.id('Penicillins'), pg_temp.id('flucloxacillin'), pg_temp.id('amoxicillin'),
                pg_temp.id('metformin')],
          ARRAY[pg_temp.id('amoxicillin'), pg_temp.id('perindopril')]) f) lines;
    -- The stub's nine rows for the eight pairs, exactly: two paths for amoxicillin against itself,
    -- and a no-match or unplaced row passed through as drugref gave it.
    ASSERT got = ARRAY[
        'Penicillins / amoxicillin / match / same-group / Penicillins',
        'Penicillins / perindopril / no-match',
        'amoxicillin / amoxicillin / match / same-drug',
        'amoxicillin / amoxicillin / match / same-group / Penicillins',
        'amoxicillin / perindopril / no-match',
        'flucloxacillin / amoxicillin / match / same-group / Penicillins',
        'flucloxacillin / perindopril / no-match',
        'metformin / amoxicillin / unplaced',
        'metformin / perindopril / unplaced'], format('the facts: %s', got);
    ASSERT NOT EXISTS (SELECT 1 FROM cairn_drug_facts(NULL, NULL)) AND
           NOT EXISTS (SELECT 1 FROM cairn_drug_facts('{}', ARRAY[pg_temp.id('amoxicillin')])) AND
           NOT EXISTS (SELECT 1 FROM cairn_drug_facts(ARRAY[NULL::uuid], ARRAY[pg_temp.id('amoxicillin')])),
        'no allergens: no facts';
    -- Only allergy groups have a status; a MED-RT class or an unknown code has none.
    ASSERT (SELECT array_agg(pg_temp.name(s.group_code) || ': ' || s.review_status)
              FROM cairn_drug_group_status(ARRAY[pg_temp.id('Penicillins'),
                                                 pg_temp.id('Penicillin-class Antibacterial [EPC]'),
                                                 gen_random_uuid()]) s)
           = ARRAY['Penicillins: structural-seed'], 'the group status';
END $$;
ROLLBACK;

-- 3. an_unreadable_drugref_is_unavailable_not_a_fault (Review Focus 3; ruling P1). The test
--    database's function owner is a superuser, for whom every privilege check is true — so the
--    doors are handed, inside this rolled-back transaction, to a throwaway non-superuser role
--    that can read some of drugref but not all. No shared role's grants are touched.
BEGIN;
\ir ../../crates/cairn-node/tests/fixtures/drug_stub.sql
CREATE FUNCTION pg_temp.answers_nothing(why text) RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    pen uuid := (SELECT class_uuid FROM drugref.substance_class WHERE class_name = 'Penicillins');
    amox uuid := (SELECT moiety_uuid FROM drugref.substance_moiety WHERE display_name = 'amoxicillin');
BEGIN
    ASSERT cairn_drug_available() IS FALSE, why || ': not available';
    ASSERT NOT EXISTS (SELECT 1 FROM cairn_drug_search('amox', 20)), why || ': no options';
    ASSERT NOT EXISTS (SELECT 1 FROM cairn_drug_facts(ARRAY[pen], ARRAY[amox])), why || ': no facts';
    ASSERT NOT EXISTS (SELECT 1 FROM cairn_drug_group_status(ARRAY[pen])), why || ': no group status';
END $$;
DO $$
DECLARE
    r text := 'cairn_drug_probe_' || pg_backend_pid();  -- unique on the cluster, gone at ROLLBACK
    f text;
BEGIN
    EXECUTE format('CREATE ROLE %I NOLOGIN', r);
    FOREACH f IN ARRAY ARRAY['cairn_drug_available()', 'cairn_drug_search(text, integer)',
                             'cairn_drug_facts(uuid[], uuid[])', 'cairn_drug_group_status(uuid[])'] LOOP
        EXECUTE format('ALTER FUNCTION %s OWNER TO %I', f, r);
    END LOOP;
    EXECUTE format('GRANT USAGE ON SCHEMA drugref TO %I', r);
    EXECUTE format('GRANT SELECT ON ALL TABLES IN SCHEMA drugref TO %I', r);
    -- A function is executable by PUBLIC unless revoked; revoke so the probe's own grant decides.
    REVOKE EXECUTE ON FUNCTION drugref.allergy_check(uuid, uuid) FROM PUBLIC;
    EXECUTE format('GRANT EXECUTE ON FUNCTION drugref.allergy_check(uuid, uuid) TO %I', r);
    -- The control: with everything readable, the probe-owned doors answer.
    ASSERT cairn_drug_available(), 'the probe owner can read all of it';
    ASSERT (SELECT count(*) FROM cairn_drug_search('amox', 20)) = 1, 'and the search answers';
    ASSERT (SELECT count(*) FROM cairn_drug_group_status(
                ARRAY[(SELECT class_uuid FROM drugref.substance_class WHERE class_name = 'Penicillins')])) = 1,
        'and the group status answers';

    -- The check not executable: unavailable, and every door answers nothing, with no error.
    EXECUTE format('REVOKE EXECUTE ON FUNCTION drugref.allergy_check(uuid, uuid) FROM %I', r);
    PERFORM pg_temp.answers_nothing('allergy_check not executable');
    EXECUTE format('GRANT EXECUTE ON FUNCTION drugref.allergy_check(uuid, uuid) TO %I', r);

    -- The groups unreadable: the same.
    EXECUTE format('REVOKE SELECT ON drugref.allergy_group FROM %I', r);
    PERFORM pg_temp.answers_nothing('allergy_group unreadable');
    EXECUTE format('GRANT SELECT ON drugref.allergy_group TO %I', r);

    -- The schema itself unreadable (no USAGE): the same.
    EXECUTE format('REVOKE USAGE ON SCHEMA drugref FROM %I', r);
    PERFORM pg_temp.answers_nothing('an unusable schema');
END $$;
ROLLBACK;

-- 3b. a_drugref_without_allergy_check_is_unavailable (Review Focus 3): a drugref from before
--     allergy groups — its tables are there, the check is not — is unavailable, not a fault. So is
--     one without the allergy_group table.
BEGIN;
\ir ../../crates/cairn-node/tests/fixtures/drug_stub.sql
DROP FUNCTION drugref.allergy_check(uuid, uuid);
DO $$
BEGIN
    ASSERT cairn_drug_available() IS FALSE, 'no allergy_check: not available';
    ASSERT NOT EXISTS (SELECT 1 FROM cairn_drug_search('amox', 20)), 'no allergy_check: no options';
    ASSERT NOT EXISTS (SELECT 1 FROM cairn_drug_facts(ARRAY[gen_random_uuid()], ARRAY[gen_random_uuid()])),
        'no allergy_check: no facts';
END $$;
DROP TABLE drugref.allergy_group;
DO $$
BEGIN
    ASSERT cairn_drug_available() IS FALSE, 'no allergy_group: not available';
    ASSERT NOT EXISTS (SELECT 1 FROM cairn_drug_search('penicillin', 20)), 'no allergy_group: no options';
    ASSERT NOT EXISTS (SELECT 1 FROM cairn_drug_group_status(ARRAY[gen_random_uuid()])),
        'no allergy_group: no group status';
END $$;
ROLLBACK;

-- 3c. a_reshaped_allergy_check_is_unavailable (ruling T2R-b): an allergy_check(uuid, uuid) whose
--     RETURNS TABLE is not drugref db/068's would make the facts door raise on its columns, so the
--     door reads it as unavailable instead — false, zero rows, no error.
BEGIN;
\ir ../../crates/cairn-node/tests/fixtures/drug_stub.sql
DO $$
BEGIN
    ASSERT cairn_drug_available(), 'control: the stub has db/068''s shape';
END $$;
DROP FUNCTION drugref.allergy_check(uuid, uuid);
CREATE FUNCTION drugref.allergy_check(p_prescribed uuid, p_allergen uuid)
RETURNS TABLE (verdict text, group_name text) LANGUAGE sql STABLE AS $$ SELECT 'match', NULL::text $$;
DO $$
DECLARE
    pen uuid := (SELECT class_uuid FROM drugref.substance_class WHERE class_name = 'Penicillins');
    amox uuid := (SELECT moiety_uuid FROM drugref.substance_moiety WHERE display_name = 'amoxicillin');
BEGIN
    ASSERT cairn_drug_available() IS FALSE, 'a reshaped allergy_check: not available';
    ASSERT NOT EXISTS (SELECT 1 FROM cairn_drug_search('amox', 20)), 'reshaped: no options';
    ASSERT NOT EXISTS (SELECT 1 FROM cairn_drug_facts(ARRAY[pen], ARRAY[amox])), 'reshaped: no facts';
    ASSERT NOT EXISTS (SELECT 1 FROM cairn_drug_group_status(ARRAY[pen])), 'reshaped: no group status';
END $$;
ROLLBACK;

-- 4. The floor: the coding slot opens for drug entries (D1), and a medication is never a group (D3).
BEGIN;
CREATE FUNCTION pg_temp.door(remote boolean) RETURNS void LANGUAGE sql AS $$
    SELECT set_config('cairn.remote_apply', CASE WHEN remote THEN 'on' ELSE '' END, true);
$$;
-- `stmt` runs at the current door without a refusal, or this test fails naming it.
CREATE FUNCTION pg_temp.admitted(stmt text) RETURNS void LANGUAGE plpgsql AS $$
BEGIN
    EXECUTE stmt;
EXCEPTION WHEN raise_exception THEN
    RAISE EXCEPTION 'FAIL: refused (%) at remote_apply=%: %', SQLERRM, current_setting('cairn.remote_apply', true), stmt;
END $$;
-- `stmt` is refused at the current door, with a message containing `frag`.
CREATE FUNCTION pg_temp.refused(stmt text, frag text) RETURNS void LANGUAGE plpgsql AS $$
BEGIN
    BEGIN
        EXECUTE stmt;
    EXCEPTION WHEN raise_exception THEN
        IF position(frag IN SQLERRM) = 0 THEN
            RAISE EXCEPTION 'FAIL: refused for the wrong reason: % (wanted "%")', SQLERRM, frag;
        END IF;
        RETURN;
    END;
    RAISE EXCEPTION 'FAIL: admitted (wanted "%") at remote_apply=%: %', frag, current_setting('cairn.remote_apply', true), stmt;
END $$;
-- Refused at the authoring door, the same statement admitted at the remote door (ADR-0051).
CREATE FUNCTION pg_temp.authoring_only(stmt text, frag text) RETURNS void LANGUAGE plpgsql AS $$
BEGIN
    PERFORM pg_temp.door(false);
    PERFORM pg_temp.refused(stmt, frag);
    PERFORM pg_temp.door(true);
    PERFORM pg_temp.admitted(stmt);
    PERFORM pg_temp.door(false);
END $$;
-- A coding object; `code` defaults to a canonical uuid computed from the system's name.
CREATE FUNCTION pg_temp.coding(sys text, code text DEFAULT NULL) RETURNS jsonb LANGUAGE sql AS $$
    SELECT jsonb_build_object('system', sys, 'code', coalesce(code, md5(sys)::uuid::text), 'display', 'amoxicillin');
$$;
-- The allergy floor called on an entry of `kind` carrying `coding`.
CREATE FUNCTION pg_temp.allergy(kind text, coding jsonb) RETURNS text LANGUAGE sql AS $$
    SELECT format('SELECT cairn_check_allergy_assertion(''clinical.allergy.asserted'', %L::jsonb)',
        jsonb_build_object(
            'schema_version', 'clinical.allergy/1',
            'plaintext_twin', 'Allergy: amoxicillin → rash',
            'payload', jsonb_build_object(
                'allergen_kind', kind,
                'substance',     jsonb_build_object('term', 'amoxicillin', 'coding', coding),
                'kind',          'allergy',
                'reaction',      '{"category":"rash","text":null}'::jsonb,
                'onset',         '{"precision":"not-known","value":null}'::jsonb,
                'source',        'seen-documented',
                'refs',          '[]'::jsonb)));
$$;
-- The medication assertion's coding check, and the two coding overlays' floor.
CREATE FUNCTION pg_temp.med(coding jsonb) RETURNS text LANGUAGE sql AS $$
    SELECT format('SELECT cairn_check_medication_coding(%L::jsonb)',
                  jsonb_build_object('substance', jsonb_build_object('term', 'amoxicillin', 'coding', coding)));
$$;
CREATE FUNCTION pg_temp.med_overlay(t text, coding jsonb) RETURNS text LANGUAGE sql AS $$
    SELECT format('SELECT cairn_check_medication_coding_overlay(%L, %L::jsonb)', t,
                  jsonb_build_object('payload', jsonb_build_object(
                      'medication_id', md5('a thread')::uuid::text,
                      'corrects',      md5('a coding')::uuid::text,
                      'coding',        coding)));
$$;
SELECT pg_temp.door(false);
-- A drug entry may be coded as one drug or as a group.
SELECT pg_temp.admitted(pg_temp.allergy('drug', pg_temp.coding('drugref-moiety')));
SELECT pg_temp.admitted(pg_temp.allergy('drug', pg_temp.coding('drugref-class')));
SELECT pg_temp.admitted(pg_temp.allergy('drug', NULL));
-- An `other` entry may not (D1), nor may a drug entry be coded any other way (authoring door).
SELECT pg_temp.authoring_only(pg_temp.allergy('other', pg_temp.coding('drugref-moiety')),
    'allergy assertion: substance.coding is for a drug entry only');
SELECT pg_temp.authoring_only(pg_temp.allergy('drug', pg_temp.coding('drugref-product')),
    'allergy assertion: substance.coding.system must be drugref-moiety or drugref-class, got drugref-product');
SELECT pg_temp.authoring_only(pg_temp.allergy('drug', pg_temp.coding('drugref-class', upper(md5('x')::uuid::text))),
    'canonical lowercase-hyphenated uuid');
-- At the remote door a system this node has never heard of is admitted (structure only), and
-- refused at the authoring door by db/041's registry tier.
SELECT pg_temp.authoring_only(pg_temp.allergy('drug', pg_temp.coding('a-newer-peers-system', 'X-1')),
    'unknown coding system');
-- Structure still refuses at both doors.
SELECT pg_temp.door(true);
SELECT pg_temp.refused(pg_temp.allergy('drug', '{"system":"drugref-class","code":"x","display":" "}'),
    'substance.coding.display');
SELECT pg_temp.door(false);
-- A medication is one drug, never a group (D3), at both of its authoring doors.
SELECT pg_temp.admitted(pg_temp.med(pg_temp.coding('drugref-moiety')));
SELECT pg_temp.authoring_only(pg_temp.med(pg_temp.coding('drugref-class')),
    'medication assertion: substance.coding names a drug group (drugref-class), never one drug');
SELECT pg_temp.authoring_only(pg_temp.med_overlay('clinical.medication-coding.asserted', pg_temp.coding('drugref-class')),
    'medication coding: coding names a drug group (drugref-class), never one drug');
SELECT pg_temp.authoring_only(
    pg_temp.med_overlay('clinical.medication-coding-correction.asserted', pg_temp.coding('drugref-class')),
    'medication coding-correction: coding names a drug group (drugref-class), never one drug');
SELECT pg_temp.admitted(pg_temp.med_overlay('clinical.medication-coding.asserted', pg_temp.coding('drugref-moiety')));
-- The registry knows the group token, and says it is for allergens only.
DO $$
BEGIN
    ASSERT (SELECT code_format = 'uuid' AND note LIKE '%allergen coding only%'
              FROM medication_coding_system WHERE system = 'drugref-class'),
        'drugref-class is registered as a uuid system for allergen coding only';
END $$;
-- The applier copies the coding into the projection as the object (an unsealed row inserted as
-- the owner: cairn_clear_payload reads its body as the clear payload; the dispatcher projects it).
INSERT INTO event_log (event_id, patient_id, event_type, schema_version, hlc_wall,
                       hlc_counter, node_origin, signed_bytes, content_address,
                       body, contributors, signer_key_id, plaintext_twin)
SELECT md5('a coded entry')::uuid, gen_random_uuid(), 'clinical.allergy.asserted', 'clinical.allergy/1',
       3, 0, 'n1', '\x7601'::bytea, '\x1220'::bytea || digest('\x7601'::bytea, 'sha256'),
       jsonb_build_object('allergen_kind', 'drug',
                          'substance', jsonb_build_object('term', 'penicillin', 'coding', pg_temp.coding('drugref-class')),
                          'kind', 'allergy', 'reaction', '{"category":"rash","text":null}'::jsonb,
                          'onset', '{"precision":"not-known","value":null}'::jsonb,
                          'source', 'seen-documented', 'refs', '[]'::jsonb),
       '[]'::jsonb, 'deadbeef', 'mirror fixture';
DO $$
BEGIN
    ASSERT (SELECT coding = pg_temp.coding('drugref-class') FROM clinical_allergy
             WHERE event_id = md5('a coded entry')::uuid),
        'the projection holds the signed coding object';
END $$;
ROLLBACK;

\o
