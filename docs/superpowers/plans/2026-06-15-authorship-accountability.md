# Authorship & Accountability — Spec Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Write the authorship-and-accountability model into the canonical Cairn spec — a new immutable ADR plus edits to five aspect documents and the two working docs — so authorship becomes compositional and legal responsibility becomes a separable, possibly-absent, possibly-proxied attribute.

**Architecture:** This is documentation work, not code. The source of truth is Markdown under `docs/spec/`; the *why* lives in an immutable, numbered ADR under `docs/spec/decisions/`. There is no test framework — **verification = the MkDocs site builds clean and every new cross-reference resolves.** The validated design this plan implements is [docs/superpowers/specs/2026-06-15-authorship-accountability-design.md](../specs/2026-06-15-authorship-accountability-design.md); read it before starting.

**Tech Stack:** Markdown (GitHub/Obsidian callout syntax `> [!NOTE]`), MkDocs Material (`mkdocs.yml`), build via `uv run --with mkdocs-material --with mkdocs-callouts -- mkdocs build`. ADRs are append-only and immutable (see `docs/spec/decisions/README.md`).

**Conventions that bind every task:**
- The spec carries **no in-file changelogs and no filename version suffixes**; git is the line history.
- Author callouts in `> [!NOTE]` / `> [!IMPORTANT]` form so they render on GitHub *and* as Material admonitions.
- Never commit the generated `site/` (gitignored).
- Section numbers are stable anchors; cross-references like `§5.9` must keep resolving.
- Commit after each task with a conventional-commit message ending in the `Co-Authored-By` trailer.

---

## File map (what changes, and why)

| File | Responsibility | Change |
|---|---|---|
| `docs/spec/decisions/0007-authorship-and-accountability.md` | The *why* — immutable rationale | **Create** |
| `docs/spec/decisions/README.md` | ADR index table | Add the 0007 row |
| `docs/spec/index.md` | Mission + principles + version + map | Add principle 10; bump version 0.9 → 0.10 |
| `docs/spec/data-model.md` | The envelope (the *what*) | Contributor set, role enum, responsibility attribute, additive/suppressing property |
| `docs/spec/security.md` | Signing / attestation / trusted base | Decouple signature from attestation; AI-agent identity + recall query |
| `docs/spec/identity.md` | Projections / trust states (the consumer side) | Responsibility-state → trust projection; the three consumer layers |
| `docs/spec/open-questions.md` | Open architecture questions | Note the AI-authorship thread resolved; record deferred follow-ons |
| `CLAUDE.md` | Agent guidance | Note 10th principle + resolved thread |
| `docs/HANDOVER.md` | Working scaffolding | Add a "Resolved 2026-06-15" section |

**Task order matters:** the ADR (Task 1) is the anchor every other document links to, so it goes first. The spec version bump and final build verification go last.

---

## Task 1: Create ADR-0007 (the rationale) + index it

**Files:**
- Create: `docs/spec/decisions/0007-authorship-and-accountability.md`
- Modify: `docs/spec/decisions/README.md` (the `## Index` table, after the 0006 row)

- [ ] **Step 1: Write the ADR file**

Create `docs/spec/decisions/0007-authorship-and-accountability.md` with exactly this content:

```markdown
# ADR-0007 — Authorship is compositional; accountability is separable

- **Status:** Accepted
- **Date:** 2026-06-15

## Context

A binary "AI-generated" tag cannot carry the requirements that AI-authored clinical information brings,
and that information is about to become pervasive: AI scribing and transcription, result-grading,
triage, warnings, and notifications. A flag is something a human must remember to set, it is binary
where reality is a spectrum (shared authorship), and — most importantly — it conflates two things that
must be kept apart: *who or what produced the content* and *who answers for it*.

Today every clinical event carries a single `author` fused with its `signature`
([data-model §3.5](../data-model.md#35-event-storage-model-hybrid-envelope)). For a human author the
signature does double duty — it proves origin/integrity *and* expresses "I vouch for this." AI
authorship breaks that fusion: an AI's output still needs a cryptographic signature (provenance,
integrity, and the ability to answer "which events did model X v2.3 produce?" when a model is later
found defective), but that signature confers no legal responsibility.

The validating case (real, from an emergency physician's practice): a remote community with very high
baseline diabetes / renal failure / rheumatic heart disease, where nearly every pathology result flags
formally abnormal and review capacity is overwhelmed and dangerously delayed. An AI triage that flags
results *dangerously abnormal in the patient's own context* is **strictly additive** — it can only
*raise* a result's priority, never lower it, never auto-file, never remove the human review obligation.
Worst case equals the paper baseline; best case is strictly better. Win-or-no-change. Nothing was taken
from the paper floor, so nothing new was created to answer for.

## Decision

**Authorship is compositional; accountability is a separable attribute.** This is recorded as the
**tenth founding principle** ([index.md](../index.md#founding-principles-the-lens-for-every-decision)).

1. **Contributor set.** An event's `author` becomes a *set* of contributors. Each entry is
   `{ identity, role, descriptor?, responsibility? }`. `identity` is a registered actor — human, **AI
   agent** (model + version + vendor + deploying node), or device. The lone-human note is a one-element
   set. "AI-generated" is the *emergent reading* "the set contains a non-human author and no human in a
   responsibility-bearing role" — never a flag.

2. **Closed core role enum + free descriptor.** Roles are a closed enum (like `event_type`), small
   enough that the safety/DB layer can reason about them and the taxonomy cannot sprawl. It is
   partitioned into *responsibility-bearing* (`authored`, `ordered`, `attested`) and *contributory*
   (`drafted`, `transcribed`, `graded`, `triaged`, `suggested`). An optional free-text descriptor rides
   alongside; no safety logic branches on it.

3. **Responsibility as `{ held_by, on_behalf_of }`.** Not a bare boolean. Absent = un-vouched (legitimate).
   `held_by` a human with no `on_behalf_of` = ordinary self-attestation. `held_by` an AI agent with
   `on_behalf_of` a legal entity = the **proxy** case (accountability routes to the owner/deployer). It is
   orthogonal to human/machine: *"AI is never responsible" is a policy default mapping, not a schema law.*
   The column exists from day one, so the transition toward AI accountability needs no migration.

4. **Signature decoupled from attestation.** A signature proves *origin + integrity*; *attestation* (a
   responsibility-bearing role) confers *responsibility*. Every event is signed, including AI output;
   **signed ≠ vouched-for.** AI agents therefore carry their own registered cryptographic identity,
   making their authorship recall-traceable though (by current policy) never accountable.

5. **No responsible party is legitimate, and structurally characterised.** The additive-vs-suppressing
   nature of an output is a *recordable, projectable property*. An output is *suppressing* if it can
   reduce, defer, de-prioritise, auto-file, or auto-resolve something a human would otherwise have acted
   on (it can cause a loss versus paper). Whether an *un-owned suppressing* output is permitted is policy
   (principle 9); an override toward permitting it is itself an explicit, audited, owned configuration act.

6. **Lifecycle rides existing lineage.** Within-event co-authorship is the contributor set;
   responsibility that attaches *over time* (AI drafts now, a human vouches later) is an ordinary
   append-only event referencing the draft — exactly how signatures, addenda, and corrections already
   work ([data-model §3.1](../data-model.md#31-append-only-clinical-event-log-source-of-truth)). No new
   overlay stream.

7. **Consumer side, three layers** (mirroring the safety-projection design,
   [identity §5.9](../identity.md#59-sensitivity-grade-the-safety-projection-and-break-glass-visibility-scope)):
   an informational floor that never gates (principle 3); a projected trust signal feeding the existing
   chart/event trust states (principle 4 — "no human vouches yet" is acknowledged uncertainty); and an
   expressible-but-never-mandatory policy rung ("un-vouched suppressing output must be attested before it
   takes effect").

## Consequences

- **Easier:** AI scribing, AI triage, and ordinary human notes are one model, not three special cases.
  The "software needs a human to take responsibility" → "the AI colleague is accountable (initially as
  proxy for its owner)" transition is a policy change with **no schema migration** — the attribute was
  always there. The defect/recall question ("which events did agent X v2.3 author?") is a first-class
  query.
- **Harder / new trusted surface:** AI agents now need a registered cryptographic identity and key
  custody — a non-human actor in the §9 trusted base, a blast-radius concern when implementation begins.
  Classifying an output as additive vs suppressing must be defined (author-declared vs output-type-derived)
  and, where policy demands, enforced.
- **The bet:** that keeping responsibility *separable and possibly-absent* — rather than forcing a human
  to own every machine output — matches how AI will actually enter clinical work, and that recording the
  proxy chain now spares a painful retrofit later. We would know the bet is wrong if real deployments find
  the additive/suppressing line unworkable to draw, or if the contributor-set envelope measurably slows
  the Pi-class chart read (the principle-3 floor).
- **Policy-neutral (principle 9):** Cairn records who authored, in what role, and who answers — and stays
  indifferent to whether machines ever hold responsibility in their own right.
```

- [ ] **Step 2: Add the ADR to the index table**

In `docs/spec/decisions/README.md`, add this row to the `## Index` table immediately after the `0006` row:

```markdown
| [0007](0007-authorship-and-accountability.md) | Authorship is compositional; accountability is separable | Accepted | 2026-06-15 |
```

- [ ] **Step 3: Verify links resolve (build the site)**

Run from repo root:
```bash
uv run --with mkdocs-material --with mkdocs-callouts -- mkdocs build 2>&1 | tail -20
```
Expected: build finishes; **no `WARNING` lines** mentioning `0007` or unresolved links. (MkDocs prints a warning for any broken relative link.)

- [ ] **Step 4: Commit**

```bash
git add docs/spec/decisions/0007-authorship-and-accountability.md docs/spec/decisions/README.md
git commit -m "$(printf 'docs(adr): ADR-0007 authorship is compositional, accountability separable\n\nCo-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>')"
```

---

## Task 2: data-model.md — the envelope (contributor set, roles, responsibility, additive/suppressing)

**Files:**
- Modify: `docs/spec/data-model.md` (the envelope bullet at §3.5, line ~39; then add a new subsection §3.9 at end of the §3 block)

- [ ] **Step 1: Amend the envelope-columns bullet (§3.5)**

In `docs/spec/data-model.md`, find the `**Typed/normalized envelope columns**` bullet (line ~39) and replace the phrase `author / device, signature,` with:

```
the **contributor set** ([§3.9](#39-authorship-and-accountability)) replacing the single author/device field, the **signature** (origin + integrity only — *not* attestation, [§3.9](#39-authorship-and-accountability)),
```

- [ ] **Step 2: Append the new subsection §3.9**

Append this section to `docs/spec/data-model.md` at the end of the §3 block (after §3.8, before any later top-level section; if §3.8 is the last section in the file, append at EOF):

```markdown
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
  tagged.

- **Role — a closed core enum + free descriptor.** Roles are a **closed enum** (like `event_type`), kept
  small so the safety/DB layer reasons about them unambiguously and the taxonomy cannot sprawl into an
  unbounded folksonomy. It is partitioned by whether a role *bears or transfers responsibility*:
  **responsibility-bearing** (`authored`, `ordered`, `attested`) vs **contributory** (`drafted`,
  `transcribed`, `graded`, `triaged`, `suggested`). An optional **free-text descriptor** carries nuance
  the machinery never branches on.

- **Responsibility — `{ held_by, on_behalf_of }`, not a boolean.** *Absent* = un-vouched (a legitimate
  state, below). `held_by` a human, no `on_behalf_of` = ordinary self-attestation. `held_by` an AI agent
  with `on_behalf_of` a legal entity = the **proxy** case — the output is accountable, accountability
  routing to its owner/deployer. The attribute is **orthogonal to human/machine**: *"AI is never
  responsible" is a policy default mapping, not a schema law.* The column exists from day one, so the
  transition from "software needs a human to take responsibility" toward "the AI colleague is accountable
  (initially as proxy for its owner)" is a **policy change with no schema migration**.

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

- **Lifecycle rides existing lineage.** Responsibility that attaches *over time* — an AI fires a draft
  now (`{AI, drafted}`, un-vouched); a human vouches later — is a **new event referencing the draft**
  (`{human, attested, responsibility: human}`), exactly how signatures, addenda, and corrections already
  work ([§3.1](#31-append-only-clinical-event-log-source-of-truth)). No new overlay stream; principle 1 is
  satisfied (the draft is never mutated). How the clinician *sees* authorship and responsibility-state is
  [identity §5.10](identity.md#510-authorship-and-responsibility-state-the-consumer-side).
```

- [ ] **Step 3: Build to verify anchors resolve**

```bash
uv run --with mkdocs-material --with mkdocs-callouts -- mkdocs build 2>&1 | tail -20
```
Expected: no warnings referencing `data-model`, `#39`, `#72`, or `#510`. (Tasks 3 and 4 create the `§7.2` and `§5.10` targets; if you run this build before those tasks, the forward links will warn — that is expected and clears once Tasks 3–4 land. To avoid the noise, you may defer this build to Task 5's final verification.)

- [ ] **Step 4: Commit**

```bash
git add docs/spec/data-model.md
git commit -m "$(printf 'docs(spec): data-model contributor set, role enum, separable responsibility\n\nCo-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>')"
```

---

## Task 3: security.md — signature/attestation decoupling + AI-agent identity

**Files:**
- Modify: `docs/spec/security.md` (add a new `## 7.2` after the existing `## 7.1`)

- [ ] **Step 1: Append §7.2**

In `docs/spec/security.md`, after the end of the `## 7.1 Erasure (the severity ladder)` section (and before any later top-level section; if §7.1 is the last section, append at EOF), add:

```markdown
## 7.2 Signing, attestation, and AI-agent identity

> [!IMPORTANT]
> **A signature proves origin and integrity; attestation confers responsibility. They are separable
> acts** (founding principle 10, [ADR-0007](decisions/0007-authorship-and-accountability.md)).

- **Two jobs, unfused.** For a human author the cryptographic signature and the act of vouching collapse
  into one, which is why the envelope historically carried a single `author` + `signature`. AI authorship
  forces them apart: every event is **signed** (origin + integrity, by whatever authored it — including an
  AI agent), but a signature confers **no legal attestation**. *Signed ≠ vouched-for.* Responsibility is a
  separate per-contributor attribute carried by a responsibility-bearing role
  ([data-model §3.9](data-model.md#39-authorship-and-accountability)).

- **AI agents are registered cryptographic identities.** An AI author signs with its own key, bound to
  `model + version + vendor + deploying node`. This makes AI authorship as auditable and **recall-traceable**
  as a human's even though it is (by current policy) never accountable: when a model version is later found
  defective, *"which events did agent X v2.3 author?"* is a first-class query. The AI-agent identity
  **registry and its key custody are part of the trusted base** — a non-human actor inside the
  safety-critical surface ([§9 blast-radius rule](language-substrate.md)); keep it small and reviewer-legible.

- **Policy-neutral (principle 9).** Whether a deployment ever lets responsibility be *held_by* an AI agent
  (as proxy for its owner, or eventually in its own right) is configuration, not a stance Cairn takes. The
  signing/attestation mechanism is indifferent to that choice.
```

- [ ] **Step 2: Build to verify**

```bash
uv run --with mkdocs-material --with mkdocs-callouts -- mkdocs build 2>&1 | tail -20
```
Expected: no warnings referencing `security` or `#72`.

- [ ] **Step 3: Commit**

```bash
git add docs/spec/security.md
git commit -m "$(printf 'docs(spec): decouple signature from attestation; register AI-agent identity\n\nCo-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>')"
```

---

## Task 4: identity.md — responsibility-state → trust projection (consumer side)

**Files:**
- Modify: `docs/spec/identity.md` (add a new `## 5.10` after the existing `## 5.9`; extend the "Chart trust states" line ~81)

- [ ] **Step 1: Extend the chart-trust-states contract line**

In `docs/spec/identity.md`, find the line beginning `**Chart trust states (projection-side contract):**` (line ~81) and append this sentence to the end of that line:

```
 Responsibility-state composes into this contract: an event whose authorship is un-vouched (a non-human author with no responsibility-bearing human) renders with an explicit *unattested* marker — a form of acknowledged uncertainty (principle 4), distinct from *wrong* ([§5.10](#510-authorship-and-responsibility-state-the-consumer-side)).
```

- [ ] **Step 2: Append §5.10**

In `docs/spec/identity.md`, after the end of `## 5.9` (append at EOF if §5.9 is last), add:

```markdown
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
```

- [ ] **Step 3: Build to verify**

```bash
uv run --with mkdocs-material --with mkdocs-callouts -- mkdocs build 2>&1 | tail -20
```
Expected: no warnings referencing `identity`, `#510`, or `#39`.

- [ ] **Step 4: Commit**

```bash
git add docs/spec/identity.md
git commit -m "$(printf 'docs(spec): surface responsibility-state through the trust projection\n\nCo-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>')"
```

---

## Task 5: index.md — the 10th founding principle + version bump

**Files:**
- Modify: `docs/spec/index.md` (principles list after item 9, line ~99; version line, line 9)

- [ ] **Step 1: Add founding principle 10**

In `docs/spec/index.md`, immediately after principle `9. **Policy-neutral infrastructure**` (the block ending at line ~99, before the `---` separator at line ~101), add:

```markdown
10. **Authorship is compositional; accountability is separable** — the author of a clinical event is a
    *set* of contributors (human, AI agent, or device), each in a declared role. **Legal responsibility
    is a distinct attribute, orthogonal to authorship and to whether a contributor is human or machine**:
    it may be absent (no one vouches), held, or proxied (held on another's behalf). A signature proves
    *origin and integrity*; *attestation* confers *responsibility*; the two are separable. Cairn records
    who authored, in what role, and who answers for it — and is indifferent to whether, over time,
    machines come to hold responsibility in their own right. "AI-generated" is therefore an emergent
    reading, never a flag ([data-model §3.9](data-model.md#39-authorship-and-accountability),
    [security §7.2](security.md#72-signing-attestation-and-ai-agent-identity),
    [ADR-0007](decisions/0007-authorship-and-accountability.md)).
```

- [ ] **Step 2: Bump the spec version**

In `docs/spec/index.md` line 9, change `**Spec version:** 0.9` to `**Spec version:** 0.10`.

- [ ] **Step 3: Final full build — must be warning-clean**

```bash
uv run --with mkdocs-material --with mkdocs-callouts -- mkdocs build 2>&1 | tee /tmp/cairn-build.log | tail -25
grep -i "warning" /tmp/cairn-build.log || echo "NO WARNINGS — clean build"
```
Expected: `NO WARNINGS — clean build`. If any warning names a broken link or anchor, fix the referenced file before continuing.

- [ ] **Step 4: Cross-reference sanity check**

Confirm every new anchor target exists:
```bash
cd docs/spec
grep -l "## 3.9 Authorship and accountability" data-model.md
grep -l "## 7.2 Signing, attestation, and AI-agent identity" security.md
grep -l "## 5.10 Authorship and responsibility-state" identity.md
grep -c "0007-authorship-and-accountability" index.md data-model.md security.md identity.md decisions/README.md
cd ../..
```
Expected: the three `grep -l` print their filenames; the final `grep -c` shows a non-zero count in each of the five files.

- [ ] **Step 5: Commit**

```bash
git add docs/spec/index.md
git commit -m "$(printf 'docs(spec): add founding principle 10 (authorship/accountability); bump to v0.10\n\nCo-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>')"
```

---

## Task 6: open-questions.md — record the resolution and deferred follow-ons

**Files:**
- Modify: `docs/spec/open-questions.md`

- [ ] **Step 1: Add a resolution note + follow-ons**

In `docs/spec/open-questions.md`, append this block at the end of the file (it records that AI-authorship is now settled and parks the genuinely-deferred pieces):

```markdown
## Resolved — authorship & accountability (AI-authored clinical information)

The general problem behind "tagging AI-generated content" (AI scribe, transcription, result-grading,
triage, notifications) is **resolved** by founding principle 10 and
[ADR-0007](decisions/0007-authorship-and-accountability.md): authorship is a contributor set and legal
responsibility is a separable, possibly-absent, possibly-proxied attribute
([data-model §3.9](data-model.md#39-authorship-and-accountability)). The notification-economy item (10)
is unaffected — it concerns priority/noise, not authorship.

**Deferred follow-ons (not blocking):**
- **Closed role-enum membership** — the bearing/non-bearing *partition* is settled; the exact member list
  is to be finalised in `data-model.md` (`dictated`, `reviewed`, `co-signed` are candidates).
- **AI-agent identity registry** — registration, keying, version-pinning, and key custody for non-human
  actors; relation to the §9 trusted base and the keystore (a safety-critical / blast-radius concern).
- **Additive-vs-suppressing classification** — author-declared, output-type-derived, or both; and how it
  is validated/enforced where policy demands. The sharpest of the follow-ons; may warrant its own
  case-mining session.
- **Proxy/liability semantics** — what `on_behalf_of` legally binds is out of scope; Cairn records the
  chain, jurisdictions interpret it.
```

- [ ] **Step 2: Build + commit**

```bash
uv run --with mkdocs-material --with mkdocs-callouts -- mkdocs build 2>&1 | grep -i warning || echo "clean"
git add docs/spec/open-questions.md
git commit -m "$(printf 'docs(spec): record authorship/accountability resolution and follow-ons\n\nCo-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>')"
```

---

## Task 7: Update the working docs (CLAUDE.md + HANDOVER.md)

**Files:**
- Modify: `CLAUDE.md` (the "four governing principles" / invariants area and the open-questions paragraph)
- Modify: `docs/HANDOVER.md` (add a "Resolved 2026-06-15" section at the top of the resolved list)

- [ ] **Step 1: Note the 10th principle in CLAUDE.md**

In `CLAUDE.md`, in the paragraph that enumerates the founding principles (the "Two more architectural invariants" / nine-principles area), add a sentence recording that a **tenth** principle now exists:

```markdown
A **tenth founding principle** — *authorship is compositional; accountability is separable* — generalizes
"AI-generated content" into a contributor set plus a separable, possibly-absent, possibly-proxied
responsibility attribute (signature proves origin/integrity; attestation confers responsibility)
([ADR-0007](docs/spec/decisions/0007-authorship-and-accountability.md), spec §3.9 / §5.10 / §7.2).
```

- [ ] **Step 2: Add the HANDOVER resolution section**

In `docs/HANDOVER.md`, immediately under the `## Read these first` block's trailing `---` (i.e. as the newest entry at the top of the "Resolved …" sequence), insert:

```markdown
## Resolved 2026-06-15 — authorship & accountability (now spec v0.10)

Reframed "tag AI-generated content" (raised the prior session) into a general model and a **tenth
founding principle**: **authorship is compositional; accountability is separable**
([ADR-0007](spec/decisions/0007-authorship-and-accountability.md)). No new overlay stream — it reuses the
envelope and existing lineage.

- **Contributor set** replaces the single `author` field: `{identity, role, descriptor?, responsibility?}`,
  identity = human / AI agent (model+version+vendor+node) / device. "AI-generated" is the emergent reading
  "non-human author + no responsible human," never a flag. ([data-model §3.9](spec/data-model.md))
- **Responsibility = `{held_by, on_behalf_of}`** — absent / held / proxied; orthogonal to human-vs-machine.
  *"AI is never responsible" is a policy default, not a schema law* → the transition toward AI accountability
  needs no migration.
- **Signature decoupled from attestation** — signed proves origin+integrity, attestation confers
  responsibility; *signed ≠ vouched-for*; AI agents get a registered crypto identity for recall-traceability.
  ([security §7.2](spec/security.md))
- **No responsible party is legitimate** for a *strictly additive* (win-or-no-change) output — the
  pathology-triage case. Additive-vs-suppressing is a recordable property; un-owned *suppressing* output is
  policy-gated (principle 9). Consumer side = three layers on the existing trust projection
  ([identity §5.10](spec/identity.md)).

**Open follow-ons:** exact role-enum membership; AI-agent identity registry + key custody (trusted-base /
blast-radius); additive-vs-suppressing classification (sharpest — author-declared vs derived); proxy/liability
semantics (out of scope — Cairn records the chain). See [open-questions.md](spec/open-questions.md).
```

- [ ] **Step 3: Commit**

```bash
git add CLAUDE.md docs/HANDOVER.md
git commit -m "$(printf 'docs: note 10th principle (authorship/accountability) in CLAUDE.md + HANDOVER\n\nCo-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>')"
```

---

## Final verification (run after all tasks)

- [ ] **Clean build, no warnings:**
```bash
uv run --with mkdocs-material --with mkdocs-callouts -- mkdocs build 2>&1 | grep -i warning || echo "CLEAN"
```
Expected: `CLEAN`.

- [ ] **All five spec files reference ADR-0007, and the three new anchors exist:**
```bash
cd docs/spec
grep -c "0007-authorship-and-accountability" index.md data-model.md security.md identity.md open-questions.md decisions/README.md
grep -q "## 3.9 Authorship and accountability" data-model.md && grep -q "## 7.2 Signing, attestation" security.md && grep -q "## 5.10 Authorship and responsibility-state" identity.md && echo "ANCHORS OK"
cd ../..
```
Expected: each file shows a non-zero count; prints `ANCHORS OK`.

- [ ] **Version bumped:**
```bash
grep "Spec version" docs/spec/index.md
```
Expected: shows `0.10`.

- [ ] **Regenerate HANDOVER if any other working state changed this session** (per CLAUDE.md convention).

---

## Self-review notes (done while writing this plan)

- **Spec coverage:** every section of the design doc maps to a task — §1 reframe → ADR Context (T1); §2 principle → T5 + T1; §3 data model → T2; §4 no-responsibility → T2 (§3.9 bullet) + ADR (T1); §5 signature decoupling → T3; §6 lifecycle → T2 (§3.9 lineage bullet); §7 consumer side → T4; §8 spec placement → all tasks; §9 follow-ons → T6.
- **Placeholder scan:** the only deliberately-open item is the *exact role-enum membership*, which is correctly parked as a follow-on (T6), not a placeholder in the shipped prose — the prose names a concrete candidate set and fixes the partition.
- **Anchor consistency:** the three new anchors (`#39-authorship-and-accountability`, `#72-signing-attestation-and-ai-agent-identity`, `#510-authorship-and-responsibility-state-the-consumer-side`) are used identically everywhere they are referenced; MkDocs slugifies `## 3.9 Authorship and accountability` → `39-authorship-and-accountability` (it strips the dot), which is why the links omit the dot. **If the final build warns on any of these, trust the build's reported slug and update the links to match.**
