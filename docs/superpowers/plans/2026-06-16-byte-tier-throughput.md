# Byte-Tier Throughput Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the stub blob fetcher in the walking skeleton with a resumable, windowed, multi-source (swarm), per-chunk-verified byte tier that closes Spike 0001 §8.2.

**Architecture:** BLAKE3 verified streaming via the `bao` crate. A blob's bytes are fetched as fixed-size **slices** that each self-verify against the content address (`blob_address`), so chunks can arrive out of order, from any source, and be persisted incrementally in a `blob_chunk` table. A bounded pool of worker threads pulls slice indexes off a shared queue, round-robins across a swarm peer list, and persists verified slices; when all are present the blob is assembled, whole-blob-verified, and flipped to `present`. The byte tier keeps its own thread + preemptible budget so it never starves clinical sync (the #9 availability floor).

**Tech Stack:** Rust (`cairn-event`, `cairn-sync`), `bao` 0.13 + `blake3` 1, PostgreSQL (`db/003_blobs.sql`), Python stdlib harness.

---

## Design references

- Spec: [docs/superpowers/specs/2026-06-16-byte-tier-throughput-design.md](../specs/2026-06-16-byte-tier-throughput-design.md)
- Spike §8.2 / §4.4: [docs/spikes/0001-walking-skeleton-wan-sync-and-pi-cost.md](../../spikes/0001-walking-skeleton-wan-sync-and-pi-cost.md)

**Verified facts (probed against bao 0.13.1, blake3 1.8.5):**
- `bao::encode::outboard(bytes: &[u8]) -> (Vec<u8>, blake3::Hash)` — the returned hash equals `blake3::hash(bytes)`.
- `bao::encode::SliceExtractor::new_outboard(Cursor<Vec<u8>>, Cursor<Vec<u8>>, start: u64, len: u64)` implements `Read`; reading to end yields the slice bytes.
- `bao::decode::SliceDecoder::new(Cursor<Vec<u8>>, &blake3::Hash, start: u64, len: u64)` implements `Read`; reading verifies and yields the content bytes, erroring on any mismatch (tampered slice, wrong offset, or wrong root all fail).
- License: `CC0-1.0 OR Apache-2.0` (AGPL-3.0-compatible).

**Conventions:** run `cargo` commands from `poc/walking-skeleton/`. The release binary is `poc/walking-skeleton/target/release/cairn-sync`. `cargo test` and `cargo clippy --all-targets -- -D warnings` must stay green (the project bar).

---

## File structure

- `crates/cairn-event/Cargo.toml` — add `bao = "0.13"`.
- `crates/cairn-event/src/lib.rs` — add `blob_outboard`, `extract_slice`, `verify_slice` + the root-hash helper, with unit tests. Safety-critical core; stays small.
- `db/003_blobs.sql` — add `blob_store.outboard` column and the `blob_chunk` table.
- `crates/cairn-sync/src/main.rs` — `SLICE_BYTES`; `BlobSlice` request + `BlobSliceResponse`; serve-side slice extraction (+ test-only `--corrupt` fault injection); `do_blobd` rewrite (worker pool, swarm, resume, persist); `put-blob`/`gen-blob` store the outboard; CLI `--blob-peer` (repeatable), `--window`, `blobd --metrics`; `run`/`blobd` wiring.
- `harness/bench_blob.py` — new measurement + selftest harness (throughput, resume, swarm, lying-peer, availability floor).
- `poc/walking-skeleton/README.md` — document the byte tier and new commands.
- `docs/spikes/0001-...md` §8.2 — mark the throughput fix delivered (after the user's real-link run).

---

## Task 1: bao verified-streaming seam in `cairn-event` (TDD, pure)

**Files:**
- Modify: `crates/cairn-event/Cargo.toml`
- Modify: `crates/cairn-event/src/lib.rs`

- [ ] **Step 1: Add the dependency**

In `crates/cairn-event/Cargo.toml`, under `[dependencies]`, after the `blake3` line add:

```toml
bao = "0.13"              # BLAKE3 verified streaming: per-slice content verification (§4.4)
```

- [ ] **Step 2: Write the failing tests**

In `crates/cairn-event/src/lib.rs`, inside `mod tests { ... }` (after the existing `blob_address_is_blake3_multihash` test), add:

```rust
    #[test]
    fn outboard_root_equals_blob_address() {
        let data = vec![0x33u8; 700_000];
        let ob = blob_outboard(&data);
        // The bao root must equal the BLAKE3 root we content-address by.
        let addr = blob_address(&data);
        assert_eq!(blake3_root_from_address(&addr).unwrap().as_bytes(), &addr[2..]);
        // A slice extracted with this outboard verifies against the address root.
        let root = blake3_root_from_address(&addr).unwrap();
        let slice = extract_slice(&data, &ob, 0, data.len() as u64).unwrap();
        let got = verify_slice(&slice, &root, 0, data.len() as u64).unwrap();
        assert_eq!(got, data);
    }

    #[test]
    fn verify_slice_accepts_good_and_rejects_bad() {
        let data: Vec<u8> = (0..600_000u32).map(|i| (i % 251) as u8).collect();
        let ob = blob_outboard(&data);
        let addr = blob_address(&data);
        let root = blake3_root_from_address(&addr).unwrap();

        let (start, len) = (256u64 * 1024, 256u64 * 1024);
        let slice = extract_slice(&data, &ob, start, len).unwrap();

        // Good slice verifies and returns the right bytes.
        let got = verify_slice(&slice, &root, start, len).unwrap();
        assert_eq!(got, data[start as usize..(start + len) as usize]);

        // Tampered slice bytes -> reject.
        let mut bad = slice.clone();
        let m = bad.len() / 2;
        bad[m] ^= 0x01;
        assert!(verify_slice(&bad, &root, start, len).is_err());

        // Right slice, wrong claimed offset -> reject.
        assert!(verify_slice(&slice, &root, 0, len).is_err());

        // Right slice, wrong root -> reject.
        let other = blake3_root_from_address(&blob_address(b"different")).unwrap();
        assert!(verify_slice(&slice, &other, start, len).is_err());
    }
```

- [ ] **Step 3: Run the tests to verify they fail**

Run: `cargo test -p cairn-event outboard 2>&1 | tail -5` and `cargo test -p cairn-event verify_slice 2>&1 | tail -5`
Expected: FAIL — `cannot find function blob_outboard` / `extract_slice` / `verify_slice` / `blake3_root_from_address`.

- [ ] **Step 4: Implement the functions**

In `crates/cairn-event/src/lib.rs`, after the `blob_address` function (around line 125), add:

```rust
use std::io::{Cursor, Read};

/// Compute the BLAKE3 verified-streaming **outboard** tree for a blob's bytes.
/// Stored alongside the bytes on a node that holds them; needed only to *serve*
/// slices. The bao root of this encoding equals `blake3::hash(bytes)` — i.e. the
/// `blob_address` payload — so it binds to the existing content address (§4.4).
pub fn blob_outboard(bytes: &[u8]) -> Vec<u8> {
    let (outboard, hash) = bao::encode::outboard(bytes);
    debug_assert_eq!(hash.as_bytes(), &blob_address(bytes)[2..]);
    outboard
}

/// Recover the 32-byte BLAKE3 root from a multihash blob address (`0x1e 0x20` + 32).
pub fn blake3_root_from_address(addr: &[u8]) -> Result<blake3::Hash, EventError> {
    if addr.len() != 34 || addr[0..2] != BLAKE3_MULTIHASH_PREFIX {
        return Err(EventError::BadKeyId);
    }
    let bytes: [u8; 32] = addr[2..].try_into().map_err(|_| EventError::BadKeyId)?;
    Ok(blake3::Hash::from(bytes))
}

/// Server side: extract a verified bao slice covering `[start, start+len)` from a
/// blob's `content` and precomputed `outboard` tree. The returned bytes are the
/// verified-streaming slice (interleaved tree nodes + data) the client decodes.
pub fn extract_slice(
    content: &[u8],
    outboard: &[u8],
    start: u64,
    len: u64,
) -> Result<Vec<u8>, EventError> {
    let mut ex = bao::encode::SliceExtractor::new_outboard(
        Cursor::new(content.to_vec()),
        Cursor::new(outboard.to_vec()),
        start,
        len,
    );
    let mut out = Vec::new();
    ex.read_to_end(&mut out).map_err(|e| EventError::Cose(e.to_string()))?;
    Ok(out)
}

/// Client side — THE safety seam (§4.4): decode and verify a slice against the
/// known root, returning the verified content bytes. A tampered slice, a slice
/// claimed at the wrong offset, or verification against the wrong root all error,
/// so a lying source can never have its bytes accepted.
pub fn verify_slice(
    slice: &[u8],
    root: &blake3::Hash,
    start: u64,
    len: u64,
) -> Result<Vec<u8>, EventError> {
    let mut dec = bao::decode::SliceDecoder::new(Cursor::new(slice.to_vec()), root, start, len);
    let mut out = Vec::new();
    dec.read_to_end(&mut out).map_err(|_| EventError::BadSignature)?;
    Ok(out)
}
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p cairn-event 2>&1 | tail -8`
Expected: PASS — all tests including the two new ones; existing tests still green.

- [ ] **Step 6: Lint**

Run: `cargo clippy -p cairn-event --all-targets -- -D warnings 2>&1 | tail -5`
Expected: no warnings.

- [ ] **Step 7: Commit**

```bash
git add crates/cairn-event/Cargo.toml crates/cairn-event/src/lib.rs Cargo.lock
git commit -m "feat(byte-tier): bao verified-streaming seam in cairn-event"
```

---

## Task 2: Schema — outboard column + blob_chunk table

**Files:**
- Modify: `db/003_blobs.sql`

- [ ] **Step 1: Add the outboard column**

In `db/003_blobs.sql`, inside the `CREATE TABLE IF NOT EXISTS blob_store (...)` definition, after the `content BYTEA, ...` line and before the `present BOOLEAN ...` line, add:

```sql
    outboard     BYTEA,                  -- bao verified-streaming tree; set with content, serves slices
```

- [ ] **Step 2: Add the blob_chunk table**

In `db/003_blobs.sql`, after the `blob_store` table's closing `);` and before the `blob_note_reference` function, add:

```sql
-- Persistent partial-fetch state (§8.2 resumability). Each VERIFIED slice lands
-- here as it arrives — out of order, from any swarm source — keyed by its index
-- (offset / SLICE_BYTES). ON CONFLICT DO NOTHING makes chunk apply idempotent
-- set-union, exactly like the event plane. When every index for a blob is present
-- the byte tier assembles, whole-blob-verifies, fills blob_store, and deletes
-- these rows. A restart therefore resumes by fetching only the missing indexes.
CREATE TABLE IF NOT EXISTS blob_chunk (
    blob_address BYTEA       NOT NULL,
    chunk_index  INT         NOT NULL,
    content      BYTEA       NOT NULL,   -- verified bytes for this slice
    received_at  TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (blob_address, chunk_index)
);
```

- [ ] **Step 3: Verify the schema loads**

Run (uses a scratch DB; adjust `--conn` to a local PG you can write to):

```bash
psql "$PGCONN" -qc "drop table if exists event_log,hlc_state,sync_state,patient_chart,blob_store,blob_chunk cascade;"
cargo run -q -- init --conn "$PGCONN" 2>&1 | tail -4
psql "$PGCONN" -tAc "select column_name from information_schema.columns where table_name='blob_store' and column_name='outboard';"
psql "$PGCONN" -tAc "select to_regclass('blob_chunk');"
```

Expected: `init` applies `001/002/003`; the two `psql` queries print `outboard` and `blob_chunk`.

- [ ] **Step 4: Commit**

```bash
git add db/003_blobs.sql
git commit -m "feat(byte-tier): outboard column + blob_chunk partial-fetch table"
```

---

## Task 3: Wire protocol — BlobSlice + serve-side extraction + outboard at write

**Files:**
- Modify: `crates/cairn-sync/src/main.rs`

- [ ] **Step 1: Replace the chunk constant and protocol types**

In `crates/cairn-sync/src/main.rs`, change the chunk constant (line ~33):

```rust
const SLICE_BYTES: usize = 256 * 1024; // window/slice granularity (tuned; amortizes bao tree overhead)
```

In the `Request` enum, replace the `BlobChunk { addr_hex, offset, len }` variant with:

```rust
    /// Byte tier: a BLAKE3 verified-streaming slice of a blob.
    BlobSlice {
        addr_hex: String,
        offset: u64,
        len: u64,
    },
```

Replace the `BlobResponse` struct with:

```rust
#[derive(Serialize, Deserialize)]
struct BlobSliceResponse {
    found: bool,
    total_len: u64,
    /// hex-encoded bao slice (skeleton ships hex; the real tier ships raw bytes).
    slice_hex: String,
}
```

- [ ] **Step 2: Serve a verified slice (with test-only fault injection)**

In `serve_conn`, replace the entire `Request::BlobChunk { .. } => { ... }` match arm with:

```rust
        Request::BlobSlice {
            addr_hex,
            offset,
            len,
        } => {
            let addr = hex::decode(&addr_hex)?;
            let row = client.query_opt(
                "SELECT content, outboard, octet_length(content)
                 FROM blob_store WHERE blob_address=$1 AND present AND outboard IS NOT NULL",
                &[&addr],
            )?;
            let resp = match row {
                Some(r) => {
                    let content: Vec<u8> = r.get(0);
                    let outboard: Vec<u8> = r.get(1);
                    let total = r.get::<_, i32>(2) as u64;
                    // Clamp the final slice to the blob's end.
                    let len = len.min(total.saturating_sub(offset));
                    let mut slice = cairn_event::extract_slice(&content, &outboard, offset, len)?;
                    // TEST-ONLY fault injection: if started with --corrupt, flip a byte of
                    // every outgoing slice so the receiver's per-slice verify (§4.4) rejects
                    // it. This proves the swarm heals around a lying/faulty source; it is
                    // never enabled in a real node.
                    if corrupt && !slice.is_empty() {
                        let m = slice.len() / 2;
                        slice[m] ^= 0x01;
                    }
                    BlobSliceResponse {
                        found: true,
                        total_len: total,
                        slice_hex: hex::encode(&slice),
                    }
                }
                None => BlobSliceResponse {
                    found: false,
                    total_len: 0,
                    slice_hex: String::new(),
                },
            };
            serde_json::to_vec(&resp)?
        }
```

- [ ] **Step 3: Thread the `corrupt` flag through serve**

Change `serve_conn`'s signature and `cmd_serve`'s signature to carry `corrupt: bool`.

`serve_conn` signature (line ~852):

```rust
fn serve_conn(conn: &str, mut stream: TcpStream, corrupt: bool) -> R<()> {
```

`cmd_serve` (line ~744):

```rust
fn cmd_serve(conn: String, listen: &str, corrupt: bool) -> R<()> {
    let listener = TcpListener::bind(listen)?;
    eprintln!("serving on {listen}{}", if corrupt { " (CORRUPT: test fault injection)" } else { "" });
    for stream in listener.incoming() {
        let stream = stream?;
        let conn = conn.clone();
        std::thread::spawn(move || {
            if let Err(e) = serve_conn(&conn, stream, corrupt) {
                eprintln!("connection error: {e}");
            }
        });
    }
    Ok(())
}
```

- [ ] **Step 4: Store the outboard when bytes become local (put-blob)**

In `cmd_put_blob`, replace the body with the outboard-aware version:

```rust
fn cmd_put_blob(conn: &str, file: &str, media: &str) -> R<()> {
    let bytes = std::fs::read(file)?;
    let addr = blob_address(&bytes);
    let outboard = cairn_event::blob_outboard(&bytes);
    let len = bytes.len() as i64;
    let mut client = postgres::Client::connect(conn, postgres::NoTls)?;
    client.execute(
        "INSERT INTO blob_store (blob_address, media_type, byte_len, content, outboard, present, fetched_at)
         VALUES ($1,$2,$3,$4,$5,TRUE,clock_timestamp())
         ON CONFLICT (blob_address) DO UPDATE
            SET content=EXCLUDED.content, outboard=EXCLUDED.outboard, present=TRUE,
                byte_len=EXCLUDED.byte_len, fetched_at=clock_timestamp()",
        &[&addr, &media, &len, &bytes, &outboard],
    )?;
    println!("stored blob {} ({} bytes, {})", hex::encode(&addr), len, media);
    Ok(())
}
```

- [ ] **Step 5: Update the two `cmd_serve` call sites**

In `main`, the `"serve"` arm becomes:

```rust
        "serve" => cmd_serve(
            need(conn),
            &need(flag(&args, "--listen")),
            args.iter().any(|a| a == "--corrupt"),
        )?,
```

In `cmd_run`, the serve thread spawn becomes (honest serve in `run`):

```rust
        std::thread::spawn(move || {
            if let Err(e) = cmd_serve(c, &l, false) {
                eprintln!("serve thread exited: {e}");
            }
        });
```

- [ ] **Step 6: Build (do_blobd still references old types — expected to fail until Task 4)**

Run: `cargo build 2>&1 | tail -15`
Expected: compile errors in `do_blobd`/`cmd_blobd` referencing `Request::BlobChunk` / `BlobResponse`. That is fine — Task 4 rewrites them. Do NOT commit yet; Task 3 and Task 4 land together.

> Note: Tasks 3 and 4 are committed together (Step in Task 4) because the protocol rename leaves `do_blobd` temporarily uncompilable. They form one atomic change.

---

## Task 4: Fetch engine rewrite — worker pool, swarm, resume, persist

**Files:**
- Modify: `crates/cairn-sync/src/main.rs`

- [ ] **Step 1: Add imports**

At the top of `crates/cairn-sync/src/main.rs`, extend the `std::collections`/`std::sync` imports:

```rust
use std::collections::{HashSet, VecDeque};
use std::sync::{Arc, Mutex};
```

(Replace the existing `use std::sync::Arc;` line; keep the existing `atomic` import line.)

- [ ] **Step 2: Replace `do_blobd` and `cmd_blobd`**

Replace the whole `do_blobd` function and `cmd_blobd` function with:

```rust
/// The lazy byte tier (§6.6 / §8.2): for each blob whose bytes are missing, fetch
/// its slices with `window` worker threads, each round-robining across the swarm
/// `peers`, each verifying every slice against the content address (§4.4) before
/// persisting it to `blob_chunk`. Verified slices accumulate across passes/drops
/// (resumable); when every index is present the blob is assembled, whole-blob
/// re-verified, and flipped to present. Every worker sleeps `budget_ms` between
/// requests so windowing stays preemptible and never starves clinical sync
/// (ADR-0013 availability floor). Returns metrics for the harness.
fn do_blobd(
    client: &mut postgres::Client,
    conn: &str,
    peers: &[String],
    window: usize,
    budget_ms: u64,
) -> R<serde_json::Value> {
    let missing = client.query(
        "SELECT encode(blob_address,'hex'), byte_len FROM blob_store WHERE NOT present",
        &[],
    )?;

    let mut completed = 0usize;
    let rejected = Arc::new(AtomicU64::new(0));
    let fetched = Arc::new(AtomicU64::new(0));

    for row in missing {
        let addr_hex: String = row.get(0);
        let byte_len: Option<i64> = row.get(1);
        let total = match byte_len {
            Some(n) if n > 0 => n as u64,
            _ => continue, // length unknown -> can't chunk yet; a later reference fills it
        };
        let addr = hex::decode(&addr_hex)?;
        let n_chunks = total.div_ceil(SLICE_BYTES as u64) as usize;

        // Resume: which indexes are already persisted?
        let have: HashSet<i32> = client
            .query("SELECT chunk_index FROM blob_chunk WHERE blob_address=$1", &[&addr])?
            .iter()
            .map(|r| r.get::<_, i32>(0))
            .collect();
        let todo: VecDeque<usize> = (0..n_chunks).filter(|i| !have.contains(&(*i as i32))).collect();

        if !todo.is_empty() {
            let queue = Arc::new(Mutex::new(todo));
            let mut handles = Vec::new();
            for w in 0..window.max(1) {
                let queue = Arc::clone(&queue);
                let rejected = Arc::clone(&rejected);
                let fetched = Arc::clone(&fetched);
                let peers = peers.to_vec();
                let addr_hex = addr_hex.clone();
                let addr = addr.clone();
                let conn = conn.to_string();
                handles.push(std::thread::spawn(move || {
                    // Worker returns (); DB/link errors are logged and the worker moves on
                    // (the index stays missing and is retried next pass). A Box<dyn Error>
                    // return would not be Send across the thread boundary.
                    let mut wc = match postgres::Client::connect(&conn, postgres::NoTls) {
                        Ok(c) => c,
                        Err(e) => {
                            eprintln!("blob worker connect failed: {e}");
                            return;
                        }
                    };
                    let root = match cairn_event::blake3_root_from_address(&addr) {
                        Ok(r) => r,
                        Err(_) => return,
                    };
                    loop {
                        let idx = match queue.lock().unwrap().pop_front() {
                            Some(i) => i,
                            None => break,
                        };
                        let offset = idx as u64 * SLICE_BYTES as u64;
                        let len = (SLICE_BYTES as u64).min(total - offset);
                        // Try peers (offset by worker+index for swarm spread) until one
                        // returns a slice that VERIFIES. A lying/faulty source is rejected
                        // here and the next source is tried — the per-slice-verify payoff.
                        // try_request (single attempt) fails over fast, unlike request's backoff.
                        let mut got: Option<Vec<u8>> = None;
                        for k in 0..peers.len() {
                            let peer = &peers[(w + idx + k) % peers.len()];
                            std::thread::sleep(Duration::from_millis(budget_ms)); // preemptible budget
                            let raw = match try_request(
                                peer,
                                &Request::BlobSlice { addr_hex: addr_hex.clone(), offset, len },
                            ) {
                                Ok(r) => r,
                                Err(_) => continue, // link drop / dead peer -> next source
                            };
                            let resp: BlobSliceResponse = match serde_json::from_slice(&raw) {
                                Ok(r) => r,
                                Err(_) => continue,
                            };
                            if !resp.found {
                                continue;
                            }
                            let slice = match hex::decode(&resp.slice_hex) {
                                Ok(s) => s,
                                Err(_) => continue,
                            };
                            match cairn_event::verify_slice(&slice, &root, offset, len) {
                                Ok(bytes) => {
                                    got = Some(bytes);
                                    break;
                                }
                                Err(_) => {
                                    rejected.fetch_add(1, Ordering::Relaxed);
                                }
                            }
                        }
                        if let Some(bytes) = got {
                            if let Err(e) = wc.execute(
                                "INSERT INTO blob_chunk (blob_address, chunk_index, content)
                                 VALUES ($1,$2,$3) ON CONFLICT DO NOTHING",
                                &[&addr, &(idx as i32), &bytes],
                            ) {
                                eprintln!("blob_chunk insert failed: {e}");
                            } else {
                                fetched.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                        // If no source verified this index, leave it missing; the next
                        // do_blobd pass retries it from persisted state (resumable).
                    }
                }));
            }
            for h in handles {
                let _ = h.join();
            }
        }

        // Assemble if every index is now present.
        let have_now: i64 = client
            .query_one("SELECT count(*) FROM blob_chunk WHERE blob_address=$1", &[&addr])?
            .get(0);
        if have_now as usize == n_chunks && n_chunks > 0 {
            let rows = client.query(
                "SELECT content FROM blob_chunk WHERE blob_address=$1 ORDER BY chunk_index",
                &[&addr],
            )?;
            let mut buf = Vec::with_capacity(total as usize);
            for r in rows {
                let c: Vec<u8> = r.get(0);
                buf.extend_from_slice(&c);
            }
            // Belt-and-suspenders whole-blob verify before serving as present (§4.4).
            if blob_address(&buf) == addr {
                let outboard = cairn_event::blob_outboard(&buf);
                let mut tx = client.transaction()?;
                tx.execute(
                    "UPDATE blob_store SET content=$1, outboard=$2, present=TRUE, byte_len=$3,
                         fetched_at=clock_timestamp() WHERE blob_address=$4",
                    &[&buf, &outboard, &(buf.len() as i64), &addr],
                )?;
                tx.execute("DELETE FROM blob_chunk WHERE blob_address=$1", &[&addr])?;
                tx.commit()?;
                completed += 1;
                eprintln!("fetched blob {} ({} bytes, verified)", &addr_hex[..16], buf.len());
            } else {
                // Per-slice verify should make this unreachable; purge and retry if not.
                client.execute("DELETE FROM blob_chunk WHERE blob_address=$1", &[&addr])?;
                eprintln!("blob {} failed whole-blob verify — purged", &addr_hex[..16]);
            }
        }
    }

    Ok(serde_json::json!({
        "op": "blobd",
        "blobs_completed": completed,
        "slices_fetched": fetched.load(Ordering::Relaxed),
        "slices_rejected": rejected.load(Ordering::Relaxed),
        "window": window,
        "peers": peers.len()
    }))
}

fn cmd_blobd(
    conn: &str,
    peers: &[String],
    window: usize,
    budget_ms: u64,
    metrics: bool,
) -> R<()> {
    let mut client = postgres::Client::connect(conn, postgres::NoTls)?;
    let m = do_blobd(&mut client, conn, peers, window, budget_ms)?;
    if metrics {
        println!("{m}");
    } else {
        println!(
            "byte tier: {} blob(s) completed, {} slices fetched, {} rejected",
            m["blobs_completed"], m["slices_fetched"], m["slices_rejected"]
        );
    }
    Ok(())
}
```

> The `let mut slices_ok` / `slices_rejected` bindings are assigned once at the end; if clippy flags the initial `0` as never-read, change their declarations to `let slices_ok;` / `let slices_rejected;`.

- [ ] **Step 3: Update the `run` blob thread to use the new signature**

In `cmd_run`, the blob thread currently calls `do_blobd(&mut bclient, &p, budget_ms)`. Replace the blob-thread block so it carries the swarm peers + window. Change `cmd_run`'s signature to accept them and replace the thread body:

`cmd_run` signature — add `blob_peers: Vec<String>` and `window: usize` parameters (after `peer_name`):

```rust
#[allow(clippy::too_many_arguments)]
fn cmd_run(
    conn: &str,
    listen: &str,
    peer: &str,
    peer_name: &str,
    blob_peers: Vec<String>,
    window: usize,
    interval_ms: u64,
    budget_ms: u64,
    log_path: &str,
    duration_s: u64,
) -> R<()> {
```

Blob-thread block — replace it with:

```rust
    let blobs_fetched = Arc::new(AtomicU64::new(0));
    {
        let conn = conn.to_string();
        let peers = if blob_peers.is_empty() { vec![peer.to_string()] } else { blob_peers.clone() };
        let counter = Arc::clone(&blobs_fetched);
        std::thread::spawn(move || match postgres::Client::connect(&conn, postgres::NoTls) {
            Ok(mut bclient) => loop {
                match do_blobd(&mut bclient, &conn, &peers, window, budget_ms) {
                    Ok(m) => {
                        counter.fetch_add(m["blobs_completed"].as_u64().unwrap_or(0), Ordering::Relaxed)
                    }
                    Err(_) => 0, // peer unreachable: the next pass retries, never fatal
                };
                std::thread::sleep(Duration::from_millis(interval_ms));
            },
            Err(e) => eprintln!("blob thread could not connect: {e}"),
        });
    }
```

- [ ] **Step 4: Build**

Run: `cargo build 2>&1 | tail -15`
Expected: compile errors only in `main`'s arg dispatch (the `blobd`/`run` arms still call old signatures) — fixed in Task 5. If there are errors inside `do_blobd`/`cmd_run` bodies, fix them before proceeding.

> Tasks 3+4+5 commit together (the protocol rename spans all three). The commit is in Task 5.

---

## Task 5: CLI surface — swarm peers, window, gen-blob, dispatch

**Files:**
- Modify: `crates/cairn-sync/src/main.rs`

- [ ] **Step 1: Add a repeatable-flag helper**

Next to the existing `flag` helper, add:

```rust
/// All values for a repeatable flag, e.g. `--blob-peer A --blob-peer B`.
fn flags(args: &[String], name: &str) -> Vec<String> {
    args.iter()
        .enumerate()
        .filter(|(_, a)| a.as_str() == name)
        .filter_map(|(i, _)| args.get(i + 1).cloned())
        .collect()
}
```

- [ ] **Step 2: Add the `gen-blob` command**

Add this function next to `cmd_put_blob`:

```rust
/// Mint a large local blob (random-ish bytes) and store it present, so a real
/// multi-MB windowed fetch can be driven on the link without shipping a file. The
/// bytes come from a tiny xorshift PRNG (content just needs to be addressable and
/// distinct, not cryptographically random).
fn cmd_gen_blob(conn: &str, size_mb: usize, media: &str) -> R<()> {
    let n = size_mb.max(1) * 1024 * 1024;
    let mut buf = vec![0u8; n];
    let mut x = (now_ms() as u64) | 1;
    for b in buf.iter_mut() {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        *b = (x & 0xff) as u8;
    }
    let addr = blob_address(&buf);
    let outboard = cairn_event::blob_outboard(&buf);
    let len = buf.len() as i64;
    let mut client = postgres::Client::connect(conn, postgres::NoTls)?;
    client.execute(
        "INSERT INTO blob_store (blob_address, media_type, byte_len, content, outboard, present, fetched_at)
         VALUES ($1,$2,$3,$4,$5,TRUE,clock_timestamp())
         ON CONFLICT (blob_address) DO UPDATE
            SET content=EXCLUDED.content, outboard=EXCLUDED.outboard, present=TRUE,
                byte_len=EXCLUDED.byte_len, fetched_at=clock_timestamp()",
        &[&addr, &media, &len, &buf, &outboard],
    )?;
    println!(
        "{}",
        serde_json::json!({"op":"gen_blob","addr": hex::encode(&addr),"bytes": len,"media": media})
    );
    Ok(())
}
```

- [ ] **Step 3: Rewrite the `blobd`, `run`, and `serve` dispatch arms + add `gen-blob`**

In `main`, replace the `"blobd"` arm and the `"run"` arm, and add a `"gen-blob"` arm:

```rust
        "gen-blob" => cmd_gen_blob(
            &need(conn),
            flag(&args, "--size-mb").and_then(|s| s.parse().ok()).unwrap_or(8),
            &flag(&args, "--media").unwrap_or_else(|| "application/dicom".into()),
        )?,
        "blobd" => {
            let single = flag(&args, "--peer");
            let mut peers = flags(&args, "--blob-peer");
            if peers.is_empty() {
                peers.push(need(single));
            }
            cmd_blobd(
                &need(conn),
                &peers,
                flag(&args, "--window").and_then(|s| s.parse().ok()).unwrap_or(4),
                flag(&args, "--budget-ms").and_then(|s| s.parse().ok()).unwrap_or(20),
                args.iter().any(|a| a == "--metrics"),
            )?
        }
        "run" => cmd_run(
            &need(conn),
            &need(flag(&args, "--listen")),
            &need(flag(&args, "--peer")),
            &need(flag(&args, "--peer-name")),
            flags(&args, "--blob-peer"),
            flag(&args, "--window").and_then(|s| s.parse().ok()).unwrap_or(4),
            flag(&args, "--interval-ms").and_then(|s| s.parse().ok()).unwrap_or(2000),
            flag(&args, "--budget-ms").and_then(|s| s.parse().ok()).unwrap_or(20),
            &flag(&args, "--log").unwrap_or_else(|| "cairn-run.jsonl".into()),
            flag(&args, "--duration-s").and_then(|s| s.parse().ok()).unwrap_or(0),
        )?,
```

- [ ] **Step 4: Update the usage text**

In `usage()`, replace the `blobd`, `gen` area and `run` lines with:

```
  gen-blob    --conn URI [--size-mb N] [--media MEDIA_TYPE]   (mint a large local blob to fetch)
  pull        --conn URI --peer HOST:PORT --peer-name NAME [--metrics]
  blobd       --conn URI (--peer HOST:PORT | --blob-peer HOST:PORT ...) [--window N] [--budget-ms N] [--metrics]
  serve       --conn URI --listen HOST:PORT [--corrupt]
  run         --conn URI --listen HOST:PORT --peer HOST:PORT --peer-name NAME
              [--blob-peer HOST:PORT ...] [--window N] [--interval-ms N] [--budget-ms N] [--log PATH] [--duration-s N]
```

- [ ] **Step 5: Build, test, lint**

```bash
cargo build --release 2>&1 | tail -5
cargo test 2>&1 | tail -8
cargo clippy --all-targets -- -D warnings 2>&1 | tail -5
```

Expected: clean build; all existing tests pass; no clippy warnings.

- [ ] **Step 6: Smoke test the protocol end-to-end (single source)**

Run (set `PGCONN` to a writable local PG; the daemon stores bytes inline so a small blob is fine):

```bash
BIN=target/release/cairn-sync
psql "$PGCONN" -qc "drop table if exists event_log,hlc_state,sync_state,patient_chart,blob_store,blob_chunk cascade;"
$BIN init --conn "$PGCONN"
$BIN gen-blob --conn "$PGCONN" --size-mb 4 --media application/dicom   # source has the bytes
ADDR=$($BIN gen-blob --conn "$PGCONN" --size-mb 4 2>/dev/null | python3 -c "import sys,json;print(json.load(sys.stdin)['addr'])")
$BIN serve --conn "$PGCONN" --listen 127.0.0.1:7790 &
SRV=$!
# Reference the blob as missing on a SECOND db, then fetch it from the first:
psql "$PGCONN2" -qc "drop table if exists event_log,hlc_state,sync_state,patient_chart,blob_store,blob_chunk cascade;"
$BIN init --conn "$PGCONN2"
psql "$PGCONN2" -qc "select blob_note_reference(decode('$ADDR','hex'),'application/dicom', 4194304);"
$BIN blobd --conn "$PGCONN2" --peer 127.0.0.1:7790 --window 4 --budget-ms 5 --metrics
psql "$PGCONN2" -tAc "select present, octet_length(content) from blob_store where blob_address=decode('$ADDR','hex');"
kill $SRV
```

Expected: the `blobd` metrics line shows `blobs_completed: 1`, `slices_rejected: 0`; the final query prints `t|4194304` (present, full length).

- [ ] **Step 7: Commit Tasks 3–5 together**

```bash
git add crates/cairn-sync/src/main.rs
git commit -m "feat(byte-tier): windowed, resumable, swarm, per-slice-verified fetch + gen-blob/--blob-peer/--window"
```

---

## Task 6: Measurement + selftest harness (`harness/bench_blob.py`)

**Files:**
- Create: `harness/bench_blob.py`

- [ ] **Step 1: Write the harness**

Create `harness/bench_blob.py` with exactly this content:

```python
#!/usr/bin/env python3
"""Byte-tier throughput harness — Spike 0001 §8.2.

Drives the `cairn-sync` binary to validate the production byte tier: windowed +
resumable + multi-source swarm + per-slice BLAKE3 verification, without starving
clinical sync (the ADR-0013 availability floor). Stdlib only; `psql` is used for
setup (present on any node running PostgreSQL).

USE A RELEASE BINARY: cargo build --release.

WARNING: `selftest` DROPs and recreates the Cairn tables on the target DB(s).
Refuses to run without --force.

Checks:
  T1 windowed fetch    : a multi-MB blob fetches + verifies; report throughput + window
  T2 resume            : a fetch interrupted mid-transfer completes from persisted chunks
  T3 swarm             : chunks pulled from two honest sources still converge
  T4 lying peer        : a --corrupt source is rejected per-slice and healed by a good source
  T5 availability floor: clinical pull p95 unaffected during a concurrent windowed fetch
"""

import argparse
import json
import os
import subprocess
import sys
import time
from statistics import median


def p95(xs):
    if not xs:
        return 0.0
    s = sorted(xs)
    return s[min(len(s) - 1, int(round(0.95 * (len(s) - 1))))]


class Node:
    def __init__(self, bin_path, conn, name, listen=None):
        self.bin = bin_path
        self.conn = conn
        self.name = name
        self.listen = listen

    def _run(self, *args, background=False):
        cmd = [self.bin, args[0], "--conn", self.conn, *args[1:]]
        if background:
            return subprocess.Popen(cmd, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        out = subprocess.run(cmd, capture_output=True, text=True)
        if out.returncode != 0:
            raise RuntimeError(f"{' '.join(cmd)}\n{out.stderr.strip()}")
        return out.stdout

    def _json(self, *args):
        lines = [l for l in self._run(*args).splitlines() if l.strip().startswith("{")]
        return json.loads(lines[-1])

    def init(self):
        self._run("init")

    def reset(self):
        subprocess.run(
            ["psql", self.conn, "-qc",
             "drop table if exists event_log,hlc_state,sync_state,patient_chart,"
             "blob_store,blob_chunk cascade;"],
            capture_output=True, text=True,
        )

    def gen(self, key, patients=1, count=20, rate=0.0, background=False):
        return self._run("gen", "--node", self.name, "--key", key,
                         "--patients", str(patients), "--count", str(count),
                         "--rate", str(rate), background=background)

    def serve(self, corrupt=False):
        args = ["serve", "--listen", self.listen]
        if corrupt:
            args.append("--corrupt")
        return self._run(*args, background=True)

    def gen_blob(self, size_mb, media="application/dicom"):
        return self._json("gen-blob", "--size-mb", str(size_mb), "--media", media)

    def reference_blob(self, addr_hex, media, length):
        subprocess.run(
            ["psql", self.conn, "-qc",
             f"select blob_note_reference(decode('{addr_hex}','hex'),'{media}',{length});"],
            capture_output=True, text=True, check=True,
        )

    def blobd(self, peers, window=4, budget_ms=2, background=False):
        args = ["blobd", "--window", str(window), "--budget-ms", str(budget_ms), "--metrics"]
        for p in peers:
            args += ["--blob-peer", p]
        if background:
            return self._run(*args, background=True)
        return self._json(*args)

    def pull(self, peer_addr, peer_name):
        return self._json("pull", "--peer", peer_addr, "--peer-name", peer_name, "--metrics")

    def present(self, addr_hex):
        out = subprocess.run(
            ["psql", self.conn, "-tAc",
             f"select present, coalesce(octet_length(content),0) "
             f"from blob_store where blob_address=decode('{addr_hex}','hex');"],
            capture_output=True, text=True, check=True,
        ).stdout.strip()
        if not out:
            return (False, 0)
        present, length = out.split("|")
        return (present == "t", int(length))

    def chunk_count(self, addr_hex):
        out = subprocess.run(
            ["psql", self.conn, "-tAc",
             f"select count(*) from blob_chunk where blob_address=decode('{addr_hex}','hex');"],
            capture_output=True, text=True, check=True,
        ).stdout.strip()
        return int(out or 0)


def cmd_selftest(args):
    if not args.force:
        sys.exit("selftest is destructive (drops Cairn tables). Re-run with --force.")
    src = Node(args.bin, args.conn, "src", args.listen)
    src2 = Node(args.bin, args.conn_b, "src2", args.listen_b)
    dst = Node(args.bin, args.conn_c, "dst")
    for n in (src, src2, dst):
        n.reset()
        n.init()

    size_mb = args.size_mb
    nbytes = size_mb * 1024 * 1024
    media = "application/dicom"
    rows = []

    # Both honest sources hold the SAME blob (gen-blob is deterministic per-call only,
    # so generate on src and copy bytes to src2 via a file round-trip through put-blob).
    blob = src.gen_blob(size_mb, media)
    addr = blob["addr"]
    # Materialize identical bytes on src2 so it is a genuine second source: export
    # src's content as hex, write a file, put-blob it on src2 (same bytes -> same addr).
    tmp = f"/tmp/cairn_blob_{os.getpid()}.bin"
    hexout = subprocess.run(
        ["psql", src.conn, "-tAc",
         f"select encode(content,'hex') from blob_store where blob_address=decode('{addr}','hex')"],
        capture_output=True, text=True, check=True,
    ).stdout.strip()
    with open(tmp, "wb") as f:
        f.write(bytes.fromhex(hexout))
    src2._run("put-blob", "--file", tmp, "--media", media)

    serves = [src.serve(), src2.serve()]
    src2_corrupt = None
    time.sleep(0.5)
    try:
        # T1 windowed fetch (single honest source).
        dst.reference_blob(addr, media, nbytes)
        t0 = time.time()
        m = dst.blobd([src.listen], window=args.window, budget_ms=args.budget_ms)
        # Loop passes until complete (a pass makes progress; resumable).
        passes = 1
        while not dst.present(addr)[0] and passes < 200:
            m = dst.blobd([src.listen], window=args.window, budget_ms=args.budget_ms)
            passes += 1
        elapsed = time.time() - t0
        present, length = dst.present(addr)
        mbps = (nbytes / (1 << 20)) / elapsed if elapsed > 0 else 0.0
        rows.append(("T1", "windowed fetch", present and length == nbytes,
                     f"{size_mb} MB in {elapsed:.1f}s ({mbps:.1f} MB/s), window {args.window}, {passes} pass(es)"))

        # T2 resume: INTERRUPT a fetch mid-transfer (a single blobd call drains the
        # whole queue, so resume only manifests on interruption), confirm a partial
        # set of chunks persisted, then resume to completion from those chunks. window=1
        # + 50ms budget makes the fetch take ~n_chunks*50ms so a 0.6s kill lands mid-way.
        SLICE = 262144  # MUST match SLICE_BYTES in cairn-sync
        t2_mb = max(size_mb, 8)
        t2_bytes = t2_mb * 1024 * 1024
        n_chunks = (t2_bytes + SLICE - 1) // SLICE
        t2_blob = src.gen_blob(t2_mb, media)
        t2_addr = t2_blob["addr"]
        # mirror the bytes onto src2 is unnecessary here (single source).
        dst.reset(); dst.init()
        dst.reference_blob(t2_addr, media, t2_bytes)
        bd = dst.blobd([src.listen], window=1, budget_ms=50, background=True)
        time.sleep(0.6)
        bd.terminate(); bd.wait()
        partial = dst.chunk_count(t2_addr)
        passes = 0
        while not dst.present(t2_addr)[0] and passes < 200:
            dst.blobd([src.listen], window=args.window, budget_ms=args.budget_ms)
            passes += 1
        resumed = dst.present(t2_addr)[0] and dst.present(t2_addr)[1] == t2_bytes
        rows.append(("T2", "resume across interrupt", resumed and 0 < partial < n_chunks,
                     f"{partial}/{n_chunks} chunks persisted at interrupt, then resumed to complete"))

        # T3 swarm: fresh dst, two honest sources.
        dst.reset(); dst.init()
        dst.reference_blob(addr, media, nbytes)
        passes = 0
        while not dst.present(addr)[0] and passes < 200:
            m = dst.blobd([src.listen, src2.listen], window=args.window, budget_ms=args.budget_ms)
            passes += 1
        rows.append(("T3", "swarm (2 sources)", dst.present(addr)[0] and dst.present(addr)[1] == nbytes,
                     f"converged from 2 sources in {passes} pass(es)"))

        # T4 lying peer: stop src2's honest serve, restart it as a CORRUPT source on the
        # same port; dst fetches from [liar, honest] so a rejected slice heals via src.
        serves[1].terminate(); serves[1].wait()
        time.sleep(0.3)
        src2_corrupt = src2.serve(corrupt=True)
        time.sleep(0.5)
        dst.reset(); dst.init()
        dst.reference_blob(addr, media, nbytes)
        rejected_total = 0
        passes = 0
        while not dst.present(addr)[0] and passes < 200:
            m = dst.blobd([src2.listen, src.listen], window=args.window, budget_ms=args.budget_ms)
            rejected_total += int(m["slices_rejected"])
            passes += 1
        healed = dst.present(addr)[0] and dst.present(addr)[1] == nbytes
        rows.append(("T4", "lying peer healed", healed and rejected_total > 0,
                     f"{rejected_total} slice(s) rejected by per-slice verify, then healed"))

        # T5 availability floor: clinical pull p95 unaffected during a windowed fetch.
        dst.reset(); dst.init()
        key = f"/tmp/cairn_floor_{os.getpid()}.key"
        dst.reference_blob(addr, media, nbytes)

        def drain():
            for _ in range(200):
                if dst.pull(src.listen, src.name)["applied_new"] == 0:
                    return

        def sample():
            src.gen(key, patients=1, count=20)
            return dst.pull(src.listen, src.name)["elapsed_ms"]

        drain()
        base = [sample() for _ in range(args.rounds)]
        bd = dst.blobd([src.listen], window=args.window, budget_ms=args.budget_ms, background=True)
        during = []
        while bd.poll() is None and len(during) < args.rounds * 3:
            during.append(sample())
        bd.wait()
        base_p95, during_p95 = p95(base), p95(during)
        tol = args.tolerance
        floor_ok = during_p95 <= base_p95 * (1 + tol) + 5.0
        rows.append(("T5", "availability floor", floor_ok,
                     f"clinical pull p95 base {base_p95:.0f}ms -> during {during_p95:.0f}ms (tol {int(tol*100)}%)"))
    finally:
        for s in serves:
            s.terminate()
        if src2_corrupt:
            src2_corrupt.terminate()
        try:
            os.remove(tmp)
        except OSError:
            pass

    # Render.
    w = max(len(r[1]) for r in rows)
    print(f"\n{'':4}{'check':<{w}}  result  detail")
    print("-" * (12 + w + 50))
    ok = True
    for code, name, passed, detail in rows:
        ok = ok and passed
        print(f"{code:<4}{name:<{w}}  {'PASS' if passed else 'FAIL':<6}  {detail}")
    print("-" * (12 + w + 50))
    print(f"\nByte tier: {'PASS' if ok else 'FAIL'}\n")
    sys.exit(0 if ok else 1)


def main():
    ap = argparse.ArgumentParser(description="Cairn byte-tier throughput harness (Spike 0001 §8.2)")
    sub = ap.add_subparsers(dest="cmd", required=True)
    st = sub.add_parser("selftest", help="local multi-node validation (destructive)")
    st.add_argument("--bin", default="target/release/cairn-sync")
    st.add_argument("--conn", required=True, help="source node PG conn")
    st.add_argument("--conn-b", dest="conn_b", required=True, help="second source PG conn")
    st.add_argument("--conn-c", dest="conn_c", required=True, help="fetcher PG conn")
    st.add_argument("--listen", default="127.0.0.1:7790")
    st.add_argument("--listen-b", default="127.0.0.1:7791")
    st.add_argument("--size-mb", type=int, default=8)
    st.add_argument("--window", type=int, default=4)
    st.add_argument("--budget-ms", type=int, default=2)
    st.add_argument("--rounds", type=int, default=8)
    st.add_argument("--tolerance", type=float, default=0.30)
    st.add_argument("--force", action="store_true")
    st.set_defaults(func=cmd_selftest)
    args = ap.parse_args()
    args.func(args)


if __name__ == "__main__":
    main()
```

- [ ] **Step 2: Make it executable**

Run: `chmod +x harness/bench_blob.py`

- [ ] **Step 3: Run the selftest against local PG**

You need three writable PG databases (or three schemas). Example with three local DBs:

```bash
cd poc/walking-skeleton
cargo build --release 2>&1 | tail -3
python3 harness/bench_blob.py selftest \
  --conn "postgres://localhost/cairn_a" \
  --conn-b "postgres://localhost/cairn_b" \
  --conn-c "postgres://localhost/cairn_c" \
  --size-mb 8 --window 4 --force
```

Expected: a table with **T1–T5 all PASS**. T1 reports throughput; T4 reports a non-zero rejected-slice count that then heals; T5 shows clinical p95 roughly unchanged during the fetch.

> T2 relies on a timed interrupt (kill at 0.6s of a ~`n_chunks`×50ms fetch). If it ever records `partial == 0` (killed too early) or `partial == n_chunks` (too late), it's a timing miss, not a logic failure — re-run, or widen the gap by raising the T2 blob size. The mechanism under test (resume from persisted `blob_chunk`) is deterministic; only the kill timing is probabilistic.

- [ ] **Step 4: Commit**

```bash
git add harness/bench_blob.py
git commit -m "test(byte-tier): bench_blob.py — windowed/resume/swarm/lying-peer/floor selftest"
```

---

## Task 7: Documentation

**Files:**
- Modify: `poc/walking-skeleton/README.md`

- [ ] **Step 1: Read the README's byte-tier section**

Run: `grep -n -i "blob\|byte tier\|blobd\|lazy" poc/walking-skeleton/README.md`
Read the surrounding sections to match tone and find where the byte tier and command list are documented.

- [ ] **Step 2: Update the byte-tier description and command list**

Update the README so it reflects the production-shaped tier. The byte tier is now:
- **resumable** — verified slices persist in `blob_chunk`; a fetch resumes across drops/restarts;
- **windowed** — `--window N` worker threads fetch slices concurrently;
- **swarm** — `--blob-peer` is repeatable; each slice is tried against sources in turn until one verifies;
- **per-slice verified** — every slice self-verifies against the BLAKE3 `blob_address` via bao verified streaming (`cairn-event::verify_slice`), so a lying/faulty source is rejected and healed by another;
- still **preemptible + own-thread** — `--budget-ms` between requests, the #9 availability-floor discipline preserved.

Add the new commands to the command list: `gen-blob`, the `blobd` `--blob-peer`/`--window`/`--metrics` flags, and `serve --corrupt` (label it test-only fault injection). Add a one-line pointer to `harness/bench_blob.py` for measuring it.

- [ ] **Step 3: Commit**

```bash
git add poc/walking-skeleton/README.md
git commit -m "docs(byte-tier): document the windowed/resumable/swarm/verified byte tier"
```

---

## Final verification (before handing back for the real-link run)

- [ ] `cargo test 2>&1 | tail -8` — all green.
- [ ] `cargo clippy --all-targets -- -D warnings 2>&1 | tail -5` — no warnings.
- [ ] `python3 harness/bench_blob.py selftest --conn ... --conn-b ... --conn-c ... --size-mb 8 --force` — T1–T5 PASS.
- [ ] Report the T1 throughput and T4 rejected-slice numbers to the user so they can compare on the real Cape York ↔ Dorrigo link.

**Deferred to the real-link run (user-driven, not in this plan):** running `bench_blob.py`/`run` over the actual ~710 ms link to measure real throughput + resume-across-a-real-drop, and (after it concludes) marking Spike 0001 §8.2 as delivered and noting the chosen `SLICE_BYTES`/`--window` in the run notes.
