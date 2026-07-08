# ADR-0015 — Event serialization, signatures, and content addressing: tagged, migratable primitives over three structural moves

- **Status:** Accepted
- **Date:** 2026-06-16
- **Resolution (2026-07-07):** the one provisional sub-choice — the **BLAKE3 blob digest** (Decision §4) — is
  now **confirmed; no longer provisional.** Bet B **B4** on real ARM (Raspberry Pi 5 / Cortex-A76) measured
  **BLAKE3 913 vs SHA-256 230 MB/s** (BLAKE3 ~4× faster), so BLAKE3's chunk-verification benefit carries no
  throughput penalty on the target hardware — the "revisit if materially slower" condition below did not
  trigger. First measured 2026-06-25 and reproduced caveat-free on the 2026-07-07 PG 18 + NVMe clean re-run
  ([Spike 0001 §9.5](../../spikes/0001-walking-skeleton-wan-sync-and-pi-cost.md#95-clean-re-run-pg-18-nvme-2026-07-07-pass-both-caveats-resolved)).
  This note *overlays* — it does not erase — the "provisional" prose below (Context, Decision §4, and the
  risk list), which faithfully records the pre-Bet-B state.

## Context

The day-one event envelope ([data-model §3.5](../data-model.md#35-event-storage-model-hybrid-envelope))
reserves a **signature**, a **canonical signed byte form** ([§3.13](../data-model.md#313-schema-evolution-event-format-and-the-legibility-twin)),
and a **self-describing content digest** for attachments ([§3.14](../data-model.md#314-attachments-content-addressed-blobs-and-the-rendition-set)) —
but it deliberately did **not** fix the concrete primitives (which serialization, which signature
algorithm, which hash). Those are the least-reversible choices in the system: a production event is
signed once and verified forever across a fleet with unbounded version skew, so the *shape* of the
signed bytes and the *agility* of the algorithm identifiers cannot be retrofitted. The spec's standing
rule is "fix the rule, not the language" ([§9](../language-substrate.md#91-selection-rule-by-defect-blast-radius)),
so the primitives were carried as **validate-then-ratify defaults** through the first implementation
spike rather than asserted from taste.

[Spike 0001 §4](../../spikes/0001-walking-skeleton-wan-sync-and-pi-cost.md) specified those defaults and
their rationale; [Spike 0001 §8](../../spikes/0001-walking-skeleton-wan-sync-and-pi-cost.md) ran them on
a **real adverse WAN** — a ~710 ms-RTT, lossy Starlink/WireGuard path between a Cape York node (PG 16) and
a Dorrigo node (PG 18.4). **Bet A passed on all six §5 rows**, and three results bear directly on this
decision:

- **A2 — 0 signature-verification failures** across 792 events in both directions over a lossy
  high-latency link. Because verification byte-compares the *stored* bytes rather than re-canonicalizing
  (move 1 below), a serialization round-trip cannot silently corrupt a signature; the field run confirms
  this is structural, not luck.
- **A5 — 494–495 bytes/event** on the clinical plane (budget 4096). The deterministic-CBOR/COSE
  compactness bet pays off on a metered satellite link, where canonical JSON's base64-of-binary bloat
  would be a direct, recurring cost.
- **A1 — idempotent set-union convergence to byte-identical content-address hashes** on both nodes,
  including a conflicting shared-patient overlay resolved to the same winner. Content-addressing (the
  multihash digest, move 2) is what makes *same bytes → same address → zero-merge convergence* hold.

This ADR ratifies the primitives, per [Spike 0001 §7.1](../../spikes/0001-walking-skeleton-wan-sync-and-pi-cost.md).
It refines [ADR-0001](0001-fat-postgres-thin-daemon.md) (the envelope), [ADR-0012](0012-schema-evolution-event-format-and-legibility-across-time.md)
(the canonical signed byte form), and [ADR-0013](0013-attachments-content-addressed-lazy-blob-tier.md)
(the content-addressed digest). One sub-choice — the **blob** digest — is left **provisional** pending the
Bet B ARM throughput number (see Decision §4).

## Decision

The load-bearing commitments are **three structural moves**, not the primitives. The moves make the
primitive choice *reversible*; the primitives are tagged, migratable defaults.

### 1. The three structural moves (binding)

1. **Sign the stored bytes; parse a view; never re-serialize.** The signed artifact is an opaque byte
   string (a COSE_Sign1 wire blob); the structured form is *parsed out of* those exact bytes and never
   round-tripped back to verify. Verification is `hash(stored_bytes)` + signature check. This collapses the
   safety-critical determinism burden from *"every implementation must canonicalize identically, forever"*
   to *"the signer serialized once; everyone else byte-compares,"* and makes [§3.13](../data-model.md#313-schema-evolution-event-format-and-the-legibility-twin)
   lossless passthrough automatic. (It also defuses the draft status of deterministic-CBOR profiles: only
   the *signer* needs a fixed encoding we control.)
2. **Self-describing, algorithm-tagged digests and signatures.** Every digest is a multihash (algorithm +
   length prefix); every signature sits under a COSE `alg` header. The algorithm travels with the data, so
   the day-one default is reversible *by policy* — a wrong choice is a migration, never a re-format. This
   extends the [§3.14](../data-model.md#314-attachments-content-addressed-blobs-and-the-rendition-set)
   self-describing-digest commitment to the event digest and the signature.
3. **Re-attestation is an overlay.** An immortal, verify-forever record will outlive any one primitive's
   strength. The append-only model already supplies the migration mechanism: re-signing an event under a
   stronger primitive is just another overlay event referencing the original ([§3.1](../data-model.md#31-append-only-clinical-event-log-source-of-truth)),
   exactly like a correction. The future-proof primitive need not be in the bytes today — only the tag from
   move 2, plus the recognition that re-signing is overlay, never mutation. This is what lets the
   post-quantum cost be deferred safely.

### 2. Serialization — deterministic CBOR in a COSE_Sign1 envelope

The signed bytes are **deterministic CBOR** ([RFC 8949](https://www.rfc-editor.org/rfc/rfc8949) §4.2 /
the CDE profile) carried as the payload of a **COSE_Sign1** ([RFC 9052](https://www.rfc-editor.org/rfc/rfc9052))
structure. Chosen over canonical JSON ([RFC 8785](https://www.rfc-editor.org/rfc/rfc8785)) because it is
binary-native (no base64 bloat for the many digests/keys/signatures — the A5 payoff), has a far smaller
determinism edge-case surface, and gives a standardized, `alg`-tagged signature container with **native
multi-signer support** for the [§3.9](../data-model.md#39-authorship-and-accountability) contributor set.
Human-legibility is **not** sacrificed: the [§3.13](../data-model.md#313-schema-evolution-event-format-and-the-legibility-twin)
plaintext legibility twin already owns it, which frees the signed form to optimize purely for determinism,
compactness, and a small verifier.

### 3. Event signature — Ed25519

**Ed25519** ([RFC 8032](https://www.rfc-editor.org/rfc/rfc8032)): fast and small on Pi-class hardware,
deterministic nonce (no ECDSA RNG footgun), ubiquitous and libsodium/OpenSSL-clean, and the *same primitive
the WireGuard transport already runs* — one fewer family in the trusted base. Carried under a COSE `alg`
tag so a post-quantum signature (ML-DSA / SLH-DSA, [FIPS 204/205](https://csrc.nist.gov/pubs/fips/204/final))
is a later **overlay re-attestation**, not a format break.

### 4. Content addressing — SHA-256 for events; BLAKE3 for blobs (the blob line provisional)

- **Event content address: SHA-256**, multihash-wrapped — ubiquitous, often in-silicon, `pgcrypto`-native,
  the conservative default. **Firm.**
- **Blob content address: BLAKE3**, multihash-wrapped — **provisional pending Bet B.** Its internal
  Merkle-tree structure lets *chunks* of a blob be verified independently, which is the direct enabler of
  the [sync §6.6](../sync.md#66-attachments-the-lazy-byte-tier) chunked / resumable / multi-source-swarm
  byte tier. Bet A's §8.2 finding **sharpened** this: on the 710 ms link, latency (not bandwidth) binds blob
  transfer, so the production tier *must* window/pipeline many chunks in flight and pull from multiple
  sources — and BLAKE3's independent-chunk verification is exactly what makes that windowing safe. The one
  open input is the **ARM throughput** of BLAKE3 vs SHA-256 on a Pi (Bet B B4); the digest is multihash-tagged
  (move 2), so even reversing this is an additive migration, and the chunk-verification fit sets a high bar
  to overturn.

### 5. Key custody — Ed25519 now; FROST earmarked

Event and actor signing keys are Ed25519. For the high-value **steward / distribution-plane key**
([security §7.6](../security.md#76-the-software-distribution-plane-signed-releases-and-extension-load)) and
institutional keys ([ADR-0011](0011-actor-registry-version-pinning-and-key-custody.md)), **FROST threshold
Schnorr** ([RFC 9591](https://www.rfc-editor.org/rfc/rfc9591)) is earmarked for quorum custody and clean
rotation; it layers on the same key family, so it needs no envelope change and is out of scope to implement
now.

### 6. Dismissed (with reasons)

- **BLS aggregation** — real benefit (many sigs → one), but pairing crypto is heavy on a Pi, a large
  reviewer surface ([principle 8](../index.md#founding-principles-the-lens-for-every-decision)), and the
  payoff is thin when most events have a single author.
- **Post-quantum *now*** — SLH-DSA signatures are 8–50 KB (murders the A5 bandwidth result and the Pi),
  ML-DSA libraries are far less battle-tested than Ed25519. Move 3 (overlay re-attestation) is the answer.
- **RSA / ECDSA-P256** — only advantage is legacy PKI / smartcard interop; belongs at the interop boundary,
  handled by the move-2 `alg` tag, never the internal default.
- **Protobuf / Avro for the signed form** — Protobuf is not deterministic across implementations; Avro needs
  a schema registry (the central coupling [§6.5](../sync.md#65-schema-evolution-two-planes-and-lossless-forwarding)
  routes around). Fine inside projections, never as the signed bytes.

### 7. Blast radius ([§9](../language-substrate.md#91-selection-rule-by-defect-blast-radius))

The safety-critical surface is small and reviewable: **COSE parse + Ed25519 verify + multihash check**.
It runs in-database via the [ADR-0002](0002-in-database-rust-pgrx-escape-hatch.md) pgrx hatch so no
unverified row can enter the log; `pgcrypto` covers SHA-2; the content-address invariant is already
expressible in core SQL (the skeleton enforces it as a `CHECK`). The verify gate is the recurring §9 seam —
the one path that must be unbypassable.

## Consequences

**Easier / now established (some field-validated):**
- Idempotent set-union convergence is structural (same bytes → same multihash address → zero merge) —
  field-confirmed by Bet A1.
- The eager clinical plane is compact on metered links (Bet A5, ~494 B/event) — the deterministic-CBOR/COSE
  choice paying off where canonical JSON would not.
- Verification is byte-comparison, not re-canonicalization, so a serialization round-trip cannot break a
  signature (Bet A2, zero failures on a lossy link).
- Crypto-agility (including the PQC path) is free: tagged algorithms + overlay re-attestation.
- BLAKE3's independent-chunk verification unlocks the windowed/resumable/swarm byte tier the real link
  showed is necessary (Spike §8.2).

**Harder / what we are betting on:**
- The **signer's** deterministic encoder must be pinned and stable; move 1 contains the blast radius to the
  signer (verifiers byte-compare), but a signer-side encoding change is a new event format, not a free
  upgrade.
- The verify gate must actually **move into pgrx** to be unbypassable; in the skeleton it lives in the Rust
  applier (the §9 seam, explicitly deferred).
- Deterministic-CBOR (CDE) is still a **draft** profile — acceptable only because move 1 means we never
  depend on cross-implementation canonicalization at verification time.
- The **blob digest** is provisional: if Bet B shows BLAKE3 materially slower than SHA-256 on ARM with no
  offsetting benefit, revisit — but the chunk-verification fit makes that unlikely, and reversing it is an
  additive multihash migration, not a re-format.

**How we would know the bet is failing:** signature breakage attributable to a serialization round-trip
(Bet A2 would have shown it — it did not); canonicalization drift across independent implementations (move 1
prevents it); or a post-quantum threat arriving faster than overlay re-attestation can be rolled across the
fleet (the one scenario that would force PQC into the bytes early — and the `alg` tag is what makes even
that a migration rather than a rebuild).

**Not a new founding principle.** This ratifies primitives under the existing principles — append-only
(content addressing, re-attestation-as-overlay), legibility across time (the twin freeing the signed form),
acknowledged uncertainty (provisional-by-design defaults), and small auditable safety surface (§9).
