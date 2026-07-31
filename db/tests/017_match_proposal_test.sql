-- db/tests/017_match_proposal_test.sql
-- Issue #79 — the advisory match_proposal band CHECK (matcher B2 follow-up minor).
--
-- WHAT THIS GUARDS: `band` carries the matcher's immutable propose-time assessment and its
-- values are owned by the Python `cairn_matcher.pipeline.banding.Band` enum. Only the
-- Python pipeline writes this table today, so the CHECK is defence in depth: it stops a
-- writer that is NOT that pipeline (a psql session, a migration script, a future service)
-- storing a band string no reader can interpret. Advisory table, so this is the cheap kind
-- of safety — a bad row is a bad PROPOSAL a human reviews, never record corruption.
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

-- `status` must stay UNCONSTRAINED — the deliberate asymmetry documented in db/017. It is
-- the open disposition axis (human verdicts, matcher auto-application, retraction, and
-- whatever a later slice adds); only `band` is enum-owned. Pin the intent so a future
-- reader does not "complete" the work above by locking `status` down too, which would
-- break the next disposition value the moment it is introduced.
DO $$
DECLARE
    low  uuid := '00000000-0000-4000-8000-000000000005';
    high uuid := '00000000-0000-4000-8000-000000000006';
BEGIN
    INSERT INTO match_proposal
        (patient_low, patient_high, score_total, band, veto_findings, evidence, matcher_version,
         status)
    VALUES (low, high, 9.0, 'review', '[]'::jsonb, '[]'::jsonb, 'sqltest',
            'a-status-no-slice-has-invented-yet');
    RAISE NOTICE 'PASS: status remains deliberately open';
END $$;

-- Leave no residue for the next file in the run.
DELETE FROM match_proposal WHERE matcher_version = 'sqltest';
