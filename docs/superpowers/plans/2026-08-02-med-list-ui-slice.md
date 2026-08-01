# Med-list UI slice (Tauri 2) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship Cairn's first runnable clinical surface — a Tauri 2 window showing one patient's
medication list, with whole-list sign-off as **one human gesture** ([#288](https://github.com/cairn-ehr/cairn-ehr/issues/288)) and per-row cease.

**Architecture:** A shared pure crate holds the medication read model and the sign-off targeting
rule, so "what the UI says it will sign" and "what actually gets signed" can never be two different
answers. `cairn-node` gains the first clinical *read* path over the existing projection views plus a
bundling orchestrator, both exposed as CLI verbs so the reference UI uses no privileged path. The
Tauri backend computes the whole view model in Rust; the webview renders it as semantic HTML.

**Tech Stack:** Rust 1.96 · tokio-postgres · Tauri 2 · vanilla TypeScript (no framework) ·
PostgreSQL 18 + `cairn_pgx`

**Design spec:** [`docs/superpowers/specs/2026-08-02-med-list-ui-slice-design.md`](../specs/2026-08-02-med-list-ui-slice-design.md)

## Three refinements to the approved spec, discovered while planning

State these to the steward before starting; none changes the slice's scope or its clinical contract.

1. **The read model and `sign_off_targets` move to a new shared pure crate**
   `crates/cairn-medication-view`, rather than living in the GUI tab crate (spec §3/§5). Reason: the
   CLI's `sign_off_medication_list` and the UI's "this row will be signed" badge **must** be the same
   rule. If the rule lived in the GUI, the CLI would re-derive it, and a divergence renders a green
   badge over an unsigned thread — a silent clinical falsehood. The crate is pure (no DB, no GUI), so
   the tab crate still tests instantly without a database driver.
2. **`list_patient_medications` returns ceased rows too**, marked `MedicationStatus::Ceased`.
   Paper-parity: a struck line stays visible on a drug chart; it does not vanish. Costs one extra
   query and makes "ceased rows are never sign-off targets" a real, testable rule instead of a
   vacuous one.
3. **The `MedWrite` port is dropped** (spec §3). Nothing consumes it: the Tauri backend calls
   `cairn-node` directly in async commands, and a sync trait wrapping async writes buys nothing.
   `ClinicalData::medications()` stays, because it gives the window a `--mock` mode that runs with no
   database — which is what the operator accessibility pass and the timing runbook need on a laptop.

## Global Constraints

- **Licence:** AGPL-3.0-only. Every new dependency must be AGPL-3.0-compatible and checked *before*
  it is added (`cargo deny check licenses`). Tauri 2 is MIT/Apache-2.0 — compatible.
- **Rust:** edition 2021, `rust-version = "1.96"`, pinned by `rust-toolchain.toml`. CI runs
  `-D warnings`; `cargo fmt --check` gates both cargo trees.
- **TDD:** the failing test is written and *seen to fail* before the implementation, every task.
- **Inline documentation** aimed at a junior contributor: *why* it exists and *how* it fits, not what
  the next line does.
- **File size:** aim under 500 lines; split by responsibility when a file grows past it.
- **No hard-coded cryptographic material in tests** (house rule 6): derive keys at runtime via
  `generate_key()` or `std::array::from_fn`, never a byte-array or string literal.
- **DB-gated tests** self-skip without `$CAIRN_TEST_PG` and serialize cluster-wide via
  `db::test_serial_guard`. Local: `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test"`.
- **One-way dependency:** `cairn-gui/*` may depend on `crates/*`; nothing in `crates/*` may ever
  depend on `cairn-gui/*`.
- **Projections are read, never re-derived.** Staleness comes from
  `medication_thread_attestation.stale`; the UI never recomputes it.

## Shipping order if the slice cannot land whole

This is the largest single slice in the clinical build so far. The task order is also the fallback
order: **Tasks 1–4 are independently valuable and ship on their own** — the first clinical read path,
whole-list sign-off, and both CLI verbs, usable without any UI and reusable by the future native API.
Tasks 5–12 build the window on top. If the session ends mid-plan, say exactly which tasks landed in
HANDOVER rather than describing the slice as done.

## File Structure

```
crates/cairn-medication-view/          NEW  pure shared read model + targeting rule
    Cargo.toml
    src/lib.rs                              re-exports
    src/row.rs                              MedicationRow, MemberVouch, VouchState, MedicationStatus
    src/targeting.rs                        sign_off_targets

crates/cairn-node/src/
    medication/read.rs                 NEW  SQL -> Vec<MedicationRow> (first clinical read path)
    medication/signoff.rs              NEW  sign_off_medication_list (N HLCs, ONE txn)
    medication/mod.rs                  MOD  register + re-export the two modules
    ui_timing.rs                       NEW  fold_sample, size_bucket, record_gesture
    lib.rs                             MOD  pub mod ui_timing
    db.rs                              MOD  append db/044 to SCHEMA
    main.rs                            MOD  +MedicationList +MedicationSignOff

crates/cairn-event/src/schema_generation.rs   MOD  SCHEMA_GENERATION 43 -> 44
db/044_ui_gesture_timing.sql           NEW  node-local aggregate-only timing table

crates/cairn-node/tests/
    medication_read.rs                 NEW  DB-gated read-path tests
    medication_signoff.rs              NEW  DB-gated bundling tests
    ui_timing.rs                       NEW  DB-gated aggregate round-trip

cairn-gui/
    Cargo.toml                         MOD  members: -nothing, +tab-medications, +tauri
    cairn-gui-shell/Cargo.toml         MOD  drop iced dep + `gui` feature + the bin
    cairn-gui-shell/src/app.rs         DEL  iced view layer
    cairn-gui-shell/src/a11y_dump.rs   DEL  iced-bound dump
    cairn-gui-shell/src/bin/gui.rs     DEL  iced binary
    cairn-gui-data/src/port.rs         MOD  ClinicalData += medications()
    cairn-gui-data/src/mock.rs         MOD  fixture medication rows
    cairn-gui-tabs/cairn-gui-tab-medications/   NEW  MedListView, build_view, Semantic impl
    cairn-gui-tauri/                   NEW  Tauri backend + src-ui/ (vanilla TS)
        src/state.rs                        session key + 15-min idle re-lock
        src/commands.rs                     med_list / unlock / sign_off / cease
        src/main.rs
        src-ui/index.html, main.ts, style.css
        results/RUNBOOK.md, TEMPLATE.md
```

---

## Task 1: The shared read model and the sign-off targeting rule

**Files:**
- Create: `crates/cairn-medication-view/Cargo.toml`
- Create: `crates/cairn-medication-view/src/lib.rs`
- Create: `crates/cairn-medication-view/src/row.rs`
- Create: `crates/cairn-medication-view/src/targeting.rs`
- Modify: `Cargo.toml` (root workspace `members`)

**Interfaces:**
- Consumes: nothing.
- Produces: `MedicationStatus::{Active, Ceased}`, `VouchState::{Absent, Fresh{by:String}, Stale{by:String}}`
  with `VouchState::needs_signature(&self) -> bool`, `MemberVouch { medication_id: Uuid, vouch: VouchState }`,
  `MedicationRow` (fields listed in Step 3), and
  `sign_off_targets(rows: &[MedicationRow]) -> Vec<Uuid>`.

- [ ] **Step 1: Create the crate manifest**

`crates/cairn-medication-view/Cargo.toml`:

```toml
[package]
name = "cairn-medication-view"
version = "0.1.0"
edition.workspace = true
rust-version.workspace = true
license.workspace = true
repository.workspace = true
publish = false

[dependencies]
uuid = { version = "1", features = ["v7", "serde"] }
serde = { version = "1", features = ["derive"] }
```

- [ ] **Step 2: Add the crate to the root workspace**

In `/Cargo.toml`, add to `members` (keep the existing entries and the `exclude` line untouched):

```toml
members = [
    "crates/cairn-event",
    "crates/cairn-medication-view",
    "crates/cairn-sync",
    "crates/cairn-node", # cairn-node added in Task 6
]
```

- [ ] **Step 3: Write the failing tests for the model and the targeting rule**

`crates/cairn-medication-view/src/targeting.rs` — tests first, at the bottom of the file:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::row::{MedicationRow, MedicationStatus, MemberVouch, VouchState};

    /// Deterministic uuids without a random source: `Uuid::from_u128` makes each id
    /// readable in a failure message and stable across runs.
    fn uid(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    fn row(group: u128, status: MedicationStatus, members: Vec<MemberVouch>) -> MedicationRow {
        MedicationRow {
            group_id: uid(group),
            patient_id: uid(999),
            term: "metformin".into(),
            coding_display: None,
            formulation: None,
            dose_amount: None,
            dose_unit: None,
            sig: None,
            started_value: None,
            started_precision: None,
            status,
            members,
            reconciliation_flagged: false,
            coding_conflict: false,
        }
    }

    fn member(id: u128, vouch: VouchState) -> MemberVouch {
        MemberVouch { medication_id: uid(id), vouch }
    }

    #[test]
    fn an_unvouched_thread_is_a_target() {
        let rows = vec![row(1, MedicationStatus::Active, vec![member(1, VouchState::Absent)])];
        assert_eq!(sign_off_targets(&rows), vec![uid(1)]);
    }

    #[test]
    fn a_stale_vouch_is_a_target() {
        let rows = vec![row(1, MedicationStatus::Active,
            vec![member(1, VouchState::Stale { by: "abc".into() })])];
        assert_eq!(sign_off_targets(&rows), vec![uid(1)]);
    }

    /// THE load-bearing case (#288, drug-chart semantics): another clinician's current
    /// signature on a drug line is left exactly as it is. Signing it over would silently
    /// move responsibility for that drug from them to the current user.
    #[test]
    fn a_fresh_vouch_by_someone_else_is_left_alone() {
        let rows = vec![row(1, MedicationStatus::Active,
            vec![member(1, VouchState::Fresh { by: "dr_b".into() })])];
        assert!(sign_off_targets(&rows).is_empty());
    }

    /// A reconciled group is ONE displayed row over several threads; only the members
    /// that actually need a signature are signed.
    #[test]
    fn a_mixed_freshness_group_targets_only_its_needy_members() {
        let rows = vec![row(1, MedicationStatus::Active, vec![
            member(1, VouchState::Fresh { by: "dr_b".into() }),
            member(2, VouchState::Stale { by: "dr_b".into() }),
            member(3, VouchState::Absent),
        ])];
        assert_eq!(sign_off_targets(&rows), vec![uid(2), uid(3)]);
    }

    /// A struck line on a paper chart is not re-signed. Ceased rows stay VISIBLE
    /// (refinement 2) but are never targets.
    #[test]
    fn ceased_rows_are_never_targets() {
        let rows = vec![row(1, MedicationStatus::Ceased, vec![member(1, VouchState::Absent)])];
        assert!(sign_off_targets(&rows).is_empty());
    }

    #[test]
    fn an_empty_list_yields_no_targets() {
        assert!(sign_off_targets(&[]).is_empty());
    }

    /// The order is what assigns HLCs in the orchestrator, so it must not depend on
    /// row order or on how many groups a thread appears under.
    #[test]
    fn targets_are_sorted_and_deduplicated() {
        let rows = vec![
            row(9, MedicationStatus::Active, vec![member(9, VouchState::Absent)]),
            row(2, MedicationStatus::Active, vec![member(2, VouchState::Absent),
                                                  member(2, VouchState::Absent)]),
        ];
        assert_eq!(sign_off_targets(&rows), vec![uid(2), uid(9)]);
    }

    #[test]
    fn needs_signature_covers_absent_and_stale_only() {
        assert!(VouchState::Absent.needs_signature());
        assert!(VouchState::Stale { by: "x".into() }.needs_signature());
        assert!(!VouchState::Fresh { by: "x".into() }.needs_signature());
    }
}
```

- [ ] **Step 4: Run the tests to verify they fail**

Run: `cargo test -p cairn-medication-view`
Expected: FAIL — the crate does not compile (`row` and `sign_off_targets` do not exist).

- [ ] **Step 5: Write the model**

`crates/cairn-medication-view/src/row.rs`:

```rust
//! The medication read model shared by the node's read path, the CLI, and the UI.
//!
//! WHY A SHARED CRATE. Two consumers must agree on one question — *which threads does a
//! sign-off gesture attest?* `cairn-node`'s orchestrator answers it to decide what to
//! sign; the UI answers it to tell the clinician what is about to be signed. If those
//! were two implementations, a divergence would put a green "signed" badge over a thread
//! nobody signed. So the model and the rule live here, and both sides depend on it.
//!
//! This crate is deliberately pure: no database driver, no GUI toolkit. That is what lets
//! the GUI tab crate test in milliseconds without Postgres in the build tree.
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Whether a displayed medication group is still being taken.
///
/// Ceased rows are RETAINED in the list, not filtered out: a struck line stays visible on
/// a paper drug chart, and dropping it would lose that parity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MedicationStatus {
    Active,
    Ceased,
}

/// The ADR-0049 sign-off state of ONE medication thread.
///
/// `by` is the attester's hex key id, as recorded in `medication_attestation.attester_kid`.
/// Staleness is NOT computed here — it is read from `medication_thread_attestation.stale`,
/// which the database derives from the set-commitment compare. A second implementation of
/// staleness would be a second answer to a safety question.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum VouchState {
    /// No attestation on this thread at all.
    Absent,
    /// A current vouch.
    Fresh { by: String },
    /// A vouch whose set-commitment no longer matches the thread's content.
    Stale { by: String },
}

impl VouchState {
    /// True when a sign-off gesture must (re-)vouch this thread.
    pub fn needs_signature(&self) -> bool {
        matches!(self, VouchState::Absent | VouchState::Stale { .. })
    }

    /// The attester's key id, when there is one.
    pub fn attester(&self) -> Option<&str> {
        match self {
            VouchState::Absent => None,
            VouchState::Fresh { by } | VouchState::Stale { by } => Some(by),
        }
    }
}

/// One member thread of a displayed row, with the vouch that thread carries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemberVouch {
    pub medication_id: Uuid,
    pub vouch: VouchState,
}

/// One displayed row = one medication GROUP.
///
/// A group is what `patient_medication_current` emits: reconciled duplicate threads
/// (ADR-0047) collapse into a single clinical statement. Attestation, however, is
/// per-THREAD, so the row carries its members and each member's vouch. That group/thread
/// asymmetry is the most defect-prone seam in this slice — see the tests in
/// `targeting.rs` and `crates/cairn-node/tests/medication_read.rs`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MedicationRow {
    pub group_id: Uuid,
    pub patient_id: Uuid,
    /// The free-text term as asserted — may legitimately be vague ("little white pill").
    pub term: String,
    /// The ADR-0059 coded display name, when the drug has been coded.
    pub coding_display: Option<String>,
    pub formulation: Option<String>,
    pub dose_amount: Option<String>,
    pub dose_unit: Option<String>,
    pub sig: Option<String>,
    pub started_value: Option<String>,
    pub started_precision: Option<String>,
    pub status: MedicationStatus,
    pub members: Vec<MemberVouch>,
    /// This group shares a duplicate key with another un-reconciled group
    /// (`patient_medication_reconciliation_flag`). Advisory worklist, never auto-resolved.
    pub reconciliation_flagged: bool,
    /// Two different drug anchors inside one reconciled group
    /// (`medication_group_coding_conflict`) — a possible mis-reconciliation.
    pub coding_conflict: bool,
}
```

- [ ] **Step 6: Write the targeting rule**

At the top of `crates/cairn-medication-view/src/targeting.rs`, above the tests written in Step 3:

```rust
//! The one definition of what a single sign-off gesture attests (#288).
use crate::row::{MedicationRow, MedicationStatus};
use uuid::Uuid;

/// Which threads a single sign-off gesture attests.
///
/// Paper drug-chart semantics: each drug line carries the signature of the person
/// responsible for THAT drug, so a thread already holding a non-stale vouch keeps its
/// existing signatory untouched. Returns THREAD ids, not group ids, because ADR-0049
/// attestation is per-thread.
///
/// The result is sorted and deduplicated. That is not cosmetic: the orchestrator mints one
/// HLC per target in this order, so an unstable order would make two runs over the same
/// list assign different HLCs to the same threads.
pub fn sign_off_targets(rows: &[MedicationRow]) -> Vec<Uuid> {
    let mut targets: Vec<Uuid> = rows
        .iter()
        // A struck line is not re-signed. Ceased rows stay visible for parity with a
        // paper chart, but they are never targets.
        .filter(|row| row.status == MedicationStatus::Active)
        .flat_map(|row| row.members.iter())
        .filter(|member| member.vouch.needs_signature())
        .map(|member| member.medication_id)
        .collect();
    targets.sort();
    targets.dedup();
    targets
}
```

`crates/cairn-medication-view/src/lib.rs`:

```rust
//! Shared, pure medication read model + the sign-off targeting rule. See `row.rs` for why
//! this is its own crate rather than living in the node or the GUI.
pub mod row;
pub mod targeting;

pub use row::{MedicationRow, MedicationStatus, MemberVouch, VouchState};
pub use targeting::sign_off_targets;
```

- [ ] **Step 7: Run the tests to verify they pass**

Run: `cargo test -p cairn-medication-view`
Expected: PASS, 8 tests.

- [ ] **Step 8: Check formatting, lints and licences**

Run: `cargo fmt --check && cargo clippy -p cairn-medication-view -- -D warnings && cargo deny check licenses`
Expected: clean.

- [ ] **Step 9: Commit**

```bash
git add Cargo.toml Cargo.lock crates/cairn-medication-view
git commit -m "feat(#288): the shared medication read model and sign-off targeting rule

One definition of which threads a sign-off gesture attests, depended on by both
the node orchestrator and the UI badge. Two implementations would eventually
disagree, and the visible symptom is a green 'signed' badge over an unsigned
thread. Paper drug-chart semantics: a thread already holding a non-stale vouch
keeps its existing signatory."
```

---

## Task 2: The first clinical read path

**Files:**
- Create: `crates/cairn-node/src/medication/read.rs`
- Modify: `crates/cairn-node/src/medication/mod.rs`
- Modify: `crates/cairn-node/Cargo.toml` (add the `cairn-medication-view` dependency)
- Test: `crates/cairn-node/tests/medication_read.rs`

**Interfaces:**
- Consumes: `cairn_medication_view::{MedicationRow, MedicationStatus, MemberVouch, VouchState}`.
- Produces: `cairn_node::medication::read::list_patient_medications(client, patient) -> anyhow::Result<Vec<MedicationRow>>`,
  generic over `impl tokio_postgres::GenericClient + Sync` so it can read inside a caller's
  transaction (Task 3 depends on this).

- [ ] **Step 1: Add the dependency**

In `crates/cairn-node/Cargo.toml`, under `[dependencies]`:

```toml
cairn-medication-view = { path = "../cairn-medication-view" }
```

- [ ] **Step 2: Add a shared medication test setup to `tests/common/mod.rs`**

Two suites in this plan need the identical setup. `crates/cairn-node/tests/common/mod.rs` is this
directory's existing home for scaffolding two suites would otherwise write identically (#120), so it
goes there once rather than being copy-pasted — the drift that file exists to prevent.

Append to `crates/cairn-node/tests/common/mod.rs`:

```rust
/// Truncate the event log, the custody plane and every medication projection, then enroll
/// one DEVICE actor (mints medication threads) and one HUMAN actor (signs and attests).
/// Returns `(device_sk, device_kid, human_sk, human_kid)`.
///
/// Each medication table is truncated behind a `to_regclass` guard because it is created
/// by a later migration than the core clinical tables: the guard keeps one shared helper
/// correct on a database migrated only partway, instead of erroring on a table that does
/// not exist yet. Same discipline as `setup` above.
pub async fn medication_setup(c: &Client) -> (SigningKey, String, SigningKey, String) {
    c.batch_execute(
        "TRUNCATE event_log, actor_event, patient_chart, \
         node_unwrap_key, event_dek, event_clear, erasure_shred_log CASCADE",
    )
    .await
    .unwrap();
    c.batch_execute(
        "DO $$ BEGIN \
           IF to_regclass('public.medication_statement') IS NOT NULL THEN TRUNCATE medication_statement; END IF; \
           IF to_regclass('public.medication_cessation') IS NOT NULL THEN TRUNCATE medication_cessation; END IF; \
           IF to_regclass('public.medication_dose_event') IS NOT NULL THEN TRUNCATE medication_dose_event; END IF; \
           IF to_regclass('public.medication_dose_correction') IS NOT NULL THEN TRUNCATE medication_dose_correction; END IF; \
           IF to_regclass('public.medication_reconciliation') IS NOT NULL THEN TRUNCATE medication_reconciliation; END IF; \
           IF to_regclass('public.medication_group_member') IS NOT NULL THEN TRUNCATE medication_group_member; END IF; \
           IF to_regclass('public.medication_projection_flag') IS NOT NULL THEN TRUNCATE medication_projection_flag; END IF; \
           IF to_regclass('public.medication_coding') IS NOT NULL THEN TRUNCATE medication_coding; END IF; \
           IF to_regclass('public.medication_attestation') IS NOT NULL THEN TRUNCATE medication_attestation; END IF; \
         END $$;",
    )
    .await
    .unwrap();
    let (sk_d, kid_d) = generate_key().unwrap();
    let (sk_h, kid_h) = generate_key().unwrap();
    c.execute(
        "SELECT enroll_actor('device', '{\"role\":\"registration-desk\"}', $1)",
        &[&kid_d],
    )
    .await
    .unwrap();
    c.execute(
        "SELECT enroll_actor('human', '{\"role\":\"clinician\"}', $1)",
        &[&kid_h],
    )
    .await
    .unwrap();
    // ADR-0052: register THIS node's unwrap key (derived from the device key) so the strict
    // door can wrap every sealed event's DEK into custody — attestation events are
    // clinical.* and born-sealed too. A node has exactly ONE unwrap key regardless of who
    // signs individual events; deriving it from the human key would collide on the
    // node_unwrap_key singleton.
    let secret = cairn_event::seal::derive_unwrap_secret(&sk_d.to_bytes());
    c.execute(
        "SELECT cairn_register_unwrap_key($1)",
        &[&cairn_event::seal::unwrap_public(&secret).as_slice()],
    )
    .await
    .unwrap();
    (sk_d, kid_d, sk_h, kid_h)
}
```

- [ ] **Step 3: Write the failing DB-gated tests**

`crates/cairn-node/tests/medication_read.rs`:

```rust
//! The first clinical READ path (#288 med-list slice): `list_patient_medications` over the
//! existing medication projections.
//!
//! The group/thread asymmetry is what these tests exist for. `patient_medication_current`
//! emits one row per GROUP (reconciled duplicates collapse, ADR-0047) while attestation is
//! per THREAD (ADR-0049). Every test below pins one way that asymmetry can be got wrong.
//!
//! DB-gated on $CAIRN_TEST_PG, serialized cluster-wide via db::test_serial_guard. Key
//! material is minted at runtime (house rule 6).
mod common;

use cairn_event::SigningKey;
use cairn_medication_view::{MedicationStatus, VouchState};
use cairn_node::db;
use cairn_node::medication::read::list_patient_medications;
use cairn_node::medication::{
    assert_medication, attest_medication_thread, cease_medication, reconcile_medications,
    AssertMedicationInput, AttestParams, CeaseMedicationInput, ReconcileInput,
};
use common::{cs, medication_setup as setup};
use tokio_postgres::Client;
use uuid::Uuid;

/// Assert one medication and return its thread id.
async fn assert_one(
    c: &mut Client,
    sk: &SigningKey,
    kid: &str,
    origin: &str,
    patient: Uuid,
    term: &str,
) -> Uuid {
    assert_medication(
        c, sk, kid, origin, patient,
        &AssertMedicationInput {
            term,
            coding: None,
            formulation: None,
            dose_amount: Some("500"),
            dose_unit: Some("mg"),
            sig: None,
            info_source: "patient",
            started: None,
            started_precision: None,
        },
        None, None,
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn a_single_unvouched_medication_reads_as_absent() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return };
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let (sk, kid, _hsk, _hkid) = setup(&c).await;
    let patient = Uuid::now_v7();

    let thread = assert_one(&mut c, &sk, &kid, "origin-a", patient, "metformin").await;

    let rows = list_patient_medications(&c, patient).await.unwrap();
    assert_eq!(rows.len(), 1, "one assert, one displayed row");
    assert_eq!(rows[0].term, "metformin");
    assert_eq!(rows[0].status, MedicationStatus::Active);
    assert_eq!(rows[0].members.len(), 1);
    assert_eq!(rows[0].members[0].medication_id, thread);
    assert_eq!(rows[0].members[0].vouch, VouchState::Absent);
}

#[tokio::test]
async fn an_attested_thread_reads_as_fresh_with_its_attester() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return };
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let (sk, kid, hsk, hkid) = setup(&c).await;
    let patient = Uuid::now_v7();

    let thread = assert_one(&mut c, &sk, &kid, "origin-a", patient, "metformin").await;
    let params = AttestParams { human_sk: &hsk, human_kid: &hkid, basis: None, note: None };
    attest_medication_thread(&mut c, &sk, "origin-a", &params, patient, thread).await.unwrap();

    let rows = list_patient_medications(&c, patient).await.unwrap();
    assert_eq!(rows[0].members[0].vouch, VouchState::Fresh { by: hkid.clone() });
}

/// A reconciled pair is ONE row over TWO member threads — the group/thread asymmetry.
#[tokio::test]
async fn a_reconciled_pair_reads_as_one_row_with_two_members() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return };
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let (sk, kid, _hsk, _hkid) = setup(&c).await;
    let patient = Uuid::now_v7();

    let a = assert_one(&mut c, &sk, &kid, "origin-a", patient, "metformin").await;
    let b = assert_one(&mut c, &sk, &kid, "origin-a", patient, "Metformin XR").await;
    reconcile_medications(
        &mut c, &sk, &kid, "origin-a",
        &ReconcileInput { patient, thread_a: a, thread_b: b, note: None },
        None, None,
    )
    .await
    .unwrap();

    let rows = list_patient_medications(&c, patient).await.unwrap();
    assert_eq!(rows.len(), 1, "a reconciled pair collapses to ONE displayed row");
    let mut members: Vec<Uuid> = rows[0].members.iter().map(|m| m.medication_id).collect();
    members.sort();
    let mut expected = vec![a, b];
    expected.sort();
    assert_eq!(members, expected, "both threads are members of the one row");
}

/// A ceased medication stays VISIBLE, marked ceased — a struck line on a paper chart is
/// not erased (refinement 2 of the plan).
#[tokio::test]
async fn a_ceased_medication_is_retained_and_marked_ceased() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return };
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let (sk, kid, _hsk, _hkid) = setup(&c).await;
    let patient = Uuid::now_v7();

    let thread = assert_one(&mut c, &sk, &kid, "origin-a", patient, "metformin").await;
    cease_medication(
        &mut c, &sk, &kid, "origin-a", patient, thread,
        &CeaseMedicationInput { stopped: None, stopped_precision: None, reason: Some("rash") },
        None, None,
    )
    .await
    .unwrap();

    let rows = list_patient_medications(&c, patient).await.unwrap();
    assert_eq!(rows.len(), 1, "a ceased drug is still on the chart");
    assert_eq!(rows[0].status, MedicationStatus::Ceased);
}

#[tokio::test]
async fn another_patients_medications_are_not_returned() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return };
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let (sk, kid, _hsk, _hkid) = setup(&c).await;
    let mine = Uuid::now_v7();
    let theirs = Uuid::now_v7();

    assert_one(&mut c, &sk, &kid, "origin-a", theirs, "warfarin").await;

    assert!(list_patient_medications(&c, mine).await.unwrap().is_empty());
}

#[tokio::test]
async fn a_patient_with_no_medications_reads_as_an_empty_list() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return };
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let _ = setup(&c).await;

    assert!(list_patient_medications(&c, Uuid::now_v7()).await.unwrap().is_empty());
}
```

- [ ] **Step 4: Run the tests to verify they fail**

Run: `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test -p cairn-node --test medication_read`
Expected: FAIL to compile — `cairn_node::medication::read` does not exist.

- [ ] **Step 5: Write the read path**

`crates/cairn-node/src/medication/read.rs`:

```rust
//! Cairn's first clinical READ path (#288 med-list slice).
//!
//! Everything before this slice authored events; nothing read clinical content back out
//! in Rust. This module maps the existing medication projections into the shared
//! `cairn_medication_view` model — and it is the ONLY such mapping: the med-list UI reads
//! through it today, and the future native API (ADR-0023, Phase 8) is expected to wrap
//! this same function rather than re-derive the joins.
//!
//! WHY THREE SMALL QUERIES AND NOT ONE JOIN. The list, the per-thread vouches, and the two
//! advisory flags answer three different questions over three different grains (group,
//! thread, worklist). One join would need two levels of aggregation and would be far
//! harder for a reviewer to check against the view definitions in db/031-034. Three plain
//! queries plus an explicit assembly step in Rust is the reviewer-legible shape §9 asks
//! for, and each query is independently checkable against its view.
//!
//! Generic over `GenericClient` so a caller can read through an open transaction — the
//! sign-off orchestrator (`signoff.rs`) relies on that to compute its targets in the same
//! snapshot it writes in.
use cairn_medication_view::{MedicationRow, MedicationStatus, MemberVouch, VouchState};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

/// Read one patient's medication list: current drugs AND ceased ones.
///
/// Ceased rows are retained deliberately. A struck line stays visible on a paper drug
/// chart; dropping it here would lose that parity and would hide a drug the clinician may
/// need to see was recently stopped. They carry `MedicationStatus::Ceased` and are never
/// sign-off targets (`cairn_medication_view::sign_off_targets`).
pub async fn list_patient_medications(
    client: &(impl tokio_postgres::GenericClient + Sync),
    patient: Uuid,
) -> anyhow::Result<Vec<MedicationRow>> {
    let members = read_member_vouches(client, patient).await?;
    let reconciliation_flagged = read_reconciliation_flagged_groups(client, patient).await?;
    let coding_conflict = read_coding_conflict_groups(client, patient).await?;

    // `patient_medication_current` and `_past` carry the SAME column set by design (a
    // db/033 replay-safety constraint), so one shared row mapper serves both.
    let mut rows = Vec::new();
    for (sql, status) in [
        (CURRENT_SQL, MedicationStatus::Active),
        (PAST_SQL, MedicationStatus::Ceased),
    ] {
        for db_row in client.query(sql, &[&patient]).await? {
            let group_id: Uuid = db_row.get("medication_id");
            rows.push(MedicationRow {
                group_id,
                patient_id: db_row.get("patient_id"),
                term: db_row.get("term"),
                coding_display: db_row.get("coding_display"),
                formulation: db_row.get("formulation"),
                dose_amount: db_row.get("dose_amount"),
                dose_unit: db_row.get("dose_unit"),
                sig: db_row.get("sig"),
                started_value: db_row.get("started_value"),
                started_precision: db_row.get("started_precision"),
                status,
                members: members.get(&group_id).cloned().unwrap_or_default(),
                reconciliation_flagged: reconciliation_flagged.contains(&group_id),
                coding_conflict: coding_conflict.contains(&group_id),
            });
        }
    }
    // Stable display order: coded/asserted term, then the group id as the tiebreak. Sorted
    // in Rust rather than SQL so the order cannot depend on the database's collation
    // (ADR-0045 — a locale-dependent ORDER BY is a node-local property).
    rows.sort_by(|a, b| {
        a.term
            .as_bytes()
            .cmp(b.term.as_bytes())
            .then_with(|| a.group_id.cmp(&b.group_id))
    });
    Ok(rows)
}

const CURRENT_SQL: &str = "SELECT medication_id, patient_id, term, formulation, dose_amount, \
     dose_unit, sig, started_value, started_precision, coding_display \
     FROM patient_medication_current WHERE patient_id = $1";

const PAST_SQL: &str = "SELECT medication_id, patient_id, term, formulation, dose_amount, \
     dose_unit, sig, started_value, started_precision, coding_display \
     FROM patient_medication_past WHERE patient_id = $1";

/// Every locally-known thread for this patient, grouped by the row it displays under,
/// carrying the ADR-0049 vouch it holds.
///
/// The LEFT JOIN is what makes an unattested thread readable at all: it produces a row
/// with a NULL attester, which maps to `VouchState::Absent`. `stale` is read, never
/// recomputed — db/034 derives it from the set-commitment compare.
async fn read_member_vouches(
    client: &(impl tokio_postgres::GenericClient + Sync),
    patient: Uuid,
) -> anyhow::Result<HashMap<Uuid, Vec<MemberVouch>>> {
    let sql = "SELECT g.group_id, g.medication_id, a.attester_kid, a.stale \
               FROM medication_thread_group g \
               LEFT JOIN medication_thread_attestation a ON a.medication_id = g.medication_id \
               WHERE g.patient_id = $1 \
               ORDER BY g.group_id, g.medication_id";
    let mut out: HashMap<Uuid, Vec<MemberVouch>> = HashMap::new();
    for row in client.query(sql, &[&patient]).await? {
        let attester: Option<String> = row.get("attester_kid");
        let stale: Option<bool> = row.get("stale");
        let vouch = match (attester, stale) {
            (Some(by), Some(true)) => VouchState::Stale { by },
            (Some(by), _) => VouchState::Fresh { by },
            (None, _) => VouchState::Absent,
        };
        out.entry(row.get("group_id"))
            .or_default()
            .push(MemberVouch { medication_id: row.get("medication_id"), vouch });
    }
    Ok(out)
}

/// Groups touched by an un-reconciled-duplicate flag.
///
/// `patient_medication_reconciliation_flag` reports THREAD ids spanning more than one
/// group, so every group those threads display under is flagged — that is exactly the
/// pair (or set) the clinician is being asked to look at.
async fn read_reconciliation_flagged_groups(
    client: &(impl tokio_postgres::GenericClient + Sync),
    patient: Uuid,
) -> anyhow::Result<HashSet<Uuid>> {
    let sql = "SELECT DISTINCT g.group_id \
               FROM patient_medication_reconciliation_flag f \
               CROSS JOIN LATERAL unnest(f.medication_ids) AS t(medication_id) \
               JOIN medication_thread_group g ON g.medication_id = t.medication_id \
               WHERE f.patient_id = $1";
    Ok(client
        .query(sql, &[&patient])
        .await?
        .iter()
        .map(|r| r.get("group_id"))
        .collect())
}

/// Groups whose members carry two different drug anchors (ADR-0059 decision 5) — a
/// possible mis-reconciliation. The view is not patient-scoped, so it is joined through
/// `medication_thread_group` to scope it to this chart.
async fn read_coding_conflict_groups(
    client: &(impl tokio_postgres::GenericClient + Sync),
    patient: Uuid,
) -> anyhow::Result<HashSet<Uuid>> {
    let sql = "SELECT DISTINCT cc.group_id \
               FROM medication_group_coding_conflict cc \
               JOIN medication_thread_group g ON g.group_id = cc.group_id \
               WHERE g.patient_id = $1";
    Ok(client
        .query(sql, &[&patient])
        .await?
        .iter()
        .map(|r| r.get("group_id"))
        .collect())
}
```

In `crates/cairn-node/src/medication/mod.rs`, add `pub mod read;` next to the existing `mod`
declarations (public, because the GUI and the CLI both call it by path).

- [ ] **Step 6: Run the tests to verify they pass**

Run: `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test -p cairn-node --test medication_read`
Expected: PASS, 6 tests.

- [ ] **Step 7: Commit**

```bash
git add crates/cairn-node/src/medication/read.rs crates/cairn-node/src/medication/mod.rs \
        crates/cairn-node/Cargo.toml crates/cairn-node/tests/common/mod.rs \
        crates/cairn-node/tests/medication_read.rs Cargo.lock
git commit -m "feat(#288): the first clinical read path

Nothing before this slice read clinical content back out in Rust — the
medication projections were exercised only in tests. list_patient_medications
maps them into the shared view model, and it is the only such mapping: the
future native API is expected to wrap it rather than re-derive the joins.

Ceased rows are retained and marked, not filtered: a struck line stays visible
on a paper drug chart."
```

---

## Task 3: Whole-list sign-off — one gesture, one transaction

**Files:**
- Create: `crates/cairn-node/src/medication/signoff.rs`
- Modify: `crates/cairn-node/src/medication/mod.rs`
- Test: `crates/cairn-node/tests/medication_signoff.rs`

**Interfaces:**
- Consumes: `read::list_patient_medications`, `cairn_medication_view::sign_off_targets`,
  `attestation::attest_thread_in_tx`, `AttestParams`, `sealed_submit::ensure_unwrap_key`,
  `db::next_hlc`.
- Produces: `SignOffOutcome { attested: Vec<Uuid>, event_ids: Vec<Uuid> }` and
  `sign_off_medication_list(client, node_sk, node_origin, params, patient) -> anyhow::Result<SignOffOutcome>`.

- [ ] **Step 1: Write the failing DB-gated tests**

`crates/cairn-node/tests/medication_signoff.rs`:

```rust
//! Whole-list sign-off (#288): ONE human gesture attests every thread on the chart whose
//! vouch is absent or stale, in ONE transaction.
//!
//! The N per-thread attestations are a cryptographic artifact of ADR-0049's commitment
//! model, not N human acts: `attest_thread_in_tx` takes an already-unsealed key by
//! reference, so one unseal and one transaction cover all N.
//!
//! DB-gated on $CAIRN_TEST_PG, serialized via db::test_serial_guard. Runtime key material.
mod common;

use cairn_event::{generate_key, SigningKey};
use cairn_node::db;
use cairn_node::medication::signoff::sign_off_medication_list;
use cairn_node::medication::{
    assert_medication, attest_medication_thread, cease_medication, AssertMedicationInput,
    AttestParams, CeaseMedicationInput,
};
use common::{cs, medication_setup as setup};
use tokio_postgres::Client;
use uuid::Uuid;

async fn assert_one(
    c: &mut Client, sk: &SigningKey, kid: &str, origin: &str, patient: Uuid, term: &str,
) -> Uuid {
    assert_medication(
        c, sk, kid, origin, patient,
        &AssertMedicationInput {
            term, coding: None, formulation: None,
            dose_amount: Some("500"), dose_unit: Some("mg"), sig: None,
            info_source: "patient", started: None, started_precision: None,
        },
        None, None,
    ).await.unwrap()
}

/// How many attestation rows exist for a thread.
async fn attestation_count(c: &Client, thread: Uuid) -> i64 {
    c.query_one(
        "SELECT count(*) FROM medication_attestation WHERE medication_id = $1",
        &[&thread],
    ).await.unwrap().get(0)
}

#[tokio::test]
async fn one_gesture_attests_every_unvouched_thread() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return };
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let (sk, kid, hsk, hkid) = setup(&c).await;
    let patient = Uuid::now_v7();

    let a = assert_one(&mut c, &sk, &kid, "origin-a", patient, "metformin").await;
    let b = assert_one(&mut c, &sk, &kid, "origin-a", patient, "amlodipine").await;
    let d = assert_one(&mut c, &sk, &kid, "origin-a", patient, "atorvastatin").await;

    let params = AttestParams { human_sk: &hsk, human_kid: &hkid, basis: None, note: None };
    let out = sign_off_medication_list(&mut c, &sk, "origin-a", &params, patient).await.unwrap();

    assert_eq!(out.attested.len(), 3, "one gesture, three threads");
    assert_eq!(out.event_ids.len(), 3, "one attestation event per thread");
    for t in [a, b, d] {
        assert_eq!(attestation_count(&c, t).await, 1, "thread {t} vouched exactly once");
    }
}

/// THE #288 contract: another clinician's current signature is left exactly as it is.
#[tokio::test]
async fn a_thread_with_a_fresh_vouch_is_left_untouched() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return };
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let (sk, kid, hsk, hkid) = setup(&c).await;
    // A SECOND human — "Dr B" — whose signature must survive the first human's sign-off.
    // `generate_key()` returns (SigningKey, hex kid); the kid is never a literal (rule 6).
    let (other_sk, other_kid) = generate_key().unwrap();
    c.execute(
        "SELECT enroll_actor('human', '{\"role\":\"clinician\"}', $1)",
        &[&other_kid],
    )
    .await
    .unwrap();
    let patient = Uuid::now_v7();

    let signed_by_other = assert_one(&mut c, &sk, &kid, "origin-a", patient, "metformin").await;
    let unsigned = assert_one(&mut c, &sk, &kid, "origin-a", patient, "amlodipine").await;

    let other = AttestParams { human_sk: &other_sk, human_kid: &other_kid, basis: None, note: None };
    attest_medication_thread(&mut c, &sk, "origin-a", &other, patient, signed_by_other)
        .await.unwrap();

    let me = AttestParams { human_sk: &hsk, human_kid: &hkid, basis: None, note: None };
    let out = sign_off_medication_list(&mut c, &sk, "origin-a", &me, patient).await.unwrap();

    assert_eq!(out.attested, vec![unsigned], "only the unsigned thread is signed");
    assert_eq!(
        attestation_count(&c, signed_by_other).await, 1,
        "Dr B's signature is not signed over"
    );
    let who: String = c.query_one(
        "SELECT attester_kid FROM medication_thread_attestation WHERE medication_id = $1",
        &[&signed_by_other],
    ).await.unwrap().get(0);
    assert_eq!(who, other_kid, "the drug line still carries Dr B's name");
}

#[tokio::test]
async fn a_ceased_thread_is_not_signed() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return };
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let (sk, kid, hsk, hkid) = setup(&c).await;
    let patient = Uuid::now_v7();

    let thread = assert_one(&mut c, &sk, &kid, "origin-a", patient, "metformin").await;
    cease_medication(
        &mut c, &sk, &kid, "origin-a", patient, thread,
        &CeaseMedicationInput { stopped: None, stopped_precision: None, reason: None },
        None, None,
    ).await.unwrap();

    let params = AttestParams { human_sk: &hsk, human_kid: &hkid, basis: None, note: None };
    let out = sign_off_medication_list(&mut c, &sk, "origin-a", &params, patient).await.unwrap();

    assert!(out.attested.is_empty(), "a struck line is not re-signed");
    assert_eq!(attestation_count(&c, thread).await, 0);
}

/// An empty chart signs nothing and does NOT error. Recording "nil medications, reviewed"
/// has no record-layer home yet — issue #331.
#[tokio::test]
async fn an_empty_list_signs_nothing_without_erroring() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return };
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let (sk, _kid, hsk, hkid) = setup(&c).await;

    let params = AttestParams { human_sk: &hsk, human_kid: &hkid, basis: None, note: None };
    let out = sign_off_medication_list(&mut c, &sk, "origin-a", &params, Uuid::now_v7())
        .await.unwrap();

    assert!(out.attested.is_empty());
    assert!(out.event_ids.is_empty());
}

/// All-or-nothing: an unenrolled attester is refused by the db/005 responsibility gate,
/// and NO thread ends up vouched — not even the ones processed before the failure.
#[tokio::test]
async fn a_refused_attestation_rolls_the_whole_gesture_back() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return };
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let (sk, kid, _hsk, _hkid) = setup(&c).await;
    let patient = Uuid::now_v7();

    let a = assert_one(&mut c, &sk, &kid, "origin-a", patient, "metformin").await;
    let b = assert_one(&mut c, &sk, &kid, "origin-a", patient, "amlodipine").await;

    // Never enrolled: the db/005 responsibility gate refuses this attester.
    let (stranger_sk, stranger_kid) = generate_key().unwrap();
    let params = AttestParams {
        human_sk: &stranger_sk, human_kid: &stranger_kid, basis: None, note: None,
    };

    let err = sign_off_medication_list(&mut c, &sk, "origin-a", &params, patient).await;
    assert!(err.is_err(), "an unenrolled attester must be refused");
    assert_eq!(attestation_count(&c, a).await, 0, "no partial sign-off survives");
    assert_eq!(attestation_count(&c, b).await, 0, "no partial sign-off survives");
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test -p cairn-node --test medication_signoff`
Expected: FAIL to compile — `cairn_node::medication::signoff` does not exist.

- [ ] **Step 3: Write the orchestrator**

`crates/cairn-node/src/medication/signoff.rs`:

```rust
//! Whole-list medication sign-off — the record-layer half of #288.
//!
//! ADR-0049 attestation is per THREAD, so vouching for a chart means authoring one
//! attestation per thread that needs one. That is N cryptographic acts, but it must be
//! ONE human act: `attest_thread_in_tx` takes an already-unsealed key by reference, so one
//! unseal and one transaction cover all N. This module is what turns that permission into
//! a callable verb — and it lives in the node, not the UI, so the CLI has the same gesture
//! and the reference UI uses no privileged path (ADR-0021).
//!
//! The bundling shape (mint the HLCs, open one transaction, attest each thread, commit) is
//! the one `reconciliation.rs` already uses for exactly two threads, generalised to N.
use crate::medication::{read::list_patient_medications, AttestParams};
use cairn_medication_view::sign_off_targets;
use uuid::Uuid;

/// What one sign-off gesture did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignOffOutcome {
    /// The thread ids that were vouched, in the order they were attested.
    pub attested: Vec<Uuid>,
    /// The attestation event ids, positionally matching `attested`.
    pub event_ids: Vec<Uuid>,
}

/// Attest every thread on this patient's chart whose vouch is absent or stale, in one
/// transaction.
///
/// # Why the target set is read twice
///
/// HLCs must be minted BEFORE the transaction opens: `node_hlc_tick()` advances node state,
/// and minting inside a transaction that later aborts would roll the tick back. But the
/// attestations must be computed against the same snapshot they are written in. So the
/// list is read once outside the transaction (to size the HLC mint) and once inside it (to
/// decide what to sign), and the two must agree.
///
/// If they do not — a medication arrived, or someone else signed a thread, in the
/// milliseconds between — the gesture is REFUSED rather than silently adjusted. That is
/// the clinically correct answer: the clinician vouched for the list they were looking at,
/// and signing a different list on their behalf would be exactly the silent substitution
/// the "never silently refresh on screen" rule exists to prevent. The caller refreshes and
/// the clinician signs again.
pub async fn sign_off_medication_list(
    client: &mut tokio_postgres::Client,
    node_sk: &cairn_event::SigningKey,
    node_origin: &str,
    params: &AttestParams<'_>,
    patient: Uuid,
) -> anyhow::Result<SignOffOutcome> {
    // The node holds custody of every sealed body it writes, attestations included
    // (ADR-0052). Idempotent, and committed ahead of the transaction.
    crate::medication::sealed_submit::ensure_unwrap_key(client, node_sk).await?;

    let expected = sign_off_targets(&list_patient_medications(&*client, patient).await?);
    if expected.is_empty() {
        // Nothing to vouch for. NOT an error: an empty chart is a legitimate state. That
        // "no regular medications, reviewed" cannot itself be recorded is a real gap,
        // tracked as issue #331 — the caller renders the gesture as unavailable.
        return Ok(SignOffOutcome { attested: vec![], event_ids: vec![] });
    }

    // One HLC per attestation, minted up front and consumed in target order (which
    // `sign_off_targets` sorts, so the assignment is deterministic).
    let mut hlcs = Vec::with_capacity(expected.len());
    for _ in 0..expected.len() {
        hlcs.push(crate::db::next_hlc(client, node_origin).await?);
    }

    let tx = client.transaction().await?;
    let actual = sign_off_targets(&list_patient_medications(&tx, patient).await?);
    if actual != expected {
        anyhow::bail!(
            "the medication list changed while it was being signed ({} thread(s) when read, \
             {} in the signing transaction); nothing was signed — refresh the list and sign \
             again so the vouch covers what was actually reviewed",
            expected.len(),
            actual.len()
        );
    }

    let mut event_ids = Vec::with_capacity(actual.len());
    for (thread, hlc) in actual.iter().zip(hlcs) {
        event_ids.push(
            crate::medication::attest_thread_in_tx(&tx, params, patient, *thread, hlc).await?,
        );
    }
    tx.commit().await?;

    Ok(SignOffOutcome { attested: actual, event_ids })
}
```

In `crates/cairn-node/src/medication/mod.rs`, add `pub mod signoff;`.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test -p cairn-node --test medication_signoff`
Expected: PASS, 5 tests.

- [ ] **Step 5: Run the whole workspace suite**

Run: `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test --workspace`
Expected: PASS. (Run the *workspace* suite, not `-p cairn-node`: a signature change in a medication
orchestrator has broken `cairn-sync/tests/clinical_pull.rs` before, and a per-crate run misses it.)

- [ ] **Step 6: Commit**

```bash
git add crates/cairn-node/src/medication/signoff.rs crates/cairn-node/src/medication/mod.rs \
        crates/cairn-node/tests/medication_signoff.rs
git commit -m "feat(#288): whole-list medication sign-off in one transaction

ADR-0049 attestation is per thread, so vouching for a chart authors N events —
but attest_thread_in_tx takes an already-unsealed key by reference, so one
unseal and one transaction cover all N. This turns that permission into a verb,
in the node rather than the UI, so the CLI has the same gesture.

The target set is read twice, outside the transaction to size the HLC mint and
inside it to decide what to sign. A mismatch REFUSES the gesture rather than
silently adjusting it: the clinician vouched for the list they were looking at."
```

---

## Task 4: Expose both as CLI verbs

**Files:**
- Modify: `crates/cairn-node/src/main.rs` (the `Cmd` enum and its `match`)

**Interfaces:**
- Consumes: `read::list_patient_medications`, `signoff::sign_off_medication_list`, the existing
  `load_signing_key` / `load_attester_key` / `AttestFlags` helpers.
- Produces: `cairn-node medication-list <patient> [--json]` and
  `cairn-node medication-sign-off <patient> --attest-as <key>`.

- [ ] **Step 1: Add `serde_json` derive support to the view model**

`MedicationRow` already derives `Serialize` (Task 1). No change needed — confirm with
`cargo tree -p cairn-medication-view` that `serde` is present.

- [ ] **Step 2: Add the two commands to the `Cmd` enum**

In `crates/cairn-node/src/main.rs`, after the existing `MedicationAttest` variant:

```rust
    /// Read a patient's medication list — current drugs and ceased ones, each with the
    /// clinician whose signature it carries. The read path the reference UI uses; a
    /// future native API (ADR-0023) is expected to wrap the same function.
    MedicationList {
        /// The patient UUID whose chart to read.
        patient: Uuid,
        /// Emit JSON instead of the human-readable table.
        #[arg(long)]
        json: bool,
    },
    /// Sign off the whole medication list in ONE gesture (#288): attests every thread
    /// whose vouch is absent or stale, in one transaction. Threads already carrying a
    /// current signature keep it — a drug line carries the signature of the person
    /// responsible for that drug.
    MedicationSignOff {
        /// The patient UUID whose chart is being signed off.
        patient: Uuid,
        #[command(flatten)]
        attest: AttestFlags,
    },
```

- [ ] **Step 3: Add the match arms**

In the `match` in `main()`, after the `Cmd::MedicationAttest` arm:

```rust
        Cmd::MedicationList { patient, json } => {
            let db = cairn_node::db::connect(&cli.conn).await?;
            let rows = cairn_node::medication::read::list_patient_medications(&db, patient).await?;
            if json {
                println!("{}", serde_json::to_string_pretty(&rows)?);
            } else if rows.is_empty() {
                // Deliberately explicit: an empty chart is a real clinical state, and
                // silence would read as "the query failed" (issue #331 covers recording
                // "nil medications, reviewed" as an act).
                println!("no medications recorded for {patient}");
            } else {
                for row in &rows {
                    let name = row.coding_display.as_deref().unwrap_or(&row.term);
                    let dose = match (&row.dose_amount, &row.dose_unit) {
                        (Some(a), Some(u)) => format!(" {a} {u}"),
                        (Some(a), None) => format!(" {a}"),
                        _ => String::new(),
                    };
                    let status = match row.status {
                        cairn_medication_view::MedicationStatus::Active => "current",
                        cairn_medication_view::MedicationStatus::Ceased => "ceased",
                    };
                    // One line per member thread's signature state, because that is what
                    // a sign-off acts on — a row-level summary would hide a mixed group.
                    let vouches: Vec<String> = row
                        .members
                        .iter()
                        .map(|m| match &m.vouch {
                            cairn_medication_view::VouchState::Absent => "unsigned".to_string(),
                            cairn_medication_view::VouchState::Fresh { by } => {
                                format!("signed by {}", &by[..8.min(by.len())])
                            }
                            cairn_medication_view::VouchState::Stale { by } => {
                                format!("signed by {} (out of date)", &by[..8.min(by.len())])
                            }
                        })
                        .collect();
                    println!("{name}{dose} [{status}] — {}", vouches.join("; "));
                    if row.reconciliation_flagged {
                        println!("    ! possible un-reconciled duplicate");
                    }
                    if row.coding_conflict {
                        println!("    ! two different drug anchors in this group");
                    }
                }
            }
        }
        Cmd::MedicationSignOff { patient, attest } => {
            let node_sk = load_signing_key(&cli.key, true)?;
            let node_kid = hex::encode(node_sk.verifying_key().to_bytes());
            let mut db = cairn_node::db::connect(&cli.conn).await?;
            let id = cairn_node::identity::load_local(&db).await?;
            ensure_registration_actor(&db, &node_kid).await?;

            let held = attest_key_and_kid(&attest)?;
            let params = held
                .as_ref()
                .map(|(sk, kid)| cairn_node::medication::AttestParams {
                    human_sk: sk,
                    human_kid: kid,
                    basis: attest.basis.as_deref(),
                    note: attest.note.as_deref(),
                })
                .ok_or_else(|| {
                    anyhow::anyhow!("--attest-as is required: a sign-off IS the human vouch")
                })?;

            let out = cairn_node::medication::signoff::sign_off_medication_list(
                &mut db, &node_sk, &id.node_id_hex, &params, patient,
            )
            .await?;

            if out.attested.is_empty() {
                println!("nothing to sign off for {patient}: every drug already carries a current signature");
            } else {
                println!("signed off {} medication thread(s) for {patient}", out.attested.len());
                for (thread, event) in out.attested.iter().zip(&out.event_ids) {
                    println!("  {thread} -> attestation {event}");
                }
            }
        }
```

`attest_key_and_kid` is the existing helper in `main.rs` that turns `AttestFlags` into an
`Option<(SigningKey, String)>`; if its name differs, use whatever `Cmd::MedicationAttest` calls.

- [ ] **Step 4: Build and exercise both verbs by hand**

Run:
```bash
cargo build -p cairn-node
./target/debug/cairn-node medication-list --help
./target/debug/cairn-node medication-sign-off --help
```
Expected: both print help with the documented flags; the crate compiles with no warnings.

- [ ] **Step 5: Run the workspace suite**

Run: `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test --workspace`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/cairn-node/src/main.rs
git commit -m "feat(#288): medication-list and medication-sign-off CLI verbs

The read path and the one-gesture sign-off as public CLI surface, so the
reference UI provably uses no privileged path (ADR-0021). medication-list
prints one signature-state line per member thread rather than a row-level
summary, because a mixed reconciled group is exactly what a summary would hide."
```

---

## Task 5: Retire the superseded iced rendering layer

**Files:**
- Modify: `cairn-gui/cairn-gui-shell/Cargo.toml`
- Delete: `cairn-gui/cairn-gui-shell/src/app.rs`, `src/a11y_dump.rs`, `src/bin/gui.rs`
- Modify: `cairn-gui/cairn-gui-shell/src/lib.rs`
- Modify: `cairn-gui/cairn-gui-tabs/cairn-gui-tab-demographics/src/lib.rs`,
  `cairn-gui/cairn-gui-tabs/cairn-gui-tab-note/src/lib.rs` (remove any `gui`-gated `view()` bodies)

**Interfaces:**
- Consumes: nothing.
- Produces: a `cairn-gui` workspace with no iced dependency; `cairn_gui_shell::workspace` and
  `cairn_gui_shell::freshness` unchanged and still tested.

- [ ] **Step 1: Confirm what is iced-bound before deleting anything**

Run: `grep -rn "iced" cairn-gui --include='*.rs' --include='*.toml' | grep -v target`
Expected: hits only in `cairn-gui-shell/Cargo.toml`, `src/app.rs`, `src/a11y_dump.rs`,
`src/bin/gui.rs`, `src/lib.rs`, and possibly `gui`-gated blocks in the two tab crates. Anything
else is a surprise — read it before proceeding.

- [ ] **Step 2: Record the pre-change advisory state (for the #252 claim)**

Run: `cargo deny check advisories 2>&1 | tee /tmp/deny-before.txt; cd cairn-gui && cargo deny check advisories 2>&1 | tee /tmp/deny-gui-before.txt; cd ..`
Expected: capture whether `quick-xml` / RUSTSEC-2026-0194/0195 appear. This is the baseline the
claim in Step 7 is measured against — do not assume the outcome.

- [ ] **Step 3: Delete the iced layer**

```bash
git rm cairn-gui/cairn-gui-shell/src/app.rs \
       cairn-gui/cairn-gui-shell/src/a11y_dump.rs \
       cairn-gui/cairn-gui-shell/src/bin/gui.rs
```

In `cairn-gui/cairn-gui-shell/Cargo.toml`, delete the `iced` dependency line, the entire
`[features]` block, and the entire `[[bin]]` block.

In `cairn-gui/cairn-gui-shell/src/lib.rs`, remove the `app` and `a11y_dump` module declarations and
any `#[cfg(feature = "gui")]` attributes, leaving:

```rust
//! The reference shell's framework-agnostic state.
//!
//! The iced rendering layer that used to live here was retired on 2026-08-02: released
//! iced 0.14 ships no accessibility tree (spike 0004), so the reference UI moved to Tauri
//! 2. What survives is what was never framework-specific — the pane/tab workspace state
//! machine and the freshness rules — kept because the Tauri shell wires them in the next
//! slice. Tested code awaiting a consumer, not dead code.
pub mod freshness;
pub mod workspace;
```

Remove any `#[cfg(feature = "gui")]` `view()` bodies from the two tab crates, leaving their
`Semantic` impls and tests intact.

- [ ] **Step 4: Run the GUI workspace tests**

Run: `cd cairn-gui && cargo test && cd ..`
Expected: PASS — the pane/routing/freshness and semantics tests still pass, now with no iced in the
tree.

- [ ] **Step 5: Prove iced is gone**

Run: `cd cairn-gui && cargo tree | grep -c iced; cd ..`
Expected: `0`.

- [ ] **Step 6: Re-check the advisories**

Run: `cd cairn-gui && cargo deny check advisories 2>&1 | tee /tmp/deny-gui-after.txt; cd ..; diff /tmp/deny-gui-before.txt /tmp/deny-gui-after.txt`
Expected: compare honestly. **If the `quick-xml` advisories are gone, comment on
[#252](https://github.com/cairn-ehr/cairn-ehr/issues/252) with the before/after and close it. If
they are NOT gone, say so on #252 and leave it open** — the spec predicted this outcome but did not
assume it.

- [ ] **Step 7: Commit**

```bash
git add -A cairn-gui
git commit -m "chore(gui): retire the superseded iced rendering layer

Released iced 0.14 ships no accessibility tree (spike 0004), which is why the
reference UI moved to Tauri 2. Keeping a second shell meant CI building a
framework we had decided against.

Deleted: the iced view layer, the gui feature, the a11y dump, the binary.
Kept: the pane/routing/freshness state machine and the semantic contract, which
were never framework-specific and which the Tauri shell wires in the next slice."
```

---

## Task 6: Extend the data port with medications

**Files:**
- Modify: `cairn-gui/cairn-gui-data/Cargo.toml`
- Modify: `cairn-gui/cairn-gui-data/src/port.rs`
- Modify: `cairn-gui/cairn-gui-data/src/mock.rs`

**Interfaces:**
- Consumes: `cairn_medication_view::MedicationRow`.
- Produces: `ClinicalData::medications(&self, patient_uuid: &str) -> Result<Vec<MedicationRow>, DataError>`
  and a `MockData` returning a fixture list.

- [ ] **Step 1: Add the dependency**

In `cairn-gui/cairn-gui-data/Cargo.toml`:

```toml
cairn-medication-view = { path = "../../crates/cairn-medication-view" }
```

(A path dependency across the two workspaces is fine — the direction is GUI → crates, which is the
permitted one.)

- [ ] **Step 2: Write the failing mock test**

At the bottom of `cairn-gui/cairn-gui-data/src/mock.rs`:

```rust
#[cfg(test)]
mod medication_tests {
    use super::*;
    use crate::port::ClinicalData;
    use cairn_medication_view::{MedicationStatus, VouchState};

    /// The mock exists so the window runs with no database — what the operator
    /// accessibility pass and the timing runbook need on a laptop. It must therefore
    /// exercise the interesting shapes, not one bland row.
    #[test]
    fn the_fixture_list_covers_absent_fresh_stale_and_ceased() {
        let rows = MockData::default()
            .medications(cairn_medication_view::fixtures::FIXTURE_PATIENT)
            .unwrap();
        assert!(rows.len() >= 4, "the fixture must show several drugs");

        let vouches: Vec<&VouchState> =
            rows.iter().flat_map(|r| r.members.iter().map(|m| &m.vouch)).collect();
        assert!(vouches.iter().any(|v| matches!(v, VouchState::Absent)));
        assert!(vouches.iter().any(|v| matches!(v, VouchState::Fresh { .. })));
        assert!(vouches.iter().any(|v| matches!(v, VouchState::Stale { .. })));
        assert!(rows.iter().any(|r| r.status == MedicationStatus::Ceased));
    }

    /// An unknown chart is EMPTY, not an error — an empty chart is a real clinical state.
    #[test]
    fn an_unknown_patient_has_an_empty_chart() {
        assert!(MockData::default()
            .medications("11111111-1111-1111-1111-111111111111")
            .unwrap()
            .is_empty());
    }
}
```

- [ ] **Step 3: Run to verify it fails**

Run: `cd cairn-gui && cargo test -p cairn-gui-data && cd ..`
Expected: FAIL — `medications` is not a member of `ClinicalData`.

- [ ] **Step 4: Extend the trait and the mock**

In `cairn-gui/cairn-gui-data/src/port.rs`, add to the trait:

```rust
    /// One patient's medication list — current drugs and ceased ones, each carrying the
    /// signature state of its member threads. The real implementation is
    /// `cairn_node::medication::read::list_patient_medications`; this port exists so the
    /// window can also run against fixtures with no database.
    fn medications(
        &self,
        patient_uuid: &str,
    ) -> Result<Vec<cairn_medication_view::MedicationRow>, DataError>;
```

The fixture list itself goes in the **shared** crate, because Task 7's view-model tests need exactly
the same rows and two copies would drift. Create `crates/cairn-medication-view/src/fixtures.rs`:

```rust
//! A realistic sample chart, shared by the GUI's mock data port and by the view-model
//! tests. One definition, because two copies of "the interesting shapes" drift and the
//! tests then stop covering what the demo actually shows.
//!
//! Not test-only: the `--mock` window is a real, shipped mode — it is what the operator
//! accessibility pass and the timing runbook use on a machine with no database.
use crate::row::{MedicationRow, MedicationStatus, MemberVouch, VouchState};
use uuid::Uuid;

/// The patient id the mock chart belongs to.
pub const FIXTURE_PATIENT: &str = "00000000-0000-0000-0000-000000000001";

fn uid(n: u128) -> Uuid {
    Uuid::from_u128(n)
}

fn base(group: u128, term: &str, amount: &str, unit: &str) -> MedicationRow {
    MedicationRow {
        group_id: uid(group),
        patient_id: uid(1),
        term: term.to_string(),
        coding_display: None,
        formulation: Some("tablet".into()),
        dose_amount: Some(amount.into()),
        dose_unit: Some(unit.into()),
        sig: None,
        started_value: None,
        started_precision: None,
        status: MedicationStatus::Active,
        members: vec![],
        reconciliation_flagged: false,
        coding_conflict: false,
    }
}

fn member(id: u128, vouch: VouchState) -> MemberVouch {
    MemberVouch { medication_id: uid(id), vouch }
}

/// A chart covering every shape the view model has to get right: unsigned, signed by
/// SOMEONE ELSE, stale, ceased, and a reconciled group whose two members disagree.
pub fn sample_rows() -> Vec<MedicationRow> {
    let other = "b7c1e9a4f2d38650".to_string();

    let mut unsigned = base(10, "atorvastatin", "40", "mg");
    unsigned.members = vec![member(10, VouchState::Absent)];

    let mut signed_by_other = base(20, "amlodipine", "5", "mg");
    signed_by_other.members = vec![member(20, VouchState::Fresh { by: other.clone() })];

    let mut stale = base(30, "sertraline", "50", "mg");
    stale.members = vec![member(30, VouchState::Stale { by: other.clone() })];

    let mut ceased = base(40, "ibuprofen", "400", "mg");
    ceased.status = MedicationStatus::Ceased;
    ceased.members = vec![member(40, VouchState::Absent)];

    // A reconciled pair: ONE row, TWO threads, differing freshness. The group/thread
    // asymmetry that the badge and the sign-off count both have to handle.
    let mut reconciled = base(50, "metformin", "1", "g");
    reconciled.coding_display = Some("metformin hydrochloride".into());
    reconciled.members = vec![
        member(50, VouchState::Fresh { by: other }),
        member(51, VouchState::Absent),
    ];
    reconciled.reconciliation_flagged = true;

    vec![unsigned, signed_by_other, stale, ceased, reconciled]
}
```

Add `pub mod fixtures;` to `crates/cairn-medication-view/src/lib.rs`.

Then in `cairn-gui/cairn-gui-data/src/mock.rs`:

```rust
    fn medications(
        &self,
        patient_uuid: &str,
    ) -> Result<Vec<cairn_medication_view::MedicationRow>, DataError> {
        // Any other patient has an empty chart rather than an error: an empty chart is a
        // real clinical state, and the window must render it honestly.
        if patient_uuid == cairn_medication_view::fixtures::FIXTURE_PATIENT {
            Ok(cairn_medication_view::fixtures::sample_rows())
        } else {
            Ok(vec![])
        }
    }
```

- [ ] **Step 5: Run to verify it passes**

Run: `cd cairn-gui && cargo test -p cairn-gui-data && cd ..`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add cairn-gui/cairn-gui-data Cargo.lock cairn-gui/Cargo.lock
git commit -m "feat(#288): extend the clinical data port with medications

The mock fixture covers absent/fresh/stale vouches, a ceased drug and a mixed
reconciled group — the shapes the view model has to get right — so the window
runs with no database for the operator accessibility pass and the timing runs."
```

---

## Task 7: The med-list view model

**Files:**
- Create: `cairn-gui/cairn-gui-tabs/cairn-gui-tab-medications/Cargo.toml`
- Create: `cairn-gui/cairn-gui-tabs/cairn-gui-tab-medications/src/lib.rs`
- Create: `cairn-gui/cairn-gui-tabs/cairn-gui-tab-medications/src/view.rs`
- Modify: `cairn-gui/Cargo.toml` (workspace `members`)

**Interfaces:**
- Consumes: `cairn_medication_view::{MedicationRow, MedicationStatus, VouchState, sign_off_targets}`,
  `cairn_gui_tab::{Semantic, TabId, Context, semantics::{SemanticNode, Field, Role}}`.
- Produces: `MedListRowView`, `MedListView`, `build_view(rows: &[MedicationRow]) -> MedListView`,
  and `MedicationsTab` implementing `Semantic`.

- [ ] **Step 1: Create the manifest and register the crate**

`cairn-gui/cairn-gui-tabs/cairn-gui-tab-medications/Cargo.toml`:

```toml
[package]
name = "cairn-gui-tab-medications"
version = "0.1.0"
edition.workspace = true
rust-version.workspace = true
license.workspace = true
publish = false

[dependencies]
cairn-gui-tab = { path = "../../cairn-gui-tab" }
cairn-medication-view = { path = "../../../crates/cairn-medication-view" }
serde = { version = "1", features = ["derive"] }
```

Add `"cairn-gui-tabs/cairn-gui-tab-medications"` to `members` in `cairn-gui/Cargo.toml`.

- [ ] **Step 2: Write the failing view-model tests**

At the bottom of `cairn-gui/cairn-gui-tabs/cairn-gui-tab-medications/src/view.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use cairn_medication_view::{MedicationRow, MedicationStatus, MemberVouch, VouchState};
    use uuid::Uuid;

    fn uid(n: u128) -> Uuid { Uuid::from_u128(n) }

    fn row(group: u128, status: MedicationStatus, members: Vec<MemberVouch>) -> MedicationRow {
        MedicationRow {
            group_id: uid(group), patient_id: uid(999),
            term: "metformin".into(), coding_display: None, formulation: None,
            dose_amount: Some("500".into()), dose_unit: Some("mg".into()), sig: None,
            started_value: None, started_precision: None,
            status, members, reconciliation_flagged: false, coding_conflict: false,
        }
    }

    fn member(id: u128, vouch: VouchState) -> MemberVouch {
        MemberVouch { medication_id: uid(id), vouch }
    }

    #[test]
    fn a_coded_drug_displays_its_coded_name_over_the_free_text_term() {
        let mut r = row(1, MedicationStatus::Active, vec![member(1, VouchState::Absent)]);
        r.term = "little white pill".into();
        r.coding_display = Some("metformin hydrochloride".into());
        let view = build_view(&[r]);
        assert_eq!(view.rows[0].primary, "metformin hydrochloride");
    }

    /// Principle 4: a vague term is a legitimate recorded value, never blanked out.
    #[test]
    fn an_uncoded_drug_displays_its_free_text_term_unaltered() {
        let mut r = row(1, MedicationStatus::Active, vec![member(1, VouchState::Absent)]);
        r.term = "little white pill".into();
        let view = build_view(&[r]);
        assert_eq!(view.rows[0].primary, "little white pill");
    }

    /// The badge and the button must agree, because they come from ONE rule.
    #[test]
    fn rows_that_will_be_signed_match_the_sign_off_count() {
        let rows = vec![
            row(1, MedicationStatus::Active, vec![member(1, VouchState::Absent)]),
            row(2, MedicationStatus::Active,
                vec![member(2, VouchState::Fresh { by: "dr_b_key".into() })]),
            row(3, MedicationStatus::Active,
                vec![member(3, VouchState::Stale { by: "dr_b_key".into() })]),
        ];
        let view = build_view(&rows);
        assert_eq!(view.sign_off_count, 2, "two threads need a signature");
        assert!(view.rows[0].will_be_signed);
        assert!(!view.rows[1].will_be_signed, "Dr B's current signature stands");
        assert!(view.rows[2].will_be_signed);
        assert!(view.sign_off_enabled);
    }

    #[test]
    fn a_fresh_vouch_names_its_signatory() {
        let rows = vec![row(1, MedicationStatus::Active,
            vec![member(1, VouchState::Fresh { by: "abcdef0123456789".into() })])];
        let view = build_view(&rows);
        assert!(view.rows[0].vouch_label.contains("abcdef01"),
                "the clinician must see WHOSE signature it is: {}", view.rows[0].vouch_label);
    }

    #[test]
    fn a_stale_vouch_says_so() {
        let rows = vec![row(1, MedicationStatus::Active,
            vec![member(1, VouchState::Stale { by: "abcdef0123456789".into() })])];
        let view = build_view(&rows);
        assert!(view.rows[0].vouch_label.contains("out of date"),
                "got: {}", view.rows[0].vouch_label);
    }

    /// Issue #331's honest surface: nothing to sign, and the reason is stated rather than
    /// leaving a dead button.
    #[test]
    fn an_empty_chart_disables_the_gesture_and_explains_why() {
        let view = build_view(&[]);
        assert_eq!(view.sign_off_count, 0);
        assert!(!view.sign_off_enabled);
        assert!(view.empty_message.is_some());
    }

    #[test]
    fn a_fully_signed_chart_disables_the_gesture() {
        let rows = vec![row(1, MedicationStatus::Active,
            vec![member(1, VouchState::Fresh { by: "me".into() })])];
        let view = build_view(&rows);
        assert!(!view.sign_off_enabled);
        assert_eq!(view.sign_off_count, 0);
    }

    #[test]
    fn a_ceased_row_is_shown_marked_and_never_targeted() {
        let rows = vec![row(1, MedicationStatus::Ceased, vec![member(1, VouchState::Absent)])];
        let view = build_view(&rows);
        assert_eq!(view.rows.len(), 1, "a struck line stays on the chart");
        assert_eq!(view.rows[0].status_label, "ceased");
        assert!(!view.rows[0].will_be_signed);
        assert!(!view.rows[0].can_cease, "a ceased drug cannot be ceased again");
    }

    #[test]
    fn advisory_flags_are_surfaced_as_row_labels() {
        let mut r = row(1, MedicationStatus::Active, vec![member(1, VouchState::Absent)]);
        r.reconciliation_flagged = true;
        r.coding_conflict = true;
        let view = build_view(&[r]);
        assert_eq!(view.rows[0].flags.len(), 2, "got: {:?}", view.rows[0].flags);
    }

    #[test]
    fn the_dose_reads_as_amount_and_unit() {
        let view = build_view(&[row(1, MedicationStatus::Active,
            vec![member(1, VouchState::Absent)])]);
        assert_eq!(view.rows[0].dose, "500 mg");
    }

    /// Principle 4 again: an unknown dose is shown as unknown, never as a blank that reads
    /// like "no dose" or as a fabricated default.
    #[test]
    fn an_absent_dose_is_shown_as_unknown() {
        let mut r = row(1, MedicationStatus::Active, vec![member(1, VouchState::Absent)]);
        r.dose_amount = None;
        r.dose_unit = None;
        assert_eq!(build_view(&[r]).rows[0].dose, "dose not recorded");
    }
}
```

And in `src/lib.rs`, the semantics test:

```rust
#[cfg(test)]
mod semantic_tests {
    use super::*;
    use cairn_gui_tab::context::{Capabilities, Context, PatientRef, UserRef};

    fn ctx() -> Context {
        Context {
            patient: Some(PatientRef {
                uuid: "00000000-0000-0000-0000-000000000001".into(),
                display_name: "Test Patient".into(),
            }),
            user: UserRef { actor_id: "kid".into(), display_name: "Dr A".into() },
            capabilities: Capabilities::clinician_all(),
        }
    }

    /// The accessibility bar that cost iced the reference UI: every focusable control
    /// carries a non-empty label and ids are unique.
    #[test]
    fn the_rendered_contract_is_accessibility_complete() {
        let tab = MedicationsTab::new(crate::view::build_view(
            &cairn_medication_view::fixtures::sample_rows(),
        ));
        tab.semantics(&ctx()).assert_complete().expect("a11y contract must be complete");
    }

    #[test]
    fn every_row_contributes_a_labelled_cease_control() {
        let tab = MedicationsTab::new(crate::view::build_view(
            &cairn_medication_view::fixtures::sample_rows(),
        ));
        let node = tab.semantics(&ctx());
        let cease_buttons = node.fields.iter().filter(|f| f.id.starts_with("cease-")).count();
        assert!(cease_buttons > 0, "each current drug needs its own cease control");
    }
}
```

- [ ] **Step 3: Run to verify the tests fail**

Run: `cd cairn-gui && cargo test -p cairn-gui-tab-medications && cd ..`
Expected: FAIL — the crate does not compile.

- [ ] **Step 4: Write the view model**

At the top of `src/view.rs`:

```rust
//! The med-list view model: everything the window shows, computed in Rust.
//!
//! The webview renders this and decides nothing. Every clinical display question — which
//! name to show, whose signature a line carries, whether the gesture will sign this row —
//! is answered here, under `cargo test`, because a wrong answer is a clinical falsehood on
//! screen and a webview is not a place we can test that.
use cairn_medication_view::{MedicationRow, MedicationStatus, VouchState, sign_off_targets};
use serde::Serialize;
use std::collections::HashSet;

/// One rendered drug line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MedListRowView {
    /// Stable id for the DOM and for the cease command.
    pub group_id: String,
    /// The drug's coded name when it has one, else the term exactly as asserted.
    pub primary: String,
    /// Dose as "500 mg", or an explicit statement that none was recorded.
    pub dose: String,
    pub formulation: String,
    pub sig: String,
    pub started: String,
    /// "current" or "ceased".
    pub status_label: String,
    /// Whose signature this line carries, and whether it is out of date.
    pub vouch_label: String,
    /// True when the sign-off gesture will sign this row. Derived from the SAME
    /// `sign_off_targets` the orchestrator uses — never recomputed here.
    pub will_be_signed: bool,
    /// False for an already-ceased drug.
    pub can_cease: bool,
    /// Advisory worklist labels (duplicate suspicion, anchor conflict).
    pub flags: Vec<String>,
}

/// The whole window's state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MedListView {
    pub rows: Vec<MedListRowView>,
    /// How many THREADS the gesture will sign. Not the row count — a reconciled group can
    /// contribute more than one, and the clinician is entitled to know the real number.
    pub sign_off_count: usize,
    pub sign_off_enabled: bool,
    /// Why there is nothing to do, when there is nothing to do.
    pub empty_message: Option<String>,
}

/// Shown instead of a blank cell. Principle 4: an unrecorded dose is a recordable state,
/// and a blank would read either as "no dose" or as a rendering bug.
const DOSE_UNKNOWN: &str = "dose not recorded";

pub fn build_view(rows: &[MedicationRow]) -> MedListView {
    // ONE call to the shared rule. The badge on each row and the count on the button both
    // come from this set, so what the clinician is told will be signed is, by
    // construction, what the orchestrator will sign.
    let targets: HashSet<_> = sign_off_targets(rows).into_iter().collect();

    let view_rows: Vec<MedListRowView> = rows
        .iter()
        .map(|row| MedListRowView {
            group_id: row.group_id.to_string(),
            primary: row.coding_display.clone().unwrap_or_else(|| row.term.clone()),
            dose: match (&row.dose_amount, &row.dose_unit) {
                (Some(amount), Some(unit)) => format!("{amount} {unit}"),
                (Some(amount), None) => amount.clone(),
                _ => DOSE_UNKNOWN.to_string(),
            },
            formulation: row.formulation.clone().unwrap_or_default(),
            sig: row.sig.clone().unwrap_or_default(),
            started: row.started_value.clone().unwrap_or_default(),
            status_label: match row.status {
                MedicationStatus::Active => "current".into(),
                MedicationStatus::Ceased => "ceased".into(),
            },
            vouch_label: vouch_label(row),
            will_be_signed: row.members.iter().any(|m| targets.contains(&m.medication_id)),
            can_cease: row.status == MedicationStatus::Active,
            flags: flags(row),
        })
        .collect();

    let sign_off_count = targets.len();
    MedListView {
        empty_message: empty_message(rows.is_empty(), sign_off_count),
        rows: view_rows,
        sign_off_count,
        sign_off_enabled: sign_off_count > 0,
    }
}

/// Whose signature this line carries.
///
/// A reconciled group has several member threads, which can disagree. The honest summary
/// names the worst state rather than picking one member's — a group is not signed off
/// until every member is.
fn vouch_label(row: &MedicationRow) -> String {
    let unsigned = row.members.iter().filter(|m| m.vouch == VouchState::Absent).count();
    let stale: Vec<&str> = row
        .members
        .iter()
        .filter_map(|m| match &m.vouch {
            VouchState::Stale { by } => Some(by.as_str()),
            _ => None,
        })
        .collect();
    if let Some(by) = stale.first() {
        return format!("signed by {} — out of date", short_kid(by));
    }
    if unsigned > 0 {
        return "not signed".to_string();
    }
    match row.members.first().and_then(|m| m.vouch.attester()) {
        Some(by) => format!("signed by {}", short_kid(by)),
        None => "not signed".to_string(),
    }
}

/// The first 8 hex characters of an actor key id — enough to distinguish colleagues on
/// screen, short enough to read. The full id is always available in the event log.
fn short_kid(kid: &str) -> &str {
    &kid[..8.min(kid.len())]
}

fn flags(row: &MedicationRow) -> Vec<String> {
    let mut out = Vec::new();
    if row.reconciliation_flagged {
        out.push("possible duplicate — not yet reconciled".to_string());
    }
    if row.coding_conflict {
        out.push("two different drug identities in this group".to_string());
    }
    out
}

fn empty_message(no_rows: bool, sign_off_count: usize) -> Option<String> {
    if no_rows {
        // Deliberately does NOT claim the patient takes nothing: an empty chart means
        // nothing has been recorded here, which is not the same clinical statement.
        // Recording "nil medications, reviewed" is issue #331.
        Some("No medications recorded on this chart.".to_string())
    } else if sign_off_count == 0 {
        Some("Every drug on this chart carries a current signature.".to_string())
    } else {
        None
    }
}
```

`cairn-gui/cairn-gui-tabs/cairn-gui-tab-medications/src/lib.rs`:

```rust
//! The medications tab: the med-list view model plus its accessibility contract.
//!
//! The contract is DATA, declared here and asserted in CI. The webview renders from it,
//! so — unlike the iced shell, where the declaration only hoped to match what a screen
//! reader announced — what is declared here is what gets built.
pub mod view;

pub use view::{build_view, MedListRowView, MedListView};

use cairn_gui_tab::context::Context;
use cairn_gui_tab::semantics::{Field, Role, SemanticNode};
use cairn_gui_tab::{Semantic, TabId};

pub struct MedicationsTab {
    view: MedListView,
}

impl MedicationsTab {
    pub fn new(view: MedListView) -> Self {
        Self { view }
    }
}

impl Semantic for MedicationsTab {
    fn tab_id(&self) -> TabId {
        TabId("medications".into())
    }

    fn title(&self) -> String {
        "Medications".into()
    }

    fn semantics(&self, _ctx: &Context) -> SemanticNode {
        let mut fields = vec![Field {
            id: "medications-heading".into(),
            role: Role::Heading,
            label: "Medications".into(),
        }];

        for row in &self.view.rows {
            // The list item's label reads the way a screen reader user needs it: drug,
            // dose, status and WHOSE signature, in one utterance. Splitting these across
            // silent cells would make the signature state announceable only by hunting.
            fields.push(Field {
                id: format!("row-{}", row.group_id),
                role: Role::ListItem,
                label: format!(
                    "{}, {}, {}, {}",
                    row.primary, row.dose, row.status_label, row.vouch_label
                ),
            });
            if row.can_cease {
                // "Stop" alone is ambiguous when buttons are read out of table context, so
                // the accessible name names the drug.
                fields.push(Field {
                    id: format!("cease-{}", row.group_id),
                    role: Role::Button,
                    label: format!("Stop {}", row.primary),
                });
            }
        }

        fields.push(Field {
            id: "sign-off".into(),
            role: Role::Button,
            label: if self.view.sign_off_enabled {
                format!(
                    "Sign off {} unsigned medication(s)",
                    self.view.sign_off_count
                )
            } else {
                // Never an empty label, even when the control is unavailable: an unlabelled
                // focusable control is exactly what `assert_complete` refuses.
                "Nothing to sign off".into()
            },
        });

        SemanticNode { title: self.title(), fields }
    }
}
```

- [ ] **Step 5: Run to verify the tests pass**

Run: `cd cairn-gui && cargo test -p cairn-gui-tab-medications && cd ..`
Expected: PASS, 13 tests.

- [ ] **Step 6: Commit**

```bash
git add cairn-gui/Cargo.toml cairn-gui/cairn-gui-tabs/cairn-gui-tab-medications cairn-gui/Cargo.lock
git commit -m "feat(#288): the med-list view model

Every clinical display decision computed in Rust under cargo test: which name to
show, whose signature a line carries, whether the gesture will sign this row.
will_be_signed and the button's count come from ONE sign_off_targets call, so
what the clinician is told will be signed is by construction what gets signed.

A reconciled group's vouch label names its WORST member state — a group is not
signed off until every member is."
```

---

## Task 8: Node-local, aggregate-only gesture timing

**Files:**
- Create: `db/044_ui_gesture_timing.sql`
- Modify: `crates/cairn-event/src/schema_generation.rs` (`SCHEMA_GENERATION` 43 → 44)
- Modify: `crates/cairn-node/src/db.rs` (append db/044 to `SCHEMA`)
- Create: `crates/cairn-node/src/ui_timing.rs`
- Modify: `crates/cairn-node/src/lib.rs`
- Test: `crates/cairn-node/tests/ui_timing.rs`

**Interfaces:**
- Consumes: nothing.
- Produces: `cairn_node::ui_timing::{Aggregate, fold_sample, size_bucket, record_gesture, read_aggregates}`.

- [ ] **Step 1: Write the failing pure tests**

At the bottom of `crates/cairn-node/src/ui_timing.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_sample_seeds_both_estimates() {
        let a = fold_sample(None, 1_200);
        assert_eq!(a.n, 1);
        assert_eq!(a.p50_ms, 1_200);
        assert_eq!(a.p95_ms, 1_200);
    }

    #[test]
    fn a_constant_stream_converges_to_that_constant() {
        let mut a = None;
        for _ in 0..500 {
            a = Some(fold_sample(a, 1_000));
        }
        let a = a.unwrap();
        assert_eq!(a.n, 500);
        assert!((a.p50_ms as i64 - 1_000).abs() <= 50, "p50 drifted: {}", a.p50_ms);
        assert!((a.p95_ms as i64 - 1_000).abs() <= 50, "p95 drifted: {}", a.p95_ms);
    }

    /// The point of tracking p95 separately: on a stream where most gestures are fast and
    /// a few are slow, p95 must sit above p50 — that tail is what a budget has to cover.
    #[test]
    fn p95_sits_above_p50_on_a_skewed_stream() {
        let mut a = None;
        for i in 0..1_000 {
            let sample = if i % 10 == 0 { 5_000 } else { 1_000 };
            a = Some(fold_sample(a, sample));
        }
        let a = a.unwrap();
        assert!(a.p95_ms > a.p50_ms, "p50={} p95={}", a.p50_ms, a.p95_ms);
    }

    #[test]
    fn buckets_partition_list_sizes() {
        assert_eq!(size_bucket(0), "1-3");
        assert_eq!(size_bucket(1), "1-3");
        assert_eq!(size_bucket(3), "1-3");
        assert_eq!(size_bucket(4), "4-8");
        assert_eq!(size_bucket(8), "4-8");
        assert_eq!(size_bucket(9), "9+");
        assert_eq!(size_bucket(200), "9+");
    }

    /// The estimator must never emit a negative or absurd duration, whatever it is fed.
    #[test]
    fn estimates_stay_non_negative_on_a_zero_stream() {
        let mut a = None;
        for _ in 0..200 {
            a = Some(fold_sample(a, 0));
        }
        let a = a.unwrap();
        assert_eq!(a.p50_ms, 0);
        assert_eq!(a.p95_ms, 0);
    }
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p cairn-node --lib ui_timing`
Expected: FAIL — the module does not exist.

- [ ] **Step 3: Write the pure estimator**

At the top of `crates/cairn-node/src/ui_timing.rs`:

```rust
//! Node-local gesture timing for the §1.2 paper-parity budget — aggregates ONLY.
//!
//! # Why this shape is the design, not a caveat
//!
//! Per-clinician gesture timings are a productivity-surveillance dataset. Captured the
//! obvious way — user, gesture, duration, timestamp — this table is exactly what a hostile
//! administrator or an acquiring vendor would use to rank clinicians by speed, inside a
//! node the clinician cannot audit. An anti-capture project must not ship that as a side
//! effect of measuring a benchmark. It is also a safety hazard in its own right: clinicians
//! who know they are timed rush the review step the sign-off exists to force.
//!
//! So there is no user id, no patient id, no per-sample row and no timestamp anywhere in
//! this module or its table. There is nothing to re-identify because the identifying
//! columns never exist. What survives is the only thing §1.2 actually needs: what a gesture
//! costs on THIS premise.
//!
//! The rows never sync and never touch the signed clinical event stream — the same category
//! rule the reference-shell design applies to UI preferences (principle 12).

/// A running estimate for one (gesture, list-size) cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Aggregate {
    pub n: i64,
    pub p50_ms: i32,
    pub p95_ms: i32,
}

/// Fold one observed duration into the running estimate.
///
/// Uses online quantile estimation by stochastic gradient descent: for target quantile q,
/// the estimate moves up by `step * q` when the sample is above it and down by
/// `step * (1 - q)` when below, so it settles where a fraction q of samples fall below.
/// Chosen over an exact quantile because exactness needs the raw samples retained — and
/// retaining raw samples is precisely what this design refuses to do.
///
/// The step is proportional to the current estimate (with a floor of 1 ms) so convergence
/// does not depend on whether a gesture takes 50 ms or 50 s.
pub fn fold_sample(prev: Option<Aggregate>, duration_ms: i32) -> Aggregate {
    let duration_ms = duration_ms.max(0);
    let Some(prev) = prev else {
        // The first sample is the best estimate of every quantile there is.
        return Aggregate { n: 1, p50_ms: duration_ms, p95_ms: duration_ms };
    };
    Aggregate {
        n: prev.n.saturating_add(1),
        p50_ms: nudge(prev.p50_ms, duration_ms, 0.50),
        p95_ms: nudge(prev.p95_ms, duration_ms, 0.95),
    }
}

/// One SGD step of a quantile estimate toward a new sample. Never returns a negative.
fn nudge(estimate: i32, sample: i32, q: f64) -> i32 {
    let step = ((estimate as f64) / 20.0).max(1.0);
    let delta = if sample > estimate { step * q } else { -step * (1.0 - q) };
    ((estimate as f64) + delta).round().max(0.0) as i32
}

/// Which size bucket a list of `n` items falls in.
///
/// Coarse on purpose. A finer partition would let someone reconstruct a specific chart's
/// size from the aggregate table, which is the sort of leak this whole module is built to
/// avoid. Three buckets are enough to see whether cost scales with list length.
pub fn size_bucket(items: usize) -> &'static str {
    match items {
        0..=3 => "1-3",
        4..=8 => "4-8",
        _ => "9+",
    }
}
```

- [ ] **Step 4: Run to verify the pure tests pass**

Run: `cargo test -p cairn-node --lib ui_timing`
Expected: PASS, 5 tests.

- [ ] **Step 5: Write the migration**

`db/044_ui_gesture_timing.sql`:

```sql
-- db/044 — node-local UI gesture timing, AGGREGATES ONLY (#288 / §1.2).
--
-- WHY THE COLUMNS THAT ARE NOT HERE MATTER MOST. Per-clinician gesture timings are a
-- productivity-surveillance dataset. There is deliberately NO user id, NO patient id, NO
-- per-sample row and NO timestamp: there is nothing to re-identify because the identifying
-- columns never exist. An anti-capture project must not ship a ready-made monitoring
-- substrate as a side effect of measuring a paper-parity benchmark.
--
-- These rows are NODE-LOCAL. They never sync, and they never touch the append-only signed
-- clinical event stream — mixing UI-tier data into the wire core is a category error
-- (principle 12), and here it would additionally turn a site metric into a person-level
-- record.
--
-- Replay-safe: CREATE TABLE IF NOT EXISTS, and the loader re-runs every db/*.sql on every
-- connect. Nothing here is widened later without a paired ALTER (see #207).
BEGIN;

CREATE TABLE IF NOT EXISTS ui_gesture_timing (
    gesture_kind TEXT   NOT NULL,
    size_bucket  TEXT   NOT NULL,
    n            BIGINT NOT NULL DEFAULT 0,
    p50_ms       INTEGER,
    p95_ms       INTEGER,
    PRIMARY KEY (gesture_kind, size_bucket),
    -- Closed vocabularies: an unrecognised kind or bucket is a bug in the caller, and a
    -- free-text column here would be an invitation to smuggle an identifier in.
    CONSTRAINT ui_gesture_timing_kind_ck   CHECK (gesture_kind IN ('signoff', 'cease')),
    CONSTRAINT ui_gesture_timing_bucket_ck CHECK (size_bucket IN ('1-3', '4-8', '9+')),
    CONSTRAINT ui_gesture_timing_n_ck      CHECK (n >= 0),
    CONSTRAINT ui_gesture_timing_p50_ck    CHECK (p50_ms IS NULL OR p50_ms >= 0),
    CONSTRAINT ui_gesture_timing_p95_ck    CHECK (p95_ms IS NULL OR p95_ms >= 0)
);

GRANT SELECT, INSERT, UPDATE ON ui_gesture_timing TO cairn_agent;

COMMIT;
```

- [ ] **Step 6: Register the migration**

In `crates/cairn-event/src/schema_generation.rs`, change the constant and its doc line:

```rust
/// The numeric prefix of the newest migration in `db/` (`db/044_ui_gesture_timing.sql` → 44).
///
/// Bump this in the same commit that adds a `db/*.sql` file; the guard test enforces it.
pub const SCHEMA_GENERATION: i32 = 44;
```

In `crates/cairn-node/src/db.rs`, append to `SCHEMA` after the `043_deferred_readjudication` entry:

```rust
    // db/044 (#288): node-local, aggregate-only UI gesture timing behind the §1.2
    // paper-parity budget. cairn-sync does not carry it and never should — it is a
    // node-local UI metric with nothing to replicate (#284 documents that the sync
    // subset legitimately lags).
    (
        "044_ui_gesture_timing",
        include_str!("../../../db/044_ui_gesture_timing.sql"),
    ),
```

- [ ] **Step 7: Write the DB-gated round-trip test**

`crates/cairn-node/tests/ui_timing.rs`:

```rust
//! The node-local gesture-timing aggregates (#288 / §1.2): the table holds running
//! estimates and NOTHING that identifies a person, a patient or a moment.
use cairn_node::db;
use cairn_node::ui_timing::{read_aggregates, record_gesture};

fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

#[tokio::test]
async fn recording_a_gesture_creates_then_updates_one_cell() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return };
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let _guard = db::test_serial_guard(&base).await.unwrap();
    c.batch_execute("TRUNCATE ui_gesture_timing").await.unwrap();

    record_gesture(&c, "signoff", 5, 1_200).await.unwrap();
    record_gesture(&c, "signoff", 5, 1_400).await.unwrap();

    let aggregates = read_aggregates(&c).await.unwrap();
    assert_eq!(aggregates.len(), 1, "same kind and bucket -> one cell");
    let ((kind, bucket), agg) = aggregates.into_iter().next().unwrap();
    assert_eq!(kind, "signoff");
    assert_eq!(bucket, "4-8");
    assert_eq!(agg.n, 2);
}

#[tokio::test]
async fn different_buckets_are_separate_cells() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return };
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let _guard = db::test_serial_guard(&base).await.unwrap();
    c.batch_execute("TRUNCATE ui_gesture_timing").await.unwrap();

    record_gesture(&c, "signoff", 2, 900).await.unwrap();
    record_gesture(&c, "signoff", 12, 3_000).await.unwrap();

    assert_eq!(read_aggregates(&c).await.unwrap().len(), 2);
}

/// The privacy shape, asserted rather than merely documented: the table must carry no
/// column that could identify a person, a patient or a moment.
#[tokio::test]
async fn the_table_carries_no_identifying_column() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return };
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let _guard = db::test_serial_guard(&base).await.unwrap();

    let columns: Vec<String> = c
        .query(
            "SELECT column_name FROM information_schema.columns \
             WHERE table_name = 'ui_gesture_timing'",
            &[],
        )
        .await
        .unwrap()
        .iter()
        .map(|r| r.get::<_, String>(0))
        .collect();

    let mut expected = vec!["gesture_kind", "size_bucket", "n", "p50_ms", "p95_ms"];
    expected.sort();
    let mut actual: Vec<&str> = columns.iter().map(String::as_str).collect();
    actual.sort();
    assert_eq!(
        actual, expected,
        "ui_gesture_timing gained a column. If it identifies a person, a patient or a \
         moment, it must not exist — read the module header before changing this test."
    );
}
```

- [ ] **Step 8: Write the persistence functions**

Append to `crates/cairn-node/src/ui_timing.rs`:

```rust
use std::collections::HashMap;

/// Fold one observed gesture into its aggregate cell.
///
/// Read-modify-write in one statement pair under the caller's connection. A lost update
/// under concurrency costs one sample out of thousands, which is immaterial to a running
/// estimate — and a lock here would let a metric stall a clinical gesture, which is not a
/// trade this tier is allowed to make.
pub async fn record_gesture(
    client: &(impl tokio_postgres::GenericClient + Sync),
    gesture_kind: &str,
    list_items: usize,
    duration_ms: i32,
) -> anyhow::Result<()> {
    let bucket = size_bucket(list_items);
    let existing = client
        .query_opt(
            "SELECT n, p50_ms, p95_ms FROM ui_gesture_timing \
             WHERE gesture_kind = $1 AND size_bucket = $2",
            &[&gesture_kind, &bucket],
        )
        .await?
        .map(|row| Aggregate {
            n: row.get("n"),
            p50_ms: row.get::<_, Option<i32>>("p50_ms").unwrap_or(0),
            p95_ms: row.get::<_, Option<i32>>("p95_ms").unwrap_or(0),
        });

    let next = fold_sample(existing, duration_ms);
    client
        .execute(
            "INSERT INTO ui_gesture_timing (gesture_kind, size_bucket, n, p50_ms, p95_ms) \
             VALUES ($1, $2, $3, $4, $5) \
             ON CONFLICT (gesture_kind, size_bucket) DO UPDATE \
             SET n = EXCLUDED.n, p50_ms = EXCLUDED.p50_ms, p95_ms = EXCLUDED.p95_ms",
            &[&gesture_kind, &bucket, &next.n, &next.p50_ms, &next.p95_ms],
        )
        .await?;
    Ok(())
}

/// Every aggregate cell, keyed by (gesture kind, size bucket). This is the whole reporting
/// surface — there is no per-sample read because there are no per-sample rows.
pub async fn read_aggregates(
    client: &(impl tokio_postgres::GenericClient + Sync),
) -> anyhow::Result<HashMap<(String, String), Aggregate>> {
    Ok(client
        .query(
            "SELECT gesture_kind, size_bucket, n, p50_ms, p95_ms FROM ui_gesture_timing",
            &[],
        )
        .await?
        .iter()
        .map(|row| {
            (
                (row.get("gesture_kind"), row.get("size_bucket")),
                Aggregate {
                    n: row.get("n"),
                    p50_ms: row.get::<_, Option<i32>>("p50_ms").unwrap_or(0),
                    p95_ms: row.get::<_, Option<i32>>("p95_ms").unwrap_or(0),
                },
            )
        })
        .collect())
}
```

Add `pub mod ui_timing;` to `crates/cairn-node/src/lib.rs`.

- [ ] **Step 9: Run the schema guards and the new tests**

Run: `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test --workspace`
Expected: PASS — including `full_schema_list_carries_the_repo_generation` (db.rs unit test) and
`crates/cairn-event/tests/schema_generation.rs`, both of which fail loudly if the constant and the
loader list disagree with `db/`.

- [ ] **Step 10: Commit**

```bash
git add db/044_ui_gesture_timing.sql crates/cairn-event/src/schema_generation.rs \
        crates/cairn-node/src/db.rs crates/cairn-node/src/ui_timing.rs \
        crates/cairn-node/src/lib.rs crates/cairn-node/tests/ui_timing.rs
git commit -m "feat(#288): node-local, aggregate-only gesture timing

A guessed paper-parity budget is a magic number; this measures what a gesture
actually costs on THIS premise so the observed p95 can replace the seed.

The columns that are NOT here are the design. No user id, no patient id, no
per-sample row, no timestamp: per-clinician timings are a productivity
surveillance dataset, and an anti-capture project must not ship one as a side
effect of measuring a benchmark. Nothing to re-identify, by construction — and
a test asserts the column set so a later addition has to argue for itself.

Online quantile estimation by SGD rather than exact quantiles, because exactness
would require retaining the raw samples this design refuses to keep."
```

---

## Task 9: The Tauri backend — session key, idle re-lock, commands

**Files:**
- Create: `cairn-gui/cairn-gui-tauri/Cargo.toml`, `tauri.conf.json`, `build.rs`
- Create: `cairn-gui/cairn-gui-tauri/src/main.rs`, `src/state.rs`, `src/commands.rs`
- Modify: `cairn-gui/Cargo.toml` (workspace `members`)

**Interfaces:**
- Consumes: `cairn_node::medication::{read, signoff, cease_medication, AttestParams, AuthorParams, CeaseMedicationInput}`,
  `cairn_node::{db, ui_timing, identity}`, `cairn_gui_tab_medications::build_view`.
- Produces: Tauri commands `med_list`, `unlock`, `lock_state`, `sign_off`, `cease`; and
  `SessionKey::is_expired(&self, now: Instant) -> bool` with `IDLE_TIMEOUT`.

- [ ] **Step 1: Check the dependency licences before adding them**

Run: `cargo deny check licenses` after Step 2's manifest lands; Tauri 2 and its `wry`/`tao` tree are
MIT/Apache-2.0. **If anything in the tree is not AGPL-3.0-compatible, stop and report it** — an
incompatible licence is a blocker, not a cleanup-later item.

- [ ] **Step 2: Create the crate**

`cairn-gui/cairn-gui-tauri/Cargo.toml`:

```toml
[package]
name = "cairn-gui-tauri"
version = "0.1.0"
edition.workspace = true
rust-version.workspace = true
license.workspace = true
publish = false

[build-dependencies]
tauri-build = { version = "2", features = [] }

[dependencies]
cairn-node = { path = "../../crates/cairn-node" }
cairn-event = { path = "../../crates/cairn-event" }
cairn-medication-view = { path = "../../crates/cairn-medication-view" }
cairn-gui-data = { path = "../cairn-gui-data" }
cairn-gui-tab-medications = { path = "../cairn-gui-tabs/cairn-gui-tab-medications" }
tauri = { version = "2", features = [] }
tokio = { version = "1", features = ["rt-multi-thread", "macros", "sync"] }
tokio-postgres = "0.7"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
anyhow = "1"
uuid = { version = "1", features = ["v7"] }
zeroize = { version = "1", features = ["zeroize_derive"] }
clap = { version = "4", features = ["derive", "env"] }
hex = "0.4"
```

Add `"cairn-gui-tauri"` to `members` in `cairn-gui/Cargo.toml`. `build.rs` is the standard
`fn main() { tauri_build::build() }`.

- [ ] **Step 3: Write the failing session-key tests**

At the bottom of `cairn-gui/cairn-gui-tauri/src/state.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// A held key is what makes the whole-list gesture cost ONE act, so how long it is
    /// held is a clinical decision, not a timer detail. Pinned so changing it is visible
    /// in a diff.
    #[test]
    fn the_idle_timeout_is_fifteen_minutes() {
        assert_eq!(IDLE_TIMEOUT, Duration::from_secs(15 * 60));
    }

    #[test]
    fn a_key_is_live_until_the_timeout_elapses() {
        let start = Instant::now();
        let key = SessionKey::for_test(start);
        assert!(!key.is_expired(start));
        assert!(!key.is_expired(start + Duration::from_secs(14 * 60 + 59)));
    }

    #[test]
    fn a_key_expires_at_the_timeout() {
        let start = Instant::now();
        let key = SessionKey::for_test(start);
        assert!(key.is_expired(start + IDLE_TIMEOUT));
        assert!(key.is_expired(start + Duration::from_secs(60 * 60)));
    }

    /// Idle means idle: activity resets the clock, so a clinician working continuously is
    /// never asked to unlock mid-list.
    #[test]
    fn activity_pushes_the_expiry_out() {
        let start = Instant::now();
        let mut key = SessionKey::for_test(start);
        key.touch(start + Duration::from_secs(10 * 60));
        assert!(!key.is_expired(start + Duration::from_secs(20 * 60)));
        assert!(key.is_expired(start + Duration::from_secs(25 * 60)));
    }
}
```

- [ ] **Step 4: Run to verify they fail**

Run: `cd cairn-gui && cargo test -p cairn-gui-tauri && cd ..`
Expected: FAIL — `SessionKey` does not exist.

- [ ] **Step 5: Write the session state**

At the top of `cairn-gui/cairn-gui-tauri/src/state.rs`:

```rust
//! The window's session state — above all, custody of the clinician's signing key.
//!
//! # Why the key is held at all
//!
//! Whole-list sign-off must cost ONE human act (#288). If the passphrase were re-entered
//! per sign-off the gesture would cost two, and the paper counterpart says otherwise: on a
//! drug chart your identity is established by presence, not re-proved at every signature.
//! So the key is unsealed once and held.
//!
//! # Why it is re-locked
//!
//! A held key widens the unattended-workstation window. Paper has the same failure — an
//! open chart on a desk — so this is parity rather than regression, but Cairn should not be
//! WORSE than paper. The key is wiped after `IDLE_TIMEOUT` of no activity, and only the
//! 32-byte seed is retained, inside `Zeroizing`, so the wipe is real rather than a dropped
//! reference the allocator may leave behind.
use std::time::{Duration, Instant};
use zeroize::Zeroizing;

/// How long a held signing key survives without activity.
///
/// A named constant with a test pinning it, so this is a reviewable clinical decision
/// rather than a number buried in a timer.
pub const IDLE_TIMEOUT: Duration = Duration::from_secs(15 * 60);

/// The clinician's unsealed signing key, held for the session.
pub struct SessionKey {
    /// The Ed25519 seed. `SigningKey` is reconstructed per use so the only long-lived copy
    /// is inside `Zeroizing`, which wipes on drop.
    seed: Zeroizing<[u8; 32]>,
    /// Hex key id, safe to display and to log.
    pub kid: String,
    last_activity: Instant,
}

impl SessionKey {
    pub fn new(sk: cairn_event::SigningKey, now: Instant) -> Self {
        let kid = hex::encode(sk.verifying_key().to_bytes());
        Self { seed: Zeroizing::new(sk.to_bytes()), kid, last_activity: now }
    }

    /// Rebuild the usable key. Kept short-lived at every call site.
    pub fn signing_key(&self) -> cairn_event::SigningKey {
        cairn_event::SigningKey::from_bytes(&self.seed)
    }

    pub fn is_expired(&self, now: Instant) -> bool {
        now.duration_since(self.last_activity) >= IDLE_TIMEOUT
    }

    /// Record activity, pushing the expiry out.
    pub fn touch(&mut self, now: Instant) {
        self.last_activity = now;
    }

    /// A key with deterministic, runtime-derived material for the timing tests. Never a
    /// literal (house rule 6) — and never used outside `cfg(test)`.
    #[cfg(test)]
    pub fn for_test(now: Instant) -> Self {
        let seed: [u8; 32] = std::array::from_fn(|i| (i as u8).wrapping_mul(7).wrapping_add(1));
        Self::new(cairn_event::SigningKey::from_bytes(&seed), now)
    }
}
```

And below it, the application state:

```rust
/// Everything a command needs. One database connection, because a single-patient window
/// has no concurrency worth pooling for, and the mutex makes the borrow rules explicit.
pub struct AppState {
    pub db: tokio::sync::Mutex<tokio_postgres::Client>,
    /// The NODE's key — holds custody of every sealed body (ADR-0052) regardless of who
    /// signed the content. Distinct from the clinician's key in `session`.
    pub node_sk: cairn_event::SigningKey,
    pub node_origin: String,
    /// The chart this window is open on. Set once at launch (`--patient`); there is no
    /// patient picker in this slice.
    pub patient: uuid::Uuid,
    /// Path to the clinician's sealed key file, unsealed on `unlock`.
    pub attester_key_path: std::path::PathBuf,
    /// Fixture mode: read from the mock port, refuse writes.
    pub mock: bool,
    pub session: tokio::sync::Mutex<Option<SessionKey>>,
}

impl AppState {
    /// Take the held key if one is live, expiring and WIPING it first if it has gone idle.
    ///
    /// Returns the reconstructed `SigningKey` and its kid. Checking expiry here rather than
    /// at each call site means there is exactly one place a stale key can leak from.
    pub async fn live_key(&self) -> Option<(cairn_event::SigningKey, String)> {
        let mut guard = self.session.lock().await;
        let now = Instant::now();
        if guard.as_ref().is_some_and(|k| k.is_expired(now)) {
            // Dropping the SessionKey drops its Zeroizing seed, which wipes it.
            *guard = None;
        }
        let key = guard.as_mut()?;
        key.touch(now);
        Some((key.signing_key(), key.kid.clone()))
    }
}
```

- [ ] **Step 6: Run to verify the tests pass**

Run: `cd cairn-gui && cargo test -p cairn-gui-tauri && cd ..`
Expected: PASS, 4 tests.

- [ ] **Step 7: Write the commands**

`cairn-gui/cairn-gui-tauri/src/commands.rs`. The two load-bearing ones in full; the other three
follow the same shape.

```rust
//! The window's five commands. Each one is a thin adapter: it resolves state, calls a
//! `cairn-node` function, and maps the result. No clinical logic lives here — that is all
//! in `cairn-medication-view` and `cairn-gui-tab-medications`, under `cargo test`.
use crate::state::{AppState, SessionKey};
use cairn_gui_tab_medications::{build_view, MedListView};
use cairn_medication_view::MedicationRow;
use std::time::Instant;

/// Read the chart and build the view model.
///
/// A display never STARTS stale (the reference-shell freshness rule): this always reads
/// fresh from the projections, and nothing on screen is ever replaced without the clinician
/// asking for it.
#[tauri::command]
pub async fn med_list(state: tauri::State<'_, AppState>) -> Result<MedListView, String> {
    let rows = read_rows(&state).await?;
    Ok(build_view(&rows))
}

/// Sign off every unsigned or stale drug on the chart — the ONE gesture (#288).
#[tauri::command]
pub async fn sign_off(state: tauri::State<'_, AppState>) -> Result<SignOffReport, String> {
    if state.mock {
        return Err("fixture mode: this window is showing mock data and cannot write".into());
    }
    let (human_sk, human_kid) = state
        .live_key()
        .await
        // Not a failure to hide: the clinician needs to know the key re-locked, not to see
        // a gesture silently do nothing.
        .ok_or("your signing key is locked — unlock it to sign off")?;

    // Timed around the WRITE only. The clinician's reading time is measured by the operator
    // runbook, not here: capturing it would mean timing how long someone spent thinking.
    let started = Instant::now();
    let outcome = {
        let mut db = state.db.lock().await;
        let params = cairn_node::medication::AttestParams {
            human_sk: &human_sk,
            human_kid: &human_kid,
            basis: None,
            note: None,
        };
        cairn_node::medication::signoff::sign_off_medication_list(
            &mut db,
            &state.node_sk,
            &state.node_origin,
            &params,
            state.patient,
        )
        .await
        // The underlying text, never a generic string: an in-DB floor refusal is legible on
        // purpose, and "sign-off failed" would throw away the reason.
        .map_err(|e| e.to_string())?
    };
    let elapsed_ms = started.elapsed().as_millis().min(i32::MAX as u128) as i32;

    // Timing is observability, never a gate: a failure to record must not fail the clinical
    // act that succeeded.
    let db = state.db.lock().await;
    if let Err(e) = cairn_node::ui_timing::record_gesture(
        &*db,
        "signoff",
        outcome.attested.len(),
        elapsed_ms,
    )
    .await
    {
        eprintln!("gesture timing not recorded (the sign-off itself succeeded): {e}");
    }

    Ok(SignOffReport { signed: outcome.attested.len() })
}

#[derive(serde::Serialize)]
pub struct SignOffReport {
    pub signed: usize,
}

/// Read the chart rows, from the node or from fixtures.
async fn read_rows(state: &tauri::State<'_, AppState>) -> Result<Vec<MedicationRow>, String> {
    if state.mock {
        use cairn_gui_data::port::ClinicalData;
        return cairn_gui_data::mock::MockData::default()
            .medications(&state.patient.to_string())
            .map_err(|e| format!("{e:?}"));
    }
    let db = state.db.lock().await;
    cairn_node::medication::read::list_patient_medications(&*db, state.patient)
        .await
        .map_err(|e| e.to_string())
}
```

The remaining three:

- **`unlock(state, passphrase) -> Result<String, String>`** — calls the same key-loading routine
  `cairn-node`'s CLI uses for `--attester-key`, stores `SessionKey::new(sk, Instant::now())` in
  `state.session`, and returns the first 8 hex characters of the kid so the window can show *who* is
  signed in.
- **`lock_state(state) -> Result<bool, String>`** — calls `state.live_key()` and reports whether a
  key survived. The frontend polls it, so the window shows the lock rather than the clinician
  discovering it when a sign-off is refused.
- **`cease(state, group_id, reason) -> Result<(), String>`** — refuses in mock mode and without a
  live key exactly as `sign_off` does; resolves the group's member threads from `read_rows`; calls
  `cairn_node::medication::cease_medication` once per member, passing
  `AuthorParams { human_sk, human_kid }` so ADR-0053 per-write human authorship holds; then records
  `record_gesture(&*db, "cease", 1, elapsed_ms)` under the same never-gate rule.

`src/main.rs` parses `--patient <uuid>`, `--conn`, `--key`, `--attester-key` and `--mock` with clap,
builds `AppState`, and registers the five commands with `tauri::generate_handler!`.

- [ ] **Step 8: Build**

Run: `cd cairn-gui && cargo build -p cairn-gui-tauri && cd ..`
Expected: compiles clean with no warnings.

- [ ] **Step 9: Commit**

```bash
git add cairn-gui/Cargo.toml cairn-gui/cairn-gui-tauri cairn-gui/Cargo.lock
git commit -m "feat(#288): the Tauri backend — session key custody and the five commands

The key is unsealed once and held, because re-entering a passphrase per sign-off
would make the gesture cost two acts and the paper counterpart says otherwise:
on a drug chart your identity comes from presence, not from re-proving it at
every signature. It is wiped after 15 minutes idle, and only the 32-byte seed is
retained inside Zeroizing so the wipe is real.

Commands return the underlying error text, never a generic string: an in-DB
floor refusal is legible on purpose."
```

---

## Task 10: The webview — semantic HTML

**Files:**
- Create: `cairn-gui/cairn-gui-tauri/src-ui/index.html`, `main.ts`, `style.css`

**Interfaces:**
- Consumes: the `MedListView` JSON shape from Task 7 and the commands from Task 9.
- Produces: the rendered window.

- [ ] **Step 1: Write the markup skeleton**

`index.html` uses **native semantic elements only** — this is the whole reason the reference UI moved
to Tauri, so the accessibility tree comes from the browser rather than from a framework:

```html
<main>
  <h1 id="patient-heading">Medications</h1>
  <p id="lock-state" role="status" aria-live="polite"></p>
  <table id="med-table">
    <caption>Current and ceased medications, with the clinician responsible for each</caption>
    <thead>
      <tr>
        <th scope="col">Medication</th>
        <th scope="col">Dose</th>
        <th scope="col">Status</th>
        <th scope="col">Signature</th>
        <th scope="col">Action</th>
      </tr>
    </thead>
    <tbody id="med-rows"></tbody>
  </table>
  <p id="empty-message"></p>
  <button id="sign-off" type="button"></button>
</main>
```

- [ ] **Step 2: Write the renderer**

`main.ts` renders `MedListView` and does no clinical reasoning of its own:

```ts
// The webview renders and decides nothing. Every clinical display question was already
// answered in Rust under cargo test (cairn-gui-tab-medications); re-deriving any of it
// here would put an untested second answer on screen.
import { invoke } from "@tauri-apps/api/core";

interface MedListRowView {
  group_id: string; primary: string; dose: string; formulation: string;
  sig: string; started: string; status_label: string; vouch_label: string;
  will_be_signed: boolean; can_cease: boolean; flags: string[];
}
interface MedListView {
  rows: MedListRowView[]; sign_off_count: number;
  sign_off_enabled: boolean; empty_message: string | null;
}

function render(view: MedListView): void {
  const body = document.getElementById("med-rows") as HTMLTableSectionElement;
  body.replaceChildren();
  for (const row of view.rows) {
    const tr = document.createElement("tr");
    // The row that WILL be signed is marked in the DOM, not only by colour: colour alone
    // is invisible to a screen reader and to a colour-blind clinician.
    if (row.will_be_signed) tr.setAttribute("data-will-sign", "true");
    if (row.status_label === "ceased") tr.setAttribute("data-ceased", "true");

    tr.append(
      cell("th", row.primary, { scope: "row" }),
      cell("td", row.dose),
      cell("td", row.status_label),
      cell("td", row.will_be_signed ? `${row.vouch_label} — will be signed` : row.vouch_label),
    );

    const action = document.createElement("td");
    if (row.can_cease) {
      const stop = document.createElement("button");
      stop.type = "button";
      stop.id = `cease-${row.group_id}`;
      // A per-row accessible name: "Stop" alone is ambiguous when a screen reader reads
      // the buttons out of table context.
      stop.textContent = "Stop";
      stop.setAttribute("aria-label", `Stop ${row.primary}`);
      stop.addEventListener("click", () => cease(row.group_id));
      action.append(stop);
    }
    tr.append(action);

    for (const flag of row.flags) {
      const note = document.createElement("tr");
      const td = document.createElement("td");
      td.colSpan = 5;
      td.textContent = `! ${flag}`;
      note.append(td);
      body.append(tr, note);
    }
    if (row.flags.length === 0) body.append(tr);
  }

  const button = document.getElementById("sign-off") as HTMLButtonElement;
  button.disabled = !view.sign_off_enabled;
  // The count is the number of THREADS, which a reconciled group can make larger than the
  // number of visible rows. Saying the real number is the honest thing to sign.
  button.textContent = view.sign_off_enabled
    ? `Sign off ${view.sign_off_count} unsigned medication(s)`
    : "Nothing to sign off";

  const empty = document.getElementById("empty-message") as HTMLParagraphElement;
  empty.textContent = view.empty_message ?? "";
}

function cell(tag: string, text: string, attrs: Record<string, string> = {}): HTMLElement {
  const el = document.createElement(tag);
  el.textContent = text;
  for (const [k, v] of Object.entries(attrs)) el.setAttribute(k, v);
  return el;
}

async function refresh(): Promise<void> {
  render(await invoke<MedListView>("med_list"));
}

async function cease(groupId: string): Promise<void> {
  await invoke("cease", { groupId, reason: null });
  await refresh();
}

document.getElementById("sign-off")!.addEventListener("click", async () => {
  await invoke("sign_off");
  await refresh();
});

void refresh();
```

- [ ] **Step 3: Style without colour-only signalling**

`style.css` marks `[data-will-sign="true"]` with a left border *and* the textual "— will be signed"
already in the cell, and `[data-ceased="true"]` with `text-decoration: line-through` — a struck line,
as on paper. Respect `prefers-color-scheme` and never encode meaning in hue alone.

- [ ] **Step 4: Run the window against the mock**

Run: `cd cairn-gui && cargo run -p cairn-gui-tauri -- --mock --patient 00000000-0000-0000-0000-000000000001`
Expected: a window listing the fixture drugs, ceased ones struck through, the sign-off button naming
a real count.

- [ ] **Step 5: Commit**

```bash
git add cairn-gui/cairn-gui-tauri/src-ui
git commit -m "feat(#288): the med-list webview — native semantic HTML

Native table/th/button markup, because the browser's accessibility tree is the
entire reason the reference UI moved off iced. The renderer does no clinical
reasoning: every display question was answered in Rust under cargo test, and a
second answer here would be an untested one on screen.

Nothing is signalled by colour alone — the row that will be signed says so in
its text, and a ceased drug is struck through as on paper."
```

---

## Task 11: Wire the timing capture and record the first measurement

**Files:**
- Create: `cairn-gui/cairn-gui-tauri/results/RUNBOOK.md`, `results/TEMPLATE.md`
- Create: `cairn-gui/cairn-gui-tauri/results/2026-08-02-<host>.md`

**Interfaces:**
- Consumes: everything above.
- Produces: a recorded paper-parity measurement.

- [ ] **Step 1: Confirm the timing calls are in place**

Run: `grep -n "record_gesture" cairn-gui/cairn-gui-tauri/src/commands.rs`
Expected: two hits — one in `sign_off`, one in `cease`. If either is missing, add it (Task 9 Step 7).

- [ ] **Step 2: Write the runbook**

`results/RUNBOOK.md` gives the operator the exact sequence: seed a fixture patient with five drugs
of which three are unsigned (the `medication-assert` and `medication-attest` commands, spelled out),
launch the window against the real node, perform the measured gestures, then read the aggregates
back with a `SELECT * FROM ui_gesture_timing`. It states plainly that measurement **excludes finding
the patient** (the window launches with `--patient`, and the §5.3/§5.8 search funnel is unbuilt), and
that the accessibility pass is a separate live screen-reader run against the same window.

- [ ] **Step 3: Run the measurement and record it**

Fill `results/TEMPLATE.md` into a dated file: host, database, list size, observed p50/p95 for each
gesture, the screen-reader verdict, and — required — **whether the observed p95 falls inside the
provisional 15 s / 5 s budget**. If it does not, that is the finding: record it and file an issue
rather than adjusting the budget to match.

- [ ] **Step 4: Commit**

```bash
git add cairn-gui/cairn-gui-tauri/results
git commit -m "docs(#288): paper-parity runbook and the first recorded measurement

The provisional 15s/5s budget was a seeded figure. This records what the gesture
actually costs on real hardware against a real node, which is what the §1.2 time
limb is owed. Measurement excludes finding the patient and says so."
```

---

## Task 12: Update HANDOVER and ROADMAP

**Files:**
- Modify: `docs/HANDOVER.md`, `docs/ROADMAP.md`

- [ ] **Step 1: Correct the stale shell claim**

`docs/HANDOVER.md`'s "Built so far" reads *"the L3 reference-UI shell, slice 1 … pivot to Tauri 2;
PR #174"*, which reads as though a Tauri shell existed. It did not — PR #174 shipped an **iced**
shell that failed the a11y bar. Replace with an honest line naming what this slice actually built.

- [ ] **Step 2: Add the slice to ROADMAP and refresh HANDOVER's ⇒ NEXT**

Record: the first clinical read path, one-gesture sign-off, the Tauri window, the retired iced layer,
the aggregate-only timing table, and the measured-vs-provisional budget outcome. List what is
deliberately not done (issues #331, #332, the unwired pane state machine, no patient picker) and the
`--patient` launch constraint. **Prune both files back under 500 lines** while doing it.

- [ ] **Step 3: Commit**

```bash
git add docs/HANDOVER.md docs/ROADMAP.md
git commit -m "docs: record the med-list UI slice; correct the Tauri-shell claim

HANDOVER read as though a Tauri shell existed from PR #174. It did not — that PR
shipped an iced shell which then failed the accessibility bar. Corrected, and the
slice's real state recorded including what it deliberately left undone."
```

---

## Paper-parity benchmark (§1.2)

**Paper counterpart:** the **inpatient drug chart** — review the list, sign the lines that lack the
responsible clinician's signature, and strike a drug being stopped. Chosen because it is the durable
paper artifact clinicians actually sign, and because its per-line signature model is what determines
who this slice's sign-off attests (a thread already carrying a current vouch keeps its signatory).

**Steps:**

| Act | Paper *N* | Architecture-forced *M* | UI bundling target *K* |
|---|---|---|---|
| Review a 5-drug list, sign 3 unsigned/stale lines | 3 signatures | **1** | **1** |
| Cease one drug | 2 (strike + initial/date) | **1** | **1** |

`M ≤ N` on both limbs, so there is no architecture defect to file under the #217 rule. The N
per-thread attestations a 3-target sign-off authors are a cryptographic artifact of ADR-0049's
set-commitment model, not human acts: `attest_thread_in_tx` takes an already-unsealed key by
reference, so one unseal (Task 9) and one transaction (Task 3) cover all N. Task 3's
`one_gesture_attests_every_unvouched_thread` and Task 7's
`rows_that_will_be_signed_match_the_sign_off_count` are the falsifying tests.

**Time + cognitive load:** provisional budget — chart open → list rendered → unsigned lines signed
**≤ 15 s** for a 5-drug list; one cease **≤ 5 s**. These are seeded, **not** measured, figures. Task
11 measures them on real hardware and records the result; Task 8's aggregate-only capture keeps
measuring them in use, so the observed p95 supersedes the seed rather than the seed standing forever.
Measurement excludes finding the patient (the window launches with `--patient`; the §5.3/§5.8
search-before-create funnel is unbuilt) and every recorded run states that exclusion.

The cognitive-load limb is discharged by the per-row vouch state: the clinician never has to hold in
their head which lines they have already signed, and can see which lines the gesture will sign before
making it — the paper affordance of looking at the unsigned lines, not a confirmation dialog
(principle 3).

---

## Deliberate gaps

1. **No patient picker** — `--patient <uuid>` at launch; §5.3/§5.8 funnel unbuilt.
2. **A nil list cannot be signed off** — [#331](https://github.com/cairn-ehr/cairn-ehr/issues/331);
   the leading candidate is a reserved "nil" code, conditional on list-scoped staleness.
3. **The pane/routing/freshness state machine is kept but unwired** — tested code awaiting the next
   slice, not dead code.
4. **No native API** — the DB-direct read is the ADR-0021 privilege gradient, documented in the spec.
5. **DOM accessibility is verified by an operator, not CI** —
   [#332](https://github.com/cairn-ehr/cairn-ehr/issues/332).
6. **No dose editing, prescribing, or reconciliation from the UI** — read, sign off, cease.
