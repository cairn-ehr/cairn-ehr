//! DR slice 2d, design test 16 (#593): **one event id, one body — through the restore.**
//!
//! A medium can hold more than one record at a single `source_seq`. The derivation
//! (`cairn_medium::plane_records`) collapses a copy only when EVERY field agrees; a copy that
//! differs is kept and handed to the restore, deliberately, so the caller can name it rather than
//! silently keep whichever sorted first. Both suites below exercise what the restore then does,
//! through the real derivation over a real captured medium — never a hand-ordered `Vec`, because
//! the order the derivation produces is part of what is under test.
//!
//! 1. **A re-capture that differs only in custody is a no-op.** The same signed event, written a
//!    second time without its wrapped DEK — the shape a capture that straddled a crypto-shred
//!    leaves. The first copy restores; the second finds the event present and changes nothing.
//! 2. **A different body under the same `event_id` is refused as a substitution.** Two bodies
//!    under one id would leave two nodes holding different bytes for one event forever, with no
//!    alarm (db/020's review H3). The restore must pen the rival with the DOOR's reason and leave
//!    the original as what the chart reads.
//!
//! These are library-level on purpose: the CLI adds nothing to either question, and
//! `restore_cli_surface.rs` already proves the command reaches `apply_clinical_plane`.

use cairn_event::seal::Secret32;
use cairn_event::sign;
use cairn_medium::{parse_any, MediumRecord, Plane};
use cairn_node::restore::clinical::apply_clinical_plane;
use cairn_node::{backup, db, localstate, localstate_read};
use tokio_postgres::Client;

mod common;

#[path = "common/restore_kit.rs"]
mod restore_kit;
use restore_kit::{
    author_sealed_clinical_event, capture, chained_segment, cs, fixture_unwrap_secret,
    in_event_log, in_pen, provisioned_clinic, rewrite_medium, sealed_assert_body, twin_of,
    wipe_to_a_fresh_dr_machine, Authored,
};

/// A dead clinic whose medium carries one sealed chart PLUS whatever `extra` builds from that
/// chart's captured record, appended as a correctly chained segment. Returns the chart, the
/// records a restore is entitled to apply (through the real derivation), and the inherited
/// custody secret — with the replacement machine already holding custody and the registry, as
/// `restore`'s ceremony leaves it before the clinical apply.
async fn restore_ready_with_extra_records(
    c: &Client,
    dir: &std::path::Path,
    extra: impl FnOnce(&MediumRecord, &Authored, &cairn_event::SigningKey, &str) -> Vec<MediumRecord>,
) -> (Authored, Vec<MediumRecord>, Secret32) {
    let (sk, kid) = provisioned_clinic(c).await;
    let chart = author_sealed_clinical_event(c, &sk, &kid).await;
    let medium = capture(c, &sk, &kid, dir).await;

    rewrite_medium(&medium, |segments| {
        let captured = segments
            .iter()
            .filter(|s| s.plane == Plane::Clinical)
            .flat_map(|s| s.records.iter())
            .find(|r| r.signed_bytes == chart.signed_bytes)
            .expect("the capture carries the sealed chart")
            .clone();
        let records = extra(&captured, &chart, &sk, &kid);
        let appended = chained_segment(segments, Plane::Clinical, records);
        segments.push(appended);
    });

    let image = parse_any(&std::fs::read(&medium).unwrap()).unwrap();
    let trusted = backup::clinical_plane_accounting(&image).unwrap();
    assert_eq!(
        trusted.gated_out, 0,
        "anti-vacuity: the appended segment must be CHAINED, or the restore never sees it and \
         this file tests the trust gate instead"
    );

    let secret = fixture_unwrap_secret(&sk);
    let bundle = localstate_read::read_local_state(c, Some(&secret))
        .await
        .expect("the export is readable on the live node");
    wipe_to_a_fresh_dr_machine(c).await;
    let keydir = tempfile::tempdir().unwrap();
    localstate::apply_local_state(
        c,
        &bundle,
        &localstate::CustodyKeyDestination::Plaintext {
            path: &keydir.path().join("restored.key.unwrap"),
        },
    )
    .await
    .expect("custody and the registry install into the fresh database");

    (chart, trusted.records, secret)
}

/// How many of `records` sit at the same `source_seq` as `chart`.
fn copies_at_the_charts_position(records: &[MediumRecord], chart: &Authored) -> usize {
    let position = records
        .iter()
        .find(|r| r.signed_bytes == chart.signed_bytes)
        .expect("the chart is in the trusted set")
        .source_seq;
    records.iter().filter(|r| r.source_seq == position).count()
}

/// **A re-capture that differs only in custody is a no-op.**
///
/// Mutation this kills: `plane_records` collapsing duplicates by `source_seq` alone. That is the
/// "simplification" the derivation's own doc warns against — it silently keeps whichever copy
/// sorted first, which can hand a restore a DEK it cannot open or resurrect a key an erasure
/// destroyed — and it makes the second copy vanish before the restore can account for it.
#[tokio::test]
async fn a_second_copy_without_its_key_at_the_same_position_changes_nothing() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let dir = tempfile::tempdir().unwrap();

    let (chart, records, secret) =
        restore_ready_with_extra_records(&c, dir.path(), |captured, _, _, _| {
            let mut keyless = captured.clone();
            keyless.dek_wrapped = None;
            vec![keyless]
        })
        .await;
    assert_eq!(
        copies_at_the_charts_position(&records, &chart),
        2,
        "anti-vacuity: the derivation must hand BOTH copies to the restore"
    );

    let report = apply_clinical_plane(&c, &records, Some(&secret))
        .await
        .expect("a duplicate position is never a failure of the run");

    assert_eq!(report.penned(), 0, "nothing is refused: {report:?}");
    assert_eq!(
        report.already_present, 1,
        "the second copy must be accounted for as already present — not dropped unseen, and not \
         counted as a second applied record: {report:?}"
    );
    assert_eq!(
        report.applied + report.already_present,
        records.len(),
        "every record the derivation handed over lands in exactly one outcome: {report:?}"
    );
    assert!(in_event_log(&c, &chart.event_id).await);
    assert_eq!(
        twin_of(&c, &chart.event_id).await.as_deref(),
        Some(chart.twin.as_str()),
        "and the keyless second copy did not take the first copy's custody away — the body opens"
    );
}

/// **A different body under the same `event_id` is refused as a substitution.**
///
/// The rival is validly signed by the clinic's own enrolled key and targets the clinic's own
/// registered patient, so the substitution guard is the ONLY reason left for the door to refuse
/// it. It carries no wrapped DEK, on purpose: with the guard gone, a keyless record the door
/// accepts is counted `applied` outright, so the mutation below reads as a clean restore rather
/// than surfacing through the custody check.
///
/// Mutation this kills: db/020's `substitution refused` guard removed — the door's
/// `ON CONFLICT (event_id) DO NOTHING` then swallows the rival silently and the restore reports it
/// as a record it applied.
#[tokio::test]
async fn a_different_body_under_the_same_event_id_is_refused_as_a_substitution() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let dir = tempfile::tempdir().unwrap();

    let mut rival_bytes = Vec::new();
    let (chart, records, secret) =
        restore_ready_with_extra_records(&c, dir.path(), |captured, chart, sk, kid| {
            let hlc = cairn_event::Hlc {
                wall: 1,
                counter: 7,
                node_origin: "dead-clinic".into(),
            };
            let (body, _dek) = sealed_assert_body(
                kid,
                chart.patient,
                &chart.event_id,
                "a rival body under a borrowed event id",
                hlc,
            );
            rival_bytes = sign(&body, sk).unwrap().signed_bytes;
            vec![MediumRecord {
                signed_bytes: rival_bytes.clone(),
                attestation: None,
                attester_key: None,
                dek_wrapped: None,
                source_seq: captured.source_seq,
            }]
        })
        .await;
    assert_ne!(
        rival_bytes, chart.signed_bytes,
        "anti-vacuity: the rival must be a DIFFERENT body"
    );
    assert_eq!(copies_at_the_charts_position(&records, &chart), 2);

    let report = apply_clinical_plane(&c, &records, Some(&secret))
        .await
        .expect("a substitution is a refusal of one record, never a failure of the run");

    assert_eq!(
        report.penned(),
        1,
        "the rival is refused, and only the rival: {report:?}"
    );
    assert!(in_pen(&c, &rival_bytes).await, "and held in the pen");
    let reason: String = c
        .query_one(
            "SELECT reason FROM sync_quarantine WHERE content_digest = $1",
            &[&cairn_event::event_address(&rival_bytes)],
        )
        .await
        .unwrap()
        .get(0);
    assert!(
        reason.contains("substitution refused"),
        "with the DOOR's reason, which is the only place the cause exists: {reason}"
    );
    assert_eq!(
        twin_of(&c, &chart.event_id).await.as_deref(),
        Some(chart.twin.as_str()),
        "the chart reads the ORIGINAL body — the one the dead node wrote"
    );
}
