\set ON_ERROR_STOP on
\ir _scratch_database_guard.sql
-- db/073 — the in-DB mirror for the person menu's two read doors: they exist, are definers with a
-- pg_temp-last search_path, and only cairn_window may run them. The Rust suite
-- (crates/cairn-node/tests/note_menu_doors.rs) drives their rows.
--
-- Wrapped in a transaction that ROLLS BACK: nothing here commits.

BEGIN;

DO $$
DECLARE f text;
BEGIN
    FOREACH f IN ARRAY ARRAY['cairn_note_people()', 'cairn_note_drafts_by_writer()'] LOOP
        ASSERT (SELECT p.prosecdef FROM pg_proc p WHERE p.oid = f::regprocedure), f || ' is a definer';
        -- Exactly public then pg_temp: a pg_temp-FIRST path (the hijack this guards) must fail here.
        ASSERT (SELECT p.proconfig = ARRAY['search_path=public, pg_temp'] FROM pg_proc p
                 WHERE p.oid = f::regprocedure), f || ' pins search_path = public, pg_temp';
        ASSERT NOT has_function_privilege('public', f, 'EXECUTE'), f || ' is not PUBLIC';
        ASSERT has_function_privilege('cairn_window', f, 'EXECUTE'), 'the window runs ' || f;
    END LOOP;
END $$;

ROLLBACK;
