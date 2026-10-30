use super::*;

/// A malformed date of birth is a verdict about the INPUT: nobody can make that call
/// succeed, ever, without changing what was typed.
#[test]
fn a_malformed_input_refuses_about_the_input() {
    let e = crate::patient::register::dob_precision("3/2/1980")
        .expect_err("a malformed birth date refuses");
    assert!(carries_refusal_marker(&e));
    assert_eq!(
        refusal_scope(&e),
        Some(RefusalScope::Input),
        "dob_precision is the case the marker was designed for: pure, and pointless to \
         retry forever"
    );
}

/// An unprovisioned node is a verdict about the NODE, and the difference is the retry.
///
/// Retrying the identical call changes nothing — so it is a refusal, not an outage. But an
/// operator running the named command makes that same call succeed, so a surface that
/// renders this the way it renders a malformed date (no way forward but editing the form)
/// strands a clerk whose form was correct all along.
#[test]
fn an_unprovisioned_node_refuses_about_the_node_state() {
    let e = crate::actor_enrolment::not_enrolled_refusal("9f3c");
    assert!(carries_refusal_marker(&e));
    assert_eq!(
        refusal_scope(&e),
        Some(RefusalScope::NodeState),
        "an enrolment refusal is decided by how the node is provisioned, not by the form"
    );
}

/// The scope survives the `.context(…)` layers every real call site adds.
///
/// Without the whole-chain walk this returns `None` for every refusal a real orchestrator
/// produces, which is the bug `carries_refusal_marker`'s doc already records for its own
/// half of the question.
#[test]
fn the_scope_survives_the_context_layers_an_orchestrator_adds() {
    let e = crate::actor_enrolment::retired_actor_refusal("9f3c")
        .context("registering the patient")
        .context("handling the clerk's click");
    assert_eq!(refusal_scope(&e), Some(RefusalScope::NodeState));
}

/// An ordinary error is neither — the marker is opt-in, so `None` is the honest answer.
#[test]
fn an_unmarked_failure_has_no_scope() {
    let e = anyhow::anyhow!("the connection dropped");
    assert!(!carries_refusal_marker(&e));
    assert_eq!(refusal_scope(&e), None);
}

/// Both scopes are refusals. Nothing may read "not `Input`" as "not a verdict".
///
/// Pins the property that makes the split safe to add: widening the enum must never turn a
/// verdict back into an outage, so the marker question and the scope question stay
/// separate.
/// #713: an interrupted commit is found through any context layered above it, names its
/// event, and is NEVER mistaken for a verdict — it is the one failure that may have written.
#[test]
fn an_unknown_commit_outcome_is_marked_and_is_not_a_refusal() {
    let id = uuid::Uuid::from_u128(77);
    let e = commit_outcome_unknown(id, anyhow::anyhow!("connection reset"))
        .context("recording the unlink");
    assert_eq!(commit_outcome_unknown_event(&e), Some(id));
    assert!(
        !carries_refusal_marker(&e),
        "an unknown outcome is not a verdict"
    );
    assert_eq!(refusal_scope(&e), None);
    let text = format!("{e:#}");
    assert!(text.contains(&id.to_string()), "names the event: {text}");
    assert!(text.contains("connection reset"), "keeps the cause: {text}");
    // And an ordinary failure, or a verdict, does not carry it.
    assert_eq!(
        commit_outcome_unknown_event(&anyhow::anyhow!("timeout")),
        None
    );
    assert_eq!(
        commit_outcome_unknown_event(&deliberate_refusal("no")),
        None
    );
}

/// #814: a commit failure with no server answer anywhere on its chain (no `DbError`) is
/// marked outcome-unknown, naming the event, and keeps the failure as its cause. The
/// answered half needs a real `DbError`, so it is pinned in `tests/db_diagnosis.rs`.
#[test]
fn an_unanswered_commit_failure_is_marked_with_its_event() {
    let id = uuid::Uuid::from_u128(78);
    let e = commit_failure(
        id,
        anyhow::anyhow!("connection closed").context("committing a line's sign-off"),
    );
    assert_eq!(commit_outcome_unknown_event(&e), Some(id));
    assert!(format!("{e:#}").contains("connection closed"));
}

/// #823 review: a caller that says the outcome itself (auto-apply's "a re-run is safe") needs
/// the failure WITHOUT the marker's "it may be recorded twice", which would contradict it.
#[test]
fn the_unknown_outcomes_cause_is_rendered_without_the_markers_sentence() {
    let e = commit_failure(
        uuid::Uuid::from_u128(80),
        anyhow::anyhow!("connection closed").context("committing the auto-applied link"),
    )
    .context("auto-applying");
    let cause = commit_outcome_unknown_cause(&e).expect("marked");
    assert_eq!(cause, "committing the auto-applied link: connection closed");
    assert_eq!(commit_outcome_unknown_cause(&anyhow::anyhow!("x")), None);
}

/// A refusal over a changed record is marked `Stale`, and its text is untouched.
#[test]
fn a_stale_picture_refusal_is_marked_stale() {
    let e = stale_picture_refusal("x");
    assert_eq!(refusal_scope(&e), Some(RefusalScope::Stale));
    assert!(carries_refusal_marker(&e));
    assert_eq!(format!("{e:#}"), "x");
}

/// The CLI prints a node sentence as it is, and a stale one names no remedy (the window's
/// remedy is its own re-read, #797) — so the CLI adds its own line for `Stale`, wherever on
/// the chain the marker sits. And, since #822, for a `CommitOutcomeUnknown`: its marker asks
/// for a check the CLI now names. Every other scope's sentence already says what to do.
#[test]
fn only_a_stale_refusal_or_an_unknown_outcome_gets_the_cli_remedy() {
    let stale = stale_picture_refusal("x").context("signing");
    assert_eq!(cli_remedy(&stale).as_deref(), Some(CLI_STALE_REMEDY));
    assert_eq!(cli_remedy(&anyhow::anyhow!("timeout")), None);
    // #822: the marker tells the operator to check whether the event was recorded; the
    // remedy names the command that checks, with that event's id, through any context.
    let id = uuid::Uuid::from_u128(79);
    let unknown = commit_failure(id, anyhow::anyhow!("connection closed")).context("shred");
    let remedy = cli_remedy(&unknown).expect("an unknown outcome gets a remedy line");
    assert!(
        remedy.contains(&format!("cairn-node --conn <the same> event {id}")),
        "names the check, with the event, `--conn` before the subcommand: {remedy}"
    );
    for e in [
        crate::actor_enrolment::ambiguous_actor_refusal("9f3c"),
        crate::patient::register::dob_precision("not a date").unwrap_err(),
        anyhow::anyhow!("connection reset"),
    ] {
        assert_eq!(cli_remedy(&e), None, "{e:#}");
    }
}

#[test]
fn every_scope_is_still_a_refusal() {
    for e in [
        crate::actor_enrolment::ambiguous_actor_refusal("9f3c"),
        crate::patient::register::dob_precision("not a date").unwrap_err(),
        stale_picture_refusal("x"),
    ] {
        assert!(
            carries_refusal_marker(&e),
            "a scoped refusal must still answer the marker question: {e:#}"
        );
        assert!(refusal_scope(&e).is_some());
    }
}
