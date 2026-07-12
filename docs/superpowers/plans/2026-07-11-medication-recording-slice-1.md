# Medication Recording (slice 1) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the first clinical-content event stream on `cairn-node` — recording current and past medication for a patient via two append-only verbs (`clinical.medication.asserted` + `clinical.medication-cessation.asserted`) over an immortal `medication_id` thread, with an in-DB safety floor, a current/past projection, and a deterministic advisory reconciliation flag.

**Architecture:** Mirrors the existing demographics/identity slice structure exactly: pure `cairn-event` builders (return the `payload` JSON `Value`, no I/O, no UUID minting) → an in-DB floor in `db/031_medication.sql` (fail-closed event-type registry + a structural-check function + the shared `cairn_event_twin` hook) → two projection tables folded by `AFTER INSERT` triggers using the shared `cairn_hlc_overlay_wins` predicate → read VIEWs → a `cairn-node` orchestrator (`assert_medication` / `cease_medication`) that mints ids, ticks the HLC, signs with the node key (device-additive), and calls `submit_event` → flat CLI verbs.

**Tech Stack:** Rust (workspace), `serde_json`, `tokio-postgres` (0.7, `NoTls`, no sqlx/pool), `clap` v4 derive, `cairn-event` (Ed25519/COSE signing), PostgreSQL 18 with the `cairn_pgx` in-DB floor extension.

## Global Constraints

- **License:** AGPL-3.0. Add no new external dependencies (this slice needs none). — verbatim from `CLAUDE.md`.
- **TDD:** failing test first, then minimal code. No production code without a driving test. — `CLAUDE.md` house rule 2.
- **Reviewer-legible, junior-documented:** every non-trivial fn/module carries a *why/how-it-fits* comment. — house rules 3, 4; §9 for the safety-critical surface.
- **Never hard-code crypto material in tests:** derive at runtime — `cairn_event::generate_key()` for random keypairs, `std::array::from_fn(|i| …)` for deterministic seeds. Never byte-array/string literals. — house rule 6 / issue #146.
- **Additive-only wire:** new optional event-body fields are omitted when absent (never serialized as `null`); the `body` builders insert optional keys only when `Some`. — principle 11.
- **Projection determinism:** every TEXT tiebreak key in an overlay/ORDER BY is pinned `COLLATE "C"`. — ADR-0045. (Reuse `cairn_hlc_overlay_wins`, which already does this.)
- **DB-gated tests** require a local Postgres 18 with the `cairn_pgx` extension installed, reached via the `CAIRN_TEST_PG` keyword connection string. Local substrate (per project memory): `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test"`. Tests self-skip (pass) when `CAIRN_TEST_PG` is unset.
- **Branch:** `feat/medication-recording-slice-1` (already created, design doc committed).

## File Structure

- **Create** `crates/cairn-event/src/medication.rs` — pure builders + twins for both verbs. One responsibility: shape the two medication payloads and their legibility twins.
- **Modify** `crates/cairn-event/src/lib.rs` — add `pub mod medication;` (one line).
- **Create** `db/031_medication.sql` — the entire in-DB medication surface: event-type registration, structural floor, the `cairn_event_twin` extension, two projection tables + triggers, and the read VIEWs. One responsibility: the medication floor + projection.
- **Modify** `crates/cairn-node/src/db.rs` — add one `SCHEMA` slice entry for `031_medication` (one line).
- **Create** `crates/cairn-node/src/medication.rs` — the node authoring orchestrators (`assert_medication`, `cease_medication`) + their `build_*_body` helpers + `validate_term`.
- **Modify** `crates/cairn-node/src/lib.rs` — add `pub mod medication;` (one line).
- **Modify** `crates/cairn-node/src/main.rs` — add the `MedicationAssert` + `MedicationCease` CLI variants and their dispatch arms (flat verbs, matching house style).
- **Create** `crates/cairn-node/tests/medication.rs` — DB-gated integration tests.
- **Modify** `docs/HANDOVER.md`, `docs/ROADMAP.md` (if present) — currency update, bundled into this PR (project convention).

**Payload shapes (the day-one signed contract).** `event_log.body` stores the *payload only* (`submit_event` inserts `body = b -> 'payload'`), and `patient_id` is a top-level `event_log` column — so triggers read `NEW.body ->> 'medication_id'` and `NEW.patient_id`, and the floor check (which sees the full body) reads `b -> 'payload' -> …`.

```
clinical.medication.asserted   payload:
  { "medication_id": "<uuid>",
    "substance": { "term": "<non-empty>", "inn_code"?: "<id>", "formulation"?: "<enum>" },
    "dose"?: { "amount"?: "<decimal-as-string>", "unit"?: "<DoseUnit|free-text>" },
    "sig"?: "<free text>",
    "info_source": "patient-reported|clinician-observed|external-record|unknown",
    "started"?: { "value": "<date-or-range>", "precision"?: "<token>" } }

clinical.medication-cessation.asserted   payload:
  { "medication_id": "<uuid>",
    "stopped"?: { "value": "<date>", "precision"?: "<token>" },
    "reason"?: "<free text>" }
```

Only `substance.term` (non-empty) and `info_source` (non-empty, value-open) are floor-mandatory on an assert; only a valid `medication_id` on either verb. Everything else is omitted-when-unknown.

---

### Task 1: `cairn-event::medication` — pure builders + twins

**Files:**
- Create: `crates/cairn-event/src/medication.rs`
- Modify: `crates/cairn-event/src/lib.rs` (add `pub mod medication;` alongside the other `pub mod` lines near line 33)
- Test: inline `#[cfg(test)] mod tests` in `crates/cairn-event/src/medication.rs`

**Interfaces:**
- Consumes: `serde_json::{json, Value}` only. No other task.
- Produces (later tasks depend on these exact names/signatures):
  - `struct MedicationAssertion<'a>` with fields `medication_id, term: &'a str`, `inn_code, formulation, dose_amount, dose_unit, sig: Option<&'a str>`, `info_source: &'a str`, `started, started_precision: Option<&'a str>`.
  - `pub fn medication_assertion_body(a: &MedicationAssertion) -> Value`
  - `pub fn render_medication_twin(a: &MedicationAssertion) -> String`
  - `struct MedicationCessation<'a>` with fields `medication_id: &'a str`, `stopped, stopped_precision, reason: Option<&'a str>`.
  - `pub fn medication_cessation_body(c: &MedicationCessation) -> Value`
  - `pub fn render_medication_cessation_twin(c: &MedicationCessation) -> String`

- [ ] **Step 1: Add the module declaration**

In `crates/cairn-event/src/lib.rs`, add this line to the existing `pub mod` block (near line 33, keep alphabetical-ish order after `john_doe`):

```rust
pub mod medication;
```

- [ ] **Step 2: Write the failing tests** in a new file `crates/cairn-event/src/medication.rs`

```rust
//! §3.15/§3.16 medication recording — the first clinical-content builders.
//!
//! Pure: no clock, no randomness, no I/O. The cairn-node edge mints the ids,
//! stamps the HLC, and signs; these functions only shape the `payload` JSON that
//! becomes `EventBody.payload`. Optional fields are inserted only when present —
//! never serialized as null — so an added-later field never changes an existing
//! event's content address (principle 11, the demographics idiom).
//!
//! Two verbs over an immortal `medication_id` thread: an *assertion* (the patient
//! takes/took a substance) mints the thread; a *cessation* references it and ends
//! it. The only floor-mandatory clinical field is `substance.term` — everything
//! else is an honest *unknown* (principle 4), because the realistic ED history is
//! "some blood thinner, unsure which, dose unknown".

use serde_json::{json, Value};

/// A medication statement (the "start" verb). `term` is the one mandatory
/// clinical field (may be vague, e.g. "little white pill"); every `Option`
/// field is omitted from the payload when `None`.
pub struct MedicationAssertion<'a> {
    /// Immortal thread id the caller mints; a later cessation references it.
    pub medication_id: &'a str,
    /// As-asserted substance term — mandatory, non-empty.
    pub term: &'a str,
    /// Stable INN anchor; `None` = not-yet-coded (usual in slice 1, no dictionary).
    pub inn_code: Option<&'a str>,
    /// Formulation enum token (tablet, capsule, liquid, patch, …) or `None` = unknown.
    pub formulation: Option<&'a str>,
    /// Dose magnitude as a decimal string; `None` = unknown.
    pub dose_amount: Option<&'a str>,
    /// Dose unit (a small controlled token or a free-text long-tail value); `None` = unknown.
    pub dose_unit: Option<&'a str>,
    /// Free-text directions ("one BD", "PRN"); `None` = unknown.
    pub sig: Option<&'a str>,
    /// Provenance of the *claim* (who said it) — distinct from event authorship.
    /// Required-present, value-open: patient-reported|clinician-observed|external-record|unknown.
    pub info_source: &'a str,
    /// Uncertainty-capable start date value ("2024", "2024-03", "2020/2024"); `None` = unknown.
    pub started: Option<&'a str>,
    /// Precision token for `started` (year|month|day|year-range); only meaningful when `started` is Some.
    pub started_precision: Option<&'a str>,
}

/// Build the `clinical.medication.asserted` payload. Mirrors the demographics
/// `*_body` idiom: a `json!` skeleton of the always-present fields, then optional
/// keys inserted only when `Some`.
pub fn medication_assertion_body(a: &MedicationAssertion) -> Value {
    let mut substance = json!({ "term": a.term });
    {
        let s = substance.as_object_mut().expect("json! built an object");
        if let Some(c) = a.inn_code {
            s.insert("inn_code".into(), json!(c));
        }
        if let Some(f) = a.formulation {
            s.insert("formulation".into(), json!(f));
        }
    }
    let mut p = json!({
        "medication_id": a.medication_id,
        "substance": substance,
        "info_source": a.info_source,
    });
    let obj = p.as_object_mut().expect("json! built an object");
    if a.dose_amount.is_some() || a.dose_unit.is_some() {
        let mut dose = json!({});
        let d = dose.as_object_mut().expect("json! built an object");
        if let Some(amt) = a.dose_amount {
            d.insert("amount".into(), json!(amt));
        }
        if let Some(u) = a.dose_unit {
            d.insert("unit".into(), json!(u));
        }
        obj.insert("dose".into(), dose);
    }
    if let Some(s) = a.sig {
        obj.insert("sig".into(), json!(s));
    }
    if let Some(v) = a.started {
        let mut started = json!({ "value": v });
        if let Some(pr) = a.started_precision {
            started
                .as_object_mut()
                .expect("json! built an object")
                .insert("precision".into(), json!(pr));
        }
        obj.insert("started".into(), started);
    }
    p
}

/// The §3.13/§3.15 legibility twin for a medication statement — a mechanically
/// derived, honest one-line rendering. Non-empty because `term` is non-empty.
pub fn render_medication_twin(a: &MedicationAssertion) -> String {
    let mut s = String::from(a.term);
    match (a.dose_amount, a.dose_unit) {
        (Some(amt), Some(u)) => s.push_str(&format!(" {amt} {u}")),
        (Some(amt), None) => s.push_str(&format!(" {amt}")),
        _ => {}
    }
    if let Some(f) = a.formulation {
        s.push_str(&format!(" {f}"));
    }
    if let Some(sig) = a.sig {
        s.push_str(&format!(" — {sig}"));
    }
    s.push_str(&format!(" ({})", a.info_source));
    if let Some(v) = a.started {
        s.push_str(&format!(", started {v}"));
    }
    s
}

/// A medication cessation (the "stop" verb). Carries only the thread id it ends,
/// plus an optional uncertainty-capable stop date and reason.
pub struct MedicationCessation<'a> {
    pub medication_id: &'a str,
    pub stopped: Option<&'a str>,
    pub stopped_precision: Option<&'a str>,
    pub reason: Option<&'a str>,
}

/// Build the `clinical.medication-cessation.asserted` payload.
pub fn medication_cessation_body(c: &MedicationCessation) -> Value {
    let mut p = json!({ "medication_id": c.medication_id });
    let obj = p.as_object_mut().expect("json! built an object");
    if let Some(v) = c.stopped {
        let mut stopped = json!({ "value": v });
        if let Some(pr) = c.stopped_precision {
            stopped
                .as_object_mut()
                .expect("json! built an object")
                .insert("precision".into(), json!(pr));
        }
        obj.insert("stopped".into(), stopped);
    }
    if let Some(r) = c.reason {
        obj.insert("reason".into(), json!(r));
    }
    p
}

/// The legibility twin for a cessation. The drug name lives on the assertion, not
/// the cessation (which only references the thread), so this is deliberately terse
/// but always non-empty.
pub fn render_medication_cessation_twin(c: &MedicationCessation) -> String {
    let mut s = String::from("Ceased medication");
    if let Some(v) = c.stopped {
        s.push_str(&format!(" (stopped {v})"));
    }
    if let Some(r) = c.reason {
        s.push_str(&format!(" — {r}"));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn full_assertion() -> MedicationAssertion<'static> {
        MedicationAssertion {
            medication_id: "11111111-1111-7111-8111-111111111111",
            term: "atorvastatin",
            inn_code: Some("INN:atorvastatin"),
            formulation: Some("tablet"),
            dose_amount: Some("40"),
            dose_unit: Some("mg"),
            sig: Some("one BD"),
            info_source: "patient-reported",
            started: Some("2024"),
            started_precision: Some("year"),
        }
    }

    #[test]
    fn assertion_body_carries_all_present_fields() {
        let v = medication_assertion_body(&full_assertion());
        assert_eq!(v["medication_id"], "11111111-1111-7111-8111-111111111111");
        assert_eq!(v["substance"]["term"], "atorvastatin");
        assert_eq!(v["substance"]["inn_code"], "INN:atorvastatin");
        assert_eq!(v["substance"]["formulation"], "tablet");
        assert_eq!(v["dose"]["amount"], "40");
        assert_eq!(v["dose"]["unit"], "mg");
        assert_eq!(v["sig"], "one BD");
        assert_eq!(v["info_source"], "patient-reported");
        assert_eq!(v["started"]["value"], "2024");
        assert_eq!(v["started"]["precision"], "year");
    }

    #[test]
    fn assertion_body_omits_absent_optionals_never_null() {
        // The "little white pill, don't know anything else" floor case.
        let a = MedicationAssertion {
            medication_id: "22222222-2222-7222-8222-222222222222",
            term: "little white pill",
            inn_code: None,
            formulation: None,
            dose_amount: None,
            dose_unit: None,
            sig: None,
            info_source: "patient-reported",
            started: None,
            started_precision: None,
        };
        let v = medication_assertion_body(&a);
        let subst = v["substance"].as_object().unwrap();
        assert!(!subst.contains_key("inn_code"), "absent inn_code must be omitted, not null");
        assert!(!subst.contains_key("formulation"));
        let obj = v.as_object().unwrap();
        assert!(!obj.contains_key("dose"), "absent dose must be omitted entirely");
        assert!(!obj.contains_key("sig"));
        assert!(!obj.contains_key("started"));
        assert_eq!(v["substance"]["term"], "little white pill");
        assert_eq!(v["info_source"], "patient-reported");
    }

    #[test]
    fn assertion_body_dose_amount_only_omits_unit() {
        let mut a = full_assertion();
        a.dose_unit = None;
        let v = medication_assertion_body(&a);
        assert_eq!(v["dose"]["amount"], "40");
        assert!(!v["dose"].as_object().unwrap().contains_key("unit"));
    }

    #[test]
    fn assertion_twin_is_nonempty_and_reads_naturally() {
        let s = render_medication_twin(&full_assertion());
        assert!(s.contains("atorvastatin"));
        assert!(s.contains("40 mg"));
        assert!(s.contains("(patient-reported)"));
        assert!(s.contains("started 2024"));
        assert!(!s.trim().is_empty());
    }

    #[test]
    fn assertion_twin_nonempty_for_vague_term_only() {
        let a = MedicationAssertion {
            medication_id: "22222222-2222-7222-8222-222222222222",
            term: "little white pill",
            inn_code: None,
            formulation: None,
            dose_amount: None,
            dose_unit: None,
            sig: None,
            info_source: "patient-reported",
            started: None,
            started_precision: None,
        };
        let s = render_medication_twin(&a);
        assert!(s.starts_with("little white pill"));
        assert!(!s.trim().is_empty());
    }

    #[test]
    fn cessation_body_carries_and_omits_correctly() {
        let full = MedicationCessation {
            medication_id: "11111111-1111-7111-8111-111111111111",
            stopped: Some("2025-06"),
            stopped_precision: Some("month"),
            reason: Some("switched agent"),
        };
        let v = medication_cessation_body(&full);
        assert_eq!(v["medication_id"], "11111111-1111-7111-8111-111111111111");
        assert_eq!(v["stopped"]["value"], "2025-06");
        assert_eq!(v["stopped"]["precision"], "month");
        assert_eq!(v["reason"], "switched agent");

        let bare = MedicationCessation {
            medication_id: "11111111-1111-7111-8111-111111111111",
            stopped: None,
            stopped_precision: None,
            reason: None,
        };
        let vb = medication_cessation_body(&bare);
        let obj = vb.as_object().unwrap();
        assert!(!obj.contains_key("stopped"), "absent stopped omitted, not null");
        assert!(!obj.contains_key("reason"));
        assert_eq!(vb["medication_id"], "11111111-1111-7111-8111-111111111111");
    }

    #[test]
    fn cessation_twin_is_nonempty() {
        let bare = MedicationCessation {
            medication_id: "11111111-1111-7111-8111-111111111111",
            stopped: None,
            stopped_precision: None,
            reason: None,
        };
        assert!(!render_medication_cessation_twin(&bare).trim().is_empty());
    }
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test -p cairn-event medication`
Expected: FAIL to **compile** first (the module referenced by `lib.rs` but functions not yet reachable) — this is the expected red. If you wrote the module body in Step 2, the tests should compile and PASS immediately; that is acceptable here because the module and its tests were authored together (a pure-function module). To honor red→green, temporarily stub one function (e.g. make `medication_assertion_body` return `json!({})`), run to see the FAIL, then restore.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p cairn-event medication`
Expected: PASS (7 tests). Then:

Run: `cargo fmt -p cairn-event && cargo clippy -p cairn-event -- -D warnings`
Expected: no diffs, no warnings.

- [ ] **Step 5: Commit**

```bash
git add crates/cairn-event/src/medication.rs crates/cairn-event/src/lib.rs
git commit -m "feat(cairn-event): medication builders — assert + cessation payloads and twins

Pure §3.15/§3.16 builders for the first clinical-content event stream. Only
substance.term is mandatory; every other field omitted-when-unknown (principle 4).

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 2: Assert path end-to-end — floor + statement projection + `assert_medication`

Delivers the full in-DB floor for **both** verbs (cheap to do once), the `medication_statement` table + trigger, an assert-only `patient_medication_current` view, and the `assert_medication` orchestrator. Driven by DB-gated tests that a valid assert becomes current and an empty-term assert is rejected by the floor.

**Files:**
- Create: `db/031_medication.sql`
- Modify: `crates/cairn-node/src/db.rs` (add the `SCHEMA` entry)
- Create: `crates/cairn-node/src/medication.rs`
- Modify: `crates/cairn-node/src/lib.rs` (add `pub mod medication;`)
- Create: `crates/cairn-node/tests/medication.rs`

**Interfaces:**
- Consumes: `cairn_event::medication::{MedicationAssertion, medication_assertion_body, render_medication_twin}` (Task 1); `cairn_event::{sign, EventBody, Hlc, SigningKey}`; `crate::db::next_hlc`.
- Produces:
  - Rust: `struct AssertMedicationInput<'a>`; `pub fn build_assert_body(event_id: Uuid, medication_id: Uuid, patient: Uuid, input: &AssertMedicationInput, node_kid: &str, hlc: Hlc) -> EventBody`; `pub fn validate_term(term: &str) -> anyhow::Result<()>`; `pub async fn assert_medication(client: &tokio_postgres::Client, node_sk: &SigningKey, node_kid: &str, node_origin: &str, patient: Uuid, input: &AssertMedicationInput) -> anyhow::Result<Uuid>` (returns the minted `medication_id`).
  - SQL: event types `clinical.medication.asserted` / `clinical.medication-cessation.asserted` registered; `cairn_check_medication_assertion(text, jsonb)`; table `medication_statement`; view `patient_medication_current`.

- [ ] **Step 1: Write `db/031_medication.sql` (floor + statement table/trigger + assert-only view)**

**IMPORTANT — the `cairn_event_twin` hook:** open `db/025_identity_repudiate.sql`, find the `CREATE OR REPLACE FUNCTION cairn_event_twin(p_type text, b jsonb)` (around line 96–131), and copy its **entire body verbatim** into the block below where marked, then add the one medication `ELSIF` branch shown. This is required because that function is CREATE-OR-REPLACE and db/031 loads last — copying an older body would silently drop the dispute/identity/repudiate branches.

```sql
-- 031_medication.sql — the first clinical-content surface (§3.15/§3.16).
--
-- Two append-only verbs over an immortal medication_id thread:
--   clinical.medication.asserted            — patient takes/took a substance (mints the thread)
--   clinical.medication-cessation.asserted  — the thread is no longer taken (references it)
--
-- Safety floor (the only hard invariants): an assertion must carry a non-empty
-- substance.term and a non-empty info_source; both verbs must carry a valid
-- medication_id uuid. Everything else is honest-unknown (principle 4) — the floor
-- never blocks a medication write beyond these. Duplicates are ALLOWED (two
-- statements do exist); duplicate *detection* is the advisory projection's job.
--
-- event_log.body IS the payload (submit_event inserts body = b->'payload'); patient_id
-- is a top-level column. So the floor check (sees the full body b) reads b->'payload',
-- while the projection triggers (see NEW.body = the payload) read NEW.body directly.
BEGIN;

-- 1. Register both types in the fail-closed classification registry. Additive,
--    never targeting another author.
INSERT INTO event_type_class (event_type, mode, targets_other_author) VALUES
    ('clinical.medication.asserted',           'additive', FALSE),
    ('clinical.medication-cessation.asserted', 'additive', FALSE)
ON CONFLICT (event_type) DO NOTHING;

-- 2. The structural floor for both verbs. RAISE EXCEPTION per violation.
CREATE OR REPLACE FUNCTION cairn_check_medication_assertion(p_type text, b jsonb)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    p jsonb := b -> 'payload';
BEGIN
    IF p IS NULL THEN
        RAISE EXCEPTION 'medication assertion: missing payload';
    END IF;
    -- medication_id is the thread key on BOTH verbs.
    IF jsonb_typeof(p -> 'medication_id') IS DISTINCT FROM 'string' THEN
        RAISE EXCEPTION 'medication assertion: medication_id must be a uuid string';
    END IF;
    BEGIN
        PERFORM (p ->> 'medication_id')::uuid;
    EXCEPTION WHEN others THEN
        RAISE EXCEPTION 'medication assertion: medication_id must be a valid uuid';
    END;
    -- The start verb carries the clinical floor: a non-empty term + present info_source.
    IF p_type = 'clinical.medication.asserted' THEN
        IF jsonb_typeof(p -> 'substance' -> 'term') IS DISTINCT FROM 'string'
           OR length(btrim(p -> 'substance' ->> 'term')) = 0 THEN
            RAISE EXCEPTION 'medication assertion: substance.term must be a non-empty string (principle 4 floor)';
        END IF;
        IF jsonb_typeof(p -> 'info_source') IS DISTINCT FROM 'string'
           OR length(btrim(p ->> 'info_source')) = 0 THEN
            RAISE EXCEPTION 'medication assertion: info_source must be a non-empty string';
        END IF;
    END IF;
    -- The cessation verb carries only medication_id (+ optional stopped/reason) — done.
END;
$$;

-- 3. Extend the shared twin hook. COPY the ENTIRE live body of cairn_event_twin from
--    db/025_identity_repudiate.sql VERBATIM here, then add ONLY the marked medication
--    branch inside the IF/ELSIF chain (immediately before the final `END IF;`):
--
--        ELSIF p_type IN ('clinical.medication.asserted', 'clinical.medication-cessation.asserted') THEN
--            PERFORM cairn_check_medication_assertion(p_type, b);
--            v_twin_required := 'medication assertion requires a non-empty authored twin (§3.13/§3.15)';
--
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

-- 4. Projection table: one row per asserted thread. Overlay columns (hlc/origin/
--    content_address) let a replayed/duplicate assert converge deterministically.
CREATE TABLE IF NOT EXISTS medication_statement (
    medication_id     UUID PRIMARY KEY,
    patient_id        UUID NOT NULL,
    term              TEXT NOT NULL,
    inn_code          TEXT,
    formulation       TEXT,
    dose_amount       TEXT,
    dose_unit         TEXT,
    sig               TEXT,
    info_source       TEXT NOT NULL,
    started_value     TEXT,
    started_precision TEXT,
    hlc_wall          BIGINT NOT NULL,
    hlc_counter       INTEGER NOT NULL,
    origin            TEXT NOT NULL,
    content_address   BYTEA NOT NULL,
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
GRANT SELECT ON medication_statement TO cairn_agent;
CREATE INDEX IF NOT EXISTS medication_statement_patient_idx ON medication_statement (patient_id);

-- 5. Fold clinical.medication.asserted into medication_statement. NEW.body is the
--    payload; patient_id is a column. Overlay-winner keeps set-union convergence.
CREATE OR REPLACE FUNCTION medication_statement_apply()
RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE
    p jsonb := NEW.body;
BEGIN
    INSERT INTO medication_statement
        (medication_id, patient_id, term, inn_code, formulation,
         dose_amount, dose_unit, sig, info_source, started_value, started_precision,
         hlc_wall, hlc_counter, origin, content_address)
    VALUES (
        (p ->> 'medication_id')::uuid, NEW.patient_id,
        p -> 'substance' ->> 'term',
        p -> 'substance' ->> 'inn_code',
        p -> 'substance' ->> 'formulation',
        p -> 'dose' ->> 'amount',
        p -> 'dose' ->> 'unit',
        p ->> 'sig',
        p ->> 'info_source',
        p -> 'started' ->> 'value',
        p -> 'started' ->> 'precision',
        NEW.hlc_wall, NEW.hlc_counter, NEW.node_origin, NEW.content_address)
    ON CONFLICT (medication_id) DO UPDATE SET
        patient_id        = EXCLUDED.patient_id,
        term              = EXCLUDED.term,
        inn_code          = EXCLUDED.inn_code,
        formulation       = EXCLUDED.formulation,
        dose_amount       = EXCLUDED.dose_amount,
        dose_unit         = EXCLUDED.dose_unit,
        sig               = EXCLUDED.sig,
        info_source       = EXCLUDED.info_source,
        started_value     = EXCLUDED.started_value,
        started_precision = EXCLUDED.started_precision,
        hlc_wall          = EXCLUDED.hlc_wall,
        hlc_counter       = EXCLUDED.hlc_counter,
        origin            = EXCLUDED.origin,
        content_address   = EXCLUDED.content_address,
        updated_at        = clock_timestamp()
    WHERE cairn_hlc_overlay_wins(
        EXCLUDED.hlc_wall, EXCLUDED.hlc_counter, EXCLUDED.origin, EXCLUDED.content_address,
        medication_statement.hlc_wall, medication_statement.hlc_counter,
        medication_statement.origin, medication_statement.content_address);
    RETURN NULL;  -- AFTER trigger
END;
$$;
DROP TRIGGER IF EXISTS medication_statement_apply_trg ON event_log;
CREATE TRIGGER medication_statement_apply_trg
    AFTER INSERT ON event_log
    FOR EACH ROW WHEN (NEW.event_type = 'clinical.medication.asserted')
    EXECUTE FUNCTION medication_statement_apply();

-- 6. Assert-only current view (Task 3 replaces this with the cessation join).
CREATE OR REPLACE VIEW patient_medication_current AS
SELECT medication_id, patient_id, term, inn_code, formulation,
       dose_amount, dose_unit, sig, info_source, started_value, started_precision,
       updated_at AS asserted_at
FROM medication_statement;
GRANT SELECT ON patient_medication_current TO cairn_agent;

COMMIT;
```

- [ ] **Step 2: Register the migration** in `crates/cairn-node/src/db.rs` — add this as the last entry of the `SCHEMA` slice (after the `030_john_doe_local_ordinal` line):

```rust
    ("031_medication", include_str!("../../../db/031_medication.sql")),
```

- [ ] **Step 3: Write the failing test file** `crates/cairn-node/tests/medication.rs`

```rust
//! §3.15 medication recording — DB-gated on $CAIRN_TEST_PG, serialized cluster-wide
//! via db::test_serial_guard (shared-DB + TRUNCATE pattern, like identify.rs).
//! Patients need no pre-existence (offline-first: no patient FK), so tests use a
//! bare Uuid as the patient. Key material is derived at runtime (generate_key).
use cairn_event::medication::MedicationAssertion;
use cairn_event::{generate_key, sign, EventBody, SigningKey};
use cairn_node::db;
use cairn_node::medication::{assert_medication, build_assert_body, AssertMedicationInput};
use tokio_postgres::Client;
use uuid::Uuid;

fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

/// Truncate the log + medication projections and enroll a fresh device actor.
async fn setup_node(c: &Client) -> (SigningKey, String) {
    c.batch_execute("TRUNCATE event_log, actor_event, patient_chart CASCADE")
        .await
        .unwrap();
    c.batch_execute(
        "DO $$ BEGIN \
           IF to_regclass('public.medication_statement') IS NOT NULL THEN TRUNCATE medication_statement; END IF; \
           IF to_regclass('public.medication_cessation') IS NOT NULL THEN TRUNCATE medication_cessation; END IF; \
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

fn sample_input() -> AssertMedicationInput<'static> {
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

async fn current_terms(c: &Client, patient: Uuid) -> Vec<String> {
    c.query(
        "SELECT term FROM patient_medication_current WHERE patient_id = $1 ORDER BY term",
        &[&patient],
    )
    .await
    .unwrap()
    .iter()
    .map(|r| r.get::<_, String>(0))
    .collect()
}

#[tokio::test]
async fn assert_appears_as_current() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient = Uuid::now_v7();

    let med_id = assert_medication(&c, &sk, &kid, "test-node", patient, &sample_input())
        .await
        .unwrap();
    assert_eq!(current_terms(&c, patient).await, vec!["atorvastatin".to_string()]);

    // The thread id is a real minted uuid.
    let n: i64 = c
        .query_one(
            "SELECT count(*) FROM medication_statement WHERE medication_id = $1",
            &[&med_id],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(n, 1);
}

#[tokio::test]
async fn empty_term_is_rejected_by_the_floor() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient = Uuid::now_v7();

    // Bypass the Rust validate_term guard: hand-build a whitespace-only-term event
    // and submit it directly, proving the DB FLOOR rejects it (defense in depth).
    let mut input = sample_input();
    input.term = "   ";
    // Use a real HLC tick so the ONLY rejection reason is the empty term (not an
    // HLC regression against node state).
    let hlc = db::next_hlc(&c, "test-node").await.unwrap();
    let body: EventBody = build_assert_body(Uuid::now_v7(), Uuid::now_v7(), patient, &input, &kid, hlc);
    let signed = sign(&body, &sk).unwrap();
    let res = c
        .execute("SELECT submit_event($1)", &[&signed.signed_bytes])
        .await;
    let err = res.unwrap_err().to_string();
    assert!(err.contains("term"), "floor must reject empty term, got: {err}");
    assert!(current_terms(&c, patient).await.is_empty());
}

#[tokio::test]
async fn validate_term_rejects_blank() {
    // Pure guard test — no DB needed.
    assert!(cairn_node::medication::validate_term("  ").is_err());
    assert!(cairn_node::medication::validate_term("aspirin").is_ok());
}
```

- [ ] **Step 4: Run the tests to verify they fail**

Run: `cargo test -p cairn-node --test medication`
Expected: FAIL to compile — `cairn_node::medication` module does not exist yet.

- [ ] **Step 5: Create the orchestrator** `crates/cairn-node/src/medication.rs`

```rust
//! §3.15 medication recording — the node authoring surface (the first clinical
//! content written on this node). Device-additive: signed by the node/clinician
//! key with a `recorded` contributor and NO responsibility attestation in slice 1
//! (mirrors identify.rs). Two orchestrators — assert (mints a thread) and cease
//! (references it). Offline-first: cease does NOT require the assert to be present
//! locally (a cessation may legitimately be authored before its assert replicates).
use cairn_event::medication::{
    medication_assertion_body, medication_cessation_body, render_medication_cessation_twin,
    render_medication_twin, MedicationAssertion, MedicationCessation,
};
use cairn_event::{sign, EventBody, Hlc, SigningKey};
use uuid::Uuid;

const MEDICATION_SCHEMA_VERSION: &str = "clinical.medication/1";
const MEDICATION_CESSATION_SCHEMA_VERSION: &str = "clinical.medication-cessation/1";

/// The clinician-supplied fields of a medication statement. `term` is required;
/// everything else is an honest Option (unknown when None).
pub struct AssertMedicationInput<'a> {
    pub term: &'a str,
    pub inn_code: Option<&'a str>,
    pub formulation: Option<&'a str>,
    pub dose_amount: Option<&'a str>,
    pub dose_unit: Option<&'a str>,
    pub sig: Option<&'a str>,
    pub info_source: &'a str,
    pub started: Option<&'a str>,
    pub started_precision: Option<&'a str>,
}

/// Advisory Rust-side guard mirroring the DB floor: refuse a blank term with a
/// clinical message. The DB floor is the real, unbypassable enforcement.
pub fn validate_term(term: &str) -> anyhow::Result<()> {
    if term.trim().is_empty() {
        anyhow::bail!("medication term must not be empty: record WHAT the patient takes (even if vague)");
    }
    Ok(())
}

/// Assemble the signed `clinical.medication.asserted` EventBody. Pure — the caller
/// mints `event_id`/`medication_id`, supplies the HLC, and signs.
pub fn build_assert_body(
    event_id: Uuid,
    medication_id: Uuid,
    patient: Uuid,
    input: &AssertMedicationInput,
    node_kid: &str,
    hlc: Hlc,
) -> EventBody {
    let mid = medication_id.to_string();
    let a = MedicationAssertion {
        medication_id: &mid,
        term: input.term,
        inn_code: input.inn_code,
        formulation: input.formulation,
        dose_amount: input.dose_amount,
        dose_unit: input.dose_unit,
        sig: input.sig,
        info_source: input.info_source,
        started: input.started,
        started_precision: input.started_precision,
    };
    EventBody {
        event_id: event_id.to_string(),
        patient_id: patient.to_string(),
        event_type: "clinical.medication.asserted".into(),
        schema_version: MEDICATION_SCHEMA_VERSION.into(),
        hlc,
        t_effective: None,
        signer_key_id: node_kid.into(),
        contributors: serde_json::json!([{"actor_id": node_kid, "role": "recorded"}]),
        payload: medication_assertion_body(&a),
        attachments: vec![],
        plaintext_twin: Some(render_medication_twin(&a)),
    }
}

/// Record a medication the patient takes/took. Mints and returns the thread's
/// `medication_id`. Device-additive; goes through the 1-arg submit door.
pub async fn assert_medication(
    client: &tokio_postgres::Client,
    node_sk: &SigningKey,
    node_kid: &str,
    node_origin: &str,
    patient: Uuid,
    input: &AssertMedicationInput,
) -> anyhow::Result<Uuid> {
    validate_term(input.term)?;
    let hlc = crate::db::next_hlc(client, node_origin).await?;
    let event_id = Uuid::now_v7();
    let medication_id = Uuid::now_v7();
    let body = build_assert_body(event_id, medication_id, patient, input, node_kid, hlc);
    let signed = sign(&body, node_sk)?;
    client
        .execute("SELECT submit_event($1)", &[&signed.signed_bytes])
        .await?;
    Ok(medication_id)
}

/// The clinician-supplied fields of a cessation. All optional.
pub struct CeaseMedicationInput<'a> {
    pub stopped: Option<&'a str>,
    pub stopped_precision: Option<&'a str>,
    pub reason: Option<&'a str>,
}

/// Assemble the signed `clinical.medication-cessation.asserted` EventBody.
pub fn build_cease_body(
    event_id: Uuid,
    medication_id: Uuid,
    patient: Uuid,
    input: &CeaseMedicationInput,
    node_kid: &str,
    hlc: Hlc,
) -> EventBody {
    let mid = medication_id.to_string();
    let csn = MedicationCessation {
        medication_id: &mid,
        stopped: input.stopped,
        stopped_precision: input.stopped_precision,
        reason: input.reason,
    };
    EventBody {
        event_id: event_id.to_string(),
        patient_id: patient.to_string(),
        event_type: "clinical.medication-cessation.asserted".into(),
        schema_version: MEDICATION_CESSATION_SCHEMA_VERSION.into(),
        hlc,
        t_effective: None,
        signer_key_id: node_kid.into(),
        contributors: serde_json::json!([{"actor_id": node_kid, "role": "recorded"}]),
        payload: medication_cessation_body(&csn),
        attachments: vec![],
        plaintext_twin: Some(render_medication_cessation_twin(&csn)),
    }
}

/// Cease a medication thread — makes it "past". Offline-first: does NOT check the
/// assert is present locally. Returns the cessation event id.
pub async fn cease_medication(
    client: &tokio_postgres::Client,
    node_sk: &SigningKey,
    node_kid: &str,
    node_origin: &str,
    patient: Uuid,
    medication_id: Uuid,
    input: &CeaseMedicationInput,
) -> anyhow::Result<Uuid> {
    let hlc = crate::db::next_hlc(client, node_origin).await?;
    let event_id = Uuid::now_v7();
    let body = build_cease_body(event_id, medication_id, patient, input, node_kid, hlc);
    let signed = sign(&body, node_sk)?;
    client
        .execute("SELECT submit_event($1)", &[&signed.signed_bytes])
        .await?;
    Ok(event_id)
}
```

Note: `cease_medication` / `build_cease_body` / `CeaseMedicationInput` are defined now (they compile with no consumer yet) so Task 3 only adds SQL + tests. This keeps the module a single cohesive unit.

- [ ] **Step 6: Declare the module** in `crates/cairn-node/src/lib.rs` — add alongside the other `pub mod` lines:

```rust
pub mod medication;
```

- [ ] **Step 7: Run the tests to verify they pass**

Prereq: a local PG18 with `cairn_pgx` (per the project's PG test substrate). Run:

```bash
cd crates/cairn-node && CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test --test medication
```

Expected: PASS — `assert_appears_as_current`, `empty_term_is_rejected_by_the_floor`, `validate_term_rejects_blank`. (With `CAIRN_TEST_PG` unset the two DB-gated tests self-skip.)

- [ ] **Step 8: Format, lint, commit**

```bash
cargo fmt -p cairn-node && cargo clippy -p cairn-node -- -D warnings
git add db/031_medication.sql crates/cairn-node/src/db.rs crates/cairn-node/src/medication.rs crates/cairn-node/src/lib.rs crates/cairn-node/tests/medication.rs
git commit -m "feat(cairn-node): medication assert path — floor, statement projection, orchestrator

db/031 registers both clinical.medication types, adds the structural floor
(non-empty term + info_source; valid medication_id) via cairn_event_twin, and
folds asserts into medication_statement. assert_medication mints a thread id,
signs device-additive, and submits. DB-gated tests: valid assert becomes current;
empty-term rejected by the floor.

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 3: Cessation path — `medication_cessation` projection + status/past + `cease_medication`

Driven by tests: ceasing flips current→past; an orphan cessation (assert not yet local) shows no renderable row and resolves to *past* when the assert arrives.

**Files:**
- Modify: `db/031_medication.sql` (add the cessation table + trigger; replace the views with the cessation join)
- Modify: `crates/cairn-node/tests/medication.rs` (add tests)

**Interfaces:**
- Consumes: `cairn_node::medication::{cease_medication, build_cease_body, CeaseMedicationInput}` (already defined in Task 2); `build_assert_body` (to inject an assert with a chosen `medication_id`).
- Produces: SQL table `medication_cessation`; view `patient_medication` (unified, with derived `ceased`); replaced `patient_medication_current`; new `patient_medication_past`.

- [ ] **Step 1: Write the failing tests** — append to `crates/cairn-node/tests/medication.rs`

```rust
use cairn_node::medication::{build_cease_body, cease_medication, CeaseMedicationInput};

async fn past_terms(c: &Client, patient: Uuid) -> Vec<String> {
    c.query(
        "SELECT term FROM patient_medication_past WHERE patient_id = $1 ORDER BY term",
        &[&patient],
    )
    .await
    .unwrap()
    .iter()
    .map(|r| r.get::<_, String>(0))
    .collect()
}

/// Inject an assert with a CHOSEN medication_id (the orchestrator mints its own,
/// so tests that need a specific thread id build+sign+submit directly).
async fn inject_assert(
    c: &Client,
    sk: &SigningKey,
    kid: &str,
    patient: Uuid,
    medication_id: Uuid,
    input: &AssertMedicationInput<'_>,
) {
    let hlc = db::next_hlc(c, "test-node").await.unwrap();
    let body = build_assert_body(Uuid::now_v7(), medication_id, patient, input, kid, hlc);
    let signed = sign(&body, sk).unwrap();
    c.execute("SELECT submit_event($1)", &[&signed.signed_bytes])
        .await
        .unwrap();
}

#[tokio::test]
async fn cease_flips_current_to_past() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient = Uuid::now_v7();

    let med_id = assert_medication(&c, &sk, &kid, "test-node", patient, &sample_input())
        .await
        .unwrap();
    assert_eq!(current_terms(&c, patient).await, vec!["atorvastatin".to_string()]);

    cease_medication(
        &c, &sk, &kid, "test-node", patient, med_id,
        &CeaseMedicationInput { stopped: Some("2025"), stopped_precision: Some("year"), reason: Some("switched") },
    )
    .await
    .unwrap();

    assert!(current_terms(&c, patient).await.is_empty(), "ceased med leaves current");
    assert_eq!(past_terms(&c, patient).await, vec!["atorvastatin".to_string()]);
}

#[tokio::test]
async fn orphan_cessation_has_no_row_then_resolves_on_assert_arrival() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient = Uuid::now_v7();
    let med_id = Uuid::now_v7();

    // Cessation authored BEFORE its assert exists locally (offline-first).
    cease_medication(
        &c, &sk, &kid, "test-node", patient, med_id,
        &CeaseMedicationInput { stopped: None, stopped_precision: None, reason: None },
    )
    .await
    .unwrap();
    assert!(current_terms(&c, patient).await.is_empty());
    assert!(past_terms(&c, patient).await.is_empty(), "orphan cessation shows no renderable row");

    // The assert for that same thread now replicates in.
    inject_assert(&c, &sk, &kid, patient, med_id, &sample_input()).await;
    assert!(current_terms(&c, patient).await.is_empty(), "still ceased — not current");
    assert_eq!(
        past_terms(&c, patient).await,
        vec!["atorvastatin".to_string()],
        "thread now surfaces in past, arrival-order-independent"
    );
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cd crates/cairn-node && CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test --test medication`
Expected: FAIL — `patient_medication_past` view does not exist / cessation not folded.

- [ ] **Step 3: Add the cessation projection to `db/031_medication.sql`**

Insert the following **before** the final `COMMIT;`, and **replace** the assert-only `patient_medication_current` view from Task 2 with the joined versions below:

```sql
-- 7. Cessation projection. A SEPARATE table (not an UPDATE of medication_statement)
--    makes the fold arrival-order-independent: an orphan cessation (assert not yet
--    local) lands here and the join lights up as 'past' only once the assert arrives.
CREATE TABLE IF NOT EXISTS medication_cessation (
    medication_id     UUID PRIMARY KEY,
    patient_id        UUID NOT NULL,
    stopped_value     TEXT,
    stopped_precision TEXT,
    reason            TEXT,
    hlc_wall          BIGINT NOT NULL,
    hlc_counter       INTEGER NOT NULL,
    origin            TEXT NOT NULL,
    content_address   BYTEA NOT NULL,
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
GRANT SELECT ON medication_cessation TO cairn_agent;

CREATE OR REPLACE FUNCTION medication_cessation_apply()
RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE
    p jsonb := NEW.body;
BEGIN
    INSERT INTO medication_cessation
        (medication_id, patient_id, stopped_value, stopped_precision, reason,
         hlc_wall, hlc_counter, origin, content_address)
    VALUES (
        (p ->> 'medication_id')::uuid, NEW.patient_id,
        p -> 'stopped' ->> 'value',
        p -> 'stopped' ->> 'precision',
        p ->> 'reason',
        NEW.hlc_wall, NEW.hlc_counter, NEW.node_origin, NEW.content_address)
    ON CONFLICT (medication_id) DO UPDATE SET
        patient_id        = EXCLUDED.patient_id,
        stopped_value     = EXCLUDED.stopped_value,
        stopped_precision = EXCLUDED.stopped_precision,
        reason            = EXCLUDED.reason,
        hlc_wall          = EXCLUDED.hlc_wall,
        hlc_counter       = EXCLUDED.hlc_counter,
        origin            = EXCLUDED.origin,
        content_address   = EXCLUDED.content_address,
        updated_at        = clock_timestamp()
    WHERE cairn_hlc_overlay_wins(
        EXCLUDED.hlc_wall, EXCLUDED.hlc_counter, EXCLUDED.origin, EXCLUDED.content_address,
        medication_cessation.hlc_wall, medication_cessation.hlc_counter,
        medication_cessation.origin, medication_cessation.content_address);
    RETURN NULL;
END;
$$;
DROP TRIGGER IF EXISTS medication_cessation_apply_trg ON event_log;
CREATE TRIGGER medication_cessation_apply_trg
    AFTER INSERT ON event_log
    FOR EACH ROW WHEN (NEW.event_type = 'clinical.medication-cessation.asserted')
    EXECUTE FUNCTION medication_cessation_apply();

-- 8. Unified list: statement LEFT JOIN cessation → status derived. An orphan
--    cessation (no matching statement) yields NO row here (nothing to render);
--    when the statement arrives, ceased flips true. Union across all sources.
CREATE OR REPLACE VIEW patient_medication AS
SELECT s.medication_id, s.patient_id, s.term, s.inn_code, s.formulation,
       s.dose_amount, s.dose_unit, s.sig, s.info_source,
       s.started_value, s.started_precision, s.updated_at AS asserted_at,
       (c.medication_id IS NOT NULL) AS ceased,
       c.stopped_value, c.stopped_precision, c.reason
FROM medication_statement s
LEFT JOIN medication_cessation c USING (medication_id);
GRANT SELECT ON patient_medication TO cairn_agent;

CREATE OR REPLACE VIEW patient_medication_current AS
SELECT medication_id, patient_id, term, inn_code, formulation,
       dose_amount, dose_unit, sig, info_source, started_value, started_precision, asserted_at
FROM patient_medication WHERE NOT ceased;
GRANT SELECT ON patient_medication_current TO cairn_agent;

CREATE OR REPLACE VIEW patient_medication_past AS
SELECT medication_id, patient_id, term, inn_code, formulation,
       dose_amount, dose_unit, sig, info_source, started_value, started_precision,
       asserted_at, stopped_value, stopped_precision, reason
FROM patient_medication WHERE ceased;
GRANT SELECT ON patient_medication_past TO cairn_agent;
```

Also delete the Task-2 `patient_medication_current` view block (section 6) — it is superseded by section 8 above (do not leave two definitions in the file).

- [ ] **Step 4: Run to verify pass**

Run: `cd crates/cairn-node && CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test --test medication`
Expected: PASS — all Task 2 + Task 3 tests.

- [ ] **Step 5: Commit**

```bash
cargo fmt -p cairn-node && cargo clippy -p cairn-node -- -D warnings
git add db/031_medication.sql crates/cairn-node/tests/medication.rs
git commit -m "feat(cairn-node): medication cessation path — current→past, orphan-safe

medication_cessation as a separate table joined to medication_statement makes the
fold arrival-order-independent: an orphan cessation renders nothing until its assert
arrives, then surfaces in past. cease_medication is offline-first (no local-assert
requirement). DB-gated tests cover both.

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 4: Reconciliation flag (E1) — deterministic advisory duplicate detection

Driven by tests: two active threads sharing `coalesce(inn_code, normalized term)` are flagged; ceasing one clears it; distinct terms are not flagged.

**Files:**
- Modify: `db/031_medication.sql` (add the flag view)
- Modify: `crates/cairn-node/tests/medication.rs` (add tests)

**Interfaces:**
- Produces: view `patient_medication_reconciliation_flag(patient_id, dup_key, thread_count, medication_ids)`.

- [ ] **Step 1: Write the failing tests** — append to `crates/cairn-node/tests/medication.rs`

```rust
async fn flag_rows(c: &Client, patient: Uuid) -> Vec<(String, i64)> {
    c.query(
        "SELECT dup_key, thread_count FROM patient_medication_reconciliation_flag \
         WHERE patient_id = $1 ORDER BY dup_key",
        &[&patient],
    )
    .await
    .unwrap()
    .iter()
    .map(|r| (r.get::<_, String>(0), r.get::<_, i64>(1)))
    .collect()
}

#[tokio::test]
async fn two_active_same_term_are_flagged() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient = Uuid::now_v7();

    // Same drug, asserted twice (two clinicians) — differing case/whitespace must still collide.
    let mut a1 = sample_input();
    a1.term = "Atorvastatin";
    let mut a2 = sample_input();
    a2.term = "atorvastatin ";
    assert_medication(&c, &sk, &kid, "test-node", patient, &a1).await.unwrap();
    assert_medication(&c, &sk, &kid, "test-node", patient, &a2).await.unwrap();

    let flags = flag_rows(&c, patient).await;
    assert_eq!(flags.len(), 1, "one reconciliation candidate");
    assert_eq!(flags[0].1, 2, "two threads share the key");
}

#[tokio::test]
async fn ceasing_one_clears_the_flag() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient = Uuid::now_v7();

    let m1 = assert_medication(&c, &sk, &kid, "test-node", patient, &sample_input()).await.unwrap();
    let _m2 = assert_medication(&c, &sk, &kid, "test-node", patient, &sample_input()).await.unwrap();
    assert_eq!(flag_rows(&c, patient).await.len(), 1);

    // Resolution needs no new event type — cease the redundant thread.
    cease_medication(
        &c, &sk, &kid, "test-node", patient, m1,
        &CeaseMedicationInput { stopped: None, stopped_precision: None, reason: Some("duplicate") },
    )
    .await
    .unwrap();
    assert!(flag_rows(&c, patient).await.is_empty(), "flag clears once only one active thread remains");
}

#[tokio::test]
async fn distinct_terms_are_not_flagged() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient = Uuid::now_v7();

    let mut a1 = sample_input();
    a1.term = "atorvastatin";
    let mut a2 = sample_input();
    a2.term = "metformin";
    assert_medication(&c, &sk, &kid, "test-node", patient, &a1).await.unwrap();
    assert_medication(&c, &sk, &kid, "test-node", patient, &a2).await.unwrap();
    assert!(flag_rows(&c, patient).await.is_empty(), "unrelated drugs never collide");
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cd crates/cairn-node && CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test --test medication`
Expected: FAIL — `patient_medication_reconciliation_flag` view does not exist.

- [ ] **Step 3: Add the flag view** to `db/031_medication.sql`, before the final `COMMIT;`:

```sql
-- 9. E1 reconciliation flag (advisory, never auto-merges). >=2 ACTIVE threads for
--    one patient sharing coalesce(inn_code, normalized term). Deterministic — no
--    fuzzy matching (brand<->generic/typos are deferred to the Tier-A drug matcher).
--    COLLATE "C" pins the normalized-term key for cross-node determinism (ADR-0045).
--    Resolution is ceasing the redundant thread (no new event type).
CREATE OR REPLACE VIEW patient_medication_reconciliation_flag AS
SELECT patient_id,
       coalesce(inn_code, lower(btrim(term)) COLLATE "C") AS dup_key,
       count(*)                                           AS thread_count,
       array_agg(medication_id ORDER BY medication_id)    AS medication_ids
FROM patient_medication_current
GROUP BY patient_id, coalesce(inn_code, lower(btrim(term)) COLLATE "C")
HAVING count(*) > 1;
GRANT SELECT ON patient_medication_reconciliation_flag TO cairn_agent;
```

- [ ] **Step 4: Run to verify pass**

Run: `cd crates/cairn-node && CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test --test medication`
Expected: PASS — all medication tests (Tasks 2–4).

- [ ] **Step 5: Commit**

```bash
git add db/031_medication.sql crates/cairn-node/tests/medication.rs
git commit -m "feat(cairn-node): medication reconciliation flag (E1) — deterministic advisory duplicates

patient_medication_reconciliation_flag flags >=2 active threads sharing
coalesce(inn_code, normalized term); advisory only, cleared by ceasing a
duplicate. Fuzzy brand/generic detection deferred to the Tier-A matcher.

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 5: CLI verbs — `medication-assert` / `medication-cease`

Flat verbs matching the crate's house style (the `Cmd` enum has no nested groups).

**Files:**
- Modify: `crates/cairn-node/src/main.rs`

**Interfaces:**
- Consumes: `cairn_node::medication::{assert_medication, cease_medication, AssertMedicationInput, CeaseMedicationInput}`; the existing CLI helpers `load_signing_key`, `cairn_node::db::connect`, `cairn_node::identity::load_local`, `ensure_registration_actor`.

- [ ] **Step 1: Add the two variants** to the `Cmd` enum in `crates/cairn-node/src/main.rs` (place after the `IdentifyPatient { … }` variant, before the closing `}` of `enum Cmd`):

```rust
    /// Record a medication the patient takes/took (clinical.medication.asserted).
    /// Mints a medication thread id. Only --term is required; it may be vague
    /// ("little white pill"). Everything else is an honest unknown when omitted.
    MedicationAssert {
        /// The patient UUID this medication is recorded against.
        patient: Uuid,
        /// As-asserted substance term (required, may be vague).
        #[arg(long)]
        term: String,
        /// Stable INN code, if known (usually absent in slice 1 — no dictionary yet).
        #[arg(long)]
        inn_code: Option<String>,
        /// Formulation (tablet, capsule, liquid, patch, …).
        #[arg(long)]
        formulation: Option<String>,
        /// Dose magnitude (decimal, e.g. "40").
        #[arg(long)]
        dose_amount: Option<String>,
        /// Dose unit (mg, mcg, g, mL, units, puffs, drops, %, or a free-text long-tail).
        #[arg(long)]
        dose_unit: Option<String>,
        /// Free-text directions ("one BD", "PRN").
        #[arg(long)]
        sig: Option<String>,
        /// Who the claim came from: patient-reported | clinician-observed | external-record | unknown.
        #[arg(long, default_value = "unknown")]
        info_source: String,
        /// When the patient began taking it (value, e.g. "2024" or a "2020/2024" range).
        #[arg(long)]
        started: Option<String>,
        /// Precision token for --started (year|month|day|year-range).
        #[arg(long)]
        started_precision: Option<String>,
    },
    /// Cease a medication thread (clinical.medication-cessation.asserted) — makes it
    /// past. Offline-first: does not require the assert to be present locally.
    MedicationCease {
        /// The patient UUID the thread belongs to.
        patient: Uuid,
        /// The medication thread id (printed by `medication-assert`).
        medication_id: Uuid,
        /// When it was stopped (value).
        #[arg(long)]
        stopped: Option<String>,
        /// Precision token for --stopped.
        #[arg(long)]
        stopped_precision: Option<String>,
        /// Optional free-text reason.
        #[arg(long)]
        reason: Option<String>,
    },
```

- [ ] **Step 2: Add the dispatch arms** in `main()`'s `match cli.cmd { … }` (place after the `Cmd::IdentifyPatient { … } => { … }` arm). This mirrors the no-link `RegisterJohnDoe` template:

```rust
        Cmd::MedicationAssert {
            patient,
            term,
            inn_code,
            formulation,
            dose_amount,
            dose_unit,
            sig,
            info_source,
            started,
            started_precision,
        } => {
            cairn_node::medication::validate_term(&term)?;
            let node_sk = load_signing_key(&cli.key, true)?;
            let node_kid = hex::encode(node_sk.verifying_key().to_bytes());
            let db = cairn_node::db::connect(&cli.conn).await?;
            let id = cairn_node::identity::load_local(&db).await?;
            ensure_registration_actor(&db, &node_kid).await?;
            let input = cairn_node::medication::AssertMedicationInput {
                term: &term,
                inn_code: inn_code.as_deref(),
                formulation: formulation.as_deref(),
                dose_amount: dose_amount.as_deref(),
                dose_unit: dose_unit.as_deref(),
                sig: sig.as_deref(),
                info_source: &info_source,
                started: started.as_deref(),
                started_precision: started_precision.as_deref(),
            };
            let med_id = cairn_node::medication::assert_medication(
                &db, &node_sk, &node_kid, &id.node_id_hex, patient, &input,
            )
            .await?;
            println!("recorded medication for {patient}; thread {med_id}");
        }
        Cmd::MedicationCease {
            patient,
            medication_id,
            stopped,
            stopped_precision,
            reason,
        } => {
            let node_sk = load_signing_key(&cli.key, true)?;
            let node_kid = hex::encode(node_sk.verifying_key().to_bytes());
            let db = cairn_node::db::connect(&cli.conn).await?;
            let id = cairn_node::identity::load_local(&db).await?;
            ensure_registration_actor(&db, &node_kid).await?;
            let input = cairn_node::medication::CeaseMedicationInput {
                stopped: stopped.as_deref(),
                stopped_precision: stopped_precision.as_deref(),
                reason: reason.as_deref(),
            };
            let event_id = cairn_node::medication::cease_medication(
                &db, &node_sk, &node_kid, &id.node_id_hex, patient, medication_id, &input,
            )
            .await?;
            println!("ceased medication thread {medication_id}; event {event_id}");
        }
```

- [ ] **Step 3: Build and lint**

Run: `cargo build -p cairn-node && cargo clippy -p cairn-node -- -D warnings`
Expected: builds, no warnings.

- [ ] **Step 4: Manual smoke verification** (uses the `run`/`verify` discipline — exercise the real CLI end-to-end, not just tests)

With a local PG18 + `cairn_pgx` and a node key (`node.key`), run:

```bash
CONN="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test"
PATIENT=$(uuidgen)
cargo run -p cairn-node -- --conn "$CONN" medication-assert "$PATIENT" \
  --term atorvastatin --dose-amount 40 --dose-unit mg --sig "one BD" --info-source patient-reported --started 2024 --started-precision year
# copy the printed "thread <uuid>" id into MED, then:
psql "$CONN" -c "SELECT term, dose_amount, dose_unit FROM patient_medication_current WHERE patient_id = '$PATIENT';"
cargo run -p cairn-node -- --conn "$CONN" medication-cease "$PATIENT" "$MED" --stopped 2025 --stopped-precision year --reason switched
psql "$CONN" -c "SELECT term, reason FROM patient_medication_past WHERE patient_id = '$PATIENT';"
```

Expected: the assert shows one current row; after cease, zero current and one past row. Record the observed output in the commit message.

- [ ] **Step 5: Commit**

```bash
git add crates/cairn-node/src/main.rs
git commit -m "feat(cairn-node): medication-assert / medication-cease CLI verbs

Flat verbs (house style) wiring the medication orchestrators. Verified end-to-end
against a local node: assert -> current; cease -> past.

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 6: Whole-workspace green + docs currency

**Files:**
- Modify: `docs/HANDOVER.md` (regenerate the current-state section per the project convention)
- Modify: `docs/ROADMAP.md` if present (mark the medication slice-1 status)

- [ ] **Step 1: Full workspace gate**

Run:
```bash
cargo fmt --all -- --check
cargo clippy --workspace -- -D warnings
CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test --workspace
```
Expected: fmt clean; clippy clean; all tests pass (cairn-event pure + cairn-node DB-gated medication 8/8 + the existing suite green). If anything fails, fix it before proceeding (house rule 5: fix review/CI findings, or file an issue).

- [ ] **Step 2: Update `docs/HANDOVER.md`**

Add a "This session" block recording: the medication slice-1 surface built (`clinical.medication.asserted` + `clinical.medication-cessation.asserted`, db/031, `medication_statement`/`medication_cessation` projections, `patient_medication{,_current,_past,_reconciliation_flag}` views, `assert_medication`/`cease_medication` + CLI verbs); that it is the **first clinical-content event stream**; the deferred items (dose-correction, fuzzy reconciliation, delete-suppression, structured sig, Tier-A dictionary, route, staleness review, #157 collision-advisory on the medication projections, human-attested responsibility); no ADR/spec/wire change. Point at the design + plan docs.

- [ ] **Step 3: Commit the docs**

```bash
git add docs/HANDOVER.md docs/ROADMAP.md
git commit -m "docs(handover): medication recording slice-1 built (clinical.medication surface)

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

- [ ] **Step 4: Open the PR** (only if the user asks — do not push/PR unprompted)

```bash
git push -u origin feat/medication-recording-slice-1
gh pr create --title "feat: medication recording (slice 1) — the first clinical-content surface" --body "…"
```

---

## Deferred (out of scope — do NOT implement here)

Dose-correction/change overlay · fuzzy reconciliation (brand↔generic, typos, salts) · reconciliation *resolution* as a first-class event · `delete` rendering-suppression visibility overlay · structured sig/frequency (lands with prescriptions) · Tier-A dictionary + autocomplete + DDI · a separate `route` field · active review / last-confirmed staleness · the #157 HLC-triple collision advisory on the medication projections (consistency follow-on to match db/024) · human-attested clinical responsibility on a medication statement (slice 1 is device-additive).
