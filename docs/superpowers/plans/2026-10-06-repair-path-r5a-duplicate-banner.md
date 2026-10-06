# Repair path R5a — the possible-duplicate banner (#680) — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** When a chart in the displayed record has an open duplicate proposal (R4's worker wrote
it), a banner above the medication list names the other chart and shows its active medications
read-only. **Review** opens the existing compare panel on it. From there the clinician answers in
one more act: *Same person* (the existing link) or *Different people* (a new attested unlink on a
never-linked pair). The banner also says honestly when this record's duplicate check has not run.

**Architecture:**
- `db/057` adds a view `match_proposal_open`: open proposals, minus pairs already in one record
  and pairs with an **attested** unlink. Banner and (later) worklist read only this view.
- `cairn-node` gains `duplicate_review.rs`, which has three parts:
  - a pure grouping of proposals by the other side's record;
  - two reads;
  - `record_different_people`, which calls the existing `chart_link::unlink_charts` once per open
    pair.
- The window gains `duplicates/`:
  - `view.rs` holds every sentence, as pure functions;
  - `mod.rs` holds the reads, Review's admission and the new command;
  - `src-ui/duplicates.js` draws the banner.

  `link.js`'s judgement sender is generalised so *Different people* reuses its answer placement.

**Tech Stack:** PostgreSQL ≥ 18 + `cairn_pgx` (SQL view); Rust (tokio-postgres, anyhow, serde,
Tauri 2); plain JS (no npm, no bundler).

**Spec:** `docs/superpowers/specs/2026-09-27-duplicate-repair-path-679-680-681-design.md`, section
**"R5 — the banner and the worklist (#680)"** and its sub-section **"R5a — the banner, designed
2026-10-06"**. The sub-section wins wherever the two differ. ADR-0076 (D1–D6), ADR-0077.

## Global Constraints

- **AGPL-3.0; no new dependency.** No new crate, so no lockfile changes in any of the three Cargo
  trees.
- **TDD.** Every behaviour starts with a test that fails for the right reason.
- **Nothing here links.** The only identity write is `chart_link::unlink_charts`, an attested
  human judgement. No node-key fallback (ADR-0053).
- **The banner is ambient and never a modal or alert.** It is a `<section>` with
  `aria-labelledby` (an implicit region), **never `role="alert"`**. It never takes focus and is
  never re-popped (§5.12). No confirmation dialog anywhere (principle 3).
- **An absent banner means only "checked, none open".** Every failed read is a worded line in
  `DuplicateSection` (`error`, an entry note, or a check line). It is never an empty section.
- **The other record's medication lines are separate data**, never part of `ChartPane::list`, so
  they can never become a sign-off or cease target.
- **Every sentence lives in Rust** (`duplicates/view.rs`), with goldens. The JS renders and decides
  nothing. Its only state is which buttons are visible.
- **`AppState::shown` is NOT widened.** Review's admission re-reads the view at that moment.
- **`tokio::sync::Mutex` is not re-entrant.** `read_chart_of` and `chart_set_of` take
  `state.db`'s lock themselves, so **never call either while holding the lock**. Holding it
  deadlocks the window.
- **`SCHEMA_GENERATION` 56 → 57.** db/057 joins `cairn-node`'s loader list only. cairn-sync's list
  lags legitimately (#284).
- **Migration replay:** every db/*.sql re-runs on every connect. Use `CREATE OR REPLACE VIEW` with
  an explicit column list (no `mp.*`) and seed no rows. SQL is `include_str!`, so a db/*.sql edit
  needs a REBUILD before a Rust test sees it.
- **Files under 500 lines.**
  - `link/mod.rs` (497) must not grow: Review's admission moves into `duplicates/mod.rs`, a
    one-line call replaces seven lines.
  - `chart_set.rs` (621), `commands.rs` (655) and `link.js` (348) grow by ≤ 6 lines each.
  - New files stay well under 500.
- **House rule 6:** no literal key material; no binding named `salt`/`nonce`/`iv`.
- **Commit messages** say `Refs #680`, never a closing keyword. Run
  `python3 scripts/check_closing_keywords.py <msgfile>` before every commit.
- **Subagents run FOREGROUND tests only.** DB-gated suites are re-run by the controller with
  `--nocapture`, checking that no `skipped:` line appears (a self-skip prints "ok").
- **DB env** (`scripts/pg-target.sh` prints the cluster; use its port):
  ```bash
  export CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=$USER dbname=cairn_test" \
         CAIRN_TEST_PG2="host=127.0.0.1 port=5532 user=$USER dbname=cairn_test2" \
         CAIRN_TEST_PG3="host=127.0.0.1 port=5532 user=$USER dbname=cairn_test3"
  ```
  Rust: `CARGO_TARGET_DIR=/tmp/cairn-r5a-target` when an IDE is open (trap 18). The GUI tree needs
  `CAIRN_ALLOW_DB_SKIP=1` for a DB-free run.
- **`cargo doc` with `RUSTDOCFLAGS=-D warnings`** fails on an intra-doc link to a private item.
  Write private names in plain backticks, never `[`…`]`.

## Review Focus

1. **A proposal whose in-record side is a LINKED MEMBER, not the opened chart.** The banner still
   shows on the opened chart. *Different people* records the unlink on the (member, other) pair,
   with `opened = None`. Pinned in Task 2 (DB) and Task 3 (DB).
2. **Two of this record's charts each proposed against charts of ONE other record.** This makes
   one banner entry, not two. *Different people* records both pairs. Pinned in Task 2 (pure + DB)
   and Task 3 (DB).
3. **A peer's UN-attested unlink arriving by sync.** The entry stays; only an attested one clears
   it (the design's correction). Pinned in Task 1 (DB, view) and Task 2 (DB, read).
4. **Review or *Different people* pressed after a colleague's judgement already resolved the
   pair.** Refused with a reload sentence, never a comparison signed over a stale banner. Pinned in
   Task 3 (`NothingOpen`) and Task 5 (the admission sentence).
5. **The proposal read, a member's pending check, or the node status cannot be read.** Each is a
   worded line; the medication list still opens. Pinned in Task 4 (pure).

---

## File structure

| File | Change | Responsibility |
|---|---|---|
| `db/057_match_proposal_open.sql` | **create** | the `match_proposal_open` view + grant |
| `crates/cairn-event/src/schema_generation.rs` | modify | `SCHEMA_GENERATION` = 57 |
| `crates/cairn-node/src/db.rs` | modify | loader list entry for db/057 |
| `crates/cairn-node/src/chart_link.rs` | modify | `OPEN_PROPOSAL_STATUSES` becomes `pub` |
| `crates/cairn-node/tests/common/mod.rs` | modify | `seed_proposal` promoted from `chart_link.rs` |
| `crates/cairn-node/tests/identity_scaffolding_shared.rs` | modify | expected-helper list + `seed_proposal` |
| `crates/cairn-node/tests/chart_link.rs` | modify | drop its local `seed_proposal`, import common's |
| `crates/cairn-node/tests/match_proposal_open.rs` | **create** | DB tests of the view + its drift guard |
| `crates/cairn-node/src/duplicate_review.rs` | **create** | `OpenProposal`, `PossibleDuplicate`, `orient`, `group_by_other_record`, reads, `record_different_people` |
| `crates/cairn-node/src/lib.rs` | modify | `pub mod duplicate_review;` |
| `crates/cairn-node/tests/duplicate_review.rs` | **create** | DB tests of the reads and the judgement |
| `cairn-gui/cairn-gui-tauri/src/duplicates/view.rs` | **create** | every banner sentence (pure) |
| `cairn-gui/cairn-gui-tauri/src/duplicates/view_tests.rs` | **create** | goldens for `view.rs` |
| `cairn-gui/cairn-gui-tauri/src/duplicates/mod.rs` | **create** | `duplicate_section`, `admit_other`, `different_people_impl`, the Tauri command |
| `cairn-gui/cairn-gui-tauri/src/main.rs` | modify | `mod duplicates;` + register the command |
| `cairn-gui/cairn-gui-tauri/src/chart_set.rs` | modify | `ChartPane::duplicates`; `chart_pane` takes it |
| `cairn-gui/cairn-gui-tauri/src/commands.rs` | modify | `med_list_impl` builds the section; drift-guard fixture |
| `cairn-gui/cairn-gui-tauri/src/link/mod.rs` | modify | `resolve_pair` calls `duplicates::admit_other` |
| `cairn-gui/cairn-gui-tauri/src-ui/index.html` | modify | the banner section, the *Different people* button, the script tag |
| `cairn-gui/cairn-gui-tauri/src-ui/duplicates.js` | **create** | draw the banner; Review; *Different people* |
| `cairn-gui/cairn-gui-tauri/src-ui/link.js` | modify | `sendJudgement(command, button)`; clear/label the new button |
| `cairn-gui/cairn-gui-tauri/src-ui/main.js` | modify | `render` / `clearChart` call the banner |
| `cairn-gui/cairn-gui-tauri/results/RUNBOOK.md` | modify | §11: the banner's stopwatch pass |
| design page, HANDOVER, ROADMAP | modify | as-built note; state |

---

### Task 1: db/057 — the open-proposal view

**Files:**
- Create: `db/057_match_proposal_open.sql`
- Modify: `crates/cairn-event/src/schema_generation.rs` (the constant and its doc line, ~line 42)
- Modify: `crates/cairn-node/src/db.rs` (after the `056_match_pending` entry, ~line 366)
- Modify: `crates/cairn-node/src/chart_link.rs:229` (`const` → `pub const`, with a doc comment)
- Modify: `crates/cairn-node/tests/common/mod.rs`, `tests/identity_scaffolding_shared.rs`,
  `tests/chart_link.rs` (promote `seed_proposal`)
- Test: `crates/cairn-node/tests/match_proposal_open.rs`

**Interfaces:**
- Produces: the view `match_proposal_open` (columns `patient_low, patient_high, score_total,
  band, veto_findings, evidence, matcher_version, status, created_at, updated_at`);
  `pub const cairn_node::chart_link::OPEN_PROPOSAL_STATUSES: [&str; 3]`;
  test helper `common::seed_proposal(c: &Client, a: Uuid, b: Uuid, status: &str)`.

- [ ] **Step 1: Promote `seed_proposal` to `tests/common/mod.rs`.**
  - Move the function verbatim from `tests/chart_link.rs` (lines ~71–89, doc comment included)
    to the end of `tests/common/mod.rs`, as `pub async fn seed_proposal(...)`.
  - In `chart_link.rs`, delete the local copy and add `seed_proposal` to its
    `use common::{…}` line.
  - In `identity_scaffolding_shared.rs::derivation_finds_the_expected_helpers`, add
    `"async fn seed_proposal("` to the expected vector, in sorted position.
  - Add a comment line to that test's list: "`seed_proposal` (R5a): `chart_link.rs`'s fixture,
    promoted when `match_proposal_open.rs` and `duplicate_review.rs` needed the identical shape."
  - Run, expecting both to PASS:
    `cargo test -p cairn-node --test identity_scaffolding_shared` and
    `cargo test -p cairn-node --test chart_link --no-run`.

- [ ] **Step 2: Write the failing view tests** in `crates/cairn-node/tests/match_proposal_open.rs`:

```rust
//! db/057 (repair path R5a, #680): `match_proposal_open` is the ONE answer to "which proposals
//! still need a human". A pair leaves it when (1) both charts read as one record, or (2) an
//! ATTESTED unlink stands for it — never for an un-attested one: unlinks are not veto-gated and
//! the ADR-0030 agent writer can author one, so counting it would let any unreviewed writer
//! silently clear a duplicate banner (design page "R5a", the correction to the R5 bullets).
//!
//! DB-gated on $CAIRN_TEST_PG; serialized through `db::test_serial_guard`. Key material is
//! minted at runtime (house rule 6).
mod common;
use cairn_node::chart_link::{LinkVerb, OPEN_PROPOSAL_STATUSES};
use cairn_node::db;
use common::{
    apply_remote_attested, apply_remote_raw, cs, enroll_human, link_assertion_event,
    register_pair, seed_proposal, setup, submit_link_event, submit_registration,
};
use tokio_postgres::Client;
use uuid::Uuid;

const TABLES: [&str; 5] = [
    "patient_link",
    "person_member",
    "identity_projection_flag",
    "link_veto_flag",
    "match_proposal",
];

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
}

/// The open pairs, canonical, sorted.
async fn open_pairs(c: &Client) -> Vec<(Uuid, Uuid)> {
    c.query(
        "SELECT patient_low::text, patient_high::text FROM match_proposal_open ORDER BY 1, 2",
        &[],
    )
    .await
    .unwrap()
    .iter()
    .map(|r| {
        (
            r.get::<_, String>(0).parse().unwrap(),
            r.get::<_, String>(1).parse().unwrap(),
        )
    })
    .collect()
}

fn canon(a: Uuid, b: Uuid) -> (Uuid, Uuid) {
    (a.min(b), a.max(b))
}

#[test]
fn the_view_lists_exactly_chart_links_open_statuses() {
    // Trap 16: pin the COMPOSED expression, not its pieces. If chart_link.rs gains or loses an
    // open status, the banner and the judgement writer would disagree on what "open" means.
    let sql = include_str!("../../../db/057_match_proposal_open.sql");
    let quoted: Vec<String> = OPEN_PROPOSAL_STATUSES
        .iter()
        .map(|s| format!("'{s}'"))
        .collect();
    let expected = format!("mp.status IN ({})", quoted.join(", "));
    assert!(sql.contains(&expected), "db/057 must contain `{expected}`");
}

#[tokio::test]
async fn open_statuses_are_listed_and_closed_ones_are_not() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &TABLES).await;
    let mut expected = vec![];
    for status in [
        "pending", "accepted", "review", "rejected", "applied", "auto_applied", "retracted",
    ] {
        let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
        register_pair(&c, &sk, &kid, a, b).await;
        seed_proposal(&c, a, b, status).await;
        if OPEN_PROPOSAL_STATUSES.contains(&status) {
            expected.push(canon(a, b));
        }
    }
    expected.sort();
    assert_eq!(open_pairs(&c).await, expected);
}

#[tokio::test]
async fn a_pair_already_in_one_record_is_not_open_directly_or_through_a_third_chart() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &TABLES).await;
    let (a, b, m, x, y) = (
        Uuid::now_v7(),
        Uuid::now_v7(),
        Uuid::now_v7(),
        Uuid::now_v7(),
        Uuid::now_v7(),
    );
    register_pair(&c, &sk, &kid, a, b).await;
    submit_registration(&c, &sk, &kid, m, 1).await;
    register_pair(&c, &sk, &kid, x, y).await;
    seed_proposal(&c, a, b, "pending").await;
    seed_proposal(&c, x, y, "pending").await;
    // a–m–b: one record through m (an un-attested link moves no proposal status, so only the
    // component filter can hide a–b). x–y: directly linked.
    submit_link_event(&c, &sk, &kid, a, m, 10, true).await;
    submit_link_event(&c, &sk, &kid, m, b, 11, true).await;
    submit_link_event(&c, &sk, &kid, x, y, 12, true).await;
    assert_eq!(open_pairs(&c).await, vec![]);
}

#[tokio::test]
async fn only_an_attested_unlink_closes_a_pair() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &TABLES).await;
    let (sk_h, kid_h) = enroll_human(&c).await;
    let (a, b, x, y) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
    register_pair(&c, &sk, &kid, a, b).await;
    register_pair(&c, &sk, &kid, x, y).await;
    seed_proposal(&c, a, b, "pending").await;
    seed_proposal(&c, x, y, "pending").await;
    // Both arrive through the SYNC door, which never moves a local proposal's status — so the
    // view's patient_link filter is the only thing that can hide either pair.
    let human = link_assertion_event(&kid_h, a, b, LinkVerb::Unlink, now_ms(), 0, "peer", true);
    apply_remote_attested(&c, &sk_h, human, &sk_h, &kid_h)
        .await
        .expect("the peer's attested unlink lands");
    let agent = link_assertion_event(&kid, x, y, LinkVerb::Unlink, now_ms(), 0, "peer", false);
    apply_remote_raw(&c, &sk, agent)
        .await
        .expect("the peer's un-attested unlink lands");
    assert_eq!(
        open_pairs(&c).await,
        vec![canon(x, y)],
        "the attested unlink closes a–b; the agent's unlink must NOT close x–y"
    );
}

#[tokio::test]
async fn the_view_survives_a_schema_replay() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &TABLES).await;
    let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
    register_pair(&c, &sk, &kid, a, b).await;
    seed_proposal(&c, a, b, "pending").await;
    drop(c);
    let c = db::connect_and_load_schema(&base).await.unwrap();
    assert_eq!(open_pairs(&c).await, vec![canon(a, b)]);
}
```

  Check `apply_remote_raw`'s exact signature in `tests/common/mod.rs` (~line 355) before
  compiling; it takes `(c, sk, body)`.

- [ ] **Step 3: Run the tests and confirm they fail.**
  - Run `cargo test -p cairn-node --test match_proposal_open -- --nocapture`.
  - Expected: a compile error (`OPEN_PROPOSAL_STATUSES` is private, and the `include_str!` file
    is missing).

- [ ] **Step 4: Make `OPEN_PROPOSAL_STATUSES` public.** In `crates/cairn-node/src/chart_link.rs:229`:

```rust
/// The `match_proposal.status` values a human judgement may still move — "open". db/057's view
/// `match_proposal_open` lists exactly these (`tests/match_proposal_open.rs` pins the composed
/// SQL), so the banner and this writer always agree on what "open" means.
pub const OPEN_PROPOSAL_STATUSES: [&str; 3] = ["pending", "accepted", "review"];
```

- [ ] **Step 5: Write `db/057_match_proposal_open.sql`.**

```sql
-- db/057_match_proposal_open.sql
-- Repair path R5a (#680; design page "R5a — the banner, designed 2026-10-06").
--
-- WHAT: the proposals that still need a human. R4's worker writes match_proposal rows (db/017);
-- a human answers one from the possible-duplicate banner (R5a) or, later, the worklist (R5b).
-- This view is the ONE predicate both read, so they can never disagree on what is "open".
--
-- A ROW IS OPEN WHEN, all three:
--   1. its status is one a human judgement may still move — exactly chart_link.rs's
--      OPEN_PROPOSAL_STATUSES (pinned by tests/match_proposal_open.rs);
--   2. its two charts do NOT read as one record (same person_member.person_id, db/018). Any
--      standing link joins them — attested or not; a doubted un-attested link is R1b's to show,
--      not this view's to hide behind. A chart never touched by linkage has no person_member
--      row and so is never "the same record" as anything;
--   3. there is NO ATTESTED unlink for the pair in patient_link. ONLY attested: unlinks are not
--      veto-gated and the ADR-0030 agent writer can author one, so counting an un-attested
--      unlink would let any unreviewed writer silently clear a duplicate banner (the mirror of
--      R1b's ruling that an un-attested unlink is not a doubt). `attested` is the STORED column
--      (ADR-0076 decision 5) — never re-derive it through event_log.
--
-- CONVERGENCE WITHOUT SYNCING match_proposal: the proposal table is node-local and does not
-- replicate. A colleague's attested judgement arriving by sync changes patient_link /
-- person_member, and this view drops the pair at read time — no status write anywhere. A local
-- judgement also moves the row's status (chart_link.rs), which (1) catches first.
--
-- An explicit column list, never mp.*: every db/*.sql replays on every connect, and
-- CREATE OR REPLACE VIEW may only APPEND columns — an expanded * would change shape the day
-- match_proposal gains one.
CREATE OR REPLACE VIEW match_proposal_open AS
SELECT mp.patient_low, mp.patient_high, mp.score_total, mp.band, mp.veto_findings,
       mp.evidence, mp.matcher_version, mp.status, mp.created_at, mp.updated_at
  FROM match_proposal mp
 WHERE mp.status IN ('pending', 'accepted', 'review')
   AND NOT EXISTS (
         SELECT 1
           FROM person_member a
           JOIN person_member b ON b.person_id = a.person_id
          WHERE a.patient_id = mp.patient_low
            AND b.patient_id = mp.patient_high)
   AND NOT EXISTS (
         SELECT 1
           FROM patient_link pl
          WHERE pl.low = mp.patient_low
            AND pl.high = mp.patient_high
            AND pl.state = 'unlink'
            AND pl.attested);

-- cairn_agent may already read match_proposal (db/017); the view adds no reach beyond it.
GRANT SELECT ON match_proposal_open TO cairn_agent;
```

- [ ] **Step 6: Register db/057 and bump the generation.**
  - In `crates/cairn-node/src/db.rs`, after the `056_match_pending` entry:

```rust
    // db/057 (repair path R5a, #680): `match_proposal_open`, the one "still needs a human"
    // predicate the banner (and R5b's worklist) read. cairn-sync's list lags legitimately (#284).
    (
        "057_match_proposal_open",
        include_str!("../../../db/057_match_proposal_open.sql"),
    ),
```

  - In `crates/cairn-event/src/schema_generation.rs`:
    - set `pub const SCHEMA_GENERATION: i32 = 57;`
    - change the doc line to `(`db/057_match_proposal_open.sql` → 57)`.

- [ ] **Step 7: Rebuild and run the tests; confirm they pass.**
  - Run `cargo test -p cairn-node --test match_proposal_open -- --nocapture` (all PASS, no
    `skipped:`).
  - Run `cargo test -p cairn-event --test schema_generation` (PASS).
  - Run `cargo test -p cairn-node --test chart_link -- --nocapture` (PASS: the moved helper).

- [ ] **Step 8: Mutation check.**
  - In db/057, change `AND pl.attested` to `AND TRUE`, rebuild and run
    `only_an_attested_unlink_closes_a_pair`. It must FAIL.
  - Restore the file (copy from scratchpad, never `git checkout --`), rebuild, and confirm PASS.

- [ ] **Step 9: Commit.**

```bash
git add db/057_match_proposal_open.sql crates/cairn-event/src/schema_generation.rs \
  crates/cairn-node/src/db.rs crates/cairn-node/src/chart_link.rs \
  crates/cairn-node/tests/{common/mod.rs,identity_scaffolding_shared.rs,chart_link.rs,match_proposal_open.rs}
git commit -m "feat(R5a): db/057 — match_proposal_open, the one 'still needs a human' predicate (Refs #680)"
```

---

### Task 2: the node's banner read — `duplicate_review.rs` (pure grouping + two reads)

**Files:**
- Create: `crates/cairn-node/src/duplicate_review.rs`
- Modify: `crates/cairn-node/src/lib.rs` (`pub mod duplicate_review;`, alphabetical, after
  `duplicate_check`)
- Test: inline `#[cfg(test)] mod tests` (pure) + `crates/cairn-node/tests/duplicate_review.rs` (DB)

**Interfaces:**
- Consumes: the view `match_proposal_open` (Task 1); `crate::patient::person::person_charts`;
  `crate::chart_link::canonical_pair`.
- Produces:
  - `pub struct OpenProposal { pub here: Uuid, pub other: Uuid, pub vetoed: bool, pub created_ms: i64 }`
  - `pub struct PossibleDuplicate { pub other_record: ChartSet, pub review_chart: Uuid, pub pairs: Vec<(Uuid, Uuid)>, pub vetoed: bool, pub newest_ms: i64 }`
  - `pub fn orient(low: Uuid, high: Uuid, charts: &ChartSet) -> Option<(Uuid, Uuid)>` → `(here, other)`
  - `pub fn group_by_other_record(found: Vec<(OpenProposal, ChartSet)>) -> Vec<PossibleDuplicate>`
  - `pub async fn open_proposals_touching(client: &(impl GenericClient + Sync), charts: &ChartSet) -> anyhow::Result<Vec<OpenProposal>>`
  - `pub async fn possible_duplicates(client: &(impl GenericClient + Sync), charts: &ChartSet) -> anyhow::Result<Vec<PossibleDuplicate>>`
  - `pub async fn open_pairs_between(client: &(impl GenericClient + Sync), left: &ChartSet, right: &ChartSet) -> anyhow::Result<Vec<(Uuid, Uuid)>>`

- [ ] **Step 1: Write the failing pure tests** in `duplicate_review.rs`'s test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }
    fn set(v: &[u128]) -> ChartSet {
        ChartSet::new(v.iter().map(|n| id(*n))).unwrap()
    }
    fn prop(here: u128, other: u128, vetoed: bool, created_ms: i64) -> OpenProposal {
        OpenProposal { here: id(here), other: id(other), vetoed, created_ms }
    }

    #[test]
    fn a_pair_is_oriented_by_which_side_the_record_holds() {
        let record = set(&[1, 2]);
        assert_eq!(orient(id(1), id(9), &record), Some((id(1), id(9))));
        assert_eq!(orient(id(2), id(9), &record), Some((id(2), id(9))));
        assert_eq!(orient(id(0), id(1), &record), Some((id(1), id(0))));
        // Both inside or both outside: not a banner pair.
        assert_eq!(orient(id(1), id(2), &record), None);
        assert_eq!(orient(id(8), id(9), &record), None);
    }

    /// Review Focus 2: two of my charts proposed against two charts of ONE other record are one
    /// entry, standing for both pairs; Review compares against the NEWEST proposal's chart.
    #[test]
    fn proposals_against_one_other_record_are_one_entry() {
        let other = set(&[8, 9]);
        let got = group_by_other_record(vec![
            (prop(1, 8, false, 100), other.clone()),
            (prop(2, 9, true, 200), other.clone()),
        ]);
        assert_eq!(got.len(), 1);
        let e = &got[0];
        assert_eq!(e.other_record, other);
        assert_eq!(e.review_chart, id(9), "the newest proposal's other chart");
        assert_eq!(e.pairs, vec![(id(1), id(8)), (id(2), id(9))]);
        assert!(e.vetoed, "any vetoed pair marks the entry");
        assert_eq!(e.newest_ms, 200);
    }

    #[test]
    fn entries_are_newest_first_and_records_stay_apart() {
        let got = group_by_other_record(vec![
            (prop(1, 7, false, 100), set(&[7])),
            (prop(1, 9, false, 300), set(&[9])),
        ]);
        let order: Vec<Uuid> = got.iter().map(|e| e.review_chart).collect();
        assert_eq!(order, vec![id(9), id(7)]);
    }

    #[test]
    fn nothing_found_is_no_entries() {
        assert!(group_by_other_record(vec![]).is_empty());
    }
}
```

- [ ] **Step 2: Run them and confirm they fail.**
  - Run `cargo test -p cairn-node --lib duplicate_review`.
  - Expected: a compile error (module or items not found).

- [ ] **Step 3: Write the module** (header, types, pure functions, reads):

```rust
//! The possible-duplicate banner's node reads, and its "Different people" judgement (repair
//! path R5a, #680; design page "R5a — the banner, designed 2026-10-06").
//!
//! R4's worker writes `match_proposal` rows (db/017). Which of them still need a human is ONE
//! answer — db/057's view `match_proposal_open` — and everything here reads only that view:
//! - [`possible_duplicates`]: every open proposal between a displayed chart set and a chart
//!   OUTSIDE it, grouped by the other side's RECORD, so the banner shows one entry per other
//!   person however many of their charts were proposed;
//! - [`open_pairs_between`]: the open pairs joining two records — Review's admission (the window
//!   does not widen `AppState::shown`; it asks this, at that moment) and what "Different people"
//!   judges;
//! - [`record_different_people`]: an attested unlink on each of those pairs.
//!
//! Nothing here links, and nothing here writes a proposal status: `chart_link::unlink_charts`
//! writes the event and moves the proposal, in its own transaction, as it does for R2b-2.

use crate::chart_link::{canonical_pair, unlink_charts, LinkOutcome, Reviewer};
use crate::patient::person::person_charts;
use anyhow::Context;
use cairn_medication_view::ChartSet;
use std::collections::{BTreeMap, HashMap};
use tokio_postgres::{Client, GenericClient};
use uuid::Uuid;

/// One open proposal that crosses a record's boundary: `here` is inside the displayed set,
/// `other` outside it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenProposal {
    pub here: Uuid,
    pub other: Uuid,
    /// The matcher recorded veto findings for the pair (shown as a note; Review shows WHICH,
    /// read fresh by the compare panel — never worded here from the stored JSON).
    pub vetoed: bool,
    /// When the proposal was written, epoch milliseconds — ordering only.
    pub created_ms: i64,
}

/// One banner entry: every open proposal between the displayed set and ONE other record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PossibleDuplicate {
    /// The other side's whole record (`person_charts`), so the banner can name every chart of
    /// that person and read their medications as one list.
    pub other_record: ChartSet,
    /// The chart Review compares against: the other side of the NEWEST proposal.
    pub review_chart: Uuid,
    /// Every open pair this entry stands for, canonical `(low, high)`, sorted, no duplicates —
    /// what "Different people" records an unlink on.
    pub pairs: Vec<(Uuid, Uuid)>,
    pub vetoed: bool,
    pub newest_ms: i64,
}

/// `(here, other)` for a pair crossing `charts`'s boundary, or `None` when both or neither
/// side is inside it. **Pure.**
pub fn orient(low: Uuid, high: Uuid, charts: &ChartSet) -> Option<(Uuid, Uuid)> {
    match (charts.contains(&low), charts.contains(&high)) {
        (true, false) => Some((low, high)),
        (false, true) => Some((high, low)),
        _ => None,
    }
}

/// Group proposals by the other side's record; newest entry first. **Pure.**
///
/// Each input carries the record its `other` chart belongs to (read by the caller). Two of my
/// charts proposed against two charts of the same other person are ONE entry: the banner is
/// about people, as the front door is since R3.
pub fn group_by_other_record(found: Vec<(OpenProposal, ChartSet)>) -> Vec<PossibleDuplicate> {
    let mut groups: BTreeMap<Vec<Uuid>, PossibleDuplicate> = BTreeMap::new();
    for (p, record) in found {
        let pair = canonical_pair(p.here, p.other);
        let key = record.members().to_vec();
        match groups.get_mut(&key) {
            Some(g) => {
                g.pairs.push(pair);
                g.vetoed |= p.vetoed;
                if p.created_ms > g.newest_ms {
                    g.newest_ms = p.created_ms;
                    g.review_chart = p.other;
                }
            }
            None => {
                groups.insert(
                    key,
                    PossibleDuplicate {
                        other_record: record,
                        review_chart: p.other,
                        pairs: vec![pair],
                        vetoed: p.vetoed,
                        newest_ms: p.created_ms,
                    },
                );
            }
        }
    }
    let mut entries: Vec<PossibleDuplicate> = groups
        .into_values()
        .map(|mut g| {
            g.pairs.sort();
            g.pairs.dedup();
            g
        })
        .collect();
    entries.sort_by(|a, b| {
        b.newest_ms
            .cmp(&a.newest_ms)
            .then_with(|| a.review_chart.cmp(&b.review_chart))
    });
    entries
}

fn ids(charts: &ChartSet) -> Vec<String> {
    charts.members().iter().map(Uuid::to_string).collect()
}

/// Every open proposal with exactly one side in `charts`, newest first.
pub async fn open_proposals_touching(
    client: &(impl GenericClient + Sync),
    charts: &ChartSet,
) -> anyhow::Result<Vec<OpenProposal>> {
    let rows = client
        .query(
            "SELECT patient_low::text AS low, patient_high::text AS high, \
                    veto_findings <> '[]'::jsonb AS vetoed, \
                    (extract(epoch FROM created_at) * 1000)::bigint AS created_ms \
               FROM match_proposal_open \
              WHERE (patient_low = ANY($1::text[]::uuid[])) \
                 <> (patient_high = ANY($1::text[]::uuid[])) \
              ORDER BY created_at DESC, patient_low, patient_high",
            &[&ids(charts)],
        )
        .await
        .context("reading the open duplicate proposals for this record")?;
    rows.iter()
        .map(|r| {
            let low: Uuid = r.get::<_, String>("low").parse()?;
            let high: Uuid = r.get::<_, String>("high").parse()?;
            let (here, other) = orient(low, high, charts)
                .with_context(|| format!("proposal {low}–{high} does not cross this record"))?;
            Ok(OpenProposal {
                here,
                other,
                vetoed: r.get("vetoed"),
                created_ms: r.get("created_ms"),
            })
        })
        .collect()
}

/// The banner's entries for `charts`: open proposals grouped by the other side's record.
/// One `person_charts` read per distinct other chart.
pub async fn possible_duplicates(
    client: &(impl GenericClient + Sync),
    charts: &ChartSet,
) -> anyhow::Result<Vec<PossibleDuplicate>> {
    let open = open_proposals_touching(client, charts).await?;
    let mut records: HashMap<Uuid, ChartSet> = HashMap::new();
    let mut found = Vec::with_capacity(open.len());
    for p in open {
        let record = match records.get(&p.other) {
            Some(r) => r.clone(),
            None => {
                let r = person_charts(client, p.other).await?;
                records.insert(p.other, r.clone());
                r
            }
        };
        found.push((p, record));
    }
    Ok(group_by_other_record(found))
}

/// The open pairs joining `left` and `right` (either orientation), canonical and sorted.
/// Empty means: no open proposal joins the two records any more.
pub async fn open_pairs_between(
    client: &(impl GenericClient + Sync),
    left: &ChartSet,
    right: &ChartSet,
) -> anyhow::Result<Vec<(Uuid, Uuid)>> {
    let rows = client
        .query(
            "SELECT patient_low::text, patient_high::text FROM match_proposal_open \
              WHERE (patient_low = ANY($1::text[]::uuid[]) AND patient_high = ANY($2::text[]::uuid[])) \
                 OR (patient_low = ANY($2::text[]::uuid[]) AND patient_high = ANY($1::text[]::uuid[])) \
              ORDER BY 1, 2",
            &[&ids(left), &ids(right)],
        )
        .await
        .context("reading the open duplicate proposals between two records")?;
    rows.iter()
        .map(|r| Ok((r.get::<_, String>(0).parse()?, r.get::<_, String>(1).parse()?)))
        .collect()
}
```

  `LinkOutcome`, `Reviewer`, `unlink_charts` and `Client` are used by Task 3. Until then, put
  `#[allow(unused_imports)]` on that `use` line, and remove it in Task 3.

- [ ] **Step 4: Run the pure tests and confirm they pass.**
  - Run `cargo test -p cairn-node --lib duplicate_review` (PASS).

- [ ] **Step 5: Write the failing DB tests** in `crates/cairn-node/tests/duplicate_review.rs`:

```rust
//! Repair path R5a (#680): the banner's node read over db/057's `match_proposal_open`.
//! DB-gated on $CAIRN_TEST_PG; serialized via `db::test_serial_guard`; keys minted at runtime.
mod common;
use cairn_medication_view::ChartSet;
use cairn_node::chart_link::LinkVerb;
use cairn_node::db;
use cairn_node::duplicate_review::{open_pairs_between, possible_duplicates};
use common::{
    apply_remote_attested, apply_remote_raw, cs, enroll_human, link_assertion_event,
    register_pair, seed_proposal, setup, submit_link_event, submit_registration,
};
use uuid::Uuid;

const TABLES: [&str; 5] = [
    "patient_link",
    "person_member",
    "identity_projection_flag",
    "link_veto_flag",
    "match_proposal",
];

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
}

#[tokio::test]
async fn the_entry_shows_on_both_charts() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &TABLES).await;
    let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
    register_pair(&c, &sk, &kid, a, b).await;
    seed_proposal(&c, a, b, "pending").await;
    let on_a = possible_duplicates(&c, &ChartSet::single(a)).await.unwrap();
    let on_b = possible_duplicates(&c, &ChartSet::single(b)).await.unwrap();
    assert_eq!(on_a.len(), 1);
    assert_eq!(on_a[0].review_chart, b);
    assert_eq!(on_b.len(), 1);
    assert_eq!(on_b[0].review_chart, a);
}

/// Review Focus 1 + 2: the proposal's in-record side is a linked MEMBER (m), not the opened
/// chart; and two members proposed against one other record make one entry.
#[tokio::test]
async fn a_members_proposals_show_once_per_other_record() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &TABLES).await;
    let (a, m, x, y) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
    register_pair(&c, &sk, &kid, a, m).await;
    register_pair(&c, &sk, &kid, x, y).await;
    submit_link_event(&c, &sk, &kid, a, m, 10, true).await; // my record: a + m
    submit_link_event(&c, &sk, &kid, x, y, 11, true).await; // the other record: x + y
    seed_proposal(&c, a, x, "pending").await;
    seed_proposal(&c, m, y, "review").await;
    let mine = ChartSet::new([a, m]).unwrap();
    let got = possible_duplicates(&c, &mine).await.unwrap();
    assert_eq!(got.len(), 1, "one other person, one entry");
    assert_eq!(got[0].other_record, ChartSet::new([x, y]).unwrap());
    let mut want = vec![(a.min(x), a.max(x)), (m.min(y), m.max(y))];
    want.sort();
    assert_eq!(got[0].pairs, want);
    assert_eq!(
        open_pairs_between(&c, &mine, &ChartSet::new([x, y]).unwrap()).await.unwrap(),
        want
    );
}

/// Review Focus 3, at the read: a peer's un-attested unlink leaves the entry; an attested one
/// clears it — with no local status write.
#[tokio::test]
async fn only_a_peers_attested_unlink_clears_the_entry() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &TABLES).await;
    let (sk_h, kid_h) = enroll_human(&c).await;
    let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
    register_pair(&c, &sk, &kid, a, b).await;
    seed_proposal(&c, a, b, "pending").await;
    let agent = link_assertion_event(&kid, a, b, LinkVerb::Unlink, now_ms(), 0, "peer", false);
    apply_remote_raw(&c, &sk, agent).await.unwrap();
    assert_eq!(possible_duplicates(&c, &ChartSet::single(a)).await.unwrap().len(), 1);
    let human = link_assertion_event(&kid_h, a, b, LinkVerb::Unlink, now_ms() + 1, 0, "peer", true);
    apply_remote_attested(&c, &sk_h, human, &sk_h, &kid_h).await.unwrap();
    assert!(possible_duplicates(&c, &ChartSet::single(a)).await.unwrap().is_empty());
}

#[tokio::test]
async fn a_chart_with_no_open_proposal_has_no_entry() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &TABLES).await;
    let a = Uuid::now_v7();
    submit_registration(&c, &sk, &kid, a, 1).await;
    assert!(possible_duplicates(&c, &ChartSet::single(a)).await.unwrap().is_empty());
}
```

- [ ] **Step 6: Run the DB tests and confirm they pass.**
  - Run `cargo test -p cairn-node --test duplicate_review -- --nocapture` (all PASS, no
    `skipped:`).
  - If any fails, the cause is in Step 3's code, not the test. Read the failure before changing
    either.

- [ ] **Step 7: Commit.**

```bash
git add crates/cairn-node/src/duplicate_review.rs crates/cairn-node/src/lib.rs \
  crates/cairn-node/tests/duplicate_review.rs
git commit -m "feat(R5a): the node's possible-duplicate read, grouped by the other record (Refs #680)"
```

---

### Task 3: "Different people" on the node — `record_different_people`

**Files:**
- Modify: `crates/cairn-node/src/duplicate_review.rs` (append; remove Task 2's `allow`)
- Test: `crates/cairn-node/tests/duplicate_review.rs` (append)

**Interfaces:**
- Consumes: `open_pairs_between` (Task 2); `chart_link::{unlink_charts, Reviewer, LinkOutcome}`.
- Produces:
  - `pub enum DifferentPeople { NothingOpen, Judged(Vec<PairJudgement>) }`
  - `pub struct PairJudgement { pub low: Uuid, pub high: Uuid, pub outcome: anyhow::Result<LinkOutcome> }`
  - `pub async fn record_different_people(client: &mut Client, left: &ChartSet, right: &ChartSet, reviewer: &Reviewer<'_>, node_origin: &str) -> anyhow::Result<DifferentPeople>`

- [ ] **Step 1: Write the failing DB tests** (append to `tests/duplicate_review.rs`; add
  `record_different_people, DifferentPeople` to the `use cairn_node::duplicate_review::{…}` line,
  and `use cairn_node::chart_link::{LinkEffect, Reviewer};`):

```rust
async fn status_of(c: &tokio_postgres::Client, a: Uuid, b: Uuid) -> String {
    let (lo, hi) = (a.min(b), a.max(b));
    c.query_one(
        "SELECT status FROM match_proposal WHERE patient_low = $1::text::uuid AND patient_high = $2::text::uuid",
        &[&lo.to_string(), &hi.to_string()],
    )
    .await
    .unwrap()
    .get(0)
}

/// Review Focus 1 + 2: every open pair between the two records gets an ATTESTED unlink — the
/// member's pair included, judged with `opened = None` — each proposal moves to `rejected`, and
/// the banner empties.
#[tokio::test]
async fn different_people_records_an_attested_unlink_on_every_open_pair() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &TABLES).await;
    let (sk_h, kid_h) = enroll_human(&c).await;
    let (a, m, x, y) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
    register_pair(&c, &sk, &kid, a, m).await;
    register_pair(&c, &sk, &kid, x, y).await;
    submit_link_event(&c, &sk, &kid, a, m, 10, true).await;
    submit_link_event(&c, &sk, &kid, x, y, 11, true).await;
    seed_proposal(&c, a, x, "pending").await;
    seed_proposal(&c, m, y, "pending").await;
    let (mine, theirs) = (ChartSet::new([a, m]).unwrap(), ChartSet::new([x, y]).unwrap());
    let who = Reviewer { human_sk: &sk_h, human_kid: &kid_h };

    let out = record_different_people(&mut c, &mine, &theirs, &who, "r5a-test").await.unwrap();
    let DifferentPeople::Judged(judged) = out else { panic!("two open pairs were judged") };
    assert_eq!(judged.len(), 2);
    for j in &judged {
        let effect = j.outcome.as_ref().expect("each unlink is recorded").effect;
        assert_eq!(effect, LinkEffect::TookEffect);
    }
    for (p, q) in [(a, x), (m, y)] {
        assert_eq!(status_of(&c, p, q).await, "rejected");
        let attested: bool = c
            .query_one(
                "SELECT attested FROM patient_link WHERE low = $1::text::uuid AND high = $2::text::uuid AND state = 'unlink'",
                &[&p.min(q).to_string(), &p.max(q).to_string()],
            )
            .await
            .unwrap()
            .get(0);
        assert!(attested, "a human's judgement, attested");
    }
    assert!(possible_duplicates(&c, &mine).await.unwrap().is_empty());
}

/// Review Focus 4: a colleague already resolved it — nothing is signed.
#[tokio::test]
async fn different_people_on_a_resolved_pair_records_nothing() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &TABLES).await;
    let (sk_h, kid_h) = enroll_human(&c).await;
    let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
    register_pair(&c, &sk, &kid, a, b).await;
    seed_proposal(&c, a, b, "pending").await;
    let peer = link_assertion_event(&kid_h, a, b, LinkVerb::Unlink, now_ms(), 0, "peer", true);
    apply_remote_attested(&c, &sk_h, peer, &sk_h, &kid_h).await.unwrap();
    let before: i64 = c.query_one("SELECT count(*) FROM event_log", &[]).await.unwrap().get(0);
    let who = Reviewer { human_sk: &sk_h, human_kid: &kid_h };
    let out = record_different_people(&mut c, &ChartSet::single(a), &ChartSet::single(b), &who, "r5a-test")
        .await
        .unwrap();
    assert!(matches!(out, DifferentPeople::NothingOpen));
    let after: i64 = c.query_one("SELECT count(*) FROM event_log", &[]).await.unwrap().get(0);
    assert_eq!(before, after, "nothing signed");
}
```

- [ ] **Step 2: Run the tests and confirm they fail.**
  - Run `cargo test -p cairn-node --test duplicate_review -- --nocapture`.
  - Expected: a compile error (`record_different_people` is not defined).

- [ ] **Step 3: Implement** (append to `duplicate_review.rs`; drop the `#[allow(unused_imports)]`):

```rust
/// One pair's "Different people" judgement and what it did (or why it was not recorded).
#[derive(Debug)]
pub struct PairJudgement {
    pub low: Uuid,
    pub high: Uuid,
    pub outcome: anyhow::Result<LinkOutcome>,
}

/// What "Different people" did.
#[derive(Debug)]
pub enum DifferentPeople {
    /// No open proposal joins the two records any more (a colleague's judgement, here or by
    /// sync, resolved it since the banner was drawn). Nothing was signed.
    NothingOpen,
    /// One attested unlink per open pair, each in its own transaction (`unlink_charts`).
    Judged(Vec<PairJudgement>),
}

/// "Different people": an attested unlink on EVERY open pair between `left` (the displayed
/// record) and `right` (the other record), read fresh here — never the pairs the banner showed,
/// which may be stale. Each is `unlink_charts(low, high, None, …)`: both charts of a proposal
/// are held here (the matcher scores only local charts), so the judgement files under a subject,
/// and a pair whose in-record side is a linked member — not the chart on screen — is judged the
/// same way (#699 (a)'s third-chart filing is for a link, not a proposal).
///
/// Honest, not atomic: several pairs are several events. A failure on one is carried in its
/// [`PairJudgement`] and the others still stand; the failed pair stays open and stays on the
/// banner. `Err` only when the open pairs could not be read — nothing was signed.
pub async fn record_different_people(
    client: &mut Client,
    left: &ChartSet,
    right: &ChartSet,
    reviewer: &Reviewer<'_>,
    node_origin: &str,
) -> anyhow::Result<DifferentPeople> {
    let pairs = open_pairs_between(&*client, left, right).await?;
    if pairs.is_empty() {
        return Ok(DifferentPeople::NothingOpen);
    }
    let mut judged = Vec::with_capacity(pairs.len());
    for (low, high) in pairs {
        let outcome = unlink_charts(client, low, high, None, reviewer, node_origin).await;
        judged.push(PairJudgement { low, high, outcome });
    }
    Ok(DifferentPeople::Judged(judged))
}
```

- [ ] **Step 4: Run the tests and confirm they pass.**
  - Run `cargo test -p cairn-node --test duplicate_review -- --nocapture` (PASS, no `skipped:`).
  - Run `cargo clippy -p cairn-node --all-targets -- -D warnings` (clean).

- [ ] **Step 5: Commit.**

```bash
git add crates/cairn-node/src/duplicate_review.rs crates/cairn-node/tests/duplicate_review.rs
git commit -m "feat(R5a): 'Different people' — an attested unlink on every open pair between two records (Refs #680)"
```

---

### Task 4: the banner's wording — `duplicates/view.rs` (pure)

**Files:**
- Create: `cairn-gui/cairn-gui-tauri/src/duplicates/view.rs`
- Create: `cairn-gui/cairn-gui-tauri/src/duplicates/view_tests.rs`
- Create: `cairn-gui/cairn-gui-tauri/src/duplicates/mod.rs` (just `pub mod view;` for now)
- Modify: `cairn-gui/cairn-gui-tauri/src/main.rs` (`mod duplicates;`, alphabetical)

**Interfaces:**
- Consumes:
  - `crate::chart_set::MemberLine` (`patient_id`, `name`, `text`);
  - `crate::link::view::{medication_lines, refused, LinkReportView}`;
  - `crate::funnel::view::ErrorView`;
  - `cairn_node::duplicate_check::{chart_line, status_line, CheckState}`;
  - `cairn_node::chart_link::LinkEffect`;
  - `cairn_gui_tab_medications::view::MedListView`.
- Produces:
  - constants:
    - `MAX_SHOWN: usize = 3`
    - `HEADING`
    - `OTHER_CHART_LABEL`
    - `VETO_NOTE`
    - `NOTHING_OPEN`
    - `NOT_SHOWN_OR_RESOLVED`
    - `DIFFERENT_PEOPLE_BUTTON`
  - `#[derive(Default, Serialize)] pub struct DuplicateSection { entries, more: Option<String>, error: Option<String>, check_lines: Vec<String> }`
  - `pub struct DuplicateEntryView { review_chart, heading, identity_lines, notes, medications_heading, medications, medication_notes }` (all `String` / `Vec<String>`)
  - `pub struct ChartCheck { pub chart: Uuid, pub pending: Result<bool, String> }`
  - `pub struct PairResult { pub low: Uuid, pub high: Uuid, pub outcome: Result<LinkEffect, ErrorView> }`
  - `pub fn entry_view(review_chart: Uuid, vetoed: bool, identities: Result<Vec<MemberLine>, String>, meds: Result<MedListView, String>) -> DuplicateEntryView`
  - `pub fn section_view(entries: Result<(Vec<DuplicateEntryView>, usize), String>, check_lines: Vec<String>) -> DuplicateSection`
  - `pub fn check_lines(opened: Uuid, checks: &[ChartCheck], members: &[MemberLine], status: Result<CheckState, String>) -> Vec<String>`
  - `pub fn fixture_section() -> DuplicateSection`
  - `pub fn different_people_report(results: Vec<PairResult>) -> Result<LinkReportView, ErrorView>`

- [ ] **Step 1: Write the failing tests** in `duplicates/view_tests.rs`:

```rust
//! Goldens for every banner sentence (R5a). The wording IS the safety content (principle 3):
//! an absent banner must only ever mean "checked, none open".
use super::*;
use crate::chart_set::MemberLine;
use cairn_node::chart_link::LinkEffect;
use cairn_node::duplicate_check::CheckState;
use uuid::Uuid;

fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}

fn member(n: u128, name: &str) -> MemberLine {
    MemberLine {
        patient_id: id(n).to_string(),
        name: name.into(),
        text: format!("{name} · born 1950-01-07 · identity confirmed · chart {}", id(n)),
    }
}

/// Review Focus 5 (golden): a never-proposed, checked chart gets NOTHING — the webview hides
/// the section, so the chart reads exactly as before R5a.
#[test]
fn a_checked_chart_with_no_proposal_has_an_empty_section() {
    let checks = [ChartCheck { chart: id(1), pending: Ok(false) }];
    let lines = check_lines(id(1), &checks, &[], Ok(CheckState::Current { last_ran: None }));
    assert_eq!(section_view(Ok((vec![], 0)), lines), DuplicateSection::default());
}

/// Review Focus 5: a failed proposal read is an error line, never an empty banner.
#[test]
fn a_failed_proposal_read_is_worded_never_empty() {
    let s = section_view(Err("connection reset".into()), vec![]);
    assert_eq!(
        s.error.as_deref(),
        Some("Could not check for possible duplicates: connection reset")
    );
    assert!(s.entries.is_empty());
}

#[test]
fn an_entry_names_the_other_chart_and_its_current_medications() {
    // The shared fixture carries both current and ceased drugs (link/view_tests.rs relies on it).
    let source = cairn_medication_view::fixtures::sample_chart();
    let active = source
        .rows
        .iter()
        .filter(|r| r.status == cairn_medication_view::MedicationStatus::Active)
        .count();
    assert!(active > 0 && active < source.rows.len());
    let meds = cairn_gui_tab_medications::view::build_view(&source);
    let e = entry_view(id(9), false, Ok(vec![member(9, "Mary SMYTHE")]), Ok(meds));
    assert_eq!(e.heading, "Possible duplicate — not yet reviewed");
    assert_eq!(
        e.identity_lines,
        vec![format!("Mary SMYTHE · born 1950-01-07 · identity confirmed · chart {}", id(9))]
    );
    assert_eq!(e.medications_heading, "On the other chart — not part of this record");
    assert_eq!(e.medications.len(), active, "ceased lines are not the other chart's current drugs");
    assert!(e.notes.is_empty());
    assert_eq!(e.review_chart, id(9).to_string());
}

#[test]
fn a_vetoed_entry_says_facts_disagree_and_points_at_review() {
    let e = entry_view(id(9), true, Ok(vec![member(9, "X")]), Err("x".into()));
    assert!(e.notes.contains(&VETO_NOTE.to_string()));
}

#[test]
fn unread_parts_of_an_entry_are_worded_and_the_entry_is_kept() {
    let e = entry_view(id(9), false, Err("timeout".into()), Err("sealed".into()));
    assert_eq!(e.identity_lines, vec![format!("chart {}", id(9))]);
    assert!(e.notes.iter().any(|n| n == "The other chart's name and date of birth could not be read: timeout"));
    assert_eq!(e.medications, Vec::<String>::new());
    assert_eq!(e.medication_notes, vec!["Its medications could not be read here: sealed".to_string()]);
}

#[test]
fn entries_beyond_the_cap_are_counted_never_dropped_silently() {
    let e = entry_view(id(9), false, Ok(vec![]), Err("x".into()));
    let s = section_view(Ok((vec![e.clone(), e.clone(), e], 5)), vec![]);
    assert_eq!(s.more.as_deref(), Some("2 more possible duplicates of this record are not shown here."));
    let e = entry_view(id(9), false, Ok(vec![]), Err("x".into()));
    let s = section_view(Ok((vec![e], 2)), vec![]);
    assert_eq!(s.more.as_deref(), Some("1 more possible duplicate of this record is not shown here."));
}

#[test]
fn a_pending_member_is_named_and_the_node_status_explains_it() {
    let checks = [
        ChartCheck { chart: id(1), pending: Ok(true) },
        ChartCheck { chart: id(2), pending: Ok(true) },
        ChartCheck { chart: id(3), pending: Err("boom".into()) },
    ];
    let lines = check_lines(
        id(1),
        &checks,
        &[member(2, "Ann LEE")],
        Ok(CheckState::CatchingUp { waiting: 2, config_recheck: false }),
    );
    assert_eq!(
        lines,
        vec![
            "This chart: duplicate check not yet run since its identity details last changed.".to_string(),
            format!("Linked chart Ann LEE (chart {}): duplicate check not yet run since its identity details last changed.", id(2)),
            format!("Linked chart {}: duplicate check status unknown — boom", id(3)),
            "Duplicate check running — 2 charts waiting.".to_string(),
        ]
    );
}

#[test]
fn a_stalled_node_says_so_even_when_this_record_is_checked() {
    let checks = [ChartCheck { chart: id(1), pending: Ok(false) }];
    let lines = check_lines(
        id(1),
        &checks,
        &[],
        Ok(CheckState::Stalled { waiting: 3, last_ran: Some("09:15".into()) }),
    );
    assert_eq!(lines, vec!["Duplicate check is behind — last ran 09:15; 3 charts waiting.".to_string()]);
}

#[test]
fn an_unreadable_node_status_is_never_omitted() {
    let checks = [ChartCheck { chart: id(1), pending: Ok(false) }];
    let lines = check_lines(id(1), &checks, &[], Err("denied".into()));
    assert_eq!(lines, vec!["Duplicate check status unknown: denied".to_string()]);
}

#[test]
fn fixture_mode_says_no_check_has_run() {
    let s = fixture_section();
    assert_eq!(s.check_lines, vec!["Duplicate check has never run on this node.".to_string()]);
    assert!(s.entries.is_empty() && s.error.is_none());
}

#[test]
fn different_people_reports_each_pair_and_reloads() {
    let r = different_people_report(vec![PairResult {
        low: id(1),
        high: id(9),
        outcome: Ok(LinkEffect::TookEffect),
    }])
    .unwrap();
    assert!(r.reload);
    assert_eq!(
        r.sentence,
        format!(
            "Recorded: charts {} and {} are different people. This possible duplicate is closed on \
             this node, and on every node once the judgement syncs.",
            id(1),
            id(9)
        )
    );
}

#[test]
fn a_partly_failed_judgement_names_the_pair_left_open() {
    let r = different_people_report(vec![
        PairResult { low: id(1), high: id(8), outcome: Ok(LinkEffect::TookEffect) },
        PairResult { low: id(2), high: id(9), outcome: Err(refused("held elsewhere")) },
    ])
    .unwrap();
    assert!(r.sentence.contains(&format!(
        "NOT recorded for charts {} and {}: held elsewhere — it stays on the banner.",
        id(2),
        id(9)
    )));
}

#[test]
fn a_wholly_failed_judgement_is_the_refusal_itself() {
    let err = different_people_report(vec![PairResult {
        low: id(1),
        high: id(9),
        outcome: Err(refused("held elsewhere")),
    }])
    .unwrap_err();
    assert_eq!(err.text, "held elsewhere");
    assert_eq!(different_people_report(vec![]).unwrap_err().text, NOTHING_OPEN);
}
```

- [ ] **Step 2: Run the tests and confirm they fail.**
  - Run `cd cairn-gui && CAIRN_ALLOW_DB_SKIP=1 cargo test -p cairn-gui-tauri duplicates`.
  - Expected: a compile error.

- [ ] **Step 3: Implement `view.rs`:**

```rust
//! Every sentence the possible-duplicate banner shows, as pure functions (repair path R5a,
//! #680; design page "R5a — the banner, designed 2026-10-06").
//!
//! The banner is AMBIENT (§5.12): it never takes focus, never re-pops, and is never a modal.
//! What it may never do is be ABSENT when something is unknown — an empty section must only
//! ever mean "checked, none open". So every failed read below becomes a worded line, and
//! [`check_lines`] says when this record's check has not run, reusing R4's own sentences.
use crate::chart_set::MemberLine;
use crate::funnel::view::ErrorView;
use crate::link::view::{medication_lines, refused, LinkReportView};
use cairn_gui_tab_medications::view::MedListView;
use cairn_node::chart_link::LinkEffect;
use cairn_node::duplicate_check::{chart_line, status_line, CheckState};
use serde::Serialize;
use uuid::Uuid;

/// At most this many entries are drawn (each costs a medication read at chart open); the rest
/// are counted in `DuplicateSection::more`, never dropped silently.
pub const MAX_SHOWN: usize = 3;
pub const HEADING: &str = "Possible duplicate — not yet reviewed";
/// Above the other chart's lines: they are someone else's until a human links the two.
pub const OTHER_CHART_LABEL: &str = "On the other chart — not part of this record";
/// A proposal with veto findings. Which facts — and whether "verified" applies — is the compare
/// panel's to say (R2b-1's rule), read fresh; never worded here from stored JSON.
pub const VETO_NOTE: &str =
    "Some recorded facts disagree between these charts — Review shows which.";
/// "Different people" (or Review) after the pair was already judged or resolved.
pub const NOTHING_OPEN: &str = "this possible duplicate has already been judged or resolved — \
     nothing was done; reload the chart";
/// Review of a chart no list showed and no open proposal joins to this record (any more).
pub const NOT_SHOWN_OR_RESOLVED: &str = "that chart is not in a list on screen, and is not an \
     open possible duplicate of this record — reload the chart, or search again";
/// The button's label; `key_locked_for` names it (a locked key names the button pressed).
pub const DIFFERENT_PEOPLE_BUTTON: &str = "Different people — not the same person";

/// The banner, as `med_list` hands it to the webview inside `ChartPane`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct DuplicateSection {
    pub entries: Vec<DuplicateEntryView>,
    /// "N more …" when entries beyond [`MAX_SHOWN`] exist.
    pub more: Option<String>,
    /// The proposals could not be read at all.
    pub error: Option<String>,
    /// This record's check lines (pending charts, the node's status when it explains them).
    pub check_lines: Vec<String>,
}

/// One possible duplicate, ready to draw.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DuplicateEntryView {
    /// The chart Review compares against.
    pub review_chart: String,
    pub heading: String,
    /// One line per chart of the other record (`member_line`'s text).
    pub identity_lines: Vec<String>,
    /// Unread identity, veto note.
    pub notes: Vec<String>,
    pub medications_heading: String,
    /// The other record's CURRENT lines (the compare panel's own wording, `medication_lines`).
    pub medications: Vec<String>,
    /// Its list's notes, or why it could not be read.
    pub medication_notes: Vec<String>,
}

/// One member chart's "has the check run since its identity changed?" answer.
pub struct ChartCheck {
    pub chart: Uuid,
    pub pending: Result<bool, String>,
}

/// One pair of a "Different people" judgement, as the window saw its outcome.
pub struct PairResult {
    pub low: Uuid,
    pub high: Uuid,
    pub outcome: Result<LinkEffect, ErrorView>,
}

pub fn entry_view(
    review_chart: Uuid,
    vetoed: bool,
    identities: Result<Vec<MemberLine>, String>,
    meds: Result<MedListView, String>,
) -> DuplicateEntryView {
    let mut notes = vec![];
    let identity_lines = match identities {
        Ok(lines) if !lines.is_empty() => lines.into_iter().map(|l| l.text).collect(),
        Ok(_) => vec![format!("chart {review_chart}")],
        Err(e) => {
            notes.push(format!("The other chart's name and date of birth could not be read: {e}"));
            vec![format!("chart {review_chart}")]
        }
    };
    if vetoed {
        notes.push(VETO_NOTE.into());
    }
    let (medications, medication_notes) = match meds {
        Ok(list) => medication_lines(&list),
        Err(e) => (vec![], vec![format!("Its medications could not be read here: {e}")]),
    };
    DuplicateEntryView {
        review_chart: review_chart.to_string(),
        heading: HEADING.into(),
        identity_lines,
        notes,
        medications_heading: OTHER_CHART_LABEL.into(),
        medications,
        medication_notes,
    }
}

/// `entries` is `(the shown entries, how many exist)` or the read's error.
pub fn section_view(
    entries: Result<(Vec<DuplicateEntryView>, usize), String>,
    check_lines: Vec<String>,
) -> DuplicateSection {
    match entries {
        Err(e) => DuplicateSection {
            entries: vec![],
            more: None,
            error: Some(format!("Could not check for possible duplicates: {e}")),
            check_lines,
        },
        Ok((shown, total)) => {
            let hidden = total.saturating_sub(shown.len());
            let more = match hidden {
                0 => None,
                1 => Some("1 more possible duplicate of this record is not shown here.".into()),
                n => Some(format!("{n} more possible duplicates of this record are not shown here.")),
            };
            DuplicateSection { entries: shown, more, error: None, check_lines }
        }
    }
}

/// The record's check lines. A checked record on a node that is not stalled has none.
///
/// R4's `chart_line` says "This chart: …" — ambiguous on a linked record — so only the OPENED
/// chart uses it; every other member is named. The node's status line follows whenever a member
/// line was shown (it says why "not yet run" is not moving) or the node is stalled; an
/// unreadable status is always shown.
pub fn check_lines(
    opened: Uuid,
    checks: &[ChartCheck],
    members: &[MemberLine],
    status: Result<CheckState, String>,
) -> Vec<String> {
    let label = |chart: Uuid| -> String {
        if chart == opened {
            return "This chart".into();
        }
        let id = chart.to_string();
        match members.iter().find(|m| m.patient_id == id) {
            Some(m) => format!("Linked chart {} (chart {chart})", m.name),
            None => format!("Linked chart {chart}"),
        }
    };
    let mut lines = vec![];
    for c in checks {
        match &c.pending {
            Ok(false) => {}
            Ok(true) if c.chart == opened => lines.push(chart_line(true).to_string()),
            Ok(true) => lines.push(format!(
                "{}: duplicate check not yet run since its identity details last changed.",
                label(c.chart)
            )),
            Err(e) => lines.push(format!("{}: duplicate check status unknown — {e}", label(c.chart))),
        }
    }
    match status {
        Err(e) => lines.push(format!("Duplicate check status unknown: {e}")),
        Ok(state) => {
            if !lines.is_empty() || matches!(state, CheckState::Stalled { .. }) {
                lines.push(status_line(&state));
            }
        }
    }
    lines
}

/// Fixture mode has no proposals and no worker: say so, honestly.
pub fn fixture_section() -> DuplicateSection {
    DuplicateSection {
        check_lines: vec![status_line(&CheckState::NeverRun { waiting: 0 })],
        ..DuplicateSection::default()
    }
}

/// The outcome line for "Different people". Nothing recorded → the first refusal itself (so
/// the webview applies its retry advice: a locked key keeps the button, a verdict takes it).
/// Anything recorded → one sentence per pair, and `reload` (the banner must re-read).
pub fn different_people_report(results: Vec<PairResult>) -> Result<LinkReportView, ErrorView> {
    if results.iter().all(|r| r.outcome.is_err()) {
        return Err(results
            .into_iter()
            .find_map(|r| r.outcome.err())
            .unwrap_or_else(|| refused(NOTHING_OPEN)));
    }
    let sentence = results.iter().map(pair_sentence).collect::<Vec<_>>().join(" ");
    Ok(LinkReportView { sentence, reload: true })
}

fn pair_sentence(r: &PairResult) -> String {
    let (low, high) = (r.low, r.high);
    match &r.outcome {
        Ok(LinkEffect::TookEffect) => format!(
            "Recorded: charts {low} and {high} are different people. This possible duplicate is \
             closed on this node, and on every node once the judgement syncs."
        ),
        Ok(LinkEffect::StillJoined) => format!(
            "Recorded that charts {low} and {high} are different people — but they read as one \
             record through other links, so nothing was split. The links joining them are listed \
             under \"How these charts are linked\"."
        ),
        Ok(LinkEffect::Outranked) => format!(
            "Recorded that charts {low} and {high} are different people, but a judgement already \
             standing for that pair outranks it; the charts stay as that judgement left them."
        ),
        Err(e) => format!(
            "NOT recorded for charts {low} and {high}: {} — it stays on the banner.",
            e.text
        ),
    }
}

#[cfg(test)]
#[path = "view_tests.rs"]
mod tests;
```

  `duplicates/mod.rs` for now:

```rust
//! The possible-duplicate banner (repair path R5a, #680). See `view.rs` for every sentence.
pub mod view;
```

  Add `mod duplicates;` to `main.rs` between `mod commands;` and `mod funnel;`.

- [ ] **Step 4: Run the tests and confirm they pass.**
  - Run `cd cairn-gui && CAIRN_ALLOW_DB_SKIP=1 cargo test -p cairn-gui-tauri duplicates` (PASS).
  - Dead-code warnings for items Task 5 uses are expected. Do not silence them with `allow`;
    Task 5 removes them.

- [ ] **Step 5: Commit.**

```bash
git add cairn-gui/cairn-gui-tauri/src/duplicates cairn-gui/cairn-gui-tauri/src/main.rs
git commit -m "feat(R5a): every banner sentence, pure and golden-tested (Refs #680)"
```

---

### Task 5: the window's wiring — the section in `med_list`, Review's admission, the command

**Files:**
- Modify: `cairn-gui/cairn-gui-tauri/src/duplicates/mod.rs`
- Modify: `cairn-gui/cairn-gui-tauri/src/chart_set.rs` (`ChartPane`, `chart_pane`, its 3 tests)
- Modify: `cairn-gui/cairn-gui-tauri/src/commands.rs` (`med_list_impl`; the drift-guard
  `ChartPane` literal)
- Modify: `cairn-gui/cairn-gui-tauri/src/link/mod.rs` (`resolve_pair`'s `shown` lookup)
- Modify: `cairn-gui/cairn-gui-tauri/src/main.rs` (register `duplicates::record_different_people`)

**Interfaces:**
- Consumes:
  - Task 4's view;
  - `cairn_node::duplicate_review::{possible_duplicates, open_pairs_between, record_different_people, DifferentPeople}`;
  - `cairn_node::duplicate_check::{chart_check_pending, read_snapshot, classify, STALLED_AFTER_SECS}`;
  - `cairn_node::patient::person::chart_identities`;
  - `crate::chart_set::{member_line, check_displayed_set, CHANGED}`;
  - `crate::link::{chart_set_of}`;
  - `crate::link::view::{refused, key_locked_for, OTHER_CHANGED, THIS_CHANGED, NOT_ON_SCREEN}`;
  - `crate::link::unlink_view::unlink_error_view`;
  - `crate::commands::read_chart_of`.
- Produces:
  - `pub async fn duplicate_section(state: &AppState, opened: Uuid, charts: &ChartSet, members: &[MemberLine]) -> DuplicateSection`
  - `pub(crate) async fn admit_other(state: &AppState, left: &ChartSet, other: Uuid) -> Result<String, ErrorView>`
  - `pub async fn different_people_impl(state: &AppState, patient_id: &str, charts: Vec<String>, other_id: &str, other_charts: Vec<String>) -> Result<LinkReportView, ErrorView>`
  - `#[tauri::command] pub async fn record_different_people(...)` (camelCase keys `patientId, charts, otherId, otherCharts`)
  - `ChartPane::duplicates: DuplicateSection`.

- [ ] **Step 1: Write the failing tests** in `duplicates/mod.rs`'s test module (mock `AppState`,
  the pattern of `link/mod.rs`'s tests):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use cairn_patient_search::{Candidate, TrustState};

    fn fixture() -> Uuid {
        cairn_gui_data::mock::fixtures::FIXTURE_UUID.parse().unwrap()
    }

    #[tokio::test]
    async fn fixture_mode_shows_no_entries_and_says_no_check_ran() {
        let state = AppState::mock(Some(fixture()));
        let s = duplicate_section(&state, fixture(), &ChartSet::single(fixture()), &[]).await;
        assert_eq!(s, view::fixture_section());
    }

    #[tokio::test]
    async fn a_shown_chart_is_admitted_by_its_list() {
        let state = AppState::mock(Some(fixture()));
        let other = Uuid::from_u128(2);
        state.shown.lock().await.insert(
            other,
            Candidate {
                patient_id: other,
                display_name: "Other Person".into(),
                age: None,
                trust: TrustState::Confirmed,
                last_activity: None,
                locale: None,
                photo_ref: None,
            },
        );
        let name = admit_other(&state, &ChartSet::single(fixture()), other).await.unwrap();
        assert_eq!(name, "Other Person");
    }

    /// Fixture mode has no proposals, so an unshown chart keeps today's refusal word for word.
    #[tokio::test]
    async fn in_fixture_mode_an_unshown_chart_keeps_the_not_on_screen_refusal() {
        let state = AppState::mock(Some(fixture()));
        let err = admit_other(&state, &ChartSet::single(fixture()), Uuid::from_u128(2))
            .await
            .unwrap_err();
        assert_eq!(err.text, crate::link::view::NOT_ON_SCREEN);
    }

    #[tokio::test]
    async fn different_people_is_bound_to_the_chart_on_screen() {
        let state = AppState::mock(Some(fixture()));
        let err = different_people_impl(&state, &Uuid::from_u128(9).to_string(), vec![], "x", vec![])
            .await
            .unwrap_err();
        assert!(err.text.contains("not the chart"), "{}", err.text);
    }

    #[tokio::test]
    async fn different_people_refuses_a_changed_set() {
        let state = AppState::mock(Some(fixture()));
        let p = fixture().to_string();
        let err = different_people_impl(
            &state,
            &p,
            vec![p.clone(), Uuid::from_u128(7).to_string()],
            &Uuid::from_u128(2).to_string(),
            vec![],
        )
        .await
        .unwrap_err();
        assert_eq!(err.text, crate::link::view::THIS_CHANGED);
    }

    #[tokio::test]
    async fn fixture_mode_cannot_record_different_people() {
        let state = AppState::mock(Some(fixture()));
        let p = fixture().to_string();
        let err = different_people_impl(&state, &p, vec![p.clone()], &Uuid::from_u128(2).to_string(), vec![])
            .await
            .unwrap_err();
        assert!(err.text.contains("fixture mode"), "{}", err.text);
    }
}
```

- [ ] **Step 2: Run the tests and confirm they fail.**
  - Run `cd cairn-gui && CAIRN_ALLOW_DB_SKIP=1 cargo test -p cairn-gui-tauri duplicates::tests`.
  - Expected: a compile error.

- [ ] **Step 3: Implement `duplicates/mod.rs`** (above its tests):

```rust
//! The possible-duplicate banner (repair path R5a, #680; design page "R5a — the banner,
//! designed 2026-10-06"): the section `med_list` carries, Review's admission, and the
//! "Different people" command. Every sentence is in `view.rs`; every DB rule is in
//! `cairn_node::duplicate_review` (DB-tested there). This module only orders the reads and
//! applies the chart-command rules.
//!
//! LOCKING: `state.db` is a `tokio::sync::Mutex`, which is NOT re-entrant, and
//! `read_chart_of` / `chart_set_of` take it themselves. So [`duplicate_section`] reads in two
//! phases — everything else under one lock, then each entry's medications after releasing it —
//! and nothing here calls either helper while holding the lock (that would deadlock the window).
pub mod view;

use crate::chart_set::{check_displayed_set, member_line, MemberLine, CHANGED};
use crate::commands::read_chart_of;
use crate::funnel::view::{ErrorView, Retry};
use crate::link::chart_set_of;
use crate::link::unlink_view::unlink_error_view;
use crate::link::view::{
    key_locked_for, refused, LinkReportView, NOT_ON_SCREEN, OTHER_CHANGED, THIS_CHANGED,
};
use crate::state::{AppState, Now};
use cairn_medication_view::ChartSet;
use cairn_node::db_diagnosis::operator_chain;
use cairn_node::duplicate_check::{chart_check_pending, classify, read_snapshot, STALLED_AFTER_SECS};
use cairn_node::duplicate_review::{self, DifferentPeople};
use uuid::Uuid;
use view::{
    check_lines, entry_view, fixture_section, section_view, ChartCheck, DuplicateSection,
    PairResult, DIFFERENT_PEOPLE_BUTTON, MAX_SHOWN, NOTHING_OPEN, NOT_SHOWN_OR_RESOLVED,
};

/// The banner for the displayed record. Never fails: every failure is a worded line (an absent
/// banner must mean "checked, none open"). `members` are the record's member lines, already
/// read for the header — used only to name a pending member.
pub async fn duplicate_section(
    state: &AppState,
    opened: Uuid,
    charts: &ChartSet,
    members: &[MemberLine],
) -> DuplicateSection {
    let Some(db) = state.db.as_ref() else {
        return fixture_section();
    };
    // Phase 1, under ONE lock: the proposals, each shown entry's identities, the checks.
    let (found, identities, checks, status) = {
        let db = db.lock().await;
        let found = duplicate_review::possible_duplicates(&*db, charts)
            .await
            .map_err(|e| operator_chain(&e));
        let mut identities = vec![];
        if let Ok(entries) = &found {
            for entry in entries.iter().take(MAX_SHOWN) {
                identities.push(
                    cairn_node::patient::person::chart_identities(&*db, &entry.other_record)
                        .await
                        .map(|ids| ids.iter().map(member_line).collect::<Vec<_>>())
                        .map_err(|e| operator_chain(&e)),
                );
            }
        }
        let mut checks = vec![];
        for chart in charts.members() {
            checks.push(ChartCheck {
                chart: *chart,
                pending: chart_check_pending(&db, *chart).await.map_err(|e| operator_chain(&e)),
            });
        }
        let status = read_snapshot(&db)
            .await
            .map(|s| classify(&s, STALLED_AFTER_SECS))
            .map_err(|e| operator_chain(&e));
        (found, identities, checks, status)
    }; // lock released here: `read_chart_of` below takes it again.
    // Phase 2: each shown entry's medications through the SAME read opening that chart gives
    // (§5.9 custody and sealing unchanged).
    let entries = match found {
        Err(e) => Err(e),
        Ok(found) => {
            let total = found.len();
            let mut shown = vec![];
            for (entry, ids) in found.iter().take(MAX_SHOWN).zip(identities) {
                let meds = read_chart_of(state, entry.review_chart)
                    .await
                    .map(|list| cairn_gui_tab_medications::view::build_view(&list));
                shown.push(entry_view(entry.review_chart, entry.vetoed, ids, meds));
            }
            Ok((shown, total))
        }
    };
    section_view(entries, check_lines(opened, &checks, members, status))
}

/// Review's admission for the compare panel (`link::resolve_pair`): a chart a list on screen
/// showed (`AppState::shown`, unchanged), OR one an open proposal joins to `left`'s record at
/// this moment — the banner showed it. `shown` is deliberately NOT widened (design "R5a"): a
/// pair a colleague resolved a second ago is refused here, never silently compared. Returns the
/// name the list showed ("" for a banner admission — only fixture mode reads it).
pub(crate) async fn admit_other(
    state: &AppState,
    left: &ChartSet,
    other: Uuid,
) -> Result<String, ErrorView> {
    let shown = state.shown.lock().await.get(&other).map(|c| c.display_name.clone());
    if let Some(name) = shown {
        return Ok(name);
    }
    let Some(db) = state.db.as_ref() else {
        return Err(refused(NOT_ON_SCREEN)); // fixture mode has no proposals
    };
    let right = chart_set_of(state, other).await?; // takes the lock itself — not held here
    let db = db.lock().await;
    let pairs = duplicate_review::open_pairs_between(&*db, left, &right)
        .await
        .map_err(|e| ErrorView {
            text: format!(
                "Could not read whether that chart is an open possible duplicate of this record \
                 — nothing was done: {}",
                operator_chain(&e)
            ),
            retry: Retry::Now,
        })?;
    if pairs.is_empty() {
        Err(refused(NOT_SHOWN_OR_RESOLVED))
    } else {
        Ok(String::new())
    }
}

/// "Different people — not the same person" (R5a; offered only on a banner's comparison).
///
/// The rules, IN THIS ORDER (each pinned by a test, mirroring `link::link_impl`): the chart on
/// screen; this record's set is the one compared (`THIS_CHANGED`); fixture mode; the other
/// record's set is the one compared (`OTHER_CHANGED`); the key. Then the node judges every pair
/// still open between the two records, read fresh — `NothingOpen` means a colleague got there
/// first and nothing was signed.
pub async fn different_people_impl(
    state: &AppState,
    patient_id: &str,
    charts: Vec<String>,
    other_id: &str,
    other_charts: Vec<String>,
) -> Result<LinkReportView, ErrorView> {
    let patient = state.displayed_patient(patient_id).await.map_err(refused)?;
    let left = check_displayed_set(&chart_set_of(state, patient).await?, &charts).map_err(|e| {
        if e == CHANGED { refused(THIS_CHANGED) } else { refused(e) }
    })?;
    if state.is_mock() {
        return Err(refused("fixture mode: this window is showing mock data and cannot write"));
    }
    let other: Uuid = other_id.parse().map_err(|_| refused(NOTHING_OPEN))?;
    let right = check_displayed_set(&chart_set_of(state, other).await?, &other_charts).map_err(
        |e| if e == CHANGED { refused(OTHER_CHANGED) } else { refused(e) },
    )?;
    let (human_sk, human_kid) = state
        .live_key(Now::read())
        .await
        .ok_or_else(|| key_locked_for(DIFFERENT_PEOPLE_BUTTON))?;
    let mut db = state
        .db
        .as_ref()
        .ok_or_else(|| refused("no database connection"))?
        .lock()
        .await;
    let reviewer = cairn_node::chart_link::Reviewer { human_sk: &human_sk, human_kid: &human_kid };
    let outcome = duplicate_review::record_different_people(
        &mut db,
        &left,
        &right,
        &reviewer,
        &state.node_origin,
    )
    .await
    .map_err(|e| ErrorView {
        text: format!(
            "Could not read whether this possible duplicate is still open — nothing was done: {}",
            operator_chain(&e)
        ),
        retry: Retry::Now,
    })?;
    match outcome {
        DifferentPeople::NothingOpen => Err(refused(NOTHING_OPEN)),
        DifferentPeople::Judged(judged) => view::different_people_report(
            judged
                .into_iter()
                .map(|j| PairResult {
                    low: j.low,
                    high: j.high,
                    outcome: j.outcome.map(|o| o.effect).map_err(|e| unlink_error_view(&e)),
                })
                .collect(),
        ),
    }
}

#[tauri::command]
pub async fn record_different_people(
    state: tauri::State<'_, AppState>,
    patient_id: String,
    charts: Vec<String>,
    other_id: String,
    other_charts: Vec<String>,
) -> Result<LinkReportView, ErrorView> {
    different_people_impl(&state, &patient_id, charts, &other_id, other_charts).await
}
```

  Implementer checks, before compiling:
  - **Lock types.** `state.db` is the type `link::compare_impl` locks. `chart_check_pending` /
    `read_snapshot` take `&tokio_postgres::Client`, so pass `&db` (deref coercion from the guard)
    or `&*db`. `record_different_people` takes `&mut Client`, so pass `&mut db` (a
    `MutexGuard<Client>` derefs mutably).
  - **`LinkOutcome::effect`** is a `Copy` field (`LinkEffect` derives `Copy`).
  - **`unlink_view`'s visibility.** `link/unlink_view.rs` is `pub mod unlink_view;` and
    `unlink_error_view` is `pub`; keep both as they are.

- [ ] **Step 4: Wire it in.**
  - `chart_set.rs`:
    - add `pub duplicates: crate::duplicates::view::DuplicateSection,` to `ChartPane`, with doc
      `/// The possible-duplicate banner (R5a); empty and hidden when checked and none open.`;
    - add a fourth parameter `duplicates: crate::duplicates::view::DuplicateSection` to
      `chart_pane`, and set the field;
    - pass `crate::duplicates::view::DuplicateSection::default()` in its 3 test calls.
  - `commands.rs` `med_list_impl`, replacing the last line:

```rust
    // The possible-duplicate banner (R5a). Never fails the open either: every failure inside
    // it is a worded line.
    let duplicates = crate::duplicates::duplicate_section(
        state,
        patient,
        &list.charts,
        members.as_deref().unwrap_or(&[]),
    )
    .await;
    Ok(chart_pane(&list, members, edges, duplicates))
```

  - In `commands.rs`'s drift-guard test, add `duplicates: Default::default(),` to the
    `ChartPane { … }` literal.
  - `link/mod.rs` `resolve_pair`: replace the `shown_name` block (the `state.shown.lock()…
    .ok_or_else(|| refused(NOT_ON_SCREEN))?;` statement) with:

```rust
    // A list on screen, or — R5a — an open proposal the banner showed (`duplicates::admit_other`).
    let shown_name = crate::duplicates::admit_other(state, &left, other).await?;
```

    and remove `NOT_ON_SCREEN` from the `use view::{…}` list if it is now unused there (the
    tests still use `view::NOT_ON_SCREEN` by path).
  - `main.rs`: add `duplicates::record_different_people,` after `link::search::link_search,`.

- [ ] **Step 5: Run the tests and confirm they pass.**
  - Run `cd cairn-gui && CAIRN_ALLOW_DB_SKIP=1 cargo test -p cairn-gui-tauri`. The whole crate
    must PASS, including the existing `link::tests::compare_refuses_a_chart_no_list_showed`
    (unchanged text in fixture mode).
  - Run `cd cairn-gui && cargo clippy --locked --all-targets -- -D warnings` (clean; no dead
    code left from Task 4).

- [ ] **Step 6: Commit.**

```bash
git add cairn-gui/cairn-gui-tauri/src
git commit -m "feat(R5a): the banner in med_list, Review's admission by open proposal, and 'Different people' (Refs #680)"
```

---

### Task 6: the webview — banner, Review, *Different people*

**Files:**
- Create: `cairn-gui/cairn-gui-tauri/src-ui/duplicates.js`
- Modify: `cairn-gui/cairn-gui-tauri/src-ui/index.html`, `src-ui/link.js`, `src-ui/main.js`
- Modify: `cairn-gui/cairn-gui-tauri/src/duplicates/mod.rs` (the JS drift guard test)

**Interfaces:**
- Consumes:
  - `pane.duplicates` (`DuplicateSection`) and `DuplicateEntryView` (Task 4);
  - the command `record_different_people` (Task 5);
  - from link.js: `openLinkPanel`, `compare`, `compared`, `compareToken`, `linkAnswerPlace`,
    `sayAnywhere`, `linkChanged`, `closeLinkPanel`;
  - from main.js: `el`, `cell`, `setMessage`, `say`, `refresh`;
  - `failureText` (funnel.js).
- Produces:
  - `renderDuplicates(section)`, `clearDuplicates()` (duplicates.js);
  - `sendJudgement(command, button)` (link.js).

- [ ] **Step 1: Write the failing drift guard** (append to `duplicates/mod.rs`'s tests). It is
  the same idiom as `funnel_js_reads_no_field_the_backend_does_not_send`:

```rust
    /// duplicates.js is untyped: a Rust field rename would not break the build, it would draw
    /// an empty banner. Every field it reads must be one the backend sends.
    #[test]
    fn duplicates_js_reads_no_field_the_backend_does_not_send() {
        use crate::commands::tests::fields_read_in;
        let js = include_str!("../../src-ui/duplicates.js");
        let entry = view::entry_view(Uuid::from_u128(9), true, Ok(vec![]), Err("x".into()));
        let keys = |v: serde_json::Value| -> std::collections::BTreeSet<String> {
            v.as_object().unwrap().keys().cloned().collect()
        };
        for (binding, available) in [
            ("section", keys(serde_json::to_value(DuplicateSection::default()).unwrap())),
            ("entry", keys(serde_json::to_value(&entry).unwrap())),
        ] {
            let read = fields_read_in(js, binding);
            assert!(!read.is_empty(), "duplicates.js no longer reads `{binding}` — rename it here");
            for field in read {
                assert!(available.contains(&field), "duplicates.js reads `{binding}.{field}`, not sent");
            }
        }
    }
```

  Also extend `commands.rs`'s main.js guard so a `pane.duplicates` read is checked. `ChartPane`
  already serialises the field after Task 5, so no change is needed there. Confirm by running it.

- [ ] **Step 2: Run the guard and confirm it fails.**
  - Run `cd cairn-gui && CAIRN_ALLOW_DB_SKIP=1 cargo test -p cairn-gui-tauri duplicates_js`.
  - Expected: a compile error (`duplicates.js` is missing for `include_str!`).

- [ ] **Step 3: index.html.** Insert directly after the closing `</section>` of `#session` and
  before `#chart-warnings`:

```html
        <!-- The possible-duplicate banner (R5a, #680). AMBIENT, never a modal and never
             role="alert": it takes no focus, makes no sound and is never re-popped (§5.12). It sits
             ABOVE the list it qualifies (DOM order is clinical). Hidden only when this record is
             checked and nothing is open — an absent banner never means "unknown". The other
             chart's lines are someone else's until a human links the two: they are drawn here,
             never in #med-table, and can never be signed or ceased. -->
        <section id="possible-duplicates" aria-labelledby="duplicates-heading" hidden>
          <h2 id="duplicates-heading">Possible duplicate records</h2>
          <p id="duplicates-error" hidden></p>
          <ul id="duplicates-entries"></ul>
          <p id="duplicates-more" hidden></p>
          <ul id="duplicates-checks" aria-label="Duplicate check for this record"></ul>
        </section>
```

  In `#link-panel`, directly after the `link-confirm` button:

```html
          <!-- Offered only on a comparison opened from the banner's Review (R5a). -->
          <button id="link-different" type="button" hidden>Different people — not the same person</button>
```

  After `<script src="unlink.js"></script>`: `<script src="duplicates.js"></script>`.

- [ ] **Step 4: link.js.** Three small edits.
  - **(a)** In `clearComparison`'s id list, add `"link-different"`, so a search-driven
    comparison never shows it.
  - **(b)** In `updateLinkLock`, after the unlink label:

```js
  // "Different people" (R5a) obeys the same ambient lock rule; duplicates.js loads after this.
  el("link-different").textContent = keyUnlocked
    ? "Different people — not the same person"
    : "Different people — not the same person (unlock your signing key first)";
```

  - **(c)** Generalise `linkCompared` without changing its behaviour:
    - rename its body to `async function sendJudgement(command, button)`;
    - replace `invoke("link_records", …)` with `invoke(command, …)`, with the same payload keys;
    - remove the line `const button = el("link-confirm");`, since the parameter replaces it;
    - add `linkCompared`:

```js
/** Send the Link judgement for whatever `compare` last rendered. */
async function linkCompared() {
  return sendJudgement("link_records", el("link-confirm"));
}
```

    Update the moved function's doc comment to read "Send a judgement (`link_records` or R5a's
    `record_different_people`) over whatever `compare` last rendered; `button` is the one
    pressed, disabled for the round trip and hidden on a verdict."

- [ ] **Step 5: duplicates.js.**

```js
// The possible-duplicate banner (repair path R5a, #680). Renders and decides nothing: every
// sentence comes from Rust (`duplicates/view.rs`), every check from the backend
// (`duplicates/mod.rs`). Classic script, loaded after main.js, funnel.js, link.js and unlink.js,
// sharing their scope.
//
// Its one piece of state is which buttons are visible. Review reuses the "Same person as…"
// panel (link.js) pre-filled with the banner's chart; only a comparison opened HERE shows
// "Different people" — `clearComparison` hides it again for any other comparison.
"use strict";

/** Draw `pane.duplicates`. Hidden only when there is truly nothing to say. */
function renderDuplicates(section) {
  setMessage(el("duplicates-error"), section.error || "");
  el("duplicates-entries").replaceChildren(...section.entries.map(duplicateItem));
  setMessage(el("duplicates-more"), section.more || "");
  el("duplicates-checks").replaceChildren(...section.check_lines.map((t) => cell("li", t)));
  el("possible-duplicates").hidden =
    !section.error && section.entries.length === 0 && section.check_lines.length === 0 && !section.more;
}

/** Empty the banner (a chart change; never one patient's banner under another's header). */
function clearDuplicates() {
  renderDuplicates({ entries: [], more: null, error: null, check_lines: [] });
}

/** One possible duplicate: who, why to look, their current drugs, and Review. */
function duplicateItem(entry) {
  const li = document.createElement("li");
  li.append(cell("h3", entry.heading));
  for (const line of entry.identity_lines) li.append(cell("p", line));
  for (const note of entry.notes) li.append(cell("p", note));
  li.append(cell("h4", entry.medications_heading));
  const meds = document.createElement("ul");
  meds.append(...entry.medications.map((t) => cell("li", t)));
  meds.append(...entry.medication_notes.map((t) => cell("li", t)));
  li.append(meds);
  const review = document.createElement("button");
  review.type = "button";
  review.textContent = "Review";
  review.addEventListener("click", () => reviewDuplicate(entry.review_chart));
  li.append(review);
  return li;
}

/**
 * Open the comparison on the banner's chart. The backend admits it only while an open proposal
 * still joins it to this record (`duplicates::admit_other`). "Different people" is shown only
 * when THIS comparison is the one now on the panel (a newer click, or a close, wins).
 */
async function reviewDuplicate(otherId) {
  openLinkPanel();
  await compare(otherId);
  el("link-different").hidden = !(compared !== null && compared.otherId === otherId);
}

el("link-different").addEventListener("click", () =>
  sendJudgement("record_different_people", el("link-different")),
);
```

- [ ] **Step 6: main.js.**
  - In `render`, after `setMessage(el("record-links-error"), pane.links_error);`, add
    `renderDuplicates(pane.duplicates);`.
  - In `clearChart`, after the `record-links-error` line:

```js
  // duplicates.js loads after this file; guarded like link.js's call into unlink.js.
  if (typeof clearDuplicates === "function") clearDuplicates();
```

- [ ] **Step 7: Run the guards and the crate; confirm they pass.**
  - Run `cd cairn-gui && CAIRN_ALLOW_DB_SKIP=1 cargo test -p cairn-gui-tauri`. All PASS,
    including both JS drift guards.

- [ ] **Step 8: Headless walk** (the memory recipe, "Webview mock walk recipe"; no committed rig).
  1. Copy `src-ui` to the scratchpad.
  2. Stub `window.__TAURI__.core.invoke` with Rust-shaped payloads before `main.js` loads:
     - a `med_list` pane whose `duplicates` has one entry (two medication lines) and one check
       line;
     - `compare_records` returning a `ComparisonView` with `can_link: true`;
     - `record_different_people` returning `{sentence: "Recorded: …", reload: true}`.
  3. Serve the copy and drive it with Playwright.
  4. Assert **visibility** (`getComputedStyle(el).display !== "none"` and no `hidden` ancestor),
     never just `textContent`:
     1. the banner is visible above `#chart-warnings`;
     2. it has no `role="alert"`;
     3. Review opens `#link-panel` with `#link-different` visible;
     4. a search-driven Compare (stub `link_search`) leaves `#link-different` hidden;
     5. clicking *Different people* puts the sentence on `#outcome` and re-reads (`med_list`
        called again).
  5. Record the walk's result in the as-built note (Task 7). Nothing from the walk is committed.

- [ ] **Step 9: Commit.**

```bash
git add cairn-gui/cairn-gui-tauri/src-ui cairn-gui/cairn-gui-tauri/src/duplicates/mod.rs
git commit -m "feat(R5a): the banner in the window — Review pre-fills the comparison; 'Different people' beside Link (Refs #680)"
```

---

### Task 7: docs, runbook, follow-ons, full gates, PR

**Files:** design page (as-built note under "R5a"), `cairn-gui/cairn-gui-tauri/results/RUNBOOK.md`
(§11), `docs/HANDOVER.md`, `docs/ROADMAP.md`.

- [ ] **Step 1: RUNBOOK §11 — "Review a possible duplicate from the banner (R5a, #680)."**
  It is live only (fixture mode has no proposals). Setup:
  1. Register two near-identical charts with `$NODE patient-register` (the same name and DOB,
     as §10's `reg`).
  2. Run the worker once:
     `uv run --project matcher --extra pipeline cairn-matcher watch --once --dsn "$CONN"`.
  3. Open chart A in the window.

  Stopwatch: start at the press of **Review**. Stop when the outcome line reads "Recorded: …" or
  "Linked — …". Budget ≤ 20 s.

  Also record chart-open time with the banner present against the single-chart open (budget ≤
  the single-chart open). State plainly that a miss is a finding to file, never a budget to
  adjust.

- [ ] **Step 2: The design page's as-built note** ("R5a — as built (2026-10-06)").
  - Record every deviation from the plan.
  - Record the headless walk's result.
  - Record the measurement debt (runbook §11, owed by a human).

- [ ] **Step 3: Follow-ons.**
  - Comment on #722: "the mock has no proposal fixture either (R5a)".
  - Comment on #680: "R5a built on PR #NNN; R5b (worklist) next. Its view is
    `match_proposal_open`; the attested-unlink correction is recorded in the design page."
  - File, rather than fix, anything a review finds out of scope (rule 5).

- [ ] **Step 4: HANDOVER + ROADMAP.**
  - ⇒ NEXT becomes R5b.
  - Add R5a's durable rules:
    - the attested-only filter;
    - `shown` not widened;
    - the two-phase lock;
    - every failure is a line;
    - "Different people" only from the banner.
  - Add `SCHEMA_GENERATION` 57 and the §1.2 line.
  - Keep both files under ~500 lines.

- [ ] **Step 5: Full gates, in CI's order, AFTER the last edit** (trap 18; memory "a gate is
  evidence only for the tree it ran against"):
  - `cargo fmt --all -- --check`, plus the same in `cairn-gui/`;
  - `cargo clippy --workspace --all-targets -- -D warnings`;
  - `cd cairn-gui && cargo clippy --locked --all-targets -- -D warnings`;
  - `RUSTDOCFLAGS=-D warnings cargo doc --workspace --no-deps`, plus the same in `cairn-gui/`;
  - `scripts/run-db-gated-tests.sh` in the background. Read the log's last line. It takes about
    2 hours; do the docs while it runs;
  - `cd cairn-gui && CAIRN_TEST_PG=… cargo test` (the live tree);
  - `cargo test -p cairn-node --test paper_parity_plan_section`.

- [ ] **Step 6: PR.**
  - Run `python3 scripts/check_closing_keywords.py` on each commit message.
  - Push with `git -c credential.helper= -c 'credential.helper=!gh auth git-credential' push -u
    origin feat/r5-banner-worklist`.
  - Open the PR "Repair path R5a: the possible-duplicate banner (#680)", with a body that says
    "Refs #680", never a closing word. Draft if any gate is red.

---

## Paper-parity benchmark (§1.2)

- **Paper counterpart:** the records clerk lays the two folders side by side on the desk, then
  clips them together, or writes "different people" on the tray slip. A clinician who opens one
  folder finds the clerk's slip on top: "possible duplicate — see folder B", with B's drug sheet
  attached.
- **Steps:**
  - To resolve: paper 3 human acts (fetch the other folder, lay them side by side, clip or mark)
    → architecture-forced 2 (*Review*, then *Same person* or *Different people*; the signature
    click IS the clip, ADR-0053) → UI target 2. `M ≤ N`.
  - To read: opening a chart with a banner adds 0 acts. The other chart's medications are on
    screen without a click, the decision the maintainer took for ambient safety.
  - An unlock when the key is locked is the existing session act, not one this slice adds.
- **Time + cognitive load:** budget ≤ 20 s from *Review* to a recorded judgement, of which the
  side-by-side read is the load. A chart open with the banner ≤ the single-chart open. Measured by
  RUNBOOK §11, a human act owed by this slice. The cognitive load the banner adds at open is one
  labelled block above the list, never a dialog. A miss is a finding (file an issue), never a
  budget change.
