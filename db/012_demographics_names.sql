-- Cairn — demographic NAMES: the retained-set + display-winner projection (spec §4.2).
--
-- Slice 3 of the demographics subsystem. Names are the first field that needs BOTH a
-- retained set (every name kept as matching evidence) AND a single display-winner
-- selected from it. A name reuses the slice-2 generic `demographic.field.asserted`
-- event with field='name'; the generic floor (db/011 cairn_check_demographic_field)
-- and the authored-twin enforcement already accept it, so this migration adds NO floor
-- change and NO new event type — only the projection. The display-winner is a VIEW
-- (a pure deterministic function of the set), so there is no winner-pointer to maintain.
-- Matching (§5.2) is a separate, later subsystem.

BEGIN;

-- The §4.2 retained set: one row per distinct (patient, use, value) name. use_key
-- folds an absent/blank `use` to 'unspecified' so it is a valid NOT-NULL key component
-- (mirrors patient_identifier.match_key), AND ASCII-lower-cases it (COLLATE "C"): `use`
-- is an OPEN, author-chosen vocabulary, so "Legal"/"LEGAL"/"legal" are the same category —
-- a deterministic fold makes the display-winner's legal-tier test (and member dedup)
-- case-insensitive AND convergent across nodes (a locale lower() is collation-dependent),
-- while use_raw below keeps the authored casing for legibility/audit. provenance_rank is
-- cached (reuses db/011's cairn_provenance_rank) so the recency/provenance test is a plain
-- tuple compare.
CREATE TABLE IF NOT EXISTS patient_name (
    patient_id         UUID    NOT NULL,
    use_key            TEXT    NOT NULL,   -- lower(coalesce(NULLIF(trim(use),''),'unspecified') COLLATE "C")
    value              TEXT    NOT NULL,   -- the authored display string (opaque to the core)
    use_raw            TEXT,               -- the original `use` facet (NULL when absent)
    provenance         TEXT    NOT NULL,
    provenance_rank    INT     NOT NULL,
    last_hlc_wall      BIGINT  NOT NULL,
    last_hlc_count     INTEGER NOT NULL,
    asserted_origin    TEXT    NOT NULL,
    updated_at         TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (patient_id, use_key, value)
);

-- Incremental maintenance: fold exactly the one new name event into the retained set.
-- event_log.body holds b->'payload' (see db/005 submit_event INSERT).
--
-- The per-type trigger is superseded by cairn_projection_dispatch_trg (db/005,
-- ADR-0057); this fn is now registered in cairn_projection_apply below.
DROP TRIGGER IF EXISTS patient_name_apply_trg ON event_log;
-- The old zero-arg trigger-function signature is superseded by the (event_log)
-- apply-fn signature below; CREATE OR REPLACE cannot change a function's arg
-- list (it would overload, not replace), so drop the old signature explicitly
-- (same idiom as db/005's `DROP FUNCTION IF EXISTS submit_event(bytea, bytea, bytea);`).
DROP FUNCTION IF EXISTS patient_name_apply();

CREATE OR REPLACE FUNCTION patient_name_apply(e event_log)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    p      jsonb := e.body;
    fld    text  := p ->> 'field';
    v_use  text  := NULLIF(trim(p -> 'facets' ->> 'use'), '');
    v_key  text;
    v_rank int;
BEGIN
    -- ADR-0052 §2 seal-robustness (#10): a wrongly-sealed NON-clinical row holds CIPHERTEXT
    -- in e.body (refused at submit; admitted lenient at apply for lossless sync). Reading it
    -- below would drive NULLs into this projection and freeze the sync watermark — so a sealed
    -- row projects NOTHING (harmless ciphertext noise; no custody, no leak).
    IF e.sealed THEN RETURN; END IF;
    -- Only NAME events project here. dob/sex-at-birth (db/011) and any unknown field
    -- are ignored — names get their own multi-valued shape. (This fn and the
    -- patient_demographic apply fn both dispatch on demographic.field.asserted; each
    -- gates to its own fields and writes a different table, so order is irrelevant.)
    IF fld <> 'name' THEN
        RETURN;
    END IF;
    -- Lower-case the key: `use` is open vocabulary, so "Legal"==="legal" as a category.
    -- Folding here makes the legal-tier display test case-insensitive and collapses
    -- casing variants of the same use to one member; use_raw keeps the authored form.
    -- COLLATE "C" forces a deterministic ASCII fold (A-Z→a-z, bytes ≥128 untouched): a
    -- locale-default lower() is collation-dependent (e.g. Turkic 'I'→dotless 'ı'), which
    -- would make two nodes compute a DIFFERENT use_key for the same event and diverge the
    -- retained-set PK across the fleet — federation must stay convergent (the legal token
    -- is ASCII, so C-folding loses nothing here).
    v_key  := lower(coalesce(v_use, 'unspecified') COLLATE "C");
    v_rank := cairn_provenance_rank(p ->> 'provenance');

    INSERT INTO patient_name AS pn
        (patient_id, use_key, value, use_raw, provenance, provenance_rank,
         last_hlc_wall, last_hlc_count, asserted_origin)
    VALUES
        (e.patient_id, v_key, p ->> 'value', v_use, p ->> 'provenance', v_rank,
         e.hlc_wall, e.hlc_counter, e.node_origin)
    -- Per (patient, use, value) member, keep the MOST-RECENT assertion as its
    -- representative (recency-first tuple, matching the display rule). The compare is a
    -- deterministic, apply-order-independent function of the member's assertion set, so
    -- every node converges to the same row. A re-assertion that does not advance the
    -- tuple leaves the row unchanged (set-union idempotency).
    ON CONFLICT (patient_id, use_key, value) DO UPDATE SET
        use_raw         = EXCLUDED.use_raw,
        provenance      = EXCLUDED.provenance,
        provenance_rank = EXCLUDED.provenance_rank,
        last_hlc_wall   = EXCLUDED.last_hlc_wall,
        last_hlc_count  = EXCLUDED.last_hlc_count,
        asserted_origin = EXCLUDED.asserted_origin,
        updated_at      = clock_timestamp()
    WHERE (EXCLUDED.last_hlc_wall, EXCLUDED.last_hlc_count,
           EXCLUDED.provenance_rank, EXCLUDED.asserted_origin COLLATE "C")
        > (pn.last_hlc_wall, pn.last_hlc_count,
           pn.provenance_rank, pn.asserted_origin COLLATE "C");
    RETURN;
END;
$$;

-- A trigger fn could never be called directly; a plain fn gets PUBLIC EXECUTE by
-- default. Same discipline as every privileged fn in db/005 (Task-1 review finding).
REVOKE EXECUTE ON FUNCTION patient_name_apply(event_log) FROM PUBLIC;

-- The §4.2 display-winner: one row per patient, selected from the retained set with NO
-- stored pointer. The ORDER BY is the whole rule:
--   1) prefer use_key='legal' (a legal name always outranks any non-legal — a 2010 legal
--      beats a 2024 alias). use_key is stored lower-cased, so this matches any authored
--      casing of the legal-use token ("Legal"/"LEGAL");
--   2) recency-first within the tier (newest legal name wins — recency beats provenance
--      for names, the deliberate divergence from DOB's provenance-lock);
--   3) provenance_rank then asserted_origin break exact-recency ties deterministically;
--   4) use_key then value are the FINAL total-order tiebreak. asserted_origin alone is
--      unique per event ONLY while every node stamps a distinct (wall,counter,origin) —
--      true for honest nodes, but a single buggy/hostile authoring node could mint two
--      events with an identical HLC tuple, leaving DISTINCT ON to pick arbitrarily and
--      two nodes to display DIFFERENT names from the SAME event set (a silent set-union
--      divergence in the field this project most obsesses about). Appending the retained
--      set's remaining PK columns (use_key, value) makes the order total over rows, so
--      the displayed name converges regardless of client HLC hygiene. These appended keys
--      are TEXT, so — like node_origin — they are collation-sensitive: two nodes running
--      with different default collations (e.g. "C" vs an ICU locale) could order the SAME
--      byte strings differently and diverge on the display winner. Per ADR-0045 (#69) this
--      is now fixed here: the trigger's ON CONFLICT WHERE tiebreak and this VIEW's ORDER BY
--      both pin every TEXT tiebreak key to COLLATE "C", so convergence holds regardless of
--      each node's default collation.
-- DRIFT WARNING (#159): this exact ORDER BY is DUPLICATED in db/025 — the identity-repudiate
--      migration CREATE-OR-REPLACEs patient_name_current to anti-join struck names, and (loading
--      LAST) ITS copy is the live one. The two clauses must stay byte-identical or the live db/025
--      view silently diverges from this template. Nothing in SQL enforces that: DISTINCT ON forces
--      each view to carry its own ORDER BY, and db/025 must anti-join BEFORE picking the winner, so
--      the ordering can't be factored into a shared base view. Lockstep is guarded by the no-DB
--      test crates/cairn-node/tests/name_winner_order_drift.rs — edit BOTH clauses together.
-- When no legal name exists, the newest name of ANY use wins (the unidentified-patient
-- fallback) — paper-parity: the chart header always shows something.
CREATE OR REPLACE VIEW patient_name_current AS
SELECT DISTINCT ON (patient_id)
    patient_id, use_key, value, use_raw, provenance, provenance_rank,
    last_hlc_wall, last_hlc_count, asserted_origin, updated_at
FROM patient_name
ORDER BY patient_id,
         (use_key = 'legal') DESC,
         last_hlc_wall DESC, last_hlc_count DESC,
         provenance_rank DESC, asserted_origin COLLATE "C" DESC,
         use_key COLLATE "C" DESC, value COLLATE "C" DESC;

GRANT SELECT ON patient_name, patient_name_current TO cairn_agent;

-- Registered apply fn for the #208/ADR-0057 generic dispatcher (db/005) + cairn_reproject
-- heal/rebuild (db/039). #214 + steady-state discipline: converge this row to the migration
-- text on every connect, but stay write-free once already converged (no dead tuples, no
-- validate-trigger fire).
INSERT INTO cairn_projection_apply AS r (event_type, apply_fn, projection_tables, run_order, heal_safe)
VALUES ('demographic.field.asserted', 'patient_name_apply', ARRAY['patient_name'], 30, TRUE)
ON CONFLICT (event_type, apply_fn) DO UPDATE SET
    projection_tables = EXCLUDED.projection_tables,
    run_order         = EXCLUDED.run_order,
    heal_safe         = EXCLUDED.heal_safe
WHERE (r.projection_tables, r.run_order, r.heal_safe)
      IS DISTINCT FROM (EXCLUDED.projection_tables, EXCLUDED.run_order, EXCLUDED.heal_safe);

COMMIT;
