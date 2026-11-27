# ADR-0113 — How an identifier's `system` is spelled

- **Status:** Accepted
- **Date:** 2026-11-27
- **Spec version at acceptance:** 0.115
- **Refines:** [ADR-0033](0033-patient-identifier-representation.md) (the `system` facet's spelling; its
  decision is otherwise unchanged, and it is not edited)
- **Issues:** [#1203](https://github.com/cairn-ehr/cairn-ehr/issues/1203) (enforcement, a pick list of known
  systems, facility-scoped record numbers, subdivisions: all still open)

## Context

ADR-0033 made an identifier's `system` the namespace the hard veto keys on: *same `system`, different
`normalized` value* forces a human decision ([identity §5.2](../identity.md#52-matching-pipeline-safety-asymmetric-false-merge-worse-than-false-split)).
It said what a `system` is, but not how one is spelled. The node compares it as exact, case-sensitive
text, and the in-DB floor checks only that it is present and non-empty (db/010).

So two spellings of one scheme are two systems to the node, and the failure is silent in both
directions. A registrar who records `medicare-au` and another who records `Medicare` for two *different*
people have produced two different Medicare numbers that the veto never compares. A clerk who searches
`medicare-au=…` does not find a chart whose number was recorded as `au-medicare`. Before this ADR the
tree itself spelled Medicare four ways: `medicare-au` (the spec and ADR-0033), `au-medicare` (the matcher
generator and most fixtures), `Medicare` and `medicare` (tests).

## Decision

Decided by the maintainer, 2026-11-27.

1. **A `system` is lowercase.** `mrn`, never `MRN`. The same holds for every term of that kind (the
   `use`/`type` vocabulary already was).
2. **A scheme issued in a region ends in `-` and that region's ISO 3166-1 alpha-2 code, lowercase.** The
   scheme name comes first and the region last: `medicare-au`, `medicare-us`, `ihi-au`, `dva-au`,
   `nhi-nz`, `nhs-number-gb`, `kennitala-is`. The suffix is always written, even where only one country
   issues the scheme, so a reader never has to know which schemes are unique.
3. ADR-0033's example `nhs-number` reads as `nhs-number-gb` from now. Its `medicare-au` already conforms.
4. The explicit sentinel `unknown` and placeholder systems that name no real scheme (`national-id`,
   `mrn-local` in the matcher's synthetic generator) keep their form. They are lowercase and claim no
   region.
5. Every example, fixture and test in the tree follows this convention. Nothing in the node's behaviour
   changes: the floor and the comparisons are as they were.

## Consequences

- **Easier:** one spelling to copy, so examples stop seeding drift; a reader can tell a scheme's issuing
  country from its name.
- **Harder / the bet:** this is a convention, and nothing checks it yet. The CLI still takes whatever
  `system` is typed. We bet that the GUI's identifier field, when it is built, offers known systems as a
  pick list so no human types a namespace (#1203, item 2), and that enforcement is decided before real
  identifiers are recorded (#1203, item 1).
- **Not settled:** a bare `mrn` is not globally meaningful, because a facility issues it and not a region.
  Two hospitals' `mrn` numbers are the same system to the node, so the veto fires against what may be a
  true match. The facility-scoped form is open (#1203, item 3). So is a finer region for state-issued
  schemes, for which ISO 3166-2 lowercased (`-au-qld`) is the obvious extension (#1203, item 4).
- **How we'd know it fails:** a review worklist that holds two charts apart only because their systems
  differ in case or word order, or a search miss traced to a spelling.
