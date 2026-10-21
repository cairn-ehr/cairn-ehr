//! Every arm of the withhold and release outcome sentences (`outcome.rs`). The sentences are
//! Rust, not the webview's, precisely so that each combination — done, not done, not known — is
//! pinned here (ADR-0060 decision 2: partial completion is reported, never implied).
use super::*;
use crate::funnel::view::Recorded;
use uuid::Uuid;

fn unknown(n: u128) -> UnknownThread {
    UnknownThread {
        thread: Uuid::from_u128(n),
        event: Uuid::from_u128(n + 100),
        cause: "connection closed".to_string(),
    }
}

#[test]
fn a_withhold_that_landed_says_the_w_is_recorded() {
    assert_eq!(
        withhold_outcome(None),
        (
            "Withheld — the W is recorded on this drug.".to_string(),
            None
        )
    );
}

/// #814 applied to the W: a commit with no ordinary answer may have written the hold, so the
/// sentence says NOT known — never "Withheld", never a failure — names the thread and the event,
/// and marks the list behind the record so the re-read can say so.
#[test]
fn a_withhold_with_an_unknown_commit_says_it_is_not_known() {
    let u = unknown(1);
    let (text, behind) = withhold_outcome(Some(&u));
    assert_eq!(behind, Some(Recorded::Unknown));
    assert!(
        text.starts_with("Whether this drug was withheld is NOT known"),
        "{text}"
    );
    assert!(
        text.contains(&format!(
            "thread {} (event {}: connection closed)",
            u.thread, u.event
        )),
        "{text}"
    );
    assert!(!text.contains("Withheld —"), "{text}");
}

#[test]
fn a_release_counts_what_it_released() {
    assert_eq!(
        release_outcome(1, 0, 0, &[], &[]),
        "Released 1 hold(s) on this drug."
    );
    assert_eq!(
        release_outcome(0, 0, 0, &[], &[]),
        "Released 0 hold(s) on this drug."
    );
}

/// A hold shown that is no longer on the line (ended, or released by someone else, after the
/// screen was drawn) is counted, not silently dropped: the clinician pressed Release on it.
#[test]
fn a_release_names_holds_no_longer_on_the_line() {
    assert_eq!(
        release_outcome(1, 2, 0, &[], &[]),
        "Released 1 hold(s) on this drug. 2 hold(s) shown are no longer on this line (ended or \
         released meanwhile) and were left as they are."
    );
}

/// A hold on the line the clinician never saw (a colleague's W that arrived after the line was
/// drawn) stays in force (ADR-0081 decision 4) — and the report SAYS so, as information, never
/// as a failure: "Released 1" alone would read as "this drug is now givable".
#[test]
fn a_release_says_a_hold_it_was_not_shown_is_still_in_force() {
    let text = release_outcome(1, 0, 2, &[], &[]);
    assert_eq!(
        text,
        "Released 1 hold(s) on this drug. 2 hold(s) added since this line was drawn are still \
         in force (they were not shown, so were not released)."
    );
    assert!(!text.contains("NOT"), "information, not a failure: {text}");
}

#[test]
fn a_release_names_what_it_did_not_release() {
    let failed = [
        "thread x: refused".to_string(),
        "hold h on thread y: held back".to_string(),
    ];
    assert_eq!(
        release_outcome(1, 0, 0, &failed, &[]),
        "Released 1 hold(s) on this drug. 2 item(s) were NOT released and are still on this line: \
         thread x: refused; hold h on thread y: held back"
    );
}

/// "Released 0" is left out beside an unknown: that thread's holds may be released, so the count
/// would be false. Kept when nothing at all is unknown (above).
#[test]
fn a_release_with_an_unknown_commit_drops_released_zero() {
    let u = [unknown(1)];
    let text = release_outcome(0, 0, 0, &[], &u);
    assert!(!text.contains("Released 0"), "{text}");
    assert!(
        text.starts_with(
            "Whether the holds on 1 thread(s) of this drug were released is NOT known"
        ),
        "{text}"
    );
    assert!(
        text.contains(&format!(
            "thread {} (event {}: connection closed)",
            u[0].thread, u[0].event
        )),
        "{text}"
    );
}

#[test]
fn a_mixed_release_keeps_every_part_in_order() {
    let failed = ["thread x: refused".to_string()];
    let u = [unknown(1), unknown(2)];
    let text = release_outcome(2, 1, 3, &failed, &u);
    let order = [
        "Released 2 hold(s) on this drug.",
        "1 hold(s) shown are no longer on this line",
        "3 hold(s) added since this line was drawn are still in force",
        "1 item(s) were NOT released and are still on this line: thread x: refused",
        "Whether the holds on 2 thread(s) of this drug were released is NOT known",
    ];
    let mut at = 0;
    for part in order {
        let found = text[at..]
            .find(part)
            .unwrap_or_else(|| panic!("{part:?} missing or out of order: {text}"));
        at += found + part.len();
    }
    assert!(
        text.contains("connection closed); thread "),
        "joined with \"; \": {text}"
    );
}
