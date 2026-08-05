-- Cairn — patient registration (spec §5.3/§5.8, ADR-0060; issue #344).
--
-- The act that brings a patient chart into being, and the in-DB floor beneath it.
--
-- # Why registration is an event at all
--
-- Before this migration a standard chart came into being as a SIDE EFFECT of whatever
-- event happened to carry its patient_id first. §5.8 requires the create act to record
-- that N near-matches were displayed to the person who chose to create anyway — and a
-- side effect has nowhere to record anything. So registration becomes an act, with
-- §5.3's three classes (standard / unidentified / pseudonymous) as one discriminant so
-- the precedence rule never needs an exception.
--
-- # Why this file is the safety-critical half of the slice
--
-- `cairn_event::registration::RegistrationAssertion` (the Rust wire type) deliberately
-- PERMITS illegal states: `Standard` with no search, `Unidentified` WITH a search. That
-- is the twelfth founding principle applied — the compatibility floor is enforced
-- unbypassably in the DATABASE, not in one client's types, so a bespoke UI talking raw
-- SQL cannot admit a malformed registration either. This file is the only thing standing
-- between those illegal states and the permanent record, and a defect here admits a
-- malformed clinical record FOREVER (append-only: there is no UPDATE to fix it with).
-- Hence: one distinct, legible exception per rule, and both directions of every rule
-- covered by tests (crates/cairn-node/tests/patient_registration.rs, and the SQL mirror
-- db/tests/045_patient_registration_test.sql).
--
-- # What is deliberately NOT here
--
--   * The PRECEDENCE rule (`cairn_patient_has_events` and the db/005 call site that
--     would refuse clinical content on a chart with no registration) belongs to issue
--     #345. Adding it here would change the admission contract for ~83 existing call
--     sites in one commit; it gets its own slice.
--   * Any AUTHORSHIP requirement. A standard registration with NO human author is
--     ACCEPTED — spec §2.6: authorship confidence is a GRADE, not a gate (§5.11). A gate
--     here would block care documentation at 03:00 when a clerk's key is not unlocked,
--     push named patients through the John Doe path, and leave no forensic record in the
--     case it fires. Naming a registrar who did not authenticate is already refused for
--     free by db/005's UNCONDITIONAL `cairn_authorship_bound` (step 4b) — this file adds
--     no rule and needs none.

BEGIN;

-- 1. Additive registration of the new event type (fail-closed registry, ADR-0010: an
--    unclassified type is refused at both doors, so this row is what makes the verb
--    exist at all). `targets_other_author` is FALSE: a registration ADDS a chart-birth
--    claim, it never forecloses on another author's event, so it must not be routed
--    through the ADR-0043 suppression owner-gate.
--
--    DO UPDATE with an IS DISTINCT FROM guard rather than DO NOTHING (#214 idiom): the
--    loader replays every db/*.sql on every connect, so a stale or tampered row heals to
--    the migration text — while the guard keeps the steady-state replay write-free (no
--    dead tuple, no validate-trigger fire) once the row already matches.
INSERT INTO event_type_class AS r (event_type, mode, targets_other_author) VALUES
    ('identity.registration.asserted', 'additive', FALSE)
ON CONFLICT (event_type) DO UPDATE SET
    mode                 = EXCLUDED.mode,
    targets_other_author = EXCLUDED.targets_other_author
WHERE (r.mode, r.targets_other_author)
      IS DISTINCT FROM (EXCLUDED.mode, EXCLUDED.targets_other_author);

-- 2. The structural floor.
--
--    Signature is the unified (p_type text, b jsonb) the #173 registry dispatches with.
--    `p_type` is unused here — this file registers exactly ONE event type, so the check
--    already knows what it is validating; the parameter exists so every registered
--    check_fn has one shape the dispatcher can call blind.
--
--    Runs at BOTH doors: db/005 step 8 (local authoring) and db/020 step 8 (remote
--    apply) both dispatch through `cairn_event_twin`. Every rule below is STRUCTURAL —
--    a judgment about the SHAPE of the claim, not about local policy or a local registry
--    — which is precisely why it is safe to refuse at the remote door too: a peer that
--    produced one of these shapes produced something no conformant door of any version
--    could have minted, so refusing it cannot freeze a watermark on an honest event.
CREATE OR REPLACE FUNCTION cairn_check_registration_assertion(p_type text, b jsonb)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    p         jsonb := b -> 'payload';
    v_class   text;
    v_search  jsonb;
    v_query   jsonb;
    v_displayed jsonb;
    v_has_term  boolean;
BEGIN
    IF p IS NULL THEN
        RAISE EXCEPTION 'registration assertion: missing payload';
    END IF;

    -- 2a. class — §5.3's CLOSED set. The class is the discriminant every other rule below
    --     keys off, so a fourth class admitted here would be a registration that NO rule
    --     applies to: it would slip past the search rules entirely. Absent and non-string
    --     land in this same branch deliberately — "no class at all" is not a weaker error
    --     than "a class we do not know", it is the same failure to say which act this is.
    --     Adding a member here means adding it to `RegistrationClass` in
    --     crates/cairn-event/src/registration.rs in the same commit, and vice versa.
    IF jsonb_typeof(p -> 'class') IS DISTINCT FROM 'string'
       OR (p ->> 'class') NOT IN ('standard', 'unidentified', 'pseudonymous') THEN
        RAISE EXCEPTION 'registration assertion: unknown registration class "%" — §5.3 admits exactly standard, unidentified, pseudonymous',
            COALESCE(p ->> 'class', '<absent>');
    END IF;
    v_class := p ->> 'class';

    IF v_class <> 'standard' THEN
        -- 2b. A non-standard registration STATES WHY. Unlike `standard` — where the class
        --     IS the explanation and a mandatory free-text box would be a required field
        --     satisfiable only by fabrication (principle 4) — "unidentified" and
        --     "pseudonymous" are exceptional acts whose reason ("unconscious ED arrival,
        --     no ID", "court-ordered protective care") is genuinely informative and is the
        --     only record of why this chart was born outside the normal path.
        IF jsonb_typeof(p -> 'basis') IS DISTINCT FROM 'string'
           OR length(trim(p ->> 'basis')) = 0 THEN
            RAISE EXCEPTION 'registration assertion: a non-standard registration states why — basis must be a non-empty string on a % registration (§5.3/§5.4)',
                v_class;
        END IF;

        -- 2c. THE RULE MOST LIKELY TO BE WRONGLY RELAXED. Absence of `search` for the
        --     non-standard classes is STRUCTURAL, not merely optional. An implementation
        --     that only made `search` optional would satisfy every other rule in this
        --     function and still let a John Doe carry a search attestation — and there is
        --     nothing to search WITH on an unconscious patient with no name, no birth date
        --     and no identifier, so any such attestation is a precise untruth (principle
        --     4: an imprecise near-truth always beats a precise untruth). It would also be
        --     read months later as evidence that a human looked and found nothing, which
        --     is exactly the question a duplicate investigation turns on.
        --
        --     `p ? 'search'` (key presence), not a null test: an explicit `"search": null`
        --     is still an author asserting something about a search, and this is the one
        --     door that can still refuse it. §5.4's search-AFTER-create path records its
        --     result as a later identity event, never retro-fitted onto the birth act.
        IF p ? 'search' THEN
            RAISE EXCEPTION 'registration assertion: a % registration carrying a search claims a search attestation the registrar could not have made — absence here is structural (§5.4)',
                v_class;
        END IF;
        RETURN;
    END IF;

    -- 2d. class = standard ⇒ the search is MANDATORY. This is the whole point of §5.8: the
    --     create act must record the search that preceded it, or a duplicate found six
    --     months later cannot be diagnosed. "Was the other chart on screen when the clerk
    --     clicked create?" — yes means human judgement failed (fix the UI), no means the
    --     search failed (fix the comparator), and those have OPPOSITE fixes. Without the
    --     search there is no way to tell them apart, ever.
    IF jsonb_typeof(p -> 'search') IS DISTINCT FROM 'object' THEN
        RAISE EXCEPTION 'registration assertion: standard registration must carry its search (§5.8 — the create act records the search that preceded it)';
    END IF;
    v_search := p -> 'search';

    -- 2e. query — present, an object, and carrying at least ONE non-empty term. A search
    --     with no terms cannot have found anything, so "0 candidates displayed" from it is
    --     not evidence of absence; it is an attestation with nothing behind it, and it
    --     would read exactly like a diligent search that genuinely found nothing.
    --
    --     Which shapes count as a term is deliberately GENEROUS (any one of the three
    --     suffices): §5.8 does not require a name search — searching by MRN alone is a
    --     complete and often better search — and a floor that demanded a name would be
    --     cultural capture (ADR-0014's argument, applied here). The bar is only that the
    --     registrar typed SOMETHING searchable.
    v_query := v_search -> 'query';
    IF jsonb_typeof(v_query) IS DISTINCT FROM 'object' THEN
        RAISE EXCEPTION 'registration assertion: a search with no terms is not a search — search.query must be an object (§5.8)';
    END IF;
    -- Each arm defends against the wrong TYPE as well as emptiness: the CASE collapses a
    -- non-array to an empty array so a malformed `name_tokens` contributes no term rather
    -- than raising a raw "cannot extract elements from a scalar" with no field named.
    -- `t #>> '{}'` is the idiom for reading the text out of a jsonb SCALAR (`t ->> 0`
    -- addresses an array element and would return NULL here).
    --
    -- THREE-VALUED LOGIC, and why the extra care is not pedantry. `jsonb_typeof(x)` returns
    -- SQL NULL for an ABSENT key, and `NULL = 'string'` is NULL, not FALSE. Written the
    -- obvious way (`= 'string'` plus a bare `IF NOT v_has_term`), the whole OR-chain
    -- evaluates to NULL for `"query": {}` — every key absent — and `IF NOT NULL` is not
    -- taken, so a term-less search is ADMITTED. That is a fail-OPEN defect on the safety
    -- floor, the one direction that must never happen, and it was caught only because the
    -- test suite asserts the empty-query refusal directly. `IS NOT DISTINCT FROM` is
    -- NULL-free by construction, and the `IS NOT TRUE` below refuses on NULL as well as on
    -- FALSE: two independent reasons this cannot fail open again.
    v_has_term :=
        EXISTS (SELECT 1 FROM jsonb_array_elements(
                    CASE WHEN jsonb_typeof(v_query -> 'name_tokens') = 'array'
                         THEN v_query -> 'name_tokens' ELSE '[]'::jsonb END) AS t
                 WHERE jsonb_typeof(t) IS NOT DISTINCT FROM 'string'
                   AND length(trim(t #>> '{}')) > 0)
        OR (jsonb_typeof(v_query -> 'birth_date') IS NOT DISTINCT FROM 'string'
            AND length(trim(COALESCE(v_query ->> 'birth_date', ''))) > 0)
        OR EXISTS (SELECT 1 FROM jsonb_array_elements(
                       CASE WHEN jsonb_typeof(v_query -> 'identifiers') = 'array'
                            THEN v_query -> 'identifiers' ELSE '[]'::jsonb END) AS i
                    WHERE jsonb_typeof(i) IS NOT DISTINCT FROM 'object'
                      AND length(trim(COALESCE(i ->> 'value', ''))) > 0);
    IF v_has_term IS NOT TRUE THEN
        RAISE EXCEPTION 'registration assertion: a search with no terms is not a search — search.query must carry at least one non-empty name token, birth date, or identifier value (§5.8)';
    END IF;

    -- 2f. displayed — present, an array, every element a UUID.
    --
    --     The attestation NAMES the candidates rather than counting them, because a bare
    --     "N = 3" cannot answer whether the duplicate found later was among them. An
    --     element that is not a patient id names nothing and would silently inflate the
    --     count that the projection derives from this array's length.
    --
    --     AN EMPTY ARRAY MUST PASS. `[]` is the NORMAL case for a genuinely new patient:
    --     the search ran and correctly found nothing. Tightening this into a non-empty
    --     requirement would make registering the first patient on a fresh node impossible.
    --     (An empty ARRAY is also entirely different from an absent `search` key — the
    --     first says a search ran and found nothing, the second says no search ran.)
    v_displayed := v_search -> 'displayed';
    IF jsonb_typeof(v_displayed) IS DISTINCT FROM 'array' THEN
        RAISE EXCEPTION 'registration assertion: candidate list malformed — search.displayed must be an array of patient uuids (an EMPTY array is valid: the search ran and found nothing)';
    END IF;
    IF EXISTS (SELECT 1 FROM jsonb_array_elements(v_displayed) AS el
                WHERE jsonb_typeof(el) IS DISTINCT FROM 'string'
                   OR NOT pg_input_is_valid(el #>> '{}', 'uuid')) THEN
        RAISE EXCEPTION 'registration assertion: candidate list malformed — every element of search.displayed must be a uuid string naming a candidate that was on screen';
    END IF;

    -- 2g. incomplete — present and a JSON BOOLEAN. ADR-0060 decision 2: completeness must
    --     be STATED, never assumed by its absence. Defaulting a missing flag to false
    --     would let a node that KNEW it could not show everything it found (a comparator
    --     it lacks, a candidate it could not read, a result set it truncated) present the
    --     search as exhaustive — the search-failed vs judgement-failed distinction above
    --     collapses again, silently and in the safe-looking direction.
    --
    --     Pinned to a JSON boolean rather than cast with `::boolean`, which would inherit
    --     Postgres's permissive input syntax: `1`, `"true"` and `"yes"` would all read as
    --     stated-complete while `"banana"` failed with a raw type error naming no field.
    --     The spelling is permanent once it is inside a signed body (db/042's argument for
    --     `strike`, same trap).
    IF jsonb_typeof(v_search -> 'incomplete') IS DISTINCT FROM 'boolean' THEN
        RAISE EXCEPTION 'registration assertion: completeness must be stated, not assumed — search.incomplete must be present and a JSON boolean (ADR-0060)';
    END IF;
END;
$$;
REVOKE EXECUTE ON FUNCTION cairn_check_registration_assertion(text, jsonb) FROM PUBLIC;

-- 3. Register the floor + the hard twin requirement in the ADR-0048 registry. Placed
--    AFTER the function above so db/005's fail-closed registration trigger (which
--    resolves check_fn(text, jsonb) via to_regprocedure at INSERT time) sees it declared.
--
--    A registration must stay legible to a reader with no schema at all (§3.13,
--    principle 11): "Patient registered (standard registration); searched before
--    creating, 2 near-match(es) displayed" answers the duplicate question decades from
--    now, whatever became of this table. #214 DO UPDATE arm so a tampered row heals on
--    replay; the IS DISTINCT FROM guard keeps a converged replay write-free.
INSERT INTO cairn_event_twin_check AS r (event_type, check_fn, twin_required_msg) VALUES
    ('identity.registration.asserted', 'cairn_check_registration_assertion',
     'registration requires a non-empty authored twin (§3.13)')
ON CONFLICT (event_type) DO UPDATE SET
    check_fn          = EXCLUDED.check_fn,
    twin_required_msg = EXCLUDED.twin_required_msg
WHERE (r.check_fn, r.twin_required_msg)
      IS DISTINCT FROM (EXCLUDED.check_fn, EXCLUDED.twin_required_msg);

-- 4. The projection: a RETAINED SET, not a standing overlay.
--
--    Every registration event keeps its own row — the primary key includes the event's
--    content address. That is deliberate and it is the opposite of every demographic
--    overlay in db/010-014, for a clinical reason: a second registration for a chart that
--    already has one is EVIDENCE (someone registered the same patient twice, or two nodes
--    minted the same chart), and an overlay that kept only a winner would destroy exactly
--    the record a duplicate investigation needs. The winner is chosen at READ time by the
--    `_current` view below, so nothing is lost to pick it.
--
--    NO PAIRED `ALTER TABLE ... ADD COLUMN IF NOT EXISTS` accompanies this CREATE, because
--    the table is introduced whole here and there is nothing to widen. That is a statement
--    about TODAY, not a licence: `CREATE TABLE IF NOT EXISTS` NO-OPS on a database that
--    already has the table, so any column added to the body below in a LATER commit must
--    ship with its own idempotent ALTER or it will never reach an upgraded-in-place node,
--    and every apply-fn INSERT naming it will then fail at trigger depth — a total write
--    outage for this event type (issue #207, guarded by migration_replay_widening.rs).
CREATE TABLE IF NOT EXISTS patient_registration (
    patient_id           UUID    NOT NULL,
    class                TEXT    NOT NULL,   -- §5.3: standard | unidentified | pseudonymous
    basis                TEXT,               -- why, for the non-standard classes; NULL for standard
    -- Derived at projection time from jsonb_array_length(search.displayed). The WIRE
    -- CARRIES NO SUCH FIELD, and must not: two representations of one number is a lie
    -- waiting to happen (a `displayed_count` beside a `displayed` array can disagree, and
    -- the signed body would make the disagreement permanent). This column is a READ
    -- CONVENIENCE derived from the array, never a second source of truth — the array in
    -- the event body remains authoritative, and it is what names WHICH candidates were on
    -- screen, which a count can never answer.
    --
    -- 0 for the non-standard classes, where no search ran at all. That is NOT the same
    -- fact as a standard registration whose search found nothing, and the two are told
    -- apart by `class` (and by `search_incomplete IS NULL`), never by this column alone.
    displayed_count      INTEGER NOT NULL,
    search_incomplete    BOOLEAN,            -- NULL ⇔ no search ran (non-standard classes)
    registered_hlc_wall  BIGINT  NOT NULL,
    registered_hlc_count INTEGER NOT NULL,
    registered_origin    TEXT    NOT NULL,
    content_address      BYTEA   NOT NULL,   -- the registering event's address; part of the PK
    first_seen           TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (patient_id, content_address)
);

-- 5. Fold exactly one registration event into the retained set.
--
--    ADR-0052 §2 seal-robustness: a wrongly-sealed NON-clinical row holds CIPHERTEXT in
--    e.body. Only `clinical.*` bodies are born-sealed, and db/005 refuses a sealed
--    identity body outright — but the APPLY door stays lenient (a refusal there would
--    freeze the sync watermark on a verifiable event), so such a row can still reach this
--    function. Reading its ciphertext would drive NULLs into NOT NULL columns and wedge
--    the watermark anyway, so a sealed row projects NOTHING: harmless noise, no custody,
--    no leak. Same guard, same reasoning, as db/010's patient_identifier_apply.
--
--    ON CONFLICT DO NOTHING is CORRECT HERE, and is NOT the #254 first-applied-wins bug
--    being repeated: that bug arose where the PK was a semantic key (patient, system,
--    match_key) that two DIFFERENT events could share, so "do nothing" silently kept
--    whichever event happened to apply first. Here the PK INCLUDES the content address,
--    which is unique per distinct event — so a conflict means the SAME event is being
--    applied twice (set-union re-delivery, or a cairn_reproject heal), and the existing
--    row is byte-for-byte the row we would write. Doing nothing is genuinely idempotent,
--    and there is no winner to pick.
CREATE OR REPLACE FUNCTION patient_registration_apply(e event_log)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    p jsonb := e.body;
BEGIN
    IF e.sealed THEN RETURN; END IF;
    INSERT INTO patient_registration
        (patient_id, class, basis, displayed_count, search_incomplete,
         registered_hlc_wall, registered_hlc_count, registered_origin, content_address)
    VALUES (
        e.patient_id,
        p ->> 'class',
        p ->> 'basis',
        -- jsonb_array_length is STRICT, so an absent search yields NULL → 0. The floor
        -- above guarantees this path is only ever reached with `displayed` an array (or
        -- with no search at all, on a non-standard class), so this never raises.
        COALESCE(jsonb_array_length(p -> 'search' -> 'displayed'), 0),
        -- NULL when no search ran — the honest "not applicable", distinct from FALSE
        -- ("a search ran and it was complete").
        (p -> 'search' -> 'incomplete')::boolean,
        e.hlc_wall, e.hlc_counter, e.node_origin, e.content_address)
    ON CONFLICT (patient_id, content_address) DO NOTHING;
    RETURN;
END;
$$;

-- A plain (non-trigger) function gets PUBLIC EXECUTE by default; the projection layer is
-- door-driven and must not be callable by the runtime role. Same discipline as every
-- privileged fn in db/005.
REVOKE EXECUTE ON FUNCTION patient_registration_apply(event_log) FROM PUBLIC;

-- 6. The chart's birth act — EARLIEST wins.
--
--    Note the ASC. This is the mirror image of every standing-state overlay in the
--    codebase, which order DESC because the LATEST claim supersedes: a name, an address,
--    a dose is whatever was last asserted. A registration is not a standing state, it is a
--    BIRTH: the act that brought this chart into being already happened, and a later
--    registration event for the same chart cannot un-happen it. So the earliest is the
--    real one, and the later ones are the evidence that something went wrong (retained in
--    the table above, exactly so an investigation can see them).
--
--    The full ordering key is total, so every node converges on the same winner from the
--    same event set regardless of arrival order: (hlc_wall, hlc_counter) is the causal
--    order, node_origin breaks a genuine concurrent tie, and content_address — unique per
--    distinct event — closes it absolutely. COLLATE "C" on the text member per ADR-0045:
--    without it a federation of nodes with different default collations could rank two
--    origins differently and read a DIFFERENT birth act for the same chart. (BYTEA has no
--    collation, so content_address needs none.)
CREATE OR REPLACE VIEW patient_registration_current AS
SELECT DISTINCT ON (patient_id)
    patient_id, class, basis, displayed_count, search_incomplete,
    registered_hlc_wall, registered_hlc_count, registered_origin, content_address
FROM patient_registration
ORDER BY patient_id,
         registered_hlc_wall ASC, registered_hlc_count ASC,
         registered_origin COLLATE "C" ASC, content_address ASC;

GRANT SELECT ON patient_registration, patient_registration_current TO cairn_agent;

-- 7. Register the apply fn with the ADR-0057 generic dispatcher (db/005) and the
--    cairn_reproject heal/rebuild path (db/039).
--
--    run_order 10 is the single-fn default — the dispatcher orders only WITHIN one
--    event_type, and this type has exactly one apply fn. It matches patient_chart_apply's
--    10 because a registration is likewise a chart-birth event.
--
--    heal_safe = TRUE: the apply above is idempotent by construction (content-addressed
--    PK + DO NOTHING), so replaying an already-projected event changes nothing — unlike
--    note.added's counter, which can only heal via truncate-then-replay.
INSERT INTO cairn_projection_apply AS r (event_type, apply_fn, projection_tables, run_order, heal_safe)
VALUES ('identity.registration.asserted', 'patient_registration_apply',
        ARRAY['patient_registration'], 10, TRUE)
ON CONFLICT (event_type, apply_fn) DO UPDATE SET
    projection_tables = EXCLUDED.projection_tables,
    run_order         = EXCLUDED.run_order,
    heal_safe         = EXCLUDED.heal_safe
WHERE (r.projection_tables, r.run_order, r.heal_safe)
      IS DISTINCT FROM (EXCLUDED.projection_tables, EXCLUDED.run_order, EXCLUDED.heal_safe);

COMMIT;
