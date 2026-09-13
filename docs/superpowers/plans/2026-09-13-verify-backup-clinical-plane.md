# `verify-backup` asks the clinical-plane question — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `cairn-node verify-backup` reports what the clinical half of a restore would bring back, and fails with `backup SHORT` when this node's own backup sidecar proves the medium holds less than the last backup wrote to that path.

**Architecture:** One new pure module, `crates/cairn-node/src/backup/clinical_verdict.rs`, decides the summary line, the advisory and the refusal from facts the caller gathers. The `Cmd::VerifyBackup` arm in `main.rs` gathers those facts (clinical accounting, legacy flag, sidecar evidence) and prints or bails. A `cairn-medium` test pins the invariant that makes the "untrusted records" notice unnecessary here, and DB-gated CLI tests drive the real binary.

**Tech Stack:** Rust (workspace edition, `warnings = "deny"`), `cairn-medium`, `tokio-postgres` test fixtures, PostgreSQL 18 with `cairn_pgx` for DB-gated tests.

**Spec:** `docs/superpowers/specs/2026-09-13-verify-backup-clinical-plane-design.md` (approved 2026-09-13). Executors read both.

## Global Constraints

- Paper-parity: not clinical-surface — an unattended operator health check's exit code; it adds no human act.
- **No migration, no `SCHEMA_GENERATION` bump, no wire/medium/sidecar format change, no new CLI flag, no ADR, no spec-version bump** (the spec version stays `0.71`; this slice changes no decision).
- **Exit-code policy (maintainer decision, 2026-09-13): fail only on evidence.** Refuse iff the sidecar describes the medium named by `--from` (`health_describes_medium`) AND records `clinical_watermark = Some(s)` AND the medium's newest trusted clinical `source_seq` is `None` or `< s`. Everything else: print, exit unaffected.
- Operator strings, verbatim: summary `clinical-plane records OK: {n} verified, newest seq {s}` (+ `, {k} byte-identical re-capture(s) collapsed` when k > 0); `clinical plane: EMPTY — …`; `clinical plane: NONE — …`; refusal starts `backup SHORT:`.
- House rules: TDD (failing test first); inline docs a junior can follow (why, not just what); pure functions; new files under 500 lines; never hard-code crypto material in tests (derive at runtime, and never name a non-crypto value `salt`/`nonce`/`iv`).
- `warnings = "deny"`: an unused import or dead helper is a build failure. Shared test fixtures carry `#![allow(dead_code)]` like `tests/common/mod.rs` does.
- **Never `git checkout -- <file>` to undo a mutation.** Copy the file to the scratchpad first and `cp` it back; run `git status --short` afterwards. Every task's shell steps assume `SCR=/private/tmp/claude-501/-Users-hherb-src-cairn-ehr/fb12adf1-0109-4a3e-ac33-c3824607041a/scratchpad` is set.
- **Never pipe `cargo test` into `tail`/`head`** (it masks the exit code). If a narrow `cargo test --test X` stalls on a freshly linked binary (macOS Gatekeeper), run the printed `target/debug/deps/X-<hash>` directly. If cargo blocks on the target-dir lock (the IDE's rust-analyzer holds it), set `CARGO_TARGET_DIR=/tmp/cairn-target-567`.
- DB-gated tests: `export CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test"` (PG18 + `cairn_pgx` on this Mac). Without it the DB tests self-skip, and since #450 a bare run fails unless `CAIRN_ALLOW_DB_SKIP=1`.
- Commit messages use `feat(#567):` / `test(#567):` / `docs(#567):` (the parenthesis keeps GitHub's closing-keyword parser from firing). End every commit with `Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>`.

---

## File map

| file | change | responsibility |
|---|---|---|
| `crates/cairn-medium/src/health/tests.rs` | modify (append) | pins *a medium that gates records out is never sound* |
| `crates/cairn-node/src/backup.rs` | modify | declares `pub mod clinical_verdict;`; splits `straddled_positions` + `describe_positions` out of `straddled_duplicate_notice` (output unchanged) |
| `crates/cairn-node/src/backup/clinical_verdict.rs` | **create** | the pure verdict, the shortfall rule, the one impure evidence adapter, their unit tests |
| `crates/cairn-node/tests/common/clinic_kit.rs` | **create** | the CLI clinic fixtures, moved out of `verify_backup_scope.rs` unchanged apart from `pub` |
| `crates/cairn-node/tests/verify_backup_scope.rs` | modify | uses `clinic_kit`; one new stdout assertion in the restorable-kit test |
| `crates/cairn-node/tests/verify_backup_clinical_plane.rs` | **create** | the three new CLI tests |
| `crates/cairn-node/src/main.rs` | modify | the `Cmd::VerifyBackup` arm: wiring + rewritten scope comment |
| `docs/spec/security.md`, `crates/cairn-node/src/{localstate.rs,localstate_read.rs}`, `crates/cairn-node/tests/medium_point_in_time.rs`, the design doc | modify | scope clause; retire the stale "slice 2e" references; spec erratum on the advisory wording |
| `docs/HANDOVER.md`, `docs/ROADMAP.md` | modify | session record |

---

### Task 1: Pin the invariant that makes the untrusted-records notice unreachable in `verify-backup`

**Files:**
- Modify: `crates/cairn-medium/src/health/tests.rs` (append at end of file)

**Interfaces:**
- Consumes: `testkit::unsigned_chain_of(n) -> MediumV3`, `testkit::bytes(seed, len)`, `assess`, `crate::chain::plane_records_with_accounting`.
- Produces: test `a_medium_that_gates_records_out_is_never_sound` (referenced by name from Task 2's and Task 5's doc comments).

This is a **characterisation pin** on existing behaviour, so it passes on first run. Its TDD proof is the mutation in Step 3, which must turn it red.

- [ ] **Step 1: Append the test**

```rust
/// **A medium that holds records back from a restore is never SOUND** (#567).
///
/// `cairn-node verify-backup` deliberately does NOT print `untrusted_clinical_notice` — the
/// warning `restore` prints when records sit past the last verified chain link — and this
/// invariant is the whole reason: every fault that stops `verified_through` advancing ALSO lands
/// in `faults`, so `sound()` is false and `verify-backup` has already refused the medium as
/// UNSOUND before it could print any clinical all-clear.
///
/// **If this test fails**, some fault now retracts `verified_through` without failing `sound()`,
/// and `verify-backup` would print `clinical-plane records OK` over a medium whose tail a
/// restore will not trust. Do not weaken this test: wire `untrusted_clinical_notice` into the
/// `Cmd::VerifyBackup` arm of `crates/cairn-node/src/main.rs`.
///
/// The fixture is UNSIGNED on purpose. On a signed segment, mangling `prev_commitment` also
/// invalidates the attestation, so a `ChainBroken` that stopped failing `sound()` would still be
/// caught by `AttestationInvalid` and this arm would pass for the wrong reason (see
/// `chain/tests.rs`, `a_chain_break_is_located_not_merely_counted`, I8).
///
/// Every arm asserts `gated > 0` FIRST — the positive control. A mutation that retracted nothing
/// would satisfy "not (sound and gated)" vacuously and prove nothing.
#[test]
fn a_medium_that_gates_records_out_is_never_sound() {
    type M = crate::container::MediumV3;
    type Break = fn(&mut M);
    let breaks: [(&str, Break); 4] = [
        ("a broken chain link", |m: &mut M| {
            m.segments[1].prev_commitment = "deadbeef".into()
        }),
        ("a segment lying about its index", |m: &mut M| {
            m.segments[1].index = 9
        }),
        ("an empty segment", |m: &mut M| m.segments[1].records.clear()),
        ("an attestation that does not verify", |m: &mut M| {
            m.segments[1].attestation = Some(testkit::bytes(9, 64))
        }),
    ];

    let clean = testkit::unsigned_chain_of(4);
    let h = assess(&clean);
    assert!(h.sound(), "the fixture must start sound: {:?}", h.chain.faults);
    assert_eq!(gated_out(&clean, &h), 0, "and hold nothing back");

    for (what, break_it) in breaks {
        let mut m = testkit::unsigned_chain_of(4);
        break_it(&mut m);
        let h = assess(&m);
        let gated = gated_out(&m, &h);
        assert!(
            gated > 0,
            "{what}: the fixture must actually hold records back, or this arm proves nothing"
        );
        assert!(
            !h.sound(),
            "{what}: {gated} record(s) are held back from a restore, yet the medium reports \
             SOUND — `verify-backup` would print a clinical all-clear over them. Wire \
             `untrusted_clinical_notice` into it; do not weaken this test."
        );
    }
}

/// Records held back by the trust gate, across both planes this build routes.
fn gated_out(m: &crate::container::MediumV3, h: &MediumHealth) -> usize {
    [crate::segment::Plane::Node, crate::segment::Plane::Clinical]
        .into_iter()
        .map(|plane| crate::chain::plane_records_with_accounting(m, &h.chain, plane).gated_out)
        .sum()
}
```

- [ ] **Step 2: Run it — expect PASS**

Run: `cargo test -p cairn-medium --lib a_medium_that_gates_records_out_is_never_sound`
Expected: `test result: ok. 1 passed`.

- [ ] **Step 3: Prove it can fail (mutation)**

```bash
SCR=/private/tmp/claude-501/-Users-hherb-src-cairn-ehr/fb12adf1-0109-4a3e-ac33-c3824607041a/scratchpad
cp crates/cairn-medium/src/health.rs "$SCR/health.rs.orig"
```
In `crates/cairn-medium/src/health.rs`, change the body of `sound()` from
`self.chain.chain_intact() && self.records.all_intact() && !self.truncated_tail`
to `self.records.all_intact() && !self.truncated_tail`.

Run: `cargo test -p cairn-medium --lib a_medium_that_gates_records_out_is_never_sound`
Expected: FAIL, message starting `a broken chain link: 3 record(s) are held back`.

Restore it and confirm the tree is clean apart from the test:
```bash
cp "$SCR/health.rs.orig" crates/cairn-medium/src/health.rs
git status --short   # only crates/cairn-medium/src/health/tests.rs listed
cargo test -p cairn-medium --lib
```
Expected: every `cairn-medium` lib test passes.

- [ ] **Step 4: Commit**

```bash
git add crates/cairn-medium/src/health/tests.rs
git commit -m "test(#567): a medium that holds records back from a restore is never sound

verify-backup does not print untrusted_clinical_notice, and this is why:
every fault that retracts verified_through also fails sound(), so the
command has already refused the medium. Mutation (sound() ignoring the
chain) turns it red.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

### Task 2: The pure clinical-plane verdict

**Files:**
- Create: `crates/cairn-node/src/backup/clinical_verdict.rs`
- Modify: `crates/cairn-node/src/backup.rs` — declare the module near the top (after the `use` lines ending at `use crate::medium::{MediumImage, Plane};`, line ~31), and split `straddled_duplicate_notice` (lines ~155–199).

**Interfaces:**
- Consumes: `cairn_medium::{PlaneRecords, MediumRecord}`; `super::{BackupHealth, health_describes_medium}`.
- Produces (used by Task 4 in `main.rs`):
  - `pub fn straddled_positions(records: &[cairn_medium::MediumRecord]) -> Vec<i64>` (in `backup.rs`)
  - `pub fn describe_positions(positions: &[i64]) -> String` (in `backup.rs`)
  - `pub struct ClinicalPlaneFacts<'a> { pub accounting: &'a cairn_medium::PlaneRecords, pub legacy: bool, pub recorded_for_this_medium: Option<i64> }`
  - `pub struct ClinicalPlaneVerdict { pub summary: String, pub advisory: Option<String>, pub refusal: Option<String> }` (derives `Debug, Clone, PartialEq, Eq`)
  - `pub fn clinical_plane_verdict(facts: &ClinicalPlaneFacts<'_>) -> ClinicalPlaneVerdict`
  - `pub fn shortfall(medium_newest: Option<i64>, recorded: Option<i64>) -> Option<i64>`
  - `pub fn recorded_watermark_for(health: Option<&BackupHealth>, medium: &std::path::Path) -> Option<i64>`

- [ ] **Step 1: Split `straddled_duplicate_notice` without changing its output (refactor under existing tests)**

In `crates/cairn-node/src/backup.rs`, replace the body of `straddled_duplicate_notice` and add two functions directly above its doc comment:

```rust
/// PURE. Every `source_seq` at which `records` holds two or more records, ascending, each once.
///
/// Only meaningful on the TRUSTED set from [`clinical_plane_accounting`], where wholly identical
/// re-capture duplicates are already collapsed — so a repeated position here is always two
/// DIFFERENT records. Shared by `restore`'s [`straddled_duplicate_notice`] and `verify-backup`'s
/// advisory (`clinical_verdict`), which word the same finding for different moments: after an
/// apply, and before anything has been applied.
pub fn straddled_positions(records: &[cairn_medium::MediumRecord]) -> Vec<i64> {
    let mut seqs: Vec<i64> = records.iter().map(|r| r.source_seq).collect();
    seqs.sort_unstable();
    let mut repeated: Vec<i64> = Vec::new();
    for pair in seqs.windows(2) {
        if pair[0] == pair[1] && repeated.last() != Some(&pair[0]) {
            repeated.push(pair[0]);
        }
    }
    repeated
}

/// PURE. The first ten positions, comma-separated, then `" (and N more)"` if there are more —
/// so an operator message stays readable on a medium with thousands of them.
pub fn describe_positions(positions: &[i64]) -> String {
    let shown: Vec<String> = positions.iter().take(10).map(|s| s.to_string()).collect();
    let more = if positions.len() > 10 {
        format!(" (and {} more)", positions.len() - 10)
    } else {
        String::new()
    };
    format!("{}{more}", shown.join(", "))
}
```

and the notice body becomes:

```rust
pub fn straddled_duplicate_notice(records: &[cairn_medium::MediumRecord]) -> Option<String> {
    let repeated = straddled_positions(records);
    if repeated.is_empty() {
        return None;
    }
    Some(format!(
        "WARNING: this medium holds two or more DIFFERENT records at the same source \
         position(s): {}. A byte-identical re-capture is collapsed silently and is \
         expected; these differ — typically a capture that straddled an unwrap-key rotation \
         or a crypto-shred, so the copies disagree about CUSTODY. All of them were applied \
         (the apply door is idempotent and refuses custody for an already-shredded target, so \
         nothing erased can come back). Review these positions: only you can tell which \
         capture reflects what the dead node actually held.",
        describe_positions(&repeated)
    ))
}
```

Run: `cargo test -p cairn-node --lib backup::tests`
Expected: PASS (including `two_different_records_at_one_seq_are_named_for_the_operator`). The notice text is byte-identical to before.

- [ ] **Step 2: Declare the module and write the failing tests**

In `crates/cairn-node/src/backup.rs`, after the top-of-file `use` block:

```rust
/// What `verify-backup` says about a medium's clinical plane, and when it refuses (#567).
pub mod clinical_verdict;
```

Create `crates/cairn-node/src/backup/clinical_verdict.rs` containing ONLY the test module for now, plus stub signatures that `todo!()` so it compiles and the tests fail:

```rust
//! (module docs are written in Step 4)

use std::path::Path;

use cairn_medium::PlaneRecords;

use super::BackupHealth;

pub struct ClinicalPlaneFacts<'a> {
    pub accounting: &'a PlaneRecords,
    pub legacy: bool,
    pub recorded_for_this_medium: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClinicalPlaneVerdict {
    pub summary: String,
    pub advisory: Option<String>,
    pub refusal: Option<String>,
}

pub fn clinical_plane_verdict(_facts: &ClinicalPlaneFacts<'_>) -> ClinicalPlaneVerdict {
    todo!()
}

pub fn shortfall(_medium_newest: Option<i64>, _recorded: Option<i64>) -> Option<i64> {
    todo!()
}

pub fn recorded_watermark_for(_health: Option<&BackupHealth>, _medium: &Path) -> Option<i64> {
    todo!()
}

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_medium::MediumRecord;

    /// Runtime-derived bytes for a fixture field — never a literal (house rule 6: a byte
    /// literal in a custody field trips CodeQL's hard-coded-cryptographic-value query).
    fn filler(seed: u8, len: usize) -> Vec<u8> {
        (0..len).map(|i| seed.wrapping_add(i as u8)).collect()
    }

    /// One record at `seq`. `variant` changes the custody sidecar, so two records at one seq
    /// with different variants are DIFFERENT records (a straddled re-capture).
    fn rec(seq: i64, variant: Option<u8>) -> MediumRecord {
        MediumRecord {
            signed_bytes: filler(1, 8),
            attestation: None,
            attester_key: None,
            dek_wrapped: variant.map(|v| filler(v, 16)),
            source_seq: seq,
        }
    }

    fn plane(records: Vec<MediumRecord>, collapsed: usize) -> PlaneRecords {
        PlaneRecords {
            records,
            gated_out: 0,
            collapsed,
        }
    }

    fn verdict(acc: &PlaneRecords, legacy: bool, recorded: Option<i64>) -> ClinicalPlaneVerdict {
        clinical_plane_verdict(&ClinicalPlaneFacts {
            accounting: acc,
            legacy,
            recorded_for_this_medium: recorded,
        })
    }

    fn health_for(medium_path: &str, clinical_watermark: Option<i64>) -> BackupHealth {
        BackupHealth {
            version: super::super::SUPPORTED_HEALTH_VERSION,
            last_backup_unix: 0,
            medium_path: medium_path.into(),
            medium_bytes: 0,
            node_events: 1,
            clinical_events: 0,
            clinical_watermark,
            export_covers_seq: None,
            extra: serde_json::Map::new(),
        }
    }

    // --- the shortfall rule, alone -------------------------------------------------------

    #[test]
    fn no_evidence_is_never_a_shortfall() {
        assert_eq!(shortfall(None, None), None);
        assert_eq!(shortfall(Some(40), None), None);
    }

    #[test]
    fn an_empty_medium_against_evidence_is_short() {
        assert_eq!(shortfall(None, Some(40)), Some(40));
    }

    #[test]
    fn a_medium_behind_the_evidence_is_short() {
        assert_eq!(shortfall(Some(39), Some(40)), Some(40));
    }

    /// The boundary: holding exactly what the last backup recorded is complete.
    #[test]
    fn a_medium_level_with_the_evidence_is_not_short() {
        assert_eq!(shortfall(Some(40), Some(40)), None);
    }

    /// A backup that wrote the medium durably and then failed to write its sidecar leaves the
    /// medium AHEAD of the evidence. Holding more than recorded is not a shortfall.
    #[test]
    fn a_medium_ahead_of_the_evidence_is_not_short() {
        assert_eq!(shortfall(Some(41), Some(40)), None);
    }

    // --- the verdict ---------------------------------------------------------------------

    #[test]
    fn records_present_without_evidence_report_count_and_newest_seq() {
        let acc = plane(vec![rec(3, None), rec(5, None), rec(8, None)], 0);
        let v = verdict(&acc, false, None);
        assert_eq!(v.summary, "clinical-plane records OK: 3 verified, newest seq 8");
        assert_eq!(v.advisory, None);
        assert_eq!(v.refusal, None);
    }

    #[test]
    fn collapsed_re_captures_are_named_in_the_summary() {
        let acc = plane(vec![rec(1, None), rec(2, None)], 4);
        let v = verdict(&acc, false, None);
        assert_eq!(
            v.summary,
            "clinical-plane records OK: 2 verified, newest seq 2, 4 byte-identical \
             re-capture(s) collapsed"
        );
    }

    #[test]
    fn a_straddled_duplicate_is_advisory_and_never_refuses() {
        let acc = plane(vec![rec(7, Some(4)), rec(7, None), rec(8, None)], 0);
        let v = verdict(&acc, false, Some(8));
        let advisory = v.advisory.expect("two different records at one seq must be named");
        assert!(advisory.contains(": 7."), "the position is named: {advisory}");
        assert!(
            advisory.contains("A restore would apply all of them"),
            "worded for a check that has applied nothing: {advisory}"
        );
        assert!(
            !advisory.contains("were applied"),
            "restore's past tense would be false here: {advisory}"
        );
        assert_eq!(v.refusal, None, "a straddle is not a restorability failure");
    }

    #[test]
    fn an_empty_plane_without_evidence_says_empty_and_does_not_refuse() {
        let acc = plane(vec![], 0);
        let v = verdict(&acc, false, None);
        assert!(v.summary.starts_with("clinical plane: EMPTY — "), "{}", v.summary);
        assert!(v.summary.contains("restore NO patient data"), "{}", v.summary);
        assert_eq!(v.refusal, None, "a fresh clinic's empty plane is a correct backup");
    }

    #[test]
    fn a_legacy_medium_without_evidence_says_none_and_does_not_refuse() {
        let acc = plane(vec![], 0);
        let v = verdict(&acc, true, None);
        assert!(v.summary.starts_with("clinical plane: NONE — "), "{}", v.summary);
        assert!(v.summary.contains("CAIRNB1/CAIRNB2"), "{}", v.summary);
        assert_eq!(v.refusal, None);
    }

    #[test]
    fn an_empty_plane_against_evidence_refuses_naming_both_sides() {
        let acc = plane(vec![], 0);
        let v = verdict(&acc, false, Some(4812));
        let refusal = v.refusal.expect("the sidecar proves this path held clinical events");
        assert!(refusal.starts_with("backup SHORT: "), "{refusal}");
        assert!(refusal.contains("through seq 4812"), "{refusal}");
        assert!(refusal.contains("no clinical records at all"), "{refusal}");
        assert!(refusal.contains("run `backup --to`"), "the remedy: {refusal}");
        assert!(
            v.summary.starts_with("clinical plane: EMPTY"),
            "the plane line still prints first: {}",
            v.summary
        );
    }

    #[test]
    fn a_plane_behind_evidence_refuses_naming_both_seqs() {
        let acc = plane(vec![rec(1, None), rec(30, None)], 0);
        let refusal = verdict(&acc, false, Some(40)).refusal.expect("30 < 40");
        assert!(refusal.contains("through seq 40"), "{refusal}");
        assert!(refusal.contains("only through seq 30"), "{refusal}");
    }

    /// A legacy file at a path where this node last wrote clinical events is not what that
    /// backup wrote either: `backup` converts a legacy medium to CAIRNB3 on its next capture.
    #[test]
    fn a_legacy_medium_against_evidence_refuses() {
        let acc = plane(vec![], 0);
        assert!(verdict(&acc, true, Some(12)).refusal.is_some());
    }

    #[test]
    fn a_plane_level_with_evidence_does_not_refuse() {
        let acc = plane(vec![rec(40, None)], 0);
        assert_eq!(verdict(&acc, false, Some(40)).refusal, None);
    }

    // --- the evidence adapter ------------------------------------------------------------

    #[test]
    fn a_sidecar_describing_this_medium_is_evidence() {
        let h = health_for("/nonexistent-567/cairn.medium", Some(40));
        assert_eq!(
            recorded_watermark_for(Some(&h), Path::new("/nonexistent-567/cairn.medium")),
            Some(40)
        );
    }

    /// The case only a node-global sidecar can create: a rotation drive at another path.
    /// That sidecar is about some other artifact — possibly another node's — never this one.
    #[test]
    fn a_sidecar_describing_another_medium_is_not_evidence() {
        let h = health_for("/nonexistent-567/drive-a.medium", Some(40));
        assert_eq!(
            recorded_watermark_for(Some(&h), Path::new("/nonexistent-567/drive-b.medium")),
            None
        );
    }

    #[test]
    fn no_sidecar_or_no_recorded_watermark_is_not_evidence() {
        let medium = Path::new("/nonexistent-567/cairn.medium");
        assert_eq!(recorded_watermark_for(None, medium), None);
        let v1_shaped = health_for("/nonexistent-567/cairn.medium", None);
        assert_eq!(recorded_watermark_for(Some(&v1_shaped), medium), None);
    }
}
```

(The adapter tests use paths that do not exist on purpose: `health_describes_medium` falls back to a literal comparison when the recorded medium is gone, a case `verify_backup_scope.rs` already pins.)

- [ ] **Step 3: Run the tests — expect FAIL**

Run: `cargo test -p cairn-node --lib backup::clinical_verdict`
Expected: compiles, and every `clinical_verdict` test FAILS with a `not yet implemented` panic.

- [ ] **Step 4: Implement**

Replace everything above `#[cfg(test)]` in `clinical_verdict.rs` with:

```rust
//! What `verify-backup` says about a medium's CLINICAL plane, and when it refuses (#567).
//!
//! # Why this exists
//!
//! Since #554 slice 2d, `restore` applies both planes. `verify-backup` — the cron health check
//! whose one job is *"can I still recover from this medium?"* — reported the federation plane
//! alone, so an operator could read green and rotate a drive whose clinical plane was empty.
//!
//! # What it decides
//!
//! One pure function, [`clinical_plane_verdict`], so the whole policy is testable with no
//! medium, no database and no CLI:
//!
//! - a **summary line** saying what the clinical half of a restore would bring back;
//! - an **advisory** when two DIFFERENT records share a `source_seq` (a re-capture that straddled
//!   a custody change). It never changes the exit code — as in `restore`;
//! - a **refusal**, `backup SHORT`, ONLY ON EVIDENCE: this node's own `backup-status.json`
//!   describes this medium and records a newer clinical watermark than the medium holds
//!   ([`shortfall`]). Maintainer decision, 2026-09-13.
//!
//! # What it deliberately does not do
//!
//! (Design: `docs/superpowers/specs/2026-09-13-verify-backup-clinical-plane-design.md`.)
//!
//! - **Warn about records past the last verified chain link.** `verify-backup` has already
//!   refused such a medium as UNSOUND before this runs; `cairn-medium`'s
//!   `a_medium_that_gates_records_out_is_never_sound` pins why that holds.
//! - **Report holes in the `source_seq` run.** Every duplicate apply burns an IDENTITY value, so
//!   holes are routine on a federating node and a gap warning would fire forever (#549).
//! - **Fail an empty plane without evidence.** From the bytes alone a fresh clinic's empty plane
//!   and a copy cut at a section boundary look identical, and only one of them is a bad backup.

use std::path::Path;

use cairn_medium::PlaneRecords;

use super::BackupHealth;

/// The facts `verify-backup` has gathered about one medium's clinical plane. Built by the
/// caller, which alone touches the filesystem; everything downstream of this is pure.
pub struct ClinicalPlaneFacts<'a> {
    /// The trusted records and the derivation's own arithmetic, from
    /// [`super::clinical_plane_accounting`] — never re-derived here.
    pub accounting: &'a PlaneRecords,
    /// A CAIRNB1/CAIRNB2 medium: its format predates the clinical plane entirely.
    pub legacy: bool,
    /// The clinical watermark this node's last backup recorded FOR THIS MEDIUM, or `None`
    /// whenever there is no such evidence. Build it with [`recorded_watermark_for`], which is
    /// what keeps a sidecar about some other drive from counting.
    pub recorded_for_this_medium: Option<i64>,
}

/// What to print, and whether to fail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClinicalPlaneVerdict {
    /// One line for stdout, always present: the clinical half of the command's claim.
    pub summary: String,
    /// A warning for stderr that does not change the exit code.
    pub advisory: Option<String>,
    /// When `Some`, the command must fail with exactly this message (it starts `backup SHORT:`).
    pub refusal: Option<String>,
}

/// PURE. Decide everything `verify-backup` says about the clinical plane. See the module docs.
pub fn clinical_plane_verdict(facts: &ClinicalPlaneFacts<'_>) -> ClinicalPlaneVerdict {
    let newest = newest_seq(facts.accounting);
    ClinicalPlaneVerdict {
        summary: summary_line(facts, newest),
        advisory: straddled_advisory(facts.accounting),
        refusal: shortfall(newest, facts.recorded_for_this_medium)
            .map(|recorded| short_refusal(recorded, newest)),
    }
}

/// PURE. `Some(recorded)` when the medium holds LESS than this node's last backup recorded for
/// it: nothing at all, or a newest seq below the recorded one. `None` when there is no evidence,
/// or the medium holds at least what was recorded.
///
/// Level is complete. AHEAD is also fine: a backup can write the medium durably and then fail to
/// write its sidecar, which leaves the medium holding more than the sidecar says.
pub fn shortfall(medium_newest: Option<i64>, recorded: Option<i64>) -> Option<i64> {
    let recorded = recorded?;
    match medium_newest {
        Some(held) if held >= recorded => None,
        _ => Some(recorded),
    }
}

/// The evidence rule's ONE impure step: the sidecar's clinical watermark, but only when that
/// sidecar describes the medium under test.
///
/// `backup-status.json` is node-global — one file beside the signing key, rewritten by every
/// backup to any path. A sidecar naming another path is a statement about another artifact,
/// possibly another node's (this command does not bind a medium to `--key`'s node), so it is not
/// evidence about this one. `health_describes_medium` canonicalizes paths, which is why this
/// is not pure and why it stays out of [`clinical_plane_verdict`].
pub fn recorded_watermark_for(health: Option<&BackupHealth>, medium: &Path) -> Option<i64> {
    health
        .filter(|h| super::health_describes_medium(&h.medium_path, medium))
        .and_then(|h| h.clinical_watermark)
}

/// The newest trusted clinical `source_seq`: the same number `cairn_medium::watermark` returns
/// over the same verified prefix, which is what `backup` recorded in the sidecar.
fn newest_seq(accounting: &PlaneRecords) -> Option<i64> {
    accounting.records.iter().map(|r| r.source_seq).max()
}

fn summary_line(facts: &ClinicalPlaneFacts<'_>, newest: Option<i64>) -> String {
    match newest {
        Some(seq) => {
            let collapsed = match facts.accounting.collapsed {
                0 => String::new(),
                k => format!(", {k} byte-identical re-capture(s) collapsed"),
            };
            format!(
                "clinical-plane records OK: {} verified, newest seq {seq}{collapsed}",
                facts.accounting.records.len()
            )
        }
        None if facts.legacy => "clinical plane: NONE — this CAIRNB1/CAIRNB2 medium predates \
                                 the clinical plane and carries no patient data at all. If \
                                 this node holds charts, they are NOT on this medium."
            .to_string(),
        None => "clinical plane: EMPTY — this medium would restore NO patient data. If this \
                 node holds charts, they are NOT on this medium."
            .to_string(),
    }
}

/// The straddled-duplicate finding, worded for a check that has applied nothing. `restore`'s
/// [`super::straddled_duplicate_notice`] says the copies "were applied", which would be false here.
fn straddled_advisory(accounting: &PlaneRecords) -> Option<String> {
    let repeated = super::straddled_positions(&accounting.records);
    if repeated.is_empty() {
        return None;
    }
    Some(format!(
        "WARNING: this medium holds two or more DIFFERENT records at the same source \
         position(s): {}. A byte-identical re-capture is collapsed silently and is expected; \
         these differ — typically a capture that straddled an unwrap-key rotation or a \
         crypto-shred, so the copies disagree about CUSTODY. A restore would apply all of them \
         (the apply door is idempotent and refuses custody for an already-shredded target, so \
         nothing erased can come back). This does not fail the check, but review these \
         positions: only you can tell which capture reflects what this node actually held.",
        super::describe_positions(&repeated)
    ))
}

fn short_refusal(recorded: i64, newest: Option<i64>) -> String {
    let held = match newest {
        Some(seq) => format!("clinical records only through seq {seq}"),
        None => "no clinical records at all".to_string(),
    };
    format!(
        "backup SHORT: this node's last backup to this medium recorded clinical events through \
         seq {recorded}, but the medium holds {held}. The file at this path is not what that \
         backup wrote — a truncated copy, or an older one put back in its place — and a restore \
         from it would bring back less than this node last captured. Remedy: run `backup --to` \
         this path again while this node still holds its events, or locate the complete copy. \
         (Rotating drives through one mount point? The drive that missed the latest backup \
         reads SHORT until its own next backup catches it up — and until then it really would \
         restore less.)"
    )
}
```

- [ ] **Step 5: Run the tests — expect PASS, then clippy**

Run: `cargo test -p cairn-node --lib backup::`
Expected: all `backup::tests` and `backup::clinical_verdict::tests` pass.

Run: `cargo clippy -p cairn-node --lib --tests -- -D warnings`
Expected: no warnings. (`rustfmt` may re-wrap the long string literals in `summary_line`; run `cargo fmt -p cairn-node` and re-run the tests if it does.)

- [ ] **Step 6: Mutations (each must turn a named test red; restore from the scratchpad copy after each)**

```bash
SCR=/private/tmp/claude-501/-Users-hherb-src-cairn-ehr/fb12adf1-0109-4a3e-ac33-c3824607041a/scratchpad
cp crates/cairn-node/src/backup/clinical_verdict.rs "$SCR/clinical_verdict.rs.orig"
```
1. In `shortfall`, `held >= recorded` → `held > recorded`. Expect red: `a_medium_level_with_the_evidence_is_not_short`, `a_plane_level_with_evidence_does_not_refuse`.
2. In `shortfall`, replace `_ => Some(recorded)` with `Some(_) => Some(recorded), None => None`. Expect red: `an_empty_medium_against_evidence_is_short`, `an_empty_plane_against_evidence_refuses_naming_both_sides`.
3. In `recorded_watermark_for`, delete the `.filter(...)` line. Expect red: `a_sidecar_describing_another_medium_is_not_evidence`.

After each: `cp "$SCR/clinical_verdict.rs.orig" crates/cairn-node/src/backup/clinical_verdict.rs`, then `cargo test -p cairn-node --lib backup::clinical_verdict` → green. Finish with `git status --short`.

- [ ] **Step 7: Commit**

```bash
git add crates/cairn-node/src/backup.rs crates/cairn-node/src/backup/clinical_verdict.rs
git commit -m "feat(#567): the pure clinical-plane verdict verify-backup will print

Summary line, straddled-duplicate advisory worded for a check that has
applied nothing, and a backup SHORT refusal only on evidence: the sidecar
describes this medium and records a newer clinical watermark than it holds.
straddled_positions/describe_positions split out of restore's notice,
whose text is unchanged. Three mutations run, all killed.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

### Task 3: Move the CLI clinic fixtures into a shared module

**Files:**
- Create: `crates/cairn-node/tests/common/clinic_kit.rs`
- Modify: `crates/cairn-node/tests/verify_backup_scope.rs` — remove the fixture block (from the comment banner `// Fixtures for the DB-gated tests below.` at line ~243 through the end of `fn clinical_records` at line ~473) and its now-unused imports.

**Interfaces:**
- Consumes: `crate::common::submit_registration` (each including test binary declares `mod common;`).
- Produces (for Task 4): `pub struct Clinic { pub _guard, pub db: Client, pub base: String, pub sk: SigningKey, pub kid: String, pub dir: tempfile::TempDir }` with `pub fn key(&self)`, `pub fn medium(&self)`, `pub fn cli(&self) -> std::process::Command`; `pub async fn establish_clinic() -> Option<Clinic>`; `pub fn write_existing_escrow(key, sk, op, code)`; `pub async fn author_sealed_clinical_event(c, sk, kid) -> Vec<u8>`; `pub fn as_v3`, `pub fn clinical_records`, `pub fn cs`, `pub fn sealed_assert_body`.

A pure move: behaviour must not change, and the existing suite is the test.

- [ ] **Step 1: Baseline — the existing suite is green before the move**

Run: `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test -p cairn-node --test verify_backup_scope`
Expected: `test result: ok. <N> passed; 0 failed`. Write `<N>` down: the move must reproduce exactly that count, and every DB-gated test in it must have run (no `skipped: set CAIRN_TEST_PG` lines in the output).

- [ ] **Step 2: Create `tests/common/clinic_kit.rs`**

Header, then the moved block verbatim, with `pub` added to the struct, its fields, its three methods, and every `fn`/`async fn`:

```rust
//! A solo clinic node for tests that drive the REAL `cairn-node` binary: a live database, the
//! node's signing identity written to a key FILE (which `--key` needs), and a temporary
//! directory standing in for the operator's backup volume.
//!
//! Moved out of `verify_backup_scope.rs` (#567) so a second suite can share it instead of
//! copying it — `tests/common/dead_node.rs` in `cairn-sync` set that convention: a fixture that
//! has already had two ways to make a test pass for the wrong reason should exist once.
//!
//! Include it with `#[path = "common/clinic_kit.rs"] mod clinic_kit;`. The including binary must
//! ALSO declare `mod common;` — `author_sealed_clinical_event` registers each chart through
//! `common::submit_registration` (#345: a chart's first event must be its registration).
#![allow(dead_code)] // each including suite uses a different subset

use crate::common;
use cairn_event::keys::Secret32;
use cairn_event::seal::{seal_event_payload, seal_stub_twin};
use cairn_event::{sign, EventBody, Hlc, SigningKey};
use cairn_medium::{MediumImage, MediumRecord, MediumV3, Plane};
use cairn_node::{db, identity, keystore};
use tokio_postgres::Client;
use uuid::Uuid;

// … the moved fixture block, `pub` added as described above …
```

Inside the moved block, the one call `common::submit_registration(...)` stays as written (it resolves through the `use crate::common;` above).

- [ ] **Step 3: Point `verify_backup_scope.rs` at it**

Directly under the existing `mod common;` line, add:

```rust
#[path = "common/clinic_kit.rs"]
mod clinic_kit;
use clinic_kit::{
    as_v3, author_sealed_clinical_event, clinical_records, establish_clinic, write_existing_escrow,
};
```

Delete the moved block. Then build: `cargo test -p cairn-node --test verify_backup_scope --no-run`. For each `unused import` error, delete exactly that import from the top of `verify_backup_scope.rs`; for each `cannot find` error, add that name to the `use clinic_kit::{…}` list. Add nothing else.

- [ ] **Step 4: Re-run — same count, green**

Run: `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test -p cairn-node --test verify_backup_scope`
Expected: the Step 1 count, all passed. Then the source guards that read `tests/`:
`cargo test -p cairn-node --test identity_scaffolding_shared --test db_gate_actually_ran --test crypto_sink_names_are_genuine`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/cairn-node/tests/common/clinic_kit.rs crates/cairn-node/tests/verify_backup_scope.rs
git commit -m "test(#567): the CLI clinic fixture moves to tests/common/clinic_kit.rs

A pure move so a second verify-backup suite shares it rather than copying
it. verify_backup_scope.rs drops ~230 lines; its suite passes unchanged.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

### Task 4: Wire the verdict into `verify-backup`, driven by failing CLI tests

**Files:**
- Create: `crates/cairn-node/tests/verify_backup_clinical_plane.rs`
- Modify: `crates/cairn-node/tests/verify_backup_scope.rs` (one assertion in `verify_backup_is_restorable_then_refuses_once_the_export_falls_behind`)
- Modify: `crates/cairn-node/src/main.rs` — `Cmd::VerifyBackup` arm (currently lines ~2643–2913)

**Interfaces:**
- Consumes: Task 2's `clinical_verdict::{clinical_plane_verdict, recorded_watermark_for, ClinicalPlaneFacts}`, `backup::{clinical_plane_accounting, read_health, health_path_for, health_describes_medium}`; Task 3's `clinic_kit`.
- Produces: the shipped behaviour.

- [ ] **Step 1: Write the new CLI suite (fails against today's binary)**

Create `crates/cairn-node/tests/verify_backup_clinical_plane.rs`:

```rust
//! `verify-backup` asks about the CLINICAL plane (#567), driven through the real binary.
//!
//! Before #567 this command printed one line about the federation plane and nothing about the
//! clinical plane a restore now applies. These tests pin the three things the design
//! (`docs/superpowers/specs/2026-09-13-verify-backup-clinical-plane-design.md`) decided:
//!
//! 1. an older copy put back at the path the nightly backup writes to fails `backup SHORT` —
//!    the node's own sidecar proves the path held more;
//! 2. a clinical chain break with every record signature intact still fails, and fails BEFORE
//!    any clinical all-clear is printed (the reason `untrusted_clinical_notice` is not wired);
//! 3. a sidecar about ANOTHER path is not evidence, even when it records clinical events.
//!
//! The pure policy is unit-tested in `src/backup/clinical_verdict.rs`; these prove the arm
//! gathers the right facts and acts on the verdict.

mod common;

#[path = "common/clinic_kit.rs"]
mod clinic_kit;

use cairn_medium::{assess, parse_any, serialize_v3, MediumImage, Plane};
use cairn_node::backup;
use clinic_kit::{author_sealed_clinical_event, establish_clinic, Clinic};

/// Run a `cairn-node` command that must succeed, panicking with its stderr if it does not.
fn run_ok(cmd: &mut std::process::Command, what: &str) -> std::process::Output {
    let out = cmd.output().unwrap();
    assert!(
        out.status.success(),
        "{what} must succeed; stderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    out
}

fn verify(cl: &Clinic, medium: &std::path::Path) -> std::process::Output {
    cl.cli()
        .args(["verify-backup", "--from"])
        .arg(medium)
        .output()
        .unwrap()
}

fn backup_to(cl: &Clinic, medium: &std::path::Path, what: &str) {
    run_ok(cl.cli().args(["backup", "--to"]).arg(medium), what);
}

/// The failure #567 exists for. Night 1 backs up a node with no charts; the file is copied
/// aside; a chart is written and night 2 captures it; then the night-1 copy is put back at the
/// same path — what a same-mount-point rotation, or a restored-from-an-old-copy drive, looks
/// like. Before #567 this verified green (the kit verdict saw an empty clinical plane as nothing
/// to cover) and a restore from it would have brought back no charts.
#[tokio::test]
async fn an_older_copy_at_the_backed_up_path_fails_as_short() {
    let Some(cl) = establish_clinic().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    backup_to(&cl, &cl.medium(), "night 1's backup (no charts yet)");
    let night_one = cl.dir.path().join("night-1.medium");
    std::fs::copy(cl.medium(), &night_one).unwrap();

    author_sealed_clinical_event(&cl.db, &cl.sk, &cl.kid).await;
    backup_to(&cl, &cl.medium(), "night 2's backup (one chart)");

    // Positive control: the evidence this test depends on really exists. Without it a green
    // run could mean "no sidecar was written", not "the rule works".
    let health = backup::read_health(&backup::health_path_for(&cl.key()))
        .expect("night 2 wrote backup-status.json");
    let recorded = health
        .clinical_watermark
        .expect("night 2's sidecar records the clinical capture");
    assert!(backup::health_describes_medium(&health.medium_path, &cl.medium()));

    std::fs::copy(&night_one, cl.medium()).unwrap();
    let v = verify(&cl, &cl.medium());
    let stdout = String::from_utf8_lossy(&v.stdout);
    let stderr = String::from_utf8_lossy(&v.stderr);
    assert!(
        !v.status.success(),
        "an older copy at the backed-up path must fail the health check; stdout:\n{stdout}"
    );
    assert!(stderr.contains("backup SHORT"), "named as SHORT: {stderr}");
    assert!(
        stderr.contains(&format!("through seq {recorded}")),
        "naming what the last backup recorded: {stderr}"
    );
    assert!(
        stdout.contains("clinical plane: EMPTY"),
        "and the plane line printed before the refusal says what the copy holds: {stdout}"
    );
}

/// A clinical segment whose chain link is broken while every record's signature still
/// verifies. `verify_backup_scope.rs`'s corrupt-clinical test flips a byte INSIDE a record,
/// which is the signature path; this is the chain path. It must fail as UNSOUND, and no
/// clinical all-clear may have printed before it.
#[tokio::test]
async fn a_broken_clinical_chain_link_fails_before_any_clinical_all_clear() {
    let Some(cl) = establish_clinic().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    author_sealed_clinical_event(&cl.db, &cl.sk, &cl.kid).await;
    backup_to(&cl, &cl.medium(), "the backup");

    let MediumImage::V3(mut m) = parse_any(&std::fs::read(cl.medium()).unwrap()).unwrap() else {
        panic!("backup writes CAIRNB3")
    };
    assert!(assess(&m).sound(), "positive control: sound before the break");
    let target = m
        .segments
        .iter()
        .position(|s| s.plane == Plane::Clinical)
        .expect("the backup captured a clinical segment");
    m.segments[target].prev_commitment = "deadbeef".into();
    std::fs::write(cl.medium(), serialize_v3(&m.segments).unwrap()).unwrap();

    // Prove the break is on the CHAIN path, not the signature path.
    let MediumImage::V3(broken) = parse_any(&std::fs::read(cl.medium()).unwrap()).unwrap() else {
        panic!("still CAIRNB3")
    };
    let h = assess(&broken);
    assert!(h.records.all_intact(), "every record signature still verifies");
    assert!(!h.chain.chain_intact(), "and the chain does not");

    let v = verify(&cl, &cl.medium());
    let stdout = String::from_utf8_lossy(&v.stdout);
    let stderr = String::from_utf8_lossy(&v.stderr);
    assert!(!v.status.success(), "a broken chain must fail; stdout:\n{stdout}");
    // UNSOUND, not SHORT. Both would be true of this file: the break gates the clinical records
    // out, so the trusted set is empty while the sidecar recorded a clinical watermark for this
    // path. Soundness is checked first because its remedy ("locate another copy") is the right
    // one for a damaged medium; "run backup again" would append to a broken chain.
    assert!(stderr.contains("backup UNSOUND"), "{stderr}");
    assert!(!stderr.contains("backup SHORT"), "{stderr}");
    assert!(
        !stdout.contains("clinical"),
        "no clinical-plane line may precede the refusal: {stdout}"
    );
}

/// A sidecar about ANOTHER path is not evidence, even when it records clinical events.
///
/// This pins a GREEN over a drive that lacks this node's charts, and it is deliberate: the
/// sidecar is node-global and `verify-backup` does not bind a medium to `--key`'s node, so a
/// sidecar about drive A may describe a different node than drive B. Closing it needs a kit
/// identity the sidecar can bind to (#551). A non-empty drive at another path already fails
/// COVERAGE-UNKNOWN; that asymmetry predates #567 (design §7).
#[tokio::test]
async fn a_sidecar_about_another_path_is_not_evidence_even_with_charts() {
    let Some(cl) = establish_clinic().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let drive_a = cl.dir.path().join("drive-a.medium");
    let drive_b = cl.dir.path().join("drive-b.medium");
    backup_to(&cl, &drive_b, "drive B's backup (no charts yet)");
    author_sealed_clinical_event(&cl.db, &cl.sk, &cl.kid).await;
    backup_to(&cl, &drive_a, "drive A's backup (one chart)");

    let health = backup::read_health(&backup::health_path_for(&cl.key())).expect("sidecar");
    assert!(
        health.clinical_watermark.is_some(),
        "positive control: the sidecar DOES record clinical events"
    );
    assert!(backup::health_describes_medium(&health.medium_path, &drive_a));
    assert!(!backup::health_describes_medium(&health.medium_path, &drive_b));

    let v = verify(&cl, &drive_b);
    let stdout = String::from_utf8_lossy(&v.stdout);
    let stderr = String::from_utf8_lossy(&v.stderr);
    assert!(
        v.status.success(),
        "a sidecar about drive A is not evidence about drive B; stderr:\n{stderr}"
    );
    assert!(!stderr.contains("backup SHORT"), "{stderr}");
    assert!(stdout.contains("clinical plane: EMPTY"), "{stdout}");
}
```

- [ ] **Step 2: Add the positive-path assertion to the existing restorable-kit test**

In `verify_backup_scope.rs`, `verify_backup_is_restorable_then_refuses_once_the_export_falls_behind`, directly after the `assert!(v1.status.success(), …)` block:

```rust
    // #567: the clinical half of the claim, cross-checked against the database's own count —
    // never a hardcoded number (a registration rides event_log beside the medication assert).
    let in_db: i64 = cl
        .db
        .query_one("SELECT count(*) FROM event_log", &[])
        .await
        .unwrap()
        .get(0);
    let stdout1 = String::from_utf8_lossy(&v1.stdout);
    assert!(
        stdout1.contains(&format!("clinical-plane records OK: {in_db} verified")),
        "a sound kit states what the clinical half of a restore would bring back: {stdout1}"
    );
```

- [ ] **Step 3: Run — expect FAIL on the new behaviour**

Run: `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test -p cairn-node --test verify_backup_clinical_plane --test verify_backup_scope`
Expected: `an_older_copy_at_the_backed_up_path_fails_as_short` FAILS (exits 0), `a_sidecar_about_another_path_is_not_evidence_even_with_charts` FAILS (no `clinical plane: EMPTY` line), `verify_backup_is_restorable_then_refuses_once_the_export_falls_behind` FAILS (no clinical line). `a_broken_clinical_chain_link_fails_before_any_clinical_all_clear` PASSES already (UNSOUND predates #567); it is here to kill Step 6's mutation.

- [ ] **Step 4: Wire the arm in `main.rs`**

(a) Replace the comment paragraph that begins `// ⚠️ THAT SCOPING IS NOW A KNOWN GAP, not a safety property.` (ends `Tracked as its own slice (#567).`) with:

```rust
            // THE CLINICAL PLANE IS ASKED ABOUT EXPLICITLY (#567). Since #554 slice 2d `restore`
            // applies both planes, so a green here that described only the federation plane
            // said nothing about the half a solo clinic depends on. After the federation line
            // below, `clinical_verdict` prints what the clinical half of a restore would bring
            // back, and fails `backup SHORT` ONLY ON EVIDENCE — this node's own sidecar
            // describes this medium and records a newer clinical watermark than it holds
            // (maintainer decision, 2026-09-13). Two things it deliberately does NOT do:
            //   - warn about records past the last verified chain link. Such a medium is not
            //     sound, so `refuse_unsound_medium` below has already failed it; `cairn-medium`'s
            //     `a_medium_that_gates_records_out_is_never_sound` pins why, and says to wire
            //     `untrusted_clinical_notice` here if that ever stops holding;
            //   - report holes in the `source_seq` run — burned IDENTITY values make them routine
            //     on a federating node (#549).
```

(b) Directly after the `println!("federation-plane events OK: {}/{} verified", …);` statement, insert:

```rust
            // #567 — the clinical plane. The sidecar is read ONCE, here, and shared with the
            // kit verdict further down. Checked BEFORE the export: over a short medium the
            // export looks AHEAD and `kit_verdict` would call the kit restorable, so the
            // shortfall is the more fundamental finding and its remedy comes first.
            let health_path = cairn_node::backup::health_path_for(&cli.key);
            let health = cairn_node::backup::read_health(&health_path);
            let clinical_plane = cairn_node::backup::clinical_plane_accounting(&image)?;
            let clinical = cairn_node::backup::clinical_verdict::clinical_plane_verdict(
                &cairn_node::backup::clinical_verdict::ClinicalPlaneFacts {
                    accounting: &clinical_plane,
                    legacy: matches!(image, cairn_node::medium::MediumImage::Legacy(_)),
                    recorded_for_this_medium:
                        cairn_node::backup::clinical_verdict::recorded_watermark_for(
                            health.as_ref(),
                            &from,
                        ),
                },
            );
            println!("{}", clinical.summary);
            if let Some(advisory) = &clinical.advisory {
                eprintln!("{advisory}");
            }
            if let Some(refusal) = clinical.refusal {
                anyhow::bail!("{refusal}");
            }
```

(c) Further down, delete the now-duplicated two lines
`let health_path = cairn_node::backup::health_path_for(&cli.key);` and
`let health = cairn_node::backup::read_health(&health_path);`
and in the comment above them change `That is the one place this command reads \`cli.key\`` to `That is the one place this command reads \`cli.key\` (read once, above, where the clinical-plane check shares it)`.

(d) In the `RESIDUAL, filed as #551` comment, change `a format decision for slice 2e, not this task.` to `a format decision tracked by #551.`

- [ ] **Step 5: Run — expect PASS, plus the neighbouring suites**

Run:
```bash
export CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test"
cargo test -p cairn-node --test verify_backup_clinical_plane --test verify_backup_scope \
  --test backup_health_v2 --test reads_both_medium_revisions --test backup_carries_both_planes
```
Expected: all pass. `cargo clippy -p cairn-node --all-targets -- -D warnings` → clean; `cargo fmt --check` → clean.

- [ ] **Step 6: Mutations in the arm (restore from the scratchpad copy after each)**

```bash
SCR=/private/tmp/claude-501/-Users-hherb-src-cairn-ehr/fb12adf1-0109-4a3e-ac33-c3824607041a/scratchpad
cp crates/cairn-node/src/main.rs "$SCR/main.rs.orig"
```
1. Delete the `if let Some(refusal) … bail!` block. Expect red: `an_older_copy_at_the_backed_up_path_fails_as_short`.
2. Replace `recorded_watermark_for(health.as_ref(), &from)` with `health.as_ref().and_then(|h| h.clinical_watermark)`. Expect red: `a_sidecar_about_another_path_is_not_evidence_even_with_charts`.
3. Move the whole inserted #567 block (from `let health_path` to the `bail!` block) to just BEFORE the `cairn_node::backup::refuse_unsound_medium(` call. Expect red: `a_broken_clinical_chain_link_fails_before_any_clinical_all_clear` — its stdout now carries `clinical plane: EMPTY` (the break gated every clinical record out) and its stderr names `backup SHORT` instead of `backup UNSOUND`.

After each: `cp "$SCR/main.rs.orig" crates/cairn-node/src/main.rs`; re-run the one suite; `git status --short`.

- [ ] **Step 7: Commit**

```bash
git add crates/cairn-node/src/main.rs crates/cairn-node/tests/verify_backup_clinical_plane.rs crates/cairn-node/tests/verify_backup_scope.rs
git commit -m "feat(#567): verify-backup asks the clinical-plane question

It prints what the clinical half of a restore would bring back and fails
backup SHORT when this node's own sidecar describes the medium and records
a newer clinical watermark than it holds. A sidecar about another path is
not evidence. Checked before the export, whose kit verdict reads a short
medium as covered. Three arm mutations run, all killed.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

### Task 5: Documentation the branch owes, and the stale "slice 2e" references

**Files:**
- Modify: `docs/spec/security.md` (the `"the event log survives"` bullet, ~line 199; and the `owed by DR slice 2e` clause, ~line 182)
- Modify: `crates/cairn-node/src/localstate.rs` (~line 211–216), `crates/cairn-node/src/localstate_read.rs` (~line 111–113), `crates/cairn-node/tests/medium_point_in_time.rs` (~line 40)
- Modify: `docs/superpowers/specs/2026-09-13-verify-backup-clinical-plane-design.md` (§4 table row and §5.2 bullet on the advisory)

**Interfaces:** none (prose and comments only).

- [ ] **Step 1: File the issue the shred-legibility sentence needs a home in**

Run `gh issue list --state all --search "verify-backup shred" --limit 10`. If no issue covers *"`verify-backup` cannot say that a medium predates a shred"*, create one:

```bash
gh issue create --title "verify-backup cannot say that a medium predates a crypto-shred" --body "$(cat <<'EOF'
`docs/spec/security.md` (the *Erasure survives DR* bullet) lists two Cairn obligations once a
medium is understood as a point in time: make the residue **legible**, and state the asymmetry
plainly. It records the first as *not built*, owed by "DR slice 2e" — a label retired when
ADR-0067 took that slice's ADR, so the obligation had no home.

**The gap:** a medium captured before a body was crypto-shredded still carries that body's
wrapped DEK (correctly — trap 7 in HANDOVER, pinned by
`medium_point_in_time.rs::a_medium_restores_the_state_at_capture_time`). Nothing tells the
operator holding that medium that it would restore a key the live node has since destroyed, so
a clinic cannot see which drives still have to be rotated out before an erasure is complete
across copies.

**Not a request to filter old segments** — that would rewrite signed history. It asks for an
operator-facing statement, most likely in `verify-backup`, beside the clinical-plane report
added by #567.
EOF
)"
```

Record the new issue number as `N` for Step 2.

- [ ] **Step 2: `security.md`**

(a) Replace `owed by DR slice 2e` with `tracked by [#N](https://github.com/cairn-ehr/cairn-ehr/issues/N)`.

(b) In the *"the event log survives"* bullet, after the sentence ending `…independent of a clinical segment's chain.`, add:

```markdown
>   Since [#567](https://github.com/cairn-ehr/cairn-ehr/issues/567) `verify-backup` also reports
>   the clinical plane a restore would apply, and fails a medium holding less than this node's own
>   last backup recorded for that path. It does not report holes in the `source_seq` run
>   ([#549](https://github.com/cairn-ehr/cairn-ehr/issues/549)), and a kit verified on another
>   machine carries no evidence it could compare against
>   ([#551](https://github.com/cairn-ehr/cairn-ehr/issues/551)).
```

(Keep the `>` blockquote prefix the surrounding bullet uses.)

- [ ] **Step 3: Retire the three stale code-comment references to "2e's ADR"**

First confirm what they should point at: `grep -n "decision 1" docs/spec/decisions/0067-a-restore-reads-the-clinical-plane.md` (the actor registry re-entering on the export container's AEAD alone) and `grep -rn "AEAD\|not verify-on-apply" crates/cairn-node/src/main.rs` (what `restore` prints about it). Then:

- `localstate.rs`: replace `That is acceptable ONLY because \`apply_local_state\` does not yet insert them (Task 11 is the write half; slice 2d is the apply half) — the caveat is recorded here so 2e's ADR is not the first place it is written down.` with `Since slice 2d \`apply_local_state\` DOES insert them. That is accepted deliberately in ADR-0067 decision 1, and \`restore\` says so to the operator — the precondition this comment used to rest on ("does not yet insert them") no longer holds, and the acceptance replaced it.`
- `localstate_read.rs`: replace `2e's ADR owes that caveat; do not let this comment be the only place it is written down.` with `ADR-0067 decision 1 records that caveat, and \`restore\` prints it to the operator.`
- `medium_point_in_time.rs`: replace `slice 2e's ADR owes the sentence in as many words.` with `\`docs/spec/security.md\` states it in as many words (the *Erasure survives DR* bullet); no ADR does.`

If a grep in this step shows ADR-0067 decision 1 or the restore output do NOT say what these replacements claim, stop and write the comment to say what is actually true instead.

Run: `grep -rn "slice 2e\|2e's ADR" crates docs/spec` → expect no matches.

- [ ] **Step 4: Erratum in this slice's own design doc**

In §4's table, change the straddled row's stderr cell from `` `straddled_duplicate_notice` (advisory, as in `restore`) `` to `a straddled-duplicate advisory worded for a check that has applied nothing (exit unaffected, as in \`restore\`)`. In §5.2, change `the straddled-duplicate notice, reused as it is` to `the straddled-duplicate finding — the position search is shared with \`restore\`, but the wording is not: \`restore\`'s notice says the copies "were applied", which would be false here`.

- [ ] **Step 5: Build the docs and run the guards that read comments**

Run: `uv run --with-requirements docs/requirements.txt -- mkdocs build > "$SCR/mkdocs-567.log" 2>&1; echo "exit=$?"` → `exit=0`. Then `grep -n "security.md" "$SCR/mkdocs-567.log"` → no warning that names `security.md` (compare against a build of `main` if unsure whether a warning is new).
Run: `cargo test -p cairn-node --test dr_clinical_guarantee_gap --test medium_point_in_time --no-run` then execute both with `CAIRN_TEST_PG` set → PASS.

- [ ] **Step 6: Comments on #549 and #551**

```bash
gh issue comment 549 --body "verify-backup now reports the clinical plane (#567, PR #588) and deliberately does NOT report holes in its source_seq run: until a known-burned set exists, a gap warning would fire on every federating node. When this issue lands, verify-backup is the natural first consumer of 'unfilled gaps minus known-burned'."
gh issue comment 551 --body "#567 (PR #588) narrows the same-mount-point false green without closing it: when the sidecar describes the --from path and records a newer clinical watermark than the medium holds, verify-backup now fails backup SHORT, so a drive that missed the latest backup no longer reads Restorable. Still open, and now pinned by verify_backup_clinical_plane.rs::a_sidecar_about_another_path_is_not_evidence_even_with_charts: an EMPTY drive at a DIFFERENT path stays green even when this node has charts, because the node-global sidecar may describe another node's medium. A kit identity the sidecar can bind to is what closes both."
```

- [ ] **Step 7: Commit**

```bash
git add docs/spec/security.md crates/cairn-node/src/localstate.rs crates/cairn-node/src/localstate_read.rs crates/cairn-node/tests/medium_point_in_time.rs docs/superpowers/specs/2026-09-13-verify-backup-clinical-plane-design.md
git commit -m "docs(#567): security.md says what verify-backup now claims; 'slice 2e' is retired

Three code comments and one spec clause still deferred to an ADR that
will never be written under that name. The shred-legibility obligation
gets an issue; the registry-AEAD caveat points at ADR-0067 decision 1;
the rotation sentence points at security.md, where it already lives.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

### Task 6: Gate, review, and the session record

**Files:**
- Modify: `docs/HANDOVER.md`, `docs/ROADMAP.md`

- [ ] **Step 1: Start the full local gate in the background**

Run (background): `scripts/run-db-gated-tests.sh > "$SCR/gate-567.log" 2>&1; echo "exit=$?" >> "$SCR/gate-567.log"`
This takes hours on macOS when many binaries relink; do Steps 2–4 while it runs. CI's `clippy + cargo test (cairn_pgx floor)` job is the other full gate.

- [ ] **Step 2: The non-DB gates CI runs**

```bash
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc -p cairn-node -p cairn-medium --no-deps
```
Expected: all clean. (An intra-doc link to a PRIVATE item fails `cargo doc`; `newest_seq`, `summary_line`, `straddled_advisory`, `short_refusal` must not be linked.)

- [ ] **Step 3: Code review of the branch diff**

Dispatch a review (superpowers:requesting-code-review) over `git diff main...HEAD`. Fix every finding in place, or file an issue for one that cannot be fixed here (house rule 5). Then review the fix diff alone once more.

- [ ] **Step 4: HANDOVER and ROADMAP**

- HANDOVER ⇒ NEXT: #567 is DONE (PR #588); the DR test debt is now the seven unwritten §7 design tests only; name what #567 did not close (#549 gaps, #551 off-node evidence and the empty-drive-at-another-path green, the new shred-legibility issue `N`). Trap 7: replace *"the rotation sentence is therefore UNWRITTEN in any decision record, and this trap is currently its only home"* with the truth — `security.md` states it; no ADR does. Add the session line (2026-09-13) and a short *Recent sessions* entry with what generalises: **read the command's existing refusals before adding a warning** (the chain-break warning #567 asked for was unreachable), and **reused operator text can be false in its new context** (`restore`'s notice said "were applied").
- ROADMAP: the #567 entry under the DR section; keep every open issue number.
- Keep both under ~500 lines where that does not lose an open issue number.

- [ ] **Step 5: Read the gate, then commit, push, ready the PR**

Read `$SCR/gate-567.log`: require `exit=0` and no `test result: FAILED`. (A killed binary exits 101 with no FAILED line; a red that names `local_node` fixture state is #583's shape — truncate `local_node` in `cairn_test` and re-run that binary before trusting it.)

```bash
git add docs/HANDOVER.md docs/ROADMAP.md
git commit -m "docs(#567): HANDOVER and ROADMAP record the slice

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
git push
gh pr ready 588
```
Update PR #588's body: what changed, the three scope findings, the exit-code decision, the residuals (#549, #551, issue N), how it was gated, and `Closes #567.`
