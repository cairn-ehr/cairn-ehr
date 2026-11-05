# Contributing workflow

How a change is actually made in Cairn — the mechanics, the disciplines, and how to pick your first
task. This page is the practical companion to the authoritative
[Governance & Contributing](../principles/GOVERNANCE.md); where this page is brief on a *rule* (the
licence, the DCO, how decisions are recorded), Governance is the source of truth. The root
`CONTRIBUTING.md` carries the operational detail that changes with the tooling (the local test gate,
the required CI checks, writing a PR body).

---

## The non-negotiables (read once, internalize)

- **AGPL-3.0, inbound = outbound.** Everything you contribute is under the
  [AGPL-3.0](../principles/GOVERNANCE.md#4-licensing-and-contributor-terms). **Every dependency you
  add must be AGPL-3.0-compatible — check its licence *before* adding it.** An incompatible licence is
  a blocker, not a cleanup-later item; `cargo-deny` (`deny.toml`) enforces the allow-list in CI.
- **DCO, not a CLA.** Sign off every commit: `git commit -s` (adds the `Signed-off-by` line). The
  project deliberately uses **no CLA** — that keeps the copyleft strong and the project uncapturable.
- **The mission is the tie-breaker; paper-parity is the governing law.** When convenience conflicts
  with the mission (open standards, no proprietary dependency, no mandatory cloud, data sovereignty,
  patient safety), the mission wins. No clinical workflow may be slower, harder, more cognitively
  demanding, or impossible than its paper equivalent — and **confirmation dialogs are not an
  acceptable safety mechanism.**
- **Read the ADR before reopening a settled question.** Load-bearing decisions are
  [immutable ADRs](../spec/decisions/README.md). A genuine reversal is a *new superseding ADR*, never
  an edit. Don't re-litigate without a new reason.

---

## The coding house rules

These apply to **all** code, spikes included (they are in `CLAUDE.md` too, with their history):

1. **Licensing is non-negotiable** — see above.
2. **Test-driven development.** Write the failing test *first*, then the code that makes it pass. No
   production code without a test that drove it. This is especially load-bearing on the
   safety-critical surface, where a silent defect can corrupt the record.
3. **Document inline for a junior developer.** Every non-trivial function/module carries comments
   that make its **flow and purpose** clear to someone newly joining — *why* it exists and *how* it
   fits, not a restatement of the next line. Reviewer-legibility is the default everywhere (and the
   hard rule on safety-critical code).
4. **Prefer pure, reusable functions over clever complexity.** Small pure functions with explicit
   inputs/outputs beat intricate or "smart-looking" code. If a reviewer has to puzzle out what it
   does, simplify it.
5. **Fix review findings; if you can't, file an issue.** Never let a known defect pass silently.
6. **Never hard-code cryptographic material in tests — and never give a non-cryptographic value a
   cryptographic name.** Derive test keys/seeds at runtime rather than writing literals, *and* keep
   the words `salt`, `nonce` and `iv` for real cryptographic constructions — CodeQL picks its sink by
   the binding's name, so a test helper parameter called `salt` raises a critical alert at every call
   site. Call a discriminator a `seed`, `variant` or `lineage`. Enforced by
   `crates/cairn-node/tests/crypto_sink_names_are_genuine.rs`; read open alerts with
   `scripts/codeql-alerts.sh` rather than assuming one is the familiar false positive.
7. **Every clinical-surface slice plan carries a falsifiable paper-parity benchmark.** A plan that
   adds or changes a clinical workflow — at any layer — has a `## Paper-parity benchmark (§1.2)`
   section: the named paper counterpart, the step count (paper *N* → architecture-forced *M* → UI
   bundling target *K*; **`M > N` is an architecture defect**, file it), and a time/cognitive-load
   budget whose measurement is owed by the slice that first exposes a runnable surface. A plan below
   the clinical surface writes one line instead: `Paper-parity: not clinical-surface — <reason>`.
   Enforced by `crates/cairn-node/tests/paper_parity_plan_section.rs`. Measurements are human acts,
   recorded via the `RUNBOOK.md` files under `*/results/`.
8. **Every working session ends in at least a draft PR — and begins by looking for one.** Work that
   exists only on a local branch is invisible, and has cost a full session of re-building a slice
   that was already finished. Push and open a PR (draft if incomplete or blocked, saying so in the
   body) before you stop. Before you start on anything `docs/HANDOVER.md` names as next, check the
   open PRs and recent branches: HANDOVER describes `main`, and the work may already exist elsewhere.

And the cross-cutting engineering rule from the [spec](../spec/language-substrate.md): **choose the
language by defect blast radius** — safety-critical → Rust or in-database (reviewer-legible, small);
fit-for-purpose → optimize for iteration. The integration boundary is the **PostgreSQL boundary**;
avoid FFI coupling. See
[Architecture for developers](architecture-for-developers.md#6-choosing-a-language-defect-blast-radius).

---

## The build loop: brainstorm → spec → plan → TDD → review

Cairn's slices are built with a deliberate **spec-driven development (SDD)** loop, and the artifacts
are kept in the repo so the reasoning is legible after the fact:

1. **Brainstorm** — pressure-test the idea against the principles and clinical realism. Surface
   flaws *early and plainly* (criticism is strongly encouraged here — see below).
2. **Spec** — write a design for the slice under `docs/superpowers/specs/` (dated). Larger efforts
   get one design that is then cut into numbered slices (e.g. the GUI sign-in design → slices 1, 2,
   3a, 3b-1, …), each with its own plan and PR.
3. **Plan** — break the slice into ordered TDD tasks under `docs/superpowers/plans/`, including the
   paper-parity section (house rule 7).
4. **TDD** — implement task by task, failing test first. A per-task review follows each task, and
   its fixes are their own commits (`fix(...): Task N review — …`).
5. **Review** — a whole-branch review before merge, and a review of the PR itself; fix findings or
   file issues (house rule 5).
6. **Record** — the load-bearing decisions become an ADR, the spec version is bumped in
   `docs/spec/index.md`, any new "do not undo this" rule gets a block in
   [Durable rules](durable-rules.md) with the test that pins it, the runbook gets the slice's human
   measurement section, and `docs/HANDOVER.md` / `docs/ROADMAP.md` are updated.

> [!NOTE]
> `docs/superpowers/` is working scaffolding and is **excluded from the published docs site**. Browse
> it directly in the repo for worked examples — every slice has a matching `specs/<date>-*.md` and
> `plans/<date>-*.md` you can read as a template.

If a change is load-bearing — it constrains the data model, the identity algebra, sync semantics,
security posture, or governance — it gets an **ADR** (context → decision → consequences), allocated
in order in `docs/spec/decisions/`, and the relevant `docs/spec/` aspect file is updated. The spec
carries **no in-file changelogs and no version-suffixed filenames** — git is the history and the
version lives in `spec/index.md`.

---

## Surface flaws early — criticism is the culture

This project explicitly wants you to **point out flaws, risks, and gaps plainly and immediately**,
with the specific failure scenario — in a design, a mechanism, someone else's idea, or your own.
Don't soften it into vagueness or wait to be asked. The maintainer is a front-line clinician who
often reviews while sleep-deprived and *wants* contributors to have their back: a mistake caught
early beats being bitten later. If an idea survives scrutiny, say so; if it has caveats, enumerate
them.

---

## Mechanics: branch → commit → PR

1. **Branch** off `main` for your change.
2. **Develop** with TDD; keep commits focused and the rationale tied to the principles. The history
   uses conventional prefixes scoped to the area: `feat(note): …`, `fix(window): …`,
   `docs(handover): …`.
3. **Sign off every commit:** `git commit -s`.
4. **Run the local gates** before pushing:
   - Rust (root workspace): `cargo fmt --all --check` and
     `cargo clippy --workspace --tests -- -D warnings`.
   - Anything touching the database, the floor or `cairn-node`: `scripts/run-db-gated-tests.sh`
     (needs PG ≥ 18 + `cairn_pgx`; see [Getting started](getting-started.md#database-gated-tests)).
   - `cairn_pgx`: `cargo pgrx test` in `extensions/cairn_pgx`.
   - The window: `cargo test --workspace` and clippy in `cairn-gui/`.
   - Matcher: `cd matcher && uv run ruff check . && uv run pytest` (and the `--extra pipeline`
     integration tests if you touched the pipeline).
   - Docs (if you touched any): `uv run --with-requirements docs/requirements.txt -- mkdocs build --strict`.
5. **Open a pull request** against `main` — as a **draft** while work or review is in progress.
   CI runs the checks listed in [Getting started → What CI runs](getting-started.md#8-what-ci-runs);
   the required ones are tabled in `CONTRIBUTING.md`. Expect review focused on principle-alignment,
   clinical realism, and — for safety-critical surfaces — reviewer-legibility. Small, well-reasoned
   changes merge faster than large ones.
6. **Mind closing keywords.** GitHub closes an issue when `fixes`/`closes`/`resolves` sits next to a
   reference — in the body, the **title** or a commit message — even in a sentence saying the
   opposite (`it does not fix #500` closes #500). The closing-keyword guard refuses such a PR; the
   root `CONTRIBUTING.md` shows how to word around it.

---

## Clinical case-mining is a first-class contribution

**You do not need to write code to contribute.** A well-described front-line failure mode is often
*more* valuable than a patch — it is the single most productive design activity in the project's
history. The architecture's primitives have so far absorbed every real case without needing new
architecture, and finding that out is itself a useful result; occasionally a case surfaces a new ADR.
Worked examples live in [`docs/case-studies/`](../case-studies/README.md).

A good case includes:

- **the workflow and its setting** (what the clinician is actually doing, where);
- **its paper-era counterpart** (how it worked on paper);
- **exactly where it breaks** — in time, steps, cognitive load, or safety;
- **what the honest outcome should be.**

Open an issue describing it. No code required.

A related non-code contribution: **running a measurement or accessibility pass.** Each slice's
paper-parity budget and screen-reader pass is a human act, scripted step by step in
`cairn-gui/cairn-gui-tauri/results/RUNBOOK.md`; several sections are owed and waiting for someone to
run them.

---

## Picking your first task

- **Read `docs/HANDOVER.md` first** — its ⇒ NEXT block names the live goal, and its "Open threads —
  pick one" menu is the curated list of what's ready (desk-doable now vs. blocked on hardware or
  external access). Then check the open PRs (house rule 8).
- **The live build front** moves; HANDOVER names it. In November 2026 it was the **GUI milestone** —
  sign in, find or register a patient, demographics, a progress note, an allergy, a charted
  medication, sign off, sign out — built as numbered sign-in/authorship slices on the reference
  window and the note stream. The [Codebase tour](codebase-tour.md) walks the code under it.
- **Good first contributions** that don't require deep context: clinical case-mining (above); a
  runbook pass (above); docs fixes (including to *this manual* when it drifts); and the small
  recorded follow-ups filed as GitHub issues, many of which name the exact file and test.
- **Check [`docs/ROADMAP.md`](../ROADMAP.md)** for where a prospective change sits in the build order,
  and the [ADR log](../spec/decisions/README.md) for whether the question is already settled.

> [!IMPORTANT]
> **Security issues are reported privately.** Cairn is a health record — defects can affect patient
> safety and confidentiality. Do **not** open a public issue for a suspected vulnerability; report it
> per [Governance §9](../principles/GOVERNANCE.md#9-responsible-disclosure-security).
