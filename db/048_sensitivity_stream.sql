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
--
--    The substitute control is the SECTION 12 ceremony below (called from db/005): a bound
--    human author, enforced at the LOCAL authoring door only.
--
--    Do NOT fold the withdrawal's non-empty `rationale` into that sentence — it is a
--    SEPARATE rule with a DIFFERENT scope: a structural floor in section 4, registered in
--    the ADR-0048 twin-check registry and dispatched through cairn_event_twin, which BOTH
--    doors call (db/005 step 8, db/020 step 8). So a rationale-less peer withdrawal IS
--    refused remotely; only the bound-human-author half is local-only. Conflating the two
--    is what ADR-0062's erratum E2 was written to correct, and it matters in both
--    directions: it overstates what the remote door lets through, and it understates what
--    a peer must already supply.
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
--    (section 7), never a row deletion at apply. Same arrival-order independence as ADR-0059
--    decision 3 as built in db/042, where a strike NULLs the anchor rather than deleting the
--    row.
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
--    THE SEALED ARM IS NOT THE USUAL "PROJECT NOTHING", AND MUST NOT BE SIMPLIFIED INTO ONE.
--    Only clinical.* is born-sealed and db/005 refuses a sealed sensitivity body, but the
--    APPLY door is lenient by design (db/020 never rejects a sealed event), so such a row can
--    still reach here. Reading its ciphertext would drive NULLs into NOT NULL columns, so it
--    cannot be projected literally — but every OTHER non-clinical projection answers that by
--    returning, and for those the cost is losing a FACT (a demographic field goes unindexed,
--    visibly). Here the cost would be losing a PROTECTION: no row means no standing assertion
--    means cairn_effective_sensitivity computes 'routine' for a chart the peer is holding at
--    'sequestered'. Silent, invisible, and the disclosure direction.
--
--    So an unreadable assertion is projected as a DELIBERATELY UNRECOGNISABLE one: patient_id
--    is in the clear on the envelope even for a sealed row, and 'unreadable' matches no arm of
--    section 11 except the catch-all, while ranking MAX (section 1's ELSE). The chart coarsens
--    — the honest answer to "a peer asserted a grade here and I cannot read which". This keeps
--    the file's one invariant intact: EVERY path coarsens on ignorance, none exposes on it.
--
--    The WITHDRAWAL arm below keeps the plain RETURN, and the asymmetry is the point: dropping
--    a withdrawal leaves the assertion it targeted STANDING, which over-protects. Same
--    ignorance, same safe direction, opposite handling — because the two verbs move the grade
--    in opposite directions.
CREATE OR REPLACE FUNCTION sensitivity_assertion_apply(e event_log)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    p jsonb := e.body;
BEGIN
    IF e.sealed THEN
        INSERT INTO sensitivity_assertion
            (content_address, event_id, patient_id, subject_kind, subject_id,
             grade, source, rationale, hlc_wall, hlc_counter, node_origin)
        VALUES (
            e.content_address, e.event_id, e.patient_id,
            -- Neither value is in any recognised vocabulary, deliberately: the kind falls to
            -- section 11's catch-all (chart-wide) and the grade to section 1's ELSE (MAX).
            'unreadable', e.event_id, 'unreadable', 'unreadable', NULL,
            e.hlc_wall, e.hlc_counter, e.node_origin)
        ON CONFLICT (content_address) DO NOTHING;
        RETURN;
    END IF;
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
--    THE `w.patient_id = p_patient_id` PIN IN THE `NOT EXISTS` IS LOAD-BEARING — without it
--    a withdrawal authored on chart B could strip chart A's protection.
--    A content address being globally unique makes a withdrawal UNAMBIGUOUS about which
--    assertion it names — it does NOT make a cross-chart withdrawal impossible. The
--    human-author ceremony (db/005) is a LOCAL-door-only rule, so a peer's mis-targeted or
--    hostile withdrawal is admitted leniently at the remote door; without the patient_id
--    pin here, a withdrawal authored on chart B naming chart A's content_address would
--    strip chart A's protection. That is the unrecoverable direction (a grade can only be
--    lowered by mistake, never raised back silently), so the extra join column is
--    load-bearing, not a tidiness nice-to-have.
-- A withdrawal only counts if it is AUTHORITATIVE (#380, ADR-0064). This one clause is the
-- whole of §5.9's protection-removing control, and it is HERE — the single definition of
-- "what still applies" that cairn_effective_sensitivity (section 11), db/049's
-- cairn_prospective_sensitivity and the CLI read path all delegate to — precisely so no
-- consumer can be written that forgets it, and so part C's custody dial inherits it for
-- free. Do NOT push this check up into the callers: that is the per-dial duplication that
-- produced #404 and #399 one file over.
--
-- The withdrawal stays in the log, replicates, converges and is re-assertable; it simply
-- does not participate in this set difference. Nothing is refused at either door, so
-- nothing forks (#342), and nothing PROTECTIVE is ever gated — only lowering is.
--
-- `cairn_claim_authority` computes at READ, not at apply. TWO axes follow, and they are NOT
-- the same shape — one heals, one does not:
--
--   1. THE UNREPLICATED TARGET (heals). A withdrawal naming a target that has not arrived
--      here is inert today (R2 cannot resolve, but R1 still carries an attested claim
--      alone) and self-heals the moment the target lands — no re-apply, no second event, no
--      stamped-at-apply verdict to go stale (claim_authority.rs's Task 3 pins this).
--
--   2. ACTOR-REGISTRY STATE CHANGING AFTER ADMISSION (does NOT heal). Both R1 and R2
--      resolve their actor through `actor_current` (db/005), which EXCLUDES a revoked actor
--      (db/004:64-68). So revoking an attester — or the self-withdrawer — AFTER their
--      withdrawal landed flips it to 'unverified', the withdrawal drops out of the set
--      difference below, the assertion RE-STANDS and the grade goes back UP.
--
--      `supersede` DOES THIS TOO, by a different route, and #409 does not currently name it:
--      `actor_current` is `DISTINCT ON (actor_id)` over `op IN ('enroll','supersede')` taking
--      the latest row, and a supersede row carries neither `signing_key_id` nor `kind`
--      (db/004) — so R1's key match finds nothing and R2's `kind = 'human'` goes NULL. Both
--      fall to 'unverified', identically to a revoke. Latent only because no rotate-key door
--      exists yet, but an ADR-0029 skill-epoch bump is exactly the routine, benign event that
--      would trigger it the day one is built (#410 review finding).
--
--      Confirmed by a throwaway check in a scratch database during the build — not by a
--      committed test (no revoke-then-reread scenario is covered). The direction is SAFE
--      (protection is
--      restored, never removed), so this is a declared consequence of read-time authority
--      rather than a defect — but whether it is RIGHT is undecided and is issue #409
--      (contamination cascade says authority follows revocation; a clinician merely leaving
--      should probably not silently re-seal charts they lawfully opened). Do not "fix" it
--      here without reading that issue: the healing in axis 1 and the re-raise in axis 2
--      are the same property, and you cannot keep one without the other.
--
-- WHAT IS *NOT* REACHABLE, so do not add a third axis for it: a BEARING withdrawal whose
-- attester is not yet enrolled HERE at ARRIVAL time. apply_remote_event's non-deferred
-- attestation gate refuses an unenrolled attester outright (db/020:251-254) — that
-- withdrawal never lands at all, so it can never sit here "inert" waiting for its attester
-- to enrol.
--
-- THE QUALIFIER "BEARING" IS LOAD-BEARING and an earlier version of this comment dropped it
-- (#410 review finding I5), reasoning instead from the type being CLASSIFIED. Classification
-- alone does not reach that gate: db/020's condition is
-- `IF NOT v_deferred AND (v_mode = 'suppressing' OR v_bears)`, and
-- `sensitivity.grade-withdrawal.asserted` is registered `'additive'` (section 2), so only
-- the `v_bears` disjunct can fire — i.e. only when a contributor carries a `responsibility`
-- object. A NON-bearing peer withdrawal skips the gate entirely and lands with
-- `attester_key` NULL, which is why the plain `recorded` shape is admissible at all (it is
-- the shape claim_authority.rs's un-attested fixtures use). Such a row is 'unverified' for
-- want of evidence, not because evidence was rejected. The conclusion above still holds; the
-- route to it is different. claim_authority.rs's trailing comment states it correctly.
--
-- Note the asymmetry with axis 2: the door screens the registry at arrival, and nothing
-- re-screens it afterwards. Confirmed by a throwaway check in a scratch database during the
-- build (#380 Task 3) — not by a committed test.
-- !! THE AUTHORITY TEST BELOW IS POSITIVE (`IN`), NOT NEGATIVE (`<> 'unverified'`) !!
-- Do not "simplify" it back. This is the ONE site where a verdict STRIPS protection, so the
-- polarity decides what an UNRECOGNISED verdict does. Written negatively, anything that is
-- not byte-for-byte 'unverified' — a typo in the CASE, and far more plausibly a FOURTH
-- verdict added by some future ADR — silently GAINS the power to strip a grade, and passes
-- every existing test on the way in. Written positively, an unrecognised verdict withholds.
--
-- That is the same doctrine `cairn_sensitivity_rank`'s ELSE states at the top of this file
-- ("an unrecognised value ranking 0 would WITHHOLD PROTECTION … the failure mode here must
-- be over-coarsening, never disclosure"), and the same one `cairn_claim_authority`'s own
-- header states as principle 4 ("uncertainty withholds power, it never confers it"). This
-- clause used to contradict both (#410 review finding C3). The verdict never crosses the
-- wire — it is computed locally, and db/005 and db/048 load together on every connect — so
-- unlike the open TEXT grade ladder there is no forward-compatibility reason to leave it
-- open. Pinned by claim_authority.rs's `an_unrecognised_verdict_withholds_the_power_to_strip`.
CREATE OR REPLACE FUNCTION cairn_sensitivity_standing(p_patient_id uuid)
RETURNS TABLE (content_address bytea, subject_kind text, subject_id uuid, grade text)
LANGUAGE sql STABLE AS $$
    SELECT a.content_address, a.subject_kind, a.subject_id, a.grade
    FROM sensitivity_assertion a
    WHERE a.patient_id = p_patient_id
      AND NOT EXISTS (SELECT 1 FROM sensitivity_withdrawal w
                       WHERE w.withdraws = a.content_address
                         AND w.patient_id = p_patient_id
                         AND cairn_claim_authority(w.event_id, a.event_id)
                             IN ('attested', 'self'));
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
    -- WHY TWO to_regclass PROBES AND NOT ONE: the UNION below spans FIVE tables from TWO
    -- migration files (medication_statement/medication_cessation/medication_coding from db/031;
    -- medication_dose_event/medication_dose_correction from db/032). Checking only
    -- medication_statement silently assumed the other four always arrive with it. No shipped
    -- loader ever lists one of 031/032 without the other (cairn-node loads both; cairn-sync
    -- loads neither) — but the loop that replays db/*.sql is not one atomic transaction, so a
    -- loader that crashes mid-replay between 031 and 032 is a real (if narrow) window in which
    -- medication_statement exists and medication_dose_event does not. Checking one
    -- representative table PER SOURCE FILE (031 and 032) costs nothing and closes that window
    -- instead of assuming it shut.
    IF to_regclass('public.medication_statement') IS NULL
       OR to_regclass('public.medication_dose_event') IS NULL THEN
        RETURN NULL;
    END IF;

    SELECT content_address INTO v_ca FROM event_log WHERE event_id = p_event_id;
    IF v_ca IS NULL THEN
        RETURN NULL;
    END IF;

    -- The five medication-thread projection tables from db/031 and db/032, unioned on their
    -- shared (medication_id, content_address) shape. NOTE: db/032's dose-point table is named
    -- medication_dose_event (not medication_dose) — checked against the actual db/032
    -- CREATE TABLE rather than assumed, since a wrong name here would bind at first
    -- EXECUTION (plpgsql), not at CREATE time, and fail silently until an event on that
    -- specific thread type was looked up.
    --
    -- NOT "every medication-thread projection table": db/034's medication_attestation also
    -- carries both medication_id and content_address (PRIMARY KEY (event_id), so genuinely
    -- per-event) and is deliberately absent. Adding it would need a THIRD to_regclass probe,
    -- since db/034 is outside cairn-sync's subset too. The omission is safe by construction —
    -- an attestation's event type is clinical.%, so it fails cairn_event_type_has_no_thread and
    -- takes the conservative bound instead, which over-protects.
    --
    -- WHAT THIS RESOLVES, AND WHAT IT DOES NOT — stated plainly so the comment cannot be
    -- read as promising more than it does. FOUR of these five tables are one-row-per-key
    -- upserts carrying only the CURRENT WINNING event's content_address:
    --   * medication_statement, medication_cessation, medication_coding —
    --     `PRIMARY KEY (medication_id)` with `ON CONFLICT (medication_id) DO UPDATE` apply
    --     functions (db/031), HLC-overlaid, ONE row per medication_id.
    --   * medication_dose_correction — `PRIMARY KEY (corrected_dose_event_id)` with
    --     `ON CONFLICT (corrected_dose_event_id) DO UPDATE SET ... content_address =
    --     EXCLUDED.content_address` (db/032). Being keyed on the dose event it CORRECTS is
    --     not per-event granularity for the correction itself: re-correcting the same dose
    --     point overwrites the address, so the earlier correction stops resolving here.
    -- A superseded event's content_address is therefore GONE from those four the moment a
    -- later event overlays it — even on a node holding full custody, looking that old event
    -- up here finds nothing.
    --
    -- BUT "EVERY SUPERSEDED MEDICATION EVENT RESOLVES TO NULL" IS FALSE, and an earlier draft
    -- of this comment (and of ADR-0062) said it. medication_dose_event is keyed PER EVENT
    -- (`PRIMARY KEY (dose_event_id)`, `ON CONFLICT ... DO NOTHING`) and db/032:403 registers
    -- `medication_dose_seed_initial` for `clinical.medication.asserted`, seeding a row whose
    -- dose_event_id IS that assert's own event_id and whose content_address is that assert's
    -- own. So a superseded `clinical.medication.asserted` — and likewise a superseded
    -- `-dose-change.asserted` — still resolves here, permanently and precisely.
    --
    -- The real limitation is narrower and differently shaped: it is a superseded `ceased` or
    -- `coding` event (their tables are keyed on medication_id) and a RE-corrected
    -- `dose-correction` (keyed on the dose point it corrects, so an earlier correction of the
    -- same point stops resolving). Those fall to section 11's bound, which over-protects.
    -- ADR-0062 erratum E4 (issue #374) carries the same correction.
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
-- 10b. THE TYPE GATE ON THE UNRESOLVED-THREAD BOUND (maintainer ruling): scope the bound
--      (section 11) to event types that could plausibly carry a medication thread.
--
--      Without this gate, `cairn_event_thread(...) IS NULL` was ALSO true for every note,
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
-- 10c. WHICH CHART A MEDICATION THREAD BELONGS TO, when this node can tell at all.
--
--      Returns the thread's patient_id, or NULL when this node CANNOT ANSWER — either the
--      medication projections are absent (the cairn-sync subset; same to_regclass guard and
--      same LANGUAGE plpgsql late-binding requirement as section 10) or the thread has simply
--      not replicated here yet.
--
--      THE NULL IS THE WHOLE POINT, and it is why callers must test
--      "known here AND demonstrably elsewhere" rather than "not known to be here".
--      Set-union sync has no ordering, so a thread-scoped assertion legitimately arrives
--      before the thread it names (the same arrival-order independence section 9 states for
--      withdrawals and section 11 states for event-scoped assertions). A caller that treated
--      NULL as "wrong chart" would fire on every honest not-yet-replicated thread, and — on a
--      custody-less node, where medication_statement is empty for EVERY thread because
--      cairn_clear_payload cannot open the sealed bodies — it would fire on all of them at
--      once, coarsening the entire chart. That is exactly the failure section 10b's type gate
--      was added to prevent, so it must not be reintroduced here by the back door.
CREATE OR REPLACE FUNCTION cairn_thread_patient(p_thread uuid)
RETURNS uuid LANGUAGE plpgsql STABLE AS $$
DECLARE v_patient uuid;
BEGIN
    IF to_regclass('public.medication_statement') IS NULL THEN
        RETURN NULL;
    END IF;
    SELECT patient_id INTO v_patient
      FROM medication_statement WHERE medication_id = p_thread;
    RETURN v_patient;
END;
$$;

-- ---------------------------------------------------------------------------
-- 11. The effective grade: max by rank over standing assertions on
--     {this event, its thread, its patient}, with the winning subject named.
--
--     THE THREAD BRANCH IS THE SUBTLE ONE (ADR-0062, design §10b):
--       * thread resolves            -> that thread's standing assertions
--       * unresolved, EVENT TYPE COULD carry a thread (the section 10b type gate —
--         cairn_event_type_has_no_thread), chart HAS any thread-scoped assertion
--                                    -> ALL of the chart's thread assertions. A precise
--                                       conservative bound, not a sentinel: the event
--                                       belongs to SOME thread here, so the tightest safe
--                                       answer is the max over the chart's thread grades.
--       * unresolved, chart has none, OR the event type is one we KNOW has no thread
--         concept at all (a note, a demographic edit, ...) -> nothing. Without the
--         "chart has none" half every medication event on every custody-less node would
--         coarsen maximally; without the section 10b type gate EVERY event on the chart —
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
--     sentinel) — there genuinely is no winning assertion to name.
--
--     THE "DID ANYTHING WIN" TEST IS `content_address IS NOT NULL`, NEVER
--     `subject_kind <> 'none'`. subject_kind carries no CHECK (section 6, deliberately — it
--     is an open vocabulary), so `{"subject_kind":"none"}` is a structurally valid assertion
--     that both doors admit. Were 'none' still doubling as the nothing-applies sentinel, such
--     an assertion would return ('sequestered', 'none', <a real address>) and a caller
--     following this file's own guidance would read "nothing applies" while a standing
--     sequestered grade was in force — the disclosure direction, in code written to spec.
--     content_address cannot collide that way: it is a BYTEA PRIMARY KEY (section 6), so a
--     real winner always has one and only the no-winner case is NULL.
--
--     A MIS-TARGETED KNOWN subject_kind COARSENS INSTEAD OF SILENTLY MATCHING NOTHING.
--     Without the extra OR arms in the last branch below, a 'patient'-kind assertion whose
--     subject_id names a DIFFERENT patient (a typo, a UI bug, a hostile peer), an
--     'event'-kind assertion whose subject_id names no event on THIS chart, or a
--     'thread'-kind assertion naming a thread that is demonstrably on another chart, matched
--     none of the arms above and contributed nothing — while an entirely UNRECOGNISED kind
--     correctly coarsened. That asymmetry meant the safer path (coarsen on confusion) was
--     reserved for kinds a future peer invents, and withheld from a kind we already know,
--     mis-used. An assertion that names something we cannot match here was still an
--     ATTEMPT to protect something, so — like the unrecognised-kind case it now joins —
--     it coarsens rather than evaporating.
--
--     THAT BRANCH REPORTS subject_kind = 'coarsened', NOT THE ROW'S OWN KIND. A report that
--     echoed the raw kind would say "winning subject: this event" for an assertion that is in
--     fact blurring the WHOLE CHART by mis-target — the exact confusion the named-subject
--     requirement exists to prevent (a reader must be able to tell "one thing to go and look
--     at" from "everything"). 'coarsened' says what actually happened: something applies
--     chart-wide that we could not match to a specific subject.
--
--     That is the OVER-protecting half of a mis-target, and it is all this read model can
--     do: chart B, the one the author meant to seal, is not mentioned by this query at all.
--     The UNDER-protecting half — chart B silently keeping 'routine' — is unfixable at read
--     time and is refused at authoring instead (section 12), which is why both halves of a
--     mis-typed chart-wide raise are now covered.
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
        -- types that could plausibly carry a thread (cairn_event_type_has_no_thread, §10b).
        SELECT s.grade, s.subject_kind, s.content_address
        FROM standing s, ev
        WHERE s.subject_kind = 'thread' AND ev.thread IS NULL
          AND NOT cairn_event_type_has_no_thread(ev.event_type)
        UNION ALL
        -- an UNRECOGNISED subject kind, OR a KNOWN kind that is MIS-TARGETED: read as
        -- chart-wide, bounded by this envelope's patient (over-select, never silently miss
        -- — db/006's recall discipline). The two extra OR arms catch exactly the shapes the
        -- arms above cannot: a 'patient' assertion naming a DIFFERENT patient, and an
        -- 'event' assertion naming an event that does not exist ON THIS CHART — checked
        -- once per standing row, not per queried event, so it fires identically for every
        -- event on the chart, exactly like the unrecognised-kind arm it now sits beside.
        --
        -- THE 'event' ARM HAS THREE CAUSES, NOT TWO, AND THE THIRD IS DELIBERATE, NOT A BUG
        -- TO "FIX" LATER. "No event x with x.event_id = s.subject_id
        -- AND x.patient_id = ev.patient_id" is true for (a) a genuinely wrong chart, (b) an
        -- invalid/dangling id, and (c) an event that IS real and IS on this chart but has
        -- simply NOT REPLICATED to this node YET — set-union sync has no ordering, so an
        -- event-scoped sensitivity assertion can arrive before the event it targets, exactly
        -- the same arrival-order independence section 9 already states for withdrawals
        -- ("a withdrawal can arrive BEFORE the assertion it withdraws"). Case (c) means this
        -- arm can TRANSIENTLY coarsen the whole chart on a partially-replicated node until
        -- the target event lands, at which point the row moves to the precisely-targeted
        -- 'event' arm above and the coarsening self-resolves. That is the correct,
        -- over-protective direction (principle 4: an imprecise near-truth beats a precise
        -- untruth) — DO NOT narrow this arm to exclude case (c), e.g. by trying to tell
        -- "not yet arrived" apart from "never will": there is no local signal that
        -- distinguishes them, and guessing wrong in that narrowing is the disclosure
        -- direction this whole arm exists to prevent.
        --
        -- THE 'thread' ARM USES THE OPPOSITE TEST TO THE 'event' ARM, DELIBERATELY.
        -- 'event' asks "is the target ABSENT from this chart" because event_log is present on
        -- every node and is never custody-gated, so absence is a usable (if replication-lagged)
        -- signal. medication_statement is BOTH custody-gated (cairn_clear_payload cannot open a
        -- sealed body without the DEK, so the table is empty for every thread on a custody-less
        -- node) AND absent entirely on the cairn-sync subset. There, "not found" is the NORMAL
        -- state, carrying no information whatever — so this arm asks the POSITIVE question
        -- instead: is the named thread known here AND demonstrably on a DIFFERENT chart? NULL
        -- (cannot tell) coalesces to this chart's own id and the arm stays silent, leaving the
        -- unresolved-thread bound above to give the conservative answer. Using the 'event'
        -- arm's shape here would coarsen every chart on every custody-less node at once, which
        -- is precisely what section 10b's type gate exists to prevent.
        SELECT s.grade, 'coarsened'::text, s.content_address
        FROM standing s, ev
        WHERE s.subject_kind NOT IN ('event', 'thread', 'patient')
           OR (s.subject_kind = 'patient' AND s.subject_id <> ev.patient_id)
           OR (s.subject_kind = 'event' AND NOT EXISTS (
                   SELECT 1 FROM event_log x
                   WHERE x.event_id = s.subject_id AND x.patient_id = ev.patient_id))
           OR (s.subject_kind = 'thread'
               AND COALESCE(cairn_thread_patient(s.subject_id), ev.patient_id)
                   <> ev.patient_id)
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
-- Granted EXPLICITLY, matching the other three — it worked without this line on the
-- default PUBLIC EXECUTE grant, but that left it one blanket REVOKE away from silently
-- breaking cairn_effective_sensitivity's read path, and the rest of this file grants
-- explicitly on purpose.
GRANT EXECUTE ON FUNCTION cairn_event_type_has_no_thread(text) TO cairn_agent;
GRANT EXECUTE ON FUNCTION cairn_thread_patient(uuid) TO cairn_agent;

-- ---------------------------------------------------------------------------
-- 11b. The §5.9 withdrawal worklist (ADR-0064). A VIEW, deliberately, and not a flag ledger.
--
-- WHY NOT THE ADR-0058 t_effective_ceiling_flag IDIOM (db/040's table, reached via db/049's
-- safety_overclaim_flag — NOT literally the next file): that records a judgement AT THE
-- DOOR, and authority is computed at READ precisely because the answer
-- IMPROVES — a withdrawal is inert today because its target has not replicated, and clears
-- tomorrow when it does (section 9's axis 1; R2 then resolves).
--
-- NOT "or its attester is not enrolled here": that half was wrong (#410 review finding I5)
-- and contradicted section 9's own axis-2 note 370 lines above, which states categorically
-- that an unenrolled attester on a BEARING withdrawal is refused at arrival and never lands
-- to sit here. Nor does `attester_key` ever change on an admitted row, so an un-attested
-- withdrawal's R1 verdict cannot improve either. The target-replication route is the real
-- one, and it is the one the tests exercise. An apply-time ledger would fill with
-- rows that were true for an afternoon, and a worklist that is mostly stale is §5.12's
-- alert-fatigue disease, self-inflicted, in the one place we are building a control.
-- The rule for choosing: FLAG WHAT CANNOT SELF-HEAL; VIEW WHAT CAN. db/049's
-- safety-overclaim flag is a published byte and takes the other branch.
--
-- Two reasons, and the second is the one nothing else in the system would show:
--   'inert'             — the gate stopped it. Transient; disappears when it heals.
--   'stranger-attested' — the gate LET IT THROUGH. An accountable human lowered a grade on
--                         a chart they had NO PRIOR PRESENCE ON AT THE MOMENT OF THE STRIP.
--                         Permanent in the sense that matters: nothing the flagged actor
--                         does AFTERWARDS — including simply continuing to work on this
--                         chart — can clear this row (see the HLC bound below). It is a
--                         fact about a completed act, fixed at the instant the act happened.
--
-- 'stranger-attested' reuses the chart-standing question that ADR-0064 REJECTED as an
-- authority input. That is not an inconsistency: as authority it fails the locum, the
-- night-cover registrar and the receiving ED, who must not be second-class; as SALIENCE it
-- blocks nothing and delays nothing — the withdrawal has already taken effect. §5.13's
-- duplicate-sweep posture: surface, never block.
--
-- WHY NOT A `node_origin` COMPARISON, DESPITE THE COLUMN BEING CALLED THAT (task-4
-- ruling 2 finding). The only per-node identity this schema tracks is `local_node`
-- (db/007), and db/007 — the WHOLE file, `trust_peer` included — is DELIBERATELY ABSENT
-- from the cairn-sync subset (crates/cairn-sync/src/main.rs's own header comment): a
-- reference to it here would fail db/048's load on a sync node and take clinical sync
-- down. There is also no OTHER canonical "this node" signal: `node_origin` on every row
-- (this table, `sensitivity_assertion`, `event_log` itself) is copied VERBATIM from the
-- event body's own self-asserted `hlc.node_origin` field, identically at both doors
-- (db/005:~1299, db/020:~427) — a client-chosen string, not a verified one, and there is
-- no reference value to compare it against without `local_node`. So this view answers a
-- DIFFERENT, and better, question: not "which network node relayed this" (topology,
-- unverifiable here) but "has the accountable human ever authored anything else on THIS
-- CHART" (an actual fact `event_log`/`actor_current` can answer in the cairn-sync subset
-- too) — which is what "stranger to the chart" means in the design's own prose. `w.
-- node_origin` is carried through as a plain OUTPUT column for an operator's own
-- investigation, but never drives `reason`.
--
-- A DELIBERATE WIDENING THE BRIEF'S OWN SKETCH DID NOT HAVE. The brief's placeholder SQL
-- gated this arm on `w.node_origin IS DISTINCT FROM cairn_this_node_origin()`, which would
-- have EXCLUDED a purely LOCAL strip by a human with no prior presence on the chart — only
-- a cross-node one would have listed. This view lists BOTH: a locally-authored withdrawal
-- by someone who has never touched this chart before is exactly as unaccountable as a
-- remote one, and node-of-origin was never what made it accountable — presence on the
-- chart was. A reader comparing this view to the brief's own wording should expect that
-- widening, not read it as drift.
--
-- WHO IS "THE ACCOUNTABLE ACTOR": for R1 ('attested'), the VOUCHED attester — resolved
-- from `attester_key` through `actor_current`. NOTE this is NOT byte-for-byte
-- `cairn_claim_authority`'s own R1 test, despite resolving the same key: R1 counts ALL
-- actors mapped to the key and requires them ALL to be human
-- (`count(*) = 1 AND bool_and(kind = 'human')`, db/005) — so a key mapped to one human AND
-- one agent is WITHHELD by R1 (ambiguous, 'unverified'). This query instead filters to
-- `kind = 'human'` FIRST and only then requires exactly one such row, which differs from
-- R1 ONLY on that dual-mapped-key case — and never in a way this view can actually observe:
-- `responsible_actor_id` is only ever CONSULTED when `verdict <> 'unverified'`, and a row
-- with `verdict = 'attested'` already passed `cairn_claim_authority`'s own stricter R1 test
-- to get that verdict — meaning the key resolved to exactly one actor, period, before this
-- query ever ran. So on the only path where this resolution is used for a `verdict = 'attested'`
-- row, it agrees with R1 exactly; THAT is not a second, looser copy of the same rule — the claim
-- is defended for R1 ONLY, not for the COALESCE as a whole (see the caveat below). For R2
-- ('self'), there is USUALLY no attester to resolve (`attester_key` is typically NULL), so this
-- falls back to the withdrawal's own `actor_id` — which R2 already requires to equal the TARGET
-- assertion's own actor. That fallback is why R2 ordinarily never needs a special case: the actor
-- withdrawing their OWN claim necessarily HAS other content on the chart (the claim itself), so
-- the "no prior presence" test below always excludes it from 'stranger-attested' — precisely
-- production's `sensitivity::withdraw_sensitivity` self-signed, self-attested shape
-- (task 4's third test).
--
-- THE EDGE CASE THAT NARROWS THE CLAIM (salience-only, essentially unreachable): the COALESCE
-- below branches on `attester_key IS NOT NULL`, not on the row's actual `verdict`. #408 tracks a
-- key mapped to MORE THAN ONE actor (e.g. one human, one agent) — cairn_claim_authority's R1
-- rejects that as ambiguous ('unverified'), but a withdrawal whose SIGNER separately satisfies R2
-- can still verdict 'self' while carrying that same non-NULL, R1-failing `attester_key`. This
-- query's human-filtered sub-select is looser than R1 and can still resolve one such key to a
-- single human — so on a 'self'-verdict row carrying an incidental attester_key, the first
-- COALESCE branch wins and returns the ATTESTER's actor rather than the actor R2's verdict is
-- actually grounded on. That attester may have no prior presence on the chart even though the
-- true self-withdrawer does, producing a spurious 'stranger-attested' row.
--
-- "NO PRIOR PRESENCE" IS BOUNDED TO EVENTS AT OR BEFORE THE WITHDRAWAL, BY HLC
-- (`hlc_wall`, `hlc_counter` — both columns already on `event_log`, both in the cairn-sync
-- subset). REVIEW FINDING, task-4 Important #1: the first cut of this predicate checked
-- for ANY OTHER event by the responsible actor with NO TIME BOUND at all — which meant the
-- flagged actor could clear their OWN row simply by continuing to work on the chart
-- afterwards: the locum strips a grade on a chart they have never touched, the row
-- appears, they document the consultation ten minutes later, the row vanishes before
-- anyone triages it, and nothing anywhere records that the strip happened. That made the
-- evidence of the one act this view exists to surface erasable by the party being
-- flagged, through the entirely innocent act of continuing to work — in a design whose own
-- position is that the record is the control and the gate is only the forcing function, a
-- record the flagged party can clear is not a control. The question this row asks is "did
-- this actor have any relationship to this chart AT THE MOMENT they stripped its
-- protection" — later activity answers a different, less useful question, and letting it
-- clear the row would make the "permanent... fixed at the instant the act happened" claim
-- above false. An event authored BEFORE the withdrawal but REPLICATED here AFTER it still
-- clears the row — that is correct, and consistent with the arrival-order self-healing
-- everywhere else in this slice (section 9; this view's own 'inert' arm below): it reveals
-- the actor genuinely did have prior presence, this node simply could not see it yet.
--
-- KNOWN GAP, NOT FIXED HERE (task-4 Minor #3; CORRECTED by #410 review finding I5).
-- A withdrawal mis-stamped with the WRONG chart's `patient_id` — naming a real assertion
-- that in fact lives on a DIFFERENT chart — never lowers anything, because
-- `cairn_sensitivity_standing` is patient-scoped on BOTH sides (section 9). Neither door
-- refuses it on admission: the ceremony's chart-mismatch checks (section 12) are in the
-- ASSERTION branch only. So the strip is silently ineffective, which is clinically harmless
-- and is exactly the kind of act a worklist would want to show.
--
-- WHAT THIS VIEW DOES WITH IT DEPENDS ON THE VERDICT, and an earlier version of this
-- comment claimed — wrongly — that it "NEVER MATCHES EITHER ARM" and appears "AT ALL, under
-- ANY reason". That is true of the FIRST arm only. The `judged` CTE's
-- `LEFT JOIN sensitivity_assertion a ON a.content_address = w.withdraws` below is NOT
-- patient-qualified, so a mis-chart withdrawal still resolves its target and still gets a
-- real verdict:
--   * verdict 'unverified' — arm 1 tests standing on `w.patient_id` and finds nothing, so
--     the row is genuinely INVISIBLE. That is the documented gap.
--   * verdict 'attested'/'self' — arm 2 never consults standing at all. It fires whenever
--     the responsible actor has no prior presence on the (wrong) chart, and the row surfaces
--     as `'stranger-attested'` — a MISLABEL, not an omission: it names a clinician as having
--     stripped protection they did not strip, on a row this file's own prose calls permanent.
--     The inverse case is the common one and is right by luck: on a mistyped chart the
--     clinician usually IS working on that chart, so prior presence suppresses the row.
-- ADR-0064's Known limitations states this corrected version; this comment now matches it.
-- Still left as a documented gap rather than a third arm — the clean fix is to qualify the
-- join with `a.patient_id = w.patient_id` and give the mismatch its own reason string, which
-- is smaller than this comment but changes the view's contract, so it is tracked separately.
-- Not exercised by this task's tests.
--
-- WHY 'inert' ALSO ASKS `cairn_sensitivity_standing`, NOT JUST THIS ROW'S OWN VERDICT.
-- A withdrawal's OWN `cairn_claim_authority(w.event_id, a.event_id)` verdict can never
-- change once stamped un-attested — `attester_key` on an already-admitted row is fixed
-- forever. So if a SECOND, authoritative withdrawal later strips the SAME target, the
-- first (still-unverified) withdrawal's row would stay listed 'inert' FOREVER under a
-- naive per-row reading — noise about a problem that is already solved, the exact alert
-- fatigue this view exists to avoid. Gating on "is the target STILL standing"
-- (`cairn_sensitivity_standing`, section 9's own set-difference) makes an inert row
-- self-clear the moment ANY accountable route achieves the same effect, not only when
-- THIS SPECIFIC withdrawal's own attestation improves — which is what "the view asks the
-- CURRENT question rather than replaying a stamped verdict" (ADR-0064 decision 6) actually means
-- for a chart carrying more than one withdrawal of the same target. A target that has
-- simply not REPLICATED here yet (arrival-order independence, section 9's own note) is
-- NOT yet in `sensitivity_assertion` at all — `target_content_address IS NULL` — and must
-- still be listed 'inert': there is nothing to check standing OF yet, so the OR below
-- treats "not landed" and "landed and still standing" as the same "still worth watching"
-- case, and only "landed and ALREADY stripped elsewhere" as moot.
CREATE OR REPLACE VIEW sensitivity_withdrawal_worklist AS
WITH judged AS (
    SELECT w.content_address, w.event_id, w.patient_id, w.withdraws, w.node_origin,
           w.rationale, w.hlc_wall, w.hlc_counter,
           a.content_address AS target_content_address,
           cairn_claim_authority(w.event_id, a.event_id) AS verdict,
           -- The accountable actor: the vouched R1 attester if there is one (exactly one
           -- human, or NULL if ambiguous/absent — the `count(*) = 1` guard is what keeps
           -- a key mapped to several actors from silently picking one), else the
           -- withdrawal's own actor (the R2 self case, which is always already excluded
           -- below because that actor authored the target itself).
           COALESCE(
               (SELECT CASE WHEN count(*) = 1 THEN max(act.actor_id) END
                  FROM event_log le, actor_current act
                 WHERE le.event_id = w.event_id
                   AND le.attester_key IS NOT NULL
                   AND act.signing_key_id = encode(le.attester_key, 'hex')
                   AND act.kind = 'human'),
               (SELECT le2.actor_id FROM event_log le2 WHERE le2.event_id = w.event_id)
           ) AS responsible_actor_id
      FROM sensitivity_withdrawal w
      LEFT JOIN sensitivity_assertion a ON a.content_address = w.withdraws
)
SELECT content_address, event_id, patient_id, withdraws,
       CASE WHEN verdict = 'unverified' THEN 'inert' ELSE 'stranger-attested' END AS reason,
       node_origin, rationale,
       -- #421: the accountable actor — the fact the row exists to report. The CTE has
       -- always computed it (the vouched R1 attester, or the withdrawal's own actor for
       -- the R2 self case); dropping it here meant a consumer could say a withdrawal was
       -- ineffective but never who authored it. APPENDED, never inserted mid-list:
       -- CREATE OR REPLACE VIEW permits adding a trailing column and refuses a reorder,
       -- so appending is the only shape that survives migration replay on a live database.
       responsible_actor_id
  FROM judged
 WHERE (verdict = 'unverified'
        AND (target_content_address IS NULL
             OR EXISTS (SELECT 1 FROM cairn_sensitivity_standing(judged.patient_id) st
                         WHERE st.content_address = judged.target_content_address)))
    OR (verdict <> 'unverified'
        AND NOT EXISTS (SELECT 1 FROM event_log other
                          WHERE other.patient_id = judged.patient_id
                            AND other.actor_id = judged.responsible_actor_id
                            AND other.event_id <> judged.event_id
                            -- Bounded to AT OR BEFORE the withdrawal's own HLC — see the
                            -- "NO PRIOR PRESENCE IS BOUNDED..." comment above (Important #1).
                            AND (other.hlc_wall, other.hlc_counter)
                                <= (judged.hlc_wall, judged.hlc_counter)));
GRANT SELECT ON sensitivity_withdrawal_worklist TO cairn_agent;

-- ---------------------------------------------------------------------------
-- 11b. HOW MANY MEDICATION EVENTS ON THIS CHART THIS NODE CANNOT OPEN.
--
-- The §5.9 operator report lists one line per medication thread it can project. That list
-- is silently incomplete on any node holding sealed bodies without the DEK:
-- `medication_statement_apply` opens its payload through `cairn_clear_payload` and RETURNs
-- early on NULL (db/031), so an unopenable event projects no row and the thread simply is
-- not there.
--
-- The report used to INFER that state from "no threads projected AND some assertion
-- stands". That proxy is wrong in both directions — grading is opt-in, so most
-- custody-blind charts carry no standing assertion at all and read as genuinely empty
-- (#383 surviving inside its own fix) — and it says nothing at all about PARTIAL custody,
-- where a plausible truncated list is the most dangerous output of the three.
--
-- It does not need inferring. `event_log` keeps the sealed row whether or not this node
-- can read it; `event_clear` is exactly the set it can. The difference IS the fact.
--
-- A DEFINER, because `event_clear` is `REVOKE ALL ... FROM cairn_agent` at db/005 — the
-- clear shadow is deliberately not readable by the runtime role, and this must not widen
-- that. It returns a COUNT, never a body, so it discloses only how much this node cannot
-- see, which is precisely the fact the operator is owed.
--
-- pg_temp LAST (#426): reads event_log and event_clear UNQUALIFIED, and a blinded read
-- here would return zero — rendering as "custody is complete", the reassuring answer.
CREATE OR REPLACE FUNCTION cairn_patient_sealed_medication_without_custody(p_patient uuid)
RETURNS bigint
LANGUAGE sql STABLE
SECURITY DEFINER SET search_path = public, pg_temp
AS $$
    SELECT count(*)
      FROM event_log e
     WHERE e.patient_id = p_patient
       AND e.event_type LIKE 'clinical.medication%'
       AND e.sealed
       AND NOT EXISTS (SELECT 1 FROM event_clear c WHERE c.event_id = e.event_id);
$$;

REVOKE EXECUTE ON FUNCTION cairn_patient_sealed_medication_without_custody(uuid) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION cairn_patient_sealed_medication_without_custody(uuid) TO cairn_agent;
GRANT EXECUTE ON FUNCTION cairn_patient_sealed_medication_without_custody(uuid) TO cairn_node;

-- ---------------------------------------------------------------------------
-- 12. The ceremony. Called from db/005 (LOCAL authoring) and from NOWHERE ELSE.
--
--     Raising is frictionless — err toward confidential — with these exceptions:
--       * NO ASSERTION MAY CARRY A CATEGORY. Never lawful, at any scope: the body is plaintext
--         and replicates unconditionally, so the category is itself the disclosure.
--       * A MIS-TARGETED SUBJECT IS REFUSED, for all three kinds. A 'patient' subject must
--         name this chart; an 'event' or 'thread' subject must not be one this node can
--         positively place on a DIFFERENT chart. In every case the read model can only cover
--         the over-protecting half, and the under-protecting half — the thing the author meant
--         to grade silently staying 'routine' — is undetectable afterwards.
--       * ANY GRADE WITH CHART-WIDE REACH STATES WHY. That is every kind except 'event' and
--         'thread', because section 11 gives chart-wide effect to everything it does not
--         recognise. Once part B coarsens safety projections such a grade blurs every signal
--         on the chart, including the ones with nothing sensitive about them, so the rationale
--         is what the person who later has to unwind it gets to read.
--
--     Lowering always costs: a bound human author (ADR-0053) plus a rationale. ADR-0061
--     decision 4 REFUSED an authorship gate on registration because that blocks CARE
--     DOCUMENTATION; a withdrawal is an administrative act with a consent basis, blocks
--     nothing clinical (the content stays readable to everyone who already has custody —
--     only the GRADE stays high), so the asymmetry is deliberate, not an oversight.
--
--     WHY NOT HERE, WHY NOT AT db/020: this function judges the EVENT ITSELF (its type and
--     payload shape), so by db/005 step 8b's own rule it belongs among the checks that run
--     BEFORE anything is written — never among the four trailing refusals, which read the
--     NODE'S CONFIGURATION or the log's own state (the missing unwrap key, substitution, and
--     the two erasure-target checks; db/005 draws that distinction explicitly at its step 9). And it must never be
--     called from apply_remote_event: set-union sync has no ordering and peers run
--     different local policies, so a door check at APPLY would let one peer's honestly
--     rationale-less act be refused by another peer's stricter node, forking the event set
--     and wedging replication (ADR-0060, the #342 trap). For a RAISE specifically that
--     refusal would be worse than a wedge: refusing a peer's protective assertion would
--     leave THIS node computing a LOWER grade than the peer already holds — the refusal
--     would itself be a disclosure. crates/cairn-node/tests/sensitivity_ceremony.rs pins
--     both halves of the asymmetry so it is tested, not merely commented.
--
--     p_authorship_actor is the verified-human-attester bytea db/005 already computes at
--     its step 4b (the value fed to cairn_authorship_bound) — NULL unless a valid
--     attestation token from an enrolled human actor was presented for this event. Passing
--     it in rather than re-deriving it keeps this function a pure judgement over its three
--     inputs, with no second lookup that could drift from what step 4b already verified.
CREATE OR REPLACE FUNCTION cairn_sensitivity_ceremony_ok(
    p_type text, b jsonb, p_authorship_actor bytea
) RETURNS void LANGUAGE plpgsql AS $$
-- NOTHING IN THE DECLARE MAY TOUCH THE ENVELOPE. db/005 calls this at step 8a for EVERY event
-- it admits, not only for sensitivity ones, so a DECLARE initialiser runs on every write to the
-- node. An earlier draft of this fix put `v_chart uuid := (b ->> 'patient_id')::uuid` here,
-- which made every note, demographic edit and medication event on the node pay for — and depend
-- on — a cast this function does not own the precondition for. All envelope reads therefore
-- happen inside the type-gated branch below, where they are reached only by the two event types
-- this function actually judges.
DECLARE
    p jsonb := b -> 'payload';
    v_kind    text;
    v_chart   uuid;
    v_subject uuid;
    v_target  uuid;   -- which chart the named subject actually belongs to, when knowable
BEGIN
    IF p_type NOT IN ('sensitivity.grade.asserted', 'sensitivity.grade-withdrawal.asserted') THEN
        RETURN;
    END IF;
    -- THE CATEGORY MUST NEVER REACH THE WIRE. These bodies are plaintext and replicate
    -- UNCONDITIONALLY, so `category: "termination-of-pregnancy"` on an assertion IS the
    -- disclosure the whole mechanism exists to prevent (ADR-0006 decision 4; this file's own
    -- header says so). cairn-event's builder cannot emit the field, but the builder is not a
    -- floor: a bespoke UI (ADR-0021 blesses those) or a client talking raw SQL reaches
    -- submit_event directly, and the twelfth founding principle is that the DATABASE is the
    -- layer such a client cannot walk past. So it is refused here.
    --
    -- LOCAL DOOR ONLY, like every other rule in this function, and for a sharper reason than
    -- usual: a peer that sent a category has ALREADY leaked it — the bytes are on the wire and
    -- in that peer's log. Refusing at apply would not un-disclose anything; it would only fork
    -- the event set and wedge replication (ADR-0060). This rule stops nodes from AUTHORING the
    -- disclosure, which is the only thing a door can actually accomplish.
    IF p_type = 'sensitivity.grade.asserted' AND p ? 'category' THEN
        RAISE EXCEPTION 'sensitivity: a grade assertion must never carry a category — these bodies are plaintext and replicate unconditionally, so the category IS the disclosure the grade exists to prevent (ADR-0006 decision 4); keep the matched category node-local';
    END IF;
    -- A CHART-WIDE GRADE MUST NAME THE CHART IT IS AUTHORED ON.
    --
    -- `sensitivity-assert --patient A --subject-kind patient --subject-id B` is two
    -- hand-typed UUIDs, and a mis-typed pair fails in BOTH directions at once. Section 11's
    -- catch-all arm covers the over-protecting direction (chart A coarsens). It cannot
    -- cover the other: chart B — the chart the author meant to seal — keeps reading
    -- 'routine' forever, with no error and nothing anywhere surfacing the mismatch. A
    -- clinician who believes they sealed a chart and did not is the unrecoverable failure
    -- (a grade computed too HIGH is honest degradation; too LOW discloses), and no read
    -- model can detect it after the fact, because nothing on chart B ever mentions the
    -- assertion. So it is refused at authoring, where the author is still present to fix it.
    --
    -- Checked FIRST, before the rationale rule below: on a mis-targeted rationale-less
    -- raise, demanding a rationale first would send the author away to write a justification
    -- for the wrong chart.
    --
    -- Compared AS uuid, not as text: both values are hand-typed or hand-assembled, and
    -- casing/whitespace differences must not read as "different chart". Section 3's
    -- structural floor has already proved subject_id parses (db/005 dispatches
    -- cairn_event_twin at step 8, BEFORE this call at step 8a), and if that ever changed the
    -- cast would RAISE — refusing the event, which is the safe direction for this door.
    --
    -- LOCAL DOOR ONLY, and deliberately NOT in db/020: a peer that mis-typed the same pair
    -- must still be admitted, or its event forks the event set and wedges replication
    -- (ADR-0060). The peer's chart A coarsens here exactly as section 11 says — refusing a
    -- protective act is never the answer, so this rule stops at the door the author is
    -- standing in front of.
    IF p_type = 'sensitivity.grade.asserted' THEN
        v_kind    := p ->> 'subject_kind';
        v_chart   := (b ->> 'patient_id')::uuid;
        v_subject := (p ->> 'subject_id')::uuid;

        IF v_kind = 'patient' AND v_subject IS DISTINCT FROM v_chart THEN
            RAISE EXCEPTION 'sensitivity: a chart-wide grade must name THIS chart — subject_id % is not this chart (patient_id %); set subject_id to the chart being graded, or grade a thread or a single event instead',
                p ->> 'subject_id', b ->> 'patient_id';
        END IF;

        -- THE SAME MIS-TARGET RULE, FOR THE OTHER TWO SUBJECT KINDS.
        --
        -- The argument the chart-wide rule is built on transfers unchanged: --patient and
        -- --subject-id are two hand-typed UUIDs in ALL THREE cases, and a mis-typed pair fails
        -- in both directions at once. Section 11's catch-all arm covers the over-protecting
        -- half (this chart coarsens). It can never cover the other half — the event or thread
        -- the author MEANT to grade keeps reading 'routine' forever, because the assertion
        -- carries THIS chart's patient_id and cairn_sensitivity_standing is patient-scoped, so
        -- nothing on the intended chart ever mentions it. A clinician who believes they sealed
        -- something and did not is the unrecoverable failure; it is refused here, where the
        -- author is still present to fix it.
        --
        -- "KNOWN HERE AND DEMONSTRABLY ELSEWHERE", never "not known to be here" — this is the
        -- predicate that keeps the rule compatible with set-union sync. A target that has not
        -- replicated yet reads NULL and the rule stays silent, so an honest out-of-order write
        -- is never refused; only a target this node can positively place on a DIFFERENT chart
        -- raises. That also makes the rule self-satisfying: once the target lands, a re-attempt
        -- either passes (right chart) or refuses (wrong chart), never flip-flops.
        IF v_kind = 'event' THEN
            SELECT e.patient_id INTO v_target FROM event_log e WHERE e.event_id = v_subject;
            IF v_target IS NOT NULL AND v_target IS DISTINCT FROM v_chart THEN
                RAISE EXCEPTION 'sensitivity: event % is on chart %, not this chart (%) — the event you meant to grade would have stayed ungraded; correct --subject-id or --patient',
                    v_subject, v_target, v_chart;
            END IF;
        ELSIF v_kind = 'thread' THEN
            v_target := cairn_thread_patient(v_subject);
            IF v_target IS NOT NULL AND v_target IS DISTINCT FROM v_chart THEN
                RAISE EXCEPTION 'sensitivity: medication thread % is on chart %, not this chart (%) — the thread you meant to grade would have stayed ungraded; correct --subject-id or --patient',
                    v_subject, v_target, v_chart;
            END IF;
        END IF;

        -- ANYTHING WITH CHART-WIDE BLAST RADIUS STATES WHY — and that is decided by what the
        -- READ MODEL does with the kind, not by the kind's spelling.
        --
        -- This rule used to read `= 'patient'`, which left a hole big enough to drive the whole
        -- ceremony through: section 11 grants chart-wide effect to EVERY kind it does not
        -- recognise, so `subject_kind: "chart"` (or any other unrecognised string) was a
        -- rationale-free, ceremony-free chart-wide raise straight through the LOCAL door. The
        -- gate and the effect were keyed on different things, and only the gate was narrow.
        -- Inverting it — demand the rationale unless the kind is one of the two we KNOW is
        -- narrowly scoped — ties the ceremony to the blast radius permanently, and gives a
        -- future kind the safe default for free (an unrecognised kind coarsens chart-wide in
        -- section 11 AND owes a rationale here, with nobody needing to remember to add it).
        --
        -- COALESCE so a NULL kind demands the rationale rather than skipping the check. The
        -- structural floor (section 3, dispatched at db/005 step 8, before this call) already
        -- proves subject_kind is a non-empty string, so this cannot fire — it is here because
        -- the fail-closed direction should not depend on another function's guarantee holding.
        IF COALESCE(v_kind, '') NOT IN ('event', 'thread')
           AND (jsonb_typeof(p -> 'rationale') IS DISTINCT FROM 'string'
                OR length(trim(p ->> 'rationale')) = 0) THEN
            RAISE EXCEPTION 'sensitivity: a grade with chart-wide reach states why — supply a rationale (subject_kind "%" coarsens every signal on this chart; only a thread- or event-scoped grade needs none)',
                v_kind;
        END IF;
    END IF;

    IF p_type = 'sensitivity.grade-withdrawal.asserted' AND p_authorship_actor IS NULL THEN
        RAISE EXCEPTION 'sensitivity: withdrawing a grade requires a bound human author — removing protection is accountable (ADR-0053; raising one is not)';
    END IF;
END;
$$;

-- ---------------------------------------------------------------------------
-- 13. The category blacklist — the AUTOMATIC source (ADR-0006 §3).
--
--     Ships EMPTY. Cairn provides the lookup MECHANISM, never the list: what is sensitive
--     is cultural, regional and personal, and shipping a list would be Cairn making the
--     policy (principle 9). A deployment (a clinic, a region, a single practitioner) is who
--     decides that e.g. "sti-screen" or "termination-of-pregnancy" belongs on this table, by
--     writing rows into it — and the SQL mirror below asserts the shipped table is empty
--     precisely so this stays true: a non-empty seed here would be an un-reviewable policy
--     choice smuggled into "infrastructure".
CREATE TABLE IF NOT EXISTS sensitivity_category_map (
    category TEXT PRIMARY KEY,
    grade    TEXT NOT NULL,
    note     TEXT NOT NULL DEFAULT ''
);
GRANT SELECT ON sensitivity_category_map TO cairn_agent;
REVOKE INSERT, UPDATE, DELETE ON sensitivity_category_map FROM PUBLIC;

--     A PURE lookup that yields a CANDIDATE. It authors nothing — all three ADR-0006
--     workflows are the same call site with different callers:
--       silent apply     -> the caller authors the assertion as an advisory actor
--       acceptance first -> the caller shows the candidate, a human authors it
--       manual only      -> the caller never calls this
--
--     THE SUBJECT IS NEVER THE PATIENT. This function cannot express a chart-wide candidate
--     at all: a coded hit on one drug blanket-grading an entire chart is exactly
--     "chart-wide as the default for highly sensitive records", which is the thing the
--     friction in section 12 exists to prevent. The caller pairs the returned grade with
--     the event or thread that carried the coded field — the return shape (grade, category)
--     has no patient/subject column to fill in even by accident.
CREATE OR REPLACE FUNCTION cairn_sensitivity_candidate(p_coded jsonb)
RETURNS TABLE (grade text, category text)
LANGUAGE sql STABLE AS $$
    SELECT m.grade, m.category
    FROM sensitivity_category_map m
    WHERE m.category = (p_coded ->> 'category')
    -- A deliberate no-op TODAY (`category` is the primary key, so this WHERE matches at most
    -- one row) kept for the day a deployment widens that key — e.g. per-locale or per-source
    -- rows — at which point "highest grade wins" is the only safe tie-break. Cheap to keep,
    -- and adding it later means remembering to.
    ORDER BY cairn_sensitivity_rank(m.grade) DESC
    LIMIT 1;
$$;
GRANT EXECUTE ON FUNCTION cairn_sensitivity_candidate(jsonb) TO cairn_agent;

COMMIT;
