# ADR-0026 — Node durability and disaster recovery: backup-as-cold-peer, new-identity restore, and shred-aware backups

- **Status:** Accepted; decisions 1 and 3, for progress-note drafts partly superseded by [ADR-0087](0087-the-first-note-stream-and-its-draft.md)
- **Date:** 2026-06-20
- **Refines:** [ADR-0005](0005-erasure-key-custody-and-crypto-shredding.md)

## Context

The spec carries a great deal of machinery for **deliberately** destroying trust material — the
[ADR-0005](0005-erasure-key-custody-and-crypto-shredding.md) crypto-shred ladder, the
[§7.1](../security.md#71-erasure-the-severity-ladder) keystore-is-safety-critical posture, the rule that *"keys
must not be silently reconstructable from ordinary DB backups after destruction."* It says almost nothing about
the **symmetric accidental case**: a node's disk dies. What survives?

The only disaster-recovery answer anywhere in the spec is one row of the
[sync §6.3](../sync.md#63-failure-modes-designed-for) failure table — *"Node destroyed → re-provision from
parent; only unsync'd local events are at risk."* That has two holes the mission cannot live with:

- **It assumes a parent exists.** The [ADR-0017](0017-federation-admission-sovereignty-peering-and-trust-anchors.md)
  **sovereignty floor** says a node needs no permission to run alone — the genuinely solo rural clinic on a
  satellite link is a first-class deployment, not a degenerate one. For that node replication provides **zero**
  durability: its only copy is local, and a dead disk is total loss.
- **It explicitly excludes the trust material.** The keystore is off the sync plane by construction
  ([§7.5](../security.md#75-the-actor-registry-enrollment-version-pinning-and-key-custody)): the private signing
  key is node-bound, the DEK store is separate and destroyable. So even a *federated* node, whose clinical events
  are durable through peers, never receives its **keys** from anyone. A dead disk means the node can no longer
  sign as itself, and any body whose DEK lived only there becomes permanently keyless noise — *the exact outcome
  crypto-shredding is designed to produce, by accident.*

Stated plainly: **the spec designed intentional key-death as a first-class, audited, irreversible operation and
left accidental key-death — and key survival — completely undesigned.** This is a paper-parity violation
([principle 3](../index.md#founding-principles-the-lens-for-every-decision)) in waiting: a paper chart survives
the computer dying; a Cairn node's keystore currently does not. And the recovery *shape* (where keys live, how a
node is reconstituted, how it interacts with erasure) is **day-one and can't be retrofitted** onto an
append-only log, like the [ADR-0013](0013-attachments-content-addressed-lazy-blob-tier.md) attachment-reference
shape.

The forces collide: durability (survive hardware loss) vs. erasure (a backup must not resurrect an erased body)
vs. anti-capture ([principle 7](../index.md#founding-principles-the-lens-for-every-decision) — no mandatory
cloud) vs. paper-parity (a physical, possession-based off-site analogue). Three realizations make the design
fall out of existing canon rather than introduce a new subsystem:

- **A backup is just another replication peer.** The medium holding a node's signed event set is a normal Cairn
  peer; restore is set-union apply through the existing verify-on-apply path. The backup inherits the sync trust
  model for free.
- **Recovery is the actor algebra.** A restored node is a *new* actor, `supersede`-linked to the dead one
  ([§7.5](../security.md#75-the-actor-registry-enrollment-version-pinning-and-key-custody)) — so the private
  signing key never needs to exist in a backup at all.
- **Erasure survives DR because crypto-shred is already an append-only event.** A restore replays the shred log;
  shred *completion* includes propagation to node-controlled backups. A backup can no more silently defeat
  erasure than a sibling node can.

## Decision

Specify **node durability and disaster recovery** as the append-only, key-custody, paper-parity, and
acknowledged-uncertainty principles applied to the worst case (total hardware loss). Canonical home:
[security §7.10](../security.md#710-node-durability-and-disaster-recovery), with mechanism notes in
[data-model §3.8](../data-model.md#38-erasure-and-key-custody) and [sync §6.2](../sync.md#62-consistency-model)/[§6.3](../sync.md#63-failure-modes-designed-for).
**No new founding principle** — it is principles 1/2/3/4 applied to DR. It adds **one can't-retrofit, day-one
requirement** (the recovery-secret escrow and the sealed local-state export must exist at provisioning).

1. **An explicit loss model and an honest guarantee.** On total hardware loss of a solo node, restored from the
   sealed medium plus its recovery secret: the **clinical event log survives** (verified on apply; RPO = last
   stream to the medium); **projections are rebuilt** ([ADR-0001](0001-fat-postgres-thin-daemon.md), never
   stored); **node-default data-at-rest keys survive** (else every ordinary body is noise, and a solo node has no
   peer to re-supply them); **sealed-episode DEKs survive minus any erased ones** (the shred log is replayed —
   point 6); the **private signing key deliberately does not survive** (point 4); **federation credentials are
   dead** → re-peer; **machine identity + PRNG seed are regenerated** ([data-model §3.2](../data-model.md#32-identity-time));
   **drafts and config survive** via the sealed export. The honest guarantee mirrors the
   [ADR-0005](0005-erasure-key-custody-and-crypto-shredding.md) erasure ceiling's two load-bearing hedges: a
   restored node recovers every event it had captured (verifiably), reads every non-erased body, honors every
   erasure, and returns to service under a new supersede-linked identity. The **declared, bounded losses** are
   (a) events written after the last capture, (b) the dead node's ability to sign as itself, and (c) — if the
   recovery secret is *also* lost — everything, because we will not pretend an encrypted artifact whose key is
   gone is recoverable.

2. **Clinical events back up as a cold peer.** Not a new subsystem — a configuration of the existing sync daemon
   ([ADR-0001](0001-fat-postgres-thin-daemon.md)) whose peer is a local, always-attached, encrypted volume
   instead of a network node. It set-unions signed events onto the medium continuously, so the medium holds a
   normal Cairn event set with nothing backup-specific about it. Restore is set-union apply through the **existing
   verify-on-apply path**: a tampered or bit-rotted medium fails the same signature / content-address invariants
   that catch a malicious peer ([ADR-0015](0015-event-serialization-signatures-and-content-addressing.md)) — so
   there is **no separate "is the backup intact?" mechanism**, and the backup is self-verifying and
   tamper-evident by construction.

3. **Non-event trust material rides a sealed local-state export.** The things that are not events and so cannot
   ride the cold peer — the data-at-rest keystore (node-default keys + sealed-episode DEKs), node config, and the
   draft/scratchpad store ([data-model §3.10](../data-model.md#310-session-identity-event-authorship-and-draft-durability)) —
   are written as an encrypted bundle into the *same* artifact. This is the **only component that touches private
   key material**, so it is the small safety-critical surface ([§9.1](../language-substrate.md#91-selection-rule-by-defect-blast-radius)).
   The **private signing key is excluded** (point 4): a stolen, unsealed artifact yields *read access but not a
   signing identity*.

4. **New identity on recovery, `supersede`-linked — the private signing key is never backed up.** A restored
   node mints a fresh keypair (ideally hardware-bound and non-extractable), and the registry records a
   `supersede` from the dead node-UUID to the new one — already in the closed actor-event algebra
   ([§7.5](../security.md#75-the-actor-registry-enrollment-version-pinning-and-key-custody): `enroll / supersede /
   revoke / suspend / rotate-key`), **no new mechanism**. The dead node's past events stay signature-verifiable
   forever (signing publics are immortal); the new node simply cannot sign *as* the old one, which is correct — a
   destroyed node is a new physical trust boundary. This **eliminates the most dangerous backup surface entirely**:
   a stolen backup cannot resurrect a node identity, and the scheme composes with non-extractable hardware keys
   (TPM / Secure Enclave) that *cannot* be backed up even in principle. The cost — re-running
   [§7.7](../security.md#77-federation-admission-peering-trust-anchors-and-the-custodian-contract) admission to
   re-peer — is rare, already a designed ceremony, and a no-op for a solo node. If a presumed-dead node later
   resurrects (false death), `supersede` ≠ kill: both identities are valid and reconcile via the actor algebra
   (`revoke` one) — never-merge-always-overlay, for nodes.

5. **The recovery secret is paper-escrow at the floor, pluggable upward.** A backup is encrypted, and the secret
   that unseals it must live **off** the node or it dies with the node. The floor — for the most isolated clinic,
   anti-capture-faithful, no cloud — is a **one-time printed recovery code / QR** sealed in the practice safe,
   optionally **Shamir M-of-N** split across trustees. Federated deployments may opt up to a hardware token or
   **peer-quorum (social) recovery**. Fractal, like the rest of Cairn
   ([principle 6](../index.md#founding-principles-the-lens-for-every-decision)). The escrow is generated **once at
   provisioning** (and on deliberate re-key) and is pre-positioned — it needs nothing at backup time. The
   secret's own survival is the **new single point of failure, named not hidden** (loss model point 1c); M-of-N
   and off-site copies are its mitigation.

6. **Erasure survives DR: shred-as-replayed-event, and shred completion includes backup propagation.**
   Crypto-shred is already a signed, append-only, syncing event ([ADR-0005](0005-erasure-key-custody-and-crypto-shredding.md)).
   A restore does not merely rehydrate the medium — it **replays the shred log and re-applies erasures before any
   data is projected**, destroying the named DEK during restore (so a body erased before the backup is never even
   projected). The honest path honors erasure automatically; resurrecting an erased body requires restoring while
   *deliberately ignoring* the shred log — the paper *"I shredded the document but kept a photocopy and lied,"*
   explicit malfeasance, not a silent default. The one sharp edge — an erasure performed *after* the last backup,
   whose body is on the medium but whose shred event is not — is closed by making **shred completion ⊇ backup
   propagation**: a crypto-shred is not "done" until the shred event has reached all node-controlled, attached
   backup media and the affected key material there is re-wrapped. Detached / offline / never-reconnecting media
   remain the declared honest ceiling — the same ceiling [ADR-0005](0005-erasure-key-custody-and-crypto-shredding.md)
   already names, now explicitly extended to backups.

7. **Backup health is a first-class honest-assembly fact.** A node that cannot currently back up is running
   without a net and must say so, exactly as the chart surfaces sync freshness
   ([§6.2](../sync.md#62-consistency-model)): *"last successful backup N h ago,"* medium full / detached / failing.
   Making the absence of a safety net **visible** is the same honest-assembly gain that surfaces known-missing
   record parts.

## Consequences

- **Easier.** A solo node — the sovereignty-floor deployment — finally has a durability story that does not
  require a parent, and it reuses the sync engine, the actor algebra, and the erasure stream rather than adding a
  subsystem. The backup is self-verifying for free (point 2). "Sealed body survives DR" becomes an **explicit,
  audited key-custody decision** ([ADR-0019](0019-author-scoped-record-export-the-medico-legal-copy.md) is the
  general mechanism) rather than a silent property of backups — the [ADR-0005](0005-erasure-key-custody-and-crypto-shredding.md)
  posture preserved. A stolen backup is read-only noise without the recovery secret and **never** a resurrected
  identity (point 4).
- **Harder / new trusted surface.** The sealed local-state exporter's key handling, the restore orchestrator's
  shred-replay + verify-on-apply gate + `supersede` minting, and the recovery-secret sealing are
  **safety-critical** ([§9.1](../language-substrate.md#91-selection-rule-by-defect-blast-radius), Rust / in-DB,
  reviewer-legible): a defect is silent data loss, an erasure defeat, or identity forgery. Backup cadence,
  medium-health surfacing, escrow rendering, and dashboards are fit-for-purpose. The recovery secret's survival
  is a real new operational burden — named, with M-of-N as the mitigation, but real.
- **The bet.** That the worst case decomposes cleanly into *events (durable, self-verifying via the sync model)*
  + *trust material (a small sealed export gated on an off-node, paper-first recovery secret)* — and that
  new-identity-on-recovery's re-peering cost is acceptable in every real deployment. We would know it is wrong if
  a deployment genuinely required a restored node to resurrect as its *same* cryptographic identity (e.g. a
  hub credential too costly to re-establish), which would be a signal to add a deliberate, audited
  key-escrow rung — never to put the signing key in an ordinary backup.
- **No new founding principle; no new event stream** (shred and `supersede` ride the existing planes). One
  **day-one, can't-retrofit requirement**: the recovery-secret escrow and the sealed local-state export must be
  established at the provisioning ceremony ([§7.6](../security.md#76-the-software-distribution-plane-signed-releases-and-extension-load)).
  Refines [ADR-0005](0005-erasure-key-custody-and-crypto-shredding.md); leans on
  [ADR-0001](0001-fat-postgres-thin-daemon.md)/[ADR-0011](0011-actor-registry-version-pinning-and-key-custody.md)/[ADR-0017](0017-federation-admission-sovereignty-peering-and-trust-anchors.md).
