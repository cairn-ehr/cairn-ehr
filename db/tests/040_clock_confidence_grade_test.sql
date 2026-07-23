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
