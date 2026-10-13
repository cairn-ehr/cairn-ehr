\set ON_ERROR_STOP on
\ir _scratch_database_guard.sql
-- db/tests/019_apply_proposal_test.sql
-- #704 — db/019's invariant, enforced in the floor rather than only stated:
--
--     applied_event_id IS NOT NULL  <=>  status IN ('applied', 'auto_applied')
--
-- WHY IT MATTERS. "Did this proposal produce a link?" is answered by `applied_event_id`, never by
-- `status` alone (db/019's header), and the matcher's ADR-0078 reopen clears the id as it reverts
-- an overruled auto-link to 'pending'. Until this constraint, every one of those writers held the
-- invariant by convention; any other writer (raw SQL, a future seam) could break it, and a
-- 'pending' row still pointing at a link event reads as linked to every consumer that follows the
-- header's own advice. Principle 12: the floor, not the writers, owns it.
--
-- Same idiom as db/017's band and status checks: guarded on the deparsed definition, added
-- NOT VALID so one pre-existing bad advisory row can never abort a schema load.

-- Present after a full replay.
DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_constraint
                    WHERE conname = 'match_proposal_applied_event_check'
                      AND conrelid = 'match_proposal'::regclass) THEN
        RAISE EXCEPTION 'FAIL: match_proposal_applied_event_check missing after replay (#704)';
    END IF;
    RAISE NOTICE 'PASS: match_proposal_applied_event_check present';
END $$;

-- Installed on a pre-existing table by the guarded ALTER, and write-free once current.
DO $$
BEGIN
    ALTER TABLE match_proposal DROP CONSTRAINT match_proposal_applied_event_check;
END $$;
\i db/019_apply_proposal.sql
DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_constraint
                    WHERE conname = 'match_proposal_applied_event_check'
                      AND conrelid = 'match_proposal'::regclass) THEN
        RAISE EXCEPTION 'FAIL: replay did not install the applied-event CHECK on an existing table (#704)';
    END IF;
    PERFORM set_config('cairn.test_applied_check_oid',
        (SELECT oid::text FROM pg_constraint
          WHERE conname = 'match_proposal_applied_event_check'
            AND conrelid = 'match_proposal'::regclass), false);
END $$;
\i db/019_apply_proposal.sql
DO $$
BEGIN
    IF (SELECT oid::text FROM pg_constraint
         WHERE conname = 'match_proposal_applied_event_check'
           AND conrelid = 'match_proposal'::regclass)
       IS DISTINCT FROM current_setting('cairn.test_applied_check_oid') THEN
        RAISE EXCEPTION 'FAIL: replay re-created the applied-event CHECK although it was current (#704)';
    END IF;
    RAISE NOTICE 'PASS: guarded ALTER installs the CHECK, and is write-free once current';
END $$;

BEGIN;

-- Both directions are refused: an applied status with no event, and an event on an open row.
DO $$
BEGIN
    BEGIN
        INSERT INTO match_proposal
            (patient_low, patient_high, score_total, band, veto_findings, evidence,
             matcher_version, status)
        VALUES ('00000000-0000-4000-8000-000000000701', '00000000-0000-4000-8000-000000000702',
                9.0, 'auto_candidate', '[]'::jsonb, '[]'::jsonb, 'sqltest', 'auto_applied');
        RAISE EXCEPTION 'FAIL: an auto_applied row with no applied_event_id was stored (#704)';
    EXCEPTION WHEN check_violation THEN
        RAISE NOTICE 'PASS: an applied status without its event is refused';
    END;
    BEGIN
        INSERT INTO match_proposal
            (patient_low, patient_high, score_total, band, veto_findings, evidence,
             matcher_version, status, applied_event_id)
        VALUES ('00000000-0000-4000-8000-000000000703', '00000000-0000-4000-8000-000000000704',
                9.0, 'review', '[]'::jsonb, '[]'::jsonb, 'sqltest', 'pending',
                '00000000-0000-7000-8000-000000000799');
        RAISE EXCEPTION 'FAIL: a pending row pointing at a link event was stored — it reads as '
                        'linked to every consumer that tests applied_event_id (#704)';
    EXCEPTION WHEN check_violation THEN
        RAISE NOTICE 'PASS: an event on an open row is refused';
    END;
END $$;

-- The two shapes writers actually leave are admitted: an applied row WITH its event, and the
-- matcher's ADR-0078 reopen (auto_applied -> pending) clearing the id in the same UPDATE.
DO $$
BEGIN
    INSERT INTO match_proposal
        (patient_low, patient_high, score_total, band, veto_findings, evidence,
         matcher_version, status, applied_event_id)
    VALUES ('00000000-0000-4000-8000-000000000705', '00000000-0000-4000-8000-000000000706',
            9.0, 'auto_candidate', '[]'::jsonb, '[]'::jsonb, 'sqltest', 'auto_applied',
            '00000000-0000-7000-8000-000000000798');
    UPDATE match_proposal SET status = 'pending', applied_event_id = NULL
     WHERE patient_low = '00000000-0000-4000-8000-000000000705';
    RAISE NOTICE 'PASS: an applied row and its reopen-with-clear are both admitted';
EXCEPTION WHEN check_violation THEN
    RAISE EXCEPTION 'FAIL: a shape a real writer leaves was refused (#704): %', SQLERRM;
END $$;

ROLLBACK;
