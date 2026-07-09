# Actor `actor_id` Collision Floor — Implementation Plan (issue #152)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `enroll_actor` fail closed when the computed `actor_id` already binds a **distinct** signing key anywhere in `actor_event` history, closing the silent identity-merge hole (#152) on the trust-anchor floor.

**Architecture:** One in-DB floor change in `db/004_actors.sql`: a pure `STABLE` predicate `cairn_actor_id_key_conflict(actor_id, key)` plus a guard in `enroll_actor` that raises before inserting. Coverage is a CI-gated Rust integration test mirroring the #99 `suppression_owner_gate.rs` pattern, with a parallel SQL assertion block in the canonical floor-test file. A short ADR (0044) refines ADR-0011/0029; one spec sentence in security §7.5 + a spec-version bump.

**Tech Stack:** PostgreSQL 18 + `cairn_pgx` (pgrx) extension; PL/pgSQL; Rust `tokio-postgres` integration tests; mkdocs (docs build check).

## Global Constraints

- **AGPL-3.0**; no new dependencies (none needed).
- **TDD** — failing test first, then the floor change (load-bearing on the §9 safety-critical surface).
- **`db/004` edited in place** — pre-clinical posture, the #99 hardening pattern (no real clinical data / legacy nodes yet). No new migration file.
- **`actor_id` derivation is unchanged** — the key is NOT hashed in (ADR-0011 §5 rotate-key must preserve `actor_id`). This is an enforcement gate only.
- **Reject predicate:** distinct signing key for the same `actor_id`, **whole history** (incl. revoked rows). Idempotent same-`(pinned,key)` re-enroll passes.
- **Single door:** only `enroll_actor` (no remote-apply door for actor enrollment exists yet). Record the forward caveat for the future actor-sync apply door.
- **Test DB:** `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test"` (PG18 + `cairn_pgx` on the Mac :5532 cluster). DB-gated tests self-serialize cluster-wide via an advisory lock.
- **CI gate is `cargo test --workspace`**, not `db/tests/*.sql`. The Rust integration test is the load-bearing coverage.
- **House rules:** prefer pure/reusable functions; inline docs legible to a junior; keep files focused; all tests green before commit.

---

## Task 1: The floor change + CI-gated Rust test (TDD)

**Files:**
- Create: `crates/cairn-node/tests/actor_enroll_collision.rs`
- Modify: `db/004_actors.sql` (add `cairn_actor_id_key_conflict`; guard inside `enroll_actor`; comments)

**Interfaces:**
- Consumes: `enroll_actor(p_kind TEXT, p_pinned JSONB, p_key TEXT) RETURNS BYTEA` (existing); `cairn_actor_id(JsonB)` (existing pgrx); `cairn_node::db::connect_and_load_schema(&str) -> Client`; `cairn_event::generate_key() -> Result<(SigningKey, String)>` (returns `(sk, kid_hex)`).
- Produces: `cairn_actor_id_key_conflict(p_actor_id BYTEA, p_key TEXT) RETURNS BOOLEAN` (STABLE) — TRUE iff some `actor_event` row has `actor_id = p_actor_id` and `signing_key_id IS DISTINCT FROM p_key`.

- [ ] **Step 1: Write the failing Rust integration test**

Create `crates/cairn-node/tests/actor_enroll_collision.rs`:

```rust
//! ADR-0044 / issue #152 — enroll fail-closed on actor_id collision. actor_id is the
//! content-address of the PINNED set only (not the signing key, which must stay mutable
//! across rotate-key). Two DISTINCT keys enrolled with an IDENTICAL pinned set collide
//! into one actor_id; actor_current's `DISTINCT ON (actor_id)` then silently drops the
//! earlier key (a silent identity merge — principle 2). enroll_actor now refuses it.
//! Real Postgres, gated on $CAIRN_TEST_PG, serialized cluster-wide.
use cairn_event::generate_key;
use cairn_node::db;
use tokio_postgres::Client;

fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

/// The RAISE message for a failed statement (Display renders only "db error"; the text
/// lives in the DbError payload — project convention, see suppression_owner_gate.rs).
fn db_msg(e: &tokio_postgres::Error) -> String {
    e.as_db_error()
        .map(|d| d.message().to_string())
        .unwrap_or_else(|| e.to_string())
}

/// Fresh actor registry for each test (isolation; the whole-history check would
/// otherwise see prior tests' committed rows).
async fn reset(c: &Client) {
    c.batch_execute("TRUNCATE event_log, actor_event, patient_chart CASCADE")
        .await
        .unwrap();
}

const ENROLL_HUMAN: &str = "SELECT enroll_actor('human', '{\"role\":\"clinician\"}', $1)";

#[tokio::test]
async fn distinct_key_same_pinned_is_refused() {
    let Some(cs) = cs() else { return };
    let c = db::connect_and_load_schema(&cs).await.unwrap();
    reset(&c).await;
    let (_sk1, kid1) = generate_key().unwrap();
    let (_sk2, kid2) = generate_key().unwrap();
    // First human enrolls fine.
    c.execute(ENROLL_HUMAN, &[&kid1]).await.unwrap();
    // Second human, IDENTICAL pinned set, DIFFERENT key → same actor_id → refused.
    let err = c
        .execute(ENROLL_HUMAN, &[&kid2])
        .await
        .expect_err("colliding enroll must be refused");
    assert!(
        db_msg(&err).contains("already enrolled with a different signing key"),
        "expected the collision RAISE, got: {}",
        db_msg(&err)
    );
    // The first key remains the sole current identity for that actor_id.
    let n: i64 = c
        .query_one(
            "SELECT count(*) FROM actor_current WHERE signing_key_id = $1",
            &[&kid1],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(n, 1, "the first-enrolled key must survive");
}

#[tokio::test]
async fn idempotent_same_key_re_enroll_is_allowed() {
    let Some(cs) = cs() else { return };
    let c = db::connect_and_load_schema(&cs).await.unwrap();
    reset(&c).await;
    let (_sk, kid) = generate_key().unwrap();
    c.execute(ENROLL_HUMAN, &[&kid]).await.unwrap();
    // Same (pinned, key) again → allowed (re-runnable provisioning, matcher per-epoch re-enroll).
    c.execute(ENROLL_HUMAN, &[&kid])
        .await
        .expect("idempotent same-key re-enroll must be allowed");
}

#[tokio::test]
async fn distinct_pinned_sets_do_not_collide() {
    let Some(cs) = cs() else { return };
    let c = db::connect_and_load_schema(&cs).await.unwrap();
    reset(&c).await;
    let (_sk1, kid1) = generate_key().unwrap();
    let (_sk2, kid2) = generate_key().unwrap();
    c.execute(
        "SELECT enroll_actor('human', '{\"role\":\"clinician\",\"actor\":\"A\"}', $1)",
        &[&kid1],
    )
    .await
    .unwrap();
    // Different pinned set → different actor_id → no false positive.
    c.execute(
        "SELECT enroll_actor('human', '{\"role\":\"clinician\",\"actor\":\"B\"}', $1)",
        &[&kid2],
    )
    .await
    .expect("distinct pinned sets must both enroll");
    let n: i64 = c
        .query_one("SELECT count(*) FROM actor_current", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(n, 2, "two distinct actors must be current");
}

#[tokio::test]
async fn actor_id_is_immortal_after_revoke() {
    let Some(cs) = cs() else { return };
    let c = db::connect_and_load_schema(&cs).await.unwrap();
    reset(&c).await;
    let (_sk1, kid1) = generate_key().unwrap();
    let (_sk2, kid2) = generate_key().unwrap();
    // Enroll, capture the actor_id, then revoke it.
    let aid: Vec<u8> = c
        .query_one(ENROLL_HUMAN, &[&kid1])
        .await
        .unwrap()
        .get(0);
    c.execute(
        "INSERT INTO actor_event (actor_id, op) VALUES ($1, 'revoke')",
        &[&aid],
    )
    .await
    .unwrap();
    // A DIFFERENT key re-using that (now-revoked) actor_id is STILL refused — the
    // whole-history check enforces principle-2 immortality (no post-revoke reuse).
    let err = c
        .execute(ENROLL_HUMAN, &[&kid2])
        .await
        .expect_err("post-revoke reuse by a different key must be refused");
    assert!(
        db_msg(&err).contains("already enrolled with a different signing key"),
        "expected the collision RAISE, got: {}",
        db_msg(&err)
    );
}
```

- [ ] **Step 2: Run the tests to verify they FAIL**

Run: `cd /Users/hherb/src/cairn-ehr && CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test -p cairn-node --test actor_enroll_collision -- --nocapture`

Expected: `distinct_key_same_pinned_is_refused` and `actor_id_is_immortal_after_revoke` FAIL (the second enroll currently *succeeds* — no guard yet — so `expect_err` panics). `idempotent_same_key_re_enroll_is_allowed` and `distinct_pinned_sets_do_not_collide` PASS already (they assert current-good behavior, guarding against a too-broad fix).

- [ ] **Step 3: Add the floor guard in `db/004_actors.sql`**

Insert the pure predicate immediately **before** the `enroll_actor` definition (after the `actor_current` VIEW, around line 70):

```sql
-- issue #152: an actor_id is the content-address of the PINNED set only, never the
-- signing key (the key must stay mutable across a future rotate-key, ADR-0011 §5). So
-- two DIFFERENT signing keys enrolled with an IDENTICAL pinned set compute the SAME
-- actor_id, and actor_current's `DISTINCT ON (actor_id)` silently keeps only the
-- latest key — a silent identity merge on the trust anchor (principle 2). This pure
-- predicate is TRUE iff some existing actor_event row already binds this actor_id to a
-- DIFFERENT key. Whole history (incl. revoked rows): an actor_id is immortal and is
-- never reusable by a different key, even after revoke. STABLE + a small pure function
-- so it is independently testable and reusable at the future actor-sync apply door.
CREATE OR REPLACE FUNCTION cairn_actor_id_key_conflict(p_actor_id BYTEA, p_key TEXT)
RETURNS BOOLEAN LANGUAGE sql STABLE AS $$
    SELECT EXISTS (
        SELECT 1 FROM actor_event
        WHERE actor_id = p_actor_id
          AND signing_key_id IS DISTINCT FROM p_key
    );
$$;
```

Then modify `enroll_actor` to guard before the INSERT:

```sql
-- Enroll an actor; its identity is derived in-DB from the pinned set (cairn_pgx),
-- so "identity = hash of what is pinned" is enforced, not asserted. Because the
-- signing key is deliberately NOT part of that hash (rotate-key preserves actor_id,
-- ADR-0011 §5), enroll must fail CLOSED if the computed actor_id already binds a
-- DIFFERENT key — otherwise two distinct actors silently merge (issue #152). A HUMAN
-- actor therefore needs a person-distinguishing determinant in its pinned set (a handle
-- / registration id); the minimal `{"role":...}` collides across people. That field is
-- left to the future enrollment surface (ADR-0011 keeps pinned-set CONTENTS as policy);
-- this floor only makes a forgotten determinant LOUD on the second enroll.
-- SINGLE DOOR: there is no remote-apply door for actor enrollment yet (INSERT INTO
-- actor_event lives only here). When actor-event sync lands (ADR-0011 §4), mirror this
-- check at that apply door — same shape as the #154 apply-door caveat.
CREATE OR REPLACE FUNCTION enroll_actor(p_kind TEXT, p_pinned JSONB, p_key TEXT)
RETURNS BYTEA LANGUAGE plpgsql AS $$
DECLARE aid BYTEA;
BEGIN
    aid := cairn_actor_id(p_pinned);
    IF cairn_actor_id_key_conflict(aid, p_key) THEN
        RAISE EXCEPTION
            'enroll_actor: actor_id % is already enrolled with a different signing key (silent identity-merge refused, issue #152)',
            encode(aid, 'hex')
        USING HINT =
            'Give this actor a distinguishing pinned determinant (e.g. a person handle / registration id), or use rotate-key to add a key to the SAME actor.';
    END IF;
    INSERT INTO actor_event (actor_id, op, kind, pinned, signing_key_id)
    VALUES (aid, 'enroll', p_kind, p_pinned, p_key);
    RETURN aid;
END;
$$;
```

Add `EXECUTE` revocation for the new helper next to the existing REVOKE block at the file end (defense in depth — it reads the trust-anchor table; keep it off `PUBLIC`, though STABLE + read-only makes it low-risk):

```sql
REVOKE EXECUTE ON FUNCTION cairn_actor_id_key_conflict(bytea, text) FROM PUBLIC;
```

- [ ] **Step 4: Run the tests to verify they PASS**

Run: `cd /Users/hherb/src/cairn-ehr && CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test -p cairn-node --test actor_enroll_collision -- --nocapture`

Expected: all four tests PASS.

- [ ] **Step 5: Guard against regressions — run the full cairn-node DB-gated suite**

Run: `cd /Users/hherb/src/cairn-ehr && CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test -p cairn-node`

Expected: all pass. (No existing test enrolls two identical-pinned humans; `setup()` truncates `actor_event` per test, so state is isolated.) If any test fails because it enrolls a colliding pair, that test is itself exercising the #152 bug — fix it by giving each human a distinct pinned determinant (as `suppression_owner_gate.rs` already does), and note it.

- [ ] **Step 6: Commit**

```bash
cd /Users/hherb/src/cairn-ehr
git add db/004_actors.sql crates/cairn-node/tests/actor_enroll_collision.rs
git commit -m "fix(floor): enroll fail-closed on actor_id collision with a distinct key (#152)

cairn_actor_id hashes only the pinned set, so two distinct signing keys with
an identical pinned set collide into one actor_id and actor_current silently
drops the earlier key (a silent identity-merge on the trust anchor, principle 2).
New pure STABLE cairn_actor_id_key_conflict predicate + an enroll_actor guard
refuse a distinct-key collision across the whole actor_event history (immortal
even after revoke); idempotent same-key re-enroll still passes. Single door
(no actor-sync apply door yet). CI-gated Rust integration test.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 2: SQL floor-test mirror + local doc parity

**Files:**
- Modify: `db/tests/004_actors_test.sql` (append a collision-refusal assertion block)

**Interfaces:**
- Consumes: `enroll_actor(...)`, `cairn_actor_id_key_conflict(bytea, text)` from Task 1.
- Produces: nothing (test-only).

- [ ] **Step 1: Add the SQL assertion block**

Append inside the existing `BEGIN … ROLLBACK` transaction of `db/tests/004_actors_test.sql`, before the final `ROLLBACK;`:

```sql
-- issue #152: enroll fails CLOSED when the same pinned set (→ same actor_id) is
-- enrolled with a DIFFERENT signing key. The minimal human pinned set is the classic
-- collision; a distinguishing determinant is what keeps two clinicians distinct.
DO $$
DECLARE aid1 bytea; aid2 bytea;
BEGIN
    aid1 := enroll_actor('human', '{"role":"clinician"}', 'keyAAAA');
    -- Idempotent same-key re-enroll is allowed (re-runnable provisioning).
    aid2 := enroll_actor('human', '{"role":"clinician"}', 'keyAAAA');
    IF aid1 IS DISTINCT FROM aid2 THEN
        RAISE EXCEPTION 'collision test FAILED: same (pinned,key) should map to one actor_id';
    END IF;
    -- Different key, identical pinned set → must raise.
    BEGIN
        PERFORM enroll_actor('human', '{"role":"clinician"}', 'keyBBBB');
        RAISE EXCEPTION 'collision test FAILED: distinct-key collision was NOT refused';
    EXCEPTION WHEN others THEN
        IF SQLERRM LIKE '%different signing key%' THEN
            RAISE NOTICE 'actor_id collision refusal OK';
        ELSE RAISE; END IF;
    END;
    -- The pure predicate agrees.
    IF NOT cairn_actor_id_key_conflict(aid1, 'keyBBBB') THEN
        RAISE EXCEPTION 'predicate FAILED: keyBBBB should conflict with aid1';
    END IF;
    IF cairn_actor_id_key_conflict(aid1, 'keyAAAA') THEN
        RAISE EXCEPTION 'predicate FAILED: keyAAAA is the SAME key, no conflict';
    END IF;
END $$;
```

- [ ] **Step 2: Run the SQL floor-test file against the cluster**

Run:
```bash
cd /Users/hherb/src/cairn-ehr
psql "host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" -v ON_ERROR_STOP=1 \
  -f db/004_actors.sql -f db/tests/004_actors_test.sql
```

Expected: output includes `NOTICE:  actor_id collision refusal OK` and the pre-existing `monotonic tiebreak OK` / `append-only OK`, with no ERROR. (The whole run is inside a transaction that ROLLBACKs.)

- [ ] **Step 3: Commit**

```bash
cd /Users/hherb/src/cairn-ehr
git add db/tests/004_actors_test.sql
git commit -m "test(floor): SQL mirror for the actor_id collision refusal (#152)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 3: ADR-0044 + spec §7.5 sentence + version bump

**Files:**
- Create: `docs/spec/decisions/0044-enroll-fail-closed-on-actor-id-collision.md`
- Modify: `docs/spec/security.md` (§7.5, one sentence + the human-determinant note)
- Modify: `docs/spec/index.md` (spec version bump)
- Modify: `docs/spec/decisions/README.md` (ADR index row, if it enumerates ADRs)

**Interfaces:** none (docs only).

- [ ] **Step 1: Write ADR-0044**

Create `docs/spec/decisions/0044-enroll-fail-closed-on-actor-id-collision.md`:

```markdown
# ADR-0044 — Enroll fails closed on `actor_id` collision; human actors carry a person-distinguishing determinant

- **Status:** Accepted (refines [ADR-0011](0011-actor-registry-version-pinning-and-key-custody.md), [ADR-0029](0029-skill-epoch-as-pinned-actor-determinant.md))
- **Date:** 2026-07-09

## Context

[ADR-0011](0011-actor-registry-version-pinning-and-key-custody.md) fixed an actor's identity as the
content-address of its **pinned determinant set**, and [ADR-0029](0029-skill-epoch-as-pinned-actor-determinant.md)
named the skill-epoch / served-model digest as pinned determinants. In `db/004_actors.sql`,
`enroll_actor` derives `actor_id = cairn_actor_id(pinned)` accordingly. For an **AI agent** the pinned set is
rich (vendor, model, version, config, skill-epoch, deploying node), so two distinct agents naturally get
distinct `actor_id`s. But ADR-0011 explicitly gives a **human** *no behavioural-config dimension* — and did
not say what else distinguishes two humans. The minimal human pinned set (e.g. `{"role":"clinician"}`) is
therefore **identical across people**, so two different clinicians compute the **same** `actor_id`.

`actor_current` is `DISTINCT ON (actor_id) … ORDER BY (recorded_at, seq) DESC`. When two keys share an
`actor_id`, the later enrollment's key wins and the earlier key **silently drops out of `actor_current`** —
the two humans are merged into one identity, and the survivor's key can author as the merged actor, with **no
error**. This is a silent identity-merge on the trust-anchor floor: a direct violation of principle 2
(*identity is a claim; never merge — always link; never erase — always overlay*). It surfaced during the
ADR-0043 owner-gate work (the tests hit it and worked around it with distinct pinned tags) and was then
confirmed against `db/004`.

The tempting fix — fold the signing key into `actor_id` — is **wrong**: ADR-0011 §5 mandates `rotate-key`,
under which an actor keeps the **same** `actor_id` across a key change (old publics retained for historical
verification). `actor_id` must stay **stable across key rotation**, so the key cannot be one of its
determinants. The distinguishing determinant for a human must be something *other* than the key and stable
across rotation.

## Decision

1. **`enroll_actor` fails closed on an `actor_id` collision with a distinct key.** Before inserting, it
   refuses if any existing `actor_event` row already binds the computed `actor_id` to a **different**
   `signing_key_id` — checked across the **whole history**, including revoked rows. An `actor_id` is
   **immortal**: it is never reusable by a different key, even after `revoke` (principle 2, the immortal-UUID
   rule now applied to actors). The refusal is legible and names the remedy. Idempotent re-enroll with the
   **same** key passes (re-runnable provisioning; the per-epoch matcher-actor re-enroll).

2. **A human actor's pinned set carries a person-distinguishing determinant.** A handle / registration id
   (whatever the future §7.5 enrollment surface adopts) keeps two clinicians' `actor_id`s genuinely distinct,
   exactly as two AI agents differ by their rich pinned sets. Cairn does **not** hard-code *which* field
   (ADR-0011 keeps pinned-set **contents** as policy); the floor rejection in (1) makes a forgotten
   determinant **loud** on the second enroll rather than silently merging.

3. **This is enforcement of ADR-0011's existing intent, not a new determinant.** The `actor_id` derivation is
   unchanged; (1) closes an enforcement gap the same way [ADR-0043](0043-suppression-self-only-disagreement-is-additive.md)
   closed the suppression owner-gate gap. It is scoped to the one implemented write door (`enroll_actor`);
   there is no remote-apply door for actor enrollment yet. When actor-event sync lands (ADR-0011 §4), the
   same check must be mirrored at that apply door — the analogue of the [#154](https://github.com/cairn-ehr/cairn-ehr/issues/154)
   apply-door caveat.

**Canonical home:** [security §7.5](../security.md#75-the-actor-registry-enrollment-version-pinning-and-key-custody).

## Consequences

- **Easier:** two real clinicians (or two agents) can never silently merge on the trust anchor; the failure is
  loud, auditable, and repairable by adding a distinguishing determinant — no data loss.
- **Harder:** the future human-enrollment surface must supply a person-distinguishing determinant (a small,
  cheap-at-provisioning obligation). A second write door (actor-event sync) must carry the same check.
- **The bet:** the collision check plus a person-distinguishing human determinant is the whole fix — no change
  to `actor_id` derivation, `actor_current`, or `rotate-key` (which remains the sanctioned same-actor / new-key
  path). We would revisit if a legitimate flow needs two distinct actors to share one `actor_id` (none is known).
- **No new founding principle.** This is principle 2 applied to the actor registry — as ADR-0011 already
  established — now enforced at the enroll door.
```

- [ ] **Step 2: Add the spec sentence in security §7.5**

Read the §7.5 region first to match prose style:

Run: `grep -n "7.5\|actor_id\|pinned\|enroll" docs/spec/security.md | head`

Then add (near where §7.5 describes enrollment / the pinned set) a sentence such as:

```markdown
Because `actor_id` is the content-address of the pinned set alone (never the signing key, which stays mutable
across `rotate-key`), enrollment **fails closed** if the computed `actor_id` already binds a different signing
key — two actors can never silently merge (principle 2, [ADR-0044](decisions/0044-enroll-fail-closed-on-actor-id-collision.md)).
A **human** actor therefore carries a person-distinguishing determinant (a handle / registration id) in its
pinned set, just as an AI agent is distinguished by its richer determinant set.
```

- [ ] **Step 3: Bump the spec version in index.md**

Run: `grep -n -i "version" docs/spec/index.md | head`

Bump the stated spec version by one minor (v0.44 → v0.45), matching the prior bump convention (ADR-0043 was v0.44). Edit the exact version string found.

- [ ] **Step 4: Add the ADR index row (if the decisions README tabulates ADRs)**

Run: `grep -n "0043\|0042" docs/spec/decisions/README.md`

If a table/list row per ADR exists, add a 0044 row mirroring the 0043 row's format:

```markdown
| [0044](0044-enroll-fail-closed-on-actor-id-collision.md) | Enroll fails closed on `actor_id` collision; human actors carry a person-distinguishing determinant | §7.5 (refines 0011/0029) |
```

- [ ] **Step 5: Build the docs to verify no broken links / valid markdown**

Run: `cd /Users/hherb/src/cairn-ehr && uv run --with-requirements docs/requirements.txt -- mkdocs build 2>&1 | tail -20`

Expected: build succeeds; no `WARNING` about the new ADR file's links.

- [ ] **Step 6: Commit**

```bash
cd /Users/hherb/src/cairn-ehr
git add docs/spec/decisions/0044-enroll-fail-closed-on-actor-id-collision.md docs/spec/security.md docs/spec/index.md docs/spec/decisions/README.md
git commit -m "docs(spec): ADR-0044 enroll fail-closed on actor_id collision; §7.5 + version bump (#152)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 4: Final verification, HANDOVER/ROADMAP, PR

**Files:**
- Modify: `docs/HANDOVER.md`, `docs/ROADMAP.md`

**Interfaces:** none.

- [ ] **Step 1: Full workspace verification (the CI gate, locally)**

Run:
```bash
cd /Users/hherb/src/cairn-ehr
cargo fmt --all -- --check
cargo clippy --workspace --tests -- -D warnings
CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test --workspace
```

Expected: fmt clean, clippy clean, all tests pass. Fix anything that fails before proceeding (rustfmt the new test file if `--check` complains).

- [ ] **Step 2: Update HANDOVER.md and ROADMAP.md**

Add a concise "This session (2026-07-09) — #152 actor_id collision floor" block at the top of `docs/HANDOVER.md` (mirror the #99 block's shape: what changed, the one helper, single-door + the forward caveat, ADR-0044, spec bump, tests, follow-ups). Mark [#152](https://github.com/cairn-ehr/cairn-ehr/issues/152) as fixed. Keep it brief; prune an older block if the file exceeds ~500 lines. Update the top `Spec/ADRs: v0.44` → `v0.45` line. In `docs/ROADMAP.md`, note the enroll collision floor under Phase 3/4 (identity/actor floor) if a matching line exists.

- [ ] **Step 3: Commit the docs currency**

```bash
cd /Users/hherb/src/cairn-ehr
git add docs/HANDOVER.md docs/ROADMAP.md
git commit -m "docs: HANDOVER/ROADMAP — actor_id collision floor done (ADR-0044, #152)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

- [ ] **Step 4: Push and open the PR**

```bash
cd /Users/hherb/src/cairn-ehr
git push -u origin fix/actor-id-collision-floor-152
gh pr create --base main --title "fix(floor): enroll fail-closed on actor_id collision (#152)" --body "$(cat <<'EOF'
## Summary
Closes the silent identity-merge hole on the actor trust-anchor floor (#152): two distinct signing keys enrolled with an identical pinned set collided into one `actor_id`, and `actor_current`'s `DISTINCT ON (actor_id)` silently dropped the earlier key.

`enroll_actor` now fails **closed** when the computed `actor_id` already binds a **different** signing key anywhere in `actor_event` history (immortal even after `revoke`); idempotent same-key re-enroll still passes. The signing key is deliberately **not** hashed into `actor_id` (ADR-0011 §5 `rotate-key` must preserve it), so this is an enforcement gate, not a derivation change.

- New pure `STABLE` `cairn_actor_id_key_conflict(actor_id, key)` predicate + an `enroll_actor` guard (`db/004`, edited in place — pre-clinical posture).
- **Single door:** no remote-apply door for actor enrollment exists yet; forward caveat recorded (mirror at that door when actor-event sync lands — analogue of #154).
- Human pinned sets should carry a person-distinguishing determinant (guidance; the floor makes a forgotten one loud). Field left to the future enrollment surface (ADR-0011 keeps pinned-set contents as policy).
- **ADR-0044** refines ADR-0011/0029; spec §7.5 sentence + version bump v0.44→v0.45.

## Tests
CI-gated Rust integration test `actor_enroll_collision.rs` (collision refused; idempotent re-enroll allowed; distinct pinned sets don't collide; immortality after revoke) + a SQL mirror in `db/tests/004_actors_test.sql`. Full cairn-node DB-gated workspace green; fmt + clippy clean; mkdocs builds.

## Design / plan
`docs/superpowers/specs/2026-07-09-actor-id-collision-floor-design.md` · `docs/superpowers/plans/2026-07-09-actor-id-collision-floor.md`

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

- [ ] **Step 5: Report the PR URL to the user.**

---

## Self-Review notes

- **Spec coverage:** design §1 (floor rejection) → Task 1; §2 (human guidance) → Task 1 comment + Task 3 ADR/§7.5; §3 scope boundaries (single door, in-place, rotate-key compat) → Task 1 comments + Task 3 ADR; test plan 1–5 → Task 1 (Rust) + Task 2 (SQL predicate assertions). All covered.
- **Type consistency:** `cairn_actor_id_key_conflict(bytea, text) -> boolean` used identically in Task 1 (def + guard), Task 1 revoke, and Task 2 assertions. `generate_key() -> (SigningKey, String)` and `connect_and_load_schema` match `suppression_owner_gate.rs`.
- **Placeholder scan:** none — every step carries concrete code/commands. The only intentionally-discovered values are exact version strings / §7.5 insertion point (Task 3 steps grep first, then edit), because the current spec version string and §7.5 line numbers must be read live rather than guessed.
