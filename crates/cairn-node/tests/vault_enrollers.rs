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

// ---- The optional site expiry for unconfirmed keys (§5.5, last bullet) ----

/// Age key `v` by `days`: its introducing (and any confirm) row is back-dated. The registry is
/// append-only (`actor_event_no_update`), so the owner lifts that trigger for this fixture alone,
/// inside ONE transaction: DDL is transactional in Postgres, so if anything between fails (or the
/// test panics and drops the connection) the trigger comes back with the rollback, and no later
/// suite on this shared database runs with the registry left writable.
async fn age_key(c: &tokio_postgres::Client, v: u8, days: i32) {
    c.batch_execute("BEGIN; ALTER TABLE actor_event DISABLE TRIGGER actor_event_no_update")
        .await
        .unwrap();
    let aged = c
        .execute(
            "UPDATE actor_event SET recorded_at = recorded_at - make_interval(days => $2) \
              WHERE signing_key_id = $1",
            &[&kid_of(v), &days],
        )
        .await;
    let end = if aged.is_ok() {
        "ALTER TABLE actor_event ENABLE TRIGGER actor_event_no_update; COMMIT"
    } else {
        "ROLLBACK"
    };
    c.batch_execute(end).await.unwrap();
    aged.unwrap();
}

/// When the window should say signing stops for `staff`, as the window asks it.
async fn expires_at(c: &tokio_postgres::Client, staff: &str) -> Option<std::time::SystemTime> {
    c.query_one("SELECT cairn_vault_unconfirmed_expires_at($1)", &[&staff])
        .await
        .unwrap()
        .get(0)
}

async fn set_expiry(c: &tokio_postgres::Client, days: Option<i32>) {
    c.execute("SELECT cairn_vault_set_unconfirmed_expiry($1)", &[&days])
        .await
        .unwrap();
}

/// The reference default is no expiry: an unconfirmed key a year old still signs.
#[tokio::test]
async fn no_expiry_by_default() {
    db_gated!();
    let (_g, c, _, _) = fresh().await;
    staff(&c).await;
    enrol(&c, "anurse", "nurse", ANURSE, "jsmith", JSMITH)
        .await
        .unwrap();
    age_key(&c, ANURSE, 365).await;
    assert_eq!(status(&c, ANURSE).await, "unconfirmed");
    assert_eq!(expires_at(&c, "anurse").await, None);
    assert_eq!(
        release(&c, "anurse", &pass_of(ANURSE), "clinical").await.0,
        "released"
    );
}

/// Past the site's limit an unconfirmed key is not released, for any passphrase, and nothing is
/// counted (like 'not-current', the answer comes before the count). A superuser's confirmation,
/// or switching the expiry off, lets it sign again.
#[tokio::test]
async fn an_expired_unconfirmed_key_is_not_released() {
    db_gated!();
    let (_g, c, _, _) = fresh().await;
    staff(&c).await;
    enrol(&c, "anurse", "nurse", ANURSE, "jsmith", JSMITH)
        .await
        .unwrap();
    enrol(&c, "bnurse", "nurse", BNURSE, "jsmith", JSMITH)
        .await
        .unwrap();
    age_key(&c, ANURSE, 31).await;
    age_key(&c, BNURSE, 25).await;
    set_expiry(&c, Some(30)).await;

    let now = std::time::SystemTime::now();
    assert!(expires_at(&c, "anurse").await.unwrap() < now);
    let b = expires_at(&c, "bnurse")
        .await
        .expect("bnurse is inside the limit");
    let left = b.duration_since(now).unwrap().as_secs();
    assert!((4 * 86_400..6 * 86_400).contains(&left), "{left}");
    // What the window reads: the same instant, as RFC 3339 UTC; nothing for a confirmed key.
    let shown = cairn_node::vault_admin::unconfirmed_expires_at(&c, "bnurse")
        .await
        .unwrap()
        .expect("a date to warn with");
    let same: bool = c
        .query_one(
            "SELECT $1::text::timestamptz = date_trunc('second', cairn_vault_unconfirmed_expires_at('bnurse'))",
            &[&shown],
        )
        .await
        .unwrap()
        .get(0);
    assert!(same && shown.ends_with('Z'), "{shown}");
    // The superuser's worklist carries the same date for each unconfirmed key (I1).
    let listed: Vec<(String, Option<String>)> = cairn_node::vault_admin::unconfirmed(&c)
        .await
        .unwrap()
        .into_iter()
        .map(|u| (u.staff_id, u.stops_at))
        .collect();
    assert!(
        listed.contains(&("bnurse".to_string(), Some(shown.clone()))),
        "{listed:?}"
    );
    assert_eq!(
        cairn_node::vault_admin::unconfirmed_expires_at(&c, "jsmith")
            .await
            .unwrap(),
        None
    );

    for pass in [pass_of(ANURSE), pass_of(BNURSE)] {
        let (o, seed, retry) = release(&c, "anurse", &pass, "clinical").await;
        assert_eq!((o.as_str(), seed, retry), ("expired", None, None));
    }
    let (failures, last): (i64, Option<i64>) = c
        .query_one(
            "SELECT failures, last_attempt_ms FROM cairn_vault_attempt_state('anurse')",
            &[],
        )
        .await
        .map(|r| (r.get(0), r.get(1)))
        .unwrap();
    assert_eq!(
        (failures, last),
        (0, None),
        "an expired key's tries are not counted"
    );
    assert_eq!(
        release(&c, "bnurse", &pass_of(BNURSE), "clinical").await.0,
        "released",
        "inside the limit it still signs"
    );

    judge(&c, ANURSE, true, "boss", BOSS).await.unwrap();
    assert_eq!(expires_at(&c, "anurse").await, None);
    assert_eq!(
        release(&c, "anurse", &pass_of(ANURSE), "clinical").await.0,
        "released"
    );
    set_expiry(&c, None).await;
    assert_eq!(expires_at(&c, "bnurse").await, None);
}

/// A confirmed key never expires, however old.
#[tokio::test]
async fn a_confirmed_key_never_expires() {
    db_gated!();
    let (_g, c, _, _) = fresh().await;
    staff(&c).await;
    age_key(&c, JSMITH, 400).await;
    set_expiry(&c, Some(1)).await;
    assert_eq!(status(&c, JSMITH).await, "confirmed");
    assert_eq!(expires_at(&c, "jsmith").await, None);
    assert_eq!(
        release(&c, "jsmith", &pass_of(JSMITH), "clinical").await.0,
        "released"
    );
    // Leave the site with no limit, whatever runs next on this database.
    set_expiry(&c, None).await;
}

/// A limit is a whole number of days greater than zero; zero or less is refused, not stored.
#[tokio::test]
async fn a_non_positive_expiry_is_refused() {
    db_gated!();
    let (_g, c, _, _) = fresh().await;
    for days in [0, -3, 36501] {
        let e = c
            .execute(
                "SELECT cairn_vault_set_unconfirmed_expiry($1)",
                &[&Some(days)],
            )
            .await
            .expect_err("a non-positive limit");
        assert!(msg(&e).contains("at least one day"), "{}", msg(&e));
    }
}

/// A doubtful key (its approver since repudiated) is still unconfirmed and expires like one: the
/// more suspect case must never be the one that escapes the site's limit (db/061
/// `cairn_vault_key_expires_at`, `IN ('unconfirmed', 'doubtful')`).
#[tokio::test]
async fn a_doubtful_key_expires_like_an_unconfirmed_one() {
    db_gated!();
    let (_g, c, _, _) = fresh().await;
    staff(&c).await;
    enrol(&c, "anurse", "nurse", ANURSE, "jsmith", JSMITH)
        .await
        .unwrap();
    judge(&c, JSMITH, false, "boss", BOSS).await.unwrap();
    assert_eq!(status(&c, ANURSE).await, "doubtful");
    age_key(&c, ANURSE, 10).await;
    set_expiry(&c, Some(5)).await;
    assert!(expires_at(&c, "anurse").await.is_some());
    assert_eq!(
        release(&c, "anurse", &pass_of(ANURSE), "clinical").await.0,
        "expired"
    );
    set_expiry(&c, None).await;
}
