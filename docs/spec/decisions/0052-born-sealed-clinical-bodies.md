# ADR-0052 — Born-sealed clinical bodies: erasability as the shipped default

- **Status:** Accepted
- **Date:** 2026-07-17
- **Refines:** [ADR-0005](0005-erasure-key-custody-and-crypto-shredding.md) (crypto-shredding + the
  severity ladder — this ADR turns 0005 §2's *"reserved, off by default"* per-record seal into the
  **shipped default** for clinical bodies, so every rung stays reachable for every event);
  [ADR-0006](0006-visibility-scope-replication-and-the-safety-projection.md) (the safety projection —
  this ADR states its previously-unwritten erasure semantics); [ADR-0026](0026-node-durability-and-disaster-recovery.md)
  (the keystore, backup-as-cold-peer, shred-aware backups — the node unwrap key rides its escrow);
  [ADR-0048](0048-twin-check-registry-dispatch.md) (the per-type twin/floor-check dispatcher — the seal
  door runs the *same* checks on decrypted plaintext); [ADR-0051](0051-contributor-role-vocabulary-floor-and-responsibility-wire-shape.md)
  (the strict-submit / lenient-apply door precedent this ADR reuses); [ADR-0013](0013-attachments-content-addressed-lazy-blob-tier.md)
  (blob bytes inherit born-sealed via the per-blob DEK); [ADR-0012](0012-schema-evolution-event-format-and-legibility-across-time.md)
  (additive-only wire — the custody sidecar and the sealed `schema_version` are its application).
- **Resolves:** [#189](https://github.com/cairn-ehr/cairn-ehr/issues/189) (2026-07-15 review finding C1,
  Critical/window-closing — the seal-by-default posture), [#92](https://github.com/cairn-ehr/cairn-ehr/issues/92)
  (2026-07-02 — the ADR-0005 erasure-ladder composition collisions).

## Context

[ADR-0005](0005-erasure-key-custody-and-crypto-shredding.md) resolved erasure as *key-custody
redistribution* — crypto-shred a body by destroying its DEK, never the row. But its rungs only work on
a body **sealed under a per-record DEK at write time**, and §2 reserves that shape while leaving it
**off by default** (default deployments use whole-storage encryption). [Data-model §3.5](../data-model.md#35-event-storage-model-hybrid-envelope)
concedes the shape "cannot be retrofitted onto an append-only log without re-encrypting history." Three
consequences the corpus never states, all window-closing now that production clinical (medication)
events are being authored:

1. **The default forecloses ADR-0005's own headline triggers.** In a plaintext-default deployment the
   retention **ceiling** (scheduled, clock-driven destruction) and no-retention-basis subject requests
   hit ordinary plaintext events that are **permanently un-shreddable**. Rungs 2–4 are unreachable for
   the whole record; retention-ceiling compliance is impossible.
2. **"Sensitivity recognized later" cannot be honored.** The clinically common case — stigma realized
   weeks after the fact — cannot retro-seal a body already replicated in plaintext to N nodes. The
   [§5.9](../identity.md#59-sensitivity-grade-the-safety-projection-and-break-glass-visibility-scope)
   sensitivity stream can raise visibility overlays but never claw plaintext back.
3. **Three composition collisions (#92) hide behind ADR-0005's "resolved" label.** (a) The legibility
   twin's location under seal is *implied, never stated* — a future implementer materializing
   `plaintext_twin` into an FTS column turns every seal into a full-content leak. (b) Every
   [ADR-0048](0048-twin-check-registry-dispatch.md) check fn reads the plaintext body, so the seal path
   either bypasses the mandatory-twin floor or needs a structurally different door — unspecified. (c)
   The mandatory clear-text attachment descriptor survives figure-granular shred by design ("photo of
   self-harm scars, left forearm" outlives the pixels). Plus: rung-2's "no discoverable institutional
   record" is unachievable without day-one preconditions no ADR wires in, and no ADR states the safety
   projection's own erasure semantics.

The deciding argument is [principle 9](../index.md#founding-principles-the-lens-for-every-decision)
(policy-neutral infrastructure): a plaintext default **silently forecloses rungs 2–4 for the entire
record** — that *is* the system taking a policy stance (against erasability). Born-sealed under the
node's own custody forecloses nothing and hides nothing; it is the only genuinely policy-neutral
default. The window closes with the first production clinical event, so this is a pre-production wire
fix, not a migration ([ADR-0040](0040-signing-context-domain-separation.md)/[ADR-0051](0051-contributor-role-vocabulary-floor-and-responsibility-wire-shape.md)
precedent: pre-production is exactly when to fix wire-freeze items).

## Decision

**Every clinical JSONB body is born sealed** — sealed at write under a per-event DEK wrapped for the
node's own key. This is an **erasability substrate, not confidentiality**: the node reads its own data
freely, projections and full-text search behave exactly as before, nothing is hidden from anyone. Its
sole effect is that **every ADR-0005 erasure rung stays reachable for every event, forever.**

Rejected alternatives, recorded: *status quo* (plaintext default, seal opt-in — permanently forecloses
rungs 2–4 for ordinary events) and *category-sealed at write* (blacklist-driven — everything the
blacklist misses is foreclosed exactly as the status quo, and the "recognized later" case still dies).
No new founding principle: this is principles [1](../index.md#founding-principles-the-lens-for-every-decision)
(append-only), [4](../index.md#founding-principles-the-lens-for-every-decision) (acknowledged
uncertainty / declared-not-guaranteed deletion), [9](../index.md#founding-principles-the-lens-for-every-decision)
(policy-neutral infrastructure) and [12](../index.md#founding-principles-the-lens-for-every-decision)
(the unbypassable in-DB floor) applied to erasability.

### 1. Erasable vs. sequestered — one word split into two properties

The word "sealed" has been conflating two independent properties; this ADR names them apart:

- **Erasable** (new, the shipped **default**, universal for clinical JSONB bodies): body ciphertext
  under a per-event DEK whose custody **includes the node itself**. Hides nothing — the node decrypts
  its own data at will. It exists so the erasure ladder stays reachable. This **refines ADR-0005 §2**:
  what §2 reserved as *"per-record encryption, off by default"* becomes *born-sealed under node custody,
  on by default* for clinical bodies.
- **Sequestered** (the existing ADR-0005 rung-1 / [ADR-0006](0006-visibility-scope-replication-and-the-safety-projection.md)
  posture — graded, opt-in): the **same DEK** with custody **narrowed** (node → named
  clinicians/patient), the safety projection coarsened, the twin under seal.

Because both use the same DEK, **sequestering an already-erasable event is a key-custody change, not a
rewrite.** This dissolves "sensitivity recognized later": no retro-encryption — just re-wrap or withdraw
keys, plus an honest declaration of any already-replicated plaintext derivatives (of which, born-sealed,
the log holds none). Sequester/custody-narrowing implementation is deferred (§8); the posture is
ratified now.

### 2. Scope, stated normatively

Born-sealed covers **clinical JSONB bodies only.** The following stay **plaintext by necessity** — the
machinery binds on them, so the ADR says so plainly rather than leaving it implied:

- **Demographic assertions** — the typed columns the matcher ([§5.2](../identity.md#52-matching-pipeline-safety-asymmetric-false-merge-worse-than-false-split))
  and coherence checks bind on.
- **Identity-algebra events** — the link/unlink/reattribute/identify/dispute stream.
- **Node-plane events** — federation, actor-registry, sync-control.
- **Erasure-plane events** — the shred tombstone (§6) **must outlive all keys**, so it can never itself
  be sealed under a key it may have to survive.

**Blob bytes inherit born-sealed** via the per-blob DEK already reserved in [ADR-0013](0013-attachments-content-addressed-lazy-blob-tier.md)/[ADR-0042](0042-concrete-attachment-reference-shape.md)
(`SealRef { alg, dek_wrap }`); implementation is deferred to the blob-tier slice (§8), the posture
ratified now.

### 3. Seal-then-sign, per-event DEK, the twin under the seal

The author **seals, then signs**: the signed payload carries the sealed container and the signature
covers the **ciphertext** — so the signature verifies on every node and **after shred**, exactly as
ADR-0005 requires. AEAD makes decryption deterministic, so *signature-over-ciphertext + DEK-in-hand* is
non-repudiation of the plaintext. The ratified sealed-payload **container shape** is:

```json
{"sealed": true, "alg": "xchacha20poly1305", "nonce": "<hex 24B>", "ct": "<hex>"}
```

AEAD is XChaCha20-Poly1305 (the `seal.rs` house algorithm); the `alg` field keeps the shape crypto-agile
under [additive evolution](0012-schema-evolution-event-format-and-legibility-across-time.md). The
**AAD binds `event_id`**, so a sealed container cannot be lifted from one event and replayed under
another's signature.

**The legibility twin travels inside the sealed region under the same DEK** — the one normative sentence
#92(a) asked for. **A sealed twin must never be materialized into any plaintext index.** The outer
[§3.13](../data-model.md#313-schema-evolution-event-format-and-the-legibility-twin) `plaintext_twin` on
a sealed row is a **signed mechanical stub** naming only type + seal state (principle 11: the row stays
honestly self-describing as *what it is*; a custody-holding reader recovers the real twin by decrypt).
The operational twin (the FTS/RAG substrate) is derived at door time into a mutable derived-plaintext
table (`event_clear`, §4/§6), never a plaintext column on the append-only row — the structural answer to
collision (a): **on a sealed row there is no plaintext twin column to leak from.**

DEK granularity is **per-event** (finest shred granularity, simplest keystore). A per-episode key
hierarchy remains the ADR-0005 measurement question, now with a concrete hook — the accompanying
slice's perf bench (§8).

### 4. The key-custody plane

- **Wrapped DEKs live in a mutable keystore table beside the log — never inside the signed bytes.**
  Custody changes must not touch the immutable artifact, and an append-only row cannot hold rotating,
  multi-holder custody. The table is **`event_dek`** (`event_id`, `holder`, `dek_wrapped`). The reserved
  `db/001` `event_log.dek_wrapped` column is **retired unused** by this
  design — it is exactly the append-only slot that cannot carry rotating custody. It is left in place
  (dropping it fires no trigger but breaks the replay-idempotence discipline for no gain) and documented
  as **superseded by `event_dek`**. `event_log.sealed` **stays** — it is an immutable birth property.
- **The node unwrap key is X25519**, derived from the node's Ed25519 signing seed by HKDF with
  `info = "cairn-node-unwrap-x25519-v1"`. Wrapping needs only the **public half**, so the DB holds
  nothing secret — an ordinary DB backup can **never** reconstruct a DEK (the ADR-0005 *"keys must not
  be silently reconstructable from ordinary DB backups"* requirement, now structural). Deriving the
  unwrap key from the signing seed means the **existing [ADR-0026](0026-node-durability-and-disaster-recovery.md)
  op-passphrase + recovery-code escrow already covers it** — no new key-management mechanism, and KEK
  escrow is therefore **mandatory** (KEK loss = whole-record loss). The public half is published as a
  signed **unwrap-key certificate**: CBOR body `{"kid": <hex ed25519 pub>, "x25519_pub": <hex 32B>}`,
  signed under its own ADR-0040 signing context `CTX_UNWRAP_KEY` (`"application/cairn-unwrap-key+cbor"`)
  so it can never be replayed as an event, attestation, or pairing bundle.
- **Sync:** custody travels as a **wrapped-DEK sidecar** beside the event on the clinical wire (additive
  wire field, ADR-0012). For erasable-tier rows the sender re-wraps the DEK for any admitted peer
  (custody follows admission trust). Sequestered rows ship ciphertext + safety projection only; no
  sidecar unless the peer is a named holder. A peer without custody **admits the row and cannot read
  it** — set-union losslessness holds; **confidentiality lives in key custody, never in withholding
  rows** ([ADR-0006](0006-visibility-scope-replication-and-the-safety-projection.md) confirmed, not
  amended).

### 5. Two doors — the floor makes born-sealed unbypassable

The seal posture is enforced at the in-DB submit/apply floor
([principle 12](../index.md#founding-principles-the-lens-for-every-decision)), reusing the
[ADR-0051](0051-contributor-role-vocabulary-floor-and-responsibility-wire-shape.md) *strict-submit /
lenient-apply* asymmetry:

- **Submit door (strict):** it **refuses an UNSEALED `clinical.*` body** — this is what makes
  born-sealed the unbypassable posture, not a convention a client can skip. The caller passes the signed
  sealed bytes **plus the DEK**; the door verifies the signature over the ciphertext, **decrypts in-DB**
  (XChaCha20-Poly1305 via `cairn_pgx` — the [ADR-0002](0002-in-database-rust-pgrx-escape-hatch.md) pgrx
  escape hatch exists for exactly this), runs the **full** existing [ADR-0048](0048-twin-check-registry-dispatch.md)
  twin/floor checks on the plaintext, builds projections + the `event_clear` operational-twin row, wraps
  the DEK for the node, and stores it in `event_dek`. The floor stays unbypassable *because* it decrypts
  and checks in the database. A sealed event **without its DEK** is refused at submit — a door only
  authors what it can stand behind.
- **Apply door (lenient):** with a custody sidecar it does the same, leniently. A **foreign plaintext
  body** is admitted (set-union losslessness — an older/foreign node's unsealed event is never
  rejected). A **sealed event arriving without custody** is admitted on **structural checks only** —
  *can't read → never reject.* Confidentiality is preserved (no key, no read); losslessness is preserved
  (the row lands and converges).

### 6. Shred (rung 3) and its blast radius

An audited rung-3 crypto-shred is three coordinated acts:

1. **Destroy the wrapped-DEK rows** for the event in `event_dek` (the mutable keystore — never the log).
2. **Scrub all derived plaintext:** the `event_clear` operational-twin row, every projection, and **any
   FTS/RAG index** — #92(b)'s **mandatory index invalidation**. A shred that leaves the body's text
   searchable is not a shred.
3. **Append the signed `erasure.shred.asserted` event** — the audit tombstone (*existed → destroyed,
   basis Z*), itself plaintext-by-necessity (§2).

**The log row is never touched:** its signature still verifies, set-union still converges, a resurrected
opaque row is keyless noise. **Restore/re-sync replays the shred log before projecting** — a cold-peer
restore ([ADR-0026](0026-node-durability-and-disaster-recovery.md)) re-applies erasures *before* it
builds any projection, so a restored backup can no more resurrect an erased body than a sibling can.
**Custody is never granted to an already-shredded event** (arrival-order independence): if a DEK sidecar
arrives *after* the shred tombstone, the door refuses to populate `event_dek` — the shred wins
regardless of message order. The shred log table is **`erasure_shred_log`**.

### 7. The prose-only closures

- **Safety-projection erasure semantics (#92(c)):** the [§5.9](../identity.md#59-sensitivity-grade-the-safety-projection-and-break-glass-visibility-scope)
  safety projection **coarsens but survives** a rung-3 shred — the Rh-after-termination case requires
  the signal to outlive the body. It is shreddable **only at rung-4** best-effort oblivion, and its
  survival is **named in the honest-erasure ceiling** (the declared "what remains" list of ADR-0005 §5).
- **Attachment descriptor (#189(c)):** the [§3.14](../data-model.md#314-attachments-content-addressed-blobs-and-the-rendition-set)
  **public descriptor is graded**, mirroring the safety-projection coarseness ladder — the *precise*
  descriptor lives under the seal, the public stub's coarseness follows the sensitivity grade. "Photo of
  self-harm scars, left forearm" never sits in plaintext on a sequestered event; what survives a pixel
  shred is the coarse stub, declared in the ceiling.
- **Rung-2 preconditions (#92(d)):** deniable deletion is **only reachable for episodes born
  pseudonymous with abstracted routing** ([§5.6](../identity.md#56-pseudonymous-sanctioned-care) + the
  ADR-0006 envelope-abstraction dial) — stated as **day-one preconditions, never offered
  retroactively.** A record that was born identified cannot later become deniably-deleted, because the
  plaintext envelope already proved its existence. Prose only; no code this slice.

### 8. Deferred, explicitly named

- **Per-episode DEK hierarchy** — the ADR-0005 keystore-overhead-vs-shred-precision measurement
  question; measure via the accompanying slice's perf bench against the Bet-B latency budget.
- **Shred authorization policy hooks** — the gravity/authorization gate around rung-3 (ADR-0005 names
  the keystore safety-critical; the *who-may-shred* policy surface is later).
- **Sequester / custody-narrowing implementation** — the ADR-0006 mechanism beyond the minimal seal
  path (re-wrap/withdraw, the sensitivity-stream code).
- **Blob-byte sealing** — born-sealing the [ADR-0013](0013-attachments-content-addressed-lazy-blob-tier.md)
  byte tier (posture ratified now, code in the blob-tier slice).
- **Unwrap-key rotation** — rotating the node X25519 unwrap key (the derivation and escrow are fixed
  now; rotation mechanics later).

### 9. Interaction with ADR-0049 (sign-off currency)

Born-sealing changes what the [ADR-0049](0049-commitment-based-sign-off-currency.md) thread commitment
measures. ADR-0049's staleness compare reads a sealed thread's content-event set through
`cairn_clear_payload` (§3 above) — i.e. only events **this node holds custody of** contribute to the
recomputed commitment. The commitment is therefore a function of local **custody**, not, as ADR-0049
Decision-3 states, a pure function of the append-only content-event **set**. This breaks ADR-0049's own
stated soundness argument — *"sound because thread content is append-only / grow-only"* — for the
**sealed, partial-custody** case only: a node that is missing custody of a later sealed content event
cannot see that the set has grown, so its recomputed commitment can coincidentally match the pinned one
for a thread that in fact grew — a **false-fresh** read, corrupting the staleness signal rather than
honestly degrading it.

To hold ADR-0049's soundness in that case, `reviewed_count` — ADR-0049's *"legibility hint only, never a
staleness authority"* — is promoted, for sealed threads, to a **safe-direction withholding tripwire**:
whenever a node's readable content-event count for a thread (`cairn_medication_thread_readable_count`) is
**less than** the attestation's `reviewed_count`, the sign-off is forced **stale/unknown**, never reported
"current," regardless of whether the (necessarily partial) commitment happens to match. This **refines**
ADR-0049's "never a staleness authority" statement for the sealed case; it does not retract it. On an
**unsealed or full-custody** thread the refinement is inert — nothing is ever unreadable there, so the
readable count can never fall short of `reviewed_count`, the tripwire's condition never fires, and the
commitment compare alone remains the sole staleness authority, exactly as ADR-0049 states.

The added direction is safe by construction: the tripwire can only **withhold** "fresh," never assert it
in a case the plain commitment compare would have missed — the same "err toward re-review" posture
ADR-0049 Decision-3 already commits to, and principle 4 (acknowledged uncertainty — an imprecise
near-truth over a precise untruth) applied to a staleness signal specifically.

**Residual, tracked as a follow-up, not closed here:** the tripwire is blind to the case where the
readable count coincidentally equals `reviewed_count` while the thread has in fact grown by a sealed
content event this node cannot attribute to the thread *at all* — a sealed-no-custody row it cannot even
count. Closing that gap needs thread-membership metadata that survives custody loss — a separate design
decision, deferred.

The accompanying **walking-skeleton seal slice** lands on `clinical.medication` (the only live
clinical-content stream) — the thinnest end-to-end thread through seal-at-write, the two doors, the
`event_dek`/`event_clear`/`erasure_shred_log` custody plane, the sync sidecar, shred, and restore-replay,
proving the collisions closed while they are cheap. PR number pending.

## Consequences

**Easier / gained:**

- **Every erasure rung is reachable for every clinical event, permanently** — retention-ceiling
  compliance and no-retention-basis erasure become possible in a *default* deployment, which the
  plaintext default made impossible.
- **"Sensitivity recognized later" dissolves into a key-custody change** — sequestering re-wraps the
  same DEK; there is never plaintext to claw back, because born-sealed leaves none in the log.
- **The three #92 collisions are closed structurally, not by convention:** the twin lives under the same
  DEK and there is *no* plaintext twin column on a sealed row to leak (a); the seal door runs the *same*
  ADR-0048 checks on decrypted plaintext, so the mandatory-twin floor is neither bypassed nor forked
  (b); the attachment descriptor is graded, so a precise descriptor never outlives a shred (c).
- **DEKs are unreconstructable from ordinary DB backups by construction** (public-half-only in the DB),
  and the unwrap key rides existing ADR-0026 escrow — no new key-management surface.

**Harder / the bet:**

- **The submit door now decrypts in-DB.** The seal/verify/decrypt/check/wrap path joins the reviewed
  safety-critical floor ([§9](../language-substrate.md)); its strict-submit/lenient-apply split (refuse
  unsealed `clinical.*` and sealed-without-DEK at submit; admit both leniently at apply) is a subtle
  contract a future maintainer must not "simplify" into symmetry — the door's doc comment must carry the
  warning, as ADR-0051's does.
- **Seal/unseal cost lands on the Pi-class latency budget** ([ADR-0001](0001-fat-postgres-thin-daemon.md)).
  Per-event DEKs are the simplest keystore but the largest key count; whether a per-episode hierarchy is
  needed is the deferred measurement, now with a bench hook.
  *Measurement (`cairn-sync bench-seal`, ~1.5 KB medication body, N=10 000, release, Apple-Silicon dev box):*
  seal ≈ 15 µs, wrap ≈ 42 µs, unwrap ≈ 40 µs, unseal ≈ 12 µs → **whole seal→wrap→unwrap→unseal pipeline
  ≈ 0.11 ms/event**, ~37× under the Bet-B ~4 ms budget; the X25519 DEK-wrap dominates, the symmetric body
  AEAD is cheapest. Per-event DEKs are comfortably affordable on a dev-class node — the Pi-class re-run
  (the actual budget target) stays the deferred per-episode-hierarchy question.
- **KEK escrow is mandatory** — born-sealed means KEK loss is whole-record loss. This is a real
  operational obligation the default now carries (mitigated by riding ADR-0026's existing recovery
  machinery).
- **A node without the seal-capable binary can admit but not project sealed events** — honest
  degradation per ADR-0012's two planes (moot pre-production; the pre-ADR-0052 plaintext dev/PoC logs
  are wiped, never synced through, per the ADR-0051 pre-clinical-posture precedent).

**How we'd know it's wrong:** if the per-event seal cost blows the latency budget on a Pi-class node and
a per-episode hierarchy still cannot recover it (born-sealed too expensive to be the universal default),
or if a real deployment needs an erasability semantics that born-sealed's rungs still cannot express —
then the *granularity or the ladder*, not the born-sealed principle, is what needs rework. The
born-sealed-as-policy-neutral-default argument is load-bearing; the DEK granularity is the replaceable
part.
