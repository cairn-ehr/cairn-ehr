# C1 — Identity linkage core Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the authoritative §5.1/§5.7 identity linkage core — `link`/`unlink` event types through the existing `submit_event` door, an HLC-overlay `patient_link` edge table, and a `person_member` connected-component ("golden identity") projection with clean unmerge — plus pure Rust builders and a thin demonstrated unified-read VIEW.

**Architecture:** Two new **additive** event types register in `event_type_class` and add a branch to the `cairn_event_twin` hook — the `submit_event` safety door (db/005) is reused verbatim, never re-declared. A trigger-maintained `patient_link` overlay table (latest HLC wins the edge state) feeds a `person_member` projection recomputed per-touched-component by a bounded recursive walk, with a loud oversize guard. Matcher-independent; carries principle 2 ("unmerge is always clean").

**Tech Stack:** PostgreSQL ≥ 18 + PL/pgSQL (in-DB safety-critical floor + projections), Rust (`cairn-event` pure builders; `cairn-node` tokio-postgres integration tests), `cairn_pgx` (COSE/Ed25519 verify, already installed).

## Global Constraints

- **AGPL-3.0**; every dependency AGPL-3.0-compatible. No new dependency is added by this plan.
- **TDD** — failing test first, then minimal code. Load-bearing: this is the §9 safety-critical surface.
- **Reviewer-legible inline docs** for a junior contributor — *why* and *how it fits*, not just *what*.
- **Files under ~500 lines** where feasible; `db/018_identity_linkage.sql` is one focused file.
- **Culture-neutral floor** — the in-DB floor validates structure only (valid UUIDs, distinct, non-empty provenance); no name/date/locale assumptions.
- **Additive only** — no `submit_event` re-declaration; no change to existing event types; new types fail-closed if unregistered.
- **Omit-when-absent** — optional payload facets (`confidence`) are omitted entirely when absent, never serialized as JSON `null`.
- **DB-gated tests** need `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test"` (PG18 + `cairn_pgx`); they self-serialize via `db::test_serial_guard` and re-apply all migrations via `db::connect_and_load_schema`.

---

### Task 1: Pure Rust link/unlink builders (`cairn-event`)

**Files:**
- Create: `crates/cairn-event/src/identity.rs`
- Modify: `crates/cairn-event/src/lib.rs:27` (add `pub mod identity;` beside `pub mod demographics;`)
- Test: `crates/cairn-event/src/identity.rs` (`#[cfg(test)]` module)

**Interfaces:**
- Produces:
  - `pub struct LinkAssertion<'a> { pub subject_a: &'a str, pub subject_b: &'a str, pub provenance: &'a str, pub confidence: Option<&'a str> }`
  - `pub fn link_assertion_body(a: &LinkAssertion) -> serde_json::Value`
  - `pub fn unlink_assertion_body(a: &LinkAssertion) -> serde_json::Value`
  - `pub fn render_link_twin(a: &LinkAssertion) -> String`
  - `pub fn render_unlink_twin(a: &LinkAssertion) -> String`

- [ ] **Step 1: Write the failing tests**

Create `crates/cairn-event/src/identity.rs`:

```rust
//! Identity linkage assertion builders (spec §5.1/§5.7 — matcher piece C1). Pure:
//! explicit inputs, no I/O, no DB. The safety-critical structural floor and the
//! connected-component projection live in the database (db/018); these functions
//! only shape and render the event a node will sign. Mirrors `demographics.rs`.

use serde_json::{json, Value};

/// One §5.7 link/unlink assertion between two immortal patient UUIDs. `subject_a`
/// and `subject_b` are the two UUIDs whose linkage is asserted; the event_type
/// (link vs unlink) — not the payload — carries the direction. The in-DB floor
/// (db/018) rejects a self-link (a == b) and an empty provenance.
pub struct LinkAssertion<'a> {
    pub subject_a: &'a str,  // §5.7 — one immortal subject UUID (string form)
    pub subject_b: &'a str,  // §5.7 — the other immortal subject UUID
    pub provenance: &'a str, // §4.1 provenance ladder — required-present, value-open
    pub confidence: Option<&'a str>, // acknowledged uncertainty (principle 4); omitted when None
}

/// Shared payload shape for link and unlink (identical; the event_type distinguishes
/// them). `confidence` is omitted entirely when absent — never serialized as null —
/// so the in-DB floor's key-presence checks see exactly what the author asserted.
fn assertion_body(a: &LinkAssertion) -> Value {
    let mut p = json!({
        "subject_a": a.subject_a,
        "subject_b": a.subject_b,
        "provenance": a.provenance,
    });
    if let Some(c) = a.confidence {
        p.as_object_mut()
            .expect("json! built an object")
            .insert("confidence".into(), json!(c));
    }
    p
}

/// Build the `identity.link.asserted` payload (the value of `EventBody.payload`).
pub fn link_assertion_body(a: &LinkAssertion) -> Value {
    assertion_body(a)
}

/// Build the `identity.unlink.asserted` payload — same shape as a link.
pub fn unlink_assertion_body(a: &LinkAssertion) -> Value {
    assertion_body(a)
}

/// Render the §4.5-style legibility twin for a link: profile-independent plaintext.
pub fn render_link_twin(a: &LinkAssertion) -> String {
    format!("link: {} ↔ {} ({})", a.subject_a, a.subject_b, a.provenance)
}

/// Render the §4.5-style legibility twin for an unlink.
pub fn render_unlink_twin(a: &LinkAssertion) -> String {
    format!("unlink: {} ↔ {} ({})", a.subject_a, a.subject_b, a.provenance)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> LinkAssertion<'static> {
        LinkAssertion {
            subject_a: "aaaaaaaa-0000-0000-0000-000000000001",
            subject_b: "bbbbbbbb-0000-0000-0000-000000000002",
            provenance: "matcher:cfg@hash",
            confidence: None,
        }
    }

    #[test]
    fn body_has_subjects_and_provenance() {
        let b = link_assertion_body(&sample());
        assert_eq!(b["subject_a"], "aaaaaaaa-0000-0000-0000-000000000001");
        assert_eq!(b["subject_b"], "bbbbbbbb-0000-0000-0000-000000000002");
        assert_eq!(b["provenance"], "matcher:cfg@hash");
    }

    #[test]
    fn confidence_omitted_when_absent_never_null() {
        let b = link_assertion_body(&sample());
        assert!(
            b.get("confidence").is_none(),
            "confidence must be omitted entirely when absent, never serialized as null"
        );
    }

    #[test]
    fn confidence_present_when_given() {
        let a = LinkAssertion { confidence: Some("0.91"), ..sample() };
        let b = link_assertion_body(&a);
        assert_eq!(b["confidence"], "0.91");
    }

    #[test]
    fn link_and_unlink_bodies_are_identical() {
        assert_eq!(link_assertion_body(&sample()), unlink_assertion_body(&sample()));
    }

    #[test]
    fn twins_distinguish_link_from_unlink() {
        assert!(render_link_twin(&sample()).starts_with("link: "));
        assert!(render_unlink_twin(&sample()).starts_with("unlink: "));
        assert!(render_link_twin(&sample()).contains("matcher:cfg@hash"));
    }
}
```

Note: `..sample()` in `confidence_present_when_given` requires `LinkAssertion` to be constructed field-by-field; since `sample()` returns a value and the struct has `Copy`-able `&str`/`Option<&str>` fields, functional-update works because the remaining fields are copied. (All fields are `&str`/`Option<&str>`, which are `Copy`.)

- [ ] **Step 2: Register the module**

In `crates/cairn-event/src/lib.rs`, directly below line 27 (`pub mod demographics;`) add:

```rust
pub mod identity;
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test -p cairn-event identity`
Expected: FAIL to compile / test failures (module just added; tests reference the not-yet-final code). If it compiles and passes immediately, that is also acceptable for pure builders — but confirm all five tests are collected.

- [ ] **Step 4: Confirm implementation compiles and passes**

The implementation is already in the file from Step 1. Run: `cargo test -p cairn-event identity`
Expected: PASS — 5 tests.

- [ ] **Step 5: Clippy + commit**

Run: `cargo clippy -p cairn-event -- -D warnings`
Expected: clean.

```bash
git add crates/cairn-event/src/identity.rs crates/cairn-event/src/lib.rs
git commit -m "feat(identity): pure link/unlink assertion builders (C1)"
```

---

### Task 2: In-DB structural floor + twin hook + migration wiring (`db/018`)

**Files:**
- Create: `db/018_identity_linkage.sql`
- Modify: `crates/cairn-node/src/db.rs:3` (bump array length `16`→`17`) and `:23` (add the `018` entry after `017_match_proposal`)
- Test: `crates/cairn-node/tests/identity_linkage.rs`

**Interfaces:**
- Produces (SQL, callable from `submit_event` via the twin hook):
  - event types `identity.link.asserted`, `identity.unlink.asserted` registered `additive`
  - `cairn_check_link_assertion(b jsonb) RETURNS void` — raises on structural violation
  - `cairn_event_twin(text, jsonb)` extended with the identity branch (preserves demographic + generic behavior)
- Consumes: `submit_event(bytea)` (db/005), `cairn_check_identifier_assertion` / `cairn_check_demographic_field` / `cairn_twin_skeleton` (db/010/011/015), `cairn_event::{sign, EventBody, Hlc}`, `cairn_event::identity::*` (Task 1).

- [ ] **Step 1: Write the failing integration tests**

Create `crates/cairn-node/tests/identity_linkage.rs`:

```rust
//! Integration coverage for the §5.1/§5.7 identity linkage core (db/018): the
//! link/unlink event types, the structural floor, the patient_link edge overlay,
//! the person_member connected-component projection, and the person_chart
//! unified-read VIEW. Real Postgres, gated on `$CAIRN_TEST_PG`, serialized
//! cluster-wide via `db::test_serial_guard`. The advisory matcher (§5.2, Python)
//! and the proposal→apply seam (C2) are separate subsystems, NOT exercised here.
use cairn_event::identity::{
    link_assertion_body, render_link_twin, render_unlink_twin, unlink_assertion_body, LinkAssertion,
};
use cairn_event::{generate_key, sign, EventBody, Hlc, SigningKey};
use cairn_node::db;
use tokio_postgres::Client;
use uuid::Uuid;

fn cs() -> Option<String> { std::env::var("CAIRN_TEST_PG").ok() }

/// The Postgres error message text for a failed statement. `tokio_postgres::Error`'s
/// Display renders only as the literal "db error" — the actual `RAISE EXCEPTION`
/// message lives in the DbError payload — so every floor-rejection assertion must go
/// through here (project convention: see `twin_globalise.rs`, `admission.rs`).
fn db_msg(e: &tokio_postgres::Error) -> String {
    e.as_db_error().map(|d| d.message().to_string()).unwrap_or_else(|| e.to_string())
}

/// Truncate the clinical + linkage tables and enroll one agent signer. Returns (sk, kid).
/// `patient_link` / `person_member` are created by LATER sections of db/018 (Tasks 3–4),
/// so they are truncated behind a `to_regclass` guard — this keeps the single `setup()`
/// helper correct at every stage as the migration grows, with no cross-task edits.
async fn setup(c: &Client) -> (SigningKey, String) {
    c.batch_execute(
        "TRUNCATE event_log, actor_event, patient_chart, patient_identifier, \
         patient_demographic CASCADE")
        .await.unwrap();
    c.batch_execute(
        "DO $$ BEGIN \
           IF to_regclass('public.patient_link')  IS NOT NULL THEN TRUNCATE patient_link;  END IF; \
           IF to_regclass('public.person_member') IS NOT NULL THEN TRUNCATE person_member; END IF; \
         END $$;")
        .await.unwrap();
    let (sk, kid) = generate_key().unwrap();
    c.execute(
        "SELECT enroll_actor('agent', '{\"model\":\"reg-stub\",\"version\":\"1\",\"skill_epoch\":\"e\"}', $1)",
        &[&kid],
    ).await.unwrap();
    (sk, kid)
}

/// Sign + submit one link OR unlink event through the real submit_event door.
/// `wall` is the HLC wall clock (higher = newer); patient_id is set to subject_a
/// by convention (an identity event is "about" subject_a's linkage). Returns the
/// submit result so a test can assert acceptance or a specific rejection.
async fn submit_link(
    c: &Client, sk: &SigningKey, kid: &str, a: Uuid, b: Uuid, wall: i64, is_link: bool,
) -> Result<u64, tokio_postgres::Error> {
    submit_link_prov(c, sk, kid, a, b, wall, is_link, "matcher:cfg@test").await
}

/// As `submit_link`, but with an explicit provenance string (for the empty-provenance
/// rejection test). Sends the provenance verbatim; an empty string must be rejected.
async fn submit_link_prov(
    c: &Client, sk: &SigningKey, kid: &str, a: Uuid, b: Uuid, wall: i64, is_link: bool, prov: &str,
) -> Result<u64, tokio_postgres::Error> {
    let a_s = a.to_string();
    let b_s = b.to_string();
    let la = LinkAssertion { subject_a: &a_s, subject_b: &b_s, provenance: prov, confidence: None };
    let (etype, sver, payload, twin) = if is_link {
        ("identity.link.asserted", "identity.link/1", link_assertion_body(&la), render_link_twin(&la))
    } else {
        ("identity.unlink.asserted", "identity.unlink/1", unlink_assertion_body(&la), render_unlink_twin(&la))
    };
    let body = EventBody {
        event_id: Uuid::now_v7().to_string(),
        patient_id: a_s.clone(),
        event_type: etype.into(),
        schema_version: sver.into(),
        hlc: Hlc { wall, counter: 0, node_origin: "n".into() },
        t_effective: None,
        signer_key_id: kid.into(),
        contributors: serde_json::json!([{"actor_id": kid, "role": "recorded"}]),
        payload,
        attachments: vec![],
        plaintext_twin: Some(twin),
    };
    let signed = sign(&body, sk).unwrap();
    c.execute("SELECT submit_event($1)", &[&signed.signed_bytes]).await
}

#[tokio::test]
async fn valid_link_is_accepted() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
    submit_link(&c, &sk, &kid, a, b, 100, true).await.expect("valid link accepted");
}

#[tokio::test]
async fn self_link_is_rejected() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let a = Uuid::now_v7();
    let err = submit_link(&c, &sk, &kid, a, a, 100, true).await.unwrap_err();
    assert!(db_msg(&err).contains("self-link"), "self-link must be refused: {}", db_msg(&err));
}

#[tokio::test]
async fn empty_provenance_is_rejected() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
    let err = submit_link_prov(&c, &sk, &kid, a, b, 100, true, "   ").await.unwrap_err();
    assert!(db_msg(&err).contains("provenance"), "empty provenance must be refused: {}", db_msg(&err));
}

#[tokio::test]
async fn missing_twin_is_rejected() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
    // Build a link event with NO authored twin — the identity floor HARD-requires one.
    let (a_s, b_s) = (a.to_string(), b.to_string());
    let la = LinkAssertion { subject_a: &a_s, subject_b: &b_s, provenance: "p", confidence: None };
    let body = EventBody {
        event_id: Uuid::now_v7().to_string(),
        patient_id: a_s.clone(),
        event_type: "identity.link.asserted".into(),
        schema_version: "identity.link/1".into(),
        hlc: Hlc { wall: 100, counter: 0, node_origin: "n".into() },
        t_effective: None,
        signer_key_id: kid.clone(),
        contributors: serde_json::json!([{"actor_id": kid, "role": "recorded"}]),
        payload: link_assertion_body(&la),
        attachments: vec![],
        plaintext_twin: None,
    };
    let signed = sign(&body, &sk).unwrap();
    let err = c.execute("SELECT submit_event($1)", &[&signed.signed_bytes]).await.unwrap_err();
    assert!(db_msg(&err).contains("authored twin"), "twin-less identity event must be refused: {}", db_msg(&err));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd crates/cairn-node && CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test --test identity_linkage`
Expected: FAIL — the crate does not compile (db/018 not wired; `identity_linkage.rs` references a not-yet-loaded migration → tests error at `submit_event` with "unknown event_type").

- [ ] **Step 3: Create `db/018` with registration + floor + twin hook**

Create `db/018_identity_linkage.sql`:

```sql
-- db/018_identity_linkage.sql
-- Cairn — §5.1/§5.7 identity linkage core (matcher piece C1).
--
-- WHAT: the authoritative destination for identity linkage. Adds the additive
-- `identity.link.asserted` / `identity.unlink.asserted` event types, a
-- culture-neutral structural floor, an HLC-overlay `patient_link` edge table, and
-- a `person_member` connected-component ("golden identity") projection with clean
-- unmerge (principle 2 — never merge, always link; unmerge is always clean).
--
-- The safety-critical write door submit_event (db/005) is REUSED verbatim: new
-- types register in event_type_class and add a branch to the cairn_event_twin hook.
-- Advisory matching (§5.2) and the proposal→apply seam (C2) are NOT here.

BEGIN;

-- 1. Register the two additive identity event types (fail-closed registry, ADR-0010).
--    additive + targets_other_author=FALSE: a link neither suppresses nor targets
--    another author's event, so the existing gate requires NO attestation for a
--    matcher-authored link (§5.2 "auto above threshold"); a human who vouches simply
--    includes a responsibility-bearing contributor, which the gate already attests.
INSERT INTO event_type_class (event_type, mode, targets_other_author) VALUES
    ('identity.link.asserted',   'additive', FALSE),
    ('identity.unlink.asserted', 'additive', FALSE)
ON CONFLICT (event_type) DO NOTHING;

-- 2. The §5.7 structural floor. Culture-neutral: validates STRUCTURE only — two
--    distinct valid UUID subjects and a non-empty provenance. Each violation is a
--    distinct legible exception (the cairn_check_identifier_assertion pattern).
CREATE OR REPLACE FUNCTION cairn_check_link_assertion(b jsonb)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    p jsonb := b -> 'payload';
    a text;
    c text;
BEGIN
    IF p IS NULL THEN
        RAISE EXCEPTION 'link assertion: missing payload';
    END IF;
    -- subject_a / subject_b: present, string.
    IF jsonb_typeof(p -> 'subject_a') IS DISTINCT FROM 'string'
       OR jsonb_typeof(p -> 'subject_b') IS DISTINCT FROM 'string' THEN
        RAISE EXCEPTION 'link assertion: subject_a and subject_b must be uuid strings (§5.7)';
    END IF;
    a := p ->> 'subject_a';
    c := p ->> 'subject_b';
    -- ...valid UUIDs (a bad cast here is a legible reject, not an opaque crash).
    BEGIN
        PERFORM a::uuid;
        PERFORM c::uuid;
    EXCEPTION WHEN others THEN
        RAISE EXCEPTION 'link assertion: subject_a/subject_b must be valid uuids (§5.7)';
    END;
    -- ...and distinct (a self-link is meaningless and would corrupt the component walk).
    IF a::uuid = c::uuid THEN
        RAISE EXCEPTION 'link assertion: self-link refused (subject_a = subject_b) (§5.1)';
    END IF;
    -- provenance: present, non-empty (§4.1 ladder; value-open, "unknown" is honest).
    IF jsonb_typeof(p -> 'provenance') IS DISTINCT FROM 'string'
       OR length(trim(p ->> 'provenance')) = 0 THEN
        RAISE EXCEPTION 'link assertion: provenance must be a non-empty string (§4.1)';
    END IF;
    -- confidence: optional; when present must not be JSON null (omit-when-absent
    -- discipline — a null confidence is a serialization bug, not "unknown", which is
    -- expressed by omitting the key; principle 4).
    IF (p ? 'confidence') AND (p -> 'confidence') = 'null'::jsonb THEN
        RAISE EXCEPTION 'link assertion: confidence must be omitted when absent, never null (principle 4)';
    END IF;
END;
$$;

-- 3. Extend the per-type twin hook. Identity link/unlink: run the floor + HARD-require
--    an authored twin (like demographics). This CREATE OR REPLACE PRESERVES db/010's
--    demographic branches and db/015's honest-degrade fallback for every other type —
--    it only adds the identity branch (submit_event itself is NEVER re-declared).
CREATE OR REPLACE FUNCTION cairn_event_twin(p_type text, b jsonb)
RETURNS text LANGUAGE plpgsql AS $$
DECLARE
    v_twin        text    := b ->> 'plaintext_twin';
    v_authored    boolean := v_twin IS NOT NULL AND length(regexp_replace(v_twin, '\s+', '', 'g')) > 0;
    v_demographic boolean := false;
    v_identity    boolean := false;
BEGIN
    -- Per-type structural floor.
    IF p_type = 'demographic.identifier.asserted' THEN
        PERFORM cairn_check_identifier_assertion(b);
        v_demographic := true;
    ELSIF p_type = 'demographic.field.asserted' THEN
        PERFORM cairn_check_demographic_field(b);
        v_demographic := true;
    ELSIF p_type IN ('identity.link.asserted', 'identity.unlink.asserted') THEN
        PERFORM cairn_check_link_assertion(b);
        v_identity := true;
    END IF;

    -- Authored twin present → carry it verbatim (principle 11; the conformant path).
    IF v_authored THEN
        RETURN v_twin;
    END IF;

    -- Absent/blank twin: demographic AND identity types HARD-require it; every other
    -- type degrades honestly to a flagged derived skeleton (ADR-0039).
    IF v_demographic THEN
        RAISE EXCEPTION 'submit_event: demographic assertion requires a non-empty authored twin (§4.5)';
    ELSIF v_identity THEN
        RAISE EXCEPTION 'submit_event: identity linkage assertion requires a non-empty authored twin (§5.7)';
    END IF;
    RETURN cairn_twin_skeleton(p_type, b);
END;
$$;

COMMIT;
```

- [ ] **Step 4: Wire `db/018` into the migration array**

In `crates/cairn-node/src/db.rs`, change the array length on line 3:

```rust
const SCHEMA: [(&str, &str); 17] = [
```

and add, immediately after the `017_match_proposal` line (line 23):

```rust
    ("018_identity_linkage", include_str!("../../../db/018_identity_linkage.sql")),
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cd crates/cairn-node && CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test --test identity_linkage`
Expected: PASS — `valid_link_is_accepted`, `self_link_is_rejected`, `empty_provenance_is_rejected`, `missing_twin_is_rejected`. (Tests that reference `patient_link`/`person_member`/`person_chart` come in later tasks.)

- [ ] **Step 6: Commit**

```bash
git add db/018_identity_linkage.sql crates/cairn-node/src/db.rs crates/cairn-node/tests/identity_linkage.rs
git commit -m "feat(identity): link/unlink event types + structural floor + twin hook (C1)"
```

---

### Task 3: `patient_link` edge overlay + maintenance trigger (edge only)

**Files:**
- Modify: `db/018_identity_linkage.sql` (add the `patient_link` table + `patient_link_apply` trigger before `COMMIT;`)
- Test: `crates/cairn-node/tests/identity_linkage.rs` (add tests)

**Interfaces:**
- Produces: `patient_link(low, high, state, hlc_wall, hlc_counter, origin, provenance, confidence, updated_at)` maintained by an AFTER-INSERT trigger; edge `state` = latest-HLC of link/unlink for the canonical `(low, high)` pair.
- Consumes: `event_log` columns `body`, `event_type`, `hlc_wall`, `hlc_counter`, `node_origin` (db/001).

- [ ] **Step 1: Write the failing tests**

Add to `crates/cairn-node/tests/identity_linkage.rs` a helper and tests:

```rust
/// Read the standing edge state for a pair, or None if no edge row exists.
async fn edge_state(c: &Client, a: Uuid, b: Uuid) -> Option<String> {
    let (lo, hi) = if a < b { (a, b) } else { (b, a) };
    let row = c.query_opt(
        "SELECT state FROM patient_link WHERE low = $1 AND high = $2", &[&lo, &hi],
    ).await.unwrap();
    row.map(|r| r.get::<_, String>(0))
}

#[tokio::test]
async fn link_creates_a_standing_edge() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
    submit_link(&c, &sk, &kid, a, b, 100, true).await.unwrap();
    assert_eq!(edge_state(&c, a, b).await.as_deref(), Some("link"));
}

#[tokio::test]
async fn newer_unlink_overlays_older_link() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
    submit_link(&c, &sk, &kid, a, b, 100, true).await.unwrap();   // link @100
    submit_link(&c, &sk, &kid, a, b, 200, false).await.unwrap();  // unlink @200 (newer)
    assert_eq!(edge_state(&c, a, b).await.as_deref(), Some("unlink"));
}

#[tokio::test]
async fn older_link_does_not_overlay_newer_unlink() {
    // Out-of-order arrival must converge: the highest-HLC assertion wins regardless
    // of the order events land (offline-first set-union).
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
    submit_link(&c, &sk, &kid, a, b, 200, false).await.unwrap();  // unlink @200 lands first
    submit_link(&c, &sk, &kid, a, b, 100, true).await.unwrap();   // link @100 lands later (older)
    assert_eq!(edge_state(&c, a, b).await.as_deref(), Some("unlink"),
               "older link must not overlay a newer unlink");
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd crates/cairn-node && CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test --test identity_linkage`
Expected: the three new tests FAIL — `patient_link` relation does not exist yet.

- [ ] **Step 3: Add the table + trigger to `db/018`**

In `db/018_identity_linkage.sql`, immediately **before** the final `COMMIT;`, insert:

```sql
-- 4. patient_link: the standing-edge overlay (same shape as patient_identifier). One
--    row per canonical (low, high) pair; the latest-HLC link/unlink assertion wins the
--    `state`. Never merge, always overlay — link then a later unlink ⇒ edge gone.
CREATE TABLE IF NOT EXISTS patient_link (
    low         UUID    NOT NULL,
    high        UUID    NOT NULL,
    state       TEXT    NOT NULL CHECK (state IN ('link', 'unlink')),
    hlc_wall    BIGINT  NOT NULL,
    hlc_counter INTEGER NOT NULL,
    origin      TEXT    NOT NULL,
    provenance  TEXT    NOT NULL,
    confidence  TEXT,
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (low, high),
    CHECK (low < high)
);
GRANT SELECT ON patient_link TO cairn_agent;

-- Incremental maintenance: fold exactly the one new link/unlink event into the edge
-- overlay. The whole row overlays atomically only when the incoming HLC is strictly
-- greater than the stored one (ON CONFLICT ... WHERE) — so out-of-order arrival
-- converges to the highest-HLC assertion. (Component recompute is added in the next
-- task; this version maintains the edge only.)
CREATE OR REPLACE FUNCTION patient_link_apply()
RETURNS trigger LANGUAGE plpgsql AS $$
DECLARE
    p       jsonb := NEW.body;
    a       uuid  := (p ->> 'subject_a')::uuid;
    b       uuid  := (p ->> 'subject_b')::uuid;
    lo      uuid  := LEAST(a, b);
    hi      uuid  := GREATEST(a, b);
    v_state text  := CASE WHEN NEW.event_type = 'identity.link.asserted' THEN 'link' ELSE 'unlink' END;
BEGIN
    INSERT INTO patient_link
        (low, high, state, hlc_wall, hlc_counter, origin, provenance, confidence)
    VALUES
        (lo, hi, v_state, NEW.hlc_wall, NEW.hlc_counter, NEW.node_origin,
         p ->> 'provenance', p ->> 'confidence')
    ON CONFLICT (low, high) DO UPDATE SET
        state       = EXCLUDED.state,
        hlc_wall    = EXCLUDED.hlc_wall,
        hlc_counter = EXCLUDED.hlc_counter,
        origin      = EXCLUDED.origin,
        provenance  = EXCLUDED.provenance,
        confidence  = EXCLUDED.confidence,
        updated_at  = clock_timestamp()
    WHERE (EXCLUDED.hlc_wall, EXCLUDED.hlc_counter, EXCLUDED.origin)
        > (patient_link.hlc_wall, patient_link.hlc_counter, patient_link.origin);
    RETURN NULL;  -- AFTER trigger
END;
$$;

DROP TRIGGER IF EXISTS patient_link_apply_trg ON event_log;
CREATE TRIGGER patient_link_apply_trg
    AFTER INSERT ON event_log
    FOR EACH ROW WHEN (NEW.event_type IN ('identity.link.asserted', 'identity.unlink.asserted'))
    EXECUTE FUNCTION patient_link_apply();
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd crates/cairn-node && CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test --test identity_linkage`
Expected: PASS — all Task 2 + Task 3 tests.

- [ ] **Step 5: Commit**

```bash
git add db/018_identity_linkage.sql crates/cairn-node/tests/identity_linkage.rs
git commit -m "feat(identity): patient_link HLC-overlay edge projection + trigger (C1)"
```

---

### Task 4: `person_member` component projection + recompute + oversize guard

**Files:**
- Modify: `db/018_identity_linkage.sql` (add `person_member`, `cairn_max_component_size`, `cairn_recompute_component`; replace `patient_link_apply` to also recompute)
- Test: `crates/cairn-node/tests/identity_linkage.rs` (add tests)

**Interfaces:**
- Produces: `person_member(patient_id, person_id, updated_at)` — `person_id` = min-UUID of the connected component over standing `link` edges; `cairn_recompute_component(p_seed uuid) RETURNS void`; `cairn_max_component_size() RETURNS integer` (reads `cairn.max_component_size` GUC, default 10000).
- Consumes: `patient_link` (Task 3).

- [ ] **Step 1: Write the failing tests**

Add to `crates/cairn-node/tests/identity_linkage.rs`:

```rust
/// The person_id a UUID currently projects to, or None if it has no person_member row.
/// UUIDs are passed as text and cast in SQL (`$1::text::uuid`) and read back via
/// `::text` — this project's tokio-postgres has no uuid ToSql/FromSql (project
/// convention: see `match_veto.rs`).
async fn person_of(c: &Client, p: Uuid) -> Option<Uuid> {
    let p_s = p.to_string();
    c.query_opt(
        "SELECT person_id::text FROM person_member WHERE patient_id = $1::text::uuid",
        &[&p_s],
    ).await.unwrap().map(|r| r.get::<_, String>(0).parse().unwrap())
}

#[tokio::test]
async fn linked_pair_shares_min_uuid_person() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
    submit_link(&c, &sk, &kid, a, b, 100, true).await.unwrap();
    let expected = a.min(b);
    assert_eq!(person_of(&c, a).await, Some(expected));
    assert_eq!(person_of(&c, b).await, Some(expected));
}

#[tokio::test]
async fn transitive_links_form_one_person() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let (a, b, d) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
    submit_link(&c, &sk, &kid, a, b, 100, true).await.unwrap();
    submit_link(&c, &sk, &kid, b, d, 110, true).await.unwrap();
    let expected = a.min(b).min(d);
    assert_eq!(person_of(&c, a).await, Some(expected));
    assert_eq!(person_of(&c, b).await, Some(expected));
    assert_eq!(person_of(&c, d).await, Some(expected));
}

#[tokio::test]
async fn diamond_unlink_stays_merged() {
    // A-B, B-C, A-C all linked; unlink A-B. Still connected via A-C-B → one person.
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let (a, b, cc) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
    submit_link(&c, &sk, &kid, a, b, 100, true).await.unwrap();
    submit_link(&c, &sk, &kid, b, cc, 110, true).await.unwrap();
    submit_link(&c, &sk, &kid, a, cc, 120, true).await.unwrap();
    submit_link(&c, &sk, &kid, a, b, 130, false).await.unwrap(); // unlink A-B (not a bridge)
    let expected = a.min(b).min(cc);
    assert_eq!(person_of(&c, a).await, Some(expected));
    assert_eq!(person_of(&c, b).await, Some(expected));
    assert_eq!(person_of(&c, cc).await, Some(expected));
}

#[tokio::test]
async fn chain_unlink_splits_component() {
    // Chain A-B-C; unlink A-B (a bridge) → {A} and {B,C}.
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let (a, b, cc) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
    submit_link(&c, &sk, &kid, a, b, 100, true).await.unwrap();
    submit_link(&c, &sk, &kid, b, cc, 110, true).await.unwrap();
    submit_link(&c, &sk, &kid, a, b, 120, false).await.unwrap(); // unlink the A-B bridge
    assert_eq!(person_of(&c, a).await, Some(a), "A now isolated → maps to itself");
    let bc = b.min(cc);
    assert_eq!(person_of(&c, b).await, Some(bc));
    assert_eq!(person_of(&c, cc).await, Some(bc));
}

#[tokio::test]
async fn re_link_is_idempotent() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
    submit_link(&c, &sk, &kid, a, b, 100, true).await.unwrap();
    submit_link(&c, &sk, &kid, a, b, 105, true).await.unwrap(); // a second, later link of the same pair
    let expected = a.min(b);
    assert_eq!(person_of(&c, a).await, Some(expected));
    assert_eq!(person_of(&c, b).await, Some(expected));
    let n: i64 = c.query_one("SELECT count(*) FROM patient_link WHERE state='link'", &[])
        .await.unwrap().get(0);
    assert_eq!(n, 1, "re-linking the same pair is one standing edge, not two");
}

#[tokio::test]
async fn oversize_component_guard_rejects() {
    // With a tiny cap, the link that would grow the component past it is refused
    // wholesale (fail-loud, never a silent cap). Cap is a per-session GUC.
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    c.batch_execute("SET cairn.max_component_size = 3").await.unwrap();
    let (a, b, cc, d) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
    submit_link(&c, &sk, &kid, a, b, 100, true).await.unwrap();  // {A,B} size 2 — ok
    submit_link(&c, &sk, &kid, b, cc, 110, true).await.unwrap(); // {A,B,C} size 3 — ok
    let err = submit_link(&c, &sk, &kid, cc, d, 120, true).await.unwrap_err(); // size 4 — refuse
    assert!(db_msg(&err).contains("exceeds max size"), "oversize component must be refused: {}", db_msg(&err));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd crates/cairn-node && CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test --test identity_linkage`
Expected: the six new tests FAIL — `person_member` relation does not exist.

- [ ] **Step 3: Add the projection + recompute, and replace the trigger**

In `db/018_identity_linkage.sql`, immediately **before** the `patient_link_apply` function definition (added in Task 3), insert the projection table + helpers:

```sql
-- 5. person_member: the golden-identity projection. person_id = the MINIMUM UUID in
--    the connected component (a derived canonical representative — the "person" is a
--    projection, never a stored immortal id; principle 2). A UUID that once had an edge
--    and is now isolated gets a row mapping to itself; a UUID never touched by any
--    linkage event has no row at all (the person_chart VIEW coalesces to self).
CREATE TABLE IF NOT EXISTS person_member (
    patient_id UUID PRIMARY KEY,
    person_id  UUID NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
GRANT SELECT ON person_member TO cairn_agent;

-- Configurable oversize guard. A component larger than this is a matcher pathology
-- (mass false-merge); we REFUSE the offending event rather than silently corrupt
-- membership (never a silent cap — the db/017b oversized-block discipline). Reads a
-- session GUC so it is operationally tunable and testable; default 10000.
CREATE OR REPLACE FUNCTION cairn_max_component_size()
RETURNS integer LANGUAGE sql STABLE AS $$
    SELECT COALESCE(NULLIF(current_setting('cairn.max_component_size', true), '')::integer, 10000);
$$;

-- Recompute the connected component around one seed UUID over the STANDING link edges
-- (state='link'), and rewrite person_member for every member to point at the min-UUID
-- representative. Cost is bounded by the touched component's size, not the table's —
-- keeping chart reads O(1) (the ADR-0001/Bet-B incremental-projection discipline).
CREATE OR REPLACE FUNCTION cairn_recompute_component(p_seed uuid)
RETURNS void LANGUAGE plpgsql AS $$
DECLARE
    v_members uuid[];
    v_person  uuid;
BEGIN
    -- Bounded BFS: walk standing link edges outward from the seed (undirected — an
    -- edge stored as (low, high) is traversable from either endpoint).
    WITH RECURSIVE comp(node) AS (
        SELECT p_seed
        UNION
        SELECT CASE WHEN pl.low = comp.node THEN pl.high ELSE pl.low END
        FROM comp
        JOIN patient_link pl
          ON pl.state = 'link' AND (pl.low = comp.node OR pl.high = comp.node)
    )
    SELECT array_agg(node) INTO v_members FROM comp;

    -- Fail loud on a pathological component (mass false-merge) — never silently cap.
    IF array_length(v_members, 1) > cairn_max_component_size() THEN
        RAISE EXCEPTION
            'identity linkage: component around % exceeds max size % — refusing to project (matcher pathology)',
            p_seed, cairn_max_component_size();
    END IF;

    -- The canonical representative is the minimum UUID in the component.
    v_person := (SELECT min(m) FROM unnest(v_members) AS m);

    INSERT INTO person_member (patient_id, person_id, updated_at)
    SELECT m, v_person, clock_timestamp() FROM unnest(v_members) AS m
    ON CONFLICT (patient_id) DO UPDATE SET
        person_id  = EXCLUDED.person_id,
        updated_at = clock_timestamp();
END;
$$;
```

Then **replace** the `patient_link_apply` function body (the one from Task 3) so it recomputes both endpoints after the edge upsert. Change the two lines `    RETURN NULL;  -- AFTER trigger` / `END;` at the end of that function to:

```sql
    -- Recompute the touched component(s). Recomputing BOTH endpoints is always
    -- correct: a link merges (both endpoints reach the same union); an unlink splits
    -- into at most the piece containing `lo` and the piece containing `hi`, and every
    -- previously-connected node is reachable from one of them.
    PERFORM cairn_recompute_component(lo);
    PERFORM cairn_recompute_component(hi);
    RETURN NULL;  -- AFTER trigger
END;
$$;
```

(The full function now ends with the two `PERFORM` calls before `RETURN NULL;`. The `DROP TRIGGER … / CREATE TRIGGER …` block from Task 3 stays unchanged below it.)

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd crates/cairn-node && CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test --test identity_linkage`
Expected: PASS — all tests through Task 4 (13 total so far).

- [ ] **Step 5: Commit**

```bash
git add db/018_identity_linkage.sql crates/cairn-node/tests/identity_linkage.rs
git commit -m "feat(identity): person_member component projection + bounded recompute + oversize guard (C1)"
```

---

### Task 5: `person_chart` demonstrated unified-read VIEW

**Files:**
- Modify: `db/018_identity_linkage.sql` (add the `person_chart` VIEW + grant before `COMMIT;`)
- Test: `crates/cairn-node/tests/identity_linkage.rs` (add a test)

**Interfaces:**
- Produces: `person_chart` VIEW — every `patient_chart` row tagged with `person_id` (its component representative, or its own `patient_id` when unknown to the link graph).
- Consumes: `patient_chart` (db/002), `person_member` (Task 4).

- [ ] **Step 1: Write the failing test**

Add to `crates/cairn-node/tests/identity_linkage.rs`:

```rust
/// Submit a minimal patient.created so the patient has a patient_chart row to union.
async fn submit_patient_created(c: &Client, sk: &SigningKey, kid: &str, p: Uuid, wall: i64) {
    let body = EventBody {
        event_id: Uuid::now_v7().to_string(),
        patient_id: p.to_string(),
        event_type: "patient.created".into(),
        schema_version: "patient/1".into(),
        hlc: Hlc { wall, counter: 0, node_origin: "n".into() },
        t_effective: None,
        signer_key_id: kid.into(),
        contributors: serde_json::json!([{"actor_id": kid, "role": "recorded"}]),
        payload: serde_json::json!({"name": "T", "dob": "1990", "sex": "x"}),
        attachments: vec![],
        plaintext_twin: None, // non-demographic type → honest-degrade skeleton (db/015)
    };
    let signed = sign(&body, sk).unwrap();
    c.execute("SELECT submit_event($1)", &[&signed.signed_bytes]).await
        .expect("patient.created accepted");
}

#[tokio::test]
async fn person_chart_unions_member_streams() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
    submit_patient_created(&c, &sk, &kid, a, 100).await;
    submit_patient_created(&c, &sk, &kid, b, 101).await;
    submit_link(&c, &sk, &kid, a, b, 110, true).await.unwrap();
    let person = a.min(b).to_string();
    // Selecting by the shared person_id returns BOTH member charts.
    let n: i64 = c.query_one(
        "SELECT count(*) FROM person_chart WHERE person_id = $1::text::uuid", &[&person],
    ).await.unwrap().get(0);
    assert_eq!(n, 2, "person_chart must union both member UUIDs' chart rows under one person_id");
}

#[tokio::test]
async fn person_chart_defaults_unlinked_to_self() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let a = Uuid::now_v7();
    submit_patient_created(&c, &sk, &kid, a, 100).await; // never linked → no person_member row
    let a_s = a.to_string();
    let pid: String = c.query_one(
        "SELECT person_id::text FROM person_chart WHERE patient_id = $1::text::uuid", &[&a_s],
    ).await.unwrap().get(0);
    assert_eq!(pid, a_s, "a UUID unknown to the link graph is its own person");
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd crates/cairn-node && CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test --test identity_linkage`
Expected: the two new tests FAIL — `person_chart` relation does not exist.

- [ ] **Step 3: Add the VIEW to `db/018`**

In `db/018_identity_linkage.sql`, immediately **before** the final `COMMIT;`, insert:

```sql
-- 6. Demonstrated unified-read VIEW (§5.1 "the unified chart unions the event streams
--    of all member UUIDs"). Thin by design: every patient_chart row is tagged with its
--    person_id — its component representative, or its own patient_id when unknown to the
--    link graph. Selecting WHERE person_id = X returns all member charts. The REAL
--    unified-chart read surface (ordering, dedup, trust states) is the API/UI tier,
--    above the foundation line — deliberately out of scope for C1.
CREATE OR REPLACE VIEW person_chart AS
    SELECT COALESCE(pm.person_id, pc.patient_id) AS person_id, pc.*
    FROM patient_chart pc
    LEFT JOIN person_member pm ON pm.patient_id = pc.patient_id;

GRANT SELECT ON person_chart TO cairn_agent;
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd crates/cairn-node && CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test --test identity_linkage`
Expected: PASS — the full `identity_linkage` suite (15 tests).

- [ ] **Step 5: Full-suite regression + clippy**

Run: `cd crates/cairn-node && CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test` then `cargo clippy --workspace -- -D warnings`
Expected: all existing cairn-node integration tests still green (db/018 is additive; the extended `cairn_event_twin` preserves demographic + generic behavior); clippy clean.

- [ ] **Step 6: Commit**

```bash
git add db/018_identity_linkage.sql crates/cairn-node/tests/identity_linkage.rs
git commit -m "feat(identity): person_chart demonstrated unified-read VIEW (C1)"
```

---

### Task 6: Documentation — HANDOVER + ROADMAP

**Files:**
- Modify: `docs/HANDOVER.md` (new "This session" entry; move C1 from "next" to built)
- Modify: `docs/ROADMAP.md` (add "Slice 13 — §5.1/§5.7 identity linkage core (piece C1)" under Phase 4; update the "Remaining matcher pieces" / "piece C" note)

**Interfaces:** none (docs only).

- [ ] **Step 1: Update `docs/HANDOVER.md`**

Prepend a new "This session (2026-07-01)" block summarizing C1 (link/unlink event types through the reused `submit_event` door; `patient_link` HLC-overlay edges; `person_member` min-UUID component projection with bounded per-endpoint recompute + oversize guard; `person_chart` demonstrated union VIEW; `cairn-event::identity` pure builders; db/018, no SCHEMA-floor change beyond additive DDL, no ADR/spec bump — implements settled §5.1/§5.7). Demote the previous "This session" block to "Prior session." In the "Open threads" matcher paragraph, mark **piece C1 BUILT** and note **C2 (proposal→apply seam)** + **C3+ (identify/repudiate/dispute/reattribute)** as the next identity slices. Record the test command: `cd crates/cairn-node && CAIRN_TEST_PG=… cargo test --test identity_linkage`. Keep the file under 500 lines (prune the oldest condensed detail if needed).

- [ ] **Step 2: Update `docs/ROADMAP.md`**

Under Phase 4, after "Slice 12", add a "Slice 13 — §5.1/§5.7 identity linkage core (piece C1)" bullet in the same condensed style (what was built + files + "no `db/` floor bypass, additive, no SCHEMA/ADR/spec change"). Update the "Remaining matcher pieces … piece C — the §5.7 link-apply seam" line to reflect that **C1 (linkage core) is now built**; C2 (proposal→apply seam) and C3+ (the rest of the algebra) remain.

- [ ] **Step 3: Verify line counts**

Run: `wc -l docs/HANDOVER.md docs/ROADMAP.md`
Expected: both ≤ ~500 (prune if over).

- [ ] **Step 4: Commit**

```bash
git add docs/HANDOVER.md docs/ROADMAP.md
git commit -m "docs(identity): record C1 linkage core built (HANDOVER + ROADMAP)"
```

---

## Self-Review

**Spec coverage** (design doc → task):
- Two additive event types through reused `submit_event` → Task 2. ✓
- Structural floor (distinct valid UUIDs, non-empty provenance, self-link rejected, confidence non-null) → Task 2. ✓
- Authored twin HARD-required for identity types (preserving demographic + generic behavior) → Task 2. ✓
- `patient_link` HLC-overlay edge table + out-of-order convergence → Task 3. ✓
- `person_member` min-UUID component projection; transitive merge; diamond-unlink stays merged; chain-unlink splits; isolated-maps-to-self; idempotent re-link → Task 4. ✓
- Bounded per-endpoint recompute + loud oversize guard (never silent cap) → Task 4. ✓
- `person_chart` demonstrated union VIEW + unlinked-defaults-to-self → Task 5. ✓
- Pure Rust builders, confidence omit-when-absent, twin rendering → Task 1. ✓
- Deferred items (C2, C3+, real unified read, trust states, coherence re-trigger) → recorded in design "Out of scope"; not implemented, correct. ✓

**Placeholder scan:** none — every step carries full code/commands/expected output.

**Type consistency:** `LinkAssertion` fields (`subject_a`/`subject_b`/`provenance`/`confidence`) match between Task 1 (Rust) and Task 2's payload keys and the SQL floor. `submit_link`/`submit_link_prov`/`edge_state`/`person_of` helper signatures are defined once and reused. `cairn_recompute_component`/`cairn_max_component_size` names match between definition (Task 4) and the trigger's `PERFORM` calls. Event type strings (`identity.link.asserted`/`identity.unlink.asserted`) are identical across the registry insert, the twin hook, the trigger `WHEN`, and the Rust builder. `person_id` = min-UUID convention is consistent across Tasks 4–5 and the tests. ✓

**One known intra-file build note:** `db/018` grows across Tasks 2→5. Because every object uses `CREATE OR REPLACE` / `CREATE TABLE IF NOT EXISTS` / `DROP TRIGGER IF EXISTS` and `connect_and_load_schema` re-runs the whole file each test, each task's file is independently valid and idempotent. Task 4 replaces Task 3's `patient_link_apply` body (adding the two `PERFORM` recompute calls) — the trigger `CREATE` below it is unchanged.
