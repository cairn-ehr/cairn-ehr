-- 042_medication_coding_overlay.sql — coding as a separately-authored act (ADR-0059
-- decision 3), slice 6b of clinical.medication (data-model §3.3).
--
-- Slice 6a (db/041) shipped only INLINE coding on the assertion, so a medication recorded
-- uncoded could never become coded and a wrong coding could never be repaired. Two overlay
-- verbs close that:
--   clinical.medication-coding.asserted             — code a thread not coded inline
--   clinical.medication-coding-correction.asserted  — replace the claim, or STRIKE it
--
-- WHY A STRIKE EXISTS: a reviewer who establishes a medication is NOT metformin but cannot
-- say what it is has, without one, only two options — leave a known-wrong anchor standing
-- (it keeps feeding the dup-key and the group display), or invent a substitute identity
-- they cannot vouch for. The second is the fabrication principle 4 forbids. Append-only
-- means the correction event is the only repair path, so it must be able to say
-- "not that, and I don't know."
--
-- WHY A SEPARATE FILE (not an edit to db/041): SCHEMA_GENERATION is derived from the newest
-- db/ prefix and this is a FLOOR change. Issue #188 exists so an older binary cannot
-- CREATE OR REPLACE a newer safety check back down; an in-place edit of db/041 alone could
-- not bump the generation and would leave that downgrade silent.
BEGIN;

-- 1. Both types are ADDITIVE and do not target another author. This matters: a coding
--    correction supersedes a claim that may have been authored by someone else, but it
--    ADDS a claim rather than suppressing one — the original stays in the log and the
--    projection picks a winner by HLC. Registering targets_other_author = TRUE would route
--    these through the ADR-0043 suppression owner-gate, which would refuse a pharmacist
--    correcting a coding authored by a different coder — contradicting the premise of
--    ADR-0059 decision 3. Same classification as
--    clinical.medication-dose-correction.asserted (db/032).
--    #214/#254: converge on replay via DO UPDATE, write-free once converged.
INSERT INTO event_type_class AS r (event_type, mode, targets_other_author) VALUES
    ('clinical.medication-coding.asserted',            'additive', FALSE),
    ('clinical.medication-coding-correction.asserted', 'additive', FALSE)
ON CONFLICT (event_type) DO UPDATE SET
    mode                 = EXCLUDED.mode,
    targets_other_author = EXCLUDED.targets_other_author
WHERE (r.mode, r.targets_other_author)
      IS DISTINCT FROM (EXCLUDED.mode, EXCLUDED.targets_other_author);

-- 2. The overlay floor: medication_id on both verbs; corrects + the coding/strike
--    exclusivity on the correction; the coding TRIPLE delegated to db/041's shared
--    cairn_check_coding_object so the inline and overlay paths cannot drift on what a valid
--    coding claim is (two tiers, canonical-uuid pin, strict-submit/lenient-apply — all
--    inherited, none restated).
CREATE OR REPLACE FUNCTION cairn_check_medication_coding_overlay(p_type text, b jsonb)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    p          jsonb := b -> 'payload';
    v_has_code boolean;
    v_strike   boolean;
BEGIN
    IF p IS NULL THEN
        RAISE EXCEPTION 'medication coding: missing payload';
    END IF;
    IF jsonb_typeof(p -> 'medication_id') IS DISTINCT FROM 'string'
       OR NOT pg_input_is_valid(p ->> 'medication_id', 'uuid') THEN
        RAISE EXCEPTION 'medication coding: medication_id must be a valid uuid string';
    END IF;

    IF p_type = 'clinical.medication-coding.asserted' THEN
        -- A plain coding overlay must actually carry a coding. On the ASSERTION an absent
        -- coding is the honest not-yet-coded floor (principle 4); here it is incoherent —
        -- an event whose whole purpose is to code something has nothing to say without
        -- one. Structural, so refused at BOTH doors: no registry judgment is involved.
        IF p -> 'coding' IS NULL OR jsonb_typeof(p -> 'coding') = 'null' THEN
            RAISE EXCEPTION 'medication coding: coding is required on a coding overlay (to un-code, use clinical.medication-coding-correction.asserted with strike)';
        END IF;
        PERFORM cairn_check_coding_object(p -> 'coding', 'medication coding: coding');
        RETURN;
    END IF;

    -- The correction verb.
    IF jsonb_typeof(p -> 'corrects') IS DISTINCT FROM 'string'
       OR NOT pg_input_is_valid(p ->> 'corrects', 'uuid') THEN
        RAISE EXCEPTION 'medication coding-correction: corrects must be a valid uuid string';
    END IF;
    -- The target's EXISTENCE is deliberately NOT required: the corrected event may
    -- replicate later, or never. Refusing an unknown target would make a correction
    -- impossible on a node that has not yet received the coding it fixes (offline-first;
    -- the same contract db/032's dose-correction floor states in as many words).
    --
    -- jsonb_typeof(...) = 'null' is checked because an explicit JSON `"coding": null` reads
    -- as the jsonb value 'null', not SQL NULL — treating it as absent keeps a peer whose
    -- serializer emits explicit nulls from having a verifiable event refused over an
    -- encoding style choice (db/041's argument, same trap).
    v_has_code := p -> 'coding' IS NOT NULL AND jsonb_typeof(p -> 'coding') IS DISTINCT FROM 'null';

    -- `strike` is pinned to a JSON BOOLEAN, absent-or-boolean and nothing else.
    -- `(p ->> 'strike')::boolean` would have inherited Postgres's permissive boolean input
    -- syntax instead of stating a rule: `1`, `"true"` and `"yes"` would all strike a
    -- coding, while `"banana"` would fail with a raw `invalid input syntax for type
    -- boolean` naming no field. Both directions are wrong for the one bit that decides
    -- whether a drug identity is retracted — a peer whose serializer stringifies booleans
    -- would author strikes this node never agreed to accept, and the spelling is permanent
    -- once frozen into a signed body (db/041's canonical-uuid argument, applied to a
    -- boolean). Structural, so this refuses at BOTH doors: it is a shape judgment, not a
    -- registry one, exactly like substance.term.
    IF p -> 'strike' IS NOT NULL
       AND jsonb_typeof(p -> 'strike') NOT IN ('boolean', 'null') THEN
        RAISE EXCEPTION 'medication coding-correction: strike must be a JSON boolean, got % (%)',
            jsonb_typeof(p -> 'strike'), p -> 'strike';
    END IF;
    v_strike := coalesce((p -> 'strike')::boolean, FALSE);

    IF v_has_code AND v_strike THEN
        RAISE EXCEPTION 'medication coding-correction: a correction cannot both replace and strike — carry a coding OR strike, not both';
    END IF;
    IF NOT v_has_code AND NOT v_strike THEN
        RAISE EXCEPTION 'medication coding-correction: a correction must carry a replacement coding or strike = true (an omitted coding must never silently un-code a medication)';
    END IF;
    IF v_has_code THEN
        PERFORM cairn_check_coding_object(p -> 'coding', 'medication coding-correction: coding');
    END IF;
END;
$$;
REVOKE EXECUTE ON FUNCTION cairn_check_medication_coding_overlay(text, jsonb) FROM PUBLIC;

-- 3. Register both verbs' floor + hard twin requirement (the ADR-0048 registry). Placed
--    AFTER the function so db/005's fail-closed registration trigger sees it declared.
INSERT INTO cairn_event_twin_check AS r (event_type, check_fn, twin_required_msg) VALUES
    ('clinical.medication-coding.asserted',            'cairn_check_medication_coding_overlay',
     'medication coding requires a non-empty authored twin (§3.13/§3.3)'),
    ('clinical.medication-coding-correction.asserted', 'cairn_check_medication_coding_overlay',
     'medication coding correction requires a non-empty authored twin (§3.13/§3.3)')
ON CONFLICT (event_type) DO UPDATE SET
    check_fn          = EXCLUDED.check_fn,
    twin_required_msg = EXCLUDED.twin_required_msg
WHERE (r.check_fn, r.twin_required_msg)
      IS DISTINCT FROM (EXCLUDED.check_fn, EXCLUDED.twin_required_msg);

-- 4. A struck coding needs a row that says "deliberately not coded" rather than no row at
--    all. Deleting the row would break arrival-order independence: a coding event arriving
--    AFTER the strike with a LOWER HLC would have nothing to lose the overlay race
--    against, and would silently win. So the anchor columns become nullable and the row
--    carries a flag.
--
--    Dropping NOT NULL is a WIDENING (every existing row still satisfies the looser
--    constraint), and the ADD COLUMN is the #207 paired-ALTER an upgraded-in-place database
--    needs — db/031's `CREATE TABLE IF NOT EXISTS` is a silent no-op there, so a column or
--    constraint introduced after the table has ever been created can only arrive this way.
ALTER TABLE medication_coding ALTER COLUMN coding_system  DROP NOT NULL;
ALTER TABLE medication_coding ALTER COLUMN coding_code    DROP NOT NULL;
ALTER TABLE medication_coding ALTER COLUMN coding_display DROP NOT NULL;

--    `struck` is GENERATED, not written (PR-review finding 1). It says exactly "this thread
--    has no drug identity", which is definitionally `coding_code IS NULL` — the floor
--    admits a coding triple only all-three-or-none, so a NULL anchor can arise no other way
--    than a strike. Storing that bit separately made it a THIRD thing three different apply
--    fns each had to remember to keep in step, and one of them did not: db/031's INLINE
--    coding upsert (`clinical.medication.asserted` writes this table too) names the anchor
--    columns and not `struck`, so an inline coding that WON the HLC race over an
--    earlier-arriving strike left a live anchor beside a stale `struck = TRUE`.
--
--    That is arrival-order dependence, the one thing set-union sync cannot tolerate: node A
--    asserts an inline coding, node B strikes it at a lower HLC while offline, both nodes
--    end up holding both events and the assertion wins on both — but B applied the strike
--    first and A did not, so two honest nodes read a `cairn_agent`-readable column
--    differently (the ADR-0045 class, same shape as the #295 collation hazard).
--    A generated column removes the writer entirely: no apply fn can set it, so no apply fn
--    can forget it, and a fourth writer arriving in a later slice inherits the invariant for
--    free. Deliberately NOT a CHECK constraint — a violated CHECK would abort the projection
--    apply and wedge that event forever, and manufacturing a new one-event sync-wedge (the
--    hazard ADR-0058 closed) to police a redundant bit would be a bad trade.
--
--    The DROP-then-ADD converts a database that already loaded an earlier build of THIS
--    unmerged migration, where `struck` landed as a plain column. It is guarded on
--    attgenerated so it runs at most once — the migrations re-run on every connect, and an
--    unguarded DROP/ADD would rewrite the table every time.
--
--    The coder worklist (part 8) reads `struck`, so on such a database it must go first;
--    part 8 recreates it a few statements below, inside this same transaction, so there is
--    no window where it is missing. Dropped BY NAME rather than with CASCADE deliberately:
--    a future dependent nobody thought about must break this migration loudly instead of
--    being silently dropped and re-created only if someone remembered to.
DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_attribute
                    WHERE attrelid = 'medication_coding'::regclass
                      AND attname  = 'struck'
                      AND attgenerated = 's') THEN
        DROP VIEW IF EXISTS patient_medication_uncoded;
        ALTER TABLE medication_coding DROP COLUMN IF EXISTS struck;
        ALTER TABLE medication_coding ADD COLUMN struck BOOLEAN NOT NULL
            GENERATED ALWAYS AS (coding_code IS NULL) STORED;
    END IF;
END $$;

-- 5. Apply the plain coding overlay. Same table, same winner rule as the INLINE coding
--    (db/031 part 5) — that is what makes this slice additive: no view is re-routed, and
--    every consumer of medication_coding (the widened read views, the (system, code)
--    dup-key, the prefer-coded group display, the anchor-conflict view) keeps working.
CREATE OR REPLACE FUNCTION medication_coding_apply(e event_log)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    -- ADR-0052: sealed rows carry ciphertext in body; the clear payload lives in
    -- event_clear (populated by the door BEFORE this row, same txn). NULL = sealed without
    -- custody here: nothing to project — honest degradation.
    p     jsonb := cairn_clear_payload(e);
    v_med uuid;
BEGIN
    IF p IS NULL THEN RETURN; END IF;
    v_med := (p ->> 'medication_id')::uuid;
    -- #192: a coding event must not silently re-home a thread onto another chart.
    -- Local door RAISEs; remote apply converges-and-flags (never refuses — ADR-0056).
    PERFORM cairn_guard_medication_patient(v_med, e.patient_id, e.content_address);

    -- `struck` is absent from both lists on purpose: it is GENERATED from coding_code
    -- (part 4), so it cannot be written and cannot be forgotten.
    INSERT INTO medication_coding
        (medication_id, patient_id, coding_system, coding_code, coding_display,
         hlc_wall, hlc_counter, origin, content_address)
    VALUES (
        v_med,
        -- The thread's STANDING chart when one is known (#192: the same discipline db/031's
        -- inline write follows, so a contradicting event that LOST the overlay race cannot
        -- file the coding under its own losing patient), else this event's own claim — an
        -- overlay may arrive BEFORE the assert it codes, when no standing chart exists yet
        -- and patient_id is NOT NULL. cairn_medication_thread_patient reads this table too,
        -- so that fallback claim is what a later assert is checked against.
        coalesce(cairn_medication_thread_patient(v_med), e.patient_id),
        p -> 'coding' ->> 'system',
        p -> 'coding' ->> 'code',
        p -> 'coding' ->> 'display',
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
    RETURN;
END;
$$;
REVOKE EXECUTE ON FUNCTION medication_coding_apply(event_log) FROM PUBLIC;

-- 6. Apply a correction: a replacement writes the new triple, a strike writes NULLs — and
--    `struck` follows from the NULL anchor on its own (part 4), never written here. The NULL
--    anchor is also what makes the downstream degradation automatic: the E1 dup-key's
--    coalesce falls back to the term branch by itself ('code:' || NULL is NULL in SQL), and
--    the anchor-conflict view excludes an anchor-less member outright (db/033).
CREATE OR REPLACE FUNCTION medication_coding_correction_apply(e event_log)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    p        jsonb := cairn_clear_payload(e);
    v_med    uuid;
    v_struck boolean;
BEGIN
    IF p IS NULL THEN RETURN; END IF;
    v_med := (p ->> 'medication_id')::uuid;
    PERFORM cairn_guard_medication_patient(v_med, e.patient_id, e.content_address);
    -- The floor guarantees exactly one of coding / strike, so this is a clean either-or.
    -- jsonb_typeof(...) = 'null' is checked because an explicit JSON null reads as the
    -- jsonb value 'null', not SQL NULL — the same trap db/031's inline write documents, and
    -- the reason a bare IS NULL test here would try to write a triple of SQL NULLs for a
    -- REPLACEMENT, silently turning it into a strike.
    v_struck := p -> 'coding' IS NULL OR jsonb_typeof(p -> 'coding') = 'null';

    INSERT INTO medication_coding
        (medication_id, patient_id, coding_system, coding_code, coding_display,
         hlc_wall, hlc_counter, origin, content_address)
    VALUES (
        v_med,
        coalesce(cairn_medication_thread_patient(v_med), e.patient_id),
        CASE WHEN v_struck THEN NULL ELSE p -> 'coding' ->> 'system'  END,
        CASE WHEN v_struck THEN NULL ELSE p -> 'coding' ->> 'code'    END,
        CASE WHEN v_struck THEN NULL ELSE p -> 'coding' ->> 'display' END,
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
    RETURN;
END;
$$;
REVOKE EXECUTE ON FUNCTION medication_coding_correction_apply(event_log) FROM PUBLIC;

-- 7. Register both apply fns with the ADR-0057 dispatcher.
--    medication_patient_conflict_flag is in both inventories because
--    cairn_guard_medication_patient can write it on a remote-apply conflict — rebuild-scope
--    metadata must be exhaustive, never knowingly incomplete.
--    NOTE: medication_coding is now written by THREE event types (the assertion's inline
--    coding plus these two), so cairn_reproject will refuse a narrow single-type prefix
--    rebuild over it (db/039) — correct, and precisely the reason that refusal exists: a
--    narrow rebuild would truncate the other types' rows.
--    run_order 10 is the single-fn default: the dispatcher orders only WITHIN one
--    event_type (db/005's FOR loop filters on event_type first), and each of these types
--    has exactly one apply fn — db/031's 20 exists solely because
--    clinical.medication.asserted carries two.
INSERT INTO cairn_projection_apply AS r (event_type, apply_fn, projection_tables, run_order, heal_safe) VALUES
    ('clinical.medication-coding.asserted',            'medication_coding_apply',
     ARRAY['medication_coding', 'medication_patient_conflict_flag'], 10, TRUE),
    ('clinical.medication-coding-correction.asserted', 'medication_coding_correction_apply',
     ARRAY['medication_coding', 'medication_patient_conflict_flag'], 10, TRUE)
ON CONFLICT (event_type, apply_fn) DO UPDATE SET
    projection_tables = EXCLUDED.projection_tables,
    run_order         = EXCLUDED.run_order,
    heal_safe         = EXCLUDED.heal_safe
WHERE (r.projection_tables, r.run_order, r.heal_safe)
      IS DISTINCT FROM (EXCLUDED.projection_tables, EXCLUDED.run_order, EXCLUDED.heal_safe);

-- 8. The coder worklist. ADR-0059 decision 3 makes an uncoded medication "an honest
--    not-yet-coded state routed to a coder worklist, never a forced guess" — this is that
--    route. Active (non-ceased) threads with no LIVE anchor: either never coded (no
--    medication_coding row at all) or struck (a row whose anchor is NULL).
--
--    previously_struck separates the two, and the distinction is CLINICAL, not bookkeeping:
--    "nobody has coded this yet" invites a coder to code it, whereas "a reviewer
--    established this is NOT what it was coded as" is a warning against re-coding it from
--    the same weak evidence that produced the error. Both must appear — a struck coding is
--    genuinely uncoded and must not vanish from the queue — but a coder needs to see which
--    is which.
--
--    A CEASED thread is excluded: the worklist is a queue of live clinical identity
--    questions, not an archive audit.
--
--    Created only HERE, in one file, so it never enters the multi-file view-replay problem
--    (#207 — every migration re-runs on every connect, and a view defined in two files
--    silently takes whichever definition loads last).
CREATE OR REPLACE VIEW patient_medication_uncoded AS
SELECT s.patient_id,
       s.medication_id,
       s.term,
       coalesce(mc.struck, FALSE)         AS previously_struck,
       to_timestamp(s.hlc_wall / 1000.0)  AS asserted_at
FROM medication_statement s
LEFT JOIN medication_coding mc ON mc.medication_id = s.medication_id
WHERE mc.coding_code IS NULL
  AND NOT EXISTS (SELECT 1 FROM medication_cessation c
                   WHERE c.medication_id = s.medication_id);
GRANT SELECT ON patient_medication_uncoded TO cairn_agent;

COMMIT;
