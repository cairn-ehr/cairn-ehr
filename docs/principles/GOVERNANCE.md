# Governance & Contributing

*A principles-level document. It states how Cairn is governed, how decisions are made and
recorded, and how to contribute. Like the rest of `docs/principles/`, it carries high authority:
when it conflicts with working notes it wins, and when it conflicts with the mission, the mission
wins.*

**Status:** Architecture / specification phase. The project is small and early; this document
describes governance proportionate to that stage and the path it will grow along, deliberately
avoiding premature scaffolding. It evolves by overlay (see [Amending this document](#amending-this-document)).

---

## 1. The mission is the tie-breaker

Cairn exists to be a **free, offline-resilient, vendor-independent health record built as a public
good** — one that keeps working at 3 a.m. when the network is down, runs on hardware a clinic
already owns, and belongs to no vendor. That mission, stated in the root `README` and in
[spec/index.md](../spec/index.md), is the **supreme governing rule of the project**.

Every decision — technical, organizational, or commercial — is measured against it. When
convenience conflicts with the mission (open standards, no proprietary dependency, no mandatory
cloud, data sovereignty, patient safety), **the mission wins**. The project is explicitly
**anti-capture and anti-vendor-lock-in**, rooted in hard experience of public health-IT efforts
collapsing under conflicting commercial interests. We hold the mission above convenience precisely
because no one else in the room is incentivized to.

This is not hostility to commercial activity — reaching the clinics and ministries Cairn serves
will often require paid hosting, deployment, integration, training, and support, and people doing
that work in good faith are part of how the mission succeeds. The line the project draws is about
**capture, not commerce** (see [§5](#5-stewardship-of-the-name)).

## 2. What governs what (the authority hierarchy)

Cairn's reasoning is kept legible on purpose; the document hierarchy is itself a governance
mechanism. From highest authority down:

1. **`docs/principles/`** — canonical statements of mission and governance (this document and
   [Stewardship of the Name](STEWARDSHIP-OF-THE-NAME.md)). The mission also lives in the root
   `README` and in [spec/index.md](../spec/index.md).
2. **`docs/spec/`** — the canonical architecture specification, one file per aspect.
   - **`docs/spec/decisions/`** — the **ADR log**, the home of *why*. ADRs are numbered, dated, and
     **immutable**: a reversal is a new superseding ADR, never an edit — the project's own *"never
     erase, always overlay"* applied to its own documentation.
3. **`docs/HANDOVER.md`** — disposable working scaffolding, **not** a source of truth; regenerated
   each working session. If it disagrees with the canonical docs, the canonical docs win.

The **twelve founding principles** ([spec/index.md](../spec/index.md)) are the lens every design
choice is checked against — the first four (append-only + causal ordering; identity is a claim,
never a fact; paper-parity; acknowledged uncertainty) before anything else.

## 3. How decisions are made

**Decisions are made by reasoned argument against the founding principles, and the load-bearing
ones are recorded as ADRs.** The discipline, not the org chart, is what makes the project
trustworthy at this stage.

- **Anything load-bearing gets an ADR.** A decision that constrains the data model, the identity
  algebra, the sync semantics, security posture, or the project's governance is captured as an ADR
  (context → decision → consequences). This is what lets a future contributor understand *why*
  before reopening a question — and **settled questions are not reopened without reading the ADR
  and bringing a new reason.** A genuine reversal is written as a *new* superseding ADR.
- **Clinical realism decides ties between technical options.** The most productive design mode so
  far is **case-mining**: testing whether the existing primitives absorb a real front-line failure
  mode. A well-described clinical failure mode is first-class evidence (see [§7](#7-contributing)).
- **The language/substrate rule is governance, not just engineering.** Components are chosen by
  *defect blast radius* ([spec §9](../spec/language-substrate.md)): safety-critical logic (a defect
  can silently corrupt the record, mis-merge patients, leak data, or crash an unattended node) is
  built in **Rust or in-database**, optimized for reviewer-legibility and kept as small as
  possible; fit-for-purpose logic optimizes for iteration speed. The smallest, most-reviewable
  surface carries the most safety weight.

**Stewardship today, broader governance as it matures.** The project is currently founder-led and
held in trust on behalf of its community (see [§5](#5-stewardship-of-the-name) and
[§6](#6-the-stewarding-entity-intended)). As contributors and adopters accumulate, governance is
intended to broaden — maintainers with review authority over defined areas, a technical steering
group for cross-cutting decisions, and ultimately the stewarding non-profit. The commitments in
this document (the mission as tie-breaker, the licensing terms, and name-stewardship) are designed
to bind whatever structure emerges, including the steward itself.

## 4. Licensing and contributor terms

- **AGPL-3.0 end to end — non-negotiable.** The whole project is under the
  [GNU Affero General Public License v3.0](../../LICENSE), and **every component and dependency must
  be AGPL-3.0-compatible**. This guarantees the system — and any networked service built on it —
  stays free for everyone who depends on it.
- **Inbound = outbound.** Contributions are accepted **under the same AGPL-3.0** the project
  carries. By contributing, you license your contribution under AGPL-3.0.
- **Developer Certificate of Origin (DCO), not a CLA.** Contributors certify origin with a
  `Signed-off-by` line (the standard [DCO](https://developercertificate.org/)); commits are signed
  off with `git commit -s`. The project **deliberately does not use a Contributor License
  Agreement.** A CLA that assigns rights or grants relicensing power to a single entity is exactly
  the **capture surface** this project exists to guard against: it would let a future steward
  proprietize the commons the community built. Refusing it keeps the copyleft strong and keeps no
  one — including the steward — able to take the project private. This is a mission commitment, not
  a mere process choice.

## 5. Stewardship of the name

The companion principles document **[Stewardship of the Name](STEWARDSHIP-OF-THE-NAME.md) is part
of this governance** and is authoritative on the subject. In brief:

> **The name belongs to the mission, not to any entity.** The AGPL protects the software's freedom;
> the *name* is the asset the license alone does not protect, so it is protected deliberately and
> separately. **Anyone may build on Cairn and earn a living doing so; no one may *be* Cairn except
> the community that stewards it.**

The durable threat to a copyleft project is rarely a hostile fork (the AGPL makes forks weak —
anyone can fork back); it is **capture of the name** — an entity wearing the project's identity to
convert community trust into private advantage. Offering paid hosting/deployment/support *of* Cairn,
described truthfully, is welcomed; presenting any product or company *as* Cairn ("Cairn EHR Pro,"
"Cairn Inc.") is not. The steward defends the name *for* the mission and is itself bound by this
principle — it may not usurp the name any more than an outsider may.

## 6. The stewarding entity (intended)

The name and project assets are to be held by a **stewarding non-profit** (a foundation or
equivalent), established as the project matures; until it exists, they are held in trust by the
founder(s) on behalf of the community and under the same principles. The steward's role is
**custodial, not proprietary**.

Two items are **deliberately parked** until the project has enough momentum and a clearer
adoption/funding geography — recorded here so they are not forgotten, and not to be re-litigated
without new reason:

- **The stewarding entity and its jurisdiction** (options floated include a German
  Stiftung / gemeinnütziger Verein, a US 501(c)(3), or an umbrella such as the Software Freedom
  Conservancy or a health-specific foundation).
- **A formal trademark / wordmark registration** for "Cairn," held by the stewarding entity, as the
  instrument that gives the name-stewardship principle legal force. The principle is recorded now;
  the legal instrument follows when there is enough substance to be worth protecting.

## 7. Contributing

This project is for the people who have to use these systems and the people who keep them running —
**clinicians, health-IT engineers, and anyone who has been failed by an EHR and believes it could
be otherwise.** Clinical realism is valued here as highly as code.

### 7.1 Clinical case-mining is a first-class contribution

**A well-described failure mode from the front line is a genuine contribution** — often a more
valuable one than a patch. The single most productive design activity has been testing whether the
architecture's primitives absorb a real clinical failure mode; many founding decisions came
directly out of such cases (the nightly-imaging-sync that froze a remote hospital; the subpoena that
should never have disclosed a record; the relocated patient whose home naming conventions must
travel with them).

A good case includes: **the workflow and its setting**, **its paper-era counterpart**, **exactly
where it breaks** (in time, steps, cognitive load, or safety), and **what the honest outcome should
be**. Open an issue describing it — no code required. The right answer is frequently that the
existing primitives already absorb it, which is itself a useful result; sometimes it surfaces a new
ADR.

### 7.2 Every change is checked against the principles

Any proposed change — to the spec or, later, to code — is evaluated against the
[founding principles](../spec/index.md), the first four before anything else, and against
**paper-parity** as the governing law: name the paper-era counterpart and show the workflow is not
slower, harder, more cognitively demanding, or impossible than it. **Confirmation dialogs are not an
acceptable safety mechanism** — they fail paper-parity; restore the physical affordance instead.

### 7.3 Contributing to the specification

The spec is the current product, so most contribution today is design work.

- **Source is Markdown; HTML is generated, never hand-edited.** Author callouts in GitHub/Obsidian
  syntax (`> [!NOTE]`) so they render on GitHub *and* as Material admonitions. The site builds with
  `uv run --with mkdocs-material --with mkdocs-callouts --with mkdocs-redirects -- mkdocs build`
  (config `mkdocs.yml`); the generated `site/` is never committed.
- **No in-file changelogs and no version-suffixed filenames** — git is the line history; the spec
  version lives in [spec/index.md](../spec/index.md). Each aspect file keeps its section numbering
  so cross-references stay valid.
- **Load-bearing decisions need an ADR**, allocated in order from the
  [decision log](../spec/decisions/README.md); reversals supersede, never edit. **Read the relevant
  ADR before reopening a settled question.**

### 7.4 Contributing code (when implementation begins)

There is **no code, build system, or tests yet** — implementation has not started. When it does:

- **Match the component to its defect blast radius** ([§3](#3-how-decisions-are-made),
  [spec §9](../spec/language-substrate.md)). Safety-critical code is Rust or in-database, written
  above all to be *reviewer-legible*, and kept small; fit-for-purpose code optimizes for iteration.
- **The integration boundary is the database boundary.** Each component talks to its node's
  PostgreSQL (≥ 18); avoid FFI coupling.
- **AGPL-3.0-compatible dependencies only**, no exceptions.

### 7.5 Mechanics

- Develop on a feature branch; open a pull request against the default branch with a clear
  rationale tied to the principles.
- **Sign off every commit** (`git commit -s`) per the [DCO](#4-licensing-and-contributor-terms).
- Expect review focused on principle-alignment, clinical realism, and (for safety-critical
  surfaces) reviewer-legibility. Small, well-reasoned changes are easier to accept than large ones.

## 8. Code of conduct

Cairn is built by people who care about patients and about each other's time and judgment. The
standard is simple and non-negotiable: **engage in good faith, treat contributors and their
front-line experience with respect, assume competence, and keep the shared goal — patient safety —
above ego.** Harassment, dismissiveness, and bad-faith conduct have no place here.

A formal **Contributor Covenant** code of conduct will be adopted as the community grows; until
then this statement governs, and a maintainer may act on it.

## 9. Responsible disclosure (security)

Cairn is a health record: defects can affect patient safety and confidentiality. **Report a
suspected security vulnerability privately to the maintainers — do not open a public issue for it.**
Until a dedicated security contact and `SECURITY.md` are published, report to the project
maintainer ([horst.herb@gmail.com](mailto:horst.herb@gmail.com)) with enough detail to reproduce.
Good-faith disclosure will be acknowledged and credited; we will not pursue researchers acting in
good faith. (A formal policy and contact will follow as implementation begins.)

## Amending this document

Governance evolves by **overlay**, like everything else in Cairn. Changes are made by pull request
with a stated rationale; substantial changes are themselves part of the project's recorded history
(git, and an ADR where the change is load-bearing). The **entrenched commitments** — the mission as
tie-breaker, AGPL-3.0 with no CLA, and the stewardship of the name — are intended to be the hardest
to change and may not be amended in a way that enables capture; they bind future contributors and
the stewarding entity alike. *The commitments are recorded now so they cannot quietly drift later.*
