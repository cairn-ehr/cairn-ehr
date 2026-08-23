# ADR-0065 — Narrow the custody, never the reach

- **Status:** Accepted
- **Date:** 2026-08-23
- **Derives from:** [ADR-0052](0052-born-sealed-clinical-bodies.md) §4 + erratum E1 (*withhold the key,
  never the bytes*) — this ADR is that rule given a ladder — and
  [ADR-0064](0064-admit-the-claim-withhold-the-power.md)'s closing handoff (*a custody dial derived from
  the effective grade is only as strong as its most-custodial holder*), whose **conclusion is adopted and
  whose argument is corrected** (decision 3.1)
- **Applies:** principle 1 (append-only — a narrowing is an overlay, never an edit) · principle 3
  (paper-parity, **governing law here**: the sealed envelope in the file, which opens with your fingers
  and shows the tear) · principle 4 (acknowledged uncertainty — a holder list is never presented as the
  truth about who holds a key) · principle 5 (availability over consistency — the keyring is local
  because a keyring reached over the network fails at 3am) · principle 9 (policy-neutral infrastructure) ·
  principle 12 (the floor is in the database) ·
  [ADR-0004](0004-dynamic-sync-scope-prefetch-not-authority.md) (the key-acquisition trichotomy
  break-glass rides) · [ADR-0005](0005-erasure-key-custody-and-crypto-shredding.md) (custody, not
  deletion) · [ADR-0012](0012-schema-evolution-event-format-and-legibility-across-time.md) (additive
  evolution; schema generation is a **local node property**, which is why enforcement is
  generation-local) · [ADR-0026](0026-node-durability-and-disaster-recovery.md) decision 4 (the signing
  key is never backed up — the fact that makes per-actor cryptographic custody a silent loss mode) ·
  [ADR-0060](0060-partial-validity-a-defect-on-one-line-never-invalidates-another.md) (*the system may
  fail to record an order; it may never cancel one*) ·
  [ADR-0062](0062-the-sensitivity-stream-and-the-inverted-unknown.md) decisions 2, 4, 8, 9 ·
  [ADR-0063](0063-the-safety-projection-and-the-seal-as-coarsening-boundary.md) decision 2 and its
  blast-radius argument
- **Canonical spec home:** [identity §5.9](../identity.md#59-sensitivity-grade-the-safety-projection-and-break-glass-visibility-scope)

## Context

§5.9 parts A and B compute and report. [ADR-0062](0062-the-sensitivity-stream-and-the-inverted-unknown.md)
built a graded, append-only sensitivity stream whose effective grade is a projection;
[ADR-0063](0063-the-safety-projection-and-the-seal-as-coarsening-boundary.md) built the de-identified
safety signal that survives a seal and a shred; [ADR-0064](0064-admit-the-claim-withhold-the-power.md)
established that only an accountable human may *lower* a grade. **None of them withholds any content
from anybody.** Part C ([#376](https://github.com/cairn-ehr/cairn-ehr/issues/376)) is where a grade
stops being advisory.

It was blocked on [#231](https://github.com/cairn-ehr/cairn-ehr/issues/231) — while `cairn-sync serve`
verified a puller's unwrap certificate against its own signature only, transport was the sole gate on
read-custody and a narrowing would have been *"protection that is real in the projection layer and
absent at the wire."* #231 is closed (ADR-0052 erratum E1); the block is lifted.

What remained was a genuine fork, and ADR-0064 declined to settle it: derive the custody dial from the
effective grade, or express it as a signed act. Behind that sat three unresolved questions — which
subject may feed which dial, how the decision-9 conservative bound interacts, and what happens to a node
that already fetched the DEK.

The fork did not resolve on its own terms. It resolved by asking what sequester is *for*.

## Decision

### 1. The custody ladder, and the invariant that makes it safe

```
rung 0   custody follows admission           the unchanged default
rung 1   custody narrowed to named NODES     serve withholds the DEK from non-holder peers
rung 2   custody narrowed to named ACTORS    the floor gates QUIET unwrap at a holder node
         ─────────────────────────────────
         break-glass                         available at EVERY rung, audited and notified
```

> **Custody narrowing changes the cost and the noise of reading. It never changes whether the content
> can be reached.**

**Node custody is the norm; per-clinician custody is the exception.** A blanket per-clinician policy
would cause unbearable friction inside a location and make normal work impossible — in an ED the team
reads the chart. And break-glass must stay **rare** to stay meaningful, which it does precisely because
node custody is the norm: at a holder node, reading sensitive content is ordinary work with no ceremony.
Break-glass fires only off-ladder — a non-holder node, or a clinician outside a rung-2 set.

The rejected shape is worth naming because it is the obvious one: making break-glass the route for the
*normal* case. That is [§5.11](../identity.md#511-point-of-care-identity-possession-fast-authentication-and-salvage)'s
confirmation-dialog disease and [§5.12](../identity.md#512-the-notification-economy-salience-responsibility-routing-and-the-acknowledgment-floor)'s
alert fatigue in one gesture: a ceremony repeated until it is performed unread.

**Two invariants are inherited unchanged:** *withhold the key, never the bytes* (a non-holder still
receives ciphertext and the safety projection — refusing rows would fork the event set for no
confidentiality gain), and *coarseness varies, existence never disappears*.

### 2. The node's own DEK is the keyring, and the floor is the glass

The break-glass keyring needs **no new key material**. `event_dek` already holds this node's wrapped copy
of every DEK, `REVOKE`d from `PUBLIC` and `cairn_agent` and granted only to `cairn_node`. So:

- **Rung 2 break-glass is local** — the in-DB floor lets an actor through and writes the audit row **in
  the same transaction**. No key moves.
- **Rung 1 break-glass is a network act** — a non-holder asks a holder for the DEK with an audited
  justification, which is [ADR-0004](0004-dynamic-sync-scope-prefetch-not-authority.md)'s acquisition
  trichotomy that §5.9 already names (*"from sibling/parent on reconnect"*).

**The keyring is local, never a remote provider.** A keyring reached over the network fails at 3am in a
partitioned remote clinic — an availability failure on the safety path. Break glass locally; the audit
event replicates as an ordinary append-only event and the notification discharges when the link returns.

**Consequence for sequencing, stated because the issue tracker records it backwards:** part C and part D
are **not separable**. A narrowing without an audited break-glass path creates content nobody can reach.
[#377](https://github.com/cairn-ehr/cairn-ehr/issues/377) says *"blocked on C"*; **the glass has to exist
before anything is sealed behind it.**

### 3. Custody is an additive field on the sensitivity assertion, not a new event type

`sensitivity.grade.asserted` gains an optional `custody` object. Four properties fall out rather than
being built:

- **One gesture, not two.** Two independently-settable dials means one is independently *forgettable* —
  protection real in the projection and absent at the wire, which is #376's own *"worse than shipping
  nothing"* argument reinstated by its own fix. One signed act keeps `M = N` against paper (§1.2).
- **[ADR-0064](0064-admit-the-claim-withhold-the-power.md)'s authority floor is inherited free.** Widening
  custody is protection-**removing**, and it is expressed as withdraw-by-reference on the assertion
  carrying it — so it already routes through `cairn_claim_authority` at the single site every dial keys
  on. ADR-0064's promise that *"part C's dial inherits it structurally"* becomes true by construction
  rather than by anyone remembering.
- **Additive per [ADR-0012](0012-schema-evolution-event-format-and-legibility-across-time.md)** — no new
  event type, so none of the four pinned registry row-counts move and `SCHEMA_GENERATION` rises only for
  the floor migration.
- **Honest nodes agree.** The custody set is a signed fact, not a per-node derivation.

#### 3.1 Correcting ADR-0064's handoff argument

ADR-0064 says derivation fails because a well-custodied peer computes a **lower** grade and serves the
DEK. That mechanism does not establish the conclusion. In the thread-resolution case the well-custodied
node computes the **true** grade and the custody-less node is **over**-protecting — decision 9's bound
working exactly as designed.

The genuine quiet leaks are different, and in both cases **both nodes are honest**:

- **Registry divergence.** ADR-0064's verdict is computed at read through the *live local* actor
  registry. Node A has revoked actor Z; node B has not. A withdrawal authored by Z is `unverified` and
  inert on A — the grade stands, A withholds — and authorised on B, so **B serves the DEK**. Neither node
  is misbehaving; they hold different registries.
- **Replication lag.** The assertion has not reached B. B serves.

Explicitness closes the first. Nothing closes the second — it is a distributed system — and it is
**declared** (decision 8) rather than papered over.

The conclusion ADR-0064 reached is adopted; the argument is replaced. Recording the correction matters
because the original argument would have justified the wrong mitigation — hardening thread resolution,
which is not where the leak is.

### 4. Custody narrows on `event` and `patient`, never on `thread`

Thread membership is knowable only with custody
([ADR-0062](0062-the-sensitivity-stream-and-the-inverted-unknown.md) decision 9), so a custody-less node
cannot tell which events a thread-scoped narrowing covers. Its two options are both wrong: **serve them**
(the silent leak the narrowing exists to prevent), or **apply the conservative bound** and withhold every
unresolvable clinical event on the chart — which makes break-glass routine on precisely the nodes that
see the patient least, destroying decision 1.

**The bound is right for disclosure and wrong for custody.** This is the same asymmetry
[ADR-0064](0064-admit-the-claim-withhold-the-power.md) decision 8 found for the overclaim detector:
over-coarsening is safe when it withholds a *disclosure* and unsafe the moment it drives a different
mechanism. Two ADRs have now hit this from opposite directions, so state it generally: **"conservative"
is a property of a direction, not of a value — before reusing a bound, ask what the bound now drives.**

A thread-scoped custody narrowing is therefore **refused at the local authoring door, admitted at the
remote door** ([#342](https://github.com/cairn-ehr/cairn-ehr/issues/342)'s no-fork rule) and surfaced on
the worklist.

This answers #376's first question. Chart-wide (`patient`) custody narrowing **is** legitimate: the
staff-member-as-patient case narrows the whole chart to the practice node, which causes no local
routinisation — only remote break-glass, which is correct and rare. #376's worry that chart-wide
narrowing makes a chart unusable was about narrowing to named *clinicians*; under this ladder that is
rung 2, and separate.

### 5. Unparseable custody holds nobody — and the grade still stands

Two claims that pull in opposite directions, on purpose.

**Fail closed on custody.** A node that cannot parse who the holders are must not assume the requester is
one of them. **The keyring is what makes fail-closed affordable**: the cost is a loud read, not a lost
record. Without the keyring the identical rule would silently destroy access, which is why this decision
cannot be lifted out of this ADR and applied elsewhere unchanged.

**Never refuse the assertion for it.** `custody` is a **field on** a sensitivity assertion. Refusing the
assertion drops the **grade** — protection destroyed by a malformed protection field, #342's fork trap
pointed at its own foot, and [ADR-0060](0060-partial-validity-a-defect-on-one-line-never-invalidates-another.md)
one subsystem over.

This is the rule with three implementations and no name. It gets one here:

> **Refuse at a door only what that door can drop whole.**
> A malformed sensitivity assertion drops one assertion — refuse it. A malformed **field on** a clinical
> event drops the event that carries it — admit it, and make the read model total. The question is never
> how defective the bytes are; it is **what else dies with them.**

Fourth implementation, first name. The three that came before are ADR-0062's structural-vs-ceremony
split, ADR-0063 decision 2's mint-strict/arrive-permissive rule, and ADR-0056's admit-uninterpreted
floor.

### 6. Unknown ranks MAX here too — for a different reason, and the difference is the point

Consistent with `db/048`/`db/049`, and **not for their reason.** There, rank MAX withholds *protection*
or mutes a warning if inverted. Here it withholds *quiet access*, and the content stays reachable through
break-glass.

The three ladders agree; the arguments do not. This needs its own shouting comment for exactly the reason
the other two carry one: the next author to "align the ladders" will find this one already aligned, and
may carry the wrong justification forward into a fourth site where reachability is **not** guaranteed —
at which point fail-closed stops being affordable and starts destroying access.

### 7. Rung 2 is floor-enforced, not cryptographic

Per-actor cryptography is genuinely available. `--author-as` takes a **passphrase-sealed key file**, so a
clinician's signing secret never enters the node or the database, and ADR-0052's HKDF derivation works
one level down: an actor X25519 unwrap key, with the DB holding only public halves. A node administrator
with full database access could not open a body wrapped only to Dr X.

It is **not built**, for two reasons:

- **The keyring makes that boundary loudly crossable anyway.** Against an attacker with node-level DB
  access the cryptography was never buying protection — that attacker can break glass. It was buying
  **noise**, and the floor produces the same noise at a fraction of the cost. `event_dek` is already
  `REVOKE`d from `PUBLIC` and `cairn_agent`, so principle 12's unbypassable floor genuinely covers it.
- **It creates a silent, unrecoverable loss mode.** There is no escrow for actor keys, and
  [ADR-0026](0026-node-durability-and-disaster-recovery.md) decision 4 makes that deliberate for node
  keys too — *"the private signing key is never backed up."* A clinician who leaves plus a laptop that
  dies renders clinical content permanently unreadable, with **no `erasure_shred_log` row to say so**. It
  simply stops opening. ADR-0052 states this risk plainly for the node KEK (*KEK loss = whole-record
  loss, hence escrow is mandatory*); per-actor wrapping reproduces it at actor granularity where no
  escrow exists. An EHR may lose a record deliberately, audibly and by ceremony; it may not lose one by
  a forgotten passphrase.

So rung 2 keeps the DEK wrapped to the node, and the **floor** decides who reads quietly. Per-actor
wrapping is recorded as a named deferred hardening with the exact threat it would close — *quiet read by
node-level database access* — not discarded.

**Rung 2 is blocked on something that does not exist: a reader identity.** `--author-as` and
`--attest-as` attribute **writes**; the med-list read path takes a patient and returns rows with no actor
in scope. `sealed_submit.rs` already anticipates the gesture in a comment — *"a reader that needs more
breaks glass"* — with nothing behind it. That surface is
[§5.11](../identity.md#511-point-of-care-identity-possession-fast-authentication-and-salvage)
point-of-care identity, unbuilt.

### 8. Break-glass is loud in three directions, and they have different jobs

- **Location — immediate, in-chart.** This is the torn envelope colleagues see, and it is the direction
  that actually restrains. What stops routine break-glass on paper is not a notification the patient may
  read months later; it is that the tear is visible now, locally, to the people you work beside. Named
  first because if it is implemented as a background email the control evaporates and decision 1's
  rarity guarantee goes with it.
- **Custodian** — the accountability trail to whoever narrowed the custody.
- **Patient** — the sovereignty trail, and the one §5.9's mission argument most requires.

All three are §5.12 **discharging obligations**, not fire-and-forget: an append-only obligation that
closes when delivery is acknowledged. This makes them an instance of the notification economy and of
Case 0001's open-loop item, not new machinery. Part D owns their delivery; **location ships with part C**
because it needs no channel.

### 9. What this buys, stated so no reader infers more

The same posture [ADR-0064](0064-admit-the-claim-withhold-the-power.md) decision 9 declared, one dial
over: **narrowing buys a default and a record, not a lock.** Rung 1 is a real gate against an honest peer
running this generation of the floor. Rung 2 is a real gate against everyone except node-level database
access. Neither is a wall, and neither is described to a clinician as one.

This is not a weakness relative to the thing being replaced. Paper's sealed envelope is openable by
anyone who holds the file; what paper provides is not a lock but an unmistakable record that it was
opened — and, crucially, one that never renders the contents unreachable to the clinician who needs
them at 3am.

## Paper-parity benchmark (§1.2)

This changes a clinical workflow at the in-DB floor, so it carries a benchmark rather than the
forced-rationale escape. Counterpart: **the sealed envelope in the paper file.**

| act | paper *N* | architecture *M* | UI target *K* |
|---|---|---|---|
| seal it, record who may open | 1 | 1 (one assertion carries grade **and** custody) | 1 |
| read it at a holder node | 0 extra | 0 extra | 0 |
| read it elsewhere | 1 (tear it; the tear is visible) | 1 (invoke break-glass; the audit is automatic) | 1 |

`M = N` at every step — **no architecture defect to file.** The decision that keeps it there is decision
3: had custody been a separate event type, sealing would have cost two acts against paper's one.

Part C1's runnable surface is the CLI, so it owes the **machine-side** budget: a non-narrowed pull shows
no regression beyond noise, and a break-glass round trip to a reachable holder completes in **≤ 5 s**.
The clinician-gesture budget is owed by the UI slice that first exposes the gesture. **If a measurement
falls outside its budget, that is the finding — file an issue; never adjust the budget.**

## Rejected alternatives

- **Derive the custody dial from the effective grade** (ADR-0064's offered alternative). Rejected on the
  corrected argument in 3.1: registry divergence and replication lag make two **honest** nodes disagree
  about who may read quietly, and the disagreement is silent. A control that a faithful peer defeats *by
  computing correctly* is not weak — it is incoherent.
- **A separate `custody.narrowed` event type.** Rejected: two gestures, and the one that gets forgotten
  is the one nothing displays. `M > N` against paper, which CLAUDE.md rule 7 makes a defect rather than
  a cost.
- **Per-actor cryptographic custody now.** Deferred with its threat named — decision 7.
- **A named holder *list* as the safety story.** Rejected: nothing can un-know a fetched DEK, so a
  rendered *"custody: N1, N2"* is a **precise untruth in the reassuring direction on a confidentiality
  surface** (principle 4) — the defect shape of ADR-0064's known gap
  [#436](https://github.com/cairn-ehr/cairn-ehr/issues/436), one dial over. The list is an enforcement
  input; it is never what a clinician is told.
- **A remote break-glass keyring provider.** Genuinely stronger audit — the record would live with
  another party and could not be deleted by the breaker — but it fails under partition at 3am, and
  availability wins on the safety path.
- **Paper-escrow recoverability per sequester** (a printed recovery code per sealed body, ADR-0026
  decision 5's rung). Rejected in favour of the keyring: a physical artifact per act, and the safe
  holding it is a custody set nobody named.
- **Blanket per-clinician custody as the default.** Rejected as unworkable inside a location — the
  finding that produced the whole ladder.

## Known limitations

- **Narrowing is forward-looking.** A peer that pulled before the act landed keeps what it has. Nothing
  un-knows a DEK, and no surface may imply otherwise.
- **Enforcement is schema-generation-local.** A node that does not understand the `custody` field serves
  the DEK. This is ADR-0012's two-plane model working as designed — schema version is a **local node
  property**, there is no lockstep fleet upgrade — not a hole to patch.
- **A node is not a person.** Rung 1 alone does not address the threat that motivates most §5.9 cases: a
  colleague at the same practice — the receptionist who is the patient's neighbour, the nurse related to
  the ex-partner. Rung 2 addresses it, and rung 2 is blocked on §5.11.
- **Notification can be the disclosure, and in the domestic-violence case the danger.** *"Dr Z opened
  sealed content on your record at Clinic A"* tells the recipient that sealed content exists and where.
  Delivered to a household phone or a shared family email — which is what a remote-community demographics
  record often holds — it reaches the abuser, **with a pointer**, and the record was sequestered because
  of that person. Part D owes coarsened notification content, a patient-controlled channel, and a
  recorded patient preference including *in-record only, never push*. Getting this wrong converts a
  privacy feature into a safety incident.
- **Break-glass routinises if the location signal is weak** — decision 8.
- **An attacker with node-level database access reads quietly at rung 2.** Accepted: under the keyring
  they can break glass anyway, so cryptography would have bought noise rather than protection.

## Consequences

**Easier.**

- Sequester becomes buildable without a new key tier, a new escrow mechanism, or a per-actor key
  management surface — decision 2.
- Recoverability and confidentiality stop being in tension. Every previous shape of this design traded
  one against the other; the keyring makes them the same mechanism.
- Fail-closed becomes affordable on a confidentiality dial (decision 5), which is normally the one place
  it is too expensive.
- ADR-0064's structural inheritance claim is discharged: widening custody routes through
  `cairn_claim_authority` with no new gate.
- The three-implementations-no-name rule finally has a name and a test (decision 5).
- No new event type, no ADR-0057 registry entry, none of the four pinned registry row-counts move.
  Stated so a reviewer can check rather than assume.

**Harder.**

- **Part C and part D are one body of work.** The tracker's dependency direction is reversed, and the
  glass must ship before the seal.
- Decision 5's fail-closed rule is safe **only** while the keyring guarantee holds. If a future change
  makes any content unreachable, that decision becomes a destroyer of access — the most dangerous
  coupling this ADR introduces, and the reason decisions 5 and 6 both carry the reachability argument
  explicitly rather than by reference.
- Rung 2 is blocked on a reader identity that does not exist, so the ladder ships with its most
  clinically-motivated rung unbuilt, and that gap must be stated to users rather than implied away.

**The bet.** That a control which is a *default plus a record* is worth more in a clinical system than a
lock that can silently destroy a record — and that the restraint comes from colleagues seeing the tear,
not from a notification read months later. We would know it is wrong if break-glass rates measured on a
real deployment show it is routine rather than exceptional, which is exactly what decision 8's location
signal exists to prevent and what part D's audit trail makes measurable.
