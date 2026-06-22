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
-- diverge. Two structural guards enforce that here:
--   1. a distinct `local_ref` DOMAIN, so a surrogate cannot be passed where a
--      `uuid` is expected (a leak becomes a type error, db/tests/008_*_test.sql);
--   2. all interning/de-interning is confined to the two functions below
--      (intern_patient on ingress, patient_uuid on egress) — the projection's
--      private chokepoints, mirroring the §9.6 submit/egress floor.
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
-- The type-system guard. local_ref is structurally a bigint, but the NAMED type
-- is what stops a surrogate being silently used as a global id: a column or
-- function parameter declared `uuid` will not accept a `local_ref`, and vice
-- versa. CREATE DOMAIN is not IF-NOT-EXISTS-able, so guard it idempotently.
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

CREATE TABLE IF NOT EXISTS chart_note_s (
    note_seq     BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    patient_lref local_ref NOT NULL REFERENCES patient_ref (local_ref),  -- 8-byte surrogate FK
    event_id     UUID NOT NULL,
    recorded_at  TIMESTAMPTZ
);
CREATE INDEX IF NOT EXISTS chart_note_s_patient_idx ON chart_note_s (patient_lref);

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
-- Incremental maintenance, AFTER INSERT on event_log — the same trigger-driven,
-- no-full-recompute path as 002 (ADR-0001). It interns the patient (ingress
-- chokepoint) and folds the event into both child projections so B5 measures the
-- two shapes under an identical write stream.
-- ---------------------------------------------------------------------------
CREATE OR REPLACE FUNCTION surrogate_project_apply()
RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE v_lref BIGINT;
BEGIN
    IF NEW.event_type IN ('patient.created', 'patient.amended') THEN
        -- Establish the anchor binding as soon as the patient is first seen.
        PERFORM intern_patient(NEW.patient_id);

    ELSIF NEW.event_type = 'note.added' THEN
        v_lref := intern_patient(NEW.patient_id);
        INSERT INTO chart_note_u (patient_id, event_id, recorded_at)
            VALUES (NEW.patient_id, NEW.event_id, NEW.recorded_at);
        INSERT INTO chart_note_s (patient_lref, event_id, recorded_at)
            VALUES (v_lref, NEW.event_id, NEW.recorded_at);
    END IF;
    RETURN NULL;  -- AFTER trigger
END;
$$;

DROP TRIGGER IF EXISTS event_log_project_surrogate ON event_log;
CREATE TRIGGER event_log_project_surrogate AFTER INSERT ON event_log
    FOR EACH ROW EXECUTE FUNCTION surrogate_project_apply();

COMMIT;
