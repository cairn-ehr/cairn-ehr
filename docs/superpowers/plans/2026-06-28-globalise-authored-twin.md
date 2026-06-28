# Globalise the Authored Legibility Twin — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make every event type carry an author-materialised §3.13/§4.5 plaintext legibility twin; reposition the receiver-derived skeleton to a flagged honest-degradation fallback used only when an event genuinely lacks an authored twin.

**Architecture:** Authoring side (Rust `cairn-event`) gains a pure `resolve_twin` (prefer authored, else derive) and `materialise_generic_twin` (author the twin into a body before signing). The in-DB floor (`db/015`) mirrors the same rule: prefer `b->>'plaintext_twin'`, else derive an (improved, payload-rendering) skeleton — except the two demographic types keep ADR-0034's hard requirement. Authored-vs-derived is **not stored**; it is a read-time projection of the immutable signed body (`cairn_twin_is_authored`). `cairn-sync` (spike-grade write path) is updated to use the same two helpers.

**Tech Stack:** Rust (cairn-event, cairn-sync, cairn-node), PostgreSQL 18 + `cairn_pgx` (pgrx), plpgsql/SQL.

## Global Constraints

- **AGPL-3.0**; every dependency AGPL-3.0-compatible. No new dependencies are introduced by this plan.
- **TDD** — failing test first, then minimal code (load-bearing: this touches the safety-critical in-DB floor).
- **Inline docs for a junior contributor** — every non-trivial function/branch carries a *why* comment.
- **Files under ~500 lines** where feasible; prefer small pure functions.
- **No `submit_event` re-declaration** — only the `cairn_event_twin` hook changes (the demographics-era single-source rule; avoids copy-paste drift of the validated door).
- **DB-gated tests** need `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test"` (PG18 + `cairn_pgx`). They self-serialize cluster-wide via `db::test_serial_guard`; a skipped test (no env var) prints `skipped:` and returns.
- **Demographic exception (ADR-0034):** `demographic.identifier.asserted` and `demographic.field.asserted` keep a HARD authored-twin requirement (reject twin-less). Every other type degrades honestly.
- Commit message trailer on every commit:
  `Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>`

---

## File Structure

- `docs/spec/decisions/0039-globalise-authored-legibility-twin.md` — **new** ADR (the *why*).
- `docs/spec/decisions/README.md` — add the ADR-0039 index row.
- `docs/spec/index.md` — bump spec version 0.39 → 0.40.
- `docs/spec/data-model.md` — one §3.13 sentence generalising the carried authored twin.
- `crates/cairn-event/src/lib.rs` — add `resolve_twin`, `materialise_generic_twin` (+ private `twin_is_present`); update `plaintext_twin` doc; unit tests.
- `db/015_globalise_twin.sql` — **new** migration: improved `cairn_twin_skeleton`, generalised `cairn_event_twin`, `cairn_twin_is_authored`, `event_twin_provenance` view.
- `crates/cairn-node/src/db.rs` — register `015_globalise_twin` in `SCHEMA` (13 → 14 entries).
- `crates/cairn-node/tests/twin_globalise.rs` — **new** integration tests.
- `crates/cairn-sync/src/main.rs` — apply + authoring paths use the shared helpers.

---

## Task 1: ADR-0039 + spec touch (the *why*)

**Files:**
- Create: `docs/spec/decisions/0039-globalise-authored-legibility-twin.md`
- Modify: `docs/spec/decisions/README.md:61` (append a row)
- Modify: `docs/spec/index.md:9` (version bump)
- Modify: `docs/spec/data-model.md:232` (one sentence)

**Interfaces:**
- Produces: ADR-0039 (referenced by `db/015` comments and the cairn-event doc comments in later tasks).

- [ ] **Step 1: Write the ADR**

Create `docs/spec/decisions/0039-globalise-authored-legibility-twin.md`:

```markdown
# ADR-0039 — Globalise the authored legibility twin (honest-degradation floor)

- **Status:** Accepted
- **Date:** 2026-06-28
- **Refines:** [ADR-0012](0012-schema-evolution-event-format-and-legibility-across-time.md) (principle 11 / legibility across time),
  [ADR-0034](0034-demographic-legibility-twin.md) (generalises the demographic carried twin to all event classes)

## Context

[Principle 11](../index.md) requires every event to stay human-readable for as long as it exists,
via a signed plaintext legibility twin materialised by the author — who understands the schema —
and carried forward, never re-derived by a reader that may be generations behind ([§3.13](../data-model.md#313-schema-evolution-event-format-and-the-legibility-twin)).
[ADR-0034](0034-demographic-legibility-twin.md) applied this to demographic assertions. But every
other event type still fell through the validated door's `cairn_event_twin` hook to a
*receiver-derived* skeleton (`cairn_twin_skeleton`), which is the legibility-across-time hole the
spike left open. Making the authored twin simply mandatory at the floor would reject a twin-less
event from an older or non-conformant peer — breaking set-union convergence (principle 1,
availability over consistency, and ADR-0012's no-lockstep heterogeneous fleet).

## Decision

The author-materialised twin is **global**: every conformant author renders and signs a §3.13 twin
into the body, for every event type. The in-DB floor **prefers** the authored twin and, when it is
absent or blank, **degrades honestly** — it stores the event with a mechanically-derived skeleton
twin (now rendering the payload, not just a header) rather than rejecting it. Convergence is
preserved; the derived twin is a non-authoritative local projection.

**Authored-vs-derived is not stored.** Because the immutable signed body either carries a non-empty
`plaintext_twin` or it does not, the distinction is a derivable read-time projection of
`signed_bytes` (`cairn_twin_is_authored`), exposed via the `event_twin_provenance` view for a future
re-authoring / duplicate-sweep / audit worklist. No new column, and the validated `submit_event`
door is not re-declared — only its `cairn_event_twin` hook changes.

**Demographic exception (unchanged from [ADR-0034](0034-demographic-legibility-twin.md)):** the two
demographic assertion types keep a *hard* authored-twin requirement. A twin-less demographic event
cannot come from an older peer (an older node rejects the unknown demographic type at classification),
so its absence is a same-version bug and is rejected.

## Consequences

- **Easier:** every event is legible from its author's faithful rendering across arbitrary schema
  skew; the skeleton survives only as an honest, flagged fallback; the `cairn-event::plaintext_twin`
  renderer is repositioned as the canonical generic authoring renderer (one reusable function).
- **The trade:** a twin-less event still stores a crude, receiver-derived twin — but it is flagged
  (`twin_authored = false`), so a reader/sweep can tell faithful from best-effort, and a conformant
  author never produces one.
- **Two-place rule (accepted):** "prefer non-empty authored, else derive" lives in SQL (the floor)
  and Rust (`resolve_twin`, used by cairn-sync), each unit-tested, cross-linked by comment. A future
  option is to unify it into a single pgrx function at the cost of an extension rebuild.
- **Out of scope:** the §3.13 `rendered-by` (schema + renderer version) stamp; per-type prose
  renderers for the placeholder clinical types; routing `cairn-sync` through `submit_event`.
```

- [ ] **Step 2: Add the ADR index row**

In `docs/spec/decisions/README.md`, after the `0038` row (line 61), append:

```markdown
| [0039](0039-globalise-authored-legibility-twin.md) | Globalise the authored legibility twin; honest-degradation floor for non-demographic types | Accepted (refines 0012, 0034) | 2026-06-28 |
```

- [ ] **Step 3: Bump the spec version**

In `docs/spec/index.md` line 9, change `**Spec version:** 0.39` to `**Spec version:** 0.40`.

- [ ] **Step 4: Add the §3.13 sentence**

In `docs/spec/data-model.md`, at the end of the line-232 bullet (the "There are two twins …" sentence, just before the "At the point of care…" clause OR at the bullet's end), append:

```markdown
 The **carried authored twin is global to every event class**, not only demographics: a conformant author materialises it for every event; the in-DB floor prefers it and, for non-demographic types, **degrades honestly** to a flagged mechanically-derived twin when an older or non-conformant peer omitted it (set-union is never broken), with authored-vs-derived recoverable from the signed body ([ADR-0039](decisions/0039-globalise-authored-legibility-twin.md)).
```

- [ ] **Step 5: Sanity-check the edits**

Run: `grep -n "0039" docs/spec/decisions/README.md docs/spec/data-model.md && grep -n "0.40" docs/spec/index.md`
Expected: the README row, the data-model sentence, and the version bump all present.

- [ ] **Step 6: Commit**

```bash
git add docs/spec/decisions/0039-globalise-authored-legibility-twin.md docs/spec/decisions/README.md docs/spec/index.md docs/spec/data-model.md
git commit -m "spec(twin): ADR-0039 — globalise authored legibility twin (v0.40)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 2: cairn-event — `resolve_twin` + `materialise_generic_twin`

**Files:**
- Modify: `crates/cairn-event/src/lib.rs` (add functions near `plaintext_twin` at line 274; tests in the existing `mod tests`)

**Interfaces:**
- Consumes: `plaintext_twin(&EventBody) -> String` (existing, line 274), `EventBody` (line 79), `sign`, `verify_self_described`, `generate_key`, `Hlc`, `AttachmentRef`.
- Produces:
  - `pub fn resolve_twin(body: &EventBody) -> String` — prefer non-empty `body.plaintext_twin`, else `plaintext_twin(body)`.
  - `pub fn materialise_generic_twin(body: EventBody) -> EventBody` — set `plaintext_twin = Some(plaintext_twin(&body))` iff currently blank; idempotent.

- [ ] **Step 1: Write the failing unit tests**

Add to the `mod tests` block in `crates/cairn-event/src/lib.rs` (near the existing twin tests around line 790):

```rust
// Globalised-twin helpers (ADR-0039). A reusable note body whose payload renders into a twin.
fn sample_note_body() -> EventBody {
    EventBody {
        event_id: "00000000-0000-7000-8000-000000000001".into(),
        patient_id: "00000000-0000-7000-8000-000000000002".into(),
        event_type: "note.added".into(),
        schema_version: "note/1".into(),
        hlc: Hlc { wall: 7, counter: 0, node_origin: "n".into() },
        t_effective: None,
        signer_key_id: "k".into(),
        contributors: serde_json::json!([{"actor_id": "k", "role": "recorded"}]),
        payload: serde_json::json!({"text": "BP 120/80, afebrile"}),
        attachments: vec![],
        plaintext_twin: None,
    }
}

#[test]
fn resolve_twin_prefers_authored_else_derives() {
    let mut body = sample_note_body();
    // Absent authored twin → derive (identical to the mechanical renderer).
    assert_eq!(resolve_twin(&body), plaintext_twin(&body));
    // Whitespace-only authored twin → still derive (treated as blank).
    body.plaintext_twin = Some("   \n".into());
    assert_eq!(resolve_twin(&body), plaintext_twin(&body));
    // Non-empty authored twin → carried verbatim.
    body.plaintext_twin = Some("Progress note: BP 120/80".into());
    assert_eq!(resolve_twin(&body), "Progress note: BP 120/80");
}

#[test]
fn materialise_generic_twin_fills_blank_and_is_idempotent() {
    let body = sample_note_body();
    let m = materialise_generic_twin(body.clone());
    let twin = m.plaintext_twin.as_deref().expect("twin materialised");
    assert!(!twin.trim().is_empty(), "materialised twin is non-empty");
    assert_eq!(twin, plaintext_twin(&body), "materialised == the generic rendering");
    // Idempotent: an already-authored twin is preserved unchanged.
    let mut authored = sample_note_body();
    authored.plaintext_twin = Some("kept verbatim".into());
    let m2 = materialise_generic_twin(authored);
    assert_eq!(m2.plaintext_twin.as_deref().unwrap(), "kept verbatim");
}

#[test]
fn materialised_twin_roundtrips_through_sign_verify() {
    let (sk, kid) = generate_key().unwrap();
    let mut body = sample_note_body();
    body.signer_key_id = kid;
    let body = materialise_generic_twin(body);
    let signed = sign(&body, &sk).unwrap();
    let decoded = verify_self_described(&signed.signed_bytes).unwrap();
    assert_eq!(decoded.plaintext_twin, body.plaintext_twin);
    assert!(decoded.plaintext_twin.is_some(), "a materialised twin survives the wire");
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p cairn-event resolve_twin materialise materialised_twin 2>&1 | tail -20`
Expected: FAIL — `cannot find function resolve_twin` / `materialise_generic_twin` in this scope.

- [ ] **Step 3: Implement the helpers**

In `crates/cairn-event/src/lib.rs`, immediately after the `plaintext_twin` function (after line 288), add:

```rust
/// True iff an Option twin is present and not just whitespace. The single blank-test
/// shared by `resolve_twin` and `materialise_generic_twin` (DRY).
fn twin_is_present(twin: &Option<String>) -> bool {
    matches!(twin.as_deref(), Some(t) if !t.trim().is_empty())
}

/// Resolve the twin to STORE for an event, following the globalised-twin rule (ADR-0039):
/// prefer the author-materialised twin (principle 11 — the author renders it faithfully and
/// signs it in, so a reader generations behind never re-derives from a schema it may not
/// understand); fall back to the mechanically-derived twin only when the author left it absent
/// or blank (an older / non-conformant peer). The in-DB floor (db/015 `cairn_event_twin`)
/// mirrors this exact rule for the validated write door — keep the two in sync.
pub fn resolve_twin(body: &EventBody) -> String {
    if twin_is_present(&body.plaintext_twin) {
        // Safe: twin_is_present guarantees Some(non-blank).
        body.plaintext_twin.clone().unwrap()
    } else {
        plaintext_twin(body)
    }
}

/// Materialise the generic authored twin into a body BEFORE signing, so a conformant author
/// globalises the §3.13 twin in one call (ADR-0039). Idempotent: an already-authored twin
/// (e.g. a demographic builder's tailored twin) is left untouched, so this is safe to call on
/// any body. Must run before `sign`, as the twin becomes part of the signed/content-addressed body.
pub fn materialise_generic_twin(mut body: EventBody) -> EventBody {
    if !twin_is_present(&body.plaintext_twin) {
        body.plaintext_twin = Some(plaintext_twin(&body));
    }
    body
}
```

Also update the doc comment on `plaintext_twin` (line 271-274) — change the opening to note its repositioned role:

```rust
/// Mechanically derive the §3.13 plaintext legibility twin from a body. This is BOTH the
/// canonical generic *authoring* renderer (a conformant author materialises this into the body
/// via `materialise_generic_twin`, then signs it in — ADR-0039) AND the crude shape the floor
/// falls back to when an event arrives without an authored twin. Crude on purpose: derivable by
/// *any* node from the structured content, so a node generations behind still reads it as prose.
pub fn plaintext_twin(body: &EventBody) -> String {
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p cairn-event resolve_twin materialise materialised_twin 2>&1 | tail -20`
Expected: PASS (3 tests).

- [ ] **Step 5: Full crate test + clippy**

Run: `cargo test -p cairn-event 2>&1 | tail -15 && cargo clippy -p cairn-event 2>&1 | tail -5`
Expected: all green; no clippy warnings.

- [ ] **Step 6: Commit**

```bash
git add crates/cairn-event/src/lib.rs
git commit -m "feat(cairn-event): resolve_twin + materialise_generic_twin (ADR-0039)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 3: db/015 migration + registration + integration tests

**Files:**
- Create: `db/015_globalise_twin.sql`
- Modify: `crates/cairn-node/src/db.rs:3` (array size 13 → 14) and `:20` (append entry)
- Create: `crates/cairn-node/tests/twin_globalise.rs`

**Interfaces:**
- Consumes: `submit_event` (db/005, unchanged), `cairn_body` (pgrx), `cairn_check_identifier_assertion` (db/010), `cairn_check_demographic_field` (db/011), `cairn_event::demographics::{IdentifierAssertion, identifier_assertion_body, render_identifier_twin}`, `db::{connect_and_load_schema, test_serial_guard}`.
- Produces (SQL): `cairn_twin_skeleton(text, jsonb)` (improved), `cairn_event_twin(text, jsonb)` (generalised), `cairn_twin_is_authored(bytea) -> boolean`, view `event_twin_provenance(event_id, twin_authored)`.

- [ ] **Step 1: Write the failing integration tests** (full test file below in Step 5 — write it first, here, before the migration exists)

Create `crates/cairn-node/tests/twin_globalise.rs` with the contents shown in **Step 5 below**, then proceed to Step 2 to watch it fail. (The file is listed once, in Step 5, to keep the test code in one place — write it now.)

- [ ] **Step 2: Run the integration tests to verify they fail**

Run: `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test -p cairn-node --test twin_globalise 2>&1 | tail -25`
Expected: FAIL — `cairn_twin_is_authored` / `event_twin_provenance` do not exist (the migration is not written/registered yet), and the twin-less `note.added` still gets the old header-only skeleton so the `contains("BP 120/80")` assertion fails. (If `$CAIRN_TEST_PG` is unset every test prints `skipped:` and passes vacuously — set the env var to get a real red.)

- [ ] **Step 3: Write the migration**

Create `db/015_globalise_twin.sql`:

```sql
-- Cairn — globalise the authored legibility twin (ADR-0039, refines ADR-0012/0034).
--
-- Every event type now carries an author-materialised §3.13/§4.5 plaintext twin. The floor
-- PREFERS the authored twin; for non-demographic types it degrades HONESTLY to a derived
-- skeleton when the author omitted it (older / non-conformant peer), so set-union convergence
-- is never broken. Demographic types keep ADR-0034's HARD requirement. submit_event (db/005)
-- is reused verbatim — only the cairn_event_twin hook changes (single-source door, no drift).

BEGIN;

-- Improved mechanical fallback: now renders the PAYLOAD too (closes the db/005 TODO), so a
-- derived twin is still genuinely legible. Crude + deterministic by design.
CREATE OR REPLACE FUNCTION cairn_twin_skeleton(p_type text, b jsonb)
RETURNS text LANGUAGE sql IMMUTABLE AS $$
    SELECT format('[%s] %s for patient %s%s',
                  p_type,
                  b ->> 'schema_version',
                  b ->> 'patient_id',
                  CASE WHEN b -> 'payload' IS NULL THEN ''
                       ELSE E'\n' || jsonb_pretty(b -> 'payload') END);
$$;

-- The generalised per-type twin hook. Demographic types: structural floor + HARD authored-twin
-- requirement (ADR-0034). Every other type: prefer the authored twin; derive+flag if absent
-- (ADR-0039 honest degradation). The authored-vs-derived flag is NOT stored here — it is
-- recoverable from signed_bytes via cairn_twin_is_authored below.
CREATE OR REPLACE FUNCTION cairn_event_twin(p_type text, b jsonb)
RETURNS text LANGUAGE plpgsql AS $$
DECLARE
    v_twin        text    := b ->> 'plaintext_twin';
    v_authored    boolean := v_twin IS NOT NULL AND length(trim(v_twin)) > 0;
    v_demographic boolean := false;
BEGIN
    -- Per-type structural floor (demographics only, for now).
    IF p_type = 'demographic.identifier.asserted' THEN
        PERFORM cairn_check_identifier_assertion(b);
        v_demographic := true;
    ELSIF p_type = 'demographic.field.asserted' THEN
        PERFORM cairn_check_demographic_field(b);
        v_demographic := true;
    END IF;

    -- Authored twin present → carry it verbatim (principle 11; the conformant path, EVERY type).
    IF v_authored THEN
        RETURN v_twin;
    END IF;

    -- Absent/blank twin:
    --   demographic types HARD-require it (ADR-0034 — a twin-less demographic event is a
    --     same-version bug; an older node rejects the unknown type at classification).
    --   every other type degrades honestly to a flagged derived skeleton (ADR-0039).
    IF v_demographic THEN
        RAISE EXCEPTION 'submit_event: demographic assertion requires a non-empty authored twin (§4.5)';
    END IF;
    RETURN cairn_twin_skeleton(p_type, b);
END;
$$;

-- Read-time provenance: was the twin author-materialised, or derived by the floor? Recovered
-- from the immutable signed body (the author either signed a non-empty plaintext_twin or did
-- not), so no stored flag is needed. cairn_body is the pgrx COSE/CBOR parser (db/005 dependency).
CREATE OR REPLACE FUNCTION cairn_twin_is_authored(p_signed bytea)
RETURNS boolean LANGUAGE sql STABLE AS $$
    SELECT t IS NOT NULL AND length(trim(t)) > 0
    FROM (SELECT cairn_body(p_signed) ->> 'plaintext_twin' AS t) s;
$$;

-- Worklist surface for a future re-authoring / duplicate-sweep / audit pass: which stored
-- events carry an author-faithful twin vs a best-effort derived one.
CREATE OR REPLACE VIEW event_twin_provenance AS
    SELECT event_id, cairn_twin_is_authored(signed_bytes) AS twin_authored
    FROM event_log;

GRANT SELECT ON event_twin_provenance TO cairn_agent;

COMMIT;
```

- [ ] **Step 4: Register the migration**

In `crates/cairn-node/src/db.rs`:
- Line 3: change `const SCHEMA: [(&str, &str); 13] = [` to `const SCHEMA: [(&str, &str); 14] = [`.
- After line 20 (the `014_demographics_address` entry), add:

```rust
    ("015_globalise_twin", include_str!("../../../db/015_globalise_twin.sql")),
```

- [ ] **Step 5: The integration test file (written in Step 1; shown here in full)**

Create `crates/cairn-node/tests/twin_globalise.rs`:

```rust
//! Integration coverage for ADR-0039 — the globalised authored legibility twin. Real Postgres,
//! gated on `$CAIRN_TEST_PG`, serialized cluster-wide via `db::test_serial_guard`. Proves: an
//! authored twin on a non-demographic event passes through verbatim and reads back as authored;
//! a twin-less event degrades honestly to a flagged, payload-rendering derived skeleton (set-union
//! preserved); a twin-less demographic event is still HARD-rejected (ADR-0034).
use cairn_event::demographics::{identifier_assertion_body, IdentifierAssertion};
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

/// Author + sign + submit one note.added for `patient`, optionally carrying an authored twin.
async fn submit_note(
    c: &Client, sk: &SigningKey, kid: &str, patient: Uuid, wall: i64, twin: Option<&str>,
) -> Result<u64, tokio_postgres::Error> {
    let body = EventBody {
        event_id: Uuid::now_v7().to_string(),
        patient_id: patient.to_string(),
        event_type: "note.added".into(),
        schema_version: "note/1".into(),
        hlc: Hlc { wall, counter: 0, node_origin: "n".into() },
        t_effective: None,
        signer_key_id: kid.into(),
        contributors: serde_json::json!([{"actor_id": kid, "role": "recorded"}]),
        payload: serde_json::json!({"text": "BP 120/80, afebrile"}),
        attachments: vec![],
        plaintext_twin: twin.map(|t| t.to_string()),
    };
    let signed = sign(&body, sk).unwrap();
    c.execute("SELECT submit_event($1)", &[&signed.signed_bytes]).await
}

#[tokio::test]
async fn authored_twin_on_note_passes_through_verbatim_and_reads_authored() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let p = Uuid::now_v7();
    submit_note(&c, &sk, &kid, p, 1, Some("Progress note: BP 120/80, afebrile"))
        .await.expect("authored-twin note accepted");
    let p_str = p.to_string();
    let twin: String = c.query_one(
        "SELECT plaintext_twin FROM event_log WHERE patient_id::text=$1", &[&p_str])
        .await.unwrap().get(0);
    assert_eq!(twin, "Progress note: BP 120/80, afebrile", "authored twin carried verbatim");
    let authored: bool = c.query_one(
        "SELECT cairn_twin_is_authored(signed_bytes) FROM event_log WHERE patient_id::text=$1",
        &[&p_str]).await.unwrap().get(0);
    assert!(authored, "an authored twin reads back as authored");
    // The provenance view agrees.
    let view_authored: bool = c.query_one(
        "SELECT ep.twin_authored FROM event_twin_provenance ep
           JOIN event_log el USING (event_id) WHERE el.patient_id::text=$1",
        &[&p_str]).await.unwrap().get(0);
    assert!(view_authored, "event_twin_provenance flags it authored");
}

#[tokio::test]
async fn twinless_note_degrades_to_flagged_skeleton() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let p = Uuid::now_v7();
    submit_note(&c, &sk, &kid, p, 1, None)
        .await.expect("twin-less note still accepted (set-union preserved)");
    let p_str = p.to_string();
    let twin: String = c.query_one(
        "SELECT plaintext_twin FROM event_log WHERE patient_id::text=$1", &[&p_str])
        .await.unwrap().get(0);
    assert!(twin.starts_with("[note.added]"), "derived skeleton twin, got: {twin}");
    assert!(twin.contains("BP 120/80"), "skeleton renders the payload (db/005 TODO closed)");
    let authored: bool = c.query_one(
        "SELECT cairn_twin_is_authored(signed_bytes) FROM event_log WHERE patient_id::text=$1",
        &[&p_str]).await.unwrap().get(0);
    assert!(!authored, "a derived twin reads back as NOT authored (honest flag)");
}

#[tokio::test]
async fn twinless_demographic_is_still_hard_rejected() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c).await;
    let p = Uuid::now_v7();
    // A structurally VALID identifier assertion, but with the authored twin DROPPED.
    let a = IdentifierAssertion {
        value: "943 476 5919", system: "nhs-number", provenance: "document-verified",
        normalized: Some("9434765919"), profile: Some("nhs-number@b3-abc"), use_: Some("national-id"),
    };
    let body = EventBody {
        event_id: Uuid::now_v7().to_string(),
        patient_id: p.to_string(),
        event_type: "demographic.identifier.asserted".into(),
        schema_version: "demographic.identifier/1".into(),
        hlc: Hlc { wall: 1, counter: 0, node_origin: "n".into() },
        t_effective: None,
        signer_key_id: kid.clone(),
        contributors: serde_json::json!([{"actor_id": kid, "role": "recorded"}]),
        payload: identifier_assertion_body(&a),
        attachments: vec![],
        plaintext_twin: None, // <-- the floor must reject this for a demographic type
    };
    let signed = sign(&body, &sk).unwrap();
    let err = c.execute("SELECT submit_event($1)", &[&signed.signed_bytes])
        .await.expect_err("twin-less demographic must be rejected");
    let msg = err.to_string();
    assert!(msg.contains("authored twin") || msg.contains("§4.5"), "rejection cites the twin: {msg}");
    // Triple-gate: nothing landed in the log.
    let n: i64 = c.query_one(
        "SELECT count(*) FROM event_log WHERE patient_id::text=$1", &[&p.to_string()])
        .await.unwrap().get(0);
    assert_eq!(n, 0, "rejected demographic event is not stored");
}
```

- [ ] **Step 6: Run the integration tests to verify they pass**

Run: `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test -p cairn-node --test twin_globalise 2>&1 | tail -25`
Expected: PASS (3 tests).

- [ ] **Step 7: Regression — the existing demographics + attestation suites still pass**

Run: `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test -p cairn-node --test demographics --test attestation 2>&1 | tail -20`
Expected: PASS (the demographic happy-path still carries its authored twin verbatim; twin-less `note.added` baselines in attestation.rs now get a derived skeleton instead of the old header-only skeleton, which those tests do not assert on, so they remain green).

- [ ] **Step 8: Commit**

```bash
git add db/015_globalise_twin.sql crates/cairn-node/src/db.rs crates/cairn-node/tests/twin_globalise.rs
git commit -m "feat(db): §3.13 globalise authored twin — floor prefers authored, derive+flag fallback (ADR-0039)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 4: cairn-sync mirror fix

**Files:**
- Modify: `crates/cairn-sync/src/main.rs:25` (imports), `:173` (apply path), `:327-348` (authoring path)

**Interfaces:**
- Consumes: `resolve_twin`, `materialise_generic_twin` (Task 2).

- [ ] **Step 1: Update the imports**

In `crates/cairn-sync/src/main.rs` line 25, change:

```rust
use cairn_event::{blob_address, plaintext_twin, sign, sign_attestation, verify_self_described, AttestationBody, EventBody, Hlc, SigningKey};
```

to (drop `plaintext_twin` — now unused directly; add the two helpers):

```rust
use cairn_event::{blob_address, materialise_generic_twin, resolve_twin, sign, sign_attestation, verify_self_described, AttestationBody, EventBody, Hlc, SigningKey};
```

- [ ] **Step 2: Fix the apply path (carry authored, derive only if absent)**

In `apply_signed`, line 173, change `let twin = plaintext_twin(&body);` to:

```rust
    // ADR-0039: carry the author-materialised twin; derive only if the author omitted it.
    // Mirrors the in-DB floor (db/015 cairn_event_twin) for this spike-grade write path.
    let twin = resolve_twin(&body);
```

- [ ] **Step 3: Fix the authoring path (materialise before signing)**

In the event-authoring function (around lines 327-348), after the `EventBody { … plaintext_twin: None }` literal is constructed (line 343) and BEFORE `let signed = sign(&body, sk)?;` (line 345), insert a materialise step and derive the stored twin from the body. Replace:

```rust
    let signed = sign(&body, sk)?;
    let body_json = serde_json::to_string(&body.payload)?;
    let contributors_json = serde_json::to_string(&body.contributors)?;
    let twin = plaintext_twin(&body);
```

with:

```rust
    // ADR-0039: globalise the authored twin — materialise it into the body BEFORE signing, so
    // this node emits a conformant author-faithful twin rather than relying on receivers to derive.
    let body = materialise_generic_twin(body);
    let signed = sign(&body, sk)?;
    let body_json = serde_json::to_string(&body.payload)?;
    let contributors_json = serde_json::to_string(&body.contributors)?;
    let twin = resolve_twin(&body);
```

(Note: `let body = …` shadows the prior `body`; the original was `let body = EventBody { … }`, so shadowing with the materialised value is clean and the later `sign`, `body.payload`, `body.contributors`, `body.event_id` all read the materialised body.)

- [ ] **Step 4: Build + clippy (the twin-selection logic itself is unit-tested in Task 2)**

Run: `cargo build -p cairn-sync 2>&1 | tail -10 && cargo clippy -p cairn-sync 2>&1 | tail -10`
Expected: builds clean; no `unused import: plaintext_twin` warning; no clippy warnings.

- [ ] **Step 5: Commit**

```bash
git add crates/cairn-sync/src/main.rs
git commit -m "feat(cairn-sync): carry authored twin, materialise on authoring (ADR-0039)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 5: Full verification + HANDOVER/ROADMAP refresh

**Files:**
- Modify: `docs/HANDOVER.md`, `docs/ROADMAP.md`

- [ ] **Step 1: Full workspace test + clippy**

Run: `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test --workspace 2>&1 | tail -30`
Then: `cargo clippy --workspace 2>&1 | tail -10`
Expected: all suites green; no clippy warnings. If anything fails, fix it (or, if out of scope, file a GitHub issue per house rule 5) before proceeding.

- [ ] **Step 2: Refresh HANDOVER.md**

Replace the top "This session" paragraph with a concise summary: globalised the §3.13/§4.5 authored twin to every event type (ADR-0039, spec v0.40); the floor prefers the authored twin and degrades honestly to a flagged, payload-rendering derived skeleton for non-demographic types (set-union preserved); demographic types keep ADR-0034's hard requirement; authored-vs-derived is a derivable read-time projection (`cairn_twin_is_authored` / `event_twin_provenance`), no stored flag, no `submit_event` re-declaration; closed the db/005:29 TODO; cairn-sync mirrors the rule. Note new tests (cairn-event 3 unit, cairn-node 3 integration) and that the deferral "globalise the authored twin" is now closed. Move that item out of the "Open threads" menu.

- [ ] **Step 3: Refresh ROADMAP.md**

In the Phase 1 "Legibility twin" line and/or the Phase 4 demographics line, mark the authored-twin globalisation done (ADR-0039); drop "globalise the authored twin" from the "Next" list.

- [ ] **Step 4: Commit**

```bash
git add docs/HANDOVER.md docs/ROADMAP.md
git commit -m "docs: HANDOVER/ROADMAP — authored twin globalised (ADR-0039)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

- [ ] **Step 5: Push + open PR**

```bash
git push -u origin globalise-authored-twin
gh pr create --base main --title "Globalise the authored legibility twin (ADR-0039)" --body "$(cat <<'EOF'
## Summary
Globalises the §3.13/§4.5 author-materialised legibility twin to every event type (principle 11).

- The in-DB floor (`db/015`) prefers the authored twin; for non-demographic types it **degrades honestly** to a flagged, payload-rendering derived skeleton when the author omitted it (older/non-conformant peer) — set-union convergence preserved. Demographic types keep ADR-0034's **hard** requirement.
- Authored-vs-derived is a **derivable** read-time projection of the signed body (`cairn_twin_is_authored`, `event_twin_provenance` view) — no stored flag, no `submit_event` re-declaration (only the `cairn_event_twin` hook changes).
- `cairn-event` gains pure `resolve_twin` + `materialise_generic_twin`; `cairn-sync` carries the authored twin on apply and materialises it on authoring.
- Improved `cairn_twin_skeleton` now renders the payload — closes the `db/005:29` TODO.
- New ADR-0039 (refines ADR-0012/0034); spec v0.39 → v0.40.

## Tests
- cairn-event: 3 unit tests (resolve/materialise/round-trip).
- cairn-node: 3 integration tests (authored verbatim + flag; twin-less degrade + flag; twin-less demographic hard-reject). Demographics + attestation suites regress green.

## Design
`docs/superpowers/specs/2026-06-28-globalise-authored-twin-design.md`

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

---

## Self-Review (completed by plan author)

- **Spec coverage:** Decision 1 (honest degradation) → Task 3 floor + tests; Decision 2 (scope: door + cairn-sync) → Tasks 3 & 4; Decision 3 (one generic renderer) → Task 2; derivable flag → Task 3 (`cairn_twin_is_authored`/view); ADR + spec → Task 1; demographic exception → Task 3 floor + the hard-reject test. All covered.
- **Placeholder scan:** no TBD/TODO-in-plan; every code step shows full code.
- **Type consistency:** `resolve_twin(&EventBody)->String`, `materialise_generic_twin(EventBody)->EventBody`, `twin_is_present(&Option<String>)->bool`, `cairn_twin_is_authored(bytea)->boolean`, `event_twin_provenance(event_id, twin_authored)` — used consistently across Tasks 2/3/4. SCHEMA size 13 → 14 matches the single appended entry.
```
