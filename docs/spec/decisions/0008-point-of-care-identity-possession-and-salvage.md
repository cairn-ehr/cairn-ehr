# ADR-0008 — Point-of-care identity: possession binding, fast authentication, and work-salvage

- **Status:** Accepted
- **Date:** 2026-06-15

## Context

Two open questions — §11.9 (the *armed write-context* / wrong-chart possession model) and §11.12
(authentication vs. paper-parity on shared workstations) — were treated as separate. They are one
problem: the point-of-care binding of *which patient* and *which clinician* to a write. Paper bound
both in a single physical situation — you, present, holding one patient's folder, pen in hand — and
both bindings were continuously, ambiently visible (the folder label; your physical presence; your
initials at the note's end). Windowing broke the *subject* binding (the wrong-chart misfile);
shared login broke the *author* binding (everyone writing under one session). Restoring them is one
act, not two.

§11.12 had been framed as a trade-off — fast/proximity sessions *vs.* security posture. That framing
is the error, and it is the same shape as the error ADR-0006 corrected for "scope" and ADR-0007
corrected for "signature": **one word is carrying two jobs that run at different frequencies.**
*Authentication* fuses **gatekeeping** (may this person touch the system at all? — coarse, rare, can
be heavy) with **attribution** (who authored *this* event? — fine, per-write, must be paper-cheap).
Deployed EHRs make every access a re-authentication, dragging gatekeeping cost onto every write;
that is unbearable, so clinicians defeat it — shared logins, never logging out, writing under each
other's names. **The audit-trail collapse that "security posture" worries about is *caused by* the
parity violation, not traded against it.** Make per-write attribution sub-second and the rational
incentive to share a login disappears.

Validating cases, from an emergency physician's practice across several health systems:

- **Workstation contention.** In some departments two-to-five clinicians fight for a single
  workstation; a workstation costs less than a few hours of consultant salary, yet the shortage
  burns clinician-hours *every day*. Fast user-switching and work-salvage are therefore the **common
  case**, not an edge case. Software cannot buy hardware, but it can make a switch cost ~0.
- **Documentation rhythm is irreducibly heterogeneous.** In one department, one consultant writes
  after each consult; another batches notes in her rooms; others document live during the
  consultation, or via an AI scribe with copy-paste; and when the system is down, everyone writes
  retrospective batches hours later. No single rhythm may be privileged.
- **Stranded work, and the `[Dr X:]` hack.** A clinician starts a note, is pulled to an emergency or
  finds someone else logged in before auto-lock fired, and the system holds the half-written note
  hostage to the wrong session. The real-world "solutions" are all bad: a hand-typed `[Dr X:]`
  authorship note in the free text (honest but ugly), a save under the wrong author (dishonest and
  dangerous), or losing the work (clinicians have been driven to shouting at, and hitting, the
  computer). This is one of the most common and most enraging frictions in deployed EHRs.

## Decision

The point-of-care identity tension is **illusory**: paper-parity and accountability are the same
requirement, reached by unbundling gatekeeping from attribution and restoring possession. Canonical
home: [identity §5.11](../identity.md#511-point-of-care-identity-possession-fast-authentication-and-salvage).

1. **Unbundle gatekeeping from attribution.** They run at different frequencies and are bound
   separately. The load-bearing infrastructure invariant: **`session.user` and `event.author` are
   independently bindable — the data model must never assume `note.author == session.user`**
   ([data-model §3.10](../data-model.md#310-session-identity-event-authorship-and-draft-durability)).
   This single invariant is what makes everything below possible, and its absence is exactly why
   deployed EHRs cannot salvage stranded work.

2. **Possession binds `(clinician, patient)` in one ambient gesture.** Exactly one chart is *in hand*
   for writing (reading many is free); the write surface carries the patient's colour + persistent
   photo + name/age as the *visual environment*, not a thing to be checked — ambient, peripheral,
   zero cognitive cost, the opposite of a confirmation dialog. The arming gesture is **cheap in time
   but high in distinctiveness** (the precise antidote to reflexive click-through): a deliberate,
   spatially-specific motor act tied to *this* patient — a band-tap at the bedside (the patient's own
   token, paper-exact), or a drag into the single in-hand slot at a workstation. The author binds by
   ambient proximity (token/badge/etc.), never by a login *screen*; presence-driven de-arm, never a
   timer-logout. The gesture must cost the same **cold or warm** — re-arming patient #3 from this
   morning's batch is as cheap as arming the patient in front of you. Confirmation dialogs are
   explicitly **not** the wrong-chart mechanism ([identity §5.8](../identity.md#58-registration-documentation-workflow-normative), principle 3).

3. **Authentication exists; its three pains are removed.** The earlier "no login screen ever" framing
   is wrong — authentication is infrastructure and must be provided. What clinicians hate is not the
   gate but three pains stacked on it; each is removed by an operational principle that is a
   *corollary of an existing founding principle*, not a new axiom:
   - **Never make the user wait if engineering can avoid it** (the latency limb of paper-parity,
     principle 3; [vision §1.2](../vision.md#12-the-paper-parity-test-normative)). The gate is
     perceptually instant — MRU-defaulted user selector, type-a-few-chars-and-enter, **no spinner**,
     heavy work done in the background while the clinician already writes, and **cache-and-hide, not
     cache-and-clear** so re-display is instant where resources allow. Instant re-auth is the
     *precondition* that makes presence-driven auto-de-arm parity-legal: auto-lock is only not a
     regression because the re-arm after it is free.
   - **Always a fallback — no dead-ends, no IT dependency** (an availability + paper-parity corollary,
     principles 5 + 3). A resilience ladder — badge → password → self-recovery (security-Q / SMS /
     recovery codes) → **audited break-glass** — every rung self-service, bottoming out in the
     existing partition-honest break-glass primitive ([identity §5.9](../identity.md#59-sensitivity-grade-the-safety-projection-and-break-glass-visibility-scope)),
     because "forgot everything during a 3 a.m. partition with no IT and no network" must still
     resolve. Recovery is break-glass for the auth layer. This is the severity-ladder motif recurring
     a third time (erasure ladder, disclosure-coarsening ladder, now an auth-resilience ladder).
   - **Never make the user redo work already done** (work-preservation: append-only, principle 1,
     extended to the *pre-commit* side of the commit boundary; and identity-repair, principle 2,
     applied to the *author*). See ruling 4.

4. **Stranded work is salvaged by identity-repair, not a wall — `sign-as` is the default.** Because
   `session.user ≠ event.author`, a clinician whose draft is stranded in the wrong session resolves
   it with a trichotomy: **sign-as** (attribute *this note* to me, authenticate me as author, session
   untouched — the default, because forcing a switch makes *two* people redo work), **switch** (change
   the session, explicit), or **stay**. `sign-as` is fundamentally about rescuing **your own** stranded
   work, where note-level attribution is exactly correct; it requires authenticating *as* the claimed
   author, so it is strictly *more* honest than today's silent-session-author save. The audit log
   records *drafted-in-session-of-A, signed-by-B-via-sign-as* — append-only, the acknowledged-
   uncertainty path (provisional authorship resolved by overlay) made concrete. This is the existing
   identity-repair philosophy — *prevention cannot be complete, therefore repair is first-class,
   cheap, fast, forensically clean* ([identity §5](../identity.md#5-identity-subsystem)) — applied to
   the authoring act: the backstop for the inevitable imperfection of presence-driven locking.

5. **Authorship is note-level; span-granular authorship is rejected.** A note is one event with a
   note-level contributor set (ADR-0007 unchanged); authorship of *spans within* one note is **not**
   modelled — it would complicate every note to serve a rare edge case (one author types part, a
   second finishes and signs). In the dominant salvage case the whole note genuinely is the signer's
   work, so note-level attribution is correct, not a compromise; the rare cross-author case retains
   the cheap free-text `[Dr X:]` escape hatch. Where structural truth matters there, authorship (who
   typed) and attestation (who vouches) already separate cleanly (ADR-0007 / [security §7.2](../security.md#72-signing-attestation-and-ai-agent-identity)).

6. **Make contention cheap — the software's answer to the workstation shortage.** Collapse the
   per-switch tax to ~0 so a shared station approximates *N* private ones, restoring the paper desk
   where several clinicians each held their own folder at once: a station may hold **multiple warm,
   resident, hidden `(clinician, patient, draft)` contexts**, each kept alive by its owner's token and
   surfaced (one displayed at a time, for confidentiality + focus) by proximity. This is
   cache-and-hide taken to its end and is bought by the same invariant as ruling 1 — the draft/context
   store is keyed by `(author, patient)`, not by the session. Resource-bound, hence policy/hardware-gated.

7. **Rhythm-agnostic by construction.** Live, after-each-patient, batch-much-later, AI-scribed, and
   forced-retrospective documentation are all first-class: bitemporal time already absorbs them
   (`t_effective` freely backdated, clash-flagged against the `t_recorded` ceiling;
   [data-model §3.6](../data-model.md#36-bitemporal-event-time-recording-time-vs-effective-time)), and
   the arming gesture's cold = warm cost (ruling 2) means a six-hours-late batch is not a degraded mode.

8. **Mechanism, not policy (principle 9), and resource-proportional (principle 4).** Cairn ships the
   possession primitive, the proximity/token session model, ambient identity display, the
   authorship-confidence grade, patient-bound clipboard payloads (cross-context paste is detectable
   *structurally* — the content carries its origin `patient_uuid` — not heuristically), and the
   `session ≠ author` + durable-draft invariants. Deployment selects token tech (NFC / BLE / phone /
   pluggable biometric / plain local credential), which ladder rungs exist, whether unattributed
   writes are permitted, the de-arm threshold, and whether `sign-as` or a forced switch is offered.
   The possession primitive **degrades to no special hardware** — a Pi clinic with no badges still
   gets the on-screen in-hand slot, a local credential, ambient display, singular arming, and no
   network gate; token hardware *enhances* possession, never a requirement.

**Authorship-confidence is a grade, not a gate (acknowledged uncertainty, principle 4).** Where author
identity cannot be cheaply established (badge forgotten, two badges in range, emergency), the system
does **not** block — it records honestly and refines by overlay: *attested* (token present,
cryptographically signed), *asserted* (named, token absent/ambiguous, raised later), *unattributed*
(authored-at-station-X, identity unknown — never a guess). This composes into the existing
chart/event trust projection ([identity §5.7](../identity.md#57-identity-event-algebra-closed-set-all-append-only-syncable-auditable) / [§5.10](../identity.md#510-authorship-and-responsibility-state-the-consumer-side)) — no new stream. Passive proximity only
*narrows* candidates; the explicit arming gesture *selects* — **proximity is a hint, not an
authority**, echoing ADR-0004.

## Consequences

- **Easier:** the security win comes *from* the parity win, not in tension with it — sub-second,
  lossless switching removes the incentive to share logins, so honest per-write attribution and
  paper-parity are achieved together. The most enraging deployed-EHR friction (stranded work) gets a
  clean, audited, append-only repair primitive that replaces all three bad current hacks. Heterogeneous
  documentation rhythms, AI scribes, and contended stations fall out of primitives already in the spec
  (bitemporal time, contributor set, patient-bound clipboard, break-glass, trust projection) — no new
  founding principle and no new event stream.
- **Harder / new trusted surface:** the `(clinician, patient)` binding and the authorship stamp are
  safety-critical (a defect mis-binds the subject or mis-attributes the author) → they belong in the
  small Rust/in-database trusted surface alongside the identity algebra ([§9 blast-radius](../language-substrate.md)); the
  proximity/UI layer (badge/BLE reading) is fit-for-purpose (a defect shows the wrong name *ambiently*
  and is caught instantly). The seam between them — UI proximity event → authoritative authorship stamp
  — is the one safety-critical path, structurally like the §5.9 seal-time projection seam. Durable,
  session-decoupled drafts are new local state to design and protect.
- **The bet:** that possession + instant proximity auth + salvage clears the paper-parity benchmark at
  ED pace without degrading into reflexive click-through, and that the `session ≠ author` invariant is
  enough infrastructure to let policy/UI do the rest. We would know the bet is wrong if the arming
  gesture proves too slow cold (the batch case), if presence-driven de-arm creates a *new* friction
  (locking on a step-back to think), or if real deployments need span-granular authorship often enough
  that the note-level simplification (ruling 5) hurts.
- **Policy-neutral (principle 9):** Cairn provides the possession, proximity-auth, fallback-ladder, and
  salvage *mechanisms*, and takes no side on token technology, which rungs are offered, or whether a
  given context blocks unattributed writes.
