-- #208/ADR-0057 — the cairn_projection_apply structural guards, SQL mirror of
-- crates/cairn-node/tests/projection_registry.rs's dispatcher_is_the_only_event_log_insert_trigger
-- and registry_row_count_is_pinned. Run after the schema is loaded. Uses a
-- transaction that ROLLBACKs so it leaves no residue.
BEGIN;

-- 1. Registry membership pinned. The throwaway DB this script builds loads
--    every migration in numeric order (scripts/run-db-sql-tests.sh), including
--    the spike-only db/008 the product loaders deliberately skip (issue #67) —
--    so the expected count here is the product count 22 + db/008's 3 = 25.
--    Kept in lockstep with the Rust mirror
--    (projection_registry.rs::registry_row_count_is_pinned, which asserts 22 on
--    the product-loader DB that never loads db/008).
DO $$
DECLARE n bigint;
BEGIN
    SELECT count(*) INTO n FROM cairn_projection_apply;
    IF n <> 25 THEN
        RAISE EXCEPTION 'cairn_projection_apply: expected 25 rows on the sqltest DB, found %', n;
    END IF;
END $$;

-- 2. ADR-0057's rule, enforced structurally: the dispatcher is the ONLY
--    row-level AFTER INSERT trigger on event_log (event_log_no_update is a
--    BEFORE UPDATE OR DELETE guard and must not count).
DO $$
DECLARE n bigint;
BEGIN
    SELECT count(*) INTO n
    FROM pg_trigger t JOIN pg_class cl ON cl.oid = t.tgrelid
    WHERE cl.relname = 'event_log' AND NOT t.tgisinternal
      AND pg_get_triggerdef(t.oid) LIKE '%AFTER INSERT%';
    IF n <> 1 THEN
        RAISE EXCEPTION 'event_log: expected exactly the dispatcher AFTER INSERT trigger, found %', n;
    END IF;
END $$;

-- 3. Registration-time fail-closed still bites (bogus fn refused). Both our own
--    sentinel RAISE and cairn_check_projection_registry_fn's raise are plain
--    RAISE EXCEPTION (default SQLSTATE P0001 / condition raise_exception, db/005),
--    so catch that condition and discriminate by exact message: an exact match on
--    our sentinel means validation wrongly accepted the row (re-raise, fail the
--    test); anything else is the validation trigger's own "does not exist"
--    message — expected, swallowed here.
DO $$
BEGIN
    INSERT INTO cairn_projection_apply (event_type, apply_fn, projection_tables, run_order, heal_safe)
    VALUES ('test.sql.bogus', 'no_such_fn_sql', ARRAY['patient_chart'], 10, true);
    RAISE EXCEPTION 'bogus apply_fn was accepted';
EXCEPTION WHEN raise_exception THEN
    IF SQLERRM = 'bogus apply_fn was accepted' THEN RAISE; END IF;
    -- expected: the validation trigger raised — swallowed, test passes.
END $$;

ROLLBACK;
