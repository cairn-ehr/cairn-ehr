//! DB-gated: the clinicians' key vault floor (db/061).
//!
//! The vault replaces a key FILE: each clinician's Ed25519 seed sits in `vault_member`, sealed under
//! their passphrase, and the ONLY way out is `cairn_vault_release`, which checks the passphrase inside
//! Postgres and hands back the seed for one act. These tests pin the floor's promises:
//!
//! * the released seed is the enrolled key, whatever case/whitespace the staff ID is typed in;
//! * a wrong passphrase is an ANSWER (`outcome = 'wrong'`), never a raised error (an error can carry
//!   the passphrase into the server log, and would roll back the failure counter);
//! * failures 1–3 are free, the 4th starts a back-off, and a try during the back-off (`waiting`)
//!   neither unseals nor counts — a back-off, never a lock-out;
//! * a superuser's key is never released for a clinical act (spec §5.1), and a clinician's key never
//!   enrols anyone;
//! * enrolment needs a superuser's attestation over THIS request;
//! * the runtime role cannot read the wrapped keys, yet can still use the release door.
//!
//! Every member is built through the real doors (`cairn_vault_bootstrap`, `cairn_vault_enrol`) with
//! blobs from `seal::seal` and keys from runtime-computed seeds (house rule 6: no key literals).
use cairn_event::keys::Secret32;
use cairn_event::{sign_attestation, SigningKey};
use cairn_node::{db, seal};
use tokio_postgres::Client;

fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

/// A deterministic 32-byte seed built at runtime; `variant` gives each person their own.
fn test_seed(variant: u8) -> [u8; 32] {
    std::array::from_fn(|i| (i as u8).wrapping_mul(13).wrapping_add(variant))
}

/// The hex verifying-key id of a seed — what the registry calls `signing_key_id`.
fn kid_of(seed: [u8; 32]) -> String {
    hex::encode(SigningKey::from_bytes(&seed).verifying_key().to_bytes())
}

/// Seal `seed` under `passphrase`, exactly as the enrolment surface will.
fn sealed_blob(seed: [u8; 32], passphrase: &str) -> Vec<u8> {
    let sealed = seal::seal(
        &Secret32::from_bytes(seed),
        passphrase,
        &seal::generate_recovery_code(),
    )
    .unwrap();
    seal::to_cbor(&sealed)
}

/// One serialised, freshly-loaded connection with an empty vault and actor registry.
/// The returned guard must be held for the whole test.
async fn fresh() -> (Client, Client) {
    let base = cs().expect("guarded by caller");
    let guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    c.batch_execute(
        "TRUNCATE vault_attempt, vault_member, vault_terminal, event_log, actor_event, patient_chart CASCADE",
    )
    .await
    .unwrap();
    (guard, c)
}

const SU_PASS: &str = "boss passphrase";
const CLIN_PASS: &str = "nurse passphrase";

/// Bootstrap the site's first superuser, `boss`, at the "console". Returns its key id.
async fn bootstrap_boss(c: &Client) -> String {
    let seed = test_seed(1);
    let kid = kid_of(seed);
    c.query_one(
        "SELECT cairn_vault_bootstrap($1, $2, $3::text::jsonb, $4, $5, $6, $7, $8)",
        &[
            &"boss",
            &"Site Superuser",
            &r#"{"role":"superuser","staff_id":"boss"}"#,
            &kid,
            &sealed_blob(seed, SU_PASS),
            &Some("the usual"),
            &Some("0400 000 000"),
            &None::<&str>,
        ],
    )
    .await
    .unwrap();
    kid
}

/// The superuser's attestation token over the enrolment `(staff_id, kid, role)`.
async fn enrol_token(
    c: &Client,
    staff_id: &str,
    kid: &str,
    role: &str,
    signer: [u8; 32],
) -> Vec<u8> {
    let addr: Vec<u8> = c
        .query_one(
            "SELECT cairn_vault_enrol_address($1, $2, $3)",
            &[&staff_id, &kid, &role],
        )
        .await
        .unwrap()
        .get(0);
    sign_attestation(
        &addr,
        &kid_of(signer),
        "superuser",
        &SigningKey::from_bytes(&signer),
    )
    .unwrap()
}

/// Call `cairn_vault_enrol` for a clinician whose pinned role is `role`.
async fn enrol(
    c: &Client,
    staff_id: &str,
    seed: [u8; 32],
    role: &str,
    superuser_staff_id: &str,
    token: &[u8],
) -> Result<Vec<u8>, tokio_postgres::Error> {
    let pinned = format!(r#"{{"role":"{role}","staff_id":"{staff_id}"}}"#);
    c.query_one(
        "SELECT cairn_vault_enrol($1, $2, $3, $4::text::jsonb, $5, $6, $7, $8, $9, $10, $11)",
        &[
            &staff_id,
            &"J Smith",
            &role,
            &pinned,
            &kid_of(seed),
            &sealed_blob(seed, CLIN_PASS),
            &None::<&str>,
            &None::<&str>,
            &Some("j@example.org"),
            &superuser_staff_id,
            &token,
        ],
    )
    .await
    .map(|r| r.get(0))
}

/// Bootstrap `boss` and enrol clinician `jsmith`. Returns (boss kid, jsmith kid).
async fn boss_and_jsmith(c: &Client) -> (String, String) {
    let boss = bootstrap_boss(c).await;
    let seed = test_seed(2);
    let token = enrol_token(c, "jsmith", &kid_of(seed), "clinician", test_seed(1)).await;
    enrol(c, "jsmith", seed, "clinician", "boss", &token)
        .await
        .expect("a superuser's token over this request enrols");
    (boss, kid_of(seed))
}

/// One release outcome, as plain Rust values.
#[derive(Debug)]
struct Release {
    outcome: String,
    seed: Option<Vec<u8>>,
    signing_key_id: Option<String>,
    retry_after_ms: Option<i64>,
}

/// Call the release door. A raised error here fails the "never raises" contract outright.
async fn release(c: &Client, staff_id: &str, passphrase: &str, purpose: &str) -> Release {
    let row = c
        .query_one(
            "SELECT outcome, seed, signing_key_id, retry_after_ms FROM cairn_vault_release($1, $2, $3)",
            &[&staff_id, &passphrase, &purpose],
        )
        .await
        .expect("cairn_vault_release must never raise");
    Release {
        outcome: row.get(0),
        seed: row.get(1),
        signing_key_id: row.get(2),
        retry_after_ms: row.get(3),
    }
}

async fn failures(c: &Client, staff_id: &str) -> i32 {
    c.query_one(
        "SELECT failures FROM vault_attempt WHERE staff_id = $1",
        &[&staff_id],
    )
    .await
    .unwrap()
    .get(0)
}

fn db_message(e: &tokio_postgres::Error) -> String {
    e.as_db_error()
        .map(|d| d.message().to_string())
        .unwrap_or_else(|| e.to_string())
}

#[tokio::test]
async fn release_returns_the_enrolled_key() {
    let Some(_) = cs() else {
        eprintln!("skipped");
        return;
    };
    let (_g, c) = fresh().await;
    let (_, jsmith) = boss_and_jsmith(&c).await;
    let r = release(&c, "jsmith", CLIN_PASS, "clinical").await;
    assert_eq!(r.outcome, "released");
    let seed: [u8; 32] = r
        .seed
        .expect("a released row carries the seed")
        .try_into()
        .unwrap();
    assert_eq!(kid_of(seed), jsmith);
    assert_eq!(r.signing_key_id.as_deref(), Some(jsmith.as_str()));
    assert_eq!(r.retry_after_ms, None);
}

#[tokio::test]
async fn release_normalises_the_staff_id() {
    let Some(_) = cs() else {
        eprintln!("skipped");
        return;
    };
    let (_g, c) = fresh().await;
    let (_, jsmith) = boss_and_jsmith(&c).await;
    let r = release(&c, " JSmith ", CLIN_PASS, "clinical").await;
    assert_eq!(r.outcome, "released");
    assert_eq!(r.signing_key_id.as_deref(), Some(jsmith.as_str()));
}

#[tokio::test]
async fn a_wrong_passphrase_is_an_answer_not_an_error() {
    let Some(_) = cs() else {
        eprintln!("skipped");
        return;
    };
    let (_g, c) = fresh().await;
    boss_and_jsmith(&c).await;
    let r = release(&c, "jsmith", "not it", "clinical").await;
    assert_eq!(r.outcome, "wrong");
    assert_eq!(r.seed, None);
    assert_eq!(failures(&c, "jsmith").await, 1);
    // An unknown staff ID is an answer too, and counts nothing (there is no member to count against).
    let r = release(&c, "nobody", "not it", "clinical").await;
    assert_eq!(r.outcome, "unknown");
    assert_eq!(r.seed, None);
}

#[tokio::test]
async fn the_fourth_failure_starts_a_back_off() {
    let Some(_) = cs() else {
        eprintln!("skipped");
        return;
    };
    let (_g, c) = fresh().await;
    boss_and_jsmith(&c).await;
    for n in 1..=3 {
        let r = release(&c, "jsmith", "not it", "clinical").await;
        assert_eq!(r.outcome, "wrong", "failure {n}");
        assert_eq!(
            r.retry_after_ms, None,
            "failures 1-3 are free (failure {n})"
        );
    }
    let r = release(&c, "jsmith", "not it", "clinical").await;
    assert_eq!(r.outcome, "wrong");
    assert!(
        r.retry_after_ms.is_some_and(|ms| ms > 0),
        "the 4th failure names a wait: {r:?}"
    );
    let r = release(&c, "jsmith", "not it", "clinical").await;
    assert_eq!(r.outcome, "waiting");
    assert!(r.retry_after_ms.is_some_and(|ms| ms > 0), "{r:?}");
    assert_eq!(
        failures(&c, "jsmith").await,
        4,
        "a waiting try is not a failure"
    );
}

#[tokio::test]
async fn waiting_does_not_count_a_failure() {
    let Some(_) = cs() else {
        eprintln!("skipped");
        return;
    };
    let (_g, c) = fresh().await;
    boss_and_jsmith(&c).await;
    for _ in 0..4 {
        release(&c, "jsmith", "not it", "clinical").await;
    }
    // Push the wait out so the test cannot race the 1-second first back-off.
    c.execute(
        "UPDATE vault_attempt SET next_allowed_at = clock_timestamp() + interval '1 minute' WHERE staff_id = 'jsmith'",
        &[],
    )
    .await
    .unwrap();
    let r = release(&c, "jsmith", CLIN_PASS, "clinical").await;
    assert_eq!(r.outcome, "waiting", "the right passphrase waits too");
    assert_eq!(r.seed, None, "waiting never unseals");
    assert!(r.retry_after_ms.is_some_and(|ms| ms > 0));
    assert_eq!(failures(&c, "jsmith").await, 4);

    c.execute(
        "UPDATE vault_attempt SET next_allowed_at = clock_timestamp() - interval '1 second' WHERE staff_id = 'jsmith'",
        &[],
    )
    .await
    .unwrap();
    let r = release(&c, "jsmith", CLIN_PASS, "clinical").await;
    assert_eq!(r.outcome, "released");
    assert_eq!(
        failures(&c, "jsmith").await,
        0,
        "a success resets the count"
    );
    let next: Option<std::time::SystemTime> = c
        .query_one(
            "SELECT next_allowed_at FROM vault_attempt WHERE staff_id = 'jsmith'",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(next, None);
}

#[tokio::test]
async fn a_superuser_key_is_not_released_for_a_clinical_act() {
    let Some(_) = cs() else {
        eprintln!("skipped");
        return;
    };
    let (_g, c) = fresh().await;
    boss_and_jsmith(&c).await;
    let r = release(&c, "boss", SU_PASS, "clinical").await;
    assert_eq!(r.outcome, "refused-purpose");
    assert_eq!(r.seed, None);
    // The same key IS released for the act a superuser exists for.
    assert_eq!(
        release(&c, "boss", SU_PASS, "enrol").await.outcome,
        "released"
    );
}

#[tokio::test]
async fn a_clinician_key_cannot_enrol() {
    let Some(_) = cs() else {
        eprintln!("skipped");
        return;
    };
    let (_g, c) = fresh().await;
    boss_and_jsmith(&c).await;
    let r = release(&c, "jsmith", CLIN_PASS, "enrol").await;
    assert_eq!(r.outcome, "refused-purpose");
    assert_eq!(r.seed, None);
    // A purpose outside the vocabulary is refused the same way, never raised.
    assert_eq!(
        release(&c, "jsmith", CLIN_PASS, "nonsense").await.outcome,
        "refused-purpose"
    );
}

#[tokio::test]
async fn enrol_needs_a_superuser_token_over_this_request() {
    let Some(_) = cs() else {
        eprintln!("skipped");
        return;
    };
    let (_g, c) = fresh().await;
    boss_and_jsmith(&c).await;

    let seed = test_seed(3);
    let kid = kid_of(seed);

    // A genuine superuser token, but over a DIFFERENT staff ID: refused, and the message never
    // echoes the token.
    let other = enrol_token(&c, "someone-else", &kid, "clinician", test_seed(1)).await;
    let e = enrol(&c, "anurse", seed, "clinician", "boss", &other)
        .await
        .expect_err("a token over another request must not enrol");
    let msg = db_message(&e);
    assert!(msg.contains("attestation"), "{msg}");
    assert!(!msg.contains(&hex::encode(&other)[..16]), "{msg}");

    // A token signed by a clinician, naming the clinician as the enroller: refused.
    let by_clinician = enrol_token(&c, "anurse", &kid, "clinician", test_seed(2)).await;
    let e = enrol(&c, "anurse", seed, "clinician", "jsmith", &by_clinician)
        .await
        .expect_err("a clinician cannot enrol");
    assert!(db_message(&e).contains("superuser"), "{}", db_message(&e));

    // The right token enrols, and the registry holds the new key as a human actor.
    let good = enrol_token(&c, "anurse", &kid, "clinician", test_seed(1)).await;
    let actor_id = enrol(&c, "anurse", seed, "clinician", "boss", &good)
        .await
        .unwrap();
    let kind: String = c
        .query_one(
            "SELECT kind FROM actor_current WHERE signing_key_id = $1 AND actor_id = $2",
            &[&kid, &actor_id],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(kind, "human");
}

#[tokio::test]
async fn the_runtime_role_cannot_read_the_vault() {
    let Some(_) = cs() else {
        eprintln!("skipped");
        return;
    };
    let (_g, c) = fresh().await;
    let (_, jsmith) = boss_and_jsmith(&c).await;
    c.batch_execute("SET ROLE cairn_agent").await.unwrap();

    let e = c
        .query("SELECT sealed FROM vault_member", &[])
        .await
        .expect_err("the runtime role must not read wrapped keys");
    assert_eq!(
        e.code(),
        Some(&tokio_postgres::error::SqlState::INSUFFICIENT_PRIVILEGE)
    );

    // Nor may it call the unwrapping primitive directly and skip the back-off.
    let e = c
        .query("SELECT cairn_vault_unseal('\\x00'::bytea, 'x')", &[])
        .await
        .expect_err("cairn_vault_release is the only door to cairn_vault_unseal");
    assert_eq!(
        e.code(),
        Some(&tokio_postgres::error::SqlState::INSUFFICIENT_PRIVILEGE)
    );

    // The door itself still works under that role.
    let r = release(&c, "jsmith", CLIN_PASS, "clinical").await;
    assert_eq!(r.outcome, "released");
    assert_eq!(r.signing_key_id.as_deref(), Some(jsmith.as_str()));
    c.batch_execute("RESET ROLE").await.unwrap();
}

/// Postgres's own CHECK error carries "Failing row contains (…)", every column printed — the
/// sealed blob included. The vault's insert re-raises with the constraint's name only.
#[tokio::test]
async fn a_refused_enrolment_never_echoes_the_sealed_blob() {
    let Some(_) = cs() else {
        eprintln!("skipped");
        return;
    };
    let (_g, c) = fresh().await;
    let seed = test_seed(4);
    let blob = sealed_blob(seed, SU_PASS);
    let e = c
        .query_one(
            "SELECT cairn_vault_bootstrap($1, $2, $3::text::jsonb, $4, $5, $6, $7, $8)",
            &[
                &"boss",
                &"Site Superuser",
                &r#"{"role":"superuser","staff_id":"boss"}"#,
                &kid_of(seed),
                &blob,
                &None::<&str>,
                &None::<&str>, // no phone and no email: vault_member_needs_a_contact
                &None::<&str>,
            ],
        )
        .await
        .expect_err("a member nobody can reach must be refused");
    let d = e.as_db_error().expect("a database refusal");
    let said = format!("{} {:?} {:?}", d.message(), d.detail(), d.hint());
    assert!(said.contains("vault_member_needs_a_contact"), "{said}");
    assert!(!said.contains(&hex::encode(&blob)[..16]), "{said}");
    assert_eq!(d.detail(), None, "{said}");
}
