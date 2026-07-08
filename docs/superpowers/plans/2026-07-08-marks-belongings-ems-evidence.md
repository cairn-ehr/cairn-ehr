# §5.4 marks / belongings / EMS-context identity evidence — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add three text-shaped `kind` values — `mark`, `belongings`, `ems-context` — to the existing `identity.evidence.asserted` event type, with a `cairn-node` author path and CLI, recording clinician-observed non-demographic corroboration on an unidentified patient's chart.

**Architecture:** Pure builders extend `cairn-event::identity_evidence` (payload + authored twin + a closed-set kind parser). A new `cairn-node::identity_evidence` module carries the honest-content floor (`validate_description`), the pure `EventBody` assembly, and an async orchestrator that authors the event in one `submit_event` call (no blob tier — the observation is text, `attachments` stays empty). One CLI subcommand wires it. An e2e DB-gated test proves read-back.

**Tech Stack:** Rust (`cairn-event`, `cairn-node`), `tokio-postgres`, `serde_json`, PostgreSQL ≥ 18 with `cairn_pgx`.

## Global Constraints

- **No new migration, floor change, SCHEMA bump, ADR, or spec-prose change.** The `identity.evidence.asserted` type is already registered (`db/028`), additive, non-demographic; the `db/015` twin floor carries the authored twin verbatim.
- **AGPL-3.0**; no new dependencies (everything used is already in-tree).
- **TDD**: failing test first, then minimal code; all tests green before commit.
- **`cairn-event` has NO `anyhow`** (uses `thiserror`) — pure functions there return `Option`/`Value`/`String`, never `anyhow::Result`. The `anyhow` error framing lives in the `cairn-node` layer.
- **Provenance is fixed** `"clinician-observed"` for all three kinds; the relayed/hearsay distinction lives in the free-text `basis`. (The floor refuses an empty `description`; whether a UI silently supplies a default is *soft policy* above our line — principle 12 — not our concern here.)
- **`description` required, non-empty**; `basis` optional and omitted (never null) when absent.
- **Closed `kind` set validated at the author edge** (typo-drift guard); the event type stays wire-open per ADR-0012.
- Reuse `cairn_event::evidence::CLINICIAN_OBSERVED_PROVENANCE` and the existing `IDENTITY_EVIDENCE_EVENT_TYPE` / `IDENTITY_EVIDENCE_SCHEMA_VERSION` constants.
- DB-gated tests need `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test"` (PG18 + `cairn_pgx`); they self-serialize via `db::test_serial_guard`.

---

### Task 1: `cairn-event` pure builders (constants, kind parser, payload, twin)

**Files:**
- Modify: `crates/cairn-event/src/identity_evidence.rs` (append to the existing module + its `#[cfg(test)] mod tests`)

**Interfaces:**
- Consumes: `crate::evidence::CLINICIAN_OBSERVED_PROVENANCE` (`&str = "clinician-observed"`); `serde_json::{json, Value}` (already imported at top of file).
- Produces:
  - `pub const MARK_EVIDENCE_KIND: &str = "mark";`
  - `pub const BELONGINGS_EVIDENCE_KIND: &str = "belongings";`
  - `pub const EMS_CONTEXT_EVIDENCE_KIND: &str = "ems-context";`
  - `pub const TEXT_EVIDENCE_KINDS: [&str; 3]`
  - `pub fn parse_text_evidence_kind(kind: &str) -> Option<&'static str>`
  - `pub fn text_evidence_body(kind: &str, description: &str, basis: Option<&str>) -> serde_json::Value`
  - `pub fn render_text_evidence_twin(kind: &str, description: &str, basis: Option<&str>) -> String`

- [ ] **Step 1: Write the failing tests** — append to the `tests` module in `crates/cairn-event/src/identity_evidence.rs`:

```rust
    #[test]
    fn text_evidence_kinds_parse_to_canonical_constants_and_reject_unknowns() {
        assert_eq!(parse_text_evidence_kind("mark"), Some(MARK_EVIDENCE_KIND));
        assert_eq!(parse_text_evidence_kind("belongings"), Some(BELONGINGS_EVIDENCE_KIND));
        assert_eq!(parse_text_evidence_kind("ems-context"), Some(EMS_CONTEXT_EVIDENCE_KIND));
        assert_eq!(parse_text_evidence_kind("photo"), None, "photo is the attachment path, not a text kind");
        assert_eq!(parse_text_evidence_kind("Mark"), None, "case-sensitive: canonical spelling only");
        assert_eq!(parse_text_evidence_kind(""), None);
        // The closed set and the constants cannot drift apart.
        assert_eq!(TEXT_EVIDENCE_KINDS,
                   [MARK_EVIDENCE_KIND, BELONGINGS_EVIDENCE_KIND, EMS_CONTEXT_EVIDENCE_KIND]);
    }

    #[test]
    fn text_body_carries_kind_provenance_description_and_optional_basis() {
        let with = text_evidence_body("mark", "scar on left forearm ~5cm", Some("primary survey"));
        assert_eq!(with["kind"], "mark");
        assert_eq!(with["provenance"], "clinician-observed");
        assert_eq!(with["description"], "scar on left forearm ~5cm");
        assert_eq!(with["basis"], "primary survey");

        let without = text_evidence_body("belongings", "blue wallet, €40, keys", None);
        assert_eq!(without["kind"], "belongings");
        assert_eq!(without["description"], "blue wallet, €40, keys");
        assert!(without.get("basis").is_none(), "absent basis is omitted, never null");
    }

    #[test]
    fn text_twin_is_legible_names_the_kind_and_appends_basis_only_when_present() {
        let with = render_text_evidence_twin("ems-context", "found unconscious at bus stop", Some("reported by paramedic"));
        assert!(with.contains("ems-context"), "twin names the kind: {with}");
        assert!(with.contains("found unconscious at bus stop"), "description: {with}");
        assert!(with.contains("reported by paramedic"), "basis when present: {with}");
        assert!(with.contains(" — "), "basis is set off by an em-dash");

        let without = render_text_evidence_twin("mark", "tattoo of an anchor, right shoulder", None);
        assert!(without.contains("tattoo of an anchor"), "{without}");
        assert!(!without.contains(" — "), "no trailing basis separator when basis is None: {without}");
        assert!(!without.trim().is_empty());
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p cairn-event identity_evidence:: -- --nocapture`
Expected: FAIL — `cannot find value/function` for `parse_text_evidence_kind`, `text_evidence_body`, `render_text_evidence_twin`, `MARK_EVIDENCE_KIND`, etc.

- [ ] **Step 3: Write the minimal implementation** — insert into `crates/cairn-event/src/identity_evidence.rs` after the existing `PHOTO_EVIDENCE_KIND` const (before `photo_evidence_body`):

```rust
/// The `kind` discriminator for a distinguishing bodily mark (scar, tattoo, amputation, ...).
pub const MARK_EVIDENCE_KIND: &str = "mark";
/// The `kind` discriminator for personal belongings found on the patient.
pub const BELONGINGS_EVIDENCE_KIND: &str = "belongings";
/// The `kind` discriminator for the EMS pickup context (where/how the patient was found).
pub const EMS_CONTEXT_EVIDENCE_KIND: &str = "ems-context";

/// The closed set of text-shaped evidence kinds, in one place so the parser below and any
/// future reader share a single source of truth (a test pins it to the three constants).
pub const TEXT_EVIDENCE_KINDS: [&str; 3] =
    [MARK_EVIDENCE_KIND, BELONGINGS_EVIDENCE_KIND, EMS_CONTEXT_EVIDENCE_KIND];

/// Map an input kind string to its canonical `&'static str`, returning `None` for anything
/// outside the closed set. The set is closed at the AUTHOR edge — a typo-drift guard so the
/// event log does not accumulate a mess of near-synonym kinds over time — NOT on the wire: the
/// event type stays additively open per ADR-0012. Returns `Option` (not `anyhow::Result`)
/// because `cairn-event` is dependency-light; the node/CLI layer supplies the error framing.
pub fn parse_text_evidence_kind(kind: &str) -> Option<&'static str> {
    TEXT_EVIDENCE_KINDS.into_iter().find(|k| *k == kind)
}

/// Build the payload for a TEXT identity-evidence event: `{ kind, provenance, description,
/// basis? }`. There is no attachment — the observation IS the `description` text (compare the
/// photo path, where the bytes ride `EventBody.attachments` and the descriptor lives on the
/// attachment). `basis` (how/why observed; for ems-context, the relayed source) is optional and
/// omitted entirely when None (principle 4: never manufacture a basis).
pub fn text_evidence_body(kind: &str, description: &str, basis: Option<&str>) -> Value {
    let mut body = json!({
        "kind": kind,
        "provenance": crate::evidence::CLINICIAN_OBSERVED_PROVENANCE,
        "description": description,
    });
    if let Some(b) = basis {
        body["basis"] = json!(b);
    }
    body
}

/// Render the authored §3.13/§4.5 twin for a TEXT identity-evidence event: names the kind, then
/// the observed description directly (no attachment to defer to), then the basis if stated. A
/// pure mechanical derivation (ADR-0039 pattern) the db/015 floor carries verbatim.
pub fn render_text_evidence_twin(kind: &str, description: &str, basis: Option<&str>) -> String {
    let mut out = format!("identity evidence ({kind}): {description}");
    if let Some(b) = basis {
        out.push_str(&format!(" — {b}"));
    }
    out
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p cairn-event identity_evidence::`
Expected: PASS (all existing photo tests + the three new ones).

- [ ] **Step 5: Commit**

```bash
git add crates/cairn-event/src/identity_evidence.rs
git commit -m "feat(event): text kinds for identity.evidence.asserted (mark/belongings/ems-context)

Closed-set kind parser + text payload/twin builders on the existing event type;
no attachment (the observation is the description). No wire/floor/SCHEMA change.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 2: `cairn-node::identity_evidence` author path

**Files:**
- Create: `crates/cairn-node/src/identity_evidence.rs`
- Modify: `crates/cairn-node/src/lib.rs` (add `pub mod identity_evidence;` in alphabetical position, between `pub mod identity;` and `pub mod john_doe;`)

**Interfaces:**
- Consumes: Task 1's `parse_text_evidence_kind`, `text_evidence_body`, `render_text_evidence_twin`; `IDENTITY_EVIDENCE_EVENT_TYPE`, `IDENTITY_EVIDENCE_SCHEMA_VERSION` (existing consts); `cairn_event::{sign, EventBody, Hlc, SigningKey}`; `crate::db::next_hlc(&Client, &str) -> anyhow::Result<Hlc>`.
- Produces:
  - `pub fn validate_description(description: &str) -> anyhow::Result<()>`
  - `pub fn build_text_evidence_body(event_id: Uuid, patient_id: Uuid, kid: &str, hlc: Hlc, kind: &str, description: &str, basis: Option<&str>) -> EventBody`
  - `pub async fn assert_text_evidence(client: &Client, sk: &SigningKey, kid: &str, node_origin: &str, patient_id: Uuid, kind: &str, description: &str, basis: Option<&str>) -> anyhow::Result<Uuid>`

- [ ] **Step 1: Write the failing pure tests** — create `crates/cairn-node/src/identity_evidence.rs` with the module doc + `use`s + a `#[cfg(test)] mod tests` (implementation stubbed to `todo!()` so it compiles-then-fails, OR write impl in Step 3 — here, write the tests first against not-yet-written fns):

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn hlc() -> Hlc { Hlc { wall: 7, counter: 0, node_origin: "n".into() } }

    #[test]
    fn validate_description_refuses_empty_and_whitespace_only() {
        // The honest-content floor lives in the library, so a caller bypassing the CLI
        // (a UI backend) still cannot author an evidence assertion that says nothing.
        assert!(validate_description("").is_err(), "empty refused");
        assert!(validate_description("   \t\n").is_err(), "whitespace-only refused");
        assert!(validate_description("scar on left forearm").is_ok(), "real description accepted");
    }

    #[test]
    fn body_is_the_identity_evidence_event_with_text_payload_and_empty_attachments() {
        let pid = Uuid::now_v7();
        let eid = Uuid::now_v7();
        let body = build_text_evidence_body(
            eid, pid, "kid", hlc(), "mark", "scar on left forearm ~5cm", Some("primary survey"));

        assert_eq!(body.event_type, IDENTITY_EVIDENCE_EVENT_TYPE);
        assert_eq!(body.schema_version, IDENTITY_EVIDENCE_SCHEMA_VERSION);
        assert_eq!(body.patient_id, pid.to_string());
        assert_eq!(body.payload["kind"], "mark");
        assert_eq!(body.payload["description"], "scar on left forearm ~5cm");
        assert_eq!(body.payload["provenance"], "clinician-observed");
        assert_eq!(body.payload["basis"], "primary survey");
        // No attachment for a text kind — the empty vec preserves content-address identity.
        assert!(body.attachments.is_empty(), "text evidence carries no attachment");
        // additive event → recorded role, no attestation demanded
        assert_eq!(body.contributors[0]["role"], "recorded");
        // authored, legible twin naming the kind and description
        let twin = body.plaintext_twin.as_deref().unwrap();
        assert_eq!(twin, &render_text_evidence_twin("mark", "scar on left forearm ~5cm", Some("primary survey")));
        assert!(twin.contains("scar on left forearm"));
    }

    #[test]
    fn body_omits_basis_and_still_renders_a_twin_when_basis_absent() {
        let body = build_text_evidence_body(
            Uuid::now_v7(), Uuid::now_v7(), "kid", hlc(), "belongings", "blue wallet, €40, keys", None);
        assert!(body.payload.get("basis").is_none(), "absent basis omitted, never null");
        assert!(body.plaintext_twin.as_deref().unwrap().contains("blue wallet"));
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail (do not compile)**

Run: `cargo test -p cairn-node --lib identity_evidence::`
Expected: FAIL — the module/functions do not exist yet (`unresolved module` or `cannot find function`).

- [ ] **Step 3: Write the minimal implementation** — put the module body ABOVE the `tests` module in `crates/cairn-node/src/identity_evidence.rs`:

```rust
//! §5.4 TEXT identity-evidence author path (marks / belongings / EMS-context). The non-photo
//! sibling of `photo_evidence.rs`: the observation is free text in the payload, so there is no
//! blob and no attachment — the whole author path is ONE `submit_event` call. Splits into pure
//! helpers (unit-tested here) and an async orchestrator (e2e-tested in
//! tests/identity_evidence_text.rs).

use cairn_event::identity_evidence::{
    parse_text_evidence_kind, render_text_evidence_twin, text_evidence_body,
    IDENTITY_EVIDENCE_EVENT_TYPE, IDENTITY_EVIDENCE_SCHEMA_VERSION,
};
use cairn_event::{sign, EventBody, Hlc, SigningKey};
use tokio_postgres::Client;
use uuid::Uuid;

/// PURE: enforce the honest-content requirement (§5.4 / principle 4) — an evidence assertion
/// must say what was observed, so an empty or whitespace-only description is refused. This lives
/// in the library (not only the CLI edge) so EVERY caller — a future UI backend included —
/// inherits the guarantee. (Whether a UI silently supplies a default is soft policy above this
/// floor, per principle 12; the floor's job is only to refuse a genuinely empty claim.)
pub fn validate_description(description: &str) -> anyhow::Result<()> {
    if description.trim().is_empty() {
        anyhow::bail!("identity-evidence description must be non-empty (§5.4/principle 4: say what was observed)");
    }
    Ok(())
}

/// PURE: assemble the signed `EventBody` for a TEXT identity-evidence event. `kind` must already
/// be a canonical constant (the caller validates via `parse_text_evidence_kind`). The payload
/// carries the clinical framing (kind/provenance/description/basis); the twin is authored from
/// the same text; `attachments` stays empty (there is nothing to attach). The sole contributor
/// is the recording actor with role `recorded` — additive, no attestation.
pub fn build_text_evidence_body(
    event_id: Uuid,
    patient_id: Uuid,
    kid: &str,
    hlc: Hlc,
    kind: &str,
    description: &str,
    basis: Option<&str>,
) -> EventBody {
    let twin = render_text_evidence_twin(kind, description, basis);
    EventBody {
        event_id: event_id.to_string(),
        patient_id: patient_id.to_string(),
        event_type: IDENTITY_EVIDENCE_EVENT_TYPE.into(),
        schema_version: IDENTITY_EVIDENCE_SCHEMA_VERSION.into(),
        hlc,
        t_effective: None,
        signer_key_id: kid.into(),
        contributors: serde_json::json!([{"actor_id": kid, "role": "recorded"}]),
        payload: text_evidence_body(kind, description, basis),
        attachments: vec![],
        plaintext_twin: Some(twin),
    }
}

/// Author a TEXT identity-evidence event on an existing chart. Validates the description and the
/// kind FIRST so a bad call never reaches the DB (single source of truth for both checks). No
/// blob tier and a single statement, so no explicit transaction is needed — `submit_event` is
/// itself atomic. Ticks the HLC once (self-committing, like `john_doe.rs`/`photo_evidence.rs`).
/// Returns the new event id.
#[allow(clippy::too_many_arguments)] // signer + node context + the kind/description/basis inputs
pub async fn assert_text_evidence(
    client: &Client,
    sk: &SigningKey,
    kid: &str,
    node_origin: &str,
    patient_id: Uuid,
    kind: &str,
    description: &str,
    basis: Option<&str>,
) -> anyhow::Result<Uuid> {
    validate_description(description)?;
    let canonical_kind = parse_text_evidence_kind(kind)
        .ok_or_else(|| anyhow::anyhow!("unknown identity-evidence kind {kind:?}; expected one of mark|belongings|ems-context"))?;

    let hlc = crate::db::next_hlc(client, node_origin).await?;
    let event_id = Uuid::now_v7();
    let body = build_text_evidence_body(event_id, patient_id, kid, hlc, canonical_kind, description, basis);
    let signed = sign(&body, sk)?;

    client.execute("SELECT submit_event($1)", &[&signed.signed_bytes]).await?;
    Ok(event_id)
}
```

Then add the module declaration to `crates/cairn-node/src/lib.rs`:

```rust
pub mod identity_evidence;
```

(alphabetically, immediately after `pub mod identity;`).

- [ ] **Step 4: Run the pure tests + clippy to verify they pass**

Run: `cargo test -p cairn-node --lib identity_evidence:: && cargo clippy -p cairn-node`
Expected: PASS; clippy clean (no warnings on the new module).

- [ ] **Step 5: Commit**

```bash
git add crates/cairn-node/src/identity_evidence.rs crates/cairn-node/src/lib.rs
git commit -m "feat(node): text identity-evidence author path (assert_text_evidence)

One-statement submit_event author path for mark/belongings/ems-context evidence;
honest-content floor (validate_description) in the library, empty attachments.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 3: e2e read-back test (DB-gated)

**Files:**
- Create: `crates/cairn-node/tests/identity_evidence_text.rs`

**Interfaces:**
- Consumes: Task 2's `cairn_node::identity_evidence::assert_text_evidence`; existing `cairn_node::john_doe::register_john_doe`, `cairn_node::identity::{provision, load_local}`, `cairn_node::db::{connect_and_load_schema, reset_node_federation_tables, test_serial_guard}`.
- Produces: nothing consumed downstream (integration test).

- [ ] **Step 1: Write the failing e2e tests** — create `crates/cairn-node/tests/identity_evidence_text.rs`:

```rust
//! §5.4 TEXT identity-evidence e2e (marks / belongings / EMS-context). Registers a John Doe,
//! records a distinguishing-mark evidence assertion through `assert_text_evidence`, and proves:
//! the event lands with the right type/kind/description/provenance, the authored twin is legible,
//! `attachments` is empty, and the orchestrator's guards reject a bad kind / empty description
//! before writing anything. Real Postgres, gated on $CAIRN_TEST_PG.

use cairn_node::db;
use tokio_postgres::Client;

fn cs() -> Option<String> { std::env::var("CAIRN_TEST_PG").ok() }

async fn setup(c: &Client) -> (cairn_event::SigningKey, String) {
    c.batch_execute(
        "TRUNCATE event_log, actor_event, patient_chart, patient_identifier, \
         patient_demographic, patient_name, blob_store, blob_chunk CASCADE").await.unwrap();
    db::reset_node_federation_tables(c).await.unwrap();
    let (sk, kid) = cairn_event::generate_key().unwrap();
    c.execute(
        "SELECT enroll_actor('agent', '{\"model\":\"reg-stub\",\"version\":\"1\",\"skill_epoch\":\"e\"}', $1)",
        &[&kid]).await.unwrap();
    cairn_node::identity::provision(c, &sk, &kid, "test-node", "127.0.0.1:0").await.unwrap();
    (sk, kid)
}

#[tokio::test]
async fn mark_evidence_lands_with_kind_description_provenance_and_legible_twin() {
    let Some(base) = cs() else { eprintln!("skip: no CAIRN_TEST_PG"); return; };
    let _guard = db::test_serial_guard(&base).await.unwrap();  // conn string; hold until drop
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let id = cairn_node::identity::load_local(&c).await.unwrap();

    // A realistic §5.4 flow: register the unidentified patient, then record a mark.
    let (patient, _callsign) = cairn_node::john_doe::register_john_doe(
        &mut c, &sk, &kid, &id.node_id_hex, "ED", "site1", "2026-07-08",
        "unconscious ED arrival, no ID").await.unwrap();

    let event_id = cairn_node::identity_evidence::assert_text_evidence(
        &c, &sk, &kid, &id.node_id_hex, patient, "mark",
        "scar on left forearm, ~5cm, healed", Some("visible on primary survey")).await.unwrap();

    // Read the derived body view + top-level twin/attachments columns back.
    let eid = event_id.to_string();
    let r = c.query_one(
        "SELECT event_type, body->'payload'->>'kind', body->'payload'->>'description', \
                body->'payload'->>'provenance', attachments::text, plaintext_twin \
         FROM event_log WHERE event_id = $1::text::uuid", &[&eid]).await.unwrap();
    let event_type: String = r.get(0);
    let kind: String = r.get(1);
    let description: String = r.get(2);
    let provenance: String = r.get(3);
    let atts: String = r.get(4);
    let twin: String = r.get(5);

    assert_eq!(event_type, "identity.evidence.asserted");
    assert_eq!(kind, "mark");
    assert_eq!(description, "scar on left forearm, ~5cm, healed");
    assert_eq!(provenance, "clinician-observed");
    assert_eq!(atts, "[]", "a text kind carries no attachment: {atts}");
    assert!(twin.contains("scar on left forearm"), "twin is legible: {twin}");
    assert!(twin.contains("mark"), "twin names the kind: {twin}");
    assert!(twin.contains("visible on primary survey"), "twin carries the basis: {twin}");
}

#[tokio::test]
async fn assert_text_evidence_rejects_bad_kind_and_empty_description_without_writing() {
    let Some(base) = cs() else { eprintln!("skip: no CAIRN_TEST_PG"); return; };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let id = cairn_node::identity::load_local(&c).await.unwrap();
    let (patient, _callsign) = cairn_node::john_doe::register_john_doe(
        &mut c, &sk, &kid, &id.node_id_hex, "ED", "site1", "2026-07-08",
        "unconscious ED arrival, no ID").await.unwrap();

    // Unknown kind → error, no event authored.
    let bad_kind = cairn_node::identity_evidence::assert_text_evidence(
        &c, &sk, &kid, &id.node_id_hex, patient, "scar", "left forearm", None).await;
    assert!(bad_kind.is_err(), "an unknown kind must be refused");

    // Empty description → error, no event authored.
    let empty = cairn_node::identity_evidence::assert_text_evidence(
        &c, &sk, &kid, &id.node_id_hex, patient, "mark", "   ", None).await;
    assert!(empty.is_err(), "an empty description must be refused");

    // Nothing landed for this patient.
    let pid = patient.to_string();
    let n: i64 = c.query_one(
        "SELECT count(*) FROM event_log WHERE patient_id = $1::text::uuid \
         AND event_type = 'identity.evidence.asserted'", &[&pid]).await.unwrap().get(0);
    assert_eq!(n, 0, "no evidence event should have been written by the rejected calls");
}
```

- [ ] **Step 2: Run the tests to verify they fail (compile error / behavior)**

Run: `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test -p cairn-node --test identity_evidence_text`
Expected: FAIL to compile until Task 2 is in place; with Task 2 present, the tests should pass — if run before Task 2, expect `unresolved`/`no function assert_text_evidence`.

(If Task 2 is already committed, this is a green run confirming integration; the "failing" state is the pre-Task-2 compile error.)

- [ ] **Step 3: (No new implementation)** — this task adds only the test; the code under test ships in Task 2. If the read-back reveals a mismatch (e.g. a wrong column path), fix the assertion or the Task-2 code as needed and note it.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test -p cairn-node --test identity_evidence_text`
Expected: PASS (both tests). Without `CAIRN_TEST_PG` they print `skip:` and pass trivially.

- [ ] **Step 5: Commit**

```bash
git add crates/cairn-node/tests/identity_evidence_text.rs
git commit -m "test(node): e2e text identity evidence — mark lands, twin legible, guards reject

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 4: CLI subcommand `assert-identity-evidence`

**Files:**
- Modify: `crates/cairn-node/src/main.rs` (add the `AssertIdentityEvidence` variant to the `Cmd` enum after `AssertPhotoEvidence`, ~line 344; add its match arm after the `AssertPhotoEvidence` arm, ~line 905)

**Interfaces:**
- Consumes: Task 1's `cairn_event::identity_evidence::parse_text_evidence_kind`; Task 2's `cairn_node::identity_evidence::{validate_description, assert_text_evidence}`; existing `load_signing_key`, `ensure_registration_actor`, `cairn_node::db::connect`, `cairn_node::identity::load_local`.
- Produces: a runnable CLI subcommand (no downstream code consumer).

- [ ] **Step 1: Add the enum variant** — in `crates/cairn-node/src/main.rs`, after the `AssertPhotoEvidence { ... }` variant (before the closing `}` of the `Cmd` enum):

```rust
    /// Record clinician-observed TEXT identity evidence on an existing chart (§5.4): a
    /// distinguishing mark, personal belongings, or the EMS pickup context. Non-attachment —
    /// the observation is free text. OWNER ceremony: enrolls the node key as a registration
    /// actor on first use (a real UI attaches the operating clerk's *human* actor).
    AssertIdentityEvidence {
        /// The patient UUID to record evidence on.
        patient: Uuid,
        /// The evidence kind: mark | belongings | ems-context (closed set; typo-rejected).
        #[arg(long)]
        kind: String,
        /// Honest description of what was observed (required, non-empty — principle 4).
        #[arg(long)]
        description: String,
        /// How/why it was observed; for ems-context, note the relayed source here (optional).
        #[arg(long)]
        basis: Option<String>,
    },
```

- [ ] **Step 2: Add the match arm** — after the `Cmd::AssertPhotoEvidence { .. } => { ... }` arm (before the closing `}` of the `match cli.cmd`):

```rust
        Cmd::AssertIdentityEvidence { patient, kind, description, basis } => {
            // Fast-fail on a bad kind or empty description before any DB work — the library
            // re-checks both (single source of truth: parse_text_evidence_kind + validate_description).
            cairn_event::identity_evidence::parse_text_evidence_kind(&kind)
                .ok_or_else(|| anyhow::anyhow!("unknown --kind {kind:?}; expected mark|belongings|ems-context"))?;
            cairn_node::identity_evidence::validate_description(&description)?;
            let sk = load_signing_key(&cli.key, true)?;
            let kid = hex::encode(sk.verifying_key().to_bytes());
            let db = cairn_node::db::connect(&cli.conn).await?;
            let id = cairn_node::identity::load_local(&db).await?;
            ensure_registration_actor(&db, &kid).await?;

            let event_id = cairn_node::identity_evidence::assert_text_evidence(
                &db, &sk, &kid, &id.node_id_hex, patient, &kind, &description, basis.as_deref()).await?;
            println!("recorded {kind} identity evidence {event_id} on {patient}");
        }
```

- [ ] **Step 3: Verify it compiles and the subcommand is wired**

Run: `cargo build -p cairn-node && cargo run -p cairn-node -- assert-identity-evidence --help`
Expected: build succeeds; `--help` shows `--kind`, `--description`, `--basis` and the `<PATIENT>` positional.

- [ ] **Step 4: Run the full node suite + clippy**

Run: `cargo test -p cairn-node && cargo clippy --workspace`
Expected: all green; clippy clean. (DB-gated tests skip without `CAIRN_TEST_PG`; run once WITH it set to exercise Task 3 end-to-end.)

- [ ] **Step 5: Commit**

```bash
git add crates/cairn-node/src/main.rs
git commit -m "feat(node): assert-identity-evidence CLI (mark/belongings/ems-context)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Final verification (before the PR)

- [ ] `cargo fmt --all` (format), `cargo clippy --workspace` (clean).
- [ ] Full suite with the DB: `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test --workspace` — all green.
- [ ] Manual smoke on a provisioned node: register a John Doe, then
      `assert-identity-evidence <patient> --kind belongings --description "blue wallet, €40, house keys" --basis "handed over by EMS"` — prints the event id; and confirm a bad `--kind` is rejected.
- [ ] Update `docs/HANDOVER.md` + `docs/ROADMAP.md`: record this slice; move marks/belongings/EMS-context from "Remaining §5.4" to built; keep both files concise.
