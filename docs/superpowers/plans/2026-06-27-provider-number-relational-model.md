# Provider-number relational model (demographics gap B remainder) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Specify the provider-number person×location/org relational model — an abstract identity-bearing **entity**, reified **relationships** carrying their own identifier sets, and **subject-kind partitioning** making ADR-0033 non-conflation structural — as demographics §4.6 and ADR-0035; bump the spec to 0.36.

**Architecture:** Pure spec-prose change across Markdown files — **no code**. The "test cycle" per task is the **mkdocs build** (broken cross-references surface as warnings) plus targeted `grep` anchor checks. Mirrors the just-merged ADR-0032/0033/0034 demographics changes in structure and tone.

**Tech Stack:** Markdown; mkdocs-material. Build command (from CLAUDE.md):
`uv run --with mkdocs-material --with mkdocs-callouts --with mkdocs-redirects -- mkdocs build`

## Global Constraints

- **No in-file changelogs, no version suffixes** — git is the line history; spec version lives only in `index.md`.
- **Author callouts in GitHub/Obsidian syntax** (`> [!NOTE]`) so they render on GitHub and as Material admonitions.
- **Never commit the generated `site/`** (gitignored).
- **ADRs are immutable once accepted** — ADR-0035 is new; do not edit existing ADRs.
- **Terminology guard:** "canonical identifier" = ADR-0031's UUIDv7+multihash; an external/billing identifier uses **"normalized form"**, never "canonical".
- **Non-conflation invariant:** a billing/entity identifier must never be a patient match key or a signing credential; the same value may recur across partitions (the WorkCover AHPRA case) — the invariant keys on **position**, not value.
- **Branch:** `provider-number-relational-model` (already created; design doc already committed there).
- Source-of-truth design: [docs/superpowers/specs/2026-06-27-provider-number-relational-model-design.md](../specs/2026-06-27-provider-number-relational-model-design.md).

---

### Task 1: ADR-0035 — the decision record (the *why*)

**Files:**
- Create: `docs/spec/decisions/0035-entities-relationships-and-provider-numbers.md`

**Interfaces:**
- Produces: the ADR file later tasks cross-reference as `[ADR-0035](decisions/0035-entities-relationships-and-provider-numbers.md)` (from spec root) / `[ADR-0035](0035-entities-relationships-and-provider-numbers.md)` (from within `decisions/`).

- [ ] **Step 1: Write the ADR file** with this exact content:

```markdown
# ADR-0035 — Entities, relationships, and the provider-number relational model

- **Status:** Accepted
- **Date:** 2026-06-27
- **Refines:** [ADR-0033](0033-patient-identifier-representation.md), [ADR-0014](0014-locale-pluggable-matcher-comparators.md)

## Context

[ADR-0033](0033-patient-identifier-representation.md) / [§4.4](../demographics.md#44-identifiers-representation) settled patient-identifier *representation* and drew the patient-vs-professional boundary, but **deferred** one piece it named: a billing/provider number **is not a per-person fact.** In the systems the maintainer has worked in (AU Medicare the sharpest case) a provider number is issued per **(practitioner × practice-location)** — one clinician carries a *different* number at each site they bill from, a third at the hospital. Neither §4.4 (subject = the patient) nor [§7.5](../security.md#75-the-actor-registry-enrollment-version-pinning-and-key-custody) (subject = the clinician-as-actor, a per-person identity) has a home for an identifier that is an attribute of an **edge** between two parties.

Billing is unsystematic *within* a single country, let alone across them: some payers scope a number to a physical location, some to a billing organisation, some to a contract. Hard-coding "location" or "org" as the relational anchor would repeat the cultural-capture mistake [§4.3](../demographics.md#43-address-the-three-facet-value)/§4.4 avoided for addresses and identifiers. The entity/edge **shape is a can't-retrofit, day-one decision** (as with [ADR-0013](0013-attachments-content-addressed-lazy-blob-tier.md)/[ADR-0032](0032-culture-neutral-address-representation.md)), so it is fixed now.

## Decision

Demographics gains a **generalised identifier subject**: the §4.4 identifier-set machinery (`value/system/normalized/profile/use·type`, set-union, append-only, provenance ladder, hard-veto matching) is lifted from *subject = patient* to **subject = any entity**, plus a reified **relationship** subject. Homed as [§4.6](../demographics.md#46-entities-relationships-and-provider-identifiers).

1. **Entity** — a first-class append-only entity, canonical-identified ([ADR-0031](0031-canonical-identifiers-and-node-local-surrogate-keys.md)) like a patient/actor, asserted through [§4.1](../demographics.md#41-demographic-assertions). It carries an open **`kind`** (`person`, `organization`, `location`, …), a 0:many §4.4 identifier set, the mandatory [§4.5](../demographics.md#45-the-demographic-legibility-twin) legibility twin, and an optional [§4.3](../demographics.md#43-address-the-three-facet-value) address facet. A person-entity who also signs carries an optional **one-way `actor_ref`** to its [§7.5](../security.md#75-the-actor-registry-enrollment-version-pinning-and-key-custody) actor-UUID — pointing entity→actor only, conferring **no** signing authority, absent for orgs/locations/suppliers.

2. **Relationship** — a reified, directional, typed edge between two entities with its **own** canonical identity, asserted append-only: `from`/`to` (entity-UUIDs) + an open **`type`** (`practices-at`, `bills-under`, `employed-by`, `supplies`, …) + its **own** 0:many §4.4 identifier set. **This is where the AU Medicare provider number lives** — a `medicare-provider` identifier on the (person `practices-at` location) edge; a payer that scopes per-billing-entity uses person×organisation, supply uses org×org. Validity is asserted/superseded/ended through the [§5.7](../identity.md#57-identity-event-algebra-closed-set-all-append-only-syncable-auditable)/§7.5 append-only algebra shape (`assert/supersede/revoke/end`), **never overwritten**; because the edge is reified, ending one affiliation never touches another, and the [§7.7](../security.md#77-federation-admission-peering-trust-anchors-and-the-custodian-contract) revocation cascade scopes to exactly that edge.

3. **Subject-kind partitioning — structural non-conflation.** Every identifier set is tagged `(subject_kind, subject_uuid)`, `subject_kind ∈ {patient, entity, relationship}`. **Matching only ever compares identifiers within one partition**: the [identity §5.2](../identity.md#52-matching-pipeline-safety-asymmetric-false-merge-worse-than-false-split) patient-match pipeline pulls `patient` and can never see a billing number; the §4.4 hard veto applies within a partition only. A billing/entity identifier never authorizes signing (only §7.5 actor keys sign; `actor_ref` is one-way). So [ADR-0033](0033-patient-identifier-representation.md)'s *stated* non-conflation becomes *enforced* — a billing number cannot become a patient match key (wrong partition) nor a signing credential (only actors sign), **by construction**.

4. **Position, not value.** The invariant keys on **where** an identifier sits, never on its value. The **same** AHPRA registration string may validly appear at once as a licensure/authority credential on the §7.5 actor **and** as a §4.6 billing identifier (a WorkCover case requiring the treating clinician's AHPRA cited on the claim). That is **not** conflation — one partition slot serving two roles would be; the same value across two partitions for two purposes is exactly what the model must allow. *Who signs* stays distinct from *who/how billed*, even when the number is identical.

**Matching, validation, floor — reuse §4.4 within a partition.** Hard veto = same `system`/different `normalized`, forces a human decision, honest degradation when the normalizer profile is absent, `system: unknown` never vetoes; validators flag-not-reject ([principle 4](../index.md#founding-principles-the-lens-for-every-decision)). The culture-neutral in-DB floor adds only: an entity has a text `kind` + non-empty twin; a relationship names two existing entity-UUIDs + a `type` + non-empty twin; **the `subject_kind` tag is present and immutable** (cross-partition comparison structurally impossible). The floor never holds a profile, runs a checksum, validates a billing format, or branches on an unknown `kind`/`type` ([principle 12](../index.md#founding-principles-the-lens-for-every-decision)).

**Safety class** ([§9](../language-substrate.md)): the entity/relationship *data* is **fit-for-purpose** (a wrong provider number → a rejected claim, caught immediately — it does not corrupt the record, mis-merge a patient, or leak). The **`subject_kind` partition, the one-way `actor_ref`, and the append-only floor invariants** are **safety-critical** (in-DB/Rust) — they are what prevent conflation.

## Consequences

- **Easier:** any billing-number scoping (person×location, person×org, org×org supply) is expressible with **no schema migration** (kind + type + system are data); the same entity registry serves suppliers/contracts; per-edge revocation is exact; signing and billing are structurally separate.
- **Harder / the bet:** a reified-relationship + entity registry is more moving parts than a flat per-person field, and (as in [ADR-0014](0014-locale-pluggable-matcher-comparators.md)) we bet content-addressed profiles distribute reliably off the clinical plane so cross-node matching of provider/location identifiers degrades honestly rather than mis-firing.
- **How we'd know the bet fails:** a billing/entity identifier is observed reaching the patient-match pipeline, or an `actor_ref` is observed granting signing authority (either is a partition-leak correctness bug — the structural guarantee was violated); or relationship-identifier matching produces same-system holds at a rate that swamps human review where profiles have not propagated.
- **No new founding principle** — an application of principles 1/2/4/11/12 reusing ADR-0014's profile machinery and the §5.7/§7.5 algebra; the new `subject_kind` tag makes an existing invariant *enforced*, not a new architectural axis.
```

- [ ] **Step 2: Build to verify all cross-reference links resolve**

Run: `uv run --with mkdocs-material --with mkdocs-callouts --with mkdocs-redirects -- mkdocs build 2>&1 | grep -i "warn\|error" || echo "CLEAN"`
Expected: `CLEAN` (no broken-link warnings referencing the new ADR or its anchors).

- [ ] **Step 3: Commit**

```bash
git add docs/spec/decisions/0035-entities-relationships-and-provider-numbers.md
git commit -m "spec(adr): ADR-0035 entities, relationships & provider-number model

Generalises the §4.4 identifier machinery from subject=patient to
subject=any entity, plus reified relationships carrying their own
identifier sets (the AU Medicare provider number). Subject-kind
partitioning makes ADR-0033 non-conflation structural; one-way
non-authorizing actor_ref keeps signing distinct from billing;
same-value-across-partitions (WorkCover AHPRA) allowed by keying on
position not value. Refines ADR-0033 and ADR-0014.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 2: demographics §4.6 + update the §4.4 boundary paragraph

**Files:**
- Modify: `docs/spec/demographics.md:66` (the §4.4 "provider number is relational … deferred" sentence)
- Modify: `docs/spec/demographics.md` (append new §4.6 after §4.5, currently ending at line 80)

**Interfaces:**
- Consumes: ADR-0035 (Task 1) at `[ADR-0035](decisions/0035-entities-relationships-and-provider-numbers.md)`.
- Produces: anchor `#46-entities-relationships-and-provider-identifiers` referenced by Tasks 3 and 4.

- [ ] **Step 1: Update the §4.4 boundary sentence** — replace the final clause of the line at `demographics.md:66`. Find:

```markdown
A **provider number is relational** (different per practice/location → scoped to person×org); that model is **deferred** ([ADR-0033](decisions/0033-patient-identifier-representation.md)).
```
Replace with:
```markdown
A **provider number is relational** (different per practice/location → scoped to person×org); that model is specified in [§4.6](#46-entities-relationships-and-provider-identifiers) ([ADR-0035](decisions/0035-entities-relationships-and-provider-numbers.md)).
```

- [ ] **Step 2: Append the new §4.6 section** at the end of `demographics.md` (after the §4.5 block ending at line 80):

```markdown

## 4.6 Entities, relationships, and provider identifiers

[§4.4](#44-identifiers-representation) settled **patient**-identifier representation and noted that a **provider number is relational** — different per practice/location — and deferred that model. This section closes it, and in doing so **generalises the §4.4 identifier machinery from *subject = patient* to *subject = any entity***, plus a reified **relationship** subject ([ADR-0035](decisions/0035-entities-relationships-and-provider-numbers.md), [principle 12](index.md#founding-principles-the-lens-for-every-decision)). The provider-number person×location/org case is the worked example; the same shape serves orgs, locations, suppliers, and contracts.

**The entity.** A first-class append-only **entity** — canonical-identified (UUIDv7 + multihash, [ADR-0031](decisions/0031-canonical-identifiers-and-node-local-surrogate-keys.md)) like a patient or actor, asserted through [§4.1](#41-demographic-assertions). It carries:

- **`kind`** — a **recommended-but-open** vocabulary (`person`, `organization`, `location`, …); the core never branches on an unknown kind beyond partitioning.
- a 0:many **identifier set** — the [§4.4](#44-identifiers-representation) facets verbatim (`value` / `system` / `normalized` / `profile` / `use·type`), set-union, append-only, provenance ladder. A person-entity carries provider IDs; a location facility/site IDs; an organisation ABN/registration IDs.
- the mandatory [§4.5](#45-the-demographic-legibility-twin) **legibility twin**; an optional [§4.3](#43-address-the-three-facet-value) **address** facet (esp. for location/organisation entities).
- for a person-entity who also signs clinical events, an optional **one-way `actor_ref`** to its [§7.5](security.md#75-the-actor-registry-enrollment-version-pinning-and-key-custody) actor-UUID — entity→actor only, conferring **no** signing authority, absent for suppliers/orgs/locations. A wrong `actor_ref` is repaired by an append-only overlay ([principle 2](index.md#founding-principles-the-lens-for-every-decision)), never erased.

**The relationship (where billing numbers live).** A **relationship** is a reified, directional, typed edge between two entities with its **own** canonical identity, asserted append-only: **`from`/`to`** (entity-UUIDs) + a recommended-but-open **`type`** (`practices-at`, `bills-under`, `employed-by`, `supplies`, …) + its **own** 0:many [§4.4](#44-identifiers-representation) identifier set. The **AU Medicare provider number** is a `medicare-provider` identifier on the (person `practices-at` location) edge; a payer that scopes per-billing-entity uses person×organisation, supply uses org×org — the billing mess is absorbed because the edge terminates at *any* entity. Validity is asserted/superseded/ended through the [§5.7](identity.md#57-identity-event-algebra-closed-set-all-append-only-syncable-auditable)/[§7.5](security.md#75-the-actor-registry-enrollment-version-pinning-and-key-custody) append-only algebra shape (`assert`/`supersede`/`revoke`/`end`), **never overwritten**: a locum leaving Clinic A ends *that* relationship, Clinic B untouched, and the [§7.7](security.md#77-federation-admission-peering-trust-anchors-and-the-custodian-contract) revocation cascade scopes to exactly that edge.

**Subject-kind partitioning (the non-conflation guarantee).** Every identifier set is tagged `(subject_kind, subject_uuid)` with `subject_kind ∈ {patient, entity, relationship}` (patient = the [§4.4](#44-identifiers-representation) sets). **Matching only ever compares identifiers within one partition** — the [identity §5.2](identity.md#52-matching-pipeline-safety-asymmetric-false-merge-worse-than-false-split) patient-match pipeline pulls `patient` and **can never see** a billing number; provider de-duplication pulls `entity, kind=person`; the §4.4 hard veto applies within a partition only. A billing/entity identifier **never authorizes signing** (only [§7.5](security.md#75-the-actor-registry-enrollment-version-pinning-and-key-custody) actor keys sign; `actor_ref` is one-way). So [ADR-0033](decisions/0033-patient-identifier-representation.md)'s non-conflation becomes **structural**: a billing number cannot act as a patient match key (wrong partition) nor a signing credential (only actors sign), by construction rather than convention.

**Position, not value.** The partition keys on **where** an identifier sits, never on its value. The **same** AHPRA registration string may validly appear at once as a licensure/authority credential on the [§7.5](security.md#75-the-actor-registry-enrollment-version-pinning-and-key-custody) actor (the *who-may-sign* facet) **and** as a §4.6 billing identifier — a WorkCover insurance case that *requires* the treating clinician's AHPRA cited on the claim. This is **not** conflation: one partition slot serving two roles would be; the same value across two partitions for two purposes is exactly what the model allows. *Who signs* (actor credential) stays distinct from *who/how billed* (entity·relationship identifier), even when the number is identical.

**Matching, validation, floor — reuse §4.4 within a partition.** Identifier matching reuses [§4.4](#44-identifiers-representation) verbatim (hard veto = same `system`/different `normalized`, forces a human decision, honest degradation when the profile is absent, `system: unknown` never vetoes); validators flag-not-reject. Every entity/relationship assertion carries the mandatory [§4.5](#45-the-demographic-legibility-twin) twin — *"Organization: Top End Medical Clinic; ABN 12 345 678 901"*, *"Relationship (practices-at): Dr Smith → Top End Medical Clinic; Medicare provider 2426789A, from 2024-03."* The culture-neutral in-DB **floor** enforces only structural invariants — an entity carries a text `kind` + non-empty twin; an identifier element obeys the §4.4 rules; a relationship names two **existing** entity-UUIDs + a `type` + a non-empty twin; **the `subject_kind` tag is present and immutable** (cross-partition comparison structurally impossible) — and **never holds a profile, runs a checksum, validates a billing format, branches on an unknown `kind`/`type`, or rejects on validation** ([principle 12](index.md#founding-principles-the-lens-for-every-decision)). The entity/relationship *data* is fit-for-purpose; the **partition tag, one-way `actor_ref`, and floor invariants** are safety-critical ([§9](language-substrate.md)) — they are what prevent conflation.
```

- [ ] **Step 3: Build to verify links**

Run: `uv run --with mkdocs-material --with mkdocs-callouts --with mkdocs-redirects -- mkdocs build 2>&1 | grep -i "warn\|error" || echo "CLEAN"`
Expected: `CLEAN`.

- [ ] **Step 4: Verify the new anchor and the updated boundary cross-ref resolve**

Run: `grep -n "## 4.6 Entities, relationships, and provider identifiers" docs/spec/demographics.md && grep -c "46-entities-relationships-and-provider-identifiers" docs/spec/demographics.md`
Expected: the heading prints; count ≥ 1 (the §4.4 boundary line at `demographics.md:66` now references the new anchor).

- [ ] **Step 5: Commit**

```bash
git add docs/spec/demographics.md
git commit -m "spec(demographics): §4.6 entities, relationships & provider identifiers

Generalises §4.4 identifier sets to subject=any entity (open kind:
person/organization/location/…) + reified relationships carrying their
own identifier sets (the AU Medicare provider number). Subject-kind
partitioning ({patient,entity,relationship}) as structural non-conflation;
one-way non-authorizing actor_ref; position-not-value (WorkCover AHPRA).
§4.4 boundary sentence now points to §4.6 (no longer deferred). ADR-0035.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 3: identity §5.2 — subject-kind partition cross-ref

**Files:**
- Modify: `docs/spec/identity.md:22` (the §5.2 coherence-check bullet — already references §4.4)

**Interfaces:**
- Consumes: the demographics `#46-entities-relationships-and-provider-identifiers` anchor (Task 2).

- [ ] **Step 1: Append the partition note** — the line at `identity.md:22` currently ends with "…holds for human review rather than declaring a mismatch from formatting noise." Append one sentence to the **end** of that bullet:

Add (appended to the existing line, after the final period):
```markdown
 Matching is **subject-kind-partitioned** ([§4.6](demographics.md#46-entities-relationships-and-provider-identifiers)): this pipeline compares only patient-subject identifiers, so a provider/billing number (an entity- or relationship-subject identifier) can never act as a patient match key.
```

- [ ] **Step 2: Build to verify links**

Run: `uv run --with mkdocs-material --with mkdocs-callouts --with mkdocs-redirects -- mkdocs build 2>&1 | grep -i "warn\|error" || echo "CLEAN"`
Expected: `CLEAN`.

- [ ] **Step 3: Commit**

```bash
git add docs/spec/identity.md
git commit -m "spec(identity): §5.2 note matching is subject-kind-partitioned

The patient-match pipeline compares only patient-subject identifiers, so
a provider/billing number can never act as a patient match key (ADR-0035 §4.6).

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 4: security §7.5 + §7.7 cross-refs (position-not-value; per-edge cascade)

**Files:**
- Modify: `docs/spec/security.md` (the §7.5 "Key custody, un-conflated" bullet at line 80 — append a sentence)
- Modify: `docs/spec/security.md:122` (the §7.7 controlling-entity cascade bullet — append a parenthetical)

**Interfaces:**
- Consumes: the demographics `#46-entities-relationships-and-provider-identifiers` anchor (Task 2).

- [ ] **Step 1: Append the §7.5 cross-ref** — at the **end** of the "Key custody, un-conflated — opposite lifecycles." bullet (line 80), append:

```markdown
 A clinician's **billing/relational provider identifiers** (Medicare provider numbers, payer IDs) are **not** actor credentials and live in [demographics §4.6](demographics.md#46-entities-relationships-and-provider-identifiers) on the entity/relationship, linked back by a one-way `actor_ref`; the *same* registration value (e.g. AHPRA) may appear both here as a licensure credential and there as a billing identifier — **position, not value, decides the role** ([ADR-0035](decisions/0035-entities-relationships-and-provider-numbers.md)).
```

- [ ] **Step 2: Append the §7.7 per-edge-scope parenthetical** — at the **end** of the "Cascade over the issuance/affiliation graph" bullet (line 122), append:

```markdown
 A [demographics §4.6](demographics.md#46-entities-relationships-and-provider-identifiers) provider-at-location relationship is reified, so revoking one affiliation (and its billing identifiers) scopes to exactly that edge — a clinician suspended at one site keeps the others.
```

- [ ] **Step 3: Build to verify links**

Run: `uv run --with mkdocs-material --with mkdocs-callouts --with mkdocs-redirects -- mkdocs build 2>&1 | grep -i "warn\|error" || echo "CLEAN"`
Expected: `CLEAN`.

- [ ] **Step 4: Commit**

```bash
git add docs/spec/security.md
git commit -m "spec(security): §7.5/§7.7 cross-ref the §4.6 entity model

§7.5: billing/relational provider IDs live in §4.6 linked by one-way
actor_ref; position-not-value decides credential-vs-billing role.
§7.7: the reified provider-at-location edge scopes per-edge revocation.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 5: spec version bump + ADR index row + mkdocs nav

**Files:**
- Modify: `docs/spec/index.md:9` (spec version)
- Modify: `docs/spec/decisions/README.md:57` (append ADR-0035 row after the 0034 row)
- Modify: `mkdocs.yml:132` (append ADR-0035 nav entry after the 0034 entry)

**Interfaces:**
- Consumes: ADR-0035 (Task 1).

- [ ] **Step 1: Bump the spec version** — replace at `index.md:9`:

Old:
```markdown
**Spec version:** 0.35 · **License target:** AGPL-3.0 (all components AGPL-3.0-compatible).
```
New:
```markdown
**Spec version:** 0.36 · **License target:** AGPL-3.0 (all components AGPL-3.0-compatible).
```

- [ ] **Step 2: Add the ADR-0035 index row** — insert immediately after the line at `decisions/README.md:57` (the 0034 row):

```markdown
| [0035](0035-entities-relationships-and-provider-numbers.md) | Entities, relationships, and the provider-number relational model | Accepted (refines 0033, 0014) | 2026-06-27 |
```

- [ ] **Step 3: Add the mkdocs nav entry** — insert immediately after the line at `mkdocs.yml:132` (the ADR-0034 entry):

```yaml
      - ADR-0035 · Entities, relationships & the provider-number relational model: spec/decisions/0035-entities-relationships-and-provider-numbers.md
```

- [ ] **Step 4: Build to verify links + nav**

Run: `uv run --with mkdocs-material --with mkdocs-callouts --with mkdocs-redirects -- mkdocs build 2>&1 | grep -i "warn\|error" || echo "CLEAN"`
Expected: `CLEAN`.

- [ ] **Step 5: Verify version + row + nav**

Run: `grep -n "Spec version:.*0.36" docs/spec/index.md && grep -n "0035-entities-relationships-and-provider-numbers" docs/spec/decisions/README.md mkdocs.yml`
Expected: version prints one match; the ADR slug prints in both README.md and mkdocs.yml.

- [ ] **Step 6: Commit**

```bash
git add docs/spec/index.md docs/spec/decisions/README.md mkdocs.yml
git commit -m "spec: bump 0.35 -> 0.36 (ADR-0035, demographics §4.6); index + nav

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

### Task 6: HANDOVER + ROADMAP currency

**Files:**
- Modify: `docs/HANDOVER.md` (header version, session summary, gap-B open-thread bullet, ADR index table)
- Modify: `docs/ROADMAP.md` (Phase 4 demographics line)

**Interfaces:**
- Consumes: all prior tasks (this records them).

- [ ] **Step 1: Update HANDOVER.md** — set the header to spec **v0.36 (+ADR-0035)**; replace the "This session" summary with a concise paragraph covering ADR-0035 / demographics §4.6 (abstract entity with open kind; reified relationships carrying their own identifier sets — the AU Medicare provider number; subject-kind partitioning {patient,entity,relationship} as structural non-conflation; one-way non-authorizing `actor_ref`; position-not-value / WorkCover AHPRA; fit-for-purpose data vs safety-critical partition); **close demographics gap B** (its representation half closed earlier via ADR-0033, the relational remainder now closed) and remove the "gap B remainder / provider-number person×org" follow-on from the open-thread menu; add the ADR-0035 row to the HANDOVER ADR index table. Keep under 500 lines (prune the oldest prior-session paragraph if needed).

- [ ] **Step 2: Update ROADMAP.md** — in Phase 4, extend the demographics line: the **provider-number relational model is now specified** ([ADR-0035](spec/decisions/0035-entities-relationships-and-provider-numbers.md), [§4.6](spec/demographics.md)) — abstract entity + reified relationships + subject-kind partitioning; remove the "Open follow-on: provider-number person×org model" clause (now closed).

- [ ] **Step 3: Build to verify links**

Run: `uv run --with mkdocs-material --with mkdocs-callouts --with mkdocs-redirects -- mkdocs build 2>&1 | grep -i "warn\|error" || echo "CLEAN"`
Expected: `CLEAN`.

- [ ] **Step 4: Commit**

```bash
git add docs/HANDOVER.md docs/ROADMAP.md
git commit -m "docs: HANDOVER/ROADMAP currency — ADR-0035, v0.36, gap B closed

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Final verification (after all tasks)

- [ ] **Full clean build:** `uv run --with mkdocs-material --with mkdocs-callouts --with mkdocs-redirects -- mkdocs build 2>&1 | grep -i "warn\|error" || echo "CLEAN"` → `CLEAN`.
- [ ] **Partition invariant stated** in both ADR-0035 and §4.6: `grep -c "subject_kind" docs/spec/demographics.md docs/spec/decisions/0035-entities-relationships-and-provider-numbers.md` → ≥ 1 in each.
- [ ] **Position-not-value example present** in both: `grep -ci "workcover\|position, not value\|position not value" docs/spec/demographics.md docs/spec/decisions/0035-entities-relationships-and-provider-numbers.md` → ≥ 1 in each.
- [ ] **§4.4 boundary no longer says "deferred":** `grep -n "deferred" docs/spec/demographics.md` → no match on the §4.4 provider-number line (the only remaining `deferred` mentions, if any, are unrelated).
- [ ] **Terminology guard:** the §4.6 normalized form is never called "canonical": `grep -in "canonical" docs/spec/demographics.md | grep -i "normal\|identifier"` → only ADR-0031's distinct meaning.
- [ ] Open PR to `main`, linking the design doc and noting demographics gap B is fully closed.
```
