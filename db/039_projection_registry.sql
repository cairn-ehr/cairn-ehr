-- db/039_projection_registry.sql
-- Cairn — generic reprojection (#208 / ADR-0057): the heal/rebuild entry point.
--
-- The registry + the ONE dispatcher trigger live in db/005 (both loaders carry
-- it). This file adds what only the FULL story needs: the replay entry point,
-- its operational log, the event_type index the replay scan (and #266's
-- reclassification scans) use, and the retirement of the legacy bespoke
-- demographic backfill this mechanism subsumes.
--
-- connect_and_load_schema re-runs every migration each connect: everything
-- below is idempotent.

BEGIN;

-- Prefix-scoped replay + exact-type scans. text_pattern_ops: LIKE 'p%' must
-- use the index regardless of database collation.
CREATE INDEX IF NOT EXISTS event_log_type_idx
    ON event_log (event_type text_pattern_ops);

-- Node-LOCAL operational record of every reproject run (like node_schema:
-- never signed, never on the wire — principle 12). The loader-gating tests and
-- `cairn-node status`-style surfaces read it; the bench reads elapsed_ms.
CREATE TABLE IF NOT EXISTS reproject_log (
    id          BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    ran_at      TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    prefix      TEXT        NOT NULL,
    rebuild     BOOLEAN     NOT NULL,
    source      TEXT        NOT NULL,   -- 'loader' | 'cli' | 'test' | 'manual'
    events_seen BIGINT      NOT NULL,
    elapsed_ms  BIGINT      NOT NULL,
    -- heal-mode rows skipped as heal_safe=false, as 'event_type:apply_fn'.
    -- Honest degradation is only honest if it is VISIBLE (spec §3.13 spirit).
    skipped_fns TEXT[]      NOT NULL DEFAULT '{}'
);

DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'cairn_node') THEN
        CREATE ROLE cairn_node NOLOGIN;
    END IF;
END $$;
GRANT SELECT ON reproject_log TO cairn_node;

-- The generic heal/rebuild replay (ADR-0057 decision D2).
--
--  * HEAL (default): no deletes. Every apply is insert-or-better, and every
--    projection is arrival-order-independent (set-union sync requires it), so
--    replaying the log through the SAME registered fns the live dispatcher
--    uses converges every winner row — a stale winner is itself some event's
--    tuple, dominated by the true maximum under the corrected comparison.
--    heal_safe=false rows (counter shapes) are SKIPPED and reported.
--  * REBUILD (wrote-garbage defects): TRUNCATE each in-scope projection table
--    first — refusing if any table is also fed by an out-of-prefix type, so a
--    narrow rebuild can never silently wipe another type's rows.
--
-- Runs in the LENIENT apply posture (cairn.remote_apply = on, transaction-
-- local): replayed events include remotely-admitted ones, and the clamp-vs-
-- raise helpers (#192 patient consistency, the component-size caps) must
-- clamp-and-flag here exactly as they did at admission. Events already
-- admitted by a door stay admitted: replay heals, doors refuse (ADR-0056's
-- gate-effect-not-presence, applied to replay).
--
-- Owner-only: it can TRUNCATE projections. The loader and the `cairn-node
-- reproject` CLI connect with owner privileges; the runtime role cannot call it.
CREATE OR REPLACE FUNCTION cairn_reproject(
    p_prefix  text    DEFAULT '',
    p_rebuild boolean DEFAULT false,
    p_source  text    DEFAULT 'manual'
) RETURNS TABLE(event_type text, events_replayed bigint)
LANGUAGE plpgsql
-- Pinned like cairn_event_twin's dynamic dispatch (Task-1 review): the %I EXECUTE
-- below must never resolve into an attacker-shadowed schema, regardless of caller.
SET search_path = public
AS $$
DECLARE
    v_started timestamptz := clock_timestamp();
    v_tbl     text;
    v_type    text;
    v_fns     text[];
    v_fn      text;
    v_n       bigint;
    v_total   bigint := 0;
    v_skipped text[] := '{}';
    v_skip    text[];
BEGIN
    PERFORM set_config('cairn.remote_apply', 'on', true);  -- true = SET LOCAL

    IF p_rebuild THEN
        FOR v_tbl IN
            SELECT DISTINCT unnest(r.projection_tables)
            FROM cairn_projection_apply r
            WHERE r.event_type LIKE p_prefix || '%'
        LOOP
            IF EXISTS (
                SELECT 1 FROM cairn_projection_apply r
                WHERE v_tbl = ANY (r.projection_tables)
                  AND r.event_type NOT LIKE p_prefix || '%'
            ) THEN
                RAISE EXCEPTION
                    'cairn_reproject: rebuilding prefix "%" would truncate projection table "%", which is also fed by event types outside that prefix. Widen the prefix or use heal mode.',
                    p_prefix, v_tbl;
            END IF;
            EXECUTE format('TRUNCATE %I', v_tbl);
        END LOOP;
    END IF;

    FOR v_type, v_fns, v_skip IN
        SELECT r.event_type,
               array_agg(r.apply_fn ORDER BY r.run_order, r.apply_fn)
                   FILTER (WHERE p_rebuild OR r.heal_safe),
               array_agg(r.event_type || ':' || r.apply_fn ORDER BY r.apply_fn)
                   FILTER (WHERE NOT (p_rebuild OR r.heal_safe))
        FROM cairn_projection_apply r
        WHERE r.event_type LIKE p_prefix || '%'
        GROUP BY r.event_type
        ORDER BY r.event_type
    LOOP
        v_skipped := v_skipped || COALESCE(v_skip, '{}');
        IF COALESCE(v_skip, '{}') <> '{}' THEN
            RAISE NOTICE 'cairn_reproject: heal mode skipping non-heal-safe %', v_skip;
        END IF;
        v_n := 0;
        IF v_fns IS NOT NULL THEN
            -- Set-based apply, one full-table pass per (type, fn) — replaces the former
            -- per-event PL/pgSQL loop (measured at ~25% of a 2M-event rebuild's cost: the
            -- Bet-B run clocked a full rebuild at 49 min before this change). count(*)
            -- wraps a subquery whose target list calls the apply fn: the fn is VOLATILE,
            -- so the planner can never prune it from the scan — it still runs exactly once
            -- per eligible row — but the row-by-row plpgsql FOR loop, the per-event dynamic
            -- re-EXECUTE, and the composite-value marshalling into a loop variable are all
            -- gone; Postgres streams rows through the aggregate itself instead. Do NOT drop
            -- the aggregate and EXECUTE a bare 'SELECT fn(el) FROM event_log ...' — without
            -- it SPI materializes the whole result set in memory before returning, a real
            -- risk at 2M+ rows on the resource-constrained node floor (ADR-0001 Bet B, the
            -- Pi target).
            --
            -- ORDER BY is deliberately DROPPED (the old per-event loop replayed in HLC
            -- order): every projection is arrival-order-independent by construction (#115;
            -- set-union sync requires it, and the multi-node convergence suites in
            -- overlay_tiebreaker.rs pin it), so heal/rebuild converges to the same winner
            -- regardless of scan order — a sequential/index scan needs no sort to get there.
            --
            -- Interleaving also changes: the old loop ran every fn for event 1, then every
            -- fn for event 2, .... This runs fn1 over ALL events, then fn2 over all events.
            -- Licensed because sibling apply fns registered for the same event_type write
            -- DISJOINT projection tables and never read each other's output (verified at
            -- planning time and re-verified in the Task 4-6 conversions), so there is no
            -- ordering dependency between fn1's full pass and fn2's full pass to preserve.
            --
            -- events_replayed for this type = v_n from the LAST fn's pass. Every fn for the
            -- same type scans the identical WHERE (event_type, eligibility), so every pass
            -- counts the same eligible rows; taking the last is equivalent to taking any.
            FOREACH v_fn IN ARRAY v_fns LOOP
                EXECUTE format(
                    'SELECT count(*) FROM (SELECT %I(el) FROM event_log el '
                    'WHERE el.event_type = $1 AND cairn_replay_eligible(el)) AS replay',
                    v_fn)
                USING v_type INTO v_n;
            END LOOP;
        END IF;
        event_type      := v_type;
        events_replayed := v_n;
        v_total         := v_total + v_n;
        RETURN NEXT;
    END LOOP;

    INSERT INTO reproject_log (prefix, rebuild, source, events_seen, elapsed_ms, skipped_fns)
    VALUES (p_prefix, p_rebuild, p_source, v_total,
            (extract(epoch FROM clock_timestamp() - v_started) * 1000)::bigint,
            v_skipped);
END;
$$;

REVOKE EXECUTE ON FUNCTION cairn_reproject(text, boolean, text) FROM PUBLIC;

COMMIT;
