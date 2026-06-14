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
> ([ADR-0001](decisions/0001-fat-postgres-thin-daemon.md)).

## 3.2 Identity & time
- **UUIDv7 primary keys everywhere** (native `uuidv7()` in PostgreSQL 18) — globally unique, offline-generable, time-ordered.
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

(Demographics are not modeled as a mutable record — see [§4](demographics.md).)

## 3.4 Interoperability
- Internal schema is event-sourced relational; a **FHIR R4/R5 façade** provides import/export and interop. **FHIR is a façade — a boundary skin, never the storage model** (see [§3.5](#35-event-storage-model-hybrid-envelope)). Cairn's internal model is canonical (a national-scale system is the thing others integrate *against*); FHIR is generated on demand for exchange with external/legacy systems and is not allowed to dictate the schema.

## 3.5 Event storage model — hybrid envelope
> Resolves former open question §11.2 — see [ADR-0001](decisions/0001-fat-postgres-thin-daemon.md).

The clinical event log ([§3.1](#31-append-only-clinical-event-log-source-of-truth)) is stored as **append-only event tables with a hybrid shape**, splitting columns by *what must be machine-enforced or matched* vs. *what is opaque clinical content*:

- **Typed/normalized envelope columns** — everything the safety machinery, identity subsystem, sync layer, and matcher must read or constrain: `uuidv7` primary key ([§3.2](#32-identity-time)), `patient_uuid` (FK), the **HLC** as typed fields (physical timestamp, logical counter, node id; [§3.2](#32-identity-time)) — this *is* the recording time `t_recorded`, author / device, signature, `event_type` (a **closed enum**), scope keys (facility / department / encounter), `created_at`, and **`t_effective` with its precision/interval qualifier** ([§3.6](#36-bitemporal-event-time-recording-time-vs-effective-time), [§3.7](#37-acknowledged-uncertainty-uncertainty-capable-value-types)). Invariants live here because constraints can reach these columns (e.g. the `t_effective ≤ t_recorded` ceiling); JSONB they cannot reach unbypassably.
- **Cairn-native JSONB clinical body** — the actual clinical payload (note/observation/order/result/etc.). JSONB avoids re-modeling the sprawling clinical content as relational tables and keeps the FHIR façade cheap, **without** adopting FHIR's resource graph as the schema. The body's integrity is its **signature**, not a SQL constraint — appropriate, since clinical content is immutable and signed.
  - **The body slot is encryption-capable by construction — reserved from day one** ([§3.8](#38-erasure-and-key-custody), [ADR-0005](decisions/0005-erasure-key-custody-and-crypto-shredding.md)). A body is *either* plaintext JSONB *or* **ciphertext under a per-unit data-encryption key (DEK)** wrapped for a set of key-holders (`{node}` by default; optionally `{patient}` and/or named `{clinicians}`). This shape cannot be retrofitted onto an append-only log without re-encrypting history, so it is fixed now even though per-record encryption is **off by default** (default deployments use whole-storage encryption, [§7](security.md)). The envelope columns are never encrypted — identity, sync, and matching bind on them.
  - **A sealed body emits a plaintext *safety projection* sibling** ([identity §5.9](identity.md#59-sensitivity-grade-the-safety-projection-and-break-glass-visibility-scope), [ADR-0006](decisions/0006-visibility-scope-replication-and-the-safety-projection.md)): de-identified safety **classes** (interaction/allergy class, Rh-sensitizing event) + a **severity grade**, mechanically projected from the body's coded fields and replicated in the clear like an allergy, so decision-support fires on a sealed episode without disclosing it. Its coarseness is set by a graded, append-only **sensitivity** stream (effective grade = projection). Relatedly, the **semantic scope key may be abstracted to an opaque "confidential-episode" routing token** — the only envelope generalization permitted; `patient_uuid` and the HLC stay plaintext so identity/sync/matching still bind.
- **Demographic-assertion events are the exception: their fields are typed columns, not JSONB**, because the matcher ([§5.2](identity.md#52-matching-pipeline-safety-asymmetric-false-merge-worse-than-false-split)) and the coherence checks ([§4.2](demographics.md#42-per-field-projection-policy), [§5.2](identity.md#52-matching-pipeline-safety-asymmetric-false-merge-worse-than-false-split)) read them and the identity algebra enforces invariants on them.

Rule of thumb: *normalized/typed where invariants, identity, sync, or matching bind; JSONB for clinical bodies; FHIR only at the façade.*

## 3.6 Bitemporal event time (recording time vs. effective time)
> Surfaced while case-mining former open question §11.3 — see [ADR-0003](decisions/0003-bitemporal-time-and-acknowledged-uncertainty.md).

Every event carries **two times**, because the time a thing is *done* is almost never the time it is *recorded*: a busy ED clinician may write the resuscitation note hours later, after the patient has moved to ICU; professionals enter data for the same patient at different times and places, patient sometimes present, sometimes not. There is no way — short of total surveillance — to objectively capture "time performed"; the system records what it *can* know objectively and lets the human assert the rest.

- **`t_recorded`** — the objective time the event entered the log, carried by the **HLC** ([§3.2](#32-identity-time)). Machine-assigned, immutable, the basis for causal ordering and sync. It is the **hard ceiling** on effective time: an event cannot have been performed *after* it was recorded, so **`t_effective ≤ t_recorded` is an envelope invariant**. A violation is *prima facie* falsification, rejected/flagged at write.
- **`t_effective`** — the author's assertion of when the event actually happened. It defaults to `t_recorded`, may be freely **backdated** by the author (a routine, legitimate act — *not* falsification), and is the time **displayed** to clinicians, with `t_recorded` shown in brackets.

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

- **Per-record encryption with a key-holder hierarchy** (`{node}` + optionally `{patient}`/`{clinicians}`) is the substrate; it is **off by default**, reserved for the cases below, because patient-held keys trade availability for confidentiality (a lost patient key = oblivion) and the default must not.
- **A policy-neutral severity ladder** of erasure mechanisms (hide → sequester → deniable sealed-escrow deletion → audited crypto-shred → best-effort oblivion) spans the worst-case extremes (indefinite retention ↔ complete erasure). Cairn builds the rungs; **which are reachable is policy/UI configuration**, never a stance the system takes.
- **Deletion is best-effort and *declared*, never guaranteed** (a corollary of acknowledged uncertainty, [§3.7](#37-acknowledged-uncertainty-uncertainty-capable-value-types)): offline nodes, old backups, and WORM cannot be confirmed. The strongest honest claim is *"to our knowledge, we have erased all copies in our existence."*

The full ladder, the deniable-deletion design (the institution holds nothing; the clinician's cover migrates to a self-held sealed copy), and the keystore's safety-critical status are in [ADR-0005](decisions/0005-erasure-key-custody-and-crypto-shredding.md) and [§7](security.md).
