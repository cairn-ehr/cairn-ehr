-- Cairn walking skeleton — a trigger-maintained projection (Spike 0001 §3.5, Bet B).
--
-- poc/replication-failover derived "current truth" with VIEWs (recomputed per
-- query — nothing to measure). Bet B asks the load-bearing ADR-0001 question:
-- is *incremental* projection maintenance cheap enough on a Pi to keep chart
-- reads local and fast? That only has an answer if the projection is a real
-- trigger-maintained TABLE updated AFTER INSERT — which is what this file builds.
--
-- This is the "fat Postgres" tier (ADR-0001/§9.4): all merge/projection logic
-- lives in the database, trigger-maintained, PL/pgSQL by default with a per-
-- projection pgrx (in-DB Rust) escape hatch if Bet B shows PL/pgSQL is too slow.

BEGIN;

-- ── Shared overlay-winner predicate (#115) ──────────────────────────────────────────────
-- Every standing-state overlay (patient_chart below, plus patient_link/db-018,
-- chart_dispute/db-023, chart_identity_state/db-024, name_repudiation/db-025) folds a new
-- event in only when it OUTRANKS the stored winner. Ranking is the HLC total order (wall,
-- then counter, then origin) exactly as before — BUT a Byzantine or broken signer can reuse
-- its own (wall, counter, origin) triple across two DIFFERENT signed bodies. A plain
-- strict-`>` guard is then false in both directions, so the winner would be decided by
-- ARRIVAL ORDER and two honest nodes could converge to different standing state — a silent
-- cross-node divergence in the safety-critical projection layer, exactly what "sync = safe
-- set-union" must not allow. The event's content_address (the BYTEA multihash of its signed
-- bytes) is the deterministic final tiebreaker: canonical, UNIQUE, byte-compared (so it is
-- immune to the TEXT-collation concern that #69 tracks for the `origin` comparison), and
-- never shared by two distinct events. Appending it makes the overlay pick the SAME winner
-- on every node even under an HLC collision. COALESCE encodes "no current winner yet" (an
-- overlay's first insert, e.g. patient_chart's note-only row): -1 wall/counter and '' origin
-- sort below any real event, and an empty bytea sorts below any real \x1220… address, so a
-- real event always beats an absent one. Only the CURRENT side is COALESCEd — the NEW side is
-- always a real, fully-populated event.
-- Its #157 collision-detection sibling (cairn_hlc_triple_collision + hlc_collision_log) lives in db/029.
CREATE OR REPLACE FUNCTION cairn_hlc_overlay_wins(
    new_wall bigint, new_counter int, new_origin text, new_addr bytea,
    cur_wall bigint, cur_counter int, cur_origin text, cur_addr bytea
) RETURNS boolean LANGUAGE sql IMMUTABLE AS $$
    SELECT (new_wall, new_counter, new_origin, new_addr)
         > (COALESCE(cur_wall, -1), COALESCE(cur_counter, -1),
            COALESCE(cur_origin, ''), COALESCE(cur_addr, '\x'::bytea));
$$;

-- The projection Bet B times: one row per patient, kept current by overlay.
CREATE TABLE IF NOT EXISTS patient_chart (
    patient_id     UUID PRIMARY KEY,
    name           TEXT,
    dob            TEXT,
    sex            TEXT,
    -- Provenance of the winning demographic event (HLC of the last overlay).
    demo_hlc_wall  BIGINT,
    demo_hlc_count INTEGER,
    demo_origin    TEXT,
    demo_content_address BYTEA,   -- winning demographic event's content address; #115 tiebreak
                                  -- (nullable: a note-only row has no demographic winner yet)
    note_count     INTEGER     NOT NULL DEFAULT 0,
    last_activity  TIMESTAMPTZ,
    updated_at     TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);

-- Incremental maintenance: AFTER INSERT on event_log, fold exactly the one new
-- event into the projection. No full recompute — that is the whole point of the
-- measurement. "Latest demographic wins by HLC order" is an overlay, never an
-- edit to the log (principle #2): superseded versions remain in event_log.
CREATE OR REPLACE FUNCTION patient_chart_apply()
RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE
    v_cur record;  -- current demographic winner, for #157 collision detection
BEGIN
    IF NEW.event_type IN ('patient.created', 'patient.amended') THEN
        -- #157: before overlaying, detect a Byzantine HLC-triple collision against the current
        -- demographic winner and record an advisory signal. Reads the demo_* provenance columns
        -- aliased to the predicate's parameter names; a note-only row has null demo_* → the
        -- null-safe predicate returns false (no false signal).
        SELECT demo_hlc_wall AS hlc_wall, demo_hlc_count AS hlc_counter,
               demo_origin AS origin, demo_content_address AS content_address
          INTO v_cur
          FROM patient_chart WHERE patient_id = NEW.patient_id;
        IF FOUND AND cairn_hlc_triple_collision(
                NEW.hlc_wall, NEW.hlc_counter, NEW.node_origin, NEW.content_address,
                v_cur.hlc_wall, v_cur.hlc_counter, v_cur.origin, v_cur.content_address) THEN
            PERFORM cairn_record_hlc_collision(
                'patient_chart', NEW.patient_id::text,
                NEW.hlc_wall, NEW.hlc_counter, NEW.node_origin,
                NEW.content_address, v_cur.content_address);
        END IF;

        INSERT INTO patient_chart AS pc (
            patient_id, name, dob, sex,
            demo_hlc_wall, demo_hlc_count, demo_origin, demo_content_address,
            last_activity, updated_at)
        VALUES (
            NEW.patient_id,
            NEW.body ->> 'name', NEW.body ->> 'dob', NEW.body ->> 'sex',
            NEW.hlc_wall, NEW.hlc_counter, NEW.node_origin, NEW.content_address,
            NEW.recorded_at, clock_timestamp())
        ON CONFLICT (patient_id) DO UPDATE SET
            -- Only overlay if this event is HLC-later than the current winner.
            name           = CASE WHEN cairn_hlc_overlay_wins(NEW.hlc_wall, NEW.hlc_counter, NEW.node_origin, NEW.content_address,
                                     pc.demo_hlc_wall, pc.demo_hlc_count, pc.demo_origin, pc.demo_content_address)
                                  THEN NEW.body ->> 'name' ELSE pc.name END,
            dob            = CASE WHEN cairn_hlc_overlay_wins(NEW.hlc_wall, NEW.hlc_counter, NEW.node_origin, NEW.content_address,
                                     pc.demo_hlc_wall, pc.demo_hlc_count, pc.demo_origin, pc.demo_content_address)
                                  THEN NEW.body ->> 'dob' ELSE pc.dob END,
            sex            = CASE WHEN cairn_hlc_overlay_wins(NEW.hlc_wall, NEW.hlc_counter, NEW.node_origin, NEW.content_address,
                                     pc.demo_hlc_wall, pc.demo_hlc_count, pc.demo_origin, pc.demo_content_address)
                                  THEN NEW.body ->> 'sex' ELSE pc.sex END,
            demo_content_address = CASE WHEN cairn_hlc_overlay_wins(NEW.hlc_wall, NEW.hlc_counter, NEW.node_origin, NEW.content_address,
                                     pc.demo_hlc_wall, pc.demo_hlc_count, pc.demo_origin, pc.demo_content_address)
                                  THEN NEW.content_address ELSE pc.demo_content_address END,
            demo_hlc_wall  = GREATEST(pc.demo_hlc_wall, NEW.hlc_wall),
            demo_hlc_count = CASE WHEN cairn_hlc_overlay_wins(NEW.hlc_wall, NEW.hlc_counter, NEW.node_origin, NEW.content_address,
                                     pc.demo_hlc_wall, pc.demo_hlc_count, pc.demo_origin, pc.demo_content_address)
                                  THEN NEW.hlc_counter ELSE pc.demo_hlc_count END,
            demo_origin    = CASE WHEN cairn_hlc_overlay_wins(NEW.hlc_wall, NEW.hlc_counter, NEW.node_origin, NEW.content_address,
                                     pc.demo_hlc_wall, pc.demo_hlc_count, pc.demo_origin, pc.demo_content_address)
                                  THEN NEW.node_origin ELSE pc.demo_origin END,
            last_activity  = GREATEST(pc.last_activity, NEW.recorded_at),
            updated_at     = clock_timestamp();

    ELSIF NEW.event_type = 'note.added' THEN
        INSERT INTO patient_chart AS pc (patient_id, note_count, last_activity, updated_at)
        VALUES (NEW.patient_id, 1, NEW.recorded_at, clock_timestamp())
        ON CONFLICT (patient_id) DO UPDATE SET
            note_count    = pc.note_count + 1,
            last_activity = GREATEST(pc.last_activity, NEW.recorded_at),
            updated_at    = clock_timestamp();
    END IF;

    RETURN NULL;  -- AFTER trigger
END;
$$;

DROP TRIGGER IF EXISTS event_log_project ON event_log;
CREATE TRIGGER event_log_project AFTER INSERT ON event_log
    FOR EACH ROW EXECUTE FUNCTION patient_chart_apply();

COMMIT;
