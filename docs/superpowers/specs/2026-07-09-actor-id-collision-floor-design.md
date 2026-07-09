# Design — enroll fail-closed on `actor_id` collision (issue #152)

**Date:** 2026-07-09 · **Issue:** [#152](https://github.com/cairn-ehr/cairn-ehr/issues/152) ·
**Tier:** safety-critical in-DB floor (§9) · **Refines:** ADR-0011 (+ADR-0029)

## Problem

`enroll_actor` (in [`db/004_actors.sql`](../../../db/004_actors.sql)) derives
`actor_id = cairn_actor_id(p_pinned)` — a content-address of the **pinned determinant set only**
(ADR-0011: *identity is the hash of what is pinned*). Two **different** signing keys enrolled with an
**identical** pinned set (e.g. the minimal `{"role":"clinician"}`) therefore compute the **same**
`actor_id`. `actor_current` is `DISTINCT ON (actor_id) … ORDER BY (recorded_at, seq) DESC`, so the later
enrollment's key wins and the earlier key **silently drops out** — the two actors are merged with no error.

This is a silent identity-merge on the trust-anchor floor. It violates founding principle 2 (*identity is a
claim; never merge — always link*). Surfaced by the ADR-0043 owner-gate tests (they worked around it by
giving each test human a distinct pinned tag), then confirmed against `db/004`.

## The load-bearing constraint

The obvious "fix" — hash the signing key into `actor_id` — is **wrong**. ADR-0011 §5 mandates `rotate-key`:
a human/agent keeps the **same** `actor_id` across key rotation (old publics retained for historical
verification). So `actor_id` must be **stable across key changes**; the key cannot be a determinant of it.
The distinguishing determinant for a human must be something *other* than the key and *stable* across
rotation (a person handle / registration id). The fix is therefore an **enforcement gate on enroll**, not a
change to the derivation.

## Decision

### 1. Floor rejection (the load-bearing change)

`enroll_actor` computes `aid := cairn_actor_id(p_pinned)`, then **before inserting** refuses the enroll if
any existing `actor_event` row with `actor_id = aid` carries a `signing_key_id` **distinct from `p_key`**.

- **Whole history**, including `revoke` rows: an `actor_id` is never reusable by a different key, even after
  revoke (principle 2 — patient/actor UUIDs are immortal; the same rule for actors).
- **Idempotent re-enroll with the same key passes** — supports re-runnable provisioning and the matcher
  per-epoch re-enroll (`matcher_actor.rs`), which re-enrolls the same `(pinned, key)` on startup.
- Legible `RAISE EXCEPTION` naming the collision and the remedy: *give the actor a distinguishing pinned
  determinant, or use `rotate-key` to add a key to the same actor.*

The collision predicate is factored as a small pure `STABLE` SQL helper
`cairn_actor_id_key_conflict(p_actor_id BYTEA, p_key TEXT) RETURNS BOOLEAN` — independently testable and
reusable at the future actor-sync apply door (house rule: prefer pure, reusable functions).

### 2. Human distinguishing determinant — guidance only (YAGNI)

No human-enrollment product surface exists yet (`enroll_actor` is called for `agent`/`device` in code;
humans appear only in tests). So we do **not** invent a required human field. Instead the ADR + a `db/004`
comment state: *a human actor's pinned set SHOULD carry a person-distinguishing determinant (handle /
registration id); the floor rejection makes a forgotten determinant loud on the second enroll.* The actual
field is left to the future enrollment surface — ADR-0011 keeps pinned-set **contents** as policy.

### 3. Scope boundaries (deliberate)

- **Single door.** There is no remote-apply door for actor enrollment yet (`INSERT INTO actor_event` lives
  only in `enroll_actor`; `db/020` merely *reads* `actor_current`). Forward caveat recorded in the ADR and
  a comment: when actor-event sync lands (ADR-0011 §4 says enrollment is meant to sync), mirror this check
  at that apply door — same shape as the #154 apply-door caveat.
- **`rotate-key` / `supersede` unimplemented.** The check is written so it will not obstruct them when they
  land: they are distinct ops, and `rotate-key` is the *sanctioned* same-actor / new-key route. The enroll
  gate targets only the accidental-collision case.
- **`db/004` edited in place** (pre-clinical posture; the #99 hardening pattern — no real clinical data or
  legacy nodes yet).

## Files touched

- `db/004_actors.sql` — new `cairn_actor_id_key_conflict` helper; `enroll_actor` calls it and raises;
  comment on the human-determinant guidance + the single-door forward caveat.
- `db/tests/004_actors_test.sql` — the new floor tests (below).
- `docs/spec/decisions/0044-*.md` — the short refining ADR.
- `docs/spec/security.md` §7.5 — one sentence (enroll fail-closes on `actor_id` collision with a distinct
  key; human actors carry a person-distinguishing determinant). Spec version bump in `docs/spec/index.md`.
- `docs/HANDOVER.md`, `docs/ROADMAP.md` — end-of-session currency.

## Test plan (TDD, DB-gated)

Failing test first, then the floor change.

1. **Collision rejected** — enroll `(pinned=P, key=K1)`; a second enroll `(P, K2)` (K2≠K1) raises.
2. **Idempotent re-enroll allowed** — enroll `(P, K1)` twice succeeds (no raise).
3. **No false positive** — enroll `(P1, K1)` and `(P2, K2)` with P1≠P2 both succeed.
4. **Immortality after revoke** — enroll `(P, K1)`, `revoke` that `actor_id`, then enroll `(P, K2)` still
   raises (whole-history check, not `actor_current`).
5. **Pure predicate** — `cairn_actor_id_key_conflict(aid, key)` returns the expected boolean directly for
   the same-key, different-key, and no-row cases.

All DB-gated (need PG18 + `cairn_pgx`; `CAIRN_TEST_PG=…`). Existing tests already enroll distinct pinned
sets, so none should regress; the implementation step will confirm the full `db/tests` + workspace suites
stay green.

## Non-goals

- Deduplicating idempotent same-key enroll rows (harmless; out of scope).
- Any change to `cairn_actor_id` derivation, to `actor_current`, or to the demographics/matcher surfaces.
- Implementing `rotate-key` / `supersede` (separate future slices).
