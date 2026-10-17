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
