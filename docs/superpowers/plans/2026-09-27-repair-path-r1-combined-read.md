# Repair path R1 — the combined read (and #334) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking. **Subagent briefs say FOREGROUND ONLY** (a subagent waiting on a background job never wakes).

**Goal:** Opening a chart that is linked to others shows one medication list over every chart in its link component — each row naming its source chart(s) — and every chart command names the chart SET it showed and refuses when that set changed.

**Architecture:** One SQL function (`cairn_person_charts`, db/054) answers "which charts are this person" for every reader. The Rust read selects medication groups by **membership** (any member thread on a chart in the set) instead of by the view's display-winner `patient_id` — which is also what fixes #334 (a group spanning two charts no longer vanishes from the loser's chart). "Cross-patient" becomes "reaches a chart OUTSIDE the set". A set-aware duplicate flag (db/054) keeps the same drug recorded on two linked charts from showing twice unflagged. Sign-off attests each thread under its own chart; the GUI binds commands to the displayed set.

**Tech Stack:** PostgreSQL 18 (PL/pgSQL/SQL, `db/*.sql` replayed on every connect), Rust (`cairn-node`, pure `cairn-medication-view`), Tauri 2 + plain JS (`cairn-gui/`, a separate Cargo workspace with its own lockfile).

**Spec:** `docs/superpowers/specs/2026-09-27-duplicate-repair-path-679-680-681-design.md` (section *R1*) and [ADR-0076](../../spec/decisions/0076-duplicate-repair-a-linked-chart-reads-as-one-and-a-human-judgement-outranks-a-machine.md) decisions 1–3.

## Global Constraints

- AGPL-3.0; **no new dependency** in any crate (none is needed).
- TDD: every production line is driven by a failing test first; DB-gated tests self-skip without `CAIRN_TEST_PG` and follow the file's existing `let Some(base) = cs() else { … return; }; let _guard = db::test_serial_guard(&base)…` preamble.
- `db/*.sql` replays on **every** connect: `CREATE OR REPLACE FUNCTION` only; no view column-set change; a SQL change needs a **rebuild** (it is `include_str!`'d).
- New definer/SQL functions carry `SET search_path = public, pg_temp` (#426).
- UUIDs are bound as text and cast in SQL (`$1::text::uuid`, arrays `$1::text[]::uuid[]`) — `tokio-postgres` has no `uuid` feature here (read.rs's *UUID BINDING* note).
- Collation-independence (ADR-0045): any TEXT compare deciding a winner or a key uses `COLLATE "C"`; sort in Rust, not SQL, where order is shown.
- A single never-linked chart without a cross-patient group must read **exactly** as before (golden test, Task 4).
- House rule 6: no literal key/seed/salt/nonce in tests; `generate_key()` as the suites already do.
- Every new `pub fn` in `crates/cairn-node/tests/common/mod.rs` is ALSO added to the hand-written expected-helper array in `crates/cairn-node/tests/identity_scaffolding_shared.rs` (`derivation_finds_the_expected_helpers`).
- Files stay under ~500 lines where feasible: new behaviour goes in NEW files (`patient/person.rs`, `cairn-medication-view/src/chart_set.rs`, `tests/combined_read.rs`, `tests/person_charts.rs`), never onto `medication_read.rs` (751) or `funnel/commands.rs` (894).
- Gate before each commit: `cargo fmt --check` (both trees), and at the end the full `scripts/run-db-gated-tests.sh` with a scratch `CARGO_TARGET_DIR` (trap 18: a targeted `--test X` green is not proof), `cargo clippy --all-targets -- -D warnings`, `RUSTDOCFLAGS=-D warnings cargo doc --no-deps` in BOTH trees, and the cairn-gui gate.

## Review Focus

1. **A cluster of three or more charts** (A–B, B–C linked; A–C never directly). Expect all three in the set from any member. → Task 2 test `a_transitive_cluster_is_one_set_from_every_member`.
2. **Link, unlink, relink.** Expect the set to follow the standing edges only. → Task 2 test `an_unlinked_chart_reads_alone_again`.
3. **A group whose other patient is outside the set while a third is inside** (A–B linked; a reconciled group spans B and an unlinked C). Expect it flagged cross-patient on the A+B view and withheld from sign-off. → Task 4 test `a_group_reaching_outside_the_set_is_still_a_hazard`.
4. **The same drug recorded on two linked charts.** Expect two lines BOTH flagged "possible duplicate — not yet reconciled" (never silent). → Task 4 test `the_same_drug_on_two_linked_charts_is_flagged`.
5. **A link landing between the list on screen and the sign-off click.** Expect a refusal naming the change, nothing signed. → Task 5 test `a_sign_off_is_refused_when_the_chart_set_changed` and Task 7 `sign_off_refuses_a_changed_set`.

**Stated limit (not fixed here):** the component follows every standing `link`, including an un-attested synced link flagged by `link_veto_flag` (the chart then reads *under-review*). R1 shows each member's trust on its header line so that state is visible; R2's precedence rule narrows which links stand. **Filed, not fixed (Task 8):** reconciling the two duplicate threads of a linked pair is refused by db/033's local cross-patient guard — the write-side mirror of this read.

---

### Task 1: `ChartSet` — the displayed set as a type (pure)

**Files:**
- Create: `crates/cairn-medication-view/src/chart_set.rs`
- Modify: `crates/cairn-medication-view/src/lib.rs` (add `mod chart_set; pub use chart_set::ChartSet;`)

**Interfaces:**
- Produces: `pub struct ChartSet` (private `Vec<Uuid>`, sorted, deduplicated, never empty); `ChartSet::single(Uuid) -> ChartSet`; `ChartSet::new(impl IntoIterator<Item = Uuid>) -> Option<ChartSet>` (`None` iff empty); `fn members(&self) -> &[Uuid]`; `fn contains(&self, &Uuid) -> bool`; `fn contains_all(&self, &[Uuid]) -> bool`; `fn is_linked(&self) -> bool` (more than one chart); `Serialize` as a JSON array of uuid strings; `PartialEq, Eq, Clone, Debug`.

Why a type: the set is compared across an IPC hop and across two reads, and a `Vec<Uuid>` in a different order would compare unequal and refuse a correct sign-off — the constructor is the only place order is decided. Why in this crate: `PatientMedicationList` (here) is its first consumer; the chart model moved here the same way when it gained a second consumer, and R3 moves `ChartSet` to a shared identity crate if the front door needs it.

- [ ] **Step 1: Write the failing tests** (in `chart_set.rs`, `#[cfg(test)] mod tests`)

```rust
use super::*;

fn u(n: u128) -> Uuid {
    Uuid::from_u128(n)
}

#[test]
fn the_order_given_never_changes_the_set() {
    let a = ChartSet::new([u(3), u(1), u(2)]).unwrap();
    let b = ChartSet::new([u(2), u(3), u(1)]).unwrap();
    assert_eq!(a, b, "the same charts in another order are the same set");
    assert_eq!(a.members(), &[u(1), u(2), u(3)]);
}

#[test]
fn a_repeated_chart_counts_once() {
    let s = ChartSet::new([u(1), u(1), u(2)]).unwrap();
    assert_eq!(s.members(), &[u(1), u(2)]);
}

#[test]
fn an_empty_set_cannot_be_made() {
    assert!(ChartSet::new(std::iter::empty()).is_none());
}

#[test]
fn a_single_chart_is_not_linked_and_two_are() {
    assert!(!ChartSet::single(u(1)).is_linked());
    assert!(ChartSet::new([u(1), u(2)]).unwrap().is_linked());
}

#[test]
fn contains_all_is_a_subset_test() {
    let s = ChartSet::new([u(1), u(2)]).unwrap();
    assert!(s.contains_all(&[u(2), u(1)]));
    assert!(!s.contains_all(&[u(1), u(3)]));
    assert!(s.contains_all(&[]), "the empty list is inside every set");
}

#[test]
fn it_serializes_as_a_plain_array_of_ids() {
    let s = ChartSet::new([u(2), u(1)]).unwrap();
    assert_eq!(
        serde_json::to_string(&s).unwrap(),
        format!("[\"{}\",\"{}\"]", u(1), u(2))
    );
}
```

(`serde_json` is already a dev-dependency of this crate.)

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p cairn-medication-view chart_set`
Expected: FAIL — `ChartSet` not found.

- [ ] **Step 3: Implement**

```rust
//! The set of charts a read covers — one chart, or every chart in a link component
//! (ADR-0076 decision 1).
//!
//! A chart command must name the set the clinician SAW and refuse when it changed
//! (decision 3). That comparison crosses an IPC hop and two database reads, so the set's
//! ORDER must never make two equal sets compare unequal: the constructor sorts and
//! deduplicates, and it is the only place a `ChartSet` is built. It is never empty — a
//! read always covers at least the chart that was opened.
use serde::Serialize;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct ChartSet(Vec<Uuid>);

impl ChartSet {
    /// One chart that is linked to nothing — the pre-ADR-0076 case, and still the common one.
    pub fn single(chart: Uuid) -> Self {
        Self(vec![chart])
    }

    /// The set of `charts`, in canonical order. `None` when there are none: a read that
    /// covers no chart at all is a bug in the caller, not a state to render.
    pub fn new(charts: impl IntoIterator<Item = Uuid>) -> Option<Self> {
        let mut v: Vec<Uuid> = charts.into_iter().collect();
        v.sort();
        v.dedup();
        (!v.is_empty()).then_some(Self(v))
    }

    pub fn members(&self) -> &[Uuid] {
        &self.0
    }

    pub fn contains(&self, chart: &Uuid) -> bool {
        self.0.binary_search(chart).is_ok()
    }

    /// Whether every one of `charts` is in this set. The cross-patient test: a medication
    /// group is a hazard exactly when its charts are NOT all inside the set being read.
    pub fn contains_all(&self, charts: &[Uuid]) -> bool {
        charts.iter().all(|c| self.contains(c))
    }

    /// More than one chart: the header shows the linked members.
    pub fn is_linked(&self) -> bool {
        self.0.len() > 1
    }
}
```

- [ ] **Step 4: Run to verify it passes** — `cargo test -p cairn-medication-view chart_set` → PASS.
- [ ] **Step 5: Commit** — `git add crates/cairn-medication-view && git commit -m "feat(R1): ChartSet — the charts a read covers, in one canonical order (ADR-0076)"`

---

### Task 2: `cairn_person_charts` (db/054) and its Rust reader

**Files:**
- Create: `db/054_person_charts.sql`
- Create: `crates/cairn-node/src/patient/person.rs`
- Modify: `crates/cairn-node/src/patient/mod.rs` (`pub mod person;`)
- Modify: `crates/cairn-node/src/db.rs` (append `("054_person_charts", include_str!("../../../db/054_person_charts.sql"))` to the node's FULL list, after `053_substitution_guard`, with a comment: node-only — cairn-sync's list legitimately lags, #284, because no door it loads calls this function)
- Modify: `crates/cairn-event/src/schema_generation.rs` (`SCHEMA_GENERATION: i32 = 54`, and the doc line naming the newest file)
- Modify: `crates/cairn-node/tests/common/mod.rs` (new `pub async fn submit_link_event`), `crates/cairn-node/tests/identity_scaffolding_shared.rs` (add `"submit_link_event"` to the expected-helper array)
- Test: `crates/cairn-node/tests/person_charts.rs`

**Interfaces:**
- Consumes: `ChartSet` (Task 1).
- Produces: SQL `cairn_person_charts(p_patient uuid) RETURNS SETOF uuid`; SQL `cairn_medication_duplicate_groups(p_charts uuid[]) RETURNS SETOF uuid` (used by Task 4); Rust `pub async fn person_charts(client: &(impl tokio_postgres::GenericClient + Sync), patient: Uuid) -> anyhow::Result<ChartSet>`; test helper `pub async fn submit_link_event(c: &Client, sk: &SigningKey, kid: &str, a: Uuid, b: Uuid, wall: i64, is_link: bool)` (panics on refusal — a setup helper).

- [ ] **Step 1: Promote the link helper.** Move `identity_linkage.rs`'s `submit_link_prov` body into `common/mod.rs` as `submit_link_event` (fixed provenance `"test:link"`, `.expect("link event accepted")`), with a doc comment saying it is setup scaffolding for suites that need two charts linked and do not test the link door itself. Add `"submit_link_event"` to `identity_scaffolding_shared.rs`'s expected array. Leave `identity_linkage.rs`'s own helper in place (it tests refusals and needs the `Result`).

- [ ] **Step 2: Write the failing DB tests** (`tests/person_charts.rs`)

```rust
//! ADR-0076 decision 1: `cairn_person_charts` is the ONE answer to "which charts are this
//! person", read by every combined read. A chart never linked is a set of one; a link
//! component is read whole from any member; an unlink splits it again.
mod common;
use cairn_node::db;
use cairn_node::patient::person::person_charts;
use cairn_medication_view::ChartSet;
use common::{cs, medication_setup as setup, submit_link_event, submit_registration};
use uuid::Uuid;

async fn fresh(c: &tokio_postgres::Client, sk: &cairn_event::SigningKey, kid: &str) -> Uuid {
    let p = Uuid::now_v7();
    submit_registration(c, sk, kid, p, 0).await;
    p
}

#[tokio::test]
async fn a_never_linked_chart_is_a_set_of_one() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    c.batch_execute("TRUNCATE patient_link, person_member").await.unwrap();
    let (sk, kid, _, _) = setup(&c).await;
    let a = fresh(&c, &sk, &kid).await;
    assert_eq!(person_charts(&c, a).await.unwrap(), ChartSet::single(a));
}

#[tokio::test]
async fn a_linked_pair_is_one_set_from_either_side() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    c.batch_execute("TRUNCATE patient_link, person_member").await.unwrap();
    let (sk, kid, _, _) = setup(&c).await;
    let a = fresh(&c, &sk, &kid).await;
    let b = fresh(&c, &sk, &kid).await;
    submit_link_event(&c, &sk, &kid, a, b, 10, true).await;
    let both = ChartSet::new([a, b]).unwrap();
    assert_eq!(person_charts(&c, a).await.unwrap(), both);
    assert_eq!(person_charts(&c, b).await.unwrap(), both, "the same set from the other side");
}

#[tokio::test]
async fn a_transitive_cluster_is_one_set_from_every_member() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    c.batch_execute("TRUNCATE patient_link, person_member").await.unwrap();
    let (sk, kid, _, _) = setup(&c).await;
    let a = fresh(&c, &sk, &kid).await;
    let b = fresh(&c, &sk, &kid).await;
    let x = fresh(&c, &sk, &kid).await;
    // a–b and b–x: a and x were never linked to each other, and are still one person.
    submit_link_event(&c, &sk, &kid, a, b, 10, true).await;
    submit_link_event(&c, &sk, &kid, b, x, 11, true).await;
    let all = ChartSet::new([a, b, x]).unwrap();
    for member in [a, b, x] {
        assert_eq!(person_charts(&c, member).await.unwrap(), all, "from {member}");
    }
}

#[tokio::test]
async fn an_unlinked_chart_reads_alone_again() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    c.batch_execute("TRUNCATE patient_link, person_member").await.unwrap();
    let (sk, kid, _, _) = setup(&c).await;
    let a = fresh(&c, &sk, &kid).await;
    let b = fresh(&c, &sk, &kid).await;
    submit_link_event(&c, &sk, &kid, a, b, 10, true).await;
    submit_link_event(&c, &sk, &kid, a, b, 11, false).await;
    assert_eq!(person_charts(&c, a).await.unwrap(), ChartSet::single(a));
    assert_eq!(person_charts(&c, b).await.unwrap(), ChartSet::single(b));
    // Relinked: the set follows the standing edge, not the first one ever written.
    submit_link_event(&c, &sk, &kid, a, b, 12, true).await;
    assert_eq!(person_charts(&c, a).await.unwrap(), ChartSet::new([a, b]).unwrap());
}
```

(If `medication_setup`'s TRUNCATE list does not reach `patient_link`/`person_member`, the explicit `TRUNCATE` line above is what isolates each test; keep it BEFORE `setup`, because `setup` truncates `event_log` with `CASCADE` and the overlay tables are projections of it.)

- [ ] **Step 3: Run to verify it fails**

Run: `CAIRN_TEST_PG=… cargo test -p cairn-node --test person_charts`
Expected: FAIL — unresolved import `cairn_node::patient::person`.

- [ ] **Step 4: Write db/054**

```sql
-- db/054 — the person's chart set, and a duplicate flag over it (ADR-0076 decision 1).
--
-- WHY IN THE DATABASE. Every combined read — the medication list now, allergies when they
-- exist, the duplicate banner (R5) — must agree on which charts are "this person". One
-- function answers it, over db/018's person_member projection, so no two readers can
-- disagree about the set they are combining.
--
-- REPLAY-SAFE: CREATE OR REPLACE only; nothing here changes a table or a view.
BEGIN;

-- 1. The chart set: every chart in p_patient's link component, or p_patient alone when it
--    has never been linked (person_member has no row for a chart no linkage event touched;
--    an unlinked chart that once had an edge maps to itself). Ordered for stable reads;
--    callers canonicalise in Rust (ChartSet) regardless.
CREATE OR REPLACE FUNCTION cairn_person_charts(p_patient uuid)
RETURNS SETOF uuid
LANGUAGE sql STABLE
SET search_path = public, pg_temp
AS $$
    SELECT m.patient_id
    FROM person_member m
    WHERE m.person_id = (SELECT person_id FROM person_member WHERE patient_id = p_patient)
    UNION
    SELECT p_patient
    ORDER BY 1
$$;
GRANT EXECUTE ON FUNCTION cairn_person_charts(uuid) TO cairn_agent;

-- 2. Un-reconciled duplicates ACROSS a chart set. patient_medication_reconciliation_flag
--    (db/033) groups by patient_id, so the same drug recorded on two LINKED charts is two
--    groups on two patients and is never flagged — on a combined list that is two unflagged
--    lines for one drug, a double-dose reading hazard. This is the same rule over the SET:
--    active threads on any chart in p_charts sharing a dup_key and spanning more than one
--    group. Returns the flagged GROUP ids (every group those threads display under).
--    DRIFT: the dup_key expression is byte-identical to db/033's view (and db/031's);
--    medication_dup_key_drift.rs pins that — change all three together or none.
CREATE OR REPLACE FUNCTION cairn_medication_duplicate_groups(p_charts uuid[])
RETURNS SETOF uuid
LANGUAGE sql STABLE
SET search_path = public, pg_temp
AS $$
    SELECT DISTINCT t.group_id
    FROM (
        SELECT COALESCE(gm.group_id, s.medication_id) AS group_id,
               coalesce('code:' || (mc.coding_system COLLATE "C") || '|' || (mc.coding_code COLLATE "C"),
                        'term:' || lower(btrim(s.term) COLLATE "C")) AS dup_key
        FROM medication_statement s
        LEFT JOIN medication_group_member gm ON gm.medication_id = s.medication_id
        LEFT JOIN medication_coding mc ON mc.medication_id = s.medication_id
        WHERE s.patient_id = ANY(p_charts)
          AND NOT EXISTS (SELECT 1 FROM medication_cessation c WHERE c.medication_id = s.medication_id)
    ) t
    WHERE t.dup_key IN (
        SELECT dup_key FROM (
            SELECT COALESCE(gm.group_id, s.medication_id) AS group_id,
                   coalesce('code:' || (mc.coding_system COLLATE "C") || '|' || (mc.coding_code COLLATE "C"),
                            'term:' || lower(btrim(s.term) COLLATE "C")) AS dup_key
            FROM medication_statement s
            LEFT JOIN medication_group_member gm ON gm.medication_id = s.medication_id
            LEFT JOIN medication_coding mc ON mc.medication_id = s.medication_id
            WHERE s.patient_id = ANY(p_charts)
              AND NOT EXISTS (SELECT 1 FROM medication_cessation c WHERE c.medication_id = s.medication_id)
        ) u
        GROUP BY dup_key
        HAVING count(DISTINCT group_id) > 1
    )
$$;
GRANT EXECUTE ON FUNCTION cairn_medication_duplicate_groups(uuid[]) TO cairn_agent;

COMMIT;
```

(Implementer: the inner subquery is repeated so the SQL stays a plain, reviewable `IN` over one grain — a CTE is equally fine if it reads more clearly; the dup_key literal must appear byte-identical to db/033's in whichever form you choose, and Step 7's drift guard is what checks it.)

- [ ] **Step 5: Write `patient/person.rs`**

```rust
//! Which charts are this person (ADR-0076 decision 1) — the Rust face of db/054's
//! `cairn_person_charts`. Every combined read calls this, so every reader agrees on the set.
use cairn_medication_view::ChartSet;
use uuid::Uuid;

/// The chart set `patient` belongs to: every chart in its link component, or itself alone.
///
/// Errors only on a database failure. An empty answer cannot happen (the SQL always
/// includes `patient` itself), and is reported as an error rather than guessed around
/// should the function ever be changed to return one.
pub async fn person_charts(
    client: &(impl tokio_postgres::GenericClient + Sync),
    patient: Uuid,
) -> anyhow::Result<ChartSet> {
    let rows = client
        .query(
            "SELECT c::text AS chart FROM cairn_person_charts($1::text::uuid) AS c",
            &[&patient.to_string()],
        )
        .await?;
    let charts: Result<Vec<Uuid>, uuid::Error> =
        rows.iter().map(|r| r.get::<_, String>("chart").parse()).collect();
    ChartSet::new(charts?).ok_or_else(|| {
        anyhow::anyhow!("cairn_person_charts returned no chart for {patient}; expected at least itself")
    })
}
```

- [ ] **Step 6: Bump `SCHEMA_GENERATION` to 54 and add the loader entry**, rebuild, run `cargo test -p cairn-node --test person_charts` → PASS, and `cargo test -p cairn-node --lib db::` (the generation/list guard) → PASS.

- [ ] **Step 7: Drift guard for the dup_key expression** — create `crates/cairn-node/tests/medication_dup_key_drift.rs` (no DB, the `name_winner_order_drift.rs` pattern): `include_str!` db/033 and db/054, extract the `coalesce('code:' … )` expression text with a fixed start marker `coalesce('code:' || (` and end marker `COLLATE "C"))`, normalise whitespace, and assert db/054's copies equal db/033's. Include a positive control: the extractor finds exactly the expected number of occurrences (1 in db/033's view select, plus its GROUP BY copy; 2 in db/054) so a guard that finds nothing cannot pass. Run → PASS; then change one character of db/054's copy, run → FAIL, revert (record the mutation in the commit message).

- [ ] **Step 8: Commit** — `git add db/054_person_charts.sql crates/ && git commit -m "feat(R1): cairn_person_charts and a set-wide duplicate flag (db/054, SCHEMA 54, ADR-0076)"`

---

### Task 3: the chart model carries its set, each row its source charts, each member its chart

**Files:**
- Modify: `crates/cairn-medication-view/src/row.rs` (`MedicationRow.source_charts: Vec<Uuid>`; `MemberVouch.patient_id: Uuid`)
- Modify: `crates/cairn-medication-view/src/chart.rs` (`PatientMedicationList.charts: ChartSet`; `empty(charts: ChartSet)`)
- Modify: `crates/cairn-medication-view/src/fixtures.rs`, `targeting.rs` tests, and every construction site: `cairn-gui/cairn-gui-tabs/cairn-gui-tab-medications/src/view.rs` (2+2+4), `cairn-gui/cairn-gui-tabs/cairn-gui-tab-medications/src/lib.rs` (2 `empty()`), `cairn-gui/cairn-gui-data/src/mock/mod.rs` (1 `empty()`), `crates/cairn-node/src/medication/read.rs` (1 each — filled properly in Task 4; here set `source_charts: vec![patient_id]`, `patient_id: patient` and `charts: ChartSet::single(patient)` so behaviour is unchanged)

**Interfaces:**
- Produces: `MedicationRow::source_charts` (sorted, the charts owning ≥1 member thread of the group); `MemberVouch::patient_id` (the chart that thread lives on — the chart its attestation must name); `PatientMedicationList::charts` (the set this list was read over); `PatientMedicationList::empty(charts: ChartSet)`.

This task is a pure shape change with **no behaviour change**; the whole existing suite is its test.

- [ ] **Step 1: Write the failing test** in `chart.rs` tests:

```rust
#[test]
fn an_empty_list_still_says_which_charts_it_covers() {
    let one = Uuid::from_u128(9);
    let list = PatientMedicationList::empty(ChartSet::single(one));
    assert_eq!(list.charts.members(), &[one]);
    assert!(list.rows.is_empty());
}
```

- [ ] **Step 2: Run** `cargo test -p cairn-medication-view` → FAIL (no `charts`, `empty` takes no argument).
- [ ] **Step 3: Add the three fields** with doc comments (why each exists — `patient_id` on a member: *a combined list's sign-off attests each thread under the chart it lives on, ADR-0076 decision 2, so the member must carry it*; `source_charts`: *the row names where the drug was recorded, so a clinician reading a combined list can tell which chart a line came from*), and update every construction site listed above. Fixtures: `sample_rows()` gets `source_charts: vec![FIXTURE_PATIENT_UUID]` for ordinary rows and two charts for the cross-patient fixture row; `sample_chart()` gets `charts: ChartSet::single(<fixture patient uuid>)`; mock `medications()` returns `empty(ChartSet::single(parsed patient))` (parse failure → the existing `DataError` path).
- [ ] **Step 4: Run** `cargo test -p cairn-medication-view && cargo test -p cairn-node --lib` and, in `cairn-gui/`, `cargo test` (declare `CAIRN_ALLOW_DB_SKIP=1`) → PASS, no behaviour change.
- [ ] **Step 5: Commit** — `feat(R1): the chart model names its chart set, row sources and member charts (no behaviour change)` — commit BOTH trees' changes together, and refresh `cairn-gui/Cargo.lock` only if cargo changed it (it should not: no dependency changed).

---

### Task 4: the set read — select by membership, fix #334, flag duplicates across the set

**Files:**
- Modify: `crates/cairn-node/src/medication/read.rs`
- Modify: `crates/cairn-node/tests/medication_read.rs` (re-express the two #334 tests — see Step 6)
- Create: `crates/cairn-node/tests/combined_read.rs`

**Interfaces:**
- Consumes: `person_charts` (Task 2), `cairn_medication_duplicate_groups` (Task 2), the Task 3 fields.
- Produces: `pub async fn list_patient_medications(client, patient: Uuid) -> anyhow::Result<PatientMedicationList>` — **same signature**, now reads `person_charts(patient)`; plus `pub async fn list_chart_set_medications(client, charts: &ChartSet) -> anyhow::Result<PatientMedicationList>` (what the first calls). Pure private helpers: `fn missing_groups(member_groups: impl Iterator<Item = Uuid>, shown: &HashSet<Uuid>) -> Vec<Uuid>` and `fn reaches_outside(set: &ChartSet, group_charts: &[Uuid]) -> bool`.

**The read, rewritten (keep the module's "several small queries" shape):**
1. `members` ← `read_member_vouches(client, charts)`: the existing SQL with `WHERE g.patient_id = ANY($1::text[]::uuid[])`, also selecting `g.patient_id::text` into `MemberVouch::patient_id`.
2. `groups` ← the keys of `members` (every group with a member thread on a chart in the set). **This is the #334 fix**: a group is found through ITS members, not through `medication_group_display`'s single winning `patient_id`.
3. Rows ← `list_sql(view)` becomes `… FROM {view} WHERE medication_id = ANY($1::text[]::uuid[])` bound to `groups`; the existing dedup-by-`group_id` stays (the view still emits one row per `(group, patient)` for a cross-chart group — the comment is updated to say the dedup now also collapses a group inside the set).
4. `group_charts` ← one query: `SELECT group_id::text, array_agg(DISTINCT patient_id::text) FROM medication_thread_group WHERE group_id = ANY($1…) GROUP BY group_id` → each row's `source_charts` (sorted).
5. `cross_patient` for a row ← `reaches_outside(charts, &all_charts_of_group)` where `all_charts_of_group` comes from `medication_group_cross_patient.patients` for groups that view lists (the view already includes charts known only through an orphan cessation — keep reading it for that reason) — a group is a hazard iff that list is not wholly inside the set.
6. `reconciliation_flagged` ← `SELECT g::text FROM cairn_medication_duplicate_groups($1::text[]::uuid[]) g` over the set (replaces the per-patient view query).
7. `coding_conflict` ← unchanged SQL but scoped `g.patient_id = ANY(set)`.
8. `groups_missing_from_chart` ← `missing_groups(members.keys(), &seen)` — kept as the defensive net; by construction it is now empty unless a view drops a group it should not, and its doc comment says so.
9. `separation_targets` ← unchanged, over the hazardous groups.
10. `charts` ← the set.

- [ ] **Step 1: Pure helper tests** in `read.rs`'s test module:

```rust
#[test]
fn a_group_wholly_inside_the_set_is_not_a_hazard() {
    let set = ChartSet::new([Uuid::from_u128(1), Uuid::from_u128(2)]).unwrap();
    assert!(!reaches_outside(&set, &[Uuid::from_u128(2), Uuid::from_u128(1)]));
}

#[test]
fn a_group_reaching_one_chart_outside_is_a_hazard() {
    let set = ChartSet::new([Uuid::from_u128(1), Uuid::from_u128(2)]).unwrap();
    assert!(reaches_outside(&set, &[Uuid::from_u128(2), Uuid::from_u128(3)]));
}

#[test]
fn a_group_with_members_but_no_row_is_missing() {
    let shown: HashSet<Uuid> = [Uuid::from_u128(1)].into();
    assert_eq!(
        missing_groups([Uuid::from_u128(2), Uuid::from_u128(1)].into_iter(), &shown),
        vec![Uuid::from_u128(2)]
    );
}
```

Also update `both_chart_views_are_read_with_the_same_columns` to assert `WHERE medication_id = ANY($1::text[]::uuid[])`.

- [ ] **Step 2: DB tests** in `combined_read.rs`:

```rust
//! ADR-0076 R1: a linked chart reads as ONE list over its chart set, each row naming its
//! source chart; a group is found through its members (the #334 fix); a group reaching a
//! chart OUTSIDE the set is still a hazard; the same drug on two linked charts is flagged.
mod common;
use cairn_event::SigningKey;
use cairn_medication_view::{sign_off_targets, withheld_rows, ChartSet};
use cairn_node::db;
use cairn_node::medication::read::list_patient_medications;
use cairn_node::medication::{assert_medication, AssertMedicationInput};
use common::{cs, medication_setup as setup, submit_link_event, submit_registration};
use tokio_postgres::Client;
use uuid::Uuid;

/// One registered chart (#345: the birth act precedes everything recorded about it).
async fn chart(c: &Client, sk: &SigningKey, kid: &str) -> Uuid {
    let p = Uuid::now_v7();
    submit_registration(c, sk, kid, p, 0).await;
    p
}

/// Assert one active medication on `patient`; returns its thread id. Same shape as
/// `medication_read.rs`'s helper of the same name (kept file-local: two suites, one line of
/// difference each, is below the bar for `common/`).
async fn assert_one(c: &mut Client, sk: &SigningKey, kid: &str, patient: Uuid, term: &str) -> Uuid {
    assert_medication(
        c, sk, kid, "origin-a", patient,
        &AssertMedicationInput {
            term, coding: None, formulation: None,
            dose_amount: Some("500"), dose_unit: Some("mg"), sig: None,
            info_source: "patient", started: None, started_precision: None,
        },
        None, None,
    )
    .await
    .unwrap()
}

/// Fold two threads into one group with `first` as the group id — the peer-arrival shape
/// `medication_read.rs`'s #334 tests use (the local door refuses a cross-chart reconcile).
async fn group(c: &Client, first: Uuid, second: Uuid) {
    c.execute(
        "INSERT INTO medication_group_member (medication_id, group_id) VALUES \
         ($1::text::uuid, $1::text::uuid), ($2::text::uuid, $1::text::uuid)",
        &[&first.to_string(), &second.to_string()],
    )
    .await
    .unwrap();
}
```

Every test below opens with these five lines, written as `// PREAMBLE` in the plan and in full in the file (a macro would hide the `return` a reader needs to see):

```rust
let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
let _guard = db::test_serial_guard(&base).await.unwrap();
let mut c = db::connect_and_load_schema(&base).await.unwrap();
c.batch_execute("TRUNCATE patient_link, person_member").await.unwrap();
let (sk, kid, _hsk, _hkid) = setup(&c).await;
```

```rust
#[tokio::test]
async fn a_linked_pair_reads_as_one_list_with_source_labels() {
    // PREAMBLE
    let a = chart(&c, &sk, &kid).await;
    let b = chart(&c, &sk, &kid).await;
    let met = assert_one(&mut c, &sk, &kid, a, "metformin").await;
    let aml = assert_one(&mut c, &sk, &kid, b, "amlodipine").await;
    submit_link_event(&c, &sk, &kid, a, b, 10, true).await;

    for opened in [a, b] {
        let list = list_patient_medications(&c, opened).await.unwrap();
        assert_eq!(list.charts, ChartSet::new([a, b]).unwrap(), "opened {opened}");
        assert_eq!(list.rows.len(), 2, "both charts' drugs, from either side");
        let row = |g: Uuid| list.rows.iter().find(|r| r.group_id == g).unwrap();
        assert_eq!(row(met).source_charts, vec![a]);
        assert_eq!(row(aml).source_charts, vec![b]);
        assert!(list.rows.iter().all(|r| !r.cross_patient), "nothing reaches outside the set");
    }
}

#[tokio::test]
async fn a_group_inside_the_set_shows_once_and_is_signable() {
    // PREAMBLE
    let a = chart(&c, &sk, &kid).await;
    let b = chart(&c, &sk, &kid).await;
    let ta = assert_one(&mut c, &sk, &kid, a, "metformin").await;
    let tb = assert_one(&mut c, &sk, &kid, b, "metformin").await;
    group(&c, ta, tb).await;
    submit_link_event(&c, &sk, &kid, a, b, 10, true).await;

    let list = list_patient_medications(&c, b).await.unwrap();
    assert_eq!(list.rows.len(), 1, "one drug, one line — never the view's two (group, patient) rows");
    let row = &list.rows[0];
    assert_eq!(row.source_charts, {
        let mut v = vec![a, b];
        v.sort();
        v
    });
    assert!(!row.cross_patient, "both charts are this person: not a wrong-chart hazard");
    let owner = |t: Uuid| row.members.iter().find(|m| m.medication_id == t).unwrap().patient_id;
    assert_eq!(owner(ta), a);
    assert_eq!(owner(tb), b);
    let mut both = vec![ta, tb];
    both.sort();
    assert_eq!(sign_off_targets(&list.rows), both, "a group inside the set is signable");
}

#[tokio::test]
async fn a_group_reaching_outside_the_set_is_still_a_hazard() {
    // PREAMBLE
    let a = chart(&c, &sk, &kid).await;
    let b = chart(&c, &sk, &kid).await;
    let outsider = chart(&c, &sk, &kid).await;
    let tb = assert_one(&mut c, &sk, &kid, b, "warfarin").await;
    let to = assert_one(&mut c, &sk, &kid, outsider, "warfarin").await;
    group(&c, tb, to).await;
    submit_link_event(&c, &sk, &kid, a, b, 10, true).await; // the outsider is NOT linked

    let list = list_patient_medications(&c, a).await.unwrap();
    assert_eq!(list.rows.len(), 1);
    assert!(list.rows[0].cross_patient, "the group reaches a chart that is not this person");
    assert_eq!(withheld_rows(&list.rows), vec![tb], "and is withheld from sign-off");
    assert!(list.separation_targets.contains_key(&tb), "with the arguments to separate it");

    // #334's other half: the outsider's own chart SHOWS the group too (it used to vanish).
    let theirs = list_patient_medications(&c, outsider).await.unwrap();
    assert_eq!(theirs.rows.len(), 1);
    assert!(theirs.rows[0].cross_patient);
    assert!(theirs.groups_missing_from_chart.is_empty());
}

#[tokio::test]
async fn the_same_drug_on_two_linked_charts_is_flagged() {
    // PREAMBLE
    let a = chart(&c, &sk, &kid).await;
    let b = chart(&c, &sk, &kid).await;
    assert_one(&mut c, &sk, &kid, a, "metformin").await;
    assert_one(&mut c, &sk, &kid, b, "Metformin ").await; // the dup_key lowers and trims

    // Positive control: before the link each chart holds ONE metformin and nothing is flagged,
    // so the flag below is about the SET, not about metformin.
    let alone = list_patient_medications(&c, a).await.unwrap();
    assert!(alone.rows.iter().all(|r| !r.reconciliation_flagged));

    submit_link_event(&c, &sk, &kid, a, b, 10, true).await;
    let list = list_patient_medications(&c, a).await.unwrap();
    assert_eq!(list.rows.len(), 2, "two recordings, two lines — until reconciled");
    assert!(
        list.rows.iter().all(|r| r.reconciliation_flagged),
        "the same drug on two linked charts must never show twice unflagged"
    );
}

#[tokio::test]
async fn an_unlink_splits_the_read_again() {
    // PREAMBLE
    let a = chart(&c, &sk, &kid).await;
    let b = chart(&c, &sk, &kid).await;
    assert_one(&mut c, &sk, &kid, a, "metformin").await;
    assert_one(&mut c, &sk, &kid, b, "amlodipine").await;
    submit_link_event(&c, &sk, &kid, a, b, 10, true).await;
    submit_link_event(&c, &sk, &kid, a, b, 11, false).await;
    let list = list_patient_medications(&c, a).await.unwrap();
    assert_eq!(list.charts, ChartSet::single(a));
    assert_eq!(list.rows.len(), 1);
}
```

  **The golden** — `a_never_linked_chart_reads_exactly_as_before`: on the **Task 3 commit** (before this task changes the read), write a test that builds one chart holding an active drug, a ceased drug (`cease_medication`), a reconciled same-chart pair (through the real reconcile orchestrator `cairn_node::medication::reconciliation::reconcile_medications`, which the local door permits within one chart) and a coded drug; serialise `list_patient_medications` with `serde_json::to_value`, remove the keys `charts`, and from every row `source_charts` and every member `patient_id` (new in Task 3; the golden is about everything that existed before), replace every minted uuid by its role name (`"<active>"`, `"<ceased>"`, …) through a lookup map so the literal is stable, and print it. Paste the printed JSON as the expected literal, then assert equality. It must pass on the Task 3 commit AND after this task. (The ids are minted per run, hence the role-name substitution — without it the golden could never be a literal.)

- [ ] **Step 3: Run** `cargo test -p cairn-node --test combined_read` → the new tests FAIL (golden passes only on the Task 3 commit — capture it there first).
- [ ] **Step 4: Rewrite the read** as listed above; every changed query's doc comment says why it is scoped by set/membership.
- [ ] **Step 5: Run** `--test combined_read` and `--test medication_read` → combined PASS; medication_read's two #334 tests FAIL (expected — they pin the old invisible state).
- [ ] **Step 6: Re-express the two #334 tests** — never just flip an assertion (the #671 lesson: inverting one half of a pair deletes the other half):
  - `a_cross_patient_group_is_missing_from_the_losing_patients_chart` → rename `a_cross_patient_group_shows_on_both_charts_flagged` (#334 fixed): B's chart now SHOWS the group, `cross_patient`, `groups_missing_from_chart` empty; A's likewise; both charts' sign-off withhold it (`withheld == [thread_a]`) and carry both threads in `separation_targets`. Keep every existing assertion about `separation_targets` and withholding.
  - `an_incomplete_chart_still_signs_every_line_it_can_show` → rename `a_hazardous_line_never_blocks_a_sound_one` — the #339 property is unchanged and must stay pinned: B's warfarin is SIGNED, the cross-patient group is WITHHELD and reported with its targets, A's thread is untouched. The "invisible group" half is now covered by the pure `missing_groups` test (Step 1), and the doc comment says so.
- [ ] **Step 7: Run** `--test medication_read --test combined_read --test medication_signoff` → PASS. Run the libc-locale DB too if any test touches case (none should; note it in the commit if not needed).
- [ ] **Step 8: Commit** — `fix(#334): the medication read selects groups by membership over the chart set (ADR-0076 decision 1)` — use `fix(#334):` (parenthesised: does not auto-close; the PR body closes it).

---

### Task 5: sign-off over a set — each thread under its own chart, and a changed set refuses

**Files:**
- Modify: `crates/cairn-node/src/medication/signoff.rs`
- Modify: `crates/cairn-node/src/main.rs` (the `MedicationSignOff` arm: pass `None`, and print the chart set signed across when it has more than one chart)
- Modify: `crates/cairn-sync/tests/clinical_pull.rs` **only if** it calls `sign_off_medication_list` (grep first; the cross-crate-signature trap)
- Test: `crates/cairn-node/tests/combined_read.rs` (append)

**Interfaces:**
- Consumes: `MemberVouch::patient_id`, `PatientMedicationList::charts`.
- Produces: `pub async fn sign_off_medication_list(client, _node_sk, node_origin, params, patient: Uuid, displayed: Option<&ChartSet>) -> anyhow::Result<SignOffOutcome>`; `SignOffOutcome.charts: ChartSet`; pure `fn thread_charts(rows: &[MedicationRow]) -> HashMap<Uuid, Uuid>` (thread → its chart).

Rules (ADR-0076 decisions 2 and 3):
- `displayed: Some(set)` and `first_read.charts != *set` → refuse BEFORE minting any HLC: *"the linked charts changed while this list was on screen (shown: …; now: …); nothing was signed — reload the chart and sign again"*. `None` (the CLI, which shows no list) skips only this check.
- `first_read.charts != second_read.charts` → refuse with the same sentence shape (a link landing between the two reads).
- Each target is attested as `attest_thread_in_tx(&tx, params, thread_charts[&thread], thread, hlc)`; a target with no entry is a `FailedLine` ("its chart could not be read"), never attested under the opened chart.

- [ ] **Step 1: Failing tests** (append to `combined_read.rs`; add `use cairn_node::medication::signoff::sign_off_medication_list; use cairn_node::medication::AttestParams; use common::attestation_count;` and use `(sk, kid, hsk, hkid)` from the preamble):

```rust
fn params<'a>(hsk: &'a SigningKey, hkid: &'a str) -> AttestParams<'a> {
    AttestParams { human_sk: hsk, human_kid: hkid, basis: None, note: None }
}

#[tokio::test]
async fn a_combined_sign_off_attests_each_thread_under_its_own_chart() {
    // PREAMBLE (binding hsk, hkid)
    let a = chart(&c, &sk, &kid).await;
    let b = chart(&c, &sk, &kid).await;
    let met = assert_one(&mut c, &sk, &kid, a, "metformin").await;
    let aml = assert_one(&mut c, &sk, &kid, b, "amlodipine").await;
    submit_link_event(&c, &sk, &kid, a, b, 10, true).await;
    let shown = list_patient_medications(&c, a).await.unwrap().charts;

    let out = sign_off_medication_list(&mut c, &sk, "origin-a", &params(&hsk, &hkid), a, Some(&shown))
        .await
        .unwrap();
    assert_eq!(out.attested.len(), 2, "one gesture covers both charts' lines");
    for (thread, event) in out.attested.iter().zip(&out.event_ids) {
        let row = c
            .query_one(
                "SELECT patient_id::text AS p FROM event_log WHERE event_id = $1::text::uuid",
                &[&event.to_string()],
            )
            .await
            .unwrap();
        let on: Uuid = row.get::<_, String>("p").parse().unwrap();
        let expected = if *thread == met { a } else { assert_eq!(*thread, aml); b };
        assert_eq!(on, expected, "thread {thread} is attested under the chart it lives on");
    }
}

#[tokio::test]
async fn a_sign_off_is_refused_when_the_chart_set_changed() {
    // PREAMBLE (binding hsk, hkid)
    let a = chart(&c, &sk, &kid).await;
    let b = chart(&c, &sk, &kid).await;
    let met = assert_one(&mut c, &sk, &kid, a, "metformin").await;
    let shown = list_patient_medications(&c, a).await.unwrap().charts; // {a}: what was on screen
    submit_link_event(&c, &sk, &kid, a, b, 10, true).await; // lands while the list is shown

    let err = sign_off_medication_list(&mut c, &sk, "origin-a", &params(&hsk, &hkid), a, Some(&shown))
        .await
        .unwrap_err();
    assert!(format!("{err:#}").contains("linked charts changed"), "{err:#}");
    assert_eq!(attestation_count(&c, met).await, 0, "nothing was signed");
}

#[tokio::test]
async fn the_cli_form_signs_the_set_it_finds() {
    // PREAMBLE (binding hsk, hkid)
    let a = chart(&c, &sk, &kid).await;
    let b = chart(&c, &sk, &kid).await;
    assert_one(&mut c, &sk, &kid, a, "metformin").await;
    assert_one(&mut c, &sk, &kid, b, "amlodipine").await;
    submit_link_event(&c, &sk, &kid, a, b, 10, true).await;
    let out = sign_off_medication_list(&mut c, &sk, "origin-a", &params(&hsk, &hkid), a, None)
        .await
        .unwrap();
    assert_eq!(out.attested.len(), 2);
    assert_eq!(out.charts, ChartSet::new([a, b]).unwrap());
}
```
- [ ] **Step 2: Run** → FAIL (arity).
- [ ] **Step 3: Implement** the three rules; update every caller (`main.rs`, `medication_signoff.rs`/`medication_read.rs` tests pass `None`; the GUI caller is Task 7). Doc comment on the new parameter: why `None` exists and why it is not the default for a surface that shows a list.
- [ ] **Step 4: Run** `--test combined_read --test medication_signoff --test medication_read` and `cargo test -p cairn-sync --test clinical_pull` if touched → PASS.
- [ ] **Step 5: Commit** — `feat(R1): a combined sign-off attests each thread under its own chart and refuses a changed set (ADR-0076 decisions 2-3)`

---

### Task 6: the member identity lines

**Files:**
- Modify: `crates/cairn-node/src/patient/person.rs`
- Test: `crates/cairn-node/tests/person_charts.rs` (append)

**Interfaces:**
- Produces: `pub struct ChartIdentity { pub patient_id: Uuid, pub name: Option<String>, pub birth_date: Option<String>, pub trust: String }`; `pub async fn chart_identities(client, charts: &ChartSet) -> anyhow::Result<Vec<ChartIdentity>>` — one per member, in `ChartSet` order. `name` from `patient_name_current.value`, `birth_date` from `patient_demographic` (`field = 'dob'`, its `value` verbatim — never reformatted: the difference between two members' dates is the point), `trust` from `chart_trust.trust_state`, absent row → `"confirmed"` (the `person_chart_trust` convention).

No winner is chosen across members (ADR-0076 decision 1): the header lists each chart's own line.

- [ ] **Step 1: Failing test** (append to `person_charts.rs`; add `use cairn_event::demographics::{dob_assertion_body, render_dob_twin}; use cairn_node::patient::person::chart_identities; use common::{chart_named, submit_signed, EventSpec};`):

```rust
async fn dob(c: &tokio_postgres::Client, sk: &cairn_event::SigningKey, kid: &str, p: Uuid, v: &str) {
    submit_signed(c, sk, kid, EventSpec {
        patient: p,
        event_type: "demographic.field.asserted",
        schema_version: "demographic.field/1",
        payload: dob_assertion_body(v, "day", None, "patient-stated"),
        plaintext_twin: Some(render_dob_twin(v, "day", "patient-stated")),
        wall: 5,
    })
    .await
    .expect("dob accepted");
}

#[tokio::test]
async fn each_member_reports_its_own_name_and_date() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    c.batch_execute("TRUNCATE patient_link, person_member").await.unwrap();
    let (sk, kid, _, _) = setup(&c).await;
    let smith = chart_named(&c, &sk, &kid, 1, "Mary SMITH").await;
    let smythe = chart_named(&c, &sk, &kid, 1, "Mary SMYTHE").await;
    let bare = fresh(&c, &sk, &kid).await; // no name, no date
    dob(&c, &sk, &kid, smith, "1950-07-01").await;
    dob(&c, &sk, &kid, smythe, "1950-01-07").await; // the day/month slip that made the duplicate
    submit_link_event(&c, &sk, &kid, smith, smythe, 10, true).await;
    submit_link_event(&c, &sk, &kid, smythe, bare, 11, true).await;

    let set = person_charts(&c, smith).await.unwrap();
    let lines = chart_identities(&c, &set).await.unwrap();
    assert_eq!(
        lines.iter().map(|l| l.patient_id).collect::<Vec<_>>(),
        set.members().to_vec(),
        "one line per member, in set order"
    );
    let of = |p: Uuid| lines.iter().find(|l| l.patient_id == p).unwrap();
    assert_eq!(of(smith).name.as_deref(), Some("Mary SMITH"));
    assert_eq!(of(smith).birth_date.as_deref(), Some("1950-07-01"));
    assert_eq!(of(smythe).birth_date.as_deref(), Some("1950-01-07"), "never a merged winner");
    assert_eq!(of(bare).name, None, "absence is None, never an empty string");
    assert_eq!(of(bare).birth_date, None);
    assert_eq!(of(smith).trust, "confirmed");
}
```
- [ ] **Step 2: Run** → FAIL. **Step 3: Implement** (three small queries, joined in Rust by id, the read.rs style). **Step 4: Run** → PASS.
- [ ] **Step 5: Commit** — `feat(R1): each linked chart's own identity line, no winner chosen (ADR-0076 decision 1)`

---

### Task 7: the window — show the set, name it on every command

**Files:**
- Modify: `cairn-gui/cairn-gui-tabs/cairn-gui-tab-medications/src/view.rs` (`MedListView.charts: Vec<String>`; row label `source` when the list is linked) — **and split it**: it is 645 lines; move the row-building (`MedListRowView` + its builder fns) into a new `row_view.rs` in the same crate in this task's first commit, behaviour-neutral, before adding anything.
- Create: `cairn-gui/cairn-gui-tauri/src/chart_set.rs` (the displayed-set check — keeps `funnel/window.rs` and `commands.rs` from growing)
- Modify: `cairn-gui/cairn-gui-tauri/src/commands.rs` (`med_list` returns `ChartPane { list: MedListView, members: Vec<MemberLine> }`; `sign_off`/`cease` take `charts: Vec<String>`)
- Modify: `cairn-gui/cairn-gui-tauri/src-ui/main.js`, `src-ui/index.html` (a `<ul id="linked-charts">` under the identity header, hidden when not linked)

**Interfaces:**
- Consumes: `list_patient_medications`, `sign_off_medication_list(…, Some(&set))`, `chart_identities`, `MemberVouch::patient_id`.
- Produces: `pub fn check_displayed_set(read: &ChartSet, displayed: &[String]) -> Result<(), String>` (pure; the refusal sentence lives here once); `#[derive(Serialize)] pub struct MemberLine { patient_id: String, text: String }` where `text` is `"<name or (no name recorded)> · born <dob or 'date of birth not recorded'> · identity <trust> · chart <id>"`.

Rules:
- `med_list_impl` keeps `displayed_patient(patient_id)` FIRST (the 2c rule, unchanged), then reads, then returns the view with `charts` and, when linked, `members`.
- `sign_off_impl(patient_id, charts)`: `displayed_patient` first; then `read_chart_of` (mock or live) and `check_displayed_set(&chart.charts, &charts)?` — BEFORE the fixture-mode refusal, so a changed set is reported as that even in fixture mode; then pass `Some(&chart.charts)` to the orchestrator, whose own first-read compare is the authoritative one (the window's check is the early, mock-testable one; both are wanted — the window's read and the orchestrator's are two snapshots). `check_displayed_set`'s two sentences: unparseable → *"this window could not tell which charts are on screen — reopen the chart"*; different → *"the linked charts changed while this list was on screen — nothing was done; reload the chart"*.
- `MedListRowView` gains `source: Option<String>` — the row's source chart id(s) joined by ", ", `Some` only when the list's `charts.is_linked()`.
- `cease_impl(patient_id, charts, group, reason)`: `displayed_patient`; read the chart; `check_displayed_set(&chart.charts, &charts)?`; cease each member on **`member.patient_id`**, never on the opened chart.
- JS: `render` stores `renderedCharts = view.list.charts`; sign-off and cease send `{ patientId: renderedPatient, charts: renderedCharts }`; `clearChart` clears it; the linked list renders one `<li>` per `members` entry (text only, no buttons in R1); a row whose `source` differs from the opened chart shows it in the Medication cell's accessible text ("metformin — recorded on chart …").

- [ ] **Step 1: behaviour-neutral split** of `view.rs` → `row_view.rs`; run the cairn-gui gate → PASS; commit `refactor(gui): row view model in its own file (view.rs was 645 lines)`.
- [ ] **Step 2: Failing tests.** In `cairn-gui-tauri/src/chart_set.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use cairn_medication_view::ChartSet;
    use uuid::Uuid;

    fn ids(v: &[u128]) -> Vec<String> {
        v.iter().map(|n| Uuid::from_u128(*n).to_string()).collect()
    }
    fn set(v: &[u128]) -> ChartSet {
        ChartSet::new(v.iter().map(|n| Uuid::from_u128(*n))).unwrap()
    }

    #[test]
    fn the_same_set_in_any_order_is_accepted() {
        assert!(check_displayed_set(&set(&[1, 2]), &ids(&[2, 1])).is_ok());
    }

    #[test]
    fn a_different_set_is_refused_with_a_reload_sentence() {
        let err = check_displayed_set(&set(&[1, 2]), &ids(&[1])).unwrap_err();
        assert!(err.contains("linked charts changed"), "{err}");
        assert!(err.contains("reload"), "{err}");
    }

    #[test]
    fn an_unparseable_id_is_refused() {
        let err = check_displayed_set(&set(&[1]), &["not-a-uuid".to_string()]).unwrap_err();
        assert!(err.contains("could not tell which charts"), "{err}");
    }
}
```

In `cairn-gui-tab-medications` (`row_view.rs` tests): `a_linked_list_labels_each_row_with_its_source` — `build_view` over `sample_chart()` with `charts` replaced by a two-member `ChartSet` and one row's `source_charts` set to the second member: that row's `source` is `Some(<second id>)`, every other row's is `None`; over the unmodified single-chart fixture every `source` is `None`.

In `cairn-gui-tauri/src/commands.rs` tests: `sign_off_refuses_a_changed_set` — `AppState::mock(Some(fixture))`; `sign_off_impl(&state, &fixture_s, vec![fixture_s.clone(), Uuid::from_u128(99).to_string()])` → `Err` containing `"linked charts changed"`. The set check runs before the fixture-mode refusal, so the assertion proves the order.
- [ ] **Step 3: Run** (in `cairn-gui/`, `CAIRN_ALLOW_DB_SKIP=1 cargo test`) → FAIL. **Step 4: Implement.** **Step 5: Run** → PASS; `cargo clippy --all-targets -- -D warnings` and `RUSTDOCFLAGS=-D warnings cargo doc --no-deps` in `cairn-gui/` → clean.
- [ ] **Step 6: A mock walk of the webview** — the 2c technique (a headless browser over `src-ui/` with a stand-in `invoke` returning Rust-shaped payloads): a linked `med_list` payload renders the member list and source labels; sign-off sends `charts`. Record the walk in the PR; the live Tauri-IPC pass stays a human act.
- [ ] **Step 7: Commit** — `feat(R1): the window shows the linked charts and every chart command names the set (ADR-0076 decision 3)`

---

### Task 8: currency, filings, and the gate

**Files:** `docs/HANDOVER.md`, `docs/ROADMAP.md`, the design page (a dated note under *R1* if anything changed while building), `cairn-gui/cairn-gui-tauri/results/RUNBOOK.md` (a note that a linked chart's header lists its members — the accessibility pass should read it).

- [ ] **Step 1: File the write-side mirror** — `gh issue create`: *"Reconciling the duplicate threads of two LINKED charts is refused by db/033's local cross-patient guard (ADR-0076 R1 follow-up)"*, body: the guard at `db/033_medication_reconciliation.sql` (the `v_pa <> v_pb` raise) refuses a reconciliation across charts that `cairn_person_charts` now calls one person; the combined read flags the duplicate (`cairn_medication_duplicate_groups`) but the clinician cannot resolve it by reconciliation, only by ceasing one; a decision is needed on whether the guard should admit threads within one link component, and what an unlink then does to such a group (it becomes a flagged cross-patient group — surfaced, never auto-separated). Record the number in HANDOVER.
- [ ] **Step 2: Full gate, in CI's order, after the last edit** — `CARGO_TARGET_DIR=/tmp/cairn-r1 scripts/run-db-gated-tests.sh` (background it; ~2 h), `cargo fmt --check` both trees, `cargo clippy --workspace --all-targets -- -D warnings`, `RUSTDOCFLAGS=-D warnings cargo doc --workspace --no-deps`, `cargo deny check`, the cairn-gui gate (`CAIRN_ALLOW_DB_SKIP=1` + DB strings for `cairn-gui-live`), `python3 scripts/check_closing_keywords.py` over every commit message, `uv run --with-requirements docs/requirements.txt -- mkdocs build --strict`. Read each log's last line — `cmd; echo exit=$?` lies under a pipe.
- [ ] **Step 3: HANDOVER + ROADMAP** — R1 built; ⇒ NEXT is R2; the new trap (a combined read's duplicate flag is db/054's, NOT the per-patient view — "simplifying" back to `patient_medication_reconciliation_flag` re-hides the double line); the durable rule "every chart command names the displayed SET"; both documents condensed toward 500 lines without dropping an open issue number.
- [ ] **Step 4: Commit, push, mark PR #688 ready** with a body that closes #334 (`Closes #334`) and names #679/#680/#681 as `Refs` only.

## Paper-parity benchmark (§1.2)

- **Paper counterpart:** two paper folders of one patient clipped together; the clinician reads both.
- **Steps:** reading a linked chart — paper 1 (open the clipped folders) → architecture-forced 1 (open the chart; the set is read with it) → UI target 1. Signing off a combined list — paper 1 (sign the drug chart) → forced 1 → target 1: one gesture still covers every line, across both charts. `M ≤ N` throughout; R1 adds no act. The refusal on a changed set is a re-read the paper chart never needs only because paper cannot change under the reader's hand — it adds an act only when the record really changed, which is the case it exists for.
- **Time + cognitive load:** budget — opening a linked chart ≤ the single-chart open (median 222 ms write half measured for sign-off; the read's added cost is one `cairn_person_charts` call and one duplicate-flag query, ≤ 20 ms at the med-list's scale, to be measured by the runbook's med-list section on a linked fixture); cognitive load — each row names its source chart only when the list is linked, so a never-linked chart reads exactly as before. Measurement owed by this slice's runbook pass (a human act).
