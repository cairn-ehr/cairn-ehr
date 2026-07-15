# Medication dose effective-date / reason correction — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Extend the existing `clinical.medication-dose-correction.asserted` verb so a correction can patch a targeted dose point's effective date and clinical reason (in addition to the dose value), with an explicit `strike` for set-to-unknown; the corrected effective date drives current-dose winner selection.

**Architecture:** Per-field patch over the slice-2 dose-timeline overlay (`medication_dose_correction`, db/032). Three independent patch groups — `dose`, `effective`, `reason` — each *set* / *strike* / *keep*. New migration `db/035` `ALTER`-extends the overlay table, re-defines the correction floor + apply trigger + the two dose-timeline views (same column sets — no view widening). Rust builder (`cairn-event`) + orchestrator (`cairn-node`) + CLI gain the new fields. No new event type; the overlay stays one-row-per-point HLC-wins convergent.

**Tech Stack:** Rust (workspace crates `cairn-event`, `cairn-node`), PostgreSQL 18 + `cairn_pgx` (pgrx), `tokio-postgres`, `clap`, `serde_json`. Design spec: [docs/superpowers/specs/2026-07-15-medication-dose-effective-correction-design.md](../specs/2026-07-15-medication-dose-effective-correction-design.md).

## Global Constraints

- **AGPL-3.0**; every dependency AGPL-3.0-compatible (no new deps in this slice).
- **TDD** — failing test first, then minimal code (load-bearing on the in-DB floor, §9 safety-critical).
- **Inline docs for a junior contributor** — every non-trivial fn/module explains *why* it exists and *how* it fits.
- **Files under ~500 lines** where feasible; the touched files stay well under.
- **Never hard-code cryptographic material in tests** — derive at runtime (`generate_key()`); house rule 6.
- **All tests pass before commit** — `cargo test --workspace`, `cargo fmt --check` (both cargo trees), `cargo clippy --workspace -- -D warnings`.
- **Optional payload fields are omitted when absent, never serialized as `null`** (content-address stability, principle 11).
- **No SCHEMA-counter bump, no new event type, no new envelope field**; db/031–034 files untouched (db/032's objects are `CREATE OR REPLACE`/`ALTER`-extended at load by db/035).
- **DB-gated tests** read `CAIRN_TEST_PG` (e.g. `host=127.0.0.1 port=5532 user=hherb dbname=cairn_test`), self-serialize via `db::test_serial_guard`, and `return` early when the env var is absent.
- **Schema files load from an explicit `include_str!` array in `crates/cairn-node/src/db.rs`** — a new `db/*.sql` file does NOT load until registered there.

---

### Task 1: Pure `cairn-event` correction builder + twin (per-field patch + strike + note)

**Files:**
- Modify: `crates/cairn-event/src/medication/dose.rs` (struct `DoseCorrection` ~35-42; `dose_correction_body` ~91-107; `render_dose_correction_twin` ~111-126; correction tests ~202-279)

**Interfaces:**
- Produces: `DoseCorrection<'a>` with fields `medication_id`, `corrects`, `dose_amount: Option<&str>`, `dose_unit: Option<&str>`, `effective: Option<&str>`, `effective_precision: Option<&str>`, `reason: Option<&str>` (now the *point's clinical reason*), `strike: &'a [&'a str]`, `note: Option<&str>`, `info_source: Option<&str>`; `dose_correction_body(&DoseCorrection) -> serde_json::Value`; `render_dose_correction_twin(&DoseCorrection) -> String`. Names unchanged, so `mod.rs` re-exports need no edit.

- [ ] **Step 1: Rewrite the correction tests (failing) for the new struct + semantics**

Replace the correction test fns (`correction_body_carries_and_omits_correctly`, `correction_to_unknown_omits_dose`, `correction_twin_nonempty_including_to_unknown`, `correction_twin_surfaces_info_source_when_present`) in the `#[cfg(test)] mod tests` block with:

```rust
    fn full_correction() -> DoseCorrection<'static> {
        DoseCorrection {
            medication_id: "11111111-1111-7111-8111-111111111111",
            corrects: "33333333-3333-7333-8333-333333333333",
            dose_amount: Some("20"),
            dose_unit: Some("mg"),
            effective: Some("2024-01"),
            effective_precision: Some("month"),
            reason: Some("titration"),
            strike: &[],
            note: Some("mis-keyed the date"),
            info_source: Some("clinician-observed"),
        }
    }

    #[test]
    fn correction_body_carries_all_patch_fields() {
        let v = dose_correction_body(&full_correction());
        assert_eq!(v["corrects"], "33333333-3333-7333-8333-333333333333");
        assert_eq!(v["dose"]["amount"], "20");
        assert_eq!(v["dose"]["unit"], "mg");
        assert_eq!(v["effective"]["value"], "2024-01");
        assert_eq!(v["effective"]["precision"], "month");
        assert_eq!(v["reason"], "titration");
        assert_eq!(v["note"], "mis-keyed the date");
        assert_eq!(v["info_source"], "clinician-observed");
        assert!(!v.as_object().unwrap().contains_key("strike"), "empty strike omitted");
    }

    #[test]
    fn correction_body_effective_only_omits_dose_and_reason() {
        // Fix ONLY the date — dose/reason keys absent (patch: untouched, not struck).
        let c = DoseCorrection {
            medication_id: "11111111-1111-7111-8111-111111111111",
            corrects: "33333333-3333-7333-8333-333333333333",
            dose_amount: None,
            dose_unit: None,
            effective: Some("2024-01"),
            effective_precision: None,
            reason: None,
            strike: &[],
            note: None,
            info_source: None,
        };
        let v = dose_correction_body(&c);
        let obj = v.as_object().unwrap();
        assert!(!obj.contains_key("dose"), "untouched dose omitted");
        assert!(!obj.contains_key("reason"), "untouched reason omitted");
        assert!(!obj.contains_key("strike"));
        assert_eq!(v["effective"]["value"], "2024-01");
        assert!(!v["effective"].as_object().unwrap().contains_key("precision"));
    }

    #[test]
    fn correction_body_strike_emits_group_list() {
        // "the 40 was a guess — strike the dose to unknown" (principle 4, now explicit).
        let c = DoseCorrection {
            medication_id: "11111111-1111-7111-8111-111111111111",
            corrects: "33333333-3333-7333-8333-333333333333",
            dose_amount: None,
            dose_unit: None,
            effective: None,
            effective_precision: None,
            reason: None,
            strike: &["dose"],
            note: Some("was a guess"),
            info_source: None,
        };
        let v = dose_correction_body(&c);
        assert_eq!(v["strike"][0], "dose");
        assert!(!v.as_object().unwrap().contains_key("dose"), "struck group carries no set value");
        assert_eq!(v["note"], "was a guess");
    }

    #[test]
    fn correction_twin_is_nonempty_for_set_strike_and_note() {
        let s = render_dose_correction_twin(&full_correction());
        assert!(s.contains("20 mg"));
        assert!(s.contains("2024-01"));
        assert!(s.contains("titration"));
        assert!(s.contains("mis-keyed the date"));
        assert!(s.contains("clinician-observed"));

        let struck = DoseCorrection {
            medication_id: "11111111-1111-7111-8111-111111111111",
            corrects: "33333333-3333-7333-8333-333333333333",
            dose_amount: None, dose_unit: None,
            effective: None, effective_precision: None,
            reason: None, strike: &["effective"], note: None, info_source: None,
        };
        let t = render_dose_correction_twin(&struck);
        assert!(t.contains("effective"), "twin names the struck group, got: {t}");
        assert!(!t.trim().is_empty());
    }
```

- [ ] **Step 2: Run the tests to verify they fail (compile error / assertion)**

Run: `cargo test -p cairn-event medication::dose 2>&1 | tail -20`
Expected: FAIL — `DoseCorrection` has no field `effective`/`strike`/`note` (compile error).

- [ ] **Step 3: Extend the `DoseCorrection` struct**

Replace the struct (dose.rs ~35-42) with:

```rust
/// Clinician-supplied fields of a dose correction — a PER-FIELD PATCH of a targeted
/// dose point. Each group (dose / effective / reason) is independently *set* (a Some /
/// non-empty value), *struck* (named in `strike` → set-unknown), or *kept* (absent).
/// `reason` is the point's clinical reason (why the dose is what it is); `note` is why
/// THIS correction was made (audit). See ADR-0050.
pub struct DoseCorrection<'a> {
    pub medication_id: &'a str,
    /// The dose event (`dose_event_id`) this correction targets.
    pub corrects: &'a str,
    /// Set the dose amount; paired with `dose_unit`. None = don't set via a value.
    pub dose_amount: Option<&'a str>,
    /// Set the dose unit. None = don't set via a value.
    pub dose_unit: Option<&'a str>,
    /// Set the point's effective date (value). None = don't touch effective.
    pub effective: Option<&'a str>,
    /// Precision token for `effective` (year|month|day|year-range); only with `effective`.
    pub effective_precision: Option<&'a str>,
    /// Set the point's clinical reason. None = don't touch reason.
    pub reason: Option<&'a str>,
    /// Groups to set-unknown — subset of {"dose","effective","reason"}. Empty = none.
    pub strike: &'a [&'a str],
    /// Why THIS correction was made (audit; always additive). None = omit.
    pub note: Option<&'a str>,
    /// Provenance of the correction claim. None = omit.
    pub info_source: Option<&'a str>,
}
```

- [ ] **Step 4: Rewrite `dose_correction_body`**

Replace the fn (dose.rs ~91-107) with:

```rust
/// Build the `clinical.medication-dose-correction.asserted` payload. Each optional
/// group is emitted only when set; `strike` only when non-empty; `note`/`info_source`
/// only when present (honest-omit, never null — principle 11).
pub fn dose_correction_body(d: &DoseCorrection) -> Value {
    let mut p = json!({
        "medication_id": d.medication_id,
        "corrects": d.corrects,
    });
    let obj = p.as_object_mut().expect("json! built an object");
    if let Some(dose) = dose_object(d.dose_amount, d.dose_unit) {
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
    if !d.strike.is_empty() {
        obj.insert("strike".into(), json!(d.strike));
    }
    if let Some(n) = d.note {
        obj.insert("note".into(), json!(n));
    }
    if let Some(src) = d.info_source {
        obj.insert("info_source".into(), json!(src));
    }
    p
}
```

- [ ] **Step 5: Rewrite `render_dose_correction_twin`**

Replace the fn (dose.rs ~111-126) with:

```rust
/// The §3.13 legibility twin for a dose correction. Lists set fields, struck groups,
/// the clinical reason and the audit note. Always non-empty ("Dose correction" prefix).
pub fn render_dose_correction_twin(d: &DoseCorrection) -> String {
    let mut parts: Vec<String> = Vec::new();
    match (d.dose_amount, d.dose_unit) {
        (Some(a), Some(u)) => parts.push(format!("dose {a} {u}")),
        (Some(a), None) => parts.push(format!("dose {a}")),
        (None, Some(u)) => parts.push(format!("dose {u}")),
        (None, None) => {}
    }
    if let Some(v) = d.effective {
        parts.push(format!("effective {v}"));
    }
    if let Some(r) = d.reason {
        parts.push(format!("reason \"{r}\""));
    }
    for g in d.strike {
        parts.push(format!("{g} struck (unknown)"));
    }
    let mut s = String::from("Dose correction");
    if !parts.is_empty() {
        s.push_str(": ");
        s.push_str(&parts.join(", "));
    }
    if let Some(n) = d.note {
        s.push_str(&format!(" — {n}"));
    }
    if let Some(src) = d.info_source {
        s.push_str(&format!(" ({src})"));
    }
    s
}
```

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cargo test -p cairn-event medication::dose 2>&1 | tail -20`
Expected: PASS (all `medication::dose` tests, including the unchanged `DoseChange` tests).

- [ ] **Step 7: fmt + commit**

```bash
cargo fmt -p cairn-event
git add crates/cairn-event/src/medication/dose.rs
git commit -m "feat(medication): per-field-patch dose correction builder (effective/reason/strike/note)"
```

---

### Task 2: `cairn-node` orchestrator input + build fn + update all call sites

**Files:**
- Modify: `crates/cairn-node/src/medication/dose.rs` (`CorrectDoseInput` ~100-106; `build_dose_correction_body` ~120-127 mapping; test `build_correction_sets_type_schema_corrects` ~258-279)
- Modify: `crates/cairn-node/tests/medication_dose.rs` (all 8 `CorrectDoseInput { … }` sites, incl. the correct-to-unknown one at ~535)

**Interfaces:**
- Consumes: `cairn_event::medication::DoseCorrection` (Task 1).
- Produces: `CorrectDoseInput<'a>` with `dose_amount`, `dose_unit`, `effective: Option<&str>`, `effective_precision: Option<&str>`, `reason: Option<&str>`, `strike: &'a [&'a str]`, `note: Option<&str>`, `info_source: Option<&str>`. `build_dose_correction_body(...)` and `correct_dose(...)` signatures otherwise unchanged.

- [ ] **Step 1: Update the pure build test (failing) to the new input shape**

Replace `build_correction_sets_type_schema_corrects` (dose.rs ~258-279) body's input with:

```rust
        let input = CorrectDoseInput {
            dose_amount: Some("20"),
            dose_unit: Some("mg"),
            effective: Some("2024-01"),
            effective_precision: Some("month"),
            reason: Some("titration"),
            strike: &[],
            note: Some("mis-keyed"),
            info_source: None,
        };
```

And add two assertions after the existing ones in that test:

```rust
        assert_eq!(b.payload["effective"]["value"], "2024-01");
        assert_eq!(b.payload["reason"], "titration");
```

- [ ] **Step 2: Run to verify it fails (compile error)**

Run: `cargo test -p cairn-node --lib medication::dose 2>&1 | tail -20`
Expected: FAIL — `CorrectDoseInput` has no field `effective`/`strike`/`note`.

- [ ] **Step 3: Extend `CorrectDoseInput` and its mapping**

Replace `CorrectDoseInput` (dose.rs ~100-106) with:

```rust
/// Clinician-supplied fields of a dose correction (per-field patch; see ADR-0050).
/// A group is *set* (Some / value), *struck* (named in `strike` → unknown), or *kept*
/// (absent). `reason` = the point's clinical reason; `note` = why this correction exists.
pub struct CorrectDoseInput<'a> {
    pub dose_amount: Option<&'a str>,
    pub dose_unit: Option<&'a str>,
    pub effective: Option<&'a str>,
    pub effective_precision: Option<&'a str>,
    pub reason: Option<&'a str>,
    pub strike: &'a [&'a str],
    pub note: Option<&'a str>,
    pub info_source: Option<&'a str>,
}
```

In `build_dose_correction_body` (dose.rs ~120-127), replace the `DoseCorrection { … }` literal with:

```rust
    let d = DoseCorrection {
        medication_id: &mid,
        corrects: &corrects_s,
        dose_amount: input.dose_amount,
        dose_unit: input.dose_unit,
        effective: input.effective,
        effective_precision: input.effective_precision,
        reason: input.reason,
        strike: input.strike,
        note: input.note,
        info_source: input.info_source,
    };
```

- [ ] **Step 4: Update all 8 `CorrectDoseInput` sites in the integration test for compilation + intent**

For the seven dose-value correction sites (lines ~189, 486, 574, 651, 669, 739, 832 — each currently `{ dose_amount: Some(...), dose_unit: Some(...), info_source, reason }`), add the four new fields, mapping the old `reason` (a correction-why) to `note` and leaving the point-`reason` untouched:

```rust
        // pattern applied to each dose-value correction site (values per site):
        &CorrectDoseInput {
            dose_amount: Some("…"),      // unchanged per site
            dose_unit: Some("…"),        // unchanged per site
            effective: None,
            effective_precision: None,
            reason: None,
            strike: &[],
            note: Some("…"),             // the site's old `reason` string moves here
            info_source: None,           // unchanged per site
        },
```

For the **correct-to-unknown** site (~535, the `dose_amount: None` one asserting `amt == None`), convert omission → explicit strike:

```rust
        &CorrectDoseInput {
            dose_amount: None,
            dose_unit: None,
            effective: None,
            effective_precision: None,
            reason: None,
            strike: &["dose"],
            note: Some("was a guess"),
            info_source: None,
        },
```

(Its assertions `amt == None`, `unit == None`, `corrected == true` remain valid: a struck dose reads unknown, and a correction row exists.)

- [ ] **Step 5: Run to verify lib + integration compile and pass**

Run: `cargo test -p cairn-node --lib medication::dose 2>&1 | tail -20`
Expected: PASS (pure build test).
Run: `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test -p cairn-node --test medication_dose 2>&1 | tail -25`
Expected: PASS — existing behavior preserved (db/035 not loaded yet; the old projection still surfaces a struck/empty-amount correction as `NULL` via its whole-row logic).

- [ ] **Step 6: fmt + commit**

```bash
cargo fmt -p cairn-node
git add crates/cairn-node/src/medication/dose.rs crates/cairn-node/tests/medication_dose.rs
git commit -m "feat(medication): extend CorrectDoseInput for per-field patch; convert correct-to-unknown to explicit strike"
```

---

### Task 3: `db/035` migration (columns + backfill + floor + trigger + views) + register; driving DB tests

**Files:**
- Create: `db/035_medication_dose_effective_correction.sql`
- Modify: `crates/cairn-node/src/db.rs` (register in the `include_str!` array, before the closing `];` at ~179)
- Test: `crates/cairn-node/tests/medication_dose.rs` (append the two driving tests + a small `history()` helper if not present)

**Interfaces:**
- Consumes: `correct_dose` / `CorrectDoseInput` (Task 2), `change_dose`, `assert_medication`, `resolve_correction_target`, existing test helpers `setup_node`, `sample_assert`, `current_dose`.
- Produces: db/035 objects — extended `medication_dose_correction` columns (`effective_value`, `effective_precision`, `note`, `dose_corrected`, `effective_corrected`, `reason_corrected`), the extended `cairn_check_medication_dose` floor, the extended `medication_dose_correction_apply` trigger fn, and the `medication_current_dose` / `patient_medication_dose_history` views reading corrected effective/reason.

- [ ] **Step 1: Write the two driving DB-gated tests (failing)**

Append to `crates/cairn-node/tests/medication_dose.rs`. First a helper that reads the current-dose effective value (add near `current_dose`):

```rust
/// The current-dose winner's effective_value for a thread (None if no timeline).
async fn current_effective(c: &Client, med_id: Uuid) -> Option<String> {
    c.query_opt(
        "SELECT effective_value FROM medication_current_dose WHERE medication_id = $1::text::uuid",
        &[&med_id.to_string()],
    )
    .await
    .unwrap()
    .and_then(|r| r.get::<_, Option<String>>(0))
}
```

```rust
/// Headline: correcting a point's effective date FORWARD makes a previously-earlier
/// point win as the current dose (winner selection is by effective date, so the fix is
/// bitemporal repair, not a label). Assert (2020) → change to 80mg effective 2025-06 →
/// change to 60mg effective 2024-01. Current = the 2025-06/80mg point. Then correct the
/// 80mg point's effective back to 2023-01: now the 60mg/2024-01 point is the latest → wins.
#[tokio::test]
async fn corrected_effective_flips_current_dose_winner() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient = Uuid::now_v7();
    let med_id = Uuid::now_v7();

    assert_medication(&mut c, &sk, &kid, "test-node", patient, med_id, &sample_assert(), None)
        .await
        .unwrap();
    let late = change_dose(&mut c, &sk, &kid, "test-node", patient, med_id,
        &ChangeDoseInput { dose_amount: Some("80"), dose_unit: Some("mg"),
            effective: Some("2025-06"), effective_precision: Some("month"),
            info_source: "clinician-observed", reason: Some("titration") }, None)
        .await.unwrap();
    change_dose(&mut c, &sk, &kid, "test-node", patient, med_id,
        &ChangeDoseInput { dose_amount: Some("60"), dose_unit: Some("mg"),
            effective: Some("2024-01"), effective_precision: Some("month"),
            info_source: "clinician-observed", reason: None }, None)
        .await.unwrap();

    let (amt0, _u, _de, _c0) = current_dose(&c, med_id).await;
    assert_eq!(amt0.as_deref(), Some("80"), "before correction the 2025-06 point is current");

    // Correct the 80mg point's effective back to 2023-01 (a date-only patch).
    correct_dose(&mut c, &sk, &kid, "test-node", patient, med_id, late,
        &CorrectDoseInput { dose_amount: None, dose_unit: None,
            effective: Some("2023-01"), effective_precision: Some("month"),
            reason: None, strike: &[], note: Some("mis-keyed the date"), info_source: None }, None)
        .await.unwrap();

    let (amt1, _u, _de, _c1) = current_dose(&c, med_id).await;
    assert_eq!(amt1.as_deref(), Some("60"),
        "after the date fix the 2024-01/60mg point is latest and wins");
    // And the corrected effective is surfaced on the moved point via history.
    assert_eq!(current_effective(&c, med_id).await.as_deref(), Some("2024-01"));
}

/// The floor rejects a no-op correction (touches no group) — under patch semantics a
/// bare correction is meaningless (slice 2's implicit "omit = strike dose" is gone).
#[tokio::test]
async fn floor_rejects_no_op_correction() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let patient = Uuid::now_v7();
    let med_id = Uuid::now_v7();
    assert_medication(&mut c, &sk, &kid, "test-node", patient, med_id, &sample_assert(), None)
        .await.unwrap();
    let target = resolve_correction_target(&c, med_id, None).await.unwrap();

    let err = correct_dose(&mut c, &sk, &kid, "test-node", patient, med_id, target,
        &CorrectDoseInput { dose_amount: None, dose_unit: None, effective: None,
            effective_precision: None, reason: None, strike: &[], note: None, info_source: None },
        None)
        .await
        .unwrap_err();
    assert!(format!("{err:#}").contains("must set or strike at least one"),
        "expected no-op floor rejection, got: {err:#}");
}
```

- [ ] **Step 2: Run to verify they fail**

Run: `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test -p cairn-node --test medication_dose corrected_effective_flips floor_rejects_no_op 2>&1 | tail -25`
Expected: FAIL — `corrected_effective_flips…` gets `Some("80")` still current (old projection ignores the effective correction); `floor_rejects_no_op…` gets Ok (old floor accepts a bare correction).

- [ ] **Step 3: Create `db/035_medication_dose_effective_correction.sql`**

```sql
-- 035_medication_dose_effective_correction.sql — slice 5 of clinical.medication (§3.15/§3.16).
--
-- Extends the slice-2 dose-correction overlay (db/032) so a correction PATCHES a targeted
-- dose point per-field: dose (amount+unit), effective (value+precision), and the point's
-- clinical reason. Omit a group = keep; name it = set; list it in `strike` = set-unknown.
-- The CORRECTED effective date drives current-dose winner selection (bitemporal repair,
-- not a display label). db/031-034 UNTOUCHED — all slice-5 SQL is here (ADR-0050).
--
-- Convergence: the overlay stays ONE row per corrected point, highest-HLC-wins WHOLESALE
-- (cairn_hlc_overlay_wins), so set-union sync stays convergent. Per-field patch therefore
-- applies WITHIN one correction (vs the original point); a later correction supersedes an
-- earlier one rather than field-merging (documented boundary; field-merge would need
-- per-field HLC tracking).
BEGIN;

-- 1. Extend the correction overlay with effective + note + per-group touched-flags. The
--    flags disambiguate "struck to NULL" from "untouched" — a nullable value column
--    cannot. Added nullable so a pre-035 row (flags NULL) is distinguishable; backfilled
--    in step 2.
ALTER TABLE medication_dose_correction ADD COLUMN IF NOT EXISTS effective_value     TEXT;
ALTER TABLE medication_dose_correction ADD COLUMN IF NOT EXISTS effective_precision TEXT;
ALTER TABLE medication_dose_correction ADD COLUMN IF NOT EXISTS note                TEXT;
ALTER TABLE medication_dose_correction ADD COLUMN IF NOT EXISTS dose_corrected      BOOLEAN;
ALTER TABLE medication_dose_correction ADD COLUMN IF NOT EXISTS effective_corrected BOOLEAN;
ALTER TABLE medication_dose_correction ADD COLUMN IF NOT EXISTS reason_corrected    BOOLEAN;

-- 2. Backfill pre-035 rows (idempotent; guarded on dose_corrected IS NULL). A slice-2
--    correction row was a whole-row DOSE correction, and `reason` held the correction-why
--    (now `note`). New rows always set the flags, so this touches a legacy row at most
--    once and never re-clobbers a new one.
UPDATE medication_dose_correction
   SET dose_corrected      = TRUE,
       effective_corrected = FALSE,
       reason_corrected    = FALSE,
       note                = reason,
       reason              = NULL
 WHERE dose_corrected IS NULL;

-- 3. Extend the correction floor (whole fn re-created; the dose-change branch is
--    byte-identical to db/032, the correction branch gains strike/patch validation).
--    Registry mapping (db/032) is untouched — same fn name/signature.
CREATE OR REPLACE FUNCTION cairn_check_medication_dose(p_type text, b jsonb)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    p jsonb := b -> 'payload';
BEGIN
    IF p IS NULL THEN
        RAISE EXCEPTION 'medication dose: missing payload';
    END IF;
    IF jsonb_typeof(p -> 'medication_id') IS DISTINCT FROM 'string' THEN
        RAISE EXCEPTION 'medication dose: medication_id must be a uuid string';
    END IF;
    BEGIN
        PERFORM (p ->> 'medication_id')::uuid;
    EXCEPTION WHEN others THEN
        RAISE EXCEPTION 'medication dose: medication_id must be a valid uuid';
    END;

    IF p_type = 'clinical.medication-dose-change.asserted' THEN
        IF jsonb_typeof(p -> 'info_source') IS DISTINCT FROM 'string'
           OR length(btrim(p ->> 'info_source')) = 0 THEN
            RAISE EXCEPTION 'medication dose-change: info_source must be a non-empty string';
        END IF;
        IF NOT (
            (p -> 'dose' ->> 'amount') IS NOT NULL OR (p -> 'dose' ->> 'unit') IS NOT NULL
            OR (p -> 'effective' ->> 'value') IS NOT NULL
            OR COALESCE(jsonb_typeof(p -> 'reason') = 'string' AND length(btrim(p ->> 'reason')) > 0, FALSE)
        ) THEN
            RAISE EXCEPTION 'medication dose-change: must carry a dose, an effective date, or a reason (principle 4 floor)';
        END IF;
    ELSIF p_type = 'clinical.medication-dose-correction.asserted' THEN
        IF jsonb_typeof(p -> 'corrects') IS DISTINCT FROM 'string' THEN
            RAISE EXCEPTION 'medication dose-correction: corrects must be a uuid string';
        END IF;
        BEGIN
            PERFORM (p ->> 'corrects')::uuid;
        EXCEPTION WHEN others THEN
            RAISE EXCEPTION 'medication dose-correction: corrects must be a valid uuid';
        END;
        -- `strike`, if present, is a JSON array over the closed group set.
        IF p ? 'strike' THEN
            IF jsonb_typeof(p -> 'strike') IS DISTINCT FROM 'array' THEN
                RAISE EXCEPTION 'medication dose-correction: strike must be a JSON array of group names';
            END IF;
            IF EXISTS (
                SELECT 1 FROM jsonb_array_elements_text(p -> 'strike') g
                WHERE g NOT IN ('dose', 'effective', 'reason')
            ) THEN
                RAISE EXCEPTION 'medication dose-correction: strike may only contain dose|effective|reason';
            END IF;
        END IF;
        -- A group cannot be both set and struck.
        IF (p ? 'dose'      AND COALESCE(p -> 'strike' ? 'dose', FALSE))
           OR (p ? 'effective' AND COALESCE(p -> 'strike' ? 'effective', FALSE))
           OR (p ? 'reason'    AND COALESCE(p -> 'strike' ? 'reason', FALSE)) THEN
            RAISE EXCEPTION 'medication dose-correction: a group cannot be both set and struck';
        END IF;
        -- A SET group must carry a value — keeps `strike` the one canonical way to unknown.
        IF p ? 'dose' AND NOT ((p -> 'dose' ->> 'amount') IS NOT NULL OR (p -> 'dose' ->> 'unit') IS NOT NULL) THEN
            RAISE EXCEPTION 'medication dose-correction: a set dose must carry amount and/or unit (use strike to set unknown)';
        END IF;
        IF p ? 'effective' AND (p -> 'effective' ->> 'value') IS NULL THEN
            RAISE EXCEPTION 'medication dose-correction: a set effective must carry a value (use strike to set unknown)';
        END IF;
        IF p ? 'reason' AND COALESCE(length(btrim(p ->> 'reason')), 0) = 0 THEN
            RAISE EXCEPTION 'medication dose-correction: a set reason must be a non-empty string (use strike to set unknown)';
        END IF;
        -- Not a no-op: must set or strike at least one group.
        IF NOT (
            p ? 'dose' OR p ? 'effective' OR p ? 'reason'
            OR (p ? 'strike' AND jsonb_array_length(p -> 'strike') > 0)
        ) THEN
            RAISE EXCEPTION 'medication dose-correction: must set or strike at least one of dose|effective|reason (principle 4 floor)';
        END IF;
    END IF;
END;
$$;

-- 4. Fold a correction as a per-field patch overlay keyed by the TARGET dose event.
--    Each group's touched-flag = (set OR struck); its value = the set value, or NULL when
--    struck / untouched (the view uses the flag, never the raw NULL, to decide). HLC-wins
--    wholesale on a re-correction of the same point (ON CONFLICT). Offline-first: the
--    target need not exist locally.
CREATE OR REPLACE FUNCTION medication_dose_correction_apply()
RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE
    p jsonb := NEW.body;
    v_dose_set     boolean := p ? 'dose';
    v_eff_set      boolean := p ? 'effective';
    v_reason_set   boolean := p ? 'reason';
    v_dose_struck   boolean := COALESCE(p -> 'strike' ? 'dose', FALSE);
    v_eff_struck    boolean := COALESCE(p -> 'strike' ? 'effective', FALSE);
    v_reason_struck boolean := COALESCE(p -> 'strike' ? 'reason', FALSE);
BEGIN
    INSERT INTO medication_dose_correction
        (corrected_dose_event_id, medication_id, patient_id,
         amount, unit, effective_value, effective_precision, reason, note, info_source,
         dose_corrected, effective_corrected, reason_corrected,
         hlc_wall, hlc_counter, origin, content_address)
    VALUES (
        (p ->> 'corrects')::uuid, (p ->> 'medication_id')::uuid, NEW.patient_id,
        CASE WHEN v_dose_set THEN p -> 'dose' ->> 'amount' END,
        CASE WHEN v_dose_set THEN p -> 'dose' ->> 'unit'   END,
        CASE WHEN v_eff_set  THEN p -> 'effective' ->> 'value'     END,
        CASE WHEN v_eff_set  THEN p -> 'effective' ->> 'precision' END,
        CASE WHEN v_reason_set THEN p ->> 'reason' END,
        p ->> 'note',
        p ->> 'info_source',
        (v_dose_set OR v_dose_struck),
        (v_eff_set OR v_eff_struck),
        (v_reason_set OR v_reason_struck),
        NEW.hlc_wall, NEW.hlc_counter, NEW.node_origin, NEW.content_address)
    ON CONFLICT (corrected_dose_event_id) DO UPDATE SET
        medication_id       = EXCLUDED.medication_id,
        patient_id          = EXCLUDED.patient_id,
        amount              = EXCLUDED.amount,
        unit                = EXCLUDED.unit,
        effective_value     = EXCLUDED.effective_value,
        effective_precision = EXCLUDED.effective_precision,
        reason              = EXCLUDED.reason,
        note                = EXCLUDED.note,
        info_source         = EXCLUDED.info_source,
        dose_corrected      = EXCLUDED.dose_corrected,
        effective_corrected = EXCLUDED.effective_corrected,
        reason_corrected    = EXCLUDED.reason_corrected,
        hlc_wall            = EXCLUDED.hlc_wall,
        hlc_counter         = EXCLUDED.hlc_counter,
        origin              = EXCLUDED.origin,
        content_address     = EXCLUDED.content_address,
        updated_at          = clock_timestamp()
    WHERE cairn_hlc_overlay_wins(
        EXCLUDED.hlc_wall, EXCLUDED.hlc_counter, EXCLUDED.origin, EXCLUDED.content_address,
        medication_dose_correction.hlc_wall, medication_dose_correction.hlc_counter,
        medication_dose_correction.origin, medication_dose_correction.content_address);
    RETURN NULL;
END;
$$;
-- (db/032 already created medication_dose_correction_apply_trg on the same fn name; no
--  trigger DDL is needed here — the trigger picks up the replaced body.)

-- 5. Rework the two dose-timeline views to read corrected effective/reason via the
--    touched-flags. SAME column sets as db/032 (no widening — replay-safe). The effective
--    SORT KEY uses the corrected value, so winner selection + trail order reflect the fix.
CREATE OR REPLACE VIEW medication_current_dose AS
SELECT DISTINCT ON (de.medication_id)
    de.medication_id, de.patient_id, de.dose_event_id,
    CASE WHEN corr.dose_corrected THEN corr.amount ELSE de.amount END AS amount,
    CASE WHEN corr.dose_corrected THEN corr.unit   ELSE de.unit   END AS unit,
    CASE WHEN corr.effective_corrected THEN corr.effective_value     ELSE de.effective_value     END AS effective_value,
    CASE WHEN corr.effective_corrected THEN corr.effective_precision ELSE de.effective_precision END AS effective_precision,
    (corr.corrected_dose_event_id IS NOT NULL) AS corrected
FROM medication_dose_event de
LEFT JOIN medication_dose_correction corr
    ON corr.corrected_dose_event_id = de.dose_event_id
   AND corr.medication_id = de.medication_id
ORDER BY de.medication_id,
         cairn_dose_effective_sort_key(
             CASE WHEN corr.effective_corrected THEN corr.effective_value ELSE de.effective_value END,
             de.hlc_wall) COLLATE "C" DESC,
         de.hlc_wall DESC, de.hlc_counter DESC, de.origin COLLATE "C" DESC, de.content_address DESC;
GRANT SELECT ON medication_current_dose TO cairn_agent;

CREATE OR REPLACE VIEW patient_medication_dose_history AS
SELECT de.medication_id, de.patient_id, de.dose_event_id, de.is_initial,
       CASE WHEN corr.dose_corrected THEN corr.amount ELSE de.amount END AS amount,
       CASE WHEN corr.dose_corrected THEN corr.unit   ELSE de.unit   END AS unit,
       CASE WHEN corr.effective_corrected THEN corr.effective_value     ELSE de.effective_value     END AS effective_value,
       CASE WHEN corr.effective_corrected THEN corr.effective_precision ELSE de.effective_precision END AS effective_precision,
       de.info_source,
       CASE WHEN corr.reason_corrected THEN corr.reason ELSE de.reason END AS reason,
       (corr.corrected_dose_event_id IS NOT NULL) AS corrected,
       to_timestamp(de.hlc_wall / 1000.0) AS recorded_at
FROM medication_dose_event de
LEFT JOIN medication_dose_correction corr
    ON corr.corrected_dose_event_id = de.dose_event_id
   AND corr.medication_id = de.medication_id
ORDER BY de.medication_id,
         cairn_dose_effective_sort_key(
             CASE WHEN corr.effective_corrected THEN corr.effective_value ELSE de.effective_value END,
             de.hlc_wall) COLLATE "C" ASC,
         de.hlc_wall ASC, de.hlc_counter ASC, de.origin COLLATE "C" ASC, de.content_address ASC;
GRANT SELECT ON patient_medication_dose_history TO cairn_agent;

COMMIT;
```

- [ ] **Step 4: Register db/035 in the schema loader**

In `crates/cairn-node/src/db.rs`, insert before the closing `];` (~179):

```rust
    // §3.15/§3.16 slice 5: dose-correction per-field patch — effective/reason columns,
    // strike-aware floor + trigger, corrected-effective winner selection (ADR-0050).
    (
        "035_medication_dose_effective_correction",
        include_str!("../../../db/035_medication_dose_effective_correction.sql"),
    ),
```

- [ ] **Step 5: Run the driving tests to verify they pass**

Run: `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test -p cairn-node --test medication_dose corrected_effective_flips floor_rejects_no_op 2>&1 | tail -25`
Expected: PASS.
Run (regression — the whole dose suite): `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test -p cairn-node --test medication_dose 2>&1 | tail -25`
Expected: PASS (all, including the strike-converted correct-to-unknown test).

- [ ] **Step 6: fmt + commit**

```bash
cargo fmt -p cairn-node
git add db/035_medication_dose_effective_correction.sql crates/cairn-node/src/db.rs crates/cairn-node/tests/medication_dose.rs
git commit -m "feat(medication): db/035 per-field dose correction — corrected effective drives winner selection"
```

---

### Task 4: Comprehensive DB-gated coverage (floor + projection)

**Files:**
- Test: `crates/cairn-node/tests/medication_dose.rs` (append)

**Interfaces:**
- Consumes: everything from Tasks 2–3 + helpers `setup_node`, `sample_assert`, `current_dose`, `current_effective`, and a `dose_history` helper added below.

- [ ] **Step 1: Add a history-reading helper**

```rust
/// (amount, effective_value, reason) rows of a thread's dose history, effective-ASC.
async fn dose_history(c: &Client, med_id: Uuid) -> Vec<(Option<String>, Option<String>, Option<String>)> {
    c.query(
        "SELECT amount, effective_value, reason FROM patient_medication_dose_history \
         WHERE medication_id = $1::text::uuid \
         ORDER BY cairn_dose_effective_sort_key(effective_value, extract(epoch FROM recorded_at)::bigint*1000) COLLATE \"C\" ASC, dose_event_id",
        &[&med_id.to_string()],
    )
    .await
    .unwrap()
    .iter()
    .map(|r| (r.get(0), r.get(1), r.get(2)))
    .collect()
}
```

- [ ] **Step 2: Write the coverage tests (floor + projection)**

Append these tests. Each opens its own connection/guard (copy the 6-line preamble from `floor_rejects_no_op_correction`):

```rust
// Floor: an unknown strike token is rejected legibly (closed group set).
#[tokio::test]
async fn floor_rejects_unknown_strike_token() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let (patient, med_id) = (Uuid::now_v7(), Uuid::now_v7());
    assert_medication(&mut c, &sk, &kid, "test-node", patient, med_id, &sample_assert(), None).await.unwrap();
    let target = resolve_correction_target(&c, med_id, None).await.unwrap();
    let err = correct_dose(&mut c, &sk, &kid, "test-node", patient, med_id, target,
        &CorrectDoseInput { dose_amount: None, dose_unit: None, effective: None, effective_precision: None,
            reason: None, strike: &["bogus"], note: None, info_source: None }, None)
        .await.unwrap_err();
    assert!(format!("{err:#}").contains("strike may only contain"), "got: {err:#}");
}

// Floor: a group set AND struck in the same correction is a contradiction.
#[tokio::test]
async fn floor_rejects_set_and_struck_same_group() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let (patient, med_id) = (Uuid::now_v7(), Uuid::now_v7());
    assert_medication(&mut c, &sk, &kid, "test-node", patient, med_id, &sample_assert(), None).await.unwrap();
    let target = resolve_correction_target(&c, med_id, None).await.unwrap();
    let err = correct_dose(&mut c, &sk, &kid, "test-node", patient, med_id, target,
        &CorrectDoseInput { dose_amount: Some("20"), dose_unit: Some("mg"), effective: None, effective_precision: None,
            reason: None, strike: &["dose"], note: None, info_source: None }, None)
        .await.unwrap_err();
    assert!(format!("{err:#}").contains("both set and struck"), "got: {err:#}");
}

// Projection: correcting a point's reason surfaces the corrected reason (closes the
// slice-2 dead-column gap) and leaves the dose + effective untouched (per-field keep).
#[tokio::test]
async fn corrected_reason_surfaces_and_other_groups_kept() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let (patient, med_id) = (Uuid::now_v7(), Uuid::now_v7());
    assert_medication(&mut c, &sk, &kid, "test-node", patient, med_id, &sample_assert(), None).await.unwrap();
    let pt = change_dose(&mut c, &sk, &kid, "test-node", patient, med_id,
        &ChangeDoseInput { dose_amount: Some("80"), dose_unit: Some("mg"), effective: Some("2025-06"),
            effective_precision: Some("month"), info_source: "clinician-observed", reason: Some("titration") }, None)
        .await.unwrap();
    correct_dose(&mut c, &sk, &kid, "test-node", patient, med_id, pt,
        &CorrectDoseInput { dose_amount: None, dose_unit: None, effective: None, effective_precision: None,
            reason: Some("dose reduction, not titration"), strike: &[], note: Some("wrong reason keyed"),
            info_source: None }, None)
        .await.unwrap();
    let hist = dose_history(&c, med_id).await;
    // The corrected point keeps 80mg + 2025-06 but shows the corrected reason.
    assert!(hist.iter().any(|(a, e, r)|
        a.as_deref() == Some("80") && e.as_deref() == Some("2025-06")
        && r.as_deref() == Some("dose reduction, not titration")),
        "corrected reason must surface with dose/effective kept, got: {hist:?}");
}

// Projection: strike dose → unknown, while effective/reason on the same point are kept.
#[tokio::test]
async fn strike_dose_reads_unknown_others_kept() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let (patient, med_id) = (Uuid::now_v7(), Uuid::now_v7());
    assert_medication(&mut c, &sk, &kid, "test-node", patient, med_id, &sample_assert(), None).await.unwrap();
    let pt = change_dose(&mut c, &sk, &kid, "test-node", patient, med_id,
        &ChangeDoseInput { dose_amount: Some("80"), dose_unit: Some("mg"), effective: Some("2025-06"),
            effective_precision: Some("month"), info_source: "clinician-observed", reason: Some("titration") }, None)
        .await.unwrap();
    correct_dose(&mut c, &sk, &kid, "test-node", patient, med_id, pt,
        &CorrectDoseInput { dose_amount: None, dose_unit: None, effective: None, effective_precision: None,
            reason: None, strike: &["dose"], note: Some("was a guess"), info_source: None }, None)
        .await.unwrap();
    let (amt, unit, _de, corrected) = current_dose(&c, med_id).await;
    assert_eq!(amt, None, "struck dose reads unknown");
    assert_eq!(unit, None);
    assert!(corrected);
    assert_eq!(current_effective(&c, med_id).await.as_deref(), Some("2025-06"), "effective kept");
}

// Convergence: a later (higher-HLC) correction of the SAME point supersedes the earlier
// one WHOLESALE (documented boundary — not field-merged).
#[tokio::test]
async fn later_correction_supersedes_earlier_wholesale() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let (patient, med_id) = (Uuid::now_v7(), Uuid::now_v7());
    assert_medication(&mut c, &sk, &kid, "test-node", patient, med_id, &sample_assert(), None).await.unwrap();
    let pt = change_dose(&mut c, &sk, &kid, "test-node", patient, med_id,
        &ChangeDoseInput { dose_amount: Some("80"), dose_unit: Some("mg"), effective: Some("2025-06"),
            effective_precision: Some("month"), info_source: "clinician-observed", reason: None }, None)
        .await.unwrap();
    // Correction A: fix the effective only.
    correct_dose(&mut c, &sk, &kid, "test-node", patient, med_id, pt,
        &CorrectDoseInput { dose_amount: None, dose_unit: None, effective: Some("2024-01"),
            effective_precision: Some("month"), reason: None, strike: &[], note: None, info_source: None }, None)
        .await.unwrap();
    // Correction B (later HLC): fix the dose only — supersedes A wholesale.
    correct_dose(&mut c, &sk, &kid, "test-node", patient, med_id, pt,
        &CorrectDoseInput { dose_amount: Some("40"), dose_unit: Some("mg"), effective: None,
            effective_precision: None, reason: None, strike: &[], note: None, info_source: None }, None)
        .await.unwrap();
    let (amt, _u, _de, _c) = current_dose(&c, med_id).await;
    assert_eq!(amt.as_deref(), Some("40"), "B's dose wins");
    assert_eq!(current_effective(&c, med_id).await.as_deref(), Some("2025-06"),
        "B did not touch effective → reverts to the original (wholesale supersede, documented boundary)");
}

// Backfill: a pre-035-shaped row (flags NULL, reason = correction-why) is normalized to
// dose_corrected=TRUE / note=reason / reason=NULL, and the backfill is idempotent.
#[tokio::test]
async fn backfill_normalizes_legacy_row_idempotently() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    c.batch_execute(
        "DO $$ BEGIN IF to_regclass('public.medication_dose_correction') IS NOT NULL \
           THEN TRUNCATE medication_dose_correction; END IF; END $$;").await.unwrap();
    // Simulate a legacy row: value columns set, all touched-flags NULL, reason=why.
    c.execute(
        "INSERT INTO medication_dose_correction \
           (corrected_dose_event_id, medication_id, patient_id, amount, unit, reason, \
            dose_corrected, effective_corrected, reason_corrected, \
            hlc_wall, hlc_counter, origin, content_address) \
         VALUES (gen_random_uuid(), gen_random_uuid(), gen_random_uuid(), '20', 'mg', 'mis-keyed', \
            NULL, NULL, NULL, 1, 0, 'legacy', '\\x00')",
        &[]).await.unwrap();
    let backfill = "UPDATE medication_dose_correction \
        SET dose_corrected = TRUE, effective_corrected = FALSE, reason_corrected = FALSE, \
            note = reason, reason = NULL \
        WHERE dose_corrected IS NULL";
    c.execute(backfill, &[]).await.unwrap();
    let row = c.query_one(
        "SELECT dose_corrected, effective_corrected, reason_corrected, note, reason \
         FROM medication_dose_correction", &[]).await.unwrap();
    assert_eq!(row.get::<_, bool>(0), true);
    assert_eq!(row.get::<_, bool>(1), false);
    assert_eq!(row.get::<_, bool>(2), false);
    assert_eq!(row.get::<_, Option<String>>(3).as_deref(), Some("mis-keyed"));
    assert_eq!(row.get::<_, Option<String>>(4), None);
    // Idempotent: a second run touches nothing (all flags now non-NULL).
    let n = c.execute(backfill, &[]).await.unwrap();
    assert_eq!(n, 0, "backfill must be idempotent");
}
```

- [ ] **Step 3: Run the coverage tests**

Run: `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test -p cairn-node --test medication_dose 2>&1 | tail -30`
Expected: PASS (all). If any reveals a migration bug, fix `db/035…`/the trigger/floor and re-run (house rule 5 — fix, don't defer).

- [ ] **Step 4: fmt + commit**

```bash
cargo fmt -p cairn-node
git add crates/cairn-node/tests/medication_dose.rs
git commit -m "test(medication): cover per-field dose correction (strike floor, reason surface, wholesale supersede, backfill)"
```

---

### Task 5: CLI wiring (`--effective`, `--effective-precision`, `--note`, `--strike`)

**Files:**
- Modify: `crates/cairn-node/src/main.rs` (`MedicationCorrectDose` variant ~669-691; its handler ~1757-1796)

**Interfaces:**
- Consumes: `cairn_node::medication::CorrectDoseInput` (Task 2).

- [ ] **Step 1: Extend the `MedicationCorrectDose` clap variant**

Replace the variant (main.rs ~669-691) with:

```rust
    MedicationCorrectDose {
        /// The patient UUID the thread belongs to.
        patient: Uuid,
        /// The medication thread id.
        medication_id: Uuid,
        /// The dose event to correct. Defaults to the current dose point of the thread.
        #[arg(long)]
        target: Option<Uuid>,
        /// Set the corrected dose magnitude (with --dose-unit). Omit to leave the dose
        /// unchanged; use --strike dose to set it unknown.
        #[arg(long)]
        dose_amount: Option<String>,
        /// Set the corrected dose unit.
        #[arg(long)]
        dose_unit: Option<String>,
        /// Set the corrected effective date (e.g. 2024-01). Omit to leave it unchanged;
        /// use --strike effective to set it unknown.
        #[arg(long)]
        effective: Option<String>,
        /// Precision token for --effective (year|month|day|year-range).
        #[arg(long)]
        effective_precision: Option<String>,
        /// Set the corrected clinical reason for the dose (e.g. "titration"). Omit to
        /// leave it unchanged; use --strike reason to set it unknown.
        #[arg(long)]
        reason: Option<String>,
        /// Group(s) to set unknown: dose | effective | reason (repeatable).
        #[arg(long)]
        strike: Vec<String>,
        /// Why this correction was made (audit note, e.g. "mis-keyed the date").
        #[arg(long)]
        note: Option<String>,
        /// Optional provenance of the correction claim.
        #[arg(long)]
        info_source: Option<String>,
        #[command(flatten)]
        attest: AttestFlags,
    },
```

- [ ] **Step 2: Extend the handler**

Replace the handler arm (main.rs ~1757-1796) — add the new bindings to the destructure and the `CorrectDoseInput`:

```rust
        Cmd::MedicationCorrectDose {
            patient,
            medication_id,
            target,
            dose_amount,
            dose_unit,
            effective,
            effective_precision,
            reason,
            strike,
            note,
            info_source,
            attest,
        } => {
            let node_sk = load_signing_key(&cli.key, true)?;
            let node_kid = hex::encode(node_sk.verifying_key().to_bytes());
            let mut db = cairn_node::db::connect(&cli.conn).await?;
            let id = cairn_node::identity::load_local(&db).await?;
            ensure_registration_actor(&db, &node_kid).await?;
            let corrects =
                cairn_node::medication::resolve_correction_target(&db, medication_id, target)
                    .await?;
            let strike_refs: Vec<&str> = strike.iter().map(String::as_str).collect();
            let input = cairn_node::medication::CorrectDoseInput {
                dose_amount: dose_amount.as_deref(),
                dose_unit: dose_unit.as_deref(),
                effective: effective.as_deref(),
                effective_precision: effective_precision.as_deref(),
                reason: reason.as_deref(),
                strike: &strike_refs,
                note: note.as_deref(),
                info_source: info_source.as_deref(),
            };
            let resolved = resolve_attester(&db, &attest).await?;
            let params = attest_params(&resolved, &attest);
            let event_id = cairn_node::medication::correct_dose(
                &mut db,
                &node_sk,
                &node_kid,
                &id.node_id_hex,
                patient,
                medication_id,
                corrects,
                &input,
                params.as_ref(),
            )
            .await?;
            println!("dose correction recorded for thread {medication_id} (target {corrects}); event {event_id}");
        }
```

- [ ] **Step 3: Verify build + clippy**

Run: `cargo build -p cairn-node 2>&1 | tail -5`
Expected: builds clean.
Run: `cargo clippy -p cairn-node -- -D warnings 2>&1 | tail -5`
Expected: no warnings.

- [ ] **Step 4: fmt + commit**

```bash
cargo fmt -p cairn-node
git add crates/cairn-node/src/main.rs
git commit -m "feat(medication): CLI flags for dose effective/reason/strike/note correction"
```

---

### Task 6: ADR-0050 + spec bump to v0.51

**Files:**
- Create: `docs/spec/decisions/0050-dose-correction-per-field-patch.md`
- Modify: `docs/spec/index.md` (version 0.50 → 0.51; a one-line §3.15/§3.16 note)
- Modify: `docs/spec/decisions/README.md` (append the ADR-0050 index row, matching the existing table format)

**Interfaces:** none (docs).

- [ ] **Step 1: Write ADR-0050**

Create `docs/spec/decisions/0050-dose-correction-per-field-patch.md` following the format of `0049-commitment-based-sign-off-currency.md` (title, date 2026-07-15, status Accepted, Context / Decision / Consequences). Content must record:
- **Context:** slice-2's dose correction fixed the dose *value* only; a mis-keyed effective date (which drives current-dose winner selection) and clinical reason were uncorrectable; the correction `reason` column was stored but never surfaced.
- **Decision:** the correction is a **per-field patch** of three groups (dose / effective / reason) — omit = keep, set = override, `strike` = set-unknown; an explicit `strike` array keeps set-to-unknown first-class (principle 4) without the whole-point-restatement that would silently wipe untouched fields; the **corrected effective date participates in winner selection** (bitemporal repair); the correction rationale moves to a separate `note`, distinct from the point's clinical `reason`. `schema_version` bumps to `/2`.
- **Consequences:** existing verb reused (no new event type / envelope field / floor bypass); the overlay stays one-row-per-point HLC-wins **wholesale**, so a later correction of the same point supersedes rather than field-merges — **field-merge is a deliberate future refinement** needing per-field HLC tracking; a bare (no-op) correction is now rejected (slice-2's implicit "omit = strike dose" is gone, replaced by explicit `--strike dose`).

- [ ] **Step 2: Bump spec version + add the §3.15/§3.16 note**

In `docs/spec/index.md`, change `**Spec version:** 0.50` → `0.51`. Add one line to the §3.15/§3.16 medication prose: a dose correction patches the targeted point's dose / effective / reason (per-field; `strike` for unknown), and a corrected effective date participates in current-dose winner selection ([ADR-0050]).

- [ ] **Step 3: Append the ADR index row**

In `docs/spec/decisions/README.md`, add a table row for ADR-0050 in the same format as ADR-0049.

- [ ] **Step 4: Build docs (sanity) + commit**

Run: `uv run --with-requirements docs/requirements.txt -- mkdocs build 2>&1 | tail -5`
Expected: builds with no broken-link errors on the new ADR.

```bash
git add docs/spec/decisions/0050-dose-correction-per-field-patch.md docs/spec/index.md docs/spec/decisions/README.md
git commit -m "docs(adr): ADR-0050 dose correction per-field patch; spec v0.50 → v0.51"
```

---

## Post-implementation (session wrap — outside the task loop)

- Run the **full gate**: `cargo fmt --check` (both cargo trees), `cargo clippy --workspace -- -D warnings`, `CAIRN_TEST_PG=… cargo test --workspace`. All green before the PR.
- Update **HANDOVER.md** + **ROADMAP.md** (add Slice 34 / the new top block; prune to stay concise) per the `/nextsession` rules.
- Commit, push, open a PR to `main` describing the slice and linking the design/ADR; note the reshaped correction semantics + the wholesale-supersede boundary for reviewers.

## Self-Review

**Spec coverage:** §1 gap → Tasks 1–5; §2 payload → Task 1 (builder) + Task 3 (floor/trigger); §3 patch semantics → Tasks 1/3 + Task 4 tests; §4 reason/note → Tasks 1–3 + Task 4 `corrected_reason_surfaces…`; §5 projection (winner from corrected effective, reason surfaced, no widening) → Task 3 views + Task 4 tests; §6 migration mechanics (ALTER, backfill, floor, trigger, views) → Task 3 + Task 4 backfill test; §7 Rust+CLI → Tasks 1/2/5; §8 ADR + spec → Task 6; §9 testing → Tasks 1/3/4; §10 boundaries (out-of-scope started; wholesale supersede) → Task 4 `later_correction_supersedes…` + ADR consequences. All covered.

**Placeholder scan:** ADR-0050 body (Task 6 Step 1) is described by required-content bullets rather than verbatim prose — deliberate (it mirrors an existing ADR's structure); every code/SQL step is complete. No TBD/TODO.

**Type consistency:** `DoseCorrection`/`CorrectDoseInput` field names (`effective`, `effective_precision`, `reason`, `strike: &[&str]`, `note`, `info_source`) match across Tasks 1/2/5; `strike_refs: Vec<&str>` → `strike: &strike_refs` matches the `&'a [&'a str]` field; migration column names (`effective_value`, `effective_precision`, `note`, `dose_corrected`, `effective_corrected`, `reason_corrected`) match between Task 3 ALTER, trigger, views, and Task 4 assertions.
