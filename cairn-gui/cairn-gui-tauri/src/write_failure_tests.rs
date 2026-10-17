use super::*;

#[test]
fn each_class_earns_its_retry() {
    assert_eq!(retry_for(&DataError::Stale("x".into())), Retry::AfterReread);
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

/// The node's own authoritative stale refusal (`ensure_same_charts`' producer) reaches sign-off
/// through `node_failure`; it must come out `AfterReread`, the retry Task 5's re-read acts on.
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
    assert_eq!(view.retry, Retry::AfterReread);
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
    }
}
