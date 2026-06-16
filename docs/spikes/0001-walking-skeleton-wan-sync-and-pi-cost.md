# Spike 0001 — Walking Skeleton, WAN-Sync Validation, and Pi Cost

- **Status:** Proposed (first implementation task)
- **Date:** 2026-06-16
- **Validates:** [ADR-0001](../spec/decisions/0001-fat-postgres-thin-daemon.md) (projection cost on weak
  hardware), the [§6.2](../spec/sync.md#62-consistency-model) set-union convergence claim under a *real*
  partition, the [ADR-0013](../spec/decisions/0013-attachments-content-addressed-lazy-blob-tier.md)
  availability floor, and the day-one **serialization / signature / digest** primitives (§4 below).
- **Does not yet ratify anything.** The primitive defaults here are *validate-then-ratify*: this spike
  is how we learn whether they hold. The ADR that fixes them is written **after** the spike, citing its
  results.

> [!NOTE]
> This is build-prep, not architecture. The numbered spec (§1–§11) and the ADR log describe a *decided*
> design; a spike is an implementation task that *exercises* that design against reality. Spikes live
> under `docs/spikes/` so the spec stays a clean statement of what Cairn is, and the spike record stays a
> clean statement of what we tried and learned.

---

## 1. Why this spike, and why now

The handover names "the Pi-benchmark spike" as the designed first implementation task. But the test
environment now available — a **MacBook in Cape York (Bamaga)** on portable Starlink-mini and a **DGX
Spark in Dorrigo, NSW** on Starlink, joined over a WireGuard VPN — does not actually stress the bet the
Pi-benchmark exists to test. Both machines are *fast*; the DGX Spark especially is the opposite of the
Pi profile. So the two are separate bets, and this spike treats them as such:

| Bet | What stresses it | When | Character |
|---|---|---|---|
| **A — sync convergence + partition + bandwidth economy** over a real adverse WAN | Cape York ↔ Dorrigo over Starlink/WireGuard (have it *now*) | this week | **design-validity** (is the wire protocol / convergence model *right*) |
| **B — projection & keystore cost** on weak hardware | a Pi-5-class node on a flaky link (have it *next week*) | next week | **performance** (with a documented mitigation ladder already in hand) |

Design-validity is the harder thing to retrofit, so Bet A is the higher-value thing to learn early — and
it is exactly the bet the available environment stresses. Bet B is the documented go/no-go on the
[ADR-0001](../spec/decisions/0001-fat-postgres-thin-daemon.md) compute bet; its mitigation ladder
(PL/pgSQL → pgrx → external Rust, [ADR-0002](../spec/decisions/0002-in-database-rust-pgrx-escape-hatch.md))
means a "slow" result is a tuning task, not a design failure.

Both bets ride **one shared prerequisite**: a minimal walking skeleton (§3). Build it once; run it on the
WAN now and on the Pi next week.

---

## 2. What this spike is *not*

- **Not a product.** No clinical UI, no FHIR façade, no matcher, no break-glass, no real demographics.
- **Not the full envelope.** It reserves the day-one *shape* (§3, §4) but stubs everything whose absence
  doesn't change the bet (rich contributor sets, comparator profiles, rendition sets, the keystore
  hierarchy beyond a single DEK).
- **Not a security review.** WireGuard is assumed as the transport; the [§7](../spec/security.md) trust
  model (mTLS, actor registry, distribution plane) is out of scope here.

---

## 3. The walking skeleton (shared prerequisite)

The smallest thing that is *genuinely* the architecture, not a mock of it. On each node: PostgreSQL ≥ 18,
a signer, a verifier, a thin Rust ship/apply loop on logical decoding, and one real trigger-maintained
projection.

1. **Event envelope table** carrying the [§3.5](../spec/data-model.md#35-event-storage-model-hybrid-envelope)
   day-one columns — the can't-retrofit set, reserved now even where stubbed:
   - `event_id` (UUIDv7), `hlc` (`t_recorded` ceiling, [§3.6](../spec/data-model.md#36-bitemporal-event-time-recording-time-vs-effective-time)),
     `t_effective` (freely backdatable assertion),
   - `schema_version` (the [§3.13](../spec/data-model.md#313-schema-evolution-event-format-and-the-legibility-twin)
     join key),
   - **`signed_bytes` (BYTEA)** — the opaque canonical-CBOR event, *the* signed artifact (§4, move 1),
   - `body` (JSONB) — a **derived view** parsed *from* `signed_bytes`, for indexing/projection only,
     never re-serialized back,
   - `digest` (self-describing multihash) and `signature` (COSE_Sign1) (§4),
   - `plaintext_twin` (TEXT) — the mandatory [§3.13](../spec/data-model.md#313-schema-evolution-event-format-and-the-legibility-twin)
     legibility twin,
   - an **encryption-capable body slot** indicator + DEK-wrap placeholder ([§3.8](../spec/data-model.md#38-erasure-and-key-custody)) — stubbed seal path,
   - an **attachment-reference** shape ([§3.14](../spec/data-model.md#314-attachments-content-addressed-blobs-and-the-rendition-set)) — one real blob ref, BLAKE3-addressed (§4),
   - a minimal **contributor** field ([§3.9](../spec/data-model.md#39-authorship-and-accountability)) — single author is enough.
2. **Signer.** Serialize the event deterministically → `signed_bytes`; compute `digest`; produce a
   COSE_Sign1 Ed25519 signature (§4).
3. **Verifier.** Hash and verify **over the stored bytes** (§4, move 1). Runs in-DB via pgrx where it
   gates an invariant, external for the spike harness otherwise.
4. **Thin Rust ship/apply loop.** Logical decoding (`pgoutput`/wal2json) →
   ship over WireGuard → apply as **idempotent set-union** keyed on `(event_id, digest)`
   ([§6.1](../spec/sync.md#61-mechanism)). Carries **no merge logic** ([§9.4](../spec/language-substrate.md#94-merge-projection-boundary-fat-postgres-thin-rust-daemon)).
5. **One real projection.** A trigger-maintained (`AFTER INSERT`) incremental table — minimally a
   per-patient demographics-current or an event-watermark projection — so Bet B measures the *actual*
   projection-maintenance path, not a stand-in.
6. **A lazy byte tier stub** ([§6.6](../spec/sync.md#66-attachments-the-lazy-byte-tier)): a separately
   budgeted, preemptible, chunked blob transfer with content-verification on fetch — enough to run the
   §5 availability-floor test.

---

## 4. Serialization, signature, and digest primitives

The biggest available advantage over a naïve "canonical JSON + Ed25519" is **not a cleverer primitive**;
it is three structural moves that shrink the safety-critical surface and make the primitive choice
*reversible*. Those moves are the load-bearing commitments; the concrete primitives are tagged,
migratable defaults the spike validates.

### 4.1 The three structural moves (load-bearing)

1. **Sign the stored bytes; parse a view; never re-serialize.** The signed artifact is an **opaque byte
   string** (`signed_bytes`); the structured form is *parsed out of* those exact bytes, never round-tripped
   back. Verification is `hash(stored_bytes)` + signature-check — never a re-encode. This shrinks the
   determinism burden from *"every implementation must canonicalize identically, forever"* to *"the signer
   serialized once; everyone else byte-compares."* It is already implied by
   [§3.13](../spec/data-model.md#313-schema-evolution-event-format-and-the-legibility-twin) ("signature
   covers a canonical byte form, never re-serialized JSONB") and the
   [§3.14](../spec/data-model.md#314-attachments-content-addressed-blobs-and-the-rendition-set) lossless
   passthrough; this spike makes it explicit and load-bearing.
2. **Self-describing, algorithm-tagged digests and signatures.** Every digest carries a multihash prefix
   and every signature a COSE `alg` header, so the day-one choice is reversible *by policy*, not baked into
   the byte layout. This is what makes everything in §4.2 low-stakes — "wrong" is a migration, not a rewrite.
   It extends the [§3.14](../spec/data-model.md#314-attachments-content-addressed-blobs-and-the-rendition-set)
   self-describing-digest commitment to the event digest and signature.
3. **Re-attestation is an overlay — so crypto-migration is free.** An immortal, verify-forever record will
   eventually outlive any one primitive's strength. The append-only model already has the mechanism:
   "re-sign this event under a stronger primitive" is just another overlay event referencing the original,
   exactly like a correction. We do not need the future-proof primitive in the bytes today; we need the
   tag from move 2 plus the recognition that re-signing is overlay, never mutation. This is what defers the
   post-quantum cost safely (§4.3).

### 4.2 Day-one defaults (tagged, migratable)

| Concern | Default | Why it beats the naïve choice |
|---|---|---|
| **Event serialization** | **Deterministic CBOR** ([RFC 8949](https://www.rfc-editor.org/rfc/rfc8949) §4.2 / CDE profile) inside a **COSE_Sign1** ([RFC 9052](https://www.rfc-editor.org/rfc/rfc9052)) envelope | Binary-native (no base64 bloat for the many digests/keys/sigs), **compact** → directly helps the Bet-A bandwidth economy, far smaller determinism edge-case surface than canonical JSON, and a standardized, `alg`-tagged signature structure with **native multi-signer support** for [§3.9](../spec/data-model.md#39-authorship-and-accountability) contributor sets. Human-legibility is *not* sacrificed: the [§3.13](../spec/data-model.md#313-schema-evolution-event-format-and-the-legibility-twin) plaintext twin already owns it, which frees the signed form to optimize purely for determinism + compactness + a small verifier. |
| **Event signature** | **Ed25519** ([RFC 8032](https://www.rfc-editor.org/rfc/rfc8032)) | Fast and small on Pi-class hardware, deterministic nonce (no ECDSA RNG footgun), libsodium/OpenSSL-clean, and the *same primitive the WireGuard transport already runs* — one fewer family in the trusted base. Carried under a COSE `alg` tag so PQC is a later overlay, not a format break. |
| **Event digest** | **SHA-256**, multihash-wrapped | Ubiquitous, often in-silicon, pgcrypto-native, the conservative default; the wrapper keeps it per-digest migratable. |
| **Blob digest (attachments)** | **BLAKE3**, multihash-wrapped | See §4.4. |
| **Steward / institutional key custody** | **Ed25519 now; FROST threshold ([RFC 9591](https://www.rfc-editor.org/rfc/rfc9591)) earmarked** | Quorum custody + clean rotation for the high-value distribution-plane key ([§7.6](../spec/security.md#76-the-software-distribution-plane-signed-releases-and-extension-load)) and institutional keys ([ADR-0011](../spec/decisions/0011-actor-registry-version-pinning-and-key-custody.md)); layers *on top of* a Schnorr/Ed25519 key, so no envelope change. Out of scope to *implement* in this spike — recorded so the day-one shape doesn't preclude it. |

All of the above are open RFCs / open specs with multiple independent implementations, no patent
encumbrance, no HSM or network required to verify offline — clean against vendor independence (principle 7)
and AGPL. The safety-critical verify path (COSE parse + Ed25519 verify + multihash) is a small, reviewable
Rust surface, run in-DB via the [ADR-0002](../spec/decisions/0002-in-database-rust-pgrx-escape-hatch.md)
pgrx hatch (`coset` + `ed25519-dalek` + `ciborium`), with pgcrypto covering SHA-2.

> [!NOTE]
> Move 1 defuses the one maturity risk here: CDE / deterministic-CBOR is still a draft profile, but because
> verifiers **byte-compare stored bytes** rather than re-canonicalize, only the *signer* needs a fixed
> encoding we control — we never bet correctness on a canonicalization standard being finalized.

### 4.3 Honest dismissals (alternatives weighed and not taken)

- **BLS (BLS12-381) signature aggregation** — real advantage (many sigs → one constant-size aggregate) but
  pairing crypto is heavy on a Pi, a much larger reviewer surface (principle 8), and the payoff doesn't
  materialize when most events have a single author.
- **Post-quantum ML-DSA / SLH-DSA (FIPS 204/205)** — the one alternative with a mission-deep rationale
  (records are immortal), but paying the cost now is wrong: SLH-DSA signatures are 8–50 KB (murders the
  bandwidth bet *and* the Pi), ML-DSA libraries are far less battle-tested than Ed25519. Move 3 is the
  answer — tag the primitive, re-attest under ML-DSA as an overlay when the threat clock demands it.
- **RSA / ECDSA-P256** — only advantage is legacy PKI / smartcard interop (e.g. national e-signature
  regimes); belongs at the interop boundary, handled by the move-2 `alg` tag, never as the internal default.
- **Protobuf / Avro for the signed form** — Protobuf is explicitly *not* deterministic across
  implementations; Avro needs a schema registry — the exact central coupling
  [§6.5](../spec/sync.md#65-schema-evolution-two-planes-and-lossless-forwarding) routes around. Fine inside
  projections, never as the signed bytes.

### 4.4 BLAKE3 for blobs (the attachment digest)

The blob tier is content-addressed ([§3.14](../spec/data-model.md#314-attachments-content-addressed-blobs-and-the-rendition-set)),
so its digest is a parallel choice to the event digest — and here the default diverges from SHA-256 on
purpose. **BLAKE3's internal tree (Merkle) structure lets chunks of a blob be verified independently**,
which is a direct structural fit for the [§6.6](../spec/sync.md#66-attachments-the-lazy-byte-tier)
**chunked, preemptible, resumable, multi-source swarm** byte tier:

- A gigabyte DICOM fetched over Starlink can be **preempted** mid-transfer (the
  [ADR-0013](../spec/decisions/0013-attachments-content-addressed-lazy-blob-tier.md) availability floor) and
  **resumed** later, verifying each arrived chunk against the tree without re-hashing the whole blob.
- Chunks pulled from **different sources** (LAN sibling, parent, patient-carried device) each
  self-verify as they land — the swarm-fetch property, with zero trust in any source.
- BLAKE3 is also **fast on weak/ARM hardware** and parallelizes, which matters for the Pi (Bet B measures
  exactly this — §6).

SHA-256 stays the conservative **event** digest; BLAKE3 is the **blob** digest. Both are multihash-wrapped,
so the choice is per-digest and migratable, and a node that meets an unfamiliar digest algorithm degrades
to honest "can't verify here" rather than mis-verifying — the legibility-ladder pattern applied to hashing.
The blob still carries **no separate signature**: the signed event names it by BLAKE3 digest and the event
signature covers that digest ([§3.14](../spec/data-model.md#314-attachments-content-addressed-blobs-and-the-rendition-set)).

---

## 5. Bet A — WAN-sync validation (Cape York ↔ Dorrigo, now)

**Setup.** Skeleton (§3) on both nodes; WireGuard over the two Starlink links; a load generator that emits
realistic clinical-event streams plus one large blob, with controllable partitions (drop/restore the
WireGuard interface) and injectable clock skew.

**Measure / assert:**

| # | Question | Method | PASS threshold |
|---|---|---|---|
| A1 | Does set-union **converge** after an arbitrary partition? | Partition; write on both sides (including conflicting overlays on the same patient); restore | Both nodes reach an **identical event set and identical projections**, deterministically, with no operator intervention |
| A2 | Do signatures survive the wire? | Verify every applied event on the receiver | **Zero** verification failures attributable to serialization round-trip (move 1 should make this structurally impossible — a failure here is a bug, not noise) |
| A3 | Does HLC ordering hold under real latency + skew? | Inject clock skew; check causal order and the `t_recorded` ceiling | Causal order preserved; `t_recorded` never precedes a cause; skew flagged, never silently reordered |
| A4 | Does the **availability floor** hold? | Start a multi-hundred-MB blob fetch *during* a burst of clinical writes | Clinical-event sync **p95 latency unaffected** by the concurrent blob transfer; blob is chunked/preemptible ([ADR-0013](../spec/decisions/0013-attachments-content-addressed-lazy-blob-tier.md)) |
| A5 | Is the eager plane **slim**? | Measure bytes-on-wire per clinical event (excluding blobs) | Within target budget over a metered link (record actual; target on the order of a few KB/event) |
| A6 | Is assembly-state **honest**? | Reference a blob whose bytes haven't arrived | Peer shows *"referenced here — not yet retrieved"* ([§6.2](../spec/sync.md#62-consistency-model)), never a silent absence |

**FAIL signals & what they'd mean:** divergence (A1) → the merge model is wrong; signature breakage on the
wire (A2) → a canonicalization/round-trip bug slipped past move 1; blob transfer starving clinical sync
(A4) → byte-tier isolation is priority-ordering, not the separate budget ADR-0013 requires.

---

## 6. Bet B — projection & keystore cost on the Pi (next week)

**Setup.** The *same* skeleton on a Raspberry-Pi-5-class node (rural-clinic profile, low concurrency,
[§8](../spec/deployment.md)), on a deliberately flaky link.

**Measure / assert:**

| # | Question | Method | PASS threshold |
|---|---|---|---|
| B1 | Is **projection maintenance** cheap? | Time the `AFTER INSERT` trigger path at rural-clinic write rates | Single-op maintenance well within interactive budget; no unbounded growth with log size |
| B2 | Does a **chart read** beat paper? | Time a realistic multi-event chart assembly from projections | Faster than "grab the paper chart" — the [§1.2](../spec/vision.md) paper-parity floor (record the distribution; target sub-second) |
| B3 | What does the **keystore** cost? | Crypto-shred ([ADR-0005](../spec/decisions/0005-erasure-key-custody-and-crypto-shredding.md)) at per-event vs per-episode DEK granularity | A granularity whose key-management cost is acceptable on the Pi; informs the §3.8 key hierarchy |
| B4 | Crypto throughput on ARM? | Ed25519 verify/s; BLAKE3 vs SHA-256 hashing throughput | Verify + hash keep up with sync + chart-read load; confirms or revises the §4 blob-digest default on real ARM |

**Mitigation ladder if a threshold misses** (per [ADR-0002](../spec/decisions/0002-in-database-rust-pgrx-escape-hatch.md)):
PL/pgSQL → **pgrx (in-DB Rust)** for the hot projection → external Rust as the last resort. A miss tells us
*which rung*, not *whether the design works*.

---

## 7. Exit criteria → ratification

When both bets have run:

1. **If Bet A passes**, fold the three structural moves (§4.1) and the validated primitive defaults (§4.2,
   §4.4) into a new **ADR** (the serialization / signature / digest decision) and add back-pointers from
   [§3.5](../spec/data-model.md#35-event-storage-model-hybrid-envelope) / [§3.13](../spec/data-model.md#313-schema-evolution-event-format-and-the-legibility-twin) / [§3.14](../spec/data-model.md#314-attachments-content-addressed-blobs-and-the-rendition-set).
   *If A2 or A4 reveal a flaw, the ADR records the revised choice instead.*
2. **If Bet B passes at PL/pgSQL**, ADR-0001's load-bearing bet is confirmed at the lowest rung. **If it
   needs pgrx**, that is the expected ADR-0002 outcome and is recorded as such. **If even external Rust
   can't meet B2**, that is a genuine go/no-go signal to revisit the projection model.
3. Either way, the skeleton becomes the seed of the real implementation — it is built to be the
   architecture, not thrown away.
