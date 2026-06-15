# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this repository is

Cairn is an **offline-first, vendor-independent electronic health record**, currently in the
**architecture / specification phase — there is no code, build system, or tests yet.** The
current artifacts are design documents and the reasoning behind them. Work here is design work:
clinical case-mining, stress-testing the data model, and writing/refining specification prose.
Do not invent build/test/run instructions; they don't exist until implementation begins.

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
     `uv run --with mkdocs-material --with mkdocs-callouts --with mkdocs-redirects -- mkdocs build` (config: `mkdocs.yml`).
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
spec §3.13 / §6.5 / §7.6). Remaining open §11 items: **§11.6** (attachment strategy) and **§11.7**
(locale-pluggable matcher comparators).

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

## Working conventions

- **The user is an EM physician** who codes mostly in Python and brings real ED/hospital failure
  modes from multiple health systems. Case-mining (testing whether existing primitives absorb a
  real clinical failure mode) is the most productive mode — the event-overlay primitives have so
  far absorbed every case without needing new architecture. Treat clinical realism as first-class.
- **The mission is the tie-breaker.** The project is explicitly anti-capture / anti-vendor-lock-in.
  When convenience conflicts with the mission (open standards, no proprietary dependency, no
  mandatory cloud, data sovereignty), the mission wins.
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
  [identity §5.9](docs/spec/identity.md)). It also absorbed the ADR-0005 rung-1 metadata follow-on. Of the
  remaining open questions, **§11.9 (armed write-context)** is now the sharpest.
