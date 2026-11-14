//! Tests for `row_view.rs`'s set-aside versions (batch charting slices 2a and 2b), split from
//! `row_view_tests.rs` to keep each file under 500 lines.
use crate::test_rows::{member, row};
use cairn_medication_view::{MedicationStatus, VouchState};

/// Batch charting slice 2a: a version is shown under the line it was set aside beside, joined on
/// the line's member thread (`alternative_to`), and never under another line, though it be the
/// same drug (regular and PRN oxycodone). Its words are the Global Constraints' copy.
#[test]
fn versions_join_under_their_companion_only() {
    use crate::row_view::{version_line, versions_under, VersionView};
    use crate::test_rows::uid;
    let regular = row(
        1,
        MedicationStatus::Active,
        vec![
            member(10, VouchState::Absent),
            member(11, VouchState::Absent),
        ],
    );
    let prn = row(
        2,
        MedicationStatus::Ceased,
        vec![member(20, VouchState::Absent)],
    );
    let (a, b, c) = (version("a"), version("b"), version("c"));
    let versions = [
        (uid(11), a.clone()),
        (uid(20), b.clone()),
        (uid(11), c.clone()),
        (uid(99), version("x")),
    ];
    let lines = |shown: Vec<VersionView>| shown.into_iter().map(|v| v.line).collect::<Vec<_>>();
    assert_eq!(lines(versions_under(&regular, &versions)), [a.line, c.line]);
    assert_eq!(
        lines(versions_under(&prn, &versions)),
        [b.line],
        "a stopped line keeps its own"
    );
    assert_eq!(
        version_line("GP letter", "metoprolol 25 mg (external-record)", None),
        "GP letter: metoprolol 25 mg (external-record) — set aside, not current"
    );
    assert_eq!(
        version_line("Pharmacy", "metoprolol 25 mg", Some("packet 25 mg")),
        "Pharmacy: metoprolol 25 mg — set aside, not current — packet 25 mg"
    );
}

/// A version with no settlement, offered both buttons: what slice 2a drew, plus 2b's fields.
fn version(line: &str) -> crate::row_view::VersionView {
    crate::row_view::VersionView {
        line: line.to_string(),
        version_id: format!("v-{line}"),
        settled: vec![],
        open: false,
        can_settle: true,
        restored: false,
    }
}

/// Batch charting slice 2b: each settlement as the Global Constraints word it — who (else "signer
/// not known") and when (`local_instant`, so today's reads as the clock time) — and the line a
/// restore replaced, under the new line.
#[test]
fn settle_words_reads_checked_and_restored() {
    use crate::row_view::{settle_words, SettleLine};
    use chrono::{FixedOffset, TimeZone, Utc};
    let now = FixedOffset::east_opt(10 * 3600)
        .unwrap()
        .with_ymd_and_hms(2026, 11, 14, 9, 0, 0)
        .unwrap();
    let at = Utc.with_ymd_and_hms(2026, 11, 13, 22, 30, 0).unwrap(); // 08:30 at the workstation
    let checked = SettleLine::Checked {
        source: "GP letter",
        why: Some("old packet"),
    };
    assert_eq!(
        settle_words(checked, Some("Dr Bea Smith"), &at, &now),
        "checked by Dr Bea Smith, 08:30: not current — from GP letter — old packet"
    );
    let bare = SettleLine::Checked {
        source: "Pharmacy",
        why: None,
    };
    assert_eq!(
        settle_words(bare, None, &at, &now),
        "checked by signer not known, 08:30: not current — from Pharmacy"
    );
    assert_eq!(
        settle_words(SettleLine::Restored, Some("Dr Bea Smith"), &at, &now),
        "restored by Dr Bea Smith, 08:30"
    );
    let replaced = SettleLine::Replaced {
        line: "metoprolol 50 mg 50mg bd (patient-reported)",
    };
    assert_eq!(
        settle_words(replaced, Some("Dr Bea Smith"), &at, &now),
        "replaced: metoprolol 50 mg 50mg bd (patient-reported) — Dr Bea Smith, 08:30"
    );
}

/// "{u} to check" counts only the versions still open: no check, no restore, AND a current line.
/// Versions of a stopped line are not (ruling R3: nothing is left to compete with), though they
/// keep their buttons; nor is a checked or a restored one. The line a restore replaced is shown
/// under the restore's own line only (`under_line`, on its member threads).
#[test]
fn a_stopped_companions_versions_are_not_open() {
    use crate::row_view::{under_line, versions_under};
    use crate::test_rows::uid;
    let current = row(
        1,
        MedicationStatus::Active,
        vec![member(10, VouchState::Absent)],
    );
    let stopped = row(
        2,
        MedicationStatus::Ceased,
        vec![member(20, VouchState::Absent)],
    );
    let checked = crate::row_view::VersionView {
        settled: vec!["checked by x".into()],
        ..version("checked")
    };
    let restored = crate::row_view::VersionView {
        settled: vec!["restored by x".into()],
        restored: true,
        can_settle: false,
        ..version("restored")
    };
    let versions = [
        (uid(10), version("open")),
        (uid(10), checked),
        (uid(20), version("beside a stopped line")),
        (uid(20), restored),
    ];
    let open =
        |shown: Vec<crate::row_view::VersionView>| shown.iter().map(|v| v.open).collect::<Vec<_>>();
    assert_eq!(open(versions_under(&current, &versions)), [true, false]);
    assert_eq!(open(versions_under(&stopped, &versions)), [false, false]);
    assert!(
        versions_under(&stopped, &versions)[0].can_settle,
        "its buttons stay"
    );
    let replaced = [
        (uid(10), "replaced: a".to_string()),
        (uid(30), "replaced: b".to_string()),
    ];
    assert_eq!(under_line(&current, &replaced), ["replaced: a"]);
    assert!(under_line(&stopped, &replaced).is_empty());
}
