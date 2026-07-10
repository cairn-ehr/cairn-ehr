# HLC-triple Collision Advisory Signal (#157) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** When an overlay resolves a Byzantine HLC-triple collision (two distinct events sharing one `(hlc_wall, hlc_counter, origin)` triple, #115), record it as an append-only, convergent, advisory signal for later human / §5.13-sweep review — without changing the resolution or gating the apply path.

**Architecture:** A new `db/029_hlc_collision_log.sql` defines a shared pure predicate `cairn_hlc_triple_collision`, a convergent append-only `hlc_collision_log` table (canonical unordered `content_address` pair as the dedup key), and a never-raising `cairn_record_hlc_collision` recorder. Each of the five uniform standing-state overlay triggers (`db/002`, `db/018`, `db/023`, `db/024`, `db/025`) gets a minimal detect-and-record step *before* its existing (byte-for-byte unchanged) upsert. Detection lives in the projection trigger, so it is door-agnostic (fires for both `submit_event` and `apply_remote_event`).

**Tech Stack:** PostgreSQL ≥ 18 (SQL / PL-pgSQL, the "fat Postgres" floor tier), `cairn_pgx` (pgrx) for verify; Rust integration tests via `tokio-postgres`, gated on `$CAIRN_TEST_PG`, serialized cluster-wide by `db::test_serial_guard`.

## Global Constraints

- **AGPL-3.0**; every dependency AGPL-3.0-compatible (no new deps in this slice).
- **Safety-critical floor code** → SQL / PL-pgSQL, optimized for reviewer-legibility (§9 defect-blast-radius rule).
- **TDD** — failing test first, then the code that makes it pass.
- **Inline docs for a junior contributor** on every non-trivial function/block (house rule 3).
- **Advisory/observability only** — no `RAISE`, no veto; the recorder must never block the apply path (availability over consistency). The #115 resolution (`cairn_hlc_overlay_wins` + the `ON CONFLICT ... WHERE` upserts) stays **byte-for-byte unchanged**.
- **No wire / event-format / floor-gate / SCHEMA-array / ADR / spec change.** No new event type.
- **Pre-clinical posture:** existing migration files are edited **in place** (the #99/#115 pattern); the new objects live in a new `db/029` file.
- **New migrations must be registered** in the `SCHEMA` array in `crates/cairn-node/src/db.rs` (explicit `include_str!` list — files are not globbed).
- DB-gated tests need `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test"` (PG18 + `cairn_pgx`). Run from repo root: `CAIRN_TEST_PG=… cargo test -p cairn-node`.
- Verify commands (Task 7): `cargo fmt --all -- --check`; `cargo clippy --workspace --all-targets -- -D warnings`; `cargo test --workspace`; `uv run --with-requirements docs/requirements.txt -- mkdocs build`.

---

## File structure

| File | Responsibility | Action |
|---|---|---|
| `db/029_hlc_collision_log.sql` | The predicate, the append-only table, the recorder | **Create** |
| `crates/cairn-node/src/db.rs` | Register `029` in the `SCHEMA` load array | Modify (1 entry) |
| `db/002_projection.sql` | `patient_chart_apply` detect-and-record; predicate-comment pointer to db/029 | Modify (trigger body) |
| `db/018_identity_linkage.sql` | `patient_link_apply` detect-and-record | Modify (trigger body) |
| `db/023_identity_dispute.sql` | `chart_dispute_apply` detect-and-record | Modify (trigger body) |
| `db/024_identity_identify.sql` | `chart_identity_state_apply` detect-and-record | Modify (trigger body) |
| `db/025_identity_repudiate.sql` | `name_repudiation_apply` detect-and-record | Modify (trigger body) |
| `crates/cairn-node/tests/hlc_collision_signal.rs` | Pure-predicate + recorder unit tests (no event builders) | **Create** |
| `crates/cairn-node/tests/overlay_tiebreaker.rs` | Fold "exactly one convergent collision row" into the 5 convergence tests; add a negative test; harness truncation of `hlc_collision_log` | Modify |
| `docs/HANDOVER.md`, `docs/ROADMAP.md` | Reflect #157 done | Modify (Task 7) |

---

### Task 1: `db/029` floor objects + their direct unit tests

**Files:**
- Create: `db/029_hlc_collision_log.sql`
- Modify: `crates/cairn-node/src/db.rs` (add `029` to `SCHEMA`, after the `028_identity_evidence` entry, before the closing `];` at line 137)
- Create/Test: `crates/cairn-node/tests/hlc_collision_signal.rs`

**Interfaces:**
- Produces (SQL):
  - `cairn_hlc_triple_collision(new_wall bigint, new_counter int, new_origin text, new_addr bytea, cur_wall bigint, cur_counter int, cur_origin text, cur_addr bytea) RETURNS boolean` — `true` iff the HLC triples are equal and `content_address`es distinct.
  - `hlc_collision_log(overlay text, subject_key text, hlc_wall bigint, hlc_counter int, origin text, addr_lo bytea, addr_hi bytea, detected_at timestamptz)`, `PRIMARY KEY (overlay, addr_lo, addr_hi)`.
  - `cairn_record_hlc_collision(p_overlay text, p_subject_key text, p_wall bigint, p_counter int, p_origin text, p_addr_a bytea, p_addr_b bytea) RETURNS void` — canonicalizes the unordered pair and appends idempotently; never raises.
- Consumes: nothing from earlier tasks.

- [ ] **Step 1: Write the failing predicate + recorder unit tests**

Create `crates/cairn-node/tests/hlc_collision_signal.rs`:

```rust
//! Advisory Byzantine-collision signal (#157): the pure `cairn_hlc_triple_collision` predicate
//! and the convergent `cairn_record_hlc_collision` recorder, tested directly (no event builders).
//! The five per-overlay integration assertions live in `overlay_tiebreaker.rs` (they reuse that
//! file's event builders). Real Postgres, gated on `$CAIRN_TEST_PG`, serialized cluster-wide via
//! `db::test_serial_guard`.
use cairn_node::db;
use tokio_postgres::Client;

fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

/// Evaluate the pure collision predicate in the DB. Current side is non-null here (the overlays
/// only call it after a FOUND row); the null-current case is exercised via the overlays.
#[allow(clippy::too_many_arguments)] // mirrors the same-shaped `wins()` helper in overlay_tiebreaker.rs
async fn collides(
    c: &Client,
    nw: i64,
    nc: i32,
    no: &str,
    na: Vec<u8>,
    cw: i64,
    cc: i32,
    co: &str,
    ca: Vec<u8>,
) -> bool {
    c.query_one(
        "SELECT cairn_hlc_triple_collision($1,$2,$3,$4,$5,$6,$7,$8)",
        &[&nw, &nc, &no, &na, &cw, &cc, &co, &ca],
    )
    .await
    .unwrap()
    .get(0)
}

#[tokio::test]
async fn triple_collision_predicate_is_equal_triple_distinct_address() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();

    // Equal (wall, counter, origin) but DISTINCT content_address → collision.
    assert!(collides(&c, 5, 3, "peer", vec![1], 5, 3, "peer", vec![2]).await);
    assert!(collides(&c, 5, 3, "peer", vec![2], 5, 3, "peer", vec![1]).await);
    // Identical address (same event, an idempotent re-apply) → NOT a collision.
    assert!(!collides(&c, 5, 3, "peer", vec![7], 5, 3, "peer", vec![7]).await);
    // Any triple difference → NOT a collision, even with distinct addresses.
    assert!(!collides(&c, 6, 3, "peer", vec![1], 5, 3, "peer", vec![2]).await); // wall
    assert!(!collides(&c, 5, 4, "peer", vec![1], 5, 3, "peer", vec![2]).await); // counter
    assert!(!collides(&c, 5, 3, "peerX", vec![1], 5, 3, "peer", vec![2]).await); // origin
}

#[tokio::test]
async fn recorder_canonicalizes_and_dedups_the_unordered_pair() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    c.batch_execute("TRUNCATE hlc_collision_log").await.unwrap();

    let a: Vec<u8> = vec![0x11, 0x20, 0x01];
    let b: Vec<u8> = vec![0x11, 0x20, 0x02]; // b > a by byte comparison

    // Record the SAME collision twice with the address arguments in OPPOSITE order — the
    // set-union convergence claim: arrival order must not change the stored row.
    c.execute(
        "SELECT cairn_record_hlc_collision('t','s',5,3,'peer',$1,$2)",
        &[&a, &b],
    )
    .await
    .unwrap();
    c.execute(
        "SELECT cairn_record_hlc_collision('t','s',5,3,'peer',$1,$2)",
        &[&b, &a],
    )
    .await
    .unwrap();

    let row = c
        .query_one(
            "SELECT count(*)::int, min(addr_lo), max(addr_hi) FROM hlc_collision_log WHERE overlay='t'",
            &[],
        )
        .await
        .unwrap();
    let n: i32 = row.get(0);
    let lo: Vec<u8> = row.get(1);
    let hi: Vec<u8> = row.get(2);
    assert_eq!(n, 1, "the unordered pair dedups to exactly one row");
    assert_eq!(lo, a, "addr_lo is the byte-lesser address");
    assert_eq!(hi, b, "addr_hi is the byte-greater address");
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test -p cairn-node --test hlc_collision_signal -- --nocapture`
Expected: FAIL — the schema load errors (`function cairn_hlc_triple_collision does not exist` / `relation "hlc_collision_log" does not exist`), because `db/029` does not exist yet.

- [ ] **Step 3: Create `db/029_hlc_collision_log.sql`**

```sql
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
-- inline in the overlay triggers. IS [NOT] DISTINCT FROM keeps it null-total (a real event always
-- has a non-null triple; the overlays only call it after a FOUND current row, but null-safety keeps
-- the predicate honest for the note-only patient_chart row whose demographic winner is still null).
CREATE OR REPLACE FUNCTION cairn_hlc_triple_collision(
    new_wall bigint, new_counter int, new_origin text, new_addr bytea,
    cur_wall bigint, cur_counter int, cur_origin text, cur_addr bytea
) RETURNS boolean LANGUAGE sql IMMUTABLE AS $$
    SELECT new_wall = cur_wall
       AND new_counter = cur_counter
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
```

- [ ] **Step 4: Register `db/029` in the schema load array**

In `crates/cairn-node/src/db.rs`, add this entry immediately after the `028_identity_evidence` tuple and before the closing `];`:

```rust
    // #157: the Byzantine HLC-triple collision advisory signal. Defines the shared
    // cairn_hlc_triple_collision predicate + the convergent hlc_collision_log + the never-gating
    // recorder; the five overlay triggers (db/002/018/023/024/025) call the recorder. PL/pgSQL is
    // late-bound, so those triggers may reference this file's functions before it loads — all
    // migrations load before any event is applied.
    (
        "029_hlc_collision_log",
        include_str!("../../../db/029_hlc_collision_log.sql"),
    ),
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test -p cairn-node --test hlc_collision_signal -- --nocapture`
Expected: PASS (2 tests) — `triple_collision_predicate_is_equal_triple_distinct_address`, `recorder_canonicalizes_and_dedups_the_unordered_pair`.

- [ ] **Step 6: Commit**

```bash
git add db/029_hlc_collision_log.sql crates/cairn-node/src/db.rs crates/cairn-node/tests/hlc_collision_signal.rs
git commit -m "feat(floor): db/029 HLC-triple collision advisory signal — predicate/table/recorder (#157)"
```

---

### Task 2: Wire `patient_chart` (db/002) + convergence-signal harness + negative test

**Files:**
- Modify: `db/002_projection.sql` (`patient_chart_apply`, the `patient.created`/`patient.amended` branch, lines ~66-98)
- Modify: `crates/cairn-node/tests/overlay_tiebreaker.rs` (add `collision_rows` helper; truncate `hlc_collision_log` in `setup` + `reset_between_orders`; fold assertion into `patient_chart_converges_under_hlc_collision`; add `distinct_triples_record_no_collision`)

**Interfaces:**
- Consumes: `cairn_hlc_triple_collision`, `cairn_record_hlc_collision`, `hlc_collision_log` (Task 1); the existing `setup`, `apply`, `reset_between_orders`, `amended_event` helpers in `overlay_tiebreaker.rs`.
- Produces: the `collision_rows(c, overlay) -> Vec<(Vec<u8>, Vec<u8>)>` test helper reused by Tasks 3-6; `patient_chart` collision recording (`overlay = 'patient_chart'`, `subject_key = patient_id::text`).

- [ ] **Step 1: Add the failing test assertions + helper**

In `crates/cairn-node/tests/overlay_tiebreaker.rs`, add the helper near `wins` (after line 49):

```rust
/// All recorded collision rows for an overlay, as (addr_lo, addr_hi) byte pairs, ordered so the
/// vec is comparable across arrival orders. Empty when no Byzantine collision was detected.
async fn collision_rows(c: &Client, overlay: &str) -> Vec<(Vec<u8>, Vec<u8>)> {
    c.query(
        "SELECT addr_lo, addr_hi FROM hlc_collision_log WHERE overlay = $1 ORDER BY addr_lo, addr_hi",
        &[&overlay],
    )
    .await
    .unwrap()
    .iter()
    .map(|r| (r.get(0), r.get(1)))
    .collect()
}
```

Add `hlc_collision_log` to the unconditional `TRUNCATE` in **both** `setup` (line 145-148) and `reset_between_orders` (line 189-192) — append `, hlc_collision_log` to each `TRUNCATE event_log, ...` list so each arrival-order run starts with an empty signal log.

Fold the signal assertion into `patient_chart_converges_under_hlc_collision` — after the order-1 read (`n1`, ~line 730) capture the single collision row, and after the order-2 read (`n2`, ~line 742) assert it is identical. Insert before the final `assert_eq!(n1, n2, ...)`:

```rust
    // #157: the resolved collision is also SURFACED — exactly one advisory row, identical across
    // both arrival orders (the signal is itself a convergent set-union projection).
    let sig2 = collision_rows(&c, "patient_chart").await;
    assert_eq!(sig1.len(), 1, "one collision recorded, order 1");
    assert_eq!(sig2.len(), 1, "one collision recorded, order 2");
    assert_eq!(sig1, sig2, "the advisory signal converges across arrival order (#157)");
```

and capture `sig1` right after the `n1` query (before `reset_between_orders`):

```rust
    let sig1 = collision_rows(&c, "patient_chart").await;
```

Add the negative test at the end of the file:

```rust
#[tokio::test]
async fn distinct_triples_record_no_collision() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid, _sk_h, _kid_h) = setup(&c).await;

    // Two ordinary amendments at DIFFERENT HLC triples (wall 5000 then 5001) — a normal overlay,
    // never a Byzantine collision. No advisory row must be recorded.
    let p = Uuid::now_v7();
    let e_a = sign(&amended_event(&kid, p, "Alice A", 5000, 7), &sk)
        .unwrap()
        .signed_bytes;
    let e_b = sign(&amended_event(&kid, p, "Bob B", 5001, 0), &sk)
        .unwrap()
        .signed_bytes;
    apply(&c, &e_a).await.expect("amend A applies");
    apply(&c, &e_b).await.expect("amend B applies");

    assert!(
        collision_rows(&c, "patient_chart").await.is_empty(),
        "distinct HLC triples are normal overlay, not a Byzantine collision (#157)"
    );
}
```

- [ ] **Step 2: Run to verify failure**

Run: `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test -p cairn-node --test overlay_tiebreaker patient_chart -- --nocapture`
Expected: FAIL — `patient_chart_converges_under_hlc_collision` asserts `sig1.len() == 1` but the trigger records nothing yet (0 rows); `distinct_triples_record_no_collision` passes (0 rows is correct even before wiring).

- [ ] **Step 3: Wire detection into `patient_chart_apply`**

In `db/002_projection.sql`, change the function header to declare a current-side record, and add the detect-and-record step at the top of the demographic branch. Replace the header line `RETURNS trigger LANGUAGE plpgsql AS $$` + `BEGIN` (lines 64-65) and the branch opener (line 66) so it reads:

```sql
RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE
    v_cur record;  -- current demographic winner, for #157 collision detection
BEGIN
    IF NEW.event_type IN ('patient.created', 'patient.amended') THEN
        -- #157: before overlaying, detect a Byzantine HLC-triple collision against the current
        -- demographic winner and record an advisory signal. Reads the demo_* provenance columns
        -- aliased to the predicate's parameter names; a note-only row has null demo_* → the
        -- null-safe predicate returns false (no false signal).
        SELECT demo_hlc_wall AS hlc_wall, demo_hlc_count AS hlc_counter,
               demo_origin AS origin, demo_content_address AS content_address
          INTO v_cur
          FROM patient_chart WHERE patient_id = NEW.patient_id;
        IF FOUND AND cairn_hlc_triple_collision(
                NEW.hlc_wall, NEW.hlc_counter, NEW.node_origin, NEW.content_address,
                v_cur.hlc_wall, v_cur.hlc_counter, v_cur.origin, v_cur.content_address) THEN
            PERFORM cairn_record_hlc_collision(
                'patient_chart', NEW.patient_id::text,
                NEW.hlc_wall, NEW.hlc_counter, NEW.node_origin,
                NEW.content_address, v_cur.content_address);
        END IF;

        INSERT INTO patient_chart AS pc (
```

(The existing `INSERT INTO patient_chart AS pc (...)` and everything after it is unchanged.) Also append a one-line pointer to the `cairn_hlc_overlay_wins` comment block (near line 32): `-- Its #157 collision-detection sibling (cairn_hlc_triple_collision + hlc_collision_log) lives in db/029.`

- [ ] **Step 4: Run to verify pass**

Run: `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test -p cairn-node --test overlay_tiebreaker -- --nocapture`
Expected: PASS — `patient_chart_converges_under_hlc_collision` (now with the signal assertions), `distinct_triples_record_no_collision`, and the four other convergence tests (still resolution-only, unchanged) all green.

- [ ] **Step 5: Commit**

```bash
git add db/002_projection.sql crates/cairn-node/tests/overlay_tiebreaker.rs
git commit -m "feat(floor): patient_chart overlay records the Byzantine HLC collision signal (#157)"
```

---

### Task 3: Wire `patient_link` (db/018) + convergence-signal assertion

**Files:**
- Modify: `db/018_identity_linkage.sql` (`patient_link_apply`, DECLARE at lines 233-239, body from line 240)
- Modify: `crates/cairn-node/tests/overlay_tiebreaker.rs` (`patient_link_converges_under_hlc_collision`)

**Interfaces:**
- Consumes: Task 1 SQL; Task 2's `collision_rows` helper + harness truncation.
- Produces: `patient_link` collision recording (`overlay = 'patient_link'`, `subject_key = lo::text || '|' || hi::text`).

- [ ] **Step 1: Add the failing signal assertion**

In `patient_link_converges_under_hlc_collision`, capture `sig1` right after the `state1` query (~line 286) and assert convergence before the final `assert_eq!(state1, state2, ...)`:

```rust
    let sig1 = collision_rows(&c, "patient_link").await;   // after the state1 read
```
```rust
    // #157: the resolved link/unlink collision is surfaced — one convergent advisory row.
    let sig2 = collision_rows(&c, "patient_link").await;
    assert_eq!(sig1.len(), 1, "one collision recorded, order 1");
    assert_eq!(sig2.len(), 1, "one collision recorded, order 2");
    assert_eq!(sig1, sig2, "the advisory signal converges across arrival order (#157)");
```

- [ ] **Step 2: Run to verify failure**

Run: `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test -p cairn-node --test overlay_tiebreaker patient_link -- --nocapture`
Expected: FAIL — `sig1.len() == 1` fails (0 rows; the trigger does not record yet).

- [ ] **Step 3: Wire detection into `patient_link_apply`**

In `db/018_identity_linkage.sql`, add `v_cur record;` to the DECLARE block (after line 239's `v_state` line), and insert the detect-and-record step immediately after the advisory-lock `PERFORM pg_advisory_xact_lock(...)` (line 249), before the `INSERT INTO patient_link`:

```sql
    -- #157: detect a Byzantine HLC-triple collision against the current standing link and record
    -- an advisory signal before overlaying. lo/hi are the canonical pair already computed above.
    SELECT hlc_wall, hlc_counter, origin, content_address
      INTO v_cur
      FROM patient_link WHERE low = lo AND high = hi;
    IF FOUND AND cairn_hlc_triple_collision(
            NEW.hlc_wall, NEW.hlc_counter, NEW.node_origin, NEW.content_address,
            v_cur.hlc_wall, v_cur.hlc_counter, v_cur.origin, v_cur.content_address) THEN
        PERFORM cairn_record_hlc_collision(
            'patient_link', lo::text || '|' || hi::text,
            NEW.hlc_wall, NEW.hlc_counter, NEW.node_origin,
            NEW.content_address, v_cur.content_address);
    END IF;
```

- [ ] **Step 4: Run to verify pass**

Run: `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test -p cairn-node --test overlay_tiebreaker patient_link -- --nocapture`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add db/018_identity_linkage.sql crates/cairn-node/tests/overlay_tiebreaker.rs
git commit -m "feat(floor): patient_link overlay records the Byzantine HLC collision signal (#157)"
```

---

### Task 4: Wire `chart_dispute` (db/023) + convergence-signal assertion

**Files:**
- Modify: `db/023_identity_dispute.sql` (`chart_dispute_apply`, DECLARE at lines 150-156, body from line 157)
- Modify: `crates/cairn-node/tests/overlay_tiebreaker.rs` (`chart_dispute_converges_under_hlc_collision`)

**Interfaces:**
- Consumes: Task 1 SQL; Task 2 harness.
- Produces: `chart_dispute` collision recording (`overlay = 'chart_dispute'`, `subject_key = p->>'dispute_id'`).

- [ ] **Step 1: Add the failing signal assertion**

In `chart_dispute_converges_under_hlc_collision`, capture `sig1` after the `s1` query (~line 407) and assert before the final `assert_eq!(s1, s2, ...)`:

```rust
    let sig1 = collision_rows(&c, "chart_dispute").await;   // after the s1 read
```
```rust
    // #157: the resolved open-vs-resolved collision is surfaced — one convergent advisory row.
    let sig2 = collision_rows(&c, "chart_dispute").await;
    assert_eq!(sig1.len(), 1, "one collision recorded, order 1");
    assert_eq!(sig2.len(), 1, "one collision recorded, order 2");
    assert_eq!(sig1, sig2, "the advisory signal converges across arrival order (#157)");
```

- [ ] **Step 2: Run to verify failure**

Run: `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test -p cairn-node --test overlay_tiebreaker chart_dispute -- --nocapture`
Expected: FAIL — `sig1.len() == 1` fails (0 rows).

- [ ] **Step 3: Wire detection into `chart_dispute_apply`**

In `db/023_identity_dispute.sql`, add `v_cur record;` to the DECLARE block (after line 156's `v_detail`), and insert the detect-and-record step immediately after `BEGIN` (line 157), before the subject-consistency guard `IF current_setting(...)`:

```sql
    -- #157: detect a Byzantine HLC-triple collision against the current dispute state and record
    -- an advisory signal before overlaying. Placed before the subject-consistency guard so the
    -- collision is observed regardless of which door (local submit / remote apply) we are on.
    SELECT hlc_wall, hlc_counter, origin, content_address
      INTO v_cur
      FROM chart_dispute WHERE dispute_id = (p ->> 'dispute_id')::uuid;
    IF FOUND AND cairn_hlc_triple_collision(
            NEW.hlc_wall, NEW.hlc_counter, NEW.node_origin, NEW.content_address,
            v_cur.hlc_wall, v_cur.hlc_counter, v_cur.origin, v_cur.content_address) THEN
        PERFORM cairn_record_hlc_collision(
            'chart_dispute', p ->> 'dispute_id',
            NEW.hlc_wall, NEW.hlc_counter, NEW.node_origin,
            NEW.content_address, v_cur.content_address);
    END IF;
```

- [ ] **Step 4: Run to verify pass**

Run: `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test -p cairn-node --test overlay_tiebreaker chart_dispute -- --nocapture`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add db/023_identity_dispute.sql crates/cairn-node/tests/overlay_tiebreaker.rs
git commit -m "feat(floor): chart_dispute overlay records the Byzantine HLC collision signal (#157)"
```

---

### Task 5: Wire `chart_identity_state` (db/024) + convergence-signal assertion

**Files:**
- Modify: `db/024_identity_identify.sql` (`chart_identity_state_apply`, DECLARE at lines 160-168, body from line 169)
- Modify: `crates/cairn-node/tests/overlay_tiebreaker.rs` (`chart_identity_state_converges_under_hlc_collision`)

**Interfaces:**
- Consumes: Task 1 SQL; Task 2 harness.
- Produces: `chart_identity_state` collision recording (`overlay = 'chart_identity_state'`, `subject_key = p->>'subject'`).

- [ ] **Step 1: Add the failing signal assertion**

In `chart_identity_state_converges_under_hlc_collision`, capture `sig1` after the `s1` query (~line 523) and assert before the final `assert_eq!(s1, s2, ...)`:

```rust
    let sig1 = collision_rows(&c, "chart_identity_state").await;   // after the s1 read
```
```rust
    // #157: the resolved pending-vs-identified collision is surfaced — one convergent advisory row.
    let sig2 = collision_rows(&c, "chart_identity_state").await;
    assert_eq!(sig1.len(), 1, "one collision recorded, order 1");
    assert_eq!(sig2.len(), 1, "one collision recorded, order 2");
    assert_eq!(sig1, sig2, "the advisory signal converges across arrival order (#157)");
```

- [ ] **Step 2: Run to verify failure**

Run: `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test -p cairn-node --test overlay_tiebreaker chart_identity_state -- --nocapture`
Expected: FAIL — `sig1.len() == 1` fails (0 rows).

- [ ] **Step 3: Wire detection into `chart_identity_state_apply`**

In `db/024_identity_identify.sql`, add `v_cur record;` to the DECLARE block (after line 168's `v_detail`), and insert the detect-and-record step immediately after `BEGIN` (line 169), before the `INSERT INTO chart_identity_state`:

```sql
    -- #157: detect a Byzantine HLC-triple collision against the current identity state and record
    -- an advisory signal before overlaying pending-vs-identified.
    SELECT hlc_wall, hlc_counter, origin, content_address
      INTO v_cur
      FROM chart_identity_state WHERE subject = (p ->> 'subject')::uuid;
    IF FOUND AND cairn_hlc_triple_collision(
            NEW.hlc_wall, NEW.hlc_counter, NEW.node_origin, NEW.content_address,
            v_cur.hlc_wall, v_cur.hlc_counter, v_cur.origin, v_cur.content_address) THEN
        PERFORM cairn_record_hlc_collision(
            'chart_identity_state', p ->> 'subject',
            NEW.hlc_wall, NEW.hlc_counter, NEW.node_origin,
            NEW.content_address, v_cur.content_address);
    END IF;
```

- [ ] **Step 4: Run to verify pass**

Run: `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test -p cairn-node --test overlay_tiebreaker chart_identity_state -- --nocapture`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add db/024_identity_identify.sql crates/cairn-node/tests/overlay_tiebreaker.rs
git commit -m "feat(floor): chart_identity_state overlay records the Byzantine HLC collision signal (#157)"
```

---

### Task 6: Wire `name_repudiation` (db/025) + convergence-signal assertion

**Files:**
- Modify: `db/025_identity_repudiate.sql` (`name_repudiation_apply`, DECLARE at line 180, body from line 181)
- Modify: `crates/cairn-node/tests/overlay_tiebreaker.rs` (`name_repudiation_converges_under_hlc_collision`)

**Interfaces:**
- Consumes: Task 1 SQL; Task 2 harness.
- Produces: `name_repudiation` collision recording (`overlay = 'name_repudiation'`, `subject_key = (p->>'subject') || '|' || (p->>'value')`).

- [ ] **Step 1: Add the failing signal assertion**

In `name_repudiation_converges_under_hlc_collision`, capture `sig1` after the `r1` query (~line 647) and assert before the final `assert_eq!(r1, r2, ...)`:

```rust
    let sig1 = collision_rows(&c, "name_repudiation").await;   // after the r1 read
```
```rust
    // #157: the resolved repudiation-reason collision is surfaced — one convergent advisory row.
    let sig2 = collision_rows(&c, "name_repudiation").await;
    assert_eq!(sig1.len(), 1, "one collision recorded, order 1");
    assert_eq!(sig2.len(), 1, "one collision recorded, order 2");
    assert_eq!(sig1, sig2, "the advisory signal converges across arrival order (#157)");
```

- [ ] **Step 2: Run to verify failure**

Run: `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test -p cairn-node --test overlay_tiebreaker name_repudiation -- --nocapture`
Expected: FAIL — `sig1.len() == 1` fails (0 rows).

- [ ] **Step 3: Wire detection into `name_repudiation_apply`**

In `db/025_identity_repudiate.sql`, add `v_cur record;` to the DECLARE block (after line 180's `p jsonb := NEW.body;`), and insert the detect-and-record step immediately after `BEGIN` (line 181), before the `INSERT INTO name_repudiation`:

```sql
    -- #157: detect a Byzantine HLC-triple collision against the current repudiation of this exact
    -- (subject, value) and record an advisory signal before overlaying the winning `reason`. Note
    -- the SUPPRESSION decision itself is value-keyed + idempotent (see the #69 note below), so this
    -- only ever surfaces which advisory `reason` the collision resolved to — never un-suppresses.
    SELECT hlc_wall, hlc_counter, origin, content_address
      INTO v_cur
      FROM name_repudiation
      WHERE subject = (p ->> 'subject')::uuid AND value = p ->> 'value';
    IF FOUND AND cairn_hlc_triple_collision(
            NEW.hlc_wall, NEW.hlc_counter, NEW.node_origin, NEW.content_address,
            v_cur.hlc_wall, v_cur.hlc_counter, v_cur.origin, v_cur.content_address) THEN
        PERFORM cairn_record_hlc_collision(
            'name_repudiation', (p ->> 'subject') || '|' || (p ->> 'value'),
            NEW.hlc_wall, NEW.hlc_counter, NEW.node_origin,
            NEW.content_address, v_cur.content_address);
    END IF;
```

- [ ] **Step 4: Run to verify pass**

Run: `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test -p cairn-node --test overlay_tiebreaker name_repudiation -- --nocapture`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add db/025_identity_repudiate.sql crates/cairn-node/tests/overlay_tiebreaker.rs
git commit -m "feat(floor): name_repudiation overlay records the Byzantine HLC collision signal (#157)"
```

---

### Task 7: Whole-workspace verification + docs

**Files:**
- Modify: `docs/HANDOVER.md`, `docs/ROADMAP.md`

- [ ] **Step 1: Format + lint**

Run: `cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings`
Expected: no diff, no warnings. (If fmt reports a diff, run `cargo fmt --all` and re-stage.)

- [ ] **Step 2: Full DB-gated workspace suite**

Run: `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test --workspace -- --nocapture`
Expected: all green — the two new `hlc_collision_signal` tests, the six `overlay_tiebreaker` convergence tests (now asserting the signal) + the negative test, and NO regressions across identity/demographics/sync suites. Confirm the five `*_converges_under_hlc_collision` resolution assertions still pass (the #115 resolution is unchanged).

- [ ] **Step 3: Docs build**

Run: `uv run --with-requirements docs/requirements.txt -- mkdocs build`
Expected: builds clean (no new pages; sanity check only).

- [ ] **Step 4: Update HANDOVER.md and ROADMAP.md**

In `docs/HANDOVER.md`: replace the "PR-review follow-on filed (#157)" note in the current-session block with a concise "done" summary (advisory `hlc_collision_log` recorded at all five overlays via a shared `cairn_hlc_triple_collision` predicate + convergent recorder; projection-read-side only; no wire/floor-gate/ADR/spec change; the Python §5.13-sweep consumer is a documented future seam). Prune older session blocks as needed to keep the file under 500 lines.

In `docs/ROADMAP.md`: extend the Phase 2 "Deterministic overlay convergence" bullet with a sentence noting the #157 advisory signal is now recorded (append-only `hlc_collision_log`, convergent, non-gating), and mark #157 done. Keep under 500 lines.

- [ ] **Step 5: Commit**

```bash
git add docs/HANDOVER.md docs/ROADMAP.md
git commit -m "docs: HANDOVER/ROADMAP — HLC-triple collision advisory signal done (#157)"
```

---

## Self-review notes

- **Spec coverage:** §3.1 predicate → Task 1; §3.2 table+recorder → Task 1; §3.3 five-overlay wiring (incl. per-overlay `subject_key` + current-side columns) → Tasks 2-6; §3.4 file placement (`db/029` + in-place edits + `db.rs` registration) → Tasks 1-6; §5 tests (pure predicate, per-overlay convergence in both orders, negative, idempotence) → Task 1 (predicate + recorder idempotence), Tasks 2-6 (per-overlay convergence), Task 2 (negative). All covered.
- **Convergence claim** is tested two ways: the recorder unit test (swapped-arg dedup, Task 1) and the per-overlay both-arrival-orders assertion (Tasks 2-6).
- **Non-gating** is structural (`ON CONFLICT DO NOTHING`, no `RAISE`) and implicitly held by every convergence test still passing (the applies all `.expect(...)` success).
- **`subject_key` rendering** is consistent per the design table: `patient_id::text` / `lo||'|'||hi` / `dispute_id` / `subject` / `subject||'|'||value`.
- **3-way collisions** recorded pairwise — an accepted, documented limit (design §4); no task needed.
