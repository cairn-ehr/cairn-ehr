-- Cairn walking skeleton — the dual-identifier discipline (ADR-0031, data-model §3.18).
--
-- WHY THIS FILE EXISTS (for a junior dev joining the team)
-- --------------------------------------------------------
-- Cairn's identity must be globally unique and offline-mintable, so the canonical
-- identifier of a patient is a UUIDv7 (event_log.patient_id). That is the right
-- key for *identity* and the wrong key for physical *join keys*: a 16-byte UUID
-- repeated across every projection row, and indexed many times, inflates every
-- index and evicts cache — and on Pi-class hardware a slow chart read fails
-- paper-parity (principle 3), which in an EHR is a SAFETY issue, not a nicety.
--
-- The fix (ADR-0031): keep the canonical UUID on the wire/signed plane, and in
-- the LOCAL projection plane intern it to a dense node-local bigint "surrogate"
-- used as the physical foreign-key/join key. The surrogate is ~3x smaller and
-- sequential, so its indexes are small and cache-resident.
--
-- THE ONE HARD RULE: the surrogate must NEVER leave the projection plane — never
-- in a signed body, never on the inter-node wire, never as a content-address
-- input, never as a stable API identity. If it leaked, two nodes would assign
-- different integers to the same patient and set-union sync would silently
-- diverge. The guarantee that actually stops that is structural and lives in TWO
-- places — note the `local_ref` DOMAIN is NOT, by itself, the leak-proof barrier:
--   1. THE LOAD-BEARING GUARANTEE: the canonical/signed plane is typed `uuid`
--      (event_log.patient_id) and `bigint <> uuid`, so a surrogate cannot be cast
--      into a signed body — backed by the G2 test (event_log stays surrogate-free,
--      db/tests/008_*_test.sql). A bigint simply cannot become a uuid.
--   2. all interning/de-interning is confined to the two functions below
--      (intern_patient on ingress, patient_uuid on egress) — the projection's
--      private chokepoints, mirroring the §9.6 submit/egress floor.
-- The `local_ref` DOMAIN is a complementary intent-signal + ONE-directional guard
-- (a `uuid` won't coerce to `local_ref`), NOT a symmetric barrier: a domain over
-- bigint accepts any bigint, so the surrogate→bigint direction is not blocked by
-- it (proven honestly by G4 in the test). Don't over-trust the domain.
--
-- This file is also the build-prep artifact for Spike 0001 Bet B5: it stands up
-- a UUID-keyed child projection (chart_note_u, today's shape) and a surrogate-
-- keyed one (chart_note_s, the ADR-0031 shape) from the SAME event stream, so the
-- Pi run can MEASURE whether the smaller foreign-key index actually pays on ARM.
--
-- Pure SQL on purpose: it depends only on 001_envelope.sql (event_log) — no pgrx,
-- no cairn_verify — because the discipline lives wholly in the projection plane.

BEGIN;

-- ---------------------------------------------------------------------------
-- The type-system intent-signal. local_ref is structurally a bigint; the NAMED
-- type documents "this is a node-local surrogate, not a global id" and gives a
-- ONE-directional guard: a column/parameter declared `uuid` will not accept a
-- `local_ref` (uuid and bigint do not inter-coerce). It is NOT symmetric — a
-- parameter declared `local_ref` accepts any plain `bigint`, so it does not by
-- itself stop the surrogate→bigint direction. The leak-proof guarantee is the
-- `uuid` typing of the signed plane + G2 (see header). CREATE DOMAIN is not
-- IF-NOT-EXISTS-able, so guard it idempotently.
-- ---------------------------------------------------------------------------
DO $$ BEGIN
    CREATE DOMAIN local_ref AS BIGINT;
EXCEPTION WHEN duplicate_object THEN NULL; END $$;

-- ---------------------------------------------------------------------------
-- The interning dictionary = the ANCHOR row: the ONE place the UUID<->surrogate
-- binding lives, carrying BOTH fields. ("Carry both" is correct here and only
-- here; carrying the UUID on every *referencing* row would re-import the exact
-- fan-out cost we are removing — ADR-0031.) local_ref is a dense IDENTITY PK;
-- patient_id is UNIQUE so interning is idempotent. ~8 extra bytes per *patient*,
-- not per *reference*.
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS patient_ref (
    local_ref   BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    patient_id  UUID   NOT NULL UNIQUE
);

-- Ingress: resolve a canonical UUID to its node-local surrogate, minting on first
-- sight. Concurrency-safe: two sessions interning the same new patient race on
-- the UNIQUE index; ON CONFLICT lets the loser read the winner's ref. Returns the
-- typed surrogate so callers thread `local_ref`, never a bare bigint.
CREATE OR REPLACE FUNCTION intern_patient(p_patient UUID)
RETURNS local_ref LANGUAGE plpgsql AS $$
DECLARE v BIGINT;
BEGIN
    -- Fast path: already interned (the overwhelmingly common case).
    SELECT local_ref INTO v FROM patient_ref WHERE patient_id = p_patient;
    IF FOUND THEN RETURN v; END IF;
    -- Mint, tolerating a concurrent minter.
    INSERT INTO patient_ref (patient_id) VALUES (p_patient)
        ON CONFLICT (patient_id) DO NOTHING
        RETURNING local_ref INTO v;
    IF v IS NULL THEN  -- someone else won the race; read their ref
        SELECT local_ref INTO v FROM patient_ref WHERE patient_id = p_patient;
    END IF;
    RETURN v;
END;
$$;

-- Egress: rehydrate a surrogate back to its canonical UUID. Every wire/API egress
-- path goes through here, so the global id — never the surrogate — crosses the
-- node boundary. Parameter is typed `local_ref`, so a stray uuid won't type-check.
CREATE OR REPLACE FUNCTION patient_uuid(p_ref local_ref)
RETURNS UUID LANGUAGE sql STABLE AS $$
    SELECT patient_id FROM patient_ref WHERE local_ref = p_ref;
$$;

-- ---------------------------------------------------------------------------
-- Two child projections of note events, identical but for the patient key — the
-- A/B that Spike 0001 Bet B5 measures. Each holds one row per note.added event,
-- the realistic high-fan-out case where the key-width cost actually lands.
--   chart_note_u : keyed by the 16-byte canonical UUID (today's shape)
--   chart_note_s : keyed by the 8-byte node-local surrogate (the ADR-0031 shape)
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS chart_note_u (
    note_seq    BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    patient_id  UUID NOT NULL,             -- 16-byte canonical FK, repeated per note
    event_id    UUID NOT NULL,
    recorded_at TIMESTAMPTZ
);
CREATE INDEX IF NOT EXISTS chart_note_u_patient_idx ON chart_note_u (patient_id);
-- #208/ADR-0057: the PK is the surrogate note_seq (an identity column), not event_id,
-- so a replayed note.added event (heal-mode reproject, or the same event redelivered)
-- would otherwise INSERT a second row for the SAME event — the note_seq identity has
-- no natural-key protection of its own. event_id is the natural key (one row per
-- event); unique-index it so the apply fn's ON CONFLICT (event_id) below can dedup by
-- EVENT identity, making this projection heal-safe.
CREATE UNIQUE INDEX IF NOT EXISTS chart_note_u_event_idx ON chart_note_u (event_id);

CREATE TABLE IF NOT EXISTS chart_note_s (
    note_seq     BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    patient_lref local_ref NOT NULL REFERENCES patient_ref (local_ref),  -- 8-byte surrogate FK
    event_id     UUID NOT NULL,
    recorded_at  TIMESTAMPTZ
);
CREATE INDEX IF NOT EXISTS chart_note_s_patient_idx ON chart_note_s (patient_lref);
-- Same #208/ADR-0057 replay-idempotency fix as chart_note_u above.
CREATE UNIQUE INDEX IF NOT EXISTS chart_note_s_event_idx ON chart_note_s (event_id);

-- Egress view: how a sync-emit / API read of the surrogate-keyed child looks. It
-- joins back to the anchor and exposes the canonical UUID only — the surrogate
-- (patient_lref) is deliberately NOT projected, so it cannot ride the wire.
CREATE OR REPLACE VIEW chart_note_s_egress AS
    SELECT s.event_id,
           r.patient_id,            -- canonical uuid, rehydrated at the boundary
           s.recorded_at
    FROM chart_note_s s
    JOIN patient_ref r ON r.local_ref = s.patient_lref;

-- ---------------------------------------------------------------------------
-- Incremental maintenance — the same trigger-driven, no-full-recompute path as
-- 002 (ADR-0001), now folded into the #208/ADR-0057 generic dispatcher (db/005)
-- rather than its own bespoke per-type trigger. It interns the patient (ingress
-- chokepoint) and folds the event into both child projections so B5 measures the
-- two shapes under an identical write stream.
--
-- The old bespoke trigger is superseded by cairn_projection_dispatch_trg (db/005,
-- ADR-0057); this fn is now registered in cairn_projection_apply below (#208) —
-- registered IN THIS FILE (not db/005) because db/008 is spike-only and loaded by
-- neither product loader (crates/cairn-node/src/db.rs, crates/cairn-sync/src/main.rs),
-- only by scripts/run-db-sql-tests.sh's throwaway rig.
-- ---------------------------------------------------------------------------
DROP TRIGGER IF EXISTS event_log_project_surrogate ON event_log;
-- The old zero-arg trigger-function signature is superseded by the (event_log)
-- apply-fn signature below; CREATE OR REPLACE cannot change a function's arg
-- list (it would overload, not replace), so drop the old signature explicitly —
-- without it an upgraded-in-place DB keeps BOTH the zero-arg trigger fn and its
-- trigger, double-firing every projection (ADR-0057).
DROP FUNCTION IF EXISTS surrogate_project_apply();

CREATE OR REPLACE FUNCTION surrogate_project_apply(e event_log)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE v_lref BIGINT;
BEGIN
    IF e.event_type IN ('patient.created', 'patient.amended') THEN
        -- Establish the anchor binding as soon as the patient is first seen.
        -- intern_patient is idempotent-by-design (ON CONFLICT (patient_id) DO NOTHING +
        -- read-back, db/008 part "Ingress" above) — no further replay guard needed here.
        PERFORM intern_patient(e.patient_id);

    ELSIF e.event_type = 'note.added' THEN
        v_lref := intern_patient(e.patient_id);
        -- ON CONFLICT (event_id) DO NOTHING: chart_note_u/chart_note_s are keyed by the
        -- surrogate note_seq identity column, not event_id, so without this a heal-mode
        -- reproject (or any redelivery of the same event) would insert a SECOND row for
        -- the same note — dedup by EVENT identity via the unique indexes added above.
        INSERT INTO chart_note_u (patient_id, event_id, recorded_at)
            VALUES (e.patient_id, e.event_id, e.recorded_at)
            ON CONFLICT (event_id) DO NOTHING;
        INSERT INTO chart_note_s (patient_lref, event_id, recorded_at)
            VALUES (v_lref, e.event_id, e.recorded_at)
            ON CONFLICT (event_id) DO NOTHING;
    END IF;
    RETURN;
END;
$$;
-- A trigger fn could never be called directly; a plain fn gets PUBLIC EXECUTE by
-- default. Same discipline as every privileged fn in db/005 (Task-1 review finding).
REVOKE EXECUTE ON FUNCTION surrogate_project_apply(event_log) FROM PUBLIC;

-- Registered apply fn for the #208/ADR-0057 generic dispatcher (db/005) +
-- cairn_reproject heal/rebuild (db/039) — registered HERE (not db/005) because this
-- migration is spike-only (see the header note above): only a rig that loads db/008
-- ever sees these three rows, which is exactly right (the product loaders' registry
-- stays at its product-scope row count). run_order 20 (after db/005's own
-- patient_chart_apply registration at run_order 10 for the SAME three event types) —
-- both fire per event, patient_chart_apply first, mirroring the old alphabetical
-- trigger-name firing order. #214 + steady-state discipline: converge these rows to
-- the migration text on every connect, but stay write-free once already converged
-- (no dead tuples, no validate-trigger fire).
INSERT INTO cairn_projection_apply AS r (event_type, apply_fn, projection_tables, run_order, heal_safe) VALUES
    ('patient.created', 'surrogate_project_apply', ARRAY['patient_ref'], 20, TRUE),
    ('patient.amended', 'surrogate_project_apply', ARRAY['patient_ref'], 20, TRUE),
    ('note.added',      'surrogate_project_apply', ARRAY['patient_ref', 'chart_note_u', 'chart_note_s'], 20, TRUE)
ON CONFLICT (event_type, apply_fn) DO UPDATE SET
    projection_tables = EXCLUDED.projection_tables,
    run_order         = EXCLUDED.run_order,
    heal_safe         = EXCLUDED.heal_safe
WHERE (r.projection_tables, r.run_order, r.heal_safe)
      IS DISTINCT FROM (EXCLUDED.projection_tables, EXCLUDED.run_order, EXCLUDED.heal_safe);

COMMIT;
