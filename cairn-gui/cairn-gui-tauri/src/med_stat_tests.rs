//! Pure tests for `med_stat.rs`: which line is a stat ORDER, and the time given as typed. No
//! database; the DB-gated walks are `med_stat_db_tests.rs`.
use super::*;
use cairn_node::medication::line::read_directions;
use chrono::TimeZone;

/// "Now" for the time tests: a fixed instant, so a test never depends on the wall clock.
fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 14, 10, 0, 0).unwrap()
}

/// `t` as the field would send it, in this machine's zone (the zone `given_time` reads it in).
fn typed(t: DateTime<Utc>) -> String {
    t.with_timezone(&chrono::Local)
        .format("%Y-%m-%dT%H:%M")
        .to_string()
}

#[test]
fn ceftriaxone_1g_iv_stat_is_a_stat_order() {
    let d = read_directions("1g iv stat");
    assert!(is_stat_order(ChartedAs::StartedHere, &d), "{d:?}");
}

/// A stat dose then a regimen is a regimen with a first dose, not a one-off order; a prn stat is
/// "when needed", never once. Neither may leave the list at its first give.
#[test]
fn stat_then_daily_and_prn_stat_are_not() {
    for sig in ["1g iv stat then daily", "1g iv prn stat", "1g iv daily"] {
        let d = read_directions(sig);
        assert!(!is_stat_order(ChartedAs::StartedHere, &d), "{sig}: {d:?}");
    }
}

/// "Already taking" (or nobody said) records what the patient takes; it orders nothing to give.
#[test]
fn an_already_taking_stat_is_not() {
    let d = read_directions("1g iv stat");
    assert!(!is_stat_order(ChartedAs::PreExisting, &d));
    assert!(!is_stat_order(ChartedAs::Unknown, &d));
}

#[test]
fn given_time_defaults_to_now() {
    assert_eq!(given_time(None, now()), Ok(now()));
    // An empty field is "not typed", as the note pane reads it (`notes_time::parse_stated`).
    assert_eq!(given_time(Some(""), now()), Ok(now()));
}

#[test]
fn a_later_time_is_refused() {
    let later = typed(now() + chrono::Duration::minutes(5));
    assert_eq!(
        given_time(Some(&later), now()),
        Err(GIVEN_LATER_THAN_NOW.to_string())
    );
}

/// An earlier time is kept as typed — even one before the order was charted (§10 Q3: shown,
/// never refused).
#[test]
fn an_earlier_time_is_kept() {
    let earlier = now() - chrono::Duration::minutes(30);
    assert_eq!(given_time(Some(&typed(earlier)), now()), Ok(earlier));
}

/// A date with no hour must never sign as midnight: refused with the parser's own words.
#[test]
fn a_partly_typed_time_is_refused_in_words() {
    for partial in ["2026-11-14", "2026-11-14T", "2026-11-14T09"] {
        let said = given_time(Some(partial), now()).unwrap_err();
        assert_eq!(said, format!("not a time: {partial}"));
    }
}

/// What a strike says: "waiting again" only when the struck given had stopped the order.
#[test]
fn a_strike_says_waiting_again_only_when_the_given_stopped_the_order() {
    assert_eq!(
        struck_outcome(true),
        "Entered in error: the stat dose is waiting to be given again."
    );
    assert_eq!(struck_outcome(false), "Entered in error.");
}

/// A give or strike that LANDED answers Ok even when the pane could not be read again: an error
/// would offer a retry, and a retry gives the dose twice.
#[test]
fn a_landed_act_whose_reread_failed_still_answers_ok() {
    let said = landed("Given.", Err("the lock was lost".into()));
    assert!(said.pane.is_none());
    assert!(
        said.outcome
            .starts_with("Signed — but the medication list could not be read again")
            && said.outcome.contains("the lock was lost"),
        "{}",
        said.outcome
    );
}
