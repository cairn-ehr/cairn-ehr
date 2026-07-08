# Spike 0001 — Walking Skeleton, WAN-Sync Validation, and Pi Cost

- **Status:** Bet A **PASS** (run 2026-06-16 over the Cape York ↔ Dorrigo WireGuard link — see §8; the run also
  surfaced and fixed a real availability-floor bug in the field `run` loop, §8.1) → **§4 primitives ratified as
  [ADR-0015](../spec/decisions/0015-event-serialization-signatures-and-content-addressing.md)** (blob-digest
  line was provisional pending Bet B — **now confirmed**, ADR-0015 Resolution 2026-07-07). **Bet B (Pi) PASS**
  (run 2026-06-25 on a Raspberry Pi 5 / **8 GB** — see
  §9): all §6 gates green with large headroom; the **B4 ARM crypto number confirms ADR-0015's BLAKE3
  blob-digest default** (BLAKE3 ~4× SHA-256 on Cortex-A76). The 2026-06-25 run carried two honest caveats
  (USB-2-limited storage §9.2; ran on PG 16 because `cairn_pgx` couldn't build on PG 18 §9.3) that cost
  *precision, not the verdict* — **both are now resolved by the clean PG 18 + NVMe re-run (2026-07-07, §9.5)**:
  B1 held p95 3.99 ms *below* the old number **at 10× the log size** (2 M events), flat ×2.50; B5 ×1.40
  FK-index shrink at 2 M rows; crypto reproduced within noise. **Bet B is now caveat-free.**
- **Date:** 2026-06-16 (Bet A); 2026-06-25 (Bet B); 2026-07-07 (Bet B clean re-run, §9.5)
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

## 6. Bet B — projection & keystore cost on the Pi (prepared; awaiting the board)

**Setup.** The *same* skeleton on a Raspberry-Pi-5-class node (rural-clinic profile, low concurrency,
[§8](../spec/deployment.md)), on a deliberately flaky link. **Target board:** a Raspberry Pi 5 / 16 GB
with a 1 TB SSD (the suspected realistic floor); a **Pi 4 / 8 GB** is the follow-on floor experiment
(older, cheaper hardware — does it still clear the gates?). The full field procedure is
`poc/walking-skeleton/PI-RUNBOOK.md`.

**Measure / assert:**

| # | Question | Method | PASS threshold |
|---|---|---|---|
| B1 | Is **projection maintenance** cheap? | Time the `AFTER INSERT` trigger path at rural-clinic write rates | Single-op maintenance well within interactive budget; no unbounded growth with log size |
| B2 | Does a **chart read** beat paper? | Time a realistic multi-event chart assembly from projections | Faster than "grab the paper chart" — the [§1.2](../spec/vision.md) paper-parity floor (record the distribution; target sub-second) |
| B3 | What does the **keystore** cost? | Crypto-shred ([ADR-0005](../spec/decisions/0005-erasure-key-custody-and-crypto-shredding.md)) at per-event vs per-episode DEK granularity | A granularity whose key-management cost is acceptable on the Pi; informs the §3.8 key hierarchy |
| B4 | Crypto throughput on ARM? | Ed25519 verify/s; BLAKE3 vs SHA-256 hashing throughput | Verify + hash keep up with sync + chart-read load; confirms or revises the §4 blob-digest default on real ARM |
| B5 | Does **surrogate-key interning** actually pay on ARM? ([ADR-0031](../spec/decisions/0031-canonical-identifiers-and-node-local-surrogate-keys.md)) | Build the projection two ways — canonical `uuid`/`bytea` FKs vs `bigint` `local_ref` FKs interned via a dictionary — and compare: FK index size, chart-read join time, buffer-cache residency, and the `submit_event` interning-upsert cost on ingress | `bigint`-surrogate FK indexes materially smaller and chart-read joins **no slower** (ideally faster); ingress interning-upsert cost within the B1 maintenance budget — confirms or **narrows** the ADR-0031 interning scope |

**Mitigation ladder if a threshold misses** (per [ADR-0002](../spec/decisions/0002-in-database-rust-pgrx-escape-hatch.md)):
PL/pgSQL → **pgrx (in-DB Rust)** for the hot projection → external Rust as the last resort. A miss tells us
*which rung*, not *whether the design works*.

> [!NOTE]
> **B5 — surrogate-key interning is a *scope*-finding measurement, not a go/no-go.**
> [ADR-0031](../spec/decisions/0031-canonical-identifiers-and-node-local-surrogate-keys.md) fixes the
> *discipline* (canonical UUID/multihash identity on the wire; node-local `bigint` surrogates as the
> physical join key, never escaping the projection) as a design decision; B5 measures only *how much* the
> interning pays on real ARM and *which* references are worth interning. The candidates, in cost order, are
> the wide random `BYTEA` references (`content_address`/`actor_id`/`blob_address`) and the high-fan-out
> `patient_id` — `event_id` PKs stay UUIDv7. Like [ADR-0001](../spec/decisions/0001-fat-postgres-thin-daemon.md)'s
> compute bet, a "no measurable win" result on the Pi **narrows** the scope (keep UUIDv7-only where interning
> doesn't earn its indirection) rather than overturning the discipline. This row touches **only** the
> fit-for-purpose projection schema + harness; the §9 Rust safety surface is untouched.

#### 6.2 B5 — prepared (2026-06-22): the guard + bench artifacts

B5 is **built and runnable**; only the ARM numbers await the board. The artifacts are pure SQL
(no pgrx, no Rust rebuild — the discipline lives wholly in the projection plane), driven by one runner:

- **`db/008_surrogate_projection.sql`** — the dual-identifier projection: the `local_ref` domain (the
  type-system leakage guard), the `patient_ref` interning dictionary (the anchor row carrying *both*
  fields), the `intern_patient`/`patient_uuid` ingress/egress chokepoints, and two child projections of
  note events — `chart_note_u` (16-byte canonical-UUID FK) vs `chart_note_s` (8-byte surrogate FK) —
  maintained from the *same* event stream so the A/B is honest.
- **`db/tests/008_surrogate_test.sql`** — the **leakage guard** (G1–G6), the load-bearing half: it
  mechanically asserts the surrogate never reaches the signed plane (`event_log` is surrogate-free —
  the load-bearing guarantee, since the signed plane is typed `uuid` and `bigint <> uuid`), that the
  `local_ref` domain gives the one-directional guard it actually can (a `uuid` won't coerce to a
  surrogate) while being honest that it is *not* a two-way barrier, interning is idempotent/dense, the anchor carries both while
  referencing rows carry only the surrogate, and egress rehydrates the canonical UUID. Runs in review, off-Pi.
- **`db/bench/b5_surrogate.sql`** + **`db/bench/run_b5.sh`** — seeds N patients × M notes and reports the
  FK-index size ratio (B5.1), heap sizes (B5.2), and `EXPLAIN (ANALYZE, BUFFERS)` of the UUID-keyed vs
  surrogate-keyed "all notes for one patient" read (B5.3/B5.4). On-Pi invocation: PI-RUNBOOK §6.1.

**x86 sanity run** (dev box, PG 16, 2 000 patients × 50 notes = 100 000 child rows): guard **ALL PASS**;
FK-index `shrink_factor` **≈1.3×** with random-UUID keys, surrogate read one extra single-row anchor hit.
The ~1.3× is a *floor* on the win — the real ARM target uses larger fan-out, and UUIDv7's k-sortable index
on the dev box already removes the random-insertion penalty that a national `content_address` (random
multihash) FK would still pay; the Pi run is where the verdict lands.

**Deliberate scope choice:** B5 is **not** wired into the default `init` schema (`crates/cairn-sync` /
`crates/cairn-node`). A second always-on projection trigger would fold surrogate-maintenance cost into the
**B1** measurement and conflate the ADR-0001 question. So B5 is an opt-in bench loaded by its own runner;
folding interning into the *real* projection is a product-build step, not a shadow projection bolted onto
the skeleton. The §9 Rust safety surface is untouched; `cargo test --workspace` is unaffected.

### 6.1 Preparation status — runbook + a self-describing, floor-finding harness (2026-06-18)

Bet B is **prepared and waiting only on the physical board.** The harness
(`poc/walking-skeleton/harness/bench_b.py`) and the daemon commands it drives
(`bench-insert`, `chart`, `bench`, `gen`) were built and validated green on x86; the remaining work was to
make the run **reproducible and its numbers trustworthy on real hardware**, which this preparation did:

- **A field runbook** — `poc/walking-skeleton/PI-RUNBOOK.md`: SSD-not-SD-card (the one
  Pi mistake that silently invalidates B1/B2), PostgreSQL 18 on arm64 via PGDG, the `performance` governor +
  cooling + throttle check, deployment-honest PG tuning (`fsync`/`synchronous_commit` left **on** — a clinic
  node must survive power loss), the release build on the Pi, and the prescribed run.
- **A self-describing environment header on every run.** Bet B is a *hardware-class* bet, so a §6 number is
  meaningless without its host — and the host matters concretely (the same release binary measured SHA-256 at
  ~1500 MB/s on a SHA-NI host and ~200 MB/s on one without). The harness now records board, cores/RAM, kernel,
  PG version, **which block device PGDATA sits on** (shouting if it's an SD card), CPU governor, Pi throttle
  state (`vcgencmd get_throttled`), and the build profile — and warns on anything that would skew the result.
  `--json-out` writes the header + thresholds + every row as a durable artifact; `--label` tags the board.
- **The floor question is answerable from one board.** Each gated row prints its **headroom** (× under budget
  / over floor). Big headroom on B1/B2 means the projection/chart path is nowhere near the constraint, so the
  floor is set by the *smallest*-headroom row (expected to be **B4** raw crypto, which tracks clock/core, not
  tuning). So the Pi 5 / 16 GB run already predicts whether a smaller board is viable, before it is plugged in.
- **Realistic B2.** The demographic panel size is now a harness parameter (`--patients`), so the "fattest
  patient" chart read is a realistic heavy multimorbid chart (~`count/patients` notes), not a degenerate one.

The Rust safety surface was **not** touched (the §9 blast-radius discipline — all of the above is
fit-for-purpose Python in the harness); `cargo test --release` stays green (6/6).

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

---

## 8. Bet A — results (Cape York ↔ Dorrigo, 2026-06-16) — **PASS** (and one real bug, fixed)

Ran the §5 table over the real link: a MacBook (**Cape York** node, WireGuard `10.0.0.2`, PostgreSQL 16) and
the **DGX Spark** (**Dorrigo** node, WireGuard `10.0.0.3`, a user-owned **PostgreSQL 18.4** instance). **The
link was genuinely adverse — a satellite path with ~710 ms RTT** (`ping` min/avg/max 667/710/775 ms, with
loss), which is exactly the design-validity stress Bet A exists to apply.

Exercised through the **unattended field path** — `cairn-sync run` on each node (serve + pull + lazy blob
fetch on a timer, drop-resilient, one JSON line/cycle) summarised by `bet_a.py analyze` per node and
cross-compared with `bet_a.py report`. Scenario: a partition (each node writes independently, plus a
**conflicting demographic overlay on one shared patient**), then both nodes `run` for 150 s under continuous
2 ev/s clinical load, with a 2 MB DICOM blob put on Dorrigo and referenced on Cape York (the lazy-fetch / A4
/ A6 case).

| # | Question | Result | Detail (clean canonical run) |
|---|---|---|---|
| **A1** | set-union converges after partition | **PASS** | both nodes reach **792 events, event-hash AND projection-hash identical**; the conflicting shared-patient overlay resolved to the **same winner on both sides** (`Alma Tjapaltjarri (Dorrigo)`, deterministic HLC `(wall, counter, origin)` tie-break), no operator intervention |
| **A2** | signatures survive the wire | **PASS** | **0** verify-failures on apply across the whole event set, both directions (move 1 — sign-the-stored-bytes — makes this structural) |
| **A3** | HLC ordering under latency + skew | **PASS** | local clock merged past every applied event on both nodes; max HLC↔record gap **reported** (Cape York 35 s / Dorrigo 42 s — the partition window), **flagged, never auto-resolved** |
| **A4** | availability floor | **PASS (after the fix below)** | clinical sync ran 30 cycles to full convergence (median inter-cycle gap **5.0 s**, max 11.5 s) **while** the blob fetched lazily on a separate tier — no head-of-line stall |
| **A5** | eager plane slim | **PASS** | **494–495 B/event** on the clinical plane (budget 4096) — directly the deterministic-CBOR/COSE compactness bet |
| **A6** | honest assembly-state | **PASS** | the referenced-but-unfetched blob shows as **referenced-not-present** on the fetching node, never a silent absence (it was still in-flight at the 150 s cutoff — the tier yielding to clinical work, exactly as intended) |

**Bet A: PASS — proceed to ratify the §4 primitives** (the per-§7.1 ADR; optionally gated on Bet B's ARM
crypto-throughput number, which touches the §4.4 blob-digest default). The set-union/signature/HLC/bytes
core was independently corroborated by an earlier two-node SSH-driven run (442 events, same verdicts) before
the field path existed.

### 8.1 The real bug the link surfaced — an availability-floor violation in the run loop (fixed)

The first canonical run **FAILED A1** — and the failure was instructive, not noise. The field `run` loop
fetched blobs **inline in the clinical pull cycle** (`do_blobd` runs a whole-blob fetch to completion each
cycle). On the 710 ms link, the 2 MB blob's ~32 sequential round-trips **head-of-line-blocked the Cape York
node's entire 150 s run**: it logged **one cycle**, pulled **zero** clinical events, and never converged
(396 vs 792) — even though its *serve* thread happily fed Dorrigo the whole time. That is precisely the
failure [ADR-0013](../spec/decisions/0013-attachments-content-addressed-lazy-blob-tier.md) names: **byte
transfer reducing clinical-data availability** (the Kimberley nightly-imaging-grinds-everything-to-a-halt
case, reproduced in miniature on a real satellite link).

**Fix (in this change):** the lazy byte tier now runs on **its own thread** in `cairn-sync run` — like the
serve thread — so blob fetching is on a separate cadence and **can never block the clinical pull loop**
(the ADR-0013 *separately-budgeted byte tier*, not mere priority ordering). Re-running: Cape York completed
**30 clinical cycles and fully converged** while the same blob fetched lazily in the background. `cargo test`
+ `clippy` green.

### 8.2 Carried into the real byte-tier build (not blocking, deferred)

The skeleton's `do_blobd` is still a **stub** in two ways the link exposed, both already mandated by
ADR-0013 and left for the production byte tier:

1. **Pull is synchronous, one round-trip per 64 KiB chunk.** Latency, not bandwidth, binds: a 64 MB blob is
   ~1024 sequential RTTs (~12 min) on this link regardless of throughput. The real tier must **pipeline/window
   many chunks in flight** and **pull from multiple sources** (swarm) — BLAKE3's independent-chunk
   verification (§4.4) is what makes that windowing safe.
2. **The whole-blob fetch is not resumable across passes** — it restarts from offset 0 on any mid-fetch drop,
   so on a flaky high-latency link a large blob may never complete within a session (in the clean run the
   2 MB blob was still `referenced-only` at cutoff). The real tier must **persist partial bytes and resume**
   (ADR-0013 *chunked/resumable*). Moving the fetch off the clinical loop (8.1) is the **availability** fix;
   pipelining + resumability is the **throughput** fix.

**STATUS — implemented (PR #12) and DELIVERED on the real link 2026-06-16.** Both deferred items above are
built: `do_blobd` is **windowed** (worker pool, `--window N` ≤16), **resumable** (verified slices persist in a
`blob_chunk` set-union table; a restart fetches only the missing indexes), **multi-source swarm**
(`--blob-peer` repeatable, per-slice failover), and **per-slice BLAKE3 verified** via `bao`
(`cairn-event::verify_slice` — a lying/faulty source is rejected per-slice and healed by another). Slices
travel as **raw binary frames** (not hex, which would halve measured throughput).

**Real Cape York ↔ Dorrigo run (2026-06-16, ~680–860 ms-RTT WireGuard satellite link;** MacBook 10.0.0.2
fetcher ↔ DGX Spark 10.0.0.3 source, PG18 :5444; driver `harness/wan_spike.py`, raw log alongside it).
**All three claims the local selftest can't exercise PASS:**

| Claim | Result |
|-------|--------|
| **T1 windowing / RTT reduction** | 4 MB fetched in **21.4 s windowed (w8)** vs **101 s sequential (w1) = 4.7×**, single pass. Round-trips collapse from ~64 sequential (the 64 KiB stub) to ~2 windowed waves. |
| **T2 resume across a real drop** | Killed mid-fetch at 25 s → **14/16 verified slices persisted**, then resumed from `blob_chunk` and completed (only the 2 missing indexes refetched). |
| **T5 availability floor** | Clinical pull p95 **3064 → 3918 ms (+28 %)** during a concurrent windowed fetch at `--budget-ms 20`; **clinical sync never stalled** (the ADR-0013 floor — availability preserved). |

**Tuning findings (the "choose `SLICE_BYTES`/`--window`" deliverable):**
- **Throughput is RTT-bound, ~0.2 MB/s per link.** The cost is dominated by a **fresh TCP connection +
  slow-start per slice** on a high bandwidth-delay-product link, so throughput comes from **parallel flows**,
  not bigger slices: window sweep at 4 MB gave **w4 0.12 / w8 0.19 / w16 0.17 MB/s** (peaks ≈ **window 8**,
  16 flows give no further gain — bandwidth/contention saturated), and a *larger* slice was *worse*
  (256 KiB 0.19 → 1 MiB 0.16 → 4 MiB 0.11 MB/s, the 4 MiB case being a single slow-starting flow).
- **Keep `SLICE_BYTES = 256 KiB`** (confirmed good) and **default `--window` 4–8** (8 for throughput, lower
  to stay floor-conservative). On a *shared-bandwidth* link the availability-floor knob is **window width
  (concurrency)**, not `--budget-ms` (its inter-request sleep is negligible against a ~750 ms RTT).
- **Next throughput lever (beyond this spike):** connection reuse / persistent streaming instead of one TCP
  connection per slice — the production object-store tier already noted in the skeleton README. Windowing
  delivers the §8.2 requirement (parallel, resumable, verified); the per-connection cost is the next ceiling.

---

## 9. Bet B — results (Raspberry Pi 5 / 8 GB, 2026-06-25) — **PASS** (with two honest caveats)

Ran the §6 table on a **Raspberry Pi 5 Model B / 8 GB** (4× Cortex-A76 @ 2.4 GHz, Debian 13 / kernel 6.18,
active cooling, `performance` governor, `throttle=0x0` throughout), against a release arm64 `cairn-sync`
built natively on the board (~75 s). This is a *more constrained* board than the runbook's Pi 5 / 16 GB
target — between the spec's "Pi 5 class" floor and the Pi 4 / 8 GB follow-on candidate — so the headroom it
shows is a stronger floor signal than the 16 GB board would give.

The artifacts are committed next to this spike at
[`poc/walking-skeleton/results/`](../../poc/walking-skeleton/results/) (`betb-pi5-full.json`,
`betb-pi5-crypto.json`, `betb-pi5-b5.log`) — each self-describing per §6.1.

| # | Question | Result | Detail |
|---|---|---|---|
| **B1** | projection maintenance cheap + flat as the log grows? | **PASS** | p95 **4.38 ms** @ 202,000 events — **11× under** the 50 ms budget; growth **×2.15** vs 22,000 events (flat ≤ ×3.0) |
| **B2** | chart read beats paper? | **PASS** | p50 2.4 ms / p95 **2.5 ms** over a 200-note chart — **394× under** the 1 s paper-parity floor |
| **B3** | keystore (crypto-shred) cost? | **INFO** | DEK-wrap **1,581,249/s**, body-seal **242 MB/s** (per-episode unwrap = 1 op; per-event = N) |
| **B4** | crypto keeps up on ARM? | **PASS** | Ed25519 **5,490 verify/s** (2.7× over the 2,000 floor); **BLAKE3 915 vs SHA-256 230 MB/s** — BLAKE3 ~4× faster ⇒ **ADR-0015's blob-digest default holds on ARM** |
| **B5** | does surrogate-key interning pay on ARM? ([ADR-0031](../spec/decisions/0031-canonical-identifiers-and-node-local-surrogate-keys.md)) | **CONFIRMS (narrows nothing)** | FK-index **shrink ×1.39** (4880→3512 kB / 500k rows); heap 37→33 MB; surrogate read **8 vs 5 buffer hits** (one extra single-row anchor lookup, both sub-ms all-cache) ⇒ interning pays *and* the read stays competitive. Leakage guard **G1–G6 ALL PASS** |

**Verdict: Bet B PASS — go on Pi-5-class hardware at the lowest (PL/pgSQL + pgrx) rung.** ADR-0001's compute
bet holds; no mitigation-ladder escalation was needed.

### 9.1 The floor read (headroom)

The smallest-headroom gate is **B4 raw crypto (2.7× over floor)** — exactly as §6.1 predicted, since crypto
tracks clock/cores, not tuning or storage. B1 (11×) and B2 (394×) sit far from their budgets. Because even
the *constrained* 8 GB board with *crippled USB-2 storage* (below) clears B1/B2 by 11×/394×, a Pi-5-class
node is comfortably inside the envelope, and the B4 margin says a somewhat weaker core still has room. The
go/no-go verdict is therefore **robust to both caveats below — they cost precision, not the conclusion.**

### 9.2 Caveat 1 — storage ran on a USB-2-limited dock (and the power saga that forced it)

The board on hand was the **worst-case rig** the spike anticipated: 8 GB RAM and the SSD attached **over USB,
not NVMe**. The run surfaced a real, **deployment-relevant power finding** worth recording for
[§8 deployment](../spec/deployment.md):

- **A Pi 5 browns out under combined CPU + bus-powered-SSD load on a generic 5 V/3 A supply — even at only
  ~9–13 W average draw.** The cause is *transient* current spikes (the A76 cores ramp current sub-millisecond)
  sagging the rail past the under-voltage trip, compounded by the firmware capping the USB port budget to
  600 mA on a non-PD supply (which a bus-powered SSD then competes for). Average watts is the wrong metric;
  instantaneous rail sag is the right one. *Two brownouts corrupted the boot SD card mid-write* before the
  cause was nailed down. CPU-only load was always stable; only **CPU + direct-USB-SSD** crashed it.
- **The fix that unblocked the run:** move the SSD onto a **powered hub/dock** so its draw comes off the hub,
  not the Pi. With the SSD offloaded, combined full-load held `throttle=0x0` and the rail stayed ~4.93 V.
- **The cost of that workaround:** the only powered hub available was a multifunction dock whose downstream
  data ports negotiate **USB 2.0** for this drive — so PGDATA storage ran at **~35 MB/s** (vs ~306 MB/s for
  the same SSD direct on USB-3). B1/B2/B5 are therefore **storage-bound and pessimistic**; B3/B4 are
  disk-independent and unaffected. That B1/B2 still pass by 11×/394× *on USB-2* is what makes the verdict
  caveat-robust — real USB-3 only widens the margin.
- A flaky **Samsung T1** bridge also dropped under load through the hub until a **`usb-storage.quirks=
  04e8:8001:u`** kernel param (disable UAS → BOT) stabilised it — a concrete clinic-BOM note for older USB-SATA
  bridges.

> **Deployment-BOM finding (for §8):** a rural-clinic Pi node's **power supply and storage-attachment path are
> part of the validated bill of materials**, not an afterthought. Recipe: an **official 27 W / 5 V·5 A PD
> supply**, a **short 5 A-rated cable**, active cooling, and either a **directly-attached USB-3 SSD** or a
> **true USB-3 powered hub** (verify the SSD enumerates at 5000 Mbps, not 480). Generic phone chargers and
> long/thin cables brown out a Pi 5 under clinical load and can corrupt the boot medium.

### 9.3 Caveat 2 — ran on PG 16, because `cairn_pgx` can't build on PG 18 (a build-prep finding)

`cairn-sync init` requires the in-DB Rust extension **`cairn_pgx`**, which is pinned to **`pgrx =0.12.9`
with only a `pg16` feature** — it **does not build against PostgreSQL 18**, the deployment floor the runbook
tells you to install. So the DB-backed gates (B1/B2/B5) were run on **PG 16.14** (PG 18.4 was also installed,
but only the crypto-only B3/B4 path, which needs no DB, can use it). This is a genuine inconsistency between
`poc/walking-skeleton/PI-RUNBOOK.md` ("record on PG 18") and the code, and a **build-prep
to-do: port `cairn_pgx` to a pgrx release with PG 18 support** before the clean re-run.

**Bonus validation:** `cairn_pgx` *built, linked, installed, and loaded cleanly on Pi 5 arm64* (1.08 MB `.so`,
via `cargo-pgrx` on the board) — so the **§9 in-DB Rust safety surface is confirmed to compile and run on
real ARM**, complementing the Android pgrx result in [Spike 0003](0003-postgres-on-android-bionic-node.md).

### 9.4 Follow-ups

1. **Clean re-run on PG 18 + NVMe + official PSU — DONE 2026-07-07 (§9.5).** Both caveats resolved; the
   authoritative, precision numbers are in. The two blockers cleared: `cairn_pgx` now builds on PG 18 (pgrx
   0.18.1, `pg18` feature — §9.3 caveat gone) and storage moved to a PCIe NVMe HAT (better than the USB-3 SSD
   this item asked for — §9.2 caveat gone). Prediction held: B1 stayed flat *and* faster at 10× the log size.
2. **ADR-0015 — DONE 2026-07-07.** The B4 ARM number (**BLAKE3 913 vs SHA-256 230 MB/s**, reproduced from the
   2026-06-25 915) **confirmed** the provisional blob-digest default; ADR-0015 now carries a dated **Resolution**
   overlay dropping "provisional" from the BLAKE3 blob-digest line.
3. **ADR-0031:** the ×1.39 FK-index shrink with a competitive surrogate read **confirms** the interning
   discipline pays on ARM and **narrows nothing** — interning earns its indirection for the wide/high-fan-out
   references, as designed.

### 9.5 Clean re-run — PG 18 + NVMe, 2026-07-07 — **PASS** (both caveats resolved)

The §9.4 follow-up #1 clean re-run, on the **same Raspberry Pi 5 / 8 GB** board (`Raspie58`), this time with
both 2026-06-25 caveats removed: the SSD is now a **PCIe M.2 NVMe HAT** (not the USB-2-limited dock of §9.2 —
`pg_storage_device: /dev/nvme0n1`, `pg_on_sd_card: false`), the run is on **PostgreSQL 18.4** (not the PG 16
of §9.3), and it ran on the **official 5 A PSU** (`throttle` clean `0x0` throughout, ~55–60 °C, `performance`
governor). The blocker that forced PG 16 last time is gone: **`cairn_pgx` now builds and loads on PG 18**
(pgrx **0.18.1**, `pg18` feature; extension version **0.3.0**, satisfying the daemon's `REQUIRED_PGX_FLOOR`).
This run also pushes the flatness ladder to the full **2,000,000-event** tier the impatient first pass skipped.
Tuning is scaled to the 8 GB board (`shared_buffers 2GB`, `effective_cache_size 5GB`; `fsync` +
`synchronous_commit` left `on` — the safe rural-clinic config). Artifacts:
[`poc/walking-skeleton/results/`](../../poc/walking-skeleton/results/) (`betb-pi5-nvme-pg18-full.json`,
`betb-pi5-nvme-pg18-crypto.json`, `betb-pi5-nvme-pg18-b5.log`).

| # | Question | Result | Detail (this run — PG 18 + NVMe) | 2026-06-25 (PG 16 + USB-2) |
|---|---|---|---|---|
| **B1** | projection maintenance cheap + flat? | **PASS** | p95 **3.99 ms @ 2,004,000 events** — **13× under** the 50 ms budget; growth **×2.50** vs 54,000 events (flat ≤ ×3.0) | p95 4.38 ms @ 202k (11×); growth ×2.15 |
| **B2** | chart read beats paper? | **PASS** | p50 4.3 ms / p95 **4.5 ms** over a **374-note** chart — **222× under** the 1 s paper-parity floor | p95 2.5 ms over 200 notes (394×) |
| **B3** | keystore (crypto-shred) cost? | **INFO** | DEK-wrap **1,587,411/s**, body-seal **242 MB/s** | 1,581,249/s, 242 MB/s |
| **B4** | crypto keeps up on ARM? | **PASS** | Ed25519 **5,501 verify/s** (2.8× over the 2,000 floor); **BLAKE3 913 vs SHA-256 230 MB/s** ⇒ ADR-0015 default holds | 5,490/s; 915 vs 230 |
| **B5** | surrogate-key interning pays? (ADR-0031) | **CONFIRMS** | FK-index **shrink ×1.40** (18→13 MB / **2,000,000 rows**); heap 146→130 MB; surrogate read **8 vs 5 buffer hits** (both sub-ms all-cache). Guard **G1–G6 ALL PASS** | ×1.39 @ 500k rows; 8 vs 5 |

**Verdict: Bet B PASS, now caveat-free.** The headline is B1: the storage caveat mattered even less than
feared — on real NVMe, projection-maintenance p95 landed **below** the old USB-2 number (3.99 ms vs 4.38 ms)
**while running at 10× the log size** (2.0 M vs 202 k events), and stayed flat (×2.50) across a **×37**
log-growth jump (54 k → 2 M). B3/B4 are disk-independent and reproduced the 2026-06-25 crypto numbers within
noise (confirming rig consistency), so **ADR-0015's BLAKE3 blob-digest default is confirmed on ARM without an
asterisk**. Measured on-disk cost at the 2 M tier: **~1,515 B/event** for `event_log` (feeds the sizing
appendix with a real number, well under the ~2–4 KB estimate). The board remains the constrained **8 GB**
(not the 16 GB runbook target), so the headroom shown is still a *stronger-than-target* floor signal.
