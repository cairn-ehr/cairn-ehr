\set ON_ERROR_STOP on
\ir _scratch_database_guard.sql
-- db/tests/017_match_proposal_test.sql
-- Issue #79 — the advisory match_proposal band CHECK (matcher B2 follow-up minor).
--
-- WHAT THIS GUARDS: `band` carries the matcher's immutable propose-time assessment and its
-- values are owned by the Python `cairn_matcher.pipeline.banding.Band` enum. Several
-- writers touch this TABLE (see db/017's header), but only the Python pipeline writes the
-- `band` COLUMN, so the CHECK is defence in depth: it stops a writer that is NOT that
-- pipeline (a psql session, a migration script, a future service) storing a band string no
-- reader can interpret. Advisory table, so this is the cheap kind of safety — a bad row is
-- a bad PROPOSAL a human reviews, never record corruption.
--
-- The Python-side twin of this file is matcher/tests/test_match_proposal_band_check.py,
-- which drives the DB with every `Band` member so the enum and this constraint cannot
-- drift apart silently (issue #119's two-place-mapping failure mode).

-- #207 paired-ALTER discipline: db/017 creates match_proposal with CREATE TABLE IF NOT
-- EXISTS, so on an ALREADY-EXISTING database the CREATE is a no-op and only the guarded
-- ALTER can install this constraint. Assert it is actually present after a full replay,
-- so a future refactor that drops or reorders that ALTER fails loudly here instead of
-- leaving long-lived databases silently unconstrained.
DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_constraint
                    WHERE conname = 'match_proposal_band_check'
                      AND conrelid = 'match_proposal'::regclass) THEN
        RAISE EXCEPTION 'FAIL: match_proposal_band_check missing after replay (#207)';
    END IF;
    RAISE NOTICE 'PASS: match_proposal_band_check present';
END $$;

-- ...and prove the ALTER path specifically, which the assertion above does NOT reach: this
-- runner uses a THROWAWAY database, so match_proposal was just created fresh and the inline
-- CONSTRAINT on the CREATE supplied the constraint. The paired ALTER exists for the other
-- case — a LONG-LIVED database (cairn_test, any deployed node) where the table predates the
-- constraint, the CREATE TABLE IF NOT EXISTS is a no-op, and the ALTER is the only thing
-- that can install it. Simulate exactly that: drop the constraint so the table looks like an
-- old database's, replay the schema file, and assert the constraint returns.
--
-- Both suites always build fresh databases, so WITHOUT this simulation nothing anywhere
-- exercises the ALTER, and the file could ship constraining new databases only.
DO $$
BEGIN
    ALTER TABLE match_proposal DROP CONSTRAINT match_proposal_band_check;
END $$;
\i db/017_match_proposal.sql
DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_constraint
                    WHERE conname = 'match_proposal_band_check'
                      AND conrelid = 'match_proposal'::regclass) THEN
        RAISE EXCEPTION 'FAIL: replay did not re-install the CHECK on an existing table — '
                        'the paired ALTER is missing or mis-guarded (#207)';
    END IF;
    RAISE NOTICE 'PASS: guarded ALTER re-installs the CHECK on a pre-existing table';
END $$;

-- A STALE constraint must converge too, not just an absent one. This is the failure mode a
-- name-keyed guard (`IF NOT EXISTS … conname = …`) cannot see: the constraint exists, so the
-- guard skips, and a widened value set never reaches any database that already had the old
-- one. Simulate an older node whose constraint predates a band: install a deliberately
-- NARROW constraint under the same name, replay, and assert the full current set is storable
-- again. Uses the enum's own second value, so it keeps working if the set later widens.
DO $$
BEGIN
    ALTER TABLE match_proposal DROP CONSTRAINT match_proposal_band_check;
    ALTER TABLE match_proposal
        ADD CONSTRAINT match_proposal_band_check CHECK (band IN ('review')) NOT VALID;
END $$;
\i db/017_match_proposal.sql
DO $$
DECLARE
    low  uuid := '00000000-0000-4000-8000-00000000000a';
    high uuid := '00000000-0000-4000-8000-00000000000b';
BEGIN
    -- 'auto_candidate' is exactly what the stale narrow constraint forbade.
    INSERT INTO match_proposal
        (patient_low, patient_high, score_total, band, veto_findings, evidence, matcher_version)
    VALUES (low, high, 9.0, 'auto_candidate', '[]'::jsonb, '[]'::jsonb, 'sqltest');
    RAISE NOTICE 'PASS: a STALE (narrower) constraint converges on replay';
EXCEPTION WHEN check_violation THEN
    RAISE EXCEPTION 'FAIL: replay left a stale narrow CHECK in place — the guard is keyed '
                    'on the constraint NAME rather than its definition, so a widened band '
                    'set can never reach an existing database (#79)';
END $$;

-- ...and the guard must be WRITE-FREE in the steady state. It runs on every connect, and a
-- DROP+ADD takes an ACCESS EXCLUSIVE lock, so a guard that misfires every time would stall
-- concurrent readers on a live node. A re-add creates a NEW pg_constraint row, so a stable
-- oid across a replay proves the guard correctly recognised an up-to-date constraint. This
-- is also what catches a future Postgres whose deparsed CHECK text stops matching the
-- `want` literal in db/017.
DO $$
DECLARE before_oid oid;
BEGIN
    SELECT oid INTO before_oid FROM pg_constraint
      WHERE conname = 'match_proposal_band_check'
        AND conrelid = 'match_proposal'::regclass;
    PERFORM set_config('cairn.test_band_check_oid', before_oid::text, false);
END $$;
\i db/017_match_proposal.sql
DO $$
DECLARE after_oid oid;
BEGIN
    SELECT oid INTO after_oid FROM pg_constraint
      WHERE conname = 'match_proposal_band_check'
        AND conrelid = 'match_proposal'::regclass;
    IF after_oid::text IS DISTINCT FROM current_setting('cairn.test_band_check_oid') THEN
        RAISE EXCEPTION 'FAIL: replay re-created the CHECK although it was already current '
                        '— the guard is not write-free in the steady state, so every '
                        'connect would take an ACCESS EXCLUSIVE lock (#79)';
    END IF;
    RAISE NOTICE 'PASS: guard is write-free when the constraint is already current';
END $$;

-- Both enum values must be accepted. If a future slice narrows the CHECK (or misspells a
-- value in it), the matcher's own output stops being storable — caught here.
DO $$
DECLARE
    low  uuid := '00000000-0000-4000-8000-000000000001';
    high uuid := '00000000-0000-4000-8000-000000000002';
    n    int;
BEGIN
    INSERT INTO match_proposal
        (patient_low, patient_high, score_total, band, veto_findings, evidence, matcher_version)
    VALUES (low, high, 9.0, 'auto_candidate', '[]'::jsonb, '[]'::jsonb, 'sqltest')
    ON CONFLICT (patient_low, patient_high) DO UPDATE SET band = 'auto_candidate';

    UPDATE match_proposal SET band = 'review'
      WHERE patient_low = low AND patient_high = high;

    SELECT count(*) INTO n FROM match_proposal
      WHERE patient_low = low AND patient_high = high AND band = 'review';
    IF n <> 1 THEN
        RAISE EXCEPTION 'FAIL: both band values must be storable, got % row(s)', n;
    END IF;
    RAISE NOTICE 'PASS: auto_candidate and review both accepted';
END $$;

-- A band outside the enum must be REFUSED. Without this the test above would pass equally
-- well against a table carrying no CHECK at all.
DO $$
DECLARE
    low  uuid := '00000000-0000-4000-8000-000000000003';
    high uuid := '00000000-0000-4000-8000-000000000004';
BEGIN
    BEGIN
        INSERT INTO match_proposal
            (patient_low, patient_high, score_total, band, veto_findings, evidence, matcher_version)
        VALUES (low, high, 9.0, 'not_a_real_band', '[]'::jsonb, '[]'::jsonb, 'sqltest');
        RAISE EXCEPTION 'FAIL: a band outside the Band enum was accepted';
    EXCEPTION WHEN check_violation THEN
        RAISE NOTICE 'PASS: non-enum band rejected by check_violation';
    END;
END $$;

-- ---------------------------------------------------------------------------------------------
-- #744 — `status` is CONSTRAINED too, now. It used to be pinned here as deliberately open, so
-- the next disposition value could be introduced without a schema change. The cost of that
-- openness turned out to be a safety hole on the human-review path: db/057's
-- `match_proposal_open` lists the OPEN statuses, so a status nobody recognises — a typo in any
-- writer, a raw-SQL session, a future service — is silently CLOSED. That proposal reaches neither
-- the banner nor the worklist, and "no banner" is supposed to mean only "checked, none open". The
-- definition-guarded ALTER makes a widening cheap (edit the list; every database converges on its
-- next replay), so the CHECK costs the next status one line, not a migration.
-- ---------------------------------------------------------------------------------------------

-- The constraint is present after a full replay.
DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_constraint
                    WHERE conname = 'match_proposal_status_check'
                      AND conrelid = 'match_proposal'::regclass) THEN
        RAISE EXCEPTION 'FAIL: match_proposal_status_check missing after replay (#744)';
    END IF;
    RAISE NOTICE 'PASS: match_proposal_status_check present';
END $$;

-- The guarded ALTER installs it on a PRE-EXISTING table (the only path a deployed node takes:
-- its CREATE TABLE IF NOT EXISTS is a no-op), exactly as the band check's simulation above does.
DO $$
BEGIN
    ALTER TABLE match_proposal DROP CONSTRAINT match_proposal_status_check;
END $$;
\i db/017_match_proposal.sql
DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_constraint
                    WHERE conname = 'match_proposal_status_check'
                      AND conrelid = 'match_proposal'::regclass) THEN
        RAISE EXCEPTION 'FAIL: replay did not install the status CHECK on an existing table (#744)';
    END IF;
    RAISE NOTICE 'PASS: guarded ALTER installs the status CHECK on a pre-existing table';
END $$;

-- ...and the guard is write-free once current: a stable oid across a replay.
DO $$
BEGIN
    PERFORM set_config('cairn.test_status_check_oid',
        (SELECT oid::text FROM pg_constraint
          WHERE conname = 'match_proposal_status_check'
            AND conrelid = 'match_proposal'::regclass), false);
END $$;
\i db/017_match_proposal.sql
DO $$
BEGIN
    IF (SELECT oid::text FROM pg_constraint
         WHERE conname = 'match_proposal_status_check'
           AND conrelid = 'match_proposal'::regclass)
       IS DISTINCT FROM current_setting('cairn.test_status_check_oid') THEN
        RAISE EXCEPTION 'FAIL: replay re-created the status CHECK although it was current — '
                        'every connect would take an ACCESS EXCLUSIVE lock (#744)';
    END IF;
    RAISE NOTICE 'PASS: the status guard is write-free when current';
END $$;

-- Every status a writer in the tree sets is storable. The list is the whole disposition axis:
-- pending (the matcher's proposal), accepted/rejected/applied (a human, C2 and chart_link),
-- auto_applied/review (C2b auto_apply.rs), retracted (the matcher's withdrawal). `applied_event_id`
-- rides along so db/019's own CHECK (#704) is satisfied for the two applied statuses.
DO $$
DECLARE
    s    text;
    i    int := 0;
BEGIN
    FOREACH s IN ARRAY ARRAY['pending', 'accepted', 'review', 'rejected', 'applied',
                             'auto_applied', 'retracted'] LOOP
        i := i + 1;
        INSERT INTO match_proposal
            (patient_low, patient_high, score_total, band, veto_findings, evidence,
             matcher_version, status, applied_event_id)
        VALUES (('00000000-0000-4000-8000-0000000001' || lpad(i::text, 2, '0'))::uuid,
                ('00000000-0000-4000-8000-0000000002' || lpad(i::text, 2, '0'))::uuid,
                9.0, 'review', '[]'::jsonb, '[]'::jsonb, 'sqltest', s,
                CASE WHEN s IN ('applied', 'auto_applied')
                     THEN '00000000-0000-7000-8000-000000000099'::uuid END);
    END LOOP;
    RAISE NOTICE 'PASS: every disposition a writer sets is storable';
EXCEPTION WHEN check_violation THEN
    RAISE EXCEPTION 'FAIL: status "%" is refused, but a writer in the tree sets it (#744): %',
        s, SQLERRM;
END $$;

-- A status nobody recognises is REFUSED — the property #744 exists for. Without this block the
-- one above passes equally well against a table with no status CHECK at all.
DO $$
BEGIN
    BEGIN
        INSERT INTO match_proposal
            (patient_low, patient_high, score_total, band, veto_findings, evidence,
             matcher_version, status)
        VALUES ('00000000-0000-4000-8000-000000000005', '00000000-0000-4000-8000-000000000006',
                9.0, 'review', '[]'::jsonb, '[]'::jsonb, 'sqltest', 'pendng');
        RAISE EXCEPTION 'FAIL: a misspelt status was stored — that proposal would vanish from '
                        'the banner and the worklist with no error anywhere (#744)';
    EXCEPTION WHEN check_violation THEN
        RAISE NOTICE 'PASS: an unrecognised status is refused at the write';
    END;
END $$;

-- Leave no residue for the next file in the run.
DELETE FROM match_proposal WHERE matcher_version = 'sqltest';
