-- 079_medication_versions.sql — batch charting slice 2a/2b: "From:", the "why", and (later in this
-- file) the set-aside version of a medication line. Canonical home: data-model §3.3.
--
-- §1 (this section) is the FLOOR for two optional free-text keys on a
-- `clinical.medication.asserted` payload (the pure builder is crates/cairn-event/src/medication/
-- source_note.rs):
--   source_detail  where the claim came from, finer than `info_source` ("GP letter", "daughter,
--                  from the packet")
--   why            why the clinician charted the line this way
-- They are TWO keys because a provenance and a rationale are different facts (see source_note.rs).
-- The projection columns live on `medication_charting` (db/077 §2).
--
-- HOW IT FITS. db/031's cairn_check_medication_assertion PERFORMs the function below. plpgsql
-- resolves the callee at run time, so db/031 may call a function this later file defines; and a
-- floor change lives in a NEW file rather than an edit to db/031/077 so it ships as its own
-- schema generation (#188, as db/077's header explains).
--
-- THE TWO TIERS (ADR-0056, the db/075 idiom). STRUCTURAL — a present key must be a JSON string
-- (the projection reads it with `->>` and must never be handed an object) — refuses at BOTH doors.
-- Everything else — non-blank, no control character — refuses at the AUTHORING door only
-- (`cairn.remote_apply` <> 'on'): a peer's signed words must never wedge sync.
BEGIN;

CREATE OR REPLACE FUNCTION cairn_check_medication_source_note(b jsonb)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    p jsonb := b -> 'payload';
    k text;
BEGIN
    FOREACH k IN ARRAY ARRAY['source_detail', 'why'] LOOP
        -- ---- Structural: both doors. Absent is fine; a JSON null is NOT a string. ----
        IF p ? k AND jsonb_typeof(p -> k) IS DISTINCT FROM 'string' THEN
            RAISE EXCEPTION 'medication assertion: % must be a string', k;
        END IF;
    END LOOP;

    -- ---- Authoring door only (ADR-0056): the one line below is the only thing that
    --      distinguishes the two doors. ----
    IF current_setting('cairn.remote_apply', true) = 'on' THEN
        RETURN;
    END IF;
    FOREACH k IN ARRAY ARRAY['source_detail', 'why'] LOOP
        IF p ? k THEN
            IF btrim(p ->> k) = '' THEN
                RAISE EXCEPTION 'medication assertion: % must not be blank (omit the key instead)', k;
            END IF;
            -- Newlines and tabs included: these are one-line labels in a list row.
            IF (p ->> k) ~ '[[:cntrl:]]' THEN
                RAISE EXCEPTION 'medication assertion: % must not contain control characters', k;
            END IF;
        END IF;
    END LOOP;
END;
$$;
-- The cairn_check_* family is revoked uniformly (#382; db/005).
REVOKE EXECUTE ON FUNCTION cairn_check_medication_source_note(jsonb) FROM PUBLIC;

COMMIT;
