\set ON_ERROR_STOP on
-- recall_event marks a target without deleting it (principle 2).
DO $$
DECLARE n_before bigint; n_after bigint; tgt uuid;
BEGIN
    SELECT count(*) INTO n_before FROM event_log;
    SELECT event_id INTO tgt FROM event_log LIMIT 1;
    IF tgt IS NOT NULL THEN
        PERFORM recall_event(tgt, 'skill-epoch contamination test');
        SELECT count(*) INTO n_after FROM event_log;
        IF n_after <> n_before THEN RAISE EXCEPTION 'recall ERASED data: % -> %', n_before, n_after; END IF;
        IF NOT EXISTS (SELECT 1 FROM recall_overlay WHERE target_event_id = tgt)
            THEN RAISE EXCEPTION 'recall overlay missing'; END IF;
        RAISE NOTICE 'recall OK: overlay added, no data erased';
    END IF;
END $$;

-- A recall naming an event that is NOT in the log must fail loud (issue #99):
-- a fat-fingered UUID silently "recalling" nothing is the worst failure mode.
DO $$ BEGIN
    BEGIN
        PERFORM recall_event(gen_random_uuid(), 'fat-fingered target');
        RAISE EXCEPTION 'FK check FAILED: recall of a nonexistent event succeeded';
    EXCEPTION WHEN foreign_key_violation THEN
        RAISE NOTICE 'recall FK OK: unknown target refused';
    END;
END $$;

-- events_by_actor_epoch resolves against registry HISTORY (issue #99): after an
-- epoch bump (revoke + re-enroll of the same key), the OLD epoch's events must
-- remain selectable — exactly those, none of the new epoch's — and an event with
-- no attribution stamp is over-selected into every registered epoch, flagged.
BEGIN;
DO $$
DECLARE
    aid_a bytea; aid_b bytea;
    e_a uuid := gen_random_uuid(); e_b uuid := gen_random_uuid(); e_null uuid := gen_random_uuid();
BEGIN
    -- issue #166: this suite maps ONE key ('histkey') across THREE epochs to force
    -- actor_id=NULL and exercise the events_by_actor_epoch NULL-attribution fallback.
    -- enroll_actor now refuses that dual mapping, so stage it via a raw INSERT (the state
    -- still arises from non-enroll paths; the recall projection must still cope). actor_id
    -- is computed as the door would, so the epoch_regs join matches.
    INSERT INTO actor_event (actor_id, op, kind, pinned, signing_key_id)
    VALUES (cairn_actor_id('{"model":"m","version":"1","skill_epoch":"hist-a"}'::jsonb),
            'enroll', 'agent', '{"model":"m","version":"1","skill_epoch":"hist-a"}'::jsonb, 'histkey')
    RETURNING actor_id INTO aid_a;
    INSERT INTO actor_event (actor_id, op) VALUES (aid_a, 'revoke');
    INSERT INTO actor_event (actor_id, op, kind, pinned, signing_key_id)
    VALUES (cairn_actor_id('{"model":"m","version":"1","skill_epoch":"hist-b"}'::jsonb),
            'enroll', 'agent', '{"model":"m","version":"1","skill_epoch":"hist-b"}'::jsonb, 'histkey')
    RETURNING actor_id INTO aid_b;

    -- Owner-path rows (bypassing the doors) with explicit attribution stamps: one
    -- per epoch, plus one honestly-unattributed (NULL) row for the same key.
    INSERT INTO event_log (event_id, patient_id, event_type, schema_version, hlc_wall,
        hlc_counter, node_origin, signed_bytes, content_address, body, contributors,
        signer_key_id, plaintext_twin, actor_id)
    VALUES
      (e_a,    gen_random_uuid(), 'note.added','x',1,0,'n','\x01'::bytea,'\x1220'||digest('\x01'::bytea,'sha256'),'{}','[]','histkey','t', aid_a),
      (e_b,    gen_random_uuid(), 'note.added','x',2,0,'n','\x02'::bytea,'\x1220'||digest('\x02'::bytea,'sha256'),'{}','[]','histkey','t', aid_b),
      (e_null, gen_random_uuid(), 'note.added','x',3,0,'n','\x03'::bytea,'\x1220'||digest('\x03'::bytea,'sha256'),'{}','[]','histkey','t', NULL);

    -- The superseded epoch selects its own event + the unattributed one — never e_b.
    IF NOT EXISTS (SELECT 1 FROM events_by_actor_epoch('histkey','hist-a') x
                   WHERE x.event_id = e_a AND x.attribution = 'pinned') THEN
        RAISE EXCEPTION 'epoch-history FAILED: superseded epoch lost its pinned event (issue #99)';
    END IF;
    IF EXISTS (SELECT 1 FROM events_by_actor_epoch('histkey','hist-a') x WHERE x.event_id = e_b) THEN
        RAISE EXCEPTION 'epoch-history FAILED: another epoch''s pinned event leaked into hist-a';
    END IF;
    IF NOT EXISTS (SELECT 1 FROM events_by_actor_epoch('histkey','hist-a') x
                   WHERE x.event_id = e_null AND x.attribution = 'unattributed') THEN
        RAISE EXCEPTION 'epoch-history FAILED: unattributed event missing from recall set (must over-select)';
    END IF;
    -- A never-registered epoch selects nothing.
    IF EXISTS (SELECT 1 FROM events_by_actor_epoch('histkey','hist-z')) THEN
        RAISE EXCEPTION 'epoch-history FAILED: unregistered epoch selected events';
    END IF;

    -- Registry lag: an epoch registered AFTER events were admitted cannot trust
    -- those events' stamps for exclusion — the stamps were written before this node
    -- knew the epoch existed (the origin may have authored under it all along). The
    -- earlier events must over-select into the late epoch, flagged
    -- 'pre-registration'; NULL-stamped rows stay 'unattributed'.
    INSERT INTO actor_event (actor_id, op, kind, pinned, signing_key_id)
    VALUES (cairn_actor_id('{"model":"m","version":"1","skill_epoch":"hist-c"}'::jsonb),
            'enroll', 'agent', '{"model":"m","version":"1","skill_epoch":"hist-c"}'::jsonb, 'histkey');
    IF NOT EXISTS (SELECT 1 FROM events_by_actor_epoch('histkey','hist-c') x
                   WHERE x.event_id = e_a AND x.attribution = 'pre-registration') THEN
        RAISE EXCEPTION 'registry-lag FAILED: pre-registration event missing from late epoch''s recall set';
    END IF;
    IF NOT EXISTS (SELECT 1 FROM events_by_actor_epoch('histkey','hist-c') x
                   WHERE x.event_id = e_null AND x.attribution = 'unattributed') THEN
        RAISE EXCEPTION 'registry-lag FAILED: unattributed event missing from late epoch''s recall set';
    END IF;
    RAISE NOTICE 'events_by_actor_epoch history resolution OK';
END $$;
ROLLBACK;
