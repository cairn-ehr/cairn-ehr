//! Pure parts of `release_intake`: the status wording and the stored-chain rebuild.
use super::*;

#[test]
fn status_names_channel_root_release_and_incident() {
    let s = render_status(&StatusFacts {
        root_damaged: false,
        channel: Some("cairn-stable".into()),
        root_version: Some(2),
        newest_release: Some(7),
        open_incidents: vec!["Two different trust files both claim version 2.".into()],
    });
    assert!(s.contains("cairn-stable") && s.contains("version 2") && s.contains("release held: 7"));
    assert!(s.contains("Updates are stopped: Two different"));
}

#[test]
fn status_of_an_empty_node_says_so() {
    let s = render_status(&StatusFacts::default());
    assert!(s.contains("No release channel") && s.contains("No release held"));
}

#[test]
fn an_empty_stored_chain_is_no_channel() {
    assert_eq!(rebuild_history(&[]).err(), Some(Refusal::NoChannel));
}

#[test]
fn a_damaged_root_is_not_reported_as_no_channel() {
    let s = render_status(&StatusFacts {
        root_damaged: true,
        ..StatusFacts::default()
    });
    assert!(
        s.contains("damaged") && !s.contains("No release channel"),
        "{s}"
    );
}
