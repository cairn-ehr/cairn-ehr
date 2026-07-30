-- db/043_deferred_readjudication.sql
-- Cairn — reclassification is RE-ADJUDICATION FIRST, backfill second (ADR-0056 decision 4,
-- issue #266).
--
-- WHAT: `cairn_readjudicate_deferred` — the pass that turns an admitted-uninterpreted event
-- (db/020, issue #265) into a fully-powered one, but ONLY after re-running the floor checks
-- that classification gates.
--
-- WHY THE ORDER IS LOAD-BEARING. Admitting an event uninterpreted necessarily SKIPS every
-- refusal derived from its mode or its target relationship. In db/020 all three sit
-- downstream of the classification lookup:
--
--   * the suppressing⇒attestation gate,
--   * the overlay-target-exists refusal,
--   * the ADR-0043 cross-author-suppression refusal.
--
-- Those are DEFERRED WITH the interpretation, not waived by it. If classification arrival
-- only rebuilt projection rows, a deferred event would gain power having never passed the
-- gate that exists to bound it. Re-running them HERE, before cairn_reproject, is what makes
-- "no unattested suppression" hold at EVERY INSTANT rather than being violated-then-repaired.
--
-- WHERE THE PIECES LIVE. The marker table itself is in db/001, next to event_log, not in
-- this file: db/005's cairn_replay_eligible and cairn_suppression_author_ok both read it and
-- both are LANGUAGE sql, whose bodies resolve table names at CREATE time. The three
-- consumers of the marker are therefore:
--
--   * db/020        — writes it (admission),
--   * db/005        — reads it (the replay gate + the ADR-0043 owner-gate's carried-token
--                     exclusion),
--   * this file     — consumes it (promotion) or annotates it (a recorded refusal).
--
-- connect_and_load_schema re-runs every migration each connect: everything below is
-- idempotent.

BEGIN;

DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'cairn_node') THEN
        CREATE ROLE cairn_node NOLOGIN;
    END IF;
END $$;

-- Read-only for the runtime role: the `cairn-node deferred` listing and any future operator
-- surface. The PASS itself is owner-only (the REVOKE at the foot of this file) — it GRANTS
-- POWER, so it belongs to the same privilege tier as cairn_reproject (db/039).
GRANT SELECT ON event_deferred TO cairn_node;

CREATE OR REPLACE FUNCTION cairn_readjudicate_deferred()
RETURNS TABLE(promoted_type text, promoted_count bigint)
LANGUAGE plpgsql
-- Pinned like every dispatching function in this schema: the helper calls below must never
-- resolve into an attacker-shadowed schema, regardless of caller.
SET search_path = public
AS $$
DECLARE
    r          record;
    b          jsonb;
    v_bears    boolean;
    v_target   uuid;
    v_err      text;
    v_clear    jsonb;
    v_apply_fn text;
    -- type → count of events promoted this run. A jsonb accumulator rather than a temp
    -- table: the deferred set is tiny by construction (empty on a healthy node), and this
    -- keeps the function free of any object a concurrent caller could collide on.
    v_promoted jsonb := '{}'::jsonb;
BEGIN
    -- These are PEER-ARRIVED events, so every check below must run on the LENIENT tier the
    -- door would have used. db/041's cairn_check_medication_coding reads this marker; without
    -- it, gate 0 would refuse a verifiable peer event outright — the sync-watermark freeze
    -- db/020's own step-8 comment warns about, and precisely what ADR-0056 forbids.
    -- SET LOCAL (is_local = true): scoped to this transaction, exactly as cairn_reproject
    -- (db/039) does for its whole run.
    PERFORM set_config('cairn.remote_apply', 'on', true);

    FOR r IN
        SELECT d.event_id, d.event_type, el AS el_row, el.signed_bytes, el.content_address,
               el.attestation, el.attester_key, c.mode, c.targets_other_author
          FROM event_deferred d
          JOIN event_log el       ON el.event_id  = d.event_id
          -- Only rows whose type this node can NOW classify are candidates. A still-unknown
          -- type simply stays deferred, untouched and unflagged — it has not failed
          -- anything, it is merely still uninterpreted.
          JOIN event_type_class c ON c.event_type = d.event_type
         -- HLC (causal) order, so a deferred overlay is adjudicated AFTER the deferred
         -- target it points at: the target is promoted first, and the ADR-0043 gate below
         -- then sees its now-VOUCHED attester_key rather than the carried one it would have
         -- ignored. Collation-independent on node_origin (ADR-0045) — an ICU/locale
         -- collation must not order this differently from "C" on another node.
         ORDER BY el.hlc_wall, el.hlc_counter, el.node_origin COLLATE "C"
    LOOP
        v_err := NULL;
        -- Per-row subtransaction. A failure here must NEVER propagate: this pass runs inside
        -- connect_and_load_schema, so a raise would abort the whole schema load and wedge
        -- the node on one bad event — precisely the failure mode ADR-0056 exists to remove.
        -- The refusal is captured and recorded instead.
        BEGIN
            -- Re-derive the envelope from the SIGNED BYTES, never from the projection
            -- columns: the predicates below must see exactly what the door saw, and a
            -- reconstruction from columns would drift from db/020 on the next edit.
            b := cairn_body(r.signed_bytes);
            IF b IS NULL THEN
                RAISE EXCEPTION 'stored signed bytes no longer parse';
            END IF;

            -- Deferred gate 0 — the per-type STRUCTURAL floor (db/020 step 8). It was skipped
            -- at admission for the same reason gates 1-3 were: the type had no registry row,
            -- so cairn_event_twin found neither a check_fn nor a twin_required_msg and fell
            -- through to the skeleton. Now the row exists, so the check must run — otherwise
            -- this check is WAIVED rather than deferred, and this file's header is false.
            --
            -- cairn_clear_payload is reused rather than reimplementing db/020's
            -- sealed/unsealed branching, so the two paths cannot drift on what a readable
            -- body is. NULL = sealed with no custody here: skip, exactly as the door does —
            -- a structural check cannot run on ciphertext. Gate 4 still proves such an event
            -- can project.
            v_clear := cairn_clear_payload(r.el_row);
            IF v_clear IS NOT NULL THEN
                PERFORM cairn_event_twin(r.event_type, jsonb_set(b, '{payload}', v_clear));
            END IF;

            -- Deferred gate 1 — the suppressing⇒attestation gate (db/020 step 4). The token
            -- being verified here is the one the door CARRIED without checking; this is the
            -- check it was carried for.
            v_bears := EXISTS (
                SELECT 1 FROM jsonb_array_elements(b -> 'contributors') AS e
                WHERE e ? 'responsibility');
            IF r.mode = 'suppressing' OR v_bears THEN
                IF r.attestation IS NULL OR r.attester_key IS NULL THEN
                    RAISE EXCEPTION
                        '% requires attestation (no token travelled with the event) — un-vouched suppress/responsibility refused',
                        r.event_type;
                END IF;
                IF NOT cairn_attestation_ok(r.attestation, r.content_address, r.attester_key) THEN
                    RAISE EXCEPTION 'attestation token invalid or not bound to this event';
                END IF;
                IF NOT EXISTS (SELECT 1 FROM actor_current
                               WHERE signing_key_id = encode(r.attester_key,'hex')
                                 AND kind = 'human') THEN
                    RAISE EXCEPTION 'attester is not an enrolled human actor (forged human author refused)';
                END IF;
                IF NOT cairn_responsibility_bound(b, r.attester_key) THEN
                    RAISE EXCEPTION 'a contributor claims responsibility for an actor other than the verified attester (issue #195)';
                END IF;
                -- VOUCHED, at last. Every check the door would have run has now run
                -- against this token, so it stops being "carried" and becomes a real
                -- vouch. Inside the per-row subtransaction deliberately: if a LATER
                -- gate refuses this event, this clear rolls back with it and the token
                -- stays honestly unvouched.
                DELETE FROM event_attestation_unvouched WHERE event_id = r.event_id;
            END IF;

            -- Deferred gates 2 and 3 — overlay-target-exists and the ADR-0043 owner-gate
            -- (db/020 step 5). Gate 2 can legitimately fail on a target still in flight from
            -- another peer, which is exactly why the loader runs this pass on EVERY connect
            -- and not only on a schema-generation change: that failure resolves when the
            -- target lands, with no code-plane update to trigger a retry.
            IF r.targets_other_author THEN
                v_target := cairn_suppression_target_id(b);
                IF NOT EXISTS (SELECT 1 FROM event_log WHERE event_id = v_target) THEN
                    RAISE EXCEPTION 'overlay targets unknown event %', v_target;
                END IF;
                IF r.mode = 'suppressing'
                   AND NOT cairn_suppression_author_ok(v_target, r.attester_key) THEN
                    RAISE EXCEPTION
                        'cross-author suppression refused — a suppress of another human''s event may not be admitted; disagreement is additive. (ADR-0043)';
                END IF;
            END IF;

            -- Deferred gate 4 — PROVE IT TAKES EFFECT (PR #302 review finding F1).
            --
            -- Gates 0-3 answer "should this event have power?". This one answers "CAN it
            -- take power?", and skipping it bricked the node: the marker delete below
            -- commits, the event becomes replay-eligible, the loader's heal then raises on
            -- it, and because event_log is append-only nothing can undo that. Every
            -- subsequent connect repeated the same failure and the generation stamp never
            -- advanced. Measured: three consecutive connects failed, node_schema frozen.
            --
            -- Running the apply fns HERE, inside the per-row subtransaction, makes the
            -- marker delete conditional on them succeeding: a raise sets v_err, the
            -- subtransaction rolls back every projection write it made, and the marker stays.
            -- The invariant that buys: a PROMOTED EVENT IS ONE THAT HAS ALREADY PROJECTED
            -- CLEANLY. That holds for a stricter apply fn written years from now, which gate
            -- 0 alone would not cover.
            --
            -- WHY PER-EVENT DISPATCH IS AFFORDABLE HERE and not in cairn_reproject: db/039
            -- is deliberately set-based (one full-table pass per (type, fn)) because the
            -- per-event loop it replaced was ~25% of a 2M-event rebuild at the Pi target.
            -- That argument does not transfer — event_deferred is empty on a healthy node
            -- and tiny by construction otherwise.
            --
            -- heal_safe mirrors heal mode (db/039): a fn that only converges under a
            -- TRUNCATE cannot prove anything by running over live rows.
            FOR v_apply_fn IN
                SELECT apply_fn FROM cairn_projection_apply
                 WHERE event_type = r.event_type AND heal_safe
                 ORDER BY run_order, apply_fn
            LOOP
                EXECUTE format('SELECT %I($1)', v_apply_fn) USING r.el_row;
            END LOOP;
        EXCEPTION WHEN OTHERS THEN
            v_err := SQLERRM;
        END;

        IF v_err IS NULL THEN
            -- PROMOTED. Deleting the marker IS the promotion: cairn_replay_eligible reads its
            -- absence, so the event becomes visible to the reprojection the caller runs next.
            DELETE FROM event_deferred WHERE event_id = r.event_id;
            v_promoted := jsonb_set(
                v_promoted, ARRAY[r.event_type],
                to_jsonb(COALESCE((v_promoted ->> r.event_type)::bigint, 0) + 1));
        ELSE
            -- STILL POWERLESS, and now flagged legibly (decision 4). The marker stays, so the
            -- event remains replay-ineligible; the next connect retries it, which is how the
            -- target-arrived-later case heals itself.
            UPDATE event_deferred
               SET adjudication_error = v_err,
                   last_attempt_at    = clock_timestamp()
             WHERE event_id = r.event_id;
        END IF;
    END LOOP;

    -- One row per type that gained at least one promoted event. Gate 4 above already ran
    -- each promoted event's heal-safe apply fns inside its own promotion subtransaction, so
    -- the loader no longer needs this to scope a targeted heal (Task 6, PR #302 finding F1) —
    -- it now exists so the loader can tell the operator which types just gained power.
    RETURN QUERY
        SELECT k, v::bigint FROM jsonb_each_text(v_promoted) AS t(k, v);
END;
$$;

-- Owner-only, exactly like cairn_reproject (db/039): this function GRANTS POWER to events
-- that were admitted without it. The loader and the CLI connect with owner privileges; the
-- runtime role must not be able to promote anything.
REVOKE EXECUTE ON FUNCTION cairn_readjudicate_deferred() FROM PUBLIC;

COMMIT;
