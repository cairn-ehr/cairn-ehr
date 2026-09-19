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
  door)` on `pg_input_is_valid` and `cairn_hlc_nonneg_or_raise(wall, counter, door)`.
- **db/007:** `cairn_node_roles()` (the vocabulary, called by the CHECK itself) + `cairn_node_role_or_raise(role, door)`; the
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
4. the live `node_event_role_check` definition **references `cairn_node_roles()`**, so the
   vocabulary cannot be re-inlined into two places — plus a case proving the CHECK is still a
   FLOOR (it refuses a raw INSERT), since the guard above could read as making it decorative.

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
3. a **local** fault (injected the same way, `40001`) still **freezes**: the safe direction is
   untouched. Plus an anti-vacuity control that the injected trigger fires only for the marked
   event, and the new arm's own freeze path — a pen that cannot be written.

## Task 7 — Mutation run

`scripts/mutations/2026-09-20-621.sh`, adapted from #619's (clean-tree positive control, unique
anchors both ways, refuses an unknown id). Ledger below; a survivor is declared and reasoned, never
hidden.

## Task 8 — Docs

ADR-0074 (+ `mkdocs.yml` nav line in the same commit — `--strict`), spec §-note in the node/federation
aspect file, HANDOVER trap 14 + ⇒ NEXT, ROADMAP entry, and the `docs/requirements.txt`-pinned docs
build. Close #621 by hand after merge (the closing-keyword guard).

## Mutation ledger

`scripts/mutations/2026-09-20-621.sh` — **13 defined, 13 run, 13 killed**, each at the assertion
that names its claim (the harness prints the panic line). No survivors, declared or otherwise.

The harness itself needed one fix first, and it is the reason the run is trustworthy: a
mis-assembled copy ran **zero** mutations and still printed *"tree is clean: every revert landed"*
— true of a run that never happened. It now compares the number of mutations that RAN with what
the arguments asked for.

| id | mutation | killed by |
| --- | --- | --- |
| M1 | the admission gate's `event_id` guard reverts to the bare cast (#621's defect, verbatim) | `node_door_refusals_are_p0001::a_non_uuid_event_id_…` |
| M2 | the same reversion, measured against the CATALOGUE guards alone | `node_door_input_guards::no_node_door_casts_to_uuid_bare` |
| M3 | the LOCAL door's `event_id` guard reverts | `node_door_refusals_are_p0001::a_non_uuid_event_id_…` |
| M4 | the RESTORE door's `event_id` guard reverts | `node_door_refusals_are_p0001::a_non_uuid_event_id_…` |
| M5 | the admission gate's clock guard deleted | `…::a_negative_hlc_wall_…` |
| M6 | the clock guard checks the WALL only — the plausible half-guard | `…::a_negative_hlc_counter_…` |
| M7 | the role guard deleted from the admission gate | `…::an_unknown_peer_role_…` |
| M8 | `target_event_id` reverts to its bare cast | `…::a_non_uuid_target_event_id_…` |
| M9 | the UUID validator becomes NARROWER than the cast it replaces (canonical spellings only) | `…::a_well_formed_event_still_applies_however_its_id_is_spelled` |
| M10 | the role vocabulary re-inlined into the CHECK (behaviour identical today) | `node_door_input_guards::the_role_check_reads_the_one_vocabulary` |
| M11 | a dropped connection (no SQLSTATE) starts penning | `node_pull_refusal_class::no_sqlstate_means_nothing_was_decided…` |
| M12 | the `22` class claimed as LOCAL — #621's defect restated in Rust | `node_pull_refusal_class::a_failure_that_will_recur_identically…` |
| M13 | the new arm stops freezing when its pen could not be written | `node_pull_deterministic_refusal::a_deterministic_refusal_whose_pen_cannot_be_written_freezes` |

M9 and M10 are the two worth reading twice: each leaves every REFUSAL test green and is caught only
by a positive control (M9) or a structural guard (M10). M9 is the mirror of PR #623's finding 1 —
a validator narrower than the parser it stands in for — and M10 is the drift that would let a
widened vocabulary freeze an older node's link.

## What the work actually changed, beyond the plan

- **The `role` CHECK was a fourth deterministic raise the issue did not list**, reachable by any
  trusted author today and by version skew tomorrow. Found by reading the table rather than the
  issue.
- **`restore_node_event` refuses an already-enrolled node**, so its fixture provisions nothing and
  restores a genesis first. The plan assumed the three doors shared one fixture shape.
- **A `serve_raw` row's bytes are already in `node_event` under a different id**, so a clean
  re-apply conflicts on the `content_address` UNIQUE (`23505`) rather than the primary key — which
  the new arm now pens. It is a fixture artifact, not a production path (identical bytes always
  collide on the primary key first), and it cost the anti-vacuity control one rewrite. Recorded in
  that test.
- **The clinical plane has the same defect**, filed as
  [#626](https://github.com/cairn-ehr/cairn-ehr/issues/626) rather than folded in (maintainer
  decision: `db/020` is the 100k-event hot path).
