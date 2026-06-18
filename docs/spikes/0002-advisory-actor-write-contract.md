# Spike 0002 — Advisory-Actor Write Contract (kastellan ↔ Cairn)

- **Status:** **Proposed** (drafted 2026-06-18; not yet run). Extends the
  [Spike 0001](0001-walking-skeleton-wan-sync-and-pi-cost.md) walking skeleton.
- **Date:** 2026-06-18
- **Motivation:** [Ecosystem eval 0001](../ecosystem/0001-agent-and-messaging-plugins-kastellan-localmail.md)
  concluded that kastellan and localmail fit as a three-membrane, nested-chokepoint stack. That conclusion is
  *reasoning*; this spike turns the narrowest load-bearing claim into a *demonstration*.
- **Validates:** the **advisory-tier integration contract** — that an external advisory agent can author into Cairn
  through the validated write path as an **additive, un-attested, provenance-anchored, recallable** event, **and**
  that the in-DB safety floor rejects everything a buggy or hostile agent must not be allowed to do. Concretely:
  [ADR-0021](../spec/decisions/0021-layering-the-node-api-and-ui-pluralism.md) (floor-in-DB; nothing above L0 on the
  inter-node path), [ADR-0022](../spec/decisions/0022-validated-submit-surface-the-write-path.md) (the validated
  submit surface), [ADR-0007](../spec/decisions/0007-authorship-and-accountability.md) (contributor set; signature ≠
  attestation), [ADR-0010](../spec/decisions/0010-additive-vs-suppressing-classification.md) (additive vs
  suppressing), and [ADR-0011](../spec/decisions/0011-actor-registry-version-pinning-and-key-custody.md) (actor
  registry, version-pinning, contamination cascade).
- **Does not yet ratify anything.** Passing this spike is the **trigger** to write two ADRs (see §6): the parked
  [ADR-0011](../spec/decisions/0011-actor-registry-version-pinning-and-key-custody.md) **skill-epoch** refinement, and
  an ADR fixing the advisory-actor integration contract. Both are written *after* the spike, citing its results.

> [!NOTE]
> Build-prep, not architecture. The numbered spec (§1–§11) and the ADR log describe a *decided* design; a spike is an
> implementation task that *exercises* that design against reality. This one exercises the **record gate** — Cairn's
> in-DB floor — which is the half of the kastellan nesting that lives in Cairn. (The agent-action gate, CASSANDRA, is
> kastellan-internal and out of scope here.)

---

## 1. Why this spike, and why now

The ecosystem eval established *fit*; the cheapest way to convert "fits" into "demonstrably fits, and the floor holds"
is to build the **one** end-to-end thread that is hard to retrofit: an advisory agent authoring a clinical advisory
into Cairn **through the validated submit surface**, never around it. The day-one shapes it pins down — the actor
record, the contributor set, the additive/suppressing classification, the provenance reference — are exactly the
*can't-retrofit* set; their speed is not in question, their **shape** is.

| Bet | What stresses it | Character |
|---|---|---|
| **C — the advisory-actor write contract** | a stand-in agent authoring an additive, un-attested, provenance-bearing triage advisory through `submit_event`, **plus** a buggy/hostile agent trying to breach the floor with direct DB access | **design-validity** (is the integration contract *right* and *unbypassable*) |

This is a **design-validity** bet, like Spike 0001's Bet A — not a performance bet. A "slow" result would be a tuning
task; a "the floor can't express this" result is design feedback that goes back to
[ADR-0022](../spec/decisions/0022-validated-submit-surface-the-write-path.md).

---

## 2. What this spike is *not*

- **Not kastellan itself.** It uses a minimal **Python agent stand-in** that mimics the integration *contract*
  (registers as an actor, reads a provenance-bearing input, authors an advisory). Porting real kastellan is out of
  scope — the agent is advisory, so the stand-in is fit-for-purpose Python
  ([§9.1](../spec/language-substrate.md)).
- **Not localmail itself.** The "source" is a single content-addressed blob standing in for a mirrored mail (the
  [§6.6](../spec/sync.md) byte tier already in the skeleton). It validates the **provenance-anchoring shape**, not
  localmail's mirror.
- **Not the notification economy.** [ADR-0009](../spec/decisions/0009-notification-economy-salience-routing-and-the-acknowledgment-floor.md)
  salience/ack/escalation is the *consumer* of the advisory and is stubbed (one optional stretch row, §5 C6).
- **Not CASSANDRA, and not a transport security review** — it inherits the [§7](../spec/security.md) stubs (WireGuard
  transport, no real distribution plane) from Spike 0001.

---

## 3. What gets built (on top of the Spike 0001 skeleton)

The smallest pieces of the **write path** that are *genuinely* the architecture, not a mock — added to
`poc/walking-skeleton`. Safety-critical pieces in-DB/Rust; the agent stand-in in Python
([§9.1 blast-radius rule](../spec/language-substrate.md)).

1. **Actor registry** ([ADR-0011](../spec/decisions/0011-actor-registry-version-pinning-and-key-custody.md)) — an
   append-only, version-pinned table over the closed actor-event algebra (`enroll` / `supersede` / `revoke`). Pinned
   standing-config columns: vendor, model, version, weights-ref, inference-config, system-prompt-ref, tool-&-RAG
   config, deploying-node — **plus a `skill_epoch` content-address** (the parked refinement, exercised here). Enroll
   one agent actor with its own Ed25519 signing identity.
2. **Contributor set + responsibility** on the event envelope
   ([ADR-0007](../spec/decisions/0007-authorship-and-accountability.md)) — replace Spike 0001's single-author stub
   ([§3.9](../spec/data-model.md)) with a set of `{identity, role, responsibility?}`. The agent authors with role
   `triaged` and **no responsibility attribute** → "AI-generated, un-vouched" is true *by construction*, never a flag.
3. **A minimal `submit_event`** ([ADR-0022](../spec/decisions/0022-validated-submit-surface-the-write-path.md)) — one
   generic validated append, dispatching to additively-registered validators by `(event_type, schema_version)`,
   running the write-time seams that matter for this bet, atomically and in-DB:
   attestation-token check → envelope + Tier-1 ceiling → **additive-vs-suppressing classification**
   ([ADR-0010](../spec/decisions/0010-additive-vs-suppressing-classification.md)) → provenance binding (the source
   blob's content-address) → canonicalize + plaintext twin + sign + idempotent append. PL/pgSQL + the Spike 0001 pgrx
   verifier.
4. **The agent stand-in** (Python) — loads its actor identity + skill-epoch, reads the provenance blob, computes a
   trivial urgency score, and authors the advisory **only** through `submit_event` (never raw `INSERT`). The agent's
   DB role gets `EXECUTE` on the submit function and `SELECT` on projections — **not** `INSERT` on the event table
   (the [§9.4](../spec/language-substrate.md) / [ADR-0021](../spec/decisions/0021-layering-the-node-api-and-ui-pluralism.md)
   grant model extended to a machine client).
5. **Recall** — a query "events authored by actor-UUID X under skill-epoch E", and a `revoke`/recall **overlay** that
   marks affected events without erasing (the contamination cascade, in miniature).

---

## 4. The floor under a hostile or buggy agent

The sharpest half of the bet: a *misbehaving* agent — or an attacker who has compromised one — must not breach the
floor **even with direct DB access**. This is
[ADR-0021](../spec/decisions/0021-layering-the-node-api-and-ui-pluralism.md)'s "floor-in-DB, direct access safe by
construction" made *checkable*. The spike enumerates the attempts the in-DB floor must reject (§5, row C5):

- an **unsigned or malformed** event;
- a **forged human author** with no attestation token (the agent claiming a clinician vouched);
- a **suppressing event authored un-attested** (an un-vouched machine trying to *hide* signal —
  [ADR-0010](../spec/decisions/0010-additive-vs-suppressing-classification.md));
- a **raw `INSERT`** into the event table, bypassing `submit_event`;
- a **post-hoc salience downgrade** of *another* author's event.

Each must fail closed, with a **legible rejection reason** — a buggy agent should produce a clear error, not a silent
corruption.

---

## 5. PASS / FAIL

| # | Claim | PASS threshold |
|---|---|---|
| **C1** | Additive authorship, un-attested | the advisory commits and reads back with contributor set `{agent, triaged}` and **no** responsibility → "AI-generated / un-vouched" is *emergent* ([ADR-0007](../spec/decisions/0007-authorship-and-accountability.md)); no boolean "is_ai" flag exists anywhere |
| **C2** | Additive, never suppressing | the additive advisory is **accepted**; an otherwise-identical **suppressing** event authored un-attested is **rejected** in-DB ([ADR-0010](../spec/decisions/0010-additive-vs-suppressing-classification.md)) |
| **C3** | Provenance-anchored | the advisory carries the source blob's content-address; re-verifying the blob against that digest succeeds; the reference survives a full `sign → ship → apply` round-trip ([ADR-0013](../spec/decisions/0013-attachments-content-addressed-lazy-blob-tier.md)) |
| **C4** | Version-pinned + recallable | "authored by actor-UUID X under skill-epoch E" returns exactly this advisory; bumping **any** pinned determinant (incl. `skill_epoch`) mints a *new* actor-UUID via `supersede`; a `revoke` overlay marks affected events **without erasing** |
| **C5** | Floor holds against a hostile agent | **every** attempt in §4 is rejected, each with a legible reason; the committed-event set is unchanged after the attacks |
| **C6** *(stretch)* | Surfaces as a notification projection | the advisory appears as a *delta over the log* for the responsible actor, and an acknowledgment is an **append-only audit event**, never auto-satisfied ([ADR-0009](../spec/decisions/0009-notification-economy-salience-routing-and-the-acknowledgment-floor.md)) |

---

## 6. Exit criteria → what gets ratified

- **C1–C5 PASS** → write **two** things, each now *demonstrated* rather than asserted:
  1. the parked **[ADR-0011](../spec/decisions/0011-actor-registry-version-pinning-and-key-custody.md) skill-epoch
     refinement** (skill-epoch as a pinned determinant of an agent actor's identity);
  2. an **ADR for the advisory-actor integration contract** — how an external advisory actor attaches at L2/L3 and
     authors through the floor — promoting the
     [ecosystem 0001](../ecosystem/0001-agent-and-messaging-plugins-kastellan-localmail.md) conclusion from
     *evaluation* to *decision*.
- **Any FAIL is design feedback, not a defect to paper over.** If the floor cannot express "reject
  suppressing-un-attested," that is a gap in the submit surface
  ([ADR-0022](../spec/decisions/0022-validated-submit-surface-the-write-path.md) completeness bet) sent back to
  design. If recall cannot bound to a skill-epoch, the refinement shape is wrong and the parked ADR does not get
  written as drafted.
- **C6** de-risks the [ADR-0009](../spec/decisions/0009-notification-economy-salience-routing-and-the-acknowledgment-floor.md)
  consumer side; its absence does **not** block ratification.

---

## 7. Blast-radius (§9) note

Faithful to the [§9.1](../spec/language-substrate.md) rule, as Spike 0001 was:

- **Safety-critical (in-DB / Rust):** `submit_event` and its validators, the actor registry + version-pinning, the
  additive-vs-suppressing gate, the attestation-token check, the recall/contamination overlay. A defect here can
  silently mis-attribute or admit a forbidden write — the recurring trusted-seam motif.
- **Fit-for-purpose (Python):** the agent stand-in and its urgency score. A defect yields a worse *advisory* a human
  reviews — never a corrupted record — which is the whole reason the advisory tier may iterate fast.
