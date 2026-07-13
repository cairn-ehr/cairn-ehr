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

-- Per-type twin/floor-check registry (#173, ADR-0048). Sibling of event_type_class:
-- a new event type registers its structural check + twin requirement by INSERTing ONE
-- row here (additive), instead of copying the whole cairn_event_twin dispatch chain into
-- a new migration. The single stable dispatcher (below) reads this table. Columns are
-- independent: check_fn NULL ⇒ no structural floor for this type; twin_required_msg NULL
-- ⇒ an absent authored twin degrades honestly to a skeleton (ADR-0039) rather than raising.
CREATE TABLE IF NOT EXISTS cairn_event_twin_check (
    event_type         TEXT PRIMARY KEY,
    check_fn           TEXT,
    twin_required_msg  TEXT
);

-- Fail-closed at REGISTRATION time (not first-call): a registered check_fn must exist with
-- the unified (text, jsonb) signature. A slice that registers a typo'd or not-yet-created
-- check fn fails loudly on schema load, for this migration and every future one, with
-- nothing to remember. (to_regprocedure returns NULL for an absent function; valid type
-- names never raise.) Residual: this validates registration, not later function mutation —
-- a migration that broke a check fn's signature afterwards would surface at runtime
-- (the dispatcher's EXECUTE raises, still fail-closed).
CREATE OR REPLACE FUNCTION cairn_check_twin_registry_fn()
RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.check_fn IS NOT NULL
       AND to_regprocedure(NEW.check_fn || '(text, jsonb)') IS NULL THEN
        RAISE EXCEPTION 'cairn_event_twin_check: check_fn %(text, jsonb) does not exist (fail closed)', NEW.check_fn;
    END IF;
    RETURN NEW;
END;
$$;

DROP TRIGGER IF EXISTS cairn_event_twin_check_validate ON cairn_event_twin_check;
CREATE TRIGGER cairn_event_twin_check_validate
    BEFORE INSERT OR UPDATE ON cairn_event_twin_check
    FOR EACH ROW EXECUTE FUNCTION cairn_check_twin_registry_fn();

-- Safety surface (like event_type_class): a row pointing a type's check at a no-op would
-- drop its floor. Lock it down; submit_event reads it as its SECURITY DEFINER owner, so
-- cairn_agent needs no grant.
REVOKE INSERT, UPDATE, DELETE ON cairn_event_twin_check FROM PUBLIC;

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

-- Suppression owner-gate (ADR-0043 / issue #99). A suppressing overlay
-- (salience.downgrade / visibility.suppress) that forecloses on a HUMAN author's
-- event is self-only: only that human may suppress it. Cross-human suppression is
-- refused — disagreement is expressed additively (a note referencing the target),
-- never by touching another author's content (principle 1/2, paper-parity).
-- An agent-authored / un-owned advisory (no responsible human) stays dismissable by
-- any enrolled human — the clinician-overrides-the-machine path (principle 10), NOT
-- the burying of a colleague.
--
-- The target's human authors = {signer_key_id if it EVER resolved to a kind='human'
-- actor} ∪ {hex(attester_key) if a human attestation is stored}. Empty set ⇒
-- agent/un-owned ⇒ permitted. Non-empty ⇒ permitted only if the attester is in it.
-- STABLE (reads event_log + actor_event). Shared by BOTH doors so a replicated
-- cross-human suppress faces the identical refusal (principle 12). Safe direction:
-- an unknown/ambiguous attester on human-authored content refuses, never permits.
--
-- Signer human-ness is resolved from the append-only actor_event HISTORY, not
-- actor_current — mirroring the discipline db/020 step 2 already uses for stamping.
-- Authorship is an immutable historical fact: a plain note.added stores no
-- attester_key, so its ONLY human-author signal is the signer's registry kind AT
-- AUTHORING TIME. If a departed/rotated author's key later drops out of
-- actor_current (revoke, or supersede onto a new key), querying actor_current here
-- would silently empty the human-author set and flip the gate open — over-permission
-- on the safety floor (any enrolled human could then suppress a departed colleague's
-- notes), which contradicts ADR-0043's never-over-permission invariant. actor_event
-- is append-only, so this branch is monotonic: a key that was ever human stays human
-- for this check forever. Wrong direction is over-refusal, never over-permission.
CREATE OR REPLACE FUNCTION cairn_suppression_author_ok(p_target UUID, p_attester_key BYTEA)
RETURNS boolean LANGUAGE sql STABLE AS $$
    WITH tgt AS (
        SELECT el.signer_key_id, el.attester_key
        FROM event_log el WHERE el.event_id = p_target
    ),
    human_authors AS (
        SELECT t.signer_key_id AS kid FROM tgt t
        WHERE EXISTS (SELECT 1 FROM actor_event ae
                      WHERE ae.signing_key_id = t.signer_key_id
                        AND ae.op IN ('enroll','supersede')
                        AND ae.kind = 'human')
        UNION
        SELECT encode(t.attester_key, 'hex') FROM tgt t
        WHERE t.attester_key IS NOT NULL
    )
    SELECT NOT EXISTS (SELECT 1 FROM human_authors)
        OR EXISTS (SELECT 1 FROM human_authors h WHERE h.kid = encode(p_attester_key, 'hex'));
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
    v_t_eff        TIMESTAMPTZ;
    v_att          BYTEA;
    v_att_key      BYTEA;
    v_actor_ids    BYTEA[];
    v_actor_id     BYTEA;
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
        -- Keep the boolean floor; attach the legible reason as DETAIL so an operator can
        -- tell a wire-format skew / pre-ADR-0040 context mismatch from actual tampering
        -- (issue #109). cairn_verify already returned false, so cairn_verify_error is
        -- non-NULL here; the coalesce guards only the impossible NULL case.
        RAISE EXCEPTION 'submit_event: signature verification failed (unsigned or malformed event)'
            USING DETAIL = coalesce(cairn_verify_error(p_signed), 'unknown');
    END IF;
    b := cairn_body(p_signed);
    IF b IS NULL THEN
        RAISE EXCEPTION 'submit_event: event body could not be parsed after verify';
    END IF;

    v_event_id := (b ->> 'event_id')::uuid;
    v_type     := b ->> 'event_type';
    -- content_address = sha256 of the signed wire bytes (the COSE envelope), identical to event_address() in cairn-event and the db/001 CHECK. (Distinct from canonical_json_address, which hashes the actor pinned-set body for actor_id.) Attestation tokens bind to THIS value.
    v_ca       := '\x1220'::bytea || digest(p_signed, 'sha256');

    -- 1b. t_effective wire pin (issue #91/H4): parse the asserted claim through the ONE
    --     explicit-offset validator (db/001 cairn_t_effective), so the stored instant is
    --     identical on every node regardless of session TimeZone/DateStyle.
    v_t_eff := cairn_t_effective(b ->> 't_effective');

    --     Bitemporal tier-1 ceiling (ADR-0003 §3.6): t_recorded (the HLC wall) is the
    --     OBJECTIVE ceiling; t_effective is the freely-BACKDATABLE claim. Backdating is
    --     legitimate (t_effective in the past); forward-dating past t_recorded is not —
    --     a node cannot have "recorded" a fact before its own clock reached that instant,
    --     so t_effective > t_recorded is prima-facie falsification and is rejected here (a
    --     signed envelope invariant, not soft policy).
    IF v_t_eff IS NOT NULL
       AND v_t_eff > to_timestamp((b -> 'hlc' ->> 'wall')::bigint / 1000.0) THEN
        RAISE EXCEPTION 'submit_event: t_effective (%) is after t_recorded ceiling (HLC wall % ms) — prima-facie forward-dating / falsification (ADR-0003 tier-1)',
            b ->> 't_effective', b -> 'hlc' ->> 'wall';
    END IF;

    -- 2. Resolve the signer against the actor registry (must be enrolled, non-revoked)
    --    and RECORD the resolution (issue #99): a unique key->actor mapping stamps the
    --    admitting actor_id on the row, so a later contamination-cascade recall selects
    --    this event exactly even after the key is re-enrolled under a new skill_epoch.
    --    A key concurrently registered to several actors stamps NULL — attribution
    --    honestly unknown (principle 4) — and the recall query (db/006) over-selects
    --    NULL rows rather than ever missing one.
    SELECT array_agg(DISTINCT actor_id) INTO v_actor_ids
        FROM actor_current WHERE signing_key_id = b ->> 'signer_key_id';
    IF v_actor_ids IS NULL THEN
        RAISE EXCEPTION 'submit_event: signer % is not an enrolled, non-revoked actor', b ->> 'signer_key_id';
    END IF;
    v_actor_id := CASE WHEN array_length(v_actor_ids, 1) = 1 THEN v_actor_ids[1] END;

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
        -- Store the VERIFIED responsibility proof beside the event (issue #91/M7):
        -- it must keep travelling with the event on the sync wire, or a downstream
        -- node could never re-run this gate at its own apply door.
        v_att     := p_attestation;
        v_att_key := p_attester_key;
    END IF;

    -- 5. Target-existence gate for an overlay on another author's event.
    --    (The skeleton stores the target in the body as `target_event_id`.)
    --
    --    The owner-gate (was DEFERRED here; now closed) is enforced below by
    --    cairn_suppression_author_ok (ADR-0043 / issue #99).
    IF v_targets_other AND (b -> 'payload' ? 'target_event_id') THEN
        v_target_id := (b -> 'payload' ->> 'target_event_id')::uuid;
        IF NOT EXISTS (SELECT 1 FROM event_log WHERE event_id = v_target_id) THEN
            RAISE EXCEPTION 'submit_event: overlay targets unknown event %', v_target_id;
        END IF;

        -- ADR-0043 owner-gate: a suppressing overlay of a HUMAN author's event is
        -- self-only. Cross-human suppression is refused; express disagreement
        -- additively. (Agent advisories are un-owned ⇒ cairn_suppression_author_ok
        -- returns TRUE ⇒ dismissable.) p_attester_key is non-NULL here: step 4
        -- already refused a suppressing event without a valid human token.
        IF v_mode = 'suppressing'
           AND NOT cairn_suppression_author_ok(v_target_id, p_attester_key) THEN
            RAISE EXCEPTION 'submit_event: cross-author suppression refused — you may only suppress your own events; express disagreement additively (a note referencing the target). (ADR-0043)';
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
         signer_key_id, plaintext_twin, attachments, attestation, attester_key, actor_id)
    VALUES (
        v_event_id, (b ->> 'patient_id')::uuid, v_type, b ->> 'schema_version',
        (b -> 'hlc' ->> 'wall')::bigint, (b -> 'hlc' ->> 'counter')::int,
        b -> 'hlc' ->> 'node_origin',
        v_t_eff,
        p_signed, v_ca, b -> 'payload', b -> 'contributors',
        b ->> 'signer_key_id', v_twin, COALESCE(b -> 'attachments','[]'::jsonb),
        v_att, v_att_key, v_actor_id)
    ON CONFLICT (event_id) DO NOTHING;

    -- Idempotent re-submit of the SAME event is a silent no-op (set-union).
    -- But a DIFFERENT event reusing this event_id (substitution) must not pass
    -- silently: compare the stored content-address to what we just verified.
    IF NOT FOUND THEN
        IF (SELECT content_address FROM event_log WHERE event_id = v_event_id) <> v_ca THEN
            RAISE EXCEPTION 'submit_event: event_id % already exists with different content (substitution refused)', v_event_id;
        END IF;
    END IF;

    -- Learn any attachment references, per rendition (reference-eager, byte-lazy).
    -- Shared with the remote-apply door via cairn_learn_attachment_refs (db/027) so the
    -- two doors never drift.
    PERFORM cairn_learn_attachment_refs(b);

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
