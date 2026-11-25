-- db/086 — the node's release store and the window's three read-only doors (#909 slice C2).
--
-- WHY: a terminal fetches its steward-signed update THROUGH its node. The node's intake
-- (`release-intake`) verifies and stores what it takes in; the terminal's window reads it back
-- through three doors and hands the bytes to its installer, which verifies against the
-- terminal's OWN pin. So nothing here is trusted by the terminal: the doors serve bytes, they
-- never judge them.
--
--   * release_root     — the trust-root chain the node holds, one row per version.
--   * release_bundle   — one signed release (manifest + signatures + inclusion proof).
--   * release_artifact — the files a bundle names, content-addressed by digest.
--   * release_incident — a fork the node saw; while one is unresolved the offer door is silent.
--
-- These are NODE-LOCAL operational state: never a clinical event, never synced. The tables are
-- owner-only (REVOKE FROM PUBLIC, nothing granted to cairn_window); the doors are the only way
-- the window reaches them. All three are read-only SECURITY DEFINER functions.
--
-- Replay-safe: CREATE TABLE IF NOT EXISTS, CREATE OR REPLACE FUNCTION, idempotent grants.

BEGIN;

CREATE TABLE IF NOT EXISTS release_root (
    version     bigint PRIMARY KEY,
    channel_id  text   NOT NULL,
    payload     bytea  NOT NULL,
    signatures  bytea[] NOT NULL,
    root_hash   bytea  NOT NULL
);

CREATE TABLE IF NOT EXISTS release_bundle (
    release_version    bigint PRIMARY KEY,
    channel_id         text   NOT NULL,
    manifest           bytea  NOT NULL,
    release_signatures bytea[] NOT NULL,
    inclusion_proof    bytea,
    taken_in_at        timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS release_artifact (
    digest          bytea  NOT NULL,
    release_version bigint NOT NULL REFERENCES release_bundle (release_version),
    idx             int    NOT NULL,
    kind            text   NOT NULL,
    os              text,
    arch            text,
    platform        text,
    generation_line text,
    byte_len        bigint NOT NULL,
    content         bytea  NOT NULL,
    -- Keyed by the manifest entry, not the bytes: identical bytes may legitimately appear twice
    -- (e.g. one program for two platforms), each entry with its own os/arch/platform (R15).
    PRIMARY KEY (release_version, idx)
);
CREATE INDEX IF NOT EXISTS release_artifact_digest ON release_artifact (digest);

CREATE TABLE IF NOT EXISTS release_incident (
    id          bigserial PRIMARY KEY,
    channel_id  text NOT NULL,
    version     bigint,
    detected_at timestamptz NOT NULL DEFAULT now(),
    detail      text NOT NULL,
    resolved_at timestamptz
);

REVOKE ALL ON release_root, release_bundle, release_artifact, release_incident FROM PUBLIC;

-- What this terminal may be offered: the `window` artifacts for its os/arch/platform, newest
-- release first. NO rows while an unresolved incident exists (a fork freezes updates).
CREATE OR REPLACE FUNCTION cairn_release_offer(p_os text, p_arch text, p_platform text)
RETURNS TABLE(release_version bigint, manifest bytea, release_signatures bytea[], idx int,
              digest bytea, byte_len bigint, generation_line text)
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = public, pg_temp AS $$
    SELECT b.release_version, b.manifest, b.release_signatures, a.idx, a.digest, a.byte_len,
           a.generation_line
      FROM release_bundle b
      JOIN release_artifact a ON a.release_version = b.release_version
     WHERE a.kind = 'window' AND a.os = p_os AND a.arch = p_arch AND a.platform = p_platform
       AND NOT EXISTS (SELECT 1 FROM release_incident i WHERE i.resolved_at IS NULL)
     ORDER BY b.release_version DESC, a.idx;
$$;

-- The trust-root chain after version p_after, ascending (the terminal walks it from its pin).
-- A NULL p_after is read as 0 (the whole chain), not an error: a terminal with no pin asks for all.
CREATE OR REPLACE FUNCTION cairn_release_roots(p_after bigint)
RETURNS TABLE(version bigint, payload bytea, signatures bytea[])
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = public, pg_temp AS $$
    SELECT r.version, r.payload, r.signatures FROM release_root r
     WHERE r.version > coalesce(p_after, 0) ORDER BY r.version;
$$;

-- Store file bytes out of line and uncompressed, so PostgreSQL can slice a chunk from TOAST
-- without reading the whole (up to 64 MiB) artifact on every 256 KiB call. Repeatable.
ALTER TABLE release_artifact ALTER COLUMN content SET STORAGE EXTERNAL;

-- One slice of a `window` artifact: p_len bytes from the 0-based byte p_offset ('' at or past the
-- end, for any bigint offset). Capped at 256 KiB so the window's one shared connection is held
-- for one chunk at a time. An unknown digest, a non-`window` kind (stored but never served) and an
-- open incident (incidents freeze EVERY channel; there is one today) are errors, never empty.
CREATE OR REPLACE FUNCTION cairn_release_chunk(p_digest bytea, p_offset bigint, p_len int)
RETURNS bytea LANGUAGE plpgsql STABLE SECURITY DEFINER SET search_path = public, pg_temp AS $$
DECLARE
    v_chunk bytea;
BEGIN
    IF p_len IS NULL OR p_len < 0 THEN
        RAISE EXCEPTION 'a release chunk length must be given and not negative';
    END IF;
    IF p_len > 262144 THEN
        RAISE EXCEPTION 'a release chunk is at most 262144 bytes';
    END IF;
    IF p_offset IS NULL OR p_offset < 0 THEN
        RAISE EXCEPTION 'a release chunk offset must be given and not negative';
    END IF;
    IF EXISTS (SELECT 1 FROM release_incident i WHERE i.resolved_at IS NULL) THEN
        RAISE EXCEPTION 'release updates are stopped while an incident is open';
    END IF;
    -- least() keeps the ::int cast in range even where the planner folds it ahead of the CASE
    -- (the CASE already answers '' for any offset at or past the end).
    SELECT CASE WHEN p_offset >= octet_length(a.content) THEN ''::bytea
                ELSE substring(a.content FROM (least(p_offset, 2147483646) + 1)::int FOR p_len) END
      INTO v_chunk
      FROM release_artifact a WHERE a.digest = p_digest AND a.kind = 'window' ORDER BY a.release_version DESC LIMIT 1;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'no release artifact has that digest';
    END IF;
    RETURN v_chunk;
END;
$$;

REVOKE ALL ON FUNCTION cairn_release_offer(text, text, text) FROM PUBLIC;
REVOKE ALL ON FUNCTION cairn_release_roots(bigint) FROM PUBLIC;
REVOKE ALL ON FUNCTION cairn_release_chunk(bytea, bigint, int) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION cairn_release_offer(text, text, text) TO cairn_window;
GRANT EXECUTE ON FUNCTION cairn_release_roots(bigint) TO cairn_window;
GRANT EXECUTE ON FUNCTION cairn_release_chunk(bytea, bigint, int) TO cairn_window;

COMMIT;
