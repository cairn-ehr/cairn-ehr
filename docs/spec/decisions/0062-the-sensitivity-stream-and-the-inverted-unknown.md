# ADR-0062 — The sensitivity stream and the inverted unknown

- **Status:** Accepted
- **Date:** 2026-08-10
- **Derives from:** [ADR-0006](0006-visibility-scope-replication-and-the-safety-projection.md) decision 3
  (sensitivity is a graded, multi-source, append-only stream; the effective grade is a projection;
  declassification is an authorized overlay, never an erasure) — this ADR decides the six things ADR-0006
  left open, plus four more the implementation forced
- **Applies:** principle 2 (never merge, always link; never erase, always overlay) · principle 4
  (acknowledged uncertainty, [ADR-0003](0003-bitemporal-time-and-acknowledged-uncertainty.md)) ·
  principle 9 (policy-neutral infrastructure, [ADR-0005](0005-erasure-key-custody-and-crypto-shredding.md)) ·
  principle 11 (additive-only evolution, [ADR-0012](0012-schema-evolution-event-format-and-legibility-across-time.md)) ·
  [ADR-0043](0043-suppression-self-only-disagreement-is-additive.md) (agent advisories are dismissable) ·
  [ADR-0045](0045-collation-independent-projection-tiebreaks.md) (collation-free tiebreak) ·
  [ADR-0048](0048-twin-check-registry-dispatch.md) (twin/floor-check registry) ·
  [ADR-0052](0052-born-sealed-clinical-bodies.md) (born-sealed bodies; §2's plaintext list; §9's
  custody-relative commitment) · [ADR-0053](0053-per-write-human-authorship.md) (per-write human
  authorship) · [ADR-0056](0056-unknown-event-types-admitted-uninterpreted.md) (the floor gates *effect*,
  not presence) · [ADR-0057](0057-generic-reprojection-registered-apply-dispatch.md) (registered apply
  dispatch) · [ADR-0060](0060-partial-validity-a-defect-on-one-line-never-invalidates-another.md) (a
  local-authoring rule is never a wire rule) · [ADR-0061](0061-registration-is-an-act-that-carries-its-search.md)
  decision 4 (the authorship gate that was *refused*, and why this one is not that one)
- **Canonical spec home:** [identity §5.9](../identity.md#59-sensitivity-grade-the-safety-projection-and-break-glass-visibility-scope)

## Context

[§5.9](../identity.md#59-sensitivity-grade-the-safety-projection-and-break-glass-visibility-scope) has
been settled architecture since v0.8 and unbuilt ever since. ADR-0006 decision 3 fixes its *shape* —
graded, multi-source, append-only assertions; the effective grade is a projection; the highest standing
assertion wins; declassification is an authorized overlay, never an erasure — and deliberately stops
there, because *what* is confidential is cultural, regional and personal, and Cairn ships the mechanism
rather than the policy.

Building it (issue [#232](https://github.com/cairn-ehr/cairn-ehr/issues/232)) made two things clear.

**#232 is four subsystems, not one**, and only the first is decided here:

| | Piece | State |
|---|---|---|
| **A** | the sensitivity stream: graded append-only assertions + the effective-grade projection | **this ADR** |
| **B** | safety-projection emission (de-identified class + severity, coarsened by the grade) | filed; carries [#294](https://github.com/cairn-ehr/cairn-ehr/issues/294) |
| **C** | sequester: custody narrowing (re-wrap / withdraw DEKs) | filed; **blocked on [#231](https://github.com/cairn-ehr/cairn-ehr/issues/231)** |
| **D** | break-glass: audited key-*use*, partition-honest | filed; blocked on C |

**Part A enforces nothing, and saying so is part of the decision.** It computes and reports a grade;
nothing withholds content on the strength of one. That is not an unfinished edge — it is the only honest
place to stop. A projection-layer filter with no custody narrowing beneath it is theatre one layer up
(principle 12: the floor is in the database, and a client talking raw SQL walks straight past anything
above it). And C genuinely cannot be built yet: `cairn-sync serve` verifies an unwrap-key certificate
against its own signature and self-consistency only, never against the admitted-peer trust set
([#231](https://github.com/cairn-ehr/cairn-ehr/issues/231)), so transport is today the sole gate on
read-custody. Narrowing a body's custody to two named clinicians while that hole stands is defeated by
asking the serve port for the DEK.

The second thing: **ADR-0006's shape is under-determined in ways that each have a safe answer and an
unsafe one that looks identical in review.** Those are what follow.

## Decision

### 1. Three subject granularities; the effective grade is the max over all three

An assertion names exactly one subject: an **`event`**, a **`thread`** (today a `medication_id`), or a
**`patient`** (the whole chart). The effective grade of an event is

```
effective(E) = max-by-rank over standing assertions on { E, E's thread, E's patient }
standing     = asserted AND NOT withdrawn
```

with ties broken on `content_address` — a `BYTEA` multihash, collation-free per
[ADR-0045](0045-collation-independent-projection-tiebreaks.md)/[#115](https://github.com/cairn-ehr/cairn-ehr/issues/115).
The tiebreak decides only *which assertion is named as the reason*; the grade itself is order-free.

**Max is the whole reason this converges.** It is commutative, associative and idempotent — a
join-semilattice, i.e. a grow-only CRDT — so set-union sync converges on the grade with **HLC ordering
mattering not at all**. Two nodes that receive the same assertions in opposite orders compute the same
answer, with no last-writer-wins rule to get wrong. (The convergence test names *given equal custody* in
its title, and decision 9 is why.)

**Inheritance is computed at READ, never backfilled at write.** Grading a thread therefore covers events
authored *before* the grading and events authored *after* it, with no migration, no re-signing, and
nobody having to remember. That is what makes the mechanism usable by a clinician who realises three
weeks late that a thread should have been confidential from the start.

**And uncertainty can only ever protect.** Every unknown ranks MAX (decision 2) and combines by max, so
doubt anywhere in the chain raises the grade. There is no path through this computation where confusion
*lowers* protection.

### 2. An unrecognised grade ranks MAX — deliberately inverting the `clock_grade` precedent

```
'routine' → 0 · 'sensitive' → 10 · 'restricted' → 20 · 'sequestered' → 30 · ELSE → 2147483647
```

Open `TEXT`, no `CHECK` domain: a future grade from an upgraded peer is admitted verbatim (principle 11,
additive-only). Gaps of 10 leave room to interpose deployment terms later without renumbering.

The `ELSE` is the decision. It **inverts** `cairn_clock_grade_rank`'s `ELSE 0`
(`db/040_clock_confidence_grade.sql`), and the inversion is load-bearing rather than an inconsistency:

- In db/040, an unrecognised value ranking 0 **withholds reject power**. The worst case is that a
  peer's newer clock grade fails to *reject* something. Safe.
- Here, an unrecognised value ranking 0 would **withhold protection**. An older node reading a peer's
  newer `protected-witness` grade as "not sensitive" emits an uncoarsened safety projection and renders
  the body in the clear — a leak on *exactly* the events that most needed protecting.

The dangerous property is that the wrong answer *looks right*: it matches the established pattern one
migration over, so a future reviewer "fixing" the inconsistency would reopen the leak while believing
they were tidying up. The failure mode is chosen to be **over-coarsening** (honest degradation, repaired
by upgrading the node) and never **disclosure** (unrecoverable). This is ADR-0006's own *when unsure, err
toward essential*, in the confidentiality dimension. The inversion is stated in a shouting comment at the
function for that reason.

**Absence is not unknown, and the distinction survives into the code.** No assertion at all contributes
nothing and reads as `routine`; only an unparseable or unrecognised **grade value** ranks MAX. Collapsing
the two would make every event in every record maximally sensitive. This is principle 4's *not-yet-asked*
versus *unknown* — the same distinction the fourth founding principle exists to keep, applied to a rank
function.

### 3. Declassification is withdraw-by-reference, evaluated as a set difference at read

A `sensitivity.grade-withdrawal.asserted` names the specific assertion's `content_address`. That
assertion **stays in the log**, readable and re-assertable. Nothing is erased (ADR-0006 decision 3), and
the chart history shows that the grade was lowered, by whom, and why.

**Standing is a set difference evaluated at READ, never a row deletion at apply**, and that is not a
stylistic choice. Set-union sync has no ordering, so a withdrawal can arrive *before* the assertion it
withdraws — and it must still take effect when the assertion lands. A delete-at-apply implementation
would silently drop such a withdrawal on the floor and leave a grade standing that a human had
accountably removed. So: no foreign key from withdrawal to assertion, no delete, and one function
(`cairn_sensitivity_standing`) as the single definition of what "still applies" means. Same
arrival-order independence as [ADR-0059](0059-medication-drug-coding-drugref-moiety-anchor.md)'s *a
strike NULLs the anchor rather than deleting the row*.

The corollary of no-FK is that a withdrawal is only as well-targeted as its author. Standing therefore
pins the withdrawal's own **`patient_id`** as well as the target address: a content address being
globally unique makes a withdrawal unambiguous about *which* assertion it names, but does **not** make a
cross-chart withdrawal impossible, and the remote door is deliberately lenient (decision 7). Without that
pin, a withdrawal authored on chart B naming chart A's assertion strips chart A's protection — the
unrecoverable direction.

### 4. Sensitivity assertions are plaintext by necessity — extending ADR-0052 §2

[ADR-0052](0052-born-sealed-clinical-bodies.md) §2 enumerates what stays unsealed because the machinery
binds on it. Sensitivity assertions join that list, and the reason has the same shape as the shred
tombstone's:

> **A node must READ the grade in order to coarsen, and coarsening is exactly what a node holding no
> custody of the graded body must still do.**

Sealing the grade under the key it governs is circular — the node that most needs to know *"treat this
carefully"* is precisely the one that cannot open it. So both event types are born unsealed, replicate
unconditionally (dial 1, §5.9), and are readable everywhere the chart is.

This is not a weakening of ADR-0052; it is the same rule ADR-0052 already states. Sealing is for
**content**; what the machinery must act on travels in the clear, and is therefore held to a strict
discipline about what it may contain — decision 5.

### 5. The matched blacklist category never travels on the wire

ADR-0006 decision 4 warns that a plaintext scope key (`department = sexual-health`) can be the whole
disclosure. A plaintext, unconditionally-replicated body carrying `category: "termination-of-pregnancy"`
**is** the disclosure this mechanism exists to prevent — worse than the scope key, because it is attached
to the exact event it describes.

So an assertion carries **subject, grade and provenance (`human` | `advisory`) — never the matched
category.** Where a tag came from is node-local audit at most. The blacklist lookup
(`cairn_sensitivity_candidate`, decision 8) returns `(grade, category)` to its *caller*, and the caller
puts only the grade on the wire.

The table it reads, `sensitivity_category_map`, **ships empty** and stays empty in the migration, with
the SQL mirror asserting exactly that. Cairn ships the lookup mechanism, never the list: what is
sensitive is cultural, regional and personal, and a seeded row would be an un-reviewable policy choice
smuggled in as infrastructure (principle 9).

### 6. ADR-0043's "agent advisories are dismissable by anyone" does not reach a protective auto-tag

[ADR-0043](0043-suppression-self-only-disagreement-is-additive.md) makes agent advisories dismissable by
anyone — correctly, because an advisory that only its author can clear becomes permanent noise. An
auto-applied protective tag is authored by an advisory actor, so read literally, ADR-0043 would let any
user silently strip it.

**Dismissing a protective tag is a lowering**, and every lowering routes through decision 7's ceremony.

Stated as its own decision because the alternative is not a gap but a **quiet contradiction between two
accepted ADRs, resolving in the unsafe direction**: a reader implementing dismissal from ADR-0043 alone
would build a one-click strip of a confidentiality grade and have an ADR to cite for it. Both event types
are classified `('additive', targets_other_author = FALSE)` for a related reason — a withdrawal is
cross-author *by design* (ADR-0006 requires declassification by *authority*), so it must not be routed
through the ADR-0043 self-only suppression owner-gate, and the ceremony is its substitute control.

### 7. Raising is frictionless; lowering is a ceremony — at the LOCAL authoring door only

| Act | Local door (`submit_event`, db/005 step 8a) | Remote door (`apply_remote_event`) |
|---|---|---|
| Raise, `event` / `thread` | no ceremony — any accountable contributor | **admit** |
| Raise, `patient` (chart-wide) | **rationale required** | **admit** |
| Withdrawal (lowering) | **bound human author (ADR-0053) + rationale** | **admit** |

The asymmetry is the matcher's *false merge ≫ false split* one axis over: **never block a protective act;
always make a protection-removing act accountable.**

**The ceremony is a local-authoring rule and never a wire rule** ([ADR-0060](0060-partial-validity-a-defect-on-one-line-never-invalidates-another.md);
the [#342](https://github.com/cairn-ehr/cairn-ehr/issues/342) trap), for two independent reasons:

1. Peers run different local policies. A door check at apply would let one peer's honestly
   rationale-less act be refused by another peer's stricter node, **forking the event set and wedging
   replication** on entirely honest traffic — the failure this project has now hit four times
   (ADR-0056, ADR-0058, ADR-0061 decision 3, #268).
2. **For a raise it is strictly worse than a wedge.** Refusing a peer's protective assertion leaves this
   node computing a *lower* grade than the peer already holds — so **the refusal is itself a
   disclosure.** A "stricter" door would be the less safe one.

Both halves are *tested*, not merely commented: the remote door is pinned to admit exactly the three
shapes the local door refuses.

**Why a bound human author here, when [ADR-0061](0061-registration-is-an-act-that-carries-its-search.md)
decision 4 refused one for registration.** ADR-0061 rejected an authorship gate because it blocks *care
documentation*: registration sits upstream of everything, so a refused registration means nothing at all
can be recorded about that patient, and staff route around it by registering cooperative patients as John
Does. None of that applies here. A withdrawal is an **administrative act with a consent basis**, not care
documentation, and refusing it **blocks nothing clinical** — the content stays fully readable to everyone
who already has custody; only the *grade* stays high, which is the safe direction to be stuck in. The
asymmetry between the two ADRs is deliberate and reasoned, not an oversight, and is recorded here so that
a future reader harmonising them does not "fix" one into the other.

### 8. Chart-wide grading is expressible, deliberately effortful, and never automatic

Whole-chart grading is **necessary**, and it cannot be served by grading threads: the staff member
treated at their own hospital, the public figure, the domestic-violence case where the *fact of any care*
is the risk, child protection. The catastrophic failure in every one of those is a **new thread opened by
a clinician who does not know**, and patient-scope is the only subject that covers threads nobody has
imagined yet.

It is also the one act here whose blast radius is the entire record. Once part B lands, a chart-wide
grade coarsens **every** safety signal on that chart — the metformin interaction and the penicillin
allergy blur along with the reason the grade exists. Two consequences follow, and the second is the
serious one:

- **The signal stops carrying information.** If everything on a chart is blurred, blurring distinguishes
  nothing — §5.12's alert-fatigue disease in the confidentiality dimension.
- **Break-glass fatigue.** The clinician learns that on *this* patient they always have to break glass,
  so they break it reflexively on arrival. That is principle 3's named enemy — the confirmation-dialog
  click-through — reappearing as an audited access event, and it is **worse than the dialog**: every
  reflexive break-glass writes a record that looks like a deliberate, justified access, so the one that
  mattered becomes indistinguishable from the three hundred that did not. The audit log degrades from
  evidence to noise, and it does so silently.

Three controls, and deliberately **no cap**:

1. **A chart-wide raise requires a `rationale`** — the single exception to frictionless raising. It is
   what the person who later has to unwind it gets to read.
2. **The automatic path cannot express a chart-wide candidate at all.** `cairn_sensitivity_candidate`
   returns `(grade, category)` and has no subject column to fill in even by accident; the caller pairs
   the grade with the event or thread that carried the coded field. A coded hit on one drug blanket-grading
   an entire chart is precisely *"chart-wide as the default for highly sensitive records"*, which is the
   thing the friction exists to prevent.
3. **The read surface always names which subject won** — `sequestered (chart-wide)` versus
   `sequestered (this thread)`. Without it, nobody can tell *why* a chart is uniformly blurred, and
   therefore nobody can fix it: a chart-wide assertion is one thing to go and look at, while twenty
   individually-graded threads are twenty.

**Capping chart-wide below `sequestered` was considered and rejected** — see the rejected alternatives.

### 9. The effective grade is node-relative, not a global fact

`medication_id` lives **inside the sealed payload**, and `event_log` carries no thread column in the
clear. The medication projections are populated through `cairn_clear_payload`, so on a node holding no
custody the rows are absent, `E → thread` resolution fails, and a thread's grade would not apply —
producing a *lower* grade on precisely the node least entitled to see anything.

The resolution follows decision 2's direction rather than adding machinery:

```
thread contribution =
    resolved T                                   → max over standing assertions on T
    unresolved, chart HAS thread-assertions      → max over ALL the chart's thread-assertions
    unresolved, chart has none                   → nothing
```

The middle rule is a **precise conservative bound, not a sentinel**: an unresolvable event belongs to
*some* thread on this chart, so the tightest safe answer is the max over that chart's thread grades. It
needs no artificial MAX value, and the third rule keeps the uncertainty from biting where it cannot
matter — without it, every medication event on every custody-less node would coarsen maximally,
recreating decision 8's everything-is-blurred problem by accident.

**The bound is required by §5.9 today, not only by part C.** When an event is crypto-shredded (rung 3),
db/037 scrubs its derived projection rows, so its thread is unresolvable **on every node, permanently —
including the authoring one**. Without the bound, a shredded event's thread grade evaporates at the
moment of shred and its safety projection renders uncoarsened, contradicting §5.9's *"the safety
projection outlives the body it protects — coarsens but survives."* The bound is what makes
coarsen-but-survive true after a shred, independently of sequester.

**Hence: the effective grade is a function of local custody, not a global fact — and it is non-monotone
in custody.** Gaining custody can *lower* a displayed grade as the bound collapses to the true value.
This is a known pattern here rather than a surprise: [ADR-0052](0052-born-sealed-clinical-bodies.md) §9
found exactly the same thing about [ADR-0049](0049-commitment-based-sign-off-currency.md)'s thread
commitment, which born-sealing turned from a pure function of the content-event *set* into a function of
local *custody*. Two consequences carry forward, and both must be designed for rather than discovered:

- **A UI showing a grade must tolerate it dropping as DEKs arrive.** A grade that falls is not a bug
  report.
- **Any cross-node equality test is valid only *given equal custody*.** Stated loosely, such a test
  either fails spuriously or — far worse — gets "fixed" by deleting the bound, reopening the leak it
  exists to close. The custody qualifier is in the test's own name for that reason.

It opens **no new inference channel**: the bound reveals the chart's highest thread grade to a node that
can resolve nothing, but assertions are plaintext and replicate unconditionally (decision 4), so that
grade was already readable there.

### 10. The conservative bound is scoped to thread-bearing event types

Applied bluntly, "thread unresolvable ⇒ take the bound" is also true of every note, demographic edit,
identity assertion, registration and sensitivity event — none of which can **ever** belong to a
medication thread, resolved or not. The effect was that a single thread-scoped `sequestered` assertion
coarsened the *entire chart*: every note, every demographic field, everything. Thread-scoping silently
behaved like chart-wide scoping, defeating the reason a narrower subject kind exists at all.

So the bound applies to **`clinical.%` and to unrecognised/future event types**, while types this version
*positively knows* are thread-free contribute nothing. Note the direction of the default:
`cairn_event_type_has_no_thread` returns TRUE only for the namespaces we have confirmed
(`demographic.` / `identity.` / `note.` / `patient.` / `sensitivity.` / `erasure.`); **anything
unrecognised keeps the bound**, mirroring decision 2's `ELSE` MAX. A future clinical stream inherits the
bound for free simply by not appearing in that list — the safe default requires nobody to remember to add
it.

**This is principle 4 one level up.** A note having no medication thread is a **fact**, not uncertainty,
and coarsening on a fact we hold is not caution — it is fabricated doubt, and it costs exactly the
precision that made thread-scoping worth building. Acknowledged uncertainty means acknowledging
uncertainty where it exists, and *declining to invent it where it does not*.

## Rejected alternatives

**Unknown ranks 0, matching `cairn_clock_grade_rank` (db/040).** The consistent-looking answer, and the
leaking one. Rank 0 in db/040 withholds *reject power*; rank 0 here withholds *protection*, so an older
node reads a peer's newer grade as "not sensitive" and renders a confidential body in the clear.
Consistency between two rank functions is worth nothing when the two ranks mean opposite things. Rejected
in favour of MAX, with the reasoning shouted in a comment at the function, because this is the one
"cleanup" most likely to be attempted in good faith.

**Capping chart-wide below `sequestered`.** Tempting, given decision 8's blast radius: let a chart-wide
assertion reach `restricted` at most, so no single act can seal an entire record. Rejected, because
whole-chart sequestration is exactly right for a legitimate protected-witness deployment, and foreclosing
it is **Cairn taking a policy stance about which patients deserve which protection** — precisely what
principle 9 forbids. The controls are friction, non-automaticity and visibility (decision 8's three); the
ceiling stays open.

**Self-only withdrawal — the ADR-0043 shape.** Let only the assertion's author withdraw it. It has the
appeal of a clean ownership rule, and it **deadlocks every real case**: the asserting clinician has
retired, the patient who requested the grade has left the practice, the advisory actor that auto-tagged
it has been superseded. A protection nobody alive can lower is not a strong protection — it is a record
that accumulates permanent, un-removable grades until the grading mechanism is worked around entirely.
ADR-0006 decision 3 already requires declassification by **authority**, not by ownership; the ceremony
(decision 7) is what makes authority accountable, and it is why both event types are classified
`targets_other_author = FALSE` rather than being routed through the self-only suppression gate.

**A plaintext thread reference on `event_log`.** This would make thread resolution custody-free and
delete decisions 9 and 10 entirely — a real simplification, which is why it deserves a recorded refusal
rather than silence. It fails on its own terms: *"these eight events form one thread"* **is itself
linkage information**, and thread size and timing re-identify. It would trade a coarsening cost for a
disclosure, on a plane that replicates unconditionally — the exact trade decision 5 refuses one paragraph
over. It is also an envelope wire change, which ADR-0052 §2 scopes deliberately.

## Known limitations

**Thread resolution resolves only a thread's current head**
([#374](https://github.com/cairn-ehr/cairn-ehr/issues/374)). `cairn_event_thread` maps an event to its
thread by looking its `content_address` up across the medication projections — and four of those five
tables (`medication_statement`, `medication_cessation`, `medication_coding`, `medication_dose_correction`)
are one-row-per-key upserts carrying only the *winning* event's address. Only `medication_dose_event` is
keyed per event. So **every superseded medication event resolves to NULL even on a node with full
custody**, and falls into decision 9's bound.

The direction is safe — unresolvable coarsens, never exposes — but the population taking the bound is far
broader than intended: the bound exists to cover custody gaps and shredded bodies, and it is silently
absorbing ordinary superseded events on fully-custodial nodes. The precision cost is real (a chart with
one `sequestered` thread coarsens every superseded event to `sequestered` rather than to its own thread's
grade). Fixing it means resolving from the event's own body rather than from a winner-keyed projection,
which puts a body read on the safety-critical grade path and must behave identically on a custody-less
node — a decision, not a patch. Recorded here rather than left in the code so that a reader of decision 9
does not assume resolution is general.

## Consequences

**Easier.**
- A grade is stated once and applies forever afterwards, in both temporal directions, with no backfill:
  thread inheritance is computed at read.
- Convergence is free and needs no ordering rule — max over a set is a grow-only CRDT.
- The mechanism is genuinely policy-neutral: an empty blacklist, an open grade vocabulary, three
  ADR-0006 workflows that are the same call site with different callers, and no shipped opinion about
  what is confidential.
- Every failure direction in the computation is over-coarsening. There is no path where confusion,
  version skew, a mis-targeted assertion or a missing DEK *lowers* a grade.

**Harder.**
- A grade is now **node-relative** (decision 9). Every future consumer — UI, safety projection,
  sequester — has to be written knowing that the number it reads can fall when custody arrives, and that
  two honest nodes may legitimately disagree.
- A withdrawal's `rationale` is clear text, forever, and it replicates. A rationale naming the condition
  (*"patient consented after her termination follow-up"*) leaks precisely what the grade protects. The UI
  must warn at the point of entry; a sealed-rationale variant is filed as a follow-on, and until it
  exists this is a real, live hazard rather than a theoretical one.
- Chart-wide grading is available to anyone who can write a rationale, and its second-order cost
  (break-glass fatigue) does not show up until part B lands and part D is in routine use — which is late.
  The three controls are what we have; whether they are enough is the bet below.

**The bet.** That an *advisory, honest, always-computable* grade beats an enforcing one built too early.
Part A withholds nothing, and deliberately: the enforcement that matters is custody narrowing in the DB
floor, and shipping a projection-layer filter first would have produced a system that *looks* protective,
that a clinician would trust, and that a raw-SQL client walks straight past. We would rather ship a grade
that is honest about enforcing nothing than a filter that is dishonest about enforcing something.

**How we would know the bet fails.** Charts drifting toward uniform chart-wide grades — a rising share of
`patient`-subject assertions, or a rising share of events whose winning subject is `chart-wide`, is the
leading indicator of decision 8's failure mode, and it is a one-line query over
`sensitivity_assertion`. Once part D lands, the direct measure is break-glass rate per chart: a chart on
which every access is a break-glass is a chart whose audit log has stopped being evidence. If either
climbs, the answer is a paper-parity investigation of how grades are being applied — not a stricter
floor, which is what pushed people to blanket-grade in the first place.

**First instance.** `db/048_sensitivity_stream.sql` (the ladder, both structural floors, the two retained
sets, the standing/thread/effective read model, the ceremony, and the empty category map), the ceremony
call at `db/005_submit.sql` step 8a, `crates/cairn-event/src/sensitivity.rs` (the pure wire builders and
twins), `crates/cairn-node/src/sensitivity.rs` with the `sensitivity-assert` / `sensitivity-withdraw` /
`patient-sensitivity` verbs, and `db/tests/048_sensitivity_stream_test.sql` mirroring the Rust suites.
`SCHEMA_GENERATION` 47 → 48; db/048 is loaded by **both** the `cairn-node` and `cairn-sync` schema lists,
because a node that stores the assertion in `event_log` without the projection computes `routine` and
renders the body in the clear.
