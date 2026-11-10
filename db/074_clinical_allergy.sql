-- 074_clinical_allergy.sql — the allergy list (allergy slice 1; canonical home data-model §3.3):
-- `clinical.allergy.asserted` (one entry) and `clinical.allergy-none.asserted` ("none known").
--
-- WHAT IS ON THE WIRE. Two signed, born-sealed event types (ADR-0052: every `clinical.%` type is
-- sealed at write). The pure builders live in crates/cairn-event/src/allergy.rs.
--   clinical.allergy/1       {allergen_kind, substance: {term, coding}, kind,
--                             reaction: {category, text}, onset: {precision, value}, source, refs}
--   clinical.allergy-none/1  {scope}
-- The envelope's t_effective is unset on both: the reaction's time is `onset`, inside the body,
-- and "asked" is the event's own recording time (its HLC wall). No safety signal is emitted
-- (design §4: no supersession for a clear signal until #406/#401).
--
-- THE TWO TIERS (ADR-0051, the db/041 idiom). A STRUCTURAL rule — a key missing, the wrong JSON
-- type, a blank term, onset.value present when the precision says it is not known — refuses at
-- BOTH doors: no node can read such a body. A CLOSED-VOCABULARY rule — an enumerated word, the
-- shape of onset.value for its precision, an onset in the future, a coding on an entry that is
-- not a drug or in a system an allergen may not carry (allergy slice 3), a correction ref that is not exactly one `correction-of` naming an entry on this
-- chart (allergy slice 2) — refuses at the AUTHORING door only (`cairn.remote_apply` <> 'on'). At the
-- remote door a newer peer's word is ADMITTED and projected as itself: refusing it would wedge
-- sync on every node older than its author (ADR-0056). That is why the projection's vocabulary
-- columns carry no CHECK.
BEGIN;

-- 1. Register both types in the fail-closed classification registry: additive claims, never
--    targeting another author's event (a later correction is a NEW entry carrying a ref).
INSERT INTO event_type_class (event_type, mode, targets_other_author) VALUES
    ('clinical.allergy.asserted', 'additive', FALSE),
    ('clinical.allergy-none.asserted', 'additive', FALSE)
ON CONFLICT (event_type) DO NOTHING;

-- 2. The shape of a dated value for its precision, and "not after the recording day". Authoring
--    door only (the caller decides). Returns the refusal reason WITHOUT the field name, or NULL
--    when the date is fine, so each caller keeps one RAISE per rule and names its own field: the
--    entry's `onset.value`, and a de-label's `date.value` (db/075, the same shape — ADR-0096's
--    onset). Pure apart from now().
--
--    WHY "+14 hours": the recording day is the clinician's calendar day, and this database does
--    not know the terminal's time zone. UTC+14 is the furthest-ahead zone on earth, so a date
--    after THAT day is in the future everywhere — refusing only those never refuses a clinician
--    who typed today's date before the database's own day caught up.
CREATE OR REPLACE FUNCTION cairn_allergy_onset_problem(p_precision text, p_value text)
RETURNS text LANGUAGE plpgsql AS $$
DECLARE
    v_latest   text := to_char(now() AT TIME ZONE 'UTC' + interval '14 hours', 'YYYY-MM-DD');
    v_shape_ok boolean;
BEGIN
    IF p_precision = 'not-known' THEN
        RETURN NULL;
    END IF;
    -- (A CASE's own THEN would end an IF condition early in PL/pgSQL, so it is assigned first.)
    v_shape_ok := CASE p_precision
        WHEN 'day'   THEN p_value ~ '^\d{4}-\d{2}-\d{2}$' AND pg_input_is_valid(p_value, 'date')
        WHEN 'month' THEN p_value ~ '^\d{4}-(0[1-9]|1[0-2])$'
        WHEN 'year'  THEN p_value ~ '^\d{4}$'
    END;
    IF v_shape_ok IS NOT TRUE THEN
        RETURN format('"%s" is not a valid %s (YYYY-MM-DD, YYYY-MM or YYYY)', p_value, p_precision);
    END IF;
    -- The shape admits 0000 at year and month precision; no calendar has that year (a day is
    -- already refused above: pg_input_is_valid knows no year 0). The window refuses it too.
    IF left(p_value, 4) = '0000' THEN
        RETURN format('"%s" has year 0000, which no calendar has', p_value);
    END IF;
    -- ISO forms compare as text; cut today to the value's own length (year, month or day).
    IF p_value > left(v_latest, length(p_value)) THEN
        RETURN format('"%s" is in the future (after the recording day)', p_value);
    END IF;
    RETURN NULL;
END;
$$;
REVOKE EXECUTE ON FUNCTION cairn_allergy_onset_problem(text, text) FROM PUBLIC;

-- 3. The entry floor. Each rule its own message, structural rules first (both doors), then the
--    closed vocabularies (authoring door only).
CREATE OR REPLACE FUNCTION cairn_check_allergy_assertion(p_type text, b jsonb)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    p           jsonb   := b -> 'payload';
    v_authoring boolean := current_setting('cairn.remote_apply', true) IS DISTINCT FROM 'on';
    v_onset_why text;
    v_ref       jsonb;    -- a correction's one ref (authoring door)
    v_ref_ok    boolean;
BEGIN
    -- ---- Structural: both doors ----
    IF jsonb_typeof(p) IS DISTINCT FROM 'object' THEN
        RAISE EXCEPTION 'allergy assertion: missing payload';
    END IF;
    IF jsonb_typeof(p -> 'allergen_kind') IS DISTINCT FROM 'string' THEN
        RAISE EXCEPTION 'allergy assertion: allergen_kind must be a string';
    END IF;
    IF jsonb_typeof(p -> 'substance') IS DISTINCT FROM 'object' THEN
        RAISE EXCEPTION 'allergy assertion: substance must be an object {term, coding}';
    END IF;
    -- "Blank" is the twin's own Unicode-exact test (db/005), so nodes on differently-collated
    -- databases agree on the same signed bytes; it is btrim, widened to every Unicode space.
    IF jsonb_typeof(p #> '{substance,term}') IS DISTINCT FROM 'string'
       OR NOT cairn_twin_is_present(p #>> '{substance,term}') THEN
        RAISE EXCEPTION 'allergy assertion: substance.term must be a non-blank string';
    END IF;
    IF NOT (p -> 'substance' ? 'coding') THEN
        RAISE EXCEPTION 'allergy assertion: substance.coding must be present (null when uncoded)';
    END IF;
    -- One definition of a valid coding object (db/041, ADR-0059): null passes; an object needs
    -- three non-empty strings. At the authoring door it also runs the coding-system registry tier.
    PERFORM cairn_check_coding_object(p #> '{substance,coding}', 'allergy assertion: substance.coding');
    IF jsonb_typeof(p -> 'kind') IS DISTINCT FROM 'string' THEN
        RAISE EXCEPTION 'allergy assertion: kind must be a string';
    END IF;
    IF jsonb_typeof(p -> 'reaction') IS DISTINCT FROM 'object' THEN
        RAISE EXCEPTION 'allergy assertion: reaction must be an object {category, text}';
    END IF;
    IF jsonb_typeof(p #> '{reaction,category}') IS DISTINCT FROM 'string' THEN
        RAISE EXCEPTION 'allergy assertion: reaction.category must be a string';
    END IF;
    IF jsonb_typeof(p #> '{reaction,text}') IS NULL
       OR jsonb_typeof(p #> '{reaction,text}') NOT IN ('string', 'null') THEN
        RAISE EXCEPTION 'allergy assertion: reaction.text must be present, a string or null';
    END IF;
    IF jsonb_typeof(p -> 'onset') IS DISTINCT FROM 'object' THEN
        RAISE EXCEPTION 'allergy assertion: onset must be an object {precision, value}';
    END IF;
    IF jsonb_typeof(p #> '{onset,precision}') IS DISTINCT FROM 'string' THEN
        RAISE EXCEPTION 'allergy assertion: onset.precision must be a string';
    END IF;
    -- Principle 4: "not known" is an answer, and it carries no value — exactly then.
    IF jsonb_typeof(p #> '{onset,value}') IS DISTINCT FROM
       (CASE WHEN p #>> '{onset,precision}' = 'not-known' THEN 'null' ELSE 'string' END) THEN
        RAISE EXCEPTION 'allergy assertion: onset.value must be null exactly when onset.precision is not-known, else a string';
    END IF;
    IF jsonb_typeof(p -> 'source') IS DISTINCT FROM 'string' THEN
        RAISE EXCEPTION 'allergy assertion: source must be a string';
    END IF;
    IF jsonb_typeof(p -> 'refs') IS DISTINCT FROM 'array' THEN
        RAISE EXCEPTION 'allergy assertion: refs must be an array';
    END IF;

    -- ---- Closed vocabularies: the authoring door only (ADR-0056) ----
    IF NOT v_authoring THEN
        RETURN;
    END IF;
    -- The schema tag is checked at the authoring door ONLY, and the appliers project a peer's /2
    -- under /1's structure — a deliberate departure from db/064's "project /1 only" (ADR-0096
    -- decision 8): allergy text is never rendered as markup, so there is no unknown-format risk
    -- to fence; an additive /2 keeps /1's keys and types (principle 11), which the structural
    -- tier above still enforces at both doors; and a projected row errs toward over-warning,
    -- where an unprojected /2 allergy would vanish from the strip. (A /2 that CHANGED a /1 key's
    -- type would be refused structurally and stall sync — additive-only evolution forbids it.)
    IF b ->> 'schema_version' IS DISTINCT FROM 'clinical.allergy/1' THEN
        RAISE EXCEPTION 'allergy assertion: schema_version must be clinical.allergy/1, got %', b ->> 'schema_version';
    END IF;
    IF p ->> 'allergen_kind' NOT IN ('drug', 'other') THEN
        RAISE EXCEPTION 'allergy assertion: allergen_kind must be drug or other, got %', p ->> 'allergen_kind';
    END IF;
    IF p ->> 'kind' NOT IN ('allergy', 'adverse-effect') THEN
        RAISE EXCEPTION 'allergy assertion: kind must be allergy or adverse-effect, got %', p ->> 'kind';
    END IF;
    IF p #>> '{reaction,category}' NOT IN
       ('anaphylaxis', 'angioedema', 'urticaria', 'rash', 'gi-upset', 'other', 'not-known') THEN
        RAISE EXCEPTION 'allergy assertion: reaction.category % is not in this node''s vocabulary', p #>> '{reaction,category}';
    END IF;
    IF p #>> '{onset,precision}' NOT IN ('day', 'month', 'year', 'not-known') THEN
        RAISE EXCEPTION 'allergy assertion: onset.precision must be day, month, year or not-known, got %', p #>> '{onset,precision}';
    END IF;
    v_onset_why := cairn_allergy_onset_problem(p #>> '{onset,precision}', p #>> '{onset,value}');
    IF v_onset_why IS NOT NULL THEN
        RAISE EXCEPTION 'allergy assertion: onset.value %', v_onset_why;
    END IF;
    IF p ->> 'source' NOT IN ('patient-stated', 'seen-documented', 'another-record') THEN
        RAISE EXCEPTION 'allergy assertion: source must be patient-stated, seen-documented or another-record, got %', p ->> 'source';
    END IF;
    -- The coding slot (allergy slice 3; D1, M2). Only a DRUG entry is coded, and only as one drug
    -- (drugref-moiety) or as a drug group (drugref-class) — the two codes the medication check can
    -- use. The object's shape, the system being registered and the code being a canonical uuid
    -- were checked above by cairn_check_coding_object; this narrows the registered systems to the
    -- two an allergen may carry (drugref-product, say, is registered for medications only).
    -- Authoring door only: a newer peer may code an allergen in a way this node does not know.
    IF jsonb_typeof(p #> '{substance,coding}') = 'object' THEN
        IF p ->> 'allergen_kind' IS DISTINCT FROM 'drug' THEN
            RAISE EXCEPTION 'allergy assertion: substance.coding is for a drug entry only';
        END IF;
        IF p #>> '{substance,coding,system}' NOT IN ('drugref-moiety', 'drugref-class') THEN
            RAISE EXCEPTION 'allergy assertion: substance.coding.system must be drugref-moiety or drugref-class, got %',
                p #>> '{substance,coding,system}';
        END IF;
    END IF;
    -- A CORRECTION (allergy slice 2): refs is [] or exactly one {rel: correction-of, event}
    -- naming the line it replaces (plan M2: a chain, not the root). The target must be an entry
    -- already held here, on THIS chart (D2, the vouch precedent: a correction filed under the
    -- wrong patient is the #689 failure). Authoring door only (D6): a peer may correct a line
    -- this node has not received yet, and a ref the reader cannot parse is ignored (over-warns).
    IF p -> 'refs' <> '[]'::jsonb THEN
        v_ref := p #> '{refs,0}';
        -- A CASE, not one long OR: `-` on a non-object raises, and SQL never promises the order
        -- an OR is evaluated in; a CASE's branch runs only when its test held.
        v_ref_ok := CASE WHEN jsonb_array_length(p -> 'refs') = 1 AND jsonb_typeof(v_ref) = 'object'
            THEN v_ref - 'rel' - 'event' = '{}'::jsonb          -- exactly those two keys
                 AND v_ref ->> 'rel' = 'correction-of'
                 AND jsonb_typeof(v_ref -> 'event') = 'string'
                 AND pg_input_is_valid(v_ref ->> 'event', 'uuid')
            ELSE false END;
        IF v_ref_ok IS NOT TRUE THEN
            RAISE EXCEPTION 'allergy assertion: refs must be [] or one {rel: correction-of, event}';
        END IF;
        IF NOT EXISTS (SELECT 1 FROM event_log
                        WHERE event_id = (v_ref ->> 'event')::uuid
                          AND event_type = 'clinical.allergy.asserted'
                          AND patient_id = (b ->> 'patient_id')::uuid) THEN
            RAISE EXCEPTION 'allergy assertion: correction-of target % is not an allergy entry on this chart',
                v_ref ->> 'event';
        END IF;
    END IF;
END;
$$;
-- The cairn_check_* family is revoked uniformly (#382; db/005).
REVOKE EXECUTE ON FUNCTION cairn_check_allergy_assertion(text, jsonb) FROM PUBLIC;

-- 4. The "none known" floor: scope is a string at both doors, drug | all at the authoring door.
CREATE OR REPLACE FUNCTION cairn_check_allergy_none_assertion(p_type text, b jsonb)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    p jsonb := b -> 'payload';
BEGIN
    IF jsonb_typeof(p) IS DISTINCT FROM 'object' THEN
        RAISE EXCEPTION 'allergy-none assertion: missing payload';
    END IF;
    IF jsonb_typeof(p -> 'scope') IS DISTINCT FROM 'string' THEN
        RAISE EXCEPTION 'allergy-none assertion: scope must be a string';
    END IF;
    IF current_setting('cairn.remote_apply', true) = 'on' THEN
        RETURN;
    END IF;
    IF b ->> 'schema_version' IS DISTINCT FROM 'clinical.allergy-none/1' THEN
        RAISE EXCEPTION 'allergy-none assertion: schema_version must be clinical.allergy-none/1, got %', b ->> 'schema_version';
    END IF;
    IF p ->> 'scope' NOT IN ('drug', 'all') THEN
        RAISE EXCEPTION 'allergy-none assertion: scope must be drug or all, got %', p ->> 'scope';
    END IF;
END;
$$;
REVOKE EXECUTE ON FUNCTION cairn_check_allergy_none_assertion(text, jsonb) FROM PUBLIC;

-- 5. Register both floors + the hard twin requirement (#173). DO UPDATE, not DO NOTHING (#214):
--    the loader replays this file, so the rows CONVERGE to the migration text; the IS DISTINCT
--    FROM guard keeps the steady-state replay write-free.
INSERT INTO cairn_event_twin_check (event_type, check_fn, twin_required_msg) VALUES
    ('clinical.allergy.asserted', 'cairn_check_allergy_assertion',
     'allergy assertion requires a non-empty authored twin (§3.13/§3.3)'),
    ('clinical.allergy-none.asserted', 'cairn_check_allergy_none_assertion',
     'allergy-none assertion requires a non-empty authored twin (§3.13/§3.3)')
ON CONFLICT (event_type) DO UPDATE SET
    check_fn          = EXCLUDED.check_fn,
    twin_required_msg = EXCLUDED.twin_required_msg
WHERE (cairn_event_twin_check.check_fn, cairn_event_twin_check.twin_required_msg)
      IS DISTINCT FROM (EXCLUDED.check_fn, EXCLUDED.twin_required_msg);

-- 6. The projections: one row per event, keyed by the event itself. An entry is never overlaid
--    in place (a correction is a NEW event carrying a ref), so a replay converges by ON CONFLICT
--    DO NOTHING. content_address is what a shred scrubs by (db/037); patient_id is how a chart
--    reads its list. These columns are final (a widened CREATE TABLE never reaches an existing
--    table on replay).
CREATE TABLE IF NOT EXISTS clinical_allergy (
    event_id          UUID PRIMARY KEY,
    patient_id        UUID NOT NULL,
    allergen_kind     TEXT,
    term              TEXT,
    coding            JSONB,
    kind              TEXT,
    reaction_category TEXT,
    reaction_text     TEXT,
    onset_precision   TEXT,
    onset_value       TEXT,
    source            TEXT,
    refs              JSONB NOT NULL,
    hlc_wall          BIGINT,
    hlc_counter       INTEGER,
    origin            TEXT,
    content_address   BYTEA NOT NULL
);
CREATE TABLE IF NOT EXISTS clinical_allergy_none (
    event_id        UUID PRIMARY KEY,
    patient_id      UUID NOT NULL,
    scope           TEXT NOT NULL,
    hlc_wall        BIGINT,
    hlc_counter     INTEGER,
    origin          TEXT,
    content_address BYTEA NOT NULL
);
-- The note/medication posture: the runtime role reads its projections.
GRANT SELECT ON clinical_allergy, clinical_allergy_none TO cairn_agent;
CREATE INDEX IF NOT EXISTS clinical_allergy_patient_idx ON clinical_allergy (patient_id);
CREATE INDEX IF NOT EXISTS clinical_allergy_content_address_idx ON clinical_allergy (content_address);
CREATE INDEX IF NOT EXISTS clinical_allergy_none_patient_idx ON clinical_allergy_none (patient_id);
CREATE INDEX IF NOT EXISTS clinical_allergy_none_content_address_idx ON clinical_allergy_none (content_address);

-- 7. The appliers. Each reads the CLEAR payload (ADR-0052): NULL means this node holds no custody
--    for the sealed body — nothing to project, honest degradation; the late-custody path re-runs
--    it when a key lands (heal_safe below). An uncoded substance stores SQL NULL, not JSON null.
--    No schema_version filter here, unlike db/064's notes: a peer's /2 projects under /1's keys
--    (why: the comment at step 3's schema_version check; ADR-0096 decision 8).
CREATE OR REPLACE FUNCTION clinical_allergy_apply(e event_log)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    p jsonb := cairn_clear_payload(e);
BEGIN
    IF p IS NULL THEN RETURN; END IF;
    INSERT INTO clinical_allergy
        (event_id, patient_id, allergen_kind, term, coding, kind, reaction_category, reaction_text,
         onset_precision, onset_value, source, refs, hlc_wall, hlc_counter, origin, content_address)
    VALUES (
        e.event_id, e.patient_id,
        p ->> 'allergen_kind',
        p #>> '{substance,term}',
        NULLIF(p #> '{substance,coding}', 'null'::jsonb),
        p ->> 'kind',
        p #>> '{reaction,category}',
        p #>> '{reaction,text}',
        p #>> '{onset,precision}',
        p #>> '{onset,value}',
        p ->> 'source',
        p -> 'refs',
        e.hlc_wall, e.hlc_counter, e.node_origin, e.content_address)
    ON CONFLICT (event_id) DO NOTHING;
END;
$$;
-- An applier WRITES a projection; callable by PUBLIC it would let any role forge rows.
REVOKE EXECUTE ON FUNCTION clinical_allergy_apply(event_log) FROM PUBLIC;

CREATE OR REPLACE FUNCTION clinical_allergy_none_apply(e event_log)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    p jsonb := cairn_clear_payload(e);
BEGIN
    IF p IS NULL THEN RETURN; END IF;
    INSERT INTO clinical_allergy_none
        (event_id, patient_id, scope, hlc_wall, hlc_counter, origin, content_address)
    VALUES (e.event_id, e.patient_id, p ->> 'scope',
            e.hlc_wall, e.hlc_counter, e.node_origin, e.content_address)
    ON CONFLICT (event_id) DO NOTHING;
END;
$$;
REVOKE EXECUTE ON FUNCTION clinical_allergy_none_apply(event_log) FROM PUBLIC;

-- 8. Register the appliers with the generic dispatcher (db/005, ADR-0057) and cairn_reproject.
--    heal_safe: one row per event, insert-or-nothing, so re-running is a no-op. #214 idiom.
INSERT INTO cairn_projection_apply AS r (event_type, apply_fn, projection_tables, run_order, heal_safe) VALUES
    ('clinical.allergy.asserted', 'clinical_allergy_apply', ARRAY['clinical_allergy'], 20, TRUE),
    ('clinical.allergy-none.asserted', 'clinical_allergy_none_apply', ARRAY['clinical_allergy_none'], 20, TRUE)
ON CONFLICT (event_type, apply_fn) DO UPDATE SET
    projection_tables = EXCLUDED.projection_tables,
    run_order         = EXCLUDED.run_order,
    heal_safe         = EXCLUDED.heal_safe
WHERE (r.projection_tables, r.run_order, r.heal_safe)
      IS DISTINCT FROM (EXCLUDED.projection_tables, EXCLUDED.run_order, EXCLUDED.heal_safe);

COMMIT;
