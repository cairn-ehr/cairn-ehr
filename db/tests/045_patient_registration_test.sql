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
