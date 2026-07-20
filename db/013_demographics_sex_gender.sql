-- Cairn — demographic sex/gender projection policy: administrative-sex + gender-identity
-- (spec §4.2). Slice 4 of the demographics subsystem.
--
-- Adds the other two of the three §4.2 sex/gender fields on the SAME
-- demographic.field.asserted spine (db/011): no new event type, no new door, no floor
-- change (both values are OPEN strings — principle 4). The one new mechanic is a
-- PER-FIELD WINNER POLICY: gender-identity is recency-first (newest wins regardless of
-- provenance — the inverse of slice-2's provenance-first ordering), while
-- administrative-sex joins dob/sex-at-birth as provenance-first (a document-anchored
-- marker an unverified claim must not displace). A single IMMUTABLE classifier is the
-- source of truth for BOTH the projection gate and the winner ordering, so every node
-- converges identically. Matching (§5.2) is a separate, later subsystem.

BEGIN;

-- The per-field winner policy (spec §4.2). Source of truth for the projection: it gates
-- which fields project (NULL => the field is carried in event_log + legible via its twin
-- but never projected — the ADR-0012 federation-forward degrade for a field this node
-- does not recognise) AND selects the winner ordering. IMMUTABLE so it is trigger-safe
-- and every node computes the identical policy. Names (field='name') are deliberately
-- ABSENT — they project through their own db/012 retained-set table, not here.
CREATE OR REPLACE FUNCTION cairn_demographic_field_policy(p_field text)
RETURNS text LANGUAGE sql IMMUTABLE AS $$
    SELECT CASE p_field
        WHEN 'dob'                THEN 'provenance-first'
        WHEN 'sex-at-birth'       THEN 'provenance-first'
        WHEN 'administrative-sex' THEN 'provenance-first'
        WHEN 'gender-identity'    THEN 'recency-first'
        ELSE NULL
    END;
$$;

-- The §4.2 projection, now policy-driven. Supersedes db/011's definition (standard
-- latest-loaded-wins additive migration); db/012/names is untouched (it projects through
-- patient_name, not here). One row per (patient, field) holds the current DISPLAY winner;
-- full assertion history stays in event_log as the matching evidence (principle 2 — an
-- overlay, never an edit). event_log.body holds b->'payload' (see db/005 submit_event).
-- No DROP TRIGGER/DROP FUNCTION pair here: db/011 (the first file to define this fn) already
-- owns them, and CREATE OR REPLACE below reuses the SAME (event_log)-signature object db/011
-- created — including its REVOKE EXECUTE, which CREATE OR REPLACE preserves across a body
-- swap (same OID, same ACL). This file only redefines the policy-driven body.
CREATE OR REPLACE FUNCTION patient_demographic_apply(e event_log)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    p      jsonb := e.body;
    fld    text  := p ->> 'field';
    v_rank int   := cairn_provenance_rank(p ->> 'provenance');
    policy text  := cairn_demographic_field_policy(fld);
BEGIN
    -- ADR-0052 §2 seal-robustness (#10): a wrongly-sealed NON-clinical row holds CIPHERTEXT
    -- in e.body (refused at submit; admitted lenient at apply for lossless sync). Reading it
    -- below would drive NULLs into this projection and freeze the sync watermark — so a sealed
    -- row projects NOTHING (harmless ciphertext noise; no custody, no leak).
    IF e.sealed THEN RETURN; END IF;
    -- Projection gate: a field with no winner policy is not projected (it is still in
    -- event_log and legible via its twin). Replaces slice-2's hard-coded field list.
    IF policy IS NULL THEN
        RETURN;
    END IF;

    INSERT INTO patient_demographic AS pd
        (patient_id, field, value, facets, provenance, provenance_rank,
         asserted_hlc_wall, asserted_hlc_count, asserted_origin, content_address)
    VALUES
        (e.patient_id, fld, p ->> 'value', p -> 'facets', p ->> 'provenance', v_rank,
         e.hlc_wall, e.hlc_counter, e.node_origin, e.content_address)
    -- Winner ordering by policy. BOTH tuples are TOTAL orders (node_origin is the final
    -- deterministic tiebreak), so every node converges to the same winner regardless of
    -- apply order.
    --   provenance-first: rank leads -> a verified value LOCKS vs lower provenance,
    --     recency breaks equal-provenance ties (dob, sex-at-birth, administrative-sex).
    --   recency-first:    HLC leads  -> newest wins REGARDLESS of provenance, provenance
    --     then origin break equal-HLC ties (gender-identity).
    -- pd.field == EXCLUDED.field (the PK), so the policy is identical on both sides.
    ON CONFLICT (patient_id, field) DO UPDATE SET
        value              = EXCLUDED.value,
        facets             = EXCLUDED.facets,
        provenance         = EXCLUDED.provenance,
        provenance_rank    = EXCLUDED.provenance_rank,
        asserted_hlc_wall  = EXCLUDED.asserted_hlc_wall,
        asserted_hlc_count = EXCLUDED.asserted_hlc_count,
        asserted_origin    = EXCLUDED.asserted_origin,
        content_address    = EXCLUDED.content_address,
        updated_at         = clock_timestamp()
    -- COLLATE "C" tiebreak per ADR-0045 (#69): the origin/value keys must order by raw byte
    -- value, not the database's default (possibly locale-dependent, e.g. ICU) collation —
    -- else two nodes with different locale settings could pick different winners for the
    -- same tied event pair, breaking cross-node convergence. Applies in both policy branches.
    -- content_address is the FINAL tiebreak (#194/finding A6): `value` alone was not a
    -- total order — two events sharing (triple, value) but differing in facets compared
    -- equal in both directions, so first-applied won and two honest nodes could diverge in
    -- exactly the columns the db/016 veto floor reads (facets ->> 'precision',
    -- provenance_rank). bytea byte-order, unique per distinct event; a NULL incumbent
    -- (pre-#194 row) keeps the incumbent — legacy ties only, every new write stamps it.
    WHERE CASE cairn_demographic_field_policy(pd.field)
        WHEN 'recency-first' THEN
            (EXCLUDED.asserted_hlc_wall, EXCLUDED.asserted_hlc_count,
             EXCLUDED.provenance_rank, EXCLUDED.asserted_origin COLLATE "C", EXCLUDED.value COLLATE "C",
             EXCLUDED.content_address)
          > (pd.asserted_hlc_wall, pd.asserted_hlc_count,
             pd.provenance_rank, pd.asserted_origin COLLATE "C", pd.value COLLATE "C",
             pd.content_address)
        ELSE
            (EXCLUDED.provenance_rank, EXCLUDED.asserted_hlc_wall,
             EXCLUDED.asserted_hlc_count, EXCLUDED.asserted_origin COLLATE "C", EXCLUDED.value COLLATE "C",
             EXCLUDED.content_address)
          > (pd.provenance_rank, pd.asserted_hlc_wall,
             pd.asserted_hlc_count, pd.asserted_origin COLLATE "C", pd.value COLLATE "C",
             pd.content_address)
    END;
    RETURN;
END;
$$;

-- Defensive tombstone only: db/011 already drops this trigger (the per-type trigger is
-- superseded by cairn_projection_dispatch_trg, db/005, ADR-0057); kept here so an
-- upgraded-in-place database that somehow still carries this trigger name sheds it
-- regardless of which of db/011/db/013 it last loaded. The CREATE TRIGGER that used to
-- follow is gone — dispatch is now the single db/005 dispatcher reading
-- cairn_projection_apply (db/011's registration row), not a per-type trigger.
DROP TRIGGER IF EXISTS patient_demographic_apply_trg ON event_log;

-- The one-time carried-not-projected catch-up that lived here (a bespoke
-- cairn_demographic_backfill(), run on EVERY connect) is retired by ADR-0057:
-- the generic cairn_reproject (db/039) replays through the SAME apply fn the
-- trigger uses (one winner-logic implementation, zero drift), and the loader
-- runs it exactly when new projection capability can arrive — on a schema-
-- generation change — instead of on every connect (#208). The DROP below is
-- the tombstone that sheds the fn from upgraded-in-place databases.
DROP FUNCTION IF EXISTS cairn_demographic_backfill();

COMMIT;
