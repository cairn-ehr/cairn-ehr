//! ADR-0066, the RESTORE half — "identity dies with the disk; custody must not".
//!
//! A solo clinic's disk dies. `cairn-node restore` deliberately mints a **fresh** signing
//! identity (ADR-0026 decision 4) because the old signing seed was never backed up. Until
//! ADR-0066 the node's X25519 DEK-unwrap key was HKDF-derived from that seed, so the
//! restored node could not open a single one of its own born-sealed bodies. Decision 4's
//! answer is **adoption**: the export carries the dead node's independent unwrap secret,
//! and restore INSTALLS it — re-sealed under the restored node's own new operator secrets —
//! rather than minting one. It must adopt rather than mint because `node_unwrap_key` is a
//! singleton whose registrar refuses a differing key.
//!
//! # What each test in here is, so nobody mis-reads one
//!
//! - [`a_restored_node_opens_a_pre_restore_sealed_body`] is a **PROMISE TEST, not a TDD
//!   driver.** It passed the moment it was written, because every piece it composes already
//!   existed. That is deliberate and it is not vacuous: it states the slice's whole claim
//!   end-to-end over the real production functions, so it reddens if any later change breaks
//!   the chain (a rotated derivation, a re-sealed-under-the-wrong-secret install, a keystore
//!   format drift). A test that can only ever be green is vacuous; this one has a failure
//!   mode, it simply does not have one *today*.
//! - Every other test here **was** TDD-driven: each named behaviour did not exist and each
//!   failed first for the reason its name gives.
//!
//! # Anti-vacuity
//!
//! The DB fixture is deliberately NON-EMPTY: a real wrapped custody row exists before the
//! export is read, so "the restore installed nothing" cannot pass for "the restore worked".
//! Every byte of key material is produced by a PRODUCTION function — `generate_unwrap_sealed`
//! mints the dead node's key, `seal_event_payload` mints the DEK, the database's own
//! `cairn_wrap_dek` wraps it — never hand-written in a test (house rule 6).
//!
//! DB-gated tests need `CAIRN_TEST_PG`; the offline ones always run.

use cairn_node::db;
use cairn_node::localstate::{
    apply_local_state, build_export_container, establish_lsk, from_cbor, parse_container,
    read_local_state, recovered_unwrap_secret, unseal_local_state_rec, CustodyKeyDestination,
    LocalState,
};
use tempfile::tempdir;
use uuid::Uuid;

fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

/// The operator secrets of the node that DIED. Passphrases, not key material (they feed
/// Argon2), so a literal is correct here — house rule 6 governs keys, seeds, nonces and IVs.
const DEAD_OP: &str = "op-passphrase-of-the-node-that-died";
const DEAD_REC: &str = "recovery-code-of-the-node-that-died";
/// The operator secrets the RESTORED node mints for itself. Different on purpose: the whole
/// point of adoption is that the same custody secret ends up sealed under NEW secrets.
const NEW_OP: &str = "op-passphrase-of-the-restored-node";
const NEW_REC: &str = "recovery-code-of-the-restored-node";

// ---------------------------------------------------------------------------------------
// The promise
// ---------------------------------------------------------------------------------------

/// **PROMISE TEST** (see the module header): a node restored under a FRESH signing identity
/// can still open a body sealed before the disaster.
///
/// It composes only production functions, in the order restore composes them: the dead node
/// mints an independent unwrap key and wraps a DEK to it; the restored node writes that same
/// secret into its own keystore under its own new operator secrets; the DEK opens.
#[test]
fn a_restored_node_opens_a_pre_restore_sealed_body() {
    let dir = tempdir().unwrap();

    // The dead node: an independent unwrap key, and a DEK wrapped to it.
    let dead_key = dir.path().join("dead.unwrap");
    let dead_pub =
        cairn_node::keystore::generate_unwrap_sealed(&dead_key, DEAD_OP, DEAD_REC).unwrap();
    // Runtime-derived, never a literal (house rule 6 / CodeQL rust/hard-coded-cryptographic-value).
    let dek: [u8; 32] = std::array::from_fn(|i| (i as u8).wrapping_mul(11).wrapping_add(5));
    let wrapped = cairn_event::seal::wrap_dek_for(&dek, &dead_pub).unwrap();
    let dead_secret = cairn_node::keystore::load_unwrap_secret(&dead_key, Some(DEAD_OP)).unwrap();

    // The restored node: a DIFFERENT signing identity (ADR-0026 decision 4 mints one), and
    // the dead node's unwrap secret installed from the export under the NEW secrets.
    //
    // This goes through `CustodyKeyDestination::install` — the production install path — and
    // NOT through `keystore::write_unwrap_sealed` beneath it. That distinction is the whole
    // value of the test: calling the lower function would leave this green even if the
    // restore path were reverted to a no-op, and the header's claim that it catches "an
    // install re-sealed under the wrong secret" would be a claim about code it never runs.
    let restored_key = dir.path().join("restored.unwrap");
    CustodyKeyDestination::Sealed {
        path: &restored_key,
        op_pass: NEW_OP,
        recovery_code: NEW_REC,
    }
    .install(&dead_secret)
    .unwrap();
    let restored_secret =
        cairn_node::keystore::load_unwrap_secret(&restored_key, Some(NEW_OP)).unwrap();

    assert_eq!(
        cairn_event::seal::unwrap_dek(&wrapped, &restored_secret)
            .expect("ADR-0066: a restored node must open custody it inherited")
            .as_slice(),
        &dek,
        "identity died with the disk; custody did not"
    );
}

// ---------------------------------------------------------------------------------------
// The pure validator — refusing a secret that cannot be a key
// ---------------------------------------------------------------------------------------

/// The happy direction FIRST, so the refusal below cannot pass for the wrong reason (a
/// validator that refused everything would satisfy the refusal test on its own).
#[test]
fn a_well_formed_recovered_secret_is_returned_verbatim() {
    let dir = tempdir().unwrap();
    let key = dir.path().join("dead.unwrap");
    cairn_node::keystore::generate_unwrap_sealed(&key, DEAD_OP, DEAD_REC).unwrap();
    let secret = cairn_node::keystore::load_unwrap_secret(&key, Some(DEAD_OP)).unwrap();

    // Field-by-field rather than `..LocalState::empty()`: the struct implements `Drop` (it
    // wipes the secret), and Rust forbids functional-update syntax on such a type.
    let mut bundle = LocalState::empty();
    bundle.unwrap_secret = Some(secret.to_vec());
    let recovered = recovered_unwrap_secret(&bundle)
        .expect("a 32-byte secret is exactly what the slot is for")
        .expect("…and it must come back, not be silently dropped");
    assert_eq!(
        *recovered, *secret,
        "the recovered secret must be the dead node's, byte for byte — a re-derivation or a \
         truncation here opens nothing"
    );

    // And absence is a legitimate answer, distinct from malformed: a pre-ADR-0066 export.
    assert!(
        recovered_unwrap_secret(&LocalState::empty())
            .expect("an absent secret is not an error")
            .is_none(),
        "an export written before ADR-0066 carries no secret; that is a warning, not a refusal"
    );
}

/// A slot that is not 32 bytes must be REFUSED, never installed. Installing it would write a
/// keystore file that opens nothing and register a public half derived from garbage — and
/// `node_unwrap_key`'s registrar would then refuse the real key forever after.
#[test]
fn a_recovered_secret_that_is_not_32_bytes_is_refused() {
    let dir = tempdir().unwrap();
    let key = dir.path().join("dead.unwrap");
    cairn_node::keystore::generate_unwrap_sealed(&key, DEAD_OP, DEAD_REC).unwrap();
    let secret = cairn_node::keystore::load_unwrap_secret(&key, Some(DEAD_OP)).unwrap();

    // Derived from real material by truncation rather than written out, so no byte literal
    // ever appears in a cryptographic position (house rule 6).
    let mut truncated = secret.to_vec();
    truncated.pop();
    let mut bundle = LocalState::empty();
    bundle.unwrap_secret = Some(truncated);

    let err = recovered_unwrap_secret(&bundle)
        .expect_err("31 bytes cannot be an X25519 secret — refuse it")
        .to_string();
    assert!(
        err.contains("31") && err.contains("32"),
        "the refusal must say what it got and what it needed, or an operator cannot act on \
         it: {err}"
    );
}

// ---------------------------------------------------------------------------------------
// The at-rest posture of the installed key
// ---------------------------------------------------------------------------------------

/// The inherited key follows the RESTORED node's at-rest posture — sealed beside a sealed
/// signing key, plaintext beside a `--insecure-plaintext` one — and is proven readable back
/// before the caller is allowed to register its public half.
#[test]
fn the_custody_destination_writes_at_the_restored_nodes_own_posture() {
    let dir = tempdir().unwrap();
    let source = dir.path().join("source.unwrap");
    cairn_node::keystore::generate_unwrap_sealed(&source, DEAD_OP, DEAD_REC).unwrap();
    let secret = cairn_node::keystore::load_unwrap_secret(&source, Some(DEAD_OP)).unwrap();

    // Sealed: readable under the NEW operator passphrase, and under the NEW recovery code.
    let sealed_at = dir.path().join("sealed-node.key.unwrap");
    CustodyKeyDestination::Sealed {
        path: &sealed_at,
        op_pass: NEW_OP,
        recovery_code: NEW_REC,
    }
    .install(&secret)
    .expect("installing under the restored node's own secrets must succeed");
    assert_eq!(
        *cairn_node::keystore::load_unwrap_secret(&sealed_at, Some(NEW_OP)).unwrap(),
        *secret,
        "the operator passphrase of the RESTORED node must open the inherited key"
    );
    assert_eq!(
        *cairn_node::keystore::load_unwrap_secret(&sealed_at, Some(NEW_REC)).unwrap(),
        *secret,
        "so must its recovery code — a restored node with only one live recipient has half \
         an escrow and does not know it"
    );
    assert!(
        cairn_node::keystore::load_unwrap_secret(&sealed_at, Some(DEAD_OP)).is_err(),
        "the DEAD node's passphrase must NOT open it: the point of re-sealing is that the \
         operator carries one set of secrets forward, not the dead disk's"
    );

    // Plaintext: the `restore --insecure-plaintext` posture.
    let plain_at = dir.path().join("plain-node.key.unwrap");
    CustodyKeyDestination::Plaintext { path: &plain_at }
        .install(&secret)
        .expect("a plaintext-provisioned node must still inherit custody");
    assert_eq!(
        *cairn_node::keystore::load_unwrap_secret(&plain_at, None).unwrap(),
        *secret,
        "an unsealed inherited key must read back with no secret at all"
    );
}

// ---------------------------------------------------------------------------------------
// The whole restore-side loop, against a real database
// ---------------------------------------------------------------------------------------

/// Stage the database exactly as the dying node left it: an independent unwrap key
/// registered, and one real wrapped custody row. Returns the dead node's keystore path and
/// the secret an operator would have loaded from it.
///
/// The custody row is INSERTED here rather than authored through the strict clinical door.
/// That is a deliberate, narrow exception, and the reasons are worth stating because the
/// house default is the opposite: (a) `event_dek` has no foreign key to `event_log`, so the
/// row is well-formed on its own; (b) the door path is already covered end-to-end by
/// `dr_clinical_guarantee_gap.rs`, which is the EXPORT side's suite; (c) this suite is about
/// the RESTORE side, and dragging the whole medication fixture in would buy no additional
/// coverage of it while multiplying the runtime. The DEK and the wrap are still produced by
/// production code — `seal_event_payload` mints the DEK, and the row is wrapped by the
/// database's own `cairn_wrap_dek`, the same function `submit_event` calls.
async fn dead_node_with_one_custody_row(
    c: &tokio_postgres::Client,
    dir: &std::path::Path,
) -> (std::path::PathBuf, zeroize::Zeroizing<[u8; 32]>) {
    c.batch_execute("TRUNCATE node_unwrap_key, event_dek, erasure_shred_log CASCADE")
        .await
        .expect("start from a node holding no custody at all");

    let dead_key = dir.join("dead-node.key.unwrap");
    let dead_pub = cairn_node::keystore::generate_unwrap_sealed(&dead_key, DEAD_OP, DEAD_REC)
        .expect("the dead node provisions its independent custody key");
    c.execute(
        "SELECT cairn_register_unwrap_key($1)",
        &[&dead_pub.as_slice()],
    )
    .await
    .expect("…and registers its public half, as `init` does");

    let event_id = Uuid::now_v7().to_string();
    let (_container, dek) = cairn_event::seal::seal_event_payload(
        &serde_json::json!({"substance": {"term": "amoxicillin"}}),
        "amoxicillin — asserted",
        &event_id,
    )
    .expect("production sealing mints the DEK");
    c.execute(
        "INSERT INTO event_dek (event_id, dek_wrapped) \
         VALUES ($1::text::uuid, cairn_wrap_dek($2, (SELECT unwrap_pub FROM node_unwrap_key)))",
        &[&event_id, &dek.as_slice()],
    )
    .await
    .expect("one real wrapped custody row, wrapped by the same function the door uses");

    let secret = cairn_node::keystore::load_unwrap_secret(&dead_key, Some(DEAD_OP)).unwrap();
    (dead_key, secret)
}

/// **The restore-side claim.** A bundle exported from the dying node, applied into a fresh
/// database beside a freshly-minted signing key, must leave the restored node holding the
/// dead node's custody key — on disk under its OWN secrets, and registered in its OWN
/// database — and must report the custody rows it carried.
#[tokio::test]
async fn a_restore_installs_and_registers_the_inherited_unwrap_key() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let dir = tempdir().unwrap();
    let (_dead_key, dead_secret) = dead_node_with_one_custody_row(&c, dir.path()).await;

    // The export, through the production producer.
    let bundle = read_local_state(&c, Some(&dead_secret))
        .await
        .expect("the dying node's export must build");
    assert!(
        !bundle.is_empty(),
        "anti-vacuity: the bundle must genuinely carry something, or 'restore installed \
         nothing' would pass for 'restore worked'"
    );
    assert_eq!(
        bundle.episode_deks.len(),
        1,
        "exactly the one custody row staged above"
    );

    // ---- The bundle must TRAVEL, not just exist in memory (review finding I3). ----
    //
    // Everything below is applied to a bundle that has been through the real off-node
    // journey: sealed into a `CAIRNL1` container, written as bytes, parsed back, unsealed
    // with the dead node's recovery code, and CBOR-decoded. Without this hop, nothing in
    // this slice ever serializes a populated `unwrap_secret`, so a serde attribute change
    // that dropped the field would leave every test here green and every real restore
    // silently keyless — this slice's own failure shape, one layer down.
    let dead_lsk = establish_lsk(DEAD_OP, DEAD_REC).expect("the dying node's escrow");
    let container = build_export_container(&dead_lsk, DEAD_OP, &bundle)
        .expect("the export ceremony must seal the populated bundle");
    let parsed = parse_container(&container).expect("the sibling file must parse back");
    let plaintext = zeroize::Zeroizing::new(
        unseal_local_state_rec(&parsed, DEAD_REC)
            .expect("the OLD node's recovery code is what a restore has to hand"),
    );
    let bundle = from_cbor(&plaintext).expect("the travelled bundle must decode");
    assert_eq!(
        bundle.unwrap_secret.as_deref(),
        Some(dead_secret.as_slice()),
        "the secret must survive seal -> bytes -> parse -> unseal -> decode intact; if this \
         reddens, the export carries no key and no restore anywhere can inherit custody"
    );
    assert_eq!(
        bundle.episode_deks.len(),
        1,
        "and so must the custody row it opens"
    );

    // The restore target: a fresh database (no unwrap key registered) and a key path that
    // does not exist yet, because `restore` has just minted the signing key beside it.
    c.batch_execute("TRUNCATE node_unwrap_key")
        .await
        .expect("a restore target database is fresh");
    let new_key = dir.path().join("restored-node.key");
    let new_unwrap = cairn_node::keystore::unwrap_key_path_for(&new_key);
    assert!(!new_unwrap.exists(), "precondition: nothing installed yet");

    let report = apply_local_state(
        &c,
        &bundle,
        &CustodyKeyDestination::Sealed {
            path: &new_unwrap,
            op_pass: NEW_OP,
            recovery_code: NEW_REC,
        },
    )
    .await
    .expect("ADR-0066 decision 4: a restore ADOPTS the exported unwrap key");

    assert_eq!(
        report.unwrap_key_installed(),
        Some(new_unwrap.as_path()),
        "the report must name the file it wrote, because that is what the operator is told"
    );
    assert_eq!(
        report.episode_deks_carried(),
        1,
        "the carried-but-not-yet-applied count must be reported, not swallowed (#500)"
    );

    // Half one: the file. Sealed under the RESTORED node's secrets, opening the DEAD node's
    // custody.
    let installed = cairn_node::keystore::load_unwrap_secret(&new_unwrap, Some(NEW_OP))
        .expect("the restored node must be able to read its own custody key");
    assert_eq!(
        *installed, *dead_secret,
        "it must be the DEAD node's secret — minting a fresh one here is the whole defect \
         ADR-0066 exists to close"
    );

    // Half two: the registration. Each half failing alone is a different silent disaster.
    let registered: Vec<u8> = c
        .query_one("SELECT unwrap_pub FROM node_unwrap_key", &[])
        .await
        .expect("the restored node must have registered a custody key")
        .get(0);
    assert_eq!(
        registered,
        cairn_event::seal::unwrap_public(&installed).to_vec(),
        "the registered public half must match the installed secret, or the node writes \
         events it can never open"
    );

    // And the point of all of it: the inherited key opens the inherited custody.
    let carried = cairn_node::localstate::episode_dek_from_cbor(&bundle.episode_deks[0]).unwrap();
    cairn_event::seal::unwrap_dek(&carried.dek_wrapped, &installed)
        .expect("the restored node must open the custody row it inherited");
}

/// A malformed secret must be refused BEFORE anything is written or registered. Order is the
/// whole point: a keystore file written first and refused second is unrecoverable, because
/// the registrar is only consulted afterwards (`main.rs` records the same reasoning for
/// `establish-unwrap-key`).
#[tokio::test]
async fn a_malformed_recovered_secret_is_refused_before_anything_is_written() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let dir = tempdir().unwrap();
    let (_dead_key, dead_secret) = dead_node_with_one_custody_row(&c, dir.path()).await;

    let mut bundle = read_local_state(&c, Some(&dead_secret)).await.unwrap();
    let mut truncated = bundle.unwrap_secret.take().expect("the export carries one");
    truncated.pop(); // 31 bytes: derived, never a literal
    bundle.unwrap_secret = Some(truncated);

    c.batch_execute("TRUNCATE node_unwrap_key").await.unwrap();
    let new_unwrap =
        cairn_node::keystore::unwrap_key_path_for(&dir.path().join("restored-node.key"));

    let err = apply_local_state(
        &c,
        &bundle,
        &CustodyKeyDestination::Sealed {
            path: &new_unwrap,
            op_pass: NEW_OP,
            recovery_code: NEW_REC,
        },
    )
    .await
    .expect_err("a secret that is not 32 bytes must not be installed silently")
    .to_string();

    assert!(
        err.contains("31") && err.contains("32"),
        "the refusal must be legible to the operator standing at a dead clinic: {err}"
    );
    assert!(
        !new_unwrap.exists(),
        "nothing may be written before the secret is validated — a bad file here is \
         unrecoverable, since the registrar only ever sees the public half afterwards"
    );
    let registered: i64 = c
        .query_one("SELECT count(*) FROM node_unwrap_key", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        registered, 0,
        "and nothing may be registered either: the singleton registrar would then refuse \
         the real key forever"
    );
}

/// The refusal that must NOT be weakened. `apply_local_state` installs what it knows how to
/// install; for a slot it has no home for it still bails loudly rather than dropping the
/// content silently. `drafts` is the stand-in — a bundle from a future node with a draft
/// store this build has never heard of.
#[tokio::test]
async fn apply_local_state_still_refuses_a_slot_this_build_cannot_apply() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let dir = tempdir().unwrap();
    let new_unwrap =
        cairn_node::keystore::unwrap_key_path_for(&dir.path().join("restored-node.key"));

    let mut bundle = LocalState::empty();
    bundle.drafts = vec![b"an unsent note from a node this build is older than".to_vec()];
    let err = apply_local_state(
        &c,
        &bundle,
        &CustodyKeyDestination::Sealed {
            path: &new_unwrap,
            op_pass: NEW_OP,
            recovery_code: NEW_REC,
        },
    )
    .await
    .expect_err(
        "silently discarding recovered content is the failure this whole area exists \
                 to prevent",
    )
    .to_string();
    assert!(
        err.contains("drafts"),
        "the refusal must name the slot, or an operator cannot tell what was withheld: {err}"
    );
}

/// **The registrar refusal, and the state it leaves behind (review finding I3).**
///
/// `apply_local_state` installs the inherited key and THEN registers its public half. That
/// order is right for the ordinary case and its reasoning is written at the call site: a
/// registered public half whose secret is not on disk is unrecoverable, whereas a written
/// file with no registration is fixed by re-running.
///
/// But `cairn_register_unwrap_key` REFUSES a differing key, and that path had no test — every
/// other restore test truncates `node_unwrap_key` first. It is reachable, and by the route
/// this slice's own warnings name as the top hazard: an operator runs `establish-unwrap-key`
/// against the fresh restore target before restoring into it, registering a key derived from
/// the NEW signing seed.
///
/// What this pins is not just "it errors" — a raw `db error` string would satisfy that and
/// tell an operator nothing. It pins that the message explains WHICH of the two disagreeing
/// halves is the real one, and that the file is left in place rather than half-removed,
/// because that file holds the dead node's only custody key.
#[tokio::test]
async fn a_differing_registration_refuses_the_restore_legibly_and_keeps_the_file() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let dir = tempdir().unwrap();
    let (_dead_key, dead_secret) = dead_node_with_one_custody_row(&c, dir.path()).await;
    let bundle = read_local_state(&c, Some(&dead_secret)).await.unwrap();

    // The hazard state: a DIFFERENT key already registered in the restore target. Generated,
    // so it is independent of the dead node's by construction (house rule 6 — the generator,
    // never a literal).
    c.batch_execute("TRUNCATE node_unwrap_key").await.unwrap();
    let interloper = cairn_event::seal::generate_unwrap_secret().unwrap();
    let interloper_pub = cairn_event::seal::unwrap_public(&interloper);
    c.execute(
        "SELECT cairn_register_unwrap_key($1)",
        &[&interloper_pub.as_slice()],
    )
    .await
    .expect("stage the key an ill-timed `establish-unwrap-key` would have registered");

    let new_unwrap =
        cairn_node::keystore::unwrap_key_path_for(&dir.path().join("restored-node.key"));
    let err = apply_local_state(
        &c,
        &bundle,
        &CustodyKeyDestination::Sealed {
            path: &new_unwrap,
            op_pass: NEW_OP,
            recovery_code: NEW_REC,
        },
    )
    .await
    .expect_err("a differing registration must refuse the restore, not be papered over")
    .to_string();

    assert!(
        err.contains("singleton"),
        "the refusal must say WHY it cannot be retried against this database: {err}"
    );
    assert!(
        err.contains("establish-unwrap-key"),
        "and must name the likely cause, which is the only thing that makes it actionable: {err}"
    );
    assert!(
        err.contains(&new_unwrap.display().to_string()),
        "and must name the file that holds the real key: {err}"
    );

    // THE POINT of the second half: the written file is the dead node's REAL custody key and
    // the registration is the wrong one. Deleting the file on this path would destroy the one
    // artifact worth keeping, so it must still be there — and still openable.
    assert!(
        new_unwrap.exists(),
        "the installed file must survive the refusal: it holds the dead node's real key"
    );
    let recovered = cairn_node::keystore::load_unwrap_secret(&new_unwrap, Some(NEW_OP))
        .expect("and must still open under the restore ceremony's own passphrase");
    assert_eq!(
        *recovered, *dead_secret,
        "and must be the DEAD node's secret — the half the operator has to keep"
    );
}

/// **A well-formed key that opens nothing must be refused before it is installed
/// (review finding I4).**
///
/// Every key in this plane is a bare 32-byte array — the X25519 secret half, its public
/// half, the Ed25519 signing seed, a DEK. The compiler cannot tell them apart, and neither
/// can a length check. So the most catastrophic single mistake available here — the PUBLIC
/// half travelling in the secret slot — used to pass `recovered_unwrap_secret`, pass
/// `install`'s read-after-write (which proves only that the file holds the bytes we wrote),
/// and get REGISTERED. `node_unwrap_key` is a singleton, so the real key is then refused
/// forever and the clinic's record is unreadable with every surface reporting success.
///
/// One trial unwrap of one carried DEK separates well-formed from correct. This test stages
/// exactly that substitution.
#[tokio::test]
async fn a_secret_that_opens_no_carried_custody_is_refused_before_installing() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let dir = tempdir().unwrap();
    let (_dead_key, dead_secret) = dead_node_with_one_custody_row(&c, dir.path()).await;

    let mut bundle = read_local_state(&c, Some(&dead_secret)).await.unwrap();
    assert_eq!(
        bundle.episode_deks.len(),
        1,
        "precondition: there must be custody to test the key against, or the check below \
         legitimately does nothing and this test would prove nothing"
    );

    // THE SUBSTITUTION: the PUBLIC half where the secret belongs. Exactly 32 bytes, derived
    // at runtime from real material (house rule 6), and completely useless as a key.
    let public_half = cairn_event::seal::unwrap_public(&dead_secret);
    bundle.unwrap_secret = Some(public_half.to_vec());

    c.batch_execute("TRUNCATE node_unwrap_key").await.unwrap();
    let new_unwrap =
        cairn_node::keystore::unwrap_key_path_for(&dir.path().join("restored-node.key"));

    let err = apply_local_state(
        &c,
        &bundle,
        &CustodyKeyDestination::Sealed {
            path: &new_unwrap,
            op_pass: NEW_OP,
            recovery_code: NEW_REC,
        },
    )
    .await
    .expect_err("a 32-byte value that opens no custody must never be installed")
    .to_string();

    assert!(
        err.contains("does NOT open the custody"),
        "the refusal must say what was actually wrong — not merely that something failed: {err}"
    );
    assert!(
        !new_unwrap.exists(),
        "and nothing may be written: this check exists to run BEFORE the irreversible step"
    );
    let registered: i64 = c
        .query_one("SELECT count(*) FROM node_unwrap_key", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        registered, 0,
        "and nothing registered — the singleton would otherwise foreclose the real key"
    );
}

/// The positive control for the check above: a bundle with no custody rows carries nothing
/// to test the key against, and must still restore.
///
/// Without this, `secret_opens_the_carried_custody` could be implemented as "always refuse"
/// and the test above would still pass — while every node that had not yet written a sealed
/// body became unrestorable.
#[tokio::test]
async fn a_bundle_with_no_custody_rows_still_restores() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let dir = tempdir().unwrap();

    c.batch_execute("TRUNCATE node_unwrap_key, event_dek, erasure_shred_log CASCADE")
        .await
        .unwrap();
    let dead_key = dir.path().join("dead-node.key.unwrap");
    cairn_node::keystore::generate_unwrap_sealed(&dead_key, DEAD_OP, DEAD_REC).unwrap();
    let dead_secret = cairn_node::keystore::load_unwrap_secret(&dead_key, Some(DEAD_OP)).unwrap();

    // A node provisioned but never used clinically: a key, and no custody at all.
    let bundle = read_local_state(&c, Some(&dead_secret)).await.unwrap();
    assert!(
        bundle.episode_deks.is_empty(),
        "precondition: this bundle must carry no custody"
    );

    let new_unwrap =
        cairn_node::keystore::unwrap_key_path_for(&dir.path().join("restored-node.key"));
    let applied = apply_local_state(
        &c,
        &bundle,
        &CustodyKeyDestination::Sealed {
            path: &new_unwrap,
            op_pass: NEW_OP,
            recovery_code: NEW_REC,
        },
    )
    .await
    .expect("a provisioned-but-unused node must still restore its custody key");

    assert_eq!(applied.unwrap_key_installed(), Some(&*new_unwrap));
    assert_eq!(applied.episode_deks_carried(), 0);
}
