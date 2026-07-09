# Design — Surface the Byzantine HLC-triple collision as an advisory signal (#157)

**Date:** 2026-07-09
**Issue:** [#157](https://github.com/cairn-ehr/cairn-ehr/issues/157)
**Follow-on from:** #115 part 1 (PR #156), which added `cairn_hlc_overlay_wins()` — the
`content_address` deterministic final tiebreaker for the five uniform standing-state overlays.
**Spec/ADR impact:** none (projection-read-side observability only; no wire/event-format/floor-gate
change, no new event type, no SCHEMA-array change, no ADR/spec bump).

---

## 1. Problem

`cairn_hlc_overlay_wins()` (db/002, #115) folds a new event into a standing-state overlay only when it
outranks the stored winner on `(hlc_wall, hlc_counter, origin, content_address)`. The final key,
`content_address` (the BYTEA multihash of the signed bytes), exists solely to break a tie on the
`(wall, counter, origin)` triple — a tie an **honest** signer can never produce, because it would mean
the same node emitted two *different* signed bodies under one HLC triple. Such a tie is therefore proof
of a **broken or hostile (Byzantine) signer**.

#115 correctly prioritizes **convergence**: every node picks the same winner (higher `content_address`).
For the Byzantine case there is no clinically "correct" winner, so deterministic-but-arbitrary is the
best achievable *resolution*. But the collision is currently resolved **entirely silently** — nothing
records that a node observed two distinct events colliding on one HLC triple.

This matters most for `chart_dispute` (the arbitrary winner flips **open ↔ resolved**, i.e. under-review
vs confirmed — clinician-visible) and `patient_link` (merge vs un-merge). A silently-resolved arbitrary
flip in a clinician-visible state should be surfaced for human review.

## 2. Goal & non-goals

**Goal:** raise an **append-only, advisory, convergent** signal whenever an overlay detects a genuine
HLC-triple collision between two distinct `content_address`es, so a human (or the future §5.13 background
sweep) can review rather than let the arbitrary hash-winner stand unexamined.

**Non-goals (hard constraints):**
- **Not** a change to the resolution. Convergence via `content_address` is correct and stays
  byte-for-byte unchanged.
- **Must not gate or block** the safety-critical apply path (availability over consistency). No `RAISE`,
  no veto — purely additive.
- **No policy.** The floor emits the *mechanism* (a durable record); "surface to a human" / "feed the
  sweep" is a higher-tier concern (principle 9 — mechanism, never policy; the four-layer model).

## 3. Design

Three pieces, all mirroring the existing #115 structure.

### 3.1 Detection — a shared pure predicate

A sibling to `cairn_hlc_overlay_wins`, defined alongside it:

```sql
-- true iff the HLC triples are EQUAL and the content_addresses are DISTINCT — i.e. exactly the
-- Byzantine case cairn_hlc_overlay_wins resolves arbitrarily. An honest signer can never produce it.
CREATE OR REPLACE FUNCTION cairn_hlc_triple_collision(
    new_wall bigint, new_counter int, new_origin text, new_addr bytea,
    cur_wall bigint, cur_counter int, cur_origin text, cur_addr bytea
) RETURNS boolean LANGUAGE sql IMMUTABLE AS $$
    SELECT new_wall = cur_wall
       AND new_counter = cur_counter
       AND new_origin IS NOT DISTINCT FROM cur_origin
       AND new_addr IS DISTINCT FROM cur_addr;
$$;
```

Pure, `IMMUTABLE`, reused at all five call sites (house rule 1). `IS DISTINCT FROM` / `IS NOT DISTINCT
FROM` are null-safe; in practice a real event always has a non-null triple, but null-safety keeps the
predicate total and matches `cairn_hlc_overlay_wins`'s COALESCE-of-current discipline.

### 3.2 Record — the convergent append-only table + a recorder helper

```sql
CREATE TABLE IF NOT EXISTS hlc_collision_log (
    overlay      TEXT        NOT NULL,   -- which overlay saw it: 'patient_chart' | 'patient_link' | ...
    subject_key  TEXT        NOT NULL,   -- text rendering of the overlay's conflict key (see 3.3)
    hlc_wall     BIGINT      NOT NULL,   -- the colliding HLC triple ...
    hlc_counter  INTEGER     NOT NULL,
    origin       TEXT        NOT NULL,
    addr_lo      BYTEA       NOT NULL,   -- canonical unordered pair of the two colliding events:
    addr_hi      BYTEA       NOT NULL,   --   least(a,b) / greatest(a,b) by BYTEA byte comparison
    detected_at  TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),  -- node-local observation metadata
    PRIMARY KEY (overlay, addr_lo, addr_hi)      -- convergent dedup: one row per collision per node
);

-- Never raises, never gates. Canonicalizes the unordered pair so arrival order does not matter, then
-- appends idempotently. Called from an overlay trigger ONLY when cairn_hlc_triple_collision is true.
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
```

**Why convergent:** `LEAST/GREATEST` on the raw BYTEA makes `{addr_a, addr_b}` an *unordered* pair
(byte comparison — the same collation-free property #115 relies on). Whichever colliding event a node
happens to apply second, it records the identical `(addr_lo, addr_hi)` → the anomaly is itself a
set-union projection: every node that has seen **both** events holds **exactly one** row for the
collision. `ON CONFLICT DO NOTHING` makes it idempotent under re-apply / cross-node re-observation and
guarantees it can never raise (so it can never gate the apply path). `detected_at` is deliberately
**not** part of the key — it is node-local observation metadata (when *this* node first noticed),
intentionally non-convergent.

**The two colliding events are the natural key.** Two distinct `content_address`es globally identify two
distinct events; `overlay` is functionally determined by them (each event type routes to exactly one
overlay). `overlay` is included in the PK for defensiveness and readable per-overlay querying; the
triple/`subject_key` columns are descriptive redundancy (derivable, kept for worklist legibility).

### 3.3 Wiring — minimal edit to each of the five overlay triggers

Each overlay is an `AFTER INSERT` plpgsql trigger on `event_log` that already knows `NEW` and its
conflict key. Immediately **before** the existing (untouched) upsert, add a detect-and-record step:

```sql
SELECT hlc_wall, hlc_counter, origin, content_address
  INTO v_cur
  FROM <overlay_table> WHERE <conflict-key> = <NEW's key>;
IF FOUND AND cairn_hlc_triple_collision(
        NEW.hlc_wall, NEW.hlc_counter, NEW.node_origin, NEW.content_address,
        v_cur.hlc_wall, v_cur.hlc_counter, v_cur.origin, v_cur.content_address) THEN
    PERFORM cairn_record_hlc_collision(
        '<overlay>', <subject_key::text>,
        NEW.hlc_wall, NEW.hlc_counter, NEW.node_origin,
        NEW.content_address, v_cur.content_address);
END IF;
-- ... existing INSERT ... ON CONFLICT ... WHERE cairn_hlc_overlay_wins(...) unchanged ...
```

The existing resolution statement stays **byte-for-byte unchanged**. One extra indexed single-row
`SELECT` per applied event — negligible against the Bet B budget (p95 ~4 ms, large headroom).

Detection lives in the projection trigger, so it is **door-agnostic**: it fires for both `submit_event`
(db/005) and `apply_remote_event` (db/020) with zero changes to either door. (In practice only the
apply door, which takes the wire HLC verbatim, can drive two events into the same triple — but the
detection is placed where it is uniform and needs no door awareness.)

**`subject_key` rendering per overlay** (the heterogeneous conflict keys → text):

| Overlay (file) | Conflict key | `subject_key` |
|---|---|---|
| `patient_chart` (db/002) | `patient_id` (UUID) | `patient_id::text` |
| `patient_link` (db/018) | `(low, high)` (UUID pair) | `low::text \|\| '\|' \|\| high::text` |
| `chart_dispute` (db/023) | `dispute_id` (UUID) | `dispute_id::text` |
| `chart_identity_state` (db/024) | `subject` (UUID) | `subject::text` |
| `name_repudiation` (db/025) | `(subject, value)` | `subject::text \|\| '\|' \|\| value` |

For `patient_chart` the "current side" is the winning **demographic** event, so `v_cur` reads the
`demo_hlc_wall / demo_hlc_count / demo_origin / demo_content_address` columns (nullable on a note-only
row — `FOUND` + null-safe predicate handle the absent-winner case: no collision recorded, correct).

### 3.4 File placement

- **New `db/029_hlc_collision_log.sql`** — defines `cairn_hlc_triple_collision`, `hlc_collision_log`,
  and `cairn_record_hlc_collision`. Loaded after the overlays; plpgsql late-binding makes the load order
  a non-issue (all DDL loads before any event is applied).
- **In-place edits** to `db/002`, `db/018`, `db/023`, `db/024`, `db/025` — the minimal detect-and-record
  block in each trigger, plus (in db/002's predicate comment) a one-line pointer to its db/029 sibling.
  In-place edits follow the pre-clinical posture and the #99/#115 hardening pattern.

## 4. Deliberately out of scope (YAGNI)

- **No Python §5.13 consumer.** The table is the read surface (`SELECT * FROM hlc_collision_log`);
  wiring it into the background duplicate/anomaly sweep or a human worklist is a documented future seam
  (analogous to the `localstate` DEK seams).
- **No resolution / ack state.** A collision has no "resolved" event yet; the table *is* the worklist.
  An ack/resolution overlay is a future slice if a surface ever needs one.
- **No VIEW.** A filtering VIEW adds nothing until there is ack state.
- **3-way collisions** (A, B, C all sharing one triple) are recorded **pairwise** — whichever pairs a
  node observes at its upserts, e.g. `(A,B)` then `(A,C)`. Recorded honestly (not collapsed into a
  set); noted as a limitation. Already deep in broken-signer territory.
- **No advisory signal on the pre-#115 demographic overlays** (db/010–014). #115 deliberately narrowed
  to the five uniform standing-state overlays; the demographic overlays' residual is #69's remit.

## 5. Tests (TDD, DB-gated Rust — mirrors `overlay_tiebreaker.rs`)

1. **Pure predicate** `cairn_hlc_triple_collision`:
   - true only when `(wall, counter, origin)` equal **and** `content_address` distinct;
   - false on any triple difference (wall, counter, or origin);
   - false when `content_address` identical (same event);
   - null-current side → false (absent winner is not a collision).
2. **Per overlay (all five)** — apply two HLC-colliding events (identical triple, distinct
   `content_address`) through the **remote-apply door** (`apply_remote_event`, the only door taking the
   wire HLC verbatim) in **both arrival orders**:
   - exactly **one** `hlc_collision_log` row for the pair;
   - **identical** `(addr_lo, addr_hi)` regardless of arrival order (the convergence claim);
   - correct `overlay` and `subject_key`.
3. **Negative** — two events with *distinct* HLC triples applied to the same overlay subject (a normal
   overlay) → **zero** collision rows.
4. **Idempotence** — re-applying the same colliding event (cross-node re-observation) → still exactly
   one row (`ON CONFLICT DO NOTHING`).

All existing `overlay_tiebreaker.rs` convergence tests must remain green (resolution unchanged); full
cairn-node DB-gated workspace green; fmt + clippy clean; mkdocs builds.

## 6. Principle check

- **Principle 1 (append-only + causal ordering):** the signal is a pure append; the event log and the
  resolution are untouched.
- **Principle 2 (identity is a claim; never merge/erase):** surfacing a Byzantine link/dispute collision
  for human review *strengthens* auditability of identity state.
- **Principle 9 (policy-neutral infrastructure):** the floor emits the durable mechanism only; whether
  and how to alert a human is a higher-tier policy decision.
- **Availability over consistency:** the record can never raise or block; a partition-time apply still
  succeeds, with the anomaly recorded for later human review.
