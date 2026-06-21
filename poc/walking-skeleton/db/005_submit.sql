-- Cairn walking skeleton — the validated submit surface (Spike 0002 §4.4 / ADR-0022).
--
-- submit_event is the ONE generic write door. It runs the write-time seams in-DB,
-- atomically: verify (cairn_pgx) -> resolve actor -> classify additive/suppressing
-- -> gate attestation -> owner-gate cross-author overlays -> bind provenance ->
-- append. The grant floor (REVOKE INSERT on event_log; GRANT EXECUTE here) makes
-- direct DB access safe by construction (ADR-0021). Every rejection is legible.

BEGIN;

-- Additive vs suppressing classification (ADR-0010). A new event type adds a row
-- here (additive-only registry); unknown types are rejected (fail closed).
CREATE TABLE IF NOT EXISTS event_type_class (
    event_type            TEXT PRIMARY KEY,
    mode                  TEXT NOT NULL CHECK (mode IN ('additive','suppressing')),
    targets_other_author  BOOLEAN NOT NULL DEFAULT FALSE
);
INSERT INTO event_type_class (event_type, mode, targets_other_author) VALUES
    ('patient.created', 'additive',    FALSE),
    ('patient.amended', 'additive',    FALSE),
    ('note.added',      'additive',    FALSE),
    ('advisory.added',  'additive',    FALSE),
    ('salience.downgrade','suppressing', TRUE),
    ('visibility.suppress','suppressing', TRUE)
ON CONFLICT (event_type) DO NOTHING;

CREATE OR REPLACE FUNCTION submit_event(
    p_signed       BYTEA,
    p_attestation  BYTEA DEFAULT NULL,
    p_attester_key BYTEA DEFAULT NULL
) RETURNS UUID
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = public
AS $$
DECLARE
    b              JSONB;
    v_event_id     UUID;
    v_ca           BYTEA;
    v_type         TEXT;
    v_mode         TEXT;
    v_targets_other BOOLEAN;
    v_bears        BOOLEAN;
    v_target_id    UUID;
    v_target_origin TEXT;
    v_twin         TEXT;
    c              JSONB;
BEGIN
    -- 1. Signature floor (C5.1). cairn_verify is the in-DB pgrx gate.
    IF NOT cairn_verify(p_signed) THEN
        RAISE EXCEPTION 'submit_event: signature verification failed (unsigned or malformed event)';
    END IF;
    b := cairn_body(p_signed);
    IF b IS NULL THEN
        RAISE EXCEPTION 'submit_event: event body could not be parsed after verify';
    END IF;

    v_event_id := (b ->> 'event_id')::uuid;
    v_type     := b ->> 'event_type';
    v_ca       := '\x1220'::bytea || digest(p_signed, 'sha256');

    -- 2. Resolve the signer against the actor registry (must be enrolled, non-revoked).
    IF NOT EXISTS (SELECT 1 FROM actor_current WHERE signing_key_id = b ->> 'signer_key_id') THEN
        RAISE EXCEPTION 'submit_event: signer % is not an enrolled, non-revoked actor', b ->> 'signer_key_id';
    END IF;

    -- 3. Classify (fail closed on unknown type).
    SELECT mode, targets_other_author INTO v_mode, v_targets_other
        FROM event_type_class WHERE event_type = v_type;
    IF v_mode IS NULL THEN
        RAISE EXCEPTION 'submit_event: unknown event_type % (no classification — fail closed)', v_type;
    END IF;

    -- Does any contributor claim a responsibility (bearing role with attestation)?
    v_bears := EXISTS (
        SELECT 1 FROM jsonb_array_elements(b -> 'contributors') AS e
        WHERE e ? 'responsibility');

    -- 4. Attestation gate. A suppressing event, OR any asserted responsibility,
    --    requires a valid attestation token bound to THIS event (C2, C5.2, C5.3).
    IF v_mode = 'suppressing' OR v_bears THEN
        IF p_attestation IS NULL OR p_attester_key IS NULL THEN
            RAISE EXCEPTION 'submit_event: % requires attestation (no token presented) — un-vouched suppress/responsibility refused', v_type;
        END IF;
        IF NOT cairn_attestation_ok(p_attestation, v_ca, p_attester_key) THEN
            RAISE EXCEPTION 'submit_event: attestation token invalid or not bound to this event';
        END IF;
        IF NOT EXISTS (SELECT 1 FROM actor_current
                       WHERE signing_key_id = encode(p_attester_key,'hex') AND kind = 'human') THEN
            RAISE EXCEPTION 'submit_event: attester is not an enrolled human actor (forged human author refused)';
        END IF;
    END IF;

    -- 5. Owner-gate: a suppressing overlay that targets another author's event must
    --    be attested by a human (already enforced in step 4); record the linkage.
    --    (The skeleton stores the target in the body as `target_event_id`.)
    IF v_targets_other AND (b -> 'payload' ? 'target_event_id') THEN
        v_target_id := (b -> 'payload' ->> 'target_event_id')::uuid;
        SELECT node_origin INTO v_target_origin FROM event_log WHERE event_id = v_target_id;
        IF v_target_origin IS NULL THEN
            RAISE EXCEPTION 'submit_event: overlay targets unknown event %', v_target_id;
        END IF;
    END IF;

    -- 6. Provenance binding (C3): an advisory must cite its source blob's address.
    IF v_type = 'advisory.added' THEN
        IF jsonb_array_length(COALESCE(b -> 'attachments', '[]'::jsonb)) = 0 THEN
            RAISE EXCEPTION 'submit_event: advisory.added must carry a provenance attachment reference';
        END IF;
    END IF;

    -- 7. Derive the plaintext twin (mechanical; the §3.13 substrate) and append.
    v_twin := format('[%s] %s for patient %s', v_type, b ->> 'schema_version', b ->> 'patient_id');

    INSERT INTO event_log
        (event_id, patient_id, event_type, schema_version, hlc_wall, hlc_counter,
         node_origin, t_effective, signed_bytes, content_address, body, contributors,
         signer_key_id, plaintext_twin, attachments)
    VALUES (
        v_event_id, (b ->> 'patient_id')::uuid, v_type, b ->> 'schema_version',
        (b -> 'hlc' ->> 'wall')::bigint, (b -> 'hlc' ->> 'counter')::int,
        b -> 'hlc' ->> 'node_origin',
        NULLIF(b ->> 't_effective','null')::timestamptz,
        p_signed, v_ca, b -> 'payload', b -> 'contributors',
        b ->> 'signer_key_id', v_twin, COALESCE(b -> 'attachments','[]'::jsonb))
    ON CONFLICT DO NOTHING;

    -- Learn any attachment references (reference-eager, byte-lazy).
    FOR c IN SELECT * FROM jsonb_array_elements(COALESCE(b -> 'attachments','[]'::jsonb)) LOOP
        PERFORM blob_note_reference(decode(c ->> 'digest_hex','hex'), c ->> 'media_type',
                                    (c ->> 'byte_len')::bigint);
    END LOOP;

    RETURN v_event_id;
END;
$$;

-- The grant floor (C5.4 / ADR-0021): no direct event_log writes; the only door is
-- submit_event. The agent reads projections + the log, executes the door, nothing else.
REVOKE INSERT, UPDATE, DELETE ON event_log FROM PUBLIC;
REVOKE INSERT, UPDATE, DELETE ON event_log FROM cairn_agent;
GRANT EXECUTE ON FUNCTION submit_event(bytea, bytea, bytea) TO cairn_agent;
GRANT SELECT ON event_log, patient_chart, actor_current TO cairn_agent;

COMMIT;
