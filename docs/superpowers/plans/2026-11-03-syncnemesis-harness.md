# Plan — syncnemesis: a randomised multi-node convergence harness

- **Issue:** [#934](https://github.com/cairn-ehr/cairn-ehr/issues/934). Its parent is the eval 0005 follow-up epic
  [#933](https://github.com/cairn-ehr/cairn-ehr/issues/933).
- **Source:** [eval 0005 §3.5](../../ecosystem/0005-cockroachdb-design-lessons-for-the-sync-plane.md#35-test-the-distributed-system-as-a-system).
  CockroachDB's kvnemesis / Jepsen-Elle / roachtest are the model. We reuse ideas only: the CockroachDB
  Software License is not AGPL-compatible.
- **Date:** 2026-11-03
- **Shape:** seven slices, one PR each (S0–S6), in order. Each slice is TDD: write the invariant or
  operation's failing check first, then the code that makes it pass.

Paper-parity: not clinical-surface — this is test infrastructure that exercises the sync plane; it adds and changes no clinical workflow, gesture or screen.

---

## 1. Goal and the five invariants

Cairn's flagship claim is that sync is safe, idempotent set-union and that every node converges (sync §6.1).
The harness tests that claim as a **system**. It runs *N* real nodes through a seeded random schedule of:

- authoring, pulls, partitions and heals;
- clock and version skew;
- sneakernet and destroy-and-restore.

After a final full heal it asserts the invariants below. On failure it prints the seed and a minimised schedule.

| # | Invariant | Checked as |
|---|---|---|
| I1 | **Set-union convergence** | Every node's set of `event_log` content addresses is identical. The same holds for `node_event` within the trust neighbourhood. |
| I2 | **Projection convergence** | Nodes on the same code read identically through **every registered projection**, enumerated from the projection registry (`db/039`), not a hand-kept list. |
| I3 | **Custody is total** | Every event the harness authored, per its own ledger, is present on every node whose scope includes it. Nothing is lost across restore (ADR-0072). |
| I4 | **Power is earned** | No suppression, attestation or permission takes effect on a node before that node has re-adjudicated it (`cairn_replay_eligible`, ADR-0057). Content never waits; permissions always wait (ADR-0054). |
| I5 | **No silent wedge** | Every verifiable event is applied, or it sits in a pen the schedule *expected*. No unexpected pen remains unacked. No watermark is frozen without a loud cycle. |

**Why each one:**
- I1 is the set-union claim itself.
- I2 catches the gap sync §6.1 openly admits: convergence of bytes is not convergence of display.
- I3 is principle 1 under failure.
- I4 is the fail-safe direction of the floor.
- I5 encodes ADR-0074's lesson that a wedged link is a loss in all but name.

## 2. Decisions, with rationale

**D1 — Location: `crates/cairn-sync/tests/syncnemesis.rs`, with modules under `crates/cairn-sync/tests/common/nemesis/`.**
The harness must drive the real `cairn-sync` binary, and `CARGO_BIN_EXE_*` is only available to integration
tests of the package that defines the binary. `cairn-node` is already a dependency there (`clinical_pull.rs`).

**D2 — Ephemeral per-run databases, not the shared `$CAIRN_TEST_PG`/`$CAIRN_TEST_PG2`.**
The harness creates `cairn_nem_<pid>_<i>` with `CREATE DATABASE` on the cluster that `$CAIRN_TEST_PG` points at.
It loads the schema into each one (`db::connect_and_load_schema`) and drops them at the end. A failing run keeps
them when `SYNCNEMESIS_KEEP=1`, for forensics.

Why: the shared databases are TRUNCATE-shared and serialised by `test_serial_guard`, and there are only two of
them. A harness that needs N ≥ 3 nodes and runs for minutes must neither hold the advisory lock that 247 suites
contend for, nor truncate their tables.

It needs `CREATEDB`. CI already connects as `postgres`. Without it the harness skips loudly through the
repo-wide gate: register it with `tests/db_gate_actually_ran.rs`.

**D3 — Sync goes through the real binaries over loopback; no library seam yet.**
`serve` and `pull` are driven exactly as `clinical_pull.rs` drives them, using `tests/common/serve.rs` (#457).
The pull loop is where the watermark, pen and freeze logic lives (`cairn-sync/src/main.rs`). Calling a
re-extracted library function would test a different code path.

The cost is one process spawn per pull. Measure it in S1. Extract a seam later **only** if the measured runtime
forces it, and if the extraction keeps one shared code path (the #227 pattern).

**D4 — Determinism: generate the whole schedule up front from a seed, then execute it sequentially.**
Use proptest's `TestRunner` API, as `floor_properties.rs` does (MIT/Apache-2.0, already a dev-dependency in the
workspace). A schedule is a `Vec<Op>`, and proptest shrinks it to a minimal counter-example. The harness runs no
concurrency of its own inside a schedule. The only real concurrency is `serve` against `pull`, which is the
concurrency production has. Every failure prints `SYNCNEMESIS_SEED=<n>` and the shrunk schedule. Setting that
variable replays the run.

**D5 — The harness keeps an oracle ledger.**
Every `Op::Author` records the content address it produced, plus the scope and the node it was authored on.
I3 is checked against the ledger, not against any node, because a node cannot be its own witness.

**D6 — Expected divergences are an explicit, self-expiring registry.**
Some divergences are known and open today: the node plane skips and advances where the clinical plane pens
(#268); the clinical plane freezes on non-`P0001` failures (#626); the node plane fails closed on unknown types
(#301); the actor-registry restore discards silently (#569).

Each one gets a `KnownGap { issue, matcher }` entry. Two rules apply:
- **A divergence that matches no entry fails.**
- **An entry that no longer reproduces under its pinned seed also fails.** So when an issue is fixed, the
  harness forces its entry to be removed.

This is not skipping a test. Every divergence is asserted, named and tied to an issue.

**D7 — The clock seam (S4): one `cairn_wall_ms()` function, not libfaketime.**
Postgres's `clock_timestamp()` cannot be skewed per database. libfaketime works per *cluster*, through
`LD_PRELOAD`. It is Linux-centric, and macOS SIP blocks `DYLD_INSERT_LIBRARIES` for system binaries, which breaks
the cross-platform rule even for test infrastructure.

So S4 introduces `cairn_wall_ms()` (VOLATILE) in `db/001`. It routes through it every clock read that orders or
bounds:
- `node_hlc_tick`;
- the four drift ceilings (`db/005`, `db/007`, `db/009`, `db/020`);
- `cairn_clock_health` (`db/040`);
- `cairn_actor_append`'s clock guard (`db/004`).

A single-source guard test pins it, so no new raw read can appear on those paths (the #227 / #626 pattern).
`created_at` defaults stay on `clock_timestamp()`, because they are audit stamps, not ordering.

The harness then **redefines** `cairn_wall_ms()` per ephemeral database, as `real + offset_i`. This creates no
new bypass: a database owner can already redefine any function, and the harness owns these throwaway databases.

Caveat to test first: `force_schema_replay` and the schema replay in `connect_and_load_schema` restore the
original definition. The harness must re-apply its override after every connect it makes, and a harness
self-test must show that the override is live when a pull runs.

This seam is also exactly where [#935](https://github.com/cairn-ehr/cairn-ehr/issues/935)'s peer-corrected time
will plug in, so it is not throwaway work.

**D8 — Comparison is over canonical dumps.**
For I1, compare the sorted content addresses. For I2, compare each registered projection, read with an explicit
`ORDER BY` on its key and dumped as text under `COLLATE "C"` (ADR-0045). Never compare the default output order
of a raw `SELECT *`.

## 3. Slices

### S0 — Skeleton: ephemeral nodes, seed, operation log (no sync yet)

1. **Test first.** `nemesis_self_test_creates_and_drops_n_nodes`: create 3 ephemeral databases, load the schema,
   provision each as a node (the `init` / identity path that `clinical_pull.rs` uses), then drop them. Assert
   that they are gone afterwards, and that they survive under `SYNCNEMESIS_KEEP=1`.
2. Add a `Nemesis` struct holding nodes, keys, listen ports (bind `:0`, so parallel CI jobs never collide), and
   the ledger.
3. Add the `Op` enum (initially `Author` and `Noop`), a proptest strategy for it, and an executor.
4. **Test first.** `same_seed_same_schedule`: the generator is a pure function of the seed. Pin it.
5. On failure: print the seed, the shrunk schedule, and the names of the kept databases.

*Rationale:* the scaffolding must itself be trustworthy before it can judge anything else. A harness that
leaks databases, or cannot replay a seed, produces noise instead of evidence.

### S1 — Author, pull, heal; I1–I3 on 3 nodes in a full mesh

1. Add `Op::Author { node, kind }`. `kind` draws from the production orchestrators: registration
   (`submit_registration`, needed first on every chart, #345), demographic assertions, and medication
   assert / change-dose / cease (`cairn_node::medication`).
2. Add `Op::Pull { from, to }`: one real `cairn-sync pull` against `from`'s `serve`.
3. Add a final `heal()`: pull every ordered pair until a full round changes nothing. **Bound it** (N² × a small
   constant), and fail if it does not quiesce: non-quiescence is itself a finding.
4. **Tests first.** Write I1, I2 and I3 as pure functions over dumps. Fixture-test each against hand-made
   divergent dumps, so that it is known to fail when it should (the anti-vacuity rule from
   `paper_parity_plan_section.rs`).
5. Run with a small budget: 16 cases × ≤ 30 operations. **Measure** the wall time per case. That figure sets the
   S6 budgets; do not guess them.

*Rationale:* this is the walking skeleton of the flagship property. Today it is checked only pairwise
(`clinical_pull.rs`). Even this slice catches order-dependence that a fixed two-node test cannot.

### S2 — Partitions, topologies, and the full sweep

1. Add `Op::Partition { a, b }` / `Op::Heal { a, b }`. A partition makes a pull between `a` and `b` a no-op until
   it heals.
2. Generate topologies: a full mesh; hub-and-spoke (topology §2, a parent and its children); and two siblings
   behind one parent. Only edges in the topology carry pulls.
3. Add `Op::FullSweep { from, to }`, a pull with the full-sweep flag, so the incremental-versus-sweep floor
   (issue #196) is exercised.
4. Strengthen I1: after `heal()` over the topology, every node in the same connected component converges.
   Nodes in different components may differ only by events authored in the other component.

*Rationale:* partitions are the case Cairn exists for (sync §6.3). Hub-and-spoke tests store-and-forward
through the parent, which a full mesh hides.

### S3 — Overlays, suppression, identity; I4 and I5

1. Add `Author` kinds:
   - corrections and addenda with `refs`;
   - suppressing overlays (attested and unattested);
   - a human vouch (attestation);
   - identity link / unlink / repudiate (ADR-0076–0078);
   - sensitivity assertions (ADR-0062).
2. Add `Op::InjectRefused { to }`: hand `to` a verifiable event that its floor refuses, for example one signed by
   an unenrolled actor. Record it in the ledger as "expected penned".
3. **Tests first.** I4: on every node, after every operation (not only at the end), no suppression or attestation
   is effective unless that node has the attester's registry state, and that state is clean. I5: the set of
   pens left after heal equals exactly the set of expected pens, apart from `KnownGap` entries.
4. Seed the `KnownGap` registry with #268 / #626 / #301 / #569, each with a seed that reproduces it.

*Rationale:* I4 is checked **per step** because "power is earned" is a safety property of every intermediate
state, not just of the end state. The danger is a suppression that hides an allergy for ten minutes before a
re-adjudication catches up.

### S4 — The clock seam and clock faults (reproduces F1)

1. **Test first.** The single-source guard: every ordering or bounding clock read on the D7 paths goes through
   `cairn_wall_ms()`. Then do the refactor. It is behaviour-neutral, so the whole existing suite must stay green
   unchanged.
2. Add a harness override of `cairn_wall_ms()` per node (D7), with the self-test that the override is live during
   a pull.
3. Add `Op::Skew { node, offset_ms }`, drawn from {0, ±seconds, ±hours, ±25 h, −months}. Add
   `Op::DeadRtc { node }`: the node restarts at a fixed old time.
4. Add invariant I6 — **causal order is never inverted**. For every event with `refs`, its HLC is greater than
   its target's HLC on every node.
5. **Expected outcome:** F1 ([#935](https://github.com/cairn-ehr/cairn-ehr/issues/935)) shows up as an I6
   failure, and as a node-plane `revoke` that never applies. Register it as a `KnownGap` tied to #935, with its
   seed. That gives #935 its reproduction for free.

*Rationale:* clocks are the one fault that a set-union design does *not* make harmless, and CockroachDB's
correctness depends on them. This is the slice that turns eval 0005's main finding into a standing test.

### S5 — Version skew, sneakernet, destroy-and-restore, node and actor planes

1. Add `Op::AuthorUnknownType { node }`: an event type that other nodes cannot classify. The receivers must admit
   it uninterpreted (ADR-0056). Reuse the approach of `deferred_admission.rs`.
2. Add `Op::Reclassify { node }`, which simulates the code plane arriving: reprojection must re-adjudicate first
   (ADR-0057). I4 must hold throughout.
3. Add `Op::Sneakernet { from, to }`, through `cairn-medium` (a capture on one node, applied on another).
4. Add `Op::DestroyAndRestore { node }`: the `cairn-node backup` / `restore` ceremony (ADR-0026, 0067–0072).
   The restored node gets a new identity linked by `supersede`. I3 must hold across this.
5. Add node-plane and actor-plane operations: pair, unpeer, enrol, rotate-key, revoke, suspend. These use the
   node-plane pull (`cairn_node::sync::pull_into` / `pull_once`). Extend I1 to `node_event` within the trust
   neighbourhood.

*Rationale:* each of these is a door on which a past divergence was found by review. Putting them under one
generator is what makes those divergences findable as a class.

### S6 — CI wiring and the regression corpus

1. **Per-PR budget:** chosen from S1's measurement so that the job adds ≤ 3 minutes. Probably about 16 cases at
   N = 3. It runs in the existing DB-gated job that already sets `CAIRN_TEST_PG`.
2. **Nightly budget:** about 30–60 minutes, N = 5, with longer schedules. Add a scheduled workflow. Failures
   upload the shrunk schedule as an artefact.
3. **Regression corpus:** `crates/cairn-sync/tests/nemesis-seeds.txt`. Every seed that ever failed is replayed
   on every PR, forever. This is cheap and catches regressions of fixed bugs.
4. Add a HANDOVER line and a RUNBOOK entry on how to replay a seed and read a kept database.

## 4. Known gaps and non-goals (keep this section current)

- **No real network faults below TCP.** Packet loss and reordering are not simulated. Pull frames are atomic
  per page. The TLA+ model ([#942](https://github.com/cairn-ehr/cairn-ehr/issues/942)) covers reordering for
  the quarantine protocol.
- **No concurrency inside the harness**, by D4. Concurrency between two pulls that write into one node at once
  is a later extension (two `pull` processes against one database). The doors are transactional, but that is
  an assumption until tested.
- **Blob byte tier (ADR-0013):** out of scope for S0–S6. Content-addressed self-verification makes it a weaker
  candidate. Add it later if a blob-plane divergence is ever found.
- **Performance:** the harness measures correctness, not budgets. Paper-parity time budgets stay with the
  RUNBOOK measurement cluster.

## 5. Invariants that later issues will add

| Issue | Adds |
|---|---|
| [#937](https://github.com/cairn-ehr/cairn-ehr/issues/937) order-unknown | Nodes on the same code agree on both the winner and the *order-unknown* flag. |
| [#938](https://github.com/cairn-ehr/cairn-ehr/issues/938) rule epochs | Same events plus same epochs gives the same display, regardless of when each node upgraded. |
| [#939](https://github.com/cairn-ehr/cairn-ehr/issues/939) frontiers | A completeness frontier is never claimed beyond what was delivered. |
| [#941](https://github.com/cairn-ehr/cairn-ehr/issues/941) surplus GC | I3 holds under random GC. |
| [#935](https://github.com/cairn-ehr/cairn-ehr/issues/935) F1 | The F1 `KnownGap` entry is removed; I6 holds under every skew in S4. |
