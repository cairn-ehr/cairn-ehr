# 7. Security & Compliance (macroscopic)

- **Encryption at rest** mandatory below facility tier (LUKS + per-database encryption).
- **Per-record encryption — reserved from day one, off by default.** Beyond whole-storage encryption, the event envelope reserves an **encryption-capable body slot** ([data-model §3.5](data-model.md#35-event-storage-model-hybrid-envelope)): a clinical body may be sealed under a per-unit data-encryption key (DEK) **wrapped for a key-holder hierarchy** — `{node}` by default, optionally `{patient}` and/or named `{clinicians}`. It is opt-in, for special cases (stigma-sensitive episodes, deniable deletion), because a patient-held key trades availability for confidentiality. The shape is fixed now because it cannot be retrofitted onto an append-only log.
- **Offline authentication:** cached short-lived credentials/certificates per device and user; offline access automatically narrows; break-glass with mandatory retrospective audit.
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
