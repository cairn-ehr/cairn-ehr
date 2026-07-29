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
    -- type → count of events promoted this run. A jsonb accumulator rather than a temp
    -- table: the deferred set is tiny by construction (empty on a healthy node), and this
    -- keeps the function free of any object a concurrent caller could collide on.
    v_promoted jsonb := '{}'::jsonb;
BEGIN
    FOR r IN
        SELECT d.event_id, d.event_type, el.signed_bytes, el.content_address,
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

    -- One row per type that gained at least one promoted event. The loader uses these to
    -- scope a targeted heal when there was no generation change to trigger a full one.
    RETURN QUERY
        SELECT k, v::bigint FROM jsonb_each_text(v_promoted) AS t(k, v);
END;
$$;

-- Owner-only, exactly like cairn_reproject (db/039): this function GRANTS POWER to events
-- that were admitted without it. The loader and the CLI connect with owner privileges; the
-- runtime role must not be able to promote anything.
REVOKE EXECUTE ON FUNCTION cairn_readjudicate_deferred() FROM PUBLIC;

COMMIT;
