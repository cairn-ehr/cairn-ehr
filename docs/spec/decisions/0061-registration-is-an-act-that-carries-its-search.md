# ADR-0061 — Registration is an act that carries its search

- **Status:** Accepted
- **Date:** 2026-08-05
- **Derives from:** principle 2 (identity is a claim, never a fact,
  [§5.1](../identity.md#51-linkage-layer-never-merge-always-link)); principle 3 (paper-parity,
  [§1.2](../vision.md#12-the-paper-parity-test-normative)); principle 4 (acknowledged uncertainty,
  [ADR-0003](0003-bitemporal-time-and-acknowledged-uncertainty.md))
- **Applies:** [ADR-0014](0014-locale-pluggable-matcher-comparators.md) (the matcher is advisory; the hub
  sweep is the false-split backstop) · [ADR-0022](0022-validated-submit-surface-the-write-path.md) (the
  validated write door) · [ADR-0048](0048-twin-check-registry-dispatch.md) (twin-check registry) ·
  [ADR-0051](0051-contributor-role-vocabulary-floor-and-responsibility-wire-shape.md) (strict-submit /
  lenient-apply) · [ADR-0052](0052-born-sealed-clinical-bodies.md) (sealed ⇒ clinical) ·
  [ADR-0053](0053-per-write-human-authorship.md) (per-write human authorship) ·
  [ADR-0058](0058-grade-gated-teffective-ceiling.md) (a remote door admits and flags, never rejects) ·
  [ADR-0060](0060-partial-validity-a-defect-on-one-line-never-invalidates-another.md) (partial completion
  is reported, never implied)
- **Canonical spec home:** [§5.3](../identity.md#53-registration-classes) /
  [§5.8](../identity.md#58-registration-documentation-workflow-normative)
- **Errata:** **E1**–**E3**, appended 2026-08-06 after the implementation review found three passages
  describing code that was never built. Each is a marked blockquote immediately below the passage it
  corrects, the original wording is preserved above it, and **no decision content changes** — see the
  errata rule in [README](README.md#rules).
- **Implementation notes:** **N1** (2026-08-08) — decision 3's deferred enforcement shipped. Appended
  below decision 3 in the same never-substitute form as the errata; nothing above it is edited, and this
  ADR's index row in [README](README.md) still reads as it did on the day it was accepted.

## Context

[§5.8](../identity.md#58-registration-documentation-workflow-normative) item 1 has been normative since
v0.1: *"'new patient' unreachable until local-scope matching has run; candidates shown with photo/age/
locale/last visit; **the create button records that N near-matches were displayed**."* Building it
(issue [#344](https://github.com/cairn-ehr/cairn-ehr/issues/344)) turned up two facts that make it more
than "add a search box".

**There was no registration path at all.** `cairn-node` had `register-john-doe`, `enroll-human`,
`identify-patient` and the medication verbs. A *standard* chart came into being as a **side effect** of
whatever event happened to carry its `patient_id` first. A side effect has nowhere to record anything, so
§5.8's recording obligation had nothing to attach to — and a recording obligation is event-shaped, i.e.
the can't-retrofit kind: a chart created today without that record can never acquire one honestly.

**There was no patient search of any kind.** The
[§5.2](../identity.md#52-matching-pipeline-safety-asymmetric-false-merge-worse-than-false-split)
matcher does batch pairwise sweeps over the whole population. Nothing answered *"a clerk typed a name —
which charts might this be?"*

The failure mode this whole mechanism serves is not the moment of registration. It is **six months
later**, when a duplicate surfaces and somebody has to work out what went wrong.

## Decision

### 1. Registration is an act — one event type, three classes, no carve-out

`identity.registration.asserted` (schema version 1) carries
[§5.3](../identity.md#53-registration-classes)'s three classes as a `class` discriminant:
`standard` / `unidentified` / `pseudonymous`.

The alternative was a new type for standard charts only, leaving [§5.4](../identity.md#54-unidentified-registration-john-doe-baked-into-the-root)
John Doe on its old path. It was rejected because of what it does to the **precedence rule** — *the first
event carrying a new `patient_id` must be a registration*. With one type the rule reads exactly that, full
stop. With two paths it has to read *"…unless the chart is a John Doe"*, and **an "unless" in a safety
floor is where the next defect lives**: every future reader has to rediscover which exception applies to
their case, and every future class multiplies the carve-outs.

Re-expressing the shipped John Doe path cost tests and no data migration (the project is pre-clinical, and
the wire discipline that matters is never *breaking* the wire, which adding a type does not do —
[ADR-0056](0056-unknown-event-types-admitted-uninterpreted.md) admits unknown types uninterpreted, so a
peer on older code carries this event without harm). It also records a fact §5.3 asserts and no code held:
**which class a chart was registered under.**

### 2. The attestation NAMES the displayed candidates; it does not count them

`search.displayed` carries the candidate `patient_id`s that were actually on the screen, in display order.
There is deliberately **no `displayed_count`** — the count is `length(displayed)`, and two representations
of one number is a lie waiting to happen.

The reason is the six-months-later question, and it is the load-bearing paragraph of this ADR. When a
duplicate surfaces, the investigator has exactly one question: **was the duplicate on the screen when the
clerk clicked create?**

- **Yes** → human judgement failed. Fix the UI, the display fields, the candidate ordering, or the
  training.
- **No** → the search failed. Fix the comparator, the blocking keys, or the recall.

**Those have opposite fixes, and a bare `N = 3` cannot tell them apart.** A count says three charts were
shown; it cannot say whether *this* chart was one of them. Retuning a comparator that was working, or
redesigning a screen that was fine, are both expensive and both wrong — and the count leaves no way to
know which mistake you are making. The identifiers are the only form of the record that answers the
question actually asked of it.

Two limits on how the named set may be read:

- **It is weak, honestly-graded evidence.** The clerk may never have read the list. Displayed-and-not-
  chosen is **not** an `unlink` and must never be projected as a judgement that the charts differ. Whether
  it later feeds the §5.2 matcher's gold set is a separate question, deliberately unanswered here.
- **An empty `displayed` is normal and must stay legal.** A genuinely new patient produces a search that
  correctly found nothing. `[]` ("the search ran, found nothing") and an absent `search` object ("no search
  ran") are distinguished by the object's *presence*, never by its length. Tightening the floor into a
  non-empty requirement would make registering the first patient on a fresh node impossible; a test exists
  so it cannot be tightened away silently.

`search.incomplete` is **required, not optional** —
[ADR-0060](0060-partial-validity-a-defect-on-one-line-never-invalidates-another.md) decision 2 (*partial
completion is reported, never implied*) binds here as much as on the drug chart. If the node could not read
some candidate it found, the attestation says so rather than implying the search was exhaustive.

### 3. Strict local submit, lenient remote apply — and the enforcement is deliberately deferred

The precedence rule, once turned on ([#345](https://github.com/cairn-ehr/cairn-ehr/issues/345)), is
enforced at `submit_event` and **not** at `apply_remote_event`.

Set-union sync has no ordering guarantee, so a peer's clinical event legitimately arrives *before* the
registration event that licenses it. A fail-closed remote door would then wedge replication on **entirely
honest traffic** — the failure mode this project has now hit three times
([ADR-0056](0056-unknown-event-types-admitted-uninterpreted.md),
[ADR-0058](0058-grade-gated-teffective-ceiling.md), and
[#268](https://github.com/cairn-ehr/cairn-ehr/issues/268)).
[ADR-0051](0051-contributor-role-vocabulary-floor-and-responsibility-wire-shape.md)'s strict-submit/
lenient-apply and ADR-0058's *"the remote door admits-and-flags, never rejects"* are the same pattern; this
decision follows it rather than re-deriving it.

The rule is self-satisfying afterwards: once a peer's clinical event has landed, a local write to that
chart is no longer a *first* event, so no local refusal follows from the lenient admission. The residue is
a chart with no registration row until the peer's registration syncs — **queryable** over the projection,
but not surfaced in any UI yet.

> [!IMPORTANT]
> **The rule is settled here; its enforcement ships separately, and this is why.** Measuring the change
> rather than assuming it turned up two facts:
>
> 1. **`patient.created` already exists** — a walking-skeleton event type classified `additive` (`db/005`),
>    driving **two** registered appliers, not one: `patient_chart_apply` → `patient_chart` (`db/005`, run
>    order 10, and the branch at `db/002`) *and* `surrogate_project_apply` → `patient_ref` (`db/008`, run
>    order 20). Retiring the type means retiring both. Its `{name, dob, sex}` payload is superseded by
>    demographics slices 1–5,
>    **no structural floor** and no twin-check row. It is an unfloored registration act, and it must be
>    **retired** by the same change that turns the rule on. Grandfathering it as a permitted first event
>    would put back exactly the "unless" decision 1 exists to remove.
> 2. **The rule converts ~83 submit call sites across ~38 `cairn-node` test files**, plus 37
>    `patient.created` references in `cairn-sync`/`cairn-event`. That is the whole DB-gated suite, and a
>    mechanical rewrite of 38 fixtures deserves its own review where each converted fixture's intent can be
>    checked.
>
> > **Erratum E1 (2026-08-06) — factual; the decision is unchanged.** The *"37 `patient.created`
> > references in `cairn-sync`/`cairn-event`"* in item 2 is a **large overcount**, and the figure should
> > not be planned against. As measured on this branch, those two crates contain **9** textual occurrences
> > of the string (6 in `cairn-sync`, 3 in `cairn-event`), of which only **5** are event-type literals in
> > code — the other 4 are prose in comments. `submit_patient_created` appears in **neither** crate: all
> > 19 of its uses are `cairn-node` tests, already inside the ~83-call-site / ~38-file figure, so any
> > count combining the two was double-counting the same call sites.
> >
> > The more useful correction is what those 5 literals *are*, since it changes the shape of the work and
> > not just its size. `cairn-event` has **no Postgres dependency at all** — its 3 occurrences are
> > CBOR/serialization fixtures in which the event-type string is arbitrary and nothing is ever submitted.
> > In `cairn-sync`, two are bench/seed emitters and one is a test-helper body, and `emit_event` persists
> > through a raw `INSERT INTO event_log` rather than through `submit_event` — which is where decision 3
> > places the precedence rule. So the rule as specified does not reach these crates the way a reader of
> > *"plus 37 references"* would reasonably assume; whether #345 also retires the event-type *name* in
> > fixtures is a separate scoping choice, and one it should make deliberately rather than inherit from a
> > number. Item 2's **conclusion** is untouched: #345 remains a whole-suite rewrite deserving its own
> > review, on the strength of the ~83/~38 `cairn-node` figure, which this erratum does not disturb.
>
> Tracked as [#345](https://github.com/cairn-ehr/cairn-ehr/issues/345).
>
> **So when this slice lands the funnel is complete but NOT yet unbypassable. A client can still mint a
> chart by asserting a name.** Stated plainly rather than implied — the same discipline ADR-0060 decision 2
> applies to clinical output, applied here to our own build state. Nothing in §5.3/§5.8 may be read as a
> guarantee until #345 closes.
>
> > **Implementation note N1 (2026-08-08) — the deferral above is now closed; the decision is unchanged.**
> > #345 shipped as slice 64. The paragraph above stands as written — it was true of slice 63, and it is
> > left standing rather than rewritten so the ADR keeps reading as it did on the day it was accepted. What
> > changed is only the build state it describes: `submit_event` step 8b (`db/005`, over the
> > `cairn_patient_has_events` predicate in `db/001`) now refuses any first event on a chart that is not an
> > `identity.registration.asserted`, and `apply_remote_event` deliberately still does not — exactly the
> > split this decision specifies. `db/047` retires `patient.created` in the same change, so decision 1's
> > "no unless" holds literally. **§5.3/§5.8 may now be read as a guarantee at the strict local door**, and
> > only there. Pinned by `crates/cairn-node/tests/patient_precedence.rs` and
> > `db/tests/047_registration_precedence_test.sql`.

### 4. Rejected alternative: gating a standard registration on a bound human author

This is the section most likely to save a future reader from "fixing" an absence that is deliberate.

An attestation whose whole purpose is the later investigation should surely answer **who was looking**, not
only *what was on screen*. The tempting move is to make `class=standard` refuse without a bound human
author. **That move is wrong.**

**The honest half is already free.** `cairn_authorship_bound` runs *unconditionally on every event* at the
submit door (`db/005`, step 4b): any responsibility-bearing contributor's `actor_id` must be the event's
signer or the verified attester ([ADR-0053](0053-per-write-human-authorship.md)). So a registration that
**names** a human registrar is already unforgeable, with no new rule anywhere. What the door deliberately
does not do is require such a contributor to *exist*: a bearing-less event is **graded** `Device`, not
refused.

**Requiring one would violate [§5.11](../identity.md#511-point-of-care-identity-possession-fast-authentication-and-salvage)
outright:** *"Authorship-confidence is a grade, not a gate… where author identity cannot be cheaply
established (badge forgotten, two in range, emergency), the system never blocks."* Three concrete failures
follow from gating, and each is worse than the problem it purports to solve:

1. **It blocks care documentation, not merely registration.** 03:00 in the ED; the clerk's personal key is
   not unlocked (locum's first shift, dead card reader, enrolment ceremony never run). The registration is
   refused — and because decision 3's precedence rule makes registration the **first event for a new
   `patient_id`**, *nothing at all can be recorded about that patient*. A control on a registration form
   turns into an outage on the clinical record, because registration sits upstream of everything.
2. **Its real effect is to push named, cooperative patients through the John Doe path.** `class=unidentified`
   needs neither a search nor an author. Faced with a prompt they cannot satisfy at 03:00, staff will
   register the perfectly cooperative Mrs Smith as an unidentified patient to get past it — and now the
   chart carries a callsign instead of her name, no search attestation, and an identity-pending trust state
   that has to be resolved later by a human. **A control people route around by degrading the record is not
   a control**; it is a mechanism for manufacturing the exact duplicates the funnel exists to prevent.
3. **It is self-defeating on its own terms: a gate that refuses produces NO forensic record.** For a
   mechanism whose entire justification is the six-months-later investigation, refusing to write is
   strictly worse than writing *"registrar unattributed"* — which is at least true, auditable, honest, and
   present in the record when the investigator arrives. The gate destroys the evidence it was installed to
   collect.

**So: no floor change, and no new authorship rule anywhere.** The registration path takes an *optional*
registrar key. When the registrar signs, the existing binding makes the claim unforgeable and the existing
classifier grades it `Attested`. When they cannot, the event records `Device` — *authored at this node,
registrar unattributed* — never a guess (principle 4's explicit unknown), composing into the §5.7/§5.10
trust projection with **no new stream**.

> **Erratum E2 (2026-08-06) — factual; the decision is unchanged.** *"The registration path takes an
> optional registrar key"* describes a path that **was never built**. `patient-register` has no such flag,
> `register_patient` takes no attester parameter, and there is no `Attested` registration anywhere in the
> shipped slice: **every** registration it authors records `Device`. What this section *decides* — no floor
> change, no new authorship rule, and `Device` rather than a guess when the registrar is unattributed —
> stands exactly as written, and the floor half of it is real (`db/005`'s unconditional
> `cairn_authorship_bound`). Only the claim that the opt-in half already exists is wrong. The attested path
> is future work, tracked as [#359](https://github.com/cairn-ehr/cairn-ehr/issues/359) (`identify-patient`
> already carries the attester plumbing to mirror); nothing reading this ADR may assume it until #359
> closes.

**Wanting attested registrations is policy, not mechanism** (principle 9). A deployment that requires it
expresses it as [ADR-0024](0024-hard-policy-expression-the-policy-assertion-stream.md) hard policy or a
role gate; the CLI and the later UI may nudge as soft policy
([ADR-0021](0021-layering-the-node-api-and-ui-pluralism.md)). Cairn ships the grade. The quality signal
survives regardless: *"standard registrations graded `Device`"* is a one-line query and belongs on the same
hub worklist as ADR-0014's duplicate sweep.

### 5. The registration body is not born-sealed, and that has an erasure consequence

[ADR-0052](0052-born-sealed-clinical-bodies.md)'s two doors enforce **sealed ⇒ clinical**. A registration is
demographic-plane, so its body is written **in the clear** — including the third-party candidate UUIDs of
decision 2.

**Recorded consequence, so it cannot honestly be missed:** an
[ADR-0005](0005-erasure-key-custody-and-crypto-shredding.md) rung-2 ("deniable") erasure of a candidate's
identity **must reach the registration attestations that name them**, or the erased chart stays discoverable
by anyone who can read the funnel's record. This is structurally the same footnote
[§5.5(a)](../identity.md#55-reattribution-one-primitive-tiered-workflows) already carries for the matcher's
known-alias pool: derived or incidental state that names a subject is part of the erasure surface, even when
it was never the subject's own record.

### 6. A registration asserts everything it was given — name, DOB and identifiers — with provenance `registrar-entered`

A standard registration authors the `demographic.field.asserted` name and/or DOB **and one
`demographic.identifier.asserted` event per supplied identifier**, all **in the same transaction** as the
registration act, when and only when each was actually supplied. Without this the funnel does not close: the
search reads **`patient_identifier`, `patient_demographic` and `patient_name`** — one projection per blocking
pass — so a registration that wrote none of them left the chart unfindable and the very next search for the
same person minted a duplicate. Nothing is fabricated to fill a gap: a registration with no name given writes
no name, because a placeholder would be a precise untruth (principle 4) and worse than the honest absence.

The rule was found by running the real CLI end to end, in two halves, and the second half is the more
instructive one:

- **[#350](https://github.com/cairn-ehr/cairn-ehr/issues/350)** — the act wrote no name and no DOB, so passes
  2 and 3 could not find the chart the funnel had just created.
- **The same defect one pass over, caught in this branch's final review.** `patient-register` accepts
  repeatable `--identifier system=value`, parses it strictly, **searches on it and signs it into the permanent
  attestation** — and then discarded it. Pass 1 is the *highest-precision* pass and the one gesture the floor
  blesses as "a complete and often better search", and `--identifier` on `patient-register` is the only place
  in the CLI an operator can enter an MRN at all. So a clerk registering from an MRN card and later searching
  that same MRN got nothing back, and — worse — an identifier-only registration (no name, no DOB, explicitly
  supported) produced a chart with **no searchable content on any of the three passes**: unreachable,
  permanently, by every search the slice ships.

The lesson recorded, because two rounds found the same shape: **an act that searches on a term and attests to
that term must also persist it.** Anything else signs a diligent-looking record of a search whose own subject
the record cannot later be found by.

**The identifiers are asserted with no `normalized` key and no `profile`**, and that is a correctness choice
rather than a stub. A registration desk holds no §4.4 comparator profile ([ADR-0014](0014-locale-pluggable-matcher-comparators.md)),
so naming one would be a fabrication, and the floor refuses a materialised `normalized` key that does not name
the profile which produced it. With both absent the projection's `match_key` falls back to the as-entered
`value`, which is exactly what the identifier pass compares a clerk's typed value against.

**The value is stored TRIMMED, and so is the query — maintainer decision, final review.** The original text
here read *"the value is stored verbatim, untrimmed: the pass is an exact compare, so silently tidying the
stored value would make the chart unfindable by the very query it was registered from."* That reasoning was
correct as far as it went, but it was solving only half the problem: it is true only because the *query*
side did not trim either, so leaving the stored side untrimmed merely kept the two sides consistently wrong
together. The maintainer's fix is to trim BOTH sides — `crates/cairn-node/src/patient/register.rs`'s
`supplied_identifiers` on the way in, `crates/cairn-patient-search/src/query.rs`'s `SearchQuery::new` on the
way out — matching what `birth_date` already does one field over
(`SearchQuery::new` in `crates/cairn-patient-search/src/query.rs`: *"a clerk's stray leading/trailing
space … must not silently defeat it"*). There is no principled reason a pasted identifier should be held
to a laxer standard than a typed date; the earlier asymmetry was an oversight, not a considered
distinction.

This closes a real cross-gesture miss, not a hypothetical one: a clerk pastes an MRN card into
`patient-register --identifier "MRN= 12345"` (trailing space intact, as pasted), the chart is created and
the space-padded value stored; later, at `patient-search --identifier MRN=12345` (typed clean, no padding),
db/046 pass 1's `pi.value = (q ->> 'value')` compares `"12345 "` against `"12345"` and finds nothing. The
funnel had just attested to searching on that exact identifier and signed it into a permanent record — and
the very next search for it, by anyone, missed the chart it created. Trimming both sides makes the stored
value and every future query's value agree bit-for-bit, so a search on the same identifier — padded or not,
on either end — always finds what was registered.

**The provenance is `registrar-entered`, not `patient-stated`**, and the reasoning is the maintainer's:

> `patient-stated` would frequently be a *precise untruth*. At a registration desk the speaker is often a
> third party — a parent for a young child, a carer for a cognitively or speech-impaired patient.
> `registrar-entered` is honest about what actually happened: a clerk wrote down what they were told, by
> someone, unverified.

This is principle 4 at the level of a vocabulary term. `patient-stated` claims to know *who spoke*;
`registrar-entered` claims only what the registrar can vouch for. The weaker term is the true one, and a
true weak claim beats a false strong one.

**Live consequence, tracked rather than glossed:** `registrar-entered` is not yet ranked in `db/011`'s
[§4.1](../demographics.md#41-demographic-assertions) provenance ladder, so
`cairn_provenance_rank` gives it the unrecognised-term default of **0** — below `inferred`. That default is
the safe direction for an unknown term (an unknown term can never *displace* a known-provenance value), but
it is the wrong rank for this one: a registrar's typed DOB currently loses to worse evidence.
[#351](https://github.com/cairn-ehr/cairn-ehr/issues/351) holds it open; ranking the term is a decision for
whoever next audits the whole ladder, not a silent edit here.

**A known residual on this same path, stated rather than left implied.** The argument above refuses a
fabricated placeholder *name* as a precise untruth — while the DOB written beside it is validated only for
**shape**, never for calendar validity, so `1980-13-45` is accepted as day precision and signed permanently.
That is a fabricated *date* in an immutable record, i.e. the same fault this decision rejects, surviving one
field over. It follows from the deliberately parse-free, culture-neutral floor rather than from an oversight,
but the inconsistency is real and is tracked at
[#352](https://github.com/cairn-ehr/cairn-ehr/issues/352).

### 7. John Doe is deliberately asymmetric, and the asymmetry is correct

A §5.4 unidentified registration asserts a **callsign** (so the chart is findable and renders an obvious
placeholder header) and **no name and no DOB** — because there are none. Asserting either would be the
precise untruth principle 4 forbids, and a plausible fake name is precisely what §5.4 already prohibits.

That is not an inconsistency with decision 6. The rule is symmetric — **assert what you know** — and the
content differs because the knowledge differs. §5.4 routes what *is* known about an unidentified patient
through **observed-evidence assertions**: estimated age with its basis, observed sex, photo, distinguishing
marks, belongings, EMS pickup context. Those are a different and honest claim, with their own provenance,
and they feed the matcher as real features. The John Doe path asserts everything it knows and nothing it
does not, exactly as the standard path does.

Correspondingly, `search` is **structurally absent** — not empty — for the non-standard classes, and the
floor refuses a non-standard registration that carries one at all. A search attestation on an unconscious
patient would be a precise untruth about an act nobody performed. §5.4 already answers this correctly and
differently: the matcher re-runs on every new evidence assertion, i.e. **search-after-create, by necessity**.

## Consequences

**Easier.**
- The six-months-later investigation is answerable from the record itself, with no reconstruction and no
  guessing which of two opposite fixes is called for.
- The precedence rule, once turned on (#345), is a single sentence with no exceptions, which is the only
  form a safety floor is reliably reviewable in.
- Charts now record their registration class, a §5.3 fact no code previously held.
- The funnel and the §5.4 identification path are the **same surface**: candidate search returns
  identity-pending charts with their trust state, so a John Doe registered an hour ago is exactly the chart
  a clerk finds when the family arrives with a name. A search that hid them would force a duplicate every
  time an unidentified patient is later named.

**Harder.**
- Every registration now writes a permanent record naming third parties in the clear, so rung-2 erasure has
  one more surface to reach (decision 5).
- The attestation's honesty depends on the displaying surface and the attesting act never disagreeing about
  what was shown, and **the type system does not enforce that** — `SearchAttestation`'s fields are public and
  the wire builder takes a bare `&[Uuid]`, so a caller *can* construct an attestation naming candidates
  nobody displayed. What we have instead is convention plus one test: there is **one constructor**
  (`SearchAttestation::from_displayed`, derived from the `CandidateList` that was shown) and **one conversion
  site** between the read model and the wire builder (`register::build_registration_body`), pinned by the
  round-trip test in `cairn-node/tests/patient_register.rs`. That is a *disciplined-caller* guarantee, not a
  structural one — stated precisely because a permanent record that overclaims here would be trusted years
  later. Closing the gap properly means a constructor-only type, and that is future work, not a claim to
  make now.
- The candidate search adds a latency-sensitive path to registration, which paper-parity
  ([§1.2](../vision.md#12-the-paper-parity-test-normative)) budgets at ≤ 5 s to find an existing chart and
  ≤ 20 s to register a new one. The interactive measurement is owed by the first slice with a runnable
  surface; the CLI slice measures the node-tier write cost only.

  > **Erratum E3 (2026-08-06) — factual; the decision is unchanged.** *"the CLI slice measures the
  > node-tier write cost"* describes work that **was not done**: nothing is wired into `patient-register`,
  > no results artifact exists, and `db/044`'s `gesture_kind` CHECK (`signoff`, `cease`) would refuse a
  > registration gesture row outright until an additive migration widens it. **BOTH** halves of the §1.2
  > measurement are therefore owed — the interactive half by the first slice with a runnable surface, the
  > node-tier write-cost half as [#360](https://github.com/cairn-ehr/cairn-ehr/issues/360). The budget
  > itself (≤ 5 s to find an existing chart, ≤ 20 s to register a new one) is unchanged; only the claim
  > that half of it had already been measured is wrong.

**The bet.** That an honest, *advisory* funnel plus a named-candidate record beats an enforcing one. The
search never blocks, never vetoes and never auto-decides, because a missed candidate produces a **false
split** — §5.2's explicitly safe direction — and ADR-0014 already names the standing backstop: the hub-tier
aggressive background duplicate sweep, whose worklist yield doubles as the miss-rate metric. What is
safety-critical is not *finding* the duplicate but **whether a registration carries a well-formed
attestation** — a property of the event itself, therefore checkable in the database and, once db/045 is
loaded, unbypassable even by a client talking raw SQL (principle 12). Note the exact scope, because it is
narrower than it first reads: the floor governs the *shape* of a registration that occurs. That one
**occurs at all** before clinical content lands is the precedence rule, and that is
[#345](https://github.com/cairn-ehr/cairn-ehr/issues/345) — see decision 3.

**How we would know the bet fails.** The hub sweep's worklist yield rising rather than falling once the
funnel is in routine use — duplicates being created at a rate the funnel is not reducing. Two distinct
diagnoses, and decision 2 is what lets the record distinguish them: duplicates whose twin **was** displayed
means the display or the ordering is at fault; duplicates whose twin was **not** displayed means recall is.
If the answer turns out to be neither — the search ran, the twin was shown, the clerk read it and created
anyway, repeatedly — then the funnel is being defeated by workload, and the answer is a paper-parity
investigation of the registration desk, not a stricter floor.

**First instance.** `db/045_patient_registration.sql` (the structural floor + the retained-set
`patient_registration` projection with its earliest-wins current view), `db/046_patient_search.sql`
(`cairn_search_candidates`, the three-pass advisory disjunction), `crates/cairn-patient-search` (the pure
read model and the one definition of what a registration attests), `cairn-event::registration` (the wire
shape), `cairn-node`'s `patient::search` / `patient::register`, the `patient-search` / `patient-register`
CLI verbs, and `john_doe.rs` re-expressed onto the same act.
