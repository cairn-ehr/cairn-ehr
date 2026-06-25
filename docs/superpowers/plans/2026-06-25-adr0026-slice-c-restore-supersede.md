# ADR-0026 slice C — restore + new-identity supersede Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the apply/restore half of ADR-0026 — a self-trusting in-DB restore door fenced to fresh nodes, the node-level `supersede` ceremony, and a `cairn-node restore` command that rehydrates a node's event history under a fresh, supersede-linked identity.

**Architecture:** A backup medium (slice B) holds the node's signed `node_event` set. Restore runs against a fresh, un-enrolled DB: it verifies the medium, mints a new sealed keypair (the old signing key is never backed up), applies the old events through a new `restore_node_event` door (which works only while `local_node` is empty — the structural fence that makes it a no-op on a live node), authors a new genesis, and authors a `node.superseded` event linking the dead node-id to the new one. The node then re-peers from empty.

**Tech Stack:** Rust (`cairn-node` crate, tokio-postgres, clap, anyhow, zeroize), PostgreSQL (PL/pgSQL SECURITY DEFINER doors), the `cairn_pgx` extension (`cairn_verify`/`cairn_body`), `cairn-event` (sign / verify_self_described / event_address).

## Global Constraints

- **License:** AGPL-3.0; every dependency AGPL-3.0-compatible. No new dependencies are introduced by this plan.
- **TDD:** failing test first, then code. Load-bearing — the restore door is safety-critical surface (a defect is a federation-admission bypass or silent data loss).
- **Inline docs for a junior dev:** every non-trivial function/door carries *why it exists + how it fits*, not just *what*.
- **Pure functions in reusable modules** over clever complexity; files under ~500 lines where feasible (the new orchestration lives in its own `restore.rs`, not bolted onto `main.rs`/`identity.rs`).
- **All tests pass before committing.**
- **Additive-only schema evolution (ADR-0012):** widening the `op` CHECK to a superset rejects nothing it previously accepted; the new door/view are additive.
- **DB-gated tests** read `CAIRN_TEST_PG`, take `db::test_serial_guard`, then `db::connect_and_load_schema` + `db::reset_node_federation_tables`. They need a local PG with `cairn_pgx` installed (`cargo pgrx install` against the local PG). They self-skip (print + return) when `CAIRN_TEST_PG` is unset.
- **Spec home:** [ADR-0026](../../spec/decisions/0026-node-durability-and-disaster-recovery.md) §7.10; design doc `docs/superpowers/specs/2026-06-25-adr0026-slice-c-restore-supersede-design.md`; issue [#50](https://github.com/cairn-ehr/cairn-ehr/issues/50).

## File Structure

- **Create** `db/009_node_supersede_and_restore.sql` — op-CHECK widen, `restore_node_event` door + grants, `node_lineage` view + grant. One `BEGIN; … COMMIT;` migration.
- **Modify** `db/007_node_federation.sql` — add the `supersede` branch to `submit_node_event` (its canonical home; single source of truth, no duplication).
- **Modify** `crates/cairn-node/src/db.rs` — append `009` to the `SCHEMA` array.
- **Modify** `crates/cairn-node/src/identity.rs` — add `author_supersede`.
- **Create** `crates/cairn-node/src/restore.rs` — pure helpers (`resolve_dead_node_id`, `old_genesis_meta`) + DB orchestration (`apply_medium`, `finalize_identity`, `RestoreOutcome`).
- **Modify** `crates/cairn-node/src/lib.rs` — declare `pub mod restore;`.
- **Modify** `crates/cairn-node/src/main.rs` — add the `Restore` subcommand.
- **Modify** `crates/cairn-node/src/identity.rs` (Status) + `main.rs` (Status print) — lineage line.
- **Create** `db/tests/009_node_supersede_test.sql` — op-CHECK + `node_lineage` (pure SQL, owner inserts).
- **Create** `crates/cairn-node/tests/restore.rs` — DB-gated: door fence/accept/tamper + full round-trip.
- **Modify** `crates/cairn-node/tests/` — supersede authoring test (folded into `restore.rs` or a small `supersede.rs`; this plan uses `restore.rs`).
- **Modify** `docs/HANDOVER.md`, `docs/ROADMAP.md` — reflect slice C done.

---

### Task 1: Schema migration — widen op CHECK + `node_lineage` view

**Files:**
- Create: `db/009_node_supersede_and_restore.sql`
- Modify: `crates/cairn-node/src/db.rs:3-11` (SCHEMA array) and `:94-100` (loader already iterates SCHEMA — no change there)
- Test: `db/tests/009_node_supersede_test.sql`

**Interfaces:**
- Produces: a loadable `db/009`; `node_event.op` accepts `'supersede'`; view `node_lineage(superseded_node_id bytea, new_node_id bytea, hlc_wall bigint, hlc_counter int, recorded_at timestamptz)`.
- Consumes: `db/007` table `node_event`, role `cairn_node`.

- [ ] **Step 1: Write the failing SQL test**

Create `db/tests/009_node_supersede_test.sql`:

```sql
\set ON_ERROR_STOP on
-- ADR-0026 slice C — schema tests for the node-level supersede op + lineage view.
-- PURE SQL (no pgrx / no cairn_verify): inserts as the table OWNER straight into
-- node_event (the door REVOKEs bind cairn_node/PUBLIC, not the owner), so this
-- exercises the op CHECK constraint and the node_lineage view in isolation.
-- Run with: psql -v ON_ERROR_STOP=1 -f db/001..009 then this file.

-- Helper: a content-address that satisfies the 001/007 CHECK for given bytes.
-- node_event.content_address must equal '\x1220' || sha256(signed_bytes).

DO $$
DECLARE
    v_sb  bytea := convert_to('supersede-fixture', 'UTF8');
    v_ca  bytea := '\x1220'::bytea || digest(convert_to('supersede-fixture','UTF8'), 'sha256');
    v_old bytea := '\x1220'::bytea || digest(convert_to('old-node','UTF8'), 'sha256');
    v_new bytea := '\x1220'::bytea || digest(convert_to('new-node','UTF8'), 'sha256');
BEGIN
    -- The widened CHECK must ACCEPT op='supersede'.
    INSERT INTO node_event (node_event_id, op, author_node_id, subject_node_id,
        signer_key_id, hlc_wall, hlc_counter, node_origin, signed_bytes, content_address)
    VALUES (gen_random_uuid(), 'supersede', v_new, v_old,
        'deadbeef', 1, 0, 'test', v_sb, v_ca);

    -- node_lineage must resolve the edge new <- old.
    PERFORM 1 FROM node_lineage
        WHERE superseded_node_id = v_old AND new_node_id = v_new;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'node_lineage did not resolve the supersede edge';
    END IF;
END $$;

-- The CHECK must still REJECT an unknown op (fail-closed).
DO $$
BEGIN
    BEGIN
        INSERT INTO node_event (node_event_id, op, author_node_id, subject_node_id,
            signer_key_id, hlc_wall, hlc_counter, node_origin, signed_bytes, content_address)
        VALUES (gen_random_uuid(), 'bogus', '\x00', '\x00', 'k', 0, 0, 't',
            convert_to('x','UTF8'),
            '\x1220'::bytea || digest(convert_to('x','UTF8'),'sha256'));
        RAISE EXCEPTION 'op CHECK accepted an unknown op (should fail closed)';
    EXCEPTION WHEN check_violation THEN
        NULL; -- expected
    END;
END $$;

\echo '009_node_supersede_test: PASS'
```

- [ ] **Step 2: Run it to verify it fails**

Run (replace `cairn_test` with your local test DB; it must already have `db/001`–`008` + `cairn_pgx` loaded — `connect_and_load_schema` does 001–007, and 008 is needed only by other tests):

```bash
psql "$CAIRN_TEST_PG" -v ON_ERROR_STOP=1 \
  -f db/001_envelope.sql -f db/002_projection.sql -f db/003_blobs.sql \
  -f db/004_actors.sql -f db/005_submit.sql -f db/006_recall.sql \
  -f db/007_node_federation.sql -f db/tests/009_node_supersede_test.sql
```

Expected: FAIL — `op` CHECK rejects `'supersede'` (constraint not yet widened) and/or `node_lineage` does not exist.

- [ ] **Step 3: Write the migration**

Create `db/009_node_supersede_and_restore.sql`:

```sql
-- Cairn — node-level supersede + self-trusting restore (ADR-0026 slice C).
--
-- WHY: slice B exports a node's signed node_event set to a cold-peer medium; this is
-- the APPLY half. Restoring a node's OWN history into a fresh DB cannot use the live
-- apply_remote_node_event gate (that is the PEER-admission path — it rejects events
-- whose author is not an already-trusted peer, which a fresh node has none of). So we
-- add a SELF-TRUSTING restore door, fenced so it is a permanent no-op on a live node,
-- plus the node-level `supersede` op (a restored node mints a NEW key — the signing key
-- is never backed up — and records supersede(dead -> new), already the actor-algebra
-- shape for agents, now applied to nodes). See ADR-0026 §7.10 points 1/2/4.

BEGIN;

-- (1) Widen the op CHECK additively (ADR-0012): a superset rejects nothing previously
-- accepted. The constraint is the auto-named column CHECK from db/007's CREATE TABLE.
ALTER TABLE node_event DROP CONSTRAINT IF EXISTS node_event_op_check;
ALTER TABLE node_event ADD CONSTRAINT node_event_op_check
    CHECK (op IN ('enroll','peer','revoke','supersede'));

-- (2) The supersede lineage view: who superseded whom. Read by `status`/audit. A
-- supersede event's author is the NEW (live) node; its subject is the dead node-id.
CREATE OR REPLACE VIEW node_lineage AS
SELECT ne.subject_node_id AS superseded_node_id,
       ne.author_node_id  AS new_node_id,
       ne.hlc_wall, ne.hlc_counter, ne.recorded_at
FROM node_event ne
WHERE ne.op = 'supersede';

GRANT SELECT ON node_lineage TO cairn_node;

COMMIT;
```

(The `restore_node_event` door is added to this file in Task 3.)

- [ ] **Step 4: Wire 009 into the schema loader**

Modify `crates/cairn-node/src/db.rs`. Change the array size and append the entry:

```rust
const SCHEMA: [(&str, &str); 8] = [
    ("001_envelope",      include_str!("../../../db/001_envelope.sql")),
    ("002_projection",    include_str!("../../../db/002_projection.sql")),
    ("003_blobs",         include_str!("../../../db/003_blobs.sql")),
    ("004_actors",        include_str!("../../../db/004_actors.sql")),
    ("005_submit",        include_str!("../../../db/005_submit.sql")),
    ("006_recall",        include_str!("../../../db/006_recall.sql")),
    ("007_node_federation", include_str!("../../../db/007_node_federation.sql")),
    ("009_node_supersede_and_restore", include_str!("../../../db/009_node_supersede_and_restore.sql")),
];
```

(Note: `008` is intentionally not loaded by the node crate — it is the clinical surrogate-projection plane, unused here. `009` only depends on `007`.)

- [ ] **Step 5: Run the SQL test to verify it passes**

Run the same command as Step 2 but add `-f db/009_node_supersede_and_restore.sql` before the test file:

```bash
psql "$CAIRN_TEST_PG" -v ON_ERROR_STOP=1 \
  -f db/001_envelope.sql -f db/002_projection.sql -f db/003_blobs.sql \
  -f db/004_actors.sql -f db/005_submit.sql -f db/006_recall.sql \
  -f db/007_node_federation.sql -f db/009_node_supersede_and_restore.sql \
  -f db/tests/009_node_supersede_test.sql
```

Expected: `009_node_supersede_test: PASS`. Also confirm the crate still builds: `cargo build -p cairn-node` (the `include_str!` path resolves).

- [ ] **Step 6: Commit**

```bash
git add db/009_node_supersede_and_restore.sql db/tests/009_node_supersede_test.sql crates/cairn-node/src/db.rs
git commit -m "harden(node): widen node_event op CHECK + node_lineage view (ADR-0026 slice C)"
```

---

### Task 2: `supersede` authoring path — `submit_node_event` + `identity::author_supersede`

**Files:**
- Modify: `db/007_node_federation.sql` (the `submit_node_event` function body, ~lines 160-230)
- Modify: `crates/cairn-node/src/identity.rs` (add `author_supersede` after `author_unpeer`, ~line 142)
- Test: `crates/cairn-node/tests/restore.rs` (new file — first test)

**Interfaces:**
- Produces: `submit_node_event` accepts a `node.superseded` event (op `supersede`); `identity::author_supersede(db: &Client, sk: &SigningKey, key_id: &str, node_origin: &str, old_node_id_hex: &str) -> anyhow::Result<String>` (returns the new event's content-address hex).
- Consumes: `node_event_body` (identity.rs), `next_hlc` (identity.rs), the `node_lineage` view (Task 1).

- [ ] **Step 1: Write the failing DB-gated test**

Create `crates/cairn-node/tests/restore.rs`:

```rust
//! ADR-0026 slice C — restore (apply) + new-identity supersede.
//! DB-gated: needs CAIRN_TEST_PG (local PG with cairn_pgx installed).

use cairn_node::{db, identity, keystore};

fn cs() -> Option<String> { std::env::var("CAIRN_TEST_PG").ok() }

/// A live node can author a node.superseded event naming a dead node-id; it lands as an
/// op='supersede' row and node_lineage resolves the edge (new <- old).
#[tokio::test]
async fn author_supersede_records_a_lineage_edge() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let a = db::connect_and_load_schema(&base).await.unwrap();
    db::reset_node_federation_tables(&a).await.ok();

    let tmp = tempfile::tempdir().unwrap();
    let (sk, kid) = keystore::generate_plaintext(&tmp.path().join("a.key")).unwrap();
    identity::provision(&a, &sk, &kid, "A", "127.0.0.1:7920").await.unwrap();
    let id = identity::load_local(&a).await.unwrap();

    // Author supersede naming a fabricated "old" node-id (any hex node-id works here;
    // the full restore flow supplies the real one in Task 5).
    let old_hex = "1220".to_string() + &"ab".repeat(32);
    identity::author_supersede(&a, &sk, &kid, &id.node_id_hex, &old_hex).await.unwrap();

    let row = a.query_one(
        "SELECT encode(superseded_node_id,'hex') AS old, encode(new_node_id,'hex') AS new
         FROM node_lineage", &[]).await.unwrap();
    let old: String = row.get("old");
    let new: String = row.get("new");
    assert_eq!(old, old_hex, "lineage subject == the dead node-id");
    assert_eq!(new, id.node_id_hex, "lineage author == the live (new) node-id");
}
```

- [ ] **Step 2: Run it to verify it fails**

```bash
CAIRN_TEST_PG="$CAIRN_TEST_PG" cargo test -p cairn-node --test restore author_supersede_records_a_lineage_edge -- --nocapture
```

Expected: FAIL — `identity::author_supersede` does not exist (compile error), and/or `submit_node_event` rejects the unknown event_type `node.superseded`.

- [ ] **Step 3: Extend `submit_node_event` in `db/007`**

In `db/007_node_federation.sql`, in `submit_node_event`, add `supersede` to the op map and insert branch.

First, extend the op map (find the `v_op := CASE v_type` block, ~line 180):

```sql
    v_op := CASE v_type
        WHEN 'node.enrolled' THEN 'enroll'
        WHEN 'peer.added'    THEN 'peer'
        WHEN 'peer.revoked'  THEN 'revoke'
        WHEN 'node.superseded' THEN 'supersede'   -- ADR-0026 slice C
        ELSE NULL END;
```

Then, after the `IF v_signer <> v_local_key THEN … END IF;` guard (~line 210, which applies to all locally-authored ops) and BEFORE the `peer_node_id_hex` guard, insert the supersede branch:

```sql
    -- supersede (ADR-0026 slice C): a restored node records that it succeeds a dead node.
    -- Authored by THIS node's current (new) key; subject = the superseded (dead) node-id.
    -- A distinct payload field (superseded_node_id_hex, not peer_node_id_hex) keeps the
    -- intent legible — the superseded node is NOT a peer.
    IF v_op = 'supersede' THEN
        IF v_payload ->> 'superseded_node_id_hex' IS NULL THEN
            RAISE EXCEPTION 'submit_node_event: node.superseded missing superseded_node_id_hex in payload';
        END IF;
        INSERT INTO node_event (node_event_id, op, author_node_id, subject_node_id,
            signer_key_id, hlc_wall, hlc_counter, node_origin, signed_bytes, content_address)
        VALUES (v_eid, 'supersede', v_local_node,
            decode(v_payload ->> 'superseded_node_id_hex','hex'),
            v_signer, (b -> 'hlc' ->> 'wall')::bigint, (b -> 'hlc' ->> 'counter')::int,
            b -> 'hlc' ->> 'node_origin', p_signed, v_ca)
        ON CONFLICT (node_event_id) DO NOTHING;
        RETURN v_eid;
    END IF;
```

- [ ] **Step 4: Add `author_supersede` to `identity.rs`**

After `author_unpeer` (~line 142), add:

```rust
/// Author a `node.superseded` event and submit it (ADR-0026 slice C).
///
/// A restored node mints a fresh keypair (the old signing key is never backed up) and
/// records that its new identity SUCCEEDS the dead node. This is the actor-algebra
/// `supersede` applied to nodes: the dead node's past events stay signature-verifiable
/// forever, but the new node cannot sign as the old one — a destroyed node is a new
/// physical trust boundary. Authored by THIS (new) node's key, like peer/revoke.
pub async fn author_supersede(
    db: &Client,
    sk: &SigningKey,
    key_id: &str,
    node_origin: &str,
    old_node_id_hex: &str,
) -> anyhow::Result<String> {
    let (wall, counter) = next_hlc(db).await?;
    let body = node_event_body("node.superseded", key_id, node_origin, wall, counter,
        serde_json::json!({ "superseded_node_id_hex": old_node_id_hex }));
    let signed = sign(&body, sk)?;
    let bytes = signed.signed_bytes.clone();
    db.execute("SELECT submit_node_event($1)", &[&bytes]).await?;
    Ok(hex::encode(event_address(&signed.signed_bytes)))
}
```

- [ ] **Step 5: Run the test to verify it passes**

```bash
CAIRN_TEST_PG="$CAIRN_TEST_PG" cargo test -p cairn-node --test restore author_supersede_records_a_lineage_edge -- --nocapture
```

Expected: PASS. Also run `cargo clippy -p cairn-node --all-targets` clean.

- [ ] **Step 6: Commit**

```bash
git add db/007_node_federation.sql crates/cairn-node/src/identity.rs crates/cairn-node/tests/restore.rs
git commit -m "harden(node): node-level supersede authoring path (ADR-0026 slice C)"
```

---

### Task 3: The self-trusting `restore_node_event` door

**Files:**
- Modify: `db/009_node_supersede_and_restore.sql` (add the door + grants inside the migration, before `COMMIT;`)
- Test: `crates/cairn-node/tests/restore.rs` (add three tests)

**Interfaces:**
- Produces: `restore_node_event(p_signed bytea) RETURNS uuid` — SECURITY DEFINER; verifies signature + content-address, NO peer-trust check, fenced fail-closed unless `local_node` is empty; never writes `local_node`. Granted EXECUTE to `cairn_node`.
- Consumes: `cairn_verify`/`cairn_body` (pgrx), `node_current` view, `hlc_state` (db/007).

- [ ] **Step 1: Write the failing DB-gated tests**

Append to `crates/cairn-node/tests/restore.rs`:

```rust
use cairn_event::{sign, EventBody, Hlc, SigningKey};

/// Mint a real signed node.enrolled event for an arbitrary key (no DB). Mirrors how
/// identity::provision builds genesis, so its content-address == the node-id.
fn synth_enroll(sk: &SigningKey, name: &str) -> Vec<u8> {
    let kid = hex::encode(sk.verifying_key().to_bytes());
    let body = EventBody {
        event_id: uuid::Uuid::now_v7().to_string(),
        patient_id: identity::NIL_PATIENT.into(),
        event_type: "node.enrolled".into(),
        schema_version: "node/1".into(),
        hlc: Hlc { wall: 1, counter: 0, node_origin: name.into() },
        t_effective: None,
        signer_key_id: kid,
        contributors: serde_json::json!([]),
        payload: serde_json::json!({ "display_name": name, "address": "127.0.0.1:7999" }),
        attachments: vec![],
    };
    sign(&body, sk).unwrap().signed_bytes
}

/// The restore door must be a no-op on a LIVE node: with a genesis present, it raises a
/// legible "already enrolled" error. This is the structural fence that prevents the
/// self-trusting door from ever bypassing peer-admission on a running node.
#[tokio::test]
async fn restore_door_rejects_on_a_live_node() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let a = db::connect_and_load_schema(&base).await.unwrap();
    db::reset_node_federation_tables(&a).await.ok();

    let tmp = tempfile::tempdir().unwrap();
    let (sk, kid) = keystore::generate_plaintext(&tmp.path().join("a.key")).unwrap();
    identity::provision(&a, &sk, &kid, "Live", "127.0.0.1:7921").await.unwrap();

    let other = cairn_event::generate_key().unwrap().0;
    let ev = synth_enroll(&other, "Intruder");
    let err = a.execute("SELECT restore_node_event($1)", &[&ev]).await.unwrap_err();
    assert!(err.to_string().contains("already enrolled"),
        "fence must reject on a live node, got: {err}");
}

/// Into a FRESH (un-enrolled) DB, the door applies a validly-signed enroll without any
/// peer-trust — exactly what a node rehydrating its own history needs.
#[tokio::test]
async fn restore_door_accepts_into_an_empty_db() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let a = db::connect_and_load_schema(&base).await.unwrap();
    db::reset_node_federation_tables(&a).await.ok();

    let other = cairn_event::generate_key().unwrap().0;
    let ev = synth_enroll(&other, "Restored");
    a.execute("SELECT restore_node_event($1)", &[&ev]).await
        .expect("door applies into an empty DB without peer-trust");
    let n: i64 = a.query_one("SELECT count(*) FROM node_event WHERE op='enroll'", &[])
        .await.unwrap().get(0);
    assert_eq!(n, 1, "the restored enroll is present");
    // The door must NOT set local_node (only a real new genesis does).
    let ln: i64 = a.query_one("SELECT count(*) FROM local_node", &[]).await.unwrap().get(0);
    assert_eq!(ln, 0, "restore_node_event must never write local_node");
}

/// A tampered/bit-rotted medium event fails the door's signature check — the same
/// invariant slice B proves catches a hostile peer.
#[tokio::test]
async fn restore_door_rejects_a_tampered_event() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let a = db::connect_and_load_schema(&base).await.unwrap();
    db::reset_node_federation_tables(&a).await.ok();

    let other = cairn_event::generate_key().unwrap().0;
    let mut ev = synth_enroll(&other, "Tampered");
    let mid = ev.len() / 2;
    ev[mid] ^= 0x01; // break the signature
    let err = a.execute("SELECT restore_node_event($1)", &[&ev]).await.unwrap_err();
    assert!(err.to_string().contains("verification failed"),
        "tampered event must fail the signature check, got: {err}");
}
```

- [ ] **Step 2: Run them to verify they fail**

```bash
CAIRN_TEST_PG="$CAIRN_TEST_PG" cargo test -p cairn-node --test restore restore_door -- --nocapture
```

Expected: FAIL — `restore_node_event` does not exist yet (function undefined).

- [ ] **Step 3: Add the door to `db/009`**

In `db/009_node_supersede_and_restore.sql`, before `COMMIT;`, add:

```sql
-- (3) The self-trusting restore door. Unlike apply_remote_node_event (the PEER-admission
-- gate), this applies a node's OWN history into a fresh DB WITHOUT a peer-trust check —
-- a fresh node has no trust set yet. The danger (a federation-admission bypass) is closed
-- structurally: the door fails closed unless local_node is empty, so on any LIVE node it
-- is a permanent no-op. Signature + content-address ARE enforced, so a tampered/bit-rotted
-- medium event is rejected exactly as a hostile peer would be (ADR-0026 point 2). The door
-- NEVER writes local_node — only a real new genesis (submit_node_event) does, and that is
-- what permanently fences this door closed at the end of a restore.
CREATE OR REPLACE FUNCTION restore_node_event(p_signed BYTEA)
RETURNS UUID
LANGUAGE plpgsql SECURITY DEFINER SET search_path = public
AS $$
DECLARE
    b JSONB; v_type TEXT; v_op TEXT; v_ca BYTEA; v_eid UUID; v_signer TEXT;
    v_payload JSONB; v_author_node BYTEA; v_subject BYTEA;
BEGIN
    -- FENCE: restore is only into a fresh, un-enrolled node.
    IF EXISTS (SELECT 1 FROM local_node WHERE id) THEN
        RAISE EXCEPTION 'restore_node_event: node already enrolled; restore applies only into a fresh node (live admission is apply_remote_node_event)';
    END IF;
    IF NOT cairn_verify(p_signed) THEN
        RAISE EXCEPTION 'restore_node_event: signature verification failed';
    END IF;
    b := cairn_body(p_signed);
    v_type := b ->> 'event_type'; v_eid := (b ->> 'event_id')::uuid;
    v_signer := b ->> 'signer_key_id'; v_payload := b -> 'payload';
    v_ca := '\x1220'::bytea || digest(p_signed, 'sha256');
    v_op := CASE v_type WHEN 'node.enrolled' THEN 'enroll' WHEN 'peer.added' THEN 'peer'
                        WHEN 'peer.revoked' THEN 'revoke' WHEN 'node.superseded' THEN 'supersede'
                        ELSE NULL END;
    IF v_op IS NULL THEN
        RAISE EXCEPTION 'restore_node_event: unknown node event_type % (fail closed)', v_type;
    END IF;

    IF v_op = 'enroll' THEN
        INSERT INTO node_event (node_event_id, op, author_node_id, subject_node_id,
            signer_key_id, hlc_wall, hlc_counter, node_origin, signed_bytes, content_address)
        VALUES (v_eid, 'enroll', v_ca, v_ca, v_signer,
            (b -> 'hlc' ->> 'wall')::bigint, (b -> 'hlc' ->> 'counter')::int,
            b -> 'hlc' ->> 'node_origin', p_signed, v_ca)
        ON CONFLICT (node_event_id) DO NOTHING;
    ELSE
        -- The node's own enroll is restored first (medium seq order), so its key resolves.
        SELECT node_id INTO v_author_node FROM node_current WHERE signer_key_id = v_signer;
        IF v_author_node IS NULL THEN
            RAISE EXCEPTION 'restore_node_event: author key % maps to no restored enroll (apply genesis first)', v_signer;
        END IF;
        v_subject := CASE v_op
            WHEN 'supersede' THEN decode(v_payload ->> 'superseded_node_id_hex','hex')
            ELSE decode(v_payload ->> 'peer_node_id_hex','hex') END;
        IF v_subject IS NULL THEN
            RAISE EXCEPTION 'restore_node_event: % missing subject node id in payload', v_type;
        END IF;
        INSERT INTO node_event (node_event_id, op, author_node_id, subject_node_id,
            signer_key_id, peer_pubkey, fingerprint, role, scope_hint, target_event_id,
            hlc_wall, hlc_counter, node_origin, signed_bytes, content_address)
        VALUES (v_eid, v_op, v_author_node, v_subject,
            v_signer, v_payload ->> 'peer_pubkey', v_payload ->> 'fingerprint',
            v_payload ->> 'role', v_payload ->> 'scope_hint',
            NULLIF(v_payload ->> 'target_event_id','')::uuid,
            (b -> 'hlc' ->> 'wall')::bigint, (b -> 'hlc' ->> 'counter')::int,
            b -> 'hlc' ->> 'node_origin', p_signed, v_ca)
        ON CONFLICT (node_event_id) DO NOTHING;
    END IF;
    -- Clock never falls behind a restored event (HLC invariant A3, mirrors the apply path).
    UPDATE hlc_state SET
        hlc_wall    = GREATEST(hlc_wall, (b -> 'hlc' ->> 'wall')::bigint),
        hlc_counter = CASE
            WHEN (b -> 'hlc' ->> 'wall')::bigint > hlc_wall THEN (b -> 'hlc' ->> 'counter')::int
            WHEN (b -> 'hlc' ->> 'wall')::bigint = hlc_wall THEN GREATEST(hlc_counter, (b -> 'hlc' ->> 'counter')::int)
            ELSE hlc_counter END
        WHERE id;
    RETURN v_eid;
END;
$$;

REVOKE EXECUTE ON FUNCTION restore_node_event(bytea) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION restore_node_event(bytea) TO cairn_node;
```

- [ ] **Step 4: Run the tests to verify they pass**

```bash
CAIRN_TEST_PG="$CAIRN_TEST_PG" cargo test -p cairn-node --test restore restore_door -- --nocapture
```

Expected: all three PASS. `cargo clippy -p cairn-node --all-targets` clean.

- [ ] **Step 5: Commit**

```bash
git add db/009_node_supersede_and_restore.sql crates/cairn-node/tests/restore.rs
git commit -m "harden(node): self-trusting restore_node_event door, empty-genesis fenced (ADR-0026 slice C)"
```

---

### Task 4: Pure restore helpers — `resolve_dead_node_id` + `old_genesis_meta`

**Files:**
- Create: `crates/cairn-node/src/restore.rs`
- Modify: `crates/cairn-node/src/lib.rs` (add `pub mod restore;`)
- Test: unit tests inside `restore.rs`

**Interfaces:**
- Produces:
  - `RestoreError` (thiserror enum: `Decode(String)`, `NoGenesis`, `Ambiguous(usize)`).
  - `resolve_dead_node_id(events: &[Vec<u8>], explicit: Option<&str>) -> Result<String, RestoreError>` — returns the hex node-id of the dead node (single-enroll auto-detect; explicit overrides; multi-enroll without explicit errors).
  - `old_genesis_meta(events: &[Vec<u8>], node_id_hex: &str) -> Option<(String, String)>` — `(display_name, address)` from the enroll whose content-address == `node_id_hex`.
- Consumes: `cairn_event::verify_self_described`, `cairn_event::event_address`, `backup::parse_medium` (slice B).

- [ ] **Step 1: Write the failing unit tests**

Create `crates/cairn-node/src/restore.rs` with ONLY the tests + stubs that fail to compile/pass:

```rust
//! ADR-0026 slice C — restore orchestration (apply a backup medium under a new identity).
//!
//! WHY: the live apply_remote_node_event gate is the PEER-admission path and rejects a
//! node rehydrating its OWN history (no trust set in a fresh DB). Restore therefore uses
//! the self-trusting restore_node_event door (db/009), then mints a fresh key and records
//! a node-level supersede (the old signing key is never backed up). This module holds the
//! PURE helpers (dead-node-id resolution, old-genesis metadata) and the thin DB
//! orchestration; main.rs owns key-minting + recovery-code printing (as `init` does).

use cairn_event::{event_address, verify_self_described};

#[derive(thiserror::Error, Debug)]
pub enum RestoreError {
    #[error("decode: {0}")]
    Decode(String),
    #[error("medium has no genesis (node.enrolled) event")]
    NoGenesis,
    #[error("medium carries {0} enrolls; pass --superseded-node <hex> to pick the dead node")]
    Ambiguous(usize),
}

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_event::{sign, EventBody, Hlc, SigningKey};

    fn enroll(sk: &SigningKey, name: &str) -> Vec<u8> {
        let kid = hex::encode(sk.verifying_key().to_bytes());
        let body = EventBody {
            event_id: uuid::Uuid::now_v7().to_string(),
            patient_id: crate::identity::NIL_PATIENT.into(),
            event_type: "node.enrolled".into(),
            schema_version: "node/1".into(),
            hlc: Hlc { wall: 1, counter: 0, node_origin: name.into() },
            t_effective: None,
            signer_key_id: kid,
            contributors: serde_json::json!([]),
            payload: serde_json::json!({ "display_name": name, "address": "10.0.0.1:7843" }),
            attachments: vec![],
        };
        sign(&body, sk).unwrap().signed_bytes
    }

    fn sk() -> SigningKey { cairn_event::generate_key().unwrap().0 }
    fn node_id(ev: &[u8]) -> String { hex::encode(event_address(ev)) }

    #[test]
    fn single_enroll_auto_detects_the_dead_node() {
        let k = sk();
        let ev = enroll(&k, "Solo");
        let got = resolve_dead_node_id(&[ev.clone()], None).unwrap();
        assert_eq!(got, node_id(&ev), "the sole enroll is the dead node");
    }

    #[test]
    fn multiple_enrolls_require_an_explicit_arg() {
        let a = enroll(&sk(), "A");
        let b = enroll(&sk(), "B");
        let err = resolve_dead_node_id(&[a, b], None).unwrap_err();
        assert!(matches!(err, RestoreError::Ambiguous(2)));
    }

    #[test]
    fn explicit_arg_overrides_auto_detect() {
        let a = enroll(&sk(), "A");
        let b = enroll(&sk(), "B");
        let want = node_id(&b);
        let got = resolve_dead_node_id(&[a, b], Some(&want)).unwrap();
        assert_eq!(got, want);
    }

    #[test]
    fn no_enroll_is_an_error() {
        let err = resolve_dead_node_id(&[], None).unwrap_err();
        assert!(matches!(err, RestoreError::NoGenesis));
    }

    #[test]
    fn old_genesis_meta_reads_name_and_address() {
        let k = sk();
        let ev = enroll(&k, "Clinic-7");
        let (name, addr) = old_genesis_meta(&[ev.clone()], &node_id(&ev)).unwrap();
        assert_eq!(name, "Clinic-7");
        assert_eq!(addr, "10.0.0.1:7843");
    }
}
```

- [ ] **Step 2: Run them to verify they fail**

```bash
cargo test -p cairn-node --lib restore:: -- --nocapture
```

Expected: FAIL — `resolve_dead_node_id` / `old_genesis_meta` not defined.

- [ ] **Step 3: Implement the pure helpers**

In `restore.rs`, add `pub mod restore;` to `lib.rs` first:

```rust
pub mod restore;
```

Then add the helpers above the `#[cfg(test)]` block in `restore.rs`:

```rust
/// Every enroll (node.enrolled) on the medium, as (node_id_hex, body) pairs. A node-id
/// is the content-address of its genesis, so we hash each verified enroll's bytes. Only
/// events that VERIFY are considered (a corrupt enroll cannot name a node).
fn enrolls(events: &[Vec<u8>]) -> Vec<(String, cairn_event::EventBody)> {
    events.iter().filter_map(|e| {
        let body = verify_self_described(e).ok()?;
        if body.event_type == "node.enrolled" {
            Some((hex::encode(event_address(e)), body))
        } else {
            None
        }
    }).collect()
}

/// Resolve the dead node's id (hex) to supersede on restore.
///
/// - `explicit` (operator's --superseded-node) always wins — it is normalized to lower
///   hex but otherwise trusted (the operator knows which node they are restoring).
/// - else, if the medium has exactly ONE enroll, that is the dead node (the solo-clinic
///   case — ADR-0026's primary deployment).
/// - else it is ambiguous (a federated node whose log holds peers' genesis too) and we
///   fail closed, telling the operator to pass --superseded-node.
pub fn resolve_dead_node_id(
    events: &[Vec<u8>],
    explicit: Option<&str>,
) -> Result<String, RestoreError> {
    if let Some(e) = explicit.map(str::trim).filter(|s| !s.is_empty()) {
        return Ok(e.to_ascii_lowercase());
    }
    let found = enrolls(events);
    match found.len() {
        0 => Err(RestoreError::NoGenesis),
        1 => Ok(found.into_iter().next().unwrap().0),
        n => Err(RestoreError::Ambiguous(n)),
    }
}

/// The (display_name, address) recorded in the enroll whose content-address == node_id.
/// Used so the new genesis keeps the node's name/address (paper-parity: a restored node
/// is the same clinic). Returns None if no such enroll is on the medium.
pub fn old_genesis_meta(events: &[Vec<u8>], node_id_hex: &str) -> Option<(String, String)> {
    let want = node_id_hex.to_ascii_lowercase();
    enrolls(events).into_iter().find(|(id, _)| *id == want).map(|(_, body)| {
        let name = body.payload.get("display_name").and_then(|v| v.as_str())
            .unwrap_or("restored-node").to_string();
        let addr = body.payload.get("address").and_then(|v| v.as_str())
            .unwrap_or("").to_string();
        (name, addr)
    })
}
```

- [ ] **Step 4: Run them to verify they pass**

```bash
cargo test -p cairn-node --lib restore:: -- --nocapture
```

Expected: all five PASS. `cargo clippy -p cairn-node --all-targets` clean.

- [ ] **Step 5: Commit**

```bash
git add crates/cairn-node/src/restore.rs crates/cairn-node/src/lib.rs
git commit -m "harden(node): pure restore helpers (dead-node-id resolution + genesis meta) (ADR-0026 slice C)"
```

---

### Task 5: Restore orchestration — `apply_medium` + `finalize_identity` + round-trip

**Files:**
- Modify: `crates/cairn-node/src/restore.rs` (add DB orchestration)
- Test: `crates/cairn-node/tests/restore.rs` (add the round-trip test)

**Interfaces:**
- Produces:
  - `apply_medium(db: &Client, events: &[Vec<u8>]) -> anyhow::Result<usize>` — applies each signed event through `restore_node_event`; returns the count applied.
  - `RestoreOutcome { new_node_id_hex: String, superseded_node_id_hex: String, events_applied: usize }`.
  - `finalize_identity(db, sk, kid, name, address, old_node_id_hex) -> anyhow::Result<RestoreOutcome>` — authors the new genesis (via `identity::provision`) then the supersede (via `identity::author_supersede`).
- Consumes: `identity::provision`, `identity::author_supersede` (Task 2), `restore_node_event` (Task 3).

- [ ] **Step 1: Write the failing round-trip test**

Append to `crates/cairn-node/tests/restore.rs`:

```rust
/// Full round-trip: back up node A's event set, restore it into a FRESH db under a NEW
/// identity, and assert the ADR-0026 point-1/4 guarantees hold.
#[tokio::test]
async fn restore_round_trip_rehydrates_under_a_new_identity() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let a = db::connect_and_load_schema(&base).await.unwrap();
    db::reset_node_federation_tables(&a).await.ok();

    // --- original node A ---
    let tmp = tempfile::tempdir().unwrap();
    let (sk_a, kid_a) = keystore::generate_plaintext(&tmp.path().join("a.key")).unwrap();
    identity::provision(&a, &sk_a, &kid_a, "Clinic-A", "127.0.0.1:7930").await.unwrap();
    let old = identity::load_local(&a).await.unwrap();

    // Read A's event set the way `backup` does (signed_bytes in seq order).
    let medium: Vec<Vec<u8>> = cairn_node::backup::read_event_set(&a).await.unwrap();
    assert_eq!(medium.len(), 1, "solo node A has just its genesis");

    // --- simulate disk death: a fresh, un-enrolled DB ---
    db::reset_node_federation_tables(&a).await.ok();

    // --- restore under a NEW key ---
    let (sk_new, kid_new) = keystore::generate_plaintext(&tmp.path().join("new.key")).unwrap();
    let dead = cairn_node::restore::resolve_dead_node_id(&medium, None).unwrap();
    assert_eq!(dead, old.node_id_hex, "auto-detected dead node == A");
    let (name, addr) = cairn_node::restore::old_genesis_meta(&medium, &dead).unwrap();

    let applied = cairn_node::restore::apply_medium(&a, &medium).await.unwrap();
    assert_eq!(applied, 1);
    let outcome = cairn_node::restore::finalize_identity(
        &a, &sk_new, &kid_new, &name, &addr, &dead).await.unwrap();

    // (a) old events present:
    let n_enroll: i64 = a.query_one(
        "SELECT count(*) FROM node_event WHERE op='enroll'", &[]).await.unwrap().get(0);
    assert_eq!(n_enroll, 2, "old genesis (restored) + new genesis");
    // (b) local_node == the new id:
    let local = identity::load_local(&a).await.unwrap();
    assert_eq!(local.node_id_hex, outcome.new_node_id_hex);
    assert_ne!(local.node_id_hex, old.node_id_hex, "new physical trust boundary");
    // (c) supersede recorded with subject == old id:
    let edge = a.query_one(
        "SELECT encode(superseded_node_id,'hex') AS old, encode(new_node_id,'hex') AS new
         FROM node_lineage", &[]).await.unwrap();
    assert_eq!(edge.get::<_, String>("old"), old.node_id_hex);
    assert_eq!(edge.get::<_, String>("new"), local.node_id_hex);
    // (d) trust_peer empty -> must re-peer:
    let peers: i64 = a.query_one("SELECT count(*) FROM trust_peer", &[]).await.unwrap().get(0);
    assert_eq!(peers, 0, "a restored node re-peers from empty (ADR-0026 point 4)");
    // (e) the restore door is now fenced closed:
    let other = cairn_event::generate_key().unwrap().0;
    let ev = synth_enroll(&other, "Late");
    let err = a.execute("SELECT restore_node_event($1)", &[&ev]).await.unwrap_err();
    assert!(err.to_string().contains("already enrolled"), "door closes after genesis");
}
```

- [ ] **Step 2: Run it to verify it fails**

```bash
CAIRN_TEST_PG="$CAIRN_TEST_PG" cargo test -p cairn-node --test restore restore_round_trip -- --nocapture
```

Expected: FAIL — `apply_medium` / `finalize_identity` / `RestoreOutcome` not defined.

- [ ] **Step 3: Implement the orchestration**

In `restore.rs`, add (above the test module):

```rust
use tokio_postgres::Client;

/// What a completed restore produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestoreOutcome {
    pub new_node_id_hex: String,
    pub superseded_node_id_hex: String,
    pub events_applied: usize,
}

/// Apply every signed event from the medium through the self-trusting restore door, in
/// medium order (the node's own genesis is first, so non-enroll events resolve their
/// author). Idempotent: the door's ON CONFLICT DO NOTHING makes re-application a no-op.
/// MUST run while the DB is still un-enrolled — the door fails closed once a genesis
/// exists (which is exactly what finalize_identity creates next).
pub async fn apply_medium(db: &Client, events: &[Vec<u8>]) -> anyhow::Result<usize> {
    use anyhow::Context;
    for (i, e) in events.iter().enumerate() {
        db.execute("SELECT restore_node_event($1)", &[e]).await
            .with_context(|| format!("applying restored event #{i}"))?;
    }
    Ok(events.len())
}

/// After the medium is applied, mint the node's NEW identity: author a fresh genesis
/// (sets local_node = NEW and permanently fences the restore door closed), then author a
/// node-level supersede(dead -> new). The signing key is the freshly-minted one (the old
/// key was never backed up). Returns the new + superseded node-ids for the operator.
pub async fn finalize_identity(
    db: &Client,
    sk: &cairn_event::SigningKey,
    key_id: &str,
    name: &str,
    address: &str,
    old_node_id_hex: &str,
) -> anyhow::Result<RestoreOutcome> {
    let new_node_id_hex = crate::identity::provision(db, sk, key_id, name, address).await?;
    crate::identity::author_supersede(db, sk, key_id, &new_node_id_hex, old_node_id_hex).await?;
    Ok(RestoreOutcome {
        new_node_id_hex,
        superseded_node_id_hex: old_node_id_hex.to_ascii_lowercase(),
        events_applied: 0, // set by the caller, which knows the medium length
    })
}
```

Note: `events_applied` is populated by the CLI (Task 6) from `apply_medium`'s return; `finalize_identity` leaves it 0 to avoid threading the count. (The round-trip test reads the count from `apply_medium` directly.)

- [ ] **Step 4: Run it to verify it passes**

```bash
CAIRN_TEST_PG="$CAIRN_TEST_PG" cargo test -p cairn-node --test restore restore_round_trip -- --nocapture
```

Expected: PASS. `cargo clippy -p cairn-node --all-targets` clean.

- [ ] **Step 5: Commit**

```bash
git add crates/cairn-node/src/restore.rs crates/cairn-node/tests/restore.rs
git commit -m "harden(node): restore orchestration (apply medium + finalize new identity) (ADR-0026 slice C)"
```

---

### Task 6: CLI `restore` subcommand

**Files:**
- Modify: `crates/cairn-node/src/main.rs` (add `Restore` to the `Cmd` enum + its match arm)

**Interfaces:**
- Consumes: `db::connect_and_load_schema`, `backup::{parse_medium, verify_events}`, `restore::{resolve_dead_node_id, old_genesis_meta, apply_medium, finalize_identity}`, `keystore::{generate_sealed, generate_plaintext}`, `seal::generate_recovery_code`, `resolve_passphrase`, `print_recovery_code`.

- [ ] **Step 1: Add the subcommand variant**

In `main.rs`, in `enum Cmd`, after `VerifyBackup { … }`:

```rust
    /// Restore a node from a cold-peer backup medium into a FRESH, un-enrolled database
    /// (ADR-0026 slice C). Verifies the medium, mints a NEW sealed keypair (the old
    /// signing key is never backed up), rehydrates the old event history through the
    /// self-trusting restore door, authors a new genesis, and records a supersede linking
    /// the dead node to the new one. The node then re-peers from empty.
    Restore {
        /// Path of the backup medium to restore (as written by `backup`).
        #[arg(long)]
        from: PathBuf,
        /// For a federated medium with multiple enrolls: the dead node-id (hex) to
        /// supersede. Optional for a solo node (auto-detected from the sole enroll).
        #[arg(long)]
        superseded_node: Option<String>,
        /// Operational passphrase for the NEW sealed key (else CAIRN_KEY_PASSPHRASE, else prompt).
        #[arg(long, env = "CAIRN_KEY_PASSPHRASE")]
        passphrase: Option<String>,
        /// Write the new key UNSEALED (test nodes only — no recovery escrow).
        #[arg(long)]
        insecure_plaintext: bool,
    },
```

- [ ] **Step 2: Add the match arm**

In `main()`'s `match cli.cmd`, add (after the `VerifyBackup` arm):

```rust
        Cmd::Restore { from, superseded_node, passphrase, insecure_plaintext } => {
            // 1. Read + verify the medium offline (no DB needed yet). Bail on tamper.
            let bytes = std::fs::read(&from)
                .with_context(|| format!("reading backup medium {}", from.display()))?;
            let events = cairn_node::backup::parse_medium(&bytes)?;
            let report = cairn_node::backup::verify_events(&events);
            if !report.all_intact() {
                anyhow::bail!(
                    "refusing to restore a medium that fails self-verification: {}/{} intact, \
                     first bad at index {:?}",
                    report.intact, report.total, report.first_bad
                );
            }
            // 2. Resolve the dead node-id (solo auto-detect, else --superseded-node).
            let dead = cairn_node::restore::resolve_dead_node_id(&events, superseded_node.as_deref())?;
            let (name, address) = cairn_node::restore::old_genesis_meta(&events, &dead)
                .unwrap_or_else(|| ("restored-node".to_string(), String::new()));

            // 3. Connect to the FRESH db and load the schema (DDL: owner privileges, like init).
            let db = cairn_node::db::connect_and_load_schema(&cli.conn).await?;
            if cairn_node::identity::load_local_opt(&db).await?.is_some() {
                anyhow::bail!(
                    "target database already has an enrolled node; restore is only into a \
                     fresh, un-enrolled database (the restore door is fenced closed otherwise)"
                );
            }

            // 4. Mint the NEW key (the old signing key was never backed up).
            let (sk, kid) = if insecure_plaintext {
                eprintln!("WARNING: --insecure-plaintext: new key written UNSEALED (test use only)");
                cairn_node::keystore::generate_plaintext(&cli.key)?
            } else {
                let op = resolve_passphrase(passphrase)?;
                let code = Zeroizing::new(cairn_node::seal::generate_recovery_code());
                print_recovery_code(&code);
                cairn_node::keystore::generate_sealed(&cli.key, &op, &code)?
            };

            // 5. Apply old events through the self-trusting door (db still un-enrolled),
            //    then author the new genesis + supersede.
            let applied = cairn_node::restore::apply_medium(&db, &events).await?;
            let outcome = cairn_node::restore::finalize_identity(
                &db, &sk, &kid, &name, &address, &dead).await?;

            println!("restored {applied} event(s) from {}", from.display());
            println!("new node {}", outcome.new_node_id_hex);
            println!("supersedes {}", outcome.superseded_node_id_hex);
            println!("re-peer with `cairn-node pair-offer` / `pair-accept` (trust resets on restore)");
        }
```

- [ ] **Step 3: Verify the whole workspace builds + all tests pass**

```bash
cargo build -p cairn-node
CAIRN_TEST_PG="$CAIRN_TEST_PG" cargo test -p cairn-node -- --nocapture
cargo clippy -p cairn-node --all-targets
```

Expected: builds; all tests pass (DB-gated ones run with `CAIRN_TEST_PG` set); clippy clean.

- [ ] **Step 4: Manual smoke (optional, documents the ceremony)**

```bash
# (against a throwaway DB) init -> backup -> drop+recreate db -> restore
cairn-node --conn "$CAIRN_TEST_PG" --key /tmp/a.key init --name Solo --address 127.0.0.1:7843 --insecure-plaintext
cairn-node --conn "$CAIRN_TEST_PG" --key /tmp/a.key backup --to /tmp/cairn.medium
# simulate disk death: recreate the database, then:
cairn-node --conn "$CAIRN_TEST_PG" --key /tmp/new.key restore --from /tmp/cairn.medium --insecure-plaintext
cairn-node --conn "$CAIRN_TEST_PG" --key /tmp/new.key status   # shows new id (supersedes old)
```

- [ ] **Step 5: Commit**

```bash
git add crates/cairn-node/src/main.rs
git commit -m "harden(node): cairn-node restore subcommand (ADR-0026 slice C)"
```

---

### Task 7: `status` lineage line

**Files:**
- Modify: `crates/cairn-node/src/identity.rs` (the `Status` struct + `status()`)
- Modify: `crates/cairn-node/src/main.rs` (the `Status` print arm)
- Test: `crates/cairn-node/tests/restore.rs` (add a small assertion via the existing round-trip, or a focused query test)

**Interfaces:**
- Produces: `Status.supersedes: Option<String>` — the dead node-id this node supersedes, if any.
- Consumes: `node_lineage` view.

- [ ] **Step 1: Write the failing test**

Append to `crates/cairn-node/tests/restore.rs`:

```rust
/// After a restore, `status` reports the supersede lineage (this node supersedes the dead one).
#[tokio::test]
async fn status_reports_supersede_lineage() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let a = db::connect_and_load_schema(&base).await.unwrap();
    db::reset_node_federation_tables(&a).await.ok();

    let tmp = tempfile::tempdir().unwrap();
    let (sk, kid) = keystore::generate_plaintext(&tmp.path().join("a.key")).unwrap();
    identity::provision(&a, &sk, &kid, "A", "127.0.0.1:7940").await.unwrap();
    let id = identity::load_local(&a).await.unwrap();
    let old_hex = "1220".to_string() + &"cd".repeat(32);
    identity::author_supersede(&a, &sk, &kid, &id.node_id_hex, &old_hex).await.unwrap();

    let st = identity::status(&a, &tmp.path().join("a.key")).await.unwrap();
    assert_eq!(st.supersedes.as_deref(), Some(old_hex.as_str()),
        "status must surface the supersede lineage");
}
```

- [ ] **Step 2: Run it to verify it fails**

```bash
CAIRN_TEST_PG="$CAIRN_TEST_PG" cargo test -p cairn-node --test restore status_reports_supersede_lineage -- --nocapture
```

Expected: FAIL — `Status` has no `supersedes` field.

- [ ] **Step 3: Add the field + query**

In `identity.rs`, add to the `Status` struct (after `last_backup`):

```rust
    /// The dead node-id this node supersedes (ADR-0026 slice C), if it was restored from
    /// a backup under a new identity. `None` for a node provisioned fresh via `init`.
    pub supersedes: Option<String>,
```

In `status()`, before the `Ok(Status { … })`, add the lineage query:

```rust
    // Supersede lineage (ADR-0026 slice C): if THIS node supersedes a dead one, surface it.
    // `node_lineage.new_node_id` is the author (this node); there is at most one for v1.
    let supersedes: Option<String> = db
        .query_opt(
            "SELECT encode(superseded_node_id,'hex') AS old FROM node_lineage \
             WHERE new_node_id = (SELECT node_id FROM local_node WHERE id) \
             ORDER BY hlc_wall DESC, hlc_counter DESC LIMIT 1",
            &[],
        )
        .await?
        .map(|r| r.get::<_, String>("old"));
```

Add `supersedes,` to the returned `Status { … }`.

- [ ] **Step 4: Add the print line in `main.rs`**

In the `Cmd::Status` arm, after `println!("last_backup   {}", st.last_backup);`:

```rust
            if let Some(old) = &st.supersedes {
                println!("supersedes    {old}");
            }
```

- [ ] **Step 5: Run the test to verify it passes**

```bash
CAIRN_TEST_PG="$CAIRN_TEST_PG" cargo test -p cairn-node --test restore status_reports_supersede_lineage -- --nocapture
cargo clippy -p cairn-node --all-targets
```

Expected: PASS; clippy clean. Also re-run the full suite: `CAIRN_TEST_PG="$CAIRN_TEST_PG" cargo test -p cairn-node`.

- [ ] **Step 6: Commit**

```bash
git add crates/cairn-node/src/identity.rs crates/cairn-node/src/main.rs crates/cairn-node/tests/restore.rs
git commit -m "harden(node): status surfaces supersede lineage (ADR-0026 slice C)"
```

---

### Task 8: Docs — HANDOVER + ROADMAP

**Files:**
- Modify: `docs/HANDOVER.md`
- Modify: `docs/ROADMAP.md`

- [ ] **Step 1: Update HANDOVER.md**

Replace the slice-B "export half closed this session" bullet's tail and the "Still open" line to reflect slice C done. Specifically, mark the issue-#50 item closed: change the "Next up: **slice C …**" sentence under the node-gaps menu to record slice C as closed (restore door fenced empty-genesis, node-level supersede, `cairn-node restore`, round-trip green), and move the remaining open item to just the sealed local-state export (ADR-0026 point 3). Update the session header line to this session's date and summary.

- [ ] **Step 2: Update ROADMAP.md**

In Phase 5, update the "Backup-as-cold-peer" bullet: append that **restore-apply + new-identity supersede (slice C)** is done (self-trusting `restore_node_event` door, `node.superseded` op, `cairn-node restore`), leaving only the sealed local-state export (point 3) of ADR-0026 open.

- [ ] **Step 3: Verify the full suite once more, then commit**

```bash
cargo build -p cairn-node
CAIRN_TEST_PG="$CAIRN_TEST_PG" cargo test -p cairn-node
cargo clippy -p cairn-node --all-targets
git add docs/HANDOVER.md docs/ROADMAP.md
git commit -m "docs: HANDOVER + ROADMAP reflect ADR-0026 slice C (restore + supersede) done"
```

---

## Self-Review

**Spec coverage:**
- Empty-genesis fence → Task 3 (door) + round-trip (e). ✓
- Dead-node-id auto-detect + explicit fallback → Task 4 + Task 6. ✓
- Restore flow (verify → resolve → mint → apply → genesis → supersede → re-peer) → Tasks 5/6 + round-trip. ✓
- Schema: op-CHECK widen (Task 1), restore_node_event (Task 3), submit_node_event supersede (Task 2), node_lineage (Task 1). ✓
- `author_supersede` (Task 2); `restore.rs` orchestration (Tasks 4/5); status lineage (Task 7). ✓
- Tests: door fence/accept/tamper (Task 3); round-trip a–e (Task 5); pure resolution (Task 4); SQL op-CHECK + view (Task 1). ✓
- Scope boundary (sealed export, shred-replay deferred) → documented in spec; no task (correctly out of scope). ✓
- Honest limitation (retry = clean DB) → enforced by Task 6 step 2's "already enrolled" bail; documented in spec. ✓

**Placeholder scan:** none — every step has concrete code/commands. (`events_applied: 0` in `finalize_identity` is intentional and documented, not a placeholder.)

**Type consistency:** `resolve_dead_node_id`/`old_genesis_meta` signatures match between Task 4 definition and Task 5/6 call sites; `RestoreOutcome` fields (`new_node_id_hex`, `superseded_node_id_hex`, `events_applied`) consistent; `author_supersede` signature consistent across Tasks 2/5; `Status.supersedes: Option<String>` consistent Tasks 7. `restore_node_event(bytea)` grant/call consistent Tasks 3/5/6.
