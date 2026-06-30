# Contributing workflow

How a change is actually made in Cairn — the mechanics, the disciplines, and how to pick your first
task. This page is the practical companion to the authoritative
[Governance & Contributing](../principles/GOVERNANCE.md); where this page is brief on a *rule* (the
licence, the DCO, how decisions are recorded), Governance is the source of truth.

---

## The non-negotiables (read once, internalize)

- **AGPL-3.0, inbound = outbound.** Everything you contribute is under the
  [AGPL-3.0](../principles/GOVERNANCE.md#4-licensing-and-contributor-terms). **Every dependency you
  add must be AGPL-3.0-compatible — check its licence *before* adding it.** An incompatible licence is
  a blocker, not a cleanup-later item.
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

These apply to **all** code, spikes included (they are in `CLAUDE.md` too):

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

And the cross-cutting engineering rule from the [spec](../spec/language-substrate.md): **choose the
language by defect blast radius** — safety-critical → Rust or in-database (reviewer-legible, small);
fit-for-purpose → optimize for iteration. The integration boundary is the **PostgreSQL boundary**;
avoid FFI coupling. See
[Architecture for developers](architecture-for-developers.md#5-choosing-a-language-defect-blast-radius).

---

## The build loop: brainstorm → spec → plan → TDD

Cairn's slices are built with a deliberate **spec-driven development (SDD)** loop, and the artifacts
are kept in the repo so the reasoning is legible after the fact:

1. **Brainstorm** — pressure-test the idea against the four principles and clinical realism. Surface
   flaws *early and plainly* (criticism is strongly encouraged here — see below).
2. **Spec** — write a design spec for the slice. These live under `docs/superpowers/specs/`
   (dated, one per slice).
3. **Plan** — break it into ordered TDD tasks. These live under `docs/superpowers/plans/`.
4. **TDD** — implement task by task, failing test first.
5. **Review** — a whole-branch review before merge; fix findings or file issues.

> [!NOTE]
> `docs/superpowers/` is working scaffolding and is **excluded from the published docs site**. Browse
> it directly in the repo for worked examples — every demographics slice and node feature has a
> matching `specs/<date>-*.md` and `plans/<date>-*.md` pair you can read as a template.

If a change is load-bearing — it constrains the data model, the identity algebra, sync semantics,
security posture, or governance — it also gets an **ADR** (context → decision → consequences),
allocated in order in `docs/spec/decisions/`, and the relevant `docs/spec/` aspect file is updated.
The spec carries **no in-file changelogs and no version-suffixed filenames** — git is the history and
the version lives in `spec/index.md`.

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

1. **Branch** off the default branch (`main`) for your change.
2. **Develop** with TDD; keep commits focused and the rationale tied to the principles.
3. **Sign off every commit:** `git commit -s`.
4. **Run the local gates** before pushing (there is no code CI yet — see
   [Getting started → What CI runs today](getting-started.md#7-what-ci-runs-today)):
   - Rust: `cargo test --workspace` and `cargo clippy --workspace --all-targets` (clean).
   - In-DB / `cairn_pgx`: `cargo pgrx test`; the DB-gated `cairn-node` tests with `CAIRN_TEST_PG` set.
   - Matcher: `cd matcher && uv run pytest` (and the `--extra pipeline` integration tests if you
     touched the pipeline).
   - Docs (if you touched any): `uv run --with-requirements docs/requirements.txt -- mkdocs build --strict`.
5. **Open a pull request** against `main` with a clear rationale. Expect review focused on
   principle-alignment, clinical realism, and — for safety-critical surfaces — reviewer-legibility.
   Small, well-reasoned changes merge faster than large ones.

---

## Clinical case-mining is a first-class contribution

**You do not need to write code to contribute.** A well-described front-line failure mode is often
*more* valuable than a patch — it is the single most productive design activity in the project's
history. The architecture's primitives have so far absorbed every real case without needing new
architecture, and finding that out is itself a useful result; occasionally a case surfaces a new ADR.

A good case includes:

- **the workflow and its setting** (what the clinician is actually doing, where);
- **its paper-era counterpart** (how it worked on paper);
- **exactly where it breaks** — in time, steps, cognitive load, or safety;
- **what the honest outcome should be.**

Open an issue describing it. No code required.

---

## Picking your first task

- **Read `docs/HANDOVER.md` first** — its "Open threads — pick one" menu is the live, curated list of
  what's ready to work on (desk-doable now vs. blocked on hardware/external access), kept current each
  session.
- **The live build front** is the demographics subsystem and the §5.2 matcher on `cairn-node` — new
  slices reuse the spine in `db/010`–`db/017` and `cairn-event::demographics`. The
  [Codebase tour](codebase-tour.md) walks that spine.
- **Good first contributions** that don't require deep context: clinical case-mining (above); docs
  fixes (including to *this manual* when it drifts); adding the missing **code CI** (`cargo
  test`/`clippy`, `uv run pytest`); and the small recorded follow-ups filed as GitHub issues.
- **Check [`docs/ROADMAP.md`](../ROADMAP.md)** for where a prospective change sits in the build order,
  and the [ADR log](../spec/decisions/README.md) for whether the question is already settled.

> [!IMPORTANT]
> **Security issues are reported privately.** Cairn is a health record — defects can affect patient
> safety and confidentiality. Do **not** open a public issue for a suspected vulnerability; report it
> per [Governance §9](../principles/GOVERNANCE.md#9-responsible-disclosure-security).
