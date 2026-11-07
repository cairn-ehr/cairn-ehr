//! Tests for the person menu's words (`person_menu_view.rs`) and the commands that need no
//! database (`person_menu.rs`). Every exact string of the slice's copy is pinned here.
use crate::person_menu::{person_menu_impl, whos_here_counts_impl};
use crate::person_menu_view::*;
use crate::state::AppState;
use crate::whos_here::Claim;
use cairn_node::note::menu::{MenuItem, Person, PersonMenu, Why};
use cairn_node::note::menu_read::MenuRead;
use cairn_patient_search::{Candidate, TrustState};
use chrono::{DateTime, FixedOffset, TimeZone, Utc};
use std::collections::HashMap;
use uuid::Uuid;

fn utc() -> FixedOffset {
    FixedOffset::east_opt(0).unwrap()
}

fn at(h: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 7, h, 0, 0).unwrap()
}

fn person(actor: &str, name: Option<&str>) -> Person {
    Person {
        actor: actor.into(),
        staff_id: actor.trim_start_matches("act-").into(),
        display_name: name.map(String::from),
        keys: vec![],
    }
}

fn read(people: Vec<Person>) -> MenuRead {
    MenuRead {
        people,
        drafts: vec![],
        parts: vec![],
    }
}

fn patients(chart: Uuid, name: &str) -> HashMap<Uuid, Candidate> {
    let c = Candidate {
        patient_id: chart,
        display_name: name.into(),
        age: None,
        trust: TrustState::Confirmed,
        last_activity: None,
        locale: None,
        photo_ref: None,
    };
    HashMap::from([(chart, c)])
}

fn item(chart: Uuid, part: Option<Uuid>, writer: Option<&str>, why: Why) -> MenuItem {
    MenuItem {
        chart,
        part,
        when: at(9),
        writer: writer.map(String::from),
        why,
    }
}

fn parts(v: &PersonMenuView) -> (&MenuGroupView, &MenuGroupView, &MenuGroupView) {
    match v {
        PersonMenuView::Menu {
            own,
            cosign,
            review,
            ..
        } => (own, cosign, review),
        PersonMenuView::Nobody => panic!("expected a menu"),
    }
}

#[test]
fn headings_count_and_empty_groups_are_exact() {
    let r = read(vec![]);
    let none = menu_view(&PersonMenu::default(), &r, &HashMap::new(), &utc(), at(12));
    match &none {
        PersonMenuView::Menu {
            count,
            count_words,
            count_label,
            ..
        } => {
            assert_eq!(*count, 0);
            assert_eq!(count_words, "Waiting for you: 0");
            assert_eq!(count_label, "Waiting for you: 0 — open the list");
        }
        PersonMenuView::Nobody => panic!(),
    }
    let (own, cosign, review) = parts(&none);
    assert_eq!(own.heading, "Your signature");
    assert_eq!(cosign.heading, "Needs your co-signature");
    assert_eq!(
        review.heading,
        "Your supervisees' recent notes (last 7 days)"
    );
    for g in [own, cosign, review] {
        assert_eq!(g.empty.as_deref(), Some("Nothing here."));
        assert!(g.rows.is_empty());
    }
}

#[test]
fn the_count_is_own_plus_cosign_never_review() {
    let (c, p) = (Uuid::from_u128(1), Uuid::from_u128(2));
    let menu = PersonMenu {
        own: vec![item(c, Some(p), None, Why::WentInUnsigned)],
        cosign: vec![
            item(
                c,
                Some(p),
                Some("act-bea"),
                Why::NeedsCoSign { others: vec![] }
            );
            2
        ],
        review: vec![item(c, Some(p), Some("act-bea"), Why::Review); 5],
    };
    let v = menu_view(&menu, &read(vec![]), &HashMap::new(), &utc(), at(12));
    match v {
        PersonMenuView::Menu {
            count, count_words, ..
        } => {
            assert_eq!(count, 3);
            assert_eq!(count_words, "Waiting for you: 3");
        }
        PersonMenuView::Nobody => panic!(),
    }
}

#[test]
fn rows_say_what_state_and_how_to_open() {
    let (c, p, d) = (Uuid::from_u128(1), Uuid::from_u128(2), Uuid::from_u128(3));
    let r = read(vec![
        person("act-bea", Some("Dr Bea")),
        person("act-cy", Some("Dr Cy")),
        person("act-al", Some("Dr Al")),
        person("act-di", None),
    ]);
    let menu = PersonMenu {
        own: vec![
            item(
                c,
                None,
                None,
                Why::Draft {
                    draft_id: d,
                    due: at(10),
                    stuck: true,
                },
            ),
            item(c, Some(p), None, Why::WentInUnsigned),
            item(c, Some(p), None, Why::KeyRepudiated),
        ],
        cosign: vec![
            item(
                c,
                Some(p),
                Some("act-bea"),
                Why::NeedsCoSign { others: vec![] },
            ),
            item(
                c,
                Some(p),
                Some("act-bea"),
                Why::NeedsCoSign {
                    others: vec!["act-cy".into(), "act-al".into(), "act-di".into()],
                },
            ),
            item(
                c,
                Some(p),
                Some("act-bea"),
                Why::NeedsCoSign {
                    others: vec!["act-cy".into()],
                },
            ),
        ],
        review: vec![
            item(c, Some(p), Some("act-bea"), Why::Review),
            item(c, Some(p), Some("act-nobody"), Why::Review),
        ],
    };
    let v = menu_view(&menu, &r, &patients(c, "Ann Jones"), &utc(), at(12));
    let (own, cosign, review) = parts(&v);
    let when = cairn_node::note::view::instant(at(9), &utc(), at(12).date_naive());

    let draft = &own.rows[0];
    assert_eq!(draft.what, "Draft note");
    assert_eq!(draft.focus, format!("draft:{d}"));
    assert_eq!(draft.patient, "Ann Jones");
    assert_eq!(draft.patient_id, c.to_string());
    assert_eq!(draft.open_label, format!("Open note: Ann Jones, {when}"));
    assert_eq!(
        draft.state,
        crate::notes::commit::due_words(at(10), true, true, false, &utc(), at(12).date_naive())
    );
    assert_eq!(own.rows[1].what, "Progress note");
    assert_eq!(own.rows[1].focus, format!("part:{p}"));
    assert_eq!(own.rows[1].state, "Went in unsigned — sign it");
    assert_eq!(
        own.rows[2].state,
        "Counts as unsigned — check it is yours, then sign or correct it"
    );
    assert_eq!(cosign.rows[0].state, "By Dr Bea — needs your co-signature");
    assert_eq!(
        cosign.rows[1].state,
        "By Dr Bea — needs your co-signature (you or Dr Al, Dr Cy or a writer this node cannot name)"
    );
    assert_eq!(
        cosign.rows[2].state,
        "By Dr Bea — needs your co-signature (you or Dr Cy)"
    );
    assert_eq!(review.rows[0].state, "By Dr Bea — not yet reviewed");
    assert_eq!(
        review.rows[1].state,
        "By a writer this node cannot name — not yet reviewed"
    );
    assert!(own.empty.is_none());
}

#[test]
fn a_chart_the_read_could_not_name_is_said_so() {
    let c = Uuid::from_u128(1);
    let menu = PersonMenu {
        own: vec![item(c, Some(c), None, Why::WentInUnsigned)],
        ..Default::default()
    };
    let v = menu_view(&menu, &read(vec![]), &HashMap::new(), &utc(), at(12));
    assert_eq!(parts(&v).0.rows[0].patient, "a chart this node cannot name");
}

#[test]
fn picker_suffix_only_when_positive() {
    let v = counts_view(vec![("sue".into(), 3), ("bea".into(), 0)]);
    assert_eq!(v[0].suffix, " · 3 waiting");
    assert_eq!(v[0].count, 3);
    assert_eq!(v[1].suffix, "");
    assert_eq!(COUNTS_FAILED, "Waiting counts could not be read.");
    assert_eq!(
        NO_LONGER_WAITS,
        "That note no longer waits in your list — the list was read again."
    );
}

#[tokio::test]
async fn nobody_named_reads_no_database() {
    let state = AppState::mock(None);
    for claim in [Claim::NotAsked, Claim::Unknown] {
        state.whos_here.lock().await.pick(claim);
        assert!(matches!(
            person_menu_impl(&state).await.unwrap(),
            PersonMenuView::Nobody
        ));
    }
    // Fixture mode with somebody named: an empty menu, and no counts.
    state.whos_here.lock().await.pick(Claim::Named {
        staff_id: "sue".into(),
        display_name: "Dr Sue".into(),
    });
    match person_menu_impl(&state).await.unwrap() {
        PersonMenuView::Menu { count, .. } => assert_eq!(count, 0),
        PersonMenuView::Nobody => panic!(),
    }
    assert!(whos_here_counts_impl(&state).await.unwrap().is_empty());
}
