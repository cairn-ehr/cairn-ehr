# ADR-0039 — Globalise the authored legibility twin (honest-degradation floor)

- **Status:** Accepted
- **Date:** 2026-06-28
- **Refines:** [ADR-0012](0012-schema-evolution-event-format-and-legibility-across-time.md) (principle 11 / legibility across time),
  [ADR-0034](0034-demographic-legibility-twin.md) (generalises the demographic carried twin to all event classes)

## Context

[Principle 11](../index.md) requires every event to stay human-readable for as long as it exists,
via a signed plaintext legibility twin materialised by the author — who understands the schema —
and carried forward, never re-derived by a reader that may be generations behind ([§3.13](../data-model.md#313-schema-evolution-event-format-and-the-legibility-twin)).
[ADR-0034](0034-demographic-legibility-twin.md) applied this to demographic assertions. But every
other event type still fell through the validated door's `cairn_event_twin` hook to a
*receiver-derived* skeleton (`cairn_twin_skeleton`), which is the legibility-across-time hole the
spike left open. Making the authored twin simply mandatory at the floor would reject a twin-less
event from an older or non-conformant peer — breaking set-union convergence (principle 1,
availability over consistency, and ADR-0012's no-lockstep heterogeneous fleet).

## Decision

The author-materialised twin is **global**: every conformant author renders and signs a §3.13 twin
into the body, for every event type. The in-DB floor **prefers** the authored twin and, when it is
absent or blank, **degrades honestly** — it stores the event with a mechanically-derived skeleton
twin (now rendering the payload, not just a header) rather than rejecting it. Convergence is
preserved; the derived twin is a non-authoritative local projection.

**Authored-vs-derived is not stored.** Because the immutable signed body either carries a non-empty
`plaintext_twin` or it does not, the distinction is a derivable read-time projection of
`signed_bytes` (`cairn_twin_is_authored`), exposed via the `event_twin_provenance` view for a future
re-authoring / duplicate-sweep / audit worklist. No new column, and the validated `submit_event`
door is not re-declared — only its `cairn_event_twin` hook changes.

**Demographic exception (unchanged from [ADR-0034](0034-demographic-legibility-twin.md)):** the two
demographic assertion types keep a *hard* authored-twin requirement. A twin-less demographic event
cannot come from an older peer (an older node rejects the unknown demographic type at classification),
so its absence is a same-version bug and is rejected.

## Consequences

- **Easier:** every event is legible from its author's faithful rendering across arbitrary schema
  skew; the skeleton survives only as an honest, flagged fallback; the `cairn-event::plaintext_twin`
  renderer is repositioned as the canonical generic authoring renderer (one reusable function).
- **The trade:** a twin-less event still stores a crude, receiver-derived twin — but it is flagged
  (`twin_authored = false`), so a reader/sweep can tell faithful from best-effort, and a conformant
  author never produces one.
- **Two-place rule (accepted):** "prefer non-empty authored, else derive" lives in SQL (the floor)
  and Rust (`resolve_twin`, used by cairn-sync), each unit-tested, cross-linked by comment. A future
  option is to unify it into a single pgrx function at the cost of an extension rebuild.
- **Out of scope:** the §3.13 `rendered-by` (schema + renderer version) stamp; per-type prose
  renderers for the placeholder clinical types; routing `cairn-sync` through `submit_event`.
