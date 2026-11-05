//! Tests for `notes_compound.rs`: a note and its addenda as the pane shows them — ONE text, one
//! heading, a parts disclosure only when there is more than one part — and an addendum draft's
//! "Adding to…" line (sign-in slice 3c-1). Pure: no state, no database.
use super::*;
use crate::notes::parse_target;
use crate::notes::parts::{draft_view, note_pane};
use crate::notes::tests::{bne, utc};
use crate::whos_here::Claim;
use cairn_node::note::compound::compounds;
use cairn_node::note::draft::{DraftPart, NoteDraft};
use cairn_node::note::read::{NoteRow, PatientNoteList};
use cairn_node::note::view::{NoteGrade, PartWriter};

/// The chart on screen.
const OPENED: u128 = 7;
/// A chart linked to it (ADR-0076): its notes are read, but a draft cannot live there.
const LINKED: u128 = 8;

fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}

fn nov(d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 11, d).unwrap()
}

/// Note `n` on chart `chart`, adding to `to`, begun (and stated for) `min` minutes after
/// 2025-11-02 17:00Z — last year, so its words carry the date whatever today is.
fn note(n: u128, chart: u128, to: Option<u128>, min: i64, grade: NoteGrade) -> NoteRow {
    let t = utc("2025-11-02T17:00:00Z") + chrono::Duration::minutes(min);
    NoteRow {
        event_id: id(n),
        chart: id(chart),
        text: format!("part {n}"),
        t_effective: t,
        effective_basis: "stated".into(),
        recording_started: t,
        grade,
        addendum_to: to.map(id),
        writer: PartWriter::TheSigner,
    }
}

fn signed(by: &str) -> NoteGrade {
    NoteGrade::Signed { by: by.into() }
}

/// The one compound `notes` group into.
fn one(notes: Vec<NoteRow>) -> CompoundNote {
    let mut all = compounds(notes);
    assert_eq!(all.len(), 1, "{all:?}");
    all.remove(0)
}

fn view_of(c: &CompoundNote) -> CompoundView {
    compound_view(c, id(OPENED), &bne(), nov(3))
}

/// A one-part note reads exactly as a note did before 3c-1: its own text and heading, no
/// disclosure, no state (its byline already says how it went in).
#[test]
fn a_one_part_note_has_no_parts_label_or_state() {
    let v = view_of(&one(vec![note(1, OPENED, None, 0, signed("Dr Ada"))]));
    assert_eq!(v.root_event_id, id(1).to_string());
    assert_eq!(v.text, "part 1");
    assert_eq!(v.byline, "signed by Dr Ada");
    assert_eq!(v.byline_spoken, "signed by Dr Ada");
    assert_eq!(v.when, "3 Nov 2025 03:00 (stated)");
    assert_eq!((v.state, v.parts_label), (None, None));
    assert!(v.parts.is_empty(), "no disclosure for one part");
    assert!(v.can_add, "a note on the chart on screen can be added to");
    // What "Add to this note" opens the editor saying, before any draft exists to read it from.
    assert_eq!(
        v.adds_line,
        "Adding to the note of 3 Nov 2025 03:00 (stated)"
    );
}

/// Three parts (a later writer's, then an addendum by another clinician) read as one text in
/// part order, headed by the root; the disclosure holds each part, and the state says one part
/// went in unsigned.
#[test]
fn a_three_part_note_reads_as_one_text() {
    let v = view_of(&one(vec![
        note(3, OPENED, Some(1), 30, NoteGrade::WentInUnsigned),
        note(1, OPENED, None, 0, signed("Dr Ada")),
        note(2, OPENED, Some(1), 10, signed("Dr Bo")),
    ]));
    assert_eq!(v.text, "part 1\n\npart 2\n\npart 3");
    assert_eq!(v.byline, "signed by Dr Ada", "the root's heading");
    assert_eq!(v.when, "3 Nov 2025 03:00 (stated)");
    assert_eq!(v.parts_label.as_deref(), Some("3 parts"));
    assert_eq!(v.state.as_deref(), Some("includes an unsigned part"));
    let parts: Vec<&str> = v.parts.iter().map(|p| p.text.as_str()).collect();
    assert_eq!(parts, ["part 1", "part 2", "part 3"]);
    assert_eq!(v.parts[2].event_id, id(3).to_string());
    // Every part signed is said too, once there is more than one part.
    let both = view_of(&one(vec![
        note(1, OPENED, None, 0, signed("Dr Ada")),
        note(2, OPENED, Some(1), 10, signed("Dr Bo")),
    ]));
    assert_eq!(both.state.as_deref(), Some("every part signed"));
    assert_eq!(both.parts_label.as_deref(), Some("2 parts"));
}

/// A note on a linked chart is shown, but offers no "Add to this note": the draft would live on a
/// chart this pane does not list.
#[test]
fn a_linked_charts_note_cannot_be_added_to() {
    let v = view_of(&one(vec![note(1, LINKED, None, 0, signed("Dr Ada"))]));
    assert!(!v.can_add);
    // Decided by the ROOT's chart: an addendum on the chart on screen to a linked chart's note
    // does not make the note one this pane can add to.
    let mixed = view_of(&one(vec![
        note(1, LINKED, None, 0, signed("Dr Ada")),
        note(2, OPENED, Some(1), 10, signed("Dr Bo")),
    ]));
    assert!(!mixed.can_add);
}

/// An addendum whose note this read does not hold (withheld on a doubted link) heads its own
/// compound, and says so — never reading as a note of its own.
#[test]
fn an_orphan_says_it_adds_to_a_note_not_shown() {
    let v = view_of(&one(vec![note(2, OPENED, Some(1), 10, signed("Dr Bo"))]));
    assert_eq!(v.byline, "Adds to a note not shown here · signed by Dr Bo");
    assert_eq!(
        v.byline_spoken,
        "Adds to a note not shown here · signed by Dr Bo"
    );
    assert_eq!(v.text, "part 2");
}

/// A draft with the target on screen.
fn draft(n: u128, addendum_to: Option<Uuid>) -> NoteDraft {
    let t = utc("2026-11-02T17:12:00Z");
    NoteDraft {
        draft_id: id(n),
        terminal_id: "pc-1".into(),
        recording_started: t,
        last_keystroke: t,
        parts: vec![DraftPart {
            seq: 1,
            writer: None,
            writer_name: None,
            narrative: "obs stable".into(),
            stated_time: None,
            recording_started: t,
            last_keystroke: t,
        }],
        due: utc("2026-11-02T20:12:00Z"),
        overdue: false,
        stuck: false,
        addendum_to,
    }
}

/// An addendum draft has its own line saying what it adds to, apart from its `words`: the note's
/// stated time, or when its writing began when no time was stated (never "no stated time · writing
/// began …", which would run into the draft's own "started …" and leave two unstated notes alike);
/// "a note not shown here" when the pane does not show it. A new note's draft has no such line.
#[test]
fn an_addendum_draft_says_what_it_adds_to() {
    let mut unstated = note(5, OPENED, None, -60, signed("Dr Cy"));
    unstated.effective_basis = "recording-started".into();
    let list = PatientNoteList {
        notes: vec![
            note(1, OPENED, None, 0, signed("Dr Ada")),
            note(2, OPENED, Some(1), 10, signed("Dr Bo")),
            unstated,
        ],
        withheld_on_linked: 0,
        drafts: vec![
            draft(50, Some(id(1))),
            draft(51, Some(id(404))),
            draft(52, None),
            // A target that is a later part, not the root: the raw-SQL case, and the one where
            // a link judgement shows the note an orphan added to. The note IS shown.
            draft(53, Some(id(2))),
            draft(54, Some(id(5))),
        ],
        discards: vec![],
        discards_withheld_on_linked: 0,
    };
    let pane = note_pane(
        list,
        id(OPENED),
        Some("pc-1"),
        &Claim::NotAsked,
        &Default::default(),
        &bne(),
        nov(3),
    );
    let lines: Vec<Option<&str>> = pane.drafts.iter().map(|d| d.adding_to.as_deref()).collect();
    assert_eq!(
        lines,
        [
            Some("Adding to the note of 3 Nov 2025 03:00 (stated)"),
            Some("Adding to a note not shown here"),
            None,
            Some("Adding to the note of 3 Nov 2025 03:00 (stated)"),
            Some("Adding to the note begun 3 Nov 2025 02:00"),
        ]
    );
    // The words are a draft's words, as before 3c-1: the line is said once, on its own.
    let new_note = draft_view(
        &draft(52, None),
        Some("pc-1"),
        &Claim::NotAsked,
        false,
        &bne(),
        nov(3),
        &Default::default(),
    );
    assert!(new_note.words.starts_with("unsigned draft · "));
    for d in &pane.drafts {
        assert_eq!(d.words, new_note.words);
        assert!(!d.words.contains("Adding to"), "{}", d.words);
    }
    // The window sends back exactly the draft's own target with every save, Sign and Discard:
    // a different one would be refused as `elsewhere` (worded "belongs to another chart").
    let targets: Vec<Option<String>> = pane.drafts.iter().map(|d| d.addendum_to.clone()).collect();
    assert_eq!(
        targets,
        [
            Some(id(1).to_string()),
            Some(id(404).to_string()),
            None,
            Some(id(2).to_string()),
            Some(id(5).to_string()),
        ]
    );
    assert_eq!(pane.notes.len(), 2, "a note and its addendum are one");
    // The editor "Add to this note" opens says the note's own line, the very words its draft
    // reads back with once the pane is read again (stated and unstated alike).
    for (note, d) in [
        (&pane.notes[0], &pane.drafts[0]),
        (&pane.notes[1], &pane.drafts[4]),
    ] {
        assert_eq!(d.adding_to.as_deref(), Some(note.adds_line.as_str()));
    }
}

/// The target the webview sends: absent is a new note; a malformed id is refused, never read as
/// a new note (which would sign the addendum as a note of its own).
#[test]
fn a_target_is_read_or_refused() {
    assert_eq!(parse_target(None).unwrap(), None);
    assert_eq!(parse_target(Some(&id(1).to_string())).unwrap(), Some(id(1)));
    let bad = parse_target(Some("not-a-uuid")).unwrap_err();
    assert_eq!(bad.text, "not a note id");
}

/// The target the webview sent reaches the node on every write — never a hard-coded `None`,
/// which would put a Sign or Discard pressed inside the debounce in as a new note (Review Focus
/// 2). Read from the source, because a fixture window refuses before the door is reached.
#[test]
fn every_write_hands_the_node_the_target() {
    let flat = |src: &str, f: &str| -> String {
        crate::source_guard::rust_fn(src, f)
            .split_whitespace()
            .collect()
    };
    let notes = include_str!("notes.rs");
    let save = flat(notes, "note_draft_save_impl");
    assert!(save.contains("narrative,stated,target,"), "{save}");
    let sign = flat(notes, "note_sign_impl");
    assert!(sign.contains("addendum_to:target,"), "{sign}");
    let discard = flat(include_str!("notes_discard.rs"), "discard_attempt");
    assert!(discard.contains("addendum_to:target,"), "{discard}");
    for body in [save, sign, discard] {
        assert!(body.contains("parse_target(addendum_to)?"), "{body}");
    }
}
