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

-- The system name's SHAPE, added by ALTER rather than inline above: `CREATE TABLE IF NOT
-- EXISTS` is a silent no-op on a database that already has the table, so a constraint
-- introduced after the table has ever been created can only arrive this way (#207 — the
-- paired-ALTER rule). Guarded on pg_constraint so the whole file stays replay-idempotent.
--
-- WHY IT MATTERS: the E1 dup-key (db/031 + db/033) and the anchor-conflict view (db/033)
-- both flatten an anchor to `<system>|<code>`, which makes `|` a load-bearing SEPARATOR.
-- A system registered as `a|b` would let its codes collide with system `a`'s code `b|…`,
-- silently keying two DIFFERENT substances as one duplicate group. Constraining the
-- SYSTEM alone is sufficient: with systems `|`-free, the first `|` after the prefix is
-- always the separator, so the flattened key parses unambiguously whatever the code
-- holds — and codes cannot be constrained here anyway, they arrive inside signed bodies.
DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_constraint
                    WHERE conname = 'medication_coding_system_system_shape') THEN
        ALTER TABLE medication_coding_system
            ADD CONSTRAINT medication_coding_system_system_shape
            CHECK (length(btrim(system)) > 0 AND position('|' IN system) = 0);
    END IF;
END $$;

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

-- 2a. The coding-object checks, independent of WHERE the object sits in a payload.
--     Slice 6a's only caller reads substance.coding on the assertion; slice 6b's overlay
--     types (db/042) carry the SAME object at payload.coding. Extracting the checks keeps
--     ONE definition of what a valid coding claim is — the two-tier split, the
--     canonical-uuid pin and the strict/lenient door behaviour cannot drift apart between
--     the inline and overlay paths.
--
--     p_prefix is the caller's message prefix (e.g. 'medication assertion:
--     substance.coding'), so each caller's refusals keep naming the field the way its own
--     authors wrote it — a coder reading "medication coding-correction: coding.display …"
--     should not be sent looking for a `substance` object that verb does not have.
CREATE OR REPLACE FUNCTION cairn_check_coding_object(c jsonb, p_prefix text)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    -- db/020 sets this transaction-local marker on the sync-apply path; the same idiom
    -- cairn_guard_medication_patient uses to tell the doors apart (db/031 part 3b). This
    -- check (via the twin-registered check_fns that call it — cairn_check_medication_coding
    -- for the inline path, cairn_check_medication_coding_overlay in db/042 for the
    -- overlays) was the FIRST check_fn-side reader of this marker: every earlier reader
    -- lived one layer down, in a projection-apply function fired by the AFTER-INSERT
    -- trigger. That mattered: db/020 used to raise the marker just before the event_log
    -- INSERT, AFTER step 8 (the cairn_event_twin dispatch) had already run — so a check_fn
    -- reading it here would always see it unset, unable to ever tell "remote" from
    -- "local". db/020 now raises the marker BEFORE step 8 specifically so this check can
    -- see it. A future reader must not "tidy" that set_config back down next to the
    -- INSERT — doing so would silently re-break this door's leniency and reintroduce the
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
        RAISE EXCEPTION '% must be an object {system, code, display} (ADR-0059)', p_prefix;
    END IF;

    -- Structural tier — both doors. display is NOT optional: it is the honest-degradation
    -- label, the whole reason a drugref-less node can still read a coded medication.
    FOREACH v_key IN ARRAY ARRAY['system', 'code', 'display'] LOOP
        IF jsonb_typeof(c -> v_key) IS DISTINCT FROM 'string'
           OR length(btrim(c ->> v_key)) = 0 THEN
            RAISE EXCEPTION
                '%.% must be a non-empty string (ADR-0059 decision 2 — display is the honest-degradation label)',
                p_prefix, v_key;
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
            '%: unknown coding system "%" — this door only authors codings it can vouch for; register it in medication_coding_system (ADR-0059 decision 7)',
            p_prefix, c ->> 'system';
    END IF;
    IF v_format = 'uuid' THEN
        -- pg_input_is_valid (PG18+) checks parseability without a subtransaction and
        -- without a catch-all `WHEN others` that would relabel an unrelated internal
        -- error (out-of-memory, whatever) as "requires a uuid code" — the exception
        -- handler this replaced could not tell "malformed input" from "something else
        -- broke" apart, and always blamed the caller.
        IF NOT pg_input_is_valid(c ->> 'code', 'uuid') THEN
            RAISE EXCEPTION
                '%: coding system "%" requires a uuid code, got "%" (a drugref moiety id is a UUIDv5)',
                p_prefix, c ->> 'system', c ->> 'code';
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
                '%: coding system "%" requires the canonical lowercase-hyphenated uuid form, got "%" (use % instead)',
                p_prefix, c ->> 'system', c ->> 'code', ((c ->> 'code')::uuid)::text;
        END IF;
    END IF;
END;
$$;
REVOKE EXECUTE ON FUNCTION cairn_check_coding_object(jsonb, text) FROM PUBLIC;

-- 2b. The assertion's floor check — the path lookup, delegating every check above.
--     Called from cairn_check_medication_assertion (db/031); plpgsql resolves the call at
--     EXECUTION, so living in a later file is fine. Kept as its own function (rather than
--     inlining the delegation into db/031) so db/031 keeps naming one stable check per
--     concern and the twin-registry lookup test can still find it.
CREATE OR REPLACE FUNCTION cairn_check_medication_coding(p jsonb)
RETURNS void LANGUAGE plpgsql AS $$
BEGIN
    PERFORM cairn_check_coding_object(
        p -> 'substance' -> 'coding', 'medication assertion: substance.coding');
END;
$$;
REVOKE EXECUTE ON FUNCTION cairn_check_medication_coding(jsonb) FROM PUBLIC;

COMMIT;
