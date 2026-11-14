//! The warning threshold in the window (batch-charting slice 3, ADR-0104). At the default only
//! an allergy mark warns; lowered by the administrator to `adverse-effect`, a previous adverse
//! effect on a line being charted is a warning too: a Start line needs a reason for it, it is
//! drawn as loud as a mark under a `Warning` tag (never `Allergy`, maintainer Q1), and the
//! override names its record. The threshold is read on the draft path only. DB walks are
//! `allergy_check_threshold_db_tests.rs`.
use super::*;
use crate::allergy_check::ALLERGIES_NOT_READ;
use crate::med_chart_webview_tests::MED_JS;
use crate::source_guard::{js_fn, position};
use cairn_node::allergy::check::WarningThreshold;

const NOTE: &str = "Previous adverse effect: codeine → GI upset";
const TAGS_JS: &str = include_str!("../src-ui/allergy_tags.js");
const LIST_JS: &str = include_str!("../src-ui/med_list.js");

/// A check with one adverse-effect note from record 9, at `threshold`.
fn adverse(threshold: WarningThreshold) -> LineCheck {
    LineCheck {
        notes: vec![NOTE.into()],
        adverse_against: vec![Uuid::from_u128(9)],
        threshold,
        ..LineCheck::default()
    }
}

#[test]
fn a_lowered_threshold_asks_a_start_line_for_a_reason() {
    let start = ChartedAs::StartedHere;
    let at_default = adverse(WarningThreshold::AllergyHit);
    assert_eq!(
        override_before_the_key(start, &at_default, None).unwrap(),
        None
    );
    let lowered = adverse(WarningThreshold::AdverseEffect);
    assert_eq!(
        override_before_the_key(start, &lowered, None)
            .unwrap_err()
            .text,
        NEEDS_A_REASON
    );
    assert_eq!(
        override_before_the_key(start, &lowered, Some(" tolerated last year ")).unwrap(),
        Some("tolerated last year")
    );
    // An already-taking line records what is taken: never a reason, at any threshold.
    assert_eq!(
        override_before_the_key(ChartedAs::PreExisting, &lowered, None).unwrap(),
        None
    );
}

/// A lowered warning AND an unverified allergy (review focus 2): the warning takes the reason,
/// so the "check did not run" refusal is not the answer.
#[test]
fn a_lowered_warning_takes_a_reason_even_with_an_unverified_allergy() {
    let both = LineCheck {
        unverified: true,
        ..adverse(WarningThreshold::AdverseEffect)
    };
    assert_eq!(
        override_before_the_key(ChartedAs::StartedHere, &both, Some("seen by the team")).unwrap(),
        Some("seen by the team")
    );
}

#[test]
fn the_view_splits_warning_notes_from_quiet_ones() {
    let v = view(adverse(WarningThreshold::AllergyHit));
    assert!(!v.reason_needed && v.warning_notes.is_empty());
    assert_eq!(v.notes, [NOTE]);
    let v = view(adverse(WarningThreshold::AdverseEffect));
    assert!(v.reason_needed && v.notes.is_empty());
    assert_eq!(v.warning_notes, [NOTE]);
    // A note with no record (the allergy list not read) stays quiet, even lowered.
    let v = view(LineCheck {
        notes: vec![ALLERGIES_NOT_READ.into()],
        unverified: true,
        threshold: WarningThreshold::AdverseEffect,
        ..LineCheck::default()
    });
    assert!(!v.reason_needed && v.warning_notes.is_empty());
    assert_eq!(v.notes, [ALLERGIES_NOT_READ]);
}

/// The override at Sign names the adverse-effect record too, after the marks.
#[test]
fn the_override_names_what_warned() {
    let mut c = adverse(WarningThreshold::AdverseEffect);
    c.marks.push("Allergy recorded: x".into());
    c.against.push(Uuid::from_u128(1));
    assert_eq!(
        override_ids(&c),
        [
            Uuid::from_u128(1).to_string(),
            Uuid::from_u128(9).to_string()
        ]
    );
    assert_eq!(
        override_ids(&adverse(WarningThreshold::AllergyHit)),
        Vec::<String>::new()
    );
}

/// An unreadable threshold over-warns (the check module's safe error); a read word is parsed.
#[test]
fn an_unread_threshold_over_warns() {
    use crate::allergy_check::threshold_of;
    assert_eq!(
        threshold_of(Ok("allergy-hit".into())),
        WarningThreshold::AllergyHit
    );
    assert_eq!(
        threshold_of(Ok("adverse-effect".into())),
        WarningThreshold::AdverseEffect
    );
    assert_eq!(
        threshold_of(Err(anyhow::anyhow!("connection lost"))),
        WarningThreshold::AdverseEffect
    );
}

/// The words (Q1): `Warning`, never `Allergy`, and "not checked" still said beside it.
#[test]
fn a_warning_that_is_not_an_allergy_is_tagged_warning() {
    assert!(
        TAGS_JS.contains("const WARNING_TAG = \"Warning\";"),
        "{TAGS_JS}"
    );
    let draw = js_fn(MED_JS, "drawMarks");
    let mark = position(draw, "check.marks.length > 0");
    let warned = position(draw, "check.warning_notes.length > 0");
    let unchecked = position(draw, "} else if (check.unverified)");
    assert!(mark < warned && warned < unchecked, "{draw}");
    let branch = &draw[warned..unchecked];
    assert!(branch.contains("WARNING_TAG"), "{branch}");
    assert!(!branch.contains("ALLERGY_TAG"), "{branch}");
    assert!(branch.contains("\" · \" + NOT_CHECKED_TAG"), "{branch}");
    assert!(
        branch.contains("tag.className = \"allergy-tag\";"),
        "{branch}"
    );
    // Drawn loud, between the marks and the quiet notes.
    let loud = position(
        draw,
        "check.warning_notes.map((note) => cell(\"li\", note, { class: \"allergy-prominent\" }))",
    );
    let quiet = position(
        draw,
        "check.notes.map((note) => cell(\"li\", note, { class: \"allergy-quiet\" }))",
    );
    assert!(
        position(draw, "check.marks.map(") < loud && loud < quiet,
        "{draw}"
    );
    // The script's own stand-in checks carry the field drawMarks reads.
    assert!(MED_JS.contains(
        "const LINE_NOT_CHECKED = { marks: [], warning_notes: [], notes: [], unverified: true };"
    ));
    assert!(LIST_JS.contains(
        "const NO_MARKS = { marks: [], warning_notes: [], notes: [], unverified: false };"
    ));
}
