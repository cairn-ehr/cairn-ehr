# Design — Authorship & Accountability

**Date:** 2026-06-15
**Status:** Validated in brainstorming; ready to be written into the spec (new ADR-0007 + spec edits).
**Origin:** A previous session raised "tagging AI-generated content" (AI scribe / AI transcription).
This session reframed that narrow tagging need into a general model of **(co)authorship with
separable legal responsibility**, because AI-authored clinical information (triage, result-grading,
warnings, notifications, scribing) is about to become pervasive and a binary "AI/not-AI" flag cannot
carry the requirements it brings.

---

## 1. The reframe

"AI-generated" is the wrong primitive. It is a flag someone must remember to set, it is binary where
reality is a spectrum, and it cannot express the thing that actually matters clinically and legally:
**who (or what) contributed to this event, in what role, and who answers for it.**

The replacement: authorship becomes **compositional** (a set of contributors), and **accountability
becomes a separate, possibly-absent, possibly-proxied attribute** carried by specific contributors.
"AI-generated" then falls out as an *emergent reading* of the model — an event is AI-generated iff
its contributor set contains a non-human author and no human holds a responsibility-bearing role.
Nobody has to tag it; it is true by construction.

This is the same move Cairn has made repeatedly: replace a brittle boolean with an append-only,
overlay-friendly structure that records reality without forcing a premature judgement (identity links,
sensitivity grades, bitemporal time). Here the move is applied to authorship and accountability.

---

## 2. The new founding principle (10th)

> **10. Authorship is compositional; accountability is separable.**
> The author of a clinical event is a *set* of contributors — human, AI agent, or device — each in a
> declared role. **Legal responsibility is a distinct attribute, orthogonal to authorship and to
> whether a contributor is human or machine.** It may be absent (no one vouches), held (by a
> contributor), or proxied (held by one party on behalf of another). A cryptographic signature proves
> *origin and integrity*; *attestation* confers *responsibility*; the two are separable acts. Cairn
> records who authored, in what role, and who answers for it — and is indifferent to whether, over
> time, machines come to hold responsibility in their own right.

This is not yet a widely-adopted commitment in health IT. It is spelled out as a founding principle
precisely because it is *forced by reality* (AI authorship is arriving regardless) and Cairn's method
is to make the forced-by-reality thing explicit and immutable rather than implicit and ad hoc.

It composes with, rather than replaces, the existing principles:
- **1 (append-only)** — responsibility attaches over time through new events referencing originals.
- **4 (acknowledged uncertainty)** — "no human vouches for this yet" is a first-class recordable
  state, distinct from *wrong*, from *not-yet-reviewed*, and from *refused*.
- **9 (policy-neutral infrastructure)** — whether AI may ever be accountable, and whether un-owned
  output is permitted, are policy decisions Cairn facilitates but never makes.

---

## 3. Data model

### 3.1 Contributor set (replaces the single `author` envelope field)

Today the envelope carries a single `author` fused with `signature`
([data-model.md §3.5](../../spec/data-model.md)). That single field becomes a **contributor set**.
Each entry:

| Field | Meaning |
|---|---|
| `identity` | A registered actor: **human**, **AI agent** (model + version + vendor + deploying node), or **device**. |
| `role` | From a **closed core enum** (§3.2), legible to the safety/DB layer. |
| `descriptor` | Optional free text for human-readable nuance the machinery never branches on. |
| `responsibility` | Optional `{ held_by, on_behalf_of }` (§3.3). Absent = un-vouched. |

The ordinary human note is a **one-element set** — the common case gets no heavier. An AI-scribed note
the clinician edited and signed is a two-element set: `{AI, drafted}` + `{clinician, attested,
responsibility: clinician}`. Mixed authorship and mixed responsibility live inside a single immutable
row.

### 3.2 The closed core role enum (+ free descriptor)

Roles are a **closed enum** (like `event_type`), kept small so the accountability machinery in
Rust/DB can reason about them unambiguously and so the taxonomy cannot sprawl into an unbounded
folksonomy. The enum is partitioned by whether a role *bears or transfers responsibility*:

- **Responsibility-bearing:** `authored`, `ordered`, `attested`.
- **Contributory (non-bearing by default):** `drafted`, `transcribed`, `graded`, `triaged`,
  `suggested`.

(The exact members are to be finalised when written into the spec; the *partition* is the load-bearing
idea.) An optional **free-text descriptor** rides alongside for nuance, but no safety logic branches on
it.

### 3.3 Responsibility as `{ held_by, on_behalf_of }` — the proxy chain

Responsibility is **not a bare boolean**. It is "held by **X**, on behalf of **Y**":

- **Absent** — no one vouches (legitimate; see §4).
- **`held_by` = human, no `on_behalf_of`** — ordinary self-attestation.
- **`held_by` = AI agent, `on_behalf_of` = legal entity** — the **proxy** case: the AI's output is
  accountable, with the accountability routing to its owner/deployer.
- **(future) `held_by` = AI agent, no `on_behalf_of`** — an AI accountable in its own right.

Because responsibility is orthogonal to human/machine, **"AI is never responsible" is a policy
*default mapping*, not a schema law.** The column exists from day one; the transition from
"software needs a human to take responsibility" → "the AI colleague has proven reliable and is
accountable (initially as proxy for its owner)" requires **no schema migration and no extra work** —
only the policy that populates the attribute changes. The infrastructure is deliberately indifferent
to the philosophical/political question of AI personhood; it just facilitates a painless transition.

---

## 4. No responsible party is legitimate — and structurally characterised

An event may have **zero** responsible contributors. The validating case (real, from the user's
practice): a remote indigenous community with very high baseline diabetes / renal failure / rheumatic
heart disease, where nearly every pathology result flags formally abnormal, overwhelming review
capacity. An AI triage that flags results *dangerously abnormal in the patient's own context* is:

- **Strictly additive** — it can only *raise* a result's priority / surface it earlier. It never
  lowers priority, never auto-files, never removes the existing human review obligation.
- Therefore **win-or-no-change**: worst case is exactly the paper baseline (everything still gets
  reviewed on the old timeline); best case is strictly better.
- Therefore **nothing new to answer for** — no accountability was created because nothing was taken
  away from the paper floor. This is principle 3 (paper-parity) read as: a safety net laid *under* the
  floor, never a hole cut *in* it.

**The additive-vs-suppressing nature of an output is a recordable, projectable property** (the
mechanism). An output is *suppressing* when it can reduce, defer, de-prioritise, auto-file, or
auto-resolve something a human would otherwise have acted on — i.e. it can cause a *loss* versus paper.

Whether an **un-owned *suppressing*** output is permitted is **policy** (principle 9). Cairn ships the
distinction and records it; it does not refuse. Consistent with the erasure-ladder / sensitivity
pattern, an override toward permitting un-owned suppression is itself an **explicit, audited, owned
configuration act** — someone is on record as having permitted it.

---

## 5. Signature decoupled from attestation

Today `signature` does two jobs fused into one, because for a human author they collapse:

1. **Integrity + provenance** — "this exact content came from this identity, unaltered" (a
   cryptographic fact).
2. **Legal attestation** — "I, a responsible party, vouch for this" (an accountability claim).

AI authorship forces them apart. The model: **every event is signed** (integrity + provenance, by
whatever authored it — including AI), but **a signature confers no responsibility.** *Signed ≠
vouched-for.* Attestation is the separate act, recorded as a responsibility-bearing contributor.

**Corollary — AI agents carry their own registered cryptographic identity** (model + version + vendor
+ deploying node). This makes AI authorship as auditable and **recall-traceable** as human authorship
even though it is (by current policy) never accountable: when a model version is later found defective,
"which events did agent X v2.3 author?" is a first-class query. Registration of AI-agent identities is
itself part of the trusted base.

---

## 6. Responsibility lifecycle rides existing lineage

No new overlay stream is needed. Two dimensions, two existing mechanisms:

- **Within one event** — co-authorship of a single immutable row is the contributor set (§3.1).
- **Across events** — responsibility that *attaches over time* uses ordinary append-only lineage. The
  AI fires a draft now (`{AI, drafted}`, no responsibility); a human vouches later via a **new event**
  referencing the draft (`{human, attested, responsibility: human}`) — the exact mechanism by which
  signatures, addenda, and corrections already work ([data-model.md §3.1/§3.4](../../spec/data-model.md)).
  Principle 1 is satisfied: the draft is never mutated.

So the AI-scribe lifecycle (draft → clinician edits → clinician signs) and the AI-triage lifecycle
(fire advisory now → optionally vouched later) are the *same* shape, and it is a shape Cairn already
has.

---

## 7. Consumer side — three layers (mirrors the safety-projection design)

Authorship the clinician cannot see is useless. Responsibility-state is surfaced in three layers, the
same structure as the sensitivity / safety-projection design
([identity.md §5.9](../../spec/identity.md)):

1. **Informational floor (always).** The record honestly shows provenance and responsibility-state —
   "AI-drafted, unattested" vs "attested by Dr X". It **never gates, blocks, or forces** anything;
   surfacing it *is* the job. (Principle 3 — confirmation dialogs are explicitly not a safety
   mechanism.)
2. **Projected trust signal.** Responsibility-state feeds the existing **chart/event trust projection**
   (*confirmed / unconfirmed / under-review*, [identity.md:81](../../spec/identity.md)). Un-vouched AI
   content can render visually distinct, or be held out of certain auto-derived projections until
   vouched — still never a hard block. "No human vouches yet" is **acknowledged uncertainty**
   (principle 4).
3. **Expressible policy rung.** "Un-vouched *suppressing* AI output must be attested before it takes
   effect" is an *available* policy, never mandatory — tying back to the additive/suppressing
   distinction in §4.

---

## 8. Where this lands in the spec

| Artifact | Change |
|---|---|
| **ADR-0007** (new) | Authorship & accountability — the *why*. Immutable, dated. Should record the pathology-triage case and the principle-10 statement. |
| **index.md** | Add **founding principle 10**; extend the document map / open-questions resolution notes; bump spec version. |
| **data-model.md §3** | Contributor set + closed role enum + `responsibility {held_by, on_behalf_of}` in the envelope; additive-vs-suppressing as a recordable property. |
| **security.md §7** | Signature/attestation decoupling; AI-agent registered cryptographic identity; recall-traceability query. |
| **identity.md §5.x** | Responsibility-state → trust-projection wiring; the three consumer layers. |
| **CLAUDE.md / HANDOVER.md** | Note the 10th principle and the resolved open thread. |

---

## 9. Open follow-ons (deliberately deferred)

- **Exact closed role-enum membership** — finalise members and the bearing/non-bearing partition when
  writing data-model.md. The partition is settled; the list is not.
- **AI-agent identity registry** — how agents are registered, keyed, and version-pinned; key custody
  for non-human actors; relation to the §9 trusted base and the keystore. Touches the safety-critical
  surface (blast-radius rule).
- **Additive-vs-suppressing classification** — is it author-declared, output-type-derived, or both?
  How is it validated/enforced where policy demands it? (Sharpest of the follow-ons.)
- **Proxy/liability semantics** — what `on_behalf_of` legally binds is out of scope for the spec;
  Cairn records the chain, jurisdictions interpret it.
- **Interaction with the safety projection** — an AI-authored, un-vouched, *sealed* safety signal:
  does responsibility-state coarsen under the same disclosure ladder? Likely yes; confirm when writing.
