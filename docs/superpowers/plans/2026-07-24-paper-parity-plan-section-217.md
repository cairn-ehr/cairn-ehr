# Paper-parity Benchmark Slice-Plan Section (#217) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make vision.md §1.2 paper-parity a *falsifiable, enforced* required section of every clinical-surface slice plan — closing the last open item of the 2026-07-15 review course (#217).

**Architecture:** A process+tooling slice, no spec/ADR/wire/SCHEMA change. Three enforcement layers — canonical rule text in `CONTRIBUTING.md`, a one-line pointer in `CLAUDE.md` (so the rule is in context while a plan is *written*), and a no-DB Rust source-guard test in `crates/cairn-node/tests/` that rides the existing `rust.yml` `test` job. The guard scans `docs/superpowers/plans/*.md`, forward-only from a fixed cutoff, and fails any bound plan carrying neither the falsifiable benchmark section nor a substantive forced-rationale "not clinical-surface" escape line. Plus one filed GitHub issue capturing the med-list sign-off as a UI-slice obligation.

**Tech Stack:** Rust (std::fs only, no new deps), Markdown docs, `gh` CLI.

Design: [`docs/superpowers/specs/2026-07-24-paper-parity-benchmark-slice-plan-section-217.md`](../specs/2026-07-24-paper-parity-benchmark-slice-plan-section-217.md).

## Global Constraints

Paper-parity: not clinical-surface — a CI-guard + docs process slice; it ships no clinician-reachable clinical workflow, so §1.2's time/steps/cognitive-load benchmark does not apply to this plan itself.

- **AGPL-3.0**; no new dependencies (std only in the guard test).
- **TDD:** failing test first (compile-failure RED for the pure helpers), then minimal code. House rules 1 (pure reusable functions), 3 (junior-legible inline docs), 4 (readability over cleverness).
- **No DB:** the guard is a pure source-level `#[test]` in `crates/cairn-node/tests/` — it needs no `CAIRN_TEST_PG` and runs in plain `cargo test` and in the existing `rust.yml` `test` job (`cargo clippy -D warnings` + `cargo test --workspace`). **No new workflow wiring.** This mirrors `crates/cairn-node/tests/twin_dispatch_single_source.rs` and `name_winner_order_drift.rs` — read them first.
- **Repo-dir path idiom** (verbatim from `twin_dispatch_single_source.rs`): `PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../<repo-relative>").canonicalize()`. `CARGO_MANIFEST_DIR` is `crates/cairn-node`; the repo root is two levels up.
- **Forward-only cutoff:** `const CUTOFF: (i32,u32,u32) = (2026, 7, 24)` — a FIXED date, never "today", so the verdict is deterministic. Plans dated strictly before it are exempt (they are the historical record; retrofitting them would be the "never erase" violation in miniature, principle 2).
- **Steps limb is judged on what the architecture *forecloses*, not on rendered gestures** (ADR-0021: gesture bundling is a layer-3/4 UI/policy job; the architecture must merely not foreclose it). `M > N` is an *architecture* defect; `M ≤ N` but a UI exposing more is a *UI* defect.
- **Pre-commit gates** (run before every commit that touches Rust): `cargo fmt --all`, then `cargo clippy --workspace --all-targets -- -D warnings`, then `cargo test --workspace`. Docs-only commits skip clippy/test but must keep `mkdocs build --strict` green if they touch `docs/spec/` (this slice does not).

---

## Task 1: The source-guard test + pure classifier

**Files:**
- Create: `crates/cairn-node/tests/paper_parity_plan_section.rs`
- Reference (read first, do not modify): `crates/cairn-node/tests/twin_dispatch_single_source.rs`

**Interfaces:**
- Produces (used only within this file): `plan_date(&str) -> Result<(i32,u32,u32), String>`, `classify_declaration(&str) -> Declaration`, `verdict((i32,u32,u32), (i32,u32,u32), &Declaration) -> Verdict`, plus `enum Declaration { Benchmark, NotClinical { reason: String }, Missing }` and `enum Verdict { Exempt, Pass, Fail(String) }`.
- Consumes: nothing from other tasks.

- [ ] **Step 1: Write the failing helper + fixture tests (RED)**

Create `crates/cairn-node/tests/paper_parity_plan_section.rs` with **only** the module doc, the `use` lines, the consts, and the tests below — the three helper fns and two enums are *referenced but not yet defined*, so the crate fails to compile (this is the RED):

```rust
//! #217 — the paper-parity benchmark is a REQUIRED section of every clinical-surface slice plan.
//!
//! vision.md §1.2 makes paper-parity normative and falsifiable: "every clinical workflow must name
//! its paper-era equivalent and benchmark against it in time, steps, and cognitive load ... a
//! workflow that loses to paper is a design defect and is tracked as one." Before this guard the
//! rule was enforced by taste (2026-07-15 review, finding I9/G). This is a SOURCE-LEVEL guard (no
//! DB): it scans docs/superpowers/plans/*.md and fails if any plan dated on/after the cutoff carries
//! neither the falsifiable benchmark section NOR the forced-rationale "not clinical-surface" escape
//! line. It runs in every `cargo test` / CI pass (it needs no database).
//!
//! FORWARD-ONLY: plans dated before the cutoff are exempt — they are the historical record of what
//! we built, and retrofitting them would be the "never erase" violation in miniature (principle 2).
//! The guard binds the Tauri client slice and everything after it.
//!
//! ANTI-VACUITY: on landing day almost no plan is bound, so the directory scan alone could pass
//! while checking little. The classifier/verdict are therefore ALSO pinned by fixture tests over
//! synthetic known-good / known-bad plan strings (below), which fail if the parsing logic regresses
//! regardless of how many real plans are bound.

use std::fs;
use std::path::PathBuf;

/// The forward-only boundary as (year, month, day). Plans whose filename date is strictly BEFORE
/// this are exempt. A FIXED date (not "today") so the guard's verdict is deterministic and never
/// shifts as the clock moves. Set to the day the #217 rule landed.
const CUTOFF: (i32, u32, u32) = (2026, 7, 24);

/// The heading a compliant benchmark section carries (matched as a substring, so the trailing
/// "(§1.2)" and the markdown "## " prefix are irrelevant).
const BENCHMARK_HEADING: &str = "Paper-parity benchmark";

/// The three §1.2 limb labels a compliant section must name.
const FIELD_LABELS: [&str; 3] = ["Paper counterpart", "Steps", "Time + cognitive load"];

/// The forced-rationale escape: a plan below the clinical surface states this, with a substantive
/// reason, instead of the benchmark section. Matched as a substring anywhere in the plan.
const ESCAPE_PREFIX: &str = "Paper-parity: not clinical-surface";

/// Minimum length (unicode scalar values) of the escape's reason. A CRUDE proxy: it defeats silence
/// and one-word checkboxes but cannot detect bad faith — human/agent review does that. Its value is
/// making an absence of thought visible in the diff.
const MIN_REASON_LEN: usize = 30;

// ---- pure-helper unit tests (no filesystem) ----

#[test]
fn plan_date_parses_a_conventional_name() {
    assert_eq!(plan_date("2026-07-24-paper-parity.md"), Ok((2026, 7, 24)));
}

#[test]
fn plan_date_rejects_a_malformed_name() {
    assert!(plan_date("draft-notes.md").is_err());
    assert!(plan_date("2026_07_24-x.md").is_err());
    assert!(plan_date("26-7-2-x.md").is_err());
}

#[test]
fn classify_recognises_a_full_benchmark_section() {
    let plan = "## Paper-parity benchmark (§1.2)\n\
                - Paper counterpart: the drug chart, one signature\n\
                - Steps: paper 1 -> architecture 1\n\
                - Time + cognitive load: budget <= 2 s\n";
    assert_eq!(classify_declaration(plan), Declaration::Benchmark);
}

#[test]
fn classify_recognises_the_escape_line() {
    let plan = "## Global Constraints\n\
                Paper-parity: not clinical-surface — pure sync-cursor bookkeeping, no workflow.\n";
    match classify_declaration(plan) {
        Declaration::NotClinical { reason } => assert!(reason.starts_with("pure sync-cursor")),
        other => panic!("expected NotClinical, got {other:?}"),
    }
}

#[test]
fn classify_reports_missing_when_neither_present() {
    assert_eq!(classify_declaration("## Task 1\nsome plan text"), Declaration::Missing);
}

#[test]
fn verdict_exempts_a_pre_cutoff_plan_even_if_missing() {
    assert_eq!(verdict((2026, 7, 13), CUTOFF, &Declaration::Missing), Verdict::Exempt);
}

#[test]
fn verdict_passes_a_bound_benchmark() {
    assert_eq!(verdict((2026, 7, 24), CUTOFF, &Declaration::Benchmark), Verdict::Pass);
}

#[test]
fn verdict_passes_a_bound_substantive_escape() {
    let reason = "pure sync-cursor bookkeeping, no clinician-reachable workflow".to_string();
    assert_eq!(
        verdict((2026, 8, 1), CUTOFF, &Declaration::NotClinical { reason }),
        Verdict::Pass
    );
}

#[test]
fn verdict_fails_a_bound_too_short_escape() {
    let reason = "n/a".to_string();
    assert!(matches!(
        verdict((2026, 8, 1), CUTOFF, &Declaration::NotClinical { reason }),
        Verdict::Fail(_)
    ));
}

#[test]
fn verdict_fails_a_bound_missing_declaration() {
    assert!(matches!(
        verdict((2026, 7, 24), CUTOFF, &Declaration::Missing),
        Verdict::Fail(_)
    ));
}
```

- [ ] **Step 2: Run to verify RED**

Run: `cargo test -p cairn-node --test paper_parity_plan_section 2>&1 | tail -20`
Expected: FAIL to compile — `cannot find function 'plan_date'`, `cannot find type 'Declaration'`, etc.

- [ ] **Step 3: Implement the pure helpers (GREEN for the unit tests)**

Append to the same file, *above* the tests (so the module reads top-down: doc → consts → helpers → tests). Move the `#[test]` block below these definitions if your editor added them after:

```rust
/// Repo-root plans directory. CARGO_MANIFEST_DIR is crates/cairn-node; the plans live two levels up.
/// Same idiom as twin_dispatch_single_source.rs's `db_dir()`.
fn plans_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/superpowers/plans")
        .canonicalize()
        .expect("docs/superpowers/plans/ dir")
}

/// Parse the leading `YYYY-MM-DD` of a plan filename into (year, month, day).
///
/// Every plan follows the `YYYY-MM-DD-<slug>.md` convention (67/67 at write time). A filename that
/// does not is a LOUD `Err` naming the file — never a silent skip, which would be a free way to dodge
/// the guard by mis-naming a plan. Operates on bytes: the prefix is ASCII, and byte checks avoid any
/// char-boundary panic on an unexpected multibyte name.
fn plan_date(filename: &str) -> Result<(i32, u32, u32), String> {
    let b = filename.as_bytes();
    let shape_ok = b.len() >= 11
        && b[0..4].iter().all(u8::is_ascii_digit)
        && b[4] == b'-'
        && b[5..7].iter().all(u8::is_ascii_digit)
        && b[7] == b'-'
        && b[8..10].iter().all(u8::is_ascii_digit)
        && b[10] == b'-';
    if !shape_ok {
        return Err(format!(
            "plan filename does not start with YYYY-MM-DD-: {filename:?} \
             (the convention is required so the #217 guard can tell which plans are bound)"
        ));
    }
    // Positions 0..10 are verified ASCII digits/dashes, so these slices and parses cannot fail.
    let y = filename[0..4].parse().expect("4 ascii digits");
    let m = filename[5..7].parse().expect("2 ascii digits");
    let d = filename[8..10].parse().expect("2 ascii digits");
    Ok((y, m, d))
}

/// How a plan declares its paper-parity posture.
#[derive(Debug, PartialEq, Eq)]
enum Declaration {
    /// The benchmark heading + all three field labels are present.
    Benchmark,
    /// The forced-rationale escape line is present; `reason` is the text after the prefix.
    NotClinical { reason: String },
    /// Neither.
    Missing,
}

/// The reason text following the escape prefix on its line, with leading dashes / em-dash / colon /
/// whitespace stripped. `None` if the plan has no escape line. Forgiving of `—`, `--`, or `:` as the
/// separator so an author is not tripped by punctuation choice.
fn escape_reason(contents: &str) -> Option<String> {
    for line in contents.lines() {
        if let Some(idx) = line.find(ESCAPE_PREFIX) {
            let after = &line[idx + ESCAPE_PREFIX.len()..];
            let reason = after
                .trim_start_matches(|c: char| {
                    c == '-' || c == '\u{2014}' || c == ':' || c.is_whitespace()
                })
                .trim();
            return Some(reason.to_string());
        }
    }
    None
}

/// Classify a plan's paper-parity declaration. A full benchmark section wins over an escape line if
/// (contradictorily) both are present — a plan that did the benchmark is a clinical-surface plan.
/// The classifier is STRUCTURAL, not semantic: a plan that merely quotes the empty template trips
/// `Benchmark`; catching that is review's job, not the guard's (see the module doc).
fn classify_declaration(contents: &str) -> Declaration {
    let has_benchmark = contents.contains(BENCHMARK_HEADING)
        && FIELD_LABELS.iter().all(|label| contents.contains(label));
    if has_benchmark {
        return Declaration::Benchmark;
    }
    match escape_reason(contents) {
        Some(reason) => Declaration::NotClinical { reason },
        None => Declaration::Missing,
    }
}

/// The final verdict for one plan.
#[derive(Debug, PartialEq, Eq)]
enum Verdict {
    /// Dated before the cutoff — forward-only exemption.
    Exempt,
    /// Bound and compliant.
    Pass,
    /// Bound and non-compliant; the string explains why (surfaced in the panic).
    Fail(String),
}

/// Decide a plan's verdict from its date and declaration. Pure: no filesystem, so it is exhaustively
/// unit-tested, independent of which real plans exist.
fn verdict(
    date: (i32, u32, u32),
    cutoff: (i32, u32, u32),
    declaration: &Declaration,
) -> Verdict {
    if date < cutoff {
        return Verdict::Exempt;
    }
    match declaration {
        Declaration::Benchmark => Verdict::Pass,
        Declaration::NotClinical { reason } if reason.chars().count() >= MIN_REASON_LEN => {
            Verdict::Pass
        }
        Declaration::NotClinical { reason } => Verdict::Fail(format!(
            "'not clinical-surface' reason is too short ({} chars; need >= {MIN_REASON_LEN}): {reason:?}",
            reason.chars().count()
        )),
        Declaration::Missing => Verdict::Fail(
            "no '## Paper-parity benchmark (§1.2)' section and no \
             'Paper-parity: not clinical-surface — <reason>' escape line"
                .to_string(),
        ),
    }
}
```

- [ ] **Step 4: Run unit tests to verify GREEN**

Run: `cargo test -p cairn-node --test paper_parity_plan_section 2>&1 | tail -20`
Expected: all 10 unit/fixture tests PASS (the fs test does not exist yet).

- [ ] **Step 5: Add the filesystem guard over the real plans directory**

Append this test to the `#[test]` block:

```rust
// ---- the filesystem guard over the real plans directory ----

/// Every plan dated on/after the cutoff must be `Pass` or `Exempt`, never `Fail`. This is the live
/// enforcement; the fixture tests above keep it from passing vacuously if the classifier regresses.
#[test]
fn every_bound_plan_declares_paper_parity() {
    let mut failures: Vec<String> = Vec::new();
    for entry in fs::read_dir(plans_dir()).expect("read plans dir") {
        let path = entry.expect("dir entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        // A mis-named .md is a loud failure, not a silent skip.
        let date = plan_date(&name).expect("plan filename must be YYYY-MM-DD-<slug>.md");
        let contents = fs::read_to_string(&path).expect("read plan");
        if let Verdict::Fail(why) = verdict(date, CUTOFF, &classify_declaration(&contents)) {
            failures.push(format!("{name}: {why}"));
        }
    }
    assert!(
        failures.is_empty(),
        "plans dated on/after the #217 cutoff must carry a paper-parity benchmark section \
         (or a substantive 'not clinical-surface' escape line). Offenders:\n{}",
        failures.join("\n")
    );
}
```

- [ ] **Step 6: Run the whole file + fmt + clippy**

Run: `cargo test -p cairn-node --test paper_parity_plan_section 2>&1 | tail -20`
Expected: all 11 tests PASS. This slice's own plan (`2026-07-24-paper-parity-plan-section-217.md`, dated == cutoff) is bound and passes: it quotes the template so `classify_declaration` returns `Benchmark` (and it *also* carries the escape line in its Global Constraints for human readers — `Benchmark` wins, both are fine).

Run: `cargo fmt --all && cargo clippy -p cairn-node --all-targets -- -D warnings 2>&1 | tail -15`
Expected: no diff from fmt on the new file; clippy clean.

- [ ] **Step 7: Commit**

```bash
git add crates/cairn-node/tests/paper_parity_plan_section.rs
git commit -s -m "test(#217): source-guard — clinical-surface plans must carry the §1.2 paper-parity section

No-DB source guard (mirrors twin_dispatch_single_source.rs) scanning
docs/superpowers/plans/*.md forward-only from a fixed 2026-07-24 cutoff. Pure
plan_date/classify_declaration/verdict helpers, exhaustively unit-tested plus
fixture-pinned against vacuity, and a filesystem test over the real plans dir.
Rides the existing rust.yml test job; no new workflow wiring.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 2: The canonical rule text (CONTRIBUTING.md) + the CLAUDE.md pointer

**Files:**
- Modify: `CONTRIBUTING.md` (add a section after the CI section, ends at line 50)
- Modify: `CLAUDE.md` (add house rule 7 after the current rule 6, ~line 189)

**Interfaces:** none (prose).

- [ ] **Step 1: Add the rule section to CONTRIBUTING.md**

Append after the final line of `CONTRIBUTING.md` (after the "See [GOVERNANCE.md] …" paragraph):

```markdown

## Paper-parity benchmark — a required slice-plan section

Paper-parity is the [governing law](docs/spec/vision.md#12-the-paper-parity-test-normative): §1.2
makes it **falsifiable** — *every clinical workflow must name its paper-era equivalent and benchmark
against it in time, steps, and cognitive load; a workflow that loses to paper is a design defect and
is tracked as one.* To keep that from being enforced by taste, **every slice plan for a slice that
adds or changes a clinical workflow — at any layer, the in-DB floor and event core included — must
carry a Paper-parity benchmark section:**

```markdown
## Paper-parity benchmark (§1.2)

- **Paper counterpart:** <named concretely — e.g. "the drug chart: one signature, one form, one act">
- **Steps (paper → Cairn):** paper N human acts → architecture forces M → UI bundling target K.
  <If M > N: "FAILS parity (architecture defect) → tracked as #NNN.">
- **Time + cognitive load:** budget — <e.g. "re-attest a 6-thread list in ≤ 1 gesture, ≤ 2 s">.
  Unmeasured (no runnable surface); measurement owed by <the slice that first exposes one>.
```

Three things make this honest rather than ceremonial:

- **Steps are judged on what the architecture *forecloses*, not on rendered gestures.** Bundling N
  events into one human gesture is a UI/policy job ([ADR-0021](docs/spec/decisions/0021-layering-the-node-api-and-ui-pluralism.md));
  the architecture's duty is only to *not foreclose* it (and ideally promote it). So `M` is the human
  acts the design **forces** — the floor no UI can bundle away. **`M > N` is an architecture defect**
  (file an issue, per §1.2 and house rule 5). `M ≤ N` but a UI exposing more than `K` is a **UI**
  defect, tracked against that UI slice.
- **Only the step-count is binding at plan time.** Steps are countable from the design; *time* and
  *cognitive load* need a runnable workflow. So the section states a step-count claim now and a
  time/load *budget* now, with the measurement owed (and named) by the first slice that ships a
  runnable surface. Declaring a budget we cannot yet measure — rather than fabricating a number — is
  acknowledged uncertainty (principle 4) applied to our own process.
- **Below-the-clinical-surface plans take a forced-rationale escape,** not a checkbox. One line:

  ```markdown
  Paper-parity: not clinical-surface — <substantive recorded reason>.
  ```

  A confirmation-style "N/A" is refused; the reason must be substantive (this is §1.2's own permitted
  friction — a forced-rationale gate, never a click-through — applied to the plan document).

**Enforcement.** A no-DB source-guard test
([`crates/cairn-node/tests/paper_parity_plan_section.rs`](crates/cairn-node/tests/paper_parity_plan_section.rs))
runs inside the existing `cargo test` gate and fails any plan dated on/after 2026-07-24 that carries
neither the section nor a substantive escape line. It is **forward-only** — the plans written before
the rule are the historical record and are left untouched (principle 2). The Tauri reference-client
slice is the first plan it binds.
```

- [ ] **Step 2: Verify the CONTRIBUTING.md fenced blocks are balanced**

Run: `grep -c '^```' CONTRIBUTING.md`
Expected: an **even** number (every fence closed). Note the nested inner fences use the same `` ``` `` — confirm by eye that the outer "## Paper-parity benchmark" section renders as intended (the two inner ```` ```markdown ```` blocks each close). If GitHub mis-renders the nested fence, switch the two inner blocks to `~~~markdown … ~~~`.

- [ ] **Step 3: Add house rule 7 to CLAUDE.md**

In `CLAUDE.md`, immediately after house rule 6 (the "Never hard-code cryptographic material" block, ending "…derives all key material from `rand_bytes`."), insert:

```markdown
7. **Every clinical-surface slice plan carries a falsifiable paper-parity benchmark (§1.2).** A plan
   for a slice that adds or changes a clinical workflow — at ANY layer, the in-DB floor and event core
   included — must carry a `## Paper-parity benchmark (§1.2)` section: the named paper counterpart, the
   step count (paper *N* human acts → architecture-forced *M* → UI bundling target *K*; **`M > N` is an
   architecture defect** — file it, per rule 5), and a time/cognitive-load *budget* whose measurement is
   owed by the slice that first exposes a runnable surface. A below-the-surface plan takes the
   forced-rationale escape instead — one line, `Paper-parity: not clinical-surface — <substantive
   reason>`. Gesture bundling is a UI/policy job (ADR-0021) — the architecture must merely not foreclose
   it. Enforced forward-only by `crates/cairn-node/tests/paper_parity_plan_section.rs` (rides the
   existing `cargo test` gate) and stated in `CONTRIBUTING.md`. (2026-07-15 review finding I9/G, #217.)
```

- [ ] **Step 4: Re-run the guard (the docs edits must not break it) + confirm no Rust changed**

Run: `cargo test -p cairn-node --test paper_parity_plan_section 2>&1 | tail -5`
Expected: 11 PASS (unchanged — these are docs edits, but the guard scans `plans/`, not `CONTRIBUTING.md`/`CLAUDE.md`).

- [ ] **Step 5: Commit**

```bash
git add CONTRIBUTING.md CLAUDE.md
git commit -s -m "docs(#217): CONTRIBUTING rule + CLAUDE house-rule 7 for the §1.2 paper-parity plan section

Canonical rule text (form, forced-rationale escape, M>N architecture-defect
consequence, forward-only enforcement) in CONTRIBUTING.md; a one-line pointer as
CLAUDE.md house rule 7 so the rule is in context while a plan is written.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

---

## Task 3: File the med-list sign-off UI-slice obligation

**Files:** none (GitHub issue).

**Interfaces:** none.

- [ ] **Step 1: Create the tracked artifact**

This is §1.2's "tracked as one" for the one workflow the review named — but correctly scoped as a **UI-slice** obligation, not an architecture defect (verified: `attest_thread_in_tx` takes an already-unsealed key, so one human gesture yields N signed commitments; the architecture forces `M = 1`).

```bash
gh issue create \
  --title "UI obligation: med-list whole-list sign-off must collapse to one human gesture (#217 first entry)" \
  --label documentation \
  --body "$(cat <<'EOF'
**Tier: UI/policy (layer 3/4), not architecture.** First live entry of the #217 paper-parity rule.

Paper counterpart of medication reconciliation sign-off: **one signature on one form.** ADR-0049
composes whole-list currency from N per-thread attestations. That is *not* an architecture defect:
`attest_thread_in_tx` takes an **already-unsealed** human key by reference, so an orchestrator unseals
once (`load_attester_key`) and signs N thread commitments in a single transaction. The N signatures
are a cryptographic artifact of ADR-0049's set-commitment staleness model, not N human acts — the
architecture forces **M = 1** and *permits* one-gesture bundling.

**The obligation is on the reference UI:** the med-list sign-off surface must expose whole-list
sign-off as **one human gesture** (paper N=1, UI bundling target K=1), never one gesture per thread.
The deferred ADR-0049 "list reviewed at T" summary event is the record-layer promotion path if a
worklist wants one.

**Owed by:** the Tauri reference-client med-list slice. That slice's plan must carry the §1.2
Paper-parity benchmark section (per CONTRIBUTING.md), with this issue named as its Steps-limb target.

Ref: 2026-07-15 whole-project review finding I9/G; issue #217; ADR-0049; ADR-0021.
EOF
)"
```

- [ ] **Step 2: Record the issue number**

Note the returned issue number (call it `#NEW`) — it is referenced in the HANDOVER/ROADMAP updates in Task 4.

---

## Task 4: Close out — HANDOVER, ROADMAP, full-suite green

**Files:**
- Modify: `docs/HANDOVER.md`
- Modify: `docs/ROADMAP.md`

**Interfaces:** none.

- [ ] **Step 1: Full workspace suite green**

Run: `cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings 2>&1 | tail -5`
Expected: clean. (`cargo test --workspace` requires `CAIRN_TEST_PG`; the DB-gated tests self-skip without it. The #217 guard is no-DB and runs regardless — confirm with the Task 1 command.)

Run (if `CAIRN_TEST_PG` is available — see HANDOVER "Test env"): `CAIRN_TEST_PG="host=127.0.0.1 port=5532 user=hherb dbname=cairn_test" cargo test --workspace 2>&1 | tail -15`
Expected: all pass. If the DB is not up, state that plainly in the PR body (the #217 guard itself needs no DB and is verified above).

- [ ] **Step 2: Update the HANDOVER `⇒ NEXT` block**

In `docs/HANDOVER.md`, edit the `⇒ NEXT` block so it reflects that **#217 is now done and the 2026-07-15 review course is FULLY closed** (P1–P5 + the whole Priority-6 queue + #217). The remaining unblocked feature work is unchanged: matcher #211, medication slices 6+, #287, and the new UI-slice obligation `#NEW` (owed by the future Tauri med-list slice). Add a one-paragraph session entry near the top dated 2026-07-24 summarizing this slice (rule text + CLAUDE house-rule 7 + the no-DB guard + `#NEW`), matching the terse style of the existing session entries. Keep the change surgical; do not rewrite unrelated paragraphs.

- [ ] **Step 3: Add ROADMAP Slice 52**

In `docs/ROADMAP.md`, append a **Slice 52** entry after Slice 51 (matcher #209/#210), in the same condensed style:

> **Slice 52 — the #217 paper-parity plan-section rule (2026-07-24; the last open 2026-07-15 review-course item; branch `feat/paper-parity-plan-section-217`; no spec/ADR/wire/SCHEMA change — a process rule + a no-DB source guard).** §1.2 paper-parity was normative but enforced by taste. Now every clinical-surface slice plan (all layers, forward-only from 2026-07-24) must carry a falsifiable `## Paper-parity benchmark (§1.2)` section — named paper counterpart + a step count judged on what the architecture *forecloses* (gesture bundling is a layer-3/4 job, ADR-0021; `M > N` = architecture defect) + a time/cognitive-load budget whose measurement is owed by the first runnable surface — or a forced-rationale `Paper-parity: not clinical-surface — <reason>` escape. Three layers: `CONTRIBUTING.md` rule text, `CLAUDE.md` house rule 7, and `crates/cairn-node/tests/paper_parity_plan_section.rs` (pure `plan_date`/`classify_declaration`/`verdict`, fixture-pinned against vacuity, riding the existing `cargo test` gate — no new workflow). Filed `#NEW` — the med-list whole-list sign-off UI obligation (one human gesture; the architecture already permits it, `M=1`), the rule's first live entry, owed by the Tauri med-list slice. **The 2026-07-15 review course is now FULLY closed.**

(Replace `#NEW` with the Task 3 issue number.)

- [ ] **Step 4: Commit**

```bash
git add docs/HANDOVER.md docs/ROADMAP.md
git commit -s -m "docs(#217): HANDOVER + ROADMAP — #217 done, review course fully closed (Slice 52)

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>"
```

- [ ] **Step 5: Push and open the PR**

```bash
git push -u origin feat/paper-parity-plan-section-217
gh pr create --base main --title "#217: paper-parity benchmark as a required slice-plan section" --body "$(cat <<'EOF'
Closes #217 — the last open item of the 2026-07-15 whole-project review course (finding I9/G).

## What

vision.md §1.2 makes paper-parity falsifiable, but nothing built had been recorded in that form —
"enforced by taste, not by the falsifiable form the spec mandates." This adds a required
Paper-parity benchmark section to every clinical-surface slice plan and enforces it.

- **Rule (CONTRIBUTING.md):** named paper counterpart + step count (**judged on what the architecture
  forecloses**, not rendered gestures — bundling is a layer-3/4 job per ADR-0021; `M > N` = architecture
  defect) + a time/cognitive-load *budget* whose measurement is owed by the first runnable surface. A
  below-the-surface plan takes a forced-rationale `Paper-parity: not clinical-surface — <reason>` escape.
- **Pointer (CLAUDE.md house rule 7):** in context while a plan is written.
- **Guard (`crates/cairn-node/tests/paper_parity_plan_section.rs`):** a no-DB source-level test — pure
  `plan_date`/`classify_declaration`/`verdict`, exhaustively unit-tested and fixture-pinned against
  vacuity, plus a filesystem scan of `docs/superpowers/plans/`. Forward-only from a fixed 2026-07-24
  cutoff; the 67 pre-existing plans are the historical record and are untouched (principle 2). Rides the
  existing `rust.yml` `test` job — no new workflow wiring.
- **Tracked artifact (#NEW):** the med-list whole-list sign-off must collapse to one human gesture — a
  UI-slice obligation, *not* an architecture defect (verified: one key-unseal → N signed commitments, so
  the architecture forces M=1). Owed by the future Tauri med-list slice; the rule's first live entry.

## Scope / non-goals

Process + tooling only — no spec/ADR/wire/SCHEMA change. Binds plans, not specs. Does not write the
Tauri slice plan. The "substantive reason" check is a crude length proxy (defeats silence, not bad
faith — review's job), stated honestly in the code.

## Design

docs/superpowers/specs/2026-07-24-paper-parity-benchmark-slice-plan-section-217.md

🤖 Generated with [Claude Code](https://claude.com/claude-code)
EOF
)"
```

(Replace `#NEW` with the Task 3 issue number before running.)

---

## Self-review notes (author)

- **Spec coverage:** every spec section maps to a task — required form + rule text → Task 2; three enforcement layers → Task 1 (CI) + Task 2 (CONTRIBUTING + CLAUDE); guard mechanics incl. anti-vacuity fixtures + loud malformed-filename failure + fixed cutoff → Task 1; the tracked parity artifact → Task 3; TDD order → Task 1 steps; HANDOVER/ROADMAP → Task 4.
- **Type consistency:** `plan_date`/`classify_declaration`/`verdict`, `Declaration::{Benchmark, NotClinical{reason}, Missing}`, `Verdict::{Exempt, Pass, Fail(String)}`, `CUTOFF`, `MIN_REASON_LEN` are used identically across all steps.
- **Known structural limit (documented, not a gap):** a plan that *quotes* the empty template classifies as `Benchmark` — structural, not semantic; review catches gaming. This is why this plan file itself passes as `Benchmark` (it quotes the form) while also carrying the honest escape line.
