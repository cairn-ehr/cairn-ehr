# Repair path R2b-1 — "Same person as…": compare and link from the window — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** From an open chart, a clinician finds another chart, sees both RECORDS side by side (every
member of each, with every cross-pair veto finding first), and links them with the one signature
their unlocked key already covers — and the window reports what the link actually did.

**Architecture:** A node-side read (`cairn-node/src/patient/compare.rs`) assembles each chart's
identity facts and the set-against-set veto findings; the window (`cairn-gui-tauri/src/link/`) adds
two commands, `compare_records` and `link_records`, that apply the screen/set checks every chart
command applies, then call the read and R2a's `chart_link::link_charts`; all wording is built by
pure functions in `link/view.rs`; `src-ui/link.js` only renders. `chart_link`'s own pre-check
refusals are first re-minted as marked verdicts so the window never words a verdict as an outage.

**Tech Stack:** Rust (tokio-postgres, anyhow, serde), PostgreSQL ≥ 18 + `cairn_pgx`, Tauri 2 with
plain JavaScript (no npm, no bundler).

**Spec:** `docs/superpowers/specs/2026-09-27-duplicate-repair-path-679-680-681-design.md` — section
"R2 — link and unlink…" and its sub-section **"R2b — the window's gesture (designed 2026-09-28)"**.
ADR-0076 decisions 3–5. This plan builds R2b-1 only; R2b-2 (unlink, #699) is a separate plan.

## Global Constraints

- **AGPL-3.0**; **no new dependency** in any tree (every crate used below is already a dependency of
  the crate that uses it — check `Cargo.toml` before adding a `use`; if one is missing, stop and ask).
- **No SQL object, no migration: `SCHEMA_GENERATION` stays 55.** If a task seems to need a
  `db/*.sql` change, stop — the design says the read uses existing tables and functions only.
- **No confirmation dialog anywhere** (principle 3). The panel's safety is what it SHOWS.
- **No node-key fallback**: a judgement is signed and attested by the unlocked human key (ADR-0053).
- **Every chart command names the chart on screen AND the displayed set** (`AppState::displayed_patient`
  first, then `chart_set::check_displayed_set`) — and `link_records` names the OTHER record's
  displayed set too (design: "decision 3 widened to the right-hand side").
- **Absence is worded, never blank** (principle 4): held chart → "not recorded"; chart not held here
  → "unknown — registration not yet received here".
- **Never "no conflicts"**: an empty veto list renders nothing, because an absent finding is not a clearance.
- **Files under 500 lines** (house rule 4). `commands.rs` is 637 already — do NOT add to it.
- **CodeQL**: a test assertion message must never echo a name, DOB or other identity text built from
  fixture data (`rust/cleartext-logging`); use static messages. No binding named `salt`/`nonce`/`iv`.
- **Doc comments for a junior reader** on every non-trivial function (house rule 3). Match the
  surrounding files' comment density.
- **A `pub fn` added to `crates/cairn-node/tests/common/mod.rs` must also be added to
  `identity_scaffolding_shared.rs`'s expected-helper list** — so this plan keeps new test helpers
  FILE-LOCAL instead.
- **Gates.** Root DB-gated sweep: `scripts/run-db-gated-tests.sh` (no args; it discovers the
  cluster). Targeted DB-gated run of one suite:
  `CAIRN_TEST_PG="host=127.0.0.1 port=$(scripts/pg-target.sh | cut -d' ' -f2) user=$USER dbname=cairn_test" cargo test -p cairn-node --test <suite>`
  — a targeted `ok` is NOT proof (trap 18); the sweep is. DB-free runs need `CAIRN_ALLOW_DB_SKIP=1`.
  GUI tree: `cd cairn-gui && CAIRN_ALLOW_DB_SKIP=1 cargo test --workspace && cargo clippy --workspace --all-targets --locked -- -D warnings && RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps`.
  Root: `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`,
  `RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps`. With an IDE open, prefix cargo with
  `CARGO_TARGET_DIR=/tmp/cairn-r2b1-target`. Never pipe a cargo run into `tail` (it masks the exit code).
- **Commit messages**: run `python3 scripts/check_closing_keywords.py <file-with-message>` before
  committing anything that names an issue; `Refs #681` is safe, `closes`/`fixes` adjacent to a number is not.

## Review Focus

1. **The opened chart (or any member of its record) comes back from the in-chart search and is
   picked** → refused "that chart is already part of this record", never a self/intra-record link
   (`compare_impl` test in Task 5; the webview also filters them, Task 6).
2. **The other record changes between Compare and Link** (a peer's link lands and joins B to D) →
   `link_records` refuses "the other record changed while you were comparing — nothing was done;
   compare again" (Task 5 test with a mismatching `other_charts`).
3. **A veto between the picked chart and a THIRD chart already linked to the opened one** (A–C
   linked, B picked, B–C DOB clash) → the finding is shown, tagged with the B–C pair (Task 3 DB test).
4. **Part of the comparison cannot be read** (the other record's medications, or a fact query) →
   the panel still shows what WAS read, names what was not, and offers NO Link button (Task 4 pure test).
5. **A `chart_link` pre-check refusal (a chart not held here, a non-human key, the same chart twice)**
   → classified as a verdict, not an outage, so the window never says "try again" (Task 1 DB tests,
   Task 4 pure test of the classification).

---

## File structure

| File | Responsibility |
|---|---|
| `crates/cairn-node/src/chart_link.rs` (modify) | Pre-check refusals minted as marked verdicts. |
| `crates/cairn-node/src/db_diagnosis.rs` (modify, doc only) | `node_state_refusal`'s call-site list names `chart_link`. |
| `crates/cairn-node/src/patient/compare.rs` (create) | `ChartFacts`, `chart_facts`, `VetoFinding`, `cross_vetoes`, `order_findings`. |
| `crates/cairn-node/src/patient/person.rs` (modify) | `read_held`, `read_trusts` become `pub(crate)` for reuse. |
| `crates/cairn-node/src/patient/mod.rs` (modify) | `pub mod compare;` |
| `crates/cairn-node/tests/chart_compare.rs` (create) | DB-gated tests for the read. |
| `crates/cairn-node/tests/chart_link.rs` (modify) | Refusal-scope assertions on three existing tests. |
| `crates/cairn-node/tests/db_errors_stay_legible.rs` (modify) | `compare.rs` joins `GUARDED` with its own count pin. |
| `cairn-gui/cairn-gui-tauri/src/link/view.rs` (create) | Pure: `comparison_view`, `finding_line`, `medication_lines`, `link_report`, `link_error_view`, `fixture_facts`. |
| `cairn-gui/cairn-gui-tauri/src/link/mod.rs` (create) | `compare_impl`, `link_impl`, `chart_set_of`, Tauri forwarders. |
| `cairn-gui/cairn-gui-tauri/src/commands.rs` (modify) | `read_chart_of` → `pub(crate)`. |
| `cairn-gui/cairn-gui-tauri/src/main.rs` (modify) | `mod link;` + two handlers. |
| `cairn-gui/cairn-gui-tauri/src-ui/index.html`, `link.js` (create), `main.js`, `style.css` | The panel. |
| `cairn-gui/cairn-gui-tauri/results/RUNBOOK.md`, `TEMPLATE.md` | Section 9: compare and link. |

---

### Task 1: `chart_link`'s pre-check refusals are verdicts, not outages (#702's class)

Today `judge` refuses with bare `anyhow::bail!`/`Error::msg`, which carry neither the
`DeliberateRefusal` marker nor a SQLSTATE — so `cairn_gui_live::error::data_error_from` would call
them `Unavailable` and the window would say "try again" to a verdict.

Scope choice (write it in the doc comment): **same chart twice → `deliberate_refusal`** (about the
input, forever); **a chart not held / not in the record → `node_state_refusal`** (the identical call
succeeds once this node holds the chart — node state, not the input); **a key that is not an
enrolled human → `node_state_refusal`** (an operator's `enroll-human` makes it succeed).

**Files:**
- Modify: `crates/cairn-node/src/chart_link.rs:23` (import), `:517-541` (the three refusals)
- Modify: `crates/cairn-node/src/db_diagnosis.rs:451-455` (doc: call sites now include `chart_link`)
- Test: `crates/cairn-node/tests/chart_link.rs` (three existing tests gain an assertion)

**Interfaces:**
- Consumes: `crate::db_diagnosis::{deliberate_refusal, node_state_refusal}` (both `pub(crate)`),
  `cairn_node::db_diagnosis::{refusal_scope, RefusalScope}` (pub).
- Produces: unchanged signatures; `link_charts`/`unlink_charts` errors from the pre-checks now answer
  `refusal_scope(&e) == Some(RefusalScope::Input | RefusalScope::NodeState)`. Display text unchanged.

- [ ] **Step 1: Write the failing assertions.** In `crates/cairn-node/tests/chart_link.rs`, add to
  the imports: `use cairn_node::db_diagnosis::{refusal_scope, RefusalScope};`. Then change the three
  tests so they keep the `anyhow::Error` before stringifying it:

  In `a_non_human_key_is_refused_and_nothing_moves` replace the `let err = … .to_string();` block with:

  ```rust
      let err = link_charts(&mut c, a, b, &agent, ORIGIN).await.unwrap_err();
      // #702's class: a verdict must be MARKED, or the window words it as an outage ("try again").
      // NodeState: an operator's `enroll-human` makes the identical call succeed.
      assert_eq!(
          refusal_scope(&err),
          Some(RefusalScope::NodeState),
          "a non-human reviewer is a node-state verdict"
      );
      let err = err.to_string();
  ```

  In `a_chart_this_node_has_never_seen_is_refused_before_signing` replace `let err = r.unwrap_err().to_string();` with:

  ```rust
          let err = r.unwrap_err();
          assert_eq!(
              refusal_scope(&err),
              Some(RefusalScope::NodeState),
              "a chart not held here is a node-state verdict: it succeeds once the chart arrives"
          );
          let err = err.to_string();
  ```

  In `a_chart_cannot_be_linked_to_itself` replace the `let err = … .to_string();` block with:

  ```rust
      let err = link_charts(&mut c, a, a, &who, ORIGIN).await.unwrap_err();
      assert_eq!(
          refusal_scope(&err),
          Some(RefusalScope::Input),
          "the same chart twice is a verdict about the input, forever"
      );
      let err = err.to_string();
  ```

- [ ] **Step 2: Run to verify they fail.**
  Run: `CAIRN_TEST_PG="…cairn_test" cargo test -p cairn-node --test chart_link -- refused itself`
  Expected: FAIL — `left: None, right: Some(NodeState)` (and `Some(Input)` for the self-link test).

- [ ] **Step 3: Implement.** In `chart_link.rs` change the import to
  `use crate::db_diagnosis::{deliberate_refusal, node_state_refusal, LocalDbFault};` and in `judge`:

  ```rust
      if a == b {
          // A verdict about the INPUT: no retry, by anyone, ever changes it.
          return Err(deliberate_refusal(format!(
              "{a} and {b} are the same chart — a chart cannot be linked to itself"
          )));
      }
  ```

  ```rust
      // A verdict about this NODE's state: the identical call succeeds once the chart (or the
      // record joining them) has arrived here. Marked, so a surface words it as a verdict and
      // never as an outage to retry (#702).
      let about = admit_judgement(verb, (a, a_held), (b, b_held), shared_record)
          .map_err(node_state_refusal)?;
  ```

  ```rust
      {
          return Err(node_state_refusal(format!(
              "key {} is not an enrolled human actor — linking or unlinking charts is a human \
               judgement (unlock a clinician's key)",
              reviewer.human_kid
          )));
      }
  ```

  In `db_diagnosis.rs`'s `node_state_refusal` doc, replace "The three current call sites are the
  [`crate::actor_enrolment`] refusals" with "The current call sites are the three
  [`crate::actor_enrolment`] refusals and `chart_link`'s held-chart and enrolled-human pre-checks
  (R2b-1)". Keep the rest of the paragraph.

- [ ] **Step 4: Run to verify they pass.** Same command. Expected: PASS. Also run
  `CAIRN_ALLOW_DB_SKIP=1 cargo test -p cairn-node --lib chart_link` (the pure `admit_judgement`
  tests still compare `Err(String)` — unchanged). Expected: PASS.

- [ ] **Step 5: Commit.**
  ```bash
  git add crates/cairn-node/src/chart_link.rs crates/cairn-node/src/db_diagnosis.rs crates/cairn-node/tests/chart_link.rs
  git commit -m "fix(R2b-1): chart_link's pre-check refusals are marked verdicts, not outages (Refs #702)"
  ```

---

### Task 2: `chart_facts` — each member chart's identity facts, read over a set

**Files:**
- Create: `crates/cairn-node/src/patient/compare.rs`
- Modify: `crates/cairn-node/src/patient/mod.rs` (add `pub mod compare;` after `pub mod person;`)
- Modify: `crates/cairn-node/src/patient/person.rs:131` and `:185` (`async fn read_held` / `read_trusts` → `pub(crate) async fn`)
- Test: `crates/cairn-node/tests/chart_compare.rs` (create)
- Modify: `crates/cairn-node/tests/db_errors_stay_legible.rs` (`GUARDED` + a count pin)

**Interfaces:**
- Consumes: `patient::person::{read_held, read_trusts, trust_of}`, `ChartSet::members()`, `LocalDbFault::new(&str, tokio_postgres::Error)`.
- Produces (all `pub`, all fields `pub`, all `#[derive(Debug, Clone, PartialEq, Eq)]`, `ChartFacts` also `Default`):
  ```rust
  pub struct NameFact { pub value: String, pub use_: Option<String>, pub provenance: String }
  pub struct FieldFact { pub value: String, pub provenance: String }
  pub struct IdentifierFact { pub system: String, pub value: String, pub provenance: String }
  pub struct AddressFact { pub use_: Option<String>, pub display: String, pub provenance: String }
  pub struct ChartFacts {
      pub patient_id: Uuid, pub held: bool, pub trust: String,
      pub names: Vec<NameFact>, pub aliases: Vec<String>,
      pub dob: Option<FieldFact>, pub sex_at_birth: Option<FieldFact>,
      pub identifiers: Vec<IdentifierFact>, pub addresses: Vec<AddressFact>,
  }
  pub async fn chart_facts<C: GenericClient + Sync>(client: &C, set: &ChartSet) -> anyhow::Result<Vec<ChartFacts>>
  ```
  Returned in `set.members()` order, one per member, always.

- [ ] **Step 1: Write the failing DB tests.** Create `crates/cairn-node/tests/chart_compare.rs`:

  ```rust
  //! Repair path R2b-1 — the side-by-side comparison's node read (`patient::compare`).
  //!
  //! Two paper front sheets laid next to each other: every member chart of each record, with
  //! EVERY name it carries (not just the display winner), its earlier recorded names apart,
  //! DOB and sex-at-birth with provenance, identifiers and addresses. Real Postgres, gated on
  //! `$CAIRN_TEST_PG`, serialized via `db::test_serial_guard`.
  mod common;
  use cairn_event::demographics::{
      address_assertion_body, dob_assertion_body, identifier_assertion_body, name_assertion_body,
      render_address_twin, render_dob_twin, render_identifier_twin, render_name_twin,
      render_sex_at_birth_twin, sex_at_birth_assertion_body, AddressAssertion, IdentifierAssertion,
  };
  use cairn_event::identity::{render_repudiate_twin, repudiation_assertion_body, RepudiationAssertion};
  use cairn_event::{ClockGrade, EventBody, Hlc};
  use cairn_medication_view::ChartSet;
  use cairn_node::db;
  use cairn_node::patient::compare::chart_facts;
  use common::{
      cs, enroll_human, medication_setup as setup, submit_attested, submit_link_event,
      submit_registration, submit_signed, EventSpec,
  };
  use uuid::Uuid;

  /// One demographic-field event (name / dob / sex / address) at `wall`.
  async fn field(
      c: &tokio_postgres::Client,
      sk: &cairn_event::SigningKey,
      kid: &str,
      p: Uuid,
      wall: i64,
      payload: serde_json::Value,
      twin: String,
  ) {
      submit_signed(
          c,
          sk,
          kid,
          EventSpec {
              patient: p,
              event_type: "demographic.field.asserted",
              schema_version: "demographic.field/1",
              payload,
              plaintext_twin: Some(twin),
              wall,
          },
      )
      .await
      .expect("demographic field accepted");
  }

  async fn fresh(c: &tokio_postgres::Client, sk: &cairn_event::SigningKey, kid: &str) -> Uuid {
      let p = Uuid::now_v7();
      submit_registration(c, sk, kid, p, 1).await;
      p
  }

  #[tokio::test]
  async fn every_fact_of_a_chart_is_read_with_its_provenance() {
      let Some(base) = cs() else {
          eprintln!("skipped: set CAIRN_TEST_PG");
          return;
      };
      let _g = db::test_serial_guard(&base).await.unwrap();
      let c = db::connect_and_load_schema(&base).await.unwrap();
      let (sk, kid, _, _) = setup(&c).await;
      let p = fresh(&c, &sk, &kid).await;
      field(&c, &sk, &kid, p, 2, name_assertion_body("Mary SMITH", Some("legal"), "patient-stated"),
            render_name_twin("Mary SMITH", Some("legal"), "patient-stated")).await;
      field(&c, &sk, &kid, p, 3, name_assertion_body("Mary JONES", Some("maiden"), "patient-stated"),
            render_name_twin("Mary JONES", Some("maiden"), "patient-stated")).await;
      field(&c, &sk, &kid, p, 4, dob_assertion_body("1950-07-01", "day", Some("document"), "document-verified"),
            render_dob_twin("1950-07-01", "day", "document-verified")).await;
      field(&c, &sk, &kid, p, 5, sex_at_birth_assertion_body("female", "patient-stated"),
            render_sex_at_birth_twin("female", "patient-stated")).await;
      let addr = AddressAssertion { display: "1 Main St, Bamaga", provenance: "patient-stated",
                                    use_: Some("residential"), geo: None, structured: None };
      field(&c, &sk, &kid, p, 6, address_assertion_body(&addr), render_address_twin(&addr)).await;
      let id = IdentifierAssertion { value: "1234 56789 0", system: "au-medicare", provenance: "document-verified",
                                     normalized: None, profile: None, use_: None };
      submit_signed(&c, &sk, &kid, EventSpec {
          patient: p, event_type: "demographic.identifier.asserted",
          schema_version: "demographic.identifier/1",
          payload: identifier_assertion_body(&id), plaintext_twin: Some(render_identifier_twin(&id)), wall: 7,
      }).await.expect("identifier accepted");

      let facts = chart_facts(&c, &ChartSet::single(p)).await.unwrap();
      assert_eq!(facts.len(), 1);
      let f = &facts[0];
      assert_eq!(f.patient_id, p);
      assert!(f.held, "a registered chart is held here");
      assert_eq!(f.trust, "confirmed");
      // EVERY retained name, legal first — a maiden name is often the very clue.
      let names: Vec<(&str, Option<&str>)> =
          f.names.iter().map(|n| (n.value.as_str(), n.use_.as_deref())).collect();
      assert_eq!(names, vec![("Mary SMITH", Some("legal")), ("Mary JONES", Some("maiden"))]);
      assert!(f.aliases.is_empty());
      let dob = f.dob.as_ref().expect("a dob was asserted");
      assert_eq!((dob.value.as_str(), dob.provenance.as_str()), ("1950-07-01", "document-verified"));
      assert_eq!(f.sex_at_birth.as_ref().map(|s| s.value.as_str()), Some("female"));
      assert_eq!(f.identifiers.len(), 1);
      assert_eq!(f.identifiers[0].system, "au-medicare");
      assert_eq!(f.addresses.len(), 1);
      assert_eq!(f.addresses[0].use_.as_deref(), Some("residential"));
  }

  #[tokio::test]
  async fn a_repudiated_name_moves_to_the_aliases() {
      let Some(base) = cs() else {
          eprintln!("skipped: set CAIRN_TEST_PG");
          return;
      };
      let _g = db::test_serial_guard(&base).await.unwrap();
      let c = db::connect_and_load_schema(&base).await.unwrap();
      let (sk, kid, _, _) = setup(&c).await;
      let (sk_h, kid_h) = enroll_human(&c).await;
      let p = fresh(&c, &sk, &kid).await;
      field(&c, &sk, &kid, p, 2, name_assertion_body("John DOE", Some("legal"), "patient-stated"),
            render_name_twin("John DOE", Some("legal"), "patient-stated")).await;
      field(&c, &sk, &kid, p, 3, name_assertion_body("Jack DOE", Some("legal"), "patient-stated"),
            render_name_twin("Jack DOE", Some("legal"), "patient-stated")).await;
      let s = p.to_string();
      let r = RepudiationAssertion { subject: &s, value: "John DOE", reason: "confessed fabricated persona" };
      let body = EventBody {
          event_id: Uuid::now_v7().to_string(),
          patient_id: s.clone(),
          event_type: "identity.repudiate.asserted".into(),
          schema_version: "identity.repudiate.asserted/1".into(),
          hlc: Hlc { wall: 10, counter: 0, node_origin: "n".into() },
          t_effective: None,
          signer_key_id: kid.clone(),
          contributors: serde_json::json!([
              {"actor_id": kid_h, "role": "attested", "responsibility": {"held_by": kid_h}}
          ]),
          payload: repudiation_assertion_body(&r),
          attachments: vec![],
          plaintext_twin: Some(render_repudiate_twin(&r)),
          clock_grade: ClockGrade::SelfAsserted,
          safety: None,
      };
      submit_attested(&c, &sk, body, &sk_h, &kid_h).await.expect("repudiation accepted");

      let f = &chart_facts(&c, &ChartSet::single(p)).await.unwrap()[0];
      assert!(f.names.iter().all(|n| n.value != "John DOE"), "a repudiated name is not a current name");
      assert_eq!(f.aliases, vec!["John DOE".to_string()], "it is kept, apart, as an earlier name");
  }

  #[tokio::test]
  async fn a_linked_set_reads_every_member_and_an_unheld_one_says_so() {
      let Some(base) = cs() else {
          eprintln!("skipped: set CAIRN_TEST_PG");
          return;
      };
      let _g = db::test_serial_guard(&base).await.unwrap();
      let c = db::connect_and_load_schema(&base).await.unwrap();
      c.batch_execute("TRUNCATE patient_link, person_member").await.unwrap();
      let (sk, kid, _, _) = setup(&c).await;
      let a = fresh(&c, &sk, &kid).await;
      let unheld = Uuid::now_v7(); // named by a link; its registration never arrived here
      submit_link_event(&c, &sk, &kid, a, unheld, 10, true).await;
      let set = ChartSet::new([a, unheld]).unwrap();

      let facts = chart_facts(&c, &set).await.unwrap();
      assert_eq!(
          facts.iter().map(|f| f.patient_id).collect::<Vec<_>>(),
          set.members().to_vec(),
          "one entry per member, in the set's own order"
      );
      let u = facts.iter().find(|f| f.patient_id == unheld).unwrap();
      assert!(!u.held);
      assert_eq!(u.trust, "unknown", "no row about an unheld chart is not evidence of 'confirmed'");
      assert!(u.names.is_empty() && u.dob.is_none(), "absent, never invented");
  }
  ```

  (Run `cargo fmt` after pasting — the one-line `field(...)` calls above are compressed for the plan.)
  If `submit_link_event` for a chart with no registration is refused by db/005 step 8b, file the link
  under the HELD chart: the helper already uses `a` (the first argument) as `patient_id` — keep `a` held.

- [ ] **Step 2: Run to verify they fail.**
  Run: `CAIRN_TEST_PG="…cairn_test" cargo test -p cairn-node --test chart_compare`
  Expected: FAIL to compile — `unresolved import cairn_node::patient::compare`.

- [ ] **Step 3: Implement `compare.rs`.** Make `read_held` and `read_trusts` in `person.rs`
  `pub(crate)`. Add `pub mod compare;` to `patient/mod.rs`. Create `compare.rs`:

  ```rust
  //! The side-by-side comparison a human reads before linking two records (repair path R2b-1,
  //! ADR-0076 decision 4; design "R2b — the window's gesture").
  //!
  //! Paper counterpart: the clerk fetches the other folder and lays the two FRONT SHEETS side by
  //! side. This module reads what those front sheets carry, per member chart, and nothing is
  //! merged: two duplicates differ by exactly the typo that made them, so a "winner" would erase
  //! the evidence (the same rule as `person::ChartIdentity`).
  //!
  //! WHY EVERY NAME, NOT THE DISPLAY WINNER. `patient_name_current` picks one name per chart. A
  //! maiden or preferred name is often precisely what tells a clerk two charts are one woman, so
  //! the comparison reads every retained name with its `use`. Names a human REPUDIATED (§5.7) are
  //! listed apart as `aliases` — "was recorded as" — never mixed in with the current ones.
  //!
  //! SHAPE: one flat `WHERE patient_id = ANY(...)` query per fact kind, joined in Rust — the
  //! `person.rs` style, for the reason `chart_identities` gives (each projection has its own
  //! absence convention, and an outer join per projection is harder to review). Every query goes
  //! through [`rows`], which names the step on failure (#467: `db_errors_stay_legible.rs` guards
  //! this file).
  use crate::db_diagnosis::LocalDbFault;
  use crate::patient::person::{read_held, read_trusts, trust_of};
  use anyhow::Context;
  use cairn_medication_view::ChartSet;
  use std::collections::HashMap;
  use tokio_postgres::{GenericClient, Row};
  use uuid::Uuid;

  /// One retained, non-repudiated name: the value exactly as authored, its `use` facet as the
  /// author gave it (`None` when absent — not "legal"), and its provenance.
  #[derive(Debug, Clone, PartialEq, Eq)]
  pub struct NameFact {
      pub value: String,
      pub use_: Option<String>,
      pub provenance: String,
  }

  /// A single-valued demographic field's standing value and where it came from — the
  /// provenance is what tells a clerk "document-verified" from "patient-stated".
  #[derive(Debug, Clone, PartialEq, Eq)]
  pub struct FieldFact {
      pub value: String,
      pub provenance: String,
  }

  /// One identifier: its namespace, the value as entered, and its provenance.
  #[derive(Debug, Clone, PartialEq, Eq)]
  pub struct IdentifierFact {
      pub system: String,
      pub value: String,
      pub provenance: String,
  }

  /// One current address (one per `use`, db/014's `patient_address_current`).
  #[derive(Debug, Clone, PartialEq, Eq)]
  pub struct AddressFact {
      pub use_: Option<String>,
      pub display: String,
      pub provenance: String,
  }

  /// Everything the comparison shows about ONE member chart. Every absence is `None` or empty —
  /// never a placeholder: the window words it, and "not recorded" differs from "unknown — the
  /// registration has not arrived" (`held`), a distinction only the reader can make.
  #[derive(Debug, Clone, PartialEq, Eq, Default)]
  pub struct ChartFacts {
      pub patient_id: Uuid,
      /// A `patient_chart` row exists — the same meaning as `person::ChartIdentity::held`.
      pub held: bool,
      /// [`trust_of`]'s answer: a `chart_trust` state, `confirmed`, or `unknown` when not held.
      pub trust: String,
      /// Legal first, then newest first — the order a front sheet would list them.
      pub names: Vec<NameFact>,
      /// Repudiated names (`patient_alias_pool`), oldest first.
      pub aliases: Vec<String>,
      pub dob: Option<FieldFact>,
      pub sex_at_birth: Option<FieldFact>,
      pub identifiers: Vec<IdentifierFact>,
      pub addresses: Vec<AddressFact>,
  }

  // The name read repeats `patient_name_current`'s repudiation predicate (db/025) because it
  // must return EVERY retained name, not that view's one winner. `a_repudiated_name_moves_to_
  // the_aliases` pins that the two agree on what "repudiated" means.
  const NAMES_SQL: &str = "SELECT n.patient_id::text AS patient_id, n.value, n.use_raw, n.provenance \
       FROM patient_name n \
       WHERE n.patient_id = ANY($1::text[]::uuid[]) \
         AND NOT EXISTS (SELECT 1 FROM name_repudiation r \
                          WHERE r.subject = n.patient_id AND r.value = n.value) \
       ORDER BY n.patient_id, (n.use_key = 'legal') DESC, n.last_hlc_wall DESC, \
                n.last_hlc_count DESC, n.value COLLATE \"C\"";
  const ALIASES_SQL: &str = "SELECT patient_id::text AS patient_id, value FROM patient_alias_pool \
       WHERE patient_id = ANY($1::text[]::uuid[]) \
       ORDER BY patient_id, hlc_wall, hlc_counter, value COLLATE \"C\"";
  const FIELDS_SQL: &str = "SELECT patient_id::text AS patient_id, field, value, provenance \
       FROM patient_demographic \
       WHERE field IN ('dob', 'sex-at-birth') AND patient_id = ANY($1::text[]::uuid[])";
  const IDENTIFIERS_SQL: &str = "SELECT patient_id::text AS patient_id, system, value, provenance \
       FROM patient_identifier WHERE patient_id = ANY($1::text[]::uuid[]) \
       ORDER BY patient_id, system COLLATE \"C\", value COLLATE \"C\"";
  const ADDRESSES_SQL: &str = "SELECT patient_id::text AS patient_id, use_raw, display, provenance \
       FROM patient_address_current WHERE patient_id = ANY($1::text[]::uuid[]) \
       ORDER BY patient_id, use_key COLLATE \"C\"";

  /// Run one per-set query, naming the step if it fails (the ONE `LocalDbFault` site for
  /// [`chart_facts`]'s five reads).
  async fn rows<C: GenericClient + Sync>(
      client: &C,
      sql: &str,
      ids: &[String],
      step: &str,
  ) -> anyhow::Result<Vec<Row>> {
      Ok(client
          .query(sql, &[&ids])
          .await
          .map_err(|e| LocalDbFault::new(step, e))?)
  }

  /// The row's `patient_id` column as a `Uuid`.
  fn patient_of(row: &Row) -> anyhow::Result<Uuid> {
      Ok(row.get::<_, String>("patient_id").parse()?)
  }

  /// Read [`ChartFacts`] for every member of `set`, in the set's own order.
  ///
  /// Generic over `GenericClient` so a caller may read inside a transaction. Errors on the first
  /// failed read — the window then shows the comparison as incomplete and offers no Link button.
  pub async fn chart_facts<C: GenericClient + Sync>(
      client: &C,
      set: &ChartSet,
  ) -> anyhow::Result<Vec<ChartFacts>> {
      let ids: Vec<String> = set.members().iter().map(Uuid::to_string).collect();
      let held = read_held(client, &ids)
          .await
          .context("reading which of the charts this node holds")?;
      let trusts = read_trusts(client, &ids)
          .await
          .context("reading the charts' identity states")?;
      let mut by_id: HashMap<Uuid, ChartFacts> = set
          .members()
          .iter()
          .map(|id| {
              let is_held = held.contains(id);
              let facts = ChartFacts {
                  patient_id: *id,
                  held: is_held,
                  trust: trust_of(is_held, trusts.get(id).map(String::as_str)),
                  ..ChartFacts::default()
              };
              (*id, facts)
          })
          .collect();
      // `get_mut` can only miss if a query returned a chart it was not asked about; skipping it
      // is then correct (it is not a member), so no `expect`.
      for row in rows(client, NAMES_SQL, &ids, "reading the charts' names").await? {
          if let Some(f) = by_id.get_mut(&patient_of(&row)?) {
              f.names.push(NameFact {
                  value: row.get("value"),
                  use_: row.get("use_raw"),
                  provenance: row.get("provenance"),
              });
          }
      }
      for row in rows(client, ALIASES_SQL, &ids, "reading the charts' earlier names").await? {
          if let Some(f) = by_id.get_mut(&patient_of(&row)?) {
              f.aliases.push(row.get("value"));
          }
      }
      for row in rows(client, FIELDS_SQL, &ids, "reading the charts' dates of birth and sex").await? {
          if let Some(f) = by_id.get_mut(&patient_of(&row)?) {
              let fact = FieldFact { value: row.get("value"), provenance: row.get("provenance") };
              match row.get::<_, String>("field").as_str() {
                  "dob" => f.dob = Some(fact),
                  _ => f.sex_at_birth = Some(fact), // the query admits only the two fields
              }
          }
      }
      for row in rows(client, IDENTIFIERS_SQL, &ids, "reading the charts' identifiers").await? {
          if let Some(f) = by_id.get_mut(&patient_of(&row)?) {
              f.identifiers.push(IdentifierFact {
                  system: row.get("system"),
                  value: row.get("value"),
                  provenance: row.get("provenance"),
              });
          }
      }
      for row in rows(client, ADDRESSES_SQL, &ids, "reading the charts' addresses").await? {
          if let Some(f) = by_id.get_mut(&patient_of(&row)?) {
              f.addresses.push(AddressFact {
                  use_: row.get("use_raw"),
                  display: row.get("display"),
                  provenance: row.get("provenance"),
              });
          }
      }
      // Every member was inserted above, so `remove` always finds it.
      Ok(set.members().iter().filter_map(|id| by_id.remove(id)).collect())
  }
  ```

- [ ] **Step 4: Guard the file (#467).** In `tests/db_errors_stay_legible.rs`: add
  `"crates/cairn-node/src/patient/compare.rs",` to `GUARDED` (keep it sorted); add to the module doc's
  "Scope" paragraph that the guarded set now includes `patient/compare.rs` (R2b-1 — the comparison a
  human links on) — change "Six files" to "Seven files"; and add a pin after the `chart_link` one:

  ```rust
  /// How many `LocalDbFault`s `patient/compare.rs` builds: its `rows` helper (all five of
  /// `chart_facts`'s reads) and `cross_vetoes`'s one query. The two `person.rs` reads it reuses
  /// carry `.context(…)` instead — `person.rs` is outside `GUARDED` (#485).
  const COMPARE_LOCAL_DB_FAULT_SITES: usize = 1;

  /// Every postgres call the comparison makes names what it was doing (R2b-1).
  #[test]
  fn every_postgres_call_in_the_comparison_names_what_it_was_doing() {
      let root = sources::repo_root();
      let text = flattened_code(
          &std::fs::read_to_string(root.join("crates/cairn-node/src/patient/compare.rs"))
              .expect("compare.rs is in the tree"),
      );
      let found = text.matches("LocalDbFault::new(").count();
      assert_eq!(
          found, COMPARE_LOCAL_DB_FAULT_SITES,
          "compare.rs builds {found} `LocalDbFault`s, expected {COMPARE_LOCAL_DB_FAULT_SITES}. \
           If you ADDED a postgres call, wrap it and bump the constant."
      );
  }
  ```

  (Task 3 bumps the constant to 2 when `cross_vetoes` lands.)

- [ ] **Step 5: Run to verify they pass.**
  Run: `CAIRN_TEST_PG="…" cargo test -p cairn-node --test chart_compare` then
  `CAIRN_ALLOW_DB_SKIP=1 cargo test -p cairn-node --test db_errors_stay_legible`.
  Expected: PASS both. Then `cargo clippy -p cairn-node --all-targets -- -D warnings` — clean.

- [ ] **Step 6: Commit.**
  ```bash
  git add crates/cairn-node/src/patient crates/cairn-node/tests/chart_compare.rs crates/cairn-node/tests/db_errors_stay_legible.rs
  git commit -m "feat(R2b-1): chart_facts — every member chart's front-sheet facts, read over a set"
  ```

---

### Task 3: `cross_vetoes` — every hard-veto finding across two records

**Files:**
- Modify: `crates/cairn-node/src/patient/compare.rs` (append)
- Modify: `crates/cairn-node/tests/chart_compare.rs` (append tests)
- Modify: `crates/cairn-node/tests/db_errors_stay_legible.rs` (`COMPARE_LOCAL_DB_FAULT_SITES` → 2)

**Interfaces:**
- Consumes: db/016 `cairn_match_veto(uuid, uuid) RETURNS TABLE(veto_kind, severity, subject, detail)`.
- Produces:
  ```rust
  #[derive(Debug, Clone, PartialEq, Eq)]
  pub struct VetoFinding { pub left: Uuid, pub right: Uuid, pub kind: String, pub severity: String, pub subject: String, pub detail: String }
  pub fn order_findings(findings: Vec<VetoFinding>) -> Vec<VetoFinding>
  pub async fn cross_vetoes<C: GenericClient + Sync>(client: &C, left: &ChartSet, right: &ChartSet) -> anyhow::Result<Vec<VetoFinding>>
  ```

- [ ] **Step 1: Write the failing tests.** Append to `compare.rs` a test module:

  ```rust
  #[cfg(test)]
  mod tests {
      use super::*;

      fn finding(severity: &str, l: u128, r: u128, kind: &str) -> VetoFinding {
          VetoFinding {
              left: Uuid::from_u128(l),
              right: Uuid::from_u128(r),
              kind: kind.into(),
              severity: severity.into(),
              subject: kind.into(),
              detail: String::new(),
          }
      }

      /// A hard veto is the fact a clerk most needs, so it is read FIRST — before any
      /// degrade-hold — whatever order the database returned them in.
      #[test]
      fn hard_vetoes_come_first_then_a_stable_order() {
          let ordered = order_findings(vec![
              finding("degrade_hold", 1, 2, "identifier"),
              finding("hard_veto", 3, 4, "dob"),
              finding("hard_veto", 1, 2, "sex-at-birth"),
          ]);
          let got: Vec<(&str, u128)> = ordered
              .iter()
              .map(|f| (f.severity.as_str(), f.left.as_u128()))
              .collect();
          assert_eq!(got, vec![("hard_veto", 1), ("hard_veto", 3), ("degrade_hold", 1)]);
      }
  }
  ```

  Append to `tests/chart_compare.rs` (add `cross_vetoes` to the `use cairn_node::patient::compare::…` line):

  ```rust
  /// The set-against-set case (design "R2b"): A is already linked to C; B is picked. Linking A–B
  /// also joins B to C, so a B–C clash must be shown even though A and B agree.
  #[tokio::test]
  async fn a_clash_with_a_third_chart_already_in_the_record_is_found() {
      let Some(base) = cs() else {
          eprintln!("skipped: set CAIRN_TEST_PG");
          return;
      };
      let _g = db::test_serial_guard(&base).await.unwrap();
      let c = db::connect_and_load_schema(&base).await.unwrap();
      c.batch_execute("TRUNCATE patient_link, person_member").await.unwrap();
      let (sk, kid, _, _) = setup(&c).await;
      let (a, b, third) = (fresh(&c, &sk, &kid).await, fresh(&c, &sk, &kid).await, fresh(&c, &sk, &kid).await);
      for (p, wall, value) in [(third, 2, "1975-01-02"), (b, 3, "1980-07-15")] {
          field(&c, &sk, &kid, p, wall,
                dob_assertion_body(value, "day", Some("document"), "document-verified"),
                render_dob_twin(value, "day", "document-verified")).await;
      }
      submit_link_event(&c, &sk, &kid, a, third, 10, true).await;
      let left = ChartSet::new([a, third]).unwrap();
      let right = ChartSet::single(b);

      let findings = cross_vetoes(&c, &left, &right).await.unwrap();
      assert_eq!(findings.len(), 1, "exactly the B–third DOB clash");
      let f = &findings[0];
      assert_eq!((f.left, f.right), (third, b), "tagged with the pair it concerns");
      assert_eq!(f.severity, "hard_veto");
      assert_eq!(f.kind, "dob");
  }

  #[tokio::test]
  async fn two_charts_with_nothing_to_compare_have_no_findings() {
      let Some(base) = cs() else {
          eprintln!("skipped: set CAIRN_TEST_PG");
          return;
      };
      let _g = db::test_serial_guard(&base).await.unwrap();
      let c = db::connect_and_load_schema(&base).await.unwrap();
      let (sk, kid, _, _) = setup(&c).await;
      let (a, b) = (fresh(&c, &sk, &kid).await, fresh(&c, &sk, &kid).await);
      assert!(cross_vetoes(&c, &ChartSet::single(a), &ChartSet::single(b)).await.unwrap().is_empty());
  }
  ```

- [ ] **Step 2: Run to verify they fail.** `CAIRN_ALLOW_DB_SKIP=1 cargo test -p cairn-node --lib patient::compare`
  → FAIL to compile (`order_findings` not found). The DB suite fails to compile too.

- [ ] **Step 3: Implement.** Append to `compare.rs` (above the test module):

  ```rust
  /// One `cairn_match_veto` finding between a chart of the LEFT record and a chart of the RIGHT
  /// one, tagged with that pair so the window can say which two charts disagree.
  #[derive(Debug, Clone, PartialEq, Eq)]
  pub struct VetoFinding {
      pub left: Uuid,
      pub right: Uuid,
      /// `identifier`, `dob` or `sex-at-birth` (db/016's closed vocabulary).
      pub kind: String,
      /// `hard_veto` or `degrade_hold`.
      pub severity: String,
      pub subject: String,
      /// db/016's own human-readable reason — shown verbatim, never reworded.
      pub detail: String,
  }

  /// Hard vetoes first, then a stable order (left, right, kind), so the panel always reads the
  /// same way for the same records. Pure.
  pub fn order_findings(mut findings: Vec<VetoFinding>) -> Vec<VetoFinding> {
      findings.sort_by(|x, y| {
          (x.severity != "hard_veto", x.left, x.right, &x.kind)
              .cmp(&(y.severity != "hard_veto", y.left, y.right, &y.kind))
      });
      findings
  }

  // Every left × right pair, in one statement. `l <> r` is belt and braces: two records are two
  // link components, so they share no chart — but a caller that passed overlapping sets must not
  // be told a chart "clashes" with itself.
  const VETO_SQL: &str = "SELECT l::text AS l, r::text AS r, v.veto_kind, v.severity, \
              coalesce(v.subject, '') AS subject, coalesce(v.detail, '') AS detail \
       FROM unnest($1::text[]::uuid[]) AS l \
       CROSS JOIN unnest($2::text[]::uuid[]) AS r \
       CROSS JOIN LATERAL cairn_match_veto(l, r) AS v \
       WHERE l <> r";

  /// Every veto finding between a chart of `left` and a chart of `right`.
  ///
  /// SET AGAINST SET, never chart against chart (design "R2b"): with A open and already linked to
  /// C, linking B to A also joins B to C — so a B–C clash is exactly as relevant as an A–B one.
  /// A hard veto forces a human decision and never an automatic refusal (§5.13): this only
  /// REPORTS; the human may still link.
  pub async fn cross_vetoes<C: GenericClient + Sync>(
      client: &C,
      left: &ChartSet,
      right: &ChartSet,
  ) -> anyhow::Result<Vec<VetoFinding>> {
      let l: Vec<String> = left.members().iter().map(Uuid::to_string).collect();
      let r: Vec<String> = right.members().iter().map(Uuid::to_string).collect();
      let found = client
          .query(VETO_SQL, &[&l, &r])
          .await
          .map_err(|e| LocalDbFault::new("checking the two records for veto findings", e))?;
      let findings = found
          .iter()
          .map(|row| {
              Ok(VetoFinding {
                  left: row.get::<_, String>("l").parse()?,
                  right: row.get::<_, String>("r").parse()?,
                  kind: row.get("veto_kind"),
                  severity: row.get("severity"),
                  subject: row.get("subject"),
                  detail: row.get("detail"),
              })
          })
          .collect::<anyhow::Result<Vec<_>>>()?;
      Ok(order_findings(findings))
  }
  ```

  Bump `COMPARE_LOCAL_DB_FAULT_SITES` to `2` and update its doc line.

- [ ] **Step 4: Run to verify they pass.** The lib test, the `chart_compare` suite and
  `db_errors_stay_legible` (commands as in Task 2). Expected: PASS.

- [ ] **Step 5: Commit.**
  ```bash
  git add crates/cairn-node/src/patient/compare.rs crates/cairn-node/tests/chart_compare.rs crates/cairn-node/tests/db_errors_stay_legible.rs
  git commit -m "feat(R2b-1): cross_vetoes — every veto finding across two records, hard first"
  ```

---

### Task 4: the panel's wording — pure view builders (`link/view.rs`)

**Files:**
- Create: `cairn-gui/cairn-gui-tauri/src/link/view.rs`
- Create: `cairn-gui/cairn-gui-tauri/src/link/mod.rs` (for now only `pub mod view;`)
- Modify: `cairn-gui/cairn-gui-tauri/src/main.rs:24-27` (add `mod link;`)

**Interfaces:**
- Consumes: `cairn_node::patient::compare::{ChartFacts, NameFact, VetoFinding}`,
  `cairn_node::chart_link::LinkEffect`, `cairn_medication_view::ChartSet`,
  `cairn_gui_tab_medications::view::MedListView` (fields `rows`, `withheld_message`,
  `missing_message`; row fields `primary`, `dose`, `sig`, `status_label`),
  `cairn_gui_live::error::data_error_from(&anyhow::Error) -> DataError`,
  `cairn_gui_data::port::DataError`, `crate::funnel::view::{ErrorView, Retry}`,
  `cairn_patient_search::Candidate`.
- Produces (all `pub`, views `#[derive(Debug, Clone, PartialEq, Eq, Serialize)]`):
  ```rust
  pub struct ColumnView { pub patient_id: String, pub heading: String }
  pub struct FactRowView { pub label: String, pub cells: Vec<String> }
  pub struct ComparisonView {
      pub findings: Vec<String>, pub columns: Vec<ColumnView>, pub left_count: usize,
      pub rows: Vec<FactRowView>, pub other_charts: Vec<String>,
      pub other_medications: Vec<String>, pub other_medication_notes: Vec<String>,
      pub problems: Vec<String>, pub can_link: bool,
  }
  pub struct ComparisonParts {
      pub left: Result<Vec<ChartFacts>, String>, pub right: Result<Vec<ChartFacts>, String>,
      pub findings: Result<Vec<VetoFinding>, String>, pub other_medications: Result<MedListView, String>,
  }
  pub struct LinkReportView { pub sentence: String, pub reload: bool }
  pub fn finding_line(f: &VetoFinding) -> String
  pub fn medication_lines(list: &MedListView) -> (Vec<String>, Vec<String>)
  pub fn comparison_view(parts: ComparisonParts, other_charts: &ChartSet) -> ComparisonView
  pub fn link_report(effect: LinkEffect, charts: &ChartSet) -> LinkReportView
  pub fn link_error_view(e: &anyhow::Error) -> ErrorView
  pub fn refused(text: impl Into<String>) -> ErrorView   // Retry::Never
  pub fn fixture_facts(patient: Uuid, name: &str, trust: &str) -> ChartFacts
  pub const OTHER_CHANGED: &str
  pub const ALREADY_IN_RECORD: &str
  pub const NOT_ON_SCREEN: &str
  ```

- [ ] **Step 1: Write the failing tests.** Create `link/view.rs` containing only its module doc, the
  `use` lines, and this test module (the functions come in Step 3):

  ```rust
  #[cfg(test)]
  mod tests {
      use super::*;
      use cairn_node::patient::compare::{FieldFact, NameFact};

      fn id(n: u128) -> Uuid {
          Uuid::from_u128(n)
      }

      fn held(n: u128) -> ChartFacts {
          ChartFacts {
              patient_id: id(n),
              held: true,
              trust: "confirmed".into(),
              names: vec![NameFact { value: "N".into(), use_: Some("legal".into()), provenance: "patient-stated".into() }],
              dob: Some(FieldFact { value: "1950-07-01".into(), provenance: "document-verified".into() }),
              ..ChartFacts::default()
          }
      }

      fn meds() -> MedListView {
          cairn_gui_tab_medications::view::build_view(&cairn_medication_view::fixtures::sample_chart())
      }

      fn parts() -> ComparisonParts {
          ComparisonParts {
              left: Ok(vec![held(1)]),
              right: Ok(vec![held(2)]),
              findings: Ok(vec![]),
              other_medications: Ok(meds()),
          }
      }

      #[test]
      fn a_full_comparison_is_linkable_and_names_the_other_record() {
          let v = comparison_view(parts(), &ChartSet::single(id(2)));
          assert!(v.can_link && v.problems.is_empty());
          assert_eq!(v.left_count, 1);
          assert_eq!(v.columns.len(), 2);
          assert_eq!(v.other_charts, vec![id(2).to_string()], "sent back with the link");
          assert!(v.rows.iter().all(|r| r.cells.len() == 2), "every row has a cell per chart");
      }

      /// Review Focus 4: a comparison read only in part shows what it has, names what it lacks,
      /// and offers NO link — a judgement needs the whole picture.
      #[test]
      fn a_partial_comparison_names_what_is_missing_and_cannot_link() {
          let mut p = parts();
          p.other_medications = Err("connection reset".into());
          let v = comparison_view(p, &ChartSet::single(id(2)));
          assert!(!v.can_link);
          assert_eq!(v.problems.len(), 1);
          assert!(v.problems[0].contains("medications"), "names the part that is missing");
          assert_eq!(v.columns.len(), 2, "the facts that WERE read are still shown");
      }

      #[test]
      fn unreadable_facts_leave_no_columns_and_cannot_link() {
          let mut p = parts();
          p.right = Err("boom".into());
          let v = comparison_view(p, &ChartSet::single(id(2)));
          assert!(!v.can_link);
          assert_eq!(v.left_count, 1);
          assert_eq!(v.columns.len(), 1, "no invented column for a record that was not read");
      }

      /// Principle 4: absence is worded — and differently for a chart this node does not hold.
      #[test]
      fn an_absent_fact_says_not_recorded_or_unknown() {
          let mut unheld = ChartFacts { patient_id: id(2), held: false, trust: "unknown".into(), ..ChartFacts::default() };
          unheld.names.clear();
          let mut p = parts();
          p.right = Ok(vec![unheld]);
          let v = comparison_view(p, &ChartSet::single(id(2)));
          let dob = v.rows.iter().find(|r| r.label == "Date of birth").unwrap();
          assert_eq!(dob.cells[0], "1950-07-01 (document-verified)");
          assert_eq!(dob.cells[1], "unknown — registration not yet received here");
          let mut p = parts();
          p.right = Ok(vec![ChartFacts { patient_id: id(2), held: true, trust: "confirmed".into(), ..ChartFacts::default() }]);
          let v = comparison_view(p, &ChartSet::single(id(2)));
          let dob = v.rows.iter().find(|r| r.label == "Date of birth").unwrap();
          assert_eq!(dob.cells[1], "not recorded");
      }

      #[test]
      fn a_finding_is_a_plain_fact_naming_both_charts() {
          let f = VetoFinding {
              left: id(1), right: id(2), kind: "dob".into(), severity: "hard_veto".into(),
              subject: "dob".into(), detail: "verified dob clash (precision day): 'x' vs 'y'".into(),
          };
          let line = finding_line(&f);
          assert!(line.starts_with("Verified facts differ"), "{line}");
          assert!(line.contains(&id(1).to_string()) && line.contains(&id(2).to_string()));
          assert!(line.contains("verified dob clash"), "db/016's own words, verbatim");
          let hold = finding_line(&VetoFinding { severity: "degrade_hold".into(), ..f });
          assert!(hold.starts_with("Facts differ, not verified"), "{hold}");
      }

      /// Never "no conflicts": an empty finding list renders nothing at all.
      #[test]
      fn no_findings_means_no_lines_and_no_clearance_sentence() {
          let v = comparison_view(parts(), &ChartSet::single(id(2)));
          assert!(v.findings.is_empty());
      }

      #[test]
      fn only_current_medications_are_listed_and_warnings_carry_over() {
          let (lines, _notes) = medication_lines(&meds());
          let current = meds().rows.iter().filter(|r| r.status_label == "current").count();
          assert_eq!(lines.len(), current, "ceased drugs are not 'active medications'");
      }

      #[test]
      fn an_empty_list_says_so() {
          let empty = cairn_gui_tab_medications::view::build_view(
              &cairn_medication_view::PatientMedicationList::empty(ChartSet::single(id(2))),
          );
          let (lines, notes) = medication_lines(&empty);
          assert!(lines.is_empty());
          assert!(notes.iter().any(|n| n.contains("No current medications")), "absence is named");
      }

      #[test]
      fn a_link_that_took_effect_reloads_and_one_outranked_does_not() {
          let set = ChartSet::new([id(1), id(2)]).unwrap();
          let took = link_report(LinkEffect::TookEffect, &set);
          assert!(took.reload && took.sentence.starts_with("Linked"), "{}", took.sentence);
          assert!(took.sentence.contains("2 charts"));
          let lost = link_report(LinkEffect::Outranked, &ChartSet::single(id(1)));
          assert!(!lost.reload, "a disagreement is shown, never reloaded away");
          assert!(lost.sentence.contains("NOT in effect"), "{}", lost.sentence);
      }

      /// Review Focus 5 (outage half): an outage is "not confirmed", never "nothing changed" — a
      /// connection lost mid-commit leaves the outcome unknown.
      #[test]
      fn an_outage_is_worded_not_confirmed_and_retryable() {
          let outage = anyhow::anyhow!("connection reset");
          assert_eq!(link_error_view(&outage).retry, Retry::Now);
          let text = link_error_view(&outage).text;
          assert!(text.contains("not confirmed"), "{text}");
      }
  }
  ```

  (A marked `DeliberateRefusal` cannot be minted outside `cairn-node` — `deliberate_refusal` is
  `pub(crate)` on purpose — so the verdict half of the classification is pinned by Task 1's DB tests
  plus `cairn-gui-live`'s own `data_error_from` tests; this test pins the outage half and that the
  view reuses `data_error_from`.)

- [ ] **Step 2: Run to verify they fail.**
  `cd cairn-gui && CAIRN_ALLOW_DB_SKIP=1 cargo test -p cairn-gui-tauri link::view` → FAIL to compile.

- [ ] **Step 3: Implement** above the test module in `link/view.rs`:

  ```rust
  //! Every sentence the "Same person as…" panel shows, as pure functions (R2b-1).
  //!
  //! The webview renders and decides nothing (see `src-ui/main.js`'s header): on this panel the
  //! wording IS the safety content — the panel's safety is what it SHOWS, never an "are you
  //! sure?" (principle 3) — so every sentence is built and tested here.
  use crate::funnel::view::{ErrorView, Retry};
  use cairn_gui_data::port::DataError;
  use cairn_gui_tab_medications::view::MedListView;
  use cairn_medication_view::ChartSet;
  use cairn_node::chart_link::LinkEffect;
  use cairn_node::patient::compare::{ChartFacts, NameFact, VetoFinding};
  use serde::Serialize;
  use uuid::Uuid;

  /// Refused because the picked chart is already one of this record's charts.
  pub const ALREADY_IN_RECORD: &str =
      "that chart is already part of this record — there is nothing to link";
  /// Refused because the picked chart was never in a list on screen.
  pub const NOT_ON_SCREEN: &str = "that chart was not in a list on screen — search again";
  /// Refused because the OTHER record's charts changed between Compare and Link (decision 3,
  /// widened to the right-hand side: a peer's link landing mid-review must not clip a chart into
  /// this record sight unseen).
  pub const OTHER_CHANGED: &str =
      "the other record changed while you were comparing — nothing was done; compare again";

  /// One chart's column heading.
  #[derive(Debug, Clone, PartialEq, Eq, Serialize)]
  pub struct ColumnView {
      pub patient_id: String,
      pub heading: String,
  }

  /// One fact kind across every chart, a cell per column.
  #[derive(Debug, Clone, PartialEq, Eq, Serialize)]
  pub struct FactRowView {
      pub label: String,
      pub cells: Vec<String>,
  }

  /// What `compare_records` hands the webview.
  #[derive(Debug, Clone, PartialEq, Eq, Serialize)]
  pub struct ComparisonView {
      /// Veto findings, worded; hard ones first. Empty → render nothing (never "no conflicts").
      pub findings: Vec<String>,
      /// Left record's charts first, then the right record's.
      pub columns: Vec<ColumnView>,
      /// How many of `columns` belong to "This record".
      pub left_count: usize,
      pub rows: Vec<FactRowView>,
      /// The OTHER record's chart set as displayed — the webview sends it back with the link.
      pub other_charts: Vec<String>,
      /// The other record's CURRENT medications, one line each.
      pub other_medications: Vec<String>,
      /// Its list's own warnings (withheld / missing), or the empty-list sentence.
      pub other_medication_notes: Vec<String>,
      /// What could NOT be read. Non-empty → `can_link` is false.
      pub problems: Vec<String>,
      pub can_link: bool,
  }

  /// The four reads a comparison is built from, each of which may have failed on its own.
  pub struct ComparisonParts {
      pub left: Result<Vec<ChartFacts>, String>,
      pub right: Result<Vec<ChartFacts>, String>,
      pub findings: Result<Vec<VetoFinding>, String>,
      pub other_medications: Result<MedListView, String>,
  }

  /// What `link_records` hands back: the sentence for the outcome line, and whether the chart
  /// should be re-read (it should whenever the record may have changed).
  #[derive(Debug, Clone, PartialEq, Eq, Serialize)]
  pub struct LinkReportView {
      pub sentence: String,
      pub reload: bool,
  }

  /// A refusal the clinician cannot retry as-is.
  pub fn refused(text: impl Into<String>) -> ErrorView {
      ErrorView { text: text.into(), retry: Retry::Never }
  }

  /// One veto finding as a plain fact: which kind of disagreement, db/016's own words, and the
  /// two charts. No judgement words ("likely", "probably different") — a hard veto forces a
  /// human decision, it does not make it (§5.13). An unknown severity is shown verbatim.
  pub fn finding_line(f: &VetoFinding) -> String {
      let label = match f.severity.as_str() {
          "hard_veto" => "Verified facts differ",
          "degrade_hold" => "Facts differ, not verified",
          other => other,
      };
      format!("{label} — {} — between chart {} and chart {}", f.detail, f.left, f.right)
  }

  /// The word for an absent fact: "not recorded" on a chart held here; "unknown" on one whose
  /// registration has not arrived (nothing says it was never recorded). Principle 4.
  fn absent(f: &ChartFacts) -> String {
      if f.held {
          "not recorded".into()
      } else {
          "unknown — registration not yet received here".into()
      }
  }

  /// Join a list of values, or the absence word when there are none.
  fn joined(f: &ChartFacts, values: Vec<String>) -> String {
      if values.is_empty() {
          absent(f)
      } else {
          values.join("; ")
      }
  }

  fn name_text(n: &NameFact) -> String {
      let use_ = n.use_.as_deref().unwrap_or("use not recorded");
      format!("{} ({use_}, {})", n.value, n.provenance)
  }

  /// The column heading: the first current name (or its absence) and the chart id, whole — it is
  /// what ties the column to each medication row's source label.
  fn heading(f: &ChartFacts) -> String {
      let name = f.names.first().map(|n| n.value.clone()).unwrap_or_else(|| absent(f));
      format!("{name} · chart {}", f.patient_id)
  }

  /// The fact rows, in the order a front sheet is read. Each closure renders one chart's cell.
  fn fact_rows(charts: &[ChartFacts]) -> Vec<FactRowView> {
      type Cell = fn(&ChartFacts) -> String;
      let kinds: [(&str, Cell); 7] = [
          ("Names", |f| joined(f, f.names.iter().map(name_text).collect())),
          // An empty alias pool is a fact ("none"), not an unknown.
          ("Earlier recorded names", |f| {
              if f.aliases.is_empty() { "none".into() } else { f.aliases.join("; ") }
          }),
          ("Date of birth", |f| {
              f.dob.as_ref().map(|d| format!("{} ({})", d.value, d.provenance)).unwrap_or_else(|| absent(f))
          }),
          ("Sex at birth", |f| {
              f.sex_at_birth.as_ref().map(|d| format!("{} ({})", d.value, d.provenance)).unwrap_or_else(|| absent(f))
          }),
          ("Identifiers", |f| {
              joined(f, f.identifiers.iter().map(|i| format!("{}: {} ({})", i.system, i.value, i.provenance)).collect())
          }),
          ("Addresses", |f| {
              joined(f, f.addresses.iter().map(|a| {
                  format!("{} ({})", a.display, a.use_.as_deref().unwrap_or(&a.provenance))
              }).collect())
          }),
          ("Identity", |f| f.trust.clone()),
      ];
      kinds
          .iter()
          .map(|(label, cell)| FactRowView {
              label: (*label).into(),
              cells: charts.iter().map(cell).collect(),
          })
          .collect()
  }

  /// The other record's CURRENT medications as one line each, plus the notes to show before
  /// them (the list's own withheld/missing warnings, or the named absence of any drug).
  pub fn medication_lines(list: &MedListView) -> (Vec<String>, Vec<String>) {
      let lines: Vec<String> = list
          .rows
          .iter()
          .filter(|r| r.status_label == "current")
          .map(|r| format!("{} — {} — {}", r.primary, r.dose, r.sig))
          .collect();
      let mut notes: Vec<String> = [&list.withheld_message, &list.missing_message]
          .into_iter()
          .flatten()
          .cloned()
          .collect();
      if lines.is_empty() {
          notes.push("No current medications recorded on the other record.".into());
      }
      (lines, notes)
  }

  /// Assemble the panel from whatever was read. Pure: the availability rule (show what was read,
  /// name what was not, offer no link unless everything was read) is tested with no database.
  pub fn comparison_view(parts: ComparisonParts, other_charts: &ChartSet) -> ComparisonView {
      let mut problems = vec![];
      let mut take = |r: Result<Vec<ChartFacts>, String>, what: &str| match r {
          Ok(v) => v,
          Err(e) => {
              problems.push(format!("{what} could not be read: {e}"));
              vec![]
          }
      };
      let left = take(parts.left, "This record's identity facts");
      let right = take(parts.right, "The other record's identity facts");
      let findings = match parts.findings {
          Ok(f) => f.iter().map(finding_line).collect(),
          Err(e) => {
              problems.push(format!("The check for disagreeing facts could not be run: {e}"));
              vec![]
          }
      };
      let (other_medications, other_medication_notes) = match parts.other_medications {
          Ok(list) => medication_lines(&list),
          Err(e) => {
              problems.push(format!("The other record's medications could not be read: {e}"));
              (vec![], vec![])
          }
      };
      let charts: Vec<ChartFacts> = left.iter().chain(right.iter()).cloned().collect();
      ComparisonView {
          findings,
          columns: charts
              .iter()
              .map(|f| ColumnView { patient_id: f.patient_id.to_string(), heading: heading(f) })
              .collect(),
          left_count: left.len(),
          rows: fact_rows(&charts),
          other_charts: other_charts.members().iter().map(Uuid::to_string).collect(),
          other_medications,
          other_medication_notes,
          can_link: problems.is_empty(),
          problems,
      }
  }

  /// What the link did, as the outcome line says it. Never "linked" for a link that did not
  /// take effect (R2a: recorded is not took effect).
  pub fn link_report(effect: LinkEffect, charts: &ChartSet) -> LinkReportView {
      match effect {
          LinkEffect::TookEffect => LinkReportView {
              sentence: format!(
                  "Linked — this record now combines {} charts.",
                  charts.members().len()
              ),
              reload: true,
          },
          LinkEffect::Outranked => LinkReportView {
              sentence: "Recorded, but NOT in effect: a later judgement on this pair says these \
                         are different people. The two judgements disagree — settle it with \
                         the person who made the other one; pressing Link again changes nothing."
                  .into(),
              reload: false,
          },
          // R2a never returns this for a link; worded honestly in case it ever does.
          LinkEffect::StillJoined => LinkReportView {
              sentence: "Recorded, but the record did not change the way a link should — the \
                         chart is being re-read so you can see what it now combines."
                  .into(),
              reload: true,
          },
      }
  }

  /// A failed link, worded by the SAME classification the funnel uses (`data_error_from`: a
  /// `P0001` floor refusal or a marked `cairn-node` verdict is a verdict; anything else an
  /// outage) and the funnel's `Retry` vocabulary (#702). An outage is "not confirmed", never
  /// "nothing changed": a connection lost during the commit leaves the outcome unknown, and the
  /// node's own message (carried in `t`) says to check before retrying. A second identical link
  /// is harmless (the same standing state), so `Retry::Now` is safe.
  pub fn link_error_view(e: &anyhow::Error) -> ErrorView {
      match cairn_gui_live::error::data_error_from(e) {
          DataError::Refused(t) => ErrorView {
              text: format!("The link was refused: {t}"),
              retry: Retry::Never,
          },
          DataError::NotProvisioned(t) => ErrorView {
              text: format!("This node cannot record the link until an operator acts: {t}"),
              retry: Retry::AfterOperator,
          },
          DataError::Unavailable(t) => ErrorView {
              text: format!("The link was not confirmed: {t}"),
              retry: Retry::Now,
          },
          DataError::NotFound => refused("The link was not recorded."),
      }
  }

  /// A fixture chart's facts for `--mock`, where there is no database: the name the list showed,
  /// nothing else. Enough to walk and time the panel; fixture mode refuses the link itself.
  pub fn fixture_facts(patient: Uuid, name: &str, trust: &str) -> ChartFacts {
      ChartFacts {
          patient_id: patient,
          held: true,
          trust: trust.into(),
          names: vec![NameFact { value: name.into(), use_: None, provenance: "fixture".into() }],
          ..ChartFacts::default()
      }
  }
  ```

  Create `link/mod.rs` with a module doc line and `pub mod view;`. Add `mod link;` to `main.rs`.
  Check `cairn_gui_live::error` is `pub mod error` in `cairn-gui-live/src/lib.rs`; if it is private,
  use the re-export the funnel uses (grep `data_error_from` in `cairn-gui-tauri/src`) rather than
  widening visibility.

- [ ] **Step 4: Run to verify they pass.** Same command → PASS. `cargo fmt --all` in `cairn-gui/`.
  `cairn-gui-tauri` is a BINARY crate, so items only its tests use are dead code to clippy. Run
  `cargo clippy -p cairn-gui-tauri --all-targets -- -D warnings`; if it flags unused items, do NOT
  add `#[allow(dead_code)]` — commit this task without the clippy gate and let Task 5 (which uses
  every item) be the first commit that must pass clippy. Say so in the commit message.

- [ ] **Step 5: Commit.**
  ```bash
  git add cairn-gui/cairn-gui-tauri/src/link cairn-gui/cairn-gui-tauri/src/main.rs
  git commit -m "feat(R2b-1): the comparison panel's wording as pure view builders"
  ```

---

### Task 5: the window commands `compare_records` and `link_records`

**Files:**
- Modify: `cairn-gui/cairn-gui-tauri/src/link/mod.rs`
- Modify: `cairn-gui/cairn-gui-tauri/src/commands.rs:389` (`async fn read_chart_of` → `pub(crate) async fn`)
- Modify: `cairn-gui/cairn-gui-tauri/src/main.rs:86-99` (register two handlers)

**Interfaces:**
- Consumes: `AppState::{displayed_patient, live_key, is_mock, db, shown, chart, node_origin}`,
  `crate::chart_set::check_displayed_set(&ChartSet, &[String]) -> Result<ChartSet, String>`,
  `crate::commands::read_chart_of(&AppState, Uuid) -> Result<PatientMedicationList, String>`,
  `cairn_gui_tab_medications::view::build_view`, `cairn_node::patient::person::person_charts`,
  `cairn_node::patient::compare::{chart_facts, cross_vetoes}`,
  `cairn_node::chart_link::{link_charts, Reviewer}`, everything Task 4 produces.
- Produces: `pub async fn compare_impl(state: &AppState, patient_id: &str, charts: Vec<String>, other_id: &str) -> Result<ComparisonView, ErrorView>`;
  `pub async fn link_impl(state: &AppState, patient_id: &str, charts: Vec<String>, other_id: &str, other_charts: Vec<String>) -> Result<LinkReportView, ErrorView>`;
  Tauri commands `compare_records(patient_id, charts, other_id)` and
  `link_records(patient_id, charts, other_id, other_charts)` (JS keys `patientId`, `charts`,
  `otherId`, `otherCharts`).

- [ ] **Step 1: Write the failing tests** at the bottom of `link/mod.rs`:

  ```rust
  #[cfg(test)]
  mod tests {
      use super::*;
      use cairn_patient_search::{Candidate, TrustState};

      fn fixture() -> Uuid {
          cairn_gui_data::mock::fixtures::FIXTURE_UUID.parse().unwrap()
      }

      /// A window open on the fixture chart, with `others` shown by a list on screen.
      async fn window_showing(others: &[Uuid]) -> AppState {
          let state = AppState::mock(Some(fixture()));
          let mut shown = state.shown.lock().await;
          for id in others {
              shown.insert(*id, Candidate {
                  patient_id: *id,
                  display_name: "Other Person".into(),
                  age: None,
                  trust: TrustState::Confirmed,
                  last_activity: None,
                  locale: None,
                  photo_ref: None,
              });
          }
          drop(shown);
          state
      }

      fn on_screen() -> (String, Vec<String>) {
          (fixture().to_string(), vec![fixture().to_string()])
      }

      #[tokio::test]
      async fn compare_is_bound_to_the_chart_on_screen() {
          let other = Uuid::from_u128(2);
          let state = window_showing(&[other]).await;
          let err = compare_impl(&state, &Uuid::from_u128(9).to_string(), vec![], &other.to_string())
              .await
              .unwrap_err();
          assert!(err.text.contains("not the chart"), "{}", err.text);
      }

      #[tokio::test]
      async fn compare_refuses_a_changed_set() {
          let other = Uuid::from_u128(2);
          let state = window_showing(&[other]).await;
          let (p, _) = on_screen();
          let err = compare_impl(&state, &p, vec![p.clone(), Uuid::from_u128(7).to_string()], &other.to_string())
              .await
              .unwrap_err();
          assert!(err.text.contains("linked charts changed"), "{}", err.text);
      }

      #[tokio::test]
      async fn compare_refuses_a_chart_no_list_showed() {
          let state = window_showing(&[]).await;
          let (p, charts) = on_screen();
          let err = compare_impl(&state, &p, charts, &Uuid::from_u128(2).to_string()).await.unwrap_err();
          assert_eq!(err.text, view::NOT_ON_SCREEN);
      }

      /// Review Focus 1: the in-chart search returns the opened chart itself.
      #[tokio::test]
      async fn compare_refuses_a_chart_already_in_the_record() {
          let state = window_showing(&[fixture()]).await;
          let (p, charts) = on_screen();
          let err = compare_impl(&state, &p, charts, &p).await.unwrap_err();
          assert_eq!(err.text, view::ALREADY_IN_RECORD);
      }

      #[tokio::test]
      async fn a_fixture_comparison_has_a_column_per_chart_and_can_be_walked() {
          let other = Uuid::from_u128(2);
          let state = window_showing(&[other]).await;
          let (p, charts) = on_screen();
          let v = compare_impl(&state, &p, charts, &other.to_string()).await.unwrap();
          assert_eq!(v.columns.len(), 2);
          assert_eq!(v.left_count, 1);
          assert_eq!(v.other_charts, vec![other.to_string()]);
          assert!(v.can_link, "fixture mode reads everything it has; it refuses only the WRITE");
      }

      /// Review Focus 2: the other record changed between Compare and Link.
      #[tokio::test]
      async fn link_refuses_when_the_other_record_changed() {
          let other = Uuid::from_u128(2);
          let state = window_showing(&[other]).await;
          let (p, charts) = on_screen();
          let err = link_impl(&state, &p, charts, &other.to_string(),
                              vec![other.to_string(), Uuid::from_u128(4).to_string()])
              .await
              .unwrap_err();
          assert_eq!(err.text, view::OTHER_CHANGED);
      }

      /// Every check runs BEFORE fixture mode's own refusal, so the fixture refusal is proof
      /// that they all passed.
      #[tokio::test]
      async fn link_in_fixture_mode_passes_every_check_then_refuses_to_write() {
          let other = Uuid::from_u128(2);
          let state = window_showing(&[other]).await;
          let (p, charts) = on_screen();
          let err = link_impl(&state, &p, charts, &other.to_string(), vec![other.to_string()])
              .await
              .unwrap_err();
          assert!(err.text.contains("fixture mode"), "{}", err.text);
      }

      #[tokio::test]
      async fn link_refuses_a_chart_no_list_showed_before_anything_else() {
          let state = window_showing(&[]).await;
          let (p, charts) = on_screen();
          let other = Uuid::from_u128(2).to_string();
          let err = link_impl(&state, &p, charts, &other, vec![other.clone()]).await.unwrap_err();
          assert_eq!(err.text, view::NOT_ON_SCREEN);
      }
  }
  ```

- [ ] **Step 2: Run to verify they fail.** `cd cairn-gui && CAIRN_ALLOW_DB_SKIP=1 cargo test -p cairn-gui-tauri link::tests` → FAIL to compile.

- [ ] **Step 3: Implement** in `link/mod.rs` above the tests:

  ```rust
  //! "Same person as…" — compare two records side by side and link them (repair path R2b-1,
  //! ADR-0076 decisions 3–5; design "R2b — the window's gesture").
  //!
  //! Paper counterpart: fetch the other folder, lay the front sheets side by side, clip. Three
  //! acts — find (the in-chart search, which reuses `browse`: it adds its results to
  //! `AppState::shown` and never switches the open chart), Compare, Link. The Link click IS the
  //! signature under the unlocked key (ADR-0053); there is no confirmation dialog (principle 3).
  //!
  //! Every command here applies the chart-command rules IN THIS ORDER, and each test pins one:
  //! the chart on screen (`displayed_patient`), the displayed set (`check_displayed_set`), the
  //! other chart was shown by a list (`shown`), it is not already in the record, and — for the
  //! link — the OTHER record is still the set the clinician compared (decision 3 widened to the
  //! right-hand side). Only then fixture mode, then the key.
  pub mod view;

  use crate::chart_set::check_displayed_set;
  use crate::commands::read_chart_of;
  use crate::funnel::view::ErrorView;
  use crate::state::{AppState, Now};
  use cairn_medication_view::ChartSet;
  use uuid::Uuid;
  use view::{
      comparison_view, fixture_facts, link_error_view, link_report, refused, ComparisonParts,
      ComparisonView, LinkReportView, ALREADY_IN_RECORD, NOT_ON_SCREEN, OTHER_CHANGED,
  };

  /// A read's error as the text a comparison part carries (the operator chain, legible).
  /// A plain generic fn, not a closure: it is used at two different `T`s.
  fn as_text<T>(r: anyhow::Result<T>) -> Result<T, String> {
      r.map_err(|e| cairn_node::db_diagnosis::operator_chain(&e))
  }

  /// The record `patient` belongs to: its link component live, itself alone in fixture mode
  /// (fixture charts are never linked). A failure here is a READ that failed — nothing was
  /// judged — so it is worded as one, retryable, never as a link outcome.
  async fn chart_set_of(state: &AppState, patient: Uuid) -> Result<ChartSet, ErrorView> {
      let Some(db) = state.db.as_ref() else {
          return Ok(ChartSet::single(patient));
      };
      let db = db.lock().await;
      cairn_node::patient::person::person_charts(&*db, patient)
          .await
          .map_err(|e| ErrorView {
              text: format!(
                  "Could not read which charts this record combines — nothing was done: {}",
                  cairn_node::db_diagnosis::operator_chain(&e)
              ),
              retry: crate::funnel::view::Retry::Now,
          })
  }

  /// The four screen checks both commands share. Returns the opened chart, its displayed set,
  /// and the other chart (with the name the list showed, for fixture mode).
  async fn resolve_pair(
      state: &AppState,
      patient_id: &str,
      charts: &[String],
      other_id: &str,
  ) -> Result<(Uuid, ChartSet, Uuid, String), ErrorView> {
      let patient = state.displayed_patient(patient_id).await.map_err(refused)?;
      let left = check_displayed_set(&chart_set_of(state, patient).await?, charts).map_err(refused)?;
      let other: Uuid = other_id.parse().map_err(|_| refused(NOT_ON_SCREEN))?;
      let shown_name = state
          .shown
          .lock()
          .await
          .get(&other)
          .map(|c| c.display_name.clone())
          .ok_or_else(|| refused(NOT_ON_SCREEN))?;
      if left.contains(&other) {
          return Err(refused(ALREADY_IN_RECORD));
      }
      Ok((patient, left, other, shown_name))
  }

  /// "Compare" — read both records and build the panel.
  pub async fn compare_impl(
      state: &AppState,
      patient_id: &str,
      charts: Vec<String>,
      other_id: &str,
  ) -> Result<ComparisonView, ErrorView> {
      let (patient, left, other, shown_name) = resolve_pair(state, patient_id, &charts, other_id).await?;
      let right = chart_set_of(state, other).await?;
      // The other record's list: the SAME custody-applied read opening it would give (§5.9).
      let meds = read_chart_of(state, other)
          .await
          .map(|list| cairn_gui_tab_medications::view::build_view(&list));
      let parts = match state.db.as_ref() {
          None => {
              let header = state.chart.lock().await.as_ref().map(|c| c.header.name.clone());
              ComparisonParts {
                  left: Ok(vec![fixture_facts(patient, &header.unwrap_or_default(), "confirmed")]),
                  right: Ok(vec![fixture_facts(other, &shown_name, "confirmed")]),
                  findings: Ok(vec![]),
                  other_medications: meds,
              }
          }
          Some(db) => {
              let db = db.lock().await;
              ComparisonParts {
                  left: as_text(cairn_node::patient::compare::chart_facts(&*db, &left).await),
                  right: as_text(cairn_node::patient::compare::chart_facts(&*db, &right).await),
                  findings: as_text(cairn_node::patient::compare::cross_vetoes(&*db, &left, &right).await),
                  other_medications: meds,
              }
          }
      };
      Ok(comparison_view(parts, &right))
  }

  /// "Link — same person" — the attested judgement, then what it did.
  pub async fn link_impl(
      state: &AppState,
      patient_id: &str,
      charts: Vec<String>,
      other_id: &str,
      other_charts: Vec<String>,
  ) -> Result<LinkReportView, ErrorView> {
      let (patient, _left, other, _) = resolve_pair(state, patient_id, &charts, other_id).await?;
      check_displayed_set(&chart_set_of(state, other).await?, &other_charts)
          .map_err(|_| refused(OTHER_CHANGED))?;
      if state.is_mock() {
          return Err(refused("fixture mode: this window is showing mock data and cannot write"));
      }
      // Linking is a clinical act, so taking the key counts as activity (`live_key`).
      let (human_sk, human_kid) = state
          .live_key(Now::read())
          .await
          .ok_or_else(|| refused("your signing key is locked — unlock it to link these charts"))?;
      let mut db = state
          .db
          .as_ref()
          .ok_or_else(|| refused("no database connection"))?
          .lock()
          .await;
      let reviewer = cairn_node::chart_link::Reviewer { human_sk: &human_sk, human_kid: &human_kid };
      // No gesture-timing row: db/044's `gesture_kind` CHECK admits only signoff/cease; the
      // runbook's stopwatch measures this gesture (design "R2b").
      let outcome = cairn_node::chart_link::link_charts(&mut db, patient, other, &reviewer, &state.node_origin)
          .await
          .map_err(|e| link_error_view(&e))?;
      Ok(link_report(outcome.effect, &outcome.charts))
  }

  // ---- Tauri forwarders (camelCase JS keys → snake_case parameters). ----

  #[tauri::command]
  pub async fn compare_records(
      state: tauri::State<'_, AppState>,
      patient_id: String,
      charts: Vec<String>,
      other_id: String,
  ) -> Result<ComparisonView, ErrorView> {
      compare_impl(&state, &patient_id, charts, &other_id).await
  }

  #[tauri::command]
  pub async fn link_records(
      state: tauri::State<'_, AppState>,
      patient_id: String,
      charts: Vec<String>,
      other_id: String,
      other_charts: Vec<String>,
  ) -> Result<LinkReportView, ErrorView> {
      link_impl(&state, &patient_id, charts, &other_id, other_charts).await
  }
  ```

  Make `read_chart_of` `pub(crate)` in `commands.rs`, and add `link::compare_records,` and
  `link::link_records,` to `generate_handler![…]` in `main.rs`. If `Now` is not exported from
  `state.rs` as used above, import it the way `commands.rs` does. If `ErrorView` lacks
  `Deserialize`/`Clone` for Tauri's error return, check how `funnel::commands::browse` returns
  `Result<BrowseView, ErrorView>` and follow it exactly.

- [ ] **Step 4: Run to verify they pass.** `cd cairn-gui && CAIRN_ALLOW_DB_SKIP=1 cargo test -p cairn-gui-tauri`
  → PASS. `cargo clippy -p cairn-gui-tauri --all-targets --locked -- -D warnings` → clean.
  `wc -l cairn-gui-tauri/src/link/*.rs` → each under 500 (split the fixture branch into a helper if not).

- [ ] **Step 5: Commit.**
  ```bash
  git add cairn-gui/cairn-gui-tauri/src
  git commit -m "feat(R2b-1): compare_records / link_records — both records named, then the attested link"
  ```

---

### Task 6: the panel in the webview

**Files:**
- Modify: `cairn-gui/cairn-gui-tauri/src-ui/index.html` (a button in `#identity`; `#link-panel` between `</header>` and `#session`… see below; load `link.js`)
- Create: `cairn-gui/cairn-gui-tauri/src-ui/link.js`
- Modify: `cairn-gui/cairn-gui-tauri/src-ui/main.js` (`renderLockState` tells the panel the lock state)
- Modify: `cairn-gui/cairn-gui-tauri/src-ui/style.css` (column-group heading, panel spacing)

**Interfaces:**
- Consumes: Tauri commands `browse` (`{form: {revision, raw_name, birth_date}}` → `BrowseView
  {revision, candidates:[{patient_id,name,age,trust}], summary, incomplete_reason}`),
  `compare_records`, `link_records` (Task 5); globals from `main.js`: `el`, `setMessage`, `say`,
  `refresh`, `renderedPatient`, `renderedCharts`.
- Produces: `updateLinkLock(unlocked)` (global, called by `renderLockState`); `closeLinkPanel()`
  (global, called by `funnel.js`'s chart-closing path so a panel never outlives its chart).

- [ ] **Step 1: Markup.** In `index.html`, inside `#identity` right before the "Find another
  patient" button, add `<button id="same-person" type="button">Same person as…</button>`. After
  `</header>` (before `#session`), add:

  ```html
        <!-- "Same person as…" (R2b-1). An ordinary section, never a dialog: nothing blocks the
             chart, and closing it just hides it. DOM ORDER IS CLINICAL — the disagreeing facts
             come before the table, so a screen reader meets them first. Its safety is what it
             SHOWS; there is no "are you sure?" (principle 3). -->
        <section id="link-panel" aria-labelledby="link-heading" hidden>
          <h2 id="link-heading" tabindex="-1">Is this the same person as another chart?</h2>
          <form id="link-search" autocomplete="off">
            <label for="link-name">Part of a name</label>
            <input id="link-name" type="text" />
            <label for="link-dob">Date of birth (optional)</label>
            <input id="link-dob" type="text" placeholder="YYYY-MM-DD" />
          </form>
          <p id="link-search-status" role="status" aria-live="polite"></p>
          <ul id="link-candidates" aria-label="Other charts matching this search"></ul>
          <p id="link-problems" role="alert" hidden></p>
          <ul id="link-findings" role="alert" aria-label="Facts that disagree between the two records" hidden></ul>
          <table id="link-table" hidden>
            <caption>The two records, side by side</caption>
            <thead></thead>
            <tbody></tbody>
          </table>
          <section id="link-other-meds-section" aria-labelledby="link-other-meds-heading" hidden>
            <h3 id="link-other-meds-heading">On the other record — not part of this one until linked</h3>
            <ul id="link-other-meds-notes"></ul>
            <ul id="link-other-meds"></ul>
          </section>
          <button id="link-confirm" type="button" hidden>Link — same person</button>
          <p id="link-status" role="status" aria-live="polite"></p>
          <button id="link-close" type="button">Close comparison</button>
        </section>
  ```

  and `<script src="link.js"></script>` after `funnel.js`.

- [ ] **Step 2: `link.js`.**

  ```js
  // "Same person as…" (repair path R2b-1). Renders and decides nothing: every sentence comes
  // from Rust (`link/view.rs`), every check from the backend (`link/mod.rs`). Classic script,
  // loaded after main.js and funnel.js, sharing their scope (`el`, `setMessage`, `say`,
  // `refresh`, `renderedPatient`, `renderedCharts`).
  "use strict";

  /** Browse answers older than the last keystroke are dropped. */
  let linkRevision = 0;
  /** What the panel compared — sent back with Link so the backend can refuse a changed record. */
  let compared = null; // { otherId, otherCharts }
  let keyUnlocked = false;

  function openLinkPanel() {
    compared = null;
    el("link-candidates").replaceChildren();
    clearComparison();
    el("link-panel").hidden = false;
    el("link-heading").focus();
  }

  function closeLinkPanel() {
    el("link-panel").hidden = true;
    compared = null;
    clearComparison();
    el("same-person").focus();
  }

  function clearComparison() {
    for (const id of ["link-findings", "link-table", "link-other-meds-section", "link-confirm", "link-problems"]) {
      el(id).hidden = true;
    }
    el("link-findings").replaceChildren();
    el("link-table").tHead.replaceChildren();
    el("link-table").tBodies[0].replaceChildren();
    setMessage(el("link-status"), "");
  }

  async function runLinkSearch() {
    const revision = ++linkRevision;
    const form = { revision, raw_name: el("link-name").value, birth_date: el("link-dob").value };
    const list = el("link-candidates");
    if (!form.raw_name.trim() && !form.birth_date.trim()) {
      list.replaceChildren();
      return;
    }
    try {
      const view = await invoke("browse", { form });
      if (view.revision !== linkRevision) return;
      // Charts already in this record are left off: there is nothing to compare them against.
      const inRecord = new Set(renderedCharts || []);
      list.replaceChildren(
        ...view.candidates.filter((c) => !inRecord.has(c.patient_id)).map((c) => {
          const li = document.createElement("li");
          const b = document.createElement("button");
          b.type = "button";
          b.textContent = "Compare: " + c.name + " — " + c.age + " — identity " + c.trust;
          b.addEventListener("click", () => compare(c.patient_id));
          li.append(b);
          return li;
        }),
      );
      el("link-search-status").textContent = view.summary;
    } catch (failure) {
      if (revision !== linkRevision) return;
      list.replaceChildren();
      el("link-search-status").textContent = failure.text || String(failure);
    }
  }

  async function compare(otherId) {
    clearComparison();
    try {
      const view = await invoke("compare_records", {
        patientId: renderedPatient, charts: renderedCharts, otherId,
      });
      renderComparison(view);
      compared = { otherId, otherCharts: view.other_charts };
    } catch (failure) {
      el("link-status").textContent = failure.text || String(failure);
    }
  }

  function renderComparison(view) {
    const problems = el("link-problems");
    problems.textContent = view.problems.join(" ");
    problems.hidden = view.problems.length === 0;

    const findings = el("link-findings");
    findings.replaceChildren(...view.findings.map((f) => cell("li", f)));
    findings.hidden = view.findings.length === 0; // never a "no conflicts" line

    const table = el("link-table");
    const groups = document.createElement("tr");
    groups.append(cell("td", ""));
    groups.append(cell("th", "This record", { scope: "colgroup", colspan: String(view.left_count) }));
    const right = view.columns.length - view.left_count;
    if (right > 0) groups.append(cell("th", "Other record", { scope: "colgroup", colspan: String(right) }));
    const heads = document.createElement("tr");
    heads.append(cell("td", ""));
    for (const col of view.columns) heads.append(cell("th", col.heading, { scope: "col" }));
    table.tHead.replaceChildren(groups, heads);
    table.tBodies[0].replaceChildren(
      ...view.rows.map((row) => {
        const tr = document.createElement("tr");
        tr.append(cell("th", row.label, { scope: "row" }));
        for (const text of row.cells) tr.append(cell("td", text));
        return tr;
      }),
    );
    table.hidden = view.columns.length === 0;

    el("link-other-meds-notes").replaceChildren(...view.other_medication_notes.map((n) => cell("li", n)));
    el("link-other-meds").replaceChildren(...view.other_medications.map((m) => cell("li", m)));
    el("link-other-meds-section").hidden = false;

    // No Link button unless the whole comparison was read (design: a judgement needs the whole picture).
    el("link-confirm").hidden = !view.can_link;
    updateLinkLock(keyUnlocked);
  }

  function updateLinkLock(unlocked) {
    keyUnlocked = Boolean(unlocked);
    el("link-confirm").textContent = keyUnlocked
      ? "Link — same person"
      : "Link — same person (unlock your signing key first)";
  }

  async function linkCompared() {
    if (compared === null) return;
    try {
      const report = await invoke("link_records", {
        patientId: renderedPatient, charts: renderedCharts,
        otherId: compared.otherId, otherCharts: compared.otherCharts,
      });
      say(report.sentence);
      if (report.reload) {
        closeLinkPanel();
        await refresh();
      } else {
        el("link-status").textContent = report.sentence; // Outranked: keep the panel open
      }
    } catch (failure) {
      el("link-status").textContent = failure.text || String(failure);
    }
  }

  el("same-person").addEventListener("click", openLinkPanel);
  el("link-close").addEventListener("click", closeLinkPanel);
  el("link-confirm").addEventListener("click", linkCompared);
  el("link-search").addEventListener("input", runLinkSearch);
  el("link-search").addEventListener("submit", (e) => e.preventDefault());
  el("link-panel").addEventListener("keydown", (e) => {
    if (e.key === "Escape") closeLinkPanel();
  });
  ```

  `cell` is defined in `main.js` (same shared scope). In `main.js`'s `renderLockState`, after the
  unlocked branch sets the text, add: `if (typeof updateLinkLock === "function") updateLinkLock(lock.unlocked);`.
  In `funnel.js`, find where the chart is closed (the `close_chart` path that calls `clearChart()`)
  and add `if (typeof closeLinkPanel === "function" && !el("link-panel").hidden) closeLinkPanel();`
  beside `clearChart()` — a panel must never outlive its chart.

- [ ] **Step 3: Style.** In `style.css`, add minimal rules matching the file's existing tokens:
  `#link-panel { margin-block: 1rem; }`, `#link-table th[scope="colgroup"] { text-align: start; }`,
  `#link-findings li { font-weight: 600; }`. Reuse existing colour variables only.

- [ ] **Step 4: Walk it headless** (the memory recipe "webview mock walk"): copy `src-ui/` to the
  scratchpad, stub `window.__TAURI__.core.invoke` BEFORE `main.js` with payloads shaped like the
  Rust types (a `ComparisonView` with one hard finding, 2 columns, `can_link: true`; a
  `LinkReportView` for TookEffect and one for Outranked), serve it (`python3 -m http.server`), drive
  with Playwright. Check, and record the result in the PR body:
  1. the findings list precedes the table in DOM order;
  2. focus lands on `#link-heading` on open and returns to `#same-person` on close/Escape;
  3. with `can_link: false` there is no Link button and `#link-problems` is shown;
  4. an Outranked report keeps the panel open with its sentence; TookEffect closes it and calls `med_list`;
  5. charts in `renderedCharts` are not offered as candidates.

- [ ] **Step 5: Commit.**
  ```bash
  git add cairn-gui/cairn-gui-tauri/src-ui
  git commit -m "feat(R2b-1): the Same-person-as panel — findings first, both records side by side, Link"
  ```

---

### Task 7: runbook section 9, docs, full gates

**Files:**
- Modify: `cairn-gui/cairn-gui-tauri/results/RUNBOOK.md`, `cairn-gui/cairn-gui-tauri/results/TEMPLATE.md`
- Modify: `docs/superpowers/specs/2026-09-27-duplicate-repair-path-679-680-681-design.md` (an "as built" note under R2b)
- Modify: `docs/HANDOVER.md`, `docs/ROADMAP.md`

- [ ] **Step 1: Runbook.** Read `RUNBOOK.md`'s section 8 (the front door) and add a section 9 in
  the same format: *Compare and link* — live and `--mock`; start with a chart open; stopwatch from
  pressing **Same person as…** to the outcome line reading "Linked —"; steps: type part of the other
  name, press **Compare**, read the panel, press **Link**; budget **review-and-link ≤ 20 s**; record
  the number of findings shown and whether the key was already unlocked. VoiceOver: the findings are
  announced before the table; the table's column groups read "This record"/"Other record". Add the
  matching blank rows to `TEMPLATE.md`. State that a figure outside budget is a finding to file,
  never a budget to adjust.

- [ ] **Step 2: As-built note** in the design page under the R2b section: list any deviation this
  plan made — at least: fixture facts carry the name only (a `Candidate` carries an age, not a DOB);
  the right-set refusal reuses `check_displayed_set` and maps both its failures to `OTHER_CHANGED`;
  link gesture timing is not recorded server-side.

- [ ] **Step 3: Full gates, in CI's order, AFTER the last edit.** Root: `cargo fmt --all -- --check`,
  `cargo clippy --workspace --all-targets -- -D warnings`, `RUSTDOCFLAGS="-D warnings" cargo doc
  --workspace --no-deps`, then `scripts/run-db-gated-tests.sh` (long — run it in the background and
  read the LOG's last line; it also runs `cairn-gui-live`). GUI tree (`cd cairn-gui`):
  `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --locked -- -D warnings`,
  `CAIRN_ALLOW_DB_SKIP=1 cargo test --workspace`, `RUSTDOCFLAGS="-D warnings" cargo doc --workspace
  --no-deps`. Also `CAIRN_ALLOW_DB_SKIP=1 cargo test -p cairn-node --test paper_parity_plan_section`.
  Every one must be green; paste the summary lines into the PR body.

- [ ] **Step 4: HANDOVER and ROADMAP.** ⇒ NEXT: R2b-1 built on PR #707; next R2b-2 (unlink + #699 (a),
  the design's R2b-2 bullets). Add an R2b-1 ROADMAP entry (what it built, tests, §1.2) and prune:
  keep both under ~500 lines, never dropping an open issue number.

- [ ] **Step 5: Commit, push, update PR #707** (title: drop "WIP" only if every gate is green; body:
  what was built, the gate results, the headless-walk results, the human acts still owed — the
  stopwatch figure and the live Tauri-IPC pass on a real pair).
  ```bash
  git add -A && git commit -m "docs(R2b-1): runbook section 9, as-built note, HANDOVER and ROADMAP"
  git push
  ```

---

## Paper-parity benchmark (§1.2)

- **Paper counterpart:** the records clerk fetches the other folder, lays the two front sheets side
  by side, and clips the folders together.
- **Steps:** paper 3 (fetch, lay side by side, clip) → architecture-forced 3 (find → Compare → Link;
  the Link click IS the signature under the unlocked key, ADR-0053, so authorship adds no act) → UI
  bundling target 3. `M ≤ N`. An unlock, when the key is locked, is the existing session act, not
  one this slice adds. From R5's banner the "find" is already done: 3 → 2 → 2.
- **Time + cognitive load:** review-and-link ≤ 20 s, of which the side-by-side read is the load.
  The veto findings come first so the facts that most need reading are read first; the panel shows
  both RECORDS (every member), so the clerk is never asked to remember a third chart that is not on
  screen. Measured by runbook section 9 (Task 7) — a human act; owed by this slice's runnable
  surface.
