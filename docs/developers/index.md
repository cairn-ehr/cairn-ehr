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

1. **[Getting started](getting-started.md)** — install the toolchain, build every component (three
   cargo trees, the extension, the window, the matcher, the docs), and get your first test passing.
   Start here.
2. **[Architecture for developers](architecture-for-developers.md)** — the mental model you need
   before the code makes sense: the four layers, *fat Postgres / thin daemon*, the append-only event
   spine, the two doors and two planes, and what a clinical write signed by a human looks like.
3. **[Repository map](repository-map.md)** — what every top-level directory and crate is, and which
   ones are load-bearing versus historical.
4. **[Codebase tour](codebase-tour.md)** — a guided reading path through the actual source:
   demographics end-to-end, the matcher's veto/score split, the progress note, the reference window,
   and the CLI.
5. **[Contributing workflow](contributing-workflow.md)** — how a change is actually made here:
   the eight house rules, branches, sign-off, the brainstorm→spec→plan→TDD loop, ADRs, reviews, and
   how to pick your first task.
6. **[Running the duplicate check](running-the-duplicate-check.md)** — operating the matcher's
   commit-time worker on a real node.
7. **[Durable rules](durable-rules.md)** — the per-slice "do not undo this" rules and the numbered
   traps, each with the test that pins it. Not a cover-to-cover read: **read the block for any code
   you are about to touch.**
8. **[Glossary](glossary.md)** — every Cairn-specific term in one place (HLC, the twin, the floor,
   the two doors, born-sealed, the key vault, a draft, a slice…).

You don't have to read all of it before contributing. Read 1 and 2, skim 3–5, and come back.

---

## Where the project actually is right now

Cairn's **architecture is complete** (every original open question is closed and recorded
in the [ADR log](../spec/decisions/README.md); the current spec version is stated in
[`spec/index.md`](../spec/index.md)), viability was proven by
[proof-of-concept spikes](../spikes/README.md), and the **first production clinical surface is under
construction** on the `cairn-node` crate, built slice by slice. In broad strokes, what exists:

- **The event core, the in-DB floor and federation** — signed events, the strict and remote doors,
  node-plane and clinical-plane sync, backup and disaster-recovery restore.
- **Patient identity** — demographics, the advisory matcher and its in-DB veto, the identity
  algebra (link, unlink, dispute, identify, repudiate), John Doe, registration as an act that
  carries its search, and the duplicate repair path.
- **Clinical content** — `clinical.medication` and `clinical.note` (progress notes with drafts), all
  under **born-sealed bodies** and **per-write human authorship**, plus the sensitivity stream and
  the safety projection.
- **A runnable reference window** (`cairn-gui/`, Tauri 2) — the search-before-create front door,
  the chart, duplicate review, the clinicians' **key vault** and sign-in, and the note editor.

There are still **no allergies, prescriptions, orders, or results**, and no native API yet. Don't
assume build/run instructions beyond what this manual and `cairn-node --help` describe.

Two living documents track the moving edge — and because they move, this manual deliberately does
**not** restate their contents, it points at them:

- **`docs/HANDOVER.md`** — the single most current snapshot of *exactly* what was built last session,
  what is in flight, and what is deferred. **Read it at the start of every session**, and then check
  the open pull requests: HANDOVER describes `main`, and the newest work is often on a draft PR. It
  is disposable working scaffolding (regenerated each session), not a source of truth — when it
  disagrees with the spec or an ADR, the canonical docs win.
- **[`docs/ROADMAP.md`](../ROADMAP.md)** — the foundation build order (wire core → in-DB floor → sync
  → identity → security → federation → blobs → native API) and how far each phase has progressed.

> [!IMPORTANT]
> The high-authority [Governance](../principles/GOVERNANCE.md) still describes the project's
> *pre-code* phase in places ("most contribution today is design work"). That was true when it was
> written and remains true for the **rules** it states (licensing, DCO, the decision process). The
> build has since moved into code — this manual covers that code phase. The governing rules in that
> document still apply unchanged.

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
  - add, remove, or rename a **top-level directory**, a **crate** (in any of the three cargo trees),
    or a **script** → [Repository map](repository-map.md);
  - change how a component is **built or tested** (toolchain version, a new `cargo`/`uv` invocation,
    a new env var, a new required CI check) → [Getting started](getting-started.md) (and the root
    `CONTRIBUTING.md`, which tables the required checks);
  - change the **contribution mechanics** (branching, sign-off, the SDD/TDD loop, a house rule) →
    [Contributing workflow](contributing-workflow.md);
  - add a rule a later change could undo in good faith → its block in [Durable rules](durable-rules.md),
    with the test that pins it;
  - build a new **clinical stream** or a new reference-window surface → the
    [Codebase tour](codebase-tour.md) if it changes the recommended reading path, and the
    "where the project is" list above;
  - introduce a **new core concept** worth a newcomer knowing → [Glossary](glossary.md) and, if it
    changes the mental model, [Architecture for developers](architecture-for-developers.md).
- **The site build is strict.** `mkdocs build --strict` runs on every PR (see
  [Getting started → The docs site](getting-started.md#7-the-documentation-site)); a dead internal link
  or a broken nav entry blocks the merge button. Run it locally before you push doc changes.
- **When in doubt, link don't copy.** Prefer a link to the canonical doc over a paraphrase that can
  rot.

If you find something in this manual that is wrong or stale, that is itself a bug worth fixing —
open a PR, or at minimum an issue.
