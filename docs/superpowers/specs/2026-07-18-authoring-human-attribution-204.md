# Design — the authoring-human slice: per-write human authorship on the clinical event (issue #204)

**Date:** 2026-07-18 · **Issue:** [#204](https://github.com/cairn-ehr/cairn-ehr/issues/204)
(2026-07-15 review finding C3, Important/window-closing) · **Plane:** wire core + in-DB floor + node
· **Tier:** safety-critical (§9 Rust + in-DB) · **Vehicle:** one composing ADR (**ADR-0053**) +
spec §3.9/§3.10 prose + an authorship slice on `clinical.medication` · **Spec:** 0.53 → 0.54

## Problem

Every clinical event authored today mints exactly one contributor —
`{actor_id: node_key, role: "recorded"}`, the recording device. There is **no human author** on the
event. The `--attest-as` path ([ADR-0049](../../spec/decisions/0049-commitment-based-sign-off-currency.md))
adds a human's *responsibility* as a **separate** `clinical.medication-attestation.asserted` event.

So the build has **responsibility without authorship** — the exact mirror image of the deployed-EHR
failure [§3.9](../../spec/data-model.md#39-authorship-and-accountability)/[§3.10](../../spec/data-model.md#310-session-identity-event-authorship-and-draft-durability)
diagnoses. By §3.9's own rule — *"an event is AI-generated iff its set contains a non-human author and
no human in a responsibility-bearing role"* — every un-attested medication row reads as **machine-generated
content**, and the [§5.10](../../spec/identity.md#510-authorship-and-responsibility-state-the-consumer-side)
informational floor would render the whole med list as un-vouched machine content.
[ADR-0051](../../spec/decisions/0051-contributor-role-vocabulary-floor-and-responsibility-wire-shape.md)
ratified `recorded` precisely to make that interim reading *honest* (device-recorded, no human authorship
claimed) rather than *illegal* — while scheduling this slice to end the interim before the next clinical
stream.

The load-bearing invariant that closes the gap is [ADR-0008](../../spec/decisions/0008-point-of-care-identity-possession-and-salvage.md)'s
`session.user ≠ event.author`: authorship binds by a per-write *attribution* act, not by whoever holds the
session. It has no implementation and, until this slice, no owning ROADMAP entry — while the *second* half of
the principle-10 split (attestation) shipped first. Each additional device-additive clinical stream deepens
the eventual retrofit; the window is cheapest to close now, on the one clinical stream that exists.

## Decision (approved 2026-07-18)

Ship the **authorship half of principle 10** onto the medication stream. A clinical event can carry an
**authenticated human author** — `{human, "authored"}` + `{node, "recorded"}`, **signed by the human**, the
node sealing the body and holding its DEK (custody) and staying `recorded`. Floor-enforced so the authorship
claim is **unforgeable at the door that mints it**, and **graded, never refused**, at the door that receives
it from the federation.

This realizes `session.user ≠ event.author` at the **data / floor / CLI** layer. ADR-0008's headline UX —
durable session-decoupled **drafts** and the **`sign-as`** stranded-work salvage — presumes a draft store
and a session/UI layer that do not exist yet (the reference UI is a Tauri shell, slice 1); those are the
implementation *above* the invariant and are **explicitly deferred to the UI layer**. The can't-retrofit
pieces — the wire shape and the floor binding — are what this slice lays down.

**Authorship is a grade, not a gate** (ADR-0008): the floor never *requires* a human author. The device-only
path (batch / emergency / no enrolled human present) stays first-class and unchanged.

### The seven ratified decisions

1. **Cut.** The node/floor/CLI realization of `session.user ≠ event.author`. Draft-durability and `sign-as`
   salvage defer to the UI layer.

2. **Binding model — the human author signs; the node holds custody.** Born-sealed already split *signing*
   from *custody*: `seal_and_sign(body, sk)` signs the sealed bytes, `ensure_unwrap_key(client, node_sk)`
   grants the *node* the DEK. So the author (human) **signs** the sealed clinical event while the node
   **seals it and holds the DEK** — `session ≠ author` made cryptographic, with no new token machinery. The
   pattern is already precedent: `identity.link.asserted` is signed by the accepting human
   (`apply_proposal.rs`; `signer_key_id = human_kid`).

3. **Wire shape — authorship only, this slice.**
   ```
   clinical.medication.asserted
     signer_key_id : <human>                              # session(node) ≠ author(human)
     contributors  : [ {actor_id:<human>, role:"authored"},   # signs → authenticated
                       {actor_id:<node>,  role:"recorded"} ]   # seals + holds DEK
     (no responsibility object)
   ```
   The `authored` role is *responsibility-bearing* but carried **without** a `responsibility` object — a
   legitimate "authored, not-yet-vouched" state (§3.9: *absent = un-vouched, a legitimate state*). Per-event
   or per-thread responsibility remains the separate ADR-0049 attestation event, unchanged. Device-only
   fallback is unchanged: node signs, `[{node, "recorded"}]`.

4. **Role — uniform `authored`.** All medication verbs (assert / cease / dose-change / dose-correction /
   reconcile / separate) carry the human as `authored`: they record clinical *statements* about medications,
   not prescriptions. `ordered` / `co-signed` are reserved for a future prescribing/dispensing stream;
   clinical context makes finer distinctions implicit until then.

5. **Strict submit door (db/005) — enforce the authorship binding.** Today's floor has a hole:
   `cairn_check_contributors` checks only role-in-vocab, and `cairn_responsibility_bound` checks only entries
   that carry `responsibility`. So a node could **forge** `{human_X, "authored"}` while signing with a
   *different* key and no token — recording "Dr X authored this" when Dr X never touched it (the
   [#195](https://github.com/cairn-ehr/cairn-ehr/issues/195) hazard, one field over). The binding, generalising
   #195 / ADR-0051 §2 from responsibility to authorship:

   > A contributor whose role is **responsibility-bearing** and whose `actor_id` resolves to a **human**
   > actor must be authenticated as **the event's signer** *or* **a verified attester**
   > (`signer_key_id` or the verified `attester_key`).

   - `recorded` is *contributory* → exempt (the node need not sign); existing device events untouched.
   - Existing attestation / `identity.link` events: human = signer = attester → satisfied.
   - New `{human, "authored"}`: human = signer → satisfied. A forged `{human_X, "authored"}` signed by
     another key with no token → **refused at strict submit**.

   This is the "attribution token" the issue names: for the human-signs case *the signature itself is the
   attribution proof*; a future token-backed author (an author who did not sign — verbal order, AI-scribe)
   is deferred, but the rule already accommodates it (verified attester arm).

6. **Apply door (db/020) — admit and grade; no new refusal.** The apply door already verifies signatures
   in-DB (`cairn_verify`), runs `cairn_check_contributors(..., false)` lenient, and already refuses the
   *provably*-false authorship shapes (a bad or contradictory attester token — "forged human author refused",
   #195). This slice adds **nothing** to the apply door's refusals. It refuses only what it can prove false;
   an *unverifiable* human-author claim (actor ≠ signer, no verifiable token) is **admitted and graded**, not
   refused. See the rationale below — this is the decisive, principle-anchored choice.

7. **Grading — one shared classifier, upgradable.** A single pure predicate
   `classify_authorship_confidence` (the ADR-0051 `classify_role` discipline) grades an event's authorship:
   - **`attested`** — a human author authenticated as the event's signer or a verified attester.
   - **`unverified`** — a human-author claim this node cannot verify (actor ≠ signer, no verifiable credential;
     a forgery, *or* an author authenticated by a scheme this older node cannot parse — the two are
     indistinguishable at the door). Rendered *"authorship claimed, not authenticated here"*, never *attested*,
     never dropped, and **upgradable** when a newer node re-grades it.
   - **`device`** — recorded-only, no human author (the honest device-additive default, ADR-0051).

   This slice ships `attested` + `device`; `unverified` is the apply-side grade. The middle **`asserted`**
   rung (a *named* human author with no key present — verbal/telephone orders) defers with the UI and the
   token-author path.

### Why apply admits-and-grades (the load-bearing rationale)

When a remote med event carries `{human_X, "authored"}`, signed by node_Y (signature verified), with no
verifiable token for human_X, the door **cannot distinguish** a **forgery** (node_Y fabricated it) from a
**future-credential authorship** (a later ADR authenticates authors by a scheme node_Y used but this older
node cannot parse — [ADR-0012](../../spec/decisions/0012-schema-evolution-event-format-and-legibility-across-time.md)
*guarantees* such events arrive). This slice never mints that shape — we only ever mint `author == signer`,
which verifies cleanly and is admitted either way — so the ambiguous shape arises only from a hostile node or
a future node.

- **Paper-parity (the governing argument, §1.2).** On paper, a note initialled "Dr X" that you cannot verify
  is *still in the chart, still readable*; nobody shreds the folder because a signature is illegible — they
  read it and weigh the attribution with suspicion. **Grading-not-refusing is the paper-parity move.** Refusing
  a lawful-but-unverifiable future med event would make a real medication **invisible** to a clinician —
  interaction-checking, the med list, all silently missing it — which is *inferior* to paper (paper never
  makes a med vanish because provenance is doubtful). "A real medication never becomes invisible" is the
  harder patient-safety floor, and it wins.
- **ADR-0012 (never refuse what you can't understand).** The apply door must not refuse an event merely
  because it cannot verify a credential; that conflates *forged* with *authored under a scheme I am too old to
  understand*, and the latter is guaranteed by additive evolution.
- **Consistency with ADR-0051's author-vs-admit asymmetry.** *Strict submit* (local, understands its own
  schema) **enforces** — "author only what you can stand behind." *Apply* (remote, may see future schemas)
  **admits and grades** — "admit whatever verifiably-signed future the wire brings; refuse only the provably
  false." Same shape, one field over.
- **The forgery is attributable, not silent.** A forged remote claim is a *signed* act by node_Y — evidence
  for the [ADR-0018](../../spec/decisions/0018-federation-revocation-cascade-and-the-anchor-as-power.md)
  revocation cascade, and displayed honestly as *unverified*.
- **The residual risk is bounded to one predicate.** A forged row lands in the log, so consumers must honour
  the grade; the mitigation is the standard Cairn move — **one shared `classify_authorship_confidence`** —
  collapsing "every consumer must be disciplined" to "one predicate must be correct." The naive-external-reader
  risk (a FHIR façade ignoring the grade) is the universal graded-field risk (sensitivity, clock-confidence,
  trust-state), not unique here.

*(Correction of an earlier framing: the wedge risk is not a general property of apply-door refusals. A RAISE
in the door's **own admission checks** is caught by `do_pull` and dropped into the durable quarantine pen
(db/021), advancing past it — no wedge. The `81dc025` born-sealed wedge was a **projection trigger** raising
*after* admission, a different code path. So B (apply-refuses) was rejected on the epistemic / paper-parity
grounds above, not on wedge risk.)*

## ADR-0053 content (the ratified decisions)

ADR-0053 — **Per-write human authorship: the authoring signature, the authorship binding, and the
authorship-confidence grade.** Refines ADR-0008 (implements its `session.user ≠ event.author` invariant),
ADR-0007 (the authorship half of the compositional set), ADR-0051 (extends the contributor floor from
responsibility to authorship; reuses the author-vs-admit asymmetry), ADR-0052 (the human signs the
*sealed* body; the node keeps custody), ADR-0012 (grade-don't-refuse under version skew). Resolves #204.
Canonical spec home: §3.9 / §3.10. No new event type; no new envelope field; no schema migration (the
contributor-set fields exist from day one — this fixes an authoring path and a floor binding).

## Node / code slice (all verbs funnel through one path)

- **`cairn-event` body-builders** (`medication/assert|cessation|dose|reconciliation`): accept an optional
  author `(kid, role="authored")`. When present, prepend `{actor_id:<human>, role:"authored"}` to
  `contributors` and set `signer_key_id = <human>`. Absent → today's `recorded`-only shape, node signs.
  Pure functions; still testable in `cairn-event`.
- **`cairn-node::medication::sealed_submit`** (`seal_sign_submit` / `seal_and_sign`): thread an optional
  **author signing key**. When present, `seal_and_sign(body, author_sk)` (the human signs the sealed bytes)
  while **`ensure_unwrap_key(client, node_sk)` stays the node** — custody is the node's regardless of who
  signs. A test pins that we never register the author's key as the node unwrap key.
- **db/005 floor:** add the authorship binding (a new pure predicate, e.g. `cairn_authorship_bound(b, signer,
  attester)`, or an extension of the existing responsibility predicate) wired into the strict door; a Rust
  mirror only if a Rust caller needs it, under the standing drift-guard discipline. **db/020 unchanged.**
- **Grading:** `classify_authorship_confidence` — a pure predicate over `(contributors, signer_key_id,
  verified attester)`. Rust home in `cairn-event::contributor` beside `classify_role`; SQL mirror only where a
  projection needs it (deferred until a consumer does).
- **CLI:** `--author-as <key>` + `--author-passphrase` (env `CAIRN_AUTHOR_PASSPHRASE`), reusing the
  `resolve_attester` enrolled-human load + check (a parallel `resolve_author`). Composes with `--attest-as`:
  the author signs the clinical event; the attester signs the separate ADR-0049 attestation. Absent →
  device-additive, unchanged.

## TDD plan (RED first)

Prove the mechanism on **`assert`**, then roll across cease / dose-change / dose-correction / reconcile /
separate (all share `seal_sign_submit`). RED-first tests:

1. **Forged authorship refused at strict submit** — `{human_X, "authored"}` not signed by human_X, no token
   → db/005 rejects (the safety core).
2. **Human-signed authorship admitted + converges** — `{human, "authored"}` + `{node, "recorded"}`, signer =
   human → submitted, syncs A→B, projection equal.
3. **Device-only still valid** — no `--author-as` → `[{node,"recorded"}]`, node signs, unchanged.
4. **Node keeps custody when the human signs** — the DEK is still wrapped for the node; a rung-3 shred of a
   human-authored event still destroys the body (born-sealed invariant holds under human signature).
5. **Suppression owner-gate recognises the human author** — a human-authored med event resolves its human
   author via `signer_key_id` (`cairn_suppression_author_ok`), so a cross-human suppress of it is refused.
6. **Grade classifier** — `attested` for a human-signed event, `device` for recorded-only, `unverified` for a
   synthesised actor≠signer/no-token event (pure `cairn-event` unit tests).
7. **Apply admits the unverifiable claim** — a remote `{human_X,"authored"}` signed by a node key, no token →
   admitted (not quarantined) and grades `unverified` (guards against a regression that starts refusing it).

## Spec prose touched

- **§3.9** — the authorship binding (a bearing-role human contributor must be signer or verified attester)
  and the authorship-confidence grade; the authored-without-responsibility state made explicit.
- **§3.10** — `session.user ≠ event.author` realized: the authoring signature is the per-write attribution
  act; the node-as-session / human-as-author split; drafts + `sign-as` explicitly deferred to the UI layer.
- **index.md** — spec version 0.53 → 0.54; ADR-0053 in the decision index.

## Out of scope (explicitly)

- Durable session-decoupled **drafts** and the **`sign-as`** stranded-work salvage (ADR-0008 rulings 4/6) —
  UI layer; no draft store or session model exists yet.
- The **`asserted`** grade (a *named* human author with no key present — verbal/telephone orders) and the
  **token-backed author** (an author who did not sign — AI-scribe, dictation) — deferred; the strict binding's
  "verified attester" arm and the grade ladder already reserve room for them.
- **Author + responsibility on the same clinical event** (collapsing the separate attestation for the common
  self-vouch case) — deliberately not this slice; responsibility stays the ADR-0049 overlay.
- Any change to **db/020** apply-door refusal logic, or to the **twin registry** (no new event type).
- Authorship on **non-medication** streams — medication is the only clinical stream; the mechanism is
  stream-agnostic and rolls forward with each new stream.
