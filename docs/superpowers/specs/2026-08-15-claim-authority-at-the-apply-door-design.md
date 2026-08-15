# Design — what makes a claim authoritative at the apply door

- **Date:** 2026-08-15
- **Issues:** closes [#380](https://github.com/cairn-ehr/cairn-ehr/issues/380) (nothing on the wire
  controls a protection-REMOVING act); gives [#245](https://github.com/cairn-ehr/cairn-ehr/issues/245)
  its first consumer and its SQL mirror; discharges the emission half of
  [#405](https://github.com/cairn-ehr/cairn-ehr/issues/405) (part 2, the un-licensed rung)
- **Forcing case, NOT designed here:** [#376](https://github.com/cairn-ehr/cairn-ehr/issues/376)
  (§5.9 part C — sequester / custody narrowing)
- **Canonical spec home:** [identity §5.9](../../spec/identity.md#59-sensitivity-grade-the-safety-projection-and-break-glass-visibility-scope)
  for the sensitivity application; the rule itself is a floor property and belongs beside
  [§9.5](../../spec/language-substrate.md) / principle 12
- **Builds on:** [ADR-0056](../../spec/decisions/0056-unknown-event-types-admitted-uninterpreted.md)
  (*custody is total; interpretation is deferred; power is earned* — this design is that sentence given a
  mechanism) · [ADR-0062](../../spec/decisions/0062-the-sensitivity-stream-and-the-inverted-unknown.md)
  decisions 3, 6, 7 and 9 · [ADR-0063](../../spec/decisions/0063-the-safety-projection-and-the-seal-as-coarsening-boundary.md)
  decision 2 · [ADR-0052](../../spec/decisions/0052-born-sealed-clinical-bodies.md) §4 + erratum E1
  (*withhold the key, never the bytes*) · [ADR-0053](../../spec/decisions/0053-per-write-human-authorship.md)
  (the attestation this design consumes) · [ADR-0051](../../spec/decisions/0051-contributor-role-vocabulary-floor-and-responsibility-wire-shape.md)
  (`classify_role`, the shared-pure-classifier discipline) ·
  [ADR-0058](../../spec/decisions/0058-grade-gated-teffective-ceiling.md) (the record-a-flag-rather-than-refuse
  idiom) · [ADR-0060](../../spec/decisions/0060-partial-validity-a-defect-on-one-line-never-invalidates-another.md)
  (an advisory field may never cancel clinical content)
- **Will produce:** ADR-0064

---

## 1. The pattern, and what this design deliberately is not

Three findings, one closed and two open, are the same defect:

| | surface | what the floor validated | what it never asked |
|---|---|---|---|
| [#231](https://github.com/cairn-ehr/cairn-ehr/issues/231) (closed) | the unwrap-cert serve door | the cert's own signature and self-consistency | whether its `kid` was in the trust set |
| [#380](https://github.com/cairn-ehr/cairn-ehr/issues/380) | `sensitivity.grade-withdrawal.asserted` | the claim's shape (a non-empty rationale, at both doors) | whether anyone accountable stood behind lowering the grade |
| [#405](https://github.com/cairn-ehr/cairn-ehr/issues/405) part 2 | `EventBody.safety` | the signal's shape (`cairn_check_safety_signal`) | whether the chart's grade licensed the rung claimed |

In each case the floor established that a claim was **well-formed** and then treated well-formedness as
**authority**. Part C would add a fourth instance, and the first where the cost of being wrong is a DEK
rather than a wrongly-rendered line: a sequester keyed on the effective grade inherits #380 directly —
strip the grade un-attested, and the serve door computes `routine` and hands out the key.

**This design fixes the general defect and does not design part C.** Part C is used throughout as the
falsification case — *does this let sequester key on a grade nobody can silently strip?* — and §8 records
the one sharpened finding it hands forward. #376's own three decisions (which subject feeds which dial,
the decision-9 bound interaction, re-wrap mechanics) stay part C's to take.

**The primitive is not new.** It is [ADR-0056](../../spec/decisions/0056-unknown-event-types-admitted-uninterpreted.md)'s
*custody is total; interpretation is deferred; power is earned* and #231's own resolution
*withhold the key, never the bytes*, stated as one rule with one mechanism:

> **Admit the claim; withhold the power.**

That the frame post-hoc explains the hole already fixed — by an argument nobody made at the time — is the
best evidence available that it is the right frame rather than a story fitted to three bugs.

### What already exists and is unwired

The doctrine is written down in three places and implemented in none:

- `classify_authorship_confidence` → `Attested` / `Unverified` / `Device`
  ([`crates/cairn-event/src/contributor.rs:144`](../../../crates/cairn-event/src/contributor.rs)) is pure,
  total, property-tested, and carries the comment *"NOT YET WIRED TO A READ PATH … do not read this
  type's existence as evidence that grading is in force"* ([#245](https://github.com/cairn-ehr/cairn-ehr/issues/245)).
- [`db/005_submit.sql:635`](../../../db/005_submit.sql) states the rule outright — *"apply admits and
  GRADES (`classify_authorship_confidence`), never refuses"* — as the reason `cairn_authorship_bound` is
  strict-door-only.
- `cairn_ceiling_classify` → `ok` / `flag` / `reject` plus the append-only `t_effective_ceiling_flag`
  ([`db/040`](../../../db/040_clock_confidence_grade.sql), ADR-0058) is a shipped *third verdict* —
  admit, and record — used in exactly one subsystem.

So the floor already has a vocabulary for authority, a mechanism for a non-refusing verdict, and a
doctrine saying to use both. Every dial nonetheless reads a claim's shape and stops.

---

## 2. Decisions this design takes (→ ADR-0064)

1. **Authority is a human actor this node can hold responsible** — two sufficient routes, R1 (vouched
   human attestation) and R2 (human self-withdrawal). Not the relaying machine, not the actor's
   relationship to the chart.
2. **Authority gates *effect*, never *admission*, and only in the withholding direction.** A claim below
   the bar may always raise protection and may never lower it. Nothing is refused at either door.
3. **The bar is a fixed floor, not a deployment threshold.** Principle 9's policy-neutrality governs
   *what* is confidential, not *whether* a protection-removing act must be accountable.
4. **The verdict is computed at read, never stamped at apply**, because both routes' answers legitimately
   improve as replication and actor enrolment proceed.
5. **One predicate, consulted at exactly one site per dial** — never re-derived per consumer.
6. **Flag what cannot self-heal; view what can.**
7. **The control is the record; the gate is the forcing function.** This buys accountability, not
   authorization, and the ADR says so in those words.

---

## 3. The rule

```
authority(claim, target) =
    'attested'    if R1
    'self'        if R2
    'unverified'  otherwise
```

**R1 — vouched human attestation.** `cairn_attestation_vouched(claim.event_id)` is true, **and** the
attester resolves in `actor_current` to `kind = 'human'`.

The vouch predicate is not optional politeness. Both doors store `event_log.attestation` /
`attester_key` **even when the token was not verified as a vouch**, marking the row in
`event_attestation_unvouched`; [`db/001:490`](../../../db/001_envelope.sql) names
`cairn_attestation_vouched` as *"the ONE way to ask"* and predicts *"a fifth will arrive with the next
type that reads `event_log.attester_key`."* This is that fifth. Reading the column directly would grade
a forged token as attested — the very defect this design exists to close, one layer down.

**R2 — human self-withdrawal.** The claim's `event_log.actor_id` equals the target's, **both are
non-NULL**, and that actor is `kind = 'human'`.

- `actor_id` rather than `signing_key_id`, so the identity survives key rotation within one actor
  (ADR-0011: it content-addresses the pinned determinant set).
- A NULL `actor_id` — a key concurrently mapped to several actors, attribution honestly unknown — is
  **not** self. Principle 4: uncertainty withholds power, it never confers it.
- **The `kind = 'human'` clause is load-bearing, not decoration.** Without it, the advisory matcher that
  auto-applied a protective tag could strip its own tag with no human anywhere in the loop — precisely
  what [ADR-0062 decision 6](../../spec/decisions/0062-the-sensitivity-stream-and-the-inverted-unknown.md)
  forbids (*"ADR-0043's agent advisories are dismissable by anyone does not reach a protective
  auto-tag"*). With it, both routes say the same thing and decision 6 is enforced structurally rather
  than restated in prose.

### Why this does not reopen ADR-0062 decision 7

Decision 7 keeps the withdrawal ceremony local-door-only for two reasons. **Neither reaches this
mechanism**, because nothing here refuses anything:

1. *A door check at apply forks the event set (the [#342](https://github.com/cairn-ehr/cairn-ehr/issues/342)
   trap).* No refusal, so no fork. The event lands, converges, is readable, appears in chart history and
   is re-assertable. Set-union losslessness is untouched.
2. *For a raise, the refusal is itself a disclosure.* Only lowering is gated. A protective act is never
   impeded by any path through this design.

So #380's dilemma dissolves rather than being traded off: **the ceremony's authorship half stops being
local-door-only, not by becoming a refusal at apply, but by becoming a condition of effect.**

### Why it does not deadlock

ADR-0062 rejected *self-only* withdrawal because it deadlocks — the asserting clinician retired, the
patient left, the advisory actor was superseded. This is not self-only: R1 is the primary route and R2 an
alternative.

More importantly, **the local door already requires a bound human author for a withdrawal**
(decision 7), so every locally-authored withdrawal clears the bar **by construction**. The only claims
that go inert are cross-node ones with no human behind them, and for those the remedy is not
re-asserting the grade — it is *attesting the withdrawal*, which is what the local door demands of
everyone anyway.

---

## 4. Where it lives

```sql
CREATE OR REPLACE FUNCTION cairn_claim_authority(p_event_id uuid, p_target_event_id uuid)
RETURNS text LANGUAGE sql STABLE
SET search_path = public
AS $$ … $$;
GRANT EXECUTE ON FUNCTION cairn_claim_authority(uuid, uuid) TO cairn_agent;
```

**Home: `db/005_submit.sql`.** Three constraints select it and there is no second candidate.

- **Eager binding forbids a later migration.** `cairn_sensitivity_standing` is `LANGUAGE sql`, which
  resolves references at CREATE time, so a predicate defined in a new `db/050` would not exist when
  db/048 loads and a genuinely **fresh** database would fail there. This is the failure db/005's own
  comment documents for `cairn_clear_payload` and db/034.
- **Converting `standing` to plpgsql for late binding is the wrong trade.** It is db/048's own
  `cairn_event_thread` precedent, but `standing` is a `LANGUAGE sql` STABLE function the planner can
  **inline**, and it sits on the read path of every safety-projection read. Losing inlining on the
  Pi-class latency budget to gain a tidier file is not a trade worth making.
- **db/005 is in both schema lists, loads after db/004, and already hosts this predicate family.**
  `actor_current` (a view, db/004) resolves; db/048 loads later; and `cairn_authorship_bound`,
  `cairn_responsibility_bound` and `cairn_check_contributors` are already there.
  `cairn_claim_authority` is the read-side twin of `cairn_authorship_bound` — contributor.rs already
  says the two *"ask the same question at opposite doors"* — so they belong in one file.

**Fixed arity two, from day one.** A caller with no target passes an explicit `NULL` and gets R1 only.
This is deliberate defence against the trap recorded from
[#404](https://github.com/cairn-ehr/cairn-ehr/issues/404): Postgres **overloads** on a changed argument
list rather than replacing, and migration replay never drops what a file stops creating — so a later
1-arg → 2-arg widening would leave a stale definition alive in every existing database, silently serving
any call site missed, including `has_function_privilege` pins. Fixing the arity now means it never
arises.

### The tables it may touch

Every one is present in **both** the `cairn-node` and `cairn-sync` schema lists:

| table / view | migration | in cairn-sync subset |
|---|---|---|
| `event_log` (`actor_id`, `attestation`, `attester_key`) | db/001 | yes |
| `event_attestation_unvouched` + `cairn_attestation_vouched` | db/001 | yes |
| `actor_current` (`kind`, `signing_key_id`) | db/004 | yes |
| `sensitivity_assertion` / `sensitivity_withdrawal` | db/048 (self) | yes |

`trust_peer` (db/007) is **deliberately absent** from the cairn-sync loader
([`crates/cairn-sync/src/main.rs:176`](../../../crates/cairn-sync/src/main.rs)). Had trust-set origin
been taken as an authority fact, `cairn_sensitivity_standing` would have failed to create on the sync
node and taken clinical sync down entirely. Recorded because it is the second independent reason to
reject that fact — the first being that *any* admitted peer satisfies it, which is #380's own shape one
layer up.

**Idiom to copy verbatim** for the attester-kind test, from
[`db/020:252`](../../../db/020_apply_remote_event.sql):
`WHERE signing_key_id = encode(p_attester_key,'hex') AND kind = 'human'`.

---

## 5. The seam — one clause, three consumers

`cairn_sensitivity_standing` ([`db/048:328`](../../../db/048_sensitivity_stream.sql)) is a set difference:
assertions minus withdrawals. Both `event_id`s are in hand at the seam, so the change is one predicate in
the existing `NOT EXISTS`:

```sql
    WHERE a.patient_id = p_patient_id
      AND NOT EXISTS (SELECT 1 FROM sensitivity_withdrawal w
                       WHERE w.withdraws = a.content_address
                         AND w.patient_id = p_patient_id
                         AND cairn_claim_authority(w.event_id, a.event_id) <> 'unverified')
```

That is the whole enforcement. **Three consumers inherit it with no change to any of them:**

- `cairn_effective_sensitivity` (db/048 §11) — display coarsening;
- `cairn_prospective_sensitivity` (db/049 §6) — so the emitted safety rung is chosen against a grade that
  cannot be silently stripped;
- `crates/cairn-node/src/sensitivity.rs:319` — the CLI read path.

And part C's custody dial, whenever it lands, inherits it the same way — **structurally, not by anyone
remembering.**

This is the anti-drift decision, and it is why the seam matters more than the predicate. `standing` is
the one definition the two otherwise-duplicated read models genuinely share (ADR-0063 records the
duplication as its single largest drift risk, and #404 is that risk realised). A per-dial authority check
would put the same judgement in three places that have already been shown to diverge.

**Stated normatively for the ADR:**

> Authority is consulted where a protection-removing act's **effect** is computed — exactly one site per
> dial — never at each consumer.

---

## 6. Making the inert visible

### 6a. A view, not a ledger

`t_effective_ceiling_flag` records a judgement at the door. That is wrong here. Authority is computed at
read precisely **because the answer improves**: a withdrawal reads `unverified` today because its target
has not replicated (ADR-0062 decision 3 requires exactly that ordering to work) or its attester is not
yet enrolled here, and clears tomorrow. An apply-time ledger would fill with rows that were true for an
afternoon, and a worklist that is mostly stale is
[§5.12](../../spec/identity.md#512-the-notification-economy-salience-responsibility-routing-and-the-acknowledgment-floor)'s
alert-fatigue disease, self-inflicted, in the one place we are building a control.

`sensitivity_withdrawal_worklist` is therefore a **view**: always accurate, self-clearing, no table, no
row-count pins, no interaction with `cairn_execute_shred`'s scrub. Nothing is lost that is not
reconstructible from the append-only log at any time. `GRANT SELECT … TO cairn_agent`.

### 6b. Two rows, two reasons

| reason | condition | clears when |
|---|---|---|
| `inert` | authority is `unverified` — the withdrawal is in the log but moves nothing | the attester enrols here, or the target assertion replicates |
| `stranger-attested` | authority is `attested`/`self`, the withdrawal was authored on another node, and its actor has no prior authored content on this chart | never — it is a fact about a completed act |

The second row is the **detect** half of #380 that survives the gate: an attested strip by a human with
no business on that chart clears the bar and would otherwise be invisible, because nothing in the system
watches withdrawals.

It reuses the chart-standing fact that was **rejected as an authority input** — deliberately, and the
distinction is the point. As authority it fails the locum, the night-cover registrar and the receiving
ED, who have never touched the chart and must not be second-class. As **salience** it carries none of
that cost: it blocks nothing, delays nothing, and the stranger's withdrawal takes effect immediately.
This is the [§5.13](../../spec/identity.md#513-locale-pluggable-comparators-the-matcher-extension-point)
duplicate-sweep posture — *surface, never block* — and the same reason ADR-0062's *how we would know the
bet fails* is a query rather than a constraint.

### 6c. The general rule

> **Flag what cannot self-heal; view what can.**

Both idioms now appear in one subsystem, so the ADR states the rule for choosing rather than leaving the
next author to guess.

---

## 7. #405 part 2 — the other side of the rule

An over-claimed rung falls on the flag side. `cairn_check_safety_signal` validates shape only and never
compares the claimed rung against what the chart's standing grade licenses, so an enrolled-but-hostile
client with direct DB access (Spike-0002's C1–C5 threat model, treated here as live) can submit
`{"rung":"precise","class":"antiretroviral-interaction"}` on a `sequestered` chart.

The door **cannot refuse it**: ADR-0060 forbids an advisory field cancelling a medication assert, and the
door cannot rewrite `event_log.safety` without making the column disagree with `signed_bytes`. So it
takes the ADR-0058 idiom — `cairn_record_safety_overclaim_flag`, recording
`emitted_rung_rank < licensed_rung_rank`, making the bypass auditable at zero clinical cost. A ledger and
not a view, because the condition is a **published byte**: permanent, unable to improve, and durable
evidence.

**At the LOCAL door only — and this deliberately breaks the ADR-0058 precedent it copies.**
`cairn_record_ceiling_flag` is called at *both* doors (db/005:825 and db/020:145), so a reviewer will
read local-only as an oversight and tidy it into symmetry. It is not, and the reason is that the two
conditions mean different things at the remote door:

- **Locally**, the node's own grade is authoritative for its own authoring, so an emitted rung finer than
  it licenses is **unambiguously anomalous** — the daemon's `apply_safety_rung` was bypassed. High
  signal, every time.
- **Remotely**, ADR-0063 decision 2 says this arrives *routinely and honestly*: an older peer predating
  the slice, a differently-custodial peer computing a lower grade, and a hostile peer all deliver exactly
  the same bytes, and the node cannot tell them apart. Flagging there would fire on ordinary traffic and
  accuse honest peers — §5.12 alert fatigue, in a ledger nobody could then trust.

A clock grade is a claim about the authoring node's own clock and stays meaningful at both doors; a
safety rung is a claim about *this* chart's grade, which is node-relative. Same idiom, different
question.

Read-time re-coarsening (ADR-0063 decision 2) already bounds the *effect*; this bounds the *silence*.

**Out of scope here, stays on #405:** part 1, the read-side `REVOKE SELECT (safety) ON event_log FROM
cairn_agent` question. It is a grant posture, not an authority question, and needs its own audit of
`SELECT *` by that role.

---

## 8. Running part C against it

The #380 attack, step by step. An enrolled actor on peer P wants the DEK for a sequestered body on
chart X:

1. authors an un-attested `sensitivity.grade-withdrawal.asserted` naming the chart-wide assertion;
2. it replicates and **is admitted** — nothing refused, nothing forked;
3. `cairn_claim_authority` returns `unverified`;
4. the withdrawal drops out of the set difference; the assertion still stands;
5. `cairn_effective_sensitivity` = `sequestered`;
6. part C's serve door withholds the DEK and ships the sealed ciphertext anyway (ADR-0052 E1's
   *withhold the key, never the bytes*);
7. the withdrawal appears on the worklist as `inert`.

**The forcing case passes**, and emission inherits it: `cairn_prospective_sensitivity` now chooses the
rung against the same unstrippable grade.

Three attacks fail cleanly, each pinned as a test rather than assumed (§11): manufacturing R2 by
authoring-then-withdrawing your own assertion lowers only your own, since the grade is MAX over standing;
an agent actor cannot buy R1, because the attester must resolve to `kind = 'human'`; and an attestation
cannot be lifted from another event, because the token is bound to its content address.

### The finding this hands forward to #376

**A custody dial derived from the effective grade is only as strong as its most-custodial holder.**

The grade is node-relative (ADR-0062 decision 9): a node with *more* custody legitimately computes a
*lower* grade, because the conservative bound collapses to the true value. So a well-custodied peer,
asked for the DEK, hands it out on a grade this node considers `sequestered` — and no amount of authority
hardening changes that. Authority protects a grade from being **stripped**; it cannot make a grade that
is honestly lower elsewhere be higher there.

An explicit custody act — a signed `custody.narrowed`-shaped event rather than a value derived from the
sensitivity stream — has no such property, because it is not derived from anything. That is #376's open
*"which subject feeds which dial"* with a new argument attached, and it is the most useful thing this
pass hands to part C. It is **not decided here.**

The other two #376 questions are untouched: nothing here bears on the decision-9 bound interaction, and
*nothing can un-know a DEK already fetched* remains part C's declared, forward-looking limitation.

---

## 9. What this does not fix, declared

**It buys accountability, not authorization.** #380 goes from *any enrolled actor, un-attested* to *any
enrolled human actor, attested and on the record*. It does **not** become "only someone with standing on
this chart," because that fact was rejected: it fails the locum, the night-cover registrar and the
receiving ED, and it is replication-relative besides.

This is the correct residual rather than a shortfall, and it is paper's own posture. On paper, anyone who
can reach the file can open the sealed envelope; what paper provides is not a lock but an unmistakable
record that it was opened — which is
[#377](https://github.com/cairn-ehr/cairn-ehr/issues/377)'s entire argument for break-glass. **The record
is the control and the gate is the forcing function**, and the ADR must say that in those words so no
reader infers a lock from a mechanism that is not one.

**A second axis of node-relativity.** ADR-0062 decision 9 established that the effective grade is
custody-relative. It is now custody-**and-actor-knowledge**-relative: a peer's honest attested withdrawal
reads `unverified` here until that attester is enrolled here, so the grade stays high — honest,
self-healing, and in the safe direction, but new. Two consequences:

- every cross-node equality test's `given equal custody` qualifier widens to
  **`given equal custody and equal actor knowledge`**, in the test's own name, for the reason ADR-0062
  already gives: stated loosely, such a test either fails spuriously or gets "fixed" by deleting the
  thing it guards;
- ADR-0062's UI guidance grows a second case — *a grade that falls when an attester enrols is not a bug
  report either.*

**Nothing here reaches an already-published byte.** ADR-0063's declared limitations stand unchanged.

---

## 10. Paper-parity benchmark (§1.2, CLAUDE.md house rule 7)

This changes a clinical workflow at the in-DB floor, so it carries a benchmark rather than the
forced-rationale escape.

- **Paper counterpart:** lowering a confidentiality marking on a paper chart — striking the restriction
  and initialling it.
- **Paper *N* = 1** human act (strike + initial: one signed act by a named person).
- **Architecture-forced *M* = 1.** The withdrawal carries the attestation the local door already demands
  of every locally-authored withdrawal (ADR-0062 decision 7). No new gesture is added for the clinician
  doing the work. **M = N — no architecture defect.**
- **UI bundling target *K* = 1** — unchanged; the attestation rides the existing sign gesture.
- **Time / cognitive load:** no change at the authoring surface, so no new budget is owed. The one
  changed experience is a *reading* one on a peer node — a grade that does not drop when expected — which
  the worklist (§6b) exists to explain. Budget: an operator must be able to answer *"why did this
  withdrawal not take effect?"* in **one query with no raw SQL**, satisfied by the `inert` row naming the
  reason.

The cross-node case has **no paper counterpart at all** — paper does not replicate — which is worth
stating rather than eliding, because it means paper-parity constrains the local half of this design and
is silent on the remote half.

---

## 11. Test plan (TDD — every test red first)

**Core behaviour**

1. An un-attested cross-node withdrawal **lands, converges and is readable** — and does **not** lower the
   effective grade. (The whole point; assert both halves, or a refusal would pass.)
2. An attested cross-node withdrawal lowers.
3. A locally-authored withdrawal **always** lowers — pins the no-deadlock claim rather than asserting it
   in prose.
4. **An advisory actor cannot strip its own auto-tag** (R2's `kind = 'human'` clause, ADR-0062
   decision 6). Without this test the clause is one "simplification" from gone.

**Arrival-order independence (ADR-0062 decision 3)**

5. Inert because the target has not replicated → effective once the target lands.
6. Inert because the attester is not enrolled → effective on enrolment.

**Anti-regression — the four that matter more than the features**

7. **A mutation test on the seam:** deleting the authority clause from `cairn_sensitivity_standing` must
   turn something red. This is #404's lesson applied on the first try — that class gate was pinned by
   *nothing*, and widening it left all 26 safety tests and 21 SQL mirrors green.
8. **A fresh-database load of the cairn-sync subset** — db/048 must still CREATE with the new reference.
   The eager-binding trap shows only on a fresh database, never on a replayed dev one.
9. **Rust↔SQL lockstep** between `classify_authorship_confidence` and `cairn_claim_authority`: the same
   contributor set, signer and attester must grade identically on both sides.
10. `has_function_privilege` pins for the new function, and the SQL mirror in `db/tests/`, run via
    `scripts/run-db-sql-tests.sh`.

**The three failed attacks (§8)**

11. Authoring-then-withdrawing one's own assertion lowers only that assertion.
12. An agent actor's attestation does not confer R1.
13. An attestation bound to another event's content address does not verify.

**Worklist**

14. Both rows appear for their conditions; the `inert` row disappears when the condition clears;
    `stranger-attested` persists.

**Row-count pins:** no new event type, no new projection, no ADR-0057 registry entry — **none of the four
counts move.** Stated so a reviewer can check rather than assume.

---

## 12. Files, and the repo's standing traps

| file | change |
|---|---|
| `db/005_submit.sql` | `cairn_claim_authority` + its grant, beside the existing predicate family |
| `db/048_sensitivity_stream.sql` | one clause in `cairn_sensitivity_standing`; the `sensitivity_withdrawal_worklist` view + grant |
| `db/049_safety_projection.sql` | `cairn_record_safety_overclaim_flag` + the local-door call (§7) |
| `db/tests/005_*`, `db/tests/048_*`, `db/tests/049_*` | SQL mirrors |
| `crates/cairn-node/tests/` | the suites in §11 |
| `crates/cairn-event/src/contributor.rs` | the lockstep comment gains its SQL twin's name |

**Traps this slice must not walk into** — each has bitten this repo already:

- **`LANGUAGE sql` binds eagerly at CREATE time.** The reason the predicate is in db/005 (§4). Verify on
  a genuinely fresh database *and* on the cairn-sync subset, not on a replayed dev one.
- **Postgres overloads on a changed argument list.** Fixed arity two from day one (§4). If it must ever
  change, db/005's `DROP FUNCTION IF EXISTS` idiom and every call site in one pass.
- **Never read `event_log.attester_key` directly.** `cairn_attestation_vouched` is the only way to ask
  (§3).
- **`SCHEMA_GENERATION`** rises if a new `db/0NN` is added; this design adds none, but the db/048 and
  db/049 edits still need the migration-replay discipline (`CREATE OR REPLACE`, idempotent).
- **A live IDE's rust-analyzer holds the shared `target/` lock** — use a scratch `CARGO_TARGET_DIR`.

---

## 13. Follow-ons to file

- **Part C's dial decision**, sharpened by §8: derive custody from the grade, or narrow it by an explicit
  custody act? Comment on [#376](https://github.com/cairn-ehr/cairn-ehr/issues/376) rather than opening a
  new issue.
- **[#245](https://github.com/cairn-ehr/cairn-ehr/issues/245)** narrows to the *display* half — the
  §5.10 authorship-confidence read surface. Its SQL mirror lands here; its consumer does not.
- **[#405](https://github.com/cairn-ehr/cairn-ehr/issues/405) part 1** (the `cairn_agent` column grant on
  `event_log.safety`) stays open and unaddressed here.
- **Break-glass ([#377](https://github.com/cairn-ehr/cairn-ehr/issues/377))** inherits `cairn_claim_authority`
  when it lands; if part C and part D each add a protection-removing act, the class reaches three members
  and the ADR-0048/0057 **registered-dispatch** refactor becomes worth taking. Recorded so it is a
  deliberate later step rather than a discovery.
