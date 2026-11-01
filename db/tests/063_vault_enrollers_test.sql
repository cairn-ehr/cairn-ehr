\set ON_ERROR_STOP on
\ir _scratch_database_guard.sql
-- db/063 — the in-DB mirror for the vault's slice-2 doors: the PURE pieces they stand on (the
-- four addresses each signed act is bound to, and the role → purpose map of the release door)
-- and the grants a raw-SQL client meets. The Rust suite (crates/cairn-node/tests/vault_enrollers.rs)
-- drives the doors end to end; this pins each formula, so a change to one is a failing line here.
--
-- Wrapped in a transaction that ROLLS BACK: nothing here commits.

BEGIN;

-- 1. The four addresses: sha2-256 multihashes over the documented preimages, staff IDs normalised.
DO $$
DECLARE
    k text := repeat('ab', 32);
BEGIN
    ASSERT cairn_vault_rekey_address(' JSmith ', 'clinician', k) = '\x1220'::bytea || sha256(convert_to(
        'cairn-vault-rekey' || E'\n' || 'jsmith' || E'\n' || 'clinician' || E'\n' || k, 'UTF8')),
        'the re-key preimage';
    ASSERT cairn_vault_rekey_address('jsmith', 'clinician', k) <> cairn_vault_rekey_address('jsmith', 'nurse', k),
        'the re-key binds the role';
    ASSERT to_regprocedure('cairn_vault_rekey_address(text, text)') IS NULL,
        'no stale 2-argument re-key address survives';
    ASSERT cairn_vault_grant_address(' JSmith ', true, 0) = '\x1220'::bytea || sha256(convert_to(
        'cairn-vault-grant' || E'\n' || 'jsmith' || E'\n' || 'grant' || E'\n' || '0', 'UTF8')),
        'the grant preimage';
    ASSERT cairn_vault_grant_address('jsmith', false, 42) = '\x1220'::bytea || sha256(convert_to(
        'cairn-vault-grant' || E'\n' || 'jsmith' || E'\n' || 'withdraw' || E'\n' || '42', 'UTF8')),
        'the withdraw preimage binds the prior state';
    ASSERT cairn_vault_grant_address('jsmith', true, 0) <> cairn_vault_grant_address('jsmith', true, 1),
        'a grant token is good for the state it changes, and only that';
    ASSERT cairn_vault_judge_address(k, true) = '\x1220'::bytea || sha256(convert_to(
        'cairn-vault-judge' || E'\n' || k || E'\n' || 'confirm', 'UTF8')), 'the confirm preimage';
    ASSERT cairn_vault_judge_address(k, false) = '\x1220'::bytea || sha256(convert_to(
        'cairn-vault-judge' || E'\n' || k || E'\n' || 'repudiate', 'UTF8')), 'the repudiate preimage';
    ASSERT cairn_vault_reseal_address('nobody-here', '\x00') IS NULL,
        'no member, no address (nothing to sign over)';
END $$;

-- 2. The reseal address binds the CURRENT stored blob and the new one, by digest only.
DO $$
BEGIN
    INSERT INTO vault_member (staff_id, display_name, role, actor_id, signing_key_id, sealed,
                              contact_email, enrolled_by)
    VALUES ('mirror-reseal', 'M', 'clinician', '\x00', repeat('ef', 32), '\x0102', 'm@x', 'console');
    ASSERT cairn_vault_reseal_address(' Mirror-Reseal ', '\x0304') = '\x1220'::bytea || sha256(convert_to(
        'cairn-vault-reseal' || E'\n' || 'mirror-reseal' || E'\n' || encode(sha256('\x0102'), 'hex')
        || E'\n' || encode(sha256('\x0304'), 'hex'), 'UTF8')), 'the reseal preimage';
END $$;

-- 3. The release door's purpose map (§5.1; defined in db/061, ahead of the door that calls it).
DO $$
BEGIN
    ASSERT cairn_vault_purpose_allowed('superuser', false, 'enrol')
       AND cairn_vault_purpose_allowed('superuser', false, 'reseal')
       AND NOT cairn_vault_purpose_allowed('superuser', true, 'clinical'),
        'a superuser enrols and re-seals, never signs clinically';
    ASSERT cairn_vault_purpose_allowed('admin', false, 'enrol')
       AND cairn_vault_purpose_allowed('admin', false, 'reseal')
       AND NOT cairn_vault_purpose_allowed('admin', true, 'clinical'),
        'an admin is administrative only';
    ASSERT cairn_vault_purpose_allowed('clinician', false, 'clinical')
       AND cairn_vault_purpose_allowed('clinician', false, 'reseal')
       AND NOT cairn_vault_purpose_allowed('clinician', false, 'enrol')
       AND cairn_vault_purpose_allowed('clinician', true, 'enrol'),
        'a clinician enrols only while an enroller';
    ASSERT NOT cairn_vault_purpose_allowed('clinician', true, 'other')
       AND NOT cairn_vault_purpose_allowed('clinician', true, NULL),
        'a purpose outside the vocabulary is refused';
END $$;

-- 4. The grants: the window reaches the doors it calls; the console-only and internal pieces
--    are the owner's alone.
DO $$
DECLARE f text;
BEGIN
    FOREACH f IN ARRAY ARRAY[
        'cairn_vault_rekey(text, text, text, text, text, text, bytea, text, text, bytea)',
        'cairn_vault_set_enroller(text, boolean, text, bytea)',
        'cairn_vault_judge_key(text, boolean, text, bytea)',
        'cairn_vault_reseal(text, bytea, text, bytea)',
        'cairn_vault_reseal_address(text, bytea)',
        'cairn_vault_grant_prior(text)',
        'cairn_vault_is_enroller(text)',
        'cairn_vault_unconfirmed()',
        'cairn_vault_enrollers()',
        'cairn_vault_replaced_notice(text)',
        'cairn_vault_unconfirmed_expires_at(text)'
    ] LOOP
        ASSERT has_function_privilege('cairn_agent', f, 'EXECUTE'), format('the window must reach %s', f);
    END LOOP;
    FOREACH f IN ARRAY ARRAY[
        'cairn_vault_bootstrap_rekey(text, text, text, bytea, text, text, text)',
        'cairn_vault_approver_ok(text, bytea, bytea, boolean)',
        'cairn_vault_set_unconfirmed_expiry(integer)',
        'cairn_vault_key_expires_at(text)'
    ] LOOP
        ASSERT NOT has_function_privilege('cairn_agent', f, 'EXECUTE'), format('%s is the owner''s alone', f);
        ASSERT NOT has_function_privilege('public', f, 'EXECUTE'), format('%s is revoked from PUBLIC', f);
    END LOOP;
    -- The site's policy (§5.5) is custody state, the owner's alone, like vault_terminal.
    ASSERT NOT has_table_privilege('cairn_agent', 'vault_policy', 'SELECT')
       AND NOT has_table_privilege('cairn_agent', 'vault_policy', 'UPDATE'), 'vault_policy is the owner''s';
END $$;

ROLLBACK;
