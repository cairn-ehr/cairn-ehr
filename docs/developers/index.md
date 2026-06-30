# Developer Manual

Welcome. This manual is the **practical, hands-on guide for developers joining Cairn** — from
seasoned systems engineers to people writing their first line of Rust or SQL. It exists alongside
the architecture spec and the governance documents, but it answers a different question:

> *"I have the repo cloned. What is this, how is it built, and how do I make a change that gets
> merged?"*

If you are looking for **why** the system is the way it is, that lives in the
[architecture spec](../spec/index.md) and the [ADR log](../spec/decisions/README.md). If you are
looking for the **governance and licensing rules** you must follow, that lives in
[Governance & Contributing](../principles/GOVERNANCE.md). This manual is the bridge between them and
the code in front of you.

> [!NOTE]
> **Cairn is an offline-first, vendor-independent electronic health record** built as a public
> good. *The grid goes down. The chart stays up.* Read the [Vision](../spec/vision.md) and the
> [twelve founding principles](../spec/index.md) before you write code — they are not decoration,
> the entire architecture is downstream of them, and every change is reviewed against them.

---

## Who this manual is for

- **New contributors** who want to get a build green and understand the lay of the land.
- **Junior developers** — every page assumes you may be new to Rust, to PostgreSQL internals, or to
  health IT. Terms are defined in the [Glossary](glossary.md); the worked examples walk through real
  code line by line.
- **Clinicians and domain experts** who want to understand the codebase enough to test it against
  real front-line failure modes. You do **not** need to write code to contribute — see
  [Clinical case-mining](contributing-workflow.md#clinical-case-mining-is-a-first-class-contribution).

---

## Read these in order

1. **[Getting started](getting-started.md)** — install the toolchain, build every component, and get
   your first test passing. Start here.
2. **[Architecture for developers](architecture-for-developers.md)** — the mental model you need
   before the code makes sense: the four layers, *fat Postgres / thin daemon*, and the append-only
   event spine. Follows one real write from Rust through the database to a projection.
3. **[Repository map](repository-map.md)** — what every top-level directory is and which ones are
   load-bearing versus historical.
4. **[Codebase tour](codebase-tour.md)** — a guided reading path through the actual source, using the
   demographics slice as the worked example end-to-end.
5. **[Contributing workflow](contributing-workflow.md)** — how a change is actually made here:
   branches, sign-off, the brainstorm→spec→plan→TDD loop, ADRs, reviews, and how to pick your first
   task.
6. **[Glossary](glossary.md)** — every Cairn-specific term in one place (HLC, the twin, the floor, a
   veto, a projection, an actor, a slice…).

You don't have to read all of it before contributing. Read 1 and 2, skim 3–5, and come back.

---

## Where the project actually is right now

Cairn's **architecture is complete** (spec v0.40; every original open question is closed and recorded
in the [ADR log](../spec/decisions/README.md)), viability was proven by
[proof-of-concept spikes](../spikes/README.md), and the **first production clinical surface is now
under construction** — the patient **demographics** subsystem on the `cairn-node` crate, built
slice by slice.

Two living documents track the moving edge — and because they move, this manual deliberately does
**not** restate their contents, it points at them:

- **`docs/HANDOVER.md`** — the single most current snapshot of *exactly* what was built last session,
  what is in flight, and what is deferred. **Read it at the start of every session.** It is
  disposable working scaffolding (regenerated each session), not a source of truth — when it
  disagrees with the spec or an ADR, the canonical docs win.
- **[`docs/ROADMAP.md`](../ROADMAP.md)** — the foundation build order (wire core → in-DB floor → sync
  → identity → security → federation → blobs → native API) and how far each phase has progressed.

> [!IMPORTANT]
> The high-authority [Governance](../principles/GOVERNANCE.md) and root `CONTRIBUTING.md` describe
> the project's *pre-code* phase ("most contribution today is design work"). That was true when they
> were written and remains true for the **rules** they state (licensing, DCO, the decision process).
> The build has since moved into code — this manual covers that code phase. The governing rules in
> those documents still apply unchanged.

---

## Keeping this manual honest (maintenance contract)

A developer manual that drifts out of date is worse than none — it sends newcomers down dead ends.
This manual is **part of the published docs site** and is meant to be **actively maintained**. The
discipline that keeps it true:

- **Describe structure and process, not live counts.** Pages here name real directories, files, CLI
  subcommands, and workflows that change rarely. They deliberately avoid restating volatile facts
  (test counts, "current slice", spec version number) — those live in `docs/HANDOVER.md`,
  [`docs/ROADMAP.md`](../ROADMAP.md), and [`spec/index.md`](../spec/index.md), which are the
  single sources of truth this manual links to.
- **Update triggers.** Touch the relevant page in the *same change* when you:
  - add, remove, or rename a **top-level directory** or a **crate** → [Repository map](repository-map.md);
  - change how a component is **built or tested** (toolchain version, a new `cargo`/`uv` invocation,
    a new env var) → [Getting started](getting-started.md);
  - change the **contribution mechanics** (branching, sign-off, the SDD/TDD loop) →
    [Contributing workflow](contributing-workflow.md);
  - introduce a **new core concept** worth a newcomer knowing → [Glossary](glossary.md) and, if it
    changes the mental model, [Architecture for developers](architecture-for-developers.md).
- **The site build is strict.** `mkdocs build --strict` runs on every PR (see
  [Getting started → The docs site](getting-started.md#6-the-documentation-site)); a dead internal link
  or a broken nav entry blocks the merge button. Run it locally before you push doc changes.
- **When in doubt, link don't copy.** Prefer a link to the canonical doc over a paraphrase that can
  rot.

If you find something in this manual that is wrong or stale, that is itself a bug worth fixing —
open a PR, or at minimum an issue.
