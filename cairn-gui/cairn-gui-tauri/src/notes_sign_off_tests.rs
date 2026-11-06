//! Tests for `notes_sign_off.rs` and the views it reads (sign-in slice 3c-2): whether a note
//! offers "Sign off this note" and which parts it shows, the strip's "also covers" line for an
//! addendum, the sign-off's refusals and Sign's success line. Pure, but for the refusals against
//! `AppState::mock`; the DB-gated test is `notes_sign_off_db_tests.rs`.
use super::*;
use crate::funnel::view::Retry;
use crate::notes::compound::{compound_view, CompoundView};
use crate::notes::parts::note_pane;
use crate::notes::tests::{bne, utc};
use crate::notes::words::signed_words;
use crate::whos_here::Claim;
use cairn_event::note_attestation::{VouchAct, VouchRole};
use cairn_node::note::compound::{compounds, CompoundNote};
use cairn_node::note::draft::{DraftPart, NoteDraft};
use cairn_node::note::read::{NoteRow, PatientNoteList};
use cairn_node::note::view::{NoteGrade, PartWriter};
use cairn_node::note::vouch_read::{PartVouch, Signer};
use chrono::NaiveDate;

/// The chart on screen, and one linked to it.
const OPENED: u128 = 7;
const LINKED: u128 = 8;

fn id(n: u128) -> Uuid {
    Uuid::from_u128(n)
}

fn who(actor: &str, name: &str) -> Signer {
    Signer {
        actor: actor.into(),
        name: name.into(),
        student: false,
        unconfirmed: false,
        repudiated: false,
    }
}

/// Part `n` on `chart`, adding to `to`, stated `min` minutes after 2025-11-02 17:00Z (so its
/// words carry the date), signed by `signer` or — `None` — gone in unsigned, drafted by
/// `drafted` (an actor id) when given.
fn part(
    n: u128,
    chart: u128,
    to: Option<u128>,
    min: i64,
    signer: Option<Signer>,
    drafted: Option<&str>,
) -> NoteRow {
    let t = utc("2025-11-02T17:00:00Z") + chrono::Duration::minutes(min);
    NoteRow {
        event_id: id(n),
        chart: id(chart),
        text: format!("part {n}"),
        t_effective: t,
        effective_basis: "stated".into(),
        recording_started: t,
        grade: match &signer {
            Some(s) => NoteGrade::Signed { by: s.name.clone() },
            None => NoteGrade::WentInUnsigned,
        },
        addendum_to: to.map(id),
        writer: match drafted {
            Some(_) => PartWriter::Drafted("Nurse A".into()),
            None => PartWriter::TheSigner,
        },
        // The envelope's signer is the grade's, as the window's own Sign makes it.
        signer_human_actor: signer.as_ref().map(|s| s.actor.clone()),
        signed_by: signer,
        drafted_actor: drafted.map(str::to_string),
        vouches: vec![],
    }
}

fn one(notes: Vec<NoteRow>) -> CompoundNote {
    let mut all = compounds(notes);
    assert_eq!(all.len(), 1, "{all:?}");
    all.remove(0)
}

fn nov3() -> NaiveDate {
    NaiveDate::from_ymd_opt(2026, 11, 3).unwrap()
}

fn view(c: &CompoundNote, actor: Option<&str>) -> CompoundView {
    compound_view(c, id(OPENED), actor, &bne(), nov3())
}

/// "Sign off this note" is offered exactly when the node's rule says the note still needs it —
/// on a linked chart's note too: each vouch files under its part's own chart, and no draft is
/// involved.
#[test]
fn can_sign_off_follows_needs_sign_off() {
    let ada = || Some(who("act-ada", "Dr Ada"));
    let unsigned = one(vec![part(1, OPENED, None, 0, None, Some("act-a"))]);
    assert!(view(&unsigned, None).can_sign_off);
    let signed = one(vec![part(1, OPENED, None, 0, ada(), None)]);
    assert!(!view(&signed, None).can_sign_off, "Dr Ada signed it all");
    let mixed = one(vec![
        part(1, OPENED, None, 0, ada(), None),
        part(2, OPENED, Some(1), 10, Some(who("act-bo", "Dr Bo")), None),
    ]);
    assert!(view(&mixed, None).can_sign_off, "no one signed every part");
    let mut by_ada = part(2, OPENED, Some(1), 10, None, Some("act-a"));
    by_ada.vouches.push(PartVouch {
        event_id: id(90),
        act: VouchAct::SignOff,
        role: VouchRole::Attested,
        by: who("act-ada", "Dr Ada"),
    });
    let signed_off = one(vec![part(1, OPENED, None, 0, ada(), None), by_ada]);
    assert!(!view(&signed_off, None).can_sign_off);
    let linked = one(vec![part(1, LINKED, None, 0, None, Some("act-a"))]);
    let v = view(&linked, None);
    assert!(
        !v.can_add && v.can_sign_off,
        "a linked chart's note may be signed off"
    );
}

/// A part signed by a since-repudiated key counts as unsigned (ADR-0093), so the note offers the
/// press even though a signer is named.
#[test]
fn a_repudiated_part_offers_sign_off() {
    let mut rex = who("act-rex", "Dr Rex");
    rex.repudiated = true;
    let c = one(vec![part(1, OPENED, None, 0, Some(rex), None)]);
    assert!(view(&c, None).can_sign_off);
}

/// The window sends back the ids it showed; the node refuses if the note holds any other.
#[test]
fn shown_parts_are_the_compounds_part_ids_in_order() {
    let c = one(vec![
        part(3, OPENED, Some(1), 20, None, None),
        part(1, OPENED, None, 0, None, None),
        part(2, OPENED, Some(1), 10, None, None),
    ]);
    let ids: Vec<String> = c.parts().iter().map(|p| p.event_id.to_string()).collect();
    assert_eq!(view(&c, None).shown_parts, ids);
    let single = one(vec![part(1, OPENED, None, 0, None, None)]);
    assert_eq!(view(&single, None).shown_parts, [id(1).to_string()]);
}

/// A draft adding to note 1.
fn draft(addendum_to: Uuid) -> NoteDraft {
    let t = utc("2026-11-02T17:12:00Z");
    NoteDraft {
        draft_id: id(50),
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
        addendum_to: Some(addendum_to),
    }
}

/// The strip's line names the claim's OWN unsigned parts, each by its time, "; "-joined, and the
/// ids the webview sends back with Sign. The same words reach "Add to this note" (CompoundView)
/// and a reopened draft (DraftView), from one function.
#[test]
fn the_covers_line_names_the_claims_own_unsigned_parts() {
    let notes = || {
        vec![
            part(1, OPENED, None, 0, Some(who("act-bo", "Dr Bo")), None),
            part(2, OPENED, Some(1), 10, None, Some("act-a")),
            part(3, OPENED, Some(1), 20, None, Some("act-other")),
            part(4, OPENED, Some(1), 30, None, Some("act-a")),
        ]
    };
    let v = view(&one(notes()), Some("act-a"));
    assert_eq!(
        v.covers_line.as_deref(),
        Some(
            "Your signature also covers your earlier unsigned parts of this note: \
             3 Nov 2025 03:10 (stated); 3 Nov 2025 03:30 (stated)"
        )
    );
    assert_eq!(v.covers, [id(2).to_string(), id(4).to_string()]);
    // One part: singular.
    let v = view(&one(notes()), Some("act-other"));
    assert_eq!(
        v.covers_line.as_deref(),
        Some(
            "Your signature also covers your earlier unsigned part of this note: \
             3 Nov 2025 03:20 (stated)"
        )
    );
    assert_eq!(v.covers, [id(3).to_string()]);
    // A part with no stated time is named by when its writing began, as "Adding to …" does —
    // never "no stated time · writing began …", which would run into the next part's words.
    let unstated = |n, min| {
        let mut p = part(n, OPENED, Some(1), min, None, Some("act-a"));
        p.effective_basis = "recording-started".into();
        p
    };
    let root = || part(1, OPENED, None, 0, Some(who("act-bo", "Dr Bo")), None);
    let v = view(&one(vec![root(), unstated(2, 10)]), Some("act-a"));
    assert_eq!(
        v.covers_line.as_deref(),
        Some(
            "Your signature also covers your earlier unsigned part of this note: \
             begun 3 Nov 2025 03:10"
        )
    );
    let stated = part(3, OPENED, Some(1), 20, None, Some("act-a"));
    let v = view(&one(vec![root(), stated, unstated(4, 30)]), Some("act-a"));
    assert_eq!(
        v.covers_line.as_deref(),
        Some(
            "Your signature also covers your earlier unsigned parts of this note: \
             3 Nov 2025 03:20 (stated); begun 3 Nov 2025 03:30"
        )
    );
    // A reopened draft reads back the very same line and ids.
    let list = PatientNoteList {
        notes: notes(),
        withheld_on_linked: 0,
        drafts: vec![draft(id(1))],
        discards: vec![],
        discards_withheld_on_linked: 0,
    };
    let pane = note_pane(
        list,
        id(OPENED),
        Some("pc-1"),
        &Claim::NotAsked,
        Some("act-a"),
        &Default::default(),
        &bne(),
        nov3(),
    );
    assert_eq!(pane.drafts[0].covers_line, pane.notes[0].covers_line);
    assert_eq!(pane.drafts[0].covers, pane.notes[0].covers);
    assert_eq!(pane.notes[0].covers.len(), 2);
}

/// No claim (or a claim whose actor this node does not know), or no unsigned part of theirs: no
/// line, nothing sent back. A part someone vouched for since is signed, so it is not covered.
#[test]
fn no_covers_line_without_a_claim_or_without_parts() {
    let mut vouched = part(2, OPENED, Some(1), 10, None, Some("act-a"));
    vouched.vouches.push(PartVouch {
        event_id: id(90),
        act: VouchAct::Addendum,
        role: VouchRole::Attested,
        by: who("act-a", "Nurse A"),
    });
    let notes = || {
        vec![
            part(1, OPENED, None, 0, Some(who("act-bo", "Dr Bo")), None),
            part(3, OPENED, Some(1), 20, None, Some("act-a")),
        ]
    };
    for (c, actor) in [
        (one(notes()), None),
        // An unknown key's actor is empty, and matches no one.
        (one(notes()), Some("")),
        (one(notes()), Some("act-nobody")),
        (
            one(vec![
                part(1, OPENED, None, 0, Some(who("act-a", "Nurse A")), None),
                vouched.clone(),
            ]),
            Some("act-a"),
        ),
    ] {
        let v = view(&c, actor);
        assert_eq!(v.covers_line, None, "{actor:?}");
        assert!(v.covers.is_empty(), "{actor:?}");
    }
    // A draft whose note is not shown has no line either.
    let pane = note_pane(
        PatientNoteList {
            notes: notes(),
            withheld_on_linked: 0,
            drafts: vec![draft(id(404))],
            discards: vec![],
            discards_withheld_on_linked: 0,
        },
        id(OPENED),
        Some("pc-1"),
        &Claim::NotAsked,
        Some("act-a"),
        &Default::default(),
        &bne(),
        nov3(),
    );
    assert_eq!(pane.drafts[0].covers_line, None);
    assert!(pane.drafts[0].covers.is_empty());
}

/// Each refusal is the exact sentence, and asks for a re-read: the pane on screen is behind the
/// record (a part landed or went; the clinician signed it off in another window). The chart on
/// screen and fixture mode come first, as Sign's do.
#[tokio::test]
async fn sign_off_refusals_are_worded_exactly() {
    let changed = sign_off_refusal_view(SignOffRefusal::Changed);
    assert_eq!(
        changed.text,
        "This note changed since it was shown — read the chart again, then sign it off. \
         Nothing was signed."
    );
    assert_eq!(changed.retry, Retry::AfterReread(Recorded::No));
    let nothing = sign_off_refusal_view(SignOffRefusal::NothingToSign);
    assert_eq!(
        nothing.text,
        "You have already signed every part of this note. Nothing more was signed."
    );
    assert_eq!(nothing.retry, Retry::AfterReread(Recorded::No));
    assert_eq!(
        SIGNED_OFF,
        "Signed off — every part of this note now carries your signature."
    );
    assert_eq!(
        sign_off_unknown_words(id(5), "connection reset"),
        format!(
            "Whether this note was signed off is NOT known — no ordinary answer came back while \
             signing it off (event {}: connection reset). Once the window can reach the node \
             again, the chart shows whether it was signed off.",
            id(5)
        )
    );

    let chart = id(OPENED);
    let state = AppState::mock(Some(chart));
    let shown = vec![id(1).to_string()];
    let err = note_sign_off_impl(&state, &id(424).to_string(), "x", &shown, None)
        .await
        .unwrap_err();
    assert!(err.text.contains("not the chart"), "{}", err.text);
    let err = note_sign_off_impl(&state, &chart.to_string(), "x", &shown, None)
        .await
        .unwrap_err();
    assert!(err.text.contains("fixture mode"), "{}", err.text);
}

/// Final review M7: a sign-off the node failed says what failed before the node's own sentence,
/// as Sign's "Could not sign this note: " does, with the node failure's retry unchanged. Only the
/// node's sentence takes it: a refusal and the key's words are whole sentences already.
#[test]
fn a_failed_sign_off_leads_with_what_failed() {
    let e = anyhow::anyhow!("note attestation: the floor said no");
    let view = sign_off_failure(&e);
    assert_eq!(
        view.text,
        "Could not sign off this note: note attestation: the floor said no"
    );
    assert_eq!(view.retry, node_failure(&e).retry);
    let source = include_str!("notes_sign_off.rs");
    assert!(source.contains("None => return Err(sign_off_failure(&e)),"));
    assert!(!source.contains("Err(node_failure(&e))"), "{source}");
}

/// A malformed root or shown id is refused as "not a note id" before the key is asked for: a
/// passphrase that signs nothing is a step paper never has. Sign's list is read the same way.
#[test]
fn sign_off_asks_for_no_key_on_a_malformed_id() {
    let good = id(1).to_string();
    let (root, shown) = sign_off_ids(&good, &[good.clone(), id(2).to_string()]).unwrap();
    assert_eq!((root, shown), (id(1), vec![id(1), id(2)]));
    assert_eq!(
        sign_off_ids("junk", std::slice::from_ref(&good))
            .unwrap_err()
            .text,
        "not a note id"
    );
    assert_eq!(
        sign_off_ids(&good, &["junk".into()]).unwrap_err().text,
        "not a note id"
    );
    // A Sign the webview sent without a list under-claims (vouches nothing), never fails.
    assert_eq!(
        crate::notes::parse_note_ids(None).unwrap(),
        Vec::<Uuid>::new()
    );
    assert_eq!(
        crate::notes::parse_note_ids(Some(&[])).unwrap(),
        Vec::<Uuid>::new()
    );
    assert_eq!(
        crate::notes::parse_note_ids(Some(&["junk".into()]))
            .unwrap_err()
            .text,
        "not a note id"
    );
    // The sign-off reads its ids before the key, as Sign does (a CI-running source pin: the
    // fixture window refuses before either is reached).
    let flat: String =
        crate::source_guard::rust_fn(include_str!("notes_sign_off.rs"), "note_sign_off_impl")
            .split_whitespace()
            .collect();
    let ids = flat.find("sign_off_ids(").expect(&flat);
    assert!(ids < flat.find("signer_for_act").unwrap(), "{flat}");
    // Sign parses its list before the key, and hands the node exactly that list.
    let flat: String = crate::source_guard::rust_fn(include_str!("notes.rs"), "note_sign_impl")
        .split_whitespace()
        .collect();
    let parse = flat.find("parse_note_ids(also_vouches)?").expect(&flat);
    assert!(parse < flat.find("signer_for_act").unwrap(), "{flat}");
    assert!(flat.contains("also_vouches:&listed,"), "{flat}");
}

/// Sign's success line says how many of the strip's parts now read as signed, and how many were
/// not the signer's by key (Review Focus 1: Nurse B signs while who's here still says Nurse A).
#[test]
fn the_sign_success_line_counts_vouched_and_unvouched() {
    const SIGNED: &str = "Signed — the note is on the chart.";
    assert_eq!(signed_words(0, 0), SIGNED);
    assert_eq!(
        signed_words(1, 1),
        format!("{SIGNED} Your earlier unsigned part now reads as signed.")
    );
    assert_eq!(
        signed_words(3, 3),
        format!("{SIGNED} Your 3 earlier unsigned parts now read as signed.")
    );
    assert_eq!(
        signed_words(0, 2),
        format!("{SIGNED} 2 listed part(s) were not covered: not yours by key, or already signed.")
    );
    assert_eq!(
        signed_words(1, 3),
        format!(
            "{SIGNED} Your earlier unsigned part now reads as signed. 2 listed part(s) were \
             not covered: not yours by key, or already signed."
        )
    );
}
