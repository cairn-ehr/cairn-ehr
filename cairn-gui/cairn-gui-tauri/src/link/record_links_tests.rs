//! The links list's unit tests (R2b-2, R6), moved out of `record_links.rs` in R7 to keep that
//! file under 500 lines. R7's own tests are in `record_links_unverified_tests.rs`.

use super::*;

fn edge(attested: bool) -> RecordEdge {
    RecordEdge {
        low: Uuid::from_u128(1),
        high: Uuid::from_u128(2),
        attested,
        recorded_on: "2026-09-28".into(),
    }
}
/// A db/054 doubt on the same pair `edge` builds (charts 1 and 2).
fn doubt(kind: DoubtKind) -> LinkDoubt {
    LinkDoubt {
        low: Uuid::from_u128(1),
        high: Uuid::from_u128(2),
        kind,
        recorded_on: "2026-10-01".into(),
    }
}
fn member(n: u128, name: &str) -> MemberLine {
    MemberLine {
        patient_id: Uuid::from_u128(n).to_string(),
        name: name.into(),
        born: String::new(),
        text: String::new(),
    }
}

#[test]
fn a_link_line_names_both_charts_how_it_was_made_and_when() {
    let members = [member(1, "SMITH John"), member(2, "SMYTHE John")];
    let human = record_link_line(&edge(true), &members);
    assert!(human.text.contains("SMITH John") && human.text.contains("SMYTHE John"));
    assert!(human.text.contains("by a clinician's judgement"));
    assert!(human.text.contains("2026-09-28 (UTC)"));
    assert_eq!(human.low, Uuid::from_u128(1).to_string());
    let machine = record_link_line(&edge(false), &members);
    assert!(machine
        .text
        .contains("without a clinician's confirmation on record here"));
    assert!(
        !machine.text.contains("matcher"),
        "un-attested is not proof of the matcher"
    );
}

#[test]
fn a_chart_missing_from_the_members_is_named_by_its_id_alone() {
    let line = record_link_line(&edge(true), &[]);
    assert!(line.text.contains(&Uuid::from_u128(1).to_string()));
}

/// Review Focus 5.
#[test]
fn an_unread_link_list_says_so_and_offers_no_unlink() {
    let (links, error) = links_section(Err("connection reset".into()), &[], true);
    assert!(links.is_empty());
    let error = error.expect("an unread list is said, never shown empty");
    assert!(error.contains("could not be read"));
    assert!(error.contains("Read this chart again"));
}

#[test]
fn a_read_list_carries_no_error() {
    let (links, error) = links_section(
        Ok(RecordLinks {
            unverified: Ok(vec![]),
            edges: vec![edge(true)],
            doubts: vec![],
        }),
        &[],
        true,
    );
    assert_eq!(links.len(), 1);
    assert!(error.is_none());
}

#[test]
fn a_linked_record_with_no_edges_read_says_so() {
    let (links, error) = links_section(Ok(RecordLinks::default()), &[], true);
    assert!(links.is_empty());
    assert!(error
        .expect("never blank")
        .contains("press \"Read this chart again\""));
}

#[test]
fn a_single_chart_with_no_edges_is_silent() {
    let (links, error) = links_section(Ok(RecordLinks::default()), &[], false);
    assert!(links.is_empty() && error.is_none());
}

#[test]
fn a_clash_line_says_it_is_in_doubt_and_why() {
    let links = RecordLinks {
        unverified: Ok(vec![]),
        edges: vec![edge(false)],
        doubts: vec![doubt(DoubtKind::Clash)],
    };
    let (lines, err) = links_section(Ok(links), &[], true);
    assert!(err.is_none());
    assert_eq!(lines.len(), 1);
    assert!(lines[0]
        .text
        .contains("in doubt: this node's identity check finds a clash"));
    assert_eq!(lines[0].button, REVIEW_LINK);
    assert_eq!(lines[0].kind, PairKind::Link);
}

/// With two doubted links, judging this one leaves the hold: the note may say only what is
/// always true — WHILE this link is in doubt, only a line recorded on this chart alone can be
/// signed off (hazard.rs withholds a line shared with another chart too, so "lines from other
/// charts" would undersell it) — never that a judgement of this link lifts the hold ("until").
#[test]
fn a_clash_note_never_promises_that_judging_this_link_lifts_the_hold() {
    assert!(!CLASH_NOTE.contains("until"), "{CLASH_NOTE}");
    assert_eq!(
        CLASH_NOTE,
        " — in doubt: this node's identity check finds a clash between these two charts; \
         while this link is in doubt, only lines recorded on this chart alone can be signed \
         off"
    );
}

#[test]
fn an_undoubted_link_line_is_byte_identical_to_before() {
    let links = RecordLinks {
        unverified: Ok(vec![]),
        edges: vec![edge(true)],
        doubts: vec![],
    };
    let (lines, _) = links_section(Ok(links), &[], true);
    assert_eq!(lines[0].text, record_link_line(&edge(true), &[]).text);
    assert_eq!(lines[0].button, REVIEW_LINK);
}

#[test]
fn a_separated_pair_gets_its_own_line_after_the_links() {
    let sep = LinkDoubt {
        low: Uuid::from_u128(1),
        high: Uuid::from_u128(3),
        kind: DoubtKind::Separated,
        recorded_on: "2026-10-01".into(),
    };
    let links = RecordLinks {
        unverified: Ok(vec![]),
        edges: vec![edge(false)],
        doubts: vec![sep],
    };
    let (lines, _) = links_section(Ok(links), &[], true);
    assert_eq!(lines.len(), 2);
    let l = &lines[1];
    assert_eq!(l.kind, PairKind::Separated);
    assert_eq!(l.button, REVIEW_JUDGEMENT);
    assert!(l.text.contains(
        "a clinician recorded on 2026-10-01 (UTC) that these are different people, but \
         other links still join them"
    ));
}

/// A clash doubt naming a pair that is NOT a standing edge cannot come from one snapshot
/// (db/054's clash arm is a property of a standing link), so the two reads disagree. Dropping
/// the doubt would leave the R1b hold pointing at a line nobody can find; the list is said
/// unread instead, never drawn without the mark (PR #777 review).
#[test]
fn a_clash_on_a_pair_with_no_line_is_said_never_dropped() {
    let links = RecordLinks {
        unverified: Ok(vec![]),
        edges: vec![edge(true)],
        doubts: vec![LinkDoubt {
            high: Uuid::from_u128(3),
            ..doubt(DoubtKind::Clash)
        }],
    };
    let (lines, error) = links_section(Ok(links), &[], true);
    assert!(lines.is_empty(), "no line drawn without its mark");
    assert!(error
        .expect("never silent")
        .contains("press \"Read this chart again\""));
}

/// Each line carries the attestation it SHOWS, so the review panel can tell a line drawn
/// before a colleague's confirm from the pair as it now stands (`unlink::line_still_shows`).
/// A separation is always a clinician's judgement (db/054 reports only attested ones).
#[test]
fn a_line_carries_the_attestation_it_shows() {
    assert!(record_link_line(&edge(true), &[]).attested);
    assert!(!record_link_line(&edge(false), &[]).attested);
    assert!(separated_line(&doubt(DoubtKind::Separated), &[]).attested);
}

/// db/018 makes it unreachable, but a pair reported as BOTH kinds must still render: the
/// link line carries the note, the separation gets its own line, nothing panics.
#[test]
fn a_pair_reported_as_both_kinds_renders_both_lines() {
    let links = RecordLinks {
        unverified: Ok(vec![]),
        edges: vec![edge(false)],
        doubts: vec![doubt(DoubtKind::Clash), doubt(DoubtKind::Separated)],
    };
    let (lines, _) = links_section(Ok(links), &[], true);
    assert_eq!(lines.len(), 2);
    assert_eq!(
        (lines[0].kind, lines[1].kind),
        (PairKind::Link, PairKind::Separated)
    );
    assert!(lines[0].text.contains("in doubt"));
}

/// The webview sends the line's `kind` back as text; only the two serialized spellings parse.
#[test]
fn a_pair_kind_parses_only_its_own_spellings() {
    for k in [PairKind::Link, PairKind::Separated] {
        let sent = serde_json::to_value(k).unwrap();
        assert_eq!(PairKind::parse(sent.as_str().unwrap()), Some(k));
    }
    assert_eq!(PairKind::parse("Link"), None);
    assert_eq!(PairKind::parse(""), None);
}

/// A separation alone is not a standing link: a linked record whose edge read came back
/// empty is still contradictory, whatever doubts were read beside it.
#[test]
fn a_linked_record_with_only_doubts_still_says_reload() {
    let links = RecordLinks {
        unverified: Ok(vec![]),
        edges: vec![],
        doubts: vec![doubt(DoubtKind::Separated)],
    };
    let (lines, error) = links_section(Ok(links), &[], true);
    assert!(lines.is_empty());
    assert!(error
        .expect("never blank")
        .contains("press \"Read this chart again\""));
}
