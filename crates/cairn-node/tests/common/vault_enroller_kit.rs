//! Shared fixtures for the vault's slice-2 door suites (`vault_enrollers.rs`: grants, enrolment,
//! judging a key; `vault_rekey.rs`: re-key and reseal). One copy, so the two suites build members
//! through the same real doors, with the same runtime-derived keys (house rule 6) and the same
//! passphrase-per-seed rule ([`pass_of`]).
#![allow(dead_code)] // each including suite uses a different subset

use cairn_event::keys::Secret32;
use cairn_event::{generate_key, sign_attestation, SigningKey};
use cairn_node::{db, seal};
use tokio_postgres::types::ToSql;
use tokio_postgres::{Client, Error};

pub fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

/// A deterministic seed built at runtime; `variant` gives each key its own.
pub fn test_seed(variant: u8) -> [u8; 32] {
    std::array::from_fn(|i| (i as u8).wrapping_mul(29).wrapping_add(variant))
}

pub const BOSS: u8 = 1;
pub const JSMITH: u8 = 2;
pub const BWONG: u8 = 3;
pub const ANURSE: u8 = 4;
pub const BNURSE: u8 = 5;
pub const ADMIN: u8 = 6;
pub const JSMITH2: u8 = 7;
pub const JSMITH3: u8 = 8;
pub const BOSS2: u8 = 9;

pub fn kid_of(v: u8) -> String {
    hex::encode(
        SigningKey::from_bytes(&test_seed(v))
            .verifying_key()
            .to_bytes(),
    )
}

/// Every key's passphrase, derived from its seed variant, so a re-key's new passphrase differs.
pub fn pass_of(v: u8) -> String {
    format!("passphrase number {v}")
}

pub fn blob_under(v: u8, passphrase: &str) -> Vec<u8> {
    let sealed = seal::seal(
        &Secret32::from_bytes(test_seed(v)),
        passphrase,
        &seal::generate_recovery_code(),
    )
    .unwrap();
    seal::to_cbor(&sealed)
}

pub fn blob(v: u8) -> Vec<u8> {
    blob_under(v, &pass_of(v))
}

pub fn display(staff: &str) -> String {
    format!("Dr {staff}")
}

pub fn pinned(role: &str, staff: &str) -> String {
    format!(r#"{{"role":"{role}","handle":"{staff}"}}"#)
}

/// An attestation token over `addr` by the key of seed `v` (an approver's act, or a member's proof).
pub fn sign(addr: &[u8], v: u8) -> Vec<u8> {
    sign_attestation(
        addr,
        &kid_of(v),
        "approved",
        &SigningKey::from_bytes(&test_seed(v)),
    )
    .unwrap()
}

pub async fn bytes(c: &Client, sql: &str, params: &[&(dyn ToSql + Sync)]) -> Vec<u8> {
    c.query_one(sql, params).await.unwrap().get(0)
}

pub fn msg(e: &Error) -> String {
    e.as_db_error()
        .map(|d| d.message().to_string())
        .unwrap_or_else(|| e.to_string())
}

/// Serialised, freshly loaded, empty vault and registry, plus this node's device actor and an
/// unwrap key (so a clinical act can be submitted; `vault_lib.rs::fresh`).
pub async fn fresh() -> (Client, Client, SigningKey, String) {
    let base = cs().expect("guarded by caller");
    let guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    c.batch_execute(
        "TRUNCATE vault_attempt, vault_member, vault_terminal, vault_policy, event_log, actor_event, \
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

pub async fn boot(c: &Client) {
    c.execute(
        "SELECT cairn_vault_bootstrap('boss', 'Dr boss', $1::text::jsonb, $2, $3, NULL, '0400', NULL)",
        &[&pinned("superuser", "boss"), &kid_of(BOSS), &blob(BOSS)],
    )
    .await
    .unwrap();
}

/// Enrol `staff` as `role` with key `v`, approved by `approver` signing with key `by`.
pub async fn enrol(
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
pub async fn set_enroller(
    c: &Client,
    staff: &str,
    grant: bool,
    su: &str,
    by: u8,
) -> Result<(), Error> {
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
pub async fn judge(c: &Client, v: u8, confirm: bool, su: &str, by: u8) -> Result<(), Error> {
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
pub async fn rekey(
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
        "SELECT cairn_vault_rekey_address($1, $2, $3)",
        &[&staff, &role, &kid_of(v)],
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
pub async fn release(
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

pub async fn status(c: &Client, v: u8) -> String {
    c.query_one("SELECT cairn_key_confirmation($1)", &[&kid_of(v)])
        .await
        .unwrap()
        .get(0)
}

pub async fn actor_of(c: &Client, staff: &str) -> Vec<u8> {
    bytes(
        c,
        "SELECT actor_id FROM vault_member WHERE staff_id = $1",
        &[&staff],
    )
    .await
}

pub async fn current_key(c: &Client, actor: &[u8]) -> Option<String> {
    c.query_opt(
        "SELECT signing_key_id FROM actor_current WHERE actor_id = $1",
        &[&actor],
    )
    .await
    .unwrap()
    .map(|r| r.get(0))
}

/// boss (bootstrapped), jsmith and bwong (clinicians, enrolled by boss, both granted enroller).
pub async fn staff(c: &Client) {
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
