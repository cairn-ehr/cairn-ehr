# ADR-0040 — Signing-context domain separation, and one signature per event

- **Status:** Accepted
- **Date:** 2026-07-02
- **Refines:** [ADR-0015](0015-event-serialization-signatures-and-content-addressing.md) (the COSE_Sign1/Ed25519 primitives),
  [ADR-0007](0007-authorship-and-accountability.md) (co-signing as overlay),
  [ADR-0030](0030-advisory-actor-integration-contract.md) (the attestation token this separates from the event)

## Context

Cairn signs three structurally different artifacts with the same Ed25519 key family and, until
this ADR, the same undifferentiated COSE_Sign1 shape: **events** (the clinical and node planes,
both an `EventBody`), **attestation tokens** (the ADR-0030 responsibility proof), and **pairing
bundles** (the ADR-0017 §7 out-of-band offer). Nothing in the signed bytes said *which kind* a
signature was minted for: `external_aad` was empty and the protected header carried no
content-type. Cross-context replay — presenting a signature minted in one context for
verification in another — failed only because the three payload types happened to have disjoint
required fields. That is structural luck, not a guarantee, and principle 11's additive-only
evolution actively erodes it: parsers ignore unknown fields, so a future payload that grows a
superset of another type's fields becomes cross-parseable, and a signature crosses with it. The
concrete failure is not hypothetical in shape: a signed artifact whose payload parses as an
attestation would *confer responsibility* (ADR-0007) the signer never intended to confer.

A second, adjacent wire-freeze item ([review 2026-07-02](../../code_reviews/2026-07-02-comprehensive-review.md),
finding B4; [issue #95](https://github.com/cairn-ehr/cairn-ehr/issues/95)): ADR-0015 §2 cited
COSE's "native multi-signer support" as a rationale, but ratified **COSE_Sign1** — which
[RFC 9052](https://www.rfc-editor.org/rfc/rfc9052) defines as the *single-signer* variant
(multi-signer is COSE_Sign, a different byte layout). Since the stored bytes are sacred (ADR-0015
move 1), drifting from Sign1 to Sign later would be a new event format. The ambiguity had to be
resolved *before* the format freezes, in one direction or the other.

Both fixes invalidate every signature minted before them. There is no production clinical data
(the first production surface is weeks old and pre-deployment), so the cost is a dev/test
re-initialization — the cheapest this will ever be.

## Decision

### 1. One signature per event — COSE_Sign1 is ratified as deliberate

An event carries **exactly one signature: the authoring actor's**, in a COSE_Sign1 envelope.
Multi-party accountability is *not* expressed by multiple envelope signatures:

- the **contributor set** rides *inside* the signed body (§3.9) — who took part, in what role;
- **responsibility** is conferred by a separable **attestation token** bound to the event's
  content-address (ADR-0030), never by a second envelope signature;
- **co-signing and re-attestation are overlay events** referencing the original (ADR-0007 §6,
  ADR-0015 move 3), exactly like corrections.

ADR-0015's "native multi-signer support" phrase is hereby superseded as a *justification* (the
serialization choice it argued for is unchanged and stays ratified). COSE_Sign — and the
multi-signer envelope generally — is **dismissed**: it would put a mutable-feeling social process
(gathering signatures) inside an immutable artifact, where the overlay model already expresses it
append-only.

### 2. Every signature is domain-separated by a signing context

A **signing context** is a short registered string naming the *kind* of signed artifact. It is
bound into every signature **twice**, deliberately redundantly:

1. **In the COSE protected header as the content type** (RFC 9052 §3.1, label 3) — inside the
   signed bytes, self-describing on the wire. A reader decades from now can tell what a blob *is*
   without guessing at payload shapes (principle 11), and a verifier rejects a wrong or missing
   context with a legible `ContextMismatch` *before* touching the payload.
2. **As the COSE `external_aad`** in the Sig_structure — so cross-context verification fails
   *cryptographically*, in the signature math itself, even against a hypothetical future verifier
   that forgets the header check. The header check is policy; the aad is physics.

The initial registry:

| Context string | Artifact |
|---|---|
| `application/cairn-event+cbor` | Events — clinical AND node planes (both sign an `EventBody`) |
| `application/cairn-attestation+cbor` | Attestation tokens (ADR-0030) |
| `application/cairn-pairing+cbor` | Pairing bundles (ADR-0017 §7) |

The registry is **closed and additive-only** (principle 11 applied to the registry itself): a new
kind of signed artifact gets a **new** string; an existing string is never repurposed or changed,
because it is load-bearing in every signature ever minted under it. The clinical and node planes
share one context deliberately — they share the `EventBody` structure and the same verify gate,
and their separation is enforced *inside* the signed payload by `event_type` plus each door's
fail-closed classification, which is already the load-bearing plane boundary.

### 3. Fail closed; no grandfathering

Verifiers reject uncontextualized (pre-ADR-0040) blobs. There is no acceptance window for legacy
signatures: with zero production data, grandfathering would be pure attack surface (an
uncontextualized blob is exactly the cross-context-capable artifact this ADR exists to kill).
Dev/test events are re-signed; a dev federation re-initializes.

### 4. Blast radius and enforcement seam

The implementation lives where the §9 rule wants it: **one generic sign helper and one generic
verify helper** in `cairn-event`, which every public signing/verification function delegates to —
so threading the context cannot be forgotten by construction. The in-DB floor (`cairn_verify` /
`cairn_attestation_ok` via the ADR-0002 pgrx hatch) inherits the enforcement unchanged, because it
already delegates to the same single implementation. No SQL, wire-protocol, or schema change: the
context rides inside `signed_bytes`, which every door already stores verbatim.

## Consequences

**Easier / now guaranteed:**
- Cross-context replay is impossible by cryptography, not by the accident of disjoint payload
  fields; additive payload evolution can no longer create signature-crossing overlap.
- Every signed blob is self-describing about its kind — a forensic or archival reader (principle
  11) identifies artifacts without payload heuristics.
- The Sign1-vs-Sign ambiguity is closed before the freeze: one signature per event, plurality by
  overlay, consistent with the append-only grain of everything else.

**Harder / accepted costs:**
- Every pre-ADR-0040 signature is invalid. Dev federations re-initialize; the walking-skeleton
  era's signed fixtures are historical only. (Deliberate — see §3.)
- The context string adds ~30 bytes to each signed artifact (well inside the A5 budget; the twin
  dwarfs it).
- A future *legitimate* need for a second envelope signature (e.g. a hardware token co-signing at
  write time) must be designed as an overlay or a new context, never by widening Sign1 — that is
  the point, but it forecloses shortcuts.

**How we would know the bet is failing:** an artifact kind emerges that genuinely cannot express
its multi-party semantics as contributor-set + attestation + overlay (would force revisiting §1);
or interop pressure demands standard COSE multi-signature envelopes at the FHIR/interop boundary
(handled there by the façade, never by changing the internal format — same answer as RSA/ECDSA in
ADR-0015 §6).

**Not a new founding principle.** This is principle 11 (additive-only, self-describing artifacts)
and the §9 small-auditable-surface rule applied to the signature layer, plus the ADR-0015 moves
carried to their conclusion.
