-- db/084 — the terminal trust set, the gateway record, and the registration turn (#909 slice B).
--
-- WHY: slice B puts a pinned mTLS gateway in front of the node's Postgres so a window on ANOTHER
-- machine can reach it. The gateway admits only terminals whose key an operator has approved, and
-- this file is the database floor under that:
--
--   * terminal_trust — one row per terminal key the gateway has ever seen: `pending` (seen, not
--     yet approved), `active` (an operator admitted it) or `revoked` (final: a revoked terminal
--     comes back only with a NEW key). The gateway's login (cairn_terminal_service) may only
--     READ it and report an attempt (cairn_terminal_attempt, which can add or touch a row and
--     never change a decided one). Admit and revoke are the OWNER's: nobody who can reach the
--     network can approve itself. Unapproved attempts are capped at the 64 newest, so a hostile
--     LAN cannot grow the table without bound.
--   * terminal_gateway — the one row naming where the gateway forwards to (written by the owner).
--   * registration_turn — the one row registrations take turns on. It REPLACES slice A's
--     advisory lock (pg_advisory_xact_lock(0x4341_524E_5247)): an advisory lock is takeable by
--     any login, the window's included, so a window could hold it and starve registration. A
--     row lock is takeable only through cairn_registration_take_turn, whose EXECUTE the window
--     does not hold.
--
-- Replay-safe: CREATE TABLE IF NOT EXISTS, CREATE OR REPLACE FUNCTION, idempotent grants.

-- ---------------------------------------------------------------------------
-- The terminal trust set.
-- ---------------------------------------------------------------------------

CREATE TABLE IF NOT EXISTS terminal_trust (
    terminal_pubkey text PRIMARY KEY CHECK (terminal_pubkey ~ '^[0-9a-f]{64}$'),
    fingerprint     text NOT NULL,
    label           text,
    status          text NOT NULL CHECK (status IN ('pending', 'active', 'revoked')),
    first_seen      timestamptz NOT NULL,
    last_seen       timestamptz NOT NULL,
    decided_by      text,
    decided_at      timestamptz
);

REVOKE ALL ON terminal_trust FROM PUBLIC;
GRANT SELECT ON terminal_trust TO cairn_terminal_service;

-- The short fingerprint an operator reads aloud to approve a terminal: the first 10 bytes of the
-- sha256 of the 32 key bytes, as five upper-case 4-hex groups joined by '-'. Must equal
-- cairn_event::short_fingerprint (a test pins it).
CREATE OR REPLACE FUNCTION cairn_terminal_fingerprint(p_pubkey text)
RETURNS text LANGUAGE sql IMMUTABLE SECURITY DEFINER SET search_path = public, pg_temp AS $$
    SELECT concat_ws('-', substr(h, 1, 4), substr(h, 5, 4), substr(h, 9, 4), substr(h, 13, 4),
                     substr(h, 17, 4))
      FROM (SELECT upper(encode(substring(sha256(decode(p_pubkey, 'hex')) FROM 1 FOR 10), 'hex'))
                   AS h) x;
$$;

-- The gateway reports that key p_pubkey just connected. An unknown key is recorded `pending`; a
-- known key only has last_seen touched, and its status is NEVER changed here (a revoked terminal
-- cannot be re-pended by knocking). Then every pending row beyond the 64 newest goes.
CREATE OR REPLACE FUNCTION cairn_terminal_attempt(p_pubkey text)
RETURNS void LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, pg_temp AS $$
BEGIN
    INSERT INTO terminal_trust (terminal_pubkey, fingerprint, status, first_seen, last_seen)
    VALUES (p_pubkey, cairn_terminal_fingerprint(p_pubkey), 'pending',
            clock_timestamp(), clock_timestamp())
    ON CONFLICT (terminal_pubkey) DO UPDATE SET last_seen = clock_timestamp();

    DELETE FROM terminal_trust
     WHERE status = 'pending'
       AND terminal_pubkey IN (SELECT t.terminal_pubkey FROM terminal_trust t
                                WHERE t.status = 'pending'
                                ORDER BY t.last_seen DESC, t.terminal_pubkey
                               OFFSET 64);
END;
$$;

-- The one row with this fingerprint, locked; raises unless exactly one matches. Shared by admit
-- and revoke so both refuse an unknown or ambiguous fingerprint identically.
CREATE OR REPLACE FUNCTION cairn_terminal_lock_one(p_fingerprint text)
RETURNS terminal_trust LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, pg_temp AS $$
DECLARE
    v_rows terminal_trust[];
BEGIN
    SELECT array_agg(t) INTO v_rows FROM (
        SELECT * FROM terminal_trust WHERE fingerprint = upper(btrim(p_fingerprint)) FOR UPDATE
    ) t;
    IF v_rows IS NULL THEN
        RAISE EXCEPTION 'no terminal has the fingerprint %', p_fingerprint;
    ELSIF cardinality(v_rows) > 1 THEN
        RAISE EXCEPTION 'more than one terminal has the fingerprint %; refusing to guess', p_fingerprint;
    END IF;
    RETURN v_rows[1];
END;
$$;

-- OWNER ONLY. Admits the one pending terminal with this fingerprint under a label, and returns
-- its key. Raises on an empty label, no match, an ambiguous match, or a match that is not
-- pending (an active one needs nothing; a revoked one is final).
CREATE OR REPLACE FUNCTION cairn_terminal_admit(p_fingerprint text, p_label text)
RETURNS text LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, pg_temp AS $$
DECLARE
    v_row terminal_trust;
BEGIN
    IF btrim(coalesce(p_label, '')) = '' THEN
        RAISE EXCEPTION 'a terminal needs a label to be admitted';
    END IF;
    v_row := cairn_terminal_lock_one(p_fingerprint);
    IF v_row.status <> 'pending' THEN
        RAISE EXCEPTION 'terminal % is % and cannot be admitted (only a pending one can)',
            v_row.fingerprint, v_row.status;
    END IF;
    UPDATE terminal_trust
       SET status = 'active', label = p_label, decided_by = session_user, decided_at = now()
     WHERE terminal_pubkey = v_row.terminal_pubkey;
    RETURN v_row.terminal_pubkey;
END;
$$;

-- OWNER ONLY. Revokes the one pending or active terminal with this fingerprint (final), and
-- returns its key. Raises on no match or an ambiguous match; revoking a revoked one is a no-op.
CREATE OR REPLACE FUNCTION cairn_terminal_revoke(p_fingerprint text)
RETURNS text LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, pg_temp AS $$
DECLARE
    v_row terminal_trust;
BEGIN
    v_row := cairn_terminal_lock_one(p_fingerprint);
    IF v_row.status <> 'revoked' THEN
        UPDATE terminal_trust
           SET status = 'revoked', decided_by = session_user, decided_at = now()
         WHERE terminal_pubkey = v_row.terminal_pubkey;
    END IF;
    RETURN v_row.terminal_pubkey;
END;
$$;

-- ---------------------------------------------------------------------------
-- The gateway record: where the pinned gateway forwards to, and who last checked it.
-- ---------------------------------------------------------------------------

CREATE TABLE IF NOT EXISTS terminal_gateway (
    only_row   boolean PRIMARY KEY DEFAULT true CHECK (only_row),
    upstream   text NOT NULL,
    checked_at timestamptz NOT NULL,
    checked_by text NOT NULL
);

REVOKE ALL ON terminal_gateway FROM PUBLIC;
GRANT SELECT ON terminal_gateway TO cairn_terminal_service;

-- ---------------------------------------------------------------------------
-- The registration turn (replaces slice A's advisory lock).
-- ---------------------------------------------------------------------------

CREATE TABLE IF NOT EXISTS registration_turn (
    only_row boolean PRIMARY KEY DEFAULT true CHECK (only_row)
);

REVOKE ALL ON registration_turn FROM PUBLIC;
INSERT INTO registration_turn DEFAULT VALUES ON CONFLICT DO NOTHING;

-- Take the registration turn: lock the one row until the caller's transaction ends, so
-- registrations (the service's and the CLI's) run one at a time node-wide; the second one's
-- search then sees the first's chart. With p_if_pending, return false and lock NOTHING when no
-- request cairn_registration_claim would return exists: a row lock writes WAL, and the service
-- polls every 250 ms, so an unconditional turn would be ~4 WAL writes a second on an idle node.
-- A request arriving just after a false waits one poll; the claim, under the lock, is the
-- authoritative re-check.
CREATE OR REPLACE FUNCTION cairn_registration_take_turn(p_if_pending boolean DEFAULT false)
RETURNS boolean LANGUAGE plpgsql SECURITY DEFINER SET search_path = public, pg_temp AS $$
BEGIN
    IF p_if_pending AND NOT EXISTS (
        SELECT 1 FROM registration_request q
         WHERE q.status = 'pending' AND q.deadline > clock_timestamp()) THEN
        RETURN false;
    END IF;
    PERFORM 1 FROM registration_turn FOR UPDATE;
    RETURN true;
END;
$$;

REVOKE ALL ON FUNCTION cairn_terminal_fingerprint(text), cairn_terminal_attempt(text),
    cairn_terminal_lock_one(text), cairn_terminal_admit(text, text), cairn_terminal_revoke(text),
    cairn_registration_take_turn(boolean) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION cairn_terminal_attempt(text), cairn_registration_take_turn(boolean)
    TO cairn_terminal_service;
