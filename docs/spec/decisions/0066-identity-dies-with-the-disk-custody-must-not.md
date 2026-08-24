# ADR-0066 — Identity dies with the disk; custody must not

- **Status:** Accepted
- **Date:** 2026-08-24
- **Derives from:** [ADR-0026](0026-node-durability-and-disaster-recovery.md) decision 4 (*"New identity
  on recovery, `supersede`-linked — **the private signing key is never backed up**"*) and
  [ADR-0052](0052-born-sealed-clinical-bodies.md) decision 4 (*"**The node unwrap key is X25519**,
  derived from the node's Ed25519 signing seed by HKDF with `info = "cairn-node-unwrap-x25519-v1"`"*).
  **Each is sound alone; their composition is the defect** — confirmed in code on 2026-08-23 and pinned
  by `crates/cairn-node/tests/dr_clinical_guarantee_gap.rs`, a suite written to be inverted rather than
  extended.
- **Supersedes, in exactly one clause:** ADR-0052 decision 4's **derivation** clause, and nothing else.
  ADR-0052 is **not** superseded as a whole and is not reworded — it carries a marked
  **[Erratum E2](0052-born-sealed-clinical-bodies.md) (2026-08-24)** under decision 4 pointing here, which
  is the house mechanism for amending an immutable ADR. The rest of its decision 4 (public-half-
  only in the database, the `CTX_UNWRAP_KEY` unwrap-key certificate, mandatory KEK escrow, the wrapped-DEK
  sync sidecar) stands unaltered, as does every other decision in it. Its stated *rationale* for deriving —
  *"no new key-management mechanism"*, meaning **no second ceremony for the operator** — is **preserved,
  not overturned** (decision 1). **[ADR-0026](0026-node-durability-and-disaster-recovery.md) decision 4
  stands unchanged and gets stronger** (decision 2).
- **Applies:** principle 1 (append-only — a wrapped DEK is custody over an immutable artifact, so losing
  the key is the one way to lose a record without touching a row) · principle 4 (acknowledged uncertainty,
  and its corollary *deletion is best-effort and declared, never guaranteed* — read in the mirror: an
  **undeclared** loss is the defect here) · principle 9 (policy-neutral infrastructure — ADR-0052 made
  bodies born-sealed precisely so no erasure rung is foreclosed; a custody key that cannot survive DR
  forecloses **all** of them, by accident) · principle 12 (the floor is in the database — `node_unwrap_key`'s
  singleton registrar is what makes decision 4 *forced* rather than chosen) ·
  [ADR-0005](0005-erasure-key-custody-and-crypto-shredding.md) (erasure is redistribution of key custody —
  this ADR is the **accidental** case of the same mechanism) ·
  [ADR-0011](0011-actor-registry-version-pinning-and-key-custody.md) (key custody and the registry — the
  registry is the piece slice 2 must also carry) ·
  [ADR-0026](0026-node-durability-and-disaster-recovery.md) ·
  [ADR-0052](0052-born-sealed-clinical-bodies.md) ·
  [ADR-0056](0056-unknown-event-types-admitted-uninterpreted.md) (admit uninterpreted — the posture the
  restore door inherits when slice 2 builds it)
- **Canonical spec home:** [security §7.10](../security.md#710-node-durability-and-disaster-recovery),
  with the custody-lifecycle mechanism note in
  [data-model §3.8](../data-model.md#38-erasure-and-key-custody)
- **Closes:** [#495](https://github.com/cairn-ehr/cairn-ehr/issues/495) **at the decision level** (the code
  is the accompanying slice). It does **not** close
  [#500](https://github.com/cairn-ehr/cairn-ehr/issues/500) — see the warning after the decisions, which
  is the most important paragraph in this record.

## Context

[ADR-0026](0026-node-durability-and-disaster-recovery.md) decision 1 makes three promises about a
restored node's clinical tier: *"the **clinical event log survives**"*, *"**node-default data-at-rest keys
survive** (else every ordinary body is noise, and a solo node has no peer to re-supply them)"*, and
*"**sealed-episode DEKs survive minus any erased ones**"*. On 2026-08-23 all three were confirmed false
in the built system. This ADR decides the third; the warning below grades the other two rather than
letting them read as satisfied.

**The coupling.** One 32-byte seed is currently both the node's **identity** (Ed25519 signing) and its
**data custody** (the X25519 secret HKDF-derived from it). ADR-0026 decision 4 deliberately kills the
identity on recovery — *"a restored node mints a fresh keypair"*, and the old signing key is never in the
backup, which is exactly right: a stolen medium must never resurrect a signing identity. But a fresh seed
derives a **different** X25519 secret, so every `event_dek` row the restored node inherits is wrapped to a
public half whose private half no longer exists anywhere. The rows are intact, signed, and permanently
unopenable.

**The blast radius is the whole clinical record, and it grew silently.** When ADR-0052 made *every*
clinical JSONB body born-sealed, it converted this from an edge case affecting opt-in sealed episodes into
total loss of clinical content on restore. ADR-0026's own context names the outcome precisely: *"the exact
outcome crypto-shredding is designed to produce, **by accident**."* It is the loss mode ADR-0052 itself
named — *"KEK loss = whole-record loss"*, which is why it made escrow **mandatory** — arriving through a door
escrow does not cover, because the escrowed secret unseals a key that was **regenerated rather than restored**.

**Scope: solo, and that is not a hedge.** A **federated** node that re-peers does recover custody — the
serve arm re-wraps each DEK against the puller's current unwrap certificate. The **solo** clinic, the
[ADR-0017](0017-federation-admission-sovereignty-peering-and-trust-anchors.md) sovereignty-floor
deployment for which ADR-0026 says *"replication provides **zero** durability"*, has no such rescue. It is
the deployment DR exists for, and it is the one that loses everything.

**Both ADRs were internally consistent. The contradiction lived only where they meet, which is code.** No
document was wrong; no reviewer of either ADR alone could have caught it. Two findings are worth carrying
forward, and they are the reason this ADR exists at all rather than a one-line patch:

- *A deferral is only honest while its stated precondition holds, and nothing in the repo watches for one
  expiring.* `localstate.rs` declared its empty slots truthfully — *"the federation-node tier has no
  clinical surface yet"* — and ADR-0052 made that sentence false without reopening it.
- *Cross-ADR claims about the same key material must be checked at the seam, and the seam is never prose.*

> **Identity dies with the disk. Custody must not.**

## Decision

### 1. The node unwrap key is an independent X25519 keypair

The node's DEK-unwrap secret is **generated**, not derived. It has its own lifecycle: minted at
provisioning, held in its own sealed file at rest, inspected and reported independently of the signing
key.

**This supersedes ADR-0052 decision 4's derivation clause only, and preserves its rationale.** That clause
existed to avoid a *second operator ceremony* — *"the existing ADR-0026 op-passphrase + recovery-code
escrow already covers it — no new key-management mechanism."* That property is kept in full: the
independent key is sealed under **the same two secrets** (op-passphrase **or** recovery code), so the
operator still has exactly one escrow ceremony, one printed code, one safe. What is dropped is the
derivation, which bought nothing the sealing does not already buy and cost the record on every restore.

Everything downstream is unchanged by construction: the wrap/unwrap boundary cannot tell a generated
keypair from a derived one, the database still holds only the public half, and a DB backup still cannot
reconstruct a DEK.

### 2. ADR-0026 decision 4 stands unchanged — and becomes stronger

The private signing key is still **never** backed up. A stolen, unsealed export must still yield *read
access but not a signing identity* (ADR-0026 point 3's stated test), and a restored node is still a new
actor, `supersede`-linked to the dead one.

The property does not merely survive this ADR, it improves: **after this decision nothing else depends on
the seed surviving.** Before, decision 4's flat sentence was quietly load-bearing for something it never
mentioned, and honouring it destroyed data. Now it can stay flat — no narrowing erratum, no exception.
This is why *escrow the signing seed* is rejected below rather than treated as a trade-off: it would have
solved a data-loss hole by opening an impersonation hole.

### 3. The unwrap secret rides the `CAIRNL1` sealed export

It needs no new vehicle. [ADR-0026](0026-node-durability-and-disaster-recovery.md) point 3 already defines
the sealed local-state export for exactly this class of material — *"the things that are not events and so
cannot ride the cold peer"* — and already excludes the signing key. Its `episode_deks` slot is already
named for `event_dek` custody; its dual-recipient seal (op-passphrase **or** recovery code) is already the
escrow an unwrap secret needs; and it is written as a **sibling of the medium** on every backup, so it
shares the medium's fate.

The export therefore satisfies **point 3's** stated test unchanged — *"a stolen, unsealed artifact yields
**read access but not a signing identity**"* — the same sentence decision 2 cites, and the locus to quote when
citing this test anywhere (point 4 makes the related but differently-worded claim that *"a stolen backup cannot
resurrect a node identity"*). Someone holding the export plus a secret can read the dead node's records; they still cannot
sign as it. That is the boundary ADR-0026 drew on purpose, and this ADR keeps it exactly where it was.

The slot is **typed and additive** ([ADR-0012](0012-schema-evolution-event-format-and-legibility-across-time.md)):
an older bundle still reads, and an older *build* refuses a newer bundle **loudly** rather than dropping
key material silently.

### 4. A restored node adopts the exported unwrap key; it does not mint one

This is forced by the floor, not chosen for convenience. `node_unwrap_key` (`db/037`) is a **singleton
whose registrar refuses a differing key** — *"a different unwrap key is registered — rotation is a separate
ceremony (ADR-0052)"*. A restored node that minted a fresh unwrap key would be unable to register it
beside inherited custody rows, and if it could, every one of those rows would be orphaned anyway.

So restore **installs the recovered secret** and registers its public half, and the ordering that follows
is forced rather than stylistic: the key must be installed and registered before any custody row is
touched, because wrapping needs the public half present.

**No keyring exists, and none is needed.** One node, one unwrap key, adopted whole. The multi-key
machinery a mint-on-restore design would have required is avoided entirely — a structural simplification,
not a deferral.

### 5. Existing nodes adopt their currently-derived secret as their first independent key

A node provisioned before this ADR has `event_dek` rows wrapped to the public half of its *derived*
secret. Migration re-derives that secret **once** and adopts it as the node's first independent key. This
is **lossless**: no row is rewrapped, no `event_dek` is migrated, every sealed body keeps opening, and the
registered public half never changes — so the singleton registrar of decision 4 is satisfied without a
rotation ceremony that does not exist.

**The adoption path works only while the derived secret is still reconstructible**, i.e. while the node
still holds its original signing seed. That is what makes breaking the derivation cheap *today* and more
expensive every week: a node that has already been restored under the old coupling has no old seed, so
nothing here recovers its inherited DEKs. Stated in Known limitations rather than implied away.

The derivation function therefore survives — **as the migration path and nothing else**. It is contained
by a shouting doc comment and a **pinned production call-site list** (the `hex_decode_helper.rs`
convention), so a future edit cannot quietly re-couple identity to custody. *The count failing is the
guard working.*

### 6. Registering the public half is a provisioning act, not a write-path side effect

Registration moves to `init` / `establish-unwrap-key`. The sealed-write path **verifies** that a key is
registered and **fails loudly, naming the remedy**, when one is not.

Two reasons, and the second is the durable one. Mechanically, an independent key is a file the write path
has no business reaching, and threading it through would cascade across every seal-and-submit call site.
Substantively, *"register whatever key this signer implies, on first write"* is the **same coupling one
layer up**: it makes a node's custody key an implicit consequence of who happened to sign first, when a
custody key is a **provisioned fact about the node**. A node with no registered unwrap key would write
sealed bodies it could never crypto-shred — foreclosing the erasure ladder ADR-0052 exists to keep
reachable — so refusing beats degrading. A refusal an operator cannot act on is not a safety control,
which is why the message names the command.

### 7. The export excludes a shredded event's DEK

The export carries `event_dek` rows **wrapped, verbatim** — no raw key material ever lands in it — and it
carries **nothing for an event already crypto-shredded**.

[ADR-0026](0026-node-durability-and-disaster-recovery.md) states the requirement in its **Context** — *"A backup
can no more silently defeat erasure than a sibling node can"* — and implements it in **point 6**
(shred-as-replayed-event, shred completion ⊇ backup propagation); the sentence is not itself in point 6, so
look for it one section up. [ADR-0052](0052-born-sealed-clinical-bodies.md) decision 6 makes a shred destroy
the custody row. **A key that never crosses the restore boundary cannot be
resurrected by one**, which is stronger than replaying the shred log after the fact and does not depend on
replay ordering. The two mechanisms compose: the export omits the key, and the shred log still replays.
This asymmetry — the survivor's DEK **must** be present, the shredded event's **must never** be — is the
load-bearing property of the export's tests.

> [!WARNING]
> **What this ADR does *not* make true. Read this before citing the DR guarantee anywhere.**
>
> - **The backup medium still carries no clinical event.** It exports the **federation plane** only
>   (`node_event`: enroll, peer, revoke, supersede). A restored node recovers who it peered with and
>   **zero patients**. That is [#500](https://github.com/cairn-ehr/cairn-ehr/issues/500), it is the next
>   slice, and **this ADR does not fix it.** Until it lands, this decision hands a restored node a working
>   key and nothing to open with it. Neither half is useful alone: the key without the bytes opens
>   nothing, the bytes without the key are noise.
> - **"Node-default data-at-rest keys survive" has no subject.** There is no node-default data-at-rest key
>   tier in the built system — only per-event DEKs. The export slot exists and nothing produces it. The
>   clause is neither honoured nor violated; it names a tier that would have to exist first, and it must
>   not be read as satisfied by anything here.
> - **`cairn-sync` still derives its own unwrap secret.** It has no **production** dependency on `cairn-node`
>   (only a dev-dependency, for tests) — a layering choice, not a structural impossibility — and so cannot
>   read the new keystore file. After this change a freshly-provisioned node registers an independent
>   key while the sync daemon derives a different one. The accompanying slice makes that divergence
>   **fail fast at startup** rather than degrade quietly — a serve arm that cannot open its own custody
>   looks exactly like a peer with no custody to offer, which is the silent failure this whole ADR is
>   about. Loud, not fixed: the real fix is a shared keystore path, and it is filed.
>
> An ADR that read as though the DR hole were closed would repeat the exact failure it was written about —
> *a document whose stated precondition has expired.*

## Paper-parity benchmark (§1.2)

The restore ceremony is an operator workflow on the clinical record's survival path, so it carries a
benchmark rather than the forced-rationale escape.

**Counterpart:** the **off-site duplicate chart** — the practice that photocopies its records into another
building and carries the box back after a fire.

| act | paper *N* | architecture *M* | UI target *K* |
|---|---|---|---|
| recover the record after total loss | 2 (fetch the box; shelve it) | 3 (attach the medium; run restore and answer its prompts; confirm the echoed identity) | 2 |

**`M > N`, so per house rule 7 this is filed as an architecture defect, not argued away.** The extra act
is the identity confirmation, which has no paper counterpart because a paper box carries no cryptographic
identity to mis-assign. Two notes for whoever picks it up: the escrow secret and the restore invocation
are **one** interactive ceremony rather than two acts, and a sole-enroll medium's provenance is already
unambiguous — the confirmation is forced only on the federated/unsigned paths, so *K* = 2 is reachable
without weakening anything.

**Budget:** a restore of a 100k-event medium completes in **≤ 10 min** unattended after the operator's last
keystroke, and the operator needs **one secret** (op-passphrase or recovery code) and **no knowledge of the
dead node's configuration**. Measurement is owed by the slice that first exposes a runnable end-to-end
restore — which is the [#500](https://github.com/cairn-ehr/cairn-ehr/issues/500) slice, not this one, since
a restore that recovers zero clinical events has nothing to measure. **If a measurement falls outside its
budget, that is the finding — file it; never adjust the budget.**

The ongoing cost is where the comparison favours the architecture and it belongs in the record: paper's
duplicate costs a photocopy per page, forever; this costs a nightly cron.

## Consequences

**Easier.**

- **The DR hole is closed at the key layer with no new artifact and no new ceremony.** Both vehicles
  already existed and are now used for what they were built for: the medium carries signed events, the
  sealed export carries node-local non-event material. The operator's ceremony count does not move.
- **ADR-0026 decision 4 needs no erratum and no narrowing.** *"The private signing key is never backed
  up"* can stay flat, because nothing else now depends on the seed surviving.
- **No keyring, no key hierarchy, no rotation dependency** — decision 4's singleton makes adoption the
  only coherent restore behaviour, which deletes a whole design space rather than deferring it.
- **The migration is lossless** — no rewrap, no `event_dek` migration, no downtime, and the registered
  public half never changes.
- **The write path gets an honest failure** where it had a silent side effect (decision 6), and
  `status` can report identity and custody independently, because they are now independent things.

**Harder / newly true.**

- **The sealed export becomes authorization-relevant in the next slice.** It already holds read power;
  when it gains the actor registry (`actor_event` is an unsigned, node-local table that never rides the
  event plane, and without it a restored node refuses every clinical event it just read back), whoever
  holds the export **and** a secret can stand up a node that trusts the dead node's actors. That is
  acceptable only because it is dual-sealed, physically co-located with the medium, and consumed through
  a door fenced to a fresh node — **mitigated, not eliminated**, and it must be stated to operators
  rather than discovered.
- **A second key file joins the escrow's blast radius.** Losing the escrow secret still loses the record;
  that single point of failure is ADR-0026's declared one, unchanged, and M-of-N remains its mitigation.
- **`cairn-sync` diverges until it shares the keystore** — loud, filed, and named in the warning above.
- **Unwrap-key rotation still has no path.** This design does not need it (restore adopts), but a node
  that *suspects* its unwrap key is compromised has nowhere to go, and the singleton registrar refuses a
  swap by design. Filed rather than absorbed.

**The bet.** That splitting one seed into an identity that is deliberately mortal and a custody key that
is deliberately durable is the whole fix — that read access and signing authority are genuinely separable,
and that a stolen export yielding the former without the latter is a boundary we can keep drawing. We
would know it is wrong if a deployment produced a case where possessing the export plus a secret is as
dangerous as possessing the signing key, at which point the answer is a stronger seal or an M-of-N gate on
the export, **never** returning custody to the seed.

## Rejected alternatives

- **Escrow the signing seed.** Backs the signing key up, so a stolen medium can forge events as that node.
  It trades a data-loss hole for an impersonation hole and contradicts ADR-0026 decision 4's central
  property (*a stolen backup cannot resurrect a node identity*), which also composes with non-extractable
  hardware keys that cannot be backed up even in principle.
- **Escrow only the derived secret, keeping the derivation.** Keeps ADR-0052 decision 4 literally true,
  but you are then persisting and escrowing an independent secret anyway — this decision with the
  derivation left in as decoration, and the coupling still sitting there for the next person to trip over.
- **Declare the loss.** Defensible before born-sealed; since ADR-0052 the blast radius is the entire
  clinical record, and ADR-0026's own framing already calls that *"the exact outcome crypto-shredding is
  designed to produce, by accident."* Principle 4 permits declaring a loss; it does not permit designing
  one in and calling the declaration a remedy.
- **Mint a fresh unwrap key on restore and rewrap.** There is nothing to rewrap *from* — the old secret is
  what is missing — and `node_unwrap_key`'s registrar refuses the swap. It also implies a keyring nobody
  needs.
- **Keep registering the public half from the write path.** Convenient, and the same coupling one layer
  up: it makes a node's custody key an implicit consequence of who signed first (decision 6).
- **Put the unwrap secret on the backup medium.** Breaks ADR-0026 decision 2's *"a normal Cairn event set
  with nothing backup-specific about it"* and puts private key material on the container whose whole
  security story is that it holds only signed, self-verifying events.
- **Widen the existing sealed signing-key file to carry both secrets.** The loader distinguishes a sealed
  bundle from a raw seed by shape; changing the sealed payload would break that detection and the
  plaintext-key path for no gain. A sibling file costs nothing and keeps each key independently
  inspectable.

## Known limitations

- **This ADR alone recovers nothing.** With the medium still carrying no clinical event
  ([#500](https://github.com/cairn-ehr/cairn-ehr/issues/500)), a restored node holds a working unwrap key
  and an empty clinical log. The guarantee is discharged only when both slices land, and the acceptance
  test is one test: **author a born-sealed clinical event, back up, destroy the database, restore, read
  the body back.** Anything less tests a component, not the promise.
- **A node already restored under the old coupling is not rescued.** Its inherited DEKs were wrapped to a
  secret derived from a seed that no longer exists. Decision 5's adoption path needs the original seed;
  where it is gone, the loss is real and unrecoverable. Stated, because a reader could otherwise take this
  ADR as retroactive.
- **The export's blast radius grows in the next slice** (see Consequences) — read power today,
  authorization-relevant state tomorrow.
- **Unwrap-key rotation remains unbuilt**, so a suspected-compromise path does not exist.
- **RPO is unchanged**: the last stream to the medium. Events authored after the last backup are gone, as
  ADR-0026 always said.
- **The byte tier does not travel.** Attachment blobs are not on the medium — the reference rides the
  event, the bytes do not ([ADR-0013](0013-attachments-content-addressed-lazy-blob-tier.md)'s design, not
  a defect) — so a restored node's renditions degrade to references-only and the operator must be told.
