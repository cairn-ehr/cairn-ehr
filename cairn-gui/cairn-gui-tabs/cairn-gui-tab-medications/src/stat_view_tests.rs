//! Tests for `stat_view.rs`: the stat line's words (charting slice 2, Global Constraints' copy).
//! The workstation is fixed at UTC+10, so every time below reads as its local clock time.
use crate::stat_view::{stat_words, struck_words, GiveIn, StatWordsIn};
use chrono::{DateTime, FixedOffset, TimeZone, Utc};

/// 09:00 on 14 Nov at the workstation (UTC+10).
fn now() -> DateTime<FixedOffset> {
    FixedOffset::east_opt(10 * 3600)
        .unwrap()
        .with_ymd_and_hms(2026, 11, 14, 9, 0, 0)
        .unwrap()
}

/// `h:m` this morning at the workstation, as UTC (the day before, in UTC).
fn at(h: u32, m: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 13, h + 14, m, 0).unwrap()
}

/// An order charted at 08:00 by Dr Ann Lee, with `gives`.
fn order(gives: Vec<GiveIn>) -> StatWordsIn {
    StatWordsIn {
        ordered_at: at(8, 0),
        orderer: Some("Dr Ann Lee".into()),
        gives,
    }
}

fn give(h: u32, m: u32, giver: &str) -> GiveIn {
    GiveIn {
        given_at: at(h, m),
        giver: Some(giver.into()),
        struck: None,
    }
}

#[test]
fn not_yet_given_reads_ordered_then_not_yet_given() {
    assert_eq!(
        stat_words(&order(vec![]), &now()),
        "stat · ordered 08:00 Dr Ann Lee · not yet given"
    );
    assert!(struck_words(&order(vec![]), &now()).is_empty());
}

/// Gives chain after the order in time order, whatever order they arrive in (two terminals).
#[test]
fn given_reads_in_time_order() {
    assert_eq!(
        stat_words(&order(vec![give(8, 20, "RN Bo")]), &now()),
        "stat · ordered 08:00 Dr Ann Lee · given 08:20 RN Bo"
    );
    let two = order(vec![give(8, 40, "RN Cy"), give(8, 20, "RN Bo")]);
    assert_eq!(
        stat_words(&two, &now()),
        "stat · ordered 08:00 Dr Ann Lee · given 08:20 RN Bo · given 08:40 RN Cy"
    );
}

/// §10 Q3: a dose given before the order was charted (a verbal order, charted after) is shown as
/// such, never refused and never reordered.
#[test]
fn given_before_charted_reads_before_it_was_charted() {
    assert_eq!(
        stat_words(&order(vec![give(7, 45, "RN Bo")]), &now()),
        "stat · given 07:45 RN Bo, before it was charted 08:00 Dr Ann Lee"
    );
}

/// A struck give leaves the standing line ("not yet given" again) and reads as its own line: who
/// struck it, when, and why when one was given. A standing give beside it still chains.
#[test]
fn a_struck_give_reads_who_struck_it_and_why() {
    let struck = |why: Option<&str>| GiveIn {
        struck: Some((Some("RN Di".into()), at(8, 30), why.map(Into::into))),
        ..give(8, 20, "RN Bo")
    };
    let o = order(vec![struck(Some("wrong patient's chart"))]);
    assert_eq!(
        stat_words(&o, &now()),
        "stat · ordered 08:00 Dr Ann Lee · not yet given"
    );
    assert_eq!(
        struck_words(&o, &now()),
        ["given 08:20 RN Bo — entered in error by RN Di, 08:30 — wrong patient's chart"]
    );
    let o = order(vec![give(8, 50, "RN Cy"), struck(None)]);
    assert_eq!(
        stat_words(&o, &now()),
        "stat · ordered 08:00 Dr Ann Lee · given 08:50 RN Cy"
    );
    assert_eq!(
        struck_words(&o, &now()),
        ["given 08:20 RN Bo — entered in error by RN Di, 08:30"]
    );
}

/// A name that cannot be read as a person's is said so (2b's rule), for each of the three.
#[test]
fn a_signer_not_known_is_said() {
    let o = StatWordsIn {
        ordered_at: at(8, 0),
        orderer: None,
        gives: vec![
            GiveIn {
                given_at: at(8, 10),
                giver: None,
                struck: Some((None, at(8, 15), None)),
            },
            GiveIn {
                giver: None,
                ..give(8, 20, "x")
            },
        ],
    };
    assert_eq!(
        stat_words(&o, &now()),
        "stat · ordered 08:00 signer not known · given 08:20 signer not known"
    );
    assert_eq!(
        struck_words(&o, &now()),
        ["given 08:10 signer not known — entered in error by signer not known, 08:15"]
    );
}

/// The time given is stated to the minute (the field pre-fills `now` with seconds 00), so a give
/// in the order's own minute is NOT before it: an order charted 08:00:41 and given, untouched,
/// at 08:00 reads in time order, never "before it was charted" (principle 4: a precise untruth).
#[test]
fn a_give_in_the_orders_own_minute_is_not_before_it() {
    let o = StatWordsIn {
        ordered_at: at(8, 0) + chrono::TimeDelta::seconds(41),
        ..order(vec![give(8, 0, "RN Bo")])
    };
    assert_eq!(
        stat_words(&o, &now()),
        "stat · ordered 08:00:41 Dr Ann Lee · given 08:00 RN Bo"
    );
}

/// Two gives before the order say "before it was charted" once, on the later one (where the
/// order sits in time); a give after the order chains after it.
#[test]
fn before_it_was_charted_is_said_once() {
    let two_before = order(vec![give(7, 45, "RN Cy"), give(7, 30, "RN Bo")]);
    assert_eq!(
        stat_words(&two_before, &now()),
        "stat · given 07:30 RN Bo · given 07:45 RN Cy, before it was charted 08:00 Dr Ann Lee"
    );
    let mixed = order(vec![give(8, 20, "RN Cy"), give(7, 45, "RN Bo")]);
    assert_eq!(
        stat_words(&mixed, &now()),
        "stat · given 07:45 RN Bo, before it was charted 08:00 Dr Ann Lee · given 08:20 RN Cy"
    );
}
