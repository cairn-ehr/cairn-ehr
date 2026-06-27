# Demographic Identifier Assertion Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the §4.4 patient-identifier demographic assertion end-to-end on `cairn-node` — author (pure Rust) → in-DB structural floor → set-union projection — as the first production clinical surface and the spine the other demographic fields reuse.

**Architecture:** A new additive event type `demographic.identifier.asserted` flows through the existing validated `submit_event` door. The §4.5 authored legibility twin rides in the signed body (new additive `EventBody` field); a new in-DB floor helper enforces the §4.4 culture-neutral structural invariants; a `patient_identifier` projection table is maintained set-union by an `AFTER INSERT` trigger. Matching/veto, CLI, and globalised authored-twin are out of scope (see spec).

**Tech Stack:** Rust (`cairn-event` pure functions, `cairn-node` integration tests, `tokio-postgres`), PostgreSQL ≥ 18 PL/pgSQL (`db/010`), the `cairn_pgx` in-DB verify gate (unchanged).

## Global Constraints

- **License:** AGPL-3.0; every dependency AGPL-3.0-compatible (no new dependencies in this plan).
- **TDD:** failing test first, then minimal code. Load-bearing on the in-DB floor (the safety-critical surface, §9).
- **Inline docs** for a junior contributor on every non-trivial function — *why/how it fits*, not just *what*.
- **File size:** aim < 500 lines; no unrelated refactor of the already-773-line `cairn-event/src/lib.rs` (new Rust goes in a new `demographics` module).
- **Substrate:** `cairn-node` already loads `db/001–006` + node tier via `crates/cairn-node/src/db.rs` `SCHEMA`; demographics is a new `db/010` migration appended to that array.
- **DB-gated tests** require `$CAIRN_TEST_PG` (a PG ≥ 18 with `cairn_pgx` installed) and self-serialize via `db::test_serial_guard`. They no-op (print "skipped") when the env var is absent.
- **Event-type name:** `demographic.identifier.asserted` · **schema_version:** `demographic.identifier/1`.
- **Twin scope:** authored-twin carry + floor-check is **demographics-only** in this slice; legacy event types keep deriving their skeleton twin.

---

### Task 1: Additive `EventBody.plaintext_twin` field

**Files:**
- Modify: `crates/cairn-event/src/lib.rs` (the `EventBody` struct, ~line 76-89; tests at end)
- Modify (mechanical): every `EventBody { … }` literal — `crates/cairn-sync/src/main.rs`, `crates/cairn-node/src/identity.rs`, `crates/cairn-node/src/restore.rs`, `crates/cairn-node/src/medium.rs`, `crates/cairn-node/tests/restore.rs`, `crates/cairn-node/tests/admission.rs`, `crates/cairn-node/tests/attestation.rs`

**Interfaces:**
- Produces: `EventBody.plaintext_twin: Option<String>` — the §4.5 authored twin, carried in the signed body. Absent (`None`) ⇒ omitted from the wire encoding (no content-address change vs. the pre-field shape).

- [ ] **Step 1: Write the failing tests** (append to the `#[cfg(test)] mod tests` in `crates/cairn-event/src/lib.rs`)

```rust
    // A None authored-twin must NOT change the wire bytes vs. the pre-field shape,
    // so every existing event's content-address is preserved (append-only, principle 1).
    #[test]
    fn twin_absent_is_wire_identical_to_pre_field_shape() {
        #[derive(serde::Serialize)]
        struct LegacyBody<'a> {
            event_id: &'a str, patient_id: &'a str, event_type: &'a str,
            schema_version: &'a str, hlc: &'a Hlc, t_effective: Option<String>,
            signer_key_id: &'a str, contributors: &'a serde_json::Value,
            payload: &'a serde_json::Value, attachments: &'a Vec<AttachmentRef>,
        }
        let hlc = Hlc { wall: 1, counter: 0, node_origin: "n".into() };
        let contributors = serde_json::json!([{"actor_id": "k", "role": "triaged"}]);
        let payload = serde_json::json!({"text": "hi"});
        let attachments: Vec<AttachmentRef> = vec![];
        let legacy = LegacyBody {
            event_id: "e", patient_id: "p", event_type: "note.added",
            schema_version: "advisory/1", hlc: &hlc, t_effective: None,
            signer_key_id: "k", contributors: &contributors, payload: &payload,
            attachments: &attachments,
        };
        let body = EventBody {
            event_id: "e".into(), patient_id: "p".into(), event_type: "note.added".into(),
            schema_version: "advisory/1".into(), hlc: hlc.clone(), t_effective: None,
            signer_key_id: "k".into(), contributors: contributors.clone(),
            payload: payload.clone(), attachments: vec![], plaintext_twin: None,
        };
        let mut legacy_bytes = Vec::new();
        ciborium::into_writer(&legacy, &mut legacy_bytes).unwrap();
        assert_eq!(canonical_cbor(&body).unwrap(), legacy_bytes,
                   "None twin must encode byte-identically to the pre-field shape");
    }

    // Bytes authored before the field existed must still decode (forward-compat).
    #[test]
    fn legacy_bytes_decode_with_twin_none() {
        let body = EventBody {
            event_id: "e".into(), patient_id: "p".into(), event_type: "note.added".into(),
            schema_version: "advisory/1".into(),
            hlc: Hlc { wall: 1, counter: 0, node_origin: "n".into() }, t_effective: None,
            signer_key_id: "k".into(),
            contributors: serde_json::json!([]), payload: serde_json::json!({}),
            attachments: vec![], plaintext_twin: None,
        };
        let bytes = canonical_cbor(&body).unwrap();
        let decoded: EventBody = ciborium::from_reader(&bytes[..]).unwrap();
        assert_eq!(decoded.plaintext_twin, None);
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p cairn-event twin_absent_is_wire_identical_to_pre_field_shape legacy_bytes_decode_with_twin_none`
Expected: FAIL — `EventBody` has no field `plaintext_twin`.

- [ ] **Step 3: Add the field** — in `crates/cairn-event/src/lib.rs`, append to `EventBody` **after** `attachments` (last field, so positional/array CBOR stays additive):

```rust
    /// The §4.5 materialised legibility twin, authored into the signed body. Absent
    /// (None) for legacy event types whose twin submit_event still derives; present
    /// for demographic assertions, where the in-DB floor (db/010) requires it.
    /// `skip_serializing_if` ⇒ a None twin is omitted from the wire, so adding this
    /// field never changes an existing event's bytes/content-address (additive-only,
    /// principle 11 / ADR-0012).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plaintext_twin: Option<String>,
```

- [ ] **Step 4: Fix every `EventBody { … }` literal** — add `plaintext_twin: None,` to each. Find them all:

Run: `rg -n "EventBody \{" crates/ --type rust | grep -v "pub struct"`
Add `plaintext_twin: None,` as the last field in each literal (the demographic tests in Task 3/4 will set `Some(...)`).

- [ ] **Step 5: Run the full cairn-event + cairn-node build/tests to verify green**

Run: `cargo test -p cairn-event && cargo build -p cairn-node --tests`
Expected: PASS / builds clean (all literals updated).

- [ ] **Step 6: Commit**

```bash
git add crates/cairn-event/src/lib.rs crates/cairn-sync crates/cairn-node
git commit -m "feat(event): additive EventBody.plaintext_twin (§4.5 authored twin carrier)"
```

---

### Task 2: Pure identifier-assertion builders (`cairn-event::demographics`)

**Files:**
- Create: `crates/cairn-event/src/demographics.rs`
- Modify: `crates/cairn-event/src/lib.rs` (add `pub mod demographics;` near the top, after the existing `use`/module declarations)

**Interfaces:**
- Produces:
  - `struct IdentifierAssertion<'a> { value, system, provenance: &str; normalized, profile, use_: Option<&str> }`
  - `fn identifier_assertion_body(a: &IdentifierAssertion) -> serde_json::Value` — the §4.4 payload; optional facets omitted (never null) when absent.
  - `fn render_identifier_twin(a: &IdentifierAssertion) -> String` — `"<system>, <provenance>: <value>"`.

- [ ] **Step 1: Write the failing tests** — create `crates/cairn-event/src/demographics.rs` with only the tests first:

```rust
//! Demographic assertion builders (spec §4). Slice 1: the §4.4 patient
//! **identifier** assertion. Pure: explicit inputs, no I/O, no DB. The
//! safety-critical structural floor lives in the database (db/010); these
//! functions only shape and render the event a node will sign.

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> IdentifierAssertion<'static> {
        IdentifierAssertion {
            value: "943 476 5919", system: "nhs-number",
            provenance: "document-verified",
            normalized: Some("9434765919"), profile: Some("nhs-number@b3-abc"),
            use_: Some("national-id"),
        }
    }

    #[test]
    fn body_includes_all_facets_when_present() {
        let v = identifier_assertion_body(&sample());
        assert_eq!(v["field"], "identifier");
        assert_eq!(v["value"], "943 476 5919");
        assert_eq!(v["system"], "nhs-number");
        assert_eq!(v["provenance"], "document-verified");
        assert_eq!(v["normalized"], "9434765919");
        assert_eq!(v["profile"], "nhs-number@b3-abc");
        assert_eq!(v["use"], "national-id");
    }

    #[test]
    fn body_omits_absent_optional_facets_never_null() {
        let a = IdentifierAssertion {
            value: "X1", system: "unknown", provenance: "patient-stated",
            normalized: None, profile: None, use_: None,
        };
        let v = identifier_assertion_body(&a);
        let obj = v.as_object().unwrap();
        assert!(!obj.contains_key("normalized"), "absent facet must be omitted, not null");
        assert!(!obj.contains_key("profile"));
        assert!(!obj.contains_key("use"));
    }

    #[test]
    fn twin_renders_profile_independent_plaintext() {
        assert_eq!(
            render_identifier_twin(&sample()),
            "nhs-number, document-verified: 943 476 5919"
        );
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p cairn-event demographics`
Expected: FAIL — `IdentifierAssertion` / functions not defined.

- [ ] **Step 3: Add `pub mod demographics;` to `lib.rs`** and write the minimal implementation at the top of `demographics.rs` (above the test module):

```rust
use serde_json::{json, Value};

/// One §4.4 identifier assertion. `normalized` present without a `profile` is
/// rejected by the in-DB floor (db/010), so a caller materialising a normalized
/// form must also name the profile that produced it (the §4.4 materialised-key rule).
pub struct IdentifierAssertion<'a> {
    pub value: &'a str,      // §4.4 mandatory — as-entered, never rewritten
    pub system: &'a str,     // §4.4 mandatory — stable namespace (or the literal "unknown")
    pub provenance: &'a str, // §4.1 provenance ladder — required-present, value-open
    pub normalized: Option<&'a str>, // §4.4 — materialised matching key when a profile is present
    pub profile: Option<&'a str>,    // §4.4 — namespace@hash validator-bundle reference
    pub use_: Option<&'a str>,       // §4.4 — recommended-but-open use/type vocabulary
}

/// Build the §4.4 identifier-assertion payload (the value of `EventBody.payload`).
/// Optional facets are omitted entirely when absent — never serialized as null —
/// so the in-DB floor's key-presence checks see exactly what the author asserted.
pub fn identifier_assertion_body(a: &IdentifierAssertion) -> Value {
    let mut p = json!({
        "field": "identifier",
        "provenance": a.provenance,
        "value": a.value,
        "system": a.system,
    });
    let obj = p.as_object_mut().expect("json! built an object");
    if let Some(n) = a.normalized { obj.insert("normalized".into(), json!(n)); }
    if let Some(pr) = a.profile   { obj.insert("profile".into(),    json!(pr)); }
    if let Some(u) = a.use_       { obj.insert("use".into(),        json!(u)); }
    p
}

/// Render the §4.5 materialised legibility twin: profile-independent plaintext,
/// `"<system>, <provenance>: <value>"`. The namespace is always legible without a
/// registry; a human-friendly system label is a UI-layer refinement, not floor data.
pub fn render_identifier_twin(a: &IdentifierAssertion) -> String {
    format!("{}, {}: {}", a.system, a.provenance, a.value)
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p cairn-event demographics`
Expected: PASS (3 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/cairn-event/src/lib.rs crates/cairn-event/src/demographics.rs
git commit -m "feat(event): pure §4.4 identifier-assertion body + §4.5 twin builders"
```

---

### Task 3: The `db/010` migration + happy-path / set-union / degradation proof

**Files:**
- Create: `db/010_demographics.sql`
- Modify: `crates/cairn-node/src/db.rs` (add `010_demographics` to the `SCHEMA` array)
- Create: `crates/cairn-node/tests/demographics.rs`

**Interfaces:**
- Consumes: `cairn_event::demographics::{IdentifierAssertion, identifier_assertion_body, render_identifier_twin}`; `cairn_event::{sign, generate_key, EventBody, Hlc}`; `cairn_node::db`.
- Produces: SQL function `cairn_check_identifier_assertion(b jsonb) returns void`; table `patient_identifier`; the `demographic.identifier.asserted` classification; a `submit_event` that carries the authored twin for demographic events. Test helper `assert_identifier(...)` reused by Task 4.

- [ ] **Step 1: Write the migration `db/010_demographics.sql`**

```sql
-- Cairn — demographic identifier assertions (spec §4.1/§4.4/§4.5, ADR-0033/0034).
--
-- The first production clinical surface. Adds the `demographic.identifier.asserted`
-- event type, the §4.4 structural floor (culture-neutral: no profile, no checksum,
-- no format validation — those are advisory and live above the floor), the §4.5
-- authored-twin carry through submit_event, and a set-union `patient_identifier`
-- projection. Matching/veto (§5.2) is a separate, later subsystem and NOT here.

BEGIN;

-- Additive registration: a new event type adds a row (fail-closed registry, ADR-0010).
INSERT INTO event_type_class (event_type, mode, targets_other_author) VALUES
    ('demographic.identifier.asserted', 'additive', FALSE)
ON CONFLICT (event_type) DO NOTHING;

-- The §4.4 structural floor. Enforces ONLY culture-neutral invariants; never holds a
-- profile, runs a checksum, or validates a format (those flag-not-reject above the
-- floor — principle 12 / §4.4). Each violation is a distinct legible exception.
CREATE OR REPLACE FUNCTION cairn_check_identifier_assertion(b jsonb)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    p jsonb := b -> 'payload';
BEGIN
    IF p IS NULL THEN
        RAISE EXCEPTION 'identifier assertion: missing payload';
    END IF;
    -- value: present, string, non-empty (§4.4 mandatory, the evidence facet).
    IF jsonb_typeof(p -> 'value') IS DISTINCT FROM 'string'
       OR length(trim(p ->> 'value')) = 0 THEN
        RAISE EXCEPTION 'identifier assertion: value must be a non-empty string (§4.4 mandatory)';
    END IF;
    -- system: present, string, non-empty (§4.4 mandatory; may be the literal "unknown").
    IF jsonb_typeof(p -> 'system') IS DISTINCT FROM 'string'
       OR length(trim(p ->> 'system')) = 0 THEN
        RAISE EXCEPTION 'identifier assertion: system must be a non-empty string (§4.4 mandatory)';
    END IF;
    -- provenance: present, string, non-empty (§4.1 ladder; value-open, "unknown" is honest).
    IF jsonb_typeof(p -> 'provenance') IS DISTINCT FROM 'string'
       OR length(trim(p ->> 'provenance')) = 0 THEN
        RAISE EXCEPTION 'identifier assertion: provenance must be a non-empty string (§4.1)';
    END IF;
    -- normalized: optional; when present must be a string AND name a profile
    -- (the §4.4 materialised-key rule: a materialised matching key needs the bundle
    -- that produced it, so a profile-less node can trust it).
    IF (p ? 'normalized') AND (p -> 'normalized') IS DISTINCT FROM 'null'::jsonb THEN
        IF jsonb_typeof(p -> 'normalized') IS DISTINCT FROM 'string' THEN
            RAISE EXCEPTION 'identifier assertion: normalized must be a string when present (§4.4)';
        END IF;
        IF jsonb_typeof(p -> 'profile') IS DISTINCT FROM 'string'
           OR length(trim(p ->> 'profile')) = 0 THEN
            RAISE EXCEPTION 'identifier assertion: normalized materialised requires a named profile (§4.4)';
        END IF;
    END IF;
END;
$$;

-- The §4.2 set-union projection: one row per (patient, system, match_key). Identifiers
-- are set-union, never LWW — first-seen wins, re-assertion is a no-op, same-system /
-- different-normalized keeps BOTH rows (the veto SIGNAL preserved as data; the veto
-- itself is out of scope). `use` is a reserved word, so the column is `use_type`.
CREATE TABLE IF NOT EXISTS patient_identifier (
    patient_id         UUID    NOT NULL,
    system             TEXT    NOT NULL,
    match_key          TEXT    NOT NULL,   -- coalesce(normalized, value)
    value              TEXT    NOT NULL,
    normalized         TEXT,
    profile            TEXT,
    use_type           TEXT,
    provenance         TEXT    NOT NULL,
    asserted_hlc_wall  BIGINT  NOT NULL,
    asserted_hlc_count INTEGER NOT NULL,
    asserted_origin    TEXT    NOT NULL,
    first_seen         TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (patient_id, system, match_key)
);

-- Incremental set-union maintenance: fold exactly the one new identifier event into
-- the projection. event_log.body holds b->'payload' (see db/005 submit_event INSERT).
CREATE OR REPLACE FUNCTION patient_identifier_apply()
RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE
    p    jsonb := NEW.body;
    norm text  := NULLIF(p ->> 'normalized', '');
BEGIN
    INSERT INTO patient_identifier
        (patient_id, system, match_key, value, normalized, profile, use_type,
         provenance, asserted_hlc_wall, asserted_hlc_count, asserted_origin)
    VALUES
        (NEW.patient_id, p ->> 'system', COALESCE(norm, p ->> 'value'),
         p ->> 'value', norm, p ->> 'profile', p ->> 'use', p ->> 'provenance',
         NEW.hlc_wall, NEW.hlc_counter, NEW.node_origin)
    ON CONFLICT (patient_id, system, match_key) DO NOTHING;
    RETURN NULL;
END;
$$;

DROP TRIGGER IF EXISTS patient_identifier_apply_trg ON event_log;
CREATE TRIGGER patient_identifier_apply_trg
    AFTER INSERT ON event_log
    FOR EACH ROW WHEN (NEW.event_type = 'demographic.identifier.asserted')
    EXECUTE FUNCTION patient_identifier_apply();

GRANT SELECT ON patient_identifier TO cairn_agent;

COMMIT;
```

- [ ] **Step 2: Re-declare `submit_event` to carry the authored twin** — append to `db/010_demographics.sql` **before** the `COMMIT;` a `CREATE OR REPLACE FUNCTION submit_event(...)` that is the `db/005` body with exactly one change: replace the twin derivation (db/005 line ~119-120) with the demographic branch. Copy the full current `submit_event` from `db/005_submit.sql` and substitute step 7:

```sql
    -- 7. Twin (§4.5) + floor. Demographic assertions carry the AUTHORED twin and pass
    --    the §4.4 structural floor; legacy types keep the derived skeleton twin.
    IF v_type = 'demographic.identifier.asserted' THEN
        PERFORM cairn_check_identifier_assertion(b);
        v_twin := b ->> 'plaintext_twin';
        IF v_twin IS NULL OR length(trim(v_twin)) = 0 THEN
            RAISE EXCEPTION 'submit_event: demographic assertion requires a non-empty authored twin (§4.5)';
        END IF;
    ELSE
        v_twin := format('[%s] %s for patient %s', v_type, b ->> 'schema_version', b ->> 'patient_id');
    END IF;
```

(Keep the rest of `submit_event` — verify, actor resolve, classify, attestation gate, target gate, provenance binding, INSERT, idempotency, attachment learning — byte-for-byte from db/005. The GRANT/REVOKE block at the end of db/005 is unaffected; do not restate it here.)

- [ ] **Step 3: Register the migration** — in `crates/cairn-node/src/db.rs`, extend the `SCHEMA` array length to `9` and add after the `009` entry:

```rust
    ("010_demographics",  include_str!("../../../db/010_demographics.sql")),
```
(Update `const SCHEMA: [(&str, &str); 8]` → `; 9]`.)

- [ ] **Step 4: Write the happy-path / set-union / degradation tests** — create `crates/cairn-node/tests/demographics.rs`:

```rust
//! Integration coverage for the §4.4 demographic identifier assertion: the in-DB
//! floor + the set-union patient_identifier projection. Real Postgres, gated on
//! $CAIRN_TEST_PG, serialized cluster-wide via db::test_serial_guard (the shared-DB
//! + TRUNCATE pattern, identical to attestation.rs). Matching/veto is a separate
//! subsystem and is NOT exercised here.
use cairn_event::demographics::{identifier_assertion_body, render_identifier_twin, IdentifierAssertion};
use cairn_event::{generate_key, sign, EventBody, Hlc, SigningKey};
use cairn_node::db;
use tokio_postgres::Client;
use uuid::Uuid;

fn cs() -> Option<String> { std::env::var("CAIRN_TEST_PG").ok() }

/// Truncate the clinical tables and enroll one agent signer. Returns (sk, kid).
async fn setup(c: &Client) -> (SigningKey, String) {
    c.batch_execute("TRUNCATE event_log, actor_event, patient_chart, patient_identifier CASCADE")
        .await.unwrap();
    let (sk, kid) = generate_key().unwrap();
    c.execute(
        "SELECT enroll_actor('agent', '{\"model\":\"reg-stub\",\"version\":\"1\",\"skill_epoch\":\"e\"}', $1)",
        &[&kid],
    ).await.unwrap();
    (sk, kid)
}

/// Author + sign + submit one §4.4 identifier assertion for `patient`. Returns the
/// raw submit result so rejection tests (Task 4) can assert the error.
async fn assert_identifier(
    c: &Client, sk: &SigningKey, kid: &str, patient: Uuid, wall: i64,
    a: &IdentifierAssertion<'_>,
) -> Result<u64, tokio_postgres::Error> {
    let body = EventBody {
        event_id: Uuid::now_v7().to_string(),
        patient_id: patient.to_string(),
        event_type: "demographic.identifier.asserted".into(),
        schema_version: "demographic.identifier/1".into(),
        hlc: Hlc { wall, counter: 0, node_origin: "n".into() },
        t_effective: None,
        signer_key_id: kid.into(),
        contributors: serde_json::json!([{"actor_id": kid, "role": "recorded"}]),
        payload: identifier_assertion_body(a),
        attachments: vec![],
        plaintext_twin: Some(render_identifier_twin(a)),
    };
    let signed = sign(&body, sk).unwrap();
    c.execute("SELECT submit_event($1)", &[&signed.signed_bytes]).await
}

#[tokio::test]
async fn happy_path_appends_and_projects_with_authored_twin() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let p = Uuid::now_v7();
    let a = IdentifierAssertion {
        value: "943 476 5919", system: "nhs-number", provenance: "document-verified",
        normalized: Some("9434765919"), profile: Some("nhs-number@b3-abc"),
        use_: Some("national-id"),
    };
    assert_identifier(&c, &sk, &kid, p, 1, &a).await.expect("valid assertion accepted");

    // Projection: one row, keyed on the normalized match_key.
    let row = c.query_one(
        "SELECT match_key, value, profile, provenance, plaintext_twin
           FROM patient_identifier pi JOIN event_log el ON el.patient_id = pi.patient_id
          WHERE pi.patient_id = $1", &[&p]).await.unwrap();
    let match_key: String = row.get(0);
    let value: String = row.get(1);
    let twin: String = row.get(4);
    assert_eq!(match_key, "9434765919");
    assert_eq!(value, "943 476 5919");
    assert_eq!(twin, "nhs-number, document-verified: 943 476 5919",
               "the AUTHORED twin is stored (cairn_body passed the top-level field through)");
}

#[tokio::test]
async fn set_union_dedups_same_key_keeps_different_key() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let p = Uuid::now_v7();
    let same = IdentifierAssertion {
        value: "943 476 5919", system: "nhs-number", provenance: "patient-stated",
        normalized: Some("9434765919"), profile: Some("nhs-number@b3-abc"), use_: None,
    };
    let same_formatted = IdentifierAssertion { value: "9434765919", ..same_clone(&same) };
    let other = IdentifierAssertion {
        value: "111 222 3334", system: "nhs-number", provenance: "patient-stated",
        normalized: Some("1112223334"), profile: Some("nhs-number@b3-abc"), use_: None,
    };
    assert_identifier(&c, &sk, &kid, p, 1, &same).await.unwrap();
    assert_identifier(&c, &sk, &kid, p, 2, &same_formatted).await.unwrap(); // same normalized → dedup
    assert_identifier(&c, &sk, &kid, p, 3, &other).await.unwrap();          // different normalized → 2nd row
    let n: i64 = c.query_one(
        "SELECT count(*) FROM patient_identifier WHERE patient_id=$1 AND system='nhs-number'",
        &[&p]).await.unwrap().get(0);
    assert_eq!(n, 2, "same-normalized dedups; different-normalized keeps both");
}

// Helper: clone an IdentifierAssertion's borrowed fields (test-only convenience).
fn same_clone<'a>(a: &IdentifierAssertion<'a>) -> IdentifierAssertion<'a> {
    IdentifierAssertion {
        value: a.value, system: a.system, provenance: a.provenance,
        normalized: a.normalized, profile: a.profile, use_: a.use_,
    }
}

#[tokio::test]
async fn honest_degradation_no_normalized_no_profile_accepted() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let p = Uuid::now_v7();
    let a = IdentifierAssertion {
        value: "OLD-CARD-77", system: "unknown", provenance: "imported",
        normalized: None, profile: None, use_: None,
    };
    assert_identifier(&c, &sk, &kid, p, 1, &a).await.expect("profile-less assertion accepted");
    let mk: String = c.query_one(
        "SELECT match_key FROM patient_identifier WHERE patient_id=$1", &[&p])
        .await.unwrap().get(0);
    assert_eq!(mk, "OLD-CARD-77", "match_key falls back to value when normalized absent");
}
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `CAIRN_TEST_PG="$CAIRN_TEST_PG" cargo test -p cairn-node --test demographics`
Expected: PASS (3 tests; or all "skipped" if `$CAIRN_TEST_PG` unset — set it first).

- [ ] **Step 6: Commit**

```bash
git add db/010_demographics.sql crates/cairn-node/src/db.rs crates/cairn-node/tests/demographics.rs
git commit -m "feat(db): §4.4 identifier floor + set-union patient_identifier projection"
```

---

### Task 4: Floor rejection coverage + legacy regression (the safety gate)

**Files:**
- Modify: `crates/cairn-node/tests/demographics.rs` (add the rejection + regression tests)

**Interfaces:**
- Consumes: the `setup`, `assert_identifier`, `same_clone`, `cs` helpers from Task 3.

- [ ] **Step 1: Write the failing/red rejection tests** — append to `crates/cairn-node/tests/demographics.rs`. Each asserts the submit errors AND nothing is written:

```rust
/// Submit a raw body (bypassing the typed builder) so we can author floor-violating
/// payloads the safe builder would never produce. Returns the submit result.
async fn submit_raw_demographic(
    c: &Client, sk: &SigningKey, kid: &str, patient: Uuid,
    payload: serde_json::Value, twin: Option<&str>,
) -> Result<u64, tokio_postgres::Error> {
    let body = EventBody {
        event_id: Uuid::now_v7().to_string(),
        patient_id: patient.to_string(),
        event_type: "demographic.identifier.asserted".into(),
        schema_version: "demographic.identifier/1".into(),
        hlc: Hlc { wall: 1, counter: 0, node_origin: "n".into() },
        t_effective: None, signer_key_id: kid.into(),
        contributors: serde_json::json!([{"actor_id": kid, "role": "recorded"}]),
        payload, attachments: vec![],
        plaintext_twin: twin.map(|t| t.to_string()),
    };
    let signed = sign(&body, sk).unwrap();
    c.execute("SELECT submit_event($1)", &[&signed.signed_bytes]).await
}

async fn assert_rejected_and_empty(
    c: &Client, sk: &SigningKey, kid: &str, p: Uuid,
    payload: serde_json::Value, twin: Option<&str>, label: &str,
) {
    let r = submit_raw_demographic(c, sk, kid, p, payload, twin).await;
    assert!(r.is_err(), "{label}: must be rejected by the floor");
    let n: i64 = c.query_one("SELECT count(*) FROM event_log WHERE patient_id=$1", &[&p])
        .await.unwrap().get(0);
    assert_eq!(n, 0, "{label}: nothing appended to event_log");
    let m: i64 = c.query_one("SELECT count(*) FROM patient_identifier WHERE patient_id=$1", &[&p])
        .await.unwrap().get(0);
    assert_eq!(m, 0, "{label}: nothing projected");
}

#[tokio::test]
async fn floor_rejects_each_invariant_violation() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let good_twin = Some("nhs-number, document-verified: x");

    // value empty
    assert_rejected_and_empty(&c, &sk, &kid, Uuid::now_v7(),
        serde_json::json!({"field":"identifier","value":"","system":"nhs-number","provenance":"x"}),
        good_twin, "value-empty").await;
    // system missing
    assert_rejected_and_empty(&c, &sk, &kid, Uuid::now_v7(),
        serde_json::json!({"field":"identifier","value":"v","provenance":"x"}),
        good_twin, "system-missing").await;
    // provenance missing
    assert_rejected_and_empty(&c, &sk, &kid, Uuid::now_v7(),
        serde_json::json!({"field":"identifier","value":"v","system":"nhs-number"}),
        good_twin, "provenance-missing").await;
    // normalized non-text
    assert_rejected_and_empty(&c, &sk, &kid, Uuid::now_v7(),
        serde_json::json!({"field":"identifier","value":"v","system":"s","provenance":"x","normalized":123,"profile":"p@h"}),
        good_twin, "normalized-non-text").await;
    // normalized without profile (the materialised-key rule)
    assert_rejected_and_empty(&c, &sk, &kid, Uuid::now_v7(),
        serde_json::json!({"field":"identifier","value":"v","system":"s","provenance":"x","normalized":"vv"}),
        good_twin, "normalized-without-profile").await;
    // empty authored twin
    assert_rejected_and_empty(&c, &sk, &kid, Uuid::now_v7(),
        serde_json::json!({"field":"identifier","value":"v","system":"s","provenance":"x"}),
        Some(""), "empty-twin").await;
}

#[tokio::test]
async fn legacy_patient_created_still_uses_derived_twin() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let p = Uuid::now_v7();
    // A legacy additive event with NO authored twin must still be accepted and get
    // the derived skeleton twin (the demographics-only twin scope, regression guard).
    let body = EventBody {
        event_id: Uuid::now_v7().to_string(), patient_id: p.to_string(),
        event_type: "patient.created".into(), schema_version: "demo/1".into(),
        hlc: Hlc { wall: 1, counter: 0, node_origin: "n".into() }, t_effective: None,
        signer_key_id: kid.clone(),
        contributors: serde_json::json!([{"actor_id": kid, "role": "recorded"}]),
        payload: serde_json::json!({"name":"A B","dob":"1980","sex":"x"}),
        attachments: vec![], plaintext_twin: None,
    };
    let signed = sign(&body, &sk).unwrap();
    c.execute("SELECT submit_event($1)", &[&signed.signed_bytes]).await
        .expect("legacy event with no authored twin still accepted");
    let twin: String = c.query_one(
        "SELECT plaintext_twin FROM event_log WHERE event_id=$1",
        &[&Uuid::parse_str(&body.event_id).unwrap()]).await.unwrap().get(0);
    assert!(twin.starts_with("[patient.created]"), "legacy derives the skeleton twin");
}
```

- [ ] **Step 2: Run to verify they pass** (the floor written in Task 3 should already satisfy them; if any rejection slips through, fix `cairn_check_identifier_assertion` / the twin gate in `db/010`)

Run: `cargo test -p cairn-node --test demographics`
Expected: PASS (all 5 tests in the file).

- [ ] **Step 3: Run the whole node suite for regressions**

Run: `cargo test -p cairn-node && cargo test -p cairn-event && cargo clippy --workspace --all-targets`
Expected: PASS, clippy clean.

- [ ] **Step 4: Commit**

```bash
git add crates/cairn-node/tests/demographics.rs
git commit -m "test(db): §4.4 floor rejections + legacy derived-twin regression"
```

---

## Self-Review

**Spec coverage** (design doc → tasks):
- §4.4 payload facets (value/system/normalized/profile/use) → Task 2 builder, Task 3 floor + projection ✓
- §4.5 authored twin materialised in the signed body → Task 1 (field) + Task 2 (render) + Task 3 (carry/floor) ✓
- §4.4 floor invariants (value/system/provenance/normalized-text/normalized⇒profile) → Task 3 helper + Task 4 rejections ✓
- §4.2 set-union projection (dedup same key, keep different) → Task 3 set-union test ✓
- Honest degradation (no normalized + no profile accepted) → Task 3 ✓
- `cairn_body` passthrough of the new top-level field → Task 3 happy-path twin assertion ✓
- Additive CBOR (no address change) → Task 1 ✓
- Legacy regression (derived twin still works) → Task 4 ✓
- Out-of-scope (matching/veto, CLI, global twin) → not implemented, per design ✓

**Placeholder scan:** none — every step has concrete code/commands.

**Type consistency:** `IdentifierAssertion` fields (`value/system/provenance/normalized/profile/use_`) match across Tasks 2–4; `identifier_assertion_body`/`render_identifier_twin` signatures consistent; SQL `match_key = coalesce(normalized,value)` consistent between the trigger (Task 3) and the assertions in the tests; `EventBody.plaintext_twin: Option<String>` used consistently (Task 1 definition; `Some`/`None` at call sites).

**Note for the implementer:** Task 3 Step 2 requires copying the current `submit_event` body verbatim from `db/005_submit.sql` and changing only step 7. Read `db/005_submit.sql` in full before writing `db/010` so the re-declaration is byte-faithful except for the documented branch.
