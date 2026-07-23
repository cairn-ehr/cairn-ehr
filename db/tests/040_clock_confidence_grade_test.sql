-- Issue #216 / ADR-0058 — the grade-gated ceiling classifier truth table.
DO $$
DECLARE w bigint := 1_700_000_000_000;  -- an arbitrary hlc wall (ms)
BEGIN
    -- backdating is always ok, at any grade
    IF cairn_ceiling_classify(w, 'self-asserted', to_timestamp((w-60000)/1000.0)) <> 'ok'
        THEN RAISE EXCEPTION 'FAIL: backdate must be ok'; END IF;
    -- NULL t_effective is ok
    IF cairn_ceiling_classify(w, 'self-asserted', NULL) <> 'ok'
        THEN RAISE EXCEPTION 'FAIL: null t_eff must be ok'; END IF;
    -- self-asserted forward = FLAG, never reject (the principle-4 fix: even decades ahead)
    IF cairn_ceiling_classify(w, 'self-asserted', to_timestamp((w + 60000)/1000.0)) <> 'flag'
        THEN RAISE EXCEPTION 'FAIL: self-asserted forward must flag'; END IF;
    IF cairn_ceiling_classify(w, 'self-asserted', to_timestamp((w::numeric + 2e12)/1000.0)) <> 'flag'
        THEN RAISE EXCEPTION 'FAIL: self-asserted far-forward must still flag, never reject'; END IF;
    -- unknown behaves like self-asserted (open above)
    IF cairn_ceiling_classify(w, 'unknown', to_timestamp((w + 60000)/1000.0)) <> 'flag'
        THEN RAISE EXCEPTION 'FAIL: unknown forward must flag'; END IF;
    -- an UNRECOGNIZED (future) grade is treated as rank 0 → open above → flag (never reject)
    IF cairn_ceiling_classify(w, 'quantum-entangled', to_timestamp((w + 60000)/1000.0)) <> 'flag'
        THEN RAISE EXCEPTION 'FAIL: unknown future grade must flag, not reject'; END IF;
    -- a high grade (DORMANT this slice) re-arms reject above its bound
    IF cairn_ceiling_classify(w, 'hardware-sourced', to_timestamp((w + 1000)/1000.0)) <> 'flag'
        THEN RAISE EXCEPTION 'FAIL: hardware within W must flag'; END IF;
    IF cairn_ceiling_classify(w, 'hardware-sourced', to_timestamp((w + 3_600_000)/1000.0)) <> 'reject'
        THEN RAISE EXCEPTION 'FAIL: hardware far past W must reject'; END IF;
    RAISE NOTICE 'PASS: cairn_ceiling_classify truth table';
END $$;

DO $$
DECLARE ca bytea := '\x1220'::bytea || digest('ceiling-flag-test', 'sha256'); n int;
BEGIN
    PERFORM cairn_record_ceiling_flag(ca, 1_700_000_000_000, to_timestamp(1_700_000_060.0), 'self-asserted', 'flag');
    PERFORM cairn_record_ceiling_flag(ca, 1_700_000_000_000, to_timestamp(1_700_000_060.0), 'self-asserted', 'flag');
    SELECT count(*) INTO n FROM t_effective_ceiling_flag WHERE content_address = ca;
    IF n <> 1 THEN RAISE EXCEPTION 'FAIL: content_address dedup expected 1 row, got %', n; END IF;
    -- clock_health returns exactly one row with the expected column set
    PERFORM 1 FROM cairn_clock_health();
    IF NOT FOUND THEN RAISE EXCEPTION 'FAIL: cairn_clock_health returned no row'; END IF;
    RAISE NOTICE 'PASS: flag dedup + clock_health';
END $$;

-- #207 paired-ALTER discipline: event_log's canonical CREATE lives in db/001, but
-- clock_grade is added by an ALTER here in db/040. Assert it actually landed after
-- a full replay, so a future refactor that drops/reorders this ALTER fails loudly
-- instead of drifting the column out of existence silently.
DO $$
DECLARE t text;
BEGIN
    SELECT data_type INTO t FROM information_schema.columns
      WHERE table_name='event_log' AND column_name='clock_grade';
    IF t IS NULL THEN RAISE EXCEPTION 'FAIL: event_log.clock_grade missing after replay (#207)'; END IF;
    RAISE NOTICE 'PASS: event_log.clock_grade present';
END $$;

-- Review finding 1: cairn_clock_health() must be reachable by cairn_agent even though
-- hlc_state itself grants that role nothing (the table is door-only). The prior test above
-- only called the function as the owning/superuser role, so it never exercised this path.
-- SECURITY DEFINER + GRANT EXECUTE is the fix; this proves it holds from the caller's seat.
SET ROLE cairn_agent;
DO $$
BEGIN
    PERFORM 1 FROM cairn_clock_health();
    IF NOT FOUND THEN
        RAISE EXCEPTION 'FAIL: cairn_clock_health unreachable as cairn_agent';
    END IF;
    RAISE NOTICE 'PASS: cairn_clock_health reachable as cairn_agent';
END $$;
RESET ROLE;

-- Review finding 3b: the unique index on content_address is NULLS DISTINCT (the default),
-- so multiple flag rows with a NULL content_address must coexist rather than collapse to
-- one via ON CONFLICT (content_address) DO NOTHING. Only a real (non-NULL) address dedups.
DO $$
DECLARE n int;
BEGIN
    PERFORM cairn_record_ceiling_flag(NULL, 1, to_timestamp(1.0), 'unknown', 'flag');
    PERFORM cairn_record_ceiling_flag(NULL, 1, to_timestamp(1.0), 'unknown', 'flag');
    SELECT count(*) INTO n FROM t_effective_ceiling_flag WHERE content_address IS NULL;
    IF n < 2 THEN
        RAISE EXCEPTION 'FAIL: expected >= 2 NULL-content_address rows to coexist, got %', n;
    END IF;
    RAISE NOTICE 'PASS: NULL content_address rows coexist (NULLS DISTINCT)';
END $$;
