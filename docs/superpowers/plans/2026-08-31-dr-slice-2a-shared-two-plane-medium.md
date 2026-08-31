# DR slice 2a — the shared, two-plane, append-only backup medium: Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Extract the backup-medium container format into a shared crate both binaries can use, and give it an append-only, chained, two-plane revision (`CAIRNB3`) that can carry clinical events alongside the federation plane.

**Architecture:** `crates/cairn-medium` is a pure crate (no DB, no I/O, no async). Today's `crates/cairn-node/src/medium.rs` moves into it verbatim and is then split by responsibility; `cairn-node` re-exports it so all 12 existing call sites compile untouched. On top of that, a new `CAIRNB3` revision replaces CAIRNB2's unappendable head marker with **one uniform structure repeated**: a length-prefixed, plane-tagged segment carrying its own signed attestation, chained to its predecessor. Appending costs O(new records), a torn append is self-limiting, and CAIRNB1/CAIRNB2 media keep parsing byte-for-byte as they do today.

**Tech Stack:** Rust 1.96, `cairn-event` (Ed25519/COSE sign + verify, content addressing), `hex`, `thiserror`, `uuid`, `serde_json`. No new third-party dependency is introduced.

**Spec:** [`docs/superpowers/specs/2026-08-31-dr-slice-2a-shared-two-plane-medium-design.md`](../specs/2026-08-31-dr-slice-2a-shared-two-plane-medium-design.md)

**Paper-parity: not clinical-surface** — a pure container format with no operator-visible act and no clinical workflow at any layer; the DR ceremony changes in slices 2c and 2d, which carry the falsifiable measurement.

---

## Global Constraints

Every task's requirements implicitly include this section.

- **Licence: AGPL-3.0-only.** House rule 1. **This slice adds no new third-party dependency** — every crate it uses is already vetted in this workspace. If you find yourself reaching for one, stop and ask.
- **TDD.** House rule 2: the failing test is written and *run* first. No production line exists without a test that drove it.
- **Inline documentation for a junior developer.** House rule 3: every non-trivial function and module says *why* it exists and how it fits, not just what the next line does.
- **Files under 500 lines** where feasible. House rule 4. This is why the crate is split into seven modules.
- **Never hard-code cryptographic material in tests.** House rule 6: keys, seeds, salts and nonces are **derived at runtime** (`cairn_event::generate_key()`, `std::array::from_fn(|i| …)`), never written as literals. A literal trips CodeQL's `rust/hard-coded-cryptographic-value` (critical) and blocks the scan until a human dismisses it (#146).
- **Rust 1.96**, pinned in `rust-toolchain.toml`. `[lints] workspace = true` in the manifest — the workspace denies `warnings` and clippy `all`.
- **rustfmt defaults.** Run `cargo fmt` before every commit; CI gates on `cargo fmt --check`.
- **Three Cargo trees, three lockfiles.** The root workspace, `extensions/cairn_pgx` and `cairn-gui` are separate trees; the last two are `exclude`d but ship anyway and depend on the root crates **by path**. `cairn-gui/cairn-gui-tauri` depends on `cairn-node`, so any change to `cairn-node`'s dependency set makes `cairn-gui/Cargo.lock` stale. **No root-workspace gate sees this** — CI runs clippy on the GUI tree with `--locked`, which *refuses* to regenerate.
- **Test commands.** Use `--all-targets` when the question is "is this reachable from production" (`--lib` and `--bin X` compile with `cfg(test)`, so unused items look used). Use `--no-fail-fast`. **Never pipe cargo to `tail`** — it masks cargo's exit status.
- **This slice needs no database.** Every test here is pure. If you run a wider suite, `export CAIRN_ALLOW_DB_SKIP=1` or the DB-gated suites fail rather than self-skip (#450).
- **Gate cost.** Adding a workspace member touches `Cargo.lock`, which relinks ~134 test binaries; macOS runs a one-time-per-binary Gatekeeper assessment on each. **Budget hours for the full local gate** and start it in the background. During development use `cargo test -p cairn-medium` and `cargo test -p cairn-node`.
- **Honesty constraint, specific to this slice.** 2a **closes nothing.** When it merges, the medium still carries no clinical event, `backup.rs::read_event_set` still reads `node_event` only, and `dr_clinical_guarantee_gap.rs::medium_carries_the_federation_plane_and_no_clinical_event` still passes **as a pin on the defect**. No doc comment, test name, or commit message may imply #500 is fixed. Every deferral written here names the slice that retires it.

---

## File Structure

**Created:**

| Path | Responsibility |
|---|---|
| `crates/cairn-medium/Cargo.toml` | Manifest; workspace lints; no new dependencies. |
| `crates/cairn-medium/src/lib.rs` | Crate docs (the format's invariants in one place) + explicit re-exports of the public surface. |
| `crates/cairn-medium/src/error.rs` | `BackupError`. |
| `crates/cairn-medium/src/chunk.rs` | `put_chunk` / `take_chunk` — the `[u32 BE len][bytes]` primitive, and its cap. |
| `crates/cairn-medium/src/marker.rs` | **CAIRNB2 only, frozen.** `SelfMarker`, `event_set_commitment`, `build/verify_self_attestation`, `scan_enrolls`, `enrolls`. |
| `crates/cairn-medium/src/container.rs` | Magic dispatch; CAIRNB1/B2 parse + serialize; CAIRNB3 section framing. |
| `crates/cairn-medium/src/segment.rs` | **CAIRNB3.** `MediumRecord`, `Plane`, `Segment`, their codecs, the segment attestation and its commitment. |
| `crates/cairn-medium/src/verify.rs` | `VerifyReport`, `verify_event(s)`, `verify_medium_bytes`, the chain pass, watermark, self-id. |

**Modified:**

| Path | Change |
|---|---|
| `crates/cairn-event/src/lib.rs` | Gains `pub const NIL_PATIENT`. |
| `crates/cairn-node/src/identity.rs:7` | `NIL_PATIENT` becomes a re-export of `cairn_event::NIL_PATIENT`. |
| `crates/cairn-node/src/lib.rs:16` | `pub mod medium;` → `pub use cairn_medium as medium;`. |
| `crates/cairn-node/src/medium.rs` | **Deleted** (moved to the new crate). |
| `crates/cairn-node/Cargo.toml` | Gains `cairn-medium = { path = "../cairn-medium" }`. |
| `Cargo.toml` | `crates/cairn-medium` added to `members`. |
| `Cargo.lock`, `cairn-gui/Cargo.lock`, `extensions/cairn_pgx/Cargo.lock` | Refreshed. |
| `docs/HANDOVER.md`, `docs/ROADMAP.md` | Slice entry + what is still open. |

**Untouched, deliberately:** `crates/cairn-node/src/backup.rs`, `restore.rs`, `main.rs` and the `tests/` suites. That they compile and pass **without edits** is the extraction's proof, exactly as it was for `cairn-keystore`'s 221 call sites in #503.

---

## Task 1: Move `NIL_PATIENT` into `cairn-event`

`medium.rs` reaches into `cairn-node` in exactly one place — `crate::identity::NIL_PATIENT`, the zero-UUID used as the `patient_id` of node-plane event bodies. It is a wire-level constant, so it belongs in `cairn-event`. This is the only thing standing between `medium.rs` and a clean extraction.

**Files:**
- Modify: `crates/cairn-event/src/lib.rs` (add the const beside `SHA2_256_MULTIHASH_PREFIX`, around line 52)
- Modify: `crates/cairn-node/src/identity.rs:7`
- Test: `crates/cairn-event/src/lib.rs` (inline `#[cfg(test)]`), `crates/cairn-node/tests/nil_patient_is_one_constant.rs`

**Interfaces:**
- Consumes: nothing.
- Produces: `cairn_event::NIL_PATIENT: &str`. `cairn_node::identity::NIL_PATIENT` keeps its path and value.

- [ ] **Step 1: Write the failing test in `cairn-event`**

Append to `crates/cairn-event/src/lib.rs`'s existing `#[cfg(test)] mod tests`:

```rust
    /// The nil UUID used as `patient_id` on events that are about the NODE, not a
    /// patient (node enrolments, pairings, backup-medium attestations). It lives here
    /// because it is a WIRE constant: it appears inside signed bodies, so a second
    /// spelling anywhere would be a second wire format.
    #[test]
    fn nil_patient_is_the_zero_uuid() {
        assert_eq!(
            crate::NIL_PATIENT,
            "00000000-0000-0000-0000-000000000000",
            "the nil patient id is a wire constant and must never drift"
        );
        assert!(
            crate::NIL_PATIENT.parse::<uuid::Uuid>().is_ok(),
            "it must parse as a UUID — it is bound as one at every apply door"
        );
    }
```

- [ ] **Step 2: Run it and verify it fails**

Run: `cargo test -p cairn-event nil_patient_is_the_zero_uuid`
Expected: FAIL — `error[E0425]: cannot find value 'NIL_PATIENT' in the crate root`.

- [ ] **Step 3: Add the constant**

In `crates/cairn-event/src/lib.rs`, immediately after `pub const BLAKE3_MULTIHASH_PREFIX`:

```rust
/// The nil UUID, used as `patient_id` on events that are about the NODE rather than a
/// patient — node enrolments, pairings, and the backup medium's own attestations.
///
/// It lives in `cairn-event` because it is a **wire** constant: it is serialized inside
/// signed bodies, so a second spelling in another crate would be a second wire format
/// that no test compares. `cairn_node::identity::NIL_PATIENT` re-exports this one value.
pub const NIL_PATIENT: &str = "00000000-0000-0000-0000-000000000000";
```

- [ ] **Step 4: Run the test and verify it passes**

Run: `cargo test -p cairn-event nil_patient_is_the_zero_uuid`
Expected: PASS.

- [ ] **Step 5: Write the failing guard that the two paths are one value**

Create `crates/cairn-node/tests/nil_patient_is_one_constant.rs`:

```rust
//! The nil-patient UUID must have exactly ONE definition in the workspace.
//!
//! It moved to `cairn-event` (a wire constant belongs with the wire format) and
//! `cairn_node::identity` re-exports it. This guard exists because the failure mode of a
//! re-export quietly becoming a second literal is invisible: both spellings are the same
//! string today, so nothing breaks until someone edits one of them — and by then it is
//! inside signed bytes on two nodes that no longer agree.
//!
//! Pointer equality is the assertion, not string equality: two identical `&'static str`
//! literals may or may not be interned, but a genuine re-export is always the SAME
//! pointer. That is what distinguishes "re-exported" from "copied".

#[test]
fn node_identity_reexports_the_cairn_event_constant() {
    assert_eq!(
        cairn_node::identity::NIL_PATIENT,
        cairn_event::NIL_PATIENT,
        "the two paths must name the same value"
    );
    assert!(
        std::ptr::eq(
            cairn_node::identity::NIL_PATIENT.as_ptr(),
            cairn_event::NIL_PATIENT.as_ptr()
        ),
        "cairn_node::identity::NIL_PATIENT must RE-EXPORT cairn_event::NIL_PATIENT, \
         not re-declare an identical literal — a copy drifts silently"
    );
}
```

- [ ] **Step 6: Run it and verify it fails**

Run: `cargo test -p cairn-node --test nil_patient_is_one_constant`
Expected: FAIL on the `ptr::eq` assertion — `identity.rs` still declares its own literal.

- [ ] **Step 7: Turn the declaration into a re-export**

In `crates/cairn-node/src/identity.rs`, replace line 7:

```rust
pub const NIL_PATIENT: &str = "00000000-0000-0000-0000-000000000000";
```

with:

```rust
// The nil patient id is a WIRE constant (it is serialized inside signed bodies), so it
// lives in `cairn-event` with the rest of the wire format. Re-exported — not
// re-declared — so there is exactly one value in the workspace; `tests/
// nil_patient_is_one_constant.rs` asserts that with pointer equality, because two
// identical literals would compare equal and still drift apart later.
pub use cairn_event::NIL_PATIENT;
```

- [ ] **Step 8: Run both tests and the crates that use the constant**

Run: `cargo test -p cairn-event -p cairn-node --all-targets --no-fail-fast nil_patient`
Expected: both PASS.

Run: `cargo build -p cairn-node --all-targets`
Expected: clean — no `unused_imports`, no `dead_code`. (A plain build, not a filtered test run: `cargo test --bin`/`--lib` compiles with `cfg(test)` and makes unreachable items look used.)

- [ ] **Step 9: Commit**

```bash
cargo fmt
git add crates/cairn-event/src/lib.rs crates/cairn-node/src/identity.rs \
        crates/cairn-node/tests/nil_patient_is_one_constant.rs
git commit -m "refactor(#500): NIL_PATIENT is a wire constant — one definition, in cairn-event

medium.rs's only crate-internal dependency, and the last thing standing between it
and a clean extraction into a shared crate. Re-exported from cairn_node::identity
rather than re-declared, guarded by pointer equality so a future copy is caught."
```

---

## Task 2: Create `crates/cairn-medium` and move `medium.rs` into it verbatim

One behaviour-preserving move. The file arrives whole; the split into modules is Task 3. Keeping them apart means a reviewer can check "nothing changed" and "nothing changed" separately, instead of reading a 700-line diff that does both.

**Files:**
- Create: `crates/cairn-medium/Cargo.toml`, `crates/cairn-medium/src/lib.rs`
- Delete: `crates/cairn-node/src/medium.rs`
- Modify: `Cargo.toml` (root `members`), `crates/cairn-node/Cargo.toml`, `crates/cairn-node/src/lib.rs:16`
- Modify: `Cargo.lock`, `cairn-gui/Cargo.lock`, `extensions/cairn_pgx/Cargo.lock`

**Interfaces:**
- Consumes: `cairn_event::NIL_PATIENT` (Task 1).
- Produces: the crate `cairn_medium`, whose public surface is exactly today's `medium.rs` public surface: `MEDIUM_MAGIC_V1`, `MEDIUM_MAGIC_V2`, `SELF_ATTEST_TYPE`, `BackupError`, `SelfMarker`, `Container`, `EnrollScan`, `VerifyReport`, `scan_enrolls`, `enrolls`, `event_set_commitment`, `build_self_attestation`, `verify_self_attestation`, `serialize_container`, `parse_container`, `parse_medium`, `verify_event`, `verify_events`, `verify_medium_bytes`, `serialize_and_verify_container`. Reachable from `cairn-node` at the unchanged path `cairn_node::medium::*` / `crate::medium::*`.

- [ ] **Step 1: Write the manifest**

Create `crates/cairn-medium/Cargo.toml`:

```toml
[package]
name = "cairn-medium"
version = "0.1.0"
edition.workspace = true
rust-version.workspace = true
license.workspace = true
repository.workspace = true
# The backup-medium container format, shared by cairn-node (which writes the node
# plane and orchestrates restore) and, from slice 2b, cairn-sync (which owns the
# clinical plane). Not published: like the sibling crates it carries a version-less
# `cairn-event` path dependency, which cargo-deny's wildcard gate allows only for an
# unpublished crate.
publish = false

# Inherit the central workspace lint policy (#144).
[lints]
workspace = true

[dependencies]
cairn-event = { path = "../cairn-event" }
# Every dependency below is already a vetted cairn-node dependency — this crate is an
# extraction, so it adds no new licence surface (house rule 1). hex, thiserror and
# uuid are dual MIT/Apache-2.0; serde_json is dual MIT/Apache-2.0. All
# AGPL-3.0-compatible.
hex = "0.4"
serde_json = "1"
thiserror = "1"
uuid = { version = "1", features = ["v7"] }
```

These four requirements were checked against `crates/cairn-node/Cargo.toml` and `crates/cairn-event/Cargo.toml` on 2026-08-31 and match both verbatim, so the new crate resolves to the versions the workspace has already locked and vetted. If they have since moved, copy the current ones rather than these.

- [ ] **Step 2: Register the member**

In the root `Cargo.toml`, add to `members` in alphabetical position:

```toml
    "crates/cairn-medium", # the shared backup-medium container format (#500 slice 2a)
```

- [ ] **Step 3: Move the file**

```bash
mkdir -p crates/cairn-medium/src
git mv crates/cairn-node/src/medium.rs crates/cairn-medium/src/lib.rs
```

Then make exactly three edits to the moved file, and no others:

1. Replace both occurrences of `crate::identity::NIL_PATIENT` with `cairn_event::NIL_PATIENT` (lines 172 and 456 in the original).
2. Rewrite the two intra-doc links that pointed at `cairn-node` modules — `[`crate::restore::Provenance::SignedFederated`]` (module docs, and again in `verify_self_attestation`'s docs) and `[`crate::restore::RestoreError::NoVerifiableGenesis`]` (in `EnrollScan`'s docs) — as plain text naming the type, e.g. `` `cairn_node::restore::Provenance::SignedFederated` ``. **They must not stay as intra-doc links**: they cannot resolve from this crate, and CI runs `cargo doc` with `RUSTDOCFLAGS=-D warnings`, which fails two jobs on a broken link.
3. Add a crate-level header above the existing module docs:

```rust
//! The backup-medium container format (ADR-0026 slice B, issue #53).
//!
//! WHY A CRATE: `cairn-node` writes the federation plane and orchestrates restore, and
//! from slice 2b `cairn-sync` writes the clinical plane — it owns the clinical event
//! log, the wire protocol and the transport seam. Both need this format, and a
//! production dependency from `cairn-sync` onto `cairn-node` (an application crate
//! carrying clap, rustls, rcgen and tokio-postgres) is the wrong direction. Same shape
//! as `cairn-keystore`, extracted for the same reason in #503.
//!
//! Pure by construction: no database, no I/O, no async. Serialization, parsing and
//! signature checks only, so every property below is unit-testable without a fixture
//! larger than a byte slice.
//!
//! SCOPE TODAY: this crate carries the format. It does NOT read a database and does not
//! decide what goes on a medium — `cairn-node`'s `backup.rs` still reads `node_event`
//! and nothing else, which is issue #500 and is NOT fixed by this crate existing.
```

- [ ] **Step 4: Add the dependency and the re-export in `cairn-node`**

In `crates/cairn-node/Cargo.toml`, beside the existing `cairn-keystore` line:

```toml
cairn-medium = { path = "../cairn-medium" }
```

In `crates/cairn-node/src/lib.rs`, delete `pub mod medium;` (line 16) and extend the existing re-export block:

```rust
// The backup-medium container format now lives in its own crate so `cairn-sync` can
// write the clinical plane onto the same medium this node writes its federation plane
// to (issue #500 slice 2a), without depending on a node application. Re-exported rather
// than renamed at the call sites — `crate::medium::…` and `cairn_node::medium::…` appear
// at 12 sites across backup.rs, restore.rs, main.rs and two test suites, and every one of
// them compiling untouched is what proves the move changed no behaviour. This is not a
// deprecated shim: `cairn-node` genuinely still offers this module, implemented elsewhere.
pub use cairn_medium as medium;
```

- [ ] **Step 5: Refresh all three lockfiles**

```bash
cargo check --workspace
(cd cairn-gui && cargo check --workspace)
(cd extensions/cairn_pgx && cargo check)
git status --short -- '*Cargo.lock'
```

Expected: **all three** lockfiles modified. `cairn-gui/cairn-gui-tauri` depends on `cairn-node` by path, so its lock now needs the new crate. If `cairn-gui/Cargo.lock` is *not* listed, stop — CI's `--locked` clippy on that tree is the only gate that sees this, and it refuses to regenerate (`error: cannot update the lock file … because --locked was passed`).

- [ ] **Step 6: Run the moved tests and every call site**

Run: `cargo test -p cairn-medium --all-targets --no-fail-fast`
Expected: the 19 moved unit tests PASS.

Run: `cargo test -p cairn-node --all-targets --no-fail-fast backup restore`
Expected: PASS, with **zero edits** to `backup.rs`, `restore.rs`, `main.rs`, `tests/backup.rs` or `tests/restore.rs`. If any of them needed editing, the move was not verbatim — find out what changed rather than fixing the call site.

Run: `cargo doc -p cairn-medium --no-deps`
Expected: clean. (CI runs this with `RUSTDOCFLAGS=-D warnings`; a broken intra-doc link fails two jobs.)

- [ ] **Step 7: Verify the lockfile guard and the source sweep still pass**

Run: `cargo test -p cairn-node --test cargo_lockfiles_tracked --test unwrap_secret_is_not_derived`
Expected: PASS. `PRODUCTION_TREES` is `["crates", "extensions", "cairn-gui"]` — directory-based, so the new crate is swept automatically and **nothing is added to any allow-list**. `cairn-medium` calls `derive_unwrap_secret` nowhere.

- [ ] **Step 8: Commit**

```bash
cargo fmt
git add -A
git commit -m "refactor(#500): extract crates/cairn-medium — the medium format, moved verbatim

cairn-sync owns the clinical plane and from slice 2b must write it onto the same
medium cairn-node writes its federation plane to, but medium.rs lived in cairn-node
and a production dependency on a node application is the wrong direction. Same shape
#503 resolved by extracting cairn-keystore.

Moved whole, with three edits and no others: NIL_PATIENT now comes from cairn-event,
two intra-doc links into cairn-node became plain text (they cannot resolve from here
and cargo doc runs with -D warnings), and a crate header. cairn-node re-exports the
crate as its own 'medium' module, so all 12 call sites compile untouched - that is
the move's whole proof.

This fixes NOTHING. backup.rs still reads node_event and only node_event; the medium
still carries no clinical event. That is #500 and it stays open."
```

---

## Task 3: Split the moved file into modules

Pure code motion, guarded by the tests that just passed. Today's file is 706 lines and grows in Tasks 4–8; house rule 4 wants it under 500.

**Files:**
- Create: `crates/cairn-medium/src/{error,chunk,marker,container,verify}.rs`
- Modify: `crates/cairn-medium/src/lib.rs` (becomes docs + module declarations + re-exports)

**Interfaces:**
- Consumes: Task 2's crate.
- Produces: the same public surface at the same paths (`cairn_medium::parse_container`, etc.), now backed by five modules. `chunk::{put_chunk, take_chunk, MAX_CHUNK_BYTES}` become `pub(crate)` — Tasks 4–7 build on them.

- [ ] **Step 1: Create the modules and move each item, with its tests**

No behaviour changes. Move each item **and the tests that exercise it**:

| Module | Items | Tests that move with them |
|---|---|---|
| `error.rs` | `BackupError` | — |
| `chunk.rs` | `MAX_CHUNK_BYTES`, `put_chunk`, `take_chunk` (both `pub(crate)`) | `parse_rejects_a_truncated_frame` |
| `marker.rs` | `SELF_ATTEST_TYPE`, `SelfMarker`, `EnrollScan`, `scan_enrolls`, `enrolls`, `event_set_commitment`, `build_self_attestation`, `verify_self_attestation`, `put_marker` | the six `signed_attestation_*` tests and `tampered_signed_attestation_fails_closed` |
| `container.rs` | `MEDIUM_MAGIC_V1`, `MEDIUM_MAGIC_V2`, `KIND_NONE/UNSIGNED/SIGNED`, `Container`, `serialize_container`, `parse_container`, `take_frames`, `parse_medium` | the four `container_roundtrips_*` / `legacy_cairnb1_*` tests and `parse_rejects_missing_magic_and_unknown_kind` |
| `verify.rs` | `VerifyReport`, `verify_event`, `verify_events`, `verify_medium_bytes`, `serialize_and_verify_container` | `verify_pinpoints_a_tampered_event_through_the_container`, `serialize_and_verify_refuses_a_tampered_set` |

**Visibility the split forces.** Splitting one file into five turns three previously-private
calls into cross-module ones. Make exactly these `pub(crate)`, and nothing else:
`chunk::{MAX_CHUNK_BYTES, put_chunk, take_chunk}` (every module frames with them),
`marker::put_marker` (called by `container::serialize_container`). Everything else that was
private stays private — widening more than the compiler demands is how a crate's public
surface grows without anyone deciding to grow it.

The four shared test helpers — `sk()`, `kid()`, `node_id()`, `enroll()` — are needed by several modules' tests. Put them in a single `#[cfg(test)] pub(crate) mod testkit;` module (`crates/cairn-medium/src/testkit.rs`) and have each module's `mod tests` import them. Do **not** copy them: four copies is four places for the `enroll()` fixture to drift, and the fixture is what every attestation test's meaning rests on.

- [ ] **Step 2: Write `lib.rs` as docs + declarations + explicit re-exports**

```rust
//! (the crate header written in Task 2 stays here, unchanged)
//!
//! # Module map
//!
//! - [`chunk`] — the `[u32 BE len][bytes]` primitive every other module frames with.
//! - [`marker`] — **CAIRNB2 only, and frozen.** The head self-marker and its
//!   whole-set commitment. It serves media that already exist and gains nothing:
//!   CAIRNB3's equivalent is [`segment`], because a whole-set commitment cannot
//!   survive an append (see the crate docs). Do not extend this module.
//! - [`container`] — magic dispatch and the on-disk framing of every revision.
//! - [`verify`] — signature verification, and (from slice 2a) the chain pass.

mod chunk;
mod container;
mod error;
mod marker;
mod verify;

#[cfg(test)]
mod testkit;

pub use container::{
    parse_container, parse_medium, serialize_container, Container, MEDIUM_MAGIC_V1,
    MEDIUM_MAGIC_V2,
};
pub use error::BackupError;
pub use marker::{
    build_self_attestation, enrolls, event_set_commitment, scan_enrolls, verify_self_attestation,
    EnrollScan, SelfMarker, SELF_ATTEST_TYPE,
};
pub use verify::{
    serialize_and_verify_container, verify_event, verify_events, verify_medium_bytes, VerifyReport,
};
```

Explicit re-exports, not `pub use module::*` — a glob makes it impossible to see from `lib.rs` what this crate promises, and this crate's promise is the thing three other crates read.

- [ ] **Step 3: Run the tests — the same 19, still passing**

Run: `cargo test -p cairn-medium --all-targets --no-fail-fast`
Expected: **19 tests, all PASS**, the same names as before the split. A test that vanished was dropped, not moved; a test that failed means the split changed behaviour.

Run: `cargo test -p cairn-node --all-targets --no-fail-fast backup restore`
Expected: PASS, still with zero edits to `cairn-node`.

- [ ] **Step 4: Check the file sizes**

Run: `wc -l crates/cairn-medium/src/*.rs`
Expected: every file under 500 lines. If one is not, it is `marker.rs` — split its attestation half into `marker/attest.rs`.

- [ ] **Step 5: Commit**

```bash
cargo fmt
git add -A
git commit -m "refactor(#500): split cairn-medium by responsibility

Pure code motion, no behaviour change: the same 19 tests pass under the same names,
and cairn-node still needs no edits. The moved file was 706 lines and grows in the
next five tasks; house rule 4 wants it under 500.

marker.rs is marked FROZEN in the module map: it serves CAIRNB2 media and gains
nothing, because CAIRNB3's equivalent cannot be a whole-set commitment."
```

---

## Task 4: `MediumRecord` and its codec

The first new code. A record is one event as the medium carries it — deliberately the same five fields `EventsResponse` carries on the sync wire (`events`, `attestations`, `attester_keys`, `wrapped_deks`, `seqs`), because a medium that carried less would be a lookalike rather than a peer, and a restore would silently lose the attestation a suppressing event needs to be admitted at all.

**Files:**
- Create: `crates/cairn-medium/src/segment.rs`
- Modify: `crates/cairn-medium/src/lib.rs` (declare + re-export)

**Interfaces:**
- Consumes: `chunk::{put_chunk, take_chunk}`, `error::BackupError`.
- Produces:
  - `pub struct MediumRecord { pub signed_bytes: Vec<u8>, pub attestation: Option<Vec<u8>>, pub attester_key: Option<Vec<u8>>, pub dek_wrapped: Option<Vec<u8>>, pub source_seq: i64 }`
  - `pub(crate) fn put_record(out: &mut Vec<u8>, r: &MediumRecord)`
  - `pub(crate) fn take_record(rest: &[u8]) -> Result<(MediumRecord, &[u8]), BackupError>`

- [ ] **Step 1: Write the failing tests**

Create `crates/cairn-medium/src/segment.rs` with only a `#[cfg(test)] mod tests` block for now:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    /// Runtime-derived bytes for a fixture field. NEVER a literal: a byte-array literal in
    /// a crypto context trips CodeQL's `rust/hard-coded-cryptographic-value` (house rule 6,
    /// issue #146), and a wrapped DEK is exactly such a context.
    fn bytes(seed: u8, len: usize) -> Vec<u8> {
        (0..len).map(|i| seed.wrapping_add(i as u8)).collect()
    }

    fn record(flags: u8) -> MediumRecord {
        MediumRecord {
            signed_bytes: bytes(1, 40),
            attestation: (flags & 0b001 != 0).then(|| bytes(2, 16)),
            attester_key: (flags & 0b010 != 0).then(|| bytes(3, 32)),
            dek_wrapped: (flags & 0b100 != 0).then(|| bytes(4, 48)),
            source_seq: 7,
        }
    }

    /// Every combination of the three optional fields survives a round trip. All eight,
    /// not a sample: the flags byte is a bitfield, and a codec that drops one bit is
    /// exactly the defect that would lose custody on one class of event and no other.
    #[test]
    fn every_flag_combination_roundtrips() {
        for flags in 0..8u8 {
            let r = record(flags);
            let mut out = Vec::new();
            put_record(&mut out, &r);
            let (back, rest) = take_record(&out).expect("decodes");
            assert_eq!(back, r, "flags {flags:03b} did not round-trip");
            assert!(rest.is_empty(), "flags {flags:03b} left {} trailing bytes", rest.len());
        }
    }

    /// A node-plane record is a clinical record with every optional field absent. One
    /// shape serves both planes; a second shape would be a second place for a check to
    /// go stale (the #173 twin-dispatch lesson).
    #[test]
    fn a_node_plane_record_carries_no_custody() {
        let r = record(0b000);
        assert_eq!((r.attestation.as_ref(), r.attester_key.as_ref(), r.dek_wrapped.as_ref()),
                   (None, None, None));
        let mut out = Vec::new();
        put_record(&mut out, &r);
        assert_eq!(take_record(&out).unwrap().0, r);
    }

    /// An ABSENT optional field decodes as `None`, never as `Some(vec![])`.
    ///
    /// This is the assertion most likely to pass vacuously, and it is load-bearing: at the
    /// apply door "no attestation travelled" is refused fail-closed for a suppressing
    /// event, while "an empty attestation travelled" is a token that fails validation for a
    /// different reason and reports differently. Conflating them turns a fail-closed gate
    /// into a confusing one.
    #[test]
    fn an_absent_field_is_none_and_an_empty_one_is_some_empty() {
        let mut absent = record(0b000);
        absent.source_seq = -1; // a legal seq: the medium records what the source said
        let mut out = Vec::new();
        put_record(&mut out, &absent);
        assert_eq!(take_record(&out).unwrap().0.attestation, None);

        let present_but_empty = MediumRecord { attestation: Some(Vec::new()), ..record(0b000) };
        let mut out2 = Vec::new();
        put_record(&mut out2, &present_but_empty);
        assert_eq!(
            take_record(&out2).unwrap().0.attestation,
            Some(Vec::new()),
            "an empty attestation is a DIFFERENT fact from an absent one"
        );
    }

    /// An unrecognised flag bit means a newer writer put fields here we cannot parse.
    /// Refuse loudly rather than decode the prefix we understand and silently drop the
    /// rest — a silently-truncated record is #500's failure shape at record scale.
    #[test]
    fn an_unknown_flag_bit_is_refused_by_name() {
        let mut out = Vec::new();
        put_record(&mut out, &record(0b000));
        // The flags byte sits immediately after the signed_bytes chunk.
        let flags_at = 4 + 40;
        out[flags_at] = 0b1000;
        let err = take_record(&out).expect_err("must refuse");
        let msg = err.to_string();
        assert!(msg.contains("flag"), "the error must name the flags byte: {msg}");
        assert!(msg.contains("1000") || msg.contains('8'), "and the unknown bits: {msg}");
    }

    /// A truncated record is reported, never panics.
    #[test]
    fn a_truncated_record_is_an_error_not_a_panic() {
        let mut out = Vec::new();
        put_record(&mut out, &record(0b111));
        for cut in [1usize, 5, 20, out.len() - 1] {
            assert!(take_record(&out[..cut]).is_err(), "cut at {cut} must error");
        }
    }
}
```

- [ ] **Step 2: Run and verify they fail**

Run: `cargo test -p cairn-medium --lib segment`
Expected: FAIL to compile — `MediumRecord`, `put_record`, `take_record` do not exist.

- [ ] **Step 3: Implement the type and codec**

Prepend to `crates/cairn-medium/src/segment.rs`:

```rust
//! CAIRNB3 — the append-only, plane-tagged segment and the records it carries.
//!
//! WHY THIS EXISTS, and why it is not [`crate::marker`]: a CAIRNB2 medium carries ONE
//! head marker whose signature commits to `event_set_commitment(events)` — the whole
//! sorted event set. That is unappendable by construction. Adding a single event changes
//! the commitment, so every append would need the head re-signed, and rewriting the head
//! shifts every byte after it: a whole-file rewrite on every backup, over a log that
//! grows for the life of a clinic.
//!
//! So CAIRNB3 has no head block. Each SEGMENT — one append increment — carries its own
//! signed attestation naming this node and committing to its own records plus its
//! predecessor's commitment. Appending costs O(new records), the chain is verifiable
//! end-to-end, and a segment lifted from another medium fails on its predecessor.
//!
//! A RECORD is one event as the medium carries it. Its fields are deliberately the same
//! five `cairn-sync`'s `EventsResponse` carries on the wire (`events`, `attestations`,
//! `attester_keys`, `wrapped_deks`, `seqs`), because slice 2b addresses the medium
//! through that same protocol. A medium carrying less would be a lookalike, not a peer —
//! and a restore through `apply_remote_event` would silently lose the attestation a
//! suppressing event needs in order to be admitted at all.

use crate::chunk::{put_chunk, take_chunk};
use crate::error::BackupError;

/// Flags byte: which optional fields follow the `signed_bytes` chunk.
const FLAG_ATTESTATION: u8 = 0b001;
const FLAG_ATTESTER_KEY: u8 = 0b010;
const FLAG_DEK: u8 = 0b100;
/// Every bit this build understands. A record setting anything outside this mask was
/// written by a newer Cairn and is REFUSED — see `take_record`.
const KNOWN_FLAGS: u8 = FLAG_ATTESTATION | FLAG_ATTESTER_KEY | FLAG_DEK;

/// One event on the medium, in the shape the sync wire carries it.
///
/// `source_seq` is the CAPTURING node's local insertion order for this event — the
/// medium's cursor. It is stored per record rather than per segment so the medium can
/// answer a cursored request the way a serving node does, and so an interrupted restore
/// can resume from where it stopped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediumRecord {
    /// The event itself: COSE_Sign1 bytes, verbatim, never re-serialized.
    pub signed_bytes: Vec<u8>,
    /// The human attestation token, when one travelled. `None` (no token) and
    /// `Some(vec![])` (an empty token) are DIFFERENT facts and must stay distinguishable:
    /// the apply door refuses a suppressing event with no token fail-closed, and reports
    /// an invalid token differently.
    pub attestation: Option<Vec<u8>>,
    /// The attester's public key, when one travelled.
    pub attester_key: Option<Vec<u8>>,
    /// This event's DEK, wrapped to the capturing node's unwrap public key. `None`
    /// whenever no custody travels: the event is unsealed, this node holds no DEK for it,
    /// or it has been shredded here.
    pub dek_wrapped: Option<Vec<u8>>,
    /// The capturing node's local `seq` for this event.
    pub source_seq: i64,
}

/// Encode one record. Pure.
pub(crate) fn put_record(out: &mut Vec<u8>, r: &MediumRecord) {
    put_chunk(out, &r.signed_bytes);
    let mut flags = 0u8;
    if r.attestation.is_some() {
        flags |= FLAG_ATTESTATION;
    }
    if r.attester_key.is_some() {
        flags |= FLAG_ATTESTER_KEY;
    }
    if r.dek_wrapped.is_some() {
        flags |= FLAG_DEK;
    }
    out.push(flags);
    for field in [&r.attestation, &r.attester_key, &r.dek_wrapped] {
        if let Some(v) = field {
            put_chunk(out, v);
        }
    }
    out.extend_from_slice(&r.source_seq.to_be_bytes());
}

/// Read one optional field iff its flag bit is set, advancing `rest`. Pure and reusable —
/// the three optional fields differ only in which bit gates them, and three inline copies
/// is three places for one of them to be forgotten.
fn take_optional(
    flags: u8,
    bit: u8,
    rest: &mut &[u8],
) -> Result<Option<Vec<u8>>, BackupError> {
    if flags & bit == 0 {
        return Ok(None);
    }
    let (v, r) = take_chunk(rest)?;
    *rest = r;
    Ok(Some(v.to_vec()))
}

/// Decode one record, returning it and the remainder. Errors (never panics) on a
/// truncated record or an unrecognised flag bit.
pub(crate) fn take_record(rest: &[u8]) -> Result<(MediumRecord, &[u8]), BackupError> {
    let (signed_bytes, rest) = take_chunk(rest)?;
    let (&flags, mut rest) = rest.split_first().ok_or_else(|| {
        BackupError::Decode("truncated record: no flags byte after signed_bytes".into())
    })?;
    // Fail closed on an unknown bit. A newer writer setting bit 3 has put a field here we
    // cannot locate, so everything after it — including this record's source_seq and every
    // following record — would decode as garbage. Refusing names what we did not
    // understand; decoding the prefix would silently drop it.
    if flags & !KNOWN_FLAGS != 0 {
        return Err(BackupError::Decode(format!(
            "record sets unknown flag bit(s) {:04b} (this build understands {KNOWN_FLAGS:03b}); \
             the medium was written by a newer Cairn — upgrade this node before reading it",
            flags & !KNOWN_FLAGS
        )));
    }
    let attestation = take_optional(flags, FLAG_ATTESTATION, &mut rest)?;
    let attester_key = take_optional(flags, FLAG_ATTESTER_KEY, &mut rest)?;
    let dek_wrapped = take_optional(flags, FLAG_DEK, &mut rest)?;
    if rest.len() < 8 {
        return Err(BackupError::Decode(format!(
            "truncated record: {} byte(s) where an 8-byte source_seq was expected",
            rest.len()
        )));
    }
    let (seq, rest) = rest.split_at(8);
    let source_seq = i64::from_be_bytes(seq.try_into().expect("8 bytes"));
    Ok((
        MediumRecord {
            signed_bytes: signed_bytes.to_vec(),
            attestation,
            attester_key,
            dek_wrapped,
            source_seq,
        },
        rest,
    ))
}
```

Declare and re-export in `lib.rs`: add `mod segment;` and `pub use segment::MediumRecord;`.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p cairn-medium --lib segment -- --nocapture`
Expected: all five PASS.

- [ ] **Step 5: Mutation-check the two assertions most likely to be vacuous**

The recorded lesson is that a green suite survived 7 of 11 production mutations (PR #410), and that a mutation which does not change the property tests nothing. Verify each of these *fails*, then revert:

1. In `put_record`, always write the three chunks regardless of `Option` → `every_flag_combination_roundtrips` must fail.
2. In `take_record`, drop the `flags & !KNOWN_FLAGS` guard → `an_unknown_flag_bit_is_refused_by_name` must fail.
3. In `take_record`, decode an absent field as `Some(Vec::new())` → `an_absent_field_is_none_and_an_empty_one_is_some_empty` must fail.

If any mutation leaves the suite green, the test is not asserting what it claims — fix the test before moving on.

- [ ] **Step 6: Commit**

```bash
cargo fmt
git add -A
git commit -m "feat(#500): MediumRecord — one event as the medium carries it

The five fields are deliberately the five EventsResponse carries on the sync wire, so
slice 2b can address the medium through the same protocol a network peer uses. Absent
and empty optional fields stay distinguishable (the apply door treats them
differently), and an unknown flag bit is REFUSED by name rather than decoded as a
prefix — a silently truncated record is #500's failure shape at record scale."
```

---

## Task 5: `Plane`, `Segment` and the section framing

**Files:**
- Modify: `crates/cairn-medium/src/segment.rs`, `crates/cairn-medium/src/lib.rs`

**Interfaces:**
- Consumes: Task 4's `MediumRecord`, `put_record`, `take_record`.
- Produces:
  - `pub enum Plane { Node, Clinical }` with `pub fn tag(self) -> u8` and `pub fn from_tag(t: u8) -> Option<Plane>`
  - `pub struct Segment { pub plane: Plane, pub index: u32, pub prev_commitment: String, pub self_node_id_hex: String, pub attestation: Option<Vec<u8>>, pub records: Vec<MediumRecord> }`
  - `pub struct UnknownSegment { pub plane_tag: u8, pub index: u32, pub record_count: u32 }`
  - `pub(crate) fn put_segment(out: &mut Vec<u8>, seg: &Segment)` — writes the `[u32 len][segment]` section
  - `pub(crate) enum TakenSection { Known(Segment), Unknown(UnknownSegment) }`
  - `pub(crate) fn take_section(rest: &[u8]) -> Result<Option<(TakenSection, &[u8])>, BackupError>` — `Ok(None)` means a torn tail

- [ ] **Step 1: Write the failing tests**

Add to `segment.rs`'s `mod tests`:

```rust
    fn segment(plane: Plane, index: u32, n: usize) -> Segment {
        Segment {
            plane,
            index,
            prev_commitment: if index == 0 { String::new() } else { "beef".into() },
            self_node_id_hex: "abcd".into(),
            attestation: Some(bytes(9, 64)),
            records: (0..n).map(|_| record(0b111)).collect(),
        }
    }

    #[test]
    fn a_segment_roundtrips_through_its_section_framing() {
        for plane in [Plane::Node, Plane::Clinical] {
            let seg = segment(plane, 3, 4);
            let mut out = Vec::new();
            put_segment(&mut out, &seg);
            let (taken, rest) = take_section(&out).expect("no error").expect("not torn");
            assert!(rest.is_empty());
            match taken {
                TakenSection::Known(back) => assert_eq!(back, seg),
                TakenSection::Unknown(u) => panic!("known plane decoded as unknown: {u:?}"),
            }
        }
    }

    /// An unsigned segment still names itself. The signing key may be unavailable at
    /// capture, and an unavailable key must never BLOCK a backup — it travels flagged.
    /// `self_node_id_hex` is what closes the operator-typo footgun, exactly as
    /// `SelfMarker::Unsigned` does on a CAIRNB2 medium.
    #[test]
    fn an_unsigned_segment_still_carries_its_self_id() {
        let seg = Segment { attestation: None, ..segment(Plane::Clinical, 0, 2) };
        let mut out = Vec::new();
        put_segment(&mut out, &seg);
        let (taken, _) = take_section(&out).unwrap().unwrap();
        match taken {
            TakenSection::Known(back) => {
                assert_eq!(back.attestation, None, "unsigned stays unsigned");
                assert_eq!(back.self_node_id_hex, "abcd", "and still names itself");
            }
            other => panic!("{other:?}"),
        }
    }

    /// An unrecognised plane tag is NAMED, never skipped. The header layout is fixed
    /// regardless of plane, so index and record count are still readable — and reporting
    /// them is what lets a caller say "12 clinical records I could not read" rather than
    /// silently restoring a medium that is missing a plane.
    #[test]
    fn an_unknown_plane_tag_is_named_not_skipped() {
        let seg = segment(Plane::Clinical, 5, 12);
        let mut out = Vec::new();
        put_segment(&mut out, &seg);
        out[4] = 99; // the plane tag is the first byte of the segment, after the u32 length
        let (taken, rest) = take_section(&out).unwrap().unwrap();
        assert!(rest.is_empty(), "an unknown section is consumed whole, by its length");
        match taken {
            TakenSection::Unknown(u) => {
                assert_eq!((u.plane_tag, u.index, u.record_count), (99, 5, 12));
            }
            other => panic!("expected Unknown, got {other:?}"),
        }
    }

    /// A torn append yields `Ok(None)` — "nothing complete here" — not an error. This is
    /// the property that makes an append-only medium safe to write in place: a crash mid
    /// append costs the last increment and nothing else.
    #[test]
    fn a_torn_tail_reports_incomplete_rather_than_corrupt() {
        let seg = segment(Plane::Clinical, 1, 3);
        let mut out = Vec::new();
        put_segment(&mut out, &seg);
        for cut in [0usize, 1, 3, 4, 10, out.len() - 1] {
            assert_eq!(
                take_section(&out[..cut]).expect("a short tail is not an error").is_none(),
                true,
                "a tail cut at {cut} must read as incomplete, never as corrupt"
            );
        }
    }

    /// A length prefix larger than the cap is CORRUPTION, not a torn tail. The two send an
    /// operator to different places — "your last backup was interrupted, run it again"
    /// versus "this medium is damaged" — so they must never collapse into one verdict.
    #[test]
    fn an_absurd_section_length_is_corruption_not_a_torn_tail() {
        let mut out = Vec::new();
        put_segment(&mut out, &segment(Plane::Node, 0, 1));
        out[..4].copy_from_slice(&u32::MAX.to_be_bytes());
        let err = take_section(&out).expect_err("must be an error, not Ok(None)");
        assert!(err.to_string().contains("cap"), "must name the cap: {err}");
    }
```

- [ ] **Step 2: Run and verify they fail**

Run: `cargo test -p cairn-medium --lib segment`
Expected: FAIL to compile — `Plane`, `Segment`, `put_segment`, `take_section`, `TakenSection` do not exist.

- [ ] **Step 3: Implement**

Add to `segment.rs`:

```rust
/// Upper bound on one section. A section holds ONE capture increment, which slice 2b
/// bounds by a batch limit; 256 MiB is generous for that and still caps a corrupt length
/// prefix. Note the real protection against a bogus prefix is that decoding works over an
/// in-memory slice and refuses a length exceeding what remains — this cap is what
/// separates "damaged medium" from "interrupted backup" (see `take_section`).
const MAX_SECTION_BYTES: usize = 256 * 1024 * 1024;

/// Which plane a segment's records belong to. The two planes share ONE record shape and
/// ONE codec; the tag is how a reader knows which door the records are destined for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Plane {
    /// `node_event` — enrolments, pairings, supersedes. Records carry no custody.
    Node,
    /// `event_log` — the clinical, demographic, identity, registration and erasure streams.
    Clinical,
}

impl Plane {
    pub fn tag(self) -> u8 {
        match self {
            Plane::Node => 1,
            Plane::Clinical => 2,
        }
    }

    /// `None` for a tag this build does not know — the caller reports it as an
    /// [`UnknownSegment`] rather than skipping it.
    pub fn from_tag(t: u8) -> Option<Plane> {
        match t {
            1 => Some(Plane::Node),
            2 => Some(Plane::Clinical),
            _ => None,
        }
    }

    /// The stable string used inside a segment attestation's signed payload.
    pub fn label(self) -> &'static str {
        match self {
            Plane::Node => "node",
            Plane::Clinical => "clinical",
        }
    }
}

/// One append increment: a run of records, tagged with its plane, positioned in the
/// chain, and (normally) signed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    pub plane: Plane,
    /// Position in the medium's single chain, in file order, from 0. NOT per-plane: one
    /// chain over the whole medium detects a reordering or a splice ACROSS planes, which
    /// two independent chains could not.
    pub index: u32,
    /// The preceding segment's commitment; empty for index 0.
    pub prev_commitment: String,
    /// Which node wrote this segment. Empty before enrolment (a node with no identity to
    /// name). Present even when unsigned — that is what closes the operator-typo footgun.
    pub self_node_id_hex: String,
    /// The signed `node.segment_attested` bytes, or `None` when the signing key was not
    /// available at capture. An unavailable key never BLOCKS a backup; it travels flagged.
    pub attestation: Option<Vec<u8>>,
    pub records: Vec<MediumRecord>,
}

/// A segment whose plane tag this build does not recognise. Reported, never skipped: its
/// header layout is fixed regardless of plane, so we can still say how much we could not
/// read — and NAMING what was not understood is the difference between honest degradation
/// and a medium that parses cleanly while missing a plane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownSegment {
    pub plane_tag: u8,
    pub index: u32,
    pub record_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TakenSection {
    Known(Segment),
    Unknown(UnknownSegment),
}

/// Encode one segment as a length-prefixed section. Pure.
///
/// The outer `[u32 len]` is what makes a torn append detectable without parsing the
/// segment: a reader that has fewer than `len` bytes left knows the append was cut short,
/// and stops cleanly at the last complete section.
pub(crate) fn put_segment(out: &mut Vec<u8>, seg: &Segment) {
    let mut body = Vec::new();
    body.push(seg.plane.tag());
    body.extend_from_slice(&seg.index.to_be_bytes());
    put_chunk(&mut body, seg.prev_commitment.as_bytes());
    put_chunk(&mut body, seg.self_node_id_hex.as_bytes());
    put_chunk(&mut body, seg.attestation.as_deref().unwrap_or(&[]));
    body.extend_from_slice(&(seg.records.len() as u32).to_be_bytes());
    for r in &seg.records {
        put_record(&mut body, r);
    }
    debug_assert!(body.len() <= MAX_SECTION_BYTES, "section exceeds the medium cap");
    out.extend_from_slice(&(body.len() as u32).to_be_bytes());
    out.extend_from_slice(&body);
}

/// Read one section.
///
/// Three outcomes, deliberately distinct:
///   - `Ok(Some(..))` — a complete section, known or unknown plane;
///   - `Ok(None)` — a TORN TAIL: fewer bytes remain than the section claims, which is what
///     an interrupted append looks like. The caller keeps everything before it and flags
///     the tail. Remedy: run the backup again.
///   - `Err(..)` — CORRUPTION: a length prefix beyond the cap, or a malformed body. The
///     remedy is different ("this medium is damaged"), so the verdicts never collapse.
pub(crate) fn take_section(rest: &[u8]) -> Result<Option<(TakenSection, &[u8])>, BackupError> {
    if rest.len() < 4 {
        return Ok(None); // not even a complete length prefix — torn
    }
    let len = u32::from_be_bytes([rest[0], rest[1], rest[2], rest[3]]) as usize;
    if len > MAX_SECTION_BYTES {
        return Err(BackupError::Decode(format!(
            "medium section length {len} exceeds the {MAX_SECTION_BYTES}-byte cap — the \
             medium is damaged (an INTERRUPTED backup reads as a short tail, not as this)"
        )));
    }
    if rest.len() < 4 + len {
        return Ok(None); // the section is complete on no copy of this file — torn
    }
    let (body, tail) = (&rest[4..4 + len], &rest[4 + len..]);
    let (&plane_tag, b) = body
        .split_first()
        .ok_or_else(|| BackupError::Decode("empty medium section: no plane tag".into()))?;
    if b.len() < 4 {
        return Err(BackupError::Decode(
            "medium section truncated: no segment index after the plane tag".into(),
        ));
    }
    let (idx, b) = b.split_at(4);
    let index = u32::from_be_bytes(idx.try_into().expect("4 bytes"));
    let (prev, b) = take_chunk(b)?;
    let (self_id, b) = take_chunk(b)?;
    let (att, b) = take_chunk(b)?;
    if b.len() < 4 {
        return Err(BackupError::Decode(
            "medium section truncated: no record count".into(),
        ));
    }
    let (count_bytes, mut b) = b.split_at(4);
    let record_count = u32::from_be_bytes(count_bytes.try_into().expect("4 bytes"));

    let Some(plane) = Plane::from_tag(plane_tag) else {
        // NAMED, never skipped. We consumed the section by its length, so parsing
        // continues past it — but the caller is told exactly what it did not understand.
        return Ok(Some((
            TakenSection::Unknown(UnknownSegment { plane_tag, index, record_count }),
            tail,
        )));
    };

    let mut records = Vec::with_capacity(record_count as usize);
    for _ in 0..record_count {
        let (r, next) = take_record(b)?;
        records.push(r);
        b = next;
    }
    if !b.is_empty() {
        return Err(BackupError::Decode(format!(
            "medium section has {} trailing byte(s) after its {record_count} record(s)",
            b.len()
        )));
    }
    let to_string = |v: &[u8], what: &str| -> Result<String, BackupError> {
        std::str::from_utf8(v)
            .map(str::to_string)
            .map_err(|_| BackupError::Decode(format!("segment {what} is not UTF-8")))
    };
    Ok(Some((
        TakenSection::Known(Segment {
            plane,
            index,
            prev_commitment: to_string(prev, "prev_commitment")?,
            self_node_id_hex: to_string(self_id, "self_node_id_hex")?,
            attestation: (!att.is_empty()).then(|| att.to_vec()),
            records,
        }),
        tail,
    )))
}
```

Re-export in `lib.rs`: `pub use segment::{MediumRecord, Plane, Segment, UnknownSegment};`.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p cairn-medium --lib segment`
Expected: all PASS (Task 4's five plus these five).

- [ ] **Step 5: Mutation-check the torn-tail/corruption split**

Change `take_section`'s `if rest.len() < 4 + len { return Ok(None) }` to return an `Err` instead. `a_torn_tail_reports_incomplete_rather_than_corrupt` must fail. Revert. Then remove the `len > MAX_SECTION_BYTES` arm; `an_absurd_section_length_is_corruption_not_a_torn_tail` must fail. Revert.

- [ ] **Step 6: Commit**

```bash
cargo fmt && git add -A
git commit -m "feat(#500): Plane, Segment, and the length-prefixed section framing

The outer length prefix is what makes an append-only medium safe to write in place: a
reader with fewer bytes left than a section claims knows the append was interrupted and
stops at the last complete section. That verdict is kept DISTINCT from corruption — an
over-cap length is damage, a short tail is an interrupted backup, and the two send an
operator to different places.

An unrecognised plane tag is reported with its index and record count, never skipped:
a medium that parses cleanly while missing a plane is #500's failure shape one layer
down."
```

---

## Task 6: The segment attestation and its commitment

**Files:**
- Modify: `crates/cairn-medium/src/segment.rs`, `crates/cairn-medium/src/marker.rs`, `crates/cairn-medium/src/lib.rs`

**Interfaces:**
- Consumes: Task 5's `Segment`, `Plane`; `cairn_event::{sign, verify_self_described, event_address, EventBody, Hlc, ClockGrade, SigningKey, NIL_PATIENT}`.
- Produces:
  - `pub const SEGMENT_ATTEST_TYPE: &str = "node.segment_attested"`
  - `pub fn segment_commitment(records: &[MediumRecord]) -> String`
  - `pub fn build_segment_attestation(sk: &SigningKey, key_id: &str, self_node_id_hex: &str, plane: Plane, index: u32, prev_commitment: &str, records: &[MediumRecord]) -> Vec<u8>`
  - `pub fn verify_segment_attestation(seg: &Segment) -> Option<String>` — the attested self node-id, or `None` (fail closed)
  - `marker::commitment_over(items: &[&[u8]]) -> String` (`pub(crate)`), which `event_set_commitment` now delegates to

- [ ] **Step 1: Write the failing tests**

```rust
    /// Shorthand for this module's tests: `n` salted records under one signed segment.
    /// Delegates to `tests_support::signed` (added in Step 3b) rather than building its own
    /// segment — ONE fixture builder for the whole crate, because every attestation
    /// assertion below rests on exactly what it produces.
    fn signed_segment(sk: &SigningKey, self_id: &str, plane: Plane, index: u32, prev: &str, n: usize) -> Segment {
        let records = (0..n).map(|i| tests_support::salted_record(1, i as u8)).collect();
        tests_support::signed(sk, self_id, plane, index, prev, records)
    }

    #[test]
    fn a_signed_segment_verifies_and_names_its_node() {
        let sk = crate::testkit::sk();
        let seg = signed_segment(&sk, "abcd", Plane::Clinical, 0, "", 3);
        assert_eq!(verify_segment_attestation(&seg), Some("abcd".to_string()));
    }

    /// The commitment is order-independent: frame reordering is harmless under set-union,
    /// so it must not invalidate an attestation.
    #[test]
    fn the_commitment_is_order_independent() {
        let a = record(0b001);
        let b = record(0b100);
        assert_eq!(
            segment_commitment(&[a.clone(), b.clone()]),
            segment_commitment(&[b, a]),
            "reordering records must not change the commitment"
        );
    }

    /// Adding, removing or altering ONE record breaks the attestation.
    #[test]
    fn altering_a_record_breaks_the_attestation() {
        let sk = crate::testkit::sk();
        let mut seg = signed_segment(&sk, "abcd", Plane::Node, 0, "", 2);
        seg.records[0].signed_bytes[0] ^= 0xff;
        assert_eq!(verify_segment_attestation(&seg), None, "must fail closed");

        let mut short = signed_segment(&sk, "abcd", Plane::Node, 0, "", 2);
        short.records.pop();
        assert_eq!(verify_segment_attestation(&short), None);
    }

    /// The attestation binds the segment's POSITION as well as its contents: replaying a
    /// genuine segment at another index, or under another plane tag, fails.
    #[test]
    fn the_attestation_binds_plane_index_and_predecessor() {
        let sk = crate::testkit::sk();
        let good = signed_segment(&sk, "abcd", Plane::Clinical, 4, "cafe", 2);
        assert!(verify_segment_attestation(&good).is_some());

        for mutated in [
            Segment { index: 5, ..good.clone() },
            Segment { plane: Plane::Node, ..good.clone() },
            Segment { prev_commitment: "f00d".into(), ..good.clone() },
        ] {
            assert_eq!(
                verify_segment_attestation(&mutated),
                None,
                "a genuine attestation must not validate a segment moved in the chain"
            );
        }
    }

    /// An unsigned segment yields no attested id — never a wrong one. Fail closed.
    #[test]
    fn an_unsigned_segment_attests_nothing() {
        let seg = Segment { attestation: None, ..segment(Plane::Clinical, 0, 1) };
        assert_eq!(verify_segment_attestation(&seg), None);
    }

    /// A tampered attestation withholds the id rather than misdirecting it. The attacker
    /// holds no private key, so a WRONG-but-valid attestation cannot be forged — the only
    /// achievable outcome is withholding, which fails closed.
    #[test]
    fn a_tampered_attestation_fails_closed() {
        let sk = crate::testkit::sk();
        let mut seg = signed_segment(&sk, "abcd", Plane::Clinical, 0, "", 1);
        let att = seg.attestation.as_mut().unwrap();
        let last = att.len() - 1;
        att[last] ^= 0x01;
        assert_eq!(verify_segment_attestation(&seg), None);
    }

    /// `event_set_commitment` keeps its exact CAIRNB2 value after being refactored to
    /// share `commitment_over` with `segment_commitment`. A changed value would
    /// invalidate every existing signed medium in the field.
    #[test]
    fn event_set_commitment_is_unchanged_by_the_shared_helper() {
        let sk = crate::testkit::sk();
        let e1 = crate::testkit::enroll(&sk, "a");
        let e2 = crate::testkit::enroll(&sk, "b");
        // Pinned by construction: the commitment of a one-event set is the multihash of
        // that event's own address, which we can compute here independently.
        let one = crate::marker::event_set_commitment(&[e1.clone()]);
        let expected = hex::encode(cairn_event::event_address(&cairn_event::event_address(&e1)));
        assert_eq!(one, expected, "the CAIRNB2 commitment must not change");
        assert_ne!(one, crate::marker::event_set_commitment(&[e1, e2]));
    }
```

- [ ] **Step 2: Run and verify they fail**

Run: `cargo test -p cairn-medium --lib segment`
Expected: FAIL to compile — `build_segment_attestation`, `verify_segment_attestation`, `segment_commitment` do not exist.

- [ ] **Step 3: Extract the shared commitment helper in `marker.rs`**

Replace `event_set_commitment`'s body so both commitments share one implementation, and its documented value does not change:

```rust
/// Hash a set of byte strings, order-independently: content-address each, sort, concatenate,
/// hash the concatenation. Pure.
///
/// Shared by [`event_set_commitment`] (CAIRNB2's whole-set bind) and
/// `segment::segment_commitment` (CAIRNB3's per-segment bind) so there is ONE definition of
/// what "a commitment over these bytes" means. The sort is what makes it order-independent:
/// frame reordering is harmless under set-union, so it must not invalidate a signature.
pub(crate) fn commitment_over(items: &[&[u8]]) -> String {
    let mut addresses: Vec<Vec<u8>> = items.iter().map(|e| event_address(e)).collect();
    addresses.sort();
    hex::encode(event_address(&addresses.concat()))
}

/// A deterministic, order-independent commitment to a medium's event SET.
/// (…keep the existing doc comment verbatim…)
pub fn event_set_commitment(events: &[Vec<u8>]) -> String {
    let refs: Vec<&[u8]> = events.iter().map(Vec::as_slice).collect();
    commitment_over(&refs)
}
```

- [ ] **Step 4: Implement the attestation in `segment.rs`**

```rust
/// Event type of the in-container segment attestation. Like `node.self_attested` it NEVER
/// enters `node_event`, never syncs and is never registered in the in-DB twin registry —
/// it lives in the backup container only, which is what lets it record a local
/// self-distinction that set-union convergence would otherwise erase.
pub const SEGMENT_ATTEST_TYPE: &str = "node.segment_attested";

/// Commitment over a segment's records — over their `signed_bytes` only, since that is the
/// event; the sidecar fields (attestation, key, DEK) are custody that a legitimate
/// re-capture may re-wrap. Order-independent, sharing `marker::commitment_over`.
pub fn segment_commitment(records: &[MediumRecord]) -> String {
    let refs: Vec<&[u8]> = records.iter().map(|r| r.signed_bytes.as_slice()).collect();
    crate::marker::commitment_over(&refs)
}

/// Build the signed attestation for one segment.
///
/// NOT pure: it mints a fresh `event_id` (`Uuid::now_v7`), exactly as
/// [`crate::marker::build_self_attestation`] does, so two calls differ. Harmless — the
/// `event_id` is neither committed to nor checked on verify; the authority comes from the
/// signature plus the four binds in the payload.
pub fn build_segment_attestation(
    sk: &SigningKey,
    key_id: &str,
    self_node_id_hex: &str,
    plane: Plane,
    index: u32,
    prev_commitment: &str,
    records: &[MediumRecord],
) -> Vec<u8> {
    let body = EventBody {
        event_id: uuid::Uuid::now_v7().to_string(),
        patient_id: cairn_event::NIL_PATIENT.into(),
        event_type: SEGMENT_ATTEST_TYPE.into(),
        schema_version: "node/1".into(),
        // Never ordered against anything, so a fixed 0/0 HLC — as the self-attestation does.
        hlc: Hlc { wall: 0, counter: 0, node_origin: self_node_id_hex.into() },
        t_effective: None,
        signer_key_id: key_id.into(),
        contributors: serde_json::json!([{"actor_id": key_id, "role": "recorded"}]),
        payload: serde_json::json!({
            "self_node_id_hex": self_node_id_hex,
            "plane": plane.label(),
            "segment_index": index,
            "record_count": records.len(),
            "segment_commitment": segment_commitment(records),
            "prev_commitment": prev_commitment,
        }),
        attachments: vec![],
        plaintext_twin: None,
        clock_grade: cairn_event::ClockGrade::SelfAsserted,
        safety: None,
    };
    sign(&body, sk).expect("segment-attestation signing").signed_bytes
}

/// Verify one segment's attestation against the segment it sits in. Returns the attested
/// `self_node_id_hex` IFF every bind holds, else `None`.
///
/// Fail closed, in the same asymmetry the CAIRNB2 marker has: an attacker holds no private
/// key, so tampering can only WITHHOLD the identification (falling back to a manual
/// choice), never misdirect it. The four binds:
///   - the attestation's own signature verifies and it is a `node.segment_attested`;
///   - its `segment_commitment` matches THIS segment's records;
///   - its `plane`, `segment_index` and `record_count` match this segment's position;
///   - its `prev_commitment` matches this segment's — so a genuine segment replayed
///     elsewhere in the chain fails.
///
/// The remaining bind — that the named node has a genesis on THIS medium, signed by the
/// same key — needs the whole medium and lives in `verify::chain_report`.
pub fn verify_segment_attestation(seg: &Segment) -> Option<String> {
    let bytes = seg.attestation.as_deref()?;
    let body = verify_self_described(bytes).ok()?;
    if body.event_type != SEGMENT_ATTEST_TYPE {
        return None;
    }
    let p = &body.payload;
    let matches = p.get("segment_commitment")?.as_str()? == segment_commitment(&seg.records)
        && p.get("plane")?.as_str()? == seg.plane.label()
        && p.get("segment_index")?.as_u64()? == u64::from(seg.index)
        && p.get("record_count")?.as_u64()? == seg.records.len() as u64
        && p.get("prev_commitment")?.as_str()? == seg.prev_commitment;
    if !matches {
        return None;
    }
    Some(p.get("self_node_id_hex")?.as_str()?.to_ascii_lowercase())
}
```

Add the imports `use cairn_event::{sign, verify_self_described, EventBody, Hlc, SigningKey};` at the top of `segment.rs`, and re-export `SEGMENT_ATTEST_TYPE`, `segment_commitment`, `build_segment_attestation`, `verify_segment_attestation` from `lib.rs`.

Make `testkit`'s `sk()` and `enroll()` `pub(crate)` so `segment.rs`'s tests can use them.

- [ ] **Step 3b: Add the crate-wide fixture builder**

`container.rs` (Task 7) and `verify.rs` (Task 8) both build signed media. They must use the
**same** builder as `segment.rs`'s own tests: three private copies would be three fixtures
whose meanings drift, and every attestation assertion in this crate rests on what this
produces. Add to `segment.rs`, above its `mod tests`:

```rust
/// Shared test fixtures for every module in this crate.
#[cfg(test)]
pub(crate) mod tests_support {
    use super::*;
    use cairn_event::SigningKey;

    /// Runtime-derived record bytes. `salt` distinguishes one fixture chain from another.
    ///
    /// **It is load-bearing, not decoration.** Without it every fixture chain is
    /// byte-identical, so a segment "spliced from another medium" would carry an identical
    /// predecessor commitment and Task 8's splice test would pass while asserting nothing.
    /// NEVER a literal byte array — house rule 6 (#146).
    pub(crate) fn salted_record(salt: u8, n: u8) -> MediumRecord {
        let mk = |seed: u8, len: usize| -> Vec<u8> {
            (0..len)
                .map(|i| {
                    seed.wrapping_add(salt)
                        .wrapping_mul(n.wrapping_add(1))
                        .wrapping_add(i as u8)
                })
                .collect()
        };
        MediumRecord {
            signed_bytes: mk(1, 40),
            attestation: Some(mk(2, 16)),
            attester_key: None,
            dek_wrapped: Some(mk(4, 48)),
            source_seq: i64::from(n) + 1,
        }
    }

    /// One signed segment over `records`, correctly positioned in a chain.
    pub(crate) fn signed(
        sk: &SigningKey,
        self_id: &str,
        plane: Plane,
        index: u32,
        prev: &str,
        records: Vec<MediumRecord>,
    ) -> Segment {
        let kid = hex::encode(sk.verifying_key().to_bytes());
        let attestation =
            build_segment_attestation(sk, &kid, self_id, plane, index, prev, &records);
        Segment {
            plane,
            index,
            prev_commitment: prev.to_string(),
            self_node_id_hex: self_id.to_string(),
            attestation: Some(attestation),
            records,
        }
    }
}
```

**Note the fixture's records are not validly-signed events** — `salted_record` produces
arbitrary bytes. That is correct for Tasks 6–7, which test framing and the attestation
binds, neither of which verifies an Ed25519 signature. Task 8's `verify_records` test needs
genuinely signed records and builds them from `testkit::enroll` instead.

- [ ] **Step 4b: Confirm the new event type needs no DB registration**

Run: `grep -rn "segment_attested\|self_attested" db/`
Expected: **no matches for either.** Both live in the backup container only, so — unlike a real event type — this adds nothing to `twin_registry.rs`, `db/tests/034`, or the `cairn_projection_apply` counts. If `self_attested` ever does appear in `db/`, stop: the assumption behind this task has changed.

- [ ] **Step 5: Run the tests**

Run: `cargo test -p cairn-medium --all-targets --no-fail-fast`
Expected: all PASS — Tasks 4–5's ten, these seven, and the 19 moved CAIRNB2 tests **including** the six `signed_attestation_*` ones, which prove `commitment_over` did not change `event_set_commitment`'s value.

- [ ] **Step 6: Mutation-check the binds**

Delete each of the five conjuncts in `verify_segment_attestation`'s `matches` in turn. Each deletion must turn at least one test red:
`segment_commitment` → `altering_a_record_breaks_the_attestation`; `plane`, `segment_index`, `prev_commitment` → `the_attestation_binds_plane_index_and_predecessor`; `record_count` → `altering_a_record_breaks_the_attestation`'s `short` case. **A conjunct no test kills is a conjunct that is not doing anything** — add the test rather than removing the check.

- [ ] **Step 7: Commit**

```bash
cargo fmt && git add -A
git commit -m "feat(#500): the segment attestation — the marker, made appendable

CAIRNB2's head marker commits to the whole sorted event set, so any append needs it
re-signed and rewriting it shifts every following byte. The segment attestation is the
same object made incremental: it names this node AND binds the segment's contents,
plane, position and predecessor, so appending costs one signature and a genuine segment
replayed elsewhere in the chain fails.

event_set_commitment now delegates to a shared commitment_over and is pinned to its
exact prior value — a change there would invalidate every signed medium in the field.

node.segment_attested lives in the container only, like node.self_attested: it never
enters node_event, never syncs, and registers nothing in the in-DB twin registry."
```

---

## Task 7: The `CAIRNB3` container — serialize, parse, dispatch

**Files:**
- Modify: `crates/cairn-medium/src/container.rs`, `crates/cairn-medium/src/lib.rs`

**Interfaces:**
- Consumes: Task 5's `put_segment`, `take_section`, `TakenSection`.
- Produces:
  - `pub const MEDIUM_MAGIC_V3: &[u8] = b"CAIRNB3\n"`
  - `pub struct MediumV3 { pub segments: Vec<Segment>, pub unknown: Vec<UnknownSegment>, pub truncated_tail: bool }`
  - `pub enum MediumImage { Legacy(Container), V3(MediumV3) }`
  - `pub fn serialize_v3(segments: &[Segment]) -> Vec<u8>`
  - `pub fn append_segment(medium: &mut Vec<u8>, seg: &Segment)`
  - `pub fn parse_any(bytes: &[u8]) -> Result<MediumImage, BackupError>`

`parse_container` and `serialize_container` are **not touched** — CAIRNB1/B2 keep their exact code path, which is the strongest available statement that existing media are unaffected.

- [ ] **Step 1: Write the failing tests**

Add to `container.rs`'s `mod tests`:

```rust
    #[test]
    fn a_v3_medium_roundtrips_both_planes() {
        let sk = crate::testkit::sk();
        let node = crate::segment::tests_support::signed(&sk, "abcd", Plane::Node, 0, "", 2);
        let clin = crate::segment::tests_support::signed(
            &sk, "abcd", Plane::Clinical, 1, &segment_commitment(&node.records), 3);
        let bytes = serialize_v3(&[node.clone(), clin.clone()]);
        match parse_any(&bytes).unwrap() {
            MediumImage::V3(m) => {
                assert_eq!(m.segments, vec![node, clin]);
                assert!(m.unknown.is_empty());
                assert!(!m.truncated_tail);
            }
            MediumImage::Legacy(_) => panic!("CAIRNB3 magic must not parse as legacy"),
        }
    }

    /// Appending is byte-wise: the existing image is untouched and the new section lands
    /// at the end. This is the property that makes capture O(new records).
    #[test]
    fn appending_leaves_the_existing_bytes_untouched() {
        let sk = crate::testkit::sk();
        let first = crate::segment::tests_support::signed(&sk, "abcd", Plane::Node, 0, "", 1);
        let mut image = serialize_v3(&[first.clone()]);
        let before = image.clone();
        let second = crate::segment::tests_support::signed(
            &sk, "abcd", Plane::Clinical, 1, &segment_commitment(&first.records), 1);
        append_segment(&mut image, &second);
        assert_eq!(&image[..before.len()], &before[..], "an append must not rewrite a byte");
        match parse_any(&image).unwrap() {
            MediumImage::V3(m) => assert_eq!(m.segments.len(), 2),
            other => panic!("{other:?}"),
        }
    }

    /// A torn append yields every complete segment before it, plus the flag. Nothing
    /// earlier is lost, and the loss that did occur is visible.
    #[test]
    fn a_torn_append_yields_the_complete_prefix_and_says_so() {
        let sk = crate::testkit::sk();
        let a = crate::segment::tests_support::signed(&sk, "abcd", Plane::Node, 0, "", 1);
        let b = crate::segment::tests_support::signed(
            &sk, "abcd", Plane::Clinical, 1, &segment_commitment(&a.records), 4);
        let mut image = serialize_v3(&[a.clone()]);
        let intact = image.len();
        append_segment(&mut image, &b);
        image.truncate(intact + 12); // a crash partway through the second section
        match parse_any(&image).unwrap() {
            MediumImage::V3(m) => {
                assert_eq!(m.segments, vec![a], "the complete prefix survives whole");
                assert!(m.truncated_tail, "and the torn tail is REPORTED, never silent");
            }
            other => panic!("{other:?}"),
        }
    }

    /// An unknown plane is collected and named while parsing continues past it.
    #[test]
    fn an_unknown_plane_is_collected_and_parsing_continues() {
        let sk = crate::testkit::sk();
        let a = crate::segment::tests_support::signed(&sk, "abcd", Plane::Node, 0, "", 1);
        let b = crate::segment::tests_support::signed(
            &sk, "abcd", Plane::Clinical, 1, &segment_commitment(&a.records), 2);
        let mut image = serialize_v3(&[a.clone(), b.clone()]);
        // Corrupt the FIRST section's plane tag: 8 magic bytes + 4 length bytes.
        image[crate::container::MEDIUM_MAGIC_V3.len() + 4] = 77;
        match parse_any(&image).unwrap() {
            MediumImage::V3(m) => {
                assert_eq!(m.segments, vec![b], "the readable segment still parses");
                assert_eq!(m.unknown.len(), 1, "and the unreadable one is NAMED");
                assert_eq!(m.unknown[0].plane_tag, 77);
            }
            other => panic!("{other:?}"),
        }
    }

    /// CAIRNB1 and CAIRNB2 still dispatch to the legacy path, byte for byte.
    #[test]
    fn legacy_media_still_parse_as_legacy() {
        let sk = crate::testkit::sk();
        let events = vec![crate::testkit::enroll(&sk, "a")];
        let v2 = serialize_container(Some(&SelfMarker::Unsigned("abcd".into())), &events);
        match parse_any(&v2).unwrap() {
            MediumImage::Legacy(c) => {
                assert_eq!(c.events, events);
                assert_eq!(c.self_marker, Some(SelfMarker::Unsigned("abcd".into())));
            }
            other => panic!("a CAIRNB2 medium must not become a V3 image: {other:?}"),
        }
    }

    #[test]
    fn a_medium_with_no_recognised_magic_is_refused() {
        assert!(parse_any(b"NOTACAIRN\n").is_err());
    }
```

The fixture builder `segment::tests_support` already exists (Task 6, Step 3b). Use it —
do not add a second one. Task 7's tests call it as
`tests_support::signed(&sk, "abcd", Plane::Node, 0, "", vec![tests_support::salted_record(1, 0)])`,
with `use crate::segment::tests_support;` at the top of `container.rs`'s `mod tests`.

- [ ] **Step 2: Run and verify they fail**

Run: `cargo test -p cairn-medium --lib container`
Expected: FAIL to compile.

- [ ] **Step 3: Implement**

`container.rs` needs these at the top:

```rust
use crate::segment::{put_segment, take_section, Plane, Segment, TakenSection, UnknownSegment};
```

and, in its `mod tests`, `use crate::segment::{segment_commitment, tests_support};`.

Add to `container.rs`:

```rust
/// Magic for the append-only, two-plane medium (issue #500 slice 2a). Distinct from
/// CAIRNB1/CAIRNB2 so a reader never has to guess, and from the keystore's CAIRNK1 and the
/// local-state export's CAIRNL1 so the four artifacts can never be confused.
pub const MEDIUM_MAGIC_V3: &[u8] = b"CAIRNB3\n";

/// A parsed CAIRNB3 image.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediumV3 {
    /// Every complete, recognised segment, in file order.
    pub segments: Vec<Segment>,
    /// Segments whose plane tag this build does not recognise — NAMED, never skipped, so a
    /// consumer that needs completeness can refuse rather than silently restore a medium
    /// that is missing a plane.
    pub unknown: Vec<UnknownSegment>,
    /// The final section was cut short: an interrupted append. Everything before it is
    /// intact. Remedy: run the backup again; the watermark did not advance past the last
    /// verified segment, so the lost increment is re-captured.
    pub truncated_tail: bool,
}

/// Either revision of the format, as parsed. Legacy media keep their exact prior code path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MediumImage {
    /// CAIRNB1 / CAIRNB2 — a head marker plus bare event frames.
    Legacy(Container),
    /// CAIRNB3 — chained, plane-tagged segments.
    V3(MediumV3),
}

/// Serialize a fresh CAIRNB3 image. Pure.
pub fn serialize_v3(segments: &[Segment]) -> Vec<u8> {
    let mut out = Vec::from(MEDIUM_MAGIC_V3);
    for seg in segments {
        crate::segment::put_segment(&mut out, seg);
    }
    out
}

/// Append one segment to an existing CAIRNB3 image, in place.
///
/// Byte-wise append: nothing already in `medium` is read or rewritten, which is what makes
/// a capture cost O(new records). The caller writing this to disk owes the durability half
/// — `write` then `sync_all()` BEFORE advancing any health record, so health can only ever
/// under-claim (slice 2c owns that; this crate does no I/O).
pub fn append_segment(medium: &mut Vec<u8>, seg: &Segment) {
    crate::segment::put_segment(medium, seg);
}

/// Parse a medium of any revision, dispatching on its magic.
pub fn parse_any(bytes: &[u8]) -> Result<MediumImage, BackupError> {
    let Some(mut rest) = bytes.strip_prefix(MEDIUM_MAGIC_V3) else {
        // Not CAIRNB3 — hand it to the untouched legacy parser, which refuses anything
        // that is not CAIRNB1/CAIRNB2.
        return Ok(MediumImage::Legacy(parse_container(bytes)?));
    };
    let mut segments = Vec::new();
    let mut unknown = Vec::new();
    let mut truncated_tail = false;
    while !rest.is_empty() {
        match crate::segment::take_section(rest)? {
            None => {
                truncated_tail = true;
                break;
            }
            Some((taken, next)) => {
                match taken {
                    crate::segment::TakenSection::Known(s) => segments.push(s),
                    crate::segment::TakenSection::Unknown(u) => unknown.push(u),
                }
                rest = next;
            }
        }
    }
    Ok(MediumImage::V3(MediumV3 { segments, unknown, truncated_tail }))
}
```

- [ ] **Step 4: Run the tests**

Run: `cargo test -p cairn-medium --all-targets --no-fail-fast`
Expected: all PASS, including the 19 legacy tests.

- [ ] **Step 5: Mutation-check the two silence risks**

Set `truncated_tail` to a hard-coded `false` → `a_torn_append_yields_the_complete_prefix_and_says_so` must fail. Then make the `Unknown` arm `continue` without pushing → `an_unknown_plane_is_collected_and_parsing_continues` must fail. Both are the "silently missing" shape, so both must be killed by a test.

- [ ] **Step 6: Commit**

```bash
cargo fmt && git add -A
git commit -m "feat(#500): CAIRNB3 — the append-only container and its magic dispatch

parse_container and serialize_container are untouched: CAIRNB1/CAIRNB2 media keep their
exact prior code path, which is the strongest available statement that existing media
are unaffected. parse_any dispatches on magic.

append_segment writes bytes and reads none, so capture costs O(new records). A torn
append yields every complete segment before it plus a truncated_tail flag, and an
unrecognised plane is collected with its index and record count rather than skipped."
```

---

## Task 8: The chain pass, the watermark, and self-identification

**Files:**
- Modify: `crates/cairn-medium/src/verify.rs`, `crates/cairn-medium/src/lib.rs`

**Interfaces:**
- Consumes: Tasks 5–7.
- Produces:
  - `pub enum SegmentFault { AttestationInvalid { plane: Plane, index: u32 }, CommitmentMismatch { plane: Plane, index: u32 }, ChainBroken { plane: Plane, index: u32, expected: String, found: String }, SelfIdUnbound { plane: Plane, index: u32, self_node_id_hex: String } }`
  - `pub struct ChainReport { pub segments: usize, pub signed: usize, pub unsigned: usize, pub faults: Vec<SegmentFault>, pub verified_through: Option<usize> }` with `pub fn intact(&self) -> bool`
  - `pub fn chain_report(m: &MediumV3) -> ChainReport`
  - `pub fn watermark(m: &MediumV3, report: &ChainReport, plane: Plane) -> Option<i64>`
  - `pub fn self_id_from_chain(m: &MediumV3, report: &ChainReport) -> Option<String>`
  - `pub fn verify_records(m: &MediumV3) -> VerifyReport`

- [ ] **Step 1: Write the failing tests**

```rust
    #[test]
    fn a_well_formed_chain_verifies_through_its_last_segment() {
        let (m, _) = crate::testkit::chain_of(3);
        let r = chain_report(&m);
        assert!(r.intact(), "faults: {:?}", r.faults);
        assert_eq!(r.verified_through, Some(2));
        assert_eq!((r.segments, r.signed, r.unsigned), (3, 3, 0));
    }

    /// A break is located by plane AND index. "chain invalid" sends an operator nowhere.
    #[test]
    fn a_chain_break_is_located_not_merely_counted() {
        let (mut m, _) = crate::testkit::chain_of(4);
        m.segments[2].prev_commitment = "deadbeef".into();
        let r = chain_report(&m);
        assert!(!r.intact());
        assert_eq!(r.verified_through, Some(1), "verified through the last GOOD segment");
        assert!(
            r.faults.iter().any(|f| matches!(
                f,
                SegmentFault::AttestationInvalid { index: 2, .. }
                    | SegmentFault::ChainBroken { index: 2, .. }
            )),
            "the fault must name segment 2: {:?}",
            r.faults
        );
    }

    /// A genuine segment spliced from ANOTHER medium fails on its predecessor, even though
    /// its own signature and commitment are perfectly valid.
    #[test]
    fn a_spliced_segment_fails_on_its_predecessor() {
        let (mut mine, _) = crate::testkit::chain_of(2);
        let (theirs, _) = crate::testkit::chain_of(2);
        mine.segments[1] = theirs.segments[1].clone();
        let r = chain_report(&mine);
        assert!(!r.intact(), "a foreign segment must not validate here");
    }

    /// The watermark comes from the last VERIFIED segment, so a torn or broken tail costs
    /// exactly one increment: its records are re-captured, never lost.
    #[test]
    fn the_watermark_ignores_everything_after_the_last_verified_segment() {
        let (mut m, seqs) = crate::testkit::chain_of(3);
        let good = watermark(&m, &chain_report(&m), Plane::Clinical);
        assert_eq!(good, Some(seqs.last().copied().unwrap()));
        m.segments[2].records[0].signed_bytes[0] ^= 0xff; // breaks segment 2
        let after = watermark(&m, &chain_report(&m), Plane::Clinical);
        assert!(after < good, "a broken tail must not advance the cursor: {after:?} vs {good:?}");
    }

    /// A plane with no verified segment has NO watermark — `None`, never `Some(0)`. Zero is
    /// a claim ("I hold everything up to seq 0"); the honest answer is "I do not know".
    #[test]
    fn a_plane_with_no_verified_segment_has_no_watermark() {
        let (m, _) = crate::testkit::chain_of(1); // clinical only
        assert_eq!(watermark(&m, &chain_report(&m), Plane::Node), None);
    }

    /// Self-identification takes the LAST verified attestation and binds the named id to a
    /// genesis present on this medium, signed by the same key.
    #[test]
    fn self_id_binds_the_named_node_to_a_genesis_on_this_medium() {
        let (m, _) = crate::testkit::chain_with_genesis();
        let r = chain_report(&m);
        assert!(self_id_from_chain(&m, &r).is_some());
    }

    /// A named node with no genesis on the medium yields NOTHING. Fail closed: the marker
    /// can be withheld, never turned into a wrong-but-valid identity.
    #[test]
    fn an_unbound_self_id_is_withheld_not_guessed() {
        let (mut m, _) = crate::testkit::chain_with_genesis();
        m.segments[0].records.clear(); // remove the genesis; the attestation now mismatches
        let r = chain_report(&m);
        assert_eq!(self_id_from_chain(&m, &r), None);
    }

    /// Every record's SIGNATURE is checked, not merely its commitment.
    ///
    /// This is a distinct property and it is easy to lose: the chain pass verifies
    /// attestations and commitments, and a commitment is over content ADDRESSES — which a
    /// tampered blob still has. So in a SIGNED segment tampering is caught twice (the
    /// address changes, so the commitment fails), but in an UNSIGNED segment there is no
    /// attestation at all, and without this pass nothing would check the bytes.
    #[test]
    fn a_tampered_record_is_caught_even_in_an_unsigned_segment() {
        let mut m = crate::testkit::unsigned_chain_of(2);
        assert!(chain_report(&m).intact(), "unsigned but well-formed");
        let clean = verify_records(&m);
        assert_eq!(clean.first_bad, None, "the fixture's records must verify to begin with");

        m.segments[1].records[0].signed_bytes[0] ^= 0xff;
        let report = verify_records(&m);
        assert_eq!(report.total, 2);
        assert_eq!(report.intact, 1);
        assert_eq!(report.first_bad, Some(1), "and it must NAME which record failed");
    }

    /// An all-unsigned medium identifies nobody, and says so without inventing a fault.
    #[test]
    fn an_unsigned_medium_identifies_nobody_and_is_not_a_fault() {
        let m = crate::testkit::unsigned_chain_of(2);
        let r = chain_report(&m);
        assert_eq!((r.signed, r.unsigned), (0, 2));
        assert!(r.intact(), "unsigned is not a FAULT — it is a declared limitation");
        assert_eq!(self_id_from_chain(&m, &r), None);
    }
```

Add these three fixtures to `testkit.rs`. Note `chain_of` takes a **salt**: two chains
built with different salts hold genuinely different records, which is what gives
`a_spliced_segment_fails_on_its_predecessor` something real to assert. With one shared
salt both chains would be byte-identical and the test would pass while proving nothing.

```rust
use crate::segment::{segment_commitment, tests_support, MediumRecord, Plane, Segment};
use crate::container::MediumV3;

/// `n` signed CLINICAL segments, correctly chained, with ascending `source_seq`.
/// Returns the medium and every seq it wrote, so a caller can assert on the watermark.
pub(crate) fn chain_of(n: usize, salt: u8) -> (MediumV3, Vec<i64>) {
    let sk = sk();
    let mut segments: Vec<Segment> = Vec::new();
    let mut seqs = Vec::new();
    let mut prev = String::new();
    for i in 0..n {
        let records = vec![tests_support::salted_record(salt, i as u8)];
        seqs.extend(records.iter().map(|r| r.source_seq));
        let seg = tests_support::signed(&sk, "abcd", Plane::Clinical, i as u32, &prev, records);
        prev = segment_commitment(&seg.records);
        segments.push(seg);
    }
    (MediumV3 { segments, unknown: vec![], truncated_tail: false }, seqs)
}

/// A medium whose segment 0 is a NODE-plane segment carrying a real `node.enrolled`, so
/// `self_id_from_chain`'s genesis bind has something to bind to; segment 1 is clinical.
/// The attested self id is the genesis's own content address — that is what a node-id IS.
pub(crate) fn chain_with_genesis() -> (MediumV3, cairn_event::SigningKey) {
    let sk = sk();
    let genesis = enroll(&sk, "a");
    let self_id = hex::encode(cairn_event::event_address(&genesis));
    let node_records = vec![MediumRecord {
        signed_bytes: genesis,
        attestation: None,
        attester_key: None,
        dek_wrapped: None,
        source_seq: 1,
    }];
    let s0 = tests_support::signed(&sk, &self_id, Plane::Node, 0, "", node_records);
    let prev = segment_commitment(&s0.records);
    let s1 = tests_support::signed(
        &sk,
        &self_id,
        Plane::Clinical,
        1,
        &prev,
        vec![tests_support::salted_record(9, 0)],
    );
    (MediumV3 { segments: vec![s0, s1], unknown: vec![], truncated_tail: false }, sk)
}

/// `n` clinical segments written with NO signing key available — correctly chained, and
/// carrying their self id, but not tamper-evident.
///
/// Its records hold GENUINELY SIGNED events (via `enroll`), unlike `salted_record`'s
/// arbitrary bytes. `verify_records` checks Ed25519 signatures, so a fixture built from
/// salted bytes would fail verification before any test tampered with it — and the test
/// would then pass for the wrong reason, proving nothing about tampering.
pub(crate) fn unsigned_chain_of(n: usize) -> MediumV3 {
    let sk = sk();
    let mut segments = Vec::new();
    let mut prev = String::new();
    for i in 0..n {
        let records = vec![MediumRecord {
            signed_bytes: enroll(&sk, &format!("node-{i}")),
            attestation: None,
            attester_key: None,
            dek_wrapped: None,
            source_seq: i as i64 + 1,
        }];
        let seg = Segment {
            plane: Plane::Clinical,
            index: i as u32,
            prev_commitment: prev.clone(),
            self_node_id_hex: "abcd".into(),
            attestation: None,
            records,
        };
        prev = segment_commitment(&seg.records);
        segments.push(seg);
    }
    MediumV3 { segments, unknown: vec![], truncated_tail: false }
}
```

Update the tests above to pass a salt: `chain_of(3, 1)` everywhere, except
`a_spliced_segment_fails_on_its_predecessor`, which builds `chain_of(2, 1)` and
`chain_of(2, 2)` — **different salts, or the test asserts nothing.**

- [ ] **Step 2: Run and verify they fail**

Run: `cargo test -p cairn-medium --lib verify`
Expected: FAIL to compile.

- [ ] **Step 3: Implement**

`verify.rs` needs these at the top:

```rust
use crate::container::MediumV3;
use crate::segment::{Plane, Segment};
```

Add:

```rust
/// One thing wrong with one segment. Every variant carries the segment's PLANE and INDEX:
/// "clinical segment 7 breaks the chain" sends an operator somewhere, "chain invalid"
/// does not — and the standing rule in this codebase is NAME, NEVER COUNT.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SegmentFault {
    /// The attestation is present but does not verify against this segment (tampered, or
    /// the segment was moved in the chain).
    AttestationInvalid { plane: Plane, index: u32 },
    /// The records do not hash to the attested commitment.
    CommitmentMismatch { plane: Plane, index: u32 },
    /// `prev_commitment` does not match the preceding segment's commitment.
    ChainBroken { plane: Plane, index: u32, expected: String, found: String },
    /// A signed segment names a node with no matching genesis on this medium.
    SelfIdUnbound { plane: Plane, index: u32, self_node_id_hex: String },
}

/// What a chain pass found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChainReport {
    pub segments: usize,
    pub signed: usize,
    /// Segments written without a signing key. NOT a fault: an unavailable key must never
    /// block a backup, so an unsigned segment travels flagged — it simply is not
    /// tamper-evident, and no caller may treat it as if it were.
    pub unsigned: usize,
    pub faults: Vec<SegmentFault>,
    /// Position (into `MediumV3::segments`) of the last segment verified with every
    /// preceding one. `None` when the first segment already fails. Everything at or before
    /// this is trustworthy; everything after it is not, which is what bounds the loss from
    /// a torn or damaged tail to exactly the increments after this point.
    pub verified_through: Option<usize>,
}

impl ChainReport {
    /// No faults. Unsigned segments do not make a medium un-intact.
    pub fn intact(&self) -> bool {
        self.faults.is_empty()
    }
}

/// Walk the chain once, in file order.
///
/// The walk STOPS advancing `verified_through` at the first fault: a chain is a chain, and
/// a segment after a break has no verified predecessor to hang from. Faults after that
/// point are still collected and reported, because "one break" and "the whole tail is
/// rubble" are different operator situations.
pub fn chain_report(m: &MediumV3) -> ChainReport {
    let mut faults = Vec::new();
    let mut signed = 0;
    let mut unsigned = 0;
    let mut verified_through = None;
    let mut expected_prev = String::new();
    let mut still_good = true;

    for (i, seg) in m.segments.iter().enumerate() {
        let mut ok = true;
        if seg.prev_commitment != expected_prev {
            faults.push(SegmentFault::ChainBroken {
                plane: seg.plane,
                index: seg.index,
                expected: expected_prev.clone(),
                found: seg.prev_commitment.clone(),
            });
            ok = false;
        }
        match &seg.attestation {
            None => unsigned += 1,
            Some(_) => {
                signed += 1;
                if crate::segment::verify_segment_attestation(seg).is_none() {
                    faults.push(SegmentFault::AttestationInvalid {
                        plane: seg.plane,
                        index: seg.index,
                    });
                    ok = false;
                }
            }
        }
        expected_prev = crate::segment::segment_commitment(&seg.records);
        if ok && still_good {
            verified_through = Some(i);
        } else {
            still_good = false;
        }
    }

    ChainReport { segments: m.segments.len(), signed, unsigned, faults, verified_through }
}

/// The highest `source_seq` this medium can be TRUSTED to hold for `plane`.
///
/// Derived from `verified_through`, never from the file's tail. That is the property that
/// makes a torn append cost exactly one increment: an unverifiable trailing segment does
/// not advance the cursor, so the next capture re-writes its records rather than skipping
/// past them.
///
/// `None` — never `Some(0)` — when no verified segment of that plane exists. Zero is a
/// CLAIM ("everything up to seq 0 is here"); the honest answer to "what do you hold?" when
/// nothing verified is "I do not know".
pub fn watermark(m: &MediumV3, report: &ChainReport, plane: Plane) -> Option<i64> {
    let through = report.verified_through?;
    m.segments[..=through]
        .iter()
        .filter(|s| s.plane == plane)
        .flat_map(|s| s.records.iter().map(|r| r.source_seq))
        .max()
}

/// Verify every record's SIGNATURE, across every segment, in file order.
///
/// Separate from [`chain_report`] because it answers a different question and one does not
/// imply the other. The chain pass checks attestations and COMMITMENTS, and a commitment is
/// taken over content addresses — which a tampered blob still has, just a different one. In
/// a signed segment that is enough (the commitment fails), but an UNSIGNED segment has no
/// attestation, so without this pass nothing checks its bytes at all.
///
/// Reuses [`verify_event`] unchanged, so a record on a medium faces exactly the check a
/// replicated event faces at the apply door: no second definition of "valid".
pub fn verify_records(m: &MediumV3) -> VerifyReport {
    let events: Vec<Vec<u8>> = m
        .segments
        .iter()
        .flat_map(|s| s.records.iter().map(|r| r.signed_bytes.clone()))
        .collect();
    verify_events(&events)
}

/// Which node this medium belongs to, from the LAST verified signed segment.
///
/// Two binds, mirroring the CAIRNB2 marker's: the attestation must verify against its own
/// segment (done in `chain_report`), and the node it names must have a genesis
/// (`node.enrolled`) present on THIS medium, signed by the SAME key that signed the
/// attestation. The second bind is what makes a foreign attestation unusable: only the
/// node that signed its own genesis could have signed this.
///
/// `None` on any doubt. Fail closed — a withheld identification falls back to an operator
/// choice, whereas a wrong one records an immutable supersede against the wrong node.
pub fn self_id_from_chain(m: &MediumV3, report: &ChainReport) -> Option<String> {
    let through = report.verified_through?;
    // Every node-plane record on the medium, as candidate genesis events.
    let node_events: Vec<Vec<u8>> = m
        .segments
        .iter()
        .filter(|s| s.plane == Plane::Node)
        .flat_map(|s| s.records.iter().map(|r| r.signed_bytes.clone()))
        .collect();
    let found = crate::marker::enrolls(&node_events);

    for seg in m.segments[..=through].iter().rev() {
        // Every arm is `continue`, never `?`: an unsigned or unverifiable segment means
        // "keep looking further back", not "give up on the whole medium". A `?` here would
        // let one unsigned tail segment hide a perfectly good identification beneath it.
        let Some(att) = seg.attestation.as_deref() else {
            continue;
        };
        let Some(id) = crate::segment::verify_segment_attestation(seg) else {
            continue;
        };
        let Ok(body) = cairn_event::verify_self_described(att) else {
            continue;
        };
        let attester = body.signer_key_id;
        if found
            .iter()
            .any(|(gid, genesis)| *gid == id && genesis.signer_key_id == attester)
        {
            return Some(id);
        }
    }
    None
}
```

Re-export the four items from `lib.rs`.

- [ ] **Step 4: Run the tests**

Run: `cargo test -p cairn-medium --all-targets --no-fail-fast`
Expected: everything PASS.

- [ ] **Step 5: Mutation-check the three assertions most likely to be vacuous**

1. `watermark`: replace `report.verified_through?` with `Some(m.segments.len() - 1)` → `the_watermark_ignores_everything_after_the_last_verified_segment` must fail.
2. `watermark`: return `Some(0)` instead of `None` when nothing verified → `a_plane_with_no_verified_segment_has_no_watermark` must fail.
3. `self_id_from_chain`: drop the `genesis.signer_key_id == attester` conjunct → `an_unbound_self_id_is_withheld_not_guessed` must fail.
4. `verify_records`: return `VerifyReport { total: 0, intact: 0, first_bad: None }` unconditionally → `a_tampered_record_is_caught_even_in_an_unsigned_segment` must fail. (If it does not, the chain pass is silently covering for it and the unsigned case is untested.)

- [ ] **Step 6: Commit**

```bash
cargo fmt && git add -A
git commit -m "feat(#500): the chain pass, the watermark, and self-identification

A fault names its plane and index — 'clinical segment 7 breaks the chain' sends an
operator somewhere, 'chain invalid' does not.

The watermark comes from verified_through, never from the file's tail, which is what
bounds the loss from a torn append to exactly one increment. A plane with no verified
segment has NO watermark: None, never Some(0), because zero is a claim and the honest
answer is 'I do not know'.

Self-identification takes the last verified attestation and binds the named node to a
genesis on this medium signed by the same key, failing closed on any doubt — the marker
can be withheld, never turned into a wrong-but-valid identity."
```

---

## Task 9: Documentation and the full gate

**Files:**
- Modify: `crates/cairn-medium/src/lib.rs` (the format invariants, in one place)
- Modify: `docs/HANDOVER.md`, `docs/ROADMAP.md`
- Modify: `docs/superpowers/specs/2026-08-31-dr-slice-2a-shared-two-plane-medium-design.md` (two corrections found while building)

- [ ] **Step 1: Write the format's invariants into `lib.rs`**

A reader arriving at this crate needs the *rules*, not a tour of the modules. Add to the crate docs:

```rust
//! # The invariants, in one place
//!
//! 1. **CAIRNB1 and CAIRNB2 are frozen.** They parse today exactly as they did before this
//!    crate existed, through untouched code. Media in the field are unaffected, forever.
//! 2. **CAIRNB3 is append-only.** `append_segment` writes bytes and reads none. Nothing
//!    already on a medium is ever rewritten.
//! 3. **Every segment is chained.** Its attestation binds its contents, its plane, its
//!    position and its predecessor's commitment. A genuine segment replayed elsewhere in a
//!    chain, or spliced from another medium, fails.
//! 4. **A torn tail is not corruption.** Fewer bytes than a section claims means an
//!    interrupted append: keep the complete prefix, flag the tail, re-capture. An over-cap
//!    length prefix IS corruption. The two verdicts never collapse, because they send an
//!    operator to different places.
//! 5. **Trust stops at `verified_through`.** The watermark is derived from it, never from
//!    the file's tail, which bounds the loss from any tail damage to one increment.
//! 6. **Nothing unrecognised is skipped in silence.** An unknown plane tag is reported with
//!    its index and record count; an unknown record flag bit is REFUSED. A medium that
//!    parses cleanly while missing a plane is the exact failure shape #500 is about.
//! 7. **Unsigned is a declared limitation, not a fault.** An unavailable signing key never
//!    blocks a backup. It travels flagged, and no caller may treat it as tamper-evident.
//! 8. **`None` is not zero.** A plane with no verified segment has no watermark. Zero is a
//!    claim; absence is the honest answer.
```

- [ ] **Step 2: Correct two things in the spec that building revealed**

Both are working-document corrections, not ADR edits, so they are made in place with a note:

1. §7 says a fault is reported "by segment index and plane" while implying per-plane numbering. The built chain is **one global chain in file order**, so `index` is the medium-wide position. Fix the wording and say why one chain: it detects a reorder or splice **across** planes, which two independent chains could not.
2. Add to §12's deferred list:

```markdown
- **Streaming parse.** `parse_any` reads a whole image into memory. The section framing is
  exactly what makes a streaming reader possible as a later, purely additive change, but a
  medium larger than RAM cannot be parsed today. **2b decides**, since `MediumTransport` is
  the first consumer that can meet one.
```

- [ ] **Step 3: Run the full local gate**

```bash
export CAIRN_ALLOW_DB_SKIP=1   # only if you are NOT running the DB-gated suites
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo doc --workspace --no-deps
cargo test --workspace --all-targets --no-fail-fast
(cd cairn-gui && cargo clippy --workspace --locked --all-targets -- -D warnings)
(cd extensions/cairn_pgx && cargo check --locked)
```

**Start the workspace test run in the background and do the docs pass while it runs** — a new workspace member touches `Cargo.lock`, which relinks ~134 test binaries, each drawing a one-time macOS Gatekeeper assessment. Budget hours. Do **not** pipe cargo to `tail`: it masks the exit status.

The `--locked` runs on the other two trees are the *only* gate that catches a stale sibling lockfile. If either fails with `cannot update the lock file … because --locked was passed`, go back to Task 2 Step 5.

- [ ] **Step 4: Update HANDOVER and ROADMAP**

ROADMAP gains a slice entry; HANDOVER's ⇒ NEXT gains the state of the programme. Both must say plainly:

- **2a closed nothing.** #500 is open; the medium still carries no clinical event.
- `dr_clinical_guarantee_gap.rs::medium_carries_the_federation_plane_and_no_clinical_event` still passes **as a pin on the defect**, and 2d inverts it.
- The next build is **2b** (the transport seam + the paged pull).
- The loose end named in spec §12: **custody must apply the same `erasure_shred_log` exclusion on both the medium path and the export path, or a shredded body comes back on restore. 2c decides which is authoritative.**

Prune both files back under 500 lines while you are there (HANDOVER is currently over).

- [ ] **Step 5: Commit and open the PR**

```bash
cargo fmt && git add -A
git commit -m "docs(#500): slice 2a — the format's invariants, and what is still broken"
git push -u origin design/500-dr-slice-2a-medium-format
gh pr create --base main \
  --title "DR slice 2a: the shared, two-plane, append-only backup medium (#500)" \
  --body-file - <<'BODY'
## What this does NOT do

**It does not fix [#500](https://github.com/cairn-ehr/cairn-ehr/issues/500).** After this
merges the backup medium still carries no clinical event: `backup.rs::read_event_set` still
reads `SELECT signed_bytes FROM node_event` and nothing else, and
`dr_clinical_guarantee_gap.rs::medium_carries_the_federation_plane_and_no_clinical_event`
still passes **as a pin on the defect**. A restored solo node still recovers who it peered
with and zero patients. Slice 2d inverts that test; this is slice 2a of five.

## What it builds

The container format every later piece reads and writes.

- **`crates/cairn-medium`** — today's `medium.rs` moved verbatim then split by
  responsibility, so `cairn-sync` can write the clinical plane onto the same medium
  `cairn-node` writes its federation plane to without depending on a node application. All
  12 existing call sites compile untouched; that is the extraction's proof (#503's pattern).
- **`CAIRNB3`** — an append-only revision. CAIRNB2's head marker commits to the whole
  sorted event set, so any append needs it re-signed and rewriting it shifts every following
  byte: a whole-file rewrite per backup, over a log that grows for the life of a clinic.
  CAIRNB3 has no head block. Each plane-tagged segment carries its own signed attestation
  naming this node and chained to its predecessor, so appending costs one signature.
- CAIRNB1/CAIRNB2 media parse through **untouched code**.

## Decisions worth a reviewer's attention

- **The marker and the segment attestation are one object.** Self-identification becomes
  "the last verified segment attestation", under the same binds the CAIRNB2 marker has —
  and the chain *narrows* the documented converged-peer splice, since a foreign segment must
  also match its predecessor.
- **A torn tail is not corruption.** Fewer bytes than a section claims means an interrupted
  append: keep the prefix, flag the tail, re-capture. An over-cap length is damage. The two
  verdicts never collapse, because they send an operator to different places.
- **`None` is not zero.** A plane with no verified segment has no watermark.
- **Nothing unrecognised is skipped in silence** — an unknown plane tag is named with its
  index and record count; an unknown record flag bit is refused.

## The loose end this slice names but does not close

Custody must apply the same `erasure_shred_log` exclusion on **both** the medium path and
the `CAIRNL1` export path, or a shredded body comes back on restore. 2a only makes both
expressible; **2c decides which is authoritative.** Spec §12 carries it so it cannot be lost.

Spec: `docs/superpowers/specs/2026-08-31-dr-slice-2a-shared-two-plane-medium-design.md`
Plan: `docs/superpowers/plans/2026-08-31-dr-slice-2a-shared-two-plane-medium.md`

Refs #500, #101, #512, #521

🤖 Generated with [Claude Code](https://claude.com/claude-code)
BODY
```

The PR body must open with what this does **not** do — that the medium still carries no clinical event and #500 stays open — before describing what it builds. The single most likely misreading of this branch is that the DR hole is closed.

---

## Deferred, each naming the slice that retires it

- **Writing real clinical records** onto a medium — no DB read exists in this crate by construction. → **2c**
- **The transport seam and the paged pull** (`MediumTransport`, the `EventsAfterSeq` batch limit) → **2b**
- **Streaming parse** for a medium larger than RAM — the section framing makes it additive → **2b**
- **The actor registry riding `CAIRNL1`** → **2d**
- **Health and scope honesty** in `backup-status.json` / `status` / `verify-backup` → **2c**
- **Which path owns custody**, and the shared `erasure_shred_log` exclusion both must apply → **2c**
- **Whether an unsigned segment should be restorable without operator confirmation** → **2d**
- **The superseding ADR** → **2e**
- **Custody newtypes** (`Secret32`/`PublicKey32`) → **[#511](https://github.com/cairn-ehr/cairn-ehr/issues/511), its own slice, after 2a and before 2c.** Deliberately not here: this crate holds zero `[u8; 32]`, and the migration's 83 sites across four crates would cost 2a its only proof — that every existing call site compiled untouched.
