-- Cairn — the vault's slice-2 doors: enrollers, re-key, judging a key, change of passphrase
-- (GUI sign-in slice 2; design:
-- docs/superpowers/specs/2026-11-01-sign-in-slice-2-rekey-and-local-enrolment-design.md).
--
-- WHY THIS FILE EXISTS. Slice 1 (db/061) let only a superuser enrol, and had no way to give a
-- clinician a new key at all. In a real ED that sends the newcomer back to paper or onto someone
-- else's login (design §1). So:
--   * an ENROLLER — a clinician or admin member a superuser granted the role — may enrol and
--     re-key in person (§5.1–§5.3). What an enroller issues is UNCONFIRMED until a superuser
--     confirms it; it signs meanwhile (unconfirmed never blocks signing by itself, §5.5);
--   * a RE-KEY keeps the person's ACTOR and writes a `rotate-key` row (db/004), so the old key
--     signs nothing new while everything it signed stays theirs (§5.3, §5.6);
--   * a superuser CONFIRMS or REPUDIATES a key (§5.5); repudiation is final for that key;
--   * a member may RESEAL their own key under a new passphrase (§5.4);
--   * after a restore the vault is empty but the registry is not: the re-key door re-creates the
--     member row on the existing actor (§5.8).
--
-- THE TRUST BOUNDARY. Every door below is reachable by cairn_agent (the window) — so by any
-- raw-SQL client holding it. None may act without a SIGNED approval verified HERE:
--   * the approver rule lives in ONE function, cairn_vault_approver_ok, which every door calls;
--   * each act is signed over its own ADDRESS (a sha2-256 multihash, domain-separated by a
--     leading tag, like db/061's cairn_vault_enrol_address), so a token for one act can never be
--     replayed as another, and a clinician-enroller's key, released for 'enrol', can never be
--     tricked into signing a clinical event (design §5.1);
--   * every approval lands APPEND-ONLY in actor_event (approved_by + approval, db/004), never
--     only on the member row a re-key overwrites — the cascade (§5.5) must know who approved
--     every key there ever was.
--
-- db/061's four promises still hold for everything here: no door returns vault_member.sealed
-- (the reseal address returns a hash built from digests, nothing more); the passphrase never
-- reaches these doors at all (a reseal's passphrase check goes through the counted release
-- door, purpose 'reseal'); refusals never echo a token or a blob; every door is SECURITY
-- DEFINER with `SET search_path = public, pg_temp`, revoked from PUBLIC.
--
-- WHAT LIVES IN db/061 INSTEAD. The release and enrol doors (db/061) call the purpose map, the
-- enroller rule, the approver rule and the 'replaced' notice, so those are defined in db/061,
-- AHEAD of the doors: on any replay (an upgrade from 62 included) a door and everything it calls
-- then change in one transaction, and a concurrent sign-in can never meet a door calling a
-- function not yet defined (db/061 promise 2). This file holds the doors only the window calls.

BEGIN;

-- The addresses each signed act is bound to. All four follow cairn_vault_enrol_address (db/061):
-- '\x1220' || sha256 of newline-joined fields, the first a domain tag, staff IDs normalised. A
-- NULL field makes the address NULL, which no token verifies against. The window asks these for
-- the bytes to sign; the SQL mirror (db/tests/063) restates each formula.

-- A re-key: this staff ID, holding this role, gets this new key. The role is bound so that,
-- with no member row to check it against (after a restore), a token cannot be spent on another
-- {role, handle} actor of the same staff ID. The key is fresh and single-use (db/004 refuses a key
-- any actor ever held), so the token is good once. The first version took no role: dropped, so no
-- stale 2-argument overload survives a replay.
DROP FUNCTION IF EXISTS cairn_vault_rekey_address(text, text);
CREATE OR REPLACE FUNCTION cairn_vault_rekey_address(p_staff_id text, p_role text, p_new_key text)
RETURNS bytea LANGUAGE sql IMMUTABLE PARALLEL SAFE AS $$
    SELECT '\x1220'::bytea || sha256(convert_to(
        'cairn-vault-rekey' || E'\n' || cairn_vault_staff_id(p_staff_id) || E'\n' || p_role
        || E'\n' || p_new_key, 'UTF8'));
$$;

-- A grant or withdrawal of the enroller role. p_prior is the `seq` of the member's latest
-- grant/withdraw row (0 if none; cairn_vault_grant_prior): it binds the token to the state it
-- changes, so a grant token replayed after a withdrawal verifies against a different address.
CREATE OR REPLACE FUNCTION cairn_vault_grant_address(p_staff_id text, p_grant boolean, p_prior bigint)
RETURNS bytea LANGUAGE sql IMMUTABLE PARALLEL SAFE AS $$
    SELECT '\x1220'::bytea || sha256(convert_to(
        'cairn-vault-grant' || E'\n' || cairn_vault_staff_id(p_staff_id) || E'\n'
        || CASE p_grant WHEN true THEN 'grant' WHEN false THEN 'withdraw' END
        || E'\n' || p_prior::text, 'UTF8'));
$$;

-- A confirmation or repudiation of one key. Each happens at most once per key (a second confirm
-- or any act after a repudiation is refused), so a replay changes nothing.
CREATE OR REPLACE FUNCTION cairn_vault_judge_address(p_signing_key_id text, p_confirm boolean)
RETURNS bytea LANGUAGE sql IMMUTABLE PARALLEL SAFE AS $$
    SELECT '\x1220'::bytea || sha256(convert_to(
        'cairn-vault-judge' || E'\n' || p_signing_key_id || E'\n'
        || CASE p_confirm WHEN true THEN 'confirm' WHEN false THEN 'repudiate' END, 'UTF8'));
$$;

-- A reseal: replace THIS stored blob by THAT one. It binds the digest of the blob being replaced,
-- so a proof is good for one change only. A definer because it reads the stored blob; it returns
-- only the address — a hash over digests — never the blob or its digest on its own. NULL for an
-- unknown staff ID.
CREATE OR REPLACE FUNCTION cairn_vault_reseal_address(p_staff_id text, p_new_sealed bytea)
RETURNS bytea LANGUAGE sql STABLE SECURITY DEFINER SET search_path = public, pg_temp AS $$
    SELECT '\x1220'::bytea || sha256(convert_to(
        'cairn-vault-reseal' || E'\n' || m.staff_id || E'\n' || encode(sha256(m.sealed), 'hex')
        || E'\n' || encode(sha256(p_new_sealed), 'hex'), 'UTF8'))
      FROM vault_member m WHERE m.staff_id = cairn_vault_staff_id(p_staff_id);
$$;

-- The p_prior the window must sign a grant/withdraw over (see cairn_vault_grant_address). The
-- window cannot read actor_event, so it asks this. 0 for no grant history or no such member.
CREATE OR REPLACE FUNCTION cairn_vault_grant_prior(p_staff_id text)
RETURNS bigint LANGUAGE sql STABLE SECURITY DEFINER SET search_path = public, pg_temp AS $$
    SELECT COALESCE((SELECT g.seq FROM vault_member m, cairn_vault_latest_grant(m.actor_id) g
                      WHERE m.staff_id = cairn_vault_staff_id(p_staff_id)), 0);
$$;

-- The member row's custody columns, rewritten (a re-key or a reseal). Like db/061's
-- cairn_vault_insert_member, its one job beyond the UPDATE is to re-raise a violated constraint
-- by NAME only: Postgres's own error DETAIL ("Failing row contains (…)") would print the sealed
-- blob. Invoker rights, revoked from PUBLIC.
CREATE OR REPLACE FUNCTION cairn_vault_update_custody(
    p_staff_id text, p_signing_key_id text, p_sealed bytea, p_hint text, p_display_name text,
    p_phone text, p_email text)
RETURNS void LANGUAGE plpgsql AS $$
BEGIN
    UPDATE vault_member m
       SET signing_key_id = p_signing_key_id, sealed = p_sealed, hint = p_hint,
           display_name = p_display_name, contact_phone = p_phone, contact_email = p_email
     WHERE m.staff_id = p_staff_id;
EXCEPTION WHEN check_violation OR not_null_violation OR unique_violation THEN
    RAISE EXCEPTION 'vault update refused: %', SQLERRM;
END;
$$;

-- The shared body of both re-key doors, AFTER the approval was checked: give the person a new key
-- on their EXISTING actor, in one transaction.
--   * With a member row: the role must be the stored one (a re-key never changes the role: that
--     would be another actor); NULL display name / phone / email keep the stored ones; key, blob
--     and hint are replaced; the failure count is reset (success_mark moves up to every attempt
--     consumed so far — the counters are sequences, db/061 promise 4, so they never go back).
--   * With NO member row (after a restore, §5.8): the actor {role, handle = staff ID} must be in
--     the registry; a display name is required, and the table's CHECK requires a contact.
-- Either way a `rotate-key` row carries the new key and the approval (db/004 refuses a revoked or
-- superseded actor and any key ever held before), plus a `confirm` row when p_confirm (approved
-- by a superuser or the console). Invoker rights, revoked from PUBLIC.
CREATE OR REPLACE FUNCTION cairn_vault_install_key(
    p_staff_id text, p_role text, p_display_name text, p_phone text, p_email text,
    p_new_key text, p_sealed bytea, p_hint text,
    p_approved_by text, p_approval bytea, p_confirm boolean, p_enrolled_by text)
RETURNS bytea LANGUAGE plpgsql AS $$
DECLARE
    v_staff    text := cairn_vault_staff_id(p_staff_id);
    v_found    boolean;
    v_role     text;
    v_actor    bytea;
    v_name     text;
    v_phone    text;
    v_email    text;
    v_mark     bigint;
    v_failures bigint;
BEGIN
    SELECT m.role, m.actor_id, m.display_name, m.contact_phone, m.contact_email
      INTO v_role, v_actor, v_name, v_phone, v_email
      FROM vault_member m WHERE m.staff_id = v_staff FOR UPDATE;
    v_found := FOUND;
    IF v_found THEN
        IF v_role IS DISTINCT FROM p_role THEN
            RAISE EXCEPTION 'vault re-key refused: % is enrolled as %, not % — a re-key never changes the role',
                v_staff, v_role, COALESCE(p_role, 'NULL');
        END IF;
    ELSE
        IF NULLIF(btrim(p_display_name), '') IS NULL THEN
            RAISE EXCEPTION 'vault re-key refused: % has no member row (as after a restore), so a display name is required to re-create it', v_staff;
        END IF;
        v_actor := cairn_actor_id(jsonb_build_object('role', p_role, 'handle', v_staff));
        IF NOT EXISTS (SELECT 1 FROM actor_event e WHERE e.actor_id = v_actor AND e.op = 'enroll') THEN
            RAISE EXCEPTION 'vault re-key refused: no % actor with the handle % is in the registry — a person never enrolled here needs an enrolment, not a re-key',
                COALESCE(p_role, 'NULL'), v_staff;
        END IF;
    END IF;

    PERFORM cairn_actor_append(v_actor, 'rotate-key', p_new_key, p_approved_by, p_approval);
    IF p_confirm THEN
        PERFORM cairn_actor_append(v_actor, 'confirm', p_new_key, p_approved_by, p_approval);
    END IF;

    IF NOT v_found THEN
        PERFORM cairn_vault_insert_member(v_staff, p_display_name, p_role, v_actor, p_new_key,
                                          p_sealed, p_hint, p_phone, p_email, p_enrolled_by, NULL);
    ELSE
        PERFORM cairn_vault_update_custody(v_staff, p_new_key, p_sealed, p_hint,
                                           COALESCE(p_display_name, v_name),
                                           COALESCE(p_phone, v_phone), COALESCE(p_email, v_email));
        -- Under the attempt row's lock, as the release door counts (db/061 step 2).
        SELECT a.success_mark INTO v_mark FROM vault_attempt a WHERE a.staff_id = v_staff FOR UPDATE;
        SELECT s.failures INTO v_failures FROM cairn_vault_attempt_state(v_staff) s;
        UPDATE vault_attempt a SET success_mark = v_mark + v_failures WHERE a.staff_id = v_staff;
    END IF;
    RETURN v_actor;
END;
$$;

-- Re-key in person (§5.3, §5.8): approved by a superuser (confirmed at once) or an enroller
-- (unconfirmed). Returns the actor_id — the SAME one the member always had. Refuses any
-- `superuser` target (a locked-out superuser is the console's, cairn_vault_bootstrap_rekey) and,
-- for an enroller, an `admin` target. p_role is checked against the member row when one exists,
-- so the limits apply to the role the person really holds.
CREATE OR REPLACE FUNCTION cairn_vault_rekey(
    p_staff_id text, p_role text, p_display_name text, p_phone text, p_email text,
    p_new_key text, p_sealed bytea, p_hint text, p_approver_staff_id text, p_token bytea)
RETURNS bytea LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, pg_temp AS $$
DECLARE
    v_kind text;
    v_key  text;
BEGIN
    IF p_role = 'superuser' THEN
        RAISE EXCEPTION 'cairn_vault_rekey: a superuser is re-keyed only at the node''s console (vault-rekey-superuser), never by another member';
    END IF;
    v_kind := cairn_vault_approver_ok(p_approver_staff_id,
                                      cairn_vault_rekey_address(p_staff_id, p_role, p_new_key), p_token, true);
    IF v_kind = 'enroller' AND p_role = 'admin' THEN
        RAISE EXCEPTION 'cairn_vault_rekey: an enroller may not re-key a superuser or an admin — ask a superuser';
    END IF;
    SELECT m.signing_key_id INTO v_key
      FROM vault_member m WHERE m.staff_id = cairn_vault_staff_id(p_approver_staff_id);
    RETURN cairn_vault_install_key(p_staff_id, p_role, p_display_name, p_phone, p_email,
                                   p_new_key, p_sealed, p_hint, v_key, p_token,
                                   v_kind = 'superuser', cairn_vault_staff_id(p_approver_staff_id));
END;
$$;

-- Owner only (the node's console): re-key a SUPERUSER with nobody to approve it — the restore
-- case (§5.8: the first superuser back after a disk loss), or a locked-out superuser. The console
-- is the approver ('console', no token) and the key is confirmed at once. No grant: being able
-- to run this IS the authority, as for cairn_vault_bootstrap.
CREATE OR REPLACE FUNCTION cairn_vault_bootstrap_rekey(
    p_staff_id text, p_display_name text, p_new_key text, p_sealed bytea, p_hint text,
    p_phone text, p_email text)
RETURNS bytea LANGUAGE sql SECURITY DEFINER SET search_path = public, pg_temp AS $$
    SELECT cairn_vault_install_key(p_staff_id, 'superuser', p_display_name, p_phone, p_email,
                                   p_new_key, p_sealed, p_hint, 'console', NULL, true, 'console');
$$;

-- Grant or withdraw the enroller role (§5.1) — superuser only, signed over the member's CURRENT
-- grant state (cairn_vault_grant_address). The member row is locked first, so two acts on one
-- person cannot both read the same prior state. A superuser cannot hold the grant (they already
-- have every enroller power), and a no-op (granting a grant, withdrawing nothing) is refused.
CREATE OR REPLACE FUNCTION cairn_vault_set_enroller(p_staff_id text, p_grant boolean,
                                                    p_superuser_staff_id text, p_token bytea)
RETURNS void LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, pg_temp AS $$
DECLARE
    v_staff   text := cairn_vault_staff_id(p_staff_id);
    v_role    text;
    v_actor   bytea;
    v_prior   bigint;
    v_granted boolean;
    v_su_key  text;
BEGIN
    SELECT m.role, m.actor_id INTO v_role, v_actor FROM vault_member m WHERE m.staff_id = v_staff FOR UPDATE;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'cairn_vault_set_enroller: % is not an enrolled member', v_staff;
    END IF;
    IF v_role = 'superuser' THEN
        RAISE EXCEPTION 'cairn_vault_set_enroller: % is a superuser, who already holds every enroller power', v_staff;
    END IF;
    SELECT g.seq, g.granted INTO v_prior, v_granted FROM cairn_vault_latest_grant(v_actor) g;
    PERFORM cairn_vault_approver_ok(p_superuser_staff_id,
                                    cairn_vault_grant_address(v_staff, p_grant, COALESCE(v_prior, 0)),
                                    p_token, false);
    IF COALESCE(v_granted, false) = p_grant THEN
        RAISE EXCEPTION 'cairn_vault_set_enroller: % already % the enroller grant', v_staff,
            CASE WHEN p_grant THEN 'holds' ELSE 'does not hold' END;
    END IF;
    SELECT m.signing_key_id INTO v_su_key
      FROM vault_member m WHERE m.staff_id = cairn_vault_staff_id(p_superuser_staff_id);
    PERFORM cairn_actor_append(v_actor, CASE WHEN p_grant THEN 'grant-enroller' ELSE 'withdraw-enroller' END,
                               NULL, v_su_key, p_token);
END;
$$;

-- Confirm or repudiate one human key (§5.5) — superuser only. Confirm: only a key awaiting it
-- ('unconfirmed' or 'doubtful'). Repudiate: any key not already repudiated — a confirmed one
-- included (an impostor found later). Repudiation is FINAL for that key: nothing confirms it
-- again; the remedy is a new re-key. The key lock (enroll_actor's, same seed) serialises two
-- judgements of one key, so a confirm and a repudiate cannot both read the status first.
CREATE OR REPLACE FUNCTION cairn_vault_judge_key(p_signing_key_id text, p_confirm boolean,
                                                 p_superuser_staff_id text, p_token bytea)
RETURNS void LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, pg_temp AS $$
DECLARE
    v_actor  bytea;
    v_status text;
    v_su_key text;
BEGIN
    PERFORM cairn_vault_approver_ok(p_superuser_staff_id,
                                    cairn_vault_judge_address(p_signing_key_id, p_confirm), p_token, false);
    PERFORM pg_advisory_xact_lock(hashtextextended(p_signing_key_id, 1));
    SELECT e.actor_id INTO v_actor FROM actor_event e
     WHERE e.signing_key_id = p_signing_key_id AND e.op IN ('enroll', 'rotate-key') AND e.kind = 'human'
     LIMIT 1;
    IF v_actor IS NULL THEN
        RAISE EXCEPTION 'cairn_vault_judge_key: no person''s actor holds or held key %', p_signing_key_id;
    END IF;
    v_status := cairn_key_confirmation(p_signing_key_id);
    IF v_status = 'repudiated' THEN
        RAISE EXCEPTION 'cairn_vault_judge_key: key % is repudiated, which is final — the remedy is a new re-key', p_signing_key_id;
    END IF;
    IF p_confirm AND v_status NOT IN ('unconfirmed', 'doubtful') THEN
        RAISE EXCEPTION 'cairn_vault_judge_key: key % is %, not awaiting confirmation', p_signing_key_id, v_status;
    END IF;
    SELECT m.signing_key_id INTO v_su_key
      FROM vault_member m WHERE m.staff_id = cairn_vault_staff_id(p_superuser_staff_id);
    PERFORM cairn_actor_append(v_actor, CASE WHEN p_confirm THEN 'confirm' ELSE 'repudiate' END,
                               p_signing_key_id, v_su_key, p_token);
END;
$$;

-- Change my passphrase (§5.4): replace the member's sealed blob (and hint) with one the member
-- sealed themselves. p_proof is the member's OWN attestation over
-- cairn_vault_reseal_address(staff ID, p_new_sealed), checked against their key — so only someone
-- who just had the key released (purpose 'reseal', through the counted door) can make it, and only
-- for this exact change. The key does not change; no registry row is written. NULL p_hint clears
-- the hint (the form always sends what it shows). A caller who seals the WRONG seed only locks
-- themselves out, and the release door's key-id check (vault.rs, release_from_row) catches it.
CREATE OR REPLACE FUNCTION cairn_vault_reseal(p_staff_id text, p_new_sealed bytea, p_hint text,
                                              p_proof bytea)
RETURNS void LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, pg_temp AS $$
DECLARE
    v_staff text := cairn_vault_staff_id(p_staff_id);
    v_key   text;
    v_name  text;
    v_phone text;
    v_email text;
BEGIN
    SELECT m.signing_key_id, m.display_name, m.contact_phone, m.contact_email
      INTO v_key, v_name, v_phone, v_email
      FROM vault_member m WHERE m.staff_id = v_staff FOR UPDATE;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'cairn_vault_reseal: % is not an enrolled member', v_staff;
    END IF;
    IF cairn_attestation_ok(p_proof, cairn_vault_reseal_address(v_staff, p_new_sealed),
                            decode(v_key, 'hex')) IS NOT TRUE THEN
        RAISE EXCEPTION 'cairn_vault_reseal: the proof is not this member''s own attestation over this exact change';
    END IF;
    PERFORM cairn_vault_update_custody(v_staff, v_key, p_new_sealed, p_hint, v_name, v_phone, v_email);
END;
$$;

-- The superuser's worklist (§5.5): every CURRENT key still unconfirmed or doubtful, who approved
-- it and when. A key whose member row is gone (after a restore) is still listed, by its handle.
CREATE OR REPLACE FUNCTION cairn_vault_unconfirmed()
RETURNS TABLE(staff_id text, display_name text, signing_key_id text, approver_staff_id text,
              approved_at timestamptz, doubtful boolean)
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = public, pg_temp AS $$
    SELECT COALESCE(m.staff_id, a.pinned ->> 'handle'), m.display_name, a.signing_key_id,
           (SELECT h.staff_id FROM cairn_vault_key_holder(i.approved_by) h LIMIT 1),
           i.recorded_at, cairn_key_confirmation(a.signing_key_id) = 'doubtful'
      FROM actor_current a
      LEFT JOIN vault_member m ON m.actor_id = a.actor_id
      CROSS JOIN LATERAL (
          SELECT e.approved_by, e.recorded_at FROM actor_event e
           WHERE e.signing_key_id = a.signing_key_id AND e.op IN ('enroll', 'rotate-key')
             AND e.approved_by IS NOT NULL
           ORDER BY e.recorded_at DESC, e.seq DESC LIMIT 1) i
     WHERE cairn_key_confirmation(a.signing_key_id) IN ('unconfirmed', 'doubtful')
     ORDER BY i.recorded_at, 1;
$$;

-- The superuser's grant panel: every member who may hold the grant (everyone but superusers) and
-- whether they hold it NOW — the grant state, not cairn_vault_is_enroller (a granted member whose
-- key awaits confirmation still holds the grant; the panel must not offer to grant it again).
CREATE OR REPLACE FUNCTION cairn_vault_enrollers()
RETURNS TABLE(staff_id text, display_name text, enroller boolean)
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = public, pg_temp AS $$
    SELECT m.staff_id, m.display_name,
           COALESCE((SELECT g.granted FROM cairn_vault_latest_grant(m.actor_id) g), false)
      FROM vault_member m WHERE m.role <> 'superuser' ORDER BY m.staff_id;
$$;

-- Grants. Every definer and internal helper is first revoked from PUBLIC; the window (cairn_agent,
-- which cairn_window inherits, db/062) gets exactly the doors it calls. The pure address and
-- purpose functions keep PUBLIC EXECUTE, as cairn_vault_enrol_address does: they read nothing.
-- cairn_vault_bootstrap_rekey has NO grant: the console's alone.
REVOKE EXECUTE ON FUNCTION cairn_vault_reseal_address(text, bytea) FROM PUBLIC;
REVOKE EXECUTE ON FUNCTION cairn_vault_grant_prior(text) FROM PUBLIC;
REVOKE EXECUTE ON FUNCTION cairn_vault_update_custody(text, text, bytea, text, text, text, text) FROM PUBLIC;
REVOKE EXECUTE ON FUNCTION cairn_vault_install_key(text, text, text, text, text, text, bytea, text, text, bytea, boolean, text) FROM PUBLIC;
REVOKE EXECUTE ON FUNCTION cairn_vault_rekey(text, text, text, text, text, text, bytea, text, text, bytea) FROM PUBLIC;
REVOKE EXECUTE ON FUNCTION cairn_vault_bootstrap_rekey(text, text, text, bytea, text, text, text) FROM PUBLIC;
REVOKE EXECUTE ON FUNCTION cairn_vault_set_enroller(text, boolean, text, bytea) FROM PUBLIC;
REVOKE EXECUTE ON FUNCTION cairn_vault_judge_key(text, boolean, text, bytea) FROM PUBLIC;
REVOKE EXECUTE ON FUNCTION cairn_vault_reseal(text, bytea, text, bytea) FROM PUBLIC;
REVOKE EXECUTE ON FUNCTION cairn_vault_unconfirmed() FROM PUBLIC;
REVOKE EXECUTE ON FUNCTION cairn_vault_enrollers() FROM PUBLIC;

GRANT EXECUTE ON FUNCTION cairn_vault_reseal_address(text, bytea) TO cairn_agent;
GRANT EXECUTE ON FUNCTION cairn_vault_grant_prior(text) TO cairn_agent;
GRANT EXECUTE ON FUNCTION cairn_vault_rekey(text, text, text, text, text, text, bytea, text, text, bytea) TO cairn_agent;
GRANT EXECUTE ON FUNCTION cairn_vault_set_enroller(text, boolean, text, bytea) TO cairn_agent;
GRANT EXECUTE ON FUNCTION cairn_vault_judge_key(text, boolean, text, bytea) TO cairn_agent;
GRANT EXECUTE ON FUNCTION cairn_vault_reseal(text, bytea, text, bytea) TO cairn_agent;
GRANT EXECUTE ON FUNCTION cairn_vault_unconfirmed() TO cairn_agent;
GRANT EXECUTE ON FUNCTION cairn_vault_enrollers() TO cairn_agent;

COMMIT;
