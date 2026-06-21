# 11. Open Questions (next brainstorming targets)

*Numbering is stable (used as a reference identity elsewhere). Resolved items are kept
struck-through; their resolution lives in the cited document and the linked ADR.*

1. ~~**Build vs. adapt the sync backbone.**~~ **RESOLVED** ([ADR-0001](decisions/0001-fat-postgres-thin-daemon.md), [sync §6.1](sync.md#61-mechanism)): build a thin custom Rust service on logical decoding; borrow pgactive/SymmetricDS patterns, do not depend on them.
2. ~~**Storage model.**~~ **RESOLVED** ([ADR-0001](decisions/0001-fat-postgres-thin-daemon.md), [data-model §3.5](data-model.md#35-event-storage-model-hybrid-envelope)): hybrid event envelope — typed envelope columns where invariants/identity/sync/matching bind; Cairn-native JSONB clinical bodies; FHIR is a façade only.
3. ~~**Dynamic sync scopes.**~~ **RESOLVED** ([ADR-0004](decisions/0004-dynamic-sync-scope-prefetch-not-authority.md), [sync §6.4](sync.md#64-scope-is-a-prefetch-hint-not-an-authority)): scope is an administrative *prefetch hint*, not an authority; a transfer triggers *acquisition*, not reassignment; access follows legitimate-need + audit; the surviving requirement is honest assembly-state disclosure. The case also surfaced the **bitemporal time model** and the **acknowledged-uncertainty** principle — [ADR-0003](decisions/0003-bitemporal-time-and-acknowledged-uncertainty.md), [data-model §3.6](data-model.md#36-bitemporal-event-time-recording-time-vs-effective-time)/[§3.7](data-model.md#37-acknowledged-uncertainty-uncertainty-capable-value-types).
4. ~~**Schema migrations across a fleet of offline nodes.**~~ **RESOLVED** ([ADR-0012](decisions/0012-schema-evolution-event-format-and-legibility-across-time.md), [data-model §3.13](data-model.md#313-schema-evolution-event-format-and-the-legibility-twin), [sync §6.5](sync.md#65-schema-evolution-two-planes-and-lossless-forwarding), [security §7.6](security.md#76-the-software-distribution-plane-signed-releases-and-extension-load)): schema evolution is *the append-only/overlay and acknowledged-uncertainty principles applied to the schema itself*, routed through **two planes that run at different speeds** — clinical events sync (set-union, AP, never executable code; the *format* evolves **forward-compatibly**) while code/DDL/extensions travel a **separate signed, per-architecture, sneakernet-capable distribution plane**; the schema/extension version is a *local node property*, so there is **no lockstep fleet upgrade**. Four day-one event-format invariants (`schema_version`; a mandatory signed **plaintext legibility twin**; **lossless passthrough** of original signed bytes; **additive-only** evolution) make a stuck-old node a safe, legible, forwarding, preserving participant. The **version-skew window is infinite for custody, best-effort for understanding**, and the legibility ladder *unifies with* the [ADR-0006](decisions/0006-visibility-scope-replication-and-the-safety-projection.md) safety projection (`min(parse-capability, visibility-clearance)`). Surfaced **founding principle 11 — legibility across time.**
5. ~~**Tombstones & retention.**~~ **RESOLVED** ([ADR-0005](decisions/0005-erasure-key-custody-and-crypto-shredding.md), [data-model §3.8](data-model.md#38-erasure-and-key-custody), [security §7.1](security.md#71-erasure-the-severity-ladder)): erasure is **redistribution of key-custody, not deletion of data** — crypto-shredding (destroy the DEK, never mutate the append-only log) on an encryption-capable body slot, exposed as a **policy-neutral severity ladder** (hide → sequester → deniable sealed-escrow deletion → audited crypto-shred → best-effort oblivion). Deletion is best-effort and *declared*, never guaranteed; the honest ceiling is *"to our knowledge, we have erased all copies in our existence."* Absorbs the [ADR-0004](decisions/0004-dynamic-sync-scope-prefetch-not-authority.md) surplus-copy-GC follow-on (per-node key custody erases one node's copy while the rightful holder keeps theirs).
6. ~~**Attachment strategy:** inline vs. content-addressed blob store with lazy sync.~~ **RESOLVED** ([ADR-0013](decisions/0013-attachments-content-addressed-lazy-blob-tier.md), [data-model §3.14](data-model.md#314-attachments-content-addressed-blobs-and-the-rendition-set), [sync §6.6](sync.md#66-attachments-the-lazy-byte-tier)): attachments are **content-addressed blobs referenced by the signed event, never inlined** (the append-only principle applied to large binaries — the content digest is to a blob what the signature is to a body; same bytes → same address → idempotent set-union). The **reference is eager, the bytes are lazy**, on a **resource-isolated byte tier** that can never starve clinical sync (the availability floor — *blob transfer must never reduce clinical-data availability*; chunked/preemptible/separately-budgeted, not merely priority-ordered). **Byte-replication is opt-in and separately scoped** (references everywhere, bytes by election — a starved node is references-only, fetch-on-demand), and content-addressing gives **multi-source, self-verifying swarm fetch** (LAN sibling / parent / patient-carried device, zero trust in source). The **rendition set** is the binary's legibility twin (raw + preview + report text), adding a **retrievability** axis to the [§3.13](data-model.md#313-schema-evolution-event-format-and-the-legibility-twin) ladder (`min(retrievable, parseable, cleared)`); **erasure** (per-blob DEK crypto-shred, no convergent encryption) and **lossless passthrough** (never transcode in place) inherit unchanged. The one can't-retrofit piece is the **day-one attachment-reference shape** (self-describing digest + seal indicator + descriptor + rendition set + inline path). DICOM/WADO/XDS is a façade, never the store. **No new founding principle.**
7. ~~**Locale-pluggable matcher comparators:** define the extension point (comparator API, weight configuration, evaluation harness per deployment).~~ **RESOLVED** ([ADR-0014](decisions/0014-locale-pluggable-matcher-comparators.md), [identity §5.13](identity.md#513-locale-pluggable-comparators-the-matcher-extension-point), [demographics §4.1](demographics.md#41-demographic-assertions)): the matcher is **advisory**, so this is structurally low-stakes — **no envelope reserve, one small additive data-model field, no new founding principle**. Hardcoding one culture's name/date/address model is **cultural capture**; comparators are pluggable (field-typed, agreement-leveled, *no-data-is-never-disagreement*, provenance-aware, over the multi-valued name *history set*). The resolution to *"comparators must travel with the data"* without syncing code or a central registry: a **content-addressed comparator-profile tag travels with each assertion** (defaults from the registering node's locale, registrar-overridable for relocation/visitor cases), the comparator **code** travels the distribution plane, and a node lacking a record's comparator **degrades honestly to human review, never forcing the wrong comparator** (safety-preserving — uncertainty can only *withhold* an auto-link). The confident-reject blind spot (silent false splits) is closed by a **low-priority, preemptible, aggressive background duplicate sweep at the hub**, emitting an advisory worklist whose yield *is* the miss-rate/drift metric, plus opportunistic re-match and a point-of-care discovery affordance. Hard vetoes **force a human decision, never an auto-reject**. The matcher is a **registered actor** (config version-pinned, recall via contamination cascade); GitHub doubles as a federated, signed, content-addressed registry (convenience, never a dependency).
8. ~~**Visibility-scope semantics on links.**~~ **RESOLVED** ([ADR-0006](decisions/0006-visibility-scope-replication-and-the-safety-projection.md), [identity §5.9](identity.md#59-sensitivity-grade-the-safety-projection-and-break-glass-visibility-scope), [sync §6.4](sync.md#64-scope-is-a-prefetch-hint-not-an-authority), [security §7](security.md), [data-model §3.5](data-model.md#35-event-storage-model-hybrid-envelope)): **replication is never the confidentiality boundary** — a safety-relevant sensitive episode replicates *unconditionally* (yes, it reaches the node); confidentiality lives only in key-custody + visibility + envelope-abstraction. A sealed body emits a de-identified, severity-graded **safety projection** (mechanical from coded fields) so decision-support warns without disclosing; coarseness is set by a graded, multi-source, append-only **sensitivity** stream (blacklist + grading system + human editability — Cairn ships the mechanism, policy combines them). **Break-glass** is audited key-*use*, partition-honest. Also answers the [ADR-0005](decisions/0005-erasure-key-custody-and-crypto-shredding.md) rung-1 follow-on (what safety metadata remains while a body is sealed).
9. ~~**Armed write-context interaction model.**~~ **RESOLVED** (with §11.12 — one problem) ([ADR-0008](decisions/0008-point-of-care-identity-possession-and-salvage.md), [identity §5.11](identity.md#511-point-of-care-identity-possession-fast-authentication-and-salvage)): possession binds `(clinician, patient)` in one ambient, high-distinctiveness gesture (the antidote to click-through), cold = warm cost; the arming gesture and ambient identity display restore paper's possession without confirmation dialogs.
10. ~~**Notification economy.**~~ **RESOLVED** ([ADR-0009](decisions/0009-notification-economy-salience-routing-and-the-acknowledgment-floor.md), [identity §5.12](identity.md#512-the-notification-economy-salience-responsibility-routing-and-the-acknowledgment-floor), [data-model §3.11](data-model.md#311-notifications-as-projections-responsibility-routing-and-acknowledgment), [security §7.4](security.md#74-notification-acknowledgment-and-the-safety-floor)): *"priority"* is one word hiding orthogonal dials, and the load-bearing split is **salience ≠ interruptiveness** (a standing fact is ambient; only an urgent transition is interruptive, once). A notification is a **projection** (a delta against the clinician's own audit history) + an append-only **acknowledgment** event — no mailbox, no new stream. **Noise reduction is suppression, and suppression is accountable** ([ADR-0007](decisions/0007-authorship-and-accountability.md)): demotion/coalescing/digest is additive and free; filtering/auto-ack is owned and audited. Follow-up responsibility is a **co-equal inbox + graded append-only responsibility-tag overlay** (orderer intrinsic; policy adds fallbacks; timeout-reassignment), and **routing is never a visibility gate** — a present clinician always sees the result (the consumer-side mirror of [ADR-0006](decisions/0006-visibility-scope-replication-and-the-safety-projection.md)). Safety floor: filtering changes modality, never extinguishes a hard-ack class; escalation never dead-ends; the inbox is partition-honest (no false *"all caught up."*).
11. ~~**In-database vs. application-layer merge boundary.**~~ **RESOLVED** ([ADR-0001](decisions/0001-fat-postgres-thin-daemon.md), [language-substrate §9.4](language-substrate.md#94-merge-projection-boundary-fat-postgres-thin-rust-daemon)): structural invariants + identity event algebra + all projections in Postgres (trigger-maintained incremental tables); thin Rust daemon ships/applies but carries no merge logic; matcher stays Python-advisory; per-projection Rust escape hatch on measured Pi-performance need.
12. ~~**Authentication vs. paper-parity tension.**~~ **RESOLVED** (with §11.9 — one problem) ([ADR-0008](decisions/0008-point-of-care-identity-possession-and-salvage.md), [identity §5.11](identity.md#511-point-of-care-identity-possession-fast-authentication-and-salvage), [security §7.3](security.md#73-point-of-care-authentication-possession-and-salvage), [data-model §3.10](data-model.md#310-session-identity-event-authorship-and-draft-durability)): the tension is **illusory** — the audit-trail collapse is *caused by* the parity violation, so unbundling **gatekeeping** (rare, coarse) from **attribution** (per-write, cheap) achieves both together. Load-bearing invariant: `session.user ≠ event.author`, independently bindable. Three pains removed via never-wait, always-a-fallback (ladder bottoming in break-glass), and **never-redo-work** (the `sign-as` salvage = identity-repair applied to authorship).
13. ~~**Operational observability.**~~ **RESOLVED — out of core (separable add-on, not architecture).** Fleet/node health (sync lag, disk pressure, key/credential expiry, hardware throttling, a node unseen for weeks) is **separable add-on software**, not a core-record concern. PostgreSQL already supplies the substrate (`LISTEN`/`NOTIFY`, logical decoding, `pg_stat_*`); the record-level facts that *are* safety-relevant are already first-class (the [§6.2](sync.md#62-consistency-model) freshness indicator, the [§7.10](security.md#710-node-durability-and-disaster-recovery) backup-health fact). A central dashboard for a health network or clinic chain is **optional add-on software a deployment may take or leave** — never a mandatory phone-home ([principle 7](index.md#founding-principles-the-lens-for-every-decision)). Cairn's core neither requires nor precludes it.
14. ~~**Trusted-time anchoring.**~~ **RESOLVED** ([ADR-0027](decisions/0027-trusted-time-anchoring.md), [data-model §3.17](data-model.md#317-trusted-time-anchoring-the-clock-confidence-grade-and-the-bracketed-t_recorded), [security §7.11](security.md#711-trusted-time-anchoring-the-notary-anchor-node-role), [sync §6.8](sync.md#68-time-attestation-rides-the-gossip-plane)): **principle 4 applied to wall-clock truth** — `t_recorded` becomes a **graded interval** (the §3.7 uncertainty-capable time type), carrying a **clock-confidence grade** as a day-one envelope field (`unknown < self-asserted < network-synced < hardware-sourced < externally-anchored < multi-anchor-corroborated`; later anchor tokens are overlays that refine up — [ADR-0015](decisions/0015-event-serialization-signatures-and-content-addressing.md) re-attestation-as-overlay). The 2001-era "trusted notary" is reframed as **two pluggable planes** (clock-setting/lower-bound: NTS/Roughtime/GNSS/TPM; existence-proof/upper-bound: a **transparency-log-shaped, multi-anchor** attestation, RFC-3161 as one supported anchor type, threshold via FROST) on the [ADR-0017](decisions/0017-federation-admission-sovereignty-peering-and-trust-anchors.md) anchor spectrum — no Cairn-owned root. **Offline is a bracket, not a degradation:** peer cross-attestation gives the lower bound and deferred **Merkle-root batch** notarization the upper bound (and the privacy fix), riding the existing gossip plane — *the same machinery pointed at time*. Public-chain anchoring is named-but-unshipped (pluggable future, never a dependency). **No new founding principle.**

## Resolved — authorship & accountability (AI-authored clinical information)

The general problem behind "tagging AI-generated content" (AI scribe, transcription, result-grading,
triage, notifications) is **resolved** by founding principle 10 and
[ADR-0007](decisions/0007-authorship-and-accountability.md): authorship is a contributor set and legal
responsibility is a separable, possibly-absent, possibly-proxied attribute
([data-model §3.9](data-model.md#39-authorship-and-accountability)). The notification-economy item (10)
is unaffected — it concerns priority/noise, not authorship.

**Deferred follow-ons (not blocking):**
- ~~**Closed role-enum membership**~~ — **RESOLVED** ([ADR-0028](decisions/0028-finalized-closed-contributor-role-enum.md),
  [data-model §3.9](data-model.md#39-authorship-and-accountability)): the closed enum is ratified at **6
  responsibility-bearing** (`authored`, `ordered`, `attested`, `co-signed`, `witnessed`, `dictated`) + **5
  contributory** (`drafted`, `transcribed`, `graded`, `triaged`, `suggested`). `co-signed`/`witnessed`/`dictated`
  earn slots because hard policy and the [identity §5.10](identity.md#510-authorship-and-responsibility-state-the-consumer-side)
  projection must branch on supervisory sign-off / occurrence-attestation / the dictation-transcription gap;
  **`reviewed` is rejected** (it is either `attested` or an acknowledgment — admitting it would re-fuse the
  signature≠attestation split). Roles describe *contribution to the record, not the clinical act* (so `performed`
  is out of scope); the set is closed and additive-only. **No new founding principle; no schema migration.**
- ~~**AI-agent identity registry**~~ — **RESOLVED** ([ADR-0011](decisions/0011-actor-registry-version-pinning-and-key-custody.md), [security §7.5](security.md#75-the-actor-registry-enrollment-version-pinning-and-key-custody), [data-model §3.12](data-model.md#312-actor-identity-in-the-registry)): a **general actor registry** (human/device/AI, AI the forcing case), **immutable & version-pinned** over a closed actor-event algebra (`enroll/supersede/revoke/suspend/rotate-key`) — never merge/erase, always link/overlay. **Identity granularity tracks objectively-recordable behavioral determinants** (AI pins vendor/model/version/weights/**inference config**/system-prompt/tools/node; per-call variance stamped on the event; humans carry no config dimension — mood/fatigue is real but not objectively recordable). Enrolment is audited with a **mandatory human backstop** (introduction-accountability; output-responsibility stays policy). Key custody un-conflated: **signing publics immortal, DEKs destroyable**. A model recall **reuses the contamination cascade**. Registry + algebra + verification are §9 trusted-base; the agent runtime is fit-for-purpose.
- ~~**Additive-vs-suppressing classification**~~ — **RESOLVED** ([ADR-0010](decisions/0010-additive-vs-suppressing-classification.md), [data-model §3.9](data-model.md#39-authorship-and-accountability), [identity §5.10](identity.md#510-authorship-and-responsibility-state-the-consumer-side)/[§5.12](identity.md#512-the-notification-economy-salience-responsibility-routing-and-the-acknowledgment-floor)): **structurally derived, not declared** — additive ≡ overlay, suppressing ≡ foreclosure (the append-only principle applied to the attention layer); the test is *"could a human still independently see and act on everything?"*. **Demotion is additive, only hiding/auto-deciding is suppressing** (a closed enumerated set); enforcement is a structural in-DB owner-gate; **responsibility is conserved** (relocated to the audited config act, never abolished); **declaration is a one-way caution ratchet** (the de-facto-suppression handle). Triage is a salience-scoring extension point (trend rules + AI oversight; mechanism, not policy), and **automation-complacency atrophy is detected** as an additive governance signal.
- **Proxy/liability semantics** — what `on_behalf_of` legally binds is out of scope; Cairn records the
  chain, jurisdictions interpret it.

## Resolved — national-scale record discovery (first contact, no central index)

How a node that does **not** hold the whole population's records discovers that a record exists for a
first-contact patient (new to the region) and requests it is **RESOLVED**
([ADR-0016](decisions/0016-record-discovery-and-the-replicated-essential-tier.md),
[sync §6.7](sync.md#67-record-discovery-and-the-replicated-essential-state-tier),
[identity §5.2](identity.md#52-matching-pipeline-safety-asymmetric-false-merge-worse-than-false-split)):
discovery is a **local matcher query** against a **replicated essential-state tier** (each person's
essential safety snapshot + a blocking-key summary on every federated node) — **no national Master
Patient Index** (the capture surface), no real-time dependency, no dependence on a patient-carried token.
The tier carries **current state, not transaction history** (the affordability boundary); the full record
follows lazily via [§6.4](sync.md#64-scope-is-a-prefetch-hint-not-an-authority) acquisition once a match is
confirmed. "Essential" is a graded, multi-source, append-only flag, and the confidential-essential case
composes with the [§5.9](identity.md#59-sensitivity-grade-the-safety-projection-and-break-glass-visibility-scope)
safety projection. Sizing validated against real-system data: ~2.5 TB and ~75–150 kbit/s for 100 M people.

## Resolved — Custodian & Federation Admission

Surfaced as a hard dependency of [ADR-0016](decisions/0016-record-discovery-and-the-replicated-essential-tier.md)
and now **RESOLVED** ([ADR-0017](decisions/0017-federation-admission-sovereignty-peering-and-trust-anchors.md),
[security §7.7](security.md#77-federation-admission-peering-trust-anchors-and-the-custodian-contract)): a node
needs **no permission to run alone** (the *sovereignty floor*); federation is **mutual, signed, append-only
peering** (the [§7.5](security.md#75-the-actor-registry-enrollment-version-pinning-and-key-custody) actor algebra
applied to nodes), gated by **pluggable, self-hostable trust anchors** (no Cairn-owned root) that span the
spectrum from a two-node practice LAN (direct pairwise) through a self-sovereign practice network (its own
issuing key) to a **national registry as a node role**. The **custodian contract** is signed, verifiable
metadata bound to the credential (Cairn ships format/verification/revocation; legal force is jurisdiction).
Admission gates the **outer boundary only** — *peered is not may-see-everything*; intra-federation
confidentiality stays [ADR-0006](decisions/0006-visibility-scope-replication-and-the-safety-projection.md)
key-custody + visibility. Verification is offline-capable; revocation reuses the contamination cascade as an
honestly-stale signed feed. **No new founding principle** (one operational corollary: the sovereignty floor).

**Revocation refined** ([ADR-0018](decisions/0018-federation-revocation-cascade-and-the-anchor-as-power.md),
[security §7.7](security.md#77-federation-admission-peering-trust-anchors-and-the-custodian-contract), pressure-tested
on the struck-off-operator-with-subsidiaries case): revocation is **enforced by the counterparties, never the
revoked node**; it is **forward-looking distrust, not retroactive erasure**; it **cascades over the
issuance/affiliation graph** (by chain + a controlling-entity attribute — *revoke the principal, not the key*);
**anchor revocation ≠ voluntary unpeering**; a **trust anchor is a position of power** whose blast radius Cairn
minimises (sovereignty floor, multi-anchor default — *never mandate a single anchor*, audited signed revocation,
availability floor) but whose *legitimate* exclusion it cannot and must not prevent; partition-honest with a
local-read-never-fails-closed freshness knob; **clawback of already-synced data is an authorities' matter, not
Cairn's**.

## Resolved — author-scoped record export (the clinician's medico-legal copy)

How a clinician retains their own records as a litigation defence across a portfolio career (compounding
per-workplace loss risk) is **RESOLVED** ([ADR-0019](decisions/0019-author-scoped-record-export-the-medico-legal-copy.md),
[security §7.8](security.md#78-author-scoped-record-export-the-medico-legal-copy)): a first-class, **audited**
export selected by **contributor identity** ([§3.9](data-model.md#39-authorship-and-accountability)), **strictly
author-scoped** (progress notes, path/imaging *requests*, referrals — the reasoning and actioning; **not** results,
which are a separable practice-custodianship duty), **self-verifying and legible across time** (signed bytes +
plaintext twins → court-admissible decades on), with a **policy-neutral seal ladder** (author-readable /
authority-public-key-sealed / both). It is the general mechanism behind [ADR-0005](decisions/0005-erasure-key-custody-and-crypto-shredding.md)
rung-2's escrowed clinician copy; the erasure interaction is the intended honest ceiling. **No new founding
principle** (refines [ADR-0007](decisions/0007-authorship-and-accountability.md)).

## Resolved — application layering, the node API, and UI pluralism

How a **plurality of UIs** (small teams building bespoke front-ends quickly and safely) can be facilitated
**without ever compromising the guarantee the mission rests on — any Cairn node interoperating with any
other, regardless of UI or policy** — is **RESOLVED** ([ADR-0021](decisions/0021-layering-the-node-api-and-ui-pluralism.md),
[language-substrate §9.5](language-substrate.md#95-layering-the-node-api-and-ui-pluralism-uniform-core-plural-edges)):
the inter-node contract is the **signed event core, below UI and policy** — already fixed and
UI-independent by construction ([ADR-0015](decisions/0015-event-serialization-signatures-and-content-addressing.md)/[ADR-0001](decisions/0001-fat-postgres-thin-daemon.md)/[ADR-0012](decisions/0012-schema-evolution-event-format-and-legibility-across-time.md)/[ADR-0017](decisions/0017-federation-admission-sovereignty-peering-and-trust-anchors.md))
— so nothing above it may sit on the inter-node path. A **four-layer model** (L0 wire core / L1 node
enforcement floor / L2 policy + API / L3 UI) puts the compatibility boundary **below the application
layer**. The bypass tension (*"UI → API or DB directly"*) dissolves by putting the **floor in the
database**: safety/compatibility invariants are enforced in-DB (validated submit functions + RLS +
constraints), so direct DB access is safe by construction and *"via API vs DB directly"* is a **privilege
gradient, not a contradiction** — L2 is ergonomics + deployment hard policy, never the sole wall.
**Hard vs soft policy** is the [§9.1](language-substrate.md#91-selection-rule-by-defect-blast-radius)
blast-radius rule applied to policy. The **anti-drift guarantee:** a UI is a pure producer/consumer over a
contract it cannot alter (the *node* owns serialization/signing), the native API evolves **additively**
([principle 11](index.md#founding-principles-the-lens-for-every-decision) applied to the contract) and is
**capability-described + conformance-tested**, so a bespoke UI can produce wrong content but **never a
wire-incompatible event**. **Native API ≠ the FHIR façade** (two surfaces); the steward's reference UI is
built only on the public API (anti-capture turned inward). Surfaced **founding principle 12 — uniform core,
plural edges.**

The **completeness of that submit surface** (the bet ADR-0021 rests on) is then specified
([ADR-0022](decisions/0022-validated-submit-surface-the-write-path.md),
[language-substrate §9.6](language-substrate.md#96-the-validated-submit-surface-the-write-path)): because the
system is append-only, *almost every write is the same operation*, so the surface is **one generic
validated-append** (`submit_event`, type-validated by additively-registered dispatch — a new event type adds
a validator, never a new door) **plus a small closed set of non-append operations** (erasure/key-custody,
author-scoped export, blob byte-tier put). It is **small and complete** by construction, and is the **in-DB
convergence of every write-time seam** the spec has named (authorship stamp, clash detection, seal-time
safety projection, suppressing owner-gate, legibility-twin derivation, canonicalize+sign). Signing must be
reachable from the in-DB path (else the floor would be incomplete for direct-DB callers — the database
process is part of the node's trusted base). Authoring (`submit_event`, signs) is distinct from applying
(verifies peer signatures, never re-signs). **No new founding principle** (refines ADR-0021/ADR-0001).

The **native API contract** (the anti-drift tool for bespoke UIs) is then specified
([ADR-0023](decisions/0023-native-api-contract-capability-and-conformance.md),
[language-substrate §9.7](language-substrate.md#97-the-native-api-contract-capability-description-and-conformance)):
API compatibility is the **same problem as schema evolution** (permanent offline version skew), so the
contract is **additive capability flags over a mandatory baseline, not a monotonic version number**, with the
[§3.13](data-model.md#313-schema-evolution-event-format-and-the-legibility-twin) `min()` ladder for graceful
degradation. A node serves a **self-describing capability descriptor** (a projection of its local-node
schema/extension/config properties — not new state; transport-independent); negotiation is **stateless
description + client-side degradation, not a handshake**, and degradation may cut experience but never
correctness/safety (the mandatory core is the floor). The **conformance suite** is the executable contract in
two faces — *wire/node* (does it correctly participate in L0; the "any node talks to any node" guarantee made
checkable, a federation admission gate) and *API* (does the L2 API honor the contract for the capabilities it
claims; capability-partitioned, additively versioned, tests never removed). It is **self-runnable and
self-verifiable** — open, signed, content-addressed (the [ADR-0014](decisions/0014-locale-pluggable-matcher-comparators.md)
registry pattern), never a steward-issued certificate (anti-capture turned inward, a second time), and doubles
as the spec's executable form (principle 11). **No new founding principle** (refines ADR-0021).

Finally, **how hard policy is expressed** is specified ([ADR-0024](decisions/0024-hard-policy-expression-the-policy-assertion-stream.md),
[security §7.9](security.md#79-hard-policy-expression-projection-and-enforcement)), **closing the ADR-0021
layering/API arc** (0021 → 0022 → 0023 → 0024): hard policy is just Cairn's universal shape applied to policy
itself — an **append-only, signed, scoped policy-assertion stream with an effective-policy projection**, a
**declarative selection over a closed Cairn-shipped mechanism set, never arbitrary code** (the selection is
data on the event plane; the evaluation code travels the §7.6 distribution plane). The *DB-anchored vs
role-gated-L2* question dissolves — same expression, the enforcement *locus* is a [§9.1](language-substrate.md#91-selection-rule-by-defect-blast-radius)
blast-radius call (the §9.6 submit surface + RLS read the projection in-DB by default). Authoring is
authority-gated (bootstrapped at provisioning); policy is **scoped and floor-composing** (a federation floor
ratchets stricter, never weaker; local non-floor policy is node-autonomous — the sovereignty floor), and
partition-honest (last-known policy; local reads never fail closed). It **unifies the scattered "expressible
policy rungs"** (§5.10, ADR-0005/0006/0009/0010, §7.5/7.6/7.7) under one mechanism and closes
[ADR-0010](decisions/0010-additive-vs-suppressing-classification.md)'s conservation-of-responsibility loop
(the audited config act is now a concrete policy event). **No new founding principle** — it is the mechanism
*of* principle 9.

## Resolved — node durability & disaster recovery

How a node — especially the **solo node** with no peer (the [ADR-0017](decisions/0017-federation-admission-sovereignty-peering-and-trust-anchors.md)
sovereignty floor) — survives total hardware loss without a backup that can resurrect an erased body is
**RESOLVED** ([ADR-0026](decisions/0026-node-durability-and-disaster-recovery.md),
[security §7.10](security.md#710-node-durability-and-disaster-recovery)). The spec had designed *deliberate*
key-death ([ADR-0005](decisions/0005-erasure-key-custody-and-crypto-shredding.md)) but left *accidental*
key-death undesigned, and the only DR answer ([sync §6.3](sync.md#63-failure-modes-designed-for)) assumed a
parent and excluded the off-sync-plane keystore. The resolution dissolves into existing primitives:
**clinical events back up as a cold peer** (the sync daemon's peer is a local encrypted volume; restore is
set-union apply through the existing verify-on-apply path — self-verifying, no new integrity check);
**non-event trust material rides a sealed local-state export** (the only private-key-touching surface);
**recovery mints a new `supersede`-linked identity** ([§7.5](security.md#75-the-actor-registry-enrollment-version-pinning-and-key-custody)
actor algebra) so the **private signing key never enters a backup** (a stolen backup can't resurrect the node;
composes with non-extractable hardware keys); the **off-node recovery secret is paper-escrow at the floor**
(printed code / QR, optional Shamir M-of-N), pluggable up to token / peer-quorum, no mandatory cloud; and
**erasure survives DR** because crypto-shred is an append-only event the restore replays, with **shred
completion ⊇ backup propagation** closing the post-backup-shred window (detached media = the honest ceiling).
**Backup health is a first-class honest-assembly fact.** **No new founding principle** (principles 1/2/3/4
applied to DR); one **day-one, can't-retrofit** requirement — the recovery-secret escrow and sealed export must
exist at provisioning.

## Resolved — trusted-time anchoring

How a node establishes **wall-clock truth** for `t_recorded` (the HLC gives causal *ordering*, not truth; a
drifting RTC with no sync can place the medico-legal ceiling arbitrarily) — without a write-time network
dependency or a central authority — is **RESOLVED** ([ADR-0027](decisions/0027-trusted-time-anchoring.md),
[data-model §3.17](data-model.md#317-trusted-time-anchoring-the-clock-confidence-grade-and-the-bracketed-t_recorded),
[security §7.11](security.md#711-trusted-time-anchoring-the-notary-anchor-node-role),
[sync §6.8](sync.md#68-time-attestation-rides-the-gossip-plane)). It is
[principle 4](index.md#founding-principles-the-lens-for-every-decision) applied to time: `t_recorded` is a
**graded interval**, not a point (the §3.7 uncertainty-capable time type), carrying a single ordered
**clock-confidence grade** as a **day-one envelope field** with later anchor tokens as refining overlays. The
RFC-3161 notary is generalized into **two pluggable planes** — clock-setting (NTS/Roughtime/GNSS/TPM, the lower
bound) and a **transparency-log-shaped, multi-anchor** existence-proof (the upper bound, RFC-3161 as one anchor
type, threshold via FROST) — on the [ADR-0017](decisions/0017-federation-admission-sovereignty-peering-and-trust-anchors.md)
trust-anchor spectrum, **no Cairn-owned root**. **Offline is a bracket, not a degradation**: peer
cross-attestation (lower bound) and deferred **Merkle-root batch** notarization (upper bound + privacy) ride the
existing gossip plane — *the same append-only/signed/Merkle machinery pointed at time.* Public-chain anchoring is
named-but-unshipped. **No new founding principle**; one day-one can't-retrofit field (the grade + interval).

**With this, every original §11 open architecture question is closed.**

## Resolved — advisory-actor integration contract and skill-epoch refinement

[Spike 0002](../spikes/0002-advisory-actor-write-contract.md) (advisory-actor write contract) passed C1–C5 on
2026-06-21 — an external advisory agent authors an additive, un-attested, provenance-anchored, recallable advisory
through the validated in-DB floor, and the floor rejects every hostile-agent attempt even with direct DB access
([ADR-0021](decisions/0021-layering-the-node-api-and-ui-pluralism.md)'s *"direct DB access safe by construction"*
made checkable and confirmed). Per its §6 exit criteria, that triggered **two ADRs**.
**[ADR-0029](decisions/0029-skill-epoch-as-pinned-actor-determinant.md)** (refines
[ADR-0011](decisions/0011-actor-registry-version-pinning-and-key-custody.md); canonical home
[security §7.5](security.md#75-the-actor-registry-enrollment-version-pinning-and-key-custody)) names the
**crystallised-skill digest (skill epoch)** and the **served-model digest** as pinned determinants of an agent
actor's identity: a skill bump is an audited supersession, recall bounds to a skill epoch via the contamination
cascade, and a shared serving fabric cannot silently mutate a pinned identity.
**[ADR-0030](decisions/0030-advisory-actor-integration-contract.md)** (refines
[ADR-0021](decisions/0021-layering-the-node-api-and-ui-pluralism.md)/[ADR-0022](decisions/0022-validated-submit-surface-the-write-path.md);
canonical home [language-substrate §9.8](language-substrate.md#98-the-advisory-actor-integration-contract)) fixes
the **advisory-actor integration contract** — an L2/L3 actor authors only through `submit_event`, un-vouched by
construction, additive-never-suppressing (suppression needs human attestation), provenance-anchored, recallable,
enforced unbypassably at the L1 floor — promoting
[ecosystem/0001](../ecosystem/0001-agent-and-messaging-plugins-kastellan-localmail.md) from evaluation to decision.
**No new founding principle** (both apply the existing principles 2/8/10/12). Honest ceiling (ADR-0030): the
attestation *success* path is not yet exercised (only the rejection half), and is the first thing to build atop the
contract.

The remaining generative threads are build-prep (the Bet B Pi compute-cost run) and continued clinical case-mining.
