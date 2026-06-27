# Demographic Legibility Twin (ADR-0034, gap C) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Bind every demographic assertion to the principle-11 legibility twin (§3.13 / ADR-0012) via one uniform, profile-independent, materialised-at-authoring rule — generalizing the ad-hoc address `display` / identifier `value` facets and guaranteeing future field shapes inherit it.

**Architecture:** Prose-only spec change. New ADR-0034 (refines ADR-0012; adjacent to 0032/0033) records the unifying decision; a new demographics §4.5 states the rule; §4.3/§4.4 are lightly re-worded to call `display`/`value` the *value-core* of the §4.5 twin rather than "the twin"; §3.13 gains a cross-ref naming demographic assertions as a twin-bearing event class. No new envelope field, no new founding principle, no code.

**Tech Stack:** Markdown (GitHub/Obsidian callout syntax `> [!NOTE]`), mkdocs-material build, git.

## Global Constraints

- **Spec is Markdown; HTML is generated.** Never hand-edit `site/`; never commit it (gitignored).
- **Build command (verbatim):** `uv run --with mkdocs-material --with mkdocs-callouts --with mkdocs-redirects -- mkdocs build`
- **ADRs are immutable.** ADR-0034 is a *new* file; never edit 0012/0032/0033 to reverse them. (Adding the new ADR's number to their "see also" is not needed — cross-refs point forward from 0034.)
- **No in-file changelogs, no version suffixes in filenames.** The spec version lives only in `index.md`.
- **Callouts** authored as `> [!NOTE]` so they render on GitHub *and* as Material admonitions.
- **Product neutrality:** never name record-system products or the maintainer's prior projects in spec/ADR prose. Use generic placeholders (`nhs-number`, `medicare-au`) for issuing systems, as 0033 does.
- **Relative links** resolve from the file's own directory (e.g. from `docs/spec/demographics.md`, an ADR is `decisions/0034-...md`; the index is `index.md`).
- **No new founding principle; no new envelope field.** If a task seems to require either, stop — the design is wrong, not the plan.

---

### Task 1: Write ADR-0034

**Files:**
- Create: `docs/spec/decisions/0034-demographic-legibility-twin.md`

**Interfaces:**
- Consumes: the approved design `docs/superpowers/specs/2026-06-27-demographic-legibility-twin-design.md`.
- Produces: ADR-0034 at a stable path/anchor that Task 2 (§4.5), Task 3 (§3.13 cross-ref, decisions README, mkdocs nav, index version) and Task 5 (HANDOVER/ROADMAP) all link to.

- [ ] **Step 1: State the acceptance check (the "test")**

The ADR must, on inspection, satisfy every one of these — verified by grep in Step 3:
  - Header block: `Status: Accepted`, `Date: 2026-06-27`, `Refines: [ADR-0012]...`.
  - Names the rule (uniform twin on every demographic assertion), the reconciliation (`display`/`value` as value-core), the forward guarantee, the floor invariant, and the legibility≠matching boundary.
  - Says **"no new founding principle"** and **"no new envelope field"** explicitly.
  - Links ADR-0012, ADR-0032, ADR-0033, principle 11, principle 12, demographics §4.5, data-model §3.13, identity §5.2.

- [ ] **Step 2: Write the ADR file**

Create `docs/spec/decisions/0034-demographic-legibility-twin.md` with exactly this content:

```markdown
# ADR-0034 — The demographic legibility twin: every demographic assertion stays human-readable without its profile

- **Status:** Accepted
- **Date:** 2026-06-27
- **Refines:** [ADR-0012](0012-schema-evolution-event-format-and-legibility-across-time.md)

## Context

[§3.13](../data-model.md#313-schema-evolution-event-format-and-the-legibility-twin) ([ADR-0012](0012-schema-evolution-event-format-and-legibility-across-time.md)) mandates a signed, mechanically-derived plaintext **legibility twin on every event**, so a node generations behind — or lacking a profile — can still *read* the event as a clinician reads a progress note ([principle 11](../index.md#founding-principles-the-lens-for-every-decision)). Demographic assertions ([§4.1](../demographics.md#41-demographic-assertions)) **are** events, so they already inherit the twin at the envelope level. But §4 never said so, and the two representation gaps closed alongside this one each invented a **field-level** facet that overlaps the **event-level** twin without reconciling them:

- [ADR-0032](0032-culture-neutral-address-representation.md) called the address **`display`** facet "the principle-11 legibility twin" (materialised at authoring, profile-independent).
- [ADR-0033](0033-patient-identifier-representation.md) called the identifier **`value`** facet "the principle-11 legibility analogue."

Three gaps remain. **(1) Unreconciled levels** — a reader cannot tell whether the field-level facet and the §3.13 event twin are one thing or two that can drift. **(2) Most fields have no stated twin** — names, DOB, sex/gender, phone, deceased status, photo ([§4.2](../demographics.md#42-per-field-projection-policy)) carry no legibility statement. **(3) No forward guarantee** — nothing forces a *future* jurisdiction-defined demographic field shape to be legible without its profile, which is exactly how [principle 11](../index.md#founding-principles-the-lens-for-every-decision) silently regresses for demographics: a profile-dependent field is added, a profile-less node renders it as opaque structured noise, and no rule was broken.

## Decision

Demographics is bound to the §3.13 legibility twin by one **uniform rule** — canonical home [demographics §4.5](../demographics.md#45-the-demographic-legibility-twin); the twin mechanism itself is unchanged from [§3.13](../data-model.md#313-schema-evolution-event-format-and-the-legibility-twin).

1. **Every demographic assertion carries the §3.13 twin — no exceptions.** A demographic assertion is a [§4.1](../demographics.md#41-demographic-assertions) event, so it already carries the mandatory signed, mechanically-derived plaintext twin. The twin renders **this demographic fact** as profile-independent plaintext — field + human-readable value + `use`/provenance context (*"Address (residential), document-verified: 12 Smith St, Darwin NT 0800, Australia"*; *"NHS number, document-verified: 943 476 5919"*; *"Date of birth (patient-stated): about 1980 (year only)"* — imprecise facts ([principle 4](../index.md#founding-principles-the-lens-for-every-decision)) render legibly too). For a self-legible scalar (a name string, a DOB) the twin is mechanically the value's own plaintext rendering — the uniformity, not the redundancy, is the point.

2. **It is materialised at authoring and profile-independent.** The twin never requires the field's profile/schema to render, and is carried in the signed event — generalizing [ADR-0032](0032-culture-neutral-address-representation.md)'s "materialise `display` into the signed event at authoring" from one field to all of §4.

3. **`display` and `value` are named instances, not separate twins.** [§4.3](../demographics.md#43-address-the-three-facet-value) `display` and [§4.4](../demographics.md#44-identifiers-representation) `value` are the **value-core** the twin wraps for those fields. There is one twin per assertion; it cannot diverge from a second ([§3.15](../data-model.md#315-the-active-write-model-thin-encounters-co-produced-legibility-and-the-delete-vs-erase-distinction) "one twin, born at authoring").

4. **Forward guarantee.** Any future jurisdiction-defined demographic field shape inherits this rule by construction: it cannot be introduced in a form a profile-less node renders as opaque noise. This is [principle 11](../index.md#founding-principles-the-lens-for-every-decision) made un-forgettable for demographics — the demographic analogue of ADR-0012's additive-only schema evolution.

5. **Culture-neutral floor; advisory verification** ([principle 12](../index.md#founding-principles-the-lens-for-every-decision)). The in-DB floor enforces only the structural invariant — *every demographic assertion carries a non-empty plaintext twin* — and never validates the twin's content, never holds a profile, never runs a formatter. A profile-holding node may re-derive the twin from the structured value and flag drift (`twin == render(value / parts)`), advisory only, never a floor gate — the same treatment §4.3 gives `display == formatter(parts)` and §4.4 gives `normalized == normalizer(value)`.

6. **Legibility is not matching.** The twin is for **reading**, never a matching shortcut. A profile-less node reads the twin but **still** degrades matching to human review per [ADR-0032](0032-culture-neutral-address-representation.md)/[ADR-0033](0033-patient-identifier-representation.md) and [identity §5.2](../identity.md#52-matching-pipeline-safety-asymmetric-false-merge-worse-than-false-split). The matching keys (`normalized`, `geo`, structured `parts`) stay separate from the twin and are unchanged here. Twin readability never upgrades or downgrades a link decision.

## Consequences

- **Easier:** any future demographic field shape is legible on any node by construction; the field-level/event-level twin ambiguity is resolved (one twin per assertion); names/DOB/phone gain an explicit legibility statement they lacked; auditing/RAG/full-text over demographics inherits the §3.13 substrate for free.
- **Harder / the bet:** authoring code must materialise a faithful plaintext twin for *every* demographic field, including future ones — the discipline ADR-0032 applied to `display`, now fleet-wide for §4. We bet this is cheap (the twin already had to exist per §3.13) and that mechanical derivation keeps twin and value from drifting (the same §3.13 bet).
- **How we'd know the bet fails:** a demographic assertion is observed whose twin requires a profile to render (a profile-less node shows opaque noise — the rule was violated at authoring); or twin and structured value drift in practice despite mechanical derivation (poisoning audit/RAG — the §3.13 risk, surfaced by the advisory cross-facet check).
- **No new founding principle; no new envelope field.** This is an application of [principle 11](../index.md#founding-principles-the-lens-for-every-decision) (legibility across time) and [principle 12](../index.md#founding-principles-the-lens-for-every-decision) (culture-neutral floor) reusing the existing [ADR-0012](0012-schema-evolution-event-format-and-legibility-across-time.md) twin mechanism. The contribution is unification + a forward guarantee, not a new mechanism.
```

- [ ] **Step 3: Verify the ADR satisfies the acceptance check**

Run:
```bash
cd /Users/hherb/src/cairn-ehr
f=docs/spec/decisions/0034-demographic-legibility-twin.md
grep -q "Status:\*\* Accepted" "$f" && \
grep -q "Date:\*\* 2026-06-27" "$f" && \
grep -q "Refines:\*\* \[ADR-0012\]" "$f" && \
grep -q "no new founding principle" "$f" && \
grep -q "no new envelope field" "$f" && \
grep -q "Legibility is not matching" "$f" && \
grep -q "value-core" "$f" && \
grep -q "0033-patient-identifier-representation.md" "$f" && \
grep -q "0032-culture-neutral-address-representation.md" "$f" && \
echo "ADR-0034 acceptance: PASS" || echo "ADR-0034 acceptance: FAIL"
```
Expected: `ADR-0034 acceptance: PASS`

- [ ] **Step 4: Commit**

```bash
cd /Users/hherb/src/cairn-ehr
git add docs/spec/decisions/0034-demographic-legibility-twin.md
git commit -m "docs(adr): ADR-0034 — the demographic legibility twin (gap C)"
```

---

### Task 2: Add demographics §4.5 and reconcile §4.3/§4.4

**Files:**
- Modify: `docs/spec/demographics.md` (append §4.5 after §4.4 line 66; reword one bullet in §4.3 ~line 32; reword one bullet in §4.4 ~line 51)

**Interfaces:**
- Consumes: ADR-0034 at `decisions/0034-demographic-legibility-twin.md` (Task 1).
- Produces: anchor `#45-the-demographic-legibility-twin` linked from ADR-0034, §3.13 (Task 3), and identity. The §4.5 heading text must be exactly `## 4.5 The demographic legibility twin` so the GitHub-slug anchor matches the links written in Task 1.

- [ ] **Step 1: State the acceptance check**

After the edit: `demographics.md` contains a `## 4.5 The demographic legibility twin` section; §4.3's `display` bullet no longer calls itself "the … legibility twin" but "the **value-core**" of the §4.5 twin; §4.4's `value` bullet no longer says "legibility facet"/"legibility analogue" standalone but ties to §4.5 as value-core; mkdocs builds clean (Task 4 runs the full build).

- [ ] **Step 2: Reword the §4.3 `display` bullet**

In `docs/spec/demographics.md`, find the `display` bullet (line ~32) beginning ``- **`display`** *(mandatory)* — the complete human-readable address, the [principle 11]...``. Replace the phrase ``the [principle 11](index.md#founding-principles-the-lens-for-every-decision) legibility twin`` with:

```
the **value-core** of the [§4.5](#45-the-demographic-legibility-twin) demographic legibility twin for an address ([principle 11](index.md#founding-principles-the-lens-for-every-decision))
```

Leave the rest of the bullet (derived/authored/materialised behaviour) unchanged — behaviour does not change.

- [ ] **Step 3: Reword the §4.4 `value` bullet**

In the §4.4 facet list, find the `value` bullet (line ~51) ``- **`value`** *(mandatory)* — the as-entered identifier string; the evidence/legibility facet ([principle 1]...``. Replace ``the evidence/legibility facet`` with:

```
the evidence facet and the **value-core** of the [§4.5](#45-the-demographic-legibility-twin) demographic legibility twin for an identifier
```

Leave the rest (always sufficient alone, never destroyed/rewritten) unchanged.

- [ ] **Step 4: Append §4.5**

After the last line of §4.4 (line 66, the professional-ID boundary paragraph), append:

```markdown

## 4.5 The demographic legibility twin

A demographic assertion ([§4.1](#41-demographic-assertions)) is an event, so it **already carries the mandatory [§3.13](data-model.md#313-schema-evolution-event-format-and-the-legibility-twin) signed, mechanically-derived plaintext legibility twin** ([principle 11](index.md#founding-principles-the-lens-for-every-decision), [ADR-0034](decisions/0034-demographic-legibility-twin.md)). This section binds demographics to that invariant and adds the two demographic-specific requirements the [§4.3](#43-address-the-three-facet-value) address case discovered, **uniformly across every field**.

- **The twin renders *this demographic fact* as profile-independent plaintext** — field + human-readable value + `use`/provenance context. *"Address (residential), document-verified: 12 Smith St, Darwin NT 0800, Australia"*; *"NHS number, document-verified: 943 476 5919"*; *"Name (legal): 田中 太郎"*; *"Date of birth (patient-stated): about 1980 (year only)"* — imprecise facts ([principle 4](index.md#founding-principles-the-lens-for-every-decision)) render legibly too.
- **Materialised at authoring, profile-independent.** The twin never requires the field's profile/schema to render and is carried in the signed event, so a node lacking that profile — or generations of schema behind — still reads the fact. This generalizes [§4.3](#43-address-the-three-facet-value)'s "materialise `display` into the signed event at authoring" from one field to all of §4.
- **No exceptions; `display`/`value` are named instances.** The twin is mandatory on *every* demographic assertion. [§4.3](#43-address-the-three-facet-value) `display` and [§4.4](#44-identifiers-representation) `value` are the **value-core** the twin wraps for those fields — one twin per assertion, never a second that can drift ([§3.15](data-model.md#315-the-active-write-model-thin-encounters-co-produced-legibility-and-the-delete-vs-erase-distinction)). For a self-legible scalar (a name string, a DOB) the value-core is the value's own plaintext rendering. The uniformity, not the redundancy, is the point: it is what stops a *future* field shape from silently regressing [principle 11](index.md#founding-principles-the-lens-for-every-decision).

**Forward guarantee.** Any future jurisdiction-defined demographic field shape inherits this rule by construction — it cannot be introduced in a form a profile-less node renders as opaque structured noise. This is the demographic analogue of [ADR-0012](decisions/0012-schema-evolution-event-format-and-legibility-across-time.md)'s additive-only schema evolution.

**Culture-neutral floor; advisory verification** ([principle 12](index.md#founding-principles-the-lens-for-every-decision)). The in-DB floor enforces only the structural invariant — *every demographic assertion carries a non-empty plaintext twin* — and **never validates the twin's content, never holds a profile, never runs a formatter**. A profile-holding node may re-derive the twin from the structured value and flag drift (`twin == render(value / parts)`), **advisory only, never a floor gate** — the same treatment [§4.3](#43-address-the-three-facet-value) gives `display == formatter(parts)` and [§4.4](#44-identifiers-representation) gives `normalized == normalizer(value)`.

**Legibility is not matching.** The twin is for **reading**, never a matching shortcut. A profile-less node reads the twin but **still** degrades matching to human review per [ADR-0032](decisions/0032-culture-neutral-address-representation.md)/[ADR-0033](decisions/0033-patient-identifier-representation.md) and [identity §5.2](identity.md#52-matching-pipeline-safety-asymmetric-false-merge-worse-than-false-split); the matching keys (`normalized`, `geo`, structured `parts`) stay separate from the twin. Twin readability never upgrades or downgrades a link decision ([ADR-0034](decisions/0034-demographic-legibility-twin.md)).
```

- [ ] **Step 5: Verify the section and reconciliation landed**

Run:
```bash
cd /Users/hherb/src/cairn-ehr
f=docs/spec/demographics.md
grep -q "^## 4.5 The demographic legibility twin" "$f" && \
grep -q "value-core.*demographic legibility twin for an address" "$f" && \
grep -q "value-core.*demographic legibility twin for an identifier" "$f" && \
grep -q "Legibility is not matching" "$f" && \
! grep -q "the \[principle 11\](index.md#founding-principles-the-lens-for-every-decision) legibility twin; always sufficient" "$f" && \
echo "demographics §4.5 acceptance: PASS" || echo "demographics §4.5 acceptance: FAIL"
```
Expected: `demographics §4.5 acceptance: PASS`

- [ ] **Step 6: Commit**

```bash
cd /Users/hherb/src/cairn-ehr
git add docs/spec/demographics.md
git commit -m "docs(spec): demographics §4.5 demographic legibility twin; reconcile §4.3/§4.4 facets"
```

---

### Task 3: Wire §3.13 cross-ref, version bump, decisions index, and mkdocs nav

**Files:**
- Modify: `docs/spec/data-model.md` (the §3.13 twin bullet, ~line 232)
- Modify: `docs/spec/index.md:9` (spec version)
- Modify: `docs/spec/decisions/README.md` (append ADR-0034 row after line 56)
- Modify: `mkdocs.yml` (append ADR-0034 nav entry after line 131)

**Interfaces:**
- Consumes: ADR-0034 (Task 1) at `0034-demographic-legibility-twin.md`; §4.5 anchor (Task 2).
- Produces: a buildable nav + a discoverable §3.13→§4.5 link; spec version `0.35`.

- [ ] **Step 1: State the acceptance check**

`index.md` says `0.35`; `decisions/README.md` has an ADR-0034 row; `mkdocs.yml` has an ADR-0034 nav line; §3.13's twin bullet names demographic assertions as a twin-bearing event class linking §4.5.

- [ ] **Step 2: Add the §3.13 cross-ref**

In `docs/spec/data-model.md`, in the §3.13 bullet that begins ``- **A mandatory, signed, mechanically-derived plaintext legibility twin on every event**`` (~line 232), append this sentence to the **end of that bullet** (after the existing final sentence about the carried twin co-produced inline):

```
 **Demographic assertions ([§4.1](demographics.md#41-demographic-assertions)) are a twin-bearing event class**: every demographic assertion carries this twin, materialised profile-independently so a node lacking the field's profile still reads the fact ([§4.5](demographics.md#45-the-demographic-legibility-twin), [ADR-0034](decisions/0034-demographic-legibility-twin.md)).
```

- [ ] **Step 3: Bump the spec version**

In `docs/spec/index.md`, line 9, change `**Spec version:** 0.34` to `**Spec version:** 0.35`.

- [ ] **Step 4: Add the decisions-README row**

In `docs/spec/decisions/README.md`, immediately after the ADR-0033 row (line 56), add:

```markdown
| [0034](0034-demographic-legibility-twin.md) | The demographic legibility twin: every demographic assertion stays human-readable without its profile | Accepted (refines 0012) | 2026-06-27 |
```

- [ ] **Step 5: Add the mkdocs nav entry**

In `mkdocs.yml`, immediately after the ADR-0033 line (line 131), add (match the exact indentation of the ADR-0033 line — 6 spaces before `- ADR`):

```yaml
      - ADR-0034 · The demographic legibility twin: spec/decisions/0034-demographic-legibility-twin.md
```

- [ ] **Step 6: Verify the wiring**

Run:
```bash
cd /Users/hherb/src/cairn-ehr
grep -q "Spec version:\*\* 0.35" docs/spec/index.md && \
grep -q "0034-demographic-legibility-twin.md" docs/spec/decisions/README.md && \
grep -q "ADR-0034 · The demographic legibility twin: spec/decisions/0034-demographic-legibility-twin.md" mkdocs.yml && \
grep -q "twin-bearing event class" docs/spec/data-model.md && \
echo "wiring acceptance: PASS" || echo "wiring acceptance: FAIL"
```
Expected: `wiring acceptance: PASS`

- [ ] **Step 7: Commit**

```bash
cd /Users/hherb/src/cairn-ehr
git add docs/spec/data-model.md docs/spec/index.md docs/spec/decisions/README.md mkdocs.yml
git commit -m "docs(spec): §3.13 cross-ref to §4.5; spec 0.34→0.35; ADR-0034 in decisions index + nav"
```

---

### Task 4: Build the site and audit links (the integration test)

**Files:** none modified (verification only; fixes, if any, go back into the relevant file)

**Interfaces:**
- Consumes: all edits from Tasks 1–3.
- Produces: a clean mkdocs build proving every new cross-reference resolves.

- [ ] **Step 1: Run the mkdocs build**

Run:
```bash
cd /Users/hherb/src/cairn-ehr
uv run --with mkdocs-material --with mkdocs-callouts --with mkdocs-redirects -- mkdocs build 2>&1 | tee /tmp/mkdocs-build.log
```
Expected: build completes; the log shows no `WARNING` lines referencing `0034`, `demographics.md`, `#45-the-demographic-legibility-twin`, or a broken link from `data-model.md`. (mkdocs may emit pre-existing warnings unrelated to this change; confirm none are introduced by our four edited files.)

- [ ] **Step 2: Grep the build log for new broken-link warnings**

Run:
```bash
grep -iE "warn|broken|unrecognized|not found" /tmp/mkdocs-build.log | grep -iE "0034|0033|0032|demographic|#45" || echo "no new link warnings: PASS"
```
Expected: `no new link warnings: PASS` (if any line prints, open the named file and fix the link, then re-run Step 1).

- [ ] **Step 3: Confirm the generated site is not staged**

Run:
```bash
cd /Users/hherb/src/cairn-ehr
git status --porcelain site/ ; git check-ignore site >/dev/null && echo "site/ ignored: PASS" || echo "site/ NOT ignored: FAIL"
```
Expected: no `site/` paths listed by `git status`; `site/ ignored: PASS`.

- [ ] **Step 4: No commit**

This task verifies; it produces no commit unless Step 2 forced a fix (in which case commit that fix to the file it belongs to with an explanatory message, then re-run Steps 1–2).

---

### Task 5: Update HANDOVER and ROADMAP currency

**Files:**
- Modify: `docs/HANDOVER.md` (top "This session" block; "Open threads" demographics line)
- Modify: `docs/ROADMAP.md` (Phase 4 demographics line, ~line 53)

**Interfaces:**
- Consumes: the completed spec change (gap C closed; spec 0.35).
- Produces: HANDOVER/ROADMAP that name gap C as closed and gap B's provider-number remainder as the sole open demographics follow-on. Both stay under 500 lines.

- [ ] **Step 1: State the acceptance check**

HANDOVER's lead block describes this session as closing gap C (ADR-0034 + §4.5) at spec v0.35; the demographics open-thread line drops gap C and keeps only the provider-number person×org remainder of gap B. ROADMAP Phase 4 mentions ADR-0034/§4.5 and updates the "Open follow-ons" to drop gap C.

- [ ] **Step 2: Rewrite the HANDOVER lead block**

In `docs/HANDOVER.md`, update the header line (line 3) version token from `v0.34 (+ADR-0033)` to `v0.35 (+ADR-0034)`, and replace the current "This session (2026-06-27)" paragraph (the ADR-0033 one, lines ~7–22) with a concise gap-C summary. Keep it brief (≤ ~10 lines). Use this text:

```markdown
**This session (2026-06-27):** closed demographics **gap C** — tied the [principle 11](index.md) legibility
twin to **all** demographic assertions. New **[ADR-0034](spec/decisions/0034-demographic-legibility-twin.md)**
+ **demographics §4.5**. One uniform rule: every demographic assertion is a §3.13 event, so it already carries
the mandatory signed plaintext twin; §4.5 binds demographics to it, requires the twin **materialised at
authoring + profile-independent** (a profile-less node always reads the fact), reconciles the ad-hoc §4.3
`display` / §4.4 `value` facets as the **value-core** the one twin wraps (no second twin that can drift), and
guarantees any **future** field shape inherits it by construction. Floor enforces only "non-empty twin
present" (never twin content); cross-facet `twin == render(value)` is advisory. Explicit **legibility ≠
matching** boundary: the twin is for reading, matching still degrades to human review per ADR-0032/0033.
§3.13 cross-ref added; spec 0.34→0.35; brainstorm→design→plan→execute (under `docs/superpowers/`); mkdocs
clean. **Open demographics follow-on:** gap B remainder — the **provider-number person×org** relational model
(professional IDs already fixed in the §7.5 actor registry, not conflated with patient IDs).
```

(The prior ADR-0033 / ADR-0032 session paragraphs move down unchanged as "Earlier"/"Prior session" history; trim the oldest prior-session blocks if HANDOVER exceeds 500 lines.)

- [ ] **Step 3: Update the HANDOVER demographics open-thread line**

In the "Open threads" menu, the bullet currently reads "**Demographics gaps B & C** … **C** — tie the principle-11 legibility twin …". Replace that bullet with:

```markdown
- **Demographics gap B remainder** (gap C **closed** this session — ADR-0034/§4.5): the open piece is the
  **provider-number person×org** relational model (professional IDs already fixed in the §7.5 actor registry,
  never conflated with patient IDs — boundary stated in ADR-0033/§4.4).
```

- [ ] **Step 4: Add the ADR-0034 row to the HANDOVER ADR index table**

In the "ADR index" table near the end of HANDOVER, after the ADR-0033 row, add:

```markdown
| [0034](spec/decisions/0034-demographic-legibility-twin.md) | The demographic legibility twin: every demographic assertion legible without its profile | §4.5 (refines 0012) |
```

- [ ] **Step 5: Update ROADMAP Phase 4**

In `docs/ROADMAP.md`, the Phase 4 demographics bullet (~line 53) ends with "Open follow-ons: **provider-number person×org** model (gap B remainder) and demographic legibility-twin (gap C)." Replace that trailing sentence with:

```markdown
**Demographic legibility twin specified** ([ADR-0034](spec/decisions/0034-demographic-legibility-twin.md), [§4.5](spec/demographics.md)): every demographic assertion carries the §3.13 principle-11 twin, materialised profile-independently, with `display`/`value` reconciled as its value-core and a forward guarantee for future field shapes. Open follow-on: **provider-number person×org** model (gap B remainder).
```

- [ ] **Step 6: Verify currency + line counts**

Run:
```bash
cd /Users/hherb/src/cairn-ehr
grep -q "v0.35 (+ADR-0034)" docs/HANDOVER.md && \
grep -q "gap C \*\*closed\*\*" docs/HANDOVER.md && \
grep -q "0034-demographic-legibility-twin.md" docs/ROADMAP.md && \
echo "currency acceptance: PASS" || echo "currency acceptance: FAIL"
wc -l docs/HANDOVER.md docs/ROADMAP.md
```
Expected: `currency acceptance: PASS`; both files ≤ 500 lines (trim oldest history blocks if over).

- [ ] **Step 7: Commit**

```bash
cd /Users/hherb/src/cairn-ehr
git add docs/HANDOVER.md docs/ROADMAP.md
git commit -m "docs: HANDOVER/ROADMAP currency — gap C closed (ADR-0034, §4.5, spec 0.35)"
```

---

## Self-Review (completed by plan author)

**Spec coverage** — every design section maps to a task:
- Design §1 rule → Task 2 Step 4 (§4.5) + Task 1 (ADR decision pt 1–2).
- Design §2 reconciliation → Task 2 Steps 2–3 + Task 1 decision pt 3.
- Design §3 forward guarantee → Task 1 decision pt 4 + Task 2 §4.5 "Forward guarantee".
- Design §4 floor/verification → Task 1 decision pt 5 + Task 2 §4.5 "Culture-neutral floor".
- Design §5 boundary → Task 1 decision pt 6 + Task 2 §4.5 "Legibility is not matching".
- Design §6 deliverables → ADR (T1), §4.5 (T2), §4.3/§4.4 fix (T2), §3.13 cross-ref + version + index + nav (T3), build (T4), HANDOVER/ROADMAP (T5).

**Placeholder scan** — no TBD/TODO/"handle edge cases"; every edit shows verbatim text.

**Consistency** — the §4.5 heading text `## 4.5 The demographic legibility twin` produces the GitHub slug `#45-the-demographic-legibility-twin` used identically in Task 1, Task 2 (internal), and Task 3. ADR filename `0034-demographic-legibility-twin.md` is identical across Tasks 1/3/5. Version `0.35` consistent across Task 3 and Task 5.
```
