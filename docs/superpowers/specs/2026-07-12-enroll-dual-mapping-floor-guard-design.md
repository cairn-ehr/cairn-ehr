# Design — enroll dual-mapping floor guard (issue #166)

**Date:** 2026-07-12 · **Issue:** [#166](https://github.com/cairn-ehr/cairn-ehr/issues/166)
· **Touches:** `db/004_actors.sql`, `crates/cairn-node/src/enroll.rs`, tests, ADR-0046, spec §7.5
· **Class:** safety-critical in-DB floor (§9 — Rust/SQL, reviewer-legible)

## 1. Problem

`submit_event` (`db/005`) resolves the signer of every event to an actor purely by its
signing key:

```sql
SELECT array_agg(DISTINCT actor_id) INTO v_actor_ids
    FROM actor_current WHERE signing_key_id = b ->> 'signer_key_id';
v_actor_id := CASE WHEN array_length(v_actor_ids, 1) = 1 THEN v_actor_ids[1] END;
```

If a signing key maps to **more than one** live `actor_id`, `v_actor_id` is set to `NULL`
for **every** event that key authors node-wide — a silent, irreversible attribution loss
(graceful *degradation* to "attribution honestly unknown", principle 4 — not
misattribution, but a real loss).

The `db/004` floor does not prevent that dual mapping. Its existing guard,
`cairn_actor_id_key_conflict(actor_id, key)`, protects only the **A-direction** — one
`actor_id` claimed by two *different* keys (ADR-0044 / issue #152). The **B-direction** —
one *key* enrolled under two *different* `actor_id`s — is unguarded. It is allowed even
**serially** today (enroll key K as actor A1 with pinned P1, then enroll the same K as A2
with a distinct pinned P2: each `enroll_actor` computes a *different* `actor_id`, each
A-direction check sees a fresh `actor_id` with no rows, both insert), and the Rust
orchestrator's read-then-write guard for it (`enroll.rs` Guard 1) is racy and per-caller,
so every future enrollment door (matcher agent, device, the ADR-0044 §3 actor-sync apply
door) re-derives the same hole.

**Non-goal / already safe:** the per-epoch matcher actor uses a *fresh signing key per
epoch* (`matcher_actor.rs` — "a fresh key per epoch gives UNIQUE key→actor attribution"),
so a one-key→one-actor invariant does **not** break per-epoch re-enrollment.

## 2. The invariant

The symmetric completion of ADR-0044: **≤ 1 distinct `actor_id` per signing key, over the
whole registry history.** A signing key is lifelong-bound to one actor identity; a fresh
`enroll` binding a key that ever bound a *different* actor is refused. This is
**whole-history / anti-key-reuse**, deliberately mirroring ADR-0044's whole-history
A-direction guard (immortality + anti-resurrection): a retired (revoked/superseded) key
can **never** become a new actor.

Rationale for whole-history over live-only (both prevent the `db/005` NULL harm; the
difference is only whether a *revoked* actor's key may later enroll a *new* actor):
identity conservation is the mission posture ("never merge/erase, always link/overlay"),
and a key permanently naming one entity is the same discipline ADR-0044 already applies in
the mirror direction. A signing key is a cryptographic identity of one entity; reusing it
for a second actor is exactly the kind of silent identity blur the floor exists to refuse.

## 3. Mechanism (mirrors the ADR-0044 / #152 pattern exactly)

Not a unique/exclusion constraint. Idempotent re-enroll deliberately inserts a **second**
`enroll` row carrying the *same* key and *same* `actor_id` (re-runnable provisioning; the
`cairn_actor_id_key_conflict` "same-key re-enroll allowed" case), which a partial unique
index over `signing_key_id` would reject. The invariant is not a plain uniqueness, so it
is expressed as a **pure predicate + advisory-lock serialization**, the identical shape
the A-direction already uses and reuses at future doors.

### 3.1 New pure predicate (`db/004`, edited in place — pre-clinical posture, the #99/#152 pattern)

```sql
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

- **Whole-history** over `actor_event` (immortal — a revoked actor's key never frees up);
  uses the existing `actor_event_key_idx (signing_key_id)`.
- `actor_id IS DISTINCT FROM p_actor_id` (BYTEA): TRUE only for a *different* actor, so the
  idempotent same-key/same-actor re-enroll never trips it.
- `op IN ('enroll','supersede')`: the key-bearing ops. `revoke`/pre-key rows carry a NULL
  `signing_key_id` and are excluded from `signing_key_id = p_key` anyway; the explicit `op`
  filter documents intent and keeps the predicate symmetric with the A-direction's reasoning.
- `STABLE`, pure, independently testable, and **reusable** at the future actor-sync apply
  door (ADR-0044 §3) and a future `rotate-key`/`supersede` door.

### 3.2 `enroll_actor` — two-lock serialization + the B-check

Before the insert, acquire **the key lock first, then the actor_id lock** (a fixed global
order — the only function taking a *pair* of advisory locks, and every other advisory-lock
user in the tree takes a single fixed-key lock alone, so no cross-ordering cycle is
possible → deadlock-free). Then run the existing A-check **and** the new B-check:

```sql
-- serialize concurrent enrolls of the SAME key (this #166 guard) and of the SAME
-- actor_id (the ADR-0044 guard). Key lock FIRST, then actor_id lock: one consistent
-- acquire order across the single enroll door => no deadlock. Distinct seed (…,1) keeps
-- the key-lock namespace from colliding with the actor_id-lock namespace (…,0).
PERFORM pg_advisory_xact_lock(hashtextextended(p_key, 1));
PERFORM pg_advisory_xact_lock(hashtextextended(encode(aid, 'hex'), 0));

IF cairn_actor_id_key_conflict(aid, p_key) THEN
    RAISE EXCEPTION ... ;              -- unchanged (A-direction, #152)
END IF;
IF cairn_key_actor_id_conflict(p_key, aid) THEN
    RAISE EXCEPTION
        'enroll_actor: signing key % already binds a different actor_id — a fresh enroll is refused: one key mapping to two actors silently NULLs that key''s authorship node-wide (db/005; issue #166)', p_key
    USING HINT =
        'A genuinely different entity needs its own key. To add a key to the SAME actor, use rotate-key/supersede (no door yet). A retired key never becomes a new actor (whole-history, anti-key-reuse).';
END IF;
```

Under READ COMMITTED, a concurrent same-key enroll blocks on the key lock, and after the
winner commits the loser re-reads the committed `actor_event` row → B-check fires → refused.

### 3.3 Privilege floor

`REVOKE EXECUTE ON FUNCTION cairn_key_actor_id_conflict(text, bytea) FROM PUBLIC;`
(defense-in-depth — it reads the trust-anchor table; same treatment as
`cairn_actor_id_key_conflict`).

## 4. Rust orchestrator (`crates/cairn-node/src/enroll.rs`)

The floor now enforces the dual-mapping invariant **race-safely**, so `enroll_human_actor`
Guard 1 is **reframed** (not removed): it stays as *advisory legibility + the idempotent
`AlreadyEnrolled` shortcut* (avoids inserting a duplicate `enroll` row on a re-run and
returns a clean outcome), exactly the "advisory-mirrors-the-floor" pattern Guard 2 already
uses. Its docstring/inline comment drops the "accepted limitation / racy / tracked in
issue #166" language and instead states that `db/004`'s `cairn_key_actor_id_conflict` + the
per-key advisory lock are the enforcement. **This clears the house-rule-5 debt the issue
tracks.** No behavioural change to the accept/refuse decisions; the floor is now the
backstop the Rust guard always claimed to defer to.

## 5. Tests (TDD, DB-gated, serial-refusal per the #152 convention)

Extend `crates/cairn-node/tests/actor_enroll_collision.rs` (the natural home — same
subject, same helpers `reset`/`db_msg`/`test_serial_guard`):

1. **`dual_mapping_serial_is_refused`** — enroll key K as A1 (pinned P1); enrolling the
   *same* K as A2 (distinct pinned P2) is refused with the #166 message. RED before the
   guard (proves the serial hole), GREEN after.
2. **`idempotent_same_key_same_actor_still_allowed`** — re-enroll K with the *same* pinned
   passes (no regression to the #152 re-runnable-provisioning behaviour).
3. **`distinct_keys_distinct_actors_ok`** — two distinct keys → two distinct actors (the
   matcher-style / normal case): no false refusal.
4. **`key_reuse_after_revoke_is_refused`** — enroll K→A1, revoke A1, then enroll K→A2
   (distinct pinned) is refused (proves whole-history / anti-key-reuse; live-only would
   allow it).

SQL mirror in `db/tests/004_actors_test.sql` (asserting the serial refusal, matching the
#152 SQL mirror). The advisory-lock race is documented inline in `enroll_actor` (the #152
convention — no nondeterministic concurrency test).

**RED discipline:** confirm test 1 and test 4 fail before the `db/004` guard is added, then
add the predicate + `enroll_actor` change to turn them GREEN.

### 5.1 Existing tests that lean on dual-mapping-via-`enroll_actor` (must migrate)

The new guard forbids what three existing tests deliberately do — enroll **one key** under
**several `actor_id`s** through `enroll_actor`. Each migrates by *intent* (the same
distinction the #152 side-fix drew: a test that leaned on the bug being fixed is corrected,
not deleted). No other test reuses a key across actors (all others enroll distinct
`kid_a`/`kid_h`/… per actor — verified).

- **`db/tests/004_actors_test.sql`** — enrolls key `'deadbeef'` under `epoch-a` **and**
  `epoch-b`. Intent: prove *bumping `skill_epoch` mints a different `actor_id`* (a
  **pinned → actor_id** property). Fix: give `epoch-b` a **distinct key** (what the real
  matcher does — a fresh key per epoch). The property still holds; the key reuse was
  incidental.
- **`db/tests/006_recall_test.sql`** (`histkey` × `hist-a`/`hist-b`/`hist-c`) and
  **`crates/cairn-node/tests/recall_epoch.rs`** (`kid` × `epoch-a`/`epoch-b`, via the
  `enroll_epoch` helper) — the **contamination-cascade recall** suites. Their coverage
  *depends* on the dual mapping: it forces `db/005` to stamp `actor_id = NULL`, which is the
  only way to exercise the `el.actor_id IS NULL` → `'unattributed'` fallback branch of
  `events_by_actor_epoch` (db/006). Distinct keys would stamp every event and **delete that
  coverage**. Fix: stage the multi-epoch mapping via a **raw `INSERT INTO actor_event`**
  (computing `actor_id := cairn_actor_id(pinned)` exactly as `enroll_actor` would, so the
  recall query's `epoch_regs` join still matches), **bypassing** the guarded door — the
  established `suppression_owner_gate.rs:288` precedent ("raw INSERT, NOT `enroll_actor` —
  since #152 the door refuses this"). A short comment on each records *why*: the dual mapping
  is a state the enroll door no longer produces but the recall projection must still handle
  (historical rows / a future sync-apply door that has not yet mirrored the guard / raw
  paths), so the test stages it directly.

## 6. Docs

- **ADR-0046** (refines ADR-0044): the B-direction whole-history key→actor invariant, the
  whole-history/anti-key-reuse rationale, and the **future-door mirror obligation**
  (rotate-key/`supersede` and the ADR-0044 §3 actor-sync apply door MUST run the B-check).
- **Spec §7.5** note (actor registry) recording the completed bidirectional invariant;
  spec version `v0.46 → v0.47` in `docs/spec/index.md`.
- Add ADR-0046 to the ADR index in `docs/HANDOVER.md` + the CLAUDE.md-style table; update
  `docs/ROADMAP.md` Phase 2 (enroll collision floor now bidirectional); **close #166**.

## 7. Out of scope (declared)

- `rotate-key`/`supersede` and the actor-sync apply door do not exist yet; ADR-0046 records
  the mirror obligation for when they land (the analogue of the #154 apply-door caveat).
- No wire / event-format / SCHEMA / projection change (floor-authorization only).
- The A-direction guard, `actor_current`, and `db/005`'s NULL-on-ambiguity degradation are
  unchanged — this makes the ambiguity *unreachable via the enroll door*, it does not
  change how `db/005` copes if it ever arose by another path. `events_by_actor_epoch`
  (db/006) and its `actor_id IS NULL` fallback are unchanged and still tested (see §5.1):
  the guard narrows *how the state arises*, not the projection's duty to cope with it.
