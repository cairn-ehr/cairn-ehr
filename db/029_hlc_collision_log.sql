-- Advisory Byzantine HLC-triple collision signal (#157) — a follow-on to #115.
--
-- WHY THIS FILE EXISTS
-- `cairn_hlc_overlay_wins` (db/002, #115) resolves a standing-state overlay by the HLC total
-- order (wall, counter, origin) with the event `content_address` (the BYTEA multihash of the
-- signed bytes) as the deterministic FINAL tiebreaker. That final key only ever decides a winner
-- when the (wall, counter, origin) triple TIES — and an honest signer can NEVER produce that tie,
-- because it would mean one node emitted two DIFFERENT signed bodies under one HLC triple. Such a
-- tie is therefore proof of a broken or hostile (Byzantine) signer.
--
-- #115 correctly makes that case CONVERGE (every node picks the higher content_address). But it
-- resolves it SILENTLY. This file adds the missing observability: when an overlay sees the
-- collision, it records an append-only, advisory signal a human (or the future §5.13 background
-- duplicate/anomaly sweep) can review — so the arbitrary hash-winner does not stand unexamined,
-- most importantly for chart_dispute (open↔resolved flips) and patient_link (merge↔un-merge).
--
-- HARD CONSTRAINTS (see the design doc 2026-07-09-hlc-collision-advisory-signal-design.md):
--   * NOT a change to the resolution — the db/002/018/023/024/025 upserts are untouched.
--   * Advisory only — the recorder NEVER raises and NEVER gates the apply path (availability over
--     consistency). No new event type, no wire/SCHEMA/ADR/spec change.

BEGIN;

-- ── The detection predicate (sibling of cairn_hlc_overlay_wins) ──────────────────────────────
-- true iff the HLC triples are EQUAL and the content_addresses are DISTINCT — i.e. exactly the
-- Byzantine case cairn_hlc_overlay_wins resolves arbitrarily. Pure/IMMUTABLE so it is safe to call
-- inline in the overlay triggers. All three equality terms use IS NOT DISTINCT FROM so the
-- predicate is null-total: a real incoming event (non-null wall/counter/origin) against a null
-- current side (a note-only patient_chart row whose demographic winner is still absent) returns
-- FALSE, never NULL.
CREATE OR REPLACE FUNCTION cairn_hlc_triple_collision(
    new_wall bigint, new_counter int, new_origin text, new_addr bytea,
    cur_wall bigint, cur_counter int, cur_origin text, cur_addr bytea
) RETURNS boolean LANGUAGE sql IMMUTABLE AS $$
    SELECT new_wall IS NOT DISTINCT FROM cur_wall
       AND new_counter IS NOT DISTINCT FROM cur_counter
       AND new_origin IS NOT DISTINCT FROM cur_origin
       AND new_addr IS DISTINCT FROM cur_addr;
$$;

-- ── The convergent append-only anomaly log ───────────────────────────────────────────────────
-- One row per detected collision per node. The two colliding content_addresses are the natural
-- key: they globally identify the two distinct events, and `overlay` is functionally determined by
-- them (each event type routes to exactly one overlay). The addresses are stored as a CANONICAL
-- UNORDERED pair (addr_lo = least, addr_hi = greatest by BYTEA byte comparison), so whichever event
-- a node happens to apply second it records the IDENTICAL row → the anomaly is itself a set-union
-- projection: every node that has seen BOTH events holds exactly one row for the collision.
-- The triple + subject_key columns are descriptive redundancy kept for worklist legibility.
-- detected_at is deliberately NOT part of the key — it is node-local observation metadata (when
-- THIS node first noticed), intentionally non-convergent.
--
-- CONVERGENCE CAVEAT: this "exactly one row per node" guarantee holds for SEQUENTIAL apply of
-- the two colliding events. Because detection is a SELECT-then-upsert in the AFTER-INSERT
-- trigger under READ COMMITTED and the clinical apply door takes no apply-serializing lock, two
-- CONCURRENT applies of the colliding pair could each miss the other's not-yet-committed winner
-- and record zero rows. This is advisory-only degradation — the #115 RESOLUTION stays correct
-- regardless — and the §5.13 background duplicate/anomaly sweep is the intended miss backstop.
CREATE TABLE IF NOT EXISTS hlc_collision_log (
    overlay      TEXT        NOT NULL,   -- 'patient_chart' | 'patient_link' | 'chart_dispute' | ...
    subject_key  TEXT        NOT NULL,   -- text rendering of the overlay's conflict key
    hlc_wall     BIGINT      NOT NULL,   -- the colliding HLC triple ...
    hlc_counter  INTEGER     NOT NULL,
    origin       TEXT        NOT NULL,
    addr_lo      BYTEA       NOT NULL,   -- least(a, b): canonical unordered pair of the two events
    addr_hi      BYTEA       NOT NULL,   -- greatest(a, b)
    detected_at  TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (overlay, addr_lo, addr_hi)
);

-- ── The recorder ─────────────────────────────────────────────────────────────────────────────
-- Called from an overlay trigger ONLY when cairn_hlc_triple_collision is true. Canonicalizes the
-- unordered pair via LEAST/GREATEST so arrival order does not matter, then appends idempotently.
-- ON CONFLICT DO NOTHING guarantees it can never raise on a re-observation — so it can never gate
-- the apply path (availability over consistency). SQL (not plpgsql): a single INSERT, no control flow.
-- Caller invariant: this relies on non-null arguments (every hlc_collision_log column is NOT NULL)
-- — it is called only when cairn_hlc_triple_collision is TRUE, which requires a non-null current
-- side, and each overlay passes non-null NEW.* fields, so a NULL-arg NOT-NULL violation cannot
-- arise from the wiring.
CREATE OR REPLACE FUNCTION cairn_record_hlc_collision(
    p_overlay text, p_subject_key text,
    p_wall bigint, p_counter int, p_origin text,
    p_addr_a bytea, p_addr_b bytea
) RETURNS void LANGUAGE sql AS $$
    INSERT INTO hlc_collision_log
        (overlay, subject_key, hlc_wall, hlc_counter, origin, addr_lo, addr_hi)
    VALUES (
        p_overlay, p_subject_key, p_wall, p_counter, p_origin,
        LEAST(p_addr_a, p_addr_b), GREATEST(p_addr_a, p_addr_b))
    ON CONFLICT (overlay, addr_lo, addr_hi) DO NOTHING;
$$;

COMMIT;
