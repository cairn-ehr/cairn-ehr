-- 060_medication_cessation_reason.sql — every stop's reason stays on the medication list (#857;
-- data-model §3.3).
--
-- WHY THIS EXISTS. A stop of a stopped line is admitted on purpose (ADR-0082 decision 4): its
-- reason may be the only record of an adverse reaction while no allergy stream exists. But
-- the list read its reasons from two one-winner projections, so the second stop ERASED the
-- first from view in two places:
--   * per thread — medication_cessation (db/031) keeps one row per medication_id and a newer
--     stop overwrites `reason`, including with NULL (`--reason` is optional);
--   * per line   — medication_group_cessation (db/033) keeps one member thread's stop.
-- So "angioedema" became "patient request", or nothing; principle 4: a recorded fact shown as
-- nothing.
--
-- WHAT IS STORED. One row per cessation event that gave a non-blank reason, keyed by that
-- event's content_address (no overlay, no winner: every stop's reason is its own fact, like
-- db/059's holds). The two winner projections above are untouched: they still decide whether
-- a line is stopped and which stop's date and reason the line SHOWS; this table supplies the
-- OTHER reasons the reader (cairn-node `medication/read.rs`) lists beside it.
--
-- HOW EXISTING NODES GET THEIR ROWS. This file bumps SCHEMA_GENERATION to 60, and a
-- generation change makes the loader run a full heal (`cairn_reproject('', false, 'loader')`,
-- db/039), which replays every cessation through the apply fn registered below. Shredding a
-- stop deletes its row (db/037's cairn_execute_shred, keyed by content_address).
--
-- Bodies are born sealed (ADR-0052): the apply fn reads cairn_clear_payload(e) and does nothing
-- when this node holds no custody (honest degradation); custody arriving later re-runs it.
-- Node-only, like every medication file (cairn-sync's list lags legitimately, #284).
BEGIN;

CREATE TABLE IF NOT EXISTS medication_cessation_reason (
    content_address BYTEA   PRIMARY KEY,
    medication_id   UUID    NOT NULL,
    reason          TEXT    NOT NULL CHECK (btrim(reason) <> ''),
    hlc_wall        BIGINT  NOT NULL,
    hlc_counter     INTEGER NOT NULL,
    origin          TEXT    NOT NULL
);
GRANT SELECT ON medication_cessation_reason TO cairn_agent;
-- The list read joins on the thread.
CREATE INDEX IF NOT EXISTS medication_cessation_reason_medication_idx
    ON medication_cessation_reason (medication_id);

-- Insert-or-nothing: a replay (heal, late custody) re-runs this for the same event, and the
-- per-thread patient guard is already run by medication_cessation_apply (db/031) for the same
-- event in the same dispatch, so it is not repeated here.
CREATE OR REPLACE FUNCTION medication_cessation_reason_apply(e event_log)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    p jsonb := cairn_clear_payload(e);
BEGIN
    IF p IS NULL OR btrim(coalesce(p ->> 'reason', '')) = '' THEN RETURN; END IF;
    INSERT INTO medication_cessation_reason
        (content_address, medication_id, reason, hlc_wall, hlc_counter, origin)
    VALUES (e.content_address, (p ->> 'medication_id')::uuid, p ->> 'reason',
            e.hlc_wall, e.hlc_counter, e.node_origin)
    ON CONFLICT (content_address) DO NOTHING;
END;
$$;
-- Same discipline as every apply fn (db/031): a plain fn gets PUBLIC EXECUTE by default.
REVOKE EXECUTE ON FUNCTION medication_cessation_reason_apply(event_log) FROM PUBLIC;

-- Registered beside db/031's medication_cessation_apply for the same event type (the
-- dispatcher runs every registered fn, db/005). #214: converge to this text, write-free once
-- converged.
INSERT INTO cairn_projection_apply AS r (event_type, apply_fn, projection_tables, run_order, heal_safe) VALUES
    ('clinical.medication-cessation.asserted', 'medication_cessation_reason_apply',
     ARRAY['medication_cessation_reason'], 10, TRUE)
ON CONFLICT (event_type, apply_fn) DO UPDATE SET
    projection_tables = EXCLUDED.projection_tables,
    run_order         = EXCLUDED.run_order,
    heal_safe         = EXCLUDED.heal_safe
WHERE (r.projection_tables, r.run_order, r.heal_safe)
      IS DISTINCT FROM (EXCLUDED.projection_tables, EXCLUDED.run_order, EXCLUDED.heal_safe);

COMMIT;
