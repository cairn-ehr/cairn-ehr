//! Tests for `link/unlink_view.rs`, in a sibling file like `view_tests.rs`.
use super::*;
use cairn_node::patient::compare::{FieldFact, NameFact};

fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}

fn held(n: u128) -> ChartFacts {
    ChartFacts {
        patient_id: id(n),
        held: true,
        trust: "confirmed".into(),
        names: vec![NameFact {
            value: "N".into(),
            use_: Some("legal".into()),
            provenance: "patient-stated".into(),
        }],
        dob: Some(FieldFact {
            value: "1950-07-01".into(),
            provenance: "document-verified".into(),
            precision: Some("day".into()),
        }),
        ..ChartFacts::default()
    }
}

fn set(v: &[u128]) -> ChartSet {
    ChartSet::new(v.iter().map(|n| Uuid::from_u128(*n))).unwrap()
}

fn parts() -> UnlinkParts {
    UnlinkParts {
        low: Ok(vec![held(1)]),
        high: Ok(vec![held(2)]),
        findings: Ok(vec![]),
    }
}

#[test]
fn an_unlink_comparison_has_the_two_charts_and_sends_back_the_record() {
    let v = unlink_comparison_view(parts(), &set(&[1, 2, 3]), id(1), id(2));
    assert_eq!(v.columns.len(), 2);
    assert_eq!(
        v.charts.len(),
        3,
        "the record compared FROM, sent back with the unlink"
    );
    assert_eq!(
        (v.low.clone(), v.high.clone()),
        (id(1).to_string(), id(2).to_string())
    );
    assert!(v.can_unlink);
}

#[test]
fn a_partial_unlink_comparison_names_what_is_missing_and_cannot_unlink() {
    let mut p = parts();
    p.findings = Err("timeout".into());
    let v = unlink_comparison_view(p, &set(&[1, 2]), id(1), id(2));
    assert!(!v.can_unlink);
    assert!(v.problems.iter().any(|m| m.contains("could not be run")));
}

#[test]
fn an_unread_chart_is_named_and_cannot_unlink() {
    let mut p = parts();
    p.high = Err("gone".into());
    let v = unlink_comparison_view(p, &set(&[1, 2]), id(1), id(2));
    assert!(!v.can_unlink);
    assert!(v.problems.iter().any(|m| m.contains(&id(2).to_string())));
}

#[test]
fn a_split_names_the_charts_that_left_and_reloads() {
    let r = unlink_report(
        LinkEffect::TookEffect,
        id(2),
        id(3),
        &set(&[1, 2, 3]),
        &set(&[1, 2]),
    );
    assert!(r.sentence.starts_with("Unlinked"));
    assert!(r.sentence.contains(&id(3).to_string()));
    assert!(r.reload);
}

/// Review Focus 2.
#[test]
fn still_joined_never_reads_as_done_and_points_at_the_links_list() {
    let r = unlink_report(
        LinkEffect::StillJoined,
        id(2),
        id(3),
        &set(&[1, 2, 3]),
        &set(&[1, 2, 3]),
    );
    assert!(!r.sentence.starts_with("Unlinked"));
    assert!(r.sentence.contains("still"));
    assert!(r.sentence.contains("How these charts are linked"));
    assert!(
        r.reload,
        "the list must re-read: this link is gone from it, the others remain"
    );
}

#[test]
fn outranked_is_a_disagreement_to_settle_not_retry() {
    let r = unlink_report(
        LinkEffect::Outranked,
        id(2),
        id(3),
        &set(&[1, 2, 3]),
        &set(&[1, 2, 3]),
    );
    assert!(r.sentence.contains("NOT in effect"));
    assert!(r.sentence.contains("the same person"));
    assert!(!r.reload);
}

#[test]
fn an_unlink_error_is_worded_as_an_unlink() {
    let v = crate::link::view::judgement_error_from(
        "unlink",
        cairn_gui_data::port::DataError::Refused("x".into()),
    );
    assert!(v.text.starts_with("The unlink was refused"));
}

#[test]
fn outranked_does_not_call_a_retry_a_no_op() {
    let r = unlink_report(
        LinkEffect::Outranked,
        id(2),
        id(3),
        &set(&[1, 2, 3]),
        &set(&[1, 2, 3]),
    );
    assert!(!r.sentence.contains("changes nothing"));
    assert!(r
        .sentence
        .contains("would normally record a newer judgement that overrules theirs"));
}

#[test]
fn a_took_effect_that_split_nothing_is_not_called_unlinked() {
    let r = unlink_report(
        LinkEffect::TookEffect,
        id(2),
        id(3),
        &set(&[1, 2, 3]),
        &set(&[1, 2, 3]),
    );
    assert!(!r.sentence.starts_with("Unlinked"));
    assert!(r.sentence.contains("did not change"));
    assert!(r.reload);
}

#[test]
fn a_split_uses_plural_agreement() {
    let r = unlink_report(
        LinkEffect::TookEffect,
        id(2),
        id(3),
        &set(&[1, 2, 3]),
        &set(&[1, 2]),
    );
    assert!(r.sentence.contains("are no longer part of this record"));
}

#[test]
fn charts_that_joined_concurrently_are_named() {
    let r = unlink_report(
        LinkEffect::TookEffect,
        id(2),
        id(3),
        &set(&[1, 2, 3]),
        &set(&[1, 2, 4]),
    );
    assert!(r.sentence.contains("also includes chart(s)"));
    assert!(r.sentence.contains(&id(4).to_string()));
}

#[test]
fn still_joined_says_other_links() {
    let r = unlink_report(
        LinkEffect::StillJoined,
        id(2),
        id(3),
        &set(&[1, 2, 3]),
        &set(&[1, 2, 3]),
    );
    assert!(r.sentence.contains("through other links"));
}

#[test]
fn still_joined_names_charts_that_left_concurrently_without_claiming_no_change() {
    // Chart 4 was in the comparison but is gone from the record now (a peer unlinked it).
    let r = unlink_report(
        LinkEffect::StillJoined,
        id(2),
        id(3),
        &set(&[1, 2, 3, 4]),
        &set(&[1, 2, 3]),
    );
    assert!(
        !r.sentence.contains("did not change"),
        "the record did change, so the line must not say otherwise"
    );
    assert!(r.sentence.contains("did not split them"));
    assert!(r.sentence.contains("left this record"));
    assert!(r.sentence.contains(&id(4).to_string()));
    // Nothing changed: the plain sentence, no concurrent-change tail.
    let same = unlink_report(
        LinkEffect::StillJoined,
        id(2),
        id(3),
        &set(&[1, 2, 3]),
        &set(&[1, 2, 3]),
    );
    assert!(same.sentence.contains("did not change"));
    assert!(!same.sentence.contains("Meanwhile"));
}

#[test]
fn still_joined_names_charts_that_joined_concurrently_without_claiming_no_change() {
    // Chart 4 was not in the comparison but is on the record now (a peer linked it).
    let r = unlink_report(
        LinkEffect::StillJoined,
        id(2),
        id(3),
        &set(&[1, 2, 3]),
        &set(&[1, 2, 3, 4]),
    );
    assert!(
        !r.sentence.contains("did not change"),
        "the record did change, so the line must not say otherwise"
    );
    assert!(r.sentence.contains("did not split them"));
    assert!(r.sentence.contains("joined this record"));
    assert!(r.sentence.contains(&id(4).to_string()));
}

#[test]
fn an_unread_low_chart_is_named_and_cannot_unlink() {
    let mut p = parts();
    p.low = Err("gone".into());
    let v = unlink_comparison_view(p, &set(&[1, 2]), id(1), id(2));
    assert!(!v.can_unlink);
    assert!(v.problems.iter().any(|m| m.contains(&id(1).to_string())));
}

#[test]
fn every_unlink_error_arm_is_worded_as_an_unlink() {
    use crate::link::view::judgement_error_from as f;
    use cairn_gui_data::port::DataError;
    assert!(f("unlink", DataError::NotProvisioned("x".into()))
        .text
        .starts_with("This node cannot record the unlink yet"));
    assert!(f("unlink", DataError::Unavailable("x".into()))
        .text
        .starts_with("The unlink was not confirmed"));
    assert_eq!(
        f("unlink", DataError::NotFound).text,
        "The unlink was not recorded."
    );
}

#[test]
fn the_link_gone_text_says_nothing_was_done() {
    assert!(LINK_GONE.contains("no longer part of this record"));
    assert!(LINK_GONE.contains("nothing was done"));
}

/// THE UNLINK PANEL'S DRIFT GUARD — the guard `commands.rs` keeps for `main.js` and
/// `funnel::commands` for `funnel.js` (PR #711 review). `unlink.js` is untyped, so a Rust field
/// rename renders `undefined` instead of breaking the build. Here that is not cosmetic:
/// `report.reload` undefined sends every split down the Outranked path (panel left open, chart
/// never re-read beside an "Unlinked" sentence), and `failure.retry` undefined leaves the Unlink
/// button live on a verdict.
#[test]
fn unlink_js_reads_no_field_the_backend_does_not_send() {
    use crate::commands::tests::fields_read_in;
    use crate::funnel::view::{ErrorView, Retry};
    use std::collections::BTreeSet;
    let js = include_str!("../../src-ui/unlink.js");
    let view = unlink_comparison_view(parts(), &set(&[1, 2]), id(1), id(2));
    let keys = |v: serde_json::Value| -> BTreeSet<String> {
        v.as_object()
            .expect("payload must serialize to an object")
            .keys()
            .cloned()
            .collect()
    };
    let payloads = [
        ("view", serde_json::to_value(&view).unwrap()),
        ("col", serde_json::to_value(&view.columns[0]).unwrap()),
        ("row", serde_json::to_value(&view.rows[0]).unwrap()),
        (
            "report",
            serde_json::to_value(LinkReportView {
                sentence: String::new(),
                reload: true,
            })
            .unwrap(),
        ),
        (
            "failure",
            serde_json::to_value(ErrorView {
                text: String::new(),
                retry: Retry::Never,
            })
            .unwrap(),
        ),
    ];
    for (binding, payload) in payloads {
        let available = keys(payload);
        let read = fields_read_in(js, binding);
        assert!(
            !read.is_empty(),
            "unlink.js no longer reads `{binding}` at all — rename the binding in this guard, \
             don't delete it"
        );
        for field in read {
            assert!(
                available.contains(&field),
                "unlink.js reads `{binding}.{field}`, which the backend does not send. \
                 Available: {available:?}"
            );
        }
    }
    // The other direction, for the fields whose SILENCE is the dangerous failure.
    assert!(fields_read_in(js, "report").contains("reload"));
    assert!(fields_read_in(js, "failure").contains("retry"));
    assert!(fields_read_in(js, "view").contains("can_unlink"));
}
