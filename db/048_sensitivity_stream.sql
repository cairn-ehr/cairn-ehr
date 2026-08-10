-- Cairn — the §5.9 sensitivity stream (ADR-0006 decision 3, ADR-0062; issue #232 part A).
--
-- Sensitivity is not a boolean on a body: it is an append-only stream of graded assertions
-- whose EFFECTIVE value is a projection (never merge, always overlay). This file ships the
-- stream, the projection, and nothing else. It ENFORCES NOTHING — a grade computed here
-- withholds no content. Sequester (custody narrowing) is #232 part C and is blocked on #231.
--
-- # Why these bodies are plaintext
--
-- ADR-0052 §2 lists what stays unsealed because the machinery binds on it. Sensitivity
-- assertions join that list: a node must READ the grade in order to coarsen, and coarsening
-- is exactly what a node holding no custody of the graded body must still do. Sealing the
-- grade under the key it governs is circular.
--
-- # What must never appear in these bodies
--
-- The matched blacklist CATEGORY. A plaintext, unconditionally-replicated body carrying
-- `category: "termination-of-pregnancy"` IS the disclosure this whole mechanism exists to
-- prevent (ADR-0006 decision 4).

BEGIN;

-- ---------------------------------------------------------------------------
-- 1. The ladder.
--
--    Open TEXT, no CHECK domain: a future grade from an upgraded peer is ADMITTED verbatim
--    (additive-only, principle 11). Gaps of 10 leave room to interpose deployment terms
--    later without renumbering.
--
--    !! READ THIS BEFORE "FIXING" THE ELSE BRANCH !!
--    ELSE is MAX, deliberately INVERTING cairn_clock_grade_rank's ELSE 0 (db/040). There,
--    an unrecognised value ranking 0 is safe because rank 0 WITHHOLDS REJECT POWER. Here,
--    an unrecognised value ranking 0 would WITHHOLD PROTECTION: an older node reading a
--    peer's newer `grade:protected-witness` as "not sensitive" emits an uncoarsened safety
--    projection and renders the body in the clear — a leak on exactly the events that most
--    needed protecting, in code that looks correct because it matches db/040's pattern.
--    The failure mode here must be over-coarsening (honest, repaired by upgrading the node),
--    never disclosure (unrecoverable).
--
--    ABSENCE IS NOT UNKNOWN. No assertion at all contributes nothing and reads as 'routine'
--    (see cairn_effective_sensitivity below); an unparseable or unrecognised GRADE VALUE
--    ranks MAX. Collapsing the two would make every event in the record maximally sensitive
--    — principle 4's not-yet-asked vs unknown.
CREATE OR REPLACE FUNCTION cairn_sensitivity_rank(g text)
RETURNS int LANGUAGE sql IMMUTABLE AS $$
    SELECT CASE g
        WHEN 'routine'     THEN 0
        WHEN 'sensitive'   THEN 10
        WHEN 'restricted'  THEN 20
        WHEN 'sequestered' THEN 30
        ELSE 2147483647    -- unknown / future / NULL: coarsen, never expose
    END;
$$;

-- ---------------------------------------------------------------------------
-- 2. Classify both verbs. 'additive' with targets_other_author = FALSE.
--
--    A WITHDRAWAL is cross-author BY DESIGN: ADR-0006 decision 3 requires declassification
--    by AUTHORITY, not the ADR-0043 self-only suppression rule, because the self-only rule
--    deadlocks every real case (the asserting clinician retired; the patient who asserted
--    has left the practice). So it must NOT be routed through the suppression owner-gate.
--    The substitute control is the §6 ceremony in db/005: a bound human author plus a
--    rationale, enforced at the LOCAL door only.
INSERT INTO event_type_class AS r (event_type, mode, targets_other_author) VALUES
    ('sensitivity.grade.asserted',            'additive', FALSE),
    ('sensitivity.grade-withdrawal.asserted', 'additive', FALSE)
ON CONFLICT (event_type) DO UPDATE SET
    mode                 = EXCLUDED.mode,
    targets_other_author = EXCLUDED.targets_other_author
WHERE (r.mode, r.targets_other_author)
      IS DISTINCT FROM (EXCLUDED.mode, EXCLUDED.targets_other_author);

-- ---------------------------------------------------------------------------
-- 3. The structural floor for an assertion.
--
--    Note what is NOT refused: an unrecognised `subject_kind`. A closed set here would
--    wedge the apply door the first time an upgraded peer sent `episode` (ADR-0056 — the
--    floor gates EFFECT, not presence). The projection interprets an unknown kind
--    conservatively instead (section 6).
CREATE OR REPLACE FUNCTION cairn_check_sensitivity_grade(p_type text, b jsonb)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    p jsonb := b -> 'payload';
BEGIN
    IF p IS NULL THEN
        RAISE EXCEPTION 'sensitivity assertion: missing payload';
    END IF;

    IF jsonb_typeof(p -> 'subject_kind') IS DISTINCT FROM 'string'
       OR length(trim(p ->> 'subject_kind')) = 0 THEN
        RAISE EXCEPTION 'sensitivity assertion: subject_kind must be a non-empty string';
    END IF;

    -- jsonb_typeof(NULL) is NULL, and `NULL IS DISTINCT FROM 'string'` is TRUE, so an
    -- ABSENT key lands in this branch rather than falling through (the #346 fail-OPEN
    -- pattern, avoided deliberately).
    IF jsonb_typeof(p -> 'subject_id') IS DISTINCT FROM 'string' THEN
        RAISE EXCEPTION 'sensitivity assertion: subject_id must be a uuid string';
    END IF;
    BEGIN
        PERFORM (p ->> 'subject_id')::uuid;
    EXCEPTION WHEN others THEN
        RAISE EXCEPTION 'sensitivity assertion: subject_id "%" is not a valid uuid',
            p ->> 'subject_id';
    END;

    -- A blank grade would rank MAX and coarsen everything — safe-looking, but a shape no
    -- author meant to write, and it would mask a UI bug forever (append-only: no UPDATE).
    IF jsonb_typeof(p -> 'grade') IS DISTINCT FROM 'string'
       OR length(trim(p ->> 'grade')) = 0 THEN
        RAISE EXCEPTION 'sensitivity assertion: grade must be a non-empty string';
    END IF;

    IF jsonb_typeof(p -> 'source') IS DISTINCT FROM 'string'
       OR length(trim(p ->> 'source')) = 0 THEN
        RAISE EXCEPTION 'sensitivity assertion: source must be a non-empty string (human | advisory)';
    END IF;
END;
$$;

-- ---------------------------------------------------------------------------
-- 4. The structural floor for a withdrawal.
--
--    `withdraws` is decoded through cairn_decode_hex_or_raise (db/001, issue #228) so a
--    malformed value fails with the door named AND with SQLSTATE P0001. That code is a
--    CONTRACT with cairn-sync's pull loop: P0001 means "deliberate, skip and re-offer",
--    while any other SQLSTATE is read as a transient fault the cursor FREEZES on. A bare
--    decode() raises in class 22 and would stall sync from that peer permanently.
CREATE OR REPLACE FUNCTION cairn_check_sensitivity_withdrawal(p_type text, b jsonb)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    p jsonb := b -> 'payload';
BEGIN
    IF p IS NULL THEN
        RAISE EXCEPTION 'sensitivity withdrawal: missing payload';
    END IF;

    IF jsonb_typeof(p -> 'withdraws') IS DISTINCT FROM 'string' THEN
        RAISE EXCEPTION 'sensitivity withdrawal: withdraws must be the hex content_address of the assertion being withdrawn';
    END IF;
    PERFORM cairn_decode_hex_or_raise('withdraws', p ->> 'withdraws', 'sensitivity withdrawal');

    -- The rationale is the whole ceremony's evidence. Structural (non-empty) here; the
    -- LOCAL door additionally requires a bound human author (section 8).
    IF jsonb_typeof(p -> 'rationale') IS DISTINCT FROM 'string'
       OR length(trim(p ->> 'rationale')) = 0 THEN
        RAISE EXCEPTION 'sensitivity withdrawal: rationale must be a non-empty string (the audited why — ADR-0006 decision 3)';
    END IF;
END;
$$;

-- ---------------------------------------------------------------------------
-- 5. Twin-check registrations (ADR-0048). ADDING A ROW HERE MEANS BUMPING THE EXPECTED
--    COUNT IN **BOTH** crates/cairn-node/tests/twin_registry.rs AND
--    db/tests/034_twin_registry_test.sql — the count lives in two places on purpose.
INSERT INTO cairn_event_twin_check AS r (event_type, check_fn, twin_required_msg) VALUES
    ('sensitivity.grade.asserted', 'cairn_check_sensitivity_grade',
     'sensitivity assertion requires a non-empty authored twin (a grade must be legible without a schema — principle 11)'),
    ('sensitivity.grade-withdrawal.asserted', 'cairn_check_sensitivity_withdrawal',
     'sensitivity withdrawal requires a non-empty authored twin (the audited why must be legible)')
ON CONFLICT (event_type) DO UPDATE SET
    check_fn          = EXCLUDED.check_fn,
    twin_required_msg = EXCLUDED.twin_required_msg
WHERE (r.check_fn, r.twin_required_msg)
      IS DISTINCT FROM (EXCLUDED.check_fn, EXCLUDED.twin_required_msg);

COMMIT;
