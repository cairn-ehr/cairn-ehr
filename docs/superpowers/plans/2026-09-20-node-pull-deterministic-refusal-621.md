# Plan — a deterministic door failure is a refusal, not a fault (#621)

**Design:** [`2026-09-20-node-pull-deterministic-refusal-621-design.md`](../specs/2026-09-20-node-pull-deterministic-refusal-621-design.md) ·
**Issue:** [#621](https://github.com/cairn-ehr/cairn-ehr/issues/621) · **Clinical twin filed:** [#626](https://github.com/cairn-ehr/cairn-ehr/issues/626) ·
**Branch:** `fix/621-node-pull-non-p0001-freeze`

Paper-parity: not clinical-surface — sync/admission plumbing below the API layer (ADR-0021's four-layer
model); no clinician-visible workflow is added or changed, and no clinical event type is touched.

TDD throughout: every task writes its failing test first, and the SQL work is proven by a mutation
(ledger at the bottom). No migration file is added — db/001, db/007 and db/009 are edited in place, as
#619 did, so `SCHEMA_GENERATION` stays **53**.

## Task 1 — RED: the doors' malformed-input refusals (behaviour)

New `crates/cairn-node/tests/node_door_refusals_are_p0001.rs`, built on `common/node_plane_kit.rs`
(`node_event_spelled` gives an arbitrary `event_id` text; a new kit helper gives an arbitrary HLC and
payload). For each of the three doors — `submit_node_event`, `apply_remote_node_event` (db/007) and
`restore_node_event` (db/009) — and each malformed input:

1. a non-UUID `event_id`;
2. a non-UUID `payload.target_event_id` on a `peer.revoked`;
3. a negative `hlc.wall`; and a negative `hlc.counter`;
4. a `role` outside the vocabulary on a `peer.added`.

assert the refusal carries **`P0001`** and names its **door** and its **field**. Plus the anti-vacuity
half: a well-formed event still applies through each door, and a valid odd UUID *spelling*
(`spelled_oddly`) is still accepted — the helper must not be narrower than the cast it replaces.

Red on the current tree with `22P02` / `23514`.

## Task 2 — GREEN: the doors become total

- **db/001:** extract the value-characterising prefix out of `cairn_decode_hex_or_raise` into a pure
  `cairn_value_glimpse(text)` (same output, one home), then add `cairn_uuid_or_raise(field, value,
  door)` on `pg_input_is_valid` and `cairn_node_hlc_nonneg_or_raise(wall, counter, door)`.
- **db/007:** `cairn_node_role_is_known(text)` IMMUTABLE + `cairn_node_role_or_raise(role, door)`; the
  `node_event_role_check` constraint is re-pointed at the predicate with an idempotent
  `DROP CONSTRAINT IF EXISTS` / `ADD CONSTRAINT` pair (db/009's `op` precedent). Both doors call the
  three helpers; no bare `::uuid` remains in either body.
- **db/009:** the same three call sites in `restore_node_event`.

Grants mirror `cairn_decode_hex_or_raise` (REVOKE from PUBLIC).

## Task 3 — Source + catalogue guards

Extend `node_door_refusals_are_p0001.rs` (or a sibling) with the guards `hex_decode_helper.rs` taught:

1. each helper is declared **once**, in the file the design names (db/001 for the two shared ones, so
   cairn-sync's subset can reach them — the #198 trap);
2. **every door still calls it** — a vanished call restores the illegible raise with the tree green.
   Asked of `pg_proc.prosrc` (what actually runs), like `substitution_guard_covers_every_writer.rs`;
3. **no bare `::uuid` in any of the three door bodies** — the rule that keeps the next cast from
   reopening the freeze;
4. the live `node_event_role_check` definition **references `cairn_node_role_is_known`**, so the
   vocabulary cannot be re-inlined into two places.

## Task 4 — RED: the puller's classifier (pure, no DB)

Unit tests in `crates/cairn-node/src/sync.rs` for `deterministic_apply_failure(Option<&str>)`:
`None` and `08/40/42/53/55/57/58` are NOT deterministic (this node's own trouble → freeze);
`22P02`, `23514`, `23502`, `XX000` are. Plus a drift guard, `sqlstate_classes_agree.rs`: the class
list in `cairn-node`'s classifier and the one in `cairn-sync`'s `apply_failure_is_local` are read from
source and must be equal — two planes, one list, until #626 merges them.

## Task 5 — GREEN: pen the deterministic refusal

`pull_into`'s last arm splits: local/no-SQLSTATE freezes exactly as today (same line, same reason);
anything else pens through the existing `pen_or_freeze` with a reason in the database's vocabulary
(SQLSTATE + `legible_db_error`), counts `quarantined`, and lets the cursor advance. Doc-comment on
`pull_into` updated — its four-freeze-path list is now three-plus-one and the module header states
the new partition.

## Task 6 — The pull, end to end (DB-gated, self-pull)

In the same suite family as #619's (`common/node_plane_kit.rs`'s `self_node` / `serve_raw`):

1. a verifiable event with a non-UUID `event_id` is **skipped**, the cursor **advances**, nothing is
   penned, nothing freezes (the D1+D3 path — this is the issue's own scenario);
2. a deterministic non-P0001 — fault-injected by a `cairn_test_*` trigger on `node_event` raising
   `USING ERRCODE = '23514'`, dropped at test start and end (no residue, HANDOVER's rule) — is
   **penned**, not frozen, and the pen row is ack-able;
3. a **local** fault (a `SET ROLE` to a role without INSERT rights → `42501`, the seam #619's review
   found) still **freezes**: the safe direction is untouched.

## Task 7 — Mutation run

`scripts/mutations/2026-09-20-621.sh`, adapted from #619's (clean-tree positive control, unique
anchors both ways, refuses an unknown id). Ledger below; a survivor is declared and reasoned, never
hidden.

## Task 8 — Docs

ADR-0074 (+ `mkdocs.yml` nav line in the same commit — `--strict`), spec §-note in the node/federation
aspect file, HANDOVER trap 14 + ⇒ NEXT, ROADMAP entry, and the `docs/requirements.txt`-pinned docs
build. Close #621 by hand after merge (the closing-keyword guard).

## Mutation ledger

Filled in during Task 7; each row is *mutation → the test that killed it*.

| id | mutation | killed by |
| --- | --- | --- |
| | | |
