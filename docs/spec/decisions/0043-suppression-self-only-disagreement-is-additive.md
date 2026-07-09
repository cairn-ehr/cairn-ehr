# ADR-0043 — Suppression is self-only; disagreement is additive

- **Status:** Accepted
- **Date:** 2026-07-09
- **Refines:** [ADR-0010](0010-additive-vs-suppressing-classification.md) (the suppressing owner-gate),
  [ADR-0022](0022-validated-submit-surface-the-write-path.md) (the write-path step-5 hard-policy gate)

## Context

The validated write floor had an owner-gate hole flagged by the 2026-07-02 comprehensive review
(finding A10) and tracked as [#99](https://github.com/cairn-ehr/cairn-ehr/issues/99). A suppressing
overlay that forecloses on a specific prior event — `salience.downgrade` / `visibility.suppress`, the
only two `suppressing + targets_other_author=TRUE` types — was admitted once *some* enrolled human
attested it (write-path step 4) and the target event existed (step 5). Nothing constrained **whose**
event was being suppressed: any enrolled clinician could downgrade or hide any other author's clinical
content. `db/005_submit.sql` marked this `DEFERRED`, calling real owner/authority semantics "an
ADR-level design question."

[ADR-0010](0010-additive-vs-suppressing-classification.md) had already established *conservation of
responsibility* — a suppressing operation is never truly un-owned; accountability sits either at the
event's responsible contributor or at the explicit, audited configuration act that permitted a class of
un-owned suppression. That gives **accountability** (someone answers for the suppression) but not
**entitlement** (who is allowed to suppress *this* event). The 2026-07-02 review framed unrestricted
cross-author suppression — any attested human silently burying or demoting any other author's entry —
as a bug, not a feature: on paper, a clinician cannot un-write a colleague's ink, only add their own.
This ADR resolves the deferred question and closes the last open sub-item of #99 (the other three —
the recall-epoch join, the `recall_overlay` FK, and the `actor_current` wall-clock tiebreak — were
already fixed on 2026-07-02).

## Decision

**A suppressing overlay that targets another *human author's* event is refused at the floor.
Suppression of human-authored content is self-only. Disagreement with another author's content is
expressed additively** — a note or advisory that references their event, never one that touches it —
**never by suppressing their event. Agent-authored / un-owned advisories (no responsible human) stay
dismissable by any enrolled human** — that is the clinician-overrides-the-machine path, not the burying
of a colleague.

1. **Principle-correct, not a compromise.** Reaching into another author's content to hide or demote it
   is the append-only violation (principle 1); the correct move is to overlay *your own* view
   additively. Read correctly, paper-parity (principle 3) forbids un-writing a colleague's entry —
   striking it to illegibility is exactly the silent-falsification malfeasance paper-parity excludes —
   so a permission/notification gate to *enable* cross-author suppression would have been machinery in
   service of a violation, not a safety feature. Self-only makes the suppression's owner and the
   authored-content's owner the same person, the tightest form of ADR-0010's conservation of
   responsibility. **Self-suppression stays allowed**: the digital equivalent of an author drawing a
   line through their own erroneous entry and initialling it — the original stays in the log, legible
   under the overlay.

2. **"Author" means *human* author (principle 10).** The rule protects **human-authored** content only.
   An agent-authored advisory ([ADR-0030](0030-advisory-actor-integration-contract.md);
   [ADR-0007](0007-authorship-and-accountability.md), principle 10: authorship is compositional but an
   agent bears no responsibility) is the machine's suggestion, not a colleague's clinical judgment — a
   clinician dismissing it via `salience.downgrade` / `visibility.suppress` is the intended
   *human-overrides-the-machine* workflow, not the burying of another author. Suppressing an un-owned
   advisory therefore stays open to any enrolled human.

   A target is **human-authored** iff its `signer_key_id` was *ever* enrolled or superseded as a
   `kind='human'` actor — resolved from the **append-only `actor_event` history**, not the mutable
   `actor_current` view — **or** it carries a stored human attestation (`event_log.attester_key IS NOT
   NULL` — the floor only ever stores a `kind='human'` attester key). The target's set of human authors
   is `H = {signer_key_id if it was ever a kind='human' actor} ∪ {hex(attester_key) if attester_key IS
   NOT NULL}`. **`H` empty ⇒ dismissable by any enrolled human; `H` non-empty ⇒ self-only** (the attester
   must be a member of `H`). Resolving the signer against **history rather than `actor_current`** is
   load-bearing: authorship is an immutable historical fact, so a departed or key-rotated author whose
   original key has since dropped out of `actor_current` (revoke, or supersede onto a new key) must stay
   in `H`. Querying `actor_current` would silently empty `H` and flip the gate open for *any* enrolled
   human — over-permission on the safety floor, contradicting this ADR's never-over-permission invariant.
   `actor_event` is append-only, so the branch is monotonic: a key that was ever human stays human for
   this check forever (wrong direction is over-refusal, never over-permission). This is computed from
   stored columns plus one registry lookup — no fragile contributor-JSON parsing.

3. **Both doors, one shared helper (principle 12).** The gate is implemented once as
   `cairn_suppression_author_ok(target_event_id, attester_key) RETURNS boolean` (STABLE, defined ahead
   of `submit_event` in `db/005_submit.sql`) and called identically from both write-time seams:
   `submit_event` (`db/005_submit.sql`) and the remote-apply door (`db/020_apply_remote_event.sql`).
   A single floor function, called from both doors, means a **replicated** cross-author suppression
   faces the same refusal a locally-authored one does — a peer cannot launder a cross-author suppression
   in over the wire. (Completeness at the apply door is bounded by the receiving node's ability to resolve
   the target's human authorship: unconditional when the target carries a stored `attester_key` — that
   column travels *with* the event, so every node computes the same `H` — but for a plain-signed human
   note it depends on that node having learned the author's `kind='human'` enrolment. That residual is the
   node-local-registry limitation `apply_remote_event` already carries, tracked as
   [#154](https://github.com/cairn-ehr/cairn-ehr/issues/154); see the caveat under Consequences.) This is
   the same "uniform core" discipline
   ([index principle 12](../index.md#founding-principles-the-lens-for-every-decision)) already used for
   the legibility-twin hook ([ADR-0039](0039-globalise-authored-legibility-twin.md)).

   The gate fires only when the event is `suppressing`, `targets_other_author = TRUE`, and the payload
   carries a `target_event_id` — the same predicate the [§9.6](../language-substrate.md#96-the-validated-submit-surface-the-write-path)
   write path already used to reach the target-existence check.

## Consequences

**Easier / now guaranteed:**
- The owner-gate hole in #99 is closed: a suppressing overlay can no longer bury or demote another human
  author's event without their own attestation.
- Disagreement between clinicians is always visible in the record — an author's entry can be
  disagreed-with, but never made to disappear or fade from under them.
- One helper, two call sites — no drift is possible between the local-author door and the remote-apply
  door.

**Harder / accepted costs, deliberate divergences:**
- **Deliberate divergence from [ADR-0010](0010-additive-vs-suppressing-classification.md) §2.**
  ADR-0010 classified *demotion* as additive ("only hiding-to-nothing or auto-deciding is
  suppressing" — demotion needs no owner). This ADR keeps `salience.downgrade` in the **gated
  (self-only)** set when it targets another author's event anyway: demoting another author's content
  still *de-facto buries* it (ADR-0010 §5's own concern about automation complacency and de-facto
  suppression), so cross-author demotion is refused on the same footing as cross-author hiding. This is
  the maintainer's clinical call, recorded here as an intentional narrowing of ADR-0010's demotion
  boundary for the specific case of *another author's* event; `db/005_submit.sql` already classified
  `salience.downgrade` as `suppressing`, so no reclassification of the type itself was needed.
- **The [§5.9](../identity.md#59-sensitivity-grade-the-safety-projection-and-break-glass-visibility-scope)
  sensitivity-sealing carve-out.** Sealing a sensitive episode genuinely hides cross-author content, but
  through a **separate, specifically-authorized mechanism** ([ADR-0006](0006-visibility-scope-replication-and-the-safety-projection.md))
  that emits a de-identified, severity-graded safety projection so nothing safety-relevant is lost. That
  is a distinct, independently-authorized path — not the generic clinician `visibility.suppress` overlay
  gated here. This ADR governs only the generic suppression overlays; §5.9 keeps its own authority path,
  out of scope for this decision.
- **`repudiate` (`identity.repudiate.asserted`) is untouched.** It is `suppressing` but
  `targets_other_author = FALSE` — it is value-grained (it strikes a known-false *name*, not a target
  *event*). Because the gate fires only when `v_targets_other AND` the payload carries `target_event_id`,
  `repudiate`'s `targets_other_author = FALSE` classification structurally excludes it from ever reaching
  the gate's `v_targets_other` predicate; this is exercised by
  `crates/cairn-node/tests/identity_repudiate.rs`.
- **Safe-refuse direction.** A same-human-different-key case reads as cross-author and is refused — the
  safe direction, since the clinician re-expresses as a note (mild friction, never a silent bury). The
  failure mode of the whole gate is **over-refusal on human-authored content, never over-permission** —
  with one bounded exception, next.
- **The apply-door gate inherits the node-local-registry limitation
  ([#154](https://github.com/cairn-ehr/cairn-ehr/issues/154)).** For a target carrying a stored human
  `attester_key`, `H` is registry-independent — the column travels with the event, so every node computes
  the same `H` and the refusal is unconditional. But a **plain-signed human note** (no self-attestation,
  `attester_key IS NULL`) is human-authored *only* by its signer's `kind='human'` enrolment, resolved
  against the **receiving node's** local `actor_event`. A node that has not yet learned that enrolment
  computes an empty `H` and would *admit* a cross-human suppress its origin refused — the one direction in
  which this door can over-permit. This is the same limitation `apply_remote_event`'s attester check
  already carries (its header's "local registry, by design for now"); the origin always refuses, and it
  closes once actor enrolment reliably federates ahead of the events that cite it. Tracked as #154, not
  left implicit (house rule 5).
- **No role hierarchy, care-team, or delegation model introduced**, and none is needed — the answer is
  "no cross-author suppression," not "cross-author suppression under authority X." No notification tier
  is introduced either: there is nothing to notify about, since the suppression simply does not happen —
  the disagreeing clinician authors their own note through the normal additive path.

**How we would know the bet is failing:** a real clinical workflow needs a *legitimate* form of
cross-author suppression this ADR forecloses (e.g. a supervising clinician who must be able to demote a
trainee's entry under some accountability structure) — that would need a new, explicitly-authorized
mechanism analogous to §5.9's sealing, not a reopening of this gate; or the self-only rule proves too
strict in practice (clinicians routinely hit the refusal and file it as friction rather than re-express
additively), which would be visible in the audit log as a spike in refused-suppression attempts.

**Not a new founding principle.** This is principle 1 (append-only, never erase — always overlay) and
principle 10 (authorship is compositional; an agent bears no responsibility) applied to the suppression
overlay's entitlement question, plus the §9/principle 12 one-floor-two-doors discipline
[ADR-0039](0039-globalise-authored-legibility-twin.md) already established.
