# 7. Security & Compliance (macroscopic)

- **Encryption at rest** mandatory below facility tier (LUKS + per-database encryption).
- **Per-record encryption — reserved from day one, off by default.** Beyond whole-storage encryption, the event envelope reserves an **encryption-capable body slot** ([data-model §3.5](data-model.md#35-event-storage-model-hybrid-envelope)): a clinical body may be sealed under a per-unit data-encryption key (DEK) **wrapped for a key-holder hierarchy** — `{node}` by default, optionally `{patient}` and/or named `{clinicians}`. It is opt-in, for special cases (stigma-sensitive episodes, deniable deletion), because a patient-held key trades availability for confidentiality. The shape is fixed now because it cannot be retrofitted onto an append-only log.
- **Offline authentication:** cached short-lived credentials/certificates per device and user; offline access automatically narrows; break-glass with mandatory retrospective audit. The point-of-care model — fast proximity sessions, the gatekeeping/attribution split, and work-salvage — is [§7.3](#73-point-of-care-authentication-possession-and-salvage) / [identity §5.11](identity.md#511-point-of-care-identity-possession-fast-authentication-and-salvage).
- **Audit log is an event stream**, syncing upstream at highest priority.
- mTLS between nodes; enrollment via explicit trust/provisioning ceremony (also regenerates machine identity and PRNG seed — see [data-model §3.2](data-model.md#32-identity-time)).
- **Visibility scopes on link events** ([§5.6](identity.md#56-pseudonymous-sanctioned-care)): access-control and identity-linkage decisions are coupled by design. A sensitive episode replicates unconditionally (replication is never the confidentiality boundary); confidentiality is enforced at key-custody and visibility, and a sealed body still emits a de-identified, severity-graded **safety projection** so decision-support warns without disclosing — see [§5.9](identity.md#59-sensitivity-grade-the-safety-projection-and-break-glass-visibility-scope), [ADR-0006](decisions/0006-visibility-scope-replication-and-the-safety-projection.md).
- **Break-glass is audited key-*use***, distinct from the key-*destruction* of erasure ([§7.1](#71-erasure-the-severity-ladder)): it acquires/uses a DEK to unseal a sequestered body where a key-holder is reachable, and is partition-honest where none is (*"sealed content exists here; the key is not present"*). The architecture always provides it; whether the UI offers it and what authorization it demands is policy.
- Compliance posture (GDPR/HIPAA/national law) is configuration; core guarantees (encryption, audit, access control) are universal.

## 7.1 Erasure (the severity ladder)
> Resolves former open question §11.5 — see [ADR-0005](decisions/0005-erasure-key-custody-and-crypto-shredding.md). Mechanism summary; the body-slot shape is [data-model §3.5](data-model.md#35-event-storage-model-hybrid-envelope)/[§3.8](data-model.md#38-erasure-and-key-custody).

**Erasure is the redistribution of key-custody, not the deletion of data.** Nothing in the append-only log is mutated; the deletion primitive is **crypto-shredding** — destroying a body's DEK leaves an immutable, signature-valid, sync-safe row that is now keyless noise (the only deletion model compatible with append-only + WORM). Cairn builds a **policy-neutral ladder**; **which rungs a deployment offers is configuration, never a stance Cairn takes** — it facilitates conflicting legal/health-system requirements without taking sides.

| Rung | Mechanism | Trace |
|---|---|---|
| 0 **Hide** | repudiation / reattribution overlay ([§5.5](identity.md#55-reattribution-one-primitive-tiered-workflows)) | full audit |
| 1 **Sequester** | per-record encryption; *policy-defined* safety-relevant metadata may remain; break-glass audited | audited |
| 2 **Deniable deletion** | destroy institution's index + node DEK; sealed copies escrowed to patient + chosen clinician(s); **institution holds nothing** | **none** |
| 3 **Audited crypto-shred** | destroy all keys; immutable shred event records *existed → destroyed, basis Z* | proof-of-destruction |
| 4 **Best-effort oblivion** | shred keys *and* all known custodian copies | declared best-effort |

- **Rung 2 (deniable) vs. rung 3 (audited) pull opposite ways and that is deliberate.** Rung 3's tombstone *proves* existence + lawful destruction (clinician medico-legal cover); rung 2 must leave **no trace** — a tombstone would prove the record existed, which is exactly what the patient needs gone. In rung 2 the clinician's cover **migrates** to their own retained sealed copy, producible later by the patient's consent; the institution can honestly answer a subpoena "no record". Policy selects the rung; the system takes no side.
- **The honest-erasure ceiling (normative).** The strongest claim Cairn ever makes is **"to our knowledge, we have erased all copies in our existence"** — both hedges load-bearing, both corollaries of acknowledged uncertainty ([§3.7](data-model.md#37-acknowledged-uncertainty-uncertainty-capable-value-types)): offline nodes/backups/WORM cannot be confirmed (*"to our knowledge"*), and sealed copies a patient or trusted clinician holds are outside the institution's boundary (*"in our existence"*).
- **The keystore is safety-critical.** Key destruction is irreversible; an *accidental* shred is catastrophic data loss (founding principle 1). It carries the same gravity/authorization/audit as the erasure it effects, and keys must not be silently reconstructable from ordinary DB backups after destruction.

## 7.2 Signing, attestation, and AI-agent identity

> [!IMPORTANT]
> **A signature proves origin and integrity; attestation confers responsibility. They are separable
> acts** (founding principle 10, [ADR-0007](decisions/0007-authorship-and-accountability.md)).

- **Two jobs, unfused.** For a human author the cryptographic signature and the act of vouching collapse
  into one, which is why the envelope historically carried a single `author` + `signature`. AI authorship
  forces them apart: every event is **signed** (origin + integrity, by whatever authored it — including an
  AI agent), but a signature confers **no legal attestation**. *Signed ≠ vouched-for.* Responsibility is a
  separate per-contributor attribute carried by a responsibility-bearing role
  ([data-model §3.9](data-model.md#39-authorship-and-accountability)).

- **AI agents are registered cryptographic identities.** An AI author signs with its own key, bound to
  `model + version + vendor + deploying node`. This makes AI authorship as auditable and **recall-traceable**
  as a human's even though it is (by current policy) never accountable: when a model version is later found
  defective, *"which events did agent X v2.3 author?"* is a first-class query. The AI-agent identity
  **registry and its key custody are part of the trusted base** — a non-human actor inside the
  safety-critical surface ([§9 blast-radius rule](language-substrate.md)); keep it small and reviewer-legible.

- **Policy-neutral (principle 9).** Whether a deployment ever lets responsibility be *held_by* an AI agent
  (as proxy for its owner, or eventually in its own right) is configuration, not a stance Cairn takes. The
  signing/attestation mechanism is indifferent to that choice.

## 7.3 Point-of-care authentication, possession, and salvage
> Resolves former open questions §11.9/§11.12 — see [ADR-0008](decisions/0008-point-of-care-identity-possession-and-salvage.md). Canonical design: [identity §5.11](identity.md#511-point-of-care-identity-possession-fast-authentication-and-salvage); invariants: [data-model §3.10](data-model.md#310-session-identity-event-authorship-and-draft-durability).

The "fast sessions vs. security posture" tension is **illusory**: the deployed-EHR audit-trail collapse is *caused by* the paper-parity violation (expensive per-write authentication makes shared logins rational, and sharing is what destroys attribution), so honest per-write attribution and paper-parity are achieved **together**, not traded.

- **Unbundle gatekeeping from attribution.** Coarse, rare gatekeeping (device trust, mTLS enrolment, shift-start) is separable from fine, per-write attribution. The load-bearing invariant — `session.user ≠ event.author`, independently bindable ([data-model §3.10](data-model.md#310-session-identity-event-authorship-and-draft-durability)) — keeps attribution paper-cheap without weakening gatekeeping.
- **A resilience ladder, no dead-ends:** *badge → password → self-recovery (security-Q / SMS / recovery codes) → audited break-glass*. Every rung is self-service (no IT dependency), and the floor is the existing partition-honest **break-glass** primitive (§7 above, [identity §5.9](identity.md#59-sensitivity-grade-the-safety-projection-and-break-glass-visibility-scope)) — recovery is break-glass for the auth layer, because "forgot everything during a 3 a.m. partition with no network" must still resolve. Which rungs a deployment offers is policy (principle 9).
- **Salvage over loss:** `sign-as` lets a clinician sign their own stranded work as the true author without disturbing the logged-in session (the default; *switch* and *stay* are the alternatives) — replacing the wrong-author save and the lost-work rage with a clean, audited, append-only repair.
- **Authorship-confidence is a grade, not a gate** (acknowledged uncertainty): *attested* / *asserted* / *unattributed* compose into the existing trust projection; the system never blocks care to authenticate, it records honestly and refines by overlay.
