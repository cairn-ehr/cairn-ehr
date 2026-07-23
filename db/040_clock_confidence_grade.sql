-- db/040 — the ADR-0027 clock-confidence grade + the grade-gated t_effective ceiling
-- (issue #216, ADR-0058). Loaded by BOTH replayers (cairn-node full + cairn-sync subset):
-- db/020 references these helpers and the flag table via PL/pgSQL late-binding, so this
-- file MUST be in cairn-sync's SCHEMA list. Additive-only; adding this file bumps
-- SCHEMA_GENERATION to 40 (the #188 downgrade guard).

-- The ratified grade vocabulary (STRICT-door gate). The COLUMN is plain text, never a
-- closed CHECK domain: a FUTURE grade value from an upgraded peer must be ADMITTED
-- verbatim at the lenient door (additive-only, principle 11), while a node may not AUTHOR
-- one it does not know (strict-submit / lenient-apply, ADR-0051).
CREATE OR REPLACE FUNCTION cairn_clock_grade_is_ratified(g text)
RETURNS boolean LANGUAGE sql IMMUTABLE AS $$
    SELECT g IN ('unknown','self-asserted','network-synced',
                 'hardware-sourced','externally-anchored','multi-anchor-corroborated');
$$;

-- Ordered rank (0 = least trusted). An unrecognized/future value ranks 0 — the SAFE
-- direction: it can only WITHHOLD reject power, never grant it.
CREATE OR REPLACE FUNCTION cairn_clock_grade_rank(g text)
RETURNS int LANGUAGE sql IMMUTABLE AS $$
    SELECT CASE g
        WHEN 'self-asserted' THEN 1
        WHEN 'network-synced' THEN 2
        WHEN 'hardware-sourced' THEN 3
        WHEN 'externally-anchored' THEN 4
        WHEN 'multi-anchor-corroborated' THEN 5
        ELSE 0 END;  -- 'unknown' and every unrecognized value
$$;

-- The forward-slowness tolerance upper bound (ms). Ranks 0-1 (unknown / self-asserted)
-- return NULL = OPEN ABOVE: a node that cannot prove what time it is has no standing to
-- call a timestamp impossible (principle 4 applied recursively). The higher-grade widths
-- are DORMANT this slice — no node mints above self-asserted until a verified clock source
-- lands (ADR-0058 deferred) — and are placeholders to be replaced with real per-source
-- uncertainty then.
CREATE OR REPLACE FUNCTION cairn_ceiling_upper_ms(p_hlc_wall bigint, p_grade text)
RETURNS bigint LANGUAGE sql IMMUTABLE AS $$
    SELECT CASE
        WHEN cairn_clock_grade_rank(p_grade) <= 1 THEN NULL
        ELSE p_hlc_wall + CASE p_grade
            WHEN 'network-synced'            THEN 3600000
            WHEN 'hardware-sourced'          THEN 5000
            WHEN 'externally-anchored'       THEN 2000
            WHEN 'multi-anchor-corroborated' THEN 1000
            ELSE 0 END
    END;
$$;

-- The single classification rule both doors call. 'ok' (admit clean) | 'flag' (admit +
-- advisory record) | 'reject' (strict door only; production-unreachable this slice).
CREATE OR REPLACE FUNCTION cairn_ceiling_classify(p_hlc_wall bigint, p_grade text, p_t_eff timestamptz)
RETURNS text LANGUAGE sql IMMUTABLE AS $$
    SELECT CASE
        WHEN p_t_eff IS NULL THEN 'ok'
        WHEN p_t_eff <= to_timestamp(p_hlc_wall / 1000.0) THEN 'ok'
        WHEN cairn_ceiling_upper_ms(p_hlc_wall, p_grade) IS NULL THEN 'flag'
        WHEN p_t_eff <= to_timestamp(cairn_ceiling_upper_ms(p_hlc_wall, p_grade) / 1000.0) THEN 'flag'
        ELSE 'reject'
    END;
$$;
