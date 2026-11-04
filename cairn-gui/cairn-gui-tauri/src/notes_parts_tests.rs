//! Tests for `notes_parts.rs`, the note pane's views: each draft's parts under their writers'
//! claimed names, which part the current claim may edit, which parts a blank editor leaves holding
//! text, and the pane's words for its notes. Pure: no state, no database.
use super::*;
use crate::notes::tests::{bne, utc};
use crate::notes::time::stated_time_in;
use cairn_node::note::view::{NoteGrade, PartWriter};
use uuid::Uuid;

fn draft(terminal: &str) -> NoteDraft {
    NoteDraft {
        draft_id: Uuid::from_u128(5),
        terminal_id: terminal.into(),
        recording_started: utc("2026-11-02T17:12:00Z"),
        last_keystroke: utc("2026-11-02T17:40:00Z"),
        parts: vec![DraftPart {
            seq: 1,
            writer: None,
            writer_name: None,
            narrative: "BP <90".into(),
            stated_time: Some(utc("2026-11-02T17:00:00Z")),
            recording_started: utc("2026-11-02T17:12:00Z"),
            last_keystroke: utc("2026-11-02T17:40:00Z"),
        }],
    }
}

/// A draft by Nurse A, then Dr B, at terminal `pc-1`.
fn two_writers() -> NoteDraft {
    let mut d = draft("pc-1");
    d.parts[0].writer = Some("nurse-a".into());
    d.parts[0].writer_name = Some("Nurse A".into());
    let mut second = d.parts[0].clone();
    second.seq = 2;
    second.writer = Some("dr-b".into());
    second.writer_name = Some("Dr B".into());
    second.narrative = "plan: review".into();
    second.stated_time = None;
    d.parts.push(second);
    d
}

fn named(staff_id: &str) -> Claim {
    Claim::Named {
        staff_id: staff_id.into(),
        display_name: staff_id.into(),
    }
}

/// Every part is shown, each under its writer's claimed name; only the tail is editable, and only
/// by the writer it belongs to, at the terminal holding the draft.
#[test]
fn only_the_current_writers_tail_is_editable() {
    let d = two_writers();
    let editable = |claim: &Claim, terminal| -> Vec<bool> {
        draft_view(&d, Some(terminal), claim, &bne(), nov(3))
            .parts
            .iter()
            .map(|p| p.editable)
            .collect()
    };
    assert_eq!(editable(&named("dr-b"), "pc-1"), [false, true]);
    assert_eq!(
        editable(&named("nurse-a"), "pc-1"),
        [false, false],
        "A appends"
    );
    assert_eq!(editable(&named("dr-b"), "pc-2"), [false, false], "not here");
    assert_eq!(editable(&Claim::NotAsked, "pc-1"), [false, false]);
    assert_eq!(editable(&Claim::Unknown, "pc-1"), [false, false]);
    let view = draft_view(&d, Some("pc-1"), &named("dr-b"), &bne(), nov(3));
    let (a, b) = (&view.parts[0], &view.parts[1]);
    assert_eq!(
        (a.writer_shown.as_str(), a.writer_spoken.as_str()),
        ("?Nurse A", "unverified Nurse A")
    );
    assert_eq!(
        (a.text.as_str(), b.text.as_str()),
        ("BP <90", "plan: review")
    );
    assert_eq!(a.stated_time.as_deref(), Some("2026-11-03T03:00"));
    assert_eq!(b.stated_time, None);
    assert_eq!(a.words, "started 03:12 · last typed 03:40");
}

/// Ruling M3: two "Don't know" writers share one unknown tail (NULL = NULL at the floor), so the
/// tail stays editable for an unknown claim and carries its text — an empty editor would REPLACE
/// it on the next save.
#[test]
fn an_unknown_tail_stays_editable_for_dont_know() {
    let d = draft("pc-1");
    let view = draft_view(&d, Some("pc-1"), &Claim::Unknown, &bne(), nov(3));
    let tail = &view.parts[0];
    assert!(tail.editable);
    assert_eq!(tail.text, "BP <90");
    assert_eq!(tail.writer_shown, "writer not known");
    assert!(!draft_view(&d, Some("pc-1"), &named("dr-b"), &bne(), nov(3)).parts[0].editable);
}

fn nov(d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 11, d).unwrap()
}

#[test]
fn a_draft_says_when_it_was_started_and_last_typed_and_where() {
    let here = draft_view(
        &draft("pc-1"),
        Some("pc-1"),
        &Claim::NotAsked,
        &bne(),
        nov(3),
    );
    assert!(here.here);
    assert_eq!(
        here.words,
        "unsigned draft · started 03:12 · last typed 03:40"
    );
    assert_eq!(here.parts[0].text, "BP <90", "the raw text, to reopen");
    let there = draft_view(
        &draft("pc-2"),
        Some("pc-1"),
        &Claim::NotAsked,
        &bne(),
        nov(4),
    );
    assert!(!there.here);
    assert_eq!(
        there.words,
        "unsigned draft · started 3 Nov 03:12 · last typed 3 Nov 03:40 \
         · being written at another terminal"
    );
}

#[test]
fn the_pane_words_each_note_and_any_withheld_ones() {
    // Last year, so the words carry the date whatever today is.
    let t = utc("2025-11-02T17:00:00Z");
    let list = PatientNoteList {
        notes: vec![NoteRow {
            event_id: Uuid::from_u128(1),
            chart: Uuid::from_u128(7),
            text: "BP <90".into(),
            t_effective: t,
            effective_basis: "stated".into(),
            recording_started: t,
            grade: NoteGrade::Signed {
                by: "Dr Ada".into(),
            },
            addendum_to: None,
            writer: PartWriter::Drafted("Nurse A".into()),
        }],
        withheld_on_linked: 2,
        drafts: vec![draft("pc-1")],
    };
    let pane = note_pane(list, Some("pc-1"), &Claim::NotAsked, &bne(), nov(3));
    let row = &pane.notes[0];
    assert_eq!(row.text, "BP <90");
    assert_eq!(row.byline, "written by ?Nurse A · signed by Dr Ada");
    // I3 (#973): the `?` is heard as "unverified", as in a draft's parts.
    assert_eq!(
        row.byline_spoken,
        "written by unverified Nurse A · signed by Dr Ada"
    );
    assert_eq!(row.when, "3 Nov 2025 03:00 (stated)");
    assert!(pane.drafts[0].here);
    let withheld = pane.withheld_sentence.expect("withheld notes are said");
    assert!(withheld.starts_with("2 note(s)"), "{withheld}");
    let none = note_pane(
        PatientNoteList {
            notes: vec![],
            withheld_on_linked: 0,
            drafts: vec![],
        },
        Some("pc-1"),
        &Claim::NotAsked,
        &bne(),
        nov(3),
    );
    assert_eq!(none.withheld_sentence, None);
}

/// Which parts the on-screen text does NOT replace: all of them, unless the tail is the writer's
/// own (two "Don't know" writers share an unknown tail, ruling M3).
#[test]
fn the_other_parts_are_those_the_editor_does_not_replace() {
    let mut d = draft("pc-1");
    d.parts[0].writer = Some("nurse-a".into());
    assert!(
        others_have_text(&d.parts, Some("dr-b")),
        "B appends below A"
    );
    assert!(
        !others_have_text(&d.parts, Some("nurse-a")),
        "A rewrites her own"
    );
    d.parts[0].writer = None;
    assert!(!others_have_text(&d.parts, None));
    d.parts[0].narrative = "  ".into();
    assert!(
        !others_have_text(&d.parts, Some("dr-b")),
        "a blank part is no text"
    );
}

/// A reopened draft's stated time comes back as the field holds it, seconds included, so the next
/// save or Sign sends the same instant — never one silently rounded to the minute.
#[test]
fn a_stated_time_with_seconds_survives_reopening() {
    let mut d = draft("pc-1");
    d.parts[0].stated_time = Some(utc("2026-11-02T17:00:30Z"));
    let view = draft_view(&d, Some("pc-1"), &Claim::Unknown, &bne(), nov(3));
    let stated = view.parts[0].stated_time.as_deref();
    assert_eq!(stated, Some("2026-11-03T03:00:30"));
    let back = stated_time_in(stated.unwrap(), &bne());
    assert_eq!(back, Ok(utc("2026-11-02T17:00:30Z")));
}

/// Before setup has read this terminal's id, the list still shows every draft — none as this
/// terminal's, so none opens for editing here — rather than refusing to show the chart's notes.
#[test]
fn without_a_terminal_id_no_draft_is_here() {
    let list = PatientNoteList {
        notes: vec![],
        withheld_on_linked: 0,
        drafts: vec![draft("pc-1")],
    };
    let pane = note_pane(list, None, &Claim::Unknown, &bne(), nov(3));
    assert_eq!(pane.drafts.len(), 1);
    assert!(!pane.drafts[0].here);
    assert!(!pane.drafts[0].parts[0].editable);
}

/// Hardening (final review item 7): the pane names the claim it chose the editable part for, so
/// the window's editor captures THAT writer — the editable flag and the writer cannot diverge.
#[test]
fn the_pane_names_the_claim_it_chose_the_editable_part_for() {
    let list = || PatientNoteList {
        notes: vec![],
        withheld_on_linked: 0,
        drafts: vec![two_writers()],
    };
    let dr_b = Claim::Named {
        staff_id: "dr-b".into(),
        display_name: "Dr B".into(),
    };
    let pane = note_pane(list(), Some("pc-1"), &dr_b, &bne(), nov(3));
    assert_eq!(pane.claim.kind, "named");
    assert_eq!(pane.claim.staff_id.as_deref(), Some("dr-b"));
    assert!(pane.drafts[0].parts[1].editable, "Dr B's own tail");
    let nobody = note_pane(list(), Some("pc-1"), &Claim::NotAsked, &bne(), nov(3));
    assert_eq!(nobody.claim.kind, "not-asked");
    assert_eq!(nobody.claim.staff_id, None);
}
