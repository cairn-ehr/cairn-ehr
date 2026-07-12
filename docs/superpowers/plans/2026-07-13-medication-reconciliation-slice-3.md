# Medication Reconciliation Resolution (slice 3) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make "these two medication threads are the same real drug" a first-class, symmetric, reversible *link* between `medication_id` threads (never a false cessation), collapsing a reconciled group to one row in the current-medication list.

**Architecture:** Two additive verbs (`clinical.medication-reconciliation.asserted` / `clinical.medication-separation.asserted`) over a canonical `(low, high)` thread pair, folded by an HLC-overlay edge table + a connected-component grouping projection (min-UUID canonical) that mirrors the identity `patient_link` → `person_member` machinery verbatim. Collapsed current/past views derive group status by *latest-effective wins* and group dose by the slice-2 bitemporal rule. Pure Rust builders + an in-DB floor + projection (safety-critical write path, §9).

**Tech Stack:** Rust (`cairn-event` pure builders, `cairn-node` async orchestrators + clap CLI), PostgreSQL ≥ 18 + `cairn_pgx` (PL/pgSQL floor + trigger projections), `tokio-postgres`.

Design spec: [docs/superpowers/specs/2026-07-13-medication-reconciliation-resolution-design.md](../specs/2026-07-13-medication-reconciliation-resolution-design.md).

## Global Constraints

- **AGPL-3.0**; every dependency AGPL-compatible (no new deps in this slice).
- **Safety-critical substrate (§9):** the write path is pure Rust + in-DB floor + projection.
- **TDD:** failing test first, then minimal code. No production code without a test that drove it.
- **Device-additive throughout:** every event signs with the node key, contributor role `"recorded"`, no responsibility attestation (mirrors slices 1/2 / `identify.rs`).
- **Offline-first:** NO local-existence check on either subject thread (subjects/threads may replicate later). `submit_event` is the 1-arg door.
- **Append-only / additive:** both verbs register `mode='additive'`, `targets_other_author=FALSE`. A reconciliation forecloses nothing.
- **Collation-independent tiebreaks (ADR-0045):** every projection winner/ordering tiebreak over a TEXT key uses `COLLATE "C"`.
- **Migration-replay safety:** `connect_and_load_schema` re-runs ALL `db/*.sql` in order every connect. A `CREATE OR REPLACE VIEW` that a later migration widens/narrows breaks reconnect ("cannot drop columns from view"). The reworked `patient_medication_current`/`_past` MUST keep byte-identical column sets to db/032; the reworked flag view (which renames a column) uses `DROP VIEW IF EXISTS` + `CREATE VIEW` (replay-safe because db/033 always loads after db/031).
- **Never hard-code crypto material in tests (house rule 6):** key material via `generate_key()`, never literals.
- **File-size guideline:** keep files < 500 lines; `cairn-event/src/medication/` is already a module (`assert`/`cessation`/`dose`) — add `reconciliation.rs` there.
- **DB-gated test conn string:** `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test"` (PG18 + cairn_pgx). Tests early-return `Ok(())` when the env var is absent, and self-serialize via `db::test_serial_guard`.
- **Advisory-lock key** for the group recompute race: `x'4341524E4D52'::bigint` (`'CARNMR'`), distinct from db/018's `CARNLK` and the test-serialization guard key.
- **Spec version bump:** v0.47 → v0.48 (`docs/spec/index.md` only). New **ADR-0047** is immutable once committed.

---

## Task 1: Pure event builders + twins (`cairn-event`)

**Files:**
- Create: `crates/cairn-event/src/medication/reconciliation.rs`
- Modify: `crates/cairn-event/src/medication/mod.rs`
- Test: inline `#[cfg(test)]` in `reconciliation.rs`

**Interfaces:**
- Produces:
  - `struct ReconciliationAssertion<'a> { subject_a: &'a str, subject_b: &'a str, provenance: &'a str, reason: Option<&'a str> }`
  - `fn reconciliation_body(a: &ReconciliationAssertion) -> serde_json::Value`
  - `fn render_reconciliation_twin(a: &ReconciliationAssertion) -> String`
  - `fn render_separation_twin(a: &ReconciliationAssertion) -> String`
  - (the payload is identical for both verbs — event_type distinguishes them, mirroring `identity.rs` link/unlink)

- [ ] **Step 1: Write the failing tests**

Add to a new file `crates/cairn-event/src/medication/reconciliation.rs`:

```rust
//! Medication reconciliation / separation builders (slice 3). Pure: shapes only
//! payload JSON, mirroring `identity.rs` link/unlink. Two immortal `medication_id`
//! threads are asserted to be (reconciliation) or to NOT be (separation) the same
//! real drug; the event_type — not the payload — carries the direction. The
//! in-DB floor (db/033) rejects a self-reconcile (a == b) and an empty provenance.
use serde_json::{json, Value};

/// One reconciliation/separation assertion between two immortal `medication_id`
/// threads. `reason` is omitted entirely when absent (never serialized as null),
/// so the floor's key-presence checks see exactly what the author asserted.
pub struct ReconciliationAssertion<'a> {
    pub subject_a: &'a str,  // a medication_id thread (string uuid)
    pub subject_b: &'a str,  // the other medication_id thread, distinct from subject_a
    pub provenance: &'a str, // §4.1 ladder — required-present, value-open ("clinician-judgment")
    pub reason: Option<&'a str>, // free-text ("brand vs generic"); omitted when None
}

/// Shared payload shape for reconciliation and separation (identical; the
/// event_type distinguishes them).
fn assertion_body(a: &ReconciliationAssertion) -> Value {
    let mut p = json!({
        "subject_a": a.subject_a,
        "subject_b": a.subject_b,
        "provenance": a.provenance,
    });
    if let Some(r) = a.reason {
        p.as_object_mut()
            .expect("json! built an object")
            .insert("reason".into(), json!(r));
    }
    p
}

/// Build the `clinical.medication-reconciliation.asserted` payload.
pub fn reconciliation_body(a: &ReconciliationAssertion) -> Value {
    assertion_body(a)
}

/// Build the `clinical.medication-separation.asserted` payload — same shape.
pub fn separation_body(a: &ReconciliationAssertion) -> Value {
    assertion_body(a)
}

/// The §3.13 legibility twin for a reconciliation. Always non-empty.
pub fn render_reconciliation_twin(a: &ReconciliationAssertion) -> String {
    let mut s = format!(
        "Reconciled as the same medication: {} ↔ {} ({})",
        a.subject_a, a.subject_b, a.provenance
    );
    if let Some(r) = a.reason {
        s.push_str(&format!(" — {r}"));
    }
    s
}

/// The §3.13 legibility twin for a separation (the never-erase reversal).
pub fn render_separation_twin(a: &ReconciliationAssertion) -> String {
    let mut s = format!(
        "Separated as distinct medications: {} ↔ {} ({})",
        a.subject_a, a.subject_b, a.provenance
    );
    if let Some(r) = a.reason {
        s.push_str(&format!(" — {r}"));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> ReconciliationAssertion<'static> {
        ReconciliationAssertion {
            subject_a: "11111111-1111-7111-8111-111111111111",
            subject_b: "22222222-2222-7222-8222-222222222222",
            provenance: "clinician-judgment",
            reason: Some("brand vs generic"),
        }
    }

    #[test]
    fn body_carries_subjects_provenance_reason() {
        let v = reconciliation_body(&sample());
        assert_eq!(v["subject_a"], "11111111-1111-7111-8111-111111111111");
        assert_eq!(v["subject_b"], "22222222-2222-7222-8222-222222222222");
        assert_eq!(v["provenance"], "clinician-judgment");
        assert_eq!(v["reason"], "brand vs generic");
    }

    #[test]
    fn reason_omitted_when_absent_not_null() {
        let a = ReconciliationAssertion { reason: None, ..sample() };
        let v = separation_body(&a);
        assert!(
            !v.as_object().unwrap().contains_key("reason"),
            "absent reason omitted entirely, not null"
        );
        assert_eq!(v["subject_a"], "11111111-1111-7111-8111-111111111111");
    }

    #[test]
    fn separation_payload_matches_reconciliation_shape() {
        // The two verbs carry an identical body; only event_type differs.
        assert_eq!(reconciliation_body(&sample()), separation_body(&sample()));
    }

    #[test]
    fn twins_are_nonempty_and_read_naturally() {
        let r = render_reconciliation_twin(&sample());
        assert!(r.contains("Reconciled"));
        assert!(r.contains("brand vs generic"));
        assert!(!r.trim().is_empty());
        let s = render_separation_twin(&sample());
        assert!(s.contains("Separated"));
        assert!(!s.trim().is_empty());
        // non-empty even with no reason
        let bare = ReconciliationAssertion { reason: None, ..sample() };
        assert!(!render_reconciliation_twin(&bare).trim().is_empty());
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p cairn-event reconciliation`
Expected: FAIL — module `reconciliation` not found / unresolved import (the file isn't wired into `mod.rs` yet).

- [ ] **Step 3: Wire the module into `mod.rs`**

Modify `crates/cairn-event/src/medication/mod.rs` — add the module and re-exports:

```rust
pub mod assert;
pub mod cessation;
pub mod dose;
pub mod reconciliation;

pub use assert::{medication_assertion_body, render_medication_twin, MedicationAssertion};
pub use cessation::{
    medication_cessation_body, render_medication_cessation_twin, MedicationCessation,
};
pub use dose::{
    dose_change_body, dose_correction_body, render_dose_change_twin, render_dose_correction_twin,
    DoseChange, DoseCorrection,
};
pub use reconciliation::{
    reconciliation_body, render_reconciliation_twin, render_separation_twin, separation_body,
    ReconciliationAssertion,
};
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p cairn-event reconciliation`
Expected: PASS (4 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/cairn-event/src/medication/reconciliation.rs crates/cairn-event/src/medication/mod.rs
git commit -m "feat(cairn-event): medication reconciliation/separation payload builders + twins (slice 3)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 2: Node body builders + async orchestrators (`cairn-node`)

**Files:**
- Modify: `crates/cairn-node/src/medication.rs` (append reconciliation section)
- Test: inline `#[cfg(test)]` in `medication.rs` (pure body-builder tests)

**Interfaces:**
- Consumes: `cairn_event::medication::{ReconciliationAssertion, reconciliation_body, separation_body, render_reconciliation_twin, render_separation_twin}`; `cairn_event::{sign, EventBody, Hlc, SigningKey}`; `crate::db::next_hlc`.
- Produces:
  - `struct ReconcileInput<'a> { provenance: &'a str, reason: Option<&'a str> }`
  - `fn build_reconcile_body(event_id: Uuid, subject_a: Uuid, subject_b: Uuid, patient: Uuid, input: &ReconcileInput, node_kid: &str, hlc: Hlc) -> EventBody`
  - `fn build_separate_body(event_id, subject_a, subject_b, patient, input, node_kid, hlc) -> EventBody`
  - `async fn reconcile_medications(client, node_sk, node_kid, node_origin, patient, subject_a, subject_b, input) -> anyhow::Result<Uuid>`
  - `async fn separate_medications(client, node_sk, node_kid, node_origin, patient, subject_a, subject_b, input) -> anyhow::Result<Uuid>`
  - `fn validate_distinct_subjects(a: Uuid, b: Uuid) -> anyhow::Result<()>` (advisory Rust guard mirroring the floor)

- [ ] **Step 1: Write the failing tests**

Append to `crates/cairn-node/src/medication.rs` inside a new test module:

```rust
#[cfg(test)]
mod reconciliation_build_tests {
    use super::*;
    use cairn_event::Hlc;

    fn hlc() -> Hlc {
        Hlc { wall: 1_700_000_000_000, counter: 0, node_origin: "test-node".into() }
    }

    #[test]
    fn build_reconcile_sets_type_schema_twin() {
        let a = Uuid::now_v7();
        let b = Uuid::now_v7();
        let input = ReconcileInput { provenance: "clinician-judgment", reason: Some("brand vs generic") };
        let body = build_reconcile_body(Uuid::now_v7(), a, b, Uuid::now_v7(), &input, "kid", hlc());
        assert_eq!(body.event_type, "clinical.medication-reconciliation.asserted");
        assert_eq!(body.schema_version, "clinical.medication-reconciliation/1");
        assert_eq!(body.payload["subject_a"], a.to_string());
        assert_eq!(body.payload["subject_b"], b.to_string());
        assert_eq!(body.payload["provenance"], "clinician-judgment");
        assert_eq!(body.contributors[0]["role"], "recorded");
        assert!(body.t_effective.is_none());
        assert!(body.plaintext_twin.as_deref().unwrap().contains("Reconciled"));
    }

    #[test]
    fn build_separate_sets_type_schema_twin() {
        let a = Uuid::now_v7();
        let b = Uuid::now_v7();
        let input = ReconcileInput { provenance: "clinician-judgment", reason: None };
        let body = build_separate_body(Uuid::now_v7(), a, b, Uuid::now_v7(), &input, "kid", hlc());
        assert_eq!(body.event_type, "clinical.medication-separation.asserted");
        assert_eq!(body.schema_version, "clinical.medication-separation/1");
        assert!(body.plaintext_twin.as_deref().unwrap().contains("Separated"));
    }

    #[test]
    fn distinct_subjects_guard() {
        let a = Uuid::now_v7();
        assert!(validate_distinct_subjects(a, Uuid::now_v7()).is_ok());
        assert!(validate_distinct_subjects(a, a).is_err());
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p cairn-node --lib reconciliation_build`
Expected: FAIL — `build_reconcile_body` / `ReconcileInput` / `validate_distinct_subjects` not defined.

- [ ] **Step 3: Write the implementation**

Append to `crates/cairn-node/src/medication.rs` (before the `#[cfg(test)]` modules). First extend the top-of-file `use` to include the reconciliation builders:

```rust
use cairn_event::medication::{
    reconciliation_body, render_reconciliation_twin, render_separation_twin, separation_body,
    ReconciliationAssertion,
};
```

Then add the section:

```rust
const RECONCILIATION_SCHEMA_VERSION: &str = "clinical.medication-reconciliation/1";
const SEPARATION_SCHEMA_VERSION: &str = "clinical.medication-separation/1";

/// Clinician-supplied fields of a reconciliation/separation. `provenance` is
/// required by the floor; the CLI defaults it to "clinician-judgment".
pub struct ReconcileInput<'a> {
    pub provenance: &'a str,
    pub reason: Option<&'a str>,
}

/// Advisory Rust guard mirroring the DB floor: refuse a self-reconcile. The DB
/// floor is the real, unbypassable enforcement.
pub fn validate_distinct_subjects(a: Uuid, b: Uuid) -> anyhow::Result<()> {
    if a == b {
        anyhow::bail!("reconciliation subjects must be two DIFFERENT medication threads");
    }
    Ok(())
}

/// Shared body assembler. `verb` selects the event_type/schema/twin; the payload is
/// identical either way (mirrors identity link/unlink).
fn build_reconcile_like_body(
    event_type: &str,
    schema_version: &str,
    twin: String,
    event_id: Uuid,
    subject_a: Uuid,
    subject_b: Uuid,
    patient: Uuid,
    a: &ReconciliationAssertion<'_>,
    payload: serde_json::Value,
    node_kid: &str,
    hlc: Hlc,
) -> EventBody {
    let _ = (subject_a, subject_b, a); // subjects already interned into `payload`/`twin` by the caller
    EventBody {
        event_id: event_id.to_string(),
        patient_id: patient.to_string(),
        event_type: event_type.into(),
        schema_version: schema_version.into(),
        hlc,
        t_effective: None,
        signer_key_id: node_kid.into(),
        contributors: serde_json::json!([{"actor_id": node_kid, "role": "recorded"}]),
        payload,
        attachments: vec![],
        plaintext_twin: Some(twin),
    }
}

/// Assemble the signed `clinical.medication-reconciliation.asserted` EventBody. Pure.
pub fn build_reconcile_body(
    event_id: Uuid,
    subject_a: Uuid,
    subject_b: Uuid,
    patient: Uuid,
    input: &ReconcileInput<'_>,
    node_kid: &str,
    hlc: Hlc,
) -> EventBody {
    let sa = subject_a.to_string();
    let sb = subject_b.to_string();
    let a = ReconciliationAssertion {
        subject_a: &sa,
        subject_b: &sb,
        provenance: input.provenance,
        reason: input.reason,
    };
    build_reconcile_like_body(
        "clinical.medication-reconciliation.asserted",
        RECONCILIATION_SCHEMA_VERSION,
        render_reconciliation_twin(&a),
        event_id,
        subject_a,
        subject_b,
        patient,
        &a,
        reconciliation_body(&a),
        node_kid,
        hlc,
    )
}

/// Assemble the signed `clinical.medication-separation.asserted` EventBody. Pure.
pub fn build_separate_body(
    event_id: Uuid,
    subject_a: Uuid,
    subject_b: Uuid,
    patient: Uuid,
    input: &ReconcileInput<'_>,
    node_kid: &str,
    hlc: Hlc,
) -> EventBody {
    let sa = subject_a.to_string();
    let sb = subject_b.to_string();
    let a = ReconciliationAssertion {
        subject_a: &sa,
        subject_b: &sb,
        provenance: input.provenance,
        reason: input.reason,
    };
    build_reconcile_like_body(
        "clinical.medication-separation.asserted",
        SEPARATION_SCHEMA_VERSION,
        render_separation_twin(&a),
        event_id,
        subject_a,
        subject_b,
        patient,
        &a,
        separation_body(&a),
        node_kid,
        hlc,
    )
}

/// Assert two medication threads are the same real drug. Device-additive; offline-
/// first (no local existence check on either thread). Returns the event id.
#[allow(clippy::too_many_arguments)] // signer + node context + patient/2 subjects/input, mirrors dose orchestrators
pub async fn reconcile_medications(
    client: &tokio_postgres::Client,
    node_sk: &SigningKey,
    node_kid: &str,
    node_origin: &str,
    patient: Uuid,
    subject_a: Uuid,
    subject_b: Uuid,
    input: &ReconcileInput<'_>,
) -> anyhow::Result<Uuid> {
    validate_distinct_subjects(subject_a, subject_b)?;
    let hlc = crate::db::next_hlc(client, node_origin).await?;
    let event_id = Uuid::now_v7();
    let body = build_reconcile_body(event_id, subject_a, subject_b, patient, input, node_kid, hlc);
    let signed = sign(&body, node_sk)?;
    client
        .execute("SELECT submit_event($1)", &[&signed.signed_bytes])
        .await?;
    Ok(event_id)
}

/// Reverse a reconciliation ("actually two different drugs"). Device-additive;
/// offline-first. Returns the event id.
#[allow(clippy::too_many_arguments)]
pub async fn separate_medications(
    client: &tokio_postgres::Client,
    node_sk: &SigningKey,
    node_kid: &str,
    node_origin: &str,
    patient: Uuid,
    subject_a: Uuid,
    subject_b: Uuid,
    input: &ReconcileInput<'_>,
) -> anyhow::Result<Uuid> {
    validate_distinct_subjects(subject_a, subject_b)?;
    let hlc = crate::db::next_hlc(client, node_origin).await?;
    let event_id = Uuid::now_v7();
    let body = build_separate_body(event_id, subject_a, subject_b, patient, input, node_kid, hlc);
    let signed = sign(&body, node_sk)?;
    client
        .execute("SELECT submit_event($1)", &[&signed.signed_bytes])
        .await?;
    Ok(event_id)
}
```

> **Simplification note for the implementer:** the `build_reconcile_like_body` helper above takes unused `subject_a/subject_b/a` params for symmetry; if clippy flags them, drop them from the signature (they're already interned into `payload`/`twin`). Keep the two public `build_*_body` functions and their signatures exactly as specified — Task 6 (CLI) and the tests depend on them.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p cairn-node --lib reconciliation_build`
Expected: PASS (3 tests). Then `cargo clippy -p cairn-node --lib -- -D warnings` clean (simplify `build_reconcile_like_body` per the note if clippy complains about unused args).

- [ ] **Step 5: Commit**

```bash
git add crates/cairn-node/src/medication.rs
git commit -m "feat(cairn-node): reconcile/separate medication orchestrators + pure body builders (slice 3)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 3: In-DB floor — registration, structural check, twin dispatch (`db/033` part 1)

**Files:**
- Create: `db/033_medication_reconciliation.sql`
- Test: Create `crates/cairn-node/tests/medication_reconciliation.rs` (floor tests only in this task)

**Interfaces:**
- Produces (SQL): registers `clinical.medication-reconciliation.asserted` / `clinical.medication-separation.asserted` in `event_type_class`; `cairn_check_medication_reconciliation(p_type text, b jsonb)`; extends `cairn_event_twin` with two branches (HARD-require authored twin, like identity).
- Consumes (test): `cairn_node::medication::{reconcile_medications, separate_medications, ReconcileInput}`; the `build_reconcile_body` pure builder for hand-built hostile bodies.

- [ ] **Step 1: Write the failing tests**

Create `crates/cairn-node/tests/medication_reconciliation.rs`:

```rust
//! §3.15/§3.16 medication reconciliation resolution (slice 3) — DB-gated on
//! $CAIRN_TEST_PG, serialized cluster-wide via db::test_serial_guard. Patients and
//! threads need no pre-existence (offline-first). Key material is runtime-derived.
use cairn_event::{generate_key, sign, EventBody, SigningKey};
use cairn_node::db;
use cairn_node::medication::{
    assert_medication, build_reconcile_body, cease_medication, change_dose, reconcile_medications,
    separate_medications, AssertMedicationInput, CeaseMedicationInput, ChangeDoseInput,
    ReconcileInput,
};
use tokio_postgres::Client;
use uuid::Uuid;

fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

fn db_msg(e: &tokio_postgres::Error) -> String {
    e.as_db_error()
        .map(|d| d.message().to_string())
        .unwrap_or_else(|| e.to_string())
}

/// Truncate the log + every medication projection and enroll a fresh device actor.
async fn setup_node(c: &Client) -> (SigningKey, String) {
    c.batch_execute("TRUNCATE event_log, actor_event, patient_chart CASCADE")
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

fn sample_assert(term: &'static str) -> AssertMedicationInput<'static> {
    AssertMedicationInput {
        term,
        inn_code: None,
        formulation: Some("tablet"),
        dose_amount: Some("40"),
        dose_unit: Some("mg"),
        sig: Some("one BD"),
        info_source: "patient-reported",
        started: Some("2024"),
        started_precision: Some("year"),
    }
}

#[tokio::test]
async fn floor_accepts_valid_reconciliation() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient = Uuid::now_v7();
    let a = Uuid::now_v7();
    let b = Uuid::now_v7();
    let input = ReconcileInput { provenance: "clinician-judgment", reason: Some("brand vs generic") };
    // Offline-first: neither thread need exist locally.
    let ev = reconcile_medications(&c, &sk, &kid, "test-node", patient, a, b, &input)
        .await
        .unwrap();
    let n: i64 = c
        .query_one("SELECT count(*) FROM event_log WHERE event_id = $1", &[&ev])
        .await
        .unwrap()
        .get(0);
    assert_eq!(n, 1, "the reconciliation event landed in the log");
}

#[tokio::test]
async fn floor_rejects_self_reconcile() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient = Uuid::now_v7();
    let a = Uuid::now_v7();
    // Hand-build a self-reconcile (bypass the Rust guard) and submit directly.
    let input = ReconcileInput { provenance: "clinician-judgment", reason: None };
    let hlc = db::next_hlc(&c, "test-node").await.unwrap();
    let body: EventBody = build_reconcile_body(Uuid::now_v7(), a, a, patient, &input, &kid, hlc);
    let signed = sign(&body, &sk).unwrap();
    let res = c
        .execute("SELECT submit_event($1)", &[&signed.signed_bytes])
        .await;
    let err = db_msg(&res.unwrap_err());
    assert!(err.contains("self-reconcile") || err.contains("distinct"), "got: {err}");
}

#[tokio::test]
async fn floor_rejects_missing_provenance() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient = Uuid::now_v7();
    let input = ReconcileInput { provenance: "   ", reason: None };
    let hlc = db::next_hlc(&c, "test-node").await.unwrap();
    let body: EventBody =
        build_reconcile_body(Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7(), patient, &input, &kid, hlc);
    let signed = sign(&body, &sk).unwrap();
    let res = c
        .execute("SELECT submit_event($1)", &[&signed.signed_bytes])
        .await;
    let err = db_msg(&res.unwrap_err());
    assert!(err.contains("provenance"), "got: {err}");
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test -p cairn-node --test medication_reconciliation floor_`
Expected: FAIL — `submit_event` rejects the unknown event type (fail-closed registry) OR the test imports don't resolve yet. (If `CAIRN_TEST_PG` is unset the tests early-return and "pass" vacuously; run against the cluster.)

- [ ] **Step 3: Write `db/033` part 1**

Create `db/033_medication_reconciliation.sql`:

```sql
-- 033_medication_reconciliation.sql — slice 3 of the clinical.medication surface.
--
-- Two append-only verbs over a canonical (low, high) medication_id thread pair:
--   clinical.medication-reconciliation.asserted — the two threads are the same real drug
--   clinical.medication-separation.asserted     — the never-erase reversal (two distinct drugs)
--
-- Mirrors the identity patient_link -> person_member connected-component machinery
-- (db/018), one level down over medication_id threads. Additive: a reconciliation
-- forecloses nothing (both threads' histories survive); ADR-0043's owner-gate does
-- not apply, so cross-author reconciliation is allowed. db/031 and db/032 are
-- untouched EXCEPT the three view reworks in part 3 (same-column-set replay rule).
-- See ADR-0047.
BEGIN;

-- 1. Register both verbs (fail-closed registry, ADR-0010). Additive, never targeting
--    another author (a reconciliation neither suppresses nor forecloses either thread).
INSERT INTO event_type_class (event_type, mode, targets_other_author) VALUES
    ('clinical.medication-reconciliation.asserted', 'additive', FALSE),
    ('clinical.medication-separation.asserted',     'additive', FALSE)
ON CONFLICT (event_type) DO NOTHING;

-- 2. Structural floor for both verbs. Culture-neutral: two distinct valid UUID
--    subjects + a valid patient + non-empty provenance. Nothing clinical is blocked
--    (principle 4). Mirrors cairn_check_link_assertion.
CREATE OR REPLACE FUNCTION cairn_check_medication_reconciliation(p_type text, b jsonb)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    p jsonb := b -> 'payload';
    a text;
    c text;
BEGIN
    IF p IS NULL THEN
        RAISE EXCEPTION 'medication reconciliation: missing payload';
    END IF;
    IF jsonb_typeof(p -> 'subject_a') IS DISTINCT FROM 'string'
       OR jsonb_typeof(p -> 'subject_b') IS DISTINCT FROM 'string' THEN
        RAISE EXCEPTION 'medication reconciliation: subject_a and subject_b must be uuid strings';
    END IF;
    a := p ->> 'subject_a';
    c := p ->> 'subject_b';
    BEGIN
        PERFORM a::uuid;
        PERFORM c::uuid;
    EXCEPTION WHEN others THEN
        RAISE EXCEPTION 'medication reconciliation: subject_a/subject_b must be valid uuids';
    END;
    IF a::uuid = c::uuid THEN
        RAISE EXCEPTION 'medication reconciliation: self-reconcile refused (subjects must be distinct)';
    END IF;
    -- patient_id is a top-level envelope column; the floor sees the full body b.
    IF jsonb_typeof(b -> 'patient_id') IS DISTINCT FROM 'string' THEN
        RAISE EXCEPTION 'medication reconciliation: patient_id must be a uuid string';
    END IF;
    BEGIN
        PERFORM (b ->> 'patient_id')::uuid;
    EXCEPTION WHEN others THEN
        RAISE EXCEPTION 'medication reconciliation: patient_id must be a valid uuid';
    END;
    IF jsonb_typeof(p -> 'provenance') IS DISTINCT FROM 'string'
       OR length(btrim(p ->> 'provenance')) = 0 THEN
        RAISE EXCEPTION 'medication reconciliation: provenance must be a non-empty string (§4.1)';
    END IF;
END;
$$;

-- 3. Extend the shared twin hook. PRESERVES every existing branch from db/032
--    verbatim and adds ONLY the two reconciliation branches. submit_event itself is
--    never re-declared. Identity-critical linkage -> HARD-require an authored twin.
CREATE OR REPLACE FUNCTION cairn_event_twin(p_type text, b jsonb)
RETURNS text LANGUAGE plpgsql AS $$
DECLARE
    v_twin          text := b ->> 'plaintext_twin';
    v_authored      boolean := v_twin IS NOT NULL AND length(regexp_replace(v_twin, '\s+', '', 'g')) > 0;
    v_twin_required text := NULL;
BEGIN
    IF p_type = 'demographic.identifier.asserted' THEN
        PERFORM cairn_check_identifier_assertion(b);
        v_twin_required := 'demographic assertion requires a non-empty authored twin (§4.5)';
    ELSIF p_type = 'demographic.field.asserted' THEN
        PERFORM cairn_check_demographic_field(b);
        v_twin_required := 'demographic assertion requires a non-empty authored twin (§4.5)';
    ELSIF p_type IN ('identity.link.asserted', 'identity.unlink.asserted') THEN
        PERFORM cairn_check_link_assertion(b);
        v_twin_required := 'identity linkage assertion requires a non-empty authored twin (§5.7)';
    ELSIF p_type IN ('identity.dispute.asserted', 'identity.dispute.resolved') THEN
        PERFORM cairn_check_dispute_assertion(p_type, b);
        v_twin_required := 'identity dispute assertion requires a non-empty authored twin (§5.7)';
    ELSIF p_type IN ('identity.pending.asserted', 'identity.identify.asserted') THEN
        PERFORM cairn_check_identity_state_assertion(p_type, b);
        v_twin_required := 'identity-state assertion requires a non-empty authored twin (§5.7)';
    ELSIF p_type = 'identity.repudiate.asserted' THEN
        PERFORM cairn_check_repudiation_assertion(b);
        v_twin_required := 'identity repudiation assertion requires a non-empty authored twin (§5.7)';
    ELSIF p_type IN ('clinical.medication.asserted', 'clinical.medication-cessation.asserted') THEN
        PERFORM cairn_check_medication_assertion(p_type, b);
        v_twin_required := 'medication assertion requires a non-empty authored twin (§3.13/§3.15)';
    ELSIF p_type IN ('clinical.medication-dose-change.asserted', 'clinical.medication-dose-correction.asserted') THEN
        PERFORM cairn_check_medication_dose(p_type, b);
        v_twin_required := 'medication dose assertion requires a non-empty authored twin (§3.13/§3.15)';
    ELSIF p_type IN ('clinical.medication-reconciliation.asserted', 'clinical.medication-separation.asserted') THEN
        PERFORM cairn_check_medication_reconciliation(p_type, b);
        v_twin_required := 'medication reconciliation requires a non-empty authored twin (§3.13/§3.15)';
    END IF;

    IF v_authored THEN
        RETURN v_twin;
    END IF;
    IF v_twin_required IS NOT NULL THEN
        RAISE EXCEPTION 'submit_event: %', v_twin_required;
    END IF;
    RETURN cairn_twin_skeleton(p_type, b);
END;
$$;

COMMIT;
```

> **IMPORTANT for the implementer:** the `cairn_event_twin` body above must be copied from the **live db/032 version** with only the two new reconciliation branches added — verify against `db/032_medication_dose.sql` §3 before writing, because copying a stale body silently drops later floor branches (the migration-replay hazard). Tasks 4 and 5 each append a **separate, self-contained `BEGIN;` … `COMMIT;` block** to this same file (three independent transactions in one file — each commits before the next loads, and dependencies flow forward: part 1's floor fn, then part 2's tables/trigger, then part 3's views). Part 1 (this task) is already a complete `BEGIN;`/`COMMIT;` transaction.

- [ ] **Step 4: Run tests to verify they pass**

Run: `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test -p cairn-node --test medication_reconciliation floor_`
Expected: PASS (3 floor tests).

- [ ] **Step 5: Commit**

```bash
git add db/033_medication_reconciliation.sql crates/cairn-node/tests/medication_reconciliation.rs
git commit -m "feat(db): medication reconciliation floor + twin dispatch (slice 3, db/033 part 1)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 4: Edge overlay + connected-component grouping projection (`db/033` part 2)

**Files:**
- Modify: `db/033_medication_reconciliation.sql` (append part 2 before the final `COMMIT;`)
- Test: append to `crates/cairn-node/tests/medication_reconciliation.rs`

**Interfaces:**
- Produces (SQL): `medication_reconciliation` edge overlay table; `medication_group_member` (medication_id → group_id); `medication_projection_flag`; `cairn_max_medication_group_size()`; `cairn_recompute_medication_group(seed uuid)`; `medication_reconciliation_apply()` trigger fn + trigger on both event types.

- [ ] **Step 1: Write the failing tests**

Append to `crates/cairn-node/tests/medication_reconciliation.rs`:

```rust
/// Helper: the group_id a thread maps to (or the thread itself when un-reconciled).
async fn group_of(c: &Client, med: Uuid) -> Uuid {
    let row = c
        .query_opt(
            "SELECT group_id::text FROM medication_group_member WHERE medication_id = $1::text::uuid",
            &[&med.to_string()],
        )
        .await
        .unwrap();
    match row {
        Some(r) => r.get::<_, String>(0).parse().unwrap(),
        None => med, // no row = collapses to itself
    }
}

#[tokio::test]
async fn reconcile_maps_both_threads_to_min_uuid_group() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient = Uuid::now_v7();
    let a = assert_medication(&c, &sk, &kid, "test-node", patient, &sample_assert("atorvastatin"))
        .await
        .unwrap();
    let b = assert_medication(&c, &sk, &kid, "test-node", patient, &sample_assert("atorvastatin"))
        .await
        .unwrap();
    let input = ReconcileInput { provenance: "clinician-judgment", reason: None };
    reconcile_medications(&c, &sk, &kid, "test-node", patient, a, b, &input)
        .await
        .unwrap();
    let expected = std::cmp::min(a, b);
    assert_eq!(group_of(&c, a).await, expected);
    assert_eq!(group_of(&c, b).await, expected, "both threads collapse to the min-UUID group");
}

#[tokio::test]
async fn transitive_component_and_clean_split() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient = Uuid::now_v7();
    let a = assert_medication(&c, &sk, &kid, "test-node", patient, &sample_assert("metformin"))
        .await
        .unwrap();
    let b = assert_medication(&c, &sk, &kid, "test-node", patient, &sample_assert("metformin"))
        .await
        .unwrap();
    let d = assert_medication(&c, &sk, &kid, "test-node", patient, &sample_assert("metformin"))
        .await
        .unwrap();
    let input = ReconcileInput { provenance: "clinician-judgment", reason: None };
    reconcile_medications(&c, &sk, &kid, "test-node", patient, a, b, &input).await.unwrap();
    reconcile_medications(&c, &sk, &kid, "test-node", patient, b, d, &input).await.unwrap();
    let min = std::cmp::min(a, std::cmp::min(b, d));
    assert_eq!(group_of(&c, a).await, min);
    assert_eq!(group_of(&c, d).await, min, "A-B, B-C transitively one group");
    // Separating B-D splits D back out; A-B stays together.
    separate_medications(&c, &sk, &kid, "test-node", patient, b, d, &input).await.unwrap();
    assert_eq!(group_of(&c, a).await, std::cmp::min(a, b));
    assert_eq!(group_of(&c, b).await, std::cmp::min(a, b));
    assert_eq!(group_of(&c, d).await, d, "D is isolated again after separation");
}

#[tokio::test]
async fn reconciliation_before_threads_converges() {
    // Offline-first: the reconciliation applies before either assert is local.
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient = Uuid::now_v7();
    let a = Uuid::now_v7();
    let b = Uuid::now_v7();
    let input = ReconcileInput { provenance: "clinician-judgment", reason: None };
    reconcile_medications(&c, &sk, &kid, "test-node", patient, a, b, &input).await.unwrap();
    // The edge stands and the group is computed even with no statements yet.
    assert_eq!(group_of(&c, a).await, std::cmp::min(a, b));
    assert_eq!(group_of(&c, b).await, std::cmp::min(a, b));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `CAIRN_TEST_PG=… cargo test -p cairn-node --test medication_reconciliation reconcile_maps transitive_component reconciliation_before`
Expected: FAIL — `medication_group_member` does not exist.

- [ ] **Step 3: Write `db/033` part 2**

In `db/033_medication_reconciliation.sql`, append a new self-contained transaction block after part 1's `COMMIT;`:

```sql
BEGIN;

-- 4. The standing-edge HLC overlay. One row per canonical (low, high) thread pair;
--    the latest-HLC assertion wins the state with the content_address tiebreak (#115).
--    Same shape/index discipline as db/018 patient_link.
CREATE TABLE IF NOT EXISTS medication_reconciliation (
    low             UUID    NOT NULL,
    high            UUID    NOT NULL,
    state           TEXT    NOT NULL CHECK (state IN ('reconciled', 'separated')),
    hlc_wall        BIGINT  NOT NULL,
    hlc_counter     INTEGER NOT NULL,
    origin          TEXT    NOT NULL,
    provenance      TEXT    NOT NULL,
    content_address BYTEA   NOT NULL,
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (low, high),
    CHECK (low < high)
);
GRANT SELECT ON medication_reconciliation TO cairn_agent;
-- Index the high side so the component BFS is indexed in both directions.
CREATE INDEX IF NOT EXISTS medication_reconciliation_high_idx
    ON medication_reconciliation (high) WHERE state = 'reconciled';

-- 5. The golden-medication projection: medication_id -> group_id (min-UUID
--    representative of the connected component). A thread never touched by a
--    reconciliation event has no row and collapses to itself. Mirrors person_member.
CREATE TABLE IF NOT EXISTS medication_group_member (
    medication_id UUID PRIMARY KEY,
    group_id      UUID NOT NULL,
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
GRANT SELECT ON medication_group_member TO cairn_agent;

-- Oversize guard: a component larger than this is a pathology (mass false-merge);
-- refuse on local authoring, clamp-and-flag on remote apply (a node-local GUC must
-- not fork the event set between honest nodes). Mirrors cairn_max_component_size.
CREATE OR REPLACE FUNCTION cairn_max_medication_group_size()
RETURNS integer LANGUAGE sql STABLE AS $$
    SELECT COALESCE(NULLIF(current_setting('cairn.max_medication_group_size', true), '')::integer, 10000);
$$;

CREATE TABLE IF NOT EXISTS medication_projection_flag (
    flag_id       BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    seed          UUID        NOT NULL,
    observed_size INTEGER     NOT NULL,
    cap           INTEGER     NOT NULL,
    flagged_at    TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
GRANT SELECT ON medication_projection_flag TO cairn_agent;

-- Recompute the connected component around one seed thread over the STANDING
-- reconciled edges, rewriting medication_group_member to the min-UUID representative.
-- Cost bounded by the touched component, not the table (ADR-0001 incremental
-- discipline). Mirrors cairn_recompute_component.
CREATE OR REPLACE FUNCTION cairn_recompute_medication_group(p_seed uuid)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    v_members uuid[];
    v_group   uuid;
BEGIN
    WITH RECURSIVE comp(node) AS (
        SELECT p_seed
        UNION
        SELECT CASE WHEN mr.low = comp.node THEN mr.high ELSE mr.low END
        FROM comp
        JOIN medication_reconciliation mr
          ON mr.state = 'reconciled' AND (mr.low = comp.node OR mr.high = comp.node)
    )
    SELECT array_agg(node) INTO v_members FROM comp;

    IF array_length(v_members, 1) > cairn_max_medication_group_size() THEN
        IF current_setting('cairn.remote_apply', true) = 'on' THEN
            INSERT INTO medication_projection_flag (seed, observed_size, cap)
            VALUES (p_seed, array_length(v_members, 1), cairn_max_medication_group_size());
            RETURN;
        END IF;
        RAISE EXCEPTION
            'medication reconciliation: group around % exceeds max size % — refusing to project (matcher pathology)',
            p_seed, cairn_max_medication_group_size();
    END IF;

    -- Canonical representative = the minimum UUID (uuid has a `<` operator but no
    -- min() aggregate). A seed with no edges maps to itself.
    v_group := (SELECT m FROM unnest(v_members) AS m ORDER BY m LIMIT 1);

    INSERT INTO medication_group_member (medication_id, group_id, updated_at)
    SELECT m, v_group, clock_timestamp() FROM unnest(v_members) AS m
    ON CONFLICT (medication_id) DO UPDATE SET
        group_id   = EXCLUDED.group_id,
        updated_at = clock_timestamp();
END;
$$;

-- Fold one reconciliation/separation event into the edge overlay, then recompute the
-- component around both endpoints. A txn-scoped advisory lock serializes recomputes
-- (the db/018 read-modify-write race). #157 collision recording is deferred (design §10).
CREATE OR REPLACE FUNCTION medication_reconciliation_apply()
RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE
    p       jsonb := NEW.body;
    a       uuid  := (p ->> 'subject_a')::uuid;
    b       uuid  := (p ->> 'subject_b')::uuid;
    lo      uuid  := LEAST(a, b);
    hi      uuid  := GREATEST(a, b);
    v_state text  := CASE WHEN NEW.event_type = 'clinical.medication-reconciliation.asserted'
                          THEN 'reconciled' ELSE 'separated' END;
BEGIN
    PERFORM pg_advisory_xact_lock(x'4341524E4D52'::bigint);  -- 'CARNMR'

    INSERT INTO medication_reconciliation
        (low, high, state, hlc_wall, hlc_counter, origin, provenance, content_address)
    VALUES
        (lo, hi, v_state, NEW.hlc_wall, NEW.hlc_counter, NEW.node_origin,
         p ->> 'provenance', NEW.content_address)
    ON CONFLICT (low, high) DO UPDATE SET
        state       = EXCLUDED.state,
        hlc_wall    = EXCLUDED.hlc_wall,
        hlc_counter = EXCLUDED.hlc_counter,
        origin      = EXCLUDED.origin,
        provenance  = EXCLUDED.provenance,
        content_address = EXCLUDED.content_address,
        updated_at  = clock_timestamp()
    WHERE cairn_hlc_overlay_wins(
        EXCLUDED.hlc_wall, EXCLUDED.hlc_counter, EXCLUDED.origin, EXCLUDED.content_address,
        medication_reconciliation.hlc_wall, medication_reconciliation.hlc_counter,
        medication_reconciliation.origin, medication_reconciliation.content_address);

    -- Recompute BOTH endpoints (a reconcile merges; a separation splits into at most
    -- the piece with lo and the piece with hi — both reachable from these seeds).
    PERFORM cairn_recompute_medication_group(lo);
    PERFORM cairn_recompute_medication_group(hi);
    RETURN NULL;  -- AFTER trigger
END;
$$;
DROP TRIGGER IF EXISTS medication_reconciliation_apply_trg ON event_log;
CREATE TRIGGER medication_reconciliation_apply_trg
    AFTER INSERT ON event_log
    FOR EACH ROW WHEN (NEW.event_type IN
        ('clinical.medication-reconciliation.asserted', 'clinical.medication-separation.asserted'))
    EXECUTE FUNCTION medication_reconciliation_apply();

COMMIT;
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `CAIRN_TEST_PG=… cargo test -p cairn-node --test medication_reconciliation reconcile_maps transitive_component reconciliation_before`
Expected: PASS (3 tests).

- [ ] **Step 5: Commit**

```bash
git add db/033_medication_reconciliation.sql crates/cairn-node/tests/medication_reconciliation.rs
git commit -m "feat(db): medication reconciliation edge overlay + component grouping projection (slice 3, db/033 part 2)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 5: Collapsed current/past views + group status/dose + reworked flag (`db/033` part 3)

**Files:**
- Modify: `db/033_medication_reconciliation.sql` (append part 3, ending with the file's single `COMMIT;`)
- Test: append to `crates/cairn-node/tests/medication_reconciliation.rs`

**Interfaces:**
- Produces (SQL): helper views `medication_thread_group`, `medication_group_status`, `medication_group_current_dose`, `medication_group_last_dose`, `medication_group_cessation`, `medication_group_display`; reworked `patient_medication_current`, `patient_medication_past` (collapsed, SAME columns as db/032); reworked `patient_medication_reconciliation_flag` (distinct-group count, via DROP+CREATE).

- [ ] **Step 1: Write the failing tests**

Append to `crates/cairn-node/tests/medication_reconciliation.rs`:

```rust
/// Rows in patient_medication_current for a patient (medication_id, term, dose).
async fn current_rows(c: &Client, patient: Uuid) -> Vec<(Uuid, String, Option<String>)> {
    c.query(
        "SELECT medication_id::text, term, dose_amount \
         FROM patient_medication_current WHERE patient_id = $1::text::uuid \
         ORDER BY term, medication_id",
        &[&patient.to_string()],
    )
    .await
    .unwrap()
    .iter()
    .map(|r| {
        (
            r.get::<_, String>(0).parse().unwrap(),
            r.get::<_, String>(1),
            r.get::<_, Option<String>>(2),
        )
    })
    .collect()
}

async fn flag_count(c: &Client, patient: Uuid) -> i64 {
    c.query_one(
        "SELECT count(*) FROM patient_medication_reconciliation_flag WHERE patient_id = $1::text::uuid",
        &[&patient.to_string()],
    )
    .await
    .unwrap()
    .get(0)
}

#[tokio::test]
async fn reconcile_collapses_to_one_row_and_clears_flag() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient = Uuid::now_v7();
    let a = assert_medication(&c, &sk, &kid, "test-node", patient, &sample_assert("atorvastatin")).await.unwrap();
    let b = assert_medication(&c, &sk, &kid, "test-node", patient, &sample_assert("atorvastatin")).await.unwrap();
    // Two active same-term threads: flagged, two rows.
    assert_eq!(current_rows(&c, patient).await.len(), 2);
    assert_eq!(flag_count(&c, patient).await, 1);
    // Reconcile: one row, flag clears.
    let input = ReconcileInput { provenance: "clinician-judgment", reason: None };
    reconcile_medications(&c, &sk, &kid, "test-node", patient, a, b, &input).await.unwrap();
    let rows = current_rows(&c, patient).await;
    assert_eq!(rows.len(), 1, "collapsed to one row");
    assert_eq!(rows[0].0, std::cmp::min(a, b), "keyed by the min-UUID group");
    assert_eq!(flag_count(&c, patient).await, 0, "flag cleared without a cessation");
    // Separate: re-splits, flag returns.
    separate_medications(&c, &sk, &kid, "test-node", patient, a, b, &input).await.unwrap();
    assert_eq!(current_rows(&c, patient).await.len(), 2);
    assert_eq!(flag_count(&c, patient).await, 1);
}

#[tokio::test]
async fn brand_generic_collapse_without_shared_key() {
    // No shared dup_key (never flagged) — human judgment still collapses them.
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient = Uuid::now_v7();
    let a = assert_medication(&c, &sk, &kid, "test-node", patient, &sample_assert("Lipitor")).await.unwrap();
    let b = assert_medication(&c, &sk, &kid, "test-node", patient, &sample_assert("atorvastatin")).await.unwrap();
    assert_eq!(flag_count(&c, patient).await, 0, "different terms are never flagged");
    assert_eq!(current_rows(&c, patient).await.len(), 2);
    let input = ReconcileInput { provenance: "clinician-judgment", reason: Some("brand vs generic") };
    reconcile_medications(&c, &sk, &kid, "test-node", patient, a, b, &input).await.unwrap();
    assert_eq!(current_rows(&c, patient).await.len(), 1, "collapsed by human judgment");
}

#[tokio::test]
async fn group_current_dose_is_latest_effective_across_members() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient = Uuid::now_v7();
    let a = assert_medication(&c, &sk, &kid, "test-node", patient, &sample_assert("atorvastatin")).await.unwrap();
    let b = assert_medication(&c, &sk, &kid, "test-node", patient, &sample_assert("atorvastatin")).await.unwrap();
    // Thread B gets a later-effective dose change to 80.
    let ch = ChangeDoseInput {
        dose_amount: Some("80"), dose_unit: Some("mg"),
        effective: Some("2025-06"), effective_precision: Some("month"),
        info_source: "clinician-observed", reason: Some("titration"),
    };
    change_dose(&c, &sk, &kid, "test-node", patient, b, &ch).await.unwrap();
    let input = ReconcileInput { provenance: "clinician-judgment", reason: None };
    reconcile_medications(&c, &sk, &kid, "test-node", patient, a, b, &input).await.unwrap();
    let rows = current_rows(&c, patient).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].2.as_deref(), Some("80"), "group current dose = latest-effective across members");
}

#[tokio::test]
async fn mixed_status_resolves_latest_effective() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient = Uuid::now_v7();
    // A active (dose change effective 2025-06); B ceased effective 2024-01 (earlier).
    let a = assert_medication(&c, &sk, &kid, "test-node", patient, &sample_assert("metformin")).await.unwrap();
    let b = assert_medication(&c, &sk, &kid, "test-node", patient, &sample_assert("metformin")).await.unwrap();
    let ch = ChangeDoseInput {
        dose_amount: Some("1000"), dose_unit: Some("mg"),
        effective: Some("2025-06"), effective_precision: Some("month"),
        info_source: "clinician-observed", reason: None,
    };
    change_dose(&c, &sk, &kid, "test-node", patient, a, &ch).await.unwrap();
    let cease = CeaseMedicationInput { stopped: Some("2024-01"), stopped_precision: Some("month"), reason: None };
    cease_medication(&c, &sk, &kid, "test-node", patient, b, &cease).await.unwrap();
    let input = ReconcileInput { provenance: "clinician-judgment", reason: None };
    reconcile_medications(&c, &sk, &kid, "test-node", patient, a, b, &input).await.unwrap();
    // The later-effective standing statement (A's 2025-06 dose) wins → ACTIVE.
    let rows = current_rows(&c, patient).await;
    assert_eq!(rows.len(), 1, "mixed group resolves ACTIVE (later dose beats earlier cessation)");
    assert_eq!(rows[0].2.as_deref(), Some("1000"));

    // Now cease A effective 2026 (later than the dose change) → group flips CEASED.
    let cease_a = CeaseMedicationInput { stopped: Some("2026"), stopped_precision: Some("year"), reason: None };
    cease_medication(&c, &sk, &kid, "test-node", patient, a, &cease_a).await.unwrap();
    assert_eq!(current_rows(&c, patient).await.len(), 0, "all members ceased → group ceased");
    let past: i64 = c.query_one(
        "SELECT count(*) FROM patient_medication_past WHERE patient_id = $1::text::uuid",
        &[&patient.to_string()]).await.unwrap().get(0);
    assert_eq!(past, 1, "the ceased group shows as one past row");
}

#[tokio::test]
async fn single_thread_semantics_unchanged() {
    // Regression: a lone active thread and a lone ceased thread render exactly as slices 1/2.
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient = Uuid::now_v7();
    let a = assert_medication(&c, &sk, &kid, "test-node", patient, &sample_assert("aspirin")).await.unwrap();
    let rows = current_rows(&c, patient).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].0, a, "un-reconciled thread keys by its own id");
    assert_eq!(rows[0].2.as_deref(), Some("40"), "as-asserted dose shows");
    let cease = CeaseMedicationInput { stopped: Some("2025"), stopped_precision: Some("year"), reason: Some("done") };
    cease_medication(&c, &sk, &kid, "test-node", patient, a, &cease).await.unwrap();
    assert_eq!(current_rows(&c, patient).await.len(), 0, "ceased → not current (slice-1 semantics)");
    let past: i64 = c.query_one(
        "SELECT count(*) FROM patient_medication_past WHERE patient_id = $1::text::uuid",
        &[&patient.to_string()]).await.unwrap().get(0);
    assert_eq!(past, 1);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `CAIRN_TEST_PG=… cargo test -p cairn-node --test medication_reconciliation reconcile_collapses brand_generic group_current_dose mixed_status single_thread`
Expected: FAIL — the collapsed views don't exist yet (current still shows two rows for a reconciled pair; flag doesn't count groups).

- [ ] **Step 3: Write `db/033` part 3**

Append a final self-contained transaction block to `db/033_medication_reconciliation.sql`:

```sql
BEGIN;

-- 6. Helper: every asserted thread -> (patient_id, group_id). A thread with no
--    group_member row collapses to itself (COALESCE).
CREATE OR REPLACE VIEW medication_thread_group AS
SELECT s.medication_id,
       s.patient_id,
       COALESCE(gm.group_id, s.medication_id) AS group_id
FROM medication_statement s
LEFT JOIN medication_group_member gm ON gm.medication_id = s.medication_id;
GRANT SELECT ON medication_thread_group TO cairn_agent;

-- 7. Group status by LATEST-EFFECTIVE WINS (ADR-0047 ruling 3). Compare the max
--    effective sort key across active members' dose points vs ceased members'
--    cessations. Ties (equal effective keys) resolve to 'active' (documented
--    tiebreak — keep the drug visible). A single-member group can never be "mixed"
--    (all-active or all-ceased), so this reduces EXACTLY to slice-1/2 semantics.
--    MAX(... COLLATE "C") forces byte-order max (ADR-0045).
CREATE OR REPLACE VIEW medication_group_status AS
WITH active_eff AS (
    SELECT g.group_id,
           MAX(cairn_dose_effective_sort_key(de.effective_value, de.hlc_wall) COLLATE "C") AS eff
    FROM medication_dose_event de
    JOIN medication_thread_group g ON g.medication_id = de.medication_id
    WHERE NOT EXISTS (SELECT 1 FROM medication_cessation cc WHERE cc.medication_id = de.medication_id)
    GROUP BY g.group_id
),
ceased_eff AS (
    SELECT g.group_id,
           MAX(cairn_dose_effective_sort_key(c.stopped_value, c.hlc_wall) COLLATE "C") AS eff
    FROM medication_cessation c
    JOIN medication_thread_group g ON g.medication_id = c.medication_id
    GROUP BY g.group_id
)
SELECT grp.group_id, grp.patient_id,
       CASE WHEN ce.eff IS NULL THEN 'active'
            WHEN ae.eff IS NULL THEN 'ceased'
            WHEN ae.eff >= ce.eff THEN 'active'
            ELSE 'ceased' END AS status
FROM (SELECT DISTINCT group_id, patient_id FROM medication_thread_group) grp
LEFT JOIN active_eff ae ON ae.group_id = grp.group_id
LEFT JOIN ceased_eff ce ON ce.group_id = grp.group_id;
GRANT SELECT ON medication_group_status TO cairn_agent;

-- 8. Group current dose = latest-EFFECTIVE dose point across ACTIVE members only
--    (a ceased member's doses are not "current"). Correction overlay applied per
--    point (thread-scoped join, as in db/032). One row per group.
CREATE OR REPLACE VIEW medication_group_current_dose AS
SELECT DISTINCT ON (g.group_id)
    g.group_id, g.patient_id, de.dose_event_id,
    CASE WHEN corr.corrected_dose_event_id IS NOT NULL THEN corr.amount ELSE de.amount END AS amount,
    CASE WHEN corr.corrected_dose_event_id IS NOT NULL THEN corr.unit   ELSE de.unit   END AS unit,
    de.effective_value, de.effective_precision
FROM medication_dose_event de
JOIN medication_thread_group g ON g.medication_id = de.medication_id
LEFT JOIN medication_dose_correction corr
    ON corr.corrected_dose_event_id = de.dose_event_id AND corr.medication_id = de.medication_id
WHERE NOT EXISTS (SELECT 1 FROM medication_cessation cc WHERE cc.medication_id = de.medication_id)
ORDER BY g.group_id,
         cairn_dose_effective_sort_key(de.effective_value, de.hlc_wall) COLLATE "C" DESC,
         de.hlc_wall DESC, de.hlc_counter DESC, de.origin COLLATE "C" DESC, de.content_address DESC;
GRANT SELECT ON medication_group_current_dose TO cairn_agent;

-- 9. Group last dose = latest-EFFECTIVE dose point across ALL members (for the past
--    view — the last recorded dose before stopping, incl. now-ceased members).
CREATE OR REPLACE VIEW medication_group_last_dose AS
SELECT DISTINCT ON (g.group_id)
    g.group_id, g.patient_id,
    CASE WHEN corr.corrected_dose_event_id IS NOT NULL THEN corr.amount ELSE de.amount END AS amount,
    CASE WHEN corr.corrected_dose_event_id IS NOT NULL THEN corr.unit   ELSE de.unit   END AS unit
FROM medication_dose_event de
JOIN medication_thread_group g ON g.medication_id = de.medication_id
LEFT JOIN medication_dose_correction corr
    ON corr.corrected_dose_event_id = de.dose_event_id AND corr.medication_id = de.medication_id
ORDER BY g.group_id,
         cairn_dose_effective_sort_key(de.effective_value, de.hlc_wall) COLLATE "C" DESC,
         de.hlc_wall DESC, de.hlc_counter DESC, de.origin COLLATE "C" DESC, de.content_address DESC;
GRANT SELECT ON medication_group_last_dose TO cairn_agent;

-- 10. Group's latest cessation (stopped info for the past view). One row per group.
CREATE OR REPLACE VIEW medication_group_cessation AS
SELECT DISTINCT ON (g.group_id)
    g.group_id, g.patient_id, c.stopped_value, c.stopped_precision, c.reason
FROM medication_cessation c
JOIN medication_thread_group g ON g.medication_id = c.medication_id
ORDER BY g.group_id,
         cairn_dose_effective_sort_key(c.stopped_value, c.hlc_wall) COLLATE "C" DESC,
         c.hlc_wall DESC, c.hlc_counter DESC, c.origin COLLATE "C" DESC, c.content_address DESC;
GRANT SELECT ON medication_group_cessation TO cairn_agent;

-- 11. Group display fields from the canonical member's statement: prefer the exact
--     group_id member if its assert is local, else the min-UUID member present.
CREATE OR REPLACE VIEW medication_group_display AS
SELECT DISTINCT ON (g.group_id)
    g.group_id, g.patient_id, s.term, s.inn_code, s.formulation, s.sig, s.info_source,
    s.started_value, s.started_precision,
    to_timestamp(s.hlc_wall / 1000.0) AS asserted_at
FROM medication_statement s
JOIN medication_thread_group g ON g.medication_id = s.medication_id
ORDER BY g.group_id, (s.medication_id = g.group_id) DESC, s.medication_id;
GRANT SELECT ON medication_group_display TO cairn_agent;

-- 12. Rework the current/past views to emit ONE row per group. CRITICAL: keep the
--     EXACT SAME COLUMN SET as db/032 (replay safety — db/031/032 replay on every
--     connect; widening/renaming breaks reconnect). Only rows + dose/status source
--     change. medication_id = group_id (the stable group key; = the thread itself
--     for an un-reconciled thread, so slice-1/2 behavior is preserved).
CREATE OR REPLACE VIEW patient_medication_current AS
SELECT d.group_id AS medication_id, d.patient_id, d.term, d.inn_code, d.formulation,
       cd.amount AS dose_amount, cd.unit AS dose_unit,
       d.sig, d.info_source, d.started_value, d.started_precision, d.asserted_at
FROM medication_group_display d
JOIN medication_group_status st ON st.group_id = d.group_id
LEFT JOIN medication_group_current_dose cd ON cd.group_id = d.group_id
WHERE st.status = 'active';
GRANT SELECT ON patient_medication_current TO cairn_agent;

CREATE OR REPLACE VIEW patient_medication_past AS
SELECT d.group_id AS medication_id, d.patient_id, d.term, d.inn_code, d.formulation,
       ld.amount AS dose_amount, ld.unit AS dose_unit,
       d.sig, d.info_source, d.started_value, d.started_precision, d.asserted_at,
       gc.stopped_value, gc.stopped_precision, gc.reason
FROM medication_group_display d
JOIN medication_group_status st ON st.group_id = d.group_id
LEFT JOIN medication_group_last_dose ld ON ld.group_id = d.group_id
LEFT JOIN medication_group_cessation gc ON gc.group_id = d.group_id
WHERE st.status = 'ceased';
GRANT SELECT ON patient_medication_past TO cairn_agent;

-- 13. Rework the reconciliation flag: fire only when ACTIVE threads sharing a dup_key
--     span MORE THAN ONE distinct group (un-reconciled duplicates). Reconciling
--     collapses them to one group -> no flag; separating re-splits -> flag returns.
--     Column `thread_count` is RENAMED to `group_count`, so DROP + CREATE (not
--     CREATE OR REPLACE) — replay-safe because db/033 always loads after db/031.
DROP VIEW IF EXISTS patient_medication_reconciliation_flag;
CREATE VIEW patient_medication_reconciliation_flag AS
SELECT patient_id,
       coalesce(inn_code, lower(btrim(term) COLLATE "C")) AS dup_key,
       count(DISTINCT group_id)                           AS group_count,
       array_agg(DISTINCT medication_id ORDER BY medication_id) AS medication_ids
FROM (
    SELECT s.patient_id, s.medication_id, s.inn_code, s.term,
           COALESCE(gm.group_id, s.medication_id) AS group_id
    FROM medication_statement s
    LEFT JOIN medication_group_member gm ON gm.medication_id = s.medication_id
    WHERE NOT EXISTS (SELECT 1 FROM medication_cessation c WHERE c.medication_id = s.medication_id)
) t
GROUP BY patient_id, coalesce(inn_code, lower(btrim(term) COLLATE "C"))
HAVING count(DISTINCT group_id) > 1;
GRANT SELECT ON patient_medication_reconciliation_flag TO cairn_agent;

COMMIT;
```

> **Note on the flag rename:** `medication_dose.rs`/`medication.rs` tests from slices 1/2 that reference `thread_count` do not exist (the flag column was only ever read as `count`), but grep `thread_count` across `crates/` and `db/` before renaming; if any consumer reads it, update it in this task.

- [ ] **Step 4: Run tests to verify they pass**

Run: `CAIRN_TEST_PG=… cargo test -p cairn-node --test medication_reconciliation`
Expected: PASS (all reconciliation tests — floor, grouping, collapse, status, regression).

Then run the slice-1/2 suites to prove no regression:
Run: `CAIRN_TEST_PG=… cargo test -p cairn-node --test medication --test medication_dose`
Expected: PASS (slice-1 10/10, slice-2 14/14 unchanged).

- [ ] **Step 5: Commit**

```bash
git add db/033_medication_reconciliation.sql crates/cairn-node/tests/medication_reconciliation.rs
git commit -m "feat(db): collapsed group current/past views + latest-effective status + reworked flag (slice 3, db/033 part 3)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 6: CLI wiring (`medication reconcile` / `medication separate`)

**Files:**
- Modify: `crates/cairn-node/src/main.rs` (add two `Cmd` variants + two handlers)

**Interfaces:**
- Consumes: `cairn_node::medication::{reconcile_medications, separate_medications, ReconcileInput}`; the existing `load_signing_key`, `ensure_registration_actor`, `cairn_node::identity::load_local` helpers.

- [ ] **Step 1: Add the `Cmd` variants**

In `crates/cairn-node/src/main.rs`, after the `MedicationCorrectDose { … }` variant (ends ~line 560), add:

```rust
    /// Reconcile two medication threads as the same real drug
    /// (clinical.medication-reconciliation.asserted). Symmetric, reversible, additive —
    /// both threads' histories are preserved; the current list collapses to one row.
    /// Offline-first: neither thread need be present locally.
    MedicationReconcile {
        /// The patient UUID both threads belong to.
        patient: Uuid,
        /// The first medication thread id.
        thread_a: Uuid,
        /// The second medication thread id (must differ from thread_a).
        thread_b: Uuid,
        /// Provenance of the judgment (§4.1). Defaults to "clinician-judgment".
        #[arg(long, default_value = "clinician-judgment")]
        provenance: String,
        /// Optional free-text reason ("brand vs generic", "duplicate on transfer").
        #[arg(long)]
        reason: Option<String>,
    },
    /// Separate two previously-reconciled threads — "actually two different drugs"
    /// (clinical.medication-separation.asserted). The never-erase reversal.
    MedicationSeparate {
        /// The patient UUID both threads belong to.
        patient: Uuid,
        /// The first medication thread id.
        thread_a: Uuid,
        /// The second medication thread id (must differ from thread_a).
        thread_b: Uuid,
        /// Provenance of the judgment (§4.1). Defaults to "clinician-judgment".
        #[arg(long, default_value = "clinician-judgment")]
        provenance: String,
        /// Optional free-text reason.
        #[arg(long)]
        reason: Option<String>,
    },
```

- [ ] **Step 2: Add the handlers**

In the `match cli.cmd { … }` block, after the `Cmd::MedicationCorrectDose { … } => { … }` handler, add:

```rust
        Cmd::MedicationReconcile { patient, thread_a, thread_b, provenance, reason } => {
            cairn_node::medication::validate_distinct_subjects(thread_a, thread_b)?;
            let node_sk = load_signing_key(&cli.key, true)?;
            let node_kid = hex::encode(node_sk.verifying_key().to_bytes());
            let db = cairn_node::db::connect(&cli.conn).await?;
            let id = cairn_node::identity::load_local(&db).await?;
            ensure_registration_actor(&db, &node_kid).await?;
            let input = cairn_node::medication::ReconcileInput {
                provenance: &provenance,
                reason: reason.as_deref(),
            };
            let event_id = cairn_node::medication::reconcile_medications(
                &db, &node_sk, &node_kid, &id.node_id_hex, patient, thread_a, thread_b, &input,
            )
            .await?;
            println!("reconciled threads {thread_a} + {thread_b}; event {event_id}");
        }
        Cmd::MedicationSeparate { patient, thread_a, thread_b, provenance, reason } => {
            cairn_node::medication::validate_distinct_subjects(thread_a, thread_b)?;
            let node_sk = load_signing_key(&cli.key, true)?;
            let node_kid = hex::encode(node_sk.verifying_key().to_bytes());
            let db = cairn_node::db::connect(&cli.conn).await?;
            let id = cairn_node::identity::load_local(&db).await?;
            ensure_registration_actor(&db, &node_kid).await?;
            let input = cairn_node::medication::ReconcileInput {
                provenance: &provenance,
                reason: reason.as_deref(),
            };
            let event_id = cairn_node::medication::separate_medications(
                &db, &node_sk, &node_kid, &id.node_id_hex, patient, thread_a, thread_b, &input,
            )
            .await?;
            println!("separated threads {thread_a} + {thread_b}; event {event_id}");
        }
```

- [ ] **Step 3: Build to verify it compiles**

Run: `cargo build -p cairn-node`
Expected: builds clean. Then `cargo clippy -p cairn-node -- -D warnings` clean.

- [ ] **Step 4: Manual end-to-end smoke (documented; run against the test cluster)**

```bash
# In a scratch dir with an initialized node (see existing MedicationAssert smoke).
# 1. assert two duplicate threads, note the two thread ids printed
# 2. `medication-reconcile <patient> <thread_a> <thread_b>`
# 3. confirm: SELECT * FROM patient_medication_current WHERE patient_id = <patient>;  -> one row
#            SELECT * FROM patient_medication_reconciliation_flag WHERE patient_id = <patient>;  -> no rows
# 4. `medication-separate <patient> <thread_a> <thread_b>` -> two rows + flag returns
```
Record the observed output in the PR description (paper-parity check: the duplicate cleared with no false cessation).

- [ ] **Step 5: Commit**

```bash
git add crates/cairn-node/src/main.rs
git commit -m "feat(cairn-node): medication reconcile/separate CLI verbs (slice 3)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 7: ADR-0047 + spec bump + HANDOVER/ROADMAP + file the cross-patient issue

**Files:**
- Create: `docs/spec/decisions/0047-medication-reconciliation-resolution.md`
- Modify: `docs/spec/index.md` (version v0.47 → v0.48), `docs/spec/decisions/README.md` (index row)
- Modify: `docs/HANDOVER.md`, `docs/ROADMAP.md` (slice 32 entry)
- GitHub issue: cross-patient reconciliation guard (design tension §9.1)

- [ ] **Step 1: Write ADR-0047**

Create `docs/spec/decisions/0047-medication-reconciliation-resolution.md` with exactly this approved content (immutable once committed):

````markdown
# ADR-0047 — Medication reconciliation is a link, not a cessation

- **Status:** Accepted
- **Date:** 2026-07-13
- **Refines:** principle 2 (never merge, always link); [ADR-0010](0010-additive-vs-suppressing-classification.md) (additive-vs-suppressing); the [data-model](../data-model.md) rule *"medication lists = union + flagged for clinician reconciliation on conflict"*. Follows the identity linkage precedent ([§5.7](../identity.md), `patient_link`/`person_member`).

## Context

Slice 1 of the `clinical.medication` surface records each medication as an immortal `medication_id` thread and surfaces an advisory `patient_medication_reconciliation_flag` when two *active* threads for one patient share a duplicate key. Slice 1 named exactly one "resolution": **cease the redundant thread.** That is clinically wrong. Cessation (`clinical.medication-cessation.asserted`) means *the patient stopped taking the drug* — a false clinical statement when the two threads are simply the same ongoing medication recorded twice (two nodes, an encounter re-entry, a brand recorded alongside its generic). Using cessation to silence a duplicate flag fabricates a stop event: the drug drops off the current list, and the audit log asserts a discontinuation that never happened. This is the "slice-1 wart."

The record already has the right shape for this elsewhere: the **identity** subsystem never merges two patient records — it *links* them (`identity.link.asserted`), deriving a golden identity by connected component, cleanly reversible (principle 2). Medication threads that turn out to be the same drug are the identical problem one level down.

## Decision

**Reconciling two medication threads is a first-class, symmetric, reversible *link* between threads — never a cessation.** Clearing a duplicate flag never fabricates a stop event.

1. **Two additive verbs over a thread pair**, mirroring identity link/unlink: `clinical.medication-reconciliation.asserted` (state `reconciled`) and `clinical.medication-separation.asserted` (state `separated`, the never-erase reversal — *"these are actually two different drugs"*). Both carry two distinct `medication_id` subjects; both are `additive`, `targets_other_author=FALSE`. A reconciliation forecloses nothing — **both threads' full histories, dose timelines, and cessations survive verbatim** — so [ADR-0043](0043-suppression-self-only-disagreement-is-additive.md)'s owner-gate does not apply and **cross-author reconciliation is allowed** (clinician B may reconcile threads authored by A and C — normal clinical practice).

2. **Collapse by connected component (min-UUID canonical), mirroring `person_member`.** Reconciled threads form a group whose representative is the minimum `medication_id`; the current-medication list shows **one row per group**. Reconciliation is permitted between **any** two threads, not only flagged ones — this is the *only* path today for a human to reconcile brand↔generic, which the deterministic key-based flag cannot detect. The floor validates structure only (two distinct valid UUID subjects, valid patient, non-empty provenance) — never blocks a clinical judgment (principle 4).

3. **Group status on disagreement = latest-*effective* wins.** Once threads are linked, a later event may cease one member while another stays active. The collapsed row resolves this bitemporally (consistent with the slice-2 current-dose rule): all-active → active; all-ceased → ceased; **mixed → the member with the latest-*effective* standing statement decides** (a cessation effective last month loses to a dose-change effective yesterday). A single-thread group can never be mixed, so **slice-1/2 single-thread semantics are provably unchanged** — this rule is inert until a genuine multi-thread disagreement exists.

## Consequences

**Now guaranteed:** a duplicate flag is cleared without a fabricated cessation; the drug stays on the current list as one entry; both source threads remain intact and the link is reversible with no data loss; brand↔generic can be reconciled by human judgment ahead of the deferred Tier-A drug dictionary.

**Accepted costs / deferred:**
- **Cross-patient reconciliation** (linking two patients' threads) is a pathology the offline-first floor cannot cheaply reject (both threads' patients may be non-local). The collapse view groups per patient, so a bad edge is low-stakes, reversible, and auditable; a hard guard is deferred, tracked, not left implicit (house rule 5).
- **Group display term** is taken from the canonical (min-UUID) member — a documented approximation for brand↔generic (which name shows); refining it is deferred.
- The `cairn_event_twin` dispatch grows by two branches, feeding the filed [#173](https://github.com/cairn-ehr/cairn-ehr/issues/173) registry-refactor debt (the idiom slices 1/2 already follow).
- **Device-additive**, like slices 1/2 — but because the verbs are `additive`+`targets_other_author=FALSE`, a future human-attested reconciliation adds a responsibility-bearing contributor with **zero floor change** (identity's exact pattern).

**How we would know the bet is failing:** clinicians routinely need to reconcile threads across patients (they don't — a cross-patient duplicate is a mis-identification, handled by the identity subsystem), or the min-UUID canonical produces a clinically misleading display name often enough to demand a chosen-survivor model instead of symmetric collapse.

**Not a new founding principle.** This is principle 2 (never merge, always link) and the data-model union+reconcile rule applied to medication threads, reusing the identity linkage projection pattern verbatim.
````

- [ ] **Step 2: Bump the spec version + add the ADR index row**

In `docs/spec/index.md`, change the stated spec version `v0.47` → `v0.48`. In `docs/spec/decisions/README.md`, add the ADR-0047 row to the index table (match the existing row format), and add a compact row to the ADR table in `docs/HANDOVER.md`:

```markdown
| [0047](spec/decisions/0047-medication-reconciliation-resolution.md) | Medication reconciliation is a link (never a cessation); symmetric collapse; latest-effective group status | §3.15/§3.16 (principle 2) |
```

- [ ] **Step 3: File the cross-patient guard issue**

```bash
gh issue create --title "Medication reconciliation: guard/flag a cross-patient group (needs design)" \
  --body "$(cat <<'EOF'
Slice 3 (ADR-0047) allows reconciling any two medication_id threads. The in-DB floor
validates structure only (two distinct valid UUID subjects + valid patient +
provenance) and does NOT check that both threads belong to the same patient — the
offline-first posture means both threads' patient_ids may be non-local at submit time.

The collapse views group per patient, so a pathological cross-patient reconciliation
edge is low-stakes, reversible, and auditable, but it is not actively surfaced. This
needs a DESIGN DECISION (not a trivial guard): whether to refuse at the floor (needs
both patients local — breaks offline-first), flag in medication_projection_flag on a
cross-patient component in cairn_recompute_medication_group, or leave to the hub-tier
sweep. Deferred from slice 3 design tension §9.1.
EOF
)"
```
Record the issue number and reference it in HANDOVER.

- [ ] **Step 4: Update HANDOVER.md + ROADMAP.md**

Add a top "This session" block to `docs/HANDOVER.md` summarizing slice 3 (branch, ADR-0047, spec v0.48, what BUILT, deferred items incl. the filed issue number). Add a "Slice 32 — medication reconciliation resolution" entry to `docs/ROADMAP.md` Phase 4 (matching the slice-31 entry format). Keep both concise; prune stale detail if either approaches 500 lines (house rule 8; ask the user if unsure what to cut).

- [ ] **Step 5: Build docs + commit**

Run: `uv run --with-requirements docs/requirements.txt -- mkdocs build`
Expected: builds clean (no broken links to the new ADR).

```bash
git add docs/
git commit -m "docs(adr,spec,handover,roadmap): ADR-0047 medication reconciliation; spec v0.48; slice 32

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 8: Whole-branch verification + code review

**Files:** none (verification only)

- [ ] **Step 1: Full workspace gates**

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test --workspace
uv run --with-requirements docs/requirements.txt -- mkdocs build
```
Expected: fmt clean; clippy clean; ALL tests pass (cairn-event pure incl. reconciliation; cairn-node DB-gated incl. medication 10/10, medication_dose 14/14, medication_reconciliation all; cairn-sync); mkdocs builds. If `cargo test --workspace` runs without `CAIRN_TEST_PG`, DB-gated tests vacuously pass — always run with the env var against the cluster.

- [ ] **Step 2: Request code review**

Use the superpowers:requesting-code-review skill (or `/code-review`) on the whole branch diff vs `main`. Fix any Critical/Important findings in-branch; file an issue for anything out-of-scope (house rule 5). Re-run Step 1 after fixes.

- [ ] **Step 3: Final commit (if review fixes were made)**

```bash
git add -A
git commit -m "fix: address slice-3 review findings

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Self-review notes (checked against the spec)

- **Spec §2 (two verbs)** → Tasks 1/2/3 (builders, orchestrators, registration). ✓
- **Spec §3 (signed body shape)** → Task 1 (`ReconciliationAssertion`, omit-when-absent reason). ✓
- **Spec §4 (floor)** → Task 3 (`cairn_check_medication_reconciliation`: distinct subjects, valid patient, non-empty provenance; twin dispatch). ✓
- **Spec §5 (grouping projection)** → Task 4 (edge overlay, `medication_group_member`, recompute, oversize flag). ✓
- **Spec §5.1 (status latest-effective / current dose / single-thread reduction)** → Task 5 (`medication_group_status`, `medication_group_current_dose`, regression test). ✓
- **Spec §5.2 (collapsed views + flag)** → Task 5 (reworked current/past same-columns; flag distinct-group). ✓
- **Spec §6 (orchestrators + CLI)** → Tasks 2/6. ✓
- **Spec §7 (tests)** → Tasks 3/4/5 DB-gated + Task 1/2 pure. ✓
- **Spec §8 (deliverables) / ADR-0047 / spec bump / issue** → Task 7. ✓
- **Spec §9 tensions** → cross-patient issue filed (Task 7); display-term approximation documented in `medication_group_display`; #173 twin dispatch grows by 2 (Task 3); replay-safety honored (Task 5 same-columns + DROP/CREATE flag). ✓
- **Type consistency:** `ReconcileInput`, `build_reconcile_body`/`build_separate_body`, `reconcile_medications`/`separate_medications`, `validate_distinct_subjects`, `group_count` — used identically across Tasks 2/3/5/6. ✓
```
