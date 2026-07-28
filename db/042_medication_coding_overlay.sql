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
    v_strike   := coalesce((p ->> 'strike')::boolean, FALSE);
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

COMMIT;
