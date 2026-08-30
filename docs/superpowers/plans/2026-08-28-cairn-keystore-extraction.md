# cairn-keystore Extraction Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `cairn-sync` **loads** the node's provisioned X25519 unwrap secret from the sealed `<key>.unwrap` file instead of HKDF-deriving it from its signing seed — restoring federated sync, which is inoperable on `main`.

**Architecture:** Extract `crates/cairn-keystore` (the `CAIRNK1` sealed-bundle format + the key-file loader + atomic file IO, moved verbatim from `cairn-node`), which both binaries depend on. `cairn-node` re-exports the three modules so its 221 call sites are untouched. `cairn-sync` gains a small `unwrap_key` module holding a **pure** decision table, resolves its custody key **once at startup**, and threads it into the two places that use it.

**Tech Stack:** Rust 1.96 (pinned), `argon2` + `chacha20poly1305` + `ciborium` (sealed bundle), `cairn-event` (X25519 wrap/unwrap), `postgres`, `zeroize`.

**Spec:** [`docs/superpowers/specs/2026-08-28-cairn-keystore-extraction-design.md`](../specs/2026-08-28-cairn-keystore-extraction-design.md)

Paper-parity: not clinical-surface — this plan moves an existing key-file format verbatim into a
shared crate and changes how `cairn-sync` resolves its custody key at startup; it touches no
clinical workflow, UI, or patient data model (house rule 7, CLAUDE.md).

## Global Constraints

- **Licence:** AGPL-3.0. Every dependency the new crate takes is **already a vetted `cairn-node` dependency** — no new licence surface enters the project. Do not add any dependency not listed in Task 1.
- **TDD (house rule 2):** failing test first, then the code. Key custody is the §9 safety-critical tier.
- **House rule 6 — never hard-code cryptographic material in tests.** Every seed/key/secret fixture is computed at runtime (`std::array::from_fn(|i| ...)`, `generate_key()`, `generate_unwrap_secret()`). A byte-array or string literal in a crypto context trips CodeQL `rust/hard-coded-cryptographic-value` and blocks the scan (#146).
- **Formatting:** rustfmt **defaults**, `max_width = 100`. CI has an `fmt --check` gate on both cargo trees. Run `cargo fmt --all` before every commit.
- **Lints:** `[lints] workspace = true` on every crate; CI runs `cargo clippy --workspace --tests -- -D warnings`.
- **Error recognition (repo convention):** classify by **type or `io::ErrorKind`, never by message text**, and a **type outranks a kind**. Never `format!`/`anyhow!("...: {e}")` a cause you want a classifier to see — that flattens it permanently.
- **Guard before connect:** in any DB-touching test, take `db::test_serial_guard(&base)` *before* `connect_and_load_schema`.
- **The gate is `scripts/run-db-gated-tests.sh`** — it bakes in `CAIRN_TEST_PG` / `PG2` / `PG3` and is the only gate catching this repo's three hiding modes (fail-fast, a piped exit status, a cross-crate suite `-p <crate>` never builds). **Never pipe `cargo test` to `tail`** — the pipeline's exit status is `tail`'s.
- **A DB-free `cargo test` FAILS** unless you `export CAIRN_ALLOW_DB_SKIP=1` (#450).
- **Warm target dir:** if a live IDE holds the shared `target/` lock, use `CARGO_TARGET_DIR=/tmp/cairn-503` rather than killing the IDE.

---

### Task 1: Extract `crates/cairn-keystore`

Move three files verbatim into a new workspace member and re-export them from `cairn-node`. **No logic changes.** The whole task's proof is that every existing test still passes.

**Files:**
- Create: `crates/cairn-keystore/Cargo.toml`
- Create: `crates/cairn-keystore/src/lib.rs`
- Move: `crates/cairn-node/src/seal.rs` → `crates/cairn-keystore/src/seal.rs`
- Move: `crates/cairn-node/src/keystore.rs` → `crates/cairn-keystore/src/keystore.rs`
- Move: `crates/cairn-node/src/fsio.rs` → `crates/cairn-keystore/src/fsio.rs`
- Modify: `crates/cairn-node/src/lib.rs` (drop three `pub mod`, add one `pub use`)
- Modify: `crates/cairn-node/Cargo.toml` (add the path dep)
- Modify: `Cargo.toml` (workspace members)
- Modify: `crates/cairn-node/tests/unwrap_secret_is_not_derived.rs` (one `ALLOWED` path)

**Interfaces:**
- Consumes: nothing (first task).
- Produces: `cairn_keystore::seal`, `cairn_keystore::keystore`, `cairn_keystore::fsio` — the exact public API that `cairn_node::seal` / `::keystore` / `::fsio` has today, unchanged. Later tasks use `cairn_keystore::keystore::{load_unwrap_secret, unwrap_key_path_for, KeystoreError}`.

- [ ] **Step 1: Move the three files with git, preserving history**

```bash
mkdir -p crates/cairn-keystore/src
git mv crates/cairn-node/src/seal.rs     crates/cairn-keystore/src/seal.rs
git mv crates/cairn-node/src/keystore.rs crates/cairn-keystore/src/keystore.rs
git mv crates/cairn-node/src/fsio.rs     crates/cairn-keystore/src/fsio.rs
```

`keystore.rs` refers to `crate::seal` and `crate::fsio`. Both move together into the same crate, so **those paths keep resolving with no edit.** Do not rewrite them.

- [ ] **Step 2: Write the crate manifest**

Create `crates/cairn-keystore/Cargo.toml`:

```toml
[package]
name = "cairn-keystore"
version = "0.1.0"
edition.workspace = true
rust-version.workspace = true
license.workspace = true
repository.workspace = true
# The at-rest key-file layer shared by cairn-node and cairn-sync (issue #503,
# ADR-0066). Not published: like the sibling crates it carries a version-less
# `cairn-event` path dependency, which cargo-deny's wildcard gate allows only
# for an unpublished crate.
publish = false

# Inherit the central workspace lint policy (#144).
[lints]
workspace = true

[dependencies]
cairn-event = { path = "../cairn-event" }
# Every dependency below is already a vetted cairn-node dependency — this crate
# is an extraction, so it adds no new licence surface (house rule 1). argon2,
# chacha20poly1305, getrandom and zeroize are dual MIT/Apache-2.0; ciborium is
# Apache-2.0-only. All AGPL-3.0-compatible.
argon2 = "0.5"
chacha20poly1305 = { version = "0.11", features = ["zeroize"] }
ciborium = "0.2"
getrandom = "0.4"
serde = { version = "1", features = ["derive"] }
thiserror = "1"
zeroize = "1"

[dev-dependencies]
# The moved unit tests write key files into an auto-cleaning temp dir.
tempfile = "3"
```

- [ ] **Step 3: Write the crate root**

Create `crates/cairn-keystore/src/lib.rs`:

```rust
//! The at-rest key-file layer: how a Cairn node's key material lives on disk.
//!
//! WHY THIS IS ITS OWN CRATE (issue #503). Two binaries must agree on the shape of a
//! node's key files. `cairn-node` provisions them; `cairn-sync` must LOAD the unwrap
//! secret those files hold, because since
//! [ADR-0066](../../../docs/spec/decisions/0066-identity-dies-with-the-disk-custody-must-not.md)
//! that secret is an INDEPENDENT key, no longer derivable from the node's signing seed.
//! Before this crate existed the format and its loader lived inside `cairn-node`, so
//! `cairn-sync` could not read them without depending on a whole node application — and
//! it therefore kept deriving a secret that no longer matched, which stopped federated
//! sync dead.
//!
//! The three modules are one layer, split by responsibility:
//!
//! - [`seal`] — the `CAIRNK1` sealed-bundle FORMAT: Argon2id key-encryption keys derived
//!   from two independent secrets (an operational passphrase and a paper recovery code),
//!   XChaCha20-Poly1305 over the 32-byte payload, CBOR on disk.
//! - [`keystore`] — the FILES that format is written to: the signing key and its
//!   `<key>.unwrap` custody sibling, with sealed-vs-plaintext auto-detection.
//! - [`fsio`] — the crash-safe write underneath both: temp sibling, fsync, rename, fsync
//!   the directory. A torn key file is unrecoverable key loss, so this is not optional.
//!
//! A defect anywhere here is silent key loss or a forged identity, so this is §9
//! safety-critical code: pure functions wherever entropy allows, exhaustively unit-tested,
//! and optimised for a reviewer's eye over cleverness.

pub mod fsio;
pub mod keystore;
pub mod seal;
```

- [ ] **Step 4: Register the member and the dependency**

In the root `Cargo.toml`, add to `members` (keep the list alphabetical-ish as it stands, with `cairn-keystore` before `cairn-sync`):

```toml
    "crates/cairn-keystore", # the shared at-rest key-file layer (#503)
```

In `crates/cairn-node/Cargo.toml`, add under `[dependencies]` beside the other path deps:

```toml
cairn-keystore = { path = "../cairn-keystore" }
```

Then **delete** from `crates/cairn-node/Cargo.toml` nothing at all — `argon2`, `ciborium`, `getrandom` etc. stay: `cairn-node` still uses `rpassword` for the prompt and the others may be reached through re-exported types. Removing them is a separate tidy-up and is **out of scope**; an unused-dependency warning is not part of the CI gate.

- [ ] **Step 5: Re-export from cairn-node**

In `crates/cairn-node/src/lib.rs`, **delete** these three lines:

```rust
pub mod fsio;
pub mod keystore;
pub mod seal;
```

and add, in their place (keeping the remaining `pub mod` list alphabetical):

```rust
// The at-rest key-file layer now lives in its own crate so `cairn-sync` can load the
// same sealed files this node writes (issue #503) without depending on a node
// application. Re-exported rather than renamed at the call sites: `crate::keystore::…`
// and `cairn_node::keystore::…` appear 221 times across ~30 files here, and churning
// them would bury the behavioural change in rename noise. These are not deprecated
// shims — `cairn-node` genuinely still offers these modules, implemented elsewhere.
pub use cairn_keystore::{fsio, keystore, seal};
```

- [ ] **Step 6: Update the guard's `ALLOWED` path**

In `crates/cairn-node/tests/unwrap_secret_is_not_derived.rs`, the first `ALLOWED` entry's path moves with the file:

```rust
    (
        "crates/cairn-keystore/src/keystore.rs",
        "the ADR-0066 adoption migration (`adopt_derived_unwrap_secret`) — the one place a \
         pre-ADR-0066 node re-derives its old secret to keep its existing event_dek rows openable",
    ),
```

Leave the `crates/cairn-sync/src/main.rs` entry **exactly as it is** for now; Task 6 rewrites its reason text once the derives are actually gone.

The guard asserts every entry is **live** (a dead entry fails), so if this path is wrong the guard says so. **When it fails, fix the path — never add an entry.**

- [ ] **Step 7: Build and run the full workspace test suite**

```bash
cargo fmt --all
cargo clippy --workspace --tests -- -D warnings
scripts/run-db-gated-tests.sh
```

Expected: **PASS, with zero test-file edits.** That is the whole proof of this task — 221 call sites and ~30 test files compile untouched, and every moved unit test now runs under `-p cairn-keystore`.

If `cargo doc` is run separately, note CI uses `RUSTDOCFLAGS=-D warnings`; intra-doc links of the form `[`cairn_node::keystore::key_at_rest_state`]` (e.g. `crates/cairn-node/src/main.rs:450`) still resolve through a `pub use`, but verify:

```bash
RUSTDOCFLAGS=-D warnings cargo doc --workspace --no-deps
```

- [ ] **Step 8: Commit**

```bash
git add -A
git commit -m "refactor(#503): extract crates/cairn-keystore

The CAIRNK1 sealed-bundle format, the key-file loader and the crash-safe
atomic write move verbatim out of cairn-node into a crate cairn-sync can
also depend on. No logic changes: the proof is that 221 call sites across
~30 files compile untouched behind a three-module re-export.

Refs #503"
```

---

### Task 2: The pure resolution decision table

The safety argument of this whole slice lives here, and it is provable with **no database and no filesystem**.

**Files:**
- Create: `crates/cairn-sync/src/unwrap_key.rs`
- Modify: `crates/cairn-sync/src/main.rs` (add `mod unwrap_key;` near the top, after the `use` block)
- Modify: `crates/cairn-sync/Cargo.toml` (add `cairn-keystore` and `zeroize`)

**Interfaces:**
- Consumes: `cairn_keystore::keystore::{KeystoreError, load_unwrap_secret, unwrap_key_path_for}` (Task 1).
- Produces:
  - `pub enum FileOutcome { Loaded(Zeroizing<[u8; 32]>), Absent, Unusable(String) }`
  - `pub enum Resolution { Use { secret: Zeroizing<[u8; 32]>, warning: Option<String> }, Refuse(String) }`
  - `pub fn resolve(file: FileOutcome, derived: Zeroizing<[u8; 32]>, registered: Option<&[u8; 32]>, path_display: &str) -> Resolution`

- [ ] **Step 1: Add the dependencies**

In `crates/cairn-sync/Cargo.toml`, under `[dependencies]`:

```toml
# The node's at-rest key files: cairn-sync LOADS the unwrap secret cairn-node
# provisioned, rather than deriving one that cannot match (issue #503, ADR-0066).
cairn-keystore = { path = "../cairn-keystore" }
zeroize = "1"              # hold the loaded unwrap secret in Zeroizing (issue #54)
```

- [ ] **Step 2: Declare the module**

In `crates/cairn-sync/src/main.rs`, immediately after the existing top-level `use` statements, add:

```rust
// The node's custody-key resolution (issue #503). A module rather than another
// function in this file: main.rs is ~11,700 lines, and the decision table below is
// the safety argument of the ADR-0066 conversion — it belongs somewhere a reviewer
// can hold in one screen, and it is testable with no database and no filesystem.
mod unwrap_key;
```

- [ ] **Step 3: Write the failing tests**

Create `crates/cairn-sync/src/unwrap_key.rs` containing ONLY the test module for now (the types do not exist yet, so this must fail to compile — that is the failing state):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use cairn_event::seal::unwrap_public;
    use zeroize::Zeroizing;

    /// House rule 6: key material is DERIVED at runtime, never a literal, so CodeQL's
    /// hard-coded-cryptographic-value query stays live for production code.
    fn secret_fixture(tag: u8) -> Zeroizing<[u8; 32]> {
        Zeroizing::new(std::array::from_fn(|i| tag ^ (i as u8).wrapping_mul(7)))
    }

    #[test]
    fn a_loaded_file_matching_the_registration_is_used() {
        let provisioned = secret_fixture(0x11);
        let registered = unwrap_public(&provisioned);
        let r = resolve(
            FileOutcome::Loaded(provisioned.clone()),
            secret_fixture(0x22),
            Some(&registered),
            "node.key.unwrap",
        );
        match r {
            Resolution::Use { secret, warning } => {
                assert_eq!(*secret, *provisioned, "the FILE's secret is the one used");
                assert!(warning.is_none(), "a loaded key is the normal case: no warning");
            }
            Resolution::Refuse(m) => panic!("the happy path must not refuse: {m}"),
        }
    }

    #[test]
    fn a_loaded_file_diverging_from_the_registration_refuses() {
        let registered = unwrap_public(&secret_fixture(0x33));
        let r = resolve(
            FileOutcome::Loaded(secret_fixture(0x44)),
            secret_fixture(0x55),
            Some(&registered),
            "node.key.unwrap",
        );
        let Resolution::Refuse(m) = r else {
            panic!("a divergent key must refuse");
        };
        assert!(m.contains("node.key.unwrap"), "the refusal names the file: {m}");
    }

    #[test]
    fn a_loaded_file_with_nothing_registered_is_used() {
        // No row in node_unwrap_key: nothing has been claimed, and the provisioned
        // file is the authority on what this node's custody key is.
        let provisioned = secret_fixture(0x66);
        let r = resolve(
            FileOutcome::Loaded(provisioned.clone()),
            secret_fixture(0x77),
            None,
            "node.key.unwrap",
        );
        let Resolution::Use { secret, warning } = r else {
            panic!("nothing registered is not a refusal");
        };
        assert_eq!(*secret, *provisioned);
        assert!(warning.is_none());
    }

    #[test]
    fn an_absent_file_whose_derived_key_matches_falls_back_loudly() {
        // The pre-ADR-0066 node. Its registered key IS the derived one, so the derived
        // secret is provably the key its event_dek rows are wrapped to — using it is
        // correct, not a reintroduction of the #495 coupling.
        let derived = secret_fixture(0x88);
        let registered = unwrap_public(&derived);
        let r = resolve(FileOutcome::Absent, derived.clone(), Some(&registered), "node.key.unwrap");
        let Resolution::Use { secret, warning } = r else {
            panic!("a provable pre-ADR-0066 node must start");
        };
        assert_eq!(*secret, *derived);
        let w = warning.expect("the fallback is LOUD — a silent one hides a deleted key file");
        assert!(w.contains("node.key.unwrap"), "the warning names the missing file: {w}");
        assert!(
            w.contains("establish-unwrap-key"),
            "the warning names the remedy: {w}"
        );
    }

    #[test]
    fn an_absent_file_whose_derived_key_diverges_refuses() {
        // The RESTORED node: fresh signing seed, so the derived key cannot be the one
        // its inherited event_dek rows are wrapped to. This is the #495 shape, caught.
        let registered = unwrap_public(&secret_fixture(0x99));
        let r = resolve(
            FileOutcome::Absent,
            secret_fixture(0xAA),
            Some(&registered),
            "node.key.unwrap",
        );
        assert!(
            matches!(r, Resolution::Refuse(_)),
            "a restored node must never proceed on a derived key"
        );
    }

    #[test]
    fn an_absent_file_with_nothing_registered_uses_the_derived_key_silently() {
        // Today's behaviour, preserved exactly: nothing has claimed custody, so there
        // is nothing for this daemon to be wrong about, and no warning to give.
        let derived = secret_fixture(0xBB);
        let r = resolve(FileOutcome::Absent, derived.clone(), None, "node.key.unwrap");
        let Resolution::Use { secret, warning } = r else {
            panic!("an unprovisioned node must still start");
        };
        assert_eq!(*secret, *derived);
        assert!(warning.is_none(), "no claim, no warning");
    }

    #[test]
    fn an_unusable_file_refuses_and_never_falls_back() {
        // #502 item 3, sharpened: on an adopted node a successful derive would MASK a
        // corrupt .unwrap file, and that file is the only vehicle carrying custody off
        // the machine. Present-but-unusable is not "absent".
        let derived = secret_fixture(0xCC);
        let registered = unwrap_public(&derived); // the derive WOULD have matched
        let r = resolve(
            FileOutcome::Unusable("wrong passphrase or corrupt file".into()),
            derived,
            Some(&registered),
            "node.key.unwrap",
        );
        let Resolution::Refuse(m) = r else {
            panic!("a corrupt key file must refuse even when the derive would match");
        };
        assert!(m.contains("wrong passphrase"), "the refusal carries the cause: {m}");
    }

    #[test]
    fn an_unusable_file_refuses_with_nothing_registered_too() {
        let r = resolve(
            FileOutcome::Unusable("not a sealed bundle".into()),
            secret_fixture(0xDD),
            None,
            "node.key.unwrap",
        );
        assert!(
            matches!(r, Resolution::Refuse(_)),
            "a garbage key file is a defect whether or not custody is claimed"
        );
    }
}
```

- [ ] **Step 4: Run the tests to verify they fail**

```bash
CAIRN_ALLOW_DB_SKIP=1 cargo test -p cairn-sync --bin cairn-sync unwrap_key
```

Expected: **compile error** — `resolve`, `FileOutcome`, `Resolution` are not defined.

- [ ] **Step 5: Write the implementation**

Prepend to `crates/cairn-sync/src/unwrap_key.rs`, above the test module:

```rust
//! Resolving this node's custody (unwrap) key at daemon startup — issue #503.
//!
//! WHY THIS EXISTS. Every clinical body is born sealed (ADR-0052): its payload is
//! encrypted under a per-event DEK, and that DEK is wrapped to the node's X25519 *unwrap*
//! public key. Whoever holds the matching secret can open the record; whoever does not,
//! cannot. Until ADR-0066 that secret was HKDF-derived from the node's Ed25519 signing
//! seed, so any process holding the signing key could recompute it — which is exactly why
//! a disaster-recovery-restored node (fresh signing seed, by design) could never open its
//! own inherited records (#495).
//!
//! ADR-0066 made the unwrap key INDEPENDENT: `cairn-node` mints it, seals it into a
//! `<key>.unwrap` file, and registers its public half in `node_unwrap_key`. This module is
//! how `cairn-sync` gets at it — and, when it cannot, how it decides whether starting
//! anyway is safe.
//!
//! THE SHAPE OF THE DECISION. Three inputs meet here:
//!
//! 1. what the on-disk file yielded ([`FileOutcome`]),
//! 2. the key this daemon WOULD derive the old way, and
//! 3. the key the database has REGISTERED — the only authority on which key this node's
//!    existing DEKs are actually wrapped to.
//!
//! Input 3 is what makes a fallback admissible at all: a derived key that *equals the
//! registered key* is provably the right key, whatever its provenance. A derived key that
//! differs is provably the wrong one. So the daemon never has to guess.
//!
//! [`resolve`] is PURE — no file system, no database — so the whole table is proved by
//! unit tests. The IO that feeds it lives in [`load_file_outcome`].

use zeroize::Zeroizing;

/// What reading the `<key>.unwrap` file yielded. Three outcomes, not two: "there is no
/// file" and "there is a file I cannot use" lead to OPPOSITE decisions below, and
/// collapsing them is the defect #502 item 3 recorded.
pub enum FileOutcome {
    /// The file was read and unsealed; this is the secret it holds.
    Loaded(Zeroizing<[u8; 32]>),
    /// No file at the path.
    Absent,
    /// A file is present but could not be read, unsealed, or parsed. The `String` is the
    /// operator-facing cause, already legible.
    Unusable(String),
}

/// What the daemon should do about its custody key.
pub enum Resolution {
    /// Start, using `secret`. A `warning` is present only on the pre-ADR-0066 fallback,
    /// and must be printed on EVERY startup — see [`resolve`].
    Use {
        secret: Zeroizing<[u8; 32]>,
        warning: Option<String>,
    },
    /// Refuse to start, with this operator-facing message.
    Refuse(String),
}

/// Decide which unwrap secret this daemon should run with — or that it should not run.
///
/// `registered` is the public half stored in `node_unwrap_key`, or `None` when no row
/// exists (nothing has claimed custody on this node yet). `path_display` is the file path
/// to name in messages.
///
/// The table, and why each row is what it is:
///
/// | file | registered | outcome |
/// |---|---|---|
/// | `Loaded` | matches | use it — the normal post-ADR-0066 case |
/// | `Loaded` | diverges | **refuse** — wrong key file, or a node rebuilt under another |
/// | `Loaded` | none | use it — the provisioned file is the authority |
/// | `Absent` | matches derived | use derived, **warn loudly** — a pre-ADR-0066 node |
/// | `Absent` | diverges | **refuse** — a restored node; the #495 shape |
/// | `Absent` | none | use derived, silently — nothing is claimed; today's behaviour |
/// | `Unusable` | anything | **refuse** — never mask a corrupt custody file |
///
/// The `Unusable` row is the one worth pausing on. On an adopted node the derived key
/// would MATCH the registration, so falling back would work perfectly and hide the fact
/// that the `.unwrap` file has rotted — and that file is the only vehicle carrying this
/// node's custody off the machine (the sealed local-state export reads it). The node would
/// sync happily for months and discover the loss at restore, which is precisely the
/// "every surface honest, the composite a precise untruth" failure ADR-0066 exists to
/// correct. So: absent may fall back; unusable never may.
pub fn resolve(
    file: FileOutcome,
    derived: Zeroizing<[u8; 32]>,
    registered: Option<&[u8; 32]>,
    path_display: &str,
) -> Resolution {
    match file {
        FileOutcome::Unusable(cause) => Resolution::Refuse(format!(
            "the unwrap-key file {path_display} is present but unusable: {cause}. Refusing to \
             start. This daemon will NOT fall back to deriving a key, even if the derived one \
             would match what the database registered — that would mask the loss of the only \
             file carrying this node's custody off the machine (ADR-0066, issue #503). If the \
             file is sealed, set CAIRN_KEY_PASSPHRASE; if it is damaged, restore it from the \
             node's sealed local-state export before starting."
        )),
        FileOutcome::Loaded(secret) => match registered {
            Some(reg) if cairn_event::seal::unwrap_public(&secret) != *reg => {
                Resolution::Refuse(format!(
                    "the unwrap key in {path_display} is not the one this database registered \
                     (file {}, database {}). This daemon cannot open this node's custody. Point \
                     --key/--unwrap-key at the files this node was provisioned with (ADR-0066, \
                     issue #503).",
                    hex::encode(&cairn_event::seal::unwrap_public(&secret)[..8]),
                    hex::encode(&reg[..8]),
                ))
            }
            _ => Resolution::Use {
                secret,
                warning: None,
            },
        },
        FileOutcome::Absent => match registered {
            // Nothing claimed: today's behaviour, unchanged, and nothing to warn about.
            None => Resolution::Use {
                secret: derived,
                warning: None,
            },
            Some(reg) if cairn_event::seal::unwrap_public(&derived) == *reg => Resolution::Use {
                secret: derived,
                warning: Some(format!(
                    "WARNING: no unwrap-key file at {path_display}; falling back to the \
                     pre-ADR-0066 derived key. It matches what this database registered, so it \
                     IS the key this node's sealed records are wrapped to and custody works — \
                     but this node has no independent custody key on disk, so its sealed \
                     local-state export cannot carry one, and a disaster-recovery restore would \
                     lose custody of every sealed record. Run `cairn-node \
                     establish-unwrap-key` on this node to adopt the derived secret into a real \
                     key file. Do NOT run it on a node that has just been restored and whose \
                     export could not be read — see issue #503."
                )),
            },
            Some(reg) => Resolution::Refuse(format!(
                "no unwrap-key file at {path_display}, and the key this daemon would derive \
                 from its signing seed ({}) is not the one this database registered ({}). That \
                 is the signature of a node restored under a fresh signing seed: the derived \
                 key cannot open its inherited records, and using it would silently wrap new \
                 DEKs to a key nothing can read. Recover the node's `.unwrap` file from its \
                 sealed local-state export (ADR-0066, issue #503).",
                hex::encode(&cairn_event::seal::unwrap_public(&derived)[..8]),
                hex::encode(&reg[..8]),
            )),
        },
    }
}
```

- [ ] **Step 6: Run the tests to verify they pass**

```bash
cargo fmt --all
CAIRN_ALLOW_DB_SKIP=1 cargo test -p cairn-sync --bin cairn-sync unwrap_key
```

Expected: **8 passed.**

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "test(#503): the custody-key decision table, proved without a database

Eight rows, one per situation a daemon can start in. The load-bearing
pair: an ABSENT key file may fall back to the pre-ADR-0066 derivation
when the derived key provably equals what the database registered, but a
present-but-UNUSABLE one never may — a successful derive would mask the
rot of the only file carrying this node's custody off the machine.

Refs #503"
```

---

### Task 3: The loader seam — `KeystoreError` to `FileOutcome`

The pure table is only as good as the classification feeding it. This task is where "absent" and "unusable" are actually told apart, against real files.

**Files:**
- Modify: `crates/cairn-sync/src/unwrap_key.rs`
- Modify: `crates/cairn-sync/Cargo.toml` (dev-dependency `tempfile`)

**Interfaces:**
- Consumes: `FileOutcome` (Task 2); `cairn_keystore::keystore::{KeystoreError, load_unwrap_secret}` (Task 1).
- Produces: `pub fn load_file_outcome(path: &std::path::Path, passphrase: Option<&str>) -> FileOutcome`

- [ ] **Step 1: Add the dev-dependency**

In `crates/cairn-sync/Cargo.toml` under `[dev-dependencies]`, `tempfile = "3"` is already present — confirm it is, and add it only if missing.

- [ ] **Step 2: Write the failing tests**

Append these tests inside the existing `mod tests` block in `crates/cairn-sync/src/unwrap_key.rs`:

```rust
    #[test]
    fn a_missing_file_classifies_as_absent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("node.key.unwrap");
        assert!(
            matches!(load_file_outcome(&path, None), FileOutcome::Absent),
            "a file that is not there is Absent, never Unusable"
        );
    }

    #[test]
    fn a_plaintext_secret_file_loads() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("node.key.unwrap");
        let secret = cairn_event::seal::generate_unwrap_secret().unwrap();
        cairn_keystore::keystore::write_unwrap_plaintext(&path, &secret).unwrap();
        let FileOutcome::Loaded(loaded) = load_file_outcome(&path, None) else {
            panic!("a plaintext unwrap key must load");
        };
        assert_eq!(*loaded, *secret, "the bytes cross the disk intact");
    }

    #[test]
    fn a_sealed_file_loads_with_its_passphrase() {
        // The cross-crate link that nothing tests today: cairn-node SEALS the file and
        // cairn-sync must OPEN it. DR slice 1's lesson 4 — where no test carries the
        // value across the disk, the one link that matters is proven by nothing.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("node.key.unwrap");
        let pass = "op-passphrase-for-this-test";
        let code = cairn_keystore::seal::generate_recovery_code();
        let public = cairn_keystore::keystore::generate_unwrap_sealed(&path, pass, &code).unwrap();
        let FileOutcome::Loaded(loaded) = load_file_outcome(&path, Some(pass)) else {
            panic!("a sealed unwrap key must load under its passphrase");
        };
        assert_eq!(
            cairn_event::seal::unwrap_public(&loaded),
            public,
            "the secret loaded is the one whose public half was registered"
        );
    }

    #[test]
    fn a_sealed_file_without_a_passphrase_is_unusable_not_absent() {
        // The case that MUST NOT fall back: the key is right there, we simply have no
        // secret for it. Treating this as "absent" would derive a key and carry on.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("node.key.unwrap");
        let code = cairn_keystore::seal::generate_recovery_code();
        cairn_keystore::keystore::generate_unwrap_sealed(&path, "op-pass", &code).unwrap();
        let FileOutcome::Unusable(cause) = load_file_outcome(&path, None) else {
            panic!("a sealed file with no passphrase is Unusable, never Absent");
        };
        assert!(
            cause.contains("CAIRN_KEY_PASSPHRASE"),
            "the cause names the remedy: {cause}"
        );
    }

    #[test]
    fn a_corrupt_file_is_unusable() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("node.key.unwrap");
        std::fs::write(&path, b"not a sealed bundle and not 32 bytes").unwrap();
        assert!(
            matches!(load_file_outcome(&path, None), FileOutcome::Unusable(_)),
            "garbage is Unusable"
        );
    }

    #[test]
    fn a_wrong_passphrase_is_unusable() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("node.key.unwrap");
        let code = cairn_keystore::seal::generate_recovery_code();
        cairn_keystore::keystore::generate_unwrap_sealed(&path, "right-pass", &code).unwrap();
        assert!(
            matches!(
                load_file_outcome(&path, Some("wrong-pass")),
                FileOutcome::Unusable(_)
            ),
            "a wrong passphrase must never degrade into Absent"
        );
    }
```

- [ ] **Step 3: Run to verify failure**

```bash
CAIRN_ALLOW_DB_SKIP=1 cargo test -p cairn-sync --bin cairn-sync unwrap_key
```

Expected: **compile error** — `load_file_outcome` is not defined.

- [ ] **Step 4: Implement the classifier**

Append to the non-test part of `crates/cairn-sync/src/unwrap_key.rs`:

```rust
/// Read the node's `<key>.unwrap` file and classify the result for [`resolve`].
///
/// THE CLASSIFICATION IS THE POINT. `Absent` may fall back to a derived key; `Unusable`
/// may not. Getting that boundary wrong in the permissive direction re-creates the exact
/// defect this module exists to prevent, so the recogniser is the error's TYPE and, for
/// IO, its [`std::io::ErrorKind`] — never its message text (a repo-wide convention: a
/// formatted message flattens the cause permanently and can never be taught to a
/// classifier).
///
/// Note which way each variant falls:
///
/// - `Io(NotFound)` — genuinely no file. **Absent.**
/// - `Io(_)` — a permissions error, an IO error, a directory where a file should be. The
///   file may well exist and be perfectly good; we simply could not read it. **Unusable.**
/// - `Sealed` — the file is there and valid, we just hold no secret for it. **Unusable**,
///   with the remedy named: this is the shape an operator hits by forgetting
///   `CAIRN_KEY_PASSPHRASE`, and silently deriving a key instead would be the worst
///   possible response to it.
/// - `Key(_)` — corrupt bytes, a wrong passphrase, a bundle that is neither sealed nor 32
///   bytes. **Unusable.**
pub fn load_file_outcome(path: &std::path::Path, passphrase: Option<&str>) -> FileOutcome {
    use cairn_keystore::keystore::KeystoreError;
    match cairn_keystore::keystore::load_unwrap_secret(path, passphrase) {
        Ok(secret) => FileOutcome::Loaded(secret),
        Err(KeystoreError::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => {
            FileOutcome::Absent
        }
        Err(KeystoreError::Io(e)) => FileOutcome::Unusable(format!(
            "cannot read it ({e}) — the file may exist and be intact; this daemon could not \
             open it"
        )),
        Err(KeystoreError::Sealed) => FileOutcome::Unusable(
            "it is sealed and no passphrase was supplied — set CAIRN_KEY_PASSPHRASE to the \
             node's operational passphrase (or its recovery code)"
                .into(),
        ),
        Err(KeystoreError::Key(m)) => FileOutcome::Unusable(m),
    }
}
```

- [ ] **Step 5: Run to verify pass**

```bash
cargo fmt --all
CAIRN_ALLOW_DB_SKIP=1 cargo test -p cairn-sync --bin cairn-sync unwrap_key
```

Expected: **14 passed** (8 from Task 2 + 6 here).

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "test(#503): tell an absent custody file from an unusable one

Classified by error TYPE and io::ErrorKind, never message text. The row
that matters: a SEALED file with no CAIRN_KEY_PASSPHRASE is Unusable, not
Absent — the key is right there and we merely hold no secret for it, and
deriving one instead would be the worst available response.

Also the first test carrying a sealed unwrap key across the disk between
the two crates, which nothing covered before.

Refs #503"
```

---

### Task 4: Carry the custody key instead of re-deriving it

Pure plumbing, **no behaviour change**: introduce a type that holds the signing key and the unwrap secret together, and thread it through `do_pull` and the serve path. The three startup arms still derive — Task 5 changes that. Splitting it this way means the signature churn is reviewed separately from the safety change.

**Files:**
- Modify: `crates/cairn-sync/src/unwrap_key.rs` (add `NodeCustody`)
- Modify: `crates/cairn-sync/src/main.rs` — `do_pull` (3100, 3159), `cmd_pull` (4321), `cmd_serve` (4561, 4576), `cmd_run` (4653, 4717), `serve_conn` (5172-5184), the serve CLI arm (5569)

**Interfaces:**
- Consumes: Tasks 2-3.
- Produces: `pub struct NodeCustody { pub signing_key: SigningKey, pub unwrap_secret: Zeroizing<[u8; 32]> }`; `do_pull(..., custody: Option<&NodeCustody>)`; `cmd_serve(conn, listen, corrupt, custody: Option<Arc<NodeCustody>>)`; `serve_conn(conn, stream, corrupt, custody: Option<Arc<NodeCustody>>)`.

- [ ] **Step 1: Add the type**

Append to the non-test part of `crates/cairn-sync/src/unwrap_key.rs`:

```rust
/// This node's custody identity for the life of one process: the Ed25519 signing key that
/// proves who it is, and the X25519 unwrap secret that opens what it holds.
///
/// WHY THEY TRAVEL TOGETHER. Presenting custody on the wire needs both at once — the
/// unwrap CERT is the node's unwrap public key SIGNED by its signing key, so a peer can
/// trust that re-wrapping a DEK for that public key gives it to this node and nobody else.
/// Passing them as two separate `Option`s let them disagree: before ADR-0066 the secret
/// was a pure function of the key so they could not, but that is exactly the coupling
/// ADR-0066 removed. One struct makes the mismatch unrepresentable.
pub struct NodeCustody {
    pub signing_key: cairn_event::SigningKey,
    pub unwrap_secret: Zeroizing<[u8; 32]>,
}
```

- [ ] **Step 2: Change `do_pull`'s signature**

At `crates/cairn-sync/src/main.rs:3100`, replace the `key` parameter and its comment:

```rust
    // This node's custody for the wire (ADR-0052): the signing key that proves who we
    // are and the unwrap secret that opens DEKs a peer re-wraps back for us. Since
    // ADR-0066 the secret is NOT derivable from the key, so it is resolved once at
    // startup (see `unwrap_key::resolve`) and carried here. `None` (older call paths, DB
    // tests) pulls WITHOUT custody: events still sync and sealed rows admit structurally
    // at the door — custody is simply not gained on this cycle.
    custody: Option<&unwrap_key::NodeCustody>,
```

At line ~3159, replace the derive with the carried secret:

```rust
    // This node's unwrap identity for the pull (ADR-0052 custody sidecar). We keep our
    // unwrap SECRET (to open re-wrapped DEKs the peer sends back) and present our unwrap
    // CERT (so the peer can re-wrap for us). The cert binds our X25519 unwrap public key
    // to our Ed25519 identity — the same key the DEKs come back wrapped for.
    let unwrap_secret = custody.map(|c| c.unwrap_secret.clone());
    let unwrap_cert: Option<String> = match custody {
        Some(c) => {
            let public = cairn_event::seal::unwrap_public(&c.unwrap_secret);
            Some(hex::encode(cairn_event::sign_unwrap_key_cert(
                &c.signing_key,
                &public,
            )?))
        }
        None => None,
    };
```

**That is the whole change to `do_pull`.** The `key` parameter is used at exactly three lines — its own declaration and the two above — and nowhere else in the function's 728-line body (verified 2026-08-28; the other `key` matches in range are the words "attester-key" and "the key the ledger is read back on" in comments). `unwrap_secret` keeps its exact type, `Option<Zeroizing<[u8; 32]>>`, so its downstream consumer at ~line 3400 (`&unwrap_secret`) needs no edit.

- [ ] **Step 3: Change the serve path's signature**

At `crates/cairn-sync/src/main.rs:5172`, `serve_conn`:

```rust
fn serve_conn(
    conn: &str,
    mut stream: TcpStream,
    corrupt: bool,
    custody: Option<Arc<unwrap_key::NodeCustody>>,
) -> R<()> {
    let mut client = connect_db(conn)?;
    // Our own unwrap secret, resolved once at startup and shared across connection
    // threads (ADR-0026 escrow: the secret is never in the DB). Used only to open our
    // locally-stored DEKs so the EventsAfterSeq arm can re-wrap them for the peer.
    let own_secret = custody.as_ref().map(|c| c.unwrap_secret.clone());
```

Then `cmd_serve` at 4561:

```rust
fn cmd_serve(
    conn: String,
    listen: &str,
    corrupt: bool,
    custody: Option<Arc<unwrap_key::NodeCustody>>,
) -> R<()> {
```

and its call at ~4576 passes `custody.clone()` into each accepted connection. If `own_key` is referenced elsewhere in `cmd_serve`'s body (e.g. to log a kid), reach it as `custody.as_ref().map(|c| &c.signing_key)`.

- [ ] **Step 4: Update the three call sites to build a `NodeCustody`**

`cmd_pull` (~4310), `cmd_run` (~4640) and the serve CLI arm (~5558) each currently do `load_or_create_key`, then derive, then fence. For **this** task keep the derive and simply package it:

```rust
    let (sk, _kid) = load_or_create_key(key_path)?;
    let unwrap_secret = cairn_event::seal::derive_unwrap_secret(&sk.to_bytes());
    let mine = cairn_event::seal::unwrap_public(&unwrap_secret);
    assert_unwrap_key_registered(&mut client, &mine)?;
    let custody = unwrap_key::NodeCustody {
        signing_key: sk,
        unwrap_secret,
    };
```

then pass `Some(&custody)` to `do_pull`, or `Some(Arc::new(custody))` to `cmd_serve`. `cmd_run` shares one `Arc<NodeCustody>` between its pull loop and its serve thread, exactly as it shares one `Arc<SigningKey>` today.

- [ ] **Step 5: Leave the ~24 test call sites alone**

Every `do_pull(..., None)` in the test modules keeps compiling — the parameter is still an `Option`, only its type changed. **Do not touch them.** If one fails to compile, the signature is wrong; fix the signature.

- [ ] **Step 6: Run the full gate**

```bash
cargo fmt --all
cargo clippy --workspace --tests -- -D warnings
scripts/run-db-gated-tests.sh
```

Expected: **PASS.** Behaviour is unchanged by construction — the same derived secret reaches the same two places, now carried rather than recomputed. `cairn-sync/tests/clinical_pull.rs` is the suite that proves it end to end, and `-p cairn-sync` alone does build it, but run the whole gate: it is the only one that catches a cross-crate break.

- [ ] **Step 7: Commit**

```bash
git add -A
git commit -m "refactor(#503): carry the custody key, stop re-deriving it per use

do_pull and the serve path each derived this node's unwrap secret
independently of the startup fence that had just checked it. They agreed
only because the derivation is a pure function of the signing seed — the
very coupling ADR-0066 removed. One NodeCustody value now carries the
signing key and the unwrap secret together, making a mismatch between
them unrepresentable.

No behaviour change: same secret, same two consumers, resolved once.

Refs #503"
```

---

### Task 5: Load the provisioned key — the behaviour change

**Files:**
- Modify: `crates/cairn-sync/src/main.rs` — the three startup arms; the usage text (~5415-5436)
- Modify: `crates/cairn-sync/src/unwrap_key.rs` (the startup entry point)

**Interfaces:**
- Consumes: `resolve`, `load_file_outcome`, `NodeCustody` (Tasks 2-4).
- Produces: `pub fn resolve_at_startup(client: &mut postgres::Client, key_path: &str, unwrap_path_override: Option<&str>, signing_key: SigningKey) -> Result<NodeCustody, Box<dyn std::error::Error>>`

- [ ] **Step 1: Write the failing test**

Append to `mod tests` in `crates/cairn-sync/src/unwrap_key.rs`:

```rust
    #[test]
    fn the_override_wins_over_the_sibling_default() {
        let dir = tempfile::tempdir().unwrap();
        let elsewhere = dir.path().join("custody.key");
        assert_eq!(
            unwrap_file_path("/nodes/a/node.key", Some(elsewhere.to_str().unwrap())),
            elsewhere,
            "--unwrap-key names the file outright"
        );
    }

    #[test]
    fn the_default_is_the_sibling_of_the_signing_key() {
        assert_eq!(
            unwrap_file_path("/nodes/a/node.key", None),
            std::path::Path::new("/nodes/a/node.key.unwrap"),
            "same rule cairn-node uses to place the file it provisions"
        );
    }
```

- [ ] **Step 2: Run to verify failure**

```bash
CAIRN_ALLOW_DB_SKIP=1 cargo test -p cairn-sync --bin cairn-sync unwrap_key
```

Expected: **compile error** — `unwrap_file_path` is not defined.

- [ ] **Step 3: Implement the path rule and the startup entry point**

Append to the non-test part of `crates/cairn-sync/src/unwrap_key.rs`:

```rust
/// Where this daemon's unwrap-key file lives: the `<key>.unwrap` sibling of the signing
/// key by default — the same rule `cairn-node` uses to PLACE the file it provisions, so
/// the two agree without configuration — or the explicit `--unwrap-key` path when the
/// daemon's key file is not co-located with the node's.
///
/// Pure (no IO) so the rule is testable on its own.
pub fn unwrap_file_path(key_path: &str, override_path: Option<&str>) -> std::path::PathBuf {
    match override_path {
        Some(p) => std::path::PathBuf::from(p),
        None => cairn_keystore::keystore::unwrap_key_path_for(std::path::Path::new(key_path)),
    }
}

/// Resolve this daemon's custody key at startup, or refuse to start.
///
/// This is the one DB-touching step: it reads the registered public key, hands everything
/// to the pure [`resolve`], prints any warning, and returns the custody the process runs
/// with. Called from `cmd_pull`, `cmd_run` and the `serve` CLI arm — every command that
/// puts custody on the wire — BEFORE any network IO, so a misconfigured node fails at
/// startup rather than degrading into a peer that looks like it has no custody to offer.
pub fn resolve_at_startup(
    client: &mut postgres::Client,
    key_path: &str,
    unwrap_path_override: Option<&str>,
    signing_key: cairn_event::SigningKey,
) -> Result<NodeCustody, Box<dyn std::error::Error>> {
    let path = unwrap_file_path(key_path, unwrap_path_override);
    let display = path.display().to_string();

    // `query_opt`, not `query_one`: an absent row is a legitimate "nothing claimed".
    let row = client.query_opt("SELECT unwrap_pub FROM node_unwrap_key", &[])?;
    let registered: Option<[u8; 32]> = match row.map(|r| r.get::<_, Vec<u8>>(0)) {
        None => None,
        Some(bytes) => {
            // A ROW THAT IS NOT 32 BYTES IS A REFUSAL, NEVER "nothing registered".
            // db/037 CHECKs this column, so the branch is unreachable today — but a
            // daemon that does not own this schema must not stake a safety gate on a
            // constraint in it, and folding a malformed row into `None` would be
            // fail-OPEN inside a fail-fast gate.
            Some(bytes.as_slice().try_into().map_err(|_| {
                format!(
                    "node_unwrap_key.unwrap_pub is {} bytes, not 32 — this database's custody \
                     plane is malformed and this daemon cannot tell whether its own unwrap key \
                     agrees with it. Refusing to start rather than proceeding as though no key \
                     were registered.",
                    bytes.len()
                )
            })?)
        }
    };

    // The passphrase for a sealed key file, the same environment variable cairn-node
    // reads. Absent, a sealed file classifies Unusable (with the remedy named) — it never
    // degrades into "absent" and a derived key.
    let passphrase = std::env::var("CAIRN_KEY_PASSPHRASE").ok();
    let file = load_file_outcome(&path, passphrase.as_deref());
    let derived = cairn_event::seal::derive_unwrap_secret(&signing_key.to_bytes());

    match resolve(file, derived, registered.as_ref(), &display) {
        Resolution::Refuse(message) => Err(message.into()),
        Resolution::Use { secret, warning } => {
            if let Some(w) = warning {
                // Every startup, deliberately. A once-only notice is how a deleted key
                // file stays invisible until a restore needs it.
                eprintln!("cairn-sync: {w}");
            }
            Ok(NodeCustody {
                signing_key,
                unwrap_secret: secret,
            })
        }
    }
}
```

- [ ] **Step 4: Run to verify pass**

```bash
cargo fmt --all
CAIRN_ALLOW_DB_SKIP=1 cargo test -p cairn-sync --bin cairn-sync unwrap_key
```

Expected: **16 passed.**

- [ ] **Step 5: Replace the three startup arms**

In each of `cmd_pull` (~4310), `cmd_run` (~4640) and the serve CLI arm (~5558), replace the derive-plus-fence block written in Task 4 with:

```rust
    let (sk, _kid) = load_or_create_key(key_path)?;
    // ADR-0066: LOAD this node's provisioned unwrap key rather than deriving one that
    // cannot match it. Refuses to start on a divergence, on a restored node, or on a
    // corrupt key file; warns loudly on the pre-ADR-0066 fallback (issue #503).
    let custody = unwrap_key::resolve_at_startup(&mut client, key_path, unwrap_key_path, sk)?;
```

where `unwrap_key_path: Option<&str>` comes from the new `--unwrap-key` flag. The serve CLI arm builds its own one-off `startup_client` exactly as it does today, and `drop`s it after.

`assert_unwrap_key_registered`, `unwrap_key_matches` and `unwrap_key_divergence_message` are now **dead** — their job moved into `resolve`. Delete all three plus their unit tests at ~6396. Do not leave them behind "in case": a second, unreachable copy of a safety decision is worse than none.

- [ ] **Step 6: Add the CLI flag**

`pull`, `serve` and `run` each accept `--unwrap-key PATH`, read with the existing `flag(&args, "--unwrap-key")` helper. Update the usage text at ~5415-5436, e.g. for `pull`:

```
  pull        --conn URI --peer HOST:PORT --peer-name NAME [--metrics] [--full] [--key PATH] [--unwrap-key PATH]
              (--key: this node's signing key; --unwrap-key: its custody key, default <key>.unwrap — ADR-0066)
```

- [ ] **Step 7: Run the full gate**

```bash
cargo fmt --all
cargo clippy --workspace --tests -- -D warnings
scripts/run-db-gated-tests.sh
```

Expected: **PASS.** `clinical_pull.rs` is the suite most likely to break here — it provisions both nodes and registers a **derived** unwrap key deliberately (`clinical_pull.rs:409`), which is now the *fallback* row: absent file, derived matches registered. It should pass **and print the warning**. If it fails, the classification is wrong — do not "fix" it by weakening `resolve`.

- [ ] **Step 8: Commit**

```bash
git add -A
git commit -m "feat(#503): cairn-sync loads the node's provisioned unwrap key

Federated sync works again. The daemon resolves its custody key once at
startup from the <key>.unwrap sibling (or --unwrap-key), and refuses to
start on a divergence, on a restored node's fresh-seed derivation, or on
a corrupt key file. The pre-ADR-0066 derived fallback survives only where
the derived key provably equals the registered one, and says so loudly on
every startup.

assert_unwrap_key_registered and its two helpers are deleted, not kept:
their decision now lives in one place, and an unreachable second copy of
a safety decision is worse than none.

Closes #503"
```

---

### Task 6: Close-out — the guard's reason, the bench site, docs, follow-ups

**Files:**
- Modify: `crates/cairn-sync/src/main.rs:2849` (`cmd_bench_seal`)
- Modify: `crates/cairn-node/tests/unwrap_secret_is_not_derived.rs` (the cairn-sync `ALLOWED` reason)
- Modify: `docs/HANDOVER.md`, `docs/ROADMAP.md`

- [ ] **Step 1: Drop the bench's derive**

At `crates/cairn-sync/src/main.rs:2838-2849`, `cmd_bench_seal` derives a throwaway recipient key. It measures wrap/unwrap cost; the secret's provenance is irrelevant. Replace:

```rust
    use cairn_event::seal::{
        generate_unwrap_secret, seal_event_payload, unseal_event_payload, unwrap_dek,
        unwrap_public, wrap_dek_for,
    };
```

and

```rust
    // Recipient (node) unwrap keypair, generated fresh for the benchmark. Deliberately
    // NOT derived from a signing seed: this measures crypto cost, so the secret's
    // provenance is irrelevant, and reaching for `derive_unwrap_secret` here would keep
    // a dead coupling alive in a file that no longer has one (ADR-0066, issue #503).
    let secret = generate_unwrap_secret()?;
    let public = unwrap_public(&secret);
```

Delete the now-unused `let (sk, _kid) = cairn_event::generate_key()?;` if nothing else in the function uses `sk`.

- [ ] **Step 2: Rewrite the guard's `ALLOWED` reason**

In `crates/cairn-node/tests/unwrap_secret_is_not_derived.rs`, the `crates/cairn-sync/src/main.rs` entry's reason is now **false** — cairn-sync *can* read the keystore, and #503 is closed. Replace the reason (keep the entry):

```rust
    (
        "crates/cairn-sync/src/main.rs",
        "the pre-ADR-0066 fallback in `unwrap_key::resolve_at_startup` — a node whose \
         registered key IS its derived one has no `.unwrap` file to load, and refusing to \
         start would strand it. Admissible ONLY because the derived key is checked against \
         the registration first: a restored node's derivation does not match and is refused. \
         Retire this once no pre-ADR-0066 node can exist — tracked by the #503 follow-up",
    ),
```

Confirm the guard still passes: it must find a live call in that file (it will — `resolve_at_startup` derives) and no others.

- [ ] **Step 3: File the two follow-up issues**

```bash
gh issue create --title "Retire cairn-sync's pre-ADR-0066 derived-unwrap-key fallback" --body "$(cat <<'EOF'
#503 extracted `cairn-keystore` and converted `cairn-sync` to LOAD the node's provisioned
unwrap key. One derive site remains, by design: `unwrap_key::resolve_at_startup` falls back
to the pre-ADR-0066 derivation when there is no `<key>.unwrap` file AND the derived key
provably equals the one registered in `node_unwrap_key`.

That fallback is safe (a restored node's derivation does not match and is refused) but it
keeps `derive_unwrap_secret` alive in `cairn-sync` and keeps an entry in
`crates/cairn-node/tests/unwrap_secret_is_not_derived.rs`'s `ALLOWED` list.

**Retire it when no pre-ADR-0066 node can exist.** The project is pre-clinical with no
deployed nodes, so this may be closable almost immediately — it is filed rather than done
because "no such node exists" is a maintainer's fact, not an agent's.

Acceptance: `resolve_at_startup` takes no `derived` argument; the absent-file rows collapse
to a refusal naming `cairn-node establish-unwrap-key`; the `cairn-sync` `ALLOWED` entry is
deleted (the guard fails on a dead entry, so this is self-enforcing).
EOF
)"
```

```bash
gh issue create --title "cairn-sync and cairn-node disagree on the signing-key file format, and two error messages contradict each other" --body "$(cat <<'EOF'
Found while reading for #503; deliberately left out of that slice's scope.

`cairn-node --key` (default `node.key`) is raw 32 bytes or a sealed `CAIRNK1` CBOR bundle.
`cairn-sync --key` is a **hex-encoded text** seed, and `crates/cairn-sync/src/main.rs:798`
explicitly refuses a binary file:

> `<path>` exists but is not a hex seed (it looks binary — a sealed cairn-node key?);
> refusing to overwrite it. Point --key at this daemon's own key file.

But the ADR-0066 divergence fence 450 lines above it tells the operator the opposite:

> point --key at the same key this node was provisioned with

**Following that instruction hits the other message's refusal.** A single-node deployment
cannot point both binaries at one key file. The A→B integration test only works because
`clinical_pull.rs:252-272` hand-writes the same seed twice, in two formats, and documents
why.

Now that `cairn-keystore` exists (#503), `cairn-sync` *can* read cairn-node's format. The
fix is to make it do so — but it touches every `cairn-sync` verb that loads a key, plus
`write_key_file` in the test rig and `load_or_create_key`'s load-or-CREATE semantics (a
daemon that MINTS a key where a sealed one was expected is its own hazard). Hence a
separate slice.
EOF
)"
```

- [ ] **Step 4: Update HANDOVER and ROADMAP**

In `docs/HANDOVER.md`: delete the **⚠️ OPERATIONAL** warning that federated sync is inoperable (it is no longer true), and rewrite **trap 3** — `cairn-sync` no longer derives except in the guarded fallback. Add a session entry. Keep the file **under 500 lines**.

In `docs/ROADMAP.md`: note `crates/cairn-keystore` in the code-workspace listing and record #503 closed. **Never drop an open issue number while condensing.**

- [ ] **Step 5: Final full gate**

```bash
cargo fmt --all
cargo clippy --workspace --tests -- -D warnings
RUSTDOCFLAGS=-D warnings cargo doc --workspace --no-deps
scripts/run-db-gated-tests.sh
```

Expected: **EXIT 0.** Confirm the exit code directly — `echo $?` — and **never** pipe the run to `tail`.

- [ ] **Step 6: Commit and open the PR**

```bash
git add -A
git commit -m "docs(#503): the close-out — what closed, and what did not

Refs #503"
git push -u origin refactor/503-shared-cairn-keystore
```

Open the PR against `main`, linking #503 and naming both follow-up issues.
