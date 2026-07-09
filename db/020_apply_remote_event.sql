-- db/020_apply_remote_event.sql
-- Cairn — the in-DB clinical-plane sync apply door (issue #91; review A2/A5b/M8/H4).
--
-- WHAT: `apply_remote_event` — the ONE door through which a replicated clinical event
-- enters `event_log`. The sibling of `apply_remote_node_event` (db/007, the node-event
-- plane's gate) and of `submit_event` (db/005, the local authoring door). Before this
-- file, the sync daemon verified a signature in Rust and raw-INSERTed with owner
-- privileges — bypassing actor enrollment, fail-closed classification, the attestation
-- gate on suppressing events, the demographic hard-twin rule, the t_effective rules,
-- and the event-id substitution guard. ADR-0021 places the enforcement floor BELOW the
-- inter-node path; this door is that placement made real for the clinical plane.
--
-- ONE floor, two doors: every deterministic check submit_event runs, this door runs
-- identically (same helper functions — cairn_t_effective, cairn_event_twin — so the
-- floors cannot drift). The replication-appropriate DELTAS, each reasoned:
--
--   * idempotent re-apply is a silent no-op (set-union), identical to submit_event;
--   * the local HLC merges forward past every accepted event (the A3 invariant,
--     mirrored from apply_remote_node_event) — the daemon no longer touches hlc_state;
--   * the attestation token for a suppressing event TRAVELS with it on the sync wire
--     (stored by db/001's additive columns, shipped by the daemon, re-verified here);
--   * projection maintenance must never veto a validly-signed event peers accepted:
--     this door raises the transaction-local `cairn.remote_apply` marker, and any
--     node-local-config projection guard (db/018 component cap) CLAMPS-AND-FLAGS
--     instead of RAISE-ing (review A5b). Note the distinction: the door's OWN checks
--     (signature, enrollment, classification, attestation, twin, t_effective) are
--     deterministic functions of the signed bytes — every honest node computes the
--     same verdict, so refusing cannot fork the fleet; a GUC-dependent guard is not,
--     so it must not refuse.
--
-- KNOWN LIMITATION (deliberate, documented): actor enrollment is resolved against the
-- LOCAL registry (actor_current), exactly as at the authoring door. Actor-registry
-- replication is not yet built (ADR-0011 future work), so today an event authored on a
-- peer applies only once its signer is enrolled here too (an operator ceremony). A
-- refused-but-valid event freezes the puller's watermark (cairn-sync A1 discipline)
-- and is retried each cycle, so enrollment lag delays — never loses — an event.

BEGIN;

-- The sync runtime role (created by db/007 on full nodes; created here too so the
-- walking-skeleton schema subset 001-006 + 020 stands alone).
DO $$ BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'cairn_node') THEN
        CREATE ROLE cairn_node NOLOGIN;
    END IF;
END $$;

CREATE OR REPLACE FUNCTION apply_remote_event(
    p_signed       BYTEA,
    p_attestation  BYTEA DEFAULT NULL,
    p_attester_key BYTEA DEFAULT NULL
) RETURNS UUID
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = public
AS $$
DECLARE
    b               JSONB;
    v_event_id      UUID;
    v_ca            BYTEA;
    v_type          TEXT;
    v_mode          TEXT;
    v_targets_other BOOLEAN;
    v_bears         BOOLEAN;
    v_target_id     UUID;
    v_twin          TEXT;
    v_t_eff         TIMESTAMPTZ;
    v_att           BYTEA;
    v_att_key       BYTEA;
    v_actor_ids     BYTEA[];
    v_actor_id      BYTEA;
    v_rows          INTEGER;
    v_merge_wall    BIGINT;
BEGIN
    -- 0. Size ceiling (A7a): an oversized event would wedge the 8 MiB-capped wire and
    --    backup paths at its seq forever; refuse before any crypto work.
    IF octet_length(p_signed) > cairn_max_event_bytes() THEN
        RAISE EXCEPTION 'apply_remote_event: event is % bytes, over the % -byte admission ceiling (would wedge sync/backup)',
            octet_length(p_signed), cairn_max_event_bytes();
    END IF;

    -- 1. Signature floor: the in-DB pgrx gate, unbypassable even for a caller with
    --    direct DB access (the whole point of moving apply in-DB).
    IF NOT cairn_verify(p_signed) THEN
        -- Legible reason as DETAIL (issue #109): a context mismatch (a peer still on the
        -- pre-ADR-0040 wire format) reads very differently from tampering. cairn-sync's
        -- do_pull independently re-derives the same reason in Rust (verify_self_described)
        -- for its quarantine pen, so the pen is legible even without this; the DETAIL is the
        -- SQL-boundary counterpart, surfaced to a direct psql caller and carried into
        -- apply_signed's error text for every other caller.
        RAISE EXCEPTION 'apply_remote_event: signature verification failed (unsigned or malformed event)'
            USING DETAIL = coalesce(cairn_verify_error(p_signed), 'unknown');
    END IF;
    b := cairn_body(p_signed);
    IF b IS NULL THEN
        RAISE EXCEPTION 'apply_remote_event: event body could not be parsed after verify';
    END IF;

    v_event_id := (b ->> 'event_id')::uuid;
    v_type     := b ->> 'event_type';
    -- content_address = sha256 multihash of the signed wire bytes, identical to
    -- event_address() in cairn-event and the db/001 CHECK.
    v_ca       := '\x1220'::bytea || digest(p_signed, 'sha256');

    -- 1b. t_effective wire pin (H4) + bitemporal tier-1 ceiling (ADR-0003 §3.6), via
    --     the same db/001 validator submit_event uses. Both checks are deterministic
    --     functions of the signed bytes: every honest node refuses the same events.
    v_t_eff := cairn_t_effective(b ->> 't_effective');
    IF v_t_eff IS NOT NULL
       AND v_t_eff > to_timestamp((b -> 'hlc' ->> 'wall')::bigint / 1000.0) THEN
        RAISE EXCEPTION 'apply_remote_event: t_effective (%) is after t_recorded ceiling (HLC wall % ms) — prima-facie forward-dating / falsification (ADR-0003 tier-1)',
            b ->> 't_effective', b -> 'hlc' ->> 'wall';
    END IF;

    -- 2. Resolve the signer against the actor registry (must be enrolled, non-revoked)
    --    and RECORD the resolution (issue #99). The admission GATE is actor_current,
    --    exactly as at the authoring door. The attribution STAMP, though, must be
    --    resolved against the key's ENTIRE local registry history, not its current
    --    state: a replicated event was authored under whatever epoch its origin node
    --    held AT AUTHORING TIME, which this node cannot know (the signed bytes carry
    --    only signer_key_id — the ADR-0029 refinement that would fix this is future
    --    work). Stamping the merely-current actor would misattribute an old-epoch
    --    event that arrives after a local epoch bump — silent recall under-selection,
    --    the exact #99 failure. So: unique stamp only when the key has only ever
    --    meant ONE actor on this node; otherwise NULL (honest unknown, principle 4;
    --    over-selected at recall, never missed). Node-local derived state — the
    --    signed bytes are untouched, so set-union convergence is unaffected.
    --    See the KNOWN LIMITATION note in the header: local registry, by design for now.
    IF NOT EXISTS (SELECT 1 FROM actor_current WHERE signing_key_id = b ->> 'signer_key_id') THEN
        RAISE EXCEPTION 'apply_remote_event: signer % is not an enrolled, non-revoked actor', b ->> 'signer_key_id';
    END IF;
    SELECT array_agg(DISTINCT ae.actor_id) INTO v_actor_ids
        FROM actor_event ae
        WHERE ae.op IN ('enroll','supersede')
          AND ae.signing_key_id = b ->> 'signer_key_id';
    v_actor_id := CASE WHEN array_length(v_actor_ids, 1) = 1 THEN v_actor_ids[1] END;

    -- 3. Classify (fail closed on unknown type; ADR-0010/ADR-0012 — an older node
    --    refuses a type it cannot classify rather than guessing its mode).
    SELECT mode, targets_other_author INTO v_mode, v_targets_other
        FROM event_type_class WHERE event_type = v_type;
    IF v_mode IS NULL THEN
        RAISE EXCEPTION 'apply_remote_event: unknown event_type % (no classification — fail closed)', v_type;
    END IF;

    v_bears := EXISTS (
        SELECT 1 FROM jsonb_array_elements(b -> 'contributors') AS e
        WHERE e ? 'responsibility');

    -- 4. Attestation gate. A suppressing event, or any asserted responsibility,
    --    is admitted only against a valid human attestation token bound to THIS
    --    event's content-address. The token travelled with the event on the sync
    --    wire (db/001 columns); a peer that ships a suppress without one is refused —
    --    the exact hole review A2 flagged (un-attested visibility.suppress synced in).
    IF v_mode = 'suppressing' OR v_bears THEN
        IF p_attestation IS NULL OR p_attester_key IS NULL THEN
            RAISE EXCEPTION 'apply_remote_event: % requires attestation (no token travelled with the event) — un-vouched suppress/responsibility refused', v_type;
        END IF;
        IF NOT cairn_attestation_ok(p_attestation, v_ca, p_attester_key) THEN
            RAISE EXCEPTION 'apply_remote_event: attestation token invalid or not bound to this event';
        END IF;
        IF NOT EXISTS (SELECT 1 FROM actor_current
                       WHERE signing_key_id = encode(p_attester_key,'hex') AND kind = 'human') THEN
            RAISE EXCEPTION 'apply_remote_event: attester is not an enrolled human actor (forged human author refused)';
        END IF;
        v_att     := p_attestation;
        v_att_key := p_attester_key;
    END IF;

    -- 5. Target-existence gate for an overlay on another author's event. Safe at
    --    apply because HLC order is causal: a suppress is authored by someone who
    --    HELD the target, so the target sorts earlier and (on this full-replication
    --    plane) arrives first; a suppress whose target is still in flight from
    --    another link freezes the watermark and retries until the target lands.
    IF v_targets_other AND (b -> 'payload' ? 'target_event_id') THEN
        v_target_id := (b -> 'payload' ->> 'target_event_id')::uuid;
        IF NOT EXISTS (SELECT 1 FROM event_log WHERE event_id = v_target_id) THEN
            RAISE EXCEPTION 'apply_remote_event: overlay targets unknown event %', v_target_id;
        END IF;

        -- ADR-0043 owner-gate (shared helper — see db/005): a replicated cross-human
        -- suppress faces the SAME refusal a locally-authored one does (principle 12).
        -- p_attester_key is non-NULL here (step 4 refused a suppress with no token).
        IF v_mode = 'suppressing'
           AND NOT cairn_suppression_author_ok(v_target_id, p_attester_key) THEN
            RAISE EXCEPTION 'apply_remote_event: cross-author suppression refused — a suppress of another human''s event may not be admitted; disagreement is additive. (ADR-0043)';
        END IF;
    END IF;

    -- 6. Provenance binding (C3): an advisory must cite its source blob's address.
    IF v_type = 'advisory.added' THEN
        IF jsonb_array_length(COALESCE(b -> 'attachments', '[]'::jsonb)) = 0 THEN
            RAISE EXCEPTION 'apply_remote_event: advisory.added must carry a provenance attachment reference';
        END IF;
    END IF;

    -- 7. Plaintext twin + per-type structural floor, via the SAME cairn_event_twin
    --    hook as submit_event — one floor renderer, so a twin-less demographic event
    --    is refused identically at both doors (closes the M8 asymmetry), and a
    --    twin-less generic event derives the identical skeleton at both doors.
    v_twin := cairn_event_twin(v_type, b);

    -- Raise the transaction-local remote-apply marker so projection triggers with
    -- node-local-config guards clamp-and-flag instead of vetoing (A5b; db/018 reads
    -- it). Cleared right after the INSERT (AFTER-ROW triggers run within the INSERT
    -- statement), so a later submit_event in the same transaction keeps its veto.
    PERFORM set_config('cairn.remote_apply', 'on', true);

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
    -- Capture the insert outcome BEFORE the set_config below: PERFORM overwrites
    -- FOUND, which would silently disable the substitution guard.
    GET DIAGNOSTICS v_rows = ROW_COUNT;

    PERFORM set_config('cairn.remote_apply', '', true);

    -- Idempotent re-apply of the SAME event is a silent no-op (set-union). A
    -- DIFFERENT event reusing this event_id is a substitution — two nodes holding
    -- different bytes under one event_id would diverge forever with no alarm, so it
    -- must RAISE (review H3; identical to the submit_event guard).
    IF v_rows = 0 THEN
        IF (SELECT content_address FROM event_log WHERE event_id = v_event_id) <> v_ca THEN
            RAISE EXCEPTION 'apply_remote_event: event_id % already exists with different content (substitution refused)', v_event_id;
        END IF;
    END IF;

    -- Learn any attachment references, per rendition (reference-eager, byte-lazy; ADR-0013,
    -- rendition set per ADR-0042). Shared with the submit door via cairn_learn_attachment_refs
    -- (db/027) so the two doors never drift.
    PERFORM cairn_learn_attachment_refs(b);

    -- HLC merge with a clock-drift clamp (issue #102): the local clock never falls behind an
    -- event we accepted (the A3 invariant), BUT a remote wall implausibly far in our future is
    -- clamped to now + cairn_max_hlc_drift_ms() (db/001) before it advances hlc_state, so a
    -- broken or hostile peer cannot ratchet the clinical clock without bound. This door CLAMPS
    -- where the node door (db/007) REJECTS, and the difference is forced by the pull loops:
    -- cairn-sync FREEZES its watermark on ANY refusal of a verifiable event (main.rs), so
    -- rejecting a future-dated clinical event would let one insane peer event WEDGE clinical
    -- replication — an availability regression worse than the ratchet (availability over
    -- consistency). The event itself is admitted UNCHANGED above, its original asserted wall
    -- preserved verbatim in event_log (principle 1: never rewrite the claim); only the
    -- local-clock side-effect is bounded here. (An admitted future wall still orders "latest"
    -- in projections exactly as it does today — a pre-existing, orthogonal concern, not
    -- worsened by this clamp; see issue #97.) The A3 invariant is intentionally relaxed for a
    -- Byzantine future-claim: Cairn contains dishonest events with signatures + recall, not by
    -- dragging every honest node's clock to the lie.
    v_merge_wall := LEAST((b -> 'hlc' ->> 'wall')::bigint,
                          (extract(epoch FROM clock_timestamp()) * 1000)::bigint + cairn_max_hlc_drift_ms());
    UPDATE hlc_state SET
        hlc_wall    = GREATEST(hlc_wall, v_merge_wall),
        hlc_counter = CASE
            WHEN v_merge_wall > hlc_wall THEN (b -> 'hlc' ->> 'counter')::int
            WHEN v_merge_wall = hlc_wall THEN GREATEST(hlc_counter, (b -> 'hlc' ->> 'counter')::int)
            ELSE hlc_counter END
        WHERE id;

    RETURN v_event_id;
END;
$$;

-- The grant floor (ADR-0021). Only the sync runtime role may drive the replication
-- door; the authoring agent role may not (privilege gradient — an agent authors via
-- submit_event, it does not impersonate the sync plane). PUBLIC's default EXECUTE on
-- a new function would bypass the table REVOKEs, so close it explicitly.
REVOKE EXECUTE ON FUNCTION apply_remote_event(bytea, bytea, bytea) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION apply_remote_event(bytea, bytea, bytea) TO cairn_node;
-- The sync role reads the log to SERVE events (and never writes it raw).
REVOKE INSERT, UPDATE, DELETE ON event_log FROM cairn_node;
GRANT SELECT ON event_log TO cairn_node;

COMMIT;
