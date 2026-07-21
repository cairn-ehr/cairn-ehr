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
-- set-union" must not allow. The `origin` comparison is itself collation-safe: it is compared
-- under COLLATE "C" (byte order) so every node picks the same winner regardless of its default
-- TEXT collation — the origin string can otherwise sort differently under an ICU/locale
-- collation than under "C", which would silently re-diverge the ranking across nodes with
-- different locales (ADR-0045, #69). The event's content_address (the BYTEA multihash of its
-- signed bytes) remains the deterministic final Byzantine same-origin tiebreaker: canonical,
-- UNIQUE, byte-compared (collation-free — BYTEA has no collation), and never shared by two
-- distinct events. Appending it makes the overlay pick the SAME winner on every node even
-- under an HLC collision. COALESCE encodes "no current winner yet" (an
-- overlay's first insert, e.g. patient_chart's note-only row): -1 wall/counter and '' origin
-- sort below any real event, and an empty bytea sorts below any real \x1220… address, so a
-- real event always beats an absent one. Only the CURRENT side is COALESCEd — the NEW side is
-- always a real, fully-populated event.
-- Its #157 collision-detection sibling (cairn_hlc_triple_collision + hlc_collision_log) lives in db/029.
CREATE OR REPLACE FUNCTION cairn_hlc_overlay_wins(
    new_wall bigint, new_counter int, new_origin text, new_addr bytea,
    cur_wall bigint, cur_counter int, cur_origin text, cur_addr bytea
) RETURNS boolean LANGUAGE sql IMMUTABLE AS $$
    SELECT (new_wall, new_counter, new_origin COLLATE "C", new_addr)
         > (COALESCE(cur_wall, -1), COALESCE(cur_counter, -1),
            COALESCE(cur_origin, '') COLLATE "C", COALESCE(cur_addr, '\x'::bytea));
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
-- Additive widening (#115 → issue #207): CREATE TABLE IF NOT EXISTS no-ops on a database
-- created before the column was added to the body above, so the widening must ALSO ship
-- as an idempotent ALTER (ADR-0012 additive-migration rule; the db/001 event_log pattern).
-- Without it, every trigger INSERT naming the column fails on an upgraded-in-place DB —
-- a total write outage at trigger depth. Guarded by migration_replay_widening.rs.
ALTER TABLE patient_chart ADD COLUMN IF NOT EXISTS demo_content_address BYTEA;

-- Incremental maintenance: called by cairn_projection_dispatch_trg (db/005,
-- ADR-0057) for exactly the one new event, folding it into the projection. No
-- full recompute — that is the whole point of the measurement. "Latest
-- demographic wins by HLC order" is an overlay, never an edit to the log
-- (principle #2): superseded versions remain in event_log.
--
-- The per-type trigger is superseded by cairn_projection_dispatch_trg (db/005,
-- ADR-0057); this fn is now registered in cairn_projection_apply there.
DROP TRIGGER IF EXISTS event_log_project ON event_log;
-- The old zero-arg trigger-function signature is superseded by the (event_log)
-- apply-fn signature below; CREATE OR REPLACE cannot change a function's arg
-- list (it would overload, not replace), so drop the old signature explicitly
-- (same idiom as db/005's `DROP FUNCTION IF EXISTS submit_event(bytea, bytea, bytea);`).
DROP FUNCTION IF EXISTS patient_chart_apply();

CREATE OR REPLACE FUNCTION patient_chart_apply(e event_log)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    v_cur record;  -- current demographic winner, for #157 collision detection
BEGIN
    -- ADR-0052 §2 seal-robustness (#10): a wrongly-sealed NON-clinical row holds CIPHERTEXT
    -- in e.body (refused at submit; admitted lenient at apply for lossless sync). Reading it
    -- below would drive NULLs into this projection and freeze the sync watermark — so a sealed
    -- row projects NOTHING (harmless ciphertext noise; no custody, no leak).
    IF e.sealed THEN RETURN; END IF;
    IF e.event_type IN ('patient.created', 'patient.amended') THEN
        -- #157: before overlaying, detect a Byzantine HLC-triple collision against the current
        -- demographic winner and record an advisory signal. Reads the demo_* provenance columns
        -- aliased to the predicate's parameter names; a note-only row has null demo_* → the
        -- null-safe predicate returns false (no false signal).
        SELECT demo_hlc_wall AS hlc_wall, demo_hlc_count AS hlc_counter,
               demo_origin AS origin, demo_content_address AS content_address
          INTO v_cur
          FROM patient_chart WHERE patient_id = e.patient_id;
        IF FOUND AND cairn_hlc_triple_collision(
                e.hlc_wall, e.hlc_counter, e.node_origin, e.content_address,
                v_cur.hlc_wall, v_cur.hlc_counter, v_cur.origin, v_cur.content_address) THEN
            PERFORM cairn_record_hlc_collision(
                'patient_chart', e.patient_id::text,
                e.hlc_wall, e.hlc_counter, e.node_origin,
                e.content_address, v_cur.content_address);
        END IF;

        INSERT INTO patient_chart AS pc (
            patient_id, name, dob, sex,
            demo_hlc_wall, demo_hlc_count, demo_origin, demo_content_address,
            last_activity, updated_at)
        VALUES (
            e.patient_id,
            e.body ->> 'name', e.body ->> 'dob', e.body ->> 'sex',
            e.hlc_wall, e.hlc_counter, e.node_origin, e.content_address,
            e.recorded_at, clock_timestamp())
        ON CONFLICT (patient_id) DO UPDATE SET
            -- Only overlay if this event is HLC-later than the current winner.
            name           = CASE WHEN cairn_hlc_overlay_wins(e.hlc_wall, e.hlc_counter, e.node_origin, e.content_address,
                                     pc.demo_hlc_wall, pc.demo_hlc_count, pc.demo_origin, pc.demo_content_address)
                                  THEN e.body ->> 'name' ELSE pc.name END,
            dob            = CASE WHEN cairn_hlc_overlay_wins(e.hlc_wall, e.hlc_counter, e.node_origin, e.content_address,
                                     pc.demo_hlc_wall, pc.demo_hlc_count, pc.demo_origin, pc.demo_content_address)
                                  THEN e.body ->> 'dob' ELSE pc.dob END,
            sex            = CASE WHEN cairn_hlc_overlay_wins(e.hlc_wall, e.hlc_counter, e.node_origin, e.content_address,
                                     pc.demo_hlc_wall, pc.demo_hlc_count, pc.demo_origin, pc.demo_content_address)
                                  THEN e.body ->> 'sex' ELSE pc.sex END,
            demo_content_address = CASE WHEN cairn_hlc_overlay_wins(e.hlc_wall, e.hlc_counter, e.node_origin, e.content_address,
                                     pc.demo_hlc_wall, pc.demo_hlc_count, pc.demo_origin, pc.demo_content_address)
                                  THEN e.content_address ELSE pc.demo_content_address END,
            demo_hlc_wall  = GREATEST(pc.demo_hlc_wall, e.hlc_wall),
            demo_hlc_count = CASE WHEN cairn_hlc_overlay_wins(e.hlc_wall, e.hlc_counter, e.node_origin, e.content_address,
                                     pc.demo_hlc_wall, pc.demo_hlc_count, pc.demo_origin, pc.demo_content_address)
                                  THEN e.hlc_counter ELSE pc.demo_hlc_count END,
            demo_origin    = CASE WHEN cairn_hlc_overlay_wins(e.hlc_wall, e.hlc_counter, e.node_origin, e.content_address,
                                     pc.demo_hlc_wall, pc.demo_hlc_count, pc.demo_origin, pc.demo_content_address)
                                  THEN e.node_origin ELSE pc.demo_origin END,
            last_activity  = GREATEST(pc.last_activity, e.recorded_at),
            updated_at     = clock_timestamp();

    ELSIF e.event_type = 'note.added' THEN
        INSERT INTO patient_chart AS pc (patient_id, note_count, last_activity, updated_at)
        VALUES (e.patient_id, 1, e.recorded_at, clock_timestamp())
        ON CONFLICT (patient_id) DO UPDATE SET
            note_count    = pc.note_count + 1,
            last_activity = GREATEST(pc.last_activity, e.recorded_at),
            updated_at    = clock_timestamp();
    END IF;

    RETURN;
END;
$$;

-- A trigger-shaped fn (the old signature) could never be called directly by a client —
-- only the trigger manager fired it. This apply-fn shape is a PLAIN function, so PUBLIC
-- gets EXECUTE by default; lock it down like every privileged fn in db/005
-- (cairn_event_twin, submit_event, ...). Only cairn_projection_dispatch's dynamic EXECUTE
-- calls it, running as the same owner that defined it (implicit owner EXECUTE survives
-- the REVOKE). This REVOKE is the template every later projection-apply-fn conversion
-- copies (#208/ADR-0057).
REVOKE EXECUTE ON FUNCTION patient_chart_apply(event_log) FROM PUBLIC;

COMMIT;
