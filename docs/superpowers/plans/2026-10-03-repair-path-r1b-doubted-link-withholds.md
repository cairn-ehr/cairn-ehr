# Repair path R1b — a doubted set withholds every line not on the opened chart (#697 (b), #701) — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A chart set can hold a *doubted* link: an un-attested link that db/018 flagged, or that
trips the hard veto now. While it does, the combined medication list withholds from sign-off every
line not recorded only on the opened chart. It says why in the line's own words, and names a remedy
that is a judgement of the LINK, not separation of threads. db/054's doubted-link test reads the
stored `patient_link.attested`.

**Architecture:** `cairn-medication-view` gains a row-level `WrongChartReasons { outside_set,
doubted_link }` beside the kept `cross_patient` flag. The two are built by one rule, and every
existing reader stays fail-safe. `withheld_rows` returns `WithheldLine { group_id, reasons }`, and
there is one remedy constant per reason. `cairn-node`'s read moves its pure hazard rule into
`medication/hazard.rs`, now told the opened chart. The window's row flags, its withheld report and
the CLI's list and sign-off output word each reason separately. db/054 changes one function body
(#701); there is no generation bump.

**Tech Stack:** Rust (tokio-postgres, anyhow, serde), PostgreSQL ≥ 18 + `cairn_pgx`, the `cairn-gui`
workspace (Tauri 2, plain JS; no JS change in this slice).

**Spec:** `docs/superpowers/specs/2026-09-27-duplicate-repair-path-679-680-681-design.md` — section
**"R1b — a doubted set withholds every line not on the opened chart (#697 (b), #701; designed
2026-10-03)"**. Issue #697 (the maintainer's decision comment, option (b), 2026-09-27), issue #701,
ADR-0076 decisions 1 and 5. Filed while designing: #716 (the window cannot confirm a standing link).

## Global Constraints

- **AGPL-3.0**; no new dependency.
- **TDD.** Every behaviour change starts with a test that fails for the right reason.
- **Fail-safe direction.** Over-warn, never under-warn. A reader that checks only `cross_patient`
  must stay correct; a row with no recorded reason is worded as the pre-#697 (outside-the-set) case.
- **The never-linked golden** (`combined_read.rs::a_never_linked_chart_reads_exactly_as_before`)
  stays green. The new `wrong_chart` field is stripped from it like `source_charts`.
- **Wording lives in pure functions and shared constants**, never inline in `main.rs`'s arms or in JS.
- **No SQL object added.** db/054 changes one function body only. `SCHEMA_GENERATION` stays **55**.
- **House rule 6:** no literal key material; no binding named `salt`/`nonce`/`iv`.
- **Files under 500 lines where feasible.** The new code goes in new files (`hazard.rs`,
  `doubted_link_withholds.rs`). Files already over 500 lines (`read.rs`, `combined_read.rs`,
  `cairn-gui-tauri/src/chart_set.rs`) shrink or grow by ≤ 10 lines.
- **Subagents run foreground tests only.** DB-gated suites are run by the controller, with
  `--nocapture`, checking that no `skipped:` line appears.
- **DB env for a targeted DB run:**
  ```bash
  export CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=$USER dbname=cairn_test" \
         CAIRN_TEST_PG2="host=127.0.0.1 port=5532 user=$USER dbname=cairn_test2" \
         CAIRN_TEST_PG3="host=127.0.0.1 port=5532 user=$USER dbname=cairn_test3"
  ```
  (`scripts/pg-target.sh` prints the cluster.) Use `CARGO_TARGET_DIR=/tmp/cairn-r1b-target` when an
  IDE is open (trap 18).

## Review Focus

1. **A line on the opened chart AND another member, in a doubted set.** It is withheld with the
   *doubted* reason, not the outside one; its `outside_set` stays false. Pinned in Task 2 (pure).
2. **Opening the OTHER chart of a doubted pair.** The rule mirrors: that chart's own lines are
   signable, and the first chart's lines are withheld. Pinned in Task 2 (DB).
3. **A row whose flags disagree** (`cross_patient` true with no reason, or a reason with
   `cross_patient` false). It is still never signed, and a reasonless row is worded as reaching
   outside the set. Pinned in Task 1 (pure).
4. **A ceased line on the other member of a doubted set.** It is shown, and never reported as
   withheld (it needs no signature). Pinned in Task 1 (pure).
5. **Cease from A's window on a doubted-set line recorded only on X.** It writes nothing and names
   every thread it held back. Pinned in Task 4 (pure).
6. **A line carrying both reasons.** Both sentences are shown, each with its own remedy. Pinned in
   Task 4 (window) and Task 5 (CLI).

---

## File structure

| File | Change | Responsibility |
|---|---|---|
| `crates/cairn-medication-view/src/row.rs` | modify | `WrongChartReasons`; `MedicationRow::wrong_chart`, `is_wrong_chart_hazard()`, `withheld_because()` |
| `crates/cairn-medication-view/src/targeting.rs` | modify | `WithheldLine`; `withheld_rows` → `Vec<WithheldLine>`; `withheld_group_ids`; signability reads `is_wrong_chart_hazard()` |
| `crates/cairn-medication-view/src/chart.rs` | modify | `DOUBTED_LINK_INSTRUCTION` |
| `crates/cairn-medication-view/src/lib.rs`, `fixtures.rs` | modify | exports; literal sites |
| `crates/cairn-node/src/medication/hazard.rs` | **create** | the pure hazard rule `wrong_chart_reasons` (moved from `read.rs`) + its tests |
| `crates/cairn-node/src/medication/read.rs` | modify | passes the opened chart; builds reasons; drops the moved rule |
| `crates/cairn-node/src/medication/signoff.rs` | modify | `SignOffOutcome::withheld: Vec<WithheldLine>` |
| `crates/cairn-node/src/medication/list_text.rs` | modify | `row_hazard_lines`, `withheld_signoff_lines` (CLI wording, pure) |
| `crates/cairn-node/src/main.rs` | modify | the two CLI arms call `list_text` |
| `db/054_person_charts.sql` | modify | #701 — read `pl.attested` |
| `crates/cairn-node/tests/doubted_link_withholds.rs` | **create** | DB tests for the rule (two moved from `combined_read.rs`) and #701 |
| `crates/cairn-node/tests/combined_read.rs` | modify | strip `wrong_chart` from the golden; the two doubted tests move out |
| `crates/cairn-node/tests/medication_read.rs` | modify | `out.withheld` comparisons via `withheld_group_ids` |
| `cairn-gui/cairn-gui-tabs/cairn-gui-tab-medications/src/{row_view,view,test_rows}.rs` | modify | per-reason row flag and withheld report |
| `cairn-gui/cairn-gui-tauri/src/chart_set.rs`, `commands.rs` | modify | `cease_plan` reads `is_wrong_chart_hazard()`; sign-off report passes `WithheldLine`s |

(Before Task 3, confirm the db/054 file name with
`grep -ln cairn_chart_set_has_doubted_link db/*.sql`.)

---

### Task 1: the reasons on the row, `WithheldLine`, and the doubted-link remedy constant

The shared pure crate's types, threaded through every consumer so all three trees compile with
**unchanged behaviour and wording**. The read still sets `wrong_chart` to its default here; Task 2
fills it.

**Files:**
- Modify: `crates/cairn-medication-view/src/row.rs` (the struct, then a new `impl` block and tests)
- Modify: `crates/cairn-medication-view/src/targeting.rs` (`is_signable_line`, `withheld_rows`, tests)
- Modify: `crates/cairn-medication-view/src/chart.rs` (new const + test)
- Modify: `crates/cairn-medication-view/src/lib.rs`, `src/fixtures.rs`
- Modify: `crates/cairn-node/src/medication/read.rs:162` (row literal), `src/medication/signoff.rs` (`withheld` type, test literal at ~490)
- Modify: `crates/cairn-node/src/main.rs` (~5443–5465: `out.withheld` → ids), `tests/combined_read.rs` (`strip_new_fields`, three `withheld_rows` asserts), `tests/medication_read.rs` (~601, ~707)
- Modify: `cairn-gui/cairn-gui-tabs/cairn-gui-tab-medications/src/test_rows.rs`, `src/view.rs` (`withheld_report` signature), `cairn-gui/cairn-gui-tauri/src/commands.rs:243`

**Interfaces:**
- Produces:
  - `cairn_medication_view::WrongChartReasons { pub outside_set: bool, pub doubted_link: bool }`
    (`Debug, Clone, Copy, Default, PartialEq, Eq, Serialize`), with `fn any(&self) -> bool`.
  - `MedicationRow::wrong_chart: WrongChartReasons`, serialized as `wrong_chart`.
  - `MedicationRow::is_wrong_chart_hazard(&self) -> bool` and
    `MedicationRow::withheld_because(&self) -> Option<WrongChartReasons>`.
  - `cairn_medication_view::WithheldLine { pub group_id: Uuid, pub reasons: WrongChartReasons }`.
  - `withheld_rows(&[MedicationRow]) -> Vec<WithheldLine>`, sorted by `group_id`.
  - `withheld_group_ids(&[WithheldLine]) -> Vec<Uuid>`.
  - `cairn_medication_view::DOUBTED_LINK_INSTRUCTION: &str`.
  - `SignOffOutcome::withheld: Vec<WithheldLine>`.
  - `cairn_gui_tab_medications::view::withheld_report(&[WithheldLine], &BTreeMap<Uuid, Vec<Uuid>>) -> Option<String>`.

- [ ] **Step 1: Write the failing tests (pure crate).** In `row.rs`'s `mod tests` (its `row()` helper
  gains `wrong_chart: WrongChartReasons::default(),`):

```rust
    /// A row the read marked a hazard but gave no reason (an older builder, a test fixture) is
    /// still a hazard, and is worded as the pre-#697 case: reaching outside the set.
    #[test]
    fn a_hazard_with_no_recorded_reason_is_worded_as_reaching_outside() {
        let mut r = row("warfarin", None);
        r.cross_patient = true;
        assert!(r.is_wrong_chart_hazard());
        assert_eq!(
            r.withheld_because(),
            Some(WrongChartReasons { outside_set: true, doubted_link: false })
        );
    }

    /// The converse disagreement — a reason with the flag down — fails safe: it is a hazard.
    #[test]
    fn a_reason_without_the_flag_is_still_a_hazard() {
        let mut r = row("warfarin", None);
        r.wrong_chart.doubted_link = true;
        assert!(r.is_wrong_chart_hazard());
        assert_eq!(
            r.withheld_because(),
            Some(WrongChartReasons { outside_set: false, doubted_link: true })
        );
    }

    #[test]
    fn an_ordinary_row_is_not_withheld() {
        let r = row("warfarin", None);
        assert!(!r.is_wrong_chart_hazard());
        assert_eq!(r.withheld_because(), None);
    }
```

  In `targeting.rs`'s `mod tests` (its `row()` helper gains `wrong_chart:
  WrongChartReasons::default(),`; import `crate::row::WrongChartReasons`):

```rust
    /// #697 (b): a doubted-link reason alone withholds the line, and the report carries it.
    #[test]
    fn a_doubted_link_line_is_withheld_with_its_reason() {
        let mut rows = vec![row(1, MedicationStatus::Active, vec![member(1, VouchState::Absent)])];
        rows[0].cross_patient = true;
        rows[0].wrong_chart.doubted_link = true;
        assert!(sign_off_targets(&rows).is_empty());
        assert_eq!(
            withheld_rows(&rows),
            vec![WithheldLine {
                group_id: uid(1),
                reasons: WrongChartReasons { outside_set: false, doubted_link: true },
            }]
        );
        assert_eq!(withheld_group_ids(&withheld_rows(&rows)), vec![uid(1)]);
    }

    /// Review focus 4: a CEASED line on the other member of a doubted set is shown, never
    /// signed, and never reported as withheld — it needs no signature.
    #[test]
    fn a_ceased_doubted_link_line_is_not_reported_as_withheld() {
        let mut rows = vec![row(1, MedicationStatus::Ceased, vec![member(1, VouchState::Absent)])];
        rows[0].cross_patient = true;
        rows[0].wrong_chart.doubted_link = true;
        assert!(withheld_rows(&rows).is_empty());
    }
```

  In `chart.rs`'s `mod tests`:

```rust
    /// #697 part 1: a doubted link's remedy is a judgement of the LINK — both verbs, the window's
    /// gesture by its label — and never thread separation.
    #[test]
    fn the_doubted_link_remedy_names_both_judgements_and_never_separation() {
        assert!(DOUBTED_LINK_INSTRUCTION.contains("`unlink-charts "));
        // "link-charts" alone would be satisfied by "unlink-charts": pin the confirm verb by
        // its own backtick-opened spelling.
        assert!(DOUBTED_LINK_INSTRUCTION.contains("`link-charts "));
        assert!(DOUBTED_LINK_INSTRUCTION.contains("Not the same person"));
        assert!(DOUBTED_LINK_INSTRUCTION.contains("How these charts are linked"));
        assert!(!DOUBTED_LINK_INSTRUCTION.contains("medication-separate"));
    }
```

- [ ] **Step 2: Run them and see them fail to compile** (missing types and items):
  `cargo test -p cairn-medication-view`. Expected: E0412/E0425 for `WrongChartReasons`,
  `WithheldLine`, `withheld_group_ids` and `DOUBTED_LINK_INSTRUCTION`.

- [ ] **Step 3: Implement (pure crate).** In `row.rs`, above `MedicationRow`:

```rust
/// WHY a line is withheld from sign-off as a wrong-chart hazard (#697). Two independent facts,
/// either of which is enough, and both of which can hold at once:
///
/// - `outside_set`: the group reaches a chart OUTSIDE the set the list was read over, so some
///   thread on this line is recorded on another person's chart (issue #334). Remedy: separate
///   the threads (`SEPARATION_INSTRUCTION`).
/// - `doubted_link`: the set holds a link this node DOUBTS (an un-attested link its hard veto
///   flagged, or trips now — db/054), and this line is not recorded only on the opened chart.
///   A signature is a claim about a person, and the node has positive evidence the other
///   member may be someone else. Remedy: a human judges the LINK (`DOUBTED_LINK_INSTRUCTION`).
///
/// Built by `cairn-node`'s `medication::hazard::wrong_chart_reasons`, the one rule.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct WrongChartReasons {
    pub outside_set: bool,
    pub doubted_link: bool,
}

impl WrongChartReasons {
    /// Whether either reason holds.
    pub fn any(&self) -> bool {
        self.outside_set || self.doubted_link
    }
}
```

  In `MedicationRow`, directly after `cross_patient`:

```rust
    /// Why `cross_patient` is set (#697): the reasons a renderer words, each with its own
    /// remedy. Read through `withheld_because`, never directly: that is where a row whose flag
    /// and reasons disagree is resolved in the fail-safe direction.
    pub wrong_chart: WrongChartReasons,
```

  In `impl MedicationRow`:

```rust
    /// Whether this line must be withheld from sign-off as a wrong-chart hazard. Either signal
    /// is enough, so a builder that sets only one of the two fields still withholds the line
    /// (fail-safe; the read sets both from one rule).
    pub fn is_wrong_chart_hazard(&self) -> bool {
        self.cross_patient || self.wrong_chart.any()
    }

    /// The reasons to word for a withheld line, or `None` when it is not withheld. A hazard
    /// with no recorded reason (a builder that predates #697) is worded as reaching outside the
    /// set — the only meaning the flag had before.
    pub fn withheld_because(&self) -> Option<WrongChartReasons> {
        if !self.is_wrong_chart_hazard() {
            return None;
        }
        Some(if self.wrong_chart.any() {
            self.wrong_chart
        } else {
            WrongChartReasons {
                outside_set: true,
                doubted_link: false,
            }
        })
    }
```

  Rewrite `cross_patient`'s doc comment so it is no longer the rule's full text. It should say: "This
  line is withheld from sign-off as a wrong-chart hazard; `wrong_chart` says why (#697). Kept, rather
  than replaced by `wrong_chart`, so any reader that checks only this flag stays fail-safe." Keep the
  sentence naming `medication::hazard::wrong_chart_reasons` as the rule.

  In `targeting.rs`: `is_signable_line` becomes
  `row.status == MedicationStatus::Active && !row.is_wrong_chart_hazard()`. Add the doubted-link case
  to its doc. Then:

```rust
/// One displayed line that still needs a signature and will deliberately not get one, with
/// the reasons it is withheld — so every surface can word each reason with its own remedy
/// (#697), before the gesture and after it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WithheldLine {
    pub group_id: Uuid,
    pub reasons: WrongChartReasons,
}

pub fn withheld_rows(rows: &[MedicationRow]) -> Vec<WithheldLine> {
    let mut withheld: Vec<WithheldLine> = rows
        .iter()
        .filter(|row| row.status == MedicationStatus::Active)
        .filter(|row| row.members.iter().any(|m| m.vouch.needs_signature()))
        .filter_map(|row| {
            row.withheld_because().map(|reasons| WithheldLine {
                group_id: row.group_id,
                reasons,
            })
        })
        .collect();
    withheld.sort_by_key(|line| line.group_id);
    withheld.dedup_by_key(|line| line.group_id);
    withheld
}

/// The group ids of `lines`, in order — the argument `format_hazard_groups` takes.
pub fn withheld_group_ids(lines: &[WithheldLine]) -> Vec<Uuid> {
    lines.iter().map(|line| line.group_id).collect()
}
```

  (Keep `withheld_rows`'s existing doc comment. Add one sentence: "Each line carries its reasons, so a
  renderer never words a doubted-link line with the separation remedy.")

  In `chart.rs`, after `SEPARATION_INSTRUCTION`:

```rust
/// What to do about a line withheld because the record holds a DOUBTED link (#697 (b)) —
/// worded once, for every renderer, for the same reason as [`SEPARATION_INSTRUCTION`].
///
/// WHY NOT THE SEPARATION REMEDY. Both charts are members of the record on screen; the node
/// only doubts that they are one person. Separating threads is right only if they are two
/// people, and even then the LINK is what is wrong. A human judging the link resolves both
/// cases. An attested link outranks the machine's (ADR-0076 decision 5); an attested unlink
/// splits the record. The window cannot yet confirm a link that already stands (#716), so the
/// confirm half names the CLI verb.
pub const DOUBTED_LINK_INSTRUCTION: &str =
    "A clinician must judge the doubted link: it joins two charts without a clinician's \
     confirmation on record here, and the node's hard identity check found a clash between \
     them. If they are NOT the same person, unlink them — \"Not the same person…\" beside that \
     link under \"How these charts are linked\" in the window, or `unlink-charts <chart_a> \
     <chart_b>`. If they ARE the same person, confirm the link with `link-charts <chart_a> \
     <chart_b>` (the window cannot confirm a link that already stands yet). Either judgement \
     lifts this hold. Do not separate the threads: the doubt is about the link, not the drug.";
```

  `lib.rs`: export `DOUBTED_LINK_INSTRUCTION`, `WrongChartReasons`, `WithheldLine` and
  `withheld_group_ids`. In `fixtures.rs`, add `wrong_chart: WrongChartReasons::default(),` to the
  literal at line 44. The `cross_patient` sample line (line 98) also gets
  `cross_patient.wrong_chart.outside_set = true;`, so the mock reads as the read would build it.

- [ ] **Step 4: Thread the types through the consumers (no wording change).**
  - **`read.rs:162`:** add `wrong_chart: Default::default(),` with a `// filled by Task 2` comment.
    Remove that comment in Task 2.
  - **`signoff.rs`:** make the `withheld` field `Vec<cairn_medication_view::WithheldLine>`. Its doc
    says each line carries its reasons. Add `wrong_chart: Default::default(),` to the test literal.
  - **`main.rs`:** in the sign-off arm, bind
    `let withheld_ids = cairn_medication_view::withheld_group_ids(&out.withheld);` and pass
    `&withheld_ids` where `&out.withheld` went to `format_hazard_groups`. `out.withheld.len()` and
    `.is_empty()` are unchanged.
  - **`combined_read.rs`:** `strip_new_fields` also does `row.as_object_mut().unwrap().remove("wrong_chart");`.
    Extend its doc to say R1b added it. The three `withheld_rows(&list.rows)` comparisons become
    `withheld_group_ids(&withheld_rows(&list.rows))`; import it.
  - **`medication_read.rs`:** `out.withheld` → `withheld_group_ids(&out.withheld)` at both sites.
  - **GUI `test_rows.rs`:** add `wrong_chart: WrongChartReasons::default(),`.
  - **GUI `view.rs`:** `withheld_report(withheld: &[WithheldLine], …)` keeps today's single sentence
    for now, built over `withheld_group_ids(withheld)`. `build_view` passes `&withheld_rows(&list.rows)`.
  - **GUI `commands.rs:243`:** unchanged call shape (`&outcome.withheld` is now `&[WithheldLine]`).

- [ ] **Step 5: Run every gate this task touches.**
  - `cargo test -p cairn-medication-view` → PASS.
  - Root tree:
    ```bash
    CAIRN_ALLOW_DB_SKIP=1 cargo test -p cairn-node --lib
    cargo clippy --workspace --all-targets -- -D warnings
    ```
  - `cairn-gui` tree:
    ```bash
    cargo test --manifest-path cairn-gui/Cargo.toml --workspace --exclude cairn-gui-live
    cargo clippy --manifest-path cairn-gui/Cargo.toml --workspace --all-targets -- -D warnings
    ```
  All PASS.
  The controller runs, with the DB env, `combined_read` and `medication_read` (`--nocapture`, no
  `skipped:`). The golden must be green.

- [ ] **Step 6: Commit.**

```bash
git add -A crates/cairn-medication-view crates/cairn-node cairn-gui
git commit -m "feat(R1b): the reasons a line is withheld travel on the row and the report (Refs #697)"
```

---

### Task 2: the rule — every line not only on the opened chart, in a doubted set

**Files:**
- Create: `crates/cairn-node/src/medication/hazard.rs`
- Modify: `crates/cairn-node/src/medication/mod.rs` (add `mod hazard;`)
- Modify: `crates/cairn-node/src/medication/read.rs` (opened chart in; reasons out; delete `reaches_outside`, `is_wrong_chart_hazard` and their five tests)
- Create: `crates/cairn-node/tests/doubted_link_withholds.rs`
- Modify: `crates/cairn-node/tests/combined_read.rs` (move out `a_group_across_a_vetoed_link_is_still_withheld`, `a_group_across_a_link_the_veto_now_refuses_is_withheld` and `verified_dob`)

**Interfaces:**
- Consumes: `WrongChartReasons` (Task 1).
- Produces: `pub(crate) fn medication::hazard::wrong_chart_reasons(set: &ChartSet, opened: Uuid,
  set_has_doubted_link: bool, group_charts: &[Uuid]) -> WrongChartReasons`; read rows with
  `cross_patient == wrong_chart.any()`.

- [ ] **Step 1: Write the failing pure tests.** Create `hazard.rs`:

```rust
//! The wrong-chart hazard rule (#334, ADR-0076, #697 (b)), pure so it is tested without a
//! database. Moved out of `read.rs` when the rule grew a third input (the opened chart): the
//! read assembles the facts, and this decides what they mean.
//!
//! A medication line is withheld from sign-off when signing it could vouch for another
//! person's drug. Two independent reasons, each worded with its own remedy downstream
//! (`cairn_medication_view::WrongChartReasons`):
//!
//! 1. `outside_set` — the group reaches a chart OUTSIDE the set the list was read over.
//!    Linked charts are one person (ADR-0076 decision 1); a chart outside the set is not.
//! 2. `doubted_link` — the set holds a link this node DOUBTS (db/054
//!    `cairn_chart_set_has_doubted_link`: un-attested, and flagged by db/018 or tripping the
//!    hard veto now) and the line is not recorded ONLY on the opened chart. The maintainer's
//!    #697 option (b): a signature is a claim about a person, and a hard veto is positive
//!    evidence the other member may be someone else. The line stays visible — hiding it would
//!    be the hazard if the two charts ARE one person — and a human judging the link lifts it.
//!    The rule does not read the link graph to find WHICH pair is doubted: every line not on the
//!    opened chart is withheld. That over-warns in a set of three or more, the direction this
//!    module always errs in.
use cairn_medication_view::{ChartSet, WrongChartReasons};
use uuid::Uuid;

/// Whether a group touching `group_charts` reaches a chart outside `set`.
fn reaches_outside(set: &ChartSet, group_charts: &[Uuid]) -> bool {
    !set.contains_all(group_charts)
}

/// Whether every chart the group touches is the opened chart. An EMPTY list (a group re-keyed
/// mid-read, see `MedicationRow::source_charts`) is NOT "only on the opened chart": unknown
/// provenance must not read as safe.
fn only_on_opened(opened: Uuid, group_charts: &[Uuid]) -> bool {
    !group_charts.is_empty() && group_charts.iter().all(|c| *c == opened)
}

/// The reasons a group touching `group_charts` is a wrong-chart hazard, read over `set` from
/// the `opened` chart. `group_charts` is the union of the two sources the read has (see
/// `read.rs`); duplicates are harmless.
pub(crate) fn wrong_chart_reasons(
    set: &ChartSet,
    opened: Uuid,
    set_has_doubted_link: bool,
    group_charts: &[Uuid],
) -> WrongChartReasons {
    WrongChartReasons {
        outside_set: reaches_outside(set, group_charts),
        doubted_link: set_has_doubted_link && !only_on_opened(opened, group_charts),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    /// Charts 1 and 2 are linked; 1 is the one opened.
    fn set12() -> ChartSet {
        ChartSet::new([u(1), u(2)]).unwrap()
    }

    #[test]
    fn a_group_wholly_inside_an_undoubted_set_is_not_a_hazard() {
        assert!(!wrong_chart_reasons(&set12(), u(1), false, &[u(2), u(1)]).any());
        assert!(!wrong_chart_reasons(&set12(), u(1), false, &[u(2)]).any());
    }

    #[test]
    fn a_group_reaching_one_chart_outside_is_a_hazard() {
        let r = wrong_chart_reasons(&set12(), u(1), false, &[u(2), u(3)]);
        assert_eq!(r, WrongChartReasons { outside_set: true, doubted_link: false });
    }

    /// The opened chart's own line shows that chart's own dose and vouches only for the
    /// patient whose chart is open: signable, doubted link or not. Duplicates (the two sources
    /// overlap) must not read as two charts.
    #[test]
    fn in_a_doubted_set_a_line_only_on_the_opened_chart_is_signable() {
        assert!(!wrong_chart_reasons(&set12(), u(1), true, &[u(1)]).any());
        assert!(!wrong_chart_reasons(&set12(), u(1), true, &[u(1), u(1)]).any());
    }

    /// #697 (b) itself: the OTHER member's one-chart line is withheld — signing it would vouch
    /// for a possible stranger's medication under this clinician's name.
    #[test]
    fn in_a_doubted_set_a_line_only_on_another_member_is_withheld() {
        let r = wrong_chart_reasons(&set12(), u(1), true, &[u(2)]);
        assert_eq!(r, WrongChartReasons { outside_set: false, doubted_link: true });
    }

    /// Review focus 1: a line shared between the opened chart and the other member is withheld
    /// for the DOUBTED reason — it lies inside the set, so it is not the outside case.
    #[test]
    fn in_a_doubted_set_a_line_shared_with_the_opened_chart_is_withheld() {
        let r = wrong_chart_reasons(&set12(), u(1), true, &[u(1), u(2)]);
        assert_eq!(r, WrongChartReasons { outside_set: false, doubted_link: true });
    }

    #[test]
    fn in_a_doubted_set_a_line_with_no_known_chart_is_withheld() {
        assert!(wrong_chart_reasons(&set12(), u(1), true, &[]).doubted_link);
    }

    #[test]
    fn a_line_can_carry_both_reasons() {
        let r = wrong_chart_reasons(&set12(), u(1), true, &[u(2), u(3)]);
        assert_eq!(r, WrongChartReasons { outside_set: true, doubted_link: true });
    }
}
```

  Add `mod hazard;` to `medication/mod.rs`, beside `mod dose;`.

- [ ] **Step 2: Run the new tests.**
  `CAIRN_ALLOW_DB_SKIP=1 cargo test -p cairn-node --lib medication::hazard`.
  - Expected: PASS. The function and its tests arrive together.
  - The RED for this task is the DB test in Step 3, run against the OLD read.
  - Prove that this pure test bites: temporarily change `only_on_opened` to return
    `group_charts.len() <= 1` (the R1 rule's shape).
  - `in_a_doubted_set_a_line_only_on_another_member_is_withheld` must FAIL. Revert, and run
    `git status --short` to confirm only the intended files are modified.

- [ ] **Step 3: Write the failing DB tests.** Create `tests/doubted_link_withholds.rs`. Move the two
  doubted tests and `verified_dob` from `combined_read.rs` here **verbatim first**, then edit them as
  shown. Delete them from `combined_read.rs`, and drop any import that becomes unused there
  (`dob_assertion_body`/`render_dob_twin`/`submit_signed`/`EventSpec` only if nothing else uses
  them — check with `grep`).

```rust
//! #697 (b): while a chart set holds a DOUBTED link (db/054 `cairn_chart_set_has_doubted_link`:
//! an un-attested link db/018 flagged, or that trips the hard veto now), every medication line
//! not recorded only on the OPENED chart is withheld from sign-off, carrying the `doubted_link`
//! reason; the opened chart's own lines stay signable; a human judging the link lifts it. Also
//! #701: the doubted-link test reads the stored `patient_link.attested`.
//!
//! The first two tests moved here from `combined_read.rs` (R1), where the rule withheld only
//! MULTI-chart lines; the first one's one-chart assertion is the line #697 (b) reverses.
//!
//! DB-gated on $CAIRN_TEST_PG, serialized via `db::test_serial_guard` taken BEFORE connecting.
//! Key material is minted at runtime (house rule 6). The small builders below are file-local
//! copies of `combined_read.rs`'s (one line each; below the bar for `common/`).
mod common;
use cairn_event::demographics::{dob_assertion_body, render_dob_twin};
use cairn_event::SigningKey;
use cairn_medication_view::{
    sign_off_targets, withheld_group_ids, withheld_rows, MedicationRow, PatientMedicationList,
    WrongChartReasons,
};
use cairn_node::chart_link::{link_charts, Reviewer};
use cairn_node::db;
use cairn_node::medication::read::list_patient_medications;
use cairn_node::medication::{assert_medication, AssertMedicationInput};
use common::{
    cs, medication_setup as setup, submit_link_event, submit_registration, submit_signed, EventSpec,
};
use tokio_postgres::Client;
use uuid::Uuid;

const ORIGIN: &str = "r1b-test-node";
const DOUBTED: WrongChartReasons = WrongChartReasons { outside_set: false, doubted_link: true };

async fn chart(c: &Client, sk: &SigningKey, kid: &str) -> Uuid {
    let p = Uuid::now_v7();
    submit_registration(c, sk, kid, p, 0).await;
    p
}

async fn assert_one(c: &mut Client, sk: &SigningKey, kid: &str, patient: Uuid, term: &str) -> Uuid {
    assert_medication(
        c, sk, kid, "origin-a", patient,
        &AssertMedicationInput {
            term, coding: None, formulation: None, dose_amount: Some("500"),
            dose_unit: Some("mg"), sig: None, info_source: "patient",
            started: None, started_precision: None,
        },
        None, None,
    )
    .await
    .unwrap()
}

/// Fold two threads into one group (the peer-arrival shape; see `combined_read.rs`).
async fn group(c: &Client, first: Uuid, second: Uuid) {
    c.execute(
        "INSERT INTO medication_group_member (medication_id, group_id) VALUES \
         ($1::text::uuid, $1::text::uuid), ($2::text::uuid, $1::text::uuid)",
        &[&first.to_string(), &second.to_string()],
    )
    .await
    .unwrap();
}

/// Flag the standing a–b link as db/018 would for an un-attested link that trips the veto
/// (its lifecycle is pinned by `link_veto_floor.rs`; these tests are about the read).
async fn flag_link(c: &Client, a: Uuid, b: Uuid) {
    let (lo, hi) = (a.min(b), a.max(b));
    c.execute(
        "INSERT INTO link_veto_flag (low, high, content_address) \
         SELECT low, high, content_address FROM patient_link \
         WHERE low = $1::text::uuid AND high = $2::text::uuid",
        &[&lo.to_string(), &hi.to_string()],
    )
    .await
    .unwrap();
}

fn row_of(list: &PatientMedicationList, group: Uuid) -> &MedicationRow {
    list.rows.iter().find(|r| r.group_id == group).expect("the line is shown")
}

/// The read builds `cross_patient` and `wrong_chart` from one rule; pin that on every row.
fn assert_flag_agrees(list: &PatientMedicationList) {
    for r in &list.rows {
        assert_eq!(r.cross_patient, r.wrong_chart.any(), "row {}", r.group_id);
    }
}

fn sorted(mut v: Vec<Uuid>) -> Vec<Uuid> {
    v.sort();
    v
}
```

  Then the tests (keep `verified_dob` verbatim from `combined_read.rs` below them):

```rust
/// Moved from `combined_read.rs` and REVERSED in its one-chart half (#697 (b)). A–X linked
/// with a flagged link; a group shared by A and X, a line only on X, a line only on A.
#[tokio::test]
async fn a_doubted_set_withholds_every_line_not_on_the_opened_chart() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    c.batch_execute("TRUNCATE patient_link, person_member, link_veto_flag").await.unwrap();
    let (sk, kid, _hsk, _hkid) = setup(&c).await;
    let a = chart(&c, &sk, &kid).await;
    let x = chart(&c, &sk, &kid).await;
    let ta = assert_one(&mut c, &sk, &kid, a, "warfarin").await;
    let tx = assert_one(&mut c, &sk, &kid, x, "warfarin").await;
    let only_x = assert_one(&mut c, &sk, &kid, x, "amlodipine").await;
    let only_a = assert_one(&mut c, &sk, &kid, a, "metformin").await;
    group(&c, ta, tx).await;
    submit_link_event(&c, &sk, &kid, a, x, 10, true).await;

    // Positive control: the link is NOT doubted yet, so every line is one person's.
    let before = list_patient_medications(&c, a).await.unwrap();
    assert_flag_agrees(&before);
    assert!(before.rows.iter().all(|r| !r.cross_patient));

    flag_link(&c, a, x).await;
    let list = list_patient_medications(&c, a).await.unwrap();
    assert_flag_agrees(&list);
    assert!(list.charts.is_linked(), "the doubted link still combines the read");
    assert_eq!(row_of(&list, ta).wrong_chart, DOUBTED, "the shared line: doubted, not outside");
    assert_eq!(
        row_of(&list, only_x).wrong_chart,
        DOUBTED,
        "X's one-chart line is withheld: signing it would vouch for a possible stranger's drug"
    );
    assert!(!row_of(&list, only_a).cross_patient, "A's own line is A's own");
    assert_eq!(
        sign_off_targets(&list.rows),
        vec![only_a],
        "only the opened chart's own line is signed"
    );
    assert_eq!(
        withheld_group_ids(&withheld_rows(&list.rows)),
        sorted(vec![ta, only_x]),
        "both withheld lines are REPORTED, by group id"
    );
    assert!(withheld_rows(&list.rows).iter().all(|w| w.reasons == DOUBTED));
}

/// Review focus 2: opened from X, the rule mirrors — X's own line is signable, A's is not.
#[tokio::test]
async fn seen_from_the_other_chart_the_withholding_mirrors() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    c.batch_execute("TRUNCATE patient_link, person_member, link_veto_flag").await.unwrap();
    let (sk, kid, _hsk, _hkid) = setup(&c).await;
    let a = chart(&c, &sk, &kid).await;
    let x = chart(&c, &sk, &kid).await;
    let only_x = assert_one(&mut c, &sk, &kid, x, "amlodipine").await;
    let only_a = assert_one(&mut c, &sk, &kid, a, "metformin").await;
    submit_link_event(&c, &sk, &kid, a, x, 10, true).await;
    flag_link(&c, a, x).await;

    let list = list_patient_medications(&c, x).await.unwrap();
    assert_flag_agrees(&list);
    assert_eq!(row_of(&list, only_a).wrong_chart, DOUBTED);
    assert!(!row_of(&list, only_x).cross_patient);
    assert_eq!(sign_off_targets(&list.rows), vec![only_x]);
}

/// The withholding is lifted by a human: an attested link outranks the machine's (ADR-0076
/// D5) and db/018 clears the flag, so the set holds no doubted link and every line is signable.
#[tokio::test]
async fn a_human_link_lifts_the_withholding() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    c.batch_execute("TRUNCATE patient_link, person_member, link_veto_flag").await.unwrap();
    let (sk, kid, hsk, hkid) = setup(&c).await;
    let a = chart(&c, &sk, &kid).await;
    let x = chart(&c, &sk, &kid).await;
    let only_x = assert_one(&mut c, &sk, &kid, x, "amlodipine").await;
    let only_a = assert_one(&mut c, &sk, &kid, a, "metformin").await;
    submit_link_event(&c, &sk, &kid, a, x, 10, true).await;
    flag_link(&c, a, x).await;
    let held = list_patient_medications(&c, a).await.unwrap();
    assert_eq!(sign_off_targets(&held.rows), vec![only_a], "precondition: X's line is held");

    let who = Reviewer { human_sk: &hsk, human_kid: &hkid };
    link_charts(&mut c, a, x, &who, ORIGIN).await.unwrap();
    let list = list_patient_medications(&c, a).await.unwrap();
    assert_flag_agrees(&list);
    assert!(list.rows.iter().all(|r| !r.cross_patient), "nothing is withheld any more");
    assert_eq!(sign_off_targets(&list.rows), sorted(vec![only_a, only_x]));
}
```

  The moved `a_group_across_a_link_the_veto_now_refuses_is_withheld` (#220's path) gains a one-chart
  line on X. Add `let only_x = assert_one(&mut c, &sk, &kid, x, "amlodipine").await;` after `tx`, then
  at the end:

```rust
    assert_flag_agrees(&list);
    assert_eq!(row_of(&list, only_x).wrong_chart, DOUBTED, "#697 (b) on the read-time veto path too");
    assert!(sign_off_targets(&list.rows).is_empty());
```

  It replaces the old `list.rows[0].cross_patient` assert: with two rows, index 0 is no longer one
  known line. Use `row_of(&list, ta).cross_patient` instead.

- [ ] **Step 4: The controller runs the new suite against the OLD read and sees RED.**

```bash
cargo test -p cairn-node --test doubted_link_withholds -- --nocapture
```

  Expected: compile PASS, then `a_doubted_set_withholds_every_line_not_on_the_opened_chart`,
  `seen_from_the_other_chart_the_withholding_mirrors` and the #220 test FAIL on the
  `wrong_chart`/one-chart assertions (Task 1 left `wrong_chart` defaulted and the R1 rule in place).
  No `skipped:` line.

- [ ] **Step 5: Implement in `read.rs`.**
  - `list_patient_medications` calls `list_chart_set_medications(client, &charts, patient)`.
  - Its doc says the opened chart is passed because the hazard rule needs it (#697 (b)).
  - `list_chart_set_medications` gains `opened: Uuid`. Its doc's THE ASSEMBLY step 4 now names
    `medication::hazard::wrong_chart_reasons` and the opened chart.

  Replace the `cross_patient: HashSet<Uuid>` block with:

```rust
    // Each group's wrong-chart reasons (`medication::hazard`), over EVERY chart it touches.
    // Two sources name those charts: `source_charts` (statement-derived,
    // `medication_thread_group`) and `medication_group_cross_patient.patients`, which ALSO
    // sees a thread known only through an orphan cessation (db/033, PR #219 finding 3) — the
    // reason the latter is read at all. The rule runs over their union: either one naming a
    // chart is enough. Over-warn, never under-warn.
    let empty: Vec<Uuid> = Vec::new();
    let reasons: HashMap<Uuid, WrongChartReasons> = groups
        .iter()
        .copied()
        .map(|g| {
            let touched: Vec<Uuid> = group_charts
                .get(&g)
                .unwrap_or(&empty)
                .iter()
                .chain(reached.get(&g).unwrap_or(&empty))
                .copied()
                .collect();
            (g, wrong_chart_reasons(charts, opened, doubted, &touched))
        })
        .collect();
```

  In the row literal:

```rust
                // ONE rule sets both: `cross_patient` is kept for fail-safe readers, and
                // `wrong_chart` says why (#697).
                cross_patient: reasons.get(&group_id).is_some_and(WrongChartReasons::any),
                wrong_chart: reasons.get(&group_id).copied().unwrap_or_default(),
```

  The `hazardous` list chains
  `reasons.iter().filter(|(_, r)| r.any()).map(|(g, _)| *g)` in place of `cross_patient.iter().copied()`.
  Imports: `use super::hazard::wrong_chart_reasons;` and add `WrongChartReasons` to the
  `cairn_medication_view` import. Delete `reaches_outside`, `is_wrong_chart_hazard` and the five
  tests `a_group_wholly_inside_the_set_is_not_a_hazard`, `a_group_reaching_one_chart_outside_is_a_hazard`,
  `without_a_doubted_link_…`, `with_a_doubted_link_a_group_spanning_two_charts_…` and
  `with_a_doubted_link_a_one_chart_group_…` (Step 1's tests replace them). Update the `mod tests` doc
  comment. Fix every doc reference to `is_wrong_chart_hazard`; `grep -rn is_wrong_chart_hazard
  crates cairn-gui` must then list only `MedicationRow::is_wrong_chart_hazard` and its callers. Do
  the same for `MedicationRow::cross_patient`'s doc and `chart_set.rs`'s `contains_all` doc in the
  pure crate.

- [ ] **Step 6: GREEN.** The controller re-runs `doubted_link_withholds`, `combined_read` (golden
  included) and `medication_read` (`--nocapture`, no `skipped:`): PASS. Also run
  `CAIRN_ALLOW_DB_SKIP=1 cargo test -p cairn-node --lib` and clippy (root): PASS.

- [ ] **Step 7: Commit.**

```bash
git add crates/cairn-node crates/cairn-medication-view
git commit -m "feat(R1b): a doubted set withholds every line not on the opened chart (Refs #697)"
```

---

### Task 3: #701 — `cairn_chart_set_has_doubted_link` reads `patient_link.attested`

**Files:**
- Modify: the db/054 file (`grep -ln cairn_chart_set_has_doubted_link db/*.sql`), the function at ~line 109.
- Test: `crates/cairn-node/tests/doubted_link_withholds.rs` (append).

**Interfaces:** Consumes nothing new; produces the same function signature, `(uuid[]) → boolean`.

- [ ] **Step 1: Write the failing test.** Append:

```rust
/// #701: the doubted-link test reads the STORED `patient_link.attested` (R2a's one definition,
/// evaluated when the winner was applied), not a second spelling re-derived through an
/// `event_log` join. The UPDATE below simulates nothing real: it makes the two spellings
/// DISAGREE, so the answer shows which one the function reads.
#[tokio::test]
async fn the_doubted_link_check_reads_the_stored_attested_column() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    c.batch_execute("TRUNCATE patient_link, person_member, link_veto_flag").await.unwrap();
    let (sk, kid, _hsk, _hkid) = setup(&c).await;
    let a = chart(&c, &sk, &kid).await;
    let x = chart(&c, &sk, &kid).await;
    submit_link_event(&c, &sk, &kid, a, x, 10, true).await;
    verified_dob(&c, &sk, &kid, a, "1980-07-15", 20).await;
    verified_dob(&c, &sk, &kid, x, "1975-01-02", 21).await;
    let ids = vec![a.to_string(), x.to_string()];
    let doubted = |c: &Client| {
        let ids = ids.clone();
        async move {
            c.query_one("SELECT cairn_chart_set_has_doubted_link($1::text[]::uuid[])", &[&ids])
                .await
                .unwrap()
                .get::<_, bool>(0)
        }
    };
    assert!(doubted(&c).await, "precondition: un-attested and the veto trips now");

    let (lo, hi) = (a.min(x), a.max(x));
    c.execute(
        "UPDATE patient_link SET attested = TRUE WHERE low = $1::text::uuid AND high = $2::text::uuid",
        &[&lo.to_string(), &hi.to_string()],
    )
    .await
    .unwrap();
    assert!(
        !doubted(&c).await,
        "the function must read pl.attested — a re-derivation would still say doubted"
    );
}
```

  If the closure's borrow does not compile (see the note in `chart_link.rs`'s
  `a_human_link_resolves_a_doubted_machine_link`), use a nested `async fn doubted(c: &Client,
  ids: &[String]) -> bool` instead.

- [ ] **Step 2: The controller runs it and sees RED.**
  `cargo test -p cairn-node --test doubted_link_withholds the_doubted_link_check -- --nocapture`.
  Expected: FAIL on the second assert. No `skipped:` line.

- [ ] **Step 3: Implement.** Replace the second `EXISTS` and extend the function's header comment:

```sql
    ) OR EXISTS (
        -- #701: the STORED winner attestation (R2a, ADR-0076 decision 5 — one definition,
        -- evaluated when the winner was applied). Never re-derive it through event_log: that
        -- is a second spelling, and the join dropped a legacy row whose content_address is
        -- NULL (pre-#115).
        SELECT 1
        FROM patient_link pl
        WHERE pl.state = 'link'
          AND pl.low = ANY(p_charts) AND pl.high = ANY(p_charts)
          AND NOT pl.attested
          AND cairn_has_hard_veto(pl.low, pl.high)
    )
```

  `SECURITY DEFINER`, `search_path`, the grants and the signature are unchanged. SQL is
  `include_str!`'d, so rebuild before re-running (memory: a db/*.sql mutation needs a REBUILD).

- [ ] **Step 4: GREEN.** The controller re-runs `doubted_link_withholds`, `chart_link`
  (`a_human_link_resolves_a_doubted_machine_link`) and `combined_read`: PASS, no `skipped:`. Then run
  `scripts/run-db-sql-tests.sh` (SQL mirrors): PASS.

- [ ] **Step 5: Commit.** Use `fix(#701):`; the parenthesis keeps the closing-keyword guard quiet.

```bash
git add db crates/cairn-node/tests/doubted_link_withholds.rs
git commit -m "fix(#701): the doubted-link check reads patient_link.attested, not a second spelling"
```

---

### Task 4: the window's wording — per-reason row flag, per-reason withheld report, cease

**Files:**
- Modify: `cairn-gui/cairn-gui-tabs/cairn-gui-tab-medications/src/row_view.rs` (`flags`, + test)
- Modify: `cairn-gui/cairn-gui-tabs/cairn-gui-tab-medications/src/view.rs` (`withheld_report`, + tests)
- Modify: `cairn-gui/cairn-gui-tauri/src/chart_set.rs` (`cease_plan`, + one test)

**Interfaces:**
- Consumes: `MedicationRow::withheld_because`, `is_wrong_chart_hazard`, `WithheldLine`,
  `DOUBTED_LINK_INSTRUCTION` (Task 1).
- Produces: `withheld_report`'s signature is unchanged from Task 1; it now words per reason.

- [ ] **Step 1: Write the failing tests.** In `row_view.rs` tests:

```rust
    /// #697 (b): a doubted-link line says WHY in its own words — not "shared with another
    /// patient's record", which names the wrong cause.
    #[test]
    fn a_doubted_link_line_names_the_doubted_link_not_another_patient() {
        let mut r = row(1, MedicationStatus::Active, vec![member(1, VouchState::Absent)]);
        r.cross_patient = true;
        r.wrong_chart.doubted_link = true;
        let flags = &build_view(&chart(vec![r])).rows[0].flags;
        assert!(flags.iter().any(|f| f.contains("doubts")), "{flags:?}");
        assert!(!flags.iter().any(|f| f.contains("another patient")), "{flags:?}");
    }

    #[test]
    fn a_line_with_both_reasons_says_both() {
        let mut r = row(1, MedicationStatus::Active, vec![member(1, VouchState::Absent)]);
        r.cross_patient = true;
        r.wrong_chart = cairn_medication_view::WrongChartReasons { outside_set: true, doubted_link: true };
        let flags = &build_view(&chart(vec![r])).rows[0].flags;
        assert!(flags.iter().any(|f| f.contains("another patient")), "{flags:?}");
        assert!(flags.iter().any(|f| f.contains("doubts")), "{flags:?}");
    }
```

  In `view.rs` tests:

```rust
    /// #697 part 1: a doubted-link line's report names the LINK judgement, never separation.
    #[test]
    fn a_doubted_link_report_names_the_link_judgement_not_separation() {
        let mut hazard = row(1, MedicationStatus::Active, vec![member(1, VouchState::Absent)]);
        hazard.cross_patient = true;
        hazard.wrong_chart.doubted_link = true;
        let message = build_view(&chart(vec![hazard])).withheld_message.expect("reported");
        assert!(message.contains("unlink-charts"), "{message}");
        assert!(message.contains(&uid(1).to_string()), "the line is named: {message}");
        assert!(!message.contains("medication-separate"), "{message}");
    }

    #[test]
    fn a_line_with_both_reasons_is_reported_under_both_remedies() {
        let mut hazard = row(1, MedicationStatus::Active, vec![member(1, VouchState::Absent)]);
        hazard.cross_patient = true;
        hazard.wrong_chart = cairn_medication_view::WrongChartReasons { outside_set: true, doubted_link: true };
        let message = build_view(&chart(vec![hazard])).withheld_message.expect("reported");
        assert!(message.contains("medication-separate"), "{message}");
        assert!(message.contains("unlink-charts"), "{message}");
    }
```

  In `chart_set.rs` tests (review focus 5). `plan_row(true, …)` already sets `cross_patient`, which
  is how the read marks a doubted-set line:

```rust
    /// #697 (b): in a doubted set a line recorded only on ANOTHER member is withheld; ceasing
    /// it from this chart writes nothing and names every thread it held back.
    #[test]
    fn a_hazard_line_only_on_another_member_ceases_nothing_from_here() {
        let plan = cease_plan(&plan_row(true, &[(11, 2)]), Uuid::from_u128(1));
        assert!(plan.write.is_empty());
        assert_eq!(plan.held_back.len(), 1);
        assert!(plan.held_back[0].contains(&Uuid::from_u128(11).to_string()));
    }
```

  This cease test passes on today's code. It pins the claim of the design's "Cease: no change"
  bullet; it does not drive new code.

- [ ] **Step 2: Run and see the wording tests fail.**
  `cargo test --manifest-path cairn-gui/Cargo.toml -p cairn-gui-tab-medications`. Expected:
  `a_doubted_link_line_names_…`, `a_line_with_both_reasons_says_both`, `a_doubted_link_report_…` and
  `a_line_with_both_reasons_is_reported_…` FAIL. The pre-existing cross-patient tests still pass,
  because a reasonless hazard is worded as outside.

- [ ] **Step 3: Implement.** `row_view.rs` `flags`: replace the `if row.cross_patient { … }` block with

```rust
    if let Some(why) = row.withheld_because() {
        // Per-row, and in the row's own words, because this is where the clinician is
        // looking when they wonder why the line has no signature badge. One sentence per
        // REASON (#697): each names its own cause, and the report below names each remedy.
        if why.outside_set {
            // The displayed dose comes from a whole-group pick that ignores patient, so it
            // may be the other patient's (issue #334).
            out.push(
                "shared with another patient's record — the dose shown may not be this \
                 patient's, so this line cannot be signed"
                    .to_string(),
            );
        }
        if why.doubted_link {
            // #697 (b): the record holds a link the node doubts, and this line is not on the
            // opened chart alone — signing it could vouch for a possible stranger's drug.
            out.push(
                "this record holds a link this node doubts, and this line is not recorded only \
                 on the chart you opened — it may be another person's, so it cannot be signed \
                 from here"
                    .to_string(),
            );
        }
    }
```

  `view.rs` `withheld_report`: split by reason. Doc: "One sentence per reason, each with its own
  remedy (#697). A line with both reasons is named in both."

```rust
pub fn withheld_report(
    withheld: &[WithheldLine],
    separation_targets: &BTreeMap<Uuid, Vec<Uuid>>,
) -> Option<String> {
    let groups = |pick: fn(&WithheldLine) -> bool| -> Vec<Uuid> {
        withheld.iter().filter(|l| pick(l)).map(|l| l.group_id).collect()
    };
    let outside = groups(|l| l.reasons.outside_set);
    let doubted = groups(|l| l.reasons.doubted_link);
    let mut parts = Vec::new();
    if !outside.is_empty() {
        parts.push(format!(
            "{} line(s) on this chart still need a signature but will NOT be signed: {}. {}",
            outside.len(),
            format_hazard_groups(&outside, separation_targets),
            SEPARATION_INSTRUCTION
        ));
    }
    if !doubted.is_empty() {
        parts.push(format!(
            "{} line(s) on this record still need a signature but will NOT be signed from this \
             chart — the record holds a link this node doubts, and they are not recorded only \
             on this chart: {}. {}",
            doubted.len(),
            format_hazard_groups(&doubted, separation_targets),
            DOUBTED_LINK_INSTRUCTION
        ));
    }
    (!parts.is_empty()).then(|| parts.join(" "))
}
```

  (Import `DOUBTED_LINK_INSTRUCTION` and `WithheldLine`. `withheld_group_ids` is no longer needed
  here; remove the import if it is unused.)

  `chart_set.rs` `cease_plan`: `if !row.is_wrong_chart_hazard() || m.patient_id == opened {`. Add a
  doc sentence: "A doubted-set line on another member only (#697 (b)) is held back entirely."

- [ ] **Step 4: GREEN.** Run the `cairn-gui` tree's tests and clippy:

```bash
cargo test --manifest-path cairn-gui/Cargo.toml --workspace --exclude cairn-gui-live
cargo clippy --manifest-path cairn-gui/Cargo.toml --workspace --all-targets -- -D warnings
```

  Both must PASS. Also run `cargo doc --manifest-path cairn-gui/Cargo.toml --no-deps` with
  `RUSTDOCFLAGS=-D warnings`; it must PASS. Check that no public doc links a private item.

- [ ] **Step 5: Commit.**

```bash
git add cairn-gui
git commit -m "feat(R1b): the window words a doubted-link line and its remedy separately (Refs #697)"
```

---

### Task 5: the CLI's wording — `list_text::row_hazard_lines` and `withheld_signoff_lines`

**Files:**
- Modify: `crates/cairn-node/src/medication/list_text.rs` (two pure functions + tests)
- Modify: `crates/cairn-node/src/main.rs` (the `MedicationList` arm's `if row.cross_patient` block; the `MedicationSignOff` arm's `if !out.withheld.is_empty()` block)

**Interfaces:**
- Consumes: `WrongChartReasons`, `WithheldLine`, `format_hazard_groups`, both instructions.
- Produces:
  - `pub fn row_hazard_lines(why: WrongChartReasons, group: Uuid, targets: &BTreeMap<Uuid, Vec<Uuid>>) -> Vec<String>`
  - `pub fn withheld_signoff_lines(withheld: &[WithheldLine], targets: &BTreeMap<Uuid, Vec<Uuid>>) -> Vec<String>`

- [ ] **Step 1: Write the failing tests** in `list_text.rs`'s `mod tests`:

```rust
    fn both() -> WrongChartReasons {
        WrongChartReasons { outside_set: true, doubted_link: true }
    }

    #[test]
    fn a_doubted_link_row_names_the_link_judgement_not_separation() {
        let why = WrongChartReasons { outside_set: false, doubted_link: true };
        let text = row_hazard_lines(why, Uuid::from_u128(1), &BTreeMap::new()).join("\n");
        assert!(text.contains("doubts"), "{text}");
        assert!(text.contains("unlink-charts"), "{text}");
        assert!(!text.contains("medication-separate"), "{text}");
        assert!(!text.contains("more than one patient"), "{text}");
    }

    #[test]
    fn a_row_with_both_reasons_prints_both_and_its_threads_once() {
        let targets = BTreeMap::from([(Uuid::from_u128(1), vec![Uuid::from_u128(1), Uuid::from_u128(2)])]);
        let lines = row_hazard_lines(both(), Uuid::from_u128(1), &targets);
        let text = lines.join("\n");
        assert!(text.contains("medication-separate") && text.contains("unlink-charts"), "{text}");
        assert_eq!(text.matches(&Uuid::from_u128(2).to_string()).count(), 1, "{text}");
    }

    #[test]
    fn a_signoff_report_words_each_reason_with_its_own_remedy() {
        let w = |n, outside_set, doubted_link| WithheldLine {
            group_id: Uuid::from_u128(n),
            reasons: WrongChartReasons { outside_set, doubted_link },
        };
        let text = withheld_signoff_lines(&[w(1, true, false), w(2, false, true)], &BTreeMap::new())
            .join("\n");
        assert!(text.contains("1 medication line(s) still need a signature but were NOT signed: their"), "{text}");
        assert!(text.contains("medication-separate") && text.contains("unlink-charts"), "{text}");
        assert!(text.contains("Then sign off again."), "{text}");
        assert!(withheld_signoff_lines(&[], &BTreeMap::new()).is_empty());
    }
```

- [ ] **Step 2: Run and see them fail to compile.**
  `CAIRN_ALLOW_DB_SKIP=1 cargo test -p cairn-node --lib medication::list_text` (missing functions).

- [ ] **Step 3: Implement** in `list_text.rs`. Extend the module doc by one paragraph: the withheld-line
  wording moved here from `main.rs` for the same reason, when #697 split it by reason. Then:

```rust
/// The CLI's warning lines under one withheld row of `medication-list` — one per reason, each
/// with its own remedy (#697), then the group's member threads once (the remedy's arguments;
/// this row lists only this record's half of a cross-patient group, #338 finding 1).
pub fn row_hazard_lines(
    why: WrongChartReasons,
    group: Uuid,
    targets: &BTreeMap<Uuid, Vec<Uuid>>,
) -> Vec<String> {
    let mut out = Vec::new();
    if why.outside_set {
        out.push(format!(
            "    ! this group's member threads span more than one patient — the dose shown may \
             belong to the other patient, so this line CANNOT be signed off (issue #334). {}",
            SEPARATION_INSTRUCTION
        ));
    }
    if why.doubted_link {
        out.push(format!(
            "    ! this record holds a link this node doubts, and this line is not recorded only \
             on the chart you opened — it may be another person's, so it CANNOT be signed off \
             from here (issue #697). {}",
            DOUBTED_LINK_INSTRUCTION
        ));
    }
    if why.any() {
        out.push(format!("      {}", format_hazard_groups(&[group], targets)));
    }
    out
}

/// The sign-off outcome's report of lines withheld from the gesture — printed in EVERY
/// outcome, never folded into the success line: "signed off 11" over a twelfth outstanding
/// line reads as a finished chart. One block per reason, each with its own remedy (#697).
pub fn withheld_signoff_lines(
    withheld: &[WithheldLine],
    targets: &BTreeMap<Uuid, Vec<Uuid>>,
) -> Vec<String> {
    let pick = |f: fn(&WithheldLine) -> bool| -> Vec<Uuid> {
        withheld.iter().filter(|l| f(l)).map(|l| l.group_id).collect()
    };
    let mut out = Vec::new();
    let outside = pick(|l| l.reasons.outside_set);
    if !outside.is_empty() {
        out.push(format!(
            "! {} medication line(s) still need a signature but were NOT signed: their group's \
             member threads span more than one patient, so the dose displayed may belong to the \
             other patient (issue #334). {} Then sign off again.",
            outside.len(),
            SEPARATION_INSTRUCTION
        ));
        out.push(format!("    {}", format_hazard_groups(&outside, targets)));
    }
    let doubted = pick(|l| l.reasons.doubted_link);
    if !doubted.is_empty() {
        out.push(format!(
            "! {} medication line(s) still need a signature but were NOT signed from this chart: \
             the record holds a link this node doubts, and these lines are not recorded only on \
             the chart you opened (issue #697). {} Then sign off again.",
            doubted.len(),
            DOUBTED_LINK_INSTRUCTION
        ));
        out.push(format!("    {}", format_hazard_groups(&doubted, targets)));
    }
    out
}
```

  Imports: `use cairn_medication_view::{format_hazard_groups, WithheldLine, WrongChartReasons,
  DOUBTED_LINK_INSTRUCTION, SEPARATION_INSTRUCTION}; use std::collections::BTreeMap;`.

  `main.rs`, `MedicationList` arm: replace the whole `if row.cross_patient { … }` block, its comment
  included, with

```rust
                    // One warning per reason the line is withheld, with its own remedy and the
                    // group's member threads — worded in `list_text` (#697).
                    if let Some(why) = row.withheld_because() {
                        for line in cairn_node::medication::list_text::row_hazard_lines(
                            why,
                            row.group_id,
                            &list.separation_targets,
                        ) {
                            println!("{line}");
                        }
                    }
```

  `MedicationSignOff` arm: replace the `if !out.withheld.is_empty() { … }` block with a loop over
  `withheld_signoff_lines(&out.withheld, &out.separation_targets)`. Keep its "printed in EVERY
  outcome" comment above the loop. Remove the `withheld_ids` binding Task 1 added if nothing else
  uses it.

- [ ] **Step 4: GREEN.**
  - `CAIRN_ALLOW_DB_SKIP=1 cargo test -p cairn-node --lib` → PASS.
  - Root clippy → PASS.
  - `cargo build -p cairn-node` → PASS.
  - Controller: run `cargo run -p cairn-node -- medication-list <a>` against a doubted pair set up
    by `doubted_link_withholds`'s first test's shape (or any CLI-output test in `tests/` that
    already covers `medication-list`). Check that the new lines appear.

- [ ] **Step 5: Commit.**

```bash
git add crates/cairn-node
git commit -m "feat(R1b): the CLI words a doubted-link line and its remedy separately (Refs #697)"
```

---

### Task 6: docs, the as-built note, HANDOVER/ROADMAP, full gates, PR

**Files:**
- Modify: the design page (an as-built note under R1b: every deviation from the plan, or "none").
- Modify: `docs/HANDOVER.md` (⇒ NEXT; a **R1b durable rules** block: the reasons are built by one
  rule; `cross_patient` is kept fail-safe; the doubted remedy is never separation; db/054 reads
  `pl.attested` — each with its pinning test), `docs/ROADMAP.md` (an R1b entry; #697/#701 status),
  both pruned toward 500 lines without dropping an open issue number.
- Verify: `crates/cairn-node/tests/paper_parity_plan_section.rs` passes for this plan file.

- [ ] **Step 1:** Write the as-built note and the docs.
- [ ] **Step 2:** Gates, in CI's order, AFTER the last edit (trap 18):
  - `cargo fmt --all --check`, and the same with `--manifest-path cairn-gui/Cargo.toml`;
  - clippy on both trees;
  - `RUSTDOCFLAGS=-D warnings cargo doc --workspace --no-deps`, and on the `cairn-gui` tree;
  - `cargo deny check`;
  - `scripts/run-db-gated-tests.sh` (background; about two hours — do the docs pass while it runs);
  - `python3 scripts/check_closing_keywords.py` on every commit message and on the PR body.

  Every gate must pass; report any failure with its output.
- [ ] **Step 3:** Push; take PR #717 out of draft. The body names #697 and #701 and says what each
  task built. Whether the body closes them is the maintainer's call; ask.

---

## Paper-parity benchmark (§1.2)

- **Paper counterpart:** two patient folders clipped together while a records clerk has flagged
  that they may not be the same person. A clinician reviewing the drug chart signs for the pages of
  the patient in front of them and does not sign the other folder's pages until someone settles the
  clip. They can still read every page.
- **Steps:** reading the combined list is paper 1 → architecture-forced 1 → UI target 1. A sign-off
  is paper 1 → 1 → 1 (one gesture signs every signable line). The withheld lines are named in the
  same report, so the clinician never has to work out which lines were skipped. Lifting the hold is
  paper 1 (settle the clip) → forced 1 (one attested judgement of the link) → UI target 1 for unlink
  ("Not the same person…" in the window). Confirming takes the same one act, but only in the CLI
  until **#716**: that is the one gap, filed rather than hidden. `M ≤ N` everywhere. R1b adds no act;
  it moves the doubted-set lines from signable to withheld.
- **Time + cognitive load:** no new gesture and no added read, so the budget is unchanged. Opening a
  linked chart stays ≤ the single-chart open (R1's budget, still owed by the runbook pass, a human
  act). The cognitive load falls: each withheld line now names its real cause (a doubted link) and
  the remedy that fixes it (judge the link), where it used to name thread separation. That was the
  wrong-cause, wrong-fix pattern #697 reported. The read runs one fewer join (#701).
