-- 037_born_sealed.sql — ADR-0052: born-sealed clinical bodies, the custody plane.
--
-- WHY: every clinical JSONB body is sealed at write under a per-event DEK so the
-- ADR-0005 erasure ladder stays reachable forever. These are the MUTABLE tables
-- beside the append-only log: custody may rotate and derived plaintext may be
-- scrubbed, so none of them get the append-only trigger — deliberately.
-- The reserved event_log.dek_wrapped column (db/001) is retired unused: an
-- append-only row cannot hold rotating multi-holder custody.

BEGIN;

-- ---------------------------------------------------------------------------
-- 1. The node's X25519 public unwrap key (single row). The SECRET half lives in
--    the daemon keystore (derived from the Ed25519 seed, ADR-0026 escrow) and
--    NEVER enters the database — a DB backup can never reconstruct a DEK.
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS node_unwrap_key (
    singleton     BOOLEAN     PRIMARY KEY DEFAULT TRUE CHECK (singleton),
    unwrap_pub    BYTEA       NOT NULL CHECK (octet_length(unwrap_pub) = 32),
    registered_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);

CREATE OR REPLACE FUNCTION cairn_register_unwrap_key(p_pub BYTEA) RETURNS void
LANGUAGE plpgsql SECURITY DEFINER SET search_path = public AS $$
DECLARE v_existing BYTEA;
BEGIN
    SELECT unwrap_pub INTO v_existing FROM node_unwrap_key;
    IF v_existing IS NULL THEN
        INSERT INTO node_unwrap_key (unwrap_pub) VALUES (p_pub)
        ON CONFLICT (singleton) DO NOTHING;
    ELSIF v_existing <> p_pub THEN
        -- Unwrap-key rotation re-wraps every custody row — a deliberate,
        -- separate ceremony (ADR-0052 deferred list). Refuse a silent swap.
        RAISE EXCEPTION 'cairn_register_unwrap_key: a different unwrap key is registered — rotation is a separate ceremony (ADR-0052)';
    END IF;
END;
$$;

-- ---------------------------------------------------------------------------
-- 2. Per-event DEK custody (this node's wrapped copy). Destroying a row IS the
--    local half of a crypto-shred.
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS event_dek (
    event_id    UUID        PRIMARY KEY,
    dek_wrapped BYTEA       NOT NULL,
    wrapped_at  TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);

-- ---------------------------------------------------------------------------
-- 3. The operational clear view of sealed bodies: THE single derived-plaintext
--    surface (clear payload + clear twin), populated by the doors, deleted by a
--    shred. No FK to event_log: the door inserts this row BEFORE the event_log
--    row so the AFTER INSERT projection triggers can already read it (same
--    transaction — atomicity keeps them consistent). Future FTS/RAG indexes
--    MUST build on this table and nothing else (#92 (b)).
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS event_clear (
    event_id UUID  PRIMARY KEY,
    body     JSONB NOT NULL,   -- the CLEAR payload (matches event_log.body semantics)
    twin     TEXT  NOT NULL    -- the CLEAR legibility twin
);

-- ---------------------------------------------------------------------------
-- 4. The shred log: which events have been erased here. Rebuilt idempotently
--    from the append-only log below, so a restore/full-replay re-applies every
--    shred BEFORE any custody or projection could resurrect (§3.8).
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS erasure_shred_log (
    target_event_id UUID        PRIMARY KEY,
    shred_event_id  UUID        NOT NULL,
    basis           TEXT        NOT NULL,
    shredded_at     TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);

-- ---------------------------------------------------------------------------
-- 5. Projection read helper: the ONE way a projection trigger reads a clinical
--    payload. Unsealed → the derived body column; sealed → the clear shadow
--    (NULL when this node holds no custody: the caller skips projection).
-- ---------------------------------------------------------------------------
CREATE OR REPLACE FUNCTION cairn_clear_payload(ev event_log) RETURNS jsonb
LANGUAGE sql STABLE AS $$
    SELECT CASE WHEN NOT ev.sealed THEN ev.body
                ELSE (SELECT body FROM event_clear WHERE event_id = ev.event_id)
           END
$$;

-- ---------------------------------------------------------------------------
-- 6. erasure.shred.asserted — the rung-3 audited tombstone (plaintext BY DESIGN:
--    it must outlive every key). Classified additive: the erasure arm in the
--    doors owns target handling (a shred may arrive BEFORE its target on the
--    sync wire — the targets_other gate would wrongly reject that at apply).
-- ---------------------------------------------------------------------------
INSERT INTO event_type_class (event_type, mode, targets_other_author) VALUES
    ('erasure.shred.asserted', 'additive', false)
ON CONFLICT (event_type) DO NOTHING;

CREATE OR REPLACE FUNCTION cairn_check_erasure_shred(p_type text, b jsonb) RETURNS void
LANGUAGE plpgsql AS $$
BEGIN
    IF (b -> 'payload' ->> 'target_event_id') IS NULL THEN
        RAISE EXCEPTION 'erasure.shred: payload must name target_event_id (ADR-0052)';
    END IF;
    PERFORM (b -> 'payload' ->> 'target_event_id')::uuid;
    IF COALESCE(b -> 'payload' ->> 'basis', '') = '' THEN
        RAISE EXCEPTION 'erasure.shred: payload must carry a non-empty basis (the audited "why" — ADR-0005 rung 3)';
    END IF;
END;
$$;

INSERT INTO cairn_event_twin_check (event_type, check_fn, twin_required_msg) VALUES
    ('erasure.shred.asserted', 'cairn_check_erasure_shred',
     'erasure.shred requires a non-empty authored twin (the tombstone must be legible — ADR-0052)')
ON CONFLICT (event_type) DO NOTHING;

-- ---------------------------------------------------------------------------
-- 7. Shred execution, shared by both doors (never drifts): record, scrub
--    custody + derived plaintext + provenance-precise projection rows. The
--    event_log row is NEVER touched (append-only; signature still verifies).
--
--    Both medication_statement AND medication_cessation carry content_address
--    (db/031:149, db/031:225), so both are in scope here; the dose-seed row is
--    keyed by the assert event's own id (db/032). No ADR-0052 deferral needed.
-- ---------------------------------------------------------------------------
CREATE OR REPLACE FUNCTION cairn_execute_shred(p_target uuid, p_shred_event uuid, p_basis text)
RETURNS void LANGUAGE plpgsql SECURITY DEFINER SET search_path = public AS $$
DECLARE v_ca BYTEA;
BEGIN
    INSERT INTO erasure_shred_log (target_event_id, shred_event_id, basis)
    VALUES (p_target, p_shred_event, p_basis)
    ON CONFLICT (target_event_id) DO NOTHING;

    -- Provenance-precise projection scrub: only rows THIS event produced.
    -- (Overlay winners from other, unshredded events survive — never over-erase.)
    SELECT content_address INTO v_ca FROM event_log WHERE event_id = p_target;
    IF v_ca IS NOT NULL THEN
        DELETE FROM medication_statement WHERE content_address = v_ca;
        DELETE FROM medication_cessation WHERE content_address = v_ca;
    END IF;
    -- The initial-dose seed row is keyed by the assert event's own id (db/032).
    DELETE FROM medication_dose_event WHERE dose_event_id = p_target;

    -- Derived plaintext + custody die last (the scrub above read nothing from
    -- them, so order is safety, not correctness).
    DELETE FROM event_clear WHERE event_id = p_target;
    DELETE FROM event_dek   WHERE event_id = p_target;
END;
$$;

-- Rebuild the shred log from the append-only record on every load (idempotent):
-- this is "restore replays the shred log before projecting" for the wiped-and-
-- reloaded case.
INSERT INTO erasure_shred_log (target_event_id, shred_event_id, basis)
SELECT (body ->> 'target_event_id')::uuid, event_id, COALESCE(body ->> 'basis', '(unrecorded)')
FROM event_log WHERE event_type = 'erasure.shred.asserted'
ON CONFLICT (target_event_id) DO NOTHING;

-- ---------------------------------------------------------------------------
-- 8. Grant floor: door-managed only. SELECT for the serve/read paths. Both
--    cairn_agent (db/004) and cairn_node (db/007) are created unconditionally,
--    earlier in migration order, so no existence guard is needed here — mirrors
--    the unconditional REVOKE/GRANT style used from db/022 onward.
-- ---------------------------------------------------------------------------
REVOKE ALL ON node_unwrap_key, event_dek, event_clear, erasure_shred_log FROM PUBLIC;

REVOKE ALL ON node_unwrap_key, event_dek, event_clear, erasure_shred_log FROM cairn_agent;
GRANT SELECT ON event_clear TO cairn_agent;  -- the clear READ surface (chart/FTS)

GRANT SELECT ON event_dek, erasure_shred_log, node_unwrap_key TO cairn_node;  -- serve-side custody reads

-- Postgres grants EXECUTE on a new function to PUBLIC by default, and every role
-- (including cairn_agent) is a member of PUBLIC — an un-REVOKEd SECURITY DEFINER
-- function is therefore directly callable by a below-the-floor adversary with raw
-- SQL, bypassing submit_event/apply_remote_event entirely. Explicit REVOKE FROM
-- PUBLIC before each GRANT, mirroring db/007:270 and db/005:571-575.
REVOKE EXECUTE ON FUNCTION cairn_register_unwrap_key(bytea) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION cairn_register_unwrap_key(bytea) TO cairn_node;

-- cairn_execute_shred is called ONLY by the owner's own doors (submit_event's
-- erasure arm, apply_remote_event's erasure arm), which run SECURITY DEFINER as
-- the schema owner and so still reach it — no role needs a direct GRANT here.
REVOKE EXECUTE ON FUNCTION cairn_execute_shred(uuid, uuid, text) FROM PUBLIC;

COMMIT;
