# 5. Identity Subsystem

> [!IMPORTANT]
> **Never merge — always link; never erase — always overlay.** Patient UUIDs are immortal;
> identity is an append-only stream of link/unlink/reattribute/identify/repudiate/dispute events
> ([§5.7](#57-identity-event-algebra-closed-set-all-append-only-syncable-auditable)). Every identity
> error — accidental or deliberate — must be repairable by an auditable event with no data loss.

## 5.1 Linkage layer — never merge, always link
- Patient UUIDs are immortal and immutable; clinical events reference their original UUID forever (sole exception: reattribution overlay, [§5.5](#55-reattribution-one-primitive-tiered-workflows)).
- Identity is an append-only stream of **link / unlink assertions** with provenance, HLC, confidence.
- The "person" (golden identity) is a projection: the connected component of the link graph. The unified chart unions the event streams of all member UUIDs.
- Consequences: merges sync trivially (events union); redundant links are idempotent; **unmerge is always possible and clean** (split the component; nothing was rewritten).

## 5.2 Matching pipeline (safety-asymmetric: false merge ≫ worse than false split)
- **Deterministic tier:** exact match on a strong identifier → auto-link with provenance.
- **Probabilistic tier:** Fellegi–Sunter-style scoring; conservative auto-link threshold; wide middle band raises a "possible duplicate" banner on both charts (surfacing safety content — allergies, active meds — without co-mingling) and queues human reconciliation.
- **Locale-pluggable comparators:** phonetic encodings, name structures, DOB precision handling, address semantics are deployment configuration, not hardcoded.
- **Where matching runs follows topology:** at registration within local scope (search-before-create); cross-facility at the lowest tier that sees both registrations (typically the hub); link events flow down through normal sync.
- **Coherence check (feedback loop):** the unified-chart projection continuously validates linked components against the [§4.2](demographics.md#42-per-field-projection-policy) conflict column. Contradictions (same-system identifier mismatch, verified-DOB clash, sex-at-birth clash) demote the link to human review and render the chart in *under-review* trust mode. Every new demographic assertion cheaply re-triggers local matching.

> [!NOTE]
> The matcher is **advisory** — it *proposes* link candidates (Python; see
> [technology.md](technology.md)). Applying the closed event algebra and maintaining its
> projections is authoritative, in-database logic. The seam between them is the database boundary
> ([language-substrate §9.3–§9.4](language-substrate.md#93-integration-boundary)).

## 5.3 Registration classes
| Class | Use | Properties |
|---|---|---|
| **Standard** | Normal registration | Search-before-create enforced funnel |
| **Unidentified** | Unconscious/unknown patient ("John Doe") | [§5.4](#54-unidentified-registration-john-doe-baked-into-the-root) |
| **Pseudonymous (sanctioned)** | Legally permitted anonymous/protective care | [§5.6](#56-pseudonymous-sanctioned-care) |

Registrations created during a partition are tagged and go to the **head of the upstream matching queue on reconnect** — post-partition reconciliation is a scheduled pipeline stage, not an error state.

## 5.4 Unidentified registration (John Doe) — baked into the root
- UUID minted immediately; care proceeds without delay.
- **System-generated callsign** (e.g. `Unknown-ED-<site>-<date>-A`), never plausible fake names; matcher excludes placeholder names from its feature space.
- Identity evidence captured as **clinician-observed assertions**: estimated age with basis, observed sex, photo, distinguishing marks, belongings, EMS pickup context — honest data, full matcher features.
- **Identity-pending is an active workflow state:** chart renders in *unconfirmed* trust mode ("no history available; allergies unknown"); matcher re-runs on every new evidence assertion.
- Resolution = **identification event** (who, method) + ordinary **link assertion** if a prior chart exists. On link during an active encounter, the system **pushes an alert**: "prior history now available — N allergies, M active medications — review now."
- Partition-safe by construction: registration and care are local; identification may occur at hub tier; the link event syncs down normally.

## 5.5 Reattribution — one primitive, tiered workflows

The **reattribution event** — "event set E belongs to UUID-B, not UUID-A" — is an immutable overlay all projections respect (digital strike-through: originals stay in place, excluded from the source chart's projection, visible in its chart-history view). It is **event-granular** (a single note, observation set, or order can move). Granularity lives in the primitive; *risk control lives in the workflow tier*:

| Tier | Use case | Conditions (enforced automatically) | Adjudication |
|---|---|---|---|
| **1 — Self-correction** | Misfiled documentation: clinician with multiple charts open saves into the wrong one (high-frequency, often ≥ weekly per clinician) | Author moves own event(s); within time window (same shift / 24 h, policy-config); destination patient in author's active care context (open/recent encounters) | None — one-click "move to correct patient," picker pre-filled with author's open charts. Full audit automatic. Friction target: < 10 seconds, or it competes with copy-paste-and-lose-provenance or with not fixing it at all |
| **2 — Supervised** | Not the author, window expired, or destination outside care relationship | Any Tier-1 condition unmet | One second sign-off (records officer / senior clinician) |
| **3 — Forensic** | Identity theft, disputes ([§5.5](#55-reattribution-one-primitive-tiered-workflows)), adversarial cases | — | Two-person rule; adjudication queue; affected events render *under-review* on both charts until resolved |

**Auto-escalation:** any event with executed real-world effects (administered medication, performed procedure, transfused product) is barred from Tier 1 and escalates with an incident-workflow flag. Reattribution records documentation truth; it must never paper over a clinical incident.

**Contamination cascade (mandatory on reattribution arrival, local or via sync):**
- Recompute decision support / alerts on both source and destination charts.
- Notify every user who **viewed or acted on** the misfiled content during the exposure window ("a note you read on patient B at 14:32 has been moved to patient A"). Generated locally on each node as the event lands → partition-safe by construction.
- **Disclosure-scope query as a named feature:** exposure window + viewer list is a single query over the append-only audit log (GDPR/HIPAA breach-scoping in seconds, not weeks).

**(a) Fabricated persona (deliberate false identity):** confession → link assertion to real chart + **repudiation events** marking false assertions. Repudiated values leave the displayed projection but enter a **known-alias pool** retained by the matcher (aliases are reused). The fact of presentation under a false name is preserved (medico-legally required).

**(b) Identity theft (events on victim's chart):** Tier-3 reattribution of the affected encounter(s). **Dispute event** as the patient/victim-initiated front door ("I was never there in March"), feeding the review queue.

## 5.6 Pseudonymous (sanctioned) care
- Covers legally permitted anonymous STI testing, protective aliases (domestic violence), staff treated at their own facility.
- Deliberately unlinked; flagged internally; later linking is **patient-initiated and consent-gated**.
- **Link assertions may carry a visibility scope; linking must never silently broaden access.** A sequestered episode joins the person's connected component (enabling e.g. interaction checking) without its contents flooding every chart view. Identity linkage and consent scoping intersect at the link event — this is an architectural invariant, not an edge case. The mechanism that makes "interaction checking without flooding the chart" concrete — the safety projection, the graded sensitivity stream, and break-glass — is [§5.9](#59-sensitivity-grade-the-safety-projection-and-break-glass-visibility-scope).

## 5.7 Identity event algebra (closed set; all append-only, syncable, auditable)
| Event | Resolves | Adjudication |
|---|---|---|
| `assert` | Registration & demographic updates | Automatic |
| `link` / `unlink` | Duplicates, John Doe identification, confessions | Auto above threshold, else human |
| `identify` | Identity-pending → confirmed | Human; method recorded |
| `repudiate` | Known-false assertions → alias pool | Human |
| `reattribute` | Misfiled documentation; wrong-chart contamination; identity theft | Tiered: self-service (author, windowed) / one sign-off / two-person rule ([§5.5](#55-reattribution-one-primitive-tiered-workflows)) |
| `dispute` | Patient-initiated review | Triage to queue |

**Chart trust states (projection-side contract):** *confirmed* / *unconfirmed* (identity-pending) / *under-review* (coherence failure, open dispute, pending reattribution). The chart always tells the clinician how much to trust the identity behind it. Responsibility-state composes into this contract: an event whose authorship is un-vouched (a non-human author with no responsibility-bearing human) renders with an explicit *unattested* marker — a form of acknowledged uncertainty (principle 4), distinct from *wrong* ([§5.10](#510-authorship-and-responsibility-state-the-consumer-side)).

**Biometrics:** excluded from core (vendor/AGPL minefield; poor offline performance on constrained hardware). Accommodated as one more identifier system in the multi-valued set via a pluggable module. The core must work with names, dates, photos, and human judgment alone.

## 5.8 Registration & documentation workflow (normative)
1. **Search-before-create enforced funnel:** "new patient" unreachable until local-scope matching has run; candidates shown with photo/age/locale/last visit; the create button records that N near-matches were displayed.
2. **Partition-aware duplicate expectation** (see [§5.3](#53-registration-classes)).
3. **Wrong-chart protection at point of care (read side):** demographic banner always shows photo + age + provenance-flagged identifiers; cheap "confirm patient" affordances emit verification assertions, raising provenance as a side effect of normal care.
4. **Wrong-chart protection at point of documentation (write side):** every input surface carries persistent patient identity (photo, name, age, per-patient color coding consistent across all open windows). Documentation is bound to an explicit **armed write-context** designed on **possession semantics** (paper precedent: you physically held one chart; the misfile is a disease of windowing, which abstracted possession away). One chart is "in hand" for writing at a time; picking it up is a single natural gesture; which chart is held is as unmissable as the color of a folder. Cross-window paste of patient-bound content is flagged at paste time. The concrete possession model, its fusion with fast authentication, and the work-salvage primitive are [§5.11](#511-point-of-care-identity-possession-fast-authentication-and-salvage).

> [!WARNING]
> **Confirmation dialogs are explicitly NOT the wrong-chart safety mechanism** — they fail the
> paper-parity test ([vision §1.2](vision.md#12-the-paper-parity-test-normative)). Restore the
> physical affordance (possession: one chart in one hand) instead.

## 5.9 Sensitivity grade, the safety projection, and break-glass (visibility scope)
> Resolves former open question §11.8 — see [ADR-0006](decisions/0006-visibility-scope-replication-and-the-safety-projection.md). Also answers the rung-1 follow-on left open by [ADR-0005](decisions/0005-erasure-key-custody-and-crypto-shredding.md).

"Scope" had been carrying three independent decisions; a sensitive episode is their collision point. Pulling them apart gives **four dials**, of which only the last two carry confidentiality:

| Dial | For a safety-relevant sensitive episode | Owned by |
|---|---|---|
| 1. **Replication** | **always on** (mandatory; set-union) | sync ([§6.4](sync.md#64-scope-is-a-prefetch-hint-not-an-authority), [ADR-0004](decisions/0004-dynamic-sync-scope-prefetch-not-authority.md)) |
| 2. **Decryptability** | gated; break-glass acquires the key | key custody ([§3.8](data-model.md#38-erasure-and-key-custody), [ADR-0005](decisions/0005-erasure-key-custody-and-crypto-shredding.md)) |
| 3. **Body visibility** | sealed until break-glass | visibility scope ([§5.6](#56-pseudonymous-sanctioned-care)) |
| 4. **Safety signal** | **always on, abstracted** | the safety projection, below |

- **Replication is never the confidentiality boundary.** A sensitive episode that carries any safety relevance **replicates unconditionally**; confidentiality lives entirely in dials 2–4, never in withholding the row. You cannot break glass on, nor warn about, content that never arrived — and a maximally-sealed episode still owes a future clinician a signal (a sealed pregnancy termination still implies **Rhesus-sensitization** the next antenatal clinician must act on). This *confirms* [ADR-0004](decisions/0004-dynamic-sync-scope-prefetch-not-authority.md): sync scope was never permitted to be an access control.
- **The safety projection** is a separate, de-identified signal emitted beside a sealed body: coarse safety **classes** (interaction/allergy class, *Rh-sensitizing event*, contraindication flags), a **severity grade**, and a pointer to the sealed event — never the agent, diagnosis, or sensitive scope keys. The classes are a **mechanical projection of the body's coded fields** (a coded drug's interaction class is a property of the code, not a confidentiality judgment), so it is a normal pre-seal projection step. It replicates **in the clear** like an allergy — it *is* the safety floor — and local decision-support fires a warning that **names nothing**: *"⚠ Grade X interaction with confidential content — break glass to view / discuss with the patient / document the decision."* This makes the [§5.6](#56-pseudonymous-sanctioned-care) promise concrete and is partition-safe (the warning is local, no key needed). Its granularity is a **policy-configured disclosure-coarsening ladder** (precise class → *"confidential medication, severity X"* → *"confidential content, break glass"*) mirroring the [§7.1](security.md#71-erasure-the-severity-ladder) erasure ladder. **Safety-floor invariant:** the sensitivity grade controls the projection's *coarseness, never its existence* — secrecy blurs the safety signal, never extinguishes it.
- **Sensitivity is a graded, multi-source, append-only assertion stream**; the **effective grade is a projection** (*never merge, always overlay*, the same shape as the link graph [§5.1](#51-linkage-layer-never-merge-always-link) and the per-field demographic projection [§4.2](demographics.md#42-per-field-projection-policy)). Because what is confidential is cultural/regional/personal, Cairn ships **only three infrastructure pieces** — a deployment-populated **category blacklist** (coded-category → default grade; whitelisting is impossibly wide), the **confidentiality grading system** itself, and **human editability** of tag/grade (patient request: divorce, family dispute; clinician judgment: domestic violence, mental health). *How they combine is policy* ([principle 9](index.md#founding-principles-the-lens-for-every-decision)): whether a blacklist auto-tag applies silently, requires clinician acceptance, or whether a deployment is manual-only, is a UI-layer policy decision Cairn makes expressible but never enforces. The grade drives the seal rung (dial 2) and the projection coarseness (dial 4). Effective grade is the **highest standing assertion**; **declassification is an authorized overlay, never an erasure** (mirroring the [§5.5](#55-reattribution-one-primitive-tiered-workflows) tiers).
- **The envelope is not automatically safe.** Plaintext scope keys (`department = sexual-health`) can be the whole disclosure, so the *semantic* scope key is **abstractable to an opaque "confidential-episode" routing token** ([§3.5](data-model.md#35-event-storage-model-hybrid-envelope)). This is self-reinforcing: opacifying the key means the sync prefetch predicate can no longer *select* on it, so replication degrades to "everything for this patient" — exactly the mandatory replication above. Identity/sync still bind on `patient_uuid` and HLC; only the human-meaningful label is generalized.
- **Break-glass is audited key-*use*** (distinct from key-*destruction*/erasure), the mirror of the [ADR-0004](decisions/0004-dynamic-sync-scope-prefetch-not-authority.md) acquisition trichotomy: key-holder present → local unseal; carried-with-patient (the patient is a key-holder, paper-parity-exact); from sibling/parent on reconnect; or, **none reachable → honest disclosure** *"sealed content exists here; the key is not present on this node"* (the warning already fired; only the specifics are unavailable — honest-assembly-state, [§6.2](sync.md#62-consistency-model)). It is an append-only audited access event ([§7](security.md)). **The architecture always provides break-glass; whether the UI offers it and what authorization it demands is policy.**

## 5.10 Authorship and responsibility-state (the consumer side)

> [!NOTE]
> Authorship the clinician cannot see is useless. Responsibility-state is surfaced in **three layers**,
> the same shape as the sensitivity / safety-projection design ([§5.9](#59-sensitivity-grade-the-safety-projection-and-break-glass-visibility-scope)).
> The model itself is [data-model §3.9](data-model.md#39-authorship-and-accountability) /
> [ADR-0007](decisions/0007-authorship-and-accountability.md).

1. **Informational floor (always).** The record honestly shows provenance and responsibility-state —
   *"AI-drafted, unattested"* vs *"attested by Dr X"*. It **never gates, blocks, or forces** anything;
   surfacing it *is* the job (principle 3 — confirmation dialogs are explicitly not a safety mechanism).

2. **Projected trust signal.** Responsibility-state feeds the existing **chart/event trust projection**
   (*confirmed / unconfirmed / under-review*, the projection-side contract above). Un-vouched AI content
   can render visually distinct, or be held out of certain auto-derived projections until vouched — still
   never a hard block. *"No human vouches for this yet"* is **acknowledged uncertainty** (principle 4):
   distinct from *wrong*, from *not-yet-reviewed*, and from *refused*.

3. **Expressible policy rung.** *"Un-vouched suppressing AI output must be attested before it takes
   effect"* is an *available* policy, never mandatory — tied to the additive-vs-suppressing distinction
   ([data-model §3.9](data-model.md#39-authorship-and-accountability)). Cairn ships the rung; the
   deployment decides (principle 9).

## 5.11 Point-of-care identity: possession, fast authentication, and salvage
> Resolves former open questions §11.9 (armed write-context) and §11.12 (authentication vs. paper-parity) — see [ADR-0008](decisions/0008-point-of-care-identity-possession-and-salvage.md).

§11.9 and §11.12 are **one problem**: the point-of-care binding of *which patient* and *which clinician* to a write. Paper bound both in a single physical situation — you, present, holding one folder, pen in hand — and both bindings were continuously, *ambiently* visible. Windowing broke the **subject** binding (the wrong-chart misfile); shared login broke the **author** binding. Restoring them is one act.

**The tension is illusory.** "Fast/proximity sessions vs. security posture" is a false trade-off, the same shape ADR-0006 found in "scope" and ADR-0007 in "signature": one word, *authentication*, carries two jobs at different frequencies — **gatekeeping** (may this person touch the system? coarse, rare, can be heavy) and **attribution** (who authored *this* event? fine, per-write, must be paper-cheap). Deployed EHRs fuse them, dragging gatekeeping cost onto every write; clinicians defeat that with shared logins — so the audit-trail collapse is *caused by* the parity violation, not traded against it. Make per-write attribution sub-second and the incentive to share evaporates: the security win comes **from** the parity win.

- **Possession binds `(clinician, patient)` in one ambient gesture.** Exactly one chart is *in hand* for writing (reading many is free). The write surface carries the patient's colour + persistent photo + name/age as the *visual environment* — ambient, peripheral, zero cognitive cost, the **opposite of a confirmation dialog** (a dialog demands a discrete act of attention and habituates to click-through; ambient display is passively absorbed). The arming gesture is **cheap in time but high in distinctiveness** — the antidote to reflexive click-through: a deliberate, spatially-specific motor act tied to *this* patient (a band-tap at the bedside, the patient's own token, paper-exact; or a drag into the single in-hand slot at a workstation). It must cost the same **cold or warm** — re-arming a patient from this morning's batch is as cheap as the patient in front of you.
- **Authentication exists; its three pains are removed.** Authentication is infrastructure and must be provided — the point is not to abolish the gate but to strip the three things clinicians actually hate, each a **corollary of an existing founding principle**, not a new axiom:
  1. **Never make the user wait if engineering can avoid it** (latency limb of paper-parity, principle 3; [vision §1.2](vision.md#12-the-paper-parity-test-normative)) — MRU-defaulted selector, type-a-few-chars-and-enter, **no spinner**, heavy work in the background while the clinician already writes, **cache-and-hide not cache-and-clear**. Instant re-auth is the *precondition* that makes presence-driven **auto-de-arm** parity-legal (auto-lock is only not a regression because the re-arm after it is free).
  2. **Always a fallback — no dead-ends, no IT dependency** (availability + paper-parity, principles 5 + 3) — a resilience ladder *badge → password → self-recovery (security-Q / SMS / recovery codes) → audited break-glass*, every rung self-service, **bottoming out in the existing partition-honest break-glass primitive** ([§5.9](#59-sensitivity-grade-the-safety-projection-and-break-glass-visibility-scope)). Recovery is break-glass for the auth layer. (The [§7.1](security.md#71-erasure-the-severity-ladder) severity-ladder motif recurring a third time.)
  3. **Never make the user redo work already done** (work-preservation — append-only, principle 1, extended to the *pre-commit* side of the commit boundary; and identity-repair, principle 2, applied to the *author*) — see salvage, below.
- **Stranded work is salvaged by identity-repair, not a wall.** Because **`session.user` and `event.author` are independently bindable** ([data-model §3.10](data-model.md#310-session-identity-event-authorship-and-draft-durability)) — the load-bearing invariant, and exactly what deployed EHRs lack — a clinician whose draft is stranded in the wrong session resolves it with a trichotomy: **sign-as** (attribute *this note* to me, authenticate me as author, session untouched — the **default**, because forcing a switch makes *two* people redo work), **switch** (explicit), or **stay**. `sign-as` rescues **your own** stranded work, requires authenticating *as* the claimed author (strictly *more* honest than today's silent-session-author save), and is logged append-only as *drafted-in-session-of-A, signed-by-B-via-sign-as*. It is the [§5](#5-identity-subsystem) repair philosophy — *prevention cannot be complete, therefore repair is first-class, cheap, fast, forensically clean* — applied to the authoring act; the backstop for the inevitable imperfection of presence-driven locking. It replaces the three bad real-world hacks (free-text `[Dr X:]`, wrong-author save, lost work).
- **Authorship is note-level.** A note is one event with a note-level contributor set ([§3.9](data-model.md#39-authorship-and-accountability), ADR-0007 unchanged); authorship of *spans within* one note is **not** modelled — it would complicate every note for a rare edge (one author types part, a second finishes and signs). In the dominant salvage case the whole note is the signer's work, so note-level is correct; the rare cross-author case keeps the cheap free-text escape hatch, and where structural truth matters there, authorship (who typed) and **attestation** (who vouches) already separate ([security §7.2](security.md#72-signing-attestation-and-ai-agent-identity)).
- **Authorship-confidence is a grade, not a gate** (acknowledged uncertainty, principle 4). Where author identity cannot be cheaply established (badge forgotten, two in range, emergency), the system never blocks — it records *attested* / *asserted* / *unattributed* (authored-at-station-X, identity unknown — never a guess) and refines by overlay, composing into the existing chart/event trust projection ([§5.7](#57-identity-event-algebra-closed-set-all-append-only-syncable-auditable) / [§5.10](#510-authorship-and-responsibility-state-the-consumer-side)) — no new stream. Passive proximity only *narrows* candidates; the explicit arming gesture *selects* — **proximity is a hint, not an authority** ([ADR-0004](decisions/0004-dynamic-sync-scope-prefetch-not-authority.md)).
- **Make contention cheap** (the software's answer to the workstation shortage, where 2–5 clinicians routinely fight for one station): collapse the per-switch tax to ~0 so a shared station approximates *N* private ones, restoring the paper desk where several clinicians each held their own folder at once. A station may hold **multiple warm, resident, hidden `(clinician, patient, draft)` contexts**, each kept alive by its owner's token and surfaced one-at-a-time by proximity. Bought by the same invariant (the context store is keyed by `(author, patient)`, not by the session); resource-bound, hence policy/hardware-gated.
- **Rhythm-agnostic** (live / after-each-patient / batch-much-later / AI-scribed / forced-retrospective are all first-class): bitemporal time absorbs them ([§3.6](data-model.md#36-bitemporal-event-time-recording-time-vs-effective-time)) and the cold = warm arming cost means a late batch is not a degraded mode.
- **Mechanism not policy (principle 9), resource-proportional (principle 4).** Cairn ships the possession primitive, proximity/token session model, ambient identity display, authorship-confidence grade, patient-bound clipboard payloads, and the `session ≠ author` + durable-draft invariants. Deployment selects token tech (NFC / BLE / phone / pluggable biometric / plain local credential), which ladder rungs exist, whether unattributed writes are permitted, the de-arm threshold, and whether `sign-as` is offered. The primitive **degrades to no special hardware** — a Pi clinic with no badges still gets the on-screen in-hand slot, a local credential, ambient display, singular arming, and no network gate; token hardware *enhances* possession, never a requirement.

> [!WARNING]
> The `(clinician, patient)` binding and the authorship stamp are **safety-critical** (a defect mis-binds the subject or mis-attributes the author) → Rust/in-database trusted surface, alongside the identity algebra ([§9](language-substrate.md)); proximity/UI (badge/BLE reading) is fit-for-purpose (a defect shows the wrong name *ambiently*, caught instantly). The seam — UI proximity event → authoritative authorship stamp — is the one safety-critical path, structurally like the [§5.9](#59-sensitivity-grade-the-safety-projection-and-break-glass-visibility-scope) seal-time projection seam.
