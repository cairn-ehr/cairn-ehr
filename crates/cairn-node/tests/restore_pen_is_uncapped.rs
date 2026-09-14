//! DR slice 2d, design test 7 (#593): **a restore's quarantine pen is not silently capped — at a
//! volume ABOVE the cap.**
//!
//! # The failure this pins
//!
//! The quarantine pen exists for SYNC, where it bounds a hostile or broken peer: at most
//! `MAX_QUARANTINE_ROWS_PER_PEER` (10 000) unacked rows per peer, after which it refuses to grow
//! and says *"the watermark freezes instead (delayed, never lost)"*. That promise needs a cursor to
//! freeze and a peer that will re-serve the bytes.
//!
//! A restore has neither. Every refusal is penned under the one sentinel peer `(restore)`, there
//! is no peer to re-serve anything, and `finalize_identity` fences the node as soon as the restore
//! ends. So a restore that inherited the sync quota would lose record 10 001 and every one after
//! it — with its KEY — at exit. Under born-sealed bodies (ADR-0052) the no-export path pens
//! essentially the whole clinical log, and #512's budget scale is 100 000 events, so this is not a
//! corner. db/052 therefore takes the quota as caller-supplied policy, and the restore passes
//! `NULL` (unbounded) and REPORTS instead (design §6.2).
//!
//! # Why the volume IS the test
//!
//! The SQL mirror (`db/tests/052_restore_doors_test.sql`) pins the door's unbounded arm. It cannot
//! pin that the RESTORE passes it: a restore that penned a handful of records passes against the
//! capped build too, which is exactly how this defect would ship green. The design says so in as
//! many words — *"at a handful of events it passes against the unfixed quota and proves nothing."*
//!
//! **Mutation this test kills:** `restore::clinical::pen` passing `ORDINARY_QUOTA_ROWS` /
//! `ORDINARY_QUOTA_BYTES` instead of `NULL, NULL` — the 10 001st pen raises and the run fails.
//!
//! # Why these records never touch `event_log`
//!
//! With no custody key installed, a record that carries a wrapped DEK is refused IN RUST, before
//! the apply door is called (`RefusalCause::NoCustodyKey`). So the fixture needs only 10 001 real,
//! distinct, signed, sealed records with a DEK wrapped to this node — not 10 001 admitted charts,
//! which would cost minutes of registration for nothing the assertion reads.

use cairn_event::{sign, Hlc};
use cairn_node::db;
use cairn_node::restore::clinical::{
    apply_clinical_plane, ORDINARY_QUOTA_ROWS, RESTORE_PEER_SENTINEL,
};
use uuid::Uuid;

mod common;

#[path = "common/restore_kit.rs"]
mod restore_kit;
use restore_kit::{cs, fixture_unwrap_secret, provisioned_clinic, sealed_assert_body};

/// One more record than a sync peer is allowed to have penned. The mirror constant is used
/// rather than a literal so the test cannot quietly fall BELOW the cap if the cap is raised —
/// `restore::clinical`'s doc names `cairn-sync`'s `MAX_QUARANTINE_ROWS_PER_PEER` as its source.
const OVER_THE_CAP: usize = ORDINARY_QUOTA_ROWS + 1;

/// Build `n` distinct records exactly as a medium carries a sealed clinical event: signed bytes,
/// no attestation, and a DEK wrapped to this node's unwrap key.
fn sealed_records_carrying_custody(
    sk: &cairn_event::SigningKey,
    kid: &str,
    n: usize,
) -> Vec<cairn_medium::MediumRecord> {
    let node_public = cairn_event::seal::unwrap_public(&fixture_unwrap_secret(sk));
    (0..n)
        .map(|i| {
            let event_id = Uuid::now_v7().to_string();
            let hlc = Hlc {
                wall: 1_000 + i as i64,
                counter: 0,
                node_origin: "dead-clinic".into(),
            };
            let (body, dek) = sealed_assert_body(kid, Uuid::now_v7(), &event_id, "penned", hlc);
            cairn_medium::MediumRecord {
                signed_bytes: sign(&body, sk).unwrap().signed_bytes,
                attestation: None,
                attester_key: None,
                dek_wrapped: Some(cairn_event::seal::wrap_dek_for(&dek, &node_public).unwrap()),
                source_seq: i as i64 + 1,
            }
        })
        .collect()
}

#[tokio::test]
async fn a_restore_pens_every_refusal_past_the_sync_quota_with_its_key() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    // `provisioned_clinic` leaves an enrolled registry, which is the precondition for ANY record
    // being offered: with none, the restore declines the whole plane before penning anything.
    let (sk, kid) = provisioned_clinic(&c).await;
    c.batch_execute("DELETE FROM sync_quarantine")
        .await
        .unwrap();

    let records = sealed_records_carrying_custody(&sk, &kid, OVER_THE_CAP);

    // No custody key installed — the no-export path, which is every unattended cron run.
    let result = apply_clinical_plane(&c, &records, None).await;

    // Counted and cleaned up BEFORE asserting, so a failure here cannot leave 10 000 rows under
    // `(restore)` in a shared test database for a later suite to trip over.
    let (penned, with_key): (i64, i64) = {
        let row = c
            .query_one(
                "SELECT count(*), count(dek_wrapped) FROM sync_quarantine WHERE peer = $1",
                &[&RESTORE_PEER_SENTINEL],
            )
            .await
            .unwrap();
        (row.get(0), row.get(1))
    };
    c.batch_execute("DELETE FROM sync_quarantine")
        .await
        .unwrap();

    let report = result.expect(
        "a restore must pen past the sync quota rather than fail: the quota's 'delayed, never \
         lost' promise needs a peer that re-serves, and a restore has none",
    );
    assert_eq!(
        report.penned(),
        OVER_THE_CAP,
        "every refusal must be counted: {:?}",
        report.refusals
    );
    assert_eq!(
        penned, OVER_THE_CAP as i64,
        "and every one must genuinely be IN the pen — record {OVER_THE_CAP} is the one a capped \
         pen loses"
    );
    assert_eq!(
        with_key, penned,
        "each held WITH its wrapped DEK: on a restored solo node the pen row is the last copy of \
         that key"
    );
    assert!(
        report.exceeds_ordinary_quota(),
        "and 'unbounded' must not mean 'unreported' — the summary's disk-cost note keys on this"
    );
}
