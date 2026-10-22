-- 059_medication_withhold.sql — the paper chart's "W": a time-bounded hold on a medication
-- line (ADR-0081, #803; data-model §3.3).
--
-- Two append-only verbs over an existing medication_id thread:
--   clinical.medication-withhold.asserted          — "do not give this from <from> until
--                                                     <until | reviewed>", with a reason
--   clinical.medication-withhold-release.asserted  — end the NAMED holds early (by withhold
--                                                     event id: only holds its author saw,
--                                                     and only on the thread it names)
--
-- WHAT IS STORED, AND WHAT IS NOT. Each event is its own row (no overlay, no HLC winner):
-- several holds can stand on one line at once, and a release ends only the ones it names.
-- Nothing here stores "withheld" as a fact — whether a hold is in force is a READ-TIME fold
-- against the node's clock (ADR-0081 decision 3), so a hold expires with no event and the
-- line simply resumes. Cessation still wins over any hold (decision 5), but NOT in this file:
-- the fold below returns holds on a ceased line too, and the READER (cairn-node
-- `medication/read.rs`) drops them, so any caller of cairn_medication_group_holds must filter
-- on the line's status itself.
--
-- WHERE THE REFUSALS LIVE (ADR-0081 decision 8). Two layers, deliberately split:
--   * STRUCTURAL — cairn_check_medication_withhold, run at BOTH doors (it is the registered
--     floor fn; at sync apply, wherever this node can open the body): a blank reason, a
--     missing/malformed `until`, a malformed `from`, `until` not after `from`, a hold with
--     no `from` and no `t_effective`, an empty/malformed `releases`, a null/blank/non-string
--     release `reason`, a release with no `t_effective`, a bad medication_id. A body like
--     that is malformed everywhere, so refusing it at sync apply forks nothing.
--   * CLINICAL STATE — in the apply fns, LOCAL DOOR ONLY (cairn.remote_apply not 'on'): a
--     withhold on a ceased line, a hold with `from` absent whose `until` is not after the
--     event's time, a release naming something that is not a withhold on this THREAD (plus
--     the shared #192 patient guard). These depend on
--     what THIS node happens to hold, so at sync apply they would be a node-local veto on a
--     validly-signed event — the fork ADR-0056 forbids. The fold is defined for every
--     arrival order instead (e.g. a release that arrives before its withhold).
--
-- Bodies are born sealed (ADR-0052), so the apply fns read cairn_clear_payload(e) and do
-- nothing when this node holds no custody (honest degradation, principle 4).
BEGIN;

-- 0. The thread index changed its predicate (db/034 now covers the withhold types). An
--    IF NOT EXISTS index never picks up an edited predicate, so db/034 creates it under the
--    new name _idx2 and the stale one is dropped here.
DROP INDEX IF EXISTS event_log_medication_thread_idx;

-- 1. Register both types in the fail-closed classification registry. Additive, never
--    targeting another author: a W is the withholder's own act on the thread and edits no
--    one else's prescription (ADR-0043).
INSERT INTO event_type_class (event_type, mode, targets_other_author) VALUES
    ('clinical.medication-withhold.asserted',         'additive', FALSE),
    ('clinical.medication-withhold-release.asserted', 'additive', FALSE)
ON CONFLICT (event_type) DO NOTHING;

-- 2. One hold instant. `from`/`until` are exact instants with an explicit UTC offset — the
--    cairn_t_effective wire pin (db/001), so the same signed event names the same instant on
--    every node whatever its TimeZone. This wrapper exists only to name the FIELD in the
--    refusal: cairn_t_effective's own message talks about "t_effective", which would send a
--    clinician looking at the wrong thing. NULL in → NULL out (an absent `from`); the string
--    "null" is refused here rather than read as absent (cairn_t_effective would return NULL
--    for it, silently turning a malformed field into "now").
CREATE OR REPLACE FUNCTION cairn_medication_hold_instant(p_raw text, p_field text)
RETURNS timestamptz LANGUAGE plpgsql STABLE AS $$
BEGIN
    IF p_raw IS NULL THEN
        RETURN NULL;
    END IF;
    IF p_raw = 'null' THEN
        RAISE EXCEPTION 'medication withhold: % must be an ISO-8601 instant with an explicit UTC offset, got "null"', p_field;
    END IF;
    BEGIN
        RETURN cairn_t_effective(p_raw);
    EXCEPTION WHEN others THEN
        -- DETAIL keeps the parser's own words, so a non-parse fault is not passed off as one.
        RAISE EXCEPTION 'medication withhold: % must be an ISO-8601 instant with an explicit UTC offset (e.g. 2026-10-21T20:00:00+10:00), got "%"', p_field, p_raw
            USING DETAIL = SQLERRM;
    END;
END;
$$;
REVOKE EXECUTE ON FUNCTION cairn_medication_hold_instant(text, text) FROM PUBLIC;

-- 3. The structural floor for both verbs (runs at BOTH doors — see the header). RAISE per
--    violation; the message prefix names the verb so a refusal is legible on its own.
CREATE OR REPLACE FUNCTION cairn_check_medication_withhold(p_type text, b jsonb)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    p        jsonb := b -> 'payload';
    v_verb   text  := CASE WHEN p_type = 'clinical.medication-withhold.asserted'
                           THEN 'medication withhold' ELSE 'medication release' END;
    v_from   timestamptz;
    v_until  timestamptz;
    v_el     jsonb;
BEGIN
    IF p IS NULL THEN
        RAISE EXCEPTION '%: missing payload', v_verb;
    END IF;
    -- medication_id is the thread key on BOTH verbs.
    IF jsonb_typeof(p -> 'medication_id') IS DISTINCT FROM 'string' THEN
        RAISE EXCEPTION '%: medication_id must be a uuid string', v_verb;
    END IF;
    BEGIN
        PERFORM (p ->> 'medication_id')::uuid;
    EXCEPTION WHEN others THEN
        RAISE EXCEPTION '%: medication_id must be a valid uuid', v_verb;
    END;

    IF p_type = 'clinical.medication-withhold.asserted' THEN
        -- A W without a reason is not a clinical act anyone can act on or review.
        IF jsonb_typeof(p -> 'reason') IS DISTINCT FROM 'string'
           OR length(btrim(p ->> 'reason')) = 0 THEN
            RAISE EXCEPTION 'medication withhold: reason must be a non-empty string';
        END IF;
        -- `until` is REQUIRED: an instant, or the explicit open end "reviewed". A missing
        -- field never stands in for "forever" (principle 4; ADR-0081 decision 2).
        IF jsonb_typeof(p -> 'until') IS DISTINCT FROM 'string' THEN
            RAISE EXCEPTION 'medication withhold: until is required — an instant with an explicit UTC offset, or "reviewed"';
        END IF;
        IF p ->> 'until' <> 'reviewed' THEN
            v_until := cairn_medication_hold_instant(p ->> 'until', 'until');
        END IF;
        -- `from` is optional, and optional fields are OMITTED, never null.
        IF p ? 'from' THEN
            IF jsonb_typeof(p -> 'from') IS DISTINCT FROM 'string' THEN
                RAISE EXCEPTION 'medication withhold: from, when given, must be an instant string (omit it for "from now")';
            END IF;
            v_from := cairn_medication_hold_instant(p ->> 'from', 'from');
        END IF;
        -- Instants compare as instants, whatever their offsets. An empty span is refused.
        -- (An absent `from` resolves to the event's time only in the apply fn, which is why
        -- that half of this rule is a local-door check there, not here.)
        IF v_from IS NOT NULL AND v_until IS NOT NULL AND v_until <= v_from THEN
            RAISE EXCEPTION 'medication withhold: until (%) must be after from (%)',
                p ->> 'until', p ->> 'from';
        END IF;
        -- With `from` absent the hold starts at the event's t_effective, so it must be there.
        -- The HLC wall is NO stand-in: a fast peer can push it up to 24 h ahead (db/020), and
        -- a "W from now" stamped with it would read as not started yet — the drug givable.
        -- Structural (it reads only the envelope), so it is the same verdict on every node.
        IF NOT (p ? 'from') AND jsonb_typeof(b -> 't_effective') IS DISTINCT FROM 'string' THEN
            RAISE EXCEPTION 'medication withhold: a hold without "from" must carry t_effective (the instant it starts)';
        END IF;
    ELSE
        -- A release names the holds it ends by withhold EVENT id: at least one, all uuids.
        -- Two IFs, not one OR: SQL does not promise left-to-right evaluation, and
        -- jsonb_array_length on a non-array raises its own, less legible error.
        IF jsonb_typeof(p -> 'releases') IS DISTINCT FROM 'array' THEN
            RAISE EXCEPTION 'medication release: releases must be a non-empty array of withhold event ids';
        END IF;
        IF jsonb_array_length(p -> 'releases') = 0 THEN
            RAISE EXCEPTION 'medication release: releases must be a non-empty array of withhold event ids';
        END IF;
        FOR v_el IN SELECT jsonb_array_elements(p -> 'releases') LOOP
            IF jsonb_typeof(v_el) IS DISTINCT FROM 'string' THEN
                RAISE EXCEPTION 'medication release: releases must hold withhold event ids as uuid strings';
            END IF;
            BEGIN
                PERFORM (v_el #>> '{}')::uuid;
            EXCEPTION WHEN others THEN
                RAISE EXCEPTION 'medication release: releases entry "%" is not a valid uuid', v_el #>> '{}';
            END;
        END LOOP;
        -- `reason` is optional: OMITTED, or a non-blank string — never null, blank or another
        -- JSON type (optional fields are omitted, never null).
        IF p ? 'reason' AND (jsonb_typeof(p -> 'reason') IS DISTINCT FROM 'string') THEN
            RAISE EXCEPTION 'medication release: reason, when given, must be a non-empty string (omit it otherwise)';
        END IF;
        IF p ? 'reason' AND length(btrim(p ->> 'reason')) = 0 THEN
            RAISE EXCEPTION 'medication release: reason, when given, must be a non-empty string (omit it otherwise)';
        END IF;
        -- A release ends its holds at its t_effective — never the HLC wall, for the mirror of
        -- the withhold's reason: a fast wall would leave the drug shown withheld for hours
        -- after the prescriber released it.
        IF jsonb_typeof(b -> 't_effective') IS DISTINCT FROM 'string' THEN
            RAISE EXCEPTION 'medication release: must carry t_effective (the instant the holds end)';
        END IF;
    END IF;
END;
$$;
-- PUBLIC holds EXECUTE by default; the cairn_check_* family is revoked uniformly (#382).
REVOKE EXECUTE ON FUNCTION cairn_check_medication_withhold(text, jsonb) FROM PUBLIC;

-- 4. Register both verbs' floor + hard twin requirement in the #173 registry (after the fn,
--    so the fail-closed registry trigger sees it declared). Converge-on-replay (#214), and
--    write-free once converged.
INSERT INTO cairn_event_twin_check (event_type, check_fn, twin_required_msg) VALUES
    ('clinical.medication-withhold.asserted',         'cairn_check_medication_withhold', 'medication withhold requires a non-empty authored twin (§3.13/§3.3)'),
    ('clinical.medication-withhold-release.asserted', 'cairn_check_medication_withhold', 'medication withhold release requires a non-empty authored twin (§3.13/§3.3)')
ON CONFLICT (event_type) DO UPDATE SET
    check_fn          = EXCLUDED.check_fn,
    twin_required_msg = EXCLUDED.twin_required_msg
WHERE (cairn_event_twin_check.check_fn, cairn_event_twin_check.twin_required_msg)
      IS DISTINCT FROM (EXCLUDED.check_fn, EXCLUDED.twin_required_msg);

-- 5. Projection tables. One row PER EVENT (content_address PK, insert-or-nothing): there is
--    no winner to pick between two holds, and a replay of the same event converges. No FK to
--    medication_statement: a withhold may arrive before the assert it names (offline-first).
--
--    starts_at is RESOLVED at apply: the payload's `from`, else the event's t_effective (the
--    floor refuses a hold with neither). until_at is NULL exactly when the hold is "until
--    reviewed".
CREATE TABLE IF NOT EXISTS medication_withhold (
    content_address BYTEA       PRIMARY KEY,
    event_id        UUID        NOT NULL UNIQUE,
    medication_id   UUID        NOT NULL,
    patient_id      UUID        NOT NULL,
    reason          TEXT        NOT NULL,
    starts_at       TIMESTAMPTZ NOT NULL,
    until_at        TIMESTAMPTZ,
    until_reviewed  BOOLEAN     NOT NULL,
    withholder_kid  TEXT        NOT NULL,   -- the signer: the human's key when authored (ADR-0053)
    hlc_wall        BIGINT      NOT NULL,
    hlc_counter     INTEGER     NOT NULL,
    origin          TEXT        NOT NULL,
    CHECK (until_reviewed = (until_at IS NULL))
);
GRANT SELECT ON medication_withhold TO cairn_agent;
CREATE INDEX IF NOT EXISTS medication_withhold_medication_idx ON medication_withhold (medication_id);

CREATE TABLE IF NOT EXISTS medication_withhold_release (
    content_address    BYTEA       PRIMARY KEY,
    event_id           UUID        NOT NULL UNIQUE,
    medication_id      UUID        NOT NULL,
    patient_id         UUID        NOT NULL,
    released_event_ids UUID[]      NOT NULL CHECK (cardinality(released_event_ids) > 0),
    released_at        TIMESTAMPTZ NOT NULL,
    reason             TEXT,
    hlc_wall           BIGINT      NOT NULL,
    hlc_counter        INTEGER     NOT NULL,
    origin             TEXT        NOT NULL
);
GRANT SELECT ON medication_withhold_release TO cairn_agent;
CREATE INDEX IF NOT EXISTS medication_withhold_release_medication_idx
    ON medication_withhold_release (medication_id);

-- 6. The thread's standing patient claim (#192) gains a FOURTH arm for this file's table: an
--    orphan withhold (its assert not yet here) claims the thread for its chart, exactly as an
--    orphan cessation or coding does — otherwise a later assert naming a DIFFERENT patient
--    would pass cairn_guard_medication_patient and leave medication_statement and
--    medication_withhold disagreeing about the chart. The arm lives in db/031's single
--    definition of cairn_medication_thread_patient (to_regclass-guarded there), NOT in a
--    redefinition here: a second CREATE OR REPLACE would mean every replay briefly runs
--    db/031's body without the arm.

-- 7. Fold a withhold into its row. Local-door state refusals first (see the header), then
--    an insert-or-nothing.
CREATE OR REPLACE FUNCTION medication_withhold_apply(e event_log)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    -- ADR-0052: the clear payload, or NULL when this node holds no custody.
    p          jsonb := cairn_clear_payload(e);
    v_med      uuid;
    v_reviewed boolean;
    v_starts   timestamptz;
    v_until    timestamptz;
BEGIN
    IF p IS NULL THEN RETURN; END IF;
    v_med := (p ->> 'medication_id')::uuid;
    -- #192 thread patient-consistency: local fail-loud / remote converge-and-flag.
    PERFORM cairn_guard_medication_patient(v_med, e.patient_id, e.content_address);

    v_reviewed := (p ->> 'until') = 'reviewed';
    IF NOT v_reviewed THEN
        v_until := cairn_medication_hold_instant(p ->> 'until', 'until');
    END IF;
    -- Never the HLC wall: the floor refused a hold with neither (see the check fn).
    v_starts := COALESCE(cairn_medication_hold_instant(p ->> 'from', 'from'), e.t_effective);

    IF current_setting('cairn.remote_apply', true) IS DISTINCT FROM 'on' THEN
        -- The LINE's status, not the thread's: a reconciled group with a ceased member and a
        -- later active one reads active (ADR-0047 latest-effective-wins), and a W on it is
        -- admitted. An orphan thread (no statement here) has no group row, so it passes.
        IF EXISTS (SELECT 1 FROM medication_thread_group g
                     JOIN medication_group_status s ON s.group_id = g.group_id
                    WHERE g.medication_id = v_med AND s.status = 'ceased') THEN
            RAISE EXCEPTION 'medication withhold: this line is ceased — nothing to withhold';
        END IF;
        -- With `from` absent the start is the event's own time, so an `until` already in the
        -- past makes a hold that is over before it begins.
        IF v_until IS NOT NULL AND v_until <= v_starts THEN
            RAISE EXCEPTION 'medication withhold: until (%) must be after the hold''s start (%)',
                p ->> 'until', v_starts;
        END IF;
    END IF;

    INSERT INTO medication_withhold
        (content_address, event_id, medication_id, patient_id, reason, starts_at, until_at,
         until_reviewed, withholder_kid, hlc_wall, hlc_counter, origin)
    VALUES (e.content_address, e.event_id, v_med, e.patient_id, p ->> 'reason', v_starts,
            v_until, v_reviewed, e.signer_key_id, e.hlc_wall, e.hlc_counter, e.node_origin)
    ON CONFLICT (content_address) DO NOTHING;
END;
$$;
REVOKE EXECUTE ON FUNCTION medication_withhold_apply(event_log) FROM PUBLIC;

-- 8. Fold a release into its row. Local door: every named id must be a withhold on THIS
--    THREAD (a release can never end another thread's hold, nor one its author did not see).
--    The list shows holds per LINE (a reconciled group), so when the id IS a withhold but on
--    a sibling thread, the refusal names that thread: "release it there".
--    At sync apply a release may legitimately arrive before the withhold it names; the
--    fold ends that hold once it arrives (ADR-0081 decision 4).
CREATE OR REPLACE FUNCTION medication_withhold_release_apply(e event_log)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    p     jsonb := cairn_clear_payload(e);
    v_med uuid;
    v_ids uuid[];
    v_id  uuid;
    v_on  uuid;   -- the thread a named withhold is on; NULL when it is no withhold here
BEGIN
    IF p IS NULL THEN RETURN; END IF;
    v_med := (p ->> 'medication_id')::uuid;
    PERFORM cairn_guard_medication_patient(v_med, e.patient_id, e.content_address);
    v_ids := ARRAY(SELECT x::uuid FROM jsonb_array_elements_text(p -> 'releases') AS x);

    IF current_setting('cairn.remote_apply', true) IS DISTINCT FROM 'on' THEN
        FOREACH v_id IN ARRAY v_ids LOOP
            v_on := (SELECT w.medication_id FROM medication_withhold w WHERE w.event_id = v_id);
            IF v_on IS NULL THEN
                RAISE EXCEPTION 'medication release: % is not a withhold on this thread', v_id;
            ELSIF v_on <> v_med THEN
                RAISE EXCEPTION 'medication release: % is a withhold on thread %, not %: release it there',
                    v_id, v_on, v_med;
            END IF;
        END LOOP;
    END IF;

    INSERT INTO medication_withhold_release
        (content_address, event_id, medication_id, patient_id, released_event_ids,
         released_at, reason, hlc_wall, hlc_counter, origin)
    VALUES (e.content_address, e.event_id, v_med, e.patient_id, v_ids,
            e.t_effective,  -- never the HLC wall: the floor refused a release without one
            p ->> 'reason', e.hlc_wall, e.hlc_counter, e.node_origin)
    ON CONFLICT (content_address) DO NOTHING;
END;
$$;
REVOKE EXECUTE ON FUNCTION medication_withhold_release_apply(event_log) FROM PUBLIC;

-- 9. Register both apply fns with the generic dispatcher (db/005, ADR-0057) and
--    cairn_reproject (db/039). Each list names medication_patient_conflict_flag too: the
--    shared patient guard can write there on a remote-apply conflict, and rebuild-scope
--    metadata must be exhaustive. run_order 10 matches the cessation's; it orders apply fns
--    WITHIN one event type, and each type here has just one. Converge-on-replay, write-free
--    once converged (#214).
INSERT INTO cairn_projection_apply AS r (event_type, apply_fn, projection_tables, run_order, heal_safe) VALUES
    ('clinical.medication-withhold.asserted',         'medication_withhold_apply',
     ARRAY['medication_withhold', 'medication_patient_conflict_flag'], 10, TRUE),
    ('clinical.medication-withhold-release.asserted', 'medication_withhold_release_apply',
     ARRAY['medication_withhold_release', 'medication_patient_conflict_flag'], 10, TRUE)
ON CONFLICT (event_type, apply_fn) DO UPDATE SET
    projection_tables = EXCLUDED.projection_tables,
    run_order         = EXCLUDED.run_order,
    heal_safe         = EXCLUDED.heal_safe
WHERE (r.projection_tables, r.run_order, r.heal_safe)
      IS DISTINCT FROM (EXCLUDED.projection_tables, EXCLUDED.run_order, EXCLUDED.heal_safe);

-- 10. The read-time fold: which holds are on these medication lines at instant `p_now`?
--
--     A withhold is a time-bounded HOLD, not a state that is stored: whether it is still in
--     force is a question about the clock, so it is answered when it is ASKED. Nothing here is
--     ever written back (append-only: a hold that has run its course is not "closed" by any
--     event, it simply stops being returned).
--
--     Why `p_now` is a parameter rather than now(): the same inputs must always give the same
--     answer. It makes the fold testable at pinned instants, lets a caller evaluate a whole
--     screen at ONE instant (two calls a second apart must not disagree), and lets a reviewer
--     ask "what did the chart say at 14:00?" without a time machine. STABLE (not VOLATILE)
--     because it only reads.
--
--     A withhold is RETURNED when BOTH hold:
--       * it has not run out:  `until_reviewed` (an open hold has no clock) OR `until_at > p_now`.
--         The comparison is STRICT: `until` is the first instant the hold is over, so at
--         exactly `until` it is already gone. (timestamptz compares as an instant, so
--         "+10:00" and "Z" spellings of one moment are equal.)
--       * no release ended it: no medication_withhold_release row for the SAME thread names
--         this withhold's event id in `released_event_ids` with `released_at <= p_now`. `<=`
--         because a release made at `p_now` has already happened. A release that arrived by
--         sync BEFORE the withhold it names works too: the two tables are independent and
--         this join only meets them at read time.
--
--     An EMPTY span (until <= the resolved start: refused at the local door, but a peer's can
--     be admitted) is never in force, so it is never returned either.
--
--     `pending` = `starts_at > p_now`: an advance hold ("NBM from 06:00 tomorrow") is RETURNED
--     so the chart can show it coming, flagged so a caller never treats it as in force. A
--     hold is IN FORCE exactly when it is returned and not pending. Strict `>` again:
--     at `starts_at` the hold has begun.
--
--     Lines, not threads: a withhold is filed on one thread, but a reconciled group is ONE
--     drug, so a hold on ANY member shows under the group id (medication_thread_group,
--     db/033). Separating the group moves the view, not the hold. An orphan withhold (its
--     assert not yet synced) is on no line yet and is not returned — it has nothing to hold.
--
--     Order is for display stability only: group, newest start first, event id to break ties.
CREATE OR REPLACE FUNCTION cairn_medication_group_holds(p_groups uuid[], p_now timestamptz)
RETURNS TABLE (group_id uuid, medication_id uuid, withhold_event_id uuid, reason text,
               starts_at timestamptz, until_at timestamptz, until_reviewed boolean,
               withholder_kid text, pending boolean)
LANGUAGE sql STABLE AS $$
    SELECT g.group_id, w.medication_id, w.event_id, w.reason,
           w.starts_at, w.until_at, w.until_reviewed, w.withholder_kid,
           w.starts_at > p_now
    FROM medication_withhold w
    JOIN medication_thread_group g ON g.medication_id = w.medication_id
    WHERE g.group_id = ANY (p_groups)
      AND (w.until_reviewed OR w.until_at > p_now)
      AND (w.until_reviewed OR w.until_at > w.starts_at)
      AND NOT EXISTS (
            SELECT 1 FROM medication_withhold_release r
            WHERE r.medication_id = w.medication_id
              AND w.event_id = ANY (r.released_event_ids)
              AND r.released_at <= p_now)
    ORDER BY g.group_id, w.starts_at DESC, w.event_id
$$;
REVOKE EXECUTE ON FUNCTION cairn_medication_group_holds(uuid[], timestamptz) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION cairn_medication_group_holds(uuid[], timestamptz) TO cairn_agent;

-- 11. #846 — every dose, dose correction or release whose chart is not its thread's chart.
--
--    db/031's guard (`cairn_guard_medication_patient`) flags a synced event that names another
--    chart, but only when the thread's chart is ALREADY known when the event arrives. These
--    three event kinds never establish a thread's chart (they are not arms of
--    `cairn_medication_thread_patient`), so one that arrives FIRST passes unflagged, and the
--    claim that arrives later never looks back at it: another person's dose or W-ending would
--    sit on this line with nothing to say so. A release may legitimately arrive before its W
--    (section 8), and a dose before its assert, so this is a reachable order, not a corrupt one.
--
--    A VIEW, read at list time, for the reason db/034's `medication_attestation_misfiled` is
--    one: a read-time join sees the contradiction whichever event arrived second, and after any
--    reproject in any order. The medication list reads it beside the flag table (`read.rs`
--    `read_conflict_charts`). A thread whose chart is unknown here contributes nothing: no data
--    is never disagreement (principle 4). Columns are named as the flag table's, so the two
--    union directly. A dose correction row is the one that currently stands for its target (HLC
--    wins, db/032 section 6), so a stray correction overtaken by a correct one leaves no row here;
--    the flag table still records it if it arrived after the claim.
--
--    Defined here, not in db/031, because it reads db/059's own release table.
CREATE OR REPLACE VIEW medication_thread_stray_chart AS
SELECT s.medication_id,
       tp.patient_id AS standing_patient,
       s.patient_id  AS asserted_patient
FROM (
    SELECT medication_id, patient_id FROM medication_dose_event
    UNION ALL
    SELECT medication_id, patient_id FROM medication_dose_correction
    UNION ALL
    SELECT medication_id, patient_id FROM medication_withhold_release
) s
CROSS JOIN LATERAL (
    SELECT cairn_medication_thread_patient(s.medication_id) AS patient_id
) tp
WHERE tp.patient_id IS NOT NULL
  AND tp.patient_id <> s.patient_id;
GRANT SELECT ON medication_thread_stray_chart TO cairn_agent;
-- The list filters this view by thread; the dose and release tables already index
-- medication_id, and this gives the correction table the same (otherwise a seq scan per read).
CREATE INDEX IF NOT EXISTS medication_dose_correction_med_idx
    ON medication_dose_correction (medication_id);

COMMIT;
