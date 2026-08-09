-- db/tests/001_hlc_merge_helper_test.sql — SQL mirror of the issue #227 guards.
--
-- The A3 HLC merge (drag our clock past every event we admit) used to be pasted
-- verbatim into five doors; it now lives once in cairn_node_hlc_merge (db/001). The
-- Rust suite crates/cairn-node/tests/hlc_merge_helper.rs carries the same assertions
-- and adds two SOURCE-level guards this file cannot express, because they read the
-- migration text rather than the loaded schema: that no migration re-grows a copy of
-- the merge, and that all five doors still PERFORM the helper.
--
-- Mirrored here so the properties are checked by scripts/run-db-sql-tests.sh too — the
-- lesson of PR #182, where a guard living in only one of the two places drifted.
\set ON_ERROR_STOP on
\ir _scratch_database_guard.sql
BEGIN;

-- The merge must NOT be reachable by the unprivileged runtime role. Each door applies
-- its drift ceiling (issues #102/#193) BEFORE merging, so a directly-callable merge
-- would be a ceiling bypass: one runtime connection could ratchet hlc_state into the
-- far future and wedge this node out of the federation (every peer would then refuse
-- the events it authors). Two barriers, both asserted — the helper is deliberately
-- invoker-rights, so an accidental EXECUTE grant is still stopped by the missing
-- UPDATE on hlc_state. A SECURITY DEFINER helper would have no such second barrier.
DO $$ BEGIN
    IF has_function_privilege('cairn_node', 'cairn_node_hlc_merge(bigint,integer)', 'EXECUTE') THEN
        RAISE EXCEPTION 'ratchet-door FAILED: cairn_node holds EXECUTE on cairn_node_hlc_merge';
    END IF;
    IF has_table_privilege('cairn_node', 'hlc_state', 'UPDATE') THEN
        RAISE EXCEPTION 'ratchet-door FAILED: cairn_node can raw-UPDATE hlc_state';
    END IF;
    RAISE NOTICE 'ratchet-door OK: the merge is owner-only, and hlc_state is door-only';
END $$;

-- NULL arguments fail closed rather than degrading to a silent no-op (GREATEST ignores
-- NULLs and `NULL > x` is NULL, so an unguarded merge would quietly do nothing and look
-- like it worked). Unreachable through any door today — node_event.hlc_wall and
-- event_log.hlc_wall are NOT NULL and inserted before the merge — but the helper's
-- signature is a surface the next caller does not inherit that ordering from.
DO $$ BEGIN
    BEGIN
        PERFORM cairn_node_hlc_merge(NULL::bigint, 0::integer);
        RAISE EXCEPTION 'null-guard FAILED: a NULL wall was accepted';
    EXCEPTION WHEN others THEN
        IF SQLERRM LIKE '%must not be NULL%'
            THEN RAISE NOTICE 'null-guard OK (wall): %', SQLERRM; ELSE RAISE; END IF;
    END;
    BEGIN
        PERFORM cairn_node_hlc_merge(0::bigint, NULL::integer);
        RAISE EXCEPTION 'null-guard FAILED: a NULL counter was accepted';
    EXCEPTION WHEN others THEN
        IF SQLERRM LIKE '%must not be NULL%'
            THEN RAISE NOTICE 'null-guard OK (counter): %', SQLERRM; ELSE RAISE; END IF;
    END;
END $$;

-- Monotonicity — the whole truth table of the merge, from a seeded (100, 5):
--   older wall               -> nothing moves
--   equal wall, lower count  -> nothing moves
--   equal wall, higher count -> counter advances, wall stays
--   newer wall               -> wall advances and ADOPTS the incoming counter (the
--                               incoming counter is only meaningful against its own wall)
DO $$
DECLARE
    v_wall BIGINT; v_counter INTEGER;
    c RECORD;
BEGIN
    FOR c IN SELECT * FROM (VALUES
        (50::bigint,  9::integer, 100::bigint, 5::integer, 'a strictly older wall'),
        (100::bigint, 3::integer, 100::bigint, 5::integer, 'an equal wall, lower counter'),
        (100::bigint, 7::integer, 100::bigint, 7::integer, 'an equal wall, higher counter'),
        (200::bigint, 2::integer, 200::bigint, 2::integer, 'a strictly newer wall')
    ) AS t(in_wall, in_counter, want_wall, want_counter, why)
    LOOP
        -- Re-seed per case so the cases are independent and order-insensitive.
        UPDATE hlc_state SET hlc_wall = 100, hlc_counter = 5 WHERE id;
        PERFORM cairn_node_hlc_merge(c.in_wall, c.in_counter);
        SELECT hlc_wall, hlc_counter INTO v_wall, v_counter FROM hlc_state WHERE id;
        IF v_wall <> c.want_wall OR v_counter <> c.want_counter THEN
            RAISE EXCEPTION 'monotonicity FAILED for % — merging (%, %) into (100, 5) gave (%, %), want (%, %)',
                c.why, c.in_wall, c.in_counter, v_wall, v_counter, c.want_wall, c.want_counter;
        END IF;
    END LOOP;
    RAISE NOTICE 'monotonicity OK: the merge never moves the clock backwards';
END $$;

ROLLBACK;
