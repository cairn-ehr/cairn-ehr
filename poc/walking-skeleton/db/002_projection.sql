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
BEGIN
    IF NEW.event_type IN ('patient.created', 'patient.amended') THEN
        INSERT INTO patient_chart AS pc (
            patient_id, name, dob, sex,
            demo_hlc_wall, demo_hlc_count, demo_origin,
            last_activity, updated_at)
        VALUES (
            NEW.patient_id,
            NEW.body ->> 'name', NEW.body ->> 'dob', NEW.body ->> 'sex',
            NEW.hlc_wall, NEW.hlc_counter, NEW.node_origin,
            NEW.recorded_at, clock_timestamp())
        ON CONFLICT (patient_id) DO UPDATE SET
            -- Only overlay if this event is HLC-later than the current winner.
            name           = CASE WHEN (NEW.hlc_wall, NEW.hlc_counter, NEW.node_origin)
                                     > (COALESCE(pc.demo_hlc_wall,-1), COALESCE(pc.demo_hlc_count,-1), COALESCE(pc.demo_origin,''))
                                  THEN NEW.body ->> 'name' ELSE pc.name END,
            dob            = CASE WHEN (NEW.hlc_wall, NEW.hlc_counter, NEW.node_origin)
                                     > (COALESCE(pc.demo_hlc_wall,-1), COALESCE(pc.demo_hlc_count,-1), COALESCE(pc.demo_origin,''))
                                  THEN NEW.body ->> 'dob' ELSE pc.dob END,
            sex            = CASE WHEN (NEW.hlc_wall, NEW.hlc_counter, NEW.node_origin)
                                     > (COALESCE(pc.demo_hlc_wall,-1), COALESCE(pc.demo_hlc_count,-1), COALESCE(pc.demo_origin,''))
                                  THEN NEW.body ->> 'sex' ELSE pc.sex END,
            demo_hlc_wall  = GREATEST(pc.demo_hlc_wall, NEW.hlc_wall),
            demo_hlc_count = CASE WHEN (NEW.hlc_wall, NEW.hlc_counter, NEW.node_origin)
                                     > (COALESCE(pc.demo_hlc_wall,-1), COALESCE(pc.demo_hlc_count,-1), COALESCE(pc.demo_origin,''))
                                  THEN NEW.hlc_counter ELSE pc.demo_hlc_count END,
            demo_origin    = CASE WHEN (NEW.hlc_wall, NEW.hlc_counter, NEW.node_origin)
                                     > (COALESCE(pc.demo_hlc_wall,-1), COALESCE(pc.demo_hlc_count,-1), COALESCE(pc.demo_origin,''))
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
