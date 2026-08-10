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

-- ---------------------------------------------------------------------------
-- 6. The retained sets.
--
--    patient_id is on EVERY row regardless of subject kind: it makes the whole effective-
--    grade computation one indexed scan per chart, instead of repeating #336 (the med-list
--    read path is O(all medications on the node) per chart open).
--
--    NO CHECK on subject_kind or grade: both are open vocabularies (ADR-0056/principle 11).
CREATE TABLE IF NOT EXISTS sensitivity_assertion (
    content_address BYTEA   PRIMARY KEY,   -- the producing event; provenance-precise
    event_id        UUID    NOT NULL,
    patient_id      UUID    NOT NULL,
    subject_kind    TEXT    NOT NULL,
    subject_id      UUID    NOT NULL,
    grade           TEXT    NOT NULL,
    source          TEXT    NOT NULL,
    rationale       TEXT,
    hlc_wall        BIGINT  NOT NULL,
    hlc_counter     INTEGER NOT NULL,
    node_origin     TEXT    NOT NULL,
    first_seen      TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
CREATE INDEX IF NOT EXISTS sensitivity_assertion_patient_idx
    ON sensitivity_assertion (patient_id);

--    NO FOREIGN KEY from `withdraws` to sensitivity_assertion. A withdrawal can arrive
--    BEFORE the assertion it withdraws (set-union sync has no ordering) and must still take
--    effect when the assertion lands — so "standing" is a set difference evaluated at READ
--    (section 7), never a row deletion at apply. Same arrival-order independence as
--    ADR-0059's "a strike NULLs the anchor rather than deleting the row".
CREATE TABLE IF NOT EXISTS sensitivity_withdrawal (
    content_address BYTEA   PRIMARY KEY,
    event_id        UUID    NOT NULL,
    withdraws       BYTEA   NOT NULL,
    patient_id      UUID    NOT NULL,
    rationale       TEXT    NOT NULL,
    hlc_wall        BIGINT  NOT NULL,
    hlc_counter     INTEGER NOT NULL,
    node_origin     TEXT    NOT NULL,
    first_seen      TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
CREATE INDEX IF NOT EXISTS sensitivity_withdrawal_target_idx
    ON sensitivity_withdrawal (withdraws);

GRANT SELECT ON sensitivity_assertion, sensitivity_withdrawal TO cairn_agent;

-- ---------------------------------------------------------------------------
-- 7. Apply. ON CONFLICT DO NOTHING is genuinely idempotent here (not the #254 bug): the PK
--    IS the content address, so a conflict means the SAME event applying twice and the
--    existing row is byte-for-byte what we would write.
--
--    The `e.sealed` guard mirrors every other non-clinical projection: only clinical.* is
--    born-sealed and db/005 refuses a sealed sensitivity body, but the APPLY door stays
--    lenient, so such a row can still reach here. Reading its ciphertext would drive NULLs
--    into NOT NULL columns and wedge the watermark; projecting nothing is harmless noise.
CREATE OR REPLACE FUNCTION sensitivity_assertion_apply(e event_log)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    p jsonb := e.body;
BEGIN
    IF e.sealed THEN RETURN; END IF;
    INSERT INTO sensitivity_assertion
        (content_address, event_id, patient_id, subject_kind, subject_id,
         grade, source, rationale, hlc_wall, hlc_counter, node_origin)
    VALUES (
        e.content_address, e.event_id, e.patient_id,
        p ->> 'subject_kind', (p ->> 'subject_id')::uuid,
        p ->> 'grade', p ->> 'source', p ->> 'rationale',
        e.hlc_wall, e.hlc_counter, e.node_origin)
    ON CONFLICT (content_address) DO NOTHING;
END;
$$;
REVOKE EXECUTE ON FUNCTION sensitivity_assertion_apply(event_log) FROM PUBLIC;

CREATE OR REPLACE FUNCTION sensitivity_withdrawal_apply(e event_log)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    p jsonb := e.body;
BEGIN
    IF e.sealed THEN RETURN; END IF;
    INSERT INTO sensitivity_withdrawal
        (content_address, event_id, withdraws, patient_id, rationale,
         hlc_wall, hlc_counter, node_origin)
    VALUES (
        e.content_address, e.event_id,
        cairn_decode_hex_or_raise('withdraws', p ->> 'withdraws', 'sensitivity withdrawal apply'),
        e.patient_id, p ->> 'rationale',
        e.hlc_wall, e.hlc_counter, e.node_origin)
    ON CONFLICT (content_address) DO NOTHING;
END;
$$;
REVOKE EXECUTE ON FUNCTION sensitivity_withdrawal_apply(event_log) FROM PUBLIC;

-- ---------------------------------------------------------------------------
-- 8. Register both apply fns with the ADR-0057 dispatcher + cairn_reproject heal/rebuild.
--    heal_safe = TRUE: content-addressed PK + DO NOTHING makes replay a no-op.
INSERT INTO cairn_projection_apply AS r (event_type, apply_fn, projection_tables, run_order, heal_safe)
VALUES ('sensitivity.grade.asserted', 'sensitivity_assertion_apply',
        ARRAY['sensitivity_assertion'], 10, TRUE),
       ('sensitivity.grade-withdrawal.asserted', 'sensitivity_withdrawal_apply',
        ARRAY['sensitivity_withdrawal'], 10, TRUE)
ON CONFLICT (event_type, apply_fn) DO UPDATE SET
    projection_tables = EXCLUDED.projection_tables,
    run_order         = EXCLUDED.run_order,
    heal_safe         = EXCLUDED.heal_safe
WHERE (r.projection_tables, r.run_order, r.heal_safe)
      IS DISTINCT FROM (EXCLUDED.projection_tables, EXCLUDED.run_order, EXCLUDED.heal_safe);

-- ---------------------------------------------------------------------------
-- 9. Standing = asserted minus withdrawn. ONE definition, so nothing can disagree about
--    what "still applies" means.
--
--    F1 FIX (review round 1): the `NOT EXISTS` must also pin `w.patient_id = p_patient_id`.
--    A content address being globally unique makes a withdrawal UNAMBIGUOUS about which
--    assertion it names — it does NOT make a cross-chart withdrawal impossible. The
--    human-author ceremony (db/005) is a LOCAL-door-only rule, so a peer's mis-targeted or
--    hostile withdrawal is admitted leniently at the remote door; without the patient_id
--    pin here, a withdrawal authored on chart B naming chart A's content_address would
--    strip chart A's protection. That is the unrecoverable direction (a grade can only be
--    lowered by mistake, never raised back silently), so the extra join column is
--    load-bearing, not a tidiness nice-to-have.
CREATE OR REPLACE FUNCTION cairn_sensitivity_standing(p_patient_id uuid)
RETURNS TABLE (content_address bytea, subject_kind text, subject_id uuid, grade text)
LANGUAGE sql STABLE AS $$
    SELECT a.content_address, a.subject_kind, a.subject_id, a.grade
    FROM sensitivity_assertion a
    WHERE a.patient_id = p_patient_id
      AND NOT EXISTS (SELECT 1 FROM sensitivity_withdrawal w
                       WHERE w.withdraws = a.content_address
                         AND w.patient_id = p_patient_id);
$$;

-- ---------------------------------------------------------------------------
-- 10. Event -> thread. Returns NULL when the thread cannot be determined HERE.
--
--     medication_id lives INSIDE the sealed payload, and every medication projection is
--     populated through cairn_clear_payload — so on a node holding no custody the rows are
--     absent and this returns NULL. That is not a bug to route around; section 11 turns the
--     NULL into a conservative bound. It also returns NULL for a SHREDDED event, whose
--     projection rows db/037 scrubbed — which is exactly why the bound is needed today and
--     not only after sequester lands.
CREATE OR REPLACE FUNCTION cairn_event_thread(p_event_id uuid)
RETURNS uuid LANGUAGE plpgsql STABLE AS $$
DECLARE
    v_ca     bytea;
    v_thread uuid;
BEGIN
    -- !! LANGUAGE plpgsql, NOT sql, AND THE GUARD IS LOAD-BEARING !!
    -- cairn-sync loads a SUBSET of the migrations that does NOT include db/031/032, so the
    -- medication projections DO NOT EXIST on that node. A LANGUAGE sql body binds its table
    -- references EAGERLY at CREATE time, so this function would fail to create there and
    -- db/048 would fail to load — taking clinical sync down entirely. (Same late-binding
    -- lesson as #198/#227, and the reason db/005 hosts cairn_clear_payload.) plpgsql binds
    -- at first EXECUTION, and the short-circuit below means the medication SELECT is never
    -- planned on a schema that lacks those tables.
    --
    -- Returning NULL there is not a workaround, it is the honest answer: a node that cannot
    -- see medication threads cannot resolve one, which is the SAME state as holding no
    -- custody. Section 11's conservative bound then applies. Safe direction by construction.
    --
    -- Minor fix (review round 1): the UNION below spans FIVE tables from TWO migration
    -- files (medication_statement/medication_cessation/medication_coding from db/031;
    -- medication_dose_event/medication_dose_correction from db/032). Checking only
    -- medication_statement silently assumed the other four always arrive with it. Every
    -- shipped loader (cairn-node's full set, cairn-sync's subset) does load 031 and 032
    -- as a pair — neither loader lists one without the other — but the loop that replays
    -- db/*.sql is not one atomic transaction, so a loader that crashes mid-replay between
    -- 031 and 032 is a real (if narrow) window in which medication_statement exists and
    -- medication_dose_event does not. Checking one representative table PER SOURCE FILE
    -- (031 and 032) costs nothing and closes that window instead of assuming it shut.
    IF to_regclass('public.medication_statement') IS NULL
       OR to_regclass('public.medication_dose_event') IS NULL THEN
        RETURN NULL;
    END IF;

    SELECT content_address INTO v_ca FROM event_log WHERE event_id = p_event_id;
    IF v_ca IS NULL THEN
        RETURN NULL;
    END IF;

    -- Every medication-thread projection table, unioned on their shared (medication_id,
    -- content_address) shape. NOTE: db/032's dose-point table is named
    -- medication_dose_event (not medication_dose) — checked against the actual db/032
    -- CREATE TABLE rather than assumed, since a wrong name here would bind at first
    -- EXECUTION (plpgsql), not at CREATE time, and fail silently until an event on that
    -- specific thread type was looked up.
    --
    -- F4 (review round 1) — WHAT THIS ACTUALLY RESOLVES, stated plainly so the comment
    -- cannot be read as promising more than it does: medication_statement,
    -- medication_cessation and medication_coding are `PRIMARY KEY (medication_id)` tables
    -- with `ON CONFLICT (medication_id) DO UPDATE` apply functions (db/031) — ONE row per
    -- medication_id, HLC-overlaid, carrying only the CURRENT WINNING event's
    -- content_address. A superseded assert/cessation/coding event's content_address is
    -- therefore GONE from these three tables the moment a later event overlays it — even
    -- on a node holding full custody, looking that old event up here finds nothing.
    -- medication_dose_event (and medication_dose_correction, keyed on the dose event it
    -- corrects) is the ONE table here with true per-event granularity, so only dose-point
    -- history resolves precisely.
    --
    -- This does not make thread resolution wrong: a query that gets NULL back is exactly
    -- what makes it fall through to section 11's conservative bound, which is a SAFE
    -- (over-protective, never under-protective) answer for that superseded event. The
    -- maintainer has ruled against widening this resolver to also index historical
    -- content addresses (out of scope for this slice) — the bound is what covers the
    -- rest, by design, not by accident.
    SELECT medication_id INTO v_thread FROM (
        SELECT medication_id, content_address FROM medication_statement
        UNION ALL SELECT medication_id, content_address FROM medication_cessation
        UNION ALL SELECT medication_id, content_address FROM medication_coding
        UNION ALL SELECT medication_id, content_address FROM medication_dose_event
        UNION ALL SELECT medication_id, content_address FROM medication_dose_correction
    ) t
    WHERE t.content_address = v_ca
    LIMIT 1;
    RETURN v_thread;
END;
$$;

-- ---------------------------------------------------------------------------
-- 10b. F3 (review round 1, maintainer ruling): scope the unresolved-thread BOUND (section
--      11) to event types that could plausibly carry a medication thread.
--
--      Before this fix, `cairn_event_thread(...) IS NULL` was ALSO true for every note,
--      demographic edit, identity assertion, registration and sensitivity event — none of
--      which can EVER belong to a medication thread, resolved or not — so a single
--      thread-scoped 'sequestered' assertion coarsened the ENTIRE CHART: every note,
--      every demographic field, everything. That silently made thread-scoping BEHAVE
--      like chart-wide scoping, defeating the reason a narrower subject kind exists.
--
--      TRUE only for event types THIS VERSION KNOWS cannot belong to a medication
--      thread. An unrecognised (future) type returns FALSE — "might have a thread" — so
--      it still takes the conservative bound; this mirrors cairn_sensitivity_rank's ELSE
--      MAX (unknown must coarsen, never expose). Note the deliberate asymmetry:
--      `clinical.%` (medication's own namespace) and anything not listed here keep the
--      bound; only types we have POSITIVELY confirmed are thread-free opt out. A future
--      clinical stream therefore inherits the bound automatically, for free, simply by
--      not appearing in this list — the safe default requires no one to remember to add
--      it.
CREATE OR REPLACE FUNCTION cairn_event_type_has_no_thread(p_type text)
RETURNS boolean LANGUAGE sql IMMUTABLE AS $$
    SELECT p_type LIKE 'demographic.%' OR p_type LIKE 'identity.%'
        OR p_type LIKE 'note.%'        OR p_type LIKE 'patient.%'
        OR p_type LIKE 'sensitivity.%' OR p_type LIKE 'erasure.%';
$$;

-- ---------------------------------------------------------------------------
-- 11. The effective grade: max by rank over standing assertions on
--     {this event, its thread, its patient}, with the winning subject named.
--
--     THE THREAD BRANCH IS THE SUBTLE ONE (ADR-0062, design §10b):
--       * thread resolves            -> that thread's standing assertions
--       * unresolved, EVENT TYPE COULD carry a thread (F3, review round 1 — gated by
--         cairn_event_type_has_no_thread), chart HAS any thread-scoped assertion
--                                    -> ALL of the chart's thread assertions. A precise
--                                       conservative bound, not a sentinel: the event
--                                       belongs to SOME thread here, so the tightest safe
--                                       answer is the max over the chart's thread grades.
--       * unresolved, chart has none, OR the event type is one we KNOW has no thread
--         concept at all (a note, a demographic edit, ...) -> nothing. Without the
--         "chart has none" half every medication event on every custody-less node would
--         coarsen maximally; without the type gate (F3) EVERY event on the chart —
--         including ones that structurally cannot be on a medication thread — would too.
--
--     CONSEQUENCE, stated so nobody "fixes" it: the effective grade is NON-MONOTONE IN
--     CUSTODY — gaining custody can LOWER it, as the bound collapses to the true value. The
--     grade is a function of local custody, not a global fact. ADR-0052 §9 found the same
--     about ADR-0049's thread commitment. Any cross-node equality test must therefore hold
--     custody equal.
--
--     Absence of every assertion reads as 'routine' (the coalesce below), never as unknown.
--     When nothing applies, `content_address` is left as SQL NULL (not coalesced to a
--     sentinel) — there genuinely is no winning assertion to name, and a caller that wants
--     to know whether a real assertion won just checks `subject_kind <> 'none'` instead of
--     needing a second, address-shaped "no winner" value.
--
--     F2 FIX (review round 1) — a MIS-TARGETED known subject_kind now coarsens instead of
--     silently matching nothing. Before this fix, a 'patient'-kind assertion whose
--     subject_id names a DIFFERENT patient (a typo, a UI bug, a hostile peer), or an
--     'event'-kind assertion whose subject_id names no event on THIS chart, matched none
--     of the arms above and contributed nothing — while an entirely UNRECOGNISED kind
--     correctly coarsened. That asymmetry meant the safer path (coarsen on confusion) was
--     reserved for kinds a future peer invents, and withheld from a kind we already know,
--     mis-used. An assertion that names something we cannot match here was still an
--     ATTEMPT to protect something, so — like the unrecognised-kind case it now joins —
--     it coarsens rather than evaporating.
CREATE OR REPLACE FUNCTION cairn_effective_sensitivity(p_event_id uuid)
RETURNS TABLE (grade text, subject_kind text, content_address bytea)
LANGUAGE sql STABLE AS $$
    WITH ev AS (
        SELECT e.event_id, e.patient_id, e.event_type, cairn_event_thread(e.event_id) AS thread
        FROM event_log e WHERE e.event_id = p_event_id
    ),
    standing AS (
        SELECT s.* FROM ev, LATERAL cairn_sensitivity_standing(ev.patient_id) s
    ),
    applicable AS (
        -- event-scoped, correctly targeted (subject_id names THIS event)
        SELECT s.grade, s.subject_kind, s.content_address
        FROM standing s, ev
        WHERE s.subject_kind = 'event' AND s.subject_id = ev.event_id
        UNION ALL
        -- chart-scoped, correctly targeted (subject_id names THIS chart)
        SELECT s.grade, s.subject_kind, s.content_address
        FROM standing s, ev
        WHERE s.subject_kind = 'patient' AND s.subject_id = ev.patient_id
        UNION ALL
        -- thread-scoped, resolved
        SELECT s.grade, s.subject_kind, s.content_address
        FROM standing s, ev
        WHERE s.subject_kind = 'thread' AND ev.thread IS NOT NULL
          AND s.subject_id = ev.thread
        UNION ALL
        -- thread-scoped, UNRESOLVED: the conservative bound (design §10b), gated to event
        -- types that could plausibly carry a thread (F3 — cairn_event_type_has_no_thread).
        SELECT s.grade, s.subject_kind, s.content_address
        FROM standing s, ev
        WHERE s.subject_kind = 'thread' AND ev.thread IS NULL
          AND NOT cairn_event_type_has_no_thread(ev.event_type)
        UNION ALL
        -- an UNRECOGNISED subject kind, OR a KNOWN kind that is MIS-TARGETED (F2): read as
        -- chart-wide, bounded by this envelope's patient (over-select, never silently miss
        -- — db/006's recall discipline). The two extra OR arms catch exactly the shapes the
        -- arms above cannot: a 'patient' assertion naming a DIFFERENT patient, and an
        -- 'event' assertion naming an event that does not exist ON THIS CHART (wrong chart
        -- or simply invalid) — checked once per standing row, not per queried event, so it
        -- fires identically for every event on the chart, exactly like the unrecognised-kind
        -- arm it now sits beside.
        SELECT s.grade, s.subject_kind, s.content_address
        FROM standing s, ev
        WHERE s.subject_kind NOT IN ('event', 'thread', 'patient')
           OR (s.subject_kind = 'patient' AND s.subject_id <> ev.patient_id)
           OR (s.subject_kind = 'event' AND NOT EXISTS (
                   SELECT 1 FROM event_log x
                   WHERE x.event_id = s.subject_id AND x.patient_id = ev.patient_id))
    )
    -- The LEFT JOIN LATERAL over a one-row constant is what makes this return EXACTLY ONE
    -- row even when nothing applies — so every caller can use query_one and read 'routine'
    -- rather than having to distinguish "no row" from "not sensitive". Absence is not
    -- unknown (principle 4), and that distinction is easiest to get wrong at the call site.
    SELECT COALESCE(a.grade, 'routine'),
           COALESCE(a.subject_kind, 'none'),
           a.content_address
    FROM (SELECT 1) AS one_row
    LEFT JOIN LATERAL (
        SELECT ap.grade, ap.subject_kind, ap.content_address
        FROM applicable ap
        -- Rank first; content_address breaks a tie between two equally-ranked grades
        -- deterministically (BYTEA has no collation — ADR-0045/#115).
        ORDER BY cairn_sensitivity_rank(ap.grade) DESC, ap.content_address ASC
        LIMIT 1
    ) a ON TRUE;
$$;

GRANT EXECUTE ON FUNCTION cairn_effective_sensitivity(uuid) TO cairn_agent;
GRANT EXECUTE ON FUNCTION cairn_sensitivity_standing(uuid) TO cairn_agent;
GRANT EXECUTE ON FUNCTION cairn_event_thread(uuid) TO cairn_agent;

COMMIT;
