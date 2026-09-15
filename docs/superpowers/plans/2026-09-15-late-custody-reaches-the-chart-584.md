# A late key reaches the chart (#584) — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** When a sealed event's key arrives after the event itself, both write doors run that event's heal-safe projections, so the record reaches the chart with no `cairn-node reproject` step — and `requeue`'s `reproject_owed` signal retires because a custody-reading projection is required to be heal-safe.

**Architecture:** Two small PL/pgSQL functions beside the projection dispatcher in `db/005` — `cairn_projection_dispatch_heal_safe(event_log)` (the loop `db/043`'s gate 4 carries today) and `cairn_project_late_custody(uuid)` (load the stored row, skip it unless replay-eligible, dispatch). `apply_remote_event` (db/020) and `submit_event` (db/005) call the second one after their substitution guard whenever their step 9 newly wrote `event_clear` and their `event_log` INSERT was a no-op. A catalog guard pins both invariants; cairn-sync's `requeue` loses the signal the door made unnecessary.

**Tech Stack:** PostgreSQL 18 PL/pgSQL (`db/*.sql`, loaded by `include_str!`), Rust integration tests (`tokio-postgres`, DB-gated on `$CAIRN_TEST_PG`), cairn-sync binary crate.

**Spec:** `docs/superpowers/specs/2026-09-15-late-custody-reaches-the-chart-584-design.md` — read §2 (the audit) and §2.3 (the four placement hazards) before touching a door.

## Global Constraints

- **AGPL-3.0; no new dependency.** Nothing in this plan adds a crate.
- **TDD:** every behaviour change is driven by a test written first and seen to fail; a pin over behaviour that already holds is proven instead by the named mutation in Task 7.
- **No new `db/*.sql` file and no `SCHEMA_GENERATION` bump.** Edit `db/005`, `db/020`, `db/043` only.
- **Every new SQL function:** `SET search_path = public, pg_temp` (pg_temp LAST, #426) and `REVOKE EXECUTE … FROM PUBLIC` (#382).
- **db/020's four placement rules (spec §2.3):** the late-custody call comes AFTER the substitution guard, BEFORE `set_config('cairn.remote_apply', '', true)`, only when replay-eligible (inside the helper), and after step 8 (by position).
- **House rule 6:** no hard-coded key/seed/nonce/salt literal in tests; never name a non-cryptographic value `salt`/`nonce`/`iv`.
- **Comments are written for a junior developer:** *why* and *how it fits*, not only *what*.
- **Commit convention:** `feat(#584):` / `test(#584):` / `docs(#584):` — the parenthesis keeps GitHub's closing-keyword parser away (the closing-keyword guard, #546). **Never write `close`/`fix`/`resolve` directly before `#584`** in a commit or PR body.
- **Never `git checkout -- <file>` to undo an edit** (memory: it discards all uncommitted work in the file). Undo a mutation with `git show HEAD:<path> > <path>` AFTER everything real is committed, then `git status --short` must be clean.
- **Test environment.** Shell state does not persist between tool calls, so every cargo command below begins with `. /tmp/cairn-584.env &&` (created in Task 0). It sets `CARGO_TARGET_DIR=/tmp/cairn-584-target` (a live IDE's rust-analyzer holds the shared `target/` lock) and the three `CAIRN_TEST_PG*` strings. **FOREGROUND ONLY** for any subagent: a subagent that ends its turn waiting on a background job never wakes.
- **Never pipe a cargo run into `tail`/`head`** — it masks cargo's exit code. If one freshly linked test binary stalls (macOS Gatekeeper's one-time assessment), exec the printed `target/debug/deps/<name>-<hash>` directly.
- **A SQL edit needs a rebuild** (`include_str!`): cargo does this automatically; do not run a stale binary by path after editing SQL.

## File map

| File | Change | Responsibility |
|---|---|---|
| `db/005_submit.sql` | modify | the two helpers (after `cairn_projection_dispatch_trg`); `submit_event` detects + calls; seal-robustness comment nit |
| `db/020_apply_remote_event.sql` | modify | detect a late landing; substitution guard moves above the marker clear; call |
| `db/043_deferred_readjudication.sql` | modify | gate 4's loop becomes one `PERFORM` of the shared dispatch |
| `crates/cairn-node/tests/common/late_custody_kit.rs` | create | sealed-assert fixture, the two applies, chart counters, the counting-applier probe |
| `crates/cairn-node/tests/heal_safe_dispatch.rs` | create | the helpers, called directly |
| `crates/cairn-node/tests/late_custody_reaches_the_chart.rs` | create | the doors, end to end |
| `crates/cairn-node/tests/late_custody_guards.rs` | create | catalog guards: custody readers are heal-safe; custody writers call the helper |
| `crates/cairn-node/tests/restore_one_event_id_one_body.rs` | modify | trap 9 pin inverts |
| `crates/cairn-node/tests/restore_cli_surface.rs`, `tests/common/restore_kit.rs` | modify | comments stop citing #584 as open |
| `crates/cairn-node/src/restore/clinical.rs` | modify | `CustodyDidNotLand` remedy loses the reproject clause |
| `crates/cairn-sync/src/requeue.rs` | modify | retire `chart_rebuild_owed`, `chart_rebuild_message`, `reproject_owed` |
| `crates/cairn-sync/src/main.rs` | modify | requeue loop drops the pre-apply custody read; interrupted message; help text; `decide_custody` names one step |
| `crates/cairn-sync/tests/requeue_retains_unlanded_custody.rs` | modify | pin inverts (chart 1, exit 0, no heal phase) |
| `crates/cairn-sync/tests/clinical_pull.rs` | modify | pin inverts (one step) |
| `crates/cairn-sync/tests/common/dead_node.rs` | modify | `EXIT_INCOMPLETE` copy's doc |
| `docs/spec/decisions/0070-a-late-key-reaches-the-chart.md` | create | ADR-0070 |
| `docs/spec/decisions/README.md`, `docs/spec/index.md`, `docs/spec/language-substrate.md` | modify | index row, v0.72, the "AFTER INSERT only" bullet |
| `docs/HANDOVER.md`, `docs/ROADMAP.md` | modify | currency (Task 0) and close-out (Task 8) |

---

### Task 0: Tracking documents current, and the test environment

**Files:**
- Modify: `docs/HANDOVER.md` (the `#600` WARNING block, ⇒ NEXT decision lines)
- Modify: `docs/ROADMAP.md` (the #593/#600 entry, if it says #595 is unmerged)
- Create: `/tmp/cairn-584.env` (not committed)

- [ ] **Step 1: Write the env file**

```bash
cat > /tmp/cairn-584.env <<'EOF'
export CARGO_TARGET_DIR=/tmp/cairn-584-target
export CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=$USER dbname=cairn_test"
export CAIRN_TEST_PG2="host=127.0.0.1 port=5532 user=$USER dbname=cairn_test2"
export CAIRN_TEST_PG3="host=127.0.0.1 port=5532 user=$USER dbname=cairn_test3"
EOF
scripts/pg-target.sh   # expect: 127.0.0.1 5532
```

- [ ] **Step 2: Correct HANDOVER's stale #600 block.** It says *"Until #595 merges, `main` and any branch cut from it stay red"*. PR #595 merged on 2026-09-15 and `main`'s CI is green. Replace the WARNING block with one line under ⇒ NEXT: `#600 (RUSTSEC-2026-0285, rustls → 0.23.45) is CLOSED, merged with PR #595; CI green on main 2026-09-15.`

- [ ] **Step 3: Record the 2026-09-15 decisions in ⇒ NEXT.** Replace the sentence naming *"two maintainer decisions"* with: `#594 DECIDED (2026-09-15): restore exits 3 whenever any medium record was not restored — chain break, unknown plane, AND torn tail — after the full summary; not yet built. #575 re-deferred. #584 IN PROGRESS on feat/584-late-custody-reaches-the-chart: option (a) narrowed, ADR-0070.`

- [ ] **Step 4: Verify no open issue number was dropped**

```bash
git diff docs/HANDOVER.md docs/ROADMAP.md | grep '^-' | grep -o '#[0-9]\{3\}' | sort -u > /tmp/removed.txt
git diff docs/HANDOVER.md docs/ROADMAP.md | grep '^+' | grep -o '#[0-9]\{3\}' | sort -u > /tmp/added.txt
comm -23 /tmp/removed.txt /tmp/added.txt   # every number printed must still appear elsewhere in the file
```
For each number printed, `grep -c '#NNN' docs/HANDOVER.md docs/ROADMAP.md` must be non-zero, or the number is re-added.

- [ ] **Step 5: Commit**

```bash
git add docs/HANDOVER.md docs/ROADMAP.md
git commit -m "docs(#584): HANDOVER and ROADMAP say #595 merged, and record the #594/#575 decisions

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

### Task 1: The two helpers, and db/043 gate 4 sharing the dispatch

**Files:**
- Create: `crates/cairn-node/tests/common/late_custody_kit.rs`
- Create: `crates/cairn-node/tests/heal_safe_dispatch.rs`
- Modify: `db/005_submit.sql` (insert after line 298, `CREATE TRIGGER cairn_projection_dispatch_trg …`)
- Modify: `db/043_deferred_readjudication.sql` (DECLARE `v_apply_fn`; lines 250-256)

**Interfaces:**
- Produces (SQL): `cairn_projection_dispatch_heal_safe(e event_log) RETURNS void`; `cairn_project_late_custody(p_event_id uuid) RETURNS void`.
- Produces (Rust kit, used by Tasks 2-3): `cs()`, `WALL`, `Keys { sk_device, kid_device, sk_human, kid_human }`, `fresh_node(&Client) -> Keys`, `SealedAssert { signed, dek, clear_payload, event_id, medication_id, patient, twin }`, `sealed_assert(&Keys, patient, medication_id, event_id, term, wall) -> SealedAssert`, `apply_without_key`, `apply_with_key`, `submit_with_key` (each `(&Client, &SealedAssert) -> Result<u64, tokio_postgres::Error>`), `clear_twin(&Client, Uuid) -> Option<String>`, `statement_rows(&Client, Uuid) -> i64`, `dose_seed_rows(&Client, Uuid) -> i64`, `conflict_flags(&Client, Uuid) -> i64`, `install_probe(&Client, &str)`, `remove_probe(&Client)`, `probe_runs(&Client, &str) -> i64`.

- [ ] **Step 1: Create the kit**

`crates/cairn-node/tests/common/late_custody_kit.rs`:

```rust
//! A sealed medication event this node holds WITHOUT its key, and the key arriving later — the
//! shared fixture for #584's tests (ADR-0070).
//!
//! # Why this file exists
//!
//! Custody can reach an event after the event itself: a peer serves the bytes before this node is
//! admitted (`pull --full` later re-offers them with a DEK), a restore applies a keyless copy before
//! its keyed one, a `requeue` lands a penned key. Every one of those ends in the same database
//! state — an `event_log` row with no `event_clear` row, then a second apply that writes
//! `event_clear` — so the tests drive that state directly through the two doors, with bytes built
//! by the production medication builder rather than by hand.
//!
//! Include it with `#[path = "common/late_custody_kit.rs"] mod late_custody_kit;`. The including
//! binary must ALSO declare `mod common;`, because the node is set up by `common::medication_setup`.
#![allow(dead_code)] // each including suite uses a different subset

use crate::common;
use cairn_event::{sign, Hlc, SigningKey};
use cairn_node::medication::{build_assert_body, AssertMedicationInput};
use tokio_postgres::Client;
use uuid::Uuid;

/// The test database, or `None` when `$CAIRN_TEST_PG` is unset (the repo-wide self-skip, policed
/// by `tests/db_gate_actually_ran.rs`).
pub fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

/// A realistic HLC wall (ms since epoch, ≈ 2026-06-21): below today, so no clock ceiling trips,
/// and a fixed base so a test can order its events by adding to it.
pub const WALL: i64 = 1_782_000_000_000;

/// The two actors every event here needs: the DEVICE (this node; its key derives the registered
/// unwrap key in `medication_setup`) and the HUMAN who authors and signs (ADR-0053).
pub struct Keys {
    pub sk_device: SigningKey,
    pub kid_device: String,
    pub sk_human: SigningKey,
    pub kid_human: String,
}

/// An empty clinical node with both actors enrolled and its unwrap key registered.
///
/// `medication_setup` truncates `event_log` with CASCADE, which also clears `event_deferred` through
/// its foreign key. The conflict-flag table is not on its list, so it is cleared here.
pub async fn fresh_node(c: &Client) -> Keys {
    let (sk_device, kid_device, sk_human, kid_human) = common::medication_setup(c).await;
    c.batch_execute("TRUNCATE medication_patient_conflict_flag")
        .await
        .unwrap();
    Keys {
        sk_device,
        kid_device,
        sk_human,
        kid_human,
    }
}

/// One sealed `clinical.medication.asserted`, as it travels: signed bytes, and the DEK a custody
/// holder would hand the door beside them.
pub struct SealedAssert {
    pub signed: Vec<u8>,
    /// The plaintext DEK. A test-only copy out of `Secret32`, because it is bound as a query
    /// parameter; production never widens a DEK's lifetime this way.
    pub dek: Vec<u8>,
    /// The payload BEFORE sealing — what `event_clear.body` holds once custody lands. Only
    /// `heal_safe_dispatch.rs` uses it, to write the clear view by hand.
    pub clear_payload: serde_json::Value,
    pub event_id: Uuid,
    pub medication_id: Uuid,
    pub patient: Uuid,
    /// The clear twin sealed inside the container: finding it in `event_clear` proves the body
    /// OPENED rather than that some row exists.
    pub twin: String,
}

/// Build, seal and sign a medication assert through the production builder. **Pure** apart from
/// the DEK `seal_event_payload` mints.
///
/// The caller chooses `event_id` and `medication_id` so a test can build a RIVAL (same event id,
/// different body) or a second event on the SAME thread.
pub fn sealed_assert(
    keys: &Keys,
    patient: Uuid,
    medication_id: Uuid,
    event_id: Uuid,
    term: &str,
    wall: i64,
) -> SealedAssert {
    let input = AssertMedicationInput {
        term,
        coding: None,
        formulation: None,
        dose_amount: None,
        dose_unit: None,
        sig: None,
        info_source: "patient-reported",
        started: None,
        started_precision: None,
    };
    let hlc = Hlc {
        wall,
        counter: 0,
        node_origin: "peer".into(),
    };
    let body = build_assert_body(
        event_id,
        medication_id,
        patient,
        &input,
        &keys.kid_device,
        hlc,
        None,
    );
    let mut body = cairn_event::contributor::with_human_author(body, &keys.kid_human);
    let twin = body
        .plaintext_twin
        .take()
        .expect("build_assert_body always sets a plaintext twin");
    let clear_payload = body.payload.clone();
    let (container, dek) =
        cairn_event::seal::seal_event_payload(&body.payload, &twin, &body.event_id)
            .expect("seal a well-formed medication payload");
    body.payload = container;
    body.plaintext_twin = Some(cairn_event::seal::seal_stub_twin(&body.event_type));
    let signed = sign(&body, &keys.sk_human).expect("sign the sealed body");
    SealedAssert {
        signed: signed.signed_bytes,
        dek: dek.as_bytes().to_vec(),
        clear_payload,
        event_id,
        medication_id,
        patient,
        twin,
    }
}

/// The bytes through the REMOTE door with no DEK: admitted sealed, no custody, nothing projected.
pub async fn apply_without_key(
    c: &Client,
    e: &SealedAssert,
) -> Result<u64, tokio_postgres::Error> {
    c.execute("SELECT apply_remote_event($1)", &[&e.signed]).await
}

/// The same bytes through the REMOTE door WITH the DEK.
pub async fn apply_with_key(c: &Client, e: &SealedAssert) -> Result<u64, tokio_postgres::Error> {
    c.execute(
        "SELECT apply_remote_event($1, NULL, NULL, $2)",
        &[&e.signed, &e.dek],
    )
    .await
}

/// The same bytes through the STRICT door WITH the DEK.
pub async fn submit_with_key(c: &Client, e: &SealedAssert) -> Result<u64, tokio_postgres::Error> {
    c.execute(
        "SELECT submit_event($1, NULL, NULL, $2)",
        &[&e.signed, &e.dek],
    )
    .await
}

/// The clear twin of `event_id`, or `None` when this node cannot read the body.
///
/// UUIDs are bound as text and cast in SQL: `cairn-node` does not enable tokio-postgres's
/// `with-uuid-1` feature.
pub async fn clear_twin(c: &Client, event_id: Uuid) -> Option<String> {
    c.query_opt(
        "SELECT twin FROM event_clear WHERE event_id = $1::text::uuid",
        &[&event_id.to_string()],
    )
    .await
    .unwrap()
    .map(|r| r.get(0))
}

async fn count(c: &Client, sql: &str, id: Uuid) -> i64 {
    c.query_one(sql, &[&id.to_string()]).await.unwrap().get(0)
}

/// Rows on the medication list for this thread — the number a clinician sees.
pub async fn statement_rows(c: &Client, medication_id: Uuid) -> i64 {
    count(
        c,
        "SELECT count(*) FROM medication_statement WHERE medication_id = $1::text::uuid",
        medication_id,
    )
    .await
}

/// The dose timeline's seed row: `clinical.medication.asserted` has TWO registered appliers, and a
/// fix that ran only one of them would pass a statement-only assertion.
pub async fn dose_seed_rows(c: &Client, medication_id: Uuid) -> i64 {
    count(
        c,
        "SELECT count(*) FROM medication_dose_event \
         WHERE medication_id = $1::text::uuid AND is_initial",
        medication_id,
    )
    .await
}

/// #192 cross-patient flags raised on this thread.
pub async fn conflict_flags(c: &Client, medication_id: Uuid) -> i64 {
    count(
        c,
        "SELECT count(*) FROM medication_patient_conflict_flag WHERE medication_id = $1::text::uuid",
        medication_id,
    )
    .await
}

/// Remove the counting appliers, if present. Called at test START (a predecessor that panicked
/// may have left them — the #583 reset-at-start rule) and BEFORE asserting, so a failed assertion
/// never leaves two extra rows in a registry `projection_registry.rs` pins at an exact count.
pub async fn remove_probe(c: &Client) {
    c.batch_execute(
        "DELETE FROM cairn_projection_apply \
           WHERE apply_fn IN ('cairn_test_late_custody_safe', 'cairn_test_late_custody_unsafe'); \
         DROP FUNCTION IF EXISTS cairn_test_late_custody_safe(event_log); \
         DROP FUNCTION IF EXISTS cairn_test_late_custody_unsafe(event_log); \
         DROP TABLE IF EXISTS cairn_test_late_custody_runs;",
    )
    .await
    .unwrap();
}

/// Register two appliers for `event_type` that do nothing but COUNT their runs: one heal-safe, one
/// not. Fault injection without residue — see [`remove_probe`].
///
/// Neither reads custody, so they run identically with or without a body; what differs between
/// them is only the registry's `heal_safe` flag, which is exactly the variable under test.
pub async fn install_probe(c: &Client, event_type: &str) {
    remove_probe(c).await;
    c.batch_execute(
        "CREATE TABLE cairn_test_late_custody_runs (applier text NOT NULL, event_id uuid NOT NULL); \
         CREATE FUNCTION cairn_test_late_custody_safe(e event_log) RETURNS void LANGUAGE sql AS \
           $$ INSERT INTO cairn_test_late_custody_runs VALUES ('safe', e.event_id) $$; \
         CREATE FUNCTION cairn_test_late_custody_unsafe(e event_log) RETURNS void LANGUAGE sql AS \
           $$ INSERT INTO cairn_test_late_custody_runs VALUES ('unsafe', e.event_id) $$;",
    )
    .await
    .unwrap();
    c.execute(
        "INSERT INTO cairn_projection_apply \
           (event_type, apply_fn, projection_tables, run_order, heal_safe) VALUES \
           ($1, 'cairn_test_late_custody_safe',   ARRAY['cairn_test_late_custody_runs'], 900, TRUE), \
           ($1, 'cairn_test_late_custody_unsafe', ARRAY['cairn_test_late_custody_runs'], 900, FALSE)",
        &[&event_type],
    )
    .await
    .unwrap();
}

/// How many times the probe applier named `applier` (`"safe"` or `"unsafe"`) has run.
pub async fn probe_runs(c: &Client, applier: &str) -> i64 {
    c.query_one(
        "SELECT count(*) FROM cairn_test_late_custody_runs WHERE applier = $1",
        &[&applier],
    )
    .await
    .unwrap()
    .get(0)
}
```

- [ ] **Step 2: Write the failing helper tests**

`crates/cairn-node/tests/heal_safe_dispatch.rs`:

```rust
//! #584 / ADR-0070 — the two SQL helpers a late key reaches the chart through, called directly.
//!
//! `cairn_projection_dispatch_heal_safe(event_log)` runs ONE stored event's heal-safe registered
//! appliers; `db/043`'s gate 4 and the late-custody path share it, so "which appliers may run
//! again over a live row" is spelled once. `cairn_project_late_custody(uuid)` is what the two
//! doors call: it loads the stored row and dispatches only when the row is replay-eligible.
//!
//! The door behaviour is `late_custody_reaches_the_chart.rs`; this file pins the helpers on their
//! own, with the clear view written BY HAND, so a failure here is about the helper and never about
//! a door.
//!
//! Real Postgres, gated on `$CAIRN_TEST_PG`, serialized via `db::test_serial_guard`.

mod common;
#[path = "common/late_custody_kit.rs"]
mod late_custody_kit;

use cairn_node::db;
use late_custody_kit::*;
use tokio_postgres::Client;
use uuid::Uuid;

/// Admit `e` with no key, then write its clear view directly — the state a late custody landing
/// leaves, minus the door. `event_dek` is omitted: no projection reads it.
async fn admitted_then_made_readable(c: &Client, e: &SealedAssert) {
    apply_without_key(c, e).await.expect("admitted without custody");
    // Bound as TEXT and cast: cairn-node's tokio-postgres has no serde_json feature, so a
    // `serde_json::Value` cannot be a jsonb parameter directly.
    c.execute(
        "INSERT INTO event_clear (event_id, body, twin) VALUES ($1::text::uuid, $2::text::jsonb, $3)",
        &[&e.event_id.to_string(), &e.clear_payload.to_string(), &e.twin],
    )
    .await
    .expect("write the clear view by hand");
}

/// The dispatch runs the heal-safe applier again and never the other one.
///
/// The FIRST admission runs both through the `AFTER INSERT` trigger, which ignores `heal_safe`
/// (a fresh insert is not a replay). Only the direct call distinguishes them.
#[tokio::test]
async fn the_dispatch_runs_only_heal_safe_appliers() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let keys = fresh_node(&c).await;
    install_probe(&c, "clinical.medication.asserted").await;

    let e = sealed_assert(&keys, Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7(), "amoxicillin", WALL);
    let admitted = apply_without_key(&c, &e).await;
    let dispatched = c
        .execute(
            "SELECT cairn_projection_dispatch_heal_safe(el) FROM event_log el \
             WHERE el.event_id = $1::text::uuid",
            &[&e.event_id.to_string()],
        )
        .await;
    let (safe, unsafe_) = (probe_runs(&c, "safe").await, probe_runs(&c, "unsafe").await);
    remove_probe(&c).await; // BEFORE asserting: no residue in a pinned-count registry

    admitted.expect("admitted without custody");
    dispatched.expect("the dispatch runs");
    assert_eq!(safe, 2, "admission ran it once and the dispatch once more");
    assert_eq!(
        unsafe_, 1,
        "a heal_safe = false applier is never re-run over a live row — that is what the flag means"
    );
}

/// With the body readable and the row eligible, the helper builds the chart: BOTH appliers of the
/// type (statement and dose seed).
#[tokio::test]
async fn late_custody_projection_builds_the_chart_for_an_eligible_row() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let keys = fresh_node(&c).await;
    remove_probe(&c).await;

    let e = sealed_assert(&keys, Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7(), "amoxicillin", WALL);
    admitted_then_made_readable(&c, &e).await;
    assert_eq!(
        statement_rows(&c, e.medication_id).await,
        0,
        "premise: writing event_clear alone projects nothing — the trigger is on event_log"
    );

    c.execute(
        "SELECT cairn_project_late_custody($1::text::uuid)",
        &[&e.event_id.to_string()],
    )
    .await
    .expect("the helper runs");
    assert_eq!(statement_rows(&c, e.medication_id).await, 1);
    assert_eq!(dose_seed_rows(&c, e.medication_id).await, 1);
}

/// A row carrying an `event_deferred` marker is never projected — not even with its body readable.
/// The marker means its classification-gated checks have not passed (ADR-0056); only
/// `cairn_readjudicate_deferred` may grant it power.
#[tokio::test]
async fn late_custody_projection_skips_a_deferred_row() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let keys = fresh_node(&c).await;
    remove_probe(&c).await;

    let e = sealed_assert(&keys, Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7(), "amoxicillin", WALL);
    admitted_then_made_readable(&c, &e).await;
    c.execute(
        "INSERT INTO event_deferred (event_id, event_type) \
         VALUES ($1::text::uuid, 'clinical.medication.asserted')",
        &[&e.event_id.to_string()],
    )
    .await
    .expect("mark the row deferred, as a failed re-adjudication leaves it");

    c.execute(
        "SELECT cairn_project_late_custody($1::text::uuid)",
        &[&e.event_id.to_string()],
    )
    .await
    .expect("the helper runs");
    assert_eq!(
        statement_rows(&c, e.medication_id).await,
        0,
        "a deferred row must not project through the late-custody path"
    );
}

/// An id with no `event_log` row is a silent no-op, not an error: both doors call the helper only
/// after their own INSERT, so this arm is defensive, and a defensive arm must not become a refusal.
#[tokio::test]
async fn late_custody_projection_of_an_unknown_event_does_nothing() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    c.execute(
        "SELECT cairn_project_late_custody($1::text::uuid)",
        &[&Uuid::now_v7().to_string()],
    )
    .await
    .expect("an unknown id is not an error");
}
```

- [ ] **Step 3: Run to verify it fails**

Run: `. /tmp/cairn-584.env && cargo test -p cairn-node --test heal_safe_dispatch -- --test-threads=1`
Expected: FAIL — `function cairn_projection_dispatch_heal_safe(event_log) does not exist` / `function cairn_project_late_custody(uuid) does not exist` (three tests; `the_dispatch_runs_only_heal_safe_appliers` panics at `dispatched.expect`).

- [ ] **Step 4: Add the helpers to db/005** — directly after the `CREATE TRIGGER cairn_projection_dispatch_trg … FOR EACH ROW EXECUTE FUNCTION cairn_projection_dispatch();` statement:

```sql

-- #584 / ADR-0070 — run ONE stored event's HEAL-SAFE registered apply fns.
--
-- The trigger above runs every registered applier on a FRESH insert. This runs only the
-- heal_safe ones, over a row that is ALREADY in the log, which is the situation two callers are
-- in:
--   * db/043's gate 4, proving a promoted deferred event can project before its marker goes;
--   * cairn_project_late_custody below, when an event's key arrives after the event did.
-- heal_safe = false marks a counter-shaped applier (note.added's note_count): running it over a
-- live row would count again, so neither caller may run it. Same rule as cairn_reproject's heal
-- mode (db/039), spelled once here so the two callers cannot drift.
--
-- NO eligibility filter inside, deliberately: gate 4 must run on a row whose event_deferred marker
-- is still present (that is its proof). The late-custody caller filters before calling.
--
-- search_path pinned for the %I EXECUTE, exactly like cairn_projection_dispatch (#426).
CREATE OR REPLACE FUNCTION cairn_projection_dispatch_heal_safe(e event_log)
RETURNS void LANGUAGE plpgsql
SET search_path = public, pg_temp
AS $$
DECLARE v_fn text;
BEGIN
    FOR v_fn IN
        SELECT apply_fn FROM cairn_projection_apply
        WHERE event_type = e.event_type AND heal_safe
        ORDER BY run_order, apply_fn
    LOOP
        EXECUTE format('SELECT %I($1)', v_fn) USING e;
    END LOOP;
END;
$$;
-- It writes projections, so it takes the appliers' posture (#382): callers are the SECURITY
-- DEFINER doors and the owner-only re-adjudication, which already run as the owner.
REVOKE EXECUTE ON FUNCTION cairn_projection_dispatch_heal_safe(event_log) FROM PUBLIC;

-- #584 / ADR-0070 — custody arrived for an event already in the log: bring it to the chart.
--
-- WHY THIS EXISTS. Both doors write custody (event_dek, event_clear) BEFORE their event_log INSERT
-- so the trigger above can read the clear view. When the key comes LATER — a peer served the bytes
-- before admitting us, a restore met the keyless copy first, a requeue landed a penned key — the
-- second apply writes event_clear, its INSERT hits ON CONFLICT DO NOTHING, and the trigger never
-- fires again: the body opens and the medication list stays empty. The door is the only place that
-- KNOWS custody just landed, so the doors call this.
--
-- WHAT IT DOES. Loads the row as FIRST admitted (with the attestation columns that admission
-- stored — never a row rebuilt from the caller's arguments) and runs its heal-safe appliers.
-- Skips a row that is not cairn_replay_eligible: an event_deferred marker means its
-- classification-gated checks have not passed, and only cairn_readjudicate_deferred (db/043) may
-- grant it power.
--
-- WHAT THE RESULT MEANS. The chart equals "the event arrived when its key landed", not "at its
-- first admission" — the arrival-order independence every projection already has (ADR-0070 §4).
--
-- A custody-reading applier must be heal_safe, or this would skip it and leave the chart owed a
-- rebuild; crates/cairn-node/tests/late_custody_guards.rs enforces that over the catalog.
CREATE OR REPLACE FUNCTION cairn_project_late_custody(p_event_id uuid)
RETURNS void LANGUAGE plpgsql
SET search_path = public, pg_temp
AS $$
DECLARE v_row event_log;
BEGIN
    SELECT * INTO v_row FROM event_log WHERE event_id = p_event_id;
    IF NOT FOUND THEN
        RETURN; -- defensive: both doors call this only after their own INSERT
    END IF;
    IF NOT cairn_replay_eligible(v_row) THEN
        RETURN;
    END IF;
    PERFORM cairn_projection_dispatch_heal_safe(v_row);
END;
$$;
REVOKE EXECUTE ON FUNCTION cairn_project_late_custody(uuid) FROM PUBLIC;
```

- [ ] **Step 5: Point db/043 gate 4 at the shared dispatch.** Delete `    v_apply_fn text;` from `cairn_readjudicate_deferred`'s DECLARE, and replace

```sql
            FOR v_apply_fn IN
                SELECT apply_fn FROM cairn_projection_apply
                 WHERE event_type = r.event_type AND heal_safe
                 ORDER BY run_order, apply_fn
            LOOP
                EXECUTE format('SELECT %I($1)', v_apply_fn) USING r.el_row;
            END LOOP;
```

with

```sql
            -- The loop that used to live here is cairn_projection_dispatch_heal_safe (db/005),
            -- shared with the late-custody path (#584) so the two cannot drift on which appliers
            -- may run again over a live row.
            PERFORM cairn_projection_dispatch_heal_safe(r.el_row);
```

In the comment just above (`-- heal_safe mirrors heal mode (db/039): …`) keep the text; it is still the reason.

- [ ] **Step 6: Run the helper tests and gate 4's own suite**

Run: `. /tmp/cairn-584.env && cargo test -p cairn-node --test heal_safe_dispatch -- --test-threads=1`
Expected: 4 passed.
Run: `. /tmp/cairn-584.env && cargo test -p cairn-node --test deferred_admission -- --test-threads=1`
Expected: all pass (in particular `a_promotion_that_cannot_project_never_promotes`, `connect_promotes_and_reprojects_a_deferred_event`, `classification_promotes_a_passing_deferred_event`).
Run: `. /tmp/cairn-584.env && cargo test -p cairn-node --test projection_registry --test search_path_pg_temp --test floor_execute_grants -- --test-threads=1`
Expected: all pass (registry count still 27 — the probe cleaned up).

- [ ] **Step 7: fmt, clippy, commit**

```bash
. /tmp/cairn-584.env && cargo fmt --all -- --check && cargo clippy -p cairn-node --tests -- -D warnings
git add db/005_submit.sql db/043_deferred_readjudication.sql crates/cairn-node/tests/common/late_custody_kit.rs crates/cairn-node/tests/heal_safe_dispatch.rs
git commit -m "feat(#584): one heal-safe dispatch, shared by re-adjudication and a late key

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

### Task 2: The lenient door brings a late key to the chart — and the three pins invert

**Files:**
- Create: `crates/cairn-node/tests/late_custody_reaches_the_chart.rs`
- Modify: `db/020_apply_remote_event.sql` (DECLARE; step 9 lines 405-418; lines 451-466)
- Modify: `crates/cairn-node/tests/restore_one_event_id_one_body.rs` (header item 3; the keyed-first test's doc; the pin test)
- Modify: `crates/cairn-sync/tests/clinical_pull.rs` (`an_admitted_peer_recovers_the_bodies_it_pulled_without_custody`)
- Modify: `crates/cairn-sync/tests/requeue_retains_unlanded_custody.rs` (arm 1's `medication_rows == 0` assertion only)

**Interfaces:**
- Consumes: the kit (Task 1); `cairn_project_late_custody(uuid)`.
- Produces: `apply_remote_event`'s new behaviour, which Tasks 5 and 7 rely on.

- [ ] **Step 1: Write the failing door tests**

`crates/cairn-node/tests/late_custody_reaches_the_chart.rs`:

```rust
//! #584 / ADR-0070 — a key that arrives after its event brings the record to the chart.
//!
//! # The defect
//!
//! Projections are dispatched by ONE `AFTER INSERT` trigger on `event_log`. Both doors write
//! custody (`event_dek`, `event_clear`) BEFORE their INSERT so the trigger can read the clear view.
//! When the key comes later, the second apply writes `event_clear`, its INSERT is a no-op, and the
//! trigger never fires again: the body opens and the medication list stays empty. `pull --full`,
//! `requeue` and `restore` all reached that state, and `restore` said nothing about it.
//!
//! # What these tests pin
//!
//! 1. The headline: a keyed re-apply projects BOTH appliers of the type.
//! 2. It happens once: a further keyed apply runs nothing, and a `heal_safe = false` applier never
//!    runs again (the counting probe).
//! 3. The lenient posture holds: a contradiction the late key reveals is FLAGGED, not refused —
//!    otherwise the key could never land.
//! 4. A deferred event gains its key but not its chart, until re-adjudication promotes it.
//! 5. A rival body under an existing id, carrying its own key, is refused as a substitution and
//!    projects nothing.
//! 6. The strict door has the same entrance and the same fix.
//!
//! Tests 3, 4 and 5 pass against the pre-#584 door too — they pin placement rules the fix must not
//! break, and each is proven by a named mutation in the plan (Task 7).
//!
//! Real Postgres, gated on `$CAIRN_TEST_PG`, serialized via `db::test_serial_guard`.

mod common;
#[path = "common/late_custody_kit.rs"]
mod late_custody_kit;

use cairn_node::db;
use common::db_msg; // the RAISE text — an error's Display is only "db error"
use late_custody_kit::*;
use uuid::Uuid;

/// THE HEADLINE. Admitted without its key, the record is invisible; its key arriving makes it
/// readable AND puts it on the chart, through both of the type's appliers.
#[tokio::test]
async fn a_key_arriving_after_its_event_brings_the_record_to_the_chart() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let keys = fresh_node(&c).await;
    remove_probe(&c).await;

    let e = sealed_assert(&keys, Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7(), "amoxicillin", WALL);
    apply_without_key(&c, &e).await.expect("admitted without custody");
    assert_eq!(clear_twin(&c, e.event_id).await, None, "premise: the body is unreadable");
    assert_eq!(statement_rows(&c, e.medication_id).await, 0, "premise: nothing projected");

    apply_with_key(&c, &e).await.expect("the key is admitted");
    assert_eq!(
        clear_twin(&c, e.event_id).await.as_deref(),
        Some(e.twin.as_str()),
        "the body opens"
    );
    assert_eq!(
        statement_rows(&c, e.medication_id).await,
        1,
        "THE ASSERTION THAT MATTERS: the record is on the medication list, with no reproject"
    );
    assert_eq!(
        dose_seed_rows(&c, e.medication_id).await,
        1,
        "and the type's second applier ran too — a fix that ran only one would pass the line above"
    );
}

/// The landing is the ONE moment: a third apply, keyed again, runs nothing — and the counter-shaped
/// applier is never re-run, even at the landing.
#[tokio::test]
async fn a_late_key_runs_the_heal_safe_appliers_once_and_never_again() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let keys = fresh_node(&c).await;
    install_probe(&c, "clinical.medication.asserted").await;

    let e = sealed_assert(&keys, Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7(), "amoxicillin", WALL);
    let first = apply_without_key(&c, &e).await;
    let after_first = (probe_runs(&c, "safe").await, probe_runs(&c, "unsafe").await);
    let landing = apply_with_key(&c, &e).await;
    let after_landing = (probe_runs(&c, "safe").await, probe_runs(&c, "unsafe").await);
    let again = apply_with_key(&c, &e).await;
    let after_again = (probe_runs(&c, "safe").await, probe_runs(&c, "unsafe").await);
    remove_probe(&c).await; // BEFORE asserting

    first.expect("admitted without custody");
    landing.expect("the key is admitted");
    again.expect("an idempotent re-apply is a silent no-op");
    assert_eq!(after_first, (1, 1), "premise: a fresh insert runs every applier once");
    assert_eq!(
        after_landing,
        (2, 1),
        "the landing re-runs the heal-safe applier and NOT the counter-shaped one"
    );
    assert_eq!(
        after_again,
        (2, 1),
        "custody already held: nothing new landed, so nothing runs — the trigger for the heal is \
         'this call wrote event_clear', never 'the INSERT was a no-op'"
    );
}

/// The late key reveals a #192 contradiction (the same thread asserted for a second patient). In
/// the lenient posture that is a FLAG. Were the heal to run after the door clears
/// `cairn.remote_apply`, the guard would RAISE and the key could never land.
#[tokio::test]
async fn a_contradiction_revealed_by_a_late_key_is_flagged_not_refused() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let keys = fresh_node(&c).await;
    remove_probe(&c).await;

    let thread = Uuid::now_v7();
    let standing = sealed_assert(&keys, Uuid::now_v7(), thread, Uuid::now_v7(), "amoxicillin", WALL);
    apply_with_key(&c, &standing).await.expect("the thread's first chart");
    let rival_patient = sealed_assert(&keys, Uuid::now_v7(), thread, Uuid::now_v7(), "amoxicillin", WALL + 1);
    apply_without_key(&c, &rival_patient).await.expect("admitted without custody");
    assert_eq!(conflict_flags(&c, thread).await, 0, "premise: an unreadable body cannot contradict");

    let landed = apply_with_key(&c, &rival_patient).await;
    assert!(
        landed.is_ok(),
        "the key must land — a refusal here strands it forever: {:?}",
        landed.as_ref().err().map(db_msg)
    );
    assert_eq!(
        conflict_flags(&c, thread).await,
        1,
        "the contradiction is on the worklist, as it would be had the key come with the event"
    );
}

/// A deferred event's key lands, its body opens, and its chart waits for re-adjudication — which
/// then projects it through the same shared dispatch.
#[tokio::test]
async fn a_deferred_event_gains_its_key_but_not_its_chart_until_promoted() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let keys = fresh_node(&c).await;
    remove_probe(&c).await;

    let e = sealed_assert(&keys, Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7(), "amoxicillin", WALL);
    apply_without_key(&c, &e).await.expect("admitted without custody");
    c.execute(
        "INSERT INTO event_deferred (event_id, event_type) \
         VALUES ($1::text::uuid, 'clinical.medication.asserted')",
        &[&e.event_id.to_string()],
    )
    .await
    .expect("mark it deferred, as an event awaiting re-adjudication is");

    apply_with_key(&c, &e).await.expect("the key is admitted");
    assert_eq!(clear_twin(&c, e.event_id).await.as_deref(), Some(e.twin.as_str()));
    assert_eq!(
        statement_rows(&c, e.medication_id).await,
        0,
        "a deferred event has not passed its gates: the late key must not project it"
    );

    c.batch_execute("SELECT * FROM cairn_readjudicate_deferred()")
        .await
        .expect("re-adjudication runs");
    assert_eq!(
        statement_rows(&c, e.medication_id).await,
        1,
        "promotion projects it — gate 4, through the shared dispatch"
    );
}

/// A DIFFERENT body filed under an event id this node already holds without custody, carrying its
/// own key. The door refuses it as a substitution with the door's own reason, and no projection
/// of the rival survives.
#[tokio::test]
async fn a_rival_body_carrying_its_own_key_is_refused_and_projects_nothing() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let keys = fresh_node(&c).await;
    remove_probe(&c).await;

    let (patient, event_id) = (Uuid::now_v7(), Uuid::now_v7());
    let original = sealed_assert(&keys, patient, Uuid::now_v7(), event_id, "amoxicillin", WALL);
    apply_without_key(&c, &original).await.expect("admitted without custody");
    let rival = sealed_assert(&keys, patient, Uuid::now_v7(), event_id, "warfarin", WALL);

    let err = apply_with_key(&c, &rival)
        .await
        .expect_err("a second body under one event id is a substitution");
    assert!(
        db_msg(&err).contains("substitution refused"),
        "the refusal names the substitution, not whatever a projection raised: {}",
        db_msg(&err)
    );
    assert_eq!(clear_twin(&c, event_id).await, None, "the rival's body did not stay");
    assert_eq!(statement_rows(&c, rival.medication_id).await, 0, "the rival is on no chart");
    assert_eq!(statement_rows(&c, original.medication_id).await, 0);
}
```

- [ ] **Step 2: Run to verify the headline and the once-only test fail**

Run: `. /tmp/cairn-584.env && cargo test -p cairn-node --test late_custody_reaches_the_chart -- --test-threads=1`
Expected: `a_key_arriving_after_its_event_brings_the_record_to_the_chart` FAILS at `THE ASSERTION THAT MATTERS` (left 0, right 1); `a_late_key_runs_the_heal_safe_appliers_once_and_never_again` FAILS at `after_landing` ((1, 1) vs (2, 1)); `a_contradiction_revealed_by_a_late_key_is_flagged_not_refused` FAILS at `conflict_flags` (0 vs 1). The deferred and rival tests PASS (they pin rules; Task 7 proves them).

- [ ] **Step 3: Implement in db/020.** Add to `apply_remote_event`'s DECLARE, after `v_deferred      BOOLEAN := false;`:

```sql
    -- #584 / ADR-0070: rows THIS call wrote into event_clear (0 or 1). 1 on an event that was
    -- already in the log means custody arrived late, and the chart is owed a projection.
    v_clear_rows    INTEGER := 0;
```

In step 9, directly after `INSERT INTO event_clear (event_id, body, twin) VALUES (v_event_id, b_clear -> 'payload', v_twin) ON CONFLICT (event_id) DO NOTHING;`:

```sql
            GET DIAGNOSTICS v_clear_rows = ROW_COUNT;
```

Replace the block from `    GET DIAGNOSTICS v_rows = ROW_COUNT;` through the end of the substitution guard's `END IF;` with:

```sql
    GET DIAGNOSTICS v_rows = ROW_COUNT;

    -- Idempotent re-apply of the SAME event is a silent no-op (set-union). A
    -- DIFFERENT event reusing this event_id is a substitution — two nodes holding
    -- different bytes under one event_id would diverge forever with no alarm, so it
    -- must RAISE (review H3; identical to the submit_event guard).
    --
    -- This guard sits ABOVE the marker clear below since #584, so the late-custody call can
    -- follow it while cairn.remote_apply is still 'on'. Moving it changed nothing it checks: the
    -- marker is transaction-local, and a RAISE aborts the transaction either way.
    IF v_rows = 0 THEN
        IF (SELECT content_address FROM event_log WHERE event_id = v_event_id) <> v_ca THEN
            RAISE EXCEPTION 'apply_remote_event: event_id % already exists with different content (substitution refused)', v_event_id;
        END IF;
    END IF;

    -- #584 / ADR-0070 — CUSTODY ARRIVED LATE: this call made the body readable (step 9 wrote
    -- event_clear) for an event that was already in the log (the INSERT above was a no-op), so the
    -- AFTER INSERT dispatcher did not run and will not. Run the event's heal-safe appliers now.
    -- Three placement rules, each load-bearing (design §2.3):
    --   * AFTER the substitution guard, so a rival body filed under this id never reaches an
    --     applier — the refusal a caller reads stays "substitution refused";
    --   * BEFORE the marker clear, so projection guards clamp-and-flag here exactly as they do
    --     for a first arrival (db/031's patient guard and db/033's two checks RAISE otherwise, and
    --     the key could never land);
    --   * the helper itself skips a deferred row (cairn_replay_eligible).
    IF v_rows = 0 AND v_clear_rows > 0 THEN
        PERFORM cairn_project_late_custody(v_event_id);
    END IF;

    PERFORM set_config('cairn.remote_apply', '', true);
```

(The old `-- Capture the insert outcome BEFORE the set_config below: PERFORM overwrites FOUND …` comment above `GET DIAGNOSTICS v_rows` stays.) Then update the step 9 header comment's first line to read: `-- 9. Custody + operational clear view — BEFORE the log INSERT so the AFTER INSERT projection triggers can already read the shadow (same txn). If the event is ALREADY in the log, the late-custody call after the INSERT does the projecting instead (#584).`

- [ ] **Step 4: Run the door tests**

Run: `. /tmp/cairn-584.env && cargo test -p cairn-node --test late_custody_reaches_the_chart -- --test-threads=1`
Expected: 5 passed.

- [ ] **Step 5: Invert the restore pin** in `crates/cairn-node/tests/restore_one_event_id_one_body.rs`:

1. Header item 3 becomes: `//! 3. **The same pair, KEYLESS copy first, still reaches the chart.** The keyless copy admits the event with no body; the keyed copy lands custody on it, and the door projects the late landing (#584, ADR-0070). Until #584 this order left the chart empty with nothing in the report to say so — trap 9's restore entrance.`
2. In `a_second_copy_without_its_key_at_the_same_position_changes_nothing`'s doc, replace the sentence from `The final assertion reads the CHART` to the end of the paragraph with: `Since #584 the chart no longer tells the two orders apart — both project — so the premise check is the ONLY guard of the copy order; the final assertion still pins that the keyed-first order projects.`
3. Its premise message: replace `this is trap 9's order (#584)` with `the keyless copy now arrives first — see a_keyless_copy_first_still_reaches_the_chart`.
4. Rename `a_keyless_copy_first_leaves_the_chart_unprojected_until_584` → `a_keyless_copy_first_still_reaches_the_chart`; replace its doc's first two paragraphs (from `**PIN, NOT A GUARANTEE` through `becomes the guarantee.**`) with:

```rust
/// **Trap 9's restore entrance, closed (#584, ADR-0070).**
///
/// When the keyless copy of a sealed event reaches the restore BEFORE its keyed copy, the door
/// admits the event with no body. The keyed copy then lands custody on the already-admitted event,
/// its `event_log` INSERT is a no-op — and the door, seeing it has just made the body readable for
/// an event already in the log, runs the event's heal-safe projections. Before #584 this order
/// left the medication list empty at exit 0 with a report identical to the keyed-first order; this
/// test pinned that as a known defect and was inverted when the door learned to project a late key.
```

5. Replace the final assertion with:

```rust
    assert_eq!(
        medication_rows(&c, chart.patient).await,
        1,
        "the late key reaches the chart: the door projects custody that lands after its event (#584)"
    );
```

- [ ] **Step 6: Invert the pull pin** in `crates/cairn-sync/tests/clinical_pull.rs`. Replace the doc comment of `an_admitted_peer_recovers_the_bodies_it_pulled_without_custody` with:

```rust
/// Issue #231 review — the withhold IS repairable, and since #584 it takes ONE step.
///
/// The operator line promises a remedy, so the remedy is a test. Measured in the #231 review,
/// `pull --full` alone took custody from `(0,0)` to `(1,1)` and left the medication projection at
/// ZERO: the re-apply filled `event_dek`/`event_clear`, but its `event_log` insert was a no-op and
/// the projection dispatcher is an `AFTER INSERT` trigger, so the line had to name a second step,
/// `cairn_reproject()`. ADR-0070 moved that step into the door: an apply that makes a body readable
/// for an event already in the log runs the event's heal-safe projections itself. The middle
/// assertion is now the whole recovery.
```

Replace everything from `    assert_eq!(\n        statement_count_for_med(&b, med).await,\n        0,` to the end of the function with:

```rust
    assert_eq!(
        statement_count_for_med(&b, med).await,
        1,
        "…and the chart has it, with no reproject: the door projects custody that lands after its \
         event (#584, ADR-0070). Before that, this was 0, and an operator who followed a one-step \
         line saw an empty chart and would reasonably have concluded the record was lost"
    );
}
```

- [ ] **Step 7: Invert the requeue chart assertion only** in `crates/cairn-sync/tests/requeue_retains_unlanded_custody.rs`. Replace the block that begins `    // THE REVIEW FINDING. The body opens and the chart is still empty` through its `assert_eq!(medication_rows(&c).await, 0, …);` with:

```rust
    // THE REVIEW FINDING, answered at the door. The body opens AND the chart has it: the event was
    // admitted without custody on phase one, and the door projects a key that lands afterwards
    // (#584, ADR-0070). This used to be 0, with a heal step below.
    assert_eq!(
        medication_rows(&c).await,
        1,
        "the recovered record is on the chart as soon as its key lands"
    );
```

Leave the `reproject_owed`, exit-code, heal-line and phase-three assertions unchanged in this task: `requeue` still infers a debt until Task 5 retires it, and the phase-three reproject is now a harmless no-op.

- [ ] **Step 8: Run the three pinned suites**

Run: `. /tmp/cairn-584.env && cargo test -p cairn-node --test restore_one_event_id_one_body -- --test-threads=1`
Expected: all pass.
Run: `. /tmp/cairn-584.env && cargo test -p cairn-sync --test clinical_pull an_admitted_peer_recovers -- --test-threads=1`
Expected: 1 passed (needs `CAIRN_TEST_PG2`).
Run: `. /tmp/cairn-584.env && cargo test -p cairn-sync --test requeue_retains_unlanded_custody -- --test-threads=1`
Expected: all pass.

- [ ] **Step 9: Run the door's neighbours**

Run: `. /tmp/cairn-584.env && cargo test -p cairn-node --test apply_remote_event --test medication_remote_apply --test restore_reads_the_clinical_plane --test restore_cli_surface --test deferred_admission --test safety_doors -- --test-threads=1`
Expected: all pass.

- [ ] **Step 10: fmt, clippy, commit**

```bash
. /tmp/cairn-584.env && cargo fmt --all -- --check && cargo clippy -p cairn-node -p cairn-sync --tests -- -D warnings
git add db/020_apply_remote_event.sql crates/cairn-node/tests/late_custody_reaches_the_chart.rs crates/cairn-node/tests/restore_one_event_id_one_body.rs crates/cairn-sync/tests/clinical_pull.rs crates/cairn-sync/tests/requeue_retains_unlanded_custody.rs
git commit -m "feat(#584): apply_remote_event projects a key that lands after its event

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

### Task 3: The strict door's entrance

**Files:**
- Modify: `crates/cairn-node/tests/late_custody_reaches_the_chart.rs` (append one test; header item 6 already names it)
- Modify: `db/005_submit.sql` (`submit_event` DECLARE; step 9; post-INSERT guard; the seal-robustness comment in step 7)

- [ ] **Step 1: Append the failing test**

```rust
/// The STRICT door has the same step 9 and the same no-op INSERT, so a local re-submit of an event
/// this node holds without its key — with the key — is the same late landing, and gets the same fix.
/// Judged in the strict posture (no `cairn.remote_apply` marker), as a first arrival there would be.
#[tokio::test]
async fn the_strict_door_brings_a_late_key_to_the_chart_too() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let keys = fresh_node(&c).await;
    remove_probe(&c).await;

    let e = sealed_assert(&keys, Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7(), "amoxicillin", WALL);
    apply_without_key(&c, &e).await.expect("admitted without custody");
    let submitted = submit_with_key(&c, &e).await;
    assert!(
        submitted.is_ok(),
        "the strict door admits the same bytes with their key: {:?}",
        submitted.as_ref().err().map(db_msg)
    );
    assert_eq!(clear_twin(&c, e.event_id).await.as_deref(), Some(e.twin.as_str()));
    assert_eq!(
        statement_rows(&c, e.medication_id).await,
        1,
        "the strict door projects a late key exactly as the lenient one does"
    );
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `. /tmp/cairn-584.env && cargo test -p cairn-node --test late_custody_reaches_the_chart the_strict_door -- --test-threads=1`
Expected: FAIL at `statement_rows` (0 vs 1). **If it instead fails at `submitted.is_ok()`,** stop and record the door's refusal text in the plan's review ledger: the strict door refuses this entrance for a reason unrelated to #584, and the test should assert THAT refusal (and the spec's §1 row 4 be corrected) rather than be forced green.

- [ ] **Step 3: Implement in db/005's `submit_event`.** DECLARE, after `v_twin_stub    TEXT;`:

```sql
    -- #584 / ADR-0070: rows THIS call wrote into event_clear, and rows its event_log INSERT wrote.
    v_clear_rows   INTEGER := 0;
    v_log_rows     INTEGER;
```

In step 9, directly after its `INSERT INTO event_clear … ON CONFLICT (event_id) DO NOTHING;`:

```sql
        GET DIAGNOSTICS v_clear_rows = ROW_COUNT;
```

Directly after the `event_log` INSERT's `ON CONFLICT (event_id) DO NOTHING;` add `    GET DIAGNOSTICS v_log_rows = ROW_COUNT;`, change the guard's `IF NOT FOUND THEN` to `IF v_log_rows = 0 THEN` (one spelling of "the INSERT was a no-op" per door, the same as db/020), and after that guard's closing `END IF;` add:

```sql

    -- #584 / ADR-0070 — CUSTODY ARRIVED LATE at the strict door: a re-submit of an event this node
    -- already holds without its key, now with the key. Same shape and same remedy as db/020's
    -- late-custody call; see the comment there. After the substitution guard, so a rival body
    -- never reaches an applier. No remote-apply marker here: a late landing at the strict door is
    -- judged in the strict posture, as a first arrival here would be.
    IF v_log_rows = 0 AND v_clear_rows > 0 THEN
        PERFORM cairn_project_late_custody(v_event_id);
    END IF;
```

And in step 7's seal-robustness comment, replace `(they RETURN NULL on a sealed row; db/002/010-014/018/023-025).` with `(they RETURN on a sealed row — db/002/010-014/018/023-025/045 — or, for db/048's sensitivity assertion, project a deliberately unreadable MAX-grade row).`

- [ ] **Step 4: Run the door file and the strict door's own suites**

Run: `. /tmp/cairn-584.env && cargo test -p cairn-node --test late_custody_reaches_the_chart --test search_path_pg_temp --test safety_overclaim --test shred_predicate_has_one_home -- --test-threads=1`
Expected: all pass.

- [ ] **Step 5: Run the SQL mirrors** (they replay `db/005` and exercise `submit_event` in SQL)

Run: `scripts/run-db-sql-tests.sh`
Expected: exit 0.

- [ ] **Step 6: fmt, clippy, commit**

```bash
. /tmp/cairn-584.env && cargo fmt --all -- --check && cargo clippy -p cairn-node --tests -- -D warnings
git add db/005_submit.sql crates/cairn-node/tests/late_custody_reaches_the_chart.rs
git commit -m "feat(#584): submit_event projects a key that lands after its event too

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

### Task 4: The catalog guards

**Files:**
- Create: `crates/cairn-node/tests/late_custody_guards.rs`

- [ ] **Step 1: Write the guards (pure predicates first, then the catalog)**

```rust
//! #584 / ADR-0070 — the two invariants a late key's projection rests on, checked over the
//! DATABASE CATALOGUE (what actually runs), not over `db/*.sql` text.
//!
//! 1. **A registered applier that reads custody is heal-safe.** The doors re-run only heal-safe
//!    appliers when a key lands late. An applier that reads the clear view but is registered
//!    `heal_safe = false` would be skipped at the landing and leave the chart owed a rebuild —
//!    the debt `requeue`'s `reproject_owed` used to report, now made unrepresentable instead.
//! 2. **Every function that writes `event_clear` calls `cairn_project_late_custody`.** A third
//!    custody writer that did not would reopen #584 through its own entrance. The writer set is
//!    pinned by name, so a new one is a decision rather than a drift.
//!
//! # Honest residual
//!
//! Both read an applier's OWN body (`pg_proc.prosrc`). An applier that reads custody only through a
//! helper it calls is invisible to rule 1. Every custody reader today calls `cairn_clear_payload`
//! directly in its own body, which is what the positive control asserts.
//!
//! `--` line comments are stripped before matching, so prose naming a function neither satisfies
//! nor trips a rule. The pure predicates are exercised without a database below, so this file
//! proves something even where `$CAIRN_TEST_PG` is unset.

use cairn_node::db;

fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

/// The body with every `--` line comment removed. **Pure.**
fn without_line_comments(body: &str) -> String {
    body.lines()
        .map(|line| match line.find("--") {
            Some(at) => &line[..at],
            None => line,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Uppercase with every whitespace run collapsed to one space, so `insert  into\n event_clear`
/// matches. **Pure.**
fn normalised(body: &str) -> String {
    without_line_comments(body)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_uppercase()
}

/// Does this function body read custody — the clear view a sealed body opens into? **Pure.**
fn reads_custody(body: &str) -> bool {
    let n = normalised(body);
    n.contains("CAIRN_CLEAR_PAYLOAD") || n.contains("EVENT_CLEAR")
}

/// Does this function body write the clear view? **Pure.**
fn writes_custody(body: &str) -> bool {
    normalised(body).contains("INSERT INTO EVENT_CLEAR")
}

/// Does this function body call the late-custody projection? **Pure.**
fn projects_late_custody(body: &str) -> bool {
    normalised(body).contains("CAIRN_PROJECT_LATE_CUSTODY(")
}

#[test]
fn the_predicates_read_code_and_ignore_prose() {
    assert!(reads_custody("p jsonb := cairn_clear_payload(e);"));
    assert!(!reads_custody("-- cairn_clear_payload is not called here\nRETURN;"));
    assert!(writes_custody("insert  into\n   event_clear (event_id) VALUES (x)"));
    assert!(!writes_custody("-- INSERT INTO event_clear happens in the door\nRETURN;"));
    assert!(projects_late_custody("PERFORM cairn_project_late_custody(v_event_id);"));
    assert!(!projects_late_custody(
        "-- see cairn_project_late_custody(v_event_id)\nRETURN;"
    ));
}

/// Rule 1, over every registry row.
#[tokio::test]
async fn every_custody_reading_applier_is_heal_safe() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let rows = c
        .query(
            "SELECT r.event_type, r.apply_fn, r.heal_safe, p.prosrc \
               FROM cairn_projection_apply r \
               JOIN pg_proc p ON p.oid = to_regprocedure(r.apply_fn || '(event_log)')",
            &[],
        )
        .await
        .unwrap();

    let readers: Vec<(String, String, bool)> = rows
        .iter()
        .filter(|r| reads_custody(r.get::<_, &str>(3)))
        .map(|r| (r.get(0), r.get(1), r.get(2)))
        .collect();

    // POSITIVE CONTROL: a rule that sees no readers passes over anything (#586's lesson).
    assert!(
        readers.iter().any(|(_, f, _)| f == "medication_statement_apply"),
        "the guard must see the custody readers it exists for; saw {readers:?}"
    );
    assert!(
        readers.len() >= 9,
        "expected at least the nine medication appliers to read custody; saw {readers:?}"
    );

    let unsafe_readers: Vec<_> = readers.iter().filter(|(_, _, safe)| !safe).collect();
    assert!(
        unsafe_readers.is_empty(),
        "these appliers read custody but are registered heal_safe = false, so a key that lands \
         late would skip them and leave the chart owed a rebuild (ADR-0070 decision 3). Make the \
         applier idempotent and heal-safe: {unsafe_readers:?}"
    );
}

/// Rule 2, over every PL/pgSQL function in the schema.
#[tokio::test]
async fn every_custody_writer_projects_a_late_key() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let rows = c
        .query(
            "SELECT p.proname, p.prosrc FROM pg_proc p \
               JOIN pg_namespace n ON n.oid = p.pronamespace \
               JOIN pg_language l ON l.oid = p.prolang \
              WHERE n.nspname = 'public' AND l.lanname = 'plpgsql'",
            &[],
        )
        .await
        .unwrap();

    let mut writers: Vec<(String, bool)> = rows
        .iter()
        .filter(|r| writes_custody(r.get::<_, &str>(1)))
        .map(|r| (r.get(0), projects_late_custody(r.get::<_, &str>(1))))
        .collect();
    writers.sort();

    assert_eq!(
        writers.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(),
        vec!["apply_remote_event", "submit_event"],
        "the custody writers are the two doors. A third is a DECISION: give it the late-custody \
         call (ADR-0070) and add it here"
    );
    for (name, calls) in &writers {
        assert!(
            calls,
            "{name} writes event_clear but never calls cairn_project_late_custody — a key landing \
             through it would leave the chart empty (#584)"
        );
    }
}
```

- [ ] **Step 2: Run the guards**

Run: `. /tmp/cairn-584.env && cargo test -p cairn-node --test late_custody_guards -- --test-threads=1`
Expected: 3 passed. (These are guards over shipped state, so they pass first time; Task 7 proves each with a mutation.)

- [ ] **Step 3: Run the DB-gate policing suite** — a new `CAIRN_TEST_PG` reader must be recognised.

Run: `. /tmp/cairn-584.env && cargo test -p cairn-node --test db_gate_actually_ran -- --test-threads=1`
Expected: pass.

- [ ] **Step 4: fmt, clippy, commit**

```bash
. /tmp/cairn-584.env && cargo fmt --all -- --check && cargo clippy -p cairn-node --tests -- -D warnings
git add crates/cairn-node/tests/late_custody_guards.rs
git commit -m "test(#584): custody readers are heal-safe and custody writers project a late key

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

### Task 5: `requeue` retires `reproject_owed`; three remedies lose the step

**Files:**
- Modify: `crates/cairn-sync/src/requeue.rs`
- Modify: `crates/cairn-sync/src/main.rs` (requeue loop ~4370-4520; `requeue_interrupted_message` ~509-560; help text ~6219; `decide_custody` doc + recovery clause ~5594-5635; unit tests ~7981 and ~11063)
- Modify: `crates/cairn-sync/tests/requeue_retains_unlanded_custody.rs`
- Modify: `crates/cairn-sync/tests/common/dead_node.rs` (~351)
- Modify: `crates/cairn-node/src/restore/clinical.rs` (`CustodyDidNotLand`)
- Modify: `crates/cairn-node/tests/restore_cli_surface.rs` (~140), `crates/cairn-node/tests/common/restore_kit.rs` (~227-229)

- [ ] **Step 1: Change the unit tests first (they stop compiling — the failing state)** in `crates/cairn-sync/src/requeue.rs`:
  - delete `only_late_custody_owes_a_chart_rebuild`;
  - in `sample_counts`, delete `reproject_owed: 2,`;
  - in `every_count_reaches_the_metrics_object`, replace `assert_eq!(m["reproject_owed"], 2);` with `assert!(m.get("reproject_owed").is_none(), "retired by ADR-0070: the door projects a late key");`;
  - in `the_summary_line_names_every_count`, delete the `"2 needing \`cairn-node reproject\`",` needle and add `assert!(!line.contains("reproject"), "{line}");` after the loop;
  - in `a_run_is_incomplete_exactly_when_it_leaves_work`, delete the third `RequeueCounts { released: 1, released_with_custody: 1, reproject_owed: 1, .. }` element.
  In `crates/cairn-sync/src/main.rs`'s `an_interrupted_requeue_reports_the_work_that_survived_it`: delete `reproject_owed: 7,`, delete the `msg.contains("7 of them still need \`cairn-node reproject\`")` assertion, and add `assert!(!msg.contains("reproject"), "ADR-0070: a released record owes no heal: {msg}");`. In the `decide_custody` classifier test, replace the recoverable arm's `operator_line.contains("cairn_reproject")` assertion with:

```rust
                assert!(
                    !operator_line.contains("cairn_reproject"),
                    "{lookup:?}: since ADR-0070 the full sweep brings the record to the chart \
                     by itself; a second step would send the operator to run something that \
                     changes nothing: {operator_line}"
                );
```

Run: `. /tmp/cairn-584.env && cargo test -p cairn-sync --bins -- requeue decide_custody interrupted`
Expected: COMPILE ERROR — `missing field \`reproject_owed\` in initializer of \`RequeueCounts\`` (in `sample_counts` and in `an_interrupted_requeue_reports_the_work_that_survived_it`). That is the red state: the tests now describe a struct without the field. (The rewritten `decide_custody` assertion would fail at runtime too, once it compiles.)

- [ ] **Step 2: Retire the signal in `requeue.rs`**
  - delete `chart_rebuild_owed` (and its doc) and `chart_rebuild_message` (and its doc);
  - delete the `reproject_owed` field and its doc line from `RequeueCounts`; in its type doc, `**Three are SUBSETS**` becomes `**Two are SUBSETS**` and the clause `, and \`reproject_owed\` is the part of \`released_with_custody\` whose chart still needs a heal` is deleted;
  - remove `reproject_owed` from the destructurings in `accounted_for`, `metrics`, `summary_line`; from the `json!` object; from `is_incomplete` (`self.custody_retained > 0 || self.still_quarantined > 0`); `accounted_for`'s doc says `The two subsets are named and discarded`; `is_incomplete`'s doc drops `or a released record's chart still needs a heal`;
  - `summary_line` format: `"requeue: {examined} examined — {released} released ({released_with_custody} with custody, {released_shredded} shredded), {custody_retained} kept for custody, {skipped_acked} skipped (acked), {still_quarantined} still quarantined, {vanished} vanished"`;
  - `incomplete_notice` format: `"requeue: INCOMPLETE (exit {EXIT_INCOMPLETE}) — {} row(s) still held in the pen ({} kept for custody, {} still refused by the apply door). Each is named on its own line above, with its remedy."` with the three corresponding arguments;
  - module doc: in the paragraph naming the pure functions, drop `and [\`chart_rebuild_owed\`]` and say `[\`CustodyState\`] after the door`; replace the whole **What "released" does and does not promise.** paragraph with: `**What "released" promises.** A released keyed row's key has landed, and — since ADR-0070 — so has its chart: a key that lands on an event already in the log without it is projected by the apply door itself, so there is no heal step for this command to report.`;
  - `EXIT_INCOMPLETE` doc: `A run that retained rows, left rows the door still refuses, or landed custody for a record whose chart still needs a heal has done` → `A run that retained rows or left rows the door still refuses has done`; delete the whole `⚠️ **A chart owed a heal is reported by ONE run only.** …` paragraph;
  - `CustodyState` doc: `it counts a shredded release apart from a recovered one, and it compares the state before and after the door to spot a record whose key arrived after its chart was built ([\`chart_rebuild_owed\`]).` → `it counts a shredded release apart from a recovered one.`

- [ ] **Step 3: Retire it in `main.rs`**
  - requeue loop: `let keyed: Option<(requeue::PenKey, requeue::CustodyState)>` → `let keyed: Option<requeue::PenKey>`; the `Some(wrapped)` arm drops the `let before = custody_state(…)?;` read and returns `Some(pen)`; the comment `// STEP 1 and STEP 2, for a keyed row only: open its key, and read custody BEFORE the door.` → `// STEP 1, for a keyed row only: open its key.`; in `released_how`, `Some((pen, before)) =>` → `Some(pen) =>`, `Ok(how) => Some((pen, before, how)),` → `Ok(how) => Some((pen, how)),`; below, `if let Some((pen, before, how)) = released_how {` → `if let Some((pen, how)) = released_how {` and delete the `if requeue::chart_rebuild_owed(before, how) { … }` block;
  - `requeue_interrupted_message`: delete `reproject_owed,` from the destructuring; the comment `The two subsets that say only HOW a release happened are named and discarded; \`reproject_owed\` is not, because it is work the operator still has to do.` → `The two subsets that say only HOW a release happened are named and discarded.`; the format's `(durable in event_log, their pen rows gone; {reproject_owed} of them still need \`cairn-node reproject\` to reach the chart)` → `(durable in event_log, their pen rows gone)`;
  - help text: `(exit 3 = INCOMPLETE: rows are still held, or a released record's chart needs\n               \`cairn-node reproject\`; exit 1 = the run itself failed)` → `(exit 3 = INCOMPLETE: rows are still held in the pen; exit 1 = the run itself failed)`;
  - `decide_custody`'s recovery clause becomes: `" Once that is done the puller recovers the bodies it already replicated with \`cairn-sync pull --full\` (an incremental pull cannot reach events below its cursor). The re-offer carries the key, and the apply door brings each record to the chart as its key lands."`;
  - `decide_custody`'s doc: replace the `**Why the recovery clause names TWO steps, not "pull again".**` paragraph and the numbered list and the `Step 2 is the review finding …` paragraph with:

```rust
/// **Why the recovery clause names `pull --full`, not "pull again".** `apply_remote_event` has no
/// early return for an event already in the log, and its custody insert is `ON CONFLICT (event_id)
/// DO NOTHING`, so a re-offer that *does* carry a DEK fills in the missing `event_dek` /
/// `event_clear` rows. But an incremental pull only asks for `seq > cursor`, and by the time the
/// operator reads this line the cursor is already past the custody-less events. Only the full sweep
/// (`after_seq = 0`) re-offers them. (The periodic `FULL_SWEEP_EVERY` sweep gets there eventually;
/// `--full` is the same thing on demand.)
///
/// **It used to name a second step**, `cairn_reproject()`: the #231 review measured `pull --full`
/// alone taking custody from `(0,0)` to `(1,1)` while the chart stayed empty, because the
/// projection dispatcher is an `AFTER INSERT` trigger and the re-apply inserts nothing. ADR-0070
/// (#584) moved that step into the door — an apply that makes a body readable for an event already
/// in the log runs the event's heal-safe projections itself — and
/// `an_admitted_peer_recovers_the_bodies_it_pulled_without_custody` pins the one-step recovery.
```

- [ ] **Step 4: Finish the requeue integration test** in `crates/cairn-sync/tests/requeue_retains_unlanded_custody.rs`:
  - header: in `# What "the record came back" means here`, replace the sentence `Arm 1 now walks that whole road, including the \`cairn-node reproject\` step the run names.` with `Since ADR-0070 (#584) the door projects a key that lands on an already-admitted event, so arm 1 asserts the chart straight after the release, with no heal step.`; in `# Exit status`, `(rows still held, or a chart still to heal)` → `(rows still held)`; replace mutation 6 with `6. **Stop the door projecting a late key** (delete db/020's late-custody call) → arm 1 phase two, on the chart assertion. (Recorded in ADR-0070's plan, Task 7.)`;
  - arm 1 phase two: replace from `    assert_eq!(\n        m["reproject_owed"], 1,` through the end of phase three (the `SELECT * FROM cairn_reproject` call and its `medication_rows == 1` assertion, and the final complete-run block's `#584` comment) with:

```rust
    assert!(
        m.get("reproject_owed").is_none(),
        "the heal signal is retired: the door projected the late key (ADR-0070): {m}"
    );
    assert_eq!(
        code, 0,
        "every row released with its key and its chart: a COMPLETE recovery\nstderr: {stderr}"
    );
    assert!(
        !stderr.contains("reproject"),
        "no heal instruction for a record that needs none\nstderr: {stderr}"
    );

    // An empty pen stays a complete run.
    let (code, stdout, stderr) = run_requeue(&base, &key_path);
    assert_eq!(code, 0, "an empty pen is a complete run\nstderr: {stderr}");
    assert_eq!(metrics(&stdout, &stderr)["examined"], 0);
}
```

  - `dead_node.rs` doc: `rows still held, or a\n/// released record whose chart needs \`cairn-node reproject\` (#578 review).` → `rows still held in the pen (#578 review; the chart-heal cause retired with ADR-0070).`

- [ ] **Step 5: The restore remedy and two comments**
  - `crates/cairn-node/src/restore/clinical.rs`, `CustodyDidNotLand`: delete the final two sentences `Because this record is already in the log, its projection ran without the key and wrote no chart entry; the requeue run that lands the key names the \`cairn-node reproject\` heal that adds it — and only that run says so.` The string ends at `…without redoing the restore.`
  - `crates/cairn-node/tests/restore_cli_surface.rs` (~140): `— trap 9's class (#584), where the body opens and the list is empty.` → `— the class #584 closed at the door (ADR-0070), where the body opened and the list stayed empty.`
  - `crates/cairn-node/tests/common/restore_kit.rs` (`medication_rows` doc): replace `and a sealed event admitted WITHOUT its body leaves the chart empty\n/// even once custody lands later (#584, HANDOVER trap 9).` with `and until #584 a sealed event admitted WITHOUT its body left the chart empty even once custody\n/// landed later — the door now projects that landing (ADR-0070).`

- [ ] **Step 6: Search for any survivor**

Run: `grep -rn "reproject_owed\|chart_rebuild_owed\|chart_rebuild_message" crates/ ; grep -rn "cairn-node reproject" crates/cairn-sync crates/cairn-node/src/restore`
Expected: no output.

- [ ] **Step 7: Run everything this task touched**

Run: `. /tmp/cairn-584.env && cargo test -p cairn-sync --bins`
Expected: all pass.
Run: `. /tmp/cairn-584.env && cargo test -p cairn-sync --test requeue_retains_unlanded_custody --test requeue_releases_custody -- --test-threads=1`
Expected: all pass.
Run: `. /tmp/cairn-584.env && cargo test -p cairn-node --lib restore && cargo test -p cairn-node --test restore_cli_surface --test restore_cli_applies_nothing_untrusted --test restore_cli_survives_its_own_failure -- --test-threads=1`
Expected: all pass.

- [ ] **Step 8: fmt, clippy, doc, commit**

```bash
. /tmp/cairn-584.env && cargo fmt --all -- --check && cargo clippy -p cairn-node -p cairn-sync --all-targets -- -D warnings && RUSTDOCFLAGS="-D warnings" cargo doc -p cairn-node -p cairn-sync --no-deps
git add crates/cairn-sync crates/cairn-node/src/restore/clinical.rs crates/cairn-node/tests/restore_cli_surface.rs crates/cairn-node/tests/common/restore_kit.rs
git commit -m "feat(#584): requeue stops reporting a heal the door now performs

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

(`cargo doc -D warnings` matters: a surviving intra-doc link to the deleted `chart_rebuild_owed` fails two CI jobs.)

---

### Task 6: ADR-0070 and the spec

**Files:**
- Create: `docs/spec/decisions/0070-a-late-key-reaches-the-chart.md`
- Modify: `docs/spec/decisions/README.md` (append an index row after 0069)
- Modify: `docs/spec/index.md` (`**Spec version:** 0.71` → `0.72`)
- Modify: `docs/spec/language-substrate.md` (the `Projections are trigger-maintained incremental tables` bullet)

- [ ] **Step 1: Write ADR-0070** — follow the README template exactly (`Status: Accepted`, `Date: 2026-09-15`, no `Supersedes`; a `Refines:` line naming ADR-0057 and ADR-0052). Content, in this order:
  - **Context:** the four entrances table from design §1; the audit's two findings (§2.1, §2.2) with their citations; why the door and not a caller (restore's entrance has no signal); the rejected alternatives from design §3 with one reason each.
  - **Decision:** the four decisions of design §3, verbatim in substance, each numbered.
  - **Consequences:** ADR-0057's single `AFTER INSERT` dispatcher gains a second, decided dispatch site (registered appliers only — "a projection lives only in its registered apply function" still holds); `requeue`'s `reproject_owed`, its message and its exit-3 cause are gone; `pull --full` is one step; the healed state is arrival-at-custody-time, with the three residues of design §2.4 named; charts already missing a record on an existing database are not healed by upgrading (no generation bump) — `cairn-node reproject` still heals them; the guard's `prosrc` residual; how we'd know the bet failed (a `late_custody_guards.rs` failure, or a chart found empty after `requeue` exits 0).
  Link the spec file and the plan.

- [ ] **Step 2: README index row**

```markdown
| [0070](0070-a-late-key-reaches-the-chart.md) | **A late key reaches the chart**: custody that lands after its event was admitted used to open the body and leave the chart empty (the `AFTER INSERT` dispatcher never fires on a re-apply) — reached by `pull --full`, `requeue`, a keyless-first `restore` and a strict re-submit. Both doors now run the event's **heal-safe** registered appliers when they newly write `event_clear` for an event already in the log — after the substitution guard, in the door's own posture, only if replay-eligible — through one helper shared with re-adjudication; a catalog guard makes every custody-reading applier heal-safe, so `requeue`'s `reproject_owed` retires. Refines ADR-0057 and ADR-0052. |
```

- [ ] **Step 3: `language-substrate.md`.** Change `(\`AFTER INSERT\` only — the INSERT-only log means no update/delete maintenance path)` to `(\`AFTER INSERT\` on the log, plus ONE decided second path: when custody for an already-admitted sealed event arrives, the admitting door runs that event's heal-safe appliers — [ADR-0070](decisions/0070-a-late-key-reaches-the-chart.md); the INSERT-only log still means no update/delete maintenance path)`.

- [ ] **Step 4: Build the docs**

Run: `uv run --with-requirements docs/requirements.txt -- mkdocs build --strict`
Expected: exit 0 (if `--strict` is not what CI runs, match `.github/workflows` and drop it; a broken ADR link must still fail).

- [ ] **Step 5: Commit**

```bash
git add docs/spec
git commit -m "docs(#584): ADR-0070 — a late key reaches the chart (spec v0.72)

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

### Task 7: Mutation proofs

**Files:** none committed except the plan's review ledger. Every mutation is applied, run, recorded, and undone with `git show HEAD:<path> > <path>`; `git status --short` must show only the ledger afterwards.

- [ ] **Step 1: Run each mutation; record the test that failed and the assertion text that fired.** A mutation counts as killed only if the test fails **at the assertion that names its claim** (read the panic line). For SQL mutations, the rebuild is automatic.

| # | Mutation | Command | Must fail |
|---|---|---|---|
| M1 | db/020: delete the `IF v_rows = 0 AND v_clear_rows > 0 … END IF;` block | `cargo test -p cairn-node --test late_custody_reaches_the_chart --test restore_one_event_id_one_body -- --test-threads=1` | headline at `THE ASSERTION THAT MATTERS`; `a_keyless_copy_first_still_reaches_the_chart` |
| M2 | db/005 `submit_event`: delete its late-custody block | `… --test late_custody_reaches_the_chart the_strict_door` | `the_strict_door_brings_a_late_key_to_the_chart_too` at `statement_rows`; also `late_custody_guards` `every_custody_writer_projects_a_late_key` |
| M3 | db/005 helper: delete the `cairn_replay_eligible` check | `… --test heal_safe_dispatch --test late_custody_reaches_the_chart` | `late_custody_projection_skips_a_deferred_row`; `a_deferred_event_gains_its_key_but_not_its_chart_until_promoted` at the first `statement_rows` |
| M4 | db/005 dispatch: `AND heal_safe` → removed | `… --test heal_safe_dispatch --test late_custody_reaches_the_chart` | `the_dispatch_runs_only_heal_safe_appliers` (`unsafe_`); `a_late_key_runs_…_once_and_never_again` (`after_landing`) |
| M5 | db/020: move the late-custody block BELOW `set_config('cairn.remote_apply', '', true)` | `… --test late_custody_reaches_the_chart a_contradiction` | `a_contradiction_revealed_by_a_late_key_is_flagged_not_refused` at `landed.is_ok()` |
| M6 | db/020: move the late-custody block ABOVE the substitution guard | `… --test late_custody_reaches_the_chart a_rival` | `a_rival_body_…` — **if it survives, record it as surviving and why** (lenient appliers do not raise and the RAISE rolls the projection back, so the position is a legibility rule, not an observable one) |
| M7 | db/020: condition `IF v_rows = 0 THEN` (drop `AND v_clear_rows > 0`) | `… --test late_custody_reaches_the_chart a_late_key_runs` | `after_again` ((3, 1) vs (2, 1)) |
| M8 | a medication registration row `heal_safe` → FALSE (edit db/031's VALUES for `medication_statement_apply`) | `… --test late_custody_guards` | `every_custody_reading_applier_is_heal_safe` |
| M9 | db/043 `cairn_readjudicate_deferred`: add `INSERT INTO event_clear (event_id, body, twin) SELECT NULL::uuid, NULL, NULL WHERE false;` as the first statement after `BEGIN` (a PL/pgSQL custody writer without the call) | `… --test late_custody_guards` | `every_custody_writer_projects_a_late_key` at the writer-set assertion |
| M10 | M1 again (db/020's late-custody block deleted), observed through cairn-sync's binaries | `cargo test -p cairn-sync --test requeue_retains_unlanded_custody --test clinical_pull an_admitted -- --test-threads=1` | requeue arm 1 at the chart assertion; the pull test at `statement_count_for_med` |

(Each command begins with `. /tmp/cairn-584.env &&`.)

- [ ] **Step 2: Undo and verify clean** after each mutation: `git show HEAD:<path> > <path> && git status --short` (only the plan file may show).

- [ ] **Step 3: Write the results into `## Review ledger` below and commit the plan**

```bash
git add docs/superpowers/plans/2026-09-15-late-custody-reaches-the-chart-584.md
git commit -m "docs(#584): the plan records the mutation proofs

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

### Task 8: Gate, review, tracking documents, PR

- [ ] **Step 1: Start the full local gate in the background** (≈2 h on this machine; a cross-crate SQL change relinks every test binary): `scripts/run-db-gated-tests.sh > /tmp/cairn-584-gate.log 2>&1; echo "exit=$?" >> /tmp/cairn-584-gate.log`. While it runs, do Steps 2-4. If it cannot finish, say so in the PR and let CI's full job gate it — never claim a sweep that did not complete.

- [ ] **Step 2: Code review** — dispatch a reviewer over `git diff main...HEAD` with the spec, the four placement hazards, and house rules 1-8. Fix every finding in place or file an issue (house rule 5); record each in the ledger.

- [ ] **Step 3: HANDOVER.** Retire trap 9 into a one-paragraph history note (*"closed by #584/ADR-0070: the door projects a late key; do not remove the call or its placement after the substitution guard and before the marker clear"*); add trap 10: *every `event_clear` writer calls `cairn_project_late_custody`, and a custody-reading applier is heal-safe — both pinned by `late_custody_guards.rs`*; ⇒ NEXT: #584 closed by this PR, #594 decided-not-built as the next DR item, #575 deferred; session date line; spec v0.72; prune toward 500 lines without dropping an open issue number (Task 0 Step 4's check).

- [ ] **Step 4: ROADMAP.** A condensed #584 entry (ADR-0070, no migration, no SCHEMA bump, what retired); correct Slice 66's *"Repair is TWO steps"*; keep every open issue number.

- [ ] **Step 5: Paper-parity guard and docs**

Run: `. /tmp/cairn-584.env && cargo test -p cairn-node --test paper_parity_plan_section`
Expected: pass.

- [ ] **Step 6: Commit, push, PR**

```bash
git add docs/HANDOVER.md docs/ROADMAP.md docs/superpowers/plans/2026-09-15-late-custody-reaches-the-chart-584.md
git commit -m "docs(#584): HANDOVER and ROADMAP record ADR-0070

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
git push -u origin feat/584-late-custody-reaches-the-chart
gh pr create --title "A late key reaches the chart (#584, ADR-0070)" --body-file /tmp/cairn-584-pr.md
```

The PR body names what changed per layer, the four entrances before/after, the mutation table's results, what the local gate did and did not run, and ends with `Closes #584` **on its own line** plus the attribution line. Open it as a **draft** if the gate has not finished.

---

## Paper-parity benchmark (§1.2)

**Paper counterpart:** a page that reached the ward before the chart it belongs in — held at the nurses' station, then filed once the chart turns up. Recovery is **N = 1** act (file the page).

**Steps — architecture-forced, before → after** (the acts on the recovery path once the underlying fault — an unregistered key, an unadmitted peer — is fixed; that repair is a provisioning act with no paper counterpart, owned where the fault is: ADR-0066 and #512 for restore's keys, pairing for a peer):

| Entrance | Before | After |
|---|---|---|
| `requeue` | M = 2 (requeue, then an owner-privileged `cairn-node reproject`) | **M = 1** |
| `pull --full` | M = 2 (pull --full, then `cairn_reproject()` as DB owner) | **M = 1** |
| `restore`, keyless copy first | the record silently missing | **M = 0** extra |
| `submit_event` re-submit | the record silently missing | **M = 0** extra |

**UI bundling target K = 1.** `M = N` on every entrance; the slice removes an act rather than adding one. **Time + cognitive load:** a late landing costs the same registered appliers a first arrival runs, once, plus one `GET DIAGNOSTICS` per sealed write; no new runnable surface is exposed, so no measurement is owed by this slice (the ordinary sealed-write cost is measured: median 222 ms node-tier, Slice 61). Cognitive load falls too: an operator no longer has to know that a released record may still be missing from the chart, or which owner-privileged command brings it back.

---

## Deviations from the spec (decided while planning)

- **Design test 5.1.6 (a shredded target) is not written.** It cannot fail under any mutation of this slice: a shredded target never gets an `event_clear` row, so `v_clear_rows` stays 0, and every custody-reading applier returns on a NULL clear view even if the call ran. Anti-resurrection itself stays pinned where it lives (`requeue_retains_unlanded_custody.rs` arm 5, `shred_predicate_has_one_home.rs`).
- **5.1.2 and 5.1.7 are one test** (`a_late_key_runs_the_heal_safe_appliers_once_and_never_again`): the counting probe observes both "once" and "never the unsafe one", and an idempotent medication applier could not observe "once" at all.
- **`v_clear_written boolean` is `v_clear_rows integer`**: `GET DIAGNOSTICS` writes a count.
- **The source guard is a catalogue guard** (`pg_proc.prosrc`), not a scan of `db/*.sql`: the catalogue is what runs, and it needs no SQL function-block parser.
- The spec's two guard files are one, `late_custody_guards.rs`, both reading the catalogue.
- **The helpers get their own direct-call suite** (`heal_safe_dispatch.rs`), so a helper failure is never read as a door failure.

## Review ledger

(Filled during execution: each task review, each mutation result from Task 7, each final-review finding and its disposition.)

### Task 7 — mutation proofs

| Mutation | Test(s) that failed | Assertion that fired (short quote) | Verdict (killed/survived) |
|---|---|---|---|
| M1 | `a_key_arriving_after_its_event_brings_the_record_to_the_chart`; `a_contradiction_revealed_by_a_late_key_is_flagged_not_refused`; `a_late_key_runs_the_heal_safe_appliers_once_and_never_again`; `a_keyless_copy_first_still_reaches_the_chart` | "THE ASSERTION THAT MATTERS: the record is on the medication list, with no reproject" (left: 0, right: 1) | killed |
| M2 | `the_strict_door_brings_a_late_key_to_the_chart_too`; `late_custody_guards::every_custody_writer_projects_a_late_key` | "the strict door projects a late key exactly as the lenient one does" (left: 0, right: 1); "submit_event writes event_clear but never calls cairn_project_late_custody" | killed |
| M3 | `heal_safe_dispatch::late_custody_projection_skips_a_deferred_row`; `a_deferred_event_gains_its_key_but_not_its_chart_until_promoted` | "a deferred row must not project through the late-custody path" (left: 1, right: 0); "a deferred event has not passed its gates: the late key must not project it" (left: 1, right: 0) | killed |
| M4 | `heal_safe_dispatch::the_dispatch_runs_only_heal_safe_appliers`; `a_late_key_runs_the_heal_safe_appliers_once_and_never_again` | "a heal_safe = false applier is never re-run over a live row — that is what the flag means" (left: 2, right: 1); "the landing re-runs the heal-safe applier and NOT the counter-shaped one" (left: (2,2), right: (2,1)) | killed |
| M5 | `a_contradiction_revealed_by_a_late_key_is_flagged_not_refused` | "the key must land — a refusal here strands it forever: Some(\"medication thread ...\")" (`landed.is_ok()`) | killed |
| M6 | `a_rival_body_never_reaches_an_applier` (added in the final fix wave; in Task 7 no test failed — `a_rival_body_carrying_its_own_key_is_refused_and_projects_nothing` passed unchanged, because lenient appliers do not raise and the RAISE rolls the projection back) | "the door refuses the rival as a substitution: cairn_test probe: an applier ran over this row" (`db_msg(&err).contains("substitution refused")`) | killed by a_rival_body_never_reaches_an_applier (final fix wave) at `db_msg(&err).contains("substitution refused")` — a raising heal-safe probe replaces the door's refusal when the call precedes the guard |
| M7 | `a_late_key_runs_the_heal_safe_appliers_once_and_never_again` | "custody already held: nothing new landed, so nothing runs — the trigger for the heal is 'this call wrote event_clear', never 'the INSERT was a no-op'" (left: (3,1), right: (2,1)) | killed |
| M8 | `late_custody_guards::every_custody_reading_applier_is_heal_safe` | "these appliers read custody but are registered heal_safe = false, ... [(\"clinical.medication.asserted\", \"medication_statement_apply\", false)]" | killed |
| M9 | `late_custody_guards::every_custody_writer_projects_a_late_key` | "the custody writers are the two doors. A third is a DECISION" (left: [apply_remote_event, cairn_readjudicate_deferred, submit_event], right: [apply_remote_event, submit_event]) | killed |
| M10 | `cairn-sync::requeue_retains_unlanded_custody::an_unregistered_unwrap_key_keeps_the_pen_row_and_the_fix_reaches_the_chart`; `cairn-sync::clinical_pull::an_admitted_peer_recovers_the_bodies_it_pulled_without_custody` | "the recovered record is on the chart as soon as its key lands" (left: 0, right: 1); "...and the chart has it, with no reproject: the door projects custody that lands after its event (#584, ADR-0070)" (left: 0, right: 1) | killed |

Full per-mutation evidence (edited lines, commands, panic text, undo confirmation) is in
`.superpowers/sdd/2026-09-15-late-custody-reaches-the-chart-584/task-7-report.md`.
