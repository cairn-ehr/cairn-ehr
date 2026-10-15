//! R7 (#743 part 2): the note on a link this node cannot verify. db/054's
//! `cairn_chart_set_unverified_links` lists an un-attested standing link whose charts hold
//! different identifiers in one system where at least one side carries no normalized form, so
//! this node cannot tell a real mismatch from formatting (db/016's `degrade_hold`). It is never a doubt: nothing is held, so the note says only what is known,
//! and the line keeps its one door.

use super::*;
use cairn_node::patient::edges::UnverifiedLink;

fn edge(attested: bool) -> RecordEdge {
    RecordEdge {
        low: Uuid::from_u128(1),
        high: Uuid::from_u128(2),
        attested,
        recorded_on: "2026-10-15".into(),
    }
}

/// An unverified row on the pair `edge` builds (charts 1 and 2), for one identifier system.
fn unverified(system: &str) -> UnverifiedLink {
    UnverifiedLink {
        low: Uuid::from_u128(1),
        high: Uuid::from_u128(2),
        system: system.into(),
    }
}

fn lines(links: RecordLinks) -> Vec<RecordLinkView> {
    let (lines, error) = links_section(Ok(links), &[], true);
    assert!(error.is_none(), "{error:?}");
    lines
}

#[test]
fn an_unverified_link_is_noted_with_its_system_and_keeps_its_door() {
    let l = lines(RecordLinks {
        edges: vec![edge(false)],
        unverified: Ok(vec![unverified("local-mrn")]),
        ..RecordLinks::default()
    });
    assert_eq!(l.len(), 1);
    assert!(
        l[0].text.ends_with(
            " — unverified here: the two charts hold different local-mrn identifiers, and this \
             node cannot tell a real mismatch from a difference in formatting"
        ),
        "{}",
        l[0].text
    );
    assert_eq!(l[0].button, REVIEW_LINK);
    assert!(
        !l[0].text.contains("in doubt") && !l[0].text.contains("signed off"),
        "a soft veto holds nothing, so the note must not say it does: {}",
        l[0].text
    );
}

/// Two systems on one pair are named once each, in one note.
#[test]
fn two_unverified_systems_share_one_note() {
    let l = lines(RecordLinks {
        edges: vec![edge(false)],
        unverified: Ok(vec![unverified("local-mrn"), unverified("ward-id")]),
        ..RecordLinks::default()
    });
    assert_eq!(l[0].text.matches("unverified here").count(), 1);
    assert!(
        l[0].text
            .contains("hold different local-mrn and ward-id identifiers"),
        "{}",
        l[0].text
    );
}

/// A clash line already says "in doubt" and why; a second note on it would be noise.
#[test]
fn a_clash_line_carries_only_its_clash_note() {
    let l = lines(RecordLinks {
        edges: vec![edge(false)],
        doubts: vec![LinkDoubt {
            low: Uuid::from_u128(1),
            high: Uuid::from_u128(2),
            kind: DoubtKind::Clash,
            recorded_on: "2026-10-15".into(),
        }],
        unverified: Ok(vec![unverified("local-mrn")]),
    });
    assert!(l[0].text.contains("in doubt"));
    assert!(!l[0].text.contains("unverified here"), "{}", l[0].text);
}

/// A row naming a pair that is not a standing link cannot come from the one snapshot both reads
/// share. No hold depends on it, so it is dropped and the list still draws: refusing the whole
/// list over an informational note would hide every door.
#[test]
fn a_note_naming_no_standing_link_is_dropped_and_the_list_still_draws() {
    let l = lines(RecordLinks {
        edges: vec![edge(false)],
        unverified: Ok(vec![UnverifiedLink {
            low: Uuid::from_u128(3),
            high: Uuid::from_u128(4),
            system: "local-mrn".into(),
        }]),
        ..RecordLinks::default()
    });
    assert_eq!(l.len(), 1);
    assert!(!l[0].text.contains("unverified here"));
}

/// A link nothing notes reads exactly as before R7.
#[test]
fn an_unnoted_link_line_is_byte_identical_to_before() {
    let l = lines(RecordLinks {
        edges: vec![edge(false)],
        ..RecordLinks::default()
    });
    assert_eq!(l[0], record_link_line(&edge(false), &[]));
}

/// The note's read failing must not cost the list (PR #788 review). The notes hold nothing back,
/// while the lines carry every repair door: refusing the list over them would leave a wrong link
/// un-undoable until an unrelated read recovers. So the lines draw — with no note, because an
/// unread note is unknown, not absent — and the section SAYS the verification could not be read,
/// so a line without a note is never taken for "verified".
#[test]
fn a_failed_note_read_still_draws_every_line_and_says_so() {
    let (l, error) = links_section(
        Ok(RecordLinks {
            edges: vec![edge(false)],
            unverified: Err("statement timeout".into()),
            ..RecordLinks::default()
        }),
        &[],
        true,
    );
    assert_eq!(l.len(), 1, "the line and its door survive");
    assert_eq!(l[0].button, REVIEW_LINK);
    assert!(!l[0].text.contains("unverified here"));
    let error = error.expect("an unread note is said, never shown as none");
    assert!(error.contains("could not be read"), "{error}");
    assert!(error.contains("statement timeout"), "{error}");
}
