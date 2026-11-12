//! Pure tests for the medication list's commands (`med_list.rs`, `med_list_judge.rs`,
//! `med_list_sign.rs`): the outcome's words, the writer rule, which ticked ids a Sign may cover and
//! whether a tick still holds. No database; the DB-gated walks are `med_list_db_tests.rs`.
use super::*;
use crate::med_chart::NAME_IT;
use crate::med_list_sign::{
    line_failure, med_list_changed, not_charted_line, outcome, ticked_lines, writer_matches,
    LineFailure, LINE_MOVED, MED_LIST_STALE,
};
use cairn_node::allergy::check::LineCheck;

/// A judged line held by `terminal`, ticked by hand or not, with `check` as its fresh check.
fn judged(id: u128, terminal: &str, hand_ticked: bool, check: LineCheck) -> Judged {
    Judged {
        line: DraftLine {
            line_id: Uuid::from_u128(id),
            terminal_id: terminal.into(),
            seq: id as i32,
            charted_as: "started-here".into(),
            line_text: format!("line {id}"),
            parts: serde_json::json!([]),
            reason: None,
            hand_ticked,
            ticked_against: vec![],
            finished: true,
            saved_at: "2026-11-11 10:00:00+00".into(),
            typed_drug: format!("drug {id}"),
            directions: String::new(),
            held: false,
        },
        parts: vec![],
        signable: Ok((format!("drug {id}"), vec![])),
        read: Directions::default(),
        check,
        duplicate: false,
        duplicate_of: None,
        default_ticked: false,
        offers: vec![],
    }
}

/// The outcome says how many went in, then names each line that did not, with its reason, in the
/// plan's exact words.
#[test]
fn the_outcome_words_name_each_line_not_charted() {
    let lines = [
        not_charted_line("  amox 500mg tds", NAME_IT),
        not_charted_line("warfarin 5 mg", "the line changed"),
    ];
    assert_eq!(
        outcome(2, 4, &lines),
        "Charted 2 of 4\n  amox 500mg tds — not charted: Name the medication.\n\
         warfarin 5 mg — not charted: the line changed"
    );
    assert_eq!(outcome(3, 3, &[]), "Charted 3 of 3");
    assert_eq!(
        med_list_changed(2),
        "2 ticked line(s) changed since they were ticked: nothing was signed. Look at them again"
    );
}

/// Review Focus 3: a staff id typed in the strip that is not the list's writer is refused, naming
/// the writer; an id the vault could not normalise is no one's.
#[test]
fn a_credential_for_another_staff_is_not_yours() {
    let err = writer_matches(Some("sue"), Some("bea"), "Dr Bea").unwrap_err();
    assert_eq!(
        err.text,
        "This list was written by Dr Bea; only they can sign it"
    );
    assert!(writer_matches(None, Some("bea"), "Dr Bea").is_err());
    assert!(writer_matches(Some("bea"), Some("bea"), "Dr Bea").is_ok());
}

/// A ticked id that is not a line of the fresh read, or is held by another terminal, makes the
/// whole Sign stale; the lines covered come back in LIST order, whatever order was sent.
#[test]
fn a_ticked_id_the_read_does_not_hold_here_is_stale() {
    let list = [
        judged(1, "here", true, LineCheck::default()),
        judged(2, "here", true, LineCheck::default()),
        judged(3, "there", true, LineCheck::default()),
    ];
    let id = |n: u128| Uuid::from_u128(n).to_string();
    let got = ticked_lines(&list, &[id(2), id(1)], "here").unwrap();
    let order: Vec<Uuid> = got.iter().map(|j| j.line.line_id).collect();
    assert_eq!(order, [Uuid::from_u128(1), Uuid::from_u128(2)]);
    for bad in [vec![id(3)], vec![id(9)], vec!["not-a-uuid".into()]] {
        assert_eq!(
            ticked_lines(&list, &bad, "here").unwrap_err().text,
            MED_LIST_STALE
        );
    }
}

/// A default tick holds only while the fresh facts still give it; a hand tick holds until a
/// warning it was not given against appears (design §3.2).
#[test]
fn a_tick_holds_by_its_own_rule() {
    let mut j = judged(1, "here", false, LineCheck::default());
    assert!(!tick_holds(&j), "no default tick on the fresh read");
    j.default_ticked = true;
    assert!(tick_holds(&j));
    let warned = LineCheck {
        marks: vec!["Allergy: amoxicillin".into()],
        against: vec![Uuid::from_u128(77)],
        ..LineCheck::default()
    };
    let mut j = judged(2, "here", true, warned);
    j.line.reason = Some("no alternative".into());
    assert!(!tick_holds(&j), "a warning arrived after the tick");
    j.line.ticked_against = vec![("allergy".into(), Uuid::from_u128(77))];
    assert!(tick_holds(&j), "the warning was seen when ticked");
}

/// I4: a spend that found the list moved under it (`SpendRefused`) is "look at it again", never a
/// node failure, also when `db.explain` has wrapped it in a cut-off context.
#[test]
fn a_moved_list_is_not_a_node_failure() {
    use cairn_node::medication::draft::SpendRefused;
    let bare = anyhow::Error::from(SpendRefused("changed".into()));
    assert_eq!(
        line_failure(&bare),
        LineFailure::NotCharted(LINE_MOVED.into())
    );
    let wrapped = anyhow::Error::from(SpendRefused("moved".into())).context("cut off");
    assert_eq!(
        line_failure(&wrapped),
        LineFailure::NotCharted(LINE_MOVED.into())
    );
    let other = anyhow::anyhow!("the node said no");
    assert_ne!(
        line_failure(&other),
        LineFailure::NotCharted(LINE_MOVED.into())
    );
}

/// R1 (the core of the "a tick covers what was SEEN" fix): a hand tick vouches only for the
/// warnings found NOW that were also DRAWN on the line. Kept here, DB-free, because the GUI's DB
/// walks do not run in CI (#905).
#[test]
fn a_tick_vouches_only_for_warnings_both_found_and_drawn() {
    let (a, b, c) = (
        Uuid::from_u128(0xA),
        Uuid::from_u128(0xB),
        Uuid::from_u128(0xC),
    );
    let seen = |source: &str, id: String| WarningRef {
        source: source.into(),
        reference: id,
    };
    let allergy = |id: Uuid| (ALLERGY_SOURCE.to_string(), id);
    // The intersection: b was found but never drawn; c was drawn but is gone now.
    let drawn = [
        seen("allergy", a.to_string()),
        seen("allergy", c.to_string()),
    ];
    assert_eq!(vouched(&[a, b], &drawn), [allergy(a)]);
    // Nothing drawn (or nothing sent): the tick vouches for nothing.
    assert!(vouched(&[a, b], &[]).is_empty());
    // A ref drawn in upper case is the same warning.
    let upper = [seen("allergy", a.to_string().to_uppercase())];
    assert_eq!(vouched(&[a], &upper), [allergy(a)]);
    // A ref under another source is not an allergy warning, whatever its id.
    assert!(vouched(&[a], &[seen("interaction", a.to_string())]).is_empty());
}

/// Review C1: the clinician's veto voids a default tick — on screen and at Sign alike.
#[test]
fn a_held_default_tick_does_not_hold() {
    let mut j = judged(1, "here", false, LineCheck::default());
    j.default_ticked = true;
    assert!(tick_holds(&j));
    j.line.held = true;
    assert!(!tick_holds(&j), "vetoed");
    assert!(!line_view(&j).ticked, "and drawn unticked");
    assert!(
        line_view(&j).default_ticked,
        "the default tick is still derived"
    );
}

/// Review I2: a duplicate names the line it duplicates — another list line first, else the chart's
/// current line — in Rust's words; an uncoded line or a combination never duplicates.
#[test]
fn a_duplicate_names_the_other_line_or_the_chart_line() {
    use crate::med_list_judge::duplicate_of;
    let codes = [Some("m1"), None, Some("m1"), Some("m2")];
    let texts = [
        "amox 500mg tds",
        "paracetamol 1g",
        "amox 1g bd",
        "warfarin 5mg",
    ];
    let current = [("m2".to_string(), "Warfarin 5 mg".to_string())];
    assert_eq!(
        duplicate_of(0, &codes, &texts, &current).as_deref(),
        Some("amox 1g bd")
    );
    assert_eq!(
        duplicate_of(2, &codes, &texts, &current).as_deref(),
        Some("amox 500mg tds")
    );
    assert_eq!(
        duplicate_of(3, &codes, &texts, &current).as_deref(),
        Some("Warfarin 5 mg")
    );
    assert_eq!(duplicate_of(1, &codes, &texts, &current), None);
    assert_eq!(
        already_on_the_list("Warfarin 5 mg"),
        "already on the list: Warfarin 5 mg"
    );
}
