//! Tests for `vouch.rs`'s pure rules (sign-in slice 3c-2): who signed a note off, what a sign-off
//! vouches and how, and what an addendum covers.
use super::fixture::*;
use super::*;

fn ids(n: &[u128]) -> Vec<Uuid> {
    n.iter().map(|&n| Uuid::from_u128(n)).collect()
}

#[test]
fn a_note_signed_wholly_by_its_author_needs_no_sign_off() {
    let ada = who("a1", "Dr Ada");
    let parts = vec![part(1, Some(ada.clone())), part(2, Some(ada.clone()))];
    assert_eq!(signed_off_by(&parts), Some(&ada));
    assert!(!needs_sign_off(&parts));
    assert!(!needs_sign_off(&parts[..1]), "a one-part note too");
}

#[test]
fn a_students_note_needs_sign_off_until_co_signed() {
    let mut parts = vec![part(1, Some(student("s1", "Sam")))];
    assert!(needs_sign_off(&parts), "a student never signs a note off");
    parts[0].vouches.push(vouch(
        10,
        VouchAct::SignOff,
        VouchRole::CoSigned,
        who("z1", "Dr Zed"),
    ));
    assert_eq!(
        signed_off_by(&parts).map(|s| s.name.as_str()),
        Some("Dr Zed")
    );
    assert!(!needs_sign_off(&parts));
}

#[test]
fn two_clinicians_parts_need_sign_off() {
    let parts = vec![
        part(1, Some(who("a1", "Dr Ada"))),
        part(2, Some(who("b1", "Nurse Bo"))),
    ];
    assert!(needs_sign_off(&parts));
    assert!(needs_sign_off(&[part(1, None)]), "an unsigned part too");
}

/// An unknown key (empty actor) signs nothing off and matches no presser.
#[test]
fn a_key_this_node_does_not_know_matches_no_one() {
    let parts = vec![part(1, Some(who("", "a key this node does not know")))];
    assert!(needs_sign_off(&parts));
    assert_eq!(
        sign_off_vouches(&parts, "").len(),
        1,
        "not even an empty presser"
    );
}

/// The floor co-signs only a part a key it can place as a human's signed, so a part signed by
/// a key this node cannot place is ATTESTED ("signed off by"), never co-signed.
#[test]
fn a_part_signed_by_a_key_this_node_cannot_place_is_attested_not_co_signed() {
    let parts = vec![
        part(1, Some(who("", "a key this node does not know"))),
        part(2, Some(student("s1", "Sam"))),
    ];
    assert_eq!(
        sign_off_vouches(&parts, "z1"),
        vec![
            (Uuid::from_u128(1), VouchRole::Attested),
            (Uuid::from_u128(2), VouchRole::CoSigned),
        ]
    );
}

#[test]
fn sign_off_co_signs_others_signed_parts_attests_unsigned_ones_and_skips_ones_own() {
    let parts = vec![
        part(1, Some(student("s1", "Sam"))),
        part(2, None),
        part(3, Some(who("z1", "Dr Zed"))),
    ];
    assert_eq!(
        sign_off_vouches(&parts, "z1"),
        vec![
            (Uuid::from_u128(1), VouchRole::CoSigned),
            (Uuid::from_u128(2), VouchRole::Attested),
        ]
    );
    // A part the presser already vouched for is theirs too.
    let mut vouched = parts.clone();
    vouched[1].vouches.push(vouch(
        9,
        VouchAct::SignOff,
        VouchRole::Attested,
        who("z1", "Dr Zed"),
    ));
    assert_eq!(
        sign_off_vouches(&vouched, "z1"),
        vec![(Uuid::from_u128(1), VouchRole::CoSigned)]
    );
}

#[test]
fn addendum_covers_only_the_actors_unsigned_parts() {
    let mut mine_unsigned = part(1, None);
    mine_unsigned.drafted_actor = Some("n1".into());
    let mut theirs_unsigned = part(2, None);
    theirs_unsigned.drafted_actor = Some("n2".into());
    let mut mine_vouched = part(3, None);
    mine_vouched.drafted_actor = Some("n1".into());
    mine_vouched.vouches.push(vouch(
        9,
        VouchAct::Addendum,
        VouchRole::Attested,
        who("n1", "Nurse N"),
    ));
    let mut mine_signed = part(4, Some(who("n1", "Nurse N")));
    mine_signed.drafted_actor = Some("n1".into());
    let parts = vec![
        mine_unsigned,
        theirs_unsigned,
        mine_vouched,
        mine_signed,
        part(5, None),
    ];
    assert_eq!(addendum_covers(&parts, "n1"), ids(&[1]));
    assert!(
        addendum_covers(&parts, "").is_empty(),
        "an unknown actor covers nothing"
    );
}

#[test]
fn a_later_addendum_by_another_brings_the_offer_back() {
    let ada = who("a1", "Dr Ada");
    let mut parts = vec![part(1, Some(ada.clone()))];
    assert!(!needs_sign_off(&parts));
    parts.push(part(2, Some(who("b1", "Nurse Bo"))));
    assert!(needs_sign_off(&parts));
    assert_eq!(
        sign_off_vouches(&parts, "a1"),
        vec![(Uuid::from_u128(2), VouchRole::CoSigned)]
    );
}

#[test]
fn the_shown_parts_must_be_exactly_the_notes() {
    let parts = vec![part(1, None), part(2, None)];
    assert!(shows_exactly(&parts, &ids(&[2, 1])));
    assert!(!shows_exactly(&parts, &ids(&[1])), "a part not shown");
    assert!(!shows_exactly(&parts, &ids(&[1, 2, 3])), "a part gone");
    assert!(
        !shows_exactly(&parts, &ids(&[1, 1, 2])),
        "the same count, too"
    );
    assert!(!shows_exactly(&parts, &ids(&[1, 1])), "and the same set");
}

/// Final review M2: the plan co-signs exactly what db/071's floor admits, which reads the part's
/// ENVELOPE signer (`signer_key_id`), not the key its grade names: a human's key, of an actor
/// other than the presser's. Anything else is attested, so the floor never refuses the press.
#[test]
fn sign_off_co_signs_only_what_the_floor_admits() {
    // The grade names Dr Ada (an attester), but a device signed the envelope.
    let mut device_envelope = part(1, Some(who("a1", "Dr Ada")));
    device_envelope.signer_human_actor = None;
    // The grade names Dr Ada, but the presser's own key signed the envelope.
    let mut own_envelope = part(2, Some(who("a1", "Dr Ada")));
    own_envelope.signer_human_actor = Some("z1".into());
    // Another human signed the envelope, though the grade is not `Signed`.
    let mut human_envelope = part(3, None);
    human_envelope.signer_human_actor = Some("b1".into());
    assert_eq!(
        sign_off_vouches(&[device_envelope, own_envelope, human_envelope], "z1"),
        vec![
            (Uuid::from_u128(1), VouchRole::Attested),
            (Uuid::from_u128(2), VouchRole::Attested),
            (Uuid::from_u128(3), VouchRole::CoSigned),
        ]
    );
}

// ---- sign-in slice 3c-3b (ADR-0093): a since-repudiated key's signature counts for nothing ----

#[test]
fn a_repudiated_signature_counts_as_unsigned() {
    let p = part(1, Some(repudiated("a", "Dr A")));
    assert!(!part_is_signed(&p));
    assert!(signed_for(&p).is_empty());
    assert!(needs_sign_off(&[p]));
}

/// Review Focus 1: a colleague's sign-off of a repudiated part clears nothing.
#[test]
fn a_colleagues_sign_off_does_not_clear_a_repudiated_part() {
    let mut p = part(1, Some(repudiated("a", "Dr A")));
    p.vouches.push(vouch(
        10,
        VouchAct::SignOff,
        VouchRole::Attested,
        who("z", "Dr Zed"),
    ));
    assert!(!part_is_signed(&p));
    let parts = [p];
    assert!(needs_sign_off(&parts));
    assert!(signed_off_by(&parts).is_none());
}

#[test]
fn only_the_real_persons_re_sign_clears_it() {
    let mut p = part(1, Some(repudiated("a", "Dr A")));
    p.vouches.push(vouch(
        10,
        VouchAct::SignOff,
        VouchRole::Attested,
        who("a", "Dr A"),
    ));
    assert!(re_signed(&p));
    assert!(part_is_signed(&p));
    assert!(!needs_sign_off(&[p]));
}

/// Review Focus 2: the re-keyed writer must not be answered "nothing to sign".
#[test]
fn the_re_keyed_writer_is_offered_their_repudiated_part() {
    let p = part(1, Some(repudiated("a", "Dr A")));
    let id = p.event_id;
    assert_eq!(sign_off_vouches(&[p], "a"), vec![(id, VouchRole::Attested)]);
}

/// Review Focus 3: never plan a co-sign over a repudiated signature.
#[test]
fn a_repudiated_part_is_never_planned_as_a_co_sign() {
    let p = part(1, Some(repudiated("a", "Dr A")));
    assert_eq!(p.signer_human_actor.as_deref(), Some("a"));
    let id = p.event_id;
    assert_eq!(sign_off_vouches(&[p], "z"), vec![(id, VouchRole::Attested)]);
}

/// Review Focus 5: a repudiated key's vouch counts nothing.
#[test]
fn a_repudiated_keys_vouch_counts_nothing() {
    let mut p = part(2, None);
    p.vouches.push(vouch(
        10,
        VouchAct::SignOff,
        VouchRole::Attested,
        repudiated("z", "Dr Zed"),
    ));
    assert!(!part_is_signed(&p));
}

#[test]
fn an_addendum_does_not_cover_a_repudiated_part() {
    let mut p = part(1, Some(repudiated("a", "Dr A")));
    p.drafted_actor = Some("a".into());
    assert!(addendum_covers(&[p], "a").is_empty());
}

/// #1009: once the real person has re-signed, their signature stands again, so a supervisor's
/// sign-off of it is a co-sign like any other (the floor admits it: a human's envelope key, another
/// actor) and can clear a student's "— not co-signed".
#[test]
fn a_re_signed_part_is_co_signed_again() {
    let mut sam = student("s", "Sam");
    sam.repudiated = true;
    let mut p = part(1, Some(sam));
    p.vouches.push(vouch(
        10,
        VouchAct::SignOff,
        VouchRole::Attested,
        student("s", "Sam"),
    ));
    assert!(re_signed(&p));
    let id = p.event_id;
    assert_eq!(sign_off_vouches(&[p], "z"), vec![(id, VouchRole::CoSigned)]);
}

/// ADR-0093 decision 5: a colleague's sign-off made BEFORE the re-sign starts counting after it.
/// It named the part's exact bytes, and the real person has since said the words were theirs.
#[test]
fn a_colleagues_earlier_sign_off_counts_once_re_signed() {
    let mut p = part(1, Some(repudiated("a", "Dr A")));
    p.vouches.push(vouch(
        10,
        VouchAct::SignOff,
        VouchRole::Attested,
        who("z", "Dr Zed"),
    ));
    assert!(signed_off_by(std::slice::from_ref(&p)).is_none());
    p.vouches.push(vouch(
        11,
        VouchAct::SignOff,
        VouchRole::Attested,
        who("a", "Dr A"),
    ));
    let parts = [p];
    assert_eq!(signed_off_by(&parts).map(|s| s.actor.as_str()), Some("z"));
}

/// A co-sign under a key since repudiated (a stolen-key co-sign) counts for nothing in the
/// note-level rules too, not only in the byline: the student's note still needs signing off, and
/// the real Dr Zed, re-keyed, is offered the part again as a co-sign.
#[test]
fn a_repudiated_co_signers_vouch_signs_nothing_off() {
    let mut p = part(1, Some(student("s", "Sam")));
    p.vouches.push(vouch(
        10,
        VouchAct::SignOff,
        VouchRole::CoSigned,
        repudiated("z", "Dr Zed"),
    ));
    let id = p.event_id;
    let parts = [p];
    assert!(needs_sign_off(&parts));
    assert!(signed_off_by(&parts).is_none());
    assert_eq!(
        sign_off_vouches(&parts, "z"),
        vec![(id, VouchRole::CoSigned)]
    );
}

/// ADR-0093 (c): a colleague who already signed off a repudiated, not re-signed part stays in
/// `signed_for`, so a second press plans nothing: it never files a second immutable vouch.
#[test]
fn a_colleague_who_vouched_a_repudiated_part_is_not_offered_it_again() {
    let mut p = part(1, Some(repudiated("a", "Dr A")));
    p.vouches.push(vouch(
        10,
        VouchAct::SignOff,
        VouchRole::Attested,
        who("z", "Dr Zed"),
    ));
    assert!(signed_for(&p).iter().any(|s| s.actor == "z"));
    assert!(sign_off_vouches(&[p], "z").is_empty());
}

/// The dead-key rule is decided PER PART: a repudiated part beside a standing student part is
/// attested while the student's part is still co-signed, by a colleague and (ADR-0093 (h), #1011)
/// by the re-keyed writer alike.
#[test]
fn a_repudiated_part_beside_a_standing_one_is_attested_alone() {
    let parts = [
        part(1, Some(repudiated("a", "Dr A"))),
        part(2, Some(student("s", "Sam"))),
    ];
    let plan = vec![
        (Uuid::from_u128(1), VouchRole::Attested),
        (Uuid::from_u128(2), VouchRole::CoSigned),
    ];
    assert_eq!(sign_off_vouches(&parts, "z"), plan);
    assert_eq!(sign_off_vouches(&parts, "a"), plan);
}
