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

## 1. Goal and the invariants (I1–I5 here; I6a/I6b arrive with the clock seam in S4)

Cairn's flagship claim is that sync is safe, idempotent set-union and that every node converges (sync §6.1).
The harness tests that claim as a **system**. It runs *N* real nodes through a seeded random schedule of:

- authoring, pulls, partitions and heals;
- clock and version skew;
- sneakernet and destroy-and-restore.

After a final full heal it asserts the invariants below. On failure it prints the seed and writes the minimised
schedule to a file that replays **the same operations** (D4). The run itself is reproducible only as far as D4's
determinism rules make it.
S4 adds **I6a/I6b** (causal order under clock skew, defined there), since they need the clock seam to mean
anything. §5 lists the invariants that later issues add.

| # | Invariant | Checked as |
|---|---|---|
| I1 | **Set-union convergence** | Every node's set of `event_log` content addresses is identical. From S5, when node-plane operations exist, the same holds for `node_event` within the trust neighbourhood. Until then the node plane carries only provisioning, and I1 does not assert it. |
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
The harness creates `cairn_nem_<run>_<i>` with `CREATE DATABASE` on the cluster that `$CAIRN_TEST_PG` points at.
`<run>` is a fresh random id per run (UUIDv7, already a dependency), **not the pid**. A pid recurs across runs
on a persistent cluster, and across CI jobs that share one. It loads the schema into each one
(`db::connect_and_load_schema`) and drops them at the end. A failing run keeps them when `SYNCNEMESIS_KEEP=1`,
for forensics.

A drop that only runs at the end leaks on a panic, an abort, Ctrl-C or a CI timeout, because no destructor
runs. So the harness also **labels** and **sweeps**:
- Every database it creates gets `COMMENT ON DATABASE … IS 'syncnemesis run=<run> created=<ts> keep=<bool>'`.
- At startup it drops every `cairn_nem_%` database whose comment says `keep=false`, that is older than a
  generous bound (6 h, longer than any nightly budget), and that has **no** connection in `pg_stat_activity`.
  The last two conditions keep it from dropping a concurrent run's live nodes on a shared cluster.
- Databases kept under `SYNCNEMESIS_KEEP` are never swept. The failure output names them, and the RUNBOOK entry
  (S6) says how to drop them.

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
Use proptest's `TestRunner` API, as `floor_properties.rs` does. proptest (MIT/Apache-2.0) is a dev-dependency of
`cairn-node` but **not yet of `cairn-sync`**, where D1 puts the harness. S0 adds it there at the same `"1"`
requirement, so the workspace keeps one resolved version and gains no new licence to audit. A schedule is a
`Vec<Op>`, and proptest shrinks it to a minimal counter-example. The harness runs no concurrency of its own
inside a schedule. The only real concurrency is `serve` against `pull`, which is the concurrency production has.

**A seed is a search coordinate, not a witness.** `SYNCNEMESIS_SEED=<n>` reproduces a run only while the `Op`
strategy is unchanged. S2–S5 each add `Op` variants and kinds, and that changes what every existing seed
generates. So:
- `Op` derives `serde::Serialize`/`Deserialize` (serde and serde_json are already `cairn-sync` dependencies).
- Every failure prints the seed **and writes the shrunk schedule as JSON**.
- The **schedule file is the durable artefact.** `SYNCNEMESIS_SCHEDULE=<file>` replays it directly through the
  executor and skips the generator. That replay stays valid across any generator change that does not remove
  or reshape an `Op` variant. Removing or reshaping one makes the corpus load fail with a deserialization error
  that names the file, so the change is loud, never silent. The corpus loader runs on every PR (S6), and an
  unloadable schedule fails it. `SYNCNEMESIS_SEED` stays available for re-running the *search* on the current
  generator.

**Replaying the operations is deterministic; the run is not entirely.** A replay also meets values the harness
does not choose: real wall time (every skew is `real + offset`), the keys and UUIDv7s that production code mints
during `init` and authoring, the ephemeral ports, and how long each process takes. So:
- **No invariant may hinge on those values.** Skew draws keep a margin from every bound. `±25 h` sits an hour
  outside the 24 h ceiling, never within seconds of it. I6a's admission time errs toward demanding the order
  (S4). Orderings that rest on a UUID tiebreak are compared as *sets*, not as sequences, unless the tiebreak is
  itself the property under test.
- **A witness must prove it is deterministic before it is accepted.** Registering a `KnownGap` witness, or a
  corpus entry, replays it three times, and it must reproduce on all three. A schedule that reproduces only
  sometimes is not a witness. It is a finding, about nondeterminism in the system or in the harness, and it is
  filed as one. D6's rule 2 can then trip on a single non-reproduction without being flaky.

**D5 — The harness keeps an oracle ledger.**
Every `Op::Author` records the content address it produced, plus the scope and the node it was authored on.
I3 is checked against the ledger, not against any node, because a node cannot be its own witness.

**D6 — Expected divergences are an explicit, self-expiring registry.**
Some divergences are known and open today: the node plane skips and advances where the clinical plane pens
(#268); the clinical plane freezes on non-`P0001` failures (#626); the node plane fails closed on unknown types
(#301); the actor-registry restore discards silently (#569).

Each one gets a `KnownGap { issue, matcher, witness }` entry. `witness` is a checked-in shrunk **schedule file**
(D4), not a seed: a seed would stop reproducing the gap the moment a later slice extended the generator, and
rule 2 would then fail a build in which nothing was fixed. Two rules apply:
- **A divergence that matches no entry fails.**
- **An entry whose witness schedule no longer reproduces its divergence also fails.** So when an issue is fixed,
  the harness forces its entry to be removed. Because the witness is replayed without the generator, only a
  change in the system under test can trip this rule.

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

**The override must survive the binaries' own schema replay, and today it would not.** Every connect a
`cairn-sync` process makes calls `load_schema` (`cairn-sync/src/main.rs`), which **replays every migration
unconditionally**. It has no equivalent of cairn-node's #726 replay-skip (`db::replay_can_be_skipped`). So the
first thing each `cairn-sync pull` does is run db/001's `CREATE OR REPLACE FUNCTION cairn_wall_ms()`, and that
resets the skewed node to the true clock *inside the very process whose apply doors F1 lives in*. The harness
cannot re-apply the override there, because D3 makes the binary, not the harness, own that connection. Left
as it is, S4 would never reproduce F1, and it would pass for the wrong reason.

**Extending the #726 replay-skip to `cairn-sync` does not fix this on its own.** That was this plan's first
answer, and it is wrong. `node_schema` holds **one** stamp, and that stamp names **one** `loader_build`.
`replay_can_be_skipped` (`cairn-node/src/db.rs`) refuses to skip on another loader's stamp, deliberately:
cairn-sync replays only a *subset* of the migrations, so its stamp cannot vouch for cairn-node's files. Every
harness node is touched by both binaries. The cairn-node orchestrators author, and `cairn-sync` pulls. So the
two loaders would hand the stamp back and forth: each connect sees the other's name and replays everything,
the override included.

> [!IMPORTANT]
> **Open decision — owed by the maintainer before S4 starts.** Where does the skew live so that it survives
> both loaders? Each option changes production code, which is why this is not the plan's call to make.
>
> - **A. Per-loader stamps, and a skip rule that accepts a superset.** `node_schema` gains a row per loader
>   (an additive migration). A loader may skip when the last replay, by *any* loader, ran a file set that
>   contains its own with identical bytes, at the same generation. After cairn-node's first full replay,
>   cairn-sync would then skip for good, and the override survives.
>   - *Cost:* a change to the production loader and the stamp shape, with per-file hashes.
>   - *Bonus:* it removes the same back-and-forth from production. Any node running both binaries on one
>     database pays a full replay at every alternation today.
>   - *Hazard to rule out first:* cairn-sync's subset replay running after cairn-node's could re-apply an older
>     function body that a node-only later migration had superseded (the db/011 → db/013 shape). That hazard
>     exists in production today, and A would remove it rather than add it, but it has to be checked, not
>     assumed.
> - **B. An owner-only offset in the catalog.** `cairn_wall_ms()` returns `clock + offset`, reading the offset
>   from `pg_db_role_setting` (set with `ALTER DATABASE … SET cairn.clock_offset_ms = …`), **not** from
>   `current_setting`. So a session's `SET` cannot move it, which closes the GUC objection below. Neither
>   loader's replay touches the catalog, so the skew survives both.
>   - *Cost:* a catalog lookup on every clock read. Measure it on `node_hlc_tick` before adopting B.
>   - *What it adds to production:* a knob that persists, though only a database owner can turn it (see the
>     correction below).
>   - *It fits #935:* the peer-corrected time from #935 needs a durable per-node offset anyway, and this is
>     where that offset would live.
> - **C. An owner-only offset table**, revoked from every runtime role. It has B's properties, but it is a
>   schema object: it shows up in table dumps, and it could travel inside a full-database copy onto new
>   hardware. B's per-database setting does not, unless the copy includes `--create` settings.
>
> *Recommendation:* **B**. It needs no change to the loader, it survives both loaders by construction, and it
> is the natural home for #935's offset. Whichever option is chosen, `cairn_clock_health` (db/040) must
> report a nonzero offset loudly, as a grade on `status`. And a DB-gated test must assert that the offset is
> absent on every database that is not a harness node.
>
> *Correction to this plan's first draft:* it rejected the table form as "a standing skew lever". That
> contradicted its own premise, since the owner can already redefine the function. The real difference is
> **persistence**: a redefinition heals itself at the next replay, while a knob stays until someone removes
> it. Persistence is answered by visibility (the `status` grade, the absence test), not by ruling the option
> out.

Whichever option is chosen, these hold:
1. **Assert the skew after every pull, not once.** After each `Op::Pull`/`Op::FullSweep`, and after each
   sneakernet apply or restore, the harness checks that the node's effective `cairn_wall_ms()` still carries
   its scheduled offset. If anything resets it, the run fails loudly at the operation that lost the skew,
   instead of silently testing true time. The S4 self-test makes a skewed node's *pulled-in* event reach a
   clock-bounded door, and checks that the bound used the skewed time.
2. The harness's own connects go through cairn-node's `connect_and_load_schema`. Under option A only,
   `force_schema_replay` (which `test_serial_guard` calls) would also reset a function-borne skew, so the
   harness never calls it on a skewed node.

*Rejected outright:*
- **A session GUC read with `current_setting`.** Any role can `SET` a custom placeholder GUC in its own
  session, so the authoring agent could skew its own drift ceiling. That re-opens #102's ratchet in
  production.
- **A shadow `cairn_wall_ms()` earlier on `search_path`.** The doors are
  `SECURITY DEFINER … SET search_path = public, pg_temp`, so they would never see it, which is the hardening
  working as intended.

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
2. **Test first.** `nemesis_sweeps_only_stale_unkept_idle_databases` (D2). Create labelled databases: one
   stale, one fresh, one stale but `keep=true`, and one stale but holding an open connection. Run the startup
   sweep and assert it drops only the first. The sweep is a pure function from (name, comment, age,
   connected) to drop or keep, with a thin SQL wrapper, so the decision is unit-testable without a cluster.
3. Add `proptest = "1"` to `cairn-sync`'s `[dev-dependencies]` (D4). Add a `Nemesis` struct holding nodes,
   keys, listen ports (bind `:0`, so parallel CI jobs never collide), and the ledger.
4. Add the `Op` enum (initially `Author` and `Noop`, deriving serde), a proptest strategy for it, and an
   executor that takes a `Vec<Op>` from either the strategy or a schedule file (D4).
5. **Test first.** `same_seed_same_schedule`: the generator is a pure function of the seed. Pin it. And
   `schedule_file_round_trips`: serialising a schedule and replaying it from the file executes the identical
   `Vec<Op>`.
6. On failure: print the seed and the names of the kept databases, and write the shrunk schedule file.

*Rationale:* the scaffolding must itself be trustworthy before it can judge anything else. A harness that
leaks databases, or cannot replay a failure, produces noise instead of evidence.

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
4. Seed the `KnownGap` registry with #268 / #626 / #301 / #569, each with a checked-in witness schedule that
   reproduces it (D6).

*Rationale:* I4 is checked **per step** because "power is earned" is a safety property of every intermediate
state, not just of the end state. The danger is a suppression that hides an allergy for ten minutes before a
re-adjudication catches up.

### S4 — The clock seam and clock faults (reproduces F1)

1. **Test first.** The single-source guard: every ordering or bounding clock read on the D7 paths goes through
   `cairn_wall_ms()`. Then do the refactor. It is behaviour-neutral, so the whole existing suite must stay green
   unchanged.
2. **Test first.** The mechanism chosen in D7's open decision (A, B or C), with its own tests: under A, the
   superset skip rule; under B or C, that a session `SET` cannot move the offset, and that `cairn_clock_health`
   reports a nonzero offset. **Steps 2 onward wait for that decision.** Step 1 is behaviour-neutral and does
   not depend on it.
3. Add the per-node skew (D7). Add the self-test that the skew is live *inside* a pull's apply door, and the
   per-operation check that it survived (D7, invariant 1).
4. Add `Op::Skew { node, offset_ms }`, drawn from {0, ±seconds, ±hours, ±25 h, −months}. Add
   `Op::DeadRtc { node }`: the node restarts at a fixed old time. The ledger records every node's offset over
   time, so the harness always knows the *true* time at which each event was authored. No node knows that.
5. Add invariants I6a and I6b. The split exists because the drift clamp *deliberately* inverts causal order for
   a target that is genuinely far in the future (#102's ratchet defence). An unscoped "a ref always outranks
   its target" would fail by design, and no fix to #935 could ever make it pass.
   - **I6a — honest causal order is never inverted.** Take every event *E* with `refs` to a target *T*. Let
     *a* be the **true** time at which *E*'s authoring node admitted *T*. The clamp acts at admission, not at
     authoring, so it is admission time that matters. The ledger records *a* as the **start of whichever
     operation admitted *T* on that node**, which errs toward demanding the order. Every admitting path
     defines it:
     - `Op::Pull` / `Op::FullSweep` (the clinical door, db/020);
     - `Op::Sneakernet` (the medium apply);
     - `Op::DestroyAndRestore` (the restore doors);
     - local authoring: if *T* was authored on *E*'s own node, the order is **always** demanded, because both
       came from that node's own monotone tick and no clamp sits between them.

     An admission the ledger cannot place is a **harness error**, never a skipped check: an undefined *a*
     must not silently exempt exactly the path it failed to model. If `T.wall ≤ a + cairn_max_hlc_drift_ms()`,
     then `E.hlc > T.hlc`. Both are the signed HLC values, the same on every node, so this is checked once per
     event, not per node. This is the property F1 breaks: *T* is honest, and *E*'s author is the one whose
     clock is behind.
   - **I6b — the ratchet defence holds.** If `T.wall > a + ceiling`, meaning *T*'s author was more than 24 h
     ahead of true time, then *E* may order below *T*, and that is expected. What must hold instead is a
     bound on every **merge**, not on the state. The harness reads `hlc_state` before and after each operation
     that admits remote events, and asserts that
     `hlc_after ≤ max(hlc_before, cairn_wall_ms() + ceiling, the node's own ticks during the operation)`.
     It must not assert `hlc_state ≤ cairn_wall_ms() + ceiling`. That fails by design after `DeadRtc` regresses
     a node that has already run at true time: its `hlc_state` then legitimately sits months above its clock,
     and no merge caused that. An inversion is excused **only** when the oracle classifies *T* as far-future
     at its admission. Every other inversion is an I6a failure, so the exemption cannot quietly widen to cover
     F1.
6. **Expected outcome:** F1 ([#935](https://github.com/cairn-ehr/cairn-ehr/issues/935)) shows up as an I6a
   failure, and as a node-plane `revoke` that never applies. Register it as a `KnownGap` tied to #935, with its
   witness schedule (D6). That gives #935 its reproduction for free. A `+25 h` author whose events are
   referenced by honest peers is *not* F1. It must satisfy I6b and must not match the F1 `KnownGap`.
   Also schedule `DeadRtc` on a node that has already run at true time. Its `hlc_state` is then more than
   24 h ahead of its clock, and `submit_event` (`db/005`) is expected to refuse the node's *own* authoring
   (eval 0005 §3.1, consequence 4). The executor must record an authoring refusal as an observed divergence,
   matched by the #935 `KnownGap`, not as a harness error.

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
   **Combine it with `Op::DeadRtc`:** replacement hardware is very often a fresh RTC-less Pi booting at its
   `fake-hwclock` time. `restore_node_event` (`db/009`) applies the same raw-clock drift ceiling, and any raise
   aborts the *whole* restore. So F1 is expected to show up here too, as a disaster-recovery restore that fails
   outright on a box whose clock is behind. Register it under #935's `KnownGap`, with its own witness schedule.
5. Add node-plane and actor-plane operations: pair, unpeer, enrol, rotate-key, revoke, suspend. These use the
   node-plane pull (`cairn_node::sync::pull_into` / `pull_once`). I1's `node_event` clause (§1) switches on
   here, now that the node plane carries more than provisioning.

*Rationale:* each of these is a door on which a past divergence was found by review. Putting them under one
generator is what makes those divergences findable as a class.

### S6 — CI wiring and the regression corpus

1. **Per-PR budget:** chosen from S1's measurement so that the job adds ≤ 3 minutes. Probably about 16 cases at
   N = 3. It runs in the existing DB-gated job that already sets `CAIRN_TEST_PG`.
2. **Nightly budget:** about 30–60 minutes, N = 5, with longer schedules. Add a scheduled workflow. Failures
   upload the shrunk schedule as an artefact.
3. **Regression corpus:** `crates/cairn-sync/tests/nemesis-corpus/*.json`. It holds every shrunk schedule that
   ever failed (D4), including the `KnownGap` witnesses, and every one is replayed on every PR, forever. These
   are schedules, not seeds, so the corpus keeps meaning what it meant when each entry was added, however much
   the generator has grown since. It is cheap, and it catches regressions of fixed bugs.
4. Add a HANDOVER line and a RUNBOOK entry covering: replaying a schedule file (and re-running a seed's
   search), reading a kept database, and dropping kept databases once the forensics are done (D2).

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
| [#935](https://github.com/cairn-ehr/cairn-ehr/issues/935) F1 | The F1 `KnownGap` entries (I6a, the frozen node-plane `revoke`, the self-refused authoring after a clock regression, and the S5 restore) are removed. I6a and I6b hold under every skew in S4. I6b's far-future case stays: it is the #102 defence, not a gap. |
