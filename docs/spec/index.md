# Cairn — Architecture Specification

> **The grid goes down. The chart stays up.**

Offline-first, vendor-independent electronic health record. Keeps working through any network
outage, runs anywhere from a Raspberry Pi to a hospital cluster, and belongs to no vendor.

**Status:** Architecture spec essentially complete (all open questions resolved); now proving viability through build-prep spikes (walking skeleton, WAN sync, a first federating node) — clinical implementation not yet begun.
**Spec version:** 0.36 · **License target:** AGPL-3.0 (all components AGPL-3.0-compatible).
**Core constraint:** full clinical functionality must survive loss of internet *and* intranet,
degrading gracefully down to a single workstation.

> [!NOTE]
> This is the entry point to the multi-file architecture spec. Read this page, then follow the
> [document map](#document-map). The *why* behind each decision lives in the
> [Decision log](decisions/README.md) (ADRs), not in per-file changelogs.

---

## Why this exists

Most clinicians have, at some point, watched a computerized health record make their day slower,
their workflow clumsier, or their patients less safe than the paper chart it replaced — and many
have watched promising public efforts collapse under conflicting commercial interests, where
lock-in was the business model and interoperability the thing quietly sabotaged.

Cairn starts from a different place. It has **no vendor in the room**. There is no revenue that
depends on trapping your data, no proprietary layer you must license, no cloud you are required to
trust. Because nothing here is incentivized to keep the hard problems hard, we let one thing — and
only one thing — drive every design decision: **what actually happens at the point of care,
including at 3 a.m. when the network is down.**

## The mission

Build a health record system that:

- **Keeps working through any outage.** Loss of internet, loss of the hospital intranet, or a
  single isolated computer — care continues. The clinician can always read the locally relevant
  record and write new clinical data. Synchronization catches up when connectivity returns.
- **Runs anywhere, for anyone.** The same software serves a solar-powered clinic on an intermittent
  mobile connection *and* a tertiary hospital in a wealthy country. One codebase, scaled by
  configuration — from a single workstation to a national deployment.
- **Belongs to no one but its users.** Fully open source under AGPL-3.0, built only on commodity
  hardware and open standards, with no proprietary dependency and no vendor lock-in at any layer.
- **Respects the clinician's time and judgment.** No workflow may be slower, harder, or more
  error-prone than its paper equivalent.

## The eventual goal

A genuinely free, genuinely portable electronic health record that any health system in the world —
from a one-room rural practice to a national network — can adopt, run, inspect, and adapt without
asking anyone's permission and without surrendering control of its data. An EHR that earns
clinicians' trust by being *available*, *honest*, and *fast*, and that treats patient safety and
data sovereignty as architectural guarantees rather than marketing claims.

> [!IMPORTANT]
> **Paper-parity is the governing law.** No clinical workflow may be slower, more difficult, more
> cognitively demanding, or impossible compared to its paper-record equivalent (sole exclusions:
> paper capabilities that constitute malfeasance). A workflow that loses to paper is a defect. See
> [Vision §1.2](vision.md#12-the-paper-parity-test-normative).

## Founding principles (the lens for every decision)

Everything in the architecture is downstream of these. New design choices are checked against the
first four before anything else.

1. **Append-only + causal ordering** — all clinical content is immutable, signed events ordered by
   Hybrid Logical Clocks; corrections reference originals. Sync becomes safe **set union** plus a
   small, explicitly enumerated set of clinically-reasoned merge policies.
2. **Identity is a claim, never a fact** — **never merge, always link; never erase, always
   overlay.** Patient UUIDs are immortal; identity is an append-only event stream; every error is
   repairable by an auditable event with no data loss.
3. **Paper-parity (governing law)** — see the callout above.
4. **Acknowledged uncertainty** — an imprecise near-truth always beats a precise untruth. The
   system never forces a clinician to commit data they cannot vouch for; uncertainty, imprecision,
   ranges, and an explicit *unknown* (distinct from not-yet-asked and from refused) are first-class
   recordable values, no required field is satisfiable only by fabrication, and certainty is refined
   later by overlay ([data-model §3.7](data-model.md#37-acknowledged-uncertainty-uncertainty-capable-value-types)).
   *Corollary:* **deletion is best-effort and declared, never guaranteed** — the strongest honest claim
   is *"to our knowledge, we have erased all copies in our existence"*
   ([data-model §3.8](data-model.md#38-erasure-and-key-custody), [ADR-0005](decisions/0005-erasure-key-custody-and-crypto-shredding.md)).
5. **Availability over consistency** — a clinician must always be able to read locally-relevant
   records and write new data during a partition (AP in CAP terms).
6. **Fractal topology** — one codebase at every tier; a node's role is configuration, not a
   different product.
7. **Vendor independence** — AGPL-3.0 throughout, open standards, commodity hardware, PostgreSQL;
   no proprietary services, no mandatory cloud, no license keys.
8. **Safety-critical logic is unbreakable and auditable** — implemented where whole error classes
   become unrepresentable (Rust or in-database), optimized above all for reviewer-legibility, and
   kept as small as possible.
9. **Policy-neutral infrastructure** — Cairn provides *mechanism*, never policy. Conflicting legal and
   health-system requirements (retention, erasure, disclosure, compliance posture) are *facilitated
   without taking sides*: the system builds the full range of mechanisms spanning the worst-case
   extremes and lets configuration/UI select. The clearest instances are erasure as a policy-selected
   severity ladder ([security §7.1](security.md#71-erasure-the-severity-ladder),
   [ADR-0005](decisions/0005-erasure-key-custody-and-crypto-shredding.md)), confidentiality as a
   blacklist + grading-system + human-editability the policy/UI combines ([identity §5.9](identity.md#59-sensitivity-grade-the-safety-projection-and-break-glass-visibility-scope),
   [ADR-0006](decisions/0006-visibility-scope-replication-and-the-safety-projection.md)), and compliance
   posture as configuration — an append-only policy-assertion stream + effective-policy projection
   ([security §7.9](security.md#79-hard-policy-expression-projection-and-enforcement),
   [ADR-0024](decisions/0024-hard-policy-expression-the-policy-assertion-stream.md)).
10. **Authorship is compositional; accountability is separable** — the author of a clinical event is a
    *set* of contributors (human, AI agent, or device), each in a declared role. **Legal responsibility
    is a distinct attribute, orthogonal to authorship and to whether a contributor is human or machine**:
    it may be absent (no one vouches), held, or proxied (held on another's behalf). A signature proves
    *origin and integrity*; *attestation* confers *responsibility*; the two are separable. Cairn records
    who authored, in what role, and who answers for it — and is indifferent to whether, over time,
    machines come to hold responsibility in their own right. "AI-generated" is therefore an emergent
    reading, never a flag ([data-model §3.9](data-model.md#39-authorship-and-accountability),
    [security §7.2](security.md#72-signing-attestation-and-ai-agent-identity),
    [ADR-0007](decisions/0007-authorship-and-accountability.md)).
11. **Legibility across time** — every clinical event must remain human-readable for as long as it
    exists, independent of how far the schema or software has since moved. This is paper-parity
    (principle 3) extended along the *time/version* axis: ink on paper from decades past needs no
    "version" to be read, and a Cairn event must match that — a node generations behind (or ahead)
    can always read an event as a clinician reads a progress note. *Schema is versioned data, not
    privileged structure.* The mechanism is a mandatory, signed, mechanically-derived **plaintext
    legibility twin** on every event (also the substrate for full-text search and RAG context) plus
    **additive-only** schema evolution, so the original is never lost and the rendering is always
    regenerable ([data-model §3.13](data-model.md#313-schema-evolution-event-format-and-the-legibility-twin),
    [sync §6.5](sync.md#65-schema-evolution-two-planes-and-lossless-forwarding),
    [ADR-0012](decisions/0012-schema-evolution-event-format-and-legibility-across-time.md)).
12. **Uniform core, plural edges** — the contract that makes any node interoperable with any other is the
    signed, append-only **event core** (serialization/signature format, set-union sync, the
    identity/actor algebras, additive-only evolution), and *nothing above it* — no API, policy, or UI —
    may sit on the inter-node path. **Compatibility is a property of the core, not of the application:**
    the safety/compatibility floor is enforced unbypassably *in the database* (a client talking raw SQL
    still cannot break it), and above that floor UIs, soft policy, and whole application layers may
    proliferate freely. A bespoke UI can produce content wrong for its clinic but **never a
    wire-incompatible event** — *many front-ends, one record.* This is what lets the anti-capture mission
    survive UI diversity: the native API evolves additively (principle 11 applied to the contract) and
    even the steward's reference UI is built only on the same public API everyone else uses
    ([language-substrate §9.5](language-substrate.md#95-layering-the-node-api-and-ui-pluralism-uniform-core-plural-edges),
    [ADR-0021](decisions/0021-layering-the-node-api-and-ui-pluralism.md)).

---

## Document map

| § | Aspect | Document | Covers |
|---|---|---|---|
| 1 | Vision & design goals | [vision.md](vision.md) | Goals, the paper-parity test, non-goals, guiding principles |
| 2 | Topology | [topology.md](topology.md) | Tiers, write-capability, autonomous nodes vs. thin clients |
| 3 | Data model | [data-model.md](data-model.md) | Append-only log, UUIDv7/HLC, mutable state, hybrid storage |
| 4 | Demographics | [demographics.md](demographics.md) | Assertion-stream model, per-field projection policy |
| 5 | Identity subsystem | [identity.md](identity.md) | Linkage, matching, registration classes, event algebra, trust states |
| 6 | Synchronisation | [sync.md](sync.md) | Mechanism, consistency model, designed-for failure modes |
| 7 | Security & compliance | [security.md](security.md) | Encryption, offline auth, audit, mTLS |
| 8 | Deployment profiles | [deployment.md](deployment.md) | Hardware floors and stacks per tier |
| 9 | Language & substrate | [language-substrate.md](language-substrate.md) | Defect-blast-radius selection rule; the merge boundary |
| 10 | Technology candidates | [technology.md](technology.md) | Candidate/reference components and licenses |
| 11 | Open questions | [open-questions.md](open-questions.md) | Remaining brainstorming targets |
| — | **Decision log (ADRs)** | [decisions/](decisions/README.md) | The *why* behind settled decisions |

## How this spec is organized and versioned

- **One file per aspect.** Each document owns one concern and keeps its section numbering (so
  cross-references like *§5.7* stay valid inside [identity.md](identity.md)).
- **Git is the line history.** Files are not version-suffixed; the spec version is stated here.
- **The Decision log is the home of "why."** Decisions are recorded as numbered, dated,
  **immutable** ADRs; a reversal is a *new* superseding ADR, never an edit — the project's own
  *"never erase, always overlay"* applied to its own documentation. Read the relevant ADR before
  reopening a settled question.
- **Pre-ADR history** (the changelogs from v0.1 → v0.6) is preserved verbatim in
  [decisions/0000-pre-adr-changelog-v0.1-v0.6.md](decisions/0000-pre-adr-changelog-v0.1-v0.6.md).

## Reading order

1. This page (mission + map).
2. [vision.md](vision.md) — the goals and the paper-parity test that govern everything.
3. The aspect documents in order, or jump via the map.
4. [decisions/](decisions/README.md) — when you need *why* a thing is the way it is.

*The canonical statements of mission and governance also live in the repository's root
`README.md` and in [Stewardship of the Name](../principles/STEWARDSHIP-OF-THE-NAME.md). The mission
prose above restates them as the spec's own framing.*
