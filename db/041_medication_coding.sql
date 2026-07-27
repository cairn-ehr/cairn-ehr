-- 041_medication_coding.sql — the drug-identity coding floor (ADR-0059, data-model §3.16).
--
-- ADR-0059 anchors a medication's drug identity on drugref's immortal `moiety_uuid`
-- carried as substance.coding {system, code, display}. This file holds the two pieces
-- that govern it: the vocabulary registry of admitted coding systems, and the floor
-- check db/031's per-type check calls.
--
-- WHY A SEPARATE FILE: SCHEMA_GENERATION is derived from the newest db/ prefix, and this
-- is a FLOOR change. Issue #188 exists so an older binary cannot CREATE OR REPLACE a
-- newer safety check back down; an in-place edit of db/031 alone could not bump the
-- generation and would leave that downgrade silent.
--
-- TWO TIERS, because the per-type floor runs at BOTH doors (db/020 §8 calls the same
-- cairn_event_twin hook as submit_event, deliberately — the M8 asymmetry fix):
--   structural       (three non-empty strings)  -> refuse at BOTH doors, like substance.term
--   registry-derived (known system, code shape) -> refuse locally, ADMIT remotely (ADR-0051)
-- A peer may legitimately run a newer or locally-extended registry, and a refusal on a
-- verifiable event is the sync-wedge ADR-0056 forbids.
BEGIN;

-- 1. The admitted coding systems. Register-by-row, like event_type_class /
--    cairn_event_twin_check / cairn_projection_apply. ADR-0059 decision 7 is explicit
--    that a deployment may plug a DIFFERENT drug-identity authority: that is a row here,
--    not a patch to this file (principle 9 — mechanism, never policy).
CREATE TABLE IF NOT EXISTS medication_coding_system (
    system      TEXT PRIMARY KEY,
    -- 'uuid'   : the code must parse as a uuid (drugref moiety ids are UUIDv5)
    -- 'opaque' : any non-empty string
    code_format TEXT NOT NULL CHECK (code_format IN ('uuid', 'opaque')),
    note        TEXT NOT NULL
);
GRANT SELECT ON medication_coding_system TO cairn_agent;
REVOKE INSERT, UPDATE, DELETE ON medication_coding_system FROM PUBLIC;

-- Seed the drugref composition-tree levels. Only `drugref-moiety` exists today; the two
-- finer levels are RESERVED by ADR-0059 decision 2 so strength/form-level coding lands
-- additively later without reshaping the slot. #214 convergence: DO UPDATE (never DO
-- NOTHING) so an edited seed heals on the next connect, with the IS DISTINCT FROM guard
-- keeping the steady-state replay write-free.
INSERT INTO medication_coding_system AS r (system, code_format, note) VALUES
    ('drugref-moiety',        'uuid', 'drugref immortal moiety_uuid (UUIDv5 from UNII) — the only level built today'),
    ('drugref-clinical-drug', 'uuid', 'RESERVED for a later drugref slice (substance + strength + form)'),
    ('drugref-product',       'uuid', 'RESERVED for a later drugref slice (a marketed product)')
ON CONFLICT (system) DO UPDATE SET
    code_format = EXCLUDED.code_format,
    note        = EXCLUDED.note
WHERE (r.code_format, r.note) IS DISTINCT FROM (EXCLUDED.code_format, EXCLUDED.note);

-- 2. The floor check. Called from cairn_check_medication_assertion (db/031); plpgsql
--    resolves the call at EXECUTION, so living in a later file is fine.
CREATE OR REPLACE FUNCTION cairn_check_medication_coding(p jsonb)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    c        jsonb   := p -> 'substance' -> 'coding';
    -- db/020 sets this transaction-local marker on the sync-apply path; the same idiom
    -- cairn_guard_medication_patient uses to tell the doors apart (db/031 part 3b). This
    -- fn is the FIRST twin-registered check_fn (cairn_event_twin_check, dispatched via
    -- cairn_event_twin at db/020's step 8) ever to read this marker: every earlier
    -- reader lived one layer down, in a projection-apply function fired by the
    -- AFTER-INSERT trigger. That mattered: db/020 used to raise the marker just before
    -- the event_log INSERT, AFTER step 8 had already run — so a check_fn reading it
    -- here would always see it unset, unable to ever tell "remote" from "local". db/020
    -- now raises the marker BEFORE step 8 specifically so this check can see it. A
    -- future reader must not "tidy" that set_config back down next to the INSERT —
    -- doing so would silently re-break this door's leniency and reintroduce the
    -- ADR-0056 sync-watermark freeze this file exists to avoid.
    v_remote boolean := current_setting('cairn.remote_apply', true) = 'on';
    v_key    text;
    v_format text;
BEGIN
    -- Uncoded is a permanently valid state (principle 4, the "little white pill" floor).
    -- Absent (c IS NULL, the key was never set) and an EXPLICIT JSON null are the SAME
    -- honest-unknown claim, not two different shapes: jsonb_typeof(c) = 'null' is how an
    -- explicit `"coding": null` reads once extracted with `->` (jsonb_typeof('null'::jsonb)
    -- is the string 'null', not SQL NULL). A peer whose serializer emits explicit nulls
    -- for absent optionals must not have an otherwise-verifiable event refused at the
    -- apply door over a JSON-encoding style choice — that refusal is the exact
    -- ADR-0056 watermark freeze this file exists to argue against.
    IF c IS NULL OR jsonb_typeof(c) = 'null' THEN
        RETURN;
    END IF;
    IF jsonb_typeof(c) IS DISTINCT FROM 'object' THEN
        RAISE EXCEPTION 'medication assertion: substance.coding must be an object {system, code, display} (ADR-0059)';
    END IF;

    -- Structural tier — both doors. display is NOT optional: it is the honest-degradation
    -- label, the whole reason a drugref-less node can still read a coded medication.
    FOREACH v_key IN ARRAY ARRAY['system', 'code', 'display'] LOOP
        IF jsonb_typeof(c -> v_key) IS DISTINCT FROM 'string'
           OR length(btrim(c ->> v_key)) = 0 THEN
            RAISE EXCEPTION
                'medication assertion: substance.coding.% must be a non-empty string (ADR-0059 decision 2 — display is the honest-degradation label)',
                v_key;
        END IF;
    END LOOP;

    -- Registry-derived tier — local door only (strict-submit / lenient-apply, ADR-0051).
    IF v_remote THEN
        RETURN;
    END IF;
    SELECT s.code_format INTO v_format
        FROM medication_coding_system s WHERE s.system = c ->> 'system';
    IF v_format IS NULL THEN
        RAISE EXCEPTION
            'medication assertion: unknown coding system "%" — this door only authors codings it can vouch for; register it in medication_coding_system (ADR-0059 decision 7)',
            c ->> 'system';
    END IF;
    IF v_format = 'uuid' THEN
        -- pg_input_is_valid (PG18+) checks parseability without a subtransaction and
        -- without a catch-all `WHEN others` that would relabel an unrelated internal
        -- error (out-of-memory, whatever) as "requires a uuid code" — the exception
        -- handler this replaced could not tell "malformed input" from "something else
        -- broke" apart, and always blamed the caller.
        IF NOT pg_input_is_valid(c ->> 'code', 'uuid') THEN
            RAISE EXCEPTION
                'medication assertion: coding system "%" requires a uuid code, got "%" (a drugref moiety id is a UUIDv5)',
                c ->> 'system', c ->> 'code';
        END IF;
        -- Canonical form only, not merely "parses": uuid_in accepts braces, uppercase,
        -- and a missing-hyphens spelling, so "{0F8C4B1E-...}" and "0f8c4b1e1b7a..."
        -- both parse today. The dup-key that will consume this anchor (Task 5) compares
        -- the CODE as text, so two events naming the SAME moiety in different uuid
        -- spellings would key apart — quietly defeating the immortal-anchor point of
        -- ADR-0059 — and it cannot be fixed after the fact, because the spelling is
        -- inside an already-signed body. Round-tripping the text through ::uuid::text
        -- yields Postgres' own canonical lowercase-hyphenated-no-braces form; comparing
        -- that against the original text catches every non-canonical spelling at the
        -- one door that can still refuse it.
        IF (c ->> 'code') IS DISTINCT FROM ((c ->> 'code')::uuid)::text THEN
            RAISE EXCEPTION
                'medication assertion: coding system "%" requires the canonical lowercase-hyphenated uuid form, got "%" (use % instead)',
                c ->> 'system', c ->> 'code', ((c ->> 'code')::uuid)::text;
        END IF;
    END IF;
END;
$$;
REVOKE EXECUTE ON FUNCTION cairn_check_medication_coding(jsonb) FROM PUBLIC;

COMMIT;
