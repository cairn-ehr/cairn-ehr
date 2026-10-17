//! Tests for `link/judgement_view.rs`: what a judgement did, or why it failed. Moved out of
//! `view_tests.rs` with the code they pin (#781).
use super::*;

fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}

#[test]
fn a_link_that_took_effect_reloads_and_one_outranked_does_not() {
    let set = ChartSet::new([id(1), id(2)]).unwrap();
    let took = link_report(LinkEffect::TookEffect, &set, &set);
    assert!(
        took.reload && took.sentence.starts_with("Linked"),
        "link report should indicate the link took effect and begin with 'Linked'"
    );
    assert!(took.sentence.contains("2 charts"));
    let lost = link_report(LinkEffect::Outranked, &ChartSet::single(id(1)), &set);
    assert!(!lost.reload, "a disagreement is shown, never reloaded away");
    assert!(
        !lost.sentence.contains("Linked"),
        "a link that did not take effect is never called linked"
    );
    assert!(
        lost.sentence.contains("NOT in effect"),
        "outranked link should mention it is not in effect"
    );
    // Final review M6: pressing Link again is not a no-op — it records another judgement.
    assert!(
        !lost.sentence.contains("changes nothing"),
        "a retry is not a no-op: it would normally overrule the other judgement"
    );
    assert!(
        lost.sentence
            .contains("would normally record a newer judgement that overrules theirs"),
        "outranked link should say a retry would normally overrule"
    );
}

/// Final review O1: the outcome tells the truth about what the link joined. A chart the
/// record now combines that was in NEITHER compared set (a third chart already linked to one
/// side by the time the judgement landed) is named, so it is reviewed rather than assumed.
#[test]
fn a_chart_the_comparison_never_showed_is_named() {
    let compared = ChartSet::new([id(1), id(2)]).unwrap();
    let now = ChartSet::new([id(1), id(2), id(3)]).unwrap();
    let took = link_report(LinkEffect::TookEffect, &now, &compared);
    assert!(
        took.sentence.starts_with("Linked"),
        "link report should begin with 'Linked'"
    );
    assert!(
        took.sentence.ends_with(&format!(
            "The record now also includes chart(s) {} that were not in the comparison — \
             review them.",
            id(3)
        )),
        "link report should mention the third chart not in the comparison"
    );
    let exact = link_report(LinkEffect::TookEffect, &compared, &compared);
    assert!(
        !exact.sentence.contains("not in the comparison"),
        "link report should not mention unanticipated charts when none were added"
    );
}

/// Final review M4: every arm of the funnel's error classification, worded for a link. A
/// verdict (refused / not provisioned) is never retried as-is; only an outage is retryable.
#[test]
fn every_data_error_arm_is_worded_for_a_link() {
    let refused_view = judgement_error_from("link", DataError::Refused("floor says no".into()));
    assert_eq!(refused_view.retry, Retry::Never);
    assert_eq!(refused_view.text, "The link was refused: floor says no");

    let unprovisioned =
        judgement_error_from("link", DataError::NotProvisioned("key not held".into()));
    assert_eq!(unprovisioned.retry, Retry::AfterOperator);
    assert_eq!(
        unprovisioned.text,
        "This node cannot record the link yet: key not held"
    );
    assert!(
        !unprovisioned.text.contains("operator"),
        "the not-held arm resolves by sync, not by an operator"
    );

    let outage = judgement_error_from("link", DataError::Unavailable("connection reset".into()));
    assert_eq!(outage.retry, Retry::Now);
    assert!(outage.text.contains("not confirmed"), "{}", outage.text);

    let missing = judgement_error_from("link", DataError::NotFound);
    assert_eq!(missing.retry, Retry::Never);
    assert_eq!(missing.text, "The link was not recorded.");
}

/// Review Focus 5 (outage half): an outage is "not confirmed", never "nothing changed" — a
/// connection lost mid-commit leaves the outcome unknown.
#[test]
fn an_outage_is_worded_not_confirmed_and_retryable() {
    let outage = anyhow::anyhow!("connection reset");
    assert_eq!(link_error_view(&outage).retry, Retry::Now);
    let text = link_error_view(&outage).text;
    assert!(text.contains("not confirmed"), "{text}");
}

/// #713: a judgement whose COMMIT was interrupted may have been recorded. It is never offered
/// as a live retry: a commit that landed as Outranked leaves the record reading as before, and a
/// second press would overrule the colleague it lost to. The window re-reads instead.
#[test]
fn an_unknown_commit_outcome_rereads_and_is_never_retried_live() {
    let e = cairn_node::db_diagnosis::commit_outcome_unknown(
        id(77),
        anyhow::anyhow!("connection reset"),
    )
    .context("recording the judgement");
    for (act, v) in [
        ("link", link_error_view(&e)),
        ("unlink", crate::link::unlink_view::unlink_error_view(&e)),
    ] {
        assert_eq!(v.retry, Retry::AfterReread, "{act}");
        assert_eq!(
            v.text,
            format!(
                "Whether the {act} was recorded is NOT known — the connection was lost while \
                 event {} was being saved. If it was recorded but a later judgement outranks it, \
                 the chart will look unchanged after it is read again.",
                id(77)
            )
        );
    }
}

/// PR #707 review: a locked key is not a verdict — the clinician unlocks it in seconds and the
/// identical Link then succeeds, so the refusal must keep the button (`Retry::Now`), never
/// take the comparison away and force a second Compare.
#[test]
fn a_locked_key_is_not_a_verdict() {
    let e = key_locked();
    assert_eq!(e.retry, Retry::Now);
    assert!(e.text.contains("unlock"), "it names the remedy");
}

#[test]
fn a_locked_key_names_the_button_that_was_pressed() {
    assert_eq!(
        key_locked().text,
        "your signing key is locked — unlock it, then press \"Link — same person\" again"
    );
    let e = key_locked_for("Unlink — not the same person");
    assert_eq!(e.retry, Retry::Now);
    assert!(e.text.contains("Unlink"), "it names the unlink button");
    assert!(!e.text.contains("press \"Link\""), "never the opposite act");
}

/// A refusal over a changed record: nothing was recorded, and the remedy is a re-read of the
/// whole chart, never a live retry of the same act.
#[test]
fn a_stale_refusal_is_not_recorded_and_re_reads() {
    let view = judgement_error_from("link", DataError::Stale("the record changed".into()));
    assert_eq!(view.retry, Retry::AfterReread);
    assert!(
        view.text.starts_with("The link was not recorded: "),
        "a stale refusal says nothing was recorded"
    );
    assert!(view.text.contains("the record changed"));
}

/// `stale` is the window's own "the screen is behind the record": a re-read, never a live retry.
#[test]
fn stale_re_reads_and_refused_does_not() {
    assert_eq!(stale("x").retry, Retry::AfterReread);
    assert_eq!(refused("x").retry, Retry::Never);
}

// ---- `charts_after`: the set the clinician's own act left (#797) ----

/// The report carries the record as the act left it (the node's read inside the act's own
/// transaction), as ids, so the window's re-read compares against it and never announces the
/// clinician's own change back to them.
#[test]
fn a_report_carries_the_set_its_act_left() {
    let left = ChartSet::new([id(2), id(1)]).unwrap();
    let report = link_report(LinkEffect::TookEffect, &left, &left).left(&left, id(1));
    assert_eq!(
        report.charts_after,
        Some(vec![id(1).to_string(), id(2).to_string()])
    );
}

/// A set that does not hold the chart on screen is ANOTHER chart's record (an outranked link
/// filed under the other subject reads that subject's record): it is not the record the
/// clinician holds, so no set is reported and the re-read compares against the screen.
#[test]
fn a_set_without_the_chart_on_screen_is_not_reported() {
    let others = ChartSet::new([id(2), id(3)]).unwrap();
    assert_eq!(charts_after(&others, id(1)), None);
    assert_eq!(
        charts_after(&ChartSet::single(id(1)), id(1)),
        Some(vec![id(1).to_string()])
    );
}

/// An act that changed nothing (`reload` false: an Outranked link) reports NO set, even when
/// `left` is handed the record it read (#797 final review). Carrying it would move the window's
/// held set and raise its draw floor with no re-read following — dropping a colleague's read
/// already in flight, the first news of a real change.
#[test]
fn an_outranked_link_reports_no_set() {
    let set = ChartSet::new([id(1), id(2)]).unwrap();
    let report = link_report(LinkEffect::Outranked, &set, &set).left(&set, id(1));
    assert!(!report.reload);
    assert_eq!(report.charts_after, None);
}

/// Every report starts with no set: only a command that holds the node's outcome says one
/// ("Different people" never does — it changes no record's set). Serialized as `null`, which
/// the webview reads as "compare against the screen" (an empty list would read as a set that
/// could not be told, and say nothing at all).
#[test]
fn a_report_says_no_set_until_its_command_gives_one() {
    let set = ChartSet::single(id(1));
    let report = link_report(LinkEffect::Outranked, &set, &set);
    assert_eq!(report.charts_after, None);
    let json = serde_json::to_value(&report).unwrap();
    assert!(json["charts_after"].is_null(), "serialized as null");
}

/// The window has no DB-backed tests (#738), so no GUI test reaches the node call: this reads
/// the source. Each of the three acts that can change the record the clinician holds — Link,
/// Unlink, Confirm — reports the set the node read INSIDE its transaction (`outcome.charts`),
/// for the chart on screen. A command that dropped it would announce the clinician's own link
/// back to them on the next re-read, with every other test green.
#[test]
fn link_unlink_and_confirm_report_the_set_their_act_left() {
    use crate::source_guard::rust_fn;
    let link = rust_fn(include_str!("same_person.rs"), "link_impl");
    assert!(link.contains(".left(&outcome.charts, patient)"), "link");
    let unlink = rust_fn(include_str!("unlink.rs"), "unlink_impl");
    assert!(
        unlink.contains(".left(&outcome.charts, edge.opened)"),
        "unlink"
    );
    let confirm = rust_fn(include_str!("confirm.rs"), "confirm_link_impl");
    assert!(
        confirm.contains(".left(&outcome.charts, edge.opened)"),
        "confirm"
    );
}
