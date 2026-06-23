# cairn-node Incremental Sync Watermark + Genesis HLC — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace cairn-node's full-pull-every-cycle with a skip-proof incremental `node_event` sync (local-insertion `seq` cursor + a full-sweep correctness floor), and make the genesis/peer HLC real instead of `0/0`.

**Architecture:** The serving node orders the stream by its own monotonic `seq` (a `BIGINT GENERATED ALWAYS AS IDENTITY`). The puller keeps a per-peer cursor and pulls `seq > cursor`; because newly-learned events always get a fresh high `seq`, incremental can never permanently skip a legitimate event. A periodic/trust-change-triggered full-sweep (`after_seq = 0`) is the correctness floor for the residual commit-order/rejection/remap edges. The cursor is written only through an advance-only `SECURITY DEFINER` door, so the runtime role keeps zero raw DML. Genesis HLC is fixed by a local `hlc_state` clock (`node_hlc_tick()` + merge-forward on apply), mirroring the proven `cairn-sync` pattern.

**Tech Stack:** Rust (tokio, tokio-postgres, tokio-rustls), PostgreSQL ≥16 with the `cairn_pgx` extension (`cairn_verify`/`cairn_body`), `cairn-event` (COSE_Sign1 + Ed25519).

**Spec:** [docs/superpowers/specs/2026-06-23-cairn-node-incremental-sync-design.md](../specs/2026-06-23-cairn-node-incremental-sync-design.md) · **Issue:** [#38](https://github.com/cairn-ehr/cairn-ehr/issues/38)

## Global Constraints

- **License:** AGPL-3.0; every new dependency must be AGPL-3.0-compatible. No new deps are needed for this plan — do not add any.
- **TDD:** failing test first, then minimal code. Safety-critical surface (sync convergence, the in-DB floor).
- **Inline docs for a junior dev:** every new function/SQL object carries a comment explaining *why* and *how it fits*, matching the existing density in `db/007_node_federation.sql` and `sync.rs`.
- **Schema evolution is additive only (ADR-0012):** new columns via `ADD COLUMN IF NOT EXISTS`; new tables via `CREATE TABLE IF NOT EXISTS`. Never alter/drop an existing column.
- **The wire is the principle-12 inter-node path:** the signed event core must stay byte-identical; `seq` is transport metadata only. The request enum is versioned; future changes additive.
- **The floor invariant (PR #39):** the runtime `cairn_node` role does **zero raw DML** — only `EXECUTE` on `SECURITY DEFINER` doors + `SELECT`. New mutable state is written through a validated door, never a raw grant.
- **Branch:** `harden-node-incremental-sync` (already created; the design doc is committed there).
- **DB-gated tests** read `CAIRN_TEST_PG` / `CAIRN_TEST_PG2` / `CAIRN_TEST_PG3` (fresh throwaway DBs with `cairn_pgx` installed) and serialize cluster-wide via `db::test_serial_guard`. They skip cleanly when the env vars are unset.

---

## File Structure

- `db/007_node_federation.sql` — **modify.** Add `node_event.seq` + index; `sync_cursor` table + `checkpoint_sync_cursor` door + grants; `hlc_state` table + `node_hlc_tick()` + merge-forward in `apply_remote_node_event` + grants.
- `crates/cairn-node/src/identity.rs` — **modify.** `provision`/`author_peer`/`author_unpeer` tick the real HLC before signing (replace the hardcoded `0,0`).
- `crates/cairn-node/src/sync.rs` — **modify.** `Request::NodeEventsAfterSeq`; `stream_node_events(after_seq)` with seq-prefixed frames; `pull_into`/`pull_once` gain `full_sweep` + cursor read/checkpoint; `run` full-sweep cadence + trust-change trigger.
- `crates/cairn-node/src/main.rs` — **modify (small).** Update the `pull_once` call site in the `Serve`/`Run`/pull CLI path to pass `full_sweep`.
- `crates/cairn-node/tests/sync_watermark.rs` — **create.** Incremental-only-new + the out-of-order-skip-reconciled-by-full-sweep acceptance test + wire-seq-not-in-core + cursor-door advance-only / no-raw-DML.
- `crates/cairn-node/tests/genesis_hlc.rs` — **create.** Genesis HLC non-zero + advances across events + trust_peer orders by real HLC.
- `crates/cairn-node/tests/federation.rs` — **modify.** TRUNCATE `sync_cursor`; update `pull_once` call sites to pass `full_sweep`; extend convergence to a second incremental pull.

---

## Task 1: `node_event.seq` ordering column

**Files:**
- Modify: `db/007_node_federation.sql` (after the `CREATE TABLE node_event` block, near the existing indexes ~line 33)
- Test: `crates/cairn-node/tests/sync_watermark.rs` (create)

**Interfaces:**
- Produces: `node_event.seq BIGINT` — monotonic local insertion order, auto-assigned on INSERT; `node_event_seq_idx` index.

- [ ] **Step 1: Write the failing test**

Create `crates/cairn-node/tests/sync_watermark.rs`:

```rust
//! Issue #38 — incremental sync watermark (local-insertion `seq` cursor) and the
//! full-sweep correctness floor. DB-gated: needs CAIRN_TEST_PG (fresh DB + cairn_pgx).

use cairn_node::{db, identity, keystore};

fn cs() -> Option<String> { std::env::var("CAIRN_TEST_PG").ok() }

/// Every node_event row gets a monotonic `seq` assigned at INSERT, and the genesis
/// enroll's seq is the lowest (insertion order == seq order on this node).
#[tokio::test]
async fn node_event_seq_is_monotonic_on_insert() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let a = db::connect_and_load_schema(&base).await.unwrap();
    a.batch_execute("TRUNCATE node_event, local_node, sync_cursor").await.ok();

    let tmp = tempfile::tempdir().unwrap();
    let (sk, kid) = keystore::generate_and_seal(&tmp.path().join("a.key"), None).unwrap();
    identity::provision(&a, &sk, &kid, "A", "127.0.0.1:7900").await.unwrap();

    // After genesis there is exactly one row, and its seq is NOT NULL and >= 1.
    let row = a.query_one(
        "SELECT count(*) AS n, min(seq) AS lo, max(seq) AS hi FROM node_event", &[]
    ).await.unwrap();
    let n: i64 = row.get("n");
    let lo: i64 = row.get("lo");
    let hi: i64 = row.get("hi");
    assert_eq!(n, 1, "exactly the genesis enroll");
    assert_eq!(lo, hi, "single row: lo == hi");
    assert!(lo >= 1, "seq is assigned (>= 1), got {lo}");
}
```

- [ ] **Step 2: Run it to verify it fails**

Run: `CAIRN_TEST_PG="$CAIRN_TEST_PG" cargo test -p cairn-node --test sync_watermark node_event_seq_is_monotonic_on_insert -- --nocapture`
Expected: FAIL — `column "seq" does not exist` (or the test binary fails to find the column).

- [ ] **Step 3: Add the column + index**

In `db/007_node_federation.sql`, immediately after the two existing `CREATE INDEX IF NOT EXISTS node_event_*` lines (~line 34), add:

```sql
-- Issue #38: a monotonic, node-LOCAL insertion-order key for incremental sync.
-- This is the watermark the puller cursors on (NOT the HLC and NOT recorded_at):
-- a node that newly LEARNS an event inserts it with a fresh high `seq`, so new
-- knowledge always sorts above any puller's cursor and can never be silently
-- skipped. `seq` is sync transport metadata only — never signed, never on the wire
-- core. Additive (ADR-0012): ADD COLUMN IF NOT EXISTS does not fire the append-only
-- row trigger (that fires on UPDATE/DELETE), and IDENTITY is assigned at INSERT so
-- the existing INSERT column lists need no change.
ALTER TABLE node_event ADD COLUMN IF NOT EXISTS seq BIGINT GENERATED ALWAYS AS IDENTITY;
CREATE INDEX IF NOT EXISTS node_event_seq_idx ON node_event (seq);
```

- [ ] **Step 4: Run it to verify it passes**

Run: `CAIRN_TEST_PG="$CAIRN_TEST_PG" cargo test -p cairn-node --test sync_watermark node_event_seq_is_monotonic_on_insert -- --nocapture`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add db/007_node_federation.sql crates/cairn-node/tests/sync_watermark.rs
git commit -m "feat(cairn-node): add monotonic node_event.seq ordering key (issue #38)"
```

---

## Task 2: `sync_cursor` table + advance-only `checkpoint_sync_cursor` door

**Files:**
- Modify: `db/007_node_federation.sql` (after the `seq` column from Task 1, before the `REVOKE/GRANT` floor block ~line 159)
- Test: `crates/cairn-node/tests/sync_watermark.rs`

**Interfaces:**
- Produces: table `sync_cursor(peer_addr TEXT PK, last_seq BIGINT, updated_at TIMESTAMPTZ)`; function `checkpoint_sync_cursor(p_peer_addr TEXT, p_observed_seq BIGINT) RETURNS BIGINT` (returns the resulting `last_seq`); `EXECUTE` granted to `cairn_node`, `SELECT` on the table granted to `cairn_node`, **no** raw DML grant.

- [ ] **Step 1: Write the failing test**

Append to `crates/cairn-node/tests/sync_watermark.rs`:

```rust
/// The cursor door is advance-only: a lower observed_seq is a no-op, never a rewind.
#[tokio::test]
async fn checkpoint_sync_cursor_is_advance_only() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let a = db::connect_and_load_schema(&base).await.unwrap();
    a.batch_execute("TRUNCATE node_event, local_node, sync_cursor").await.ok();

    let peer = "127.0.0.1:7901";
    let s1: i64 = a.query_one("SELECT checkpoint_sync_cursor($1,$2)", &[&peer, &10_i64])
        .await.unwrap().get(0);
    assert_eq!(s1, 10, "first checkpoint sets last_seq=10");
    // A lower value must NOT rewind.
    let s2: i64 = a.query_one("SELECT checkpoint_sync_cursor($1,$2)", &[&peer, &5_i64])
        .await.unwrap().get(0);
    assert_eq!(s2, 10, "lower observed_seq is a no-op (advance-only)");
    // A higher value advances.
    let s3: i64 = a.query_one("SELECT checkpoint_sync_cursor($1,$2)", &[&peer, &20_i64])
        .await.unwrap().get(0);
    assert_eq!(s3, 20, "higher observed_seq advances");
}

/// The floor invariant (PR #39) extended to sync_cursor: the unprivileged runtime role
/// CANNOT raw-write the table — it may only go through the validated door. Mirrors
/// `tests/floor_enforced.rs`.
fn conn_as_role(base: &str, role: &str) -> String {
    let kept: Vec<&str> = base.split_whitespace().filter(|kv| !kv.starts_with("user=")).collect();
    format!("{} user={role}", kept.join(" "))
}

#[tokio::test]
async fn runtime_role_cannot_raw_write_sync_cursor_but_door_works() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let owner = db::connect_and_load_schema(&base).await.unwrap();
    owner.batch_execute("TRUNCATE node_event, local_node, sync_cursor").await.ok();

    let role = "cairn_runtime_cursor_test";
    db::provision_runtime_role(&owner, role).await.unwrap();
    let runtime = db::connect(&conn_as_role(&base, role)).await.unwrap();

    // Raw INSERT is denied (42501).
    let raw = runtime
        .execute("INSERT INTO sync_cursor (peer_addr, last_seq) VALUES ('x', 1)", &[])
        .await;
    let err = raw.expect_err("raw INSERT into sync_cursor must be denied for the runtime role");
    assert_eq!(
        err.code(),
        Some(&tokio_postgres::error::SqlState::INSUFFICIENT_PRIVILEGE),
        "raw INSERT must fail with 42501, got: {err:?}"
    );
    // The validated door still works for the unprivileged role.
    let v: i64 = runtime
        .query_one("SELECT checkpoint_sync_cursor($1,$2)", &[&"x", &7_i64])
        .await.expect("door must work for the cairn_node-granted role").get(0);
    assert_eq!(v, 7, "door advanced the cursor for the runtime role");

    drop(runtime);
    owner.batch_execute(&format!("DROP ROLE IF EXISTS {role}")).await.ok();
}
```

> Add `use tokio_postgres;` is not needed (the SqlState path is fully qualified); the `conn_as_role` helper is duplicated from `floor_enforced.rs` deliberately (test files don't share helpers in this suite).

- [ ] **Step 2: Run it to verify both tests fail**

Run: `CAIRN_TEST_PG="$CAIRN_TEST_PG" cargo test -p cairn-node --test sync_watermark checkpoint_sync_cursor -- --nocapture`
Expected: FAIL — `function checkpoint_sync_cursor(...) does not exist` (and the raw-write test fails to find the table/door).

- [ ] **Step 3: Add the table, door, and grants**

In `db/007_node_federation.sql`, after the Task 1 `seq` block and before the `-- The grant floor.` comment (~line 152), add:

```sql
-- Issue #38: the per-peer pull checkpoint. `last_seq` is the highest serving-node
-- `seq` this node has pulled from `peer_addr`. MUTABLE node-local operational state
-- (not a signed event), so it lives OUTSIDE the append-only trigger. Keyed by peer
-- ADDRESS: the address is known before the connection (no protocol round-trip), and
-- a wrong/stale key can only cause a re-pull or a transient skip — both healed by the
-- full-sweep floor — never an incorrect admission.
CREATE TABLE IF NOT EXISTS sync_cursor (
    peer_addr  TEXT        PRIMARY KEY,
    last_seq   BIGINT      NOT NULL DEFAULT 0,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);

-- The ONE door that writes sync_cursor. The runtime role gets EXECUTE on this, never
-- raw INSERT/UPDATE — preserving the floor invariant (PR #39): the cairn_node role does
-- zero raw DML, only validated doors. ADVANCE-ONLY (GREATEST): a buggy or hostile caller
-- cannot rewind the cursor to thrash re-pulls. Returns the resulting last_seq so the
-- caller can log/assert. A forward jump can only DELAY a legitimate event (healed by the
-- sweep), never admit an unauthorized one (the admission gate is untouched).
CREATE OR REPLACE FUNCTION checkpoint_sync_cursor(p_peer_addr TEXT, p_observed_seq BIGINT)
RETURNS BIGINT
LANGUAGE plpgsql SECURITY DEFINER SET search_path = public
AS $$
DECLARE v_last BIGINT;
BEGIN
    INSERT INTO sync_cursor (peer_addr, last_seq, updated_at)
    VALUES (p_peer_addr, GREATEST(0, p_observed_seq), clock_timestamp())
    ON CONFLICT (peer_addr) DO UPDATE
        SET last_seq = GREATEST(sync_cursor.last_seq, EXCLUDED.last_seq),
            updated_at = clock_timestamp()
    RETURNING last_seq INTO v_last;
    RETURN v_last;
END;
$$;
```

Then, in the floor block, alongside the existing `GRANT SELECT ON node_event, ... TO cairn_node;` (~line 164), add the cursor grants:

```sql
-- sync_cursor: SELECT (for status/debug) but NO raw DML — writes go through the door.
GRANT SELECT ON sync_cursor TO cairn_node;
REVOKE INSERT, UPDATE, DELETE ON sync_cursor FROM PUBLIC, cairn_node;
REVOKE EXECUTE ON FUNCTION checkpoint_sync_cursor(text, bigint) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION checkpoint_sync_cursor(text, bigint) TO cairn_node;
```

- [ ] **Step 4: Run it to verify it passes**

Run: `CAIRN_TEST_PG="$CAIRN_TEST_PG" cargo test -p cairn-node --test sync_watermark checkpoint_sync_cursor_is_advance_only -- --nocapture`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add db/007_node_federation.sql crates/cairn-node/tests/sync_watermark.rs
git commit -m "feat(cairn-node): sync_cursor + advance-only checkpoint door (issue #38)"
```

---

## Task 3: Real genesis/peer HLC (`hlc_state` + `node_hlc_tick` + merge-forward + Rust tick)

**Files:**
- Modify: `db/007_node_federation.sql` (add `hlc_state` + `node_hlc_tick` near the top after `local_node`; add merge-forward in `apply_remote_node_event`; add grants)
- Modify: `crates/cairn-node/src/identity.rs` (`provision`, `author_peer`, `author_unpeer`)
- Test: `crates/cairn-node/tests/genesis_hlc.rs` (create)

**Interfaces:**
- Produces: SQL `node_hlc_tick() RETURNS TABLE(wall BIGINT, counter INTEGER)` (EXECUTE → `cairn_node`); `hlc_state` singleton. Rust authoring functions keep their signatures but now mint a real HLC internally.

- [ ] **Step 1: Write the failing test**

Create `crates/cairn-node/tests/genesis_hlc.rs`:

```rust
//! Issue #38 (Gap 4) — genesis/peer HLC must be REAL, not the 0/0 placeholder.
//! DB-gated: needs CAIRN_TEST_PG.

use cairn_node::{db, identity, keystore};
use cairn_event::{verify_self_described};

fn cs() -> Option<String> { std::env::var("CAIRN_TEST_PG").ok() }

/// Genesis enroll carries a real, non-zero HLC wall, and a later peer.added carries a
/// strictly greater HLC than the genesis (the local clock advances per authored event).
#[tokio::test]
async fn genesis_hlc_is_nonzero_and_advances() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let a = db::connect_and_load_schema(&base).await.unwrap();
    a.batch_execute("TRUNCATE node_event, local_node, sync_cursor, hlc_state").await.ok();
    a.batch_execute("INSERT INTO hlc_state (id) VALUES (TRUE) ON CONFLICT DO NOTHING").await.ok();

    let tmp = tempfile::tempdir().unwrap();
    let (sk, kid) = keystore::generate_and_seal(&tmp.path().join("a.key"), None).unwrap();
    identity::provision(&a, &sk, &kid, "A", "127.0.0.1:7902").await.unwrap();

    // Genesis HLC stored in node_event must be non-zero.
    let genesis: (i64, i32) = {
        let r = a.query_one(
            "SELECT hlc_wall, hlc_counter FROM node_event WHERE op='enroll'", &[]
        ).await.unwrap();
        (r.get("hlc_wall"), r.get("hlc_counter"))
    };
    assert!(genesis.0 > 0, "genesis hlc_wall must be real, got {}", genesis.0);

    // Author a peer.added against a hand-built bundle; its HLC must exceed genesis.
    let id = identity::load_local(&a).await.unwrap();
    let bundle = cairn_event::PairingBundle {
        node_id_hex: id.node_id_hex.clone(), pubkey_hex: id.pubkey_hex.clone(),
        address: "127.0.0.1:7903".into(),
        fingerprint: cairn_event::short_fingerprint(&id.pubkey_hex).unwrap(),
        nonce: "n".into(),
        hlc: cairn_event::Hlc { wall: 0, counter: 0, node_origin: id.node_id_hex.clone() },
    };
    identity::author_peer(&a, &sk, &kid, &id.node_id_hex, &bundle, Some("peer")).await.unwrap();

    let peer_hlc: (i64, i32) = {
        let r = a.query_one(
            "SELECT hlc_wall, hlc_counter FROM node_event WHERE op='peer'", &[]
        ).await.unwrap();
        (r.get("hlc_wall"), r.get("hlc_counter"))
    };
    assert!(peer_hlc > genesis, "peer HLC {peer_hlc:?} must exceed genesis {genesis:?}");

    // And the signed body's HLC matches what was stored (the tick happened before signing).
    let signed: Vec<u8> = a.query_one(
        "SELECT signed_bytes FROM node_event WHERE op='enroll'", &[]
    ).await.unwrap().get(0);
    let body = verify_self_described(&signed).unwrap();
    assert_eq!(body.hlc.wall, genesis.0, "signed genesis HLC == stored");
}
```

> Note: confirm `cairn_event::verify_self_described` is the public verify-and-decode helper (it is used in `cairn-sync`). If the node crate re-exports a different name, use that; the assertion on the signed body is the principle-12 anchor (the HLC lives inside the signed bytes).

- [ ] **Step 2: Run it to verify it fails**

Run: `CAIRN_TEST_PG="$CAIRN_TEST_PG" cargo test -p cairn-node --test genesis_hlc -- --nocapture`
Expected: FAIL — `genesis hlc_wall must be real, got 0` (authoring still hardcodes `0,0`).

- [ ] **Step 3a: Add `hlc_state` + `node_hlc_tick` to the schema**

In `db/007_node_federation.sql`, after the `cairn_node` role `DO $$ ... $$;` block (~line 75) and before `CREATE OR REPLACE FUNCTION submit_node_event`, add:

```sql
-- Issue #38 (Gap 4): the node's local Hybrid Logical Clock. Mirrors cairn-sync's
-- hlc_state: a singleton row advanced on every authored event and merged forward on
-- every applied remote event, so the clock never falls behind anything in the log.
-- Replaces the 0/0 genesis placeholder, making trust_peer's HLC ordering real.
CREATE TABLE IF NOT EXISTS hlc_state (
    id          BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK (id),
    hlc_wall    BIGINT  NOT NULL DEFAULT 0,
    hlc_counter INTEGER NOT NULL DEFAULT 0
);
INSERT INTO hlc_state (id) VALUES (TRUE) ON CONFLICT DO NOTHING;

-- Advance the local clock and return the new stamp. wall = max(prev_wall, now_ms);
-- counter resets to 0 when wall advances on wall-clock time, else increments (the
-- standard HLC tick). SECURITY DEFINER so the unprivileged runtime can tick via the
-- door without direct write to hlc_state.
CREATE OR REPLACE FUNCTION node_hlc_tick()
RETURNS TABLE(wall BIGINT, counter INTEGER)
LANGUAGE plpgsql SECURITY DEFINER SET search_path = public
AS $$
DECLARE
    v_now  BIGINT := (extract(epoch FROM clock_timestamp()) * 1000)::bigint;
    v_wall BIGINT; v_counter INTEGER;
BEGIN
    SELECT hlc_wall, hlc_counter INTO v_wall, v_counter FROM hlc_state WHERE id FOR UPDATE;
    IF v_now > v_wall THEN
        v_wall := v_now; v_counter := 0;
    ELSE
        v_counter := v_counter + 1;
    END IF;
    UPDATE hlc_state SET hlc_wall = v_wall, hlc_counter = v_counter WHERE id;
    wall := v_wall; counter := v_counter;
    RETURN NEXT;
END;
$$;
```

- [ ] **Step 3b: Merge the clock forward on apply, and grant EXECUTE**

In `apply_remote_node_event`, add a merge-forward just before **each** `RETURN v_eid;` (both the enroll branch ~line 219 and the peer/revoke branch ~line 246). Use the same expression in both:

```sql
    -- Clock never falls behind an event we accepted (HLC invariant A3, mirrors cairn-sync).
    UPDATE hlc_state SET
        hlc_wall    = GREATEST(hlc_wall, (b -> 'hlc' ->> 'wall')::bigint),
        hlc_counter = CASE
            WHEN (b -> 'hlc' ->> 'wall')::bigint > hlc_wall THEN (b -> 'hlc' ->> 'counter')::int
            WHEN (b -> 'hlc' ->> 'wall')::bigint = hlc_wall THEN GREATEST(hlc_counter, (b -> 'hlc' ->> 'counter')::int)
            ELSE hlc_counter END
        WHERE id;
```

Then in the floor block, after the other `GRANT EXECUTE`s, add:

```sql
REVOKE EXECUTE ON FUNCTION node_hlc_tick() FROM PUBLIC;
GRANT EXECUTE ON FUNCTION node_hlc_tick() TO cairn_node;
```

- [ ] **Step 3c: Tick the real HLC in Rust before signing**

In `crates/cairn-node/src/identity.rs`, add a helper above `provision`:

```rust
/// Advance this node's local HLC in the DB and return the new (wall, counter). The
/// clock lives in Postgres (fat-Postgres, ADR-0001) so a single authority orders all
/// authored events; the Rust side just reads the next stamp before it signs (the HLC
/// is inside the signed body, so it MUST be obtained before `sign`).
async fn next_hlc(db: &Client) -> anyhow::Result<(i64, i32)> {
    let row = db.query_one("SELECT wall, counter FROM node_hlc_tick()", &[]).await?;
    Ok((row.get("wall"), row.get("counter")))
}
```

In `provision`, replace the body construction:

```rust
    let (wall, counter) = next_hlc(db).await?;
    let body = node_event_body("node.enrolled", key_id, display_name, wall, counter,
        serde_json::json!({"display_name": display_name, "address": address}));
```

In `author_peer`, replace the `node_event_body(... 0, 0 ...)` call:

```rust
    let (wall, counter) = next_hlc(db).await?;
    let body = node_event_body("peer.added", key_id, node_origin, wall, counter, serde_json::json!({
        "peer_node_id_hex": peer.node_id_hex,
        "peer_pubkey":      peer.pubkey_hex,
        "fingerprint":      peer.fingerprint,
        "role":             role,
    }));
```

In `author_unpeer`, likewise:

```rust
    let (wall, counter) = next_hlc(db).await?;
    let body = node_event_body("peer.revoked", key_id, node_origin, wall, counter, serde_json::json!({
        "peer_node_id_hex": peer_node_id_hex,
    }));
```

- [ ] **Step 4: Run it to verify it passes**

Run: `CAIRN_TEST_PG="$CAIRN_TEST_PG" cargo test -p cairn-node --test genesis_hlc -- --nocapture`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add db/007_node_federation.sql crates/cairn-node/src/identity.rs crates/cairn-node/tests/genesis_hlc.rs
git commit -m "feat(cairn-node): real local HLC clock, replace 0/0 genesis placeholder (issue #38)"
```

---

## Task 4: Wire — `NodeEventsAfterSeq` request + seq-prefixed serve stream

**Files:**
- Modify: `crates/cairn-node/src/sync.rs` (`Request` enum ~line 41-49; `serve_conn` match ~line 218-222; `stream_node_events` ~line 231-259)
- Test: `crates/cairn-node/tests/sync_watermark.rs`

**Interfaces:**
- Consumes: `node_event.seq` (Task 1).
- Produces: `Request::NodeEventsAfterSeq { after_seq: i64 }`; `stream_node_events(tls, db, after_seq: i64)` writing frames of `[8-byte BE seq][signed_bytes]`. The `Uuid` import in `sync.rs` becomes unused — remove it.

- [ ] **Step 1: Write the failing test**

Append to `crates/cairn-node/tests/sync_watermark.rs` (this is the principle-12 guard — the streamed signed core is byte-identical to the stored event regardless of the seq prefix):

```rust
use cairn_node::sync;
use std::net::SocketAddr;

/// A pull frames each event as [8-byte seq][signed_bytes]; the signed_bytes the puller
/// receives are byte-identical to what the server stored (seq is transport metadata,
/// never part of the signed core — principle 12).
#[tokio::test]
async fn wire_seq_prefix_does_not_touch_signed_core() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let a = db::connect_and_load_schema(&base).await.unwrap();
    a.batch_execute("TRUNCATE node_event, local_node, sync_cursor, hlc_state").await.ok();
    a.batch_execute("INSERT INTO hlc_state (id) VALUES (TRUE) ON CONFLICT DO NOTHING").await.ok();

    let tmp = tempfile::tempdir().unwrap();
    let (sk, kid) = keystore::generate_and_seal(&tmp.path().join("a.key"), None).unwrap();
    identity::provision(&a, &sk, &kid, "A", "127.0.0.1:7904").await.unwrap();

    // The bytes stored for the genesis enroll.
    let stored: Vec<u8> = a.query_one(
        "SELECT signed_bytes FROM node_event WHERE op='enroll'", &[]
    ).await.unwrap().get(0);

    // Serve A to itself (trust set need not include anyone for a same-process pull that
    // only reads the stream — but mTLS is mutual, so peer A with itself for the test).
    let id = identity::load_local(&a).await.unwrap();
    let self_bundle = cairn_event::PairingBundle {
        node_id_hex: id.node_id_hex.clone(), pubkey_hex: id.pubkey_hex.clone(),
        address: "127.0.0.1:7905".into(),
        fingerprint: cairn_event::short_fingerprint(&id.pubkey_hex).unwrap(),
        nonce: "n".into(),
        hlc: cairn_event::Hlc { wall: 0, counter: 0, node_origin: id.node_id_hex.clone() },
    };
    identity::author_peer(&a, &sk, &kid, &id.node_id_hex, &self_bundle, Some("peer")).await.unwrap();
    let trust = sync::trust_store_from_db(&a).await.unwrap();

    let listen: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let (addr, serve_cfg) = sync::bind_serve(listen, &base, &sk, trust.clone()).await.unwrap();
    let serve = tokio::spawn(sync::serve(serve_cfg));

    let cfg = sync::client_config(&base, &sk, trust).await.unwrap();
    let stats = sync::pull_once(addr, cfg, true).await.unwrap(); // full_sweep = true
    serve.abort();
    assert!(stats.received >= 1, "received at least the genesis frame");

    // The genesis row is byte-for-byte the same after a round-trip (re-read; idempotent apply).
    let after: Vec<u8> = a.query_one(
        "SELECT signed_bytes FROM node_event WHERE op='enroll'", &[]
    ).await.unwrap().get(0);
    assert_eq!(stored, after, "signed core unchanged by the seq-prefixed wire");
}
```

> This test also depends on Task 5 (`pull_once` gaining the `full_sweep` arg). Implement Task 4 and Task 5 together if executing inline; under subagent-driven execution, expect this specific test to compile-fail until Task 5 lands and run it at the end of Task 5.

- [ ] **Step 2: Replace the request variant and serve match**

In `sync.rs`, replace the `Request` enum (~line 41-49):

```rust
/// A request on the clinical-federation plane. JSON, one per connection.
#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "op")]
pub enum Request {
    /// Every `node_event` whose serving-node `seq` is strictly greater than
    /// `after_seq`, in `seq` order. `after_seq = 0` returns the full set (the
    /// full-sweep path). `seq` is the server's LOCAL insertion order — the only
    /// ordering where newly-learned events always sort above a puller's cursor, so
    /// incremental can never silently skip (issue #38). This enum is versioned;
    /// future changes are additive (principle 12).
    NodeEventsAfterSeq { after_seq: i64 },
}
```

In `serve_conn` (~line 218-222), replace the match arm:

```rust
    match req {
        Request::NodeEventsAfterSeq { after_seq } => {
            stream_node_events(&mut tls, &db, after_seq).await?;
        }
    }
```

Remove the now-unused `use uuid::Uuid;` import (~line 33).

- [ ] **Step 3: Rewrite `stream_node_events` to seq-filter and seq-prefix frames**

Replace `stream_node_events` (~line 228-259):

```rust
/// Stream every `node_event` with `seq > after_seq`, ordered by `seq` (the serving
/// node's local insertion order). Each frame is `[8-byte big-endian seq][signed_bytes]`
/// so the puller can checkpoint its per-peer cursor. The seq prefix is transport
/// metadata only; the signed_bytes are the untouched signed core (principle 12).
/// `after_seq = 0` selects everything (the full-sweep path).
async fn stream_node_events<S: AsyncWriteExt + Unpin>(
    tls: &mut S,
    db: &Client,
    after_seq: i64,
) -> anyhow::Result<()> {
    let rows = db
        .query(
            "SELECT seq, signed_bytes FROM node_event WHERE seq > $1 ORDER BY seq",
            &[&after_seq],
        )
        .await
        .context("selecting node_event bytes to stream")?;
    for row in &rows {
        let seq: i64 = row.get(0);
        let bytes: Vec<u8> = row.get(1);
        // Frame payload = 8-byte BE seq ++ signed_bytes.
        let mut framed = Vec::with_capacity(8 + bytes.len());
        framed.extend_from_slice(&seq.to_be_bytes());
        framed.extend_from_slice(&bytes);
        write_frame(tls, &framed).await.context("writing a node_event frame")?;
    }
    Ok(())
}
```

- [ ] **Step 4: Verify it compiles (full run deferred to Task 5)**

Run: `cargo build -p cairn-node`
Expected: builds (the puller side still sends the old request — fixed in Task 5; if executing inline, proceed straight to Task 5 before running the gated test).

- [ ] **Step 5: Commit**

```bash
git add crates/cairn-node/src/sync.rs crates/cairn-node/tests/sync_watermark.rs
git commit -m "feat(cairn-node): seq-filtered, seq-prefixed serve stream + NodeEventsAfterSeq (issue #38)"
```

---

## Task 5: Puller — cursor read, seq-prefix parse, checkpoint at EOF

**Files:**
- Modify: `crates/cairn-node/src/sync.rs` (`pull_once` ~line 291-294; `pull_into` ~line 298-329; `run`'s `pull_into` call ~line 396)
- Modify: `crates/cairn-node/src/main.rs` (the pull/serve CLI call site that calls `pull_once`)
- Test: `crates/cairn-node/tests/sync_watermark.rs` (incremental-only-new + the acceptance test)

**Interfaces:**
- Consumes: `Request::NodeEventsAfterSeq` (Task 4); `checkpoint_sync_cursor` (Task 2).
- Produces: `pull_once(peer, cfg, full_sweep: bool)`; `pull_into(peer, tls, db, full_sweep: bool)` — reads `sync_cursor` by `peer.to_string()`, sends `after_seq`, parses the 8-byte seq prefix per frame, applies the remaining bytes, checkpoints `max_seq` at clean EOF.

- [ ] **Step 1: Write the failing tests**

Append to `crates/cairn-node/tests/sync_watermark.rs`:

```rust
/// THE ACCEPTANCE TEST (issue #38): even if the cursor is advanced PAST an un-applied
/// event (the out-of-order-commit / rejection skip scenario), a full-sweep reconciles it.
/// We simulate the skip directly: jam B's cursor for A past A's genesis seq, prove an
/// incremental pull admits nothing, then prove a full-sweep delivers the genesis.
#[tokio::test]
async fn out_of_order_skip_is_reconciled_by_full_sweep() {
    let (Some(base_a), Some(base_b)) =
        (cs(), std::env::var("CAIRN_TEST_PG2").ok())
    else { eprintln!("skipped: set CAIRN_TEST_PG and CAIRN_TEST_PG2"); return };
    let _guard = db::test_serial_guard(&base_a).await.unwrap();

    let a = db::connect_and_load_schema(&base_a).await.unwrap();
    a.batch_execute("TRUNCATE node_event, local_node, sync_cursor, hlc_state").await.ok();
    a.batch_execute("INSERT INTO hlc_state (id) VALUES (TRUE) ON CONFLICT DO NOTHING").await.ok();
    let b = db::connect_and_load_schema(&base_b).await.unwrap();
    b.batch_execute("TRUNCATE node_event, local_node, sync_cursor, hlc_state").await.ok();
    b.batch_execute("INSERT INTO hlc_state (id) VALUES (TRUE) ON CONFLICT DO NOTHING").await.ok();

    let tmp = tempfile::tempdir().unwrap();
    let (sk_a, kid_a) = keystore::generate_and_seal(&tmp.path().join("a.key"), None).unwrap();
    let (sk_b, kid_b) = keystore::generate_and_seal(&tmp.path().join("b.key"), None).unwrap();
    identity::provision(&a, &sk_a, &kid_a, "A", "127.0.0.1:7906").await.unwrap();
    identity::provision(&b, &sk_b, &kid_b, "B", "127.0.0.1:7907").await.unwrap();
    let id_a = identity::load_local(&a).await.unwrap();
    let id_b = identity::load_local(&b).await.unwrap();

    // mutual peering
    let mk = |nid: &str, pk: &str, addr: &str| cairn_event::PairingBundle {
        node_id_hex: nid.into(), pubkey_hex: pk.into(), address: addr.into(),
        fingerprint: cairn_event::short_fingerprint(pk).unwrap(),
        nonce: "n".into(), hlc: cairn_event::Hlc { wall: 0, counter: 0, node_origin: nid.into() },
    };
    identity::author_peer(&a, &sk_a, &kid_a, &id_a.node_id_hex,
        &mk(&id_b.node_id_hex, &id_b.pubkey_hex, &id_b.address), Some("peer")).await.unwrap();
    identity::author_peer(&b, &sk_b, &kid_b, &id_b.node_id_hex,
        &mk(&id_a.node_id_hex, &id_a.pubkey_hex, &id_a.address), Some("peer")).await.unwrap();

    let trust_a = sync::trust_store_from_db(&a).await.unwrap();
    let trust_b = sync::trust_store_from_db(&b).await.unwrap();
    let listen: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let (addr, serve_cfg) = sync::bind_serve(listen, &base_a, &sk_a, trust_a).await.unwrap();
    let serve = tokio::spawn(sync::serve(serve_cfg));

    // Jam B's cursor for A past A's max seq, simulating "advanced past an un-applied event".
    let max_seq_a: i64 = a.query_one("SELECT max(seq) FROM node_event", &[]).await.unwrap().get(0);
    b.execute("SELECT checkpoint_sync_cursor($1,$2)", &[&addr.to_string(), &(max_seq_a + 1000)])
        .await.unwrap();

    // Incremental pull: admits nothing new (cursor is ahead).
    let cfg = sync::client_config(&base_b, &sk_b, trust_b.clone()).await.unwrap();
    let inc = sync::pull_once(addr, cfg, false).await.unwrap();
    let a_on_b: i64 = b.query_one(
        "SELECT count(*) FROM node_event WHERE op='enroll'", &[]).await.unwrap().get(0);
    assert_eq!(a_on_b, 1, "incremental skipped A's genesis (only B's own enroll present)");
    assert_eq!(inc.admitted, 0, "incremental admitted nothing past the jammed cursor");

    // Full sweep: reconciles the skipped event.
    let cfg2 = sync::client_config(&base_b, &sk_b, trust_b).await.unwrap();
    let _ = sync::pull_once(addr, cfg2, true).await.unwrap();
    serve.abort();
    let a_on_b2: i64 = b.query_one(
        "SELECT count(*) FROM node_event WHERE op='enroll'", &[]).await.unwrap().get(0);
    assert_eq!(a_on_b2, 2, "full-sweep reconciled A's genesis (B now holds both enrolls)");
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `CAIRN_TEST_PG="$CAIRN_TEST_PG" CAIRN_TEST_PG2="$CAIRN_TEST_PG2" cargo test -p cairn-node --test sync_watermark out_of_order_skip_is_reconciled_by_full_sweep -- --nocapture`
Expected: FAIL to compile (`pull_once` takes 2 args) — that is the failing state we fix next.

- [ ] **Step 3: Thread `full_sweep` and cursor logic through the puller**

In `sync.rs`, replace `pull_once` (~line 291-294):

```rust
/// Connect to `peer` over pinned mTLS, request node events after this node's cursor for
/// `peer` (or the full set when `full_sweep`), and apply each via the in-DB gate. Opens
/// its own short-lived DB connection; `run` uses [`pull_into`] with its cycle connection.
pub async fn pull_once(peer: SocketAddr, cfg: PullConfig, full_sweep: bool) -> anyhow::Result<PullStats> {
    let db = db::connect(&cfg.db_conn).await.context("pull: connecting to DB")?;
    pull_into(peer, cfg.tls, &db, full_sweep).await
}
```

Replace `pull_into` (~line 298-329):

```rust
/// The pull itself, applying admitted events into an already-open `db`. Reads the
/// per-peer cursor (keyed by the peer ADDRESS — known before connecting, so no protocol
/// round-trip), requests `seq > cursor` (or `> 0` on a full sweep), parses the 8-byte seq
/// prefix from each frame, applies the signed bytes via the unchanged admission gate, and
/// — only at a CLEAN EOF — checkpoints the highest seq received through the advance-only
/// door. A mid-stream failure returns early WITHOUT checkpointing, so the next cycle
/// re-pulls from the last committed cursor and no event is lost (idempotent apply).
pub async fn pull_into(
    peer: SocketAddr,
    tls: Arc<ClientConfig>,
    db: &Client,
    full_sweep: bool,
) -> anyhow::Result<PullStats> {
    let peer_key = peer.to_string();
    // Cursor: 0 on a full sweep (everything) or when we have never pulled this peer.
    let after_seq: i64 = if full_sweep {
        0
    } else {
        db.query_one(
            "SELECT coalesce((SELECT last_seq FROM sync_cursor WHERE peer_addr = $1), 0)",
            &[&peer_key],
        )
        .await
        .context("reading sync cursor")?
        .get(0)
    };

    let connector = TlsConnector::from(tls);
    let tcp = TcpStream::connect(peer).await.with_context(|| format!("connecting to {peer}"))?;
    let name = ServerName::try_from("cairn-node").context("building server name")?;
    let mut tls = connector.connect(name, tcp).await.context("mTLS handshake (server pin)")?;

    let req = Request::NodeEventsAfterSeq { after_seq };
    write_frame(&mut tls, &serde_json::to_vec(&req)?).await.context("sending request")?;

    let mut stats = PullStats::default();
    let mut max_seq = after_seq;
    while let Some(frame) = read_frame(&mut tls).await.context("reading a response frame")? {
        stats.received += 1;
        // Frame = [8-byte BE seq][signed_bytes]. A short frame is a protocol error.
        if frame.len() < 8 {
            anyhow::bail!("pull: response frame shorter than the 8-byte seq prefix");
        }
        let seq = i64::from_be_bytes(frame[..8].try_into().expect("8 bytes"));
        let signed = &frame[8..];
        match db.execute("SELECT apply_remote_node_event($1)", &[&signed]).await {
            Ok(_) => stats.admitted += 1,
            Err(e) => {
                stats.rejected += 1;
                eprintln!("pull: node_event rejected (non-fatal): {e}");
            }
        }
        // Advance over RECEIVED events (stream is seq-ordered); rejections are re-tried
        // on the next full sweep. Tracking the max — not the last — is robust to any
        // server-side reordering.
        if seq > max_seq { max_seq = seq; }
    }
    // Clean EOF reached: checkpoint through the advance-only door. Only now — a mid-stream
    // error returned above without advancing the cursor.
    if max_seq > after_seq || full_sweep {
        db.execute("SELECT checkpoint_sync_cursor($1,$2)", &[&peer_key, &max_seq])
            .await
            .context("checkpointing sync cursor")?;
    }
    Ok(stats)
}
```

Update `run`'s call to `pull_into` (~line 396). `run` chooses the sweep cadence in Task 6; for now pass `false` (incremental) to keep it compiling:

```rust
        match pull_into(peer, client_tls.clone(), &cycle_db, false).await {
```

- [ ] **Step 4: Update the `main.rs` call site**

In `crates/cairn-node/src/main.rs`, find the `sync::pull_once(` call (in the pull/serve CLI path) and add the `full_sweep` argument. A one-shot CLI pull should do a full sweep (no standing cursor intent), so pass `true`:

```rust
    let stats = sync::pull_once(addr, cfg, true).await?;
```

> If `main.rs` only calls `sync::run` (not `pull_once` directly), skip this step — grep first: `grep -n "pull_once\|pull_into" crates/cairn-node/src/main.rs`.

- [ ] **Step 5: Add the incremental-only-new test**

Append to `crates/cairn-node/tests/sync_watermark.rs`:

```rust
/// After an incremental pull, the cursor advances; a second incremental pull with no new
/// events on the server ships nothing new.
#[tokio::test]
async fn incremental_pull_ships_only_new_events() {
    let (Some(base_a), Some(base_b)) = (cs(), std::env::var("CAIRN_TEST_PG2").ok())
    else { eprintln!("skipped: set CAIRN_TEST_PG and CAIRN_TEST_PG2"); return };
    let _guard = db::test_serial_guard(&base_a).await.unwrap();

    let a = db::connect_and_load_schema(&base_a).await.unwrap();
    a.batch_execute("TRUNCATE node_event, local_node, sync_cursor, hlc_state").await.ok();
    a.batch_execute("INSERT INTO hlc_state (id) VALUES (TRUE) ON CONFLICT DO NOTHING").await.ok();
    let b = db::connect_and_load_schema(&base_b).await.unwrap();
    b.batch_execute("TRUNCATE node_event, local_node, sync_cursor, hlc_state").await.ok();
    b.batch_execute("INSERT INTO hlc_state (id) VALUES (TRUE) ON CONFLICT DO NOTHING").await.ok();

    let tmp = tempfile::tempdir().unwrap();
    let (sk_a, kid_a) = keystore::generate_and_seal(&tmp.path().join("a.key"), None).unwrap();
    let (sk_b, kid_b) = keystore::generate_and_seal(&tmp.path().join("b.key"), None).unwrap();
    identity::provision(&a, &sk_a, &kid_a, "A", "127.0.0.1:7908").await.unwrap();
    identity::provision(&b, &sk_b, &kid_b, "B", "127.0.0.1:7909").await.unwrap();
    let id_a = identity::load_local(&a).await.unwrap();
    let id_b = identity::load_local(&b).await.unwrap();
    let mk = |nid: &str, pk: &str, addr: &str| cairn_event::PairingBundle {
        node_id_hex: nid.into(), pubkey_hex: pk.into(), address: addr.into(),
        fingerprint: cairn_event::short_fingerprint(pk).unwrap(),
        nonce: "n".into(), hlc: cairn_event::Hlc { wall: 0, counter: 0, node_origin: nid.into() },
    };
    identity::author_peer(&a, &sk_a, &kid_a, &id_a.node_id_hex,
        &mk(&id_b.node_id_hex, &id_b.pubkey_hex, &id_b.address), Some("peer")).await.unwrap();
    identity::author_peer(&b, &sk_b, &kid_b, &id_b.node_id_hex,
        &mk(&id_a.node_id_hex, &id_a.pubkey_hex, &id_a.address), Some("peer")).await.unwrap();

    let trust_a = sync::trust_store_from_db(&a).await.unwrap();
    let trust_b = sync::trust_store_from_db(&b).await.unwrap();
    let listen: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let (addr, serve_cfg) = sync::bind_serve(listen, &base_a, &sk_a, trust_a).await.unwrap();
    let serve = tokio::spawn(sync::serve(serve_cfg));

    // First incremental pull (cursor starts at 0) admits A's genesis + A's peer event.
    let cfg = sync::client_config(&base_b, &sk_b, trust_b.clone()).await.unwrap();
    let first = sync::pull_once(addr, cfg, false).await.unwrap();
    assert!(first.admitted >= 1, "first pull admits A's events, got {}", first.admitted);

    // Second incremental pull: cursor is now past everything → nothing received.
    let cfg2 = sync::client_config(&base_b, &sk_b, trust_b).await.unwrap();
    let second = sync::pull_once(addr, cfg2, false).await.unwrap();
    serve.abort();
    assert_eq!(second.received, 0, "second incremental pull ships nothing new");
}
```

- [ ] **Step 6: Run the watermark suite to green**

Run: `CAIRN_TEST_PG="$CAIRN_TEST_PG" CAIRN_TEST_PG2="$CAIRN_TEST_PG2" cargo test -p cairn-node --test sync_watermark -- --nocapture`
Expected: PASS (all of `node_event_seq_is_monotonic_on_insert`, `checkpoint_sync_cursor_is_advance_only`, `wire_seq_prefix_does_not_touch_signed_core`, `out_of_order_skip_is_reconciled_by_full_sweep`, `incremental_pull_ships_only_new_events`).

- [ ] **Step 7: Commit**

```bash
git add crates/cairn-node/src/sync.rs crates/cairn-node/src/main.rs crates/cairn-node/tests/sync_watermark.rs
git commit -m "feat(cairn-node): incremental pull with per-peer seq cursor + full-sweep floor (issue #38)"
```

---

## Task 6: `run` full-sweep cadence + trust-change trigger

**Files:**
- Modify: `crates/cairn-node/src/sync.rs` (`run` loop ~line 338-408)

**Interfaces:**
- Consumes: `pull_into(.., full_sweep)` (Task 5); `refresh_trust_set` (existing).
- Produces: `run` performs a full sweep every `FULL_SWEEP_EVERY` cycles and whenever the active trust set changed since the previous cycle.

- [ ] **Step 1: Add the cadence constant and trust-change detection**

In `sync.rs`, add near the top of the file (after the imports):

```rust
/// Full-sweep cadence: the puller does an incremental `seq`-cursor pull each cycle and a
/// full sweep (cursor reset to 0) every `FULL_SWEEP_EVERY` cycles. The sweep is the
/// correctness floor (issue #38): it reconciles any event a residual hazard (commit-order
/// race, a rejected-then-later-trusted author, an address remap) caused incremental to
/// skip. `node_event` is low-volume, so a frequent sweep is cheap.
const FULL_SWEEP_EVERY: u64 = 10;
```

Modify the `run` loop. After the boot trust snapshot, track the previous trust set and a cycle counter; before the `pull_into`, compute `full_sweep`. Replace the loop body's relevant section (~line 366-407):

```rust
    let mut ticker = tokio::time::interval(std::time::Duration::from_secs(interval_secs.max(1)));
    let mut cycle: u64 = 0;
    // Snapshot of the trust set as of the previous cycle, to detect peering changes.
    let mut prev_trust: HashSet<String> =
        trust_set.read().map(|s| s.clone()).unwrap_or_default();
    loop {
        ticker.tick().await;
        cycle += 1;
        let cycle_db = match db::connect(&db_conn).await {
            Ok(c) => c,
            Err(e) => {
                eprintln!("run: DB unreachable, serving last-known set, skipping pull: {e}");
                if serve_handle.is_finished() {
                    anyhow::bail!("run: serve task exited unexpectedly");
                }
                continue;
            }
        };
        if let Err(e) = refresh_trust_set(&cycle_db, &trust_set).await {
            eprintln!("run: trust refresh failed, serving last-known set: {e}");
        }
        // Full sweep on cadence OR whenever the active peer set changed this cycle (so a
        // freshly-peered node's backlog is pulled at once, not after FULL_SWEEP_EVERY).
        let now_trust: HashSet<String> =
            trust_set.read().map(|s| s.clone()).unwrap_or_default();
        let trust_changed = now_trust != prev_trust;
        prev_trust = now_trust;
        let full_sweep = trust_changed || cycle % FULL_SWEEP_EVERY == 0;

        match pull_into(peer, client_tls.clone(), &cycle_db, full_sweep).await {
            Ok(s) => eprintln!(
                "run: pull {peer}: full_sweep={full_sweep} received={} admitted={} rejected={}",
                s.received, s.admitted, s.rejected
            ),
            Err(e) => eprintln!("run: PARTITION pulling {peer}: {e}"),
        }
        if serve_handle.is_finished() {
            anyhow::bail!("run: serve task exited unexpectedly");
        }
    }
```

- [ ] **Step 2: Verify it builds and clippy is clean**

Run: `cargo build -p cairn-node && cargo clippy -p cairn-node --all-targets`
Expected: builds, no clippy warnings. (`run` is exercised by the unattended path; its cadence logic is pure boolean arithmetic verified by reading — no separate unit test is warranted for the timer loop, consistent with the existing untested `run`.)

- [ ] **Step 3: Commit**

```bash
git add crates/cairn-node/src/sync.rs
git commit -m "feat(cairn-node): run full-sweep cadence + trust-change trigger (issue #38)"
```

---

## Task 7: Update the federation E2E tests for the cursor + extend convergence

**Files:**
- Modify: `crates/cairn-node/tests/federation.rs`

**Interfaces:**
- Consumes: the full puller/serve changes (Tasks 4-5).

- [ ] **Step 1: Update TRUNCATEs and `pull_once` call sites**

In `crates/cairn-node/tests/federation.rs`, every `TRUNCATE node_event, local_node` becomes `TRUNCATE node_event, local_node, sync_cursor, hlc_state` (so a re-run on a shared DB does not inherit a stale cursor that would suppress the pull), and seed the clock right after each: add `<conn>.batch_execute("INSERT INTO hlc_state (id) VALUES (TRUE) ON CONFLICT DO NOTHING").await.ok();`.

Update both `sync::pull_once(addr, cfg)` call sites to `sync::pull_once(addr, cfg, false)` for the first pull in each direction (incremental from a fresh cursor == full set), and any verification re-pull to `true` where the test asserts a complete set.

> Grep to find them all: `grep -n "TRUNCATE node_event\|pull_once" crates/cairn-node/tests/federation.rs`. Update each occurrence; do not miss the `two_nodes_converge_then_unpeer_and_a_stranger_is_rejected` test's pulls.

- [ ] **Step 2: Extend the convergence test with a second incremental pull**

In `two_nodes_converge_then_unpeer_and_a_stranger_is_rejected`, after the initial bidirectional convergence asserts, add a no-op-incremental assertion proving the cursor suppresses re-shipping (find the spot after both nodes hold all four events):

```rust
    // Incremental re-pull after convergence ships nothing new (cursor is current).
    let cfg_b2 = sync::client_config(&base_b, &sk_b,
        sync::trust_store_from_db(&b).await.unwrap()).await.unwrap();
    let again = sync::pull_once(addr_a, cfg_b2, false).await.unwrap();
    assert_eq!(again.received, 0, "post-convergence incremental pull ships nothing");
```

> Use the actual variable name for A's serve address in that test (e.g. `addr_a`); grep the test for the `bind_serve` return to confirm the binding name before pasting.

- [ ] **Step 3: Run the whole node suite + clippy**

Run:
```bash
CAIRN_TEST_PG="$CAIRN_TEST_PG" CAIRN_TEST_PG2="$CAIRN_TEST_PG2" CAIRN_TEST_PG3="$CAIRN_TEST_PG3" \
  cargo test -p cairn-node
cargo clippy --workspace --all-targets
```
Expected: all node tests pass (the prior 17 + the new watermark/HLC tests), clippy clean.

- [ ] **Step 4: Commit**

```bash
git add crates/cairn-node/tests/federation.rs
git commit -m "test(cairn-node): cursor-aware federation E2E + post-convergence incremental no-op (issue #38)"
```

---

## Task 8: HANDOVER + issue #38 close-out

**Files:**
- Modify: `docs/HANDOVER.md` (the node "honest gaps" bullets)

- [ ] **Step 1: Update HANDOVER**

In `docs/HANDOVER.md`, under "Honest gaps / follow-ons declared in the node", mark the incremental-watermark + genesis-HLC gap **closed 2026-06-23** (referencing this PR), mirroring how the status-before-init and floor-ENFORCED gaps were struck through after PR #39. Note the remaining open node gaps (DR/recovery escrow ADR-0026; key rotation / `supersede`) stay open.

- [ ] **Step 2: Commit**

```bash
git add docs/HANDOVER.md
git commit -m "docs(handover): incremental sync watermark + genesis HLC closed (issue #38)"
```

- [ ] **Step 3: Push and open the PR**

```bash
git push -u origin harden-node-incremental-sync
gh pr create --title "harden(cairn-node): incremental sync watermark + real genesis HLC (closes #38)" --body "<summary: seq-cursor incremental + full-sweep floor; checkpoint door keeps the runtime no-raw-DML floor; real local HLC replaces the 0/0 placeholder; test plan: cargo test -p cairn-node green, clippy clean>"
```

> The PR body should state the test counts and that no clinical surface was touched, matching the PR #39 style. Mention "Closes #38".

---

## Self-Review

**Spec coverage:**
- §Components 1 (`seq` + `sync_cursor` + `checkpoint_sync_cursor`) → Tasks 1, 2. ✓
- §Components 2 (genesis HLC: `hlc_state`, `node_hlc_tick`, merge-forward, Rust tick) → Task 3. ✓
- §Components 3 (wire: `NodeEventsAfterSeq` + seq-prefixed frames) → Task 4. ✓
- §Components 4 (puller cursor read + EOF checkpoint) → Task 5. ✓
- §Components 5 (serve seq-filter) → Task 4. ✓
- §`run` full-sweep cadence + trust-change trigger → Task 6. ✓
- §Testing tests 1-6 → acceptance (T5), incremental-only-new (T5), genesis HLC (T3), wire-seq-not-in-core (T4), cursor-door advance-only / no-raw-DML (T2 covers advance-only; **gap noted below**), E2E convergence (T7). 
- §"out of scope" (key rotation, DR escrow, clinical plane) → not implemented, correctly. ✓

**Gap found & fixed inline:** the spec's test 5 also asserts the runtime role **cannot raw-write** `sync_cursor` (the `42501` half), which the initial Task 2 test omitted. **Fixed:** Task 2 Step 1 now includes `runtime_role_cannot_raw_write_sync_cursor_but_door_works`, copying the `conn_as_role` + `provision_runtime_role` pattern from `tests/floor_enforced.rs`. Spec test 5 is now fully covered (advance-only + no-raw-DML).

**Placeholder scan:** no TBD/TODO; every code step shows real code. ✓

**Type consistency:** `pull_once(peer, cfg, full_sweep: bool)` and `pull_into(peer, tls, db, full_sweep: bool)` are used consistently across Tasks 4, 5, 6, 7. `checkpoint_sync_cursor(text, bigint) RETURNS bigint` and `node_hlc_tick() RETURNS TABLE(wall bigint, counter integer)` are referenced consistently. `Request::NodeEventsAfterSeq { after_seq: i64 }` consistent. ✓
