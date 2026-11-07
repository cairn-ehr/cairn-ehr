//! Tests for `menu.rs` (sign-in slice 3c-3c). Parts are built on `vouch::fixture` and, where the
//! "as of" logic matters (Review Focus 1-3), stamped by `supervision::annotate` over real rows,
//! so the record is exercised rather than assumed. Every part is recorded at `base()`.
use super::*;
use crate::note::supervision::{annotate, Supervision, SupervisionRow};
use crate::note::vouch::fixture::{part, repudiated, vouch, who};
use cairn_event::note_attestation::{VouchAct, VouchRole};

fn base() -> DateTime<Utc> {
    part(0, None).recorded_at
}
fn ms(m: i64) -> DateTime<Utc> {
    base() + Duration::minutes(m)
}
/// A supervision row `m` minutes after the parts were recorded.
fn row(op: &str, actor: &str, related: Option<&str>, m: i64) -> SupervisionRow {
    SupervisionRow {
        op: op.into(),
        actor: actor.into(),
        related: related.map(Into::into),
        at: ms(m),
    }
}
fn link(supervisee: &str, supervisor: &str, m: i64) -> SupervisionRow {
    row("link-supervisor", supervisee, Some(supervisor), m)
}
fn annotated(mut p: NoteRow, rows: Vec<SupervisionRow>) -> NoteRow {
    annotate(&mut p, &Supervision::from_rows(rows).unwrap());
    p
}
fn person(actor: &str) -> Person {
    Person {
        actor: actor.into(),
        staff_id: format!("staff-{actor}"),
        display_name: None,
        keys: vec![],
    }
}
fn now() -> DateTime<Utc> {
    ms(60 * 24)
}
fn ids(items: &[MenuItem]) -> Vec<Option<Uuid>> {
    items.iter().map(|i| i.part).collect()
}
fn id(n: u128) -> Option<Uuid> {
    Some(Uuid::from_u128(n))
}
/// Dr Pro's (p1) part `n`, written under a co-signing requirement, supervised by `sv`.
fn owing(n: u128, rows: Vec<SupervisionRow>) -> NoteRow {
    annotated(part(n, Some(who("p1", "Dr Pro"))), rows)
}
fn required_and_linked() -> Vec<SupervisionRow> {
    vec![
        row("require-cosign", "p1", None, -10),
        link("p1", "sv", -10),
    ]
}
fn cosigned_by(p: &mut NoteRow, by: &str, rows: Vec<SupervisionRow>) {
    p.vouches.push(vouch(
        9,
        VouchAct::SignOff,
        VouchRole::CoSigned,
        who(by, by),
    ));
    annotate(p, &Supervision::from_rows(rows).unwrap());
}

#[test]
fn a_draft_with_my_part_is_in_my_group_and_counted() {
    let d = |writer: &str| MenuDraft {
        writer: writer.into(),
        draft_id: Uuid::from_u128(5),
        chart: Uuid::from_u128(900),
        due: ms(30),
        stuck: true,
    };
    let m = classify(
        &person("me"),
        &[],
        &[d("staff-me"), d("staff-other")],
        now(),
    );
    assert_eq!(m.own.len(), 1);
    assert_eq!(m.own[0].part, None);
    assert_eq!(m.own[0].when, ms(30));
    assert!(matches!(m.own[0].why, Why::Draft { stuck: true, .. }));
    assert_eq!(m.count(), 1);
}

#[test]
fn a_part_that_went_in_unsigned_is_mine_until_anyone_signs_it() {
    let mut p = part(1, None);
    p.drafted_actor = Some("me".into());
    let m = classify(&person("me"), &[p.clone()], &[], now());
    assert_eq!(m.own[0].why, Why::WentInUnsigned);
    assert_eq!(m.own[0].writer, None);
    // Someone else's drafted part is not mine.
    assert_eq!(classify(&person("zz"), &[p.clone()], &[], now()).count(), 0);
    // A colleague's attested sign-off takes it off the list.
    p.vouches.push(vouch(
        9,
        VouchAct::SignOff,
        VouchRole::Attested,
        who("z", "Dr Z"),
    ));
    assert_eq!(classify(&person("me"), &[p], &[], now()).count(), 0);
}

#[test]
fn a_part_signed_with_my_repudiated_key_is_mine_until_i_re_sign() {
    let mut p = part(1, Some(repudiated("me", "Dr Me")));
    p.vouches.push(vouch(
        8,
        VouchAct::SignOff,
        VouchRole::CoSigned,
        who("z", "Dr Z"),
    ));
    let m = classify(&person("me"), &[p.clone()], &[], now());
    assert_eq!(m.own[0].why, Why::KeyRepudiated);
    assert_eq!(m.count(), 1, "a colleague's co-sign does not remove it");
    // My vouch under a standing key (same actor, re-keyed) does.
    p.vouches.push(vouch(
        9,
        VouchAct::SignOff,
        VouchRole::Attested,
        who("me", "Dr Me"),
    ));
    assert_eq!(classify(&person("me"), &[p], &[], now()).count(), 0);
}

#[test]
fn a_repudiated_part_is_in_nobodys_cosign_group() {
    let p = annotated(
        part(1, Some(repudiated("p1", "Dr Pro"))),
        required_and_linked(),
    );
    assert!(p.needs_cosigning);
    let sv = classify(&person("sv"), &[p.clone()], &[], now());
    assert_eq!((sv.cosign.len(), sv.review.len()), (0, 0));
    assert_eq!(classify(&person("p1"), &[p], &[], now()).own.len(), 1);
}

#[test]
fn a_supervisees_part_awaiting_cosign_is_in_my_cosign_group_naming_the_others() {
    let mut rows = required_and_linked();
    let alone = owing(1, rows.clone());
    let m = classify(&person("sv"), &[alone], &[], now());
    assert_eq!(m.cosign[0].why, Why::NeedsCoSign { others: vec![] });
    assert_eq!(m.cosign[0].writer.as_deref(), Some("p1"));
    assert_eq!(m.count(), 1);

    rows.push(link("p1", "p", -5));
    let shared = owing(2, rows);
    let both = [
        classify(&person("sv"), &[shared.clone()], &[], now()),
        classify(&person("p"), &[shared], &[], now()),
    ];
    assert_eq!(
        both[0].cosign[0].why,
        Why::NeedsCoSign {
            others: vec!["p".into()]
        }
    );
    assert_eq!(
        both[1].cosign[0].why,
        Why::NeedsCoSign {
            others: vec!["sv".into()]
        }
    );
    // The writer is never their own co-signer.
    let own = owing(3, required_and_linked());
    assert_eq!(classify(&person("p1"), &[own], &[], now()).count(), 0);
}

/// Review Focus 1: required at the part's time, lifted since, still waits.
#[test]
fn a_requirement_lifted_after_signing_keeps_the_part_waiting() {
    let mut rows = required_and_linked();
    rows.push(row("lift-cosign", "p1", None, 10));
    let m = classify(&person("sv"), &[owing(1, rows)], &[], now());
    assert_eq!(m.cosign.len(), 1);
}

/// Review Focus 2: a supervisor who stepped away sees nothing of that supervisee.
#[test]
fn a_supervisor_who_stepped_away_sees_nothing_of_that_supervisee() {
    let mut rows = required_and_linked();
    rows.push(row("unlink-supervisor", "p1", Some("sv"), 10));
    rows.push(link("p1", "p", 10));
    let waiting = owing(1, rows.clone());
    let mut free = owing(2, rows.clone());
    free.needs_cosigning = false; // a note that owed no co-sign: group 3 material for a supervisor
    let m = classify(&person("sv"), &[waiting, free], &[], now());
    assert_eq!((m.count(), m.review.len()), (0, 0));
}

/// Review Focus 2: their earlier co-sign still clears the part for the remaining supervisor.
#[test]
fn a_cosign_by_a_since_unlinked_supervisor_still_clears_it_for_the_other() {
    let mut rows = required_and_linked();
    rows.push(row("unlink-supervisor", "p1", Some("sv"), 10));
    rows.push(link("p1", "p", 10));
    let mut p = owing(1, rows.clone());
    assert_eq!(
        classify(&person("p"), &[p.clone()], &[], now())
            .cosign
            .len(),
        1
    );
    cosigned_by(&mut p, "sv", rows);
    assert!(p.vouches[0].by_supervisor, "judged at the vouch's own time");
    assert_eq!(classify(&person("p"), &[p], &[], now()).count(), 0);
}

#[test]
fn a_writer_with_no_supervisor_now_is_in_nobodys_cosign_group() {
    let p = owing(1, vec![row("require-cosign", "p1", None, -10)]);
    for who in ["sv", "p1", "zz"] {
        assert_eq!(classify(&person(who), &[p.clone()], &[], now()).count(), 0);
    }
}

/// Review Focus 4: group 3 is windowed inclusive at seven days and never counted.
#[test]
fn a_reviewable_part_is_listed_for_seven_days_and_never_counted() {
    let window = Duration::days(REVIEW_DAYS);
    let mk = |n, at: DateTime<Utc>| {
        let mut p = annotated(
            part(n, Some(who("p1", "Dr Pro"))),
            vec![link("p1", "sv", -10)],
        );
        p.recorded_at = at;
        p
    };
    let edge = mk(1, now() - window);
    let stale = mk(2, now() - window - Duration::milliseconds(1));
    let m = classify(&person("sv"), &[edge, stale], &[], now());
    assert_eq!(ids(&m.review), [id(1)]);
    assert_eq!(m.review[0].why, Why::Review);
    assert_eq!(m.review[0].writer.as_deref(), Some("p1"));
    assert_eq!(m.count(), 0);
}

#[test]
fn a_part_one_supervisor_reviewed_leaves_every_supervisors_review_group() {
    let rows = vec![link("p1", "sv", -10), link("p1", "p", -10)];
    let mut p = annotated(part(1, Some(who("p1", "Dr Pro"))), rows.clone());
    assert_eq!(
        classify(&person("sv"), &[p.clone()], &[], now())
            .review
            .len(),
        1
    );
    p.vouches.push(vouch(
        9,
        VouchAct::SignOff,
        VouchRole::Attested,
        who("p", "Dr P"),
    ));
    annotate(&mut p, &Supervision::from_rows(rows).unwrap());
    for who in ["sv", "p"] {
        assert!(classify(&person(who), &[p.clone()], &[], now())
            .review
            .is_empty());
    }
}

#[test]
fn my_own_signed_part_is_in_none_of_my_groups() {
    let p = annotated(
        part(1, Some(who("me", "Dr Me"))),
        vec![link("me", "me", -10)],
    );
    let m = classify(&person("me"), &[p], &[], now());
    assert_eq!((m.own.len(), m.cosign.len(), m.review.len()), (0, 0, 0));
}

#[test]
fn rows_are_ordered_drafts_by_due_then_parts_newest_first() {
    let draft = |n: u128, due| MenuDraft {
        writer: "staff-me".into(),
        draft_id: Uuid::from_u128(n),
        chart: Uuid::from_u128(900),
        due,
        stuck: false,
    };
    let unsigned = |n: u128, at| {
        let mut p = part(n, None);
        p.drafted_actor = Some("me".into());
        p.recorded_at = at;
        p
    };
    let parts = [
        unsigned(1, ms(1)),
        unsigned(2, ms(3)),
        unsigned(4, ms(2)),
        unsigned(3, ms(2)),
    ];
    let m = classify(
        &person("me"),
        &parts,
        &[draft(7, ms(50)), draft(6, ms(40))],
        now(),
    );
    let order: Vec<_> = m.own.iter().map(|i| (i.part, i.when)).collect();
    assert_eq!(
        order,
        [
            (None, ms(40)),
            (None, ms(50)),
            (id(2), ms(3)),
            (id(3), ms(2)),
            (id(4), ms(2)),
            (id(1), ms(1))
        ]
    );
}

#[test]
fn a_person_with_no_actor_gets_an_empty_menu() {
    let mut p = part(1, None);
    p.drafted_actor = Some(String::new());
    let d = MenuDraft {
        writer: String::new(),
        draft_id: Uuid::from_u128(5),
        chart: Uuid::from_u128(900),
        due: ms(1),
        stuck: false,
    };
    let nobody = Person {
        actor: String::new(),
        staff_id: String::new(),
        ..person("x")
    };
    assert_eq!(classify(&nobody, &[p], &[d], now()), PersonMenu::default());
}
