# Patient-identifier representation (demographics gap B) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Specify patient-identifier *representation* in demographics §4.4 (namespace/profile split, materialised normalized form, advisory validation, culture-neutral floor invariants) plus a boundary paragraph fixing professional/provider IDs in the §7.5 actor registry; record it as ADR-0033; bump the spec to 0.34.

**Architecture:** Pure spec-prose change across Markdown files — no code. The "test cycle" per task is the **mkdocs build** (broken cross-references surface as build warnings) plus targeted `grep` link checks. Mirrors the just-merged ADR-0032 address change in structure and tone.

**Tech Stack:** Markdown; mkdocs-material. Build command (from CLAUDE.md):
`uv run --with mkdocs-material --with mkdocs-callouts --with mkdocs-redirects -- mkdocs build`

## Global Constraints

- **No in-file changelogs, no version suffixes** — git is the line history; spec version lives only in `index.md`.
- **Author callouts in GitHub/Obsidian syntax** (`> [!NOTE]`) so they render on GitHub and as Material admonitions.
- **Never commit the generated `site/`** (gitignored).
- **ADRs are immutable once accepted** — ADR-0033 is new; do not edit existing ADRs.
- **Terminology guard:** "canonical identifier" = ADR-0031's UUIDv7+multihash; the patient's external identifier uses **"normalized form"**, never "canonical".
- **Branch:** `identifier-representation` (already created; design doc already committed there).
- Source-of-truth design: [docs/superpowers/specs/2026-06-27-identifier-representation-design.md](../specs/2026-06-27-identifier-representation-design.md).

---

### Task 1: ADR-0033 — the decision record (the *why*)

**Files:**
- Create: `docs/spec/decisions/0033-patient-identifier-representation.md`

**Interfaces:**
- Produces: the ADR file later tasks cross-reference as `[ADR-0033](decisions/0033-patient-identifier-representation.md)` (from spec root) / `[ADR-0033](0033-patient-identifier-representation.md)` (from within `decisions/`).

- [ ] **Step 1: Write the ADR file** with this exact content:

```markdown
# ADR-0033 — Patient-identifier representation: namespace/profile split and the matching-survivable normalized form

- **Status:** Accepted
- **Date:** 2026-06-27
- **Refines:** [ADR-0014](0014-locale-pluggable-matcher-comparators.md)

## Context

Demographics [§4.2](../demographics.md#42-per-field-projection-policy) already settles the *projection policy* for patient identifiers (national ID, insurance, program IDs): a multi-valued set **keyed by issuing system**, **set union, never LWW**, with *same-system different-value = very strong evidence against link* — a **hard veto** ([identity §5.2](../identity.md#52-matching-pipeline-safety-asymmetric-false-merge-worse-than-false-split)). What §4 had **not** specified is **representation**: what an issuing system *is* as a globally-meaningful key, how a value is validated, and how the hard veto behaves on a node lacking the system's validator. This is the identifier analogue of the gap [ADR-0032](0032-culture-neutral-address-representation.md) closed for addresses.

The divergence from the address case is load-bearing: **address matching is advisory/weak; identifier matching carries a hard veto.** A naive port of the ADR-0032 address `profile` (one content-addressed `namespace@hash` that is *both* system identity and validator) would mean two nodes on different validator *versions* of one logical system (different hash) fail to recognise "same system", mis-firing or silently weakening the veto. And a node lacking the validator could read two formatting variants of one number as a *mismatch* and demote a good link to *under-review* — a [principle 3](../index.md#founding-principles-the-lens-for-every-decision) (paper-parity) regression caused by a space character. The identifier **value shape** is a can't-retrofit, day-one decision (as with [ADR-0013](0013-attachments-content-addressed-lazy-blob-tier.md) / ADR-0032), so it is fixed now.

This concerns the patient's **external** identifier and is **distinct from [ADR-0031](0031-canonical-identifiers-and-node-local-surrogate-keys.md)**, which owns the term *canonical identifier* for the system's own UUIDv7 + multihash. To avoid collision this ADR says **"normalized form"** for the matching key, never "canonical".

## Decision

A patient identifier is asserted through the existing [§4.1](../demographics.md#41-demographic-assertions) mechanism; the **value** has these facets:

1. **`value`** (mandatory) — the as-entered identifier string; the evidence/legibility facet ([principle 1](../index.md#founding-principles-the-lens-for-every-decision), [principle 11](../index.md#founding-principles-the-lens-for-every-decision) analogue), always sufficient alone, **never destroyed or rewritten**.
2. **`system`** (mandatory) — the **stable content-addressed namespace** the hard veto keys on (`nhs-number`, `medicare-au`), globally meaningful with no central registry. May be an explicit **`unknown`** sentinel ([principle 4](../index.md#founding-principles-the-lens-for-every-decision): an unrecognised number is still recordable — weak evidence, never a veto).
3. **`normalized`** (optional, **materialised at authoring when the profile is present**) — the profile's normalized form of `value`, **the matching key**. Materialising it into the signed event is the identifier analogue of ADR-0032 materialising `display`: a node lacking the profile *code* can still veto-match on the normalized form instead of mis-firing on formatting noise.
4. **`profile`** (optional) — `namespace@hash`, the **versioned validator bundle** (format + checksum + normalizer + matching comparator), riding the [security §7.6](../security.md#76-the-software-distribution-plane-signed-releases-and-extension-load) distribution plane. **Evolves independently of `system`** — the split that lets the veto key on stable identity while validators version freely.
5. **`use`/`type`** (optional) — a recommended-but-open vocabulary (`national-id`, `insurance`, `program`, `mrn`): recommended so the common case interoperates, open so it cannot become capture.

**Matching & honest degradation.** "Different value" in the hard veto means **different `normalized` forms**, not different `value` strings (`9434765919` and `943 476 5919` are the **same** identifier — no veto). The veto **forces a human decision, never an auto-link and never an auto-reject** (identity §5.2). A node decides "same vs different value" only on a basis it can trust: it compares materialised `normalized` forms (works without the profile code); if a `normalized` form is **absent** and the node **lacks the profile** to derive one, it **may treat string-equal as a positive signal but must not declare a same-system mismatch from string inequality** — it **holds for human review** rather than firing the veto or demoting a link. This is the identifier analogue of ADR-0032's *"never reinterpret `parts` under a substitute profile"*: a node never fires a hard veto on a basis it cannot trust. **`system: unknown`** never participates in the veto.

**Advisory validation; culture-neutral floor.** Per-profile validators (checksum, length, format) **flag for human review, never reject** ([principle 4](../index.md#founding-principles-the-lens-for-every-decision)). The in-DB **floor** enforces only structural invariants — `value` non-empty text; `system` present (possibly `unknown`); `normalized` is text when present; **`normalized` materialised ⇒ `profile` named** — and **never holds a profile, never runs a checksum, never rejects on validation** ([principle 12](../index.md#founding-principles-the-lens-for-every-decision)). Cross-facet verification (`normalized == normalizer(value)`) is advisory by profile-holding nodes, not floor-gated.

**The professional-ID boundary.** Patient identifiers answer *"who is this patient?"* (subject = the patient) and live in demographics §4. Professional/provider identifiers — AHPRA/GMC/NPI registration numbers, billing provider numbers — answer *"who is licensed / who may sign / who bills?"* (subject = the clinician as an *actor*) and belong to the [§7.5](../security.md#75-the-actor-registry-enrollment-version-pinning-and-key-custody) actor registry. They are **never conflated**: a person who is both carries patient IDs in demographics and registration numbers in the actor registry — conflating them would let a billing number act as a patient match key, or a patient ID act as a signing credential. A **provider number is relational** (different per practice/location → scoped to person×org); that model is **deferred** — this ADR only draws the line and states the non-conflation invariant.

## Consequences

- **Easier:** any issuing system is supported with **no schema migration** (system + profile are data); the hard veto keys reliably across nodes/locales via the stable namespace; advisory validation catches transcription errors without ever blocking a write; identifier history is intact (append-only).
- **Harder / the bet:** moving validation to data trades away DB-guaranteed format/checksum enforcement (recovered as advisory per-profile validators). A profile-less authoring node cannot materialise `normalized`, so its identifiers route to human review at matching time on capable nodes — the safe degradation, but it shifts work to humans where validators have not yet propagated. We bet (as in [ADR-0014](0014-locale-pluggable-matcher-comparators.md)) that content-addressed profiles distribute reliably off the clinical plane.
- **How we'd know the bet fails:** identifiers arrive whose `system` no available profile can normalize at scale (watch the human-review yield for same-system holds); or a node is observed firing a same-system veto without a trustworthy normalized basis (a correctness bug — the degradation rule was violated).
```

- [ ] **Step 2: Build to verify all cross-reference links resolve**

Run: `uv run --with mkdocs-material --with mkdocs-callouts --with mkdocs-redirects -- mkdocs build 2>&1 | grep -i "warn\|error" || echo "CLEAN"`
Expected: `CLEAN` (no broken-link warnings referencing the new ADR or its anchors).

- [ ] **Step 3: Commit**

```bash
git add docs/spec/decisions/0033-patient-identifier-representation.md
git commit -m "spec(adr): ADR-0033 patient-identifier representation

Namespace/profile split (stable veto key + versioned validator),
materialised normalized form so the hard veto survives a profile-less
node, advisory validation, culture-neutral floor invariants, and the
professional-ID boundary (provider-number person×org model deferred).
Refines ADR-0014; distinct from ADR-0031.

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 2: demographics §4.4 + §4.2 row cross-ref

**Files:**
- Modify: `docs/spec/demographics.md:18` (the §4.2 Identifiers row)
- Modify: `docs/spec/demographics.md` (append new §4.4 after §4.3, currently ending at line 41)

**Interfaces:**
- Consumes: ADR-0033 (Task 1) at `[ADR-0033](decisions/0033-patient-identifier-representation.md)`.
- Produces: anchor `#44-identifiers-representation` referenced by Task 3 (identity §5.2) and by the §4.2 row.

- [ ] **Step 1: Update the §4.2 Identifiers row** — replace the line at `demographics.md:18`:

Old:
```markdown
| Identifiers (national ID, insurance, program IDs) | Multi-valued set keyed by issuing system | Set union, never LWW | Same-system different-value = **very strong evidence against link** |
```
New:
```markdown
| Identifiers (national ID, insurance, program IDs) | Multi-valued set keyed by issuing system; representation per [§4.4](#44-identifiers-representation) | Set union, never LWW | Same-system different-value = **very strong evidence against link** (a hard veto; keys on the [§4.4](#44-identifiers-representation) normalized form, degrades honestly) |
```

- [ ] **Step 2: Append the new §4.4 section** at the end of `demographics.md` (after the §4.3 block ending at line 41):

```markdown

## 4.4 Identifiers: representation

Demographics [§4.2](#42-per-field-projection-policy) settles the *projection policy* for patient identifiers (set union, never LWW; same-system different-value is a hard veto). This section settles their **representation** — and it diverges from the [§4.3](#43-address-the-three-facet-value) address model in one load-bearing way: **address matching is advisory; identifier matching carries a hard veto**, so "same system" must be determinable independently of validator version, and the veto must never mis-fire on a node that lacks a validator ([ADR-0033](decisions/0033-patient-identifier-representation.md), [principle 12](index.md#founding-principles-the-lens-for-every-decision)).

> The patient's **external** identifier here is distinct from the system's own **canonical identifier** (UUIDv7 + multihash, [ADR-0031](decisions/0031-canonical-identifiers-and-node-local-surrogate-keys.md)). This section says **"normalized form"** for the matching key, never "canonical".

An identifier is a [§4.1](#41-demographic-assertions) assertion whose value has these facets, only the first two mandatory:

- **`value`** *(mandatory)* — the as-entered identifier string; the evidence/legibility facet ([principle 1](index.md#founding-principles-the-lens-for-every-decision)), always sufficient alone, **never destroyed or rewritten** (maiden insurance numbers, old transcriptions stay as matching evidence).
- **`system`** *(mandatory)* — the **stable content-addressed namespace** the hard veto keys on (`nhs-number`, `medicare-au`), globally meaningful with no central registry. May be an explicit **`unknown`** sentinel (a number copied off an unrecognised card — [principle 4](index.md#founding-principles-the-lens-for-every-decision): still recordable, *weak* evidence only, never a veto).
- **`normalized`** *(optional, materialised at authoring when the profile is present)* — the profile's normalized form of `value` (separators stripped, case/grouping canonicalised): **the matching key**. Materialising it into the signed event is the identifier analogue of [§4.3](#43-address-the-three-facet-value) materialising `display` — a node lacking the profile *code* can still veto-match correctly on it instead of mis-firing on formatting noise.
- **`profile`** *(optional)* — `namespace@hash`, the **versioned validator bundle** (format + checksum + normalizer + matching comparator), riding the [§7.6](security.md#76-the-software-distribution-plane-signed-releases-and-extension-load) distribution plane. Needed to re-derive/verify `normalized` and to run advisory validation; **evolves independently of `system`** — the split that lets the veto key on stable identity while validators version freely. Reuses the [ADR-0014](decisions/0014-locale-pluggable-matcher-comparators.md) profile-bundle machinery.
- **`use`/`type`** *(optional)* — a **recommended-but-open** vocabulary (`national-id`, `insurance`, `program`, `mrn`): recommended so the common case interoperates, open so it cannot become capture.

**Matching & honest degradation (safety-critical).** Per [identity §5.2](identity.md#52-matching-pipeline-safety-asymmetric-false-merge-worse-than-false-split), *same `system`, different value* is a hard veto that **forces a human decision, never an auto-link and never an auto-reject**. "Different value" means **different `normalized` forms**, not different `value` strings — `9434765919` and `943 476 5919` are the **same** identifier and raise no veto. A node decides "same vs different value" only on a basis it can trust:

- both assertions carry a materialised **`normalized`** form → it compares those (works **without** the profile code; the normal path, and the reason we materialise);
- a `normalized` form is **absent** and this node **also lacks the profile** to derive one → it **may treat string-equal as a positive signal, but must not declare a same-system *mismatch* from string inequality** (the difference may be pure formatting) — it **holds for human review** rather than firing the veto or demoting an existing link.

This is the identifier analogue of [§4.3](#43-address-the-three-facet-value)'s *"never reinterpret `parts` under a substitute profile"*: **a node never fires a hard veto on a basis it cannot trust.** `system: unknown` never participates in the veto.

**Advisory validation; culture-neutral floor.** Per-profile validators (checksum, length, format) **flag for human review, never reject** ([principle 4](index.md#founding-principles-the-lens-for-every-decision)) — a Medicare number off a faded card with a bad check digit, a malformed-but-real legacy ID, a number a dying patient half-recalls must all be recordable. The in-DB **floor** enforces only the culture-neutral structural invariants (`value` non-empty text; `system` present; `normalized` is text when present; **`normalized` materialised ⇒ `profile` named**) and **never holds a profile, never runs a checksum, never rejects on validation** ([principle 12](index.md#founding-principles-the-lens-for-every-decision)). Cross-facet verification (`normalized == normalizer(value)`) is an *advisory* check by profile-holding nodes, never a floor gate.

**Patient vs professional identifiers (the boundary).** This section is **patient** identifiers (subject = the patient). **Professional/provider identifiers** — AHPRA/GMC/NPI registration numbers, billing provider numbers — answer *"who is licensed / who may sign / who bills?"* (subject = the clinician as an *actor*) and live in the [§7.5](security.md#75-the-actor-registry-enrollment-version-pinning-and-key-custody) actor registry, **never conflated** with patient identifiers: a person who is both carries patient IDs here and registration numbers there — conflating them would let a billing number act as a patient match key, or a patient ID as a signing credential. A **provider number is relational** (different per practice/location → scoped to person×org); that model is **deferred** ([ADR-0033](decisions/0033-patient-identifier-representation.md)).
```

- [ ] **Step 3: Build to verify links**

Run: `uv run --with mkdocs-material --with mkdocs-callouts --with mkdocs-redirects -- mkdocs build 2>&1 | grep -i "warn\|error" || echo "CLEAN"`
Expected: `CLEAN`.

- [ ] **Step 4: Verify the new anchor and cross-refs resolve**

Run: `grep -n "## 4.4 Identifiers: representation" docs/spec/demographics.md && grep -c "44-identifiers-representation" docs/spec/demographics.md`
Expected: the heading prints; count ≥ 1 (the §4.2 row references it).

- [ ] **Step 5: Commit**

```bash
git add docs/spec/demographics.md
git commit -m "spec(demographics): §4.4 identifier representation; §4.2 row cross-ref

Facet model (value/system/normalized/profile/use), matching+honest
degradation rule (veto keys on normalized form; profile-less node holds
for review, never declares mismatch), advisory validation, floor
invariants, and the patient-vs-professional boundary. ADR-0033.

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 3: identity §5.2 cross-ref to the degradation rule

**Files:**
- Modify: `docs/spec/identity.md:22` (the §5.2 coherence-check bullet)

**Interfaces:**
- Consumes: the demographics `#44-identifiers-representation` anchor (Task 2).

- [ ] **Step 1: Add the cross-ref** — replace the line at `identity.md:22`:

Old:
```markdown
- **Coherence check (feedback loop):** the unified-chart projection continuously validates linked components against the [§4.2](demographics.md#42-per-field-projection-policy) conflict column. Contradictions (same-system identifier mismatch, verified-DOB clash, sex-at-birth clash) demote the link to human review and render the chart in *under-review* trust mode. Every new demographic assertion cheaply re-triggers local matching.
```
New:
```markdown
- **Coherence check (feedback loop):** the unified-chart projection continuously validates linked components against the [§4.2](demographics.md#42-per-field-projection-policy) conflict column. Contradictions (same-system identifier mismatch, verified-DOB clash, sex-at-birth clash) demote the link to human review and render the chart in *under-review* trust mode. Every new demographic assertion cheaply re-triggers local matching. The same-system identifier mismatch keys on the [§4.4](demographics.md#44-identifiers-representation) **normalized form** and **degrades honestly** — a node lacking the issuing-system profile holds for human review rather than declaring a mismatch from formatting noise.
```

- [ ] **Step 2: Build to verify links**

Run: `uv run --with mkdocs-material --with mkdocs-callouts --with mkdocs-redirects -- mkdocs build 2>&1 | grep -i "warn\|error" || echo "CLEAN"`
Expected: `CLEAN`.

- [ ] **Step 3: Commit**

```bash
git add docs/spec/identity.md
git commit -m "spec(identity): §5.2 cross-ref the §4.4 identifier degradation rule

The same-system mismatch veto keys on the normalized form and degrades
honestly on a profile-less node (ADR-0033).

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 4: spec version bump + ADR index row

**Files:**
- Modify: `docs/spec/index.md:9` (spec version)
- Modify: `docs/spec/decisions/README.md` (append ADR-0033 row after the 0032 row at line 55)

**Interfaces:**
- Consumes: ADR-0033 (Task 1).

- [ ] **Step 1: Bump the spec version** — replace at `index.md:9`:

Old:
```markdown
**Spec version:** 0.33 · **License target:** AGPL-3.0 (all components AGPL-3.0-compatible).
```
New:
```markdown
**Spec version:** 0.34 · **License target:** AGPL-3.0 (all components AGPL-3.0-compatible).
```

- [ ] **Step 2: Add the ADR-0033 index row** — insert immediately after the line at `decisions/README.md:55` (the 0032 row):

```markdown
| [0033](0033-patient-identifier-representation.md) | Patient-identifier representation: namespace/profile split and the matching-survivable normalized form | Accepted (refines 0014) | 2026-06-27 |
```

- [ ] **Step 3: Build to verify links**

Run: `uv run --with mkdocs-material --with mkdocs-callouts --with mkdocs-redirects -- mkdocs build 2>&1 | grep -i "warn\|error" || echo "CLEAN"`
Expected: `CLEAN`.

- [ ] **Step 4: Verify version + row**

Run: `grep -n "Spec version:.*0.34" docs/spec/index.md && grep -n "0033-patient-identifier-representation" docs/spec/decisions/README.md`
Expected: both print one match.

- [ ] **Step 5: Commit**

```bash
git add docs/spec/index.md docs/spec/decisions/README.md
git commit -m "spec: bump spec version 0.33 -> 0.34 (ADR-0033, demographics §4.4)

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 5: HANDOVER + ROADMAP currency

**Files:**
- Modify: `docs/HANDOVER.md` (session summary + the gap-B open-thread bullet)
- Modify: `docs/ROADMAP.md` (Phase 4 demographics line + the ADR index implication)

**Interfaces:**
- Consumes: all prior tasks (this records them).

- [ ] **Step 1: Update HANDOVER.md** — set the header to spec v0.34, replace the "This session" summary with a concise paragraph covering ADR-0033 / demographics §4.4 (namespace/profile split + materialised normalized form + veto-survives-degradation + professional-ID boundary), and **narrow open-thread gap B** to note its *representation* half is now closed, leaving the **provider-number person×org relational model** as the named follow-on (gap C — demographic legibility twin — unchanged). Add the ADR-0033 row to the HANDOVER ADR index table. Keep it under 500 lines (prune the oldest prior-session paragraph if needed).

- [ ] **Step 2: Update ROADMAP.md** — in Phase 4, extend the demographics line: address model (ADR-0032) **and** identifier representation (ADR-0033) now specified; open follow-ons narrowed to **provider-number person×org** (gap B remainder) and the demographic legibility twin (gap C).

- [ ] **Step 3: Build to verify links**

Run: `uv run --with mkdocs-material --with mkdocs-callouts --with mkdocs-redirects -- mkdocs build 2>&1 | grep -i "warn\|error" || echo "CLEAN"`
Expected: `CLEAN`.

- [ ] **Step 4: Commit**

```bash
git add docs/HANDOVER.md docs/ROADMAP.md
git commit -m "docs: HANDOVER/ROADMAP currency — ADR-0033, v0.34, gap B narrowed

Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

## Final verification (after all tasks)

- [ ] **Full clean build:** `uv run --with mkdocs-material --with mkdocs-callouts --with mkdocs-redirects -- mkdocs build 2>&1 | grep -i "warn\|error" || echo "CLEAN"` → `CLEAN`.
- [ ] **Terminology guard:** `grep -rn "canonical" docs/spec/demographics.md docs/spec/decisions/0033-patient-identifier-representation.md | grep -i "identifier"` → only references to ADR-0031's distinct meaning, none calling the §4.4 normalized form "canonical".
- [ ] **No `normalized` without `profile` invariant stated** in both ADR-0033 and §4.4: `grep -c "materialised ⇒ \`profile\`\|materialised ⇒ profile" docs/spec/demographics.md docs/spec/decisions/0033-patient-identifier-representation.md`.
- [ ] Open PR to `main`, linking the design doc and noting gap B's representation half is closed.
```
