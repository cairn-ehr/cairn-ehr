# Enroll Dual-Mapping Floor Guard (#166) — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `enroll_actor` (db/004) fail closed when one signing key would bind two distinct `actor_id`s, closing the silent node-wide attribution loss described in issue #166.

**Architecture:** Add a pure whole-history predicate `cairn_key_actor_id_conflict(key, actor_id)` and a per-key advisory lock to `enroll_actor`, mirroring the ADR-0044 / #152 A-direction guard exactly (this is its B-direction complement). Three existing tests deliberately map one key across several actors (to exercise the db/006 NULL-attribution recall fallback); they migrate to raw-INSERT staging first so every commit stays green.

**Tech Stack:** PostgreSQL 18 + PL/pgSQL, the `cairn_pgx` pgrx extension (`cairn_actor_id`), Rust (`tokio-postgres` DB-gated integration tests), mkdocs.

## Global Constraints

- **TDD** — failing test first, then the code that makes it pass (load-bearing on this §9 safety-critical floor surface).
- **All tests pass before every commit** (house rule 6). Each task below ends green.
- **db/004 edited in place** — the pre-clinical, single-node posture (the #99/#152 pattern; no new migration file, no SCHEMA bump — this is floor-authorization, not a wire/projection change).
- **Reviewer-legible SQL + Rust**, junior-developer inline comments explaining *why* (house rule 3).
- **Never hard-code cryptographic material in tests** — keys come from `generate_key()` (house rule 6 / CLAUDE.md rule 6).
- **DB-gated test env:** `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test"` (PG18 + `cairn_pgx` on the Mac :5532 cluster). Tests self-serialize cluster-wide via `db::test_serial_guard`, so plain `cargo test --workspace` is reliable. The `.sql` mirror tests are dev-time checks (NOT run by CI or cargo) — run them via `psql` as noted.
- **RAISE message text is load-bearing** — tests assert on substrings. The new #166 message MUST contain the exact substring `already binds a different actor_id`; the existing A-direction message (`different signing key` / `issue #152`) MUST stay unchanged.
- **Lock order:** always `pg_advisory_xact_lock(key)` **then** `pg_advisory_xact_lock(actor_id)` (one global order → deadlock-free; enroll_actor is the only pair-lock holder).

**Design doc:** `docs/superpowers/specs/2026-07-12-enroll-dual-mapping-floor-guard-design.md` (approved).

---

## Task 1: Prep — migrate dual-mapping test staging (behavior-preserving)

Three tests map one key across several `actor_id`s **through `enroll_actor`**. Task 2's guard will forbid that. Migrate them **first**, in a way that is behaviour-identical on the *current* (un-guarded) floor, so this task commits green and Task 2 breaks nothing new. Two migrate to raw-INSERT staging (they need `actor_id = NULL` coverage); one migrates to a distinct key.

**Files:**
- Modify: `crates/cairn-node/tests/recall_epoch.rs` (the `enroll_epoch` helper, ~lines 37-56)
- Modify: `db/tests/006_recall_test.sql:39,41,75`
- Modify: `db/tests/004_actors_test.sql:12` (+ its comment on lines 10-11)

**Interfaces:**
- Produces: no new public interface. `enroll_epoch(c, kid, epoch) -> Vec<u8>` keeps its exact signature and return value (the minted `actor_id`); only its internal mechanism changes from `enroll_actor` to a raw `actor_event` INSERT computing `cairn_actor_id(pinned)`.

- [ ] **Step 1: Rewrite the `recall_epoch.rs` `enroll_epoch` helper to raw-INSERT staging**

Replace the existing helper (currently `SELECT enroll_actor('agent', '{pinned}', $1)`) with:

```rust
/// Stage an agent enrollment for `kid` pinned to `epoch`; returns the minted actor_id.
///
/// This suite deliberately maps ONE key across SEVERAL epochs (=> several actor_ids) to
/// force `submit_event` (db/005) to stamp `actor_id = NULL`, the only way to exercise the
/// `events_by_actor_epoch` NULL-attribution fallback (db/006). Since issue #166 the
/// `enroll_actor` FLOOR refuses that dual mapping (a fresh enroll of an already-bound key),
/// so we stage it via a raw `actor_event` INSERT: the state still arises from non-enroll
/// paths (historical rows, a future actor-sync apply door that has not yet mirrored the
/// guard), and the recall projection must still cope. `actor_id` is computed exactly as the
/// door would (`cairn_actor_id(pinned)`) so the recall query's `epoch_regs` join matches.
async fn enroll_epoch(c: &Client, kid: &str, epoch: &str) -> Vec<u8> {
    let pinned =
        format!("{{\"model\":\"triage-stub\",\"version\":\"1\",\"skill_epoch\":\"{epoch}\"}}");
    c.query_one(
        "INSERT INTO actor_event (actor_id, op, kind, pinned, signing_key_id) \
         VALUES (cairn_actor_id($1::text::jsonb), 'enroll', 'agent', $1::text::jsonb, $2) \
         RETURNING actor_id",
        &[&pinned, &kid],
    )
    .await
    .unwrap()
    .get(0)
}
```

- [ ] **Step 2: Run the recall suite to confirm it is still green on the current floor**

Run: `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test -p cairn-node --test recall_epoch -- --nocapture`
Expected: PASS (all 6 tests) — the raw INSERT produces the identical `actor_event` rows `enroll_actor` did, so behaviour is unchanged.

- [ ] **Step 3: Migrate `db/tests/006_recall_test.sql` (3 enroll_actor calls → raw INSERT)**

Replace line 39:
```sql
    aid_a := enroll_actor('agent', '{"model":"m","version":"1","skill_epoch":"hist-a"}'::jsonb, 'histkey');
```
with:
```sql
    -- issue #166: this suite maps ONE key ('histkey') across THREE epochs to force
    -- actor_id=NULL and exercise the events_by_actor_epoch NULL-attribution fallback.
    -- enroll_actor now refuses that dual mapping, so stage it via a raw INSERT (the state
    -- still arises from non-enroll paths; the recall projection must still cope). actor_id
    -- is computed as the door would, so the epoch_regs join matches.
    INSERT INTO actor_event (actor_id, op, kind, pinned, signing_key_id)
    VALUES (cairn_actor_id('{"model":"m","version":"1","skill_epoch":"hist-a"}'::jsonb),
            'enroll', 'agent', '{"model":"m","version":"1","skill_epoch":"hist-a"}'::jsonb, 'histkey')
    RETURNING actor_id INTO aid_a;
```

Replace line 41:
```sql
    aid_b := enroll_actor('agent', '{"model":"m","version":"1","skill_epoch":"hist-b"}'::jsonb, 'histkey');
```
with:
```sql
    INSERT INTO actor_event (actor_id, op, kind, pinned, signing_key_id)
    VALUES (cairn_actor_id('{"model":"m","version":"1","skill_epoch":"hist-b"}'::jsonb),
            'enroll', 'agent', '{"model":"m","version":"1","skill_epoch":"hist-b"}'::jsonb, 'histkey')
    RETURNING actor_id INTO aid_b;
```

Replace line 75:
```sql
    PERFORM enroll_actor('agent', '{"model":"m","version":"1","skill_epoch":"hist-c"}'::jsonb, 'histkey');
```
with:
```sql
    INSERT INTO actor_event (actor_id, op, kind, pinned, signing_key_id)
    VALUES (cairn_actor_id('{"model":"m","version":"1","skill_epoch":"hist-c"}'::jsonb),
            'enroll', 'agent', '{"model":"m","version":"1","skill_epoch":"hist-c"}'::jsonb, 'histkey');
```

- [ ] **Step 4: Run the 006 SQL mirror to confirm it is still green**

Run:
```bash
CONN="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test"
psql "$CONN" -v ON_ERROR_STOP=1 \
  -f db/001_envelope.sql -f db/002_projection.sql -f db/003_blobs.sql \
  -f db/004_actors.sql -f db/005_submit.sql -f db/006_recall.sql \
  -f db/tests/006_recall_test.sql
```
Expected: ends with `NOTICE:  events_by_actor_epoch history resolution OK` and no ERROR (the test BEGINs and never COMMITs, so it rolls back on exit — no residue).

- [ ] **Step 5: Migrate `db/tests/004_actors_test.sql` epoch-bump to a distinct key**

The `epoch-a`/`epoch-b` block proves *pinned → distinct actor_id*; the shared key was incidental. Give `epoch-b` its own key (what the real matcher does — a fresh key per epoch). Change lines 10-15 from:
```sql
-- Bumping skill_epoch mints a DIFFERENT actor_id (the supersede trigger for C4).
SELECT enroll_actor('agent',
    '{"model":"triage-stub","version":"1","skill_epoch":"epoch-b"}'::jsonb,
    'deadbeef') AS aid2 \gset
```
to:
```sql
-- Bumping skill_epoch mints a DIFFERENT actor_id (the supersede trigger for C4). A fresh
-- key per epoch matches the real matcher (matcher_actor.rs) and the issue #166 floor guard
-- (one key binds at most one actor_id); the actor_id derives from the pinned set alone, so
-- the distinct key does not affect what this asserts.
SELECT enroll_actor('agent',
    '{"model":"triage-stub","version":"1","skill_epoch":"epoch-b"}'::jsonb,
    'deadbee2') AS aid2 \gset
```

- [ ] **Step 6: Run the 004 SQL mirror to confirm it is still green**

Run:
```bash
CONN="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test"
psql "$CONN" -v ON_ERROR_STOP=1 -f db/004_actors.sql -f db/tests/004_actors_test.sql
```
Expected: no ERROR; the append-only / tiebreak / collision `NOTICE`s print and the epoch-bump assertion (`epoch_bump_is_new_actor` = t) holds.

- [ ] **Step 7: Full workspace green (nothing else disturbed)**

Run: `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test --workspace`
Expected: PASS (no guard added yet — this is a pure test-staging refactor).

- [ ] **Step 8: Commit**

```bash
git add crates/cairn-node/tests/recall_epoch.rs db/tests/006_recall_test.sql db/tests/004_actors_test.sql
git commit -m "test(actors): stage dual-mapping recall setups via raw INSERT (prep for #166)

The contamination-cascade recall suites (recall_epoch.rs, 006_recall_test.sql)
deliberately map one key across epochs to force actor_id=NULL and exercise the
events_by_actor_epoch fallback. The upcoming #166 enroll guard forbids creating
that via enroll_actor, so stage it via a raw actor_event INSERT (same rows the
door would write). 004_actors_test.sql's epoch-bump gets a distinct key per
epoch (what the real matcher does). Behaviour-preserving on the current floor.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 2: The floor guard (db/004) + B-direction tests + SQL mirror (TDD)

**Files:**
- Test: `crates/cairn-node/tests/actor_enroll_collision.rs` (append two tests after line 168)
- Modify: `db/004_actors.sql` (new predicate after `cairn_actor_id_key_conflict` ~line 100; two checks + one lock in `enroll_actor` ~lines 126-133; one REVOKE ~line 162)
- Test: `db/tests/004_actors_test.sql` (append a #166 block after the keyAAAA/keyBBBB collision block, ~line 95)

**Interfaces:**
- Produces: `cairn_key_actor_id_conflict(p_key TEXT, p_actor_id BYTEA) RETURNS BOOLEAN` — pure `STABLE`; TRUE iff `p_key` already binds a *different* `actor_id` anywhere in `actor_event` history (`op IN ('enroll','supersede')`). Reusable at the future actor-sync apply door / rotate-key door.
- Consumes: the existing `cairn_actor_id(jsonb)` (from `cairn_pgx`), `actor_event`, `actor_event_key_idx`.

- [ ] **Step 1: Write the two failing B-direction tests**

Append to `crates/cairn-node/tests/actor_enroll_collision.rs`:

```rust
#[tokio::test]
async fn dual_mapping_serial_is_refused() {
    // issue #166: one signing key binding TWO actor_ids makes db/005 stamp actor_id=NULL
    // for every event that key authors node-wide (silent attribution loss). The floor now
    // refuses a fresh enroll of an already-bound key under a different pinned set.
    let Some(cs) = cs() else { return };
    let _g = db::test_serial_guard(&cs).await.unwrap();
    let c = db::connect_and_load_schema(&cs).await.unwrap();
    reset(&c).await;
    let (_sk, kid) = generate_key().unwrap();
    // Same key, first pinned set -> actor A1 (fine).
    c.execute(
        "SELECT enroll_actor('agent', '{\"model\":\"m\",\"skill_epoch\":\"a\"}', $1)",
        &[&kid],
    )
    .await
    .unwrap();
    // Same key, DIFFERENT pinned set -> a distinct actor_id A2 -> refused (would dual-map).
    let err = c
        .execute(
            "SELECT enroll_actor('agent', '{\"model\":\"m\",\"skill_epoch\":\"b\"}', $1)",
            &[&kid],
        )
        .await
        .expect_err("a second actor_id for the same key must be refused");
    assert!(
        db_msg(&err).contains("already binds a different actor_id"),
        "expected the #166 dual-mapping RAISE, got: {}",
        db_msg(&err)
    );
    // The key still resolves to exactly ONE live actor (no NULL-attribution trap opened).
    let n: i64 = c
        .query_one(
            "SELECT count(DISTINCT actor_id) FROM actor_current WHERE signing_key_id = $1",
            &[&kid],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(n, 1, "key must map to exactly one live actor after the refusal");
}

#[tokio::test]
async fn key_reuse_after_revoke_is_refused() {
    // issue #166 whole-history / anti-key-reuse (the B-direction mirror of #152's
    // anti-resurrection): a key that ever bound a different actor can NEVER enroll a new
    // one, even after that actor is revoked. This is the deliberate guard-scope choice.
    let Some(cs) = cs() else { return };
    let _g = db::test_serial_guard(&cs).await.unwrap();
    let c = db::connect_and_load_schema(&cs).await.unwrap();
    reset(&c).await;
    let (_sk, kid) = generate_key().unwrap();
    let aid: Vec<u8> = c
        .query_one(
            "SELECT enroll_actor('agent', '{\"model\":\"m\",\"skill_epoch\":\"a\"}', $1)",
            &[&kid],
        )
        .await
        .unwrap()
        .get(0);
    // Revoke actor A (raw INSERT — the registry has no runtime revoke door yet).
    c.execute(
        "INSERT INTO actor_event (actor_id, op) VALUES ($1, 'revoke')",
        &[&aid],
    )
    .await
    .unwrap();
    // Same key onto a NEW actor, even though A is no longer live -> still refused.
    let err = c
        .execute(
            "SELECT enroll_actor('agent', '{\"model\":\"m\",\"skill_epoch\":\"b\"}', $1)",
            &[&kid],
        )
        .await
        .expect_err("key-reuse onto a new actor after revoke must be refused (whole-history)");
    assert!(
        db_msg(&err).contains("already binds a different actor_id"),
        "expected the #166 whole-history RAISE, got: {}",
        db_msg(&err)
    );
}
```

- [ ] **Step 2: Run the new tests to verify they FAIL**

Run: `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test -p cairn-node --test actor_enroll_collision dual_mapping_serial_is_refused key_reuse_after_revoke_is_refused -- --nocapture`
Expected: FAIL — both `.expect_err(...)` panic because the current floor *allows* the second enroll (no B-direction guard exists), so no error is returned.

- [ ] **Step 3: Add the pure `cairn_key_actor_id_conflict` predicate to db/004**

Insert after the `cairn_actor_id_key_conflict` function (after line 100, before the `enroll_actor` comment block):

```sql
-- issue #166: the B-direction mirror of cairn_actor_id_key_conflict. submit_event (db/005)
-- resolves a signer to an actor purely by signing_key_id; if one key maps to MORE than one
-- actor_id it stamps actor_id = NULL for EVERY event that key authors node-wide (a silent,
-- irreversible attribution loss). So enroll must fail closed when p_key already binds a
-- DIFFERENT actor_id. This pure predicate is TRUE iff some existing enroll/supersede row
-- carries p_key under an actor_id other than p_actor_id.
--
-- WHOLE-HISTORY, deliberately (the guard-scope decision): a key that ever bound a different
-- actor can never enroll a new one, even after that actor is revoked/superseded — the same
-- anti-reuse posture cairn_actor_id_key_conflict takes in the A-direction (principle 2, a
-- key is one lifelong actor identity). `op IN ('enroll','supersede')` restricts to the
-- key-bearing ops; revoke rows carry a NULL signing_key_id and are excluded by the equality
-- anyway. STABLE + pure so it is independently testable and reusable at the future
-- actor-sync apply door (ADR-0044 §3) and a future rotate-key/supersede door.
CREATE OR REPLACE FUNCTION cairn_key_actor_id_conflict(p_key TEXT, p_actor_id BYTEA)
RETURNS BOOLEAN LANGUAGE sql STABLE AS $$
    SELECT EXISTS (
        SELECT 1 FROM actor_event
        WHERE signing_key_id = p_key
          AND actor_id IS DISTINCT FROM p_actor_id
          AND op IN ('enroll','supersede')
    );
$$;
```

- [ ] **Step 4: Add the per-key lock + B-check to `enroll_actor`**

In `enroll_actor`, the existing body acquires the actor_id lock then runs the A-check. Change the lock acquisition (currently the single `PERFORM pg_advisory_xact_lock(hashtextextended(encode(aid, 'hex'), 0));` at ~line 126) to acquire the **key lock first**, and add the B-check immediately after the existing A-check's `END IF;`:

Replace:
```sql
    PERFORM pg_advisory_xact_lock(hashtextextended(encode(aid, 'hex'), 0));
```
with:
```sql
    -- issue #166: serialize concurrent enrolls of the SAME KEY (this guard) as well as of
    -- the same actor_id (the #152 guard). Key lock FIRST, then actor_id lock — one global
    -- acquire order across the single enroll door, so no deadlock is possible. A distinct
    -- seed (…, 1) keeps the key-lock namespace from aliasing the actor_id-lock namespace
    -- (…, 0). Under READ COMMITTED the loser blocks on the key lock, then re-reads the
    -- winner's committed row and is refused by the B-check below.
    PERFORM pg_advisory_xact_lock(hashtextextended(p_key, 1));
    PERFORM pg_advisory_xact_lock(hashtextextended(encode(aid, 'hex'), 0));
```

Then insert, immediately after the existing A-direction `END IF;` (the one closing the `cairn_actor_id_key_conflict` block, ~line 133) and before `INSERT INTO actor_event ...`:
```sql
    -- issue #166 (B-direction): refuse if this signing key already binds a DIFFERENT
    -- actor_id anywhere in history — otherwise db/005 would NULL this key's authorship
    -- node-wide (whole-history / anti-key-reuse; see cairn_key_actor_id_conflict).
    IF cairn_key_actor_id_conflict(p_key, aid) THEN
        RAISE EXCEPTION
            'enroll_actor: signing key % already binds a different actor_id — a fresh enroll is refused: one key mapping to two actors silently NULLs that key''s authorship node-wide (db/005; issue #166)',
            p_key
        USING HINT =
            'A genuinely different entity needs its own key. To add a key to the SAME actor, use rotate-key/supersede (no door yet). A retired key never becomes a new actor (whole-history, anti-key-reuse).';
    END IF;
```

- [ ] **Step 5: Add the REVOKE for the new predicate**

After the existing `REVOKE EXECUTE ON FUNCTION cairn_actor_id_key_conflict(bytea, text) FROM PUBLIC;` (~line 162), add:
```sql
REVOKE EXECUTE ON FUNCTION cairn_key_actor_id_conflict(text, bytea) FROM PUBLIC;
```

- [ ] **Step 6: Run the new tests to verify they PASS**

Run: `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test -p cairn-node --test actor_enroll_collision -- --nocapture`
Expected: PASS — all 6 existing + 2 new tests green. (`distinct_pinned_sets_do_not_collide` and `idempotent_same_key_re_enroll_is_allowed` are the no-regression coverage for distinct-keys and idempotency; they must still pass unchanged.)

- [ ] **Step 7: Add the #166 SQL mirror to db/tests/004_actors_test.sql**

Append after the keyAAAA/keyBBBB #152 collision block (~line 95), using fresh keys:
```sql
-- issue #166 (B-direction): one signing key must not bind two actor_ids. A fresh enroll
-- under a distinct pinned set with an already-bound key is refused (else db/005 NULLs that
-- key's authorship node-wide). The mirror of the #152 A-direction guard above.
DO $$
DECLARE ok boolean := false;
BEGIN
    PERFORM enroll_actor('agent', '{"model":"m","skill_epoch":"dm-a"}'::jsonb, 'dualkey');
    BEGIN
        PERFORM enroll_actor('agent', '{"model":"m","skill_epoch":"dm-b"}'::jsonb, 'dualkey');
    EXCEPTION WHEN others THEN
        ok := (SQLERRM LIKE '%different actor_id%');
        IF NOT ok THEN
            RAISE EXCEPTION 'wrong error for #166 dual-mapping: %', SQLERRM;
        END IF;
    END;
    IF NOT ok THEN
        RAISE EXCEPTION 'FAILED: #166 dual-mapping enroll (one key, two actor_ids) was allowed';
    END IF;
    RAISE NOTICE 'issue #166 dual-mapping refusal OK';
END $$;
```

- [ ] **Step 8: Run the 004 SQL mirror**

Run:
```bash
CONN="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test"
psql "$CONN" -v ON_ERROR_STOP=1 -f db/004_actors.sql -f db/tests/004_actors_test.sql
```
Expected: prints `NOTICE:  issue #166 dual-mapping refusal OK` and no ERROR.

- [ ] **Step 9: Full workspace green + clippy + fmt**

Run:
```bash
CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test --workspace
cargo clippy --workspace --tests -- -D warnings
cargo fmt --check
```
Expected: all PASS (recall_epoch, enroll_human, attestation, apply_* all still green — the guard only affects same-key-different-actor enrolls, which no other test does).

- [ ] **Step 10: Commit**

```bash
git add db/004_actors.sql crates/cairn-node/tests/actor_enroll_collision.rs db/tests/004_actors_test.sql
git commit -m "fix(actors): enroll fails closed on key->two-actors dual mapping (#166)

The B-direction complement of ADR-0044/#152: cairn_actor_id_key_conflict
guarded one actor_id <- two keys, but nothing stopped one key -> two
actor_ids, which makes submit_event (db/005) NULL that key's authorship
node-wide. New pure whole-history predicate cairn_key_actor_id_conflict +
a per-key advisory lock (key-lock-first, deadlock-free) refuse it. Idempotent
same-key re-enroll and distinct-key enrolls are unaffected.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 3: Reframe the enroll.rs Guard 1 docs (clear the house-rule-5 debt)

The floor now enforces the invariant race-safely, so the Rust orchestrator's Guard 1 is advisory legibility + the idempotent shortcut, not the (racy) enforcement. No behaviour change.

**Files:**
- Modify: `crates/cairn-node/src/enroll.rs` (the Guard 1 docstring ~lines 113-118 and the inline comment ~lines 137-146)

**Interfaces:**
- Consumes / Produces: none changed. `enroll_human_actor` keeps its signature and its accept/refuse/`AlreadyEnrolled` behaviour exactly.

- [ ] **Step 1: Update the doc-comment for Guard 1 (lines 113-118)**

Replace the `/// 1. **Dual-mapping guard.** …` bullet with:
```rust
///    1. **Dual-mapping shortcut (advisory).** `submit_event` resolves a signer to an actor
///       purely by `signing_key_id`; if one key maps to MORE than one `actor_current` row it
///       sets `actor_id = NULL` for EVERY event that key authors node-wide (`db/005`). Since
///       issue #166 the `enroll_actor` FLOOR enforces "one key binds at most one actor_id"
///       race-safely (`cairn_key_actor_id_conflict` + a per-key advisory lock), so this check
///       is no longer the enforcement — it is advisory legibility plus the idempotent
///       shortcut: same key, same `actor_id`, already human is a re-runnable no-op we can
///       answer without a floor round-trip (and without inserting a duplicate `enroll` row).
```

- [ ] **Step 2: Update the inline comment on Guard 1 (lines 137-146)**

Replace the block starting `// Guard 1 — is this key already an actor? This is a best-effort read-then-write, NOT` … through `// tracked in issue #166.` with:
```rust
    // Guard 1 — is this key already an actor? The db/004 FLOOR is the enforcement now
    // (issue #166): enroll_actor refuses a fresh enroll of an already-bound key under a
    // different actor_id, serialized by a per-key advisory lock, so the concurrent
    // same-key/different-actor race is closed at the floor. This read is advisory: it lets
    // us return the idempotent AlreadyEnrolled outcome (same key, same actor, already human)
    // without a floor round-trip or a duplicate enroll row, and surface a friendly message
    // for the non-idempotent case before the floor's own RAISE — the same
    // advisory-mirrors-the-floor pattern as Guard 2 (attester_is_enrolled_human).
```

- [ ] **Step 3: Build + clippy + the enroll_human suite green**

Run:
```bash
cargo build -p cairn-node
cargo clippy -p cairn-node --tests -- -D warnings
CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test -p cairn-node --test enroll_human -- --nocapture
```
Expected: builds clean; the 7 `enroll_human` DB-gated tests still pass (behaviour unchanged).

- [ ] **Step 4: Commit**

```bash
git add crates/cairn-node/src/enroll.rs
git commit -m "docs(enroll): reframe Guard 1 as advisory now the floor enforces #166

The db/004 floor enforces the key->one-actor invariant race-safely, so the
Rust dual-mapping guard is advisory legibility + the idempotent AlreadyEnrolled
shortcut, not the racy enforcement. Clears the house-rule-5 debt tracked in #166.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 4: ADR-0046 + spec §7.5 note + spec version bump

**Files:**
- Create: `docs/spec/decisions/0046-enroll-fail-closed-on-key-actor-dual-mapping.md`
- Modify: `docs/spec/security.md` (§7.5, the "Because `actor_id` is the content-address …" bullet)
- Modify: `docs/spec/index.md:9` (spec version 0.46 → 0.47)

**Interfaces:** documentation only; no code.

- [ ] **Step 1: Write ADR-0046**

Create `docs/spec/decisions/0046-enroll-fail-closed-on-key-actor-dual-mapping.md`:

```markdown
# ADR-0046 — Enroll fails closed on key→actor dual mapping (the B-direction whole-history guard)

- **Status:** Accepted (refines [ADR-0044](0044-enroll-fail-closed-on-actor-id-collision.md), [ADR-0011](0011-actor-registry-version-pinning-and-key-custody.md))
- **Date:** 2026-07-12

## Context

[ADR-0044](0044-enroll-fail-closed-on-actor-id-collision.md) made `enroll_actor` (db/004) fail
closed in the **A-direction**: one `actor_id` claimed by two *different* signing keys, refused via
the whole-history predicate `cairn_actor_id_key_conflict(actor_id, key)`. The **B-direction** — one
signing *key* enrolled under two *different* `actor_id`s — was left unguarded (issue #166).

`submit_event` (db/005) resolves the author of every event to an actor purely by `signing_key_id`
(`SELECT array_agg(DISTINCT actor_id) … WHERE signing_key_id = …`; a single result stamps
`event_log.actor_id`, anything else stamps `NULL`). So a key mapping to **two** `actor_id`s silently
sets `actor_id = NULL` for **every** event that key authors node-wide — a graceful but real,
irreversible attribution loss (principle 4: "attribution honestly unknown", not misattribution). The
old floor allowed that mapping to be created even **serially** (each enroll computes a *different*
`actor_id`, and the A-direction check only inspects rows already under that `actor_id`); a Rust
orchestrator (`enroll_human_actor`) carried a read-then-write guard for it, but per-caller and racy,
so every future enrollment door (matcher agent, device, the ADR-0044 §3 actor-sync apply door) would
re-derive the same hole.

The per-epoch matcher already assigns a **fresh key per epoch** (`matcher_actor.rs`), so a
one-key→one-actor invariant does not constrain any legitimate flow.

## Decision

`enroll_actor` fails closed in the B-direction too: a fresh `enroll` binding a signing key that
already binds a **different** `actor_id` **anywhere in `actor_event` history** is refused. The
invariant is now bidirectional: **≤ 1 distinct signing key per `actor_id`, and ≤ 1 distinct
`actor_id` per signing key.**

1. A pure `STABLE` predicate `cairn_key_actor_id_conflict(p_key, p_actor_id)` — TRUE iff an
   `enroll`/`supersede` row carries `p_key` under an `actor_id` other than `p_actor_id`.
   Independently testable, reusable at the future actor-sync apply door / rotate-key door.
2. `enroll_actor` acquires a txn-scoped advisory lock on the **key first**, then on the `actor_id`
   (one global order → deadlock-free), then runs both the A- and B-direction checks. Under READ
   COMMITTED the loser of a concurrent same-key enroll re-reads the committed row and is refused,
   closing the TOCTOU race the Rust orchestrator could not.

**Whole-history, not live-only** (the deliberate scope choice): a key that ever bound a *different*
actor can never enroll a new one, even after that actor is revoked/superseded. This mirrors the
A-direction's anti-resurrection posture and the mission's identity conservation — a signing key is a
lifelong cryptographic identity of one entity; reusing it for a second actor is the silent identity
blur the floor exists to refuse. Both directions prevent the db/005 NULL harm; whole-history
additionally bans key-reuse across actors.

**Not a unique constraint:** idempotent re-enroll deliberately inserts a *second* `enroll` row with
the same key and same `actor_id` (re-runnable provisioning), which a partial unique index over
`signing_key_id` would reject. The predicate + advisory-lock shape is the ADR-0044 mechanism reused.

**Single door, mirror obligation:** the check lives on the only implemented write door
(`enroll_actor`). A future `rotate-key`/`supersede` door and the ADR-0044 §3 actor-event sync apply
door **must mirror both checks** — the analogue of the [issue #154](https://github.com/cairn-ehr/cairn-ehr/issues/154) apply-door caveat.

## Consequences

- **Safer:** the silent node-wide attribution loss is unreachable via the enroll door; the invariant
  is enforced at the floor, not re-derived per caller. The Rust `enroll_human_actor` Guard 1 becomes
  advisory legibility + the idempotent shortcut.
- **Harder / narrower:** a genuinely different entity must present its own key (already true for the
  matcher and for humans). A retired key cannot be recycled onto a new actor.
- **Unchanged:** db/005's NULL-on-ambiguity degradation and `events_by_actor_epoch`'s NULL-attribution
  fallback (db/006) stay — the guard narrows *how* the ambiguous state can arise (no longer via the
  enroll door), not the projection's duty to cope with it if it arises by another path (historical
  rows, a not-yet-guarded future sync door). The contamination-cascade recall tests stage that state
  via raw `actor_event` INSERT accordingly.

Floor-authorization only: no wire / event-format / SCHEMA / projection change.
```

- [ ] **Step 2: Add the B-direction clause to spec §7.5**

In `docs/spec/security.md`, the bullet beginning "**Immutable, version-pinned identity …**" contains the sentence "Because `actor_id` is the content-address of the pinned set **alone** … enrolment **fails closed** if the computed `actor_id` already binds a *different* signing key anywhere in history — two actors can never silently merge ([ADR-0044](decisions/0044-enroll-fail-closed-on-actor-id-collision.md), principle 2)." Extend it by appending, immediately after that sentence:

```markdown
 Symmetrically, a signing key **binds at most one `actor_id`** over the whole registry history: a fresh enrolment of a key that already binds a *different* actor is refused, so one key can never silently map to two actors (which would erase that key's authorship node-wide) — the B-direction of the same guard ([ADR-0046](decisions/0046-enroll-fail-closed-on-key-actor-dual-mapping.md), principle 2). Both future doors that bind a key to an actor (`rotate-key`/`supersede`, actor-event sync apply) must mirror both checks.
```

- [ ] **Step 3: Bump the spec version**

In `docs/spec/index.md`, change line 9 from `**Spec version:** 0.46` to `**Spec version:** 0.47`.

- [ ] **Step 4: Build the docs to verify Markdown + links**

Run: `uv run --with-requirements docs/requirements.txt -- mkdocs build`
Expected: build succeeds; no warnings about the new ADR link or the §7.5 edit.

- [ ] **Step 5: Commit**

```bash
git add docs/spec/decisions/0046-enroll-fail-closed-on-key-actor-dual-mapping.md docs/spec/security.md docs/spec/index.md
git commit -m "docs(adr): ADR-0046 — enroll fails closed on key->actor dual mapping (#166)

Records the B-direction whole-history invariant (<=1 actor_id per key) as the
symmetric completion of ADR-0044, the rationale for whole-history/anti-key-reuse,
and the future-door mirror obligation. Spec §7.5 note; version 0.46 -> 0.47.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 5: Session wrap — HANDOVER / ROADMAP / ADR index + close #166

Do this at PR time (house rules 8-9). Keep both docs concise (< 500 lines where feasible; prune older condensed blocks if needed).

**Files:**
- Modify: `docs/HANDOVER.md` (new top session block; add ADR-0046 to the ADR index table; drop #166 from the open-issues prose)
- Modify: `docs/ROADMAP.md` (Phase 2 — the enroll collision floor is now bidirectional)

- [ ] **Step 1: Add the session block + ADR-0046 row to HANDOVER.md**, note #166 closed, and update the ROADMAP Phase-2 enroll-collision bullet to say the guard is bidirectional (A-direction #152/ADR-0044 + B-direction #166/ADR-0046).

- [ ] **Step 2: Commit the doc currency update**

```bash
git add docs/HANDOVER.md docs/ROADMAP.md
git commit -m "docs(handover,roadmap): #166 enroll dual-mapping floor guard (ADR-0046)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

- [ ] **Step 3: Push + open the PR (links #166)**

```bash
git push -u origin fix/enroll-dual-mapping-floor-166
gh pr create --base main --title "fix(actors): enroll fails closed on key->actor dual mapping (#166, ADR-0046)" --body "$(cat <<'BODY'
Closes #166.

The B-direction complement of ADR-0044/#152. `enroll_actor` now fails closed when
one signing key would bind two distinct `actor_id`s — otherwise `submit_event`
(db/005) silently NULLs that key's authorship node-wide. Whole-history /
anti-key-reuse guard via a new pure predicate `cairn_key_actor_id_conflict` + a
per-key advisory lock (key-lock-first → deadlock-free), mirroring #152.

- ADR-0046 (refines 0044); spec §7.5 note; v0.46 → v0.47.
- enroll.rs Guard 1 reframed as advisory (clears the house-rule-5 debt in #166).
- Contamination-cascade recall tests (recall_epoch.rs, 006_recall_test.sql) staged
  via raw INSERT to preserve their NULL-attribution coverage; 004 epoch-bump uses a
  distinct key per epoch.
- New tests: dual_mapping_serial_is_refused, key_reuse_after_revoke_is_refused +
  the 004 SQL mirror. Full workspace green; clippy + fmt + mkdocs clean.

🤖 Generated with [Claude Code](https://claude.com/claude-code)
BODY
)"
```

---

## Self-Review

**Spec coverage** (design §-by-§ → task):
- §2 invariant → Task 2 (predicate + check) + Task 4 (ADR-0046 records it). ✓
- §3.1 predicate → Task 2 Step 3. ✓  §3.2 lock+check → Task 2 Steps 4. ✓  §3.3 REVOKE → Task 2 Step 5. ✓
- §4 enroll.rs reframe → Task 3. ✓
- §5 tests (dual-mapping, key-reuse-after-revoke; idempotent + distinct-keys already exist as no-regression; SQL mirror) → Task 2 Steps 1,7. ✓
- §5.1 test migrations (recall_epoch.rs + 006 raw INSERT; 004 distinct key) → Task 1. ✓
- §6 docs (ADR-0046, §7.5, version, HANDOVER/ROADMAP/index) → Task 4 + Task 5. ✓
- §7 out-of-scope (no wire/SCHEMA change; db/005 + db/006 unchanged) → respected; no task touches them. ✓

**Placeholder scan:** no TBD/TODO; every code step shows the actual code. ✓

**Type/name consistency:** `cairn_key_actor_id_conflict(text, bytea)` used identically in the predicate (Task 2 S3), the REVOKE (S5), the `enroll_actor` call (S4), and the ADR (Task 4). RAISE substring `already binds a different actor_id` matches every test assertion (Task 2 S1) and the SQL mirror's `%different actor_id%` (S7). `enroll_epoch` keeps its signature (Task 1 S1). ✓
