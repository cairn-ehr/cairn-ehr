//! The words a note is shown with: who wrote it, and when it applies. Pure, so the window's
//! wording is pinned here without a database or a webview.
use cairn_event::contributor::AuthorshipConfidence;
use chrono::{DateTime, NaiveDate, TimeZone, Utc};

/// What a signing key reads as when this node cannot name its holder.
pub const UNKNOWN_SIGNER: &str = "a key this node does not know";

/// A note's authorship, as the clinician reads it (ADR-0008: a grade, not a gate).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NoteGrade {
    /// A responsibility-bearing human author this node authenticated; `by` names them.
    Signed { by: String },
    /// Someone claims authorship, but this node cannot verify the claim.
    Claimed,
    /// No one took authorship: the node recorded it alone.
    WriterNotKnown,
}

/// Turn the shared authorship grade into the note's. `signer_name` is who the signing key
/// stands for (`cairn_note_signer_name`), `None` when the node does not know the key.
///
/// The match is EXHAUSTIVE, with no `_` arm, on purpose: a fourth `AuthorshipConfidence` must
/// break this build so someone decides how it reads, rather than a wildcard silently showing it
/// as one of these three (#412; `AuthorshipConfidence`'s docs give the full argument).
pub fn note_grade(confidence: AuthorshipConfidence, signer_name: Option<String>) -> NoteGrade {
    match confidence {
        AuthorshipConfidence::Attested => NoteGrade::Signed {
            by: signer_name.unwrap_or_else(|| UNKNOWN_SIGNER.into()),
        },
        AuthorshipConfidence::Unverified => NoteGrade::Claimed,
        AuthorshipConfidence::Device => NoteGrade::WriterNotKnown,
    }
}

/// The grade in words, as the note's byline.
pub fn grade_words(grade: &NoteGrade) -> String {
    match grade {
        NoteGrade::Signed { by } => format!("signed by {by}"),
        NoteGrade::Claimed => "authorship claimed, not verified here".into(),
        NoteGrade::WriterNotKnown => "writer not known".into(),
    }
}

/// When the note applies, in words, in time zone `tz` (the window passes `chrono::Local`):
/// `"03:00 (stated)"` when the writer stated a time, else `"no stated time · writing began
/// 03:12"` (design §4.3) — never a stated-looking time the writer did not give (principle 4).
/// A time on another day than today (in `tz`) carries its date.
pub fn time_words<Tz: TimeZone>(
    basis: &str,
    t_effective: DateTime<Utc>,
    recording_started: DateTime<Utc>,
    tz: &Tz,
) -> String
where
    Tz::Offset: std::fmt::Display,
{
    let today = Utc::now().with_timezone(tz).date_naive();
    time_words_on(basis, t_effective, recording_started, tz, today)
}

/// [`time_words`] against a given `today`, so the date rule is testable.
fn time_words_on<Tz: TimeZone>(
    basis: &str,
    t_effective: DateTime<Utc>,
    recording_started: DateTime<Utc>,
    tz: &Tz,
    today: NaiveDate,
) -> String
where
    Tz::Offset: std::fmt::Display,
{
    // Only "stated" claims a stated time; any other basis reads as not stated.
    if basis == "stated" {
        format!("{} (stated)", instant(t_effective, tz, today))
    } else {
        format!(
            "no stated time · writing began {}",
            instant(recording_started, tz, today)
        )
    }
}

/// `HH:MM` in `tz`, prefixed `"2 Nov "` on another day, `"2 Nov 2025 "` in another year — a
/// date in another year must not read as that month of this year.
fn instant<Tz: TimeZone>(t: DateTime<Utc>, tz: &Tz, today: NaiveDate) -> String
where
    Tz::Offset: std::fmt::Display,
{
    use chrono::Datelike;
    let t = t.with_timezone(tz);
    let day = if t.date_naive() == today {
        ""
    } else if t.year() != today.year() {
        "%-d %b %Y "
    } else {
        "%-d %b "
    };
    t.format(&format!("{day}%H:%M")).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::FixedOffset;

    fn utc(s: &str) -> DateTime<Utc> {
        s.parse().unwrap()
    }

    /// Brisbane, UTC+10, no daylight saving: 17:00Z on the 2nd is 03:00 on the 3rd.
    fn bne() -> FixedOffset {
        FixedOffset::east_opt(10 * 3600).unwrap()
    }

    fn nov(d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 11, d).unwrap()
    }

    #[test]
    fn every_authorship_grade_has_its_words() {
        let signed = note_grade(AuthorshipConfidence::Attested, Some("Dr Ada".into()));
        assert_eq!(grade_words(&signed), "signed by Dr Ada");
        let unknown_key = note_grade(AuthorshipConfidence::Attested, None);
        assert_eq!(
            grade_words(&unknown_key),
            "signed by a key this node does not know"
        );
        let claimed = note_grade(AuthorshipConfidence::Unverified, Some("Dr Ada".into()));
        assert_eq!(
            claimed,
            NoteGrade::Claimed,
            "a claim is never named as signed"
        );
        assert_eq!(
            grade_words(&claimed),
            "authorship claimed, not verified here"
        );
        let device = note_grade(AuthorshipConfidence::Device, Some("the node".into()));
        assert_eq!(device, NoteGrade::WriterNotKnown);
        assert_eq!(grade_words(&device), "writer not known");
    }

    #[test]
    fn a_stated_time_reads_as_stated() {
        let t = utc("2026-11-02T17:00:00Z");
        let began = utc("2026-11-02T17:12:00Z");
        assert_eq!(
            time_words_on("stated", t, began, &bne(), nov(3)),
            "03:00 (stated)"
        );
    }

    #[test]
    fn no_stated_time_reads_as_when_writing_began() {
        let began = utc("2026-11-02T17:12:00Z");
        assert_eq!(
            time_words_on("recording-started", began, began, &bne(), nov(3)),
            "no stated time · writing began 03:12"
        );
    }

    #[test]
    fn another_day_or_year_carries_its_date() {
        let t = utc("2026-11-02T17:00:00Z");
        assert_eq!(
            time_words_on("stated", t, t, &bne(), nov(4)),
            "3 Nov 03:00 (stated)"
        );
        let last_year = utc("2025-11-02T17:00:00Z");
        assert_eq!(
            time_words_on("stated", last_year, last_year, &bne(), nov(3)),
            "3 Nov 2025 03:00 (stated)"
        );
    }

    #[test]
    fn now_reads_without_a_date() {
        let now = Utc::now();
        assert_eq!(
            time_words("stated", now, now, &bne()),
            format!("{} (stated)", now.with_timezone(&bne()).format("%H:%M"))
        );
    }
}
