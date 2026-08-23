# Design — narrow the custody, never the reach (§5.9 part C)

- **Date:** 2026-08-23
- **Issues:** designs [#376](https://github.com/cairn-ehr/cairn-ehr/issues/376) (§5.9 part C —
  sequester / custody narrowing) and merges it with
  [#377](https://github.com/cairn-ehr/cairn-ehr/issues/377) (part D — break-glass), whose stated
  dependency direction this design **reverses**
- **Unblocked by:** [#231](https://github.com/cairn-ehr/cairn-ehr/issues/231), closed — the serve door
  now pins the unwrap-cert `kid` to `trust_peer` ([ADR-0052](../../spec/decisions/0052-born-sealed-clinical-bodies.md)
  erratum E1). #376's hard block is lifted.
- **Canonical spec home:** [identity §5.9](../../spec/identity.md#59-sensitivity-grade-the-safety-projection-and-break-glass-visibility-scope)
- **Builds on:** [ADR-0052](../../spec/decisions/0052-born-sealed-clinical-bodies.md) §4 + E1
  (*withhold the key, never the bytes*) · [ADR-0062](../../spec/decisions/0062-the-sensitivity-stream-and-the-inverted-unknown.md)
  decisions 2, 4, 8, 9 · [ADR-0063](../../spec/decisions/0063-the-safety-projection-and-the-seal-as-coarsening-boundary.md)
  decision 2 and its blast-radius argument · [ADR-0064](../../spec/decisions/0064-admit-the-claim-withhold-the-power.md)
  (the authority floor this design inherits rather than re-writes, and whose handoff finding it corrects) ·
  [ADR-0005](../../spec/decisions/0005-erasure-key-custody-and-crypto-shredding.md) (custody, not deletion) ·
  [ADR-0004](../../spec/decisions/0004-dynamic-sync-scope-prefetch-not-authority.md) (the key-acquisition
  trichotomy break-glass already rides) · [ADR-0012](../../spec/decisions/0012-schema-evolution-event-format-and-legibility-across-time.md)
  (additive evolution; schema generation is a local node property)
- **Will produce:** ADR-0065, a §5.9 spec revision, and a C1 implementation plan

---

## 1. What was actually open

§5.9 part C is the first part of the sensitivity subsystem that *enforces* anything. Parts A and B
compute and report. Part C narrows a sealed body's key custody, so that a grade stops being advisory.

Three questions were open, and one argument had been handed forward:

1. **Which subject feeds which dial** — may a chart-wide grade narrow custody on every event of the
   chart, or only coarsen the projection?
2. **The interaction with [ADR-0062](../../spec/decisions/0062-the-sensitivity-stream-and-the-inverted-unknown.md)
   decision 9's conservative bound.**
3. **Re-wrap mechanics** over the existing custody plane, and what happens to a node that already
   fetched the DEK.

And ADR-0064's handoff: *a custody dial derived from the effective grade is only as strong as its
most-custodial holder*, because the grade is node-relative — with an explicit signed custody act
offered as the alternative, explicitly **not decided**.

## 2. The reframe that settled it

The design did not resolve derived-vs-explicit on its own terms. It was settled by asking what
sequester is *for*, and the answer changed the invariant:

> **Custody narrowing changes the cost and the noise of reading. It never changes whether the content
> can be reached — at a node that holds the key, or that can reach one.**

*(The bound is a PR-review correction. The first draft stated it unbounded, which is false at rung 1
under partition — see §3, §11 and §14, and [#498](https://github.com/cairn-ehr/cairn-ehr/issues/498).)*

Three findings, in the order they arrived:

**(a) Node custody is the norm; per-clinician custody is the exception.** A blanket per-clinician
custody policy would cause unbearable friction within a location and make normal work impossible. In an
ED the team reads the chart. So the ladder has two narrowing rungs, not one, and the cheap one is the
default.

**(b) Break-glass must stay rare to stay meaningful, and it stays rare *because* node custody is the
norm.** At a holder node, reading sensitive content is ordinary work with no ceremony. Break-glass fires
only off-ladder — you are at a non-holder node, or you are not one of the named clinicians. An earlier
draft of this design made break-glass the route for the *normal* case; that is §5.11's
confirmation-dialog disease and §5.12's alert fatigue in one, and it was wrong.

**(c) A universal, audited break-glass keyring, whose use notifies patient, custodian and location.**
This is the piece that makes the rest safe. It means no narrowing can produce unreachable clinical
content **wherever the keyring reaches** — outright at rungs 0 and 2, and at rung 1 whenever a holder is
reachable — so **recoverability and confidentiality stop being in tension** and the keyring *is* the
escrow. The remainder is real and is [#498](https://github.com/cairn-ehr/cairn-ehr/issues/498): a rung-1
break-glass is a network act (§3), so a partitioned non-holder still cannot reach the content. Review
finding; the first draft of this section claimed the unbounded form.
It is also paper's actual mechanism: the sealed envelope opens with your fingers, and the torn flap is
what everyone sees.

## 3. The structural finding: no new key material is needed

Working (c) through the existing plane produced the result that shrinks this slice to something
buildable.

`event_dek` holds this node's wrapped copy of each event's DEK
([db/037_born_sealed.sql:43](../../../db/037_born_sealed.sql#L43)), and it is `REVOKE`d from `PUBLIC`
and `cairn_agent`, granted only to `cairn_node`. Therefore:

- **Rung 2's break-glass is local**: the DEK is already wrapped to this node. "Breaking glass" is the
  in-DB floor letting an actor through and writing the audit row in the same transaction. No key moves.
- **Rung 1's break-glass is a network act**: a non-holder node asks a holder for the DEK with an
  audited justification — the [ADR-0004](../../spec/decisions/0004-dynamic-sync-scope-prefetch-not-authority.md)
  acquisition trichotomy §5.9 already names (*"from sibling/parent on reconnect"*). No new key tier.

**The node's own DEK is the keyring, and the floor is the glass.** The break-glass keyring is not a new
key-management surface at all.

Two consequences follow immediately:

- **Part C and part D are not separable.** A narrowing without the audited break-glass path creates
  content nobody can reach. #377 saying *"blocked on C"* has the dependency backwards: **the glass has to
  exist before anything is sealed behind it.**
- **The keyring must be local, not a remote provider.** A keyring reached over the network fails at 3am
  in a partitioned remote clinic — an availability failure on the safety path, which
  *availability over consistency* forbids. Break glass locally; the audit event replicates as an ordinary
  append-only event and the notification discharges when the link returns.

## 4. The ladder

```
rung 0   custody follows admission           today; the unchanged default
rung 1   custody narrowed to named NODES     serve withholds the DEK from non-holder peers
rung 2   custody narrowed to named ACTORS    the floor gates QUIET unwrap at a holder node
         ─────────────────────────────────
         break-glass                         available at every rung, audited and notified
                                             (rung 1's needs a reachable holder — #498)
```

Invariants, each inherited rather than invented:

- **Withhold the key, never the bytes** (ADR-0052 E1, unchanged). A non-holder still receives ciphertext
  and the safety projection.
- **Narrowing changes cost and noise, never reach — at a node that holds the key or can reach one**
  (new, from finding (c); the bound is #498, and rung 1 offline is the one place it bites).
- **Break-glass is loud in three directions with different jobs** — *location* immediate and in-chart
  (the torn envelope colleagues see; the restraint that actually works), *custodian* and *patient* as
  the discharging accountability trail.

## 5. Custody is an additive field on the sensitivity assertion

Not a new event type. `sensitivity.grade.asserted` gains an optional `custody` object.

Four properties fall out rather than being built:

- **One gesture, not two.** Two independently-settable dials means one is independently forgettable —
  protection real in the projection and absent at the wire, which is #376's own *"worse than shipping
  nothing"* argument reinstated by its own fix. Grade and custody as one signed act keeps `M = N`
  against paper's single gesture (§1.2).
- **The [ADR-0064](../../spec/decisions/0064-admit-the-claim-withhold-the-power.md) authority floor is
  inherited free.** Widening custody is protection-removing and is expressed as withdraw-by-reference on
  the assertion carrying it, so it already routes through `cairn_claim_authority` at the one site every
  dial keys on. No new gate, and ADR-0064's *"part C's dial inherits it structurally"* becomes true by
  construction rather than by anyone remembering.
- **Additive per ADR-0012** — no new event type, so none of the four pinned registry row-counts move.
- **Honest nodes agree.** The custody set is a signed fact rather than a per-node derivation.
- **Composition is intersection, forced by the bullet above.** The free ADR-0064 inheritance holds only
  if adding an assertion can never *widen*; a union rule would let a frictionless raise (ADR-0062
  decision 7 gates only lowering) add a node to the holder set with no authority check. Intersection is
  also the custody analogue of max-over-standing-assertions. **It can empty** — two honest chart-wide
  narrowings by clinicians who never met collapse custody to nobody and make every read loud, which
  destroys finding (b). Not closed here:
  [#499](https://github.com/cairn-ehr/cairn-ehr/issues/499). Review finding; this design originally left
  the rule unstated.

### 5.1 Correcting ADR-0064's handoff argument

ADR-0064 says derivation fails because a well-custodied peer computes a *lower* grade and hands out the
DEK. That specific mechanism does not establish the conclusion: in the thread-resolution case the
well-custodied node computes the **true** grade and the custody-less node is **over**-protecting, which
is the bound working as designed (ADR-0062 decision 9).

The genuine quiet leaks are different, and both involve two **honest** nodes:

- **Registry divergence.** ADR-0064's verdict is computed at read through the live local actor registry.
  Node A has revoked actor Z; node B has not. A withdrawal authored by Z is `unverified` and inert on A —
  grade stays `sequestered`, A withholds — and authorised on B, so **B serves the DEK**. Neither node is
  misbehaving.
- **Replication lag.** The assertion has not reached B yet. B serves.

Explicitness fixes the first. Nothing fixes the second — it is a distributed system — and it is declared
as forward-looking rather than papered over. The conclusion ADR-0064 reached is right; the argument
needed replacing, and the replacement is worth recording because the original would have justified the
wrong mitigation.

## 6. Custody narrows on `event` and `patient`, never on `thread`

Thread membership is knowable only with custody (ADR-0062 decision 9). A custody-less node therefore
cannot tell which events a thread-scoped narrowing covers. It has two options and both are wrong:

- **Serve them** — a silent leak, the exact failure the narrowing exists to prevent.
- **Apply the conservative bound** and withhold every unresolvable clinical event on the chart — which
  makes break-glass routine on precisely the nodes that see the patient least, destroying finding (b).

**The bound is right for disclosure and wrong for custody.** This is the same asymmetry ADR-0064
decision 8 found for the overclaim detector: over-coarsening is safe when it withholds a *disclosure*
and unsafe when it drives a different mechanism.

So a thread-scoped custody narrowing is **refused at the local authoring door, admitted at the remote
door** (the #342 no-fork rule) and surfaced on the worklist — the same retryability axis §7 names, not a
different rule.

This also answers #376's first question. Chart-wide (`patient`) custody narrowing is legitimate and
useful — the staff-member-as-patient case narrows the whole chart to the practice node, which causes no
local routinisation at all, only remote break-glass, which is correct and rare. #376's worry that
chart-wide narrowing makes a chart unusable was about narrowing to named *clinicians*; under the ladder
that is rung 2 and separate.

## 7. Unparseable custody holds nobody — and the grade still stands

Two claims, both load-bearing, and they pull in opposite directions on purpose.

**Fail closed on custody.** If a node cannot parse who the holders are, it must not assume the requester
is one of them. This is affordable *only because* of the keyring: failing closed costs a loud read, not
a lost record. **The keyring is what makes fail-closed affordable** — without it, the same rule would
silently destroy access.

**Never refuse the assertion for it — at the REMOTE door.** `custody` is a **field on** a sensitivity
assertion. Refusing it there drops the **grade** — protection destroyed by a malformed protection field,
#342's fork trap pointed at its own foot, and ADR-0060's *the system may fail to record an order; it may
never cancel one* one subsystem over. **Locally it is refused**, like any malformed body this node's own
client mints (ADR-0063 decision 2's mint-strict/arrive-permissive rule).

**PR-review correction:** the first draft said *"never refuse"* with no door qualifier, which contradicts
§6 — where a thread-scoped narrowing IS refused locally — and contradicts the mint-strict precedent this
design cites. The rule below does not separate the two cases on its own (each drops one assertion); the
separator is **retryability, not defectiveness** — the author is standing there to fix a local refusal
and absent from a remote one.

This is the rule HANDOVER records as having *"three implementations and no name, which is why it keeps
breaking."* This design names it and states its test:

> **Refuse at a door only what that door can drop whole.**
> A malformed sensitivity assertion drops one assertion — refuse it. A malformed *field on* a clinical
> event drops the event that carries it — admit it and read it totally. The question is never how
> defective the bytes are; it is what else dies with them.

Fourth implementation, first name.

## 8. Unknown ranks MAX here too — for a different reason

Consistent with `db/048`/`db/049`, and **not** for their reason. There, MAX withholds *protection* or
mutes a warning if inverted. Here, MAX withholds *quiet access*, and the content stays reachable through
break-glass. The three ladders agree; the arguments do not. This needs its own shouting comment, because
the next person to align them will find this one already aligned and may carry the wrong reason forward
into a fourth site where it does not hold.

## 9. Rung 2 is floor-enforced, not cryptographic

Per-actor cryptography is available: `--author-as` takes a **passphrase-sealed key file**
([main.rs:201-238](../../../crates/cairn-node/src/main.rs#L201-L238)), so a clinician's signing secret
never enters the node or the database, and ADR-0052's HKDF derivation would work one level down to give
each actor an X25519 unwrap key with the DB holding only public halves. A node administrator with full
database access genuinely could not open a body wrapped only to Dr X.

It is **not** built, for two reasons:

- **The keyring makes that boundary loudly crossable anyway.** Against an attacker with node-level DB
  access, the cryptography was never buying protection — that attacker can break glass. It was buying
  *noise*, and the floor produces the same noise at a fraction of the cost.
- **It creates a silent, unrecoverable loss mode.** With no escrow for actor keys — and ADR-0026
  decision 4 makes that deliberate for node keys too (*"the private signing key is never backed up"*) —
  a clinician who leaves and a laptop that dies render clinical content permanently unreadable, with no
  `erasure_shred_log` row to say so. It simply stops opening. ADR-0052 states this risk plainly for the
  node KEK; rung-2 cryptography reproduces it at actor granularity where no escrow exists.

So rung 2 keeps the DEK wrapped to the node and the floor decides **who reads quietly**. Recorded as a
named deferred hardening with the exact threat it would close (quiet read by node-level DB access), not
discarded.

**Rung 2 is blocked on something that does not exist:** a *reader* identity. `--author-as` and
`--attest-as` attribute **writes**; the med-list read path takes a patient and returns rows, with no
actor in scope. [sealed_submit.rs:208](../../../crates/cairn-node/src/medication/sealed_submit.rs#L208)
anticipates it in a comment — *"a reader that needs more breaks glass"* — with nothing behind it. That
surface is §5.11 point-of-care identity, unbuilt.

## 10. Scope

| | scope | state |
|---|---|---|
| **C1** | rung 1: `custody.nodes` on the assertion, both doors, serve-door withholding, the audited break-glass path, in-chart honest disclosure and the location signal | **buildable now — except the chart-wide (`patient`) subject, which is blocked on #499** |
| **C2** | rung 2: floor-enforced quiet-vs-loud unwrap | **blocked on a reader identity (§5.11)** — filed with the block named |
| **D** | patient and custodian notification as §5.12 discharging obligations | after C1 |

The *location* half of the three-way notification belongs in C1, not D: it is local, needs no channel,
and is the part that actually restrains.

## 11. Paper-parity benchmark (§1.2)

Counterpart: **the sealed envelope in the paper file.**

| act | paper *N* | architecture *M* | UI target *K* |
|---|---|---|---|
| seal it and record who may open | 1 | 1 (one assertion carries grade and custody) | 1 |
| read it at a holder node | 0 extra | 0 extra | 0 |
| read it elsewhere, holder reachable | 1 (tear it; the tear is visible) | 1 (invoke break-glass; the audit is automatic) | 1 |
| read it elsewhere, **partitioned** | 1 (tear it — the envelope is in the file you hold) | **impossible** at rung 1 | — |

`M = N` on the first three rows. **The fourth is an architecture defect and is filed as one under
CLAUDE.md rule 7** ([#498](https://github.com/cairn-ehr/cairn-ehr/issues/498)) — not slower or harder but
*impossible*, which §1.2 names as a violation in its own right. Review finding; this table originally
claimed `M = N` at every step.

C1's runnable surface is the CLI, so it owes the **machine-side** budget: a non-narrowed pull shows no
regression beyond noise, and a break-glass round trip to a reachable holder completes in ≤ 5 s. The
clinician-gesture budget is owed by the UI slice that first exposes the gesture. **If a measurement falls
outside its budget, that is the finding — file an issue; never adjust the budget.**

## 12. Testing

TDD throughout, and every guard verified to fail under the revert it names (the
[#387](https://github.com/cairn-ehr/cairn-ehr/issues/387) species, recorded in `docs/ROADMAP.md` —
a mutation that does not change the property tests nothing).

The mutations that must go red:

- unparseable custody flipped from *holds nobody* to *holds everybody*;
- the apply door made to **refuse** a malformed custody rather than admit it (§7's whole point);
- a thread-scoped custody refused at the **remote** door (a fork, #342);
- the authority check stripped so a withdrawal widens custody with no accountable human — inherited from
  ADR-0064 rather than written here, so it needs its **own** pin or nothing proves the inheritance;
- a DEK served to a non-holder peer;
- break-glass unwrap succeeding without its audit row (they share one transaction).

## 13. Rejected

- **Deriving the custody dial from the effective grade** (ADR-0064's offered alternative) — rejected on
  the corrected argument in §5.1: registry divergence and replication lag make two honest nodes disagree
  about who may read quietly, and the disagreement is silent.
- **A separate `custody.narrowed` event type** — rejected: two gestures, and the one that gets forgotten
  is the one nothing displays. `M > N` against paper.
- **Per-actor cryptographic custody now** — deferred, §9.
- **A named holder *list* as the primary control** — rejected: nothing can un-know a fetched DEK, so a
  rendered list *"custody: N1, N2"* is a precise untruth in the reassuring direction on a confidentiality
  surface (principle 4), the same defect shape as ADR-0064's known gap #436 one dial over. The list
  exists as an enforcement input; it is never the safety story told to a clinician.
- **A remote break-glass keyring provider** — rejected: stronger audit (the record lives on another
  party), but it fails under partition at 3am, and availability wins on the safety path.
- **Paper-escrow recoverability per sequester** (a printed recovery code per sealed body, ADR-0026
  decision 5's rung) — rejected in favour of the keyring: it is a physical artifact per act, and the
  safe holding it is a custody set nobody named.

## 14. Declared limitations

- **Rung 1 has no offline glass** ([#498](https://github.com/cairn-ehr/cairn-ehr/issues/498)). A
  partitioned non-holder holds the ciphertext, cannot reach a holder, and falls to §5.9's
  honest-disclosure branch. Rungs 0 and 2 are unaffected. Candidate close: carried-with-patient custody.
- **Intersection can empty** ([#499](https://github.com/cairn-ehr/cairn-ehr/issues/499)) — see §5.
- **Narrowing is forward-looking.** A peer that pulled before the act landed keeps what it has. Nothing
  un-knows a DEK. No surface may imply otherwise.
- **Enforcement is schema-generation-local.** A node that does not understand the `custody` field serves
  the DEK. This is ADR-0012's two-plane model working as designed — schema version is a local node
  property — not a hole to patch.
- **A node is not a person.** Rung 1 alone does not address the threat that motivates most §5.9 cases —
  a colleague at the same practice. Rung 2 is what addresses it, and rung 2 is blocked.
- **Notification can itself be the disclosure, and in the DV case the danger.** *"Dr Z opened sealed
  content on your record at Clinic A"* tells the recipient that sealed content exists and where. Sent to
  a household phone or shared family email — which is what a remote-community demographics record often
  holds — it reaches the abuser with a pointer, and the record was sequestered because of that person.
  Part D owes: coarsened notification content, a patient-controlled channel, and a recorded patient
  preference including *notify me in-record only, never by push*.
- **Break-glass routinises if the location signal is weak.** What restrains on paper is not a
  notification the patient may read months later; it is colleagues seeing the torn envelope now. If the
  location signal is implemented as a background email, the control evaporates.
- **An attacker with node-level DB access reads quietly at rung 2.** Accepted: they can break glass
  anyway, so the cryptography would have bought noise, not protection.

## 15. Findings filed separately

*Two found during the design (#494, #495, below); two more found by the PR review of this document and
of ADR-0065 — **#498** (rung 1 has no offline glass, and the paper-parity table claimed `M = N` where it
is impossible) and **#499** (the custody composition rule was unstated, and the intersection it forces
can empty). Both are folded into the sections above.*

- **`event_dek` cannot express a named holder set, and ADR-0052 says it can.** ADR-0052 decision 4
  describes `(event_id, holder, dek_wrapped)`; the built table is `(event_id PRIMARY KEY, dek_wrapped,
  wrapped_at)`. The primary key on `event_id` alone structurally forbids multi-holder custody — the
  exact property that decision's own sentence says the design needed. This design does not need the
  column (custody is a projection over signed assertions, not keystore rows), so the resolution is an
  **erratum on ADR-0052**, not a migration.
- **ADR-0052 and ADR-0026 disagree about whether a restored node can open its own sealed bodies.**
  ADR-0052 derives the node unwrap secret from the node signing seed and says ADR-0026's escrow covers
  it; ADR-0026 decision 4 says the signing key is never backed up and a restore mints a **new** identity.
  If both hold, every born-sealed body on a restored node goes dark — the whole-record loss ADR-0052
  names. No unwrap-key handling appears in `backup.rs` or `restore.rs`. Either resolved in the sealed
  local-state export design (unread here) or a live hole in disaster recovery; worth an issue either way.
