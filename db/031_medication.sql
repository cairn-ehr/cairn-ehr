-- 031_medication.sql — the first clinical-content surface (data-model §3.3).
--
-- Two append-only verbs over an immortal medication_id thread:
--   clinical.medication.asserted            — patient takes/took a substance (mints the thread)
--   clinical.medication-cessation.asserted  — the thread is no longer taken (references it)
--
-- Safety floor (the only hard invariants): an assertion must carry a non-empty
-- substance.term and a non-empty info_source; both verbs must carry a valid
-- medication_id uuid. Everything else is honest-unknown (principle 4) — the floor
-- never blocks a medication write beyond these. Duplicates are ALLOWED (two
-- statements do exist); duplicate *detection* is the advisory projection's job.
--
-- event_log.body IS the payload (submit_event inserts body = b->'payload'); patient_id
-- is a top-level column. So the floor check (sees the full body b) reads b->'payload',
-- while the projection triggers read the clear payload via cairn_clear_payload(NEW)
-- (ADR-0052, db/037): NEW.body unchanged on an unsealed row, the event_clear shadow
-- on a sealed one, NULL when this node holds no custody — the trigger then returns
-- without projecting (honest degradation, principle 4).
BEGIN;

-- 1. Register both types in the fail-closed classification registry. Additive,
--    never targeting another author.
INSERT INTO event_type_class (event_type, mode, targets_other_author) VALUES
    ('clinical.medication.asserted',           'additive', FALSE),
    ('clinical.medication-cessation.asserted', 'additive', FALSE)
ON CONFLICT (event_type) DO NOTHING;

-- 2. The structural floor for both verbs. RAISE EXCEPTION per violation.
CREATE OR REPLACE FUNCTION cairn_check_medication_assertion(p_type text, b jsonb)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    p jsonb := b -> 'payload';
BEGIN
    IF p IS NULL THEN
        RAISE EXCEPTION 'medication assertion: missing payload';
    END IF;
    -- medication_id is the thread key on BOTH verbs.
    IF jsonb_typeof(p -> 'medication_id') IS DISTINCT FROM 'string' THEN
        RAISE EXCEPTION 'medication assertion: medication_id must be a uuid string';
    END IF;
    BEGIN
        PERFORM (p ->> 'medication_id')::uuid;
    EXCEPTION WHEN others THEN
        RAISE EXCEPTION 'medication assertion: medication_id must be a valid uuid';
    END;
    -- The start verb carries the clinical floor: a non-empty term + present info_source.
    IF p_type = 'clinical.medication.asserted' THEN
        IF jsonb_typeof(p -> 'substance' -> 'term') IS DISTINCT FROM 'string'
           OR length(btrim(p -> 'substance' ->> 'term')) = 0 THEN
            RAISE EXCEPTION 'medication assertion: substance.term must be a non-empty string (principle 4 floor)';
        END IF;
        IF jsonb_typeof(p -> 'info_source') IS DISTINCT FROM 'string'
           OR length(btrim(p ->> 'info_source')) = 0 THEN
            RAISE EXCEPTION 'medication assertion: info_source must be a non-empty string';
        END IF;
        -- ADR-0059 decision 2: the reserved inn_code slot is RETIRED. Fail loud at the
        -- authoring door (a caller still emitting it is a bug at source); ignore it on
        -- the apply path — a refusal on a verifiable peer event is the sync-wedge
        -- ADR-0056 forbids, and the slot is simply never read again.
        IF (p -> 'substance') ? 'inn_code'
           AND current_setting('cairn.remote_apply', true) IS DISTINCT FROM 'on' THEN
            RAISE EXCEPTION 'medication assertion: substance.inn_code is retired — carry substance.coding {system, code, display} instead (ADR-0059 decision 2)';
        END IF;
        -- The coding floor lives in db/041 (a floor change needs its own generation
        -- bump, #188); plpgsql resolves the call at execution, so the later file is fine.
        PERFORM cairn_check_medication_coding(p);
    END IF;
    -- The cessation verb carries only medication_id (+ optional stopped/reason) — done.
END;
$$;
-- PUBLIC holds EXECUTE by default; the cairn_check_* family is revoked uniformly (#382,
-- convention stated in db/005 above cairn_check_twin_registry_fn).
REVOKE EXECUTE ON FUNCTION cairn_check_medication_assertion(text, jsonb) FROM PUBLIC;

-- 3. Register both medication verbs' structural floor + hard twin requirement in the #173
--    registry (replaces the copied cairn_event_twin dispatch chain; the single db/005
--    dispatcher reads these rows). Placed after the floor fn above so the fail-closed
--    registry trigger (db/005) sees cairn_check_medication_assertion(text, jsonb) declared.
INSERT INTO cairn_event_twin_check (event_type, check_fn, twin_required_msg) VALUES
    ('clinical.medication.asserted',           'cairn_check_medication_assertion', 'medication assertion requires a non-empty authored twin (§3.13/§3.3)'),
    ('clinical.medication-cessation.asserted', 'cairn_check_medication_assertion', 'medication assertion requires a non-empty authored twin (§3.13/§3.3)')
-- DO UPDATE, not DO NOTHING (#214): the loader replays this file on every connect, so the
-- registry row must CONVERGE to the migration text — a stale row (e.g. the pre-#214 §3.15
-- mislabel in twin_required_msg) heals on the next connect instead of persisting forever.
-- The IS DISTINCT FROM guard keeps the steady-state replay write-free: without it every
-- connect rewrites the row (dead tuple + validate-trigger fire) even when nothing changed.
ON CONFLICT (event_type) DO UPDATE SET
    check_fn          = EXCLUDED.check_fn,
    twin_required_msg = EXCLUDED.twin_required_msg
WHERE (cairn_event_twin_check.check_fn, cairn_event_twin_check.twin_required_msg)
      IS DISTINCT FROM (EXCLUDED.check_fn, EXCLUDED.twin_required_msg);

-- 3b. Thread patient-consistency (issue #192, finding A4). A medication_id thread
--     belongs to ONE chart for life: medication_statement's PK is medication_id alone
--     and its overlay does `patient_id = EXCLUDED.patient_id`, so without a guard a
--     buggy or hostile client re-asserting an existing thread under another patient
--     silently re-homed the thread — including every dose point, which joins by
--     medication_id — onto the other chart, convergently, unflagged (a wrong-chart
--     medication list). The split follows db/023's chart_dispute subject-consistency
--     pattern: FAIL LOUD on the local door (nothing accepted yet — catch the caller
--     bug at source), CONVERGE-AND-FLAG on the sync-apply path (peers already hold
--     the validly-signed event; a node-local veto would fork the event set — the flag
--     surfaces the contradiction for humans instead). Offline-first is preserved: a
--     thread whose standing patient is locally UNKNOWN passes (never fabricate
--     certainty, principle 4).

-- The advisory worklist of observed cross-patient contradictions (sync path only —
-- the local door refuses instead). Node-local derived state, never on the wire.
CREATE TABLE IF NOT EXISTS medication_patient_conflict_flag (
    flag_id          BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    medication_id    UUID  NOT NULL,
    standing_patient UUID  NOT NULL,
    asserted_patient UUID  NOT NULL,
    content_address  BYTEA NOT NULL,   -- the contradicting event
    flagged_at       TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
GRANT SELECT ON medication_patient_conflict_flag TO cairn_agent;
-- Natural-key uniqueness keyed on EVENT identity (content_address), not observation
-- shape (medication_id/standing_patient/asserted_patient alone) — the same discipline
-- as db/018's identity_projection_flag (Task-5-adjudicated pattern): a heal-mode
-- reproject replaying the IDENTICAL contradicting event must converge, not append a
-- duplicate alarm row every run, while a genuinely NEW event that independently
-- reobserves the same (medication_id, standing_patient, asserted_patient) triple
-- (e.g. the same wrong-chart mistake recurring later) still gets its own row, because
-- its content_address differs. content_address is NOT NULL on this table from day
-- one (unlike identity_projection_flag's pre-ADR-0057 legacy rows), so no stale-index
-- migration heal is needed here — this is the first unique index this table has ever
-- carried.
CREATE UNIQUE INDEX IF NOT EXISTS medication_patient_conflict_flag_natural_idx
    ON medication_patient_conflict_flag
    (medication_id, standing_patient, asserted_patient, content_address);

-- The thread's standing patient claim: the statement's patient when asserted, else an
-- orphan cessation's (a cessation claims the thread for its chart too), else an orphan
-- coding's, else NULL — honestly unknown. STABLE (reads the projections). plpgsql, not
-- LANGUAGE sql: the body references medication_cessation, created later in this file — a
-- sql-language body is resolved at CREATE time and would break a fresh load; plpgsql
-- resolves at first execution, by which point the whole file has loaded.
--
-- ORDER IS MEANINGFUL. The statement is the authoritative clinical claim, so it wins
-- whenever it exists; the other two arms only speak for a thread whose assert has not
-- arrived (or never will). The medication_coding arm was added by slice 6b (ADR-0059
-- decision 3): a coding OVERLAY may legitimately arrive before the assert it codes, and
-- its row must be filed under some chart (medication_coding.patient_id is NOT NULL). If
-- that claim were invisible here, a later assert naming a DIFFERENT patient would sail
-- past cairn_guard_medication_patient and leave medication_statement and
-- medication_coding permanently disagreeing about which chart the thread belongs to —
-- the exact two-projections-disagree hazard #192 exists to prevent. Making the claim
-- visible instead means such an assert is refused loudly at the local door (and
-- flagged, not refused, on remote apply). The trade is deliberate: a coder who codes
-- against the wrong chart now blocks the real assert with a legible error, rather than
-- silently splitting the thread's chart across two projections.
CREATE OR REPLACE FUNCTION cairn_medication_thread_patient(p_med uuid)
RETURNS uuid LANGUAGE plpgsql STABLE AS $$
BEGIN
    RETURN COALESCE(
        (SELECT patient_id FROM medication_statement WHERE medication_id = p_med),
        (SELECT patient_id FROM medication_cessation WHERE medication_id = p_med),
        (SELECT patient_id FROM medication_coding    WHERE medication_id = p_med));
END;
$$;

-- ONE shared guard for every per-thread verb trigger (assert/cease/dose-change/
-- dose-correction), so the contract cannot drift between verbs (principle 12).
-- RAISEs on the local door; flags and returns on remote apply (db/020 sets the
-- transaction-local cairn.remote_apply marker).
CREATE OR REPLACE FUNCTION cairn_guard_medication_patient(p_med uuid, p_patient uuid, p_ca bytea)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    v_standing uuid := cairn_medication_thread_patient(p_med);
BEGIN
    IF v_standing IS NULL OR v_standing = p_patient THEN
        RETURN;
    END IF;
    IF current_setting('cairn.remote_apply', true) = 'on' THEN
        -- ADR-0057 replay-idempotency, keyed on EVENT identity (content_address), never
        -- on observation shape: a heal-mode reproject re-running this SAME contradicting
        -- event (same p_ca) must not append a duplicate worklist alarm, but a genuinely
        -- NEW event that independently reobserves the same triple is a REAL new
        -- occurrence and must still alarm (see the unique index's comment above).
        INSERT INTO medication_patient_conflict_flag
            (medication_id, standing_patient, asserted_patient, content_address)
        VALUES (p_med, v_standing, p_patient, p_ca)
        ON CONFLICT (medication_id, standing_patient, asserted_patient, content_address)
            DO NOTHING;
        RETURN;
    END IF;
    RAISE EXCEPTION
        'medication thread %: patient cannot change — a medication_id belongs to one chart for life (standing %, asserted %; issue #192)',
        p_med, v_standing, p_patient;
END;
$$;

-- 4. Projection table: one row per asserted thread. Overlay columns (hlc/origin/
--    content_address) let a replayed/duplicate assert converge deterministically.
CREATE TABLE IF NOT EXISTS medication_statement (
    medication_id     UUID PRIMARY KEY,
    patient_id        UUID NOT NULL,
    term              TEXT NOT NULL,
    inn_code          TEXT,
    formulation       TEXT,
    dose_amount       TEXT,
    dose_unit         TEXT,
    sig               TEXT,
    info_source       TEXT NOT NULL,
    started_value     TEXT,
    started_precision TEXT,
    hlc_wall          BIGINT NOT NULL,
    hlc_counter       INTEGER NOT NULL,
    origin            TEXT NOT NULL,
    content_address   BYTEA NOT NULL,
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
GRANT SELECT ON medication_statement TO cairn_agent;
CREATE INDEX IF NOT EXISTS medication_statement_patient_idx ON medication_statement (patient_id);

-- 4b. The drug-identity coding projection (ADR-0059). A SEPARATE table, not columns on
--     medication_statement, for two reasons: one fact gets one home (slice 6b's coding
--     OVERLAY events write this same table under the same winner rule, so no reader ever
--     needs a precedence rule between two homes), and it keeps 6b purely additive — rows,
--     not rewritten view bodies. No FK to medication_statement: a coding may legitimately
--     arrive before the assert it codes (arrival-order independence, the same reason
--     medication_cessation is its own table).
CREATE TABLE IF NOT EXISTS medication_coding (
    medication_id   UUID PRIMARY KEY,
    patient_id      UUID NOT NULL,
    coding_system   TEXT NOT NULL,
    coding_code     TEXT NOT NULL,
    coding_display  TEXT NOT NULL,
    hlc_wall        BIGINT  NOT NULL,
    hlc_counter     INTEGER NOT NULL,
    origin          TEXT    NOT NULL,
    content_address BYTEA   NOT NULL,
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
GRANT SELECT ON medication_coding TO cairn_agent;
CREATE INDEX IF NOT EXISTS medication_coding_anchor_idx
    ON medication_coding (coding_system, coding_code);

-- 5. Fold clinical.medication.asserted into medication_statement. e.body is the
--    payload; patient_id is a column. Overlay-winner keeps set-union convergence.
--
-- The per-type trigger is superseded by cairn_projection_dispatch_trg (db/005,
-- ADR-0057); this fn is now registered in cairn_projection_apply below (#208).
DROP TRIGGER IF EXISTS medication_statement_apply_trg ON event_log;
-- The old zero-arg trigger-function signature is superseded by the (event_log)
-- apply-fn signature below; CREATE OR REPLACE cannot change a function's arg
-- list (it would overload, not replace), so drop the old signature explicitly —
-- without it an upgraded-in-place DB keeps BOTH the zero-arg trigger fn and its
-- trigger, double-firing every projection (ADR-0057).
DROP FUNCTION IF EXISTS medication_statement_apply();

CREATE OR REPLACE FUNCTION medication_statement_apply(e event_log)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    -- ADR-0052: sealed rows carry ciphertext in body; the clear payload lives
    -- in event_clear (populated by the door BEFORE this row, same txn). NULL =
    -- sealed without custody here: nothing to project — honest degradation.
    p jsonb := cairn_clear_payload(e);
BEGIN
    IF p IS NULL THEN RETURN; END IF;
    -- #192 thread patient-consistency: local fail-loud / remote converge-and-flag.
    -- Guarded HERE (not also in the dose-seed trigger that fires on this same event),
    -- so a remote contradiction is flagged exactly once per event.
    PERFORM cairn_guard_medication_patient(
        (p ->> 'medication_id')::uuid, e.patient_id, e.content_address);

    INSERT INTO medication_statement
        (medication_id, patient_id, term, inn_code, formulation,
         dose_amount, dose_unit, sig, info_source, started_value, started_precision,
         hlc_wall, hlc_counter, origin, content_address)
    VALUES (
        (p ->> 'medication_id')::uuid, e.patient_id,
        p -> 'substance' ->> 'term',
        p -> 'substance' ->> 'inn_code',
        p -> 'substance' ->> 'formulation',
        p -> 'dose' ->> 'amount',
        p -> 'dose' ->> 'unit',
        p ->> 'sig',
        p ->> 'info_source',
        p -> 'started' ->> 'value',
        p -> 'started' ->> 'precision',
        e.hlc_wall, e.hlc_counter, e.node_origin, e.content_address)
    ON CONFLICT (medication_id) DO UPDATE SET
        patient_id        = EXCLUDED.patient_id,
        term              = EXCLUDED.term,
        inn_code          = EXCLUDED.inn_code,
        formulation       = EXCLUDED.formulation,
        dose_amount       = EXCLUDED.dose_amount,
        dose_unit         = EXCLUDED.dose_unit,
        sig               = EXCLUDED.sig,
        info_source       = EXCLUDED.info_source,
        started_value     = EXCLUDED.started_value,
        started_precision = EXCLUDED.started_precision,
        hlc_wall          = EXCLUDED.hlc_wall,
        hlc_counter       = EXCLUDED.hlc_counter,
        origin            = EXCLUDED.origin,
        content_address   = EXCLUDED.content_address,
        updated_at        = clock_timestamp()
    WHERE cairn_hlc_overlay_wins(
        EXCLUDED.hlc_wall, EXCLUDED.hlc_counter, EXCLUDED.origin, EXCLUDED.content_address,
        medication_statement.hlc_wall, medication_statement.hlc_counter,
        medication_statement.origin, medication_statement.content_address);

    -- ADR-0059: the INLINE coding claim, when the author made one. Written only when
    -- present, so a later uncoded re-assertion can never silently clear a coding —
    -- retracting a coding is slice 6b's correction event, an authored act.
    --
    -- Absent (key never set) and an EXPLICIT JSON `"coding": null` are the SAME
    -- honest-unknown claim (db/041's cairn_check_medication_coding treats them
    -- identically) but they are NOT the same to plain `IS NOT NULL`: extracting a JSON
    -- null with `->` yields the jsonb value 'null', which IS DISTINCT FROM SQL NULL, so
    -- a bare `IS NOT NULL` guard here would still enter this branch for an explicit
    -- null and try to INSERT NULL into three NOT NULL columns. jsonb_typeof(...) =
    -- 'null' catches that shape explicitly.
    --
    -- patient_id is the thread's STANDING chart, NOT e.patient_id. #192 makes a
    -- medication_id belong to one chart for life, and the statement upsert immediately
    -- above has already settled which patient that is for this thread — including the
    -- case where THIS event contradicted it and LOST the overlay race (remote apply
    -- converges-and-flags rather than refusing, so a stale cross-patient re-assert still
    -- reaches this line). Taking e.patient_id verbatim would file the coding under the
    -- losing event's patient while the statement kept the standing one — two projections
    -- disagreeing about the thread's chart. The statement row always exists by now (the
    -- upsert above inserts unconditionally when absent), so this is never NULL.
    IF p -> 'substance' -> 'coding' IS NOT NULL
       AND jsonb_typeof(p -> 'substance' -> 'coding') IS DISTINCT FROM 'null' THEN
        INSERT INTO medication_coding
            (medication_id, patient_id, coding_system, coding_code, coding_display,
             hlc_wall, hlc_counter, origin, content_address)
        VALUES (
            (p ->> 'medication_id')::uuid,
            cairn_medication_thread_patient((p ->> 'medication_id')::uuid),
            p -> 'substance' -> 'coding' ->> 'system',
            p -> 'substance' -> 'coding' ->> 'code',
            p -> 'substance' -> 'coding' ->> 'display',
            e.hlc_wall, e.hlc_counter, e.node_origin, e.content_address)
        ON CONFLICT (medication_id) DO UPDATE SET
            patient_id      = EXCLUDED.patient_id,
            coding_system   = EXCLUDED.coding_system,
            coding_code     = EXCLUDED.coding_code,
            coding_display  = EXCLUDED.coding_display,
            hlc_wall        = EXCLUDED.hlc_wall,
            hlc_counter     = EXCLUDED.hlc_counter,
            origin          = EXCLUDED.origin,
            content_address = EXCLUDED.content_address,
            updated_at      = clock_timestamp()
        WHERE cairn_hlc_overlay_wins(
            EXCLUDED.hlc_wall, EXCLUDED.hlc_counter, EXCLUDED.origin, EXCLUDED.content_address,
            medication_coding.hlc_wall, medication_coding.hlc_counter,
            medication_coding.origin, medication_coding.content_address);
    END IF;
    RETURN;
END;
$$;
-- A trigger fn could never be called directly; a plain fn gets PUBLIC EXECUTE by
-- default. Same discipline as every privileged fn in db/005 (Task-1 review finding).
REVOKE EXECUTE ON FUNCTION medication_statement_apply(event_log) FROM PUBLIC;

-- 7. Cessation projection. A SEPARATE table (not an UPDATE of medication_statement)
--    makes the fold arrival-order-independent: an orphan cessation (assert not yet
--    local) lands here and the join lights up as 'past' only once the assert arrives.
CREATE TABLE IF NOT EXISTS medication_cessation (
    medication_id     UUID PRIMARY KEY,
    patient_id        UUID NOT NULL,
    stopped_value     TEXT,
    stopped_precision TEXT,
    reason            TEXT,
    hlc_wall          BIGINT NOT NULL,
    hlc_counter       INTEGER NOT NULL,
    origin            TEXT NOT NULL,
    content_address   BYTEA NOT NULL,
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
GRANT SELECT ON medication_cessation TO cairn_agent;

-- The per-type trigger is superseded by cairn_projection_dispatch_trg (db/005,
-- ADR-0057); this fn is now registered in cairn_projection_apply below (#208).
DROP TRIGGER IF EXISTS medication_cessation_apply_trg ON event_log;
-- The old zero-arg trigger-function signature is superseded by the (event_log)
-- apply-fn signature below; CREATE OR REPLACE cannot change a function's arg
-- list (it would overload, not replace), so drop the old signature explicitly —
-- without it an upgraded-in-place DB keeps BOTH the zero-arg trigger fn and its
-- trigger, double-firing every projection (ADR-0057).
DROP FUNCTION IF EXISTS medication_cessation_apply();

CREATE OR REPLACE FUNCTION medication_cessation_apply(e event_log)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    -- ADR-0052: sealed rows carry ciphertext in body; the clear payload lives
    -- in event_clear (populated by the door BEFORE this row, same txn). NULL =
    -- sealed without custody here: nothing to project — honest degradation.
    p jsonb := cairn_clear_payload(e);
BEGIN
    IF p IS NULL THEN RETURN; END IF;
    -- #192 thread patient-consistency (see the guard's comment above). An ORPHAN
    -- cessation (no standing claim at all) still passes — offline-first. NOTE this
    -- guard can ALSO insert into medication_patient_conflict_flag on a remote-apply
    -- conflict (same as the assert trigger above) — see this fn's registration row
    -- below, which lists that table too (a recipe/inventory-doc gap caught while
    -- converting this file: the original draft listed only medication_cessation).
    PERFORM cairn_guard_medication_patient(
        (p ->> 'medication_id')::uuid, e.patient_id, e.content_address);

    INSERT INTO medication_cessation
        (medication_id, patient_id, stopped_value, stopped_precision, reason,
         hlc_wall, hlc_counter, origin, content_address)
    VALUES (
        (p ->> 'medication_id')::uuid, e.patient_id,
        p -> 'stopped' ->> 'value',
        p -> 'stopped' ->> 'precision',
        p ->> 'reason',
        e.hlc_wall, e.hlc_counter, e.node_origin, e.content_address)
    ON CONFLICT (medication_id) DO UPDATE SET
        patient_id        = EXCLUDED.patient_id,
        stopped_value     = EXCLUDED.stopped_value,
        stopped_precision = EXCLUDED.stopped_precision,
        reason            = EXCLUDED.reason,
        hlc_wall          = EXCLUDED.hlc_wall,
        hlc_counter       = EXCLUDED.hlc_counter,
        origin            = EXCLUDED.origin,
        content_address   = EXCLUDED.content_address,
        updated_at        = clock_timestamp()
    WHERE cairn_hlc_overlay_wins(
        EXCLUDED.hlc_wall, EXCLUDED.hlc_counter, EXCLUDED.origin, EXCLUDED.content_address,
        medication_cessation.hlc_wall, medication_cessation.hlc_counter,
        medication_cessation.origin, medication_cessation.content_address);
    RETURN;
END;
$$;
-- A trigger fn could never be called directly; a plain fn gets PUBLIC EXECUTE by
-- default. Same discipline as every privileged fn in db/005 (Task-1 review finding).
REVOKE EXECUTE ON FUNCTION medication_cessation_apply(event_log) FROM PUBLIC;

-- 8. Unified list: statement LEFT JOIN cessation → status derived. An orphan
--    cessation (no matching statement) yields NO row here (nothing to render);
--    when the statement arrives, ceased flips true. Combines each statement
--    with its cessation (if any) into one list; every asserted thread appears
--    regardless of who asserted it.
--
--    `asserted_at` is derived from the assert event's HLC wall component
--    (`hlc_wall`, t_recorded in ms — db/001), NOT the local `updated_at`. This is
--    the *convergent* recording time: the same on every node that holds the event,
--    so the staleness signal (§3.3/ADR-0049 currency — a med asserted years ago shows its age)
--    is honest even on a node that only just replicated an old assert. `updated_at`
--    is a local-clock fold marker (reset on every overlay apply) and would make a
--    freshly-synced old med look new and diverge between nodes — wrong for display.
CREATE OR REPLACE VIEW patient_medication AS
SELECT s.medication_id, s.patient_id, s.term, s.inn_code, s.formulation,
       s.dose_amount, s.dose_unit, s.sig, s.info_source,
       s.started_value, s.started_precision,
       to_timestamp(s.hlc_wall / 1000.0) AS asserted_at,
       (c.medication_id IS NOT NULL) AS ceased,
       c.stopped_value, c.stopped_precision, c.reason,
       -- ADR-0059: appended at the END. inn_code stays, deprecated in place and read by
       -- nothing — dropping a view column would need a DROP VIEW, and a DROP is the
       -- non-additive move principle 11 forbids.
       mc.coding_system, mc.coding_code, mc.coding_display
FROM medication_statement s
LEFT JOIN medication_cessation c USING (medication_id)
LEFT JOIN medication_coding mc USING (medication_id);
GRANT SELECT ON patient_medication TO cairn_agent;

CREATE OR REPLACE VIEW patient_medication_current AS
SELECT medication_id, patient_id, term, inn_code, formulation,
       dose_amount, dose_unit, sig, info_source, started_value, started_precision, asserted_at,
       coding_system, coding_code, coding_display
FROM patient_medication WHERE NOT ceased;
GRANT SELECT ON patient_medication_current TO cairn_agent;

CREATE OR REPLACE VIEW patient_medication_past AS
SELECT medication_id, patient_id, term, inn_code, formulation,
       dose_amount, dose_unit, sig, info_source, started_value, started_precision,
       asserted_at, stopped_value, stopped_precision, reason,
       coding_system, coding_code, coding_display
FROM patient_medication WHERE ceased;
GRANT SELECT ON patient_medication_past TO cairn_agent;

-- 9. E1 reconciliation flag (advisory, never auto-merges). >=2 ACTIVE threads for one
--    patient sharing the dup-key. ADR-0059: the key is the coding PAIR when coded, else
--    the normalized term. The PAIR, never a bare code — once the reserved finer drugref
--    levels exist, the same substance coded at moiety level on one node and clinical-drug
--    level on another would split under a bare-code key (the same blind spot one level up,
--    and a CROSS-NODE one). Each branch is prefixed so a free-text term can never collide
--    with a code key. COLLATE "C" on both branches pins cross-node determinism (ADR-0045).
--    WHAT THIS CLOSES: coded<->coded, including Lipitor<->atorvastatin once BOTH are coded.
--    WHAT IT DOES NOT: coalesce picks per ROW, so a coded and an uncoded row still key
--    apart. That case closes when the uncoded member gets CODED (offered, never forced),
--    or later by term->anchor resolution in the drug-matcher slice.
CREATE OR REPLACE VIEW patient_medication_reconciliation_flag AS
SELECT patient_id,
       coalesce('code:' || (coding_system COLLATE "C") || '|' || (coding_code COLLATE "C"),
                'term:' || lower(btrim(term) COLLATE "C")) AS dup_key,
       count(*)                                            AS thread_count,
       array_agg(medication_id ORDER BY medication_id)     AS medication_ids
FROM patient_medication_current
GROUP BY patient_id,
         coalesce('code:' || (coding_system COLLATE "C") || '|' || (coding_code COLLATE "C"),
                  'term:' || lower(btrim(term) COLLATE "C"))
HAVING count(*) > 1;
GRANT SELECT ON patient_medication_reconciliation_flag TO cairn_agent;

-- 10. Registered apply fns for the #208/ADR-0057 generic dispatcher (db/005) +
--     cairn_reproject heal/rebuild (db/039). Both verbs' projection_tables lists
--     include medication_patient_conflict_flag: cairn_guard_medication_patient
--     (shared by both triggers, part 3b above) can insert a row there on a
--     remote-apply patient conflict — omitting it from either list would let a
--     narrow cairn_reproject rebuild truncate that table without knowing this
--     event type also feeds it (rebuild-scope metadata must be exhaustive, never
--     knowingly incomplete). #214 + steady-state discipline: converge these rows
--     to the migration text on every connect, but stay write-free once already
--     converged (no dead tuples, no validate-trigger fire).
INSERT INTO cairn_projection_apply AS r (event_type, apply_fn, projection_tables, run_order, heal_safe) VALUES
    ('clinical.medication.asserted',           'medication_statement_apply',
     ARRAY['medication_statement', 'medication_coding', 'medication_patient_conflict_flag'], 20, TRUE),
    ('clinical.medication-cessation.asserted', 'medication_cessation_apply',
     ARRAY['medication_cessation', 'medication_patient_conflict_flag'], 10, TRUE)
ON CONFLICT (event_type, apply_fn) DO UPDATE SET
    projection_tables = EXCLUDED.projection_tables,
    run_order         = EXCLUDED.run_order,
    heal_safe         = EXCLUDED.heal_safe
WHERE (r.projection_tables, r.run_order, r.heal_safe)
      IS DISTINCT FROM (EXCLUDED.projection_tables, EXCLUDED.run_order, EXCLUDED.heal_safe);

COMMIT;
