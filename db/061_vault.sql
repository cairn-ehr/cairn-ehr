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
-- row. The APPROVAL of every key — the approver's signed attestation token — is kept APPEND-ONLY
-- on its registry row (`actor_event.approved_by` / `approval`, db/004; 'console' and no token for
-- a console bootstrap), since sign-in slice 2: a re-key overwrites the member row, and who
-- approved every key there ever was must survive it. The member row still keeps the FIRST
-- enrolment's token (`vault_member.enrol_token`), which re-verifies only against the key it
-- enrolled.
--
-- THE FOUR PROMISES THIS FILE KEEPS (and how):
--
--   1. The wrapped key never leaves the database — for every role but the owner (and PostgreSQL
--      superusers, whom no grant binds; the window connected as the owner when this was written, and since
--      ADR-0084 (#898) runs on its own login, cairn_window, db/062).
--      `vault_member` has NO grant to anyone (not PUBLIC, not cairn_agent, not cairn_node), and no
--      function, view or door returns the `sealed` column. Even the door that checks a passphrase (`cairn_vault_release`) returns
--      the unwrapped seed — for one act — never the blob. Why it matters: a sealed blob that
--      leaves is an OFFLINE brute-force target, where no back-off can reach the guesser. While it
--      stays here every guess goes through the back-off below.
--
--   2. The passphrase is never in an error. `cairn_vault_release` NEVER RAISES for a wrong
--      passphrase, an unknown staff ID, a forbidden purpose or a back-off: they are all ANSWERS,
--      rows with an `outcome`, reported with RETURN QUERY. A raised error can carry the
--      statement's bind parameters — the passphrase — into the server log. (The primitive it
--      calls, cairn_pgx's `cairn_vault_unseal`, likewise returns NULL rather than raising.)
--      "Never raises" is a promise at the default READ COMMITTED: a REPEATABLE READ / SERIALIZABLE
--      caller can get a 40001 on the row lock, and a lock or statement timeout can cancel the
--      call. None of those loses a count (see promise 4) — they can only refuse to answer.
--
--   3. A superuser's key is never released for a clinical act. Design §5.1: the superuser role
--      administers the site (it enrols people); it confers NO clinical authorship. A
--      clinician-superuser therefore has TWO members — two staff IDs, two keys — one per role.
--      This is enforced HERE, at the vault, not only in the window: the release door maps role
--      to the purposes it may serve and answers 'refused-purpose' for any other pairing. Since
--      sign-in slice 2 (cairn_vault_purpose_allowed, below): superuser and admin → 'enrol';
--      everyone else → 'clinical', plus 'enrol' while an enroller; every role → 'reseal' (re-wrap
--      one's own key under a new passphrase). An enrolment and a reseal are each signed over
--      their own domain-separated address, so a key released for one can never sign a clinical
--      event by mistake.
--
--   4. EVERY GUESS IS COUNTED WHERE A ROLLBACK CANNOT UNDO IT — before anything is revealed.
--      An ordinary counter column is TRANSACTIONAL: the door increments it, returns 'wrong', and
--      the caller then ends the transaction with ROLLBACK (or runs the call inside a statement
--      that errors after reading the outcome, e.g. `SELECT 1/(r.outcome <> 'wrong')::int FROM
--      cairn_vault_release(…) r`). The caller has learned the answer, and the increment is gone.
--      Any client holding the runtime role could then guess forever at full speed. So the count
--      lives in two PostgreSQL SEQUENCES per member, created at enrolment: sequence changes are
--      NEVER rolled back (PostgreSQL docs, "Sequence Manipulation Functions"). The door moves the
--      attempt counter up by one and stamps the attempt clock (the epoch in ms), both with
--      `setval`, BEFORE it unseals, so the guess is on record whatever the caller does next.
--      (`setval`, not `nextval`, so that a crash replays the exact count — see
--      cairn_vault_attempt_state.)
--      The only transactional piece is `vault_attempt.success_mark` — the attempt number of the
--      last success — and failures = attempts − success_mark. Rolling back a SUCCESS therefore
--      just leaves that attempt counted as a failure: it can only ever hurt the caller.
--      Residual, stated plainly: the attempt row is locked FOR UPDATE for the whole call, so a
--      raw-SQL client that holds a transaction open after calling the door holds that lock and
--      delays THAT ONE staff member's next sign-in until it lets go; each press meanwhile fails
--      when the window cuts its own command off (ANSWER_WITHIN, 15 s). That is a denial of
--      service by someone who already holds the runtime role; it is never a bypass.
--
-- THE BACK-OFF, AND WHY IT HAS NO HARD LOCK-OUT. Failures 1–3 are free (people mistype). From the
-- 4th, the member must wait `least(2^(failures-4), 300)` seconds before the next try: 1 s, 2 s,
-- 4 s … capped at five minutes. A try during the wait answers 'waiting' with the milliseconds
-- left, does NOT check the passphrase, and does NOT count as a failure — so a clinician pressing
-- again too soon cannot dig herself deeper. A success resets the count. There is deliberately no
-- hard lock-out: a locked-out clinician in an emergency is a paper-parity failure (paper never
-- refuses a pen), and five minutes per guess already makes an online guess at a real passphrase
-- hopeless. The residual, stated plainly: failures never decay, so another person's wrong guesses
-- can keep a named member at the five-minute cap (#903).
--
-- WHO MAY CALL WHAT:
--   * cairn_agent (the window's runtime role): cairn_vault_release, cairn_vault_hint, cairn_vault_staff_id_taken,
--     cairn_vault_terminal_phc, cairn_vault_enrol (which itself demands a superuser's or a
--     confirmed enroller's token), and slice 2's doors in db/063.
--   * owner only (the node's console, `cairn-node vault-bootstrap-superuser` and friends):
--     cairn_vault_bootstrap, cairn_vault_set_terminal_phc, and db/063's cairn_vault_bootstrap_rekey.
--   * `cairn_vault_unseal` itself is REVOKED from PUBLIC below, so the back-off cannot be skipped
--     by calling the primitive directly: for every role but the owner, cairn_vault_release (a
--     SECURITY DEFINER) is its only door. The load refuses if that REVOKE did not take.

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
-- node. Pure.
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
-- makes the person behind them first-class). There is no is_superuser column: a superuser is
-- `role = 'superuser'`, never a second column that could disagree with it. Constraints are NAMED so a refusal says which rule
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
    enrolled_by    text NOT NULL,       -- the approver's staff_id (superuser or enroller), or 'console'
    -- The approver's attestation token over cairn_vault_enrol_address of this row's FIRST
    -- enrolment (NULL for a console bootstrap, and for a member re-created by a re-key after a
    -- restore). A later re-key leaves it; that key's approval is on its actor_event row (header).
    enrol_token    bytea,
    enrolled_at    timestamptz NOT NULL DEFAULT clock_timestamp(),
    -- Names this member's two attempt sequences (promise 4). A random uuid, never the staff ID
    -- text, so no typed value ever becomes part of an identifier in dynamic DDL.
    attempt_series_id uuid NOT NULL UNIQUE DEFAULT gen_random_uuid(),
    -- Someone has to be able to reach this person to re-enrol them after a forgotten passphrase.
    CONSTRAINT vault_member_needs_a_contact
        CHECK (contact_phone IS NOT NULL OR contact_email IS NOT NULL),
    -- The enrolment address joins staff ID and role with newlines, so a newline (or any control
    -- character) inside either could make two different requests share one signed preimage.
    CONSTRAINT vault_member_has_no_control_characters
        CHECK (staff_id !~ '[[:cntrl:]]' AND role !~ '[[:cntrl:]]')
);

-- The TRANSACTIONAL half of the attempt count (promise 4): the attempt number of this member's
-- last successful release. failures = (attempts consumed) − success_mark. The row also serves as
-- the per-member lock that serialises concurrent guesses. Created with the member (trigger below).
CREATE TABLE IF NOT EXISTS vault_attempt (
    staff_id     text PRIMARY KEY REFERENCES vault_member,
    success_mark bigint NOT NULL DEFAULT 0
);

-- The name of one of a member's two attempt sequences: 'attempts' (an attempt counter, one
-- step per checked guess) or 'clock' (the epoch-ms of the last checked guess, via `setval`).
-- Derived from the member's attempt_series_id only. One home for the naming, so the trigger that
-- creates them and the door that uses them can never disagree. Pure.
CREATE OR REPLACE FUNCTION cairn_vault_sequence_name(p_series uuid, p_kind text)
RETURNS text LANGUAGE sql IMMUTABLE PARALLEL SAFE AS $$
    SELECT 'vault_' || p_kind || '_' || replace(p_series::text, '-', '');
$$;

-- Every new member gets its attempt row and its two sequences, whoever inserted it (a trigger, so
-- even a raw owner INSERT cannot produce a member the release door cannot count). The sequences
-- are OWNED BY the member's attempt_series_id column, so they go with the table, and carry no
-- grant: nobody but the owner can `setval` a clock back or a counter down.
CREATE OR REPLACE FUNCTION cairn_vault_member_created()
RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    EXECUTE format('CREATE SEQUENCE public.%I OWNED BY public.vault_member.attempt_series_id',
                   cairn_vault_sequence_name(NEW.attempt_series_id, 'attempts'));
    EXECUTE format('CREATE SEQUENCE public.%I OWNED BY public.vault_member.attempt_series_id',
                   cairn_vault_sequence_name(NEW.attempt_series_id, 'clock'));
    INSERT INTO vault_attempt (staff_id) VALUES (NEW.staff_id);
    RETURN NULL;
END;
$$;
DROP TRIGGER IF EXISTS vault_member_created ON vault_member;
CREATE TRIGGER vault_member_created AFTER INSERT ON vault_member
    FOR EACH ROW EXECUTE FUNCTION cairn_vault_member_created();

-- A member's attempt state, read WITHOUT writing: failures since the last success, and the
-- epoch-ms of the last checked guess (NULL if none yet). No row for an unknown staff ID. Reading a
-- sequence relation directly (`SELECT last_value, is_called`) consumes nothing; `is_called` is
-- false until the first setval. Invoker-rights, owner only: the release door calls it.
--
-- WHY THE DOOR COUNTS WITH setval, NEVER nextval. Both are non-transactional, but they differ
-- after a CRASH. For speed, `nextval` writes the sequence to the WAL 32 values AHEAD of what it
-- hands out (`log_cnt`), and crash recovery restores that logged value. A member who guessed
-- once just before a power cut would wake up with ~32 phantom failures: made to wait with the
-- RIGHT passphrase for up to 300 s, and charged 300 s for the first mistype after — a lock-out in
-- all but name, at exactly the moment (a remote clinic coming back from a power cut) it hurts
-- most. `setval(seq, value, true)` WAL-logs the exact value (`log_cnt` = 0), so recovery replays
-- precisely the attempts that were counted. The clock is set with setval for the same reason.
CREATE OR REPLACE FUNCTION cairn_vault_attempt_state(p_staff_id text)
RETURNS TABLE(failures bigint, last_attempt_ms bigint) LANGUAGE plpgsql STABLE AS $$
DECLARE
    v_series uuid;
    v_mark   bigint;
    v_count  bigint;
    v_last   bigint;
BEGIN
    SELECT m.attempt_series_id, a.success_mark INTO v_series, v_mark
      FROM vault_member m JOIN vault_attempt a ON a.staff_id = m.staff_id
     WHERE m.staff_id = cairn_vault_staff_id(p_staff_id);
    IF NOT FOUND THEN
        RETURN;
    END IF;
    EXECUTE format('SELECT CASE WHEN is_called THEN last_value ELSE 0 END FROM public.%I',
                   cairn_vault_sequence_name(v_series, 'attempts')) INTO v_count;
    EXECUTE format('SELECT CASE WHEN is_called THEN last_value END FROM public.%I',
                   cairn_vault_sequence_name(v_series, 'clock')) INTO v_last;
    RETURN QUERY SELECT v_count - v_mark, v_last;
END;
$$;

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
-- promotion to superuser — verifies against a different address and is refused. Pure and IMMUTABLE.
-- There is deliberately no Rust copy: the window asks this function for the address (R7), and the
-- SQL mirror (db/tests/061) restates the formula to pin it. Deliberately NOT a definer:
-- it reads no table, so a definer would add privilege for nothing.
CREATE OR REPLACE FUNCTION cairn_vault_enrol_address(p_staff_id text, p_signing_key_id text, p_role text)
RETURNS bytea LANGUAGE sql IMMUTABLE PARALLEL SAFE AS $$
    SELECT '\x1220'::bytea || sha256(convert_to(
        'cairn-vault-enrol' || E'\n' || cairn_vault_staff_id(p_staff_id)
        || E'\n' || p_signing_key_id || E'\n' || p_role, 'UTF8'));
$$;

-- ============================================================================================
-- SIGN-IN SLICE 2's RULES (design 2026-11-01, §5.1–§5.5), which the release and enrol doors below
-- call. They are defined HERE, ahead of those doors, not in db/063 with the other slice-2 doors:
-- PL/pgSQL binds a called name only when it runs, so a door replayed here calling a function that
-- a LATER file defines would, on the upgrade replay, raise "function does not exist" at a
-- concurrent sign-in until that file committed — breaking promise 2. Here, the door and its rules
-- change in one transaction. They read only tables that exist by now (vault_member above,
-- actor_event and db/004's views).
-- ============================================================================================

-- The release door's role → purpose map (§5.1), as a pure function so the SQL mirror can pin it:
--   * 'reseal' (re-wrap my own key under a new passphrase, §5.4) — every role;
--   * superuser and admin — administrative acts ('enrol'), never 'clinical' (db/061 promise 3);
--   * everyone else — 'clinical', and 'enrol' only while an enroller (p_enroller).
-- Anything else, NULL included, is refused. Pure and IMMUTABLE.
CREATE OR REPLACE FUNCTION cairn_vault_purpose_allowed(p_role text, p_enroller boolean, p_purpose text)
RETURNS boolean LANGUAGE sql IMMUTABLE PARALLEL SAFE AS $$
    SELECT COALESCE(CASE
        WHEN p_purpose = 'reseal' THEN true
        WHEN p_role IN ('superuser', 'admin') THEN p_purpose = 'enrol'
        ELSE p_purpose = 'clinical' OR (p_purpose = 'enrol' AND p_enroller)
    END, false);
$$;

-- An actor's latest grant-enroller / withdraw-enroller row: its seq, and whether it is a grant.
-- No row = never granted. Latest is (recorded_at, seq), as actor_current orders. Invoker, owner only.
CREATE OR REPLACE FUNCTION cairn_vault_latest_grant(p_actor_id bytea)
RETURNS TABLE(seq bigint, granted boolean) LANGUAGE sql STABLE AS $$
    SELECT e.seq, e.op = 'grant-enroller' FROM actor_event e
     WHERE e.actor_id = p_actor_id AND e.op IN ('grant-enroller', 'withdraw-enroller')
     ORDER BY e.recorded_at DESC, e.seq DESC LIMIT 1;
$$;

-- THE enroller rule (§5.1), in words: NULL when the member may approve as an enroller, else the
-- reason. A member may when (in this order) they hold a current grant, their key is the actor's
-- current key, and that key is confirmed (or predates the mechanism, 'outside') — an enroller
-- re-keyed by another enroller cannot enrol until a superuser confirms them (Review Focus 3).
-- Both cairn_vault_is_enroller and cairn_vault_approver_ok ask this, so they cannot disagree.
CREATE OR REPLACE FUNCTION cairn_vault_enroller_problem(p_staff_id text)
RETURNS text LANGUAGE sql STABLE AS $$
    SELECT CASE
        WHEN m.staff_id IS NULL THEN 'is not an enrolled member'
        WHEN (SELECT g.granted FROM cairn_vault_latest_grant(m.actor_id) g) IS NOT TRUE
            THEN 'holds no current enroller grant'
        WHEN NOT EXISTS (SELECT 1 FROM actor_current a
                         WHERE a.actor_id = m.actor_id AND a.signing_key_id = m.signing_key_id)
            THEN 'has a key that is not current (revoked, superseded, rotated away or repudiated)'
        WHEN cairn_key_confirmation(m.signing_key_id) NOT IN ('confirmed', 'outside')
            THEN 'has a key that is ' || cairn_key_confirmation(m.signing_key_id)
                 || ' — an enroller act needs a confirmed key, so a superuser must confirm it first'
    END
    FROM (SELECT 1) one
    LEFT JOIN vault_member m ON m.staff_id = cairn_vault_staff_id(p_staff_id);
$$;

-- May this member approve an enrolment or re-key right now? The release door asks it for the
-- 'enrol' purpose; the window may ask it to offer "Enrol staff". Reveals one boolean per staff ID.
CREATE OR REPLACE FUNCTION cairn_vault_is_enroller(p_staff_id text)
RETURNS boolean LANGUAGE sql STABLE SECURITY DEFINER SET search_path = public, pg_temp AS $$
    SELECT cairn_vault_enroller_problem(p_staff_id) IS NULL;
$$;

-- THE APPROVER RULE — the one place it lives; every door that needs an approval calls this.
-- Returns 'superuser' or 'enroller', or RAISES naming the failed check:
--   * the approver must be a vault member;
--   * a superuser approves if their key is current (enrolled, not revoked/rotated/repudiated);
--   * anyone else approves only when p_may_enroller (enrol, re-key) AND the enroller rule holds;
--   * and p_token must be the approver's own valid attestation over p_address — so it covers
--     exactly this request.
-- The door then applies its own limits on what an enroller may do (never a superuser or admin
-- target). Invoker rights, revoked from PUBLIC: reached only from inside the definer doors.
-- Refusals never echo the token.
CREATE OR REPLACE FUNCTION cairn_vault_approver_ok(p_approver_staff_id text, p_address bytea,
                                                   p_token bytea, p_may_enroller boolean)
RETURNS text LANGUAGE plpgsql AS $$
DECLARE
    v_staff   text := cairn_vault_staff_id(p_approver_staff_id);
    v_role    text;
    v_key     text;
    v_kind    text;
    v_problem text;
BEGIN
    SELECT m.role, m.signing_key_id INTO v_role, v_key FROM vault_member m WHERE m.staff_id = v_staff;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'vault approval refused: % is not an enrolled superuser or enroller', v_staff;
    END IF;
    IF v_role = 'superuser' THEN
        IF NOT EXISTS (SELECT 1 FROM actor_current a WHERE a.signing_key_id = v_key) THEN
            RAISE EXCEPTION 'vault approval refused: superuser %''s key is not a current (enrolled, non-revoked) actor', v_staff;
        END IF;
        v_kind := 'superuser';
    ELSIF p_may_enroller IS NOT TRUE THEN
        RAISE EXCEPTION 'vault approval refused: % is not a superuser — only a superuser may do this', v_staff;
    ELSE
        v_problem := cairn_vault_enroller_problem(v_staff);
        IF v_problem IS NOT NULL THEN
            RAISE EXCEPTION 'vault approval refused: % is not a superuser and, as an enroller, %', v_staff, v_problem;
        END IF;
        v_kind := 'enroller';
    END IF;
    -- IS NOT TRUE: a NULL token or address yields NULL, which must refuse.
    IF cairn_attestation_ok(p_token, p_address, decode(v_key, 'hex')) IS NOT TRUE THEN
        RAISE EXCEPTION 'vault approval refused: the approver''s attestation is invalid or does not cover this request';
    END IF;
    RETURN v_kind;
END;
$$;

-- Who stands behind a key, as actor_event.approved_by records an approver: the member filed under
-- the actor that holds or held it — or, with no member row (after a restore), that actor's pinned
-- handle and no display name. 'console' answers for an owner-console act. Invoker, owner only.
CREATE OR REPLACE FUNCTION cairn_vault_key_holder(p_key text)
RETURNS TABLE(staff_id text, display_name text) LANGUAGE sql STABLE AS $$
    SELECT 'console', 'the node''s console' WHERE p_key = 'console'
    UNION ALL
    SELECT COALESCE(m.staff_id, e.pinned ->> 'handle'), m.display_name
      FROM (SELECT ae.actor_id, ae.pinned FROM actor_event ae
             WHERE ae.signing_key_id = p_key AND ae.op IN ('enroll', 'rotate-key') LIMIT 1) e
      LEFT JOIN vault_member m ON m.actor_id = e.actor_id;
$$;

-- The facts of the "your passphrase was replaced" sentence (§5.3): when the member's CURRENT key
-- was put in by a re-key, and by whom — a row only while the notice applies: the re-key is at
-- most 14 days old, or its key is still unconfirmed (or doubtful). The release door answers
-- 'replaced' instead of 'wrong' exactly when this has a row, so the rule has ONE home.
-- 14 days is the slice-2 plan's call: long enough to cover a fortnight's roster, short enough
-- that a confirmed re-key stops being announced. It is not a site policy (yet).
CREATE OR REPLACE FUNCTION cairn_vault_replaced_notice(p_staff_id text)
RETURNS TABLE(replaced_at timestamptz, approver_display_name text)
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = public, pg_temp AS $$
    SELECT r.recorded_at,
           (SELECT h.display_name FROM cairn_vault_key_holder(r.approved_by) h LIMIT 1)
      FROM vault_member m
      CROSS JOIN LATERAL (
          SELECT e.recorded_at, e.approved_by FROM actor_event e
           WHERE e.actor_id = m.actor_id AND e.op = 'rotate-key'
             AND e.signing_key_id = m.signing_key_id
           ORDER BY e.recorded_at DESC, e.seq DESC LIMIT 1) r
     WHERE m.staff_id = cairn_vault_staff_id(p_staff_id)
       AND (r.recorded_at > now() - interval '14 days'
            OR cairn_key_confirmation(m.signing_key_id) IN ('unconfirmed', 'doubtful'));
$$;

-- The release door: check a passphrase and, for the ONE purpose this member's role may serve,
-- hand back their seed. NEVER RAISES (see the header, promise 2): every outcome is a row.
--
--   outcome          seed / signing_key_id   retry_after_ms
--   'released'       set                     NULL
--   'wrong'          NULL                    NULL for failures 1–3, else the new wait
--   'replaced'       NULL                    as 'wrong' — and counted exactly like it — when the
--                                            member's current key came from a recent or still
--                                            unconfirmed re-key (above,
--                                            cairn_vault_replaced_notice): the real owner, typing
--                                            the passphrase they know, learns it was replaced
--                                            instead of burning tries into the back-off
--   'waiting'        NULL                    ms until the next try may be made
--   'unknown'        NULL                    NULL   (no such staff ID; nothing is counted)
--   'refused-purpose' NULL                   NULL   (the role may not serve that purpose —
--                                                    promise 3, cairn_vault_purpose_allowed — or a
--                                                    purpose outside the vocabulary; checked BEFORE
--                                                    any unsealing, so a key that will not be
--                                                    released is never even unwrapped)
--   'not-current'    NULL                    NULL   (the member's actor is revoked or superseded:
--                                                    its key signs nothing the floor accepts;
--                                                    checked before counting, like the purpose)
--
-- The order is the whole point (promise 4). Steps 2–4 run under the attempt row's FOR UPDATE lock
-- (taken at step 2), so two terminals guessing the same staff ID at once are serialised and cannot
-- both slip past one wait:
--   1. look up the member, check the purpose and that its actor is current — no writes, no lock;
--   2. take the lock; read failures and the last attempt's time from the sequences — no writes;
--      still inside the wait → 'waiting' (no unsealing, no counting);
--   3. CONSUME an attempt number and stamp the clock — non-transactional, so from here on the
--      guess is on record whatever the caller does — and only THEN unseal;
--   4. right → move success_mark up to this attempt → 'released'; wrong → 'wrong' (or
--      'replaced'), with the new wait if this failure starts or extends one.
--
-- Columns are table-qualified throughout: the OUT column `signing_key_id` would otherwise be
-- ambiguous with vault_member's column of the same name.
CREATE OR REPLACE FUNCTION cairn_vault_release(p_staff_id text, p_passphrase text, p_purpose text)
RETURNS TABLE(outcome text, seed bytea, signing_key_id text, retry_after_ms bigint)
LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, pg_temp AS $$
DECLARE
    v_staff_id text := cairn_vault_staff_id(p_staff_id);
    v_member   vault_member%ROWTYPE;
    v_failures bigint;
    v_last_ms  bigint;
    v_now_ms   bigint;
    v_backoff  int;
    v_attempt  bigint;
    v_mark     bigint;
    v_seed     bytea;
BEGIN
    -- 1. Who, and for what.
    SELECT * INTO v_member FROM vault_member m WHERE m.staff_id = v_staff_id;
    IF NOT FOUND THEN
        RETURN QUERY SELECT 'unknown'::text, NULL::bytea, NULL::text, NULL::bigint;
        RETURN;
    END IF;

    -- §5.1 (promise 3): which purposes this member's role serves — the purpose map above. The
    -- enroller test is asked only for 'enrol'.
    IF NOT cairn_vault_purpose_allowed(v_member.role,
                                       p_purpose = 'enrol' AND cairn_vault_is_enroller(v_staff_id),
                                       p_purpose) THEN
        RETURN QUERY SELECT 'refused-purpose'::text, NULL::bytea, NULL::text, NULL::bigint;
        RETURN;
    END IF;
    -- A key that is not current (its actor revoked or superseded, the key rotated away or
    -- repudiated — ADMISSION, so actor_current, never actor_key_vouches) signs nothing the
    -- floor will accept, so it is not released: the member is told why, instead of seeing
    -- every act fail.
    -- Before the count, like the purpose: it reveals nothing a guess would.
    IF NOT EXISTS (SELECT 1 FROM actor_current a WHERE a.signing_key_id = v_member.signing_key_id) THEN
        RETURN QUERY SELECT 'not-current'::text, NULL::bytea, NULL::text, NULL::bigint;
        RETURN;
    END IF;

    -- 2. Backed off? Read-only: a 'waiting' answer writes nothing and counts nothing.
    --    The attempt row is both the lock and the success mark. A member WITHOUT one (only a hand
    --    DELETE can do that; the trigger always creates it) could not be backed off, so the door
    --    fails CLOSED: 'unknown', nothing unsealed, nothing written.
    SELECT a.success_mark INTO v_mark FROM vault_attempt a WHERE a.staff_id = v_staff_id FOR UPDATE;
    IF NOT FOUND THEN
        RETURN QUERY SELECT 'unknown'::text, NULL::bytea, NULL::text, NULL::bigint;
        RETURN;
    END IF;
    SELECT s.failures, s.last_attempt_ms INTO v_failures, v_last_ms
      FROM cairn_vault_attempt_state(v_staff_id) s;
    v_now_ms  := (extract(epoch FROM clock_timestamp()) * 1000)::bigint;
    v_backoff := cairn_vault_backoff_seconds(v_failures::int);      -- NULL for failures 0–3
    -- A last attempt stamped LATER than now means the wall clock was stepped back since (an NTP
    -- correction, a dead clock battery after a power cut). How long ago that attempt really was
    -- is then unknown, and an unknown wait must not become a lock-out: waiting "until the stamp
    -- plus the back-off" could mean hours or years. So the guess is checked as if the wait were
    -- over. It costs little: step 3 restamps the clock at now, so the back-off resumes from this
    -- attempt at once, and an attacker gains at most one extra guess per backward clock step.
    IF v_last_ms <= v_now_ms AND v_now_ms < v_last_ms + v_backoff * 1000 THEN
        RETURN QUERY SELECT 'waiting'::text, NULL::bytea, NULL::text,
            v_last_ms + v_backoff * 1000 - v_now_ms;
        RETURN;
    END IF;

    -- 3. Count the guess where no rollback can reach it, THEN check it. setval, not nextval: see
    --    cairn_vault_attempt_state on crashes. Exact under the row lock: attempts = mark + failures.
    v_attempt := v_mark + v_failures + 1;
    PERFORM setval(format('public.%I',
        cairn_vault_sequence_name(v_member.attempt_series_id, 'attempts'))::regclass, v_attempt, true);
    PERFORM setval(format('public.%I',
        cairn_vault_sequence_name(v_member.attempt_series_id, 'clock'))::regclass, v_now_ms);
    v_seed := cairn_vault_unseal(v_member.sealed, p_passphrase);   -- NULL = wrong; never raises

    -- 4. Answer. A wrong passphrase on a recently re-keyed member says so ('replaced', above);
    --    the guess was counted at step 3 either way.
    IF v_seed IS NULL THEN
        v_backoff := cairn_vault_backoff_seconds((v_failures + 1)::int);
        RETURN QUERY SELECT
            CASE WHEN EXISTS (SELECT 1 FROM cairn_vault_replaced_notice(v_staff_id))
                 THEN 'replaced' ELSE 'wrong' END,
            NULL::bytea, NULL::text, v_backoff::bigint * 1000;
        RETURN;
    END IF;
    UPDATE vault_attempt a SET success_mark = v_attempt WHERE a.staff_id = v_staff_id;
    RETURN QUERY SELECT 'released'::text, v_seed, v_member.signing_key_id, NULL::bigint;
END;
$$;

-- The member's own passphrase hint (shown on request at the sign-in strip). NULL if none, or no
-- such staff ID.
CREATE OR REPLACE FUNCTION cairn_vault_hint(p_staff_id text)
RETURNS text LANGUAGE sql STABLE SECURITY DEFINER SET search_path = public, pg_temp AS $$
    SELECT m.hint FROM vault_member m WHERE m.staff_id = cairn_vault_staff_id(p_staff_id);
$$;

-- Is this staff ID already enrolled? The window asks BEFORE it releases a superuser's key, so a
-- duplicate gets its own plain sentence instead of the floor's refusal (a floor P0001 carries no
-- scope, and matching its message text is forbidden: reuse the predicate). It reveals only
-- whether an ID is enrolled, which cairn_vault_release already answers ('unknown' vs 'wrong'), as
-- does the hint for any member who set one: the roster is not secret.
CREATE OR REPLACE FUNCTION cairn_vault_staff_id_taken(p_staff_id text)
RETURNS boolean LANGUAGE sql STABLE SECURITY DEFINER SET search_path = public, pg_temp AS $$
    SELECT EXISTS (SELECT 1 FROM vault_member m WHERE m.staff_id = cairn_vault_staff_id(p_staff_id));
$$;

-- The site's terminal-unlock PHC string, for the window to check the shared staff passphrase
-- against. NULL until the console has set one.
CREATE OR REPLACE FUNCTION cairn_vault_terminal_phc()
RETURNS text LANGUAGE sql STABLE SECURITY DEFINER SET search_path = public, pg_temp AS $$
    SELECT t.unlock_phc FROM vault_terminal t;
$$;

-- Owner only (the node's console): set or replace the terminal-unlock PHC string.
-- Refuses anything that is not an Argon2id PHC string, so a console mistake can never store the
-- shared passphrase itself in clear (the refusal does not echo the value).
CREATE OR REPLACE FUNCTION cairn_vault_set_terminal_phc(p_phc text)
RETURNS void LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, pg_temp AS $$
BEGIN
    IF starts_with(p_phc, '$argon2id$') IS NOT TRUE THEN
        RAISE EXCEPTION 'cairn_vault_set_terminal_phc: the value is not an Argon2id PHC string (it must start $argon2id$) — store the hash, never the passphrase';
    END IF;
    INSERT INTO vault_terminal (singleton, unlock_phc) VALUES (true, p_phc)
    ON CONFLICT (singleton) DO UPDATE SET unlock_phc = EXCLUDED.unlock_phc;
END;
$$;

-- The shared insert behind both enrolment doors. Its one job beyond the INSERT: a violated
-- constraint is re-raised with the constraint's NAME only. Postgres's own CHECK / NOT NULL error
-- carries a DETAIL "Failing row contains (…)" that would print every column — the sealed blob
-- included — into the client's error and the server log. Catching it and raising SQLERRM alone
-- (which names the constraint, never the values) keeps the blob out of the error's DETAIL.
-- Bind-parameter logging, which would log the blob as an argument, is the operator's to keep off
-- (ADR-0083 decision 5). Invoker-rights and revoked
-- from PUBLIC: it is reached only from inside the two definer doors below.
CREATE OR REPLACE FUNCTION cairn_vault_insert_member(
    p_staff_id text, p_display_name text, p_role text, p_actor_id bytea, p_signing_key_id text,
    p_sealed bytea, p_hint text, p_phone text, p_email text, p_enrolled_by text,
    p_enrol_token bytea)
RETURNS void LANGUAGE plpgsql AS $$
BEGIN
    INSERT INTO vault_member (staff_id, display_name, role, actor_id, signing_key_id, sealed,
                              hint, contact_phone, contact_email, enrolled_by, enrol_token)
    VALUES (cairn_vault_staff_id(p_staff_id), p_display_name, p_role, p_actor_id,
            p_signing_key_id, p_sealed, p_hint, p_phone, p_email, p_enrolled_by, p_enrol_token);
EXCEPTION WHEN check_violation OR not_null_violation OR unique_violation THEN
    RAISE EXCEPTION 'vault enrolment refused: %', SQLERRM;
END;
$$;

-- Enrol a member, approved by a superuser or (since sign-in slice 2) an enroller.
-- p_superuser_staff_id now means "the APPROVER's staff ID": the name stays because Postgres refuses
-- a renamed parameter under CREATE OR REPLACE, and the argument list is fixed (db/005's FIXED
-- ARITY note). Refuses unless ALL of:
--   (a)–(c) cairn_vault_approver_ok (above, the one home of the approver rule): the approver is
--       a superuser with a current key, or an enroller (current grant, current AND confirmed
--       key), and p_token is their valid attestation over cairn_vault_enrol_address of THIS
--       request — so a token cannot be replayed for another person, key or role;
--   (a2) an enroller enrols neither a 'superuser' nor an 'admin' (design §5.1);
--   (d) the pinned determinant set is EXACTLY the one the floor builds for this member,
--       `{role: p_role, handle: cairn_vault_staff_id(p_staff_id)}`. The registry and the vault
--       must agree on what this actor is, and the handle must be the NORMALISED staff ID: built
--       from the ID as typed, "Boss" and "boss" would compute two actor_ids for one person. A
--       caller-built set with another handle or an extra determinant is refused, so a raw-SQL
--       client cannot file a vault member under an actor whose determinants say something else.
-- Then the actor is enrolled in the append-only registry, carrying the approver's key and token
-- (so a superuser-approved key is confirmed at once, by a `confirm` row in the same transaction,
-- and an enroller-approved one stays unconfirmed — db/004's cairn_key_confirmation), and the vault
-- row inserted, in one transaction: either all exist or none does. Refusals name the failed check
-- and never echo p_sealed or p_token.
CREATE OR REPLACE FUNCTION cairn_vault_enrol(
    p_staff_id text, p_display_name text, p_role text, p_pinned jsonb, p_signing_key_id text,
    p_sealed bytea, p_hint text, p_phone text, p_email text, p_superuser_staff_id text,
    p_token bytea)
RETURNS bytea LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, pg_temp AS $$
DECLARE
    v_approver  text := cairn_vault_staff_id(p_superuser_staff_id);
    v_kind      text;
    v_key       text;
    v_actor_id  bytea;
BEGIN
    v_kind := cairn_vault_approver_ok(v_approver,
                                      cairn_vault_enrol_address(p_staff_id, p_signing_key_id, p_role),
                                      p_token, true);
    IF v_kind = 'enroller' AND p_role IN ('superuser', 'admin') THEN
        RAISE EXCEPTION 'cairn_vault_enrol: an enroller may not enrol a superuser or an admin — ask a superuser';
    END IF;
    -- jsonb equality ignores key order and whitespace, so this compares the SET, not its text.
    IF p_pinned IS DISTINCT FROM
       jsonb_build_object('role', p_role, 'handle', cairn_vault_staff_id(p_staff_id)) THEN
        RAISE EXCEPTION 'cairn_vault_enrol: the pinned set must be exactly {role, handle} with the role % and the normalised staff ID % as the handle',
            p_role, cairn_vault_staff_id(p_staff_id);
    END IF;

    SELECT m.signing_key_id INTO v_key FROM vault_member m WHERE m.staff_id = v_approver;
    v_actor_id := enroll_actor('human', p_pinned, p_signing_key_id, v_key, p_token);
    IF v_kind = 'superuser' THEN
        PERFORM cairn_actor_append(v_actor_id, 'confirm', p_signing_key_id, v_key, p_token);
    END IF;
    PERFORM cairn_vault_insert_member(p_staff_id, p_display_name, p_role, v_actor_id,
                                      p_signing_key_id, p_sealed, p_hint, p_phone, p_email,
                                      v_approver, p_token);
    RETURN v_actor_id;
END;
$$;

-- Owner only (the node's console): enrol a superuser with no superuser to approve it — the first
-- one on a new site, or the recovery of a locked-out last one. The role is forced to 'superuser'
-- and `enrolled_by` to 'console'; the pinned set must be exactly the one the floor builds,
-- `{role: 'superuser', handle: cairn_vault_staff_id(p_staff_id)}` (see cairn_vault_enrol's (d)
-- for why the handle is the normalised staff ID). The console is the approver: the registry row
-- says approved_by 'console' (no token) and a `confirm` row follows, so the key is confirmed.
-- No grant: being able to run this
-- IS the authority (whoever holds the database owner's credentials already holds the site).
CREATE OR REPLACE FUNCTION cairn_vault_bootstrap(
    p_staff_id text, p_display_name text, p_pinned jsonb, p_signing_key_id text, p_sealed bytea,
    p_hint text, p_phone text, p_email text)
RETURNS bytea LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, pg_temp AS $$
DECLARE v_actor_id bytea;
BEGIN
    IF p_pinned IS DISTINCT FROM
       jsonb_build_object('role', 'superuser', 'handle', cairn_vault_staff_id(p_staff_id)) THEN
        RAISE EXCEPTION 'cairn_vault_bootstrap: the pinned set must be exactly {role, handle} with the role superuser and the normalised staff ID % as the handle',
            cairn_vault_staff_id(p_staff_id);
    END IF;
    v_actor_id := enroll_actor('human', p_pinned, p_signing_key_id, 'console', NULL);
    PERFORM cairn_actor_append(v_actor_id, 'confirm', p_signing_key_id, 'console', NULL);
    PERFORM cairn_vault_insert_member(p_staff_id, p_display_name, 'superuser', v_actor_id,
                                      p_signing_key_id, p_sealed, p_hint, p_phone, p_email,
                                      'console', NULL);
    RETURN v_actor_id;
END;
$$;

-- Grants. Functions default to PUBLIC EXECUTE, so every door is first revoked from PUBLIC and then
-- granted to exactly the role the header names.
REVOKE EXECUTE ON FUNCTION cairn_vault_release(text, text, text) FROM PUBLIC;
REVOKE EXECUTE ON FUNCTION cairn_vault_hint(text) FROM PUBLIC;
REVOKE EXECUTE ON FUNCTION cairn_vault_staff_id_taken(text) FROM PUBLIC;
REVOKE EXECUTE ON FUNCTION cairn_vault_terminal_phc() FROM PUBLIC;
REVOKE EXECUTE ON FUNCTION cairn_vault_enrol(text, text, text, jsonb, text, bytea, text, text, text, text, bytea) FROM PUBLIC;
REVOKE EXECUTE ON FUNCTION cairn_vault_bootstrap(text, text, jsonb, text, bytea, text, text, text) FROM PUBLIC;
REVOKE EXECUTE ON FUNCTION cairn_vault_set_terminal_phc(text) FROM PUBLIC;
REVOKE EXECUTE ON FUNCTION cairn_vault_insert_member(text, text, text, bytea, text, bytea, text, text, text, text, bytea) FROM PUBLIC;
REVOKE EXECUTE ON FUNCTION cairn_vault_attempt_state(text) FROM PUBLIC;
REVOKE EXECUTE ON FUNCTION cairn_vault_member_created() FROM PUBLIC;
-- Slice 2's rules above: internal helpers stay the owner's; the window may ask two of them.
REVOKE EXECUTE ON FUNCTION cairn_vault_latest_grant(bytea) FROM PUBLIC;
REVOKE EXECUTE ON FUNCTION cairn_vault_enroller_problem(text) FROM PUBLIC;
REVOKE EXECUTE ON FUNCTION cairn_vault_is_enroller(text) FROM PUBLIC;
REVOKE EXECUTE ON FUNCTION cairn_vault_approver_ok(text, bytea, bytea, boolean) FROM PUBLIC;
REVOKE EXECUTE ON FUNCTION cairn_vault_key_holder(text) FROM PUBLIC;
REVOKE EXECUTE ON FUNCTION cairn_vault_replaced_notice(text) FROM PUBLIC;
-- The unwrapping primitive: unthrottled (one Argon2id per call), so reachable only through the
-- release door, which counts and backs off.
REVOKE EXECUTE ON FUNCTION cairn_vault_unseal(bytea, text) FROM PUBLIC;
-- That REVOKE is only a WARNING, and a no-op, when the loader does not own the extension's
-- functions (cairn_pgx is not superuser-only, so whoever ran CREATE EXTENSION owns them). Then the
-- runtime role could call the unthrottled primitive directly. It would still need a sealed blob,
-- which it cannot read, so this is defence in depth, not the only wall. Refuse the load instead.
-- It runs only when the schema is replayed: a cairn_pgx reinstall, which re-grants PUBLIC, skips
-- it (#908).
DO $$
BEGIN
    IF has_function_privilege('cairn_agent', 'cairn_vault_unseal(bytea, text)', 'EXECUTE') THEN
        RAISE EXCEPTION 'db/061: cairn_agent can still execute cairn_vault_unseal, so the vault''s back-off could be skipped'
            USING HINT = 'Load the schema as the role that ran CREATE EXTENSION cairn_pgx (it owns '
                         || 'cairn_vault_unseal), or REVOKE EXECUTE on it FROM PUBLIC as that role.';
    END IF;
END;
$$;

GRANT EXECUTE ON FUNCTION cairn_vault_release(text, text, text) TO cairn_agent;
GRANT EXECUTE ON FUNCTION cairn_vault_hint(text) TO cairn_agent;
GRANT EXECUTE ON FUNCTION cairn_vault_staff_id_taken(text) TO cairn_agent;
GRANT EXECUTE ON FUNCTION cairn_vault_terminal_phc() TO cairn_agent;
GRANT EXECUTE ON FUNCTION cairn_vault_enrol(text, text, text, jsonb, text, bytea, text, text, text, text, bytea) TO cairn_agent;
GRANT EXECUTE ON FUNCTION cairn_vault_is_enroller(text) TO cairn_agent;
GRANT EXECUTE ON FUNCTION cairn_vault_replaced_notice(text) TO cairn_agent;

COMMIT;
