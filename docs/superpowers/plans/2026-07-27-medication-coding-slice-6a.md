# `clinical.medication` slice 6a — inline `substance.coding` Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Carry a medication's drug-identity coding — drugref's immortal `moiety_uuid` under a structured
`substance.coding {system, code, display}` — on `clinical.medication.asserted`, retiring the reserved
`substance.inn_code` slot, and use it to close the coded↔coded duplicate blind spot and give a reconciled
group a canonical drug identity.

**Architecture:** The coding is an optional object inside the existing assertion payload (no new event
type in this slice). Its floor lives in a new `db/041_medication_coding.sql` — a registry table of
admitted coding systems plus a check function called from db/031's existing per-type floor, splitting into
a *structural* tier (refused at both doors) and a *registry-derived* tier (strict-submit, lenient-apply).
Its projection is a separate `medication_coding` table, so slice 6b's coding-overlay event types add rows
rather than rewriting view bodies. Read views gain three trailing columns; the E1 dup-key keys on the
`(system, code)` pair; `medication_group_display` prefers a coded member.

**Tech Stack:** Rust (`cairn-event` pure builders, `cairn-node` orchestrator + clap CLI, `cairn-sync`
tests), PostgreSQL 18 + `cairn_pgx`, plpgsql in-DB floor, tokio-postgres integration tests.

## Global Constraints

- **Design source:** [`docs/superpowers/specs/2026-07-27-medication-drug-coding-slice-6a-design.md`](../specs/2026-07-27-medication-drug-coding-slice-6a-design.md); the decision record is [ADR-0059](../../spec/decisions/0059-medication-drug-coding-drugref-moiety-anchor.md). Where they disagree, the ADR wins.
- **Licence:** AGPL-3.0. No new dependency is added by this slice; if one becomes tempting, stop and ask.
- **TDD:** every step writes the failing test first, runs it to see it fail for the right reason, then the minimal implementation.
- **No drugref code enters the tree in this slice.** Coding values are caller-supplied. Task 6 adds a source guard that pins this.
- **Uncoded stays first-class** (principle 4): `substance.coding` absent must remain valid at every layer, and no field may become required.
- **Never hard-code cryptographic material in tests** (house rule 6) — reuse the existing `generate_key()` / `seal_event_payload` helpers.
- **Full-workspace test runs only:** `cargo test --workspace`. A per-crate run misses the cross-crate call-site breaks this slice creates. `cargo test | tail` hides cargo's exit code — never pipe it.
- **DB-gated tests need** `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test"`; the multi-node suites additionally need `CAIRN_TEST_PG2` / `CAIRN_TEST_PG3` (`cairn_test2` / `cairn_test3` on the same cluster).
- **A widened `CREATE TABLE IF NOT EXISTS` needs a paired `ALTER TABLE … ADD COLUMN IF NOT EXISTS`** (#207). A widened view needs the *identical* trailing column list in **every** db file that creates it.
- **Registry rows converge on replay:** `ON CONFLICT … DO UPDATE … WHERE (…) IS DISTINCT FROM (…)` (#214), never `DO NOTHING`.
- **A new `db/NNN_*.sql` bumps `SCHEMA_GENERATION`** (`crates/cairn-event/src/schema_generation.rs`) and must be added to cairn-node's `SCHEMA` list in `crates/cairn-node/src/db.rs`.

## Paper-parity benchmark (§1.2)

- **Paper counterpart:** writing a drug name on a paper medication list — the clinician writes
  *"atorvastatin"*, or *"Lipitor"*, or *"little white pill"*. **N = 1** human act. Nothing on paper
  forces a code, and nothing on paper refuses the vague answer.
- **Steps:** paper **N = 1** → architecture-forced **M = 1** → UI bundling target **K = 1**. The coding
  adds **zero** forced human acts: `substance.coding` is optional in the builder, optional at the floor,
  and absent from the projection when not supplied. Task 6 pins this with a test that an assertion
  carrying no coding passes both doors unchanged, so a later slice cannot quietly make coding required.
  `M > N` here would be an architecture defect to file under house rule 5.
- **Time + cognitive load:** no budget is measured by this slice, and that is a deliberate, bounded
  deferral: 6a exposes only a CLI test/ops surface, not the clinician surface a budget would measure.
  The measurement is owed by the slice that first exposes a coding UI with type-ahead auto-fill (the
  ADR-0059 follow-on, in the [#288](https://github.com/cairn-ehr/cairn-ehr/issues/288) med-list
  neighbourhood), whose target is that picking a suggestion costs **zero** extra keystrokes over typing
  the term alone.

## File Structure

**Created:**

| File | Responsibility |
|---|---|
| `db/041_medication_coding.sql` | The `medication_coding_system` vocabulary registry + seed, and `cairn_check_medication_coding(jsonb)` — the two-tier floor check. |
| `crates/cairn-node/tests/medication_coding.rs` | DB-gated floor + projection + reconciliation tests for the coding. |
| `crates/cairn-node/tests/no_drugref_dependency.rs` | Source guard: no `db/` file and no crate references drugref (honest degradation by construction). |

**Modified:**

| File | Change |
|---|---|
| `crates/cairn-event/src/medication/assert.rs` | `SubstanceCoding` struct; `inn_code` field removed; payload + twin carry the coding. |
| `crates/cairn-event/src/medication/mod.rs` | Re-export `SubstanceCoding`. |
| `crates/cairn-event/src/schema_generation.rs` | `SCHEMA_GENERATION` 40 → 41. |
| `crates/cairn-node/src/medication/assert.rs` | `AssertMedicationInput.coding`; new pure `coding_from_parts`. |
| `crates/cairn-node/src/main.rs` | CLI: `--inn-code` → `--coding-system` / `--coding-code` / `--coding-display`. |
| `crates/cairn-node/src/db.rs` | `db/041` added to the `SCHEMA` list. |
| `db/031_medication.sql` | Retired-slot refusal + coding check call; `medication_coding` table; apply-fn write + registry inventory; widened views; new dup-key. |
| `db/032_medication_dose.sql` | Widened `patient_medication_current` / `_past` (identical trailing columns). |
| `db/033_medication_reconciliation.sql` | Widened current/past + `medication_group_display`; prefer-coded ordering; new dup-key; `medication_group_coding_conflict`. |
| ~12 test files across `cairn-node`, `cairn-sync`, `cairn-event` | `inn_code: None` → `coding: None`; `TRUNCATE medication_coding` in the medication setup helpers. |
| `docs/HANDOVER.md`, `docs/ROADMAP.md` | Slice 56 record + pruning. |

---

### Task 1: The wire shape in `cairn-event`

**Files:**
- Modify: `crates/cairn-event/src/medication/assert.rs`
- Modify: `crates/cairn-event/src/medication/mod.rs:20`
- Test: `crates/cairn-event/src/medication/assert.rs` (the in-file `mod tests`)

**Interfaces:**
- Consumes: nothing (first task).
- Produces: `pub struct SubstanceCoding<'a> { pub system: &'a str, pub code: &'a str, pub display: &'a str }`;
  `MedicationAssertion<'a>.coding: Option<SubstanceCoding<'a>>` replacing `inn_code: Option<&'a str>`;
  unchanged signatures `medication_assertion_body(&MedicationAssertion) -> serde_json::Value` and
  `render_medication_twin(&MedicationAssertion) -> String`.

- [ ] **Step 1: Write the failing tests**

In `crates/cairn-event/src/medication/assert.rs`, inside `mod tests`, change `full_assertion()` to build a
coded brand-name assertion and replace the two `inn_code` assertions with these tests:

```rust
    fn full_assertion() -> MedicationAssertion<'static> {
        MedicationAssertion {
            medication_id: "11111111-1111-7111-8111-111111111111",
            term: "Lipitor",
            coding: Some(SubstanceCoding {
                system: "drugref-moiety",
                code: "0f8c4b1e-1b7a-5c2d-9a3e-2b6f7c8d9e01",
                display: "atorvastatin",
            }),
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
    fn assertion_body_carries_the_coding_triple() {
        let v = medication_assertion_body(&full_assertion());
        assert_eq!(v["substance"]["term"], "Lipitor");
        assert_eq!(v["substance"]["coding"]["system"], "drugref-moiety");
        assert_eq!(
            v["substance"]["coding"]["code"],
            "0f8c4b1e-1b7a-5c2d-9a3e-2b6f7c8d9e01"
        );
        assert_eq!(v["substance"]["coding"]["display"], "atorvastatin");
    }

    #[test]
    fn assertion_body_omits_absent_coding_and_never_emits_the_retired_slot() {
        let mut a = full_assertion();
        a.coding = None;
        let v = medication_assertion_body(&a);
        let subst = v["substance"].as_object().unwrap();
        assert!(
            !subst.contains_key("coding"),
            "absent coding must be omitted, not null (principle 4: uncoded is first-class)"
        );
        assert!(
            !subst.contains_key("inn_code"),
            "the reserved inn_code slot is retired (ADR-0059 decision 2)"
        );
    }

    #[test]
    fn twin_appends_the_display_when_it_differs_from_the_term() {
        let s = render_medication_twin(&full_assertion());
        assert!(s.starts_with("Lipitor"));
        assert!(
            s.ends_with("[atorvastatin]"),
            "the captured display is the honest-degradation label a drugref-less \
             reader still sees, got: {s}"
        );
    }

    #[test]
    fn twin_does_not_repeat_a_display_equal_to_the_term() {
        // Case-folded compare: the clinician typed the generic name the coding resolves to.
        let mut a = full_assertion();
        a.term = "Atorvastatin";
        let s = render_medication_twin(&a);
        assert!(
            !s.contains('['),
            "a display equal to the term (case-insensitively) must add nothing, got: {s}"
        );
    }

    #[test]
    fn twin_of_an_uncoded_assertion_is_unchanged() {
        let mut a = full_assertion();
        a.coding = None;
        let s = render_medication_twin(&a);
        assert!(!s.contains('['), "no coding, no bracket: {s}");
        assert!(s.starts_with("Lipitor"));
    }
```

Also update the two existing tests that construct a `MedicationAssertion` literal
(`assertion_body_omits_absent_optionals_never_null`, `assertion_twin_nonempty_for_vague_term_only`):
replace their `inn_code: None,` line with `coding: None,`.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p cairn-event medication::assert`
Expected: FAIL to compile — `struct MedicationAssertion has no field named coding` / `cannot find struct SubstanceCoding`.

- [ ] **Step 3: Write the implementation**

In the same file, replace the `inn_code` field and its payload/twin handling:

```rust
/// A drug-identity coding claim, captured at coding time (ADR-0059).
///
/// The anchor is an *immortal identifier* — drugref's `moiety_uuid` — never a name:
/// keying on a label (even an INN) repeats the founding wound (principle 2). All three
/// fields travel together because `display` is the honest-degradation label: a node
/// without drugref still shows the preferred name, so it is never optional *within*
/// the object. The object as a whole stays optional — uncoded is first-class.
pub struct SubstanceCoding<'a> {
    /// The drugref composition-tree level. `drugref-moiety` today; the finer
    /// `drugref-clinical-drug` / `drugref-product` levels are reserved.
    pub system: &'a str,
    /// The immortal identifier itself (a `moiety_uuid`, UUIDv5 from the UNII).
    pub code: &'a str,
    /// The INN-preferred label as it read at coding time.
    pub display: &'a str,
}
```

In `MedicationAssertion`, replace the `inn_code` field with:

```rust
    /// Drug-identity coding, when someone has coded it; `None` = not-yet-coded, which
    /// is a permanently valid state (the "little white pill" floor, principle 4).
    pub coding: Option<SubstanceCoding<'a>>,
```

In `medication_assertion_body`, replace the `inn_code` insert with:

```rust
        if let Some(c) = &a.coding {
            s.insert(
                "coding".into(),
                json!({ "system": c.system, "code": c.code, "display": c.display }),
            );
        }
```

In `render_medication_twin`, append before the final `s` (after the `started` clause):

```rust
    // ADR-0059 / principle 11: the captured display is what a reader without drugref
    // still has. Repeat it only when it adds something — a clinician who typed the
    // generic name already wrote it (case-folded compare, so "Atorvastatin" counts).
    if let Some(c) = &a.coding {
        if !c.display.eq_ignore_ascii_case(a.term) {
            s.push_str(&format!(" [{}]", c.display));
        }
    }
```

In `crates/cairn-event/src/medication/mod.rs:20`, widen the re-export:

```rust
pub use assert::{
    medication_assertion_body, render_medication_twin, MedicationAssertion, SubstanceCoding,
};
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p cairn-event`
Expected: PASS (cairn-event has no DB gate, so the whole crate suite runs).

- [ ] **Step 5: Commit**

```bash
git add crates/cairn-event/src/medication/assert.rs crates/cairn-event/src/medication/mod.rs
git commit -m "feat(medication 6a): substance.coding replaces the reserved inn_code slot

The drug-identity anchor is drugref's immortal moiety_uuid carried as a
structured {system, code, display} object (ADR-0059 decisions 1-2). The
captured display renders into the legibility twin only when it differs
from the clinician's own term. Uncoded stays first-class: the key is
omitted, never null.

Refs ADR-0059"
```

---

### Task 2: The node surface — orchestrator input, pure flag parsing, CLI, call sites

**Files:**
- Modify: `crates/cairn-node/src/medication/assert.rs:14-24` (input struct), `:48-59` (builder call)
- Modify: `crates/cairn-node/src/main.rs:644-646` (clap arg), `:1759-1785` (dispatch)
- Modify (call sites, `inn_code: None` → `coding: None`): `crates/cairn-node/tests/{medication,medication_dose,medication_attestation,medication_reconciliation,medication_authorship,medication_patient_consistency,medication_remote_apply,shred_cli}.rs`, `crates/cairn-sync/tests/clinical_pull.rs`
- Modify: `crates/cairn-sync/src/main.rs:1269` (a doc/example payload string mentioning `inn_code`)
- Test: `crates/cairn-node/src/medication/assert.rs` (new in-file `mod tests`)

**Interfaces:**
- Consumes: `cairn_event::medication::SubstanceCoding` (Task 1).
- Produces: `AssertMedicationInput<'a>.coding: Option<SubstanceCoding<'a>>`;
  `pub fn coding_from_parts<'a>(system: Option<&'a str>, code: Option<&'a str>, display: Option<&'a str>) -> anyhow::Result<Option<SubstanceCoding<'a>>>`.

- [ ] **Step 1: Write the failing test**

Append to `crates/cairn-node/src/medication/assert.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coding_from_parts_accepts_all_three_or_none() {
        let none = coding_from_parts(None, None, None).expect("uncoded is valid");
        assert!(none.is_none(), "no flags at all means not-yet-coded");

        let some = coding_from_parts(
            Some("drugref-moiety"),
            Some("0f8c4b1e-1b7a-5c2d-9a3e-2b6f7c8d9e01"),
            Some("atorvastatin"),
        )
        .expect("a complete triple is valid")
        .expect("a complete triple yields a coding");
        assert_eq!(some.system, "drugref-moiety");
        assert_eq!(some.display, "atorvastatin");
    }

    #[test]
    fn coding_from_parts_refuses_a_partial_triple() {
        // A half-supplied coding must never reach the door: the DB floor would refuse
        // it anyway, but the caller deserves the error at the source, naming the gap.
        let e = coding_from_parts(Some("drugref-moiety"), Some("abc"), None)
            .expect_err("a partial coding must be refused");
        let msg = e.to_string();
        assert!(msg.contains("coding-display"), "the error names the missing flag: {msg}");
    }

    #[test]
    fn coding_from_parts_refuses_a_blank_field() {
        let e = coding_from_parts(Some("drugref-moiety"), Some("   "), Some("atorvastatin"))
            .expect_err("a blank field is not a value");
        assert!(e.to_string().contains("coding-code"), "{e}");
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p cairn-node --lib medication::assert`
Expected: FAIL to compile — `cannot find function coding_from_parts`.

- [ ] **Step 3: Write the implementation**

In `crates/cairn-node/src/medication/assert.rs`, change the import and the input struct field:

```rust
use cairn_event::medication::{
    medication_assertion_body, render_medication_twin, MedicationAssertion, SubstanceCoding,
};
```

```rust
    /// Drug-identity coding (ADR-0059) — `None` when nobody has coded it yet.
    pub coding: Option<SubstanceCoding<'a>>,
```

In `build_assert_body`, replace `inn_code: input.inn_code,` with:

```rust
        coding: input.coding.as_ref().map(|c| SubstanceCoding {
            system: c.system,
            code: c.code,
            display: c.display,
        }),
```

Add the pure parser beside `validate_term`:

```rust
/// Turn three independent CLI flags into an all-or-nothing coding claim.
///
/// Pure and total: the three flags are supplied together or not at all. A *partial*
/// triple is a caller mistake, and this is where it is caught — the DB floor would
/// refuse it too, but at the source the message can name the flag that is missing.
/// Blank is not a value: `--coding-code "  "` is a missing code, not an empty one.
pub fn coding_from_parts<'a>(
    system: Option<&'a str>,
    code: Option<&'a str>,
    display: Option<&'a str>,
) -> anyhow::Result<Option<SubstanceCoding<'a>>> {
    let parts = [
        ("coding-system", system),
        ("coding-code", code),
        ("coding-display", display),
    ];
    let missing: Vec<&str> = parts
        .iter()
        .filter(|(_, v)| v.is_none_or(|s| s.trim().is_empty()))
        .map(|(name, _)| *name)
        .collect();
    if missing.len() == parts.len() {
        return Ok(None); // not-yet-coded: a permanently valid state (principle 4)
    }
    if !missing.is_empty() {
        anyhow::bail!(
            "a drug coding needs all three parts; missing or blank: --{}. \
             Omit all three to record the medication uncoded (ADR-0059)",
            missing.join(", --")
        );
    }
    Ok(Some(SubstanceCoding {
        system: system.expect("checked non-empty above").trim(),
        code: code.expect("checked non-empty above").trim(),
        display: display.expect("checked non-empty above").trim(),
    }))
}
```

In `crates/cairn-node/src/main.rs`, replace the `inn_code` clap arg (line ~644) with:

```rust
        /// Drug-identity coding system — `drugref-moiety` today (ADR-0059).
        /// Supply all three --coding-* flags together, or none at all.
        #[arg(long)]
        coding_system: Option<String>,
        /// The immortal drug identifier (a drugref moiety_uuid).
        #[arg(long)]
        coding_code: Option<String>,
        /// The INN-preferred label as it reads at coding time.
        #[arg(long)]
        coding_display: Option<String>,
```

In the `Cmd::MedicationAssert` dispatch arm, replace `inn_code,` in the destructuring with
`coding_system,`, `coding_code,`, `coding_display,`, and replace `inn_code: inn_code.as_deref(),` in the
`AssertMedicationInput` literal with:

```rust
                coding: cairn_node::medication::coding_from_parts(
                    coding_system.as_deref(),
                    coding_code.as_deref(),
                    coding_display.as_deref(),
                )?,
```

Export it from the medication module (`crates/cairn-node/src/medication/mod.rs`): add
`coding_from_parts` to the existing `pub use assert::{…}` list.

- [ ] **Step 4: Fix every call site and run the whole workspace**

Replace `inn_code: None,` with `coding: None,` in all nine test files listed under **Files** (grep
`inn_code` to confirm none remain outside `db/` and the deprecated projection column). In
`crates/cairn-sync/src/main.rs:1269`, update the illustrative payload string
`"substance": {"term": "metformin", "inn_code": "6809"}` to
`"substance": {"term": "metformin"}` (the example predates the coding shape and must not model a retired
slot).

Run: `cargo test --workspace`
Expected: PASS. (DB-gated tests self-skip without `CAIRN_TEST_PG`; set it to run them.)

- [ ] **Step 5: Commit**

```bash
git add crates/cairn-node crates/cairn-sync
git commit -m "feat(medication 6a): --coding-system/-code/-display replace --inn-code

coding_from_parts is the all-or-nothing pure parser: three flags together
or none, blank is not a value, and a partial triple fails at the source
naming the missing flag rather than at the DB floor. Every call site
across cairn-node, cairn-sync and cairn-event moves to coding: None.

Refs ADR-0059"
```

---

### Task 3: The floor — `db/041_medication_coding.sql` and the db/031 call

**Files:**
- Create: `db/041_medication_coding.sql`
- Modify: `crates/cairn-event/src/schema_generation.rs` (40 → 41)
- Modify: `crates/cairn-node/src/db.rs:220` (append the `041` entry after `040`)
- Modify: `db/031_medication.sql:46-56` (the assert branch of `cairn_check_medication_assertion`)
- Create: `crates/cairn-node/tests/medication_coding.rs`

**Interfaces:**
- Consumes: the payload shape from Task 1 (`substance.coding`), the node surface from Task 2.
- Produces: table `medication_coding_system(system text PK, code_format text, note text)`;
  `cairn_check_medication_coding(p jsonb) RETURNS void`. Task 4 relies on neither directly, but its
  tests submit codings that must pass this floor.

- [ ] **Step 1: Write the failing tests**

Create `crates/cairn-node/tests/medication_coding.rs`. Copy the harness preamble (`cs`, `db_msg`,
`seal_and_submit`, `setup_node`) verbatim from `crates/cairn-node/tests/medication.rs:14-88`, adding
`medication_coding` to `setup_node`'s conditional TRUNCATE block:

```rust
           IF to_regclass('public.medication_coding') IS NOT NULL THEN TRUNCATE medication_coding; END IF; \
```

Then add the floor tests:

```rust
use cairn_event::medication::SubstanceCoding;
use cairn_node::medication::{assert_medication, build_assert_body, AssertMedicationInput};

/// A moiety anchor shaped like drugref's: a UUIDv5. Fixed, not random — the tests
/// assert on it. Not cryptographic material, so house rule 6 does not apply.
const MOIETY_ATORVASTATIN: &str = "0f8c4b1e-1b7a-5c2d-9a3e-2b6f7c8d9e01";

fn coded_input(term: &'static str, code: &'static str) -> AssertMedicationInput<'static> {
    AssertMedicationInput {
        term,
        coding: Some(SubstanceCoding {
            system: "drugref-moiety",
            code,
            display: "atorvastatin",
        }),
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
async fn registry_and_check_fn_load() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    // db/031's floor calls this fn by name and plpgsql resolves it only at execution,
    // so a missing db/041 would otherwise surface as a first-write surprise.
    let n: i64 = c
        .query_one(
            "SELECT count(*) FROM medication_coding_system WHERE system = 'drugref-moiety'",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(n, 1, "the drugref-moiety row must be seeded");
    let exists: Option<String> = c
        .query_one(
            "SELECT to_regprocedure('cairn_check_medication_coding(jsonb)')::text",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert!(exists.is_some(), "db/041 must declare the coding check fn");
}

#[tokio::test]
async fn a_complete_coding_is_accepted() {
    let Some(base) = cs() else { return };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient = Uuid::now_v7();
    assert_medication(
        &mut c,
        &sk,
        &kid,
        "test-node",
        patient,
        &coded_input("Lipitor", MOIETY_ATORVASTATIN),
        None,
        None,
    )
    .await
    .expect("a well-formed drugref-moiety coding must pass the floor");
}

#[tokio::test]
async fn an_uncoded_assertion_still_passes() {
    let Some(base) = cs() else { return };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let mut input = coded_input("little white pill", MOIETY_ATORVASTATIN);
    input.coding = None;
    // The principle-4 floor and the §1.2 M = N = 1 pin: coding is never required.
    assert_medication(
        &mut c,
        &sk,
        &kid,
        "test-node",
        Uuid::now_v7(),
        &input,
        None,
        None,
    )
    .await
    .expect("uncoded must stay a first-class recordable value");
}

/// Submit a raw payload at a chosen door, bypassing the Rust builder — the only way to
/// present the malformed shapes a hostile or buggy peer could send.
async fn submit_raw_substance(
    c: &Client,
    sk: &SigningKey,
    kid: &str,
    door: &str,
    substance: serde_json::Value,
) -> Result<u64, tokio_postgres::Error> {
    let hlc = db::next_hlc(c, "test-node").await.unwrap();
    let mut body = build_assert_body(
        Uuid::now_v7(),
        Uuid::now_v7(),
        Uuid::now_v7(),
        &coded_input("Lipitor", MOIETY_ATORVASTATIN),
        kid,
        hlc,
    );
    body.payload["substance"] = substance;
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
async fn structural_gaps_are_refused_at_both_doors() {
    let Some(base) = cs() else { return };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    // display is structural, not optional: it is the honest-degradation label a
    // drugref-less reader depends on (ADR-0059 decision 4).
    for door in ["submit_event", "apply_remote_event"] {
        for bad in [
            serde_json::json!({"term": "Lipitor", "coding": {"system": "drugref-moiety", "code": MOIETY_ATORVASTATIN}}),
            serde_json::json!({"term": "Lipitor", "coding": {"system": "drugref-moiety", "code": MOIETY_ATORVASTATIN, "display": "  "}}),
            serde_json::json!({"term": "Lipitor", "coding": {"code": MOIETY_ATORVASTATIN, "display": "atorvastatin"}}),
            serde_json::json!({"term": "Lipitor", "coding": "drugref-moiety"}),
        ] {
            let e = submit_raw_substance(&c, &sk, kid.as_str(), door, bad.clone())
                .await
                .expect_err(&format!("{door} must refuse {bad}"));
            assert!(
                db_msg(&e).contains("substance.coding"),
                "the refusal must name the field: {}",
                db_msg(&e)
            );
        }
    }
}

#[tokio::test]
async fn an_unregistered_system_is_refused_locally_and_admitted_remotely() {
    let Some(base) = cs() else { return };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let unknown = serde_json::json!({
        "term": "Lipitor",
        "coding": {"system": "national-formulary-xyz", "code": "A10BA02", "display": "metformin"}
    });
    // Strict submit: this door only authors codings it can vouch for (ADR-0051).
    let e = submit_raw_substance(&c, &sk, kid.as_str(), "submit_event", unknown.clone())
        .await
        .expect_err("an unregistered system must be refused at the local door");
    assert!(db_msg(&e).contains("national-formulary-xyz"), "{}", db_msg(&e));
    // Lenient apply: a peer may run a newer or locally-extended registry. Admit it.
    submit_raw_substance(&c, &sk, kid.as_str(), "apply_remote_event", unknown)
        .await
        .expect("a peer's unregistered system must be admitted, never refused");
}

#[tokio::test]
async fn a_non_uuid_code_is_refused_locally_and_admitted_remotely() {
    let Some(base) = cs() else { return };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let bad = serde_json::json!({
        "term": "Lipitor",
        "coding": {"system": "drugref-moiety", "code": "atorvastatin", "display": "atorvastatin"}
    });
    let e = submit_raw_substance(&c, &sk, kid.as_str(), "submit_event", bad.clone())
        .await
        .expect_err("a drugref-moiety code must be a uuid");
    assert!(db_msg(&e).contains("uuid"), "{}", db_msg(&e));
    submit_raw_substance(&c, &sk, kid.as_str(), "apply_remote_event", bad)
        .await
        .expect("the registry-derived tier is lenient at the apply door");
}

#[tokio::test]
async fn the_retired_inn_code_slot_is_refused_locally_and_ignored_remotely() {
    let Some(base) = cs() else { return };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let retired = serde_json::json!({"term": "Lipitor", "inn_code": "INN:atorvastatin"});
    let e = submit_raw_substance(&c, &sk, kid.as_str(), "submit_event", retired.clone())
        .await
        .expect_err("the retired slot must fail loud at the source");
    assert!(db_msg(&e).contains("substance.coding"), "{}", db_msg(&e));
    submit_raw_substance(&c, &sk, kid.as_str(), "apply_remote_event", retired)
        .await
        .expect("a verifiable peer event is never refused over a retired slot");
}
```

Add the imports the harness needs at the top of the file:
`use cairn_event::{generate_key, sign, EventBody, SigningKey}; use cairn_node::db; use tokio_postgres::Client; use uuid::Uuid;`.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test -p cairn-node --test medication_coding`
Expected: FAIL — `relation "medication_coding_system" does not exist` for the registry test, and the
refusal tests fail because nothing yet rejects the malformed codings.

- [ ] **Step 3: Write `db/041_medication_coding.sql`**

```sql
-- 041_medication_coding.sql — the drug-identity coding floor (ADR-0059, data-model §3.16).
--
-- ADR-0059 anchors a medication's drug identity on drugref's immortal `moiety_uuid`
-- carried as substance.coding {system, code, display}. This file holds the two pieces
-- that govern it: the vocabulary registry of admitted coding systems, and the floor
-- check db/031's per-type check calls.
--
-- WHY A SEPARATE FILE: SCHEMA_GENERATION is derived from the newest db/ prefix, and this
-- is a FLOOR change. Issue #188 exists so an older binary cannot CREATE OR REPLACE a
-- newer safety check back down; an in-place edit of db/031 alone could not bump the
-- generation and would leave that downgrade silent.
--
-- TWO TIERS, because the per-type floor runs at BOTH doors (db/020 §8 calls the same
-- cairn_event_twin hook as submit_event, deliberately — the M8 asymmetry fix):
--   structural       (three non-empty strings)  -> refuse at BOTH doors, like substance.term
--   registry-derived (known system, code shape) -> refuse locally, ADMIT remotely (ADR-0051)
-- A peer may legitimately run a newer or locally-extended registry, and a refusal on a
-- verifiable event is the sync-wedge ADR-0056 forbids.
BEGIN;

-- 1. The admitted coding systems. Register-by-row, like event_type_class /
--    cairn_event_twin_check / cairn_projection_apply. ADR-0059 decision 7 is explicit
--    that a deployment may plug a DIFFERENT drug-identity authority: that is a row here,
--    not a patch to this file (principle 9 — mechanism, never policy).
CREATE TABLE IF NOT EXISTS medication_coding_system (
    system      TEXT PRIMARY KEY,
    -- 'uuid'   : the code must parse as a uuid (drugref moiety ids are UUIDv5)
    -- 'opaque' : any non-empty string
    code_format TEXT NOT NULL CHECK (code_format IN ('uuid', 'opaque')),
    note        TEXT NOT NULL
);
GRANT SELECT ON medication_coding_system TO cairn_agent;
REVOKE INSERT, UPDATE, DELETE ON medication_coding_system FROM PUBLIC;

-- Seed the drugref composition-tree levels. Only `drugref-moiety` exists today; the two
-- finer levels are RESERVED by ADR-0059 decision 2 so strength/form-level coding lands
-- additively later without reshaping the slot. #214 convergence: DO UPDATE (never DO
-- NOTHING) so an edited seed heals on the next connect, with the IS DISTINCT FROM guard
-- keeping the steady-state replay write-free.
INSERT INTO medication_coding_system AS r (system, code_format, note) VALUES
    ('drugref-moiety',        'uuid', 'drugref immortal moiety_uuid (UUIDv5 from UNII) — the only level built today'),
    ('drugref-clinical-drug', 'uuid', 'RESERVED for a later drugref slice (substance + strength + form)'),
    ('drugref-product',       'uuid', 'RESERVED for a later drugref slice (a marketed product)')
ON CONFLICT (system) DO UPDATE SET
    code_format = EXCLUDED.code_format,
    note        = EXCLUDED.note
WHERE (r.code_format, r.note) IS DISTINCT FROM (EXCLUDED.code_format, EXCLUDED.note);

-- 2. The floor check. Called from cairn_check_medication_assertion (db/031); plpgsql
--    resolves the call at EXECUTION, so living in a later file is fine.
CREATE OR REPLACE FUNCTION cairn_check_medication_coding(p jsonb)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    c        jsonb   := p -> 'substance' -> 'coding';
    -- db/020 sets this transaction-local marker on the sync-apply path; the same idiom
    -- cairn_guard_medication_patient uses to tell the doors apart (db/031 part 3b).
    v_remote boolean := current_setting('cairn.remote_apply', true) = 'on';
    v_key    text;
    v_format text;
BEGIN
    -- Uncoded is a permanently valid state (principle 4, the "little white pill" floor).
    IF c IS NULL THEN
        RETURN;
    END IF;
    IF jsonb_typeof(c) IS DISTINCT FROM 'object' THEN
        RAISE EXCEPTION 'medication assertion: substance.coding must be an object {system, code, display} (ADR-0059)';
    END IF;

    -- Structural tier — both doors. display is NOT optional: it is the honest-degradation
    -- label, the whole reason a drugref-less node can still read a coded medication.
    FOREACH v_key IN ARRAY ARRAY['system', 'code', 'display'] LOOP
        IF jsonb_typeof(c -> v_key) IS DISTINCT FROM 'string'
           OR length(btrim(c ->> v_key)) = 0 THEN
            RAISE EXCEPTION
                'medication assertion: substance.coding.% must be a non-empty string (ADR-0059 decision 2 — display is the honest-degradation label)',
                v_key;
        END IF;
    END LOOP;

    -- Registry-derived tier — local door only (strict-submit / lenient-apply, ADR-0051).
    IF v_remote THEN
        RETURN;
    END IF;
    SELECT s.code_format INTO v_format
        FROM medication_coding_system s WHERE s.system = c ->> 'system';
    IF v_format IS NULL THEN
        RAISE EXCEPTION
            'medication assertion: unknown coding system "%" — this door only authors codings it can vouch for; register it in medication_coding_system (ADR-0059 decision 7)',
            c ->> 'system';
    END IF;
    IF v_format = 'uuid' THEN
        BEGIN
            PERFORM (c ->> 'code')::uuid;
        EXCEPTION WHEN others THEN
            RAISE EXCEPTION
                'medication assertion: coding system "%" requires a uuid code, got "%" (a drugref moiety id is a UUIDv5)',
                c ->> 'system', c ->> 'code';
        END;
    END IF;
END;
$$;
REVOKE EXECUTE ON FUNCTION cairn_check_medication_coding(jsonb) FROM PUBLIC;

COMMIT;
```

- [ ] **Step 4: Wire it into db/031 and bump the generation**

In `db/031_medication.sql`, inside the `IF p_type = 'clinical.medication.asserted' THEN` branch, after the
`info_source` check, add:

```sql
        -- ADR-0059 decision 2: the reserved inn_code slot is RETIRED. Fail loud at the
        -- authoring door (a caller still emitting it is a bug at source); ignore it on
        -- the apply path — a refusal on a verifiable peer event is the sync-wedge
        -- ADR-0056 forbids, and the slot is simply never read again.
        IF (p -> 'substance') ? 'inn_code'
           AND current_setting('cairn.remote_apply', true) IS DISTINCT FROM 'on' THEN
            RAISE EXCEPTION 'medication assertion: substance.inn_code is retired — carry substance.coding {system, code, display} instead (ADR-0059 decision 2)';
        END IF;
        -- The coding floor lives in db/041 (a floor change needs its own generation
        -- bump, #188); plpgsql resolves the call at execution, so the later file is fine.
        PERFORM cairn_check_medication_coding(p);
```

In `crates/cairn-event/src/schema_generation.rs`, change `SCHEMA_GENERATION` to `41` and update the doc
comment's example filename to `db/041_medication_coding.sql`.

In `crates/cairn-node/src/db.rs`, append after the `040` entry:

```rust
    // db/041 (ADR-0059): the medication coding-system vocabulary registry + the
    // substance.coding floor check db/031's per-type check calls. cairn-sync's list
    // carries no medication files at all and legitimately lags (#284).
    (
        "041_medication_coding",
        include_str!("../../../db/041_medication_coding.sql"),
    ),
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test --workspace`
Expected: PASS, including `cairn-event`'s `schema_generation` guard (the constant now matches the newest
file on disk) and cairn-node's list-embeds-the-newest-file unit test.

- [ ] **Step 6: Commit**

```bash
git add db/041_medication_coding.sql db/031_medication.sql crates/cairn-event/src/schema_generation.rs crates/cairn-node/src/db.rs crates/cairn-node/tests/medication_coding.rs
git commit -m "feat(medication 6a): the substance.coding floor + coding-system registry

db/041 adds medication_coding_system (register-by-row, so substituting a
drug-identity authority is a row and not a patch) and the two-tier check:
structural gaps refuse at BOTH doors like substance.term, while
registry-derived checks are strict-submit / lenient-apply — a peer may run
a newer registry, and refusing a verifiable event is the ADR-0056 wedge.
db/031 now refuses the retired inn_code slot locally and ignores it on
apply. A floor change gets its own generation bump (#188): 40 -> 41.

Refs ADR-0059, #188"
```

---

### Task 4: The projection — `medication_coding` table, apply write, widened views

**Files:**
- Modify: `db/031_medication.sql` (new table + ALTER; the apply-fn write; the registry inventory; three widened views)
- Modify: `db/032_medication_dose.sql:303-322` (widened current/past)
- Modify: `db/033_medication_reconciliation.sql:444-466` (widened current/past)
- Modify: `crates/cairn-node/tests/medication_coding.rs` (projection tests)
- Modify: the nine `setup_node` helpers listed in Task 2's Files (add the `medication_coding` TRUNCATE line)

**Interfaces:**
- Consumes: the floor from Task 3.
- Produces: table `medication_coding(medication_id uuid PK, patient_id uuid, coding_system text, coding_code text, coding_display text, hlc_wall bigint, hlc_counter int, origin text, content_address bytea, updated_at timestamptz)`;
  the three trailing view columns `coding_system, coding_code, coding_display` on `patient_medication`,
  `patient_medication_current`, `patient_medication_past`. Task 5 consumes both.

- [ ] **Step 1: Write the failing tests**

Append to `crates/cairn-node/tests/medication_coding.rs`:

```rust
async fn coding_row(c: &Client, med: Uuid) -> Option<(String, String, String)> {
    c.query_opt(
        "SELECT coding_system, coding_code, coding_display \
           FROM medication_coding WHERE medication_id = $1::text::uuid",
        &[&med.to_string()],
    )
    .await
    .unwrap()
    .map(|r| (r.get(0), r.get(1), r.get(2)))
}

#[tokio::test]
async fn a_coded_assertion_projects_its_coding() {
    let Some(base) = cs() else { return };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient = Uuid::now_v7();
    let med = assert_medication(
        &mut c,
        &sk,
        &kid,
        "test-node",
        patient,
        &coded_input("Lipitor", MOIETY_ATORVASTATIN),
        None,
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        coding_row(&c, med).await,
        Some((
            "drugref-moiety".to_string(),
            MOIETY_ATORVASTATIN.to_string(),
            "atorvastatin".to_string()
        ))
    );
    // The honest-degradation read: the med list itself carries the label, so a node
    // with no drug database still shows the preferred name (ADR-0059 decision 4).
    let row = c
        .query_one(
            "SELECT term, coding_display FROM patient_medication_current \
               WHERE medication_id = $1::text::uuid",
            &[&med.to_string()],
        )
        .await
        .unwrap();
    assert_eq!(row.get::<_, String>(0), "Lipitor");
    assert_eq!(row.get::<_, String>(1), "atorvastatin");
}

#[tokio::test]
async fn an_uncoded_assertion_projects_no_coding_row() {
    let Some(base) = cs() else { return };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let mut input = coded_input("little white pill", MOIETY_ATORVASTATIN);
    input.coding = None;
    let med = assert_medication(
        &mut c,
        &sk,
        &kid,
        "test-node",
        Uuid::now_v7(),
        &input,
        None,
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        coding_row(&c, med).await,
        None,
        "no coding claimed, no coding row — and nothing to clear later"
    );
}

#[tokio::test]
async fn the_deprecated_inn_code_column_survives_unread() {
    let Some(base) = cs() else { return };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    // ADR-0059 decision 2: deprecated IN PLACE, never dropped — a DROP is the
    // non-additive move principle 11 forbids (the db/036 sync_state.hlc_wall treatment).
    let n: i64 = c
        .query_one(
            "SELECT count(*) FROM information_schema.columns \
              WHERE table_name = 'medication_statement' AND column_name = 'inn_code'",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(n, 1, "the deprecated column stays");
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `CAIRN_TEST_PG=… cargo test -p cairn-node --test medication_coding`
Expected: FAIL — `relation "medication_coding" does not exist`, and
`column "coding_display" does not exist` on `patient_medication_current`.

- [ ] **Step 3: Add the projection table and the apply write (db/031)**

After the `medication_statement` table block in `db/031_medication.sql`, add:

```sql
-- 4b. The drug-identity coding projection (ADR-0059). A SEPARATE table, not columns on
--     medication_statement, for two reasons: one fact gets one home (slice 6b's coding
--     OVERLAY events write this same table under the same winner rule, so no reader ever
--     needs a precedence rule between two homes), and it keeps 6b purely additive — rows,
--     not rewritten view bodies. No FK to medication_statement: a coding may legitimately
--     arrive before the assert it codes (arrival-order independence, the same reason
--     medication_cessation is its own table).
CREATE TABLE IF NOT EXISTS medication_coding (
    medication_id   UUID PRIMARY KEY,
    patient_id      UUID NOT NULL,
    coding_system   TEXT NOT NULL,
    coding_code     TEXT NOT NULL,
    coding_display  TEXT NOT NULL,
    hlc_wall        BIGINT  NOT NULL,
    hlc_counter     INTEGER NOT NULL,
    origin          TEXT    NOT NULL,
    content_address BYTEA   NOT NULL,
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
GRANT SELECT ON medication_coding TO cairn_agent;
CREATE INDEX IF NOT EXISTS medication_coding_anchor_idx
    ON medication_coding (coding_system, coding_code);
```

Inside `medication_statement_apply`, after the `medication_statement` INSERT … ON CONFLICT block and
before `RETURN`, add:

```sql
    -- ADR-0059: the INLINE coding claim, when the author made one. Written only when
    -- present, so a later uncoded re-assertion can never silently clear a coding —
    -- retracting a coding is slice 6b's correction event, an authored act.
    IF p -> 'substance' -> 'coding' IS NOT NULL THEN
        INSERT INTO medication_coding
            (medication_id, patient_id, coding_system, coding_code, coding_display,
             hlc_wall, hlc_counter, origin, content_address)
        VALUES (
            (p ->> 'medication_id')::uuid, e.patient_id,
            p -> 'substance' -> 'coding' ->> 'system',
            p -> 'substance' -> 'coding' ->> 'code',
            p -> 'substance' -> 'coding' ->> 'display',
            e.hlc_wall, e.hlc_counter, e.node_origin, e.content_address)
        ON CONFLICT (medication_id) DO UPDATE SET
            patient_id      = EXCLUDED.patient_id,
            coding_system   = EXCLUDED.coding_system,
            coding_code     = EXCLUDED.coding_code,
            coding_display  = EXCLUDED.coding_display,
            hlc_wall        = EXCLUDED.hlc_wall,
            hlc_counter     = EXCLUDED.hlc_counter,
            origin          = EXCLUDED.origin,
            content_address = EXCLUDED.content_address,
            updated_at      = clock_timestamp()
        WHERE cairn_hlc_overlay_wins(
            EXCLUDED.hlc_wall, EXCLUDED.hlc_counter, EXCLUDED.origin, EXCLUDED.content_address,
            medication_coding.hlc_wall, medication_coding.hlc_counter,
            medication_coding.origin, medication_coding.content_address);
    END IF;
```

In the `cairn_projection_apply` registration at the foot of db/031, add `medication_coding` to the
assert row's inventory (rebuild-scope metadata must be exhaustive):

```sql
    ('clinical.medication.asserted',           'medication_statement_apply',
     ARRAY['medication_statement', 'medication_coding', 'medication_patient_conflict_flag'], 20, TRUE),
```

`heal_safe` stays `TRUE`: this is a DO-UPDATE overlay, not the `ON CONFLICT DO NOTHING` shape
[#277](https://github.com/cairn-ehr/cairn-ehr/issues/277) warns cannot be re-derived by a heal.

- [ ] **Step 4: Widen the three views, in all three files**

The trailing column list must be **identical** in every file that creates each view — db/031 replays
first on every connect, so a narrower definition anywhere fails the next connect with *"cannot drop
columns from view"* (#207).

In `db/031_medication.sql`, `patient_medication`:

```sql
CREATE OR REPLACE VIEW patient_medication AS
SELECT s.medication_id, s.patient_id, s.term, s.inn_code, s.formulation,
       s.dose_amount, s.dose_unit, s.sig, s.info_source,
       s.started_value, s.started_precision,
       to_timestamp(s.hlc_wall / 1000.0) AS asserted_at,
       (c.medication_id IS NOT NULL) AS ceased,
       c.stopped_value, c.stopped_precision, c.reason,
       -- ADR-0059: appended at the END. inn_code stays, deprecated in place and read by
       -- nothing — dropping a view column would need a DROP VIEW, and a DROP is the
       -- non-additive move principle 11 forbids.
       mc.coding_system, mc.coding_code, mc.coding_display
FROM medication_statement s
LEFT JOIN medication_cessation c USING (medication_id)
LEFT JOIN medication_coding mc USING (medication_id);
```

`patient_medication_current` and `patient_medication_past` in db/031 append
`, coding_system, coding_code, coding_display` to their select lists (they select from
`patient_medication`, so the columns pass straight through).

In `db/032_medication_dose.sql`, append `, pm.coding_system, pm.coding_code, pm.coding_display` to both
view select lists, and extend the CRITICAL comment above them:

```sql
--     The coding triple (ADR-0059) is part of that same-column-set contract now: it is
--     appended at the END in db/031, db/032 and db/033 alike.
```

In `db/033_medication_reconciliation.sql`, append `, d.coding_system, d.coding_code, d.coding_display`
to both view select lists (they select from `medication_group_display d`, which Task 5 widens — write
this step and Task 5's `medication_group_display` change together if the loader complains).

- [ ] **Step 5: Add the TRUNCATE line to the medication test helpers**

In each of `crates/cairn-node/tests/{medication,medication_coding,medication_dose,medication_attestation,medication_reconciliation,medication_authorship,medication_patient_consistency,seal_submit,seal_apply,shred_cli}.rs`,
add to the conditional TRUNCATE `DO $$` block:

```
           IF to_regclass('public.medication_coding') IS NOT NULL THEN TRUNCATE medication_coding; END IF; \
```

- [ ] **Step 6: Run the tests to verify they pass**

Run: `CAIRN_TEST_PG=… cargo test --workspace`
Expected: PASS. Then **reconnect twice** to prove replay safety (the #207 trap surfaces only on the
second connect):

Run: `CAIRN_TEST_PG=… cargo test -p cairn-node --test medication_coding -- --test-threads=1`
Expected: PASS again with no *"cannot drop columns from view"*.

- [ ] **Step 7: Commit**

```bash
git add db/031_medication.sql db/032_medication_dose.sql db/033_medication_reconciliation.sql crates/cairn-node/tests
git commit -m "feat(medication 6a): project substance.coding into medication_coding

A separate projection table rather than columns on medication_statement:
one fact, one home, and slice 6b's coding overlays then add rows instead
of rewriting view bodies. Written only when a coding is present, so an
uncoded re-assert can never silently clear one. The read views gain the
coding triple appended at the end, with the identical trailing column
list in db/031, db/032 and db/033 (#207 — db/031 replays first).

Refs ADR-0059, #207"
```

---

### Task 5: Reconciliation — the `(system, code)` dup-key, prefer-coded display, anchor conflicts

**Files:**
- Modify: `db/031_medication.sql` (the `patient_medication_reconciliation_flag` dup-key)
- Modify: `db/033_medication_reconciliation.sql` (`medication_group_display` + the group-level dup-key + a new conflict view)
- Modify: `crates/cairn-node/tests/medication_coding.rs`

**Interfaces:**
- Consumes: `medication_coding` and the widened views (Task 4).
- Produces: view `medication_group_coding_conflict(group_id uuid, anchor_count bigint, anchors text[])`;
  `dup_key` values now prefixed `code:<system>|<code>` or `term:<normalized term>`.

- [ ] **Step 1: Write the failing tests**

Append to `crates/cairn-node/tests/medication_coding.rs`:

```rust
async fn dup_keys(c: &Client, patient: Uuid) -> Vec<(String, i64)> {
    c.query(
        "SELECT dup_key, thread_count FROM patient_medication_reconciliation_flag \
           WHERE patient_id = $1::text::uuid ORDER BY dup_key",
        &[&patient.to_string()],
    )
    .await
    .unwrap()
    .iter()
    .map(|r| (r.get(0), r.get(1)))
    .collect()
}

#[tokio::test]
async fn two_coded_threads_sharing_an_anchor_raise_one_flag() {
    let Some(base) = cs() else { return };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient = Uuid::now_v7();
    // The case ADR-0059 exists for: brand and generic, different words, same substance.
    for term in ["Lipitor", "atorvastatin"] {
        assert_medication(
            &mut c,
            &sk,
            &kid,
            "test-node",
            patient,
            &coded_input(term, MOIETY_ATORVASTATIN),
            None,
            None,
        )
        .await
        .unwrap();
    }
    let flags = dup_keys(&c, patient).await;
    assert_eq!(flags.len(), 1, "one duplicate group, got {flags:?}");
    assert_eq!(flags[0].1, 2);
    assert!(
        flags[0].0.starts_with("code:drugref-moiety|"),
        "the key is the (system, code) PAIR, never a bare code: {:?}",
        flags[0].0
    );
}

#[tokio::test]
async fn a_coded_and_an_uncoded_thread_still_key_apart() {
    let Some(base) = cs() else { return };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient = Uuid::now_v7();
    assert_medication(
        &mut c,
        &sk,
        &kid,
        "test-node",
        patient,
        &coded_input("atorvastatin", MOIETY_ATORVASTATIN),
        None,
        None,
    )
    .await
    .unwrap();
    let mut uncoded = coded_input("atorvastatin", MOIETY_ATORVASTATIN);
    uncoded.coding = None;
    assert_medication(&mut c, &sk, &kid, "test-node", patient, &uncoded, None, None)
        .await
        .unwrap();
    // ADR-0059 decision 5 is explicit: a coalesce picks PER ROW, so this case is NOT
    // closed by the key. It closes when the uncoded member gets coded, or later by the
    // drug-matcher. Claiming otherwise is the overstatement the ADR review caught.
    assert!(
        dup_keys(&c, patient).await.is_empty(),
        "coded and uncoded key apart — the honest, documented blind spot"
    );
}

#[tokio::test]
async fn a_reconciled_group_displays_its_coded_member() {
    let Some(base) = cs() else { return };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient = Uuid::now_v7();
    let mut vague = coded_input("little white pill", MOIETY_ATORVASTATIN);
    vague.coding = None;
    let m_vague = assert_medication(&mut c, &sk, &kid, "test-node", patient, &vague, None, None)
        .await
        .unwrap();
    let m_coded = assert_medication(
        &mut c,
        &sk,
        &kid,
        "test-node",
        patient,
        &coded_input("Lipitor", MOIETY_ATORVASTATIN),
        None,
        None,
    )
    .await
    .unwrap();
    cairn_node::medication::reconcile_medications(
        &mut c,
        &sk,
        &kid,
        "test-node",
        patient,
        m_vague,
        m_coded,
        &cairn_node::medication::ReconcileInput {
            provenance: "clinician-judgment",
            reason: None,
        },
        None,
        None,
    )
    .await
    .unwrap();
    let display: String = c
        .query_one(
            "SELECT coding_display FROM medication_group_display \
               WHERE patient_id = $1::text::uuid",
            &[&patient.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        display, "atorvastatin",
        "the group takes its identity from the coded member, not from \"little white pill\""
    );
}

#[tokio::test]
async fn two_anchors_in_one_group_raise_a_conflict() {
    let Some(base) = cs() else { return };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient = Uuid::now_v7();
    const MOIETY_METFORMIN: &str = "3c7d9a52-4e18-5f60-8b21-6d4a0e9c7f33";
    let m1 = assert_medication(
        &mut c,
        &sk,
        &kid,
        "test-node",
        patient,
        &coded_input("Lipitor", MOIETY_ATORVASTATIN),
        None,
        None,
    )
    .await
    .unwrap();
    let m2 = assert_medication(
        &mut c,
        &sk,
        &kid,
        "test-node",
        patient,
        &coded_input("Diabex", MOIETY_METFORMIN),
        None,
        None,
    )
    .await
    .unwrap();
    cairn_node::medication::reconcile_medications(
        &mut c,
        &sk,
        &kid,
        "test-node",
        patient,
        m1,
        m2,
        &cairn_node::medication::ReconcileInput {
            provenance: "clinician-judgment",
            reason: None,
        },
        None,
        None,
    )
    .await
    .expect("reconciliation is a human judgment — never auto-refused over a coding");
    let n: i64 = c
        .query_one("SELECT count(*) FROM medication_group_coding_conflict", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        n, 1,
        "two different anchors in one group is a possible-mis-reconciliation signal — \
         surfaced, never silently resolved"
    );
}
```

Check `reconcile_medications`' real signature in `crates/cairn-node/src/medication/reconciliation.rs`
before running; match it exactly (the existing `crates/cairn-node/tests/medication_reconciliation.rs`
shows a live call).

- [ ] **Step 2: Run the tests to verify they fail**

Run: `CAIRN_TEST_PG=… cargo test -p cairn-node --test medication_coding`
Expected: FAIL — dup keys come back `term:`-shaped or empty, `medication_group_display` has no
`coding_display`, and `medication_group_coding_conflict` does not exist.

- [ ] **Step 3: Rework the dup-key in db/031**

```sql
-- 9. E1 reconciliation flag (advisory, never auto-merges). >=2 ACTIVE threads for one
--    patient sharing the dup-key. ADR-0059: the key is the coding PAIR when coded, else
--    the normalized term. The PAIR, never a bare code — once the reserved finer drugref
--    levels exist, the same substance coded at moiety level on one node and clinical-drug
--    level on another would split under a bare-code key (the same blind spot one level up,
--    and a CROSS-NODE one). Each branch is prefixed so a free-text term can never collide
--    with a code key. COLLATE "C" on both branches pins cross-node determinism (ADR-0045).
--    WHAT THIS CLOSES: coded<->coded, including Lipitor<->atorvastatin once BOTH are coded.
--    WHAT IT DOES NOT: coalesce picks per ROW, so a coded and an uncoded row still key
--    apart. That case closes when the uncoded member gets CODED (offered, never forced),
--    or later by term->anchor resolution in the drug-matcher slice.
CREATE OR REPLACE VIEW patient_medication_reconciliation_flag AS
SELECT patient_id,
       coalesce('code:' || (coding_system COLLATE "C") || '|' || (coding_code COLLATE "C"),
                'term:' || lower(btrim(term) COLLATE "C")) AS dup_key,
       count(*)                                            AS thread_count,
       array_agg(medication_id ORDER BY medication_id)     AS medication_ids
FROM patient_medication_current
GROUP BY patient_id,
         coalesce('code:' || (coding_system COLLATE "C") || '|' || (coding_code COLLATE "C"),
                  'term:' || lower(btrim(term) COLLATE "C"))
HAVING count(*) > 1;
GRANT SELECT ON patient_medication_reconciliation_flag TO cairn_agent;
```

- [ ] **Step 4: Rework `medication_group_display` and the group-level flag, and add the conflict view (db/033)**

```sql
CREATE OR REPLACE VIEW medication_group_display AS
SELECT DISTINCT ON (g.group_id)
    g.group_id, g.patient_id, s.term, s.inn_code, s.formulation, s.sig, s.info_source,
    s.started_value, s.started_precision,
    to_timestamp(s.hlc_wall / 1000.0) AS asserted_at,
    s.dose_amount, s.dose_unit,
    mc.coding_system, mc.coding_code, mc.coding_display
FROM medication_statement s
JOIN medication_thread_group g ON g.medication_id = s.medication_id
LEFT JOIN medication_coding mc ON mc.medication_id = s.medication_id
-- ADR-0059 decision 5: PREFER A CODED MEMBER, so a reconciled group takes its identity
-- from the member somebody actually identified rather than from "little white pill".
-- Then the ADR's tiebreak (system, code), then the pre-0059 keys unchanged — for a group
-- with no coded member at all the ordering degenerates to exactly what it was.
-- COLLATE "C" keeps the tiebreak collation-independent (ADR-0045).
ORDER BY g.group_id,
         (mc.medication_id IS NOT NULL) DESC,
         mc.coding_system COLLATE "C", mc.coding_code COLLATE "C",
         (s.medication_id = g.group_id) DESC,
         s.medication_id;
GRANT SELECT ON medication_group_display TO cairn_agent;
```

The group-level `patient_medication_reconciliation_flag` in db/033 takes the same key expression as
db/031's (substitute it into both the SELECT and the GROUP BY, leaving the
`count(DISTINCT group_id)` semantics and the column names untouched), and its inner subquery gains the
coding columns:

```sql
    SELECT s.patient_id, s.medication_id, mc.coding_system, mc.coding_code, s.term,
           COALESCE(gm.group_id, s.medication_id) AS group_id
    FROM medication_statement s
    LEFT JOIN medication_group_member gm ON gm.medication_id = s.medication_id
    LEFT JOIN medication_coding mc ON mc.medication_id = s.medication_id
    WHERE NOT EXISTS (SELECT 1 FROM medication_cessation c WHERE c.medication_id = s.medication_id)
```

Then append the conflict view:

```sql
-- ADR-0059 decision 5: two DIFFERENT anchors inside one reconciled group is a
-- possible-mis-reconciliation signal — two substances linked as one. Advisory worklist:
-- surfaced, never silently resolved and never auto-separated (reconciliation is a human
-- link, ADR-0047). Read-time and arrival-order independent, like
-- medication_group_cross_patient above: it lights up whenever the second coding lands
-- and clears when a separation or a coding correction repairs it.
CREATE OR REPLACE VIEW medication_group_coding_conflict AS
SELECT gm.group_id,
       count(DISTINCT (mc.coding_system, mc.coding_code)) AS anchor_count,
       array_agg(DISTINCT (mc.coding_system || '|' || mc.coding_code) COLLATE "C"
                 ORDER BY (mc.coding_system || '|' || mc.coding_code) COLLATE "C") AS anchors
FROM medication_group_member gm
JOIN medication_coding mc ON mc.medication_id = gm.medication_id
GROUP BY gm.group_id
HAVING count(DISTINCT (mc.coding_system, mc.coding_code)) > 1;
GRANT SELECT ON medication_group_coding_conflict TO cairn_agent;
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `CAIRN_TEST_PG=… cargo test --workspace`
Expected: PASS, including the pre-existing `medication_reconciliation` suite (the group-display
ordering change must not disturb uncoded groups).

- [ ] **Step 6: Commit**

```bash
git add db/031_medication.sql db/033_medication_reconciliation.sql crates/cairn-node/tests/medication_coding.rs
git commit -m "feat(medication 6a): dup-key on the (system, code) pair; prefer-coded display

The E1 key becomes coalesce(code:<system>|<code>, term:<normalized>) —
the PAIR, never a bare code, else the reserved finer drugref levels
re-split the same substance cross-node. Prefixed branches so a term can
never collide with a code key. This closes coded<->coded only; the
coded<->uncoded miss is asserted as a NEGATIVE test, because claiming
otherwise is the overstatement the ADR review caught.

medication_group_display now prefers a coded member, so a reconciled
group stops being identified by \"little white pill\". Two different
anchors in one group surface in medication_group_coding_conflict —
advisory, never auto-separated.

Refs ADR-0059, ADR-0047"
```

---

### Task 6: Honest degradation by construction, cross-node convergence, and the §5.9 issue

**Files:**
- Create: `crates/cairn-node/tests/no_drugref_dependency.rs`
- Modify: `crates/cairn-sync/tests/clinical_pull.rs` (one convergence assertion)

**Interfaces:**
- Consumes: everything above.
- Produces: no code interface; a standing structural guard and a filed issue.

- [ ] **Step 1: Write the failing source guard**

```rust
//! ADR-0059 decision 4 — honest degradation, proven by construction.
//!
//! A node without drugref must still read, sync, list and reconcile a CODED medication.
//! The strongest possible proof of that is structural: no drugref code exists in this
//! tree at all, so drugref-absent is the ONLY configuration every other test runs under.
//! A mocked absence could drift; this cannot.
//!
//! When a later slice adds the §9 advisory-tier drugref lookup, this guard must be
//! narrowed deliberately (to the trusted surface — db/ and the floor path), never simply
//! deleted: the load-bearing invariant is that the FLOOR and the PROJECTIONS never depend
//! on a drug database, not that no client code exists anywhere.
use std::fs;
use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root")
}

/// Every `.sql` under db/ and every `.rs` under crates/*/src — the trusted surface.
fn trusted_sources() -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![repo_root().join("db"), repo_root().join("crates")];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).expect("read dir") {
            let p = entry.expect("dir entry").path();
            if p.is_dir() {
                // tests/ may legitimately NAME drugref in prose; src/ and db/ may not.
                if p.file_name().is_some_and(|n| n == "target" || n == "tests") {
                    continue;
                }
                stack.push(p);
            } else if matches!(
                p.extension().and_then(|e| e.to_str()),
                Some("sql") | Some("rs")
            ) {
                out.push(p);
            }
        }
    }
    out
}

/// A mention inside a comment is fine (the ADR is cited all over db/041); an executable
/// reference is not. Crude but sufficient: flag a drugref mention on a line that is not a
/// comment.
fn offending_lines(path: &Path) -> Vec<String> {
    let text = fs::read_to_string(path).expect("read source");
    text.lines()
        .filter(|l| l.to_lowercase().contains("drugref"))
        .filter(|l| {
            let t = l.trim_start();
            !(t.starts_with("--") || t.starts_with("//") || t.starts_with("*") || t.starts_with("#"))
        })
        // A seeded registry row and the system token itself are DATA, not a dependency.
        .filter(|l| !l.contains("'drugref-moiety'") && !l.contains("drugref-clinical-drug")
                 && !l.contains("drugref-product") && !l.contains("\"drugref-moiety\""))
        .map(|l| l.trim().to_string())
        .collect()
}

#[test]
fn the_trusted_surface_never_calls_drugref() {
    let mut offenders: Vec<String> = Vec::new();
    for path in trusted_sources() {
        for line in offending_lines(&path) {
            offenders.push(format!("{}: {line}", path.display()));
        }
    }
    assert!(
        offenders.is_empty(),
        "the in-DB floor and the projections must never depend on a drug database \
         (ADR-0059 decision 4 — a coded medication reads, syncs and reconciles without \
         drugref). Offenders:\n{}",
        offenders.join("\n")
    );
}
```

- [ ] **Step 2: Run it to verify it passes for the right reason**

Run: `cargo test -p cairn-node --test no_drugref_dependency`
Expected: PASS. Then verify it is **not vacuous**: temporarily add a line
`PERFORM drugref_lookup('x');` to `db/041_medication_coding.sql`, re-run, confirm FAIL naming that line,
then remove it.

- [ ] **Step 3: Add the cross-node convergence assertion**

In `crates/cairn-sync/tests/clinical_pull.rs`, find the existing medication pull test that asserts a
statement converged on node B, and extend it: build the asserted body with a coding
(`coding: Some(SubstanceCoding { system: "drugref-moiety", code: "0f8c4b1e-1b7a-5c2d-9a3e-2b6f7c8d9e01", display: "atorvastatin" })`)
and after the pull assert on node B:

```rust
    let row = b
        .query_one(
            "SELECT coding_system, coding_code, coding_display \
               FROM medication_coding WHERE medication_id = $1::text::uuid",
            &[&med_id.to_string()],
        )
        .await
        .unwrap();
    assert_eq!(row.get::<_, String>(0), "drugref-moiety");
    assert_eq!(row.get::<_, String>(2), "atorvastatin");
```

- [ ] **Step 4: Run the full workspace with all three databases**

Run:
```bash
CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" \
CAIRN_TEST_PG2="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test2" \
CAIRN_TEST_PG3="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test3" \
cargo test --workspace
```
Expected: PASS. (If a `born_sealed_schema` positional-ROW error appears, the dev DBs predate an
`event_log` column add — recreate `cairn_test`/`2`/`3`.)

- [ ] **Step 5: File the §5.9 follow-on issue**

```bash
gh issue create \
  --title "safety projection must CARRY the coding-derived drug class, never re-derive it (ADR-0059 decision 4)" \
  --body "ADR-0059 decision 4 constrains the future §5.9 safety-projection slice (#232):

§5.9 says \"a coded drug's interaction class is a property of the code\" — a
knowledge lookup. So \"derivable from the event's own fields\" cannot mean the
READER re-derives it, which would make the §5.9 safety floor depend on drugref
after all. The class must be computed PRE-SEAL on the coding node (which by
construction had a coding authority in hand), captured, and carried on the
projection §5.9 already replicates in the clear.

ADR-0059's follow-on section lists \"the §5.9 safety projection must fire on a
drugref-less node from the captured class\" as a first-class test obligation of
the medication coding code slice, but decision 4 of the same ADR owes the class
field to the safety-projection slice. Slice 6a (2026-07-27) therefore could not
meet it — there is no safety projection to fire. This issue carries the
obligation forward so the safety-projection slice inherits it.

Blocked on: #232 (safety-projection emission)." \
  --label enhancement
```

- [ ] **Step 6: Commit**

```bash
git add crates/cairn-node/tests/no_drugref_dependency.rs crates/cairn-sync/tests/clinical_pull.rs
git commit -m "test(medication 6a): honest degradation by construction + cross-node coding

A source guard pins that no db/ file and no crate src/ path references
drugref executably, so drugref-absent is the only configuration the whole
suite runs under — a stronger proof than a mocked absence. clinical_pull
now asserts a coded medication converges to the identical coding row on
the pulling node.

Refs ADR-0059"
```

---

### Task 7: Documentation and the pull request

**Files:**
- Modify: `docs/HANDOVER.md`, `docs/ROADMAP.md`

- [ ] **Step 1: Update ROADMAP**

Add **Slice 56** after Slice 55, ~10 lines: what shipped (the coding shape, the two-tier floor + registry,
the coding projection, the pair dup-key, prefer-coded display, the conflict view), what is explicitly NOT
closed (coded↔uncoded, the overlay event types, the §5.9 class), and the follow-ons (slice 6b, the new
§5.9 issue).

- [ ] **Step 2: Update HANDOVER**

Rewrite the `⇒ NEXT` block to name **slice 6b** (the two coding-overlay event types) as the unblocked next
step, add a session block for 6a, and prune older session blocks — both files are over the 500-line target,
so condense the 2026-07-2x blocks into their one-line ROADMAP pointers rather than repeating detail.

- [ ] **Step 3: Final verification before the PR**

Run:
```bash
cargo fmt --all -- --check && cargo clippy --workspace --all-targets -- -D warnings
CAIRN_TEST_PG=… CAIRN_TEST_PG2=… CAIRN_TEST_PG3=… cargo test --workspace
```
Expected: all clean. Paste the real output into the PR body — never assert a pass you have not seen.

- [ ] **Step 4: Commit, push, open the PR**

```bash
git add docs/HANDOVER.md docs/ROADMAP.md
git commit -m "docs(medication 6a): ROADMAP slice 56 + HANDOVER currency"
git push -u origin feat/medication-coding-slice-6a-0059
gh pr create --title "feat(medication 6a): inline substance.coding, the drugref moiety anchor (ADR-0059)" --body "…"
```

The PR body states: what shipped, the two-tier floor rationale, the explicit non-claims (coded↔uncoded
still misses; no drugref code; no safety class), the new §5.9 issue number, and the verification output.

## Self-Review

- **Spec coverage:** wire shape → Task 1; retirement → Tasks 1–3; registry + floor → Task 3; projection +
  widened views → Task 4; dup-key, group display, conflict view → Task 5; honest degradation, cross-node,
  the §5.9 issue → Task 6; paper-parity → the section above plus the `an_uncoded_assertion_still_passes`
  and `coding_from_parts` tests. No spec section is unimplemented.
- **Type consistency:** `SubstanceCoding{system,code,display}` is used identically in Tasks 1, 2, 6;
  `coding_from_parts` has one signature; the SQL columns are `coding_system` / `coding_code` /
  `coding_display` everywhere (table, views, tests).
- **Known judgment call for the implementer:** Task 5's `reconcile_medications` call must be matched to
  its real signature — verify against `crates/cairn-node/tests/medication_reconciliation.rs` before
  running.
