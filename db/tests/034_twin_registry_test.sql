-- #173 — twin-check registry mechanism, SQL mirror of crates/cairn-node/tests/twin_registry.rs.
-- Run after the schema is loaded. Uses a transaction that ROLLBACKs so it leaves no residue.
BEGIN;

-- 1. Fail-closed: a bogus check_fn is refused at registration.
DO $$
BEGIN
    INSERT INTO cairn_event_twin_check (event_type, check_fn, twin_required_msg)
        VALUES ('test.bogus', 'cairn_check_nope', 'x');
    RAISE EXCEPTION 'FAIL: bogus check_fn was accepted';
EXCEPTION WHEN others THEN
    IF position('does not exist' in SQLERRM) = 0 THEN
        RAISE EXCEPTION 'FAIL: wrong error: %', SQLERRM;
    END IF;
END $$;

-- 2. The registry carries the full 15-row mapping.
DO $$
DECLARE n int;
BEGIN
    SELECT count(*) INTO n FROM cairn_event_twin_check;
    IF n <> 15 THEN RAISE EXCEPTION 'FAIL: expected 15 twin-check rows, got %', n; END IF;
END $$;

-- 3. Dispatch runs the registered check: a self-link raises via the dispatcher.
DO $$
BEGIN
    PERFORM cairn_event_twin('identity.link.asserted', jsonb_build_object(
        'schema_version','identity.link/1',
        'patient_id','00000000-0000-0000-0000-000000000001',
        'plaintext_twin','linked',
        'payload', jsonb_build_object(
            'subject_a','00000000-0000-0000-0000-0000000000aa',
            'subject_b','00000000-0000-0000-0000-0000000000aa',
            'provenance','test')));
    RAISE EXCEPTION 'FAIL: self-link was not refused';
EXCEPTION WHEN others THEN
    IF position('FAIL:' in SQLERRM) = 1 THEN RAISE; END IF;  -- re-raise our own failure
    -- The raise must come from the dispatched link check, not a spurious error (e.g. a
    -- broken/missing dispatcher would raise "function does not exist" — which is NOT proof
    -- that the check ran). Assert the caught message names the link check, so this block
    -- cannot false-green on any-old-error the way a bare `WHEN others` swallow would.
    IF position('link assertion' in SQLERRM) = 0 THEN
        RAISE EXCEPTION 'FAIL: dispatch did not reach the link check; got: %', SQLERRM;
    END IF;
END $$;

ROLLBACK;
