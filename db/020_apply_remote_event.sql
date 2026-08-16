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
--     instead of RAISE-ing (review A5b). Most of the door's OWN checks (signature,
--     enrollment, classification, attestation, t_effective) are deterministic
--     functions of the signed bytes — every honest node computes the same verdict,
--     so refusing cannot fork the fleet. `twin` is the ONE exception (since ADR-0059,
--     db/041): the per-type structural floor it dispatches to may carry a lenient,
--     node-local sub-tier (a coding-vocabulary registry a peer may run newer or
--     locally-extended) that must RETURN rather than refuse on this door — the same
--     GUC-like-dependency reasoning as a projection guard, just living one layer
--     earlier, inside `cairn_event_twin`'s dispatch instead of after it. A future
--     per-type check_fn is free to add its own such sub-tier; it must not assume the
--     whole twin dispatch is refusal-safe to fork on.
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

-- ADR-0052: the door gained p_dek (the sidecar DEK for a sealed event). A
-- CREATE OR REPLACE with a different arg list would OVERLOAD (3-arg + 4-arg →
-- ambiguous 1/2/3-arg calls), so drop the old signature first, exactly as db/005
-- does for submit_event. Idempotent across replays. Every existing caller passes
-- ≤ 3 args (the daemon's apply_remote_event($1,$2,$3), the walking-skeleton
-- apply_remote_event($1)); those resolve to this 4-arg version with p_dek
-- defaulting NULL, so no caller changes.
DROP FUNCTION IF EXISTS apply_remote_event(bytea, bytea, bytea);

CREATE OR REPLACE FUNCTION apply_remote_event(
    p_signed       BYTEA,
    p_attestation  BYTEA DEFAULT NULL,
    p_attester_key BYTEA DEFAULT NULL,
    p_dek          BYTEA DEFAULT NULL
) RETURNS UUID
LANGUAGE plpgsql
SECURITY DEFINER
SET search_path = public, pg_temp
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
    v_grade         text;              -- ADR-0058 born clock-confidence grade (issue #216)
    v_verdict       text;              -- cairn_ceiling_classify result: ok | flag | reject
    v_att           BYTEA;
    v_att_key       BYTEA;
    v_actor_ids     BYTEA[];
    v_actor_id      BYTEA;
    v_rows          INTEGER;
    v_merge_wall    BIGINT;
    -- ADR-0052 lenient sealed arm (mirror of db/005's DECLARE additions).
    v_sealed        BOOLEAN := false;  -- did the body arrive as the sealed container?
    b_clear         JSONB;             -- the CLEAR view floor checks + projections run on
    v_inner         JSONB;             -- {payload, plaintext_twin} recovered by cairn_unseal_body
    v_pub           BYTEA;             -- this node's X25519 unwrap-key public half
    v_twin_stub     TEXT;              -- the outer, signed mechanical stub twin (principle 11)
    -- ADR-0056 decision 1 (issue #265): true when this node holds no classification for the
    -- event's type. The event is ADMITTED anyway — custody is total, power is deferred.
    v_deferred      BOOLEAN := false;
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

    -- 1b. t_effective wire pin (H4), via the same db/001 validator submit_event uses:
    --     deterministic on the signed bytes, so every honest node parses the same instant.
    v_t_eff := cairn_t_effective(b ->> 't_effective');

    -- 1b'. Grade-gated ceiling (ADR-0058). LENIENT door: NEVER reject on the ceiling — a
    --      refusal of a verifiable event freezes the puller's seq watermark and WEDGES
    --      clinical sync (issue #216 F1/F2), the same rule the HLC-drift clamp further
    --      down already honors. Admit UNCHANGED; a flag/reject verdict is recorded as an
    --      advisory clash row. Absent grade (foreign/pre-slice) → 'unknown'; a future
    --      grade is admitted verbatim and ranks 0 in the classifier (safe).
    v_grade := COALESCE(b ->> 'clock_grade', 'unknown');
    v_verdict := cairn_ceiling_classify((b -> 'hlc' ->> 'wall')::bigint, v_grade, v_t_eff);
    IF v_verdict IN ('flag', 'reject') THEN
        PERFORM cairn_record_ceiling_flag(v_ca, (b -> 'hlc' ->> 'wall')::bigint, v_t_eff, v_grade, v_verdict);
    END IF;

    -- 1c. Contributor-set floor (ADR-0051, issues #203/#96): the LENIENT door — role
    --     membership NEVER rejects here (set-union losslessness: a future vocabulary
    --     member arrives partition-prefixed and classifies by its prefix; a wholly-
    --     unknown role degrades to vouching-unknown at read time, it never excludes
    --     content). Only the never-lawful shapes refuse — see cairn_check_contributors.
    PERFORM cairn_check_contributors(b, 'apply_remote_event', false);

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

    -- 3. Classify — and ADMIT-AND-DEFER when we cannot (ADR-0056 decision 1, issue #265).
    --
    --    This door used to RAISE here. That made sync.md §6.5's lossless-forwarding
    --    invariant FALSE for unknown types: a phone-tier node carrying a chart between two
    --    upgraded facilities (the §6.1 sneakernet path, the case Cairn exists for) acquired
    --    NOTHING past the first unknown-type event — the event was not merely unrendered,
    --    it was absent. Admission cannot hide anything; refusal can.
    --
    --    A deferred event is stored verbatim, re-propagated, exported, and rendered down
    --    the §3.13 legibility ladder by the skeleton twin. Step 8 needs no change for it:
    --    cairn_event_twin finds no cairn_event_twin_check row for an unregistered type, so
    --    both its check_fn and its twin_required_msg are NULL and it falls through to
    --    cairn_twin_skeleton — it never raises.
    --
    --    It yields NO projection rows and confers NO power. Two independent mechanisms
    --    enforce that, and BOTH are needed: db/005's classified-before-projected
    --    registration guard (so the AFTER-INSERT dispatcher has nothing registered to run
    --    for an unclassified type), and cairn_replay_eligible (so no reprojection path can
    --    pick it up later). Power is granted only by cairn_readjudicate_deferred (db/043),
    --    which re-runs the gates skipped below.
    --
    --    The STRICT door (db/005) deliberately still fails closed: a node may CARRY a type
    --    it has no code for, never AUTHOR one (decision 2 — ADR-0051's strict-submit/
    --    lenient-apply asymmetry applied to types, which is also what keeps classification
    --    an honest code-plane property rather than something a writer invents at runtime).
    SELECT mode, targets_other_author INTO v_mode, v_targets_other
        FROM event_type_class WHERE event_type = v_type;
    v_deferred := (v_mode IS NULL);

    v_bears := EXISTS (
        SELECT 1 FROM jsonb_array_elements(b -> 'contributors') AS e
        WHERE e ? 'responsibility');

    -- 4. Attestation gate. A suppressing event, or any asserted responsibility,
    --    is admitted only against a valid human attestation token bound to THIS
    --    event's content-address. The token travelled with the event on the sync
    --    wire (db/001 columns); a peer that ships a suppress without one is refused —
    --    the exact hole review A2 flagged (un-attested visibility.suppress synced in).
    -- The DEFERRED arm: store the travelling attestation token WITHOUT gating on it.
    --
    -- This is not an optimisation — it is what keeps admit-and-defer from silently
    -- degrading into a slower fail-closed. A suppressing event's attestation token TRAVELS
    -- with it on the sync wire (db/001's additive columns), and the gate below is the only
    -- thing that ever stored it. Skip the gate naively and the token is DROPPED — so when
    -- classifying code later arrives, cairn_readjudicate_deferred (db/043) has nothing to
    -- verify and the event can NEVER gain power. Storing it costs nothing and is what makes
    -- re-adjudication possible at all.
    --
    -- INVARIANT, and the reason db/005's cairn_suppression_author_ok had to change: an
    -- attestation on a row that carries an event_deferred marker is CARRIED, NOT VOUCHED —
    -- nothing has verified it. It is NOT true that every reader is either unreachable for
    -- deferred rows or must exclude them explicitly to stay safe: db/043's gate 4
    -- DELIBERATELY runs the projection apply fns in db/018 and db/034 against a deferred row
    -- (its event_deferred marker still present) as its proof the event can project before
    -- promotion (PR #302 finding F1), so "unreachable" does not hold for them either.
    -- What IS true: db/018 (patient_link_apply) and db/034 (medication_attestation_apply)
    -- keep treating event_log.attester_key as a vouch because they exclude
    -- event_attestation_unvouched (db/001) explicitly — keyed on the token's verification,
    -- not on deferral — exactly as cairn_suppression_author_ok (which reads the TARGET's
    -- attester_key) already does. A new reader of these columns owes that same explicit
    -- exclusion.
    IF v_deferred THEN
        v_att     := p_attestation;
        v_att_key := p_attester_key;
    END IF;

    IF NOT v_deferred AND (v_mode = 'suppressing' OR v_bears) THEN
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
        -- #195: the body's responsibility claim must name the human whose token we
        -- just verified — identical binding to db/005 (shared predicate, principle 12).
        IF NOT cairn_responsibility_bound(b, p_attester_key) THEN
            RAISE EXCEPTION 'apply_remote_event: a contributor claims responsibility for an actor other than the verified attester — unverified responsibility claim refused (issue #195)';
        END IF;
        v_att     := p_attestation;
        v_att_key := p_attester_key;
    END IF;

    -- 5. Target gate for an overlay on another author's event — UNCONDITIONAL for every
    --    targets_other type (issue #191, mirroring db/005: absence must fail CLOSED, not
    --    skip the existence check and the ADR-0043 owner-gate). Target existence is safe to
    --    demand at apply because HLC order is causal: a suppress is authored by someone who
    --    HELD the target, so the target sorts earlier and (on this full-replication plane)
    --    arrives first.
    --
    --    WHAT A REFUSAL COSTS THE PULLER, since ADR-0056 decision 5 (slice 60, #267/#270):
    --    this is a bare RAISE, so the clinical puller reads it as a DELIBERATE refusal —
    --    the bytes are penned VERBATIM in sync_quarantine, the re-offer floor pins their
    --    slot, the cursor still ADVANCES so other authors' events keep flowing, and the
    --    cycle fails loudly. It no longer FREEZES the watermark (that arm is now transient
    --    infrastructure faults only). Two members of this class, with different fates:
    --      * a malformed/absent target that can never become valid sits in the pen and its
    --        re-offers keep failing — poisoning nothing, until a human acks the exclusion;
    --      * a target still IN FLIGHT from another link is the one ORDERING-transient member
    --        here. The floor re-offers its slot every cycle, so the overlay applies and its
    --        pen row auto-releases as soon as the target lands — delayed, never lost. The
    --        accepted cost is that it holds one pen row and keeps the cycle loud until then.
    --
    -- A DEFERRED event skips this whole block. `v_targets_other` is NULL for an unclassified
    -- type, so the branch would short-circuit anyway — but relying on three-valued logic is
    -- exactly what ADR-0056's corollary forbids ("never inferred from a null classification
    -- lookup falling through the gates"). Making the skip EXPLICIT lets the reader see that
    -- the overlay-target-exists check and the ADR-0043 owner-gate are DEFERRED WITH the
    -- interpretation, not waived by it: cairn_readjudicate_deferred (db/043) re-runs both
    -- before any power is granted (decision 4).
    IF NOT v_deferred AND v_targets_other THEN
        v_target_id := cairn_suppression_target_id(b);
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

    -- 7. ADR-0052 lenient sealed arm — the MIRROR IMAGE of db/005's strict arm. A
    --    sealed event NEVER rejects here. With the DEK the full floor runs on the clear
    --    view (custody + shadow + projections, exactly as submit); WITHOUT it (not a
    --    custody holder, or a byte-lazy pull) the row is admitted on structural checks
    --    only — set-union losslessness. A plaintext clinical body is likewise ADMITTED
    --    (foreign / pre-ADR-0052 data); only the STRICT door enforces born-sealed.
    v_sealed := COALESCE((b -> 'payload' ->> 'sealed')::boolean, false);
    b_clear  := b;
    IF v_sealed AND p_dek IS NOT NULL THEN
        v_inner := cairn_unseal_body(b -> 'payload', p_dek, v_event_id::text);
        IF v_inner IS NULL THEN
            -- A presented-but-wrong DEK is a transport defect, not a reason to lose the
            -- event (the strict door RAISEs here; the sync door must not): admit
            -- structurally, custody stays withheld (v_inner NULL routes the no-custody arm).
            RAISE WARNING 'apply_remote_event: sidecar DEK failed to open sealed body % — admitting without custody', v_event_id;
        ELSE
            b_clear := jsonb_set(jsonb_set(b, '{payload}', v_inner -> 'payload'),
                                 '{plaintext_twin}', v_inner -> 'plaintext_twin');
        END IF;
    END IF;
    v_twin_stub := b ->> 'plaintext_twin';

    -- Raise the transaction-local remote-apply marker HERE — before step 8, not only
    -- before the INSERT below. The marker's window used to start right before the
    -- event_log INSERT (so it covered only the AFTER-INSERT projection triggers); that
    -- placement was never a deliberate "validation is always strict" decision, it was
    -- just incidental to projections being the marker's first consumers (db/018's
    -- component-size clamp, db/031's thread-patient guard, db/033's reconciliation
    -- guard — all projection-apply functions, all fired during the INSERT). But by the
    -- time execution reaches this line, the node genuinely IS on the sync-apply path —
    -- this whole function exists for nothing else — so the marker should have covered
    -- the WHOLE apply, validation included, from the start.
    --
    -- Widening the window to also cover step 8 (the cairn_event_twin per-type floor
    -- dispatch, immediately below) is the point of moving it: db/041's
    -- cairn_check_medication_coding is the first per-type check_fn to read this marker,
    -- and step 8 runs BEFORE the old marker placement — so a registry-derived coding
    -- check could never tell "local" from "remote" and refused a verifiable peer event
    -- outright, which is exactly the sync-watermark freeze ADR-0056 forbids. Confirmed
    -- safe to widen: every EXISTING READER of this marker (cairn_recompute_component and
    -- patient_link_apply in db/018, chart_dispute_apply in db/023,
    -- cairn_guard_medication_patient in db/031, medication_reconciliation_apply in
    -- db/033 — named, not line-numbered, since line numbers rot on the next edit)
    -- already lives in the projection-apply layer and fires strictly after this new,
    -- earlier line — none of them change behaviour. No registered check_fn read it
    -- before db/041, so there is nothing upstream of step 8 to regress either.
    --
    -- The one other WRITER is worth naming too: `cairn_reproject` (db/039) raises this
    -- same marker for its whole heal/rebuild run and never clears it — before this
    -- change that only ever relaxed projection guards during a heal. Now it also
    -- relaxes this door's validation tier for anything that runs inside the SAME
    -- transaction as a reproject call (e.g. `BEGIN; SELECT cairn_reproject(...);
    -- SELECT submit_event(<event with an unregistered coding system>); COMMIT;` would
    -- be admitted). `cairn_reproject` is owner-only (REVOKE ... FROM PUBLIC in db/039),
    -- so this is not a runtime-role bypass — an operator with EXECUTE on it already has
    -- the standing to load arbitrary schema — but it is a real widening of what that
    -- marker now means, so a future reader extending either function should know the
    -- two are now coupled.
    --
    -- Still cleared at the SAME place as before (right after the INSERT, below) — a
    -- later submit_event in the same transaction (outside a reproject) keeps its veto,
    -- unchanged.
    PERFORM set_config('cairn.remote_apply', 'on', true);

    -- 8. Plaintext twin + per-type structural floor, via the SAME cairn_event_twin hook
    --    as submit_event — one floor renderer, so a twin-less demographic event is
    --    refused identically at both doors (closes the M8 asymmetry). A sealed event with
    --    NO readable custody (no DEK, or a wrong one) cannot run the structural check on
    --    its ciphertext, so it stores the signed stub twin, degrading to the mechanical
    --    skeleton only if the author omitted one. With custody, the floor runs on the
    --    CLEAR view exactly like submit.
    IF v_sealed AND (v_inner IS NULL) THEN
        v_twin := COALESCE(NULLIF(v_twin_stub, ''), cairn_twin_skeleton(v_type, b));
    ELSE
        v_twin := cairn_event_twin(v_type, b_clear);
    END IF;

    -- 9. Custody + operational clear view — BEFORE the log INSERT so the AFTER INSERT
    --     projection triggers can already read the shadow (same txn). ANTI-RESURRECTION:
    --     an already-shredded target gets NEITHER — set-union may re-deliver the row
    --     forever, but custody never comes back (arrival-order independence). The
    --     unwrap-key-missing case is downgraded to a WARNING + skip (NOT the strict
    --     door's RAISE): a pulling node that never registered its unwrap key must still
    --     ADMIT the event, just without shred capability, rather than lose it.
    IF v_sealed AND v_inner IS NOT NULL
       AND NOT EXISTS (SELECT 1 FROM erasure_shred_log WHERE target_event_id = v_event_id) THEN
        SELECT unwrap_pub INTO v_pub FROM node_unwrap_key;
        IF v_pub IS NULL THEN
            RAISE WARNING 'apply_remote_event: node unwrap key not registered — admitting sealed event % WITHOUT custody (register the unwrap key to gain shred capability)', v_event_id;
        ELSE
            INSERT INTO event_dek (event_id, dek_wrapped)
            VALUES (v_event_id, cairn_wrap_dek(p_dek, v_pub))
            ON CONFLICT (event_id) DO NOTHING;
            INSERT INTO event_clear (event_id, body, twin)
            VALUES (v_event_id, b_clear -> 'payload', v_twin)
            ON CONFLICT (event_id) DO NOTHING;
        END IF;
    END IF;

    -- cairn.remote_apply was already raised above (before step 8), so it is already
    -- 'on' here — no second set_config needed. It stays 'on' through this INSERT's
    -- AFTER-ROW projection triggers (clamp-and-flag instead of vetoing, A5b; db/018/
    -- db/031/db/033 read it there) and is cleared immediately below.
    --
    -- The §5.9 safety signal is stored verbatim and NEVER checked here (ADR-0063): see
    -- db/049 section 4 for why this door is deliberately lenient where db/005 is strict.
    INSERT INTO event_log
        (event_id, patient_id, event_type, schema_version, hlc_wall, hlc_counter,
         node_origin, t_effective, signed_bytes, content_address, body, contributors,
         signer_key_id, plaintext_twin, attachments, attestation, attester_key, actor_id, sealed,
         clock_grade, safety)
    VALUES (
        v_event_id, (b ->> 'patient_id')::uuid, v_type, b ->> 'schema_version',
        (b -> 'hlc' ->> 'wall')::bigint, (b -> 'hlc' ->> 'counter')::int,
        b -> 'hlc' ->> 'node_origin',
        v_t_eff,
        -- body stays the honest derived view: the ciphertext container for a sealed row
        -- (event_log is append-only + never holds cleartext); the CLEAR payload lives in
        -- the event_clear shadow above.
        p_signed, v_ca, b -> 'payload', b -> 'contributors',
        b ->> 'signer_key_id',
        -- plaintext_twin for a sealed row is NEVER the clear twin (that would leak
        -- cleartext into the append-only log): store the signed stub, or the mechanical
        -- skeleton derived from the ciphertext envelope if a foreign sealed event carried
        -- no stub. (Deviates from a bare v_twin fallback precisely to keep this leak-safe:
        -- on the DEK path v_twin is the CLEAR twin.) Unsealed rows store the real twin.
        CASE WHEN v_sealed THEN COALESCE(NULLIF(v_twin_stub, ''), cairn_twin_skeleton(v_type, b))
             ELSE v_twin END,
        COALESCE(b -> 'attachments','[]'::jsonb),
        v_att, v_att_key, v_actor_id, v_sealed, v_grade, b -> 'safety')
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

    -- Record the deferred state EXPLICITLY (ADR-0056 decision 4's corollary): a node records
    -- that an event was admitted uninterpreted, and that MARKER — not the absence of an
    -- event_type_class row — is what reclassification consumes.
    --
    -- Written AFTER the log INSERT because of the FK. That ordering means the AFTER-INSERT
    -- projection dispatcher firing during that INSERT cannot see the marker, which is why
    -- db/005's classified-before-projected registration guard, not this row, is what keeps a
    -- deferred event unprojected at admission. This row is what keeps it unprojected
    -- FOREVER AFTER, via cairn_replay_eligible.
    --
    -- ON CONFLICT DO NOTHING: an idempotent re-apply of the same event must stay a silent
    -- no-op (set-union), and must never reset a recorded adjudication_error.
    IF v_deferred THEN
        INSERT INTO event_deferred (event_id, event_type)
        VALUES (v_event_id, v_type)
        ON CONFLICT (event_id) DO NOTHING;
        -- The token stored at step 4 above was never verified (nothing here COULD verify
        -- it — the gate is deferred with the interpretation), so name that state now.
        -- Only when a token actually travelled: an event that carried none has nothing
        -- unvouched about it, and a spurious row would make every reader needlessly
        -- exclude a row whose attester_key is NULL anyway.
        IF v_att_key IS NOT NULL OR v_att IS NOT NULL THEN
            INSERT INTO event_attestation_unvouched (event_id)
            VALUES (v_event_id)
            ON CONFLICT (event_id) DO NOTHING;
        END IF;
    END IF;

    -- Learn any attachment references, per rendition (reference-eager, byte-lazy; ADR-0013,
    -- rendition set per ADR-0042). Shared with the submit door via cairn_learn_attachment_refs
    -- (db/027) so the two doors never drift.
    PERFORM cairn_learn_attachment_refs(b);

    -- 10. The erasure plane at the SYNC door (ADR-0052) — LENIENT: unlike submit, there
    --     is NO target-existence requirement. A shred may precede its target on the wire;
    --     recording it in erasure_shred_log is exactly what makes the LATER-arriving
    --     target refuse custody (the custody block above tests NOT EXISTS(erasure_shred_log)
    --     — arrival-order independence half 2). The tombstone is plaintext by design
    --     (v_sealed is false for erasure.*), so b_clear = b here.
    IF v_type = 'erasure.shred.asserted' THEN
        PERFORM cairn_execute_shred(
            (b_clear -> 'payload' ->> 'target_event_id')::uuid,
            v_event_id, COALESCE(b_clear -> 'payload' ->> 'basis', '(unrecorded)'));
    END IF;

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
    -- The CLAMP is this door's admission policy and stays here; the merge itself is the
    -- shared monotone helper in db/001 (issue #227), so the node plane and the clinical
    -- plane can never drift into different clock semantics. Note the split: what differs
    -- between the doors is which wall they hand over, never how it is merged.
    --
    -- LEAST() ignores NULLs, so a body carrying no hlc.wall would yield the CEILING here
    -- rather than NULL — i.e. it would ratchet the clock a full drift-window forward. That
    -- is unreachable by construction: event_log.hlc_wall is NOT NULL and the event is
    -- inserted above, so such a body is refused by the column before reaching this line.
    -- Stated explicitly because the helper's own NULL guard cannot see through the LEAST.
    v_merge_wall := LEAST((b -> 'hlc' ->> 'wall')::bigint,
                          (extract(epoch FROM clock_timestamp()) * 1000)::bigint + cairn_max_hlc_drift_ms());
    PERFORM cairn_node_hlc_merge(v_merge_wall, (b -> 'hlc' ->> 'counter')::int);

    RETURN v_event_id;
END;
$$;

-- The grant floor (ADR-0021). Only the sync runtime role may drive the replication
-- door; the authoring agent role may not (privilege gradient — an agent authors via
-- submit_event, it does not impersonate the sync plane). PUBLIC's default EXECUTE on
-- a new function would bypass the table REVOKEs, so close it explicitly.
REVOKE EXECUTE ON FUNCTION apply_remote_event(bytea, bytea, bytea, bytea) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION apply_remote_event(bytea, bytea, bytea, bytea) TO cairn_node;
-- The sync role reads the log to SERVE events (and never writes it raw).
REVOKE INSERT, UPDATE, DELETE ON event_log FROM cairn_node;
GRANT SELECT ON event_log TO cairn_node;

COMMIT;
