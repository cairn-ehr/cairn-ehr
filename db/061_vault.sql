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
-- row. The APPROVAL of an enrolment — the superuser's signed attestation token — is kept on the
-- member row (`vault_member.enrol_token`; NULL for a console bootstrap), so who approved whom can
-- be re-verified for as long as that row stands. It lives only there: it is custody state, not an
-- append-only event, and an owner who deletes the row deletes the approval with it.
--
-- THE FOUR PROMISES THIS FILE KEEPS (and how):
--
--   1. The wrapped key never leaves the database. `vault_member` has NO grant to anyone (not
--      PUBLIC, not cairn_agent, not cairn_node), and no function, view or door returns the
--      `sealed` column. Even the door that checks a passphrase (`cairn_vault_release`) returns
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
--      delays THAT ONE staff member's next sign-in until it lets go. That is a denial of service
--      by someone who already holds the runtime role, bounded in practice by the window's 15 s
--      cut on a sign; it is never a bypass.
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
--   * cairn_agent (the window's runtime role): cairn_vault_release, cairn_vault_hint, cairn_vault_staff_id_taken,
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
    -- The approving superuser's attestation token over cairn_vault_enrol_address of this row's
    -- request (NULL for a console bootstrap, which no superuser approves). Kept so the approval
    -- can be re-verified later; see the header on what that does and does not make auditable.
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
-- The order is the whole point (promise 4). Under the attempt row's FOR UPDATE lock, so two
-- terminals guessing the same staff ID at once are serialised and cannot both slip past one wait:
--   1. look up the member and check the purpose — no writes;
--   2. read failures and the last attempt's time from the sequences — no writes; still inside the
--      wait → 'waiting' (no unsealing, no counting);
--   3. CONSUME an attempt number and stamp the clock — non-transactional, so from here on the
--      guess is on record whatever the caller does — and only THEN unseal;
--   4. right → move success_mark up to this attempt → 'released'; wrong → 'wrong', with the new
--      wait if this failure starts or extends one.
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

    -- §5.1: a superuser key serves enrolment only; every other key serves clinical acts only.
    IF p_purpose IS DISTINCT FROM
       (CASE WHEN v_member.role = 'superuser' THEN 'enrol' ELSE 'clinical' END) THEN
        RETURN QUERY SELECT 'refused-purpose'::text, NULL::bytea, NULL::text, NULL::bigint;
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

    -- 4. Answer.
    IF v_seed IS NULL THEN
        v_backoff := cairn_vault_backoff_seconds((v_failures + 1)::int);
        RETURN QUERY SELECT 'wrong'::text, NULL::bytea, NULL::text, v_backoff::bigint * 1000;
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
-- whether an ID is enrolled, which cairn_vault_hint already answers: the roster is not secret.
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
-- (which names the constraint, never the values) keeps the blob out. Invoker-rights and revoked
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

-- Enrol a member, approved by a superuser. Refuses unless ALL of:
--   (a) p_superuser_staff_id names a member whose role is 'superuser';
--   (b) that superuser's key is a current (enrolled, non-revoked) actor;
--   (c) p_token is that superuser's valid attestation over cairn_vault_enrol_address of THIS
--       request — so a token cannot be replayed for another person, key or role;
--   (d) the pinned determinant set is EXACTLY the one the floor builds for this member,
--       `{role: p_role, handle: cairn_vault_staff_id(p_staff_id)}`. The registry and the vault
--       must agree on what this actor is, and the handle must be the NORMALISED staff ID: built
--       from the ID as typed, "Boss" and "boss" would compute two actor_ids for one person. A
--       caller-built set with another handle or an extra determinant is refused, so a raw-SQL
--       client cannot file a vault member under an actor whose determinants say something else.
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
    -- jsonb equality ignores key order and whitespace, so this compares the SET, not its text.
    IF p_pinned IS DISTINCT FROM
       jsonb_build_object('role', p_role, 'handle', cairn_vault_staff_id(p_staff_id)) THEN
        RAISE EXCEPTION 'cairn_vault_enrol: the pinned set must be exactly {role, handle} with the role % and the normalised staff ID % as the handle',
            p_role, cairn_vault_staff_id(p_staff_id);
    END IF;

    v_actor_id := enroll_actor('human', p_pinned, p_signing_key_id);
    PERFORM cairn_vault_insert_member(p_staff_id, p_display_name, p_role, v_actor_id,
                                      p_signing_key_id, p_sealed, p_hint, p_phone, p_email,
                                      v_superuser, p_token);
    RETURN v_actor_id;
END;
$$;

-- Owner only (the node's console): enrol a superuser with no superuser to approve it — the first
-- one on a new site, or the recovery of a locked-out last one. The role is forced to 'superuser'
-- and `enrolled_by` to 'console'; the pinned set must be exactly the one the floor builds,
-- `{role: 'superuser', handle: cairn_vault_staff_id(p_staff_id)}` (see cairn_vault_enrol's (d)
-- for why the handle is the normalised staff ID). No grant: being able to run this
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
    v_actor_id := enroll_actor('human', p_pinned, p_signing_key_id);
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
-- The unwrapping primitive: unthrottled (one Argon2id per call), so reachable only through the
-- release door, which counts and backs off.
REVOKE EXECUTE ON FUNCTION cairn_vault_unseal(bytea, text) FROM PUBLIC;

GRANT EXECUTE ON FUNCTION cairn_vault_release(text, text, text) TO cairn_agent;
GRANT EXECUTE ON FUNCTION cairn_vault_hint(text) TO cairn_agent;
GRANT EXECUTE ON FUNCTION cairn_vault_staff_id_taken(text) TO cairn_agent;
GRANT EXECUTE ON FUNCTION cairn_vault_terminal_phc() TO cairn_agent;
GRANT EXECUTE ON FUNCTION cairn_vault_enrol(text, text, text, jsonb, text, bytea, text, text, text, text, bytea) TO cairn_agent;

COMMIT;
