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

-- Skeleton plaintext twin: the mechanical §3.13 fallback rendering. Kept as its own
-- helper so the per-type twin hook below can fall back to it without duplicating the
-- format. TODO: spec §3.13/ADR-0012 want the clinical payload rendered too.
CREATE OR REPLACE FUNCTION cairn_twin_skeleton(p_type text, b jsonb)
RETURNS text LANGUAGE sql IMMUTABLE AS $$
    SELECT format('[%s] %s for patient %s', p_type, b ->> 'schema_version', b ->> 'patient_id');
$$;

-- Per-event-type twin hook (§3.13/§4.5). Returns the plaintext legibility twin for an
-- event and, for a type that has one, enforces its structural floor (raising on
-- violation). The DEFAULT delegates every type to the skeleton; a later migration
-- CREATE OR REPLACEs this to add its own branch WITHOUT re-declaring the whole
-- validated submit_event door (so the safety-critical surface stays single-source).
CREATE OR REPLACE FUNCTION cairn_event_twin(p_type text, b jsonb)
RETURNS text LANGUAGE plpgsql AS $$
BEGIN
    RETURN cairn_twin_skeleton(p_type, b);
END;
$$;

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
    v_twin         TEXT;
    c              JSONB;
BEGIN
    -- 0. Size ceiling (review fix A7a): refuse an oversized event BEFORE the crypto work,
    --    so an event too large to replicate or back up can never be admitted (it would
    --    otherwise wedge sync at its seq forever). See cairn_max_event_bytes() (db/001).
    IF octet_length(p_signed) > cairn_max_event_bytes() THEN
        RAISE EXCEPTION 'submit_event: event is % bytes, over the % -byte admission ceiling (would wedge sync/backup)',
            octet_length(p_signed), cairn_max_event_bytes();
    END IF;

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
    -- content_address = sha256 of the signed wire bytes (the COSE envelope), identical to event_address() in cairn-event and the db/001 CHECK. (Distinct from canonical_json_address, which hashes the actor pinned-set body for actor_id.) Attestation tokens bind to THIS value.
    v_ca       := '\x1220'::bytea || digest(p_signed, 'sha256');

    -- 1b. Bitemporal tier-1 ceiling (ADR-0003 §3.6): t_recorded (the HLC wall) is the
    --     OBJECTIVE ceiling; t_effective is the freely-BACKDATABLE claim. Backdating is
    --     legitimate (t_effective in the past); forward-dating past t_recorded is not —
    --     a node cannot have "recorded" a fact before its own clock reached that instant,
    --     so t_effective > t_recorded is prima-facie falsification and is rejected here (a
    --     signed envelope invariant, not soft policy). NOTE: the string->timestamptz cast
    --     of an offset-less t_effective is session-TimeZone dependent (see issue: pin the
    --     t_effective wire format to an explicit offset); the comparison of two absolute
    --     instants below is itself timezone-independent.
    IF NULLIF(b ->> 't_effective','null') IS NOT NULL
       AND (b ->> 't_effective')::timestamptz
           > to_timestamp((b -> 'hlc' ->> 'wall')::bigint / 1000.0) THEN
        RAISE EXCEPTION 'submit_event: t_effective (%) is after t_recorded ceiling (HLC wall % ms) — prima-facie forward-dating / falsification (ADR-0003 tier-1)',
            b ->> 't_effective', b -> 'hlc' ->> 'wall';
    END IF;

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

    -- 5. Target-existence gate for an overlay on another author's event.
    --    (The skeleton stores the target in the body as `target_event_id`.)
    --
    --    DEFERRED (known limitation, not a fix): this does NOT verify that the
    --    attester is *entitled* to suppress THIS target. Step 4 only requires
    --    *some* enrolled human attester, so any human could downgrade any author's
    --    event. Real owner/authority semantics (target-author vs attester, role
    --    authority, delegation) are an ADR-level design question, not a spike hack;
    --    it is therefore left explicit here. C5.5 only demonstrates the *un-attested*
    --    cross-author downgrade is refused, which is the attestation gate, not an
    --    ownership check.
    IF v_targets_other AND (b -> 'payload' ? 'target_event_id') THEN
        v_target_id := (b -> 'payload' ->> 'target_event_id')::uuid;
        IF NOT EXISTS (SELECT 1 FROM event_log WHERE event_id = v_target_id) THEN
            RAISE EXCEPTION 'submit_event: overlay targets unknown event %', v_target_id;
        END IF;
    END IF;

    -- 6. Provenance binding (C3): an advisory must cite its source blob's address.
    IF v_type = 'advisory.added' THEN
        IF jsonb_array_length(COALESCE(b -> 'attachments', '[]'::jsonb)) = 0 THEN
            RAISE EXCEPTION 'submit_event: advisory.added must carry a provenance attachment reference';
        END IF;
    END IF;

    -- 7. Plaintext twin (§3.13/§4.5) + any per-type structural floor, via the
    --    cairn_event_twin hook so a new event type adds its branch there, not by
    --    re-declaring this whole door.
    v_twin := cairn_event_twin(v_type, b);

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
    ON CONFLICT (event_id) DO NOTHING;

    -- Idempotent re-submit of the SAME event is a silent no-op (set-union).
    -- But a DIFFERENT event reusing this event_id (substitution) must not pass
    -- silently: compare the stored content-address to what we just verified.
    IF NOT FOUND THEN
        IF (SELECT content_address FROM event_log WHERE event_id = v_event_id) <> v_ca THEN
            RAISE EXCEPTION 'submit_event: event_id % already exists with different content (substitution refused)', v_event_id;
        END IF;
    END IF;

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
-- The classification table is itself a safety surface: reclassifying a
-- suppressing op as additive would dodge the attestation gate. Lock it down;
-- submit_event reads it as its SECURITY DEFINER owner, so cairn_agent needs nothing.
REVOKE INSERT, UPDATE, DELETE ON event_type_class FROM PUBLIC;
-- submit_event is SECURITY DEFINER, so PUBLIC's default EXECUTE on a new function
-- would let *any* connected role drive the privileged write door (bypassing the
-- table REVOKEs above). Close that: only cairn_agent may knock.
REVOKE EXECUTE ON FUNCTION submit_event(bytea, bytea, bytea) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION submit_event(bytea, bytea, bytea) TO cairn_agent;
GRANT SELECT ON event_log, patient_chart, actor_current TO cairn_agent;

COMMIT;
