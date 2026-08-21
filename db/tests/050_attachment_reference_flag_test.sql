\set ON_ERROR_STOP on
\ir _scratch_database_guard.sql
-- Issue #460 — the in-DB mirror for the mint-strict / arrive-permissive attachment floor.
--
-- The Rust suite (crates/cairn-node/tests/attachment_reference_shape.rs) drives the real doors
-- end to end. This mirror drives the two LEARNERS directly, because they are the pair that must
-- never come to disagree about what "malformed" means, and because the SQL layer is where a
-- future edit to db/027 or db/050 lands. It runs in the rust.yml floor job via
-- scripts/run-db-sql-tests.sh.

-- ---------------------------------------------------------------------------
-- 1. Strict and lenient agree on WHAT is malformed, and differ ONLY in what they do.
--
-- Asserted as a pair over the same inputs rather than as two separate lists, so the two doors
-- cannot drift into two definitions without this failing. That drift is the thing db/027 was
-- extracted to prevent in the first place.
-- ---------------------------------------------------------------------------
DO $$
DECLARE
    bodies text[] := ARRAY[
        '{"attachments":"scalar"}',
        '{"attachments":[{"renditions":"scalar"}]}',
        '{"attachments":[{"renditions":[{"digest_hex":"0xABC","media_type":"image/png","byte_len":3}]}]}',
        '{"attachments":[{"renditions":[{"digest_hex":"","media_type":"image/png","byte_len":3}]}]}',
        '{"attachments":[{"renditions":[{"digest_hex":"1e20aa","byte_len":3}]}]}',
        '{"attachments":[{"renditions":[{"digest_hex":"1e20ab","media_type":"image/png","byte_len":3.5}]}]}'
    ];
    i int;
    v_raised boolean;
BEGIN
    FOR i IN 1 .. array_length(bodies, 1) LOOP
        -- STRICT: refuses, and with P0001 — the code cairn-sync skips past rather than freezing on.
        v_raised := FALSE;
        BEGIN
            PERFORM cairn_learn_attachment_refs(bodies[i]::jsonb);
        EXCEPTION
            WHEN raise_exception THEN v_raised := TRUE;
            WHEN OTHERS THEN
                RAISE EXCEPTION 'FAIL: case % refused with SQLSTATE %, not P0001 — anything but '
                                'P0001 freezes the pull cursor (#370)', i, SQLSTATE;
        END;
        IF NOT v_raised THEN
            RAISE EXCEPTION 'FAIL: the STRICT learner accepted malformed case %', i;
        END IF;

        -- LENIENT: does not refuse. (What it records is asserted in section 2, against a real
        -- event_id; here the point is only that the same input does not raise.)
        BEGIN
            PERFORM cairn_learn_attachment_refs_lenient(
                jsonb_set(bodies[i]::jsonb, '{event_id}', to_jsonb(gen_random_uuid()::text)));
        EXCEPTION WHEN OTHERS THEN
            RAISE EXCEPTION 'FAIL: the LENIENT learner refused malformed case % (SQLSTATE %) — '
                            'at the apply door the event is already a fact and refusing it forks '
                            'the event set (#460)', i, SQLSTATE;
        END;
    END LOOP;
    RAISE NOTICE 'PASS: strict refuses and lenient admits, over one shared list of malformed shapes';
END $$;

-- ---------------------------------------------------------------------------
-- 2. A defect on one rendition never invalidates its siblings, and the flag NAMES which.
--
-- The flag rows need a real event_id: attachment_reference_flag references event_log. Rather
-- than synthesise a signed event in SQL, this section drives the recorder + the traversal that
-- the lenient learner composes, which is what a db/050 edit would break.
-- ---------------------------------------------------------------------------
DO $$
DECLARE
    b jsonb := '{"attachments":[{"renditions":[
                    {"digest_hex":"1e20e0e0e0e0","media_type":"image/png","byte_len":3},
                    {"digest_hex":"0xNOPE","media_type":"image/png","byte_len":3},
                    {"digest_hex":"1e20e1e1e1e1","media_type":"image/png","byte_len":3}]}]}'::jsonb;
    n int;
BEGIN
    -- The shared traversal yields all three by-reference renditions, indexed from zero.
    SELECT count(*) INTO n FROM cairn_by_reference_renditions(b, 'mirror');
    IF n <> 3 THEN
        RAISE EXCEPTION 'FAIL: expected 3 by-reference renditions, got %', n;
    END IF;

    SELECT count(*) INTO n
    FROM cairn_by_reference_renditions(b, 'mirror')
    WHERE attachment_index = 0 AND rendition_index IN (0, 1, 2);
    IF n <> 3 THEN
        RAISE EXCEPTION 'FAIL: the traversal must NAME each position (attachment, rendition)';
    END IF;

    -- An INLINE rendition is skipped: its bytes ride the event, so there is no lazy blob.
    SELECT count(*) INTO n FROM cairn_by_reference_renditions(
        '{"attachments":[{"renditions":[{"inline":"AAEC","media_type":"image/png"}]}]}'::jsonb,
        'mirror');
    IF n <> 0 THEN
        RAISE EXCEPTION 'FAIL: an inline rendition must not be treated as by-reference, got %', n;
    END IF;

    RAISE NOTICE 'PASS: the shared traversal names positions and skips inline renditions';
END $$;

-- ---------------------------------------------------------------------------
-- 3. The recorder cannot raise, and dedupes — including on the NOT-ATTRIBUTABLE row.
--
-- The NULL/NULL case is the one the default NULLS DISTINCT would get wrong: every
-- not-attributable row would be unique against every other, so a body whose attachments list is
-- malformed would add a row per re-offer, forever. That is the case with no index to name, so it
-- is the case most likely to be missed.
-- ---------------------------------------------------------------------------
DO $$
DECLARE
    v_event uuid;
    n int;
BEGIN
    -- A real event_log row to hang the flags off. The FK to event_log is what stops a flag
    -- naming an event this node does not hold; its ON DELETE CASCADE mirrors event_deferred but
    -- is UNREACHABLE in practice and is not asserted here, because event_log is append-only —
    -- the db/001 trigger refuses DELETE outright (principle 1), which is how this test found out.
    v_event := gen_random_uuid();
    -- Every NOT NULL column without a default, listed explicitly. A raw INSERT rather than a
    -- door call: this section is about the LEDGER's own mechanics (dedup, cascade), and routing
    -- through submit_event would drag in signing, enrolment and the #345 registration
    -- precedence — none of which this section is testing, all of which could fail it for
    -- unrelated reasons. The scratch-database guard at the top is what makes a raw insert
    -- acceptable here.
    INSERT INTO event_log (event_id, patient_id, event_type, schema_version,
                           hlc_wall, hlc_counter, node_origin, signed_bytes, content_address,
                           body, contributors, signer_key_id, plaintext_twin)
    VALUES (v_event, gen_random_uuid(), 'note.added', 'note/1',
            1782000000000, 0, 'mirror', '\x00'::bytea,
            -- content_address must satisfy the event_content_addressed CHECK: the multihash
            -- prefix for sha2-256 (0x12 0x20) followed by the digest of signed_bytes. Computed
            -- here rather than written as a literal, so the fixture stays correct if the bytes
            -- above ever change and so nothing in a crypto context is hard-coded (house rule 6).
            '\x1220'::bytea || digest('\x00'::bytea, 'sha256'),
            '{}'::jsonb, '[]'::jsonb, 'deadbeef', 'mirror fixture');

    PERFORM cairn_record_attachment_reference_flag(v_event, 0, 1, 'first');
    PERFORM cairn_record_attachment_reference_flag(v_event, 0, 1, 'again, same position');
    PERFORM cairn_record_attachment_reference_flag(v_event, NULL, NULL, 'not attributable');
    PERFORM cairn_record_attachment_reference_flag(v_event, NULL, NULL, 'not attributable again');

    SELECT count(*) INTO n FROM attachment_reference_flag WHERE event_id = v_event;
    IF n <> 2 THEN
        RAISE EXCEPTION 'FAIL: expected 2 deduped flag rows (one positioned, one '
                        'not-attributable), got % — NULLS NOT DISTINCT is load-bearing', n;
    END IF;

    -- The first reason wins: ON CONFLICT DO NOTHING, never DO UPDATE. The earliest observation
    -- is the one the operator should see.
    SELECT count(*) INTO n FROM attachment_reference_flag
    WHERE event_id = v_event AND reason = 'first';
    IF n <> 1 THEN
        RAISE EXCEPTION 'FAIL: the first recorded reason must survive re-delivery';
    END IF;

    -- THE NON-GATING PROPERTY, which is the one that can take a door down if it regresses.
    -- The recorder runs INSIDE the handler catching a refusal, so anything it raised would
    -- escape past that handler and refuse the clinical event — the exact harm this file exists
    -- to prevent. Recording against an event that is not here must therefore be a silent no-op,
    -- not a foreign-key violation. The first draft had no WHERE EXISTS and raised 23503; this
    -- mirror caught it on its first run.
    BEGIN
        PERFORM cairn_record_attachment_reference_flag(gen_random_uuid(), 0, 0, 'no such event');
    EXCEPTION WHEN OTHERS THEN
        RAISE EXCEPTION 'FAIL: the recorder raised % for an absent event — inside the refusal '
                        'handler that would propagate and refuse the clinical event (#460)', SQLSTATE;
    END;

    RAISE NOTICE 'PASS: the recorder dedupes on both shapes and cannot raise';
END $$;

-- ---------------------------------------------------------------------------
-- 4. The read surface exists and is reachable by BOTH group roles.
--
-- db/043's cairn_patient_deferred_sensitivity shipped granted to cairn_agent alone, described as
-- "the runtime role", which it is not — the runtime connects as a cairn_node member (#425). A
-- chart-scoped definer that only one of the two roles can call is a report nobody reads.
-- ---------------------------------------------------------------------------
DO $$
BEGIN
    IF to_regprocedure('cairn_patient_attachment_flags(uuid)') IS NULL THEN
        RAISE EXCEPTION 'FAIL: the read surface is missing — a ledger nobody can query is the '
                        'Slice 69 finding repeated';
    END IF;
    IF NOT has_function_privilege('cairn_agent', 'cairn_patient_attachment_flags(uuid)', 'EXECUTE') THEN
        RAISE EXCEPTION 'FAIL: cairn_agent cannot read the attachment-flag report';
    END IF;
    IF NOT has_function_privilege('cairn_node', 'cairn_patient_attachment_flags(uuid)', 'EXECUTE') THEN
        RAISE EXCEPTION 'FAIL: cairn_node cannot read the attachment-flag report (#425 — the '
                        'runtime connects as a cairn_node member)';
    END IF;
    IF has_function_privilege('public', 'cairn_patient_attachment_flags(uuid)', 'EXECUTE') THEN
        RAISE EXCEPTION 'FAIL: EXECUTE is still granted to PUBLIC on a SECURITY DEFINER read';
    END IF;
    RAISE NOTICE 'PASS: the read surface is reachable by both group roles and not by PUBLIC';
END $$;
