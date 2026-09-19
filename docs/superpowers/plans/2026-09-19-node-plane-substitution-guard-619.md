# The node plane refuses a substitution at both live doors, and pens it (#619) — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended)
> or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax
> for tracking. **Subagents: FOREGROUND commands only** — a subagent that ends its turn waiting on a
> background job never wakes.

**Goal:** db/007's `submit_node_event` and `apply_remote_node_event` refuse a second, different event
under an `event_id` already held (through the shared `cairn_refuse_substitution`), and the node-plane
puller pens such a refused event instead of skipping it.

**Architecture:** Each db/007 door is restructured to one shared tail (db/009's shape) that reads the
stored content address unconditionally and calls the helper once. In `pull_into`, the verifiable-P0001
arm asks one new question — *does `node_event` already hold this id under a different address?* —
answered by STATE (a pure `substitution_reason` plus a one-row lookup), never by SQLSTATE or message
text; a yes pens the bytes through the existing quarantine. A `pg_proc` catalogue rule replaces the
hand-written door inventory.

**Tech Stack:** PL/pgSQL (PostgreSQL 18 + `cairn_pgx`), Rust (`tokio-postgres`, `cairn-event`),
`cargo test`, bash mutation harness.

**Spec:** `docs/superpowers/specs/2026-09-19-node-plane-substitution-guard-619-design.md`

## Global Constraints

- **No new migration.** db/007 is edited in place; `SCHEMA_GENERATION` stays **53**; loader lists
  unchanged. (#605 is the accepted residual.)
- **P0001 is a contract** (db/001 header): never add `USING ERRCODE` to any RAISE.
- **Every refusal in db/007 keeps its exact text and its order.** Only the shared tail is new.
- **db/007 keeps exactly 4 `cairn_decode_hex_or_raise` calls** (pinned by `hex_decode_helper.rs`).
- **db/007's `PERFORM cairn_node_hlc_merge(` count goes 3 → 1** (pin in `hlc_merge_helper.rs` moves with it).
- **Guard placement:** AFTER the `IF/ELSE`, never above it; unconditional read, never `GET DIAGNOSTICS`.
- **House rule 6:** test values in a crypto-named binding (`nonce`) are runtime-derived; discriminators
  are called `lineage`, never `seed`/`salt`/`nonce`.
- **Commit convention:** `feat(#619):` / `test(#619):` / `docs(#619):` — the parenthesis keeps GitHub's
  closing-keyword parser from closing the issue. Never write `closes #619` / `fixes #619` anywhere.
- Every commit ends with `Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>`.

## How to run tests (read once)

```bash
cd /Users/hherb/src/cairn-ehr
read -r PGHOST PGPORT <<<"$(scripts/pg-target.sh)"          # today: 127.0.0.1 5532
export CAIRN_TEST_PG="host=$PGHOST port=$PGPORT user=$(whoami) dbname=cairn_test"
export CAIRN_TEST_PG2="host=$PGHOST port=$PGPORT user=$(whoami) dbname=cairn_test2"
export CAIRN_TEST_PG3="host=$PGHOST port=$PGPORT user=$(whoami) dbname=cairn_test3"
cargo test -p cairn-node --test <suite>
```

- **Never pipe `cargo test` into `tail`/`head`** — it masks cargo's exit code. Read the `test result:` line.
- If cargo prints `Blocking waiting for file lock`, the IDE's rust-analyzer holds `target/`: add
  `CARGO_TARGET_DIR=/tmp/cairn-619-target` (first build is slow; reuse it for every later command).
- A freshly linked test binary can stall ~minutes on macOS Gatekeeper the first time it runs; if a
  narrow run hangs, exec the printed `target/debug/deps/<suite>-<hash>` directly.
- `db/*.sql` is `include_str!`-ed: an SQL edit needs a rebuild, which `cargo test` does itself.

## File structure

| File | Responsibility | Task |
|---|---|---|
| `crates/cairn-node/tests/common/sql_text.rs` (new) | SQL comment stripper + normaliser, shared by the two catalogue guards | 1 |
| `crates/cairn-node/tests/common/node_plane_kit.rs` (new) | signed node events under a chosen id; fresh/trusting nodes; the single-DB self-pull fixture | 1 |
| `crates/cairn-node/tests/late_custody_guards.rs` | uses `sql_text` instead of its private copy | 1 |
| `crates/cairn-node/tests/node_quarantine.rs` | uses `node_plane_kit`'s self-pull instead of its private copy | 1 |
| `db/007_node_federation.sql` | both doors: one shared tail + the guard | 2, 3 |
| `crates/cairn-node/tests/node_plane_one_event_id_one_body.rs` (new) | per-arm door behaviour: rival refused, repeat admitted | 2, 3 |
| `crates/cairn-node/tests/substitution_guard_covers_every_writer.rs` (new) | catalogue rule over `pg_proc` | 3 |
| `crates/cairn-node/tests/substitution_guard_is_single_source.rs` | hand-written inventory removed (pointer left) | 3 |
| `crates/cairn-node/tests/hlc_merge_helper.rs`, `db/001_envelope.sql` (comment) | merge-site count 3 → 1 | 3 |
| `crates/cairn-node/src/sync/substitution.rs` (new) | pure `substitution_reason` + `held_content_address` | 4 |
| `crates/cairn-node/src/sync.rs` | `pub mod substitution;`, `pen_or_freeze`, the P0001 arm, docs | 4, 5 |
| `crates/cairn-node/src/main.rs` | `quarantine` help text | 5 |
| `crates/cairn-node/tests/node_substitution_is_penned.rs` (new) | the pull path pens a substitution | 5 |
| `scripts/mutations/2026-09-19-619.sh` (new) | mutation harness | 6 |
| `docs/spec/decisions/0073-…md` (new), `decisions/README.md`, `mkdocs.yml`, `docs/spec/index.md`, `docs/spec/sync.md` | ADR + spec v0.75 | 7 |

---

### Task 1: Two shared test kits (behaviour-preserving move)

Two helpers the new suites need already exist privately in other suites. Copying them would recreate
"one invariant written twice" — the shape #608 came from — so they move into `tests/common/` kits,
included by `#[path]` (the `late_custody_kit.rs` idiom; kits are NOT pinned by
`identity_scaffolding_shared.rs`, which reads only `common/mod.rs`).

**Files:**
- Create: `crates/cairn-node/tests/common/sql_text.rs`
- Create: `crates/cairn-node/tests/common/node_plane_kit.rs`
- Modify: `crates/cairn-node/tests/late_custody_guards.rs` (remove `without_sql_comments`, `without_first_char`, `normalised`; include the kit)
- Modify: `crates/cairn-node/tests/node_quarantine.rs` (remove `SelfNode`, `self_node`, `pen_count`; include the kit)

**Interfaces — Produces** (every later task relies on these exact names):
- `sql_text::normalised(body: &str) -> String`, `sql_text::without_sql_comments(body: &str) -> String`
- `node_plane_kit::SENTENCE: &str`
- `node_plane_kit::cs() -> Option<String>`
- `node_plane_kit::key_hex(sk: &SigningKey) -> String`
- `node_plane_kit::address_of(signed: &[u8]) -> Vec<u8>`
- `node_plane_kit::node_id_of(genesis: &[u8]) -> String`
- `node_plane_kit::node_id_hex(lineage: u8) -> String`
- `node_plane_kit::genesis_event(sk: &SigningKey, event_id: Uuid, name: &str) -> Vec<u8>`
- `node_plane_kit::peer_event(sk: &SigningKey, event_type: &str, event_id: Uuid, subject_hex: &str) -> Vec<u8>`
- `node_plane_kit::supersede_event(sk: &SigningKey, event_id: Uuid, superseded_hex: &str) -> Vec<u8>`
- `node_plane_kit::call(c: &Client, door: &str, signed: &[u8]) -> Result<(), String>` (async)
- `node_plane_kit::held_address(c: &Client, event_id: Uuid) -> Option<Vec<u8>>` (async)
- `node_plane_kit::FreshNode { pub db: Client, pub sk: SigningKey, pub node_id: String }`
- `node_plane_kit::fresh_node(base: &str) -> FreshNode` (async)
- `node_plane_kit::trust(a: &FreshNode, peer_genesis: &[u8], peer_sk: &SigningKey)` (async)
- `node_plane_kit::SelfNode { pub a: Client, pub addr: SocketAddr, pub serve: JoinHandle<anyhow::Result<()>>, pub sk: SigningKey, _tmp }`
- `node_plane_kit::self_node(base: &str, listen_addr: &str) -> SelfNode` (async)
- `node_plane_kit::pen_count(a: &Client) -> i64` (async)
- `node_plane_kit::serve_raw(a: &Client, signed: &[u8]) -> i64` (async; returns the row's `seq`)

- [ ] **Step 1: Create `sql_text.rs` by MOVING the three functions out of `late_custody_guards.rs`**

Cut `without_sql_comments`, `without_first_char` and `normalised` (with their doc comments) out of
`crates/cairn-node/tests/late_custody_guards.rs` and paste them, bodies unchanged, into the new file
below the header. Make `without_sql_comments` and `normalised` `pub`; `without_first_char` stays private.

```rust
//! SQL function-body text helpers for the catalogue guards — shared so two guards cannot drift.
//!
//! `late_custody_guards.rs` (#584) and `substitution_guard_covers_every_writer.rs` (#619) both read
//! function bodies out of `pg_proc.prosrc` and ask what the CODE does while ignoring what the
//! comments say. That needs one comment stripper. A stripper written twice is the drift #608 was
//! made of — one invariant spelled in two places, wrong in both at once — so it was moved here,
//! unchanged, the moment a second guard needed it.
//!
//! Include with `#[path = "common/sql_text.rs"] mod sql_text;`.
//!
//! Literal-blind, like the original: a `--` inside a string literal hides the rest of its line, and a
//! `/*` inside one hides everything up to the next `*/`. No call site either guard looks for follows
//! a comment marker inside a literal; keep it that way.
#![allow(dead_code)] // each including suite uses a different subset

// <paste: fn without_sql_comments — now `pub fn`>
// <paste: fn without_first_char — stays private>
// <paste: fn normalised — now `pub fn`>
```

(The pasted text runs from the doc line `/// The body with its SQL comments removed` through the end of
`fn normalised` — today ≈ `late_custody_guards.rs:40-107`; `fn cs()` just above it STAYS. Copy it
verbatim; do not retype it.)

- [ ] **Step 2: Point `late_custody_guards.rs` at the kit**

Directly below its `//!` header (before `use cairn_node::db;`), add:

```rust
#[path = "common/sql_text.rs"]
mod sql_text;

use sql_text::normalised;
```

In the header, replace the paragraph starting `Comments — \`--\` line comments` so its first sentence
reads: `Comments — \`--\` line comments and \`/* ... */\` block comments — are stripped before matching
(by \`common/sql_text.rs\`, shared with #619's catalogue guard), so prose naming a function neither
satisfies nor trips a rule.` Leave the rest of the header as is.

- [ ] **Step 3: Create `node_plane_kit.rs`**

Move `SelfNode` and `self_node` (and `pen_count`) out of `crates/cairn-node/tests/node_quarantine.rs`
into this file, then add the builders. Two deliberate edits to the moved `self_node`, both called out
in comments: the `PairingBundle.nonce` becomes runtime-derived (house rule 6a — `nonce` is a CodeQL
sink name, and a new file with a literal there would mint a new alert), and nothing else changes.

```rust
//! Node-plane fixtures shared by the #619 suites and `node_quarantine.rs`.
//!
//! Two kinds of thing live here:
//!
//! * **Signed node events under a CALLER-CHOSEN `event_id`.** Production mints a fresh UUIDv7 for
//!   every node event (`identity::node_event_body`), so the only way to build a SUBSTITUTION — a
//!   second, different event under an id the log already holds — is to choose the id. Everything
//!   else about two rivals differs (type or payload), which is what makes their content addresses
//!   differ and the pair a substitution rather than a repeat.
//! * **The single-DB self-pull** (#111). Node A serves its own `node_event` log to itself over pinned
//!   mTLS and pulls it back, so a row raw-inserted into A's log is streamed, received and re-applied
//!   through the REAL admission gate — `pull_into`'s classification exercised end-to-end without a
//!   second database. It lived in `node_quarantine.rs` until #619's pull suite needed it too.
//!
//! Include with `#[path = "common/node_plane_kit.rs"] mod node_plane_kit;`.
#![allow(dead_code)] // each including suite uses a different subset

use cairn_event::{
    event_address, generate_key, sign, ClockGrade, EventBody, Hlc, PairingBundle, SigningKey,
};
use cairn_node::{db, identity, keystore, sync};
use std::net::SocketAddr;
use tokio_postgres::Client;
use uuid::Uuid;

/// The tail of the refusal every write door raises through `cairn_refuse_substitution` (db/053).
/// The door's own name is interpolated IN FRONT of it, so this tail is what a test can match.
pub const SENTENCE: &str = "already exists with different content (substitution refused)";

/// The test database, or `None` when `$CAIRN_TEST_PG` is unset — the repo-wide self-skip,
/// policed by `tests/db_gate_actually_ran.rs`.
pub fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

/// The hex public key — what every node event carries as `signer_key_id`, and what
/// `generate_key` / `keystore::generate_plaintext` return as the key id.
pub fn key_hex(sk: &SigningKey) -> String {
    hex::encode(sk.verifying_key().to_bytes())
}

/// The content address of signed bytes — byte-identical to db/007's
/// `'\x1220' || digest(p_signed, 'sha256')`.
pub fn address_of(signed: &[u8]) -> Vec<u8> {
    event_address(signed)
}

/// The node id a genesis DEFINES: the hex of its own content address (db/007 stores
/// `node_id = v_ca`), so a pairing bundle can name a node before its genesis is ever admitted.
pub fn node_id_of(genesis: &[u8]) -> String {
    hex::encode(event_address(genesis))
}

/// A 32-byte node id as hex, for an event's SUBJECT (a peer to add, a node superseded).
///
/// Derived at runtime (house rule 6a) and discriminated by `lineage`, never `seed`/`salt`/`nonce`
/// (6b): nothing here is cryptographic — the doors only decode it.
pub fn node_id_hex(lineage: u8) -> String {
    hex::encode(
        (0..32u8)
            .map(|i| i.wrapping_mul(11).wrapping_add(lineage))
            .collect::<Vec<u8>>(),
    )
}

/// Sign a node event under a CALLER-CHOSEN `event_id`. The single builder the three below share.
///
/// `wall` stays tiny (1–3 ms since the epoch): far below the remote door's clock-drift ceiling
/// (#102), so no test here is refused for its clock rather than for what it tests.
pub fn node_event(
    sk: &SigningKey,
    event_type: &str,
    event_id: Uuid,
    wall: i64,
    payload: serde_json::Value,
) -> Vec<u8> {
    let kid = key_hex(sk);
    let body = EventBody {
        event_id: event_id.to_string(),
        patient_id: identity::NIL_PATIENT.into(),
        event_type: event_type.into(),
        schema_version: "node/1".into(),
        hlc: Hlc {
            wall,
            counter: 0,
            node_origin: kid.clone(),
        },
        t_effective: None,
        signer_key_id: kid.clone(),
        contributors: serde_json::json!([{ "actor_id": kid, "role": "recorded" }]),
        payload,
        attachments: vec![],
        plaintext_twin: None,
        clock_grade: ClockGrade::SelfAsserted,
        safety: None,
    };
    sign(&body, sk).unwrap().signed_bytes
}

/// A `node.enrolled` genesis under a chosen id. Its content address IS the node's id.
pub fn genesis_event(sk: &SigningKey, event_id: Uuid, name: &str) -> Vec<u8> {
    node_event(
        sk,
        "node.enrolled",
        event_id,
        1,
        serde_json::json!({ "display_name": name, "address": "127.0.0.1:7999" }),
    )
}

/// A `peer.added` / `peer.revoked` about `subject_hex`, under a chosen id.
pub fn peer_event(sk: &SigningKey, event_type: &str, event_id: Uuid, subject_hex: &str) -> Vec<u8> {
    node_event(
        sk,
        event_type,
        event_id,
        2,
        serde_json::json!({ "peer_node_id_hex": subject_hex, "role": "peer" }),
    )
}

/// A `node.superseded` naming `superseded_hex`, under a chosen id. Separate from [`peer_event`]
/// because the supersede arm reads `superseded_node_id_hex`, not `peer_node_id_hex`.
pub fn supersede_event(sk: &SigningKey, event_id: Uuid, superseded_hex: &str) -> Vec<u8> {
    node_event(
        sk,
        "node.superseded",
        event_id,
        3,
        serde_json::json!({ "superseded_node_id_hex": superseded_hex }),
    )
}

/// Call a node-plane door (`submit_node_event`, `apply_remote_node_event`, `restore_node_event`)
/// and return its refusal message, if it refused. `door` is always a constant in these suites.
pub async fn call(c: &Client, door: &str, signed: &[u8]) -> Result<(), String> {
    c.execute(&format!("SELECT {door}($1)"), &[&signed])
        .await
        .map(|_| ())
        .map_err(|e| {
            e.as_db_error()
                .map(|d| d.message().to_string())
                .unwrap_or_else(|| e.to_string())
        })
}

/// What `node_event` holds under `event_id` — its content address — or `None`.
pub async fn held_address(c: &Client, event_id: Uuid) -> Option<Vec<u8>> {
    c.query_opt(
        "SELECT content_address FROM node_event WHERE node_event_id = $1::text::uuid",
        &[&event_id.to_string()],
    )
    .await
    .unwrap()
    .map(|r| r.get(0))
}

/// A freshly provisioned node, reset first, with no serve listener.
pub struct FreshNode {
    pub db: Client,
    pub sk: SigningKey,
    /// Hex of this node's own genesis content address.
    pub node_id: String,
}

/// Provision node A through the real `submit_node_event` genesis arm.
pub async fn fresh_node(base: &str) -> FreshNode {
    let db = db::connect_and_load_schema(base).await.unwrap();
    db::reset_node_federation_tables(&db).await.expect(
        "the fixture reset must succeed — swallowing it with .ok() is the shape behind the #296 \
         pollution lessons: a leftover local_node would fence the doors closed and every \
         assertion would then fail for the wrong reason",
    );
    let (sk, kid) = generate_key().unwrap();
    let node_id = identity::provision(&db, &sk, &kid, "A", "127.0.0.1:7999")
        .await
        .unwrap();
    FreshNode { db, sk, node_id }
}

/// Make `a` trust the node whose genesis is `peer_genesis` (signed by `peer_sk`): `a` authors a
/// `peer.added` through the real submit door, which is what `apply_remote_node_event`'s trust
/// checks read (`trust_peer` shows only events THIS node authored).
pub async fn trust(a: &FreshNode, peer_genesis: &[u8], peer_sk: &SigningKey) {
    let pubkey_hex = key_hex(peer_sk);
    let bundle = PairingBundle {
        node_id_hex: node_id_of(peer_genesis),
        fingerprint: cairn_event::short_fingerprint(&pubkey_hex).unwrap(),
        pubkey_hex,
        address: "127.0.0.1:7998".into(),
        // Runtime-derived (house rule 6a): `nonce` is a CodeQL sink NAME, and this field is inert
        // (#530) — nothing reads it back — so any per-run value will do.
        nonce: format!("fixture-{}", Uuid::now_v7()),
        hlc: Hlc {
            wall: 0,
            counter: 0,
            node_origin: a.node_id.clone(),
        },
    };
    identity::author_peer(&a.db, &a.sk, &key_hex(&a.sk), &a.node_id, &bundle, Some("peer"))
        .await
        .unwrap();
}

// ---------------------------------------------------------------------------
// The single-DB self-pull (moved from node_quarantine.rs).
// ---------------------------------------------------------------------------

/// <paste the doc comment of `SelfNode` from node_quarantine.rs, if any>
pub struct SelfNode {
    pub a: Client,
    pub addr: SocketAddr,
    pub serve: tokio::task::JoinHandle<anyhow::Result<()>>,
    pub sk: SigningKey,
    _tmp: tempfile::TempDir,
}

// <paste `self_node` from node_quarantine.rs as `pub async fn self_node(...)`, body unchanged EXCEPT
//  its PairingBundle's `nonce: "n".into(),` becomes:
//      // Runtime-derived (house rule 6a) — see `trust` above.
//      nonce: format!("fixture-{}", Uuid::now_v7()),
// >

/// How many rows the node quarantine pen holds (acked or not).
pub async fn pen_count(a: &Client) -> i64 {
    a.query_one("SELECT count(*) FROM node_event_quarantine", &[])
        .await
        .unwrap()
        .get(0)
}

/// Raw-insert `signed` as a row A will SERVE, under a fresh random `node_event_id` — the stand-in
/// for a peer serving these bytes. Owner privilege bypasses the grant floor, as in every #111 test.
///
/// The row's table id deliberately differs from the `event_id` inside its signed body: the puller
/// re-applies the BODY, so this is how a rival under an id A already holds can be served beside the
/// genuine row (the table cannot hold two rows under one `node_event_id`). Returns the row's `seq`.
pub async fn serve_raw(a: &Client, signed: &[u8]) -> i64 {
    a.query_one(
        "INSERT INTO node_event
             (node_event_id, op, author_node_id, subject_node_id, signer_key_id,
              hlc_wall, hlc_counter, node_origin, signed_bytes, content_address)
         VALUES (gen_random_uuid(), 'peer', '\\x00', '\\x00', 'k', 1, 0, 'n',
                 $1::bytea, '\\x1220'::bytea || digest($1::bytea, 'sha256'))
         RETURNING seq",
        &[&signed],
    )
    .await
    .expect("owner may seed a served row")
    .get(0)
}
```

- [ ] **Step 4: Point `node_quarantine.rs` at the kit**

Delete `SelfNode`, `self_node` and `pen_count` from `crates/cairn-node/tests/node_quarantine.rs`, and
below its header add:

```rust
#[path = "common/node_plane_kit.rs"]
mod node_plane_kit;

use node_plane_kit::{pen_count, self_node};
```

Leave `cs`, `BAD`, `insert_corrupt_node_event` and every test body unchanged. Then remove any `use`
that became unused (`keystore` and `std::net::SocketAddr` will; check the compiler).

- [ ] **Step 5: Verify both suites are unchanged in behaviour**

Run: `cargo test -p cairn-node --test late_custody_guards --test node_quarantine`
Expected: every test PASSES, the same test names and counts as before, **zero warnings**. (A warning
here is an unused import from Step 4.)

- [ ] **Step 6: Commit**

```bash
cargo fmt --all
git add crates/cairn-node/tests/common/sql_text.rs crates/cairn-node/tests/common/node_plane_kit.rs \
        crates/cairn-node/tests/late_custody_guards.rs crates/cairn-node/tests/node_quarantine.rs
git commit -m "test(#619): two shared kits — the SQL comment stripper and the node-plane fixtures

Moved, not copied: late_custody_guards.rs's stripper and node_quarantine.rs's
self-pull fixture each get a second user in #619, and one invariant written
twice is the drift #608 was made of. The moved PairingBundle nonce becomes
runtime-derived (house rule 6a).

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

### Task 2: `submit_node_event` refuses a substitution (the local door)

**Files:**
- Create: `crates/cairn-node/tests/node_plane_one_event_id_one_body.rs`
- Modify: `db/007_node_federation.sql` — `submit_node_event` only

**Interfaces:** Consumes Task 1's `node_plane_kit`. Produces the door behaviour Task 5's pull test relies on.

- [ ] **Step 1: Write the failing tests**

Create `crates/cairn-node/tests/node_plane_one_event_id_one_body.rs`:

```rust
//! #619 — one `node_event_id`, one body, through the two LIVE node-plane doors (db/007).
//!
//! The sibling of `restore_one_node_event_id_one_body.rs` (db/009, #615). A SUBSTITUTION is a
//! second, different event filed under an `event_id` the log already holds. Every door inserts
//! `ON CONFLICT (node_event_id) DO NOTHING` so a repeat of the SAME event stays a silent no-op —
//! set-union, principle 1 — and a substitution looks exactly like that no-op from the INSERT's side.
//! Before #619, db/007 compared nothing, so the rival vanished and the door returned the id as if
//! it had succeeded.
//!
//! What that cost, per door (stated precisely — ADR-0073 corrects #619's own "A keeps trusting C",
//! which cannot happen because `trust_peer` reads only events THIS node authored):
//!
//! * `submit_node_event` (local): a revocation this node authors under an id it already holds is
//!   dropped, and the node keeps trusting a peer it revoked — #615's shape, on the door that
//!   authors peering. Reaching it needs this node's signing key.
//! * `apply_remote_node_event` (the federation admission gate): a peer's rival is dropped, the
//!   puller counts it admitted and advances past it, and the two nodes hold different bytes under
//!   one id forever. A dropped rival GENESIS is the sharpest case: that peer's key then never
//!   resolves here, and every event it authors is refused.
//!
//! Every guarded ARM gets its own rival case, because the guard sits once in a shared tail and a
//! later edit could route one arm around it. Every door also gets an IDEMPOTENCE case — the same
//! event twice must still succeed — because a guard that refused a repeat would break set-union
//! itself; those cases pass before the guard exists and are what catch a guard moved ABOVE the
//! branch (where nothing is held yet, so it would refuse every clean write).

#[path = "common/node_plane_kit.rs"]
mod node_plane_kit;

use cairn_node::db;
use node_plane_kit::{
    address_of, call, cs, fresh_node, held_address, node_id_hex, peer_event, supersede_event,
    SENTENCE,
};
use uuid::Uuid;

/// THE LOCAL CASE. A revocation this node authors under an id it already holds must be refused
/// LOUDLY. Before #619 the door returned success, the revocation vanished, and `trust_peer` went
/// on showing the peer as active with nothing telling anyone.
#[tokio::test]
async fn the_local_door_refuses_a_rival_revocation_instead_of_dropping_it() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let a = fresh_node(&base).await;

    let contested = Uuid::now_v7();
    let peer = node_id_hex(1);
    let added = peer_event(&a.sk, "peer.added", contested, &peer);
    let revoked = peer_event(&a.sk, "peer.revoked", contested, &peer);

    call(&a.db, "submit_node_event", &added)
        .await
        .expect("the first event under a fresh id is admitted");
    let msg = call(&a.db, "submit_node_event", &revoked).await.expect_err(
        "a SECOND, different node event under a held id must be REFUSED. Returning success here \
         is #619: the revocation vanishes and this node keeps trusting the peer, in silence",
    );
    assert!(
        msg.contains("submit_node_event") && msg.contains(SENTENCE),
        "the refusal must name its door and say it was a substitution, so the operator can tell \
         it from a signature or trust failure; got: {msg}"
    );
    assert_eq!(
        held_address(&a.db, contested).await,
        Some(address_of(&added)),
        "the event first written under the id is untouched"
    );
}

/// The supersede arm reaches the same tail by a different branch, so it gets its own rival.
#[tokio::test]
async fn the_local_door_refuses_a_rival_supersede() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let a = fresh_node(&base).await;

    let contested = Uuid::now_v7();
    let held = supersede_event(&a.sk, contested, &node_id_hex(2));
    let rival = supersede_event(&a.sk, contested, &node_id_hex(3));

    call(&a.db, "submit_node_event", &held)
        .await
        .expect("the first supersede under a fresh id is admitted");
    let msg = call(&a.db, "submit_node_event", &rival)
        .await
        .expect_err("a rival supersede under a held id must be refused, not dropped");
    assert!(
        msg.contains("submit_node_event") && msg.contains(SENTENCE),
        "got: {msg}"
    );
    assert_eq!(held_address(&a.db, contested).await, Some(address_of(&held)));
}

/// A REPEAT is not a substitution. Green before the guard exists and green after — its job is to
/// fail if the guard is ever placed where it would refuse a clean or repeated write.
#[tokio::test]
async fn the_local_door_still_admits_the_same_event_twice() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let a = fresh_node(&base).await;

    let peer = peer_event(&a.sk, "peer.added", Uuid::now_v7(), &node_id_hex(4));
    let supersede = supersede_event(&a.sk, Uuid::now_v7(), &node_id_hex(5));
    for pass in 1..=2 {
        for ev in [&peer, &supersede] {
            call(&a.db, "submit_node_event", ev)
                .await
                .unwrap_or_else(|e| {
                    panic!("pass {pass}: the SAME event twice must stay a no-op, never raise: {e}")
                });
        }
    }
}
```

- [ ] **Step 2: Run to verify the two rival tests fail and the repeat test passes**

Run: `cargo test -p cairn-node --test node_plane_one_event_id_one_body`
Expected: `the_local_door_refuses_a_rival_revocation_instead_of_dropping_it` and
`the_local_door_refuses_a_rival_supersede` FAIL at their `expect_err` ("a SECOND, different node
event under a held id must be REFUSED…" — the door returned `Ok`). `the_local_door_still_admits_the_same_event_twice`
PASSES. If a rival test fails anywhere else (a setup `expect`), the fixture is wrong — fix that first.

- [ ] **Step 3: Restructure `submit_node_event` to one tail with the guard**

In `db/007_node_federation.sql`, in `submit_node_event`:

1. Add `v_found BYTEA;` to its `DECLARE` block (a new line after the existing two lines).
2. Replace everything from the line `    IF v_op = 'supersede' THEN` down to (not including) the function's
   closing `END;` with the block below. The genesis arm above it and the two `IF v_local_node IS
   NULL` / `IF v_signer <> v_local_key` checks are UNCHANGED. The supersede comment block that sits
   above `IF v_op = 'supersede'` stays where it is.

```sql
    IF v_op = 'supersede' THEN
        IF v_payload ->> 'superseded_node_id_hex' IS NULL THEN
            RAISE EXCEPTION 'submit_node_event: node.superseded missing superseded_node_id_hex in payload';
        END IF;
        INSERT INTO node_event (node_event_id, op, author_node_id, subject_node_id,
            signer_key_id, hlc_wall, hlc_counter, node_origin, signed_bytes, content_address)
        VALUES (v_eid, 'supersede', v_local_node,
            cairn_decode_hex_or_raise('superseded_node_id_hex',
                v_payload ->> 'superseded_node_id_hex', 'submit_node_event'),
            v_signer, (b -> 'hlc' ->> 'wall')::bigint, (b -> 'hlc' ->> 'counter')::int,
            b -> 'hlc' ->> 'node_origin', p_signed, v_ca)
        ON CONFLICT (node_event_id) DO NOTHING;
    ELSE
        -- subject_node_id is NOT NULL; a missing peer_node_id_hex would otherwise surface
        -- as an opaque constraint error rather than a legible rejection. The sibling case —
        -- present but MALFORMED — is caught inside cairn_decode_hex_or_raise below (issue
        -- #228). This guard is kept rather than folded into the helper because it names
        -- v_type, which the helper cannot see.
        IF v_payload ->> 'peer_node_id_hex' IS NULL THEN
            RAISE EXCEPTION 'submit_node_event: % missing peer_node_id_hex in payload', v_type;
        END IF;

        INSERT INTO node_event (node_event_id, op, author_node_id, subject_node_id,
            signer_key_id, peer_pubkey, fingerprint, role, scope_hint, target_event_id,
            hlc_wall, hlc_counter, node_origin, signed_bytes, content_address)
        VALUES (v_eid, v_op, v_local_node,
            cairn_decode_hex_or_raise('peer_node_id_hex',
                v_payload ->> 'peer_node_id_hex', 'submit_node_event'),
            v_signer, v_payload ->> 'peer_pubkey', v_payload ->> 'fingerprint',
            v_payload ->> 'role', v_payload ->> 'scope_hint',
            NULLIF(v_payload ->> 'target_event_id','')::uuid,
            (b -> 'hlc' ->> 'wall')::bigint, (b -> 'hlc' ->> 'counter')::int,
            b -> 'hlc' ->> 'node_origin', p_signed, v_ca)
        ON CONFLICT (node_event_id) DO NOTHING;
    END IF;

    -- SUBSTITUTION REFUSAL (#619, ADR-0073). Both arms above insert ON CONFLICT DO NOTHING, which
    -- is right for a REPEAT of the same event (set-union) and silently wrong for a DIFFERENT event
    -- under an id already held: the rival vanished and this door returned the id as if it had
    -- succeeded — for a peer.revoked, the node kept trusting a peer it had revoked. The comparison
    -- is the shared cairn_refuse_substitution (db/053, IS DISTINCT FROM), never an inline copy
    -- (trap 12; substitution_guard_is_single_source.rs).
    --
    -- Two placement rules, each of which a later edit might "tidy" away:
    --   * AFTER the IF/ELSE, never above it. Above the branch nothing is held yet, v_found is
    --     NULL, and IS DISTINCT FROM refuses — every clean write would be refused.
    --   * The read is UNCONDITIONAL — no GET DIAGNOSTICS ROW_COUNT. The node plane carries tens
    --     of events, and a ROW_COUNT check is only correct while each INSERT stays the last
    --     statement of its arm; a later edit would disarm it silently (db/009's rule, trap 12).
    -- The genesis arm above needs neither: it has no ON CONFLICT, so a colliding id raises.
    SELECT content_address INTO v_found FROM node_event WHERE node_event_id = v_eid;
    PERFORM cairn_refuse_substitution(v_found, v_ca, v_eid, 'submit_node_event');
    RETURN v_eid;
```

Check: `grep -c "cairn_decode_hex_or_raise(" db/007_node_federation.sql` is still what it was before
this step (the pin in `hex_decode_helper.rs` counts db/007's calls; this step only re-indents two).

- [ ] **Step 4: Run to verify all three pass**

Run: `cargo test -p cairn-node --test node_plane_one_event_id_one_body --test hex_decode_helper --test federation`
Expected: all PASS. (`federation` is the existing db/007 suite — the restructure must not move any
existing behaviour.)

- [ ] **Step 5: Commit**

```bash
git add db/007_node_federation.sql crates/cairn-node/tests/node_plane_one_event_id_one_body.rs
git commit -m "feat(#619): submit_node_event refuses a substitution instead of dropping it

A peer.revoked authored under an id this node already holds used to vanish
behind ON CONFLICT DO NOTHING while the door returned success, so the node
kept trusting the peer. One shared tail after the IF/ELSE reads what is
held and calls cairn_refuse_substitution (db/053), db/009's shape.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

### Task 3: `apply_remote_node_event` refuses a substitution, and the inventory becomes a catalogue rule

**Files:**
- Modify: `crates/cairn-node/tests/node_plane_one_event_id_one_body.rs` (append the remote-door cases)
- Create: `crates/cairn-node/tests/substitution_guard_covers_every_writer.rs`
- Modify: `db/007_node_federation.sql` — `apply_remote_node_event` only
- Modify: `crates/cairn-node/tests/substitution_guard_is_single_source.rs` (remove the hand-written inventory)
- Modify: `crates/cairn-node/tests/hlc_merge_helper.rs` (pin 3 → 1, and its prose)
- Modify: `db/001_envelope.sql` (one comment sentence)

**Interfaces:** Consumes `node_plane_kit::{trust, genesis_event, node_id_of}` and `sql_text::normalised`.

- [ ] **Step 1: Append the failing remote-door tests**

Append to `crates/cairn-node/tests/node_plane_one_event_id_one_body.rs` (and extend its `use
node_plane_kit::{…}` list with `genesis_event, trust, FreshNode`; add `use cairn_event::{generate_key,
SigningKey};`):

```rust
// ---------------------------------------------------------------------------
// apply_remote_node_event — the federation admission gate.
// ---------------------------------------------------------------------------

/// Node A, plus a peer B that A trusts and whose genesis A has admitted through the remote door.
/// That is the minimum for B's later events to REACH the guard: without it they are refused
/// earlier, by the deny-all trust checks, and a rival test would pass for the wrong reason.
/// Returns B's key and B's genesis id (the enroll-arm case reuses that id).
async fn a_with_trusted_b(base: &str) -> (FreshNode, SigningKey, Uuid) {
    let a = fresh_node(base).await;
    let (b_sk, _) = generate_key().unwrap();
    let b_genesis_id = Uuid::now_v7();
    let b_genesis = genesis_event(&b_sk, b_genesis_id, "B");
    trust(&a, &b_genesis, &b_sk).await;
    call(&a.db, "apply_remote_node_event", &b_genesis)
        .await
        .expect("A admits the genesis of a peer it trusts");
    (a, b_sk, b_genesis_id)
}

/// A trusted peer serves a rival `peer.revoked` under the id of its own `peer.added`.
#[tokio::test]
async fn the_admission_gate_refuses_a_rival_peer_event() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let (a, b_sk, _) = a_with_trusted_b(&base).await;

    let contested = Uuid::now_v7();
    let subject = node_id_hex(6);
    let held = peer_event(&b_sk, "peer.added", contested, &subject);
    let rival = peer_event(&b_sk, "peer.revoked", contested, &subject);

    call(&a.db, "apply_remote_node_event", &held)
        .await
        .expect("B's first event under a fresh id is admitted");
    let msg = call(&a.db, "apply_remote_node_event", &rival).await.expect_err(
        "a peer's SECOND, different event under a held id must be refused. Admitting it silently \
         is #619: A and B then hold different bytes under one id, forever, and nothing says so",
    );
    assert!(
        msg.contains("apply_remote_node_event") && msg.contains(SENTENCE),
        "got: {msg}"
    );
    assert_eq!(held_address(&a.db, contested).await, Some(address_of(&held)));
}

/// The supersede arm, through the remote door.
#[tokio::test]
async fn the_admission_gate_refuses_a_rival_supersede() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let (a, b_sk, _) = a_with_trusted_b(&base).await;

    let contested = Uuid::now_v7();
    let held = supersede_event(&b_sk, contested, &node_id_hex(7));
    let rival = supersede_event(&b_sk, contested, &node_id_hex(8));

    call(&a.db, "apply_remote_node_event", &held)
        .await
        .expect("B's first supersede under a fresh id is admitted");
    let msg = call(&a.db, "apply_remote_node_event", &rival)
        .await
        .expect_err("a rival supersede under a held id must be refused");
    assert!(
        msg.contains("apply_remote_node_event") && msg.contains(SENTENCE),
        "got: {msg}"
    );
    assert_eq!(held_address(&a.db, contested).await, Some(address_of(&held)));
}

/// THE SHARPEST REMOTE CASE: a rival GENESIS. C is trusted too, and its genesis reuses B's genesis
/// id. Dropped silently, C's genesis would never be stored, `node_current` would never resolve C's
/// key, and every event C authors would be refused as "author key maps to no known node" —
/// logged as recoverable, which it never would be.
#[tokio::test]
async fn the_admission_gate_refuses_a_rival_genesis() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let (a, _b_sk, b_genesis_id) = a_with_trusted_b(&base).await;

    let (c_sk, _) = generate_key().unwrap();
    let c_genesis = genesis_event(&c_sk, b_genesis_id, "C");
    trust(&a, &c_genesis, &c_sk).await;

    let msg = call(&a.db, "apply_remote_node_event", &c_genesis)
        .await
        .expect_err("a trusted node's genesis under an id already held must be refused");
    assert!(
        msg.contains("apply_remote_node_event") && msg.contains(SENTENCE),
        "got: {msg}"
    );
}

/// A REPEAT through the remote door, in every arm, is still admitted — set-union survives the
/// guard. (Green before the guard exists; it catches a guard moved above the branch.)
#[tokio::test]
async fn the_admission_gate_still_admits_the_same_event_twice() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let a = fresh_node(&base).await;
    let (b_sk, _) = generate_key().unwrap();
    let b_genesis = genesis_event(&b_sk, Uuid::now_v7(), "B");
    trust(&a, &b_genesis, &b_sk).await;
    let peer = peer_event(&b_sk, "peer.added", Uuid::now_v7(), &node_id_hex(9));
    let supersede = supersede_event(&b_sk, Uuid::now_v7(), &node_id_hex(10));

    for pass in 1..=2 {
        for ev in [&b_genesis, &peer, &supersede] {
            call(&a.db, "apply_remote_node_event", ev)
                .await
                .unwrap_or_else(|e| {
                    panic!("pass {pass}: the SAME event twice must stay a no-op, never raise: {e}")
                });
        }
    }
}
```

- [ ] **Step 2: Write the failing catalogue rule**

Create `crates/cairn-node/tests/substitution_guard_covers_every_writer.rs`:

```rust
//! #619 / ADR-0073 — every function that writes an event log calls the substitution refusal.
//!
//! Checked over the DATABASE CATALOGUE (`pg_proc`, what actually runs), like
//! `late_custody_guards.rs` rule 2.
//!
//! # Why a catalogue rule and not a list
//!
//! Until #619 the inventory of guarded doors was a hand-written list in
//! `substitution_guard_is_single_source.rs`, and it was WRONG: #615 and ADR-0072's first draft said
//! "two of the three write doors" refuse a substitution, counting the two `event_log` doors beside
//! the restore door and omitting `node_event`'s other two writers entirely — `submit_node_event` and
//! `apply_remote_node_event`, five unguarded sites, one of them the live federation admission gate.
//! A list says what its author believed. This rule derives the writer set from the functions that
//! exist, so a writer nobody thought of is found rather than trusted.
//!
//! The derived set is still PINNED by name, so a sixth writer fails here and becomes a decision —
//! give it the call (and say why) — rather than a drift.
//!
//! # Honest residuals
//!
//! * It reads a function's OWN body. A write through a helper, a `MERGE`, or a dynamic `EXECUTE
//!   format(...)` is not recognised. None exists; review a new one by hand.
//! * It covers the two EVENT logs, `event_log` and `node_event`. The actor registry's `actor_event`
//!   has the same silent-discard shape at db/052's door — that is #569, open, and widening this rule
//!   to it would fail today and pull #569 into #619.
//! * Comments are stripped before matching (`common/sql_text.rs`), so prose neither satisfies nor
//!   trips it; the stripper is literal-blind.

#[path = "common/sql_text.rs"]
mod sql_text;

use cairn_node::db;
use sql_text::normalised;

/// The test database, or `None` when `$CAIRN_TEST_PG` is unset — the repo-wide self-skip,
/// policed by `tests/db_gate_actually_ran.rs`.
fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

/// Does this body `INSERT` into one of the two event logs? **Pure.**
///
/// The table name must be followed by a space or `(`, so `node_event_quarantine` (a pen written
/// from Rust, not a door) never matches `node_event`. A `public.`-qualified write counts too.
fn writes_an_event_log(body: &str) -> bool {
    let n = normalised(body);
    ["EVENT_LOG", "NODE_EVENT"].iter().any(|table| {
        [
            format!("INSERT INTO {table} "),
            format!("INSERT INTO {table}("),
            format!("INSERT INTO PUBLIC.{table} "),
            format!("INSERT INTO PUBLIC.{table}("),
        ]
        .iter()
        .any(|needle| n.contains(needle.as_str()))
    })
}

/// Does this body call the shared refusal? **Pure.**
fn refuses_substitution(body: &str) -> bool {
    normalised(body).contains("CAIRN_REFUSE_SUBSTITUTION(")
}

#[test]
fn the_predicates_read_code_and_ignore_prose() {
    assert!(writes_an_event_log(
        "INSERT INTO node_event (node_event_id) VALUES (x)"
    ));
    assert!(writes_an_event_log(
        "insert into\n   event_log\n(event_id) values (x)"
    ));
    assert!(writes_an_event_log(
        "INSERT INTO public.event_log (event_id) VALUES (x)"
    ));
    assert!(!writes_an_event_log(
        "INSERT INTO node_event_quarantine (content_digest) VALUES (x)"
    ));
    assert!(!writes_an_event_log(
        "-- INSERT INTO node_event happens in the door\nRETURN;"
    ));
    assert!(refuses_substitution(
        "PERFORM cairn_refuse_substitution(v_found, v_ca, v_eid, 'd');"
    ));
    assert!(!refuses_substitution(
        "-- cairn_refuse_substitution(v_found) is called by the door\nRETURN;"
    ));
    assert!(!refuses_substitution(
        "/* PERFORM cairn_refuse_substitution(a, b, c, d); */ RETURN;"
    ));
}

#[tokio::test]
async fn every_event_log_writer_refuses_a_substitution() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let rows = c
        .query(
            "SELECT p.proname, p.prosrc FROM pg_proc p \
               JOIN pg_namespace n ON n.oid = p.pronamespace \
               JOIN pg_language l ON l.oid = p.prolang \
              WHERE n.nspname = 'public' AND l.lanname IN ('plpgsql', 'sql')",
            &[],
        )
        .await
        .unwrap();

    let mut writers: Vec<(String, bool)> = rows
        .iter()
        .filter(|r| writes_an_event_log(r.get::<_, &str>(1)))
        .map(|r| (r.get(0), refuses_substitution(r.get::<_, &str>(1))))
        .collect();
    writers.sort();

    // POSITIVE CONTROL and PIN in one: the rule must SEE the five writers it exists for (a rule
    // that sees none passes over anything — #586's lesson), and a sixth is a decision.
    assert_eq!(
        writers.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(),
        vec![
            "apply_remote_event",
            "apply_remote_node_event",
            "restore_node_event",
            "submit_event",
            "submit_node_event",
        ],
        "the event-log writers are these five doors. A sixth is a DECISION: give it the \
         cairn_refuse_substitution call (ADR-0072/0073) and add it here, saying why"
    );
    let unguarded: Vec<&str> = writers
        .iter()
        .filter(|(_, guarded)| !guarded)
        .map(|(n, _)| n.as_str())
        .collect();
    assert!(
        unguarded.is_empty(),
        "these functions write an event log but never call cairn_refuse_substitution, so a \
         second, different event under a held id vanishes behind ON CONFLICT DO NOTHING (#619): \
         {unguarded:?}"
    );
}
```

- [ ] **Step 3: Run both to verify they fail for the right reason**

Run: `cargo test -p cairn-node --test node_plane_one_event_id_one_body --test substitution_guard_covers_every_writer`
Expected:
- the three `the_admission_gate_refuses_a_rival_*` tests FAIL at their `expect_err` (the door returned `Ok`);
- `the_admission_gate_still_admits_the_same_event_twice` and the three local-door tests PASS;
- `the_predicates_read_code_and_ignore_prose` PASSES;
- `every_event_log_writer_refuses_a_substitution` FAILS at the SECOND assertion, naming exactly
  `["apply_remote_node_event"]` (Task 2 already guarded `submit_node_event`). If it fails at the
  FIRST assertion instead, the writer set differs from the pin — stop and investigate; do not edit
  the pin to match.

- [ ] **Step 4: Restructure `apply_remote_node_event` to one tail with the guard**

In `db/007_node_federation.sql`, in `apply_remote_node_event`:

1. Add `v_found BYTEA;` to its `DECLARE` block.
2. Replace everything from the line `    IF v_op = 'enroll' THEN` (inside THIS function — the one whose
   next line is `-- The genesis must match an active, out-of-band-confirmed peer: its`) down to (not
   including) the function's closing `END;` with:

```sql
    IF v_op = 'enroll' THEN
        -- The genesis must match an active, out-of-band-confirmed peer: its
        -- content-address is the node_id we trust, and its key is the pubkey we pinned.
        IF NOT EXISTS (SELECT 1 FROM trust_peer
                       WHERE peer_node_id = v_ca AND status = 'active' AND peer_pubkey = v_signer) THEN
            RAISE EXCEPTION 'apply_remote_node_event: genesis from an un-trusted or mismatched node (deny-all default)';
        END IF;
        INSERT INTO node_event (node_event_id, op, author_node_id, subject_node_id,
            signer_key_id, hlc_wall, hlc_counter, node_origin, signed_bytes, content_address)
        VALUES (v_eid, 'enroll', v_ca, v_ca, v_signer,
            (b -> 'hlc' ->> 'wall')::bigint, (b -> 'hlc' ->> 'counter')::int,
            b -> 'hlc' ->> 'node_origin', p_signed, v_ca)
        ON CONFLICT (node_event_id) DO NOTHING;
    ELSE
        -- peer/revoke/supersede: the author must be a currently-trusted peer (resolved by key).
        SELECT node_id INTO v_author_node FROM node_current WHERE signer_key_id = v_signer;
        IF v_author_node IS NULL THEN
            RAISE EXCEPTION 'apply_remote_node_event: author key % maps to no known node', v_signer;
        END IF;
        IF NOT EXISTS (SELECT 1 FROM trust_peer WHERE peer_node_id = v_author_node AND status = 'active') THEN
            RAISE EXCEPTION 'apply_remote_node_event: author % is not an active peer (deny-all)', encode(v_author_node,'hex');
        END IF;

        -- <keep, verbatim, the existing multi-line comment that begins
        --  "-- supersede (issue #201, ADR-0026 slice C): a restored node's lineage claim"
        --  and ends "-- resolution nor peer trust; it is an attributable, signed claim (principle 2).">
        IF v_op = 'supersede' THEN
            -- Mirror the local door's legible guard: name the missing field, never store
            -- a NULL/garbage subject.
            IF v_payload ->> 'superseded_node_id_hex' IS NULL THEN
                RAISE EXCEPTION 'apply_remote_node_event: node.superseded from % missing superseded_node_id_hex in payload', encode(v_author_node,'hex');
            END IF;
            INSERT INTO node_event (node_event_id, op, author_node_id, subject_node_id,
                signer_key_id, hlc_wall, hlc_counter, node_origin, signed_bytes, content_address)
            VALUES (v_eid, 'supersede', v_author_node,
                cairn_decode_hex_or_raise('superseded_node_id_hex',
                    v_payload ->> 'superseded_node_id_hex', 'apply_remote_node_event'),
                v_signer, (b -> 'hlc' ->> 'wall')::bigint, (b -> 'hlc' ->> 'counter')::int,
                b -> 'hlc' ->> 'node_origin', p_signed, v_ca)
            ON CONFLICT (node_event_id) DO NOTHING;
        ELSE
            -- <keep, verbatim, the existing comment that begins
            --  "-- Mirror the local door's legible guard: a trusted-but-malformed peer event"
            --  and ends "-- what tells the operator which peer to go and fix, and the helper cannot see it.">
            IF v_payload ->> 'peer_node_id_hex' IS NULL THEN
                RAISE EXCEPTION 'apply_remote_node_event: % from % missing peer_node_id_hex in payload', v_type, encode(v_author_node,'hex');
            END IF;
            INSERT INTO node_event (node_event_id, op, author_node_id, subject_node_id,
                signer_key_id, peer_pubkey, fingerprint, role, scope_hint, target_event_id,
                hlc_wall, hlc_counter, node_origin, signed_bytes, content_address)
            VALUES (v_eid, v_op, v_author_node,
                cairn_decode_hex_or_raise('peer_node_id_hex',
                    v_payload ->> 'peer_node_id_hex', 'apply_remote_node_event'),
                v_signer, v_payload ->> 'peer_pubkey', v_payload ->> 'fingerprint',
                v_payload ->> 'role', v_payload ->> 'scope_hint',
                NULLIF(v_payload ->> 'target_event_id','')::uuid,
                (b -> 'hlc' ->> 'wall')::bigint, (b -> 'hlc' ->> 'counter')::int,
                b -> 'hlc' ->> 'node_origin', p_signed, v_ca)
            ON CONFLICT (node_event_id) DO NOTHING;
        END IF;
    END IF;

    -- SUBSTITUTION REFUSAL (#619, ADR-0073) — the federation admission gate's copy of the
    -- submit_node_event tail. Every arm above inserts ON CONFLICT DO NOTHING; without this, a
    -- trusted peer's SECOND, different event under an id already held vanished, the function
    -- returned normally, the puller counted it admitted and advanced past it, and set-union never
    -- re-offered it: two nodes holding different bytes under one id, forever, in silence. A
    -- dropped rival GENESIS is the sharpest case — that peer's key would never resolve here.
    -- The refusal is P0001 like every other (db/001's contract); the node puller tells it from a
    -- routine deny-all by STATE, not by this text (crates/cairn-node/src/sync/substitution.rs).
    -- Same two placement rules as submit_node_event: AFTER the IF/ELSE, and an UNCONDITIONAL read.
    SELECT content_address INTO v_found FROM node_event WHERE node_event_id = v_eid;
    PERFORM cairn_refuse_substitution(v_found, v_ca, v_eid, 'apply_remote_node_event');

    -- Clock never falls behind an event we accepted (HLC invariant A3, mirrors cairn-sync).
    -- The REJECTION above is this door's ceiling; the helper (db/001) is the pure merge. ONE
    -- merge for all three arms since #619 folded them into this tail (it used to be three
    -- copies), and AFTER the guard, so a refused rival never advances this node's clock.
    PERFORM cairn_node_hlc_merge((b -> 'hlc' ->> 'wall')::bigint,
                                 (b -> 'hlc' ->> 'counter')::int);
    RETURN v_eid;
```

Check: `grep -c "cairn_decode_hex_or_raise(" db/007_node_federation.sql` is unchanged from before
this step, and `grep -c "PERFORM cairn_node_hlc_merge(" db/007_node_federation.sql` is now **1**.

- [ ] **Step 5: Move the merge-site pin and the prose that counts sites**

In `crates/cairn-node/tests/hlc_merge_helper.rs`:

1. In `every_door_still_calls_the_helper`, change
   `("007_node_federation.sql", 3), // apply_remote_node_event: enroll, supersede, peer/revoke` to
   `("007_node_federation.sql", 1), // apply_remote_node_event: ONE shared tail for all three arms (#619)`.
2. In that test's doc comment, replace
   `/// every \`PERFORM cairn_node_hlc_merge(...)\` deleted, including all five at once. The` with
   `/// every \`PERFORM cairn_node_hlc_merge(...)\` deleted, including all of them at once. The`,
   and replace the three lines
   ```
   /// exist for three of the five sites (see the header): the supersede arm, the
   /// peer/revoke arm and `restore_node_event` have no assertion anywhere in the tree that
   /// the clock advanced, so their merge could be dropped with the whole suite green.
   ```
   with
   ```
   /// exist for every site (see the header): `restore_node_event` has no assertion anywhere
   /// in the tree that the clock advanced, so its merge could be dropped with the whole
   /// suite green.
   ```
3. In the `//!` header, replace the sentence beginning `forward after admission" assertion exists
   for only two of the five call sites:` through `their merge leaves the entire tree green.` with:
   `forward after admission" assertion exists for two of the three call sites that remain since
   #619 folded db/007's three arms into one shared tail: db/007 (\`hlc_drift.rs\`, through the
   enroll arm, which now shares its merge with the supersede and peer/revoke arms) and db/020
   (\`apply_remote_event.rs\`). \`restore_node_event\` has none — dropping its merge leaves the
   entire tree green.` (The earlier sentence "it was copied verbatim into five places" is HISTORY
   of #227 and stays.)

In `db/001_envelope.sql`, replace
`-- ONE copy, five callers (issue #227). This block used to be pasted verbatim into every` with
`-- ONE copy (issue #227), three callers since #619 folded db/007's three arms into one tail. This block used to be pasted verbatim into every`
(the rest of that comment is history and stays).

- [ ] **Step 6: Retire the hand-written inventory**

In `crates/cairn-node/tests/substitution_guard_is_single_source.rs`, delete the whole
`every_door_this_change_guards_still_calls_the_helper` test (its doc comment included), and in its
place leave this comment:

```rust
// The INVENTORY of guarded doors used to be a hand-written list here
// (`every_door_this_change_guards_still_calls_the_helper`), and it was wrong: it omitted db/007's two
// `node_event` writers, one of them the live federation admission gate (#619). A list says what its
// author believed. The inventory is now DERIVED from the catalogue — every function that writes an
// event log must call the helper — in `substitution_guard_covers_every_writer.rs`. This file keeps
// the other half: nobody DUPLICATES the refusal.
```

- [ ] **Step 7: Run everything this task touched**

Run: `cargo test -p cairn-node --test node_plane_one_event_id_one_body --test substitution_guard_covers_every_writer --test substitution_guard_is_single_source --test hlc_merge_helper --test hex_decode_helper --test hlc_drift --test federation --test node_quarantine --test restore_one_node_event_id_one_body`
Expected: all PASS, zero warnings.

- [ ] **Step 8: Commit**

```bash
cargo fmt --all
git add db/007_node_federation.sql db/001_envelope.sql \
        crates/cairn-node/tests/node_plane_one_event_id_one_body.rs \
        crates/cairn-node/tests/substitution_guard_covers_every_writer.rs \
        crates/cairn-node/tests/substitution_guard_is_single_source.rs \
        crates/cairn-node/tests/hlc_merge_helper.rs
git commit -m "feat(#619): the federation admission gate refuses a substitution

apply_remote_node_event's three arms fold into one tail: guard once after
the IF/ELSE, then one clock merge (3 copies -> 1; hlc_merge_helper's pin
moves with it). A trusted peer's rival under a held id — a rival genesis
worst of all — no longer vanishes behind ON CONFLICT DO NOTHING.

The door inventory stops being a hand-written list: a pg_proc catalogue
rule derives every event-log writer and requires the call. The list was
how #619's census error happened.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

### Task 4: The pure substitution decision

**Files:**
- Create: `crates/cairn-node/src/sync/substitution.rs`
- Modify: `crates/cairn-node/src/sync.rs` (add `pub mod substitution;`)

**Interfaces — Produces:**
- `cairn_node::sync::substitution::substitution_reason(event_id: &str, held: Option<&[u8]>, offered: &[u8]) -> Option<String>` — `Some` iff `held` is `Some` and differs from `offered`; the reason starts with `"substitution:"` and contains the id and both addresses in lowercase hex.
- `cairn_node::sync::substitution::held_content_address(db: &Client, event_id: &str) -> anyhow::Result<Option<Vec<u8>>>`

- [ ] **Step 1: Declare the module and write the failing unit tests**

In `crates/cairn-node/src/sync.rs`, directly after the `use crate::transport::{self, TrustStore};`
line, add:

```rust
/// Is a refused node event a SUBSTITUTION? (#619, ADR-0073) — see the module's own header.
pub mod substitution;
```

Create `crates/cairn-node/src/sync/substitution.rs` with ONLY the tests and a stub, so the tests
compile and fail:

```rust
pub fn substitution_reason(_event_id: &str, _held: Option<&[u8]>, _offered: &[u8]) -> Option<String> {
    None
}

#[cfg(test)]
mod tests {
    use super::substitution_reason;

    /// A content address, derived at runtime (house rule 6a) and discriminated by a `lineage`,
    /// never a seed/salt/nonce (6b): nothing here is cryptographic.
    fn address(lineage: u8) -> Vec<u8> {
        let mut a = vec![0x12, 0x20];
        a.extend((0..32u8).map(|i| i.wrapping_mul(7).wrapping_add(lineage)));
        a
    }

    #[test]
    fn nothing_held_is_not_a_substitution() {
        assert_eq!(substitution_reason("id", None, &address(1)), None);
    }

    #[test]
    fn the_same_event_again_is_not_a_substitution() {
        let a = address(1);
        assert_eq!(
            substitution_reason("id", Some(a.as_slice()), &a),
            None,
            "an idempotent re-offer is set-union working, never a refusal of this kind"
        );
    }

    #[test]
    fn a_different_event_under_a_held_id_is_one_and_the_reason_names_both() {
        let (held, offered) = (address(1), address(2));
        let reason = substitution_reason("0199aa-contested", Some(held.as_slice()), &offered)
            .expect("a different address under a held id IS a substitution");
        assert!(
            reason.starts_with("substitution:"),
            "the pen row's reason must say what KIND of refusal it is: {reason}"
        );
        assert!(reason.contains("0199aa-contested"), "names the id: {reason}");
        assert!(
            reason.contains(&hex::encode(&held)) && reason.contains(&hex::encode(&offered)),
            "names both addresses, so an operator can find both events: {reason}"
        );
    }
}
```

- [ ] **Step 2: Run to verify the third test fails**

Run: `cargo test -p cairn-node --lib sync::substitution`
Expected: `a_different_event_under_a_held_id_is_one_and_the_reason_names_both` FAILS at its
`expect` ("a different address under a held id IS a substitution"); the other two PASS (the stub
returns `None`, which is right for them — their red phase is mutation M6 in Task 6).

- [ ] **Step 3: Write the real module**

Replace the whole file with:

```rust
//! #619 / ADR-0073 — is a refused node event a SUBSTITUTION?
//!
//! # Why the pull loop has to ask
//!
//! The node-plane pull loop (`super::pull_into`) routes a refusal by SQLSTATE. Every floor refusal
//! is a bare `RAISE EXCEPTION` — P0001, which db/001's header makes a CONTRACT — and a P0001 on a
//! verifiable event is skipped-and-advanced, because on the node plane it is almost always
//! SCOPING: an event authored by a node this one does not peer with, which heals on a later full
//! sweep once trust or code arrives. (#268's own comment explains why penning that steady-state
//! traffic would flood the pen.)
//!
//! A substitution breaks that premise. It is a second, DIFFERENT event under an `event_id` this node
//! already holds, and it can never apply here — the id is taken — so "it heals on a later sweep" is
//! false for it. It is also evidence: a peer served two different signed events under one id, which
//! an honest, bug-free peer never does. So it is PENNED — durable, loud until a human acks it.
//!
//! # Why by STATE, and not by SQLSTATE or message text
//!
//! The refusal cannot carry its own code: db/001 forbids `USING ERRCODE`, because both pull loops
//! route on P0001, and a distinct code would turn `cairn-sync`'s clinical pen into a freeze.
//! Matching the door's sentence would make English prose part of the protocol. What IS unambiguous
//! is the table: `node_event` is append-only, so a row holding this id under a different content
//! address is true now and stays true. The question is asked of the table, after the refusal.
//!
//! That also makes the answer independent of WHICH check refused. A rival from an untrusted author
//! is refused by the trust check before the door ever reaches its substitution guard — and it is
//! still a rival under a held id, still never applies, and is still penned.

use tokio_postgres::Client;

use crate::db_diagnosis::LocalDbFault;

/// The reason to pen `offered` as a substitution, or `None` when it is not one. **Pure.**
///
/// * `held` — the content address `node_event` already holds under `event_id`, if any.
/// * `offered` — the content address of the bytes a peer just served (`event_address`).
///
/// `None` when nothing is held — a fresh id is not a substitution, whatever refused it, and the
/// ordinary skip-and-advance applies — or when `held` EQUALS `offered`: the same event again, an
/// idempotent re-offer, which is set-union working. `Some` only for a different event under a
/// held id; the reason starts `substitution:` so a `cairn-node quarantine` reader can tell it from
/// an unverifiable row, and names both addresses so both events can be found.
pub fn substitution_reason(event_id: &str, held: Option<&[u8]>, offered: &[u8]) -> Option<String> {
    let held = held?;
    if held == offered {
        return None;
    }
    Some(format!(
        "substitution: this node already holds event_id {event_id} with different content \
         (held {}, offered {}) — a peer served a second, different event under an id already \
         taken, so it can never apply here. Find out why that peer did, then ack this row",
        hex::encode(held),
        hex::encode(offered)
    ))
}

/// What `node_event` holds under `event_id`: its content address, or `None` when nothing is.
///
/// An `event_id` that is not a UUID cannot be held (the column is `uuid`), so it answers `None`
/// WITHOUT a query. Parsing here rather than casting in SQL is deliberate: a cast would turn a
/// malformed id into a database error, which the caller must treat as a FREEZE — and a refused
/// event with a malformed id (an oversized one, say, refused before the door parsed it) would
/// then wedge the cursor forever.
pub async fn held_content_address(
    db: &Client,
    event_id: &str,
) -> anyhow::Result<Option<Vec<u8>>> {
    let Ok(id) = uuid::Uuid::parse_str(event_id) else {
        return Ok(None);
    };
    let row = db
        .query_opt(
            "SELECT content_address FROM node_event WHERE node_event_id = $1::text::uuid",
            &[&id.to_string()],
        )
        .await
        .map_err(|e| {
            LocalDbFault::new(
                "reading the content address this node holds under an event_id",
                e,
            )
        })?;
    Ok(row.map(|r| r.get(0)))
}

// <keep the #[cfg(test)] mod tests block from Step 1 unchanged>
```

- [ ] **Step 4: Run to verify all three pass**

Run: `cargo test -p cairn-node --lib sync::substitution`
Expected: 3 PASS, zero warnings. (If `held_content_address` warns as unused, that is expected until
Task 5 — but `pub` items in a `pub mod` of a library do not warn; if one does, stop and check the
module is declared `pub`.)

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add crates/cairn-node/src/sync.rs crates/cairn-node/src/sync/substitution.rs
git commit -m "feat(#619): the pure substitution decision, asked of the table

substitution_reason(event_id, held, offered) is Some exactly when a
different event is offered under an id already held. By STATE, never by
SQLSTATE (db/001 fixes every refusal at P0001) or by the door's sentence.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

### Task 5: The puller pens a substitution, and the operator text says so

**Files:**
- Create: `crates/cairn-node/tests/node_substitution_is_penned.rs`
- Modify: `crates/cairn-node/src/sync.rs` (`pen_or_freeze`, the P0001 arm, `pull_into`'s doc, `PullStats` doc, the INTEGRITY line)
- Modify: `crates/cairn-node/src/main.rs` (`Quarantine` help)

**Interfaces:** Consumes Task 4's `substitution::{substitution_reason, held_content_address}` and
Task 1's `node_plane_kit::{self_node, serve_raw, pen_count, peer_event, …}`.

- [ ] **Step 1: Write the failing pull tests**

Create `crates/cairn-node/tests/node_substitution_is_penned.rs`:

```rust
//! #619 / ADR-0073 — a substituted node event arriving over the network is PENNED, not skipped.
//!
//! Since Task 3 the admission gate refuses a rival under a held id, with a P0001 like every other
//! refusal. The node puller's P0001 arm skips-and-advances, because on the node plane a P0001 is
//! almost always SCOPING (an event from a node this one does not peer with) and heals on a later
//! sweep. A substitution never heals — the id is taken — so skipping it would log "recoverable,
//! non-fatal" for something that is neither, and keep no trace of a peer that served two different
//! signed events under one id. The puller asks the table (by STATE — `sync/substitution.rs`) and
//! pens it: durable, loud every cycle until a human acks.
//!
//! These use the single-DB self-pull (`common/node_plane_kit.rs`): node A holds the genuine event,
//! and a raw-inserted served row carries the rival's bytes, so the real `pull_into` streams it,
//! re-applies it through the real gate, and classifies the refusal. The ordinary scoping refusal is
//! pinned as still SKIPPED by `node_quarantine.rs::a_verifiable_but_refused_event_is_skipped_not_penned`.

#[path = "common/node_plane_kit.rs"]
mod node_plane_kit;

use cairn_event::generate_key;
use cairn_node::{db, sync};
use node_plane_kit::{
    address_of, call, cs, held_address, node_id_hex, pen_count, peer_event, self_node, serve_raw,
    SelfNode,
};
use uuid::Uuid;

/// A holds `peer.added` under `contested` (through its own door); `rival` is served under the same
/// id. Returns (held bytes, rival bytes).
async fn hold_then_serve_a_rival(
    n: &SelfNode,
    contested: Uuid,
    rival_signer: &cairn_event::SigningKey,
) -> (Vec<u8>, Vec<u8>) {
    let subject = node_id_hex(1);
    let held = peer_event(&n.sk, "peer.added", contested, &subject);
    let rival = peer_event(rival_signer, "peer.revoked", contested, &subject);
    call(&n.a, "submit_node_event", &held)
        .await
        .expect("A holds the genuine event");
    serve_raw(&n.a, &rival).await;
    (held, rival)
}

async fn full_pull(base: &str, n: &SelfNode) -> sync::PullStats {
    let cfg = sync::client_config(base, &n.sk, sync::trust_store_from_db(&n.a).await.unwrap())
        .await
        .unwrap();
    sync::pull_once(n.addr, cfg, true).await.unwrap()
}

async fn pen_reason(n: &SelfNode, rival: &[u8]) -> String {
    n.a.query_one(
        "SELECT reason FROM node_event_quarantine WHERE content_digest = $1",
        &[&address_of(rival)],
    )
    .await
    .expect("the rival has a pen row")
    .get(0)
}

#[tokio::test]
async fn a_rival_under_a_held_id_is_penned_not_skipped() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let n = self_node(&base, "127.0.0.1:7951").await;
    let contested = Uuid::now_v7();
    let signer = n.sk.clone();
    let (held, rival) = hold_then_serve_a_rival(&n, contested, &signer).await;

    let s = full_pull(&base, &n).await;

    assert_eq!(
        s.quarantined, 1,
        "the rival is PENNED — skipping it would file a substitution under 'self-healing', which \
         it can never be"
    );
    assert_eq!(
        s.rejected, 0,
        "nothing else in the stream is refused, and the rival is not counted as a skip"
    );
    assert!(s.pending >= 1, "an unacked substitution makes the pull LOUD");
    let reason = pen_reason(&n, &rival).await;
    assert!(
        reason.starts_with("substitution:") && reason.contains(&contested.to_string()),
        "the pen row says what it is and which id: {reason}"
    );
    assert_eq!(
        held_address(&n.a, contested).await,
        Some(address_of(&held)),
        "the genuine event is untouched"
    );
    n.serve.abort();
}

/// An acked substitution stays quiet: the row keeps its ack through every re-offer (the pen
/// dedupes onto it), so a human's decision is not undone by the next full sweep.
#[tokio::test]
async fn an_acked_substitution_stays_quiet_on_reoffer() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let n = self_node(&base, "127.0.0.1:7952").await;
    let signer = n.sk.clone();
    let (_held, rival) = hold_then_serve_a_rival(&n, Uuid::now_v7(), &signer).await;

    let first = full_pull(&base, &n).await;
    assert_eq!(first.quarantined, 1, "penned on the first sweep");
    let acked = sync::ack_node_quarantine(&n.a, &hex::encode(address_of(&rival)))
        .await
        .unwrap();
    assert_eq!(acked, 1, "the ack found the substitution's row");

    let second = full_pull(&base, &n).await;
    assert_eq!(second.pending, 0, "an acked substitution no longer makes the pull loud");
    assert_eq!(pen_count(&n.a).await, 1, "no second row: the re-offer deduped onto the first");
    let still_acked: bool = n
        .a
        .query_one(
            "SELECT acked FROM node_event_quarantine WHERE content_digest = $1",
            &[&address_of(&rival)],
        )
        .await
        .unwrap()
        .get(0);
    assert!(still_acked, "the re-offer must not un-ack a human decision");
    n.serve.abort();
}

/// "Whichever check refused it." A rival signed by a key A does not trust is refused by the
/// author check BEFORE the door reaches its substitution guard — and it is still a rival under a
/// held id, so it is still penned, never filed under self-healing.
#[tokio::test]
async fn a_rival_refused_by_an_earlier_check_is_still_penned() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let n = self_node(&base, "127.0.0.1:7953").await;
    let (stranger, _) = generate_key().unwrap();
    let contested = Uuid::now_v7();
    let (_held, rival) = hold_then_serve_a_rival(&n, contested, &stranger).await;

    let s = full_pull(&base, &n).await;

    assert_eq!(s.quarantined, 1, "a rival under a held id is penned whatever refused it");
    assert_eq!(s.rejected, 0, "and is not filed under self-healing");
    assert!(pen_reason(&n, &rival).await.starts_with("substitution:"));
    n.serve.abort();
}
```

Note: `SelfNode.sk` is a `SigningKey`; `.clone()` is used because `hold_then_serve_a_rival` borrows
`n` and the signer at once. If `SigningKey` is not `Clone` in this version, pass `&n.sk` directly
and drop the `signer` binding — the borrow checker allows two shared borrows.

- [ ] **Step 2: Run to verify all three fail at the pen assertion**

Run: `cargo test -p cairn-node --test node_substitution_is_penned`
Expected: all three FAIL at their first `assert_eq!(…quarantined, 1…)` with `left: 0, right: 1` —
the rival is refused (Task 3) and today's P0001 arm SKIPS it (`rejected` is 1). A failure anywhere
earlier (the `expect("A holds the genuine event")`, a TLS error) is a fixture problem — fix that first.

- [ ] **Step 3: Extract the pen handling into `pen_or_freeze`**

In `crates/cairn-node/src/sync.rs`, directly after `quarantine_node_event`, add:

```rust
/// What happened when the pull loop tried to pen one refused event.
enum PenOutcome {
    /// Penned, or its existing row bumped: durably held, so the cursor may advance past it.
    Penned,
    /// Not penned — the pen is at quota, or the write failed. The cursor must FREEZE below it:
    /// advancing past a refusal nothing holds would lose it (delayed, never lost).
    Frozen,
}

/// Pen one refused event, or report that the cursor must freeze — the pen's three outcomes,
/// handled once for both arms that pen (unverifiable bytes, #111; a substitution, #619).
async fn pen_or_freeze(
    db: &Client,
    peer_key: &str,
    signed: &[u8],
    digest: &[u8],
    seq: i64,
    reason: &str,
) -> PenOutcome {
    match quarantine_node_event(db, peer_key, signed, digest, seq, reason).await {
        Ok(true) => PenOutcome::Penned,
        Ok(false) => {
            // Pen at quota: FREEZE below this seq (delayed, never lost, loud).
            // Acking pen rows genuinely frees quota now (only UNACKED rows count),
            // so "ack to release" is a real remedy.
            eprintln!(
                "pull: node_event_quarantine for {peer_key} at capacity — \
                 freezing the cursor at seq {seq} (inspect + ack, or delete, to release)"
            );
            PenOutcome::Frozen
        }
        Err(qe) => {
            // A pen WRITE error is transient infrastructure trouble; freeze
            // conservatively rather than advancing past an un-penned refusal.
            eprintln!(
                "pull: could not pen node_event at seq {seq}: {} — freezing",
                operator_chain(&qe)
            );
            PenOutcome::Frozen
        }
    }
}
```

(Check the exact type of `peer_key` inside `pull_into` — it is `let peer_key = peer.to_string();`,
so pass `&peer_key`.)

- [ ] **Step 4: Rewrite the `Err(e)` arm of `pull_into`**

Replace the whole `Err(e) => match verify_self_described(signed) { … },` arm (from the comment
`// Classify the refusal by RE-VERIFYING the bytes ONCE` down to the arm's closing `},` just before
the loop's `}` that closes the `match db.execute(...)`) with:

```rust
            // Classify the refusal by RE-VERIFYING the bytes ONCE (bind the error for the
            // reason — do not verify twice). Four outcomes: unverifiable → pen; a verifiable
            // P0001 that is a SUBSTITUTION → pen (#619); any other verifiable P0001 (the
            // deliberate deny-all) → skip-and-sweep; anything else → freeze.
            Err(e) => match verify_self_described(signed) {
                Err(ve) => {
                    // UNVERIFIABLE: never applies without repair. Pen it durably and record the
                    // serving seq as the re-offer floor. `ve` is the legible reason (same
                    // vocabulary the DB DETAIL carries, issue #109).
                    let reason = ve.to_string();
                    let digest = event_address(signed);
                    match pen_or_freeze(db, &peer_key, signed, &digest, seq, &reason).await {
                        PenOutcome::Penned => stats.quarantined += 1, // held → cursor may advance
                        PenOutcome::Frozen => {
                            stats.frozen = Some(seq);
                            break;
                        }
                    }
                }
                // Bytes VERIFY, but the door refused. Distinguish a DELIBERATE refusal from a
                // transient DB fault by SQLSTATE: apply_remote_node_event's refusals are all
                // bare `RAISE EXCEPTION` (P0001, db/001's contract).
                Ok(body) if e.code().map(|c| c.code()) == Some("P0001") => {
                    // #619 / ADR-0073: ONE question before skipping. A P0001 is normally the
                    // self-healing deny-all — but a rival under an id this node already holds can
                    // never apply (the id is taken), so it is penned as evidence instead. Asked of
                    // the TABLE, not of the SQLSTATE or the door's sentence: see sync/substitution.rs.
                    let offered = event_address(signed);
                    let held = match substitution::held_content_address(db, &body.event_id).await
                    {
                        Ok(held) => held,
                        Err(le) => {
                            // Could not tell whether it is a substitution: FREEZE, never advance
                            // past a refusal this loop could not classify.
                            eprintln!(
                                "pull: could not check node_event at seq {seq} for a \
                                 substitution: {} — freezing",
                                operator_chain(&le)
                            );
                            stats.frozen = Some(seq);
                            break;
                        }
                    };
                    match substitution::substitution_reason(
                        &body.event_id,
                        held.as_deref(),
                        &offered,
                    ) {
                        // Deliberately no per-event log line: an ACKED row is still re-offered on
                        // every full sweep, and a per-event line would keep printing after a human
                        // had decided. The loud signal is `run`'s INTEGRITY line, which counts only
                        // UNACKED rows — the unverifiable arm's convention too.
                        Some(reason) => {
                            match pen_or_freeze(db, &peer_key, signed, &offered, seq, &reason)
                                .await
                            {
                                PenOutcome::Penned => stats.quarantined += 1,
                                PenOutcome::Frozen => {
                                    stats.frozen = Some(seq);
                                    break;
                                }
                            }
                        }
                        // The normal, self-healing deny-all (un-trusted author / unknown type):
                        // skip-and-advance; re-offered on a later peer.added / code arrival + full
                        // sweep.
                        None => {
                            stats.rejected += 1;
                            // #474 item 1: this is the arm where the door's own `RAISE` text IS
                            // the entire diagnosis — untrusted author? unknown event type? —
                            // and `{e}` printed `db error` in its place. The reason was one
                            // `as_db_error()` away the whole time.
                            eprintln!(
                                "pull: node_event refused (recoverable, non-fatal): {}",
                                legible_db_error(&e)
                            );
                        }
                    }
                }
                // <keep the existing final `Ok(_) => { … freezing (not skipped past) … }` arm
                //  EXACTLY as it is, comments included>
            },
```

- [ ] **Step 5: Update the three pieces of published operator text**

In `crates/cairn-node/src/sync.rs`:

1. `pull_into`'s doc, the bullet list under `Classification of a refusal (issue #111)`: after the
   `UNVERIFIABLE bytes …` bullet, insert:
   ```rust
   ///   * A VERIFIABLE event refused under an `event_id` this node ALREADY HOLDS with different
   ///     content — a SUBSTITUTION (#619, ADR-0073) — is PENNED the same way, whichever check
   ///     refused it: it can never apply (the id is taken), so skipping it would file it under
   ///     "self-healing". It never auto-releases; a human acks it.
   ```
   and change the next bullet's first words from `A VERIFIABLE-but-refused event` to
   `Any OTHER verifiable-but-refused event`.
2. `PullStats`' doc: replace `` `quarantined` = UNVERIFIABLE
   /// events penned this cycle (issue #111); `` with `` `quarantined` = events penned this cycle —
   /// UNVERIFIABLE bytes (issue #111), and SUBSTITUTIONS: a verifiable event under an `event_id`
   /// this node already holds with different content, which can never apply (#619); ``.
3. `run`'s INTEGRITY line: replace
   ```rust
                        "run: INTEGRITY: {} unacked quarantined node_event(s) from {peer} — \
                         inspect `cairn-node quarantine`, then fix trust/code or `ack-quarantine`",
   ```
   with
   ```rust
                        "run: INTEGRITY: {} unacked quarantined node_event(s) from {peer} — \
                         inspect `cairn-node quarantine` (each row's reason says why: unverifiable \
                         bytes, or a substitution under an event_id this node already holds), then \
                         fix the cause or `ack-quarantine`",
   ```

In `crates/cairn-node/src/main.rs`, replace the `Quarantine` doc comment
```rust
    /// List the durable node-event quarantine (issue #111): every pulled node_event
    /// this node refused as UNVERIFIABLE, with its reason, re-offer floor seq, and
    /// ack state. One JSON object per line. An unacked row makes the pull loud every
    /// cycle until its cause is fixed (auto-releases) or it is acked.
```
with
```rust
    /// List the durable node-event quarantine (issues #111, #619): every pulled
    /// node_event this node penned — UNVERIFIABLE bytes, or a SUBSTITUTION (a second,
    /// different event under an event_id this node already holds) — with its reason,
    /// re-offer floor seq, and ack state. One JSON object per line. An unacked row makes
    /// the pull loud every cycle until its cause is fixed (auto-releases) or it is acked;
    /// a substitution never auto-releases, because the id it reuses is taken.
```

- [ ] **Step 6: Run to verify**

Run: `cargo test -p cairn-node --test node_substitution_is_penned --test node_quarantine --test pull_failure_class --test pull_peer_integrity --lib`
Expected: all PASS, zero warnings. `node_quarantine.rs::a_verifiable_but_refused_event_is_skipped_not_penned`
passing is the proof the scoping class is untouched.

- [ ] **Step 7: Commit**

```bash
cargo fmt --all
git add crates/cairn-node/src/sync.rs crates/cairn-node/src/main.rs \
        crates/cairn-node/tests/node_substitution_is_penned.rs
git commit -m "feat(#619): the node puller pens a substitution instead of skipping it

A verifiable P0001 is normally self-healing scoping, skipped and swept. A
rival under an id this node already holds never heals, so the P0001 arm
now asks the table first and pens it — durable evidence of an
equivocating peer, loud until a human acks. The pen handling both arms
share is one helper (pen_or_freeze). The quarantine help, PullStats and
the INTEGRITY line's remedy stop claiming every pen row is unverifiable.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

### Task 6: Mutation harness and ledger

A test over new behaviour passes on its first run once the code exists, so each claim's red phase is
a mutation, and each must be killed at the assertion that names its claim.

**Files:**
- Create: `scripts/mutations/2026-09-19-619.sh`
- Modify: this plan (the ledger below)

- [ ] **Step 1: Write the harness**

Copy `scripts/mutations/2026-09-17-614-615.sh` to `scripts/mutations/2026-09-19-619.sh`. Keep its
infrastructure verbatim (`fail`, `require_clean`, `swap`, `run_mutation`, the compiler-kill
detection) but replace the header's first two lines with
`# Mutation harness for #619 (ADR-0073). THROWAWAY — committed only so the run's ledger in` /
`# docs/superpowers/plans/2026-09-19-node-plane-substitution-guard-619.md is reproducible.`, and
replace the three hard-coded `CAIRN_TEST_PG*` exports with a discovered cluster:

```bash
read -r PGHOST PGPORT <<<"$(scripts/pg-target.sh)" || fail "no PostgreSQL target"
U=$(whoami)
export CAIRN_TEST_PG="host=$PGHOST port=$PGPORT dbname=cairn_test user=$U"
export CAIRN_TEST_PG2="host=$PGHOST port=$PGPORT dbname=cairn_test2 user=$U"
export CAIRN_TEST_PG3="host=$PGHOST port=$PGPORT dbname=cairn_test3 user=$U"
```

Replace everything after `echo "=== #614/#615 mutation run` with:

```bash
echo "=== #619 mutation run — $(date -u +%FT%TZ) ==="
require_clean

NODE_TEST=(cargo test -p cairn-node --test)

# M1 — the local door's guard deleted. The local rival tests must catch it.
run_mutation M1 KILLED db/007_node_federation.sql \
    "    PERFORM cairn_refuse_substitution(v_found, v_ca, v_eid, 'submit_node_event');" \
    '    -- (mutation M1: guard deleted)' \
    "${NODE_TEST[@]}" node_plane_one_event_id_one_body

# M2 — the admission gate's guard deleted. The remote rival tests must catch it.
run_mutation M2 KILLED db/007_node_federation.sql \
    "    PERFORM cairn_refuse_substitution(v_found, v_ca, v_eid, 'apply_remote_node_event');" \
    '    -- (mutation M2: guard deleted)' \
    "${NODE_TEST[@]}" node_plane_one_event_id_one_body

# M3 — M1's deletion again, measured against the CATALOGUE rule alone: the inventory must catch a
# door that stopped calling the helper without any behaviour test's help.
run_mutation M3 KILLED db/007_node_federation.sql \
    "    PERFORM cairn_refuse_substitution(v_found, v_ca, v_eid, 'submit_node_event');" \
    '    -- (mutation M3: guard deleted)' \
    "${NODE_TEST[@]}" substitution_guard_covers_every_writer

# M4 — the admission gate's guard hoisted ABOVE its IF/ELSE, where nothing is held yet: every
# clean apply is refused. The IDEMPOTENCE case must catch it (the rival cases would still pass).
run_mutation M4 KILLED db/007_node_federation.sql \
    "    IF v_op = 'enroll' THEN
        -- The genesis must match an active, out-of-band-confirmed peer: its" \
    "    SELECT content_address INTO v_found FROM node_event WHERE node_event_id = v_eid;
    PERFORM cairn_refuse_substitution(v_found, v_ca, v_eid, 'apply_remote_node_event');
    IF v_op = 'enroll' THEN
        -- The genesis must match an active, out-of-band-confirmed peer: its" \
    "${NODE_TEST[@]}" node_plane_one_event_id_one_body

# M5 — the local door's guard hoisted above its IF/ELSE.
run_mutation M5 KILLED db/007_node_federation.sql \
    "    IF v_op = 'supersede' THEN
        IF v_payload ->> 'superseded_node_id_hex' IS NULL THEN
            RAISE EXCEPTION 'submit_node_event:" \
    "    SELECT content_address INTO v_found FROM node_event WHERE node_event_id = v_eid;
    PERFORM cairn_refuse_substitution(v_found, v_ca, v_eid, 'submit_node_event');
    IF v_op = 'supersede' THEN
        IF v_payload ->> 'superseded_node_id_hex' IS NULL THEN
            RAISE EXCEPTION 'submit_node_event:" \
    "${NODE_TEST[@]}" node_plane_one_event_id_one_body

# M6 — the decision inverted: an idempotent re-offer becomes a substitution, a rival does not.
run_mutation M6 KILLED crates/cairn-node/src/sync/substitution.rs \
    '    if held == offered {' \
    '    if held != offered {' \
    cargo test -p cairn-node --lib sync::substitution

# M7 — the puller never asks: every P0001 is skipped again (#619's pull-path half undone). Written
# as `.filter(|_| false)` so every binding stays used and the COMPILER cannot be what kills it.
run_mutation M7 KILLED crates/cairn-node/src/sync.rs \
    '                        &offered,
                    ) {' \
    '                        &offered,
                    ).filter(|_| false) {' \
    "${NODE_TEST[@]}" node_substitution_is_penned

# M8 — the lookup blinded: nothing is ever held, so no substitution is ever found.
run_mutation M8 KILLED crates/cairn-node/src/sync/substitution.rs \
    '    Ok(row.map(|r| r.get(0)))' \
    '    Ok(row.map(|r| r.get(0)).filter(|_: &Vec<u8>| false))' \
    "${NODE_TEST[@]}" node_substitution_is_penned

# M9 — the one shared clock merge deleted. The moved pin (3 → 1) must be live.
run_mutation M9 KILLED db/007_node_federation.sql \
    "    PERFORM cairn_node_hlc_merge((b -> 'hlc' ->> 'wall')::bigint,
                                 (b -> 'hlc' ->> 'counter')::int);
    RETURN v_eid;" \
    '    RETURN v_eid;' \
    "${NODE_TEST[@]}" hlc_merge_helper

echo "=== run complete ==="
require_clean && echo "tree is clean: every revert landed"
```

Before running, confirm each anchor occurs EXACTLY once in its file (the harness refuses otherwise,
which is the point — but check by hand so a refusal is not a surprise):
`grep -c "IF v_op = 'enroll' THEN" db/007_node_federation.sql` is 2, which is why M4's anchor
includes the next comment line; the M7 anchor must match `rustfmt`'s actual layout of the
`substitution_reason(` call in `sync.rs` — adjust the anchor text to the formatted source, never the
source to the anchor.

- [ ] **Step 2: Run it**

```bash
chmod +x scripts/mutations/2026-09-19-619.sh
git add scripts/mutations/2026-09-19-619.sh && git commit -m "test(#619): mutation harness

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
scripts/mutations/2026-09-19-619.sh > /tmp/cairn-619-mutations.log 2>&1; echo "harness exit=$?"
cat /tmp/cairn-619-mutations.log
```

(The harness needs a CLEAN tree, hence the commit first.) Expected: nine lines `expected KILLED
actual KILLED`, then `tree is clean: every revert landed`. For each KILL, confirm from a single
re-run (`cargo test … --test <suite>` with the mutation applied by hand) that the panic line is the
assertion that names the claim — e.g. M4 must fail in `the_admission_gate_still_admits_the_same_event_twice`,
not in a rival test. A DIVERGENCE is a finding: investigate, never re-label the expectation.

- [ ] **Step 3: Record the ledger in this plan and commit**

Fill the table under **Mutation ledger** below with each id, what it does, the suite, and the
observed verdict plus the failing assertion; commit with
`docs(#619): the mutation ledger — N of 9 killed`.

---

### Task 7: ADR-0073 and spec v0.75

**Files:**
- Create: `docs/spec/decisions/0073-the-node-plane-refuses-a-substitution-and-pens-it.md`
- Modify: `docs/spec/decisions/README.md` (index row), `mkdocs.yml` (nav line — SAME commit),
  `docs/spec/index.md` (`**Spec version:** 0.74` → `0.75`), `docs/spec/sync.md` (§6.3)

- [ ] **Step 1: Write the ADR**

Follow ADR-0072's shape (Status/Date/Spec version/Issues/Relates to; Context; Decision; Alternatives
rejected; Consequences; Residuals). Header:

```markdown
# ADR-0073 — The node plane refuses a substitution at both live doors, and pens it

- **Status:** Accepted
- **Date:** 2026-09-19
- **Spec version at acceptance:** 0.75
- **Issues:** [#619](https://github.com/cairn-ehr/cairn-ehr/issues/619) ·
  [#268](https://github.com/cairn-ehr/cairn-ehr/issues/268) (one class carved out; the rest open)
- **Relates to:** [ADR-0072](0072-a-restore-loses-no-record-silently.md) ·
  [ADR-0056](0056-unknown-event-types-admitted-uninterpreted.md) ·
  [ADR-0017](0017-federation-admission-sovereignty-peering-and-trust-anchors.md)
- **Amends:** ADR-0072's census. It said the restore door was the last unguarded `node_event`
  writer; db/007's two doors were not guarded either. Reverses nothing.
```

Decisions to record (one numbered paragraph each, with the WHY):
1. `submit_node_event` and `apply_remote_node_event` refuse a substitution through the shared
   `cairn_refuse_substitution`, once per door, AFTER the branch, with an unconditional read (db/009's
   shape) — two call sites, not five; the genesis arm of `submit_node_event` needs none (no `ON
   CONFLICT`: a collision raises).
2. The node puller classifies a refused event as a substitution **by state** — a row already held
   under its `event_id` with a different content address — never by SQLSTATE (db/001's P0001
   contract; a distinct code would also turn `cairn-sync`'s clinical pen into a freeze) and never by
   the door's sentence; and pens it, whichever check refused it. The scoping deny-all keeps
   skip-and-advance. This is the first member of #268's "genuinely refused history" class and does
   not decide the rest.
3. The inventory of guarded writers is a `pg_proc` catalogue rule, pinned by name — the hand list is
   how the census error happened.
4. The published operator text moves with the code (quarantine help, `PullStats`, the INTEGRITY line).

Context must carry spec §1.1's precise statement of consequences, INCLUDING the correction that
#619's "A keeps trusting C" cannot happen (`trust_peer` reads only locally-authored events).
Consequences must name the cost honestly: one primary-key lookup per verifiable P0001 refusal on the
node plane (tens of events; every full sweep re-offers the scoping refusals, so it recurs), none on
the clinical plane. Alternatives rejected: a distinct SQLSTATE; matching message text; five inline call sites; skipping
a substitution (the maintainer's ruling); doing all of #268 now; widening the catalogue rule to
`actor_event` (#569). Residuals: **#605** (in-place edit, `SCHEMA_GENERATION` stays 53 — an older
gen-53 binary can reload the unguarded doors), **#268**'s remaining classes, **#301**, **#569**,
node-plane completeness accounting, **#608**'s late-custody half, and the existing per-peer pen
quota (a substitution flood freezes that peer's cursor, as an unverifiable flood does).

**Check every factual sentence of the ADR against the tree (`git show HEAD:<file>`), not memory**,
and resolve every link — an ADR is immutable once merged (the 2026-09-16/17 lessons).

- [ ] **Step 2: Index, nav, version, §6.3**

- `docs/spec/decisions/README.md`: add a `| [0073](…) | **…** : … | Accepted | 2026-09-19 |` row after 0072's, in the same one-paragraph style.
- `mkdocs.yml`: after line `      - ADR-0072 · A restore loses no record silently: spec/decisions/0072-a-restore-loses-no-record-silently.md` add `      - ADR-0073 · The node plane refuses a substitution and pens it: spec/decisions/0073-the-node-plane-refuses-a-substitution-and-pens-it.md`.
- `docs/spec/index.md`: `**Spec version:** 0.74` → `**Spec version:** 0.75`.
- `docs/spec/sync.md` §6.3: in the row *"Peer offers an event this node's floor genuinely refuses"*,
  append to its closing **Honest status — the node/actor plane still diverges** sentence:
  ` **One class is carved out** ([ADR-0073](decisions/0073-the-node-plane-refuses-a-substitution-and-pens-it.md)): a verifiable node event refused under an \`event_id\` this node already holds with different content — a substitution — is penned like an unverifiable one, whichever check refused it, since it can never apply.`
  and add a new row after it:
  `| Peer offers a second, different event under an \`event_id\` this node already holds (a substitution) | **Refused at every write door, on both planes** — one shared comparison (\`cairn_refuse_substitution\`, [ADR-0072](decisions/0072-a-restore-loses-no-record-silently.md)), compared \`IS DISTINCT FROM\` so "cannot tell what is held" is a refusal too. Never a silent discard: \`ON CONFLICT DO NOTHING\` alone cannot tell a rival from a repeat. On the pull path the rival is **penned verbatim** (clinical plane since [#267](https://github.com/cairn-ehr/cairn-ehr/issues/267); node plane since [ADR-0073](decisions/0073-the-node-plane-refuses-a-substitution-and-pens-it.md), classified by state because every refusal shares one SQLSTATE) and never auto-releases; on the restore path the node plane aborts the ceremony. A repeat of the SAME event stays a silent no-op — that is set-union. |`

- [ ] **Step 3: Build the docs strictly**

Run: `uv run --with-requirements docs/requirements.txt -- mkdocs build --strict`
Expected: exits 0. (A missing nav line aborts `--strict`.)

- [ ] **Step 4: Commit**

```bash
git add docs/spec/decisions/0073-*.md docs/spec/decisions/README.md mkdocs.yml docs/spec/index.md docs/spec/sync.md
git commit -m "docs(#619): ADR-0073 — the node plane refuses a substitution and pens it (spec v0.75)

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

### Task 8: Full gate, tracking documents, PR (controller — not a subagent)

- [ ] **Step 1:** `cargo fmt --all -- --check` and
  `RUSTDOCFLAGS="-D warnings" cargo doc -p cairn-node --no-deps` — both clean.
- [ ] **Step 2:** `scripts/run-db-gated-tests.sh` in the BACKGROUND (hours on this machine; read the
  logged exit, never a notification wrapper's). Record suites / tests / failures.
- [ ] **Step 3:** While it runs: HANDOVER (⇒ NEXT, a trap 13 or an extension of trap 12 for db/007,
  the session entry, condensing toward ~500 lines without dropping an open issue number) and
  ROADMAP (the slice entry, #619's residuals).
- [ ] **Step 4:** Push, open the PR (body: what, why, the maintainer's two rulings, the mutation
  ledger, the gate result, residuals; `Refs #619` — never a closing keyword), and wait for CI.

---

## Mutation ledger

Run 2026-09-19 with `scripts/mutations/2026-09-19-619.sh` (M1–M8 in the full run; M9–M10 re-run
after fix round 1, see below). **Nine killed, each at the assertion that names its claim; one declared
survivor survived as declared.**

| Id | Mutation | Suite | Expected | Observed (failing assertion) |
|---|---|---|---|---|
| M1 | local guard deleted | node_plane_one_event_id_one_body | KILLED | KILLED — `the_local_door_refuses_a_rival_supersede`: "a rival supersede under a held id must be refused, not dropped: ()" |
| M2 | admission-gate guard deleted | node_plane_one_event_id_one_body | KILLED | KILLED — `the_admission_gate_refuses_a_rival_supersede`: "a rival supersede under a held id must be refused: ()" |
| M3 | local guard deleted, catalogue only | substitution_guard_covers_every_writer | KILLED | KILLED — `every_event_log_writer_refuses_a_substitution`: "… never call cairn_refuse_substitution … ["submit_node_event"]" |
| M4 | admission-gate guard hoisted above IF/ELSE | node_plane_one_event_id_one_body | KILLED | KILLED — `the_admission_gate_still_admits_the_same_event_twice`: "pass 1: the SAME event twice must stay a no-op, never raise: apply_remote_node_event: … (substitution refused)" |
| M5 | local guard hoisted above IF/ELSE | node_plane_one_event_id_one_body | KILLED | KILLED — `the_local_door_still_admits_the_same_event_twice`: "pass 1: the SAME event twice must stay a no-op, never raise: submit_node_event: … (substitution refused)" |
| M6 | decision inverted | --lib sync::substitution | KILLED | KILLED — `the_same_event_again_is_not_a_substitution`: "an idempotent re-offer is set-union working, never a refusal of this kind" |
| M7 | puller never asks | node_substitution_is_penned | KILLED | KILLED — `an_acked_substitution_stays_quiet_on_reoffer`: "penned on the first sweep" (left 0, right 1) |
| M8 | lookup blinded | node_substitution_is_penned | KILLED | KILLED — `a_rival_refused_by_an_earlier_check_is_still_penned`: "a rival under a held id is penned whatever refused it" |
| M9 | shared clock merge deleted | hlc_merge_helper | KILLED | KILLED — `every_door_still_calls_the_helper`: "every admission door must still PERFORM cairn_node_hlc_merge …" — the moved pin (3 → 1) is live |
| M10 | lookup-failure FREEZE turned into a SKIP | node_substitution_is_penned | **SURVIVED (declared before the run)** | SURVIVED, as declared — no fault-injection seam can make `held_content_address` fail inside the single-DB self-pull (a lock blocks rather than fails; the owner role bypasses grants). Bounded: a skipped substitution is re-offered on the next full sweep and penned then. A stated residual, not a silent one. |

**The harness caught itself once, and that is the durable part.** The first full run stopped at M9:
its replacement text was a bare `    RETURN v_eid;`, which occurs three times in db/007, so after the
forward swap the REVERT anchor was ambiguous and `swap`'s exactly-once count refused it, exiting
the harness with the mutation still applied — which is what stopped M10 running on top of it
(#594's defect, caught this time rather than shipped; had the script continued, `require_clean`'s
`git diff --quiet` would have stopped it one mutation later). The tree was restored by reversing exactly that two-line patch.
Fix round 1 gave M9 a unique replacement and added the reverse-direction half of the control:
`run_mutation` now refuses, before touching the file, any mutation whose replacement text already
occurs there — so an unrevertable mutation can no longer be applied at all. **The general lesson:
checking that an anchor is unique in the direction you apply it is half the check; the revert
needs the replacement to be unique too.**

## Paper-parity benchmark (§1.2)

Paper-parity: not clinical-surface — this changes how two federation write doors and the node-plane pull loop treat a forged or colliding trust-plane event; no clinician performs, sees or waits on any step of it, and no clinical workflow gains or loses an act.
