\set ON_ERROR_STOP on
\ir _scratch_database_guard.sql
-- db/072 — the in-DB mirror for the supervision doors: the PURE address each signed act is bound
-- to (its exact preimage, restated here so a change to the formula is a failing line), and the
-- grants a raw-SQL client meets. The Rust suite (crates/cairn-node/tests/supervision_doors.rs)
-- drives the doors and their matrix end to end.
--
-- Wrapped in a transaction that ROLLS BACK: nothing here commits.

BEGIN;

-- 1. The address: '\x1220' || sha256 of newline-joined fields, staff IDs normalised, the
--    supervisor '' for an op that has none, and the family's prior seq last.
DO $$
BEGIN
    ASSERT cairn_supervision_address('link-supervisor', ' BWong ', ' JSmith ', 7) = '\x1220'::bytea
        || sha256(convert_to('cairn-supervision' || E'\n' || 'link-supervisor' || E'\n' || 'bwong'
                             || E'\n' || 'jsmith' || E'\n' || '7', 'UTF8')),
        'the link preimage';
    ASSERT cairn_supervision_address('require-cosign', 'bwong', NULL, 0) = '\x1220'::bytea
        || sha256(convert_to('cairn-supervision' || E'\n' || 'require-cosign' || E'\n' || 'bwong'
                             || E'\n' || '' || E'\n' || '0', 'UTF8')),
        'no supervisor is the empty field';
    ASSERT cairn_supervision_address('link-supervisor', 'bwong', 'jsmith', 0)
        <> cairn_supervision_address('link-supervisor', 'bwong', 'jsmith', 1),
        'a token is good for the state it changes, and only that';
    ASSERT cairn_supervision_address('link-supervisor', 'bwong', 'jsmith', 0)
        <> cairn_supervision_address('unlink-supervisor', 'bwong', 'jsmith', 0),
        'the op is bound';
    ASSERT cairn_supervision_address(NULL, 'bwong', NULL, 0) IS NULL,
        'a NULL field makes no address, which no token verifies against';
    ASSERT has_function_privilege('public', 'cairn_supervision_address(text, text, text, bigint)', 'EXECUTE'),
        'the pure address reads nothing, so PUBLIC keeps it';
END $$;

-- 2. The grants: the window reaches the doors it calls; the console door and the helpers are the
--    owner's alone; PUBLIC runs no definer.
DO $$
DECLARE f text;
BEGIN
    FOREACH f IN ARRAY ARRAY[
        'cairn_supervision_act(text, text, text, text, bytea)',
        'cairn_supervision_prior(text, text, text)',
        'cairn_supervision_staff()'
    ] LOOP
        ASSERT has_function_privilege('cairn_window', f, 'EXECUTE'), format('the window must reach %s', f);
        ASSERT NOT has_function_privilege('cairn_agent', f, 'EXECUTE'),
            format('%s is never the advisory role''s (#1129, ADR-0108)', f);
        ASSERT NOT has_function_privilege('public', f, 'EXECUTE'), format('%s is revoked from PUBLIC', f);
    END LOOP;
    ASSERT has_function_privilege('cairn_window', 'cairn_supervision_record()', 'EXECUTE')
       AND NOT has_function_privilege('cairn_agent', 'cairn_supervision_record()', 'EXECUTE')
       AND NOT has_function_privilege('public', 'cairn_supervision_record()', 'EXECUTE'),
        'the record is the window''s, as db/071''s reader doors are';
    FOREACH f IN ARRAY ARRAY[
        'cairn_supervision_console_act(text, text, text)',
        'cairn_supervision_approver(text, bytea, bytea)',
        'cairn_supervision_apply(text, text, text, text, bytea, boolean)'
    ] LOOP
        ASSERT NOT has_function_privilege('cairn_agent', f, 'EXECUTE'), format('%s is the owner''s alone', f);
        ASSERT NOT has_function_privilege('public', f, 'EXECUTE'), format('%s is revoked from PUBLIC', f);
    END LOOP;
END $$;

-- 3. #1014 (ADR-0107): the record's time is the row's HLC stamp, and the rows come in that order. A
--    row written before the stamp existed reads as (recorded_at floored to the millisecond, 0) —
--    the millisecond rule it was judged by before.
DO $$
DECLARE sv bytea; got text;
BEGIN
    sv := enroll_actor('human', '{"role":"clinician","handle":"rec-hlc"}', 'recHlcK');
    INSERT INTO actor_event (actor_id, op, approved_by, recorded_at)
    VALUES (sv, 'require-cosign', 'console', '2026-01-01 00:00:00.000700+00');
    INSERT INTO actor_event (actor_id, op, approved_by, recorded_at, hlc_wall, hlc_counter)
    VALUES (sv, 'lift-cosign', 'console', '2026-01-02 00:00:00+00', 1767139200000, 3);  -- 2025-12-31
    SELECT string_agg(r.op || '@' || r.hlc_wall || '.' || r.hlc_counter, ' ')
      INTO got FROM cairn_supervision_record() r WHERE r.actor_id = encode(sv, 'hex');
    ASSERT got = 'lift-cosign@1767139200000.3 require-cosign@1767225600000.0',
        format('the record orders by the HLC stamp, old rows at (ms, 0); got %s', got);
END $$;

ROLLBACK;
