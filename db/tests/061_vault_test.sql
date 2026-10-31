\set ON_ERROR_STOP on
\ir _scratch_database_guard.sql
-- db/061 — the in-DB mirror for the clinicians' key vault's PURE checks.
--
-- WHY THIS MIRROR EXISTS: the Rust suite (crates/cairn-node/tests/vault_floor.rs) drives the doors
-- end to end with real sealed blobs and attestation tokens. What it does not pin one by one are the
-- small pure pieces those doors stand on — the staff-ID normalisation, the enrolment address, the
-- back-off schedule — and the table CHECKs and grants a raw-SQL client meets directly. A change to
-- any of them changes every door at once, so each is asserted here at the SQL layer.
--
-- Wrapped in a transaction that ROLLS BACK: nothing here commits.

BEGIN;

-- 1. Staff IDs normalise: " JSmith " is "jsmith".
DO $$
BEGIN
    ASSERT cairn_vault_staff_id(' JSmith ') = 'jsmith', 'staff IDs must normalise to lower(btrim(x))';
    ASSERT cairn_vault_staff_id(NULL) IS NULL, 'a NULL staff ID stays NULL (never a fabricated one)';
END $$;

-- 2. The enrolment address is deterministic, normalises the staff ID, and binds every field.
DO $$
DECLARE
    k   text := repeat('ab', 32);
    a   bytea := cairn_vault_enrol_address('jsmith', repeat('ab', 32), 'clinician');
BEGIN
    ASSERT a = cairn_vault_enrol_address('jsmith', k, 'clinician'), 'the address must be deterministic';
    ASSERT a = cairn_vault_enrol_address(' JSmith ', k, 'clinician'), 'the address normalises the staff ID';
    ASSERT length(a) = 34 AND substring(a FROM 1 FOR 2) = '\x1220'::bytea,
        'the address is a sha2-256 multihash';
    ASSERT a <> cairn_vault_enrol_address('jdoe', k, 'clinician'), 'the staff ID is bound';
    ASSERT a <> cairn_vault_enrol_address('jsmith', repeat('cd', 32), 'clinician'), 'the key is bound';
    ASSERT a <> cairn_vault_enrol_address('jsmith', k, 'superuser'), 'the role is bound';
    ASSERT a = '\x1220'::bytea || sha256(convert_to(
        'cairn-vault-enrol' || E'\n' || 'jsmith' || E'\n' || k || E'\n' || 'clinician', 'UTF8')),
        'the address preimage is the documented one (the Rust twin is built from it)';
END $$;

-- 3. The back-off: failures 1–3 free, then 1, 2, 4 … seconds, capped at 300, never overflowing.
DO $$
BEGIN
    ASSERT cairn_vault_backoff_seconds(1) IS NULL AND cairn_vault_backoff_seconds(3) IS NULL,
        'failures 1-3 are free';
    ASSERT cairn_vault_backoff_seconds(4) = 1, '4th failure: 1 s';
    ASSERT cairn_vault_backoff_seconds(5) = 2, '5th failure: 2 s';
    ASSERT cairn_vault_backoff_seconds(12) = 256, '12th failure: 256 s';
    ASSERT cairn_vault_backoff_seconds(13) = 300, '13th failure: capped at 300 s';
    ASSERT cairn_vault_backoff_seconds(2147483647) = 300, 'a huge count is capped, never an overflow';
END $$;

-- 4. The table CHECKs a raw-SQL writer meets. Each must be the NAMED constraint, so a refusal
--    says which rule failed.
DO $$
DECLARE c text;
BEGIN
    BEGIN
        INSERT INTO vault_member (staff_id, display_name, role, actor_id, signing_key_id, sealed,
                                  contact_email, enrolled_by)
        VALUES ('JSmith', 'J', 'clinician', '\x00', repeat('ab', 32), '\x00', 'j@x', 'console');
        RAISE EXCEPTION 'an un-normalised staff ID was accepted';
    EXCEPTION WHEN check_violation THEN
        GET STACKED DIAGNOSTICS c = CONSTRAINT_NAME;
        ASSERT c = 'vault_member_staff_id_is_normalised', c;
    END;
    BEGIN
        INSERT INTO vault_member (staff_id, display_name, role, actor_id, signing_key_id, sealed,
                                  contact_email, enrolled_by)
        VALUES ('', 'J', 'clinician', '\x00', repeat('ab', 32), '\x00', 'j@x', 'console');
        RAISE EXCEPTION 'an empty staff ID was accepted';
    EXCEPTION WHEN check_violation THEN
        GET STACKED DIAGNOSTICS c = CONSTRAINT_NAME;
        ASSERT c = 'vault_member_staff_id_is_normalised', c;
    END;
    BEGIN
        INSERT INTO vault_member (staff_id, display_name, role, actor_id, signing_key_id, sealed,
                                  enrolled_by)
        VALUES ('jsmith', 'J', 'clinician', '\x00', repeat('ab', 32), '\x00', 'console');
        RAISE EXCEPTION 'a member nobody can reach was accepted';
    EXCEPTION WHEN check_violation THEN
        GET STACKED DIAGNOSTICS c = CONSTRAINT_NAME;
        ASSERT c = 'vault_member_needs_a_contact', c;
    END;
    BEGIN
        INSERT INTO vault_member (staff_id, display_name, role, actor_id, signing_key_id, sealed,
                                  contact_email, enrolled_by)
        VALUES ('jsmith', 'J', 'clinician', '\x00', 'ABCD', '\x00', 'j@x', 'console');
        RAISE EXCEPTION 'a non-hex signing key id was accepted';
    EXCEPTION WHEN check_violation THEN
        GET STACKED DIAGNOSTICS c = CONSTRAINT_NAME;
        ASSERT c = 'vault_member_signing_key_id_is_hex', c;
    END;
    -- M2b: the enrolment address joins its fields with newlines, so a newline inside a staff ID
    -- or role could make two different requests share one preimage. Control characters are refused.
    BEGIN
        INSERT INTO vault_member (staff_id, display_name, role, actor_id, signing_key_id, sealed,
                                  contact_email, enrolled_by)
        VALUES (E'j\nsmith', 'J', 'clinician', '\x00', repeat('ab', 32), '\x00', 'j@x', 'console');
        RAISE EXCEPTION 'a staff ID with a newline was accepted';
    EXCEPTION WHEN check_violation THEN
        GET STACKED DIAGNOSTICS c = CONSTRAINT_NAME;
        ASSERT c = 'vault_member_has_no_control_characters', c;
    END;
    BEGIN
        INSERT INTO vault_member (staff_id, display_name, role, actor_id, signing_key_id, sealed,
                                  contact_email, enrolled_by)
        VALUES ('jsmith', 'J', E'clinician\nsuperuser', '\x00', repeat('ab', 32), '\x00', 'j@x', 'console');
        RAISE EXCEPTION 'a role with a newline was accepted';
    EXCEPTION WHEN check_violation THEN
        GET STACKED DIAGNOSTICS c = CONSTRAINT_NAME;
        ASSERT c = 'vault_member_has_no_control_characters', c;
    END;
    BEGIN
        INSERT INTO vault_terminal (singleton, unlock_phc) VALUES (false, 'x');
        RAISE EXCEPTION 'a second terminal row was accepted';
    EXCEPTION WHEN check_violation THEN
        NULL;
    END;
END $$;

-- 5. The grants: the runtime role reads nothing and reaches only its doors.
DO $$
BEGIN
    ASSERT NOT has_table_privilege('cairn_agent', 'vault_member', 'SELECT'),
        'cairn_agent must never read vault_member (the wrapped keys)';
    ASSERT NOT has_table_privilege('cairn_agent', 'vault_attempt', 'UPDATE'),
        'cairn_agent must never reset its own back-off';
    ASSERT NOT has_function_privilege('cairn_agent', 'cairn_vault_unseal(bytea, text)', 'EXECUTE'),
        'cairn_vault_release must be the only door to the unthrottled primitive';
    ASSERT NOT has_function_privilege('cairn_agent',
        'cairn_vault_bootstrap(text, text, jsonb, text, bytea, text, text, text)', 'EXECUTE'),
        'bootstrap is the console''s alone';
    ASSERT NOT has_function_privilege('cairn_agent', 'cairn_vault_set_terminal_phc(text)', 'EXECUTE'),
        'setting the terminal passphrase is the console''s alone';
    ASSERT has_function_privilege('cairn_agent', 'cairn_vault_release(text, text, text)', 'EXECUTE'),
        'the window must reach the release door';
END $$;

-- 6. Every member, even one inserted by hand, gets its attempt row and two sequences at birth,
--    and the runtime role can touch neither sequence (it could otherwise reset its own back-off).
DO $$
DECLARE
    v_series uuid;
    f bigint;
    l bigint;
BEGIN
    INSERT INTO vault_member (staff_id, display_name, role, actor_id, signing_key_id, sealed,
                              contact_email, enrolled_by)
    VALUES ('mirror-member', 'M', 'clinician', '\x00', repeat('ef', 32), '\x00', 'm@x', 'console')
    RETURNING attempt_series_id INTO v_series;
    ASSERT EXISTS (SELECT 1 FROM vault_attempt WHERE staff_id = 'mirror-member'),
        'a new member must get its attempt row';
    ASSERT to_regclass('public.' || cairn_vault_sequence_name(v_series, 'attempts')) IS NOT NULL
       AND to_regclass('public.' || cairn_vault_sequence_name(v_series, 'clock')) IS NOT NULL,
        'a new member must get both attempt sequences';
    ASSERT NOT has_sequence_privilege('cairn_agent',
        'public.' || cairn_vault_sequence_name(v_series, 'clock'), 'UPDATE'),
        'cairn_agent must never setval a member''s attempt clock';
    ASSERT NOT has_sequence_privilege('cairn_agent',
        'public.' || cairn_vault_sequence_name(v_series, 'attempts'), 'UPDATE,USAGE'),
        'cairn_agent must never touch a member''s attempt counter';
    SELECT failures, last_attempt_ms INTO f, l FROM cairn_vault_attempt_state('Mirror-Member ');
    ASSERT f = 0 AND l IS NULL, format('a fresh member has no failures and no last attempt (%s, %s)', f, l);
END $$;

-- 7. M7: the terminal passphrase is stored only as an Argon2id PHC string, never as plaintext.
DO $$
BEGIN
    BEGIN
        PERFORM cairn_vault_set_terminal_phc('hunter2');
        RAISE EXCEPTION 'a non-PHC terminal passphrase was accepted';
    EXCEPTION WHEN raise_exception THEN
        ASSERT SQLERRM LIKE 'cairn_vault_set_terminal_phc:%', SQLERRM;
        ASSERT SQLERRM NOT LIKE '%hunter2%', 'the refusal must not echo the value';
    END;
    PERFORM cairn_vault_set_terminal_phc('$argon2id$v=19$m=19456,t=2,p=1$AAAA$BBBB');
    ASSERT cairn_vault_terminal_phc() LIKE '$argon2id$%', 'a PHC string is accepted';
END $$;

-- 8. An unknown staff ID is an answer, not an error.
DO $$
DECLARE o text;
BEGIN
    SELECT outcome INTO o FROM cairn_vault_release('nobody-here', 'x', 'clinical');
    ASSERT o = 'unknown', o;
END $$;

ROLLBACK;
