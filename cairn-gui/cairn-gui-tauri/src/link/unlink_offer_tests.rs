//! Which verdicts the review panel offers (R6): a judgement needs the whole picture and the
//! standing address it must send back, so a partial panel, or one whose pair changed mid-read,
//! offers neither. Split from `unlink_view_tests.rs` (#781), whose fixtures it shares.
use super::tests::{held, id, separated, set, standing, unattested_link, whole_parts};
use super::*;
use crate::link::confirm_view::{LINK_THEM, PAIR_CHANGED_MID_READ};
use crate::link::record_links::PairKind;

/// A partial read offers neither verdict: a judgement needs the whole picture.
#[test]
fn a_partial_review_offers_no_verdict() {
    let mut parts = whole_parts();
    parts.findings = Err("boom".into());
    let v = link_review_view(parts, &set(&[1, 2]), id(1), id(2), &unattested_link());
    assert!(!v.can_unlink && !v.can_confirm);
    assert_eq!(v.same_label, None);
}

#[test]
fn a_separated_review_offers_no_unlink() {
    let mut parts = whole_parts();
    parts.standing = Ok(Some(standing("unlink", true)));
    let v = link_review_view(parts, &set(&[1, 2]), id(1), id(2), &separated());
    assert!(!v.can_unlink, "a separated pair is already unlinked");
    assert!(v.can_confirm && v.same_label.as_deref() == Some(LINK_THEM));
}

/// The line showed a standing judgement on the pair; the panel's read found none. The pair
/// changed between the two reads, so NEITHER verdict is offered — and Unlink now needs the
/// standing address as much as the confirm does (the node re-checks it before signing).
#[test]
fn an_unread_standing_assertion_offers_no_verdict() {
    for shown in [unattested_link(), separated()] {
        let mut parts = whole_parts();
        parts.standing = Ok(None);
        let v = link_review_view(parts, &set(&[1, 2]), id(1), id(2), &shown);
        assert!(!v.can_confirm && !v.can_unlink, "{shown:?}");
        assert_eq!(v.problems, vec![PAIR_CHANGED_MID_READ.to_string()]);
    }
}

/// Unlink is offered only with the standing address it must send back: the whole panel on a
/// standing link carries it, and `can_unlink` is never true beside an empty one.
#[test]
fn unlink_is_offered_only_with_the_standing_address() {
    let v = link_review_view(
        whole_parts(),
        &set(&[1, 2]),
        id(1),
        id(2),
        &unattested_link(),
    );
    assert!(v.can_unlink && !v.standing.is_empty());
}

/// An empty standing address is refused by both verdicts with a sentence that says what is
/// actually wrong — the comparison never read which judgement stands — never [`LINK_GONE`].
#[test]
fn an_empty_standing_address_is_refused_truthfully() {
    let err = require_standing(&[]).unwrap_err();
    assert_eq!(err.text, STANDING_UNREAD);
    assert!(
        !err.text.contains("no longer part of this record"),
        "{}",
        err.text
    );
    assert!(err.text.contains("nothing was done") && err.text.contains("review it again"));
    assert!(require_standing(&[7; 34]).is_ok());
}

/// A failed read of which judgement stands is a named problem — never a silent missing button.
#[test]
fn a_failed_standing_read_is_named_and_offers_no_verdict() {
    let mut parts = whole_parts();
    parts.standing = Err("reset".into());
    let v = link_review_view(parts, &set(&[1, 2]), id(1), id(2), &unattested_link());
    assert!(!v.can_confirm && !v.can_unlink);
    assert!(v
        .problems
        .iter()
        .any(|m| m.contains("Which judgement stands on this pair could not be read: reset")));
}

/// Through the panel: a chart not held here withholds Confirm, says why, and still offers
/// Unlink. The standing address travels to the webview, which sends it back with either verdict.
#[test]
fn a_chart_not_held_here_still_offers_unlink_but_not_confirm() {
    let mut parts = whole_parts();
    parts.high = Ok(vec![ChartFacts {
        held: false,
        ..held(2)
    }]);
    let v = link_review_view(parts, &set(&[1, 2]), id(1), id(2), &unattested_link());
    assert!(v.can_unlink && !v.can_confirm);
    assert!(v.same_note.unwrap().contains(&id(2).to_string()));
    let whole = link_review_view(
        whole_parts(),
        &set(&[1, 2]),
        id(1),
        id(2),
        &unattested_link(),
    );
    assert!(whole.can_confirm);
    assert_eq!(whole.standing, vec![7; 34]);
    assert_eq!(whole.kind, PairKind::Link);
}

/// Safety: the line and the pair's standing judgement are read under two lock acquisitions. A
/// colleague's attested unlink landing between them (a third chart still joining the record)
/// must not let the panel offer "Same person — confirm" carrying the UNLINK's address — the
/// node's pair check would accept it. Neither verdict is offered.
#[test]
fn a_standing_judgement_that_changed_mid_read_offers_no_verdict() {
    let mut parts = whole_parts();
    parts.standing = Ok(Some(standing("unlink", true)));
    let v = link_review_view(parts, &set(&[1, 2, 3]), id(1), id(2), &unattested_link());
    assert!(!v.can_confirm && !v.can_unlink);
    assert_eq!(v.same_label, None);
    assert!(
        v.standing.is_empty(),
        "the unlink's address never reaches the webview"
    );
    assert_eq!(v.problems, vec![PAIR_CHANGED_MID_READ.to_string()]);
    assert_eq!(
        PAIR_CHANGED_MID_READ,
        "This pair changed while it was being read — close the comparison and review it again."
    );
}

/// The same rule for a separation that was relinked mid-read.
#[test]
fn a_separation_relinked_mid_read_offers_no_verdict() {
    let mut parts = whole_parts();
    parts.standing = Ok(Some(standing("link", true)));
    let v = link_review_view(parts, &set(&[1, 2]), id(1), id(2), &separated());
    assert!(!v.can_confirm && !v.can_unlink && v.standing.is_empty());
    assert_eq!(v.problems, vec![PAIR_CHANGED_MID_READ.to_string()]);
}

/// A partial panel draws no "same person" button, so the note that explains what the button
/// overrides ("Linking them records your judgement over theirs…") must go with it. The note
/// that says why there is NO button (a chart not held here) is true whatever failed to read, so
/// it stays.
#[test]
fn a_partial_panel_keeps_only_the_not_held_note() {
    let mut parts = whole_parts();
    parts.standing = Ok(Some(standing("unlink", true)));
    parts.findings = Err("boom".into());
    let v = link_review_view(parts, &set(&[1, 2]), id(1), id(2), &separated());
    assert_eq!((v.same_label, v.same_note), (None, None));

    let mut parts = whole_parts();
    parts.high = Ok(vec![ChartFacts {
        held: false,
        ..held(2)
    }]);
    parts.findings = Err("boom".into());
    let v = link_review_view(parts, &set(&[1, 2]), id(1), id(2), &unattested_link());
    assert_eq!(v.same_label, None);
    let note = v
        .same_note
        .expect("the not-held note stays on a partial panel");
    assert!(note.contains("not held on this node"), "{note}");
}
