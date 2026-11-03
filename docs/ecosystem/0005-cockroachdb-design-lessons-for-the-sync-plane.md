# Ecosystem eval 0005 — CockroachDB: design lessons for Cairn's resilient distributed record

- **Status:** **Evaluation.** Spec unchanged; no ADR minted. One **suspected defect** in the shipped clock
  handling (§3.1, F1) and several candidate ADR threads (§5) were flagged. The maintainer accepted all the
  follow-ups on 2026-11-03. They are filed under epic [#933](https://github.com/cairn-ehr/cairn-ehr/issues/933)
  (§5), and the harness plan is [`superpowers/plans/2026-11-03-syncnemesis-harness.md`](../superpowers/plans/2026-11-03-syncnemesis-harness.md).
- **Date:** 2026-11-03
- **Subject:** [cockroachdb/cockroach](https://github.com/cockroachdb/cockroach). Read from its `docs/`
  tree: `design.md`, `RFCS/` (194 RFCs), `tech-notes/` and `tla-plus/`, at the default-branch HEAD on
  the date above.
- **Motivation:** the question *"what can Cairn learn, design-wise, from CockroachDB for a resilient
  distributed database?"*

> [!WARNING]
> **Ideas only. No code, no dependency.** Since v24.3 (and patches from v23.1.29, released on or after
> 2024-11-18) CockroachDB ships under the **CockroachDB Software License**. That is a proprietary
> agreement: it requires a licence key to operate, and it permits modification only "for the purposes
> of operating, evaluating, testing, fixing, integrating with, and improving CockroachDB". It is **not**
> AGPL-compatible, so under house rule 1 it is a blocker as a dependency. Older releases were BSL/CCL,
> which is also excluded. Everything below borrows *published design ideas* from prose RFCs and papers.
> Do not copy or port code from that tree.

---

## 0. Bottom line

CockroachDB is the best-documented example of a **CP** distributed SQL database built on hybrid logical
clocks. Cairn is deliberately **AP**: set-union of signed, immutable events, with availability over
consistency (ADR-0001, sync §6). Most of CockroachDB's *protocols* therefore do not transfer:

- Raft per range
- leases
- serializable cross-node transactions
- cluster-version finalization
- loss-of-quorum recovery

Each of them assumes a reachable quorum. Cairn's founding scenario is the solar Pi that has seen no
quorum for a month. Their own loss-of-quorum RFC says the system "always prioritizes consistency of the
data over availability". That is the opposite of Cairn's governing law, and it is said plainly.

The *invariants and the engineering lessons* do transfer, ranked by value:

1. **A node cannot judge clock skew against its own clock** (§3.1). CockroachDB measures clock offset
   against its *peers*. Cairn's drift ceiling measures against `clock_timestamp()` alone, and that
   produces a concrete, reachable failure on the RTC-less Pi that ADR-0058 itself names as a core case
   (**F1**).
2. **Uncertainty intervals are a read-side concept, not just a stamp** (§3.2). Two assertions whose
   `t_recorded` intervals overlap are *of unknown order*. Cairn's recency-first winner projections
   currently present that coin-toss as fact.
3. **Resolved/closed timestamps turn "freshness" into a completeness claim** (§3.3). "Last synced 4 h ago"
   could become "complete with respect to the ED server up to HLC *T*; unknown for any node not heard
   from".
4. **Admission control: one governor for all *elastic* work, paced by the bottleneck resource** (§3.4).
   This generalises ADR-0013's byte-tier isolation to full sweeps, reprojection, capture and the hub
   duplicate sweep.
5. **Randomised whole-system testing (kvnemesis / Jepsen / TLA+)** (§3.5). Cairn has strong per-door
   property tests, but I found no harness that partitions *N* nodes at random, skews their clocks and
   versions, and checks that they converge. This is the single highest-leverage transfer.
6. **A projection-rule change needs a version gate, but keyed to data, not to cluster state** (§3.6).
   This closes the gap sync §6.1 openly admits: fleet-inconsistent display after a winner-rule change.
7. **Retention discipline** (§3.7). CockroachDB's protected-timestamp failure mode (a stalled consumer
   pins garbage forever) has direct Cairn analogues: the quarantine floor, and surplus-copy garbage
   collection.

Several CockroachDB features are **validations** of choices Cairn has already made, sometimes against
CockroachDB's own grain (§4). The most striking: CockroachDB's `REGIONAL BY ROW` re-homing problem is
Cairn's ED→ICU transfer, and they solve it with the exact mutation ADR-0004 rejected.

---

## 1. Where this lands

This is a sync-plane, L0/L1 design read. It is not a candidate component. No CockroachDB software can
attach to a Cairn node (licence), and nothing like it should sit on the inter-node path (principle 12:
the wire core is the signed event set, not a database protocol).

The deployment table (deployment §8) names "HA Postgres pair — Patroni-style failover" for the hospital
core and "Cluster" for regional/national hubs. If a hub ever needs strongly-consistent internal
replication, that is a **node-internal** concern behind one logical node identity, and the AGPL-compatible
route is Postgres-native (Patroni, MIT). See the open question in §5, [Q4 — HA hub identity](#q4-ha-hub-identity).

---

## 2. CockroachDB in one paragraph (for the reviewer who hasn't read it)

Data is split into ranges. Each range is replicated with Raft, and one replica holds a lease to serve
reads and writes.

Every timestamp is a **hybrid logical clock (HLC)** value: physical time plus a logical counter. Every
incoming timestamp ratchets the local HLC forward (`design.md`, "Hybrid Logical Clock").

Correctness depends on a configured **maximum clock offset** ε, 500 ms by default. A transaction at time
*t* carries an **uncertainty interval** `(t, t+ε]`. A value seen inside that interval might have
happened before the transaction or after it. CockroachDB cannot tell, so it restarts the read above it
(`RFCS/20200811_non_blocking_txns.md`; `RFCS/20230122_read_committed_isolation.md`).

**Closed timestamps** are a leaseholder promise: "no more writes ≤ *T* will apply". That lets followers
serve consistent stale reads. **Bounded-staleness reads** choose the freshest timestamp that can be
served locally, explicitly so reads stay *available during a partition*
(`RFCS/20210519_bounded_staleness_reads.md`).

Upgrades use a monotone **cluster version** with version gates. Every node must be reachable to
finalize (`tech-notes/version_upgrades.md`).

Background work is throttled by **admission control** with token buckets
(`tech-notes/admission_control.md`).

Two protocols are specified in **TLA+**: `tla-plus/ParallelCommits` and `tla-plus/StoreLiveness`. KV
correctness is checked by **kvnemesis** plus **Jepsen/Elle** nightly.

---

## 3. Lessons that transfer

### 3.1 Skew must be judged against peers, never against yourself — and F1

**CockroachDB.** The whole design rests on a bounded offset *between* clocks. Offset is a property of a
*pair* of clocks, so it is measured from peer RPC heartbeats. A node that finds itself too far from its
peers is the one taken out of service.

That last point comes from outside knowledge: a node is believed to stop itself if its offset from a
majority of peers exceeds about 80 % of ε. The `docs/` tree does not state the self-termination rule;
it only assumes a cluster "that respects its configured maximum clock offset". Exceeding the bound is
documented as producing stale reads and lease overlap, not graceful degradation.

The key idea is that **a single node cannot tell "that peer is ahead" from "I am behind"**. Only a
comparison against several independent clocks can.

**Cairn today.** `cairn_max_hlc_drift_ms()` (`db/001_envelope.sql`, 24 h) bounds how far a *remote* HLC
wall may sit ahead of **this node's `clock_timestamp()`**. That is a correct defence against the
ratchet attack (issue #102): a hostile peer cannot drag the clock to the year 146 million. But the same
raw-`clock_timestamp()` ceiling guards **four** doors, and every one silently assumes the local clock is the
trustworthy one:

- **Node plane** (`db/007_node_federation.sql`, `apply_remote_node_event`): an event more than 24 h
  "ahead" is **rejected**. It is skipped, advanced past, and re-offered on the next full sweep. The
  comment's own reasoning is *"admitted once local time catches up"*.
- **Clinical plane** (`db/020_apply_remote_event.sql`): the event is admitted unchanged, but the local
  HLC merge is **clamped** to `now + 24 h`.
- **Restore** (`db/009_node_supersede_and_restore.sql`, `restore_node_event`, issue #193): an event more
  than 24 h "ahead" is **refused**, and this door aborts the *whole* restore on any raise.
- **Local submit** (`db/005_submit.sql`, `submit_event` step 1a, issue #187): a body whose asserted wall is
  more than 24 h "ahead" is refused at authoring.

> [!IMPORTANT]
> **F1 (suspected defect — filed as [#935](https://github.com/cairn-ehr/cairn-ehr/issues/935); a reproduction is its first deliverable).** Take the node that ADR-0058 names as a *core* failure
> mode: an RTC-less Raspberry Pi that boots "decades off true time", or (with `fake-hwclock`) at its last
> shutdown time. Then put it **offline with no NTP**, the deployment Cairn exists for. Its local clock
> is *behind* real time by more than 24 h, and "local time catching up" never happens until a human
> sets the clock. Every **honest** peer then looks like a hostile future-clock peer. Four consequences
> follow, all from reading the code above (none yet reproduced in a test):
>
> 1. **The trust plane freezes.** Every node-plane event from a correctly-clocked peer is refused and
>    re-offered forever: peering, **revocation**, supersede. A revoked node or key is not revoked
>    *here*. Clinical content still flows, because that door clamps, so the node looks healthy while its
>    trust state is stale. That is a security regression hiding behind an availability success.
> 2. **Causal inversion on the clinical plane.** The local HLC is held to `local_now + 24 h`, which is
>    below the peer's real-time events. A clinician on that Pi who corrects or adds to a peer's note or
>    demographic assertion mints an event whose HLC is **lower than its target's**. This breaks the
>    spec's claim that "causal HLC order delivers the target first" (data-model §3.19, `refs`). Every
>    recency-first winner projection (ADR-0036/0037/0038, ordered on HLC wall) then ranks the
>    *correction* as **older** than what it corrects. This happens on that node and, after upward sync,
>    on every node. Issue #97 (recency on the t_recorded axis) is related but does not cover it: #97 is
>    about *honest* clocks ordering on the wrong axis; F1 is about a *dishonest local* clock inverting a
>    causal edge.
> 3. **Disaster recovery fails outright on replacement hardware.** The box a node is restored onto is,
>    very often, exactly this Pi: fresh, RTC-less, booting at its `fake-hwclock` time. `restore_node_event`
>    (`db/009`) refuses every medium event more than 24 h ahead of that clock, and *any* refusal aborts the
>    whole restore. So restoring an honest medium onto a box whose clock is behind fails until a human sets
>    the clock, at the moment the clinic has just lost its node.
>
> 4. **The node can refuse its own clinical writes** (found while correcting this eval, not yet reproduced).
>    The orchestrators stamp their HLC from `node_hlc_tick` (db/007). When the local clock is behind
>    `hlc_state`, that function returns `hlc_state`'s wall plus a counter step. Take a node that has run at true
>    time, or synced honest events, and whose clock then *regresses*: the RTC battery dies, and the Pi reboots
>    decades off. Its `hlc_state` is now more than 24 h ahead of `clock_timestamp()`, so every event it authors
>    carries a wall that `submit_event` step 1a (`db/005`) refuses as more than 24 h ahead of local time. That
>    is a **write-availability** failure on the node's own door, the one thing ADR-0001 says must never stop.
>    `cairn_clock_health` (db/040) already detects the state, since `status` reports the RTC behind the HLC,
>    but nothing acts on it. A node whose clock was wrong from first boot, and which has never merged a
>    later wall, is not affected. Nor is a `fake-hwclock` restart, which resumes near its last shutdown.
>
> The code comment calls the relaxation "intentionally relaxed for a Byzantine future-claim". F1's peer
> is not Byzantine. The bound cannot tell the two cases apart, which is exactly CockroachDB's point.

**An AP-shaped remedy, in the order I'd do it:**

1. **Measure offset at every sync handshake, and keep the evidence.** Each pull already sees a peer's
   HLC and wall. Record `(peer, observed_offset, at)`. When **a quorum of distinct admitted peers** (not
   one) agree that *this* node is behind by more than the bound, the honest conclusion is "my clock is
   wrong":
   - downgrade the clock grade (this is issue #282's missing detector, and it works *offline-with-peers*,
     without any anchor plane);
   - raise the clock-sanity alert (#281);
   - judge every drift ceiling against **peer-median-corrected time** instead of raw
     `clock_timestamp()`.

   Requiring several independent peers keeps the ratchet defence: one hostile peer cannot move the
   median, and Roughtime/Marzullo use the same multi-source argument ADR-0027 already adopts. This
   step alone also fixes the case with **no explicit causal edge**: a new demographic value entered on
   the skewed node, which can only rank correctly once the node's clock does.
2. **Then, explicit causal edges tick past their targets.** When a local event carries `refs` (or
   overlays a target), mint its HLC as `max(tick(), target.hlc + 1)`, bounded by the peer-corrected
   ceiling from step 1. This is the HLC *receive* rule applied to a causal edge the author actually
   followed.

   > [!CAUTION]
   > *Correction (2026-11-03).* This eval's first draft put this step first and called it "bounded by
   > construction". **That was wrong.** On its own, a clinician adding to a Byzantine far-future target
   > (admitted by the clinical door's clamp) would carry the poisoned wall into this node's clock: the
   > ratchet #102 exists to stop. It is safe only *after* step 1, when the ceiling no longer depends on a
   > clock that may itself be wrong.

   > [!WARNING]
   > **Not covered: consequence 4.** Steps 1 and 2 correct skew only with evidence from peers. A node that
   > refuses its own writes after its clock regresses is most likely the offline one, with no peers to ask,
   > so this remedy leaves it refusing indefinitely. The local submit door needs a rule of its own that does
   > not depend on peers. For example: judge a node-*ticked* body against `GREATEST(clock_timestamp(),
   > hlc_state)`, which is only safe if `submit_event` never merges a submitted wall forward. Or exempt the
   > node's own ticks outright. Either way it needs a written no-ratchet argument. **Open**, for #935.
3. **Never self-terminate.** This is the one place Cairn must deliberately *not* copy CockroachDB. A CP
   database can afford to kill a skewed node; an EHR node must keep writing (ADR-0001 availability
   floor, principle 4). The AP translation of "self-terminate" is *downgrade the grade, widen the
   interval, say so on screen, keep working*.

### 3.2 Uncertainty intervals belong on the read side

**CockroachDB.** The uncertainty interval exists to *refuse to pretend*. A value whose timestamp falls in
`(t, t+ε]` is "maybe before, maybe after", and the system will not order it by guesswork. It restarts
instead. CockroachDB also caps the cost: with *observed timestamps*, after the first read on a node the
uncertainty limit there collapses to that node's clock, so at most one restart per node.

**Cairn today.** ADR-0027/0058 already give every `t_recorded` a graded interval. Cairn is ahead of
CockroachDB here: its uncertainty is *per event and graded*, not one global ε. But the interval is used
only on the *write* side (the `t_effective` ceiling). Winner projections still order by the HLC point,
with ADR-0045's collation-independent tiebreak. Take two self-asserted clocks five minutes apart: two
demographic assertions made "at the same time" on two nodes get a deterministic winner. **Deterministic
is not the same as true.** That is a precise untruth, the thing principle 4 forbids.

**Transfer.** When two competing assertions' `t_recorded` intervals **overlap** (at their born or
upgraded grades), the projection should surface them as **concurrent: order unknown** ("two recent
addresses, order uncertain"). It should not silently pick one. Keep the deterministic tiebreak so all
nodes still converge on the *display order*, but flag it.

This is CockroachDB's restart, translated: Cairn cannot "retry at a higher timestamp", but it can
decline to claim an order it doesn't have. It composes with #97: once recency moves to `t_effective`,
the same overlap test applies to effective-time intervals. Today every node mints at
`self-asserted`/`unknown` (ADR-0058), so W(grade) is open above. The rule needs a finite W for
self-asserted clocks to bite, which is itself a decision to take, not assume.

### 3.3 Resolved timestamps: from "freshness" to a completeness claim

**CockroachDB.** A rangefeed's **resolved timestamp** is a promise: *nothing new at or below T will ever
appear for this span* (`RFCS/20170613_range_feeds_storage_primitive.md`, `RFCS/20180501_change_data_capture.md`).
Delivery is at-least-once with no cross-row ordering. Consumers buffer between resolved markers and act
only on what is below one. Bounded-staleness reads use the same frontier to pick the freshest
*complete* local snapshot.

**Cairn today.** Honest assembly (sync §6.2, ADR-0004) has a freshness indicator ("last synced with
parent 4 h ago"). It wants to surface **known-missing** parts "when it can detect them". The
notification inbox may never claim "inbox zero" across a partition.

**Transfer.** A global closed timestamp is impossible in Cairn: an offline node legitimately writes
events that arrive weeks later "in the past". But a **per-link, per-origin frontier** is possible, and
it is exactly what honest assembly is reaching for:

- A peer can assert, signed, at the end of a pull: *"for scope S, you now hold everything I hold as of my
  HLC T"*. The chart can then say **"complete with respect to ED-server up to 14:02; unknown for nodes
  not heard from since …"**. That is a stronger and still-honest claim than a sync age.
- **Gap detection needs a contiguous per-origin counter.** UUIDv7 + HLC cannot reveal a *missing* event.
  An additive envelope field such as a per-`(origin node, patient)` monotone sequence would let any
  holder detect "events 41–43 from node X for this patient are missing". That is the "parent advertised
  5 episodes, only 3 arrived" case done mechanically. *Cost to weigh:* it must be inside the signed bytes,
  it reveals per-patient activity counts to every holder, and the "can't-retrofit" clock is ticking only
  in the sense that events minted without it can never be gap-checked. A candidate ADR, not a
  recommendation yet.
- Consumers that *act* (notifications, the FHIR façade, the essential-tier projector) should act on what
  lies below a frontier, the way changefeed sinks wait for a resolved marker.

### 3.4 Admission control: one governor for all elastic work

**CockroachDB.** They learned Cairn's imaging lesson the hard way. Index backfills starved foreground
traffic, and the fix was not priority alone (`tech-notes/admission_control.md`,
`tech-notes/replication_admission_control.md`):

- Work is classed **regular** or **elastic** (backfills, TTL deletes, restores, schema-change rewrites).
- Elastic work is admitted only while no regular work waits.
- Elastic work is paced by **tokens recomputed from the actual bottleneck resource** (LSM L0 health,
  provisioned disk bandwidth), not a static rate.

**Cairn today.** ADR-0013 / sync §6.6 get the principle exactly right for **attachment bytes**:
"chunked, preemptible, separately budgeted … not merely priority-ordered". But a Pi node has several
other elastic workloads that can grind a clinic to a halt in just the same way, and nothing names them
as one class:

- the paged full sync sweep (cairn-sync `FULL_SWEEP_EVERY`)
- nightly capture to the cold-peer medium (issue #552: O(whole medium), crossing its 2 s budget at
  ~23 k events)
- generic reprojection after a code-plane upgrade (ADR-0057), which on a 50 k-patient Pi (deployment
  §8.1) could be very large
- re-adjudication of deferred events (#912)
- the hub's aggressive background duplicate sweep (ADR-0014)

**Transfer.** Name an **elastic-work class** with **one governor** per node. It should yield to any
foreground clinical query and pace by the measured bottleneck (SD-card IO latency and foreground query
latency on a Pi; the link on a sat uplink), not a fixed rate. Then "blob transfer must never reduce
clinical-data availability" becomes "**no elastic work may**", which is the same law with a wider
reach. Fit-for-purpose code (§9.1); it doesn't touch the floor.

### 3.5 Test the distributed system as a system

**CockroachDB.**
- **kvnemesis** issues random concurrent KV operations and then *validates* the observed history
  against the claimed isolation level.
- **Jepsen/Elle** runs nightly.
- **roachtest** injects partitions, node kills and disk faults
  (`tech-notes/roachtest-investigation-tips/synthetic-failure-injection.md`).
- They model-check in **TLA+** only the **two** protocols that are both critical and subtle: parallel
  commits, and store liveness, which models message reordering and *clock regression on restart*.

**Cairn today.** It has excellent *per-door* testing: generative attacker-shaped bodies against the live
floor (`tests/floor_properties.rs`, proptest), and pairwise apply-order convergence for specific
projections (`tests/demographics_sex_gender.rs`, `tests/demographics.rs`). I found **no whole-system
randomised convergence harness**. Several of the known open divergences were found by review and not by
a test that would have caught them generically: node plane vs clinical plane door semantics (#268,
#301, #626), the actor-registry restore discarding a conflict (#569), and F1 above.

**Transfer: a "syncnemesis" harness.** *N* nodes (schemas or clusters on the test Postgres) and a random
schedule of:
- authored events across the closed algebras: clinical events, overlays/suppressions, identity
  link/unlink/repudiate, actor enrol/rotate/revoke
- random partitions and heals
- sneakernet medium hand-offs and destroy-and-restore (ADR-0026)
- **clock skew including dead/behind RTCs** (it would find F1)
- **version skew**: some nodes lacking a type's classification (ADR-0056)

After full heal, assert:
1. identical event sets on every node (set-union convergence);
2. identical projections among nodes on the same code;
3. no authored event lost anywhere (custody is total);
4. no power granted before re-adjudication (`cairn_replay_eligible`);
5. no watermark wedged (every verifiable event applied, or penned and acked).

The harness plan turns these five into its I1–I5 and adds **I6a/I6b**: honest causal order is never inverted,
and the #102 ratchet defence holds. These are the invariants clock skew violates, and the ones that show up F1.

Shrinking gives minimal counter-examples. Then, CockroachDB-style, put **TLA+ only on the one or two
protocols that are both safety-critical and liveness-subtle**. The best candidate is the quarantine-floor
/ re-offer watermark: *every verifiable event is eventually applied or recorded-acked; nothing is
silently dropped*. Admit-and-dispute registry adjudication (ADR-0054) is a second. TLA+ tools are MIT
— fine.

### 3.6 Version gates for behaviour, keyed to data rather than cluster state

**CockroachDB.**
- Cluster versions are monotone.
- Nodes observe at most X and X−1.
- Features ship as two gates ("prepare storage" then "enable").
- A binary outside its supported window refuses to run.
- Finalization needs **every** node reachable (`tech-notes/version_upgrades.md`,
  `RFCS/20200513_long_running_migrations.md`).

That last requirement is convergent lockstep, which ADR-0012 correctly forbids. Most of this does not
transfer, and Cairn's unbounded skew tolerance is the stronger design for its fleet. Two parts do map:
- Cairn's **strict-submit / lenient-apply** (ADR-0051/0056: a node may *carry* a type before it may
  *author* one) is the "prepare then enable" split.
- The window's schema-stamp refusal (ADR-0084) is "binary outside its window refuses".

**The gap it exposes.** sync §6.1 states it honestly: a **winner-rule change alters displayed clinical
state fleet-inconsistently, with no event recording it**. CockroachDB never has that problem, because a
behaviour change flips at one cluster version everywhere. Cairn cannot flip anything everywhere, but it
can make the rule a function of the **data** rather than of the node's upgrade date. ADR-0012 already
does this for constraints ("scoped going-forward: binds events recorded under schema ≥ X").

Apply it to projection rules: a signed, distributed *rule-epoch* event ("winner rule R2 governs
comparisons among events with `t_recorded ≥ T`"). Then every node *that has the code* computes the same
display for the same events, whenever it upgraded. A node without the code shows the old rule plus an
honest-assembly flag ("display rule outdated on this node"). This turns an admitted silent divergence
into a visible, bounded one.

### 3.7 Retention: don't let a stalled holder pin the world, and never collect the last copy

**CockroachDB.** **Protected timestamps** let a consumer (backup, changefeed) block MVCC GC. The
documented failure mode is that a *stalled* consumer's protection grows garbage for hours or days and
degrades serving traffic (`RFCS/20191009_gc_protected_timestamps.md`). **Decommissioning** is governed by
"at all cost, no data can be lost". Replicas are drained before a node leaves, and fully decommissioned
nodes are later *refused* on rejoin (`RFCS/20170608_decommission.md`).

**Cairn analogues:**
- **The quarantine floor pins the sync watermark** (sync §6.3), deliberately. The pen has **no expiry
  today**, and the ordering-transient case keeps the cycle loud until its target lands. That is fine,
  but it is the same shape as a protected timestamp. Surface the *age* of the oldest pin as an
  honest-assembly fact, so a stalled pin is visible before it becomes a months-old fixture. CockroachDB's
  "protection verification" step is a reminder that a pin you can't see is worse than one you can.
- **Surplus-copy GC** (ADR-0004 consequences: a "real, if non-urgent, parent responsibility", not yet
  designed). Adopt decommissioning's rule as the design's first invariant: **a copy is surplus only when
  ≥ k other custodians have acknowledged holding those exact event digests**. Never infer surplus from
  scope alone. Otherwise lazy revocation can GC the only copy of a workstation's unsynced events.
- **Rejoin.** CockroachDB *refuses* a decommissioned node. Cairn must not refuse its **history**, because
  custody is total: a dead workstation found in a cupboard still uploads the events nobody else has.
  Cairn may withhold its **future** authority (ADR-0064, "admit the claim, withhold the power", and
  ADR-0026's supersede). Cairn already has the better rule here; the lesson is to make sure the
  surplus-GC design honours it.

A note on erasure. CockroachDB has **nothing** per-key: encryption at rest is per store, and
fine-grained encryption is out of scope. Its RFC does admit that old data keys survive in "rarely-updated
files" that compaction never rewrites. The Postgres analogue for Cairn's crypto-shred (ADR-0005) is a
shredded DEK lingering in dead tuples before VACUUM, in WAL/archive, and in older medium generations.
ADR-0005 rung 4 already *declares* oblivion best-effort; the lesson is to name those physical remnants
in the erasure ceiling's prose.

---

## 4. Validations (Cairn already chose the better path)

| CockroachDB mechanism | Cairn equivalent | Note |
|---|---|---|
| `REGIONAL BY ROW` home region + `ON UPDATE crdb_region = gateway_region()` re-homing, "an American user inserts in Europe then travels back" (`RFCS/20210722_on_update_and_row_rehoming.md`) | ADR-0004: *nobody owns the record*; a transfer reassigns nothing, it adds an interest | Their fix *moves ownership*, a mutation needing an authority. That is precisely what ADR-0004 rejected so that ED→ICU works mid-partition. |
| GLOBAL tables: read-mostly data replicated everywhere, at the cost of commit-wait (~130 ms) | ADR-0016 replicated essential tier; reference data | Same idea (replicate small, read-everywhere data widely), without paying a consistency tax, because Cairn's essential tier is append-only overlays. |
| Bounded-staleness reads "for availability during partition" | Honest assembly + local projections (sync §6.2) | CockroachDB offers it as an *opt-in, single-statement, enterprise* read mode. In Cairn it is the default and only mode, and it is disclosed. |
| Loss-of-quorum recovery: named "*Inconsistent* Recovery", `unsafe-remove-dead-replicas`, leaves a marker on the store, warns of "false safety" | Restore ceremonies (ADR-0026, 0067–0072); operator `ack` of penned events | Same honesty discipline, applied more strongly. Cairn's restore *exits incomplete* rather than pretending (ADR-0071). Keep every operator escape hatch leaving a durable, visible marker. |
| Online schema change after Google's F1 paper (two-version descriptor leases; *unrelated to defect F1 in §3.1*) | Additive-only DDL, per-node schema, blue-green extension (ADR-0012) | Not needed: Cairn's DDL is node-local. One small borrow: build a new projection in a shadow, switch reads only when caught up ("backfill → public"), so a Pi mid-reprojection never shows a half-built chart. Worth checking ADR-0057 does this. |
| Encryption at rest: per-store keys, weekly data-key rotation | ADR-0005/0052 per-episode DEKs, born-sealed bodies, crypto-shred | Cairn is far ahead; nothing to borrow. |

---

## 5. Follow-ups (accepted 2026-11-03; filed under epic [#933](https://github.com/cairn-ehr/cairn-ehr/issues/933))

The epic gives the order and the reasons for it: the harness comes first, because every later item changes
convergence semantics and needs a system-level test to show set-union survived.

| § | Follow-up | Issue |
|---|---|---|
| 3.5 | Syncnemesis harness (plan: [`2026-11-03-syncnemesis-harness.md`](../superpowers/plans/2026-11-03-syncnemesis-harness.md)) | [#934](https://github.com/cairn-ehr/cairn-ehr/issues/934) |
| 3.1 | F1 — judge skew against peers; reproduction tests first; ADR refining 0027/0058 | [#935](https://github.com/cairn-ehr/cairn-ehr/issues/935) |
| 3.7 | Age of the oldest quarantine pin as an honest-assembly fact | [#936](https://github.com/cairn-ehr/cairn-ehr/issues/936) |
| 3.2 | ADR — overlapping intervals read as "order unknown" | [#937](https://github.com/cairn-ehr/cairn-ehr/issues/937) |
| 3.6 | ADR — data-keyed rule epochs | [#938](https://github.com/cairn-ehr/cairn-ehr/issues/938) |
| 3.3 | ADR — per-link completeness frontiers and an optional gap counter | [#939](https://github.com/cairn-ehr/cairn-ehr/issues/939) |
| 3.4 | One elastic-work governor | [#940](https://github.com/cairn-ehr/cairn-ehr/issues/940) |
| 3.7 | Surplus-copy GC invariant (≥ k acknowledged custodians) | [#941](https://github.com/cairn-ehr/cairn-ehr/issues/941) |
| 3.5 | TLA+ model of the quarantine floor | [#942](https://github.com/cairn-ehr/cairn-ehr/issues/942) |
| 3.7, 4, Q4 | Prose/check batch: erasure remnants, ADR-0057 shadow build, HA hub signing identity (Q4 below) | [#943](https://github.com/cairn-ehr/cairn-ehr/issues/943) |

### Q4 — HA hub identity

*(Open. Carried by #943. The other questions of this eval's first draft became the issues above.)*

A Patroni-style HA pair at the hospital core is one *logical* node. How does it hold **one signing identity
across failover**, when ADR-0026 wants that key hardware-bound and non-extractable? Two options are visible:
- a key that both members can use, which weakens hardware-binding;
- a key per member, which makes the hub **two peered nodes** to the fleet behind one logical service, each
  authoring under its own identity.

  `supersede` (ADR-0026) does **not** fit here, though it looks like it should. It is a one-way, dead-node
  replacement: the superseded identity's *future* authority is withheld (ADR-0064). After a failback, the
  recovered member would be admitted but powerless, and superseding back would be a cycle the algebra does
  not model. So the per-member option is peering, or a new "HA set" identity construct; it is never
  supersede per failover.

Neither CockroachDB, whose nodes are interchangeable cluster members with no per-node signing identity, nor
the spec answers this today. It is worth a sentence in deployment §8 before anyone builds the pair.
