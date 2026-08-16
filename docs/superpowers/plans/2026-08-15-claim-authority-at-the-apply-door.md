# Claim Authority at the Apply Door — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make a protection-removing claim take effect only when a human this node can hold responsible stands behind it — without refusing anything at either door.

**Architecture:** One SQL predicate (`cairn_claim_authority`) in `db/005`, consulted at exactly one site — the `NOT EXISTS` inside `cairn_sensitivity_standing` — so display coarsening, safety-rung emission and (later) part C's custody dial all inherit it structurally. Authority gates *effect*, never *admission*, and only in the withholding direction: nothing is refused, so nothing forks the event set. An admitted-but-inert withdrawal surfaces on a view; an over-claimed safety rung, which cannot self-heal, gets an append-only flag instead.

**Tech Stack:** PostgreSQL 18 + `cairn_pgx` (pgrx 0.18.1); Rust 2021 workspace (`tokio-postgres`, `tokio`); `psql` SQL mirrors driven by `scripts/run-db-sql-tests.sh`.

**Spec:** [`docs/superpowers/specs/2026-08-15-claim-authority-at-the-apply-door-design.md`](../specs/2026-08-15-claim-authority-at-the-apply-door-design.md) — read it before Task 1. The plan argues from it and does not restate its reasoning.

## Global Constraints

- **Licence:** AGPL-3.0. No new dependencies in this plan; if one becomes necessary, its licence must be AGPL-3.0-compatible and checked *before* adding.
- **TDD, no exceptions.** Failing test first, then the minimal code. This is the safety-critical in-DB floor (§9).
- **Never hard-code cryptographic material in tests** (house rule 6). Keys/seeds derive at runtime via the existing `common::setup` / `common::enroll_human` helpers; never a byte-array or string literal.
- **No new migration file.** All SQL lands in the existing `db/005`, `db/048`, `db/049`. **`SCHEMA_GENERATION` does not move** (it is 49).
- **No new event type, no new projection, no ADR-0057 registry entry** — therefore **none of the four registry row-count pins move.** If you find yourself editing `twin_registry.rs`, `db/tests/034`, `projection_registry.rs` or `db/tests/039`, stop: you have left the plan.
- **Migration replay discipline.** Every DDL statement is `CREATE OR REPLACE` / `CREATE … IF NOT EXISTS`; `connect_and_load_schema` re-runs every `db/*.sql` on every connect.
- **Fixed arity.** `cairn_claim_authority(uuid, uuid)` — never widen the argument list. Postgres *overloads* rather than replaces, and replay never drops what a file stops creating (#404).
- **Every DB-gated test takes `db::test_serial_guard(&base)` BEFORE `connect_and_load_schema`.** Guard, then connect. Every existing suite does this in execution order.
- **UUIDs bind as text:** `cairn-node` has no `with-uuid-1`, so bind `&uuid.to_string()` and cast `$1::text::uuid`.
- **Run cargo with a scratch target dir** to avoid the rust-analyzer `target/` lock: `CARGO_TARGET_DIR=/tmp/cairn-authority cargo test …`.
- **Test env:** `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test"`. Without it DB-gated tests **self-skip and cargo counts them as passed** — a green run proves nothing unless the variable is set.

## Paper-parity benchmark (§1.2)

This changes a clinical workflow at the in-DB floor — the act of lowering a confidentiality grade — so it carries a benchmark rather than the forced-rationale escape. The full argument is the spec's §10.

- **Paper counterpart:** lowering a confidentiality marking on a paper chart — striking the restriction and initialling it. One signed act by a named person.
- **Steps:** paper *N* = 1 (strike + initial). Architecture-forced *M* = 1 — the withdrawal carries the attestation the local door already demands of every locally-authored withdrawal (ADR-0062 decision 7), so no new gesture is added for the clinician doing the work. **M = N — no architecture defect.** UI bundling target *K* = 1, unchanged: the attestation rides the existing sign gesture.
- **Time + cognitive load:** no change at the authoring surface, so no new budget is owed there. The one changed experience is a *reading* one on a peer node — a grade that does not drop when expected — and Task 4's worklist exists to explain it. Budget: an operator must be able to answer *"why did this withdrawal not take effect?"* in **one query with no raw SQL**, satisfied by the `inert` row naming the reason.

The cross-node case has **no paper counterpart at all** — paper does not replicate — so paper-parity constrains the local half of this design and is silent on the remote half. Stated rather than elided.

## File Structure

| file | responsibility | change |
|---|---|---|
| `db/005_submit.sql` | the submit door + the authorship predicate family | **add** `cairn_claim_authority` beside `cairn_authorship_bound`; grants |
| `db/048_sensitivity_stream.sql` | the sensitivity stream, its floors and read model | **modify** `cairn_sensitivity_standing` (one clause); **add** `sensitivity_withdrawal_worklist` view + grant |
| `db/049_safety_projection.sql` | the safety projection and its ladders | **add** `safety_overclaim_flag` table + `cairn_record_safety_overclaim_flag`; **add** the local-door call in `db/005` |
| `crates/cairn-node/tests/claim_authority.rs` | **new** — the predicate in isolation, and the seam's behaviour | create |
| `crates/cairn-node/tests/claim_authority_worklist.rs` | **new** — the two worklist rows | create |
| `crates/cairn-node/tests/safety_overclaim.rs` | **new** — the emission-side flag | create |
| `crates/cairn-event/tests/authority_lockstep.rs` | **new** — Rust↔SQL agreement | create |
| `crates/cairn-sync/src/main.rs` (inline `#[cfg(test)]`) | subset-load coverage | **add** one test driving the new function after `locked_client` |
| `db/tests/005_*`, `db/tests/048_*`, `db/tests/049_*` | SQL mirrors | modify/extend |
| `docs/spec/decisions/0064-*.md` | the *why* | create |
| `docs/spec/identity.md` §5.9 | canonical prose | modify |
| `docs/HANDOVER.md` | current build state | modify — **Task 10 only, at PR stage** |

---

### Task 1: `cairn_claim_authority` — the predicate

**Files:**
- Modify: `db/005_submit.sql` (insert immediately after `cairn_authorship_bound`, ~line 649)
- Test: `crates/cairn-node/tests/claim_authority.rs` (create)

**Interfaces:**
- Consumes: `cairn_attestation_vouched(uuid)` (db/001), `actor_current` (db/004), `event_log.actor_id` / `.attester_key` (db/001).
- Produces: `cairn_claim_authority(p_event_id uuid, p_target_event_id uuid) RETURNS text` — one of the exact strings `'attested'`, `'self'`, `'unverified'`. Later tasks depend on these three literals and on the fixed two-argument signature.

- [ ] **Step 1: Write the failing test file**

Create `crates/cairn-node/tests/claim_authority.rs`:

```rust
//! `cairn_claim_authority` (db/005) — what makes a claim authoritative.
//!
//! Authority is a HUMAN actor this node can hold responsible, by either of two routes:
//! R1 a vouched human attestation, R2 human self-withdrawal of one's own claim. Everything
//! else is 'unverified'. See ADR-0064 and
//! docs/superpowers/specs/2026-08-15-claim-authority-at-the-apply-door-design.md.
mod common;
use cairn_event::sensitivity::*;
use common::{cs, enroll_human, setup, submit_attested, submit_registration, submit_signed, EventSpec};
use uuid::Uuid;

/// Ask the predicate directly. `target` may be None (R1-only callers pass SQL NULL).
async fn authority(
    c: &tokio_postgres::Client,
    event: Uuid,
    target: Option<Uuid>,
) -> String {
    c.query_one(
        "SELECT cairn_claim_authority($1::text::uuid, $2::text::uuid)",
        &[&event.to_string(), &target.map(|t| t.to_string())],
    )
    .await
    .unwrap()
    .get(0)
}

/// A standing assertion, submitted by `sk`/`kid`, returning its event id.
async fn assert_grade(
    c: &tokio_postgres::Client,
    sk: &ed25519_dalek::SigningKey,
    kid: &str,
    patient: Uuid,
    wall: i64,
) -> Uuid {
    let a = SensitivityAssertion {
        subject_kind: SubjectKind::Patient,
        subject_id: patient,
        grade: "sequestered",
        source: "human",
        rationale: Some("protected witness"),
    };
    let id = Uuid::now_v7();
    common::submit_signed_with_id(
        c,
        sk,
        kid,
        id,
        EventSpec {
            patient,
            event_type: SENSITIVITY_EVENT_TYPE,
            schema_version: SENSITIVITY_SCHEMA_VERSION,
            payload: sensitivity_assertion_body(&a),
            plaintext_twin: Some(render_sensitivity_twin(&a)),
            wall,
        },
    )
    .await
    .unwrap();
    id
}

#[tokio::test]
async fn an_unattested_claim_is_unverified() {
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &["sensitivity_assertion", "sensitivity_withdrawal"]).await;

    let p = Uuid::now_v7();
    submit_registration(&c, &sk, &kid, p, 1).await;
    let a = assert_grade(&c, &sk, &kid, p, 10).await;

    // The device key signed it; no attestation rides it, and the signer is not human.
    assert_eq!(authority(&c, a, None).await, "unverified");
}

#[tokio::test]
async fn an_event_with_no_attestation_at_all_is_unverified() {
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &["sensitivity_assertion", "sensitivity_withdrawal"]).await;

    let p = Uuid::now_v7();
    submit_registration(&c, &sk, &kid, p, 1).await;
    let a = assert_grade(&c, &sk, &kid, p, 10).await;

    // THE GUARD THIS TEST EXISTS FOR: cairn_attestation_vouched returns TRUE for an event
    // carrying NO attestation, because "vouched" is the ABSENCE of an unvouched marker row.
    // So `attester_key IS NOT NULL` is the actual R1 test; drop it and every unattested
    // event in the log grades 'attested'.
    assert!(
        c.query_one(
            "SELECT cairn_attestation_vouched($1::text::uuid)",
            &[&a.to_string()]
        )
        .await
        .unwrap()
        .get::<_, bool>(0),
        "precondition: an unattested event is vacuously 'vouched'"
    );
    assert_eq!(authority(&c, a, None).await, "unverified");
}

#[tokio::test]
async fn a_vouched_human_attestation_is_attested() {
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &["sensitivity_assertion", "sensitivity_withdrawal"]).await;
    let (sk_h, kid_h) = enroll_human(&c).await;

    let p = Uuid::now_v7();
    submit_registration(&c, &sk, &kid, p, 1).await;
    let a = assert_grade(&c, &sk, &kid, p, 10).await;

    let w = SensitivityWithdrawal {
        withdraws: content_address_of(&c, a).await,
        rationale: "patient consented",
    };
    let wid = Uuid::now_v7();
    let body = withdrawal_body_with_id(p, wid, &w, 20);
    submit_attested(&c, &sk, body, &sk_h, &kid_h).await.unwrap();

    assert_eq!(authority(&c, wid, Some(a)).await, "attested");
}

#[tokio::test]
async fn a_human_withdrawing_their_own_assertion_is_self() {
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &["sensitivity_assertion", "sensitivity_withdrawal"]).await;
    let (sk_h, kid_h) = enroll_human(&c).await;

    let p = Uuid::now_v7();
    submit_registration(&c, &sk, &kid, p, 1).await;
    // The HUMAN signs the assertion, so actor_id on both rows is that human's actor.
    let a = assert_grade(&c, &sk_h, &kid_h, p, 10).await;

    let w = SensitivityWithdrawal {
        withdraws: content_address_of(&c, a).await,
        rationale: "mine to lower",
    };
    let wid = Uuid::now_v7();
    let body = withdrawal_body_with_id(p, wid, &w, 20);
    common::submit_signed_raw(&c, &sk_h, body).await.unwrap();

    assert_eq!(authority(&c, wid, Some(a)).await, "self");
}

#[tokio::test]
async fn an_advisory_actor_cannot_self_withdraw_its_own_protective_tag() {
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &["sensitivity_assertion", "sensitivity_withdrawal"]).await;

    let p = Uuid::now_v7();
    submit_registration(&c, &sk, &kid, p, 1).await;
    // `setup` enrols a DEVICE/agent actor. It auto-tags, then tries to strip its own tag.
    let a = assert_grade(&c, &sk, &kid, p, 10).await;

    let w = SensitivityWithdrawal {
        withdraws: content_address_of(&c, a).await,
        rationale: "reconsidered",
    };
    let wid = Uuid::now_v7();
    let body = withdrawal_body_with_id(p, wid, &w, 20);
    common::submit_signed_raw(&c, &sk, body).await.unwrap();

    // ADR-0062 decision 6: dismissing a protective auto-tag is a LOWERING and must route
    // through the ceremony. Without the kind='human' clause on R2 this returns 'self'.
    assert_eq!(authority(&c, wid, Some(a)).await, "unverified");
}

#[tokio::test]
async fn no_target_means_r2_cannot_apply() {
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &["sensitivity_assertion", "sensitivity_withdrawal"]).await;
    let (sk_h, kid_h) = enroll_human(&c).await;

    let p = Uuid::now_v7();
    submit_registration(&c, &sk, &kid, p, 1).await;
    let a = assert_grade(&c, &sk_h, &kid_h, p, 10).await;

    // Same event, NULL target: R2 is unavailable, and this assertion carries no attestation.
    assert_eq!(authority(&c, a, None).await, "unverified");
}
```

Three helpers this file needs do not exist yet; add them to `crates/cairn-node/tests/common/mod.rs` in this step:

```rust
/// The content address of an already-submitted event. Sensitivity withdrawals name their
/// target by content address, not by event id, so tests need the mapping.
pub async fn content_address_of(c: &Client, event_id: Uuid) -> Vec<u8> {
    c.query_one(
        "SELECT content_address FROM event_log WHERE event_id = $1::text::uuid",
        &[&event_id.to_string()],
    )
    .await
    .unwrap()
    .get(0)
}

/// A withdrawal `EventBody` with a caller-chosen event id, so the test can ask the
/// predicate about it afterwards.
pub fn withdrawal_body_with_id(
    patient: Uuid,
    event_id: Uuid,
    w: &cairn_event::sensitivity::SensitivityWithdrawal,
    wall: i64,
) -> EventBody {
    body_from_spec(
        event_id,
        EventSpec {
            patient,
            event_type: cairn_event::sensitivity::SENSITIVITY_WITHDRAWAL_EVENT_TYPE,
            schema_version: cairn_event::sensitivity::SENSITIVITY_SCHEMA_VERSION,
            payload: cairn_event::sensitivity::sensitivity_withdrawal_body(w),
            plaintext_twin: Some(cairn_event::sensitivity::render_withdrawal_twin(w)),
            wall,
        },
    )
}

/// Sign and submit a pre-built body with NO attestation token.
pub async fn submit_signed_raw(
    c: &Client,
    sk: &SigningKey,
    body: EventBody,
) -> Result<u64, tokio_postgres::Error> {
    let signed = sign(&body, sk).unwrap();
    c.execute("SELECT submit_event($1)", &[&signed.signed_bytes])
        .await
}
```

> **Repo trap:** a new `pub fn` in `tests/common/mod.rs` must ALSO be added to the hand-written expected-helper array in `identity_scaffolding_shared.rs`, or `derivation_finds_the_expected_helpers` fails. Add all three names there in this step.
>
> `body_from_spec` and the exact `submit_event` arity are existing internals of `common/mod.rs` — read `submit_signed_with_id` (line ~162) and mirror what it does rather than inventing a shape. If `sensitivity_withdrawal_body` / `render_withdrawal_twin` / `SENSITIVITY_WITHDRAWAL_EVENT_TYPE` are named differently in `crates/cairn-event/src/sensitivity.rs`, use the real names — do not add aliases.

- [ ] **Step 2: Run the tests to verify they fail**

```bash
CARGO_TARGET_DIR=/tmp/cairn-authority \
CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" \
cargo test -p cairn-node --test claim_authority
```

Expected: every test FAILS with `function cairn_claim_authority(uuid, uuid) does not exist`.

- [ ] **Step 3: Implement the predicate**

In `db/005_submit.sql`, immediately after `cairn_authorship_bound`:

```sql
-- ---------------------------------------------------------------------------
-- ADR-0064 — what makes a claim AUTHORITATIVE. The read-side twin of
-- cairn_authorship_bound directly above: that one REFUSES at authoring, this one
-- GRADES at read, and the two must stay in lockstep (the same note
-- crates/cairn-event/src/contributor.rs:118 already carries).
--
-- Authority is a HUMAN ACTOR THIS NODE CAN HOLD RESPONSIBLE — never the relaying
-- machine, never the actor's relationship to the chart. Two sufficient routes:
--   R1 'attested' — a VOUCHED attestation whose attester is an enrolled human actor.
--   R2 'self'     — the claim's actor IS the target's actor, both known, and human.
-- Everything else is 'unverified'.
--
-- !! SECURITY DEFINER IS REQUIRED, NOT STYLISTIC !!
-- cairn_attestation_vouched (db/001) is REVOKEd FROM PUBLIC and event_attestation_unvouched
-- carries no SELECT grant, because db/001 reasons that every caller is a SECURITY DEFINER
-- door or a migration-owned trigger. This caller is NEITHER: cairn_sensitivity_standing
-- (db/048) is a plain LANGUAGE sql function granted to cairn_agent, and a non-definer body
-- runs as the CALLING role whether or not it inlines. Without DEFINER the first
-- cairn_effective_sensitivity call by cairn_agent fails with permission denied — the whole
-- sensitivity read path, broken by a privilege, and ONLY under the product's role. Pinned by
-- claim_authority::the_read_path_works_as_cairn_agent.
--
-- !! `attester_key IS NOT NULL` IS THE ACTUAL R1 TEST !!
-- cairn_attestation_vouched returns TRUE for an event carrying NO attestation, because
-- "vouched" means "no unvouched MARKER row exists". Delete the NULL guard and every
-- unattested event in the log grades 'attested'. Pinned by
-- an_event_with_no_attestation_at_all_is_unverified.
--
-- STRICTER THAN db/020's SIBLING, DELIBERATELY. db/020's forged-human-author check asks
-- `EXISTS (... AND kind='human')`, which admits a key mapped to BOTH a human and an agent.
-- Here that ambiguity would confer power to strip a protective grade, so the key must
-- resolve to EXACTLY ONE actor and that actor must be human. Principle 4: uncertainty
-- withholds power, it never confers it. Not a fix to db/020 — a different question.
--
-- FIXED ARITY. Never widen this argument list: Postgres OVERLOADS on a changed list rather
-- than replacing, and migration replay never drops what a file stops creating, so a stale
-- definition would survive in every existing database and silently serve any call site
-- missed (#404). A caller with no target passes an explicit NULL.
CREATE OR REPLACE FUNCTION cairn_claim_authority(p_event_id uuid, p_target_event_id uuid)
RETURNS text LANGUAGE sql STABLE
SECURITY DEFINER SET search_path = public
AS $$
    SELECT CASE
        -- R1 — a vouched attestation by an enrolled human actor.
        WHEN EXISTS (
            SELECT 1 FROM event_log e
             WHERE e.event_id = p_event_id
               AND e.attester_key IS NOT NULL          -- load-bearing; see header
               AND cairn_attestation_vouched(e.event_id)
               AND (SELECT count(*) = 1 AND bool_and(a.kind = 'human')
                      FROM actor_current a
                     WHERE a.signing_key_id = encode(e.attester_key, 'hex')))
        THEN 'attested'
        -- R2 — the human who made the claim is withdrawing their own.
        WHEN p_target_event_id IS NOT NULL AND EXISTS (
            SELECT 1 FROM event_log c
              JOIN event_log t ON t.event_id = p_target_event_id
              JOIN actor_current a ON a.actor_id = c.actor_id
             WHERE c.event_id = p_event_id
               AND c.actor_id IS NOT NULL              -- NULL = a key on several actors,
               AND t.actor_id IS NOT NULL              -- i.e. attribution honestly unknown
               AND c.actor_id = t.actor_id
               AND a.kind = 'human')                   -- ADR-0062 decision 6
        THEN 'self'
        ELSE 'unverified'
    END;
$$;
-- A SECURITY DEFINER function with PUBLIC execute is a privilege-escalation surface.
REVOKE EXECUTE ON FUNCTION cairn_claim_authority(uuid, uuid) FROM PUBLIC;
GRANT  EXECUTE ON FUNCTION cairn_claim_authority(uuid, uuid) TO cairn_agent;
```

- [ ] **Step 4: Run the tests to verify they pass**

```bash
CARGO_TARGET_DIR=/tmp/cairn-authority \
CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" \
cargo test -p cairn-node --test claim_authority
```

Expected: 6 passed. If `an_advisory_actor_cannot_self_withdraw_its_own_protective_tag` passes while you have *not* written the `a.kind = 'human'` clause, the test is wrong — check that `setup` really enrols a non-human actor.

- [ ] **Step 5: Commit**

```bash
git add db/005_submit.sql crates/cairn-node/tests/claim_authority.rs \
        crates/cairn-node/tests/common/mod.rs \
        crates/cairn-node/tests/identity_scaffolding_shared.rs
git commit -m "feat(#380): cairn_claim_authority — a human this node can hold responsible

Two routes, both requiring a human: a vouched attestation (R1) or
self-withdrawal of one's own claim (R2). SECURITY DEFINER because
cairn_attestation_vouched is locked to definer callers, and the NULL guard
on attester_key is the actual R1 test — 'vouched' means 'no unvouched
marker', which is vacuously true of an unattested event."
```

---

### Task 2: The seam — one clause, and the privilege that breaks everything

**Files:**
- Modify: `db/048_sensitivity_stream.sql:328-337` (`cairn_sensitivity_standing`)
- Test: `crates/cairn-node/tests/claim_authority.rs` (extend)

**Interfaces:**
- Consumes: `cairn_claim_authority(uuid, uuid)` from Task 1.
- Produces: no new symbol. `cairn_sensitivity_standing(uuid)` keeps its signature and return columns exactly; its *behaviour* narrows to authoritative withdrawals only. `cairn_effective_sensitivity`, `cairn_prospective_sensitivity` and `crates/cairn-node/src/sensitivity.rs:319` are unchanged and inherit it.

- [ ] **Step 1: Write the failing tests**

Append to `crates/cairn-node/tests/claim_authority.rs`:

```rust
/// The effective grade of the chart-wide assertion's own event.
async fn effective(c: &tokio_postgres::Client, event: Uuid) -> String {
    c.query_one(
        "SELECT grade FROM cairn_effective_sensitivity($1::text::uuid)",
        &[&event.to_string()],
    )
    .await
    .unwrap()
    .get(0)
}

#[tokio::test]
async fn an_unattested_withdrawal_lands_and_converges_but_does_not_lower() {
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &["sensitivity_assertion", "sensitivity_withdrawal"]).await;

    let p = Uuid::now_v7();
    submit_registration(&c, &sk, &kid, p, 1).await;
    let a = assert_grade(&c, &sk, &kid, p, 10).await;
    let target = content_address_of(&c, a).await;

    let w = SensitivityWithdrawal { withdraws: target.clone(), rationale: "strip it" };
    let wid = Uuid::now_v7();
    common::submit_signed_raw(&c, &sk, withdrawal_body_with_id(p, wid, &w, 20))
        .await
        .expect("ADMITTED — authority gates EFFECT, never admission; a refusal would fork");

    // BOTH halves matter. Assert admission first: if the door started refusing, the
    // "does not lower" assertion below would pass for entirely the wrong reason.
    let landed: i64 = c
        .query_one(
            "SELECT count(*) FROM sensitivity_withdrawal WHERE withdraws = $1",
            &[&target],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(landed, 1, "the withdrawal must land and converge");

    assert_eq!(
        effective(&c, a).await,
        "sequestered",
        "an un-attested withdrawal must not lower the grade (#380)"
    );
}

#[tokio::test]
async fn an_attested_cross_node_withdrawal_lowers() {
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &["sensitivity_assertion", "sensitivity_withdrawal"]).await;
    let (sk_h, kid_h) = enroll_human(&c).await;

    let p = Uuid::now_v7();
    submit_registration(&c, &sk, &kid, p, 1).await;
    let a = assert_grade(&c, &sk, &kid, p, 10).await;

    let w = SensitivityWithdrawal {
        withdraws: content_address_of(&c, a).await,
        rationale: "patient consented",
    };
    let wid = Uuid::now_v7();
    submit_attested(&c, &sk, withdrawal_body_with_id(p, wid, &w, 20), &sk_h, &kid_h)
        .await
        .unwrap();

    assert_eq!(effective(&c, a).await, "routine", "no deadlock: attesting is the remedy");
}

#[tokio::test]
async fn a_locally_authored_withdrawal_always_lowers() {
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &["sensitivity_assertion", "sensitivity_withdrawal"]).await;
    let (sk_h, kid_h) = enroll_human(&c).await;

    let p = Uuid::now_v7();
    submit_registration(&c, &sk, &kid, p, 1).await;
    let a = assert_grade(&c, &sk, &kid, p, 10).await;

    // The LOCAL door already demands a bound human author for a withdrawal (ADR-0062
    // decision 7), so anything it accepts clears the bar BY CONSTRUCTION. This pins the
    // no-deadlock claim instead of asserting it in prose.
    let w = SensitivityWithdrawal {
        withdraws: content_address_of(&c, a).await,
        rationale: "clinician lowered it",
    };
    let wid = Uuid::now_v7();
    submit_attested(&c, &sk, withdrawal_body_with_id(p, wid, &w, 20), &sk_h, &kid_h)
        .await
        .expect("the local ceremony accepted it, so authority must too");
    assert_eq!(effective(&c, a).await, "routine");
}

#[tokio::test]
async fn the_read_path_works_as_cairn_agent() {
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &["sensitivity_assertion", "sensitivity_withdrawal"]).await;

    let p = Uuid::now_v7();
    submit_registration(&c, &sk, &kid, p, 1).await;
    let a = assert_grade(&c, &sk, &kid, p, 10).await;

    // Slice 62's lesson in the privilege dimension: a control the OWNER can exercise and
    // the product's role cannot is not a control. cairn_attestation_vouched is REVOKEd
    // from PUBLIC and event_attestation_unvouched has no SELECT grant, so without
    // SECURITY DEFINER on cairn_claim_authority this raises 42501 and the entire
    // sensitivity read path is dead for cairn_agent.
    c.batch_execute("SET ROLE cairn_agent").await.unwrap();
    let grade: String = c
        .query_one(
            "SELECT grade FROM cairn_effective_sensitivity($1::text::uuid)",
            &[&a.to_string()],
        )
        .await
        .expect("cairn_agent must be able to read the effective grade")
        .get(0);
    c.batch_execute("RESET ROLE").await.unwrap();
    assert_eq!(grade, "sequestered");
}
```

- [ ] **Step 2: Run to verify they fail**

```bash
CARGO_TARGET_DIR=/tmp/cairn-authority \
CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" \
cargo test -p cairn-node --test claim_authority
```

Expected: `an_unattested_withdrawal_lands_and_converges_but_does_not_lower` FAILS with `assertion failed: left == right, left: "routine", right: "sequestered"` — the withdrawal currently lowers unconditionally. The other three PASS already (they describe behaviour that is correct today); that is expected and fine — they are regression pins, and one of them will start failing the moment you get Step 3 half-right.

- [ ] **Step 3: Add the clause**

In `db/048_sensitivity_stream.sql`, replace `cairn_sensitivity_standing`'s body:

```sql
-- A withdrawal only counts if it is AUTHORITATIVE (ADR-0064). This one clause is the whole
-- of §5.9's protection-removing control, and it is HERE — the single definition of "what
-- still applies" that cairn_effective_sensitivity (section 11), db/049's
-- cairn_prospective_sensitivity and the CLI read path all delegate to — precisely so no
-- consumer can be written that forgets it, and so part C's custody dial inherits it for
-- free. Do NOT push this check up into the callers: that is the per-dial duplication that
-- produced #404 and #399 one file over.
--
-- The withdrawal stays in the log, replicates, converges and is re-assertable; it simply
-- does not participate in this set difference. Nothing is refused at either door, so
-- nothing forks (#342), and nothing PROTECTIVE is ever gated — only lowering is.
CREATE OR REPLACE FUNCTION cairn_sensitivity_standing(p_patient_id uuid)
RETURNS TABLE (content_address bytea, subject_kind text, subject_id uuid, grade text)
LANGUAGE sql STABLE AS $$
    SELECT a.content_address, a.subject_kind, a.subject_id, a.grade
    FROM sensitivity_assertion a
    WHERE a.patient_id = p_patient_id
      AND NOT EXISTS (SELECT 1 FROM sensitivity_withdrawal w
                       WHERE w.withdraws = a.content_address
                         AND w.patient_id = p_patient_id
                         AND cairn_claim_authority(w.event_id, a.event_id) <> 'unverified');
$$;
```

- [ ] **Step 4: Run the full sensitivity + safety suites**

```bash
CARGO_TARGET_DIR=/tmp/cairn-authority \
CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" \
cargo test -p cairn-node --test claim_authority --test sensitivity_ladder \
  --test sensitivity_floor --test sensitivity_ceremony --test sensitivity_convergence \
  --test safety_read --test safety_emission --test safety_ladder --test safety_doors \
  --test safety_carried_class
```

Expected: all pass. **Existing sensitivity tests that submit an unattested withdrawal and expect a lowered grade will fail** — that is the design change, not a regression. Fix each by attesting the withdrawal (`submit_attested` + `enroll_human`), and add a one-line comment at each edit saying *why* it now needs an attestation, citing ADR-0064. Do **not** weaken the new clause to keep an old test green.

- [ ] **Step 5: Prove the clause is actually load-bearing (mutation check)**

Temporarily delete the `AND cairn_claim_authority(...)` line, re-run the command from Step 4, and confirm `an_unattested_withdrawal_lands_and_converges_but_does_not_lower` goes red. Restore the line.

This is #404's lesson applied on the first try: db/049's class gate was pinned by *nothing*, and widening it left all 26 safety tests and 21 SQL mirrors green. Record the observed result in the commit message. If nothing goes red, the test is not reaching the seam — fix the test before continuing.

- [ ] **Step 6: Commit**

```bash
git add db/048_sensitivity_stream.sql crates/cairn-node/tests/
git commit -m "feat(#380): only an authoritative withdrawal lowers a grade

One clause in cairn_sensitivity_standing — the single definition of 'what
still applies' that the display read model, the emission read model and the
CLI all delegate to, so every consumer inherits it structurally and part C's
custody dial will too.

Nothing is refused: the withdrawal lands, converges and is re-assertable, it
just stops participating in the set difference. Both of ADR-0062 decision 7's
arguments for a lenient apply door survive untouched.

Mutation-checked: deleting the clause turns
an_unattested_withdrawal_lands_and_converges_but_does_not_lower red."
```

---

### Task 3: Arrival-order independence — both self-heal paths

**Files:**
- Test: `crates/cairn-node/tests/claim_authority.rs` (extend)

**Interfaces:**
- Consumes: everything from Tasks 1–2. Produces nothing new — this task is entirely tests, and it exists because ADR-0062 decision 3 makes arrival order a *hard requirement*, not a nicety.

- [ ] **Step 1: Write the failing tests**

```rust
#[tokio::test]
async fn a_withdrawal_inert_because_its_target_has_not_replicated_heals_when_it_lands() {
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &["sensitivity_assertion", "sensitivity_withdrawal"]).await;
    let (sk_h, kid_h) = enroll_human(&c).await;

    let p = Uuid::now_v7();
    submit_registration(&c, &sk, &kid, p, 1).await;

    // Set-union sync has no ordering: the withdrawal legitimately arrives FIRST
    // (ADR-0062 decision 3). Its target's event_id is knowable — it is content-addressed —
    // but the row is not here yet, so R2 cannot resolve. R1 must carry it alone.
    let future_assert_id = Uuid::now_v7();
    let a = SensitivityAssertion {
        subject_kind: SubjectKind::Patient,
        subject_id: p,
        grade: "sequestered",
        source: "human",
        rationale: Some("protected witness"),
    };
    let assert_body = common::body_from_spec(
        future_assert_id,
        EventSpec {
            patient: p,
            event_type: SENSITIVITY_EVENT_TYPE,
            schema_version: SENSITIVITY_SCHEMA_VERSION,
            payload: sensitivity_assertion_body(&a),
            plaintext_twin: Some(render_sensitivity_twin(&a)),
            wall: 10,
        },
    );
    let target_ca = cairn_event::event_address(&cairn_event::sign(&assert_body, &sk).unwrap().signed_bytes);

    let w = SensitivityWithdrawal { withdraws: target_ca.clone(), rationale: "consented" };
    let wid = Uuid::now_v7();
    submit_attested(&c, &sk, withdrawal_body_with_id(p, wid, &w, 20), &sk_h, &kid_h)
        .await
        .unwrap();

    // R2 cannot resolve (no target row), but R1 stands on its own.
    assert_eq!(authority(&c, wid, None).await, "attested");

    // Now the target lands. The withdrawal must take effect — a delete-at-apply design
    // would have dropped it on the floor.
    common::submit_signed_raw(&c, &sk, assert_body).await.unwrap();
    assert_eq!(effective(&c, future_assert_id).await, "routine");
}

#[tokio::test]
async fn a_withdrawal_inert_because_its_attester_is_unknown_heals_on_enrolment() {
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &["sensitivity_assertion", "sensitivity_withdrawal"]).await;

    let p = Uuid::now_v7();
    submit_registration(&c, &sk, &kid, p, 1).await;
    let a = assert_grade(&c, &sk, &kid, p, 10).await;

    // A withdrawal authored by a human who is NOT YET enrolled here. Actor registries
    // federate (ADR-0054) with an operator ceremony, so this is ordinary, honest traffic
    // on a node that has not caught up — not an attack.
    let (sk_h, kid_h) = common::human_key_not_yet_enrolled();
    let w = SensitivityWithdrawal {
        withdraws: content_address_of(&c, a).await,
        rationale: "consented",
    };
    let wid = Uuid::now_v7();
    common::apply_remote_attested(&c, &sk, withdrawal_body_with_id(p, wid, &w, 20), &sk_h, &kid_h)
        .await
        .unwrap();

    assert_eq!(
        effective(&c, a).await,
        "sequestered",
        "honest divergence in the SAFE direction while the attester is unknown here"
    );

    // The operator enrols the peer's clinician. The grade FALLS. Per ADR-0062 decision 9
    // as extended by ADR-0064, that is not a bug report.
    common::enroll_human_key(&c, &kid_h).await;
    assert_eq!(effective(&c, a).await, "routine");
}
```

> Three more `common` helpers are needed: `body_from_spec` (make it `pub` if it is currently private), `human_key_not_yet_enrolled()` (derive a key at runtime — **house rule 6: no literals**), `apply_remote_attested` (submit through `apply_remote_event`, not `submit_event` — the remote door is the only one that admits an unknown attester) and `enroll_human_key(c, kid)`. Add every new `pub fn` name to `identity_scaffolding_shared.rs`'s expected-helper array. Model `apply_remote_attested` on how `crates/cairn-sync/tests/clinical_pull.rs` drives `apply_remote_event`.
>
> **If the remote door refuses an unknown attester outright** (db/020:252 raises *"attester is not an enrolled human actor"*), then the honest-divergence scenario cannot arise via that path — the event never lands at all. In that case: delete this second test, and instead add one line to ADR-0064's *Known limitations* recording that the attester-unknown divergence is **not reachable** because the apply door already refuses such an event, and that the only live divergence axis is the unreplicated target. Verify which is true before writing either; do not guess.

- [ ] **Step 2: Run to verify the first test fails, then implement nothing**

```bash
CARGO_TARGET_DIR=/tmp/cairn-authority \
CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" \
cargo test -p cairn-node --test claim_authority
```

These tests should **pass without new production code** — Tasks 1–2 already compute at read, which is what makes arrival-order independence fall out. If either fails, the predicate has cached or stamped something it should not have; fix the predicate, not the test.

- [ ] **Step 3: Commit**

```bash
git add crates/cairn-node/tests/
git commit -m "test(#380): arrival-order independence survives the authority gate

Computing authority at READ rather than stamping it at apply is what makes
these pass with no new production code: a withdrawal inert today because its
target has not replicated takes effect the moment it does (ADR-0062 decision 3),
and a grade that falls when an attester enrols is not a bug report."
```

---

### Task 4: `sensitivity_withdrawal_worklist` — two rows

**Files:**
- Modify: `db/048_sensitivity_stream.sql` (append after the read model, before the grants block ~line 665)
- Test: `crates/cairn-node/tests/claim_authority_worklist.rs` (create)

**Interfaces:**
- Consumes: `cairn_claim_authority`, `sensitivity_withdrawal`, `sensitivity_assertion`, `event_log`.
- Produces: view `sensitivity_withdrawal_worklist (content_address bytea, event_id uuid, patient_id uuid, withdraws bytea, reason text, node_origin text, rationale text)` where `reason` is exactly `'inert'` or `'stranger-attested'`.

- [ ] **Step 1: Write the failing test**

Create `crates/cairn-node/tests/claim_authority_worklist.rs`:

```rust
//! The §5.9 withdrawal worklist (ADR-0064). Two rows, two reasons: `inert` is what the
//! gate stopped (transient, self-clearing), `stranger-attested` is what the gate LET
//! THROUGH and nobody would otherwise see.
mod common;
use cairn_event::sensitivity::*;
use common::{cs, content_address_of, enroll_human, setup, submit_attested, submit_registration,
             withdrawal_body_with_id, EventSpec};
use uuid::Uuid;

async fn reasons(c: &tokio_postgres::Client, patient: Uuid) -> Vec<String> {
    c.query(
        "SELECT reason FROM sensitivity_withdrawal_worklist
          WHERE patient_id = $1::text::uuid ORDER BY reason",
        &[&patient.to_string()],
    )
    .await
    .unwrap()
    .iter()
    .map(|r| r.get(0))
    .collect()
}

#[tokio::test]
async fn an_inert_withdrawal_is_listed_and_clears_when_it_becomes_authoritative() {
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &["sensitivity_assertion", "sensitivity_withdrawal"]).await;
    let (sk_h, kid_h) = enroll_human(&c).await;

    let p = Uuid::now_v7();
    submit_registration(&c, &sk, &kid, p, 1).await;
    let a = common::assert_chart_grade(&c, &sk, &kid, p, 10, "sequestered").await;
    let target = content_address_of(&c, a).await;

    let w = SensitivityWithdrawal { withdraws: target.clone(), rationale: "strip" };
    common::submit_signed_raw(&c, &sk, withdrawal_body_with_id(p, Uuid::now_v7(), &w, 20))
        .await
        .unwrap();
    assert_eq!(reasons(&c, p).await, vec!["inert"]);

    // Re-issued WITH an attestation: the grade lowers and the inert row disappears,
    // because the view asks the CURRENT question rather than replaying a stamped verdict.
    let w2 = SensitivityWithdrawal { withdraws: target, rationale: "consented" };
    submit_attested(&c, &sk, withdrawal_body_with_id(p, Uuid::now_v7(), &w2, 30), &sk_h, &kid_h)
        .await
        .unwrap();
    assert!(!reasons(&c, p).await.contains(&"inert".to_string()));
}

#[tokio::test]
async fn an_attested_withdrawal_from_a_stranger_to_the_chart_is_listed() {
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &["sensitivity_assertion", "sensitivity_withdrawal"]).await;
    let (sk_h, kid_h) = enroll_human(&c).await;

    let p = Uuid::now_v7();
    submit_registration(&c, &sk, &kid, p, 1).await;
    let a = common::assert_chart_grade(&c, &sk, &kid, p, 10, "sequestered").await;

    // The attesting human has authored nothing else on this chart, and the withdrawal was
    // authored elsewhere. It CLEARS the bar — the grade does lower — and that is exactly
    // why it must be visible: accountability is the control, the gate is only the forcing
    // function (ADR-0064).
    let w = SensitivityWithdrawal {
        withdraws: content_address_of(&c, a).await,
        rationale: "consented",
    };
    common::apply_remote_attested(
        &c, &sk, withdrawal_body_with_id(p, Uuid::now_v7(), &w, 20), &sk_h, &kid_h,
    )
    .await
    .unwrap();

    assert_eq!(reasons(&c, p).await, vec!["stranger-attested"]);
}

#[tokio::test]
async fn a_local_clinicians_own_withdrawal_is_not_on_the_worklist() {
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup(&c, &["sensitivity_assertion", "sensitivity_withdrawal"]).await;
    let (sk_h, kid_h) = enroll_human(&c).await;

    let p = Uuid::now_v7();
    submit_registration(&c, &sk, &kid, p, 1).await;
    // The human authored content on this chart first, then withdraws locally.
    let a = common::assert_chart_grade(&c, &sk_h, &kid_h, p, 10, "sequestered").await;
    let w = SensitivityWithdrawal {
        withdraws: content_address_of(&c, a).await,
        rationale: "consented",
    };
    submit_attested(&c, &sk, withdrawal_body_with_id(p, Uuid::now_v7(), &w, 20), &sk_h, &kid_h)
        .await
        .unwrap();

    // The routine case must produce NO noise, or the worklist is unusable on day one
    // (§5.12 alert fatigue — the disease this project names as the enemy).
    assert!(reasons(&c, p).await.is_empty());
}
```

> Refactor `assert_grade` from Task 1 into `common::assert_chart_grade(c, sk, kid, patient, wall, grade)` so both suites share it, and register the name in `identity_scaffolding_shared.rs`.

- [ ] **Step 2: Run to verify it fails**

```bash
CARGO_TARGET_DIR=/tmp/cairn-authority \
CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" \
cargo test -p cairn-node --test claim_authority_worklist
```

Expected: FAIL with `relation "sensitivity_withdrawal_worklist" does not exist`.

- [ ] **Step 3: Add the view**

```sql
-- ---------------------------------------------------------------------------
-- The §5.9 withdrawal worklist (ADR-0064). A VIEW, deliberately, and not a flag ledger.
--
-- WHY NOT THE ADR-0058 t_effective_ceiling_flag IDIOM ONE FILE OVER: that records a
-- judgement AT THE DOOR, and authority is computed at READ precisely because the answer
-- IMPROVES — a withdrawal is inert today because its target has not replicated or its
-- attester is not enrolled here, and clears tomorrow. An apply-time ledger would fill with
-- rows that were true for an afternoon, and a worklist that is mostly stale is §5.12's
-- alert-fatigue disease, self-inflicted, in the one place we are building a control.
-- The rule for choosing: FLAG WHAT CANNOT SELF-HEAL; VIEW WHAT CAN. db/049's
-- safety-overclaim flag is a published byte and takes the other branch.
--
-- Two reasons, and the second is the one nothing else in the system would show:
--   'inert'             — the gate stopped it. Transient; disappears when it heals.
--   'stranger-attested' — the gate LET IT THROUGH. An accountable human lowered a grade on
--                         a chart they have authored nothing else on, from another node.
--                         Permanent, because it is a fact about a completed act.
--
-- 'stranger-attested' reuses the chart-standing question that ADR-0064 REJECTED as an
-- authority input. That is not an inconsistency: as authority it fails the locum, the
-- night-cover registrar and the receiving ED, who must not be second-class; as SALIENCE it
-- blocks nothing and delays nothing — the withdrawal has already taken effect. §5.13's
-- duplicate-sweep posture: surface, never block.
CREATE OR REPLACE VIEW sensitivity_withdrawal_worklist AS
SELECT w.content_address,
       w.event_id,
       w.patient_id,
       w.withdraws,
       CASE WHEN cairn_claim_authority(w.event_id, a.event_id) = 'unverified'
            THEN 'inert' ELSE 'stranger-attested' END AS reason,
       w.node_origin,
       w.rationale
FROM sensitivity_withdrawal w
LEFT JOIN sensitivity_assertion a ON a.content_address = w.withdraws
WHERE cairn_claim_authority(w.event_id, a.event_id) = 'unverified'
   OR (w.node_origin IS DISTINCT FROM cairn_this_node_origin()
       AND NOT EXISTS (
           SELECT 1 FROM event_log e
            WHERE e.patient_id = w.patient_id
              AND e.actor_id = (SELECT actor_id FROM event_log
                                 WHERE event_id = w.event_id)
              AND e.event_id <> w.event_id));
GRANT SELECT ON sensitivity_withdrawal_worklist TO cairn_agent;
```

> `cairn_this_node_origin()` is a placeholder for however this schema already identifies the local node. **Find the real one** — `db/001`/`db/038` set `node_origin` on every locally-submitted row; grep for what writes it in `db/005_submit.sql` and use that same expression. If no such helper exists, compare against the value `submit_event` stamps rather than inventing a function.

- [ ] **Step 4: Run to verify it passes**

```bash
CARGO_TARGET_DIR=/tmp/cairn-authority \
CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" \
cargo test -p cairn-node --test claim_authority_worklist
```

Expected: 3 passed. If `a_local_clinicians_own_withdrawal_is_not_on_the_worklist` fails, the `node_origin` comparison is wrong — fix that before proceeding; a worklist that lists routine local work is worse than no worklist.

- [ ] **Step 5: Commit**

```bash
git add db/048_sensitivity_stream.sql crates/cairn-node/tests/
git commit -m "feat(#380): the withdrawal worklist — inert, and stranger-attested

A view rather than a flag ledger, because authority is computed at read and
the answer improves: an apply-time ledger would fill with rows that were true
for an afternoon. Flag what cannot self-heal; view what can.

The second row is the detect half of #380 that survives the gate — an attested
strip by a human with no other presence on the chart takes effect immediately
and would otherwise be invisible."
```

---

### Task 5: `cairn_record_safety_overclaim_flag` — the other branch of the rule

**Files:**
- Modify: `db/049_safety_projection.sql` (append after the ladders)
- Modify: `db/005_submit.sql` (step 1d, beside the existing `cairn_check_safety_signal` call ~line 838)
- Test: `crates/cairn-node/tests/safety_overclaim.rs` (create)

**Interfaces:**
- Consumes: `cairn_safety_rung_rank(text)`, `cairn_safety_rung_for_rank(int)`, `cairn_sensitivity_rank(text)`, `cairn_prospective_sensitivity(uuid, uuid)` — all existing in db/049.
- Produces: table `safety_overclaim_flag (content_address bytea PRIMARY KEY, patient_id uuid, emitted_rung text, licensed_rung text, recorded_at timestamptz)` and `cairn_record_safety_overclaim_flag(bytea, uuid, text, text)`.

- [ ] **Step 1: Write the failing test**

Create `crates/cairn-node/tests/safety_overclaim.rs`:

```rust
//! #405 part 2 — a rung the chart's grade does not license is recorded, never refused.
//!
//! ADR-0060 forbids an advisory field cancelling a medication assert, and the door cannot
//! rewrite event_log.safety without making the column disagree with signed_bytes. So the
//! door records instead: the bypass becomes auditable at zero clinical cost.
mod common;
use common::{cs, medication_setup};
use uuid::Uuid;

#[tokio::test]
async fn a_precise_rung_on_a_sequestered_chart_is_admitted_and_flagged() {
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid, sk_h, kid_h) = medication_setup(&c).await;

    let p = Uuid::now_v7();
    common::submit_registration(&c, &sk, &kid, p, 1).await;
    common::assert_chart_grade(&c, &sk, &kid, p, 10, "sequestered").await;

    // A hostile client bypassing apply_safety_rung: it signs a body whose clear safety
    // field claims `precise` on a chart this node grades `sequestered` (licensed:
    // existence). Spike-0002's C1-C5 threat model, treated here as live.
    let ca = common::submit_medication_with_raw_safety(
        &c, &sk, &kid, &sk_h, &kid_h, p, 20,
        serde_json::json!({"rung":"precise","class":"antiretroviral-interaction","severity":"high"}),
    )
    .await
    .expect("ADMITTED — an advisory field may never cancel a clinical write (ADR-0060)");

    let (emitted, licensed): (String, String) = {
        let r = c
            .query_one(
                "SELECT emitted_rung, licensed_rung FROM safety_overclaim_flag
                  WHERE content_address = $1",
                &[&ca],
            )
            .await
            .expect("the overclaim must be recorded");
        (r.get(0), r.get(1))
    };
    assert_eq!((emitted.as_str(), licensed.as_str()), ("precise", "existence"));

    // The read model still coarsens — the flag bounds the SILENCE, not the effect.
    let rung: String = c
        .query_one(
            "SELECT rung FROM cairn_event_safety(
                (SELECT event_id FROM event_log WHERE content_address = $1))",
            &[&ca],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(rung, "existence");
}

#[tokio::test]
async fn a_licensed_rung_is_not_flagged() {
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid, sk_h, kid_h) = medication_setup(&c).await;

    let p = Uuid::now_v7();
    common::submit_registration(&c, &sk, &kid, p, 1).await;
    // No grade at all: `precise` is exactly what rank 0 licenses.
    let ca = common::submit_medication_with_raw_safety(
        &c, &sk, &kid, &sk_h, &kid_h, p, 20,
        serde_json::json!({"rung":"precise","class":"rh-sensitizing","severity":"high"}),
    )
    .await
    .unwrap();

    let n: i64 = c
        .query_one(
            "SELECT count(*) FROM safety_overclaim_flag WHERE content_address = $1",
            &[&ca],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(n, 0, "the ordinary path must produce no noise");
}
```

> `submit_medication_with_raw_safety` must build the body with the given `safety` value **verbatim**, bypassing `apply_safety_rung` — that is the whole point. Model it on `crates/cairn-node/src/medication/sealed_submit.rs`'s seal/sign path, and register the helper name in `identity_scaffolding_shared.rs`.

- [ ] **Step 2: Run to verify it fails**

```bash
CARGO_TARGET_DIR=/tmp/cairn-authority \
CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" \
cargo test -p cairn-node --test safety_overclaim
```

Expected: FAIL with `relation "safety_overclaim_flag" does not exist`.

- [ ] **Step 3: Implement the ledger and the door call**

In `db/049_safety_projection.sql`:

```sql
-- ---------------------------------------------------------------------------
-- #405 part 2 — a rung finer than the chart's grade licenses. RECORDED, never refused.
--
-- The door CANNOT refuse it: ADR-0060 forbids an advisory field cancelling a medication
-- assert, and rewriting event_log.safety would make the column disagree with signed_bytes
-- and quietly break the signature's meaning. So it takes ADR-0058's record-a-flag idiom.
--
-- !! LOCAL DOOR ONLY — AND THIS DELIBERATELY BREAKS THE PRECEDENT IT COPIES !!
-- cairn_record_ceiling_flag is called at BOTH doors (db/005:825 and db/020:145), so
-- local-only here reads as an oversight and WILL be tidied into symmetry. It is not:
--   * LOCALLY the node's own grade is authoritative for its own authoring, so a rung finer
--     than it licenses is unambiguously anomalous — apply_safety_rung was bypassed.
--   * REMOTELY ADR-0063 decision 2 says this arrives ROUTINELY AND HONESTLY: an older peer
--     predating the slice, a differently-custodial peer computing a lower grade, and a
--     hostile peer all deliver identical bytes and cannot be told apart. Flagging there
--     would fire on ordinary traffic and accuse honest peers — §5.12 alert fatigue, in a
--     ledger nobody could then trust.
-- A clock grade is a claim about the authoring node's own clock and stays meaningful at
-- both doors; a safety rung is a claim about THIS chart's grade, which is node-relative.
-- Same idiom, different question.
--
-- A LEDGER and not a view (ADR-0064's rule): a published byte is permanent and can never
-- improve, so there is nothing to self-heal.
CREATE TABLE IF NOT EXISTS safety_overclaim_flag (
    content_address BYTEA PRIMARY KEY,
    patient_id      UUID        NOT NULL,
    emitted_rung    TEXT        NOT NULL,
    licensed_rung   TEXT        NOT NULL,
    recorded_at     TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp()
);
GRANT SELECT ON safety_overclaim_flag TO cairn_agent;

-- Idempotent on replay: the PK is the content address, so a re-offered event re-records
-- the same row (the db/048 apply precedent, NOT the #254 bug — a conflict here means the
-- SAME event twice).
CREATE OR REPLACE FUNCTION cairn_record_safety_overclaim_flag(
    p_ca bytea, p_patient uuid, p_emitted text, p_licensed text)
RETURNS void LANGUAGE sql AS $$
    INSERT INTO safety_overclaim_flag (content_address, patient_id, emitted_rung, licensed_rung)
    VALUES (p_ca, p_patient, p_emitted, p_licensed)
    ON CONFLICT (content_address) DO NOTHING;
$$;
```

In `db/005_submit.sql`, immediately after the existing `PERFORM cairn_check_safety_signal(b);`:

```sql
    -- #405 part 2 / ADR-0064: an emitted rung FINER than this chart's grade licenses is
    -- recorded, never refused (see db/049 for why the apply door does NOT do this).
    IF b -> 'safety' ->> 'rung' IS NOT NULL THEN
        DECLARE
            v_licensed text := cairn_safety_rung_for_rank(
                cairn_sensitivity_rank(
                    (SELECT grade FROM cairn_prospective_sensitivity(
                        (b ->> 'patient_id')::uuid, NULL))));
        BEGIN
            IF cairn_safety_rung_rank(b -> 'safety' ->> 'rung')
             < cairn_safety_rung_rank(v_licensed) THEN
                PERFORM cairn_record_safety_overclaim_flag(
                    v_ca, (b ->> 'patient_id')::uuid,
                    b -> 'safety' ->> 'rung', v_licensed);
            END IF;
        END;
    END IF;
```

> **Confirm the real signatures before writing this.** `cairn_prospective_sensitivity(uuid, uuid)` returns a row/table — match how db/049's own emission path calls it. `cairn_safety_rung_for_rank` and `cairn_safety_rung_rank` exist in db/049; use their actual argument types. Lower rank = finer rung (`precise` 0 … `existence` 20), so `emitted < licensed` is the overclaim direction — verify against db/049's ladder rather than trusting this comment.
>
> **This block must not be able to fail a clinical write** (ADR-0063 decision 8, stated categorically). If any lookup here can raise, wrap it in an exception block that swallows and continues — a flag that aborts a medication assert is precisely the defect this whole area exists to prevent.

- [ ] **Step 4: Run to verify it passes**

```bash
CARGO_TARGET_DIR=/tmp/cairn-authority \
CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" \
cargo test -p cairn-node --test safety_overclaim --test safety_emission --test safety_read \
  --test safety_doors --test safety_ladder --test safety_carried_class
```

Expected: all pass.

- [ ] **Step 5: Commit**

```bash
git add db/049_safety_projection.sql db/005_submit.sql crates/cairn-node/tests/
git commit -m "feat(#405): record a safety rung the chart's grade does not license

The door cannot refuse it — ADR-0060 forbids an advisory field cancelling a
medication assert — so it records, in ADR-0058's idiom. Local door only, and
deliberately breaking the precedent it copies: remotely, ADR-0063 decision 2
says an over-fine rung arrives routinely and honestly from older and
differently-custodial peers, so flagging there would accuse honest peers.

Bounds the silence, not the effect: read-time coarsening still applies."
```

---

### Task 6: SQL mirrors

**Files:**
- Modify: `db/tests/005_*_test.sql`, `db/tests/048_sensitivity_stream_test.sql`, `db/tests/049_safety_projection_test.sql`

**Interfaces:** Consumes everything from Tasks 1–5. Produces no new symbol.

- [ ] **Step 1: Write the mirror assertions**

Append to `db/tests/048_sensitivity_stream_test.sql`, following the file's existing `DO $$ … ASSERT … END $$;` style and its direct-`event_log`-seed idiom (the mirrors have no signing key, so they INSERT rows rather than calling `submit_event` — the header explains why):

```sql
-- ---------------------------------------------------------------------------
-- SQL mirror of crates/cairn-node/tests/claim_authority.rs. ADR-0064: only an
-- AUTHORITATIVE withdrawal participates in the standing set difference.
DO $$
DECLARE
    v_patient  uuid := gen_random_uuid();
    v_assert   uuid := gen_random_uuid();
    v_withdraw uuid := gen_random_uuid();
    v_ca_a     bytea := '\x1220'::bytea || digest('authority-mirror-assert', 'sha256');
    v_ca_w     bytea := '\x1220'::bytea || digest('authority-mirror-withdraw', 'sha256');
    n          int;
BEGIN
    INSERT INTO sensitivity_assertion
        (content_address, event_id, patient_id, subject_kind, subject_id, grade, source,
         hlc_wall, hlc_counter, node_origin)
    VALUES (v_ca_a, v_assert, v_patient, 'patient', v_patient, 'sequestered', 'human',
            10, 0, 'mirror');
    INSERT INTO sensitivity_withdrawal
        (content_address, event_id, withdraws, patient_id, rationale,
         hlc_wall, hlc_counter, node_origin)
    VALUES (v_ca_w, v_withdraw, v_ca_a, v_patient, 'strip', 20, 0, 'mirror');

    -- No event_log rows exist for either id, so neither R1 nor R2 can be satisfied.
    ASSERT cairn_claim_authority(v_withdraw, v_assert) = 'unverified',
        'a withdrawal with no resolvable human behind it is unverified';

    SELECT count(*) INTO n FROM cairn_sensitivity_standing(v_patient);
    ASSERT n = 1,
        'the assertion still STANDS: an unverified withdrawal does not lower (ADR-0064/#380)';

    -- And it is on the worklist, as `inert`.
    SELECT count(*) INTO n FROM sensitivity_withdrawal_worklist
     WHERE patient_id = v_patient AND reason = 'inert';
    ASSERT n = 1, 'an inert withdrawal is listed';

    DELETE FROM sensitivity_withdrawal WHERE content_address = v_ca_w;
    DELETE FROM sensitivity_assertion  WHERE content_address = v_ca_a;
END $$;
```

Append to `db/tests/049_safety_projection_test.sql`:

```sql
-- ADR-0064 / #405 part 2: the overclaim ledger exists, is keyed on content address, and
-- is idempotent on replay.
DO $$
DECLARE
    v_ca bytea := '\x1220'::bytea || digest('overclaim-mirror', 'sha256');
    v_p  uuid  := gen_random_uuid();
    n    int;
BEGIN
    PERFORM cairn_record_safety_overclaim_flag(v_ca, v_p, 'precise', 'existence');
    PERFORM cairn_record_safety_overclaim_flag(v_ca, v_p, 'precise', 'existence');
    SELECT count(*) INTO n FROM safety_overclaim_flag WHERE content_address = v_ca;
    ASSERT n = 1, 'the overclaim ledger is idempotent on replay (PK = content address)';
    DELETE FROM safety_overclaim_flag WHERE content_address = v_ca;
END $$;
```

Append to whichever `db/tests/005_*` mirror covers the submit door's function inventory (or `048` if there is none), the privilege pins:

```sql
DO $$
BEGIN
    ASSERT NOT has_function_privilege('public', 'cairn_claim_authority(uuid,uuid)', 'EXECUTE'),
        'cairn_claim_authority is SECURITY DEFINER — PUBLIC must not hold EXECUTE';
    ASSERT has_function_privilege('cairn_agent', 'cairn_claim_authority(uuid,uuid)', 'EXECUTE'),
        'cairn_agent reads the effective grade and therefore needs EXECUTE';
    ASSERT (SELECT prosecdef FROM pg_proc WHERE proname = 'cairn_claim_authority'),
        'SECURITY DEFINER is load-bearing: cairn_attestation_vouched is REVOKEd from PUBLIC';
END $$;
```

- [ ] **Step 2: Run the mirrors**

```bash
scripts/run-db-sql-tests.sh
```

Expected: all mirrors pass. This script drops, recreates and marks a throwaway `cairn_sqltest`, so it is **also the fresh-database check** the eager-binding trap needs — a `LANGUAGE sql` reference resolved at CREATE time only fails on a database built from scratch.

- [ ] **Step 3: Commit**

```bash
git add db/tests/
git commit -m "test(#380): SQL mirrors for the authority gate, worklist and overclaim ledger

run-db-sql-tests.sh builds cairn_sqltest from scratch, so this is also the
fresh-database check the LANGUAGE sql eager-binding trap needs: db/048's
reference to a db/005 function only fails on a database built from nothing."
```

---

### Task 7: Rust↔SQL lockstep, and the cairn-sync subset

**Files:**
- Create: `crates/cairn-event/tests/authority_lockstep.rs`
- Modify: `crates/cairn-sync/src/main.rs` (inline `#[cfg(test)]` module)
- Modify: `crates/cairn-event/src/contributor.rs` (comment only)

**Interfaces:** Consumes `cairn_claim_authority` and `classify_authorship_confidence`. Produces no new symbol.

- [ ] **Step 1: Write the lockstep test**

`classify_authorship_confidence` (`Attested`/`Unverified`/`Device`) and `cairn_claim_authority` (`attested`/`self`/`unverified`) ask overlapping questions and must not drift. They are **not** identical — R2 has no Rust counterpart, and `Device` has no SQL counterpart — so pin the overlap precisely:

```rust
//! ADR-0064: cairn_claim_authority (db/005) is the SQL side of the same question
//! classify_authorship_confidence answers in Rust. They are not identical — R2 ('self')
//! has no Rust counterpart and `Device` has no SQL one — but where they overlap they must
//! agree, or a display grade and an enforcement grade disagree about one event.
mod common;

#[tokio::test]
async fn attested_in_rust_is_attested_in_sql_and_unverified_is_unverified() {
    // For each fixture: build a contributor set + signer + attester, submit it, then
    // compare classify_authorship_confidence's verdict against
    // cairn_claim_authority(event_id, NULL).
    //
    //   Attested   <-> 'attested'
    //   Unverified <-> 'unverified'
    //   Device     <-> 'unverified'   (no bearing contributor is not authority)
    //
    // Build the fixtures with the SAME helpers the db-gated suites use so the two sides
    // see byte-identical contributor sets; a hand-written JSON literal here would let the
    // Rust side pass on a shape the door never produces.
}
```

Fill the body using `crates/cairn-event/tests/property_contributor.rs` as the model for constructing contributor sets, and Task 1's `authority` helper for the SQL side. Cover all three Rust variants.

- [ ] **Step 2: Add the subset-drive test**

In `crates/cairn-sync/src/main.rs`'s inline test module, beside `db036_adds_seq_columns`:

```rust
/// ADR-0064: db/048 references cairn_claim_authority, which lives in db/005. Both are in
/// this subset — but `LANGUAGE sql` resolves references at CREATE time, so a subset that
/// carried db/048 without db/005 would fail to create cairn_sensitivity_standing and take
/// clinical sync down entirely. Drive it, do not merely load it (#386's lesson).
#[test]
fn db048_authority_gate_resolves_in_the_sync_subset() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let mut c = locked_client(&base); // loads the whole SCHEMA subset
    let verdict: String = c
        .query_one(
            "SELECT cairn_claim_authority(gen_random_uuid(), gen_random_uuid())",
            &[],
        )
        .unwrap()
        .get(0);
    assert_eq!(verdict, "unverified", "the predicate must RUN on the sync subset");
    // And the seam that calls it must be callable here too.
    let n: i64 = c
        .query_one(
            "SELECT count(*) FROM cairn_sensitivity_standing(gen_random_uuid())",
            &[],
        )
        .unwrap()
        .get(0);
    assert_eq!(n, 0);
}
```

- [ ] **Step 3: Update the contributor.rs comment**

The existing note says the Rust grade "has no production consumer today". It now has a SQL twin. Amend it to name `cairn_claim_authority` and state the lockstep obligation and the test that holds it. Do **not** claim #245 is closed — #245's *display* half is still open.

- [ ] **Step 4: Run**

```bash
CARGO_TARGET_DIR=/tmp/cairn-authority \
CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" \
cargo test -p cairn-event --test authority_lockstep
CARGO_TARGET_DIR=/tmp/cairn-authority \
CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" \
cargo test -p cairn-sync db048_authority_gate
```

Expected: both pass.

- [ ] **Step 5: Commit**

```bash
git add crates/cairn-event/ crates/cairn-sync/src/main.rs
git commit -m "test(#245): pin the Rust<->SQL authority lockstep and the sync subset

classify_authorship_confidence finally has a SQL twin, so the lockstep its doc
comment has always asserted is now a test. The subset test DRIVES the predicate
rather than merely loading db/048 — #386's lesson, applied on the first try."
```

---

### Task 8: The full gate

**Files:** none — this task runs the gate and fixes whatever it finds.

- [ ] **Step 1: Run the whole workspace**

```bash
CARGO_TARGET_DIR=/tmp/cairn-authority scripts/run-db-gated-tests.sh
```

This runs the SQL mirrors *and* the full workspace with all three connection strings baked in.

> **Do not pipe to `tail`** — that masks cargo's exit code. Read the whole output.
>
> A killed binary exits 101 with **zero** `test result: FAILED` lines; if you see 101 with no failures listed, that is the macOS `_dyld_start` loader stall, not a test failure. Diagnose with `sample <pid>`, `kill -9`, and retry — a clean full sweep is achievable.
>
> `cargo test --workspace` exceeds the 10-minute Bash cap. Run it from the controlling session, not a subagent.

- [ ] **Step 2: fmt, clippy, deny**

```bash
cargo fmt --all -- --check
CARGO_TARGET_DIR=/tmp/cairn-authority cargo clippy --workspace --all-targets -- -D warnings
cargo deny check
```

- [ ] **Step 3: Fix everything red, then commit**

```bash
git add -A
git commit -m "chore: workspace gate green for the claim-authority slice"
```

---

### Task 9: ADR-0064 and the spec prose

**Files:**
- Create: `docs/spec/decisions/0064-admit-the-claim-withhold-the-power.md`
- Modify: `docs/spec/decisions/README.md` (index row)
- Modify: `docs/spec/identity.md` §5.9
- Modify: `docs/spec/index.md` (spec version → v0.66)

- [ ] **Step 1: Write ADR-0064**

Follow ADR-0063's structure exactly (Status / Date / Derives from / Applies / Canonical spec home / Context / Decision / Rejected alternatives / Known limitations / Consequences / The bet / How we would know the bet fails / First instance). The seven decisions are §2 of the spec. Carry across, in the ADR's own voice:

- The three-surface table from spec §1 and the *admit the claim, withhold the power* framing.
- **Rejected alternatives:** trust-set origin as an authority fact (any admitted peer satisfies it — #380's own shape one layer up — *and* `trust_peer` is absent from the cairn-sync subset, so it would have taken clinical sync down); prior standing on the chart (fails the locum, the night-cover registrar and the receiving ED, and is replication-relative — kept as *salience* only); a deployment-configurable threshold; a per-dial authority check (the #404/#399 drift shape); an apply-time flag ledger for withdrawals.
- **Known limitations, stated plainly:** it buys accountability, not authorization — *the record is the control and the gate is only the forcing function*; the second axis of node-relativity, and the widened `given equal custody and equal actor knowledge` test qualifier; whichever arrival-order finding Task 3 established.
- **The finding for #376:** a custody dial derived from the effective grade is only as strong as its most-custodial holder; an explicit custody act is not. Recorded as an input to part C, explicitly **not** a decision taken here.

- [ ] **Step 2: Update §5.9 and the ADR index**

Add the authority rule to identity.md §5.9 as canonical prose, and the ADR-0064 row to `decisions/README.md`. Bump the spec version in `docs/spec/index.md` to **v0.66**.

- [ ] **Step 3: Build the docs**

```bash
uv run --with-requirements docs/requirements.txt -- mkdocs build
```

Expected: no broken-link warnings for the new file. Never commit `site/`.

- [ ] **Step 4: Commit**

```bash
git add docs/spec/
git commit -m "docs(ADR-0064): admit the claim, withhold the power

Names the principle the project already acts on in two places — ADR-0056's
'power is earned' and #231's 'withhold the key, never the bytes' — and gives it
one mechanism. Spec v0.66."
```

---

### Task 10: PR, HANDOVER currency, follow-ons

**Files:**
- Modify: `docs/HANDOVER.md`

- [ ] **Step 1: Open the PR**

```bash
git push -u origin design/claim-authority-apply-door
gh pr create --title "feat(#380): claim authority at the apply door (ADR-0064)" --body "$(cat <<'EOF'
Closes #380. Discharges #405 part 2. Gives #245 its SQL mirror.

Three findings — #231 (closed), #380 and #405 part 2 — are one defect: the
floor established that a claim was well-formed and treated well-formedness as
authority. Part C (#376) would have been the fourth, and the first where being
wrong costs a DEK.

Authority is a human actor this node can hold responsible, by either of two
routes. It gates effect and never admission, only in the withholding direction,
so nothing is refused and nothing forks. One predicate in db/005, consulted at
one site — the NOT EXISTS in cairn_sensitivity_standing — so display, emission
and part C's future custody dial inherit it structurally.

Design: docs/superpowers/specs/2026-08-15-claim-authority-at-the-apply-door-design.md

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

- [ ] **Step 2: Bring HANDOVER.md current — on this branch, before review**

Two things in `⇒ NEXT` are now wrong:

- *"Part C … **is the next thing to build, and nothing blocks it**"* — it never mentioned #380, and #380 is now closed by a design part C must be built on top of.
- The §5.9 A/B/C/D list needs the authority rule between B and C.

Rewrite that block so a fresh session reads: parts A and B shipped; **ADR-0064 is the floor part C keys on**; part C's remaining open decision is the dial question, sharpened by the finding in the spec's §8. Keep the file **under 500 lines** (#368) — cut older material to make room rather than appending.

- [ ] **Step 3: File the follow-ons**

- Comment on **#376** with the spec §8 finding (derived-from-grade vs explicit custody act). Do not open a new issue.
- Comment on **#245** narrowing it to the display half.
- Comment on **#405** confirming part 2 is closed here and part 1 remains open.

- [ ] **Step 4: Commit and push**

```bash
git add docs/HANDOVER.md
git commit -m "docs: HANDOVER — part C now keys on ADR-0064, and #380 is closed

The 'nothing blocks it' line predated #380 and is the stalest thing in the
file; part C's remaining open decision is the dial question."
git push
```

- [ ] **Step 5: Request review**

Use `superpowers:requesting-code-review`. Then `/code-review high`. Merge only after review passes and CI is green.

---

## Self-Review

**Spec coverage.** §3 (the rule) → Task 1. §4 (home, arity, SECURITY DEFINER) → Task 1. §5 (the seam) → Task 2. §6 (the worklist, both rows) → Task 4. §7 (#405 part 2) → Task 5. §8 (part C forcing case, the #376 finding) → Task 9 step 1 + Task 10 step 3. §9 (declared limitations) → Task 9 step 1. §10 (paper-parity) → carried by the spec; no code owed, and the §10 budget ("one query, no raw SQL") is satisfied by Task 4's view. §11 items 1–14 → Tasks 1–7 (item 5's mutation check is Task 2 step 5; item 10a is Task 2's `the_read_path_works_as_cairn_agent`). §12 (files, traps) → the File Structure table and the trap callouts. §13 (follow-ons) → Task 10 step 3.

**Placeholder scan.** Four steps deliberately require the executor to read existing code rather than trust this plan: the exact `submit_event` arity and `body_from_spec` shape (Task 1), the `node_origin` comparison (Task 4), the `cairn_prospective_sensitivity` / rung-rank signatures (Task 5), and the lockstep test body (Task 7). Each says exactly what to read and what the answer must satisfy — these are verification instructions, not TBDs, and inventing signatures here would be worse than sending the executor to the source. Task 3's second test carries an explicit *if this path is refused, do X instead* branch with both outcomes specified.

**Type consistency.** `cairn_claim_authority(uuid, uuid) → text` returns exactly `'attested'` / `'self'` / `'unverified'` in Task 1, and Tasks 2, 4, 6 and 7 all compare against those literals. `cairn_sensitivity_standing(uuid)`'s signature and four return columns are unchanged. The view's `reason` column yields exactly `'inert'` / `'stranger-attested'` in Tasks 4 and 6. `safety_overclaim_flag`'s columns match between Task 5's DDL, its test and Task 6's mirror. `assert_grade` is introduced in Task 1 and refactored to `common::assert_chart_grade` in Task 4 — Task 4 says so explicitly.

---

**Plan complete and saved to `docs/superpowers/plans/2026-08-15-claim-authority-at-the-apply-door.md`.**
