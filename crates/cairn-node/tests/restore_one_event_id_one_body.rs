//! DR slice 2d, design test 16 (#593): **one event id, one body — through the restore.**
//!
//! A medium can hold more than one record at a single `source_seq`. The derivation
//! (`cairn_medium::plane_records_with_accounting`, reached through
//! `backup::clinical_plane_accounting`) collapses a copy only when EVERY field agrees; a copy that
//! differs is kept and handed to the restore, deliberately, so the caller can name it rather than
//! silently keep whichever sorted first. Its sort is STABLE on `source_seq`, so copies at one
//! position reach the restore in FILE order — which is why every test here builds a real medium
//! and runs the real derivation rather than hand-ordering a `Vec`, and asserts the order it got
//! before trusting anything else.
//!
//! 1. **A re-capture that differs only in custody, keyed copy first, is a no-op.** The same signed
//!    event written a second time without its wrapped DEK — the shape a capture that straddled a
//!    crypto-shred leaves. The first copy restores and projects; the second finds the event
//!    present and changes nothing.
//! 2. **A different body under the same `event_id` is refused as a substitution.** Two bodies
//!    under one id would leave two nodes holding different bytes for one event forever, with no
//!    alarm (db/020's review H3). The restore must pen the rival with the DOOR's reason and leave
//!    the original as what the chart reads.
//! 3. **The same pair, KEYLESS copy first, still reaches the chart.** The keyless copy admits the
//!    event with no body; the keyed copy lands custody on it, and the door projects the late
//!    landing (#584, ADR-0070). Until #584 this order left the chart empty with nothing in the
//!    report to say so — trap 9's restore entrance.
//!
//! These are library-level on purpose. The CLI adds one line to these questions — the
//! `straddled_duplicate_notice` warning on stderr, which prints for all three and whose wording is
//! #597's — and `restore_cli_surface.rs` already proves the command reaches
//! `apply_clinical_plane`.

use cairn_event::seal::Secret32;
use cairn_event::{sign, SigningKey};
use cairn_medium::{parse_any, MediumRecord, Plane, Segment};
use cairn_node::restore::clinical::apply_clinical_plane;
use cairn_node::{backup, db, localstate};
use std::path::Path;
use tokio_postgres::Client;

mod common;

#[path = "common/restore_kit.rs"]
mod restore_kit;
use restore_kit::{
    author_sealed_clinical_event, capture, chained_segment, cs, fixture_unwrap_secret,
    in_event_log, keyless_record, medication_rows, pen_reason, provisioned_clinic, rewrite_medium,
    sealed_assert_body, twin_of, wipe_to_a_fresh_dr_machine, Authored,
};

/// The chart's record exactly as the capture wrote it. **Pure.**
fn captured_copy(segments: &[Segment], chart: &Authored) -> MediumRecord {
    segments
        .iter()
        .filter(|s| s.plane == Plane::Clinical)
        .flat_map(|s| s.records.iter())
        .find(|r| r.signed_bytes == chart.signed_bytes)
        .expect("the capture carries the sealed chart")
        .clone()
}

/// Append `records` to the medium as one correctly chained, unsigned clinical segment.
fn append_chained(segments: &mut Vec<Segment>, records: Vec<MediumRecord>) {
    let appended = chained_segment(segments, Plane::Clinical, records);
    segments.push(appended);
}

/// Every record at the chart's `source_seq`, in the order the derivation hands them to the
/// restore. **Pure.**
fn copies_at_the_charts_position<'a>(
    records: &'a [MediumRecord],
    chart: &Authored,
) -> Vec<&'a MediumRecord> {
    let position = records
        .iter()
        .find(|r| r.signed_bytes == chart.signed_bytes)
        .expect("the chart is in the trusted set")
        .source_seq;
    records
        .iter()
        .filter(|r| r.source_seq == position)
        .collect()
}

/// Wipe to a replacement machine and install what `restore`'s ceremony installs BEFORE the
/// clinical apply — custody and the actor registry — then return the records the restore is
/// entitled to apply (through the real derivation) and the inherited custody secret.
///
/// Reads the medium's trust accounting first, while the edit a test made is the only thing that
/// could have gone wrong: an appended segment that was not really chained would be gated out, and
/// the test would then be about the trust gate instead of about duplicates.
async fn restore_ready(
    c: &Client,
    sk: &SigningKey,
    medium: &Path,
) -> (Vec<MediumRecord>, Secret32) {
    let image = parse_any(&std::fs::read(medium).unwrap()).unwrap();
    let trusted = backup::clinical_plane_accounting(&image).unwrap();
    assert_eq!(
        trusted.gated_out, 0,
        "anti-vacuity: the appended segment must be CHAINED, or the restore never sees it and \
         this file tests the trust gate instead"
    );

    let secret = fixture_unwrap_secret(sk);
    let bundle = localstate::read_local_state(c, Some(&secret))
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

    (trusted.records, secret)
}

/// **A re-capture that differs only in custody, keyed copy first, is a no-op — and the chart
/// shows the medication.**
///
/// Mutation this kills: `plane_records_with_accounting`'s `records.dedup()` collapsing duplicates
/// by `source_seq` alone. That is the "simplification" the derivation's own doc warns against — it
/// silently keeps whichever copy sorted first, which can hand a restore a DEK it cannot open or
/// resurrect a key an erasure destroyed. It fails at the first assertion below, which is the
/// derivation half of this test's claim.
///
/// Two assertions guard the copy order. The premise check fires first, and legibly, if the
/// derivation's tie-break ever hands the keyless copy over first (a mutation that did so reddened
/// it). Since #584 the chart no longer tells the two orders apart — both project — so the premise
/// check is the ONLY guard of the copy order; the final assertion still pins that the keyed-first
/// order projects.
#[tokio::test]
async fn a_second_copy_without_its_key_at_the_same_position_changes_nothing() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let dir = tempfile::tempdir().unwrap();

    let (sk, kid) = provisioned_clinic(&c).await;
    let chart = author_sealed_clinical_event(&c, &sk, &kid).await;
    let medium = capture(&c, &sk, &kid, dir.path()).await;
    // The same record, a second time, in a LATER segment — without its key.
    rewrite_medium(&medium, |segments| {
        let captured = captured_copy(segments, &chart);
        append_chained(
            segments,
            vec![MediumRecord {
                dek_wrapped: None,
                ..captured
            }],
        );
    });
    let (records, secret) = restore_ready(&c, &sk, &medium).await;

    let copies = copies_at_the_charts_position(&records, &chart);
    assert_eq!(
        copies.len(),
        2,
        "the derivation must hand BOTH copies to the restore — collapsing by position alone \
         would decide which one survives without anyone seeing it"
    );
    assert!(
        copies[0].dek_wrapped.is_some() && copies[1].dek_wrapped.is_none(),
        "premise: this test is the KEYED-first order. If the derivation now hands the keyless \
         copy first, the tie-break changed and the keyless copy now arrives first — see \
         a_keyless_copy_first_still_reaches_the_chart"
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
    assert_eq!(
        medication_rows(&c, chart.patient).await,
        1,
        "and the CHART shows the medication — a readable body alone is not a chart (trap 9)"
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

    let (sk, kid) = provisioned_clinic(&c).await;
    let chart = author_sealed_clinical_event(&c, &sk, &kid).await;
    let medium = capture(&c, &sk, &kid, dir.path()).await;

    let hlc = cairn_event::Hlc {
        wall: 1,
        counter: 7,
        node_origin: "dead-clinic".into(),
    };
    let (rival, _dek) = sealed_assert_body(
        &kid,
        chart.patient,
        &chart.event_id,
        "a rival body under a borrowed event id",
        hlc,
    );
    let rival_bytes = sign(&rival, &sk).unwrap().signed_bytes;
    assert_ne!(
        rival_bytes, chart.signed_bytes,
        "anti-vacuity: the rival must be a DIFFERENT body"
    );
    rewrite_medium(&medium, |segments| {
        let position = captured_copy(segments, &chart).source_seq;
        append_chained(
            segments,
            vec![keyless_record(rival_bytes.clone(), position)],
        );
    });
    let (records, secret) = restore_ready(&c, &sk, &medium).await;
    assert_eq!(copies_at_the_charts_position(&records, &chart).len(), 2);

    let report = apply_clinical_plane(&c, &records, Some(&secret))
        .await
        .expect("a substitution is a refusal of one record, never a failure of the run");

    assert_eq!(
        report.penned(),
        1,
        "the rival is refused, and only the rival: {report:?}"
    );
    let reason = pen_reason(&c, &rival_bytes)
        .await
        .expect("and held in the pen");
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

/// **Trap 9's restore entrance, closed (#584, ADR-0070).**
///
/// When the keyless copy of a sealed event reaches the restore BEFORE its keyed copy, the door
/// admits the event with no body. The keyed copy then lands custody on the already-admitted event,
/// its `event_log` INSERT is a no-op — and the door, seeing it has just made the body readable for
/// an event already in the log, runs the event's heal-safe projections. Before #584 this order
/// left the medication list empty at exit 0 with a report identical to the keyed-first order; this
/// test pinned that as a known defect and was inverted when the door learned to project a late key.
///
/// How the order is made through the REAL derivation, which sorts stably on `source_seq`: the
/// capture's own copy of the chart loses its DEK in place, and the keyed copy is appended in a
/// later chained segment. Editing a record invalidates the capture segment's attestation, so that
/// segment is written unsigned — which a capture taken without the signing key legitimately is —
/// rather than left carrying a signature over bytes it no longer holds.
#[tokio::test]
async fn a_keyless_copy_first_still_reaches_the_chart() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let dir = tempfile::tempdir().unwrap();

    let (sk, kid) = provisioned_clinic(&c).await;
    let chart = author_sealed_clinical_event(&c, &sk, &kid).await;
    let medium = capture(&c, &sk, &kid, dir.path()).await;
    rewrite_medium(&medium, |segments| {
        let last = segments.last_mut().expect("the capture wrote segments");
        let in_place = last
            .records
            .iter_mut()
            .find(|r| r.signed_bytes == chart.signed_bytes)
            .expect("the capture's LAST segment holds the chart, so nothing chains from it yet");
        let keyed = in_place.clone();
        in_place.dek_wrapped = None;
        last.attestation = None;
        append_chained(segments, vec![keyed]);
    });
    let (records, secret) = restore_ready(&c, &sk, &medium).await;

    let copies = copies_at_the_charts_position(&records, &chart);
    assert_eq!(copies.len(), 2, "both copies reach the restore");
    assert!(
        copies[0].dek_wrapped.is_none() && copies[1].dek_wrapped.is_some(),
        "premise: this pin is the KEYLESS-first order, or it is not trap 9's entrance at all"
    );

    let report = apply_clinical_plane(&c, &records, Some(&secret))
        .await
        .expect("a duplicate position is never a failure of the run");

    assert_eq!(
        (report.penned(), report.already_present),
        (0, 1),
        "the report is indistinguishable from the keyed-first order — which is why nothing in a \
         restore's output flags this: {report:?}"
    );
    assert_eq!(
        twin_of(&c, &chart.event_id).await.as_deref(),
        Some(chart.twin.as_str()),
        "custody landed on the second copy, so the body opens"
    );
    assert_eq!(
        medication_rows(&c, chart.patient).await,
        1,
        "the late key reaches the chart: the door projects custody that lands after its event (#584)"
    );
}
