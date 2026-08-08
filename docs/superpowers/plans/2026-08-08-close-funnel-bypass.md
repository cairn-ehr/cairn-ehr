# Close the search-before-create bypass (#345) — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the §5.3/§5.8 funnel **unbypassable** — the first event carrying a new `patient_id`
must be a registration — and retire the legacy unfloored `patient.created` so the rule has no
"unless".

**Architecture:** One pure predicate (`cairn_patient_has_events`, db/001) and one refusal at the
**strict local door only** (`submit_event`, db/005). The remote door (db/020) is deliberately
untouched: set-union sync has no ordering, so a peer's clinical event legitimately precedes the
registration licensing it, and a fail-closed remote door would wedge replication on honest traffic
([ADR-0061](../../spec/decisions/0061-registration-is-an-act-that-carries-its-search.md) decision 3;
same shape as ADR-0051 strict-submit/lenient-apply, ADR-0058, and #268). `patient.created` is retired
by deleting its projection registrations and then its `event_type_class` row (db/047), and
registration takes over its `patient_chart` chart-birth projection. The bulk of the diff is the test
fixture sweep the rule forces.

**Tech Stack:** PostgreSQL 18 + `cairn_pgx`, PL/pgSQL, Rust 1.96 (workspace-pinned),
`tokio-postgres`, `uuid`, `serde_json`.

**Spec:** [`docs/superpowers/specs/2026-08-04-search-before-create-funnel-design.md`](../specs/2026-08-04-search-before-create-funnel-design.md) §2.3 / §4.2
**Issue:** [#345](https://github.com/cairn-ehr/cairn-ehr/issues/345)
**Branch:** `feat/close-funnel-bypass-345` (already created)

## Global Constraints

- **Licence:** AGPL-3.0. Every dependency AGPL-3.0-compatible, checked *before* adding. This plan
  adds **no new third-party dependency**.
- **TDD, always:** failing test first, watch it fail, minimal code, watch it pass, commit.
- **Every commit is green.** House rule 6. That is why the fixture sweep (Task 2) lands *before* the
  rule (Task 3): a registration event is additive and harmless without the rule, so the suite stays
  green at every commit boundary. Never commit a red suite.
- **Inline docs for a junior developer** on every non-trivial function/module: *why* it exists and
  how it fits, not what the next line does.
- **The rule is LOCAL-SUBMIT ONLY.** Do not add the check to `db/020_apply_remote_event.sql`, and do
  not "make the doors symmetric". A test asserts the lenient remote admission explicitly.
- **No authorship gate.** This slice adds no authorship requirement anywhere; an unattested standard
  registration must keep succeeding (ADR-0061 decision 4, §5.11 — a grade, not a gate).
- **Retire in the right order.** `db/005`'s projection-registry validation trigger (`db/005:153-162`)
  states the invariant explicitly: a migration retiring a type must **DELETE its
  `cairn_projection_apply` rows FIRST**, then its `event_type_class` row. The reverse order leaves a
  registered-but-unclassified type — the exact state the trigger exists to make unreachable.
- **This is the first migration in the repo that DELETEs from `event_type_class`.** `db/005:159`
  currently asserts "no migration ever DELETEs from event_type_class". That sentence must be
  corrected in the same commit that makes it false.
- **UUIDs bind as text.** `cairn-node` does not enable `tokio-postgres`'s `with-uuid-1`. Bind
  `&uuid.to_string()` and cast in SQL: `$1::text::uuid`.
- **Guard before connect.** DB-gated tests take `db::test_serial_guard(&base)` *before*
  `connect_and_load_schema`.
- **Run the FULL workspace suite** via `scripts/run-db-gated-tests.sh` (SQL mirrors + `cargo test`
  with `CAIRN_TEST_PG`/`PG2`/`PG3` baked in), never `-p cairn-node` alone — a per-crate run hides
  cross-crate call-site breaks in `cairn-sync/tests/`. Do not pipe to `tail`; it masks the exit code.
- **Never hard-code cryptographic material in tests.** Derive keys/seeds at runtime.
- **Do not start the tech-debt loop while this session holds the repo** (HANDOVER: two concurrent
  `cargo test --workspace` runs contend on one cargo lock and one `test_serial_guard` advisory lock).

> **How this was actually executed, and where it departed from the plan below.** The task order
> assumed cheap iteration; the local gate turned out to cost **~1.5–2 h per full run** (≈86 test
> binaries, each replaying 47 migrations and serializing on one cluster-wide advisory lock), so the
> "measure, revert, sweep, re-apply" loop of Tasks 1–2 was collapsed: the floor and the retirement
> were applied together and the sweep was driven by successive compile-and-run passes. The
> consequence for review is stated plainly rather than implied: **the slice was verified green as a
> whole, not commit-by-commit**, so it lands as one commit rather than the three this plan sketched.
> Two decisions were also reversed by what the code turned out to say — the `cairn-sync` SCHEMA
> subset (Task 4 step 8, rewritten below) and the placement of the registration → `patient_chart`
> registry row (db/047 rather than db/005, because db/005 loads before the type is classified).

---

## File Structure

| File | Responsibility | Task |
|---|---|---|
| `crates/cairn-node/tests/common/mod.rs` | `submit_registration` fixture replaces `submit_patient_created` | 2 |
| ~38 `crates/cairn-node/tests/*.rs` | register each patient before its first event; `patient.created` → `patient.amended` where the type was only a vehicle | 2, 4 |
| `db/001_envelope.sql` | `cairn_patient_has_events(uuid)` — pure, one indexed lookup | 3 |
| `db/005_submit.sql` | step 8b: the precedence refusal; seed-row + registry edits; correct the stale "no migration ever DELETEs" note | 3, 4 |
| `crates/cairn-node/tests/patient_precedence.rs` | **new** — the rule's own DB-gated tests | 3 |
| `db/tests/005_submit_test.sql` | SQL mirror of the precedence refusal | 3 |
| `db/002_projection.sql` | `patient_chart_apply` gains the registration (chart-birth) branch | 4 |
| `db/047_registration_precedence.sql` | **new** — registration takes over `patient_chart`; retire `patient.created` (projection rows, then class row) | 4 |
| `crates/cairn-event/src/schema_generation.rs` | `SCHEMA_GENERATION` 46 → 47 | 4 |
| `crates/cairn-node/src/db.rs` | `SCHEMA` list gains `047_registration_precedence` | 4 |
| `crates/cairn-sync/src/main.rs` | `SCHEMA` subset gains `045` + `047`; bench/test `patient.created` → `patient.amended` | 4 |
| `db/008_surrogate_projection.sql`, `db/tests/008_surrogate_test.sql`, `db/bench/b5_surrogate.sql` | spike-tier rig: `patient.created` → `patient.amended` | 4 |
| `crates/cairn-event/src/lib.rs`, `crates/cairn-event/tests/clock_grade.rs` | serialization fixtures: `patient.created` → `patient.amended` | 4 |
| `docs/spec/identity.md` | §5.8's "not yet turned on" WARNING becomes the enforced statement | 5 |
| `db/045_patient_registration.sql` | header note "the precedence rule belongs to #345" → "shipped in db/047" | 5 |
| `cairn-gui/cairn-gui-tauri/results/RUNBOOK.md` | seed step must `patient-register` before `medication-assert` | 5 |
| `docs/HANDOVER.md`, `docs/ROADMAP.md` | current state | 5 |

**Guard files that MUST move with db/047 (Task 4):** `crates/cairn-event/src/schema_generation.rs`
(46 → 47) · `crates/cairn-node/src/db.rs` (SCHEMA list). There is **no** twin-registry count change
(no new event type) and **no** new ADR (ADR-0061 already decided this; its deferral note is
historical and stays standing — errata are for claims that were false, not for work that has since
landed).

---

## Task 1: Measure the sweep — do not estimate it

The design doc's own §2.3 correction note is the precedent: the first two counts of this work were
both wrong. Measure by running the real gate with the rule on, and keep the artifact.

**Files:** none committed. Working-tree-only patch, reverted at the end of the task.

- [ ] **Step 1: Confirm a green baseline first**

```bash
scripts/run-db-gated-tests.sh
```

Expected: `== all db/tests/*.sql passed`, then every `test result: ok`. If the baseline is red, stop
and fix that first — a red baseline makes the measurement below unreadable.

- [ ] **Step 2: Apply the rule temporarily (working tree only, no commit)**

Add to `db/001_envelope.sql`, after the `event_log_patient_idx` index:

```sql
CREATE OR REPLACE FUNCTION cairn_patient_has_events(p_patient_id uuid)
RETURNS boolean LANGUAGE sql STABLE AS $$
    SELECT EXISTS (SELECT 1 FROM event_log WHERE patient_id = p_patient_id)
$$;
```

Add to `db/005_submit.sql`, immediately after step 8 (`v_twin := cairn_event_twin(...)`) and before
step 9:

```sql
    IF v_type <> 'identity.registration.asserted'
       AND NOT cairn_patient_has_events((b ->> 'patient_id')::uuid) THEN
        RAISE EXCEPTION 'submit_event: no chart exists for patient % — the first event on a chart must be its registration (identity.registration.asserted, §5.3/§5.8)',
            b ->> 'patient_id';
    END IF;
```

- [ ] **Step 3: Run the gate and capture the inventory**

```bash
scripts/run-db-gated-tests.sh 2>&1 | tee /tmp/345-sweep.log; \
  grep -E "^(test .* FAILED|failures:)" -A2 /tmp/345-sweep.log | sort -u > /tmp/345-failing-tests.txt
```

Expected: a large number of failures, every one of them reporting `no chart exists for patient`.
**Any failure with a different message is a finding, not noise** — read it before continuing (it
means the rule broke something other than fixture ordering, e.g. a product path that authors a
non-registration event first).

- [ ] **Step 4: Record the measured numbers in this plan**

Replace this line with the measured figures: *N failing tests across M files; P product (non-test)
call sites affected.* The measured list drives Task 2 and is the reviewer's coverage check.

- [ ] **Step 5: Revert the temporary patch**

```bash
git checkout -- db/001_envelope.sql db/005_submit.sql
git diff --stat   # expect: empty
```

---

## Task 2: The registration fixture + the sweep

**Files:**
- Modify: `crates/cairn-node/tests/common/mod.rs` (replace `submit_patient_created`)
- Modify: every test file in `/tmp/345-failing-tests.txt`
- Test: the existing suite is the test — it must stay green *without* the rule

**Interfaces:**
- Produces: `common::submit_registration(c: &Client, sk: &SigningKey, kid: &str, p: Uuid, wall: i64)`
  — submits one `identity.registration.asserted`, class `standard`, through the real door. Panics on
  refusal (it is always setup, never the thing under test).
- Removes: `common::submit_patient_created` (its event type is retired in Task 4).

- [ ] **Step 1: Write the failing test for the fixture itself**

Add to `crates/cairn-node/tests/patient_registration.rs`:

```rust
/// The shared fixture must produce a registration the db/045 floor ACCEPTS and the projection
/// materialises — otherwise ~38 suites would be arranging their charts with an event that only
/// looks like a registration.
#[tokio::test]
async fn shared_fixture_registers_a_chart() {
    let Some(base) = common::cs() else { return };
    let _g = db::test_serial_guard(&base).await;
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = common::setup(&c, &["patient_registration"]).await;
    let p = Uuid::now_v7();

    common::submit_registration(&c, &sk, &kid, p, 1_000).await;

    let class: String = c
        .query_one(
            "SELECT class FROM patient_registration_current WHERE patient_id = $1::text::uuid",
            &[&p.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(class, "standard");
}
```

- [ ] **Step 2: Run it and watch it fail**

```bash
scripts/run-db-gated-tests.sh
```

Expected: FAIL — `no method named submit_registration` (compile error).

- [ ] **Step 3: Implement the fixture**

In `crates/cairn-node/tests/common/mod.rs`, delete `submit_patient_created` and add:

```rust
/// Submit the §5.3 registration act that brings a chart into being, so a suite can then author
/// events about that patient.
///
/// Replaces the old `submit_patient_created`. Since #345 the in-DB floor requires the FIRST event
/// carrying a `patient_id` to be a registration (`submit_event`, db/005 step 8b), so this is not a
/// convenience — it is the arrangement step every chart needs, and a suite that skips it gets a
/// legible refusal naming this rule.
///
/// Class `standard` with a search that found nothing (`displayed: []`) is the honest fixture: it is
/// the normal case for a genuinely new patient, and it exercises the fuller floor path (the
/// non-standard classes skip §2d-§2g of db/045 entirely). Unwrapped, because this is always setup
/// for the real assertion, never the thing under test.
pub async fn submit_registration(c: &Client, sk: &SigningKey, kid: &str, p: Uuid, wall: i64) {
    let name = p.to_string();
    let tokens = [name.clone()];
    let a = RegistrationAssertion {
        class: RegistrationClass::Standard,
        basis: None,
        search: Some(SearchAttestationInput {
            terms: SearchTerms {
                name_tokens: &tokens,
                birth_date: None,
                identifiers: &[],
            },
            displayed: &[],
            incomplete: false,
        }),
    };
    submit_signed(
        c,
        sk,
        kid,
        EventSpec {
            patient: p,
            event_type: REGISTRATION_EVENT_TYPE,
            schema_version: REGISTRATION_SCHEMA_VERSION,
            payload: registration_assertion_body(&a),
            plaintext_twin: Some(render_registration_twin(&a)),
            wall,
        },
    )
    .await
    .expect("registration accepted");
}
```

with the imports:

```rust
use cairn_event::registration::{
    registration_assertion_body, render_registration_twin, RegistrationAssertion,
    RegistrationClass, SearchAttestationInput, SearchTerms, REGISTRATION_EVENT_TYPE,
    REGISTRATION_SCHEMA_VERSION,
};
```

- [ ] **Step 4: Run the fixture test and watch it pass**

```bash
scripts/run-db-gated-tests.sh
```

Expected: `shared_fixture_registers_a_chart ... ok`. Other suites may now fail *only* where they
called the removed `submit_patient_created` (compile errors) — convert those call sites to
`submit_registration` in this step.

- [ ] **Step 5: Sweep the measured inventory**

For every test in `/tmp/345-failing-tests.txt`, add `common::submit_registration(&c, &sk, &kid, p,
<wall>).await;` (or the suite's own equivalent helper) before the first event authored for that
patient. Rules for the sweep — **the review value of this task is in each of these judgements**:

1. **Pick a wall BELOW the test's own walls.** A registration is the chart's birth act; a fixture
   registering at a higher HLC than the events it precedes is a lie the projections can read
   (`patient_registration_current` picks the EARLIEST). Where a suite starts at `WALL_2026`, register
   at `WALL_2026 - 1`.
2. **A suite with its own `setup`-shaped helper registers there**, not in each test, when every test
   in the file uses one patient minted by that helper.
3. **Do not register a patient the test deliberately leaves chart-less.** Some suites assert that a
   read surfaces nothing for an unknown patient; those stay unregistered and must keep passing.
4. **Watch for count assertions.** `SELECT count(*) FROM event_log`, sync watermarks, `seq` cursors
   and reproject counts all move by one per registered patient. Update the expected number; do not
   loosen the assertion into a range.
5. **Add `patient_registration` to a suite's TRUNCATE list** when it counts registration rows or
   reads `patient_registration_current`. Do NOT add it blindly everywhere — `common::setup`'s list is
   shared, and #340's lesson is that near-identical truncation lists are not interchangeable.

- [ ] **Step 6: Run the full gate**

```bash
scripts/run-db-gated-tests.sh
```

Expected: fully green **without the rule applied** — registrations are additive, so this commit
stands on its own.

- [ ] **Step 7: Commit**

```bash
git add crates/cairn-node/tests
git commit -m "test(#345): register every fixture chart before its first event

The precedence rule lands in the next commit; the fixtures move first so
every commit boundary stays green. submit_patient_created is replaced by
submit_registration, which authors the real §5.3 act through the real door."
```

---

## Task 3: The precedence rule at the strict door

**Files:**
- Modify: `db/001_envelope.sql` (add `cairn_patient_has_events`)
- Modify: `db/005_submit.sql` (step 8b)
- Create: `crates/cairn-node/tests/patient_precedence.rs`
- Modify: `db/tests/005_submit_test.sql` (SQL mirror)

**Interfaces:**
- Produces: `cairn_patient_has_events(p_patient_id uuid) RETURNS boolean` — pure, `STABLE`,
  `LANGUAGE sql` so it inlines; one lookup on `event_log_patient_idx`.

- [ ] **Step 1: Write the failing tests**

Create `crates/cairn-node/tests/patient_precedence.rs`:

```rust
//! #345 — the §5.3/§5.8 precedence rule: the first event carrying a new `patient_id` must be its
//! registration. Enforced at the STRICT local door only (ADR-0061 decision 3).

mod common;

use cairn_event::{sign, ClockGrade, EventBody, Hlc};
use cairn_node::db;
use common::{cs, db_msg, setup, submit_registration, submit_signed, EventSpec};
use uuid::Uuid;

const WALL: i64 = 1_780_000_000_000;

/// A bare name assertion on a fresh `patient_id` is REFUSED. This is the bypass #344 left open:
/// before this rule a client minted a chart simply by asserting something about a new UUID.
#[tokio::test]
async fn a_first_event_that_is_not_a_registration_is_refused() {
    let Some(base) = cs() else { return };
    let _g = db::test_serial_guard(&base).await;
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &["patient_registration"]).await;
    let p = Uuid::now_v7();

    let err = submit_signed(
        &c,
        &sk,
        &kid,
        EventSpec {
            patient: p,
            event_type: "demographic.field.asserted",
            schema_version: "demographic.field.asserted/1",
            payload: serde_json::json!({
                "field": "birth_date", "value": "1980-01-01",
                "provenance": "patient-stated", "precision": "day"
            }),
            plaintext_twin: Some("Birth date 1980-01-01 (patient-stated)".into()),
            wall: WALL,
        },
    )
    .await
    .expect_err("a chart-less first event must be refused");

    assert!(
        db_msg(&err).contains("must be its registration"),
        "the refusal must name the rule, not merely fail: {}",
        db_msg(&err)
    );
}

/// The SAME assertion succeeds once the chart has been registered — the rule gates the ORDER, never
/// the content.
#[tokio::test]
async fn the_same_event_succeeds_after_registration() {
    let Some(base) = cs() else { return };
    let _g = db::test_serial_guard(&base).await;
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &["patient_registration"]).await;
    let p = Uuid::now_v7();

    submit_registration(&c, &sk, &kid, p, WALL - 1).await;
    submit_signed(
        &c,
        &sk,
        &kid,
        EventSpec {
            patient: p,
            event_type: "demographic.field.asserted",
            schema_version: "demographic.field.asserted/1",
            payload: serde_json::json!({
                "field": "birth_date", "value": "1980-01-01",
                "provenance": "patient-stated", "precision": "day"
            }),
            plaintext_twin: Some("Birth date 1980-01-01 (patient-stated)".into()),
            wall: WALL,
        },
    )
    .await
    .expect("accepted once the chart exists");
}

/// THE LOAD-BEARING LENIENT CASE. A peer's clinical event legitimately arrives before the
/// registration licensing it — set-union sync has no ordering. `apply_remote_event` must ADMIT it:
/// a fail-closed remote door would wedge replication on entirely honest traffic (ADR-0061 decision
/// 3; the failure mode ADR-0056 / ADR-0058 / #268 each hit).
#[tokio::test]
async fn the_remote_door_admits_an_event_for_an_unregistered_chart() {
    let Some(base) = cs() else { return };
    let _g = db::test_serial_guard(&base).await;
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &["patient_registration"]).await;
    let p = Uuid::now_v7();

    let body = EventBody {
        event_id: Uuid::now_v7().to_string(),
        patient_id: p.to_string(),
        event_type: "demographic.field.asserted".into(),
        schema_version: "demographic.field.asserted/1".into(),
        hlc: Hlc { wall: WALL, counter: 0, node_origin: "peer".into() },
        t_effective: None,
        signer_key_id: kid.clone(),
        contributors: serde_json::json!([{"actor_id": kid, "role": "recorded"}]),
        payload: serde_json::json!({
            "field": "birth_date", "value": "1980-01-01",
            "provenance": "patient-stated", "precision": "day"
        }),
        attachments: vec![],
        plaintext_twin: Some("Birth date 1980-01-01 (patient-stated)".into()),
        clock_grade: ClockGrade::SelfAsserted,
    };
    let signed = sign(&body, &sk).unwrap();

    c.execute("SELECT apply_remote_event($1)", &[&signed.signed_bytes])
        .await
        .expect("the remote door must never refuse on the precedence rule");

    let n: i64 = c
        .query_one(
            "SELECT count(*) FROM event_log WHERE patient_id = $1::text::uuid",
            &[&p.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(n, 1, "the out-of-order peer event must be stored, not penned");
}

/// A chart with no registration act on file stays QUERYABLE — the one-line projection read. The
/// lenient remote door means such charts exist by design until the peer's registration syncs; a read
/// path that hid them would turn a sync-ordering artefact into a clinical disappearance.
#[tokio::test]
async fn an_unregistered_chart_is_still_readable() {
    let Some(base) = cs() else { return };
    let _g = db::test_serial_guard(&base).await;
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &["patient_registration"]).await;
    let p = Uuid::now_v7();

    let body = EventBody {
        event_id: Uuid::now_v7().to_string(),
        patient_id: p.to_string(),
        event_type: "demographic.field.asserted".into(),
        schema_version: "demographic.field.asserted/1".into(),
        hlc: Hlc { wall: WALL, counter: 0, node_origin: "peer".into() },
        t_effective: None,
        signer_key_id: kid.clone(),
        contributors: serde_json::json!([{"actor_id": kid, "role": "recorded"}]),
        payload: serde_json::json!({
            "field": "birth_date", "value": "1980-01-01",
            "provenance": "patient-stated", "precision": "day"
        }),
        attachments: vec![],
        plaintext_twin: Some("Birth date 1980-01-01 (patient-stated)".into()),
        clock_grade: ClockGrade::SelfAsserted,
    };
    let signed = sign(&body, &sk).unwrap();
    c.execute("SELECT apply_remote_event($1)", &[&signed.signed_bytes])
        .await
        .unwrap();

    let unregistered: i64 = c
        .query_one(
            "SELECT count(*) FROM patient_demographic d \
               WHERE d.patient_id = $1::text::uuid \
                 AND NOT EXISTS (SELECT 1 FROM patient_registration_current r \
                                  WHERE r.patient_id = d.patient_id)",
            &[&p.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(unregistered, 1, "an unregistered chart must remain readable and findable");
}

/// A SECOND registration for a chart that already has events is accepted — registration is exempt
/// from the rule with no "unless", and a duplicate registration is EVIDENCE the retained-set
/// projection must keep (db/045 §4).
#[tokio::test]
async fn a_later_registration_is_never_refused() {
    let Some(base) = cs() else { return };
    let _g = db::test_serial_guard(&base).await;
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &["patient_registration"]).await;
    let p = Uuid::now_v7();

    submit_registration(&c, &sk, &kid, p, WALL).await;
    submit_registration(&c, &sk, &kid, p, WALL + 1).await;

    let n: i64 = c
        .query_one(
            "SELECT count(*) FROM patient_registration WHERE patient_id = $1::text::uuid",
            &[&p.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(n, 2, "both registrations are retained as evidence");
}
```

- [ ] **Step 2: Run and watch them fail**

```bash
scripts/run-db-gated-tests.sh
```

Expected: `a_first_event_that_is_not_a_registration_is_refused` FAILS (the submit succeeds); the
other four pass. That asymmetry is the point — it proves the first test is testing the new rule and
not something the floor already did.

- [ ] **Step 3: Add the predicate (db/001)**

After the `event_log_patient_idx` index in `db/001_envelope.sql`:

```sql
-- Does this chart exist yet? (§5.3/§5.8 precedence, ADR-0061 decision 3, issue #345.)
--
-- "The chart exists" is expressed as "the log holds at least one event for this patient" rather
-- than "a patient_registration row exists", and the difference is load-bearing: the REMOTE door is
-- lenient by design, so a chart can legitimately hold a peer's clinical event before the
-- registration licensing it has synced. Keying the rule on the projection would refuse the next
-- LOCAL write to such a chart — punishing this node for the wire's lack of ordering, which is
-- exactly the wedge ADR-0061 decision 3 exists to avoid.
--
-- LANGUAGE sql + STABLE so the planner inlines it into the caller as a plain EXISTS over
-- event_log_patient_idx (one indexed lookup on the write path, per submit).
CREATE OR REPLACE FUNCTION cairn_patient_has_events(p_patient_id uuid)
RETURNS boolean LANGUAGE sql STABLE AS $$
    SELECT EXISTS (SELECT 1 FROM event_log WHERE patient_id = p_patient_id)
$$;
-- Locked down like every predicate the doors call: submit_event is SECURITY DEFINER and calls this
-- as the migration-defining owner, so no runtime role needs EXECUTE.
REVOKE EXECUTE ON FUNCTION cairn_patient_has_events(uuid) FROM PUBLIC;
```

- [ ] **Step 4: Add the refusal (db/005 step 8b)**

Immediately after step 8 (`v_twin := cairn_event_twin(v_type, b_clear);`) and before step 9:

```sql
    -- 8b. The §5.3/§5.8 PRECEDENCE RULE (ADR-0061 decision 3, issue #345): the first event
    --     carrying a patient_id must be that chart's registration. This is what makes the
    --     search-before-create funnel unbypassable — without it a client mints a chart simply by
    --     asserting a name, and the §5.8 obligation to record the search that preceded the create
    --     has nothing to attach to.
    --
    --     ONE RULE, NO "UNLESS". Every registration class rides one event type (§5.3, ADR-0061
    --     decision 2) precisely so this sentence needs no carve-out — not for John Doe, not for
    --     the legacy patient.created (retired in db/047). An "unless" in a safety floor is where
    --     the next defect lives.
    --
    --     PLACEMENT is deliberate: last of the refusals, after every check that judges the EVENT
    --     itself (signature, clock, contributors, actor, attestation, seal, twin, structural
    --     floor) and before anything is written. A defect in the event is the author's first
    --     problem; the chart's history is the second. Ordering among refusals is a legibility
    --     property, never a safety one — every path here refuses — but it means an event that is
    --     wrong in two ways still reports the reason it always reported.
    --
    --     STRICT DOOR ONLY. db/020 (apply_remote_event) must NEVER call this: set-union sync has
    --     no ordering, so a peer's clinical event legitimately precedes the registration that
    --     licenses it, and a fail-closed remote door would freeze the puller's watermark on
    --     entirely honest traffic. Same strict-submit/lenient-apply shape as ADR-0051, and the
    --     same lesson as ADR-0056 / ADR-0058 / issue #268. The rule is self-satisfying afterwards:
    --     once a peer's event has landed, a local write to that chart is no longer a FIRST event.
    --
    --     Scoped to the envelope's patient_id, not to any patient named in a payload (an
    --     identity.link's target chart may be remote-only and legitimately unregistered here).
    IF v_type <> 'identity.registration.asserted'
       AND NOT cairn_patient_has_events((b ->> 'patient_id')::uuid) THEN
        RAISE EXCEPTION 'submit_event: no chart exists for patient % — the first event on a chart must be its registration (identity.registration.asserted, §5.3/§5.8); register the patient before recording anything about them',
            b ->> 'patient_id';
    END IF;
```

- [ ] **Step 5: Add the SQL mirror**

Append to `db/tests/005_submit_test.sql`, following that file's existing `DO $$ ... EXCEPTION` idiom:
one case asserting the refusal message contains `must be its registration`, and one asserting a
registration itself is exempt (submitting one for a fresh patient does not raise).

- [ ] **Step 6: Run the full gate**

```bash
scripts/run-db-gated-tests.sh
```

Expected: fully green, including all five new tests. If a suite outside `patient_precedence.rs`
fails with `no chart exists`, Task 2's sweep missed it — fix it here rather than weakening the rule.

- [ ] **Step 7: Commit**

```bash
git add db/001_envelope.sql db/005_submit.sql db/tests/005_submit_test.sql \
        crates/cairn-node/tests/patient_precedence.rs
git commit -m "feat(#345): the first event on a chart must be its registration

Closes the search-before-create bypass at the strict local door. The remote
door stays lenient by design (ADR-0061 decision 3): set-union sync has no
ordering, so a peer's clinical event legitimately precedes the registration
licensing it, and a fail-closed remote door would wedge replication."
```

---

## Task 4: Retire `patient.created`

**Files:**
- Create: `db/047_registration_precedence.sql`
- Modify: `db/002_projection.sql`, `db/005_submit.sql`, `db/008_surrogate_projection.sql`,
  `db/tests/008_surrogate_test.sql`, `db/bench/b5_surrogate.sql`
- Modify: `crates/cairn-event/src/schema_generation.rs`, `crates/cairn-node/src/db.rs`,
  `crates/cairn-sync/src/main.rs`, `crates/cairn-event/src/lib.rs`,
  `crates/cairn-event/tests/clock_grade.rs`
- Modify: the test files that used `patient.created` as a projection vehicle
  (`projection_registry.rs`, `deferred_admission.rs`, `overlay_tiebreaker.rs`,
  `apply_remote_event.rs`, `demographics*.rs`, `link_veto_floor.rs`, `identity_*.rs`,
  `match_veto.rs`, `patient_search.rs`, `auto_apply.rs`)

**Interfaces:**
- Produces: `patient_chart` rows are created by `identity.registration.asserted` (chart birth);
  `patient.amended` keeps the demographic overlay branch; `note.added` keeps the counter branch.
- Removes: `patient.created` from `event_type_class` and from `cairn_projection_apply`.

- [ ] **Step 1: Write the failing tests**

Add to `crates/cairn-node/tests/patient_precedence.rs`:

```rust
/// Registration takes over the chart-birth projection `patient.created` used to own: a registered
/// chart HAS a patient_chart row, so every read composed on it (search's last-activity, the
/// person_chart trust reads) sees the chart from its birth.
#[tokio::test]
async fn registration_materialises_the_chart_row() {
    let Some(base) = cs() else { return };
    let _g = db::test_serial_guard(&base).await;
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &["patient_registration"]).await;
    let p = Uuid::now_v7();

    submit_registration(&c, &sk, &kid, p, WALL).await;

    let row = c
        .query_one(
            "SELECT name, note_count, last_activity IS NOT NULL \
               FROM patient_chart WHERE patient_id = $1::text::uuid",
            &[&p.to_string()],
        )
        .await
        .expect("a registered chart has a patient_chart row");
    let name: Option<String> = row.get(0);
    assert_eq!(name, None, "registration asserts no demographics — the name comes from db/010-014");
    assert_eq!(row.get::<_, i32>(1), 0, "a fresh chart has no notes");
    assert!(row.get::<_, bool>(2), "the birth act is activity");
}

/// `patient.created` is RETIRED: the strict door refuses it as an unknown type. Grandfathering it
/// as a permitted first event would put back exactly the "unless" the one-act-three-classes design
/// removes (ADR-0061 decision 2).
#[tokio::test]
async fn the_legacy_patient_created_type_is_retired() {
    let Some(base) = cs() else { return };
    let _g = db::test_serial_guard(&base).await;
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &["patient_registration"]).await;
    let p = Uuid::now_v7();

    let err = submit_signed(
        &c,
        &sk,
        &kid,
        EventSpec {
            patient: p,
            event_type: "patient.created",
            schema_version: "patient/1",
            payload: serde_json::json!({"name": "T", "dob": "1990", "sex": "x"}),
            plaintext_twin: None,
            wall: WALL,
        },
    )
    .await
    .expect_err("the retired type must be refused");

    assert!(
        db_msg(&err).contains("unknown event_type"),
        "retirement is expressed as declassification, so the door's own fail-closed arm refuses it: {}",
        db_msg(&err)
    );

    let rows: i64 = c
        .query_one(
            "SELECT count(*) FROM cairn_projection_apply WHERE event_type = 'patient.created'",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(rows, 0, "the projection registration must be dropped BEFORE the class row (db/005:153)");
}
```

- [ ] **Step 2: Run and watch them fail**

```bash
scripts/run-db-gated-tests.sh
```

Expected: both new tests FAIL — no `patient_chart` row for a registration, and `patient.created`
still submits.

- [ ] **Step 3: Give `patient_chart_apply` its chart-birth branch (db/002)**

Add to `patient_chart_apply`, after the `note.added` branch:

```sql
    ELSIF e.event_type = 'identity.registration.asserted' THEN
        -- The chart's BIRTH (§5.3/§5.8, issue #345). Registration took over this projection from
        -- the retired patient.created, but it is deliberately NOT the same write: a registration
        -- asserts no demographics, so it materialises the row and nothing else. Name/dob/sex come
        -- from the demographic streams (db/010-014) that supersede patient.created's payload; the
        -- demo_* provenance columns stay NULL, which the overlay predicate reads as "no winner
        -- yet" (COALESCE(-1)), so the first real demographic event still wins cleanly.
        --
        -- last_activity: the birth act IS activity. It is what a candidate list shows as "last
        -- seen" for a chart created moments ago and not yet written to (patient/search.rs).
        INSERT INTO patient_chart AS pc (patient_id, last_activity, updated_at)
        VALUES (e.patient_id, e.recorded_at, clock_timestamp())
        ON CONFLICT (patient_id) DO UPDATE SET
            last_activity = GREATEST(pc.last_activity, e.recorded_at),
            updated_at    = clock_timestamp();
```

- [ ] **Step 4: Write the retirement migration (db/047)**

Create `db/047_registration_precedence.sql`:

```sql
-- Cairn — retire `patient.created`; registration takes over the chart-birth projection
-- (spec §5.3/§5.8, ADR-0061; issue #345).
--
-- # Why a type is being retired at all
--
-- `patient.created` is a walking-skeleton event type: classified `additive` in db/005,
-- projecting to `patient_chart` at run_order 10, payload `{name, dob, sex}` superseded by
-- demographics slices 1-5, with NO structural floor and no twin-check row. It is an
-- unfloored registration act. Leaving it classified while db/005 step 8b requires the first
-- event on a chart to be a registration would mean either (a) it stays a permitted first
-- event — reintroducing exactly the "unless" ADR-0061 decision 2 removes — or (b) it becomes
-- a type that can only ever be written second, which is no type at all.
--
-- # Order matters, and db/005 says so
--
-- db/005's projection-registry validation trigger records the invariant: a registered type
-- must be classified, checked at REGISTRATION time and therefore blind to a class row deleted
-- afterwards. So the projection rows go FIRST and the class row second. Reversed, this
-- migration would leave a registered-but-unclassified type — the state that trigger exists to
-- make unreachable.
--
-- # What happens to `patient.created` events that already exist
--
-- They stay in the log, exactly as principle 1 requires; nothing is rewritten. Their existing
-- patient_chart rows stay too (heal-mode reproject never truncates). A `reproject --rebuild`
-- would not re-derive them, because the type no longer resolves to an apply fn — acceptable
-- and deliberate on a pre-clinical project, and stated here so it is not discovered later.
--
-- A PEER still running older code may keep sending `patient.created`. The remote door admits
-- it UNINTERPRETED (ADR-0056): custody total, interpretation deferred, no projection, no
-- wedge. Retiring a type locally is not removing it from the wire.

BEGIN;

-- 1. Registration takes over the chart-birth projection.
--
--    Registered HERE rather than in db/045 because the apply fn (patient_chart_apply, db/002)
--    and the type's classification (db/045) must BOTH already exist when this row is
--    validated, and this file is the first point where that is true for a row this migration
--    owns. #214 DO UPDATE arm so a tampered row heals on replay; the IS DISTINCT FROM guard
--    keeps a converged replay write-free.
INSERT INTO cairn_projection_apply AS r (event_type, apply_fn, projection_tables, run_order, heal_safe)
VALUES ('identity.registration.asserted', 'patient_chart_apply', ARRAY['patient_chart'], 10, TRUE)
ON CONFLICT (event_type, apply_fn) DO UPDATE SET
    projection_tables = EXCLUDED.projection_tables,
    run_order         = EXCLUDED.run_order,
    heal_safe         = EXCLUDED.heal_safe
WHERE (r.projection_tables, r.run_order, r.heal_safe)
      IS DISTINCT FROM (EXCLUDED.projection_tables, EXCLUDED.run_order, EXCLUDED.heal_safe);

-- 2. Retire the type: projection registrations first, classification second (see the header).
--    Both are idempotent, and both are needed even though db/005 no longer SEEDS either row:
--    the loader replays every migration on every connect, but an INSERT ... ON CONFLICT DO
--    NOTHING can never REMOVE a row an older build already wrote. A database migrated in place
--    converges only because of these two DELETEs.
DELETE FROM cairn_projection_apply WHERE event_type = 'patient.created';
DELETE FROM event_type_class       WHERE event_type = 'patient.created';

COMMIT;
```

- [ ] **Step 5: Drop the seed rows and correct the stale note (db/005)**

- Remove `('patient.created', 'additive',    FALSE),` from the `event_type_class` INSERT (line ~19).
- Remove `('patient.created', 'patient_chart_apply', ARRAY['patient_chart'], 10, TRUE),` from the
  `cairn_projection_apply` INSERT (line ~1066).
- Correct the residual note at db/005:153-162, which currently asserts *"no migration ever DELETEs
  from event_type_class (every write is INSERT ... ON CONFLICT DO NOTHING)"*. Replace that clause
  with: *"db/047 (issue #345) is the first migration that DELETEs from event_type_class, retiring
  `patient.created`; it drops the projection registrations FIRST, which is what keeps this invariant
  true. Any future retirement must do the same."*

- [ ] **Step 6: Convert the spike rig and the benches**

- `db/008_surrogate_projection.sql`: `patient.created` → `patient.amended` in the
  `surrogate_project_apply` branch and in its `cairn_projection_apply` seed rows.
- `db/tests/008_surrogate_test.sql` and `db/bench/b5_surrogate.sql`: `_b5_seed_event(p,
  'patient.created', …)` → `'patient.amended'`.
- `crates/cairn-sync/src/main.rs`: the two bench emitters (~line 1187, ~line 1329) and the Byzantine
  collision test signer (~line 5278) → `patient.amended`, each with a one-line comment: *the bench
  measures the demographic-overlay path, which is unchanged by #345 — the type was retired, the
  branch was not, so prior Bet-B numbers stay comparable.*
- `crates/cairn-event/src/lib.rs` (the `event_type` doc comment + the serialization fixture) and
  `crates/cairn-event/tests/clock_grade.rs`: → `patient.amended`. These never submit — they are CBOR
  fixtures — but leaving a retired type as the canonical example is a trap for the next reader.

- [ ] **Step 7: Convert the test vehicles**

Every test that used `patient.created` to exercise something else — the projection dispatcher, the
overlay tiebreak, the deferred-admission simulation, remote apply — switches to `patient.amended`
(same payload, same classification, same projection branch) *after* registering the chart. Two
require thought rather than substitution:

1. `deferred_admission.rs`'s "deleted class row" simulation deletes `patient.created`'s rows to
   produce an unclassified type. Its victim becomes `patient.amended`; the comment explaining why
   BOTH rows are deleted stays, and gains the note that the restore-on-replay it relies on now comes
   from db/005's remaining seed rows.
2. `projection_registry.rs::dispatcher_routes_patient_created_to_patient_chart` is renamed to name
   what it now routes, and should assert the registration path too — it is the dispatcher test, and
   registration is now a two-apply-fn type (`patient_chart_apply` + `patient_registration_apply`),
   which nothing else covers.

- [ ] **Step 8: Wire the migration into both loaders**

- `crates/cairn-event/src/schema_generation.rs`: `SCHEMA_GENERATION` 46 → 47.
- `crates/cairn-node/src/db.rs`: append `("047_registration_precedence",
  include_str!("../../../db/047_registration_precedence.sql"))` with a comment naming #345.
- `crates/cairn-sync/src/main.rs`: **add BOTH `045_patient_registration` and
  `047_registration_precedence` to its SCHEMA subset.** This reverses the first reading of
  `db.rs`'s "cairn-sync legitimately lags on db/045" note, and the reason is a real dependency the
  rule creates: db/005 — which the subset DOES carry — now refuses any first event that is not a
  registration, and `identity.registration.asserted` is classified only by db/045. **A subset with
  db/005 but not db/045 is a door carrying a rule it cannot satisfy**: no chart could ever be
  authored on such a database, and `schema_subset_tests`' own local-door case (which submits a
  fresh patient's event to prove db/027 is present) proves it by failing. db/047 then follows
  db/045 because its projection registration is validated against the classification. The lag was
  honest while db/005 had no opinion about registration; it stopped being honest the moment the
  rule landed. Record that reasoning in `db.rs` beside the new entry
  ([#284](https://github.com/cairn-ehr/cairn-ehr/issues/284) is exactly this class of drift).

- [ ] **Step 9: Run the full gate**

```bash
scripts/run-db-gated-tests.sh
```

Expected: fully green. A `cairn_projection_apply: event_type ... is not classified` failure means
Step 8's ordering is wrong (db/045 must precede db/047 in the list).

- [ ] **Step 10: Commit**

```bash
git add db crates
git commit -m "feat(#345): retire patient.created; registration owns the chart birth

The walking-skeleton type was an unfloored registration act. Grandfathering it
as a permitted first event would reintroduce the 'unless' that ADR-0061
decision 2 removes. Projection registrations are dropped before the class row,
per db/005's own invariant. A peer still sending it is admitted uninterpreted
(ADR-0056) — retiring a type locally is not removing it from the wire."
```

---

## Task 5: Documentation currency

**Files:** `docs/spec/identity.md`, `db/045_patient_registration.sql`,
`docs/superpowers/specs/2026-08-04-search-before-create-funnel-design.md`,
`cairn-gui/cairn-gui-tauri/results/RUNBOOK.md`, `docs/HANDOVER.md`, `docs/ROADMAP.md`

- [ ] **Step 1: The spec's §5.8 warning becomes the enforced statement**

`docs/spec/identity.md` §5.8 item 1 carries a `> [!WARNING] **"Enforced" is the target, not yet the
state of the code.**` callout. Replace it with the enforced statement: the precedence rule is live at
the strict local door (db/005 step 8b, `cairn_patient_has_events`), `patient.created` is retired, and
the remote door stays lenient **by design** — with the ADR-0061 decision 3 reasoning kept, because a
future reader will otherwise "fix" the asymmetry. Check §5.3 for the same claim and correct it too.
**No spec version bump:** every bump so far has been ADR-paired, and this slice lands no ADR.

- [ ] **Step 2: Correct db/045's header**

Its "What is deliberately NOT here" section says the precedence rule "belongs to issue #345". Point
it at `db/047` instead, so a reader of the floor file can find the rule.

- [ ] **Step 3: Mark the design doc's §2.3 / §4.2 as shipped**

Both sections say the enforcement ships in a follow-on. Append a dated note that it shipped in #345,
naming db/047 and db/005 step 8b, and record the **measured** sweep size from Task 1 against the
~83/~38 estimate — the design doc has twice been corrected on this number, and a third data point is
worth more than a third estimate.

- [ ] **Step 4: Fix the RUNBOOK**

`cairn-gui/cairn-gui-tauri/results/RUNBOOK.md` §2 seeds a chart with `PATIENT=$(uuidgen)` followed by
`medication-assert`. That is now refused. Replace the `uuidgen` line with a real `patient-register`
call and capture the printed id, e.g.:

```bash
PATIENT=$($NODE patient-register --name "Bench Patient" --birth-date 1980-01-01 --confirm-new \
         | sed -n 's/^patient_id: //p')
```

Verify the actual output shape of `patient-register` before writing this line, and run the runbook's
first three commands to confirm. **If the verb's output cannot be captured this cleanly, that is a
finding** — file it rather than papering over it with a hand-copied UUID.

- [ ] **Step 5: Update HANDOVER + ROADMAP**

HANDOVER: ⇒ NEXT loses #345 and gains whatever the next candidate is; the Slice-64 paragraph records
the measured sweep size and the two decisions worth carrying (the placement argument; retirement
order). ROADMAP: Phase 4 records the funnel as unbypassable. Prune both toward 500 lines.

- [ ] **Step 6: Commit**

```bash
git add docs cairn-gui db/045_patient_registration.sql
git commit -m "docs(#345): the funnel is now unbypassable — spec, runbook, handover"
```

---

## Task 6: Review, then PR

- [ ] **Step 1: Whole-branch review**

Run `/code-review` at `high` over the branch diff. Fix every finding, or file an issue for what
cannot be fixed in place (house rule 5). Pay particular attention to the fixture sweep: the review
value of this slice is in whether each converted fixture still tests what it was written to test.

- [ ] **Step 2: Final gate**

```bash
scripts/run-db-gated-tests.sh
cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings
```

- [ ] **Step 3: Open the PR**

```bash
git push -u origin feat/close-funnel-bypass-345
gh pr create --title "Close the search-before-create bypass (#345)" --body "…Closes #345…"
```

---

## Self-Review

**1. Spec coverage.** The issue's four work items map to tasks: `cairn_patient_has_events` → Task 3
step 3; the db/005 refusal with db/020 untouched → Task 3 step 4 + its lenient test; retiring
`patient.created` incl. the `patient_chart` takeover → Task 4; the fixture sweep → Tasks 1-2. All
four tests the issue names are in Task 3 step 1 (refusal, success-after-registration, lenient remote
apply, unregistered chart still queryable).

**2. Placeholders.** Task 1 step 4 deliberately records a number that does not exist yet — that is
the task's deliverable, not a placeholder. Task 2 step 5 cannot enumerate ~38 files before Task 1
measures them; it instead carries the five judgement rules that govern each conversion, which is the
part a fresh implementer would otherwise get wrong.

**3. Type consistency.** `submit_registration(c, sk, kid, p, wall)` is used with that exact signature
in Tasks 2, 3 and 4. `cairn_patient_has_events(uuid) RETURNS boolean` is defined in Task 3 step 3 and
called in step 4. `REGISTRATION_EVENT_TYPE` / `REGISTRATION_SCHEMA_VERSION` /
`registration_assertion_body` / `render_registration_twin` / `RegistrationAssertion` /
`SearchAttestationInput` / `SearchTerms` / `RegistrationClass` are all existing exports of
`cairn_event::registration` (verified against the crate).

## Paper-parity benchmark (§1.2)

- **Paper counterpart:** starting a new paper chart. The clerk checks the index/shelf for an existing
  folder, then makes and labels a new one. You cannot write a note on a chart that does not exist —
  the folder is a physical precondition, and making it is a deliberate, attributable act. This slice
  restores that precondition; today the architecture lets a client write on a chart that was never
  made, which is a capability **paper never had**.
- **Steps:** paper **N = 2** human acts (search the index; make + label the folder). Architecture-
  forced **M = 1** (one registration event, which carries the search inside it — `patient-register`
  runs the search and the write in one command). UI bundling target **K = 1** gesture (type the name,
  read the candidate list, choose *none of these — create*). **M ≤ N**, so no architecture defect.
  For every workflow on an **existing** chart the delta is **zero** added steps: the rule is
  self-satisfying after the first event, and the chart-less case it refuses had no paper counterpart
  to lose.
- **Time + cognitive load:** ADR-0061's budget is unchanged and unmet-by-this-slice — ≤ 5 s to find
  an existing chart, ≤ 20 s to register a new one, owed by the first runnable registration surface
  (the interactive half) and by [#360](https://github.com/cairn-ehr/cairn-ehr/issues/360) (the
  node-tier write-cost half). This slice adds one indexed `EXISTS` per local submit; budget **< 1 ms
  at p95**, to be read off the same #360 measurement rather than measured separately. Cognitive load
  is *reduced*, not added: the refusal replaces a silent duplicate chart six months later with an
  immediate, legible instruction at the desk. **If either measurement falls outside its budget, that
  is the finding — file an issue; do not adjust the budget.**
