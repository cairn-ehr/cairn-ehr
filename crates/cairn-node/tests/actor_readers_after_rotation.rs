//! GUI sign-in slice 2, Task 2 — every reader of the actor registry asks the right question once
//! a key can be ROTATED (`rotate-key`, same actor) and REPUDIATED (the key, not the actor).
//!
//! Two questions, and the whole task is keeping them apart (design §5.5, §6):
//!
//! * **ADMISSION — "may this key sign something NEW?"** A write door (`submit_event`,
//!   `apply_remote_event`) asks `actor_current`. A rotated-away key and a repudiated key are both
//!   refused there: when a late-arriving K1 event was actually signed cannot be known, so refusing
//!   it is the safe answer.
//! * **RE-GRADE / HISTORY — "did this key legitimately belong to the actor when it signed?"** A
//!   reader that re-judges an event ALREADY in the log (`cairn_claim_authority`, db/048's
//!   worklist) asks `actor_key_vouches`, so a rotation keeps K1's past authority and a
//!   repudiation removes the impostor key's. Readers that resolve a key against registry HISTORY
//!   (db/005's suppression owner-gate, db/006's recall, db/020's attribution stamp) must count
//!   `rotate-key` rows as key-bearing, or a rotated-in key's events lose their author.
//!
//! The reader-by-reader table is `docs/superpowers/plans/2026-11-01-slice-2-actor-reader-table.md`.
//!
//! Rotation and repudiation are written through `cairn_actor_append`, the registry's one append
//! door, as the owner (these tests connect as the owner; the vault doors that will call it as a
//! definer are Task 4). Real Postgres, gated on $CAIRN_TEST_PG, serialized by the shared guard.
mod common;
use cairn_event::sensitivity::SensitivityWithdrawal;
use cairn_event::{generate_key, sign, EventBody, SigningKey};
use common::{
    apply_remote_attested, apply_remote_raw, assert_chart_grade, bearing_withdrawal_body,
    body_from_spec, content_address_of, cs, db_msg, enroll_human, enroll_human_with_role, setup,
    submit_attested, submit_registration, submit_signed, withdrawal_body_with_id, EventSpec,
};
use tokio_postgres::Client;
use uuid::Uuid;

/// The tables a sensitivity test must start empty (setup truncates the core ones).
const SENSITIVITY: &[&str] = &["sensitivity_assertion", "sensitivity_withdrawal"];

/// The actor a key was ENROLLED under. A rotated-in key has no enroll row, so callers pass the
/// actor's first key.
async fn actor_of(c: &Client, kid: &str) -> Vec<u8> {
    c.query_one(
        "SELECT actor_id FROM actor_event WHERE signing_key_id = $1 AND op = 'enroll'",
        &[&kid],
    )
    .await
    .unwrap()
    .get(0)
}

/// Re-key `aid` to a fresh, runtime-generated key (never a literal: CLAUDE.md rule 6).
async fn rotate(c: &Client, aid: &[u8]) -> (SigningKey, String) {
    let (sk, kid) = generate_key().unwrap();
    c.execute(
        "SELECT cairn_actor_append($1, 'rotate-key', $2, NULL, NULL)",
        &[&aid, &kid],
    )
    .await
    .unwrap();
    (sk, kid)
}

/// A superuser repudiates `kid` (approved_by 'console': who judged it does not matter here).
async fn repudiate(c: &Client, aid: &[u8], kid: &str) {
    c.execute(
        "SELECT cairn_actor_append($1, 'repudiate', $2, 'console', NULL)",
        &[&aid, &kid],
    )
    .await
    .unwrap();
}

/// Ask db/005's re-grade directly.
async fn authority(c: &Client, event: Uuid, target: Uuid) -> String {
    c.query_one(
        "SELECT cairn_claim_authority($1::text::uuid, $2::text::uuid)",
        &[&event.to_string(), &target.to_string()],
    )
    .await
    .unwrap()
    .get(0)
}

/// The chart-wide grade an assertion still produces — the CONSEQUENCE of the re-grade.
async fn effective(c: &Client, event: Uuid) -> String {
    c.query_one(
        "SELECT grade FROM cairn_effective_sensitivity($1::text::uuid)",
        &[&event.to_string()],
    )
    .await
    .unwrap()
    .get(0)
}

/// A fresh registered chart carrying one device-signed "sequestered" assertion; returns
/// (patient, assertion id). `wall` orders it below the caller's own events.
async fn sequestered_chart(c: &Client, sk: &SigningKey, kid: &str, wall: i64) -> (Uuid, Uuid) {
    let p = Uuid::now_v7();
    submit_registration(c, sk, kid, p, wall).await;
    let a = assert_chart_grade(c, sk, kid, p, wall + 1, "sequestered").await;
    (p, a)
}

/// The device relays a withdrawal of `assertion` that the human `kid_h` attests (the R1 shape,
/// as `claim_authority.rs`'s `a_vouched_human_attestation_is_attested` lands it). Returns its id.
#[allow(clippy::too_many_arguments)]
async fn attested_withdrawal(
    c: &Client,
    sk: &SigningKey,
    kid: &str,
    sk_h: &SigningKey,
    kid_h: &str,
    p: Uuid,
    assertion: Uuid,
    wall: i64,
) -> Uuid {
    let withdraws_hex = hex::encode(content_address_of(c, assertion).await);
    let w = SensitivityWithdrawal {
        withdraws_hex: &withdraws_hex,
        rationale: "patient consented",
    };
    let wid = Uuid::now_v7();
    let body = bearing_withdrawal_body(kid, kid_h, p, wid, &w, wall);
    apply_remote_attested(c, sk, body, sk_h, kid_h)
        .await
        .expect("a properly attested withdrawal must land while its attester is current");
    wid
}

/// A plain additive note signed by `kid` — the simplest thing a key can author.
fn note(p: Uuid, kid: &str, wall: i64) -> EventBody {
    body_from_spec(Uuid::now_v7(), kid, note_spec(p, wall))
}

fn note_spec(p: Uuid, wall: i64) -> EventSpec<'static> {
    EventSpec {
        patient: p,
        event_type: "note.added",
        schema_version: "advisory/1",
        payload: serde_json::json!({ "text": "seen, stable" }),
        plaintext_twin: None,
        wall,
    }
}

// ---------------------------------------------------------------------------------------------
// RE-GRADE: cairn_claim_authority (db/005) and the withdrawal worklist (db/048)
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn a_rotated_attester_still_vouches_for_past_claims() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();
    let (sk, kid) = setup(&c, SENSITIVITY).await;
    let (sk_h, kid_h) = enroll_human(&c).await;
    let (p, a) = sequestered_chart(&c, &sk, &kid, 1).await;
    let wid = attested_withdrawal(&c, &sk, &kid, &sk_h, &kid_h, p, a, 20).await;
    assert_eq!(authority(&c, wid, a).await, "attested", "precondition");

    // The human forgets their passphrase and is re-keyed in person: same actor, new key.
    rotate(&c, &actor_of(&c, &kid_h).await).await;

    assert_eq!(
        authority(&c, wid, a).await,
        "attested",
        "a rotation retires a key for NEW work; what it signed while current stays the actor's"
    );
    assert_eq!(
        effective(&c, a).await,
        "routine",
        "and the withdrawal it attested keeps lowering the grade"
    );
}

#[tokio::test]
async fn a_repudiated_key_loses_its_past_authority_but_the_actor_survives() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();
    let (sk, kid) = setup(&c, SENSITIVITY).await;
    let (sk_1, kid_1) = enroll_human(&c).await;
    let aid = actor_of(&c, &kid_1).await;

    // Dr A's own key K1 attests a withdrawal on chart 1.
    let (p1, a1) = sequestered_chart(&c, &sk, &kid, 1).await;
    let w1 = attested_withdrawal(&c, &sk, &kid, &sk_1, &kid_1, p1, a1, 20).await;
    // An impostor is re-keyed as Dr A (K2) and attests a withdrawal on chart 2.
    let (sk_2, kid_2) = rotate(&c, &aid).await;
    let (p2, a2) = sequestered_chart(&c, &sk, &kid, 30).await;
    let w2 = attested_withdrawal(&c, &sk, &kid, &sk_2, &kid_2, p2, a2, 40).await;
    assert_eq!(authority(&c, w2, a2).await, "attested", "precondition");

    // A superuser repudiates the impostor's key.
    repudiate(&c, &aid, &kid_2).await;

    assert_eq!(
        authority(&c, w2, a2).await,
        "unverified",
        "everything a repudiated key signed loses the authority the floor granted it (§5.5)"
    );
    assert_eq!(
        effective(&c, a2).await,
        "sequestered",
        "so the protection it stripped stands again"
    );
    // The ACTOR survives: Dr A's own earlier work keeps its authority, and nobody revoked them.
    assert_eq!(
        authority(&c, w1, a1).await,
        "attested",
        "repudiating the impostor's key must not take the real Dr A's past work with it"
    );
    let revokes: i64 = c
        .query_one(
            "SELECT count(*) FROM actor_event WHERE actor_id = $1 AND op = 'revoke'",
            &[&aid],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        revokes, 0,
        "a repudiation is of a KEY; the actor is not revoked"
    );
}

#[tokio::test]
async fn the_worklist_still_names_a_rotated_attester() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();
    let (sk, kid) = setup(&c, SENSITIVITY).await;
    let (sk_h, kid_h) = enroll_human(&c).await;
    let aid = actor_of(&c, &kid_h).await;
    // The attesting human has never written on this chart: a stranger-attested strip.
    let (p, a) = sequestered_chart(&c, &sk, &kid, 1).await;
    attested_withdrawal(&c, &sk, &kid, &sk_h, &kid_h, p, a, 20).await;
    rotate(&c, &aid).await;

    let row = c
        .query_one(
            "SELECT reason, responsible_actor_id FROM sensitivity_withdrawal_worklist \
             WHERE patient_id = $1::text::uuid",
            &[&p.to_string()],
        )
        .await
        .expect("the strip must still be listed after its attester was re-keyed");
    assert_eq!(row.get::<_, String>(0), "stranger-attested");
    assert_eq!(
        row.get::<_, Vec<u8>>(1),
        aid,
        "the accountable actor is still the human who attested, not the device that relayed it"
    );
}

/// A self-withdrawal (R2, un-attested — the remote door is the only one that admits it).
async fn self_withdraw(
    c: &Client,
    sk: &SigningKey,
    kid: &str,
    p: Uuid,
    a: Uuid,
    wall: i64,
) -> Uuid {
    let withdraws_hex = hex::encode(content_address_of(c, a).await);
    let w = SensitivityWithdrawal {
        withdraws_hex: &withdraws_hex,
        rationale: "mine to lower",
    };
    let wid = Uuid::now_v7();
    apply_remote_raw(c, sk, withdrawal_body_with_id(p, wid, kid, &w, wall))
        .await
        .expect("an un-attested self-withdrawal lands at the remote door");
    wid
}

/// R2 is about the PERSON: Dr A asserts with K1, is re-keyed, and withdraws with K2 — 'self'.
#[tokio::test]
async fn a_rekeyed_human_withdrawing_their_own_assertion_is_self() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();
    let (sk, kid) = setup(&c, SENSITIVITY).await;
    let (sk_1, kid_1) = enroll_human(&c).await;
    let p = Uuid::now_v7();
    submit_registration(&c, &sk, &kid, p, 1).await;
    let a = assert_chart_grade(&c, &sk_1, &kid_1, p, 10, "sequestered").await;
    let (sk_2, kid_2) = rotate(&c, &actor_of(&c, &kid_1).await).await;

    let w = self_withdraw(&c, &sk_2, &kid_2, p, a, 20).await;
    assert_eq!(authority(&c, w, a).await, "self");
}

/// R2's key join: the withdrawal's OWN signing key must still vouch. An impostor re-keyed as
/// Dr A (K2) self-withdraws Dr A's assertion; once K2 is repudiated that withdrawal is
/// 'unverified', while Dr A's own K1 self-withdrawal stays 'self'. Joining on the actor alone
/// would keep the impostor's 'self' — Dr A's actor still vouches through K1.
#[tokio::test]
async fn a_repudiated_keys_self_withdrawal_is_unverified_but_the_owners_stays_self() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();
    let (sk, kid) = setup(&c, SENSITIVITY).await;
    let (sk_1, kid_1) = enroll_human(&c).await;
    let aid = actor_of(&c, &kid_1).await;
    let (p1, p2) = (Uuid::now_v7(), Uuid::now_v7());
    submit_registration(&c, &sk, &kid, p1, 1).await;
    submit_registration(&c, &sk, &kid, p2, 2).await;
    let a1 = assert_chart_grade(&c, &sk_1, &kid_1, p1, 10, "sequestered").await;
    let a2 = assert_chart_grade(&c, &sk_1, &kid_1, p2, 11, "sequestered").await;
    let w1 = self_withdraw(&c, &sk_1, &kid_1, p1, a1, 20).await;
    let (sk_2, kid_2) = rotate(&c, &aid).await;
    let w2 = self_withdraw(&c, &sk_2, &kid_2, p2, a2, 30).await;
    assert_eq!(authority(&c, w2, a2).await, "self", "precondition");

    repudiate(&c, &aid, &kid_2).await;

    assert_eq!(
        authority(&c, w2, a2).await,
        "unverified",
        "a repudiated key's self-withdrawal loses its authority"
    );
    assert_eq!(
        authority(&c, w1, a1).await,
        "self",
        "the real owner's own self-withdrawal is untouched"
    );
}

// ---------------------------------------------------------------------------------------------
// ADMISSION: the write doors refuse a rotated-away or repudiated key
// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn a_rotated_away_key_cannot_submit() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();
    let (sk, kid) = setup(&c, &[]).await;
    let (sk_1, kid_1) = enroll_human(&c).await;
    let p = Uuid::now_v7();
    submit_registration(&c, &sk, &kid, p, 1).await;
    let (sk_2, kid_2) = rotate(&c, &actor_of(&c, &kid_1).await).await;

    let e = submit_signed(&c, &sk_1, &kid_1, note_spec(p, 10))
        .await
        .expect_err("a rotated-away key may sign nothing new");
    assert!(
        db_msg(&e).contains("not an enrolled, non-revoked actor"),
        "{}",
        db_msg(&e)
    );
    submit_signed(&c, &sk_2, &kid_2, note_spec(p, 11))
        .await
        .expect("positive control: the actor's new key signs");
}

#[tokio::test]
async fn a_repudiated_key_cannot_submit() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();
    let (sk, kid) = setup(&c, &[]).await;
    let (sk_1, kid_1) = enroll_human(&c).await;
    let aid = actor_of(&c, &kid_1).await;
    let p = Uuid::now_v7();
    submit_registration(&c, &sk, &kid, p, 1).await;
    let (sk_2, kid_2) = rotate(&c, &aid).await;
    repudiate(&c, &aid, &kid_2).await;

    let e = submit_signed(&c, &sk_2, &kid_2, note_spec(p, 10))
        .await
        .expect_err("a repudiated key may sign nothing new at the local door");
    assert!(
        db_msg(&e).contains("not an enrolled, non-revoked actor"),
        "{}",
        db_msg(&e)
    );
    let e = apply_remote_raw(&c, &sk_2, note(Uuid::now_v7(), &kid_2, 10))
        .await
        .expect_err("nor at the remote door");
    assert!(
        db_msg(&e).contains("not an enrolled, non-revoked actor"),
        "{}",
        db_msg(&e)
    );
    // No fall-back: the key the rotation retired does not become current again.
    let e = submit_signed(&c, &sk_1, &kid_1, note_spec(p, 11))
        .await
        .expect_err("repudiating the latest key leaves the actor with NO current key");
    assert!(
        db_msg(&e).contains("not an enrolled, non-revoked actor"),
        "{}",
        db_msg(&e)
    );
}

/// The remote door re-admits what was signed ELSEWHERE OR EARLIER — a peer's event, a medium,
/// this node's own backup on restore (`restore/clinical.rs` replays every record through
/// `apply_remote_event`). So its signer and attester gates ask `actor_key_vouches`, not
/// `actor_current`: rotation is not compromise, and after a re-key a restore must not pen the
/// clinic's own K1 history. (`submit_event` stays on `actor_current`: see the test above.)
/// The restore CLI itself is not driven here — it needs a full medium fixture; this door is
/// the only actor check on its path.
#[tokio::test]
async fn a_rotated_away_keys_past_event_is_admitted_by_remote_apply() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();
    let (sk, kid) = setup(&c, SENSITIVITY).await;
    let (sk_1, kid_1) = enroll_human(&c).await;
    let (p, a) = sequestered_chart(&c, &sk, &kid, 1).await;
    // Both events exist (signed by K1) before the rotation; they arrive after it.
    let n = note(p, &kid_1, 10);
    rotate(&c, &actor_of(&c, &kid_1).await).await;

    apply_remote_raw(&c, &sk_1, n)
        .await
        .expect("a K1-signed event must still apply after K1 was rotated away (signer gate)");
    let wid = attested_withdrawal(&c, &sk, &kid, &sk_1, &kid_1, p, a, 20).await;
    assert_eq!(
        authority(&c, wid, a).await,
        "attested",
        "and a K1 attestation still clears the attester gate and grades as attested"
    );
}

/// db/043 deferred gate 1 mirrors db/020's attester gate: an event attested by K1, deferred
/// (its type unknown here), then promoted after K1 was rotated away, must promote — not stay
/// deferred forever. A probe type of this file's own, de-classified first so a predecessor
/// that died mid-test cannot leave it classified.
#[tokio::test]
async fn a_deferred_event_attested_by_a_rotated_away_key_still_promotes() {
    const PROBE: &str = "cairn_test.rotation_deferred_probe";
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();
    let _ = setup(&c, &[]).await;
    c.execute(
        "DELETE FROM event_type_class WHERE event_type = $1",
        &[&PROBE],
    )
    .await
    .unwrap();
    let (sk_1, kid_1) = enroll_human(&c).await;
    let mut b = body_from_spec(
        Uuid::now_v7(),
        &kid_1,
        EventSpec {
            patient: Uuid::now_v7(),
            event_type: PROBE,
            schema_version: "future/1",
            payload: serde_json::json!({ "probe": "deferred, then promoted" }),
            plaintext_twin: None,
            wall: 10,
        },
    );
    // A responsibility-bearing contributor, so gate 1 (the attester check) actually runs.
    b.contributors = serde_json::json!([{
        "actor_id": kid_1, "role": "authored", "responsibility": {"held_by": kid_1}
    }]);
    apply_remote_attested(&c, &sk_1, b, &sk_1, &kid_1)
        .await
        .expect("an unclassified type is admitted uninterpreted (ADR-0056)");
    rotate(&c, &actor_of(&c, &kid_1).await).await;

    c.execute(
        "INSERT INTO event_type_class (event_type, mode, targets_other_author) \
         VALUES ($1, 'additive', FALSE) ON CONFLICT DO NOTHING",
        &[&PROBE],
    )
    .await
    .unwrap();
    let promoted = c
        .query(
            "SELECT promoted_type FROM cairn_readjudicate_deferred()",
            &[],
        )
        .await
        .unwrap()
        .len();
    c.execute(
        "DELETE FROM event_type_class WHERE event_type = $1",
        &[&PROBE],
    )
    .await
    .unwrap();
    assert_eq!(
        promoted, 1,
        "an attestation made by a since-rotated key still vouches at re-adjudication"
    );
}

// ---------------------------------------------------------------------------------------------
// HISTORY: readers that resolve a key against every key-bearing row
// ---------------------------------------------------------------------------------------------

/// db/005 `cairn_suppression_author_ok`: a note's human author is found from the signer's
/// registry history. A rotated-in key has a `rotate-key` row and no `enroll` row; if history
/// counted only `enroll`, the note would have NO human author and anyone could hide it.
#[tokio::test]
async fn a_rotated_in_keys_note_cannot_be_hidden_by_another_human() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();
    let (sk, kid) = setup(&c, &[]).await;
    let (_sk_a1, kid_a1) = enroll_human_with_role(&c, "clinician-a").await;
    let (sk_b, kid_b) = enroll_human_with_role(&c, "clinician-b").await;
    let (sk_a2, kid_a2) = rotate(&c, &actor_of(&c, &kid_a1).await).await;
    let p = Uuid::now_v7();
    submit_registration(&c, &sk, &kid, p, 1).await;
    let n = note(p, &kid_a2, 10);
    let signed = sign(&n, &sk_a2).unwrap();
    c.execute("SELECT submit_event($1)", &[&signed.signed_bytes])
        .await
        .unwrap();

    let hide = body_from_spec(
        Uuid::now_v7(),
        &kid_b,
        EventSpec {
            patient: p,
            event_type: "visibility.suppress",
            schema_version: "advisory/1",
            payload: serde_json::json!({ "target_event_id": n.event_id }),
            plaintext_twin: None,
            wall: 20,
        },
    );
    let e = submit_attested(&c, &sk_b, hide, &sk_b, &kid_b)
        .await
        .expect_err("another human may not hide Dr A's note because Dr A was re-keyed");
    assert!(
        db_msg(&e).contains("cross-author suppression refused"),
        "{}",
        db_msg(&e)
    );
}

/// db/006 `events_by_actor_epoch`: a contamination-cascade recall of (key, epoch) must select
/// what a ROTATED-IN key signed, and still what the rotated-away key signed.
#[tokio::test]
async fn a_recall_selects_what_both_keys_of_a_rotated_actor_signed() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();
    let (sk, kid) = setup(&c, &[]).await;
    let (sk_1, kid_1) = generate_key().unwrap();
    c.execute(
        "SELECT enroll_actor('agent', \
         '{\"model\":\"triage-stub\",\"version\":\"1\",\"skill_epoch\":\"e1\"}', $1)",
        &[&kid_1],
    )
    .await
    .unwrap();
    let p = Uuid::now_v7();
    submit_registration(&c, &sk, &kid, p, 1).await;
    let n1 = note(p, &kid_1, 10);
    c.execute(
        "SELECT submit_event($1)",
        &[&sign(&n1, &sk_1).unwrap().signed_bytes],
    )
    .await
    .unwrap();
    let (sk_2, kid_2) = rotate(&c, &actor_of(&c, &kid_1).await).await;
    let n2 = note(p, &kid_2, 11);
    c.execute(
        "SELECT submit_event($1)",
        &[&sign(&n2, &sk_2).unwrap().signed_bytes],
    )
    .await
    .unwrap();

    let recalled = |k: String| {
        let c = &c;
        async move {
            c.query(
                "SELECT event_id::text, attribution FROM events_by_actor_epoch($1, 'e1')",
                &[&k],
            )
            .await
            .unwrap()
            .iter()
            .map(|r| (r.get::<_, String>(0), r.get::<_, String>(1)))
            .collect::<Vec<_>>()
        }
    };
    assert_eq!(
        recalled(kid_2.clone()).await,
        vec![(n2.event_id.clone(), "pinned".to_string())],
        "a recall must never silently miss what the rotated-in key signed"
    );
    assert_eq!(
        recalled(kid_1.clone()).await,
        vec![(n1.event_id.clone(), "pinned".to_string())],
        "nor what the rotated-away key signed before the rotation"
    );
}

/// db/020 step 2: a replicated event's attribution stamp is resolved from the key's whole
/// registry history. A rotated-in key must stamp its actor, not NULL ("honestly unknown").
#[tokio::test]
async fn a_remote_event_by_a_rotated_in_key_is_attributed_to_its_actor() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();
    let _ = setup(&c, &[]).await;
    let (_sk_1, kid_1) = enroll_human(&c).await;
    let aid = actor_of(&c, &kid_1).await;
    let (sk_2, kid_2) = rotate(&c, &aid).await;
    let n = note(Uuid::now_v7(), &kid_2, 10);
    apply_remote_raw(&c, &sk_2, n.clone())
        .await
        .expect("the actor's current key is admitted at the remote door");

    let stamped: Option<Vec<u8>> = c
        .query_one(
            "SELECT actor_id FROM event_log WHERE event_id = $1::text::uuid",
            &[&n.event_id],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(stamped, Some(aid));
}
