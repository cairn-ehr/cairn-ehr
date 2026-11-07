//! Tests for `cosign.rs` (sign-in slice 3c-3a): who a co-sign is owed to, who clears it, and how
//! the byline and the sign-off offer say so. The supervision annotations are set on the fixtures
//! directly, as `supervision::annotate` would have stamped them.
use super::*;
use crate::note::view::part_byline;
use crate::note::vouch::fixture::{part, repudiated, student, vouch, who};
use crate::note::vouch::needs_sign_off;
use cairn_event::note_attestation::{VouchAct, VouchRole};

/// Dr Pro's part, signed by them, under a co-signing requirement at the part's time, with Dr Sup
/// as their supervisor now.
fn probationer() -> NoteRow {
    let mut p = part(1, Some(who("p1", "Dr Pro")));
    p.needs_cosigning = true;
    p.writer_supervisors = vec!["sv".into()];
    p
}

/// A vouch on Dr Pro's part, stamped as `annotate` would: `by_supervisor` at its time, and the
/// writer had a supervisor then.
fn supervised_vouch(n: u128, role: VouchRole, by: Signer, by_supervisor: bool) -> PartVouch {
    let mut v = vouch(n, VouchAct::SignOff, role, by);
    v.by_supervisor = by_supervisor;
    v.writer_supervised = true;
    v
}

#[test]
fn a_probationers_part_reads_not_co_signed() {
    let p = probationer();
    assert!(cosign_required(&p));
    assert!(awaits_cosign(&p));
    assert_eq!(part_byline(&p).shown, "signed by Dr Pro — not co-signed");
    // With no supervisor recorded now, the mark says so.
    let mut alone = p.clone();
    alone.writer_supervisors.clear();
    assert_eq!(
        part_byline(&alone).shown,
        "signed by Dr Pro — not co-signed (no supervisor recorded)"
    );
    // Without the requirement, a non-student's own part owes nobody a co-sign (today's rule).
    let mut free = p;
    free.needs_cosigning = false;
    assert!(!cosign_required(&free));
    assert_eq!(part_byline(&free).shown, "signed by Dr Pro");
}

/// Review Focus 2: while the writer has a supervisor, a colleague's co-sign does not clear it.
#[test]
fn a_non_supervisors_co_sign_does_not_clear_it() {
    let mut p = probationer();
    let v = supervised_vouch(9, VouchRole::CoSigned, who("z1", "Dr Zed"), false);
    assert!(!clears_cosign(&p, &v));
    p.vouches.push(v);
    assert!(awaits_cosign(&p));
    assert_eq!(
        part_byline(&p).shown,
        "signed by Dr Pro — not co-signed · co-signed by Dr Zed"
    );
}

/// Design §3.2 Q4: a supervisor's vouch clears it whatever its role, an attested one included.
#[test]
fn a_supervisors_attested_sign_off_clears_it() {
    let mut p = probationer();
    p.vouches.push(supervised_vouch(
        9,
        VouchRole::Attested,
        who("sv", "Dr Sup"),
        true,
    ));
    assert!(!awaits_cosign(&p));
    assert_eq!(
        part_byline(&p).shown,
        "signed by Dr Pro · signed off by Dr Sup"
    );
    // But never under a repudiated key, nor by the writer themself.
    let mut dead = probationer();
    let v = supervised_vouch(9, VouchRole::CoSigned, repudiated("sv", "Dr Sup"), true);
    assert!(!clears_cosign(&dead, &v));
    dead.vouches.push(v);
    assert!(awaits_cosign(&dead));
    let own = supervised_vouch(9, VouchRole::CoSigned, who("p1", "Dr Pro"), true);
    assert!(!clears_cosign(&probationer(), &own));
}

/// The fallback (Review Focus 3): with no supervisor at the co-sign, any non-student clinician
/// this node can place, other than the writer, clears it with a co-sign; an attested sign-off
/// does not, nor a student's, nor an unplaceable key's.
#[test]
fn with_no_supervisor_any_clinicians_co_sign_clears_it() {
    let mut p = probationer();
    p.writer_supervisors.clear();
    let co = vouch(
        9,
        VouchAct::SignOff,
        VouchRole::CoSigned,
        who("z1", "Dr Zed"),
    );
    assert!(clears_cosign(&p, &co));
    let attested = vouch(
        9,
        VouchAct::SignOff,
        VouchRole::Attested,
        who("z1", "Dr Zed"),
    );
    assert!(!clears_cosign(&p, &attested));
    let by_student = vouch(
        9,
        VouchAct::SignOff,
        VouchRole::CoSigned,
        student("t1", "Tess"),
    );
    assert!(!clears_cosign(&p, &by_student));
    let unplaced = vouch(9, VouchAct::SignOff, VouchRole::CoSigned, who("", "?"));
    assert!(!clears_cosign(&p, &unplaced));
    p.vouches.push(co);
    assert!(!awaits_cosign(&p));
    assert_eq!(
        part_byline(&p).shown,
        "signed by Dr Pro · co-signed by Dr Zed"
    );
    // A student's part with nothing recorded is today's rule exactly.
    let mut sam = part(2, Some(student("s1", "Sam")));
    assert!(awaits_cosign(&sam));
    sam.vouches.push(vouch(
        9,
        VouchAct::SignOff,
        VouchRole::CoSigned,
        who("z1", "Dr Zed"),
    ));
    assert!(!awaits_cosign(&sam));
}

/// A part that went in unsigned and was signed later by its own writer still owes its co-sign:
/// the mark sits straight after the writer's own vouch, the signature a co-sign countersigns.
#[test]
fn a_part_signed_later_by_its_writer_still_needs_its_co_sign() {
    let mut p = part(1, None);
    p.drafted_actor = Some("s1".into());
    // A colleague's earlier attested vouch is not the writer's signature.
    p.vouches.push(vouch(
        8,
        VouchAct::SignOff,
        VouchRole::Attested,
        who("n1", "Nurse Bo"),
    ));
    p.vouches.push(vouch(
        9,
        VouchAct::Addendum,
        VouchRole::Attested,
        student("s1", "Sam"),
    ));
    assert_eq!(writer_signature(&p).map(|s| s.actor.as_str()), Some("s1"));
    assert!(awaits_cosign(&p));
    assert_eq!(
        part_byline(&p).shown,
        "went in unsigned · signed off by Nurse Bo · signed later by Sam (student) \
         — not co-signed (no supervisor recorded)"
    );
    p.vouches.push(vouch(
        10,
        VouchAct::SignOff,
        VouchRole::CoSigned,
        who("z1", "Dr Zed"),
    ));
    assert!(!awaits_cosign(&p));
    assert_eq!(
        part_byline(&p).shown,
        "went in unsigned · signed off by Nurse Bo · signed later by Sam (student) \
         · co-signed by Dr Zed"
    );
    // A writer's own vouch under a repudiated key is no signature of theirs.
    let mut dead = part(2, None);
    dead.drafted_actor = Some("s1".into());
    dead.vouches.push(vouch(
        9,
        VouchAct::Addendum,
        VouchRole::Attested,
        repudiated("s1", "Sam"),
    ));
    assert!(writer_signature(&dead).is_none());
    assert!(!cosign_required(&dead));
}

/// Design §3.2 Q2: "Sign off this note" stays offered while a mandatory co-sign is missing, even
/// though a non-student (the writer) signed every part.
#[test]
fn the_offer_stays_while_a_mandatory_co_sign_is_missing() {
    let mut parts = vec![probationer()];
    assert!(crate::note::vouch::signed_off_by(&parts).is_some());
    assert!(needs_sign_off(&parts));
    parts[0].vouches.push(supervised_vouch(
        9,
        VouchRole::CoSigned,
        who("z1", "Dr Zed"),
        false,
    ));
    assert!(needs_sign_off(&parts), "a non-supervisor clears nothing");
    parts[0].vouches.push(supervised_vouch(
        10,
        VouchRole::CoSigned,
        who("sv", "Dr Sup"),
        true,
    ));
    assert!(!needs_sign_off(&parts));
}

/// A repudiated, not re-signed signature reads " — counts as unsigned" in the mark's place, never
/// both; once re-signed, the co-sign is owed again.
#[test]
fn a_repudiated_part_reads_counts_as_unsigned_not_both() {
    let mut p = part(1, Some(repudiated("p1", "Dr Pro")));
    p.needs_cosigning = true;
    assert!(!cosign_required(&p));
    assert_eq!(
        part_byline(&p).shown,
        "signed by Dr Pro (key had been repudiated) — counts as unsigned"
    );
    p.vouches.push(vouch(
        9,
        VouchAct::SignOff,
        VouchRole::Attested,
        who("p1", "Dr Pro"),
    ));
    assert!(awaits_cosign(&p));
    assert_eq!(
        part_byline(&p).shown,
        "signed by Dr Pro (key had been repudiated) — not co-signed (no supervisor recorded) \
         · re-signed by Dr Pro"
    );
}
