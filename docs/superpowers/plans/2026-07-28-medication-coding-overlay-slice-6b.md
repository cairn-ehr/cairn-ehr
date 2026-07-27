# `clinical.medication` slice 6b — the coding-overlay event types Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make drug coding a separately-authored act — a medication asserted uncoded can be coded later by a pharmacist or coder, a wrong coding can be corrected, and a coding established as wrong can be **struck** back to honest not-yet-coded.

**Architecture:** Two new clinical event types whose apply fns write the *existing* `medication_coding` table under the *existing* overlay-winner rule, so the whole slice is additive: no view is re-routed and no view's column set changes. A strike NULLs the anchor columns and sets a `struck` flag, which makes the dup-key and group-display degradation fall out of the existing `coalesce` — with one exception the plan fixes explicitly. A new `patient_medication_uncoded` view routes uncoded and struck threads to whoever codes them.

**Tech Stack:** Rust (`cairn-event` pure builders, `cairn-node` orchestrators + clap CLI), PostgreSQL 18 + `cairn_pgx`, plpgsql in-DB floor, tokio-postgres integration tests.

## Global Constraints

- **Design source:** [`docs/superpowers/specs/2026-07-27-medication-coding-overlay-slice-6b-design.md`](../specs/2026-07-27-medication-coding-overlay-slice-6b-design.md); the decision record is [ADR-0059](../../spec/decisions/0059-medication-drug-coding-drugref-moiety-anchor.md) decision 3. Where they disagree, the ADR wins.
- **Branch:** `feat/medication-coding-overlay-slice-6b-0059`, stacked on the unmerged slice 6a (`feat/medication-coding-slice-6a-0059`, PR #297). Everything 6a built is present and committed.
- **Licence:** AGPL-3.0. No new dependency; if one becomes tempting, stop and ask.
- **TDD:** failing test first, run it, see it fail for the right reason, then the minimal implementation.
- **No drugref code enters the tree.** `crates/cairn-node/tests/no_drugref_dependency.rs` is a source guard that will fail if it does. It must still pass at the end of every task.
- **Uncoded stays first-class** (principle 4): a medication with no coding must remain valid at every layer, and no field may become required.
- **Never hard-code cryptographic material in tests** (house rule 6). The UUID constants here are drug identifiers, not key material — those are fine.
- **Test runs:** `cargo test --workspace` exceeds this harness's 600s foreground cap — use per-test-binary runs, one foreground Bash call each, never backgrounded, never piped through `tail` (which masks cargo's exit code). Prefix `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test"` on every command; the shell does not persist between calls.
- **If `born_sealed_schema` fails with `invalid input syntax for type bigint: "unknown"`**, that is known unrelated test pollution ([#296](https://github.com/cairn-ehr/cairn-ehr/issues/296)) — recreate the test DBs, do not chase it.
- **A widened `CREATE TABLE IF NOT EXISTS` needs a paired `ALTER TABLE … ADD COLUMN IF NOT EXISTS`** (#207); the loader replays every migration on every connect against the live schema.
- **Registry rows converge on replay:** `ON CONFLICT … DO UPDATE … WHERE (…) IS DISTINCT FROM (…)` (#214), never `DO NOTHING` — including for `event_type_class`, where older sibling files still use `DO NOTHING` ([#254](https://github.com/cairn-ehr/cairn-ehr/issues/254) asks for exactly this direction).
- **Every new event type must be registered in BOTH places** — the Rust pin and its `db/tests/*.sql` mirror. Missing one fails CI since PR #251, but only after it has already drifted.
- **A new `db/NNN_*.sql` bumps `SCHEMA_GENERATION`** (`crates/cairn-event/src/schema_generation.rs`) and is added to cairn-node's `SCHEMA` list (`crates/cairn-node/src/db.rs`). cairn-sync's list carries no medication files and legitimately lags (#284).

## Paper-parity benchmark (§1.2)

- **Paper counterpart:** a pharmacist writing *"= atorvastatin"* beside *"Lipitor"* on a paper medication list — or striking that annotation through when it proves wrong. **N = 1** human act in each direction.
- **Steps:** paper **N = 1** → architecture-forced **M = 1** (coding is one event; striking is one event) → UI bundling target **K = 1**. Coding stays optional at every layer, so no existing medication workflow gains a forced step — a clinician who never codes anything is unaffected. `M > N` here would be an architecture defect to file (house rule 5).
- **Time + cognitive load:** no budget measured by this slice, deliberately: 6b exposes a CLI test/ops surface, not the clinician surface a budget would measure. Owed by the coding-UI slice ([#288](https://github.com/cairn-ehr/cairn-ehr/issues/288) neighbourhood), whose target is that accepting a suggested coding costs zero keystrokes over not coding at all.

## File Structure

**Created:**

| File | Responsibility |
|---|---|
| `crates/cairn-event/src/medication/coding.rs` | Pure payload builders + legibility twins for the two overlay types. |
| `crates/cairn-node/src/medication/coding.rs` | The two orchestrators: build body, seal, sign, submit. |
| `db/042_medication_coding_overlay.sql` | Event-type registration, the overlay floor, the `struck` schema change, both apply fns, the worklist view. |
| `crates/cairn-node/tests/medication_coding_overlay.rs` | DB-gated floor + projection + worklist tests. |

**Modified:**

| File | Change |
|---|---|
| `crates/cairn-event/src/medication/mod.rs` | Declare + re-export the new module. |
| `crates/cairn-node/src/medication/mod.rs` | Declare + re-export the two orchestrators and their input types. |
| `crates/cairn-node/src/main.rs` | Two CLI subcommands. |
| `crates/cairn-event/src/schema_generation.rs` | `SCHEMA_GENERATION` 41 → 42. |
| `crates/cairn-node/src/db.rs` | `db/042` added to the `SCHEMA` list. |
| `db/041_medication_coding.sql` | Extract `cairn_check_coding_object(c, p_prefix)`; the existing fn becomes a thin wrapper. |
| `db/033_medication_reconciliation.sql` | The prefer-coded predicate tests anchor presence, not row presence. |
| `crates/cairn-node/tests/twin_registry.rs`, `db/tests/034_twin_registry_test.sql` | 19 → 21. |
| `crates/cairn-node/tests/projection_registry.rs`, `db/tests/039_projection_registry_test.sql` | 22 → 24 / 25 → 27. |
| `crates/cairn-sync/tests/clinical_pull.rs` | Cross-node convergence of an overlay coding and a strike. |
| `docs/HANDOVER.md`, `docs/ROADMAP.md` | Slice 57 record. |

---

### Task 1: The two payload builders and their twins (`cairn-event`)

**Files:**
- Create: `crates/cairn-event/src/medication/coding.rs`
- Modify: `crates/cairn-event/src/medication/mod.rs:14-18` (module list) and `:20-22` (re-export)
- Test: in-file `mod tests` in the new file

**Interfaces:**
- Consumes: `cairn_event::medication::SubstanceCoding` (slice 6a) — `pub struct SubstanceCoding<'a> { pub system: &'a str, pub code: &'a str, pub display: &'a str }`, which is `Copy`.
- Produces:
  - `pub struct MedicationCoding<'a> { pub medication_id: &'a str, pub coding: SubstanceCoding<'a> }`
  - `pub struct MedicationCodingCorrection<'a> { pub medication_id: &'a str, pub corrects: &'a str, pub coding: Option<SubstanceCoding<'a>>, pub strike: bool, pub note: Option<&'a str> }`
  - `pub fn medication_coding_body(c: &MedicationCoding) -> serde_json::Value`
  - `pub fn medication_coding_correction_body(c: &MedicationCodingCorrection) -> serde_json::Value`
  - `pub fn render_medication_coding_twin(c: &MedicationCoding) -> String`
  - `pub fn render_medication_coding_correction_twin(c: &MedicationCodingCorrection) -> String`

- [ ] **Step 1: Write the failing tests**

Create `crates/cairn-event/src/medication/coding.rs` containing only this test module for now (the types it names come in step 3):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    const MOIETY: &str = "0f8c4b1e-1b7a-5c2d-9a3e-2b6f7c8d9e01";
    const MED: &str = "11111111-1111-7111-8111-111111111111";
    const TARGET: &str = "22222222-2222-7222-8222-222222222222";

    fn coding() -> SubstanceCoding<'static> {
        SubstanceCoding {
            system: "drugref-moiety",
            code: MOIETY,
            display: "atorvastatin",
        }
    }

    #[test]
    fn coding_body_carries_the_thread_and_the_triple() {
        let v = medication_coding_body(&MedicationCoding {
            medication_id: MED,
            coding: coding(),
        });
        assert_eq!(v["medication_id"], MED);
        assert_eq!(v["coding"]["system"], "drugref-moiety");
        assert_eq!(v["coding"]["code"], MOIETY);
        assert_eq!(v["coding"]["display"], "atorvastatin");
    }

    #[test]
    fn correction_body_with_a_replacement_carries_no_strike_key() {
        let v = medication_coding_correction_body(&MedicationCodingCorrection {
            medication_id: MED,
            corrects: TARGET,
            coding: Some(coding()),
            strike: false,
            note: Some("brand name was ambiguous"),
        });
        assert_eq!(v["corrects"], TARGET);
        assert_eq!(v["coding"]["display"], "atorvastatin");
        assert_eq!(v["note"], "brand name was ambiguous");
        assert!(
            !v.as_object().unwrap().contains_key("strike"),
            "a replacement must not also claim a strike"
        );
    }

    #[test]
    fn correction_body_with_a_strike_carries_no_coding_key() {
        let v = medication_coding_correction_body(&MedicationCodingCorrection {
            medication_id: MED,
            corrects: TARGET,
            coding: None,
            strike: true,
            note: Some("not metformin; substance unidentified"),
        });
        assert_eq!(v["strike"], true);
        assert!(
            !v.as_object().unwrap().contains_key("coding"),
            "a strike must not also carry a coding"
        );
    }

    #[test]
    fn correction_body_omits_an_absent_note() {
        let v = medication_coding_correction_body(&MedicationCodingCorrection {
            medication_id: MED,
            corrects: TARGET,
            coding: None,
            strike: true,
            note: None,
        });
        assert!(
            !v.as_object().unwrap().contains_key("note"),
            "absent note must be omitted, not null"
        );
    }

    #[test]
    fn coding_twin_names_the_substance_and_the_system() {
        let s = render_medication_coding_twin(&MedicationCoding {
            medication_id: MED,
            coding: coding(),
        });
        assert_eq!(s, "coded as atorvastatin [drugref-moiety]");
    }

    #[test]
    fn correction_twins_distinguish_replacement_from_strike() {
        let replaced = render_medication_coding_correction_twin(&MedicationCodingCorrection {
            medication_id: MED,
            corrects: TARGET,
            coding: Some(coding()),
            strike: false,
            note: Some("brand name was ambiguous"),
        });
        assert_eq!(
            replaced,
            "coding corrected to atorvastatin [drugref-moiety] — brand name was ambiguous"
        );

        let struck = render_medication_coding_correction_twin(&MedicationCodingCorrection {
            medication_id: MED,
            corrects: TARGET,
            coding: None,
            strike: true,
            note: None,
        });
        assert_eq!(struck, "coding struck — no longer coded");
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p cairn-event medication::coding`
Expected: FAIL to compile — `cannot find function medication_coding_body` / `cannot find struct MedicationCoding`.

- [ ] **Step 3: Write the implementation**

Prepend to the same file (above `mod tests`):

```rust
//! Slice-6b coding *overlay* builders — coding as a separately-authored act (ADR-0059
//! decision 3). Pure: shapes only payload JSON, no clock, no randomness, no I/O.
//!
//! A medication may be coded inline on the assertion (slice 6a) or later by whoever
//! codes it — a pharmacist or a professional coder, as a distinct contributor whose
//! coding claim never overwrites the clinician's clinical claim. A correction either
//! replaces the claim or STRIKES it: append-only means the correction event is the only
//! repair path, so it must be able to say "not that, and I don't know what it is" —
//! otherwise a reviewer who disproves a coding can only leave it standing or invent a
//! substitute identity they cannot vouch for (principle 4).
use super::SubstanceCoding;
use serde_json::{json, Value};

/// Code a medication thread that was not coded inline.
pub struct MedicationCoding<'a> {
    /// The immortal thread id being coded.
    pub medication_id: &'a str,
    /// The drug-identity claim.
    pub coding: SubstanceCoding<'a>,
}

/// Correct a coding claim — replace it, or strike it back to not-yet-coded.
///
/// Exactly one of `coding` / `strike` is meaningful; the in-DB floor refuses both and
/// neither. The strike is EXPLICIT rather than inferred from an absent coding, so a
/// caller who simply forgets the coding gets a refusal instead of silently un-coding a
/// medication.
pub struct MedicationCodingCorrection<'a> {
    pub medication_id: &'a str,
    /// The event whose coding claim this fixes — a prior coding overlay, or the
    /// assertion itself when the coding was inline. Existence is NOT required
    /// anywhere: the corrected event may replicate later, or never (offline-first).
    pub corrects: &'a str,
    /// The replacement claim. `None` with `strike` = strike to not-yet-coded.
    pub coding: Option<SubstanceCoding<'a>>,
    /// Strike the coding back to honest not-yet-coded.
    pub strike: bool,
    /// Why THIS correction was made (audit). `None` = omit the key.
    pub note: Option<&'a str>,
}

/// Serialize a coding triple as its wire object.
fn coding_object(c: &SubstanceCoding) -> Value {
    json!({ "system": c.system, "code": c.code, "display": c.display })
}

/// Build the `clinical.medication-coding.asserted` payload.
pub fn medication_coding_body(c: &MedicationCoding) -> Value {
    json!({
        "medication_id": c.medication_id,
        "coding": coding_object(&c.coding),
    })
}

/// Build the `clinical.medication-coding-correction.asserted` payload. Optional keys are
/// inserted only when present — never serialized as null (principle 11: an added-later
/// field must not change an existing event's content address).
pub fn medication_coding_correction_body(c: &MedicationCodingCorrection) -> Value {
    let mut p = json!({
        "medication_id": c.medication_id,
        "corrects": c.corrects,
    });
    let obj = p.as_object_mut().expect("json! built an object");
    if let Some(coding) = &c.coding {
        obj.insert("coding".into(), coding_object(coding));
    } else if c.strike {
        obj.insert("strike".into(), json!(true));
    }
    if let Some(n) = c.note {
        obj.insert("note".into(), json!(n));
    }
    p
}

/// The §3.13 legibility twin for a coding overlay. Non-empty by construction: the
/// display and system are both floor-mandated non-empty strings.
pub fn render_medication_coding_twin(c: &MedicationCoding) -> String {
    format!("coded as {} [{}]", c.coding.display, c.coding.system)
}

/// The §3.13 legibility twin for a coding correction — a reader must be able to tell a
/// replacement from a retraction without holding any drug database.
pub fn render_medication_coding_correction_twin(c: &MedicationCodingCorrection) -> String {
    let head = match &c.coding {
        Some(k) => format!("coding corrected to {} [{}]", k.display, k.system),
        None => "coding struck — no longer coded".to_string(),
    };
    match c.note {
        Some(n) => format!("{head} — {n}"),
        None => head,
    }
}
```

In `crates/cairn-event/src/medication/mod.rs`, add `pub mod coding;` to the module list and extend the re-exports:

```rust
pub use coding::{
    medication_coding_body, medication_coding_correction_body, render_medication_coding_twin,
    render_medication_coding_correction_twin, MedicationCoding, MedicationCodingCorrection,
};
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p cairn-event`
Expected: PASS (no database needed for this crate).

- [ ] **Step 5: Commit**

```bash
git add crates/cairn-event/src/medication/coding.rs crates/cairn-event/src/medication/mod.rs
git commit -m "feat(medication 6b): coding-overlay payload builders and twins

Two payload shapes for coding as a separately-authored act (ADR-0059
decision 3): a coding overlay, and a correction that either replaces the
claim or strikes it back to not-yet-coded. The strike is explicit rather
than inferred from an absent coding, so a caller who forgets the coding
is refused instead of silently un-coding a medication.

Refs ADR-0059"
```

---

### Task 2: The node orchestrators and CLI

**Files:**
- Create: `crates/cairn-node/src/medication/coding.rs`
- Modify: `crates/cairn-node/src/medication/mod.rs:8-13` (module list), `:15-31` (re-exports)
- Modify: `crates/cairn-node/src/main.rs` (two new `Cmd` variants + their dispatch arms)
- Test: in-file `mod tests` in the new file

**Interfaces:**
- Consumes: Task 1's builders; slice 6a's `coding_from_parts(system, code, display) -> anyhow::Result<Option<SubstanceCoding<'_>>>`; `crate::medication::sealed_submit::seal_sign_submit(client, node_sk, body, author, attest)`; `crate::db::next_hlc(client, node_origin)`.
- Produces:
  - `pub struct CodeMedicationInput<'a> { pub coding: SubstanceCoding<'a> }`
  - `pub struct CorrectCodingInput<'a> { pub corrects: Uuid, pub coding: Option<SubstanceCoding<'a>>, pub strike: bool, pub note: Option<&'a str> }`
  - `pub fn build_coding_body(event_id: Uuid, medication_id: Uuid, patient: Uuid, input: &CodeMedicationInput<'_>, node_kid: &str, hlc: Hlc) -> EventBody`
  - `pub fn build_coding_correction_body(event_id: Uuid, medication_id: Uuid, patient: Uuid, input: &CorrectCodingInput<'_>, node_kid: &str, hlc: Hlc) -> EventBody`
  - `pub fn validate_correction_shape(coding: Option<&SubstanceCoding<'_>>, strike: bool) -> anyhow::Result<()>`
  - `pub async fn code_medication(...) -> anyhow::Result<Uuid>` and `pub async fn correct_medication_coding(...) -> anyhow::Result<Uuid>`, both returning the new event's id.

- [ ] **Step 1: Write the failing test**

In the new `crates/cairn-node/src/medication/coding.rs`, start with the pure-validation tests:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn coding() -> SubstanceCoding<'static> {
        SubstanceCoding {
            system: "drugref-moiety",
            code: "0f8c4b1e-1b7a-5c2d-9a3e-2b6f7c8d9e01",
            display: "atorvastatin",
        }
    }

    #[test]
    fn a_replacement_or_a_strike_is_valid() {
        validate_correction_shape(Some(&coding()), false).expect("a replacement is valid");
        validate_correction_shape(None, true).expect("a strike is valid");
    }

    #[test]
    fn neither_is_refused_at_the_source() {
        // The DB floor refuses this too, but the caller deserves the error where the
        // mistake was made, naming the two ways out.
        let e = validate_correction_shape(None, false).expect_err("neither must be refused");
        let msg = e.to_string();
        assert!(msg.contains("--strike"), "the error names the escape: {msg}");
    }

    #[test]
    fn both_is_refused_as_incoherent() {
        let e = validate_correction_shape(Some(&coding()), true)
            .expect_err("a correction cannot both replace and strike");
        assert!(e.to_string().contains("both"), "{e}");
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p cairn-node --lib medication::coding`
Expected: FAIL to compile — `cannot find function validate_correction_shape`.

- [ ] **Step 3: Write the implementation**

Prepend to the same file:

```rust
//! Medication coding overlays — the node authoring surface for coding as a
//! separately-authored act (ADR-0059 decision 3). Offline-first: neither verb checks
//! that the thread, or the corrected event, is present locally — both may replicate
//! later, or never.
use cairn_event::medication::{
    medication_coding_body, medication_coding_correction_body,
    render_medication_coding_correction_twin, render_medication_coding_twin, MedicationCoding,
    MedicationCodingCorrection, SubstanceCoding,
};
use cairn_event::{EventBody, Hlc, SigningKey};
use uuid::Uuid;

const CODING_SCHEMA_VERSION: &str = "clinical.medication-coding/1";
const CODING_CORRECTION_SCHEMA_VERSION: &str = "clinical.medication-coding-correction/1";

/// Code a thread that was not coded inline.
pub struct CodeMedicationInput<'a> {
    pub coding: SubstanceCoding<'a>,
}

/// Correct a coding claim: replace it, or strike it back to not-yet-coded.
pub struct CorrectCodingInput<'a> {
    /// The event whose coding this fixes (a coding overlay, or the assertion itself).
    pub corrects: Uuid,
    pub coding: Option<SubstanceCoding<'a>>,
    pub strike: bool,
    pub note: Option<&'a str>,
}

/// Refuse an incoherent correction at the source. The in-DB floor is the real,
/// unbypassable enforcement; this exists so the caller sees the mistake where they made
/// it, with both ways out named.
pub fn validate_correction_shape(
    coding: Option<&SubstanceCoding<'_>>,
    strike: bool,
) -> anyhow::Result<()> {
    match (coding.is_some(), strike) {
        (true, true) => anyhow::bail!(
            "a coding correction cannot both replace and strike: supply the three --coding-* flags OR --strike, not both"
        ),
        (false, false) => anyhow::bail!(
            "a coding correction must either carry a replacement (all three --coding-* flags) or --strike it back to not-yet-coded"
        ),
        _ => Ok(()),
    }
}

/// Assemble the signed `clinical.medication-coding.asserted` EventBody. Pure.
pub fn build_coding_body(
    event_id: Uuid,
    medication_id: Uuid,
    patient: Uuid,
    input: &CodeMedicationInput<'_>,
    node_kid: &str,
    hlc: Hlc,
) -> EventBody {
    let mid = medication_id.to_string();
    let c = MedicationCoding {
        medication_id: &mid,
        coding: input.coding,
    };
    EventBody {
        event_id: event_id.to_string(),
        patient_id: patient.to_string(),
        event_type: "clinical.medication-coding.asserted".into(),
        schema_version: CODING_SCHEMA_VERSION.into(),
        hlc,
        t_effective: None,
        signer_key_id: node_kid.into(),
        contributors: serde_json::json!([{"actor_id": node_kid, "role": "recorded"}]),
        payload: medication_coding_body(&c),
        attachments: vec![],
        plaintext_twin: Some(render_medication_coding_twin(&c)),
        clock_grade: cairn_event::ClockGrade::SelfAsserted,
    }
}

/// Assemble the signed `clinical.medication-coding-correction.asserted` EventBody. Pure.
pub fn build_coding_correction_body(
    event_id: Uuid,
    medication_id: Uuid,
    patient: Uuid,
    input: &CorrectCodingInput<'_>,
    node_kid: &str,
    hlc: Hlc,
) -> EventBody {
    let mid = medication_id.to_string();
    let target = input.corrects.to_string();
    let c = MedicationCodingCorrection {
        medication_id: &mid,
        corrects: &target,
        coding: input.coding,
        strike: input.strike,
        note: input.note,
    };
    EventBody {
        event_id: event_id.to_string(),
        patient_id: patient.to_string(),
        event_type: "clinical.medication-coding-correction.asserted".into(),
        schema_version: CODING_CORRECTION_SCHEMA_VERSION.into(),
        hlc,
        t_effective: None,
        signer_key_id: node_kid.into(),
        contributors: serde_json::json!([{"actor_id": node_kid, "role": "recorded"}]),
        payload: medication_coding_correction_body(&c),
        attachments: vec![],
        plaintext_twin: Some(render_medication_coding_correction_twin(&c)),
        clock_grade: cairn_event::ClockGrade::SelfAsserted,
    }
}

/// Code an existing medication thread. Returns the coding event's id — the value a
/// later correction passes as `corrects`.
#[allow(clippy::too_many_arguments)] // signer + node context + patient/thread/input/author/attest, mirrors the sibling orchestrators
pub async fn code_medication(
    client: &mut tokio_postgres::Client,
    node_sk: &SigningKey,
    node_kid: &str,
    node_origin: &str,
    patient: Uuid,
    medication_id: Uuid,
    input: &CodeMedicationInput<'_>,
    author: Option<&crate::medication::AuthorParams<'_>>,
    attest: Option<&crate::medication::AttestParams<'_>>,
) -> anyhow::Result<Uuid> {
    let hlc = crate::db::next_hlc(client, node_origin).await?;
    let event_id = Uuid::now_v7();
    let body = build_coding_body(event_id, medication_id, patient, input, node_kid, hlc);
    crate::medication::sealed_submit::seal_sign_submit(client, node_sk, body, author, attest)
        .await?;
    Ok(event_id)
}

/// Correct (replace or strike) a thread's coding. Returns the correction event's id.
#[allow(clippy::too_many_arguments)] // as above
pub async fn correct_medication_coding(
    client: &mut tokio_postgres::Client,
    node_sk: &SigningKey,
    node_kid: &str,
    node_origin: &str,
    patient: Uuid,
    medication_id: Uuid,
    input: &CorrectCodingInput<'_>,
    author: Option<&crate::medication::AuthorParams<'_>>,
    attest: Option<&crate::medication::AttestParams<'_>>,
) -> anyhow::Result<Uuid> {
    validate_correction_shape(input.coding.as_ref(), input.strike)?;
    let hlc = crate::db::next_hlc(client, node_origin).await?;
    let event_id = Uuid::now_v7();
    let body =
        build_coding_correction_body(event_id, medication_id, patient, input, node_kid, hlc);
    crate::medication::sealed_submit::seal_sign_submit(client, node_sk, body, author, attest)
        .await?;
    Ok(event_id)
}
```

In `crates/cairn-node/src/medication/mod.rs` add `mod coding;` and:

```rust
pub use coding::{
    build_coding_body, build_coding_correction_body, code_medication, correct_medication_coding,
    validate_correction_shape, CodeMedicationInput, CorrectCodingInput,
};
```

- [ ] **Step 4: Add the two CLI subcommands**

In `crates/cairn-node/src/main.rs`, add two `Cmd` variants beside `MedicationAssert`, following its `--coding-*` flag docs verbatim:

```rust
    /// Code an existing medication thread (clinical.medication-coding.asserted).
    /// Coding is optional and separately authored — a pharmacist or coder may code a
    /// medication a clinician recorded uncoded, without touching the clinical claim.
    MedicationCode {
        /// The patient UUID this medication belongs to.
        patient: Uuid,
        /// The medication thread being coded.
        medication_id: Uuid,
        /// Drug-identity coding system — `drugref-moiety` today (ADR-0059).
        #[arg(long)]
        coding_system: Option<String>,
        /// The immortal drug identifier (a drugref moiety_uuid).
        #[arg(long)]
        coding_code: Option<String>,
        /// The INN-preferred label as it reads at coding time.
        #[arg(long)]
        coding_display: Option<String>,
        #[arg(long)]
        author: Option<String>,
    },

    /// Correct a medication's coding — replace it, or --strike it back to not-yet-coded
    /// (clinical.medication-coding-correction.asserted).
    MedicationCodeCorrect {
        /// The patient UUID this medication belongs to.
        patient: Uuid,
        /// The medication thread whose coding is being corrected.
        medication_id: Uuid,
        /// The event whose coding claim this fixes (a coding overlay, or the assertion
        /// itself when the coding was inline). Not required to be present locally.
        #[arg(long)]
        corrects: Uuid,
        #[arg(long)]
        coding_system: Option<String>,
        #[arg(long)]
        coding_code: Option<String>,
        #[arg(long)]
        coding_display: Option<String>,
        /// Strike the coding back to honest not-yet-coded — for when a reviewer
        /// establishes the coding is wrong but cannot say what the substance is.
        #[arg(long)]
        strike: bool,
        /// Why this correction was made (audit).
        #[arg(long)]
        note: Option<String>,
        #[arg(long)]
        author: Option<String>,
    },
```

Their dispatch arms mirror `Cmd::MedicationAssert`'s (load the key, connect, `ensure_registration_actor`, resolve the author) and then:

```rust
        Cmd::MedicationCode {
            patient,
            medication_id,
            coding_system,
            coding_code,
            coding_display,
            author,
        } => {
            let node_sk = load_signing_key(&cli.key, true)?;
            let node_kid = hex::encode(node_sk.verifying_key().to_bytes());
            let mut db = cairn_node::db::connect(&cli.conn).await?;
            let id = cairn_node::identity::load_local(&db).await?;
            ensure_registration_actor(&db, &node_kid).await?;
            let coding = cairn_node::medication::coding_from_parts(
                coding_system.as_deref(),
                coding_code.as_deref(),
                coding_display.as_deref(),
            )?
            .ok_or_else(|| {
                anyhow::anyhow!("coding a medication needs all three --coding-* flags")
            })?;
            let resolved_author = resolve_author(&db, &author).await?;
            let a_params = author_params(&resolved_author);
            let event_id = cairn_node::medication::code_medication(
                &mut db,
                &node_sk,
                &node_kid,
                &id.node_id_hex,
                patient,
                medication_id,
                &cairn_node::medication::CodeMedicationInput { coding },
                a_params.as_ref(),
                None,
            )
            .await?;
            println!("coded {medication_id}; event {event_id}");
        }
```

The `MedicationCodeCorrect` arm is the same shape, building `CorrectCodingInput { corrects, coding, strike, note: note.as_deref() }` (note that `coding_from_parts` returns `None` when all three flags are absent, which is exactly what a strike wants) and calling `correct_medication_coding`, printing `corrected coding on {medication_id}; event {event_id}`.

- [ ] **Step 5: Run the tests**

Run these as separate foreground calls:
`cargo test -p cairn-node --lib`, then `cargo test -p cairn-event`, then `cargo fmt --all -- --check`, then `cargo clippy --workspace --all-targets -- -D warnings`.
Expected: all PASS/clean.

- [ ] **Step 6: Commit**

```bash
git add crates/cairn-node/src/medication crates/cairn-node/src/main.rs
git commit -m "feat(medication 6b): code and code-correct orchestrators + CLI

validate_correction_shape refuses both-and-neither at the source, naming
both ways out, rather than letting the DB floor be the first thing that
tells a caller their correction is incoherent. Both verbs are
offline-first: neither the thread nor the corrected event must exist
locally.

Refs ADR-0059"
```

---

### Task 3: The floor — extract the shared check, add `db/042`, register both types

**Files:**
- Modify: `db/041_medication_coding.sql` (extract `cairn_check_coding_object`)
- Create: `db/042_medication_coding_overlay.sql`
- Modify: `crates/cairn-event/src/schema_generation.rs` (41 → 42)
- Modify: `crates/cairn-node/src/db.rs` (append the `042` entry)
- Modify: `crates/cairn-node/tests/twin_registry.rs:103` (19 → 21)
- Modify: `db/tests/034_twin_registry_test.sql:20-29` (19 → 21, and its explanatory comment)
- Create: `crates/cairn-node/tests/medication_coding_overlay.rs`

**Interfaces:**
- Consumes: slice 6a's `cairn_check_medication_coding(p jsonb)` and the `medication_coding_system` registry.
- Produces: `cairn_check_coding_object(c jsonb, p_prefix text) RETURNS void` (db/041) and `cairn_check_medication_coding_overlay(p_type text, b jsonb) RETURNS void` (db/042, the twin-registry `check_fn` for both new types).

- [ ] **Step 1: Write the failing tests**

Create `crates/cairn-node/tests/medication_coding_overlay.rs`. Copy the harness preamble (`cs`, `db_msg`, `seal_and_submit`, `setup_node`) from `crates/cairn-node/tests/medication_coding.rs` — it already truncates `medication_coding`. Then:

```rust
const MOIETY_ATORVASTATIN: &str = "0f8c4b1e-1b7a-5c2d-9a3e-2b6f7c8d9e01";

fn coding() -> SubstanceCoding<'static> {
    SubstanceCoding {
        system: "drugref-moiety",
        code: MOIETY_ATORVASTATIN,
        display: "atorvastatin",
    }
}

/// Submit a raw overlay payload at a chosen door, bypassing the Rust builders — the only
/// way to present the malformed shapes a buggy or hostile peer could send.
async fn submit_raw_overlay(
    c: &Client,
    sk: &SigningKey,
    kid: &str,
    door: &str,
    event_type: &str,
    schema_version: &str,
    payload: serde_json::Value,
) -> Result<u64, tokio_postgres::Error> {
    let hlc = db::next_hlc(c, "test-node").await.unwrap();
    let body = EventBody {
        event_id: Uuid::now_v7().to_string(),
        patient_id: Uuid::now_v7().to_string(),
        event_type: event_type.into(),
        schema_version: schema_version.into(),
        hlc,
        t_effective: None,
        signer_key_id: kid.into(),
        contributors: serde_json::json!([{"actor_id": kid, "role": "recorded"}]),
        payload,
        attachments: vec![],
        plaintext_twin: Some("coded as atorvastatin [drugref-moiety]".into()),
        clock_grade: cairn_event::ClockGrade::SelfAsserted,
    };
    match door {
        "submit_event" => seal_and_submit(c, sk, body).await,
        _ => {
            let signed = cairn_event::sign(&body, sk).unwrap();
            c.execute(&format!("SELECT {door}($1)")[..], &[&signed.signed_bytes])
                .await
        }
    }
}

#[tokio::test]
async fn both_overlay_types_are_registered() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    for t in [
        "clinical.medication-coding.asserted",
        "clinical.medication-coding-correction.asserted",
    ] {
        let n: i64 = c
            .query_one(
                "SELECT count(*) FROM cairn_event_twin_check WHERE event_type = $1",
                &[&t],
            )
            .await
            .unwrap()
            .get(0);
        assert_eq!(n, 1, "{t} must be registered in the twin-check registry");
        let (mode, targets): (String, bool) = c
            .query_one(
                "SELECT mode, targets_other_author FROM event_type_class WHERE event_type = $1",
                &[&t],
            )
            .await
            .map(|r| (r.get(0), r.get(1)))
            .unwrap();
        // A correction ADDS a claim; the original stays in the log and the projection
        // picks a winner. targets_other_author = TRUE would route these through the
        // ADR-0043 owner gate and refuse a pharmacist correcting someone else's coding.
        assert_eq!(mode, "additive", "{t}");
        assert!(!targets, "{t} must not trip the suppression owner-gate");
    }
}

#[tokio::test]
async fn a_correction_claiming_both_replacement_and_strike_is_refused() {
    let Some(base) = cs() else { return };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let bad = serde_json::json!({
        "medication_id": Uuid::now_v7().to_string(),
        "corrects": Uuid::now_v7().to_string(),
        "coding": {"system": "drugref-moiety", "code": MOIETY_ATORVASTATIN, "display": "atorvastatin"},
        "strike": true
    });
    let e = submit_raw_overlay(
        &c,
        &sk,
        &kid,
        "submit_event",
        "clinical.medication-coding-correction.asserted",
        "clinical.medication-coding-correction/1",
        bad,
    )
    .await
    .expect_err("both is incoherent");
    assert!(db_msg(&e).contains("both"), "{}", db_msg(&e));
}

#[tokio::test]
async fn a_correction_claiming_neither_is_refused() {
    let Some(base) = cs() else { return };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let bad = serde_json::json!({
        "medication_id": Uuid::now_v7().to_string(),
        "corrects": Uuid::now_v7().to_string()
    });
    let e = submit_raw_overlay(
        &c,
        &sk,
        &kid,
        "submit_event",
        "clinical.medication-coding-correction.asserted",
        "clinical.medication-coding-correction/1",
        bad,
    )
    .await
    .expect_err("neither is incoherent");
    assert!(db_msg(&e).contains("strike"), "{}", db_msg(&e));
}

#[tokio::test]
async fn an_unknown_corrects_target_is_accepted() {
    let Some(base) = cs() else { return };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    // Offline-first: the corrected event may replicate later, or never. Refusing an
    // unknown target would make a correction impossible on a node that has not yet
    // received the coding it fixes.
    let ok = serde_json::json!({
        "medication_id": Uuid::now_v7().to_string(),
        "corrects": Uuid::now_v7().to_string(),
        "strike": true
    });
    submit_raw_overlay(
        &c,
        &sk,
        &kid,
        "submit_event",
        "clinical.medication-coding-correction.asserted",
        "clinical.medication-coding-correction/1",
        ok,
    )
    .await
    .expect("an unknown corrects target must be accepted");
}

#[tokio::test]
async fn a_non_uuid_corrects_is_refused() {
    let Some(base) = cs() else { return };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let bad = serde_json::json!({
        "medication_id": Uuid::now_v7().to_string(),
        "corrects": "not-a-uuid",
        "strike": true
    });
    let e = submit_raw_overlay(
        &c,
        &sk,
        &kid,
        "submit_event",
        "clinical.medication-coding-correction.asserted",
        "clinical.medication-coding-correction/1",
        bad,
    )
    .await
    .expect_err("corrects must be a uuid");
    assert!(db_msg(&e).contains("corrects"), "{}", db_msg(&e));
}

#[tokio::test]
async fn the_inherited_triple_checks_still_fire_on_an_overlay() {
    let Some(base) = cs() else { return };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    // Structural tier: refused at BOTH doors, like substance.term.
    for door in ["submit_event", "apply_remote_event"] {
        let bad = serde_json::json!({
            "medication_id": Uuid::now_v7().to_string(),
            "coding": {"system": "drugref-moiety", "code": MOIETY_ATORVASTATIN}
        });
        let e = submit_raw_overlay(
            &c, &sk, &kid, door,
            "clinical.medication-coding.asserted",
            "clinical.medication-coding/1",
            bad,
        )
        .await
        .expect_err("a coding missing display must be refused at both doors");
        assert!(db_msg(&e).contains("display"), "{}", db_msg(&e));
    }
    // Registry-derived tier: strict-submit, lenient-apply.
    let unknown = serde_json::json!({
        "medication_id": Uuid::now_v7().to_string(),
        "coding": {"system": "national-formulary-xyz", "code": "A10BA02", "display": "metformin"}
    });
    let e = submit_raw_overlay(
        &c, &sk, &kid, "submit_event",
        "clinical.medication-coding.asserted",
        "clinical.medication-coding/1",
        unknown.clone(),
    )
    .await
    .expect_err("an unregistered system must be refused at the local door");
    assert!(db_msg(&e).contains("national-formulary-xyz"), "{}", db_msg(&e));
    submit_raw_overlay(
        &c, &sk, &kid, "apply_remote_event",
        "clinical.medication-coding.asserted",
        "clinical.medication-coding/1",
        unknown,
    )
    .await
    .expect("a peer's unregistered system must be admitted, never refused");
}
```

Add the imports the harness needs: `use cairn_event::medication::SubstanceCoding; use cairn_event::{generate_key, sign, EventBody, SigningKey}; use cairn_node::db; use tokio_postgres::Client; use uuid::Uuid;`.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test -p cairn-node --test medication_coding_overlay`
Expected: FAIL — the types are unregistered, so `cairn_event_twin` finds no `check_fn` and the malformed payloads are accepted instead of refused.

- [ ] **Step 3: Extract the shared coding check in `db/041`**

The existing `cairn_check_medication_coding(p jsonb)` reads `p -> 'substance' -> 'coding'` — a path the overlay payloads do not have (theirs is `p -> 'coding'`). Split the path lookup from the checks. Insert **before** the existing function:

```sql
-- 2a. The coding-object checks, independent of WHERE the object sits in a payload.
--     Slice 6a's only caller reads substance.coding on the assertion; slice 6b's
--     overlay types carry the same object at payload.coding. Extracting the checks
--     keeps ONE definition of what a valid coding claim is — the two-tier split, the
--     canonical-uuid pin and the strict/lenient door behaviour cannot drift apart
--     between the inline and overlay paths.
--     p_prefix is the caller's message prefix (e.g. 'medication assertion:
--     substance.coding'), so each caller's refusals keep naming the field the way its
--     own authors wrote it.
CREATE OR REPLACE FUNCTION cairn_check_coding_object(c jsonb, p_prefix text)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    v_remote boolean := current_setting('cairn.remote_apply', true) = 'on';
    v_key    text;
    v_format text;
BEGIN
    IF c IS NULL OR jsonb_typeof(c) = 'null' THEN
        RETURN;
    END IF;
    IF jsonb_typeof(c) IS DISTINCT FROM 'object' THEN
        RAISE EXCEPTION '% must be an object {system, code, display} (ADR-0059)', p_prefix;
    END IF;
    FOREACH v_key IN ARRAY ARRAY['system', 'code', 'display'] LOOP
        IF jsonb_typeof(c -> v_key) IS DISTINCT FROM 'string'
           OR length(btrim(c ->> v_key)) = 0 THEN
            RAISE EXCEPTION
                '%.% must be a non-empty string (ADR-0059 decision 2 — display is the honest-degradation label)',
                p_prefix, v_key;
        END IF;
    END LOOP;
    IF v_remote THEN
        RETURN;
    END IF;
    SELECT s.code_format INTO v_format
        FROM medication_coding_system s WHERE s.system = c ->> 'system';
    IF v_format IS NULL THEN
        RAISE EXCEPTION
            '%: unknown coding system "%" — this door only authors codings it can vouch for; register it in medication_coding_system (ADR-0059 decision 7)',
            p_prefix, c ->> 'system';
    END IF;
    IF v_format = 'uuid' THEN
        IF NOT pg_input_is_valid(c ->> 'code', 'uuid') THEN
            RAISE EXCEPTION
                '%: coding system "%" requires a uuid code, got "%" (a drugref moiety id is a UUIDv5)',
                p_prefix, c ->> 'system', c ->> 'code';
        END IF;
        IF (c ->> 'code') IS DISTINCT FROM ((c ->> 'code')::uuid)::text THEN
            RAISE EXCEPTION
                '%: coding system "%" requires the canonical lowercase-hyphenated uuid form, got "%" (use % instead)',
                p_prefix, c ->> 'system', c ->> 'code', ((c ->> 'code')::uuid)::text;
        END IF;
    END IF;
END;
$$;
REVOKE EXECUTE ON FUNCTION cairn_check_coding_object(jsonb, text) FROM PUBLIC;
```

Then replace the body of `cairn_check_medication_coding(p jsonb)` with a thin delegation. The
structural-tier messages come out byte-identical to slice 6a's; the two registry-tier messages gain the
`substance.coding` path in their prefix (`medication assertion: substance.coding: unknown coding
system …` where 6a said `medication assertion: unknown coding system …`). That is acceptable and
deliberate — slice 6a's tests assert on substrings that survive the change (`national-formulary-xyz`,
`requires a uuid code`, `canonical`), and naming the field in a refusal is an improvement, not drift.
Do not contort the prefix to chase byte-identity:

```sql
CREATE OR REPLACE FUNCTION cairn_check_medication_coding(p jsonb)
RETURNS void LANGUAGE plpgsql AS $$
BEGIN
    PERFORM cairn_check_coding_object(
        p -> 'substance' -> 'coding', 'medication assertion: substance.coding');
END;
$$;
```

Keep the existing explanatory comments above it — move the long ones about JSON-null, the two tiers and the canonical-uuid pin onto `cairn_check_coding_object`, where the logic now lives.

- [ ] **Step 4: Write `db/042_medication_coding_overlay.sql` (registration + floor half)**

```sql
-- 042_medication_coding_overlay.sql — coding as a separately-authored act (ADR-0059
-- decision 3), slice 6b of clinical.medication (data-model §3.16).
--
-- Slice 6a shipped only INLINE coding on the assertion, so a medication recorded
-- uncoded could never become coded and a wrong coding could never be repaired. Two
-- overlay verbs close that:
--   clinical.medication-coding.asserted             — code a thread not coded inline
--   clinical.medication-coding-correction.asserted  — replace the claim, or STRIKE it
--
-- WHY A STRIKE EXISTS: a reviewer who establishes a medication is NOT metformin but
-- cannot say what it is has, without one, only two options — leave a known-wrong anchor
-- standing (it keeps feeding the dup-key and the group display), or invent a substitute
-- identity they cannot vouch for. The second is the fabrication principle 4 forbids.
-- Append-only means the correction event is the only repair path, so it must be able to
-- say "not that, and I don't know."
BEGIN;

-- 1. Both types are ADDITIVE and do not target another author. This matters: a coding
--    correction supersedes a claim that may have been authored by someone else, but it
--    ADDS a claim rather than suppressing one — the original stays in the log and the
--    projection picks a winner by HLC. Registering targets_other_author = TRUE would
--    route these through the ADR-0043 suppression owner-gate, which would refuse a
--    pharmacist correcting a coding authored by a different coder — contradicting the
--    premise of ADR-0059 decision 3. Same classification as
--    clinical.medication-dose-correction.asserted (db/032).
--    #214/#254: converge on replay via DO UPDATE, write-free once converged.
INSERT INTO event_type_class AS r (event_type, mode, targets_other_author) VALUES
    ('clinical.medication-coding.asserted',            'additive', FALSE),
    ('clinical.medication-coding-correction.asserted', 'additive', FALSE)
ON CONFLICT (event_type) DO UPDATE SET
    mode                 = EXCLUDED.mode,
    targets_other_author = EXCLUDED.targets_other_author
WHERE (r.mode, r.targets_other_author)
      IS DISTINCT FROM (EXCLUDED.mode, EXCLUDED.targets_other_author);

-- 2. The overlay floor. medication_id on both; corrects + coding/strike exclusivity on
--    the correction; the coding triple delegated to db/041's shared check so the
--    inline and overlay paths cannot drift.
CREATE OR REPLACE FUNCTION cairn_check_medication_coding_overlay(p_type text, b jsonb)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    p          jsonb := b -> 'payload';
    v_has_code boolean;
    v_strike   boolean;
BEGIN
    IF p IS NULL THEN
        RAISE EXCEPTION 'medication coding: missing payload';
    END IF;
    IF jsonb_typeof(p -> 'medication_id') IS DISTINCT FROM 'string'
       OR NOT pg_input_is_valid(p ->> 'medication_id', 'uuid') THEN
        RAISE EXCEPTION 'medication coding: medication_id must be a valid uuid string';
    END IF;

    IF p_type = 'clinical.medication-coding.asserted' THEN
        -- A plain coding overlay must actually carry a coding: unlike the assertion,
        -- where absent coding is the honest not-yet-coded floor, an overlay whose whole
        -- purpose is to code something has nothing to say without one.
        IF p -> 'coding' IS NULL OR jsonb_typeof(p -> 'coding') = 'null' THEN
            RAISE EXCEPTION 'medication coding: coding is required on a coding overlay (to un-code, use clinical.medication-coding-correction.asserted with strike)';
        END IF;
        PERFORM cairn_check_coding_object(p -> 'coding', 'medication coding: coding');
        RETURN;
    END IF;

    -- The correction verb.
    IF jsonb_typeof(p -> 'corrects') IS DISTINCT FROM 'string'
       OR NOT pg_input_is_valid(p ->> 'corrects', 'uuid') THEN
        RAISE EXCEPTION 'medication coding-correction: corrects must be a valid uuid string';
    END IF;
    -- Existence of the target is NOT required — the corrected event may replicate later,
    -- or never (offline-first; the db/032 dose-correction precedent).
    v_has_code := p -> 'coding' IS NOT NULL AND jsonb_typeof(p -> 'coding') IS DISTINCT FROM 'null';
    v_strike   := coalesce((p ->> 'strike')::boolean, FALSE);
    IF v_has_code AND v_strike THEN
        RAISE EXCEPTION 'medication coding-correction: a correction cannot both replace and strike — carry a coding OR strike, not both';
    END IF;
    IF NOT v_has_code AND NOT v_strike THEN
        RAISE EXCEPTION 'medication coding-correction: a correction must carry a replacement coding or strike = true (an omitted coding must never silently un-code a medication)';
    END IF;
    IF v_has_code THEN
        PERFORM cairn_check_coding_object(p -> 'coding', 'medication coding-correction: coding');
    END IF;
END;
$$;

-- 3. Register both verbs' floor + hard twin requirement (ADR-0048 registry). Placed
--    after the fn so db/005's fail-closed registration trigger sees it declared.
INSERT INTO cairn_event_twin_check AS r (event_type, check_fn, twin_required_msg) VALUES
    ('clinical.medication-coding.asserted',            'cairn_check_medication_coding_overlay',
     'medication coding requires a non-empty authored twin (§3.13/§3.16)'),
    ('clinical.medication-coding-correction.asserted', 'cairn_check_medication_coding_overlay',
     'medication coding correction requires a non-empty authored twin (§3.13/§3.16)')
ON CONFLICT (event_type) DO UPDATE SET
    check_fn          = EXCLUDED.check_fn,
    twin_required_msg = EXCLUDED.twin_required_msg
WHERE (r.check_fn, r.twin_required_msg)
      IS DISTINCT FROM (EXCLUDED.check_fn, EXCLUDED.twin_required_msg);

COMMIT;
```

- [ ] **Step 5: Bump the generation and both registry pins**

`crates/cairn-event/src/schema_generation.rs`: `SCHEMA_GENERATION` → `42`, doc example → `db/042_medication_coding_overlay.sql`.

`crates/cairn-node/src/db.rs`, after the `041` entry:

```rust
    // db/042 (ADR-0059 decision 3): the two coding-overlay event types — their floor and
    // registration. cairn-sync's list carries no medication files and legitimately lags (#284).
    (
        "042_medication_coding_overlay",
        include_str!("../../../db/042_medication_coding_overlay.sql"),
    ),
```

`crates/cairn-node/tests/twin_registry.rs:103`: `19` → `21`, and update the assertion message.
`db/tests/034_twin_registry_test.sql`: the `n <> 19` check → `21`, and its comment (`21 = 19 + 2 (the ADR-0059 coding overlays)`).

- [ ] **Step 6: Run the tests to verify they pass**

Separate foreground calls: `cargo test -p cairn-node --test medication_coding_overlay`, `--test medication_coding`, `--test twin_registry`, `cargo test -p cairn-event`, `cargo test -p cairn-node --lib`, then `bash scripts/run-db-sql-tests.sh` if it accepts a connection string (check its usage line first; it runs the `db/tests/*.sql` mirrors).
Expected: all PASS. `medication_coding` passing unchanged is the proof that the db/041 extraction preserved slice 6a's refusal messages.

- [ ] **Step 7: Commit**

```bash
git add db/041_medication_coding.sql db/042_medication_coding_overlay.sql crates/cairn-event/src/schema_generation.rs crates/cairn-node/src/db.rs crates/cairn-node/tests db/tests/034_twin_registry_test.sql
git commit -m "feat(medication 6b): the coding-overlay floor + both type registrations

db/041's coding checks are extracted into cairn_check_coding_object so the
inline and overlay paths share ONE definition of a valid coding claim —
the two-tier split, the canonical-uuid pin and the strict/lenient door
behaviour cannot drift apart. db/042 registers both overlay types as
('additive', FALSE): a correction adds a claim rather than suppressing
one, and targets_other_author = TRUE would route it through the ADR-0043
owner gate and refuse a pharmacist correcting someone else's coding.

Twin registry 19 -> 21 in both places. SCHEMA_GENERATION 41 -> 42.

Refs ADR-0059, #254"
```

---

### Task 4: The projection — `struck`, both apply fns, cross-node convergence

**Files:**
- Modify: `db/042_medication_coding_overlay.sql` (schema change + apply fns + registry, before its `COMMIT`)
- Modify: `crates/cairn-node/tests/projection_registry.rs` (22 → 24)
- Modify: `db/tests/039_projection_registry_test.sql` (25 → 27)
- Modify: `crates/cairn-node/tests/medication_coding_overlay.rs` (projection tests)
- Modify: `crates/cairn-sync/tests/clinical_pull.rs` (cross-node convergence)

**Interfaces:**
- Consumes: `medication_coding` (slice 6a: `medication_id` PK, `patient_id`, `coding_system`, `coding_code`, `coding_display`, `hlc_wall`, `hlc_counter`, `origin`, `content_address`, `updated_at`); `cairn_hlc_overlay_wins(...)`; `cairn_guard_medication_patient(p_med, p_patient, p_ca)`; `cairn_clear_payload(e)`.
- Produces: `medication_coding.struck BOOLEAN NOT NULL DEFAULT FALSE`, nullable anchor columns, and apply fns `medication_coding_apply(event_log)` / `medication_coding_correction_apply(event_log)`.

- [ ] **Step 1: Write the failing tests**

Append to `crates/cairn-node/tests/medication_coding_overlay.rs`:

```rust
async fn coding_state(c: &Client, med: Uuid) -> Option<(Option<String>, Option<String>, bool)> {
    c.query_opt(
        "SELECT coding_system, coding_display, struck \
           FROM medication_coding WHERE medication_id = $1::text::uuid",
        &[&med.to_string()],
    )
    .await
    .unwrap()
    .map(|r| (r.get(0), r.get(1), r.get(2)))
}

/// Assert a medication uncoded, then return its thread id.
async fn assert_uncoded(c: &mut Client, sk: &SigningKey, kid: &str, patient: Uuid) -> Uuid {
    let input = cairn_node::medication::AssertMedicationInput {
        term: "little white pill",
        coding: None,
        formulation: None,
        dose_amount: None,
        dose_unit: None,
        sig: None,
        info_source: "patient-reported",
        started: None,
        started_precision: None,
    };
    cairn_node::medication::assert_medication(c, sk, kid, "test-node", patient, &input, None, None)
        .await
        .unwrap()
}

#[tokio::test]
async fn an_overlay_codes_a_previously_uncoded_thread() {
    let Some(base) = cs() else { return };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient = Uuid::now_v7();
    let med = assert_uncoded(&mut c, &sk, &kid, patient).await;
    assert_eq!(coding_state(&c, med).await, None, "uncoded to begin with");

    cairn_node::medication::code_medication(
        &mut c, &sk, &kid, "test-node", patient, med,
        &cairn_node::medication::CodeMedicationInput { coding: coding() },
        None, None,
    )
    .await
    .unwrap();
    assert_eq!(
        coding_state(&c, med).await,
        Some((
            Some("drugref-moiety".to_string()),
            Some("atorvastatin".to_string()),
            false
        ))
    );
}

#[tokio::test]
async fn a_correction_replaces_the_claim() {
    let Some(base) = cs() else { return };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient = Uuid::now_v7();
    let med = assert_uncoded(&mut c, &sk, &kid, patient).await;
    let coding_event = cairn_node::medication::code_medication(
        &mut c, &sk, &kid, "test-node", patient, med,
        &cairn_node::medication::CodeMedicationInput { coding: coding() },
        None, None,
    )
    .await
    .unwrap();

    const MOIETY_METFORMIN: &str = "3c7d9a52-4e18-5f60-8b21-6d4a0e9c7f33";
    cairn_node::medication::correct_medication_coding(
        &mut c, &sk, &kid, "test-node", patient, med,
        &cairn_node::medication::CorrectCodingInput {
            corrects: coding_event,
            coding: Some(SubstanceCoding {
                system: "drugref-moiety",
                code: MOIETY_METFORMIN,
                display: "metformin",
            }),
            strike: false,
            note: Some("misread the brand"),
        },
        None, None,
    )
    .await
    .unwrap();
    assert_eq!(
        coding_state(&c, med).await,
        Some((
            Some("drugref-moiety".to_string()),
            Some("metformin".to_string()),
            false
        ))
    );
}

#[tokio::test]
async fn a_strike_nulls_the_anchor_and_flags_the_row() {
    let Some(base) = cs() else { return };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient = Uuid::now_v7();
    let med = assert_uncoded(&mut c, &sk, &kid, patient).await;
    let coding_event = cairn_node::medication::code_medication(
        &mut c, &sk, &kid, "test-node", patient, med,
        &cairn_node::medication::CodeMedicationInput { coding: coding() },
        None, None,
    )
    .await
    .unwrap();

    // The clinical case: established as NOT that substance, with no replacement known.
    cairn_node::medication::correct_medication_coding(
        &mut c, &sk, &kid, "test-node", patient, med,
        &cairn_node::medication::CorrectCodingInput {
            corrects: coding_event,
            coding: None,
            strike: true,
            note: Some("not atorvastatin; substance unidentified"),
        },
        None, None,
    )
    .await
    .unwrap();
    assert_eq!(
        coding_state(&c, med).await,
        Some((None, None, true)),
        "a strike NULLs the anchor and flags the row — the thread is honestly uncoded again"
    );
}

#[tokio::test]
async fn an_overlay_for_an_absent_thread_still_lands() {
    let Some(base) = cs() else { return };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    // Offline-first / arrival-order independence: the assertion may replicate later.
    let orphan = Uuid::now_v7();
    cairn_node::medication::code_medication(
        &mut c, &sk, &kid, "test-node", Uuid::now_v7(), orphan,
        &cairn_node::medication::CodeMedicationInput { coding: coding() },
        None, None,
    )
    .await
    .expect("a coding for a not-yet-present thread must be accepted");
    assert!(coding_state(&c, orphan).await.is_some());
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `CAIRN_TEST_PG=… cargo test -p cairn-node --test medication_coding_overlay`
Expected: FAIL — `column "struck" does not exist`, and the coding rows are never written because no apply fn is registered.

- [ ] **Step 3: Add the schema change and both apply fns to `db/042`**

Insert before `COMMIT;`:

```sql
-- 4. A struck coding needs a row that says "deliberately not coded" rather than no row
--    at all: deleting the row would break arrival-order independence (a coding event
--    arriving AFTER the strike, with a lower HLC, would have nothing to lose against
--    and would silently win). So the anchor columns become nullable and the row carries
--    a flag. Dropping NOT NULL is a widening — every existing row still satisfies the
--    looser constraint — and the ADD COLUMN is the #207 paired-ALTER an upgraded
--    in-place database needs.
ALTER TABLE medication_coding ALTER COLUMN coding_system  DROP NOT NULL;
ALTER TABLE medication_coding ALTER COLUMN coding_code    DROP NOT NULL;
ALTER TABLE medication_coding ALTER COLUMN coding_display DROP NOT NULL;
ALTER TABLE medication_coding ADD COLUMN IF NOT EXISTS struck BOOLEAN NOT NULL DEFAULT FALSE;

-- 5. Apply the plain coding overlay. Same table, same winner rule as the inline coding
--    (db/031) — that is what makes this slice additive: no view is re-routed, and every
--    consumer of medication_coding keeps working untouched.
CREATE OR REPLACE FUNCTION medication_coding_apply(e event_log)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    p jsonb := cairn_clear_payload(e);
BEGIN
    IF p IS NULL THEN RETURN; END IF;
    -- #192: a coding event must not silently re-home a thread onto another chart.
    PERFORM cairn_guard_medication_patient(
        (p ->> 'medication_id')::uuid, e.patient_id, e.content_address);

    INSERT INTO medication_coding
        (medication_id, patient_id, coding_system, coding_code, coding_display, struck,
         hlc_wall, hlc_counter, origin, content_address)
    VALUES (
        (p ->> 'medication_id')::uuid, e.patient_id,
        p -> 'coding' ->> 'system',
        p -> 'coding' ->> 'code',
        p -> 'coding' ->> 'display',
        FALSE,
        e.hlc_wall, e.hlc_counter, e.node_origin, e.content_address)
    ON CONFLICT (medication_id) DO UPDATE SET
        patient_id      = EXCLUDED.patient_id,
        coding_system   = EXCLUDED.coding_system,
        coding_code     = EXCLUDED.coding_code,
        coding_display  = EXCLUDED.coding_display,
        struck          = EXCLUDED.struck,
        hlc_wall        = EXCLUDED.hlc_wall,
        hlc_counter     = EXCLUDED.hlc_counter,
        origin          = EXCLUDED.origin,
        content_address = EXCLUDED.content_address,
        updated_at      = clock_timestamp()
    WHERE cairn_hlc_overlay_wins(
        EXCLUDED.hlc_wall, EXCLUDED.hlc_counter, EXCLUDED.origin, EXCLUDED.content_address,
        medication_coding.hlc_wall, medication_coding.hlc_counter,
        medication_coding.origin, medication_coding.content_address);
    RETURN;
END;
$$;
REVOKE EXECUTE ON FUNCTION medication_coding_apply(event_log) FROM PUBLIC;

-- 6. Apply a correction: a replacement writes the new triple, a strike writes NULLs plus
--    struck = TRUE. The NULL anchor is what makes the downstream degradation automatic —
--    the dup-key's coalesce falls back to the term branch on its own, and the
--    anchor-conflict view's count(DISTINCT ...) ignores NULLs.
CREATE OR REPLACE FUNCTION medication_coding_correction_apply(e event_log)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    p        jsonb := cairn_clear_payload(e);
    v_struck boolean;
BEGIN
    IF p IS NULL THEN RETURN; END IF;
    PERFORM cairn_guard_medication_patient(
        (p ->> 'medication_id')::uuid, e.patient_id, e.content_address);
    -- The floor guarantees exactly one of coding / strike, so this is a clean either-or.
    -- jsonb_typeof(...) = 'null' is checked because an explicit JSON null reads as the
    -- jsonb value 'null', not SQL NULL (the same trap db/031's inline write documents).
    v_struck := p -> 'coding' IS NULL OR jsonb_typeof(p -> 'coding') = 'null';

    INSERT INTO medication_coding
        (medication_id, patient_id, coding_system, coding_code, coding_display, struck,
         hlc_wall, hlc_counter, origin, content_address)
    VALUES (
        (p ->> 'medication_id')::uuid, e.patient_id,
        CASE WHEN v_struck THEN NULL ELSE p -> 'coding' ->> 'system'  END,
        CASE WHEN v_struck THEN NULL ELSE p -> 'coding' ->> 'code'    END,
        CASE WHEN v_struck THEN NULL ELSE p -> 'coding' ->> 'display' END,
        v_struck,
        e.hlc_wall, e.hlc_counter, e.node_origin, e.content_address)
    ON CONFLICT (medication_id) DO UPDATE SET
        patient_id      = EXCLUDED.patient_id,
        coding_system   = EXCLUDED.coding_system,
        coding_code     = EXCLUDED.coding_code,
        coding_display  = EXCLUDED.coding_display,
        struck          = EXCLUDED.struck,
        hlc_wall        = EXCLUDED.hlc_wall,
        hlc_counter     = EXCLUDED.hlc_counter,
        origin          = EXCLUDED.origin,
        content_address = EXCLUDED.content_address,
        updated_at      = clock_timestamp()
    WHERE cairn_hlc_overlay_wins(
        EXCLUDED.hlc_wall, EXCLUDED.hlc_counter, EXCLUDED.origin, EXCLUDED.content_address,
        medication_coding.hlc_wall, medication_coding.hlc_counter,
        medication_coding.origin, medication_coding.content_address);
    RETURN;
END;
$$;
REVOKE EXECUTE ON FUNCTION medication_coding_correction_apply(event_log) FROM PUBLIC;

-- 7. Register both apply fns with the ADR-0057 dispatcher. medication_patient_conflict_flag
--    is in both inventories because cairn_guard_medication_patient can write it on a
--    remote-apply conflict — rebuild-scope metadata must be exhaustive, never knowingly
--    incomplete. NOTE: medication_coding is now written by THREE event types, so
--    cairn_reproject will refuse a narrow single-type prefix rebuild over it (db/039) —
--    correct, and the reason that refusal exists.
INSERT INTO cairn_projection_apply AS r (event_type, apply_fn, projection_tables, run_order, heal_safe) VALUES
    ('clinical.medication-coding.asserted',            'medication_coding_apply',
     ARRAY['medication_coding', 'medication_patient_conflict_flag'], 25, TRUE),
    ('clinical.medication-coding-correction.asserted', 'medication_coding_correction_apply',
     ARRAY['medication_coding', 'medication_patient_conflict_flag'], 26, TRUE)
ON CONFLICT (event_type, apply_fn) DO UPDATE SET
    projection_tables = EXCLUDED.projection_tables,
    run_order         = EXCLUDED.run_order,
    heal_safe         = EXCLUDED.heal_safe
WHERE (r.projection_tables, r.run_order, r.heal_safe)
      IS DISTINCT FROM (EXCLUDED.projection_tables, EXCLUDED.run_order, EXCLUDED.heal_safe);
```

- [ ] **Step 4: Update both projection-registry pins**

`crates/cairn-node/tests/projection_registry.rs`: the pinned count `22` → `24` (and the doc comment at the top of the file).
`db/tests/039_projection_registry_test.sql`: `25` → `27` (product 24 + db/008's 3), and its comment.

- [ ] **Step 5: Add the cross-node convergence test**

In `crates/cairn-sync/tests/clinical_pull.rs`, extend the medication test that already asserts a coded statement converges on node B: after the pull, submit a coding overlay and then a strike on node A, pull again, and assert node B's `medication_coding` row shows first the overlay's triple and then `struck = true` with a NULL anchor. Follow the file's existing pull idiom rather than inventing one.

- [ ] **Step 6: Run the tests**

Separate foreground calls: `cargo test -p cairn-node --test medication_coding_overlay`, `--test medication_coding`, `--test projection_registry`, `--test medication`, `cargo test -p cairn-sync --test clinical_pull`. Then run `--test medication_coding_overlay` a SECOND time (replay safety after a schema change).
Expected: all PASS.

- [ ] **Step 7: Commit**

```bash
git add db/042_medication_coding_overlay.sql crates/cairn-node/tests db/tests/039_projection_registry_test.sql crates/cairn-sync/tests/clinical_pull.rs
git commit -m "feat(medication 6b): project coding overlays and strikes

Both apply fns write the existing medication_coding table under the
existing overlay-winner rule, so the slice stays additive. A strike NULLs
the anchor and sets struck = TRUE rather than deleting the row: deleting
would break arrival-order independence, because a lower-HLC coding
arriving after the strike would have nothing to lose against.

Projection registry 22 -> 24 in both places.

Refs ADR-0059, #192"
```

---

### Task 5: Struck-aware downstream — the group-display predicate and the worklist

**Files:**
- Modify: `db/033_medication_reconciliation.sql` (the `medication_group_display` ORDER BY predicate)
- Modify: `db/042_medication_coding_overlay.sql` (the worklist view, before `COMMIT`)
- Modify: `crates/cairn-node/tests/medication_coding_overlay.rs`

**Interfaces:**
- Consumes: `medication_coding.struck` and the nullable anchor (Task 4).
- Produces: view `patient_medication_uncoded (patient_id, medication_id, term, previously_struck, asserted_at)`.

- [ ] **Step 1: Write the failing tests**

Append to `crates/cairn-node/tests/medication_coding_overlay.rs`:

```rust
#[tokio::test]
async fn a_struck_coding_stops_winning_the_group_display() {
    let Some(base) = cs() else { return };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient = Uuid::now_v7();

    // A vague thread and a coded thread, reconciled into one group. Slice 6a makes the
    // coded member the group's display; after a strike it must stop being preferred,
    // or the group reads under a coding that was explicitly retracted.
    let vague = assert_uncoded(&mut c, &sk, &kid, patient).await;
    let coded_input = cairn_node::medication::AssertMedicationInput {
        term: "Lipitor",
        coding: Some(coding()),
        formulation: None,
        dose_amount: None,
        dose_unit: None,
        sig: None,
        info_source: "patient-reported",
        started: None,
        started_precision: None,
    };
    let coded = cairn_node::medication::assert_medication(
        &mut c, &sk, &kid, "test-node", patient, &coded_input, None, None,
    )
    .await
    .unwrap();
    cairn_node::medication::reconcile_medications(
        &mut c, &sk, &kid, "test-node", patient, vague, coded,
        &cairn_node::medication::ReconcileInput {
            provenance: "clinician-judgment",
            reason: None,
        },
        None, None,
    )
    .await
    .unwrap();
    let display: Option<String> = c
        .query_one(
            "SELECT coding_display FROM medication_group_display WHERE patient_id = $1::text::uuid",
            &[&patient.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(display.as_deref(), Some("atorvastatin"));

    // Strike the coded member's coding.
    cairn_node::medication::correct_medication_coding(
        &mut c, &sk, &kid, "test-node", patient, coded,
        &cairn_node::medication::CorrectCodingInput {
            corrects: Uuid::now_v7(), // an unknown target is lawful (offline-first)
            coding: None,
            strike: true,
            note: None,
        },
        None, None,
    )
    .await
    .unwrap();
    let after: Option<String> = c
        .query_one(
            "SELECT coding_display FROM medication_group_display WHERE patient_id = $1::text::uuid",
            &[&patient.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        after, None,
        "a struck coding must stop being preferred — the group must not read under a retracted coding"
    );
}

#[tokio::test]
async fn a_struck_thread_returns_to_the_term_dup_key() {
    let Some(base) = cs() else { return };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient = Uuid::now_v7();

    // Two threads with the SAME term, one coded: they key apart (6a's documented
    // coded<->uncoded blind spot). Striking the coding makes both key on the term, so
    // the duplicate flag appears — the degradation falling out of the existing coalesce.
    let mut input = cairn_node::medication::AssertMedicationInput {
        term: "atorvastatin",
        coding: Some(coding()),
        formulation: None,
        dose_amount: None,
        dose_unit: None,
        sig: None,
        info_source: "patient-reported",
        started: None,
        started_precision: None,
    };
    let coded = cairn_node::medication::assert_medication(
        &mut c, &sk, &kid, "test-node", patient, &input, None, None,
    )
    .await
    .unwrap();
    input.coding = None;
    cairn_node::medication::assert_medication(
        &mut c, &sk, &kid, "test-node", patient, &input, None, None,
    )
    .await
    .unwrap();
    let before: i64 = c
        .query_one(
            "SELECT count(*) FROM patient_medication_reconciliation_flag WHERE patient_id = $1::text::uuid",
            &[&patient.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(before, 0, "coded and uncoded key apart (6a's documented gap)");

    cairn_node::medication::correct_medication_coding(
        &mut c, &sk, &kid, "test-node", patient, coded,
        &cairn_node::medication::CorrectCodingInput {
            corrects: Uuid::now_v7(),
            coding: None,
            strike: true,
            note: None,
        },
        None, None,
    )
    .await
    .unwrap();
    let after: i64 = c
        .query_one(
            "SELECT count(*) FROM patient_medication_reconciliation_flag WHERE patient_id = $1::text::uuid",
            &[&patient.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(after, 1, "with the anchor struck, both threads key on the term again");
}

#[tokio::test]
async fn the_worklist_distinguishes_never_coded_from_struck() {
    let Some(base) = cs() else { return };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient = Uuid::now_v7();

    let never = assert_uncoded(&mut c, &sk, &kid, patient).await;
    let struck_thread = assert_uncoded(&mut c, &sk, &kid, patient).await;
    let ev = cairn_node::medication::code_medication(
        &mut c, &sk, &kid, "test-node", patient, struck_thread,
        &cairn_node::medication::CodeMedicationInput { coding: coding() },
        None, None,
    )
    .await
    .unwrap();
    cairn_node::medication::correct_medication_coding(
        &mut c, &sk, &kid, "test-node", patient, struck_thread,
        &cairn_node::medication::CorrectCodingInput {
            corrects: ev,
            coding: None,
            strike: true,
            note: None,
        },
        None, None,
    )
    .await
    .unwrap();
    let coded_thread = assert_uncoded(&mut c, &sk, &kid, patient).await;
    cairn_node::medication::code_medication(
        &mut c, &sk, &kid, "test-node", patient, coded_thread,
        &cairn_node::medication::CodeMedicationInput { coding: coding() },
        None, None,
    )
    .await
    .unwrap();

    let rows = c
        .query(
            "SELECT medication_id, previously_struck FROM patient_medication_uncoded \
               WHERE patient_id = $1::text::uuid ORDER BY previously_struck",
            &[&patient.to_string()],
        )
        .await
        .unwrap();
    assert_eq!(rows.len(), 2, "the coded thread must not appear in the worklist");
    assert_eq!(rows[0].get::<_, Uuid>(0), never);
    assert!(!rows[0].get::<_, bool>(1), "never coded");
    assert_eq!(rows[1].get::<_, Uuid>(0), struck_thread);
    assert!(
        rows[1].get::<_, bool>(1),
        "a struck thread is genuinely uncoded and must stay in the queue, flagged"
    );
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `CAIRN_TEST_PG=… cargo test -p cairn-node --test medication_coding_overlay`
Expected: FAIL — `relation "patient_medication_uncoded" does not exist`, and the group-display test fails because a struck row is still preferred.

- [ ] **Step 3: Fix the group-display predicate (db/033)**

The prefer-coded key tests row EXISTENCE; a struck coding leaves a row whose anchor is NULL. Change it to test the ANCHOR, and extend the comment:

```sql
ORDER BY g.group_id,
         -- ADR-0059 slice 6b: prefer a member with a LIVE anchor, not merely a
         -- medication_coding ROW. A struck coding leaves a row with NULL columns, and
         -- preferring it would make the group read under a coding somebody explicitly
         -- retracted.
         (mc.coding_code IS NOT NULL) DESC,
         mc.coding_system COLLATE "C", mc.coding_code COLLATE "C",
         (s.medication_id = g.group_id) DESC,
         s.medication_id;
```

- [ ] **Step 4: Add the worklist view to db/042**

Insert before `COMMIT;`:

```sql
-- 8. The coder worklist (ADR-0059 decision 3: an uncoded medication is "an honest
--    not-yet-coded state routed to a coder worklist, never a forced guess"). Active
--    threads with no LIVE anchor: never coded (no row) or struck (row, NULL anchor).
--    previously_struck separates them, and the distinction is clinical, not
--    bookkeeping: "nobody has coded this yet" invites a coder to code it, while "a
--    reviewer established this is NOT what it was coded as" warns against re-coding it
--    from the same weak evidence that produced the error. Both appear — a struck coding
--    is genuinely uncoded and must not vanish from the queue.
--    Created only here, so it never enters the multi-file view-replay problem (#207).
CREATE OR REPLACE VIEW patient_medication_uncoded AS
SELECT s.patient_id,
       s.medication_id,
       s.term,
       coalesce(mc.struck, FALSE) AS previously_struck,
       to_timestamp(s.hlc_wall / 1000.0) AS asserted_at
FROM medication_statement s
LEFT JOIN medication_coding mc USING (medication_id)
WHERE mc.coding_code IS NULL
  AND NOT EXISTS (SELECT 1 FROM medication_cessation c WHERE c.medication_id = s.medication_id);
GRANT SELECT ON patient_medication_uncoded TO cairn_agent;
```

- [ ] **Step 5: Run the tests to verify they pass**

Separate foreground calls: `cargo test -p cairn-node --test medication_coding_overlay`, `--test medication_coding`, `--test medication_reconciliation`, `--test medication_dose`. Then re-run the first two (replay safety after touching a db/033 view).
Expected: all PASS.

- [ ] **Step 6: Commit**

```bash
git add db/033_medication_reconciliation.sql db/042_medication_coding_overlay.sql crates/cairn-node/tests/medication_coding_overlay.rs
git commit -m "feat(medication 6b): struck-aware group display + the coder worklist

The prefer-coded ordering tested whether a medication_coding ROW existed;
a struck coding leaves a row with a NULL anchor, so a retracted coding
would still have won the group display. It now tests the anchor.

patient_medication_uncoded routes uncoded threads to whoever codes them,
flagging previously-struck ones separately: 'not yet coded' invites
coding, 'established as wrong' warns against re-coding from the same
evidence.

Refs ADR-0059"
```

---

### Task 6: Documentation

**Files:**
- Modify: `docs/ROADMAP.md` (add Slice 57 after Slice 56), `docs/HANDOVER.md` (⇒ NEXT + a session block)

**Do NOT open a pull request** — the controller handles that after the final review.

- [ ] **Step 1: Add ROADMAP Slice 57**

12–18 lines in the established house voice (Slices 53–56 are the model): what shipped, naming the commits and mechanisms; the strike's rationale; the group-display predicate fix reaching back into 6a; and honestly what is NOT closed (no drugref code, the §5.9 class still owed via #294, coded↔uncoded still open, no coding UI).

- [ ] **Step 2: Update HANDOVER**

Rewrite `⇒ NEXT` so it names the genuinely next thing now that ADR-0059 is fully implemented — the candidates are the drugref term→anchor lookup (§9 advisory tier), the §5.9 safety-projection slice (#294/#232), and the med-list UI slice (#288). Add a session block for slice 6b in the established style, and carry forward the ROADMAP-pruning decision that is still owed.

- [ ] **Step 3: Commit**

```bash
git add docs/HANDOVER.md docs/ROADMAP.md
git commit -m "docs(medication 6b): ROADMAP Slice 57 + HANDOVER currency"
```

## Self-Review

- **Spec coverage:** event types → Task 1; orchestrators + CLI → Task 2; floor + registration + the DRY extraction → Task 3; projection, `struck`, cross-node → Task 4; group-display predicate + worklist → Task 5; docs → Task 6. The spec's risk about `medication_group_display` is Task 5 Step 3. Every spec section maps to a task.
- **Type consistency:** `SubstanceCoding` is used identically throughout; `MedicationCoding` / `MedicationCodingCorrection` (cairn-event) are distinct from `CodeMedicationInput` / `CorrectCodingInput` (cairn-node) by design, mirroring how `MedicationAssertion` and `AssertMedicationInput` already differ; `cairn_check_coding_object(jsonb, text)` has one signature, used by both callers.
- **Known judgement calls for the implementer:** Task 4 Step 5 and Task 6 describe their edits in prose rather than quoting final code, because both depend on surrounding text (the existing `clinical_pull` pull idiom; the current HANDOVER wording) that must be read first. Both name exactly what to assert or write.
