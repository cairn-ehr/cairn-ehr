# Cairn

> **The grid goes down. The chart stays up.**

Offline-first, vendor-independent electronic health record. Keeps working through any network
outage, runs anywhere from a Raspberry Pi to a hospital cluster, and belongs to no vendor.

**License:** AGPL-3.0 · **Status:** Spec complete; proving viability via spikes · **Database:** PostgreSQL ≥ 18

*The name:* a cairn is a hand-built stack of stones that marks the safe path — needing no power,
no network, no infrastructure, standing alone in the wilderness and still doing its job. Cairns
are built by accretion, each traveler adding a permanent stone; they are decentralized, raised by
many hands across a landscape; and they are found in nearly every culture on earth. So is this
system meant to be. *(Read as a backronym if you like: **C**linical, **A**ppend-only,
**I**nteroperable, **R**esilient **N**etwork.)*

---

## Why this exists

Most clinicians have, at some point, watched a computerized health record make their day
slower, their workflow clumsier, or their patients less safe than the paper chart it
replaced — and many have watched promising public efforts collapse under conflicting
commercial interests, where lock-in was the business model and interoperability the thing
quietly sabotaged.

Cairn starts from a different place. It has **no vendor in the room**. There is no
revenue that depends on trapping your data, no proprietary layer you must license, no cloud
you are required to trust. Because nothing here is incentivized to keep the hard problems
hard, we are free to let one thing — and only one thing — drive every design decision:

**What actually happens at the point of care, including at 3 a.m. when the network is down.**

---

## The mission

Build a health record system that:

- **Keeps working through any outage.** Loss of internet, loss of the hospital intranet, or a
  single isolated computer — care continues. The clinician can always read the locally
  relevant record and write new clinical data. Synchronization catches up when connectivity
  returns.
- **Runs anywhere, for anyone.** The same software must serve a solar-powered clinic on an
  intermittent mobile connection in a low-resource setting *and* a tertiary hospital in a
  wealthy country. One codebase, scaled by configuration — from a single workstation to a
  national deployment.
- **Belongs to no one but its users.** Fully open source under AGPL-3.0, built only on
  commodity hardware and open standards, with no proprietary dependency and no vendor lock-in
  at any layer.
- **Respects the clinician's time and judgment.** It is held to a strict standard: no workflow
  may be slower, harder, or more error-prone than its paper equivalent.

---

## The eventual goal

A genuinely free, genuinely portable electronic health record that any health system in the
world — from a one-room rural practice to a national network — can adopt, run, inspect, and
adapt without asking anyone's permission and without surrendering control of its data. An
EHR that earns clinicians' trust by being *available*, *honest*, and *fast*, and that treats
patient safety and data sovereignty as architectural guarantees rather than marketing claims.

We would consider Cairn a success when a clinic anywhere can stand the system up on
hardware it already owns, keep caring for patients through a week-long internet outage, and
never once wish it still had the paper charts.

---

## Founding principles

These are the load-bearing commitments. Everything in the architecture is downstream of them.

### 1. Availability over consistency
During a network partition a clinician must always be able to work. We deliberately accept
eventual consistency and design the data model so that this is *clinically safe* — not merely
technically tolerable.

### 2. Paper-parity (the governing law)
No clinical workflow may be slower, more difficult, more cognitively demanding, or impossible
compared to its paper-record equivalent. Every workflow must name its paper-era counterpart
and be benchmarked against it in time, steps, and cognitive load. A workflow that loses to
paper is a defect. *(The only exclusions are capabilities of paper that constitute malfeasance
— silent falsification, untraceable backdating — which we intentionally do not reproduce.)*
This is the floor, not the ceiling: where digital can clearly beat paper, we take the win.

### 3. The clinical record is append-only
All clinical content is written as immutable, signed events. Corrections are new events that
reference the originals — exactly as medico-legal documentation already works on paper. This
makes synchronization between divergent copies a safe set-union operation rather than a
dangerous merge, and it makes the record honest by construction.

### 4. Patient identity is a claim, never a fact
Patients arrive unidentified, misidentified, and sometimes deliberately under a false name.
Prevention can never be complete, so the system treats identity as something under continuous
evaluation and makes *repair* a first-class, fast, auditable operation:
**never merge — always link; never erase — always overlay.** Every identity error, accidental
or deliberate, is correctable without data loss and with a full audit trail.

### 5. Acknowledged uncertainty (an imprecise near-truth over a precise untruth)
Clinicians are routinely forced by software to enter data they cannot be sure of — a guessed date
of birth, a mandatory field with no honest answer — and the record fills with confident falsehoods
that are then trusted downstream. Cairn refuses this trade: an imprecise near-truth always beats a
precise untruth. Uncertainty, imprecision, ranges, and an explicit *unknown* (distinct from
*not-yet-asked* and from *refused*) are first-class, recordable values; no required field may be
satisfiable only by fabrication; and certainty is refined over time by overlay, never forced up
front. This keeps the record honest and never feeds the identity matcher a confident falsehood.

### 6. One system, every scale (fractal topology)
The same software runs at every tier — workstation, department, facility, region, nation. A
node's role is configuration, not a different product. This is what lets the architecture
serve both the rural clinic and the tertiary hospital without forking.

### 7. Vendor independence is non-negotiable
AGPL-3.0 throughout. Open standards (e.g. FHIR as the interoperability interface). Commodity
x86/ARM hardware, standard Linux, PostgreSQL. No proprietary services, no mandatory cloud, no
license keys. If any part of this system ever requires asking a company's permission, we have
failed.

### 8. Safety-critical logic is built to be unbreakable and auditable
Components whose defects could corrupt the record, mis-merge patients, or leak data are
implemented where whole classes of error become *unrepresentable* — in memory-safe,
strictly-typed code or enforced directly by the database — and are optimized above all for
**reviewer-legibility**. The part of the system that most needs rigorous review is kept the
smallest.

---

## What this is (and isn't), right now

**This is** a completed architecture specification, now being proven viable through
proof-of-concept code. The design was worked out deliberately and from clinical first
principles, because the decisions that matter most — the data model, the identity system, the
synchronization semantics — are the ones that are ruinously expensive to get wrong later. With
that spec settled, the hardest mechanisms are being validated in working spikes: a walking
skeleton whose append-only sync was proven over a real intermittent satellite link, a first
node that federates with its neighbours, and the safety layer tested against a deliberately
hostile AI agent.

**This is not yet** a product you can install. There is no clinical application today — the
spikes prove the architecture holds; they are not the EHR. Most contribution right now is still
design work: clinical case-mining, stress-testing the data model, and refining the spec, plus
the build-prep spikes.

---

## Design at a glance

| Concern | Approach |
|---|---|
| **Resilience** | Offline-first; every node is write-capable; syncs to its parent when able; degrades to a single standalone workstation |
| **Synchronization** | Append-only event log + causal ordering (hybrid logical clocks); merge becomes set union plus a small, explicitly clinically-reasoned set of policies |
| **Identity** | Linkage layer over immortal patient IDs; probabilistic + deterministic matching; link / unlink / reattribute / repudiate as auditable events |
| **Topology** | Fractal: workstation → department → facility → region → nation, same codebase |
| **Foundation** | PostgreSQL ≥ 18; commodity hardware down to Raspberry-Pi class; standard Linux |
| **Interoperability** | FHIR as the interface, not a lock-in |
| **Licensing** | AGPL-3.0 end to end |

For the full architecture, see the specification documents in this repository.

---

## Contributing

This project is for the people who have to use these systems and the people who have to keep
them running — clinicians, health-IT engineers, and anyone who has been failed by an EHR and
believes it could be otherwise. Clinical realism is as valued here as code: a well-described
failure mode from the front line is a genuine contribution.

**How to contribute, and how the project is governed:** see
**[CONTRIBUTING.md](CONTRIBUTING.md)** and the full
**[Governance & Contributing](docs/principles/GOVERNANCE.md)** document. In short: the architecture
spec is complete and the code so far is proof-of-concept spikes proving viability; most contribution
today is still design work (spec refinement and clinical case-mining) plus build-prep; contributions are
AGPL-3.0, inbound = outbound, under the [DCO](https://developercertificate.org/) (`git commit -s`)
with **no CLA**; and the mission is the tie-breaker.

---

## License

Distributed under the **GNU Affero General Public License v3.0 (AGPL-3.0)**. This license is a
deliberate, foundational choice: it guarantees that this system — and any networked service
built on it — remains free and open for everyone who depends on it.
