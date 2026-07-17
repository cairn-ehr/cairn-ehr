# Design — born-sealed clinical bodies: the seal-by-default posture + walking-skeleton seal slice (issues #189, #92)

**Date:** 2026-07-17 · **Issues:** [#189](https://github.com/cairn-ehr/cairn-ehr/issues/189)
(review finding C1, Critical/window-closing) + [#92](https://github.com/cairn-ehr/cairn-ehr/issues/92)
(erasure-ladder composition) · **Plane:** wire core + in-DB floor + node
· **Tier:** safety-critical (§9 Rust + in-DB) · **Vehicle:** one composing ADR (**ADR-0052**) +
spec prose + a walking-skeleton seal slice on `clinical.medication`

## Problem

ADR-0005's crypto-shred rungs only work on bodies sealed under a per-record DEK **at write
time**, and per-record sealing is off by default. `data-model.md` §3.5 itself concedes the shape
"cannot be retrofitted … without re-encrypting history." Consequences the corpus never states:

1. In a default deployment, ADR-0005's own headline triggers — the retention **ceiling**
   (scheduled, clock-driven destruction) and no-retention-basis subject requests — hit ordinary
   plaintext events that are **permanently un-shreddable**.
2. The clinically common "sensitivity recognized later" case (stigma realized weeks on) cannot
   retro-seal a body already replicated in plaintext to N nodes; the sensitivity stream can only
   raise visibility overlays.

Three composition collisions hide behind ADR-0005's "resolved" label (#92): (a) the legibility
twin's location under seal is implied, never stated — a future implementer materializing
`plaintext_twin` into an FTS column turns every seal into a full-content leak; (b) every ADR-0048
check fn reads the plaintext body, so the seal path either bypasses the mandatory-twin floor or
needs a structurally different door — unspecified; (c) the mandatory clear-text attachment
descriptor survives figure-granular shred by design ("photo of self-harm scars, left forearm"
outlives the pixels). Plus: rung 2's "no discoverable institutional record" is unachievable
without day-one preconditions no ADR wires in, and no ADR states the safety projection's own
erasure semantics.

The window closes with the first production clinical event — medication events are being
authored today.

## Decision (approved 2026-07-17)

**Born-sealed.** Every clinical JSONB body is sealed at write under a per-event DEK wrapped for
the node's own key. This is an **erasability substrate, not confidentiality** — the node reads
its own data freely, projections/FTS behave exactly as today, nothing is hidden from anyone. Its
only effect is that every ADR-0005 erasure rung stays reachable for every event forever.

Rejected alternatives, recorded for the ADR: *status quo* (plaintext default, seal opt-in —
permanently forecloses rungs 2–4 for ordinary events; retention-ceiling compliance impossible)
and *category-sealed at write* (blacklist-driven — everything the blacklist misses is foreclosed
exactly as in the status quo; the "recognized later" case still dies).

The principle-9 argument that decides it: a plaintext default silently forecloses rungs 2–4 for
the whole record — that **is** the system taking a policy stance. Born-sealed under node custody
forecloses nothing and hides nothing; it is the only policy-neutral default.

## ADR-0052 content (the ratified decisions)

### 1. The two-plane reframe: erasability ≠ confidentiality

The word "sealed" currently conflates two properties; ADR-0052 splits them:

- **Erasable** (new, default, universal for clinical JSONB bodies): body ciphertext under a
  per-event DEK; custody includes the node itself. Hides nothing. Exists so the erasure ladder
  stays reachable.
- **Sequestered** (existing ADR-0005 rung 1 / ADR-0006, graded, opt-in): the *same* DEK with
  custody **narrowed** (node → named clinicians/patient), safety projection coarsened, twin
  under seal.

Sequestering an already-erasable event is a **key-custody change, not a rewrite** — this
dissolves "sensitivity recognized later": no retro-encryption, just re-wrapping/withdrawing
keys, plus honest declaration of any already-replicated plaintext derivatives (of which, born-
sealed, there are none in the log).

### 2. Scope, stated normatively

Born-sealed covers **clinical JSONB bodies only**. Demographic assertions (typed columns the
matcher binds on), identity-algebra events, and node-plane events stay **plaintext by
necessity** — the machinery binds on them; the ADR says so plainly instead of leaving it
implied. Blob bytes inherit born-sealed via the per-blob DEK already reserved in ADR-0013;
implementation deferred to the blob-tier slice.

### 3. Seal-then-sign, per-event DEK

The author seals, then signs: the COSE payload carries `{alg, nonce, ciphertext}` and the
signature covers the **ciphertext** — so the signature verifies on every node and *after* shred,
exactly as ADR-0005 requires ("signature-over-ciphertext still verifies"). The AEAD tag makes
decryption deterministic, so signature-over-ciphertext + DEK-in-hand is non-repudiation of the
plaintext. AEAD is XChaCha20-Poly1305 (the `seal.rs` house algorithm); the `alg` field keeps the
shape crypto-agile under additive evolution.

**The legibility twin travels inside the sealed region under the same DEK** — the one normative
sentence #92(a) asked for. A sealed twin must never be materialized into any plaintext index.

DEK granularity is **per-event** in the skeleton (finest shred granularity, simplest keystore); a
per-episode key hierarchy remains the ADR-0005 measurement question, now with a concrete hook
(the skeleton's perf bench).

### 4. The key-custody plane

- Wrapped DEKs live in a **mutable keystore table beside the log — never inside the signed
  bytes** (custody changes must not touch the immutable artifact). The reserved `db/001`
  `event_log.dek_wrapped` column is **retired unused** by this design: an append-only row cannot
  hold rotating multi-holder custody, which is exactly why custody needs its own mutable table
  (`event_log.sealed` stays — it is an immutable birth property). The column is kept in place
  (dropping would fire no trigger but breaks replay-idempotence discipline for no gain) and
  documented as superseded by `event_dek`.
- The node's unwrap key is **X25519**: wrapping needs only the public half, so nothing secret
  sits in the DB. The private half lives in the existing ADR-0026 `seal.rs` keystore file with
  its op-passphrase + recovery-code escrow. An ordinary DB backup therefore can never
  reconstruct a DEK — the ADR-0005 "keys must not be silently reconstructable from ordinary DB
  backups" requirement, now structural. KEK loss = whole-record loss, so the ADR names KEK
  escrow **mandatory** (it rides the existing ADR-0026 recovery machinery, no new mechanism).
- **Sync:** custody travels as a **wrapped-DEK sidecar** beside the event on the clinical wire.
  For erasable-tier rows the sender re-wraps the DEK for any admitted peer (custody follows
  admission trust). Sequestered rows ship ciphertext + safety projection only; no sidecar unless
  the peer is a named holder. A peer without custody **admits the row and cannot read it** —
  set-union losslessness holds; confidentiality lives in key custody, never in withholding rows
  (ADR-0006 confirmed, not amended).

### 5. Shred and its blast radius

Rung-3 audited shred = **destroy the wrapped-DEK rows** (mutable keystore, not the log) +
**scrub all derived plaintext** — projections and the FTS/RAG substrate; #92(b)'s mandatory
index invalidation — + **append a signed `erasure.shred` event** (the audit tombstone: *existed
→ destroyed, basis Z*). The log row is never touched: signature still verifies, set-union still
converges, a resurrected opaque row is keyless noise.

**Restore-replays-shred:** a cold-peer restore replays the shred log and re-applies erasures
*before* projecting (§3.8 already promises this; the skeleton proves it).

### 6. The prose-only closures

- **Safety projection erasure semantics** (#92(c)): **coarsen-but-survive** on rung-3 shred —
  the Rh-after-termination case requires the signal to outlive the body. Shreddable only at
  rung-4 best-effort oblivion. Its survival is **named in the honest-erasure ceiling** (the
  declared "what remains" list).
- **Attachment descriptor** (#189(c)): the **public descriptor is graded** like the projection-
  coarseness ladder — the precise descriptor lives under the seal; the public stub's coarseness
  follows the sensitivity grade. "Photo of self-harm scars, left forearm" never sits in
  plaintext on a sequestered event; what survives a pixel shred is the coarse stub, declared in
  the ceiling.
- **Rung-2 preconditions** (#92(d)): deniable deletion is **only reachable for episodes born
  pseudonymous with abstracted routing** (§5.6 + the ADR-0006 dial-4 envelope abstraction) —
  stated as day-one preconditions; never offered retroactively. Prose only; no code this slice.

## Walking-skeleton seal slice (the code)

Stream: `clinical.medication` (the only live clinical-content stream). Thinnest end-to-end
thread through every mechanism above — the point is to surface the collisions while they are
cheap, not production depth.

### Storage shape

For sealed rows, `event_log` holds **zero plaintext**:

- `body` → a mechanical stub (`{"sealed": true, "event_type": …, "schema_version": …}`).
- `plaintext_twin` → a legibility **stub** naming type + seal state (principle 11: the row stays
  honestly self-describing as *what it is*; a custody-holding reader gets the real twin by
  decrypt).
- The **operational twin** (FTS/RAG substrate) moves to a **mutable shadow table**
  `event_twin_operational(event_id, twin, tsv)` populated at door time — deletable
  on shred without touching the append-only log. This is the structural answer to collision (a):
  there is no plaintext twin column on a sealed row to materialize from.
- New migration `db/037` (keystore table `event_dek(event_id, holder, dek_wrapped)`, shred log /
  door functions, twin shadow table). All additive; CREATE-vs-ALTER discipline per the
  migration-replay rule (no view-widening; widened CREATE TABLE needs the paired ALTER + guard
  entry).

### Doors (resolves collision (b))

- **Submit door (strict):** caller passes the signed sealed bytes **plus the DEK**. The door
  verifies the signature over the ciphertext, **decrypts in-DB** (XChaCha20-Poly1305 via
  `cairn_pgx` — the ADR-0002 pgrx escape hatch exists for exactly this), runs the **full**
  existing ADR-0048 twin/floor checks on the plaintext, builds projections + the operational-
  twin row, wraps the DEK for the node (X25519, public half in DB), stores it in the keystore
  table. The floor stays unbypassable in the database.
- **Apply door (lenient):** with a custody sidecar it does the same leniently; **without custody
  it admits on structural checks only** (can't read → never reject; strict-submit/lenient-apply,
  the ADR-0051 precedent).

### Node side

- Seal-at-write in the authoring path (medication assert): DEK generation + seal + sign + submit
  with the DEK.
- A `shred` CLI subcommand: authorization gravity, appends the signed `erasure.shred` event,
  destroys key rows, scrubs derived plaintext.
- Wrapped-DEK custody sidecar on the clinical sync wire (additive wire fields, ADR-0012).
- Restore replays the shred log before projecting.
- A **perf micro-bench** pinning seal/unseal cost per event against the Bet-B 4 ms p95 budget
  (also feeds the deferred per-episode-hierarchy measurement question).

### The E2E test thread (TDD, RED first)

Author a sealed medication event → projections/FTS behave exactly as pre-seal → sync A→B with
sidecar → B projects → **shred on A** → key rows gone, derived plaintext scrubbed, log row
intact, signature still verifies → shred syncs to B → cold-peer restore replays the shred →
the shredded body stays noise. Plus door-refusal pins (malformed seal shapes, DEK/ciphertext
mismatch) and the no-custody lenient-apply path.

### Operational caveats (deliberate)

- The sealed body shape is a **new additive `schema_version`**; pre-ADR-0052 plaintext event
  logs on dev/PoC rigs are **wiped, never synced through** (the ADR-0051 precedent — pre-clinical
  posture, no real data exists).
- A node lacking the seal-capable binary can still *admit* sealed events (set-union, structural)
  but not project them — honest degradation per ADR-0012's two planes; moot pre-production.

## Spec prose touched

- `data-model.md` §3.5/§3.8 — the erasable/sequestered split; born-sealed default; twin-under-
  same-DEK normative sentence; FTS/RAG invalidation on shred.
- `identity.md` §5.9 — safety-projection erasure semantics (coarsen-but-survive, ceiling
  disclosure).
- §3.14 — graded public attachment descriptor.
- ADR index + `index.md` spec version bump.

## Out of scope (explicitly)

- Blob-byte born-sealing (blob-tier slice later; posture ratified now).
- Per-episode DEK hierarchy (measurement question, hook shipped).
- Rung-2 pseudonymous-registration code (prose preconditions only).
- Sequester/custody-narrowing UI and the sensitivity-stream code (ADR-0006 mechanism beyond the
  skeleton's minimal seal path).
- FHIR façade interaction with sealed bodies.
