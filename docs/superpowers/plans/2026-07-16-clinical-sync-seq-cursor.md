# Clinical-plane seq cursor + periodic full sweep — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close the clinical-plane pull's HLC-watermark skip hazard (issue #196) by cursoring on a monotonic node-local `seq` plus a periodic full sweep — the #38 node-plane treatment, ported.

**Architecture:** Add `event_log.seq` (BIGINT IDENTITY, node-local insertion order); the clinical pull cursors on it (per-peer `sync_state.last_seq`) instead of the HLC watermark, the quarantine re-offer floor becomes a seq `refused_seq`, and `cmd_run` does a full sweep every 10 cycles as the correctness floor. The wire gains an additive `EventsAfterSeq` request + a parallel `seqs[]` array. The two sync planes (clinical `cairn-sync`, node `cairn-node`) stay separate implementations of the same pattern.

**Tech Stack:** Rust (sync `postgres` crate), PostgreSQL ≥ 18, in-DB migrations under `db/`, `serde_json` wire.

## Global Constraints

- **License:** AGPL-3.0; every dependency AGPL-3.0-compatible. No new dependencies in this plan.
- **Safety tier (§9):** this is safety-critical (sync/merge engine). Optimize for reviewer-legibility; comment *why*, for a junior joiner.
- **TDD:** RED test first, then minimal code, every task. No production code without a test that drove it.
- **Additive-only schema (ADR-0012 / principle 11):** every DB change is `ADD COLUMN IF NOT EXISTS` / `CREATE ... IF NOT EXISTS`. No `DROP COLUMN`. The vestigial HLC watermark/floor columns are kept, deprecated-in-place.
- **Principle 12 (wire):** `Request`/`EventsResponse` changes are additive only; the old `EventsAfter` variant stays served.
- **House rule 6:** all tests pass before every commit. `cargo fmt` + `cargo clippy -D warnings` + `cargo test --workspace` green.
- **MSRV 1.74:** use `cycle % N == 0` with `#[allow(clippy::manual_is_multiple_of)]`, not `is_multiple_of`.
- **DB-gated tests:** need `CAIRN_TEST_PG` (+ `CAIRN_TEST_PG2` for `clinical_pull.rs`); they self-serialize via a Postgres advisory lock. Run locally against the Mac cluster (`:5532`, DBs `cairn_test`/`cairn_test2`).
- **Crypto in tests:** never hard-code key material; derive at runtime (house rule 6-crypto). N/A here (tests reuse existing `generate_key`/`enrolled_key`).

---

### Task 1: `db/036` migration + SCHEMA registration + schema probe

Adds the three columns, registers the migration in both explicit SCHEMA lists, and repoints the `connect_checked` "newest piece" probe at the new columns.

**Files:**
- Create: `db/036_clinical_sync_seq.sql`
- Modify: `crates/cairn-sync/src/main.rs` — `SCHEMA` const (~line 40), `connect_checked` (~line 1506)
- Modify: `crates/cairn-node/src/db.rs` — `SCHEMA` const (~line 5, append after 035)
- Test: `crates/cairn-sync/src/main.rs` tests — new `db036_adds_seq_columns`, and migrate `connect_checked_fails_legibly_on_pre_quarantine_schema` (~line 2997)

**Interfaces:**
- Produces: `event_log.seq BIGINT`, `sync_state.last_seq BIGINT NOT NULL DEFAULT 0`, `sync_quarantine.refused_seq BIGINT NOT NULL DEFAULT 0` — consumed by Tasks 2–5.

- [ ] **Step 1: Write the failing test** (append to `mod tests` in `crates/cairn-sync/src/main.rs`)

```rust
#[test]
fn db036_adds_seq_columns() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let mut c = locked_client(&base); // loads the whole SCHEMA subset
    let ok: bool = c
        .query_one(
            "SELECT
               EXISTS (SELECT 1 FROM information_schema.columns
                       WHERE table_name='event_log'      AND column_name='seq')
           AND EXISTS (SELECT 1 FROM information_schema.columns
                       WHERE table_name='sync_state'     AND column_name='last_seq')
           AND EXISTS (SELECT 1 FROM information_schema.columns
                       WHERE table_name='sync_quarantine' AND column_name='refused_seq')",
            &[],
        )
        .unwrap()
        .get(0);
    assert!(ok, "db/036 must add event_log.seq, sync_state.last_seq, sync_quarantine.refused_seq");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `CAIRN_TEST_PG='host=localhost port=5532 dbname=cairn_test user=... ' cargo test -p cairn-sync db036_adds_seq_columns -- --nocapture`
Expected: FAIL — `db/036` not loaded (columns absent). (If `CAIRN_TEST_PG` is unset the test prints "skipped" and passes — set it.)

- [ ] **Step 3: Create the migration**

```sql
-- db/036_clinical_sync_seq.sql
-- Cairn — clinical-plane incremental-sync cursor keyed on a monotonic, node-LOCAL
-- insertion-order `seq` (issue #196, review finding B1). This ports the #38
-- node-plane treatment (db/007 node_event.seq + sync_cursor) to the clinical pull.
--
-- WHY: cairn-sync's do_pull cursored on the HLC watermark and never swept, so an
-- event landing in a peer's event_log with an HLC BELOW an already-advanced
-- watermark — a multi-hop arrival from a third node, or an L2 agent self-stamping
-- an older hlc_wall — was never re-fetched: a silent set-union / convergence
-- violation. A node-LOCAL insertion-order seq fixes it: a newly-LEARNED event
-- (whatever its HLC) always gets a fresh high seq, so it always sorts above the
-- puller's cursor and cannot be skipped. The periodic full sweep (cairn-sync
-- cmd_run) is the correctness floor for the residual BIGSERIAL out-of-order-commit
-- gap.
--
-- All changes are additive ALTERs (ADR-0012 / principle 11). No CREATE TABLE is
-- widened, so there is no migration_replay_widening.rs `WIDENED` entry to add:
-- the columns' SOLE source is the idempotent ALTER here, uniform for fresh and
-- upgraded DBs alike (exactly how db/007 added node_event.seq).

BEGIN;

-- event_log.seq — the monotonic node-local cursor key. IDENTITY is assigned at
-- INSERT, so the submit_event / apply_remote_event INSERT column lists need no
-- change (they never name seq; GENERATED ALWAYS also forbids an explicit value).
-- Never signed, never on the wire core — sync transport metadata only (principle 12).
ALTER TABLE event_log ADD COLUMN IF NOT EXISTS seq BIGINT GENERATED ALWAYS AS IDENTITY;
CREATE INDEX IF NOT EXISTS event_log_seq_idx ON event_log (seq);

-- sync_state.last_seq — the per-peer pull checkpoint (highest serving-node seq
-- pulled from `peer`). Advance-only, written by do_pull via a raw GREATEST UPDATE
-- (sync_state is node-local operational state, outside the append-only floor).
-- SUPERSEDES the sync_state.hlc_wall / hlc_counter watermark columns, which are
-- now vestigial (kept, never dropped — a DROP is the non-additive move ADR-0012
-- forbids: an older cairn-sync binary reading this DB still SELECTs hlc_wall).
ALTER TABLE sync_state ADD COLUMN IF NOT EXISTS last_seq BIGINT NOT NULL DEFAULT 0;

-- sync_quarantine.refused_seq — the serving seq at which an unverifiable event was
-- refused. The re-offer floor is now DERIVED as min(refused_seq) over a peer's
-- UNACKED rows (no persisted floor column), so it SUPERSEDES
-- sync_state.quarantine_floor_wall / _counter (likewise vestigial, kept). Legacy
-- rows default to 0 = re-offer from the log start until they resolve (safe,
-- self-limiting; mirrors node_event_quarantine.refused_seq).
ALTER TABLE sync_quarantine ADD COLUMN IF NOT EXISTS refused_seq BIGINT NOT NULL DEFAULT 0;

COMMIT;
```

- [ ] **Step 4: Register in both SCHEMA lists**

In `crates/cairn-sync/src/main.rs`, append to the `SCHEMA` array (after the `029_hlc_collision_log` entry, ~line 86):

```rust
    // db/036 (issue #196): the clinical-plane seq cursor. event_log.seq +
    // sync_state.last_seq + sync_quarantine.refused_seq. do_pull cursors on seq
    // (never the skip-prone HLC watermark) + cmd_run sweeps.
    (
        "036_clinical_sync_seq",
        include_str!("../../../db/036_clinical_sync_seq.sql"),
    ),
```

In `crates/cairn-node/src/db.rs`, append the identical entry to its `SCHEMA` array after the last existing db/035 entry (a node loads event_log too; without it the clinical column is missing on a real node). Match the surrounding formatting.

- [ ] **Step 5: Repoint the `connect_checked` probe** (`crates/cairn-sync/src/main.rs`, ~line 1506)

```rust
    let ok: bool = client
        .query_one(
            "SELECT EXISTS (SELECT 1 FROM information_schema.columns
                            WHERE table_name='event_log'
                              AND column_name='seq')
                AND EXISTS (SELECT 1 FROM information_schema.columns
                            WHERE table_name='sync_quarantine'
                              AND column_name='refused_seq')",
            &[],
        )?
        .get(0);
    if !ok {
        return Err(
            "this database predates the clinical seq-cursor schema (db/036) this binary \
             requires — run `cairn-sync init --conn <same URI>` (idempotent) to apply \
             the migrations, then retry"
                .into(),
        );
    }
```

- [ ] **Step 6: Migrate `connect_checked_fails_legibly_on_pre_quarantine_schema`** (~line 2997)

The test drops a column to simulate an old DB and expects `connect_checked` to fail legibly. Repoint it at a db/036 column. Change the `DROP COLUMN` and the message assertion:

```rust
    // Simulate a DB predating db/036: knock the seq cursor off event_log.
    c.batch_execute("ALTER TABLE event_log DROP COLUMN IF EXISTS seq")
        .unwrap();
    let err = connect_checked(&base).unwrap_err();
    assert!(
        err.to_string().contains("db/036"),
        "must name the missing migration, got: {err}"
    );
    // Restore so later serial-locked tests see the full schema.
    c.batch_execute(include_str!("../../../db/036_clinical_sync_seq.sql"))
        .unwrap();
```
(Adjust to match the test's existing structure — the exact drop/restore idiom and helper names already present; keep its skip-guard and `locked_client` usage.)

- [ ] **Step 7: Run tests to verify they pass**

Run: `cargo test -p cairn-sync db036_adds_seq_columns connect_checked_fails_legibly -- --nocapture` (with `CAIRN_TEST_PG` set)
Expected: PASS both. Then `cargo build -p cairn-node` (SCHEMA list still compiles).

- [ ] **Step 8: Commit**

```bash
git add db/036_clinical_sync_seq.sql crates/cairn-sync/src/main.rs crates/cairn-node/src/db.rs
git commit -m "feat(#196): db/036 clinical seq-cursor columns + schema registration"
```

---

### Task 2: Wire — `EventsAfterSeq` request, `seqs[]` response, serve arm

Additive wire changes + the serve-side query arm. Unit-tested (no DB) for round-trip and back-compat.

**Files:**
- Modify: `crates/cairn-sync/src/main.rs` — `enum Request` (~line 219), `struct EventsResponse` (~line 231), `serve_conn` `match req` (~line 1987), the `response_json` test helper (~line 2439), extend `events_response_decodes_pre_attestation_wire_format` (~line 2331)

**Interfaces:**
- Produces: `Request::EventsAfterSeq { after_seq: i64 }`; `EventsResponse.seqs: Vec<i64>` (serde `default`) — consumed by Task 3's `do_pull` and the serve arm.

- [ ] **Step 1: Write the failing unit tests** (in `mod tests`, no DB — put near the existing wire tests)

```rust
#[test]
fn events_after_seq_request_round_trips() {
    let req = Request::EventsAfterSeq { after_seq: 42 };
    let bytes = serde_json::to_vec(&req).unwrap();
    match serde_json::from_slice::<Request>(&bytes).unwrap() {
        Request::EventsAfterSeq { after_seq } => assert_eq!(after_seq, 42),
        other => panic!("wrong variant: {other:?}"),
    }
}

#[test]
fn events_response_seqs_field_is_additive() {
    // A response WITHOUT `seqs` (an older serve) decodes to an empty vec.
    let legacy = serde_json::json!({
        "events": ["deadbeef"], "attestations": [null], "attester_keys": [null]
    });
    let r: EventsResponse = serde_json::from_slice(&serde_json::to_vec(&legacy).unwrap()).unwrap();
    assert!(r.seqs.is_empty(), "missing seqs decodes to empty (serde default)");
    // A response WITH `seqs` round-trips.
    let with = EventsResponse {
        events: vec!["deadbeef".into()],
        attestations: vec![None],
        attester_keys: vec![None],
        seqs: vec![7],
        signing_context: None,
    };
    let back: EventsResponse =
        serde_json::from_slice(&serde_json::to_vec(&with).unwrap()).unwrap();
    assert_eq!(back.seqs, vec![7]);
}
```

Add `#[derive(Debug)]` to `Request` if not present (the round-trip test's `panic!("{other:?}")` needs it — check first; add only if missing).

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p cairn-sync events_after_seq_request_round_trips events_response_seqs_field_is_additive`
Expected: FAIL to compile — `EventsAfterSeq` variant and `seqs` field don't exist.

- [ ] **Step 3: Add the wire variant + field**

`Request` (~line 219), add after `EventsAfter`:
```rust
    /// Clinical plane, seq-cursored (issue #196): every event whose serving-node
    /// `seq` is strictly greater than `after_seq`, in `seq` order. `after_seq = 0`
    /// returns the full set (the full-sweep path). `seq` is the server's LOCAL
    /// insertion order — the only ordering where newly-learned events always sort
    /// above a puller's cursor, so incremental can never silently skip (#196).
    /// Additive (principle 12): the older `EventsAfter` variant stays served.
    EventsAfterSeq { after_seq: i64 },
```

`EventsResponse` (~line 246), add after `attester_keys`:
```rust
    /// Per-event serving-node `seq` (issue #196), PARALLEL to `events`. The puller
    /// checkpoints its per-peer cursor on the max handled seq. Additive (serde
    /// default): an older peer's response decodes with an empty vec — a new puller
    /// that sent EventsAfterSeq treats an events-without-seqs response as a
    /// wire-format error rather than checkpointing blindly (see do_pull).
    #[serde(default)]
    seqs: Vec<i64>,
```

- [ ] **Step 4: Add the serve arm** (`serve_conn`, ~line 1987, add a match arm beside `EventsAfter`)

```rust
        Request::EventsAfterSeq { after_seq } => {
            // Serve LOCAL insertion order (seq), strictly above the puller's
            // cursor. The seq prefix is transport metadata; signed_bytes are the
            // untouched signed core (principle 12). `after_seq = 0` = full sweep.
            let rows = client.query(
                "SELECT seq, encode(signed_bytes,'hex'), encode(attestation,'hex'),
                        encode(attester_key,'hex')
                 FROM event_log
                 WHERE seq > $1
                 ORDER BY seq",
                &[&after_seq],
            )?;
            let seqs = rows.iter().map(|r| r.get::<_, i64>(0)).collect();
            let events = rows.iter().map(|r| r.get::<_, String>(1)).collect();
            let attestations = rows.iter().map(|r| r.get::<_, Option<String>>(2)).collect();
            let attester_keys = rows.iter().map(|r| r.get::<_, Option<String>>(3)).collect();
            serde_json::to_vec(&EventsResponse {
                events,
                attestations,
                attester_keys,
                seqs,
                signing_context: Some(CTX_EVENT.as_str().to_string()),
            })?
        }
```

Also add `seqs: <...>` to the existing `EventsAfter` arm's `EventsResponse { .. }` literal so it still compiles. For that legacy arm, ship an empty `seqs: vec![]` (an old-style puller ignores it; a new puller never sends `EventsAfter`). Add a one-line comment saying so.

- [ ] **Step 5: Fix the `response_json` test helper** (~line 2439) — it constructs `EventsResponse` and will no longer compile without `seqs`. Auto-assign ascending seqs (the canned serve simulates a real serve returning events in seq order):

```rust
        serde_json::to_vec(&EventsResponse {
            events: events.iter().map(hex::encode).collect(),
            attestations: vec![None; events.len()],
            attester_keys: vec![None; events.len()],
            // Canned serve = events in seq order; assign 1-based seqs so the
            // puller has a per-event cursor to checkpoint/pen on (issue #196).
            seqs: (1..=events.len() as i64).collect(),
            signing_context: signing_context.map(str::to_string),
        })
```

- [ ] **Step 6: Extend `events_response_decodes_pre_attestation_wire_format`** (~line 2331) — add one line asserting `.seqs` is empty on the pre-existing legacy fixture (confirms the field is additive on the real legacy wire shape, not just a hand-built JSON).

- [ ] **Step 7: Run tests to verify they pass**

Run: `cargo test -p cairn-sync events_after_seq events_response_seqs events_response_decodes_pre_attestation`
Expected: PASS. `cargo build -p cairn-sync` clean.

- [ ] **Step 8: Commit**

```bash
git add crates/cairn-sync/src/main.rs
git commit -m "feat(#196): additive EventsAfterSeq request + seqs[] response + serve arm"
```

---

### Task 3: `do_pull` seq-cursor rewrite (the core)

Rewrites the puller to cursor on `seq`, derive the floor from `min(refused_seq)`, and freeze at the contiguous handled prefix; threads a `full_sweep` param through `cmd_run` (cadence) and `cmd_pull` (`--full`); migrates every in-file test keyed on the old HLC watermark/floor. Suite stays green.

**Files:**
- Modify: `crates/cairn-sync/src/main.rs` — `quarantine_event` (~line 1025), `do_pull` (~line 1107), `cmd_pull` (~line 1604), `cmd_run` (~line 1931), `main` arg parsing for `cmd_pull --full` (~line 2100), a new `FULL_SWEEP_EVERY` const (near line 98), the test helpers `watermark`/`floor` (~line 2471/2482) + the ~11 canned-serve tests that assert on them
- Test: same file; drive with the migrated canned-serve tests

**Interfaces:**
- Consumes: db/036 columns (Task 1); `Request::EventsAfterSeq` + `EventsResponse.seqs` (Task 2).
- Produces: `do_pull(client, peer, peer_name, full_sweep: bool)` — consumed by `cmd_pull`, `cmd_run`, and the Task 4/5 acceptance tests.

- [ ] **Step 1: Add the sweep cadence const** (near line 98)

```rust
/// Full-sweep cadence (issue #196, mirroring cairn-node's FULL_SWEEP_EVERY): the
/// clinical pull does an incremental seq-cursor pull each cycle and a full sweep
/// (after_seq = 0) every FULL_SWEEP_EVERY cycles. The sweep is the correctness
/// floor: it reconciles any event a residual hazard (BIGSERIAL out-of-order
/// commit) caused incremental to skip. Incremental = optimization; sweep = floor.
const FULL_SWEEP_EVERY: u64 = 10;
```

- [ ] **Step 2: Extend `quarantine_event` with `refused_seq`** (~line 1025)

Add a `refused_seq: i64` parameter (after `attester_key`, before `reason`), and put it in the INSERT column list only (the dedupe UPDATE leaves it untouched as forensics — mirrors node plane):

```rust
fn quarantine_event(
    client: &mut postgres::Client,
    peer_name: &str,
    signed_bytes: &[u8],
    attestation: Option<&[u8]>,
    attester_key: Option<&[u8]>,
    refused_seq: i64,
    reason: &str,
) -> R<bool> {
```
The dedupe `UPDATE ... WHERE content_digest = $1 RETURNING acked` is unchanged. The insert becomes:
```rust
            "INSERT INTO sync_quarantine
             (content_digest, signed_bytes, attestation, attester_key, peer, refused_seq, reason)
         SELECT $1,$2,$3,$4,$5,$6,$7
          WHERE (SELECT count(*) FROM sync_quarantine WHERE peer = $5) < $8
            AND (SELECT COALESCE(sum(octet_length(signed_bytes)),0)
                   FROM sync_quarantine WHERE peer = $5) + octet_length($2::bytea) <= $9
         ON CONFLICT (content_digest) DO NOTHING",
            &[
                &digest, &signed_bytes, &attestation, &attester_key, &peer_name,
                &refused_seq, &reason,
                &MAX_QUARANTINE_ROWS_PER_PEER, &MAX_QUARANTINE_BYTES_PER_PEER,
            ],
```

- [ ] **Step 3: Rewrite `do_pull`** (~line 1107) — replace the whole function body. Full replacement:

```rust
/// Pull from `peer` on the clinical plane, seq-cursored (issue #196). Reads the
/// per-peer seq cursor (`sync_state.last_seq`) and the derived re-offer floor
/// (min unacked `refused_seq`), requests `seq > after_seq` (or `> 0` on a full
/// sweep), applies each event through the in-DB door, and checkpoints the max
/// CONTIGUOUS handled seq (advance-only). A freeze (valid-but-unappliable event,
/// or a pen refusal) stops the advance below that seq — retried next cycle, never
/// skipped. Mirrors cairn-node's pull_into adapted to the clinical JSON batch.
fn do_pull(
    client: &mut postgres::Client,
    peer: &str,
    peer_name: &str,
    full_sweep: bool,
) -> R<serde_json::Value> {
    client.execute(
        "INSERT INTO sync_state (peer) VALUES ($1) ON CONFLICT (peer) DO NOTHING",
        &[&peer_name],
    )?;
    // The committed seq cursor (0 = never pulled). Node-LOCAL insertion order of
    // the SERVING node (db/036 event_log.seq), NOT the HLC — a newly-learned
    // low-HLC event still sorts ABOVE the cursor, so it can never be silently
    // skipped (the #196 fix; the old HLC watermark could and did skip it).
    let last_seq: i64 = client
        .query_one("SELECT last_seq FROM sync_state WHERE peer=$1", &[&peer_name])?
        .get(0);
    // The re-offer floor, now seq-keyed: the lowest serving seq at which this peer
    // still has an UNACKED quarantined event. Fetching from just BELOW it keeps a
    // penned slot re-offered every cycle (deduping onto its row) even as the
    // cursor advances for valid events. NULL = no unresolved pen. This DERIVES the
    // floor from the rows — no persisted floor column (a net simplification over
    // the old quarantine_floor_wall/counter + pin logic).
    let floor: Option<i64> = client
        .query_one(
            "SELECT min(refused_seq) FROM sync_quarantine WHERE peer=$1 AND NOT acked",
            &[&peer_name],
        )?
        .get(0);
    // Fetch point: a full sweep pulls everything (after_seq = 0, the correctness
    // floor for the BIGSERIAL out-of-order-commit residual); otherwise from the
    // cursor, pulled back to just below the earliest refused slot when a floor is
    // set. The `-1` is load-bearing: serve streams `seq > after_seq` (STRICT), so
    // fetching from refused_seq itself would skip the very slot we re-offer.
    let after_seq: i64 = if full_sweep {
        0
    } else {
        floor.map_or(last_seq, |f| (f - 1).min(last_seq))
    };

    let started = Instant::now();
    let raw = request(peer, &Request::EventsAfterSeq { after_seq })?;
    let wire_bytes = raw.len();
    let resp: EventsResponse = serde_json::from_slice(&raw)?;

    // Deterministic wire-format skew check (issue #108) — unchanged.
    if let Some(peer_ctx) = &resp.signing_context {
        if peer_ctx != CTX_EVENT.as_str() {
            return Err(Box::new(PullIntegrityError {
                message: format!(
                    "pull {peer_name}: peer declares signing context '{peer_ctx}' but this \
                     node expects '{}' — wire-format skew, not tampering; upgrade the older \
                     side. Batch refused, cursor untouched.",
                    CTX_EVENT.as_str()
                ),
                metrics: serde_json::Value::Null,
            }));
        }
    }

    // The seq is load-bearing for the cursor: a response carrying events but a
    // short/empty seqs array is a malformed or unexpectedly-old serve — fail
    // LOUDLY rather than checkpoint blind.
    if !resp.events.is_empty() && resp.seqs.len() != resp.events.len() {
        return Err(format!(
            "pull {peer_name}: peer returned {} events but {} seqs — cannot checkpoint the seq \
             cursor safely; the peer serves an incompatible/older wire format",
            resp.events.len(),
            resp.seqs.len()
        )
        .into());
    }

    let (mut applied, mut skipped_unverifiable, mut skipped_acked, mut event_bytes) =
        (0usize, 0usize, 0usize, 0usize);
    // Highest CONTIGUOUS handled seq; a freeze stops the advance below its seq.
    let mut max_seq = after_seq;
    let mut frozen = false;
    let mut pen_refused: Option<String> = None;

    for (i, hexed) in resp.events.iter().enumerate() {
        let seq = resp.seqs[i];
        // Decode entry + its PARALLEL attestation pair (unchanged from the HLC
        // version): a non-hex entry is penned like any other unverifiable frame.
        let decoded: Result<WireEntry, String> = hex::decode(hexed)
            .map_err(|e| format!("event entry is not valid hex: {e}"))
            .and_then(|signed| {
                let att = resp.attestations.get(i).and_then(|o| o.as_deref())
                    .map(hex::decode).transpose()
                    .map_err(|e| format!("attestation entry is not valid hex: {e}"))?;
                let akey = resp.attester_keys.get(i).and_then(|o| o.as_deref())
                    .map(hex::decode).transpose()
                    .map_err(|e| format!("attester-key entry is not valid hex: {e}"))?;
                Ok((signed, att, akey))
            });

        let refused: Option<(WireEntry, String)> = match decoded {
            Err(reason) => Some(((hexed.as_bytes().to_vec(), None, None), reason)),
            Ok((signed_bytes, att, akey)) => {
                event_bytes += signed_bytes.len();
                match apply_signed(client, &signed_bytes, att.as_deref(), akey.as_deref()) {
                    Ok(new) => {
                        if new {
                            applied += 1;
                        }
                        // Auto-release a stale pen row for these bytes once they
                        // apply (only worth a DELETE when a pen exists).
                        if floor.is_some() {
                            let digest = cairn_event::event_address(&signed_bytes);
                            client.execute(
                                "DELETE FROM sync_quarantine WHERE content_digest = $1",
                                &[&digest],
                            )?;
                        }
                        None
                    }
                    Err(e) => match verify_self_described(&signed_bytes) {
                        // Verifiable but failed to apply: FREEZE (retry, never skip).
                        Ok(_) => {
                            frozen = true;
                            eprintln!(
                                "pull {peer_name}: HALTING seq cursor at {max_seq} — a valid \
                                 event failed to apply and must not be skipped: {e}"
                            );
                            None
                        }
                        // Unverifiable: pen it.
                        Err(verr) => {
                            Some(((signed_bytes, att, akey), format!("{verr}; apply door said: {e}")))
                        }
                    },
                }
            }
        };

        if let Some(((bytes, att, akey), reason)) = refused {
            match quarantine_event(
                client, peer_name, &bytes, att.as_deref(), akey.as_deref(), seq, &reason,
            ) {
                Ok(true) => skipped_acked += 1,
                Ok(false) => {
                    skipped_unverifiable += 1;
                    eprintln!(
                        "pull {peer_name}: unverifiable event quarantined durably \
                         (sync_quarantine), slot held on the re-offer floor at seq {seq}: {reason}"
                    );
                }
                Err(qe) => {
                    frozen = true;
                    eprintln!(
                        "pull {peer_name}: HALTING seq cursor at {max_seq} — an unverifiable \
                         event could not be quarantined, so it must not be skipped: {qe}; \
                         reason: {reason}"
                    );
                    pen_refused.get_or_insert(qe.to_string());
                }
            }
        }

        // Advance over the contiguous HANDLED prefix; a freeze stops the advance.
        // Relies on serve's `ORDER BY seq` (STRICT ascending) — see the serve arm.
        if !frozen && seq > max_seq {
            max_seq = seq;
        }
    }

    // Checkpoint advance-only (GREATEST guards a buggy rewind). sync_state is
    // node-local operational state, so a raw UPDATE is correct here (unlike the
    // node plane's SECURITY DEFINER door, whose role does zero raw DML).
    if max_seq > after_seq || full_sweep {
        client.execute(
            "UPDATE sync_state
                SET last_seq = GREATEST(last_seq, $2), last_pull_at = clock_timestamp()
              WHERE peer = $1",
            &[&peer_name, &max_seq],
        )?;
    }

    let pending: i64 = client
        .query_one(
            "SELECT count(*) FROM sync_quarantine WHERE peer=$1 AND NOT acked",
            &[&peer_name],
        )?
        .get(0);
    let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
    let metrics = serde_json::json!({
        "op": "pull", "peer": peer_name,
        "shipped": resp.events.len(), "applied_new": applied,
        "skipped_unverifiable": skipped_unverifiable,
        "skipped_acked": skipped_acked,
        "watermark_frozen": frozen,
        "floor_active": pending > 0,
        "event_bytes": event_bytes, "wire_bytes": wire_bytes,
        "bytes_per_event": if resp.events.is_empty() { 0.0 }
                           else { event_bytes as f64 / resp.events.len() as f64 },
        "elapsed_ms": elapsed_ms,
        "cursor_seq": max_seq, "full_sweep": full_sweep
    });

    // LOUD failure (issue #108, generalised by the #110 review): ANY unacked
    // refusal fails the pull, every cycle, until the peer is fixed or a human acks.
    if skipped_unverifiable > 0 || pen_refused.is_some() {
        let all = !resp.events.is_empty() && skipped_unverifiable == resp.events.len();
        let diagnosis = if all {
            let declared = match &resp.signing_context {
                Some(ctx) => format!("declares signing context '{ctx}'"),
                None => "declares no signing context (a pre-ADR-0040 build would not)".to_string(),
            };
            format!(
                " ALL {} shipped events are unverifiable and the peer {declared} — it appears to \
                 serve pre-ADR-0040 (or corrupt) signatures; re-initialize/re-sign the peer, or \
                 if THIS node was at fault run `cairn-sync requeue` after fixing it.",
                resp.events.len()
            )
        } else {
            String::new()
        };
        let pen = match &pen_refused {
            Some(qe) => format!(" Quarantine pen refused (cursor frozen): {qe}"),
            None => String::new(),
        };
        return Err(Box::new(PullIntegrityError {
            message: format!(
                "pull {peer_name}: {skipped_unverifiable} unverifiable event(s) this cycle; each \
                 is preserved verbatim in sync_quarantine and its slot is held on the re-offer \
                 floor (nothing lost; valid events still applied).{diagnosis}{pen} Inspect with \
                 `cairn-sync quarantine`; a repaired/re-signed peer is picked up automatically; \
                 to accept a permanent exclusion, ack the row: \
                 UPDATE sync_quarantine SET acked = TRUE WHERE content_digest = …"
            ),
            metrics,
        }));
    }
    Ok(metrics)
}
```

Note: this removes all reads/writes of `hlc_wall/hlc_counter/quarantine_floor_*`. Before committing, grep for any remaining consumer of the removed metrics keys `watermark_wall`/`watermark_counter`: `grep -rn "watermark_wall\|watermark_counter" crates poc` — if a live consumer exists (not a frozen `poc/`), keep emitting them; otherwise the rename to `cursor_seq` stands.

- [ ] **Step 4: Thread `full_sweep` through `cmd_pull` and `cmd_run`**

`cmd_pull` (~line 1604): add a `full: bool` param; call `do_pull(&mut client, peer, peer_name, full)`. In `main`'s `pull` arm (~line 2100), parse a `--full` flag: `let full = args.iter().any(|a| a == "--full");` and pass it. Add `--full` to `usage()`.

`cmd_run` (~line 1931): compute the cadence and pass it:
```rust
        // Full sweep on cadence (issue #196): incremental each cycle, a full
        // sweep every FULL_SWEEP_EVERY cycles as the correctness floor.
        #[allow(clippy::manual_is_multiple_of)]
        let full_sweep = cycle % FULL_SWEEP_EVERY == 0;
        match do_pull(&mut client, peer, peer_name, full_sweep) {
```

- [ ] **Step 5: Migrate the test helpers** (~line 2471) — replace `watermark`/`floor`:

```rust
    /// The per-peer seq cursor (0 = never pulled).
    fn cursor(c: &mut postgres::Client, peer: &str) -> i64 {
        c.query_one("SELECT last_seq FROM sync_state WHERE peer=$1", &[&peer])
            .unwrap()
            .get(0)
    }

    /// The per-peer re-offer floor: min unacked refused_seq (None = no unresolved pen).
    fn floor(c: &mut postgres::Client, peer: &str) -> Option<i64> {
        c.query_one(
            "SELECT min(refused_seq) FROM sync_quarantine WHERE peer=$1 AND NOT acked",
            &[&peer],
        )
        .unwrap()
        .get(0)
    }
```
Also update `pull_integrity_err` and any other helper to call `do_pull(c, addr, peer, false)` (incremental) — every existing in-file `do_pull(...)` call site gains a `false` arg.

- [ ] **Step 6: Migrate the ~11 canned-serve tests.** Each currently asserts on `watermark(...)`/`floor(...)` HLC tuples; switch to the seq `cursor(...)`/`floor(...)`. The canned serve auto-assigns seqs `1..=n` (Task 2 Step 5), so for a batch `[e1, garbage, e2]` the garbage sits at seq 2 and the last event at seq 3. Apply mechanically per test:

  - `pull_pens_unverifiable_pins_floor_and_recovers_when_peer_repaired`: `watermark == (WALL+2000,0)` → `cursor(&mut c,"peer-a") == 3`; `floor == Some((WALL+1000,0))` → `floor(&mut c,"peer-a") == Some(2)`. The repaired-cycle assertions (floor clears, cursor stays) map the same way.
  - `acked_row_releases_floor_and_pull_succeeds`: floor `None` after ack → `floor(..) == None` (unchanged shape, seq-typed).
  - `pen_quota_freezes_watermark_instead_of_growing`: the freeze assertion is `m["watermark_frozen"] == true` (metric key unchanged) + `cursor` stays below the frozen seq.
  - `floor_survives_pen_failure_on_reoffer_cycle`: floor stays `Some(<seq>)`.
  - `pull_fails_loud_when_every_event_is_unverifiable`, `pull_fails_loud_on_synced_link_with_unverifiable_tail`, `pull_pens_non_hex_entry_instead_of_wedging`, `pen_byte_quota_refuses_overshooting_frame`, `pull_refuses_declared_context_mismatch_deterministically`, `requeue_releases_quarantined_events_once_cause_is_fixed`: these assert on metrics (`applied_new`, `skipped_unverifiable`, loud message) and `quarantine_rows`, none of which change — only their `do_pull(...)` calls gain the `false` arg, and any incidental `watermark(...)` call becomes `cursor(...)`.

  For each: run it, read the failure, set the expected seq value from the event's position in the canned batch. Do NOT invent behavior — the structural assertions (loud failure, pen dedupe, applied counts) are unchanged; only the cursor/floor *values* move from HLC to seq.

- [ ] **Step 7: Write the RED driver for the fix** — a canned-serve test proving the puller checkpoints and re-offers on seq:

```rust
#[test]
fn pull_checkpoints_seq_cursor_and_reoffers_on_refused_seq() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let mut c = locked_client(&base);
    let (sk, kid) = enrolled_key(&mut c);
    let e1 = peer_note(&sk, &kid, WALL_2026 + 1_000);
    let garbage = b"not a COSE_Sign1".to_vec();
    let e3 = peer_note(&sk, &kid, WALL_2026 + 2_000);
    let raw = response_json(&[&e1, &garbage, &e3], Some(CTX_EVENT.as_str()));
    let addr = serve_canned(raw, 1);
    let (_msg, m) = pull_integrity_err(&mut c, &addr, "peer-a");
    // e1(seq1) and e3(seq3) applied; garbage(seq2) penned with refused_seq=2.
    assert_eq!(m["applied_new"], 2);
    assert_eq!(cursor(&mut c, "peer-a"), 3, "cursor checkpoints the max handled seq");
    assert_eq!(floor(&mut c, "peer-a"), Some(2), "floor = the refused seq");
}
```

- [ ] **Step 8: Run the full crate test suite**

Run: `cargo test -p cairn-sync` (with `CAIRN_TEST_PG`), then `cargo fmt --all -- --check` and `cargo clippy -p cairn-sync -- -D warnings`.
Expected: all green (new test PASS, all migrated tests PASS).

- [ ] **Step 9: Commit**

```bash
git add crates/cairn-sync/src/main.rs
git commit -m "feat(#196): do_pull cursors on event_log.seq + periodic full sweep"
```

---

### Task 4: Acceptance test — low-HLC event below the cursor still converges

The headline #196/#38 regression guard, in the real-binary A→B rig.

**Files:**
- Modify: `crates/cairn-sync/tests/clinical_pull.rs` — add imports, a `foreign_note` helper, port consts, the new test; migrate the existing freeze test's `hlc_wall` read to `last_seq`

**Interfaces:**
- Consumes: `reset`, `enroll_actors`, `wait_listening`, `ServeGuard`, `cs_a`/`cs_b` (all already in the file); `cairn_event::{EventBody, Hlc, sign, generate_key}` (public); `SELECT apply_remote_event($1)` (the same door `pull` uses, proven in `medication_remote_apply.rs`).

- [ ] **Step 1: Add imports + helper + port consts** (top of `clinical_pull.rs`)

```rust
use cairn_event::{sign, EventBody, Hlc, SigningKey};
// (generate_key + db + medication orchestrators are already imported.)

const LISTEN_LOWHLC: &str = "127.0.0.1:39719";
/// A realistic PAST HLC wall (ms since epoch, ≈ 2026-06-20) — safely below "now",
/// so A's remote-apply door accepts it (the drift ceiling bounds FUTURE walls only).
const WALL: i64 = 1_782_000_000_000;

/// A validly-signed `note.added` at a CHOSEN HLC wall, from a foreign signer — the
/// multi-hop event whose HLC can sit BELOW a node's advanced cursor. The HLC lives
/// inside the signed body (freely set by the signer); `seq` is assigned by the
/// RECEIVING node at insert. So applying this to A gives it a low HLC but a fresh
/// HIGH seq — exactly the #196 skip trigger. Returns (signed_bytes, event_id).
fn foreign_note(sk: &SigningKey, kid: &str, wall: i64, text: &str) -> (Vec<u8>, String) {
    let event_id = Uuid::now_v7().to_string();
    let body = EventBody {
        event_id: event_id.clone(),
        patient_id: Uuid::now_v7().to_string(),
        event_type: "note.added".into(),
        schema_version: "note/1".into(),
        hlc: Hlc { wall, counter: 0, node_origin: "node-c".into() },
        t_effective: None,
        signer_key_id: kid.into(),
        contributors: serde_json::json!([{"actor_id": kid, "role": "recorded"}]),
        payload: serde_json::json!({"text": text}),
        attachments: vec![],
        plaintext_twin: Some(format!("Progress note: {text}")),
    };
    (sign(&body, sk).unwrap().signed_bytes, event_id)
}
```

- [ ] **Step 2: Write the failing test**

```rust
/// Issue #196: an event that lands on A with an HLC BELOW B's advanced cursor — a
/// multi-hop arrival from a third node — must still converge to B. The old HLC
/// watermark skipped it forever (its HLC sorts below B's watermark, so B never
/// re-serves it); the seq cursor cannot, because the late arrival got a fresh high
/// seq on A regardless of its low HLC.
#[tokio::test]
async fn low_hlc_below_cursor_still_converges() {
    let (Some(base_a), Some(base_b)) = (cs_a(), cs_b()) else {
        eprintln!("skipped: set CAIRN_TEST_PG and CAIRN_TEST_PG2");
        return;
    };
    let _guard = db::test_serial_guard(&base_a).await.unwrap();
    let a = db::connect_and_load_schema(&base_a).await.unwrap();
    reset(&a).await;
    let b = db::connect_and_load_schema(&base_b).await.unwrap();
    reset(&b).await;

    // A foreign "node-c" signer, enrolled as a device on BOTH nodes so each door
    // admits its notes (actor enrollment does not travel the clinical plane — #205).
    let (sk_c, kid_c) = cairn_event::generate_key().unwrap();
    for c in [&a, &b] {
        c.execute(
            "SELECT enroll_actor('device', '{\"role\":\"node-c\"}', $1)",
            &[&kid_c],
        )
        .await
        .unwrap();
    }

    // Two HIGH-HLC events land on A (seqs 1, 2). Apply through A's own door so
    // they get A-local seqs.
    let (hi1, _) = foreign_note(&sk_c, &kid_c, WALL + 20_000, "high one");
    let (hi2, _) = foreign_note(&sk_c, &kid_c, WALL + 30_000, "high two");
    for e in [&hi1, &hi2] {
        a.execute("SELECT apply_remote_event($1)", &[e]).await.unwrap();
    }

    // Serve A; pull B once. B applies both and checkpoints last_seq(node-a) = 2.
    let bin = env!("CARGO_BIN_EXE_cairn-sync");
    let _serve = ServeGuard(
        Command::new(bin)
            .args(["serve", "--conn", &base_a, "--listen", LISTEN_LOWHLC])
            .spawn()
            .expect("spawn serve"),
    );
    wait_listening(LISTEN_LOWHLC);
    let pull1 = Command::new(bin)
        .args(["pull", "--conn", &base_b, "--peer", LISTEN_LOWHLC, "--peer-name", "node-a"])
        .output()
        .expect("run pull 1");
    assert!(pull1.status.success(), "pull 1: {}", String::from_utf8_lossy(&pull1.stderr));
    let cursor: i64 = b
        .query_one("SELECT last_seq FROM sync_state WHERE peer='node-a'", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(cursor, 2, "B checkpointed the seq cursor at 2");

    // The multi-hop event: a LOW HLC (below both pulled events) lands on A LATE,
    // getting a fresh seq (3). This is the exact event the HLC watermark skipped.
    let (low, low_id) = foreign_note(&sk_c, &kid_c, WALL + 10_000, "late low-HLC arrival");
    a.execute("SELECT apply_remote_event($1)", &[&low]).await.unwrap();
    let low_uuid = Uuid::parse_str(&low_id).unwrap();

    // Pull B again (incremental). On the OLD HLC code B fetches hlc >= watermark, so
    // the low-HLC event is never served and this count stays 0. On the seq cursor B
    // fetches seq > 2 → seq 3 → the event applies.
    let pull2 = Command::new(bin)
        .args(["pull", "--conn", &base_b, "--peer", LISTEN_LOWHLC, "--peer-name", "node-a"])
        .output()
        .expect("run pull 2");
    assert!(pull2.status.success(), "pull 2: {}", String::from_utf8_lossy(&pull2.stderr));
    let present: i64 = b
        .query_one("SELECT count(*) FROM event_log WHERE event_id = $1", &[&low_uuid])
        .await
        .unwrap()
        .get(0);
    assert_eq!(present, 1, "the low-HLC multi-hop event converged to B (issue #196)");
}
```

- [ ] **Step 3: Run**

Run: `cargo test -p cairn-sync --test clinical_pull low_hlc_below_cursor_still_converges` (with `CAIRN_TEST_PG` + `CAIRN_TEST_PG2`).
Expected: PASS on the Task-3 seq cursor. (Sanity check of the regression property: this same test on the pre-#196 `do_pull` would leave `present == 0` — the skip.)

- [ ] **Step 4: Migrate the freeze test** (`a_to_b_pull_freezes_...`, ~line 474) — change the `SELECT ... hlc_wall, hlc_counter FROM sync_state` read to `last_seq`, rename the tuple to e.g. `(applied, penned, cursor)`, and assert the cursor stays **0** while B has not enrolled A's author (frozen), then advances after enrol + re-pull. Keep every other assertion.

- [ ] **Step 5: Commit**

```bash
git add crates/cairn-sync/tests/clinical_pull.rs
git commit -m "test(#196): low-HLC-below-cursor event still converges (A→B, real binary)"
```

---

### Task 5: Acceptance tests — full-sweep reconciles, idempotent re-pull

**Files:**
- Modify: `crates/cairn-sync/tests/clinical_pull.rs` — two tests + two port consts (`LISTEN_SWEEP`, `LISTEN_REPULL`)

- [ ] **Step 1: Write `full_sweep_reconciles_a_skipped_seq`**

```rust
const LISTEN_SWEEP: &str = "127.0.0.1:39720";

/// Issue #196: the full sweep is the correctness floor for the residual BIGSERIAL
/// out-of-order-commit skip. Simulate the skip by forcing B's cursor PAST an event's
/// seq; an incremental pull then cannot fetch it (`seq > cursor` excludes it), and a
/// `--full` sweep (seq > 0) reconciles it.
#[tokio::test]
async fn full_sweep_reconciles_a_skipped_seq() {
    let (Some(base_a), Some(base_b)) = (cs_a(), cs_b()) else {
        eprintln!("skipped: set CAIRN_TEST_PG and CAIRN_TEST_PG2");
        return;
    };
    let _guard = db::test_serial_guard(&base_a).await.unwrap();
    let a = db::connect_and_load_schema(&base_a).await.unwrap();
    reset(&a).await;
    let b = db::connect_and_load_schema(&base_b).await.unwrap();
    reset(&b).await;
    let (sk_c, kid_c) = cairn_event::generate_key().unwrap();
    for c in [&a, &b] {
        c.execute("SELECT enroll_actor('device', '{\"role\":\"node-c\"}', $1)", &[&kid_c])
            .await.unwrap();
    }
    // One event on A (seq 1), then a second (seq 2, the "skipped" one).
    let (e1, _) = foreign_note(&sk_c, &kid_c, WALL + 10_000, "one");
    a.execute("SELECT apply_remote_event($1)", &[&e1]).await.unwrap();
    let (e2, id2) = foreign_note(&sk_c, &kid_c, WALL + 20_000, "two");
    a.execute("SELECT apply_remote_event($1)", &[&e2]).await.unwrap();
    let id2 = Uuid::parse_str(&id2).unwrap();

    // Force B's cursor to 2 WITHOUT applying e2 (the commit-race skip).
    b.execute(
        "INSERT INTO sync_state (peer, last_seq) VALUES ('node-a', 2)
         ON CONFLICT (peer) DO UPDATE SET last_seq = 2",
        &[],
    ).await.unwrap();

    let bin = env!("CARGO_BIN_EXE_cairn-sync");
    let _serve = ServeGuard(
        Command::new(bin).args(["serve", "--conn", &base_a, "--listen", LISTEN_SWEEP])
            .spawn().expect("spawn serve"),
    );
    wait_listening(LISTEN_SWEEP);

    // Incremental pull: seq > 2 fetches nothing → e2 stays missing.
    let inc = Command::new(bin)
        .args(["pull", "--conn", &base_b, "--peer", LISTEN_SWEEP, "--peer-name", "node-a"])
        .output().expect("incremental pull");
    assert!(inc.status.success(), "inc pull: {}", String::from_utf8_lossy(&inc.stderr));
    let before: i64 = b.query_one("SELECT count(*) FROM event_log WHERE event_id=$1", &[&id2])
        .await.unwrap().get(0);
    assert_eq!(before, 0, "incremental cannot reach a seq below the forced cursor");

    // Full sweep: seq > 0 fetches everything → e2 reconciled.
    let full = Command::new(bin)
        .args(["pull", "--full", "--conn", &base_b, "--peer", LISTEN_SWEEP, "--peer-name", "node-a"])
        .output().expect("full pull");
    assert!(full.status.success(), "full pull: {}", String::from_utf8_lossy(&full.stderr));
    let after: i64 = b.query_one("SELECT count(*) FROM event_log WHERE event_id=$1", &[&id2])
        .await.unwrap().get(0);
    assert_eq!(after, 1, "the full sweep is the correctness floor (issue #196)");
}
```

- [ ] **Step 2: Write `repull_from_zero_converges`**

```rust
const LISTEN_REPULL: &str = "127.0.0.1:39721";

/// ADR-0004 "the watermark is a hint": a full sweep from seq 0 re-applies the whole
/// log as set-union no-ops and reaches an identical read-state (idempotent).
#[tokio::test]
async fn repull_from_zero_converges() {
    let (Some(base_a), Some(base_b)) = (cs_a(), cs_b()) else {
        eprintln!("skipped: set CAIRN_TEST_PG and CAIRN_TEST_PG2");
        return;
    };
    let _guard = db::test_serial_guard(&base_a).await.unwrap();
    let a = db::connect_and_load_schema(&base_a).await.unwrap();
    reset(&a).await;
    let b = db::connect_and_load_schema(&base_b).await.unwrap();
    reset(&b).await;
    let (sk_c, kid_c) = cairn_event::generate_key().unwrap();
    for c in [&a, &b] {
        c.execute("SELECT enroll_actor('device', '{\"role\":\"node-c\"}', $1)", &[&kid_c])
            .await.unwrap();
    }
    for i in 0..3 {
        let (e, _) = foreign_note(&sk_c, &kid_c, WALL + 10_000 * (i + 1), "n");
        a.execute("SELECT apply_remote_event($1)", &[&e]).await.unwrap();
    }
    let bin = env!("CARGO_BIN_EXE_cairn-sync");
    let _serve = ServeGuard(
        Command::new(bin).args(["serve", "--conn", &base_a, "--listen", LISTEN_REPULL])
            .spawn().expect("spawn serve"),
    );
    wait_listening(LISTEN_REPULL);
    let pull = Command::new(bin)
        .args(["pull", "--conn", &base_b, "--peer", LISTEN_REPULL, "--peer-name", "node-a"])
        .output().expect("pull");
    assert!(pull.status.success(), "pull: {}", String::from_utf8_lossy(&pull.stderr));
    let count1: i64 = b.query_one("SELECT count(*) FROM event_log", &[]).await.unwrap().get(0);

    // Rewind the cursor to 0 and full-sweep: must be a set-union no-op.
    b.execute("UPDATE sync_state SET last_seq = 0 WHERE peer='node-a'", &[]).await.unwrap();
    let full = Command::new(bin)
        .args(["pull", "--full", "--conn", &base_b, "--peer", LISTEN_REPULL, "--peer-name", "node-a"])
        .output().expect("full pull");
    assert!(full.status.success(), "full pull: {}", String::from_utf8_lossy(&full.stderr));
    let count2: i64 = b.query_one("SELECT count(*) FROM event_log", &[]).await.unwrap().get(0);
    assert_eq!(count1, count2, "re-pull from 0 is idempotent (ADR-0004)");
    assert!(count1 >= 3, "non-vacuous: the chart really replicated");
}
```

- [ ] **Step 3: Run both**

Run: `cargo test -p cairn-sync --test clinical_pull full_sweep_reconciles repull_from_zero`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add crates/cairn-sync/tests/clinical_pull.rs
git commit -m "test(#196): full-sweep reconciles a skipped seq + re-pull-from-zero idempotence"
```

---

### Task 6: Full-workspace green + HANDOVER/ROADMAP

**Files:**
- Modify: `docs/HANDOVER.md`, `docs/ROADMAP.md`

- [ ] **Step 1: Full workspace gate**

Run: `cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace` (with `CAIRN_TEST_PG` + `CAIRN_TEST_PG2` set).
Expected: 0 failed. If any node-plane test broke from the shared `db/036` / SCHEMA change, fix it here (should be none — db/036 is additive and cairn-node doesn't pull clinically).

- [ ] **Step 2: Update HANDOVER** — mark `#196 (B1)` DONE in the P2 list; move the "⇐ next in P2" marker to `#197 (B2)`; add a condensed session block (branch `fix/clinical-sync-seq-cursor-196`, the seq cursor + sweep, the acceptance tests, the vestigial-columns-kept decision). Convert any relative dates to absolute (2026-07-16). Prune to keep it concise.

- [ ] **Step 3: Update ROADMAP** — add a "Slice 38" (or the next number) entry recording the clinical seq-cursor slice under P2.

- [ ] **Step 4: Commit**

```bash
git add docs/HANDOVER.md docs/ROADMAP.md
git commit -m "docs(handover,roadmap): record the #196 clinical seq-cursor slice"
```

---

## Notes for the implementer

- **The two planes stay separate.** Do not try to share code with `cairn-node/src/sync.rs`; the transports differ (async framed streaming vs. sync JSON batch). The parallelism is deliberate.
- **Freeze relies on `ORDER BY seq`.** The puller's contiguous-prefix checkpoint is only correct because the serve streams strictly ascending seq. Never reorder the serve query.
- **Do not drop the vestigial HLC columns.** `sync_state.hlc_wall/hlc_counter`, `quarantine_floor_wall/counter` stay (ADR-0012 additive-only; an older binary still reads them). They are marked deprecated-in-place in db/036's comments.
- **`refused_seq` is set on INSERT only** — the dedupe UPDATE leaves it as forensics (the floor is `min` over rows, so the lowest wins regardless).
- **`connect_and_load_schema` re-runs every migration each connect** — every DB statement in db/036 is idempotent (`ADD COLUMN IF NOT EXISTS`, `CREATE INDEX IF NOT EXISTS`), so replay is safe.
```
