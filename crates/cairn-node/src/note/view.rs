//! The words a note is shown with: who wrote it, and when it applies. Pure, so the window's
//! wording is pinned here without a database or a webview.
use cairn_event::contributor::AuthorshipConfidence;
use cairn_event::note_attestation::{VouchAct, VouchRole};
use chrono::{DateTime, NaiveDate, TimeZone, Utc};

use super::read::NoteRow;
use super::vouch::same_actor;
use super::vouch_read::{PartVouch, Signer};

/// What a signing key reads as when this node cannot name its holder.
pub const UNKNOWN_SIGNER: &str = "a key this node does not know";

/// A note's authorship, as the clinician reads it (ADR-0008: a grade, not a gate).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NoteGrade {
    /// A responsibility-bearing human author this node authenticated; `by` names them.
    Signed { by: String },
    /// Someone claims authorship, but this node cannot verify the claim.
    Claimed,
    /// Went into the record unsigned, with a named human writer (the quarantine commit): no one
    /// answers for it yet; a later vouch can upgrade it.
    WentInUnsigned,
    /// No one took authorship: the node recorded it alone.
    WriterNotKnown,
}

/// Turn the shared authorship grade into the note's. `signer_name` is who the signing key
/// stands for (`cairn_note_signer_name`), `None` when the node does not know the key.
///
/// The match is EXHAUSTIVE, with no `_` arm, on purpose: a new `AuthorshipConfidence` must
/// break this build so someone decides how it reads, rather than a wildcard silently showing it
/// as one of these four (#412; `AuthorshipConfidence`'s docs give the full argument).
pub fn note_grade(confidence: AuthorshipConfidence, signer_name: Option<String>) -> NoteGrade {
    match confidence {
        AuthorshipConfidence::Attested => NoteGrade::Signed {
            by: signer_name.unwrap_or_else(|| UNKNOWN_SIGNER.into()),
        },
        AuthorshipConfidence::Unverified => NoteGrade::Claimed,
        AuthorshipConfidence::Asserted => NoteGrade::WentInUnsigned,
        AuthorshipConfidence::Device => NoteGrade::WriterNotKnown,
    }
}

/// The grade in words, as the note's byline.
pub fn grade_words(grade: &NoteGrade) -> String {
    match grade {
        NoteGrade::Signed { by } => format!("signed by {by}"),
        NoteGrade::Claimed => "authorship claimed, not verified here".into(),
        NoteGrade::WentInUnsigned => "went in unsigned".into(),
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

/// A signer's name as the chart shows it: their name, then `" (student)"` for a student and
/// `" (signer unconfirmed)"` while their key's enrolment is unconfirmed or doubtful. Pure.
pub fn signer_words(s: &Signer) -> String {
    let student = if s.student { " (student)" } else { "" };
    let unconfirmed = if s.unconfirmed {
        " (signer unconfirmed)"
    } else {
        ""
    };
    format!("{}{student}{unconfirmed}", s.name)
}

/// What one vouch adds to a byline, `" · co-signed by Dr Zed"`. Exhaustive: a new act or role
/// must decide how it reads. (`Addendum` with `CoSigned` is never stored: the floor refuses it
/// and the projection skips it.)
fn vouch_words(v: &PartVouch) -> String {
    let how = match (v.act, v.role) {
        (VouchAct::Addendum, _) => "signed later",
        (VouchAct::SignOff, VouchRole::Attested) => "signed off",
        (VouchAct::SignOff, VouchRole::CoSigned) => "co-signed",
    };
    format!(" · {how} by {}", signer_words(&v.by))
}

/// A part's byline with who signed it and who vouched for it since (slice 3c-2):
/// [`byline_words`] with the signer's name as [`signer_words`] gives it, `" — not co-signed"`
/// straight after a student's name until a non-student other than them co-signs the part, then
/// each vouch in order.
/// "signed by Sam (student) — not co-signed"; "signed by Sam (student) · co-signed by Dr Zed".
/// The spoken form marks a drafted name "unverified", as [`byline_words`] does. Pure.
pub fn part_byline(p: &NoteRow) -> Byline {
    Byline {
        shown: part_byline_marked(p, "?"),
        spoken: part_byline_marked(p, "unverified "),
    }
}

/// [`part_byline`] with `unverified` as the mark before a drafted name.
fn part_byline_marked(p: &NoteRow, unverified: &str) -> String {
    let grade = match &p.signed_by {
        Some(s) => {
            // Only a supervisor's co-sign clears the mark (design §3.4): not the student's own, nor
            // another student's, either of which a peer's remote door may have admitted.
            let co_signed = p.vouches.iter().any(|v| {
                v.role == VouchRole::CoSigned && !v.by.student && !same_actor(&v.by.actor, &s.actor)
            });
            let pending = if s.student && !co_signed {
                " — not co-signed"
            } else {
                ""
            };
            NoteGrade::Signed {
                by: format!("{}{pending}", signer_words(s)),
            }
        }
        // `signed_by` is `Some` exactly when the grade is `Signed`: the grade's own words stand.
        None => p.grade.clone(),
    };
    let mut words = byline_marked(&grade, &p.writer, unverified);
    for v in &p.vouches {
        words.push_str(&vouch_words(v));
    }
    words
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
#[path = "view_tests.rs"]
mod tests;
