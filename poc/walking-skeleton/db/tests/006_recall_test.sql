\set ON_ERROR_STOP on
-- recall_event marks a target without deleting it (principle 2).
DO $$
DECLARE n_before bigint; n_after bigint; tgt uuid;
BEGIN
    SELECT count(*) INTO n_before FROM event_log;
    SELECT event_id INTO tgt FROM event_log LIMIT 1;
    IF tgt IS NOT NULL THEN
        PERFORM recall_event(tgt, 'skill-epoch contamination test');
        SELECT count(*) INTO n_after FROM event_log;
        IF n_after <> n_before THEN RAISE EXCEPTION 'recall ERASED data: % -> %', n_before, n_after; END IF;
        IF NOT EXISTS (SELECT 1 FROM recall_overlay WHERE target_event_id = tgt)
            THEN RAISE EXCEPTION 'recall overlay missing'; END IF;
        RAISE NOTICE 'recall OK: overlay added, no data erased';
    END IF;
END $$;
