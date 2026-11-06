//! Tests for `view.rs`: every word a note is shown with, pinned without a database.
use super::*;
use crate::note::vouch::fixture::{part, repudiated, student, vouch, who};
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
    let asserted = note_grade(AuthorshipConfidence::Asserted, Some("the node".into()));
    assert_eq!(asserted, NoteGrade::WentInUnsigned);
    assert_eq!(grade_words(&asserted), "went in unsigned");
}

#[test]
fn a_drafted_name_heads_the_byline_unverified() {
    let signed = NoteGrade::Signed { by: "Dr B".into() };
    let b = byline_words(&signed, &PartWriter::Drafted("Nurse A".into()));
    assert_eq!(b.shown, "written by ?Nurse A · signed by Dr B");
    assert_eq!(b.spoken, "written by unverified Nurse A · signed by Dr B");
    for grade in [
        signed,
        NoteGrade::Claimed,
        NoteGrade::WriterNotKnown,
        NoteGrade::WentInUnsigned,
    ] {
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
            NoteGrade::WentInUnsigned,
            a(),
            "written by ?Nurse A · went in unsigned",
        ),
        (
            NoteGrade::WentInUnsigned,
            PartWriter::DraftedUnnamed,
            "written by someone not named here · went in unsigned",
        ),
        (
            NoteGrade::WentInUnsigned,
            PartWriter::TheSigner,
            "went in unsigned",
        ),
        (
            NoteGrade::WentInUnsigned,
            PartWriter::NotKnown,
            "writer not known · went in unsigned",
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
fn a_went_in_unsigned_byline_marks_the_name_in_speech() {
    let b = byline_words(
        &NoteGrade::WentInUnsigned,
        &PartWriter::Drafted("Nurse A".into()),
    );
    assert_eq!(b.shown, "written by ?Nurse A · went in unsigned");
    assert_eq!(b.spoken, "written by unverified Nurse A · went in unsigned");
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
fn a_student_part_reads_not_co_signed() {
    let p = part(1, Some(student("s1", "Sam")));
    assert_eq!(
        part_byline(&p).shown,
        "signed by Sam (student) — not co-signed"
    );
    // A sign-off that only ATTESTS (never the node's plan for a signed part) is no co-signature.
    let mut attested = p.clone();
    attested.vouches.push(vouch(
        9,
        VouchAct::SignOff,
        VouchRole::Attested,
        who("z1", "Dr Zed"),
    ));
    assert_eq!(
        part_byline(&attested).shown,
        "signed by Sam (student) — not co-signed · signed off by Dr Zed"
    );
}

#[test]
fn a_co_signed_student_part() {
    let mut p = part(1, Some(student("s1", "Sam")));
    p.vouches.push(vouch(
        9,
        VouchAct::SignOff,
        VouchRole::CoSigned,
        who("z1", "Dr Zed"),
    ));
    assert_eq!(
        part_byline(&p).shown,
        "signed by Sam (student) · co-signed by Dr Zed"
    );
}

/// Design §3.4: the supervisor's sign-off co-signs a student's part. A co-sign by the student
/// themselves, or by another student (each admitted at a peer's remote door), leaves the mark.
#[test]
fn a_students_own_co_sign_does_not_clear_not_co_signed() {
    let sam = student("s1", "Sam");
    let mut p = part(1, Some(sam.clone()));
    p.vouches
        .push(vouch(9, VouchAct::SignOff, VouchRole::CoSigned, sam));
    assert_eq!(
        part_byline(&p).shown,
        "signed by Sam (student) — not co-signed · co-signed by Sam (student)"
    );
}

#[test]
fn another_students_co_sign_does_not_clear_not_co_signed() {
    let mut p = part(1, Some(student("s1", "Sam")));
    p.vouches.push(vouch(
        9,
        VouchAct::SignOff,
        VouchRole::CoSigned,
        student("t1", "Tess"),
    ));
    assert_eq!(
        part_byline(&p).shown,
        "signed by Sam (student) — not co-signed · co-signed by Tess (student)"
    );
}

/// A co-signer whose key this node cannot place (empty actor — e.g. the student's own re-keyed key
/// whose `rotate-key` row has not synced here yet) might be anyone, the student included, so it
/// never clears the mark (ADR-0092 D10/D12).
#[test]
fn an_unplaceable_co_signer_does_not_clear_not_co_signed() {
    let mut p = part(1, Some(student("s1", "Sam")));
    p.vouches.push(vouch(
        9,
        VouchAct::SignOff,
        VouchRole::CoSigned,
        who("", UNKNOWN_SIGNER),
    ));
    assert_eq!(
        part_byline(&p).shown,
        format!("signed by Sam (student) — not co-signed · co-signed by {UNKNOWN_SIGNER}")
    );
}

#[test]
fn an_unconfirmed_signer_is_marked() {
    let mut s = who("a1", "Dr Ada");
    s.unconfirmed = true;
    let mut p = part(1, Some(s.clone()));
    assert_eq!(
        part_byline(&p).shown,
        "signed by Dr Ada (signer unconfirmed)"
    );
    p.signed_by = Some(student("s1", "Sam"));
    p.vouches
        .push(vouch(9, VouchAct::SignOff, VouchRole::CoSigned, s));
    p.writer = PartWriter::Drafted("Nurse A".into());
    let b = part_byline(&p);
    assert_eq!(
        b.shown,
        "written by ?Nurse A · signed by Sam (student) · co-signed by Dr Ada (signer unconfirmed)"
    );
    assert_eq!(
        b.spoken,
        "written by unverified Nurse A · signed by Sam (student) · co-signed by Dr Ada (signer unconfirmed)"
    );
}

#[test]
fn a_vouched_unsigned_part_reads_signed_later() {
    let mut p = part(1, None);
    p.writer = PartWriter::Drafted("Nurse A".into());
    assert_eq!(
        part_byline(&p).shown,
        "written by ?Nurse A · went in unsigned"
    );
    p.vouches.push(vouch(
        9,
        VouchAct::Addendum,
        VouchRole::Attested,
        who("n1", "Nurse A"),
    ));
    p.vouches.push(vouch(
        10,
        VouchAct::SignOff,
        VouchRole::Attested,
        who("z1", "Dr Zed"),
    ));
    assert_eq!(
        part_byline(&p).shown,
        "written by ?Nurse A · went in unsigned · signed later by Nurse A · signed off by Dr Zed"
    );
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

#[test]
fn a_repudiated_signers_byline_says_it_counts_as_unsigned() {
    let mut p = part(1, Some(repudiated("a", "Dr Ada Notewriter")));
    assert_eq!(
        part_byline(&p).shown,
        "signed by Dr Ada Notewriter (key had been repudiated) — counts as unsigned"
    );
    // A colleague's sign-off clears nothing.
    p.vouches.push(vouch(
        10,
        VouchAct::SignOff,
        VouchRole::Attested,
        who("z", "Dr Zed"),
    ));
    assert_eq!(
        part_byline(&p).shown,
        "signed by Dr Ada Notewriter (key had been repudiated) — counts as unsigned · signed off by Dr Zed"
    );
    // The real person, re-keyed, re-signs: the mark goes.
    p.vouches.push(vouch(
        11,
        VouchAct::SignOff,
        VouchRole::Attested,
        who("a", "Dr Ada Notewriter"),
    ));
    assert_eq!(
        part_byline(&p).shown,
        "signed by Dr Ada Notewriter (key had been repudiated) · signed off by Dr Zed · re-signed by Dr Ada Notewriter"
    );
}

#[test]
fn a_repudiated_students_part_counts_as_unsigned_not_uncosigned() {
    let mut s = student("s", "Sam");
    s.repudiated = true;
    assert_eq!(
        part_byline(&part(1, Some(s))).shown,
        "signed by Sam (student) (key had been repudiated) — counts as unsigned"
    );
}

#[test]
fn a_re_sign_by_a_key_since_repudiated_counts_nothing() {
    let mut p = part(1, Some(repudiated("a", "Dr Ada Notewriter")));
    p.vouches.push(vouch(
        10,
        VouchAct::SignOff,
        VouchRole::Attested,
        repudiated("a", "Dr Ada Notewriter"),
    ));
    assert!(part_byline(&p).shown.ends_with(
        " — counts as unsigned · signed off by Dr Ada Notewriter (key had been repudiated)"
    ));
    assert!(!crate::note::vouch::part_is_signed(&p));
}

#[test]
fn a_repudiated_co_signer_does_not_clear_not_co_signed() {
    let mut p = part(1, Some(student("s", "Sam")));
    p.vouches.push(vouch(
        10,
        VouchAct::SignOff,
        VouchRole::CoSigned,
        repudiated("z", "Dr Zed"),
    ));
    assert_eq!(
        part_byline(&p).shown,
        "signed by Sam (student) — not co-signed · co-signed by Dr Zed (key had been repudiated)"
    );
}
