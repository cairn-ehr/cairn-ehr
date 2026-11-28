# The demographics door, slice 1: sex, gender, address and a name's use from the CLI (#1211)

**Status:** built 2026-11-28. This plan was written with the code, not before it: the slice is one
command over events and projections that already existed, and there was no design to choose between.
What it leaves open is at the end.

## The gap

`demographic.field.asserted` has carried `administrative-sex`, `sex-at-birth`, `gender-identity`,
`address` and a name `use` since db/011 to db/014. No command wrote them:

- `patient-register` (and the window's funnel) records a name, a date of birth and identifiers.
- `assert-observed-evidence --sex` records `administrative-sex` as `clinician-observed` only: it can
  say a clinician looked, and cannot say what the patient states or what a referral says.
- `address_assertion_body` had no caller outside tests.

Found while loading a SyntheticHealthData cohort (#1214): every one of 300 patients has a sex and an
address, and all 300 were left in the file.

## What was built

- `crates/cairn-node/src/patient/demographic.rs`
  - `DemographicFact`: `Name`, `AdministrativeSex`, `SexAtBirth`, `GenderIdentity`, `Address`.
  - `facts_from(Stated)`: the command line's options as facts; a use with no value is refused.
  - `refusal(facts, provenance)`: an empty set, a missing provenance or an empty value, each by name.
  - `build_demographic_event(...)`: pure; payload and legibility twin from the existing
    `cairn-event` builders.
  - `assert_demographics(...)`: one transaction through `submit_event`; returns the event ids.
- `cairn-node patient-demographics <chart> --provenance P [--name N --name-use U]
  [--administrative-sex V] [--sex-at-birth V] [--gender-identity V] [--address LINE --address-use U]`.
- `scripts/import_synthetic_cohort.py` records each cohort patient's sex and address with it, as
  `imported`, once.

Decisions taken, each reversible by a later slice because every one is an additive event:

1. **Provenance is required and has no default.** `patient-stated` would be untrue for an import, and
   `unknown` would discard what a registrar at the desk knows. The ladder is db/011's.
2. **An address is its display line.** ADR-0032 puts structured parts under a locale profile, and no
   profile exists. `display` is the mandatory, legible part; a profile can add parts later without
   changing these events.
3. **Nothing is validated against a list.** Sex and gender are open text (principle 4).
4. **The chart must exist, and the floor says so.** No check of our own: db/005 step 8b refuses the
   first event on a chart unless it is the registration.
5. **Device-additive.** The node signs and is the `recorded` contributor, as `assert-observed-evidence`.

## Tests

- Unit (no database), in the module: each fact goes to its field with a twin; an address claims no
  structure; the refusals; the success lines; the options-to-facts mapping.
- `crates/cairn-node/tests/patient_demographics_door.rs` (DB-gated): sex and address land in one act;
  what the patient states outranks a later clinician observation; gender, sex at birth and a named
  use are recordable; an unregistered chart is refused and nothing is written.
- `scripts/tests/import_synthetic_cohort_test.py`: recorded as `imported`, once; a chart missing only
  its address gets only the address; a failed write is counted and retried.
- Run against a PG 18 node with the 300-patient seed-1 cohort: 300 sex-and-address records written,
  a second run wrote none.

## Paper-parity benchmark (§1.2)

- **Paper counterpart:** the front sheet of a paper record: the registrar writes the address on one
  line and ticks or writes the sex.
- **Steps:**
  - Record sex and address: paper N = 2 written lines → architecture M = 1 act (one command, two
    events in one transaction) → UI K = 2 fields + 1 save, owed by the window slice.
  - Stating where the fact came from: N = 0 on paper (the hand and the date imply it) → M = 1 word
    (`--provenance`). This is one more thing to state than paper asks, once per act and not per fact.
    It is what lets a patient's own statement stand against a later guess, so it is kept; a window
    should pre-fill it from the context (a registrar at the desk is `patient-stated` unless they say
    otherwise) so that K = 0 extra. Filed as an open point below, not as `M > N`: the architecture
    forces one act either way.
  - Changing an address: N = 1 (strike and rewrite) → M = 1 (record the new one; nothing is struck).
- **Time + cognitive load:** ≤ 15 s to record a sex and a one-line address from the window, with the
  chart's current sex and address readable before and after without a click. Not measurable yet: this
  slice exposes a command line for operators and importers, and the measurement is owed by the slice
  that puts these fields in the window.

## Left open

- **The window has no fields for these.** The GUI milestone lists "enter demographics"; this slice
  does not deliver it.
- **No read verb.** Nothing on the CLI prints a chart's sex or address, so the importer reads the
  projections with `psql` to avoid recording twice.
- **No Australian address profile**, so no structured parts (street, suburb, state, postcode).
- **An ended address.** There is no way to say "no longer lives here" other than recording the new
  one; db/014 keeps one address per use.
- **Human authorship.** A registrar's own key (`--author-as`, ADR-0053) is not offered here.
