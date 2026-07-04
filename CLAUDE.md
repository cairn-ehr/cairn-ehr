# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this repository is

Cairn is an **offline-first, vendor-independent electronic health record**. The **architecture spec is
complete** (all open questions resolved; the current spec version is stated in `docs/spec/index.md`,
never restated here); viability was proven by **proof-of-concept spikes**
(walking skeleton + WAN sync in `poc/walking-skeleton`, an advisory-actor write-contract spike, a first
federating node `cairn-node` under `/crates`, Postgres-on-Android). As of 2026-06-27 the **first
production clinical surface is now under construction** — the demographics §4.4 patient-identifier
assertion tier on `cairn-node` (`db/010_demographics.sql` + `cairn-event::demographics`; PR #68, slice 1
of many). So work is now a **mix of building the clinical product slice-by-slice** (graduating the proven
event/floor/projection spine into real product code, brainstorm→spec→plan→TDD) **and continued design
work** (clinical case-mining, stress-testing the data model, refining spec prose). There is a Rust/Cargo
workspace (`/crates`, `/extensions`, `/db`) and an mkdocs docs build; the clinical application is **early
and partial** — only the §4.4 identifier slice exists, so don't assume product build/run instructions
beyond what `cairn-node` actually provides.

## Document hierarchy (what wins when sources disagree)

1. **`docs/principles/`** — canonical statements of mission and governance. Highest authority.
   - `STEWARDSHIP-OF-THE-NAME.md` — the "name belongs to the mission" commitment.
   - The mission and founding principles also live in the root `README.md`.
2. **`docs/spec/`** — the canonical architecture spec, **one file per aspect**, entry point
   `docs/spec/index.md` (carries the mission prose + document map). Each aspect file keeps its
   section numbering, so cross-references like *§5.7* stay valid inside `identity.md`.
   - **`docs/spec/decisions/`** — the **ADR log**: the home of *why*. ADRs are numbered, dated, and
     **immutable** (a reversal is a new superseding ADR, never an edit — the project's own "never
     erase, always overlay"). **Read the relevant ADR before reopening any settled question.**
   - The spec carries **no in-file changelogs and no filename version suffixes**; git is the line
     history and the spec version is stated in `index.md`. Pre-ADR history (v0.1→v0.6 changelogs) is
     preserved in `docs/spec/decisions/0000-pre-adr-changelog-v0.1-v0.6.md`.
   - **HTML is generated, not hand-edited.** Source is Markdown; the site builds with
     `uv run --with-requirements docs/requirements.txt -- mkdocs build` (config: `mkdocs.yml`) —
     always via the **pinned** `docs/requirements.txt` (same as CI), never ad-hoc `--with` installs:
     unpinned docs tooling executes unaudited code at build time (see the security note in
     `docs/requirements.txt`).
     Author callouts in GitHub/Obsidian syntax (`> [!NOTE]`) so they render on GitHub *and* as
     Material admonitions. Never commit the generated `site/` (gitignored).
3. **`docs/HANDOVER.md`** — disposable working scaffolding, NOT a source of truth. It points at
   the canonical docs and captures decisions made in conversation but not yet written into them.
   **Regenerate it at the end of a working session.** If it disagrees with canonical docs, the
   canonical docs win.

When starting a session, read HANDOVER.md first for current state (open questions, decisions
pending write-up, time-sensitive items), then the canonical docs it points to.

## The four governing principles (the lens for every decision)

Check every new design choice against these four before anything else. They are load-bearing;
the entire architecture is downstream of them.

1. **Append-only + causal ordering.** All clinical content is immutable, signed events ordered
   by Hybrid Logical Clocks. Corrections are new events referencing originals. This makes sync a
   safe **set-union** operation plus a small, *explicitly enumerated* set of clinically-reasoned
   merge policies — never a dangerous merge. Proposed mutability is almost always wrong.
2. **Identity is a claim, never a fact.** **Never merge — always link; never erase — always
   overlay.** Patient UUIDs are immortal; identity is an append-only stream of link/unlink/
   reattribute/repudiate/identify/dispute events (the closed "identity event algebra," spec §5.7).
   Every identity error must be repairable by an auditable event with no data loss.
3. **Paper-parity (governing law).** No clinical workflow may be slower, harder, more cognitively
   demanding, or impossible versus its paper-record equivalent (sole exclusion: paper capabilities
   that are malfeasance — silent falsification, untraceable backdating). New workflows must name
   their paper counterpart and benchmark in time/steps/cognitive load. **Confirmation dialogs are
   explicitly NOT an acceptable safety mechanism** — they fail paper-parity; restore the physical
   affordance instead (e.g. possession semantics for wrong-chart prevention).
4. **Acknowledged uncertainty.** An imprecise near-truth always beats a precise untruth. Never force
   a clinician to commit data they cannot vouch for: uncertainty, imprecision, ranges, and an explicit
   *unknown* (distinct from *not-yet-asked* and from *refused*) are first-class recordable values; no
   required field may be satisfiable only by fabrication; certainty is refined later by overlay, never
   forced up front. Time is the canonical case — objective `t_recorded` (HLC, the ceiling) vs. asserted
   `t_effective` (the displayed, freely-backdatable claim); clashes are flagged, never auto-resolved
   ([ADR-0003](docs/spec/decisions/0003-bitemporal-time-and-acknowledged-uncertainty.md), spec §3.6/§3.7).

Two more architectural invariants worth holding: **availability over consistency** (a clinician
must always be able to read locally-relevant records and write new data during a partition; AP in
CAP terms) and **fractal topology** (one codebase at every tier — workstation→department→facility→
region→nation; a node's role is configuration, not a different product).

A **tenth founding principle** — *authorship is compositional; accountability is separable* — generalizes
"AI-generated content" into a contributor set plus a separable, possibly-absent, possibly-proxied
responsibility attribute (signature proves origin/integrity; attestation confers responsibility)
([ADR-0007](docs/spec/decisions/0007-authorship-and-accountability.md), spec §3.9 / §5.10 / §7.2).

An **eleventh founding principle** — *legibility across time* — extends paper-parity along the
time/version axis: a clinical event stays human-readable for as long as it exists, no matter how far the
schema has moved (*schema is versioned data, not privileged structure*). Mechanism: a mandatory, signed,
mechanically-derived **plaintext legibility twin** on every event (also the full-text/RAG substrate) plus
**additive-only** schema evolution. This came out of resolving §11.4 (schema migration across the offline
fleet): evolution is routed through **two planes** — clinical events sync forward-compatibly (never
executable code), while code/DDL/pgrx extensions travel a **separate signed, per-architecture,
sneakernet-capable distribution plane**; the schema/extension version is a *local node property*, so there
is no lockstep fleet upgrade ([ADR-0012](docs/spec/decisions/0012-schema-evolution-event-format-and-legibility-across-time.md),
spec §3.13 / §6.5 / §7.6).

A **twelfth founding principle** — *uniform core, plural edges* — makes the anti-capture mission durable
against UI fragmentation: the contract that makes any node interoperable with any other is the signed,
append-only **event core** (serialization/signature, set-union sync, the identity/actor algebras,
additive-only evolution), and *nothing above it* — no API, policy, or UI — may sit on the inter-node path.
The safety/compatibility floor is enforced **unbypassably in the database** (validated submit functions +
RLS + constraints), so even a client talking raw SQL cannot break it, and *"via the API vs. DB directly"*
is a **privilege gradient, not a contradiction**; above that floor, UIs and soft policy proliferate freely.
A bespoke UI can produce content wrong for its clinic but **never a wire-incompatible event** — *many
front-ends, one record*. A **four-layer model** (wire core / node enforcement floor / policy+API / UI) puts
the compatibility boundary **below the application layer**; **hard policy** is DB-anchored or role-gated,
**soft policy** lives in the UI; the native API evolves **additively** (principle 11 applied to the
contract), is capability-described + conformance-tested, is distinct from the FHIR interop façade, and even
the steward's reference UI is built only on the same public API everyone else uses
([ADR-0021](docs/spec/decisions/0021-layering-the-node-api-and-ui-pluralism.md), spec §9.5).

**§11.6 (attachment strategy) is resolved** — attachments are **content-addressed blobs referenced by the signed
event, never inlined** (append-only applied to large binaries; the digest is to a blob what the signature is to a
body). The **reference is eager, the bytes are lazy** on a **resource-isolated byte tier** that can never starve
clinical sync (the availability floor, *chunked/preemptible/separately-budgeted, not merely priority-ordered* —
motivated by a real nightly-imaging-sync-grinds-everything-to-a-halt failure); **byte-replication is opt-in and
separately scoped** (references everywhere, bytes by election; starved node = references-only, fetch-on-demand);
content-addressing gives **self-verifying multi-source swarm fetch**. The **rendition set** is the binary's
legibility twin, adding a **retrievability** axis to the §3.13 ladder (`min(retrievable, parseable, cleared)`);
erasure (per-blob DEK crypto-shred) and lossless passthrough inherit. The one can't-retrofit piece is the
**day-one attachment-reference shape**. No new founding principle
([ADR-0013](docs/spec/decisions/0013-attachments-content-addressed-lazy-blob-tier.md), spec §3.14 / §6.6).

**§11.7 (locale-pluggable matcher comparators) is resolved** — and with it **every original §11 open question
is now closed.** The matcher is **advisory**, so this is low-stakes (no envelope reserve, one small *additive*
data-model field, no new founding principle). Hardcoding one culture's name/date/address model is **cultural
capture**; comparators are pluggable and *no-data-is-never-disagreement* (principle 4). The sharp resolution to
*"comparators must travel with the data"* (relocation across wildly different naming conventions) without syncing
code or a central registry: a **content-addressed comparator-profile tag travels with each demographic assertion**
(silently defaults from the registering node's locale, registrar-overridable for relocation/visitor cases), the
comparator **code** travels the distribution plane, and a node lacking a record's comparator **degrades honestly
to human review, never forcing the wrong comparator** (safety-preserving: uncertainty can only *withhold* an
auto-link). Silent false splits are caught by a **low-priority, preemptible, aggressive background duplicate sweep
at the hub** whose advisory-worklist yield doubles as the miss-rate/drift metric. Hard vetoes **force a human
decision, never an auto-reject.** The matcher is a **registered actor** (config version-pinned, recall via
contamination cascade); GitHub doubles as a federated, signed, content-addressed registry
([ADR-0014](docs/spec/decisions/0014-locale-pluggable-matcher-comparators.md), spec §5.13 / §4.1). The remaining
generative mode is now continued **clinical case-mining** or the build-prep threads (governance doc, Pi-benchmark
spike).

## When implementation begins: language/substrate selection rule

The spec deliberately does **not** fix languages per component (spec §9). It fixes the *rule*:
**choose by defect blast radius.**

- **Safety-critical** (a defect can silently corrupt the record, mis-merge patients, leak data,
  or crash an unattended node) → **Rust or in-database (SQL / PL-pgSQL / constraints)**, optimized
  above all for **reviewer-legibility**. Members: sync/merge engine, identity event algebra and
  projections, HLC ordering, coherence checks, audit-log integrity, access control. Keep this
  surface as small as possible.
- **Fit-for-purpose** (defect is caught immediately, advisory, or cosmetic) → optimize for
  iteration speed. Members: the probabilistic matcher (advisory only — Python/ML), FHIR façade,
  integration glue, UI backends.
- **The integration boundary is the database boundary.** Each component talks to its node's
  PostgreSQL (≥ 18); Postgres is the integration substrate. Avoid FFI coupling.

All components must be **AGPL-3.0-compatible**. The whole project is AGPL-3.0 — non-negotiable.

## Coding house rules (apply to all code, spikes included)

These are load-bearing working agreements — hold to them on proof-of-concept spikes as well as product code.

1. **Licensing is non-negotiable.** All code we produce is **AGPL-3.0**; every external dependency we pull in
   must be **AGPL-3.0-compatible.** Check a dependency's license *before* adding it — an incompatible license is
   a blocker, not a cleanup-later item (this is the anti-capture mission applied to the supply chain).
2. **Test-driven development.** Write the failing test first, then the code that makes it pass. No production
   code without a test that drove it. This is especially load-bearing for the safety-critical surface (§9), where
   a silent defect can corrupt the record.
3. **Document inline for a junior developer.** Every non-trivial function/module carries comments that make its
   **flow and purpose** clear to a junior developer newly joining the team — *why* it exists and *how* it fits,
   not just *what* the next line does. Reviewer-legibility (already the §9 rule for safety-critical code) is the
   house default everywhere.
4. **Prefer pure, reusable functions over clever complexity.** Readability and reusability beat "smart-looking"
   code every time. Favour small pure functions with explicit inputs/outputs over intricate structures or
   cleverness; if a reviewer has to puzzle out what it does, simplify it.
5. **Fix review findings; if you can't, file an issue.** When a code review surfaces an error, **fix it.** If it
   can't be fixed in place (out of scope, needs a decision, blocked), **open a GitHub issue** capturing it — never
   let a known defect pass silently. (Mirrors the project's "surface flaws early" principle, made actionable.)

## Working conventions

- **The user is an EM physician** who codes mostly in Python and brings real ED/hospital failure
  modes from multiple health systems. Case-mining (testing whether existing primitives absorb a
  real clinical failure mode) is the most productive mode — the event-overlay primitives have so
  far absorbed every case without needing new architecture. Treat clinical realism as first-class.
- **The mission is the tie-breaker.** The project is explicitly anti-capture / anti-vendor-lock-in.
  When convenience conflicts with the mission (open standards, no proprietary dependency, no
  mandatory cloud, data sovereignty), the mission wins.
- **Always surface flaws early — criticism is strongly encouraged.** Where you find a flaw, risk, gap,
  or unsound step — in a design, a proposed mechanism, the user's idea, or your own — **point it out
  plainly and immediately, with the specific failure scenario.** Do not soften it into vagueness, bury
  it, or wait to be asked. The user often works sleep-deprived after long hours or rushed between
  patients, and explicitly wants you to **have their back**: catch the mistake or the not-thought-through
  step and say so. Errors caught early beat being bitten later. If an idea survives scrutiny, say so; if
  it has caveats, enumerate them. Applies to your own proposals too.
- **Don't re-litigate parked decisions** (e.g. legal entity/jurisdiction, formal trademark
  registration) without new reason — see HANDOVER.md "Parked" section.
- `docs/spec/open-questions.md` (§11) lists the open architecture questions. The
  "how much intelligence lives inside Postgres" cluster (§11.1/§11.2/§11.11) is **resolved** —
  *fat Postgres, thin Rust daemon* — see [ADR-0001](docs/spec/decisions/0001-fat-postgres-thin-daemon.md).
  §11.3 (dynamic sync-scope handoff) is also **resolved** — *scope is a prefetch hint, not an
  authority* ([ADR-0004](docs/spec/decisions/0004-dynamic-sync-scope-prefetch-not-authority.md)); it
  also surfaced the bitemporal time model and the fourth governing principle (acknowledged
  uncertainty, [ADR-0003](docs/spec/decisions/0003-bitemporal-time-and-acknowledged-uncertainty.md)).
  §11.5 (tombstones/GDPR erasure in an append-only system) is also **resolved** — *erasure is
  redistribution of key-custody, not deletion of data*: crypto-shredding on an encryption-capable body
  slot, exposed as a policy-neutral severity ladder
  ([ADR-0005](docs/spec/decisions/0005-erasure-key-custody-and-crypto-shredding.md)); it added a ninth
  founding principle (**policy-neutral infrastructure** — mechanism, never policy) and a corollary of the
  fourth (**deletion is best-effort and declared, never guaranteed**).
  §11.8 (visibility-scope ↔ sync-scope) is also **resolved** — *replication is never the confidentiality
  boundary*: a safety-relevant sensitive episode replicates unconditionally, and confidentiality lives in
  key-custody + visibility + envelope-abstraction; a sealed body emits a de-identified, severity-graded
  **safety projection** so decision-support warns without disclosing; sensitivity is a graded, append-only
  blacklist+grading+human-editability stream (Cairn ships the mechanism, policy combines it); break-glass
  is audited key-*use* ([ADR-0006](docs/spec/decisions/0006-visibility-scope-replication-and-the-safety-projection.md),
  [identity §5.9](docs/spec/identity.md)). It also absorbed the ADR-0005 rung-1 metadata follow-on.
  §11.14 (trusted-time anchoring) is also **resolved** — *principle 4 applied to wall-clock truth*: `t_recorded`
  becomes a graded interval carrying a day-one **clock-confidence grade**; the 2001-era notary is reframed as two
  pluggable planes (clock-setting + a transparency-log-shaped multi-anchor existence-proof) on the ADR-0017
  anchor spectrum, with offline as an honest bracket riding the gossip plane
  ([ADR-0027](docs/spec/decisions/0027-trusted-time-anchoring.md)). **With ADR-0027 every original §11 open
  architecture question is closed.** **[Spike 0002](docs/spikes/0002-advisory-actor-write-contract.md)** (advisory-actor
  write contract) then passed C1–C5 (the in-DB floor holds against a hostile agent even with direct DB access),
  triggering **two ADRs** (spec **v0.31**): **[ADR-0029](docs/spec/decisions/0029-skill-epoch-as-pinned-actor-determinant.md)**
  (skill-epoch + served-model digest as pinned determinants of an agent actor's identity, refining ADR-0011 — canonical
  home spec §7.5) and **[ADR-0030](docs/spec/decisions/0030-advisory-actor-integration-contract.md)** (the advisory-actor
  integration contract — L2/L3 attachment + authorship through the validated in-DB floor; promotes the ecosystem/0001
  evaluation to a decision — canonical home spec §9.8). The remaining generative threads are build-prep (the Bet B Pi
  compute-cost run) and continued clinical case-mining.
