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
   nature of an output is a *recordable, projectable property*. An output is *suppressing* when it can
   reduce, defer, de-prioritise, auto-file, or auto-resolve something a human would otherwise have acted
   on — i.e. it can cause a *loss* versus paper. Whether an *un-owned suppressing* output is permitted is
   policy (principle 9); an override toward permitting it is itself an explicit, audited, owned configuration act.

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
