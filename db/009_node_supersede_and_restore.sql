-- Cairn — node-level supersede + self-trusting restore (ADR-0026 slice C).
--
-- WHY: slice B exports a node's signed node_event set to a cold-peer medium; this is
-- the APPLY half. Restoring a node's OWN history into a fresh DB cannot use the live
-- apply_remote_node_event gate (that is the PEER-admission path — it rejects events
-- whose author is not an already-trusted peer, which a fresh node has none of). So we
-- add a SELF-TRUSTING restore door, fenced so it is a permanent no-op on a live node,
-- plus the node-level `supersede` op (a restored node mints a NEW key — the signing key
-- is never backed up — and records supersede(dead -> new), already the actor-algebra
-- shape for agents, now applied to nodes). See ADR-0026 §7.10 points 1/2/4.

BEGIN;

-- (1) Widen the op CHECK additively (ADR-0012): a superset rejects nothing previously
-- accepted. The constraint is the auto-named column CHECK from db/007's CREATE TABLE.
ALTER TABLE node_event DROP CONSTRAINT IF EXISTS node_event_op_check;
ALTER TABLE node_event ADD CONSTRAINT node_event_op_check
    CHECK (op IN ('enroll','peer','revoke','supersede'));

-- (2) The supersede lineage view: who superseded whom. Read by `status`/audit. A
-- supersede event's author is the NEW (live) node; its subject is the dead node-id.
CREATE OR REPLACE VIEW node_lineage AS
SELECT ne.subject_node_id AS superseded_node_id,
       ne.author_node_id  AS new_node_id,
       ne.hlc_wall, ne.hlc_counter, ne.recorded_at
FROM node_event ne
WHERE ne.op = 'supersede';

GRANT SELECT ON node_lineage TO cairn_node;

-- (3) The self-trusting restore door. Unlike apply_remote_node_event (the PEER-admission
-- gate), this applies a node's OWN history into a fresh DB WITHOUT a peer-trust check —
-- a fresh node has no trust set yet. The danger (a federation-admission bypass) is closed
-- structurally: the door fails closed unless local_node is empty, so on any LIVE node it
-- is a permanent no-op. Signature + content-address ARE enforced, so a tampered/bit-rotted
-- medium event is rejected exactly as a hostile peer would be (ADR-0026 point 2). The door
-- NEVER writes local_node — only a real new genesis (submit_node_event) does, and that is
-- what permanently fences this door closed at the end of a restore.
CREATE OR REPLACE FUNCTION restore_node_event(p_signed BYTEA)
RETURNS UUID
LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, pg_temp
AS $$
DECLARE
    b JSONB; v_type TEXT; v_op TEXT; v_ca BYTEA; v_eid UUID; v_signer TEXT;
    v_payload JSONB; v_author_node BYTEA; v_subject BYTEA;
    -- The content-address ALREADY stored under v_eid, read back after the INSERT so a
    -- substitution can be told from an idempotent repeat (#615; see the guard at the tail).
    v_found BYTEA;
BEGIN
    -- FENCE: restore is only into a fresh, un-enrolled node.
    IF EXISTS (SELECT 1 FROM local_node WHERE id) THEN
        RAISE EXCEPTION 'restore_node_event: node already enrolled; restore applies only into a fresh node (live admission is apply_remote_node_event)';
    END IF;
    -- Size ceiling (review fix A7a): keep the ceiling consistent across every door,
    -- including the restore path, so a restored image can never smuggle in an oversized
    -- event that later wedges this node's outbound sync.
    IF octet_length(p_signed) > cairn_max_event_bytes() THEN
        RAISE EXCEPTION 'restore_node_event: event is % bytes, over the % -byte admission ceiling',
            octet_length(p_signed), cairn_max_event_bytes();
    END IF;
    IF NOT cairn_verify(p_signed) THEN
        -- Legible reason as DETAIL (issue #109): tells a wire-format skew apart from tampering.
        RAISE EXCEPTION 'restore_node_event: signature verification failed'
            USING DETAIL = coalesce(cairn_verify_error(p_signed), 'unknown');
    END IF;
    b := cairn_body(p_signed);
    -- Clock-drift ceiling (issue #193, mirroring db/007's node door — the THIRD
    -- signed-bytes admission door gets the same bound). Restore is self-trusting (any
    -- signed enroll applies; a fresh node has no trust set) and the hlc_state merge at
    -- the end of this door (cairn_node_hlc_merge, db/001) is monotone by construction —
    -- so ONE attacker-appended event on the sneakernet
    -- medium carrying an absurd future wall would ratchet the fresh node's clock into
    -- the far future, and every event it subsequently authors would be rejected by
    -- every peer's drift ceiling: the node is wedged out of the federation with no
    -- self-healing path. The "own authored events are exempt" argument does not cover
    -- restore — the medium can contain OTHER signers' events and is attacker-
    -- appendable. Measured against clock_timestamp(), never hlc_state, so the bound
    -- cannot itself be ratcheted. A refusal here fails that one medium event; a
    -- genuine medium (walls in the past, or honest skew inside the ceiling) restores
    -- unaffected.
    IF (b -> 'hlc' ->> 'wall')::bigint
           > (extract(epoch FROM clock_timestamp()) * 1000)::bigint + cairn_max_hlc_drift_ms() THEN
        RAISE EXCEPTION 'restore_node_event: HLC wall % ms is more than % ms ahead of local time — clock-drift ceiling (issue #193)',
            (b -> 'hlc' ->> 'wall')::bigint, cairn_max_hlc_drift_ms();
    END IF;
    v_type := b ->> 'event_type';
    -- P0001 for every malformed field (#621), like the two live doors. This door aborts the
    -- WHOLE restore on any raise, so here the helpers buy legibility rather than availability:
    -- an operator reading a failed restore is told which field of which event the medium got
    -- wrong, instead of PostgreSQL's bare "invalid input syntax for type uuid".
    v_eid := cairn_uuid_or_raise('event_id', b ->> 'event_id', 'restore_node_event');
    v_signer := b ->> 'signer_key_id'; v_payload := b -> 'payload';
    v_ca := '\x1220'::bytea || digest(p_signed, 'sha256');
    PERFORM cairn_hlc_nonneg_or_raise((b -> 'hlc' ->> 'wall')::bigint,
                                      (b -> 'hlc' ->> 'counter')::int, 'restore_node_event');
    v_op := CASE v_type WHEN 'node.enrolled' THEN 'enroll' WHEN 'peer.added' THEN 'peer'
                        WHEN 'peer.revoked' THEN 'revoke' WHEN 'node.superseded' THEN 'supersede'
                        ELSE NULL END;
    IF v_op IS NULL THEN
        RAISE EXCEPTION 'restore_node_event: unknown node event_type % (fail closed)', v_type;
    END IF;

    IF v_op = 'enroll' THEN
        INSERT INTO node_event (node_event_id, op, author_node_id, subject_node_id,
            signer_key_id, hlc_wall, hlc_counter, node_origin, signed_bytes, content_address)
        VALUES (v_eid, 'enroll', v_ca, v_ca, v_signer,
            (b -> 'hlc' ->> 'wall')::bigint, (b -> 'hlc' ->> 'counter')::int,
            b -> 'hlc' ->> 'node_origin', p_signed, v_ca)
        ON CONFLICT (node_event_id) DO NOTHING;
    ELSE
        -- The node's own enroll is restored first (medium seq order), so its key resolves.
        SELECT node_id INTO v_author_node FROM node_current WHERE signer_key_id = v_signer;
        IF v_author_node IS NULL THEN
            RAISE EXCEPTION 'restore_node_event: author key % maps to no restored enroll (apply genesis first)', v_signer;
        END IF;
        -- Both the MISSING and the MALFORMED case are refused by name here (issue #228).
        -- This site used to decode bare and then test `v_subject IS NULL`, which caught
        -- only the missing case, reported it as the generic "missing subject node id"
        -- (never saying WHICH field), and could not catch the malformed one at all — a
        -- payload carrying "0xABC" raised PostgreSQL's own hex error from inside the
        -- CASE, before the guard was reached. The helper covers both and names the field,
        -- so the old guard below it is gone rather than left as unreachable code.
        --
        -- One thing IS given up, deliberately, and db/007 keeps its own NULL guards
        -- precisely to avoid giving it up: the old message interpolated v_type. Here that
        -- costs nothing worth a guard, because the FIELD NAME already carries the op —
        -- superseded_node_id_hex can only be node.superseded, peer_node_id_hex only a
        -- peer.added/peer.revoked — and added-vs-revoked tells nobody anything useful
        -- about a malformed subject. db/007's REMOTE door additionally names the authoring
        -- peer (its local door has no peer to name, and neither has restore, which reads a
        -- sneakernet medium) — that is the context worth a hand-written guard, and it is
        -- context this door does not have.
        v_subject := CASE v_op
            WHEN 'supersede' THEN cairn_decode_hex_or_raise('superseded_node_id_hex',
                v_payload ->> 'superseded_node_id_hex', 'restore_node_event')
            ELSE cairn_decode_hex_or_raise('peer_node_id_hex',
                v_payload ->> 'peer_node_id_hex', 'restore_node_event') END;
        INSERT INTO node_event (node_event_id, op, author_node_id, subject_node_id,
            signer_key_id, peer_pubkey, fingerprint, role, scope_hint, target_event_id,
            hlc_wall, hlc_counter, node_origin, signed_bytes, content_address)
        VALUES (v_eid, v_op, v_author_node, v_subject,
            v_signer, v_payload ->> 'peer_pubkey', v_payload ->> 'fingerprint',
            cairn_node_role_or_raise(v_payload ->> 'role', 'restore_node_event'),
            v_payload ->> 'scope_hint',
            CASE WHEN NULLIF(v_payload ->> 'target_event_id','') IS NULL THEN NULL
                 ELSE cairn_uuid_or_raise('target_event_id',
                        v_payload ->> 'target_event_id', 'restore_node_event') END,
            (b -> 'hlc' ->> 'wall')::bigint, (b -> 'hlc' ->> 'counter')::int,
            b -> 'hlc' ->> 'node_origin', p_signed, v_ca)
        ON CONFLICT (node_event_id) DO NOTHING;
    END IF;

    -- SUBSTITUTION REFUSAL (#615). Both branches above insert ON CONFLICT DO NOTHING, which is
    -- what makes a re-restore of the same medium a no-op — and is also what made a SECOND,
    -- DIFFERENT event under one node_event_id vanish without a word. db/005 and db/020 have
    -- refused this since their first review; this door is the one that did not.
    --
    -- WHY IT MATTERS MOST HERE. The node plane is the TRUST SET, and the silently-dropped event
    -- can be the clinic's own `peer.revoked`: the restored node then comes back trusting a peer
    -- the clinic had revoked, while the summary reads `restored N event(s)` at exit 0. The count
    -- cannot catch it — apply_medium returns the number of events OFFERED, by its own doc. And
    -- this door is self-trusting, which is precisely why the medium is the reachable attack
    -- surface: see the drift-ceiling comment above, which already establishes that the medium
    -- "can contain OTHER signers' events and is attacker-appendable".
    --
    -- THREE THINGS ABOUT THE SHAPE, each chosen rather than inherited from the other two doors:
    --   * NO GET DIAGNOSTICS. A ROW_COUNT check placed here would be correct only because the
    --     last statement of BOTH branches happens to be the INSERT. Someone later adding a
    --     statement inside either branch would disarm the guard SILENTLY — the exact failure
    --     db/020's own comment warns about. Reading the row back has no such coupling.
    --   * FAIL-CLOSED ON AN ABSENT ROW. If the read finds nothing, v_found is NULL and
    --     cairn_refuse_substitution's IS DISTINCT FROM refuses. That state should be
    --     unreachable; on the §9 surface "should be unreachable" is not a reason to pass (#608).
    --   * ONCE, NOT TWICE. Both branches write node_event under the same key, so one site covers
    --     both and there is no second copy to drift — which is the whole lesson of #608, where
    --     one invariant written twice came to be wrong in both places at once.
    --
    -- COST: one extra SELECT per NODE-plane event. A medium carries tens of those (enrolls,
    -- peers, revokes, supersedes), not the 100 003 clinical records the §1.2 restore budget was
    -- measured against — which is why db/005 and db/020 keep their ROW_COUNT check and this door
    -- does not need one.
    --
    -- A REFUSAL ABORTS THE WHOLE RESTORE, because apply_medium propagates with `?`. That is not
    -- a new posture: this door already aborts on an unknown node event type, an over-ceiling
    -- event, an HLC wall past the drift ceiling, and an author key resolving to no restored
    -- enroll. A medium carrying two rival events under one id is a compromised or corrupt
    -- medium, and restoring a node whose peer list was decided by whichever copy the medium
    -- ordered first — which whoever can append to the medium controls — is a worse outcome than
    -- refusing and sending the operator to find another copy.
    SELECT content_address INTO v_found FROM node_event WHERE node_event_id = v_eid;
    PERFORM cairn_refuse_substitution(v_found, v_ca, v_eid, 'restore_node_event');

    -- Clock never falls behind a restored event (HLC invariant A3, mirrors the apply path).
    -- The clock-drift REJECTION near the top of this function is this door's ceiling; the helper
    -- (db/001) is the pure merge —
    -- and the merge being monotone is exactly why that ceiling has to sit in front of it.
    PERFORM cairn_node_hlc_merge((b -> 'hlc' ->> 'wall')::bigint,
                                 (b -> 'hlc' ->> 'counter')::int);
    RETURN v_eid;
END;
$$;

REVOKE EXECUTE ON FUNCTION restore_node_event(bytea) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION restore_node_event(bytea) TO cairn_node;

COMMIT;
