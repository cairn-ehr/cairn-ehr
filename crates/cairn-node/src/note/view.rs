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

/// Who typed a signed part, as its contributor set says (sign-in slice 3b-1, ADR-0088 decision
/// 15). A part the signer only ATTESTED must never read like their own authored note.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PartWriter {
    /// No `drafted` contributor, and the signer did not merely attest: nothing says anyone but
    /// the grade's own author typed it (3a's notes, a signer's own part).
    TheSigner,
    /// A `drafted` contributor this node can name: their display name. A CLAIM, never verified
    /// (`drafted` is a contributory role the floor does not authenticate): render it through
    /// [`byline_words`], which marks it `?Name` and "unverified", never as a bare name.
    Drafted(String),
    /// A `drafted` contributor whose key this node cannot name (`cairn_note_signer_name` NULL).
    DraftedUnnamed,
    /// The signer ATTESTED and no writer is recorded: who typed it is not known (principle 4).
    NotKnown,
}

/// The part's writer, read from its event: `drafted` — it carries a `drafted` contributor;
/// `drafted_by` — that key's display name when this node can name it; `signer_attested` — the
/// signer's own contributor role is `attested`, not `authored`. Pure.
pub fn part_writer(drafted: bool, drafted_by: Option<String>, signer_attested: bool) -> PartWriter {
    match drafted_by {
        _ if !drafted && signer_attested => PartWriter::NotKnown,
        _ if !drafted => PartWriter::TheSigner,
        Some(name) => PartWriter::Drafted(name),
        None => PartWriter::DraftedUnnamed,
    }
}

/// A note's byline as shown, and as a screen reader should say it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Byline {
    /// "written by ?Nurse A · signed by Dr B": the `?` is text, never a glyph alone (design §3.7).
    pub shown: String,
    /// "written by unverified Nurse A · signed by Dr B": VoiceOver skips a bare `?` (#973).
    pub spoken: String,
}

/// The note's byline: [`grade_words`], headed by who typed it when that is not simply the
/// signer — "written by ?Nurse A", "written by someone not named here" or "writer not known".
/// The drafted name is a claim this node cannot verify (the writer did not sign), so it carries
/// the `?` (spoken "unverified"). Pure.
pub fn byline_words(grade: &NoteGrade, writer: &PartWriter) -> Byline {
    Byline {
        shown: byline_marked(grade, writer, "?"),
        spoken: byline_marked(grade, writer, "unverified "),
    }
}

/// [`byline_words`] with `unverified` as the mark before a drafted name.
fn byline_marked(grade: &NoteGrade, writer: &PartWriter, unverified: &str) -> String {
    let who = match writer {
        PartWriter::TheSigner => return grade_words(grade),
        PartWriter::Drafted(name) => format!("written by {unverified}{name}"),
        PartWriter::DraftedUnnamed => "written by someone not named here".to_string(),
        PartWriter::NotKnown => "writer not known".to_string(),
    };
    let graded = match grade {
        // The device grade's own words, "writer not known", would contradict a drafted writer
        // (and repeat themselves beside an unknown one): no person answers for the note.
        NoteGrade::WriterNotKnown if *writer == PartWriter::NotKnown => return who,
        NoteGrade::WriterNotKnown => "no person signed for it".to_string(),
        _ => grade_words(grade),
    };
    format!("{who} · {graded}")
}

/// When the note applies, in words, in time zone `tz` (the window passes `chrono::Local`):
/// `"03:00 (stated)"` when the writer stated a time, else `"no stated time · writing began
/// 03:12"` (design §4.3) — never a stated-looking time the writer did not give (principle 4).
/// A time on another day than `today` (in `tz`) carries its date. Pure: the caller reads the
/// clock once, so every row of one pane is dated against the same day.
pub fn time_words<Tz: TimeZone>(
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
/// date in another year must not read as that month of this year. Public so the window words a
/// draft's start and last keystroke by the same rule as a note's time.
pub fn instant<Tz: TimeZone>(t: DateTime<Utc>, tz: &Tz, today: NaiveDate) -> String
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
    fn a_drafted_name_heads_the_byline_unverified() {
        let signed = NoteGrade::Signed { by: "Dr B".into() };
        let b = byline_words(&signed, &PartWriter::Drafted("Nurse A".into()));
        assert_eq!(b.shown, "written by ?Nurse A · signed by Dr B");
        assert_eq!(b.spoken, "written by unverified Nurse A · signed by Dr B");
        for grade in [signed, NoteGrade::Claimed, NoteGrade::WriterNotKnown] {
            let own = byline_words(&grade, &PartWriter::TheSigner);
            assert_eq!(own.shown, grade_words(&grade), "{grade:?}");
            assert_eq!(own.spoken, own.shown, "{grade:?}");
        }
    }

    /// I2: a part the signer only ATTESTED never reads like their own authored note — whether
    /// nobody is recorded as its writer, or this node cannot name the writer's key.
    #[test]
    fn an_attested_part_never_reads_as_the_signers_own() {
        let signed = NoteGrade::Signed { by: "Dr B".into() };
        let cases = [
            (PartWriter::NotKnown, "writer not known · signed by Dr B"),
            (
                PartWriter::DraftedUnnamed,
                "written by someone not named here · signed by Dr B",
            ),
        ];
        for (writer, words) in cases {
            let b = byline_words(&signed, &writer);
            assert_eq!(b.shown, words, "{writer:?}");
            assert_eq!(b.spoken, words, "nothing to mark unverified: {writer:?}");
        }
    }

    /// Every grade beside every writer reads true: the device grade's "writer not known" never
    /// sits beside a drafted writer, and is never said twice.
    #[test]
    fn every_grade_and_writer_reads_true() {
        let a = || PartWriter::Drafted("Nurse A".into());
        let cases = [
            (
                NoteGrade::Claimed,
                a(),
                "written by ?Nurse A · authorship claimed, not verified here",
            ),
            (
                NoteGrade::Claimed,
                PartWriter::DraftedUnnamed,
                "written by someone not named here · authorship claimed, not verified here",
            ),
            (
                NoteGrade::Claimed,
                PartWriter::NotKnown,
                "writer not known · authorship claimed, not verified here",
            ),
            (
                NoteGrade::WriterNotKnown,
                a(),
                "written by ?Nurse A · no person signed for it",
            ),
            (
                NoteGrade::WriterNotKnown,
                PartWriter::DraftedUnnamed,
                "written by someone not named here · no person signed for it",
            ),
            (
                NoteGrade::WriterNotKnown,
                PartWriter::NotKnown,
                "writer not known",
            ),
        ];
        for (grade, writer, words) in cases {
            assert_eq!(
                byline_words(&grade, &writer).shown,
                words,
                "{grade:?} {writer:?}"
            );
        }
    }

    #[test]
    fn the_writer_is_read_from_the_contributor_set() {
        assert_eq!(part_writer(false, None, false), PartWriter::TheSigner);
        assert_eq!(part_writer(false, None, true), PartWriter::NotKnown);
        assert_eq!(
            part_writer(true, Some("Nurse A".into()), true),
            PartWriter::Drafted("Nurse A".into())
        );
        assert_eq!(part_writer(true, None, true), PartWriter::DraftedUnnamed);
    }

    #[test]
    fn a_stated_time_reads_as_stated() {
        let t = utc("2026-11-02T17:00:00Z");
        let began = utc("2026-11-02T17:12:00Z");
        assert_eq!(
            time_words("stated", t, began, &bne(), nov(3)),
            "03:00 (stated)"
        );
    }

    #[test]
    fn no_stated_time_reads_as_when_writing_began() {
        let began = utc("2026-11-02T17:12:00Z");
        assert_eq!(
            time_words("recording-started", began, began, &bne(), nov(3)),
            "no stated time · writing began 03:12"
        );
    }

    #[test]
    fn another_day_or_year_carries_its_date() {
        let t = utc("2026-11-02T17:00:00Z");
        assert_eq!(
            time_words("stated", t, t, &bne(), nov(4)),
            "3 Nov 03:00 (stated)"
        );
        let last_year = utc("2025-11-02T17:00:00Z");
        assert_eq!(
            time_words("stated", last_year, last_year, &bne(), nov(3)),
            "3 Nov 2025 03:00 (stated)"
        );
    }
}
