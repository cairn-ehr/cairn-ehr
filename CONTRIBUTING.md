# Contributing to Cairn

The full contribution guide and project governance live in one place:

### → [docs/principles/GOVERNANCE.md](docs/principles/GOVERNANCE.md)

A few essentials up front:

- **Clinical realism is a first-class contribution.** A well-described front-line failure mode — the
  workflow, its paper-era counterpart, exactly where it breaks, and the honest outcome it should have
  — is a genuine contribution, no code required. Open an issue.
- **The project is in its architecture / specification phase.** There is no code, build system, or
  tests yet; most contribution today is design work on the Markdown spec under `docs/spec/`. Load-bearing
  decisions are recorded as immutable [ADRs](docs/spec/decisions/README.md) — read the relevant one
  before reopening a settled question.
- **AGPL-3.0, inbound = outbound, DCO not CLA.** Contributions are under the
  [AGPL-3.0](LICENSE); sign off every commit (`git commit -s`) per the
  [Developer Certificate of Origin](https://developercertificate.org/). The project deliberately uses
  **no CLA** — keeping the copyleft strong and the project uncapturable.
- **The mission is the tie-breaker**, and **paper-parity is the governing law**: no clinical workflow
  may be slower, harder, more cognitively demanding, or impossible than its paper equivalent.

See [GOVERNANCE.md](docs/principles/GOVERNANCE.md) for the rest — how decisions are made, the
defect-blast-radius rule for code, stewardship of the name, the code of conduct, and responsible
disclosure.
