# Design — Medication recording (slice 1 of the `clinical.medication` surface)

**Date:** 2026-07-11 · **Branch:** `feat/medication-recording-slice-1` · **Status:** design, pending
implementation plan.

**Scope of change:** additive Rust (`cairn-event::medication`, `cairn-node::medication` + CLI) + one new
DB migration (`db/031_medication.sql`: floor + projection). Opens the medication surface under the
`clinical.*` namespace. **No new founding principle, no new envelope field, no ADR** — this graduates
already-decided spec prose (§3.15 active-write, §3.16 substance/ICD-11 discipline, data-model line 28
"union + flagged for clinician reconciliation") into product code.

---

## 1. What this is, and why now

The **first clinical-content event stream** on `cairn-node`. Everything built so far is demographics +
identity + the matcher + attachments; even the progress note (§3.19 / ADR-0041) is specced-but-unbuilt.
This slice records **current and past medication for a patient** — the clinical spine that prescriptions
(a later slice) hang off.

It is deliberately built **without any Tier-A drug-reference dictionary** (the INN/RxNorm/PBS sourcing
surveyed in [ecosystem eval 0003](../../ecosystem/0003-reference-data-sourcing-medicines-and-terminologies.md)).
A bare node records a medication as a self-contained structured value and is a complete EHR without a
bundled drug database — exactly the §3.16 / principle-12 posture. Tier A, when it arrives, attaches as
**overlay enrichment** (coding a previously-uncoded substance), never a wire change.

**Blast radius → substrate ([§9](../../spec/language-substrate.md)).** The write path can silently
corrupt the clinical record, so it is **safety-critical**: pure Rust builders + an **in-DB floor**
(validated submit + constraints) + a projection. The reconciliation *flag* (§6) is advisory and lives in
the recomputable projection. Structure mirrors the demographics slice exactly.

## 2. Event vocabulary — two verbs, a thread-UUID backbone

Each medication on the list is a **thread with an immortal UUID** (`medication_id`), reusing the identity
event-algebra shape (a stable id that events overlay, never mutate — principle 2):

| Event type | Speech act | Body carries |
|---|---|---|
| `clinical.medication.asserted` | mints the thread | the statement (patient takes / took X) |
| `clinical.medication-cessation.asserted` | references the thread | the stop → thread becomes *past* |

- A cessation is **append-only overlay**, never a mutation of the assert.
- The constant `.asserted` **speech-act suffix** is preserved (as in `demographic.field.asserted`,
  `identity.link.asserted`) rather than a bespoke `.ceased` — the noun slot carries the verb
  (`medication` vs `medication-cessation`). Each type gets its own `schema_version`
  (`clinical.medication/1`, `clinical.medication-cessation/1`), consistent with existing per-type
  versioning.
- Later slices add verbs that also reference `medication_id` (dose-correction overlay, reconciliation
  resolution) — out of scope here (§8).

## 3. The signed body (day-one shape)

The substance reference rides the **signed event**, so its shape is the can't-cheaply-retrofit piece
(same lesson as the attachment-reference shape, ADR-0042). Fixed day-one:

```
clinical.medication.asserted:
  medication_id : UUID                       # the thread — minted here
  patient       : UUID
  substance:
    term        : string   (MANDATORY, non-empty)    # as asserted; may be vague ("little white pill")
    inn_code    : <INN id> | null                     # the stable anchor slot — NULLABLE = not-yet-coded
    formulation : Formulation | unknown               # enum (tablet, capsule, liquid, patch, ...) or honest-unknown
  dose          : { amount: decimal, unit: DoseUnit } | unknown
  sig           : string | unknown                    # free-text directions ("one BD", "PRN")
  info_source   : patient-reported | clinician-observed | external-record | unknown
  started       : <uncertainty-capable date> | unknown    # §3.6 t_effective — when they began taking it

clinical.medication-cessation.asserted:
  medication_id : UUID                        # references the thread
  patient       : UUID
  stopped       : <uncertainty-capable date> | unknown    # t_effective of the stop
  reason        : string | null                # optional
```

**The uncertainty floor (principle 4, §3.7).** The **only** mandatory clinical field is
`substance.term` (non-empty). Everything else — substance identity precision, dose, formulation,
frequency, start date — is an **explicit *unknown***, distinct from *not-yet-asked* and *refused*. A
statement with genuinely nothing in it is meaningless and is rejected at the floor; everything above
`term` is optional-with-honest-unknown. This makes the realistic ED collateral history ("some blood
thinner, unsure which, dose unknown") a first-class recordable write, never a blocked one.

**`DoseUnit`** is a **small controlled enum** (`mg, mcg, g, mL, units, puffs, drops, %`) **+ a free-text
escape hatch** for the long tail, uncertainty-capable — a units confusion (mg vs mL) is a real
medication-error class, so units are not bare free-text.

**`info_source`** is the provenance of the *clinical claim* (who said it), **distinct from the event's
contributor set / attestation** (who authored and vouches for the event, §3.9). An unverified
patient-reported list reads very differently from a reconciled one; strong precedent in the demographics
evidence slices.

**`started` / `stopped`** reuse the **existing uncertainty-capable date representation** established by
the demographics DOB-range work (§3.6/§3.7) — no bespoke date type is invented. Exact encoding is a
plan-level detail matching what `cairn-event` already does for DOB.

**Legibility twin (§3.13).** Every event carries the mandatory, mechanically-derived plaintext twin —
e.g. *"Atorvastatin 40 mg tablet, one BD — patient-reported, started ~2024"* — the co-produced note line
of §3.15. There is exactly one event; the prose is a rendering *of* it and cannot diverge.

## 4. The in-DB floor (the safety-critical seam)

Maximally permissive on clinical content (principle 4 / paper-parity — **never block a medication
write**), strict on structural integrity:

- **assert:** `substance.term` non-empty; well-formed `patient` ref; append-only. Nothing else forced.
- **cessation:** well-formed `medication_id` + `patient`; append-only.
- **Duplicates are allowed.** Asserting the same medication twice is a valid write (two statements *do*
  exist); the floor never rejects it. Duplicate *detection* is the projection's advisory job (§6).

**Deliberate offline-first decision.** The cessation floor **does not require the `asserted` event to be
present locally.** In set-union sync a cessation can legitimately be authored before its assert has
replicated, or the assert may live on another node. Requiring local presence would break the
availability floor (AP, [ADR-0001](../../spec/decisions/0001-fat-postgres-thin-daemon.md)). The floor
validates only a well-formed thread reference; the projection resolves an orphan cessation honestly when
the assert arrives. *(Mirrors the identify→link slice, which deliberately skipped cross-existence
pre-checks for the same reason.)*

## 5. The projection — `patient_medication_current` (+ past)

Fold `event_log` by `medication_id`:

- thread with an active assert and **no** cessation → **current**;
- thread with a cessation → **past**.

**Union across all sources** — multiple clinicians' independent statements coexist; nothing is dropped
(the safety property). **Orphan cessation** (a cessation whose assert is not yet local): the projection
is **assert-driven**, so it contributes **no renderable row** until its assert arrives (there is no
substance to display) — but the cessation event is **retained in `event_log`, never dropped**, and the
thread surfaces directly in *past* once the assert replicates. No data loss, no dishonest placeholder.
The projection surfaces each current thread's **start/assert date** so staleness
is *visible* (decision B): a med asserted years ago with no cessation shows as current but with its age
on display — the honest treatment of the stale-paper-med-list problem. "Current" is a **projection
convenience, not a truth claim** (principle 4); active review / last-confirmed is deferred (§8).

Winner/ordering tiebreaks over TEXT keys use `COLLATE "C"` per
[ADR-0045](../../spec/decisions/0045-collation-independent-projection-tiebreaks.md) (set-union
convergence — two nodes must pick the same display winner).

## 6. Reconciliation flagging (E1 — the honest subset)

The spec commits to "union + **flagged for clinician reconciliation** on conflict" (data-model line 28).
Full duplicate detection is the drug-name equivalent of the demographics matcher (brand↔generic, typos,
salts) and needs substance identity we've deferred — building it now would be a mini-matcher with no
dictionary, throwing false conflicts and missing brand/generic. So slice 1 ships the **deterministic
subset**:

- **Flag = projection-computed, advisory.** Group *active* threads by
  `coalesce(inn_code, normalize(term))`; **≥2 active threads sharing that key → flagged as a
  reconciliation candidate** (`GROUP BY … HAVING count > 1`; `normalize` = case/whitespace fold). No
  fuzzy matching.
- **Catches:** the same coded substance asserted twice (any dose — "which atorvastatin is current?");
  the exact-same name typed by two clinicians. The unambiguous duplicates.
- **Deliberately misses (and the design says so):** brand↔generic, typos, salt/ester variants →
  **honestly deferred to the real drug-matcher + Tier-A dictionary**. Exact-normalized matching means
  **no false positives from unrelated drugs**; the one accepted case is two vague `"little white pill"`
  statements flagged as a candidate, which is a reasonable reconciliation prompt.
- **Advisory-only, never auto-merges** (principle 2: never merge, always link). The flag is a worklist
  signal, not an action.
- **Resolution needs no new event type.** The clinician clears a flag by **ceasing the redundant
  thread** (the verb we are already building) → it drops out of the *active* set → the flag clears.
- **The floor is untouched** — flagging is purely a recomputable projection concern.

## 7. Structure, substrate, and testing

Mirrors the demographics slice:

- **`crates/cairn-event/src/medication.rs`** — pure `build_*_body` builders (assert, cessation) with
  explicit inputs/outputs; unit-tested with no DB.
- **`db/031_medication.sql`** — the validated submit-floor additions + the `patient_medication_current`
  (and past) projection + the reconciliation-flag view. In-DB, reviewer-legible SQL/PL-pgSQL.
- **`crates/cairn-node/src/medication.rs`** — the async orchestrator (author an assert; author a
  cessation) + a `medication` CLI (`assert` / `cease` subcommands).
- **Tests:** TDD, failing-test-first. Pure builder tests (no DB) + **DB-gated** integration tests
  (assert appears as current; cessation flips current→past; duplicate active threads raise the flag;
  ceasing one clears it; orphan cessation is accepted and resolves on assert arrival; the `term`-empty
  reject; a positive control). Crypto material in tests is **runtime-derived**, never literal (house
  rule 6 / issue #146).

## 8. Explicitly out of scope for slice 1

Deferred, each its own later slice or the Tier-A tier:

- **Dose-correction / change-of-dose** overlay (a third verb referencing `medication_id`).
- **Fuzzy reconciliation** (brand↔generic, typos, salts) — waits on the drug-matcher + Tier-A dictionary.
- **Reconciliation *resolution* as a first-class event** (slice 1 resolves via cessation).
- **`delete` as a rendering-suppression visibility overlay** (§3.15) — the med stays in the record; the
  suppression is a separate later surface.
- **Structured sig / frequency** (dose-route-frequency-timing) — lands with the **prescription** slice,
  where it is medico-legally load-bearing.
- **Active review / "last-confirmed" staleness resolution** — slice 1 only *shows* staleness.
- **Route** as a separate field — formulation carries enough for slice 1.
- **Tier-A**: INN/RxNorm/PBS ingest, autocomplete, DDI checking, ICD-11/ATC linkage.

## 9. Design tensions recorded (resolved)

- **(B) "current" is stale-prone** → accepted; staleness made *visible* via the assert date, active
  review deferred.
- **(D) units are a med-error class** → `DoseUnit` enum + free-text escape hatch, not bare free-text.
- **(E) union duplicates the list** → E1 deterministic advisory flag shipped now; fuzzy detection
  deferred, honestly.
