# ADR-0004 — Dynamic sync scope: a prefetch hint, not an authority

- **Status:** Accepted
- **Date:** 2026-06-14
- **Supersedes:** —

## Context

Former open question [§11.3](../open-questions.md): a patient is transferred ED→ICU *mid-partition*;
who owns reassignment of the sync scope while the parent (which normally evaluates scope predicates,
[sync §6.1](../sync.md#61-mechanism)) is unreachable? Framed as ownership it is a hard
distributed-consensus problem. Case-mining dissolved the framing.

Two observations did it:

- **The record has no owner.** It is the sum of autonomous, signed parts written by different
  professionals at different places and times. It is *assembled* from those parts when it can be;
  clinicians rely on a best-effort assembly state. "Who owns the patient" is not a question the system
  needs to answer.
- **Paper-parity already answers the data plane.** On paper the chart travels *with the patient*;
  nobody phones medical records for permission to carry the folder upstairs, and records reconciles its
  index later. Making the parent's blessing a precondition for the ICU reading the chart would fail
  paper-parity during exactly the partition an AP system exists to survive.

The append-only design makes the safe answer free: acquiring a patient's events anywhere is
INSERT-only, idempotent, scoped set-union — there is nothing to merge and nothing to lose.

## Decision

**Scope is an administrative *prefetch hint*, not an access authority**
([sync §6.4](../sync.md#64-scope-is-a-prefetch-hint-not-an-authority)).

- A transfer **reassigns nothing**; it gives the receiving node *reason to assemble the patient*, so
  the node **acquires the parts** — from a sibling on the same LAN (the common
  internet-down/intranet-up case), carried with the patient (store-and-forward / sneakernet) in a
  total partition, or from the parent on reconnect. The parent **ratifies and audits**, never gates.
- **Granting scope is urgent and edge-authorized; revoking is lazy and parent-mediated.** A node never
  *moves* a scope (a mutation needing an authority); it only *adds* an interest. Surplus copies are
  garbage-collected later by the parent. (Asymmetry: lacking a needed chart is a safety/parity failure;
  holding a spare copy briefly is harmless.)
- **Access follows legitimate-need + audit, not pre-granted permission** — break-the-glass acquisition
  that is *recorded*, strictly better than paper's traceless folder.
- The surviving requirement is **honest assembly-state disclosure**
  ([sync §6.2](../sync.md#62-consistency-model)): the chart shows freshness, surfaces **known-missing**
  parts when detectable, and signals when it is partitioned and parts may exist beyond the island.

This softens [sync §6.1](../sync.md#61-mechanism): scope predicates govern *automatic* replication, not
*permitted* holding; "evaluated at the parent" is the online optimization, never the only path.

## Consequences

**Easier / gained:**

- The hard distributed-systems problem disappears: no partition-time consensus on scope ownership,
  because there is no ownership and no reassignment transaction.
- Care during a partition never waits on an authority — the bedside clinician reads/writes the
  locally-assembled chart immediately ([§1 availability](../vision.md)).
- Acquisition is the same safe set-union as all sync; the parent's role shrinks to ratify / audit /
  garbage-collect.
- Break-the-glass is auditable by construction — a privacy gain over paper.

**Harder / the bet:**

- At least a minimal **sibling/peer acquisition path** (intra-facility) is now *required* for the
  transfer case, where [topology §2](../topology.md) had filed peer sync as "a later extension".
  Hub-only purity is given up for partitions; the physical-carry bundle backstops the total-partition
  case.
- **Garbage collection of surplus copies** becomes a real (if non-urgent) parent responsibility, and
  interacts with retention/erasure ([§11.5](../open-questions.md)).
- Legitimate-need acquisition interacts with **visibility-scope / sensitive-episode** gating
  ([§11.8](../open-questions.md)): default-scope acquisition pulls the clinically-relevant set;
  sequestered episodes need a separate, separately-audited break-the-glass claim. That layering is
  deferred to §11.8.

**How we'd know it's wrong:** if surplus-copy sprawl or break-the-glass over-acquisition becomes common
enough to threaten privacy or storage on small nodes, the prefetch/GC policy — not the no-ownership
principle — needs tightening.
