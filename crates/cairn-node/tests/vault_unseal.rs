//! DB-gated: `cairn_vault_unseal` (cairn_pgx >= 0.4.0) checks a clinician's passphrase INSIDE
//! Postgres, so the passphrase-sealed key blob never has to leave the database.
use cairn_event::keys::Secret32;
use cairn_keystore::seal;
use cairn_node::db;

fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

/// A deterministic 32-byte seed built at runtime (house rule 6: no key literals).
fn test_seed() -> [u8; 32] {
    std::array::from_fn(|i| (i as u8).wrapping_mul(7).wrapping_add(3))
}

fn sealed_blob() -> Vec<u8> {
    let sealed = seal::seal(
        &Secret32::from_bytes(test_seed()),
        "correct horse",
        &seal::generate_recovery_code(),
    )
    .unwrap();
    seal::to_cbor(&sealed)
}

/// Run `cairn_vault_unseal(blob, pass)` and return the (possibly NULL) result.
async fn unseal(blob: &[u8], pass: &str) -> Option<Vec<u8>> {
    let base = cs().expect("guarded by caller");
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    // An Err here (not just a NULL) would fail the "never an error" contract.
    let row = c
        .query_one("SELECT cairn_vault_unseal($1, $2)", &[&blob, &pass])
        .await
        .expect("cairn_vault_unseal must never raise");
    row.get::<_, Option<Vec<u8>>>(0)
}

#[tokio::test]
async fn the_right_passphrase_returns_the_seed() {
    let Some(_) = cs() else {
        eprintln!("skipped");
        return;
    };
    let got = unseal(&sealed_blob(), "correct horse").await;
    assert_eq!(got, Some(test_seed().to_vec()));
}

#[tokio::test]
async fn a_wrong_passphrase_returns_null_not_an_error() {
    let Some(_) = cs() else {
        eprintln!("skipped");
        return;
    };
    assert_eq!(unseal(&sealed_blob(), "wrong horse").await, None);
}

#[tokio::test]
async fn garbage_returns_null_not_an_error() {
    let Some(_) = cs() else {
        eprintln!("skipped");
        return;
    };
    assert_eq!(unseal(&[0u8; 7], "correct horse").await, None);
}
