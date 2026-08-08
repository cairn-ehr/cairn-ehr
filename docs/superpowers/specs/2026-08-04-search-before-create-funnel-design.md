# Design — the §5.3/§5.8 search-before-create funnel (node tier)

**Date:** 2026-08-04
**Status:** approved in brainstorming, ready for a plan
**Layer:** in-DB enforcement floor + node library tier + CLI (no UI this slice)
**Discharges:** [#344](https://github.com/cairn-ehr/cairn-ehr/issues/344)
**Governing:** [§5.3](../../spec/identity.md#53-registration-classes) (registration classes) ·
[§5.8](../../spec/identity.md#58-registration-documentation-workflow-normative) (the normative funnel) ·
[§5.4](../../spec/identity.md#54-unidentified-registration-john-doe-baked-into-the-root) (John Doe) ·
[ADR-0014](../../spec/decisions/0014-locale-pluggable-matcher-comparators.md) (advisory matcher; the hub sweep is the false-split backstop) ·
[ADR-0022](../../spec/decisions/0022-validated-submit-surface-the-write-path.md) (the validated write door) ·
[ADR-0048](../../spec/decisions/0048-twin-check-registry-dispatch.md) (twin-check registry) ·
[ADR-0051](../../spec/decisions/0051-contributor-role-vocabulary-floor-and-responsibility-wire-shape.md) (strict-submit / lenient-apply) ·
[ADR-0058](../../spec/decisions/0058-grade-gated-teffective-ceiling.md) (a remote door admits and flags, never rejects) ·
[ADR-0053](../../spec/decisions/0053-per-write-human-authorship.md) (per-write human authorship) ·
[§5.11](../../spec/identity.md#511-point-of-care-identity-possession-fast-authentication-and-salvage) (authorship confidence is a grade, not a gate) ·
[ADR-0052](../../spec/decisions/0052-born-sealed-clinical-bodies.md) (sealed ⇒ clinical) ·
[ADR-0060](../../spec/decisions/0060-partial-validity-a-defect-on-one-line-never-invalidates-another.md) (partial completion is reported, never implied) ·
principle 2 (identity is a claim) · principle 3 (paper-parity) · principle 4 (acknowledged uncertainty) ·
principle 12 (the floor is unbypassable)

**ADR-0061 lands with this slice.**

---

## 1. Purpose & scope

Make **registration a first-class act** that carries the search which preceded it, and give the node
a patient search to preceed it with.

Two facts make this bigger than "add a search":

1. **There is no standard patient registration path at all.** `cairn-node` has `register-john-doe`,
   `enroll-human`, `identify-patient` and the medication verbs. A standard chart currently comes into
   being as a *side effect* of the first event carrying its `patient_id`. So §5.8's normative
   sentence — *"the create button records that N near-matches were displayed"* — has nowhere to
   attach, and that recording obligation is event-shaped, i.e. the can't-retrofit kind.
2. **There is no patient search of any kind.** The §5.2 matcher does batch pairwise sweeps over the
   whole population; nothing answers *"a clerk typed a name — which charts might this be?"*.

**In scope:** the registration event type and its structural floor check, the candidate search, the
pure read/attestation model, the `patient_registration` projection, two CLI verbs, and re-expressing
John Doe onto the same act.

**Split out to a follow-on PR** (§2.3, measured not assumed): the precedence rule's *enforcement*,
retiring `patient.created`, and the ~83-call-site fixture sweep the two require.

**Out of scope, deliberately** (§8 states every gap): all UI, candidate scoring/ranking, photo bytes,
the §5.6 pseudonymous *workflow*, and matcher convergence on the blocking keys.

## 2. Governing decisions (settled during brainstorming)

### 2.1 The obligation is floor; finding candidates is advisory

This is [ADR-0014](../../spec/decisions/0014-locale-pluggable-matcher-comparators.md)'s split
applied unchanged. A missed candidate produces a **false split** — §5.2's explicitly safe direction
("false merge ≫ worse than false split") — and ADR-0014 already names the standing backstop: the
hub-tier aggressive background duplicate sweep, whose worklist yield doubles as the miss-rate metric.
So candidate-finding never blocks, never vetoes, and never auto-decides.

What *is* safety-critical is the record of the act: whether a registration carries a well-formed
attestation is a property of the event, checkable in the database, and therefore unbypassable by a
client talking raw SQL (principle 12).

**Consequence for implementation:** the search is SQL + Rust, not a Python round-trip. A registration
path must beat paper (§5), and the §5.11 latency limb is explicit — *type a few chars and enter, no
spinner*. Calling the advisory Python tier synchronously inside the funnel would couple two services
on the latency path for no safety gain.

### 2.2 One registration act, three classes — so the floor rule has no "unless"

§5.3 already defines exactly three registration classes. Expressing them as one event type with a
`class` discriminant means the precedence rule (§2.3) reads *"the first event carrying a new
`patient_id` must be a registration"* — full stop, no carve-out. The alternative (a new type for
standard charts only, John Doe untouched) forces the safety floor to carry named exceptions, and an
"unless" in a safety floor is where the next defect lives.

Re-expressing the shipped John Doe path costs tests and no data migration: the project is
pre-clinical, and the wire discipline that matters is never *breaking* the wire, which adding a type
does not do ([ADR-0056](../../spec/decisions/0056-unknown-event-types-admitted-uninterpreted.md) —
unknown types are admitted uninterpreted, so a peer on older code carries this event without harm).

It also records a fact §5.3 asserts and no code holds today: **which class a chart was registered
under.**

### 2.3 Strict local submit, lenient remote apply — decided here, enforced in the follow-on

The precedence rule (*the first event carrying a new `patient_id` must be a registration*) is
enforced at `submit_event` and **not** at `apply_remote_event`.

> [!IMPORTANT]
> **The rule is settled; its enforcement ships in a separate PR, and this section records why.**
> Measuring the change rather than assuming it turned up two facts this design originally missed:
>
> 1. **`patient.created` already exists** — a walking-skeleton event type classified `additive`
>    (`db/005`), projecting to `patient_chart` through `patient_chart_apply` at run_order 10, with a
>    `{name, dob, sex}` payload superseded by demographics slices 1–5, **no structural floor** and no
>    twin-check row. It is an unfloored registration act. It must be **retired** by the same change
>    that turns the rule on — grandfathering it as a permitted first event would put back exactly the
>    "unless" §2.2 exists to remove.
> 2. **The rule converts ~83 submit call sites across ~38 `cairn-node` test files**, plus a *very
>    small* number of `patient.created` references in `cairn-sync`/`cairn-event` — the "37" this
>    section originally stated was a large overcount (see the correction note below). Only 4 files use
>    the existing `submit_patient_created` helper; the rest build bodies inline. That is the whole
>    DB-gated suite.
>
> A mechanical rewrite of 38 test fixtures deserves its own review, where each converted fixture's
> intent can be checked. Bundled into this slice it would swamp ~8 files of actual design. So this
> slice builds the funnel and the follow-on makes it unbypassable.
>
> **The funnel is therefore complete but not yet unbypassable when this slice lands.** A client can
> still mint a chart by asserting a name. Stated plainly rather than implied — the same discipline
> ADR-0060 decision 2 applies to clinical output.

> **Corrected after implementation (second whole-branch review).** Item 2's *"37 `patient.created`
> references in `cairn-sync`/`cairn-event`"* was a large overcount, and a first pass at correcting it
> ("8 literal strings, 13 counting helper names") was wrong too — recorded here so #345 is not scoped
> from either figure. As measured: those two crates hold **9** textual occurrences (6 `cairn-sync`, 3
> `cairn-event`), of which only **5** are event-type literals in code; the other 4 are comments.
> `submit_patient_created` appears in **neither** crate — all 19 uses are `cairn-node` tests already
> counted by the ~83/~38 figure, so any combined total double-counted them. What matters more than the
> size is the shape: `cairn-event` has **no Postgres dependency**, so its 3 occurrences are
> CBOR/serialization fixtures that never submit anything, and `cairn-sync`'s `emit_event` persists via
> a raw `INSERT INTO event_log` rather than `submit_event`, where §2.2 places the precedence rule.
> Whether #345 also retires the event-type *name* in fixtures is a separate, deliberate scoping call.
> The ~83/~38 `cairn-node` figure — the reason this is a separate slice — is unaffected. Mirrored as
> [ADR-0061](../../spec/decisions/0061-registration-is-an-act-that-carries-its-search.md) erratum E1.

Set-union sync has no ordering guarantee, so a peer's clinical event legitimately arrives *before*
the registration event that licenses it. A fail-closed remote door would then wedge replication on
entirely honest traffic — the failure mode this project has now hit three times
([ADR-0056](../../spec/decisions/0056-unknown-event-types-admitted-uninterpreted.md),
[ADR-0058](../../spec/decisions/0058-grade-gated-teffective-ceiling.md), and
[#268](https://github.com/cairn-ehr/cairn-ehr/issues/268)). ADR-0051's strict-submit/lenient-apply
and ADR-0058's *"remote door admits-and-flags, never rejects"* are the same pattern; this slice
follows it rather than re-deriving it.

Note the rule is naturally self-satisfying afterwards: once a peer's clinical event has landed, a
local write to that chart is no longer a *first* event, so no local refusal follows from the
lenient admission. The only residue is a chart with no registration row until the peer's registration
syncs, which stays **queryable** (a one-line query over the projection) but unflagged in the UI this
slice — a deliberate deferral, filed as an issue.

### 2.4 The attestation names candidates; it does not count them

The failure mode the funnel exists to serve: a duplicate surfaces six months later, and the
investigator must answer **was the duplicate on the screen when the clerk clicked create?**

- **Yes** → human judgement failed. Fix the UI, the display fields, or the training.
- **No** → the search failed. Fix the comparator, the blocking keys, or the recall.

A bare `N = 3` cannot distinguish those two, and they have opposite fixes. So the attestation carries
the candidate `patient_id`s actually displayed.

The displayed-and-not-chosen set is *weak, honestly-graded* evidence — the clerk may never have read
it. It is **not** an `unlink` and must never be projected as a judgement that the charts differ.
Whether it later feeds the §5.2 matcher's missing gold set is a separate question, deliberately not
answered here.

### 2.5 The registration body is not born-sealed, and that has a consequence

[ADR-0052](../../spec/decisions/0052-born-sealed-clinical-bodies.md)'s two doors enforce
**sealed ⇒ clinical**; a registration is demographic-plane, so its body is written in the clear —
including the third-party candidate UUIDs of §2.4.

Recorded consequence: an [ADR-0005](../../spec/decisions/0005-erasure-key-custody-and-crypto-shredding.md)
rung-2 ("deniable") erasure of a candidate's identity must reach the registration attestations that
name them, or the erased chart stays discoverable by anyone who can read the funnel's record. This is
structurally the same footnote §5.5(a) already carries for the matcher's known-alias pool, and it is
stated here so the rung-2 implementation cannot honestly miss it.

### 2.6 The registrar is recorded and graded — never required

An attestation whose purpose is the six-months-later investigation should answer **who was looking**,
not only *what was on screen*. The tempting move is to make `class=standard` refuse without a bound
human author. **That move is wrong, and this section exists to record why**, because it is the
obvious thing for a future reader to "fix".

**The mechanism already gives the honest half for free.** `cairn_authorship_bound` runs
*unconditionally on every event* at the submit door (`db/005`, step 4b): any responsibility-bearing
contributor's `actor_id` must be the event's signer or the verified attester. So a registration that
*names* a human registrar is already unforgeable, with no new floor rule. What the door deliberately
does not do is require such a contributor to exist —
`cairn_event::contributor::classify_authorship_confidence` grades a bearing-less event `Device`
rather than refusing it.

**Requiring one would violate §5.11 outright:** *"Authorship-confidence is a grade, not a gate…
where author identity cannot be cheaply established (badge forgotten, two in range, emergency), the
system never blocks."* Three concrete failures follow from gating:

1. **It blocks care documentation, not just registration.** 03:00 in the ED, the clerk's personal key
   is not unlocked (locum's first shift, dead reader, enrolment ceremony never run). The registration
   is refused — and because the §2.3 precedence rule makes registration the *first* event for a new
   `patient_id`, nothing can be recorded about that patient at all.
2. **It trains staff to degrade the record.** `class=unidentified` needs neither search nor author,
   so the gate's real-world effect is to push named, cooperative patients through the John Doe path
   to get past the prompt. A control people route around by degrading the record is not a control.
3. **It is self-defeating on its own terms.** A gate that refuses produces *no forensic record*. For
   a mechanism that exists for the later investigation, refusing to write is strictly worse than
   writing "registrar unattributed" — which is at least true, auditable, and honest.

**So: no `db/005` change, and no new authorship rule anywhere.** `patient-register` takes an
*optional* `--attester-key`. When the registrar signs, the existing binding makes the claim
unforgeable and the existing classifier grades it `Attested`. When they cannot, the event records
`Device` — *authored at this node, registrar unattributed* — never a guess (principle 4's explicit
unknown), composing into the §5.7/§5.10 trust projection with **no new stream**.

> **Corrected after implementation (second whole-branch review).** The optional
> `--attester-key` was never built: `patient-register` has no such flag and `register_patient` takes
> no attester parameter — every registration the shipped slice authors is graded `Device`. The floor
> half of this section is real (db/005's unconditional `cairn_authorship_bound`); the opt-in attested
> path is future work, tracked as [#359](https://github.com/cairn-ehr/cairn-ehr/issues/359), and must
> not be assumed by anything reading this document.

**Wanting attested registrations is policy, not mechanism** (principle 9). A deployment that requires
it expresses it as [ADR-0024](../../spec/decisions/0024-hard-policy-expression-the-policy-assertion-stream.md)
hard policy or a role gate; the CLI and the later UI nudge as soft policy (ADR-0021). Cairn ships the
grade. The quality signal survives regardless: *"standard registrations graded `Device`"* is a
one-line query and belongs on the same hub worklist as ADR-0014's duplicate sweep.

Where a registrar *does* enrol, they do so as their own role-actor (`enroll-human --role registrar`)
— the (entity, role) model working as designed; one person may hold both a clinician and a registrar
actor, linked by a shared registration id (ADR-0044,
[#168](https://github.com/cairn-ehr/cairn-ehr/issues/168)).

## 3. The event

`identity.registration.asserted`, `schema_version` 1. Registered as **one additive row** in the
ADR-0048 twin-check registry (`cairn_event_twin_check`) with a check fn
`cairn_check_registration_assertion(p_type text, b jsonb) RETURNS void`.

```json
{
  "class": "standard" | "unidentified" | "pseudonymous",
  "basis": "<free text: why this class — required for the non-standard classes only>",
  "search": {
    "query": {
      "name_tokens": ["smith", "john"],
      "birth_date": "1980-01-01",
      "identifiers": [{ "system": "MRN", "value": "12345" }]
    },
    "displayed": ["<patient uuid>", "<patient uuid>"],
    "incomplete": false
  }
}
```

Four deliberate shape choices:

- **`basis` is required only for the non-standard classes.** For a standard registration the class
  *is* the explanation; a mandatory free-text box there would be a required field satisfiable only by
  fabrication, which principle 4 forbids outright.

- **No `displayed_count`.** Two representations of one number is a lie waiting to happen; the count
  is `length(displayed)`. The floor never has to reconcile them because they cannot disagree.
- **`search` is structurally absent for the non-standard classes, not empty.** A search attestation
  on an unconscious patient would be a *precise untruth* (principle 4). §5.4 already answers John Doe
  differently and correctly: the matcher re-runs on every new evidence assertion — search-*after*-
  create, by necessity.
- **`incomplete` is required, not optional.** ADR-0060 decision 2 (*partial completion is reported,
  never implied*) binds here as much as on the drug chart: if the node could not read some candidate
  it found, the attestation says so rather than implying the search was exhaustive.

The body carries the mandatory authored §3.13 legibility twin like every other event (ADR-0039); the
twin renders the class, the basis, and the number of candidates displayed in plain language.

## 4. The floor

New migration `db/045_patient_registration.sql`.

### 4.1 Structural check (`cairn_check_registration_assertion`)

| Rule | Refusal reason |
|---|---|
| `class` present and ∈ {`standard`, `unidentified`, `pseudonymous`} | unknown registration class |
| `class≠'standard'` ⇒ `basis` present, non-blank text | a non-standard registration states why |
| `class='standard'` ⇒ `search` present and an object | standard registration must carry its search |
| `class≠'standard'` ⇒ `search` absent | a search attestation the clerk could not have made |
| `search.displayed` present, an array, every element a UUID | candidate list malformed |
| `search.incomplete` present and boolean | completeness must be stated, not assumed |
| `search.query` present, an object, at least one non-empty term | a search with no terms is not a search |

**`search.displayed` MAY be empty**, and that is the *normal* case for a genuinely new patient — a
search that correctly found nothing. `[]` and "no search ran" are distinguished by the presence of
the `search` object itself, not by its length. Nobody should later "tighten" this into a non-empty
requirement: doing so would make registering the first patient on a fresh node impossible.

**No authorship rule is added here** (§2.6). The unconditional `cairn_authorship_bound` at step 4b
already makes a named registrar unforgeable; requiring one is policy, not floor.

The check is pure (`p_type`, `b`) → void, matching the ADR-0048 unified signature, so registration in
the dispatcher is one row and the dispatcher itself is untouched.

### 4.2 Precedence predicate — FOLLOW-ON PR, not this slice

```sql
cairn_patient_has_events(p_patient_id uuid) RETURNS boolean   -- pure, one indexed lookup
```

`submit_event` (db/005) refuses a non-registration event whose `patient_id` has no prior event.
`apply_remote_event` (db/020) does not call it at all (§2.3).

**Neither the predicate nor its call site is built in this slice** (§2.3): it ships with the
`patient.created` retirement and the fixture sweep, so the enforcement and the ~83 call sites it
converts are reviewed together.

> **Shipped 2026-08-08 (#345).** The predicate is `cairn_patient_has_events(uuid)` in **db/001**
> (beside the `event_log_patient_idx` it reads); the refusal is **db/005 step 8b**, placed last
> among the door's refusals so an event that is wrong in two ways still reports the defect in
> ITSELF first. `apply_remote_event` was left untouched, and `patient_precedence.rs` asserts the
> lenient remote admission directly so a future "make the doors symmetric" change fails loudly.
> The retirement is **db/047**, which drops `patient.created`'s projection rows before its
> classification row — the order db/005's own registry-validation trigger requires, now recorded
> there as the precedent for any future retirement. Registration also took over the
> `patient_chart` chart-birth projection, so a chart registered moments ago reports its
> registration date as last activity instead of nothing.

### 4.3 Projection

`patient_registration` — a **retained set**: every registration event keeps a row, exactly as
`patient_name` retains every name. A `patient_registration_current` VIEW picks the **earliest** by
`(hlc_wall, hlc_counter, node_origin COLLATE "C", content_address)` as the birth record.

Earliest-wins rather than the usual latest-wins overlay because a registration is a *birth* act, not
a standing state; retained-set rather than one row because evidence is never discarded (principle 1).
The tiebreak is collation-independent per
[ADR-0045](../../spec/decisions/0045-collation-independent-projection-tiebreaks.md), and the apply fn
is registered in the ADR-0057 dispatcher like every projection since.

## 5. The search

`cairn_search_candidates(p_name_tokens text[], p_birth_date text, p_identifiers jsonb)` — a SQL
function returning candidate `patient_id`s from a three-pass disjunction, mirroring the matcher's
existing blocking design:

1. **shared identifier** — `patient_identifier` on `system` plus (`match_key` OR the raw `value`) —
   widened after an earlier fix so a stored row with no `normalized` key still matches on its
   as-entered value (`db/046_patient_search.sql`)
2. **exact DOB** — `patient_demographic` on the birth-date field
3. **shared name token** — `patient_name` token overlap

Union, dedup, no scoring, no ranking, and — **as built** — no ceiling on the result set at all.

> **Corrected after implementation (final review of this branch).** This section originally promised
> "an oversized-block guard that **reports rather than silently caps**". No such guard was built and
> none exists. The `incomplete` flag in §3 is raised by a *different* condition — a candidate whose
> display name could not be read — and never by result-set size. Nothing anywhere caps, reports on,
> or even measures how many candidates a blocking pass returns. Tracked as
> [#357](https://github.com/cairn-ehr/cairn-ehr/issues/357); it is not shipped and must not be
> assumed by anything reading this document.

Candidate assembly happens in Rust over the existing projections and carries, per §5.8 item 1:
display name, **age with its basis**, §5.7 **trust state**, last activity (`patient_chart`), a locale
one-liner (`patient_address_current`), and a **photo reference** — the blob digest, never bytes.

**Trust state is load-bearing, not decoration.** A John Doe registered an hour ago is precisely the
chart a clerk must find when the family arrives with a name: the funnel and the §5.4 identification
path are the same surface. A search that hid identity-pending charts would force a duplicate every
time an unidentified patient is later named.

## 6. Code shape

| Unit | Contents | Why here |
|---|---|---|
| `crates/cairn-patient-search` (**new, pure**) | `SearchQuery`, `Candidate`, `CandidateList`, `SearchAttestation::from(&CandidateList)` | No Postgres driver, so the future picker window can depend on it — the `cairn-medication-view` precedent. `SearchAttestation` built *from* the displayed list is the one definition of what a registration attests, so the surface that displays and the act that attests cannot disagree (the Slice 61 lesson; here a divergence means swearing to candidates the clerk never saw) |
| `crates/cairn-event/src/registration.rs` | pure body builders + the twin renderer | Sibling of `cairn-event::demographics` |
| `crates/cairn-node/src/patient/search.rs` | `search_patients(&client, &SearchQuery) -> CandidateList` | The ONE mapping, as `medication/read.rs` is for the drug chart; the future native API wraps it |
| `crates/cairn-node/src/patient/register.rs` | mint UUID → build body from the shown `CandidateList` → sign as the registrar when a key is available (§2.6, optional) → submit, one transaction | A chart is never half-registered (the John Doe precedent) |
| `crates/cairn-node/src/john_doe.rs` | emit the registration act as its first event | §2.2 |
| CLI | `patient-search`, `patient-register` | |

Every file targets < 500 lines; the pure crate splits by concept (`query` / `candidate` /
`attestation`) rather than growing one module.

## 7. Paper-parity benchmark (§1.2)

**Paper counterpart:** the registration desk — clerk, card index or day book, folder tabs.

| | Acts |
|---|---|
| **Paper N** | **3** — ask name + DOB · look it up in the index · write a new card and folder tab if absent |
| **Architecture-forced M** | **3** — the architecture forces a search *to have run* and its attestation to be *carried*; neither forces a discrete second gesture, because type-ahead fuses entry and search (§5.11: "type a few chars and enter, no spinner") |
| **UI bundling target K** | **2** — type-and-see → commit. Reviewing candidates is reading, not an act |

`M = N`, so no architecture defect under house rule 7.

**Budget:** ≤ **5 s** to find an existing chart (by far the commoner path) and ≤ **20 s** to register
a new one, first keystroke to committed chart.

**Measurement owed:** by the slice that first exposes a runnable surface. This slice is CLI-only, so
it measures the **node-tier write cost** as Slices 61/62 did (`cairn-node`'s existing
`ui_timing`/gesture-timing capture), and states the interactive half as owed. If a measured figure
falls outside the budget, **that is the finding** — file it; do not move the budget to fit.

> **Corrected after implementation (second whole-branch review).** The write-cost measurement
> was not done either: nothing is wired into `patient-register`, no results artifact exists, and
> `db/044`'s `gesture_kind` CHECK (`signoff`, `cease`) would refuse a registration row until an
> additive migration widens it. BOTH halves of the §1.2 measurement are owed — the interactive half
> by the first runnable surface, the node-tier write-cost half as
> [#360](https://github.com/cairn-ehr/cairn-ehr/issues/360).

## 8. Deliberately not in this slice

- **No UI, and specifically no picker inside the med-list window.** `--patient <uuid>` at launch is
  already possession-shaped (one window, one chart, chosen once, never swappable); a picker that
  *retargets* an open window re-creates the §5.8 item 4 / §5.11 windowing misfile that possession
  exists to prevent. The UI slice must **open** a chart, never retarget one. Filed separately.
- **No candidate scoring or ranking** — advisory matcher tier; the hub sweep stays the false-split
  backstop (§2.1).
- **No photo bytes** — candidates carry the blob reference; the byte tier is ADR-0013 work.
- **No §5.6 pseudonymous workflow.** The class exists in the enum so the floor is complete; the
  consent-gated linking §5.6 requires is its own slice.
- **The precedence rule's enforcement, retiring `patient.created`, and the ~83-call-site fixture
  sweep** — [#345](https://github.com/cairn-ehr/cairn-ehr/issues/345), the reason in §2.3.
- **No unregistered-chart UI flag** (§2.3) — queryable, not surfaced.
  [#354](https://github.com/cairn-ehr/cairn-ehr/issues/354).
- **No policy expression of "registrations must be attested"** (§2.6). The grade is shipped and the
  worklist query is trivial; turning that into a site requirement is ADR-0024 hard-policy work, and
  belongs to whoever has a deployment that wants it.
- **No matcher convergence on the blocking keys.** `cairn_search_candidates` and
  `matcher/pipeline/db.py` will each extract identifier / DOB / name-token keys. They are not the
  same query (the sweep blocks all × all; the funnel maps query → set), so the shared part is the key
  extraction, not the driver. [#353](https://github.com/cairn-ehr/cairn-ehr/issues/353), rather than let
  the drift pass silently (house rule 5).

## 9. Testing (TDD — failing test first)

**Pure, no DB** (`cairn-patient-search`, `cairn-event::registration`): query normalization;
`SearchAttestation::from(&CandidateList)` including `incomplete` propagation; age-with-basis
derivation; body builders and the twin's rendering; a registration body for a non-standard class
carries no `search` key at all.

**Floor, DB-gated** (`db/tests/045_patient_registration_test.sql` + a Rust sibling):

- a class outside the three is refused
- `class=standard` with no `search` is refused
- `class=unidentified` **with** a `search` is refused (the trap: absence must be structural)
- a non-UUID in `displayed` is refused; a missing `incomplete` is refused
- an empty query object is refused
- the twin requirement fires
- an **empty** `search.displayed` succeeds — a search that found nothing is the normal first-patient
  case, and the test exists so it cannot be tightened away
- `class=standard` with **no** human author **succeeds**, graded `Device` (§2.6). This test is the
  guard against a future reader turning the grade back into a gate; its failure message should say so
- `class=standard` naming a registrar who is neither signer nor verified attester is refused by the
  *existing* unconditional binding — asserted here to prove §2.6's "unforgeable for free" claim
  rather than assume it
*(The precedence tests — a bare assertion on a fresh `patient_id` refused, the same assertion after a
registration accepted, and the load-bearing lenient case where `apply_remote_event` admits an
out-of-order clinical event with no registration — belong to the follow-on PR that turns the rule on,
§2.3/§4.2.)*

**Search, DB-gated:** each blocking pass finds its candidate; an identity-pending (John Doe) chart is
returned **with** trust state `unconfirmed`; a candidate whose demographics cannot be read is
reported through `incomplete`, never silently dropped. *(An "oversized-block guard reports" test was
listed here and is NOT part of the shipped suite — no such guard was built; see the correction in §5
and [#357](https://github.com/cairn-ehr/cairn-ehr/issues/357).)*

**Regression:** the existing `john_doe.rs` suite passes with the new first event, and its
registration remains atomic in one transaction.

**Guards that must be updated in the same commit** (each has bitten this repo before):

- twin-registry row count **+1 in both** `crates/cairn-node/tests/twin_registry.rs` **and**
  `db/tests/034_twin_registry_test.sql`
- `SCHEMA_GENERATION` bump with the `db/045` entry appended to `db.rs`'s `SCHEMA` list in the same
  commit (the guard test pins this)
- full-workspace `cargo test` — not `-p cairn-node` — so a cross-crate call-site arity change in
  `cairn-sync/tests/clinical_pull.rs` cannot hide
- DB-gated tests take `db::test_serial_guard(&base)` **before** `connect_and_load_schema`
- UUID parameters bind as text and cast in SQL (`$1::text::uuid`); `with-uuid-1` is deliberately off

## 10. Risks

| Risk | Mitigation |
|---|---|
| The precedence rule converts ~83 fixture call sites and requires retiring `patient.created` | Measured, not estimated (§2.3), and split into its own PR for that reason. Those call sites *are* the bypass the funnel exists to close; each becomes an explicit registration, and any that resists is a finding, not a nuisance |
| Search latency on a large node makes the funnel slower than paper | **Accepted, NOT mitigated — corrected after implementation.** The original entry read "the blocking passes are index-backed and the oversized-block guard reports rather than scans"; **neither half is true of what shipped.** There is no index on `patient_identifier(system, match_key)` (the PK `(patient_id, system, match_key)` has the wrong leading column), none on `patient_demographic(field, value)` (PK is `(patient_id, field)`), and pass 3 is a per-row `regexp_split_to_table` over the whole of `patient_name` with no usable expression index. There is no guard, no ceiling and no reporting path. What actually holds the risk down today is node size, not design. Tracked as [#357](https://github.com/cairn-ehr/cairn-ehr/issues/357); the §7 budget remains falsifiable and owed |
| The attestation's third-party UUIDs are a disclosure surface | Recorded in §2.5 and in ADR-0061 as a rung-2 erasure obligation; not silently accepted |
| Re-expressing John Doe regresses a shipped, tested subsystem | Its suite runs unchanged as a regression gate; the new event joins the existing transaction rather than adding one |
| A future reader "fixes" the missing authorship requirement by gating it (§2.6) | §2.6 records the three failure scenarios and ADR-0061 carries it as a rejected alternative; a test asserts an unattested standard registration **succeeds**, so the gate cannot be added silently |
