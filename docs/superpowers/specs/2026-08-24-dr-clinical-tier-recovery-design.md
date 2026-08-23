# Design — identity dies with the disk; custody must not

- **Date:** 2026-08-24
- **Closes:** [#495](https://github.com/cairn-ehr/cairn-ehr/issues/495) (the key),
  [#500](https://github.com/cairn-ehr/cairn-ehr/issues/500) (the bytes), and the DR-path items of
  [#502](https://github.com/cairn-ehr/cairn-ehr/issues/502) that sit on lines these slices already edit
- **Produces:** ADR-0066, spec v0.67 → v0.68, `db/051` (SCHEMA 50 → 51), two build slices
- **Pinned by:** `crates/cairn-node/tests/dr_clinical_guarantee_gap.rs` — a suite written to be
  **inverted**, one pin per broken promise

## 1. What is actually broken

[ADR-0026](../../spec/decisions/0026-node-durability-and-disaster-recovery.md) decision 1 makes three
promises about a restored node's clinical tier. All three are false, confirmed in code on 2026-08-23:

1. *"the clinical event log survives"* — `backup.rs::read_event_set` exports
   `SELECT signed_bytes FROM node_event`. That is the **federation plane**: enroll, peer, revoke,
   supersede. No `event_log` row travels, so a restored node recovers who it peered with and zero
   patients (#500).
2. *"node-default data-at-rest keys survive"* — the slot exists and has no producer.
3. *"sealed-episode DEKs survive"* — restore mints a fresh signing seed by design (decision 4), the
   X25519 unwrap secret is HKDF-derived from that seed
   ([ADR-0052](../../spec/decisions/0052-born-sealed-clinical-bodies.md) decision 4), so every
   inherited `event_dek` row is unopenable on a solo node (#495).

**Fixing any one alone is useless.** #495 alone leaves a working key with nothing to open; #500 alone
leaves sealed bodies with no key; both together still fail, for a reason neither issue names — see §6.

The reusable finding, already recorded in HANDOVER: *a deferral is only honest while its stated
precondition holds, and nothing in the repo watches for one expiring.* `localstate.rs`'s header declared
its empty seam truthfully — *"the federation-node tier has no clinical surface yet"* — and ADR-0052 made
that sentence false without reopening it.

## 2. The reframe

One seed is currently both the node's **identity** and its **data custody**. ADR-0026 deliberately kills
the identity on recovery — correctly, since the signing key is never backed up — and the coupling makes
that take the record with it.

> **Identity dies with the disk. Custody must not.**

Both ADRs were internally consistent; the contradiction lived only where they meet, which is code. That
is the second lesson worth carrying: *cross-ADR claims about the same key material must be checked at
the seam, and the seam is never prose.*

## 3. What supersedes what

- **ADR-0026 decision 4 stands unchanged.** The private signing key is still never backed up. The
  property gets *stronger*: after this design nothing else depends on the seed surviving, so the flat
  sentence needs no narrowing erratum — the outcome #495's option 1 would have forced.
- **ADR-0052 decision 4 is superseded in exactly one clause.** The node unwrap key is no longer HKDF-
  derived from the Ed25519 signing seed. It is an independent X25519 keypair with its own lifecycle,
  escrowed under **the same operator ceremony** (op-passphrase + recovery code). ADR-0052's stated
  rationale for deriving — *"no new key-management mechanism"*, meaning no second ceremony for the
  operator — is **preserved**; only the derivation is dropped.
- **ADR-0026 decision 2's wording survives intact.** The medium stays *"a normal Cairn event set with
  nothing backup-specific about it"* because everything unsigned rides the sealed export instead (§4).

## 4. The structural finding: both vehicles already exist

The design that closes this needs no new artifact. It needs the two existing ones used for what they
were built for:

| Vehicle | Carries | Established |
|---|---|---|
| The medium (`CAIRNB2` → `CAIRNB3`) | **Signed events only** — the node plane and now the clinical plane | ADR-0026 decision 2 |
| The sealed export (`CAIRNL1`) | **Node-local non-event material** — custody rows, the actor registry, the unwrap secret | ADR-0026 point 3 |

The export is written beside the medium on every backup (`localstate_path_for(medium)`, a sibling), so
it shares the medium's fate: if the medium survives a disaster, so does the export. Its `episode_deks`
slot is already named for `event_dek` custody. Its dual-recipient seal (op-pass **or** recovery code,
via the `.lsk` sidecar established at `init`) is already the escrow the unwrap secret needs.

A third structural fact settles a question #495 left open. `node_unwrap_key`
([db/037:17](../../../db/037_born_sealed.sql)) is a **singleton whose registrar refuses a different
key** — *"rotation is a separate ceremony"*. So a restored node must **adopt** the old unwrap key
rather than mint a new one, and **no keyring is required**. The same fact makes the migration for
existing nodes lossless (§5).

## 5. Slice 1 — the key (#495)

**The keypair at rest.** `keystore.rs` gains an independent X25519 unwrap keypair in a sibling file
(`node.unwrap`), sealed under the same two secrets as `node.key`, with its own `KeyAtRest` inspection so
`status` reports each independently. A sibling file rather than a widened `node.key` payload: `load()`
distinguishes a sealed bundle from a raw 32-byte seed by shape, and changing the sealed payload would
break both that detection and the `--insecure-plaintext` path for no gain.

**Provisioning and migration.** `init` mints it. An existing node runs `establish-unwrap-key`, which
**adopts its currently-derived secret as the first independent key**. Because the derivation is
deterministic in the seed, this is lossless: no rewrap, no migration of `event_dek` rows, every sealed
body keeps opening. This is why "break the derivation" is cheap *today* and grows more expensive every
week — the adoption path only works while the derived secret is still reconstructible.

**Containing the trap.** `cairn_event::seal::derive_unwrap_secret` survives **only** as that migration
path. It moves behind a shouting doc comment with a **pinned call-site list** — the
`hex_decode_helper.rs` convention — so a future edit cannot quietly re-couple identity to custody. The
count failing is the guard working.

**Transport.** `LocalState` gains a typed `unwrap_secret` slot. This is additive under the format's
existing contract: `serde(default)` means an older bundle still reads; `deny_unknown_fields` means an
older *build* refuses a newer bundle loudly rather than dropping key material silently.
`read_local_state`'s unused `_db` parameter becomes real.

## 6. Slice 2 — the bytes, and the registry nobody had counted

**The medium.** `CAIRNB3` carries **two explicit sections** — node plane, clinical plane — rather than
one mixed list a restore would have to classify by inspecting bodies. `CAIRNB1`/`CAIRNB2` still read, as
node-plane-only. Explicit sections also give the drift guard (§8) something to check against.

**The export.** Gains `event_dek` rows **wrapped, verbatim** — no raw key material ever lands in the
export — and `actor_event` rows.

**The registry is the piece neither issue names.** [db/020:174](../../../db/020_apply_remote_event.sql)
hard-refuses any event whose signer is not an enrolled, non-revoked actor, and `actor_event`
([db/004:11](../../../db/004_actors.sql)) is an **unsigned local registry table** — not an event plane,
not replicated, not on the medium. So even with the key and the bytes both restored, a solo node would
refuse every clinical event it just read back, because it no longer knows who its own clinicians are.
Re-enrolling by hand is not a fallback: `actor_id` is the content-address of a pinned determinant set,
so the operator would have to reproduce every set exactly, from memory, during a disaster.

The registry therefore rides the sealed export as node-local material, restored through a self-trusting
door. **The ADR must say out loud that the export is now authorization-relevant state** — it decides
which keys are trusted actors — and that this is acceptable because it is dual-sealed, physically
co-located with the medium, and consumed only by a door fenced to a fresh node.

**The doors.** `db/051` (SCHEMA 50 → 51) adds `restore_event(p_signed, p_dek)` and
`restore_actor_event(…)`, both fenced on empty `local_node` exactly as
[db/009:48](../../../db/009_node_supersede_and_restore.sql) is — a permanent no-op on a live node.

**Restore is the local-medium analogue of a peer pull.** Both existing doors take a *raw* DEK and wrap
it locally ([db/020:411](../../../db/020_apply_remote_event.sql)), which is what cairn-sync's
`rewrap_custody_for_peer` feeds them. Restore does the same: the daemon unwraps each exported
`event_dek.dek_wrapped` in memory with the recovered secret and hands the raw DEK to `restore_event`,
which re-wraps and fills `event_clear`. No new custody mechanism, and the export never holds a raw DEK.

## 7. At the restore door, a refusal is data loss

This inverts the rule the other two doors follow, and the inversion is load-bearing.

At a peer door, refusing an event is safe: the peer re-offers, and the quarantine/re-offer floor makes
the refusal recoverable. **At the restore door there is no peer.** The medium is the last copy. A
refusal there is permanent loss of that event — the outcome the whole ADR exists to prevent.

So `restore_event` verifies **integrity only** — signature, content address, size ceiling (the same
ceiling as every other door, so a restored image cannot smuggle an oversized event that later wedges
outbound sync) — and **admits-and-flags everything else**, including an unenrolled signer. The registry
is restored first so the gate normally passes; when it cannot, the event is kept and the degradation is
visible rather than dropped.

This is [ADR-0056](../../spec/decisions/0056-unknown-event-types-admitted-uninterpreted.md)'s admit-uninterpreted floor
applied where it matters most, and it is the same rule ADR-0065 named — *refuse at a door only what that
door can drop whole*. Here the door can drop nothing whole, so it refuses only bytes that are not what
they claim to be.

## 8. Restore ordering is forced

Not a style choice — each step is a precondition of the next:

1. **Unseal the export** (op-pass or recovery code). A present-but-unreadable export must **stop the
   restore**, not be skipped in silence — one of #502's four spots, and it lands squarely on this path.
2. **Install the recovered unwrap secret** into the keystore.
3. **Register the unwrap public half** — `cairn_wrap_dek` in step 5 needs `node_unwrap_key` populated.
4. **Restore the actor registry** — the clinical door's enrollment gate reads it.
5. **Apply the node plane, then the clinical plane with custody** — per event: unwrap its DEK, call
   `restore_event(signed, dek)`.
6. **Finalize identity** — `finalize_identity` writes `local_node`, which **fences every restore door
   permanently shut** behind the ceremony.

## 9. The guard that stops this recurring

A hand-maintained table list is precisely what failed. The guard reads an **authority**: a catalogue
query for every table holding a `signed_bytes` column, cross-checked against a registry that declares
each one **exported** or **deliberately not, with a reason**. The quarantine pens
(`db/021`, `db/022`) are legitimately not exported — they hold unadmitted bytes a peer will re-offer.

A new table holding signed bytes fails the gate until somebody decides which it is. Same shape as the
twin-check and projection registries, and the same house rule: *where a family has an authoritative
list, read the list.*

Two honest-reporting fixes ride along, because the composite untruth was as serious as the defect:
`backup-status.json` and `status` must distinguish **what the medium carries** from **how fresh it is**,
so a node without a clinical net says so (ADR-0026 decision 7).

## 10. Scope

**In:** the two slices above, ADR-0066, spec §7.10 + §3.8 revision, the drift guard, and three of
#502's four silent-success spots — the ones these slices already edit: the **present-but-unreadable
export skipped in silence at restore** (§8 step 1), **`verify-backup` printing `backup OK: 0/0`** over a
medium that restores nothing (§9), and the **corrupt `.lsk` diagnosed as "absent"** with a remedy that
then refuses (slice 1 touches that file's lifecycle). The fourth — a discarded keystore-load reason —
stays with #502.

**Out, and named so the deferral is watched rather than forgotten:**

- **Unwrap-key rotation.** Still the separate ceremony ADR-0052 deferred; `node_unwrap_key`'s singleton
  registrar still refuses a swap. This design does not need it (restore adopts), but a node that
  *suspects* its unwrap key is compromised has no path. File it.
- **Actor-registry sync** (ADR-0011). The export carries the registry for DR only. Cross-node registry
  replication remains unbuilt, and the enrollment ceremony remains the peer-apply path.
- **The byte tier.** Attachment blobs are not on the medium; the reference travels with the event, the
  bytes do not. That is ADR-0013's design, not a defect, but a restored node's renditions degrade to
  references-only and the operator must be told.

## 11. Paper-parity benchmark (§1.2)

**Paper counterpart:** the off-site duplicate chart — the practice that photocopies its records and
keeps the copy in another building, then carries the box back after a fire.

**Steps:** paper *N* = **2** human acts to recover (fetch the box; shelve it). Architecture-forced
*M* = **3** (attach the medium; run `cairn-node restore` and answer its prompts; confirm the echoed
identity when provenance is not sole-enroll-signed). UI bundling target *K* = 2.

**`M > N`, so per house rule 7 this is filed as an architecture defect, not argued away.** The extra act
is the identity confirmation, which has no paper counterpart because a paper box has no cryptographic
identity to mis-assign. Two observations for whoever picks the issue up: the escrow secret and the
restore invocation are one interactive ceremony, not two acts, and `Provenance::Signed` on a sole-enroll
medium is already unambiguous — the confirmation is only forced on the federated/unsigned paths, so *K*
= 2 is reachable without weakening anything.

**Time + cognitive load budget:** a restore of a 100k-event medium completes in ≤ 10 min unattended
after the operator's last keystroke, and the operator needs **one secret** (op-pass or recovery code)
and **no knowledge of the dead node's configuration**. Measurement is owed by slice 2, on the rig that
already runs the DB-gated suites. **If the measurement falls outside the budget, that is the finding —
file it; never adjust the budget.**

The ongoing cost is where the comparison actually favours the architecture, and it belongs in the
record: paper's duplicate costs a photocopy per page, forever; Cairn's costs a nightly cron.

## 12. Testing — the suite is inverted, not extended

`dr_clinical_guarantee_gap.rs` was written to go red on the commit that fixes the gap, each assertion
naming what it must be inverted to. **No pin may survive both slices unchanged.**

- **Slice 1 inverts** `a_restored_nodes_fresh_seed_cannot_open_a_pre_restore_sealed_body` (a restored
  node *can* now open it) and `local_state_export_carries_no_dek_though_the_database_holds_one`.
- **Slice 2 inverts** `medium_carries_the_federation_plane_and_no_clinical_event`.
- **The MECHANISM test stays true** and its comment is updated to say why.

Anti-vacuity discipline carries over from the guard suite and is not negotiable: the node is provisioned
so the medium is genuinely non-empty; the DEK is written by the **production door**, never by the test;
and every refusal test asserts the happy path **first**, so a refusal cannot pass for the wrong reason.

The end-to-end test that makes decision 1 true is one test, and it is the acceptance criterion for slice
2: **author a born-sealed clinical event, back up, destroy the database, restore, and read the body
back.** Anything less tests a component, not the promise.

## 13. Rejected

- **Escrow the signing seed.** Backs up the signing key, so a stolen medium can forge events as that
  node. It is the reading of #495's option 1 that trades a data-loss hole for an impersonation hole.
- **Escrow the derived secret only.** Keeps decision 4 literally true, but you are then persisting and
  escrowing an independent secret anyway — option 2 with the derivation left in as decoration, and the
  coupling still there for the next person to trip over.
- **Declare the loss.** Since ADR-0052 the blast radius is the whole clinical record. ADR-0026's own
  framing calls this *"the exact outcome crypto-shredding is designed to produce, by accident."*
- **A registry section on the medium.** Breaks decision 2's *"nothing backup-specific about it"* and puts
  unsigned authorization rows on a container whose self-marker attests the event set, not them.
- **Operator re-enrolls the registry after restore.** Requires reproducing content-addressed determinant
  sets from memory, during a disaster, or the record stays refused.
- **A restore door that skips the enrollment gate.** Leaves `actor_id` NULL across the whole restored
  record — authorship grading silently degrades on every event, which is a precise untruth in the
  reassuring direction.
- **The cold peer as a genuine `cairn-sync` peer configuration** (what decision 2's wording implies).
  Structurally drift-proof and the better long-term shape, but it needs the medium to become a
  file-backed peer endpoint. Deferred in favour of the drift guard, which buys the same protection
  against the failure mode that actually occurred.

## 14. Declared limitations

- **The export becomes authorization-relevant.** Whoever holds it *and* a secret can restore a node that
  trusts the dead node's actors. Mitigated by the dual seal, physical co-location, and the fresh-node
  fence — not eliminated.
- **A backup taken before slice 2 restores as node-plane-only**, correctly and loudly. There is no way
  to retrofit clinical events into a medium that never held them.
- **RPO is unchanged**: the last stream to the medium. Events authored after the last backup are gone,
  as ADR-0026 always said.
- **The converged-peer splice** (medium.rs's known limitation) is untouched by this design.

## 15. Findings to file

1. **The restore ceremony's step count** — §11's `M > N`.
2. **Unwrap-key rotation has no path** — §10.
3. Any #502 item that does *not* sit on a line these slices edit, so it stays visible rather than
   half-closed.
