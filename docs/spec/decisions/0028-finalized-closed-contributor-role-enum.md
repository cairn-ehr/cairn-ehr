# ADR-0028 — The finalized closed contributor-role enum

- **Status:** Accepted
- **Date:** 2026-06-20
- **Refines:** [ADR-0007](0007-authorship-and-accountability.md)

## Context

[ADR-0007](0007-authorship-and-accountability.md) replaced the single `author` field with a **contributor
set**, each entry `{ identity, role, descriptor?, responsibility? }`, and made `role` a **closed core enum**
partitioned into *responsibility-bearing* and *contributory*. It listed an initial membership but explicitly
**deferred final ratification** — the [open-questions](../open-questions.md) follow-on records *"the bearing/
non-bearing partition is settled; the exact member list is to be finalised in `data-model.md` (`dictated`,
`reviewed`, `co-signed` are candidates)."* This ADR closes that follow-on.

The role enum is a **safety primitive**, not cosmetics. The apply layer branches on the bears-vs-doesn't
partition (the [ADR-0010](0010-additive-vs-suppressing-classification.md) suppressing-operation owner-gate
refuses an un-owned suppressing output by checking for a responsible bearing contributor); the *"AI-generated"*
reading is the structural *"the set contains a non-human author and no human in a responsibility-bearing role."*
So the set must stay **small and closed** — the same merge-policy / identity-event-algebra
([identity §5.7](../identity.md#57-identity-event-algebra-closed-set-all-append-only-syncable-auditable)) /
suppressing-operation discipline: the safety/DB layer reasons over an unambiguous fixed vocabulary, and
unbounded nuance is pushed to the **free-text descriptor**, which no safety logic branches on.

That gives the **bar for membership**: a role earns an enum slot **only if the safety/DB layer or hard policy
must branch on its responsibility semantics.** A distinction that is mere flavor belongs in the descriptor; a
workflow/gating distinction is **policy** ([principle 9](../index.md#founding-principles-the-lens-for-every-decision));
*"I saw it but do not vouch"* is an **acknowledgment** event ([identity §5.12](../identity.md#512-the-notification-economy-salience-responsibility-routing-and-the-acknowledgment-floor),
[ADR-0009](0009-notification-economy-salience-routing-and-the-acknowledgment-floor.md)), not authorship. Applying
that bar to the three parked candidates (plus one the case-mining surfaced) is the whole decision.

## Decision

Ratify the **closed contributor-role enum** as **eleven members**, partitioned as in
[ADR-0007](0007-authorship-and-accountability.md). Canonical home:
[data-model §3.9](../data-model.md#39-authorship-and-accountability). A free-text **descriptor** rides alongside
every entry; **no safety logic branches on it.** **No new founding principle** (this is the mechanism of
principle 10); **no schema migration** (the `role` field and descriptor existed from day one — this fixes the
closed value set).

- **Responsibility-bearing (6):** `authored`, `ordered`, `attested`, `co-signed`, `witnessed`, `dictated`.
- **Contributory (5):** `drafted`, `transcribed`, `graded`, `triaged`, `suggested`.

Three candidates **added** (all bearing), one **rejected**:

1. **`co-signed` (bearing) — added.** A supervisory countersignature endorsing another contributor's authorship
   (registrar → consultant; NP/PA → supervising physician). It *is* representable as a second `attested`
   contributor in the set, but it earns a first-class slot because **deployments gate on it** — a trainee's note
   may be policy-*pending until co-signed* — and that gating, and the responsibility-state projection
   ([identity §5.10](../identity.md#510-authorship-and-responsibility-state-the-consumer-side)), must branch on
   *supervisory* responsibility without parsing free-text. Distinct from a peer's independent `attested`.

2. **`witnessed` (bearing) — added.** Attests that an event **occurred or was observed** — *not* that record
   content is vouched-for. A distinct accountability (occurrence/observation), legally first-class in practice:
   controlled-substance waste, consent, restraint application, verbal-order read-back, death verification. The
   safety/medico-legal layer treats "who witnessed" differently from "who attested the content," so it branches.

3. **`dictated` (bearing) — added.** The human **source of clinical content by voice**. Bears responsibility for
   the clinical **intent**; the **verbatim text** passes through a `transcribed` contributor (a human scribe or
   ASR agent) and carries a **transcription-accuracy gap** — the documented *"no"*/*"now"* and drug-name ASR
   hazards — until separately verified (a `{dictator, attested}` overlay, or an honest *transcription-unverified*
   responsibility-state, [§3.7](../data-model.md#37-acknowledged-uncertainty-uncertainty-capable-value-types)).
   Distinct from `authored`, where the author owns the **exact words** as written. `dictated` pairs with
   `transcribed`; the safety value is making the *unverified verbatim* state branchable rather than silently
   reading dictation as fully-authored text.

4. **`reviewed` — rejected.** It silently means one of two things already modeled, and admitting it would
   **re-fuse the signature ≠ attestation split** ADR-0007 deliberately separated: if review *confers
   responsibility*, it is `attested` (bearing); if it is *"saw-but-not-vouching,"* it is an **acknowledgment**
   ([ADR-0009](0009-notification-economy-salience-routing-and-the-acknowledgment-floor.md)), not an authorship role.
   Its nuance, where wanted, is a descriptor on `attested`.

**Boundary clarified (prevents the next round of candidates):** these roles describe **contribution to the
record**, *not* performance of the **clinical act**. So `performed` is **out of scope** (it is clinical content —
who did the procedure — carried in the body, not a record-contribution role); `ordered` sits *on* the line and is
kept because ordering is simultaneously a record act and a clinical request whose responsibility routing the
system must reason over.

**Extension discipline (unchanged, now explicit):** the enum is **closed and additive-only.** A new member is a
deliberate, ADR-recorded act ([ADR-0012](0012-schema-evolution-event-format-and-legibility-across-time.md)
additive evolution), never an ad-hoc addition; the **default home for a new distinction is the descriptor**, and
the burden is on demonstrating the safety/policy layer must branch on it.

## Consequences

- **Easier.** Hard policy and the [identity §5.10](../identity.md#510-authorship-and-responsibility-state-the-consumer-side)
  responsibility-state projection can branch directly on supervision (co-sign gating), occurrence-attestation
  (witness), and the dictation/transcription gap — without parsing free-text descriptors. The structural
  *"AI-generated"* reading and the [ADR-0010](0010-additive-vs-suppressing-classification.md) suppressing-operation
  owner-gate **extend unchanged**: more human bearing roles simply count as *"a human vouches."*
- **Harder / trusted surface.** Each bearing role is part of what the apply layer and hard policy reason over, so
  their semantics must stay **stable and additive-only** — a [§9.1](../language-substrate.md#91-selection-rule-by-defect-blast-radius)
  blast-radius concern when implementation begins. `dictated` introduces an explicit transcription-verification
  state to surface and track.
- **The bet.** That 6 + 5 covers the responsibility-distinct contribution roles real clinical practice needs, and
  that future distinctions are descriptor / policy / acknowledgment rather than new enum members. We would know it
  is wrong if a real workflow needs the safety layer to branch on a contribution role not in the set — which is
  itself the signal to record a superseding ADR, never to widen the set informally.
- **Closes** the [ADR-0007](0007-authorship-and-accountability.md) deferred *closed role-enum membership*
  follow-on. The remaining ADR-0007 follow-on (`on_behalf_of` proxy/liability semantics) is deliberately out of
  scope — jurisdictions interpret the chain Cairn records. No new founding principle; no new event stream; no
  schema migration.
