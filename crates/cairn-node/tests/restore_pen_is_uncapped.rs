//! DR slice 2d, design test 7 (#593): **a restore's quarantine pen is not silently capped — at a
//! volume ABOVE the cap, in rows AND in bytes.**
//!
//! # The failure this pins
//!
//! The quarantine pen exists for SYNC, where it bounds a hostile or broken peer: at most
//! `MAX_QUARANTINE_ROWS_PER_PEER` (10 000) unacked rows and `MAX_QUARANTINE_BYTES_PER_PEER`
//! (64 MiB) per peer, after which it refuses to grow and says *"the watermark freezes instead
//! (delayed, never lost)"*. That promise needs a cursor to freeze and a peer that will re-serve
//! the bytes.
//!
//! A restore has neither. Every refusal is penned under the one sentinel peer `(restore)`, there
//! is no peer to re-serve anything, and `finalize_identity` fences the node as soon as the restore
//! ends. So a restore that inherited the sync quota would lose record 10 001 and every one after
//! it — with its KEY — at exit. Under born-sealed bodies (ADR-0052) the no-export path pens
//! essentially the whole clinical log, so the pen is as large as the clinic's sealed history — and
//! the design benchmarks this slice at 100 000 events (§6.2; #512 sets the ten-minute budget, not
//! the count), ten times the row cap. This is not a corner. db/052 therefore takes the quota as
//! caller-supplied policy, and the restore passes `NULL, NULL` (unbounded) and REPORTS instead
//! (design §6.2).
//!
//! # Why the volume IS the test
//!
//! The SQL mirror (`db/tests/052_restore_doors_test.sql`) pins the door's unbounded arm. It cannot
//! pin that the RESTORE passes it: a restore that penned a handful of records passes against the
//! capped build too, which is exactly how this defect would ship green. The design says so in as
//! many words — *"at a handful of events it passes against the unfixed quota and proves nothing."*
//!
//! The quota has TWO halves and each is its own way to lose records, so the fixture crosses both:
//! 10 001 records, each padded so that together they hold more than 64 MiB of signed bytes. A
//! fixture of small records crosses only the row cap, and a `pen()` that bounded bytes alone would
//! pass against it.
//!
//! **Mutations this test kills:** `restore::clinical::pen` passing any quota in place of
//! `NULL, NULL` — `ORDINARY_QUOTA_ROWS` (the 10 001st pen raises), `ORDINARY_QUOTA_BYTES` (the pen
//! that crosses 64 MiB raises), or both.
//!
//! # Why these records never touch `event_log`
//!
//! With no custody key installed, a record that carries a wrapped DEK is refused IN RUST, before
//! the apply door is called (`RefusalCause::NoCustodyKey`). So the fixture needs only 10 001 real,
//! distinct, signed, sealed records with a DEK wrapped to this node — not 10 001 admitted charts,
//! which would cost minutes of registration for nothing the assertion reads.

use cairn_event::{sign, Hlc};
use cairn_medium::MediumRecord;
use cairn_node::db;
use cairn_node::restore::clinical::{
    apply_clinical_plane, ORDINARY_QUOTA_BYTES, ORDINARY_QUOTA_ROWS, RESTORE_PEER_SENTINEL,
};
use std::collections::HashMap;
use std::path::PathBuf;
use uuid::Uuid;

mod common;

#[path = "common/restore_kit.rs"]
mod restore_kit;
use restore_kit::{cs, fixture_unwrap_secret, provisioned_clinic, sealed_assert_body};

/// One more record than a sync peer is allowed to have penned. Built from the mirror constant
/// rather than a literal, and [`the_restores_quota_mirror_matches_cairn_syncs_quota`] keeps that
/// mirror equal to the real cap, so the fixture cannot quietly fall below it if the cap is raised.
const OVER_THE_CAP: usize = ORDINARY_QUOTA_ROWS + 1;

/// Characters of padding in each record's sealed twin. The ciphertext is hex-encoded, so each one
/// costs two signed bytes: 10 001 records at ~7 KB apiece is ~70 MiB, over the 64 MiB byte cap.
/// The fixture asserts the total rather than trusting this arithmetic.
const TWIN_PADDING: usize = 3_500;

/// Build `n` distinct records exactly as a medium carries a sealed clinical event: signed bytes,
/// no attestation, and a DEK wrapped to this node's unwrap key.
///
/// The DEK is wrapped to the node's REAL public key, although no custody key is installed for this
/// run, so that each pen row holds exactly what a real no-export restore would hold. The
/// assertions only need a DEK to be present; realism is what makes the rows worth looking at when
/// this test fails.
fn sealed_records_carrying_custody(
    sk: &cairn_event::SigningKey,
    kid: &str,
    n: usize,
) -> Vec<MediumRecord> {
    let node_public = cairn_event::seal::unwrap_public(&fixture_unwrap_secret(sk));
    let twin = format!("penned {}", "x".repeat(TWIN_PADDING));
    (0..n)
        .map(|i| {
            let event_id = Uuid::now_v7().to_string();
            let hlc = Hlc {
                wall: 1_000 + i as i64,
                counter: 0,
                node_origin: "dead-clinic".into(),
            };
            let (body, dek) = sealed_assert_body(kid, Uuid::now_v7(), &event_id, &twin, hlc);
            MediumRecord {
                signed_bytes: sign(&body, sk).unwrap().signed_bytes,
                attestation: None,
                attester_key: None,
                dek_wrapped: Some(cairn_event::seal::wrap_dek_for(&dek, &node_public).unwrap()),
                source_seq: i as i64 + 1,
            }
        })
        .collect()
}

/// How many pen rows hold the SAME wrapped DEK the fixture record at their `refused_seq` carried.
/// **Pure.**
///
/// Stronger than counting non-NULL `dek_wrapped`: a pen that stored some other record's key, or a
/// truncated one, would count as "held with a key" and still lose the body.
fn rows_holding_their_own_key(fixture: &[MediumRecord], pen: &[(i64, Option<Vec<u8>>)]) -> usize {
    let expected: HashMap<i64, &Vec<u8>> = fixture
        .iter()
        .filter_map(|r| r.dek_wrapped.as_ref().map(|dek| (r.source_seq, dek)))
        .collect();
    pen.iter()
        .filter(|(seq, dek)| {
            dek.as_ref()
                .is_some_and(|held| expected.get(seq) == Some(&held))
        })
        .count()
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
    let fixture_bytes: usize = records.iter().map(|r| r.signed_bytes.len()).sum();
    assert!(
        fixture_bytes > ORDINARY_QUOTA_BYTES,
        "anti-vacuity: the fixture must cross the BYTE cap too ({fixture_bytes} bytes), or a pen \
         bounded by bytes alone passes — raise TWIN_PADDING"
    );

    // No custody key installed — the no-export path, which is every unattended cron run.
    let result = apply_clinical_plane(&c, &records, None).await;

    // Read and cleaned up BEFORE asserting, so a failure here cannot leave 10 000 rows under
    // `(restore)` in a shared test database for a later suite to trip over.
    let pen: Vec<(i64, Option<Vec<u8>>)> = c
        .query(
            "SELECT refused_seq, dek_wrapped FROM sync_quarantine WHERE peer = $1",
            &[&RESTORE_PEER_SENTINEL],
        )
        .await
        .unwrap()
        .iter()
        .map(|row| (row.get(0), row.get(1)))
        .collect();
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
        pen.len(),
        OVER_THE_CAP,
        "and every one must genuinely be IN the pen — record {OVER_THE_CAP} is the one a capped \
         pen loses"
    );
    assert_eq!(
        rows_holding_their_own_key(&records, &pen),
        OVER_THE_CAP,
        "each held WITH its own wrapped DEK: on a restored solo node the pen row is the last copy \
         of that key"
    );
    assert!(
        report.penned() > ORDINARY_QUOTA_ROWS && report.penned_bytes > ORDINARY_QUOTA_BYTES,
        "and the report counts past BOTH halves of the quota: {} rows, {} bytes",
        report.penned(),
        report.penned_bytes
    );
    assert!(
        report.exceeds_ordinary_quota(),
        "and 'unbounded' must not mean 'unreported' — the summary's disk-cost note keys on this \
         (that the note itself prints is not pinned at CLI level: #599)"
    );
}

/// The value of `const <name>: i64 = <expr>;` in Rust `source`, where `<expr>` is integer literals
/// joined by `*` (`10_000`, `64 * 1024 * 1024`). **Pure.** `None` if the declaration is absent or
/// has any other shape, so a rewrite of the constant fails the guard loudly instead of reading
/// as a different number.
fn product_const(source: &str, name: &str) -> Option<usize> {
    let prefix = format!("const {name}: i64 = ");
    let line = source
        .lines()
        .find(|l| l.trim_start().starts_with(&prefix))?;
    let expr = line.trim_start().strip_prefix(&prefix)?.strip_suffix(';')?;
    expr.split('*')
        .map(|factor| factor.trim().replace('_', "").parse::<usize>().ok())
        .product()
}

/// **The restore's quota mirror equals `cairn-sync`'s real quota.**
///
/// `restore::clinical` copies the two numbers because `cairn-sync` is a binary-only crate this one
/// cannot depend on. They feed only a report, never a decision, so drift would cost an inaccurate
/// sentence — except here: test 7 builds its "over the cap" fixture from the mirror, so a raised
/// real cap with a stale mirror would leave that fixture BELOW the cap, and the test green while
/// proving nothing. Reading the source is the only way across the crate wall
/// (`attachment_reference_shape.rs` does the same for cairn-sync's migration subset).
#[test]
fn the_restores_quota_mirror_matches_cairn_syncs_quota() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../cairn-sync/src/main.rs")
        .canonicalize()
        .expect("cairn-sync/src/main.rs exists");
    let source = std::fs::read_to_string(path).expect("read cairn-sync/src/main.rs");

    for (name, mirror) in [
        ("MAX_QUARANTINE_ROWS_PER_PEER", ORDINARY_QUOTA_ROWS),
        ("MAX_QUARANTINE_BYTES_PER_PEER", ORDINARY_QUOTA_BYTES),
    ] {
        let real = product_const(&source, name).unwrap_or_else(|| {
            panic!(
                "cairn-sync's `const {name}: i64 = …;` was not found as integer literals joined by \
                 `*` — update this guard to read its new shape, then compare it again"
            )
        });
        assert_eq!(
            mirror, real,
            "restore::clinical's mirror of cairn-sync's {name} has drifted: {mirror} vs {real}"
        );
    }
}

/// The parser the guard above depends on reads both shapes the real constants use, and refuses
/// anything else rather than guessing.
#[test]
fn product_const_reads_a_literal_and_a_product_and_nothing_else() {
    let source =
        "const A: i64 = 10_000;\n    const B: i64 = 64 * 1024 * 1024;\nconst C: i64 = x + 1;";
    assert_eq!(product_const(source, "A"), Some(10_000));
    assert_eq!(product_const(source, "B"), Some(64 * 1024 * 1024));
    assert_eq!(
        product_const(source, "C"),
        None,
        "not a product of literals"
    );
    assert_eq!(product_const(source, "D"), None, "absent");
}
