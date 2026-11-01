//! DB-gated: GUI sign-in slice 2's vault doors (db/063) — grants, enrolment by an enroller, and
//! judging a key (spec 2026-11-01-sign-in-slice-2, §5.1, §5.2, §5.5). Re-key and reseal are
//! `vault_rekey.rs`; the shared fixtures are `common/vault_enroller_kit.rs`.
//!
//! The trust boundary under test: a raw-SQL client holding `cairn_agent` can call every door
//! here, so each act must be refused unless it carries the right SIGNED approval —
//!
//! * an enroller (a member a superuser granted the role) may enrol, never a `superuser` or an
//!   `admin`, and only while their own key is CONFIRMED;
//! * only a superuser grants, withdraws, confirms or repudiates;
//! * what an enroller issues is `unconfirmed` (it still signs), what a superuser issues is
//!   confirmed at once; repudiating an enroller makes what they issued `doubtful`.
mod common;
#[path = "common/vault_enroller_kit.rs"]
#[macro_use]
mod kit;

use cairn_event::SigningKey;
use cairn_node::medication::{assert_medication, AssertMedicationInput, AuthorParams};
use kit::*;
use uuid::Uuid;

#[tokio::test]
async fn a_superuser_grants_and_an_enroller_enrols() {
    db_gated!();
    let (_g, mut c, node_sk, node_kid) = fresh().await;
    staff(&c).await;
    assert_eq!(
        release(&c, "jsmith", &pass_of(JSMITH), "enrol").await.0,
        "released"
    );
    enrol(&c, "anurse", "nurse", ANURSE, "jsmith", JSMITH)
        .await
        .unwrap();
    assert_eq!(status(&c, ANURSE).await, "unconfirmed");
    let rows = c
        .query("SELECT staff_id, signing_key_id, approver_staff_id, doubtful FROM cairn_vault_unconfirmed()", &[])
        .await
        .unwrap();
    assert_eq!(
        rows.len(),
        1,
        "only the enroller's enrolment is unconfirmed"
    );
    assert_eq!(rows[0].get::<_, String>(0), "anurse");
    assert_eq!(rows[0].get::<_, String>(1), kid_of(ANURSE));
    assert_eq!(rows[0].get::<_, String>(2), "jsmith");
    assert!(!rows[0].get::<_, bool>(3));

    // Unconfirmed never blocks signing: the new key signs a clinical act the floor accepts.
    let (o, seed, _) = release(&c, "anurse", &pass_of(ANURSE), "clinical").await;
    assert_eq!(o, "released");
    let key = SigningKey::from_bytes(&seed.unwrap().try_into().unwrap());
    let patient = Uuid::now_v7();
    common::submit_registration(&c, &node_sk, &node_kid, patient, 0).await;
    let input = AssertMedicationInput {
        term: "atorvastatin",
        coding: None,
        formulation: None,
        dose_amount: Some("40"),
        dose_unit: Some("mg"),
        sig: None,
        info_source: "patient-reported",
        started: None,
        started_precision: None,
    };
    let kid = kid_of(ANURSE);
    let author = AuthorParams {
        human_sk: &key,
        human_kid: &kid,
    };
    assert_medication(
        &mut c,
        &node_sk,
        &node_kid,
        &node_kid,
        patient,
        &input,
        Some(&author),
        None,
    )
    .await
    .expect("an unconfirmed key still signs");
}

#[tokio::test]
async fn a_superuser_enrolment_is_confirmed_at_once() {
    db_gated!();
    let (_g, c, _, _) = fresh().await;
    staff(&c).await;
    assert_eq!(status(&c, JSMITH).await, "confirmed");
    let n: i64 = c
        .query_one(
            "SELECT count(*) FROM actor_event WHERE op = 'confirm' AND signing_key_id = $1 AND approved_by = $2",
            &[&kid_of(JSMITH), &kid_of(BOSS)],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(n, 1);
    assert!(c
        .query("SELECT * FROM cairn_vault_unconfirmed()", &[])
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn bootstrap_writes_a_confirm_row() {
    db_gated!();
    let (_g, c, _, _) = fresh().await;
    boot(&c).await;
    assert_eq!(status(&c, BOSS).await, "confirmed");
    let ops: Vec<String> = c
        .query(
            "SELECT op || ':' || approved_by FROM actor_event WHERE signing_key_id = $1 ORDER BY seq",
            &[&kid_of(BOSS)],
        )
        .await
        .unwrap()
        .iter()
        .map(|r| r.get(0))
        .collect();
    assert_eq!(ops, ["enroll:console", "confirm:console"]);
}

#[tokio::test]
async fn an_enroller_cannot_enrol_a_superuser_or_an_admin() {
    db_gated!();
    let (_g, c, _, _) = fresh().await;
    staff(&c).await;
    for role in ["superuser", "admin"] {
        let e = enrol(&c, "anurse", role, ANURSE, "jsmith", JSMITH)
            .await
            .expect_err("an enroller enrols neither a superuser nor an admin");
        assert!(msg(&e).contains("enroller"), "{}", msg(&e));
    }
    // A clinician without the grant enrols nobody.
    enrol(&c, "bnurse", "nurse", BNURSE, "boss", BOSS)
        .await
        .unwrap();
    let e = enrol(&c, "anurse", "nurse", ANURSE, "bnurse", BNURSE)
        .await
        .unwrap_err();
    assert!(msg(&e).contains("superuser"), "{}", msg(&e));
}

#[tokio::test]
async fn an_enroller_cannot_grant_or_judge() {
    db_gated!();
    let (_g, c, _, _) = fresh().await;
    staff(&c).await;
    enrol(&c, "anurse", "nurse", ANURSE, "jsmith", JSMITH)
        .await
        .unwrap();
    let e = set_enroller(&c, "anurse", true, "jsmith", JSMITH)
        .await
        .unwrap_err();
    assert!(msg(&e).contains("superuser"), "{}", msg(&e));
    for confirm in [true, false] {
        let e = judge(&c, ANURSE, confirm, "jsmith", JSMITH)
            .await
            .unwrap_err();
        assert!(msg(&e).contains("superuser"), "{}", msg(&e));
    }
    assert_eq!(status(&c, ANURSE).await, "unconfirmed");
    // Nor re-key an admin; and a superuser is re-keyed only at the console.
    enrol(&c, "aadmin", "admin", ADMIN, "boss", BOSS)
        .await
        .unwrap();
    let e = rekey(&c, "aadmin", "admin", None, None, JSMITH2, "jsmith", JSMITH)
        .await
        .unwrap_err();
    assert!(msg(&e).contains("enroller"), "{}", msg(&e));
    let e = rekey(&c, "boss", "superuser", None, None, JSMITH2, "boss", BOSS)
        .await
        .unwrap_err();
    assert!(msg(&e).contains("console"), "{}", msg(&e));
}

/// Review Focus 3: an enroller whose own key came from another enroller's re-key cannot enrol
/// until a superuser confirms that key.
#[tokio::test]
async fn an_unconfirmed_enroller_cannot_enrol() {
    db_gated!();
    let (_g, c, _, _) = fresh().await;
    staff(&c).await;
    rekey(
        &c,
        "jsmith",
        "clinician",
        None,
        None,
        JSMITH2,
        "bwong",
        BWONG,
    )
    .await
    .unwrap();
    assert_eq!(status(&c, JSMITH2).await, "unconfirmed");
    assert_eq!(
        release(&c, "jsmith", &pass_of(JSMITH2), "enrol").await.0,
        "refused-purpose"
    );
    let e = enrol(&c, "anurse", "nurse", ANURSE, "jsmith", JSMITH2)
        .await
        .unwrap_err();
    assert!(msg(&e).contains("confirmed"), "{}", msg(&e));
    judge(&c, JSMITH2, true, "boss", BOSS).await.unwrap();
    assert_eq!(
        release(&c, "jsmith", &pass_of(JSMITH2), "enrol").await.0,
        "released"
    );
    enrol(&c, "anurse", "nurse", ANURSE, "jsmith", JSMITH2)
        .await
        .unwrap();
}

/// Review Focus 4: a grant token replayed after a withdrawal does not re-grant.
#[tokio::test]
async fn a_stale_grant_token_is_refused() {
    db_gated!();
    let (_g, c, _, _) = fresh().await;
    boot(&c).await;
    enrol(&c, "jsmith", "clinician", JSMITH, "boss", BOSS)
        .await
        .unwrap();
    let a = bytes(
        &c,
        "SELECT cairn_vault_grant_address('jsmith', true, 0)",
        &[],
    )
    .await;
    let first = sign(&a, BOSS);
    let grant = "SELECT cairn_vault_set_enroller('jsmith', true, 'boss', $1)";
    c.execute(grant, &[&first]).await.unwrap();
    set_enroller(&c, "jsmith", false, "boss", BOSS)
        .await
        .unwrap();
    let e = c
        .execute(grant, &[&first])
        .await
        .expect_err("a replayed grant token");
    assert!(msg(&e).contains("attestation"), "{}", msg(&e));
    let on: bool = c
        .query_one("SELECT cairn_vault_is_enroller('jsmith')", &[])
        .await
        .unwrap()
        .get(0);
    assert!(!on);
}

/// The cascade (§5.5): repudiating an enroller's key makes what it approved, and nobody has yet
/// confirmed, doubtful; what a superuser already confirmed stays confirmed.
#[tokio::test]
async fn a_repudiated_enrollers_unconfirmed_members_are_doubtful() {
    db_gated!();
    let (_g, c, _, _) = fresh().await;
    staff(&c).await;
    enrol(&c, "anurse", "nurse", ANURSE, "jsmith", JSMITH)
        .await
        .unwrap();
    enrol(&c, "bnurse", "nurse", BNURSE, "jsmith", JSMITH)
        .await
        .unwrap();
    judge(&c, BNURSE, true, "boss", BOSS).await.unwrap();
    judge(&c, JSMITH, false, "boss", BOSS).await.unwrap();
    assert_eq!(status(&c, ANURSE).await, "doubtful");
    assert_eq!(status(&c, BNURSE).await, "confirmed");
    let rows = c
        .query(
            "SELECT staff_id, doubtful FROM cairn_vault_unconfirmed()",
            &[],
        )
        .await
        .unwrap();
    let got: Vec<(String, bool)> = rows.iter().map(|r| (r.get(0), r.get(1))).collect();
    assert_eq!(got, [("anurse".to_string(), true)]);
    // A doubtful key can still be confirmed by a superuser who checked.
    judge(&c, ANURSE, true, "boss", BOSS).await.unwrap();
    assert_eq!(status(&c, ANURSE).await, "confirmed");
}
