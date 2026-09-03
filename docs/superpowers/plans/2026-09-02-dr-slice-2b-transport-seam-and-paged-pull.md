# DR slice 2b — the transport seam and the paged pull: implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended)
> or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`)
> syntax for tracking.

**Goal:** Put `cairn-sync`'s peer request behind a `Transport` trait so a backup medium can answer as a
peer, and give `EventsAfterSeq` a batch limit so a pull pages instead of shipping a whole log suffix in
one frame.

**Architecture:** A new pure library crate `crates/cairn-wire` holds the clinical-plane wire types, the
framing, the `Transport` trait and two implementations (`TcpTransport`, `MediumTransport`).
`cairn-sync`'s `do_pull` takes `&dyn Transport` and loops over pages, checkpointing its cursor after
each one. Nothing in `cairn-node` changes in this slice.

**Tech Stack:** Rust 1.96 (workspace MSRV), `serde`/`serde_json`, blocking `postgres` 0.19, existing
crates `cairn-event` (framing core, COSE) and `cairn-medium` (the CAIRNB3 medium format).

**Spec:** [`docs/superpowers/specs/2026-09-02-dr-slice-2b-transport-seam-and-paged-pull-design.md`](../specs/2026-09-02-dr-slice-2b-transport-seam-and-paged-pull-design.md)

Paper-parity: not clinical-surface — a transport seam and a wire batch limit beneath the API layer; this
slice adds no clinical workflow, changes no human act, and touches no operator command's step count.

---

## Global Constraints

- **Licence:** AGPL-3.0-only. Every dependency must be AGPL-3.0-compatible. `cairn-wire` introduces
  **no new third-party dependency** — only `serde`, `serde_json`, `hex`, `cairn-event`, `cairn-medium`,
  all already vetted in this workspace.
- **`publish = false`** on `cairn-wire` (like every other crate here): it lets the path dependencies
  omit a version without tripping the cargo-deny wildcard gate.
- **`[lints] workspace = true`** in the new crate's `Cargo.toml`. The workspace denies `warnings` and
  `clippy::all`.
- **TDD (house rule 2):** the failing test comes first, every time. No production code without a test
  that drove it.
- **Inline documentation for a junior developer (house rule 3):** every non-trivial item explains *why*
  it exists and *how* it fits, not what the next line does.
- **500-line files (house rule 4):** every file created here stays under 500 lines. `main.rs` is already
  far past it; that is [#531](https://github.com/cairn-ehr/cairn-ehr/issues/531) and is explicitly out of
  scope — **new** code goes in new modules rather than into `main.rs`.
- **Never name a non-cryptographic binding `salt`, `nonce` or `iv` (house rule 6b).**
  `crates/cairn-node/tests/crypto_sink_names_are_genuine.rs` sweeps every shipping `src/` tree, including
  this new crate, and a `#[cfg(test)]` block inside `src/` is in scope. Use `lineage`, `variant`, `seed`.
- **Test key material is derived at runtime, never a literal (house rule 6a):**
  `std::array::from_fn(|i| …)`, not a byte-array literal.
- **Three cargo trees:** the root workspace, `extensions/cairn_pgx` and `cairn-gui` each have their own
  `Cargo.lock`. `cairn-wire` is depended on **only by `cairn-sync`**, which is in neither of the other
  two graphs — but that is verified in Task 1, not assumed.
- **`cargo test` without a database FAILS** unless `CAIRN_ALLOW_DB_SKIP=1` is exported (#450).
- **Never `git checkout -- <file>`** to undo an edit; it discards uncommitted work irrecoverably.
- **Commit at the end of every task.** Attribution footer on every commit:
  `Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>`

### Running the tests

```bash
# fast, crate-scoped, no database (Tasks 1-6)
CAIRN_ALLOW_DB_SKIP=1 cargo test -p cairn-wire
CAIRN_ALLOW_DB_SKIP=1 cargo test -p cairn-sync --all-targets

# the DB-gated suites (Tasks 7-8). -p cairn-sync DOES build the cross-crate clinical_pull.rs.
export CAIRN_TEST_PG='host=127.0.0.1 port=5532 user=postgres dbname=cairn_test'
export CAIRN_TEST_PG2='host=127.0.0.1 port=5532 user=postgres dbname=cairn_test2'
export CAIRN_TEST_PG3='host=127.0.0.1 port=5532 user=postgres dbname=cairn_test3'
cargo test -p cairn-sync -- --test-threads=2

# lint gate, matching CI
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --check
```

⚠️ **Never pipe `cargo test` into `tail`** — it masks cargo's exit code, which has hidden a red gate here
before. ⚠️ **A running IDE's rust-analyzer holds the shared `target/` lock**; if a narrow `cargo test`
stalls before compiling, use `CARGO_TARGET_DIR=/tmp/cairn-2b`.

---

## File Structure

**Created:**

| file | responsibility |
|---|---|
| `crates/cairn-wire/Cargo.toml` | manifest |
| `crates/cairn-wire/src/lib.rs` | module map, the crate's invariants, re-exports |
| `crates/cairn-wire/src/wire.rs` | `Request`, `EventsResponse` — the clinical-plane protocol |
| `crates/cairn-wire/src/framing.rs` | `MAX_FRAME_BYTES`, `read_frame`, `write_frame` |
| `crates/cairn-wire/src/transport.rs` | `Transport`, `TransportError` |
| `crates/cairn-wire/src/tcp.rs` | `TcpTransport` |
| `crates/cairn-wire/src/medium.rs` | `MediumTransport` |
| `crates/cairn-wire/src/page.rs` | `DEFAULT_PAGE_EVENTS`, `PageDecision`, `page_decision` |
| `crates/cairn-sync/src/pull_page.rs` | `PageTally`, `CycleTally`, `fold_page`, `quarantine_floor` |

**Modified:**

| file | change |
|---|---|
| `Cargo.toml` | add `crates/cairn-wire` to `members` |
| `crates/cairn-sync/Cargo.toml` | add the `cairn-wire` + `cairn-medium` path dependencies |
| `crates/cairn-sync/src/main.rs` | delete the moved items; import from `cairn_wire`; `do_pull` takes a transport and loops; `serve_conn` honours `limit`; `--page` flag |
| `crates/cairn-event/src/framing.rs` | one stale comment (Task 8) |
| `crates/cairn-medium/src/chunk.rs` | one stale comment (Task 8) |
| `docs/HANDOVER.md`, `docs/ROADMAP.md` | Task 8 |

---

## Task 1: The `cairn-wire` crate — framing and wire types, moved verbatim

**Files:**
- Create: `crates/cairn-wire/Cargo.toml`, `crates/cairn-wire/src/lib.rs`,
  `crates/cairn-wire/src/framing.rs`, `crates/cairn-wire/src/wire.rs`
- Modify: `Cargo.toml` (workspace members), `crates/cairn-sync/Cargo.toml`,
  `crates/cairn-sync/src/main.rs`

**Interfaces:**
- Produces: `cairn_wire::{Request, EventsResponse, MAX_FRAME_BYTES, read_frame, write_frame}`.
  `Request` is `#[serde(tag = "op")]` with variants `EventsAfter { wall: i64, counter: i32 }`,
  `EventsAfterSeq { after_seq: i64, unwrap_cert: Option<String> }`,
  `BlobSlice { addr_hex: String, offset: u64, len: u64 }`.
  `read_frame(&mut impl Read) -> io::Result<Vec<u8>>`, `write_frame(&mut impl Write, &[u8]) -> io::Result<()>`.

**This is a verbatim move.** The proof of a faithful extraction — the same proof `cairn-keystore` (#503)
and `cairn-medium` (slice 2a) rest on — is that every existing call site compiles with nothing but an
import change, and every existing test still passes unedited. Do not rename, reorder, re-word a doc
comment, or "tidy" anything in this task. Fields, doc comments and serde attributes travel byte-for-byte.

- [ ] **Step 1: Create the manifest**

`crates/cairn-wire/Cargo.toml`:

```toml
[package]
name = "cairn-wire"
version = "0.0.0"
description = "Cairn clinical-plane sync protocol: wire types, framing, and the transport seam (a network peer or a backup medium)."
edition.workspace = true
rust-version.workspace = true
license.workspace = true
repository.workspace = true
# A workspace-internal library, not a crates.io crate: never published. Also lets the
# internal path dependencies omit a version without tripping the cargo-deny wildcard gate.
publish = false

# Inherit the central workspace lint policy (#144).
[lints]
workspace = true

[dependencies]
cairn-event = { path = "../cairn-event" }   # the shared length-prefix framing core (#212)
# The CAIRNB3 backup-medium format, so a medium can answer as a peer (#500 slice 2b).
cairn-medium = { path = "../cairn-medium" }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
hex = "0.4"
```

- [ ] **Step 2: Register the crate in the root workspace**

In the root `Cargo.toml`, add to `members`, keeping the list's existing comment style:

```toml
    "crates/cairn-medium", # the shared backup-medium container format (#500 slice 2a)
    "crates/cairn-patient-search",
    "crates/cairn-wire", # the sync wire protocol + the transport seam (#500 slice 2b)
    "crates/cairn-sync",
```

- [ ] **Step 3: Write the failing test — the wire types round-trip**

`crates/cairn-wire/src/wire.rs`, at the bottom:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    /// The op tag is a WIRE CONSTANT. A mirrored rename of the variant and its `#[serde(tag)]`
    /// would be invisible to a round-trip through this same enum, so the expectation is a
    /// literal JSON string, not a re-encode.
    #[test]
    fn events_after_seq_encodes_its_op_tag_literally() {
        let req = Request::EventsAfterSeq {
            after_seq: 7,
            unwrap_cert: None,
        };
        let json = serde_json::to_string(&req).expect("serialize");
        assert!(
            json.contains(r#""op":"EventsAfterSeq""#),
            "op tag must be the literal wire string, got {json}"
        );
        assert!(json.contains(r#""after_seq":7"#), "got {json}");
    }

    /// A response from a peer that omits every additive field must still decode — that is
    /// what `#[serde(default)]` is FOR, and it is the property principle 12 rests on.
    #[test]
    fn a_minimal_response_decodes_through_the_serde_defaults() {
        let minimal = r#"{"events":["aa"]}"#;
        let resp: EventsResponse = serde_json::from_str(minimal).expect("decode");
        assert_eq!(resp.events, vec!["aa".to_string()]);
        assert!(resp.attestations.is_empty());
        assert!(resp.attester_keys.is_empty());
        assert!(resp.seqs.is_empty());
        assert!(resp.signing_context.is_none());
        assert!(resp.wrapped_deks.is_empty());
        assert!(resp.custody_withheld.is_none());
    }
}
```

- [ ] **Step 4: Run it to watch it fail**

Run: `CAIRN_ALLOW_DB_SKIP=1 cargo test -p cairn-wire`
Expected: FAIL — the crate does not compile; `Request` / `EventsResponse` do not exist.

- [ ] **Step 5: Move `Request` and `EventsResponse` into `wire.rs`**

Cut `main.rs:491-522` (the `Request` enum with its full doc comments) and `main.rs:524-591`
(`struct EventsResponse` with its full doc comments) and paste them above the test module in
`crates/cairn-wire/src/wire.rs`. Change **only** these three things:

1. Add `pub` to both types and to **every** field and variant field (they were private in a binary).
2. Add the file header (below).
3. Add `use serde::{Deserialize, Serialize};` at the top.

File header for `wire.rs`:

```rust
//! The clinical-plane sync protocol: one JSON request, one JSON response, per connection.
//!
//! WHY THIS IS A LIBRARY AND NOT `cairn-sync`'s `main.rs`: from slice 2b a backup medium is
//! addressed through this exact protocol (ADR-0026 decision 2 — "backup is a configuration of
//! the sync daemon"), so `cairn-node`'s backup and restore paths need these types. `cairn-sync`
//! is a binary-only crate that dev-depends on `cairn-node`, so it cannot grow a `lib.rs` without
//! a dependency cycle. Same reasoning as `cairn-keystore` (#503) and `cairn-medium` (slice 2a).
//!
//! These types are MOVED VERBATIM out of `cairn-sync/src/main.rs`. Every field, doc comment and
//! serde attribute is byte-for-byte what it was, and the extraction's proof is that every call
//! site compiled untouched.
//!
//! EVOLUTION IS ADDITIVE (principle 12, ADR-0021). A new field arrives with `#[serde(default)]`
//! and a default that is safe when it is ABSENT; an existing field never changes meaning.
```

- [ ] **Step 6: Move the framing into `framing.rs`**

Cut `main.rs:620-663` — `MAX_FRAME_BYTES` (with its full doc comment), `write_frame`, `read_frame` — into
`crates/cairn-wire/src/framing.rs`. Make all three `pub`. Add:

```rust
//! Length-prefixed framing for the clinical plane: `[u32 big-endian length][payload]`.
//!
//! Moved verbatim from `cairn-sync/src/main.rs` (slice 2b). The DECISION — cap before
//! allocating, refuse at the source, u32 truncation unreachable — lives in the shared
//! `cairn_event::framing` core (#212); this module owns the clinical plane's CAP and its
//! refusal messages, because the cap is a per-plane policy (the node plane's is 8 MiB).

use std::io::{self, Read, Write};
```

Also move the four framing tests from `main.rs`'s `mod tests` into a `#[cfg(test)] mod tests` in
`framing.rs`: `read_frame_refuses_an_over_cap_length_prefix` (main.rs:6416),
`read_frame_round_trips_an_in_cap_frame` (6446), `frame_cap_holds_a_realistic_event_batch` (6462) and
`write_frame_refuses_an_over_cap_frame` (6846), plus the at-cap assertion beside it. Move them
verbatim; they are the only coverage these functions have.

- [ ] **Step 7: Write `lib.rs`**

```rust
//! The Cairn clinical-plane sync protocol, its framing, and the transport seam.
//!
//! # Why this crate exists
//!
//! `cairn-sync` owns the clinical event log and the pull loop; `cairn-node` owns backup and
//! restore. From slice 2b (#500) a backup MEDIUM is a peer — addressed through the same
//! request/response protocol a network peer is, which is what makes ADR-0026 decision 2's
//! "backup is a configuration of the sync daemon" true rather than aspirational. Both binaries
//! therefore need these types, and `cairn-sync` is binary-only and dev-depends on `cairn-node`,
//! so it cannot export them. Same shape, same reason, as `cairn-keystore` (#503).
//!
//! # Module map
//!
//! - `wire` — `Request` / `EventsResponse`. Moved verbatim from `cairn-sync`.
//! - `framing` — the `[u32 len][payload]` frame and the clinical plane's 64 MiB cap.
//! - `transport` — `Transport`, the one seam, and its error taxonomy.
//! - `tcp` — a network peer. Today's connect/retry behaviour, moved verbatim.
//! - `medium` — a CAIRNB3 backup medium answering as a peer.
//! - `page` — the paging contract: the default page size and the termination rule.
//!
//! # Scope today
//!
//! This crate does no database work. It does not decide WHAT goes on a medium and it does not
//! write one — `cairn-node`'s `backup.rs` still reads `node_event` and nothing else, which is
//! #500 and is NOT fixed by this crate existing. The medium gains clinical events in slice 2c
//! and gives them back in slice 2d.

mod framing;
mod transport;
mod wire;

pub use framing::{read_frame, write_frame, MAX_FRAME_BYTES};
pub use wire::{EventsResponse, Request};
```

(`transport`, `tcp`, `medium` and `page` are added by later tasks; `mod transport;` is declared here so
Task 2 only adds its file. If that trips an unused-module warning before Task 2 lands, declare it in
Task 2 instead — the workspace denies warnings.)

- [ ] **Step 8: Point `cairn-sync` at the new crate**

In `crates/cairn-sync/Cargo.toml`, under `[dependencies]`, after `cairn-keystore`:

```toml
# The clinical-plane wire protocol, framing and transport seam. Extracted so a backup
# medium can be addressed as a peer without cairn-node depending on this binary (#500 2b).
cairn-wire = { path = "../cairn-wire" }
```

In `main.rs`, after the `cairn_event` import block, add:

```rust
use cairn_wire::{read_frame, write_frame, EventsResponse, Request, MAX_FRAME_BYTES};
```

Delete the now-moved definitions and the moved tests. Everything else stays.

- [ ] **Step 9: Run the full crate suites**

```bash
CAIRN_ALLOW_DB_SKIP=1 cargo test -p cairn-wire
CAIRN_ALLOW_DB_SKIP=1 cargo test -p cairn-sync --all-targets
cargo clippy --workspace --all-targets -- -D warnings
```

Expected: all PASS. **If any `cairn-sync` test needed editing to compile, the move was not verbatim** —
revert that edit and find what actually changed.

- [ ] **Step 10: Verify the other two cargo trees are untouched**

```bash
cargo metadata --locked --format-version 1 --manifest-path cairn-gui/Cargo.toml > /dev/null
cargo metadata --locked --format-version 1 --manifest-path extensions/cairn_pgx/Cargo.toml > /dev/null
git status --short   # only the root Cargo.lock should have changed
```

Expected: both succeed, and neither sibling lockfile is modified — `cairn-wire` is depended on only by
`cairn-sync`, which is in neither graph. If either command fails with a lockfile-out-of-date error, run
`cargo update -w --manifest-path <that>/Cargo.toml` and commit the refreshed lockfile with the rest.

- [ ] **Step 11: Commit**

```bash
git add crates/cairn-wire Cargo.toml Cargo.lock crates/cairn-sync/Cargo.toml crates/cairn-sync/src/main.rs
git commit -m "$(cat <<'EOF'
refactor(#500): extract crates/cairn-wire — the sync wire types and framing

Slice 2b needs a medium to be addressable as a peer, and slice 2c/2d
need these types from cairn-node. cairn-sync is binary-only and
dev-depends on cairn-node, so it cannot export them without a
dependency cycle; a library crate is the same answer cairn-keystore
(#503) and cairn-medium (2a) reached for the same reason.

Request, EventsResponse, MAX_FRAME_BYTES, read_frame and write_frame
move VERBATIM, with their doc comments and serde attributes
byte-for-byte. The proof is that every cairn-sync call site compiled
with nothing but an import change and no test needed editing.

Closes nothing. #500 stays open: nothing here reads a database and no
clinical event moves.

Refs #500

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 2: The `Transport` trait and `TcpTransport`

**Files:**
- Create: `crates/cairn-wire/src/transport.rs`, `crates/cairn-wire/src/tcp.rs`
- Modify: `crates/cairn-wire/src/lib.rs`, `crates/cairn-sync/src/main.rs`

**Interfaces:**
- Consumes: `cairn_wire::{Request, read_frame, write_frame}` from Task 1.
- Produces:
  ```rust
  pub trait Transport {
      fn label(&self) -> &str;
      fn request(&self, req: &Request) -> Result<Vec<u8>, TransportError>;
  }
  pub enum TransportError {
      Exchange { label: String, source: Box<dyn std::error::Error + Send + Sync + 'static> },
      Unsupported { label: String, reason: String },
  }
  pub struct TcpTransport { /* private */ }
  impl TcpTransport {
      pub fn new(peer: impl Into<String>) -> Self;
      /// One attempt, no backoff — the byte tier fails over fast rather than retrying.
      pub fn try_once(&self, req: &Request) -> Result<Vec<u8>, TransportError>;
  }
  ```

- [ ] **Step 1: Write the failing test — the `io::Error` stays reachable**

This is the one property of this task that is not a mechanical move, and getting it wrong silently
reclassifies a hostile peer as link downtime. `crates/cairn-wire/src/transport.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::error::Error;
    use std::io;

    /// `cairn-sync`'s `chain_reaches_a_peer_frame_error` decides "this peer sent an over-cap
    /// frame" — an INTEGRITY condition, not a partition — by walking `source()` for an
    /// `io::Error` of kind `InvalidData`. A `TransportError` that formatted its cause into a
    /// string instead of keeping it as `source` would silently reclassify that peer as link
    /// downtime, and every test on the far side would stay green because those tests build the
    /// error they classify. So the chain is pinned HERE, on the error this crate produces.
    #[test]
    fn exchange_keeps_the_io_error_reachable_as_a_source() {
        let cause = io::Error::new(io::ErrorKind::InvalidData, "frame length 99 exceeds cap");
        let err = TransportError::Exchange {
            label: "tcp 10.0.0.3:9443".into(),
            source: Box::new(cause),
        };
        let found = std::iter::successors(Some(&err as &(dyn Error + 'static)), |e| e.source())
            .filter_map(|e| e.downcast_ref::<io::Error>())
            .any(|io| io.kind() == io::ErrorKind::InvalidData);
        assert!(found, "the io::Error must stay reachable through source()");
    }

    /// An unsupported request is not a failure of the link and no retry helps. It must be a
    /// DIFFERENT variant, not a differently-worded Exchange: a caller that cannot tell them
    /// apart will retry a medium four times for a blob it does not have.
    #[test]
    fn unsupported_is_a_distinct_variant_with_no_source() {
        let err = TransportError::Unsupported {
            label: "medium /vol/cairn.b3".into(),
            reason: "this medium carries no byte tier".into(),
        };
        assert!(err.source().is_none());
        assert!(err.to_string().contains("carries no byte tier"), "{err}");
    }
}
```

- [ ] **Step 2: Run it to watch it fail**

Run: `CAIRN_ALLOW_DB_SKIP=1 cargo test -p cairn-wire transport`
Expected: FAIL — `TransportError` does not exist.

- [ ] **Step 3: Write `transport.rs`**

```rust
//! The one seam: everything that can answer a [`Request`].
//!
//! WHY A TRAIT (slice 2b, #500). ADR-0026 decision 2 says clinical events back up "as a cold
//! peer … a configuration of the existing sync daemon". Until this trait existed, `cairn-sync`
//! reached its peer through one free function that opened a TCP socket, and a backup medium is
//! not a socket — so "the medium is a peer" was prose with nothing behind it. With the seam,
//! `do_pull` is transport-agnostic: slice 2d's restore drives the SAME puller, with the SAME
//! cursor, quarantine pen and custody handling, against a file.

use std::error::Error;
use std::fmt;

use crate::wire::Request;

/// Anything that can answer a [`Request`] with one response frame.
///
/// Implementors own their own retries, timeouts and reconnection: a caller sees either a
/// response frame or a [`TransportError`], never a half-finished exchange.
pub trait Transport {
    /// Where this transport actually goes — `"tcp 10.0.0.3:9443"`, `"medium /vol/cairn.b3"`.
    ///
    /// ERROR PROSE ONLY. It is deliberately NOT the peer's NAME: `sync_state` is keyed on
    /// `peer_name`, which `do_pull` keeps as its own parameter, because the cursor must stay
    /// attached to the peer's identity even when the route to it changes.
    fn label(&self) -> &str;

    /// One request, one response frame.
    fn request(&self, req: &Request) -> Result<Vec<u8>, TransportError>;
}

/// Why a request produced no usable response. **Two variants, because they have opposite
/// remedies** — the same reasoning that split `cairn_medium::BackupError` three ways.
#[derive(Debug)]
pub enum TransportError {
    /// The exchange failed: resolve, connect, write, or read. Retrying may help.
    ///
    /// ⚠️ `source` MUST stay a real, reachable cause and must never be flattened into the
    /// label or a message. `cairn-sync`'s `chain_reaches_a_peer_frame_error` walks `source()`
    /// for an `io::Error` of kind `InvalidData` — [`crate::read_frame`]'s refusal of an
    /// over-cap length prefix — to tell a PEER sending garbage from a LINK that went away.
    /// Those are different operator words (#482), and a `String` error has no chain.
    Exchange {
        label: String,
        source: Box<dyn Error + Send + Sync + 'static>,
    },
    /// This transport cannot answer this request at all — a medium asked for a blob slice, or
    /// a pre-CAIRNB3 image asked for clinical events. NOT a link failure: no retry helps, and
    /// a caller that cannot tell the two apart will retry four times for nothing.
    Unsupported { label: String, reason: String },
}

impl fmt::Display for TransportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            // The cause is the SUFFIX, and that placement is load-bearing rather than
            // stylistic: `cairn-sync`'s `operator_chain` drops a layer only when the layer
            // above it ENDS WITH that layer's rendering, so a mid-sentence `{source}` would
            // print the same transport error twice on the `run` path.
            TransportError::Exchange { label, source } => {
                write!(f, "{label}: the exchange failed: {source}")
            }
            TransportError::Unsupported { label, reason } => {
                write!(f, "{label}: cannot answer this request: {reason}")
            }
        }
    }
}

impl Error for TransportError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            TransportError::Exchange { source, .. } => Some(source.as_ref()),
            TransportError::Unsupported { .. } => None,
        }
    }
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `CAIRN_ALLOW_DB_SKIP=1 cargo test -p cairn-wire transport`
Expected: PASS (2 tests).

- [ ] **Step 5: Write the failing test for `TcpTransport`'s label**

`crates/cairn-wire/src/tcp.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::Request;

    #[test]
    fn the_label_names_the_transport_and_the_address() {
        let t = TcpTransport::new("10.0.0.3:9443");
        assert_eq!(t.label(), "tcp 10.0.0.3:9443");
    }

    /// An unresolvable address must surface as `Exchange` — the link class — not as a panic
    /// and not as `Unsupported`. Uses a syntactically valid but unroutable address so the
    /// test needs no network and no listener.
    #[test]
    fn an_unreachable_peer_is_an_exchange_failure() {
        let t = TcpTransport::new("127.0.0.1:1");
        let err = t
            .try_once(&Request::EventsAfterSeq {
                after_seq: 0,
                unwrap_cert: None,
            })
            .expect_err("nothing listens on port 1");
        assert!(matches!(err, TransportError::Exchange { .. }), "{err}");
    }
}
```

- [ ] **Step 6: Run it to watch it fail**

Run: `CAIRN_ALLOW_DB_SKIP=1 cargo test -p cairn-wire tcp`
Expected: FAIL — `TcpTransport` does not exist.

- [ ] **Step 7: Write `tcp.rs`, moving today's behaviour verbatim**

The bodies of `try_once` and `request` are `main.rs:665-679` (`try_request`) and `main.rs:681-696`
(`request`) unchanged — the same 10 s connect timeout, the same 30 s read/write timeouts, the same four
attempts with 250 ms doubling backoff. Only the error construction is new.

```rust
//! A network peer over plain TCP. Today's `cairn-sync` behaviour, moved verbatim.
//!
//! NoTls is intentional on this plane: the link is WireGuard, which is the transport and the
//! perimeter (Spike 0001's assumption). The node plane is the one with mTLS pinned to the
//! trust set (`cairn-node/src/transport.rs`); this is the walking-skeleton clinical plane.

use std::io;
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use crate::framing::{read_frame, write_frame};
use crate::transport::{Transport, TransportError};
use crate::wire::Request;

/// A peer reachable at `host:port`.
pub struct TcpTransport {
    peer: String,
    label: String,
}

impl TcpTransport {
    pub fn new(peer: impl Into<String>) -> Self {
        let peer = peer.into();
        let label = format!("tcp {peer}");
        Self { peer, label }
    }

    /// ONE attempt, no backoff.
    ///
    /// The byte tier wants this rather than [`Transport::request`]: a blob swarm round-robins
    /// across many peers, so failing over to the next source immediately beats spending four
    /// backoff attempts on a source that is down.
    pub fn try_once(&self, req: &Request) -> Result<Vec<u8>, TransportError> {
        self.exchange(req).map_err(|source| TransportError::Exchange {
            label: self.label.clone(),
            source,
        })
    }

    /// The raw exchange, with its cause UNBOXED into the error type by the callers above.
    /// Kept separate so both the single-attempt and the retrying entry points build the same
    /// `Exchange` error, with the same reachable `source`.
    fn exchange(&self, req: &Request) -> Result<Vec<u8>, Box<dyn std::error::Error + Send + Sync>> {
        // Bounded connect so a dead link fails fast instead of hanging for minutes.
        let addr = self
            .peer
            .to_socket_addrs()?
            .next()
            .ok_or_else(|| io::Error::new(io::ErrorKind::AddrNotAvailable,
                "could not resolve peer address"))?;
        let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(10))?;
        stream.set_read_timeout(Some(Duration::from_secs(30)))?;
        stream.set_write_timeout(Some(Duration::from_secs(30)))?;
        write_frame(&mut stream, &serde_json::to_vec(req)?)?;
        Ok(read_frame(&mut stream)?)
    }
}

impl Transport for TcpTransport {
    fn label(&self) -> &str {
        &self.label
    }

    /// Retry with exponential backoff. A Starlink link drops constantly; a transient failure
    /// must not fail the whole pull — it retries, and only a sustained outage surfaces as an
    /// error (which the `run` loop logs as a partition).
    fn request(&self, req: &Request) -> Result<Vec<u8>, TransportError> {
        let mut delay = Duration::from_millis(250);
        let mut last = None;
        for attempt in 0..4 {
            match self.exchange(req) {
                Ok(v) => return Ok(v),
                Err(e) => {
                    last = Some(e);
                    if attempt < 3 {
                        std::thread::sleep(delay);
                        delay *= 2;
                    }
                }
            }
        }
        Err(TransportError::Exchange {
            label: self.label.clone(),
            // Unwrap is unreachable: the loop runs four times and every arm that does not
            // return sets `last`.
            source: last.expect("four attempts always record a failure"),
        })
    }
}
```

Add `mod tcp;` and the re-exports to `lib.rs`:

```rust
mod tcp;
pub use tcp::TcpTransport;
pub use transport::{Transport, TransportError};
```

- [ ] **Step 8: Run the tests**

Run: `CAIRN_ALLOW_DB_SKIP=1 cargo test -p cairn-wire`
Expected: PASS.

- [ ] **Step 9: Switch `cairn-sync`'s call sites**

Delete `try_request` and `request` from `main.rs`. Then:

1. `do_pull` (`main.rs:3020`): change the parameter `peer: &str` to `transport: &dyn Transport` and the
   call at 3097 from `request(peer, &…)` to `transport.request(&…)`. **`peer_name` is untouched.**
2. `do_blobd` (`main.rs:4356`): `try_request(peer, &Request::BlobSlice{…})` becomes
   `TcpTransport::new(peer).try_once(&Request::BlobSlice{…})`. Keep the existing comment about
   failing over fast.
3. `cmd_pull` (`main.rs:4242`): `do_pull(&mut client, &TcpTransport::new(peer), peer_name, full, Some(&custody))`.
4. `cmd_run` (`main.rs:4647`): build `let transport = TcpTransport::new(peer);` once, above the cycle
   loop, and pass `&transport`.
5. All **24** `do_pull(&mut c, &addr, …)` call sites in the test modules become
   `do_pull(&mut c, &TcpTransport::new(&addr), …)`. Find them with
   `grep -n 'do_pull(' crates/cairn-sync/src/main.rs`.
6. Import: `use cairn_wire::{…, TcpTransport, Transport};`.

`PeerRequestError`'s `source` field is `Box<dyn Error>`; a `TransportError` boxes into it unchanged, so
`chain_reaches_a_peer_frame_error` keeps working — which the Step 1 test now pins independently.

- [ ] **Step 10: Run everything**

```bash
CAIRN_ALLOW_DB_SKIP=1 cargo test -p cairn-wire
CAIRN_ALLOW_DB_SKIP=1 cargo test -p cairn-sync --all-targets
cargo clippy --workspace --all-targets -- -D warnings
```

Expected: all PASS. Pay attention to `over_cap_frame_from_a_peer_is_an_integrity_condition`-style tests
in `main.rs` (grep `chain_reaches_a_peer_frame_error`): they are the far-side proof that the boxing
still preserves the chain.

- [ ] **Step 11: Commit**

```bash
git add crates/cairn-wire crates/cairn-sync/src/main.rs
git commit -m "$(cat <<'EOF'
feat(#500): put the peer request behind a Transport trait

ADR-0026 decision 2 says clinical events back up "as a cold peer … a
configuration of the existing sync daemon". Until now cairn-sync
reached its peer through one free function that opened a TCP socket,
and a medium is not a socket, so "the medium is a peer" was prose with
nothing behind it. do_pull now takes &dyn Transport, so slice 2d's
restore can drive the SAME puller — same cursor, same quarantine pen,
same custody handling — against a file.

TcpTransport carries today's behaviour verbatim: 10 s connect, 30 s
read/write, four attempts with 250 ms doubling backoff, plus try_once
for the byte tier, which fails over to the next swarm source rather
than retrying a dead one.

TransportError has two variants because they have opposite remedies,
and its Exchange variant keeps the io::Error reachable through
source(). That is not incidental: cairn-sync tells a peer sending an
over-cap frame (an integrity condition) from a link that went away by
walking source() for io::ErrorKind::InvalidData. Formatting the cause
into a string would reclassify a hostile peer as downtime with every
existing test still green, because those tests build the error they
classify — so the property is now pinned in cairn-wire, on the error
the transport actually produces.

Closes nothing. #500 stays open.

Refs #500

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 3: The two additive wire fields, and a serve that honours the limit

**Files:**
- Modify: `crates/cairn-wire/src/wire.rs`, `crates/cairn-sync/src/main.rs` (`serve_conn`'s
  `EventsAfterSeq` arm, ~`main.rs:5149-5280`)

**Interfaces:**
- Produces: `Request::EventsAfterSeq` gains `limit: Option<u32>` (serde default `None`);
  `EventsResponse` gains `complete: bool` (serde default `false`).
- The puller still sends `limit: None` after this task, so **behaviour is unchanged**. Task 7 turns it on.

- [ ] **Step 1: Write the failing tests**

Add to `crates/cairn-wire/src/wire.rs`'s test module:

```rust
/// The additive fields must decode ABSENT, and to the values §3 of the design specifies.
/// This is principle 12's whole guarantee, and a default that drifted would be silent.
#[test]
fn the_paging_fields_decode_absent_to_their_documented_defaults() {
    let old_req = r#"{"op":"EventsAfterSeq","after_seq":0}"#;
    match serde_json::from_str::<Request>(old_req).expect("decode") {
        Request::EventsAfterSeq { limit, unwrap_cert, after_seq } => {
            assert_eq!(after_seq, 0);
            assert!(unwrap_cert.is_none());
            assert!(limit.is_none(), "an absent limit means UNPAGINATED, not zero");
        }
        other => panic!("wrong variant: {other:?}"),
    }

    let old_resp = r#"{"events":[]}"#;
    let resp: EventsResponse = serde_json::from_str(old_resp).expect("decode");
    assert!(
        !resp.complete,
        "an absent `complete` must mean THERE MAY BE MORE. The opposite default would let a \
         server that omits the field stop a puller early and silently lose events, with the \
         cursor checkpointed as if the log had been drained."
    );
}
```

- [ ] **Step 2: Run to watch it fail**

Run: `CAIRN_ALLOW_DB_SKIP=1 cargo test -p cairn-wire the_paging_fields`
Expected: FAIL — no field named `limit`.

- [ ] **Step 3: Add the fields**

In `wire.rs`, inside `Request::EventsAfterSeq`:

```rust
        /// The maximum number of events to return in this response (slice 2b, #101 item 1).
        ///
        /// `None` means UNPAGINATED — the whole suffix in one frame, which is what this
        /// protocol did before paging existed and what a caller that has no reason to page
        /// still gets. A serving node applies it as a plain SQL `LIMIT`.
        ///
        /// Additive (serde default): the field's ABSENCE is the old behaviour, so a request
        /// that predates paging means exactly what it always meant.
        #[serde(default)]
        limit: Option<u32>,
```

In `EventsResponse`:

```rust
    /// Did this response drain the peer's log above `after_seq`? (slice 2b, #101 item 1.)
    ///
    /// A paged puller loops until this is `true`. The default is FALSE — "there may be more" —
    /// and the DIRECTION is the decision, not an accident. A server that fails to set it makes
    /// a puller ask once more: wasted work. A `true` default would make the same omission stop
    /// the puller early and SILENTLY LOSE EVENTS, with the cursor checkpointed as though the
    /// log had been drained. Principle 4 applied to a protocol field: an imprecise near-truth
    /// beats a precise untruth.
    ///
    /// An empty response that does not set this is neither an end nor a continuation, and a
    /// puller must REFUSE it rather than guess — see `cairn_wire::page_decision`.
    #[serde(default)]
    complete: bool,
```

Both fields need `pub`. Every construction site of `EventsResponse` in `main.rs` (three: the two
`serve_conn` arms and the tests) now needs `complete`, and the compiler will name them.

- [ ] **Step 4: Run to verify it passes**

Run: `CAIRN_ALLOW_DB_SKIP=1 cargo test -p cairn-wire`
Expected: PASS.

- [ ] **Step 5: Write the failing test for the serve arm**

The serve arm is DB-gated. Add to `main.rs`'s `quarantine_tests` module (which already has the DB
scaffolding), modelled on the existing serve tests there:

```rust
    /// A limited request must return at most `limit` events and must declare `complete`
    /// truthfully at, below and above the boundary. `complete` is the puller's ONLY
    /// termination signal, so a serve that reports it wrongly either strands events
    /// forever or spins the puller.
    #[test]
    fn serve_honours_the_page_limit_and_declares_completeness() {
        let Some(base) = cs() else { return };
        let mut c = locked_client(&base);
        let (sk, kid) = enrolled_key(&mut c);
        // Three events, applied locally so this node has something to serve.
        for i in 0..3 {
            let bytes = peer_note(&sk, &kid, WALL_2026 + i);
            c.execute("SELECT apply_remote_event($1, NULL, NULL, NULL)", &[&bytes])
                .unwrap();
        }

        // (limit, expected event count, expected `complete`)
        let cases: [(Option<u32>, usize, bool); 4] = [
            (Some(2), 2, false),
            // THE BOUNDARY. `rows.len() == limit` is ambiguous on its own — the limit+1
            // probe is what makes this case answerable rather than guessable.
            (Some(3), 3, true),
            (Some(9), 3, true),
            (None, 3, true),
        ];
        for (limit, want_len, want_complete) in cases {
            let raw = serve_one_request(
                &base,
                &Request::EventsAfterSeq {
                    after_seq: 0,
                    unwrap_cert: None,
                    limit,
                },
            );
            let resp: EventsResponse = serde_json::from_slice(&raw).unwrap();
            assert_eq!(resp.events.len(), want_len, "limit={limit:?}");
            assert_eq!(resp.seqs.len(), want_len, "limit={limit:?}");
            assert_eq!(resp.complete, want_complete, "limit={limit:?}");
        }
    }
```

This needs one new helper beside `serve_canned`, which serves a canned response rather than running
the real arm:

```rust
    /// Run the REAL `serve_conn` against one request and return its response frame. The
    /// existing `serve_canned` serves a pre-encoded response, which cannot exercise the serve
    /// arm itself — and the serve arm is what this test is about.
    fn serve_one_request(conn: &str, req: &Request) -> Vec<u8> {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let conn = conn.to_string();
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            serve_conn(&conn, stream, false, None).unwrap();
        });
        let raw = TcpTransport::new(&addr).try_once(req).unwrap();
        server.join().unwrap();
        raw
    }
```

⚠️ `serve_conn` opens its own database connection, and `locked_client` holds the shared `'CARN'`
advisory lock for the duration of the test. Advisory locks are per-**session**, not per-database, so a
second connection from the same test does not block on the first — but confirm the suite still passes
with `--test-threads=2`, which is the workaround the flaky `clinical_pull` case established.

⚠️ The `limit = 3` case is the one that matters. `rows.len() == limit` is ambiguous — it can mean
"exactly drained" or "cut off at exactly the boundary" — and reporting `complete: true` there when a
fourth event existed would strand it. Resolve it by asking for `limit + 1` rows and returning `limit`
(see Step 6), never by guessing from the count.

- [ ] **Step 6: Run to watch it fail, then implement**

Run: `cargo test -p cairn-sync serve_honours_the_page_limit -- --test-threads=2` (with `CAIRN_TEST_PG`
exported). Expected: FAIL.

In `serve_conn`'s `EventsAfterSeq` arm, bind the new field and change the query:

```rust
        Request::EventsAfterSeq {
            after_seq,
            unwrap_cert,
            limit,
        } => {
            // Fetch ONE MORE than asked for, then serve `limit` (slice 2b). `rows.len() ==
            // limit` cannot distinguish "the log ends exactly here" from "there is a next
            // event we cut off", and `complete` is the puller's only termination signal — a
            // wrong `true` at the boundary strands every event above it, forever, with the
            // cursor checkpointed past them. The extra row answers the question instead of
            // inferring it. `LIMIT NULL` is Postgres for "no limit", so the unpaginated path
            // is the same statement with a NULL parameter rather than a second query.
            let probe: Option<i64> = limit.map(|n| i64::from(n) + 1);
            let mut rows = client.query(
                "SELECT e.seq, … FROM event_log e … WHERE e.seq > $1 ORDER BY e.seq LIMIT $2",
                &[&after_seq, &probe],
            )?;
            let complete = match limit {
                None => true,
                Some(n) => {
                    let n = n as usize;
                    let more = rows.len() > n;
                    rows.truncate(n);
                    !more
                }
            };
```

(The `SELECT` list and its `LEFT JOIN`s are unchanged — keep the existing text and its shred-guarantee
comment verbatim; only the `LIMIT $2` and the `$2` binding are new.)

Set `complete` in the `EventsResponse` this arm builds. In the **legacy `EventsAfter` arm**, set
`complete: true` with a one-line comment: that arm is unpaginated by construction, ships the whole
suffix, and has no `limit` to honour.

- [ ] **Step 7: Run the tests**

```bash
cargo test -p cairn-sync -- --test-threads=2
CAIRN_ALLOW_DB_SKIP=1 cargo test -p cairn-wire
cargo clippy --workspace --all-targets -- -D warnings
```

Expected: PASS. The puller still sends `limit: None`, so every existing pull test sees exactly today's
single unpaginated response.

- [ ] **Step 8: Commit**

```bash
git add crates/cairn-wire/src/wire.rs crates/cairn-sync/src/main.rs
git commit -m "$(cat <<'EOF'
feat(#500): EventsAfterSeq gains a batch limit; the response declares completeness

Additive on both sides (principle 12). `limit: None` is today's
unpaginated behaviour, so this commit changes no behaviour: the puller
still sends None and every existing test sees the same single response.

`complete` defaults to FALSE, and the direction is the decision. A
server that fails to set it makes a puller ask once more — wasted work.
A `true` default would make the same omission stop the puller early and
silently lose events, with the cursor checkpointed as though the log
had been drained.

The serve arm fetches limit+1 rows and returns limit. `rows.len() ==
limit` cannot distinguish "the log ends exactly here" from "there is a
next event we cut off", and a wrong `complete: true` at that boundary
strands every event above it forever. The extra row answers the
question instead of inferring it; `LIMIT NULL` keeps the unpaginated
path on the same statement.

Refs #500, #101

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 4: `MediumTransport` — a backup medium answering as a peer

**Files:**
- Create: `crates/cairn-wire/src/medium.rs`
- Modify: `crates/cairn-wire/src/lib.rs`

**Interfaces:**
- Consumes: `cairn_medium::{MediumImage, MediumV3, MediumRecord, Plane, assess, chain_report,
  MediumHealth}`; `cairn_wire::{Request, EventsResponse, Transport, TransportError}`.
- Produces:
  ```rust
  pub struct MediumTransport { /* private */ }
  impl MediumTransport {
      pub fn new(label: impl Into<String>, image: MediumImage) -> Result<Self, TransportError>;
      pub fn health(&self) -> &cairn_medium::MediumHealth;
      /// The highest clinical `source_seq` this medium can be TRUSTED to serve.
      /// `None` when it carries no verified clinical record — never `Some(0)`.
      pub fn clinical_watermark(&self) -> Option<i64>;
  }
  impl Transport for MediumTransport { … }
  ```

- [ ] **Step 1: Write the failing tests**

`crates/cairn-wire/src/medium.rs`. Build fixtures with `cairn_medium`'s public API
(`Segment`, `MediumRecord`, `serialize_v3`, `append_segment`, `parse_any`); a local
`fn record(lineage: u8, seq: i64) -> MediumRecord` helper derives its bytes at runtime —
**do not call the parameter `salt`** (house rule 6b; `crypto_sink_names_are_genuine.rs` sweeps this file).

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use cairn_medium::{append_segment, parse_any, serialize_v3, MediumRecord, Plane, Segment};

    /// One record with deterministic, runtime-derived bytes.
    ///
    /// `lineage` distinguishes one record from another and is NOT cryptographic. It must not
    /// be called `salt`/`nonce`/`iv`: CodeQL picks its sink by the NAME of the binding a
    /// constant flows into, so those three words mint a critical alert PER CALL SITE no matter
    /// how the value is computed (house rule 6b, #527). Nothing here derives a key.
    fn record(lineage: u8, seq: i64) -> MediumRecord {
        MediumRecord {
            signed_bytes: std::array::from_fn::<u8, 32, _>(|i| lineage ^ (i as u8)).to_vec(),
            attestation: None,
            attester_key: None,
            dek_wrapped: None,
            source_seq: seq,
        }
    }

    /// An UNSIGNED segment per group. Unsigned is a declared limitation, never a fault (2a
    /// invariant 7), it chains on `prev_commitment` alone, and it keeps these fixtures free of
    /// a signing key none of these properties depend on.
    fn segments(groups: &[(Plane, Vec<MediumRecord>)]) -> Vec<Segment> {
        let mut out: Vec<Segment> = Vec::new();
        for (index, (plane, records)) in groups.iter().enumerate() {
            // `segment_commitment` takes the RECORDS, not the segment.
            let prev = out
                .last()
                .map(|s: &Segment| cairn_medium::segment_commitment(&s.records))
                .unwrap_or_default();
            out.push(Segment {
                plane: *plane,
                index: index as u32,
                prev_commitment: prev,
                self_node_id_hex: String::new(),
                attestation: None,
                records: records.clone(),
            });
        }
        out
    }

    fn transport_over(groups: &[(Plane, Vec<MediumRecord>)]) -> MediumTransport {
        let bytes = serialize_v3(&segments(groups)).expect("serialize");
        MediumTransport::new("medium /tmp/test.b3", parse_any(&bytes).expect("parse"))
            .expect("a CAIRNB3 image is servable")
    }

    /// The same, with the last segment's bytes cut short — an interrupted append.
    fn transport_over_torn(groups: &[(Plane, Vec<MediumRecord>)]) -> MediumTransport {
        let segs = segments(groups);
        let mut bytes = serialize_v3(&segs[..segs.len() - 1]).expect("serialize");
        let mut tail = Vec::new();
        append_segment(&mut tail, segs.last().expect("a last segment")).expect("append");
        // Keep a prefix of the final section: fewer bytes than its length prefix claims.
        bytes.extend_from_slice(&tail[..tail.len() / 2]);
        MediumTransport::new("medium /tmp/torn.b3", parse_any(&bytes).expect("parse"))
            .expect("a torn tail is a MILD fault — never a refusal")
    }

    /// A CAIRNB2 image: no marker, no events. The argument order is `(marker, events)`.
    fn legacy_image() -> cairn_medium::MediumImage {
        let bytes = cairn_medium::serialize_container(None, &[]).expect("serialize CAIRNB2");
        parse_any(&bytes).expect("parse")
    }

    /// Ask for a page and decode it. Every test below is about the response, not the framing.
    fn events(t: &MediumTransport, after_seq: i64, limit: Option<u32>) -> EventsResponse {
        let raw = t
            .request(&Request::EventsAfterSeq {
                after_seq,
                unwrap_cert: None,
                limit,
            })
            .expect("a clinical page");
        serde_json::from_slice(&raw).expect("decode")
    }

    #[test]
    fn a_legacy_medium_is_refused_by_name_not_answered_empty() {
        // CAIRNB1/B2 carry the federation plane and NO clinical event. Answering "0 events,
        // complete" would be #500's exact signature reproduced inside the machinery built to
        // close it: an operator would read a clean, complete restore of nothing.
        let err = MediumTransport::new("medium /tmp/x", legacy_image()).expect_err("must refuse");
        assert!(matches!(err, TransportError::Unsupported { .. }), "{err}");
        assert!(err.to_string().contains("#500"), "the refusal must name the issue: {err}");
    }

    #[test]
    fn a_blob_slice_is_unsupported_not_not_found() {
        let t = transport_over(&[(Plane::Clinical, vec![record(1, 1)])]);
        let err = t
            .request(&Request::BlobSlice { addr_hex: "aa".into(), offset: 0, len: 1 })
            .expect_err("a medium has no byte tier");
        assert!(matches!(err, TransportError::Unsupported { .. }), "{err}");
    }

    #[test]
    fn the_legacy_hlc_cursor_is_unsupported() {
        let t = transport_over(&[(Plane::Clinical, vec![record(1, 1)])]);
        let err = t
            .request(&Request::EventsAfter { wall: 0, counter: 0 })
            .expect_err("records are keyed by source_seq, not HLC");
        assert!(matches!(err, TransportError::Unsupported { .. }), "{err}");
    }

    #[test]
    fn records_are_served_in_ascending_source_seq_whatever_order_the_segments_are_in() {
        // THE test for the sort. Segments sit in CAPTURE order, which is not source_seq order
        // after a re-capture. The puller's contiguous-prefix cursor RELIES on strictly
        // ascending arrival; a medium serving capture order would advance a cursor past
        // events it had not yet delivered. A fixture already in order would pass either way.
        let t = transport_over(&[
            (Plane::Clinical, vec![record(1, 5), record(2, 6)]),
            (Plane::Clinical, vec![record(3, 2), record(4, 3)]),
        ]);
        let resp = events(&t, 0, None);
        assert_eq!(resp.seqs, vec![2, 3, 5, 6]);
    }

    #[test]
    fn only_the_clinical_plane_is_served() {
        let t = transport_over(&[
            (Plane::Node, vec![record(9, 1)]),
            (Plane::Clinical, vec![record(1, 2)]),
        ]);
        assert_eq!(events(&t, 0, None).seqs, vec![2]);
    }

    #[test]
    fn after_seq_is_strict_and_the_limit_is_honoured_with_a_truthful_complete() {
        let t = transport_over(&[(Plane::Clinical, vec![record(1, 1), record(2, 2), record(3, 3)])]);
        assert_eq!(events(&t, 1, None).seqs, vec![2, 3]);          // STRICTLY greater

        let page = events(&t, 0, Some(2));
        assert_eq!(page.seqs, vec![1, 2]);
        assert!(!page.complete);

        let exact = events(&t, 0, Some(3));                          // the boundary
        assert_eq!(exact.seqs, vec![1, 2, 3]);
        assert!(exact.complete, "the limit did not bite; there is nothing above");

        assert!(events(&t, 3, Some(2)).complete);                    // drained
    }

    #[test]
    fn wrapped_deks_pass_through_byte_identical() {
        // The medium holds no secret and cannot re-wrap. This is correct ONLY because
        // ADR-0066 / DR slice 1 make `restore` ADOPT the exported unwrap secret, so the
        // restoring node's secret IS the capturing node's.
        let mut r = record(1, 1);
        r.dek_wrapped = Some(vec![7, 7, 7]);
        let t = transport_over(&[(Plane::Clinical, vec![r])]);
        let resp = events(&t, 0, None);
        assert_eq!(resp.wrapped_deks, vec![Some("070707".to_string())]);
        assert!(resp.custody_withheld.is_none(), "nothing was withheld — custody travelled");
    }

    #[test]
    fn nothing_beyond_verified_through_is_served() {
        // A torn tail is a MILD fault with an intact prefix, so the medium is not refused —
        // refusing a recoverable medium mid-disaster is what BackupError's three-way split
        // exists to prevent. Trust simply stops at `verified_through` (2a invariant 5).
        let t = transport_over_torn(&[
            (Plane::Clinical, vec![record(1, 1)]),
            (Plane::Clinical, vec![record(2, 2)]),   // in the torn tail
        ]);
        assert_eq!(events(&t, 0, None).seqs, vec![1]);
    }

    #[test]
    fn an_empty_clinical_plane_has_no_watermark_and_never_zero() {
        // 2a invariant 8: zero is a claim, absence is the honest answer.
        let t = transport_over(&[(Plane::Node, vec![record(9, 1)])]);
        assert_eq!(t.clinical_watermark(), None);
        let resp = events(&t, 0, None);
        assert!(resp.events.is_empty());
        assert!(resp.complete);
    }

    #[test]
    fn a_medium_never_declares_a_signing_context_it_does_not_record() {
        let t = transport_over(&[(Plane::Clinical, vec![record(1, 1)])]);
        assert!(events(&t, 0, None).signing_context.is_none());
    }
}
```

- [ ] **Step 2: Run to watch them fail**

Run: `CAIRN_ALLOW_DB_SKIP=1 cargo test -p cairn-wire medium`
Expected: FAIL — `MediumTransport` does not exist.

- [ ] **Step 3: Implement `medium.rs`**

```rust
//! A CAIRNB3 backup medium, answering as a sync peer (slice 2b, #500).
//!
//! This is the half of ADR-0026 decision 2 that makes "backup is a configuration of the sync
//! daemon" mechanical rather than aspirational: slice 2d's restore drives `cairn-sync`'s own
//! puller — same cursor, same quarantine pen, same custody handling — against a file.
//!
//! PURE. It reads a `MediumImage` someone else loaded; it opens no file and touches no
//! database. The caller owns the I/O.
//!
//! WHAT IT IS NOT. It serves; it does not capture. Nothing here writes a medium — that is
//! slice 2c — and `cairn-node`'s `backup.rs` still exports `node_event` and nothing else, so
//! a medium built today has an EMPTY clinical plane and this transport will truthfully say so.
//! #500 is not closed by this module existing.
```

```rust
use cairn_medium::{assess, chain_report, MediumHealth, MediumImage, MediumRecord, Plane};

use crate::transport::{Transport, TransportError};
use crate::wire::{EventsResponse, Request};

pub struct MediumTransport {
    label: String,
    /// Clinical records within `verified_through`, ascending by `source_seq`. Materialised at
    /// construction because every request re-scans the same set, and because sorting once is
    /// what makes `request` a pure lookup.
    servable: Vec<MediumRecord>,
    health: MediumHealth,
}

impl MediumTransport {
    pub fn new(label: impl Into<String>, image: MediumImage) -> Result<Self, TransportError> {
        let label = label.into();
        let MediumImage::V3(m) = image else {
            return Err(TransportError::Unsupported {
                label,
                reason: "this medium predates the two-plane format (CAIRNB1/CAIRNB2). Those \
                         revisions carry the FEDERATION plane and no clinical event at all, so \
                         there is nothing here to restore a patient record from — see issue \
                         #500. Re-capture with a build that writes CAIRNB3."
                    .into(),
            });
        };
        let chain = chain_report(&m);
        // TRUST STOPS AT `verified_through` (2a invariant 5). Serving past it would hand a
        // puller records whose chain link never held — and the puller's cursor would then
        // advance over them. `None` (nothing verified) yields an empty set, never "all".
        let through = chain.verified_through;
        let mut servable: Vec<MediumRecord> = m
            .segments
            .iter()
            .take(through.map_or(0, |t| t + 1))
            .filter(|s| s.plane == Plane::Clinical)
            .flat_map(|s| s.records.iter().cloned())
            .collect();
        // Segments sit in CAPTURE order, which stops matching source_seq order after a
        // re-capture. The puller's contiguous-prefix cursor RELIES on strictly ascending
        // arrival, so a medium serving capture order would advance a cursor past events it
        // had not yet delivered.
        servable.sort_by_key(|r| r.source_seq);
        Ok(Self {
            label,
            servable,
            health: assess(&m),
        })
    }

    /// Everything this medium can say about its own soundness. Handed out so a restore can
    /// report SCOPE honestly — a torn tail, a missing plane, records this build cannot route —
    /// rather than inferring it from an event count.
    pub fn health(&self) -> &MediumHealth {
        &self.health
    }

    /// The highest clinical `source_seq` this medium can be trusted to serve.
    ///
    /// `None` — never `Some(0)` — when it carries no verified clinical record. Zero is a
    /// claim; absence is the honest answer (2a invariant 8), and the two lead an operator to
    /// opposite conclusions about whether a restore recovered anything.
    pub fn clinical_watermark(&self) -> Option<i64> {
        self.servable.last().map(|r| r.source_seq)
    }
}

impl Transport for MediumTransport {
    fn label(&self) -> &str {
        &self.label
    }

    fn request(&self, req: &Request) -> Result<Vec<u8>, TransportError> {
        let (after_seq, limit, unwrap_cert) = match req {
            Request::EventsAfterSeq {
                after_seq,
                limit,
                unwrap_cert,
            } => (*after_seq, *limit, unwrap_cert.as_deref()),
            Request::EventsAfter { .. } => {
                return Err(TransportError::Unsupported {
                    label: self.label.clone(),
                    reason: "records on a medium are keyed by the capturing node's source_seq; \
                             there is no HLC index here. Use EventsAfterSeq."
                        .into(),
                })
            }
            Request::BlobSlice { .. } => {
                return Err(TransportError::Unsupported {
                    label: self.label.clone(),
                    reason: "a backup medium carries no byte tier — attachment bytes replicate \
                             by election, on their own resource-isolated path (ADR-0013). This \
                             is NOT 'blob absent': fetch it from a peer that holds it."
                        .into(),
                })
            }
        };

        if unwrap_cert.is_some() {
            // NEVER SILENTLY. The cert asks the server to re-wrap each DEK for the requester,
            // and a medium holds no secret, so it cannot. Saying so is the difference between
            // an operator who knows why a chart will not render and one who does not.
            eprintln!(
                "{}: the unwrap cert in this request is IGNORED. A medium holds no secret and \
                 cannot re-wrap; every DEK travels wrapped to the CAPTURING node's unwrap key, \
                 verbatim. That is correct only because ADR-0066 makes `restore` ADOPT the \
                 exported unwrap secret, so the restoring node's secret IS the capturing \
                 node's. If that adoption path ever changes, this stops working and a restored \
                 node will hold DEKs it cannot open.",
                self.label
            );
        }

        // Fetch one more than asked, keep `limit`. `len() == limit` cannot distinguish "the
        // set ends here" from "there is a next record we cut off", and `complete` is the
        // puller's only termination signal — the same reasoning as the serve arm's limit+1.
        let start = self.servable.partition_point(|r| r.source_seq <= after_seq);
        let rest = &self.servable[start..];
        let (page, complete) = match limit {
            None => (rest, true),
            Some(n) => {
                let n = n as usize;
                (&rest[..n.min(rest.len())], rest.len() <= n)
            }
        };

        let resp = EventsResponse {
            events: page.iter().map(|r| hex::encode(&r.signed_bytes)).collect(),
            attestations: page.iter().map(|r| r.attestation.as_ref().map(hex::encode)).collect(),
            attester_keys: page.iter().map(|r| r.attester_key.as_ref().map(hex::encode)).collect(),
            seqs: page.iter().map(|r| r.source_seq).collect(),
            // A medium does not record the ADR-0040 signing context its records were minted
            // under, so it declares none rather than guessing. The puller falls back to its
            // all-unverifiable heuristic (#108) — a degraded diagnosis, not a wrong answer.
            // Slice 2c writes the segments and could carry it.
            signing_context: None,
            // Verbatim: wrapped to the CAPTURING node's key. See the unwrap_cert note above.
            wrapped_deks: page.iter().map(|r| r.dek_wrapped.as_ref().map(hex::encode)).collect(),
            // Nothing was withheld. `None` here means "custody travelled, or there was none to
            // send" — which is exactly true of a pass-through.
            custody_withheld: None,
            complete,
        };
        serde_json::to_vec(&resp).map_err(|e| TransportError::Exchange {
            label: self.label.clone(),
            source: Box::new(e),
        })
    }
}
```

Add `mod medium;` and `pub use medium::MediumTransport;` to `lib.rs`.

⚠️ `partition_point` needs `servable` sorted, which `new` guarantees — the two must not drift apart.
Note the dependency in a comment on the sort.

- [ ] **Step 4: Run to verify they pass**

Run: `CAIRN_ALLOW_DB_SKIP=1 cargo test -p cairn-wire`
Expected: PASS (all ten medium tests plus the earlier ones).

- [ ] **Step 5: Commit**

```bash
git add crates/cairn-wire
git commit -m "$(cat <<'EOF'
feat(#500): a backup medium answers as a sync peer

MediumTransport serves the clinical plane of a CAIRNB3 image through
the same request/response protocol a network peer speaks, so slice 2d's
restore can drive cairn-sync's own puller against a file — same cursor,
same quarantine pen, same custody handling.

Three refusals, each NAMED rather than answered with a plausible empty
response. A CAIRNB1/B2 image, a blob slice, and the legacy HLC cursor
all return Unsupported. A medium that answered "0 events, complete"
would be #500's exact signature reproduced inside the machinery built
to close it: a clean, complete restore of nothing.

It does NOT refuse an unsound medium. A torn tail is a mild fault with
an intact prefix, and refusing a recoverable medium mid-disaster is
what BackupError's three-way split exists to prevent. Trust stops at
verified_through instead (2a invariant 5), and health() and
clinical_watermark() are exposed so 2d can report scope honestly —
watermark is None, never Some(0), for a medium with no clinical record.

Records are sorted by source_seq. Segments sit in CAPTURE order, which
stops matching after a re-capture, and the puller's contiguous-prefix
cursor relies on strictly ascending arrival.

wrapped_deks pass through byte-identical: the medium holds no secret
and cannot re-wrap, which is correct only because ADR-0066 makes
restore ADOPT the exported unwrap secret. That precondition is stated
next to the code, not left to inference. A request carrying an unwrap
cert is told it was ignored, never silently ignored.

Closes nothing. Nothing writes a medium yet (2c) and nothing restores
one (2d), so a medium built today has an empty clinical plane.

Refs #500

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 5: The pure paging decisions

**Files:**
- Create: `crates/cairn-wire/src/page.rs`, `crates/cairn-sync/src/pull_page.rs`
- Modify: `crates/cairn-wire/src/lib.rs`, `crates/cairn-sync/src/main.rs`

**Interfaces:**
- Produces, in `cairn_wire`:
  ```rust
  pub const DEFAULT_PAGE_EVENTS: u32 = 500;
  #[derive(Debug, PartialEq, Eq)]
  pub enum PageDecision { Done, Continue, Refuse(String) }
  pub fn page_decision(complete: bool, page_len: usize, frozen: bool) -> PageDecision;
  ```
- Produces, in `cairn-sync`'s `pull_page`:
  ```rust
  pub(crate) fn quarantine_floor(
      skipped_unverifiable: usize,
      refused_verifiable: usize,
      pen_failed: bool,
      pin: Option<i64>,
      floor_at_start: Option<i64>,
  ) -> Option<i64>;
  ```

- [ ] **Step 1: Write the failing test for `page_decision`**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_complete_page_ends_the_loop() {
        assert_eq!(page_decision(true, 500, false), PageDecision::Done);
        assert_eq!(page_decision(true, 0, false), PageDecision::Done);
    }

    #[test]
    fn an_incomplete_non_empty_page_continues() {
        assert_eq!(page_decision(false, 500, false), PageDecision::Continue);
        assert_eq!(page_decision(false, 1, false), PageDecision::Continue);
    }

    #[test]
    fn an_empty_page_that_does_not_declare_completeness_is_refused() {
        // Neither an end nor a continuation. Treating it as the end risks a silent early
        // stop with the cursor checkpointed as if the log were drained; continuing spins
        // forever against the same cursor. The peer ANSWERED and the answer is unusable,
        // which is an integrity condition.
        match page_decision(false, 0, false) {
            PageDecision::Refuse(why) => assert!(why.contains("complete"), "{why}"),
            other => panic!("must refuse, got {other:?}"),
        }
    }

    #[test]
    fn a_frozen_cursor_ends_the_loop_whatever_the_page_said() {
        // Fetching another page after a freeze would pull events the puller has already
        // decided it will not handle, and the checkpoint cannot advance past the freeze.
        assert_eq!(page_decision(false, 500, true), PageDecision::Done);
        assert_eq!(page_decision(false, 0, true), PageDecision::Done);
    }
}
```

⚠️ Note the ordering the last test pins: **frozen is checked before the empty-page refusal.** A frozen
cycle is not a peer fault, and reporting it as one would send an operator to audit a healthy peer.

- [ ] **Step 2: Run to watch it fail, then implement `page.rs`**

Run: `CAIRN_ALLOW_DB_SKIP=1 cargo test -p cairn-wire page` → FAIL.

```rust
//! The paging contract: how big a page is, and how a puller knows it has reached the end.

/// Events per page by default (slice 2b, #101 item 1).
///
/// At roughly 4 KiB per event on the wire (≈1.5 KiB signed, hex-doubled, plus attestation and
/// wrapped DEK) that is about 2 MiB per page: 32× under [`crate::MAX_FRAME_BYTES`], and
/// comfortably inside the 30 s read timeout on the 700 ms-RTT double-Starlink link Spike 0001
/// measures against. A 20k-event sweep becomes 40 round trips, about 30 s of accumulated
/// latency — paid once, on a full sweep, in exchange for progress that survives an interruption.
pub const DEFAULT_PAGE_EVENTS: u32 = 500;

#[derive(Debug, PartialEq, Eq)]
pub enum PageDecision {
    /// Stop. Either the peer drained its log, or this cycle froze.
    Done,
    /// Ask for the next page from the advanced cursor.
    Continue,
    /// The peer answered with something no puller can act on. The string is the operator's
    /// diagnosis.
    Refuse(String),
}

/// Decide what to do after one page. **Pure**, so all four states are tested with no peer,
/// no socket and no database.
pub fn page_decision(complete: bool, page_len: usize, frozen: bool) -> PageDecision {
    // FROZEN FIRST, and the order is load-bearing. A freeze is this node's decision, not a
    // peer fault; letting the empty-page refusal below claim it would send an operator to
    // audit a healthy peer. It also cannot make progress: the checkpoint will not advance
    // past the freeze, so a next page would re-fetch what we have already declined to handle.
    if frozen {
        return PageDecision::Done;
    }
    if complete {
        return PageDecision::Done;
    }
    if page_len == 0 {
        return PageDecision::Refuse(
            "the peer returned an EMPTY page without declaring the stream complete. That is \
             neither an end nor a continuation: treating it as the end would checkpoint the \
             cursor as though the log were drained and silently strand every event above it, \
             and continuing would re-request the same cursor forever. The peer answered and \
             its wire format is the problem — check that it sets `complete` on every response."
                .to_string(),
        );
    }
    PageDecision::Continue
}
```

Add `mod page; pub use page::{page_decision, PageDecision, DEFAULT_PAGE_EVENTS};` to `lib.rs`.

- [ ] **Step 3: Run to verify it passes**

Run: `CAIRN_ALLOW_DB_SKIP=1 cargo test -p cairn-wire page`
Expected: PASS (4 tests).

- [ ] **Step 4: Write the failing test for `quarantine_floor`**

`crates/cairn-sync/src/pull_page.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_clean_cycle_clears_the_floor() {
        assert_eq!(quarantine_floor(0, 0, false, None, Some(5)), None);
    }

    #[test]
    fn unacked_refusals_with_a_healthy_pen_pin_at_the_first_refused_slot() {
        assert_eq!(quarantine_floor(1, 0, false, Some(7), Some(5)), Some(7));
        assert_eq!(quarantine_floor(0, 1, false, Some(7), None), Some(7));
    }

    #[test]
    fn a_pen_failure_keeps_the_most_conservative_of_the_old_floor_and_the_new_pin() {
        // A re-offered slot whose pen write FAILED produced no pin, so overwriting blindly
        // would clear a floor guarding a slot the cursor is already above — permanent
        // exclusion.
        assert_eq!(quarantine_floor(1, 0, true, Some(9), Some(5)), Some(5));
        assert_eq!(quarantine_floor(1, 0, true, None, Some(5)), Some(5));
        assert_eq!(quarantine_floor(0, 0, true, Some(9), None), Some(9));
    }

    /// THE defect paging could introduce. Page 1 refuses a slot and pins the floor; page 2 is
    /// clean. Computed PER PAGE, page 2 would CLEAR the pin page 1 set — and the cursor has
    /// already advanced past that refused event, so it would never be re-offered again.
    /// Computed over the CYCLE, the refusal is still counted and the floor still stands.
    #[test]
    fn a_clean_later_page_cannot_clear_a_pin_an_earlier_page_set() {
        let mut cycle = CycleTally::new(0);
        cycle.fold(PageTally { skipped_unverifiable: 1, pin: Some(7), ..PageTally::default() });
        cycle.fold(PageTally::default());   // a wholly clean page 2
        assert_eq!(
            quarantine_floor(
                cycle.skipped_unverifiable,
                cycle.refused_verifiable,
                cycle.pen_refused.is_some(),
                cycle.pin,
                None
            ),
            Some(7)
        );
    }

    #[test]
    fn the_earliest_pin_wins_whichever_page_carried_it() {
        let mut cycle = CycleTally::new(0);
        cycle.fold(PageTally { refused_verifiable: 1, pin: Some(9), ..PageTally::default() });
        cycle.fold(PageTally { refused_verifiable: 1, pin: Some(4), ..PageTally::default() });
        assert_eq!(cycle.pin, Some(4), "min, not first-wins: order-independent by construction");
    }

    #[test]
    fn folding_sums_the_counters_and_makes_the_flags_sticky() {
        let mut cycle = CycleTally::new(10);
        cycle.fold(PageTally { applied: 3, shipped: 5, event_bytes: 100, max_seq: 14,
                               custody_withheld: true, ..PageTally::default() });
        cycle.fold(PageTally { applied: 2, shipped: 5, event_bytes: 90, max_seq: 19,
                               frozen: true, ..PageTally::default() });
        assert_eq!((cycle.applied, cycle.shipped, cycle.event_bytes), (5, 10, 190));
        assert_eq!(cycle.max_seq, 19);
        assert!(cycle.frozen && cycle.custody_withheld);
    }
}
```

- [ ] **Step 5: Run to watch it fail, then implement `pull_page.rs`**

Run: `CAIRN_ALLOW_DB_SKIP=1 cargo test -p cairn-sync --all-targets pull_page` → FAIL.

```rust
//! What one page of a pull contributed, and what a whole cycle adds up to.
//!
//! WHY A MODULE: `do_pull` used to make ONE request and hold its per-cycle counters as a dozen
//! local `mut` bindings. Paging turns that into a loop, and the folding rules — which counters
//! sum, which flags are sticky, which value wins when two pages disagree — become decisions
//! worth testing on their own. They are pure, so they are tested with no peer and no database.
//! (New code lives here rather than in `main.rs`, which is 11.5k lines: #531.)

/// What ONE page contributed.
///
/// `Default` is what makes the fold tests readable: a test that cares about one field says so
/// with `..PageTally::default()` instead of naming thirteen it does not care about.
#[derive(Debug, Default)]
pub(crate) struct PageTally {
    /// Events the peer shipped in this page (`resp.events.len()`).
    pub(crate) shipped: usize,
    /// Events the in-DB apply door admitted as NEW.
    pub(crate) applied: usize,
    /// Entries that could not be verified and were penned (bad signature, garbage, non-hex).
    pub(crate) skipped_unverifiable: usize,
    /// Verifiable events this node's floor deliberately refused and penned (ADR-0056 d5, #267).
    pub(crate) refused_verifiable: usize,
    /// Re-offered slots a human had already acked, skipped without pinning the floor.
    pub(crate) skipped_acked: usize,
    /// Decoded signed_bytes, summed — the payload half of `bytes_per_event`.
    pub(crate) event_bytes: usize,
    /// The response frame's length on the wire.
    pub(crate) wire_bytes: usize,
    /// Highest CONTIGUOUS handled seq after this page. Seeded from the cycle's current value,
    /// so folding takes this one rather than a max.
    pub(crate) max_seq: i64,
    /// The cursor halted in this page.
    pub(crate) frozen: bool,
    /// An apply failure in this page landed on THIS NODE'S database (PR #493).
    pub(crate) local_apply_fault: bool,
    /// The pen's own refusal (quota or insert failure), if it refused.
    pub(crate) pen_refused: Option<PenRefusal>,
    /// The seq of the FIRST unacked refused event in this page.
    pub(crate) pin: Option<i64>,
    /// The peer deliberately withheld custody for this page (ADR-0052, #231).
    pub(crate) custody_withheld: bool,
    /// Content addresses of every event the door ADMITTED, for the #465 ledger read.
    pub(crate) applied_addresses: Vec<Vec<u8>>,
}

/// What the whole cycle has contributed so far. Same fields, accumulated.
#[derive(Debug)]
pub(crate) struct CycleTally {
    pub(crate) shipped: usize,
    pub(crate) applied: usize,
    pub(crate) skipped_unverifiable: usize,
    pub(crate) refused_verifiable: usize,
    pub(crate) skipped_acked: usize,
    pub(crate) event_bytes: usize,
    pub(crate) wire_bytes: usize,
    pub(crate) max_seq: i64,
    pub(crate) frozen: bool,
    pub(crate) local_apply_fault: bool,
    pub(crate) pen_refused: Option<PenRefusal>,
    pub(crate) pin: Option<i64>,
    pub(crate) custody_withheld: bool,
    pub(crate) applied_addresses: Vec<Vec<u8>>,
    /// Pages folded so far — reported as a metric, and the thing to look at when a cycle is
    /// slow for a reason no counter explains.
    pub(crate) pages: usize,
}

impl CycleTally {
    /// `max_seq` starts at the COMMITTED cursor so re-offered low-seq events (below it, kept
    /// on the wire by the floor) never rewind the checkpoint.
    pub(crate) fn new(last_seq: i64) -> Self {
        Self {
            shipped: 0,
            applied: 0,
            skipped_unverifiable: 0,
            refused_verifiable: 0,
            skipped_acked: 0,
            event_bytes: 0,
            wire_bytes: 0,
            max_seq: last_seq,
            frozen: false,
            local_apply_fault: false,
            pen_refused: None,
            pin: None,
            custody_withheld: false,
            applied_addresses: Vec::new(),
            pages: 0,
        }
    }

    /// Fold one page in. See the module doc for why each rule is what it is.
    pub(crate) fn fold(&mut self, page: PageTally) {
        self.shipped += page.shipped;
        self.applied += page.applied;
        self.skipped_unverifiable += page.skipped_unverifiable;
        self.refused_verifiable += page.refused_verifiable;
        self.skipped_acked += page.skipped_acked;
        self.event_bytes += page.event_bytes;
        self.wire_bytes += page.wire_bytes;
        // TAKE, not max: a page's `max_seq` is seeded from this value and only ever advances
        // over its own contiguous handled prefix, so it is already the running answer.
        self.max_seq = page.max_seq;
        self.frozen |= page.frozen;
        self.local_apply_fault |= page.local_apply_fault;
        self.custody_withheld |= page.custody_withheld;
        self.applied_addresses.extend(page.applied_addresses);
        if let Some(next) = page.pen_refused {
            // `merge_pen_refusal` already encodes the cross-refusal rule for a CYCLE:
            // message first-wins (text and class must describe the same event), `local_fault`
            // OR-ed (it is a fact about this node's uptime, not about one event).
            self.pen_refused = Some(crate::merge_pen_refusal(self.pen_refused.take(), next));
        }
        self.pin = match (self.pin, page.pin) {
            // MIN, not first-wins. Pages arrive in ascending seq so the two agree today, but
            // min is order-independent, and the floor's whole job is to be conservative.
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        self.pages += 1;
    }
}

/// The re-offer floor for this cycle. **Pure**, and computed over the CYCLE, never a page.
///
/// The three branches are unchanged from the single-shot version; what is new is the subject.
/// Per page, a clean page 2 would clear the pin a refusing page 1 set — and the cursor has
/// already advanced past that refused event, so it would never be re-offered again. Silent
/// exclusion is precisely what this floor exists to prevent.
pub(crate) fn quarantine_floor(
    skipped_unverifiable: usize,
    refused_verifiable: usize,
    pen_failed: bool,
    pin: Option<i64>,
    floor_at_start: Option<i64>,
) -> Option<i64> {
    if skipped_unverifiable == 0 && refused_verifiable == 0 && !pen_failed {
        None
    } else if !pen_failed {
        pin
    } else {
        match (floor_at_start, pin) {
            (Some(f), Some(p)) => Some(f.min(p)),
            (Some(f), None) => Some(f),
            (None, p) => p,
        }
    }
}
```

The module needs `use crate::{merge_pen_refusal, PenRefusal};` — both are private to the crate root
(`main.rs`), which makes them reachable from a child module through `crate::`, and neither needs to
become `pub`.

- [ ] **Step 6: Wire `quarantine_floor` into today's single-shot `do_pull`**

Replace the inline `let new_floor: Option<i64> = if … ;` expression (`main.rs:3567-3577`) with a call.
Add `mod pull_page;` beside `mod unwrap_key;` in `main.rs`. **This is a pure refactor: no behaviour
changes.** `CycleTally`/`PageTally`/`fold` are defined but not yet used by `do_pull`; add
`#[allow(dead_code)]` on them with a comment naming Task 6, or land Tasks 5 and 6 in one commit if the
workspace's `deny(warnings)` makes that awkward.

- [ ] **Step 7: Run everything**

```bash
CAIRN_ALLOW_DB_SKIP=1 cargo test -p cairn-wire
cargo test -p cairn-sync -- --test-threads=2
cargo clippy --workspace --all-targets -- -D warnings
```

Expected: PASS, with the existing floor-behaviour tests in `quarantine_tests` unchanged and green.

- [ ] **Step 8: Commit**

```bash
git add crates/cairn-wire/src crates/cairn-sync/src
git commit -m "$(cat <<'EOF'
feat(#500): the paging decisions, as pure functions

page_decision covers four states with no peer and no socket. Frozen is
checked FIRST: a freeze is this node's decision, not a peer fault, and
letting the empty-page refusal claim it would send an operator to audit
a healthy peer.

quarantine_floor is today's three-branch rule, unchanged, lifted out of
a 740-line function so it can be tested directly for the first time.
The subject is what is new: the rule is computed over the CYCLE, never
a page. Per page, a clean page 2 would clear the pin a refusing page 1
set — and the cursor has already advanced past that refused event, so
it would never be re-offered. Silent exclusion is exactly what this
floor exists to prevent, and the case now has a test.

CycleTally::fold takes the MINIMUM pin rather than first-wins. Pages
arrive in ascending seq so the two agree today; min is
order-independent, and a floor's job is to be conservative.

Pure refactor: no behaviour change. The puller still makes one request.

Refs #500, #101, #531

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 6: Extract `apply_page` — a pure refactor, still one page

**Files:**
- Modify: `crates/cairn-sync/src/main.rs`

**Interfaces:**
- Produces: `fn apply_page(client: &mut postgres::Client, ctx: &PageContext<'_>, resp: &EventsResponse,
  wire_bytes: usize, max_seq_in: i64) -> PageTally`, and
  `fn validate_page(resp: &EventsResponse, peer_name: &str, after_seq: i64) -> Result<(), PullIntegrityError>`.
- `PageContext<'a> { peer_name: &'a str, unwrap_secret: Option<&'a [u8]> }` — whatever the existing loop
  body closes over; the compiler will name the rest.

**This task changes no behaviour.** It is the verbatim-move-then-change discipline: the paging diff in
Task 7 must be readable against a small function, not against a 740-line one.

- [ ] **Step 1: Run the existing suite and record the baseline**

```bash
cargo test -p cairn-sync -- --test-threads=2 2>&1 | grep -E "^test result"
```
Write the pass counts down. They must be identical at Step 5.

- [ ] **Step 2: Extract `validate_page`**

Move the three guards — the signing-context skew check (`main.rs:3151-3166`), the
`events.len() != seqs.len()` check (3170-3186), and the ascending/positive `seqs` check
(3188-3210) — into one function returning `Result<(), PullIntegrityError>`. Move the doc comments
with them, verbatim.

**Add one guard paging requires** (and write its test first, in `quarantine_tests`, using the existing
hostile-serve fixtures):

```rust
    // Paging adds a fourth requirement the single-shot version did not need: the page's
    // FIRST seq must be strictly above the cursor we asked from. A well-formed serve
    // (`WHERE seq > $1`) always satisfies it. A peer that does not would make the next
    // page's cursor go BACKWARDS, and the loop would request the same page forever —
    // turning a hostile peer into an unbounded loop rather than a refused batch.
    if let Some(&first) = resp.seqs.first() {
        if first <= after_seq {
            return Err(PullIntegrityError {
                message: format!(
                    "pull {peer_name}: peer returned seq {first} for a request that asked for \
                     events strictly above {after_seq} — refusing to page from these values. A \
                     well-formed serve streams `WHERE seq > $1`, so this peer is buggy or \
                     hostile; paging from it would send the next page's cursor backwards and \
                     re-request the same page forever."
                ),
                metrics: serde_json::Value::Null,
                // Refused before any per-event work: this node's database was never asked.
                also_local_fault: false,
            });
        }
    }
```

- [ ] **Step 3: Extract `apply_page`**

Move the body from `let (mut applied, …)` (`main.rs:3243`) through the end of the `for` loop
(`main.rs:3550`) into `apply_page`, returning a `PageTally` instead of leaving locals behind. Keep
every comment. `attachment_flag_watermark` stays in `do_pull` — it must be read once, **before** the
first page, or the unlearnable-reference report goes blind to page 1.

`do_pull` then reads:

```rust
    let page = apply_page(client, &ctx, &resp, wire_bytes, last_seq);
    let mut cycle = CycleTally::new(last_seq);
    cycle.fold(page);
```

…and the metrics object and cursor commit below read from `cycle` instead of the old locals.

- [ ] **Step 4: Run the suite**

```bash
cargo test -p cairn-sync -- --test-threads=2 2>&1 | grep -E "^test result"
cargo clippy --workspace --all-targets -- -D warnings
```

- [ ] **Step 5: Compare against the baseline**

Expected: **identical** pass counts, plus one for the new backwards-cursor guard. Any other change
means the move was not verbatim.

- [ ] **Step 6: Commit**

```bash
git add crates/cairn-sync/src/main.rs
git commit -m "$(cat <<'EOF'
refactor(#500): lift one page's apply out of do_pull

Pure refactor ahead of paging: the loop that Task 7 adds must be
reviewable against a small function, not against a 740-line one. Same
counts, same behaviour, comments moved verbatim.

One guard is genuinely new, and paging is why. The page's first seq
must be strictly above the cursor it was requested from. A well-formed
serve (WHERE seq > $1) always satisfies it; a peer that does not would
send the next page's cursor BACKWARDS and the loop would request the
same page forever — a hostile peer becoming an unbounded loop instead
of a refused batch.

attachment_flag_watermark stays in do_pull deliberately: it is read
once, before the first page, or the unlearnable-reference report (#465)
goes blind to everything page 1 admitted.

Refs #500

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 7: The page loop

**Files:**
- Modify: `crates/cairn-sync/src/main.rs` (`do_pull`, `cmd_pull`, `cmd_run`, `usage`, `main`)

**Interfaces:**
- Consumes: everything from Tasks 1–6.
- Produces: `do_pull(client, transport: &dyn Transport, peer_name: &str, full_sweep: bool,
  page_limit: u32, custody: Option<&unwrap_key::NodeCustody>) -> R<serde_json::Value>`;
  `--page N` on `pull` and `run`, defaulting to `DEFAULT_PAGE_EVENTS`.

- [ ] **Step 1: Write the failing tests with a `FakeTransport`**

This is the first time `do_pull` becomes testable without a socket. Add `FakeTransport` to `main.rs`'s
`quarantine_tests` module (the assertions need its DB fixtures): it hands out canned frames in order and
**records the `(after_seq, limit)` of every request**, which is what makes the cursor assertions
possible at all — `serve_canned` discards the request it reads.

```rust
    /// Serves canned frames in order and records what was asked for. `Request` is not
    /// `Clone`, so the interesting fields are recorded rather than the whole value.
    struct FakeTransport {
        pages: Mutex<VecDeque<Vec<u8>>>,
        seen: Mutex<Vec<(i64, Option<u32>)>>,
    }

    impl FakeTransport {
        fn new(pages: Vec<Vec<u8>>) -> Self {
            Self {
                pages: Mutex::new(pages.into()),
                seen: Mutex::new(Vec::new()),
            }
        }
        /// The `(after_seq, limit)` of every request, in order.
        fn requests(&self) -> Vec<(i64, Option<u32>)> {
            self.seen.lock().unwrap().clone()
        }
    }

    impl Transport for FakeTransport {
        fn label(&self) -> &str {
            "fake"
        }
        fn request(&self, req: &Request) -> Result<Vec<u8>, TransportError> {
            match req {
                Request::EventsAfterSeq {
                    after_seq, limit, ..
                } => self.seen.lock().unwrap().push((*after_seq, *limit)),
                other => panic!("the puller must only send EventsAfterSeq, got {other:?}"),
            }
            self.pages
                .lock()
                .unwrap()
                .pop_front()
                // Running out of canned pages is a TEST bug, not a transport failure, and it
                // must not masquerade as a partition the puller then reports as downtime.
                .ok_or_else(|| panic!("the puller asked for more pages than the test canned"))
        }
    }

    /// A page of `events` at explicit seqs, with an explicit completeness declaration.
    /// `response_json` always seqs from 1 and always says complete; paging needs both varied.
    fn page_json(events: &[&[u8]], seqs: &[i64], complete: bool) -> Vec<u8> {
        assert_eq!(events.len(), seqs.len(), "parallel arrays");
        serde_json::to_vec(&EventsResponse {
            events: events.iter().map(hex::encode).collect(),
            attestations: vec![None; events.len()],
            attester_keys: vec![None; events.len()],
            seqs: seqs.to_vec(),
            signing_context: Some(CTX_EVENT.as_str().to_string()),
            wrapped_deks: vec![None; events.len()],
            custody_withheld: None,
            complete,
        })
        .unwrap()
    }

    /// Three pages of two events each converge, and the cursor ends at the last seq.
    #[test]
    fn a_paged_pull_converges_across_pages() {
        let Some(base) = cs() else { return };
        let mut c = locked_client(&base);
        let (sk, kid) = enrolled_key(&mut c);
        let ev: Vec<Vec<u8>> = (0..6).map(|i| peer_note(&sk, &kid, WALL_2026 + i)).collect();
        let t = FakeTransport::new(vec![
            page_json(&[&ev[0], &ev[1]], &[1, 2], false),
            page_json(&[&ev[2], &ev[3]], &[3, 4], false),
            page_json(&[&ev[4], &ev[5]], &[5, 6], true),
        ]);

        let m = do_pull(&mut c, &t, "peer-a", false, 2, None).unwrap();

        assert_eq!(m["shipped"], 6, "counters sum across pages");
        assert_eq!(m["applied_new"], 6);
        assert_eq!(m["cursor_seq"], 6);
        assert_eq!(m["pages"], 3);
        assert_eq!(
            t.requests(),
            vec![(0, Some(2)), (2, Some(2)), (4, Some(2))],
            "each page is fetched from the last seq RECEIVED"
        );
        assert_eq!(cursor(&mut c, "peer-a"), 6);
    }

    /// #101 item 1, the actual fix. A cycle that dies after page 2 must leave a durable
    /// checkpoint, so the next pull RESUMES rather than restarting from zero.
    #[test]
    fn an_interrupted_cycle_resumes_from_its_per_page_checkpoint() {
        let Some(base) = cs() else { return };
        let mut c = locked_client(&base);
        let (sk, kid) = enrolled_key(&mut c);
        let ev: Vec<Vec<u8>> = (0..6).map(|i| peer_note(&sk, &kid, WALL_2026 + i)).collect();

        // Two good pages, then a page whose seqs go BACKWARDS — a wire-format fault that
        // aborts the cycle after page 2 has already been applied and checkpointed.
        let dying = FakeTransport::new(vec![
            page_json(&[&ev[0], &ev[1]], &[1, 2], false),
            page_json(&[&ev[2], &ev[3]], &[3, 4], false),
            page_json(&[&ev[4]], &[1], false),
        ]);
        do_pull(&mut c, &dying, "peer-a", false, 2, None).expect_err("page 3 is malformed");
        assert_eq!(
            cursor(&mut c, "peer-a"),
            4,
            "pages 1-2 are durable: the whole point of a per-page checkpoint"
        );

        // The next pull picks up at 4, not 0.
        let resumed = FakeTransport::new(vec![page_json(&[&ev[4], &ev[5]], &[5, 6], true)]);
        do_pull(&mut c, &resumed, "peer-a", false, 2, None).unwrap();
        assert_eq!(resumed.requests(), vec![(4, Some(2))]);
        assert_eq!(cursor(&mut c, "peer-a"), 6);
    }

    /// An empty page that does not declare completeness is an INTEGRITY condition — the peer
    /// answered and the answer is unusable — never a partition, and never a quiet stop.
    #[test]
    fn an_empty_page_without_complete_fails_the_pull_loudly() {
        let Some(base) = cs() else { return };
        let mut c = locked_client(&base);
        let (sk, kid) = enrolled_key(&mut c);
        let ev = peer_note(&sk, &kid, WALL_2026);
        let t = FakeTransport::new(vec![
            page_json(&[&ev], &[1], false),
            page_json(&[], &[], false),
        ]);

        let boxed = do_pull(&mut c, &t, "peer-a", false, 1, None).expect_err("must refuse");
        assert!(
            boxed.downcast_ref::<PullIntegrityError>().is_some(),
            "integrity, not partition: {boxed}"
        );
        assert!(boxed.to_string().contains("complete"), "{boxed}");
        assert_eq!(cursor(&mut c, "peer-a"), 1, "page 1's progress is still durable");
    }

    /// A freeze must stop the loop. Asserted on the REQUEST COUNT, because a puller that kept
    /// asking would keep re-fetching events it has already declined to handle.
    #[test]
    fn a_freeze_stops_the_loop_and_no_further_page_is_requested() {
        let Some(base) = cs() else { return };
        let mut c = locked_client(&base);
        let (sk, kid) = enrolled_key(&mut c);
        let good = peer_note(&sk, &kid, WALL_2026);
        // A second event whose apply fails TRANSIENTLY freezes the cursor. Reuse whichever
        // fixture the module already uses for its transient-fault freeze test (grep
        // `watermark_frozen`) rather than inventing a second way to provoke one.
        let t = FakeTransport::new(vec![
            page_json(&[&good, &freezing_event(&mut c, &sk, &kid)], &[1, 2], false),
            page_json(&[&good], &[3], true), // must never be requested
        ]);

        let m = do_pull(&mut c, &t, "peer-a", false, 2, None).unwrap_err();
        let _ = m; // a frozen cycle fails loudly (#270); the assertion below is the point
        assert_eq!(t.requests().len(), 1, "the loop must stop at the freeze");
    }

    /// A full sweep's page 2 continues from the last seq RECEIVED, not from the checkpoint.
    ///
    /// THE SUBTLE ONE. `max_seq` starts at the COMMITTED cursor, so paging from it would make
    /// page 2 of a sweep jump from 2 straight past everything up to the old cursor — silently
    /// skipping exactly the events a sweep exists to reconcile. With a cursor already at 4 and
    /// a sweep starting at 0, the two candidate values differ visibly.
    #[test]
    fn a_full_sweep_pages_from_the_last_seq_received_not_from_the_checkpoint() {
        let Some(base) = cs() else { return };
        let mut c = locked_client(&base);
        let (sk, kid) = enrolled_key(&mut c);
        let ev: Vec<Vec<u8>> = (0..4).map(|i| peer_note(&sk, &kid, WALL_2026 + i)).collect();

        // Get the committed cursor to 4 with an ordinary incremental pull.
        let warm = FakeTransport::new(vec![page_json(
            &[&ev[0], &ev[1], &ev[2], &ev[3]],
            &[1, 2, 3, 4],
            true,
        )]);
        do_pull(&mut c, &warm, "peer-a", false, 10, None).unwrap();
        assert_eq!(cursor(&mut c, "peer-a"), 4);

        // Now sweep from 0 with a page size of 2.
        let sweep = FakeTransport::new(vec![
            page_json(&[&ev[0], &ev[1]], &[1, 2], false),
            page_json(&[&ev[2], &ev[3]], &[3, 4], true),
        ]);
        do_pull(&mut c, &sweep, "peer-a", true, 2, None).unwrap();
        assert_eq!(
            sweep.requests(),
            vec![(0, Some(2)), (2, Some(2))],
            "page 2 must ask from 2 (the last seq RECEIVED), never from 4 (the checkpoint)"
        );
    }
```

`cursor(&mut c, "peer-a")` is the existing `sync_state.last_seq` helper at `main.rs:7718`;
`freezing_event` is whatever the module's existing transient-fault freeze test builds — find it with
`grep -n 'watermark_frozen.*true' crates/cairn-sync/src/main.rs` and reuse it rather than inventing a
second way to provoke a freeze.

- [ ] **Step 2: Run to watch them fail**

Run: `cargo test -p cairn-sync a_paged_pull -- --test-threads=2` → FAIL.

- [ ] **Step 3: Implement the loop**

```rust
    let mut cycle = CycleTally::new(last_seq);
    let mut page_cursor = after_seq;              // the FETCH point, not the checkpoint
    let mut first_signing_context: Option<String> = None;
    let mut cursor_commit: Result<(), String> = Ok(());

    loop {
        let raw = transport
            .request(&Request::EventsAfterSeq {
                after_seq: page_cursor,
                unwrap_cert: unwrap_cert.clone(),
                limit: Some(page_limit),
            })
            .map_err(|e| PeerRequestError { message: /* the existing text */, source: Box::new(e) })?;
        let wire_bytes = raw.len();
        let resp: EventsResponse = serde_json::from_slice(&raw).map_err(/* existing */)?;
        validate_page(&resp, peer_name, page_cursor)?;
        // `cycle.pages` is incremented by `fold`, below, so it is still the count of pages
        // ALREADY folded here — zero on the first pass. One counter, not two.
        if cycle.pages == 0 {
            // The FIRST page's declaration. A peer does not change identity mid-cycle, and
            // taking the first makes the reported value independent of where the loop stopped.
            first_signing_context = resp.signing_context.clone();
        }
        if let Some(reason) = resp.custody_withheld.as_deref() {
            eprintln!("{}", custody_withheld_message(peer_name, reason));
        }

        // The next page's FETCH point is the last seq RECEIVED — never `cycle.max_seq`, which
        // is the contiguous handled prefix and starts at the COMMITTED cursor. On a full
        // sweep (after_seq = 0) the two differ by the whole history below the cursor, and
        // using the checkpoint would make page 2 jump past everything a sweep exists to
        // reconcile. Captured before folding, because folding consumes the page.
        let next_cursor = resp.seqs.last().copied();

        cycle.fold(apply_page(client, &ctx, &resp, wire_bytes, cycle.max_seq));

        // Checkpoint EVERY page — cursor and floor together. This is #101 item 1's fix: an
        // interruption at page 39 of 40 leaves a durable, conservative state instead of
        // restarting from zero. The floor is computed over the CYCLE (see `quarantine_floor`).
        let new_floor = pull_page::quarantine_floor(
            cycle.skipped_unverifiable, cycle.refused_verifiable,
            cycle.pen_refused.is_some(), cycle.pin, floor_seq,
        );
        cursor_commit = commit_cursor(client, peer_name, cycle.max_seq, new_floor);

        match page_decision(resp.complete, resp.events.len(), cycle.frozen) {
            PageDecision::Done => break,
            PageDecision::Refuse(why) => {
                return Err(Box::new(PullIntegrityError {
                    message: format!("pull {peer_name}: {why}"),
                    metrics: serde_json::Value::Null,
                    also_local_fault: false,
                }))
            }
            // `Continue` implies a non-empty page (page_decision refuses an empty one), so
            // `next_cursor` is Some; the fallback ends the loop rather than unwrapping.
            PageDecision::Continue => match next_cursor {
                Some(n) => page_cursor = n,
                None => break,
            },
        }
    }
```

Move today's `let new_floor = …` / `UPDATE sync_state` block into a small
`fn commit_cursor(client, peer_name, max_seq, floor) -> Result<(), String>` — keeping the rowcount
check (#472) and `legible_db_error` exactly as they are — so the loop can call it per page. The metrics
object, the loud diagnosis and the unlearnable report stay **after** the loop, reading from `cycle` and
`first_signing_context`. Add `"pages": cycle.pages` to the metrics.

- [ ] **Step 4: Thread `--page` through**

- `cmd_pull` and `cmd_run` take `page_limit: u32` and pass it to `do_pull`.
- In `main`, both arms: `flag(&args, "--page").and_then(|s| s.parse().ok()).unwrap_or(DEFAULT_PAGE_EVENTS)`.
- Add `[--page N]` to both lines in `usage()`, with a parenthetical:
  `(--page: events per request; default 500. A larger page trades round trips for a bigger frame.)`
- **Reject `--page 0`**: `page_decision` would see a limit of 0, the serve would return no rows, and
  the empty-non-complete refusal would fire on every cycle. Clamp to at least 1 with a `max(1)` and a
  comment, or refuse in `main` with a message. Prefer refusing: a silently-corrected flag is a lie
  about what the operator asked for.

- [ ] **Step 5: Run everything, including the real network path**

```bash
CAIRN_ALLOW_DB_SKIP=1 cargo test -p cairn-wire
cargo test -p cairn-sync -- --test-threads=2
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --check
```

Expected: PASS, **including `tests/clinical_pull.rs`** — the DB-gated A→B test that drives the real
binary, serve on A and pull on B. It now exercises the paged path end to end.

- [ ] **Step 6: Commit**

```bash
git add crates/cairn-sync/src/main.rs
git commit -m "$(cat <<'EOF'
feat(#500): the pull pages, and checkpoints every page

#101 item 1: a catch-up larger than the 30 s read timeout used to retry
the SAME oversized response forever and never progress. Every page now
commits its cursor and floor together, so an interruption at page 39 of
40 resumes where it stopped. Paged by default, 500 events per page,
--page to override.

The page cursor and the checkpoint cursor are DIFFERENT values, and
conflating them is the subtle bug this slice could have shipped. The
next page is fetched from the last seq RECEIVED; max_seq is the
contiguous handled prefix and starts at the committed cursor. On a full
sweep (after_seq = 0) they differ by the whole history below the
cursor, so paging from the checkpoint would make page 2 jump past
exactly the events a sweep exists to reconcile. It has its own test.

The floor is computed over the cycle, so a clean later page cannot
clear a pin an earlier one set. A freeze breaks the loop. The
attachment-flag watermark is read once, before page 1. The signing
context reported is page 1's, so the value does not depend on where the
loop stopped. --page 0 is refused rather than silently corrected.

do_pull is now testable without a socket for the first time
(FakeTransport), which is what let the resume, freeze and full-sweep
cases be tested at all.

Refs #500, #101

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>
EOF
)"
```

---

## Task 8: Retire the stale deferrals, and the docs

**Files:**
- Modify: `crates/cairn-sync/src/main.rs`, `crates/cairn-medium/src/chunk.rs`,
  `crates/cairn-event/src/framing.rs`, `docs/HANDOVER.md`, `docs/ROADMAP.md`

- [ ] **Step 1: Find every stale claim**

```bash
grep -rn "unpaginated\|UNPAGINATED\|Unpaginated\|issue #101\|(#101)" crates/ | sort
```

Expected: the seven sites in the design's §3 table, plus whatever this slice's own commits added.

- [ ] **Step 2: Rewrite all seven**

Each must end up saying what is TRUE after this merges. **Leaving a stale deferral standing is the
precise mechanism by which #500 hid for weeks** — `localstate.rs` declared its seam truthfully, ADR-0052
made that false without reopening it, and ROADMAP kept recording the slice as done.

| site | becomes |
|---|---|
| `FULL_SWEEP_EVERY` doc | The sweep is PAGED: it re-ships the whole peer log, but in `--page` batches with a checkpoint after each, so its cost is round trips rather than one frame that must fit. Delete the "the correctness floor stops floor-ing on the largest-history nodes" warning — that is no longer true, and it is the sentence most likely to be quoted. |
| `MAX_FRAME_BYTES` doc | The cap is now a BACKSTOP for a request that omits `limit` (and for the legacy `EventsAfter` arm, which is unpaginated by construction). Name `DEFAULT_PAGE_EVENTS` and the ≈2 MiB page it implies. |
| `write_frame` refusal text | Replace `"(pagination: issue #101)"` with a remedy an operator can act on: pass `--page`, or the peer is serving an unpaginated response. |
| `serve_conn`'s `EventsAfterSeq` arm | The suffix ships in one frame only when the request carries no `limit`. |
| `frame_cap_holds_a_realistic_event_batch` (doc + assert message) | The cap bounds an UNLIMITED request; a paged one is ~2 MiB. Keep the assertion's bounds. |
| `cairn-medium/chunk.rs` `MAX_CHUNK_BYTES` doc | `MAX_FRAME_BYTES` caps a whole BATCH response, paged or not — drop "unpaginated". |
| `cairn-event/framing.rs` module doc | "the clinical plane at 64 MiB (a whole batch response; paged since slice 2b, unlimited when a request omits `limit`)". |

Add a note wherever #101 is cited that **item 1 is closed by slice 2b while items 2 (the blob
`byte_len` wedge) and 3 (in-DB BLAKE3) remain open**, so #101 itself stays open.

- [ ] **Step 3: Comment on #101**

```bash
gh issue comment 101 --body "$(cat <<'EOF'
**Item 1 (unbounded `EventsAfter` batch) is CLOSED** by DR slice 2b (#500).

`Request::EventsAfterSeq` gained `limit: Option<u32>` and `EventsResponse` gained
`complete: bool`, both additive with serde defaults (principle 12). `do_pull` pages at
`DEFAULT_PAGE_EVENTS = 500` by default (`--page N` to override) and **commits its cursor and
quarantine floor after every page**, which is the part that actually fixes the defect: a
catch-up larger than the 30 s read timeout used to retry the same oversized response forever
and never progress. An interruption at page 39 of 40 now resumes where it stopped.

The `FULL_SWEEP_EVERY` comment that said the correctness floor "stops floor-ing exactly on the
largest-history nodes" is retired with it — the sweep is paged now, and its cost is round trips
rather than one frame that must fit.

**Items 2 and 3 are untouched, so this issue stays open:**
- item 2 — first-writer-wins blob `byte_len` in `blob_note_reference` (db/003), the refetch-loop
  wedge;
- item 3 — BLAKE3 verification is L2-only; making it an in-DB floor needs BLAKE3 exposed in
  `cairn_pgx`.
EOF
)"
```

- [ ] **Step 4: Update HANDOVER.md**

- ⇒ NEXT: slice 2b **landed and closes nothing**; #500 still open; **next build is #511** (the custody
  newtypes, sequenced after 2b and before 2c), **then 2c**.
- A new dated "Recent sessions" entry, condensing older ones to keep the file **under 500 lines**.
- Carry forward the lessons that outlive the slice: *the page cursor is not the checkpoint cursor*;
  *a per-page floor silently un-pins an earlier page's refusal*; *`complete` defaults to the uncertain
  direction*; *seven comments asserted a deferral that one slice retired, and the first guess was one*.
- Add `crates/cairn-wire` to the crate list under "Read these first".
- Note **#531** among the open threads.

- [ ] **Step 5: Update ROADMAP.md**

Add the slice 2b entry after 2a's, in the same shape: what landed, what it does NOT do, the five
pieces with **2b struck through and #511 next**. Keep under 500 lines. ⚠️ **A line cap is never a reason
to drop a live issue** — a ROADMAP condensation once orphaned 22 in one edit.

- [ ] **Step 6: Run the full local gate**

```bash
export CAIRN_TEST_PG=… CAIRN_TEST_PG2=… CAIRN_TEST_PG3=…
scripts/run-db-gated-tests.sh
```

This is the ONE command that catches all three demonstrated hiding modes (fail-fast, a piped exit
status, a cross-crate suite `-p <crate>` never builds). It also runs the `db/tests/*.sql` mirrors.
Budget **hours**, not minutes: a `Cargo.lock` change relinks ~134 test binaries under macOS's
one-time-per-binary Gatekeeper assessment. Start it in the background and do the docs pass while it runs.

Also confirm the two plan/prose gates:

```bash
CAIRN_ALLOW_DB_SKIP=1 cargo test -p cairn-node --test paper_parity_plan_section
CAIRN_ALLOW_DB_SKIP=1 cargo test -p cairn-node --test crypto_sink_names_are_genuine
CAIRN_ALLOW_DB_SKIP=1 cargo test -p cairn-node --test unwrap_secret_is_not_derived
cargo doc --workspace --no-deps   # CI runs RUSTDOCFLAGS=-D warnings
```

- [ ] **Step 7: Commit and open the PR**

```bash
git add -A
git commit -m "$(cat <<'EOF'
docs(#500): retire the pagination deferrals slice 2b made false

grep found SEVEN comments across three crates asserting that pagination
does not exist — the first guess was one. The load-bearing one is
FULL_SWEEP_EVERY's, which said in plain words that the correctness
floor stops working on exactly the nodes with the most history. That
is no longer true, and it is the sentence most likely to be quoted.

Leaving a stale deferral standing is the precise mechanism by which
#500 hid for weeks: localstate.rs declared its seam truthfully,
ADR-0052 made that false without reopening it, and ROADMAP kept
recording the slice as done.

#101 item 1 is closed; items 2 and 3 are untouched, so #101 stays open,
and every citation now says which item went.

Refs #500, #101, #531

Co-Authored-By: Claude Opus 5 <noreply@anthropic.com>
EOF
)"
git push -u origin feat/500-dr-slice-2b-transport-seam-paged-pull
gh pr create --base main \
  --title "DR slice 2b (#500): the transport seam and the paged pull" \
  --body "$(cat <<'EOF'
**This slice closes nothing. [#500](https://github.com/cairn-ehr/cairn-ehr/issues/500) stays
open.** `backup.rs::read_event_set` still exports `SELECT signed_bytes FROM node_event` and
nothing else; the medium still carries no clinical event; and
`dr_clinical_guarantee_gap.rs::medium_carries_the_federation_plane_and_no_clinical_event` still
passes **as a pin on the defect** — slice 2d inverts it, not this one. `backup-status.json`,
`status` and `verify-backup` still report health for a medium that does not hold the record.

## What lands

**`crates/cairn-wire`** — the clinical-plane wire types, framing and the transport seam, moved
out of `cairn-sync`'s binary-only `main.rs` so `cairn-node` can reach them in 2c/2d. Same
reasoning and same pattern as `cairn-keystore` (#503) and `cairn-medium` (2a); the extraction's
proof is that every existing call site compiled with nothing but an import change.

**`Transport`**, with `TcpTransport` (today's behaviour, verbatim) and **`MediumTransport`** — a
CAIRNB3 medium answering as a peer. That is what makes ADR-0026 decision 2's *"backup is a
configuration of the sync daemon"* mechanical rather than prose: 2d's restore will drive
`cairn-sync`'s own puller, with its cursor, quarantine pen and custody handling, against a file.

**Paging.** `EventsAfterSeq` gains `limit`, `EventsResponse` gains `complete`, both additive.
`do_pull` loops and checkpoints every page — the fix for
[#101](https://github.com/cairn-ehr/cairn-ehr/issues/101) item 1 (items 2 and 3 stay open).

## What a reviewer should look at hardest

1. **The page cursor is not the checkpoint cursor.** The next page is fetched from the last seq
   *received*; `max_seq` is the contiguous handled prefix and starts at the *committed* cursor.
   On a full sweep they differ by the whole history below the cursor, so conflating them would
   make page 2 skip exactly what a sweep exists to reconcile.
   (`a_full_sweep_pages_from_the_last_seq_received_not_from_the_checkpoint`)
2. **The quarantine floor is computed over the cycle, never a page.** Per page, a clean page 2
   would clear the pin a refusing page 1 set — and the cursor has already passed that event.
   (`a_clean_later_page_cannot_clear_a_pin_an_earlier_page_set`)
3. **`complete` defaults to false**, the uncertain direction: an omitted flag costs a wasted
   round trip, never a silent early stop with the cursor checkpointed as if the log were drained.
4. **`TransportError::Exchange` keeps its `io::Error` reachable through `source()`**, or a peer
   sending an over-cap frame silently reclassifies from integrity to partition.

## Also

- **#531** filed: `cairn-sync/main.rs` is 11,577 lines. This slice removes ~250 and adds ~120;
  the decomposition is deliberately out of scope so the paging diff stays reviewable.
- Seven comments across three crates asserted that pagination did not exist. All seven are
  rewritten — including `FULL_SWEEP_EVERY`'s, which said the correctness floor stops working on
  the largest-history nodes. Leaving a stale deferral standing is how #500 hid for weeks.

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

---

## Self-review

**Spec coverage.** §2 the crate → Tasks 1–2. §3 the wire contract → Task 3. §3's stale-comment table →
Task 8. §4 `MediumTransport` → Task 4. §5 the paged pull → Tasks 5–7. §6 "what stays broken" → asserted
in every commit message and in the PR body. §7 scope → #531, filed. §8 testing → distributed across the
tasks that own each property.

**Type consistency.** `do_pull`'s signature changes **twice**, deliberately, and an executor working
Task 7 should expect to touch the same call sites again:

| after | signature |
|---|---|
| today | `(client, peer: &str, peer_name, full_sweep, custody)` |
| Task 2 | `(client, transport: &dyn Transport, peer_name, full_sweep, custody)` |
| Task 7 | `(client, transport: &dyn Transport, peer_name, full_sweep, page_limit: u32, custody)` |

Splitting it that way is the point: Task 2 is a mechanical seam change across 24 test call sites with
no behaviour change, and Task 7 is the behaviour change. Merging them would put both in one diff and
make neither reviewable.

**Two deliberate under-specifications**, neither a placeholder for a decision:

- `PageContext`'s exact fields are whatever the extracted loop body closes over; the compiler names
  them in Task 6, Step 3. No behaviour depends on which struct the borrow travels in.
- The spec's §5 lists a `page_request(after_seq, limit, unwrap_cert) -> Request` helper. The plan
  builds the `Request` inline instead — it is a struct literal with three fields, and a function
  wrapping it would add a name to look up without removing anything. YAGNI; drop it from the spec's
  function list if it comes up in review.
