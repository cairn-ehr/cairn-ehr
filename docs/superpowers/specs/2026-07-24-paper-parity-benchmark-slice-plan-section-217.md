# Paper-parity benchmark as a required slice-plan section (#217) — design

**Issue:** [#217](https://github.com/cairn-ehr/cairn-ehr/issues/217) (2026-07-15 review, finding I9/G).
**Date:** 2026-07-24. **Tier:** process/tooling (no spec/ADR/wire/SCHEMA change).

## Problem

`vision.md` §1.2 makes paper-parity **normative and falsifiable**: *every clinical workflow must
name its paper-era equivalent and benchmark against it in time, steps, and cognitive load; a workflow
that loses to paper is a design defect and is tracked as one.* Nothing built so far has been recorded
in that form. Paper-parity appears as a prose aside in ~9 existing plans (e.g. "Pi input-to-paint
latency — paper-parity floor") but never as the named-counterpart + benchmark the spec mandates. The
review's phrase: it is *"enforced by taste, not by the falsifiable form the spec mandates."*

The accumulating ceremony stack (enrolment + shown-once recovery code, per-verb `--attest-as`,
per-*thread* attestation staleness under ADR-0049) is exactly the kind of thing §1.2 exists to catch,
and it currently escapes the falsifiable form.

## What this changes (scope)

A **process + tooling** change only. It binds **plan documents** (`docs/superpowers/plans/*.md`),
forward-only from the day it lands. It does **not** touch the spec, any ADR, the wire, or SCHEMA, and
it does **not** retrofit the 67 existing plans (they are the historical record of what we did —
rewriting them would be a "never erase" violation in miniature).

Explicitly **out of scope**: binding `docs/superpowers/specs/` (the issue says "slice plan"; one
obligation in one place beats two half-enforced ones); writing the Tauri client slice plan (its own
brainstorm→spec→plan cycle).

## Design decisions (settled in brainstorming)

1. **Which slices are bound — all layers (broadest cut).** Any slice adding or changing a clinical
   workflow at *any* layer, including the in-DB floor and event core. Rationale: the review's worked
   example (ADR-0049's per-thread attestation) is a **floor** decision that dictates how many gestures
   a UI must later demand — a below-the-UI slice is exactly where an un-recorded parity obligation is
   born. A UI-only cut would miss the real offender.

2. **The form is falsifiable by limb, not deferred wholesale.** §1.2's three limbs are not equally
   knowable at plan time: **steps/gestures are countable from the design itself** (no running code
   needed), while **time and cognitive load need a runnable workflow**. So the section states the
   step-count as a *binding claim now* and time/cognitive-load as a *declared budget* whose
   measurement is owed (and named) by the slice that introduces a runnable surface. Declaring a budget
   we cannot yet measure — rather than fabricating a number — is **principle 4 (acknowledged
   uncertainty) applied to our own process**.

3. **Steps are judged on what the architecture *forecloses*, not on rendered gestures** (correction
   from review of this design). Gesture bundling is a **layer-4 (UI) / layer-3 (policy)** responsibility
   under ADR-0021; the architecture's duty is *negative* ("must not foreclose bundling") and *positive*
   ("should promote it"). So the Steps field reads:

   > **Steps (paper → Cairn):** paper *N* human acts → architecture **forces** *M* (the floor no UI can
   > bundle away) → UI bundling target *K*.

   with the consequence split by fault layer:
   - `M > N` ⇒ **architecture** defect (the design forced more human acts than paper) — file an issue,
     per §1.2 "tracked as one" and house rule 5.
   - `M ≤ N` but a UI exposes more than K ⇒ **UI** defect, tracked against that UI slice.

   Worked example (verified against the code, not assumed): the whole-list medication sign-off. Paper
   `N = 1` (one signature on one form). ADR-0049 composes the list from N thread attestations, but
   `attest_thread_in_tx` takes an **already-unsealed** human key by reference — the caller unseals once
   (`load_attester_key`) and an orchestrator can loop over N threads in a single transaction. The N
   *signatures* are a cryptographic artifact of the set-commitment model, not N human acts. So the
   architecture **forces `M = 1`** and *permits* one-gesture bundling; there is **no architecture
   defect**. The gap is that no UI yet exercises the bundling — a UI-slice obligation, not an
   architecture one.

4. **Enforcement is three layers**, because a rule that is merely written down reproduces the
   taste-based enforcement the issue filed against:
   - **`CONTRIBUTING.md`** — the canonical rule text (form, when it binds, the escape, the consequence).
   - **`CLAUDE.md`** — a one-line pointer in the house rules, so the rule is in context while a plan is
     being *written*, not discovered at review.
   - **CI** — a Rust source-guard test in `crates/cairn-node/tests/`, running inside the existing
     `rust.yml` gate with no new workflow wiring (mirrors `name_winner_order_drift.rs`, the established
     no-DB source-guard idiom).

5. **The escape hatch is a forced-rationale gate, not a checkbox** — §1.2's own permitted friction
   applied to our process. A plan below the clinical surface takes the section with a single line:

   > `Paper-parity: not clinical-surface — <substantive recorded reason>`

   A missing section *and* a missing escape line ⇒ CI fails. The reason cannot be click-throughed
   (see the substantive-reason proxy below).

## The required section (the form authors write)

A bound plan carries, in its `## Global Constraints` block or as a top-level section:

```markdown
## Paper-parity benchmark (§1.2)

- **Paper counterpart:** <named concretely — e.g. "the drug chart: one signature, one form, one act">
- **Steps (paper → Cairn):** paper N human acts → architecture forces M → UI bundling target K.
  <If M > N: "FAILS parity (architecture) → tracked as #NNN.">
- **Time + cognitive load:** budget — <e.g. "re-attest a 6-thread list in ≤ 1 gesture, ≤ 2 s">.
  Unmeasured (no runnable surface); measurement owed by <the slice that introduces one>.
```

or the forced-rationale escape, one line:

```markdown
Paper-parity: not clinical-surface — <substantive recorded reason, ≥ 30 chars>.
```

## The CI guard — mechanics

Lands as `crates/cairn-node/tests/paper_parity_plan_section.rs`. It resolves the plans directory at
runtime as `env!("CARGO_MANIFEST_DIR").join("../../docs/superpowers/plans")` — two levels up from the
crate root — the same idiom as `twin_dispatch_single_source.rs`'s `db_dir()`. No database; runs in
every `cargo test` and CI pass.

**Pure helpers (unit-tested off synthetic strings, house rule 1):**

- `plan_date(filename) -> Result<Date, Err>` — parse the leading `YYYY-MM-DD` from a plan filename. A
  filename that does not parse is a **loud error naming the file**, never a silent skip (silent-ignore
  would be a free escape hatch; the convention is already 67/67).
- `classify_declaration(contents) -> Declaration` — one of:
  - `Benchmark` — has the `Paper-parity benchmark` heading **and** all three field labels
    (`Paper counterpart`, `Steps`, `Time + cognitive load` / their agreed tokens);
  - `NotClinical { reason }` — has the `Paper-parity: not clinical-surface — …` line;
  - `Missing` — neither.
- `verdict(date, cutoff, declaration) -> Verdict` — a plan dated `< cutoff` is `Exempt` (forward-only);
  a plan dated `≥ cutoff` passes on `Benchmark`, or `NotClinical` **with a substantive reason**
  (`reason.chars().count() >= 30` — Unicode scalar count, not byte length, so a reason in non-ASCII
  script is measured fairly), else `Fail`.

**Substantive-reason proxy — stated honestly.** The `≥ 30 chars` floor is a *crude* proxy. It defeats
silence and one-word checkboxes; it **cannot** detect bad faith — human/agent review does that. Its
real value is making an absence of thought visible in the diff.

**Filesystem test** over the real plans directory: every plan dated `≥ cutoff` must not be `Fail`.

**Anti-vacuity (the one real weakness, mitigated).** Forward-only means that on landing day *zero*
plans are bound, so the filesystem test passes **vacuously** — a guard green because it checks nothing
is worse than none. So `classify_declaration`/`verdict` are additionally pinned by **fixture tests**
over embedded known-good and known-bad plan strings (a compliant benchmark section; a valid escape; a
too-short escape reason; a `Missing` plan dated after the cutoff must `Fail`), independent of whether
any real plan is bound yet. This is slice 37's honesty-canary discipline.

**Cutoff constant.** A single `const CUTOFF` (the date this lands) documented in the test as the
forward-only boundary. Chosen as a fixed date, not "today", so the guard is deterministic and does not
change verdict as the clock moves.

## The tracked parity artifact (#217's "starting with the Tauri client")

File one GitHub issue capturing the med-list sign-off obligation, **correctly scoped as a UI-slice
obligation** (not an architecture defect, per decision 3):

> The reference med-list UI must collapse whole-list sign-off into **one human gesture**. The
> architecture already permits it — a human key is unsealed once and an orchestrator signs N thread
> commitments in one transaction (`attest_thread_in_tx` takes an already-unsealed key); the N
> signatures are a cryptographic artifact of ADR-0049's set-commitment model, not N human acts. The
> deferred ADR-0049 "list reviewed at T" summary event is the record-layer promotion path if a
> worklist wants one. Owed by the Tauri med-list slice; this is the first live entry of the #217 rule.

## TDD order

1. **RED** — `paper_parity_plan_section.rs`: helper unit tests + fixtures (incl. a deliberately
   non-compliant fixture that must `Fail`) + the filesystem test. Fails to compile / fails assertions
   first.
2. **GREEN** — the pure helpers, minimal.
3. `CONTRIBUTING.md` — the canonical rule text.
4. `CLAUDE.md` — the one-line house-rule pointer.
5. File the UI-slice parity issue (the tracked artifact).
6. HANDOVER / ROADMAP — record the slice; note #217 closed, review course fully closed.

## Success criteria

- A new plan dated on/after the cutoff with neither the benchmark section nor a substantive escape
  line **fails CI**.
- The same plan with either passes.
- The 67 existing plans are untouched and the guard does not flag them.
- The guard is not vacuous: the classifier/verdict fixtures fail if the parsing logic regresses, even
  with zero bound real plans.
- `CONTRIBUTING.md` states the form; `CLAUDE.md` points to it; the UI-slice parity obligation is a
  filed issue.

## Non-goals

- No spec/ADR/wire/SCHEMA change.
- No retrofit of existing plans or specs.
- No binding of `docs/superpowers/specs/`.
- No attempt to *measure* time/cognitive-load in this slice (no runnable clinical surface exists yet);
  the section declares budgets and names who owes the measurement.
- No semantic judgement of "substantive" beyond a length proxy (review's job).
