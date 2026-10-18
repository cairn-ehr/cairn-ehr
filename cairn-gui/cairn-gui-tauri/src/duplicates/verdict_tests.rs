//! Goldens for the "Different people" outcome and failure sentences (R5a): one line per pair,
//! a failed pair named, never hidden behind another's verdict, and a commit whose outcome is
//! unknown never retried live (#713). Split from `view_tests.rs` (#789); tests `verdict.rs` (#745).
use super::*;
use crate::funnel::view::{ErrorView, Recorded, Retry};
use cairn_node::chart_link::LinkEffect;
use uuid::Uuid;

fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}

#[test]
fn different_people_reports_each_pair_and_reloads() {
    let r = different_people_report(vec![PairResult {
        low: id(1),
        high: id(9),
        outcome: Ok(LinkEffect::TookEffect),
    }])
    .unwrap();
    assert!(r.reload);
    assert_eq!(
        r.sentence,
        format!(
            "Recorded: charts {} and {} are different people. This pair is closed on \
             this node, and on every node once the judgement syncs.",
            id(1),
            id(9)
        )
    );
}

#[test]
fn a_partly_failed_judgement_names_the_failed_pair() {
    let r = different_people_report(vec![
        PairResult {
            low: id(1),
            high: id(8),
            outcome: Ok(LinkEffect::TookEffect),
        },
        PairResult {
            low: id(2),
            high: id(9),
            outcome: Err(refused("held elsewhere")),
        },
    ])
    .unwrap();
    assert!(r.sentence.ends_with(&format!(
        "For charts {} and {}, held elsewhere.",
        id(2),
        id(9)
    )));
}

/// #740: a failed pair's sentence is led by its charts and CONTINUES the error's own sentence —
/// never "Not confirmed for charts …: The … was not confirmed: …" (a stutter, with a capital
/// after the colon), and never "Not confirmed" in front of a refusal, which is decided.
#[test]
fn a_failed_pair_continues_its_own_sentence_without_a_stutter() {
    let outage = |text: &str| ErrorView {
        text: text.into(),
        retry: Retry::Now,
    };
    let r = different_people_report(vec![
        PairResult {
            low: id(1),
            high: id(8),
            outcome: Ok(LinkEffect::TookEffect),
        },
        PairResult {
            low: id(2),
            high: id(9),
            outcome: Err(outage(
                "The \"different people\" judgement was not confirmed: connection reset",
            )),
        },
    ])
    .unwrap();
    assert!(
        r.sentence.ends_with(&format!(
            "For charts {} and {}, the \"different people\" judgement was not confirmed: \
             connection reset.",
            id(2),
            id(9)
        )),
        "{}",
        r.sentence
    );
    assert!(!r.sentence.contains("Not confirmed for"), "{}", r.sentence);
}

/// The continuation lowers only a capitalised first WORD: an all-capitals word ("NOT", an
/// acronym) and a text already lower-case are left exactly as they are. **Pure.**
#[test]
fn only_a_capitalised_first_word_is_lowered() {
    assert_eq!(continuation("The node refused"), "the node refused");
    assert_eq!(continuation("Whether it was"), "whether it was");
    assert_eq!(continuation("NOT known"), "NOT known");
    assert_eq!(continuation("RLS refused"), "RLS refused");
    assert_eq!(continuation("held elsewhere"), "held elsewhere");
    assert_eq!(continuation("A"), "A");
    assert_eq!(continuation(""), "");
}

/// What `unlink_charts` returns when the connection drops DURING the commit, built the way the
/// node builds it (#713: `commit_outcome_unknown`, the marker), as the window words it: outcome
/// unknown, and `AfterReread` — the webview re-reads rather than keeping a live retry.
fn commit_unknown() -> ErrorView {
    different_people_error_view(&cairn_node::db_diagnosis::commit_outcome_unknown(
        id(77),
        anyhow::anyhow!("connection reset"),
    ))
}

/// The commit-unknown sentence for "Different people", as `judgement_error_view` words it.
fn commit_unknown_text() -> String {
    format!(
        "Whether the \"different people\" judgement was recorded is NOT known — no ordinary \
         answer came back while event {} was being saved. If it was recorded but a later judgement \
         outranks it, the chart will look unchanged after it is read again.",
        id(77)
    )
}

/// Final review I2: a lone commit-unknown pair is its own ErrorView, unchanged — and since #713
/// that view re-reads (`AfterReread`) instead of keeping the button live.
#[test]
fn a_lone_commit_unknown_pair_is_its_own_error_unchanged() {
    let err = different_people_report(vec![PairResult {
        low: id(1),
        high: id(9),
        outcome: Err(commit_unknown()),
    }])
    .unwrap_err();
    assert_eq!(err, commit_unknown());
    assert_eq!(err.retry, Retry::AfterReread(Recorded::Unknown));
    assert_eq!(err.text, commit_unknown_text());
}

/// Final review I2: a commit-unknown pair beside a recorded one says its outcome is NOT known —
/// never "NOT recorded", and never "it stays on the banner": it may have been recorded, and only
/// the banner's re-read (`reload`) says which.
#[test]
fn a_commit_unknown_pair_beside_a_recorded_one_is_never_called_not_recorded() {
    let r = different_people_report(vec![
        PairResult {
            low: id(1),
            high: id(8),
            outcome: Ok(LinkEffect::TookEffect),
        },
        PairResult {
            low: id(2),
            high: id(9),
            outcome: Err(commit_unknown()),
        },
    ])
    .unwrap();
    assert!(
        r.reload,
        "the banner must re-read to learn what the lost commit did"
    );
    assert_eq!(
        r.sentence,
        format!(
            "Recorded: charts {} and {} are different people. This pair is closed on this node, \
             and on every node once the judgement syncs. For charts {} and {}, {}",
            id(1),
            id(8),
            id(2),
            id(9),
            continuation(&commit_unknown_text())
        )
    );
    assert!(!r.sentence.contains("not recorded"), "{}", r.sentence);
}

/// Final review I2 (the #713 hazard): when EVERY pair failed, a refused first pair must not
/// hide a commit-unknown second one. One ErrorView names both, and carries the commit-unknown's
/// `AfterReread` — a `Never` would claim a decided outcome that may not exist, and a `Now` would
/// invite a press over a record that may already have changed.
#[test]
fn every_failed_pair_is_named_and_a_commit_unknown_is_never_hidden_behind_a_verdict() {
    let err = different_people_report(vec![
        PairResult {
            low: id(1),
            high: id(8),
            outcome: Err(refused(
                "The \"different people\" judgement was refused: held elsewhere",
            )),
        },
        PairResult {
            low: id(2),
            high: id(9),
            outcome: Err(commit_unknown()),
        },
    ])
    .unwrap_err();
    assert_eq!(err.retry, Retry::AfterReread(Recorded::Unknown));
    assert_eq!(
        err.text,
        format!(
            "For charts {} and {}, the \"different people\" judgement was refused: held \
             elsewhere. For charts {} and {}, {}",
            id(1),
            id(8),
            id(2),
            id(9),
            continuation(&commit_unknown_text())
        )
    );
}

/// The combined retry is the most retryable class among the failed pairs, whatever their order.
#[test]
fn the_combined_retry_is_the_most_retryable_among_the_failed_pairs() {
    let fail = |n: u128, retry: Retry| PairResult {
        low: id(n),
        high: id(n + 50),
        outcome: Err(ErrorView {
            text: "x".into(),
            retry,
        }),
    };
    let combined = |rs: Vec<PairResult>| different_people_report(rs).unwrap_err().retry;
    assert_eq!(
        combined(vec![fail(1, Retry::Now), fail(2, Retry::Never)]),
        Retry::Now
    );
    assert_eq!(
        combined(vec![fail(1, Retry::Never), fail(2, Retry::AfterOperator)]),
        Retry::AfterOperator
    );
    assert_eq!(
        combined(vec![fail(1, Retry::Never), fail(2, Retry::Never)]),
        Retry::Never
    );
    // #713 / #797: a screen behind the record outranks every other class — the window must
    // re-read before anything is offered again, whatever the other pairs said.
    let stale = Retry::AfterReread(Recorded::No);
    let unknown = Retry::AfterReread(Recorded::Unknown);
    assert_eq!(combined(vec![fail(1, stale), fail(2, Retry::Now)]), stale);
    assert_eq!(combined(vec![fail(1, Retry::Never), fail(2, stale)]), stale);
    // #811 review: a stale refusal outranks an unknown outcome, in either order. It PROVES the
    // list is out of date, and the unknown pair's own sentence still says its outcome is unknown.
    assert_eq!(combined(vec![fail(1, unknown), fail(2, stale)]), stale);
    assert_eq!(combined(vec![fail(1, stale), fail(2, unknown)]), stale);
    assert_eq!(
        combined(vec![fail(1, unknown), fail(2, Retry::Now)]),
        unknown
    );
}

#[test]
fn a_wholly_failed_judgement_is_the_refusal_itself() {
    let err = different_people_report(vec![PairResult {
        low: id(1),
        high: id(9),
        outcome: Err(refused("held elsewhere")),
    }])
    .unwrap_err();
    assert_eq!(err.text, "held elsewhere");
    assert_eq!(
        different_people_report(vec![]).unwrap_err().text,
        NOTHING_OPEN
    );
}

/// Silent-failure review M1: "Different people" was pressed over two charts that were never
/// linked, so its failure must never read "The unlink was …" — that would send the clinician
/// looking for a link to undo.
#[test]
fn a_different_people_failure_is_never_worded_as_an_unlink() {
    let e = different_people_error_view(&anyhow::anyhow!("connection reset"));
    assert_eq!(
        e.text,
        "The \"different people\" judgement was not confirmed: connection reset"
    );
    assert_eq!(e.retry, Retry::Now);
}

#[test]
fn still_joined_and_outranked_sentences_are_pinned() {
    let r = different_people_report(vec![
        PairResult {
            low: id(1),
            high: id(8),
            outcome: Ok(LinkEffect::StillJoined),
        },
        PairResult {
            low: id(2),
            high: id(9),
            outcome: Ok(LinkEffect::Outranked),
        },
    ])
    .unwrap();
    assert_eq!(
        r.sentence,
        format!(
            "Recorded that charts {} and {} are different people — but they read as one record \
             through other links, so nothing was split. The links joining them are listed under \
             \"How these charts are linked\". Recorded that charts {} and {} are different people, \
             but a judgement already standing for that pair outranks it; the charts stay as that \
             judgement left them.",
            id(1),
            id(8),
            id(2),
            id(9)
        )
    );
}

/// #797 review: a partly-recorded "Different people" is a SUCCESS report, yet one of its pairs may
/// have been refused because the record changed under the screen (`Recorded::No`), or its commit
/// was interrupted (#713, `Recorded::Unknown`). The report carries which, so the window's re-read
/// knows the list is behind the record, and words it for the cause (#811). Of several such pairs,
/// `No` wins, because it proves the list out of date. Any other failure, or none, leaves it `None`.
#[test]
fn a_partly_recorded_judgement_says_when_a_failed_pair_was_behind_the_record() {
    let recorded = |n: u128| PairResult {
        low: id(n),
        high: id(n + 50),
        outcome: Ok(LinkEffect::TookEffect),
    };
    let failed = |n: u128, retry: Retry| PairResult {
        low: id(n),
        high: id(n + 50),
        outcome: Err(ErrorView {
            text: "x".into(),
            retry,
        }),
    };
    let stale = Retry::AfterReread(Recorded::No);
    let unknown = Retry::AfterReread(Recorded::Unknown);
    let behind = |rs: Vec<PairResult>| different_people_report(rs).unwrap().behind_the_record;
    assert_eq!(
        behind(vec![recorded(1), failed(2, stale)]),
        Some(Recorded::No)
    );
    assert_eq!(
        behind(vec![recorded(1), failed(2, unknown)]),
        Some(Recorded::Unknown)
    );
    assert_eq!(
        behind(vec![recorded(1), failed(2, unknown), failed(3, stale)]),
        Some(Recorded::No),
        "a proven stale pair outranks an unknown one"
    );
    for other in [Retry::Now, Retry::AfterOperator, Retry::Never] {
        assert_eq!(
            behind(vec![recorded(1), failed(2, other)]),
            None,
            "{other:?}"
        );
    }
    assert_eq!(
        behind(vec![recorded(1), recorded(2)]),
        None,
        "nothing failed"
    );
}
