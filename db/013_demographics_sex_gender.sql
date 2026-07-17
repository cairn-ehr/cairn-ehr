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
CREATE OR REPLACE FUNCTION patient_demographic_apply()
RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE
    p      jsonb := NEW.body;
    fld    text  := p ->> 'field';
    v_rank int   := cairn_provenance_rank(p ->> 'provenance');
    policy text  := cairn_demographic_field_policy(fld);
BEGIN
    -- ADR-0052 §2 seal-robustness (#10): a wrongly-sealed NON-clinical row holds CIPHERTEXT
    -- in NEW.body (refused at submit; admitted lenient at apply for lossless sync). Reading it
    -- below would drive NULLs into this projection and freeze the sync watermark — so a sealed
    -- row projects NOTHING (harmless ciphertext noise; no custody, no leak).
    IF NEW.sealed THEN RETURN NULL; END IF;
    -- Projection gate: a field with no winner policy is not projected (it is still in
    -- event_log and legible via its twin). Replaces slice-2's hard-coded field list.
    IF policy IS NULL THEN
        RETURN NULL;
    END IF;

    INSERT INTO patient_demographic AS pd
        (patient_id, field, value, facets, provenance, provenance_rank,
         asserted_hlc_wall, asserted_hlc_count, asserted_origin, content_address)
    VALUES
        (NEW.patient_id, fld, p ->> 'value', p -> 'facets', p ->> 'provenance', v_rank,
         NEW.hlc_wall, NEW.hlc_counter, NEW.node_origin, NEW.content_address)
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
    RETURN NULL;
END;
$$;

-- The trigger binding is unchanged from db/011 (same WHEN, same function name); only the
-- function body above changed. Re-create defensively so a fresh load is order-independent.
DROP TRIGGER IF EXISTS patient_demographic_apply_trg ON event_log;
CREATE TRIGGER patient_demographic_apply_trg
    AFTER INSERT ON event_log
    FOR EACH ROW WHEN (NEW.event_type = 'demographic.field.asserted')
    EXECUTE FUNCTION patient_demographic_apply();

-- One-time catch-up for events ALREADY in event_log when this node gains projection
-- capability for a field. The apply trigger only fires for NEW inserts, so a field that
-- was *carried-not-projected* under an earlier schema (db/011 recognised only
-- dob/sex-at-birth; a federated node can already hold gender-identity/administrative-sex
-- assertions synced forward from a newer peer — exactly the ADR-0012 federation degrade)
-- would otherwise stay invisible in patient_demographic until the next fresh assertion
-- happened to arrive. That silently breaks the ADR-0012 promise of "carry now, project
-- once the node understands the field". This re-folds the retained set so the projection
-- catches up on the load that introduces the policy.
--
-- It is a pure function of event_log + the (immutable) policy, so it is idempotent and
-- convergent: it recomputes the SAME policy-correct winner every node would. The
-- ON CONFLICT guard is the SAME winner comparison as the trigger, so an already-correct
-- projection row incurs no write (cheap on every reload — connect_and_load_schema replays
-- every file), a missing row is inserted, and a stale row is healed. Never a downgrade.
CREATE OR REPLACE FUNCTION cairn_demographic_backfill()
RETURNS void LANGUAGE sql AS $$
    INSERT INTO patient_demographic AS pd
        (patient_id, field, value, facets, provenance, provenance_rank,
         asserted_hlc_wall, asserted_hlc_count, asserted_origin, content_address)
    SELECT DISTINCT ON (patient_id, field)
        patient_id, field, value, facets, provenance, provenance_rank,
        hlc_wall, hlc_counter, node_origin, content_address
    FROM (
        SELECT
            e.patient_id,
            e.body ->> 'field'                                 AS field,
            e.body ->> 'value'                                 AS value,
            e.body -> 'facets'                                 AS facets,
            e.body ->> 'provenance'                            AS provenance,
            cairn_provenance_rank(e.body ->> 'provenance')     AS provenance_rank,
            e.hlc_wall, e.hlc_counter, e.node_origin, e.content_address,
            cairn_demographic_field_policy(e.body ->> 'field') AS policy
        FROM event_log e
        WHERE e.event_type = 'demographic.field.asserted'
          -- only fields this node now projects; carried-not-projected stays carried.
          AND cairn_demographic_field_policy(e.body ->> 'field') IS NOT NULL
    ) s
    -- DISTINCT ON keeps the policy-winner per (patient, field): the SAME tuple order as
    -- the trigger, expressed as a per-policy sort. recency-first leads with HLC;
    -- provenance-first leads with rank; node_origin is the final deterministic tiebreak.
    ORDER BY patient_id, field,
        (CASE WHEN policy = 'recency-first' THEN hlc_wall        ELSE provenance_rank END) DESC,
        (CASE WHEN policy = 'recency-first' THEN hlc_counter     ELSE hlc_wall END)        DESC,
        (CASE WHEN policy = 'recency-first' THEN provenance_rank ELSE hlc_counter END)     DESC,
        -- COLLATE "C" per ADR-0045 (#69): must match the trigger's byte-order tiebreak
        -- (below) so DISTINCT ON picks the SAME row the trigger would have converged on.
        node_origin COLLATE "C" DESC,
        -- `value` then `content_address` are the final total-order tiebreaks (see 011 /
        -- #194): guarantee a display-convergent winner even if a buggy node minted a
        -- duplicate HLC tuple, including two events sharing (triple, value).
        value COLLATE "C" DESC,
        content_address DESC
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
    -- COLLATE "C" tiebreak per ADR-0045 (#69) — same rationale as the trigger above: the
    -- origin/value keys must order by raw byte value so this one-time catch-up converges
    -- on the identical winner the trigger would have picked, regardless of node locale.
    -- content_address is the final tiebreak (#194), same as the trigger.
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
$$;

SELECT cairn_demographic_backfill();

COMMIT;
