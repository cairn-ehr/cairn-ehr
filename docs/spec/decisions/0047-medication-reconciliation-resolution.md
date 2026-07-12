# ADR-0047 — Medication reconciliation is a link, not a cessation

- **Status:** Accepted
- **Date:** 2026-07-13
- **Refines:** principle 2 (never merge, always link); [ADR-0010](0010-additive-vs-suppressing-classification.md) (additive-vs-suppressing); the [data-model](../data-model.md) rule *"medication lists = union + flagged for clinician reconciliation on conflict"*. Follows the identity linkage precedent ([§5.7](../identity.md), `patient_link`/`person_member`).

## Context

Slice 1 of the `clinical.medication` surface records each medication as an immortal `medication_id` thread and surfaces an advisory `patient_medication_reconciliation_flag` when two *active* threads for one patient share a duplicate key. Slice 1 named exactly one "resolution": **cease the redundant thread.** That is clinically wrong. Cessation (`clinical.medication-cessation.asserted`) means *the patient stopped taking the drug* — a false clinical statement when the two threads are simply the same ongoing medication recorded twice (two nodes, an encounter re-entry, a brand recorded alongside its generic). Using cessation to silence a duplicate flag fabricates a stop event: the drug drops off the current list, and the audit log asserts a discontinuation that never happened. This is the "slice-1 wart."

The record already has the right shape for this elsewhere: the **identity** subsystem never merges two patient records — it *links* them (`identity.link.asserted`), deriving a golden identity by connected component, cleanly reversible (principle 2). Medication threads that turn out to be the same drug are the identical problem one level down.

## Decision

**Reconciling two medication threads is a first-class, symmetric, reversible *link* between threads — never a cessation.** Clearing a duplicate flag never fabricates a stop event.

1. **Two additive verbs over a thread pair**, mirroring identity link/unlink: `clinical.medication-reconciliation.asserted` (state `reconciled`) and `clinical.medication-separation.asserted` (state `separated`, the never-erase reversal — *"these are actually two different drugs"*). Both carry two distinct `medication_id` subjects; both are `additive`, `targets_other_author=FALSE`. A reconciliation forecloses nothing — **both threads' full histories, dose timelines, and cessations survive verbatim** — so [ADR-0043](0043-suppression-self-only-disagreement-is-additive.md)'s owner-gate does not apply and **cross-author reconciliation is allowed** (clinician B may reconcile threads authored by A and C — normal clinical practice).

2. **Collapse by connected component (min-UUID canonical), mirroring `person_member`.** Reconciled threads form a group whose representative is the minimum `medication_id`; the current-medication list shows **one row per group**. Reconciliation is permitted between **any** two threads, not only flagged ones — this is the *only* path today for a human to reconcile brand↔generic, which the deterministic key-based flag cannot detect. The floor validates structure only (two distinct valid UUID subjects, valid patient, non-empty provenance) — never blocks a clinical judgment (principle 4).

3. **Group status on disagreement = latest-*effective* wins.** Once threads are linked, a later event may cease one member while another stays active. The collapsed row resolves this bitemporally (consistent with the slice-2 current-dose rule): all-active → active; all-ceased → ceased; **mixed → the member with the latest-*effective* standing statement decides** (a cessation effective last month loses to a dose-change effective yesterday). A single-thread group can never be mixed, so **slice-1/2 single-thread semantics are provably unchanged** — this rule is inert until a genuine multi-thread disagreement exists.

## Consequences

**Now guaranteed:** a duplicate flag is cleared without a fabricated cessation; the drug stays on the current list as one entry; both source threads remain intact and the link is reversible with no data loss; brand↔generic can be reconciled by human judgment ahead of the deferred Tier-A drug dictionary.

**Accepted costs / deferred:**
- **Cross-patient reconciliation** (linking two patients' threads) is a pathology the offline-first floor cannot cheaply reject (both threads' patients may be non-local). The collapse view groups per patient, so a bad edge is low-stakes, reversible, and auditable; a hard guard is deferred, tracked, not left implicit (house rule 5).
- **Group display term** is taken from the canonical (min-UUID) member — a documented approximation for brand↔generic (which name shows); refining it is deferred.
- The `cairn_event_twin` dispatch grows by two branches, feeding the filed [#173](https://github.com/cairn-ehr/cairn-ehr/issues/173) registry-refactor debt (the idiom slices 1/2 already follow).
- **Device-additive**, like slices 1/2 — but because the verbs are `additive`+`targets_other_author=FALSE`, a future human-attested reconciliation adds a responsibility-bearing contributor with **zero floor change** (identity's exact pattern).

**How we would know the bet is failing:** clinicians routinely need to reconcile threads across patients (they don't — a cross-patient duplicate is a mis-identification, handled by the identity subsystem), or the min-UUID canonical produces a clinically misleading display name often enough to demand a chosen-survivor model instead of symmetric collapse.

**Not a new founding principle.** This is principle 2 (never merge, always link) and the data-model union+reconcile rule applied to medication threads, reusing the identity linkage projection pattern verbatim.
