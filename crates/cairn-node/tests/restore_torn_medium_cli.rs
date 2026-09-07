//! #500 slice 2c review round 3 (testing gap): the round-2 ruling — `restore` must NOT
//! refuse a torn CAIRNB3 medium, unlike `verify-backup` — has NO test that drives it
//! through `main.rs` itself. Every existing test calls `cairn_node::restore`'s library
//! functions directly, so re-adding `anyhow::bail!("refusing to restore a torn medium")`
//! to the `Cmd::Restore` arm would pass the entire suite today. This file closes that gap
//! the way `tests/cli_localstate.rs` already does for two other `main.rs`-only fixes:
//! spawn the real `cairn-node` binary (`CARGO_BIN_EXE_cairn-node`, no extra test
//! dependency) so the orchestration in `main.rs` — not just its ingredients — is what
//! gets exercised.

use cairn_event::{sign, EventBody, Hlc, SigningKey};
use cairn_medium::{
    append_segment, build_segment_attestation, serialize_v3, MediumRecord, Plane, Segment,
};
use cairn_node::{db, identity};
use std::process::Command;

/// A `Command` for the freshly-built `cairn-node` binary under test.
fn cairn_node() -> Command {
    Command::new(env!("CARGO_BIN_EXE_cairn-node"))
}

/// Mint a real signed `node.enrolled` event for an arbitrary key (no DB). Mirrors
/// `tests/restore.rs`'s own `synth_enroll` — duplicated rather than shared, since
/// integration-test binaries in this crate cannot `use` another test binary's private
/// helpers and this is a per-suite need, not a cross-suite one (the same reasoning
/// `tests/capture_loop.rs`'s header already gives for its own copied fixtures).
fn synth_enroll(sk: &SigningKey, name: &str) -> Vec<u8> {
    let kid = hex::encode(sk.verifying_key().to_bytes());
    let body = EventBody {
        event_id: uuid::Uuid::now_v7().to_string(),
        patient_id: identity::NIL_PATIENT.into(),
        event_type: "node.enrolled".into(),
        schema_version: "node/1".into(),
        hlc: Hlc {
            wall: 1,
            counter: 0,
            node_origin: name.into(),
        },
        t_effective: None,
        signer_key_id: kid,
        contributors: serde_json::json!([]),
        payload: serde_json::json!({ "display_name": name, "address": "127.0.0.1:7999" }),
        attachments: vec![],
        plaintext_twin: None,
        clock_grade: cairn_event::ClockGrade::SelfAsserted,
        safety: None,
    };
    sign(&body, sk).unwrap().signed_bytes
}

/// Build a torn CAIRNB3 medium: one COMPLETE, self-identifying Node-plane segment (a
/// real genesis, really attested), followed by a second segment cut short mid-append.
fn torn_v3_medium() -> Vec<u8> {
    let sk = cairn_event::generate_key().unwrap().0;
    let kid = hex::encode(sk.verifying_key().to_bytes());
    let enroll = synth_enroll(&sk, "Self");
    let self_id = hex::encode(cairn_event::event_address(&enroll));

    let records = vec![MediumRecord {
        signed_bytes: enroll.clone(),
        attestation: None,
        attester_key: None,
        dek_wrapped: None,
        source_seq: 0,
    }];
    let attestation = build_segment_attestation(&sk, &kid, &self_id, Plane::Node, 0, "", &records);
    let complete = Segment {
        plane: Plane::Node,
        index: 0,
        prev_commitment: String::new(),
        self_node_id_hex: self_id,
        attestation: Some(attestation),
        records,
    };

    let mut bytes = serialize_v3(std::slice::from_ref(&complete)).unwrap();
    let intact = bytes.len();

    // A second, well-formed segment that never finishes landing — the torn tail. Its
    // plane/contents are irrelevant: it must simply be present-but-cut, so `parse_any`
    // reports `truncated_tail` and drops it from `MediumV3::segments` entirely.
    let torn = Segment {
        plane: Plane::Clinical,
        index: 1,
        prev_commitment: cairn_medium::segment_commitment(&complete.records),
        self_node_id_hex: String::new(),
        attestation: None,
        records: vec![
            MediumRecord {
                signed_bytes: vec![9u8; 40],
                attestation: None,
                attester_key: None,
                dek_wrapped: None,
                source_seq: 0,
            },
            MediumRecord {
                signed_bytes: vec![8u8; 40],
                attestation: None,
                attester_key: None,
                dek_wrapped: None,
                source_seq: 1,
            },
        ],
    };
    append_segment(&mut bytes, &torn).unwrap();
    bytes.truncate(intact + 12); // a crash partway through the second section — torn

    bytes
}

/// THE test that pins the round-2 ruling itself, not merely its ingredients: `restore`
/// run as a real process against a torn CAIRNB3 medium must exit ZERO, apply the
/// COMPLETE prefix (here: exactly the one genesis event before the tear), and print a
/// loud warning naming the tear — never silently, and never by refusing.
#[tokio::test]
async fn restore_recovers_the_prefix_of_a_torn_medium_and_warns() {
    let Some(base) = std::env::var("CAIRN_TEST_PG").ok() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    // Fresh, un-enrolled DB: restore's own precondition (it fences closed on a live node).
    let a = db::connect_and_load_schema(&base).await.unwrap();
    db::reset_node_federation_tables(&a).await.ok();

    let medium_bytes = torn_v3_medium();
    let dir = tempfile::tempdir().unwrap();
    let medium_path = dir.path().join("torn.medium");
    std::fs::write(&medium_path, &medium_bytes).unwrap();
    let key_path = dir.path().join("new.key");

    let out = cairn_node()
        .args(["--conn", &base, "--key"])
        .arg(&key_path)
        .args(["restore", "--from"])
        .arg(&medium_path)
        .args(["--insecure-plaintext"])
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "restore must exit 0 on a torn medium (recover the prefix, never refuse); \
         stdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        stderr.contains("TORN"),
        "restore must warn LOUDLY that the medium was torn; stderr:\n{stderr}"
    );
    assert!(
        stdout.contains("restored 1 event(s)"),
        "exactly the one genesis before the tear must be applied — the torn segment's \
         records must not appear; stdout:\n{stdout}"
    );
    assert!(
        stdout.to_lowercase().contains("torn"),
        "the end-of-run summary must repeat the tear, not just the early WARNING — an \
         operator reading only the tail of a long restore must still see it; \
         stdout:\n{stdout}"
    );

    // And the genesis actually landed in the fresh database (not merely claimed) — the
    // restored (torn-prefix) genesis plus the new node's own, exactly as
    // `restore_round_trip_rehydrates_under_a_new_identity` (tests/restore.rs) pins for an
    // untorn medium.
    let n_enroll: i64 = a
        .query_one("SELECT count(*) FROM node_event WHERE op='enroll'", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        n_enroll, 2,
        "the restored (torn-prefix) genesis plus the new node's own genesis"
    );
}
