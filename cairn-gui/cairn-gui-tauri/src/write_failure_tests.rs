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
    assert_eq!(
        retry_for(&DataError::OutcomeUnknown("x".into())),
        Retry::AfterReread(Recorded::Unknown)
    );
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
        rust_fn(src, "sign_off_impl").contains(".map_err(|e| node_failure(&e))?"),
        "sign_off_impl"
    );
}

/// A failure marked outcome-unknown by the node (#713, #814).
fn interrupted(event: uuid::Uuid) -> anyhow::Error {
    cairn_node::db_diagnosis::commit_outcome_unknown(event, anyhow::anyhow!("connection closed"))
        .context("signing off the medication list")
}

/// #816 review: the shared classifier itself never offers `Retry::Now` for a write that may have
/// landed, so a surface that forgets to word the marker first still re-reads instead of inviting
/// a second, duplicate write (the #713 hazard). Its text is the node's, untouched.
#[test]
fn node_failure_never_offers_a_retry_for_a_commit_that_may_have_landed() {
    let e = interrupted(uuid::Uuid::from_u128(816));
    let view = node_failure(&e);
    assert_eq!(view.retry, Retry::AfterReread(Recorded::Unknown));
    assert_eq!(view.text, format!("{e:#}"));
}

/// #818: what one sign-off did, every combination. Signed, NOT signed, NOT known and never tried
/// are separate facts. "Signed 0" is left out only when the line it stopped at may be signed
/// (the count would claim it is not), and kept when that line wrote nothing. Before #818 an
/// interrupted sign-off was an `Err` that discarded the signed lines and the failures' reasons.
#[test]
fn a_sign_off_outcome_keeps_signed_failed_unknown_and_untried_apart() {
    use cairn_node::medication::signoff::{SignOffInterrupted, StoppedLine};
    let t = uuid::Uuid::from_u128;
    let unknown = SignOffInterrupted {
        medication_id: t(2),
        line: StoppedLine::MayBeSigned { event: t(9) },
        cause: "connection closed".into(),
        not_attempted: vec![t(3)],
    };
    let (text, behind) = sign_off_outcome(1, &[], Some(&unknown), &[]);
    assert_eq!(behind, Some(Recorded::Unknown), "the list may be behind");
    assert!(text.starts_with("Signed 1 medication thread(s)."), "{text}");
    assert!(
        text.contains(&format!("Whether thread {} was signed is NOT known", t(2))),
        "{text}"
    );
    assert!(
        text.contains(&format!("event {}", t(9))),
        "names the event: {text}"
    );
    assert!(
        text.contains("(connection closed)"),
        "names the cause: {text}"
    );
    assert!(
        text.contains(&format!(
            "1 line(s) after it were not tried and are still unsigned: {}.",
            t(3)
        )),
        "{text}"
    );
    assert!(!text.contains("failed"), "never \"failed\": {text}");

    let (text, _) = sign_off_outcome(0, &[], Some(&unknown), &[]);
    assert!(
        !text.contains("Signed 0"),
        "that line may be signed: {text}"
    );
    assert!(
        text.starts_with(&format!("Whether thread {} was signed is NOT known", t(2))),
        "the unknown line is still said: {text}"
    );

    let wrote_nothing = SignOffInterrupted {
        line: StoppedLine::NotSigned,
        not_attempted: vec![],
        ..unknown.clone()
    };
    let (text, behind) = sign_off_outcome(0, &[], Some(&wrote_nothing), &[]);
    assert_eq!(behind, None, "nothing may have landed");
    assert!(text.starts_with("Signed 0 medication thread(s)."), "{text}");
    assert!(
        text.contains(&format!(
            "The sign-off stopped at thread {}, which was NOT signed (connection closed).",
            t(2)
        )),
        "{text}"
    );
    assert!(!text.contains("not tried"), "{text}");

    // Every part at once, in order: signed, failed, where it stopped, untried, the notes.
    let (text, _) = sign_off_outcome(
        1,
        &["thread x: refused".into()],
        Some(&unknown),
        &[Some("Withheld.".into())],
    );
    let at = |part: &str| {
        text.find(part)
            .unwrap_or_else(|| panic!("{part:?} in {text}"))
    };
    assert!(at("Signed 1") < at("could NOT be signed"), "{text}");
    assert!(at("could NOT be signed") < at("is NOT known"), "{text}");
    assert!(at("is NOT known") < at("were not tried"), "{text}");
    assert!(at("were not tried") < at("Withheld."), "{text}");

    // Uninterrupted: the sentence the webview used to build, failed lines and notes in order.
    let (text, behind) = sign_off_outcome(
        2,
        &["thread x: refused".into()],
        None,
        &[Some("Withheld.".into()), None, Some("Across.".into())],
    );
    assert_eq!(behind, None);
    assert_eq!(
        text,
        "Signed 2 medication thread(s). 1 line(s) could NOT be signed and were left unsigned: \
         thread x: refused Withheld. Across."
    );
}

/// #814: what one cease did, every combination. Stopped, NOT stopped, and not known are three
/// separate counts; "Stopped 0" is left out only when a thread's outcome is unknown (that thread
/// may be stopped, so the count would be false), and kept when nothing at all was stopped.
#[test]
fn a_cease_outcome_keeps_stopped_not_stopped_and_not_known_apart() {
    let unknown = [UnknownThread {
        thread: uuid::Uuid::from_u128(1),
        event: uuid::Uuid::from_u128(2),
        cause: "connection closed".to_string(),
    }];
    let failed = [
        "thread x: refused".to_string(),
        "thread y: held back".to_string(),
    ];

    assert_eq!(
        cease_outcome(1, &[], &[]),
        "Stopped 1 thread(s) of this drug."
    );
    assert_eq!(
        cease_outcome(0, &[], &[]),
        "Stopped 0 thread(s) of this drug."
    );
    assert_eq!(
        cease_outcome(1, &failed, &[]),
        "Stopped 1 thread(s) of this drug. 2 thread(s) were NOT stopped and are still active: \
         thread x: refused; thread y: held back"
    );

    let only_unknown = cease_outcome(0, &[], &unknown);
    assert!(
        only_unknown.starts_with("Whether 1 thread(s) of this drug were stopped is NOT known"),
        "no false \"Stopped 0\": {only_unknown}"
    );
    assert!(
        only_unknown.contains(&format!(
            "thread {} (event {}: connection closed)",
            unknown[0].thread, unknown[0].event
        )),
        "{only_unknown}"
    );

    let all = cease_outcome(2, &failed, &unknown);
    assert!(
        all.starts_with("Stopped 2 thread(s) of this drug. 2 thread(s) were NOT stopped"),
        "{all}"
    );
    assert!(
        all.contains("Whether 1 thread(s) of this drug were stopped is NOT known"),
        "{all}"
    );

    let two = [
        unknown[0].clone(),
        UnknownThread {
            thread: uuid::Uuid::from_u128(3),
            event: uuid::Uuid::from_u128(4),
            cause: "x".into(),
        },
    ];
    let both = cease_outcome(0, &failed, &two);
    assert!(both.starts_with("2 thread(s) were NOT stopped"), "{both}");
    assert!(
        both.contains("Whether 2 thread(s) of this drug were stopped is NOT known"),
        "{both}"
    );
    assert!(
        both.contains("connection closed); thread "),
        "joined with \"; \": {both}"
    );
}

/// #814: `cease_impl` sorts each thread's failure by the node's marker — an interrupted commit to
/// the unknown list (and the report marks the list behind the record), anything else to `failed`.
/// The marker's classification is `commit_outcome_unknown_event`'s, unit-tested in cairn-node;
/// what is pinned here is only that `cease_impl` is wired to it.
#[test]
fn cease_sorts_an_interrupted_commit_from_a_failure() {
    let body = crate::source_guard::rust_fn(include_str!("commands.rs"), "cease_impl");
    let flat: String = body.split_whitespace().collect();
    for shape in [
        "Err(e)=>matchcairn_node::db_diagnosis::commit_outcome_unknown_event(&e){",
        "Some(event)=>unknown.push(UnknownThread{thread:medication_id,event,cause:e.root_cause().to_string(),}),",
        "lettext=cease_outcome(ceased,&failed,&unknown);",
        "(!unknown.is_empty()).then_some(Recorded::Unknown)",
    ] {
        assert!(flat.contains(shape), "{shape}\n{body}");
    }
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
