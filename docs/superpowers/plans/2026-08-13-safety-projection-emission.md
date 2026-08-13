# Safety-projection emission (§5.9 slice B) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A sealed clinical event emits a de-identified safety signal — a coarse class and severity — whose granularity is set by the §5.9 sensitivity grade at authoring time, so decision-support can warn about a confidential episode without disclosing it.

**Architecture:** The seal boundary is the coarsening boundary. The *precise* class is looked up pre-seal on the coding node and travels **inside** the sealed payload (#294: carried, never re-derived). A *rung* chosen by the then-effective grade travels **in the clear** on the signed envelope, lands in a new additive `event_log.safety` column, and therefore survives a rung-3 crypto-shred for free. A total read model re-coarsens by the *current* grade, so a peer's finer rung cannot leak locally.

**Tech Stack:** Rust (`cairn-event` pure wire crate, `cairn-node` daemon/CLI), PostgreSQL 18 + PL/pgSQL (the unbypassable in-DB floor), `tokio-postgres`.

**Spec:** [`docs/superpowers/specs/2026-08-13-safety-projection-emission-design.md`](../specs/2026-08-13-safety-projection-emission-design.md)

**Issue:** [#375](https://github.com/cairn-ehr/cairn-ehr/issues/375), carrying [#294](https://github.com/cairn-ehr/cairn-ehr/issues/294)

## Global Constraints

- **Licence:** AGPL-3.0. Every dependency must be AGPL-3.0-compatible. This slice adds **no new dependency**.
- **TDD is mandatory.** Write the failing test, run it, watch it fail for the *stated* reason, then implement. No production line without a test that drove it.
- **Inline documentation for a junior reader** on every non-trivial function: *why it exists and how it fits*, not what the next line does.
- **Never hard-code cryptographic material in tests** (house rule 6, issue #146). Derive keys/nonces at runtime — `std::array::from_fn(|i| …)` or an existing helper. A literal byte array in a crypto context trips CodeQL `rust/hard-coded-cryptographic-value` and blocks the scan.
- **Additive-only wire evolution** (principle 11 / ADR-0012). The new `EventBody` field is **trailing** and `skip_serializing_if = "Option::is_none"`, so a `None` changes no existing content address.
- **UUIDs bind as text.** `cairn-node` does not enable tokio-postgres's `with-uuid-1`. Bind `&uuid.to_string()` and cast in SQL: `$1::text::uuid`.
- **Guard before connect.** Every DB-gated test calls `cairn_node::db::test_serial_guard(&base)` *before* `connect_and_load_schema`, and BINDS the returned guard (`let _guard = …`) — it is a `Client` holding a cluster-wide advisory lock, so an unbound one drops immediately and un-serializes the suite. The connection string comes from `common::cs()`, and a floor refusal is read through `common::db_msg(&e)` (tokio-postgres's `Display` renders only "db error").
- **Test connection strings:** `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test"` (plus `CAIRN_TEST_PG2`/`PG3` for convergence suites). Without them the DB-gated tests **self-skip and cargo counts them as passed**.
- ⚠️ **After Task 3 lands the `event_log` column, the local dev databases MUST be recreated** (`cairn_test`, `cairn_test2`, `cairn_test3` on :5532), or `born_sealed_schema.rs`'s positional `ROW` literals fail with *"invalid input syntax for type bigint"*. CI is immune (fresh DBs).
- **Two schema lists.** `db/049` goes in **both** `crates/cairn-node/src/db.rs` and `crates/cairn-sync/src/main.rs`.
- **Registry row counts do NOT move.** This slice registers no event type and no projection, so `twin_registry.rs`, `db/tests/034`, `projection_registry.rs` and `db/tests/039` are untouched. If any of them goes red, something was registered that should not have been.
- **The final gate is the full workspace**, not `-p cairn-node`: `scripts/run-db-gated-tests.sh`.

---

### Task 1: The pure wire shape — `cairn-event::safety`

The rung ladder, the coarsening function, and the `EventBody` field. Entirely pure: no database, no clock, no I/O, so it is exhaustively unit-testable without a cluster.

**Files:**
- Create: `crates/cairn-event/src/safety.rs`
- Modify: `crates/cairn-event/src/lib.rs` (add `pub mod safety;`; append the `safety` field to `EventBody`)
- Test: `crates/cairn-event/src/safety.rs` (`#[cfg(test)] mod tests`), `crates/cairn-event/src/lib.rs` (`#[cfg(test)] mod tests`)

**Interfaces:**
- Consumes: nothing (first task).
- Produces:
  - `pub enum SafetyRung { Precise, Kind, Existence }` with `pub fn as_str(self) -> &'static str` and `pub fn rank(self) -> i32`
  - `pub struct PreciseSafety<'a> { pub class: &'a str, pub severity: &'a str }`
  - `pub fn precise_safety_body(p: &PreciseSafety) -> serde_json::Value` — the **sealed** `payload.safety` object
  - `pub fn coarsen(p: &PreciseSafety, rung: SafetyRung) -> serde_json::Value` — the **clear** `EventBody.safety` object
  - `pub fn render_safety_twin(p: &PreciseSafety) -> String`
  - `EventBody.safety: Option<serde_json::Value>` (trailing field)

- [ ] **Step 1: Write the failing tests for the rung ladder and `coarsen`**

Create `crates/cairn-event/src/safety.rs` containing ONLY the test module plus the imports it needs, so the file compiles as a red test:

```rust
//! §5.9 safety projection — the wire shape of a de-identified safety signal (ADR-0006,
//! ADR-0059 decision 4, ADR-0063).
//!
//! # The two tiers, and why the seal boundary separates them
//!
//! A coded drug's interaction class is a property of the CODE — a drug-knowledge lookup.
//! So a reader cannot re-derive it without a drug database, and making the §5.9 safety
//! floor depend on one would defeat the floor (ADR-0059 decision 4 / #294). The class is
//! therefore computed PRE-SEAL on the coding node, which by construction had a coding
//! authority in hand, and it travels.
//!
//! But the precise class IS the disclosure for exactly the cases §5.9 exists for:
//! "Rh-sensitizing event" in the clear reads as "this patient had a termination".
//! So it travels in TWO tiers:
//!
//!   * `payload.safety` — the precise `{class, severity}`, sealed under the body's own DEK.
//!     A custody-holding node reads it without any drug database.
//!   * `EventBody.safety` — a RUNG chosen by the effective sensitivity grade at authoring
//!     time, in the clear. This is what a node without custody (sequestered, part C) or
//!     after a crypto-shred still sees, and it is the only coarsening that binds a peer's
//!     raw-SQL client.
use serde_json::{json, Value};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_rung_ladder_is_ordered_coarsest_last() {
        assert!(SafetyRung::Precise.rank() < SafetyRung::Kind.rank());
        assert!(SafetyRung::Kind.rank() < SafetyRung::Existence.rank());
        assert_eq!(SafetyRung::Precise.as_str(), "precise");
        assert_eq!(SafetyRung::Kind.as_str(), "kind");
        assert_eq!(SafetyRung::Existence.as_str(), "existence");
    }

    #[test]
    fn precise_carries_class_and_severity() {
        let p = PreciseSafety { class: "rh-sensitizing", severity: "high" };
        let v = coarsen(&p, SafetyRung::Precise);
        assert_eq!(v["rung"], "precise");
        assert_eq!(v["class"], "rh-sensitizing");
        assert_eq!(v["severity"], "high");
    }

    #[test]
    fn kind_drops_the_class_and_keeps_the_severity() {
        // "confidential medication, severity X" — the middle rung of §5.9's ladder. The
        // word "medication" is already in the clear on event_log.event_type, so the rung
        // carries only what is genuinely additional.
        let p = PreciseSafety { class: "rh-sensitizing", severity: "high" };
        let v = coarsen(&p, SafetyRung::Kind);
        assert_eq!(v["rung"], "kind");
        assert!(v.get("class").is_none(), "the class must not survive coarsening");
        assert_eq!(v["severity"], "high");
    }

    #[test]
    fn existence_carries_neither_but_still_exists() {
        // Coarseness varies; EXISTENCE never disappears (§5.9's safety-floor invariant).
        // This rung is the claim "there is a safety-relevant signal here and you are not
        // cleared to see what" — which is what makes break-glass a rational act.
        let p = PreciseSafety { class: "rh-sensitizing", severity: "high" };
        let v = coarsen(&p, SafetyRung::Existence);
        assert_eq!(v["rung"], "existence");
        assert!(v.get("class").is_none());
        assert!(v.get("severity").is_none());
        assert!(v.is_object(), "the signal still exists as an object");
    }

    #[test]
    fn the_sealed_body_always_carries_the_full_precision() {
        // payload.safety is under the DEK, so it is never coarsened: coarsening is what
        // the CLEAR field is for.
        let p = PreciseSafety { class: "antiretroviral-interaction", severity: "critical" };
        let v = precise_safety_body(&p);
        assert_eq!(v["class"], "antiretroviral-interaction");
        assert_eq!(v["severity"], "critical");
        assert!(v.get("rung").is_none(), "the sealed side carries no rung");
    }

    #[test]
    fn the_twin_reads_without_a_schema() {
        let p = PreciseSafety { class: "rh-sensitizing", severity: "high" };
        let t = render_safety_twin(&p);
        assert!(t.contains("rh-sensitizing"), "the class is the point: {t}");
        assert!(t.contains("high"), "the severity is actionable: {t}");
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p cairn-event --lib safety 2>&1 | tail -20`
Expected: FAIL to compile — `cannot find type SafetyRung in this scope` (and the same for `PreciseSafety`, `coarsen`, `precise_safety_body`, `render_safety_twin`). A compile failure naming the missing items IS the red state for this task.

- [ ] **Step 3: Implement the module**

Insert above the `#[cfg(test)]` block in `crates/cairn-event/src/safety.rs`:

```rust
/// How much of a safety signal is published in the clear. Ordered coarsest-last, mirroring
/// §5.9's ladder: *precise class → "confidential medication, severity X" → "confidential
/// content, break glass"*.
///
/// The rung is chosen at AUTHORING time from the effective sensitivity grade (db/049's
/// `cairn_safety_rung_for_rank`) and then frozen into the signed bytes. It cannot be
/// revised later: bytes on the wire cannot be un-published, which is exactly why the
/// choice has to bind here rather than at read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SafetyRung {
    /// The class itself. Safe only on a chart with no standing grade.
    Precise,
    /// Severity only. The event TYPE already says "medication" in the clear, so this rung
    /// deliberately adds no `kind` field — it would restate what the row already publishes.
    Kind,
    /// The signal exists; nothing about it is disclosed. Break glass to learn more.
    Existence,
}

impl SafetyRung {
    pub fn as_str(self) -> &'static str {
        match self {
            SafetyRung::Precise => "precise",
            SafetyRung::Kind => "kind",
            SafetyRung::Existence => "existence",
        }
    }

    /// Coarseness rank — higher is coarser. Gaps of 10 leave room to interpose a rung later
    /// without renumbering, the same discipline `cairn_sensitivity_rank` uses.
    pub fn rank(self) -> i32 {
        match self {
            SafetyRung::Precise => 0,
            SafetyRung::Kind => 10,
            SafetyRung::Existence => 20,
        }
    }
}

/// The full safety claim as the coding node established it. Borrowed `&str`s only, so it is
/// `Copy` and costs nothing to pass around.
#[derive(Debug, Clone, Copy)]
pub struct PreciseSafety<'a> {
    /// The coarse safety class — an interaction/allergy class, "rh-sensitizing", a
    /// contraindication flag. Open vocabulary: this crate never enumerates drug knowledge.
    pub class: &'a str,
    /// Open vocabulary; db/049 ranks the named ladder and treats anything else as MAX,
    /// because for a SAFETY signal "unknown" must mean "assume the worst".
    pub severity: &'a str,
}

/// The object that goes INSIDE the sealed payload. Never coarsened — the seal is what
/// protects it, and a custody-holder is entitled to the whole claim (#294: this is the
/// carried class a drugref-less reader depends on).
pub fn precise_safety_body(p: &PreciseSafety) -> Value {
    json!({ "class": p.class, "severity": p.severity })
}

/// The object that goes in the CLEAR on the signed envelope, cut down to `rung`.
///
/// Total and exhaustive over the ladder: adding a rung to `SafetyRung` forces a decision
/// here, which is the point — a rung with no explicit field policy would default to
/// disclosing whatever the previous arm disclosed.
pub fn coarsen(p: &PreciseSafety, rung: SafetyRung) -> Value {
    match rung {
        SafetyRung::Precise => {
            json!({ "rung": rung.as_str(), "class": p.class, "severity": p.severity })
        }
        // Fields are OMITTED, never written as null: an explicit null is an author
        // asserting something about the class, and absence is the honest "withheld".
        SafetyRung::Kind => json!({ "rung": rung.as_str(), "severity": p.severity }),
        SafetyRung::Existence => json!({ "rung": rung.as_str() }),
    }
}

/// The §3.13 legibility twin fragment for the sealed side — a reader holding no drug
/// database and no schema must still be able to read what was claimed (principle 11).
pub fn render_safety_twin(p: &PreciseSafety) -> String {
    format!("safety: {} (severity {})", p.class, p.severity)
}
```

Add the module to `crates/cairn-event/src/lib.rs` in the existing alphabetical `pub mod` block, between `pub mod registration;` and `pub mod schema_generation;`:

```rust
pub mod safety;
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p cairn-event --lib safety 2>&1 | tail -10`
Expected: PASS, 6 tests.

- [ ] **Step 5: Write the failing test for the additive `EventBody` field**

Append to the `#[cfg(test)] mod tests` block at the bottom of `crates/cairn-event/src/lib.rs`:

```rust
#[test]
fn an_absent_safety_field_changes_no_existing_content_address() {
    // Principle 11 / ADR-0012: a field added later must not re-encode existing signed
    // bytes. `skip_serializing_if` means a None emits NO CBOR key at all, so the encoding
    // of every event that carries no safety signal is byte-identical to the pre-field one.
    // Encoded from a GENUINE pre-field struct so the test cannot pass by accident — the
    // same idiom the plaintext_twin additive test above uses.
    #[derive(serde::Serialize)]
    struct PreSafetyBody<'a> {
        event_id: &'a str,
        patient_id: &'a str,
        event_type: &'a str,
        schema_version: &'a str,
        hlc: &'a Hlc,
        t_effective: Option<&'a str>,
        signer_key_id: &'a str,
        contributors: &'a serde_json::Value,
        payload: &'a serde_json::Value,
        attachments: &'a Vec<Attachment>,
        clock_grade: ClockGrade,
    }

    let body = EventBody {
        event_id: "01930000-0000-7000-8000-000000000001".into(),
        patient_id: "01930000-0000-7000-8000-000000000002".into(),
        event_type: "note.added".into(),
        schema_version: "note/1".into(),
        hlc: Hlc { wall: 1, counter: 0, node_origin: "n1".into() },
        t_effective: None,
        signer_key_id: "kid".into(),
        contributors: serde_json::json!([]),
        payload: serde_json::json!({"text": "hello"}),
        attachments: vec![],
        plaintext_twin: None,
        clock_grade: ClockGrade::SelfAsserted,
        safety: None,
    };
    let pre = PreSafetyBody {
        event_id: &body.event_id,
        patient_id: &body.patient_id,
        event_type: &body.event_type,
        schema_version: &body.schema_version,
        hlc: &body.hlc,
        t_effective: None,
        signer_key_id: &body.signer_key_id,
        contributors: &body.contributors,
        payload: &body.payload,
        attachments: &body.attachments,
        clock_grade: body.clock_grade,
    };
    let mut pre_bytes = Vec::new();
    ciborium::into_writer(&pre, &mut pre_bytes).expect("pre-field body encodes");
    assert_eq!(
        canonical_cbor(&body).expect("body encodes"),
        pre_bytes,
        "adding `safety` must not change the bytes of an event that carries none"
    );
}

#[test]
fn a_safety_signal_survives_sign_and_verify_unchanged() {
    let (sk, kid) = generate_key().expect("keypair");
    let mut body = EventBody {
        event_id: "01930000-0000-7000-8000-000000000003".into(),
        patient_id: "01930000-0000-7000-8000-000000000004".into(),
        event_type: "clinical.medication.asserted".into(),
        schema_version: "clinical.medication/1".into(),
        hlc: Hlc { wall: 2, counter: 0, node_origin: "n1".into() },
        t_effective: None,
        signer_key_id: kid.clone(),
        contributors: serde_json::json!([]),
        payload: serde_json::json!({"medication_id": "x"}),
        attachments: vec![],
        plaintext_twin: Some("t".into()),
        clock_grade: ClockGrade::SelfAsserted,
        safety: None,
    };
    body.safety = Some(crate::safety::coarsen(
        &crate::safety::PreciseSafety { class: "rh-sensitizing", severity: "high" },
        crate::safety::SafetyRung::Kind,
    ));
    let signed = sign(&body, &sk).expect("signs");
    let decoded = verify_with(&signed.signed_bytes, &sk.verifying_key()).expect("verifies");
    assert_eq!(decoded.safety, body.safety, "the signal is inside the signature");
    assert_eq!(decoded.safety.as_ref().unwrap()["rung"], "kind");
}
```

- [ ] **Step 6: Run to verify they fail**

Run: `cargo test -p cairn-event --lib safety_ 2>&1 | tail -20`
Expected: FAIL to compile — `struct EventBody has no field named safety`.

- [ ] **Step 7: Add the trailing field**

In `crates/cairn-event/src/lib.rs`, append to `EventBody` **after** `clock_grade` (trailing placement is load-bearing — field order IS the canonical encoding order):

```rust
    /// The §5.9 de-identified safety signal, in the CLEAR (ADR-0063). Carries a `rung`
    /// chosen at authoring time from the effective sensitivity grade, plus whatever that
    /// rung licenses — see [`safety::coarsen`]. The PRECISE claim lives inside the sealed
    /// payload; this is the tier a node without custody, or one reading a crypto-shredded
    /// event, still sees.
    ///
    /// `skip_serializing_if` ⇒ a None is omitted from the wire, so adding this field never
    /// changes an existing event's bytes/content-address (additive-only, principle 11 /
    /// ADR-0012). Appended TRAILING for the same reason (the ADR-0058 `clock_grade`
    /// precedent).
    ///
    /// A `serde_json::Value` rather than a typed struct on purpose: the vocabulary is open
    /// (principle 11), a future peer's rung must decode rather than fail, and the read model
    /// that interprets it is the in-DB floor, not this crate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub safety: Option<serde_json::Value>,
```

- [ ] **Step 8: Fix every `EventBody { … }` literal the new field broke**

Run: `cargo build --workspace 2>&1 | grep -E "^error|missing field" | head -40`
Add `safety: None,` to each reported literal. Expect hits across `cairn-event` tests, `cairn-node/src/**` verb builders, `cairn-node/tests/**`, and `cairn-sync`. Repeat until the build is clean.

- [ ] **Step 9: Run the tests to verify they pass**

Run: `cargo test -p cairn-event 2>&1 | tail -10`
Expected: PASS, no failures.

- [ ] **Step 10: Commit**

```bash
git add crates/cairn-event/src/safety.rs crates/cairn-event/src/lib.rs crates/
git commit -m "feat(#375): the safety signal's wire shape — two tiers, one seal boundary

The precise {class, severity} goes inside the sealed payload (#294: carried,
never re-derived); a rung chosen by the sensitivity grade goes in the clear on
the envelope. coarsen() is exhaustive over the ladder so a future rung cannot
inherit the previous arm's disclosure by default.

EventBody.safety is trailing + skip_serializing_if, so a None is byte-identical
to the pre-field encoding — pinned by a test built from a genuine pre-field
struct rather than from the new one.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

### Task 2: `db/049` — the ladders, the floor check, the empty class map

The in-DB half, standing alone: pure functions plus one empty table. No door wiring yet, so this task's tests exercise the functions directly and can go green before anything else changes.

**Files:**
- Create: `db/049_safety_projection.sql`
- Modify: `crates/cairn-event/src/schema_generation.rs` (48 → 49), `crates/cairn-node/src/db.rs` (schema list), `crates/cairn-sync/src/main.rs` (schema list)
- Test: `crates/cairn-node/tests/safety_ladder.rs`

**Interfaces:**
- Consumes: `cairn_sensitivity_rank(text) → int` (db/048).
- Produces (SQL):
  - `cairn_safety_severity_rank(text) → int`
  - `cairn_safety_rung_rank(text) → int`
  - `cairn_safety_rung_for_rank(int) → text`
  - `cairn_check_safety_signal(jsonb) → void`
  - `safety_class_map(system, code, class, severity, note)` — ships empty
  - `cairn_safety_class_candidate(jsonb) → TABLE(class text, severity text)`

- [ ] **Step 1: Write the failing test**

Create `crates/cairn-node/tests/safety_ladder.rs`:

```rust
//! §5.9 part B (ADR-0063) — the two rank ladders, the rung map, the structural floor check,
//! and the empty class map. Pure in-DB functions: no doors, no events, no signing.
mod common;
use common::{cs, db_msg};

/// Open a serialized connection, or `None` when the suite should self-skip.
///
/// Returns the GUARD alongside the client: the guard is a second `Client` holding a
/// cluster-wide advisory lock, so dropping it inside this helper would un-serialize every
/// caller. Callers bind it as `let Some((_g, c)) = connect().await else { return };`.
async fn connect() -> Option<(tokio_postgres::Client, tokio_postgres::Client)> {
    let base = cs()?;
    let guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base).await.unwrap();
    Some((guard, c))
}

#[tokio::test]
async fn an_unrecognised_severity_ranks_max() {
    let Some((_g, c)) = connect().await else { return };
    let row = c
        .query_one(
            "SELECT cairn_safety_severity_rank('none'), cairn_safety_severity_rank('critical'),
                    cairn_safety_severity_rank('severity:novel'), cairn_safety_severity_rank(NULL)",
            &[],
        )
        .await
        .expect("severity ranks");
    let (none, critical, novel, null): (i32, i32, i32, i32) =
        (row.get(0), row.get(1), row.get(2), row.get(3));
    assert_eq!(none, 0, "'none' is the floor");
    assert!(critical > none, "the ladder is ordered");
    // For a SAFETY signal, unknown must mean "assume the worst" — the opposite of muting a
    // warning nobody here can interpret.
    assert_eq!(novel, i32::MAX, "an unrecognised severity ranks MAX");
    assert_eq!(null, i32::MAX, "NULL lands on the safe side");
}

#[tokio::test]
async fn an_unrecognised_rung_ranks_coarsest() {
    let Some((_g, c)) = connect().await else { return };
    let row = c
        .query_one(
            "SELECT cairn_safety_rung_rank('precise'), cairn_safety_rung_rank('kind'),
                    cairn_safety_rung_rank('existence'), cairn_safety_rung_rank('rung:novel'),
                    cairn_safety_rung_rank(NULL)",
            &[],
        )
        .await
        .expect("rung ranks");
    let (p, k, e, novel, null): (i32, i32, i32, i32, i32) =
        (row.get(0), row.get(1), row.get(2), row.get(3), row.get(4));
    assert!(p < k && k < e, "coarsest last");
    assert_eq!(novel, i32::MAX, "an unrecognised rung is treated as coarsest");
    assert_eq!(null, i32::MAX);
}

#[tokio::test]
async fn the_rung_map_is_monotone_non_decreasing_in_grade_rank() {
    let Some((_g, c)) = connect().await else { return };
    // A higher sensitivity grade may never disclose MORE. Checked across the whole ladder
    // including the MAX sentinel, so a future grade interposed at any rank inherits a rung
    // no finer than its neighbour's.
    let rows = c
        .query(
            "SELECT r, cairn_safety_rung_rank(cairn_safety_rung_for_rank(r))
             FROM unnest(ARRAY[0, 5, 10, 15, 20, 30, 2147483647]) AS r ORDER BY r",
            &[],
        )
        .await
        .expect("rung map");
    let ranks: Vec<i32> = rows.iter().map(|r| r.get(1)).collect();
    assert!(
        ranks.windows(2).all(|w| w[0] <= w[1]),
        "the rung map must be monotone non-decreasing in grade rank: {ranks:?}"
    );

    let named = c
        .query_one(
            "SELECT cairn_safety_rung_for_rank(cairn_sensitivity_rank('routine')),
                    cairn_safety_rung_for_rank(cairn_sensitivity_rank('sensitive')),
                    cairn_safety_rung_for_rank(cairn_sensitivity_rank('restricted')),
                    cairn_safety_rung_for_rank(cairn_sensitivity_rank('sequestered')),
                    cairn_safety_rung_for_rank(cairn_sensitivity_rank('grade:protected-witness'))",
            &[],
        )
        .await
        .expect("named grades");
    assert_eq!(named.get::<_, String>(0), "precise", "no grade discloses fully");
    assert_eq!(named.get::<_, String>(1), "kind");
    assert_eq!(named.get::<_, String>(2), "existence");
    assert_eq!(named.get::<_, String>(3), "existence");
    assert_eq!(
        named.get::<_, String>(4),
        "existence",
        "an unrecognised grade ranks MAX (ADR-0062 decision 2), hence coarsest here"
    );
}

#[tokio::test]
async fn the_floor_check_admits_absence_and_every_well_formed_rung() {
    let Some((_g, c)) = connect().await else { return };
    for body in [
        r#"{}"#,
        r#"{"safety": {"rung": "precise", "class": "rh-sensitizing", "severity": "high"}}"#,
        r#"{"safety": {"rung": "kind", "severity": "high"}}"#,
        r#"{"safety": {"rung": "existence"}}"#,
        // A future peer's rung is ADMITTED, not refused — the floor gates effect, not
        // presence (ADR-0056). The read model treats it as coarsest.
        r#"{"safety": {"rung": "rung:novel"}}"#,
    ] {
        c.execute("SELECT cairn_check_safety_signal($1::jsonb)", &[&body])
            .await
            .unwrap_or_else(|e| panic!("must admit {body}: {}", db_msg(&e)));
    }
}

#[tokio::test]
async fn the_floor_check_refuses_a_class_the_rung_does_not_license() {
    let Some((_g, c)) = connect().await else { return };
    // A body claiming "existence" while carrying the class publishes what it asserts is
    // concealed. Refused where it is AUTHORED — the only place a door can help.
    let e = c
        .execute(
            "SELECT cairn_check_safety_signal($1::jsonb)",
            &[&r#"{"safety": {"rung": "existence", "class": "rh-sensitizing"}}"#],
        )
        .await
        .expect_err("a class at a coarser rung must be refused");
    let msg = db_msg(&e);
    assert!(msg.contains("class"), "the message names the offending key: {msg}");
}

#[tokio::test]
async fn the_floor_check_refuses_a_missing_rung_and_a_precise_without_a_class() {
    let Some((_g, c)) = connect().await else { return };
    for (body, needle) in [
        (r#"{"safety": {"severity": "high"}}"#, "rung"),
        (r#"{"safety": {"rung": ""}}"#, "rung"),
        (r#"{"safety": {"rung": "precise", "severity": "high"}}"#, "class"),
        (r#"{"safety": {"rung": "precise", "class": "  ", "severity": "high"}}"#, "class"),
        (r#"{"safety": "not-an-object"}"#, "object"),
    ] {
        let e = c
            .execute("SELECT cairn_check_safety_signal($1::jsonb)", &[&body])
            .await
            .expect_err(&format!("the floor must refuse {body}"));
        let msg = db_msg(&e);
        assert!(msg.contains(needle), "message must name `{needle}`: {msg}");
    }
}

#[tokio::test]
async fn the_class_map_ships_empty() {
    let Some((_g, c)) = connect().await else { return };
    // Cairn ships the LOOKUP, never the drug knowledge. A seeded row would be an
    // un-reviewable clinical policy choice smuggled in as infrastructure (principle 9) —
    // the same discipline sensitivity_category_map keeps.
    let n: i64 = c
        .query_one("SELECT count(*) FROM safety_class_map", &[])
        .await
        .expect("map exists")
        .get(0);
    assert_eq!(n, 0, "safety_class_map must ship empty");

    // And the candidate lookup is honest about that: no row, no class.
    let hit: i64 = c
        .query_one(
            "SELECT count(*) FROM cairn_safety_class_candidate(
                 '{\"system\":\"drugref-moiety\",\"code\":\"whatever\"}'::jsonb)",
            &[],
        )
        .await
        .expect("candidate lookup runs")
        .get(0);
    assert_eq!(hit, 0, "an empty map yields no candidate");
}
```

Fix the sixth test's shape before running — it must assert the refusal, not swallow it:

```rust
#[tokio::test]
async fn the_floor_check_refuses_a_missing_rung_and_a_precise_without_a_class() {
    let Some((_g, c)) = connect().await else { return };
    for (body, needle) in [
        (r#"{"safety": {"severity": "high"}}"#, "rung"),
        (r#"{"safety": {"rung": ""}}"#, "rung"),
        (r#"{"safety": {"rung": "precise", "severity": "high"}}"#, "class"),
        (r#"{"safety": {"rung": "precise", "class": "  ", "severity": "high"}}"#, "class"),
        (r#"{"safety": "not-an-object"}"#, "object"),
    ] {
        let e = c
            .execute("SELECT cairn_check_safety_signal($1::jsonb)", &[&body])
            .await
            .expect_err(&format!("must refuse {body}"));
        let msg = db_msg(&e);
        assert!(msg.contains(needle), "message must name `{needle}`: {msg}");
    }
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p cairn-node --test safety_ladder 2>&1 | tail -20`
Expected: FAIL — `function cairn_safety_severity_rank(unknown) does not exist`.

If instead every test is reported as passing with `0 filtered out` and no output, `CAIRN_TEST_PG` is unset and the tests self-skipped. Export it and re-run — a self-skip is NOT a red state.

- [ ] **Step 3: Write the migration**

Create `db/049_safety_projection.sql`:

```sql
-- 049_safety_projection.sql — §5.9 part B (ADR-0063): the de-identified safety signal.
--
-- WHY: a sealed clinical body still owes a future clinician a warning. A sealed pregnancy
-- termination implies Rhesus sensitisation the next antenatal clinician must act on. This
-- file carries the ladders that decide HOW MUCH of that warning is published, the local-door
-- floor check on its shape, and the deployment-populated (EMPTY-shipped) class lookup that
-- the AUTHORING node consults pre-seal.
--
-- WHAT IS NOT HERE: the event_log column and the door wiring (db/005, db/020), and the read
-- model (this file's sections 5-7 add it). Nothing in this file withholds any content:
-- §5.9 part B emits and coarsens a SIGNAL. Enforcement is part C (#376).

BEGIN;

-- ---------------------------------------------------------------------------
-- 1. The severity ladder.
--
--    !!! THE `ELSE` IS THE DECISION, NOT AN OVERSIGHT !!!
--    An unrecognised severity ranks MAX — "assume the worst" — and that is the SAFE
--    direction for a safety signal, exactly as it is for a sensitivity grade (ADR-0062
--    decision 2). It deliberately differs from cairn_clock_grade_rank's ELSE 0 (db/040),
--    where an unknown value withholds REJECT power and 0 is safe. Here 0 would mute a
--    warning this node cannot interpret, on precisely the events most likely to matter.
--    "Fixing" this into consistency with db/040 reopens that hole.
--
--    Open vocabulary, no CHECK domain: a future peer's severity is admitted verbatim
--    (principle 11, additive-only). Gaps of 10 leave room to interpose terms later.
-- ---------------------------------------------------------------------------
CREATE OR REPLACE FUNCTION cairn_safety_severity_rank(s text)
RETURNS int LANGUAGE sql IMMUTABLE AS $$
    SELECT CASE s
        WHEN 'none'     THEN 0
        WHEN 'low'      THEN 10
        WHEN 'moderate' THEN 20
        WHEN 'high'     THEN 30
        WHEN 'critical' THEN 40
        ELSE 2147483647            -- unknown ⇒ most severe. See the comment above.
    END;
$$;

-- ---------------------------------------------------------------------------
-- 2. The disclosure ladder. Higher rank = COARSER = less disclosed.
--
--    Same ELSE discipline, pointed the other way and for the same reason: a rung this
--    node does not recognise must be treated as the COARSEST, never as "show everything".
-- ---------------------------------------------------------------------------
CREATE OR REPLACE FUNCTION cairn_safety_rung_rank(r text)
RETURNS int LANGUAGE sql IMMUTABLE AS $$
    SELECT CASE r
        WHEN 'precise'   THEN 0
        WHEN 'kind'      THEN 10
        WHEN 'existence' THEN 20
        ELSE 2147483647            -- unknown ⇒ disclose nothing.
    END;
$$;

-- ---------------------------------------------------------------------------
-- 3. Sensitivity rank -> disclosure rung. §5.9 calls this ladder "policy-configured";
--    this slice ships the monotone default and files the deployment override.
--
--    KEYED ON THE RANK, NOT THE GRADE STRING, on purpose: ADR-0062's grade vocabulary is
--    open and its unknown-ranks-MAX inversion lives in cairn_sensitivity_rank. Keying on
--    the rank inherits both for free — a future grade interposed at rank 15 lands on
--    'kind', one at 25 lands on 'existence', and an unrecognised one lands on 'existence'
--    without anyone remembering to add it. Safe-default-by-omission, the same discipline
--    ADR-0062 decisions 2 and 10 use.
--
--    MONOTONE NON-DECREASING BY CONSTRUCTION: a higher grade can never disclose more.
-- ---------------------------------------------------------------------------
CREATE OR REPLACE FUNCTION cairn_safety_rung_for_rank(p_rank int)
RETURNS text LANGUAGE sql IMMUTABLE AS $$
    SELECT CASE
        WHEN p_rank IS NULL  THEN 'existence'   -- no answer ⇒ disclose nothing
        WHEN p_rank <= 0     THEN 'precise'     -- routine, or no standing assertion at all
        WHEN p_rank <= 10    THEN 'kind'        -- sensitive
        ELSE                      'existence'   -- restricted, sequestered, unrecognised
    END;
$$;

-- ---------------------------------------------------------------------------
-- 4. The structural floor on the CLEAR safety field.
--
--    CALLED FROM db/005 (submit_event) ONLY — DELIBERATELY NOT FROM db/020.
--
--    ADR-0062 E2 says a STRUCTURAL check (the shape of the claim) is safe at both doors
--    while a CEREMONY check (who authored it) must stay local. Read naively this check is
--    structural, so it would belong at both. THAT READING IS WRONG HERE, AND THE REASON IS
--    BLAST RADIUS.
--
--    A sensitivity assertion IS an event: refusing a malformed one drops one assertion.
--    The safety signal is a FIELD ON A CLINICAL EVENT: refusing it at the apply door drops
--    the medication assertion it rides on off this node's chart. A defect in a
--    de-identified ADVISORY signal would then destroy CLINICAL CONTENT — ADR-0060's "a
--    defect on one line never invalidates another", and its harder corollary: the system
--    may fail to record an order, but it may never cancel one.
--
--    So this follows the clock_grade precedent (db/040): constrained where MINTED, read
--    permissively where it ARRIVES. A peer that sent a self-contradictory signal has
--    already published those bytes; refusing at apply un-discloses nothing, forks the event
--    set (#342), and costs clinical content as well. Section 7's read model is total
--    instead, and never surfaces a class the rung forbids.
-- ---------------------------------------------------------------------------
CREATE OR REPLACE FUNCTION cairn_check_safety_signal(b jsonb) RETURNS void
LANGUAGE plpgsql IMMUTABLE AS $$
DECLARE
    s    jsonb := b -> 'safety';
    rung text;
BEGIN
    IF s IS NULL OR jsonb_typeof(s) = 'null' THEN
        RETURN;   -- absent: the overwhelmingly common case, and always legal.
    END IF;
    -- jsonb_typeof is checked POSITIVELY (= 'object'), never as a NOT-something: the
    -- fail-OPEN pattern issue #346 catalogues comes from comparing a NULL typeof.
    IF jsonb_typeof(s) <> 'object' THEN
        RAISE EXCEPTION 'safety: the signal must be a JSON object, got %', jsonb_typeof(s);
    END IF;

    rung := s ->> 'rung';
    IF COALESCE(rung, '') = '' THEN
        RAISE EXCEPTION 'safety: the signal must carry a non-empty rung (ADR-0063)';
    END IF;

    IF rung = 'precise' THEN
        IF COALESCE(btrim(s ->> 'class'), '') = '' THEN
            RAISE EXCEPTION 'safety: rung "precise" must carry a non-empty class — a precise rung with nothing precise in it is a claim about nothing (ADR-0063)';
        END IF;
    ELSIF s ? 'class' THEN
        -- The disclosure guard. A body claiming {"rung":"existence","class":"..."}
        -- publishes the class while asserting it is concealed, and a reader trusting the
        -- rung would render it as concealed while the class sat in the row.
        RAISE EXCEPTION 'safety: rung "%" must not carry a class — it would publish exactly what the rung says is withheld (ADR-0063)', rung;
    END IF;

    IF s ? 'severity' AND COALESCE(btrim(s ->> 'severity'), '') = '' THEN
        RAISE EXCEPTION 'safety: severity, when present, must be a non-empty string';
    END IF;
END;
$$;

-- ---------------------------------------------------------------------------
-- 5. The class lookup — the AUTHORING node's drug-knowledge seam.
--
--    Ships EMPTY, and the SQL mirror asserts it stays empty. Cairn ships the lookup
--    MECHANISM, never the drug knowledge: a seeded row would be an un-reviewable clinical
--    policy choice smuggled into "infrastructure" (principle 9) — the same discipline
--    db/048's sensitivity_category_map keeps. This table is also the seam the future
--    drugref slice populates.
--
--    KEYED ON THE PAIR (system, code), NEVER A BARE CODE: once drugref-clinical-drug
--    exists beside drugref-moiety, a bare-code key would collide across composition-tree
--    levels (ADR-0059 decision 5's argument, unchanged).
-- ---------------------------------------------------------------------------
CREATE TABLE IF NOT EXISTS safety_class_map (
    system   TEXT NOT NULL,
    code     TEXT NOT NULL,
    class    TEXT NOT NULL,
    severity TEXT NOT NULL,
    note     TEXT NOT NULL DEFAULT '',
    PRIMARY KEY (system, code)
);
GRANT SELECT ON safety_class_map TO cairn_agent;
REVOKE INSERT, UPDATE, DELETE ON safety_class_map FROM PUBLIC;

--    A PURE lookup that yields a CANDIDATE. It authors nothing and is called ONLY
--    pre-seal, by the node that is writing the event — which by construction had a coding
--    authority in hand. A READER must never call it: a reader that re-derives makes the
--    §5.9 floor depend on holding drugref after all, which is precisely the failure
--    ADR-0059 decision 4 / #294 exist to prevent.
CREATE OR REPLACE FUNCTION cairn_safety_class_candidate(p_coding jsonb)
RETURNS TABLE (class text, severity text)
LANGUAGE sql STABLE AS $$
    SELECT m.class, m.severity
    FROM safety_class_map m
    WHERE m.system = (p_coding ->> 'system')
      AND m.code   = (p_coding ->> 'code');
$$;

-- Postgres grants EXECUTE to PUBLIC by default, and every role is a member of PUBLIC, so
-- an un-REVOKEd function is directly callable by a below-the-floor adversary with raw SQL
-- (the db/037 note, and issue #382's finding about the cairn_check_* family).
REVOKE EXECUTE ON FUNCTION cairn_check_safety_signal(jsonb) FROM PUBLIC;
REVOKE EXECUTE ON FUNCTION cairn_safety_class_candidate(jsonb) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION cairn_safety_class_candidate(jsonb) TO cairn_agent;

COMMIT;
```

- [ ] **Step 4: Register the migration in both schema lists and bump the generation**

`crates/cairn-event/src/schema_generation.rs` — update the doc comment and the constant:

```rust
/// The numeric prefix of the newest migration in `db/`
/// (`db/049_safety_projection.sql` → 49).
///
/// Bump this in the same commit that adds a `db/*.sql` file; the guard test enforces it.
pub const SCHEMA_GENERATION: i32 = 49;
```

`crates/cairn-node/src/db.rs` — append after the `048_sensitivity_stream` entry:

```rust
        "049_safety_projection",
        include_str!("../../../db/049_safety_projection.sql"),
```

`crates/cairn-sync/src/main.rs` — append after its `048_sensitivity_stream` entry, with the same two lines. **Both lists**: the apply door writes the `safety` column, so a subset node missing this file would fail on every clinical apply.

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p cairn-node --test safety_ladder 2>&1 | tail -10`
Expected: PASS, 6 tests.

Run: `cargo test -p cairn-event --test schema_generation 2>&1 | tail -5`
Expected: PASS — the guard test derives 49 from `db/` and matches the constant.

- [ ] **Step 6: Commit**

```bash
git add db/049_safety_projection.sql crates/cairn-event/src/schema_generation.rs \
        crates/cairn-node/src/db.rs crates/cairn-sync/src/main.rs \
        crates/cairn-node/tests/safety_ladder.rs
git commit -m "feat(#375): db/049 — the two safety ladders, the floor check, the empty class map

Both ELSE arms rank unknown as the SAFE extreme: an unrecognised severity is
most-severe, an unrecognised rung discloses nothing. Both invert db/040's ELSE 0
for the ADR-0062 reason, and both say so in a shouting comment, because tidying
them into consistency is the cleanup most likely to be attempted in good faith.

cairn_safety_rung_for_rank keys on the sensitivity RANK, not the grade string,
so ADR-0062's open vocabulary and its unknown-ranks-MAX inversion are inherited
rather than re-spelled.

safety_class_map ships empty: Cairn ships the lookup, never the drug knowledge.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

### Task 3: The column and the asymmetric door wiring

`event_log.safety` plus the local-door check. This is the task that lands the schema change, so it is also where the dev databases get recreated.

**Files:**
- Modify: `db/049_safety_projection.sql` (add the `ALTER TABLE`), `db/005_submit.sql` (check + column), `db/020_apply_remote_event.sql` (column only)
- Test: `crates/cairn-node/tests/safety_doors.rs`

**Interfaces:**
- Consumes: `cairn_check_safety_signal(jsonb)` (Task 2), `EventBody.safety` (Task 1).
- Produces: `event_log.safety JSONB` — a faithful derived view of `b -> 'safety'` from the signed bytes, never sanitized on the way in.

- [ ] **Step 1: Write the failing test**

Create `crates/cairn-node/tests/safety_doors.rs`:

```rust
//! §5.9 part B (ADR-0063) — the DELIBERATE door asymmetry.
//!
//! The local door refuses a self-contradictory safety signal; the remote door admits it AND
//! the clinical content it rides on. The second half is the one that matters: a defect in a
//! de-identified advisory field must never cancel a clinical event (ADR-0060).
mod common;
use common::{cs, db_msg, setup};
use uuid::Uuid;

/// Build a signed `note.added` body carrying `safety` verbatim, so a test can put a shape on
/// the wire that no honest authoring path would produce.
///
/// `note.added` rather than a medication verb on purpose: it is unsealed, so the test needs
/// no DEK and exercises the ENVELOPE-level check without the seal path in the way.
fn body_with_safety(
    patient: Uuid,
    kid: &str,
    wall: i64,
    safety: Option<serde_json::Value>,
) -> cairn_event::EventBody {
    cairn_event::EventBody {
        event_id: Uuid::now_v7().to_string(),
        patient_id: patient.to_string(),
        event_type: "note.added".into(),
        schema_version: "note/1".into(),
        hlc: cairn_event::Hlc { wall, counter: 0, node_origin: "n1".into() },
        t_effective: None,
        signer_key_id: kid.into(),
        contributors: serde_json::json!([{"actor_id": kid, "role": "recorded"}]),
        payload: serde_json::json!({"text": "a note"}),
        attachments: vec![],
        plaintext_twin: Some("a note".into()),
        clock_grade: cairn_event::ClockGrade::SelfAsserted,
        safety,
    }
}

#[tokio::test]
async fn the_local_door_refuses_a_class_the_rung_does_not_license() {
    let Some(base) = cs() else { return };
    // The guard is a Client holding a cluster-wide advisory lock: it must stay BOUND for
    // the whole test, and it is taken BEFORE connect_and_load_schema (every existing suite
    // does this in execution order).
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &[]).await;
    let patient = Uuid::now_v7();

    let body = body_with_safety(
        patient,
        &kid,
        1,
        Some(serde_json::json!({"rung": "existence", "class": "rh-sensitizing"})),
    );
    let signed = cairn_event::sign(&body, &sk).expect("signs");
    let e = c
        .execute("SELECT submit_event($1, NULL, NULL, NULL)", &[&signed.signed_bytes])
        .await
        .expect_err("the local door must refuse a self-contradictory signal");
    let msg = db_msg(&e);
    assert!(msg.contains("class"), "the refusal names the offending key: {msg}");
}

#[tokio::test]
async fn the_remote_door_admits_the_same_body_and_keeps_the_clinical_content() {
    let Some(base) = cs() else { return };
    // The guard is a Client holding a cluster-wide advisory lock: it must stay BOUND for
    // the whole test, and it is taken BEFORE connect_and_load_schema (every existing suite
    // does this in execution order).
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &[]).await;
    let patient = Uuid::now_v7();

    let body = body_with_safety(
        patient,
        &kid,
        2,
        Some(serde_json::json!({"rung": "existence", "class": "rh-sensitizing"})),
    );
    let event_id: Uuid = body.event_id.parse().expect("uuid");
    let signed = cairn_event::sign(&body, &sk).expect("signs");
    c.execute("SELECT apply_remote_event($1)", &[&signed.signed_bytes])
        .await
        .expect("the remote door must ADMIT it — refusing forks the event set (#342)");

    // The half that actually matters: the clinical content landed.
    let n: i64 = c
        .query_one(
            "SELECT count(*) FROM event_log WHERE event_id = $1::text::uuid",
            &[&event_id.to_string()],
        )
        .await
        .expect("query")
        .get(0);
    assert_eq!(
        n, 1,
        "a defect in a de-identified advisory field must never cancel clinical content (ADR-0060)"
    );

    // And the column is a FAITHFUL derived view — never sanitized on the way in, which
    // would make it disagree with signed_bytes. Section 7's read model is what refuses to
    // ACT on the contradiction.
    let stored: Option<serde_json::Value> = c
        .query_one(
            "SELECT safety FROM event_log WHERE event_id = $1::text::uuid",
            &[&event_id.to_string()],
        )
        .await
        .expect("query")
        .get(0);
    let stored = stored.expect("the signal is stored");
    assert_eq!(stored["rung"], "existence");
    assert_eq!(stored["class"], "rh-sensitizing", "stored verbatim, not scrubbed");
}

#[tokio::test]
async fn a_well_formed_signal_lands_in_the_column_through_the_local_door() {
    let Some(base) = cs() else { return };
    // The guard is a Client holding a cluster-wide advisory lock: it must stay BOUND for
    // the whole test, and it is taken BEFORE connect_and_load_schema (every existing suite
    // does this in execution order).
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &[]).await;
    let patient = Uuid::now_v7();

    let body = body_with_safety(
        patient,
        &kid,
        3,
        Some(serde_json::json!({"rung": "kind", "severity": "high"})),
    );
    let event_id: Uuid = body.event_id.parse().expect("uuid");
    let signed = cairn_event::sign(&body, &sk).expect("signs");
    c.execute("SELECT submit_event($1, NULL, NULL, NULL)", &[&signed.signed_bytes])
        .await
        .expect("a well-formed signal is admitted");

    let stored: Option<serde_json::Value> = c
        .query_one(
            "SELECT safety FROM event_log WHERE event_id = $1::text::uuid",
            &[&event_id.to_string()],
        )
        .await
        .expect("query")
        .get(0);
    assert_eq!(stored.expect("stored")["severity"], "high");
}

#[tokio::test]
async fn an_event_with_no_signal_stores_null() {
    let Some(base) = cs() else { return };
    // The guard is a Client holding a cluster-wide advisory lock: it must stay BOUND for
    // the whole test, and it is taken BEFORE connect_and_load_schema (every existing suite
    // does this in execution order).
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &[]).await;
    let patient = Uuid::now_v7();

    let body = body_with_safety(patient, &kid, 4, None);
    let event_id: Uuid = body.event_id.parse().expect("uuid");
    let signed = cairn_event::sign(&body, &sk).expect("signs");
    c.execute("SELECT submit_event($1, NULL, NULL, NULL)", &[&signed.signed_bytes])
        .await
        .expect("no signal is the common case");

    let stored: Option<serde_json::Value> = c
        .query_one(
            "SELECT safety FROM event_log WHERE event_id = $1::text::uuid",
            &[&event_id.to_string()],
        )
        .await
        .expect("query")
        .get(0);
    assert!(stored.is_none(), "absence stays absence, never an empty object");
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p cairn-node --test safety_doors 2>&1 | tail -20`
Expected: FAIL — `column "safety" does not exist`.

- [ ] **Step 3: Add the column to `db/049`**

Insert into `db/049_safety_projection.sql`, immediately after `BEGIN;` and before section 1:

```sql
-- ---------------------------------------------------------------------------
-- 0. The clear signal's home: an additive column on the append-only row.
--
--    WHY A COLUMN AND NOT A PROJECTION TABLE. §5.9 requires the safety projection to
--    OUTLIVE the body it protects — to coarsen but survive a rung-3 crypto-shred. A
--    projection table would have to be explicitly EXEMPTED from cairn_execute_shred's
--    scrub (db/037), which is a standing invitation for a future reviewer to "fix" the
--    inconsistency and silently delete the one signal the spec says must survive. On the
--    append-only row it survives because event_log is never touched by a shred: the
--    guarantee is structural rather than remembered. It also needs no apply function and
--    no ADR-0057 registry entry, so no registry row-count pin moves.
--
--    It is a DERIVED VIEW of the signed bytes, exactly like `body` and `clock_grade` —
--    stored verbatim, never sanitized on the way in. Section 4 explains why the
--    interpretation, not the storage, is where a contradiction is refused.
--
--    ADD COLUMN IF NOT EXISTS does not fire the append-only trigger (that fires on
--    UPDATE/DELETE) — the same additive move db/001 makes for attestation/attester_key.
-- ---------------------------------------------------------------------------
ALTER TABLE event_log ADD COLUMN IF NOT EXISTS safety JSONB;
```

- [ ] **Step 4: Wire the local door**

In `db/005_submit.sql`, immediately after the `1c.` contributor-set floor comment block's `PERFORM`/call (i.e. after the contributor check and before the next numbered step), add:

```sql
    -- 1d. §5.9 safety-signal shape (ADR-0063). LOCAL DOOR ONLY — db/020 deliberately does
    --     NOT call this. The signal is a FIELD on a clinical event, so a refusal at the
    --     apply door would drop the clinical event it rides on; a defect in a de-identified
    --     advisory field must never cancel clinical content (ADR-0060). See db/049 section 4.
    PERFORM cairn_check_safety_signal(b);
```

Then add the column to the `INSERT INTO event_log` list — append `safety` after `clock_grade` in the column list, and `b -> 'safety'` after `v_grade` in the `VALUES` list.

- [ ] **Step 5: Wire the remote door (column only)**

In `db/020_apply_remote_event.sql`, make the *same* two INSERT edits — `safety` in the column list after `clock_grade`, `b -> 'safety'` in the VALUES list after `v_grade`. Add **no** check call. Above the INSERT, add:

```sql
    -- The §5.9 safety signal is stored verbatim and NEVER checked here (ADR-0063): see
    -- db/049 section 4 for why this door is deliberately lenient where db/005 is strict.
```

- [ ] **Step 6: Recreate the dev databases**

The `event_log` column add breaks positional `ROW` literals in stale databases.

```bash
for d in cairn_test cairn_test2 cairn_test3; do
  psql -h 127.0.0.1 -p 5532 -d postgres -c "DROP DATABASE IF EXISTS $d;" \
                              -c "CREATE DATABASE $d;"
done
```

- [ ] **Step 7: Run the tests to verify they pass**

Run: `cargo test -p cairn-node --test safety_doors 2>&1 | tail -10`
Expected: PASS, 4 tests.

Run: `cargo test -p cairn-node --test born_sealed_schema 2>&1 | tail -10`
Expected: PASS — this is the suite the stale-database trap breaks. If it fails with *"invalid input syntax for type bigint"*, Step 6 did not take effect.

- [ ] **Step 8: Commit**

```bash
git add db/049_safety_projection.sql db/005_submit.sql db/020_apply_remote_event.sql \
        crates/cairn-node/tests/safety_doors.rs
git commit -m "feat(#375): event_log.safety, and the deliberate door asymmetry

The local door refuses a self-contradictory signal; the remote door admits it
AND the clinical event it rides on. Pinned, not commented: the second assertion
is that the medication row lands, because a defect in a de-identified advisory
field must never cancel clinical content (ADR-0060).

The column is an additive derived view of the signed bytes, stored verbatim —
sanitizing on the way in would make it disagree with signed_bytes. Refusing to
ACT on a contradiction is the read model's job (next task).

On the append-only row rather than in a projection table, so coarsen-but-survive
after a crypto-shred is structural rather than an exemption someone must
remember not to 'fix'.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

### Task 4: The read model — prospective grade, and the total reader

Two functions that never author anything: the grade for an event **not yet written** (needed by Task 5's emission), and the coarsened read.

**Files:**
- Modify: `db/049_safety_projection.sql` (sections 6–7)
- Test: `crates/cairn-node/tests/safety_read.rs`

**Interfaces:**
- Consumes: `cairn_sensitivity_standing(uuid)`, `cairn_effective_sensitivity(uuid)`, `cairn_event_type_has_no_thread(text)` (db/048); `cairn_safety_rung_for_rank`, `cairn_safety_rung_rank` (Task 2); `event_log.safety` (Task 3).
- Produces:
  - `cairn_prospective_sensitivity(p_patient uuid, p_thread uuid) → TABLE(grade text, subject_kind text, content_address bytea)`
  - `cairn_event_safety(p_event_id uuid) → TABLE(rung text, class text, severity text, event_type text, grade text, subject_kind text)`
  - `cairn_patient_safety(p_patient uuid) → TABLE(event_id uuid, rung text, class text, severity text, event_type text, grade text, subject_kind text)`

- [ ] **Step 1: Write the failing test**

Create `crates/cairn-node/tests/safety_read.rs`:

```rust
//! §5.9 part B (ADR-0063) — the read model, and the three properties that make it safe:
//! it re-coarsens by the CURRENT grade, it is TOTAL over any stored shape, and it survives
//! a crypto-shred.
mod common;
use common::{cs, setup};
use uuid::Uuid;

/// Submit a `note.added` carrying a verbatim safety signal, returning its event id.
/// `note.added` is unsealed, so these tests exercise the read model without the seal path.
async fn note_with_safety(
    c: &tokio_postgres::Client,
    sk: &cairn_event::SigningKey,
    kid: &str,
    patient: Uuid,
    wall: i64,
    safety: serde_json::Value,
) -> Uuid {
    let body = cairn_event::EventBody {
        event_id: Uuid::now_v7().to_string(),
        patient_id: patient.to_string(),
        event_type: "note.added".into(),
        schema_version: "note/1".into(),
        hlc: cairn_event::Hlc { wall, counter: 0, node_origin: "n1".into() },
        t_effective: None,
        signer_key_id: kid.into(),
        contributors: serde_json::json!([{"actor_id": kid, "role": "recorded"}]),
        payload: serde_json::json!({"text": "a note"}),
        attachments: vec![],
        plaintext_twin: Some("a note".into()),
        clock_grade: cairn_event::ClockGrade::SelfAsserted,
        safety: Some(safety),
    };
    let id: Uuid = body.event_id.parse().expect("uuid");
    let signed = cairn_event::sign(&body, sk).expect("signs");
    // apply_remote_event, so a test may put a shape on the wire that the local door refuses.
    c.execute("SELECT apply_remote_event($1)", &[&signed.signed_bytes])
        .await
        .expect("admitted");
    id
}

/// Assert a chart-wide grade through the real sensitivity path (db/048).
async fn grade_chart(
    c: &tokio_postgres::Client,
    sk: &cairn_event::SigningKey,
    kid: &str,
    patient: Uuid,
    wall: i64,
    grade: &str,
) {
    let a = cairn_event::sensitivity::SensitivityAssertion {
        subject_kind: cairn_event::sensitivity::SubjectKind::Patient,
        subject_id: patient,
        grade,
        source: "human",
        rationale: Some("test fixture"),
    };
    let body = cairn_event::EventBody {
        event_id: Uuid::now_v7().to_string(),
        patient_id: patient.to_string(),
        event_type: cairn_event::sensitivity::SENSITIVITY_EVENT_TYPE.into(),
        schema_version: cairn_event::sensitivity::SENSITIVITY_SCHEMA_VERSION.into(),
        hlc: cairn_event::Hlc { wall, counter: 0, node_origin: "n1".into() },
        t_effective: None,
        signer_key_id: kid.into(),
        contributors: serde_json::json!([{"actor_id": kid, "role": "recorded"}]),
        payload: cairn_event::sensitivity::sensitivity_assertion_body(&a),
        attachments: vec![],
        plaintext_twin: Some(cairn_event::sensitivity::render_sensitivity_twin(&a)),
        clock_grade: cairn_event::ClockGrade::SelfAsserted,
        safety: None,
    };
    let signed = cairn_event::sign(&body, sk).expect("signs");
    c.execute("SELECT apply_remote_event($1)", &[&signed.signed_bytes])
        .await
        .expect("grade applied");
}

#[tokio::test]
async fn a_peers_finer_rung_is_coarsened_by_this_nodes_grade() {
    let Some(base) = cs() else { return };
    // The guard is a Client holding a cluster-wide advisory lock: it must stay BOUND for
    // the whole test, and it is taken BEFORE connect_and_load_schema (every existing suite
    // does this in execution order).
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &[]).await;
    let patient = Uuid::now_v7();

    // A peer emits `precise` — legitimately, because on ITS node the chart is routine.
    let id = note_with_safety(
        &c, &sk, &kid, patient, 10,
        serde_json::json!({"rung": "precise", "class": "rh-sensitizing", "severity": "high"}),
    ).await;
    grade_chart(&c, &sk, &kid, patient, 11, "sequestered").await;

    let row = c
        .query_one(
            "SELECT rung, class, severity FROM cairn_event_safety($1::text::uuid)",
            &[&id.to_string()],
        )
        .await
        .expect("read model");
    assert_eq!(
        row.get::<_, String>(0), "existence",
        "the grade this node holds must coarsen a peer's finer rung — emission cannot \
         control a peer's bytes, so read is the local defence"
    );
    assert!(row.get::<_, Option<String>>(1).is_none(), "no class survives");
    assert!(row.get::<_, Option<String>>(2).is_none(), "no severity survives");
}

#[tokio::test]
async fn a_self_contradictory_signal_never_surfaces_its_class() {
    let Some(base) = cs() else { return };
    // The guard is a Client holding a cluster-wide advisory lock: it must stay BOUND for
    // the whole test, and it is taken BEFORE connect_and_load_schema (every existing suite
    // does this in execution order).
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &[]).await;
    let patient = Uuid::now_v7();

    // The shape db/005 refuses and db/020 admits. Totality here is what makes that
    // leniency safe rather than merely lenient.
    let id = note_with_safety(
        &c, &sk, &kid, patient, 12,
        serde_json::json!({"rung": "existence", "class": "rh-sensitizing"}),
    ).await;

    let row = c
        .query_one("SELECT rung, class FROM cairn_event_safety($1::text::uuid)", &[&id.to_string()])
        .await
        .expect("read model");
    assert_eq!(row.get::<_, String>(0), "existence");
    assert!(
        row.get::<_, Option<String>>(1).is_none(),
        "a class is surfaced ONLY at rung 'precise', whatever the row holds"
    );
}

#[tokio::test]
async fn an_unrecognised_rung_reads_as_the_coarsest() {
    let Some(base) = cs() else { return };
    // The guard is a Client holding a cluster-wide advisory lock: it must stay BOUND for
    // the whole test, and it is taken BEFORE connect_and_load_schema (every existing suite
    // does this in execution order).
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &[]).await;
    let patient = Uuid::now_v7();

    let id = note_with_safety(
        &c, &sk, &kid, patient, 13,
        serde_json::json!({"rung": "rung:from-a-future-peer", "severity": "critical"}),
    ).await;

    let row = c
        .query_one(
            "SELECT rung, severity FROM cairn_event_safety($1::text::uuid)",
            &[&id.to_string()],
        )
        .await
        .expect("read model");
    assert_eq!(row.get::<_, String>(0), "existence", "unknown ⇒ disclose nothing");
    assert!(row.get::<_, Option<String>>(1).is_none());
}

#[tokio::test]
async fn an_event_with_no_signal_yields_no_row() {
    let Some(base) = cs() else { return };
    // The guard is a Client holding a cluster-wide advisory lock: it must stay BOUND for
    // the whole test, and it is taken BEFORE connect_and_load_schema (every existing suite
    // does this in execution order).
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &[]).await;
    let patient = Uuid::now_v7();

    let body = cairn_event::EventBody {
        event_id: Uuid::now_v7().to_string(),
        patient_id: patient.to_string(),
        event_type: "note.added".into(),
        schema_version: "note/1".into(),
        hlc: cairn_event::Hlc { wall: 14, counter: 0, node_origin: "n1".into() },
        t_effective: None,
        signer_key_id: kid.clone(),
        contributors: serde_json::json!([{"actor_id": kid, "role": "recorded"}]),
        payload: serde_json::json!({"text": "a note"}),
        attachments: vec![],
        plaintext_twin: Some("a note".into()),
        clock_grade: cairn_event::ClockGrade::SelfAsserted,
        safety: None,
    };
    let id: Uuid = body.event_id.parse().expect("uuid");
    let signed = cairn_event::sign(&body, &sk).expect("signs");
    c.execute("SELECT apply_remote_event($1)", &[&signed.signed_bytes]).await.expect("ok");

    let n: i64 = c
        .query_one(
            "SELECT count(*) FROM cairn_event_safety($1::text::uuid)",
            &[&id.to_string()],
        )
        .await
        .expect("read model")
        .get(0);
    assert_eq!(
        n, 0,
        "no signal means no row — an existence marker on every uncoded event would \
         manufacture a warning from nothing (ADR-0059 decision 4's honest floor)"
    );
}

#[tokio::test]
async fn the_signal_survives_a_crypto_shred() {
    let Some(base) = cs() else { return };
    // The guard is a Client holding a cluster-wide advisory lock: it must stay BOUND for
    // the whole test, and it is taken BEFORE connect_and_load_schema (every existing suite
    // does this in execution order).
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &[]).await;
    let patient = Uuid::now_v7();

    let id = note_with_safety(
        &c, &sk, &kid, patient, 15,
        serde_json::json!({"rung": "precise", "class": "rh-sensitizing", "severity": "high"}),
    ).await;

    // The rung-3 shred: custody and derived plaintext die; event_log never does.
    c.execute(
        "SELECT cairn_execute_shred($1::text::uuid, $1::text::uuid, 'test')",
        &[&id.to_string()],
    )
    .await
    .expect("shred");

    let row = c
        .query_one(
            "SELECT rung, class FROM cairn_event_safety($1::text::uuid)",
            &[&id.to_string()],
        )
        .await
        .expect(
            "the safety projection outlives the body it protects — the Rh-after-termination \
             signal must reach a future antenatal clinician even after the episode is erased",
        );
    assert_eq!(row.get::<_, String>(0), "precise");
    assert_eq!(row.get::<_, Option<String>>(1).as_deref(), Some("rh-sensitizing"));
}

#[tokio::test]
async fn prospective_matches_effective_given_the_same_chart_and_thread() {
    let Some(base) = cs() else { return };
    // The guard is a Client holding a cluster-wide advisory lock: it must stay BOUND for
    // the whole test, and it is taken BEFORE connect_and_load_schema (every existing suite
    // does this in execution order).
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &[]).await;
    let patient = Uuid::now_v7();

    // The anti-drift pin. cairn_prospective_sensitivity duplicates cairn_effective_
    // sensitivity's arms minus the event arm, because at emission time the event does not
    // exist yet. If the two ever disagree for an event carrying no event-scoped assertion,
    // one of them has drifted.
    grade_chart(&c, &sk, &kid, patient, 20, "restricted").await;
    let id = note_with_safety(
        &c, &sk, &kid, patient, 21,
        serde_json::json!({"rung": "existence"}),
    ).await;

    let eff: String = c
        .query_one(
            "SELECT grade FROM cairn_effective_sensitivity($1::text::uuid)",
            &[&id.to_string()],
        )
        .await
        .expect("effective")
        .get(0);
    let pro: String = c
        .query_one(
            "SELECT grade FROM cairn_prospective_sensitivity($1::text::uuid, NULL)",
            &[&patient.to_string()],
        )
        .await
        .expect("prospective")
        .get(0);
    assert_eq!(
        pro, eff,
        "prospective and effective must agree when no event-scoped assertion stands"
    );
    assert_eq!(pro, "restricted");
}

#[tokio::test]
async fn the_chart_report_names_the_winning_subject() {
    let Some(base) = cs() else { return };
    // The guard is a Client holding a cluster-wide advisory lock: it must stay BOUND for
    // the whole test, and it is taken BEFORE connect_and_load_schema (every existing suite
    // does this in execution order).
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &[]).await;
    let patient = Uuid::now_v7();

    let _ = note_with_safety(
        &c, &sk, &kid, patient, 30,
        serde_json::json!({"rung": "precise", "class": "statin-interaction", "severity": "moderate"}),
    ).await;
    grade_chart(&c, &sk, &kid, patient, 31, "sensitive").await;

    let rows = c
        .query(
            "SELECT rung, severity, grade, subject_kind FROM cairn_patient_safety($1::text::uuid)",
            &[&patient.to_string()],
        )
        .await
        .expect("chart report");
    assert_eq!(rows.len(), 1, "one signal on this chart");
    assert_eq!(rows[0].get::<_, String>(0), "kind", "'sensitive' coarsens to kind");
    assert_eq!(rows[0].get::<_, Option<String>>(1).as_deref(), Some("moderate"));
    assert_eq!(rows[0].get::<_, String>(2), "sensitive");
    // ADR-0062 decision 8 control 3: never just the grade — a grade with no named source
    // cannot be fixed.
    assert_eq!(rows[0].get::<_, String>(3), "patient");
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p cairn-node --test safety_read 2>&1 | tail -20`
Expected: FAIL — `function cairn_event_safety(uuid) does not exist`.

- [ ] **Step 3: Implement sections 6 and 7 in `db/049`**

Append before the closing `COMMIT;`:

```sql
-- ---------------------------------------------------------------------------
-- 6. The PROSPECTIVE grade — the grade for an event that does not exist yet.
--
--    cairn_effective_sensitivity takes an event_id, and at emission time the event has
--    not been written. This is the same computation MINUS the event arm: an event about
--    to be authored can carry no event-scoped assertion.
--
--    !!! KEEP IN LOCKSTEP WITH db/048 SECTION 11 !!! The two functions duplicate the
--    chart / thread / catch-all arms. crates/cairn-node/tests/safety_read.rs's
--    prospective_matches_effective_given_the_same_chart_and_thread is the anti-drift pin;
--    if you change one arm here or there and that test stays green, the pin is too weak,
--    not the change safe.
--
--    Both delegate to cairn_sensitivity_standing, which stays the SINGLE definition of
--    "what still applies" (ADR-0062 decision 3).
-- ---------------------------------------------------------------------------
CREATE OR REPLACE FUNCTION cairn_prospective_sensitivity(p_patient uuid, p_thread uuid)
RETURNS TABLE (grade text, subject_kind text, content_address bytea)
LANGUAGE sql STABLE AS $$
    WITH standing AS (
        SELECT s.* FROM cairn_sensitivity_standing(p_patient) s
    ),
    applicable AS (
        -- chart-scoped, correctly targeted
        SELECT s.grade, s.subject_kind, s.content_address
        FROM standing s
        WHERE s.subject_kind = 'patient' AND s.subject_id = p_patient
        UNION ALL
        -- thread-scoped, this thread
        SELECT s.grade, s.subject_kind, s.content_address
        FROM standing s
        WHERE s.subject_kind = 'thread' AND p_thread IS NOT NULL AND s.subject_id = p_thread
        UNION ALL
        -- The catch-all (ADR-0062 erratum E1): an assertion we cannot match to a subject
        -- here still coarsens chart-wide, reported as 'coarsened' rather than echoing its
        -- own kind. Includes a thread-scoped assertion when we have NO thread to compare
        -- against — an unresolved thread is decision 9's conservative bound, and at
        -- emission time it is the honest reading of "this event may be on that thread".
        SELECT s.grade, 'coarsened'::text, s.content_address
        FROM standing s
        WHERE s.subject_kind NOT IN ('patient', 'thread', 'event')
           OR (s.subject_kind = 'patient' AND s.subject_id <> p_patient)
           OR (s.subject_kind = 'thread'  AND (p_thread IS NULL OR s.subject_id <> p_thread))
    )
    SELECT COALESCE(a.grade, 'routine'), COALESCE(a.subject_kind, 'none'), a.content_address
    FROM (SELECT NULL::text AS grade, NULL::text AS subject_kind, NULL::bytea AS content_address) z
    LEFT JOIN LATERAL (
        SELECT * FROM applicable
        ORDER BY cairn_sensitivity_rank(applicable.grade) DESC, applicable.content_address
        LIMIT 1
    ) a ON TRUE;
$$;

-- ---------------------------------------------------------------------------
-- 7. The read model. TOTAL over any stored shape — this is what makes db/020's leniency
--    (section 4) safe rather than merely lenient.
--
--    Three totality rules, each of which must hold whatever the row contains:
--      * an unrecognised or missing rung reads as 'existence' (cairn_safety_rung_rank MAX);
--      * a class is surfaced ONLY at rung 'precise' — a class beside a coarser rung is
--        ignored, always;
--      * the rung is the COARSER of what was emitted and what this node's CURRENT grade
--        licenses, because emission cannot control a peer's bytes and read cannot
--        un-publish one.
-- ---------------------------------------------------------------------------
CREATE OR REPLACE FUNCTION cairn_event_safety(p_event_id uuid)
RETURNS TABLE (rung text, class text, severity text, event_type text,
               grade text, subject_kind text)
LANGUAGE sql STABLE AS $$
    WITH ev AS (
        SELECT e.event_id, e.event_type, e.safety
        FROM event_log e
        WHERE e.event_id = p_event_id AND e.safety IS NOT NULL
          AND jsonb_typeof(e.safety) = 'object'
    ),
    graded AS (
        SELECT ev.*, s.grade, s.subject_kind,
               -- The coarser of the two, by rank. Named `eff_rung` so the CASE below reads
               -- as "what may be disclosed", not "what was claimed".
               CASE WHEN cairn_safety_rung_rank(ev.safety ->> 'rung')
                       >= cairn_safety_rung_rank(cairn_safety_rung_for_rank(cairn_sensitivity_rank(s.grade)))
                    -- An unrecognised emitted rung ranks MAX, so this arm normalises it to
                    -- the coarsest NAMED rung rather than echoing a value no reader knows.
                    THEN CASE WHEN cairn_safety_rung_rank(ev.safety ->> 'rung') = 2147483647
                              THEN 'existence'
                              ELSE ev.safety ->> 'rung' END
                    ELSE cairn_safety_rung_for_rank(cairn_sensitivity_rank(s.grade))
               END AS eff_rung
        FROM ev, LATERAL cairn_effective_sensitivity(ev.event_id) s
    )
    SELECT g.eff_rung,
           CASE WHEN g.eff_rung = 'precise' THEN g.safety ->> 'class' END,
           CASE WHEN g.eff_rung IN ('precise', 'kind') THEN g.safety ->> 'severity' END,
           g.event_type, g.grade, g.subject_kind
    FROM graded g;
$$;

--    The chart-wide report: every standing signal, already coarsened. One query, so a UI
--    opening a chart pays one round trip (the §1.2 budget in the slice plan).
CREATE OR REPLACE FUNCTION cairn_patient_safety(p_patient uuid)
RETURNS TABLE (event_id uuid, rung text, class text, severity text, event_type text,
               grade text, subject_kind text)
LANGUAGE sql STABLE AS $$
    SELECT e.event_id, s.rung, s.class, s.severity, s.event_type, s.grade, s.subject_kind
    FROM event_log e, LATERAL cairn_event_safety(e.event_id) s
    WHERE e.patient_id = p_patient AND e.safety IS NOT NULL
    ORDER BY cairn_safety_severity_rank(s.severity) DESC, e.event_id;
$$;

GRANT EXECUTE ON FUNCTION cairn_prospective_sensitivity(uuid, uuid) TO cairn_agent;
GRANT EXECUTE ON FUNCTION cairn_event_safety(uuid) TO cairn_agent;
GRANT EXECUTE ON FUNCTION cairn_patient_safety(uuid) TO cairn_agent;
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p cairn-node --test safety_read 2>&1 | tail -20`
Expected: PASS, 7 tests.

If `prospective_matches_effective_given_the_same_chart_and_thread` fails, compare section 6's arms against `db/048` section 11's line by line — that is exactly what the pin exists to catch.

- [ ] **Step 5: Commit**

```bash
git add db/049_safety_projection.sql crates/cairn-node/tests/safety_read.rs
git commit -m "feat(#375): the read model — total, re-coarsening, and shred-surviving

Three totality rules, each pinned: an unrecognised rung reads as 'existence',
a class surfaces ONLY at rung 'precise' whatever the row holds, and the rung is
the COARSER of what was emitted and what this node's current grade licenses.
Together they are what makes db/020's leniency safe rather than merely lenient.

cairn_prospective_sensitivity duplicates db/048 section 11 minus the event arm,
because at emission time the event does not exist. The duplication is real, so
it carries a lockstep warning and an anti-drift test rather than a promise.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

### Task 5: Emission — the class captured pre-seal, the rung chosen at write

Where the two tiers are actually produced. The pure half is a helper in `cairn-event`; the impure half is one query in `seal_sign_submit`, so no clinical verb can forget it.

**Files:**
- Modify: `crates/cairn-event/src/medication/assert.rs`, `crates/cairn-event/src/medication/coding.rs` (accept an optional precise claim and write `payload.safety`), `crates/cairn-node/src/medication/sealed_submit.rs` (the coarsening), `crates/cairn-node/src/medication/assert.rs` + `coding.rs` (the map lookup)
- Create: `crates/cairn-node/src/safety.rs`
- Test: `crates/cairn-node/tests/safety_emission.rs`

**Interfaces:**
- Consumes: `PreciseSafety`, `precise_safety_body`, `coarsen`, `SafetyRung` (Task 1); `cairn_safety_class_candidate`, `cairn_prospective_sensitivity`, `cairn_safety_rung_for_rank` (Tasks 2/4).
- Produces:
  - `cairn_node::safety::lookup_class(client, coding) -> anyhow::Result<Option<(String, String)>>`
  - `cairn_node::safety::prospective_rung(client, patient, thread) -> anyhow::Result<SafetyRung>`
  - `cairn_event::medication::MedicationAssertion.safety: Option<PreciseSafety<'a>>` and the same on `MedicationCoding`
  - `seal_sign_submit` unchanged in signature; it reads `payload.safety` itself.

- [ ] **Step 1: Write the failing test**

Create `crates/cairn-node/tests/safety_emission.rs`:

```rust
//! §5.9 part B (ADR-0063) — emission. The rung is chosen from the grade standing at
//! AUTHORING time; the precise class goes under the seal. This is the only coarsening that
//! binds a peer's raw-SQL client, because it decides what is put on the wire at all.
mod common;
use cairn_event::sensitivity::SubjectKind;
use cairn_node::medication::{assert_medication, AssertMedicationInput, SubstanceCoding};
use common::{cs, medication_setup};
use uuid::Uuid;

/// Populate the deployment class map. The shipped table is EMPTY on purpose (Cairn ships
/// the lookup, never the drug knowledge), so a test that wants a class must supply one —
/// exactly as a deployment does.
async fn map_class(c: &tokio_postgres::Client, code: &str, class: &str, severity: &str) {
    c.execute(
        "INSERT INTO safety_class_map (system, code, class, severity)
         VALUES ('drugref-moiety', $1, $2, $3) ON CONFLICT DO NOTHING",
        &[&code, &class, &severity],
    )
    .await
    .expect("seed the map");
}

/// The clear signal stored on an event, or None.
async fn stored_signal(c: &tokio_postgres::Client, event: Uuid) -> Option<serde_json::Value> {
    c.query_one(
        "SELECT safety FROM event_log WHERE event_id = $1::text::uuid",
        &[&event.to_string()],
    )
    .await
    .expect("query")
    .get(0)
}

/// The event id of a thread's assertion (the thread's own assert event).
async fn assert_event_of(c: &tokio_postgres::Client, thread: Uuid) -> Uuid {
    c.query_one(
        "SELECT e.event_id FROM event_log e
         JOIN medication_statement m ON m.content_address = e.content_address
         WHERE m.medication_id = $1::text::uuid",
        &[&thread.to_string()],
    )
    .await
    .expect("the assert event")
    .get(0)
}

#[tokio::test]
async fn a_coded_assert_on_a_routine_chart_emits_the_precise_rung() {
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let mut c = cairn_node::db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid, _hsk, _hkid) = medication_setup(&c).await;
    let patient = Uuid::now_v7();
    map_class(&c, "moiety-1", "rh-sensitizing", "high").await;

    let thread = assert_medication(
        &mut c, &sk, &kid, "n1", patient,
        &AssertMedicationInput {
            term: "anti-D immunoglobulin",
            coding: Some(SubstanceCoding {
                system: "drugref-moiety", code: "moiety-1", display: "anti-D immunoglobulin",
            }),
            formulation: None, dose_amount: None, dose_unit: None, sig: None,
            info_source: "patient", started: None, started_precision: None,
        },
        None, None,
    )
    .await
    .expect("assert");

    let ev = assert_event_of(&c, thread).await;
    let s = stored_signal(&c, ev).await.expect("a coded medication emits a signal");
    assert_eq!(s["rung"], "precise", "no standing grade ⇒ full precision");
    assert_eq!(s["class"], "rh-sensitizing");
    assert_eq!(s["severity"], "high");

    // And the precise claim is ALSO under the seal — that is the tier a custody-holding
    // node reads without any drug database (#294).
    let clear: serde_json::Value = c
        .query_one(
            "SELECT body FROM event_clear WHERE event_id = $1::text::uuid",
            &[&ev.to_string()],
        )
        .await
        .expect("the sealed payload's clear shadow")
        .get(0);
    assert_eq!(clear["safety"]["class"], "rh-sensitizing");
    assert_eq!(clear["safety"]["severity"], "high");
}

#[tokio::test]
async fn a_graded_chart_coarsens_the_emitted_rung_but_never_the_sealed_claim() {
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let mut c = cairn_node::db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid, hsk, hkid) = medication_setup(&c).await;
    let patient = Uuid::now_v7();
    map_class(&c, "moiety-2", "antiretroviral-interaction", "critical").await;

    cairn_node::sensitivity::assert_sensitivity(
        &mut c, &sk, &kid, "n1", patient,
        SubjectKind::Patient, patient,
        "sensitive", Some("test fixture"),
    )
    .await
    .expect("grade the chart");
    let _ = (&hsk, &hkid);

    let thread = assert_medication(
        &mut c, &sk, &kid, "n1", patient,
        &AssertMedicationInput {
            term: "tenofovir",
            coding: Some(SubstanceCoding {
                system: "drugref-moiety", code: "moiety-2", display: "tenofovir",
            }),
            formulation: None, dose_amount: None, dose_unit: None, sig: None,
            info_source: "patient", started: None, started_precision: None,
        },
        None, None,
    )
    .await
    .expect("assert");

    let ev = assert_event_of(&c, thread).await;
    let s = stored_signal(&c, ev).await.expect("still a signal");
    assert_eq!(s["rung"], "kind", "'sensitive' coarsens to kind");
    assert!(
        s.get("class").is_none(),
        "the class must never be published in the clear on a graded chart — it IS the \
         disclosure the grade exists to prevent"
    );
    assert_eq!(s["severity"], "critical", "severity survives the middle rung");

    let clear: serde_json::Value = c
        .query_one(
            "SELECT body FROM event_clear WHERE event_id = $1::text::uuid",
            &[&ev.to_string()],
        )
        .await
        .expect("clear shadow")
        .get(0);
    assert_eq!(
        clear["safety"]["class"], "antiretroviral-interaction",
        "the sealed tier is never coarsened — the seal is what protects it"
    );
}

#[tokio::test]
async fn a_sequestered_chart_emits_existence_only() {
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let mut c = cairn_node::db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid, _h, _hk) = medication_setup(&c).await;
    let patient = Uuid::now_v7();
    map_class(&c, "moiety-3", "rh-sensitizing", "high").await;

    cairn_node::sensitivity::assert_sensitivity(
        &mut c, &sk, &kid, "n1", patient,
        SubjectKind::Patient, patient,
        "sequestered", Some("protected witness"),
    )
    .await
    .expect("grade");

    let thread = assert_medication(
        &mut c, &sk, &kid, "n1", patient,
        &AssertMedicationInput {
            term: "anti-D", coding: Some(SubstanceCoding {
                system: "drugref-moiety", code: "moiety-3", display: "anti-D",
            }),
            formulation: None, dose_amount: None, dose_unit: None, sig: None,
            info_source: "patient", started: None, started_precision: None,
        },
        None, None,
    )
    .await
    .expect("assert");

    let s = stored_signal(&c, assert_event_of(&c, thread).await).await.expect("signal");
    assert_eq!(s["rung"], "existence");
    assert!(s.get("class").is_none());
    assert!(s.get("severity").is_none(), "severity is withheld at the coarsest rung");
}

#[tokio::test]
async fn an_uncoded_medication_emits_no_signal_at_all() {
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let mut c = cairn_node::db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid, _h, _hk) = medication_setup(&c).await;
    let patient = Uuid::now_v7();

    let thread = assert_medication(
        &mut c, &sk, &kid, "n1", patient,
        &AssertMedicationInput {
            term: "little white pill", coding: None,
            formulation: None, dose_amount: None, dose_unit: None, sig: None,
            info_source: "patient", started: None, started_precision: None,
        },
        None, None,
    )
    .await
    .expect("assert");

    assert!(
        stored_signal(&c, assert_event_of(&c, thread).await).await.is_none(),
        "an uncoded medication has no class on ANY node — that is principle 4 being \
         honest, not a degradation, and manufacturing an existence marker for it would \
         reproduce §5.12's alert fatigue on day one (ADR-0059 decision 4)"
    );
}

#[tokio::test]
async fn a_coding_absent_from_the_map_emits_no_signal() {
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let mut c = cairn_node::db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid, _h, _hk) = medication_setup(&c).await;
    let patient = Uuid::now_v7();
    // Deliberately do NOT seed the map: this deployment's coding authority has no opinion
    // about this substance. Absence of knowledge is not a graded secret.

    let thread = assert_medication(
        &mut c, &sk, &kid, "n1", patient,
        &AssertMedicationInput {
            term: "atorvastatin", coding: Some(SubstanceCoding {
                system: "drugref-moiety", code: "not-in-the-map", display: "atorvastatin",
            }),
            formulation: None, dose_amount: None, dose_unit: None, sig: None,
            info_source: "patient", started: None, started_precision: None,
        },
        None, None,
    )
    .await
    .expect("assert");

    assert!(stored_signal(&c, assert_event_of(&c, thread).await).await.is_none());
}

#[tokio::test]
async fn the_reconciliation_path_emits_no_signal() {
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let mut c = cairn_node::db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid, _h, _hk) = medication_setup(&c).await;
    let patient = Uuid::now_v7();
    map_class(&c, "moiety-r", "statin-interaction", "moderate").await;

    // Written out twice rather than through a closure: an async closure capturing `sk`
    // would move it on the first call, and the second would not compile.
    let input = |term: &'static str| AssertMedicationInput {
        term,
        coding: Some(SubstanceCoding {
            system: "drugref-moiety",
            code: "moiety-r",
            display: term,
        }),
        formulation: None,
        dose_amount: None,
        dose_unit: None,
        sig: None,
        info_source: "patient",
        started: None,
        started_precision: None,
    };
    let a = assert_medication(&mut c, &sk, &kid, "n1", patient, &input("atorvastatin"), None, None)
        .await
        .expect("assert a");
    let b = assert_medication(&mut c, &sk, &kid, "n1", patient, &input("Lipitor"), None, None)
        .await
        .expect("assert b");

    // The two-thread reconcile/separate verbs go through `seal_and_sign` directly rather
    // than `seal_sign_submit`, and carry no coding of their own. The omission is
    // DELIBERATE — a reconciliation is a link between threads, not a drug claim — so it is
    // pinned rather than left to be rediscovered as a bug.
    let recon = cairn_node::medication::reconcile_medications(&mut c, &sk, &kid, "n1", patient, a, b)
        .await
        .expect("reconcile");
    assert!(
        stored_signal(&c, recon).await.is_none(),
        "a reconciliation carries no drug claim, so it emits no safety signal"
    );
}
```

Adapt `reconcile_medications`'s name and argument order to the real orchestrator: find it with
`grep -n "pub async fn reconcile" crates/cairn-node/src/medication/reconciliation.rs`. If it
returns the thread pair rather than the event id, read the event id back the way
`assert_event_of` does, keyed on `medication_reconciliation.content_address`.

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p cairn-node --test safety_emission 2>&1 | tail -20`
Expected: FAIL — `a coded medication emits a signal` panics on `None` (the column is written but nothing populates it yet).

- [ ] **Step 3: Add the precise claim to the pure medication builders**

In `crates/cairn-event/src/medication/assert.rs`, add to `MedicationAssertion`:

```rust
    /// The §5.9 precise safety claim, established pre-seal by the coding node (ADR-0063).
    /// `None` when the medication is uncoded, or when this deployment's class map has no
    /// row for the coding — both are honest absences, not withheld secrets.
    pub safety: Option<crate::safety::PreciseSafety<'a>>,
```

and in `medication_assertion_body`, after the existing optional keys:

```rust
    // Under the seal: never coarsened, because the seal is what protects it. The CLEAR
    // rung is written by the node's emission seam, not here — this crate stays pure.
    if let Some(s) = a.safety {
        obj.insert("safety".into(), crate::safety::precise_safety_body(&s));
    }
```

Make the same two edits to `MedicationCoding` and `medication_coding_body` in
`crates/cairn-event/src/medication/coding.rs`. Fix every construction site the new field
breaks (`cargo build --workspace`), adding `safety: None`.

- [ ] **Step 4: Add the node-side lookup module**

Create `crates/cairn-node/src/safety.rs`:

```rust
//! §5.9 part B (ADR-0063) — the impure half of emission: two small queries.
//!
//! The PURE half (what each rung discloses) lives in `cairn_event::safety`. This module
//! only fetches what the pure half needs: the deployment's class for a coding, and the
//! disclosure rung the chart's current grade licenses.
//!
//! # Why the lookup runs HERE and never in a reader
//!
//! A coded drug's interaction class is a property of the code — a drug-knowledge lookup.
//! A reader that re-derived it would make the §5.9 safety floor depend on holding drugref
//! after all, which is precisely the failure ADR-0059 decision 4 / #294 exist to prevent.
//! The authoring node, by construction, had a coding authority in hand at that moment; so
//! the class is captured here, sealed with the body, and CARRIED.
use cairn_event::medication::SubstanceCoding;
use cairn_event::safety::SafetyRung;
use uuid::Uuid;

/// This deployment's class + severity for a coding, or `None`.
///
/// `None` is the common case and is honest: `safety_class_map` ships empty, so a node with
/// no coding authority configured simply emits no signal. It never guesses.
pub async fn lookup_class(
    client: &tokio_postgres::Client,
    coding: &SubstanceCoding<'_>,
) -> anyhow::Result<Option<(String, String)>> {
    let coding_json = serde_json::json!({ "system": coding.system, "code": coding.code });
    let rows = client
        .query(
            "SELECT class, severity FROM cairn_safety_class_candidate($1::text::jsonb)",
            &[&coding_json.to_string()],
        )
        .await?;
    Ok(rows.first().map(|r| (r.get(0), r.get(1))))
}

/// The disclosure rung the chart's currently-standing grade licenses for an event about to
/// be authored on `thread` (pass `None` when the event belongs to no thread).
///
/// Reads through `cairn_prospective_sensitivity` rather than `cairn_effective_sensitivity`
/// because the event does not exist yet — see db/049 section 6.
///
/// KNOWN RACE, declared rather than defended against: this read and the subsequent submit
/// are separate statements, so a grade raised in between yields a rung one step too fine.
/// The window cannot be closed by moving the decision into `submit_event` — the rung must
/// be inside the SIGNED bytes, and signing happens in this daemon where the DEK lives. The
/// read model re-coarsens on every node that later holds the grade (db/049 section 7).
pub async fn prospective_rung(
    client: &tokio_postgres::Client,
    patient: Uuid,
    thread: Option<Uuid>,
) -> anyhow::Result<SafetyRung> {
    let rung: String = client
        .query_one(
            "SELECT cairn_safety_rung_for_rank(cairn_sensitivity_rank(g.grade))
             FROM cairn_prospective_sensitivity($1::text::uuid, $2::text::uuid) g",
            &[&patient.to_string(), &thread.map(|t| t.to_string())],
        )
        .await?
        .get(0);
    Ok(match rung.as_str() {
        "precise" => SafetyRung::Precise,
        "kind" => SafetyRung::Kind,
        // Anything else — including a rung a future db/049 introduces — is treated as the
        // coarsest. Disclosing on a value this build does not recognise is the one
        // direction that cannot be undone.
        _ => SafetyRung::Existence,
    })
}
```

Register it in `crates/cairn-node/src/lib.rs` beside the other `pub mod` entries:

```rust
pub mod safety;
```

- [ ] **Step 5: Coarsen in `seal_sign_submit`**

In `crates/cairn-node/src/medication/sealed_submit.rs`, inside `seal_sign_submit`, after the
`apply_author` line and before `let event_id`, insert:

```rust
    // §5.9 part B (ADR-0063): choose the CLEAR disclosure rung from the grade standing on
    // this chart right now, and attach it to the envelope. Done HERE, in the one path every
    // clinical verb submits through, so no future verb can forget it — the same argument
    // that put seal-then-sign here.
    let mut body = body;
    if let Some(precise) = body.payload.get("safety").cloned() {
        let patient_for_rung: Uuid = body.patient_id.parse().with_context(|| {
            format!("seal_sign_submit: patient_id {:?} is not a uuid", body.patient_id)
        })?;
        // The thread when the body names one; medication verbs always do, and a future
        // thread-free clinical verb honestly passes None (the chart-wide arms still apply).
        let thread_for_rung = body
            .payload
            .get("medication_id")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<uuid::Uuid>().ok());
        let rung = crate::safety::prospective_rung(client, patient_for_rung, thread_for_rung).await?;
        // The pure coarsening. Reading the class/severity back out of the payload keeps
        // this seam total over any builder that wrote a precise claim, present or future.
        let class = precise.get("class").and_then(|v| v.as_str()).unwrap_or_default();
        let severity = precise.get("severity").and_then(|v| v.as_str()).unwrap_or_default();
        body.safety = Some(cairn_event::safety::coarsen(
            &cairn_event::safety::PreciseSafety { class, severity },
            rung,
        ));
    }
```

Add `use uuid::Uuid;` to the module's imports if not already present.

- [ ] **Step 6: Look the class up in the two coding-carrying verbs**

In `crates/cairn-node/src/medication/assert.rs`, inside `assert_medication`, before `let body = build_assert_body(...)`:

```rust
    // The class is looked up on the CODING node, pre-seal (#294 / ADR-0059 decision 4).
    // An uncoded medication, or a coding this deployment's map has no row for, yields
    // None — both are honest absences and emit no signal at all.
    let class = match input.coding.as_ref() {
        Some(coding) => crate::safety::lookup_class(client, coding).await?,
        None => None,
    };
    let safety = class
        .as_ref()
        .map(|(class, severity)| cairn_event::safety::PreciseSafety { class, severity });
```

Thread `safety` through `build_assert_body` into the `MedicationAssertion` it constructs — add a
trailing parameter and pass it into the struct literal:

```rust
pub fn build_assert_body(
    event_id: Uuid,
    medication_id: Uuid,
    patient: Uuid,
    input: &AssertMedicationInput<'_>,
    node_kid: &str,
    hlc: Hlc,
    /// The §5.9 precise safety claim, already looked up by the caller. Passed in rather
    /// than looked up here so this function stays PURE and unit-testable without a
    /// database (house rule 4 / the §9 blast-radius rule).
    safety: Option<cairn_event::safety::PreciseSafety<'_>>,
) -> EventBody {
```

…and inside, on the `MedicationAssertion` literal, add `safety,`.

In `crates/cairn-node/src/medication/coding.rs`, make the same change to the overlay
orchestrator `code_medication`, which is the other verb that carries a coding:

```rust
    // Same seam, same reason: a coding OVERLAY is authored by whoever codes it — a
    // pharmacist or a professional coder — and that node is again the one holding a coding
    // authority. The class is captured here and travels; no reader re-derives it.
    let class = crate::safety::lookup_class(client, &input.coding).await?;
    let safety = class
        .as_ref()
        .map(|(class, severity)| cairn_event::safety::PreciseSafety { class, severity });
```

and thread `safety` into its `build_coding_body` the same way.

**`correct_medication_coding` is deliberately NOT changed in this slice.** A
`CodingClaim::Strike` carries no coding, so there is no class to look up; and a
`CodingClaim::Replace` is a *correction* whose safety consequences ride the thread rather
than the single event, which is the thread-rollup question this slice does not open. Leave
it emitting no signal and note it in ADR-0063's known limitations.

- [ ] **Step 7: Run the tests to verify they pass**

Run: `cargo test -p cairn-node --test safety_emission 2>&1 | tail -20`
Expected: PASS, 5 tests.

- [ ] **Step 8: Commit**

```bash
git add crates/cairn-event/src/medication/ crates/cairn-node/src/safety.rs \
        crates/cairn-node/src/lib.rs crates/cairn-node/src/medication/ \
        crates/cairn-node/tests/safety_emission.rs
git commit -m "feat(#375): emission — the class captured pre-seal, the rung chosen at write

The class is looked up on the CODING node, which by construction had a coding
authority in hand, and travels sealed with the body. The clear rung is chosen in
seal_sign_submit — the one path every clinical verb submits through, so no
future verb can forget it.

Pinned in both directions: a graded chart coarsens the emitted rung while the
SEALED claim keeps full precision, and an uncoded medication emits nothing at
all rather than an existence marker that would manufacture a warning from
nothing.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

### Task 6: The `patient-safety` verb, and #294's obligation test

The read surface, plus the test the medication coding slice owed and could not write.

**Files:**
- Modify: `crates/cairn-node/src/safety.rs` (the report reader), `crates/cairn-node/src/main.rs` (the `Cmd` variant + handler)
- Test: `crates/cairn-node/tests/safety_carried_class.rs`

**Interfaces:**
- Consumes: `cairn_patient_safety` (Task 4).
- Produces: `cairn_node::safety::chart_safety(client, patient) -> anyhow::Result<Vec<SafetyLine>>`, `pub struct SafetyLine { pub event_id, pub rung, pub class, pub severity, pub event_type, pub grade, pub subject_kind }`.

- [ ] **Step 1: Write the failing test**

Create `crates/cairn-node/tests/safety_carried_class.rs`:

```rust
//! #294's obligation, discharged: a node with NO local drug knowledge still reports the
//! precise class, proving it was CARRIED rather than re-derived.
//!
//! This is the test crates/cairn-node/tests/medication_coding.rs owed since slice 6a and
//! could not write, because there was no safety projection to fire.
mod common;
use cairn_event::sensitivity::SubjectKind;
use cairn_node::medication::{assert_medication, AssertMedicationInput, SubstanceCoding};
use common::{cs, medication_setup};
use uuid::Uuid;

#[tokio::test]
async fn a_node_with_an_empty_class_map_still_reports_the_carried_class() {
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let mut c = cairn_node::db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid, _h, _hk) = medication_setup(&c).await;
    let patient = Uuid::now_v7();

    // The AUTHORING node has a coding authority: one row in the map.
    c.execute(
        "INSERT INTO safety_class_map (system, code, class, severity)
         VALUES ('drugref-moiety', 'm-294', 'rh-sensitizing', 'high')",
        &[],
    )
    .await
    .expect("seed");

    let _thread = assert_medication(
        &mut c, &sk, &kid, "n1", patient,
        &AssertMedicationInput {
            term: "anti-D immunoglobulin",
            coding: Some(SubstanceCoding {
                system: "drugref-moiety", code: "m-294", display: "anti-D immunoglobulin",
            }),
            formulation: None, dose_amount: None, dose_unit: None, sig: None,
            info_source: "patient", started: None, started_precision: None,
        },
        None, None,
    )
    .await
    .expect("assert");

    // Now become a node with NO drug knowledge at all. The map is where every scrap of
    // local class knowledge lives, so emptying it is exactly "this node holds no drugref".
    c.execute("DELETE FROM safety_class_map", &[])
        .await
        .expect("drop all local drug knowledge");

    let lines = cairn_node::safety::chart_safety(&c, patient)
        .await
        .expect("the chart report");
    assert_eq!(lines.len(), 1, "the signal is still there");
    assert_eq!(
        lines[0].class.as_deref(),
        Some("rh-sensitizing"),
        "a drugref-less node honours the §5.9 floor for a CODED medication because the \
         class was captured pre-seal on the coding node and CARRIED — never re-derived \
         (ADR-0059 decision 4 / #294)"
    );
    assert_eq!(lines[0].severity.as_deref(), Some("high"));
}

#[tokio::test]
async fn the_report_names_nothing_beyond_what_the_rung_licenses() {
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let mut c = cairn_node::db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid, _h, _hk) = medication_setup(&c).await;
    let patient = Uuid::now_v7();

    c.execute(
        "INSERT INTO safety_class_map (system, code, class, severity)
         VALUES ('drugref-moiety', 'm-seq', 'antiretroviral-interaction', 'critical')",
        &[],
    )
    .await
    .expect("seed");
    cairn_node::sensitivity::assert_sensitivity(
        &mut c, &sk, &kid, "n1", patient,
        SubjectKind::Patient, patient,
        "sequestered", Some("test"),
    )
    .await
    .expect("grade");

    let _ = assert_medication(
        &mut c, &sk, &kid, "n1", patient,
        &AssertMedicationInput {
            term: "tenofovir", coding: Some(SubstanceCoding {
                system: "drugref-moiety", code: "m-seq", display: "tenofovir",
            }),
            formulation: None, dose_amount: None, dose_unit: None, sig: None,
            info_source: "patient", started: None, started_precision: None,
        },
        None, None,
    )
    .await
    .expect("assert");

    let lines = cairn_node::safety::chart_safety(&c, patient).await.expect("report");
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].rung, "existence");
    assert!(lines[0].class.is_none(), "the class must not reach the report");
    assert!(lines[0].severity.is_none());
    // ADR-0062 decision 8 control 3: never just the grade.
    assert_eq!(lines[0].grade, "sequestered");
    assert_eq!(lines[0].subject_kind, "patient");
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p cairn-node --test safety_carried_class 2>&1 | tail -20`
Expected: FAIL to compile — `no function or associated item named chart_safety`.

- [ ] **Step 3: Implement the reader**

Append to `crates/cairn-node/src/safety.rs`:

```rust
/// One de-identified safety line, already coarsened by db/049's read model.
///
/// `class` and `severity` are `Option` because the RUNG decides whether they exist at all —
/// they are not "missing data". A `None` class at rung `existence` is the mechanism working,
/// not a gap.
pub struct SafetyLine {
    pub event_id: Uuid,
    pub rung: String,
    pub class: Option<String>,
    pub severity: Option<String>,
    pub event_type: String,
    /// The §5.9 grade that produced this coarseness…
    pub grade: String,
    /// …and WHICH subject won it (ADR-0062 decision 8 control 3: a grade with no named
    /// source cannot be fixed, because nobody can tell one thing to go and look at from
    /// twenty).
    pub subject_kind: String,
}

/// Every standing safety signal on a chart, coarsest-safe and already de-identified.
///
/// A pure read: no signing key, no HLC tick, nothing authored. One query, so a UI opening a
/// chart pays a single round trip.
pub async fn chart_safety(
    client: &tokio_postgres::Client,
    patient: Uuid,
) -> anyhow::Result<Vec<SafetyLine>> {
    let rows = client
        .query(
            "SELECT event_id, rung, class, severity, event_type, grade, subject_kind
             FROM cairn_patient_safety($1::text::uuid)",
            &[&patient.to_string()],
        )
        .await?;
    Ok(rows
        .iter()
        .map(|r| SafetyLine {
            event_id: r.get(0),
            rung: r.get(1),
            class: r.get(2),
            severity: r.get(3),
            event_type: r.get(4),
            grade: r.get(5),
            subject_kind: r.get(6),
        })
        .collect())
}

/// The human sentence for one line — §5.9's warning that NAMES NOTHING.
///
/// Pure and total, so the CLI and any future UI cannot phrase the same signal differently.
/// The event TYPE is already plaintext on the row, so naming it discloses nothing new and
/// is what makes the middle rung read as "confidential medication" rather than
/// "confidential something".
pub fn render_safety_line(line: &SafetyLine) -> String {
    let noun = if line.event_type.starts_with("clinical.medication") {
        "medication"
    } else {
        "content"
    };
    match (line.class.as_deref(), line.severity.as_deref()) {
        (Some(class), Some(sev)) => format!("⚠ {sev} — {class}"),
        (None, Some(sev)) => format!("⚠ {sev} — confidential {noun}, break glass to view"),
        _ => format!("⚠ confidential {noun} — break glass to view"),
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p cairn-node --test safety_carried_class 2>&1 | tail -10`
Expected: PASS, 2 tests.

- [ ] **Step 5: Add the CLI verb**

In `crates/cairn-node/src/main.rs`, add a `Cmd` variant immediately after `PatientSensitivity`:

```rust
    /// Report a chart's §5.9 de-identified safety signals: one line per graded clinical
    /// event, each already coarsened by the sensitivity grade standing on this node. It
    /// NAMES NOTHING beyond what the rung licenses — no agent, no diagnosis, no scope key.
    /// REPORTS ONLY: this slice withholds no content (enforcement is #232 part C).
    PatientSafety {
        #[arg(long)]
        patient: Uuid,
    },
```

and the handler immediately after the `Cmd::PatientSensitivity` arm:

```rust
        Cmd::PatientSafety { patient } => {
            // A pure read — no signing key, no HLC tick, nothing authored.
            let db = cairn_node::db::connect(&cli.conn).await?;
            let lines = cairn_node::safety::chart_safety(&db, patient).await?;
            if lines.is_empty() {
                println!("chart {patient}: no safety signals on file");
            }
            for l in &lines {
                println!(
                    "{}  {}  (grade {}, winning subject: {})",
                    cairn_node::safety::render_safety_line(l),
                    l.event_type,
                    l.grade,
                    l.subject_kind
                );
            }
            println!(
                "(report only — nothing is withheld on the strength of these grades; \
                 enforcement needs custody narrowing, #232 part C)"
            );
        }
```

- [ ] **Step 6: Verify the verb runs**

Run: `cargo build -p cairn-node 2>&1 | tail -5 && cargo run -p cairn-node -- patient-safety --help 2>&1 | tail -10`
Expected: the help text for `patient-safety`, naming `--patient`.

- [ ] **Step 7: Commit**

```bash
git add crates/cairn-node/src/safety.rs crates/cairn-node/src/main.rs \
        crates/cairn-node/tests/safety_carried_class.rs
git commit -m "feat(#375): patient-safety, and #294's obligation discharged

The headline test empties safety_class_map — every scrap of local drug knowledge
— AFTER authoring, and the precise class still reports. That is the difference
between carried and re-derived, and it is the test the medication coding slice
has owed since slice 6a.

render_safety_line is pure and shared, so the CLI and any future UI cannot
phrase the same signal differently.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

### Task 7: The SQL mirror, and the cairn-sync subset drive test

`db/tests` mirrors are the floor's own regression suite, independent of Rust. The subset test must *drive* db/049, not merely load it — the mistake [#386](https://github.com/cairn-ehr/cairn-ehr/issues/386) records against db/048.

**Files:**
- Create: `db/tests/049_safety_projection_test.sql`
- Modify: `crates/cairn-sync/tests/schema_subset.rs` (or whichever suite loads the subset — locate with `grep -rn "048_sensitivity" crates/cairn-sync/tests/`)

- [ ] **Step 1: Write the mirror**

Create `db/tests/049_safety_projection_test.sql`:

```sql
-- SQL mirror of crates/cairn-node/tests/safety_* (run by scripts/run-db-sql-tests.sh;
-- the disposable-database rule these mirrors share is in _scratch_database_guard.sql).
-- DESTRUCTIVE: runs only against a database marked disposable (#169).
\set ON_ERROR_STOP on
\ir _scratch_database_guard.sql

-- ---------------------------------------------------------------------------
-- 1. Both ladders, and the monotone rung map. Mirrors safety_ladder.rs.
-- ---------------------------------------------------------------------------
DO $$
BEGIN
    ASSERT cairn_safety_severity_rank('none') = 0, 'none is the floor';
    ASSERT cairn_safety_severity_rank('low') < cairn_safety_severity_rank('critical'),
        'the severity ladder is ordered';
    ASSERT cairn_safety_severity_rank('severity:novel') = 2147483647,
        'an unrecognised severity ranks MAX — assume the worst (ADR-0063)';
    ASSERT cairn_safety_severity_rank(NULL) = 2147483647, 'NULL lands on the safe side';

    ASSERT cairn_safety_rung_rank('precise') < cairn_safety_rung_rank('kind'),
        'the rung ladder is ordered coarsest-last';
    ASSERT cairn_safety_rung_rank('kind') < cairn_safety_rung_rank('existence'),
        'the rung ladder is ordered coarsest-last';
    ASSERT cairn_safety_rung_rank('rung:novel') = 2147483647,
        'an unrecognised rung is treated as coarsest, never as show-everything';

    ASSERT cairn_safety_rung_for_rank(cairn_sensitivity_rank('routine')) = 'precise',
        'no standing grade discloses fully';
    ASSERT cairn_safety_rung_for_rank(cairn_sensitivity_rank('sensitive')) = 'kind';
    ASSERT cairn_safety_rung_for_rank(cairn_sensitivity_rank('restricted')) = 'existence';
    ASSERT cairn_safety_rung_for_rank(cairn_sensitivity_rank('sequestered')) = 'existence';
    ASSERT cairn_safety_rung_for_rank(cairn_sensitivity_rank('grade:future')) = 'existence',
        'an unrecognised grade ranks MAX (ADR-0062), hence coarsest here';
    ASSERT cairn_safety_rung_for_rank(NULL) = 'existence', 'no answer ⇒ disclose nothing';
END $$;

-- Monotonicity across the whole ladder, as a set: a higher grade may never disclose more.
DO $$
DECLARE v_bad int;
BEGIN
    SELECT count(*) INTO v_bad
    FROM (
        SELECT r, cairn_safety_rung_rank(cairn_safety_rung_for_rank(r)) AS rung_rank,
               lag(cairn_safety_rung_rank(cairn_safety_rung_for_rank(r)))
                   OVER (ORDER BY r) AS prev
        FROM unnest(ARRAY[0, 5, 10, 15, 20, 30, 2147483647]) AS r
    ) t
    WHERE prev IS NOT NULL AND rung_rank < prev;
    ASSERT v_bad = 0, 'the rung map must be monotone non-decreasing in grade rank';
END $$;

-- ---------------------------------------------------------------------------
-- 2. The structural floor. Mirrors safety_ladder.rs's floor tests.
-- ---------------------------------------------------------------------------
DO $$
DECLARE v_msg text;
BEGIN
    -- Admitted shapes.
    PERFORM cairn_check_safety_signal('{}'::jsonb);
    PERFORM cairn_check_safety_signal('{"safety":{"rung":"precise","class":"c","severity":"high"}}'::jsonb);
    PERFORM cairn_check_safety_signal('{"safety":{"rung":"kind","severity":"high"}}'::jsonb);
    PERFORM cairn_check_safety_signal('{"safety":{"rung":"existence"}}'::jsonb);
    -- A future peer's rung is ADMITTED: the floor gates effect, not presence (ADR-0056).
    PERFORM cairn_check_safety_signal('{"safety":{"rung":"rung:novel"}}'::jsonb);

    -- The disclosure guard: a class the rung does not license.
    BEGIN
        PERFORM cairn_check_safety_signal('{"safety":{"rung":"existence","class":"c"}}'::jsonb);
        ASSERT false, 'a class at a coarser rung must be refused';
    EXCEPTION WHEN others THEN
        GET STACKED DIAGNOSTICS v_msg = MESSAGE_TEXT;
        ASSERT v_msg LIKE '%class%', 'the refusal names the offending key: ' || v_msg;
    END;

    BEGIN
        PERFORM cairn_check_safety_signal('{"safety":{"severity":"high"}}'::jsonb);
        ASSERT false, 'a signal with no rung must be refused';
    EXCEPTION WHEN others THEN NULL;
    END;

    BEGIN
        PERFORM cairn_check_safety_signal('{"safety":{"rung":"precise","severity":"high"}}'::jsonb);
        ASSERT false, 'a precise rung with no class must be refused';
    EXCEPTION WHEN others THEN NULL;
    END;

    BEGIN
        PERFORM cairn_check_safety_signal('{"safety":"not-an-object"}'::jsonb);
        ASSERT false, 'a non-object signal must be refused';
    EXCEPTION WHEN others THEN NULL;
    END;
END $$;

-- ---------------------------------------------------------------------------
-- 3. The class map ships EMPTY, and the shipped state is the assertion.
--
--    Cairn ships the lookup MECHANISM, never the drug knowledge: a seeded row would be an
--    un-reviewable clinical policy choice smuggled into infrastructure (principle 9). This
--    is the same assertion db/tests/048 makes about sensitivity_category_map.
-- ---------------------------------------------------------------------------
DO $$
DECLARE v_n bigint;
BEGIN
    SELECT count(*) INTO v_n FROM safety_class_map;
    ASSERT v_n = 0, 'safety_class_map must ship EMPTY (principle 9)';
END $$;

-- ---------------------------------------------------------------------------
-- 4. The read model's totality, on seeded rows.
--
--    WHY NOT submit_event: that door needs a real Ed25519-signed envelope and this rig has
--    no signing key (the same limitation db/tests/047 and db/tests/048 explain). Seeding
--    event_log directly still exercises the REAL read functions, which is what is under
--    test here. Runs inside a transaction that ROLLBACKs, so it leaves no residue.
-- ---------------------------------------------------------------------------
BEGIN;

CREATE OR REPLACE FUNCTION _safety_seed_event(
    p_patient uuid, p_type text, p_safety jsonb, p_wall bigint
) RETURNS uuid LANGUAGE plpgsql AS $$
DECLARE
    v_id    uuid  := gen_random_uuid();
    v_bytes bytea := convert_to(v_id::text || p_wall::text, 'UTF8');
BEGIN
    INSERT INTO event_log (event_id, patient_id, event_type, schema_version,
                           hlc_wall, hlc_counter, node_origin, signed_bytes,
                           content_address, body, contributors, signer_key_id,
                           plaintext_twin, safety)
    VALUES (v_id, p_patient, p_type, p_type || '/1', p_wall, 0, 'sqltest', v_bytes,
            '\x1220'::bytea || digest(v_bytes, 'sha256'), '{}'::jsonb, '[]'::jsonb,
            'kid', 'twin', p_safety);
    RETURN v_id;
END $$;

DO $$
DECLARE
    v_patient uuid := gen_random_uuid();
    v_a uuid; v_b uuid; v_c uuid;
    v_rung text; v_class text;
BEGIN
    -- A self-contradictory signal: stored verbatim, but its class must never surface.
    v_a := _safety_seed_event(v_patient, 'note.added',
        '{"rung":"existence","class":"rh-sensitizing"}'::jsonb, 100);
    SELECT rung, class INTO v_rung, v_class FROM cairn_event_safety(v_a);
    ASSERT v_rung = 'existence', 'the stored rung stands when no grade coarsens it further';
    ASSERT v_class IS NULL,
        'a class is surfaced ONLY at rung precise, whatever the row holds — this totality '
        'is what makes the apply door''s leniency safe';

    -- An unrecognised rung reads as the coarsest NAMED rung, not echoed back.
    v_b := _safety_seed_event(v_patient, 'note.added',
        '{"rung":"rung:from-a-future-peer","severity":"critical"}'::jsonb, 101);
    SELECT rung INTO v_rung FROM cairn_event_safety(v_b);
    ASSERT v_rung = 'existence', 'an unrecognised rung discloses nothing';

    -- No signal at all yields no row: an existence marker on every event would
    -- manufacture a warning from nothing.
    v_c := _safety_seed_event(v_patient, 'note.added', NULL, 102);
    ASSERT NOT EXISTS (SELECT 1 FROM cairn_event_safety(v_c)),
        'no signal means no row';
END $$;

DROP FUNCTION _safety_seed_event(uuid, text, jsonb, bigint);
ROLLBACK;
```

- [ ] **Step 2: Run the mirror**

Run: `PGHOST=127.0.0.1 PGPORT=5532 ./scripts/run-db-sql-tests.sh 2>&1 | tail -20`
Expected: every mirror passes, including `049_safety_projection_test.sql`.

Forgetting `PGHOST`/`PGPORT` runs against the wrong cluster and fails with a misleading error ([#373](https://github.com/cairn-ehr/cairn-ehr/issues/373)).

- [ ] **Step 3: Make the cairn-sync subset test DRIVE db/049**

Locate the subset suite: `grep -rn "048_sensitivity" crates/cairn-sync/tests/`. Add an assertion that
*calls* a db/049 function rather than merely checking the file loaded — the #386 lesson:

```rust
#[tokio::test]
async fn the_subset_can_actually_run_the_safety_read_model() {
    let Some(base) = conn_string() else { return };
    let c = load_subset_schema(&base).await;
    // #386's lesson: loading a migration is not the same as being able to RUN it. A subset
    // node applies clinical events through apply_remote_event, which writes event_log.safety
    // — so every function that column feeds must resolve here, not merely exist upstream.
    let rung: String = c
        .query_one("SELECT cairn_safety_rung_for_rank(cairn_sensitivity_rank('sequestered'))", &[])
        .await
        .expect("db/049's ladder must be callable on a subset node")
        .get(0);
    assert_eq!(rung, "existence");
}
```

Adapt the helper names to whatever that file already uses.

- [ ] **Step 4: Run it**

Run: `cargo test -p cairn-sync 2>&1 | tail -10`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add db/tests/049_safety_projection_test.sql crates/cairn-sync/tests/
git commit -m "test(#375): the db/049 SQL mirror, and a subset test that DRIVES it

The mirror pins both ladders, the monotone rung map, the floor's admits and
refusals, the empty class map, and the read model's totality — independently of
Rust, which is the point of the db/tests suite.

The cairn-sync subset test CALLS a db/049 function rather than only loading the
file: #386 records exactly that gap against db/048's subset test, and a subset
node writes event_log.safety on every clinical apply.

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

### Task 8: ADR-0063, the spec, and the handover docs

The *why* goes in the immutable ADR log; the *what* in the spec; the current state in the disposable scaffolding.

**Files:**
- Create: `docs/spec/decisions/0063-the-safety-projection-and-the-seal-as-coarsening-boundary.md`
- Modify: `docs/spec/decisions/README.md`, `docs/spec/identity.md` (§5.9), `docs/spec/index.md` (version 0.64 → 0.65), `docs/HANDOVER.md`, `docs/ROADMAP.md`

- [ ] **Step 1: Write ADR-0063**

Follow ADR-0062's structure exactly: front-matter (Status/Date/Derives from/Applies/Canonical spec home), Context, numbered Decisions, Rejected alternatives, Known limitations, Consequences, First instance. Carry these seven decisions, each with the reasoning from the design doc's matching section:

1. The seal boundary is the coarsening boundary (design §3).
2. Coarsening binds at emission and is re-applied at read; neither alone suffices, for *different* reasons (design §5).
3. An unrecognised severity ranks MAX; an unrecognised rung is coarsest (design §6c).
4. The signal rides the append-only row, so coarsen-but-survive is structural (design §6a).
5. `safety_class_map` ships empty (design §6b).
6. The floor is local-door only; the read model is total (design §7).
7. An uncoded medication, and a coding absent from the map, emit nothing (design §4).

**Rejected alternatives** (each with the failure it produces): precise-in-the-clear + coarsen-at-read (the leak); sealed-only (no signal on the node §5.9 is written for); a projection table (a scrub exemption someone will "fix"); refusing a malformed signal at the apply door (a de-identified advisory field cancelling clinical content).

**Known limitations:** a later grade raise cannot claw back a published rung (design §5c); the read-then-sign race (design §5a); rung-4 oblivion does not exist, so the signal is permanent today.

- [ ] **Step 2: Update the spec**

In `docs/spec/identity.md` §5.9, extend the safety-projection bullet with a *"The projection's concrete shape (ADR-0063, first built 2026-08-13…)"* sub-list mirroring how ADR-0062's shape was added — the two tiers, the three rungs, emission-binds/read-defends, the local-door-only floor, the empty class map, and what an uncoded medication does. Bump `docs/spec/index.md`'s version to **v0.65** and add ADR-0063 to `docs/spec/decisions/README.md`.

- [ ] **Step 3: Update HANDOVER and ROADMAP**

`docs/HANDOVER.md`: rewrite ⇒ NEXT so part C (#376) is the head of the §5.9 thread, add Slice 67's entry to "Recent sessions" with the carry-forward lessons (the door asymmetry and its blast-radius argument; the two coarsenings; the anti-drift pin on `cairn_prospective_sensitivity`), add part B to "Built so far", and **prune to under 500 lines**. `docs/ROADMAP.md`: add the Slice 67 narrative and prune likewise.

- [ ] **Step 4: Verify the plan gate and the docs build**

Run: `cargo test -p cairn-node --test paper_parity_plan_section 2>&1 | tail -5`
Expected: PASS — this plan carries a `## Paper-parity benchmark (§1.2)` section with all three limb labels.

Run: `uv run --with-requirements docs/requirements.txt -- mkdocs build 2>&1 | tail -5`
Expected: build succeeds with no broken-link warnings for the new ADR.

- [ ] **Step 5: Commit**

```bash
git add docs/
git commit -m "docs(#375): ADR-0063 — the safety projection and the seal as coarsening boundary

Co-Authored-By: Claude Opus 5 (1M context) <noreply@anthropic.com>"
```

---

## Paper-parity benchmark (§1.2)

**Paper counterpart.** The paper chart with a confidential episode in a sealed envelope stapled inside the folder, and an allergy/interaction sticker on the front sheet. The next clinician reads the front sheet and learns *"there is something here that interacts; the detail is in the envelope"* — without opening it, and without needing anyone's permission.

**Steps.** Paper `N = 1` human act (read the front sheet). Architecture-forced `M = 0` additional human acts: the class is captured from the coding type-ahead the clinician is already using (ADR-0059's coding benchmark is itself `M = N`), the rung is chosen automatically at write from the standing grade, and the signal is read at chart open. UI bundling target `K = 0` — the signal must appear *with* the chart, never behind a click. `M < N`, so there is no architecture defect to file under house rule 7.

**Time + cognitive load.** Budget: the signal is on screen within the med-list's existing chart-open budget with **no additional round trip** (`cairn_patient_safety` is one query on the same connection), and adds **zero** clicks. Cognitive load must *fall* versus paper: the front-sheet sticker requires the clinician to notice and interpret a mark, whereas `render_safety_line` composes a sentence naming the severity. **Measurement is owed by the UI slice that first renders it** — this slice exposes only a CLI verb, which no clinician uses at the point of care. If either the query cost or the click count exceeds budget there, that is the finding: file it, do not adjust the budget.

---

## Final gate (run before opening the PR)

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings`
- [ ] `PGHOST=127.0.0.1 PGPORT=5532 ./scripts/run-db-gated-tests.sh` — the mirrors **and** the full workspace with all three connection strings. Not `-p cairn-node`: a per-crate run misses cross-crate call-site breakage in `cairn-sync/tests/clinical_pull.rs`.
- [ ] Confirm the four registry row-count pins are untouched: `twin_registry.rs`, `db/tests/034`, `projection_registry.rs`, `db/tests/039`. If any went red, something was registered that should not have been.
- [ ] `cargo deny check` in both `deny.toml` trees.
- [ ] A killed test binary exits 101 with **zero** `test result: FAILED` lines — if the run dies that way it is the macOS `_dyld_start` loader stall, not a failure. Diagnose with `sample <pid>`, `kill -9`, retry.

## Follow-ons to file as GitHub issues

1. **The deployment-configurable grade→rung map** — §5.9's *policy-configured* ladder, deferred as surface without a caller. Any override must stay monotone non-decreasing in rank; that constraint is mechanism, not policy.
2. **Rung-4 oblivion for the safety signal** — §5.9 says the projection is shreddable only at rung 4. Today it is permanent, because rung 4 does not exist.
3. **The UI warning surface and its §1.2 measurement** — the budget this plan states but cannot measure.
4. **drugref populates `safety_class_map`** — the natural consumer of the empty-map seam, part of the term→anchor lookup slice.
5. **Non-medication clinical streams** — the emission seam is generic, but only medication verbs carry a coding today.
6. **`crates/cairn-node/src/main.rs` is 3.3k lines** — over the house 500-line guideline, the same shape as [#329](https://github.com/cairn-ehr/cairn-ehr/issues/329) for `cairn-sync/src/main.rs`. This slice adds one more verb to it.
