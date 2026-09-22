# Registration/search funnel UI — slice 2b: the live ports

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development
> (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use
> checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give the funnel's two ports a node-backed implementation, so that the rules slice 2a
made executable finally act on a real record — and so that a deterministic floor refusal reaches
the clerk as a *refusal*, not as an outage they are invited to retry.

**Architecture:** A new crate, `cairn-gui-live`, in the `cairn-gui` workspace. It holds one type,
`LiveData`, implementing `cairn_gui_data::port::PatientSearch` and `PatientRegistration` over a
`tokio_postgres::Client`, delegating to `cairn_node::patient::search::search_patients` and
`cairn_node::patient::register::register_patient`. It cannot live in `cairn-gui-data` — that
crate's manifest states, deliberately, that it pulls no database driver — and it cannot live in
`/crates`, because a root crate implementing a `cairn-gui` trait would invert the one-way
dependency ADR-0021 fixes. `DataError` gains the `Refused` variant #648 asks for, and the rule
that decides it is a pure function with its own tests.

**Tech Stack:** Rust (`cairn-gui` workspace, edition 2021, rust-version 1.96), `tokio-postgres`,
`tokio::sync::Mutex`, PostgreSQL 18 with the `cairn_pgx` extension.

**Spec:** `docs/superpowers/specs/2026-09-20-registration-search-funnel-ui-design.md` — read its
*Slicing*, *Architecture* and *Error handling* sections, and the two 2026-09-22 revision notes on
the *Architecture* section that slice 2a added.

**Predecessor:** `docs/superpowers/plans/2026-09-22-registration-search-funnel-ui-slice-2a.md`
(the pure core, merged as [#646](https://github.com/cairn-ehr/cairn-ehr/pull/646)).

---

## Global Constraints

- **Licence:** AGPL-3.0-only. Every dependency added must be AGPL-3.0-compatible and must already
  be present in the `cairn-gui` tree's `deny.toml` allowances.
- **The one-way dependency (ADR-0021 / §9.5):** `cairn-gui/*` may depend on `crates/*`. Nothing in
  `crates/*` may ever depend on `cairn-gui`. This is what forces the new crate's location.
- **`cairn-gui-data` stays free of a database driver.** Its manifest says so in a comment; that
  comment is the reason this slice creates a crate rather than adding a module.
- **This slice does not touch `crates/`.** The root tree's full local gate is ~2 hours (132
  binaries); the `cairn-gui` gate is ~2 minutes. Everything here fits in the latter, and the one
  thing that wanted a root change is filed as an issue in Task 7 instead.
- **The `cairn-gui` gate, each command judged on its own exit code, never `| tail`:**
  ```
  cargo fmt --all --check --manifest-path cairn-gui/Cargo.toml
  cargo clippy --locked --manifest-path cairn-gui/Cargo.toml --workspace --tests -- -D warnings
  cargo test --locked --manifest-path cairn-gui/Cargo.toml --workspace
  RUSTDOCFLAGS=-D warnings cargo doc --locked --no-deps --manifest-path cairn-gui/Cargo.toml --workspace
  cargo deny --manifest-path cairn-gui/Cargo.toml check
  ```
  The `cargo doc` step is the one that bites: an intra-doc link from a public item to a **private**
  one is `error: public documentation for X links to private item Y` under `-D warnings`, and both
  `clippy` and `test` are blind to it.
- **Adding a `cairn-gui`-only crate moves exactly one lockfile:** `cairn-gui/Cargo.lock`. The root
  and `extensions/cairn_pgx` trees never see it. Prove it with a `--locked` run.
- **Database for the DB-gated tests:** `CAIRN_TEST_PG` pointing at `cairn_test` on the local PG18
  cluster, e.g.
  `export CAIRN_TEST_PG='host=/tmp port=5532 dbname=cairn_test'`.
- **Never hard-code cryptographic material in a test, and never give a non-cryptographic value a
  cryptographic NAME** (house rule 6). The test node key is derived at runtime with
  `std::array::from_fn`. Do not name any discriminator `salt`, `nonce` or `iv`.
- **Closing keywords:** run `python3 scripts/check_closing_keywords.py` **before** committing. A
  commit message containing `Filed rather than fixed: #648` auto-closes #648; `fix(#648):` is safe
  because the parenthesis breaks the adjacency.

---

## File Structure

| File | Responsibility |
|---|---|
| `cairn-gui/Cargo.toml` | **Modify** — add `cairn-gui-live` to `members`. |
| `cairn-gui/Cargo.lock` | **Modify** — regenerated; committed. |
| `cairn-gui/cairn-gui-data/src/port.rs` | **Modify** — add `DataError::Refused`, replace the "#648 lands in 2b" note with what the variant now means and what a caller must do with it. |
| `cairn-gui/cairn-gui-live/Cargo.toml` | **Create** — the new crate's manifest, with the reason it exists in a comment. |
| `cairn-gui/cairn-gui-live/src/lib.rs` | **Create** — crate doc (why this crate, not `cairn-gui-data`), `LiveData` and its constructor. |
| `cairn-gui/cairn-gui-live/src/error.rs` | **Create** — the pure refusal/outage rule and the `anyhow::Error` → `DataError` mapping. |
| `cairn-gui/cairn-gui-live/src/funnel.rs` | **Create** — the two `impl` blocks. |
| `cairn-gui/cairn-gui-live/tests/common/mod.rs` | **Create** — the DB gate (`cs()`), the schema/serial-guard connect, and the node-key/actor fixture. |
| `cairn-gui/cairn-gui-live/tests/db_gate_ran.rs` | **Create** — this tree's DB suite cannot go silently green. |
| `cairn-gui/cairn-gui-live/tests/attestation_through_the_port.rs` | **Create** — DB-gated: what the port registers is exactly what the prompt bounded, in order. |
| `cairn-gui/cairn-gui-live/tests/refusal_is_not_an_outage.rs` | **Create** — DB-gated: a floor refusal arrives as `Refused`, with the floor's own words. |
| `.github/workflows/rust.yml` | **Modify** — run the new DB-gated suite in the `test` job (the one with Postgres); tell the `gui` job it may skip. |
| `scripts/run-db-gated-tests.sh` | **Modify** — the local sanctioned gate runs the new suite too. |
| `docs/superpowers/specs/2026-09-20-registration-search-funnel-ui-design.md` | **Modify** — a dated revision note if the code contradicts the page (never an edit that erases it). |
| `docs/HANDOVER.md`, `docs/ROADMAP.md` | **Modify** — end-of-session currency. |

---

### Task 1: `DataError::Refused` — the variant, and what it obliges a caller to do

`cairn-gui-data`'s `DataError` today offers `NotFound` and `Unavailable(String)`. A deterministic
in-DB floor refusal — a term-less attested query at `db/045`, a chart whose first event is not its
registration at `db/005` step 8b — is neither. Reporting it as `Unavailable` tells the clerk to
retry something that **cannot** succeed, which is a precise untruth on a wrong-chart-prevention
surface (principle 4). This task adds the variant only; nothing produces it until Task 4.

**Files:**
- Modify: `cairn-gui/cairn-gui-data/src/port.rs` (the `DataError` enum and the doc comment above
  it, which currently promises the variant "lands with them in slice 2b")
- Test: `cairn-gui/cairn-gui-data/src/port.rs` (a new `#[cfg(test)] mod tests` at the foot of the
  file — this crate tests in-file)

**Interfaces:**
- Consumes: nothing.
- Produces: `cairn_gui_data::port::DataError::Refused(String)`. The `String` is the floor's own
  message, rendered for an operator. Later tasks construct it and match on it.

- [ ] **Step 1: Write the failing test**

At the foot of `cairn-gui/cairn-gui-data/src/port.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    /// A refusal and an outage must not compare equal, and must not be reachable through
    /// one arm. The window renders them with DIFFERENT advice — "try again" against "this
    /// cannot succeed as typed" — so a caller that matched them together would hand a clerk
    /// a retry button for a verdict (#648).
    #[test]
    fn a_refusal_is_not_an_outage() {
        let refused = DataError::Refused("registration refused: no search terms".into());
        let outage = DataError::Unavailable("connection closed".into());
        assert_ne!(refused, outage);
        assert!(
            !matches!(refused, DataError::Unavailable(_)),
            "a floor verdict must never arrive through the outage arm"
        );
    }

    /// The variant carries the floor's own words, not a category label. `commands.rs`'s
    /// rule 1 — return the underlying error text, never a generic string — is what makes
    /// an in-DB refusal actionable (§9.6); a `Refused` with nothing in it would be the
    /// same silence one variant over.
    #[test]
    fn a_refusal_carries_the_floors_own_words() {
        let DataError::Refused(text) = DataError::Refused("db/045: attested query has no terms".into())
        else {
            panic!("constructed as Refused");
        };
        assert!(text.contains("db/045"), "got: {text}");
    }
}
```

- [ ] **Step 2: Run the test and watch it fail**

```
cargo test --manifest-path cairn-gui/Cargo.toml -p cairn-gui-data
```
Expected: `error[E0599]: no variant or associated item named 'Refused' found for enum 'DataError'`.

- [ ] **Step 3: Add the variant and rewrite its doc**

Replace the whole doc comment above `pub enum DataError` and the enum itself with:

```rust
/// Why a port could not answer.
///
/// The three variants are three different clinical facts, and the split that matters is
/// [`DataError::Refused`] against [`DataError::Unavailable`] — the distinction
/// [#648](https://github.com/cairn-ehr/cairn-ehr/issues/648) asked for, landed here in
/// slice 2b now that a live implementation exists to produce it.
///
/// - `NotFound` — no such chart. A true, exhaustive answer.
/// - `Unavailable` — **nothing was decided.** A dropped connection, a lock timeout, a full
///   disk. The very same call may well succeed on a retry, so the window offers one.
/// - `Refused` — **the in-DB floor decided against this call and will decide the same way
///   every time** (a term-less attested query at `db/045`; a chart whose first event is not
///   its registration at `db/005` step 8b — #345/ADR-0061). A retry cannot succeed, and
///   offering one is a precise untruth on a wrong-chart-prevention surface (principle 4).
///   The payload is the floor's own message: it is legible on purpose (§9.6), and it is the
///   one thing that tells the clerk what to change.
///
/// # What a refusal does NOT change: the attestation still goes back
///
/// An earlier draft of the design reasoned that this variant would also decide whether the
/// caller calls `TokenStore::restore` — restore after an outage, pointless after a refusal.
/// **Building it showed that to be wrong, and the reason is the token store's shape.**
/// `restore` and `commit` are the two mandatory ends of every `take`; a caller that does
/// neither latches the store closed and costs a window reload. After a refusal `commit` would
/// be a lie (nothing was created), so `restore` is the only truthful end — and it is also the
/// right one, because the clerk's next act is to EDIT the form, and editing calls `discard`,
/// which destroys the doomed attestation on a new generation. A clerk who instead clicks
/// Register again gets the same legible refusal, which is honest. So both arms restore; only
/// the sentence on screen differs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DataError {
    NotFound,
    Unavailable(String),
    /// A deterministic in-DB floor verdict. See the enum doc: never retried, always legible.
    Refused(String),
}
```

- [ ] **Step 4: Run the tests and watch them pass**

```
cargo test --manifest-path cairn-gui/Cargo.toml -p cairn-gui-data
```
Expected: PASS. If the mock's `match` on `DataError` is now non-exhaustive anywhere, the
compiler will name the site — the mock never produces `Refused` (a fixture has no floor), so the
right fix is to leave the mock's behaviour alone and only satisfy exhaustiveness.

- [ ] **Step 5: Commit**

```bash
git add cairn-gui/cairn-gui-data/src/port.rs
git commit -m "feat(slice 2b): a floor refusal is not an outage (#648)"
```

---

### Task 2: The `cairn-gui-live` crate, and a DB gate that cannot go silently green

An empty crate is not worth a task on its own — but the **gate** is, and it must exist before the
first DB-gated test, or that test's first green run proves nothing. Every DB-gated test in this
repo self-skips when `CAIRN_TEST_PG` is unset, and a skipped test prints `ok`: the run where the
suite proved its invariants and the run where it returned on line 1 produce byte-identical output
(#442). The root tree closed that with `tests/common/db_gate.rs`. This tree needs the same rule,
in proportion.

**Files:**
- Create: `cairn-gui/cairn-gui-live/Cargo.toml`
- Create: `cairn-gui/cairn-gui-live/src/lib.rs`
- Create: `cairn-gui/cairn-gui-live/tests/common/mod.rs`
- Create: `cairn-gui/cairn-gui-live/tests/db_gate_ran.rs`
- Modify: `cairn-gui/Cargo.toml` (`members`)
- Modify: `cairn-gui/Cargo.lock` (regenerated)

**Interfaces:**
- Consumes: nothing.
- Produces: the crate `cairn-gui-live`; `common::cs() -> Option<String>`;
  `common::is_affirmative(&str) -> bool`.

- [ ] **Step 1: Write the manifest**

`cairn-gui/cairn-gui-live/Cargo.toml`:

```toml
[package]
name = "cairn-gui-live"
version = "0.1.0"
edition.workspace = true
rust-version.workspace = true
license.workspace = true
publish = false

[dependencies]
# WHY THIS CRATE EXISTS AS A CRATE, and is not a module in `cairn-gui-data`.
#
# `cairn-gui-data` owns the port TRAITS, and its own manifest states — deliberately — that
# it pulls no database driver, so that the pure rules and the `--mock` window can be built
# and tested with no Postgres anywhere. The live implementations need `cairn-node` and
# `tokio-postgres`, so they cannot go there without retracting that.
#
# Nor can they go in `/crates`: implementing a `cairn-gui` trait from a root crate would
# make `crates/*` depend on `cairn-gui`, which is the ONE direction ADR-0021 / §9.5 forbids
# (it is why the whole `cairn-gui` workspace is `exclude`d from the root one). A trait's
# implementation must live with the trait or with the type; here both roads lead to this
# workspace.
cairn-gui-data = { path = "../cairn-gui-data" }
cairn-gui-funnel = { path = "../cairn-gui-funnel" }
cairn-node = { path = "../../crates/cairn-node" }
cairn-event = { path = "../../crates/cairn-event" }
cairn-patient-search = { path = "../../crates/cairn-patient-search" }
tokio = { version = "1", features = ["sync"] }
tokio-postgres = "0.7"
anyhow = "1"
uuid = { version = "1", features = ["v7"] }
hex = "0.4"

[dev-dependencies]
tokio = { version = "1", features = ["rt-multi-thread", "macros", "sync"] }
```

- [ ] **Step 2: Write the crate doc and the type it will carry**

`cairn-gui/cairn-gui-live/src/lib.rs`:

```rust
//! The funnel's two ports, backed by this node's database.
//!
//! # What this crate is
//!
//! [`cairn_gui_data::port`] declares `PatientSearch` and `PatientRegistration`; `--mock`
//! implements them over fixtures. This crate implements them over a real
//! `tokio_postgres::Client`, by delegating to the `cairn-node` orchestrators that already
//! own the rules — `search_patients` (§5.8) and `register_patient` (§5.3, ADR-0061). It
//! adds **no clinical logic of its own**. There is exactly one decision in here, and it is
//! the one in [`error`]: whether a failure was a VERDICT about this call or an accident
//! that befell it.
//!
//! # Why it is a separate crate
//!
//! See the comment at the top of `Cargo.toml`. In one line: `cairn-gui-data` must stay free
//! of a database driver, and `/crates` may never depend on `cairn-gui`.
//!
//! # The window still signs nothing (§9.6)
//!
//! [`LiveData`] holds the NODE's key, because the node seals bodies and holds custody
//! (ADR-0052). It does not hold the clinician's key: a registration is not a per-write
//! human-authored clinical act in the ADR-0053 sense, and `register_patient` takes no human
//! author. When that changes, it changes in `cairn-node` first.
pub mod error;
mod funnel;

use cairn_event::SigningKey;
use tokio::sync::Mutex;
use tokio_postgres::Client;

/// One node connection, plus the identity every write is sealed under.
///
/// # Why `tokio::sync::Mutex` and not `std::sync::Mutex`
///
/// `register_patient` takes `&mut Client`, while `PatientRegistration::register` takes
/// `&self` and must return a `Send` future. A `std::sync::MutexGuard` is not `Send`, so
/// holding one across the `.await` inside `register` does not compile — and the mock's
/// "compute before the async block" trick is not available here, because the whole body is
/// awaits. The port doc in `cairn-gui-data` states this so it is not rediscovered by
/// fighting the compiler.
///
/// # One connection, not a pool
///
/// The funnel is one clerk at one keyboard; there is no concurrency worth pooling for, and a
/// mutex makes the borrow rules explicit at every call site. If a later slice puts two
/// windows on one `LiveData`, the second one waits — which is correct, not merely acceptable:
/// `register_patient` ticks this node's HLC.
pub struct LiveData {
    db: Mutex<Client>,
    node_sk: SigningKey,
    /// Hex of `node_sk`'s verifying key. Derived ONCE here rather than per call, so the kid
    /// a registration is sealed under cannot drift from the key that sealed it.
    node_kid: String,
    /// This node's origin id, as `cairn_node::identity::load_local` reports it.
    node_origin: String,
}

impl LiveData {
    /// Take ownership of a connection whose schema is already loaded.
    ///
    /// The caller connects (`cairn_node::db::connect_and_load_schema`) and reads the node
    /// identity (`cairn_node::identity::load_local`), exactly as the window's
    /// `build_live_state` already does, so that a window with one database does not end up
    /// with two connections that disagree about which node they are.
    pub fn new(db: Client, node_sk: SigningKey, node_origin: String) -> Self {
        let node_kid = hex::encode(node_sk.verifying_key().to_bytes());
        Self {
            db: Mutex::new(db),
            node_sk,
            node_kid,
            node_origin,
        }
    }
}
```

`src/error.rs` and `src/funnel.rs` do not exist yet, so add temporary stubs that compile:
`pub mod error;` needs `cairn-gui/cairn-gui-live/src/error.rs` to exist — create it containing
only `//! Placeholder — Task 3.` and create `src/funnel.rs` containing only
`//! Placeholder — Task 4.`. They are replaced wholesale in Tasks 3 and 4.

- [ ] **Step 3: Register the crate in the workspace and refresh the lockfile**

In `cairn-gui/Cargo.toml`, add `"cairn-gui-live",` to `members`, after `"cairn-gui-funnel",`.

```bash
cargo metadata --manifest-path cairn-gui/Cargo.toml --format-version 1 >/dev/null
cargo metadata --manifest-path cairn-gui/Cargo.toml --locked --format-version 1 >/dev/null
```
The second command exiting 0 is the proof CI's `--locked` clippy will be happy.

- [ ] **Step 4: Write the gate helper**

`cairn-gui/cairn-gui-live/tests/common/mod.rs`:

```rust
//! Shared rigging for this crate's DB-gated suites.
//!
//! Deliberately small. The root tree's `crates/cairn-node/tests/common/` is the real kit;
//! this is the minimum that lets a `cairn-gui` test open a schema-loaded database and sign
//! as a registered actor, and it should stay that way — a second full kit here would be a
//! second set of fixtures to keep true.
#![allow(dead_code)] // each test binary uses a different subset; that is normal for tests/common.

use cairn_event::SigningKey;
use tokio_postgres::Client;

/// The connection string for the single-node test database, or `None` when this run has no
/// database. Same variable the root tree's suites read, so one export rigs both.
pub fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

/// Does this environment-variable value mean YES?
///
/// Deliberately narrow, and the narrowness is the point (#450): `CAIRN_ALLOW_DB_SKIP=please`
/// or `=false` must NOT read as permission to skip the suite, or the opt-out becomes a way
/// to turn the gate off by typo.
pub fn is_affirmative(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}
```

- [ ] **Step 5: Write the failing gate test**

`cairn-gui/cairn-gui-live/tests/db_gate_ran.rs`:

```rust
//! This tree's DB-gated suites cannot go silently green (#442's rule, in proportion).
//!
//! Every DB-gated test in `cairn-gui-live` opens with `let Some(cs) = common::cs() else {
//! return; };` — the right default on a laptop with no PostgreSQL, and the wrong one in CI,
//! because a skipped test prints `ok`. The run that proved the funnel writes a truthful
//! attestation and the run that returned on line 1 are byte-identical in the output a PR
//! description quotes.
//!
//! So: with no `CAIRN_TEST_PG`, this test FAILS, unless the run declares that it knows —
//! `CAIRN_ALLOW_DB_SKIP=1`. The `gui` CI job sets that, because it has no database (see the
//! comment on that job); the `test` job, which has one, does not.
//!
//! This is much smaller than the root tree's `common/db_gate.rs`, and the difference is
//! honest rather than lazy: that one DERIVES the variable list from the test sources,
//! because three variables across a hundred suites is a list that rots. Here there is one
//! variable and two suites, both in this crate, both visible in one directory listing.
mod common;

const OPT_OUT: &str = "CAIRN_ALLOW_DB_SKIP";

#[test]
fn the_db_gated_suites_in_this_crate_actually_ran() {
    if common::cs().is_some() {
        return;
    }
    let declared = std::env::var(OPT_OUT)
        .ok()
        .is_some_and(|v| common::is_affirmative(&v));
    assert!(
        declared,
        "CAIRN_TEST_PG is unset, so every DB-gated test in cairn-gui-live returned on its \
         first line and printed `ok`. That is fine on a machine with no PostgreSQL — say so \
         with `export {OPT_OUT}=1`. It is not fine unnoticed: this crate's whole subject is \
         what a registration actually writes."
    );
}

/// The opt-out must not be satisfiable by accident. A bare `env::var(..).is_ok()` would let
/// `CAIRN_ALLOW_DB_SKIP=false` disable the gate, which is how a guard becomes decoration.
#[test]
fn only_an_affirmative_value_opts_out() {
    for yes in ["1", "true", "TRUE", " yes ", "on"] {
        assert!(common::is_affirmative(yes), "{yes:?} must read as yes");
    }
    for no in ["", "0", "false", "no", "off", "please", "maybe"] {
        assert!(!common::is_affirmative(no), "{no:?} must NOT read as yes");
    }
}
```

- [ ] **Step 6: Run the gate both ways**

```bash
env -u CAIRN_TEST_PG -u CAIRN_ALLOW_DB_SKIP \
  cargo test --manifest-path cairn-gui/Cargo.toml -p cairn-gui-live --test db_gate_ran
```
Expected: FAIL on `the_db_gated_suites_in_this_crate_actually_ran`.

```bash
env -u CAIRN_TEST_PG CAIRN_ALLOW_DB_SKIP=1 \
  cargo test --manifest-path cairn-gui/Cargo.toml -p cairn-gui-live --test db_gate_ran
```
Expected: PASS.

```bash
CAIRN_TEST_PG='host=/tmp port=5532 dbname=cairn_test' \
  cargo test --manifest-path cairn-gui/Cargo.toml -p cairn-gui-live --test db_gate_ran
```
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add cairn-gui/Cargo.toml cairn-gui/Cargo.lock cairn-gui/cairn-gui-live
git commit -m "feat(slice 2b): the live-port crate, and a DB gate it cannot skip past"
```

---

### Task 3: The rule — was this a verdict, or an accident?

This is the only decision in the crate, and it is the one #648 is about. Getting it backwards is a
real defect in both directions: calling an outage a refusal tells a clerk to change a form that was
fine; calling a refusal an outage hands them a retry button for a verdict, and they press it.

The discriminator is the SQLSTATE. Every refusal in the in-DB floor is a bare `RAISE EXCEPTION`,
which PostgreSQL assigns `P0001` — and `db/001_envelope.sql` states, in the comment above
`cairn_decode_hex_or_raise` (#228), that this **is a contract, not an accident of using `RAISE
EXCEPTION`**, because the node pull loop routes on it. `USING ERRCODE` is forbidden on those
refusals for that reason. Anything else — a dropped connection, a lock timeout, a serialization
failure, a full disk — is a verdict about nothing.

**Files:**
- Create (replacing the placeholder): `cairn-gui/cairn-gui-live/src/error.rs`

**Interfaces:**
- Consumes: `cairn_gui_data::port::DataError` (Task 1).
- Produces:
  - `pub fn refusal_is_deliberate(sqlstate: Option<&str>) -> bool`
  - `pub fn sqlstate_of(e: &anyhow::Error) -> Option<String>`
  - `pub fn data_error_from(e: &anyhow::Error) -> DataError`

- [ ] **Step 1: Write the failing tests**

At the foot of `cairn-gui/cairn-gui-live/src/error.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    /// The contract, pinned as a value. `db/001_envelope.sql` forbids `USING ERRCODE` on the
    /// floor's refusals precisely so this one code identifies all of them.
    #[test]
    fn a_bare_raise_exception_is_the_floors_verdict() {
        assert!(refusal_is_deliberate(Some("P0001")));
    }

    /// Everything else is an accident that befell the call, not a decision about it. These
    /// four are the ones an operator actually meets: a serialization failure, a lock
    /// timeout, a full disk, an admin shutdown.
    #[test]
    fn everything_else_is_an_accident_not_a_verdict() {
        for code in ["40001", "55P03", "53100", "57P01", "08006", "XX000"] {
            assert!(
                !refusal_is_deliberate(Some(code)),
                "{code} is not a floor verdict — treating it as one would tell the clerk to \
                 change a form that was never the problem"
            );
        }
    }

    /// NO SQLSTATE AT ALL IS NEVER A VERDICT, and this is the arm that matters most.
    ///
    /// A dropped connection, a TLS reset, a client-side decode failure: the statement never
    /// reached a decision. Defaulting these to `Refused` would tell a clerk their
    /// registration was rejected by the safety floor when the network blinked — and, worse,
    /// would tell them not to retry the one thing that would have worked.
    #[test]
    fn no_sqlstate_is_never_a_verdict() {
        assert!(!refusal_is_deliberate(None));
    }

    /// An error carrying no `tokio_postgres::Error` anywhere in its chain has no SQLSTATE to
    /// read, so it must reach the caller as an outage. `anyhow!` builds exactly that shape.
    #[test]
    fn an_error_with_no_database_cause_is_an_outage() {
        let e = anyhow::anyhow!("the node key could not be read");
        assert_eq!(sqlstate_of(&e), None);
        let DataError::Unavailable(text) = data_error_from(&e) else {
            panic!("expected an outage, got {:?}", data_error_from(&e));
        };
        assert!(
            text.contains("node key"),
            "the operator needs the real text, never a category label: {text}"
        );
    }

    /// The message must survive `anyhow`'s context layers, because that is how every
    /// `cairn-node` orchestrator reports: the outermost layer says which operation failed and
    /// an inner one says why. Dropping either half leaves the clerk with half a sentence.
    #[test]
    fn the_whole_context_chain_reaches_the_caller() {
        use anyhow::Context;
        let e = Err::<(), _>(anyhow::anyhow!("relation does not exist"))
            .context("registering the patient")
            .unwrap_err();
        let DataError::Unavailable(text) = data_error_from(&e) else {
            panic!("expected an outage");
        };
        assert!(text.contains("registering the patient"), "got: {text}");
        assert!(text.contains("relation does not exist"), "got: {text}");
    }
}
```

> **Note for the implementer — why the `P0001` end-to-end path is NOT unit-tested here.**
> `tokio_postgres::Error` has no public constructor, and a `DbError` (the thing that carries a
> SQLSTATE) cannot be built by hand at all; `cairn-node`'s own `db_diagnosis` module records
> this and says its equivalent arm "needs a live server". So `refusal_is_deliberate` is pinned
> purely above, and the proof that a real floor refusal reaches it **through the anyhow chain**
> is the DB-gated test in Task 6. That test is not a duplicate of these — it is the only thing
> that can catch a `cairn-node` call site that renders its error with
> `anyhow!("...: {}", legible_db_error(&e))`, which destroys the source and would silently turn
> every refusal into an outage.

- [ ] **Step 2: Run them and watch them fail**

```
cargo test --manifest-path cairn-gui/Cargo.toml -p cairn-gui-live --lib
```
Expected: FAIL — `cannot find function 'refusal_is_deliberate' in this scope`.

- [ ] **Step 3: Write the implementation**

Above the test module in `cairn-gui/cairn-gui-live/src/error.rs`:

```rust
//! One decision: was a failed call a VERDICT about it, or an accident that befell it?
//!
//! # The contract this rests on
//!
//! Every refusal in the in-DB floor is a bare `RAISE EXCEPTION`, which PostgreSQL assigns
//! SQLSTATE `P0001`. That is a **contract, not an accident**: `db/001_envelope.sql` states
//! it in the comment above `cairn_decode_hex_or_raise` (#228), and forbids `USING ERRCODE`
//! on those refusals, because the node pull loop routes on it. Anything else — a dropped
//! connection, a lock timeout, a serialization failure, a full disk — decided nothing.
//!
//! # ⚠️ A THIRD HOME FOR A RULE THAT SHOULD HAVE ONE
//!
//! `cairn-sync`'s `refusal_is_deliberate` and `cairn-node`'s
//! `restore::clinical::refusal_is_deliberate` are the other two, and the second one's own
//! doc already calls itself "A SECOND HOME ... Keep the two identical". This is the third,
//! and it is here because consolidating them means changing `crates/`, which is a different
//! slice's blast radius. **Filed as its own issue — see the module's entry in HANDOVER.**
//! Until then: if you change the rule, change all three. The drift costs a wrong verdict,
//! not merely an inaccurate sentence.
//!
//! The three are not *quite* redundant, and the difference is worth knowing: the other two
//! take an already-extracted `Option<&str>`, because their callers hold a
//! `tokio_postgres::Error` directly. This crate's callers hold an `anyhow::Error` from a
//! `cairn-node` orchestrator, so it must dig the SQLSTATE out of a context chain first —
//! which is [`sqlstate_of`], and is the part that can silently stop working.
use cairn_gui_data::port::DataError;

/// The SQLSTATE PostgreSQL assigns to a bare `RAISE EXCEPTION` in PL/pgSQL.
const SQLSTATE_RAISE_EXCEPTION: &str = "P0001";

/// Did the floor DELIBERATELY refuse this call? **Pure.**
///
/// `None` — no SQLSTATE reached us at all — is never a verdict. See the module doc.
pub fn refusal_is_deliberate(sqlstate: Option<&str>) -> bool {
    sqlstate == Some(SQLSTATE_RAISE_EXCEPTION)
}

/// Dig the SQLSTATE out of a `cairn-node` orchestrator's error.
///
/// Walks the whole `anyhow` chain rather than looking only at the outermost error, because
/// every orchestrator adds `.context("...")` layers naming the operation. The same walk
/// `cairn_node::db_diagnosis::operator_chain` performs, for the same reason.
///
/// **This returns `None` when a call site rendered its database error into a string** — e.g.
/// `anyhow!("submit: {}", legible_db_error(&e))` — because that destroys the source and the
/// `tokio_postgres::Error` is no longer in the chain to be found. The failure is silent and
/// one-directional: every refusal becomes an outage. Only a test against a real floor can
/// catch it, which is why Task 6's DB-gated test exists.
pub fn sqlstate_of(e: &anyhow::Error) -> Option<String> {
    e.chain()
        .find_map(|cause| cause.downcast_ref::<tokio_postgres::Error>())
        .and_then(|pg| pg.as_db_error())
        .map(|db| db.code().code().to_string())
}

/// Map a `cairn-node` orchestrator's failure onto the port's error type.
///
/// The message is `cairn_node::db_diagnosis::operator_chain`'s rendering in both arms — one
/// line, the server's message rendered once, every context layer kept. `commands.rs`'s rule
/// 1: return the underlying text, never a generic string.
pub fn data_error_from(e: &anyhow::Error) -> DataError {
    let text = cairn_node::db_diagnosis::operator_chain(e);
    if refusal_is_deliberate(sqlstate_of(e).as_deref()) {
        DataError::Refused(text)
    } else {
        DataError::Unavailable(text)
    }
}
```

- [ ] **Step 4: Run them and watch them pass**

```
cargo test --manifest-path cairn-gui/Cargo.toml -p cairn-gui-live --lib
```
Expected: PASS, 5 tests.

- [ ] **Step 5: Commit**

```bash
git add cairn-gui/cairn-gui-live/src/error.rs
git commit -m "feat(slice 2b): a verdict and an accident are told apart by one contract"
```

---

### Task 4: `PatientSearch`, live

**Files:**
- Create (replacing the placeholder): `cairn-gui/cairn-gui-live/src/funnel.rs`
- Modify: `cairn-gui/cairn-gui-live/tests/common/mod.rs` (add `connect` and `setup`)
- Create: `cairn-gui/cairn-gui-live/tests/attestation_through_the_port.rs` (the browse half; the
  registration half lands in Task 6)

**Interfaces:**
- Consumes: `LiveData` (Task 2), `data_error_from` (Task 3).
- Produces: `impl cairn_gui_data::port::PatientSearch for LiveData`;
  `common::connect(&str) -> Client`; `common::setup(&Client) -> (SigningKey, String)`.

- [ ] **Step 1: Add the database rigging to `tests/common/mod.rs`**

Append to `cairn-gui/cairn-gui-live/tests/common/mod.rs`:

```rust
/// Open the test database with its schema loaded, holding the cluster-wide advisory lock.
///
/// `test_serial_guard` is the SAME lock the root tree's DB-gated suites take
/// (`0x4341524E`), which is what lets this crate's suites run concurrently with a root
/// `cargo test` against the same cluster without interleaving truncations. Then
/// `connect_and_load_schema` on a SECOND connection replays `db/*.sql` — it must, because
/// this tree is built and run independently of the root one and may be the first thing to
/// touch a fresh database.
pub async fn connect(cs: &str) -> (Client, Client) {
    let guard = cairn_node::db::test_serial_guard(cs)
        .await
        .expect("the cluster-wide test lock");
    let db = cairn_node::db::connect_and_load_schema(cs)
        .await
        .expect("schema-loaded connection");
    (db, guard)
}

/// Clear the tables these suites write, and enrol a signer.
///
/// Mirrors `crates/cairn-node/tests/common/setup`, narrowed to the tables a registration
/// touches. The key is DERIVED at runtime, never a literal: a byte-array literal in a
/// cryptographic context is a critical CodeQL finding that blocks the scan until a human
/// dismisses it (house rule 6a), and the derivation keeps the fixture deterministic anyway.
pub async fn setup(c: &Client) -> (SigningKey, String) {
    c.batch_execute(
        "TRUNCATE event_log, actor_event, patient_chart, patient_identifier, \
         patient_demographic CASCADE",
    )
    .await
    .expect("truncate the clinical core");
    c.batch_execute(
        "DO $$ BEGIN IF to_regclass('public.patient_registration') IS NOT NULL \
         THEN TRUNCATE patient_registration; END IF; END $$;",
    )
    .await
    .expect("truncate the registration projection");

    let seed: [u8; 32] = std::array::from_fn(|i| (i as u8).wrapping_mul(11).wrapping_add(3));
    let sk = SigningKey::from_bytes(&seed);
    let kid = hex::encode(sk.verifying_key().to_bytes());
    c.execute(
        "SELECT enroll_actor('agent', \
         '{\"model\":\"funnel-port-test\",\"version\":\"1\",\"skill_epoch\":\"e\"}', $1)",
        &[&kid],
    )
    .await
    .expect("enrol the test signer");
    (sk, kid)
}
```

Add `hex = "0.4"` and `cairn-event` to `[dev-dependencies]` only if the compiler asks — they are
already normal dependencies, and a crate's own `tests/` can use its normal dependencies.

- [ ] **Step 2: Write the failing test**

`cairn-gui/cairn-gui-live/tests/attestation_through_the_port.rs`:

```rust
//! DB-gated: what the funnel's live ports actually do to a real record.
//!
//! The root tree already proves that `register_patient` stores the candidate list it is
//! handed, in order (`patient_register.rs`'s
//! `the_attestation_round_trips_from_the_displayed_list_to_the_stored_body`). This file
//! proves something that suite CANNOT: that what the PORT hands it is the list the step-3
//! prompt actually bounded — the `PromptList`, not the node's raw answer. A prompt showing
//! five could otherwise swear it displayed forty (slice 2a's own end-to-end walk had exactly
//! that bug before `bound_for_prompt` gained a call site).
mod common;

use cairn_gui_data::port::{PatientRegistration, PatientSearch};
use cairn_gui_funnel::{bound_for_prompt, TokenStore};
use cairn_gui_live::LiveData;
use cairn_patient_search::SearchQuery;

/// A fixed clock, so an age never changes under the suite.
const TODAY: &str = "2026-09-22";

/// A search that matches nothing must be an EMPTY list, never an error.
///
/// "The search failed" and "nobody matched" are different answers and only one of them is
/// evidence of absence — which is precisely the distinction that decides whether a clerk
/// creates a duplicate chart (principle 4). The port's own doc requires this; nothing
/// enforced it until here.
#[tokio::test]
async fn nobody_matched_is_an_empty_list_not_a_failure() {
    let Some(cs) = common::cs() else { return };
    let (db, _guard) = common::connect(&cs).await;
    let (sk, _kid) = common::setup(&db).await;
    let live = LiveData::new(db, sk, "n".to_string());

    let query = SearchQuery::new("zzzznobodyzzzz", Some("1900-01-01"), &[]);
    let found = live
        .search(&query, TODAY)
        .await
        .expect("a search that matches nothing still SUCCEEDS");

    assert!(found.candidates.is_empty(), "got: {:?}", found.candidates);
    assert!(
        !found.incomplete,
        "an exhaustive search that found nothing is COMPLETE — marking it partial would \
         tell the clerk to keep looking for a chart that does not exist"
    );
}
```

- [ ] **Step 3: Run it and watch it fail**

```bash
CAIRN_TEST_PG='host=/tmp port=5532 dbname=cairn_test' \
  cargo test --manifest-path cairn-gui/Cargo.toml -p cairn-gui-live \
  --test attestation_through_the_port
```
Expected: FAIL to compile — `LiveData` does not implement `PatientSearch`.

- [ ] **Step 4: Write the implementation**

`cairn-gui/cairn-gui-live/src/funnel.rs`:

```rust
//! The two port implementations. Each is a thin adapter over a `cairn-node` orchestrator:
//! resolve the connection, call, map the error. **No clinical logic lives here** — the same
//! rule `commands.rs` follows one layer up, and for the same reason: a rule that lives in an
//! adapter is a rule no test looks for.
use crate::error::data_error_from;
use crate::LiveData;
use cairn_gui_data::port::{DataError, PatientRegistration, PatientSearch};
use cairn_gui_funnel::AttestedSearch;
use cairn_patient_search::{CandidateList, SearchQuery};
use uuid::Uuid;

impl PatientSearch for LiveData {
    async fn search(&self, query: &SearchQuery, today: &str) -> Result<CandidateList, DataError> {
        let db = self.db.lock().await;
        // `today` is passed STRAIGHT THROUGH. The port's doc forbids an implementation from
        // substituting its own clock: the age beside a patient's name would then depend on
        // which clock won, with nothing on screen saying which.
        cairn_node::patient::search::search_patients(&*db, query, today)
            .await
            .map_err(|e| data_error_from(&e))
    }
}
```

- [ ] **Step 5: Run it and watch it pass**

```bash
CAIRN_TEST_PG='host=/tmp port=5532 dbname=cairn_test' \
  cargo test --manifest-path cairn-gui/Cargo.toml -p cairn-gui-live \
  --test attestation_through_the_port
```
Expected: PASS, 1 test.

- [ ] **Step 6: Commit**

```bash
git add cairn-gui/cairn-gui-live
git commit -m "feat(slice 2b): the live search port, and nothing-found is not a failure"
```

---

### Task 5: `PatientRegistration`, live

**Files:**
- Modify: `cairn-gui/cairn-gui-live/src/funnel.rs`
- Modify: `cairn-gui/cairn-gui-live/tests/attestation_through_the_port.rs`

**Interfaces:**
- Consumes: `LiveData`, `data_error_from`, `AttestedSearch`.
- Produces: `impl cairn_gui_data::port::PatientRegistration for LiveData`.

- [ ] **Step 1: Write the failing test**

Append to `cairn-gui/cairn-gui-live/tests/attestation_through_the_port.rs`:

```rust
/// The walk the design is about: browse, nothing fits, register, and the chart is findable
/// afterwards by the very name that was typed.
///
/// The last clause is what makes this more than a smoke test. `register_patient` asserts the
/// typed name and date of birth as demographics precisely so a registered chart is findable
/// (#350); a port that dropped `name` on the floor would still return a `Uuid` and still
/// pass every assertion about the attestation — and would create a chart nobody can ever
/// find again, which is the failure mode the whole funnel exists to prevent.
#[tokio::test]
async fn a_registration_creates_a_chart_the_next_search_finds() {
    let Some(cs) = common::cs() else { return };
    let (db, _guard) = common::connect(&cs).await;
    let (sk, _kid) = common::setup(&db).await;
    let live = LiveData::new(db, sk, "n".to_string());

    // The ONE typed string, feeding both the query and the name — never reassembled.
    let typed = "Ngaiterangi Waiariki";
    let query = SearchQuery::new(typed, Some("1991-03-04"), &[]);

    let before = live.search(&query, TODAY).await.expect("the browse search");
    assert!(before.candidates.is_empty(), "the fixture starts empty");

    let mut store = TokenStore::new();
    let token = store
        .record(query.clone(), bound_for_prompt(&before))
        .expect("a non-empty query mints a token");
    let attested = store.take(token).expect("the token is redeemable");

    let created = live
        .register(attested, Some(typed))
        .await
        .map_err(|(e, _returned)| e)
        .expect("a well-formed registration is accepted");
    store.commit();

    let after = live.search(&query, TODAY).await.expect("the browse search");
    let ids: Vec<_> = after.candidates.iter().map(|c| c.patient_id).collect();
    assert!(
        ids.contains(&created),
        "the chart just registered must be findable by the name it was registered under — \
         got {ids:?}"
    );
}

/// THE TEST THIS FILE EXISTS FOR.
///
/// The registration must swear to the list the PROMPT bounded, not to the node's raw answer.
/// With more candidates than `PROMPT_CAP`, the two differ — and a stored body naming all of
/// them would be a signed claim that the clerk saw a screenful they never saw, which is
/// exactly the claim someone would later use to argue they should have spotted the duplicate
/// (design decision 3).
#[tokio::test]
async fn the_stored_attestation_names_what_the_prompt_bounded_and_nothing_more() {
    let Some(cs) = common::cs() else { return };
    let (db, _guard) = common::connect(&cs).await;
    let (sk, _kid) = common::setup(&db).await;
    let live = LiveData::new(db, sk, "n".to_string());

    // Register more namesakes than the prompt can show, so the raw list and the bounded one
    // are genuinely different values.
    let shared = "Kowalczyk";
    for n in 0..(cairn_gui_funnel::PROMPT_CAP + 3) {
        let q = SearchQuery::new(&format!("{shared} Number{n}"), Some("1970-01-01"), &[]);
        let mut store = TokenStore::new();
        let t = store
            .record(q.clone(), bound_for_prompt(&cairn_patient_search::CandidateList {
                candidates: vec![],
                incomplete: false,
                incomplete_reason: None,
            }))
            .expect("a non-empty query mints a token");
        let a = store.take(t).expect("redeemable");
        live.register(a, Some(&format!("{shared} Number{n}")))
            .await
            .map_err(|(e, _)| e)
            .expect("each namesake registers");
        store.commit();
    }

    let query = SearchQuery::new(shared, None, &[]);
    let raw = live.search(&query, TODAY).await.expect("the step-3 search");
    assert!(
        raw.candidates.len() > cairn_gui_funnel::PROMPT_CAP,
        "the fixture must OVERFLOW the prompt or this test proves nothing — got {}",
        raw.candidates.len()
    );

    let prompt = bound_for_prompt(&raw);
    let shown: Vec<_> = prompt.as_list().candidates.iter().map(|c| c.patient_id).collect();

    let mut store = TokenStore::new();
    let token = store.record(query, prompt).expect("a token");
    let attested = store.take(token).expect("redeemable");
    let created = live
        .register(attested, Some("Kowalczyk Newcomer"))
        .await
        .map_err(|(e, _)| e)
        .expect("registration accepted");
    store.commit();

    let stored = common::stored_displayed(&live_db(&live).await, created).await;
    assert_eq!(
        stored, shown,
        "the signed body must name the ids the PROMPT displayed, in display order — not the \
         node's raw answer, and not a reordering of it"
    );
    assert_eq!(
        stored.len(),
        cairn_gui_funnel::PROMPT_CAP,
        "the bound must actually have bitten"
    );
}
```

> **Implementer's note.** `live_db(&live)` above is a stand-in: `LiveData::db` is private, and it
> should stay private. Resolve it by opening a **separate read connection** in the test instead —
> `common::connect(&cs)` already returns one, so keep that first `Client` for the assertions and
> build `LiveData` from a second `connect_and_load_schema`. Rewrite the two tests to take
> `(reader, _guard) = common::connect(&cs)` and
> `live = LiveData::new(cairn_node::db::connect_and_load_schema(&cs).await.unwrap(), sk, "n".into())`,
> then assert with `common::stored_displayed(&reader, created)`. Do NOT add a public accessor to
> `LiveData` for a test's convenience.

Add to `tests/common/mod.rs`:

```rust
/// Read the RAW `search.displayed` array back out of the stored event body, in order.
///
/// A query against `event_log.body`, NOT the `patient_registration` projection: the
/// projection stores only `displayed_count` (deliberately — db/045's own comment on why two
/// representations of one number is a lie waiting to happen), so the signed body is the only
/// place the actual LIST can be read back from. Goes through `::text` + `serde_json` because
/// this tree does not enable tokio-postgres's `with-serde_json-1` feature, the same
/// convention `crates/cairn-node/tests/patient_register.rs` follows.
pub async fn stored_displayed(c: &Client, patient: uuid::Uuid) -> Vec<uuid::Uuid> {
    let row = c
        .query_one(
            "SELECT (body -> 'search' -> 'displayed')::text FROM event_log \
             WHERE event_type = 'identity.registration.asserted' \
               AND (body ->> 'patient_id') = $1",
            &[&patient.to_string()],
        )
        .await
        .expect("the registration event");
    let json: String = row.get(0);
    serde_json::from_str::<Vec<uuid::Uuid>>(&json).expect("displayed is an array of uuids")
}
```

This needs `serde_json = "1"` in `[dev-dependencies]`. If the stored shape turns out to differ
(an array of objects rather than of bare ids), copy the exact reader from
`crates/cairn-node/tests/patient_register.rs::stored_displayed` verbatim rather than guessing —
**that file is the authority on the stored shape.**

- [ ] **Step 2: Run and watch it fail**

```bash
CAIRN_TEST_PG='host=/tmp port=5532 dbname=cairn_test' \
  cargo test --manifest-path cairn-gui/Cargo.toml -p cairn-gui-live \
  --test attestation_through_the_port
```
Expected: FAIL to compile — `LiveData` does not implement `PatientRegistration`.

- [ ] **Step 3: Write the implementation**

Append to `cairn-gui/cairn-gui-live/src/funnel.rs`:

```rust
impl PatientRegistration for LiveData {
    async fn register(
        &self,
        attested: AttestedSearch,
        name: Option<&str>,
    ) -> Result<Uuid, (DataError, AttestedSearch)> {
        let mut db = self.db.lock().await;
        // The query and the list come out of ONE value that has no public constructor, so
        // this call cannot be handed a pair that were never together — which is the whole
        // reason `AttestedSearch` exists (see `cairn_gui_funnel::token`).
        let outcome = cairn_node::patient::register::register_patient(
            &mut db,
            &self.node_sk,
            &self.node_kid,
            &self.node_origin,
            name,
            attested.query(),
            attested.displayed(),
        )
        .await;

        // The attestation goes BACK inside the error, and that is not decoration:
        // `TokenStore::restore` needs exactly this value to put the search back after a
        // failed write, and nothing else can obtain one. Losing it here makes the clerk
        // re-search, which is the design's "Register fails. The form keeps its values."
        // broken.
        match outcome {
            Ok(id) => Ok(id),
            Err(e) => Err((data_error_from(&e), attested)),
        }
    }
}
```

- [ ] **Step 4: Run and watch it pass**

```bash
CAIRN_TEST_PG='host=/tmp port=5532 dbname=cairn_test' \
  cargo test --manifest-path cairn-gui/Cargo.toml -p cairn-gui-live \
  --test attestation_through_the_port
```
Expected: PASS, 3 tests.

- [ ] **Step 5: Commit**

```bash
git add cairn-gui/cairn-gui-live
git commit -m "feat(slice 2b): the live registration port attests what the prompt bounded"
```

---

### Task 6: A floor refusal reaches the clerk as a refusal

The one thing no unit test can prove. `data_error_from` reads the SQLSTATE out of the `anyhow`
chain — and that chain survives only if every `cairn-node` call site on this path preserved it.
A site that rendered its error with `anyhow!("...: {}", legible_db_error(&e))` destroys the
source, and then **every refusal silently becomes an outage**: the clerk is told to retry a
registration that can never succeed. This test is the only detector.

**Files:**
- Create: `cairn-gui/cairn-gui-live/tests/refusal_is_not_an_outage.rs`

**Interfaces:**
- Consumes: everything above.
- Produces: nothing new.

- [ ] **Step 1: Write the failing test**

```rust
//! DB-gated: the in-DB floor's verdicts arrive as VERDICTS (#648).
//!
//! `data_error_from` decides refusal-vs-outage from the SQLSTATE, which it digs out of the
//! `anyhow` chain a `cairn-node` orchestrator returns. Nothing in the type system keeps that
//! chain intact: a call site anywhere on this path that renders its database error into a
//! string destroys the `tokio_postgres::Error`, and from then on every floor refusal reads
//! as an outage — a retry button on a verdict. Only a real floor can catch that, so this
//! file is the detector, and it must NEVER be relaxed into asserting merely that the call
//! failed.
mod common;

use cairn_gui_data::port::{DataError, PatientRegistration};
use cairn_gui_funnel::{bound_for_prompt, TokenStore};
use cairn_gui_live::LiveData;
use cairn_patient_search::{CandidateList, SearchQuery};

/// db/045 refuses a registration whose attested query carries no terms: an attestation that
/// names no search is not evidence of due diligence (ADR-0061). The floor decides that the
/// same way every time, so it is a refusal, not an outage.
///
/// Reaching it needs a query the funnel's own `TokenStore` would refuse to tokenise — so the
/// test builds the attestation from a query with terms and then registers under a floor that
/// sees none. Use the shape db/045 actually rejects; read that file's own comment for which
/// column it checks, and if the deterministic refusal on this path turns out to be db/005
/// step 8b instead, target that and rename the test to say so. What must NOT change is the
/// assertion: whatever the floor refuses, it must arrive as `Refused`.
#[tokio::test]
async fn a_deterministic_floor_refusal_is_refused_not_unavailable() {
    let Some(cs) = common::cs() else { return };
    let (db, _guard) = common::connect(&cs).await;
    let (sk, _kid) = common::setup(&db).await;
    let live = LiveData::new(
        cairn_node::db::connect_and_load_schema(&cs).await.unwrap(),
        sk,
        "n".to_string(),
    );

    // A malformed date of birth: `register_patient` validates the shape before ticking any
    // HLC, so the whole call refuses with zero side effects. Deterministic by construction —
    // the same string refuses identically forever.
    let query = SearchQuery::new("Refusal Probe", Some("not-a-date"), &[]);
    let mut store = TokenStore::new();
    let token = store
        .record(
            query,
            bound_for_prompt(&CandidateList {
                candidates: vec![],
                incomplete: false,
                incomplete_reason: None,
            }),
        )
        .expect("a token");
    let attested = store.take(token).expect("redeemable");

    let (err, returned) = live
        .register(attested, Some("Refusal Probe"))
        .await
        .expect_err("a malformed date of birth must refuse the whole call");

    match &err {
        DataError::Refused(text) => assert!(
            !text.is_empty(),
            "a refusal with no words is the same silence one variant over"
        ),
        other => panic!(
            "the floor's verdict arrived as {other:?}. If this is `Unavailable`, the anyhow \
             chain lost its tokio_postgres::Error somewhere on this path and EVERY refusal \
             now reads as an outage — find the call site that rendered its error into a \
             string. See src/error.rs's `sqlstate_of`."
        ),
    }

    // The other half of the port's contract: the attestation comes BACK, so the form keeps
    // its values and the token store can be settled.
    store.restore(returned);
}

/// A refusal must leave NO chart behind. `register_patient` validates the dob shape before
/// ticking any HLC or authoring the registration act, so a refused call is atomic — and a
/// partial chart would be a patient record born without the act that licenses it, which
/// db/005 step 8b exists to make impossible (#345).
#[tokio::test]
async fn a_refused_registration_creates_no_chart() {
    let Some(cs) = common::cs() else { return };
    let (reader, _guard) = common::connect(&cs).await;
    let (sk, _kid) = common::setup(&reader).await;
    let live = LiveData::new(
        cairn_node::db::connect_and_load_schema(&cs).await.unwrap(),
        sk,
        "n".to_string(),
    );

    let before: i64 = reader
        .query_one("SELECT count(*) FROM patient_chart", &[])
        .await
        .unwrap()
        .get(0);

    let query = SearchQuery::new("Refusal Probe Two", Some("1980-13-45xyz"), &[]);
    let mut store = TokenStore::new();
    let token = store
        .record(
            query,
            bound_for_prompt(&CandidateList {
                candidates: vec![],
                incomplete: false,
                incomplete_reason: None,
            }),
        )
        .expect("a token");
    let attested = store.take(token).expect("redeemable");
    let (_err, returned) = live
        .register(attested, Some("Refusal Probe Two"))
        .await
        .expect_err("refused");
    store.restore(returned);

    let after: i64 = reader
        .query_one("SELECT count(*) FROM patient_chart", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(before, after, "a refused registration must leave no chart");
}
```

- [ ] **Step 2: Run and watch it fail (or discover the real shape)**

```bash
CAIRN_TEST_PG='host=/tmp port=5532 dbname=cairn_test' \
  cargo test --manifest-path cairn-gui/Cargo.toml -p cairn-gui-live \
  --test refusal_is_not_an_outage
```

**Two outcomes are both informative, and neither is a reason to weaken the test:**

1. It fails with `other: Unavailable(..)` — the chain is broken somewhere on this path. **This is
   a real defect, and it is the one this slice is about.** If the broken call site is inside
   `cairn-gui-live`, fix it. If it is in `crates/`, that is out of this slice's blast radius:
   **file an issue** naming the file and line, quote the failing assertion in it, and mark the
   test `#[ignore]` with a comment naming the issue — never delete it, and never soften the
   assertion to `is_err()`.
2. It fails because a malformed date of birth is refused *before* reaching Postgres (a pure Rust
   `anyhow!` with no SQLSTATE at all) — then the dob is the wrong probe. Switch to a refusal
   that is genuinely raised by the floor: register the same `patient_id` twice, or use db/045's
   term-less-query refusal. The *contract under test is unchanged*; only the probe moves.

- [ ] **Step 3: Make it pass**

Whichever of the two above applies. Record what you found in the test's own doc comment — a
reader six months from now needs to know which floor check this probe lands on and why that one.

- [ ] **Step 4: Run the whole crate**

```bash
CAIRN_TEST_PG='host=/tmp port=5532 dbname=cairn_test' \
  cargo test --manifest-path cairn-gui/Cargo.toml -p cairn-gui-live
```
Expected: all green.

- [ ] **Step 5: Commit**

```bash
python3 scripts/check_closing_keywords.py
git add cairn-gui/cairn-gui-live
git commit -m "test(slice 2b): a floor verdict must not reach the clerk as an outage"
```

---

### Task 7: Make the new suite run where a database exists

A DB-gated suite that runs nowhere is a suite that proves nothing. The `gui` CI job has no
PostgreSQL — deliberately, and its own comment says so ("every test in this tree is pure") — and
standing one up there would mean the PGDG apt repo, PostgreSQL 18 **and a build of the `cairn_pgx`
extension**, because `connect_and_load_schema` needs it. The `test` job already has all three.

`cairn-gui-live` does not depend on `tauri`, so `-p cairn-gui-live` builds in the `test` job
without the WebKitGTK stack that job lacks.

**Files:**
- Modify: `.github/workflows/rust.yml` (the `test` job — a new step; the `gui` job — the opt-out
  and a corrected comment)
- Modify: `scripts/run-db-gated-tests.sh`
- File: one GitHub issue for the three-homes consolidation named in `src/error.rs`'s module doc

**Interfaces:**
- Consumes: the crate and its suites.
- Produces: CI coverage.

- [ ] **Step 1: Add the step to the `test` job**

Immediately after the `cargo test (cairn-medication-view, fixtures feature)` step:

```yaml
      # ---- the cairn-gui tree's ONE database-backed crate ------------------------------
      # `cairn-gui-live` implements the funnel's ports over a real node connection, so its
      # suites need PostgreSQL + cairn_pgx — which this job has and the `gui` job
      # deliberately does not (standing a second cluster up there would mean building the
      # pgrx extension twice per CI run). It does not depend on `tauri`, so `-p` builds it
      # here without the WebKitGTK stack this job has no reason to install.
      #
      # Without this step the whole suite would self-skip and print `ok`, which is the #442
      # failure; `db_gate_ran.rs` in that crate fails closed if CAIRN_TEST_PG is unset and
      # the run has not declared CAIRN_ALLOW_DB_SKIP.
      - name: cargo test (cairn-gui-live — the funnel's ports against a real node)
        env:
          CAIRN_TEST_PG: host=127.0.0.1 port=${{ env.PGPORT }} user=postgres dbname=cairn_test
        run: cargo test --locked --manifest-path cairn-gui/Cargo.toml -p cairn-gui-live
```

- [ ] **Step 2: Tell the `gui` job it may skip, and correct the comment that is now false**

In the `gui` job, replace the comment line
`# No Postgres: every test in this tree is pure (the view model, the session-key lock, the` …
through its closing line with:

```yaml
  # Postgres: none here, and ONE crate in this tree now needs it. `cairn-gui-live` implements
  # the funnel's ports over a real node connection; its suites run in the `test` job below,
  # which already has PostgreSQL 18 and the cairn_pgx extension. Standing a second cluster up
  # in this job would mean building that extension twice per CI run for no extra coverage.
  # Everything else in this tree is pure (the view model, the session-key lock, the JS/Rust
  # drift guard, the funnel's rules), and CAIRN_ALLOW_DB_SKIP below is the declaration that
  # the skip is intended rather than accidental (#442/#450).
```

and add to that job's `env:` block:

```yaml
      # This job has no database on purpose (see above). Declaring it is what keeps
      # cairn-gui-live's `db_gate_ran` from failing closed here while still failing closed on
      # a developer's machine that simply forgot to export CAIRN_TEST_PG.
      CAIRN_ALLOW_DB_SKIP: "1"
```

- [ ] **Step 3: Add it to the local sanctioned gate**

At the foot of `scripts/run-db-gated-tests.sh`, after `cargo test --workspace`:

```bash
# The cairn-gui tree is a SEPARATE cargo workspace (the root one `exclude`s it), so the
# run above has never covered a line of it. That was fine while every test there was pure;
# `cairn-gui-live` is not — it drives the funnel's ports against this same database. Run it
# here so the local gate and CI's `test` job ask the same question.
echo "== cargo test -p cairn-gui-live against ${CAIRN_TEST_PG}"
cargo test --manifest-path cairn-gui/Cargo.toml -p cairn-gui-live
```

- [ ] **Step 4: File the consolidation issue**

`src/error.rs`'s module doc says this is the **third** home of the P0001 rule. Fixing that means
changing `crates/`, which is a different slice's blast radius (the root tree's full local gate is
~2 hours). So file it, per house rule 5:

```bash
gh issue create \
  --title "The P0001 floor-refusal rule has three homes; give it one" \
  --body "$(cat <<'EOF'
`refusal_is_deliberate` — "a bare RAISE EXCEPTION (P0001) is the floor's verdict; anything
else decided nothing" — now exists three times:

- `crates/cairn-sync/src/main.rs`
- `crates/cairn-node/src/restore/clinical.rs` (whose own doc already calls itself "A SECOND
  HOME ... Keep the two identical")
- `cairn-gui/cairn-gui-live/src/error.rs` (new, slice 2b)

It drives a DECISION, so a drift costs a wrong verdict, not merely an inaccurate sentence:
in the node plane a wrongly-classified refusal freezes a peer link or fills the pen; in the
GUI it hands a clerk a retry button for a registration that can never succeed.

`cairn-sync` and `cairn-gui-live` both depend on `cairn-node`, so one public home in
`cairn_node::db_diagnosis` would retire all three copies — the restore module's doc already
names that as the intended end state, phrased the other way round ("if cairn-sync ever gains
a lib.rs").

Not done in slice 2b because it changes `crates/`, whose full local gate is ~2 hours against
the ~2 minutes this slice needed. Pairs naturally with #633 (nothing pins the `USING ERRCODE`
contract the rule rests on) — a shared home is the obvious place to hang that pin.
EOF
)"
```

Note the issue number in `src/error.rs`'s module doc, replacing "see the module's entry in
HANDOVER".

- [ ] **Step 5: Run the whole `cairn-gui` gate**

```bash
cargo fmt --all --check --manifest-path cairn-gui/Cargo.toml
cargo clippy --locked --manifest-path cairn-gui/Cargo.toml --workspace --tests -- -D warnings
CAIRN_TEST_PG='host=/tmp port=5532 dbname=cairn_test' cargo test --locked --manifest-path cairn-gui/Cargo.toml --workspace
RUSTDOCFLAGS=-D warnings cargo doc --locked --no-deps --manifest-path cairn-gui/Cargo.toml --workspace
cargo deny --manifest-path cairn-gui/Cargo.toml check
```
Each judged on its own exit code. Expected: all exit 0.

- [ ] **Step 6: Prove the root tree is untouched**

```bash
git diff --name-only main... | grep '^crates/' || echo "no crates/ changes — as designed"
```
Expected: the `echo`. If anything under `crates/` appears, the root gate is owed and this slice's
constraint has been broken — stop and reconsider.

- [ ] **Step 7: Commit**

```bash
python3 scripts/check_closing_keywords.py
git add .github/workflows/rust.yml scripts/run-db-gated-tests.sh cairn-gui/cairn-gui-live/src/error.rs
git commit -m "ci(slice 2b): run the funnel's port suite where a database exists"
```

---

### Task 8: Say what was found, and hand over

**Files:**
- Modify: `docs/superpowers/specs/2026-09-20-registration-search-funnel-ui-design.md`
- Modify: `docs/HANDOVER.md`, `docs/ROADMAP.md`

- [ ] **Step 1: Add a dated revision note to the design page — never an edit**

The page's *Error handling* section says *"Register fails. The form keeps its values."* and
`port.rs` said the `Refused` variant would decide whether to restore. Task 1 concluded it does
not. Add a `> **Revised 2026-09-22 (slice 2b).**` block under *Error handling* stating what the
code settled, in the same voice as the two notes slice 2a added. If the build contradicted the
page anywhere else, add a note there too. **Never edit the original sentence away** — the ADR log's
own rule applied to a design page.

- [ ] **Step 2: Update HANDOVER and ROADMAP**

`⇒ NEXT` becomes slice **2c — the window** (commands, shell state, `--patient` optional, the
frontend, the JS/Rust drift-guard extension, the §1.2 end-to-end measurement, and narrowing
`main.rs`'s *"Writes are refused in this mode"* to **clinical** writes). Carry forward every
durable rule slice 2a recorded, plus what 2b adds:

- `DataError::Refused` exists and both arms restore the attestation; only the sentence differs.
- `cairn-gui-live` is where a port implementation that needs a database goes, and why.
- The P0001 rule now has three homes; the consolidation issue is #NNN.
- The `cairn-gui` tree now has a DB-gated suite; it runs in CI's `test` job and in
  `run-db-gated-tests.sh`, and `CAIRN_ALLOW_DB_SKIP=1` is set in the `gui` job on purpose.
- Whatever Task 6 discovered about the anyhow chain.

Prune both files toward 500 lines: condense, never drop an issue number.

- [ ] **Step 3: Commit and open the PR**

```bash
python3 scripts/check_closing_keywords.py
git add docs/
git commit -m "docs(slice 2b): the live ports are built, and 2c is the window"
git push -u origin feat/funnel-ui-slice-2b-live-ports
gh pr create --title "Funnel UI slice 2b: the live ports, and a refusal that is not an outage (#648)" --body "..."
```

The PR body names #648 as addressed, links the design page and this plan, and states what 2c
still owes — above all the §1.2 end-to-end measurement, which cannot be taken until a runnable
surface exists.

---

## Paper-parity benchmark (§1.2)

This slice builds no new clinical workflow: it gives the workflow the design page already
benchmarked its data path. The benchmark is restated rather than re-derived, because a plan that
silently inherited one would be the "enforced by taste" state #217 removed.

**Paper counterpart:** the registration desk and the alphabetical patient index drawer.

**Steps:**

| | Register a new patient | Find an existing chart |
|---|---|---|
| Paper acts (N) | 5 — ask details, flip drawer, take blank card, write it, file it | 3 — ask details, flip drawer, pull card |
| Architecture-forced (M) | 4 — type fragment, read list, complete the form, answer the prompt | 2 — type fragment, pick |
| UI bundling target (K) | 4 | 2 |

`M ≤ N` for both, so there is no architecture defect to file. **This slice does not move M.** It
adds no act: the ports it builds are called by acts the design already counted. The one thing it
could have added is a step and deliberately does not — a refusal now says *this cannot succeed as
typed* instead of inviting a retry, which removes a futile act rather than adding one.

**Time + cognitive load.** The end-to-end measurement this design owes is **slice 2c's**, because
it cannot be taken until a runnable surface exists; the design page assigns it there and this plan
does not move it. What 2b owes instead is the budget's precondition: the browse search must stay
inside §5.11's *no spinner*, and the port adds only a mutex acquisition and an error map to
`search_patients`, whose own floor was re-measured under #639 (under a second on Pi-class
hardware, on ~25 000 patients). Cognitive load is unchanged by this slice — nothing here reaches a
screen.

---

## Self-review

**Spec coverage.** The design's *Slicing* section assigns 2b: commands, shell state, frontend,
drift-guard extension, DB-gated attestation test, §1.2 measurement. This plan takes **the DB-gated
attestation test** and the data path under it, and **explicitly defers the rest to 2c** — a split
agreed with the maintainer before planning, on the DR 2a/2b/2c/2d precedent, because the ports are
independently testable and the measurement needs the window. *Architecture → Ports* is Tasks 4–5;
*Architecture →* the by-value `register` note is Task 5; *Error handling → Register fails / Step-3
search fails* is Tasks 1, 3 and 6; *Testing strategy → DB-gated* is Tasks 5–6. Not covered here,
carried to 2c: *Commands*, *Shell and frontend*, *Testing strategy → Mock port* (2a built it), and
the §1.2 measurement.

**Placeholders.** One deliberate under-determination, in Task 6 Step 2: which floor check the
refusal probe lands on. It is written as a decision procedure with both outcomes named and the
contract held fixed, because the answer depends on where `register_patient` validates first, and
guessing it in a plan would be worse than instructing the implementer to look. Task 5 Step 1
carries a similar honest note about the stored `displayed` shape, pointing at the file that is
authoritative. Neither defers a *decision* — only a lookup.

**Type consistency.** `DataError::Refused(String)` (Task 1) is matched in Tasks 3 and 6.
`LiveData::new(Client, SigningKey, String)` (Task 2) is called identically in Tasks 4, 5 and 6.
`refusal_is_deliberate`/`sqlstate_of`/`data_error_from` (Task 3) are used in Task 4's and Task 5's
`map_err`. `common::cs`/`connect`/`setup`/`stored_displayed`/`is_affirmative` are defined in Tasks
2, 4 and 5 and used consistently. `bound_for_prompt(&CandidateList) -> PromptList`,
`TokenStore::{record, take, commit, restore}` and `PromptList::as_list` match slice 2a's shipped
signatures.
