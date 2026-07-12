# Medication dose overlay (slice 2) — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add two append-only verbs to the `clinical.medication` surface — a dose *change* (additive
titration) and a dose *correction* (a recorded dose was wrong) — with a queryable dose-history timeline and
a bitemporal current-dose winner.

**Architecture:** Pure Rust builders (`cairn-event`) → node orchestrators + CLI (`cairn-node`) → an in-DB
floor + a trigger-fed dose-timeline projection (`db/032`). A medication thread's dose timeline = the assert's
initial dose (point 0, seeded by a trigger) + one point per change; corrections overlay a *targeted* point.
Two projection tables (event / correction) mirror slice-1's statement/cessation split so folds are
arrival-order-independent. Everything is device-additive and additive-classified.

**Tech Stack:** Rust (tokio-postgres, serde_json, uuid, ed25519 via `cairn-event`), PostgreSQL 18 + `cairn_pgx`,
PL/pgSQL.

**Design doc:** [`docs/superpowers/specs/2026-07-12-medication-dose-overlay-design.md`](../specs/2026-07-12-medication-dose-overlay-design.md)

## Global Constraints

- **AGPL-3.0**; every dependency AGPL-3.0-compatible (none new are added here).
- **TDD** — failing test first, then code. Load-bearing (safety-critical write path, §9).
- **Never hard-code cryptographic material in tests** — keys via `generate_key()` (house rule 6 / issue #146).
- **Projection winner tiebreaks over TEXT keys use `COLLATE "C"`** ([ADR-0045](../../spec/decisions/0045-collation-independent-projection-tiebreaks.md)).
- **Device-additive**: every authored event carries `contributors = [{"actor_id": <node_kid>, "role": "recorded"}]`, `t_effective: None`, no attestation (consistent with slice 1).
- **Offline-first**: a dose event never requires its referenced thread / target to exist locally.
- **No new founding principle / envelope field / ADR / spec bump.** No change to `db/031` (all slice-2 SQL lives in `db/032`), no change to the wire envelope.
- **Event types / schema versions (verbatim):**
  - `clinical.medication-dose-change.asserted` / `clinical.medication-dose-change/1`
  - `clinical.medication-dose-correction.asserted` / `clinical.medication-dose-correction/1`
- **Test DB:** DB-gated tests read `CAIRN_TEST_PG` (e.g. `host=127.0.0.1 port=5532 user=hherb dbname=cairn_test`), skip silently if unset, and self-serialize via `db::test_serial_guard`. Pure tests need no DB.
- **File-size guideline:** aim < 500 lines/file; `cairn-event/src/medication.rs` is split into a module in Task 1.

---

### Task 1: Refactor — split `cairn-event` medication.rs into a module (no behavior change)

**Files:**
- Delete→recreate: `crates/cairn-event/src/medication.rs` → `crates/cairn-event/src/medication/mod.rs`
- Create: `crates/cairn-event/src/medication/assert.rs`
- Create: `crates/cairn-event/src/medication/cessation.rs`
- (No change to `crates/cairn-event/src/lib.rs` — `pub mod medication;` already present.)

**Interfaces:**
- Produces (unchanged public API, now re-exported from the module): `MedicationAssertion`, `medication_assertion_body`, `render_medication_twin`, `MedicationCessation`, `medication_cessation_body`, `render_medication_cessation_twin`.

This is a pure move: the current `medication.rs` (316 lines) is reorganized so no symbol path changes for
consumers (`cairn_event::medication::medication_assertion_body` etc. keep resolving via `pub use`).

- [ ] **Step 1: Create `medication/assert.rs`**

Move, verbatim from the current `medication.rs`: the `MedicationAssertion` struct (with its doc comments),
`medication_assertion_body`, `render_medication_twin`, **and** the `#[cfg(test)] mod tests` items that
exercise them: `full_assertion`, `assertion_body_carries_all_present_fields`,
`assertion_body_omits_absent_optionals_never_null`, `assertion_body_dose_amount_only_omits_unit`,
`assertion_twin_is_nonempty_and_reads_naturally`, `assertion_twin_nonempty_for_vague_term_only`,
`assertion_twin_renders_unit_without_amount`. Top of file:

```rust
//! Medication *assertion* builder (the "start" verb) — mints a medication thread.
//! Pure: no clock, no randomness, no I/O. Optional fields are inserted only when
//! present (never serialized as null), so an added-later field never changes an
//! existing event's content address (principle 11).
use serde_json::{json, Value};
```

The moved `#[cfg(test)] mod tests { use super::*; ... }` keeps its `full_assertion` helper local to this file.

- [ ] **Step 2: Create `medication/cessation.rs`**

Move, verbatim: the `MedicationCessation` struct, `medication_cessation_body`,
`render_medication_cessation_twin`, and the tests `cessation_body_carries_and_omits_correctly`,
`cessation_twin_is_nonempty`. Top of file:

```rust
//! Medication *cessation* builder (the "stop" verb) — references an existing thread.
//! Pure: shapes only the payload JSON. The drug name lives on the assertion, so the
//! cessation carries only the thread id, an optional stop date, and an optional reason.
use serde_json::{json, Value};
```

- [ ] **Step 3: Replace `medication.rs` with `medication/mod.rs`**

Delete `crates/cairn-event/src/medication.rs`; create `crates/cairn-event/src/medication/mod.rs` carrying
the original module-level doc comment plus the submodule wiring:

```rust
//! §3.15/§3.16 medication recording — the clinical-content builders.
//!
//! Pure: no clock, no randomness, no I/O. The cairn-node edge mints the ids,
//! stamps the HLC, and signs; these functions only shape the `payload` JSON that
//! becomes `EventBody.payload`. Optional fields are inserted only when present —
//! never serialized as null — so an added-later field never changes an existing
//! event's content address (principle 11, the demographics idiom).
//!
//! Verbs over an immortal `medication_id` thread: an *assertion* (`assert`) mints
//! the thread; a *cessation* (`cessation`) ends it; a *dose change / correction*
//! (`dose`, slice 2) overlays the dose over time.
pub mod assert;
pub mod cessation;

pub use assert::{medication_assertion_body, render_medication_twin, MedicationAssertion};
pub use cessation::{
    medication_cessation_body, render_medication_cessation_twin, MedicationCessation,
};
```

- [ ] **Step 4: Run the cairn-event suite — verify green (no behavior change)**

Run: `cargo test -p cairn-event`
Expected: PASS, same test count as before the move for the medication tests (9 medication tests still run).

- [ ] **Step 5: Verify downstream still compiles**

Run: `cargo build -p cairn-node`
Expected: PASS (the re-exports keep `cairn_event::medication::*` paths valid).

- [ ] **Step 6: Commit**

```bash
git add crates/cairn-event/src/medication.rs crates/cairn-event/src/medication/
git commit -m "refactor(cairn-event): split medication.rs into assert/cessation module (no behavior change)"
```

---

### Task 2: `cairn-event` — dose-change & dose-correction builders + twins (pure, TDD)

**Files:**
- Create: `crates/cairn-event/src/medication/dose.rs`
- Modify: `crates/cairn-event/src/medication/mod.rs` (add `pub mod dose;` + re-export)

**Interfaces:**
- Produces:
  - `struct DoseChange<'a> { medication_id, dose_amount: Option, dose_unit: Option, effective: Option, effective_precision: Option, info_source: &'a str, reason: Option }`
  - `fn dose_change_body(d: &DoseChange) -> serde_json::Value`
  - `fn render_dose_change_twin(d: &DoseChange) -> String`
  - `struct DoseCorrection<'a> { medication_id, corrects: &'a str, dose_amount: Option, dose_unit: Option, info_source: Option<&'a str>, reason: Option }`
  - `fn dose_correction_body(d: &DoseCorrection) -> serde_json::Value`
  - `fn render_dose_correction_twin(d: &DoseCorrection) -> String`

- [ ] **Step 1: Write failing tests in `dose.rs`**

```rust
//! Slice-2 dose *change* / *correction* builders. Pure: shapes only payload JSON.
//! A change is additive (both doses true over effective time); a correction says a
//! recorded dose was wrong and references (`corrects`) the dose event it fixes.
//! Dose fields are honest-unknown (principle 4): a change with no amount ("upped it,
//! dunno to what") and a correction to unknown ("that 40 was a guess, strike it")
//! are both first-class.
use serde_json::{json, Value};

pub struct DoseChange<'a> {
    pub medication_id: &'a str,
    pub dose_amount: Option<&'a str>,
    pub dose_unit: Option<&'a str>,
    pub effective: Option<&'a str>,
    pub effective_precision: Option<&'a str>,
    pub info_source: &'a str,
    pub reason: Option<&'a str>,
}

pub struct DoseCorrection<'a> {
    pub medication_id: &'a str,
    pub corrects: &'a str,
    pub dose_amount: Option<&'a str>,
    pub dose_unit: Option<&'a str>,
    pub info_source: Option<&'a str>,
    pub reason: Option<&'a str>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn full_change() -> DoseChange<'static> {
        DoseChange {
            medication_id: "11111111-1111-7111-8111-111111111111",
            dose_amount: Some("80"),
            dose_unit: Some("mg"),
            effective: Some("2025-06"),
            effective_precision: Some("month"),
            info_source: "clinician-observed",
            reason: Some("titration"),
        }
    }

    #[test]
    fn change_body_carries_all_present_fields() {
        let v = dose_change_body(&full_change());
        assert_eq!(v["medication_id"], "11111111-1111-7111-8111-111111111111");
        assert_eq!(v["dose"]["amount"], "80");
        assert_eq!(v["dose"]["unit"], "mg");
        assert_eq!(v["effective"]["value"], "2025-06");
        assert_eq!(v["effective"]["precision"], "month");
        assert_eq!(v["info_source"], "clinician-observed");
        assert_eq!(v["reason"], "titration");
    }

    #[test]
    fn change_body_unknown_amount_is_first_class() {
        // "they upped my metformin, don't know to what" — a known change, unknown target.
        let c = DoseChange {
            medication_id: "22222222-2222-7222-8222-222222222222",
            dose_amount: None,
            dose_unit: None,
            effective: Some("2025"),
            effective_precision: Some("year"),
            info_source: "patient-reported",
            reason: None,
        };
        let v = dose_change_body(&c);
        assert!(
            !v.as_object().unwrap().contains_key("dose"),
            "absent dose omitted entirely, not null"
        );
        assert_eq!(v["effective"]["value"], "2025");
        assert_eq!(v["info_source"], "patient-reported");
    }

    #[test]
    fn change_twin_is_nonempty_and_reads_naturally() {
        let s = render_dose_change_twin(&full_change());
        assert!(s.contains("80 mg"));
        assert!(s.contains("2025-06"));
        assert!(s.contains("titration"));
        assert!(s.contains("clinician-observed"));
        assert!(!s.trim().is_empty());
    }

    #[test]
    fn change_twin_nonempty_when_amount_unknown() {
        let c = DoseChange {
            medication_id: "22222222-2222-7222-8222-222222222222",
            dose_amount: None,
            dose_unit: None,
            effective: Some("2025"),
            effective_precision: Some("year"),
            info_source: "patient-reported",
            reason: None,
        };
        assert!(!render_dose_change_twin(&c).trim().is_empty());
    }

    #[test]
    fn correction_body_carries_and_omits_correctly() {
        let full = DoseCorrection {
            medication_id: "11111111-1111-7111-8111-111111111111",
            corrects: "33333333-3333-7333-8333-333333333333",
            dose_amount: Some("20"),
            dose_unit: Some("mg"),
            info_source: Some("clinician-observed"),
            reason: Some("mis-keyed"),
        };
        let v = dose_correction_body(&full);
        assert_eq!(v["corrects"], "33333333-3333-7333-8333-333333333333");
        assert_eq!(v["dose"]["amount"], "20");
        assert_eq!(v["reason"], "mis-keyed");
        assert_eq!(v["info_source"], "clinician-observed");
    }

    #[test]
    fn correction_to_unknown_omits_dose() {
        // "the 40 I typed was a guess — strike it, dose unknown" (principle 4).
        let c = DoseCorrection {
            medication_id: "11111111-1111-7111-8111-111111111111",
            corrects: "33333333-3333-7333-8333-333333333333",
            dose_amount: None,
            dose_unit: None,
            info_source: None,
            reason: Some("was a guess"),
        };
        let v = dose_correction_body(&c);
        assert!(
            !v.as_object().unwrap().contains_key("dose"),
            "correct-to-unknown carries no dose key"
        );
        assert!(!v.as_object().unwrap().contains_key("info_source"));
        assert_eq!(v["corrects"], "33333333-3333-7333-8333-333333333333");
    }

    #[test]
    fn correction_twin_nonempty_including_to_unknown() {
        let full = DoseCorrection {
            medication_id: "11111111-1111-7111-8111-111111111111",
            corrects: "33333333-3333-7333-8333-333333333333",
            dose_amount: Some("20"),
            dose_unit: Some("mg"),
            info_source: None,
            reason: Some("mis-keyed"),
        };
        let s = render_dose_correction_twin(&full);
        assert!(s.contains("20 mg"));
        assert!(s.contains("mis-keyed"));

        let unknown = DoseCorrection {
            medication_id: "11111111-1111-7111-8111-111111111111",
            corrects: "33333333-3333-7333-8333-333333333333",
            dose_amount: None,
            dose_unit: None,
            info_source: None,
            reason: None,
        };
        assert!(!render_dose_correction_twin(&unknown).trim().is_empty());
    }
}
```

- [ ] **Step 2: Run — verify it fails to compile (builders undefined)**

Run: `cargo test -p cairn-event dose::`
Expected: FAIL — `cannot find function dose_change_body` etc.

- [ ] **Step 3: Implement the builders + twins in `dose.rs`** (above the `#[cfg(test)]` block)

```rust
/// Build the `clinical.medication-dose-change.asserted` payload. `dose` and
/// `effective` are inserted only when present (honest-unknown).
pub fn dose_change_body(d: &DoseChange) -> Value {
    let mut p = json!({
        "medication_id": d.medication_id,
        "info_source": d.info_source,
    });
    let obj = p.as_object_mut().expect("json! built an object");
    if d.dose_amount.is_some() || d.dose_unit.is_some() {
        let mut dose = json!({});
        let o = dose.as_object_mut().expect("json! built an object");
        if let Some(a) = d.dose_amount {
            o.insert("amount".into(), json!(a));
        }
        if let Some(u) = d.dose_unit {
            o.insert("unit".into(), json!(u));
        }
        obj.insert("dose".into(), dose);
    }
    if let Some(v) = d.effective {
        let mut eff = json!({ "value": v });
        if let Some(pr) = d.effective_precision {
            eff.as_object_mut()
                .expect("json! built an object")
                .insert("precision".into(), json!(pr));
        }
        obj.insert("effective".into(), eff);
    }
    if let Some(r) = d.reason {
        obj.insert("reason".into(), json!(r));
    }
    p
}

/// The §3.13 legibility twin for a dose change. Always non-empty.
pub fn render_dose_change_twin(d: &DoseChange) -> String {
    let mut s = String::from("Dose changed");
    match (d.dose_amount, d.dose_unit) {
        (Some(a), Some(u)) => s.push_str(&format!(" to {a} {u}")),
        (Some(a), None) => s.push_str(&format!(" to {a}")),
        (None, Some(u)) => s.push_str(&format!(" to {u}")),
        (None, None) => {}
    }
    if let Some(v) = d.effective {
        s.push_str(&format!(" (effective {v})"));
    }
    if let Some(r) = d.reason {
        s.push_str(&format!(" — {r}"));
    }
    s.push_str(&format!(" ({})", d.info_source));
    s
}

/// Build the `clinical.medication-dose-correction.asserted` payload. `dose` omitted
/// entirely when both parts are absent (correct-to-unknown).
pub fn dose_correction_body(d: &DoseCorrection) -> Value {
    let mut p = json!({
        "medication_id": d.medication_id,
        "corrects": d.corrects,
    });
    let obj = p.as_object_mut().expect("json! built an object");
    if d.dose_amount.is_some() || d.dose_unit.is_some() {
        let mut dose = json!({});
        let o = dose.as_object_mut().expect("json! built an object");
        if let Some(a) = d.dose_amount {
            o.insert("amount".into(), json!(a));
        }
        if let Some(u) = d.dose_unit {
            o.insert("unit".into(), json!(u));
        }
        obj.insert("dose".into(), dose);
    }
    if let Some(src) = d.info_source {
        obj.insert("info_source".into(), json!(src));
    }
    if let Some(r) = d.reason {
        obj.insert("reason".into(), json!(r));
    }
    p
}

/// The §3.13 legibility twin for a dose correction. States the corrected value (the
/// old value lives on the corrected event). Always non-empty.
pub fn render_dose_correction_twin(d: &DoseCorrection) -> String {
    let mut s = String::from("Dose corrected");
    match (d.dose_amount, d.dose_unit) {
        (Some(a), Some(u)) => s.push_str(&format!(" to {a} {u}")),
        (Some(a), None) => s.push_str(&format!(" to {a}")),
        (None, Some(u)) => s.push_str(&format!(" to {u}")),
        (None, None) => s.push_str(" to unknown"),
    }
    if let Some(r) = d.reason {
        s.push_str(&format!(" — {r}"));
    }
    s
}
```

- [ ] **Step 4: Wire the module** — add to `crates/cairn-event/src/medication/mod.rs`:

```rust
pub mod dose;

pub use dose::{
    dose_change_body, dose_correction_body, render_dose_change_twin, render_dose_correction_twin,
    DoseChange, DoseCorrection,
};
```

(Place `pub mod dose;` next to the other `pub mod` lines and the `pub use` next to the others.)

- [ ] **Step 5: Run — verify pass**

Run: `cargo test -p cairn-event dose::`
Expected: PASS (7 tests).

- [ ] **Step 6: Commit**

```bash
git add crates/cairn-event/src/medication/
git commit -m "feat(cairn-event): dose-change + dose-correction builders and twins (medication slice 2)"
```

---

### Task 3: `cairn-node` — dose orchestrators + build bodies + target resolver

**Files:**
- Modify: `crates/cairn-node/src/medication.rs` (append; ~159 → ~300 lines, still < 500)

**Interfaces:**
- Consumes: `cairn_event::medication::{DoseChange, dose_change_body, render_dose_change_twin, DoseCorrection, dose_correction_body, render_dose_correction_twin}`, `cairn_event::{sign, EventBody, Hlc, SigningKey}`, `crate::db::next_hlc`.
- Produces:
  - `struct ChangeDoseInput<'a> { dose_amount: Option, dose_unit: Option, effective: Option, effective_precision: Option, info_source: &'a str, reason: Option }`
  - `fn build_dose_change_body(event_id: Uuid, medication_id: Uuid, patient: Uuid, input: &ChangeDoseInput, node_kid: &str, hlc: Hlc) -> EventBody`
  - `async fn change_dose(client, node_sk, node_kid, node_origin, patient: Uuid, medication_id: Uuid, input: &ChangeDoseInput) -> anyhow::Result<Uuid>` (returns the change event id)
  - `struct CorrectDoseInput<'a> { dose_amount: Option, dose_unit: Option, info_source: Option<&'a str>, reason: Option }`
  - `fn build_dose_correction_body(event_id: Uuid, medication_id: Uuid, patient: Uuid, corrects: Uuid, input: &CorrectDoseInput, node_kid: &str, hlc: Hlc) -> EventBody`
  - `async fn correct_dose(client, node_sk, node_kid, node_origin, patient: Uuid, medication_id: Uuid, corrects: Uuid, input: &CorrectDoseInput) -> anyhow::Result<Uuid>` (returns the correction event id)
  - `async fn resolve_correction_target(client, medication_id: Uuid, explicit: Option<Uuid>) -> anyhow::Result<Uuid>` (explicit target, else the current dose point)

- [ ] **Step 1: Write failing pure build tests** (append to the `#[cfg(test)]` block, or add one) in `crates/cairn-node/src/medication.rs`:

```rust
#[cfg(test)]
mod dose_build_tests {
    use super::*;
    use cairn_event::Hlc;

    fn hlc() -> Hlc {
        Hlc { wall: 1_700_000_000_000, counter: 0, node_origin: "test-node".into() }
    }

    #[test]
    fn build_change_sets_type_schema_twin() {
        let input = ChangeDoseInput {
            dose_amount: Some("80"),
            dose_unit: Some("mg"),
            effective: Some("2025-06"),
            effective_precision: Some("month"),
            info_source: "clinician-observed",
            reason: Some("titration"),
        };
        let b = build_dose_change_body(
            Uuid::now_v7(),
            Uuid::now_v7(),
            Uuid::now_v7(),
            &input,
            "kid",
            hlc(),
        );
        assert_eq!(b.event_type, "clinical.medication-dose-change.asserted");
        assert_eq!(b.schema_version, "clinical.medication-dose-change/1");
        assert!(b.plaintext_twin.as_deref().unwrap().contains("80 mg"));
        assert_eq!(b.payload["dose"]["amount"], "80");
        assert_eq!(b.contributors[0]["role"], "recorded");
        assert!(b.t_effective.is_none());
    }

    #[test]
    fn build_correction_sets_type_schema_corrects() {
        let corrects = Uuid::now_v7();
        let input = CorrectDoseInput {
            dose_amount: Some("20"),
            dose_unit: Some("mg"),
            info_source: None,
            reason: Some("mis-keyed"),
        };
        let b = build_dose_correction_body(
            Uuid::now_v7(),
            Uuid::now_v7(),
            Uuid::now_v7(),
            corrects,
            &input,
            "kid",
            hlc(),
        );
        assert_eq!(b.event_type, "clinical.medication-dose-correction.asserted");
        assert_eq!(b.schema_version, "clinical.medication-dose-correction/1");
        assert_eq!(b.payload["corrects"], corrects.to_string());
        assert!(b.plaintext_twin.as_deref().unwrap().contains("20 mg"));
    }
}
```

- [ ] **Step 2: Run — verify fail (types/functions undefined)**

Run: `cargo test -p cairn-node --lib medication::dose_build_tests`
Expected: FAIL — unresolved `ChangeDoseInput` / `build_dose_change_body` etc.

- [ ] **Step 3: Implement** — append to `crates/cairn-node/src/medication.rs`. First extend the `use` at the top to add the dose imports:

```rust
use cairn_event::medication::{
    dose_change_body, dose_correction_body, medication_assertion_body, medication_cessation_body,
    render_dose_change_twin, render_dose_correction_twin, render_medication_cessation_twin,
    render_medication_twin, DoseChange, DoseCorrection, MedicationAssertion, MedicationCessation,
};
```

Then append:

```rust
const DOSE_CHANGE_SCHEMA_VERSION: &str = "clinical.medication-dose-change/1";
const DOSE_CORRECTION_SCHEMA_VERSION: &str = "clinical.medication-dose-correction/1";

/// Clinician-supplied fields of a dose change. `info_source` required (a new clinical
/// claim); dose fields honest-unknown ("upped it, dunno to what").
pub struct ChangeDoseInput<'a> {
    pub dose_amount: Option<&'a str>,
    pub dose_unit: Option<&'a str>,
    pub effective: Option<&'a str>,
    pub effective_precision: Option<&'a str>,
    pub info_source: &'a str,
    pub reason: Option<&'a str>,
}

/// Assemble the signed `clinical.medication-dose-change.asserted` EventBody. Pure.
pub fn build_dose_change_body(
    event_id: Uuid,
    medication_id: Uuid,
    patient: Uuid,
    input: &ChangeDoseInput<'_>,
    node_kid: &str,
    hlc: Hlc,
) -> EventBody {
    let mid = medication_id.to_string();
    let d = DoseChange {
        medication_id: &mid,
        dose_amount: input.dose_amount,
        dose_unit: input.dose_unit,
        effective: input.effective,
        effective_precision: input.effective_precision,
        info_source: input.info_source,
        reason: input.reason,
    };
    EventBody {
        event_id: event_id.to_string(),
        patient_id: patient.to_string(),
        event_type: "clinical.medication-dose-change.asserted".into(),
        schema_version: DOSE_CHANGE_SCHEMA_VERSION.into(),
        hlc,
        t_effective: None,
        signer_key_id: node_kid.into(),
        contributors: serde_json::json!([{"actor_id": node_kid, "role": "recorded"}]),
        payload: dose_change_body(&d),
        attachments: vec![],
        plaintext_twin: Some(render_dose_change_twin(&d)),
    }
}

/// Record a dose change on an existing thread. Offline-first (no local existence
/// check on the thread). Returns the change event id.
pub async fn change_dose(
    client: &tokio_postgres::Client,
    node_sk: &SigningKey,
    node_kid: &str,
    node_origin: &str,
    patient: Uuid,
    medication_id: Uuid,
    input: &ChangeDoseInput<'_>,
) -> anyhow::Result<Uuid> {
    let hlc = crate::db::next_hlc(client, node_origin).await?;
    let event_id = Uuid::now_v7();
    let body = build_dose_change_body(event_id, medication_id, patient, input, node_kid, hlc);
    let signed = sign(&body, node_sk)?;
    client
        .execute("SELECT submit_event($1)", &[&signed.signed_bytes])
        .await?;
    Ok(event_id)
}

/// Clinician-supplied fields of a dose correction. All optional (correct-to-unknown).
pub struct CorrectDoseInput<'a> {
    pub dose_amount: Option<&'a str>,
    pub dose_unit: Option<&'a str>,
    pub info_source: Option<&'a str>,
    pub reason: Option<&'a str>,
}

/// Assemble the signed `clinical.medication-dose-correction.asserted` EventBody. Pure.
pub fn build_dose_correction_body(
    event_id: Uuid,
    medication_id: Uuid,
    patient: Uuid,
    corrects: Uuid,
    input: &CorrectDoseInput<'_>,
    node_kid: &str,
    hlc: Hlc,
) -> EventBody {
    let mid = medication_id.to_string();
    let corrects_s = corrects.to_string();
    let d = DoseCorrection {
        medication_id: &mid,
        corrects: &corrects_s,
        dose_amount: input.dose_amount,
        dose_unit: input.dose_unit,
        info_source: input.info_source,
        reason: input.reason,
    };
    EventBody {
        event_id: event_id.to_string(),
        patient_id: patient.to_string(),
        event_type: "clinical.medication-dose-correction.asserted".into(),
        schema_version: DOSE_CORRECTION_SCHEMA_VERSION.into(),
        hlc,
        t_effective: None,
        signer_key_id: node_kid.into(),
        contributors: serde_json::json!([{"actor_id": node_kid, "role": "recorded"}]),
        payload: dose_correction_body(&d),
        attachments: vec![],
        plaintext_twin: Some(render_dose_correction_twin(&d)),
    }
}

/// Correct a wrongly-recorded dose on a targeted dose event. Offline-first (the
/// target need not exist locally). Returns the correction event id.
pub async fn correct_dose(
    client: &tokio_postgres::Client,
    node_sk: &SigningKey,
    node_kid: &str,
    node_origin: &str,
    patient: Uuid,
    medication_id: Uuid,
    corrects: Uuid,
    input: &CorrectDoseInput<'_>,
) -> anyhow::Result<Uuid> {
    let hlc = crate::db::next_hlc(client, node_origin).await?;
    let event_id = Uuid::now_v7();
    let body =
        build_dose_correction_body(event_id, medication_id, patient, corrects, input, node_kid, hlc);
    let signed = sign(&body, node_sk)?;
    client
        .execute("SELECT submit_event($1)", &[&signed.signed_bytes])
        .await?;
    Ok(event_id)
}

/// Resolve the dose event a correction should target. If `explicit` is given, use it;
/// otherwise the current (latest-effective) dose point of the thread. Errors if the
/// thread has no local dose timeline (offline-first: pass --target explicitly then).
pub async fn resolve_correction_target(
    client: &tokio_postgres::Client,
    medication_id: Uuid,
    explicit: Option<Uuid>,
) -> anyhow::Result<Uuid> {
    if let Some(t) = explicit {
        return Ok(t);
    }
    let row = client
        .query_opt(
            "SELECT dose_event_id FROM medication_current_dose WHERE medication_id = $1::text::uuid",
            &[&medication_id.to_string()],
        )
        .await?;
    match row {
        Some(r) => Ok(r.get::<_, Uuid>(0)),
        None => anyhow::bail!(
            "no local dose timeline for thread {medication_id}; pass --target <dose_event_id> explicitly"
        ),
    }
}
```

- [ ] **Step 4: Run — verify the build tests pass**

Run: `cargo test -p cairn-node --lib medication::dose_build_tests`
Expected: PASS (2 tests). (`resolve_correction_target` compiles; its DB behavior is covered in Task 5/6.)

- [ ] **Step 5: Commit**

```bash
git add crates/cairn-node/src/medication.rs
git commit -m "feat(cairn-node): change_dose/correct_dose orchestrators + target resolver (medication slice 2)"
```

---

### Task 4: `db/032` floor — classification, structural check, twin dispatch (TDD via DB)

**Files:**
- Create: `db/032_medication_dose.sql`
- Modify: `crates/cairn-node/src/db.rs` (register `032_medication_dose` in `SCHEMA_FILES`)
- Create: `crates/cairn-node/tests/medication_dose.rs` (floor tests; extended in Tasks 5–6)

**Interfaces:**
- Consumes: `submit_event` (db/005), `event_type_class`, `cairn_check_medication_dose`, `cairn_event_twin`.
- Produces: floor rejects malformed dose-change/correction; accepts well-formed into `event_log`.

> **TDD order (this task and Tasks 5–6):** `include_str!` requires the migration file to exist before `db.rs`
> compiles, so the SQL is shown before the test below. Execute test-first: write the Step-3 test, run it
> **without** registering the migration in `db.rs` (the dose types are then unregistered → the fail-closed
> classification rejects them with a message that is *not* `info_source`/`dose`, so the reject asserts fail —
> RED). Then create + register the migration and re-run → GREEN. This gives a genuine RED→GREEN even though the
> file must exist to compile.

- [ ] **Step 1: Create `db/032_medication_dose.sql`** with the floor (projection added in Task 5). The `cairn_event_twin` body below reproduces **all** existing branches from `db/031` verbatim and adds the two dose branches (this function is CREATE-OR-REPLACE and db/032 loads last; dropping a prior branch would silently disable that floor):

```sql
-- 032_medication_dose.sql — slice 2 of the clinical.medication surface (§3.15/§3.16).
--
-- Two append-only verbs over the existing medication_id thread:
--   clinical.medication-dose-change.asserted     — the dose changed (additive; both
--                                                   doses true over effective time)
--   clinical.medication-dose-correction.asserted — a recorded dose was wrong; carries
--                                                   `corrects` = the dose event it fixes
--
-- Floor: structural only (principle 4 — never block a dose write beyond integrity).
-- Projection (added below): a dose timeline (point 0 seeded from the assert + one
-- point per change) with corrections overlaying a targeted point; current dose is the
-- latest-EFFECTIVE point (bitemporal §5.1). db/031 is UNTOUCHED — all slice-2 SQL is here.
BEGIN;

-- 1. Register both verbs. Additive, never targeting another author: a correction does
--    NOT foreclose the corrected event (kept verbatim in event_log + flagged in the
--    history); it only wins the current-dose projection. So ADR-0043's suppression
--    owner-gate does not apply and cross-author dose correction is allowed (as with a
--    corrected DOB in demographics).
INSERT INTO event_type_class (event_type, mode, targets_other_author) VALUES
    ('clinical.medication-dose-change.asserted',     'additive', FALSE),
    ('clinical.medication-dose-correction.asserted', 'additive', FALSE)
ON CONFLICT (event_type) DO NOTHING;

-- 2. Structural floor for both verbs. RAISE EXCEPTION per violation.
CREATE OR REPLACE FUNCTION cairn_check_medication_dose(p_type text, b jsonb)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    p jsonb := b -> 'payload';
BEGIN
    IF p IS NULL THEN
        RAISE EXCEPTION 'medication dose: missing payload';
    END IF;
    -- medication_id is the thread key on BOTH verbs.
    IF jsonb_typeof(p -> 'medication_id') IS DISTINCT FROM 'string' THEN
        RAISE EXCEPTION 'medication dose: medication_id must be a uuid string';
    END IF;
    BEGIN
        PERFORM (p ->> 'medication_id')::uuid;
    EXCEPTION WHEN others THEN
        RAISE EXCEPTION 'medication dose: medication_id must be a valid uuid';
    END;

    IF p_type = 'clinical.medication-dose-change.asserted' THEN
        -- A change is a new clinical claim: it carries its provenance.
        IF jsonb_typeof(p -> 'info_source') IS DISTINCT FROM 'string'
           OR length(btrim(p ->> 'info_source')) = 0 THEN
            RAISE EXCEPTION 'medication dose-change: info_source must be a non-empty string';
        END IF;
        -- Not a pure no-op: it must state a dose, an effective date, or a reason.
        IF NOT (p ? 'dose' OR p ? 'effective'
                OR (jsonb_typeof(p -> 'reason') = 'string' AND length(btrim(p ->> 'reason')) > 0)) THEN
            RAISE EXCEPTION 'medication dose-change: must carry a dose, an effective date, or a reason (principle 4 floor)';
        END IF;
    ELSIF p_type = 'clinical.medication-dose-correction.asserted' THEN
        -- `corrects` names the dose event being fixed. Existence is NOT required —
        -- offline-first: the target may replicate after the correction.
        IF jsonb_typeof(p -> 'corrects') IS DISTINCT FROM 'string' THEN
            RAISE EXCEPTION 'medication dose-correction: corrects must be a uuid string';
        END IF;
        BEGIN
            PERFORM (p ->> 'corrects')::uuid;
        EXCEPTION WHEN others THEN
            RAISE EXCEPTION 'medication dose-correction: corrects must be a valid uuid';
        END;
        -- dose optional (correct-to-unknown); info_source optional (a record fix).
    END IF;
END;
$$;

-- 3. Extend the shared twin hook. PRESERVES every existing branch from db/025+db/031
--    verbatim and adds ONLY the two dose branches. submit_event itself is never re-declared.
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

- [ ] **Step 2: Register the migration** — in `crates/cairn-node/src/db.rs`, add after the `031_medication` entry in `SCHEMA_FILES` (before the closing `];`):

```rust
    // §3.15 slice 2: medication dose change/correction floor + dose-timeline projection.
    (
        "032_medication_dose",
        include_str!("../../../db/032_medication_dose.sql"),
    ),
```

- [ ] **Step 3: Write failing floor tests** in `crates/cairn-node/tests/medication_dose.rs`:

```rust
//! §3.15 medication dose overlay (slice 2) — DB-gated on $CAIRN_TEST_PG, serialized
//! cluster-wide via db::test_serial_guard. Patients need no pre-existence (offline-first).
//! Key material is runtime-derived (generate_key), never literal (house rule 6).
use cairn_event::{generate_key, sign, EventBody, SigningKey};
use cairn_node::db;
use cairn_node::medication::{
    assert_medication, build_dose_change_body, build_dose_correction_body, change_dose,
    correct_dose, resolve_correction_target, AssertMedicationInput, ChangeDoseInput,
    CorrectDoseInput,
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

fn sample_assert() -> AssertMedicationInput<'static> {
    AssertMedicationInput {
        term: "atorvastatin",
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
async fn floor_rejects_dose_change_without_info_source() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient = Uuid::now_v7();
    let med_id = Uuid::now_v7();

    // Hand-build a dose-change with a blank info_source; submit directly.
    let input = ChangeDoseInput {
        dose_amount: Some("80"),
        dose_unit: Some("mg"),
        effective: Some("2025-06"),
        effective_precision: Some("month"),
        info_source: "   ",
        reason: None,
    };
    let hlc = db::next_hlc(&c, "test-node").await.unwrap();
    let body: EventBody =
        build_dose_change_body(Uuid::now_v7(), med_id, patient, &input, &kid, hlc);
    let signed = sign(&body, &sk).unwrap();
    let res = c
        .execute("SELECT submit_event($1)", &[&signed.signed_bytes])
        .await;
    let err = db_msg(&res.unwrap_err());
    assert!(err.contains("info_source"), "got: {err}");
}

#[tokio::test]
async fn floor_rejects_empty_dose_change_noop() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient = Uuid::now_v7();

    // info_source present but no dose / effective / reason → a pure no-op.
    let input = ChangeDoseInput {
        dose_amount: None,
        dose_unit: None,
        effective: None,
        effective_precision: None,
        info_source: "clinician-observed",
        reason: None,
    };
    let hlc = db::next_hlc(&c, "test-node").await.unwrap();
    let body: EventBody =
        build_dose_change_body(Uuid::now_v7(), Uuid::now_v7(), patient, &input, &kid, hlc);
    let signed = sign(&body, &sk).unwrap();
    let res = c
        .execute("SELECT submit_event($1)", &[&signed.signed_bytes])
        .await;
    let err = db_msg(&res.unwrap_err());
    assert!(err.contains("dose") || err.contains("no-op") || err.contains("effective"), "got: {err}");
}

#[tokio::test]
async fn floor_accepts_wellformed_change_and_correction_into_log() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient = Uuid::now_v7();

    let med_id = assert_medication(&c, &sk, &kid, "test-node", patient, &sample_assert())
        .await
        .unwrap();

    let ch = ChangeDoseInput {
        dose_amount: Some("80"),
        dose_unit: Some("mg"),
        effective: Some("2025-06"),
        effective_precision: Some("month"),
        info_source: "clinician-observed",
        reason: Some("titration"),
    };
    let change_evt = change_dose(&c, &sk, &kid, "test-node", patient, med_id, &ch)
        .await
        .unwrap();

    // A correction of the change we just made (target it explicitly).
    let corr = CorrectDoseInput {
        dose_amount: Some("60"),
        dose_unit: Some("mg"),
        info_source: None,
        reason: Some("mis-keyed"),
    };
    let target = resolve_correction_target(&c, med_id, Some(change_evt))
        .await
        .unwrap();
    correct_dose(&c, &sk, &kid, "test-node", patient, med_id, target, &corr)
        .await
        .unwrap();

    let n: i64 = c
        .query_one(
            "SELECT count(*) FROM event_log WHERE event_type LIKE 'clinical.medication-dose-%'",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(n, 2, "both dose events landed in the log");
}
```

- [ ] **Step 4: Observe RED (before registering the migration in `db.rs`)**

With the test written (Step 3) but the `SCHEMA_FILES` entry NOT yet added, run:
`CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test -p cairn-node --test medication_dose floor_rejects`
Expected: FAIL — the dose types are unregistered, so `submit_event`'s fail-closed classification rejects them with a message that does not contain `info_source`/`dose`; the reject asserts fail.

- [ ] **Step 5: Register the migration (Step 2) and re-run — verify GREEN**

Run: `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test -p cairn-node --test medication_dose floor`
Expected: PASS — both reject tests AND `floor_accepts_wellformed_change_and_correction_into_log` (that test passes an *explicit* correction target, so it does not depend on the `medication_current_dose` view; the two dose events land in `event_log`). Timeline/projection behavior is verified in Task 5.

- [ ] **Step 6: Commit**

```bash
git add db/032_medication_dose.sql crates/cairn-node/src/db.rs crates/cairn-node/tests/medication_dose.rs
git commit -m "feat(db): medication dose floor — classification + structural check + twin dispatch (slice 2)"
```

---

### Task 5: `db/032` projection — dose timeline (point-0 seed, change, current + history views)

**Files:**
- Modify: `db/032_medication_dose.sql` (insert projection SQL before the final `COMMIT;`)
- Modify: `crates/cairn-node/tests/medication_dose.rs` (add timeline tests)

**Interfaces:**
- Produces: tables `medication_dose_event`, `medication_dose_correction` (empty, no trigger yet); function `cairn_dose_effective_sort_key`; views `medication_current_dose`, `patient_medication_dose_history`; reworked `patient_medication_current` / `patient_medication_past` (dose sourced from the timeline, `dose_event_id` + `dose_corrected` exposed).

> **TDD order:** per the Task-4 note, write the Step-2 tests first and run them — RED because `medication_current_dose` / the dose triggers don't exist yet (the `current_dose` helper errors on the missing relation). Then insert the Step-1 SQL and re-run → GREEN.

- [ ] **Step 1: Insert the projection SQL** in `db/032_medication_dose.sql`, immediately **before** the final `COMMIT;`:

```sql
-- 4. Deterministic effective sort key: the ISO-ish effective string sorts
--    chronologically as bytes; a NULL effective falls back to the recording time
--    (hlc_wall → ISO string), an honest lower bound. Format mask is numeric-only so
--    it is locale-independent and identical on every node (§5.1). STABLE (to_char).
CREATE OR REPLACE FUNCTION cairn_dose_effective_sort_key(p_effective text, p_hlc_wall bigint)
RETURNS text LANGUAGE sql STABLE AS $$
    SELECT COALESCE(
        p_effective,
        to_char(to_timestamp(p_hlc_wall / 1000.0) AT TIME ZONE 'UTC', 'YYYY-MM-DD"T"HH24:MI:SS'));
$$;

-- 5. One row per dose POINT: point 0 (seeded from the assert) + one per change. PK =
--    the event's own event_id (immutable content), so a replayed event is idempotent.
CREATE TABLE IF NOT EXISTS medication_dose_event (
    dose_event_id       UUID PRIMARY KEY,
    medication_id       UUID NOT NULL,
    patient_id          UUID NOT NULL,
    amount              TEXT,
    unit                TEXT,
    effective_value     TEXT,
    effective_precision TEXT,
    is_initial          BOOLEAN NOT NULL,
    info_source         TEXT,
    reason              TEXT,
    hlc_wall            BIGINT NOT NULL,
    hlc_counter         INTEGER NOT NULL,
    origin              TEXT NOT NULL,
    content_address     BYTEA NOT NULL,
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
GRANT SELECT ON medication_dose_event TO cairn_agent;
CREATE INDEX IF NOT EXISTS medication_dose_event_med_idx ON medication_dose_event (medication_id);

-- 6. Corrections, keyed by the TARGET dose event they fix (a correction overlays a
--    specific point). HLC-wins if one point is corrected twice; converges if the
--    correction arrives before its target (orphan). Correction TABLE only here; its
--    trigger is added in the correction task.
CREATE TABLE IF NOT EXISTS medication_dose_correction (
    corrected_dose_event_id UUID PRIMARY KEY,
    medication_id           UUID NOT NULL,
    patient_id              UUID NOT NULL,
    amount                  TEXT,
    unit                    TEXT,
    reason                  TEXT,
    info_source             TEXT,
    hlc_wall                BIGINT NOT NULL,
    hlc_counter             INTEGER NOT NULL,
    origin                  TEXT NOT NULL,
    content_address         BYTEA NOT NULL,
    updated_at              TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
GRANT SELECT ON medication_dose_correction TO cairn_agent;

-- 7. Seed point 0 from the assert (a SECOND, additive trigger on the assert type; the
--    slice-1 statement/cessation triggers are untouched). dose_event_id = the assert's
--    event_id; effective = the assert's `started`.
CREATE OR REPLACE FUNCTION medication_dose_seed_initial()
RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE p jsonb := NEW.body;
BEGIN
    INSERT INTO medication_dose_event
        (dose_event_id, medication_id, patient_id, amount, unit,
         effective_value, effective_precision, is_initial, info_source, reason,
         hlc_wall, hlc_counter, origin, content_address)
    VALUES (
        NEW.event_id, (p ->> 'medication_id')::uuid, NEW.patient_id,
        p -> 'dose' ->> 'amount', p -> 'dose' ->> 'unit',
        p -> 'started' ->> 'value', p -> 'started' ->> 'precision',
        TRUE, p ->> 'info_source', NULL,
        NEW.hlc_wall, NEW.hlc_counter, NEW.node_origin, NEW.content_address)
    ON CONFLICT (dose_event_id) DO NOTHING;
    RETURN NULL;
END;
$$;
DROP TRIGGER IF EXISTS medication_dose_seed_initial_trg ON event_log;
CREATE TRIGGER medication_dose_seed_initial_trg
    AFTER INSERT ON event_log
    FOR EACH ROW WHEN (NEW.event_type = 'clinical.medication.asserted')
    EXECUTE FUNCTION medication_dose_seed_initial();

-- 8. Fold a dose change into a new timeline point.
CREATE OR REPLACE FUNCTION medication_dose_change_apply()
RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE p jsonb := NEW.body;
BEGIN
    INSERT INTO medication_dose_event
        (dose_event_id, medication_id, patient_id, amount, unit,
         effective_value, effective_precision, is_initial, info_source, reason,
         hlc_wall, hlc_counter, origin, content_address)
    VALUES (
        NEW.event_id, (p ->> 'medication_id')::uuid, NEW.patient_id,
        p -> 'dose' ->> 'amount', p -> 'dose' ->> 'unit',
        p -> 'effective' ->> 'value', p -> 'effective' ->> 'precision',
        FALSE, p ->> 'info_source', p ->> 'reason',
        NEW.hlc_wall, NEW.hlc_counter, NEW.node_origin, NEW.content_address)
    ON CONFLICT (dose_event_id) DO NOTHING;
    RETURN NULL;
END;
$$;
DROP TRIGGER IF EXISTS medication_dose_change_apply_trg ON event_log;
CREATE TRIGGER medication_dose_change_apply_trg
    AFTER INSERT ON event_log
    FOR EACH ROW WHEN (NEW.event_type = 'clinical.medication-dose-change.asserted')
    EXECUTE FUNCTION medication_dose_change_apply();

-- 9. Effective per-point value = the correction's value IF a correction row exists,
--    ELSE the event's value. Keyed on PRESENCE (not COALESCE) so a correct-to-unknown
--    (correction row with NULL amount) shows unknown, not the stale original.
CREATE OR REPLACE VIEW medication_current_dose AS
SELECT DISTINCT ON (de.medication_id)
    de.medication_id, de.patient_id, de.dose_event_id,
    CASE WHEN corr.corrected_dose_event_id IS NOT NULL THEN corr.amount ELSE de.amount END AS amount,
    CASE WHEN corr.corrected_dose_event_id IS NOT NULL THEN corr.unit   ELSE de.unit   END AS unit,
    de.effective_value, de.effective_precision,
    (corr.corrected_dose_event_id IS NOT NULL) AS corrected
FROM medication_dose_event de
LEFT JOIN medication_dose_correction corr ON corr.corrected_dose_event_id = de.dose_event_id
ORDER BY de.medication_id,
         cairn_dose_effective_sort_key(de.effective_value, de.hlc_wall) COLLATE "C" DESC,
         de.hlc_wall DESC, de.hlc_counter DESC, de.origin COLLATE "C" DESC, de.content_address DESC;
GRANT SELECT ON medication_current_dose TO cairn_agent;

-- 10. The full titration trail, chronological by effective time. Exposes dose_event_id
--     (so a correction can target a point) and the corrected flag.
CREATE OR REPLACE VIEW patient_medication_dose_history AS
SELECT de.medication_id, de.patient_id, de.dose_event_id, de.is_initial,
       CASE WHEN corr.corrected_dose_event_id IS NOT NULL THEN corr.amount ELSE de.amount END AS amount,
       CASE WHEN corr.corrected_dose_event_id IS NOT NULL THEN corr.unit   ELSE de.unit   END AS unit,
       de.effective_value, de.effective_precision, de.info_source, de.reason,
       (corr.corrected_dose_event_id IS NOT NULL) AS corrected,
       to_timestamp(de.hlc_wall / 1000.0) AS recorded_at
FROM medication_dose_event de
LEFT JOIN medication_dose_correction corr ON corr.corrected_dose_event_id = de.dose_event_id
ORDER BY de.medication_id,
         cairn_dose_effective_sort_key(de.effective_value, de.hlc_wall) COLLATE "C" ASC,
         de.hlc_wall ASC, de.hlc_counter ASC, de.origin COLLATE "C" ASC, de.content_address ASC;
GRANT SELECT ON patient_medication_dose_history TO cairn_agent;

-- 11. Rework the current/past views to source the dose from the timeline winner and
--     expose dose_event_id + dose_corrected (appended columns — CREATE OR REPLACE VIEW
--     safe). A thread with NO timeline row (e.g. a pre-slice-2 assert) falls back to the
--     as-asserted statement dose (CASE on cd presence — self-healing, no data migration).
CREATE OR REPLACE VIEW patient_medication_current AS
SELECT pm.medication_id, pm.patient_id, pm.term, pm.inn_code, pm.formulation,
       CASE WHEN cd.medication_id IS NOT NULL THEN cd.amount ELSE pm.dose_amount END AS dose_amount,
       CASE WHEN cd.medication_id IS NOT NULL THEN cd.unit   ELSE pm.dose_unit   END AS dose_unit,
       pm.sig, pm.info_source, pm.started_value, pm.started_precision, pm.asserted_at,
       cd.dose_event_id, cd.corrected AS dose_corrected
FROM patient_medication pm
LEFT JOIN medication_current_dose cd USING (medication_id)
WHERE NOT pm.ceased;
GRANT SELECT ON patient_medication_current TO cairn_agent;

CREATE OR REPLACE VIEW patient_medication_past AS
SELECT pm.medication_id, pm.patient_id, pm.term, pm.inn_code, pm.formulation,
       CASE WHEN cd.medication_id IS NOT NULL THEN cd.amount ELSE pm.dose_amount END AS dose_amount,
       CASE WHEN cd.medication_id IS NOT NULL THEN cd.unit   ELSE pm.dose_unit   END AS dose_unit,
       pm.sig, pm.info_source, pm.started_value, pm.started_precision,
       pm.asserted_at, pm.stopped_value, pm.stopped_precision, pm.reason,
       cd.dose_event_id, cd.corrected AS dose_corrected
FROM patient_medication pm
LEFT JOIN medication_current_dose cd USING (medication_id)
WHERE pm.ceased;
GRANT SELECT ON patient_medication_past TO cairn_agent;
```

> Note: `patient_medication_current` / `patient_medication_past` are defined in db/031 in the SAME column
> order for columns 1–12 (current) / 1–14 (past). Keep those leading columns identical (names + types);
> only the *source* of `dose_amount`/`dose_unit` changes and the trailing `dose_event_id`/`dose_corrected`
> are appended, which `CREATE OR REPLACE VIEW` permits. Verify against db/031 before editing if unsure.

- [ ] **Step 2: Write failing timeline tests** — append to `crates/cairn-node/tests/medication_dose.rs`:

```rust
// helper: the current dose (amount, unit, dose_event_id, corrected) for a thread.
async fn current_dose(c: &Client, med_id: Uuid) -> (Option<String>, Option<String>, Uuid, bool) {
    let r = c
        .query_one(
            "SELECT dose_amount, dose_unit, dose_event_id, dose_corrected \
             FROM patient_medication_current WHERE medication_id = $1::text::uuid",
            &[&med_id.to_string()],
        )
        .await
        .unwrap();
    (
        r.get::<_, Option<String>>(0),
        r.get::<_, Option<String>>(1),
        r.get::<_, Uuid>(2),
        r.get::<_, bool>(3),
    )
}

async fn history_amounts(c: &Client, med_id: Uuid) -> Vec<Option<String>> {
    c.query(
        "SELECT amount FROM patient_medication_dose_history \
         WHERE medication_id = $1::text::uuid",
        &[&med_id.to_string()],
    )
    .await
    .unwrap()
    .iter()
    .map(|r| r.get::<_, Option<String>>(0))
    .collect()
}

#[tokio::test]
async fn assert_seeds_point0_and_it_is_current() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient = Uuid::now_v7();

    let med_id = assert_medication(&c, &sk, &kid, "test-node", patient, &sample_assert())
        .await
        .unwrap();
    let (amt, unit, _de, corrected) = current_dose(&c, med_id).await;
    assert_eq!(amt.as_deref(), Some("40"));
    assert_eq!(unit.as_deref(), Some("mg"));
    assert!(!corrected);
    // history has exactly the initial point.
    assert_eq!(history_amounts(&c, med_id).await, vec![Some("40".to_string())]);
}

#[tokio::test]
async fn change_moves_current_and_keeps_history() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient = Uuid::now_v7();

    let med_id = assert_medication(&c, &sk, &kid, "test-node", patient, &sample_assert())
        .await
        .unwrap();
    change_dose(
        &c,
        &sk,
        &kid,
        "test-node",
        patient,
        med_id,
        &ChangeDoseInput {
            dose_amount: Some("80"),
            dose_unit: Some("mg"),
            effective: Some("2025-06"),
            effective_precision: Some("month"),
            info_source: "clinician-observed",
            reason: Some("titration"),
        },
    )
    .await
    .unwrap();

    let (amt, _u, _de, _corr) = current_dose(&c, med_id).await;
    assert_eq!(amt.as_deref(), Some("80"), "latest effective is current");
    // Both points present, chronological (40 @2024, then 80 @2025-06).
    assert_eq!(
        history_amounts(&c, med_id).await,
        vec![Some("40".to_string()), Some("80".to_string())]
    );
}

#[tokio::test]
async fn backdated_change_does_not_override_later_effective() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient = Uuid::now_v7();

    // assert dose 40 @2024 (point 0), then a real increase to 80 @2025-06.
    let med_id = assert_medication(&c, &sk, &kid, "test-node", patient, &sample_assert())
        .await
        .unwrap();
    change_dose(
        &c, &sk, &kid, "test-node", patient, med_id,
        &ChangeDoseInput { dose_amount: Some("80"), dose_unit: Some("mg"),
            effective: Some("2025-06"), effective_precision: Some("month"),
            info_source: "clinician-observed", reason: None },
    ).await.unwrap();
    // A later-RECORDED but EARLIER-effective backfill ("was 50 back in 2023").
    change_dose(
        &c, &sk, &kid, "test-node", patient, med_id,
        &ChangeDoseInput { dose_amount: Some("50"), dose_unit: Some("mg"),
            effective: Some("2023"), effective_precision: Some("year"),
            info_source: "patient-reported", reason: Some("historical backfill") },
    ).await.unwrap();

    let (amt, _u, _de, _c) = current_dose(&c, med_id).await;
    assert_eq!(amt.as_deref(), Some("80"), "latest EFFECTIVE (2025-06) stays current, not the last recorded");
}

#[tokio::test]
async fn undated_change_becomes_current_over_older_effective() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient = Uuid::now_v7();

    let med_id = assert_medication(&c, &sk, &kid, "test-node", patient, &sample_assert())
        .await
        .unwrap(); // 40 @2024
    // "they upped it, don't know to what or when" — no effective, no amount.
    change_dose(
        &c, &sk, &kid, "test-node", patient, med_id,
        &ChangeDoseInput { dose_amount: None, dose_unit: None,
            effective: None, effective_precision: None,
            info_source: "patient-reported", reason: Some("patient says increased") },
    ).await.unwrap();

    // The undated change's effective key derives from its (later) recording time, so it
    // wins over the 2024 point. Its amount is unknown (NULL) — honestly current.
    let (amt, _u, _de, _c) = current_dose(&c, med_id).await;
    assert_eq!(amt, None, "current dose is honestly unknown after an unquantified increase");
}
```

- [ ] **Step 3: Run — verify the new tests + the Task-4 accept test pass**

Run: `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test -p cairn-node --test medication_dose`
Expected: PASS for `assert_seeds_point0...`, `change_moves_current...`, `backdated_change...`, `undated_change...`, and now `floor_accepts_wellformed...` (the `medication_current_dose` view exists). Correction-specific tests are added in Task 6.

- [ ] **Step 4: Commit**

```bash
git add db/032_medication_dose.sql crates/cairn-node/tests/medication_dose.rs
git commit -m "feat(db): medication dose timeline — point-0 seed, change, current + history views (slice 2)"
```

---

### Task 6: `db/032` correction overlay — trigger + convergence tests

**Files:**
- Modify: `db/032_medication_dose.sql` (add the correction-apply trigger before `COMMIT;`)
- Modify: `crates/cairn-node/tests/medication_dose.rs` (correction tests)

**Interfaces:**
- Consumes: `medication_dose_correction` table (Task 5), `cairn_hlc_overlay_wins` (db/002), the `CASE WHEN corrected` views (Task 5).
- Produces: a correction overlays its target point; correct-to-unknown; orphan correction converges; HLC-wins between two corrections of one point.

> **TDD order:** write the Step-2 correction tests first and run them — RED because the correction-apply trigger doesn't exist yet, so a correction lands in the log but never overlays its point (current dose stays the original). Then add the Step-1 trigger and re-run → GREEN.

- [ ] **Step 1: Add the correction-apply trigger** in `db/032_medication_dose.sql`, before the final `COMMIT;` (after the view definitions is fine — triggers don't depend on views):

```sql
-- 12. Fold a correction as an HLC-winning overlay keyed by the TARGET dose event.
--     Offline-first: no check that the target exists locally (it may replicate later).
CREATE OR REPLACE FUNCTION medication_dose_correction_apply()
RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE p jsonb := NEW.body;
BEGIN
    INSERT INTO medication_dose_correction
        (corrected_dose_event_id, medication_id, patient_id, amount, unit, reason, info_source,
         hlc_wall, hlc_counter, origin, content_address)
    VALUES (
        (p ->> 'corrects')::uuid, (p ->> 'medication_id')::uuid, NEW.patient_id,
        p -> 'dose' ->> 'amount', p -> 'dose' ->> 'unit', p ->> 'reason', p ->> 'info_source',
        NEW.hlc_wall, NEW.hlc_counter, NEW.node_origin, NEW.content_address)
    ON CONFLICT (corrected_dose_event_id) DO UPDATE SET
        medication_id   = EXCLUDED.medication_id,
        patient_id      = EXCLUDED.patient_id,
        amount          = EXCLUDED.amount,
        unit            = EXCLUDED.unit,
        reason          = EXCLUDED.reason,
        info_source     = EXCLUDED.info_source,
        hlc_wall        = EXCLUDED.hlc_wall,
        hlc_counter     = EXCLUDED.hlc_counter,
        origin          = EXCLUDED.origin,
        content_address = EXCLUDED.content_address,
        updated_at      = clock_timestamp()
    WHERE cairn_hlc_overlay_wins(
        EXCLUDED.hlc_wall, EXCLUDED.hlc_counter, EXCLUDED.origin, EXCLUDED.content_address,
        medication_dose_correction.hlc_wall, medication_dose_correction.hlc_counter,
        medication_dose_correction.origin, medication_dose_correction.content_address);
    RETURN NULL;
END;
$$;
DROP TRIGGER IF EXISTS medication_dose_correction_apply_trg ON event_log;
CREATE TRIGGER medication_dose_correction_apply_trg
    AFTER INSERT ON event_log
    FOR EACH ROW WHEN (NEW.event_type = 'clinical.medication-dose-correction.asserted')
    EXECUTE FUNCTION medication_dose_correction_apply();
```

- [ ] **Step 2: Write failing correction tests** — append to `crates/cairn-node/tests/medication_dose.rs`:

```rust
#[tokio::test]
async fn correction_overlays_current_and_sets_flag() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient = Uuid::now_v7();

    let med_id = assert_medication(&c, &sk, &kid, "test-node", patient, &sample_assert())
        .await
        .unwrap(); // point 0 = 40 mg, current
    // Correct the CURRENT dose (target defaults to point 0) to 20 mg.
    let target = resolve_correction_target(&c, med_id, None).await.unwrap();
    correct_dose(
        &c, &sk, &kid, "test-node", patient, med_id, target,
        &CorrectDoseInput { dose_amount: Some("20"), dose_unit: Some("mg"),
            info_source: None, reason: Some("mis-keyed") },
    ).await.unwrap();

    let (amt, _u, _de, corrected) = current_dose(&c, med_id).await;
    assert_eq!(amt.as_deref(), Some("20"), "current dose reflects the correction");
    assert!(corrected, "corrected flag is set");
}

#[tokio::test]
async fn correct_to_unknown_shows_unknown_not_original() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient = Uuid::now_v7();

    let med_id = assert_medication(&c, &sk, &kid, "test-node", patient, &sample_assert())
        .await
        .unwrap(); // 40 mg
    let target = resolve_correction_target(&c, med_id, None).await.unwrap();
    // "the 40 was a guess — strike it, unknown."
    correct_dose(
        &c, &sk, &kid, "test-node", patient, med_id, target,
        &CorrectDoseInput { dose_amount: None, dose_unit: None,
            info_source: None, reason: Some("was a guess") },
    ).await.unwrap();

    let (amt, unit, _de, corrected) = current_dose(&c, med_id).await;
    assert_eq!(amt, None, "correct-to-unknown must NOT fall back to the original 40");
    assert_eq!(unit, None);
    assert!(corrected);
}

#[tokio::test]
async fn orphan_correction_converges_when_target_arrives() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient = Uuid::now_v7();
    let med_id = Uuid::now_v7();

    // Pick a target dose_event_id that does not exist locally yet.
    let future_target = Uuid::now_v7();
    correct_dose(
        &c, &sk, &kid, "test-node", patient, med_id, future_target,
        &CorrectDoseInput { dose_amount: Some("15"), dose_unit: Some("mg"),
            info_source: None, reason: Some("early correction") },
    ).await.unwrap();
    // The correction row exists but no dose point references it yet → no current row.
    let n: i64 = c
        .query_one(
            "SELECT count(*) FROM medication_current_dose WHERE medication_id = $1::text::uuid",
            &[&med_id.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(n, 0, "orphan correction renders nothing until its target arrives");

    // Now inject the assert whose event_id == future_target (build+sign directly to
    // choose the event_id), seeding point 0 that the correction targets.
    let hlc = db::next_hlc(&c, "test-node").await.unwrap();
    let body: EventBody = cairn_node::medication::build_assert_body(
        future_target, med_id, patient, &sample_assert(), &kid, hlc,
    );
    let signed = sign(&body, &sk).unwrap();
    c.execute("SELECT submit_event($1)", &[&signed.signed_bytes])
        .await
        .unwrap();

    let (amt, _u, _de, corrected) = current_dose(&c, med_id).await;
    assert_eq!(amt.as_deref(), Some("15"), "the pre-arrived correction now overlays point 0");
    assert!(corrected);
}

#[tokio::test]
async fn later_correction_of_same_point_wins() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient = Uuid::now_v7();

    let med_id = assert_medication(&c, &sk, &kid, "test-node", patient, &sample_assert())
        .await
        .unwrap();
    let target = resolve_correction_target(&c, med_id, None).await.unwrap();
    correct_dose(&c, &sk, &kid, "test-node", patient, med_id, target,
        &CorrectDoseInput { dose_amount: Some("20"), dose_unit: Some("mg"), info_source: None, reason: None })
        .await.unwrap();
    correct_dose(&c, &sk, &kid, "test-node", patient, med_id, target,
        &CorrectDoseInput { dose_amount: Some("25"), dose_unit: Some("mg"), info_source: None, reason: Some("re-corrected") })
        .await.unwrap();

    let (amt, _u, _de, _c) = current_dose(&c, med_id).await;
    assert_eq!(amt.as_deref(), Some("25"), "the later (higher-HLC) correction of the same point wins");
}
```

Note: `build_assert_body` is the slice-1 function already `pub` in `cairn_node::medication`.

- [ ] **Step 3: Run — verify pass (full dose suite)**

Run: `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test -p cairn-node --test medication_dose`
Expected: PASS — all floor + timeline + correction tests.

- [ ] **Step 4: Verify slice-1 medication tests still pass (no regression from the view rework)**

Run: `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test -p cairn-node --test medication`
Expected: PASS (slice-1 current/past behavior unchanged — dose still shows 40 mg via the timeline point 0).

- [ ] **Step 5: Commit**

```bash
git add db/032_medication_dose.sql crates/cairn-node/tests/medication_dose.rs
git commit -m "feat(db): medication dose correction overlay — targeted, offline-first, HLC-wins (slice 2)"
```

---

### Task 7: CLI — `medication change-dose` / `correct-dose`

**Files:**
- Modify: `crates/cairn-node/src/main.rs` (two `Cmd` variants + two handler arms)

**Interfaces:**
- Consumes: `cairn_node::medication::{ChangeDoseInput, change_dose, CorrectDoseInput, correct_dose, resolve_correction_target}`; the existing CLI helpers `load_signing_key`, `ensure_registration_actor`, `cairn_node::identity::load_local`.

- [ ] **Step 1: Add the `Cmd` variants** — in `crates/cairn-node/src/main.rs`, after the `MedicationCease { ... }` variant (before the closing `}` of the enum at line ~511):

```rust
    /// Record a dose change on an existing medication thread
    /// (clinical.medication-dose-change.asserted). Additive — the prior dose stays in
    /// the history. Offline-first: does not require the thread to be present locally.
    MedicationChangeDose {
        /// The patient UUID the thread belongs to.
        patient: Uuid,
        /// The medication thread id (printed by `medication-assert`).
        medication_id: Uuid,
        /// New dose magnitude (decimal). Omit if unknown ("upped it, dunno to what").
        #[arg(long)]
        dose_amount: Option<String>,
        /// New dose unit (mg, mcg, mL, …, or free-text).
        #[arg(long)]
        dose_unit: Option<String>,
        /// When the dose changed (value, e.g. "2025-06").
        #[arg(long)]
        effective: Option<String>,
        /// Precision token for --effective (year|month|day|year-range).
        #[arg(long)]
        effective_precision: Option<String>,
        /// Who the claim came from: patient-reported | clinician-observed | external-record | unknown.
        #[arg(long, default_value = "unknown")]
        info_source: String,
        /// Optional free-text reason ("titration", "renal dosing").
        #[arg(long)]
        reason: Option<String>,
    },
    /// Correct a wrongly-recorded dose (clinical.medication-dose-correction.asserted).
    /// The prior value stays in the record (audit); this only wins the current dose.
    MedicationCorrectDose {
        /// The patient UUID the thread belongs to.
        patient: Uuid,
        /// The medication thread id.
        medication_id: Uuid,
        /// The dose event to correct. Defaults to the current dose point of the thread.
        #[arg(long)]
        target: Option<Uuid>,
        /// The corrected dose magnitude. Omit to correct to *unknown* (strike a false precision).
        #[arg(long)]
        dose_amount: Option<String>,
        /// The corrected dose unit.
        #[arg(long)]
        dose_unit: Option<String>,
        /// Optional provenance of the correction claim.
        #[arg(long)]
        info_source: Option<String>,
        /// Optional free-text reason ("mis-keyed").
        #[arg(long)]
        reason: Option<String>,
    },
```

- [ ] **Step 2: Add the handler arms** — in the `match cli.cmd` block, after the `Cmd::MedicationCease { ... } => { ... }` arm (~line 1473):

```rust
        Cmd::MedicationChangeDose {
            patient,
            medication_id,
            dose_amount,
            dose_unit,
            effective,
            effective_precision,
            info_source,
            reason,
        } => {
            let node_sk = load_signing_key(&cli.key, true)?;
            let node_kid = hex::encode(node_sk.verifying_key().to_bytes());
            let db = cairn_node::db::connect(&cli.conn).await?;
            let id = cairn_node::identity::load_local(&db).await?;
            ensure_registration_actor(&db, &node_kid).await?;
            let input = cairn_node::medication::ChangeDoseInput {
                dose_amount: dose_amount.as_deref(),
                dose_unit: dose_unit.as_deref(),
                effective: effective.as_deref(),
                effective_precision: effective_precision.as_deref(),
                info_source: &info_source,
                reason: reason.as_deref(),
            };
            let event_id = cairn_node::medication::change_dose(
                &db,
                &node_sk,
                &node_kid,
                &id.node_id_hex,
                patient,
                medication_id,
                &input,
            )
            .await?;
            println!("dose change recorded for thread {medication_id}; event {event_id}");
        }
        Cmd::MedicationCorrectDose {
            patient,
            medication_id,
            target,
            dose_amount,
            dose_unit,
            info_source,
            reason,
        } => {
            let node_sk = load_signing_key(&cli.key, true)?;
            let node_kid = hex::encode(node_sk.verifying_key().to_bytes());
            let db = cairn_node::db::connect(&cli.conn).await?;
            let id = cairn_node::identity::load_local(&db).await?;
            ensure_registration_actor(&db, &node_kid).await?;
            let corrects =
                cairn_node::medication::resolve_correction_target(&db, medication_id, target).await?;
            let input = cairn_node::medication::CorrectDoseInput {
                dose_amount: dose_amount.as_deref(),
                dose_unit: dose_unit.as_deref(),
                info_source: info_source.as_deref(),
                reason: reason.as_deref(),
            };
            let event_id = cairn_node::medication::correct_dose(
                &db,
                &node_sk,
                &node_kid,
                &id.node_id_hex,
                patient,
                medication_id,
                corrects,
                &input,
            )
            .await?;
            println!("dose correction recorded for thread {medication_id} (target {corrects}); event {event_id}");
        }
```

- [ ] **Step 3: Verify the CLI parses (clap)**

Run: `cargo run -p cairn-node -- medication-change-dose --help` and `cargo run -p cairn-node -- medication-correct-dose --help`
Expected: PASS — both print usage (confirming the arg parser accepts the new subcommands). *(Note: clap derives kebab-case subcommand names from the PascalCase variants — `medication-change-dose` / `medication-correct-dose`, matching the existing `medication-assert` / `medication-cease`.)*

- [ ] **Step 4: End-to-end CLI smoke against the test DB** (optional but recommended — mirrors the slice-1 smoke). Using an initialized node dir + key, run `medication-assert`, `medication-change-dose`, then query `patient_medication_current` to confirm the dose moved. Record the commands run in the commit body.

- [ ] **Step 5: Commit**

```bash
git add crates/cairn-node/src/main.rs
git commit -m "feat(cli): medication change-dose + correct-dose subcommands (slice 2)"
```

---

### Task 8: Whole-workspace green + docs (HANDOVER / ROADMAP)

**Files:**
- Modify: `docs/HANDOVER.md`, `docs/ROADMAP.md`

- [ ] **Step 1: Full workspace test + lints**

Run:
```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test --workspace
```
Expected: all PASS (fmt clean, clippy clean, cairn-event + cairn-sync + cairn-node incl. DB-gated `medication` and `medication_dose`).

- [ ] **Step 2: Build the docs site** (confirms no mkdocs breakage from new spec/plan files)

Run: `uv run --with-requirements docs/requirements.txt -- mkdocs build`
Expected: PASS (no warnings-as-errors).

- [ ] **Step 3: Update `docs/HANDOVER.md`** — add a top "This session" block summarizing slice 2 (two verbs, dose timeline, bitemporal current-dose, correction overlay, device-additive, `db/032`, no ADR/spec bump), demote the medication slice-1 block, and move the "dose-correction/change overlay" out of the slice-1 deferred list into "done". Prune to keep the file concise (< 500 lines target).

- [ ] **Step 4: Update `docs/ROADMAP.md`** — under Phase 4, add a slice entry for the medication dose overlay (mirroring the slice-1 entry's style), noting the two additive verbs, the dose-timeline projection, the §5.1 winner rule, and the deferred items (cross-thread reconciliation resolution → slice 3; effective-date/reason correction; #173 twin-dispatch refactor; #157 collision advisory onto the dose projections).

- [ ] **Step 5: Commit**

```bash
git add docs/HANDOVER.md docs/ROADMAP.md
git commit -m "docs(handover,roadmap): medication dose overlay slice 2 built"
```

---

## Notes for the implementer

- **Do not touch `db/031`.** All slice-2 SQL is in `db/032`. The `cairn_event_twin` reproduction in Task 4
  must copy db/031's live body verbatim (it is CREATE-OR-REPLACE and db/032 loads last); if db/031's body
  has drifted from what is shown here, copy the *current* db/031 body and add only the two dose branches.
- **`corrects` is deliberately NOT `target_event_id`.** The built-in `target_event_id` path in `submit_event`
  (db/005) forces local existence and is gated on `targets_other_author=TRUE`; using it would break
  offline-first. Keep the distinct `corrects` field, validated as a UUID only.
- **`patient_medication_current` / `_past` column order** must match db/031 for the leading columns
  (CREATE OR REPLACE VIEW only permits appending). Read db/031's definitions before editing.
- **`#157` / `#173` are out of scope** — do not extend the collision advisory onto the dose projections or
  refactor the twin dispatch here; both are recorded as deferred.
```
