-- db/tests/043_deferred_readjudication_test.sql
-- ADR-0056 decisions 1 + 4 (issues #265/#266) — the SQL mirror of the floor.
--
-- SCOPE. These assertions cover the parts of the contract that are pure schema: the marker
-- table's shape, the replay gate reading it, the classified-before-projected registration
-- guard, and the privilege tier on the promotion function. Signature-dependent behaviour
-- (the door admitting a real signed event, a token surviving defer→promote) lives in
-- crates/cairn-node/tests/deferred_admission.rs, which can sign; SQL alone cannot.
--
-- Runs inside a transaction that ROLLBACKs, so it leaves no residue — same discipline as
-- db/tests/039_projection_registry_test.sql.
BEGIN;

-- 1. The marker table exists, and is 1:1 with event_log (PK on event_id). Without the PK a
--    second admission of the same event could double-mark it, and promotion would delete
--    only one row — leaving the event permanently replay-ineligible.
DO $$
BEGIN
    IF to_regclass('public.event_deferred') IS NULL THEN
        RAISE EXCEPTION 'event_deferred is missing — ADR-0056 has no EXPLICIT deferred state, and the corollary forbids inferring it from a null classification lookup';
    END IF;
    IF NOT EXISTS (
        SELECT 1 FROM pg_index i
        JOIN pg_class cl ON cl.oid = i.indrelid
        WHERE cl.relname = 'event_deferred' AND i.indisprimary
    ) THEN
        RAISE EXCEPTION 'event_deferred has no primary key — the marker must be 1:1 with event_log';
    END IF;
    -- adjudication_error IS decision 4's "flagged legibly"; a schema edit dropping it would
    -- silently turn a recorded refusal into an invisible one.
    IF NOT EXISTS (
        SELECT 1 FROM information_schema.columns
        WHERE table_name = 'event_deferred' AND column_name = 'adjudication_error'
    ) THEN
        RAISE EXCEPTION 'event_deferred.adjudication_error is missing — a failed re-adjudication would have nowhere legible to be recorded';
    END IF;
END $$;

-- 1b. The carried-not-vouched marker exists and is 1:1 with event_log. Without the PK a
--     double-admission could double-mark, and a single clearing DELETE would leave a row
--     behind — pinning a genuinely-vouched token as unvouched forever, which reads as
--     over-refusal on the ADR-0043 floor rather than the over-permission F2 was.
DO $$
BEGIN
    IF to_regclass('public.event_attestation_unvouched') IS NULL THEN
        RAISE EXCEPTION 'event_attestation_unvouched is missing — an unverified carried token would be indistinguishable from a verified vouch once promotion deletes the event_deferred marker (PR #302 review finding F2)';
    END IF;
    IF NOT EXISTS (
        SELECT 1 FROM pg_index i
        JOIN pg_class cl ON cl.oid = i.indrelid
        WHERE cl.relname = 'event_attestation_unvouched' AND i.indisprimary
    ) THEN
        RAISE EXCEPTION 'event_attestation_unvouched has no primary key — the marker must be 1:1 with event_log';
    END IF;
    -- The shared predicate, in BOTH directions. A helper stuck at TRUE would silently
    -- re-open F2 at all four call sites at once, which is exactly the blast radius that
    -- makes sharing it worthwhile — and worth pinning.
    IF NOT cairn_attestation_vouched(gen_random_uuid()) THEN
        RAISE EXCEPTION 'cairn_attestation_vouched returned FALSE for an unmarked event — every stored attestation would be treated as unvouched, disabling the ADR-0043 owner-gate''s attester arm entirely';
    END IF;
    DECLARE v_probe uuid;
    BEGIN
        SELECT event_id INTO v_probe FROM event_log LIMIT 1;
        IF v_probe IS NOT NULL THEN
            INSERT INTO event_attestation_unvouched (event_id) VALUES (v_probe)
            ON CONFLICT DO NOTHING;
            IF cairn_attestation_vouched(v_probe) THEN
                RAISE EXCEPTION 'cairn_attestation_vouched returned TRUE for a MARKED event — an unverified carried token would count as a real vouch (PR #302 finding F2)';
            END IF;
        END IF;
    END;
END $$;

-- 2. The projection registry refuses an UNCLASSIFIED event type (fail closed). This guard is
--    one of the two legs keeping a deferred event unprojected: the AFTER-INSERT dispatcher
--    reads cairn_projection_apply and never consults event_type_class, so an unclassified
--    type registered here would be projected at admission.
DO $$
DECLARE v_ok boolean := false;
BEGIN
    BEGIN
        INSERT INTO cairn_projection_apply (event_type, apply_fn, projection_tables)
        VALUES ('unclassified.sql.mirror', 'patient_chart_apply', ARRAY['patient_chart']);
    EXCEPTION WHEN OTHERS THEN
        IF SQLERRM LIKE '%not classified in event_type_class%' THEN
            v_ok := true;
        ELSE
            RAISE EXCEPTION 'wrong refusal for an unclassified projection registration: %', SQLERRM;
        END IF;
    END;
    IF NOT v_ok THEN
        RAISE EXCEPTION 'cairn_projection_apply accepted an UNCLASSIFIED event_type — the dispatcher could project an event admitted uninterpreted';
    END IF;
END $$;

-- 3. cairn_replay_eligible reads the marker, in BOTH directions. The FALSE direction is what
--    stops a reprojection granting unadjudicated power; the TRUE direction matters just as
--    much — a predicate stuck at FALSE would silently stop replay healing healthy events,
--    and every heal would quietly do nothing.
DO $$
DECLARE
    v_id   uuid := uuidv7();
    v_sb   bytea;
    v_elig boolean;
BEGIN
    v_sb := ('replay-gate-' || v_id::text)::bytea;
    INSERT INTO event_log (event_id, patient_id, event_type, schema_version,
        hlc_wall, hlc_counter, node_origin, signed_bytes, content_address,
        body, contributors, signer_key_id, plaintext_twin)
    VALUES (v_id, v_id, 'replay.gate.probe', 'test-1',
        (extract(epoch from now()) * 1000)::bigint, 0, 'test-node', v_sb,
        '\x1220'::bytea || digest(v_sb, 'sha256'),
        '{}'::jsonb, '[]'::jsonb, 'test-key', 'probe');
    INSERT INTO event_deferred (event_id, event_type) VALUES (v_id, 'replay.gate.probe');

    SELECT cairn_replay_eligible(el) INTO v_elig FROM event_log el WHERE el.event_id = v_id;
    IF v_elig THEN
        RAISE EXCEPTION 'cairn_replay_eligible returned TRUE for a DEFERRED event — reprojection could grant power that never passed a classification-gated floor check';
    END IF;

    DELETE FROM event_deferred WHERE event_id = v_id;
    SELECT cairn_replay_eligible(el) INTO v_elig FROM event_log el WHERE el.event_id = v_id;
    IF NOT v_elig THEN
        RAISE EXCEPTION 'cairn_replay_eligible returned FALSE for a NON-deferred event — replay would skip healthy events and every heal would silently do nothing';
    END IF;
END $$;

-- 4. The promotion pass is OWNER-ONLY, like cairn_reproject (db/039). It grants power to
--    events admitted without it, so the runtime role must not be able to call it: a
--    compromised runtime role could otherwise promote an event the floor refused.
DO $$
BEGIN
    IF has_function_privilege('cairn_node', 'cairn_readjudicate_deferred()', 'EXECUTE') THEN
        RAISE EXCEPTION 'cairn_node can EXECUTE cairn_readjudicate_deferred — the runtime role must not be able to grant power to a deferred event';
    END IF;
END $$;

ROLLBACK;
