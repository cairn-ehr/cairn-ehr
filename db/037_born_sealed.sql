-- 037_born_sealed.sql — ADR-0052: born-sealed clinical bodies, the custody plane.
--
-- WHY: every clinical JSONB body is sealed at write under a per-event DEK so the
-- ADR-0005 erasure ladder stays reachable forever. These are the MUTABLE tables
-- beside the append-only log: custody may rotate and derived plaintext may be
-- scrubbed, so none of them get the append-only trigger — deliberately.
-- The reserved event_log.dek_wrapped column (db/001) is retired unused: an
-- append-only row cannot hold rotating multi-holder custody.

BEGIN;

-- ---------------------------------------------------------------------------
-- 1. The node's X25519 public unwrap key (single row). The SECRET half lives in
--    the daemon keystore, in its own `<key>.unwrap` file sealed under the ADR-0026
--    escrow secrets, and NEVER enters the database — a DB backup can never
--    reconstruct a DEK. It is an INDEPENDENT keypair: ADR-0066 superseded ADR-0052's
--    derivation-from-the-Ed25519-seed, because disaster recovery deliberately mints a
--    fresh signing seed and the derived secret then changed, permanently orphaning
--    every inherited wrapped-DEK row (#495). The row is written by provisioning
--    (`cairn-node init` / `establish-unwrap-key`), never by the write path.
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS node_unwrap_key (
    singleton     BOOLEAN     PRIMARY KEY DEFAULT TRUE CHECK (singleton),
    unwrap_pub    BYTEA       NOT NULL CHECK (octet_length(unwrap_pub) = 32),
    registered_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);

-- The registrar. Its ONE guarantee, which ADR-0066 now leans on in several places
-- (restore adopts rather than mints BECAUSE this refuses a differing key), is:
-- **after this returns successfully, the registered key IS p_pub.**
--
-- The ON CONFLICT DO NOTHING below is what makes the insert safe against a concurrent
-- provisioning run — but DO NOTHING is also silence, and silence here would break the
-- guarantee: the loser of that race would return successfully while a DIFFERENT key sat
-- in the table. Two operators provisioning at once is rare; a caller that believes its
-- key is registered when another one is, is unrecoverable (the singleton then refuses
-- the real key forever). So the outcome is RE-READ and compared, and the same refusal
-- fires whether the conflict was found before the insert or created during it.
CREATE OR REPLACE FUNCTION cairn_register_unwrap_key(p_pub BYTEA) RETURNS void
LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, pg_temp AS $$
DECLARE v_existing BYTEA;
BEGIN
    SELECT unwrap_pub INTO v_existing FROM node_unwrap_key;
    IF v_existing IS NULL THEN
        INSERT INTO node_unwrap_key (unwrap_pub) VALUES (p_pub)
        ON CONFLICT (singleton) DO NOTHING;
        -- Re-read: DO NOTHING means a concurrent transaction won the singleton, and
        -- whatever IT registered is what this node now has. Reporting success without
        -- checking would hand this caller a false belief about its own custody.
        SELECT unwrap_pub INTO v_existing FROM node_unwrap_key;
    END IF;
    IF v_existing IS NOT NULL AND v_existing <> p_pub THEN
        -- Unwrap-key rotation re-wraps every custody row — a deliberate,
        -- separate ceremony (ADR-0052 deferred list). Refuse a silent swap.
        RAISE EXCEPTION 'cairn_register_unwrap_key: a different unwrap key is registered — rotation is a separate ceremony (ADR-0052)';
    END IF;
END;
$$;

-- ---------------------------------------------------------------------------
-- 2. Per-event DEK custody (this node's wrapped copy). Destroying a row IS the
--    local half of a crypto-shred.
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS event_dek (
    event_id    UUID        PRIMARY KEY,
    dek_wrapped BYTEA       NOT NULL,
    wrapped_at  TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);

-- ---------------------------------------------------------------------------
-- 3. event_clear (the CLEAR-view table) + cairn_clear_payload moved to db/005 so
--    db/034's LANGUAGE sql functions can bind them (LANGUAGE sql resolves refs
--    eagerly at CREATE time, and db/034 loads before this migration); the rest of
--    the custody plane stays here. See db/005 for the full ordering rationale.
-- ---------------------------------------------------------------------------

-- ---------------------------------------------------------------------------
-- 4. The shred log: which events have been erased here. Rebuilt idempotently
--    from the append-only log below, so a restore/full-replay re-applies every
--    shred BEFORE any custody or projection could resurrect (§3.8).
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS erasure_shred_log (
    target_event_id UUID        PRIMARY KEY,
    shred_event_id  UUID        NOT NULL,
    basis           TEXT        NOT NULL,
    shredded_at     TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);

-- ---------------------------------------------------------------------------
-- 5. Projection read helper cairn_clear_payload moved to db/005 (see section 3
--    above and the ordering note in db/005): db/034's LANGUAGE sql functions bind
--    it eagerly, so it must be defined before db/034 loads.
-- ---------------------------------------------------------------------------

-- ---------------------------------------------------------------------------
-- 6. erasure.shred.asserted — the rung-3 audited tombstone (plaintext BY DESIGN:
--    it must outlive every key). Classified additive: the erasure arm in the
--    doors owns target handling (a shred may arrive BEFORE its target on the
--    sync wire — the targets_other gate would wrongly reject that at apply).
-- ---------------------------------------------------------------------------
INSERT INTO event_type_class (event_type, mode, targets_other_author) VALUES
    ('erasure.shred.asserted', 'additive', false)
ON CONFLICT (event_type) DO NOTHING;

CREATE OR REPLACE FUNCTION cairn_check_erasure_shred(p_type text, b jsonb) RETURNS void
LANGUAGE plpgsql AS $$
BEGIN
    IF (b -> 'payload' ->> 'target_event_id') IS NULL THEN
        RAISE EXCEPTION 'erasure.shred: payload must name target_event_id (ADR-0052)';
    END IF;
    -- #626: a deliberate P0001 naming the field, never a raw cast's 22P02. This check runs
    -- at BOTH doors, and on the sync door a non-P0001 failure freezes the peer's pull.
    PERFORM cairn_uuid_or_raise('target_event_id', b -> 'payload' ->> 'target_event_id',
                                'erasure.shred');
    IF COALESCE(b -> 'payload' ->> 'basis', '') = '' THEN
        RAISE EXCEPTION 'erasure.shred: payload must carry a non-empty basis (the audited "why" — ADR-0005 rung 3)';
    END IF;
END;
$$;
-- PUBLIC holds EXECUTE by default; the cairn_check_* family is revoked uniformly (#382,
-- convention stated in db/005 above cairn_check_twin_registry_fn).
REVOKE EXECUTE ON FUNCTION cairn_check_erasure_shred(text, jsonb) FROM PUBLIC;

INSERT INTO cairn_event_twin_check (event_type, check_fn, twin_required_msg) VALUES
    ('erasure.shred.asserted', 'cairn_check_erasure_shred',
     'erasure.shred requires a non-empty authored twin (the tombstone must be legible — ADR-0052)')
ON CONFLICT (event_type) DO NOTHING;

-- ---------------------------------------------------------------------------
-- 7. Shred execution, shared by both doors (never drifts): record, scrub
--    custody + derived plaintext + provenance-precise projection rows. The
--    event_log row is NEVER touched (append-only; signature still verifies).
--
--    A shred that leaves the shredded body's text readable in ANY projection is not
--    a shred (ADR-0005 rung 3 / #92(b) mandatory-index invalidation). EVERY medication
--    verb writes derived plaintext, so EVERY verb's projection is in scope — not only the
--    three the walking skeleton first shipped (code-review finding #2). The provenance-precise
--    key differs per table: medication_statement / medication_coding / medication_cessation /
--    medication_dose_correction / medication_reconciliation / medication_attestation /
--    medication_withhold / medication_withhold_release all carry the PRODUCING event's
--    content_address (db/031/032/033/034/059) so they scrub by
--    `content_address = v_ca`; the initial-dose seed row is keyed by the assert event's own
--    id (db/032); and
--    medication_group_member is a DERIVED table (recomputed from the standing reconciled
--    edges, like person_member — db/033), so a shredded reconciliation must delete its edge
--    AND recompute the affected component so the erased merge stops grouping the two threads.
--    Overlay winners from OTHER, unshredded events survive (never over-erase).
-- ---------------------------------------------------------------------------
-- #863 — the medication threads an event's clear payload names: the threads the keys of every
-- one-winner medication projection it can win belong to (a thread's statement, coding, stop and
-- dose correction carry `medication_id`; a reconciliation or separation names its pair as
-- `subject_a`/`subject_b`). A thread is a superset filter: a dose correction's own key is the
-- point it corrects, and the replay's insert-or-better picks the winner within the thread.
-- CANONICAL text: the floor admits any spelling Postgres casts to uuid (upper case, braces, no
-- hyphens) and the appliers key on the cast, so a foreign client's "ABCD…" must match a sibling's
-- "abcd…" here too, or its thread is never re-folded (PR #865 review). A malformed id matches
-- nothing, never raises inside a shred.
CREATE OR REPLACE FUNCTION cairn_medication_threads_named(p jsonb)
RETURNS text[] LANGUAGE sql STABLE AS $$
    SELECT COALESCE(array_agg(k::uuid::text), '{}')
    FROM unnest(ARRAY[p ->> 'medication_id', p ->> 'subject_a', p ->> 'subject_b']) AS k
    WHERE k IS NOT NULL AND pg_input_is_valid(k, 'uuid')
$$;
REVOKE EXECUTE ON FUNCTION cairn_medication_threads_named(jsonb) FROM PUBLIC;

-- #863 — after a shred's scrub, hand the shredded event's rows back to the next event in line.
--
-- WHY. A one-winner projection (medication_statement, medication_coding, medication_cessation,
-- medication_dose_correction, medication_reconciliation) keeps ONE row per key: the winner's. The
-- scrub deletes the rows the shredded event produced; when that event WAS the winner, the key was
-- left with no row at all and nothing re-applied the runner-up. Stop a drug for "angioedema", stop
-- it again, shred the second stop: the thread had no medication_cessation row, so the line read
-- CURRENT until the next whole-log heal. The record under-stated, and in the unsafe direction.
--
-- HOW. A heal scoped to what the shredded event could have won. Every applier is insert-or-better
-- and arrival-order-independent, the property cairn_reproject's heal mode (db/039) already rests
-- on, so re-running the heal-safe appliers of the OTHER events that can win the same rows converges
-- each key to its true winner among the events that remain. "Can win the same rows" is two filters:
--   * the same threads (cairn_medication_threads_named, above), and
--   * an event type whose registered projection_tables share a non-`*_flag` table with the
--     shredded type's. Nothing enforces that a registration lists every table its applier writes;
--     it is a convention this and db/039's narrow rebuild both rely on. A `*_flag` table is an
--     append-only per-event record shared by most types, never a winner, so sharing one is
--     ignored. This keeps a stop's shred from replaying reconciliations (and taking their CARNMR
--     lock). The filter is still a superset (an assert and a dose change share
--     medication_dose_event, which is not a winner); the extra replays are idempotent.
-- Deferred events are skipped (cairn_replay_eligible). The TARGET is excluded by id: a SEALED
-- target names no thread anyway (its clear view is deleted just before this runs, so
-- cairn_clear_payload yields NULL), but the sync door also shreds a peer's PLAINTEXT event, whose
-- body stays in the log and still names its thread; replaying it would re-insert the rows the
-- scrub just deleted (PR #865 review). An EARLIER-shredded plaintext sibling is not excluded: that
-- needs the shred log, whose predicate shred_predicate_has_one_home.rs keeps in one place, and
-- cairn_reproject's heal re-projects such an event today in the same way (issue #866).
--
-- COST. Finding a thread's events reads the clear payload of every medication event of those
-- types: no index names a thread. Shreds are rare, deliberate acts.
-- shortcut: a whole-node scan of medication events per shred; add a thread index on event_log if
-- shreds become frequent or a node's medication log grows large enough to make one slow.
--
-- LOCKS. Each sibling is replayed through cairn_projection_dispatch_heal_safe, which WAITS for a
-- sealed sibling's custody lock: whoever holds it (a door granting it or re-receiving it, another
-- heal) may commit or roll back, and only after that is known can this transaction read it right.
-- Skipping a held sibling instead left the wrong winner, or none, when the holder rolled back or
-- wrote nothing (PR #865 review). A sibling whose custody is being granted RIGHT NOW is never
-- waited on: its clear view is not yet committed, so it names no thread here. That case is covered
-- by the row instead: the shred's DELETE and that door's insert-or-better upsert (ON CONFLICT DO
-- UPDATE ... WHERE, which locks the conflicting row even when the WHERE is false) serialize on the
-- winner row, so the door's later write lands after the scrub. An applier rewritten to DO NOTHING
-- plus a separate conditional UPDATE would lose that. Two shreds never wait on each other's
-- siblings in a cycle, because cairn_execute_shred serializes shreds first
-- (cairn_lock_shred_refold, below). A door that holds a sibling's lock and then waits on a row
-- this shred deleted can still meet it in a cycle; Postgres aborts one side with 40P01, a
-- retryable failure that wrote nothing (db/005's deadlock-shape note).
--
-- ERRORS. A sibling whose replay raises aborts the whole shred: the erasure is refused rather than
-- leave a key under-stated. A sibling's REFUSAL (P0001, the code every classifier reads as a
-- verdict) is re-raised bare, still P0001, with the sibling's id and type in the message, so whoever
-- reads it (the clinician, an operator reading the sync pen) sees which sibling failed rather than
-- a bare refusal of the shred. Any other code (a 40P01, an outage) is not caught and propagates
-- unchanged, so its class is untouched — re-raising it under a named code would need USING ERRCODE,
-- which floor_refusals_carry_no_errcode.rs forbids in db/. A P0001 at the sync door is still classed
-- a refusal and pens the shred; whether a failed re-fold should block an erasure at all is #867.
--
-- POSTURE. The replay runs lenient (cairn.remote_apply = on), like cairn_reproject: a replayed
-- event may have been admitted by the sync door with a clamped-and-flagged conflict that the strict
-- posture would now RAISE on, aborting the shred. The caller's setting is put back afterwards, so
-- the rest of its transaction keeps the posture it chose.
-- The pre-review signature (no target), so a dev database that loaded it keeps no stale overload.
DROP FUNCTION IF EXISTS cairn_refold_after_shred(text, text[]);
CREATE OR REPLACE FUNCTION cairn_refold_after_shred(p_target uuid, p_type text, p_threads text[])
RETURNS void LANGUAGE plpgsql SET search_path = public, pg_temp AS $$
DECLARE
    v_posture text := current_setting('cairn.remote_apply', true);
    v_ev      event_log;
    v_msg     text;
BEGIN
    IF cardinality(p_threads) = 0 THEN
        RETURN;
    END IF;
    PERFORM set_config('cairn.remote_apply', 'on', true);
    FOR v_ev IN
        SELECT e.* FROM event_log e
        WHERE e.event_type IN (
                SELECT DISTINCT b.event_type
                FROM cairn_projection_apply a
                JOIN cairn_projection_apply b ON EXISTS (
                    SELECT 1 FROM unnest(a.projection_tables) t
                    WHERE t = ANY (b.projection_tables) AND t NOT LIKE '%\_flag')
                WHERE a.event_type = p_type AND b.heal_safe)
          AND e.event_id <> p_target
          AND cairn_replay_eligible(e)
          AND cairn_medication_threads_named(cairn_clear_payload(e)) && p_threads
        ORDER BY e.hlc_wall, e.hlc_counter, e.node_origin, e.event_id
    LOOP
        BEGIN
            PERFORM cairn_projection_dispatch_heal_safe(v_ev);
        EXCEPTION WHEN raise_exception THEN
            -- P0001 only, re-raised BARE so it stays P0001 (floor_refusals_carry_no_errcode.rs).
            GET STACKED DIAGNOSTICS v_msg = MESSAGE_TEXT;
            RAISE EXCEPTION '#863 shred re-fold: replaying sibling event % (%) failed: %',
                v_ev.event_id, v_ev.event_type, v_msg;
        END;
    END LOOP;
    PERFORM set_config('cairn.remote_apply', COALESCE(v_posture, ''), true);
END;
$$;
REVOKE EXECUTE ON FUNCTION cairn_refold_after_shred(uuid, text, text[]) FROM PUBLIC;

-- #863 / PR #865 review — one shred at a time. The re-fold waits on its siblings' custody locks
-- while holding its own target's; two concurrent shreds of sibling events would wait on each other
-- (and each upserts rows the other deleted). Taking this before the custody lock serializes shreds
-- node-wide, so they never form that cycle. Shreds are rare, deliberate acts; the cost is that two
-- of them queue. Lock order in a shred: erasure-heal (shared) → this → the target's custody lock →
-- the scrub's projection row locks → siblings' custody locks → any projection lock their appliers
-- take. The scrub's row locks coming before the siblings' custody locks is the inversion db/005's
-- deadlock-shape note accepts against a door.
CREATE OR REPLACE FUNCTION cairn_lock_shred_refold()
RETURNS void LANGUAGE sql
SET search_path = public, pg_temp
AS $$
    SELECT pg_advisory_xact_lock(hashtextextended('cairn.shred-refold', 0));
$$;
REVOKE EXECUTE ON FUNCTION cairn_lock_shred_refold() FROM PUBLIC;

CREATE OR REPLACE FUNCTION cairn_execute_shred(p_target uuid, p_shred_event uuid, p_basis text)
RETURNS void LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, pg_temp AS $$
DECLARE
    v_ca BYTEA;
    v_lo uuid;
    v_hi uuid;
    v_type    text;
    v_threads text[];
BEGIN
    -- #773: wait for any whole-log heal/rebuild in progress, and make one that starts now wait for
    -- this shred (db/005's cairn_lock_erasure_heal says why). SHARED, so this lock never makes
    -- one shred wait for another (cairn_lock_shred_refold, next, does — #863); and FIRST, before
    -- the custody lock — the lock order that note fixes.
    PERFORM cairn_lock_erasure_heal(false);
    -- #863: shreds one at a time, so two re-folds never wait on each other's siblings (the note
    -- on cairn_lock_shred_refold above). After the heal lock, before the custody lock.
    PERFORM cairn_lock_shred_refold();
    -- #604: wait for any transaction still granting the target custody (db/005's
    -- cairn_lock_custody says why). Every statement below then runs on a snapshot that sees that
    -- transaction's committed custody and projection rows, so the scrub reaches them; and a key
    -- arriving after this point waits for THIS commit and then finds the shred-log row. The same
    -- lock covers re-adjudication's gate 4 and the late-custody path (#773), which take it in
    -- cairn_projection_dispatch_heal_safe.
    PERFORM cairn_lock_custody(p_target);

    INSERT INTO erasure_shred_log (target_event_id, shred_event_id, basis)
    VALUES (p_target, p_shred_event, p_basis)
    ON CONFLICT (target_event_id) DO NOTHING;

    -- Provenance-precise projection scrub: only rows THIS event produced.
    -- (Overlay winners from other, unshredded events survive — never over-erase; and a key whose
    -- winner THIS event was is re-folded from the others at the end — #863.)
    -- The threads are read now, while the target's clear view still exists (deleted below).
    SELECT e.content_address, e.event_type, cairn_medication_threads_named(cairn_clear_payload(e))
      INTO v_ca, v_type, v_threads
      FROM event_log e WHERE e.event_id = p_target;
    -- Subset-node degradation (final review): on a cairn-sync subset-only node the medication
    -- projection tables (db/031-035) are ABSENT, so an unconditional DELETE would raise
    -- "relation does not exist" and WEDGE the shred — and, via the apply door, freeze the sync
    -- watermark. Guard each PROJECTION delete with a table-existence check so the shred degrades
    -- HONESTLY on a projection-less node: custody + derived plaintext (event_clear/event_dek,
    -- below) and the erasure ledger still die — the whole erasure guarantee — there is simply
    -- no projection to scrub. The event_clear/event_dek/erasure_shred_log deletes stay
    -- unconditional (those tables ARE in the subset).
    IF v_ca IS NOT NULL THEN
        IF to_regclass('public.medication_statement') IS NOT NULL THEN
            DELETE FROM medication_statement WHERE content_address = v_ca;
        END IF;
        -- Drug-identity coding (db/031, ADR-0059). `coding_display` is the drug's preferred
        -- name and `coding_code` is the immortal moiety anchor — to any holder of a drug
        -- database that IS the substance, sitting next to patient_id in a table the runtime
        -- role can SELECT. Leaving it behind after a shred that reported success is the
        -- rung-3 failure above, and it does NOT ride medication_statement's delete: this is
        -- its own table precisely so slice 6b's coding overlays can write it independently.
        -- Same provenance-precise key as its siblings, so an overlay winner from another,
        -- unshredded event survives (never over-erase).
        IF to_regclass('public.medication_coding') IS NOT NULL THEN
            DELETE FROM medication_coding WHERE content_address = v_ca;
        END IF;
        IF to_regclass('public.medication_cessation') IS NOT NULL THEN
            DELETE FROM medication_cessation WHERE content_address = v_ca;
        END IF;
        -- Every stop's reason (db/060, #857): one row per cessation event that gave a reason,
        -- keyed by its CA.
        IF to_regclass('public.medication_cessation_reason') IS NOT NULL THEN
            DELETE FROM medication_cessation_reason WHERE content_address = v_ca;
        END IF;
        -- Withhold / release rows (db/059, ADR-0081): the hold's reason is clinical plaintext
        -- keyed by THIS event's content_address, so the same provenance-precise scrub applies.
        IF to_regclass('public.medication_withhold') IS NOT NULL THEN
            DELETE FROM medication_withhold WHERE content_address = v_ca;
        END IF;
        IF to_regclass('public.medication_withhold_release') IS NOT NULL THEN
            DELETE FROM medication_withhold_release WHERE content_address = v_ca;
        END IF;
        -- Dose-CORRECTION overlay (db/032/035): the corrected amount/unit/effective/reason/note
        -- is clinical plaintext keyed by the corrected point but carrying THIS event's CA.
        IF to_regclass('public.medication_dose_correction') IS NOT NULL THEN
            DELETE FROM medication_dose_correction WHERE content_address = v_ca;
        END IF;
        -- Reconciliation/separation edge (db/033): scrub the standing edge THIS event won, then
        -- recompute the group around BOTH endpoints so medication_group_member no longer reflects
        -- the erased merge. Recomputing both endpoints is always correct (an unlink splits a
        -- component into at most the piece containing lo and the piece containing hi — the same
        -- invariant db/018's patient_link recompute relies on).
        IF to_regclass('public.medication_reconciliation') IS NOT NULL THEN
            DELETE FROM medication_reconciliation WHERE content_address = v_ca
                RETURNING low, high INTO v_lo, v_hi;
            IF v_lo IS NOT NULL AND to_regclass('public.medication_group_member') IS NOT NULL THEN
                -- #208: cairn_recompute_medication_group (db/033) gained a p_ca
                -- (content_address) parameter for the oversize-flag dedup key. NULL
                -- here is honest degradation (db/033's own documented stance for a
                -- caller with no triggering-event context). A shred only ever REMOVES
                -- a reconciled edge, so this recompute cannot GROW a component; the
                -- oversize branch is therefore extremely unlikely here — but NOT
                -- proven impossible (removing one edge of a redundant/cyclic
                -- component can leave the remainder still above the cap, and on the
                -- local door that branch RAISEs, aborting the shred). Pre-existing
                -- raise-vs-flag asymmetry, unchanged by #208.
                PERFORM cairn_recompute_medication_group(v_lo, NULL);
                PERFORM cairn_recompute_medication_group(v_hi, NULL);
            END IF;
        END IF;
        -- Attestation overlay (db/034): attester identity + reviewed commitment/count.
        IF to_regclass('public.medication_attestation') IS NOT NULL THEN
            DELETE FROM medication_attestation WHERE content_address = v_ca;
        END IF;
    END IF;
    -- The initial-dose seed row is keyed by the assert event's own id (db/032).
    IF to_regclass('public.medication_dose_event') IS NOT NULL THEN
        DELETE FROM medication_dose_event WHERE dose_event_id = p_target;
    END IF;

    -- Derived plaintext + custody die last. The target's threads (#863) were read from
    -- event_clear above, so that read must stay before this delete.
    DELETE FROM event_clear WHERE event_id = p_target;
    DELETE FROM event_dek   WHERE event_id = p_target;

    -- #863: re-fold the keys the scrub may have emptied, from every event but the target.
    PERFORM cairn_refold_after_shred(p_target, v_type, COALESCE(v_threads, '{}'));
END;
$$;

-- Rebuild the shred log from the append-only record on every load (idempotent):
-- this is "restore replays the shred log before projecting" for the wiped-and-
-- reloaded case. "Every load" means every schema REPLAY — since ADR-0080 a fresh database
-- or a schema change, not every connect. Nothing in the product wipes this table (restore
-- writes it through db/020's shred step; reprojection does not truncate it), so a log wiped
-- by hand on an unchanged schema is rebuilt by `db::force_schema_replay` + a connect.
INSERT INTO erasure_shred_log (target_event_id, shred_event_id, basis)
SELECT (body ->> 'target_event_id')::uuid, event_id, COALESCE(body ->> 'basis', '(unrecorded)')
FROM event_log WHERE event_type = 'erasure.shred.asserted'
ON CONFLICT (target_event_id) DO NOTHING;

-- ---------------------------------------------------------------------------
-- 8. Grant floor: door-managed only. SELECT for the serve/read paths. Both
--    cairn_agent (db/004) and cairn_node (db/007) are created unconditionally,
--    earlier in migration order, so no existence guard is needed here — mirrors
--    the unconditional REVOKE/GRANT style used from db/022 onward.
-- ---------------------------------------------------------------------------
REVOKE ALL ON node_unwrap_key, event_dek, erasure_shred_log FROM PUBLIC;

REVOKE ALL ON node_unwrap_key, event_dek, erasure_shred_log FROM cairn_agent;
-- event_clear's REVOKE/GRANT (SELECT to cairn_agent) moved to db/005 alongside its
-- table definition (see section 3).

GRANT SELECT ON event_dek, erasure_shred_log, node_unwrap_key TO cairn_node;  -- serve-side custody reads

-- Postgres grants EXECUTE on a new function to PUBLIC by default, and every role
-- (including cairn_agent) is a member of PUBLIC — an un-REVOKEd SECURITY DEFINER
-- function is therefore directly callable by a below-the-floor adversary with raw
-- SQL, bypassing submit_event/apply_remote_event entirely. Explicit REVOKE FROM
-- PUBLIC before each GRANT, mirroring db/007:270 and db/005:571-575.
REVOKE EXECUTE ON FUNCTION cairn_register_unwrap_key(bytea) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION cairn_register_unwrap_key(bytea) TO cairn_node;

-- cairn_execute_shred is called ONLY by the owner's own doors (submit_event's
-- erasure arm, apply_remote_event's erasure arm), which run SECURITY DEFINER as
-- the schema owner and so still reach it — no role needs a direct GRANT here.
REVOKE EXECUTE ON FUNCTION cairn_execute_shred(uuid, uuid, text) FROM PUBLIC;

COMMIT;
