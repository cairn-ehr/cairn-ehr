# Design — the §5.3/§5.8 search-before-create funnel (node tier)

**Date:** 2026-08-04
**Status:** approved in brainstorming, ready for a plan
**Layer:** in-DB enforcement floor + node library tier + CLI (no UI this slice)
**Discharges:** [#344](https://github.com/cairn-ehr/cairn-ehr/issues/344)
**Governing:** [§5.3](../../spec/identity.md#53-registration-classes) (registration classes) ·
[§5.8](../../spec/identity.md#58-registration--documentation-workflow-normative) (the normative funnel) ·
[§5.4](../../spec/identity.md#54-unidentified-registration-john-doe-baked-into-the-root) (John Doe) ·
[ADR-0014](../../spec/decisions/0014-locale-pluggable-matcher-comparators.md) (advisory matcher; the hub sweep is the false-split backstop) ·
[ADR-0022](../../spec/decisions/0022-validated-submit-surface-the-write-path.md) (the validated write door) ·
[ADR-0048](../../spec/decisions/0048-twin-check-registry-dispatch.md) (twin-check registry) ·
[ADR-0051](../../spec/decisions/0051-contributor-role-vocabulary-floor-and-responsibility-wire-shape.md) (strict-submit / lenient-apply) ·
[ADR-0058](../../spec/decisions/0058-grade-gated-teffective-ceiling.md) (a remote door admits and flags, never rejects) ·
[ADR-0053](../../spec/decisions/0053-per-write-human-authorship.md) (per-write human authorship) ·
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

**In scope:** the registration event type and its floor check, the precedence rule, the human-author
binding on standard registration, the candidate search, the pure read/attestation model, the
`patient_registration` projection, two CLI verbs, and re-expressing John Doe onto the same act.

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

### 2.3 Strict local submit, lenient remote apply

The precedence rule is enforced at `submit_event` and **not** at `apply_remote_event`.

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

### 2.6 A standard registration binds a human author

An attestation whose purpose is the six-months-later investigation must answer **who was looking**,
not only *what was on screen*. The demographic plane submits through the plain one-argument door
(the node records; no human is bound —
[ADR-0053](../../spec/decisions/0053-per-write-human-authorship.md)'s `cairn_authorship_bound` is
scoped to sealed clinical bodies), so as first designed this event would have carried half a forensic
record. `class=standard` therefore requires a bound human author, reusing the ADR-0053 machinery the
med-list window already uses (`enroll-human`, a sealed personal key, the session unlock).

**The other two classes do not.** Putting an authentication step in front of registering an
unconscious patient is precisely where paper-parity forbids friction (§5.4: *care proceeds without
delay*); those stay node-recorded, and the identification event that later resolves them is already
human-attested (§5.7, `identify` — human, method recorded).

**This costs zero human acts** and so does not disturb §7's `M = N`. §5.11 is explicit that
gatekeeping is coarse and rare while attribution is per-write and paper-cheap: the registrar unlocks
once at the start of a shift and every registration after that is free. If a future measurement shows
otherwise, that is a §7 finding.

The registrar enrolls as their own role-actor (`enroll-human --role registrar`), which is the
(entity, role) model working as designed — one person may hold both a clinician and a registrar
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
| `class='standard'` ⇒ a bound human author (§2.6) | a standard registration names its registrar |

The check is pure (`p_type`, `b`) → void, matching the ADR-0048 unified signature, so registration in
the dispatcher is one row and the dispatcher itself is untouched.

### 4.2 Precedence predicate

```sql
cairn_patient_has_events(p_patient_id uuid) RETURNS boolean   -- pure, one indexed lookup
```

`submit_event` (db/005) refuses a non-registration event whose `patient_id` has no prior event.
`apply_remote_event` (db/020) does not call it at all (§2.3).

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

1. **shared identifier** — `patient_identifier` on `(system, normalized value)`
2. **exact DOB** — `patient_demographic` on the birth-date field
3. **shared name token** — `patient_name` token overlap

Union, dedup, no scoring, no ranking, no ceiling on the result set beyond an oversized-block guard
that **reports rather than silently caps** (the existing matcher discipline, and the source of the
`incomplete` flag in §3).

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
| `crates/cairn-node/src/patient/register.rs` | mint UUID → build body from the shown `CandidateList` → sign as the registrar (§2.6) → submit, one transaction | A chart is never half-registered (the John Doe precedent) |
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
- **No unregistered-chart UI flag** (§2.3) — queryable, not surfaced. Issue to file.
- **No matcher convergence on the blocking keys.** `cairn_search_candidates` and
  `matcher/pipeline/db.py` will each extract identifier / DOB / name-token keys. They are not the
  same query (the sweep blocks all × all; the funnel maps query → set), so the shared part is the key
  extraction, not the driver. Issue to file rather than let the drift pass silently (house rule 5).

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
- `class=standard` with no bound human author is refused; with one, it succeeds (§2.6)
- `class=unidentified` with **no** human author still succeeds — the paper-parity carve-out is
  tested, not assumed
- **precedence:** a bare name assertion on a fresh `patient_id` is refused; the same assertion after
  a registration succeeds
- **the load-bearing lenient case:** `apply_remote_event` admits an out-of-order clinical event whose
  patient has no registration — no wedge, no pen

**Search, DB-gated:** each blocking pass finds its candidate; an identity-pending (John Doe) chart is
returned **with** trust state `unconfirmed`; a candidate whose demographics cannot be read is
reported through `incomplete`, never silently dropped; the oversized-block guard reports.

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
| The precedence rule breaks an existing test that mints a patient by asserting demographics directly | Expected and wanted — those call sites are the bypass the funnel exists to close. Each becomes an explicit registration in the fixture; if any resists, that is a finding, not a nuisance |
| Search latency on a large node makes the funnel slower than paper | The blocking passes are index-backed and the oversized-block guard reports rather than scans; the §7 budget is falsifiable and measured, not assumed |
| The attestation's third-party UUIDs are a disclosure surface | Recorded in §2.5 and in ADR-0061 as a rung-2 erasure obligation; not silently accepted |
| Re-expressing John Doe regresses a shipped, tested subsystem | Its suite runs unchanged as a regression gate; the new event joins the existing transaction rather than adding one |
| Extending ADR-0053's authorship binding off the sealed clinical path (§2.6) touches `db/005`, a load-bearing door | The binding is *added* for one event type, not generalised: the existing clinical rule is untouched, and both the required (`standard`) and not-required (`unidentified`) branches are tested. If the door resists a clean additive change, that is a finding to raise before forcing it |
