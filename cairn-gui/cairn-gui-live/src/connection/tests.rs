//! Unit tests for `connection.rs`: the wording, the backoff, and the guard's `Send` pin.
use super::*;

/// #885: a refused reopen's text ENDS in its remedy, so a renderer that wraps it can tell and
/// not say the remedy twice; an outage's text never does, so its wrapper still offers the press.
#[test]
fn only_a_refused_reopen_says_the_operator_remedy() {
    for skipped in [None, Some(Duration::from_millis(500))] {
        assert!(says_operator_remedy(&lost_text(
            "c",
            Some(Refusal::Server),
            skipped
        )));
        assert!(!says_operator_remedy(&lost_text("c", None, skipped)));
    }
    assert!(!says_operator_remedy(
        "this node's signing key k is not enrolled"
    ));
}

/// #888: a line that is not a press (the front door's provisioning line) gets back the bare
/// cause of every text `lost_text` writes, and who refused it if anyone did; any other text
/// is not a failed reopen's, even one that ends in the remedy.
#[test]
fn a_failed_reopen_gives_back_its_bare_cause_and_who_refused() {
    let cause = "connecting to the database: password authentication failed [28P01]";
    for skipped in [None, Some(Duration::from_millis(500))] {
        for refused in [None, Some(Refusal::Server), Some(Refusal::Client)] {
            assert_eq!(
                failed_reopen_cause(&lost_text(cause, refused, skipped)),
                Some((cause, refused)),
                "{refused:?} {skipped:?}"
            );
        }
    }
    assert_eq!(
        failed_reopen_cause("this node's key k is not enrolled"),
        None
    );
    let not_a_reopen = format!(
        "the cause. {}, so {OPERATOR_REMEDY}",
        Refusal::Server.clause()
    );
    assert_eq!(failed_reopen_cause(&not_a_reopen), None);
}

/// #884 review: a refusal the CLIENT raised (a password the window lacks, a server proof
/// that did not check out) never says the node refused: an operator sent to the server's
/// logs would find no rejected login. It still names the operator, never the next press.
#[test]
fn a_client_refusal_is_not_said_to_be_the_nodes() {
    let cause = "connecting to the database: invalid configuration: password missing";
    for skipped in [None, Some(Duration::from_millis(500))] {
        let text = lost_text(cause, Some(Refusal::Client), skipped);
        assert!(!text.contains("refused the connection"), "{text}");
        assert!(text.contains("could not complete the sign-in"), "{text}");
        assert!(says_operator_remedy(&text), "{text}");
        assert!(!text.contains("press again in"), "{text}");
    }
}

/// #876: the cause reaches the screen, and a press the backoff skipped says so instead of
/// reading like a fresh failure.
#[test]
fn a_failed_reopen_names_its_cause_and_whether_this_press_tried() {
    let cause = "connecting to the database: connection refused (os error 61)";
    let now = lost_text(cause, None, None);
    assert!(now.contains("could not be reopened"), "{now}");
    assert!(now.ends_with(cause), "the cause, verbatim: {now}");

    // Nothing retries on a timer: the text names the PRESS that will try, never "is tried".
    let skipped = lost_text(cause, None, Some(Duration::from_millis(500)));
    assert!(skipped.contains("failed 0.5 s ago"), "{skipped}");
    assert!(skipped.contains("press again in 1.5 s"), "{skipped}");
    assert!(skipped.ends_with(cause), "{skipped}");

    // Near the end of the backoff the two figures still add up, and the wait is never 0.0.
    let late = lost_text(cause, None, Some(Duration::from_millis(1960)));
    assert!(late.contains("failed 1.9 s ago"), "{late}");
    assert!(late.contains("press again in 0.1 s"), "{late}");
}

/// A lost connection the server did not refuse is an outage: `Unavailable`, which the
/// window offers to retry, with the cause as its whole text.
#[test]
fn a_lost_connection_is_an_outage_that_keeps_its_cause() {
    let lost = ConnectionLost {
        text: "the cause".into(),
        refused: None,
    };
    assert!(matches!(DataError::from(lost), DataError::Unavailable(t) if t == "the cause"));
}

/// #880: a server that ANSWERED and refused (credentials, `pg_hba.conf`, a dropped database,
/// no CONNECT privilege) refuses every press the same way; only an operator mends it. A
/// server that is busy or starting up may take the next press, so those stay outages.
#[test]
fn only_a_refusing_server_needs_an_operator() {
    // 55000: `ALTER DATABASE … ALLOW_CONNECTIONS false`, an operator's act (#883 review).
    for code in ["28P01", "28000", "3D000", "42501", "55000"] {
        assert!(server_refused(Some(code)), "{code}");
    }
    for code in [
        None,
        Some("53300"),
        Some("57P03"),
        Some("08006"),
        Some("XX000"),
    ] {
        assert!(!server_refused(code), "{code:?}");
    }
}

/// #880: a refused reopen tells the clerk to get an operator, never to press again, and so
/// does a press inside the backoff after one. Both routes a window classifies by (`From`, and
/// `data_error_from` for cease and release, which wrap it in `anyhow`) say operator.
#[test]
fn a_refused_reopen_names_the_operator_by_every_route() {
    let cause = "connecting to the database: password authentication failed [28P01]";
    for skipped in [None, Some(Duration::from_millis(500))] {
        let text = lost_text(cause, Some(Refusal::Server), skipped);
        assert!(text.contains(cause), "{text}");
        assert!(text.contains("until an operator acts"), "{text}");
        assert!(!text.contains("press again in"), "{text}");
    }
    let lost = ConnectionLost {
        text: "t".into(),
        refused: Some(Refusal::Server),
    };
    assert!(matches!(DataError::from(lost.clone()), DataError::NotProvisioned(t) if t == "t"));
    let wrapped = anyhow::Error::from(lost);
    assert!(matches!(
        crate::error::data_error_from(&wrapped),
        DataError::NotProvisioned(t) if t == "t"
    ));
    let outage = anyhow::Error::from(ConnectionLost {
        text: "t".into(),
        refused: None,
    });
    assert!(matches!(
        crate::error::data_error_from(&outage),
        DataError::Unavailable(_)
    ));
}

#[test]
fn a_reopen_waits_out_the_backoff_after_a_failure() {
    let retry_after = NodeConnection::RETRY_AFTER;
    let failed = Instant::now();
    assert!(may_retry(None, failed), "no failure yet: try at once");
    assert!(!may_retry(Some(failed), failed), "just failed: wait");
    assert!(!may_retry(
        Some(failed),
        failed + retry_after - Duration::from_millis(1)
    ));
    assert!(may_retry(Some(failed), failed + retry_after));
}

/// Every Tauri command holds the guard across `.await`s, so its future must be `Send`.
/// Pinned here, where the guard is made; otherwise only cairn-gui-tauri's build would
/// catch a change (a `std` mutex, say) that takes `Send` away.
#[allow(dead_code)]
fn the_guard_may_be_held_across_an_await(db: &NodeConnection) -> impl Send + '_ {
    async move {
        let client = db.lock().await;
        std::future::ready(()).await;
        drop(client);
    }
}
