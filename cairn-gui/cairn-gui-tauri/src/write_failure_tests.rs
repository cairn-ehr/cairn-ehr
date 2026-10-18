use super::*;

#[test]
fn each_class_earns_its_retry() {
    assert_eq!(
        retry_for(&DataError::Stale("x".into())),
        Retry::AfterReread(Recorded::No)
    );
    assert_eq!(retry_for(&DataError::Refused("x".into())), Retry::Never);
    assert_eq!(retry_for(&DataError::NotFound), Retry::Never);
    assert_eq!(
        retry_for(&DataError::NotProvisioned("x".into())),
        Retry::AfterOperator
    );
    assert_eq!(retry_for(&DataError::Unavailable("x".into())), Retry::Now);
}

/// An unclassified failure is an outage: the node's text is kept and the act can be retried.
#[test]
fn an_unclassified_failure_keeps_its_text_and_may_retry() {
    let view = node_failure(&anyhow::anyhow!("connection refused"));
    assert_eq!(view.text, "connection refused");
    assert_eq!(view.retry, Retry::Now);
}

/// A real `stale_picture_refusal` (built here through `stale_record`; sign-off's
/// `ensure_same_charts` and `ensure_same_targets` raise the same scope) reaches the webview through
/// `node_failure`: it must come out `AfterReread`, the retry `signOff`'s stale re-read acts on.
#[test]
fn a_real_stale_refusal_is_after_reread_with_the_nodes_text() {
    use cairn_medication_view::ChartSet;
    use cairn_node::chart_link::shown::{stale_record, RecordsShown};
    let (a, b, c) = (
        uuid::Uuid::from_u128(1),
        uuid::Uuid::from_u128(2),
        uuid::Uuid::from_u128(3),
    );
    let shown_then = ChartSet::new([a, b]).expect("a non-empty record");
    let now = ChartSet::new([a, b, c]).expect("a non-empty record");
    let shown = RecordsShown {
        this: &shown_then,
        other: &shown_then,
        pair: None,
    };
    let e = stale_record(shown, a, &now).expect("a changed record is refused");
    let view = node_failure(&e);
    assert_eq!(view.retry, Retry::AfterReread(Recorded::No));
    assert_eq!(view.text, format!("{e:#}"));
}

#[test]
fn a_real_verdict_is_never_and_a_real_node_state_refusal_is_after_operator() {
    let e = cairn_node::patient::register::dob_precision("3/2/1980")
        .expect_err("a malformed birth date refuses")
        .context("registering the patient");
    let view = node_failure(&e);
    assert_eq!(view.retry, Retry::Never);
    assert_eq!(view.text, format!("{e:#}"));
    let e = cairn_node::actor_enrolment::not_enrolled_refusal("9f3c")
        .context("registering the patient");
    let view = node_failure(&e);
    assert_eq!(view.retry, Retry::AfterOperator);
    assert_eq!(view.text, format!("{e:#}"));
}

/// A locked key keeps the button; a node that cannot write needs its operator. Pinned at the
/// helpers AND at the two commands that must use them (the branches need a live key/db).
#[test]
fn a_locked_key_may_retry_and_a_node_that_cannot_write_needs_its_operator() {
    use crate::source_guard::rust_fn;
    assert_eq!(key_locked("x").retry, Retry::Now);
    assert_eq!(node_cannot_write("x").retry, Retry::AfterOperator);
    let src = include_str!("commands.rs");
    for f in ["sign_off_impl", "cease_impl"] {
        let body = rust_fn(src, f);
        assert!(
            body.contains("key_locked(\"your signing key is locked"),
            "{f}"
        );
        assert!(body.contains("node_cannot_write(\"no node key\")"), "{f}");
        assert!(
            body.contains("node_cannot_write(\"no database connection\")"),
            "{f}"
        );
        // The pre-write chart read failing is an outage, not a verdict (#797 review).
        assert!(body.contains(".map_err(chart_read_failed)?"), "{f}");
    }
    // The orchestrator's own compare is the only thing that sees a colleague's change landing
    // after the window's snapshot: its `Stale` refusal must reach the webview classified, never
    // as a plain string (#797 review — a revert to `refused(format!(…))` reads naturally here).
    assert!(
        rust_fn(src, "sign_off_impl").contains(".map_err(|e| sign_off_failure(&e))?"),
        "sign_off_impl"
    );
}

/// #814: a sign-off whose line commit was lost mid-flight (the node's marker) is NOT said to
/// have failed: whether it was recorded is not known, so the window re-reads (`Unknown`) and
/// the sentence names the event. Every other failure is `node_failure`'s, unchanged.
#[test]
fn an_interrupted_sign_off_commit_is_not_known_never_failed() {
    let event = uuid::Uuid::from_u128(814);
    let e = cairn_node::db_diagnosis::commit_outcome_unknown(
        event,
        anyhow::anyhow!("connection closed"),
    )
    .context("signing off the medication list");
    let view = sign_off_failure(&e);
    assert_eq!(view.retry, Retry::AfterReread(Recorded::Unknown));
    assert!(
        view.text
            .starts_with("Whether the sign-off was recorded is NOT known"),
        "{}",
        view.text
    );
    assert!(view.text.contains(&event.to_string()), "names the event");
    assert!(
        !view.text.contains("failed"),
        "never \"failed\": {}",
        view.text
    );

    let outage = anyhow::anyhow!("connection refused");
    assert_eq!(sign_off_failure(&outage), node_failure(&outage));
}

/// #814: a cease thread whose commit was lost mid-flight is neither stopped nor "NOT stopped and
/// still active": its own sentence says whether it was stopped is NOT known, naming the thread
/// and the event. No such thread, no sentence (and no mark on the list).
#[test]
fn an_interrupted_cease_commit_names_each_thread_as_not_known() {
    assert_eq!(cease_unknown_message(&[]), None);
    let (thread, event) = (uuid::Uuid::from_u128(1), uuid::Uuid::from_u128(2));
    let text = cease_unknown_message(&[(thread, event)]).expect("one unknown thread");
    assert!(
        text.starts_with("Whether 1 thread(s) of this drug were stopped is NOT known"),
        "{text}"
    );
    assert!(
        text.contains(&format!("thread {thread} (event {event})")),
        "{text}"
    );
    assert!(!text.contains("still active:"), "{text}");
}

/// #814: `cease_impl` sorts each thread's failure by the node's marker — an interrupted commit to
/// the unknown list (and the report marks the list behind the record), anything else to `failed`.
/// The branch needs a lost connection mid-commit, so it is pinned by source.
#[test]
fn cease_sorts_an_interrupted_commit_from_a_failure() {
    let body = crate::source_guard::rust_fn(include_str!("commands.rs"), "cease_impl");
    assert!(body.contains("commit_outcome_unknown_event(&e)"), "{body}");
    assert!(
        body.contains("Some(event) => unknown.push((medication_id, event))"),
        "{body}"
    );
    assert!(
        body.contains("unknown_message: cease_unknown_message(&unknown)"),
        "{body}"
    );
    assert!(
        body.contains("(!unknown.is_empty()).then_some(Recorded::Unknown)"),
        "{body}"
    );
}

/// A chart read that failed before anything was written is an OUTAGE: nothing was decided, so
/// the act may be retried (`Retry::Now`), as `link::chart_set_of` words the same fault. Its text
/// passes through untouched.
#[test]
fn a_failed_chart_read_before_the_write_may_be_retried() {
    let view = chart_read_failed("connection reset".into());
    assert_eq!(view.retry, Retry::Now);
    assert_eq!(view.text, "connection reset");
}
