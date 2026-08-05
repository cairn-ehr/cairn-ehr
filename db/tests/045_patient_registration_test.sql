-- db/tests/045_patient_registration_test.sql
-- #344 / ADR-0060 — the SQL mirror of the registration floor (db/045).
--
-- SCOPE. These assertions call `cairn_check_registration_assertion` DIRECTLY, so they cover
-- exactly the part of the contract that is pure structure: the closed class set, the
-- both-directions search rule, and the empty-candidate-list acceptance. Everything that
-- needs a SIGNATURE to exercise — the door admitting a real registration, the twin
-- requirement raised by the registry row, the retained-set projection and the earliest-wins
-- view, the ADR-0053 authorship refusal — lives in
-- crates/cairn-node/tests/patient_registration.rs, which can sign; SQL alone cannot.
--
-- WHY MIRROR AT ALL: the Rust suite self-skips without $CAIRN_TEST_PG, and db/tests/*.sql
-- runs against a throwaway database built from db/*.sql alone. The mirror is what proves the
-- floor is in the MIGRATION rather than in the test harness's idea of it (issue #212).
--
-- Runs inside a transaction that ROLLBACKs, so it leaves no residue — the same discipline as
-- db/tests/034 and db/tests/043. Picked up automatically: scripts/run-db-sql-tests.sh globs
-- db/tests/[0-9]*.sql, so no registration is needed.
BEGIN;

-- A helper-free idiom on purpose: each block builds its own body inline, so a reader sees
-- the exact jsonb that is being refused next to the reason it must be refused.
--
-- Every block re-raises its own 'FAIL:' message unchanged (`position('FAIL:' in SQLERRM) = 1
-- THEN RAISE`) before inspecting the caught text. Without that, a `WHEN others` handler
-- swallows the very assertion failure it was meant to report and the test false-greens.

-- 1. The class is a CLOSED set (§5.3). A fourth class would be a registration that no other
--    rule in the floor applies to — it would slip past the search rules entirely.
DO $$
BEGIN
    PERFORM cairn_check_registration_assertion('identity.registration.asserted', jsonb_build_object(
        'plaintext_twin', 'Patient registered (temporary registration)',
        'payload', jsonb_build_object('class', 'temporary')));
    RAISE EXCEPTION 'FAIL: an unknown registration class was accepted';
EXCEPTION WHEN others THEN
    IF position('FAIL:' in SQLERRM) = 1 THEN RAISE; END IF;
    IF position('unknown registration class' in SQLERRM) = 0 THEN
        RAISE EXCEPTION 'FAIL: wrong refusal for an unknown class: %', SQLERRM;
    END IF;
END $$;

-- 2. A STANDARD registration must carry its search (§5.8). Without it, a duplicate found six
--    months later can never be traced to a failed search vs. a failed human judgement — the
--    two have opposite fixes.
DO $$
BEGIN
    PERFORM cairn_check_registration_assertion('identity.registration.asserted', jsonb_build_object(
        'plaintext_twin', 'Patient registered (standard registration)',
        'payload', jsonb_build_object('class', 'standard')));
    RAISE EXCEPTION 'FAIL: a standard registration with no search was accepted';
EXCEPTION WHEN others THEN
    IF position('FAIL:' in SQLERRM) = 1 THEN RAISE; END IF;
    IF position('standard registration must carry its search' in SQLERRM) = 0 THEN
        RAISE EXCEPTION 'FAIL: wrong refusal for a search-less standard registration: %', SQLERRM;
    END IF;
END $$;

-- 3. The other direction, and the rule most likely to be wrongly relaxed: absence of the
--    search for a NON-standard class is STRUCTURAL, not merely optional. An implementation
--    that only made `search` optional would pass tests 1, 2 and 4 and still let a John Doe
--    carry a search attestation nobody could have made (there is nothing to search WITH on
--    an unconscious patient with no name and no identifier — principle 4).
DO $$
BEGIN
    PERFORM cairn_check_registration_assertion('identity.registration.asserted', jsonb_build_object(
        'plaintext_twin', 'Patient registered (unidentified registration)',
        'payload', jsonb_build_object(
            'class', 'unidentified',
            'basis', 'unconscious ED arrival, no ID',
            'search', jsonb_build_object(
                'query', jsonb_build_object('name_tokens', jsonb_build_array('smith')),
                'displayed', jsonb_build_array(),
                'incomplete', false))));
    RAISE EXCEPTION 'FAIL: an unidentified registration carrying a search was accepted';
EXCEPTION WHEN others THEN
    IF position('FAIL:' in SQLERRM) = 1 THEN RAISE; END IF;
    IF position('a search attestation the registrar could not have made' in SQLERRM) = 0 THEN
        RAISE EXCEPTION 'FAIL: wrong refusal for an unidentified registration with a search: %', SQLERRM;
    END IF;
END $$;

-- 3b. A non-standard registration STATES WHY (review finding I1: this rule shipped with no
--     test that drove it — deleting it from db/045 left every other assertion here green,
--     because block 3 above supplies a VALID basis and is refused for a different reason).
--
--     For `standard` the class IS the explanation, and a mandatory free-text box there would
--     be a required field satisfiable only by fabrication (principle 4). For the
--     non-standard classes the reverse holds: "unconscious ED arrival, no ID" is the only
--     record of why this chart was born outside the normal path, and a John Doe chart with
--     no stated reason is unauditable six months later.
--
--     All three failing shapes are exercised — absent, blank, and non-string — because the
--     rule has three arms and a test that only omitted the key would leave two unproven.
DO $$
DECLARE
    v_payload jsonb;
    v_label   text;
BEGIN
    FOREACH v_label IN ARRAY ARRAY['absent', 'blank', 'non-string'] LOOP
        v_payload := CASE v_label
            WHEN 'absent'     THEN jsonb_build_object('class', 'unidentified')
            WHEN 'blank'      THEN jsonb_build_object('class', 'unidentified', 'basis', '   ')
            ELSE                   jsonb_build_object('class', 'unidentified', 'basis', 42)
        END;
        BEGIN
            PERFORM cairn_check_registration_assertion('identity.registration.asserted',
                jsonb_build_object(
                    'plaintext_twin', 'Patient registered (unidentified registration)',
                    'payload', v_payload));
            RAISE EXCEPTION 'FAIL: a non-standard registration with a % basis was accepted', v_label;
        EXCEPTION WHEN others THEN
            IF position('FAIL:' in SQLERRM) = 1 THEN RAISE; END IF;
            IF position('non-standard registration states why' in SQLERRM) = 0 THEN
                RAISE EXCEPTION 'FAIL: wrong refusal for a % basis: %', v_label, SQLERRM;
            END IF;
        END;
    END LOOP;
END $$;

-- 3c. The non-standard ACCEPT path (review finding I2). Before this block every
--     non-standard assertion in this file expected a REFUSAL, so the §5.4 John Doe birth
--     act — the exact path the search-absence rule exists to protect — was never once
--     shown to be admissible, and `pseudonymous` was never exercised at all. Both members
--     are checked here so a divergence between §5.3's closed set and
--     `RegistrationClass::as_str()` cannot hide in the rarely-used class.
DO $$
DECLARE v_class text;
BEGIN
    FOREACH v_class IN ARRAY ARRAY['unidentified', 'pseudonymous'] LOOP
        BEGIN
            PERFORM cairn_check_registration_assertion('identity.registration.asserted',
                jsonb_build_object(
                    'plaintext_twin', 'Patient registered (' || v_class || ' registration)',
                    'payload', jsonb_build_object(
                        'class', v_class,
                        'basis', 'no ID available at presentation')));
        EXCEPTION WHEN others THEN
            RAISE EXCEPTION 'FAIL: a well-formed % registration must be accepted, got: %',
                v_class, SQLERRM;
        END;
    END LOOP;
END $$;

-- 4. An EMPTY candidate list is ACCEPTED — the normal case for a genuinely new patient: the
--    search ran and correctly found nothing. This is the anti-regression half of rule 3: a
--    future "tightening" of `displayed` into a non-empty requirement would make registering
--    the first patient on a fresh node impossible, and it would pass every refusal test
--    above. Bare PERFORM: any raise at all fails the file under ON_ERROR_STOP.
DO $$
BEGIN
    PERFORM cairn_check_registration_assertion('identity.registration.asserted', jsonb_build_object(
        'plaintext_twin', 'Patient registered (standard registration)',
        'payload', jsonb_build_object(
            'class', 'standard',
            'search', jsonb_build_object(
                'query', jsonb_build_object('name_tokens', jsonb_build_array('smith'),
                                            'birth_date', '1980-01-01'),
                'displayed', jsonb_build_array(),
                'incomplete', false))));
EXCEPTION WHEN others THEN
    RAISE EXCEPTION 'FAIL: an empty candidate list must be accepted (the search ran and found nothing), got: %', SQLERRM;
END $$;

ROLLBACK;
