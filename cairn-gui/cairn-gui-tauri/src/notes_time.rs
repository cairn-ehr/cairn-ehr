//! How the note pane reads and writes the clinician's STATED time (`notes.rs`'s child module,
//! split out to keep that file small). A `datetime-local` field holds a wall-clock time with no
//! zone; these pure functions turn it into an instant in this machine's zone and back, refuse a
//! wall time the clock never showed, and refuse at Sign a year the wire cannot carry.
use chrono::{DateTime, LocalResult, NaiveDateTime, TimeZone, Utc};

/// Whether stated time `t` (typed as `s`) has a year the record can hold, else the sentence. Pure.
///
/// The wire pin (db/001 `cairn_t_effective`) takes a four-digit year and PostgreSQL has no year
/// 0, so a signed time must fall in UTC years 1..=9999. `datetime-local` accepts years up to
/// 275760, and a slip like "20266" is easy to type: refused here, before the passphrase, rather
/// than by the floor after it (final review M2). Asked at SIGN only — a save keeps such a time,
/// so the text is never left unsaved for a typo in the year.
pub(crate) fn year_the_record_holds(s: &str, t: DateTime<Utc>) -> Result<(), String> {
    use chrono::Datelike;
    if (1..=9999).contains(&t.year()) {
        return Ok(());
    }
    Err(format!(
        "the time it happened, {s}, has a year the record cannot hold (years 1 to 9999) — \
         check the year you typed"
    ))
}

/// `t` as a `datetime-local` value in `tz`: minutes only when that is exact, else with its seconds
/// (and any fraction), so a reopened draft sends back the instant it holds, never one rounded to
/// the minute. Both forms are valid field values and both parse in [`stated_time_in`]. Pure.
pub(crate) fn datetime_local<Tz: TimeZone>(t: DateTime<Utc>, tz: &Tz) -> String
where
    Tz::Offset: std::fmt::Display,
{
    use chrono::Timelike;
    let format = if t.second() == 0 && t.nanosecond() == 0 {
        "%Y-%m-%dT%H:%M"
    } else {
        "%Y-%m-%dT%H:%M:%S%.f"
    };
    t.with_timezone(tz).format(format).to_string()
}

/// The webview's stated-time field: absent or empty is "not stated" (principle 4), else a time.
pub(crate) fn parse_stated(stated_time: Option<&str>) -> Result<Option<DateTime<Utc>>, String> {
    match stated_time {
        None | Some("") => Ok(None),
        Some(s) => stated_time_from_local(s).map(Some),
    }
}

/// A `datetime-local` value (`"2026-11-03T03:00"`, no zone) read in this machine's zone — the
/// zone the clinician reads the wall clock in.
pub fn stated_time_from_local(s: &str) -> Result<DateTime<Utc>, String> {
    stated_time_in(s, &chrono::Local)
}

/// [`stated_time_from_local`] in zone `tz`, so the rule is testable without the machine's zone.
pub(crate) fn stated_time_in<Tz: TimeZone>(s: &str, tz: &Tz) -> Result<DateTime<Utc>, String> {
    // With or without seconds: a browser sends them when the field's step asks for them.
    let parse = |v: &str| {
        NaiveDateTime::parse_from_str(v, "%Y-%m-%dT%H:%M:%S%.f")
            .or_else(|_| NaiveDateTime::parse_from_str(v, "%Y-%m-%dT%H:%M"))
    };
    // chrono reads a year of five or more digits only with a sign, and the field sends none: a
    // typo like "20266" must still SAVE (the sign refuses it, `year_the_record_holds`).
    let naive = parse(s)
        .or_else(|_| parse(&format!("+{s}")))
        .map_err(|_| format!("not a time: {s}"))?;
    pick_local(s, tz.from_local_datetime(&naive))
}

/// The instant a wall-clock time `s` means, given everything the zone answers for it. Pure.
///
/// A time that happened twice (clocks went back) takes the FIRST time the clock showed it —
/// a choice, not a certainty, recorded in ADR-0087 "What is not guaranteed" (#957 asks whether
/// it should be flagged instead). A time that never happened (clocks went forward past it) is
/// refused, with a sentence naming it — any instant chosen for it would be a time the clinician
/// never saw on a clock.
pub(crate) fn pick_local<Tz: TimeZone>(
    s: &str,
    answer: LocalResult<DateTime<Tz>>,
) -> Result<DateTime<Utc>, String> {
    match answer.earliest() {
        Some(t) => Ok(t.with_timezone(&Utc)),
        None => Err(format!(
            "{s} never happened on this machine's clock — the clocks moved forward past it; \
             choose a time before or after the change"
        )),
    }
}
