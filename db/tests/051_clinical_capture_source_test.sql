\set ON_ERROR_STOP on
\ir _scratch_database_guard.sql
-- Issue #500 slice 2c — the in-DB mirror for the clinical capture's source of truth.
--
-- WHY THIS MIRROR EXISTS: `cairn_clinical_page` and `event_custody_surviving` are the ONE
-- definition of "a shredded body's key must not travel". Three callers select from them
-- (the serve door, the local-state export, the backup capture), so a change here is a
-- change to all three at once — which is the point, and the reason it deserves a test at
-- the SQL layer rather than only through whichever Rust caller happens to cover it.

BEGIN;

-- A sealed event with surviving custody, and a second one whose body has been shredded.
--
-- event_type is 'note.added', NOT a clinical.medication.* type: note.added's projection
-- (patient_chart_apply, db/002) is a self-contained upsert that reads nothing out of the
-- body, so this fixture stays about CUSTODY rather than also having to satisfy a clinical
-- projection's own NOT NULL columns (medication_statement_apply, for one, would try to
-- INSERT a NULL medication_id from an empty body and fail for a reason unrelated to this
-- test). body/contributors/signer_key_id/plaintext_twin are NOT NULL on event_log with no
-- default (db/001), and content_address must satisfy the event_content_addressed CHECK
-- (the multihash of signed_bytes) — the values below carry no meaning beyond "a legal row".
INSERT INTO event_log (event_id, patient_id, event_type, schema_version, hlc_wall,
                       hlc_counter, node_origin, signed_bytes, content_address,
                       body, contributors, signer_key_id, plaintext_twin)
VALUES ('11111111-1111-7111-8111-111111111111', gen_random_uuid(), 'note.added',
        '1', 1, 1, 'n1', '\xaa'::bytea,
        '\x1220'::bytea || digest('\xaa'::bytea, 'sha256'),
        '{}'::jsonb, '[]'::jsonb, 'deadbeef', 'mirror fixture'),
       ('22222222-2222-7222-8222-222222222222', gen_random_uuid(), 'note.added',
        '1', 2, 1, 'n1', '\xbb'::bytea,
        '\x1220'::bytea || digest('\xbb'::bytea, 'sha256'),
        '{}'::jsonb, '[]'::jsonb, 'deadbeef', 'mirror fixture');

INSERT INTO event_dek (event_id, dek_wrapped) VALUES
    ('11111111-1111-7111-8111-111111111111', '\xdeadbeef'::bytea),
    ('22222222-2222-7222-8222-222222222222', '\xfeedface'::bytea);

-- shred_event_id and basis are NOT NULL with no default (db/037): a real shred carries its
-- own erasure.shred.asserted event id and an audited basis (ADR-0005 rung 3). Neither is
-- under test here, so shred_event_id is a fresh uuid and basis is a plain fixture label.
INSERT INTO erasure_shred_log (target_event_id, shred_event_id, basis) VALUES
    ('22222222-2222-7222-8222-222222222222', gen_random_uuid(), 'mirror fixture shred');

-- 1. The view hides the shredded row and only that row.
DO $$
DECLARE n INT;
BEGIN
    SELECT count(*) INTO n FROM event_custody_surviving
     WHERE event_id = '22222222-2222-7222-8222-222222222222';
    ASSERT n = 0, 'a shredded body''s custody must not survive in the view';
    SELECT count(*) INTO n FROM event_custody_surviving
     WHERE event_id = '11111111-1111-7111-8111-111111111111';
    ASSERT n = 1, 'an unshredded body''s custody must survive — otherwise the test above is vacuous';
END $$;

-- 2. The page function carries the event but NULLs the shredded DEK. Both halves matter:
--    dropping the EVENT would fork the event set (the #342 trap); carrying the DEK would
--    defeat the shred.
DO $$
DECLARE r RECORD;
BEGIN
    SELECT * INTO r FROM cairn_clinical_page(0, NULL)
     WHERE seq = (SELECT seq FROM event_log WHERE event_id = '22222222-2222-7222-8222-222222222222');
    ASSERT r.signed_bytes = '\xbb'::bytea, 'the shredded event itself must still travel';
    ASSERT r.dek_wrapped IS NULL, 'the shredded event''s DEK must NOT travel';
END $$;

-- 3. page_limit NULL means "no limit" (the unpaginated serve path is the SAME statement).
DO $$
DECLARE n INT;
BEGIN
    SELECT count(*) INTO n FROM cairn_clinical_page(0, NULL);
    ASSERT n >= 2, 'a NULL page_limit must not limit';
    SELECT count(*) INTO n FROM cairn_clinical_page(0, 1);
    ASSERT n = 1, 'a page_limit of 1 must return exactly one row';
END $$;

-- 4. after_seq is STRICTLY greater — the puller's cursor semantics (#196).
DO $$
DECLARE n INT; first_seq BIGINT;
BEGIN
    SELECT min(seq) INTO first_seq FROM event_log;
    SELECT count(*) INTO n FROM cairn_clinical_page(first_seq, NULL) WHERE seq = first_seq;
    ASSERT n = 0, 'after_seq must be exclusive';
END $$;

ROLLBACK;
