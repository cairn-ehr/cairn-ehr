# ADR-0067 — A restore reads the clinical plane, and what that costs

- **Status:** Accepted
- **Date:** 2026-09-10
- **Closes:** [#554](https://github.com/cairn-ehr/cairn-ehr/issues/554) — *restore does not read the
  clinical plane back: the medium holds the record, nothing gives it to a node.*
- **Derives from:** [ADR-0026](0026-node-durability-and-disaster-recovery.md) (node durability and
  disaster recovery), [ADR-0052](0052-born-sealed-clinical-bodies.md) (born-sealed bodies and the
  wrapped-DEK sidecar), [ADR-0066](0066-identity-dies-with-the-disk-custody-must-not.md) (the
  independent unwrap keypair), [ADR-0005](0005-erasure-key-custody-and-crypto-shredding.md) (erasure is
  redistribution of key custody), [ADR-0001](0001-fat-postgres-thin-daemon.md) (fat Postgres, thin
  daemon).
- **Supersedes, in exactly one clause:** [ADR-0026](0026-node-durability-and-disaster-recovery.md)
  decision 2's **implementation wording**. See decision 4 — the mechanism changed, the guarantee did
  not.

---

## Context

ADR-0026 decision 1 promises that after total hardware loss of a **solo** node, restored from the
sealed medium plus its recovery secret, *"the clinical event log survives."* That sentence was false for
the entire life of the project until now, in two different ways, and each was fixed by a different
slice.

**The key.** Until ADR-0066 the node's X25519 unwrap secret was HKDF-derived from its Ed25519 signing
seed, and ADR-0026 decision 4 says the signing key is never backed up — so a restored node minted a
fresh seed, derived a fresh unwrap secret, and every inherited `event_dek` row was noise. Closed
2026-08-24 (#495).

**The bytes, write half.** The backup medium carried the federation plane and no clinical event at all.
Closed by DR slice 2c, 2026-09-06 (#500): the medium is now a CAIRNB3 image carrying every `event_log`
row with its wrapped DEK beside it.

**The bytes, read half — this ADR.** Nothing read one back. `restore` and `verify-backup` both went
through a reader that returns the federation plane on purpose, and the carried custody and
actor-registry rows were counted and not inserted. So a solo clinic backed up nightly, passed
`verify-backup`, lost its disk, and restored a node that knew who it had peered with and **zero
patients**.

The five decisions below are the ones that are architectural rather than local. Everything else about
the slice is implementation and lives in the code and its design document.

---

## Decision 1 — The actor registry re-enters on the export container's AEAD alone

Every clinical apply door resolves its author through `actor_current`, so a restored node cannot apply
a single clinical event until its `actor_event` registry is back. The registry rides the sealed
`CAIRNL1` local-state export and is installed through a dedicated in-database door.

**Those rows are authenticated by the export container's AEAD and by nothing else.** They carry no
per-row signature. The clinical events around them are each individually signature-verified by the apply
door; the registry is not. **This is the one part of a restore that is not verify-on-apply**, and it is
accepted deliberately: whoever holds the export *and* its passphrase or recovery code already controls
the restored node completely, so refusing here would cost the clinic its record and buy nothing.

Two consequences follow, and both are obligations:

- **It is fenced twice.** The door refuses an enrolled node (a live node is never a restore target) and
  refuses any registry holding a row that is not in the set being restored (a half-provisioned node,
  never the residue of an interrupted restore). It is granted to the node role and explicitly not to
  the advisory-actor role, because an advisory actor that could "restore" a registry could re-authorise
  itself through a door that deliberately does not re-adjudicate.
- **It is printed to the operator at restore time.** A limitation that lives only in a design document
  is a limitation nobody will find.

The door **replays** rather than re-adjudicates: it validates shape and inserts. Re-running the fresh-
enrollment collision guards would refuse this node's own legitimate `revoke` and `supersede` rows, every
one of which trips them by construction — prior registration history is exactly what they are.

---

## Decision 2 — Erasure does not propagate backwards into media already written, and completing it across backups is rotation

**A body crypto-shredded after a capture keeps its wrapped DEK on that medium, and that is correct.**

It looks like an erasure that failed to propagate. It is the definition of a backup: *a backup is only
a backup if it can restore the state of the system at the time the backup was taken.* At the moment that
medium was written the body **was** readable. A medium that dropped the key later would report a state
the node was never in, and it could only do so by rewriting a segment it has already signed — forfeiting
the integrity guarantee that is the core's job.

**Never filter old segments.**

The mirror half is equally deliberate: a body shredded *before* its first capture never has its DEK
written, while its ciphertext still travels. **A shred destroys the key, never the event** (ADR-0005).
A restore must therefore key its custody decisions on **whether a record carries a DEK**, never on
whether the event is sealed — the two are different questions, and keying off sealedness would quarantine
a legitimately shredded body forever over a key that does not exist and is not supposed to.

**What the core does not do, and what a practice must be told.** Completing an erasure across backups is
**rotation**: capture fresh, destroy old. **That rotation interval IS the maximum time an erasure takes
to complete across all copies.** It is the clinic's policy call, not Cairn's — principle 9
(policy-neutral infrastructure: mechanism, never policy), and ADR-0005's corollary that *deletion is
best-effort and declared, never guaranteed*. Cairn ships the mechanism and states the interval's meaning;
the clinic chooses the number and owns the consequence.

---

## Decision 3 — The restore ceremony's order is load-bearing

```
mint the new key
  → apply the node plane            (the self-trusting door; un-enrolled fence)
  → install custody + the registry  (the unwrap key is registered here)
  → apply the CLINICAL plane
  → finalize_identity               (LAST — mints the genesis and fences everything closed)
```

Each position has its own reason, and they are not interchangeable:

- **Custody before the clinical apply**, because the apply door wraps each event's DEK to the
  **registered** public half. With no registration there is no custody to write.
- **The registry before the clinical apply**, for a different reason: without it the door refuses this
  node's own history as unenrolled — the zero-patients outcome wearing a different costume.
- **Identity last**, so the whole restore runs inside the un-enrolled fence. A clinical apply that fails
  catastrophically then leaves a database with **no genesis written** — still legitimately restorable,
  from the same medium, into the same database. Under the minimal alternative (identity where it was,
  clinical after it) the same failure leaves a node already identity-minted and already fenced, whose
  only recovery is a fresh database.

**That last property is true only because the registry door is resumable.** The registry is installed
*before* the clinical apply, so a failed attempt leaves the fence open and the registry populated. A door
refusing on "any registry row present" would turn away the very re-run this ordering exists to enable.
Resumability is therefore a **requirement of the ordering**, not a convenience of the door — which is
why the door is set-shaped rather than per-row.

The order rests on two preconditions that were verified rather than assumed, and are **pinned by source
guards** because a future migration could break the ceremony silently: neither the clinical apply door
nor the unwrap-key registrar reads the enrollment row.

---

## Decision 4 — ADR-0026 decision 2's implementation wording is superseded

ADR-0026 decision 2 describes backup as *"a configuration of the existing sync daemon"* whose restore is
*"set-union apply through the existing verify-on-apply path."*

The built system is a **medium format plus a capture/restore command**, not a cold-peer daemon
configuration; and its registry re-entry is explicitly **not** verify-on-apply (decision 1).

**The mechanism changed; the guarantee did not.** Clinical events still travel as signed events, still
enter through the same validated in-database apply door every replicated event faces, and set-union is
still what makes a re-applied medium a silent no-op. Saying this out loud is what stops the next reader
trusting decision 2's wording as a description of the code — the ADR log is the home of *why*, and a
`why` that describes a system nobody built is worse than none.

ADR-0026 is **not** superseded as a whole. Its decision 1 clinical promise may be cited as met from this
ADR forward. Its decision 1 promise 2 — *"node-default data-at-rest keys survive"* — still has **no
subject at all**: no node-default key tier exists in the built system, so it is neither honoured nor
violated. **Do not let this ADR's success be read onto it.**

---

## Decision 5 — A restore-originated quarantine entry is not subject to the per-peer quota

A clinical record a restore cannot apply is **skipped, counted, and penned** — never a reason to abort.
In the one command that exists for the disaster where re-running the backup is impossible, converting a
partial loss into a total one is the wrong trade. The pen preserves the record's **custody** alongside
its bytes, so recovering a missing export later completes the restore without redoing it.

**The ordinary per-peer pen quota does not apply**, and the carve-out is reasoned rather than convenient:

- The quota exists to stop a **hostile or broken peer** from filling local disk with refused bytes. A
  restore's input is the operator's own medium, already on local disk, already checked. There is no
  adversary to bound and no unbounded stream.
- The bytes it would refuse are bytes the node is **about to lose permanently**. A resource budget that
  trades a clinic's record for disk it has already spent is the wrong trade.
- The quota's own promise — *the watermark freezes instead, delayed but never lost* — is a **sync-path**
  guarantee needing a cursor to freeze and a peer that will re-serve. A restore has neither.

**Unbounded must not mean unreported.** The restore reports the pen's row count and byte total, and says
so explicitly when it exceeds what a sync link would have been allowed. A bound the operator can see
beats a bound that silently drops the record.

A restore also **pins no re-offer floor**. The floor exists so a peer keeps re-offering a refused slot;
no peer re-offers a medium, and a floor set here would make the restored node's first real pull re-fetch
from a position nobody will ever resolve.

---

## Consequences

- **ADR-0026 decision 1's clinical promise is met**, for a solo node, from this ADR forward — scoped as
  decision 4 scopes it, and with promise 2 still subjectless.
- **The quarantine pen is now one in-database door with two callers** (the sync daemon and the restore
  path). The daemon crate is binary-only, so the alternative was a second copy of a safety floor in
  another crate; ADR-0001 says where it belongs instead. The quota became a caller-supplied policy
  rather than a constant, which is what makes decision 5 expressible without forking the pen.
- **A requeued sealed event now recovers its custody**, on the sync path too. That path passed no DEK on
  the reasoning that a peer would re-serve the key on a later cycle — sound for sync, false for a
  restored solo node, and a strict improvement for sync when a peer is later decommissioned.
- **A restored node exits non-zero if any clinical record was refused**, after the full operator summary
  has printed. The restore is incomplete, not failed, and the message says so.

## What is still not true

- A restore's **peak memory is unbudgeted**: the reader materialises and sorts the servable clinical set.
  Fractal topology means a Pi or an Android node is a legitimate restore target, so this is named rather
  than assumed away. Streaming parse stays deferred.
- A **burned identity `seq` is still indistinguishable from a lost clinical event**
  ([#549](https://github.com/cairn-ehr/cairn-ehr/issues/549)). This slice makes the consequence visible —
  such a record pens with a legible reason — without closing the diagnosis, and does not deliver the
  `source_seq`-gap operator surface that issue also tracks.
- A nightly capture is still **O(whole medium)**
  ([#552](https://github.com/cairn-ehr/cairn-ehr/issues/552)); the read side adds parses at the same seam.
- The **kit-restorability figure still has no per-kit home**
  ([#551](https://github.com/cairn-ehr/cairn-ehr/issues/551)), and an **unmarked foreign legacy medium can
  still be destroyed by succession** ([#553](https://github.com/cairn-ehr/cairn-ehr/issues/553)).
- The **`M > N` paper-parity defect for this ceremony** remains open and filed
  ([#512](https://github.com/cairn-ehr/cairn-ehr/issues/512)). This slice does not change its numbers; it
  changes what those steps recover.
