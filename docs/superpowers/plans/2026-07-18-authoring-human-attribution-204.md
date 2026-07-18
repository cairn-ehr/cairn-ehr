# Authoring-Human Attribution (#204 / ADR-0053) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let a clinical (medication) event carry an authenticated human author — `{human,"authored"}` + `{node,"recorded"}`, signed by the human, node holding custody — floor-enforced unforgeable at the authoring door and graded (never refused) at the sync door, realizing `session.user ≠ event.author`.

**Architecture:** A pure `with_human_author` transform (cairn-event) rewrites a device-shaped body to prepend the `authored` human and make them the signer; `seal_sign_submit` gains an optional `author` (the human signs the sealed bytes while `ensure_unwrap_key` keeps the node as custodian). The db/005 strict door gains `cairn_authorship_bound` — a responsibility-bearing contributor must be the signer or the verified attester (the #195 binding, one field over). db/020 is untouched. A pure `classify_authorship_confidence` predicate defines the `attested`/`unverified`/`device` grade for consumers.

**Tech Stack:** Rust (workspace `crates/`), PostgreSQL ≥ 18 + the `cairn_pgx` pgrx extension (the in-DB floor), `tokio-postgres`, `serde_json`.

**Source of truth:** the design doc [`docs/superpowers/specs/2026-07-18-authoring-human-attribution-204.md`](../specs/2026-07-18-authoring-human-attribution-204.md). Read it before starting.

## Global Constraints

- **AGPL-3.0** for all code; every dependency AGPL-3.0-compatible. No new dependency is needed for this plan.
- **TDD** — failing test first, then code (load-bearing on this §9 safety-critical surface).
- **Rust toolchain pinned `1.96.0`**; the workspace mirrors the CI `-D warnings` lint gate — clippy must be clean.
- **Formatting gate runs on BOTH cargo trees:** `cargo fmt --all` AND `cargo fmt --manifest-path extensions/cairn_pgx/Cargo.toml` (the workspace-*excluded* extension crate — the `81dc025` lesson). Both must be clean before any commit.
- **All tests pass before committing** (house rule 6). DB-gated tests need `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test"` pointing at a PG18 cluster with **`cairn_pgx ≥ 0.3.0`** installed; without it they self-skip locally (`eprintln!("skipped: set CAIRN_TEST_PG")`) and run in CI.
- **Never hard-code cryptographic material in tests** (house rule 6) — derive keys at runtime via `cairn_event::generate_key()`, never a literal.
- **Junior-legible inline comments** on every non-trivial fn/module: *why* it exists and *how* it fits, not just *what*.
- **Files under ~500 lines** where feasible; prefer small pure functions.
- **DB files are re-run on every connect** (`connect_and_load_schema`): only `CREATE OR REPLACE FUNCTION` and additive edits are safe. This plan adds one function and edits `submit_event` (both `CREATE OR REPLACE`) — no view-widening, no `CREATE TABLE` column additions.
- **Uniform role:** the human author is always `authored` this slice (`ordered`/`co-signed` reserved for a future prescribing stream).
- **db/020 (the apply/sync door) is NOT modified** — admit-and-grade, no new refusal (design §4/§6).

---

### Task 1: ADR-0053 + spec §3.9/§3.10 prose + version bump

**Files:**
- Create: `docs/spec/decisions/0053-per-write-human-authorship.md`
- Modify: `docs/spec/data-model.md` (§3.9 authorship binding + grade; §3.10 `session≠author` realization + deferral note)
- Modify: `docs/spec/index.md` (version 0.53 → 0.54; add ADR-0053 to the decision index)

**Interfaces:**
- Produces: the canonical *why* (ADR-0053) and *what* (spec prose) that the code tasks reference. No code interface.

- [ ] **Step 1: Write ADR-0053**

Create `docs/spec/decisions/0053-per-write-human-authorship.md` with the immutable-ADR header and three sections, porting the design doc:

```markdown
# ADR-0053 — Per-write human authorship: the authoring signature, the authorship binding, and the authorship-confidence grade

- **Status:** Accepted
- **Date:** 2026-07-18
- **Refines:** [ADR-0008](0008-point-of-care-identity-possession-and-salvage.md) (implements its `session.user ≠ event.author` invariant at the data/floor/CLI layer), [ADR-0007](0007-authorship-and-accountability.md) (the authorship half of the compositional contributor set), [ADR-0051](0051-contributor-role-vocabulary-floor-and-responsibility-wire-shape.md) (extends the contributor floor from responsibility to authorship; reuses the author-vs-admit asymmetry), [ADR-0052](0052-born-sealed-clinical-bodies.md) (the human signs the *sealed* body; the node keeps custody), [ADR-0012](0012-schema-evolution-event-format-and-legibility-across-time.md) (grade-don't-refuse under version skew).
- **Resolves:** [#204](https://github.com/cairn-ehr/cairn-ehr/issues/204) (2026-07-15 review finding C3).

## Context

[Port the design doc's "Problem": every clinical event mints only {node,"recorded"};
responsibility (ADR-0049) shipped first; §3.9's "AI-generated iff non-human author + no
responsibility-bearing human" makes every un-attested med row read as machine content;
ADR-0051's `recorded` made that honest but scheduled this slice to end the interim; the
load-bearing `session.user ≠ event.author` invariant had no implementation.]

## Decision

Canonical home: spec §3.9 / §3.10. No new event type; no new envelope field; no schema
migration (the contributor-set fields exist from day one — this fixes an authoring path and a
floor binding).

[Port the design doc's seven ratified decisions verbatim: (1) cut; (2) human signs / node
custody; (3) authorship-only wire shape; (4) uniform `authored`; (5) strict-door authorship
binding; (6) apply admits + grades, no new refusal; (7) one shared classifier.]

### Why apply admits-and-grades

[Port the design doc's "Why apply admits-and-grades" — lead with PAPER-PARITY (an
unverifiable initialled note stays in the chart; refusing a lawful-but-unverifiable future med
event would make a real medication invisible, which is inferior to paper), then ADR-0012
never-refuse-what-you-can't-understand, then the ADR-0051 author-vs-admit consistency, then
attributable-not-silent, then the one-predicate mitigation.]

## Consequences

- **Easier:** the authorship half of principle 10 now ships; every med row can read as
  human-authored, not machine content; the suppression owner-gate recognises the human author
  for free.
- **Harder / trusted surface:** `cairn_authorship_bound` joins the reviewed floor; the
  strict-enforce / apply-grade split must not be "simplified" into symmetry (the doc comment
  carries the warning, as ADR-0051's does).
- **The bet:** that the human-signs case covers point-of-care authoring until the deferred
  token-author path (verbal orders, AI-scribe) is needed; the "verified attester" arm of the
  binding and the grade ladder already reserve room for it.
- **Deferred (UI layer):** durable session-decoupled drafts and `sign-as` salvage; the
  `asserted` grade (named-no-key); author+responsibility on one event.
```

- [ ] **Step 2: Update spec §3.9 and §3.10**

In `docs/spec/data-model.md` §3.9, add a bullet after the responsibility bullet stating the **authorship binding** (a responsibility-bearing contributor whose actor resolves to a human must be the event's signer or a verified attester — the #195 binding extended to authorship, floor-enforced at submit, graded not refused at apply) and the **authorship-confidence grade** (`attested`/`unverified`/`device`), and make explicit that a bearing role carried *without* a `responsibility` object is the legitimate "authored, not-yet-vouched" state. In §3.10, add a bullet stating the invariant is now *realized*: the authoring signature is the per-write attribution act (`signer_key_id = author`, node = recorded/custody), with drafts + `sign-as` explicitly deferred to the UI layer. Cite ADR-0053.

- [ ] **Step 3: Bump version and index**

In `docs/spec/index.md`, change `**Spec version:** 0.53` → `0.54`, and add the ADR-0053 row to the decision index table (mirroring the ADR-0052 row's format): `| [0053](...) | Per-write human authorship: authoring signature + authorship binding + authorship-confidence grade | §3.9/§3.10 (refines 0008/0007/0051/0052/0012) |`.

- [ ] **Step 4: Build the docs to verify no broken references**

Run: `uv run --with-requirements docs/requirements.txt -- mkdocs build`
Expected: build succeeds, no warnings about the new ADR's internal links.

- [ ] **Step 5: Commit**

```bash
git add docs/spec/decisions/0053-per-write-human-authorship.md docs/spec/data-model.md docs/spec/index.md
git commit -m "docs(#204): ADR-0053 — per-write human authorship (session≠author); spec §3.9/§3.10, v0.54"
```

---

### Task 2: cairn-event pure functions — `with_human_author` + `classify_authorship_confidence`

**Files:**
- Modify: `crates/cairn-event/src/contributor.rs` (add the transform, the grade enum, the classifier, and their unit tests)

**Interfaces:**
- Consumes: `crate::EventBody` (fields `contributors: serde_json::Value`, `signer_key_id: String`), `classify_role`, `RolePartition`.
- Produces:
  - `pub fn with_human_author(body: EventBody, human_kid: &str) -> EventBody`
  - `pub enum AuthorshipConfidence { Attested, Unverified, Device }`
  - `pub fn classify_authorship_confidence(contributors: &serde_json::Value, signer_key_id: &str, verified_attester: Option<&str>) -> AuthorshipConfidence`

- [ ] **Step 1: Write the failing tests**

Append to the `#[cfg(test)] mod tests` block in `crates/cairn-event/src/contributor.rs`:

```rust
#[test]
fn with_human_author_prepends_authored_and_makes_human_the_signer() {
    // A device-shaped body (node recorded, node signs) gains the human author IN
    // FRONT, and the human becomes the signer — session(node) ≠ author(human).
    let body = crate::EventBody {
        event_id: "e".into(),
        patient_id: "p".into(),
        event_type: "clinical.medication.asserted".into(),
        schema_version: "clinical.medication/1".into(),
        hlc: crate::Hlc { wall: 1, counter: 0, node_origin: "n".into() },
        t_effective: None,
        signer_key_id: "NODEKID".into(),
        contributors: serde_json::json!([{"actor_id": "NODEKID", "role": "recorded"}]),
        payload: serde_json::json!({}),
        attachments: vec![],
        plaintext_twin: Some("twin".into()),
    };
    let out = with_human_author(body, "HUMANKID");
    assert_eq!(out.signer_key_id, "HUMANKID");
    assert_eq!(out.contributors[0]["actor_id"], "HUMANKID");
    assert_eq!(out.contributors[0]["role"], "authored");
    assert!(out.contributors[0].get("responsibility").is_none());
    // The device recorded contributor is preserved after the human author.
    assert_eq!(out.contributors[1]["actor_id"], "NODEKID");
    assert_eq!(out.contributors[1]["role"], "recorded");
}

#[test]
fn authorship_grade_attested_when_bearing_author_is_the_signer() {
    let c = serde_json::json!([
        {"actor_id": "H", "role": "authored"},
        {"actor_id": "N", "role": "recorded"}]);
    assert_eq!(classify_authorship_confidence(&c, "H", None), AuthorshipConfidence::Attested);
}

#[test]
fn authorship_grade_attested_when_bearing_author_is_the_verified_attester() {
    let c = serde_json::json!([{"actor_id": "H", "role": "attested",
                                "responsibility": {"held_by": "H"}}]);
    // signer is the node, but the bearing human is the verified attester.
    assert_eq!(classify_authorship_confidence(&c, "N", Some("H")), AuthorshipConfidence::Attested);
}

#[test]
fn authorship_grade_unverified_when_bearing_author_is_neither_signer_nor_attester() {
    let c = serde_json::json!([
        {"actor_id": "H", "role": "authored"},   // claimed human author
        {"actor_id": "N", "role": "recorded"}]);
    // signed by the node, no token for H — a forgery OR a future credential; either way unverified.
    assert_eq!(classify_authorship_confidence(&c, "N", None), AuthorshipConfidence::Unverified);
}

#[test]
fn authorship_grade_device_when_no_bearing_contributor() {
    let c = serde_json::json!([{"actor_id": "N", "role": "recorded"}]);
    assert_eq!(classify_authorship_confidence(&c, "N", None), AuthorshipConfidence::Device);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p cairn-event contributor::tests::authorship 2>&1 | tail; cargo test -p cairn-event contributor::tests::with_human_author 2>&1 | tail`
Expected: FAIL — `cannot find function with_human_author` / `classify_authorship_confidence` / `AuthorshipConfidence`.

- [ ] **Step 3: Implement the transform and the classifier**

Add to `crates/cairn-event/src/contributor.rs` (above the `#[cfg(test)]` block):

```rust
use crate::EventBody;

/// Rewrite a device-shaped clinical body so a human takes AUTHORSHIP of it (#204 /
/// ADR-0053): prepend an `authored` contributor for the human (no `responsibility`
/// object — "authored, not-yet-vouched", a legitimate §3.9 state) and make the human
/// the event's signer. The device `recorded` contributor is preserved AFTER the
/// human — mixed sets like `{human, authored} + {node, recorded}` are compositional
/// authorship working as designed (ADR-0051). Pure; the caller then signs the sealed
/// bytes with the human's key while the node keeps custody (session ≠ author).
pub fn with_human_author(mut body: EventBody, human_kid: &str) -> EventBody {
    let author = serde_json::json!({"actor_id": human_kid, "role": "authored"});
    match body.contributors.as_array_mut() {
        Some(arr) => arr.insert(0, author),
        None => body.contributors = serde_json::json!([author]),
    }
    body.signer_key_id = human_kid.to_string();
    body
}

/// The authorship-confidence grade an event carries (ADR-0008 "a grade, not a gate";
/// ADR-0053). The single, shared reading every consumer must use so an unverifiable
/// claim is never displayed as authenticated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthorshipConfidence {
    /// A responsibility-bearing human author, authenticated as the signer or a verified attester.
    Attested,
    /// A responsibility-bearing author this node cannot verify (actor ≠ signer, no verifiable
    /// token) — a forgery OR an author authenticated by a scheme this node is too old to parse.
    /// Rendered "authorship claimed, not authenticated here", never `Attested`, and upgradable.
    Unverified,
    /// No responsibility-bearing contributor — the honest device-additive default (`recorded`).
    Device,
}

/// Grade an event's authorship from its contributor set, the verified signer, and the
/// verified attester (if any). Pure; total. A bearing contributor is "authenticated"
/// iff its actor is the signer or the verified attester; every bearing author must be
/// authenticated for `Attested`, else `Unverified`; no bearing contributor at all is
/// `Device`.
pub fn classify_authorship_confidence(
    contributors: &serde_json::Value,
    signer_key_id: &str,
    verified_attester: Option<&str>,
) -> AuthorshipConfidence {
    let bearing: Vec<&str> = contributors
        .as_array()
        .map(|a| {
            a.iter()
                .filter(|e| {
                    classify_role(e.get("role").and_then(|r| r.as_str()).unwrap_or(""))
                        == RolePartition::Bearing
                })
                .filter_map(|e| e.get("actor_id").and_then(|v| v.as_str()))
                .collect()
        })
        .unwrap_or_default();
    if bearing.is_empty() {
        return AuthorshipConfidence::Device;
    }
    let authenticated = |actor: &str| actor == signer_key_id || verified_attester == Some(actor);
    if bearing.iter().all(|a| authenticated(a)) {
        AuthorshipConfidence::Attested
    } else {
        AuthorshipConfidence::Unverified
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p cairn-event contributor 2>&1 | tail`
Expected: PASS (the four new grade/transform tests plus the existing vocabulary tests).

- [ ] **Step 5: fmt + clippy + commit**

```bash
cargo fmt --all
cargo clippy -p cairn-event --all-targets -- -D warnings
git add crates/cairn-event/src/contributor.rs
git commit -m "feat(#204): cairn-event — with_human_author + classify_authorship_confidence (ADR-0053)"
```

---

### Task 3: db/005 floor — `cairn_authorship_bound` + wire into `submit_event`

**Files:**
- Modify: `db/005_submit.sql` (add the predicate AFTER `cairn_check_contributors` ends, ~line 350; add a check in `submit_event` after step 4, ~line 558)
- Create: `crates/cairn-node/tests/authorship_binding.rs` (DB-gated predicate test)

> **DB-ordering (eager-bind trap — do not ignore).** `cairn_authorship_bound` is `LANGUAGE sql` and reads the `contributor_role` **table**. A `LANGUAGE sql` function validates its body at `CREATE` time, so the table must already exist when the function is created. The `contributor_role` table is defined at ~line 245–267 and `cairn_check_contributors` ends ~line 350 — so the new function MUST be inserted **after** `cairn_check_contributors` (NOT after `cairn_responsibility_bound` at ~line 230, which is before the table). Placing it too early makes a fresh-DB schema load fail with `relation "contributor_role" does not exist`. (This exact class of bug — a `LANGUAGE sql` function referencing a not-yet-created object — cost the ADR-0052 slice a fresh-load failure; heed it.)

**Interfaces:**
- Produces (SQL): `cairn_authorship_bound(b jsonb, p_signer text, p_attester_key bytea) RETURNS boolean` — TRUE iff every responsibility-bearing contributor's `actor_id` is the signer or the verified attester.
- Consumes: `contributor_role(role, bears)` (existing), the `submit_event` locals `b`, `v_att_key`.

- [ ] **Step 1: Write the failing predicate test**

Create `crates/cairn-node/tests/authorship_binding.rs`:

```rust
//! ADR-0053 authorship binding (db/005 `cairn_authorship_bound`). DB-gated on
//! $CAIRN_TEST_PG. The predicate is the floor's answer to forged authorship: a
//! responsibility-bearing contributor must be the event's signer or the verified
//! attester (the #195 binding, one field over). Contributory roles are exempt.
use cairn_node::db;
use tokio_postgres::Client;

fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

async fn bound(c: &Client, contributors: serde_json::Value, signer: &str) -> bool {
    let b = serde_json::json!({"contributors": contributors});
    // p_attester_key NULL: the pure-authorship (no-token) path.
    c.query_one("SELECT cairn_authorship_bound($1::jsonb, $2, NULL)", &[&b, &signer])
        .await
        .unwrap()
        .get::<_, bool>(0)
}

#[tokio::test]
async fn authorship_binding_predicate() {
    let Some(cs) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&cs).await.unwrap();
    let c = db::connect(&cs).await.unwrap();

    // bearing author == signer -> bound.
    assert!(bound(&c, serde_json::json!([{"actor_id": "H", "role": "authored"},
                                         {"actor_id": "N", "role": "recorded"}]), "H").await);
    // bearing author != signer, no token -> NOT bound (forged authorship).
    assert!(!bound(&c, serde_json::json!([{"actor_id": "H", "role": "authored"},
                                          {"actor_id": "N", "role": "recorded"}]), "N").await);
    // contributory-only (recorded) -> bound (device path exempt).
    assert!(bound(&c, serde_json::json!([{"actor_id": "N", "role": "recorded"}]), "N").await);
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p cairn-node --test authorship_binding 2>&1 | tail`
Expected: FAIL — `function cairn_authorship_bound(jsonb, text, unknown) does not exist` (or a skip if `CAIRN_TEST_PG` is unset — set it first to see the real failure).

- [ ] **Step 3: Add the predicate to db/005**

Insert the following AFTER the `cairn_check_contributors` function ends (~line 350) — it must come after the `contributor_role` table (see the DB-ordering note above). In `db/005_submit.sql`, insert:

```sql
-- Authorship binding (ADR-0053, issue #204). The authorship analog of
-- cairn_responsibility_bound (#195): a responsibility-BEARING contributor may only
-- name an actor who AUTHENTICATED to the event — the signer, or the verified
-- attester. So an `authored`/`ordered`/`attested` claim about a human is unforgeable:
-- that human either signed the bytes or attested them. Contributory roles
-- (`recorded`/`drafted`/...) are EXEMPT — a device/auxiliary contributor need not
-- sign or attest (the node stays `recorded` while the human signs). Bearing-ness
-- classifies from the ratified table, else the mandatory `bearing:` prefix (the same
-- idiom as cairn_check_contributors). STABLE (reads contributor_role) with a pinned
-- search_path (the contributor_role lookup must never resolve into a shadowed schema).
--
-- STRICT DOOR ONLY. The apply door (db/020) must NOT call this: an unverifiable
-- authorship claim there is a forgery OR an author authenticated by a scheme this
-- older node cannot parse (ADR-0012 guarantees such events arrive), and the two are
-- indistinguishable — so apply admits and GRADES (classify_authorship_confidence),
-- never refuses. Do not "simplify" this into a both-doors symmetry.
CREATE OR REPLACE FUNCTION cairn_authorship_bound(b jsonb, p_signer text, p_attester_key bytea)
RETURNS boolean LANGUAGE sql STABLE
SET search_path = public
AS $$
    SELECT NOT EXISTS (
        SELECT 1 FROM jsonb_array_elements(b -> 'contributors') AS e
        WHERE coalesce((SELECT r.bears FROM contributor_role r WHERE r.role = e ->> 'role'),
                       (e ->> 'role') LIKE 'bearing:%')
          AND (e ->> 'actor_id') IS DISTINCT FROM p_signer
          AND (p_attester_key IS NULL
               OR (e ->> 'actor_id') IS DISTINCT FROM encode(p_attester_key, 'hex')));
$$;
```

- [ ] **Step 4: Wire the check into `submit_event`**

In `db/005_submit.sql`, immediately after step 4 (the attestation gate — after the `END IF;` that closes `IF v_mode = 'suppressing' OR v_bears THEN`, ~line 558, where `v_att_key` is now the verified attester or NULL) and before step 5's target gate, insert:

```sql
    -- 4b. Authorship binding (ADR-0053, issue #204): every responsibility-bearing
    --     contributor must be AUTHENTICATED — its actor_id is the event's signer or
    --     the verified attester (v_att_key, set by step 4, else NULL). Extends the
    --     #195 responsibility<->attester binding to AUTHORSHIP so an authored/ordered
    --     claim about a human is unforgeable. Contributory roles are exempt. STRICT
    --     door only; the apply door admits + grades (see cairn_authorship_bound).
    IF NOT cairn_authorship_bound(b, b ->> 'signer_key_id', v_att_key) THEN
        RAISE EXCEPTION 'submit_event: a responsibility-bearing contributor names an actor that is neither the event signer nor the verified attester — forged authorship refused (ADR-0053; the author must sign or attest)';
    END IF;
```

- [ ] **Step 5: Run the predicate test to verify it passes**

Run: `cargo test -p cairn-node --test authorship_binding 2>&1 | tail`
Expected: PASS (the predicate resolves; all three cases hold). The schema reloads on `db::connect`, so the new function is present.

- [ ] **Step 6: Commit**

```bash
cargo fmt --all
git add db/005_submit.sql crates/cairn-node/tests/authorship_binding.rs
git commit -m "feat(#204): db/005 floor — cairn_authorship_bound + strict-door check (ADR-0053)"
```

---

### Task 4: node — thread `author` through `seal_sign_submit` + `assert_medication`

**Files:**
- Modify: `crates/cairn-node/src/medication/sealed_submit.rs` (add `AuthorParams`; add the `author` parameter to `seal_sign_submit`; apply `with_human_author`, sign with the author, keep node custody)
- Modify: `crates/cairn-node/src/medication/mod.rs` (re-export `AuthorParams`)
- Modify: `crates/cairn-node/src/medication/assert.rs` (add `author` param to `assert_medication`, thread it)
- Modify: `crates/cairn-node/src/medication/cessation.rs`, `dose.rs`, `reconciliation.rs` (insert `None` at the new `seal_sign_submit` arg position — orchestrator signatures unchanged in THIS task)
- Modify: every existing caller of `assert_medication` (insert `None` for the new `author` param): `crates/cairn-node/src/main.rs` + tests under `crates/cairn-node/tests/`
- Test: `crates/cairn-node/tests/medication_authorship.rs` (new; the human-author GREEN path)

**Interfaces:**
- Consumes: `cairn_event::contributor::with_human_author` (Task 2), `AttestParams` (existing).
- Produces:
  - `pub struct AuthorParams<'a> { pub human_sk: &'a SigningKey, pub human_kid: &'a str }` (in `sealed_submit.rs`, re-exported from `medication`)
  - `seal_sign_submit(client, node_sk, body, author: Option<&AuthorParams<'_>>, attest: Option<&AttestParams<'_>>) -> Uuid`
  - `assert_medication(client, node_sk, node_kid, node_origin, patient, input, author: Option<&AuthorParams<'_>>, attest: Option<&AttestParams<'_>>) -> Uuid`

- [ ] **Step 1: Write the failing GREEN test**

Create `crates/cairn-node/tests/medication_authorship.rs`:

```rust
//! ADR-0053 author-time human authorship on the medication stream. DB-gated on
//! $CAIRN_TEST_PG, serialized cluster-wide via db::test_serial_guard. When a human
//! author is supplied, the content event is signed by the human and carries
//! {human,"authored"} + {node,"recorded"}, while the node keeps custody (event_dek).
use cairn_event::generate_key;
use cairn_node::db;
use cairn_node::medication::{assert_medication, AssertMedicationInput, AuthorParams};
use tokio_postgres::Client;
use uuid::Uuid;

fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

async fn setup(c: &Client) -> (cairn_event::SigningKey, String, cairn_event::SigningKey, String) {
    c.batch_execute(
        "TRUNCATE event_log, actor_event, patient_chart, \
         node_unwrap_key, event_dek, event_clear, erasure_shred_log CASCADE",
    )
    .await
    .unwrap();
    c.batch_execute(
        "DO $$ BEGIN \
           IF to_regclass('public.medication_statement') IS NOT NULL THEN TRUNCATE medication_statement; END IF; \
         END $$;",
    )
    .await
    .unwrap();
    let (node_sk, node_kid) = generate_key().unwrap();
    c.execute("SELECT enroll_actor('device', '{\"role\":\"registration-desk\"}', $1)", &[&node_kid])
        .await
        .unwrap();
    let (human_sk, human_kid) = generate_key().unwrap();
    c.execute("SELECT enroll_actor('human', '{\"role\":\"clinician\"}', $1)", &[&human_kid])
        .await
        .unwrap();
    (node_sk, node_kid, human_sk, human_kid)
}

#[tokio::test]
async fn human_authored_medication_is_signed_by_the_human_node_keeps_custody() {
    let Some(cs) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&cs).await.unwrap();
    let mut c = db::connect(&cs).await.unwrap();
    let (node_sk, node_kid, human_sk, human_kid) = setup(&c).await;
    let patient = Uuid::now_v7();

    let input = AssertMedicationInput {
        term: "atorvastatin",
        inn_code: None, formulation: None, dose_amount: Some("40"), dose_unit: Some("mg"),
        sig: None, info_source: "patient-reported", started: None, started_precision: None,
    };
    let author = AuthorParams { human_sk: &human_sk, human_kid: &human_kid };
    let med_id = assert_medication(
        &mut c, &node_sk, &node_kid, &node_kid, patient, &input, Some(&author), None,
    )
    .await
    .unwrap();

    // The content event is signed by the human and names both contributors.
    let row = c
        .query_one(
            "SELECT signer_key_id, contributors, sealed FROM event_log \
             WHERE event_type = 'clinical.medication.asserted' \
               AND (cairn_clear_payload(event_log) ->> 'medication_id')::uuid = $1::text::uuid",
            &[&med_id.to_string()],
        )
        .await
        .unwrap();
    assert_eq!(row.get::<_, String>("signer_key_id"), human_kid);
    let contributors: serde_json::Value = row.get("contributors");
    assert_eq!(contributors[0]["actor_id"], human_kid);
    assert_eq!(contributors[0]["role"], "authored");
    assert_eq!(contributors[1]["actor_id"], node_kid);
    assert_eq!(contributors[1]["role"], "recorded");
    assert!(row.get::<_, bool>("sealed"));

    // The NODE (not the human) holds custody: an event_dek row exists for this event.
    let event_id: String = c
        .query_one(
            "SELECT event_id::text FROM event_log \
             WHERE (cairn_clear_payload(event_log) ->> 'medication_id')::uuid = $1::text::uuid \
               AND event_type = 'clinical.medication.asserted'",
            &[&med_id.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    let custody: i64 = c
        .query_one("SELECT count(*) FROM event_dek WHERE event_id = $1::text::uuid", &[&event_id])
        .await
        .unwrap()
        .get(0);
    assert_eq!(custody, 1, "the node must hold the DEK even though the human signed");
}
```

- [ ] **Step 2: Run the test to verify it fails to compile**

Run: `cargo test -p cairn-node --test medication_authorship 2>&1 | tail`
Expected: FAIL — `no variant or associated item named AuthorParams` / `assert_medication takes 7 arguments but 8 were supplied`.

- [ ] **Step 3: Add `AuthorParams` and the `author` parameter to `seal_sign_submit`**

In `crates/cairn-node/src/medication/sealed_submit.rs`, add the struct near the top (after the imports) and change `seal_sign_submit`:

```rust
/// The human who AUTHORS a clinical event (#204 / ADR-0053) — the FIRST half of
/// principle 10, the mirror of `AttestParams` (attestation.rs, the second half). The
/// human's key SIGNS the sealed content event (`session ≠ author`); the node still
/// holds custody. Threaded explicitly so the verb paths stay pure functions of their
/// arguments. `None` ⇒ device-additive (the node signs, `recorded`-only), unchanged.
pub struct AuthorParams<'a> {
    pub human_sk: &'a SigningKey,
    pub human_kid: &'a str,
}
```

Change the `seal_sign_submit` signature and its opening so the author (when present) rewrites the body and becomes the signer while the node keeps custody. Replace the current parameter list and the `seal_and_sign` / `ensure_unwrap_key` lines:

```rust
pub async fn seal_sign_submit(
    client: &mut tokio_postgres::Client,
    node_sk: &SigningKey,
    body: EventBody,
    author: Option<&AuthorParams<'_>>,
    attest: Option<&super::AttestParams<'_>>,
) -> anyhow::Result<uuid::Uuid> {
    // ADR-0053: when a human authors, rewrite the device-shaped body so the human is
    // an `authored` contributor AND the signer; the node stays `recorded` + custodian.
    let body = match author {
        Some(a) => cairn_event::contributor::with_human_author(body, a.human_kid),
        None => body,
    };
    // The content event is signed by the author when present, else the node (device).
    let signing_sk: &SigningKey = author.map(|a| a.human_sk).unwrap_or(node_sk);

    let event_id: uuid::Uuid = body.event_id.parse().with_context(|| {
        format!("seal_sign_submit: event_id {:?} is not a uuid", body.event_id)
    })?;
    let patient: uuid::Uuid = body.patient_id.parse().with_context(|| {
        format!("seal_sign_submit: patient_id {:?} is not a uuid", body.patient_id)
    })?;
    let node_origin = body.hlc.node_origin.clone();
    let thread = match attest {
        Some(_) => Some(thread_id_of(&body)?),
        None => None,
    };

    let (signed_bytes, dek) = seal_and_sign(body, signing_sk)?;
    // Custody is the NODE's regardless of who signed (born-sealed erasability, ADR-0052).
    ensure_unwrap_key(client, node_sk).await?;
    // ... the existing `match attest { ... }` block is UNCHANGED below this line ...
```

Leave the `match attest { None => ..., Some(params) => ... }` block exactly as it is (it already uses `signed_bytes`, `dek`, `node_origin`, `patient`, `thread`).

- [ ] **Step 4: Re-export `AuthorParams` and thread it through `assert_medication`**

In `crates/cairn-node/src/medication/mod.rs`, extend the `sealed_submit` re-export line (add a `pub use` for `AuthorParams`):

```rust
pub use sealed_submit::AuthorParams;
```

In `crates/cairn-node/src/medication/assert.rs`, add the `author` param to `assert_medication` (before `attest`) and pass it to `seal_sign_submit`:

```rust
pub async fn assert_medication(
    client: &mut tokio_postgres::Client,
    node_sk: &SigningKey,
    node_kid: &str,
    node_origin: &str,
    patient: Uuid,
    input: &AssertMedicationInput<'_>,
    author: Option<&crate::medication::AuthorParams<'_>>,
    attest: Option<&crate::medication::AttestParams<'_>>,
) -> anyhow::Result<Uuid> {
    // ... body unchanged up to the submit call ...
    crate::medication::sealed_submit::seal_sign_submit(client, node_sk, body, author, attest).await?;
    Ok(medication_id)
}
```

- [ ] **Step 5: Fix every other `seal_sign_submit` call site (insert `None`) and every `assert_medication` call site (insert `None`)**

The `seal_sign_submit` signature changed, so its three other call sites must insert `None` at the new `author` position (their orchestrator signatures are unchanged in this task):
- `crates/cairn-node/src/medication/cessation.rs` (in `cease_medication`): `seal_sign_submit(client, node_sk, body, None, attest)`
- `crates/cairn-node/src/medication/dose.rs` (in `change_dose` and `correct_dose`): `seal_sign_submit(client, node_sk, body, None, attest)`
- `crates/cairn-node/src/medication/reconciliation.rs` (`submit_reconcile_like`, the `None =>` arm): `seal_sign_submit(client, node_sk, body, None, None).await?;`

The `assert_medication` signature changed, so every caller must insert `None` for the new `author` param. Find them:

Run: `rg -n 'assert_medication\(' crates/cairn-node/src crates/cairn-node/tests`

For each call site OTHER than the new `medication_authorship.rs` test, insert `None` immediately before the final `attest`/`params` argument. Known sites: `crates/cairn-node/src/main.rs` (the `MedicationAssert` handler), and the test files `medication.rs`, `medication_attestation.rs`, `medication_patient_consistency.rs`, `medication_remote_apply.rs` (any that call `assert_medication`). The device path is unchanged — `None` means "no human author".

- [ ] **Step 6: Run the new test + the existing medication suites to verify green**

Run: `cargo test -p cairn-node --test medication_authorship --test medication --test medication_attestation 2>&1 | tail -20`
Expected: PASS — the new author test passes; the existing medication/attestation suites still pass (device path unchanged).

- [ ] **Step 7: fmt + clippy + commit**

```bash
cargo fmt --all
cargo clippy -p cairn-node --all-targets -- -D warnings
git add crates/cairn-node/src/medication/ crates/cairn-node/src/main.rs crates/cairn-node/tests/
git commit -m "feat(#204): node — human author signs the sealed event via seal_sign_submit (assert; ADR-0053)"
```

---

### Task 5: roll the `author` param across the remaining orchestrators

**Files:**
- Modify: `crates/cairn-node/src/medication/cessation.rs` (`cease_medication`)
- Modify: `crates/cairn-node/src/medication/dose.rs` (`change_dose`, `correct_dose`)
- Modify: `crates/cairn-node/src/medication/reconciliation.rs` (`reconcile_medications`, `separate_medications`, `submit_reconcile_like`)
- Modify: every existing caller of these five orchestrators (insert `None`): `crates/cairn-node/src/main.rs` + tests
- Test: extend `crates/cairn-node/tests/medication_authorship.rs`

**Interfaces:**
- Produces: `cease_medication`, `change_dose`, `correct_dose`, `reconcile_medications`, `separate_medications` each gain `author: Option<&AuthorParams<'_>>` immediately before their `attest` param; `submit_reconcile_like` gains `author` and applies it in BOTH arms.

- [ ] **Step 1: Write the failing test**

Append to `crates/cairn-node/tests/medication_authorship.rs`:

```rust
#[tokio::test]
async fn human_authored_cessation_is_signed_by_the_human() {
    let Some(cs) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&cs).await.unwrap();
    let mut c = db::connect(&cs).await.unwrap();
    let (node_sk, node_kid, human_sk, human_kid) = setup(&c).await;
    let patient = Uuid::now_v7();
    let author = AuthorParams { human_sk: &human_sk, human_kid: &human_kid };

    let input = AssertMedicationInput {
        term: "warfarin", inn_code: None, formulation: None, dose_amount: None, dose_unit: None,
        sig: None, info_source: "patient-reported", started: None, started_precision: None,
    };
    let med_id = assert_medication(&mut c, &node_sk, &node_kid, &node_kid, patient, &input, Some(&author), None)
        .await
        .unwrap();

    let cease_input = cairn_node::medication::CeaseMedicationInput {
        stopped: None, stopped_precision: None, reason: Some("bleeding risk"),
    };
    cairn_node::medication::cease_medication(
        &mut c, &node_sk, &node_kid, &node_kid, patient, med_id, &cease_input, Some(&author), None,
    )
    .await
    .unwrap();

    let signer: String = c
        .query_one(
            "SELECT signer_key_id FROM event_log WHERE event_type = 'clinical.medication-cessation.asserted'",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(signer, human_kid);
}
```

- [ ] **Step 2: Run the test to verify it fails to compile**

Run: `cargo test -p cairn-node --test medication_authorship 2>&1 | tail`
Expected: FAIL — `cease_medication takes 8 arguments but 9 were supplied`.

- [ ] **Step 3: Add `author` to the five orchestrators**

In each orchestrator, add `author: Option<&crate::medication::AuthorParams<'_>>` immediately before the `attest` parameter, and pass `author` down. For `cease_medication` / `change_dose` / `correct_dose`, change their `seal_sign_submit(...)` call from `None` (Task 4) to `author`:

```rust
crate::medication::sealed_submit::seal_sign_submit(client, node_sk, body, author, attest).await?;
```

For `reconcile_medications` / `separate_medications`, add `author` and pass it to `submit_reconcile_like(client, node_sk, node_origin, body, patient, subject_a, subject_b, author, attest)`.

In `submit_reconcile_like`, add the `author` param and apply it in BOTH arms:
```rust
async fn submit_reconcile_like(
    client: &mut tokio_postgres::Client,
    node_sk: &SigningKey,
    node_origin: &str,
    body: EventBody,
    patient: Uuid,
    subject_a: Uuid,
    subject_b: Uuid,
    author: Option<&crate::medication::AuthorParams<'_>>,
    attest: Option<&crate::medication::AttestParams<'_>>,
) -> anyhow::Result<()> {
    match attest {
        None => {
            crate::medication::sealed_submit::seal_sign_submit(client, node_sk, body, author, None).await?;
        }
        Some(params) => {
            let hlc_a = crate::db::next_hlc(client, node_origin).await?;
            let hlc_b = crate::db::next_hlc(client, node_origin).await?;
            // ADR-0053: the human authors the content event too — rewrite + sign with the
            // author key when present; the node still holds custody + signs the attestations.
            let body = match author {
                Some(a) => cairn_event::contributor::with_human_author(body, a.human_kid),
                None => body,
            };
            let signing_sk = author.map(|a| a.human_sk).unwrap_or(node_sk);
            let (signed_bytes, dek) =
                crate::medication::sealed_submit::seal_and_sign(body, signing_sk)?;
            crate::medication::sealed_submit::ensure_unwrap_key(client, node_sk).await?;
            let tx = client.transaction().await?;
            tx.execute("SELECT submit_event($1, NULL, NULL, $2)", &[&signed_bytes, &dek.as_slice()])
                .await?;
            crate::medication::attest_thread_in_tx(&tx, params, patient, subject_a, hlc_a).await?;
            crate::medication::attest_thread_in_tx(&tx, params, patient, subject_b, hlc_b).await?;
            tx.commit().await?;
        }
    }
    Ok(())
}
```

- [ ] **Step 4: Fix every caller of the five orchestrators (insert `None`)**

Run: `rg -n 'cease_medication\(|change_dose\(|correct_dose\(|reconcile_medications\(|separate_medications\(' crates/cairn-node/src crates/cairn-node/tests`

For each call site OTHER than the new authorship test, insert `None` immediately before the final `attest`/`params` argument (device path unchanged). Known sites: `crates/cairn-node/src/main.rs` (the verb handlers) and `crates/cairn-node/tests/medication*.rs`.

- [ ] **Step 5: Run the medication suites to verify green**

Run: `cargo test -p cairn-node --test medication_authorship --test medication_dose --test medication_reconciliation --test medication_attestation 2>&1 | tail -20`
Expected: PASS.

- [ ] **Step 6: fmt + clippy + commit**

```bash
cargo fmt --all
cargo clippy -p cairn-node --all-targets -- -D warnings
git add crates/cairn-node/src/medication/ crates/cairn-node/src/main.rs crates/cairn-node/tests/
git commit -m "feat(#204): node — roll the author param across cease/dose/reconcile orchestrators (ADR-0053)"
```

---

### Task 6: CLI — `--author-as` / `--author-passphrase` on the six verbs

**Files:**
- Modify: `crates/cairn-node/src/main.rs` (add `AuthorFlags`, `resolve_author`, `author_params`; add `#[command(flatten)] author: AuthorFlags` to the six verb subcommands; wire into each handler)

**Interfaces:**
- Consumes: `load_attester_key`, `cairn_node::identify::attester_is_enrolled_human` (both existing), `AuthorParams` (Task 4).
- Produces: `resolve_author(&Client, &AuthorFlags) -> Result<Option<(SigningKey, String)>>`; `author_params(&Option<(SigningKey, String)>) -> Option<AuthorParams>`.

- [ ] **Step 1: Add the `AuthorFlags` struct, `resolve_author`, and `author_params`**

In `crates/cairn-node/src/main.rs`, next to `AttestFlags` / `resolve_attester` / `attest_params`, add:

```rust
/// The `--author-as` flag set (#204 / ADR-0053): the human who AUTHORS the clinical
/// event. Present ⇒ the human's key signs the sealed content event and rides as an
/// `authored` contributor (session ≠ author); absent ⇒ device-additive (the node
/// signs, `recorded`-only), unchanged. Distinct from `--attest-as` (which layers the
/// separate ADR-0049 responsibility overlay); the two compose.
#[derive(clap::Args, Clone)]
struct AuthorFlags {
    /// Author this clinical event as a specific enrolled human: their key signs the
    /// event. Absent ⇒ device-additive (the node signs, no human author).
    #[arg(long)]
    author_as: Option<std::path::PathBuf>,
    /// Passphrase to unseal --author-as (else CAIRN_AUTHOR_PASSPHRASE, else prompt).
    #[arg(long, env = "CAIRN_AUTHOR_PASSPHRASE")]
    author_passphrase: Option<String>,
}

/// Resolve `--author-as` into a loaded human key + verified kid, or `None` when the
/// flag is absent. Runs the same enrolled-human pre-check as `resolve_attester` (the
/// db/005 authorship binding is the real enforcement — this only gives a clean error
/// before any event is authored).
async fn resolve_author(
    client: &tokio_postgres::Client,
    flags: &AuthorFlags,
) -> anyhow::Result<Option<(cairn_event::SigningKey, String)>> {
    match &flags.author_as {
        None => Ok(None),
        Some(path) => {
            let sk = load_attester_key(path, flags.author_passphrase.clone())?;
            let kid = hex::encode(sk.verifying_key().to_bytes());
            if !cairn_node::identify::attester_is_enrolled_human(client, &kid).await? {
                anyhow::bail!("--author-as key is not an enrolled human actor; run `enroll-human` first");
            }
            Ok(Some((sk, kid)))
        }
    }
}

/// Borrow a resolved author into `AuthorParams`, or `None` (device-additive).
fn author_params<'a>(
    resolved: &'a Option<(cairn_event::SigningKey, String)>,
) -> Option<cairn_node::medication::AuthorParams<'a>> {
    resolved
        .as_ref()
        .map(|(sk, kid)| cairn_node::medication::AuthorParams { human_sk: sk, human_kid: kid })
}
```

- [ ] **Step 2: Add the `author` flag group to the six verb subcommands**

In the `Cmd` enum in `crates/cairn-node/src/main.rs`, add `#[command(flatten)] author: AuthorFlags` to each of `MedicationAssert`, `MedicationCease`, `MedicationChangeDose`, `MedicationCorrectDose`, `MedicationReconcile`, `MedicationSeparate` (beside the existing `attest: AttestFlags`).

- [ ] **Step 3: Wire each handler to resolve and pass the author**

In each of the six verb handlers, after the existing `let resolved = resolve_attester(&db, &attest).await?; let params = attest_params(&resolved, &attest);`, add:

```rust
let resolved_author = resolve_author(&db, &author).await?;
let a_params = author_params(&resolved_author);
```

and change the orchestrator call to pass `a_params.as_ref()` immediately before `params.as_ref()`. Example for `MedicationAssert`:

```rust
let med_id = cairn_node::medication::assert_medication(
    &mut db, &node_sk, &node_kid, &id.node_id_hex, patient, &input,
    a_params.as_ref(), params.as_ref(),
)
.await?;
```

(Replace the `None` inserted in Task 4/5's call-site fix with `a_params.as_ref()` for these six handlers.)

- [ ] **Step 4: Build the CLI to verify it compiles**

Run: `cargo build -p cairn-node 2>&1 | tail`
Expected: PASS (compiles; the six verbs now accept `--author-as`).

- [ ] **Step 5: Smoke-test the flag is wired (help text)**

Run: `cargo run -p cairn-node -- medication-assert --help 2>&1 | grep -A1 author-as`
Expected: shows `--author-as <AUTHOR_AS>` in the help.

- [ ] **Step 6: fmt + clippy + commit**

```bash
cargo fmt --all
cargo clippy -p cairn-node --all-targets -- -D warnings
git add crates/cairn-node/src/main.rs
git commit -m "feat(#204): CLI — --author-as on the six medication verbs (ADR-0053)"
```

---

### Task 7: end-to-end floor + consequence verification tests

**Files:**
- Test: extend `crates/cairn-node/tests/medication_authorship.rs` (forged-authorship refused through the real door; device path unchanged; the suppression owner-gate now recognises the human author)

**Interfaces:**
- Consumes: everything above. No new production code (this task is verification; if a test fails, fix the relevant earlier task's code).

> **Why no apply-door integration test here.** The design's "apply admits + grades" property (§4/§6) is covered by two things that need no brittle setup: (a) the **Global Constraint that db/020 is not modified** — nothing in this slice can make the sync door start refusing; and (b) Task 2's `authorship_grade_unverified_…` unit test, which IS the grade the apply-side consumer computes. A live `apply_remote_event` test on a sealed forged event would couple to born-sealed's without-custody projection mechanics (orthogonal to #204), so we deliberately do not add one.

- [ ] **Step 1: Add the forgery helper + `db_msg`, and the strict-refusal test**

Append to `crates/cairn-node/tests/medication_authorship.rs`. The helper crafts a sealed clinical assert that NAMES a human author but is SIGNED BY THE NODE with no token — a forgery. The db/005 authorship binding (Task 3) refuses it at step 4b, *before* the born-sealed unseal/custody arms, so no unwrap-key setup is needed:

```rust
/// The Postgres RAISE text behind a failed statement.
fn db_msg(e: &tokio_postgres::Error) -> String {
    e.as_db_error().map(|d| d.message().to_string()).unwrap_or_else(|| e.to_string())
}

/// Craft a sealed `clinical.medication.asserted` that CLAIMS `human_kid` authored it
/// but is SIGNED BY THE NODE, no attestation token — a forgery. Returns the signed wire
/// bytes + the DEK (the strict door's 4th arg; unused before the step-4b refusal).
fn craft_forged_authorship_event(
    node_sk: &cairn_event::SigningKey,
    node_kid: &str,
    human_kid: &str,
    patient: Uuid,
) -> (Vec<u8>, zeroize::Zeroizing<[u8; 32]>) {
    use cairn_event::seal::{seal_event_payload, seal_stub_twin};
    use cairn_event::{sign, EventBody, Hlc};
    let event_id = Uuid::now_v7();
    let medication_id = Uuid::now_v7();
    let payload = serde_json::json!({
        "medication_id": medication_id.to_string(),
        "term": "atorvastatin", "info_source": "patient-reported"
    });
    let (container, dek) =
        seal_event_payload(&payload, "Atorvastatin (patient-reported)", &event_id.to_string()).unwrap();
    let body = EventBody {
        event_id: event_id.to_string(),
        patient_id: patient.to_string(),
        event_type: "clinical.medication.asserted".into(),
        schema_version: "clinical.medication/1".into(),
        hlc: Hlc { wall: 1_700_000_000_000, counter: 0, node_origin: node_kid.to_string() },
        t_effective: None,
        signer_key_id: node_kid.to_string(), // the NODE signs...
        contributors: serde_json::json!([
            {"actor_id": human_kid, "role": "authored"}, // ...but claims the human authored
            {"actor_id": node_kid,  "role": "recorded"}]),
        payload: container,
        attachments: vec![],
        plaintext_twin: Some(seal_stub_twin("clinical.medication.asserted")),
    };
    let signed = sign(&body, node_sk).unwrap();
    (signed.signed_bytes, dek)
}

#[tokio::test]
async fn forged_authorship_refused_at_the_strict_door() {
    let Some(cs) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&cs).await.unwrap();
    let c = db::connect(&cs).await.unwrap();
    let (node_sk, node_kid, _human_sk, human_kid) = setup(&c).await;
    let patient = Uuid::now_v7();

    let (signed, dek) = craft_forged_authorship_event(&node_sk, &node_kid, &human_kid, patient);
    let err = c
        .execute("SELECT submit_event($1, NULL, NULL, $2)", &[&signed, &dek.as_slice()])
        .await
        .expect_err("forged authorship must be refused");
    assert!(
        db_msg(&err).contains("forged authorship refused"),
        "expected the ADR-0053 authorship-binding refusal, got: {}",
        db_msg(&err)
    );
}
```

- [ ] **Step 2: Run it to verify it PASSES (the floor refuses through the real door)**

Run: `cargo test -p cairn-node --test medication_authorship forged_authorship_refused 2>&1 | tail`
Expected: PASS — the Task 3 binding refuses the forgery through the real `submit_event` door. (If it does NOT refuse, the Task 3 step-4 wiring is wrong — fix it.)

- [ ] **Step 3: Write the device-path-unchanged regression test**

Append:

```rust
#[tokio::test]
async fn device_additive_assert_still_valid_with_no_author() {
    let Some(cs) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&cs).await.unwrap();
    let mut c = db::connect(&cs).await.unwrap();
    let (node_sk, node_kid, _hs, _hk) = setup(&c).await;
    let patient = Uuid::now_v7();
    let input = AssertMedicationInput {
        term: "metformin", inn_code: None, formulation: None, dose_amount: None, dose_unit: None,
        sig: None, info_source: "patient-reported", started: None, started_precision: None,
    };
    // No author, no attest -> device-additive: node signs, recorded-only. Must succeed.
    let med_id = assert_medication(&mut c, &node_sk, &node_kid, &node_kid, patient, &input, None, None)
        .await
        .unwrap();
    let signer: String = c
        .query_one(
            "SELECT signer_key_id FROM event_log WHERE event_type = 'clinical.medication.asserted' \
               AND (cairn_clear_payload(event_log) ->> 'medication_id')::uuid = $1::text::uuid",
            &[&med_id.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(signer, node_kid, "device-additive assert is still signed by the node");
}
```

- [ ] **Step 4: Write the suppression-owner-gate consequence test**

Because a human-authored event now has `signer_key_id = human`, the floor's `cairn_suppression_author_ok` (db/005) resolves that human as an owner — so only they (not another human) may later suppress it. Append:

```rust
#[tokio::test]
async fn human_author_owns_suppression_rights() {
    let Some(cs) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&cs).await.unwrap();
    let mut c = db::connect(&cs).await.unwrap();
    let (node_sk, node_kid, human_sk, human_kid) = setup(&c).await;
    let patient = Uuid::now_v7();
    let author = AuthorParams { human_sk: &human_sk, human_kid: &human_kid };
    let input = AssertMedicationInput {
        term: "lisinopril", inn_code: None, formulation: None, dose_amount: None, dose_unit: None,
        sig: None, info_source: "patient-reported", started: None, started_precision: None,
    };
    let med_id = assert_medication(&mut c, &node_sk, &node_kid, &node_kid, patient, &input, Some(&author), None)
        .await
        .unwrap();
    let event_id: String = c
        .query_one(
            "SELECT event_id::text FROM event_log WHERE event_type = 'clinical.medication.asserted' \
               AND (cairn_clear_payload(event_log) ->> 'medication_id')::uuid = $1::text::uuid",
            &[&med_id.to_string()],
        )
        .await
        .unwrap()
        .get(0);

    // The human author IS an owner (may suppress their own event).
    let author_vk = human_sk.verifying_key().to_bytes().to_vec();
    let owns: bool = c
        .query_one("SELECT cairn_suppression_author_ok($1::text::uuid, $2)", &[&event_id, &author_vk])
        .await
        .unwrap()
        .get(0);
    assert!(owns, "the human author must own suppression rights over their own event");

    // A different human does NOT (cross-human suppression is refused — ADR-0043).
    let (other_sk, _other_kid) = cairn_event::generate_key().unwrap();
    let other_vk = other_sk.verifying_key().to_bytes().to_vec();
    let stranger_owns: bool = c
        .query_one("SELECT cairn_suppression_author_ok($1::text::uuid, $2)", &[&event_id, &other_vk])
        .await
        .unwrap()
        .get(0);
    assert!(!stranger_owns, "a stranger must NOT own suppression rights over the human's event");
}
```

- [ ] **Step 5: Run the full authorship suite**

Run: `cargo test -p cairn-node --test medication_authorship 2>&1 | tail`
Expected: PASS (grade + floor + device + suppression-owner-gate tests).

- [ ] **Step 6: Commit**

```bash
git add crates/cairn-node/tests/medication_authorship.rs
git commit -m "test(#204): forged-authorship refusal + device regression + suppression owner-gate (ADR-0053)"
```

---

### Task 8: whole-suite verification, HANDOVER + ROADMAP, PR

**Files:**
- Modify: `docs/HANDOVER.md`, `docs/ROADMAP.md`

**Interfaces:** none (release task).

- [ ] **Step 1: Run the whole workspace test suite**

Run: `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test --workspace 2>&1 | tail -30`
Expected: all pass (cairn-event, cairn-node, cairn-sync). Record the counts.

- [ ] **Step 2: fmt (both trees) + clippy + deny + docs build**

Run:
```bash
cargo fmt --all --check
cargo fmt --manifest-path extensions/cairn_pgx/Cargo.toml --check
cargo clippy --workspace --all-targets -- -D warnings
cargo deny check 2>&1 | tail
uv run --with-requirements docs/requirements.txt -- mkdocs build 2>&1 | tail
```
Expected: all clean.

- [ ] **Step 3: Update ROADMAP.md (add Slice 43) and HANDOVER.md**

Add a **Slice 43** to `docs/ROADMAP.md` summarizing #204 / ADR-0053 (the authoring-human slice: `with_human_author` + `classify_authorship_confidence`, `cairn_authorship_bound` strict-door binding, `--author-as`, grade `attested`/`unverified`/`device`, deferrals). In `docs/HANDOVER.md`, move the "⇒ NEXT" pointer from #204 [C3] to **#188 [D1]** (the schema-version guard, Priority 4), record #204 as ✅ DONE with the branch/ADR/spec-version, and update the "Session date" line + spec version to **0.54**. Keep both files under 500 lines (prune older condensed session paragraphs as needed).

- [ ] **Step 4: Commit the docs**

```bash
git add docs/HANDOVER.md docs/ROADMAP.md
git commit -m "docs(#204): HANDOVER + ROADMAP Slice 43 — authoring-human slice done (ADR-0053); next #188 [D1]"
```

- [ ] **Step 5: Push and open the PR**

```bash
git push -u origin feat/adr-0053-authoring-human-204
gh pr create --base main --title "feat(#204): ADR-0053 — per-write human authorship (session≠author) on the medication stream" --body "$(cat <<'BODY'
Resolves #204 (2026-07-15 review finding C3): the medication stream ships with an
authenticated human author. A clinical event carries {human,"authored"} + {node,"recorded"},
signed by the human while the node seals + holds custody — realizing session.user ≠ event.author
(§3.10, ADR-0008).

- Strict door (db/005) enforces the authorship binding (cairn_authorship_bound): a
  responsibility-bearing contributor must be the signer or the verified attester (the #195
  binding, one field over) — forged authorship refused at authoring.
- Apply door (db/020) UNCHANGED: admits + grades. An unverifiable authorship claim is a
  forgery OR a future-credential author this node can't parse; refusing it would make a real
  medication invisible (inferior to paper), so it grades `unverified`, never refuses.
- classify_authorship_confidence: attested / unverified / device (one shared predicate).
- Deferred (UI layer): durable drafts + sign-as salvage; the `asserted` grade; author+responsibility
  on one event.

Design: docs/superpowers/specs/2026-07-18-authoring-human-attribution-204.md
Plan:   docs/superpowers/plans/2026-07-18-authoring-human-attribution-204.md

🤖 Generated with [Claude Code](https://claude.com/claude-code)
BODY
)"
```

---

## Notes for the implementer

- **The strict/apply asymmetry is deliberate.** Never add `cairn_authorship_bound` to db/020 — an unverifiable authorship claim at the sync door is indistinguishable from a future-credential author, and refusing it wedges lawful future meds out of the record (design §4). If a reviewer suggests "symmetry," point them at ADR-0053's "Why apply admits-and-grades".
- **Custody stays the node.** `ensure_unwrap_key(client, node_sk)` must ALWAYS use `node_sk`, even when the human signs — the node holds the DEK (born-sealed erasability). Task 4 step 3 pins this; the Task 4 test asserts the `event_dek` row exists.
- **The contributor drift guard is untouched** — no new role is added (`authored` is already ratified), so `crates/cairn-node/tests/contributor_roles.rs` needs no change.
