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

-- The born grade column on the append-only log. Plain text (admits future grades), NOT
-- NULL DEFAULT 'unknown' so existing rows + any omitting writer read honestly as unknown.
-- Added by ALTER for existing DBs (#207 paired-ALTER discipline); event_log's canonical
-- CREATE lives in db/001.
ALTER TABLE event_log ADD COLUMN IF NOT EXISTS clock_grade text NOT NULL DEFAULT 'unknown';

-- The advisory clash record — a cross-type DOOR-side write, NOT an ADR-0057 projection
-- (cairn_projection_dispatch keys on event_type; the ceiling is type-independent). Both
-- doors call cairn_record_ceiling_flag on a flag/reject verdict. Append-only; keyed by the
-- flagged event's content_address so set-union re-delivery is idempotent. Survives a
-- cairn_reproject rebuild untouched (rebuild replays through the dispatch, never the doors,
-- and the inputs are immutable). A future grade we do not recognize is still recorded.
CREATE TABLE IF NOT EXISTS t_effective_ceiling_flag (
    flag_id         BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    content_address BYTEA,                          -- flagged event identity (dedup key)
    hlc_wall        BIGINT      NOT NULL,
    t_effective     TIMESTAMPTZ NOT NULL,
    clock_grade     TEXT        NOT NULL,
    verdict         TEXT        NOT NULL,           -- 'flag' | 'reject'
    flagged_at      TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
-- NULLS DISTINCT (default): a pre-column NULL never collides; a real address is unique.
CREATE UNIQUE INDEX IF NOT EXISTS t_effective_ceiling_flag_ca_idx
    ON t_effective_ceiling_flag (content_address);
GRANT SELECT ON t_effective_ceiling_flag TO cairn_agent;

CREATE OR REPLACE FUNCTION cairn_record_ceiling_flag(
    p_ca bytea, p_hlc_wall bigint, p_t_eff timestamptz, p_grade text, p_verdict text)
RETURNS void LANGUAGE sql AS $$
    INSERT INTO t_effective_ceiling_flag (content_address, hlc_wall, t_effective, clock_grade, verdict)
    VALUES (p_ca, p_hlc_wall, p_t_eff, p_grade, p_verdict)
    ON CONFLICT (content_address) DO NOTHING;
$$;

-- The honest-assembly clock-health read (ADR-0027 §7): compares the RTC to hlc_state.wall,
-- which the HLC A3 merge keeps >= every accepted event. A live derived read — never stored,
-- never an event, never synced. The CLI `status` renders it; any client reads the row.
--
-- hlc_state is deliberately door-only (db/007_node_federation.sql: "the runtime ticks via
-- the door only — never raw DML on the table") — even cairn_node has no grant on it. So this
-- read is itself a door: SECURITY DEFINER lets it run as its owner (who can read hlc_state)
-- while callers need only EXECUTE (granted below to cairn_agent); we do NOT add a direct
-- SELECT grant on hlc_state, which would be the first crack in that door-only posture.
-- search_path is pinned for the same reason every SECURITY DEFINER function pins it (see
-- cairn_projection_dispatch): an unpinned search_path lets a caller-controlled schema shadow
-- an unqualified identifier and hijack the definer's elevated privilege.
--
-- clock_timestamp() is sampled ONCE via the `t` CTE — it is volatile and re-evaluated on
-- every bare reference, so referencing it four times in one SELECT could report a rtc_now
-- microseconds apart from the value actually used to derive behind_by_ms/is_behind/
-- effective_lower_bound. Sampling once keeps all four columns mutually consistent.
CREATE OR REPLACE FUNCTION cairn_clock_health()
RETURNS TABLE(rtc_now timestamptz, hlc_floor timestamptz, behind_by_ms bigint,
              is_behind boolean, effective_lower_bound timestamptz, default_grade text)
LANGUAGE sql STABLE SECURITY DEFINER SET search_path = public AS $$
    WITH t AS (SELECT clock_timestamp() AS now)
    SELECT
        t.now,
        to_timestamp(s.hlc_wall / 1000.0),
        GREATEST(0, s.hlc_wall - (extract(epoch FROM t.now) * 1000)::bigint),
        (s.hlc_wall - (extract(epoch FROM t.now) * 1000)::bigint) > 1000,
        GREATEST(t.now, to_timestamp(s.hlc_wall / 1000.0)),
        'self-asserted'::text
    FROM hlc_state s, t WHERE s.id;
$$;
GRANT EXECUTE ON FUNCTION cairn_clock_health() TO cairn_agent;
