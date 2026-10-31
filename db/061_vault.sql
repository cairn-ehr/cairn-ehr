-- Cairn — the clinicians' key vault (GUI sign-in slice 1; design:
-- docs/superpowers/specs/2026-10-31-gui-sign-in-and-authorship-design.md).
--
-- WHAT THIS IS. Every human signature in Cairn needs that human's Ed25519 key. Until now a key
-- lived in a FILE (cairn-keystore's sealed key file) on the machine the person sat at. A shared
-- ward terminal cannot work that way: the next nurse to walk up must sign as herself, from any
-- terminal, with nothing but her staff ID and passphrase. So each clinician's seed now sits HERE,
-- in `vault_member.sealed`, sealed under their own passphrase (the same sealed-key format the
-- file used, `cairn_keystore::seal`), and the database hands it out for one act at a time.
--
-- WHY THESE ARE MUTABLE TABLES IN AN APPEND-ONLY SYSTEM. Principle 1 (append-only) governs
-- CLINICAL CONTENT: events, the record. The vault is not the record. It is key CUSTODY — the
-- in-database replacement for a key file, which was always mutable state on a disk (re-sealed on
-- a passphrase change, deleted on decommission). A failure counter and a terminal passphrase are
-- the same kind of thing: operational state about who may unlock what, right now. What IS
-- clinical-grade — that a person was enrolled as an actor, with which key — still goes through
-- the append-only actor registry (`enroll_actor`, db/004), in the same transaction as the vault
-- row. So nothing a reviewer must be able to audit later lives only in these tables.
--
-- THE THREE PROMISES THIS FILE KEEPS (and how):
--
--   1. The wrapped key never leaves the database. `vault_member` has NO grant to anyone (not
--      PUBLIC, not cairn_agent, not cairn_node), and no function, view or door returns the
--      `sealed` column. Even the door that checks a passphrase (`cairn_vault_release`) returns
--      the unwrapped seed — for one act — never the blob. Why it matters: a sealed blob that
--      leaves is an OFFLINE brute-force target, where no back-off can reach the guesser. While it
--      stays here every guess goes through the back-off below.
--
--   2. The passphrase is never in an error. `cairn_vault_release` NEVER RAISES — a wrong
--      passphrase, an unknown staff ID, a forbidden purpose and a back-off are all ANSWERS, rows
--      with an `outcome`. Two reasons. (a) A raised error can carry the statement's bind
--      parameters — the passphrase — into the server log. (b) A raise rolls back the whole
--      transaction, INCLUDING the failure counter we just incremented: a guesser could then guess
--      forever at full speed. So failures are recorded with a plain UPDATE and reported with
--      RETURN QUERY. (The primitive it calls, cairn_pgx's `cairn_vault_unseal`, likewise returns
--      NULL rather than raising.)
--
--   3. A superuser's key is never released for a clinical act. Spec §5.1: the superuser role
--      administers the site (it enrols people); it confers NO clinical authorship. A
--      clinician-superuser therefore has TWO members — two staff IDs, two keys — one per role.
--      This is enforced HERE, at the vault, not only in the window: the release door maps role
--      to the one purpose it may serve (superuser → 'enrol', everyone else → 'clinical') and
--      answers 'refused-purpose' for any other pairing.
--
-- THE BACK-OFF, AND WHY IT IS NEVER A LOCK-OUT. Failures 1–3 are free (people mistype). From the
-- 4th, the member must wait `least(2^(failures-4), 300)` seconds before the next try: 1 s, 2 s,
-- 4 s … capped at five minutes. A try during the wait answers 'waiting' with the milliseconds
-- left, does NOT check the passphrase, and does NOT count as a failure — so a clinician pressing
-- again too soon cannot dig herself deeper. A success resets the count. There is deliberately no
-- hard lock-out: a locked-out clinician in an emergency is a paper-parity failure (paper never
-- refuses a pen), and five minutes per guess already makes an online guess at a real passphrase
-- hopeless.
--
-- WHO MAY CALL WHAT:
--   * cairn_agent (the window's runtime role): cairn_vault_release, cairn_vault_hint,
--     cairn_vault_terminal_phc, cairn_vault_enrol (which itself demands a superuser's token).
--   * owner only (the node's console, `cairn-node vault-bootstrap-superuser` and friends):
--     cairn_vault_bootstrap, cairn_vault_set_terminal_phc.
--   * `cairn_vault_unseal` itself is REVOKED from PUBLIC below, so the back-off cannot be skipped
--     by calling the primitive directly: cairn_vault_release (a SECURITY DEFINER) is its only door.

BEGIN;

-- Load-time dependency gate (the db/026 pattern). The release door is PL/pgSQL, so its call to
-- cairn_vault_unseal is LATE-BOUND: against a stale (pre-0.4.0) cairn_pgx this file would load
-- cleanly and the failure would surface only at a clinician's first sign-in, as an illegible
-- `undefined function`. Refuse the load itself, legibly, for every loader.
DO $$
BEGIN
    IF to_regprocedure('cairn_vault_unseal(bytea, text)') IS NULL THEN
        RAISE EXCEPTION 'db/061 requires cairn_pgx >= 0.4.0: cairn_vault_unseal(bytea, text) is not installed'
            USING HINT = 'The installed extension library is stale. Rebuild + reinstall it '
                         || '(`cargo pgrx install` against this cluster''s PostgreSQL), then re-run init.';
    END IF;
END;
$$;

-- The ONE definition of a normalised staff ID: `lower(btrim(x))`. Used by the table's CHECK, the
-- enrolment address and every lookup, so " JSmith " typed at a terminal finds the `jsmith` who was
-- enrolled. One home, so the CHECK and the lookups can never disagree (durable-rules trap 16).
-- COLLATE pg_unicode_fast (trap 17): a bare lower() follows the cluster's collation, which differs
-- between ICU and libc builds for non-ASCII letters; this pins full Unicode case mapping on every
-- node — the same mapping Rust's `to_lowercase` uses for the address twin. Pure.
CREATE OR REPLACE FUNCTION cairn_vault_staff_id(p_staff_id text)
RETURNS text LANGUAGE sql IMMUTABLE PARALLEL SAFE AS $$
    SELECT lower(btrim(p_staff_id) COLLATE pg_unicode_fast);
$$;

-- The back-off schedule, as a pure function of the failure count: NULL (no wait) for failures
-- 1–3, then 2^(failures-4) seconds capped at 300. The exponent is clamped at 9 (2^9 = 512 > 300)
-- so a very large count can never overflow `power` — this runs inside a door that must never raise.
CREATE OR REPLACE FUNCTION cairn_vault_backoff_seconds(p_failures int)
RETURNS int LANGUAGE sql IMMUTABLE PARALLEL SAFE AS $$
    SELECT CASE WHEN p_failures >= 4
                THEN least(power(2, least(p_failures - 4, 9))::int, 300)
           END;
$$;

-- One row per ROLE-ACTOR (a clinician who is also a superuser has two rows, two staff IDs; #168
-- makes the person behind them first-class). `is_superuser` is `role = 'superuser'` — never a
-- second column that could disagree with it. Constraints are NAMED so a refusal says which rule
-- failed.
CREATE TABLE IF NOT EXISTS vault_member (
    staff_id       text PRIMARY KEY
                   CONSTRAINT vault_member_staff_id_is_normalised
                   CHECK (staff_id = cairn_vault_staff_id(staff_id) AND staff_id <> ''),
    display_name   text NOT NULL,
    role           text NOT NULL,
    actor_id       bytea NOT NULL,
    -- Lower-case hex Ed25519 public key, as actor_event stores it. The format CHECK is what makes
    -- the enrol door's `decode(…, 'hex')` of a superuser's key safe.
    signing_key_id text NOT NULL UNIQUE
                   CONSTRAINT vault_member_signing_key_id_is_hex
                   CHECK (signing_key_id ~ '^[0-9a-f]{64}$'),
    sealed         bytea NOT NULL,      -- the passphrase-sealed seed; NEVER selected by anyone
    hint           text,
    contact_phone  text,
    contact_email  text,
    enrolled_by    text NOT NULL,       -- the enrolling superuser's staff_id, or 'console'
    enrolled_at    timestamptz NOT NULL DEFAULT clock_timestamp(),
    -- Someone has to be able to reach this person to re-enrol them after a forgotten passphrase.
    CONSTRAINT vault_member_needs_a_contact
        CHECK (contact_phone IS NOT NULL OR contact_email IS NOT NULL)
);

-- The failure counter behind the back-off. Created lazily by the first release attempt.
CREATE TABLE IF NOT EXISTS vault_attempt (
    staff_id        text PRIMARY KEY REFERENCES vault_member,
    failures        int NOT NULL DEFAULT 0,
    next_allowed_at timestamptz
);

-- One shared staff passphrase per site that unlocks a terminal's window (spec §9 left the shape to
-- the builder; this is the call). Stored as an Argon2 PHC string, checked by the window.
CREATE TABLE IF NOT EXISTS vault_terminal (
    singleton  boolean PRIMARY KEY DEFAULT true CHECK (singleton),
    unlock_phc text NOT NULL
);

-- No table is readable or writable by anyone but the owner. Stated explicitly (the db/004
-- trust-anchor posture), so a stray GRANT stands out in review; the doors below are the only way in.
REVOKE ALL ON vault_member, vault_attempt, vault_terminal FROM PUBLIC, cairn_agent, cairn_node;

-- The address a superuser's attestation token signs to approve ONE enrolment: a sha2-256
-- multihash (the 0x12 0x20 prefix, as every Cairn content address) over the normalised staff ID,
-- the new key and the role. A token over any other request — another staff ID, another key, a
-- promotion to superuser — verifies against a different address and is refused. Pure and IMMUTABLE,
-- so the window's Rust twin can be checked against it byte-for-byte. Deliberately NOT a definer:
-- it reads no table, so a definer would add privilege for nothing.
CREATE OR REPLACE FUNCTION cairn_vault_enrol_address(p_staff_id text, p_signing_key_id text, p_role text)
RETURNS bytea LANGUAGE sql IMMUTABLE PARALLEL SAFE AS $$
    SELECT '\x1220'::bytea || sha256(convert_to(
        'cairn-vault-enrol' || E'\n' || cairn_vault_staff_id(p_staff_id)
        || E'\n' || p_signing_key_id || E'\n' || p_role, 'UTF8'));
$$;

-- The release door: check a passphrase and, for the ONE purpose this member's role may serve,
-- hand back their seed. NEVER RAISES (see the header, promise 2): every outcome is a row.
--
--   outcome          seed / signing_key_id   retry_after_ms
--   'released'       set                     NULL
--   'wrong'          NULL                    NULL for failures 1–3, else the new wait
--   'waiting'        NULL                    ms until the next try may be made
--   'unknown'        NULL                    NULL   (no such staff ID; nothing is counted)
--   'refused-purpose' NULL                   NULL   (superuser asked for 'clinical', or anyone
--                                                    else asked for 'enrol', or a purpose outside
--                                                    the vocabulary — checked BEFORE any unsealing,
--                                                    so a key that will not be released is never
--                                                    even unwrapped)
--
-- The attempt row is locked FOR UPDATE before the wait is checked, so two terminals guessing the
-- same staff ID at once are serialised and cannot both slip past one back-off.
--
-- Columns are table-qualified throughout: the OUT column `signing_key_id` would otherwise be
-- ambiguous with vault_member's column of the same name.
CREATE OR REPLACE FUNCTION cairn_vault_release(p_staff_id text, p_passphrase text, p_purpose text)
RETURNS TABLE(outcome text, seed bytea, signing_key_id text, retry_after_ms bigint)
LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, pg_temp AS $$
DECLARE
    v_staff_id text := cairn_vault_staff_id(p_staff_id);
    v_member   vault_member%ROWTYPE;
    v_failures int;
    v_wait     timestamptz;
    v_seed     bytea;
    v_backoff  int;
BEGIN
    SELECT * INTO v_member FROM vault_member m WHERE m.staff_id = v_staff_id;
    IF NOT FOUND THEN
        RETURN QUERY SELECT 'unknown'::text, NULL::bytea, NULL::text, NULL::bigint;
        RETURN;
    END IF;

    -- §5.1: a superuser key serves enrolment only; every other key serves clinical acts only.
    IF p_purpose IS DISTINCT FROM
       (CASE WHEN v_member.role = 'superuser' THEN 'enrol' ELSE 'clinical' END) THEN
        RETURN QUERY SELECT 'refused-purpose'::text, NULL::bytea, NULL::text, NULL::bigint;
        RETURN;
    END IF;

    INSERT INTO vault_attempt (staff_id) VALUES (v_staff_id) ON CONFLICT DO NOTHING;
    SELECT a.failures, a.next_allowed_at INTO v_failures, v_wait
      FROM vault_attempt a WHERE a.staff_id = v_staff_id FOR UPDATE;

    -- Backed off: answer with the time left. No unsealing, no counting.
    IF v_wait > clock_timestamp() THEN
        RETURN QUERY SELECT 'waiting'::text, NULL::bytea, NULL::text,
            greatest(1, ceil(extract(epoch FROM v_wait - clock_timestamp()) * 1000))::bigint;
        RETURN;
    END IF;

    v_seed := cairn_vault_unseal(v_member.sealed, p_passphrase);   -- NULL = wrong; never raises

    IF v_seed IS NULL THEN
        v_failures := v_failures + 1;
        v_backoff := cairn_vault_backoff_seconds(v_failures);       -- NULL for failures 1–3
        UPDATE vault_attempt a
           SET failures = v_failures,
               next_allowed_at = clock_timestamp() + make_interval(secs => v_backoff)
         WHERE a.staff_id = v_staff_id;
        RETURN QUERY SELECT 'wrong'::text, NULL::bytea, NULL::text, v_backoff::bigint * 1000;
        RETURN;
    END IF;

    UPDATE vault_attempt a SET failures = 0, next_allowed_at = NULL WHERE a.staff_id = v_staff_id;
    RETURN QUERY SELECT 'released'::text, v_seed, v_member.signing_key_id, NULL::bigint;
END;
$$;

-- The member's own passphrase hint (shown on request at the sign-in strip). NULL if none, or no
-- such staff ID.
CREATE OR REPLACE FUNCTION cairn_vault_hint(p_staff_id text)
RETURNS text LANGUAGE sql STABLE SECURITY DEFINER SET search_path = public, pg_temp AS $$
    SELECT m.hint FROM vault_member m WHERE m.staff_id = cairn_vault_staff_id(p_staff_id);
$$;

-- The site's terminal-unlock PHC string, for the window to check the shared staff passphrase
-- against. NULL until the console has set one.
CREATE OR REPLACE FUNCTION cairn_vault_terminal_phc()
RETURNS text LANGUAGE sql STABLE SECURITY DEFINER SET search_path = public, pg_temp AS $$
    SELECT t.unlock_phc FROM vault_terminal t;
$$;

-- Owner only (the node's console): set or replace the terminal-unlock PHC string.
CREATE OR REPLACE FUNCTION cairn_vault_set_terminal_phc(p_phc text)
RETURNS void LANGUAGE sql SECURITY DEFINER SET search_path = public, pg_temp AS $$
    INSERT INTO vault_terminal (singleton, unlock_phc) VALUES (true, p_phc)
    ON CONFLICT (singleton) DO UPDATE SET unlock_phc = EXCLUDED.unlock_phc;
$$;

-- The shared insert behind both enrolment doors. Its one job beyond the INSERT: a violated
-- constraint is re-raised with the constraint's NAME only. Postgres's own CHECK / NOT NULL error
-- carries a DETAIL "Failing row contains (…)" that would print every column — the sealed blob
-- included — into the client's error and the server log. Catching it and raising SQLERRM alone
-- (which names the constraint, never the values) keeps the blob out. Invoker-rights and revoked
-- from PUBLIC: it is reached only from inside the two definer doors below.
CREATE OR REPLACE FUNCTION cairn_vault_insert_member(
    p_staff_id text, p_display_name text, p_role text, p_actor_id bytea, p_signing_key_id text,
    p_sealed bytea, p_hint text, p_phone text, p_email text, p_enrolled_by text)
RETURNS void LANGUAGE plpgsql AS $$
BEGIN
    INSERT INTO vault_member (staff_id, display_name, role, actor_id, signing_key_id, sealed,
                              hint, contact_phone, contact_email, enrolled_by)
    VALUES (cairn_vault_staff_id(p_staff_id), p_display_name, p_role, p_actor_id,
            p_signing_key_id, p_sealed, p_hint, p_phone, p_email, p_enrolled_by);
EXCEPTION WHEN check_violation OR not_null_violation OR unique_violation THEN
    RAISE EXCEPTION 'vault enrolment refused: %', SQLERRM;
END;
$$;

-- Enrol a member, approved by a superuser. Refuses unless ALL of:
--   (a) p_superuser_staff_id names a member whose role is 'superuser';
--   (b) that superuser's key is a current (enrolled, non-revoked) actor;
--   (c) p_token is that superuser's valid attestation over cairn_vault_enrol_address of THIS
--       request — so a token cannot be replayed for another person, key or role;
--   (d) the pinned determinant set's role equals p_role (the registry and the vault must agree
--       on what this actor is).
-- Then the actor is enrolled in the append-only registry and the vault row inserted, in one
-- transaction: either both exist or neither does. Refusals name the failed check and never echo
-- p_sealed or p_token.
CREATE OR REPLACE FUNCTION cairn_vault_enrol(
    p_staff_id text, p_display_name text, p_role text, p_pinned jsonb, p_signing_key_id text,
    p_sealed bytea, p_hint text, p_phone text, p_email text, p_superuser_staff_id text,
    p_token bytea)
RETURNS bytea LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, pg_temp AS $$
DECLARE
    v_superuser text := cairn_vault_staff_id(p_superuser_staff_id);
    v_su_key    text;
    v_actor_id  bytea;
BEGIN
    SELECT m.signing_key_id INTO v_su_key
      FROM vault_member m WHERE m.staff_id = v_superuser AND m.role = 'superuser';
    IF v_su_key IS NULL THEN
        RAISE EXCEPTION 'cairn_vault_enrol: % is not an enrolled superuser — only a superuser may enrol', v_superuser;
    END IF;
    IF NOT EXISTS (SELECT 1 FROM actor_current a WHERE a.signing_key_id = v_su_key) THEN
        RAISE EXCEPTION 'cairn_vault_enrol: superuser %''s key is not a current (enrolled, non-revoked) actor', v_superuser;
    END IF;
    -- IS NOT TRUE, not NOT(...): a NULL token or address yields NULL, which must refuse.
    IF cairn_attestation_ok(p_token,
                            cairn_vault_enrol_address(p_staff_id, p_signing_key_id, p_role),
                            decode(v_su_key, 'hex')) IS NOT TRUE THEN
        RAISE EXCEPTION 'cairn_vault_enrol: the superuser''s attestation is invalid or does not cover this enrolment (staff ID, key and role)';
    END IF;
    IF p_pinned ->> 'role' IS DISTINCT FROM p_role THEN
        RAISE EXCEPTION 'cairn_vault_enrol: the pinned role % does not match the enrolled role %',
            p_pinned ->> 'role', p_role;
    END IF;

    v_actor_id := enroll_actor('human', p_pinned, p_signing_key_id);
    PERFORM cairn_vault_insert_member(p_staff_id, p_display_name, p_role, v_actor_id,
                                      p_signing_key_id, p_sealed, p_hint, p_phone, p_email,
                                      v_superuser);
    RETURN v_actor_id;
END;
$$;

-- Owner only (the node's console): enrol a superuser with no superuser to approve it — the first
-- one on a new site, or the recovery of a locked-out last one. The role is forced to 'superuser'
-- and `enrolled_by` to 'console'; the pinned set must say so too. No grant: being able to run this
-- IS the authority (whoever holds the database owner's credentials already holds the site).
CREATE OR REPLACE FUNCTION cairn_vault_bootstrap(
    p_staff_id text, p_display_name text, p_pinned jsonb, p_signing_key_id text, p_sealed bytea,
    p_hint text, p_phone text, p_email text)
RETURNS bytea LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, pg_temp AS $$
DECLARE v_actor_id bytea;
BEGIN
    IF p_pinned ->> 'role' IS DISTINCT FROM 'superuser' THEN
        RAISE EXCEPTION 'cairn_vault_bootstrap: the pinned role must be superuser, not %', p_pinned ->> 'role';
    END IF;
    v_actor_id := enroll_actor('human', p_pinned, p_signing_key_id);
    PERFORM cairn_vault_insert_member(p_staff_id, p_display_name, 'superuser', v_actor_id,
                                      p_signing_key_id, p_sealed, p_hint, p_phone, p_email,
                                      'console');
    RETURN v_actor_id;
END;
$$;

-- Grants. Functions default to PUBLIC EXECUTE, so every door is first revoked from PUBLIC and then
-- granted to exactly the role the header names.
REVOKE EXECUTE ON FUNCTION cairn_vault_release(text, text, text) FROM PUBLIC;
REVOKE EXECUTE ON FUNCTION cairn_vault_hint(text) FROM PUBLIC;
REVOKE EXECUTE ON FUNCTION cairn_vault_terminal_phc() FROM PUBLIC;
REVOKE EXECUTE ON FUNCTION cairn_vault_enrol(text, text, text, jsonb, text, bytea, text, text, text, text, bytea) FROM PUBLIC;
REVOKE EXECUTE ON FUNCTION cairn_vault_bootstrap(text, text, jsonb, text, bytea, text, text, text) FROM PUBLIC;
REVOKE EXECUTE ON FUNCTION cairn_vault_set_terminal_phc(text) FROM PUBLIC;
REVOKE EXECUTE ON FUNCTION cairn_vault_insert_member(text, text, text, bytea, text, bytea, text, text, text, text) FROM PUBLIC;
-- The unwrapping primitive: unthrottled (one Argon2id per call), so reachable only through the
-- release door, which counts and backs off.
REVOKE EXECUTE ON FUNCTION cairn_vault_unseal(bytea, text) FROM PUBLIC;

GRANT EXECUTE ON FUNCTION cairn_vault_release(text, text, text) TO cairn_agent;
GRANT EXECUTE ON FUNCTION cairn_vault_hint(text) TO cairn_agent;
GRANT EXECUTE ON FUNCTION cairn_vault_terminal_phc() TO cairn_agent;
GRANT EXECUTE ON FUNCTION cairn_vault_enrol(text, text, text, jsonb, text, bytea, text, text, text, text, bytea) TO cairn_agent;

COMMIT;
