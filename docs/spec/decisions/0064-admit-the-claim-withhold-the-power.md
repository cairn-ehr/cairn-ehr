# ADR-0064 — Admit the claim, withhold the power

- **Status:** Accepted
- **Date:** 2026-08-15
- **Derives from:** [ADR-0056](0056-unknown-event-types-admitted-uninterpreted.md) (*custody is total;
  interpretation is deferred; **power is earned***) — this ADR is that last clause given a mechanism —
  and [ADR-0052](0052-born-sealed-clinical-bodies.md) §4 + erratum E1 (*confidentiality lives in key
  custody, never in withholding rows*; the serve door **withholds custody, never the events**) — the
  shape [#231](https://github.com/cairn-ehr/cairn-ehr/issues/231) already resolved without anyone
  naming it as a general rule
- **Applies:** principle 2 (never erase, always overlay — the withdrawal stays in the log) · principle 3
  (paper-parity — striking and initialling a confidentiality marking) · principle 4 (acknowledged
  uncertainty, [ADR-0003](0003-bitemporal-time-and-acknowledged-uncertainty.md) — *uncertainty withholds
  power, it never confers it*) · principle 9 (policy-neutral infrastructure,
  [ADR-0005](0005-erasure-key-custody-and-crypto-shredding.md)) · principle 12 (the floor is in the
  database, [ADR-0021](0021-layering-the-node-api-and-ui-pluralism.md)) ·
  [ADR-0011](0011-actor-registry-version-pinning-and-key-custody.md) (`actor_id` as the identity that
  survives key rotation) · [ADR-0051](0051-contributor-role-vocabulary-floor-and-responsibility-wire-shape.md)
  (the shared-pure-classifier discipline) · [ADR-0053](0053-per-write-human-authorship.md) (the
  attestation this ADR consumes) · [ADR-0058](0058-grade-gated-teffective-ceiling.md) (the
  record-a-flag-rather-than-refuse idiom, and its deliberate divergence here) ·
  [ADR-0060](0060-partial-validity-a-defect-on-one-line-never-invalidates-another.md) (*an advisory
  field may never cancel clinical content*) ·
  [ADR-0062](0062-the-sensitivity-stream-and-the-inverted-unknown.md) (decisions 3, 6, 7 and 9 are all
  load-bearing here) · [ADR-0063](0063-the-safety-projection-and-the-seal-as-coarsening-boundary.md)
  (decision 2, and its decision 8's categorical rule)
- **Canonical spec home:** [identity §5.9](../identity.md#59-sensitivity-grade-the-safety-projection-and-break-glass-visibility-scope)

## Context

Three findings — one closed, two open — are the same defect wearing three costumes:

| | surface | what the floor validated | what it never asked |
|---|---|---|---|
| [#231](https://github.com/cairn-ehr/cairn-ehr/issues/231) (closed) | the unwrap-cert serve door | the cert's own signature and self-consistency | whether its `kid` was in the trust set |
| [#380](https://github.com/cairn-ehr/cairn-ehr/issues/380) | `sensitivity.grade-withdrawal.asserted` | the claim's shape (a non-empty rationale, at both doors) | whether anyone accountable stood behind lowering the grade |
| [#405](https://github.com/cairn-ehr/cairn-ehr/issues/405) part 2 | `EventBody.safety` | the signal's shape (`cairn_check_safety_signal`) | whether the chart's grade licensed the rung claimed |

In each case the floor established that a claim was **well-formed** and then treated
well-formedness as **authority**. [§5.9](../identity.md#59-sensitivity-grade-the-safety-projection-and-break-glass-visibility-scope)
part C ([#376](https://github.com/cairn-ehr/cairn-ehr/issues/376), sequester) would add a fourth
instance, and the first where the cost of being wrong is a **DEK** rather than a wrongly-rendered
line: a sequester keyed on the effective grade inherits #380 directly — strip the grade un-attested
and the serve door computes `routine` and hands out the key.

**This ADR fixes the general defect and does not decide part C.** Part C was used throughout as the
falsification case (*does this let sequester key on a grade nobody can silently strip?*); the one
sharpened finding it produced is recorded below as an input to #376, not as a decision taken here.

The primitive is not new. It is ADR-0056's *custody is total; interpretation is deferred; power is
earned*, and #231's own resolution — *the serve door withholds custody, never the events; the bodies
still replicate as sealed ciphertext* (ADR-0052 erratum E1) — compressed here to *withhold the key,
never the bytes*. One rule, one mechanism:

> **Admit the claim; withhold the power.**

That this frame post-hoc explains a hole already fixed — by an argument nobody made at the time — is
the best evidence available that it is the right frame rather than a story fitted to three bugs.

**The doctrine was already written down in three places and implemented in none.**
`classify_authorship_confidence` (`crates/cairn-event/src/contributor.rs:167`) is pure, total,
property-tested and carries a comment recording that **no read path consumes it**
(`contributor.rs:112-115`, [#245](https://github.com/cairn-ehr/cairn-ehr/issues/245) — still true of
its display half after this slice). `db/005_submit.sql:635` states the rule
outright — *"apply admits and GRADES (`classify_authorship_confidence`), never refuses"* — as the
reason `cairn_authorship_bound` is strict-door-only. `cairn_ceiling_classify` plus the append-only
`t_effective_ceiling_flag` (`db/040_clock_confidence_grade.sql:53,76`, ADR-0058) is a shipped **third
verdict** — *admit, and record* — used in exactly one subsystem. So the floor already had a vocabulary
for authority, a mechanism for a non-refusing verdict, and a doctrine saying to use both. Every dial
nonetheless read a claim's shape and stopped.

## Decision

### 1. Authority is a human actor this node can hold responsible

Not the relaying machine, not the actor's relationship to the chart, not the peer's membership of the
trust set. Two sufficient routes, evaluated by one predicate
(`cairn_claim_authority`, `db/005_submit.sql:704`):

```
authority(claim, target) =
    'attested'    if R1
    'self'        if R2
    'unverified'  otherwise
```

**R1 — vouched human attestation.** The claim's row carries a non-NULL `attester_key`,
`cairn_attestation_vouched(event_id)` is true, and that key resolves in `actor_current` to **exactly
one** actor, which is `kind = 'human'` (`db/005:710-718`).

Three conjuncts, each load-bearing for a different reason:

- **The `attester_key IS NOT NULL` guard is the actual test.** `cairn_attestation_vouched`
  (`db/001_envelope.sql:502`) returns TRUE for an event carrying **no attestation at all**, because
  "vouched" means *no unvouched marker row exists*. Delete the NULL guard and every unattested event
  in the log grades `'attested'`.
- **The vouch predicate is not optional politeness.** The **apply** door stores
  `event_log.attestation` / `attester_key` even when the token was **not** verified as a vouch — the
  deferred arm (`db/020:239-242`) carries it so a later re-adjudication has something to verify — and
  marks the row in `event_attestation_unvouched` (`db/020:482`, the only INSERT of that marker
  anywhere). The **local** door never does: it verifies first and RAISEs
  (`db/005:958-963`), so a stored `attester_key` there is always a vouch. db/001:490 names
  `cairn_attestation_vouched` as *"the ONE way to ask"* and predicts *"a fifth will arrive with the
  next type that reads `event_log.attester_key`."* This is that fifth. Reading the column directly
  would grade a carried-but-unverified token as attested — the very defect this ADR exists to close,
  one layer down, and it is reachable through exactly one door rather than none.
- **Exactly one actor, deliberately stricter than db/020's sibling.** The apply door's
  forged-human-author gate asks `EXISTS (… AND kind = 'human')`
  (`db/020_apply_remote_event.sql:251-254`), which admits a key mapped to both a human and an agent.
  Here that ambiguity would confer the power to strip a protective grade, so the key must resolve to
  one actor and that actor must be human. Principle 4: **uncertainty withholds power, it never confers
  it.** This is not a fix to db/020 — it is a different question asked at a different door.

**R2 — human self-withdrawal.** The claim's `event_log.actor_id` equals the target's, **both are
non-NULL**, and that actor is `kind = 'human'` (`db/005:720-728`).

- `actor_id` rather than `signing_key_id`, so the identity survives key rotation within one actor
  (ADR-0011: `actor_id` content-addresses the pinned determinant set, and the key is deliberately not
  in it).
- A NULL `actor_id` — a key concurrently mapped to several actors, attribution honestly unknown — is
  **not** self. The same principle-4 reading as R1's third conjunct.
- **The `kind = 'human'` clause enforces ADR-0062 decision 6 structurally.** Without it the advisory
  matcher that auto-applied a protective tag could strip its own tag with no human anywhere in the
  loop — exactly what decision 6 forbids (*ADR-0043's "agent advisories are dismissable by anyone"
  does not reach a protective auto-tag*). With it, both routes say the same thing and decision 6 stops
  being prose that a future reader has to remember. Pinned by
  `claim_authority.rs::an_advisory_actor_cannot_self_withdraw_its_own_protective_tag`.

**Fixed arity two, from day one.** A caller with no target passes an explicit `NULL` and gets R1 only.
Postgres **overloads** on a changed argument list rather than replacing, and migration replay never
drops what a file stops creating — so a later 1-arg → 2-arg widening would leave a stale definition
alive in every existing database, silently serving any call site missed, including
`has_function_privilege` pins, which would resolve the *stale* signature and pass. This project has
paid for that lesson twice already (`db/005:787-790` had to `DROP FUNCTION` the 3-arg `submit_event`
when ADR-0052 added `p_dek`; `db/020:55` records the same), and `db/049:345` states it as a verified
hazard. Avoided here by never creating the second signature.

**`SECURITY DEFINER` is load-bearing, not stylistic** (`db/005:706`). `cairn_attestation_vouched` is
`REVOKE EXECUTE … FROM PUBLIC` and `event_attestation_unvouched` carries no SELECT grant, because
db/001:507-509 reasons that *every caller is a SECURITY DEFINER door or a migration-owned trigger*. This
caller is neither: `cairn_sensitivity_standing` is a plain `LANGUAGE sql` function granted to
`cairn_agent`, and a non-definer body runs as the **calling** role whether or not it inlines. Without
`SECURITY DEFINER` the first `cairn_effective_sensitivity` call by `cairn_agent` fails with permission
denied — **the entire sensitivity read path, broken by a privilege, and only under the product's
role**, invisible to a suite running as the owner. Pinned by two role-switched tests, on purpose:
`claim_authority_worklist.rs::the_worklist_is_readable_as_cairn_agent` lands a real withdrawal and
reads `sensitivity_withdrawal_worklist` as `cairn_agent`, so `cairn_claim_authority` actually runs
under that role against live data — the stronger pin. `claim_authority.rs::the_read_path_works_as_
cairn_agent` reads a chart carrying no withdrawal, so the predicate is never *called* there and its
non-vacuity rests on Postgres's executor-start ACL check alone; still a real pin (a missing grant
fails before the predicate would ever run), but the weaker of the two. If the named test in either
codebase ever gets simplified, the `SECURITY DEFINER` pin should be re-anchored to whichever of the
two still lands real data through the role switch, not silently dropped.
`REVOKE … FROM PUBLIC` + `GRANT … TO cairn_agent` follows immediately (`db/005:734-735`): a definer
function with PUBLIC execute is a privilege-escalation surface.

### 2. Authority gates *effect*, never *admission*, and only in the withholding direction

Nothing is refused at either door. A claim below the bar lands, converges, is readable, appears in
chart history and is re-assertable; it simply does not participate in the set difference that computes
what still applies. **A claim below the bar may always raise protection and may never lower it.**

This is why ADR-0062 decision 7 is not reopened. Decision 7 keeps the withdrawal ceremony
local-door-only for two reasons, and **neither reaches a mechanism that refuses nothing**:

1. *A door check at apply forks the event set* (the [#342](https://github.com/cairn-ehr/cairn-ehr/issues/342)
   trap, hit four times in this project). No refusal, so no fork; set-union losslessness is untouched.
2. *For a raise, the refusal is itself a disclosure.* Only lowering is gated, so a protective act is
   never impeded by any path through this design.

So #380's dilemma dissolves rather than being traded off: the ceremony's authorship half stops being
local-door-only **not by becoming a refusal at apply, but by becoming a condition of effect**.

**And it does not deadlock.** ADR-0062 rejected *self-only* withdrawal because it deadlocks — the
asserting clinician retired, the patient left, the advisory actor was superseded. This is not
self-only: R1 is the primary route and R2 an alternative. More importantly, **the local door already
demands a bound human author for a withdrawal** (`db/048_sensitivity_stream.sql:1134-1135`, and
db/005:1191 passes it the attester key the door itself verified), so **every locally-authored
withdrawal clears the bar by construction — modulo the dual-mapped-key registry state noted under
*Known limitations***: the local door's gate asks `EXISTS (… AND kind = 'human')`
(`db/005:961-963`), while R1 asks for **exactly one** such actor (`db/005:715-717`), so a key mapped
to a human *and* an agent passes db/005 and still grades `unverified`. That is the same delta this ADR
raises against db/020, and the local door is not exempt from it. The only *ordinary* claims that go
inert are cross-node ones with no human behind them, and for those the remedy is not re-asserting the
grade — it is *attesting the withdrawal*, which is what the local door demands of everyone anyway.
Pinned by `claim_authority.rs::a_locally_authored_withdrawal_always_lowers`.

### 3. The bar is a fixed floor, not a deployment threshold

Principle 9's policy-neutrality governs **what** is confidential — which is why the category blacklist
and `safety_class_map` both ship empty. It does not govern **whether a protection-removing act must be
accountable**. A deployment-configurable authority threshold would make "may an anonymous claim strip
a grade?" a settings question, and the first deployment to answer *yes* would silently reproduce #380
in full while believing it had the control. There is no dial.

### 4. The verdict is computed at read, never stamped at apply

A withdrawal that names a target which has not replicated here yet is inert on **two** counts, and both
legitimately improve. **R2 cannot resolve at all** until the target event is present, since it compares
the two events' `actor_id`s (R1 still carries an attested claim on its own). And the **seam** has no
assertion row to match until the target's projection lands, so even an already-`attested` withdrawal
moves nothing until then. Both heal the moment the target arrives — no re-apply, no second event, no
stamped verdict to go stale. Pinned by
`claim_authority.rs::a_withdrawal_inert_because_its_target_has_not_replicated_heals_when_it_lands`,
which drives the attested shape (so the verdict reads `attested` throughout and what heals is the
*effect* — the strictly harder case to get right, because a delete-at-apply design would already have
dropped the withdrawal on the floor).

> [!IMPORTANT]
> **The sibling axis — an attested withdrawal whose attester is not yet enrolled here — is NOT
> reachable, and the design that preceded this ADR was wrong to say it was.** It predicted a "second
> axis of node-relativity" in which a peer's honest attested withdrawal would read `unverified` here
> until that attester enrolled. That never happens.
> `sensitivity.grade-withdrawal.asserted` is a **classified** type (`db/048:76`), so
> `apply_remote_event` takes its non-deferred branch, and an R1-eligible (responsibility-bearing)
> withdrawal whose attester does not resolve to an enrolled human actor is **refused outright**
> (`db/020:251-254`) — the event never lands, so it can never sit here inert waiting to heal. The
> deferred arm that stores a token *without* verifying it (`db/020:239-242`) is reached only by
> **unclassified** types, which a withdrawal never is. Verified empirically during the build, with a
> throwaway test driving exactly that shape through the apply door and observing the refusal.
>
> **What is unreachable is the *arrival-time* gap only.** Do not generalise past that, because there
> **is** a second live axis and it is the mirror image of this one: **actor-registry state that changes
> *after* admission.** Both routes resolve their actor through `actor_current` (`db/005:715-717`,
> `:712-717`), and `actor_current` **excludes a revoked actor** (`db/004_actors.sql:64-68`). So
> revoking an attester — or the self-withdrawer — after their withdrawal has landed flips it from
> `attested`/`self` to `unverified`, the withdrawal drops out of the set difference, the assertion
> **re-stands**, and the grade goes back up. Confirmed by a throwaway check against `db/004` in a
> scratch database during the build — not by a committed test, so treat this the same as the deferred-arm
> claim above: before a revoke both R1's actor-resolution conjunct and R2's join read true; after it both
> read false, and the R1 conjunct is `false` rather than `NULL` on zero rows, so the arm is cleanly
> false.
>
> So: **two live divergence axes, not one** — the unreplicated target (which heals) and post-admission
> registry change (which does not). The second is a direct consequence of computing authority at read,
> which is otherwise the property that makes everything here self-heal; you cannot have the healing
> without it. Its **direction is safe** — protection is restored, never removed, and nothing becomes
> readable that was not readable before — which is why it is a limitation and not a defect. See
> *Known limitations* and [#409](https://github.com/cairn-ehr/cairn-ehr/issues/409).
>
> **ADR-0062's *given equal custody* qualifier still stands unchanged.** The design's proposed widening
> to *"given equal custody **and equal actor knowledge**"* is **not** the right repair and must not be
> reintroduced: the arrival-time knowledge gap it named is unreachable. The honest statement is a
> different one — a cross-node equality comparison also assumes **equal actor-registry state**, which is
> a property of what each node has *revoked* — and, because `actor_event` replicates like any other
> event, of what it has had the chance to *learn* too; the two are not independent axes.

### 5. One predicate, consulted at exactly one site per dial

> Authority is consulted where a protection-removing act's **effect** is computed — exactly one site
> per dial — never at each consumer.

The whole of §5.9's protection-removing control is **one clause** in `cairn_sensitivity_standing`
(`db/048:368-378`), the single definition of *what still applies*:

```sql
    WHERE a.patient_id = p_patient_id
      AND NOT EXISTS (SELECT 1 FROM sensitivity_withdrawal w
                       WHERE w.withdraws = a.content_address
                         AND w.patient_id = p_patient_id
                         AND cairn_claim_authority(w.event_id, a.event_id) <> 'unverified');
```

Three consumers inherit it with no change to any of them: `cairn_effective_sensitivity` (db/048 §11,
display coarsening), `cairn_prospective_sensitivity` (`db/049_safety_projection.sql:274-278`, so the
emitted safety rung is chosen against a grade that cannot be silently stripped), and
`crates/cairn-node/src/sensitivity.rs:319` (the CLI read path). Part C's custody dial inherits it the
same way — **structurally, not by anyone remembering**.

This is the anti-drift decision, and it is why the seam matters more than the predicate. `standing` is
the one definition the two otherwise-duplicated read models genuinely share; ADR-0063 records that
duplication as its single largest drift risk and #404 is that risk realised. A per-dial authority check
would put the same judgement in three places that have already been shown to diverge.

**Home: `db/005_submit.sql`, and there is no second candidate.** `cairn_sensitivity_standing` is
`LANGUAGE sql`, which resolves references **eagerly at CREATE time**, so a predicate defined in a new
`db/050` would not exist when db/048 loads and a genuinely **fresh** database would fail there — the
failure db/005's own comment already documents for `cairn_clear_payload` and db/034. Converting
`standing` to plpgsql for late binding is the wrong trade: it is a `LANGUAGE sql` STABLE function the
planner can **inline**, and it sits on the read path of every safety-projection read. db/005 is in both
schema lists, loads before db/048, and already hosts this predicate family —
`cairn_claim_authority` is the read-side twin of `cairn_authorship_bound` directly above it (*the same
question at opposite doors: that one refuses at authoring, this one grades at read*). Pinned on a real
fresh load by `crates/cairn-sync/src/main.rs:4885`, which **drives** both functions on the cairn-sync
subset rather than merely loading them — [#386](https://github.com/cairn-ehr/cairn-ehr/issues/386)'s
lesson applied on the first try.

### 6. Flag what cannot self-heal; view what can

Both idioms now live in one subsystem, so the rule for choosing is stated rather than left for the next
author to guess.

**`sensitivity_withdrawal_worklist` is a VIEW** (`db/048:920-959`), because authority is computed at
read precisely **because the answer improves**. An apply-time ledger would fill with rows that were true
for an afternoon, and a worklist that is mostly stale is
[§5.12](../identity.md#512-the-notification-economy-salience-responsibility-routing-and-the-acknowledgment-floor)'s
alert-fatigue disease, self-inflicted, in the one place we are building a control. A view is always
accurate, self-clearing, has no table, moves none of the four **registry** row-count pins, and has no
interaction with `cairn_execute_shred`'s scrub; nothing is lost that is not reconstructible from the
append-only log at any time. (It does carry a contract of its own: `db/tests/048:269-290` pins the
view's seven columns, their order and their types against `information_schema`, because
`CREATE OR REPLACE VIEW` happily *appends* a trailing column and `ALTER VIEW … RENAME COLUMN` bypasses
its protections entirely.)

**`safety_overclaim_flag` is a LEDGER** (`db/049:508-526`), because its condition is a **published
byte**: permanent, unable to improve, and durable evidence. See decision 7.

Two rows, two reasons:

| reason | condition | clears when |
|---|---|---|
| `inert` | authority is `unverified`, **and** the target assertion is still standing (or has not replicated yet) | the target replicates and R2 then resolves (a self-withdrawal), **or** any other authoritative withdrawal strips the same target. Otherwise it stays listed: an un-attested row's own verdict is fixed forever, and the remedy is a properly attested withdrawal |
| `stranger-attested` | authority is `attested`/`self`, and the accountable human had **no prior presence on this chart at the moment of the strip** | **nothing the flagged actor does afterwards** can clear it — it is a fact about a completed act. An event they authored *before* the strip and that replicates *later* does clear it, correctly: it reveals prior presence this node could not yet see |

The second row is the **detect** half of #380 that survives the gate: an attested strip by a human with
no business on that chart clears the bar and would otherwise be invisible, because nothing else in the
system watches withdrawals. It reuses the chart-standing fact this ADR **rejects as an authority input**
— deliberately, and the distinction is the point. As authority it fails the locum, the night-cover
registrar and the receiving ED, who must not be second-class. As **salience** it carries none of that
cost: it blocks nothing, delays nothing, and the stranger's withdrawal takes effect immediately. This is
[§5.13](../identity.md#513-locale-pluggable-comparators-the-matcher-extension-point)'s duplicate-sweep
posture — *surface, never block*.

Three properties of these rows **depart from the design** and are decisions in their own right, because
each of them is one "simplification" away from making the row useless.

> [!WARNING]
> **`stranger-attested` asks about presence on the CHART, not about the node of origin — and the
> node-origin half of the design was abandoned, not implemented.** The design specified *"authored on
> another node **and** the actor has no prior authored content on this chart."* Only the second half
> exists. Two independent reasons, both verified:
>
> - **There is no honest way to ask the question.** The only per-node identity this schema tracks is
>   `local_node` (db/007), and db/007 — `trust_peer` included — is **deliberately absent** from the
>   cairn-sync subset (`crates/cairn-sync/src/main.rs:176`); a `LANGUAGE sql` view binds relations at
>   CREATE time, so referencing it would have failed db/048's load on a sync node and **taken clinical
>   sync down**. And `node_origin` on every row is copied **verbatim** from the event body's own
>   self-asserted `hlc.node_origin`, identically at both doors — a client-chosen string. Gating a
>   security-adjacent worklist on a value the peer you want to catch simply chooses is not a control.
> - **Locality was never what the row detected.** The consequence, stated out loud because the design's
>   own wording excluded it: **a local clinician with no prior presence on a chart now also appears on
>   the worklist.** That is the signal, not noise. A locally-authored strip by someone who has never
>   touched this chart is exactly as unaccountable as a remote one; presence on the chart is what made
>   it accountable, and node-of-origin never was. `node_origin` is carried through as a plain output
>   column for an operator's own investigation, and never drives `reason`.

**"No prior presence" is bounded in time — events at or before the withdrawal, compared on
`(hlc_wall, hlc_counter)`** (`db/048:958-959`). Without the bound the flagged actor deletes their own
flag by writing anything else on the chart afterwards: a locum strips a grade, documents the consult ten
minutes later, and the only record of the strip evaporates before anyone triages it. **In a design whose
own position is that the record is the control, a record the flagged party can clear is not a control.**
The row asks whether the actor had a relationship to this chart **at the moment they stripped its
protection**; later activity answers a different and less useful question. An event authored *before*
the withdrawal but replicating here *after* it still clears the row — correctly, and consistently with
the arrival-order self-healing everywhere else in this slice: it reveals that the actor genuinely did
have prior presence and this node simply could not see it yet. Pinned in both directions by
`claim_authority_worklist.rs::a_stranger_who_later_writes_on_the_chart_stays_listed` and
`::a_strangers_earlier_presence_that_replicates_late_clears_the_row`.

**`inert` gates on the target assertion still standing, not only on the row's own verdict**
(`db/048:947-950`). A withdrawal's own verdict can never change once it has landed un-attested —
`attester_key` on an admitted row is fixed forever — so a naive per-row reading would list it `inert`
**forever**, even after a second, authoritative withdrawal has stripped the same target. That is noise
about a problem already solved: the exact alert fatigue the view exists to avoid. Asking *is the target
still standing* makes an inert row self-clear the moment **any** accountable route achieves the same
effect. A target that has simply not replicated yet is not in `sensitivity_assertion` at all, and must
still be listed — so "not landed" and "landed and still standing" are treated as the same
still-worth-watching case, and only "landed and already stripped elsewhere" as moot.

### 7. #405 part 2 — the same rule, the other branch: record the over-claimed rung

`cairn_check_safety_signal` validates shape only and never compares the claimed rung against what the
chart's standing grade licenses, so a client with direct DB access (Spike-0002's C1–C5 threat model,
treated here as live) can submit `{"rung":"precise","class":"antiretroviral-interaction"}` on a
`sequestered` chart.

The door **cannot refuse it**: ADR-0060 forbids an advisory field cancelling a medication assert, and
the door cannot rewrite `event_log.safety` without making the column disagree with `signed_bytes` and
quietly breaking the signature's meaning. So it takes the ADR-0058 idiom —
`cairn_record_safety_overclaim_flag` recording `emitted_rung_rank < licensed_rung_rank`
(`db/005:1142-1146`), making the bypass auditable at zero clinical cost.

**At the LOCAL door only — and this deliberately breaks the ADR-0058 precedent it copies.**
`cairn_record_ceiling_flag` is called at *both* doors, so a reviewer will read local-only as an
oversight and tidy it into symmetry. It is not, and the reason is that the two conditions mean different
things at the remote door: **locally** the node's own grade is authoritative for its own authoring, so a
finer rung is unambiguously anomalous (`apply_safety_rung` was bypassed); **remotely** ADR-0063 decision
2 says the identical bytes arrive *routinely and honestly* from an older peer, a differently-custodial
peer and a hostile peer alike, and the node cannot tell them apart, so flagging there would fire on
ordinary traffic and accuse honest peers. A clock grade is a claim about the authoring node's own clock
and stays meaningful at both doors; a safety rung is a claim about *this* chart's grade, which is
node-relative. Same idiom, different question.

Read-time re-coarsening (ADR-0063 decision 2) already bounds the *effect*; this bounds the *silence*.

### 8. "Over-coarsening is safe" is an emission-side frame, and it inverts on a detector

*This decision was not in the design; the build forced it, and it is the sharpest thing this slice
learned.*

The first implementation of decision 7's check sat above db/005's unseal and passed `p_thread = NULL`
to the grade lookup, characterised in its own comment as *the conservative bound — it can only
over-coarsen, so it is never more permissive*. That sentence is true of **emission** and false of a
**detector**, and the difference is the whole of this decision:

- Coarsening a **disclosure** withholds information. The error direction is *told less than entitled*,
  which is recoverable and is why ADR-0063 makes over-coarsening the safe default everywhere on the
  emission path.
- Coarsening the **licensed** rung inside an overclaim comparison makes the test **stricter**. The
  error direction is *accused of an overclaim never made* — a **false positive**, and there is nothing
  safe about it.

The concrete failure: on a chart carrying a thread-scoped grade, a coded medication on a **different,
ungraded** thread is entitled to emit `precise` — that is exactly what #404's fix guarantees — but the
NULL lookup took ADR-0062 decision 9's chart-wide bound, computed `existence`, and recorded an
overclaim **against the daemon's own correct output on ordinary traffic**. An existing, passing test
(`crates/cairn-node/tests/safety_emission.rs:333`,
`a_grade_on_another_thread_of_the_same_chart_does_not_coarsen_this_one`) was already driving exactly
that shape and writing a false row — it passed only because nothing read the ledger.

The justification was also wrong on a plain fact: the comment claimed `payload.medication_id` was no
longer readable at that point in the door. It is — `b_clear` is built by db/005 step 7 and steps 8/8a/8b
already read it; the block was simply placed above the unseal. It now sits below it and resolves the
same thread emission resolved (`db/005:1119-1132`), inside the same nested
`BEGIN … EXCEPTION WHEN OTHERS` wrapper (so ADR-0063 decision 8's categorical rule still holds — an
advisory ledger entry may never fail a clinical write), and still before the `event_log` INSERT so a
later refusal rolls the flag back with everything else.

**The general rule, stated so it survives this file:** *a ledger whose rows are mostly false accusations
against the system's own correct output is worse than no ledger* — eventually nobody reads it, and the
true row arrives into a surface already discredited. **A detector must reproduce the emitter's inputs
exactly; "conservative" is not a defence when the conservatism runs toward accusing.** This is the kind
of mistake that survives review precisely because it invokes the project's own safety principle in the
one place that principle does not apply.

### 9. The control is the record; the gate is only the forcing function

This buys **accountability, not authorization**, and the ADR says so in those words so no reader infers
a lock from a mechanism that is not one. See *Known limitations*.

## Paper-parity benchmark (§1.2)

This changes a clinical workflow at the in-DB floor, so it carries a benchmark rather than the
forced-rationale escape.

- **Paper counterpart:** lowering a confidentiality marking on a paper chart — striking the restriction
  and initialling it.
- **Paper *N* = 1** human act (strike + initial: one signed act by a named person).
- **Architecture-forced *M* = 1.** The withdrawal carries the attestation the local door **already
  demanded** of every locally-authored withdrawal (ADR-0062 decision 7,
  `db/048:1134-1135`). No new gesture is added for the clinician doing the work.
  **M = N — no architecture defect.**
- **UI bundling target *K* = 1** — unchanged; the attestation rides the existing sign gesture.
- **Time / cognitive load:** no change at the authoring surface, so no new budget is owed. The one
  changed experience is a *reading* one on a peer node — a grade that does not drop when expected —
  which the worklist exists to explain. Budget: an operator must be able to answer *"why did this
  withdrawal not take effect?"* in **one query with no raw SQL**, satisfied by the `inert` row naming
  the reason. That budget is **owed, not met**: no shipped surface reads the view yet (see *Known
  limitations*).

**The cross-node case has no paper counterpart at all** — paper does not replicate. Worth stating
rather than eliding, because it means paper-parity constrains the local half of this design and is
silent on the remote half.

## Rejected alternatives

**Trust-set origin as an authority fact** (*"the event came from an admitted peer"*). Rejected twice
over. **Any** admitted peer satisfies it, which is #380's own shape one layer up: a membership check
mistaken for an accountability check, admitting exactly the actor the mechanism is supposed to catch.
And `trust_peer` lives in db/007, which is **deliberately absent** from the cairn-sync subset
(`crates/cairn-sync/src/main.rs:176`) — a `LANGUAGE sql` reference to it inside
`cairn_sensitivity_standing` would have failed to create on the sync node and **taken clinical sync
down entirely**. The second reason is the cheaper one to discover and the first is the one that
matters.

**Prior standing on the chart as *authority*.** The intuitive answer, and it is wrong in the direction
that hurts patients. It fails the **locum**, the **night-cover registrar** and the **receiving ED** —
none of whom has ever touched the chart, and all of whom must be able to act on it without being
second-class. It is also **replication-relative**: whether an actor has "prior content here" depends on
what has arrived at this node, so the same act would be authoritative on one peer and not on another,
which is precisely the property authority must not have. Retained as **salience only** (decision 6),
where it costs nothing because it blocks nothing.

**A deployment-configurable authority threshold.** Rejected under decision 3. Policy-neutrality is
about *what is confidential*, not about *whether removing protection must be accountable*; a dial here
would let a deployment reproduce #380 in full while believing it had the control.

**A per-dial authority check** — each consumer asking the question for itself. This is the #404/#399
drift shape exactly: `cairn_prospective_sensitivity` and `cairn_effective_sensitivity` are already a
hand-maintained mirror pair, they have **already diverged once** (#404, where a thread arm made
`p_thread` inert and emission disagreed with read on the same node), and #399 records that the
anti-drift test did not cover the arms that drifted. Putting the same judgement in three places that
have demonstrably diverged, when it can be put in the one definition they all delegate to, is a defect
chosen on purpose. Rejected in favour of the seam (decision 5).

**An apply-time flag ledger for withdrawals** (the `t_effective_ceiling_flag` idiom applied here).
Rejected under decision 6: authority is computed at read *because the answer improves*, so a stamped
verdict is a row that was true for an afternoon. **Flag what cannot self-heal; view what can.**

**The node-origin half of `stranger-attested`.** Specified in the design, abandoned in the build — see
decision 6's warning callout. Both reasons are independently sufficient: it would have taken clinical
sync down on the cairn-sync subset, and it gates on a self-asserted client string that the peer you
want to catch simply chooses.

## Known limitations

**It buys accountability, not authorization.** #380 goes from *any enrolled actor, un-attested* to *any
enrolled human actor, attested and on the record*. It does **not** become "only someone with standing on
this chart", because that fact was rejected above. This is the correct residual rather than a shortfall,
and it is paper's own posture: on paper, anyone who can reach the file can open the sealed envelope, and
what paper provides is not a lock but an unmistakable record that it was opened — which is
[#377](https://github.com/cairn-ehr/cairn-ehr/issues/377)'s entire argument for break-glass. **The
record is the control and the gate is only the forcing function.** Do not read a lock into a mechanism
that is not one.

**Revoking an actor silently re-raises every grade they lawfully declassified.** The unavoidable price
of decision 4: both routes resolve their actor through `actor_current`, which excludes a revoked actor
(`db/004_actors.sql:64-68`), so a revoke landing *after* a withdrawal flips it to `unverified`, the
assertion re-stands, and the grade goes back up — with no event recording that anything changed, and
with the worklist re-populating with `inert` rows that no longer heal (an admitted row's `attester_key`
is fixed forever, and only a fresh authoritative withdrawal of the same target clears them). The
direction is **safe** — protection restored, never removed — which is the only reason this is a
limitation rather than a defect.

**Whether it is *right* is genuinely undecided, and this ADR does not decide it**
([#409](https://github.com/cairn-ehr/cairn-ehr/issues/409)). Two readings are defensible.
Contamination-cascade logic (ADR-0011 / ADR-0029) says authority follows revocation: a recall exists
because we no longer trust what that actor did, and a stolen key that strips grades is the #380 attack
with a valid signature. Clinical reality says most revocations are mundane — a clinician leaves — and
silently re-sealing charts they lawfully opened, months later, is a state change with no author, which
sits badly beside principle 1 and beside this ADR's own position that the record is the control. Stated
here rather than left to be discovered by an operator wondering why a chart resealed itself.

**Neither new surface has a shipped reader.** `sensitivity_withdrawal_worklist` and
`safety_overclaim_flag` both carry `GRANT SELECT … TO cairn_agent` and are exercised by tests; nothing
in the workspace surfaces either to an operator. Stated rather than left as an implied capability —
ADR-0063 recorded the same defect one field over (a value written by every coded verb and read by
nothing), and the §1.2 budget above is therefore **owed, not met**. The operator surface is
[#388](https://github.com/cairn-ehr/cairn-ehr/issues/388)'s (*§5.9 operator surface is blind to
withdrawals, deferred grades, and custody-less charts*).

**A cross-chart mis-targeted withdrawal that stays UNVERIFIED is permanently inert AND permanently
invisible.** `cairn_sensitivity_standing` is patient-scoped on both sides — which is load-bearing,
because without it a withdrawal authored on chart B could strip chart A's protection. The consequence
is that such a withdrawal, mis-stamped with the **wrong** chart's `patient_id` and naming a real
assertion that lives on a **different** chart, finds nothing in `cairn_sensitivity_standing(w.patient_id)`
on any read, ever — and so also falls out of the worklist's `inert` arm, which asks whether the target
still stands *on the withdrawal's own chart*, where it never did. **This holds only for
`verdict = 'unverified'`.** An AUTHORITATIVE (`'attested'`/`'self'`) mis-chart withdrawal is different:
the worklist's second arm (`db/048:887-895`) checks the responsible actor's prior presence on the
WITHDRAWAL'S OWN chart, not the target's, so an actor with no such presence there DOES surface — as
`stranger-attested`. Neither door refuses either shape: the ceremony's chart-mismatch checks are in the
*assertion* branch only. Harmless clinically in both cases — the strip never took effect anywhere — but
for the unverified case it is exactly the kind of silently-ineffective act a worklist would want to
show, and no arm of the view names the actual condition — even the visible `stranger-attested` row for
the authoritative case reads as an ordinary stranger attestation, not as "wrong chart". Recorded as a
`KNOWN GAP` comment at `db/048:826-838` and here; not fixed, and not exercised by any test.

**The Rust↔SQL authority mapping is already violated by door-admissible shapes, in both directions.**
`classify_authorship_confidence` and `cairn_claim_authority` are documented as owing each other
lockstep, and `crates/cairn-node/tests/authority_lockstep.rs` pins agreement on **three** shapes only.
Two others disagree:

- **Rust `Attested` / SQL `'unverified'`** — R1 demands the attester key resolve to exactly **one**
  actor and be human (`db/005:715-717`), while db/020's gate demands only
  `EXISTS (… AND kind = 'human')` (`db/020:251-254`) and `actor_current.signing_key_id` carries no
  unique index. Rust checks neither uniqueness nor kind. (The *registry* state takes an owner-level
  write to create — enrolment fails closed via `cairn_key_actor_id_conflict`, `db/004_actors.sql:121`,
  and `actor_event` is REVOKEd from `cairn_agent`, `db/004:206` — but given that state, both **event**
  doors admit the event and the two graders disagree.)
- **SQL `'attested'` / Rust `Device`** — a suppressing-mode event whose only contributor is `recorded`
  passes db/020's attestation gate on `v_mode = 'suppressing'` alone, `cairn_responsibility_bound` is
  vacuously true with no responsibility claim to check (`db/005:477-483`), so `attester_key` is stored
  and R1 grades `'attested'`; Rust sees no bearing contributor and returns `Device`.

Nothing is unsafe today because **no read path consumes the Rust grade**. Which side is right belongs
to [#245](https://github.com/cairn-ehr/cairn-ehr/issues/245)'s display half and is filed as
[#408](https://github.com/cairn-ehr/cairn-ehr/issues/408); it is deliberately not decided here.

**ADR-0063's declared limitations stand unchanged.** Nothing here reaches an already-published byte: a
rung emitted while a chart was `routine` remains readable on every node that already holds the event.

### The finding this hands forward to #376 — an input, not a decision

**A custody dial *derived from* the effective grade is only as strong as its most-custodial holder.**

The grade is **node-relative** (ADR-0062 decision 9): a node with *more* custody legitimately computes
a *lower* grade, because the conservative bound collapses to the true value. So a well-custodied peer,
asked for the DEK, hands it out on a grade this node considers `sequestered` — and no amount of
authority hardening changes that. Authority protects a grade from being **stripped**; it cannot make a
grade that is honestly lower elsewhere be higher there.

An **explicit custody act** — a signed `custody.narrowed`-shaped event rather than a value derived from
the sensitivity stream — has no such property, because it is not derived from anything. That is #376's
open *"which subject feeds which dial"* question with a new argument attached, and it is the most
useful thing this pass hands to part C. **It is not decided here.** The other two #376 questions are
untouched: nothing here bears on the decision-9 bound interaction, and *nothing can un-know a DEK
already fetched* remains part C's declared, forward-looking limitation.

## Consequences

**Easier.**

- The #380 attack now fails at every step, and part C can key on the grade: an enrolled actor on a peer
  authors an un-attested withdrawal → it is **admitted**, nothing refused, nothing forked →
  `cairn_claim_authority` returns `unverified` → it drops out of the set difference → the assertion
  still stands → `cairn_effective_sensitivity` reads `sequestered` → part C's serve door withholds the
  DEK and ships the sealed ciphertext anyway (ADR-0052 E1) → the withdrawal appears on the worklist as
  `inert`. Emission inherits the same unstrippable grade through
  `cairn_prospective_sensitivity`.
- Three consumers, and every future dial, gain the control **structurally** — by delegating to
  `cairn_sensitivity_standing`, which they already did.
- ADR-0062 decision 6 (*an advisory actor may not strip its own protective auto-tag*) stops being prose
  and becomes one `kind = 'human'` clause the test suite pins.
- The project now has a stated rule for choosing between its two non-refusing verdict idioms — *flag
  what cannot self-heal; view what can* — instead of two precedents and a coin flip.
- No new event type, no new projection, no [ADR-0057](0057-generic-reprojection-registered-apply-dispatch.md)
  registry entry, no new `db/0NN`: **none of the four registry row-count pins move and
  `SCHEMA_GENERATION` does not rise.** Stated so a reviewer can check rather than assume.

**Harder.**

- `cairn_claim_authority` is `SECURITY DEFINER`, which means it is **not inlined** and is a
  privilege-escalation surface if its grants are ever loosened. The cost is bounded because it is
  evaluated only for rows where a withdrawal actually matches an assertion — zero on almost every
  chart — and `cairn_sensitivity_standing` itself stays `LANGUAGE sql` and still inlines. But the
  REVOKE/GRANT pair and the `SET search_path` are now load-bearing lines that look like boilerplate.
- The seam is a single clause in a hot read path, and a future author "simplifying"
  `cairn_sensitivity_standing` back to a plain set difference reopens #380 in full with no test-visible
  syntax error. The consequence tests (`an_unattested_withdrawal_lands_and_converges_but_does_not_lower`
  and its siblings) are what stand between that and silence.
- The worklist is a **query with no reader**. Until #388 lands, the control's visible half exists only
  in the database.
- Two documented Rust↔SQL divergences are now on the record and owed a resolution (#408) that this
  slice deliberately did not take.

**The bet.** That *accountability without authorization is the right residual* — that naming a human
who must answer for a protection-removing act, and surfacing the ones with no business on the chart,
changes behaviour more reliably than any lock this architecture could actually enforce. This is
paper's own bet, taken deliberately rather than by default: paper cannot stop the wrong person opening
the envelope either, and the record that it was opened is what makes that acceptable. We are also
betting that the two-row worklist stays small enough to read — that `inert` is genuinely transient and
`stranger-attested` genuinely rare.

**How we would know the bet fails.** Three indicators. The first two are cheap queries over surfaces
this slice ships; the third is the one that actually falsifies the bet.

1. **`inert` rows that never clear.** A rising population of withdrawals whose targets never replicate,
   or of peers shipping un-attested withdrawals as a matter of course, means the gate is fighting
   normal traffic rather than catching anomalies — and the remedy is at the authoring peer, not here.
2. **`stranger-attested` firing on routine care.** If locums, night cover and receiving EDs generate
   the bulk of the rows, the chart-presence question is measuring workforce shape rather than
   accountability, and the row should be re-cut or retired rather than made louder (§5.12). The same
   test applies to `safety_overclaim_flag`: rows attributable to the node's **own** daemon rather than
   to a bypassing client mean decision 8's failure has recurred and the detector, not the emitter, is
   wrong.
3. **A grade stripped by an attested human who cannot afterwards be identified, or whose strip nobody
   ever reviewed.** That is the bet itself failing, because the whole claim is that *the record is the
   control*. A record nobody reads is not one — which is why the missing operator surface (#388) is a
   limitation and not a nicety.

**First instance.** `cairn_claim_authority` and its grants in `db/005_submit.sql:651-735`; the one
authority clause in `cairn_sensitivity_standing` and the `sensitivity_withdrawal_worklist` view in
`db/048_sensitivity_stream.sql:328-419` and `:762-960`; `safety_overclaim_flag` /
`cairn_record_safety_overclaim_flag` in `db/049_safety_projection.sql:485-539` with the local-door call
at `db/005_submit.sql:1055-1176`; the lockstep note in `crates/cairn-event/src/contributor.rs:108-143`;
and the suites `crates/cairn-node/tests/claim_authority.rs`, `claim_authority_worklist.rs`,
`safety_overclaim.rs`, `authority_lockstep.rs`, `crates/cairn-sync/src/main.rs`'s
`db048_authority_gate_resolves_in_the_sync_subset`, with SQL mirrors in
`db/tests/048_sensitivity_stream_test.sql` and `db/tests/049_safety_projection_test.sql`. No new
migration file; `SCHEMA_GENERATION` unchanged.
