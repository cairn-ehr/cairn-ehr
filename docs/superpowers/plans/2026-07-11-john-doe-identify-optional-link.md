# §5.4 finisher 3 — `identify` → optional link — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add the node authoring path + CLI that resolves a John-Doe chart — a device-additive `identity.identify.asserted` (chart → *confirmed*) plus an optional human-attested `identity.link.asserted` joining a prior chart, atomic in one transaction.

**Architecture:** A new `cairn-node::identify` module (pure body builders + one async orchestrator) reuses the already-existing event type/floor/overlay (`db/024`), the already-existing event builders (`cairn-event::identity`), and the already-existing attested-link builder (`apply_proposal::build_attested_link_body`). A new `identify-patient` CLI subcommand drives it, loading a separate human `--attester-key` only for the optional link.

**Tech Stack:** Rust (tokio, tokio-postgres, clap), PostgreSQL ≥18 + `cairn_pgx`, `cairn-event` for serialization/signing/attestation.

## Global Constraints

- **No new event type / migration / floor change / SCHEMA / ADR / spec bump.** Additive Rust only.
- **AGPL-3.0**; no new dependencies (everything needed is already in the workspace).
- **TDD** — failing test first, then minimal code. Safety-critical tier (identity) → Rust, reviewer-legible.
- **Files under ~500 lines**; `identify.rs` split pure / IO like `john_doe.rs`.
- **Inline docs for a junior dev** on every non-trivial function — *why* it exists and how it fits.
- **All tests pass before any commit** (`cargo fmt` + `clippy -D warnings` clean).
- DB-gated tests need `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test"` (PG18 + `cairn_pgx`) and self-serialize via `db::test_serial_guard`.
- **Reuse verbatim, never re-serialize:** `cairn_event::sign`, `sign_attestation`, `event_address`; `apply_proposal::build_attested_link_body`; `cairn_event::identity::{IdentifyAssertion, identify_assertion_body, render_identify_twin}`.
- Canonical link pair is `(low, high) = (a.min(b), a.max(b))` (matches db/018 + `apply_proposal`).
- UUIDs bind as text: `.to_string()` + `$N::text::uuid` (this crate's `uuid` has no `ToSql`).

---

### Task 1: Pure identify body builder + link provenance

**Files:**
- Create: `crates/cairn-node/src/identify.rs`
- Modify: `crates/cairn-node/src/lib.rs:6` (add `pub mod identify;` in alphabetical order, after `pub mod fsio;`)
- Test: inline `#[cfg(test)]` module in `identify.rs`

**Interfaces:**
- Consumes: `cairn_event::identity::{IdentifyAssertion, identify_assertion_body, render_identify_twin}`, `cairn_event::{EventBody, Hlc}`, `uuid::Uuid`.
- Produces:
  - `pub fn build_identify_body(event_id: Uuid, patient: Uuid, method: &str, node_kid: &str, hlc: Hlc) -> EventBody`
  - `pub fn compose_identify_link_provenance(human_kid: &str) -> String`

- [ ] **Step 1: Add the module declaration**

In `crates/cairn-node/src/lib.rs`, add after line 6 (`pub mod fsio;`):

```rust
pub mod identify;
```

- [ ] **Step 2: Write the failing tests**

Create `crates/cairn-node/src/identify.rs` with the doc header, imports, and the test module (functions come in Step 4):

```rust
//! §5.4 finisher 3 — resolve a John-Doe chart: record WHO the patient is
//! (`identity.identify.asserted`, flipping the chart to *confirmed*) and, when that
//! person already has a prior chart, OPTIONALLY join the two (`identity.link.asserted`).
//!
//! Accountability (see the design doc): the identify is **device-additive** — authored
//! with the node key exactly like `register_john_doe`, no attestation. The optional link
//! MERGES a chart into a prior identity — a real human attribution — so it is signed AND
//! attested by a **human** key supplied at the CLI. This module owns the authoring seam;
//! it changes no floor and re-uses `apply_proposal::build_attested_link_body` verbatim.
//!
//! Split: pure body assembly (unit-tested, no DB) + one async orchestrator that authors
//! the identify and the optional link in ONE transaction (atomic — never confirmed-but-
//! half-linked when a link was intended).

use cairn_event::identity::{identify_assertion_body, render_identify_twin, IdentifyAssertion};
use cairn_event::{EventBody, Hlc};
use uuid::Uuid;

/// schema_version for the identify marker (mirrors `john_doe.rs`'s per-type constants).
const IDENTIFY_SCHEMA_VERSION: &str = "identity.identify.asserted/1";

#[cfg(test)]
mod tests {
    use super::*;

    fn ids() -> (Uuid, Uuid) {
        let eid = Uuid::parse_str("33333333-0000-0000-0000-000000000000").unwrap();
        let pid = Uuid::parse_str("00000000-0000-0000-0000-0000000000ab").unwrap();
        (eid, pid)
    }

    #[test]
    fn identify_body_is_device_additive_with_authored_twin() {
        let (eid, pid) = ids();
        let body = build_identify_body(
            eid,
            pid,
            "driver's licence + family confirmation",
            "nodekid",
            Hlc { wall: 7, counter: 0, node_origin: "n".into() },
        );
        assert_eq!(body.event_type, "identity.identify.asserted");
        assert_eq!(body.patient_id, pid.to_string());
        assert_eq!(body.payload["subject"], pid.to_string());
        assert_eq!(body.payload["method"], "driver's licence + family confirmation");
        assert!(body.payload.get("basis").is_none(), "an identify carries no basis");
        // Device-additive: a `recorded` contributor with NO responsibility marker, so the
        // db/005 attestation gate demands nothing (matches john_doe.rs's registration events).
        let c = &body.contributors[0];
        assert_eq!(c["role"], "recorded");
        assert!(c.get("responsibility").is_none(), "identify demands no attestation");
        // The db/024 floor HARD-requires a non-empty authored twin.
        assert!(!body.plaintext_twin.as_deref().unwrap().trim().is_empty());
    }

    #[test]
    fn link_provenance_is_nonempty_and_names_the_human() {
        let p = compose_identify_link_provenance("humankid");
        assert!(p.contains("humankid"));
        assert!(!p.trim().is_empty(), "the db/018 floor requires a non-empty provenance");
    }
}
```

- [ ] **Step 3: Run the tests to verify they fail**

Run: `cargo test -p cairn-node --lib identify:: 2>&1 | tail -20`
Expected: FAIL — `cannot find function build_identify_body` / `compose_identify_link_provenance`.

- [ ] **Step 4: Write the minimal implementation**

Insert, between the `IDENTIFY_SCHEMA_VERSION` const and the `#[cfg(test)]` module:

```rust
/// Assemble the device-additive `identity.identify.asserted` `EventBody`. Pure:
/// `event_id`, `hlc`, and the resolved strings are supplied so the body is fully
/// testable. The sole contributor is the registering node actor with role `recorded`
/// (it recorded the identification) — additive, so no attestation is demanded. `method`
/// is §5.7's "method recorded"; the db/024 floor rejects it empty.
pub fn build_identify_body(
    event_id: Uuid,
    patient: Uuid,
    method: &str,
    node_kid: &str,
    hlc: Hlc,
) -> EventBody {
    let pid = patient.to_string();
    let a = IdentifyAssertion { subject: &pid, method };
    EventBody {
        event_id: event_id.to_string(),
        patient_id: pid.clone(), // an identity-state assertion is "about" its subject's chart
        event_type: "identity.identify.asserted".into(),
        schema_version: IDENTIFY_SCHEMA_VERSION.into(),
        hlc,
        t_effective: None,
        signer_key_id: node_kid.into(),
        contributors: serde_json::json!([{"actor_id": node_kid, "role": "recorded"}]),
        payload: identify_assertion_body(&a),
        attachments: vec![],
        plaintext_twin: Some(render_identify_twin(&a)),
    }
}

/// Compose the §4.1 provenance for a link authored while resolving a John-Doe chart.
/// Non-empty by construction (the db/018 floor requires it) and legible: it records that
/// this link came from a John-Doe identification and names the vouching human.
pub fn compose_identify_link_provenance(human_kid: &str) -> String {
    format!("john-doe-identify linked-by:{human_kid}")
}
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p cairn-node --lib identify:: 2>&1 | tail -20`
Expected: PASS (2 tests).

- [ ] **Step 6: Commit**

```bash
git add crates/cairn-node/src/identify.rs crates/cairn-node/src/lib.rs
git commit -m "feat(identify): pure identify-body builder + link provenance (§5.4 finisher 3)"
```

---

### Task 2: `identify_patient` orchestrator — identify-only path

**Files:**
- Modify: `crates/cairn-node/src/identify.rs` (add types + orchestrator)
- Test: `crates/cairn-node/tests/identify.rs` (create)

**Interfaces:**
- Consumes: Task 1's `build_identify_body`; `cairn_event::{sign, SigningKey}`; `crate::db::next_hlc`; `apply_proposal::build_attested_link_body` (used in Task 3).
- Produces:
  - `pub struct LinkParams<'a> { pub prior: Uuid, pub human_sk: &'a SigningKey, pub human_kid: &'a str }`
  - `pub struct IdentifyOutcome { pub identify_event_id: Uuid, pub link_event_id: Option<Uuid> }`
  - `pub async fn identify_patient(client: &mut tokio_postgres::Client, node_sk: &SigningKey, node_kid: &str, node_origin: &str, patient: Uuid, method: &str, link: Option<LinkParams<'_>>) -> anyhow::Result<IdentifyOutcome>`

- [ ] **Step 1: Write the failing integration test**

Create `crates/cairn-node/tests/identify.rs`:

```rust
//! §5.4 finisher 3 — `identify` → optional link. DB-gated on $CAIRN_TEST_PG, serialized
//! cluster-wide via db::test_serial_guard (shared-DB + TRUNCATE pattern, like
//! attestation.rs / identity_linkage.rs). The human attester is enrolled via raw SQL here
//! (there is no enroll-human CLI yet — a separate future slice).
use cairn_event::{generate_key, SigningKey};
use cairn_node::db;
use cairn_node::identify::{identify_patient, IdentifyOutcome, LinkParams};
use tokio_postgres::Client;
use uuid::Uuid;

fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

/// Truncate the advisory-write tables and enroll the NODE key as a `device` registration
/// actor (so it may author the additive identify). Returns (node sk, node kid).
async fn setup_node(c: &Client) -> (SigningKey, String) {
    c.batch_execute("TRUNCATE event_log, actor_event, patient_chart CASCADE")
        .await
        .unwrap();
    c.batch_execute(
        "DO $$ BEGIN \
           IF to_regclass('public.patient_link')  IS NOT NULL THEN TRUNCATE patient_link;  END IF; \
           IF to_regclass('public.person_member') IS NOT NULL THEN TRUNCATE person_member; END IF; \
           IF to_regclass('public.chart_identity_state') IS NOT NULL THEN TRUNCATE chart_identity_state; END IF; \
         END $$;",
    )
    .await
    .unwrap();
    let (sk, kid) = generate_key().unwrap();
    c.execute(
        "SELECT enroll_actor('device', '{\"role\":\"registration-desk\"}', $1)",
        &[&kid],
    )
    .await
    .unwrap();
    (sk, kid)
}

/// Read the standing identity state for a subject ('pending' | 'identified' | None).
async fn identity_state(c: &Client, p: Uuid) -> Option<String> {
    c.query_opt(
        "SELECT state FROM chart_identity_state WHERE subject = $1::text::uuid",
        &[&p.to_string()],
    )
    .await
    .unwrap()
    .map(|r| r.get::<_, String>(0))
}

/// Read the effective trust_state for a chart, coalescing an absent row to 'confirmed'
/// (the chart_trust VIEW's default — a chart in the default state has no row).
async fn trust_state(c: &Client, p: Uuid) -> String {
    c.query_one(
        "SELECT COALESCE((SELECT trust_state FROM chart_trust WHERE patient_id = $1::text::uuid), 'confirmed')",
        &[&p.to_string()],
    )
    .await
    .unwrap()
    .get(0)
}

/// Author the pending marker for `patient` so the chart starts *unconfirmed* (reusing the
/// real register path would also mint a callsign; here we only need the pending state, so
/// author identify's counterpart directly through identify_patient's sibling is overkill —
/// instead drive the full flow: register a John Doe, then identify it).
async fn register_pending(c: &mut Client, sk: &SigningKey, kid: &str, node_origin: &str) -> Uuid {
    let (pid, _call, _ord) =
        cairn_node::john_doe::register_john_doe(c, sk, kid, node_origin, "ED", "site", "2026-07-11", "no ID")
            .await
            .unwrap();
    pid
}

#[tokio::test]
async fn identify_alone_flips_chart_to_confirmed() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let node_origin = "test-node";

    let pid = register_pending(&mut c, &sk, &kid, node_origin).await;
    assert_eq!(identity_state(&c, pid).await.as_deref(), Some("pending"));
    assert_eq!(trust_state(&c, pid).await, "unconfirmed");

    let out: IdentifyOutcome =
        identify_patient(&mut c, &sk, &kid, node_origin, pid, "driver's licence", None)
            .await
            .unwrap();
    assert!(out.link_event_id.is_none());
    assert_eq!(identity_state(&c, pid).await.as_deref(), Some("identified"));
    assert_eq!(trust_state(&c, pid).await, "confirmed");
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test -p cairn-node --test identify identify_alone 2>&1 | tail -25`
Expected: FAIL — `identify_patient`, `IdentifyOutcome`, `LinkParams` are unresolved.

- [ ] **Step 3: Write the minimal implementation**

In `crates/cairn-node/src/identify.rs`, extend the imports and add the types + orchestrator (identify-only; the `link` arm is filled in Task 3). Replace the import block with:

```rust
// `build_attested_link_body` lives in the sibling apply_proposal module; import it directly
// so the reuse is explicit and no link body is ever re-serialized here.
use crate::apply_proposal::build_attested_link_body;
use cairn_event::identity::{identify_assertion_body, render_identify_twin, IdentifyAssertion};
use cairn_event::{event_address, sign, sign_attestation, EventBody, Hlc, SigningKey};
use uuid::Uuid;
```

Then add, after `compose_identify_link_provenance`:

```rust
/// The optional prior-chart link inputs. Threaded explicitly (rather than read from
/// globals) so `identify_patient` stays a pure function of its arguments and is easy to
/// reason about. The link is signed AND attested by this human key.
pub struct LinkParams<'a> {
    pub prior: Uuid,
    pub human_sk: &'a SigningKey,
    pub human_kid: &'a str,
}

/// What `identify_patient` wrote: the identify event id, and the link event id when a link
/// was requested. Lets the CLI print an honest, specific confirmation.
pub struct IdentifyOutcome {
    pub identify_event_id: Uuid,
    pub link_event_id: Option<Uuid>,
}

/// Resolve a John-Doe chart: author the device-additive identify and, when `link` is given,
/// a human-attested link to a prior chart — in ONE transaction (atomic: never
/// confirmed-but-half-linked when a link was intended; if the link is refused, the identify
/// rolls back too and the chart stays *pending* to be retried).
///
/// HLC ticks (one per event) run before the transaction and self-commit; if the submit txn
/// rolls back the clock has merely advanced with no matching events, which is fine — the HLC
/// is monotonic and gaps are allowed (the same shape `register_john_doe` uses).
///
/// No cross-existence check on `patient`/`prior`: the offline-first floor (db/018/db/024)
/// does none — a pending marker or the prior chart may not have synced yet. The db/018 floor
/// rejects only a self-link (a == b) and an empty provenance.
pub async fn identify_patient(
    client: &mut tokio_postgres::Client,
    node_sk: &SigningKey,
    node_kid: &str,
    node_origin: &str,
    patient: Uuid,
    method: &str,
    link: Option<LinkParams<'_>>,
) -> anyhow::Result<IdentifyOutcome> {
    // 1. Build + sign the device-additive identify (node authors).
    let h1 = crate::db::next_hlc(client, node_origin).await?;
    let identify_event_id = Uuid::now_v7();
    let identify_body =
        build_identify_body(identify_event_id, patient, method, node_kid, h1);
    let identify_signed = sign(&identify_body, node_sk)?;

    // 2. If linking, build + sign + attest the human link BEFORE opening the txn.
    let link_prepared = match &link {
        Some(lp) => {
            let h2 = crate::db::next_hlc(client, node_origin).await?;
            let (low, high) = if patient <= lp.prior {
                (patient, lp.prior)
            } else {
                (lp.prior, patient)
            };
            let provenance = compose_identify_link_provenance(lp.human_kid);
            let link_event_id = Uuid::now_v7();
            // confidence: None — a human's direct assertion, not a matcher score.
            let link_body = build_attested_link_body(
                link_event_id, low, high, &provenance, None, lp.human_kid, h2,
            );
            let link_signed = sign(&link_body, lp.human_sk)?;
            let ca = event_address(&link_signed.signed_bytes);
            let token = sign_attestation(&ca, lp.human_kid, "attested", lp.human_sk)?;
            let attester_vk = lp.human_sk.verifying_key().to_bytes().to_vec();
            Some((link_event_id, link_signed.signed_bytes, token, attester_vk))
        }
        None => None,
    };

    // 3. One transaction: identify through the 1-arg door, link (if any) through the 3-arg
    //    door. Atomicity is the guarantee — a link rejection rolls the identify back too.
    let tx = client.transaction().await?;
    tx.execute("SELECT submit_event($1)", &[&identify_signed.signed_bytes])
        .await?;
    let link_event_id = match &link_prepared {
        Some((eid, bytes, token, vk)) => {
            tx.execute("SELECT submit_event($1,$2,$3)", &[bytes, token, vk])
                .await?;
            Some(*eid)
        }
        None => None,
    };
    tx.commit().await?;

    Ok(IdentifyOutcome { identify_event_id, link_event_id })
}
```

Delete the now-superseded plain import lines from Task 1 (the three `use cairn_event::...` / `use uuid::Uuid;` lines are replaced by the block above). Keep `IDENTIFY_SCHEMA_VERSION` and `build_identify_body`/`compose_identify_link_provenance` unchanged.

- [ ] **Step 4: Run the test to verify it passes**

Run: `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test -p cairn-node --test identify identify_alone 2>&1 | tail -25`
Expected: PASS (`identify_alone_flips_chart_to_confirmed`).

- [ ] **Step 5: Confirm the pure unit tests still pass and clippy is clean**

Run: `cargo test -p cairn-node --lib identify:: && cargo clippy -p cairn-node --all-targets 2>&1 | tail -5`
Expected: unit tests PASS; clippy clean (no warnings).

- [ ] **Step 6: Commit**

```bash
git add crates/cairn-node/src/identify.rs crates/cairn-node/tests/identify.rs
git commit -m "feat(identify): identify_patient orchestrator — identify-only path flips chart to confirmed"
```

---

### Task 3: The optional link path + atomicity

**Files:**
- Modify: `crates/cairn-node/tests/identify.rs` (add link + atomicity tests)
- (No `identify.rs` change — the link arm was written in Task 2; this task proves it.)

**Interfaces:**
- Consumes: Task 2's `identify_patient`, `LinkParams`; test helpers from Task 2.
- Produces: (tests only)

- [ ] **Step 1: Write the failing tests**

Append to `crates/cairn-node/tests/identify.rs`:

```rust
/// Enroll a second key as a `human` actor (the attester), via raw SQL — there is no
/// enroll-human CLI yet. Returns (human sk, human kid).
async fn enroll_human(c: &Client) -> (SigningKey, String) {
    let (sk, kid) = generate_key().unwrap();
    c.execute(
        "SELECT enroll_actor('human', '{\"role\":\"clinician\",\"handle\":\"dr-a\"}', $1)",
        &[&kid],
    )
    .await
    .unwrap();
    (sk, kid)
}

/// Enroll a second key as a NON-human `agent` actor (for the atomicity/rejection test).
async fn enroll_agent(c: &Client) -> (SigningKey, String) {
    let (sk, kid) = generate_key().unwrap();
    c.execute(
        "SELECT enroll_actor('agent', '{\"model\":\"m\",\"version\":\"1\",\"skill_epoch\":\"e\"}', $1)",
        &[&kid],
    )
    .await
    .unwrap();
    (sk, kid)
}

/// The person (connected-component) id for a chart, or None if it is in no link.
async fn person_of(c: &Client, p: Uuid) -> Option<Uuid> {
    c.query_opt(
        "SELECT person_id::text FROM person_member WHERE patient_id = $1::text::uuid",
        &[&p.to_string()],
    )
    .await
    .unwrap()
    .map(|r| r.get::<_, String>(0).parse().unwrap())
}

/// Count identify events on a subject (to prove atomicity rollback).
async fn identify_count(c: &Client, p: Uuid) -> i64 {
    c.query_one(
        "SELECT count(*) FROM event_log WHERE event_type = 'identity.identify.asserted' \
         AND body -> 'payload' ->> 'subject' = $1",
        &[&p.to_string()],
    )
    .await
    .unwrap()
    .get(0)
}

#[tokio::test]
async fn identify_with_link_joins_prior_chart_and_confirms() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let (h_sk, h_kid) = enroll_human(&c).await;
    let node_origin = "test-node";

    let doe = register_pending(&mut c, &sk, &kid, node_origin).await;
    let prior = Uuid::now_v7(); // a prior chart (need not pre-exist; offline-first)

    let out = identify_patient(
        &mut c, &sk, &kid, node_origin, doe, "family confirmation",
        Some(LinkParams { prior, human_sk: &h_sk, human_kid: &h_kid }),
    )
    .await
    .unwrap();

    assert!(out.link_event_id.is_some(), "a link was requested");
    assert_eq!(identity_state(&c, doe).await.as_deref(), Some("identified"));
    assert_eq!(trust_state(&c, doe).await, "confirmed");
    // Both charts now sit in ONE person component (min-uuid canonical person).
    let expected = doe.min(prior);
    assert_eq!(person_of(&c, doe).await, Some(expected));
    assert_eq!(person_of(&c, prior).await, Some(expected));
}

#[tokio::test]
async fn link_with_non_human_attester_rolls_back_the_whole_op() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let (a_sk, a_kid) = enroll_agent(&c).await; // NOT a human → attestation gate refuses
    let node_origin = "test-node";

    let doe = register_pending(&mut c, &sk, &kid, node_origin).await;
    let prior = Uuid::now_v7();

    let r = identify_patient(
        &mut c, &sk, &kid, node_origin, doe, "family confirmation",
        Some(LinkParams { prior, human_sk: &a_sk, human_kid: &a_kid }),
    )
    .await;

    assert!(r.is_err(), "a non-human attester must be refused by the floor");
    // Atomicity: the identify must NOT have committed — the chart stays *pending*.
    assert_eq!(identity_state(&c, doe).await.as_deref(), Some("pending"));
    assert_eq!(identify_count(&c, doe).await, 0, "no identify event may survive the rollback");
    assert_eq!(trust_state(&c, doe).await, "unconfirmed");
}
```

- [ ] **Step 2: Run the tests to verify they pass** (the implementation already exists from Task 2)

Run: `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test -p cairn-node --test identify 2>&1 | tail -25`
Expected: PASS — all four tests (`identify_alone_flips…`, `identify_with_link_joins…`, `link_with_non_human_attester_rolls_back…`).

> If `identify_with_link…` fails because the transaction handle is left in a poisoned state after a mid-txn error in the atomicity test, note: each test opens its own `connect_and_load_schema` client, so there is no cross-test contamination. A single-test failure here is a real defect in the Task 2 orchestrator — debug it (superpowers:systematic-debugging), do not paper over it.

- [ ] **Step 3: Commit**

```bash
git add crates/cairn-node/tests/identify.rs
git commit -m "test(identify): optional-link joins prior chart; non-human attester rolls back atomically"
```

---

### Task 4: Human-ness pre-check helper

**Files:**
- Modify: `crates/cairn-node/src/identify.rs` (add `attester_is_enrolled_human`)
- Test: `crates/cairn-node/tests/identify.rs` (add a DB-gated test)

**Interfaces:**
- Produces: `pub async fn attester_is_enrolled_human(client: &tokio_postgres::Client, attester_kid: &str) -> anyhow::Result<bool>`

- [ ] **Step 1: Write the failing test**

Append to `crates/cairn-node/tests/identify.rs`:

```rust
use cairn_node::identify::attester_is_enrolled_human;

#[tokio::test]
async fn human_precheck_distinguishes_human_from_device_and_unenrolled() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (_sk, device_kid) = setup_node(&c).await; // enrolled as `device`
    let (_h_sk, human_kid) = enroll_human(&c).await;
    let (_u_sk, unenrolled_kid) = generate_key().unwrap();

    assert!(attester_is_enrolled_human(&c, &human_kid).await.unwrap());
    assert!(!attester_is_enrolled_human(&c, &device_kid).await.unwrap());
    assert!(!attester_is_enrolled_human(&c, &unenrolled_kid).await.unwrap());
}
```

Move the `use cairn_node::identify::attester_is_enrolled_human;` line up to the other `use` lines at the top of the test file if the compiler flags the mid-file `use` (Rust allows it, but keep imports grouped for cleanliness).

- [ ] **Step 2: Run the test to verify it fails**

Run: `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test -p cairn-node --test identify human_precheck 2>&1 | tail -20`
Expected: FAIL — `attester_is_enrolled_human` unresolved.

- [ ] **Step 3: Write the minimal implementation**

In `crates/cairn-node/src/identify.rs`, add after `identify_patient`:

```rust
/// Advisory CLI pre-check: does `attester_kid` resolve to a `kind='human'` actor? Human-ness
/// is read from the append-only `actor_event` HISTORY (per ADR-0043), so a human whose key was
/// later rotated/revoked still counts as ever-enrolled-human — the same source the floor uses.
///
/// This is a LEGIBILITY aid only: it lets the CLI reject a wrong `--attester-key` with a clear
/// message BEFORE authoring anything. The real, unbypassable enforcement is the db/005
/// attestation gate inside `submit_event` (defense in depth); never rely on this check for
/// safety — a raw-SQL client that skips it still cannot attest a link with a non-human key.
pub async fn attester_is_enrolled_human(
    client: &tokio_postgres::Client,
    attester_kid: &str,
) -> anyhow::Result<bool> {
    let ok: bool = client
        .query_one(
            "SELECT EXISTS(SELECT 1 FROM actor_event \
             WHERE signing_key_id = $1 AND kind = 'human')",
            &[&attester_kid],
        )
        .await?
        .get(0);
    Ok(ok)
}
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test -p cairn-node --test identify human_precheck 2>&1 | tail -20`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/cairn-node/src/identify.rs crates/cairn-node/tests/identify.rs
git commit -m "feat(identify): advisory human-ness pre-check for the attester key"
```

---

### Task 5: The `identify-patient` CLI subcommand

**Files:**
- Modify: `crates/cairn-node/src/main.rs` (add `Cmd::IdentifyPatient` variant + its match arm)

**Interfaces:**
- Consumes: Task 2–4's `identify::{identify_patient, LinkParams, attester_is_enrolled_human}`; existing `ensure_registration_actor`, `load_signing_key`, `resolve_passphrase`, `prompt_passphrase`, `cairn_node::keystore`.
- Produces: the `identify-patient` CLI verb.

- [ ] **Step 1: Add the `Cmd` variant**

In `crates/cairn-node/src/main.rs`, inside `enum Cmd { … }`, add after the `AssertIdentityEvidence { … }` variant (before the closing `}` at line ~386):

```rust
    /// Resolve a John-Doe chart (§5.4 finisher 3): record WHO the patient is
    /// (`identity.identify.asserted`, flipping the chart to *confirmed*) and OPTIONALLY
    /// link it to a prior chart so their history joins. The identify is device-additive
    /// (node key). The link MERGES charts — a human attribution — so it requires a
    /// separate human `--attester-key` that signs+attests it; identify + link are atomic.
    IdentifyPatient {
        /// The John-Doe patient UUID being identified.
        patient: Uuid,
        /// §5.7 "method recorded": how identity was established (non-empty).
        #[arg(long)]
        method: String,
        /// Optional prior chart UUID to link this now-identified chart to.
        #[arg(long)]
        link: Option<Uuid>,
        /// Human signing key that vouches for the link. Required when --link is given.
        #[arg(long)]
        attester_key: Option<PathBuf>,
        /// Passphrase to unseal --attester-key (else CAIRN_ATTESTER_PASSPHRASE, else prompt).
        #[arg(long, env = "CAIRN_ATTESTER_PASSPHRASE")]
        attester_passphrase: Option<String>,
    },
```

- [ ] **Step 2: Add the match arm**

In the `match cli.cmd { … }` block, add after the `Cmd::AssertIdentityEvidence { … } => { … }` arm (before the closing `}` of the match at line ~1108):

```rust
        Cmd::IdentifyPatient {
            patient,
            method,
            link,
            attester_key,
            attester_passphrase,
        } => {
            // Cross-flag validation (clap cannot express "attester-key iff link"). Reject
            // both mismatches loudly — an attester with nothing to attest is a mistake worth
            // surfacing, not silently ignoring.
            match (&link, &attester_key) {
                (Some(_), None) => anyhow::bail!(
                    "--link requires --attester-key: linking to a prior chart is a human \
                     attribution that must be attested"
                ),
                (None, Some(_)) => anyhow::bail!(
                    "--attester-key was given without --link: nothing to attest"
                ),
                _ => {}
            }

            let node_sk = load_signing_key(&cli.key, true)?; // may prompt to unseal
            let node_kid = hex::encode(node_sk.verifying_key().to_bytes());
            let mut db = cairn_node::db::connect(&cli.conn).await?;
            let id = cairn_node::identity::load_local(&db).await?;
            // Owner ceremony: the node key must be an enrolled actor to author the additive
            // identify (idempotent — enrolls a `device` actor only on first use).
            ensure_registration_actor(&db, &node_kid).await?;

            // Load the human attester key + pre-check human-ness (legibility; the db/005 gate
            // is the real enforcement). Held so the borrows live across identify_patient.
            let attester = match (&link, &attester_key) {
                (Some(_), Some(path)) => {
                    let sk = load_attester_key(path, attester_passphrase)?;
                    let kid = hex::encode(sk.verifying_key().to_bytes());
                    if !cairn_node::identify::attester_is_enrolled_human(&db, &kid).await? {
                        anyhow::bail!(
                            "--attester-key ({kid}) is not an enrolled human actor; a link \
                             must be attested by a human (enroll the clinician first)"
                        );
                    }
                    Some((sk, kid))
                }
                _ => None,
            };
            let link_params = match (&link, &attester) {
                (Some(prior), Some((sk, kid))) => Some(cairn_node::identify::LinkParams {
                    prior: *prior,
                    human_sk: sk,
                    human_kid: kid,
                }),
                _ => None,
            };

            let out = cairn_node::identify::identify_patient(
                &mut db,
                &node_sk,
                &node_kid,
                &id.node_id_hex,
                patient,
                &method,
                link_params,
            )
            .await?;
            println!("identified {patient} (chart now confirmed); event {}", out.identify_event_id);
            if let (Some(prior), Some(link_eid)) = (link, out.link_event_id) {
                println!("linked to {prior}; link event {link_eid}");
            }
        }
```

- [ ] **Step 3: Add the `load_attester_key` helper**

In `crates/cairn-node/src/main.rs`, add next to `load_signing_key` (after its definition, ~line 80):

```rust
/// Load the human attester key for `identify-patient --link`. Mirrors `load_signing_key`
/// but keyed on the SEPARATE attester passphrase (flag / CAIRN_ATTESTER_PASSPHRASE / prompt)
/// so the attester key is distinct from the node's own operational key.
fn load_attester_key(
    path: &std::path::Path,
    passphrase: Option<String>,
) -> anyhow::Result<cairn_event::SigningKey> {
    use cairn_node::keystore::{load, KeystoreError};
    // Hold the secret in Zeroizing so it is wiped on drop (issue #46).
    let secret = passphrase.filter(|s| !s.is_empty()).map(Zeroizing::new);
    match load(path, secret.as_ref().map(|s| s.as_str())) {
        Ok(sk) => Ok(sk),
        Err(KeystoreError::Sealed) => {
            let p = prompt_passphrase()?;
            Ok(load(path, Some(p.as_str()))?)
        }
        Err(e) => Err(e.into()),
    }
}
```

- [ ] **Step 4: Build + confirm the CLI compiles and existing tests still pass**

Run: `cargo build -p cairn-node 2>&1 | tail -15 && cargo test -p cairn-node --lib 2>&1 | tail -10`
Expected: builds clean; lib unit tests PASS.

- [ ] **Step 5: Manually smoke-test the flag validation (no DB needed for the guard path)**

Run: `cargo run -p cairn-node -- identify-patient 11111111-1111-1111-1111-111111111111 --method x --link 22222222-2222-2222-2222-222222222222 2>&1 | tail -5`
Expected: exits non-zero with `--link requires --attester-key: …` (the validation fires before any DB/key work — note it may first try to load the key; if so, run with a throwaway `--key` path to reach the guard, or accept that the guard is covered by reading the arm. The load order in Step 2 puts validation FIRST, so it fires before key load).

- [ ] **Step 6: Commit**

```bash
git add crates/cairn-node/src/main.rs
git commit -m "feat(cli): identify-patient — device-additive identify + optional human-attested link (§5.4 finisher 3)"
```

---

### Task 6: Full-workspace verification + gates

**Files:** none (verification only)

- [ ] **Step 1: Format + lint the whole workspace**

Run: `cargo fmt --all && cargo clippy --workspace --all-targets 2>&1 | tail -10`
Expected: no diff after fmt is acceptable to commit; clippy clean (`-D warnings` parity).

- [ ] **Step 2: Run the full DB-gated workspace test suite**

Run: `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test --workspace 2>&1 | tail -30`
Expected: all pass — cairn-node (DB-gated incl. the new `identify` suite), cairn-event, cairn-sync. No failures, no ignored-that-should-run.

- [ ] **Step 3: Confirm the docs site still builds**

Run: `uv run --with-requirements docs/requirements.txt -- mkdocs build 2>&1 | tail -5`
Expected: builds clean (no new doc content, but keep the gate green).

- [ ] **Step 4: Commit any fmt-only changes**

```bash
git add -A
git commit -m "chore(identify): fmt + workspace green for §5.4 finisher 3" --allow-empty
```

---

## Self-Review

**Spec coverage:**
- CLI `identify-patient` with `--method`/`--link`/`--attester-key`/`--attester-passphrase` → Task 5. ✓
- Device-additive identify (node key, `ensure_registration_actor`) → Tasks 1, 2, 5. ✓
- Human-attested optional link, reusing `build_attested_link_body`, `confidence: None` → Tasks 2, 3. ✓
- Atomic identify+link in one txn → Task 2 (impl), Task 3 (proof). ✓
- Human enrollment as precondition, never auto-created; legible pre-check + floor enforcement → Tasks 4, 5. ✓
- No cross-existence pre-check → encoded in Task 2 doc + Task 3 test (prior need not pre-exist). ✓
- `--attester-key` required iff `--link` → Task 5 Step 2 validation. ✓
- Testing: identify flips chart; link joins; atomicity rollback; pre-check → Tasks 2–4. ✓
- No new event type/migration/floor/SCHEMA/ADR/spec → held throughout (additive Rust only). ✓

**Placeholder scan:** none — every code step shows full code; every run step shows the command + expected result.

**Type consistency:** `identify_patient` / `IdentifyOutcome` / `LinkParams` / `attester_is_enrolled_human` names match across Tasks 2–5; `build_attested_link_body(event_id, low, high, provenance, confidence, human_kid, hlc)` matches `apply_proposal.rs`; `sign`/`sign_attestation`/`event_address`/`next_hlc` signatures verified against source.

## Deferred (recorded, not built)
- `reattribute` of clinical documentation (§5.5) — needs a clinical-note surface.
- `enroll-human` ceremony CLI — its own slice (ADR-0044 person-distinguishing determinant).
- "Prior history now available" push-alert on link (§5.12) — no notification tier yet.
