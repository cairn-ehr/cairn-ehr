# ADR-0063 — The safety projection and the seal as coarsening boundary

- **Status:** Accepted
- **Date:** 2026-08-14
- **Derives from:** [ADR-0006](0006-visibility-scope-replication-and-the-safety-projection.md) (the safety
  projection's original shape: a de-identified class + severity emitted beside a sealed body, coarsened
  down a policy-configured ladder, existence never extinguished) — this ADR decides its concrete shape,
  and discharges [ADR-0059](0059-medication-drug-coding-drugref-moiety-anchor.md) decision 4's standing
  obligation that the class be **carried, never re-derived** ([#294](https://github.com/cairn-ehr/cairn-ehr/issues/294))
- **Applies:** principle 3 (paper-parity — the front-sheet sticker beside the sealed envelope) · principle 4
  (acknowledged uncertainty, [ADR-0003](0003-bitemporal-time-and-acknowledged-uncertainty.md)) ·
  principle 9 (policy-neutral infrastructure, [ADR-0005](0005-erasure-key-custody-and-crypto-shredding.md);
  also its corollary *deletion is best-effort and declared*) · principle 11 (additive-only evolution,
  [ADR-0012](0012-schema-evolution-event-format-and-legibility-across-time.md)) · principle 12 (the floor
  is in the database, [ADR-0021](0021-layering-the-node-api-and-ui-pluralism.md)) ·
  [ADR-0052](0052-born-sealed-clinical-bodies.md) (born-sealed bodies; the seal boundary this ADR reuses
  as a disclosure boundary) · [ADR-0056](0056-unknown-event-types-admitted-uninterpreted.md) (custody
  total, interpretation deferred, power earned) · [ADR-0058](0058-grade-gated-teffective-ceiling.md) (the
  trailing-envelope-field precedent, and the constrain-where-minted / read-permissively-where-it-arrives
  door split) · [ADR-0060](0060-partial-validity-a-defect-on-one-line-never-invalidates-another.md) (*the
  system may fail to record an order; it may never cancel one*) ·
  [ADR-0062](0062-the-sensitivity-stream-and-the-inverted-unknown.md) (part A — the grade this ADR
  consumes; its decision 2 inversion, decision 8 control 3, and decision 9's node-relativity are all load-bearing here)
- **Canonical spec home:** [identity §5.9](../identity.md#59-sensitivity-grade-the-safety-projection-and-break-glass-visibility-scope)

## Context

[§5.9](../identity.md#59-sensitivity-grade-the-safety-projection-and-break-glass-visibility-scope) has
carried the safety projection since v0.8, and [ADR-0006](0006-visibility-scope-replication-and-the-safety-projection.md)
fixed its purpose: a sealed pregnancy termination still implies a Rhesus-sensitization the next antenatal
clinician must act on, so a maximally-confidential body still owes the record a signal that **names
nothing**. Confidentiality blurs that signal; it never extinguishes it.

[ADR-0062](0062-the-sensitivity-stream-and-the-inverted-unknown.md) built part A of
[#232](https://github.com/cairn-ehr/cairn-ehr/issues/232) — the graded, append-only sensitivity stream and
its effective-grade projection — and enforced nothing, deliberately. This ADR builds part **B**
([#375](https://github.com/cairn-ehr/cairn-ehr/issues/375)): the safety projection itself, coarsened by
that grade.

| | Piece | State |
|---|---|---|
| **A** | the sensitivity stream: graded assertions + the effective-grade projection | shipped ([ADR-0062](0062-the-sensitivity-stream-and-the-inverted-unknown.md)) |
| **B** | **safety-projection emission — a de-identified class + severity, coarsened by the grade** | **this ADR** |
| **C** | sequester: custody narrowing (re-wrap / withdraw DEKs) | [#376](https://github.com/cairn-ehr/cairn-ehr/issues/376) |
| **D** | break-glass: audited key-*use*, partition-honest | [#377](https://github.com/cairn-ehr/cairn-ehr/issues/377), blocked on C |

**Part B still enforces nothing, and saying so is again part of the decision.** It emits and coarsens a
*signal*; it withholds no content, and content-withholding remains part C's job in the custody plane. What
changes versus part A is that the grade now **does something**: before this slice a graded event's safety
relevance was simply invisible, so confidentiality and safety were never composed at all.

What made this a real decision rather than an implementation is that §5.9 states a ladder without saying
**where on the write path the rungs are chosen** — and every plausible answer is defensible in review while
only one of them is safe. The precise class is the most disclosing thing in the system after the body
itself: for the exact cases §5.9 exists for, *the class **is** the disclosure*. Publishing it in the clear
and coarsening on the way out looks like a clean separation of concerns and is a leak. Sealing it and
publishing nothing looks conservative and starves the one node §5.9 was written for. Both mistakes are one
line of code apart.

## Decision

### 1. The seal boundary is the coarsening boundary

The precise `{class, severity}` claim is computed **pre-seal**, on the node that had a coding authority in
hand, and travels **inside the sealed payload** under the same DEK as the body it describes. A **rung**
chosen from the then-effective sensitivity grade travels **in the clear**, on the signed envelope
(`EventBody.safety`, appended trailing per the [ADR-0058](0058-grade-gated-teffective-ceiling.md)
precedent, so no existing signed byte is re-encoded and no existing content address moves).

Three rungs, mirroring §5.9's ladder (*precise class → "confidential medication, severity X" →
"confidential content, break glass"*):

```json
{"rung":"precise",   "class":"rh-sensitizing", "severity":"high"}
{"rung":"kind",      "severity":"high"}
{"rung":"existence"}
```

The middle rung deliberately carries **no `kind` field**. §5.9 reads it as *"confidential **medication**,
severity X"* — and the word *medication* is already in the clear, because `event_log.event_type` is a
plaintext column that says `clinical.medication.asserted` outright. A `kind` field would restate what the
row already publishes, so the rung carries only what is genuinely additional: the severity. The read model
composes the human sentence from `event_type` + `rung` + `severity`.

The coarsest rung **still emits a row**. `{"rung":"existence"}` is the assertion *"there is a
safety-relevant signal on this event, and you are not cleared to see what"* — §5.9's safety-floor
invariant (*coarseness varies; existence never disappears*) made concrete, and precisely what makes
break-glass a rational act rather than a fishing expedition.

**Two tiers rather than one is the whole decision, and it earns its cost by making every degradation fall
out of the seal boundary instead of needing machinery.** No case below required a mechanism of its own;
each is the seal doing what the seal already does:

| reader | gets | why |
|---|---|---|
| custody, no coding authority | the **precise class**, *available* under the seal — no shipped read surface serves it yet (`cairn_event_safety` / `chart_safety` read the clear tier), so today this is a property of where the bytes live rather than a query anyone runs | [ADR-0059](0059-medication-drug-coding-drugref-moiety-anchor.md) decision 4 / [#294](https://github.com/cairn-ehr/cairn-ehr/issues/294): carried, never re-derived |
| no custody (sequestered — part C) | the **clear rung** | the coarsening actually binds, because the class is under a key this reader does not hold |
| after a rung-3 crypto-shred | the **clear rung**, permanently | `cairn_execute_shred` never touches `event_log` (decision 4) |
| grade withdrawn later | custody-holders recover precision by re-reading the sealed body | the custody-less peer stays coarse, and was never entitled to more |

That third row is §5.9's *the safety projection outlives the body it protects* — and it is true here as a
property of **where the bytes live**, not as a rule anyone has to remember.

### 2. Coarsening binds at emission and is re-applied at read — and both are load-bearing, for different reasons

**At emission** (`crates/cairn-node/src/medication/sealed_submit.rs`, immediately before the seal):

```
precise := body.payload.safety                                (absent ⇒ nothing to do)
rank    := cairn_sensitivity_rank(cairn_prospective_sensitivity(patient, thread))
rung    := cairn_safety_rung_for_rank(rank)                   (pure, monotone)
body.safety := coarsen(precise, rung)                         (pure, in cairn-event)
```

The default map is keyed on *rank*, never on the grade string, so ADR-0062's open vocabulary and its
unknown-ranks-MAX are inherited for free rather than re-spelled:

| grade | rank | rung |
|---|---|---|
| `routine` (or no assertion at all) | 0 | `precise` |
| `sensitive` | 10 | `kind` |
| `restricted` | 20 | `existence` |
| `sequestered` | 30 | `existence` |
| anything unrecognised | MAX | `existence` |

Monotone non-decreasing in rank **by construction**: a higher grade can never disclose more. A future grade
interposed at rank 15 lands on `kind`; one at 25 lands on `existence`; an unrecognised one lands on
`existence` with nobody remembering to add it — the same safe-default-by-omission discipline ADR-0062
decisions 2 and 10 use.

**Emission is the only coarsening that binds a peer's raw-SQL client**, because it is the only one that
decides what goes on the wire *at all*. Everything downstream is a rendering choice on bytes that have
already been published, and principle 12 is explicit that a client talking raw SQL walks straight past
every rendering choice above the floor.

**At read** (`cairn_event_safety`):

```
rung := coarser_of( the emitted rung,
                    cairn_safety_rung_for_rank(rank of cairn_effective_sensitivity(event_id)) )
```

Read coarsening is **not** a redundant belt-and-braces of emission. It is the only answer to a peer that
emitted a **finer** rung than this node's grade licenses — and that peer is usually *honest*, not hostile.
The grade is **node-relative** (ADR-0062 decision 9): a peer with different custody legitimately computes a
different grade, and an older peer predating this slice emits no rung at all. All three shapes — the older
peer, the differently-custodial peer, the hostile peer — deliver the same thing: an event carrying
`{"rung":"precise", …}` onto a chart that is `sequestered` *here*. Refusing it at the apply door would fork
the event set (decision 6), so the node **admits and coarsens locally**.

**Neither coarsening alone is sufficient, and a future reader who "simplifies" away either one reopens a
distinct hole.** Emission cannot control a peer's bytes. Read cannot un-publish a byte already on the wire.
This ADR states both because the two look interchangeable in a diff and are not: deleting the emission
coarsening leaves a system that renders politely and publishes everything, and deleting the read
coarsening leaves a system that trusts every peer's judgement about this node's confidentiality.

### 3. An unrecognised severity ranks MAX; an unrecognised rung is the coarsest

```
cairn_safety_severity_rank:  'none' 0 · 'low' 10 · 'moderate' 20 · 'high' 30 · 'critical' 40 · ELSE MAX
cairn_safety_rung_rank:      'precise' 0 · 'kind' 10 · 'existence' 20 ·                        ELSE MAX
```

Both inherit ADR-0062 decision 2's inversion of `cairn_clock_grade_rank`'s `ELSE 0`
(`db/040_clock_confidence_grade.sql`), and in **both** the `ELSE` is the decision rather than a default:

- For a **severity**, MAX means *assume the worst*. An unrecognised severity from an upgraded peer must not
  sort below `critical` and quietly fall off the bottom of a warning list; a signal we cannot rank is a
  signal we have no grounds to de-prioritise.
- For a **rung**, MAX means *disclose nothing*. An unrecognised rung is a disclosure vocabulary this node
  does not understand, and the only safe reading of a permission you cannot parse is that it grants
  nothing.

The two `ELSE`s therefore push in opposite *display* directions — one shouts louder, one says less — while
being the same rule: **uncertainty may raise the alarm and may withhold the detail; it may never do the
reverse.** Both functions carry the same shouting comment ADR-0062's rank function does, for the same
reason: "tidying" either into consistency with db/040's `ELSE 0` looks like housekeeping, and mutes a
critical-severity warning in one case while opening a leak in the other.

### 4. The signal rides the append-only `event_log` row, not a projection table

```sql
ALTER TABLE event_log ADD COLUMN IF NOT EXISTS safety JSONB;
```

Additive, the same precedent as `clock_grade`, `attestation` and `attester_key`.

**A projection table was the obvious shape and is the wrong one.** §5.9 requires the safety projection to
*outlive the body it protects* — to coarsen but survive a rung-3 crypto-shred. Every other derived
projection is scrubbed by `cairn_execute_shred` (`db/037`), correctly, because a projection of a shredded
body is a copy of a shredded body. A safety-projection table would therefore have to be **explicitly
exempted** from that scrub: a single row on an exclusion list, sitting there looking exactly like an
oversight, for as long as the project exists. That is a standing invitation for a future reviewer to
"fix" the inconsistency in good faith and silently delete the one signal the spec says must survive — and
the deletion would pass every test that does not specifically pin post-shred survival.

On the append-only row the signal survives because **`event_log` is never touched by a shred at all**. The
guarantee becomes structural rather than remembered, which is the same reason ADR-0062 decision 4 puts
sensitivity assertions in the clear: what the machinery must still act on when the body is gone cannot
live under the body's own protection.

Two smaller consequences follow and are worth stating so a reviewer can check rather than assume: the
column needs no apply function, no [ADR-0057](0057-generic-reprojection-registered-apply-dispatch.md)
registry entry and no reprojection, so **none of the four registry row-count pins move**; and adding a
column to `event_log` invalidates positional `ROW` literals in stale developer databases, which is an
environmental trap this project has hit before, not a design cost.

### 5. `safety_class_map` ships empty

```sql
CREATE TABLE IF NOT EXISTS safety_class_map (
    system TEXT NOT NULL, code TEXT NOT NULL,
    class  TEXT NOT NULL, severity TEXT NOT NULL,
    note   TEXT NOT NULL DEFAULT '',
    PRIMARY KEY (system, code)
);
```

`cairn_safety_class_candidate(coding jsonb) → (class, severity)` reads it, is **pure and authorless**, and
is **called only pre-seal by an authoring verb — never by a reader.** That call-site rule is the whole of
[#294](https://github.com/cairn-ehr/cairn-ehr/issues/294), and the test suite pins it as a rule rather than
a habit.

The table ships **empty and stays empty in the migration**, with the SQL mirror asserting exactly that, for
ADR-0062 decision 5's reason one subsystem over: Cairn ships the **lookup mechanism**, never the drug
knowledge. A seeded row would be an un-reviewable clinical-policy choice smuggled in as infrastructure
(principle 9), and *which* interactions matter is a deployment's coding-authority judgement, not the
record system's. The table is also the seam a future drugref slice populates without touching any of this.

Keyed on the **pair** `(system, code)`, never a bare `code` —
[ADR-0059](0059-medication-drug-coding-drugref-moiety-anchor.md) decision 5's argument applies unchanged:
once `drugref-clinical-drug` exists beside `drugref-moiety`, a bare-`code` key collides across
composition-tree levels.

### 6. The floor is LOCAL-door only, and the read model is total

This is the subtlest decision here, and the one most likely to be "corrected" later, so it gets the fullest
argument.

ADR-0062 erratum E2 draws a clean line: a **structural** check judges *the shape of the claim being made* —
which every honest peer's event satisfies regardless of local policy — so it is safe at **both** doors; a
**ceremony** check judges *who authored it and under what local accountability*, which peers legitimately
answer differently, so it must stay **local**. Read naively, the safety field's shape check is plainly
structural, and E2 would put it at both doors.

**That reading is wrong here, and the reason is blast radius, not category.**

A sensitivity assertion **is an event**. Refusing a malformed one drops exactly one assertion, and the
chart it was aimed at is otherwise untouched. The safety signal is a **field on a clinical event**.
Refusing it at the apply door drops **the medication assertion it rides on** off this node's chart
entirely — a defect in a de-identified *advisory* field would destroy *clinical content*. That is
[ADR-0060](0060-partial-validity-a-defect-on-one-line-never-invalidates-another.md)'s *a defect on one
element never invalidates the others*, and its harder corollary: **the system may fail to record an order;
it may never cancel one.**

So the split follows the `clock_grade` precedent (`db/040`,
[ADR-0058](0058-grade-gated-teffective-ceiling.md)) exactly — an envelope-level graded field is constrained
where it is **minted** and read permissively where it **arrives**:

| | Local door (`submit_event`) | Remote door (`apply_remote_event`) |
|---|---|---|
| malformed / self-contradictory `safety` | **refuse** | **admit**, store verbatim |
| what a reader then sees | — | the coarsest safe reading |

`cairn_check_safety_signal` is called from the envelope well-formedness step of `db/005_submit.sql` only.
It admits absence (the overwhelmingly common case); requires a non-empty `rung` string when present;
requires a non-empty `class` at `rung = 'precise'`; **refuses a `class` at any rung that is not
`precise`** — a body claiming `{"rung":"existence","class":"rh-sensitizing"}` publishes the class while
asserting it is concealed, and an *unrecognised* rung is caught by the same arm, which is the safe reading;
and **refuses a `severity` at `existence` or coarser**, keyed on the rung's rank so a coarser rung
interposed later inherits the guard. That second disclosure guard was added in the 2026-08-14 review: the
read model gates severity off at `existence` (section 7), so without it the door MINTED permanently-signed
bytes that every honest reader then declined to surface — the door and the read model disagreeing about the
same rung, with the door on the side that cannot be undone. Neither guard can fail a clinical write:
`coarsen` is total over three fixed shapes and no in-repo builder can construct either refused shape. There is no `CHECK` domain: the vocabulary stays open (principle 11). Being an envelope-level
field like `clock_grade` and `attachments`, it does **not** go through the
[ADR-0048](0048-twin-check-registry-dispatch.md) twin-check registry.

Refusing at the local door is ADR-0062 erratum E5's argument one field over: a peer that sent a
self-contradictory signal has **already published those bytes**, so refusing at apply un-discloses nothing
and only forks the event set (the [#342](https://github.com/cairn-ehr/cairn-ehr/issues/342) trap) — here at
the additional price of clinical content. Stopping nodes from **authoring** the contradiction is the only
thing a door can actually accomplish.

**What makes the leniency safe rather than merely lenient is that the read model is total.**
`cairn_event_safety` never trusts the stored shape:

- an unrecognised or missing `rung` reads as **`existence`** (decision 3's `ELSE` MAX);
- **`class` is surfaced only at `rung = 'precise'`** — a class sitting beside a coarser rung is ignored,
  always, whatever the row holds;
- an unrecognised `severity` ranks MAX;
- the effective rung is the **coarser of** emitted and locally licensed (decision 2).

`event_log.safety` therefore stays an honest **derived view of the signed bytes** — never sanitized on the
way in, which would make the column disagree with `signed_bytes` and quietly break the signature's meaning
— while the sanctioned read surface **cannot be made to surface a class the rung does not license**.
Admitting a contradiction and refusing to *act* on it is
[ADR-0056](0056-unknown-event-types-admitted-uninterpreted.md)'s *custody is total; interpretation is
deferred; power is earned*, applied to a field.

### 7. An uncoded medication — and a coding absent from the map — emit nothing at all

Not `{"rung":"existence"}`. Nothing.

[ADR-0059](0059-medication-drug-coding-drugref-moiety-anchor.md) decision 4 is explicit: *for an uncoded
medication there is no class on any node, drugref present or not — that is the principle-4 "little white
pill" floor being honest, not a degradation.* Emitting an existence marker for every uncoded medication
would **manufacture a signal from an absence of knowledge**, and since most real medication lists carry
uncoded free-text entries, it would paint an existence warning across most of most charts on day one. That
is [§5.12](../identity.md#512-the-notification-economy-salience-responsibility-routing-and-the-acknowledgment-floor)'s
alert-fatigue disease — a signal that fires everywhere distinguishes nothing and is dismissed unread —
reproduced deliberately by a system whose founding principles name it as the enemy.

**A coding whose `(system, code)` is not in the map likewise emits nothing.** An empty map result means
*this deployment's coding authority has no opinion about this substance*, which is an absence of knowledge,
not a graded secret. This is principle 4 read correctly: acknowledged uncertainty means acknowledging
uncertainty where it exists and **declining to invent it where it does not** — ADR-0062 decision 10's
argument in a different dimension.

The contrast that makes the rule legible: a coding **with** a class, on a `sequestered` chart, emits
`{"rung":"existence"}`. There the signal genuinely exists and is being blurred, which is exactly the state
break-glass answers.

### 8. A half-formed class claim emits no signal rather than a malformed one

*This decision was not in the design; the build forced it, and it is the sharpest thing this slice learned.*

The first implementation read the class out of the clear payload with `unwrap_or_default()`. Trace what a
single blank cell in `safety_class_map` then does — the columns are `NOT NULL`, but `NOT NULL` is not
non-blank:

1. `cairn_safety_class_candidate` returns `('', 'high')` for that drug.
2. The verb builder writes it into `payload.safety`; emission builds a `precise` rung carrying an **empty
   class**.
3. `cairn_check_safety_signal` refuses a `precise` rung with a blank class (decision 6) — correctly.
4. `db/005_submit.sql` calls that check with a bare `PERFORM` and **no exception block**, so the `RAISE`
   aborts **the whole submit**.

**One blank map row would have cancelled every medication assert naming that drug**, on every node using
that map, with an error message about a safety signal that no clinician could act on and no clinician
caused.

This is decision 6's argument reappearing one layer up — in the *code* rather than at the *door*. Decision
6 stops an advisory field from cancelling clinical content by refusing to check it at the apply door; that
is worth nothing if the authoring path can construct a body the **local** door will refuse. So
`usable_precise_claim` exists: it reads the claim back out of the payload and yields the pair **only when
both halves are non-blank**, otherwise `None` — no clear signal, and the clinical event lands unharmed. The
sealed tier keeps whatever the builder wrote, because the sealed side has no floor to trip and a
custody-holder reading a blank class learns exactly what this node knows.

It reads the claim back out of the payload rather than threading a typed value down, so the guard is
**total over any builder that writes a precise claim** — including one not yet invented.

The general rule, stated so it survives this file: **an advisory field must never be able to fail a
clinical write** — not at a door, and not by constructing a body the door will refuse.

## Rejected alternatives

**The precise class in the clear, coarsened only at read.** The clean-looking separation of concerns:
publish everything, render responsibly. It is **the leak this whole mechanism exists to prevent**. For the
exact cases §5.9 is written for, the class *is* the disclosure — *"Rh-sensitizing event"* in the clear
reads as *"this patient had a termination or a miscarriage"*, and an antiretroviral interaction class reads
as *"this patient has HIV"*. Those bytes then replicate unconditionally (dial 1) to every peer, where any
raw-SQL client reads them straight off the row. It is the projection-layer theatre ADR-0062 refused to ship
one subsystem over: a system that *looks* protective, that a clinician would trust, and that the database
floor does not enforce. And unlike an over-coarsening, it is **unrecoverable** — bytes already sent cannot
be recalled.

**Sealed only, with nothing in the clear.** The conservative-looking answer, and it starves the exact node
§5.9 was written for. A node holding no custody — the sequestered case, the shredded case, the
partition case — would get **no signal at all**, which is the pre-part-B status quo with extra machinery.
It also contradicts the safety-floor invariant directly: coarseness would no longer vary, existence would
simply disappear.

**A dedicated safety-projection table.** Discussed under decision 4. It needs a permanent, unexplained
exemption from `cairn_execute_shred`'s scrub in order to satisfy §5.9's coarsen-but-survive — an
inconsistency that looks like a bug, that a future reviewer will eventually tidy away in good faith, and
whose tidying deletes the Rh-after-termination signal that a future antenatal clinician needs. Rejected in
favour of a guarantee nobody has to remember.

**Refusing a malformed signal at the apply door.** The consistent-with-ADR-0062-E2 answer, and it fails on
blast radius: the safety signal is a field on a clinical event, so refusing it at apply drops the
**medication assertion** — an advisory, de-identified field cancelling clinical content, which ADR-0060
forbids in as many words. It also forks the event set between honest peers running different versions
(the [#342](https://github.com/cairn-ehr/cairn-ehr/issues/342) trap, hit four times in this project
already), and it un-discloses nothing, because the peer that sent the malformed bytes has already published
them. Rejected in favour of admit-verbatim plus a total read model (decision 6).

## Known limitations

**A grade raised after authoring cannot claw back an already-replicated rung.** A `precise` rung emitted
while a chart was `routine` remains readable on every node that already holds the event, even after the
chart is graded `sequestered`. Read-time coarsening blurs it for every honest consumer, and that is all any
mechanism can do. This is [ADR-0005](0005-erasure-key-custody-and-crypto-shredding.md)'s *deletion is
best-effort and declared, never guaranteed* — principle 9's corollary — and it is the same hazard ADR-0062
already declared for a withdrawal's clear-text rationale. **It is why emission-time coarsening is the
control rather than an optimisation:** the moment of authoring is the only moment at which a decision about
what to publish can actually bind.

> [!NOTE]
> **Erratum E1 (2026-08-16) — factual; the decision is unchanged.** *"…blurs it for every **honest**
> consumer, and that is all any mechanism can do"* was **wrong about the local node**, and
> [#405](https://github.com/cairn-ehr/cairn-ehr/issues/405) part 1 was right to say so: Postgres has
> column-level privileges, and the sentence conceded as inevitable something a grant could bind. It was
> also, as written, a guarantee a reader could inherit — decision 2's *"emission is the only coarsening
> that binds a peer's raw-SQL client"* reads the same way about a case it does not cover.
>
> What was actually true when this ADR was written: `db/005` does `GRANT SELECT ON event_log … TO
> cairn_agent`, **a table-level grant covers every column added later**, so the runtime role could
> `SELECT safety FROM event_log` and read the emitted rung and class raw — skipping the read model
> entirely, on the very node whose higher grade was supposed to coarsen them.
>
> **Closed 2026-08-16** (`db/049` section 8): the table-level SELECT grant to `cairn_agent` is replaced
> by an explicit column list that omits `safety`, and `cairn_event_safety` / `cairn_patient_safety`
> became `SECURITY DEFINER` so the sanctioned read still works. A column-level `REVOKE SELECT (safety)`
> alone is **inert** while the table grant stands — the two privilege levels are tracked separately —
> which is why the fix drops to column grants entirely and accepts their fail-closed consequence: a
> column a future migration adds is unreadable by the runtime role until someone grants it deliberately.
>
> **What remains true, and is the limitation this paragraph should have stated:** an already-replicated
> rung on ANOTHER node is still beyond recall (that is the paragraph's real subject, and it is
> unaffected), and an **owner-privileged** connection on this node reads every column, as it must to run
> migrations at all. The floor binds `cairn_agent` — the role the runtime connects as and the one the
> C1–C5 threat model treats as hostile-capable — which is the same boundary principle 12 draws
> everywhere else, not a special concession here.

The partial mitigation is worth naming, but it is **not built and therefore not free**
([#407](https://github.com/cairn-ehr/cairn-ehr/issues/407), 2026-08-14 review): a node **with custody**
*could* recover precision from the sealed payload, which would bound the *loss* from coarsening at emission
to nodes holding neither custody nor a coding authority — exactly the nodes not entitled to the class. No
shipped read surface does this. Outside its own test, nothing in the workspace reads `payload.safety` at
all, so on a custody-holding node with a graded chart the clinician is told to break glass for a value that
node can decrypt one call away. Stated here rather than left as an implied capability — a field written by
every coded verb and read by nothing is the same defect as the uncalled twin renderer this slice deleted,
one level down.

**The read-then-sign race.** The prospective grade is read in one statement and the event is signed and
submitted in another, so a grade raised in that window yields a rung one step too fine. The window
**cannot be closed by moving the decision into `submit_event`**: the rung must be inside the **signed**
bytes, and signing happens in the daemon, where the DEK lives. The window is milliseconds; the error's
consequence is bounded by read-time re-coarsening on every node that later holds the grade; and the remedy
is not re-authoring the clinical event but **re-asserting the grade**. Declared rather than defended
against.

**Rung-4 oblivion does not exist, so the signal is permanent today.** §5.9 says the safety projection is
shreddable **only at rung 4** (best-effort oblivion). Rung 4 is not built, so today there is no path that
removes a safety signal at all. Stated rather than silently assumed, and filed as a follow-on.

**`cairn_prospective_sensitivity` duplicates `db/048` section 11's arms minus the precisely-targeted event
arm.** `cairn_effective_sensitivity` takes an `event_id`, and at emission the event does not exist yet, so
the prospective form takes `(patient_id, thread_id)`. The duplication is the one real drift risk in this
slice, mitigated three ways: the two functions carry cross-referencing comments, both delegate to the
single `cairn_sensitivity_standing` definition of *what still applies*, and a test pins
`prospective(patient, NULL) == effective(event)` for a **thread-less** event with no event-scoped assertion
standing.

That third mitigation is **weaker than it first reads, and the drift it was meant to catch had already
happened** (2026-08-14 review). The test uses a `note.added`, which db/048 section 10b classifies as
thread-free, and passes `NULL` for the thread — so the thread arms are not compared against
`cairn_effective_sensitivity` at all; agreement for a thread-**bearing** event remains unpinned
([#399](https://github.com/cairn-ehr/cairn-ehr/issues/399)).

**And the thread arm had diverged — [#404](https://github.com/cairn-ehr/cairn-ehr/issues/404), fixed in the
review wave.** db/048's catch-all fires only when the named thread is demonstrably on *another* chart, while
the prospective form fired whenever the assertion did not name *this* thread. That catch-all and the
matching arm above it were therefore **exhaustive** over thread-scoped assertions, which made `p_thread`
**inert**: a thread-scoped grade coarsened unconditionally — precisely the chart-wide behaviour section 10b
exists to prevent — and emission disagreed with read on the same node, publishing `existence` for an event
`cairn_effective_sensitivity` calls `routine`. The catch-all now asks db/048's positive question. Two
consequences worth recording: the correction moves in the **disclosing** direction (a medication on an
ungraded thread now emits its class, which is what this system's own canonical rule says it is entitled to),
and it is what made the thread plumbing **testable at all** — with the arms exhaustive, hardcoding
`apply_safety_rung`'s thread lookup to `None` broke no test.

**Note explicitly that the prospective form *keeps* the catch-all's dangling-event clause.** "Minus the
event arm" is easy to read as "minus everything event-shaped", and that reading is a bug. The dropped arm
is the *precisely-targeted* one (an assertion naming **this** event, which cannot exist yet). The retained
one fires when an event-scoped assertion names an event **not present on this chart** — a wrong chart, a
dangling id, or, most often and entirely legitimately, an event that has simply **not replicated yet**,
since set-union sync has no ordering. ADR-0062 erratum E1 coarsens the whole chart in that case, so every
*read* of the event about to be authored will say `existence`. If emission did not agree, it would compute
`routine` and publish a precise class that this very node then declines to display — the worst of both
tiers. It is fully computable before the new event exists, so there is no excuse for the divergence.

**`correct_medication_coding` emits no signal, deliberately.** A `CodingClaim::Strike` carries no coding, so
there is nothing to look up — a struck coding is back to *not-yet-coded*, which decision 7 already answers.
A `CodingClaim::Replace` does carry one, but a correction's safety consequences ride the **thread**: *what
does this chart's medication list now imply* is a thread-rollup question, and rolling a signal up across a
thread is a separate design this slice does not open. Attaching a per-event signal here would answer that
question by accident, in the direction that is hardest to undo — a published class cannot be recalled.

**An advisory lookup that ERRORS falls back rather than propagating — the third route decision 8 does not name.**
Decision 8 closes two ways an advisory field could fail a clinical write: at a door, and by constructing a
body the door will refuse. There is a third, and the first implementation had it — both lookups propagated
their error with `?`, so a missing grant (`db/049` REVOKEs both functions from PUBLIC), a statement timeout,
or any transient failure aborted the *medication assertion* with an error naming a safety class no clinician
caused. Both now fall back in the **withholding** direction and continue: a failed class lookup yields *no
class* (hence no signal at all, decision 7's shape), and a failed grade lookup yields **`existence`**, the
coarsest rung. Neither can guess in the disclosing direction, and the sealed tier is unaffected either way.
Stated here so a future reader knows the error route was considered rather than overlooked, and because
decision 8's rule is stated categorically: *an advisory field must never be able to fail a clinical write.*

**The safety claim is NOT rendered in the §3.13 legibility twin.** A medication event's plaintext twin names
the drug and its coding but says nothing about the class or severity, so a reader holding only the twin — the
principle-11 case, a schema-less future reader — sees the medication without its safety claim. The wire crate
briefly carried a `render_safety_twin` helper that nothing called; it was deleted rather than left standing,
because an uncalled renderer reads as an obligation discharged when it is not. Wiring the claim into the twin
changes the rendered twin of every coded medication, which is a behaviour change on its own merits; it is
tracked with [#379](https://github.com/cairn-ehr/cairn-ehr/issues/379), which already owes the twin the
sensitivity grade.

**Two call sites reach `seal_and_sign` directly** rather than going through `seal_sign_submit`
(`reconciliation::submit_reconcile_like`'s attested arm, and `attestation::attest_thread_in_tx`), so the
*one seam* guarantee — the coarsening lives in `seal_sign_submit` so no future clinical verb can forget it — is
**convention rather than structure on those paths**. This is latent only: none of those bodies carries a
drug claim today, so none writes `payload.safety` and none has a class to leak. A future two-thread verb
that *does* carry a coding would seal a precise claim and publish nothing beside it. The hazard is recorded
in a comment at `seal_and_sign` itself as well as here.

## Consequences

**Easier.**
- Confidentiality and safety are composed for the first time: a graded event's safety relevance is now
  visible without disclosing what it is.
- Every degradation — no coding authority, no custody, post-shred, post-withdrawal — falls out of the seal
  boundary rather than needing its own mechanism (decision 1's table).
- Coarsen-but-survive after a crypto-shred is structural, not remembered (decision 4).
- The ladder extends without editing it: a future grade value lands on a rung by rank, and an unrecognised
  one lands on `existence` with nobody remembering to add it.
- [#294](https://github.com/cairn-ehr/cairn-ehr/issues/294) is discharged with a test the medication-coding
  slice owed and could not write: a node whose `safety_class_map` is **emptied after authoring** still
  reports the precise class from the clear rung it holds — proving the class was carried, never re-derived.
  The test reads the CLEAR tier; the sealed tier's own custody read has no shipped surface yet (decision 1,
  and [#407](https://github.com/cairn-ehr/cairn-ehr/issues/407)). Both emission verbs are covered —
  `assert_medication` and, since the 2026-08-14 review, the `code_medication` overlay that #294 was actually
  written about.

**Harder.**
- There are now **two** places a rung is decided (emission and read), and they must not be collapsed. The
  ADR is the only home of why; a diff that deletes either looks like a cleanup.
- The clear rung is **published forever** at whatever coarseness the authoring node chose. A grade raised
  later cannot claw it back, and no future mechanism can change that.
- `cairn_prospective_sensitivity` is a hand-maintained mirror of `cairn_effective_sensitivity`'s arms.
  Every future change to ADR-0062's read model must be made twice, and the anti-drift test is what stands
  between that and a silent under-coarsening.
- ADR-0062 decision 8's chart-wide blast radius is now **visible and costly** rather than theoretical: a
  chart-wide `sequestered` grade blurs the metformin interaction and the penicillin allergy along with the
  reason the grade exists. The counter-pressure toward blanket-grading now has a measurable price.

**The bet.** That an *always-emitted, always-coarsenable, de-identified* signal beats both a precise one
and a silent one. We are betting that a clinician told *"⚠ high — confidential medication, break glass to
view"* acts correctly more often than one told nothing, and that the residual inference from the signal's
mere existence — *"this patient has something sealed"* — costs less than the missed Rh-sensitization it
prevents. ADR-0006 already took that bet architecturally; this ADR is the first time it is falsifiable in
running code.

**How we would know the bet fails.** Two indicators, in order of how early they appear. First, **existence
rungs crowding out actionable ones** — a rising share of signals at `existence` means the ladder is
blurring more than it discloses, and (per ADR-0062 decision 8) the likely cause is charts drifting toward
blanket chart-wide grades rather than anything wrong here; it is a one-line query over `event_log.safety`
joined to the grade. Second, once a UI surfaces the signal, **the dismissal rate**: a warning dismissed
without a break-glass and without a documented decision is §5.12's alert fatigue arriving, and the answer
is a paper-parity investigation of *what the clinician needed and did not get* — never a louder warning,
which is the move that caused the disease everywhere else. If `safety_class_map` populates and the
existence share does **not** fall, the two-tier shape is not buying what it costs.

**First instance.** `db/049_safety_projection.sql` (the two ladders, the rank→rung map, the local-door
floor check, the empty `safety_class_map`, `cairn_prospective_sensitivity`, and the
`cairn_event_safety` / `cairn_patient_safety` read model), `crates/cairn-event/src/safety.rs` (the pure
`coarsen` and the wire shape) with the trailing `EventBody.safety` field, the floor call and the `safety`
column in `db/005_submit.sql`, the column-only change in `db/020_apply_remote_event.sql` (no floor call —
decision 6), the coarsening seam in `crates/cairn-node/src/medication/sealed_submit.rs`,
`crates/cairn-node/src/safety.rs` (`usable_precise_claim`, `chart_safety`, `render_safety_line`) with the
`patient-safety` verb, and `db/tests/049_safety_projection_test.sql` mirroring the Rust suites.
`SCHEMA_GENERATION` 48 → 49; db/049 is loaded by **both** the `cairn-node` and `cairn-sync` schema lists —
and the subset test **drives** it rather than merely loading it, which is
[#386](https://github.com/cairn-ehr/cairn-ehr/issues/386)'s lesson from db/048 applied on the first try.
