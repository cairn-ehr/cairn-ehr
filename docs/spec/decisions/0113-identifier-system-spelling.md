# ADR-0113 — How an identifier's `system` is spelled

- **Status:** Accepted
- **Date:** 2026-11-27
- **Spec version at acceptance:** 0.115
- **Refines:** [ADR-0033](0033-patient-identifier-representation.md) (the `system` facet's spelling; its
  decision is otherwise unchanged, and it is not edited)
- **Issues:** [#1203](https://github.com/cairn-ehr/cairn-ehr/issues/1203) (enforcement, a pick list of known
  systems, facility-scoped record numbers, subdivisions: all still open); PR
  [#1204](https://github.com/cairn-ehr/cairn-ehr/pull/1204)

## Context

ADR-0033 made an identifier's `system` the namespace the hard veto keys on: *same `system`, different
`normalized` value* forces a human decision
([identity §5.2](../identity.md#52-matching-pipeline-safety-asymmetric-false-merge-worse-than-false-split)).
It said what a `system` is, but not how one is spelled. The node compares it as exact, case-sensitive
text: the veto pairs systems by `INTERSECT` and `=` (db/016), and search pass 1 by `=` (db/046). The
in-DB floor checks only that it is present and non-empty (db/010).

So two spellings of one scheme are two systems to the node, and the failure is silent in both
directions. A registrar who records `medicare-au` and another who records `Medicare` for two *different*
people have produced two different Medicare numbers that the veto never compares. A clerk who searches
`medicare-au=…` does not find a chart whose number was recorded as `au-medicare`. Before this ADR the
tree itself spelled Medicare four ways: `medicare-au` (the spec and ADR-0033), `au-medicare` (the matcher
generator and most fixtures), `Medicare` and `medicare` (tests).

The `unknown` sentinel is the sharpest case. db/016 excludes only the literal `'unknown'` from the veto,
so a number recorded under `Unknown` takes part in it, which the spec says an unrecognised number must
never do.

## Decision

Decided by the maintainer, 2026-11-27.

1. **A `system` is lowercase.** `mrn`, never `MRN`. The same holds for every term of that kind (the
   `use`/`type` vocabulary already was).
2. **A scheme issued in a region ends in `-` and that region's ISO 3166-1 alpha-2 code, lowercase.** The
   scheme name comes first and the region last: `medicare-au`, `medicare-us`, `ihi-au`, `dva-au`,
   `nhi-nz`, `nhs-number-gb`, `kennitala-is`. The suffix is always written, even where only one country
   issues the scheme, so a reader never has to know which schemes are unique. The code is the country's
   even where the scheme covers only part of it (the NHS number is not issued in Scotland or Northern
   Ireland); a finer region is #1203, item 4.
3. ADR-0033's example `nhs-number` reads as `nhs-number-gb` from now, and ADR-0035's `medicare-provider`
   as `medicare-provider-au`. ADR-0033's `medicare-au` already conforms.
4. The explicit sentinel `unknown` and placeholder systems that name no real scheme (`national-id`,
   `mrn-local` in the matcher's synthetic generator) keep their form. They are lowercase and claim no
   region. Facility-local placeholders are still spelled several ways in tests (`local-mrn`,
   `mrn:hospital-a`, `urn:example:mrn`); they wait for the facility-scoped form (#1203, item 3).
5. The live spec, code, fixtures and tests follow this convention. Accepted ADRs and dated plans keep
   their spelling as history. Test fixtures' `profile` tags were renamed with their systems for
   readability; how a profile's namespace is spelled is not decided here. Nothing in the node's
   behaviour changes: the floor and the comparisons are as they were.

## Consequences

- **Easier:** one spelling to copy, so examples stop seeding drift; a reader can tell a scheme's issuing
  country from its name.
- **Harder / the bet:** this is a convention, and nothing checks it yet. The CLI still takes whatever
  `system` is typed. We bet that the GUI's identifier field, when it is built, offers known systems as a
  pick list so no human types a namespace (#1203, item 2), and that enforcement is decided before real
  identifiers are recorded (#1203, item 1).
- **Not settled:** a bare `mrn` is not globally meaningful, because a facility issues it and not a region.
  Two hospitals' `mrn` numbers are the same system to the node, so the veto (or, without a `normalized`
  value, its held-for-review form) fires against what may be a true match. The facility-scoped form is
  open (#1203, item 3). So is a finer region for state-issued schemes, for which ISO 3166-2 lowercased
  (`-au-qld`) is the obvious extension (#1203, item 4).
- **How we'd know it fails:** a spelling split is silent, so it shows up only afterwards: two charts
  linked although their numbers differ, because each recorded one scheme under a different spelling and
  the veto never compared them; a duplicate found later whose charts share one number under two
  spellings; or a search miss traced to a spelling.
