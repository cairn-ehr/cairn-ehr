//! Tests for `notes_review.rs` (sign-in slice 3c-3a): whether a note offers "Co-sign as
//! reviewed" to the who's-here claim, and every sentence the press says. Pure, but for the
//! refusals against `AppState::mock`; the DB-gated tests are `notes_review_db_tests.rs`. The note
//! fixture is Sign off's (`notes_sign_off_tests.rs`).
use super::*;
use crate::funnel::view::Retry;
use crate::notes::sign_off::tests::{id, one, part, view, who, OPENED};
use cairn_node::note::read::NoteRow;

/// Part `n`, signed by `writer`, whose supervisors now are `sups` (actor ids).
fn supervised(n: u128, writer: &str, sups: &[&str]) -> NoteRow {
    let mut p = part(n, OPENED, None, 0, Some(who(writer, writer)), None);
    p.writer_supervisors = sups.iter().map(|s| s.to_string()).collect();
    p
}

/// The button follows the node's rule (`review::can_review`) for the who's-here claim's actor:
/// offered to the writer's supervisor, never to the writer, a stranger, an unknown key's empty
/// actor, or with nobody named. The claim only decides what is OFFERED; the key decides what is
/// signed (`notes_review_db_tests.rs`).
#[test]
fn can_review_follows_the_claims_supervision() {
    let c = one(vec![supervised(1, "act-bea", &["act-sue"])]);
    assert!(view(&c, Some("act-sue")).can_review);
    for actor in [None, Some("act-bea"), Some("act-zed"), Some("")] {
        assert!(!view(&c, actor).can_review, "{actor:?}");
    }
    // A part that owes a co-sign is Sign off's to clear, never a review's.
    let mut owed = supervised(1, "act-bea", &["act-sue"]);
    owed.needs_cosigning = true;
    assert!(!view(&one(vec![owed]), Some("act-sue")).can_review);
}

/// Each sentence is the exact one (constraints' copy), the refusals ask for a re-read, and none
/// says "signed off": a review is a co-signature, not a sign-off.
#[test]
fn review_words_are_exact() {
    assert_eq!(
        REVIEWED,
        "Co-signed as reviewed — the note now carries your co-signature."
    );
    let changed = review_refusal_view(SignOffRefusal::Changed);
    assert_eq!(
        changed.text,
        "This note changed since it was shown — read the chart again, then co-sign it. \
         Nothing was signed."
    );
    assert_eq!(changed.retry, Retry::AfterReread(Recorded::No));
    let nothing = review_refusal_view(SignOffRefusal::NothingToSign);
    assert_eq!(
        nothing.text,
        "There is nothing here for you to co-sign as reviewed. Nothing was signed."
    );
    assert_eq!(nothing.retry, Retry::AfterReread(Recorded::No));
    let unknown = review_unknown_words(id(5), "connection reset");
    assert_eq!(
        unknown,
        format!(
            "Whether this note was co-signed is NOT known — no ordinary answer came back while \
             co-signing it (event {}: connection reset). Once the window can reach the node \
             again, the chart shows whether it was co-signed.",
            id(5)
        )
    );
    let e = anyhow::anyhow!("note attestation: the floor said no");
    let failed = review_failure(&e);
    assert_eq!(
        failed.text,
        "Could not co-sign this note: note attestation: the floor said no"
    );
    assert_eq!(failed.retry, node_failure(&e).retry);
    for said in [
        REVIEWED,
        &changed.text,
        &nothing.text,
        &unknown,
        &failed.text,
    ] {
        assert!(!said.contains("signed off"), "{said}");
    }
}

/// The chart on screen and fixture mode come first, as Sign off's do.
#[tokio::test]
async fn review_refuses_another_chart_and_fixture_mode() {
    let chart = id(OPENED);
    let state = AppState::mock(Some(chart));
    let shown = vec![id(1).to_string()];
    let err = note_review_cosign_impl(&state, &id(424).to_string(), "x", &shown, None)
        .await
        .unwrap_err();
    assert!(err.text.contains("not the chart"), "{}", err.text);
    let err = note_review_cosign_impl(&state, &chart.to_string(), "x", &shown, None)
        .await
        .unwrap_err();
    assert!(err.text.contains("fixture mode"), "{}", err.text);
}

/// `note_sign_off_impl`'s order, exactly (ADR-0092 decision 14): the displayed chart, fixture
/// mode, the ids (before the key: a passphrase that signs nothing is a step paper never has),
/// the key, ONE node call, then the unknown outcome and the timing sample under Sign off's kind.
/// A node failure leads with the review's own words, never Sign off's.
#[test]
fn review_follows_sign_offs_order() {
    let flat: String =
        crate::source_guard::rust_fn(include_str!("notes_review.rs"), "note_review_cosign_impl")
            .split_whitespace()
            .collect();
    let mut at = 0;
    for step in [
        "state.displayed_patient(patient_id)",
        "state.is_mock()",
        "sign_off_ids(root_event_id,shown_parts)?",
        "signer_for_act(credential,Now::read())",
        "review_note(&mutdb,&input,&author)",
        "commit_outcome_unknown_event(&e)",
        "record_timing(state,\"note-sign-off\",1,elapsed_ms)",
    ] {
        let found = flat[at..]
            .find(step)
            .unwrap_or_else(|| panic!("{step} in {flat}"));
        at += found + step.len();
    }
    assert_eq!(flat.matches("review_note(").count(), 1, "{flat}");
    assert!(
        flat.contains("None=>returnErr(review_failure(&e)),"),
        "{flat}"
    );
    assert!(!flat.contains("sign_off_failure"), "{flat}");
}
