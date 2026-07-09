# Suppression Self-Only Owner-Gate — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close the last open sub-item of #99 — refuse cross-human suppression at both write doors (self-only on human-authored content; agent advisories stay dismissable).

**Architecture:** One shared STABLE SQL helper `cairn_suppression_author_ok(target, attester_key)` defined in `db/005_submit.sql`, called at step 5 of both `submit_event` (db/005) and `apply_remote_event` (db/020). Both migration files are edited **in place** (the established floor-hardening pattern; pre-clinical posture, no production data). The decision is recorded as immutable ADR-0043.

**Tech Stack:** PostgreSQL 18 + PL/pgSQL + `cairn_pgx` (in-DB floor); Rust `tokio-postgres` DB-gated integration tests using `cairn_event` to sign/attest; mkdocs for the ADR.

## Global Constraints

- **Language tier:** the gate is safety-critical → in-DB SQL/PL-pgSQL only, optimized for reviewer-legibility (§9).
- **AGPL-3.0**; no new dependencies.
- **TDD:** failing test first, then floor code. All new tests are DB-gated on `CAIRN_TEST_PG` and skip cleanly without it.
- **Both doors, one floor (principle 12):** the identical gate must hold on the remote-apply door via the *same* helper — no drift.
- **Safe direction:** the gate's only failure mode is over-refusal on human-authored content, never over-permission.
- **Test crypto is runtime-derived** (CLAUDE.md house rule 6) — but these tests mint real keys via `cairn_event::generate_key`, so no literals arise.
- **Run DB-gated tests with:** `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test"` (PG18 + `cairn_pgx`).
- **"Human author of target"** = `{target.signer_key_id | resolves to actor_current.kind='human'} ∪ {hex(target.attester_key) | attester_key IS NOT NULL}`. Empty set ⇒ agent/un-owned ⇒ suppression permitted.

---

### Task 1: Shared helper + local-door (db/005) gate

**Files:**
- Create: `crates/cairn-node/tests/suppression_owner_gate.rs`
- Modify: `db/005_submit.sql` (add helper before `submit_event`; add gate inside step 5, ~line 173–178)

**Interfaces:**
- Consumes: `cairn_event::{generate_key, sign, sign_attestation, event_address, EventBody, Hlc, SigningKey}`; `cairn_node::db::{connect_and_load_schema, test_serial_guard}`; SQL `submit_event($1,$2,$3)`, `submit_event($1)`, `enroll_actor(kind, pinned_json, kid)`.
- Produces: SQL `cairn_suppression_author_ok(p_target uuid, p_attester_key bytea) RETURNS boolean` (STABLE), called by both doors.

- [ ] **Step 1: Write the failing test file.**

Create `crates/cairn-node/tests/suppression_owner_gate.rs`. This mirrors `tests/attestation.rs`'s setup/body/sign pattern exactly.

```rust
//! ADR-0043 / issue #99 — the suppression owner-gate: a suppressing overlay
//! (salience.downgrade / visibility.suppress) that forecloses on a HUMAN author's
//! event is self-only. Cross-human suppression is refused at BOTH write doors;
//! agent-authored / un-owned advisories stay dismissable (clinician-overrides-machine,
//! principle 10). Real Postgres, gated on $CAIRN_TEST_PG, serialized cluster-wide.
use cairn_event::{event_address, generate_key, sign, sign_attestation, EventBody, Hlc, SigningKey};
use cairn_node::db;
use tokio_postgres::Client;
use uuid::Uuid;

const SUBMIT1: &str = "SELECT submit_event($1)";
const SUBMIT3: &str = "SELECT submit_event($1,$2,$3)";

fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

/// Enroll one agent signer + two distinct human actors (A the author, B the
/// would-be cross-author suppressor). Returns their (sk, kid) pairs.
async fn setup(
    c: &Client,
) -> (SigningKey, String, SigningKey, String, SigningKey, String) {
    c.batch_execute("TRUNCATE event_log, actor_event, patient_chart CASCADE")
        .await
        .unwrap();
    let (sk_ag, kid_ag) = generate_key().unwrap();
    let (sk_a, kid_a) = generate_key().unwrap();
    let (sk_b, kid_b) = generate_key().unwrap();
    c.execute(
        "SELECT enroll_actor('agent', '{\"model\":\"triage-stub\",\"version\":\"1\",\"skill_epoch\":\"epoch-a\"}', $1)",
        &[&kid_ag],
    ).await.unwrap();
    c.execute("SELECT enroll_actor('human', '{\"role\":\"clinician\"}', $1)", &[&kid_a])
        .await
        .unwrap();
    c.execute("SELECT enroll_actor('human', '{\"role\":\"clinician\"}', $1)", &[&kid_b])
        .await
        .unwrap();
    (sk_ag, kid_ag, sk_a, kid_a, sk_b, kid_b)
}

/// Minimal EventBody. `signer_kid` sets signer_key_id; `target` becomes
/// payload.target_event_id; `responsibility` adds a responsibility-bearing contributor.
fn body(
    event_type: &str,
    patient: Uuid,
    signer_kid: &str,
    responsibility: bool,
    target: Option<&str>,
) -> EventBody {
    let contrib = if responsibility {
        serde_json::json!([{"actor_id": signer_kid, "role": "attested", "responsibility": "attested"}])
    } else {
        serde_json::json!([{"actor_id": signer_kid, "role": "author"}])
    };
    let payload = match target {
        Some(t) => serde_json::json!({ "target_event_id": t }),
        None => serde_json::json!({ "text": "seen, stable" }),
    };
    EventBody {
        event_id: Uuid::now_v7().to_string(),
        patient_id: patient.to_string(),
        event_type: event_type.into(),
        schema_version: "advisory/1".into(),
        hlc: Hlc { wall: 1, counter: 0, node_origin: "n".into() },
        t_effective: None,
        signer_key_id: signer_kid.into(),
        contributors: contrib,
        payload,
        attachments: vec![],
        plaintext_twin: None,
    }
}

/// Author a plain additive note (no attestation) and return its event_id.
async fn author_note(c: &Client, patient: Uuid, signer_kid: &str, sk: &SigningKey) -> String {
    let b = body("note.added", patient, signer_kid, false, None);
    let s = sign(&b, sk).unwrap();
    c.execute(SUBMIT1, &[&s.signed_bytes]).await.unwrap();
    b.event_id
}

/// Try to submit a human-attested suppress of `target`. Returns Ok(()) on accept.
async fn try_suppress(
    c: &Client,
    patient: Uuid,
    event_type: &str,
    signer_kid: &str,
    signer_sk: &SigningKey,
    target: &str,
    attester_kid: &str,
    attester_sk: &SigningKey,
) -> Result<(), String> {
    let supp = body(event_type, patient, signer_kid, false, Some(target));
    let signed = sign(&supp, signer_sk).unwrap();
    let ca = event_address(&signed.signed_bytes);
    let token = sign_attestation(&ca, attester_kid, "attested", attester_sk).unwrap();
    let vk = attester_sk.verifying_key().to_bytes().to_vec();
    c.execute(SUBMIT3, &[&signed.signed_bytes, &token, &vk])
        .await
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[tokio::test]
async fn self_suppression_by_human_signer_accepted() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (_sk_ag, _kid_ag, sk_a, kid_a, _sk_b, _kid_b) = setup(&c).await;
    let p = Uuid::now_v7();
    // Human A signs a note, then A downgrades A's own note.
    let tgt = author_note(&c, p, &kid_a, &sk_a).await;
    let r = try_suppress(&c, p, "salience.downgrade", &kid_a, &sk_a, &tgt, &kid_a, &sk_a).await;
    assert!(r.is_ok(), "author suppressing their own event must be accepted: {r:?}");
}

#[tokio::test]
async fn cross_human_salience_downgrade_refused() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (_sk_ag, _kid_ag, sk_a, kid_a, sk_b, kid_b) = setup(&c).await;
    let p = Uuid::now_v7();
    // Human A authors; human B tries to downgrade A's note.
    let tgt = author_note(&c, p, &kid_a, &sk_a).await;
    let r = try_suppress(&c, p, "salience.downgrade", &kid_b, &sk_b, &tgt, &kid_b, &sk_b).await;
    assert!(r.is_err(), "cross-human downgrade must be refused");
    assert!(r.unwrap_err().contains("cross-author suppression refused"), "legible reason");
}

#[tokio::test]
async fn cross_human_visibility_suppress_refused() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (_sk_ag, _kid_ag, sk_a, kid_a, sk_b, kid_b) = setup(&c).await;
    let p = Uuid::now_v7();
    let tgt = author_note(&c, p, &kid_a, &sk_a).await;
    let r = try_suppress(&c, p, "visibility.suppress", &kid_b, &sk_b, &tgt, &kid_b, &sk_b).await;
    assert!(r.is_err(), "cross-human hide must be refused");
    assert!(r.unwrap_err().contains("cross-author suppression refused"), "legible reason");
}

#[tokio::test]
async fn self_suppression_by_human_attester_accepted() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_ag, kid_ag, sk_a, kid_a, _sk_b, _kid_b) = setup(&c).await;
    let p = Uuid::now_v7();
    // Target: an AGENT-signed note that human A vouches for (responsibility) — so the
    // target's ONLY human author is the attester A (attester_key = A, signer = agent).
    let b = body("note.added", p, &kid_ag, true, None);
    let signed = sign(&b, &sk_ag).unwrap();
    let ca = event_address(&signed.signed_bytes);
    let token = sign_attestation(&ca, &kid_a, "attested", &sk_a).unwrap();
    let vk_a = sk_a.verifying_key().to_bytes().to_vec();
    c.execute(SUBMIT3, &[&signed.signed_bytes, &token, &vk_a]).await.unwrap();
    // A (the human author-of-record) may suppress it.
    let r = try_suppress(&c, p, "salience.downgrade", &kid_a, &sk_a, &b.event_id, &kid_a, &sk_a).await;
    assert!(r.is_ok(), "the human attester-of-record may suppress: {r:?}");
}

#[tokio::test]
async fn agent_advisory_dismissable_by_any_human() {
    let Some(base) = cs() else { eprintln!("skipped: set CAIRN_TEST_PG"); return; };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_ag, kid_ag, sk_a, kid_a, _sk_b, _kid_b) = setup(&c).await;
    let p = Uuid::now_v7();
    // Target: an agent-authored, un-owned note (no human author). Human A dismisses it.
    let tgt = author_note(&c, p, &kid_ag, &sk_ag).await;
    let r = try_suppress(&c, p, "salience.downgrade", &kid_a, &sk_a, &tgt, &kid_a, &sk_a).await;
    assert!(r.is_ok(), "an agent advisory must be dismissable by any enrolled human: {r:?}");
}
```

- [ ] **Step 2: Run the tests to verify they fail.**

Run: `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test -p cairn-node --test suppression_owner_gate`
Expected: the two `cross_human_*` tests FAIL (currently the cross-human suppress is *accepted* — no gate exists yet), while the accept tests pass. (Confirms the gate is what's missing, not the harness.)

- [ ] **Step 3: Add the shared helper to `db/005_submit.sql`.**

Insert immediately before the `CREATE OR REPLACE FUNCTION submit_event` declaration (after the `cairn_event_twin` hook, ~line 46):

```sql
-- Suppression owner-gate (ADR-0043 / issue #99). A suppressing overlay
-- (salience.downgrade / visibility.suppress) that forecloses on a HUMAN author's
-- event is self-only: only that human may suppress it. Cross-human suppression is
-- refused — disagreement is expressed additively (a note referencing the target),
-- never by touching another author's content (principle 1/2, paper-parity).
-- An agent-authored / un-owned advisory (no responsible human) stays dismissable by
-- any enrolled human — the clinician-overrides-the-machine path (principle 10), NOT
-- the burying of a colleague.
--
-- The target's human authors = {signer_key_id if it resolves to a kind='human'
-- actor} ∪ {hex(attester_key) if a human attestation is stored}. Empty set ⇒
-- agent/un-owned ⇒ permitted. Non-empty ⇒ permitted only if the attester is in it.
-- STABLE (reads event_log + actor_current). Shared by BOTH doors so a replicated
-- cross-human suppress faces the identical refusal (principle 12). Safe direction:
-- an unknown/ambiguous attester on human-authored content refuses, never permits.
CREATE OR REPLACE FUNCTION cairn_suppression_author_ok(p_target UUID, p_attester_key BYTEA)
RETURNS boolean LANGUAGE sql STABLE AS $$
    WITH tgt AS (
        SELECT el.signer_key_id, el.attester_key
        FROM event_log el WHERE el.event_id = p_target
    ),
    human_authors AS (
        SELECT t.signer_key_id AS kid FROM tgt t
        WHERE EXISTS (SELECT 1 FROM actor_current ac
                      WHERE ac.signing_key_id = t.signer_key_id AND ac.kind = 'human')
        UNION
        SELECT encode(t.attester_key, 'hex') FROM tgt t
        WHERE t.attester_key IS NOT NULL
    )
    SELECT NOT EXISTS (SELECT 1 FROM human_authors)
        OR EXISTS (SELECT 1 FROM human_authors h WHERE h.kid = encode(p_attester_key, 'hex'));
$$;
```

- [ ] **Step 4: Add the gate to `submit_event` step 5.**

In `db/005_submit.sql`, inside the `IF v_targets_other AND (b -> 'payload' ? 'target_event_id') THEN` block, after the existing `overlay targets unknown event` check (~line 177), add:

```sql
        -- ADR-0043 owner-gate: a suppressing overlay of a HUMAN author's event is
        -- self-only. Cross-human suppression is refused; express disagreement
        -- additively. (Agent advisories are un-owned ⇒ cairn_suppression_author_ok
        -- returns TRUE ⇒ dismissable.) p_attester_key is non-NULL here: step 4
        -- already refused a suppressing event without a valid human token.
        IF v_mode = 'suppressing'
           AND NOT cairn_suppression_author_ok(v_target_id, p_attester_key) THEN
            RAISE EXCEPTION 'submit_event: cross-author suppression refused — you may only suppress your own events; express disagreement additively (a note referencing the target). (ADR-0043)';
        END IF;
```

Also update the step-5 `DEFERRED (known limitation ...)` comment block (~line 164–172): replace it with a one-line pointer that the owner-gate is now enforced by `cairn_suppression_author_ok` (ADR-0043), so the file no longer advertises a hole that is closed.

- [ ] **Step 5: Run the tests to verify they pass.**

Run: `CAIRN_TEST_PG=… cargo test -p cairn-node --test suppression_owner_gate`
Expected: all five tests PASS.

- [ ] **Step 6: Run the neighbouring floor suites to confirm no regression.**

Run: `CAIRN_TEST_PG=… cargo test -p cairn-node --test attestation --test identity_repudiate --test recall_epoch`
Expected: all PASS — in particular `attestation::accepts_suppressing_event_with_valid_human_token` (its target is agent-authored ⇒ still dismissable) and the repudiate suite (`targets_other_author=FALSE` ⇒ un-gated).

- [ ] **Step 7: Commit.**

```bash
git add db/005_submit.sql crates/cairn-node/tests/suppression_owner_gate.rs
git commit -m "feat(floor): self-only suppression owner-gate on submit_event (ADR-0043, #99)"
```

---

### Task 2: Remote-apply door (db/020) gate

**Files:**
- Modify: `db/020_apply_remote_event.sql` (step 5, ~line 175–180)
- Modify: `crates/cairn-node/tests/suppression_owner_gate.rs` (add one apply-door test)

**Interfaces:**
- Consumes: the `cairn_suppression_author_ok` helper from Task 1; `apply_remote_event(...)` door; the same `cairn_event` signing helpers.
- Produces: nothing new (door hardening).

- [ ] **Step 1: Write the failing apply-door test.**

Append to `crates/cairn-node/tests/suppression_owner_gate.rs`. Mirror how `tests/apply_remote_event.rs` invokes the apply door (read that file for the exact `apply_remote_event` call signature and its `$1..$n` argument list — attestation token + attester key + `t_effective` offset travel with the event; copy that call shape verbatim). The test: human A authors a note through `submit_event`; then human B's cross-human `salience.downgrade` of it is pushed through `apply_remote_event` and must be refused with the ADR-0043 reason.

```rust
// NOTE: fill the apply_remote_event(...) call from tests/apply_remote_event.rs —
// it passes (signed_bytes, attestation, attester_key, t_effective_offset). The
// assertion is the invariant:
//   let r = /* apply_remote_event with B's cross-human suppress of A's note */;
//   assert!(r.is_err(), "a synced cross-human suppress must be refused at apply");
//   assert!(r.unwrap_err().contains("cross-author suppression refused"));
```

(The implementer copies the concrete `apply_remote_event` argument binding from `tests/apply_remote_event.rs::<existing suppress test>` — do not invent a signature.)

- [ ] **Step 2: Run to verify it fails.**

Run: `CAIRN_TEST_PG=… cargo test -p cairn-node --test suppression_owner_gate apply`
Expected: FAIL — the apply door currently accepts the cross-human suppress.

- [ ] **Step 3: Add the gate to `apply_remote_event` step 5.**

In `db/020_apply_remote_event.sql`, inside the `IF v_targets_other AND (b -> 'payload' ? 'target_event_id') THEN` block, after the `overlay targets unknown event` check (~line 179), add the identical gate (function-name prefix changed):

```sql
        -- ADR-0043 owner-gate (shared helper — see db/005): a replicated cross-human
        -- suppress faces the SAME refusal a locally-authored one does (principle 12).
        -- p_attester_key is non-NULL here (step 4 refused a suppress with no token).
        IF v_mode = 'suppressing'
           AND NOT cairn_suppression_author_ok(v_target_id, p_attester_key) THEN
            RAISE EXCEPTION 'apply_remote_event: cross-author suppression refused — a suppress of another human''s event may not be admitted; disagreement is additive. (ADR-0043)';
        END IF;
```

- [ ] **Step 4: Run to verify it passes.**

Run: `CAIRN_TEST_PG=… cargo test -p cairn-node --test suppression_owner_gate`
Expected: all PASS (Task 1 five + the apply-door test).

- [ ] **Step 5: Run the apply-door suite for regressions.**

Run: `CAIRN_TEST_PG=… cargo test -p cairn-node --test apply_remote_event`
Expected: PASS.

- [ ] **Step 6: Commit.**

```bash
git add db/020_apply_remote_event.sql crates/cairn-node/tests/suppression_owner_gate.rs
git commit -m "feat(floor): enforce the suppression owner-gate at the remote-apply door too (ADR-0043, #99)"
```

---

### Task 3: ADR-0043 + spec bump

**Files:**
- Create: `docs/spec/decisions/0043-suppression-self-only-disagreement-is-additive.md`
- Modify: `docs/spec/decisions/README.md` (ADR index row)
- Modify: `docs/spec/index.md` (spec version 0.43 → 0.44)
- Modify: `docs/spec/write-path.md` (or the §9.6 submit-surface aspect file — locate with `grep -rl "step 5" docs/spec/`) — one line noting the owner-gate is now enforced, pointing at ADR-0043.

**Interfaces:** none (docs).

- [ ] **Step 1: Write ADR-0043.**

Follow the exact house style of an existing recent ADR (read `docs/spec/decisions/0042-concrete-attachment-reference-shape.md` for the heading structure: title, status/date, Context, Decision, Consequences, and the standard footer). Content, faithful to the design doc `docs/superpowers/specs/2026-07-09-suppression-self-only-owner-gate-design.md`:
  - **Context:** #99's owner-gate hole; ADR-0010 conservation-of-responsibility gave accountability but not entitlement; the review framed unrestricted cross-author suppression as a bug.
  - **Decision:** suppression of human-authored content is self-only; disagreement is additive; agent/un-owned advisories stay dismissable (principle 10); the helper's human-author definition; both doors, one helper (principle 12).
  - **Consequences:** the deliberate divergence from ADR-0010 §2 (demotion gated too); the §5.9 sensitivity-sealing carve-out; `repudiate` untouched; safe-refuse direction; no role hierarchy / notification tier introduced.
  - Cross-reference ADR-0010, ADR-0022 (§9.6 step 5.5), ADR-0006 (§5.9), ADR-0030/0007 (principle 10).

- [ ] **Step 2: Add the ADR index row** in `docs/spec/decisions/README.md`, matching the existing table format:

```markdown
| [0043](0043-suppression-self-only-disagreement-is-additive.md) | Suppression is self-only (human-authored content); disagreement is additive; agent advisories dismissable | §9.6/§3.9 (refines 0010/0022) |
```

- [ ] **Step 3: Bump the spec version** in `docs/spec/index.md` from `0.43` to `0.44` and add the one-line §9.6 pointer to ADR-0043.

- [ ] **Step 4: Build the docs to confirm no broken cross-references.**

Run: `uv run --with-requirements docs/requirements.txt -- mkdocs build`
Expected: build succeeds; no warnings about the new ADR link.

- [ ] **Step 5: Commit.**

```bash
git add docs/spec/decisions/0043-suppression-self-only-disagreement-is-additive.md docs/spec/decisions/README.md docs/spec/index.md docs/spec/*.md
git commit -m "docs(spec): ADR-0043 — suppression is self-only; disagreement is additive (#99); spec v0.44"
```

---

### Task 4: HANDOVER/ROADMAP update + close #99

**Files:**
- Modify: `docs/HANDOVER.md` (new session block; note #99 fully closed)
- Modify: `docs/ROADMAP.md` (Phase 2 floor line: owner-gate enforced)

- [ ] **Step 1: Update HANDOVER.md** — add a concise session block (2026-07-09): the suppression owner-gate is now enforced self-only at both doors via `cairn_suppression_author_ok`; #99 fully closed (its other three sub-items were already done 2026-07-02); ADR-0043 / spec v0.44. Prune older per-slice detail if the file approaches 500 lines.

- [ ] **Step 2: Update ROADMAP.md** — in Phase 2, note the ADR-0010/0022 suppressing owner-gate is now enforced in the floor (ADR-0043); remove any "DEFERRED owner-gate" wording.

- [ ] **Step 3: Full workspace test + clippy gate (the exact CI gate).**

Run:
```
cargo test --workspace
CAIRN_TEST_PG=… cargo test -p cairn-node
cargo clippy --workspace --tests -- -D warnings
cargo fmt --all --check
```
Expected: green (pure suite always; DB-gated with the env var set).

- [ ] **Step 4: Commit.**

```bash
git add docs/HANDOVER.md docs/ROADMAP.md
git commit -m "docs: HANDOVER/ROADMAP — suppression owner-gate enforced, #99 closed"
```

- [ ] **Step 5: Push + open PR + update #99.**

```bash
git push -u origin claude/suppression-self-only-owner-gate-99
gh pr create --base main --title "floor: self-only suppression owner-gate (ADR-0043, closes #99)" --body "<summary + Closes #99>"
gh issue comment 99 --body "3/4 sub-items were already fixed 2026-07-02 (recall epoch, recall FK, actor_current tiebreak). The 4th — the suppression owner-gate — is resolved by ADR-0043: suppression of human-authored content is self-only; disagreement is additive; agent advisories stay dismissable. Enforced at both write doors. Closing on merge of the linked PR."
```

## Self-Review

- **Spec coverage:** helper (Task 1 Step 3) ✓; local gate (Task 1 Step 4) ✓; apply-door gate (Task 2) ✓; all seven design test cases — self/human-signer (1.1), self/human-attester (1.4), agent-dismissable (1.5 + attestation.rs regression 1.6), cross-human downgrade (1.2), cross-human hide (1.3), apply-door refuse (Task 2), repudiate regression (Task 1 Step 6) ✓; ADR + carve-outs (Task 3) ✓; #99 update (Task 4) ✓.
- **Placeholder scan:** the only "fill from existing file" is the `apply_remote_event(...)` argument list in Task 2 Step 1 — deliberately deferred to the concrete existing test signature rather than invented, with an explicit pointer to `tests/apply_remote_event.rs`. All floor SQL and Task-1 test code is complete and literal.
- **Type consistency:** `cairn_suppression_author_ok(uuid, bytea) → boolean` is used identically in db/005 and db/020; `try_suppress`/`author_note`/`body`/`setup` signatures are self-consistent across Task 1 and Task 2.
```
