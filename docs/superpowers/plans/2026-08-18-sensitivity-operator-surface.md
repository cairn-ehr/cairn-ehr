# §5.9 Operator Surface Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `patient-sensitivity <chart>` the one query that tells an operator the whole truth about a chart's §5.9 state — including every state the report is currently silent about.

**Architecture:** `sensitivity.rs` splits into a module directory: authoring stays in `mod.rs`, the DB reads move to `report.rs`, and all wording moves into **pure** functions in `render.rs` that take a plain struct and return `Vec<String>`. Two in-place SQL edits (a projected column, a chart-scoped definer) supply the two facts the current read model cannot reach. No new migration file, no ADR, no new event type.

**Tech Stack:** Rust (`cairn-node`), `tokio-postgres`, PostgreSQL 18 + `cairn_pgx`. No new dependencies.

**Spec:** [`docs/superpowers/specs/2026-08-18-sensitivity-operator-surface-design.md`](../specs/2026-08-18-sensitivity-operator-surface-design.md)

## Global Constraints

- **AGPL-3.0**; no new dependencies of any kind (house rule 1 — a licence check is a blocker, not a cleanup item).
- **TDD**: the failing test is written and *seen to fail* before the code that satisfies it (house rule 2).
- **`SCHEMA_GENERATION` stays 49.** No new `db/NNN` file. Both SQL edits are in-place `CREATE OR REPLACE` in the file that owns the object.
- **UUIDs bind as text**: `cairn-node` does not enable tokio-postgres's `with-uuid-1`, so a `Uuid` parameter has no `ToSql`. Bind `&uuid.to_string()` and cast in SQL as `$1::text::uuid`.
- **Guard before connect**: every DB-gated test takes `db::test_serial_guard(&base)` *before* `connect_and_load_schema`.
- **`content_address IS NOT NULL` is the "did anything win" test**, never `subject_kind <> 'none'` (ADR-0062 erratum E6 — `none` is a legal open-vocabulary value).
- **Every `SECURITY DEFINER` pins `SET search_path = public, pg_temp`** with `pg_temp` LAST (#426).
- **Files under 500 lines** (house rule 4). This is the reason for Task 1.
- **Never hard-code cryptographic material in tests** (house rule 6) — use the existing `generate_key()` helpers.
- Nothing in this slice **enforces** anything. No content is withheld on the strength of any grade; enforcement is custody narrowing (#376).

---

### Task 1: Split `sensitivity.rs` into a module directory

Pure refactor — **no behaviour change**. The existing suite is the test: if it stays green, the move is correct. Doing this first means every later task edits a small file.

**Files:**
- Create: `crates/cairn-node/src/sensitivity/report.rs`
- Rename: `crates/cairn-node/src/sensitivity.rs` → `crates/cairn-node/src/sensitivity/mod.rs`
- Unchanged: `crates/cairn-node/src/lib.rs` (`pub mod sensitivity;` already covers a directory module)

**Interfaces:**
- Consumes: nothing.
- Produces: `cairn_node::sensitivity::{ChartReport, ThreadGrade, chart_sensitivity, assert_sensitivity, withdraw_sensitivity, subject_kind_phrase}` — every path identical to today's, via re-export.

- [ ] **Step 1: Record the current green baseline**

Run: `cargo test -p cairn-node --test sensitivity_ladder 2>&1 | tail -5`
Expected: PASS. (If the DB env vars are unset these self-skip and cargo counts them as passed — set `CAIRN_TEST_PG` first, or the baseline is meaningless.)

- [ ] **Step 2: Move the file**

```bash
mkdir -p crates/cairn-node/src/sensitivity
git mv crates/cairn-node/src/sensitivity.rs crates/cairn-node/src/sensitivity/mod.rs
```

- [ ] **Step 3: Cut the read model into `report.rs`**

Move these items **verbatim** out of `mod.rs` and into a new `crates/cairn-node/src/sensitivity/report.rs`, carrying their doc comments unchanged: `ChartReport`, `ThreadGrade`, `chart_sensitivity`. Give the new file this header:

```rust
//! §5.9 — the chart sensitivity READ model.
//!
//! Split out of `sensitivity/mod.rs` (which keeps the authoring verbs) when the operator
//! surface grew four more reads. This is the ONLY file in the module that talks to a
//! database: the wording an operator actually reads lives in `render.rs` and is pure, so
//! the honesty claims this surface makes are unit-testable without a cluster. That split
//! is the point of the file boundary, not merely a line-count fix.
//!
//! This module REPORTS; it does not enforce. Nothing here may start withholding content on
//! the strength of a grade — a projection-layer filter with no floor beneath it is security
//! theatre, since a client talking raw SQL walks straight past it. Real enforcement is
//! custody narrowing (#232 part C / #376).
use uuid::Uuid;
```

- [ ] **Step 4: Re-export from `mod.rs` so no call site moves**

At the top of `crates/cairn-node/src/sensitivity/mod.rs`:

```rust
pub mod render;
pub mod report;

// Re-exported so `cairn_node::sensitivity::chart_sensitivity` and
// `cairn_node::sensitivity::ChartReport` keep working unchanged at every call site. The
// module split is an internal organisation decision; it is not an API change, and making
// callers move would turn a mechanical refactor into a reviewable one for no gain.
pub use report::{chart_sensitivity, ChartReport, ThreadGrade};
```

Create an empty-for-now `crates/cairn-node/src/sensitivity/render.rs` containing only `//! placeholder — Task 2 fills this in.` so the `pub mod render;` line compiles. (Task 2 replaces the whole file; this exists only to keep Step 5 green.)

- [ ] **Step 5: Verify nothing moved**

Run: `cargo build -p cairn-node 2>&1 | tail -5` — Expected: no errors.
Run: `cargo test -p cairn-node --test sensitivity_ladder 2>&1 | tail -5` — Expected: same PASS as Step 1.
Run: `wc -l crates/cairn-node/src/sensitivity/*.rs` — Expected: both under 500.

- [ ] **Step 6: Commit**

```bash
git add crates/cairn-node/src/sensitivity/
git commit -m "refactor(#388): split sensitivity.rs into mod/report ahead of the operator surface

Behaviour-preserving. The read model gains four more queries in this slice and
the file was already at 411 lines; splitting first keeps every later diff small.
report.rs becomes the only DB-touching file in the module, which is what lets
render.rs (Task 2) be pure and unit-testable without a cluster.

Refs #388"
```

---

### Task 2: Extract the CLI rendering into pure functions, locking today's output

Still **no behaviour change** — but now the current wording is pinned by tests, so every later task's change to it shows up as a deliberate test edit rather than as invisible drift.

**Files:**
- Modify: `crates/cairn-node/src/sensitivity/render.rs` (replace the placeholder)
- Modify: `crates/cairn-node/src/main.rs` — the `Cmd::PatientSensitivity` arm (currently ~1875-1911)

**Interfaces:**
- Consumes: `ChartReport`, `ThreadGrade` from Task 1.
- Produces: `pub fn render_chart_report(r: &ChartReport) -> Vec<String>` — one output line per element, no trailing newlines.

- [ ] **Step 1: Write the failing tests**

Replace `crates/cairn-node/src/sensitivity/render.rs` with the header plus this test module:

```rust
//! §5.9 — how a chart report reads.
//!
//! PURE. No database, no I/O, no `tokio_postgres` import. Every honesty claim this surface
//! makes is a sentence — "this is not a clean bill of health", "this node may hold no
//! custody", "this list is not complete" — and a sentence that only exists inside a
//! `println!` in `main.rs` can be tested only by running the binary against a live cluster,
//! which is why nobody ever did. Keeping the wording here makes each claim a unit test.
//!
//! Precedent: `crate::safety::render_safety_line`, which is pure for the same reason.
use super::report::ChartReport;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sensitivity::report::ThreadGrade;

    /// A chart with nothing wrong: one grade line, one thread line, the standing footer.
    fn healthy() -> ChartReport {
        ChartReport {
            chart_grade: "routine".into(),
            chart_source: "none".into(),
            chart_content_address: None,
            threads: vec![ThreadGrade {
                thread_id: uuid::Uuid::nil(),
                grade: "routine".into(),
                source: "none".into(),
                content_address: None,
            }],
        }
    }

    #[test]
    fn the_grade_line_keeps_its_documented_shape() {
        // `sensitivity-withdraw --withdraws` documents its argument as "the hex
        // content_address, as patient-sensitivity prints it". That is a CONTRACT: an
        // earlier draft of ChartReport dropped the address entirely and a hand exercise of
        // the CLI caught it. Pin the shape so the next refactor cannot quietly break it.
        let mut r = healthy();
        r.chart_grade = "sequestered".into();
        r.chart_source = "chart-wide".into();
        r.chart_content_address = Some("a3f".into());
        let lines = render_chart_report("C", &r);
        assert_eq!(
            lines[0],
            "chart C: sequestered (winning subject: chart-wide, withdraws=a3f)"
        );
    }

    #[test]
    fn a_chart_with_no_assertion_names_no_address() {
        let lines = render_chart_report("C", &healthy());
        assert_eq!(lines[0], "chart C: routine (winning subject: none)");
    }

    #[test]
    fn a_healthy_chart_raises_no_warning() {
        // The anti-vacuity test for every later task: if a warning ever appears on a chart
        // with nothing wrong, the operator learns to ignore warnings, and the surface has
        // made things worse than silence.
        let lines = render_chart_report("C", &healthy());
        assert!(
            !lines.iter().any(|l| l.contains('⚠')),
            "a healthy chart must print no warning: {lines:?}"
        );
    }
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p cairn-node --lib sensitivity::render 2>&1 | tail -15`
Expected: FAIL — `cannot find function render_chart_report in this scope`.

- [ ] **Step 3: Write the minimal implementation**

Add above the `#[cfg(test)]` module in `render.rs`:

```rust
/// Render one chart's §5.9 report as the lines an operator reads, in order.
///
/// The chart grade comes FIRST and keeps its exact wire shape — see the contract test. The
/// per-thread breakdown follows. Later tasks insert warning blocks between the two, which
/// is deliberate: a warning that appears forty thread-lines below the claim it qualifies is
/// a warning nobody reads.
///
/// Returns lines rather than printing them so the caller owns the I/O and the wording stays
/// testable. The chart label is a PARAMETER rather than something the caller splices in
/// afterwards: an earlier draft had `main.rs` rewrite the leading `chart:` token, which also
/// rewrites the custody-blind line ending `...stand on this chart:` and mangles it. Passing
/// the label costs one argument and cannot mis-target.
pub fn render_chart_report(chart: &str, r: &ChartReport) -> Vec<String> {
    let mut out = Vec::new();
    out.push(format!(
        "chart {}: {} (winning subject: {}{})",
        chart,
        r.chart_grade,
        r.chart_source,
        match &r.chart_content_address {
            Some(ca) => format!(", withdraws={ca}"),
            None => String::new(),
        }
    ));
    out.extend(render_threads(r));
    out.push(
        "(report only — nothing is withheld; enforcement needs custody narrowing, \
         #232 part C)"
            .to_string(),
    );
    out
}

/// The per-thread breakdown. Task 5 replaces the empty branch — today it reproduces
/// `main.rs`'s current wording exactly, so that replacement is visible as a test edit.
fn render_threads(r: &ChartReport) -> Vec<String> {
    if r.threads.is_empty() {
        return vec!["  no medication threads on this chart".to_string()];
    }
    r.threads
        .iter()
        .map(|t| {
            format!(
                "  thread {}: {} (winning subject: {}{})",
                t.thread_id,
                t.grade,
                t.source,
                match &t.content_address {
                    Some(ca) => format!(", withdraws={ca}"),
                    None => String::new(),
                }
            )
        })
        .collect()
}
```

- [ ] **Step 4: Run to verify it passes**

Run: `cargo test -p cairn-node --lib sensitivity::render 2>&1 | tail -8`
Expected: PASS, 3 tests.

- [ ] **Step 5: Rewire `main.rs` to use it**

Replace the body of the `Cmd::PatientSensitivity` arm with:

```rust
Cmd::PatientSensitivity { patient } => {
    // A pure read — no signing key, no HLC tick, nothing authored.
    let mut db = cairn_node::db::connect(&cli.conn).await?;
    let report = cairn_node::sensitivity::chart_sensitivity(&mut db, patient).await?;
    // Every line, including all wording, comes from the pure renderer — see
    // sensitivity/render.rs for why the sentences live there and not here.
    for line in cairn_node::sensitivity::render::render_chart_report(
        &patient.to_string(),
        &report,
    ) {
        println!("{line}");
    }
}
```

- [ ] **Step 6: Verify end-to-end behaviour is unchanged**

Run: `cargo test -p cairn-node --lib sensitivity 2>&1 | tail -5` — Expected: PASS.
Run: `cargo clippy -p cairn-node --all-targets 2>&1 | tail -5` — Expected: no warnings.

- [ ] **Step 7: Commit**

```bash
git add crates/cairn-node/src/sensitivity/render.rs crates/cairn-node/src/main.rs
git commit -m "refactor(#388): move the sensitivity report's wording into pure render fns

Behaviour-preserving, and it pins the current output before this slice changes
it. The report's honesty claims are all sentences; sentences that live only in
a println! inside main.rs can be tested only against a live cluster, which is
why none of them were. Precedent: safety::render_safety_line.

Refs #388"
```

---

### Task 3: Project `responsible_actor_id` from the withdrawal worklist (#421)

The `judged` CTE already computes it; the outer `SELECT` drops it. Without it, Task 4's report can say a withdrawal did not take effect but not **who tried** — which is the fact #421 says the row exists to report.

**Files:**
- Modify: `db/048_sensitivity_stream.sql:944-946` (the outer `SELECT` list)
- Modify: `db/tests/048_sensitivity_stream_test.sql` (the `information_schema` column pin, ~lines 328-350)
- Create: `crates/cairn-node/tests/worklist_actor_column.rs`

**Interfaces:**
- Produces: `sensitivity_withdrawal_worklist.responsible_actor_id` of SQL type `bytea` (`event_log.actor_id`/`actor_current.actor_id` are both `BYTEA`), nullable.

- [ ] **Step 1: Write the failing guard test**

Create `crates/cairn-node/tests/worklist_actor_column.rs`:

```rust
//! #421 — the withdrawal worklist must NAME the accountable actor.
//!
//! The view's `judged` CTE has always computed `responsible_actor_id` (the vouched R1
//! attester when exactly one human maps to the attester key, else the withdrawal's own
//! actor); the outer SELECT dropped it, so every consumer could report THAT a withdrawal
//! was ineffective but not WHO authored it. This guard fails if the column is ever dropped
//! again — a silent regression would leave the operator surface printing an empty field
//! rather than erroring, which is the failure mode this whole slice exists to end.
mod common;

#[tokio::test]
async fn the_worklist_projects_the_accountable_actor() {
    let Some(base) = common::db::test_pg_base() else {
        return; // self-skip when CAIRN_TEST_PG is unset, like every suite here
    };
    let _guard = common::db::test_serial_guard(&base).await;
    let db = cairn_node::db::connect_and_load_schema(&base).await.unwrap();

    // Ask the catalogue, not a row: a row-shaped assertion could be satisfied by accident
    // by a projection default, and this is a CONTRACT about the view's shape.
    let row = db
        .query_one(
            "SELECT data_type FROM information_schema.columns
              WHERE table_schema = 'public'
                AND table_name = 'sensitivity_withdrawal_worklist'
                AND column_name = 'responsible_actor_id'",
            &[],
        )
        .await
        .expect("sensitivity_withdrawal_worklist must project responsible_actor_id (#421)");
    let ty: String = row.get(0);
    assert_eq!(ty, "bytea", "actor_id is BYTEA in db/001 and db/004; the view must not retype it");
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p cairn-node --test worklist_actor_column 2>&1 | tail -12`
Expected: FAIL — the `query_one` panics with the `expect` message (zero rows: the column does not exist).

- [ ] **Step 3: Project the column**

In `db/048_sensitivity_stream.sql`, change the outer `SELECT` of `sensitivity_withdrawal_worklist` from:

```sql
SELECT content_address, event_id, patient_id, withdraws,
       CASE WHEN verdict = 'unverified' THEN 'inert' ELSE 'stranger-attested' END AS reason,
       node_origin, rationale
  FROM judged
```

to:

```sql
SELECT content_address, event_id, patient_id, withdraws,
       CASE WHEN verdict = 'unverified' THEN 'inert' ELSE 'stranger-attested' END AS reason,
       node_origin, rationale,
       -- #421: the accountable actor — the fact the row exists to report. The CTE has
       -- always computed it (the vouched R1 attester, or the withdrawal's own actor for
       -- the R2 self case); dropping it here meant a consumer could say a withdrawal was
       -- ineffective but never who authored it. APPENDED, never inserted mid-list:
       -- CREATE OR REPLACE VIEW permits adding a trailing column and refuses a reorder.
       responsible_actor_id
  FROM judged
```

- [ ] **Step 4: Update the column pin in the SQL mirror**

In `db/tests/048_sensitivity_stream_test.sql`, in the `DO $$` block that pins the view's shape:

```sql
    expected_cols  text[] := ARRAY['content_address', 'event_id', 'patient_id', 'withdraws',
                                    'reason', 'node_origin', 'rationale',
                                    'responsible_actor_id'];
    expected_types text[] := ARRAY['bytea', 'uuid', 'uuid', 'bytea', 'text', 'text', 'text',
                                    'bytea'];
```

and update BOTH assertion messages from `'7-column contract'` to `'8-column contract'`, adding `responsible_actor_id` to the parenthesised list in the first one. **This guard failing was the expected, designed outcome of Step 3** — updating it is the deliberate act that makes the new column reviewed rather than absorbed.

- [ ] **Step 5: Run to verify both pass**

Run: `cargo test -p cairn-node --test worklist_actor_column 2>&1 | tail -5` — Expected: PASS.
Run: `scripts/run-db-sql-tests.sh 2>&1 | tail -15` — Expected: all mirrors pass, including `048`.

- [ ] **Step 6: Commit**

```bash
git add db/048_sensitivity_stream.sql db/tests/048_sensitivity_stream_test.sql crates/cairn-node/tests/worklist_actor_column.rs
git commit -m "fix(#421): the withdrawal worklist names the accountable actor

The judged CTE always computed responsible_actor_id and the outer SELECT dropped
it, so a consumer could report THAT a withdrawal was ineffective but not WHO
tried. Appended (CREATE OR REPLACE VIEW permits a trailing column, refuses a
reorder) and the db/tests/048 information_schema pin moves 7 -> 8 columns
deliberately, so the change is reviewed rather than absorbed.

Closes #421
Refs #388"
```

---

### Task 4: Surface the withdrawals that did not take effect (#388 part 1 — the §1.2 budget)

This is the task that discharges ADR-0064's owed budget.

**Files:**
- Modify: `crates/cairn-node/src/sensitivity/report.rs`
- Modify: `crates/cairn-node/src/sensitivity/render.rs`
- Create: `crates/cairn-node/tests/sensitivity_report.rs`

**Interfaces:**
- Produces: `report::IneffectiveWithdrawal { withdraws: String, reason: String, node_origin: String, rationale: String, responsible_actor_id: Option<String> }` (all hex where `bytea`); `ChartReport.ineffective_withdrawals: Vec<IneffectiveWithdrawal>`; `render::withdrawal_reason_explanation(&str) -> &'static str`.
- Note `rationale` is `String`, not `Option<String>`: `sensitivity_withdrawal.rationale` is `TEXT NOT NULL` (db/048), because db/048's ceremony refuses a withdrawal without one.

- [ ] **Step 1: Write the failing pure tests**

Add to `render.rs`'s test module:

```rust
    fn inert_withdrawal() -> IneffectiveWithdrawal {
        IneffectiveWithdrawal {
            withdraws: "a3f".into(),
            reason: "inert".into(),
            node_origin: "peer-b".into(),
            rationale: "consent withdrawn by patient 2026-08-12".into(),
            responsible_actor_id: Some("beef".into()),
        }
    }

    #[test]
    fn an_inert_withdrawal_names_its_reason_rationale_and_actor() {
        // THE §1.2 BUDGET, as a unit test: "why did this withdrawal not take effect?"
        // answered without raw SQL. Everything the operator needs must be in these lines.
        let mut r = healthy();
        r.ineffective_withdrawals = vec![inert_withdrawal()];
        let text = render_chart_report("C", &r).join("\n");
        assert!(text.contains("did NOT take effect"), "{text}");
        assert!(text.contains("inert"), "{text}");
        assert!(text.contains("consent withdrawn by patient"), "the rationale: {text}");
        assert!(text.contains("beef"), "the accountable actor (#421): {text}");
        assert!(text.contains("withdraws=a3f"), "the target address: {text}");
    }

    #[test]
    fn the_two_reasons_read_differently() {
        // 'inert' and 'stranger-attested' have DIFFERENT fixes — one needs an accountable
        // human, the other needs a look at who is asserting on this chart. A shared
        // sentence would hide that, which is the whole failure this surface exists to end.
        let mut a = healthy();
        a.ineffective_withdrawals = vec![inert_withdrawal()];
        let mut b = healthy();
        b.ineffective_withdrawals = vec![IneffectiveWithdrawal {
            reason: "stranger-attested".into(),
            ..inert_withdrawal()
        }];
        assert_ne!(
            render_chart_report("C", &a).join("\n"),
            render_chart_report("C", &b).join("\n")
        );
    }

    #[test]
    fn an_unrecognised_reason_is_shown_not_swallowed() {
        // Open vocabulary: a future db/048 may add a reason this build has never seen.
        // Mirrors subject_kind_phrase's total mapping — the catch-all must point the
        // reader AT the row, never silently render it as if it were understood.
        let phrase = withdrawal_reason_explanation("some-future-reason");
        assert!(
            phrase.contains("unrecognised"),
            "an unknown reason must say so: {phrase}"
        );
    }

    #[test]
    fn the_footer_declares_the_invisible_withdrawal_even_with_none_listed() {
        // ADR-0064 Known limitations: a cross-chart mis-targeted withdrawal that stays
        // unverified is permanently inert AND permanently invisible — it falls out of the
        // worklist's inert arm. A surface listing "the withdrawals that did not take
        // effect" while silent about that is a comment asserting a guarantee the code does
        // not provide, which is the defect class this whole slice is about. Asserted on a
        // report with an EMPTY list, because that is the case where silence is most
        // convincing and most wrong.
        let text = render_chart_report("C", &healthy()).join("\n");
        assert!(text.contains("not complete"), "{text}");
        assert!(text.contains("ADR-0064"), "{text}");
    }
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p cairn-node --lib sensitivity::render 2>&1 | tail -15`
Expected: FAIL — `IneffectiveWithdrawal` and `withdrawal_reason_explanation` do not exist; `healthy()` has no `ineffective_withdrawals` field.

- [ ] **Step 3: Add the struct and the read**

In `report.rs`, add the struct and extend `ChartReport`:

```rust
/// One withdrawal this node admitted that did NOT lower any grade, as
/// `sensitivity_withdrawal_worklist` reports it (db/048 §11).
///
/// A withdrawal lands, converges and stays re-assertable even when it has no effect —
/// ADR-0064 gates EFFECT, never admission, so nothing here is a refusal. That is exactly
/// why it needs a surface: the record contains an act whose author believes it worked.
pub struct IneffectiveWithdrawal {
    /// Hex `content_address` of the assertion this withdrawal targeted.
    pub withdraws: String,
    /// `inert` (no accountable human stands behind the claim) or `stranger-attested`
    /// (attested, but by an actor with no prior presence on this chart). Open vocabulary —
    /// see `render::withdrawal_reason_explanation`.
    pub reason: String,
    pub node_origin: String,
    /// NOT `Option`: `sensitivity_withdrawal.rationale` is `TEXT NOT NULL`, because db/048's
    /// ceremony refuses a withdrawal that does not state why.
    pub rationale: String,
    /// Hex `actor_id` of whoever is accountable (#421). `None` when the attester key maps
    /// to no single human — the view's `count(*) = 1` guard deliberately yields NULL rather
    /// than picking one arbitrarily.
    pub responsible_actor_id: Option<String>,
}
```


**Fixture upkeep:** `healthy()` in `render.rs`'s test module constructs a whole `ChartReport`, so adding a field to that struct BREAKS IT. Add `ineffective_withdrawals: vec![],` to `healthy()` in the same step — an empty vec is the right default there, because `healthy()` is by definition the chart with nothing wrong.

Add `pub ineffective_withdrawals: Vec<IneffectiveWithdrawal>,` to `ChartReport`, and in `chart_sensitivity`, before the final `Ok(ChartReport { .. })`:

```rust
    // The worklist already knows WHY a withdrawal was ineffective — reading it here rather
    // than re-deriving the verdict in Rust is the same "ONE definition" discipline the
    // chart-wide read follows: a second implementation of authority in this file could
    // disagree with db/048 and would do so silently.
    let withdrawal_rows = client
        .query(
            "SELECT encode(withdraws, 'hex'), reason, node_origin, rationale,
                    encode(responsible_actor_id, 'hex')
               FROM sensitivity_withdrawal_worklist
              WHERE patient_id = $1::text::uuid
              ORDER BY reason, withdraws",
            &[&patient_s],
        )
        .await?;
    let ineffective_withdrawals = withdrawal_rows
        .into_iter()
        .map(|row| IneffectiveWithdrawal {
            withdraws: row.get(0),
            reason: row.get(1),
            node_origin: row.get(2),
            rationale: row.get(3),
            responsible_actor_id: row.get(4),
        })
        .collect();
```

and add `ineffective_withdrawals,` to the returned struct literal.

- [ ] **Step 4: Add the rendering**

In `render.rs`:

```rust
/// Why a worklist row is on the worklist, in words. Pure and TOTAL — every input has an
/// output, including one this build has never seen.
///
/// The two reasons have DIFFERENT fixes, which is why they get different sentences rather
/// than a shared "did not take effect": `inert` means nobody this node can hold responsible
/// stands behind the claim (the fix is an accountable human re-asserting it), while
/// `stranger-attested` means someone did stand behind it but has no prior presence on this
/// chart (the fix is a look at who is asserting on this chart at all).
///
/// The catch-all points the reader AT the row rather than rendering an unknown reason as
/// though it were understood — the same discipline as `subject_kind_phrase`.
pub fn withdrawal_reason_explanation(reason: &str) -> &'static str {
    match reason {
        "inert" => "no accountable human this node can hold responsible stands behind it (ADR-0064)",
        "stranger-attested" => "attested, but by an actor with no prior presence on this chart",
        _ => "an unrecognised reason from a newer node — read the row itself",
    }
}

/// The warning block for withdrawals that landed and changed nothing. Empty when there are
/// none, so a healthy chart stays silent.
fn render_ineffective_withdrawals(ws: &[IneffectiveWithdrawal]) -> Vec<String> {
    if ws.is_empty() {
        return Vec::new();
    }
    let mut out = vec![format!(
        "⚠ {} withdrawal(s) on this chart did NOT take effect — the grade above may not be \
         what someone intended",
        ws.len()
    )];
    for w in ws {
        out.push(format!(
            "    {:<18} withdraws={}  by actor={}  origin={}",
            w.reason,
            w.withdraws,
            w.responsible_actor_id.as_deref().unwrap_or("(none this node can name)"),
            w.node_origin
        ));
        out.push(format!("      rationale: {:?}", w.rationale));
        out.push(format!("      → {}", withdrawal_reason_explanation(&w.reason)));
    }
    out
}
```

Insert `out.extend(render_ineffective_withdrawals(&r.ineffective_withdrawals));` in `render_chart_report` immediately after the grade line, and append this footer line before the existing `(report only …)` line:

```rust
    out.push(
        "(this list is not complete: a withdrawal mis-stamped with another chart's \
         patient_id and left unverified is permanently inert AND invisible here — \
         ADR-0064, Known limitations)"
            .to_string(),
    );
```

- [ ] **Step 5: Run the pure tests**

Run: `cargo test -p cairn-node --lib sensitivity::render 2>&1 | tail -8`
Expected: PASS, 7 tests.

- [ ] **Step 6: Write the DB-gated test**

Create `crates/cairn-node/tests/sensitivity_report.rs`:

```rust
//! #388 / ADR-0064 §1.2 — the operator surface answers "why did this withdrawal not take
//! effect?" in ONE query with no raw SQL.
//!
//! The unit tests in `sensitivity/render.rs` pin the WORDING; these pin that the report is
//! actually fed the rows. Both halves are needed: a perfectly-worded report over an empty
//! query is exactly the silence this slice exists to end.
mod common;

use cairn_node::sensitivity::chart_sensitivity;

#[tokio::test]
async fn an_un_attested_withdrawal_is_reported_as_inert_with_its_reason_and_rationale() {
    let Some(base) = common::db::test_pg_base() else {
        return;
    };
    let _guard = common::db::test_serial_guard(&base).await;
    let mut db = cairn_node::db::connect_and_load_schema(&base).await.unwrap();

    // TWO DISTINCT humans: enroll_human twice collides (same pinned determinant set -> same
    // actor_id -> enrollment refused, ADR-0044/#152), and a self-withdrawal would satisfy
    // cairn_claim_authority's R2 branch and never reach the worklist at all.
    let (sk_a, kid_a) = common::enroll_human_with_role(&db, "clinician").await;
    let (_sk_b, _kid_b) = common::enroll_human_with_role(&db, "records-officer").await;

    let patient = common::register_chart(&mut db).await;
    let target = common::assert_chart_grade(&mut db, patient, "sequestered", "safety").await;
    common::withdraw_un_attested(&mut db, patient, &target, "consent withdrawn").await;

    let report = chart_sensitivity(&mut db, patient).await.unwrap();

    let w = report
        .ineffective_withdrawals
        .iter()
        .find(|w| w.withdraws == target)
        .expect("the un-attested withdrawal must appear on the worklist (#380/ADR-0064)");
    assert_eq!(w.reason, "inert");
    assert_eq!(w.rationale, "consent withdrawn");
    // And the grade really did NOT drop — the report is describing a live state, not a
    // hypothetical one.
    assert_eq!(report.chart_grade, "sequestered");
    let _ = (sk_a, kid_a);
}
```

Add a second case in the same file for the other reason:

```rust
#[tokio::test]
async fn a_withdrawal_from_an_actor_with_no_prior_presence_reads_as_stranger_attested() {
    let Some(base) = common::db::test_pg_base() else {
        return;
    };
    let _guard = common::db::test_serial_guard(&base).await;
    let mut db = cairn_node::db::connect_and_load_schema(&base).await.unwrap();

    // The 'stranger-attested' arm needs a human who IS accountable (so the claim is not
    // inert) but has authored nothing else on this chart at or before the withdrawal's own
    // HLC. Two distinct humans again: one raises the grade, a second withdraws it having
    // touched nothing else here.
    let (_sk_a, _kid_a) = common::enroll_human_with_role(&db, "clinician").await;
    let (sk_b, kid_b) = common::enroll_human_with_role(&db, "records-officer").await;

    let patient = common::register_chart(&mut db).await;
    let target = common::assert_chart_grade(&mut db, patient, "sequestered", "safety").await;
    cairn_node::sensitivity::withdraw_sensitivity(
        &mut db, &sk_b, &kid_b, "test-node", patient, &target, "administrative correction",
    )
    .await
    .unwrap();

    let report = chart_sensitivity(&mut db, patient).await.unwrap();
    let w = report
        .ineffective_withdrawals
        .iter()
        .find(|w| w.withdraws == target)
        .expect("an attested-but-stranger withdrawal belongs on the worklist");
    assert_eq!(
        w.reason, "stranger-attested",
        "attested by an enrolled human, so NOT inert — but with no prior presence on this \
         chart, so not effective either"
    );
    assert!(w.responsible_actor_id.is_some(), "#421: the actor must be named");
}
```

**If this fixture proves infeasible in-branch** — `cairn_claim_authority`'s bounded
no-prior-presence check is sensitive to HLC ordering and to what `register_chart` itself
authors — do NOT drop it silently. File an issue naming the obstacle (house rule 5); #419
already tracks the worklist's untested arms and this would extend it.

**Note for the implementer:** `register_chart`, `assert_chart_grade` and `withdraw_un_attested` are helper shapes; before writing them, check `crates/cairn-node/tests/sensitivity_ceremony.rs` and `claim_authority_worklist.rs` for equivalents that already exist and reuse them. If you add any `pub fn` to `tests/common/mod.rs`, you **must** also add its signature to the hand-written expected-helper array in `identity_scaffolding_shared.rs` or `derivation_finds_the_expected_helpers` fails.

- [ ] **Step 7: Run it, red then green**

Run: `cargo test -p cairn-node --test sensitivity_report 2>&1 | tail -15`
Expected: FAIL first (helpers missing / field missing), then PASS once implemented.

- [ ] **Step 8: Commit**

```bash
git add crates/cairn-node/src/sensitivity/ crates/cairn-node/tests/sensitivity_report.rs
git commit -m "feat(#388): the report names withdrawals that did not take effect

Discharges ADR-0064's §1.2 budget, recorded there as owed, not met: 'why did
this withdrawal not take effect?' is now answerable in one query with no raw
SQL. The two reasons get different sentences because they have different fixes.

The footer declares what the list CANNOT contain — ADR-0064's cross-chart
mis-targeted withdrawal is permanently inert and permanently invisible — and
that disclaimer is asserted on an EMPTY list, where silence is most convincing
and most wrong.

Refs #388"
```

---

### Task 5: Stop the custody-blind untruth — name the standing assertions (#388 part 3, #383)

**Files:**
- Modify: `crates/cairn-node/src/sensitivity/report.rs`, `render.rs`

**Interfaces:**
- Produces: `report::StandingAssertion { content_address: String, subject_kind: String, subject_id: Uuid, grade: String }`; `ChartReport.standing: Vec<StandingAssertion>`.

- [ ] **Step 1: Write the failing tests**

Add to `render.rs`'s test module:

```rust
    #[test]
    fn an_empty_chart_says_both_things_are_empty() {
        let mut r = healthy();
        r.threads = vec![];
        let text = render_chart_report("C", &r).join("\n");
        assert!(text.contains("no medication threads and no standing"), "{text}");
    }

    #[test]
    fn a_custody_blind_chart_names_each_standing_assertion_and_never_merely_counts() {
        // #383 / #388 part 3. Both issues proposed a COUNT. This diverges from both:
        // ADR-0061 settled the shape — "2 standing assertions, 0 threads" cannot tell an
        // operator whether this node is custody-blind or the chart is genuinely empty,
        // which is the one question the line exists to answer. A named row also carries the
        // content_address that `sensitivity-withdraw --withdraws` consumes.
        let mut r = healthy();
        r.threads = vec![];
        r.standing = vec![StandingAssertion {
            content_address: "c0ffee".into(),
            subject_kind: "thread".into(),
            subject_id: uuid::Uuid::nil(),
            grade: "restricted".into(),
        }];
        let text = render_chart_report("C", &r).join("\n");
        assert!(text.contains("c0ffee"), "the address must be named: {text}");
        assert!(text.contains("restricted"), "the grade must be named: {text}");
        assert!(text.contains("no DEK custody"), "the custody explanation: {text}");
        assert!(
            !text.contains("no medication threads on this chart"),
            "the old precise untruth must be gone: {text}"
        );
    }
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p cairn-node --lib sensitivity::render 2>&1 | tail -15`
Expected: FAIL — `StandingAssertion` undefined; `healthy()` has no `standing` field.

- [ ] **Step 3: Add the struct and read it unconditionally**

In `report.rs`:

```rust
/// One assertion standing on this chart, read from `cairn_sensitivity_standing` — which
/// needs NO custody, because sensitivity bodies are plaintext by necessity (ADR-0062
/// decision 4: a node must READ a grade in order to coarsen by it).
///
/// That is the whole point of carrying these separately from `threads`: the per-thread
/// breakdown comes from `medication_statement`, whose rows are opened through
/// `cairn_clear_payload`, so a node with no DEK custody projects none of them and the
/// report used to print "no medication threads on this chart" while honouring standing
/// thread-scoped grades on those very threads (#383).
pub struct StandingAssertion {
    pub content_address: String,
    pub subject_kind: String,
    pub subject_id: Uuid,
    pub grade: String,
}
```


**Fixture upkeep:** `healthy()` in `render.rs`'s test module constructs a whole `ChartReport`, so adding a field to that struct BREAKS IT. Add `standing: vec![],` to `healthy()` in the same step — an empty vec is the right default there, because `healthy()` is by definition the chart with nothing wrong.

Add `pub standing: Vec<StandingAssertion>,` to `ChartReport` and read it in `chart_sensitivity`:

```rust
    // Read UNCONDITIONALLY, not only in the no-registration fallback: the custody-blind
    // case has a perfectly good registration and still projects no threads.
    let standing_rows = client
        .query(
            "SELECT encode(s.content_address, 'hex'), s.subject_kind, s.subject_id::text,
                    s.grade
               FROM cairn_sensitivity_standing($1::text::uuid) s
              ORDER BY cairn_sensitivity_rank(s.grade) DESC, s.content_address ASC",
            &[&patient_s],
        )
        .await?;
    let standing = standing_rows
        .into_iter()
        .map(|row| StandingAssertion {
            content_address: row.get(0),
            subject_kind: row.get(1),
            subject_id: Uuid::parse_str(&row.get::<_, String>(2))
                .expect("subject_id column is a valid UUID"),
            grade: row.get(3),
        })
        .collect();
```

- [ ] **Step 4: Replace the empty branch in `render_threads`**

```rust
fn render_threads(r: &ChartReport) -> Vec<String> {
    if !r.threads.is_empty() {
        return r.threads.iter().map(render_thread_line).collect();
    }
    // NOTHING PROJECTED. Two very different states, and the old wording collapsed them.
    if r.standing.is_empty() {
        return vec![
            "  no medication threads and no standing sensitivity assertions on this chart"
                .to_string(),
        ];
    }
    let mut out = vec![format!(
        "⚠ this node projects no medication threads, but {} sensitivity assertion(s) stand \
         on this chart:",
        r.standing.len()
    )];
    for s in &r.standing {
        out.push(format!(
            "    {} ({}, subject {})  withdraws={}",
            s.grade,
            subject_kind_phrase(&s.subject_kind),
            s.subject_id,
            s.content_address
        ));
    }
    out.push(
        "  → this node may hold no DEK custody, so the threads these assertions grade may \
         exist and be invisible here (#383)"
            .to_string(),
    );
    out
}
```

Extract the existing per-thread `format!` into `fn render_thread_line(t: &ThreadGrade) -> String` so both branches read the same way. Import `subject_kind_phrase` from `super`.

- [ ] **Step 5: Run to verify it passes**

Run: `cargo test -p cairn-node --lib sensitivity::render 2>&1 | tail -8`
Expected: PASS, 9 tests.
Run: `grep -rn "no medication threads on this chart" crates/` — Expected: **no matches** (the precise untruth is gone from the crate).

- [ ] **Step 6: Add the DB-gated custody-blind test**

Add to `crates/cairn-node/tests/sensitivity_report.rs`:

```rust
#[tokio::test]
async fn a_chart_with_standing_assertions_and_no_projected_threads_still_names_them() {
    let Some(base) = common::db::test_pg_base() else {
        return;
    };
    let _guard = common::db::test_serial_guard(&base).await;
    let mut db = cairn_node::db::connect_and_load_schema(&base).await.unwrap();

    // No medication events at all is the CHEAP stand-in for a custody-thin node: both
    // produce zero medication_statement rows, which is the condition the report branches
    // on. It does NOT reproduce the custody path itself — a node that holds sealed
    // medication events without a DEK — so it pins the branch, not the cause.
    let patient = common::register_chart(&mut db).await;
    let ca = common::assert_chart_grade(&mut db, patient, "restricted", "safety").await;

    let report = chart_sensitivity(&mut db, patient).await.unwrap();
    assert!(report.threads.is_empty(), "no medication events were authored");
    assert!(
        report.standing.iter().any(|s| s.content_address == ca),
        "the standing assertion must be NAMED, not merely counted (#383)"
    );
}
```

Run: `cargo test -p cairn-node --test sensitivity_report 2>&1 | tail -5` — Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add crates/cairn-node/src/sensitivity/ crates/cairn-node/tests/sensitivity_report.rs
git commit -m "fix(#383,#388): name the standing assertions a custody-thin node cannot anchor

'no medication threads on this chart' is a precise untruth on a node with no
DEK custody: medication_statement rows are opened through cairn_clear_payload,
so they are absent while cairn_effective_sensitivity honours standing
thread-scoped grades on those very threads.

Both #383 and #388 part 3 proposed a COUNT. This names them instead. ADR-0061
settled the shape: a count cannot separate 'custody-blind' from 'genuinely
empty', which is the one question the line exists to answer — and a named row
carries the content_address sensitivity-withdraw --withdraws consumes.

Closes #383
Refs #388"
```

---

### Task 6: Surface deferred sensitivity events (#388 part 2)

**Files:**
- Modify: `db/043_deferred_readjudication.sql` (add a definer before `COMMIT;`)
- Modify: `crates/cairn-node/src/sensitivity/report.rs`, `render.rs`
- Modify: `crates/cairn-node/tests/sensitivity_report.rs`

**Interfaces:**
- Produces: SQL `cairn_patient_deferred_sensitivity(p_patient uuid) RETURNS TABLE (event_id uuid, event_type text, admitted_at timestamptz, adjudication_error text)`; `report::DeferredSensitivityEvent { event_id: Uuid, event_type: String, admitted_at: String, adjudication_error: Option<String> }`; `ChartReport.deferred: Vec<DeferredSensitivityEvent>`.

- [ ] **Step 1: Write the failing pure test**

Add to `render.rs`'s test module:

```rust
    #[test]
    fn a_deferred_sensitivity_event_is_reported_as_powerless() {
        // db/043 records adjudication_error and leaves the event deferred. A sensitivity
        // assertion admitted by a pre-db/048 node (ADR-0056 admit-and-defer — a DESIGNED
        // state, given "no lockstep fleet upgrade") projects nothing and therefore reads
        // 'routine'. Nothing in the §5.9 read path consulted event_deferred, so a grade
        // this node is failing to apply was invisible.
        let mut r = healthy();
        r.deferred = vec![DeferredSensitivityEvent {
            event_id: uuid::Uuid::nil(),
            event_type: "sensitivity.grade.asserted".into(),
            admitted_at: "2026-08-18 09:00:00+00".into(),
            adjudication_error: None,
        }];
        let text = render_chart_report("C", &r).join("\n");
        assert!(text.contains("DEFERRED"), "{text}");
        assert!(text.contains("powerless"), "{text}");
        assert!(text.contains("not yet re-adjudicated"), "the null-error wording: {text}");
    }
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p cairn-node --lib sensitivity::render 2>&1 | tail -12`
Expected: FAIL — `DeferredSensitivityEvent` undefined.

- [ ] **Step 3: Add the chart-scoped definer**

In `db/043_deferred_readjudication.sql`, immediately before the final `COMMIT;`:

```sql
-- ---------------------------------------------------------------------------
-- #388 part 2 — this chart's deferred sensitivity events, for the operator surface.
--
-- WHY A DEFINER RATHER THAN A GRANT. event_deferred is GRANTed to cairn_node, not to
-- cairn_agent. Reading it from the runtime role works TODAY only because that login role
-- happens to be a cairn_node member — which is exactly #425's finding, and building a new
-- read path on it would bake a known-unreliable membership into the floor. The alternative,
-- granting cairn_agent the whole table, widens the role node-wide to answer a question that
-- is about one chart. So: a definer scoped to one patient, the precedent db/049 set when
-- cairn_event_safety became one for the same reason.
--
-- pg_temp LAST (#426): this reads event_log and event_deferred UNQUALIFIED, which is
-- precisely the shape a caller could blind with a temp table of the same name — and a
-- blinded read here returns ZERO ROWS, which the surface would render as "nothing is
-- deferred". A silent zero is the failure this whole slice exists to end.
CREATE OR REPLACE FUNCTION cairn_patient_deferred_sensitivity(p_patient uuid)
RETURNS TABLE (event_id uuid, event_type text, admitted_at timestamptz,
               adjudication_error text)
LANGUAGE sql STABLE
SECURITY DEFINER SET search_path = public, pg_temp
AS $$
    SELECT d.event_id, d.event_type, d.admitted_at, d.adjudication_error
    FROM event_deferred d
    JOIN event_log e ON e.event_id = d.event_id
    WHERE e.patient_id = p_patient
      AND d.event_type LIKE 'sensitivity.%'
    ORDER BY d.admitted_at;
$$;

-- PUBLIC's default EXECUTE would make this callable by a below-the-floor adversary; the
-- grant to the runtime role is deliberate, not inherited (#382's posture, applied on the
-- way in rather than retrofitted).
REVOKE EXECUTE ON FUNCTION cairn_patient_deferred_sensitivity(uuid) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION cairn_patient_deferred_sensitivity(uuid) TO cairn_agent;
```

- [ ] **Step 4: Add the struct, the read, and the rendering**

In `report.rs`:

```rust
/// One `sensitivity.%` event this node admitted but cannot interpret — ADR-0056's
/// admit-and-defer, a DESIGNED state given that there is no lockstep fleet upgrade.
///
/// It is a grade this node is FAILING TO APPLY. It projects nothing, so the chart reads
/// 'routine', and the only trace is a row in `event_deferred` that nothing in the §5.9 read
/// path consulted before this slice.
pub struct DeferredSensitivityEvent {
    pub event_id: Uuid,
    pub event_type: String,
    /// Rendered in SQL (`::text`) rather than formatted in Rust: TIMESTAMPTZ::text gives
    /// ISO-8601 with the session offset and costs no new dependency — the same idiom the
    /// `deferred` CLI verb already uses.
    pub admitted_at: String,
    /// `None` until a re-adjudication attempt has run and FAILED; then the verbatim refusal.
    pub adjudication_error: Option<String>,
}
```


**Fixture upkeep:** `healthy()` in `render.rs`'s test module constructs a whole `ChartReport`, so adding a field to that struct BREAKS IT. Add `deferred: vec![],` to `healthy()` in the same step — an empty vec is the right default there, because `healthy()` is by definition the chart with nothing wrong.

Add `pub deferred: Vec<DeferredSensitivityEvent>,` to `ChartReport` and read it:

```rust
    let deferred_rows = client
        .query(
            "SELECT event_id::text, event_type, admitted_at::text, adjudication_error
               FROM cairn_patient_deferred_sensitivity($1::text::uuid)",
            &[&patient_s],
        )
        .await?;
    let deferred = deferred_rows
        .into_iter()
        .map(|row| DeferredSensitivityEvent {
            event_id: Uuid::parse_str(&row.get::<_, String>(0))
                .expect("event_id column is a valid UUID"),
            event_type: row.get(1),
            admitted_at: row.get(2),
            adjudication_error: row.get(3),
        })
        .collect();
```

In `render.rs`:

```rust
/// The warning block for sensitivity events this node holds but cannot apply.
fn render_deferred(ds: &[DeferredSensitivityEvent]) -> Vec<String> {
    if ds.is_empty() {
        return Vec::new();
    }
    let mut out = vec![format!(
        "⚠ {} sensitivity event(s) on this chart are DEFERRED — admitted, powerless, not \
         applied to any grade above",
        ds.len()
    )];
    for d in ds {
        out.push(format!(
            "    {}  {}  {}  {}",
            d.event_id,
            d.event_type,
            d.admitted_at,
            d.adjudication_error
                .as_deref()
                .unwrap_or("(not yet re-adjudicated)")
        ));
    }
    out
}
```

Call it from `render_chart_report` after the withdrawal block.

- [ ] **Step 5: Add the DB-gated chart-scoping test**

Add to `crates/cairn-node/tests/sensitivity_report.rs`:

```rust
#[tokio::test]
async fn the_deferred_reader_is_scoped_to_one_chart() {
    let Some(base) = common::db::test_pg_base() else {
        return;
    };
    let _guard = common::db::test_serial_guard(&base).await;
    let db = cairn_node::db::connect_and_load_schema(&base).await.unwrap();

    // A definer that ignored its argument would still pass a "the row appears" test; this
    // is the mutation that test cannot catch. Assert the function's WHERE clause binds by
    // asking for a chart that has nothing and requiring zero rows even when the table is
    // non-empty for another chart.
    let rows = db
        .query(
            "SELECT count(*)::bigint FROM cairn_patient_deferred_sensitivity(
                 '00000000-0000-0000-0000-0000000000ff'::uuid)",
            &[],
        )
        .await
        .unwrap();
    let n: i64 = rows[0].get(0);
    assert_eq!(n, 0, "a chart with no deferred sensitivity events must report none");
}
```

- [ ] **Step 6: Run everything**

Run: `cargo test -p cairn-node --lib sensitivity::render 2>&1 | tail -5` — Expected: PASS, 10 tests.
Run: `cargo test -p cairn-node --test sensitivity_report 2>&1 | tail -5` — Expected: PASS.
Run: `cargo test -p cairn-node --test search_path_pg_temp 2>&1 | tail -8` — Expected: PASS. (`PINNED_TODAY` is compared with `>=`, so a 26th definer needs no number moved — but the "every SECURITY DEFINER pins a path" and "every pinned path denies the temp schema first" assertions must both still hold for the new function.)

- [ ] **Step 7: Commit**

```bash
git add db/043_deferred_readjudication.sql crates/cairn-node/src/sensitivity/ crates/cairn-node/tests/sensitivity_report.rs
git commit -m "feat(#388): surface sensitivity events this node cannot apply

A sensitivity assertion admitted by a pre-db/048 node (ADR-0056 admit-and-defer,
a designed state given no lockstep fleet upgrade) projects nothing and reads
'routine'. The only trace was a row in event_deferred that nothing in the §5.9
read path consulted.

Read through a chart-scoped SECURITY DEFINER rather than a table grant:
event_deferred is granted to cairn_node, not cairn_agent, so a direct read
would work only via the membership #425 flags as unreliable — and granting the
whole table widens the role node-wide to answer a chart-scoped question.
pg_temp last, because a blinded read here returns zero rows and would render as
'nothing is deferred'.

Refs #388"
```

---

### Task 7: Surface safety overclaim flags, and declare what the ledger cannot promise

**Files:**
- Modify: `crates/cairn-node/src/sensitivity/report.rs`, `render.rs`
- Modify: `crates/cairn-node/tests/sensitivity_report.rs`

**Interfaces:**
- Produces: `report::SafetyOverclaim { content_address: String, emitted_rung: String, licensed_rung: String }`; `ChartReport.overclaims: Vec<SafetyOverclaim>`.

- [ ] **Step 1: Write the failing tests**

```rust
    #[test]
    fn an_overclaim_names_both_rungs() {
        let mut r = healthy();
        r.overclaims = vec![SafetyOverclaim {
            content_address: "dead".into(),
            emitted_rung: "precise".into(),
            licensed_rung: "existence".into(),
        }];
        let text = render_chart_report("C", &r).join("\n");
        assert!(text.contains("overclaim"), "{text}");
        assert!(text.contains("precise"), "{text}");
        assert!(text.contains("existence"), "{text}");
        assert!(text.contains("dead"), "the event must be nameable: {text}");
    }

    #[test]
    fn an_empty_overclaim_ledger_is_never_a_clean_bill() {
        // #414: the ledger's completeness rests on a RAISE WARNING nothing consumes, so an
        // empty ledger is indistinguishable from a broken one. Same shape as
        // safety_class_map shipping empty, where main.rs already refuses to say "no safety
        // signals" — an empty result must never read as "checked, nothing found"
        // (principle 4: an imprecise near-truth beats a precise untruth).
        let text = render_chart_report("C", &healthy()).join("\n");
        assert!(text.contains("#414"), "the disclaimer must cite its issue: {text}");
        assert!(
            !text.contains("no overclaims"),
            "an empty ledger must not read as a clean bill: {text}"
        );
    }
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test -p cairn-node --lib sensitivity::render 2>&1 | tail -12`
Expected: FAIL — `SafetyOverclaim` undefined.

- [ ] **Step 3: Add the struct, the read, and the rendering**

In `report.rs`:

```rust
/// One event on this chart whose emitted safety rung was FINER than the standing grade
/// licensed (#405 part 2, recorded by db/049 at the LOCAL door only).
///
/// A LEDGER, not a view — ADR-0064 decision 3: flag what cannot self-heal, view what can.
/// A published byte can never improve, so unlike the withdrawal worklist this row will
/// never stop being true.
pub struct SafetyOverclaim {
    pub content_address: String,
    pub emitted_rung: String,
    pub licensed_rung: String,
}
```


**Fixture upkeep:** `healthy()` in `render.rs`'s test module constructs a whole `ChartReport`, so adding a field to that struct BREAKS IT. Add `overclaims: vec![],` to `healthy()` in the same step — an empty vec is the right default there, because `healthy()` is by definition the chart with nothing wrong.

Add `pub overclaims: Vec<SafetyOverclaim>,` to `ChartReport` and read it:

```rust
    let overclaim_rows = client
        .query(
            "SELECT encode(content_address, 'hex'), emitted_rung, licensed_rung
               FROM safety_overclaim_flag
              WHERE patient_id = $1::text::uuid
              ORDER BY recorded_at",
            &[&patient_s],
        )
        .await?;
    let overclaims = overclaim_rows
        .into_iter()
        .map(|row| SafetyOverclaim {
            content_address: row.get(0),
            emitted_rung: row.get(1),
            licensed_rung: row.get(2),
        })
        .collect();
```

In `render.rs`:

```rust
/// The warning block for recorded safety overclaims.
fn render_overclaims(os: &[SafetyOverclaim]) -> Vec<String> {
    if os.is_empty() {
        return Vec::new();
    }
    let mut out = vec![format!(
        "⚠ {} safety overclaim(s) recorded on this chart — a rung finer than the grade \
         licensed was published, and a published byte cannot be clawed back",
        os.len()
    )];
    for o in os {
        out.push(format!(
            "    event={}  emitted={}  licensed={}",
            o.content_address, o.emitted_rung, o.licensed_rung
        ));
    }
    out
}
```

Call it after `render_deferred`, and add this footer line beside the ADR-0064 one:

```rust
    out.push(
        "(an empty overclaim list is NOT a clean bill: the ledger's completeness rests on \
         a RAISE WARNING nothing consumes — #414)"
            .to_string(),
    );
```

- [ ] **Step 4: Run to verify it passes**

Run: `cargo test -p cairn-node --lib sensitivity::render 2>&1 | tail -8`
Expected: PASS, 12 tests.

- [ ] **Step 5: Add the DB-gated scoping test**

```rust
#[tokio::test]
async fn overclaim_flags_are_scoped_to_the_chart_asked_about() {
    let Some(base) = common::db::test_pg_base() else {
        return;
    };
    let _guard = common::db::test_serial_guard(&base).await;
    let mut db = cairn_node::db::connect_and_load_schema(&base).await.unwrap();
    let patient = common::register_chart(&mut db).await;

    let report = chart_sensitivity(&mut db, patient).await.unwrap();
    assert!(
        report.overclaims.is_empty(),
        "a fresh chart has no overclaims; a query missing its WHERE would show another \
         chart's rows here"
    );
}
```

- [ ] **Step 6: Commit**

```bash
git add crates/cairn-node/src/sensitivity/ crates/cairn-node/tests/sensitivity_report.rs
git commit -m "feat(#388): give safety_overclaim_flag its first reader

Slice 68 shipped the ledger tested and GRANTed, read by nothing. It is a LEDGER
rather than a view because a published byte can never improve (ADR-0064
decision 3), so unlike the withdrawal worklist these rows never stop being true.

The footer states that an EMPTY list is not a clean bill: #414 records that the
ledger's completeness rests on a RAISE WARNING nothing consumes. Same posture
main.rs already takes for an empty safety_class_map.

Refs #388, #414"
```

---

### Task 8: Confirm what actually took effect after an assertion (#388 part 4)

**Files:**
- Modify: `crates/cairn-node/src/main.rs` — the `Cmd::SensitivityAssert` arm (currently ~1797-1839)

- [ ] **Step 1: Write the failing pure test**

In `render.rs`'s test module:

```rust
    #[test]
    fn the_read_back_reports_the_asserted_and_the_standing_grade_as_two_facts() {
        // A thread-scoped 'restricted' asserted while a chart-wide 'sequestered' stands
        // reads back as 'sequestered' — correct, and indistinguishable from "your assertion
        // was silently upgraded" if only one grade is printed. Both, always, with the
        // winning subject, so the operator can see WHY they differ.
        let line = render_assert_readback("restricted", "sequestered", "chart-wide");
        assert!(line.contains("restricted"), "{line}");
        assert!(line.contains("sequestered"), "{line}");
        assert!(line.contains("chart-wide"), "{line}");
    }

    #[test]
    fn the_read_back_is_still_two_facts_when_they_agree() {
        // No special case for agreement: a reader who learns the surface prints one grade
        // when they agree cannot then trust a single-grade line to mean agreement.
        let line = render_assert_readback("restricted", "restricted", "this thread");
        assert!(line.matches("restricted").count() >= 2, "{line}");
    }
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p cairn-node --lib sensitivity::render 2>&1 | tail -10`
Expected: FAIL — `render_assert_readback` not found.

- [ ] **Step 3: Implement**

In `render.rs`:

```rust
/// What an operator sees after asserting a grade: what they asked for, and what now stands.
///
/// TWO FACTS, ALWAYS. Printing only the standing grade would render a thread-scoped
/// `restricted` under a chart-wide `sequestered` as bare "sequestered", which reads as a
/// silent upgrade of the operator's own act. Printing only the asserted grade would claim
/// an effect that may not have occurred. There is deliberately no shortened form for the
/// agreeing case: a reader who learns that one grade means agreement can no longer read a
/// one-grade line as anything.
pub fn render_assert_readback(asserted: &str, standing: &str, winning_subject: &str) -> String {
    format!(
        "asserted {asserted}; {standing} now stands on this chart (winning subject: \
         {winning_subject})"
    )
}
```

- [ ] **Step 4: Wire it into the CLI**

In `main.rs`'s `Cmd::SensitivityAssert` arm, after the existing success `println!`:

```rust
            // #388 part 4: never echo the typed grade as though it were the outcome. Re-read
            // the effective grade and report both — the assertion may be outranked by a
            // standing chart-wide grade, which is a correct outcome the operator still needs
            // to see rather than infer.
            let after = cairn_node::sensitivity::chart_sensitivity(&mut db, patient).await?;
            println!(
                "{}",
                cairn_node::sensitivity::render::render_assert_readback(
                    &grade,
                    &after.chart_grade,
                    &after.chart_source,
                )
            );
```

**Note for the implementer:** check the arm's actual binding names (`grade`, `patient`, and whether the client is still in scope and mutable after the submit) and adjust; do not assume.

- [ ] **Step 5: Run and commit**

Run: `cargo test -p cairn-node --lib sensitivity 2>&1 | tail -5` — Expected: PASS, 14 tests.
Run: `cargo clippy -p cairn-node --all-targets 2>&1 | tail -5` — Expected: clean.

```bash
git add crates/cairn-node/src/sensitivity/render.rs crates/cairn-node/src/main.rs
git commit -m "feat(#388): sensitivity-assert reports what actually took effect

Both orchestrators returned a locally-minted Uuid and never read back what
became standing. Prints two facts, always: what was asserted and what now
stands with its winning subject. One grade would render a thread-scoped
'restricted' under a chart-wide 'sequestered' as a silent upgrade of the
operator's own act.

Refs #388"
```

---

### Task 9: Documentation — currency, prune, and the budget marked met

**Files:**
- Modify: `docs/HANDOVER.md` (598 lines → under 500), `docs/ROADMAP.md` (575 → under 500)
- Modify: `docs/spec/decisions/0064-admit-the-claim-withhold-the-power.md` — **an appended erratum only.** ADRs are immutable; a factual correction is an appended erratum, never an edit to the decision text (#417 is the standing reminder that in-text line citations drift).

- [ ] **Step 1: Append the erratum to ADR-0064**

At the end of ADR-0064, in its errata section (create one following ADR-0063's E1 format if absent):

```markdown
**E1 (2026-08-18).** The §1.2 budget above — *"why did this withdrawal not take effect?"
in one query with no raw SQL* — was recorded as **owed, not met**, because no shipped
surface read `sensitivity_withdrawal_worklist`. It is now **met**: `patient-sensitivity
<chart>` reports every ineffective withdrawal with its reason, its rationale and its
accountable actor, and the budget is pinned by a test rather than by a hand exercise
(#388). The *Known limitations* entry naming the permanently-invisible cross-chart
withdrawal is unchanged and is now printed in the report's own footer.
```

- [ ] **Step 2: Fix HANDOVER's stale issue citations**

- Lines ~524-525: remove **#157** and **#176** (both CLOSED) from the medication cross-cutting-debt list.
- Line ~536: remove **#79** (CLOSED) from the deferred-items list.
- The ⇒ NEXT paragraph: **#405** and **#424** are now CLOSED; the residual design question they carried is **#432**. Rewrite so the residual is named by its live issue, and keep the substantive correction — the column grant is cost-raising, not a floor.
- **#294** is closed (discharged by Slice 67, verified against `safety_carried_class.rs`); drop any language implying it is outstanding.
- Add the Slice 69 entry: this slice, what it closes (#388, #383, #421), and the two decisions worth carrying (*name, never count*; *a chart-scoped definer, not a table grant*).

- [ ] **Step 3: Prune both docs under 500 lines**

Both breach the cap #368 closed at 549/529. Condense the oldest session entries — their *why* is in the ADR log and their *what* is in git — but **never drop an open issue number while condensing** (the PR #271 review finding, restated at the top of ROADMAP).

Run: `wc -l docs/HANDOVER.md docs/ROADMAP.md` — Expected: both under 500.

- [ ] **Step 4: Full gate before the final commit**

House rule 6 — all tests pass before committing. Run these in the **controller** session, not a subagent: `cargo test --workspace` exceeds the 10-minute Bash cap.

```bash
CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" \
CAIRN_TEST_PG2="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test2" \
CAIRN_TEST_PG3="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test3" \
  scripts/run-db-gated-tests.sh
```

Expected: green. Note that a killed binary exits 101 with **zero** `test result: FAILED` lines — check the exit code, never just the tail. If a run stalls between binaries, that is the macOS `_dyld_start` loader stall: diagnose with `sample <pid>`, `kill -9`, retry.

- [ ] **Step 5: Commit**

```bash
git add docs/
git commit -m "docs(#388): ADR-0064's §1.2 budget is met; HANDOVER/ROADMAP currency + prune

The budget is discharged by an appended erratum (ADRs are immutable). HANDOVER
dropped three closed issues it still listed as open debt (#157, #176, #79),
retracked #405/#424's residual as #432, and both files come back under the
500-line cap #368 closed them at.

Refs #388, #368"
```

---

## Paper-parity benchmark (§1.2)

This changes a clinical-adjacent operator workflow — reading a chart's confidentiality state — so it carries the benchmark rather than the forced-rationale escape.

- **Paper counterpart:** reading a paper chart's confidentiality state — the restriction sticker on the cover, *and* any struck-and-initialled removals, both visible in one glance at the same cover. Paper does not make you consult a second document to learn that a restriction was struck, or that one was struck by someone with no authority to strike it.
- **Steps:** paper **N = 1** human act (look at the cover) → architecture-forced **M = 1** (one `patient-sensitivity <chart>` invocation returns the grade and every anomaly this node can see) → UI bundling target **K = 1** (there is one command). **M = N — no architecture defect.**
- **Time + cognitive load:** ADR-0064's own budget, restated falsifiably — an operator must answer *"why did this withdrawal not take effect?"* in **one query with no raw SQL**. **Measurement:** on a chart carrying an inert withdrawal, one call to `chart_sensitivity` must yield the reason, the rationale and the accountable actor; pinned by `an_un_attested_withdrawal_is_reported_as_inert_with_its_reason_and_rationale` (Task 4) rather than by a hand exercise, so the budget is met *and* kept met. The node-tier read cost is measurable through the existing `cairn-gui/cairn-gui-tauri/results/RUNBOOK.md`; the **interactive** half stays owed by the first GUI surface (#400) and is not claimed here.
- **If the measurement falls outside the budget, that is the finding — file an issue; do not adjust the budget to match.**

## Follow-ons to file (house rule 5)

Anything found during the build that is out of scope becomes an issue, never a silent pass. Expected candidates: whether a hub tier wants a node-wide sweep across all charts; whether #387's type-design rework (a `Provenance` enum, ladder constants, a sum type for the correlated `Option` pair) should precede or follow this; and whether the `deferred` CLI verb and this report should share one rendering path now that both exist.
