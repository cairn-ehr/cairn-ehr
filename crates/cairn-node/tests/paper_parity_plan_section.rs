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
fn verdict(date: (i32, u32, u32), cutoff: (i32, u32, u32), declaration: &Declaration) -> Verdict {
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
    assert_eq!(
        classify_declaration("## Task 1\nsome plan text"),
        Declaration::Missing
    );
}

#[test]
fn verdict_exempts_a_pre_cutoff_plan_even_if_missing() {
    assert_eq!(
        verdict((2026, 7, 13), CUTOFF, &Declaration::Missing),
        Verdict::Exempt
    );
}

#[test]
fn verdict_passes_a_bound_benchmark() {
    assert_eq!(
        verdict((2026, 7, 24), CUTOFF, &Declaration::Benchmark),
        Verdict::Pass
    );
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
        let name = path
            .file_name()
            .expect("dir entry has a file name")
            .to_string_lossy()
            .into_owned();
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
