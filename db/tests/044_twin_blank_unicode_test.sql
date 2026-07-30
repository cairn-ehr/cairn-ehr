-- #75 — the §3.13 twin blank-test is ONE definition, and it is collation-DETERMINISTIC.
--
-- SQL mirror of crates/cairn-node/tests/twin_blank_parity.rs (cross-boundary equality with
-- Rust) and crates/cairn-event/src/lib.rs::twin_blankness_follows_unicode_white_space
-- (the Rust half). Run after the schema is loaded; wrapped in a ROLLBACK so it leaves no
-- residue.
--
-- WHAT WENT WRONG BEFORE: the floor spelled the blank-test as
-- `length(regexp_replace(t, '\s+', '', 'g')) > 0` in THREE places. Postgres's `\s` is
-- `[[:space:]]`, whose membership is decided by the *collation's* ctype — under a libc
-- UTF-8 collation `iswspace(U+00A0)` is true, under `C`/`ucs_basic` it is false. So the
-- floor's answer depended on how the database had been created. Since cairn_event_twin is
-- also the remote-apply gate (db/020) and RAISEs for a hard-require type, the same signed
-- event could apply on one node and raise on another — a set-union convergence break.
BEGIN;

-- 1. The battery: every Unicode White_Space=Yes code point is BLANK; the zero-width
--    look-alikes (White_Space=No — note U+FEFF is NOT whitespace) are PRESENT. Checked
--    under several input collations: cairn_twin_is_present pins `COLLATE "C"` internally,
--    so no caller's locale may change the verdict.
DO $$
DECLARE
    bad text;
BEGIN
    WITH battery(cp, expect_present) AS (
        -- White_Space=Yes → a twin of only this character is blank.
        VALUES (x'0009'::int, false), (x'000A'::int, false), (x'000B'::int, false),
               (x'000C'::int, false), (x'000D'::int, false), (x'0020'::int, false),
               (x'0085'::int, false), (x'00A0'::int, false), (x'1680'::int, false),
               (x'2000'::int, false), (x'2001'::int, false), (x'2002'::int, false),
               (x'2003'::int, false), (x'2004'::int, false), (x'2005'::int, false),
               (x'2006'::int, false), (x'2007'::int, false), (x'2008'::int, false),
               (x'2009'::int, false), (x'200A'::int, false), (x'2028'::int, false),
               (x'2029'::int, false), (x'202F'::int, false), (x'205F'::int, false),
               (x'3000'::int, false),
               -- White_Space=No → present on BOTH sides of the boundary.
               (x'200B'::int, true), (x'FEFF'::int, true), (x'0061'::int, true)
    )
    SELECT string_agg(format('U+%s', upper(to_hex(cp))), ', ')
      INTO bad
      FROM battery
     WHERE cairn_twin_is_present(chr(cp))                    IS DISTINCT FROM expect_present
        OR cairn_twin_is_present(chr(cp) COLLATE "C")        IS DISTINCT FROM expect_present
        OR cairn_twin_is_present(chr(cp) COLLATE "POSIX")    IS DISTINCT FROM expect_present
        OR cairn_twin_is_present(chr(cp) COLLATE "ucs_basic") IS DISTINCT FROM expect_present;

    IF bad IS NOT NULL THEN
        RAISE EXCEPTION 'FAIL: blank-test disagrees with Unicode White_Space at: %', bad;
    END IF;
END $$;

-- 2. NULL and mixed content.
DO $$
BEGIN
    IF cairn_twin_is_present(NULL) IS NOT FALSE THEN
        RAISE EXCEPTION 'FAIL: a NULL twin is not present';
    END IF;
    -- Real text padded with exotic whitespace is still present.
    IF NOT cairn_twin_is_present(chr(x'00A0'::int) || 'BP 120/80' || chr(x'3000'::int)) THEN
        RAISE EXCEPTION 'FAIL: padded real text must be present';
    END IF;
END $$;

-- 3. SINGLE SOURCE: every blank-test call site delegates to the one function, and no call
--    site still spells the collation-dependent `\s` form. This is the anti-drift guard —
--    the defect existed precisely because the test was written out three times.
DO $$
DECLARE
    fn   text;
    body text;
BEGIN
    FOREACH fn IN ARRAY ARRAY['cairn_event_twin',            -- db/005, the write gate
                              'cairn_twin_is_authored',      -- db/015, read predicate
                              'cairn_twin_provenance_of']    -- db/015, read predicate
    LOOP
        SELECT pg_get_functiondef(fn::regproc) INTO body;
        IF position('cairn_twin_is_present' in body) = 0 THEN
            RAISE EXCEPTION 'FAIL: % does not delegate to cairn_twin_is_present', fn;
        END IF;
        IF position('\s' in body) > 0 THEN
            RAISE EXCEPTION 'FAIL: % still uses the collation-dependent \s blank-test', fn;
        END IF;
    END LOOP;
END $$;

-- 4. End-to-end through the write gate: an unregistered type (no structural check, no
--    hard twin requirement) with a twin of only NO-BREAK SPACE must degrade to the derived
--    skeleton, never carry the pathological twin through.
DO $$
DECLARE
    v_twin text;
    v_body jsonb := jsonb_build_object(
        'schema_version', 'test.blank/1',
        'patient_id',     '00000000-0000-0000-0000-000000000001',
        'plaintext_twin', chr(x'00A0'::int),
        'payload',        jsonb_build_object('text', 'x'));
BEGIN
    v_twin := cairn_event_twin('test.unregistered.blank', v_body);
    IF v_twin = chr(x'00A0'::int) THEN
        RAISE EXCEPTION 'FAIL: an all-NBSP twin was accepted as authored';
    END IF;
    IF position('test.unregistered.blank' in v_twin) = 0 THEN
        RAISE EXCEPTION 'FAIL: expected the derived skeleton, got: %', v_twin;
    END IF;
END $$;

ROLLBACK;
