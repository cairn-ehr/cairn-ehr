\set ON_ERROR_STOP on
\ir _scratch_database_guard.sql
-- Issue #460 — the in-DB mirror for the mint-strict / arrive-permissive attachment floor.
--
-- The Rust suite (crates/cairn-node/tests/attachment_reference_shape.rs) drives the real doors
-- end to end. This mirror drives the two LEARNERS directly, because they are the pair that must
-- never come to disagree about what "malformed" means, and because the SQL layer is where a
-- future edit to db/027 or db/050 lands. It runs in the rust.yml floor job via
-- scripts/run-db-sql-tests.sh.
--
-- ⚠️ THIS MIRROR CARRIES THE LIST-SHAPE CASES ALONE, AND NOTHING ELSE CAN.
-- `EventBody.attachments` is a `Vec<Attachment>` and `Attachment.renditions` a
-- `Vec<Rendition>`, so a body whose `attachments` or `renditions` is NOT A LIST is
-- unrepresentable in Rust, and `cairn_event::sign` takes a typed `EventBody` — there is no
-- public way to sign hand-built JSON. Every list-shape assertion below is therefore the ONLY
-- coverage that exists for the fault class with the largest blast radius. Do not thin it on the
-- grounds that "the Rust suite covers the doors": it cannot reach these shapes at all.

-- ---------------------------------------------------------------------------
-- The fixture: a real event_log row to hang flags off.
--
-- A raw INSERT rather than a door call, deliberately: these sections are about the LEARNERS and
-- the LEDGER, and routing through submit_event would drag in signing, enrolment and the #345
-- registration precedence — none of which is under test, all of which could fail these for
-- unrelated reasons. The scratch-database guard at the top is what makes a raw insert acceptable.
--
-- `signed_bytes` is DERIVED from a fresh uuid rather than a literal, for two reasons: the file
-- must be re-runnable against the same database (event_log carries a UNIQUE on content_address,
-- so a fixed byte string dies on the second run with an error that looks like a test failure),
-- and nothing in a crypto context may be hard-coded (house rule 6).
-- ---------------------------------------------------------------------------
CREATE OR REPLACE FUNCTION mirror_new_event() RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE
    v_event uuid := gen_random_uuid();
    v_bytes bytea := decode(replace(v_event::text, '-', ''), 'hex');
BEGIN
    INSERT INTO event_log (event_id, patient_id, event_type, schema_version,
                           hlc_wall, hlc_counter, node_origin, signed_bytes, content_address,
                           body, contributors, signer_key_id, plaintext_twin)
    VALUES (v_event, gen_random_uuid(), 'note.added', 'note/1',
            1782000000000, 0, 'mirror', v_bytes,
            -- content_address must satisfy the event_content_addressed CHECK: the multihash
            -- prefix for sha2-256 (0x12 0x20) followed by the digest of signed_bytes.
            '\x1220'::bytea || digest(v_bytes, 'sha256'),
            '{}'::jsonb, '[]'::jsonb, 'deadbeef', 'mirror fixture');
    RETURN v_event;
END $$;

-- A body carrying one attachments value, addressed to one event.
CREATE OR REPLACE FUNCTION mirror_body(p_event uuid, p_attachments jsonb)
RETURNS jsonb LANGUAGE sql IMMUTABLE AS $$
    SELECT jsonb_build_object('event_id', p_event::text, 'attachments', p_attachments);
$$;

-- ---------------------------------------------------------------------------
-- 1. Strict and lenient agree on WHAT is malformed, and differ ONLY in what they do.
--
-- Asserted as a pair over the same inputs rather than as two separate lists, so the two doors
-- cannot drift into two definitions without this failing. That drift is the thing db/027 was
-- extracted to prevent in the first place.
--
-- ⚠️ THE LENIENT HALF ASSERTS THAT IT **RECORDS**, NOT MERELY THAT IT DOES NOT RAISE.
-- An earlier draft ran the lenient learner against a `gen_random_uuid()` event that was not in
-- event_log, so the recorder's `WHERE EXISTS` guard skipped every insert and the section passed
-- in full with the learner replaced by `BEGIN RETURN; END`. "Admits" without "records" is the
-- admitted-and-silent untruth this whole file exists to prevent, so it is pinned here.
--
-- The list is the SAME twelve shapes the Rust strict-door table carries
-- (attachment_reference_shape.rs). Six of them used to be exercised strictly only; if any one
-- escaped the lenient handler, an already-signed clinical event would be REFUSED at the apply
-- door — the exact #460 harm, with a green suite.
-- ---------------------------------------------------------------------------
DO $$
DECLARE
    bodies text[] := ARRAY[
        -- the two LIST-SHAPE faults (no rendition to name)
        '{"attachments":"hello"}',
        '{"attachments":[{"renditions":"hello"}]}',
        -- a rendition that is not an object
        '{"attachments":[{"renditions":[42]}]}',
        -- the address accessor
        '{"attachments":[{"renditions":[{"digest_hex":"0xABC","media_type":"image/png","byte_len":3}]}]}',
        '{"attachments":[{"renditions":[{"digest_hex":"abc","media_type":"image/png","byte_len":3}]}]}',
        '{"attachments":[{"renditions":[{"media_type":"image/png","byte_len":3}]}]}',
        '{"attachments":[{"renditions":[{"digest_hex":"","media_type":"image/png","byte_len":3}]}]}',
        -- the media-type accessor
        '{"attachments":[{"renditions":[{"digest_hex":"1e20aa","byte_len":3}]}]}',
        '{"attachments":[{"renditions":[{"digest_hex":"1e20ab","media_type":"   ","byte_len":3}]}]}',
        -- the byte-length accessor
        '{"attachments":[{"renditions":[{"digest_hex":"1e20ac","media_type":"image/png","byte_len":3.5}]}]}',
        '{"attachments":[{"renditions":[{"digest_hex":"1e20ad","media_type":"image/png","byte_len":-5}]}]}',
        '{"attachments":[{"renditions":[{"digest_hex":"1e20ae","media_type":"image/png","byte_len":999999999999999999999}]}]}'
    ];
    i int;
    v_raised boolean;
    v_event uuid;
    n int;
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

        -- LENIENT: does not refuse, AND leaves a row naming what it could not learn.
        v_event := mirror_new_event();
        BEGIN
            PERFORM cairn_learn_attachment_refs_lenient(mirror_body(v_event, bodies[i]::jsonb -> 'attachments'));
        EXCEPTION WHEN OTHERS THEN
            RAISE EXCEPTION 'FAIL: the LENIENT learner refused malformed case % (SQLSTATE %) — '
                            'at the apply door the event is already a fact and refusing it forks '
                            'the event set (#460)', i, SQLSTATE;
        END;

        SELECT count(*) INTO n FROM attachment_reference_flag WHERE event_id = v_event;
        IF n < 1 THEN
            RAISE EXCEPTION 'FAIL: the LENIENT learner swallowed malformed case % WITHOUT '
                            'recording it — admitted-and-silent is the "record looks complete" '
                            'untruth this floor exists to prevent (#460)', i;
        END IF;
    END LOOP;
    RAISE NOTICE 'PASS: strict refuses and lenient admits-AND-records, over one shared list of 12 malformed shapes';
END $$;

-- ---------------------------------------------------------------------------
-- 2. The shared traversal names positions and skips inline renditions.
--
-- `cairn_by_reference_renditions` is the ONE traversal both learners run (section 9 pins that
-- they both call it). It owns the list coercion and the inline skip, so the two doors cannot
-- drift into two definitions of "which renditions are even candidates".
-- ---------------------------------------------------------------------------
DO $$
DECLARE
    n int;
BEGIN
    SELECT count(*) INTO n FROM cairn_by_reference_renditions(
        '{"attachments":[{"renditions":[{"digest_hex":"aa"},{"digest_hex":"bb"}]},
                         {"renditions":[{"digest_hex":"cc"}]}]}'::jsonb, 'mirror');
    IF n <> 3 THEN
        RAISE EXCEPTION 'FAIL: expected 3 by-reference renditions, got %', n;
    END IF;

    -- DISTINCT positions, not merely three rows carrying an accepted index. A traversal that
    -- returned (0,0) three times satisfied the old count-based check.
    SELECT count(DISTINCT (attachment_index, rendition_index)) INTO n
      FROM cairn_by_reference_renditions(
        '{"attachments":[{"renditions":[{"digest_hex":"aa"},{"digest_hex":"bb"}]},
                         {"renditions":[{"digest_hex":"cc"}]}]}'::jsonb, 'mirror');
    IF n <> 3 THEN
        RAISE EXCEPTION 'FAIL: the traversal must NAME each position distinctly, got % distinct '
                        'of 3 rows', n;
    END IF;

    -- and the second attachment really is index 1, not a repeat of 0.
    SELECT count(*) INTO n FROM cairn_by_reference_renditions(
        '{"attachments":[{"renditions":[{"digest_hex":"aa"}]},
                         {"renditions":[{"digest_hex":"cc"}]}]}'::jsonb, 'mirror')
     WHERE attachment_index = 1 AND rendition_index = 0;
    IF n <> 1 THEN
        RAISE EXCEPTION 'FAIL: the second attachment must be named index 1';
    END IF;

    -- INLINE renditions carry their bytes on the event: there is no lazy blob to fetch, and
    -- noting one would create a phantom present=FALSE row that never resolves.
    SELECT count(*) INTO n FROM cairn_by_reference_renditions(
        '{"attachments":[{"renditions":[{"inline":"AAAA"}]}]}'::jsonb, 'mirror');
    IF n <> 0 THEN
        RAISE EXCEPTION 'FAIL: an inline rendition must not be treated as by-reference, got %', n;
    END IF;

    RAISE NOTICE 'PASS: the shared traversal names distinct positions and skips inline renditions';
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
    v_event := mirror_new_event();

    PERFORM cairn_record_attachment_reference_flag(v_event, 0, 1, 'first');
    PERFORM cairn_record_attachment_reference_flag(v_event, 0, 1, 'again, same position');
    PERFORM cairn_record_attachment_reference_flag(v_event, NULL, NULL, 'not attributable');
    PERFORM cairn_record_attachment_reference_flag(v_event, NULL, NULL, 'not attributable again');
    -- the per-attachment shape: an index with NO rendition index. Dedupes on its own key.
    PERFORM cairn_record_attachment_reference_flag(v_event, 2, NULL, 'whole attachment');
    PERFORM cairn_record_attachment_reference_flag(v_event, 2, NULL, 'whole attachment again');

    SELECT count(*) INTO n FROM attachment_reference_flag WHERE event_id = v_event;
    IF n <> 3 THEN
        RAISE EXCEPTION 'FAIL: expected 3 deduped flag rows (positioned, per-attachment, '
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

    RAISE NOTICE 'PASS: the recorder dedupes on all three shapes and cannot raise';
END $$;

-- ---------------------------------------------------------------------------
-- 4. The read surface is reachable by BOTH group roles — and actually reports.
--
-- db/043's cairn_patient_deferred_sensitivity shipped granted to cairn_agent alone, described as
-- "the runtime role", which it is not — the runtime connects as a cairn_node member (#425). A
-- chart-scoped definer that only one of the two roles can call is a report nobody reads.
--
-- The grants are half the test. The other half is that it RETURNS the rows: replacing the body
-- with `WHERE false` left the first draft of this section entirely green, which is the Slice 69
-- finding (a mechanism nobody can look at) reproduced inside the test for the mechanism nobody
-- can look at.
-- ---------------------------------------------------------------------------
DO $$
DECLARE
    v_event uuid;
    v_other uuid;
    v_patient uuid;
    n int;
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

    -- It reports what was flagged, for the right chart only.
    v_event := mirror_new_event();
    v_other := mirror_new_event();
    SELECT patient_id INTO v_patient FROM event_log WHERE event_id = v_event;
    PERFORM cairn_record_attachment_reference_flag(v_event, 0, 0, 'the reported one');
    PERFORM cairn_record_attachment_reference_flag(v_other, 0, 0, 'a different chart');

    SELECT count(*) INTO n FROM cairn_patient_attachment_flags(v_patient);
    IF n <> 1 THEN
        RAISE EXCEPTION 'FAIL: the read surface returned % rows for a chart with exactly one '
                        'flag — a report that says nothing is the Slice 69 finding', n;
    END IF;

    SELECT count(*) INTO n FROM cairn_patient_attachment_flags(v_patient)
     WHERE reason = 'the reported one' AND event_type = 'note.added';
    IF n <> 1 THEN
        RAISE EXCEPTION 'FAIL: the read surface must name the reason and the event type';
    END IF;

    RAISE NOTICE 'PASS: the read surface is reachable by both group roles, not by PUBLIC, and reports its chart only';
END $$;

-- ---------------------------------------------------------------------------
-- 5. A malformed renditions LIST on one attachment never invalidates its siblings.
--
-- THE REGRESSION THIS SECTION EXISTS FOR. The first implementation put the list-shape catch
-- OUTSIDE the whole traversal loop. Because a PL/pgSQL set-returning function materialises its
-- entire tuplestore before the first row is returned, a non-list `renditions` on ANY attachment
-- aborted the traversal before a single reference was learned — so an event with one bad
-- attachment among three silently lost every good reference on the other two, and the one flag
-- row it left named no index at all.
--
-- That inverts the file's own governing rule (ADR-0060: a defect on one element never
-- invalidates the others) at the coarsest possible granularity, and it is invisible to every
-- Rust test because none of them can express a non-list `renditions` (see the header).
-- ---------------------------------------------------------------------------
DO $$
DECLARE
    v_event uuid;
    n int;
BEGIN
    v_event := mirror_new_event();
    PERFORM cairn_learn_attachment_refs_lenient(mirror_body(v_event, jsonb_build_array(
        jsonb_build_object('renditions', jsonb_build_array(
            jsonb_build_object('digest_hex', '1e20cc01', 'media_type', 'image/png'))),
        jsonb_build_object('renditions', 'not a list'),
        jsonb_build_object('renditions', jsonb_build_array(
            jsonb_build_object('digest_hex', '1e20cc02', 'media_type', 'image/png'))))));

    SELECT count(*) INTO n FROM blob_store
     WHERE blob_address IN (decode('1e20cc01', 'hex'), decode('1e20cc02', 'hex'));
    IF n <> 2 THEN
        RAISE EXCEPTION 'FAIL: a malformed renditions LIST on attachment 1 discarded % of the 2 '
                        'well-formed references on attachments 0 and 2. ADR-0060: a defect on '
                        'one element never invalidates the others — and a reference lost here is '
                        'lost for good, because the event is immutable and cairn_reproject '
                        'replays the dispatch, never the doors', 2 - n;
    END IF;

    -- and the fault is attributed to the attachment it belongs to, because that index EXISTS.
    SELECT count(*) INTO n FROM attachment_reference_flag
     WHERE event_id = v_event AND attachment_index = 1 AND rendition_index IS NULL;
    IF n <> 1 THEN
        RAISE EXCEPTION 'FAIL: a malformed renditions list must be flagged at (1, NULL) — NAME, '
                        'NEVER COUNT: the attachment index is in scope when the coercion fails, '
                        'so recording NULL there is a precise untruth, not an honest unknown';
    END IF;

    SELECT count(*) INTO n FROM attachment_reference_flag WHERE event_id = v_event;
    IF n <> 1 THEN
        RAISE EXCEPTION 'FAIL: exactly one flag expected, got % — the two good attachments must '
                        'not be flagged', n;
    END IF;

    RAISE NOTICE 'PASS: one malformed renditions list flags its own attachment and spares its siblings';
END $$;

-- ---------------------------------------------------------------------------
-- 6. A malformed attachments LIST is the one fault with no index to name.
--
-- This is the case NULLS NOT DISTINCT exists for, and the only one that legitimately records
-- NULL/NULL: the attachments value was not a list, so there is no attachment to point at.
-- Nothing is lost by aborting, because nothing was learnable in the first place.
-- ---------------------------------------------------------------------------
DO $$
DECLARE
    v_event uuid;
    n int;
BEGIN
    v_event := mirror_new_event();
    PERFORM cairn_learn_attachment_refs_lenient(mirror_body(v_event, '"not a list"'::jsonb));

    SELECT count(*) INTO n FROM attachment_reference_flag
     WHERE event_id = v_event AND attachment_index IS NULL AND rendition_index IS NULL;
    IF n <> 1 THEN
        RAISE EXCEPTION 'FAIL: a non-list attachments value must record exactly one '
                        'not-attributable flag, got %', n;
    END IF;

    -- Set-union sync re-offers the same bytes freely; re-delivery must dedupe rather than grow a
    -- row per delivery. With the default NULLS DISTINCT every re-offer would add a row forever.
    PERFORM cairn_learn_attachment_refs_lenient(mirror_body(v_event, '"not a list"'::jsonb));
    PERFORM cairn_learn_attachment_refs_lenient(mirror_body(v_event, '"not a list"'::jsonb));

    SELECT count(*) INTO n FROM attachment_reference_flag WHERE event_id = v_event;
    IF n <> 1 THEN
        RAISE EXCEPTION 'FAIL: re-delivering the same malformed body grew the ledger to % rows — '
                        'NULLS NOT DISTINCT on the dedup index is load-bearing for exactly this '
                        'case, and it is NOT the PostgreSQL default', n;
    END IF;

    RAISE NOTICE 'PASS: the not-attributable flag is recorded once and dedupes on re-delivery';
END $$;

-- ---------------------------------------------------------------------------
-- 7. The recorded reason carries the accessor's DETAIL, not just its headline.
--
-- Every accessor in db/027 puts the discriminating half of its refusal in `USING DETAIL` —
-- hex_decode_helper.rs states the standard: "its DETAIL says WHICH hex fault, because truncation
-- and wrong-encoding want opposite responses from whoever reads the log." SQLERRM returns the
-- primary message ONLY, so a ledger built on SQLERRM alone records the half that does not
-- discriminate and drops the half that does, while its header calls the text verbatim.
-- ---------------------------------------------------------------------------
DO $$
DECLARE
    v_event uuid;
    v_reason text;
BEGIN
    v_event := mirror_new_event();
    PERFORM cairn_learn_attachment_refs_lenient(mirror_body(v_event, jsonb_build_array(
        jsonb_build_object('renditions', jsonb_build_array(
            jsonb_build_object('digest_hex', '0xABC', 'media_type', 'image/png'))))));

    SELECT reason INTO v_reason FROM attachment_reference_flag WHERE event_id = v_event;
    IF v_reason IS NULL OR position('digest_hex' in v_reason) = 0 THEN
        RAISE EXCEPTION 'FAIL: the reason must name the field, got %', v_reason;
    END IF;
    IF position('not a hex digit' in v_reason) = 0 THEN
        RAISE EXCEPTION 'FAIL: the reason dropped the accessor DETAIL, which is the half that '
                        'tells truncation from wrong-encoding. Got: %', v_reason;
    END IF;

    RAISE NOTICE 'PASS: the recorded reason carries message AND detail';
END $$;

-- ---------------------------------------------------------------------------
-- 8. event_log.attachments is ALWAYS a jsonb array, at every door.
--
-- The apply door admits a body whose `attachments` is not a list (that is the whole point of
-- #460), so without a coercion at the insert the malformed value would be STORED — and every
-- reader that walks it with jsonb_array_elements raises 22023 on a scalar. The victim is
-- `read_photo_refs` (crates/cairn-node/src/patient/search.rs), which is the §5.3/§5.8
-- search-before-create funnel: one peer's malformed photo event would fail the whole candidate
-- list, i.e. the wrong-chart-prevention surface.
--
-- So the refusal is not eliminated, it is RELOCATED — out of a door that pens, names and reports
-- it, into a read path with no handling at all. The column is a projection for querying; the
-- authoritative body is `signed_bytes`, so coercing a non-list to `[]` loses nothing and the
-- ledger records that it was malformed.
-- ---------------------------------------------------------------------------
DO $$
DECLARE
    v_bad boolean;
BEGIN
    IF cairn_json_list_or_empty('"scalar"'::jsonb) <> '[]'::jsonb
       OR cairn_json_list_or_empty('{"a":1}'::jsonb) <> '[]'::jsonb
       OR cairn_json_list_or_empty(NULL) <> '[]'::jsonb
       OR cairn_json_list_or_empty('null'::jsonb) <> '[]'::jsonb
       OR cairn_json_list_or_empty('[1,2]'::jsonb) <> '[1,2]'::jsonb THEN
        RAISE EXCEPTION 'FAIL: cairn_json_list_or_empty must be TOTAL — every non-array becomes '
                        'the empty array and an array passes through unchanged';
    END IF;

    -- The constraint is the floor behind the coercion: with both doors coercing, a non-array can
    -- only arrive through a CODE defect, and this makes that defect loud instead of silent.
    SELECT count(*) = 0 INTO v_bad FROM pg_constraint
     WHERE conrelid = 'event_log'::regclass AND conname = 'event_attachments_is_a_list';
    IF v_bad THEN
        RAISE EXCEPTION 'FAIL: event_log has no constraint pinning attachments to a jsonb array';
    END IF;

    RAISE NOTICE 'PASS: the attachments column is coerced at the doors and constrained in the table';
END $$;

-- ---------------------------------------------------------------------------
-- 9. Both learners run the SAME traversal — asserted at the source, not assumed.
--
-- db/050, db/020, HANDOVER and ROADMAP all state that the strict and lenient learners share
-- their accessors AND their traversal, and that this is WHY "malformed" cannot come to mean two
-- different things at the two doors. That sentence is the stated mechanism for the file's
-- central safety property, so it needs a guard rather than a reader's trust: the first
-- implementation asserted it in four files while the strict learner still carried its own
-- hand-written nested loop.
--
-- A wrong safety argument is worse than none — it disarms the guard it describes.
-- ---------------------------------------------------------------------------
DO $$
DECLARE
    n int;
BEGIN
    SELECT count(*) INTO n
      FROM pg_proc p
     WHERE p.proname IN ('cairn_learn_attachment_refs', 'cairn_learn_attachment_refs_lenient')
       AND p.prosrc LIKE '%cairn_by_reference_renditions%';
    IF n <> 2 THEN
        RAISE EXCEPTION 'FAIL: only % of the 2 learners call cairn_by_reference_renditions. The '
                        'traversal is duplicated, so the anti-drift guarantee db/050 and db/020 '
                        'both claim does not exist — either make it true or delete the claim', n;
    END IF;
    RAISE NOTICE 'PASS: both learners run the one shared traversal';
END $$;

-- ---------------------------------------------------------------------------
-- 10. The ledger is APPEND-ONLY to the roles that can read it.
--
-- cairn_agent holds SELECT so tooling can read the report. It must NOT hold INSERT/UPDATE/DELETE:
-- an agent that can fabricate a flag can accuse a peer of sending garbage it never sent, and one
-- that can delete a flag erases the only evidence that a reference is unlearnable. The ledger is
-- written by the door, through a definer, and by nothing else.
-- ---------------------------------------------------------------------------
DO $$
DECLARE
    priv text;
BEGIN
    IF NOT has_table_privilege('cairn_agent', 'attachment_reference_flag', 'SELECT') THEN
        RAISE EXCEPTION 'FAIL: cairn_agent cannot read the ledger it is meant to report on';
    END IF;
    FOREACH priv IN ARRAY ARRAY['INSERT', 'UPDATE', 'DELETE'] LOOP
        IF has_table_privilege('cairn_agent', 'attachment_reference_flag', priv) THEN
            RAISE EXCEPTION 'FAIL: cairn_agent holds % on attachment_reference_flag — a role that '
                            'can write this ledger can fabricate an accusation against a peer, and '
                            'one that can delete from it erases the evidence a reference is '
                            'unlearnable', priv;
        END IF;
    END LOOP;
    RAISE NOTICE 'PASS: the ledger is readable but not writable by cairn_agent';
END $$;

-- ---------------------------------------------------------------------------
-- 11. The NODE-WIDE report exists, reports, and is reachable by both group roles.
--
-- The chart-scoped read requires you to already know which chart to ask about, but a malformed
-- reference is discovered FROM the ledger. db/040 pairs its cairn_agent-only flag table with a
-- node-wide cairn_clock_health() granted to both roles; this is the same shape.
-- ---------------------------------------------------------------------------
DO $$
DECLARE
    v_event uuid;
    n int;
BEGIN
    IF to_regprocedure('cairn_attachment_flag_health()') IS NULL THEN
        RAISE EXCEPTION 'FAIL: the node-wide report is missing — the chart-scoped read cannot be '
                        'the only surface, because it asks you to already know the answer';
    END IF;
    IF NOT has_function_privilege('cairn_agent', 'cairn_attachment_flag_health()', 'EXECUTE')
       OR NOT has_function_privilege('cairn_node', 'cairn_attachment_flag_health()', 'EXECUTE') THEN
        RAISE EXCEPTION 'FAIL: both group roles must reach the node-wide report (#425)';
    END IF;
    IF has_function_privilege('public', 'cairn_attachment_flag_health()', 'EXECUTE') THEN
        RAISE EXCEPTION 'FAIL: EXECUTE is still granted to PUBLIC on a SECURITY DEFINER read';
    END IF;

    v_event := mirror_new_event();
    PERFORM cairn_record_attachment_reference_flag(v_event, 0, 0, 'a distinctive mirror reason');

    -- It NAMES: an example event and the reason, never a bare count.
    SELECT count(*) INTO n FROM cairn_attachment_flag_health()
     WHERE reason = 'a distinctive mirror reason'
       AND event_type = 'note.added' AND flagged >= 1 AND example_event IS NOT NULL;
    IF n <> 1 THEN
        RAISE EXCEPTION 'FAIL: the node-wide report must name the reason, the event type and an '
                        'example event — "47 flags" cannot tell an operator whether to chase a '
                        'peer''s encoder bug or one corrupted import';
    END IF;

    RAISE NOTICE 'PASS: the node-wide report names what it counts and is reachable by both group roles';
END $$;

DROP FUNCTION mirror_new_event();
DROP FUNCTION mirror_body(uuid, jsonb);
