# 3. Data Model Principles

## 3.1 Append-only clinical event log (source of truth)
- All clinical content (notes, observations, orders, results, administrations, signatures, addenda) is written as **immutable, signed events**. Corrections are new events referencing the original — matching medico-legal documentation norms.
- Immutable events cannot conflict; merging divergent logs is **set union**. This eliminates the bulk of the multi-master problem by construction.
- Current state ("the chart") is a **projection** materialized per node — rebuildable, cacheable, never synced itself.

> [!NOTE]
> Because the log is append-only and immutable, syncing the source of truth is INSERT-only,
> idempotent (UUIDv7 PK), scoped **set union** — there are no row-level clinical conflicts to
> resolve. All genuinely hard "merge" logic is confined to *derived* state (projections), which is
> rebuildable and never synced. This is the pivot the whole sync/merge design turns on
> ([ADR-0001](decisions/0001-fat-postgres-thin-daemon.md)). The *authoring* counterpart — how a write
> validly enters the log through the one validated submit surface — is
> [§9.6](language-substrate.md#96-the-validated-submit-surface-the-write-path) ([ADR-0022](decisions/0022-validated-submit-surface-the-write-path.md)).

## 3.2 Identity & time
- **UUIDv7 primary keys everywhere** (native `uuidv7()` in PostgreSQL 18) — globally unique, offline-generable, time-ordered. This is the **canonical** identity; the per-node projection plane may additionally intern these to dense `bigint` surrogates as the physical foreign-key/join key, a node-local optimization that never escapes the projection ([§3.18](#318-canonical-identifiers-and-node-local-surrogate-keys-the-dual-identifier-discipline), [ADR-0031](decisions/0031-canonical-identifiers-and-node-local-surrogate-keys.md)).
  - Collision risk is negligible mathematically (74 random bits/ms); the real vectors are engineering defects. Mitigations: server-side generation only (Postgres/PGlite `uuidv7()`), entropy-readiness gate at boot, identity regeneration in the node provisioning ceremony. Backstop: PK conflicts with mismatched content hashes are quarantined to a repair queue, never silently merged.
  - UUIDv7 leaks creation timestamps by construction → raw UUIDs are not exposed in patient-facing URLs/documents.
- **Hybrid Logical Clocks (HLC)** on every event — causal ordering tolerant of skewed wall clocks on off-grid hardware.
- **Recording time vs. effective time.** The HLC stamps *recording time* (when the event entered the log); the clinically meaningful *effective time* (when the act was performed/observed) is a separate, author-asserted value. The two are almost never equal and that is normal — see [§3.6](#36-bitemporal-event-time-recording-time-vs-effective-time).

## 3.3 Mutable non-demographic state
| Data class | Merge policy |
|---|---|
| Allergies, alerts | **Union, never auto-delete.** Removal requires explicit reconciliation event. |
| Problem & medication lists | Union + flagged for clinician reconciliation on conflict |
| Scheduling / bed management | Authoritative-node ownership (the owning tier wins) |

Two decisions refine the medication-list row above without changing this merge policy. A duplicate
medication thread is cleared by a reversible **link**, never a fabricated cessation event, with a
symmetric min-UUID collapse to one current-list row ([ADR-0047](decisions/0047-medication-reconciliation-resolution.md)).
Separately, a clinician's clinical sign-off on a medication thread is a **separable, per-thread
attestation overlay** (principle 10) whose "still current?" signal is a convergent set-commitment
compare over the thread's append-only content — sound against a lower-HLC event arriving after the
sign-off, where a head-position pin would silently misclassify it as reviewed — and is never
retracted, only superseded by a corrective event ([ADR-0049](decisions/0049-commitment-based-sign-off-currency.md)).
A third decision refines the dose-timeline overlay beneath the medication-list row: a dose
correction **patches** the targeted point's dose / effective / reason **per-field** (an explicit
`strike` group sets a field unknown), and a corrected effective date **participates in current-dose
winner selection** — fixing a mis-keyed date is bitemporal repair, not a display-label edit
([ADR-0050](decisions/0050-dose-correction-per-field-patch.md)).

(Demographics are not modeled as a mutable record — see [§4](demographics.md).)

## 3.4 Interoperability
- Internal schema is event-sourced relational; a **FHIR R4/R5 façade** provides import/export and interop. **FHIR is a façade — a boundary skin, never the storage model** (see [§3.5](#35-event-storage-model-hybrid-envelope)). Cairn's internal model is canonical (a national-scale system is the thing others integrate *against*); FHIR is generated on demand for exchange with external/legacy systems and is not allowed to dictate the schema. The **native node API** that first-class Cairn UIs bind to is a separate, richer surface from this interop façade ([language-substrate §9.5](language-substrate.md#95-layering-the-node-api-and-ui-pluralism-uniform-core-plural-edges)).

## 3.5 Event storage model — hybrid envelope
> Resolves former open question §11.2 — see [ADR-0001](decisions/0001-fat-postgres-thin-daemon.md). The concrete signature/serialization primitives (Ed25519 over deterministic-CBOR/COSE_Sign1, signing the stored bytes) are ratified in [ADR-0015](decisions/0015-event-serialization-signatures-and-content-addressing.md). Every signature is **domain-separated by a registered signing context** (protected-header content type + `external_aad`), and an event carries **exactly one envelope signature** — the authoring actor's; co-signing and re-attestation are overlay events, responsibility is the separable attestation token — see [ADR-0040](decisions/0040-signing-context-domain-separation.md).

The clinical event log ([§3.1](#31-append-only-clinical-event-log-source-of-truth)) is stored as **append-only event tables with a hybrid shape**, splitting columns by *what must be machine-enforced or matched* vs. *what is opaque clinical content*:

- **Typed/normalized envelope columns** — everything the safety machinery, identity subsystem, sync layer, and matcher must read or constrain: `uuidv7` primary key ([§3.2](#32-identity-time)), `patient_uuid` (FK), the **HLC** as typed fields (physical timestamp, logical counter, node id; [§3.2](#32-identity-time)) — this *is* the recording time `t_recorded`, the **contributor set** ([§3.9](#39-authorship-and-accountability)) replacing the single author/device field, the **signature** (origin + integrity only — *not* attestation, [§3.9](#39-authorship-and-accountability)), `event_type` (a **closed enum**), scope keys (facility / department / encounter — the `encounter` key is a *thin* context grouping, not a formal visit, [§3.15](#315-the-active-write-model-thin-encounters-co-produced-legibility-and-the-delete-vs-erase-distinction)), `created_at`, and **`t_effective` with its precision/interval qualifier** ([§3.6](#36-bitemporal-event-time-recording-time-vs-effective-time), [§3.7](#37-acknowledged-uncertainty-uncertainty-capable-value-types)). Invariants live here because constraints can reach these columns (e.g. the `t_effective ≤ t_recorded` ceiling); JSONB they cannot reach unbypassably.
- **Cairn-native JSONB clinical body** — the actual clinical payload (note/observation/order/result/etc.). JSONB avoids re-modeling the sprawling clinical content as relational tables and keeps the FHIR façade cheap, **without** adopting FHIR's resource graph as the schema. The body's integrity is its **signature**, not a SQL constraint — appropriate, since clinical content is immutable and signed.
  - **The body slot is encryption-capable by construction — reserved from day one** ([§3.8](#38-erasure-and-key-custody), [ADR-0005](decisions/0005-erasure-key-custody-and-crypto-shredding.md)). A body is *either* plaintext JSONB *or* **ciphertext under a per-unit data-encryption key (DEK)** wrapped for a set of key-holders (`{node}` by default; optionally `{patient}` and/or named `{clinicians}`). This shape cannot be retrofitted onto an append-only log without re-encrypting history. **Every clinical JSONB body is now born sealed** — sealed at write under a per-event DEK wrapped for the node's own key ([ADR-0052](decisions/0052-born-sealed-clinical-bodies.md)): an **erasability** substrate, *not* confidentiality (the node reads its own data freely; projections and full-text search behave exactly as for plaintext), whose only effect is that every [§3.8](#38-erasure-and-key-custody) erasure rung stays reachable for every event forever. This *refines* the ADR-0005 §2 posture — what §2 reserved as *per-record encryption, off by default* becomes **born-sealed under node custody, on by default** for clinical bodies. Demographic/identity/node-plane/erasure-plane events stay **plaintext by necessity** — the matcher, identity algebra, and sync bind on them, and the shred tombstone must outlive all keys. The **legibility twin travels inside the sealed region under the same DEK**; the outer plaintext twin on a sealed row is a signed mechanical stub, and a sealed twin is never materialized into any plaintext index. The envelope columns are never encrypted — identity, sync, and matching bind on them.
  - **A sealed body emits a plaintext *safety projection* sibling** ([identity §5.9](identity.md#59-sensitivity-grade-the-safety-projection-and-break-glass-visibility-scope), [ADR-0006](decisions/0006-visibility-scope-replication-and-the-safety-projection.md)): de-identified safety **classes** (interaction/allergy class, Rh-sensitizing event) + a **severity grade**, mechanically projected from the body's coded fields and replicated in the clear like an allergy, so decision-support fires on a sealed episode without disclosing it. Its coarseness is set by a graded, append-only **sensitivity** stream (effective grade = projection). Relatedly, the **semantic scope key may be abstracted to an opaque "confidential-episode" routing token** — the only envelope generalization permitted; `patient_uuid` and the HLC stay plaintext so identity/sync/matching still bind.
- **Demographic-assertion events are the exception: their fields are typed columns, not JSONB**, because the matcher ([§5.2](identity.md#52-matching-pipeline-safety-asymmetric-false-merge-worse-than-false-split)) and the coherence checks ([§4.2](demographics.md#42-per-field-projection-policy), [§5.2](identity.md#52-matching-pipeline-safety-asymmetric-false-merge-worse-than-false-split)) read them and the identity algebra enforces invariants on them.

Rule of thumb: *normalized/typed where invariants, identity, sync, or matching bind; JSONB for clinical bodies; FHIR only at the façade.*

## 3.6 Bitemporal event time (recording time vs. effective time)
> Surfaced while case-mining former open question §11.3 — see [ADR-0003](decisions/0003-bitemporal-time-and-acknowledged-uncertainty.md).

Every event carries **two times**, because the time a thing is *done* is almost never the time it is *recorded*: a busy ED clinician may write the resuscitation note hours later, after the patient has moved to ICU; professionals enter data for the same patient at different times and places, patient sometimes present, sometimes not. There is no way — short of total surveillance — to objectively capture "time performed"; the system records what it *can* know objectively and lets the human assert the rest.

- **`t_recorded`** — the objective time the event entered the log, carried by the **HLC** ([§3.2](#32-identity-time)). Machine-assigned, immutable, the basis for causal ordering and sync. It is the **hard ceiling** on effective time: an event cannot have been performed *after* it was recorded, so **`t_effective ≤ t_recorded` is an envelope invariant**. A violation is *prima facie* falsification, rejected/flagged at write.
- **`t_effective`** — the author's assertion of when the event actually happened. It defaults to `t_recorded`, may be freely **backdated** by the author (a routine, legitimate act — *not* falsification), and is the time **displayed** to clinicians, with `t_recorded` shown in brackets. **Wire form:** inside the signed body an asserted `t_effective` is an ISO-8601 instant with a **mandatory explicit UTC offset** — an offset-less local timestamp would denote a *different instant* on differently-configured nodes, a silent divergence inside an immutable signed artifact — validated at every admission door (refines [ADR-0015](decisions/0015-event-serialization-signatures-and-content-addressing.md); [issue #91](https://github.com/cairn-ehr/cairn-ehr/issues/91)).

> [!NOTE]
> The HLC gives causal *ordering* tolerant of skewed clocks — **not wall-clock truth.** A node with a drifting RTC and no time sync can place `t_recorded` arbitrarily. So `t_recorded` is not a bare point: it carries a **clock-confidence grade** and a bracketing **interval** ([§3.17](#317-trusted-time-anchoring-the-clock-confidence-grade-and-the-bracketed-t_recorded), [ADR-0027](decisions/0027-trusted-time-anchoring.md)) — [principle 4](index.md#founding-principles-the-lens-for-every-decision) applied to time. The ceiling invariant `t_effective ≤ t_recorded` reads against that bound; the grade tells a reader how much to trust the ceiling.

**Two orderings, on purpose:**

- **Integrity / sync** order by `t_recorded` (the HLC) — the objective causal order.
- **The clinical narrative** is a projection ordered by `t_effective` — the timeline a clinician reasons over. The chart can offer both lenses ("as it happened" vs. "as it was recorded"), itself a powerful audit affordance.

Mere disagreement between the two orderings is the **expected** case — a note written at 18:00 about a 14:30 event sorts into the narrative at 14:30 while staying late in recording order. Disagreement is never, by itself, a clash.

**Clash detection (flag, never resolve).** A *clash* is the narrower case where an asserted `t_effective` produces a *logical impossibility* against an objective anchor (e.g. a treatment whose effective time precedes the patient's recorded presentation to the facility).

- **Tier 1 — universal, free:** the self-ceiling `t_effective ≤ t_recorded`. Needs no domain knowledge; catches the crudest falsification; enforced as an envelope constraint.
- **Tier 2 — clinical brackets:** a small, **closed, explicitly-enumerated** set of episode-bracket constraints (*treated-before-presenting*, *inpatient-event-after-discharge*, …), where the bracketing events carry their own objective floors. This is a [§9](language-substrate.md) coherence check, **not an open rules engine** — the same closed-set discipline as the identity event algebra ([§5.7](identity.md#57-identity-event-algebra-closed-set-all-append-only-syncable-auditable)).

> [!IMPORTANT]
> On a clash the system **surfaces it and stops** — it never silently reorders and never erases.
> Either timestamp may be the wrong one, and only the humans who were there can reconcile; the UI
> offers resolution as a **new overlaying event with full audit trail**
> ([§3.1](#31-append-only-clinical-event-log-source-of-truth)). Forcing the system to pick a winner
> would manufacture a *precise untruth*, which founding principle 4
> ([§3.7](#37-acknowledged-uncertainty-uncertainty-capable-value-types)) forbids.

## 3.7 Acknowledged uncertainty (uncertainty-capable value types)
> Embodies founding principle 4 — *an imprecise near-truth beats a precise untruth* ([ADR-0003](decisions/0003-bitemporal-time-and-acknowledged-uncertainty.md)).

Most EHRs force clinicians to commit data they cannot vouch for — a required date-of-birth satisfied only by `01/01/1900`, a yes/no where the honest answer is "don't know". The record then fills with confident falsehoods that are worse than acknowledged gaps: a fake-precise DOB actively *misleads* the matcher ([§5.2](identity.md#52-matching-pipeline-safety-asymmetric-false-merge-worse-than-false-split)), where an honest "unknown" is weighted correctly. The data model therefore makes uncertainty first-class:

- **Precision-tagged and interval values.** A date may be known to the year, the month, the day, or "circa"; values may be ranges ("50–60 yo", "2–3 days", "sometime overnight"). `t_effective` ([§3.6](#36-bitemporal-event-time-recording-time-vs-effective-time)) carries such a precision/interval qualifier.
- **`null` ≠ `unknown` ≠ `refused`.** *Nobody-asked*, *asked-but-unestablished*, and *patient-declined* are clinically distinct facts the system must preserve distinctly — most EHRs collapse them into one empty cell and lose the difference.
- **No forced precision (normative).** No required field may be satisfiable *only by fabrication*. If a workflow needs a field, that field must accept an honest uncertainty value.
- **Monotonic refinement by overlay.** "circa 2019" today, "12 Mar 2019, confirmed from old records" as a later overlaying event ([§3.1](#31-append-only-clinical-event-log-source-of-truth)). Certainty increases over time **without erasure** — a natural fit with the append-only log.
- **Quantities are stored in canonical SI; the unit is intrinsic to the value.** The core never stores an ambiguous bare number and never *infers* a unit (the mg/mcg, mmol/L vs mg/dL, metric/imperial confusions are a classic clinical-safety killer). A clinical quantity is held in canonical SI, encoded against an international unit standard (e.g. UCUM); locale-specific *display* and entry framing are a UI translation governed by policy — [principle 12](index.md#founding-principles-the-lens-for-every-decision) (uniform core, plural edges) applied to quantities. This is a one-line application of an existing principle, not a separate decision.

> [!NOTE]
> **Two distinct forms of acknowledged uncertainty — don't conflate them.** This section is about
> uncertain or absent **values**: an unknown DOB, an imprecise date, an estimated age. A clinician's
> **provisional or differential assertion** — the `?diabetic` notation, a ranked differential,
> "probable PE" — is a *different* thing: an explicitly-flagged clinical **hypothesis**, carried in the
> clinical body ([§3.5](#35-event-storage-model-hybrid-envelope)), not a value-typing concern. Both
> honor founding principle 4, but they are different mechanisms. Representing differentials and their
> probabilities in the clinical body is deeper content modeling, deferred.

## 3.8 Erasure and key custody
> Resolves former open question §11.5 — see [ADR-0005](decisions/0005-erasure-key-custody-and-crypto-shredding.md). Mechanism summary; the *why* and the security posture live in the ADR and [§7](security.md).

The append-only log ([§3.1](#31-append-only-clinical-event-log-source-of-truth)) is never mutated to delete — a deleted row would break its signature and hash chain and would be **resurrected by set-union sync** from any sibling, backup, or WORM archive. Instead, **erasure is the redistribution of key-custody, not the deletion of data**: a clinical body sealed under a DEK ([§3.5](#35-event-storage-model-hybrid-envelope)) is erased by **destroying the key** (crypto-shredding). The row remains immutable, its signature still verifies, sync still works — the body is now keyless noise, and a resurrected opaque row is harmless (no key, no projection references it). This is the only deletion model compatible with append-only + WORM archival.

- **Per-record encryption with a key-holder hierarchy** (`{node}` + optionally `{patient}`/`{clinicians}`) is the substrate, now split into two properties ([ADR-0052](decisions/0052-born-sealed-clinical-bodies.md)): **erasable** — the shipped default for clinical bodies, born sealed under a per-event DEK whose custody *includes the node* (it hides nothing; it exists so the erasure ladder stays reachable) — and **sequestered** — the *same* DEK with custody **narrowed** (node → named clinicians/patient), opt-in and graded. Sequestering an already-erasable event is a **key-custody change, not a rewrite**, which is what dissolves the "sensitivity recognized later" case (no retro-encryption; just re-wrap/withdraw keys). Patient-as-key-holder stays opt-in because it trades availability for confidentiality (a lost patient key = oblivion) and the default must not.
- **A policy-neutral severity ladder** of erasure mechanisms (hide → sequester → deniable sealed-escrow deletion → audited crypto-shred → best-effort oblivion) spans the worst-case extremes (indefinite retention ↔ complete erasure). Cairn builds the rungs; **which are reachable is policy/UI configuration**, never a stance the system takes.
- **Deletion is best-effort and *declared*, never guaranteed** (a corollary of acknowledged uncertainty, [§3.7](#37-acknowledged-uncertainty-uncertainty-capable-value-types)): offline nodes, old backups, and WORM cannot be confirmed. The strongest honest claim is *"to our knowledge, we have erased all copies in our existence."*
- **Crypto-shred is itself an append-only event, so erasure survives disaster recovery.** A rung-3 shred **destroys the wrapped-DEK rows and scrubs every derived plaintext** — the operational twin, projections, and any FTS/RAG index (**mandatory invalidation**: a shred that leaves the body's text searchable is not a shred; [ADR-0052](decisions/0052-born-sealed-clinical-bodies.md)) — then appends the signed tombstone; the append-only log row is never touched. A node-controlled backup is just another replication peer (a cold peer, [security §7.10](security.md#710-node-durability-and-disaster-recovery)); a restore **replays the shred log and re-applies erasures before projecting**, and **shred completion includes propagation to all attached node-controlled backup media** (re-wrapping their key material). A backup can no more silently resurrect an erased body than a sibling node can; only detached/offline media remain the declared honest ceiling.

The full ladder, the deniable-deletion design (the institution holds nothing; the clinician's cover migrates to a self-held sealed copy), and the keystore's safety-critical status are in [ADR-0005](decisions/0005-erasure-key-custody-and-crypto-shredding.md) and [§7](security.md). **Binary attachments inherit this model unchanged** — a per-blob DEK makes a content-addressed blob crypto-shreddable exactly as a body slot is ([§3.14](#314-attachments-content-addressed-blobs-and-the-rendition-set)).

## 3.9 Authorship and accountability

> [!IMPORTANT]
> **Authorship is compositional; accountability is separable** (founding principle 10,
> [ADR-0007](decisions/0007-authorship-and-accountability.md)). "AI-generated" is not a flag — it is the
> emergent reading of a richer model.

- **Contributor set** (replaces the single `author`/device envelope field). Each event's authorship is
  a *set* of contributors; each entry is `{ identity, role, descriptor?, responsibility? }`. `identity`
  is a registered actor — **human**, **AI agent** (model + version + vendor + deploying node), or
  **device**. The ordinary human note is a one-element set, so the common case gets no heavier; an
  AI-scribed note the clinician edited and signed is `{AI, drafted}` + `{clinician, attested, …}` — mixed
  authorship and mixed responsibility inside one immutable row. **An event is "AI-generated" iff its set
  contains a non-human author and no human in a responsibility-bearing role** — true by construction, never
  tagged. The contributor set is also the selection key for a clinician's **author-scoped record export**
  ([security §7.8](security.md#78-author-scoped-record-export-the-medico-legal-copy)).

- **Role — a closed core enum + free descriptor** ([ADR-0028](decisions/0028-finalized-closed-contributor-role-enum.md)
  finalises the membership; [ADR-0051](decisions/0051-contributor-role-vocabulary-floor-and-responsibility-wire-shape.md)
  adds `recorded` and makes the closure a floor property). Roles are a **closed enum** (like `event_type`), kept small
  so the safety/DB layer reasons about them unambiguously and the taxonomy cannot sprawl into an unbounded folksonomy.
  It is partitioned by whether a role *bears or transfers responsibility*: **responsibility-bearing** (`authored`,
  `ordered`, `attested`, `co-signed`, `witnessed`, `dictated`) vs **contributory** (`drafted`, `transcribed`,
  `graded`, `triaged`, `suggested`, `recorded` — the recording device/system that captured and persisted the event:
  capture fidelity, no content, no clinical responsibility). An optional **free-text descriptor** carries nuance the
  machinery never branches on. **Closure is floor, not convention:** the authoring door fails closed on any role
  outside the ratified vocabulary, while the sync apply door **never** rejects on membership (set-union losslessness).
  A member added by a future ADR travels **partition-prefixed on the wire** (`bearing:<name>` / `contrib:<name>`, the
  prefix a permanent part of the signed value) so a node that predates it still classifies it; a role neither ratified
  nor prefixed reads as **vouching-unknown** — a first-class honest state (principle 4), never collapsed to
  "un-vouched".
  Three of the bearing roles are responsibility-distinct in ways hard policy and the [identity §5.10](identity.md#510-authorship-and-responsibility-state-the-consumer-side)
  projection branch on: **`co-signed`** (supervisory countersignature — the registrar→consultant / NP→supervisor
  sign-off a deployment may gate on, *"pending until co-signed"*); **`witnessed`** (attests an event *occurred or
  was observed* — controlled-drug waste, consent, restraint, verbal-order read-back, death verification — not that
  content is vouched-for); **`dictated`** (the voice source of clinical content: bears the clinical *intent* while
  the *verbatim text* rides a `transcribed` contributor and carries an ASR/transcription-accuracy gap until
  separately attested, distinct from `authored` owning the exact words). The bar for any future member is that the
  **safety/policy layer must branch on it** — otherwise the distinction is a descriptor (flavor), a policy gate, or
  an acknowledgment ([identity §5.12](identity.md#512-the-notification-economy-salience-responsibility-routing-and-the-acknowledgment-floor)),
  which is why **`reviewed` is deliberately *not* a role** (it is either `attested` or an acknowledgment). These
  roles describe **contribution to the record, not performance of the clinical act** (so `performed` is body
  content, not a role; `ordered` sits on the line by design). The set is **closed and additive-only** — a new
  member is an ADR-recorded act, never ad-hoc.

- **Responsibility — `{ held_by, on_behalf_of }`, not a boolean.** *Absent* = un-vouched (a legitimate
  state, below). `held_by` a human, no `on_behalf_of` = ordinary self-attestation. `held_by` an AI agent
  with `on_behalf_of` a legal entity = the **proxy** case — the output is accountable, accountability
  routing to its owner/deployer. The attribute is **orthogonal to human/machine**: *"AI is never
  responsible" is a policy default mapping, not a schema law.* The column exists from day one, so the
  transition from "software needs a human to take responsibility" toward "the AI colleague is accountable
  (initially as proxy for its owner)" is a **policy change with no schema migration**. This shape is the
  **wire form**, floor-enforced ([ADR-0051](decisions/0051-contributor-role-vocabulary-floor-and-responsibility-wire-shape.md)):
  `held_by` must name the contributor entry's own actor, which must be the verified attester (the
  issue-#195 binding chain — the record never carries a responsibility claim about someone who never
  touched the event); responsibility may ride only a responsibility-*bearing* role. `on_behalf_of` is
  wire-expressible from day one but **refused at the authoring door until a proxy-grant ADR defines its
  verification**, and admitted at the sync apply door as a signed, display-gated, unverified claim
  (refusing it there would wedge future lawful proxy events out of the set-union).

- **Authorship binding — a bearing-role human contributor must be signer or verified attester**
  ([ADR-0053](decisions/0053-per-write-human-authorship.md), generalising the
  [#195](https://github.com/cairn-ehr/cairn-ehr/issues/195)/ADR-0051 responsibility binding to
  authorship, one field over). A contributor whose role is **responsibility-bearing** and whose
  `actor_id` resolves to a **human** actor must be authenticated as **the event's signer** or **a
  verified attester** — floor-enforced at **strict submit** (a door only authors what it can stand
  behind: a forged `{human_X, "authored"}` signed by another key with no token is refused), but
  **graded, never refused, at apply** (an unverifiable claim — actor ≠ signer, no verifiable token —
  is admitted and graded, per [ADR-0012](decisions/0012-schema-evolution-event-format-and-legibility-across-time.md)'s
  never-refuse-what-you-can't-understand: the door cannot distinguish a forgery from a future
  authentication scheme it is too old to parse). A bearing role carried **without** a `responsibility`
  object is the legitimate **"authored, not-yet-vouched"** state — the ordinary case of a human who
  signs and authors an event with no attestation yet (responsibility, if any, remains the separate
  ADR-0049 overlay).

- **Authorship-confidence grade — `attested` / `unverified` / `device`** (ADR-0053,
  `classify_authorship_confidence`, the ADR-0051 `classify_role` discipline: one shared pure
  predicate, upgradable as newer nodes re-grade). **`attested`** — a human author authenticated as the
  event's signer or a verified attester. **`unverified`** — a human-author claim this node cannot
  verify; rendered *"authorship claimed, not authenticated here"*, never *attested*, never dropped,
  and re-gradable when a newer node can parse the credential. **`device`** — recorded-only, no human
  author (the honest device-additive default, [ADR-0051](decisions/0051-contributor-role-vocabulary-floor-and-responsibility-wire-shape.md)).
  The middle **`asserted`** rung (a *named* human author with no key present — verbal/telephone
  orders) and a token-backed author who did not sign (AI-scribe, dictation) are reserved by the
  binding's verified-attester arm but deferred.

- **Signature ≠ attestation.** The **signature** proves *origin + integrity* only; **attestation** (a
  responsibility-bearing role) confers *responsibility*. Every event is signed, including AI output —
  *signed ≠ vouched-for* ([security §7.2](security.md#72-signing-attestation-and-ai-agent-identity)).

- **No responsible party is legitimate, and structurally characterised.** An event may carry **zero**
  responsibility-bearing contributors. The safe-by-construction case is a **strictly additive** output —
  one that can only *raise* signal (priority, a warning) and can never reduce, defer, de-prioritise,
  auto-file, or auto-resolve something a human would otherwise act on. Its worst case is exactly the paper
  baseline (principle 3 — a safety net laid *under* the floor, never a hole cut *in* it), so nothing new is
  created to answer for. The **additive-vs-suppressing nature of an output is a recordable, projectable
  property**; whether an *un-owned suppressing* output is permitted is **policy** (principle 9), and an
  override toward permitting it is itself an explicit, audited, owned configuration act.

- **Classifying additive vs suppressing — derived, not declared** ([ADR-0010](decisions/0010-additive-vs-suppressing-classification.md),
  refining the property above). The classification is **structural**, never a producer-set flag (a
  self-declared *"I am additive"* is exactly the flag this model rejects). **Additive ≡ overlay**
  (adds a layer the human still sees and can act on; source-preserving, always-overridable, monotone) —
  the append-only principle ([§3.1](#31-append-only-clinical-event-log-source-of-truth)) applied to the
  attention/decision layer. **Suppressing ≡ foreclosure** (removes, hides, defers, auto-acknowledges,
  auto-files, auto-resolves). The falsifiable test: *could a human still independently see and act on
  everything they would have without this output?* — yes → additive, no → suppressing.
- **Demotion is additive; only hiding or auto-deciding is suppressing.** Lowering the priority of a signal
  (the flood of objectively-normal results) is additive — it still reaches the human ([identity §5.12](identity.md#512-the-notification-economy-salience-responsibility-routing-and-the-acknowledgment-floor)); the line is crossed only at hide-to-nothing or auto-action. The **suppressing operations are a closed,
  enumerated set** (the merge-policy discipline of principle 1 — auto-acknowledge, auto-resolve, auto-file,
  filter-hide, below-threshold-suppress, auto-substitute, auto-decline); additive is the open complement
  and the default, curated with a **suppressing-until-proven-additive** review discipline. **Enforcement is
  structural:** the trusted apply layer refuses a suppressing-class operation lacking a responsible owner —
  an un-owned producer is confined to the additive vocabulary by construction, the same shape as the
  [§5.5](identity.md#55-reattribution-one-primitive-tiered-workflows) Tier-1 bar and the [identity §5.12](identity.md#512-the-notification-economy-salience-responsibility-routing-and-the-acknowledgment-floor) never-withhold floor.
- **Conservation of responsibility; declaration is a one-way caution ratchet.** Suppression is never truly
  un-owned — accountability sits at the event, or (where policy permits an un-owned suppression class) at the
  explicit audited configuration act that permitted it; policy *relocates* the owner, never abolishes it.
  Author/deployer declaration may only ratchet an output **toward** needing an owner (a formally-additive but
  practically-relied-upon triage marked "treat as suppressing"), **never away** — the handle for *de-facto*
  suppression (automation complacency), whose consumer-side detection is [identity §5.10](identity.md#510-authorship-and-responsibility-state-the-consumer-side).

- **Lifecycle rides existing lineage.** Responsibility that attaches *over time* — an AI fires a draft
  now (`{AI, drafted}`, un-vouched); a human vouches later — is a **new event referencing the draft**
  (`{human, attested, responsibility: human}`), exactly how signatures, addenda, and corrections already
  work ([§3.1](#31-append-only-clinical-event-log-source-of-truth)). No new overlay stream; principle 1 is
  satisfied (the draft is never mutated). How the clinician *sees* authorship and responsibility-state is
  [identity §5.10](identity.md#510-authorship-and-responsibility-state-the-consumer-side).

## 3.10 Session identity, event authorship, and draft durability
> Resolves former open questions §11.9/§11.12 (the data-model invariants) — see [ADR-0008](decisions/0008-point-of-care-identity-possession-and-salvage.md), canonical design [identity §5.11](identity.md#511-point-of-care-identity-possession-fast-authentication-and-salvage).

Two infrastructure invariants underpin point-of-care possession and work-salvage. They are the minimal data-model commitments; everything above them ([identity §5.11](identity.md#511-point-of-care-identity-possession-fast-authentication-and-salvage)) is implementation/UI/policy. The thin **encounter** grouping that the committed events born in such a context share — and the type-through write model and delete-vs-erase distinction that govern how they are authored and edited — is [§3.15](#315-the-active-write-model-thin-encounters-co-produced-legibility-and-the-delete-vs-erase-distinction).

- **`session.user` and `event.author` are independently bindable.** The data model must **never** assume `note.author == session.user`. The contributor set ([§3.9](#39-authorship-and-accountability)) of an event is established by the *attribution* act at commit time (which authenticates the author — *attestation*, [security §7.2](security.md#72-signing-attestation-and-ai-agent-identity)), not by whoever happens to hold the *session*. This is what makes **`sign-as`** possible (attribute and sign a note as the true author without changing the logged-in user), and its absence is exactly why deployed EHRs cannot salvage stranded work. *Authentication* is thereby unbundled into **gatekeeping** (session-level, coarse, rare) and **attribution** (per-event, cheap, the binding that actually reaches `event.author`).
- **Drafts are durable and session-decoupled.** An uncommitted write-context survives an authentication-context change: it stays bound to its **subject** (the `patient_uuid` never wavers — you were always writing about this patient), is owned by its **provisional author** (so a draft follows *that clinician* to wherever they re-authenticate, and a "switch" hides but never discards the previous user's draft), and carries a provisional authorship-confidence grade resolved on commit ([identity §5.11](identity.md#511-point-of-care-identity-possession-fast-authentication-and-salvage)). This extends the append-only work-preservation guarantee ([§3.1](#31-append-only-clinical-event-log-source-of-truth)) — which protects *committed* events — to the *pre-commit* side of the commit boundary; the same value (never discard clinician effort) on both sides. The context/draft store is keyed by `(author, patient)`, not by the session, which is also what lets one contended workstation hold several warm, hidden contexts at once.
- **The invariant is realized at the data/floor/CLI layer: the authoring signature is the per-write attribution act** ([ADR-0053](decisions/0053-per-write-human-authorship.md)). A clinical event may carry an authenticated human author distinct from the recording session: `signer_key_id` is the human, the contributor set is `{human, "authored"}` + `{node, "recorded"}` ([§3.9](#39-authorship-and-accountability)), and the node — the session party — stays `recorded` and keeps body custody (seals the event and holds its DEK, [ADR-0052](decisions/0052-born-sealed-clinical-bodies.md)) regardless of who signs. This is the node-as-session / human-as-author split that makes `session.user ≠ event.author` cryptographic rather than aspirational, on the one clinical stream (medication) that exists. The **durable session-decoupled drafts** and **`sign-as`** stranded-work salvage described above remain **explicitly deferred to the UI layer** — no draft store or session/UI layer exists yet to hang them on; this slice lays down only the can't-retrofit wire shape and floor binding beneath them.

## 3.11 Notifications as projections, responsibility-routing, and acknowledgment
> Resolves former open question §11.10 — see [ADR-0009](decisions/0009-notification-economy-salience-routing-and-the-acknowledgment-floor.md), canonical design [identity §5.12](identity.md#512-the-notification-economy-salience-responsibility-routing-and-the-acknowledgment-floor). Minimal data-model commitments; the clinical model and the *why* live there and in the ADR.

Three invariants underpin the notification economy. They keep notifications inside the append-only model rather than as a side-band of mutable state.

- **A notification is a projection, never a stored mailbox.** It is a *delta* over the append-only log evaluated against the consuming clinician's own audit history (what they have viewed/acted on, [§7](security.md)). There is **no mutable unread-flag** to set and delete; *never merge, always overlay* applies here exactly as to the link graph and the sensitivity grade. **Acknowledgment is an append-only audit event** (`{who, when, action-taken?}`), a single explicit human act — **never auto-satisfied for the hard-acknowledgment class** (an auto-ack would assert a human closed the loop who did not — the silent-falsification exclusion of [vision §1.2](vision.md#12-the-paper-parity-test-normative)). No new stream: notifications derive from the clinical log; acknowledgment rides the audit stream.
- **Responsibility-to-follow-up is a graded, multi-source, append-only overlay; the effective responsible set is a projection** — the same shape as the sensitivity stream ([§3.5](#35-event-storage-model-hybrid-envelope)/[identity §5.9](identity.md#59-sensitivity-grade-the-safety-projection-and-break-glass-visibility-scope)) and the link graph. The **orderer** tag is intrinsic; policy overlays fallback tags (default critical-results owner, covering-doctor reassignment, timeout reassignment) and **more than one clinician may hold a tag at once**. The data model carries the tag overlay and the timeout-reassignment primitive; whose acknowledgment *discharges* the obligation versus merely records a view is policy.
- **Routing is never a visibility gate** (the safety floor, and the consumer-side mirror of [ADR-0006](decisions/0006-visibility-scope-replication-and-the-safety-projection.md)'s *"replication is never the confidentiality boundary"*). A result is **always** readable by a clinician who has opened the patient; an orderer-release preference is at most ambient state and the architecture **never** represents it as withholding. **Suppression is the accountable act** ([§3.9](#39-authorship-and-accountability)): demotion/coalescing/digest is additive and free; filtering-out / below-threshold-hiding / auto-acknowledge is a suppressing output and is owned, audited, and policy-gated. A hard-ack class can never be filtered to nonexistence; filtering changes modality, never existence.

## 3.12 Actor identity in the registry
> Resolves the [ADR-0007](decisions/0007-authorship-and-accountability.md) AI-agent-identity follow-on — see [ADR-0011](decisions/0011-actor-registry-version-pinning-and-key-custody.md), canonical design [security §7.5](security.md#75-the-actor-registry-enrollment-version-pinning-and-key-custody). Minimal data-model commitments.

The contributor-set `identity` ([§3.9](#39-authorship-and-accountability)) is not a free string — it resolves against a **general actor registry** (human / device / AI agent) that obeys the same append-only discipline as the rest of the model.

- **Actor identity is immutable and version-pinned; the registry is a projection over a closed actor-event algebra** (`enroll / supersede / revoke / suspend / rotate-key`) — the same shape as the [identity §5.7](identity.md#57-identity-event-algebra-closed-set-all-append-only-syncable-auditable) patient-identity algebra. Never mutated: a version bump is a new actor-UUID with a `supersede` link; a compromise is a `revoke` overlay. *Never merge, never erase — always link, always overlay*, for non-human actors too.
- **Identity granularity tracks objectively-recordable behavioral determinants.** An AI-agent identity pins the **declared standing configuration** that materially shapes behavior — `vendor, model, version, weights ref, inference/decoding config (temperature, top-p/k, sampling), system-prompt/template, tool/RAG config, deploying node`; a change to any is a supersession. **Per-invocation parameter variance is stamped on the event**, not minted as identities — the [§3.6](#36-bitemporal-event-time-recording-time-vs-effective-time) objective-vs-asserted split, so both are queryable for recall. A **human** carries no behavioral-config dimension (mood/fatigue are real but not objectively recordable; fabricating a criterion violates [§3.7](#37-acknowledged-uncertainty-uncertainty-capable-value-types)).
- **Signing publics are immortal; DEKs are destroyable** — opposite lifecycles that *"key custody"* must not smear ([security §7.5](security.md#75-the-actor-registry-enrollment-version-pinning-and-key-custody)). A historical event stays signature-verifiable forever (a superseded/revoked actor's public persists; `revoke` distrusts *new* events after a compromise-time, never *old* ones), whereas DEKs are crypto-shredded for erasure ([§3.8](#38-erasure-and-key-custody)). Every AI-agent enrolment **must record a named responsible human** (the introduction-accountability backstop, [ADR-0010](decisions/0010-additive-vs-suppressing-classification.md)); ongoing output-responsibility stays separable/policy ([§3.9](#39-authorship-and-accountability)).

## 3.13 Schema evolution, event format, and the legibility twin
> Resolves former open question §11.4 — see [ADR-0012](decisions/0012-schema-evolution-event-format-and-legibility-across-time.md). The two planes and lossless forwarding are [sync §6.5](sync.md#65-schema-evolution-two-planes-and-lossless-forwarding); the distribution plane is [security §7.6](security.md#76-the-software-distribution-plane-signed-releases-and-extension-load).

The append-only log ([§3.1](#31-append-only-clinical-event-log-source-of-truth)) cannot be migrated the classic way: a historical event signed under one schema must stay **byte-identical forever** (a rewrite breaks its signature and would be resurrected by set-union sync), and a fleet of offline nodes carries **permanent, unbounded version skew** — a node may receive an event authored under a *newer* schema it has never seen, or one *older* than its own code, and a resource-constrained site may never upgrade at all. Schema evolution is therefore *the append-only/overlay and acknowledged-uncertainty principles applied to the schema itself*. Four invariants are **reserved from day one** because, like `t_effective` ([§3.6](#36-bitemporal-event-time-recording-time-vs-effective-time)) and the encryption-capable body slot ([§3.5](#35-event-storage-model-hybrid-envelope)), they cannot be retrofitted onto an append-only log:

- **`schema_version` on every event** — the body-format version within its `event_type` family. It is deliberately also the future join key into a schema-descriptor registry, so a generic descriptor-driven renderer can be added later as pure read-side machinery with **no envelope change and no migration** (deferred by design — [ADR-0012](decisions/0012-schema-evolution-event-format-and-legibility-across-time.md)).
- **A mandatory, signed, mechanically-derived plaintext legibility twin on every event** — the [principle 11](index.md#founding-principles-the-lens-for-every-decision) substrate. Derived from the body at write-time by code that understands the format, carrying a `rendered-by` stamp (schema + renderer version), it lets a node *generations* behind read the event as a clinician reads a progress note. It is not merely a fallback: it is the version-independent substrate for **human audit, full-text search, and compact RAG context**, and that value (plus compression at rest) repays its storage cost. There are two twins — the **signed carried twin** (the author's faithful write-time rendering; travels and is trusted downstream) and an optional **locally-regenerated twin** (a projection; an upgraded node may re-derive a richer one). At the point of care the carried twin is **co-produced inline as the human-readable note line** in one keystroke flow with the structured event, so the twin is born at authoring time rather than bolted on ([§3.15](#315-the-active-write-model-thin-encounters-co-produced-legibility-and-the-delete-vs-erase-distinction)). **Demographic assertions ([§4.1](demographics.md#41-demographic-assertions)) are a twin-bearing event class**: every demographic assertion carries this twin, materialised profile-independently so a node lacking the field's profile still reads the fact ([§4.5](demographics.md#45-the-demographic-legibility-twin), [ADR-0034](decisions/0034-demographic-legibility-twin.md)). The **carried authored twin is global to every event class**, not only demographics: a conformant author materialises it for every event; the in-DB floor prefers it and, for non-demographic types, **degrades honestly** to a flagged mechanically-derived twin when an older or non-conformant peer omitted it (set-union is never broken), with authored-vs-derived recoverable from the signed body ([ADR-0039](decisions/0039-globalise-authored-legibility-twin.md)).
- **Lossless passthrough.** A node stores, re-propagates, and exports the **original signed bytes untouched** — never reject, never drop, never down-convert, never re-serialize. This requires the **signature to cover a canonical byte representation stored as such**, *not* one re-derived from JSONB (JSONB does not preserve key order, whitespace, or duplicate keys, so re-serialization would break both signature validity and the round-tripping of fields a node does not understand). A node's local annotations on an event it cannot fully parse are **additive overlays** referencing it ([§3.1](#31-append-only-clinical-event-log-source-of-truth)) — never edits.
- **Additive-only evolution** — *never erase, always overlay* ([§3.1](#31-append-only-clinical-event-log-source-of-truth)/[identity §5.1](identity.md#51-linkage-layer-never-merge-always-link)) applied to the schema: never remove or repurpose a field; never delete or renumber a closed-enum value (`event_type`, the role enum [§3.9](#39-authorship-and-accountability), the identity [§5.7](identity.md#57-identity-event-algebra-closed-set-all-append-only-syncable-auditable) and actor [§3.12](#312-actor-identity-in-the-registry) algebras) — only add, deprecate by overlay. A new constraint may only be one all historical events already satisfy, or is scoped going-forward (binds events recorded under schema ≥ X).

**The effective rendering is one projection bounded on two axes** — `min(what this node can parse, what this node is cleared to see)`. Version-skew degradation and confidentiality degradation are the **same mechanism**: the ladder rich-structured → generic-descriptor (deferred) → carried plaintext twin → the [identity §5.9](identity.md#59-sensitivity-grade-the-safety-projection-and-break-glass-visibility-scope) safety projection (sealed body) → the partition-honest floor (*"event of type X, authored by Y, N fields, not interpretable here"*). **Coarseness varies; existence never disappears** — the §5.9 safety-floor invariant generalized. The **version-skew tolerance window is infinite for custody, best-effort for understanding**: a node may never refuse or discard an event it does not understand. Local DDL/projection migration is the easy layer — projections are rebuildable and never synced ([§3.1](#31-append-only-clinical-event-log-source-of-truth)), so a bad projection schema is recovered by drop-and-rebuild; the log is never DDL-migrated to delete ([§3.8](#38-erasure-and-key-custody)).

> [!NOTE]
> **Attachments add a third axis to this `min()` — retrievability** ([§3.14](#314-attachments-content-addressed-blobs-and-the-rendition-set)). A large binary body may be present, *pending* (referenced but not yet synced), or *shredded*, so its effective rendering is `min(retrievable, parseable, cleared)`. The three axes degrade down one ladder to one honest floor; coarseness varies, existence never disappears.

## 3.14 Attachments: content-addressed blobs and the rendition set
> Resolves former open question §11.6 — see [ADR-0013](decisions/0013-attachments-content-addressed-lazy-blob-tier.md). The lazy byte tier and reference-eager replication are [sync §6.6](sync.md#66-attachments-the-lazy-byte-tier); erasure inherits [§3.8](#38-erasure-and-key-custody)/[§7.1](security.md#71-erasure-the-severity-ladder). The concrete digest algorithms (event SHA-256; blob **BLAKE3**, provisional pending the Pi/ARM throughput number) are ratified in [ADR-0015](decisions/0015-event-serialization-signatures-and-content-addressing.md).

**Attachments** — the binary clinical artifacts that are not naturally Cairn-native JSONB bodies (DICOM imaging, scanned legacy paper, clinical photography, ECG/EEG/CTG waveforms, dictation audio, endoscopy/ultrasound video, externally-signed referral PDFs, genomic data) — are **content-addressed blobs referenced by the signed event, never inlined**. This is *the append-only principle applied to large binary content*: the content digest is to a blob what the signature is to an event body. Same bytes → same address → idempotent set-union with zero merge.

- **Named by digest, integrity by reference.** The event body names each attachment by a **self-describing content digest** (algorithm + value, multihash-style), and the **event signature covers that digest** ([§3.1](#31-append-only-clinical-event-log-source-of-truth)). A blob carries no separate signature: trust chains from the signed event into the bytes, so a blob **self-verifies against any source** and tampering is detectable. (Tiny blobs below a node-tuned threshold may instead be **inlined** in the body and ride the eager plane; both forms are expressible from day one.)
- **The reference is eager; the bytes are lazy.** The attachment reference replicates with the clinical event on the normal sync plane; the bytes live in the [sync §6.6](sync.md#66-attachments-the-lazy-byte-tier) **lazy by-reference tier**. A node therefore *always knows an attachment exists* before it arrives, and fetches the bytes on legitimate need ([sync §6.4](sync.md#64-scope-is-a-prefetch-hint-not-an-authority)). A not-yet-retrieved blob renders through honest-assembly ([sync §6.2](sync.md#62-consistency-model)) as *"referenced here — not yet retrieved"* — paper-parity *exceeded* (a missing film on paper is invisibly absent).
- **The rendition set is the binary's legibility twin.** One logical attachment is **N content-addressed renditions** (raw gigabytes + kilobyte preview + extracted report text), each with its own sync priority. This resolves the [§3.13](#313-schema-evolution-event-format-and-the-legibility-twin) tension — *the plaintext twin is not derived from the pixels*; it is derived from the event's coded/descriptor fields (*"Chest CT with contrast, reported: no PE"*), and the lightweight rendition is the blob's twin. The retrievability axis (above) degrades a blob down the same legibility ladder to the same honest floor.
- **Erasure and lossless passthrough inherit unchanged.** A blob is **encryption-capable by construction** like the §3.5 body slot: plaintext (content-addressed by plaintext hash, dedup within a trust domain, whole-storage-encrypted at rest) *or* sealed under a **per-blob DEK** (content-addressed by ciphertext hash, crypto-shreddable). The [§3.8](#38-erasure-and-key-custody)/[§7.1](security.md#71-erasure-the-severity-ladder) ladder applies with no new mechanism; **GC ≠ erasure** (a garbage-collected blob is re-fetchable, a shredded one is keyless noise); **no convergent encryption** for sealed blobs (it would leak *"someone holds this exact file"*). Bytes obey [§3.13](#313-schema-evolution-event-format-and-the-legibility-twin) lossless passthrough — **never transcoded in place** (re-encoding breaks embedded signatures and changes the hash); a derived preview is a *new* rendition added, never a replacement. **The public clear-text descriptor is graded** ([ADR-0052](decisions/0052-born-sealed-clinical-bodies.md)): for a sensitive attachment the *precise* descriptor lives under the seal and only a coarsened stub is public, so a re-identifying descriptor (*"photo of self-harm scars, left forearm"*) never outlives the pixels a shred destroys — the same coarseness ladder as the [identity §5.9](identity.md#59-sensitivity-grade-the-safety-projection-and-break-glass-visibility-scope) safety projection.

**The day-one envelope reserve.** Only the **attachment-reference shape** is can't-retrofit (it rides the signed, immutable event); the store, dedup, GC, and transfer protocol are mutable infrastructure. The reference must from day one carry: a **self-describing content digest** (so a future hash algorithm is an *additive* migration while the original digest stays fixed), a **seal indicator / DEK-wrap reference** (so the attachment is crypto-shreddable later), **clear-text descriptor metadata** (media type, byte length, modality/descriptor — so a sealed/pending/unparseable blob still renders and feeds the safety projection), the **rendition set**, and the **inline-vs-reference** distinction. DICOM/WADO/IHE-XDS stays a **façade** over this, never the storage model (the §3.4 FHIR posture applied to imaging/document exchange).

> **Concrete shape ([ADR-0042](decisions/0042-concrete-attachment-reference-shape.md)).** The reference is `Attachment { descriptor, renditions: [Rendition{ role, alg, digest_hex, media_type, byte_len, inline?, seal? }] }`, with `seal` a `SealRef { alg, dek_wrap }`. The rendition set is nested from day one (structurally can't-retrofit); `seal` and `inline` are reserved (omitted from the wire when absent). Attachments on non-narrative events ride the signed `EventBody.attachments`; the [ADR-0041](decisions/0041-progress-note-narrative-format.md) note `payload.media` manifest is the same primitive plus a note-local `id`.

## 3.15 The active-write model: thin encounters, co-produced legibility, and the delete-vs-erase distinction
> See [ADR-0020](decisions/0020-active-write-thin-encounters-and-the-delete-vs-erase-distinction.md). How a clinician authors and edits at the point of care, and the data-model commitments that follow. It rides on the point-of-care possession binding ([§3.10](#310-session-identity-event-authorship-and-draft-durability), [identity §5.11](identity.md#511-point-of-care-identity-possession-fast-authentication-and-salvage)); the forced-rationale gate lives in [vision §1.2](vision.md#12-the-paper-parity-test-normative). No new envelope field, no new event stream, no new founding principle.

The write surface is where founding principles 1, 2, 3, 4 and 11 all bind at once, so the same *one-word-hides-many-dials* discipline that resolved "scope", "signature", "authentication" and "priority" applies again to "encounter", "the order's consult", "the note line", and "delete".

- **The `encounter` scope key is a *thin* context, not a formal visit.** It is an opaque **grouping id that asserts nothing about formality** — a small first-class header of the same shape as the event envelope (`{ HLC time, place/scope keys, contributor set, ≥1 linked events }`) that events point at via the [§3.5](#35-event-storage-model-hybrid-envelope) `encounter` key. Whether the context was a formal consultation, a phone call, or a five-second results-review (where one orders a test *with a comment*, no consultation having occurred) is a **separate, possibly-absent descriptor**, never forced — [principle 4](index.md#founding-principles-the-lens-for-every-decision) forbids manufacturing a consultation that did not happen, and a "virtual encounter" for one annotated order is zero-ceremony and first-class. **Guard the model against importing FHIR-`Encounter` / billing semantics: it is a grouping id, full stop.** Its author **may be non-human** — an automatic recall system spawns a context and the generated orders/letters hang off it ([§3.9](#39-authorship-and-accountability) compositional authorship applied to context creation; signature proves origin, attestation is absent or proxied to the recall-policy owner).
- **Events inherit the encounter ambiently from the armed write-context.** It is the grouping that events born in an [§3.10](#310-session-identity-event-authorship-and-draft-durability) possession context share; an event authored in the active context inherits the `encounter` key the same ambient way it inherits facility/department — no new field, no bespoke wiring.
- **Order provenance falls out of the encounter key — it is not a feature.** "Reproduce the ordering consult" = fold all events sharing that encounter key; the result-returns-later chain is a direct two-hop fold `result → references order → order.encounter → fold that encounter` (the order is the pivot and carries the key *for free* because authored in context). This **structurally explains the external-results gap**: a referral-in / post-hospital result authored under a foreign node's context carries someone else's encounter key or none, so it degrades **honestly** ([principle 4](index.md#founding-principles-the-lens-for-every-decision)) to a *labelled* fallback (*"most recent · ordering context unknown"*), **never silently presented as the ordering consult**. Cairn-to-Cairn federation can preserve the link; a foreign system cannot. A later AI cross-reference only *proposes* a link as a new event ([§3.9](#39-authorship-and-accountability) overlay discipline), never asserts one.
- **Structured event and human-readable note line are co-produced in one keystroke flow.** A **UX invariant, not a schema addition**: orders, prescriptions and referrals are authored *inside* an armed encounter, never as free-floating actions. The `rx!`/`tx!`+tab type-through opens an entry surface **beside** the note, never over it (*"never modal" extends from reading to writing*); dosing is a **smart default except where a default could harm** (paediatric/pregnant/breastfeeding/renal/hepatic → manual entry **forced**) — strip keystrokes where safe, force attention where it counts ([principle 4](index.md#founding-principles-the-lens-for-every-decision) + paper-parity together). The readable line that results is a **derived projection of the structured event — the [§3.13](#313-schema-evolution-event-format-and-the-legibility-twin) legibility twin rendered inline, born at authoring time**: there is exactly **one** event and the prose is a rendering *of* it, so it cannot diverge. This is [principle 11](index.md#founding-principles-the-lens-for-every-decision) made concrete at the point of authoring. The clinician's only freedom over the line is its **visibility**.
- **`delete` (a rendering) and `erase` (the data) are distinct verbs the conventional EHR conflates.** *"Delete only ever removes one UI aspect of the data representation, never the original data"* — **never-erase-always-overlay ([principle 2](index.md#founding-principles-the-lens-for-every-decision)) applied to the display layer.** Deleting a note line suppresses a *rendering*; the event's time, author, context and downstream processing are untouched (the test is still ordered, resulted, interaction-checked) and the data resurfaces because it never left. **The suppression is itself an explicit visibility-overlay event (who/when)** — directly auditable, not merely an inferred absence; the *that* is always recorded, the *why* may stay unstated (often patient confidentiality). The event's mandatory legibility twin ([§3.13](#313-schema-evolution-event-format-and-the-legibility-twin)) is untouched — still the signed audit/RAG substrate, just not *rendered* in that view. This slots under [ADR-0006](decisions/0006-visibility-scope-replication-and-the-safety-projection.md): confidentiality lives in visibility/presentation, never in existence/replication (the STI-screen case — the structured event persists, so safety projection and interaction-checking still protect the patient; only the prose narration is withheld). `erase` is the [§3.8](#38-erasure-and-key-custody) crypto-shred — irreversible, the ≈-never case. Routine `delete` therefore needs **zero** friction *because* it destroys nothing, which is exactly what reserves the one gate Cairn permits ([vision §1.2](vision.md#12-the-paper-parity-test-normative)) for the irreversible few.

> [!WARNING]
> The thin-encounter grouping is mostly fit-for-purpose (a mis-grouping is visible and overlay-repairable), but two seams are safety/privacy-critical and belong in the trusted apply surface ([§9](language-substrate.md)): the **`delete`-is-never-`erase`** boundary (a deletion that silently became a crypto-shred is irreversible data loss) and the **suppression-is-always-a-recorded-overlay-event** invariant (an unrecorded or leaking rendering-suppression is an audit/confidentiality breach). The recurring seam motif — one safety-critical path through an otherwise fit-for-purpose write surface.

## 3.16 Clinical concept coding: the ICD-11 interlingua and the local-terminology overlay
> See [ADR-0025](decisions/0025-icd-11-canonical-interlingua-and-local-terminology-overlay.md) for the *why* and the licence/acceptance reasoning; [ecosystem eval 0003](../ecosystem/0003-reference-data-sourcing-medicines-and-terminologies.md) for the sourcing survey. Drug substance identity anchors on the WHO INN by the same discipline. No new envelope field beyond a classification slot; no new founding principle.

The decision pathway keys on **stable concept identifiers, never free-text names** that drift and re-spell over time. **WHO ICD-11 is the canonical classification interlingua** — persistent entity URIs (`https://id.who.int/icd/entity/{id}`) + MMS stem codes, deployed from the free offline `whoicd/icd-api` container so the node never depends on the cloud ([ADR-0001](decisions/0001-fat-postgres-thin-daemon.md) availability floor). SNOMED CT is **excluded** as the canonical pivot — member/affiliate-gated and fee-bearing in non-member territories (the AMT defect), available only as a node-local licensed plug-in. ICD-11 is canonical **at the node/data-model layer, for interop, decision-support, the safety projection ([identity §5.9](identity.md#59-sensitivity-grade-the-safety-projection-and-break-glass-visibility-scope)) and reporting** — *not* baked into the terminology-agnostic wire core ([principle 12](index.md#founding-principles-the-lens-for-every-decision), [language-substrate §9.5](language-substrate.md#95-layering-the-node-api-and-ui-pluralism-uniform-core-plural-edges)).

- **A coded event stores the ICD-11 identifier as the primary structured classification value *and* a structured tag into a local-terminology concept.** The clinician's own language is never demoted to bare free-text; source and pivot coexist. The **local terminology is an append-only, on-site-curated collection of free-text terms** ([principle 1](index.md#founding-principles-the-lens-for-every-decision) applied to vocabulary — the deployment-owned *plural edge*); each term binds to an ICD-11 entity by an **append-only, overlay-able mapping assertion**.
- **Map-once-remember-forever, *offered* not forced.** A novel local term offers a one-time ICD-11 binding; thereafter it auto-translates silently. The clinician may decline and leave the mapping **deliberately open** — an honest *not-yet-coded* state ([§3.7](#37-acknowledged-uncertainty-uncertainty-capable-value-types), distinct from *unknown*/*refused*) routed to a **professional-coder worklist** rather than a forced guess that becomes coding debt. The mapping is a **separately-authored act** ([§3.9](#39-authorship-and-accountability) compositional authorship): the clinical claim is the clinician's, the coding claim the coder's, neither overwriting the other. Coding **never blocks the clinical write** ([principle 4](index.md#founding-principles-the-lens-for-every-decision) + paper-parity).
- **Two ICD-11 views, bitemporally** ([§3.6](#36-bitemporal-event-time-recording-time-vs-effective-time), [ADR-0003](decisions/0003-bitemporal-time-and-acknowledged-uncertainty.md)): the **as-asserted** code is version-pinned and immutable on the event (which ICD-11 release + mapping version produced it); the **current-best** code is re-derived through the live mapping when ICD-11 revises or a binding is corrected. Both coexist; neither erases. The [§3.13](#313-schema-evolution-event-format-and-the-legibility-twin) legibility twin preserves the source term and label *as written*, so the event stays readable across version drift ([principle 11](index.md#founding-principles-the-lens-for-every-decision)).
- **Licence posture (the body slot, applied to vocabulary):** ICD-11 codes/URIs ship **verbatim with attribution** (CC BY-ND 3.0 IGO); Cairn **bundles no Cairn-authored ICD-11↔SNOMED/ICD-10/external crosswalk** — such maps are node-local, version-pinned distribution-plane plug-ins ([ADR-0012](decisions/0012-schema-evolution-event-format-and-legibility-across-time.md)), some under separate WHO/source licences. The **on-site local-term→ICD-11 bindings are the deployment's own data** (original mappings that merely reference WHO codes verbatim), which is why retaining the source assertion also keeps Cairn licence-clean (the map stays recomputable rather than a forbidden derivative).
- **Alternative classifications are pluggable translation layers, never on the inter-node path.** ICPC-3, SNOMED CT, ICD-10-AM or a national scheme attaches as a node-local layer that *produces ICD-11* (and may bulk-populate local-term bindings); the **UI presents the terminology of choice**, while what is canonical and what crosses between nodes is the mapped ICD-11 + the structured source tag — the [ADR-0014](decisions/0014-locale-pluggable-matcher-comparators.md) pluggable-edge / uniform-pivot posture applied to classification.

> [!NOTE]
> Mostly fit-for-purpose: a mis-mapping is visible (source term and label still show) and overlay-repairable, so the auto-translate and the external plug-ins optimise for iteration ([§9](language-substrate.md)). The one safety-relevant floor is that **coding can never block or silently alter the clinical write** — an unmapped diagnosis must always be recordable, and the as-asserted code must never be mutated in place (corrections are overlays). The open-mapping worklist is an additive signal ([ADR-0010](decisions/0010-additive-vs-suppressing-classification.md)): it only raises *"these await coding,"* never hides or auto-decides.

## 3.17 Trusted-time anchoring: the clock-confidence grade and the bracketed `t_recorded`
> Resolves former open question §11.14 — see [ADR-0027](decisions/0027-trusted-time-anchoring.md). [Principle 4](index.md#founding-principles-the-lens-for-every-decision) ([§3.7](#37-acknowledged-uncertainty-uncertainty-capable-value-types)) applied to wall-clock truth. One day-one envelope field; no new founding principle.

The HLC ([§3.2](#32-identity-time)) orders events robustly under skew but does not establish wall-clock *truth*; a drifting RTC with no time sync can place `t_recorded` ([§3.6](#36-bitemporal-event-time-recording-time-vs-effective-time)) arbitrarily, with medico-legal consequences (was a record created when it claims, or backdated to cover a mistake?). The honest treatment is **not** a single authoritative timestamp but the **§3.7 uncertainty-capable time type**: `t_recorded` is a **graded interval**, where trusted-time anchoring is simply what populates the bounds and the grade. Two sub-problems are kept distinct: **clock-setting** (trustable *current* time — bounds `t_recorded` from **below**) and **existence-proof** (proving an event existed by a time — bounds it from **above**); together they **bracket** `t_recorded`.

- **The clock-confidence grade — one ordered ladder, best-corroboration-wins** (the [§7.1](security.md#71-erasure-the-severity-ladder) severity / [§3.13](#313-schema-evolution-event-format-and-the-legibility-twin) legibility / [§3.14](#314-attachments-content-addressed-blobs-and-the-rendition-set) retrievability shape): `unknown < self-asserted (RTC) < network-synced (NTS/Roughtime) < hardware-sourced (GNSS/TPM) < externally-anchored (notary/transparency-log token) < multi-anchor-corroborated`. A reader decades on sees whether a `t_recorded` was independently anchored or merely self-asserted.
- **`self-asserted` is the honest default.** With no anchor configured a node still signs a timestamp, but that proves **integrity, not external time** — it is graded *self-asserted* and **never displayed as a trusted timestamp** (distinct from *unknown*, [§3.7](#37-acknowledged-uncertainty-uncertainty-capable-value-types)).
- **Envelope floor + overlay refine** (the day-one, can't-retrofit piece). The **initial grade + interval are a mandatory envelope field**, born at mint (the best clock provenance the node had); **later anchor tokens are overlays** that upgrade the grade and tighten the interval — token renewal before algorithm obsolescence is [ADR-0015](decisions/0015-event-serialization-signatures-and-content-addressing.md) re-attestation-as-overlay. Envelope for the floor, overlay for refinement — exactly how `t_recorded` is the immutable ceiling while certainty refines upward ([§3.7](#37-acknowledged-uncertainty-uncertainty-capable-value-types)).
- **Confidence is graded, never required.** A solo offline node gets an honest bracket (a TPM monotonic floor + RTC, with the HLC for ordering), not a blocked write — the [ADR-0001](decisions/0001-fat-postgres-thin-daemon.md) availability floor and [principle 4](index.md#founding-principles-the-lens-for-every-decision) (an imprecise near-truth beats a precise untruth). The pluggable anchors, the offline bracket, and the notary node role live in [security §7.11](security.md#711-trusted-time-anchoring-the-notary-anchor-node-role) and [sync §6.8](sync.md#68-time-attestation-rides-the-gossip-plane); clock-confidence is itself a first-class honest-assembly fact, shown like sync freshness ([sync §6.2](sync.md#62-consistency-model)).

> [!IMPORTANT]
> Safety-critical ([§9.1](language-substrate.md#91-selection-rule-by-defect-blast-radius)): the grade+interval envelope field, the best-corroboration composition, and anchor-token verification (a forged/mis-verified anchor corrupts the medico-legal record). The clock-setting clients and the notary/log *server* are fit-for-purpose. HLC ordering and wall-clock truth are orthogonal and compose — this never changes causal ordering.

## 3.18 Canonical identifiers and node-local surrogate keys: the dual-identifier discipline
> See [ADR-0031](decisions/0031-canonical-identifiers-and-node-local-surrogate-keys.md). [Principle 3](index.md#founding-principles-the-lens-for-every-decision) (paper-parity — retrieval speed is a safety floor) applied to identifier representation; the [§9.5](language-substrate.md#95-layering-the-node-api-and-ui-pluralism-uniform-core-plural-edges) layering applied to physical keys. No new founding principle.

Federation requires **globally-unique, offline-mintable** identity (UUIDv7 and content-addresses, [§3.2](#32-identity-time) / [ADR-0015](decisions/0015-event-serialization-signatures-and-content-addressing.md)). Those are the right identifiers for *identity* and the wrong ones for physical *join keys*: 16-byte UUIDs and 34-byte random multihashes, propagated across a fanned-out foreign-key graph, inflate every referencing index and evict cache — and in an EHR a slow retrieval fails [paper-parity](index.md#founding-principles-the-lens-for-every-decision). Federation demands globally-unique identity; it does **not** demand that identity be the physical join key. These are two jobs owned by two planes Cairn already separates — the signed, synced **event core** versus the per-node, rebuildable **projection** ([§3.1](#31-append-only-clinical-event-log-source-of-truth)).

- **Canonical plane unchanged.** UUIDv7 / multihash identifiers are the *only* identifiers that ever appear in a signed body, on the inter-node wire, as a content-addressing input, or as a stable API identity. Immortal ([principle 2](index.md#founding-principles-the-lens-for-every-decision)).
- **Projection plane may intern canonical IDs to node-local `bigint` surrogates** (`bigint`, never `int`) as the physical FK/join key. A private interning dictionary maps each canonical ID to a dense surrogate.
- **The hard rule:** a surrogate must **never** escape the projection plane (no signed body, no wire, no content-address input, no stable API identity) — a leaked surrogate means two nodes assign different integers to the same entity and set-union sync silently diverges. Guarded structurally: a `local_ref` domain type distinct from `uuid`/`bytea`, and mapping confined to the floor functions ([§9.6](language-substrate.md#96-the-validated-submit-surface-the-write-path) `submit_event` on ingress; projection-read / sync-emit on egress). API egress is always the global ID.
- **Bind the pair once per entity, at its anchor row; references carry only the surrogate.** Carrying both the UUID and the surrogate is correct at the anchor row and on the already-signed `event_log` (where the UUIDs are mandatory anyway) and self-defeating on referencing rows (it re-imports the fan-out cost it removes). Downstream references carry only the `ref`; the UUID is recovered by a join to the anchor, only at egress.
- **Scope by where the cost is, confirmed by measurement.** Strongest case: wide random `BYTEA` references; next: high-fan-out `patient_id`; leave `event_id` PKs as UUIDv7. Magnitude/scope is measured on [Spike 0001](../spikes/0001-walking-skeleton-wan-sync-and-pi-cost.md) Bet B, exactly as [ADR-0001](decisions/0001-fat-postgres-thin-daemon.md)'s compute bet is.
- **Surrogates are not durable identity** — not stable across a projection rebuild, never portable across nodes. Anything durable that must outlive a rebuild references the canonical ID, never the surrogate.

## 3.19 The progress-note narrative format: one signed event, markdown narrative, and manifest-keyed media anchors
> See [ADR-0041](decisions/0041-progress-note-narrative-format.md). The first narrative clinical surface: [principle 11](index.md#founding-principles-the-lens-for-every-decision) (the raw body is human-readable) and [principle 12](index.md#founding-principles-the-lens-for-every-decision) (rendering belongs to the edges) applied to rich clinical text. Rides the [§3.14](#314-attachments-content-addressed-blobs-and-the-rendition-set) attachment shape, the [§3.15](#315-the-active-write-model-thin-encounters-co-produced-legibility-and-the-delete-vs-erase-distinction) encounter fold, and the [§3.10](#310-session-identity-event-authorship-and-draft-durability) draft store. No new envelope field; no new founding principle.

A progress note is **one signed event** (`clinical.note.asserted`, `schema_version =
clinical.note/1` — three-segment like every production type, opening the `clinical.*` namespace
that separates clinical content from administrative/infrastructure streams; the walking-skeleton
placeholder `note.added`/`note/1` is retired by this slice's build migration and never reaches a
production wire); **intra-note structure is never
inter-event structure.** A linked list or container-per-event note would put intra-note ordering on
the sync plane, where concurrent offline appends fork it into a tree (a merge with no
clinically-reasoned policy) and partial delivery renders dishonestly — the dangerous-merge class the
architecture precludes. One event gives intrinsic ordering, atomic arrival, one twin, one signature,
one attestation — the paper model: the clinician signs the *entry*. Everything a multi-event note
would buy already exists: between-notes structure is the encounter fold
([§3.15](#315-the-active-write-model-thin-encounters-co-produced-legibility-and-the-delete-vs-erase-distinction)),
continuation/correction is overlay, concurrent authorship is two events folded by `t_effective`, and
incremental writing lives pre-commit in the durable draft store
([§3.10](#310-session-identity-event-authorship-and-draft-durability)), never as events.

- **Body = `narrative` + `media` (+ `refs`, + nullable `encounter`).** `narrative` is a single
  markdown string in a **pinned, versioned, austere profile** (`clinical.note/1` — the profile is
  the narrative grammar of the `schema_version`, one identifier: paragraphs, bold, italic,
  headings, lists, blockquote, the anchor grammar; **raw HTML and load-bearing external URLs are
  excluded forever**; tables/footnotes/code blocks are excluded from v1 and addable additively —
  hand-written tables are usually a modeling smell: drug lists and similar constructs should be
  *generated* from structured events). `media` is a manifest of
  [§3.14](#314-attachments-content-addressed-blobs-and-the-rendition-set) attachment references,
  each with a note-local `id`; the §3.14 shape's existing **human `descriptor` is tightened to
  mandatory-non-empty** (one field — the same string the safety projection and degraded render
  read; no second description field). Structured actions
  (orders, prescriptions) are separate events rendered into the note *view* by the encounter fold —
  the note event is pure narrative.
- **One anchor grammar for every media kind:** `![<descriptor>](cairn:att/<id>)` means *"render this
  manifest entry here"* — the manifest's `media_type` decides *how* (image, audio player, waveform,
  or the honest *"referenced — not yet retrieved"* card). New modalities are new media types, zero
  format change. **The anchor's text is the manifest entry's descriptor, byte-identical** — display,
  twin, and safety projection can never disagree. Anchors are **manifest-keyed, never inline
  digests** (a 64-hex digest in prose
  wrecks raw legibility; the digest lives once in the manifest under the same signature).
  **Anchor↔manifest integrity is floor-enforced**: a dangling anchor, empty anchor text, or
  anchor-text↔descriptor mismatch is rejected at the submit floor and identically at the mirrored
  remote-apply door (deterministic over the signed bytes) — though under
  [§6.5](sync.md#65-schema-evolution-two-planes-and-lossless-forwarding) version-skew custody a
  renderer must still treat an unresolvable anchor as an honest degraded card. An un-anchored
  manifest entry is legal
  (paper-clip parity). Reserved additively: the **event-anchor** `![<text>](cairn:event/<uuid>)`,
  rendering a referenced event's twin at that position (its `<text>` free-form — no manifest entry
  to mirror) — the future home of the §3.15 type-through
  order-line-inside-narrative case.
- **The descriptor is the twin substrate.** The [§3.13](#313-schema-evolution-event-format-and-the-legibility-twin)
  authored twin is mechanically derived: the narrative verbatim, each anchor replaced by
  `[attachment: <descriptor> — <media_type>, <size>]` (plus one line per un-anchored entry), the
  `<descriptor>` read from the manifest (authoritative; the anchor text is equal by construction).
  What a
  text-only node, full-text search, and the RAG substrate see. Never record media you cannot
  describe ([§3.7](#37-acknowledged-uncertainty-uncertainty-capable-value-types)).
- **Figure-granular erasure falls out.** The note holds digest + descriptor; the bytes are a
  separately-sealable blob. A per-blob DEK crypto-shred ([§3.8](#38-erasure-and-key-custody)) kills a
  wound photo while the signed note stays **byte-identical and legible**; the anchor degrades to
  `[attachment: … — shredded]` down `min(retrievable, parseable, cleared)`.
- **Drawn graphics** (body charts, wound outlines — the everyday paper-parity case) are
  **static-profile SVG** attachments (no script, no external href, no CSS import; floor-enforced)
  with a **mandatory flattened-raster rendition** — the pinned *"what was signed"* appearance, immune
  to renderer drift. Template annotation = stroke-overlay SVG referencing the template blob's digest.
- **`refs: [{rel, event}]`** carries inter-event relationships; `rel` is a small closed enum
  (`addendum-to`, `correction-of`, `transcript-of`), evolved additively. **Floor-validated like the
  anchors:** unknown `rel` fails closed; each `event` must exist locally at submit (the author held
  what they reference, so causal HLC order delivers the target first — mirrored at the apply door).
  All rels are **additive** — they add context, never hide or demote the target (a `correction-of`
  marks the original corrected; both stay visible; cross-author corrections are legal additive
  claims). Suppression stays with the suppressing-overlay machinery and its
  `target_event_id`/owner-gate path, never a new `rel`. Audio's dual role resolves here without a
  special case: a
  recording that *is* content (verbal consent, treatment refusal, a psychotic episode, verbatim
  dictation) is a manifest entry; an AI-scribe narrative *derived from* a recording carries
  `transcript-of` to the recording's event, with roles in the
  [§3.9](#39-authorship-and-accountability) contributor set.

> [!IMPORTANT]
> Safety-critical ([§9.1](language-substrate.md#91-selection-rule-by-defect-blast-radius)), all at
> the validated submit floor ([§9.6](language-substrate.md#96-the-validated-submit-surface-the-write-path)):
> anchor↔manifest integrity (existence, non-empty text, text = descriptor), markdown- and
> SVG-profile enforcement, descriptor non-emptiness, the `refs` gate (closed enum + target
> existence), twin fidelity, and the inherited digest binding — mirrored at the remote-apply door.
> Every renderer, editor, and player is fit-for-purpose.
> The profile's austerity is what keeps the floor validator reviewer-legible.
