//! DB-gated: GUI sign-in slice 2's vault doors (db/063) — enrollers, re-key, judging a key, and
//! the change of passphrase (spec 2026-11-01-sign-in-slice-2, §5.1–§5.5 and §5.8).
//!
//! The trust boundary under test: a raw-SQL client holding `cairn_agent` can call every door
//! here, so each act must be refused unless it carries the right SIGNED approval —
//!
//! * an enroller (a member a superuser granted the role) may enrol and re-key, never a
//!   `superuser` or an `admin`, and only while their own key is CONFIRMED;
//! * only a superuser grants, withdraws, confirms or repudiates;
//! * what an enroller issues is `unconfirmed` (it still signs), what a superuser issues is
//!   confirmed at once;
//! * a re-key keeps the ACTOR and retires the old key; the old passphrase then answers
//!   `'replaced'`, so the real owner learns of it;
//! * a change of passphrase needs the member's OWN signed proof over the exact new blob.
//!
//! Members are built through the real doors, blobs from `seal::seal`, keys from runtime seeds
//! (house rule 6). Every key's passphrase is derived from its seed ([`pass_of`]).
mod common;

use cairn_event::keys::Secret32;
use cairn_event::{generate_key, sign_attestation, SigningKey};
use cairn_node::medication::{assert_medication, AssertMedicationInput, AuthorParams};
use cairn_node::{db, seal};
use tokio_postgres::types::ToSql;
use tokio_postgres::{Client, Error};
use uuid::Uuid;

fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

/// A deterministic seed built at runtime; `variant` gives each key its own.
fn test_seed(variant: u8) -> [u8; 32] {
    std::array::from_fn(|i| (i as u8).wrapping_mul(29).wrapping_add(variant))
}

const BOSS: u8 = 1;
const JSMITH: u8 = 2;
const BWONG: u8 = 3;
const ANURSE: u8 = 4;
const BNURSE: u8 = 5;
const ADMIN: u8 = 6;
const JSMITH2: u8 = 7;
const JSMITH3: u8 = 8;

fn kid_of(v: u8) -> String {
    hex::encode(
        SigningKey::from_bytes(&test_seed(v))
            .verifying_key()
            .to_bytes(),
    )
}

/// Every key's passphrase, derived from its seed variant, so a re-key's new passphrase differs.
fn pass_of(v: u8) -> String {
    format!("passphrase number {v}")
}

fn blob_under(v: u8, passphrase: &str) -> Vec<u8> {
    let sealed = seal::seal(
        &Secret32::from_bytes(test_seed(v)),
        passphrase,
        &seal::generate_recovery_code(),
    )
    .unwrap();
    seal::to_cbor(&sealed)
}

fn blob(v: u8) -> Vec<u8> {
    blob_under(v, &pass_of(v))
}

fn display(staff: &str) -> String {
    format!("Dr {staff}")
}

fn pinned(role: &str, staff: &str) -> String {
    format!(r#"{{"role":"{role}","handle":"{staff}"}}"#)
}

/// An attestation token over `addr` by the key of seed `v` (an approver's act, or a member's proof).
fn sign(addr: &[u8], v: u8) -> Vec<u8> {
    sign_attestation(
        addr,
        &kid_of(v),
        "approved",
        &SigningKey::from_bytes(&test_seed(v)),
    )
    .unwrap()
}

async fn bytes(c: &Client, sql: &str, params: &[&(dyn ToSql + Sync)]) -> Vec<u8> {
    c.query_one(sql, params).await.unwrap().get(0)
}

fn msg(e: &Error) -> String {
    e.as_db_error()
        .map(|d| d.message().to_string())
        .unwrap_or_else(|| e.to_string())
}

/// Serialised, freshly loaded, empty vault and registry, plus this node's device actor and an
/// unwrap key (so a clinical act can be submitted; `vault_lib.rs::fresh`).
async fn fresh() -> (Client, Client, SigningKey, String) {
    let base = cs().expect("guarded by caller");
    let guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    c.batch_execute(
        "TRUNCATE vault_attempt, vault_member, vault_terminal, event_log, actor_event, \
         patient_chart, node_unwrap_key, event_dek, event_clear, erasure_shred_log CASCADE",
    )
    .await
    .unwrap();
    let (node_sk, node_kid) = generate_key().unwrap();
    c.execute(
        "SELECT enroll_actor('device', '{\"role\":\"registration-desk\"}', $1)",
        &[&node_kid],
    )
    .await
    .unwrap();
    let unwrap = cairn_event::seal::generate_unwrap_secret().unwrap();
    c.execute(
        "SELECT cairn_register_unwrap_key($1)",
        &[&cairn_event::seal::unwrap_public(&unwrap)
            .as_bytes()
            .as_slice()],
    )
    .await
    .unwrap();
    (guard, c, node_sk, node_kid)
}

async fn boot(c: &Client) {
    c.execute(
        "SELECT cairn_vault_bootstrap('boss', 'Dr boss', $1::text::jsonb, $2, $3, NULL, '0400', NULL)",
        &[&pinned("superuser", "boss"), &kid_of(BOSS), &blob(BOSS)],
    )
    .await
    .unwrap();
}

/// Enrol `staff` as `role` with key `v`, approved by `approver` signing with key `by`.
async fn enrol(
    c: &Client,
    staff: &str,
    role: &str,
    v: u8,
    approver: &str,
    by: u8,
) -> Result<Vec<u8>, Error> {
    let a = bytes(
        c,
        "SELECT cairn_vault_enrol_address($1, $2, $3)",
        &[&staff, &kid_of(v), &role],
    )
    .await;
    c.query_one(
        "SELECT cairn_vault_enrol($1, $2, $3, $4::text::jsonb, $5, $6, NULL, NULL, 'x@example.org', $7, $8)",
        &[&staff, &display(staff), &role, &pinned(role, staff), &kid_of(v), &blob(v), &approver, &sign(&a, by)],
    )
    .await
    .map(|r| r.get(0))
}

/// Grant (or withdraw) the enroller role, signed by superuser `su` with key `by`.
async fn set_enroller(c: &Client, staff: &str, grant: bool, su: &str, by: u8) -> Result<(), Error> {
    let prior: i64 = c
        .query_one("SELECT cairn_vault_grant_prior($1)", &[&staff])
        .await
        .unwrap()
        .get(0);
    let a = bytes(
        c,
        "SELECT cairn_vault_grant_address($1, $2, $3)",
        &[&staff, &grant, &prior],
    )
    .await;
    c.execute(
        "SELECT cairn_vault_set_enroller($1, $2, $3, $4)",
        &[&staff, &grant, &su, &sign(&a, by)],
    )
    .await
    .map(drop)
}

/// Confirm or repudiate key `v`, signed by `su` with key `by`.
async fn judge(c: &Client, v: u8, confirm: bool, su: &str, by: u8) -> Result<(), Error> {
    let a = bytes(
        c,
        "SELECT cairn_vault_judge_address($1, $2)",
        &[&kid_of(v), &confirm],
    )
    .await;
    c.execute(
        "SELECT cairn_vault_judge_key($1, $2, $3, $4)",
        &[&kid_of(v), &confirm, &su, &sign(&a, by)],
    )
    .await
    .map(drop)
}

/// Re-key `staff` to key `v` (sealed under `pass_of(v)`), approved by `approver` with key `by`.
/// `name` and `email` are only needed when no member row exists (after a restore).
#[allow(clippy::too_many_arguments)]
async fn rekey(
    c: &Client,
    staff: &str,
    role: &str,
    name: Option<&str>,
    email: Option<&str>,
    v: u8,
    approver: &str,
    by: u8,
) -> Result<Vec<u8>, Error> {
    let a = bytes(
        c,
        "SELECT cairn_vault_rekey_address($1, $2)",
        &[&staff, &kid_of(v)],
    )
    .await;
    c.query_one(
        "SELECT cairn_vault_rekey($1, $2, $3, NULL, $4, $5, $6, NULL, $7, $8)",
        &[
            &staff,
            &role,
            &name,
            &email,
            &kid_of(v),
            &blob(v),
            &approver,
            &sign(&a, by),
        ],
    )
    .await
    .map(|r| r.get(0))
}

/// The release door's answer: (outcome, seed, retry_after_ms).
async fn release(
    c: &Client,
    staff: &str,
    passphrase: &str,
    purpose: &str,
) -> (String, Option<Vec<u8>>, Option<i64>) {
    let r = c
        .query_one(
            "SELECT outcome, seed, retry_after_ms FROM cairn_vault_release($1, $2, $3)",
            &[&staff, &passphrase, &purpose],
        )
        .await
        .expect("cairn_vault_release must never raise");
    (r.get(0), r.get(1), r.get(2))
}

async fn status(c: &Client, v: u8) -> String {
    c.query_one("SELECT cairn_key_confirmation($1)", &[&kid_of(v)])
        .await
        .unwrap()
        .get(0)
}

async fn actor_of(c: &Client, staff: &str) -> Vec<u8> {
    bytes(
        c,
        "SELECT actor_id FROM vault_member WHERE staff_id = $1",
        &[&staff],
    )
    .await
}

async fn current_key(c: &Client, actor: &[u8]) -> Option<String> {
    c.query_opt(
        "SELECT signing_key_id FROM actor_current WHERE actor_id = $1",
        &[&actor],
    )
    .await
    .unwrap()
    .map(|r| r.get(0))
}

/// boss (bootstrapped), jsmith and bwong (clinicians, enrolled by boss, both granted enroller).
async fn staff(c: &Client) {
    boot(c).await;
    for (s, v) in [("jsmith", JSMITH), ("bwong", BWONG)] {
        enrol(c, s, "clinician", v, "boss", BOSS).await.unwrap();
        set_enroller(c, s, true, "boss", BOSS).await.unwrap();
    }
}

macro_rules! db_gated {
    () => {
        if cs().is_none() {
            eprintln!("skipped: set CAIRN_TEST_PG");
            return;
        }
    };
}

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

#[tokio::test]
async fn rekey_keeps_the_actor_and_retires_the_old_key() {
    db_gated!();
    let (_g, c, _, _) = fresh().await;
    staff(&c).await;
    let actor = actor_of(&c, "jsmith").await;
    for _ in 0..3 {
        assert_eq!(release(&c, "jsmith", "not it", "clinical").await.0, "wrong");
    }
    let back = rekey(
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
    assert_eq!(back, actor, "a re-key keeps the actor");
    assert_eq!(current_key(&c, &actor).await, Some(kid_of(JSMITH2)));
    let failures = "SELECT failures FROM cairn_vault_attempt_state('jsmith')";
    let f: i64 = c.query_one(failures, &[]).await.unwrap().get(0);
    assert_eq!(f, 0, "a re-key resets the failure count");

    // The real owner, typing the passphrase they know, is told — counted like a wrong guess.
    let (o, seed, retry) = release(&c, "jsmith", &pass_of(JSMITH), "clinical").await;
    assert_eq!((o.as_str(), seed, retry), ("replaced", None, None));
    let f: i64 = c.query_one(failures, &[]).await.unwrap().get(0);
    assert_eq!(f, 1, "'replaced' counts like 'wrong'");
    let notice = c
        .query_one("SELECT replaced_at IS NOT NULL, approver_display_name FROM cairn_vault_replaced_notice(' JSmith ')", &[])
        .await
        .unwrap();
    assert!(notice.get::<_, bool>(0));
    assert_eq!(notice.get::<_, String>(1), display("bwong"));
    assert_eq!(
        release(&c, "jsmith", &pass_of(JSMITH2), "clinical").await.0,
        "released"
    );
    // A member never re-keyed gets a bare 'wrong'.
    assert_eq!(release(&c, "bwong", "not it", "clinical").await.0, "wrong");
    assert!(c
        .query("SELECT * FROM cairn_vault_replaced_notice('bwong')", &[])
        .await
        .unwrap()
        .is_empty());
}

/// Review Focus 2: an impostor's re-key, repudiated, leaves the actor with no key — not revoked —
/// and a proper re-key restores it.
#[tokio::test]
async fn rekey_after_repudiation_restores_the_actor() {
    db_gated!();
    let (_g, c, _, _) = fresh().await;
    staff(&c).await;
    let actor = actor_of(&c, "jsmith").await;
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
    judge(&c, JSMITH2, false, "boss", BOSS).await.unwrap();
    assert_eq!(current_key(&c, &actor).await, None);
    assert_eq!(
        release(&c, "jsmith", &pass_of(JSMITH2), "clinical").await.0,
        "not-current"
    );
    let e = judge(&c, JSMITH2, true, "boss", BOSS).await.unwrap_err();
    assert!(msg(&e).contains("final"), "{}", msg(&e));

    assert_eq!(
        rekey(&c, "jsmith", "clinician", None, None, JSMITH3, "boss", BOSS)
            .await
            .unwrap(),
        actor
    );
    assert_eq!(current_key(&c, &actor).await, Some(kid_of(JSMITH3)));
    assert_eq!(status(&c, JSMITH3).await, "confirmed");
    assert_eq!(
        release(&c, "jsmith", &pass_of(JSMITH3), "clinical").await.0,
        "released"
    );
}

/// §5.8: after a restore the registry is back and the vault is empty; the re-key door re-creates
/// the member on the SAME actor.
#[tokio::test]
async fn rekey_without_a_member_row_recreates_it() {
    db_gated!();
    let (_g, c, _, _) = fresh().await;
    boot(&c).await;
    let actor = enrol(&c, "jsmith", "clinician", JSMITH, "boss", BOSS)
        .await
        .unwrap();
    c.batch_execute(
        "DELETE FROM vault_attempt WHERE staff_id = 'jsmith'; DELETE FROM vault_member WHERE staff_id = 'jsmith'",
    )
    .await
    .unwrap();
    let e = rekey(
        &c,
        "jsmith",
        "clinician",
        None,
        Some("j@example.org"),
        JSMITH2,
        "boss",
        BOSS,
    )
    .await
    .unwrap_err();
    assert!(msg(&e).contains("display name"), "{}", msg(&e));
    let e = rekey(
        &c,
        "nobody",
        "clinician",
        Some("N"),
        Some("n@x"),
        JSMITH2,
        "boss",
        BOSS,
    )
    .await
    .unwrap_err();
    assert!(msg(&e).contains("registry"), "{}", msg(&e));

    let back = rekey(
        &c,
        "jsmith",
        "clinician",
        Some("Dr J Smith"),
        Some("j@example.org"),
        JSMITH2,
        "boss",
        BOSS,
    )
    .await
    .unwrap();
    assert_eq!(back, actor);
    assert_eq!(actor_of(&c, "jsmith").await, actor);
    assert_eq!(current_key(&c, &actor).await, Some(kid_of(JSMITH2)));
    assert_eq!(
        release(&c, "jsmith", &pass_of(JSMITH2), "clinical").await.0,
        "released"
    );
}

#[tokio::test]
async fn reseal_needs_the_members_own_proof() {
    db_gated!();
    let (_g, c, _, _) = fresh().await;
    staff(&c).await;
    let fresh_blob = blob_under(JSMITH, "a new passphrase");
    let a = bytes(
        &c,
        "SELECT cairn_vault_reseal_address('jsmith', $1)",
        &[&fresh_blob],
    )
    .await;
    let reseal = "SELECT cairn_vault_reseal('jsmith', $1, 'new hint', $2)";

    let e = c
        .execute(reseal, &[&fresh_blob, &sign(&a, BOSS)])
        .await
        .unwrap_err();
    assert!(msg(&e).contains("proof"), "{}", msg(&e));
    let other = bytes(
        &c,
        "SELECT cairn_vault_reseal_address('jsmith', $1)",
        &[&blob(JSMITH)],
    )
    .await;
    let e = c
        .execute(reseal, &[&fresh_blob, &sign(&other, JSMITH)])
        .await
        .unwrap_err();
    assert!(msg(&e).contains("proof"), "{}", msg(&e));
    assert!(
        !msg(&e).contains(&hex::encode(&fresh_blob)[..16]),
        "never echo the blob"
    );

    c.execute(reseal, &[&fresh_blob, &sign(&a, JSMITH)])
        .await
        .unwrap();
    assert_eq!(
        release(&c, "jsmith", &pass_of(JSMITH), "clinical").await.0,
        "wrong"
    );
    let (o, seed, _) = release(&c, "jsmith", "a new passphrase", "clinical").await;
    assert_eq!(
        (o.as_str(), seed),
        ("released", Some(test_seed(JSMITH).to_vec()))
    );
    let hint: Option<String> = c
        .query_one("SELECT cairn_vault_hint('jsmith')", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(hint.as_deref(), Some("new hint"));
    // The proof binds the blob it replaced: replaying it now (the stored blob moved on) fails.
    let e = c
        .execute(reseal, &[&fresh_blob, &sign(&a, JSMITH)])
        .await
        .unwrap_err();
    assert!(msg(&e).contains("proof"), "{}", msg(&e));
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

/// The purpose map: every role may release its key to re-seal it; a superuser or an admin never
/// for a clinical act; a clinician for an enrolment only while an enroller.
#[tokio::test]
async fn reseal_releases_the_key_for_reseal_only() {
    db_gated!();
    let (_g, c, _, _) = fresh().await;
    boot(&c).await;
    enrol(&c, "jsmith", "clinician", JSMITH, "boss", BOSS)
        .await
        .unwrap();
    enrol(&c, "aadmin", "admin", ADMIN, "boss", BOSS)
        .await
        .unwrap();
    for (s, v, released, refused) in [
        ("boss", BOSS, ["enrol", "reseal"], "clinical"),
        ("aadmin", ADMIN, ["enrol", "reseal"], "clinical"),
        ("jsmith", JSMITH, ["clinical", "reseal"], "enrol"),
    ] {
        for p in released {
            assert_eq!(
                release(&c, s, &pass_of(v), p).await.0,
                "released",
                "{s} {p}"
            );
        }
        assert_eq!(
            release(&c, s, &pass_of(v), refused).await.0,
            "refused-purpose",
            "{s} {refused}"
        );
    }
}
