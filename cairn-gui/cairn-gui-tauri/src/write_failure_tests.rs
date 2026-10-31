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
        rust_fn(src, "sign_off_impl").contains(".map_err(|e| node_failure(&db.explain(e)))?"),
        "sign_off_impl"
    );
    // #893: a sign-off the window cut off names the cut, read while the guard is held.
    assert!(
        rust_fn(src, "sign_off_impl")
            .contains("name_the_cut(outcome.interrupted.as_mut(), db.cut_off());"),
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
        "Some(event)=>unknown.push(UnknownThread{thread:medication_id,event,cause:cairn_gui_live::cause_of(&e),}),",
        // #893: a hold the window cut off says so, here and in the cause above.
        ".await.map_err(|e|db.explain(e))",
        "lettext=cease_outcome(ceased,&failed,&unknown);",
        // #843: a Stop that landed on a line already stopped says so — only when every thread
        // of this press stopped for certain (#853 review: not beside a failure or an unknown).
        "letlanded_in_full=ceased>0&&failed.is_empty()&&unknown.is_empty();",
        "lettext=matchalready_stopped_note(row,landed_in_full){Some(note)=>format!(\"{text}{note}\"),None=>text,};",
        "(!unknown.is_empty()).then_some(Recorded::Unknown)",
    ] {
        assert!(flat.contains(shape), "{shape}\n{body}");
    }
}

/// #876: in cease and release a lost connection fails THAT thread, reported with the others, so
/// an earlier thread already written is never covered by "Nothing was done". A plain `?` on the
/// lock would compile (`ErrorView: From<ConnectionLost>`) and pass every behavioural test: only
/// a backend killed BETWEEN two threads reaches it, and the window has no DB-backed tests (#738).
/// Gesture timing, after the act, must never spend the reopen the act's re-read needs.
#[test]
fn a_lost_connection_fails_one_thread_and_timing_never_reopens() {
    for (src, name) in [
        (include_str!("commands.rs"), "cease_impl"),
        (include_str!("hold/mod.rs"), "release_impl"),
    ] {
        let flat: String = crate::source_guard::rust_fn(src, name)
            .split_whitespace()
            .collect();
        assert!(
            flat.contains(
                "letresult=matchdb.lock().await{Err(lost)=>Err(anyhow::Error::from(lost)),"
            ),
            "{name}: a lost connection is this thread's failure"
        );
        assert!(
            !flat.contains(".lock().await?"),
            "{name}: never `?` on the lock"
        );
    }
    let timing = crate::source_guard::rust_fn(include_str!("commands.rs"), "record_timing");
    assert!(timing.contains(".lock_if_open()"), "{timing}");
    assert!(!timing.contains(".lock()"), "{timing}");
}

/// A chart read that failed before anything was written is an OUTAGE: nothing was decided, so
/// the act may be retried (`Retry::Now`), as `link::chart_set_of` words the same fault. Its text
/// passes through untouched.
#[test]
fn a_failed_chart_read_before_the_write_may_be_retried() {
    let view = chart_read_failed(anyhow::anyhow!("connection reset"));
    assert_eq!(view.retry, Retry::Now);
    assert_eq!(view.text, "connection reset");
}

/// #880 review: a write gesture's first chart read is where a dead connection is REOPENED, so a
/// reopen the server refused must reach the button as an operator's job, never a retry-now
/// beside a text saying pressing again fails the same way. `read_chart_of` keeps the lost
/// connection typed (a plain `?` on the lock, never mapped to text) so this can see it; the
/// `ErrorView` conversion that
/// link, unlink, same-person and duplicates reach with `?` is pinned here too.
#[test]
fn a_chart_read_lost_to_a_refused_reopen_needs_an_operator() {
    use cairn_gui_live::ConnectionLost;
    let lost = |refused| anyhow::Error::from(ConnectionLost::for_tests("cause [28P01]", refused));
    let refused = chart_read_failed(lost(true));
    assert_eq!(refused.retry, Retry::AfterOperator);
    assert!(refused.text.contains("cause [28P01]"), "{}", refused.text);
    assert_eq!(chart_read_failed(lost(false)).retry, Retry::Now);
    for (refused, retry) in [(true, Retry::AfterOperator), (false, Retry::Now)] {
        let view = ErrorView::from(ConnectionLost::for_tests("t", refused));
        assert_eq!(view.retry, retry, "refused: {refused}");
    }
    let read = crate::source_guard::rust_fn(include_str!("commands.rs"), "read_chart_of");
    assert!(
        read.contains("let mut db = db.lock().await?;"),
        "the lost connection stays typed, never mapped to text: {read}"
    );
}

/// #840's rule (maintainer's ruling, 2026-10-22): a gesture whose every write failed answers with
/// ONE class. Stale wins — any failure that proved the list out of date makes the whole gesture
/// "re-read first", the safe superset (a re-read never hurts; a live retry over a stale list is
/// refused again). Otherwise the first failure's class. No failure, no class: nothing was tried
/// (every thread held back), and the caller's report stands.
#[test]
fn a_gesture_that_wrote_nothing_takes_one_class_and_stale_wins() {
    let stale = Retry::AfterReread(Recorded::No);
    let nothing = |retries: &[Retry]| retry_when_nothing_written(0, &[], retries);
    assert_eq!(nothing(&[]), None);
    assert_eq!(nothing(&[Retry::Never]), Some(Retry::Never));
    assert_eq!(
        nothing(&[Retry::Now, Retry::Never]),
        Some(Retry::Now),
        "the first failure's class"
    );
    assert_eq!(
        nothing(&[Retry::Never, stale, Retry::Now]),
        Some(stale),
        "stale wins wherever it is"
    );
}

/// #840: only a gesture that wrote NOTHING is a refusal. One write done, or one whose outcome is
/// unknown (it may have landed), makes it a report naming every part — whatever else failed.
#[test]
fn a_gesture_that_wrote_or_may_have_written_is_a_report() {
    let failed = [Retry::AfterReread(Recorded::No)];
    assert_eq!(
        retry_when_nothing_written(1, &[], &failed),
        None,
        "one written"
    );
    let unknown = [UnknownThread {
        thread: uuid::Uuid::from_u128(1),
        event: uuid::Uuid::from_u128(2),
        cause: "connection reset".into(),
    }];
    assert_eq!(
        retry_when_nothing_written(0, &unknown, &failed),
        None,
        "one may have been written"
    );
}

/// #843 (maintainer's decision): a Stop that reaches a line the record already shows as stopped
/// (a colleague's cease landed after this screen was drawn) is still written — its reason may be
/// the only record of, say, an adverse reaction — and the outcome SAYS the line was already
/// stopped, with the earlier stop's reason, so "Stopped 1" is not read as "I stopped it". A line
/// still active says nothing extra.
#[test]
fn a_stop_of_a_line_already_stopped_says_so() {
    let rows = cairn_medication_view::fixtures::sample_rows();
    let ceased = rows
        .iter()
        .find(|r| r.status == cairn_medication_view::MedicationStatus::Ceased)
        .expect("the fixture has a ceased line");
    let note = already_stopped_note(ceased, true).expect("a ceased line earns the note");
    assert!(note.contains("already been stopped"), "{note}");
    assert!(note.contains("gastric irritation"), "{note}");
    assert!(note.contains("recorded"), "{note}");

    let mut no_reason = ceased.clone();
    no_reason.cessation_reason = None;
    let note = already_stopped_note(&no_reason, true).unwrap();
    assert!(note.contains("no reason was recorded"), "{note}");

    let active = rows
        .iter()
        .find(|r| r.status == cairn_medication_view::MedicationStatus::Active)
        .unwrap();
    assert_eq!(already_stopped_note(active, true), None);

    // A press that did not land in full — a commit got no answer (#814), or a thread failed —
    // must not say "your stop is recorded" beside the sentence saying it is NOT known, or NOT
    // stopped (#853 review).
    assert_eq!(already_stopped_note(ceased, false), None);
}

/// #893: a sign-off the window cut off names the cut, not the bare "connection closed" a node
/// restart leaves too; a stop with no cut keeps its cause.
#[test]
fn a_cut_off_sign_off_names_the_cut() {
    use cairn_node::medication::signoff::{SignOffInterrupted, StoppedLine};
    let stopped = || SignOffInterrupted {
        medication_id: uuid::Uuid::from_u128(2),
        line: StoppedLine::NotSigned,
        cause: "connection closed".into(),
        not_attempted: vec![],
    };
    let cut = cairn_gui_live::CutOff::for_tests(std::time::Duration::from_secs(15));

    let mut cut_off = stopped();
    name_the_cut(Some(&mut cut_off), Some(cut));
    assert_eq!(cut_off.cause, cut.to_string());
    assert!(cut_off.cause.contains("within 15 s"), "{}", cut_off.cause);

    let mut restarted = stopped();
    name_the_cut(Some(&mut restarted), None);
    assert_eq!(restarted.cause, "connection closed");
}
