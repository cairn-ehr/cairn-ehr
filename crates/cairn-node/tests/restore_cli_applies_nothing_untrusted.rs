//! DR slice 2d, design tests 14 and 17 (#593): **what `cairn-node restore` must NOT apply, and
//! must say it did not** — driven through the real binary.
//!
//! Every case here is a medium that holds records the restore is not entitled to apply. Each has
//! the same failure shape if it goes wrong, and it is #500's: a restore that reads "restored" to
//! a solo clinic that then believes it has its charts back.
//!
//! - **Test 14 — trust stops at `verified_through`.** A chain link broken in the middle of the
//!   file: the verified prefix restores, and not one record past the break — including a later
//!   segment whose OWN link is intact, because a segment hanging from an unverified predecessor
//!   could have been spliced in whole (2a invariant 5). This was pinned at unit level
//!   (`plane_records`, `untrusted_clinical_notice`) and for `verify-backup`; never through the
//!   command that actually applies records.
//! - **Test 17a — a legacy CAIRNB2 medium is a NAMED outcome.** It has no clinical plane at all,
//!   and the summary must say so rather than print the CAIRNB3 "carries NO clinical records" line,
//!   whose remedy ("check the capture that wrote it") is wrong for a medium that predates the
//!   plane. Its sibling pins that CAIRNB3 line positively, so 17a's "never says this" cannot go
//!   quietly vacuous if the wording changes. (CAIRNB1 reaches the same `Legacy` arm and has no
//!   fixture here: #599.)
//! - **Test 17b — a plane this build cannot route is noted with its count, never applied.** The
//!   records in it are validly signed clinical events, so "not applied" is a real claim: routed as
//!   clinical, the door would have admitted them.
//!
//! **Exit status is deliberately NOT asserted for 14 or 17b.** Both currently exit 0, matching the
//! torn-medium ruling (`restore_torn_medium_cli.rs`), while a restore that PENNED records or
//! offered none exits non-zero. Whether records a restore could not apply should also fail a
//! monitoring script is a decision, not a test finding: it is
//! [#594](https://github.com/cairn-ehr/cairn-ehr/issues/594), and either answer can land without
//! inverting a test here.

use cairn_event::{generate_key, sign, EventBody, Hlc};
use cairn_medium::{parse_any, segment_commitment, Plane, SelfMarker};
use cairn_node::{backup, db, identity};
use uuid::Uuid;

mod common;

#[path = "common/restore_kit.rs"]
mod restore_kit;
use restore_kit::{
    a_recovery_code, an_op_passphrase, author_sealed_clinical_event, capture, chained_segment,
    clinical_record_count, cs, in_event_log, in_pen, keyless_record, old_recovery_code_file,
    provisioned_clinic, restore_cli, rewrite_medium, sealed_assert_body, twin_of,
    wipe_to_a_fresh_dr_machine, write_export_beside, ExportCustody,
};

/// **Test 14.** Three nights of captures — charts A, B and C, one per night — and then B's
/// segment has its chain link broken. A restores and opens; B and C are in neither the log nor
/// the pen; and the warning prints twice, early on stderr and again in the stdout summary.
///
/// One more record rides past the break: a validly signed keyless event for A's own patient,
/// appended in its own segment with a `source_seq` LOWER than anything A's capture holds. It is
/// what makes the gate's KEY part of the test. Captures number records in increasing order, so B
/// and C sit above A's watermark as well as past the break, and a gate that filtered by position
/// in the sequence instead of position in the file would exclude them too, and pass. Only a record
/// that is past the break but low in the sequence can tell the two apart.
///
/// Mutations this kills: `plane_records_with_accounting` taking every segment instead of the
/// prefix through `verified_through` — B and C are then admitted; and a gate on
/// `source_seq <= watermark` in place of that segment prefix — the low record is then admitted.
#[tokio::test]
async fn a_restore_applies_the_verified_prefix_and_not_one_record_past_a_chain_break() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let dir = tempfile::tempdir().unwrap();
    let (op, code) = (an_op_passphrase(14), a_recovery_code(14));

    let (sk, kid) = provisioned_clinic(&c).await;
    let chart_a = author_sealed_clinical_event(&c, &sk, &kid).await;
    let medium = capture(&c, &sk, &kid, dir.path()).await;
    let chart_b = author_sealed_clinical_event(&c, &sk, &kid).await;
    capture(&c, &sk, &kid, dir.path()).await;
    let chart_c = author_sealed_clinical_event(&c, &sk, &kid).await;
    capture(&c, &sk, &kid, dir.path()).await;

    // Past the break but LOW in the sequence: never submitted, so only the medium carries it.
    let low_event_id = Uuid::now_v7().to_string();
    let hlc = Hlc {
        wall: 4,
        counter: 0,
        node_origin: "dead-clinic".into(),
    };
    let (low_body, _dek) =
        sealed_assert_body(&kid, chart_a.patient, &low_event_id, "low and late", hlc);
    let low_bytes = sign(&low_body, &sk).unwrap().signed_bytes;

    // Break the link of the segment holding B, append the low record, and count what sits past the
    // break — by hand, from the segments, never from the code under test.
    let (past_the_break, on_the_plane) = rewrite_medium(&medium, |segments| {
        let holding = |bytes: &[u8]| {
            segments
                .iter()
                .position(|s| s.records.iter().any(|r| r.signed_bytes == bytes))
                .expect("a capture wrote this chart's segment")
        };
        let (broken, c_segment) = (
            holding(&chart_b.signed_bytes),
            holding(&chart_c.signed_bytes),
        );
        assert!(
            c_segment > broken
                && segments[c_segment].prev_commitment
                    == segment_commitment(&segments[c_segment - 1].records),
            "anti-vacuity: C must sit in a LATER segment whose own link is intact, or 'hanging from \
             B' is not the case under test"
        );
        let lowest_seq = segments
            .iter()
            .flat_map(|s| s.records.iter().map(|r| r.source_seq))
            .min()
            .expect("the captures wrote records");
        segments[broken].prev_commitment = "deadbeef".into();
        let appended = chained_segment(
            segments,
            Plane::Clinical,
            vec![keyless_record(low_bytes.clone(), lowest_seq - 1)],
        );
        segments.push(appended);
        (
            clinical_record_count(&segments[broken..]),
            clinical_record_count(segments),
        )
    });
    assert!(
        past_the_break >= 5,
        "anti-vacuity: B's and C's segments (a registration and a chart each) and the low record \
         must all sit past the break, or 'not one record past it' is a statement about one segment"
    );
    // The expected `trusted` count below is this TEST's subtraction (on the plane − past the break).
    // Production counts the restored records after collapsing duplicates, so the two agree only
    // when nothing was collapsed. Check that premise rather than assume it.
    let accounting =
        backup::clinical_plane_accounting(&parse_any(&std::fs::read(&medium).unwrap()).unwrap())
            .unwrap();
    assert_eq!(
        accounting.collapsed, 0,
        "premise: every record on this medium is distinct"
    );
    write_export_beside(&c, &sk, &medium, &op, &code, ExportCustody::Carried).await;

    wipe_to_a_fresh_dr_machine(&c).await;
    let code_file = old_recovery_code_file(dir.path(), &code);
    // Exit status deliberately NOT asserted: #594 decides it (see this file's header).
    let out = restore_cli(
        &base,
        &dir.path().join("restored.key"),
        &medium,
        Some(&code_file),
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);

    // THE PREFIX IS WORTH HAVING: refusing the medium would have cost chart A too.
    assert_eq!(
        twin_of(&c, &chart_a.event_id).await.as_deref(),
        Some(chart_a.twin.as_str()),
        "the verified prefix must restore and OPEN; stdout:\n{stdout}\nstderr:\n{stderr}"
    );
    // NOT ONE RECORD PAST IT — not applied, and not penned either: the pen is a promise that a
    // `requeue` will finish the job, and these records must never enter the record at all.
    for (label, event_id, bytes) in [
        (
            "chart B, whose link is broken",
            &chart_b.event_id,
            &chart_b.signed_bytes,
        ),
        (
            "chart C, hanging from B",
            &chart_c.event_id,
            &chart_c.signed_bytes,
        ),
        (
            "the low record, past the break but below A's watermark",
            &low_event_id,
            &low_bytes,
        ),
    ] {
        assert!(
            !in_event_log(&c, event_id).await,
            "{label} sits past the last verified chain link and must not be applied"
        );
        assert!(
            !in_pen(&c, bytes).await,
            "{label} must not be penned either — a pen row promises a requeue can finish it"
        );
    }
    let trusted = on_the_plane - past_the_break;
    assert!(
        stderr.contains(&format!(
            "{past_the_break} of this medium's {on_the_plane} clinical record(s) sit PAST its \
             last verified chain link"
        )) && stderr.contains(&format!(
            "Only the {trusted} verified record(s) were restored"
        )),
        "the operator must be told how many were left behind and why, with honest counts; \
         stderr:\n{stderr}"
    );
    assert!(
        stdout.contains(&format!(
            "{past_the_break} of those {on_the_plane} were past this medium's last verified \
             chain link"
        )),
        "and the summary must repeat it — an operator reading only the tail of a long restore \
         would otherwise see numbers that quietly fail to add up; stdout:\n{stdout}"
    );
}

/// A signed CAIRNB2 medium for a node that never existed on this machine: one genesis, one
/// verifying self-marker. Built from the production primitives, as
/// `backup_carries_both_planes.rs::a_peers_legacy_medium` builds its own.
fn a_signed_legacy_medium() -> Vec<u8> {
    let (sk, kid) = generate_key().expect("entropy for the dead node's key");
    let genesis = EventBody {
        event_id: Uuid::now_v7().to_string(),
        patient_id: identity::NIL_PATIENT.into(),
        event_type: "node.enrolled".into(),
        schema_version: "node/1".into(),
        hlc: Hlc {
            wall: 1,
            counter: 0,
            node_origin: "legacy-clinic".into(),
        },
        t_effective: None,
        signer_key_id: kid.clone(),
        contributors: serde_json::json!([{"actor_id": kid, "role": "recorded"}]),
        payload: serde_json::json!({"display_name": "legacy-clinic", "address": "127.0.0.1:7998"}),
        attachments: vec![],
        plaintext_twin: None,
        clock_grade: cairn_event::ClockGrade::SelfAsserted,
        safety: None,
    };
    let events = vec![sign(&genesis, &sk).unwrap().signed_bytes];
    let node_id = hex::encode(cairn_event::event_address(&events[0]));
    let marker = cairn_medium::build_self_attestation(&sk, &kid, &node_id, &events);
    cairn_medium::serialize_container(Some(&SelfMarker::Signed(marker)), &events).unwrap()
}

/// **Test 17a.** Mutation this kills: the `Legacy` arm deleted from the restore summary, which
/// then falls through to the CAIRNB3 empty-plane note and its wrong remedy.
#[tokio::test]
async fn a_legacy_medium_is_named_as_predating_the_clinical_plane() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    wipe_to_a_fresh_dr_machine(&c).await;

    let dir = tempfile::tempdir().unwrap();
    let medium = dir.path().join("legacy.medium");
    std::fs::write(&medium, a_signed_legacy_medium()).unwrap();

    let out = restore_cli(&base, &dir.path().join("restored.key"), &medium, None);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);

    assert!(
        out.status.success(),
        "a legacy medium is a legitimate federation-only restore; stdout:\n{stdout}\n\
         stderr:\n{stderr}"
    );
    assert!(
        stdout.contains("restored 1 event(s)"),
        "anti-vacuity: the federation plane really was restored; stdout:\n{stdout}"
    );
    assert!(
        stdout.contains("predates the clinical plane")
            && stdout.contains("they are NOT on this medium"),
        "a legacy medium must be NAMED as predating patient data, so a clinic does not read \
         'restored' and believe it has its charts back; stdout:\n{stdout}"
    );
    assert!(
        !stdout.contains("this CAIRNB3 medium"),
        "and must never be described as a CAIRNB3 medium whose capture came up empty — that \
         remedy sends the operator after a capture that never had a clinical plane to write; \
         stdout:\n{stdout}"
    );
}

/// **Test 17a's other half: a CAIRNB3 medium whose clinical plane is empty is named too.**
///
/// This is what gives 17a's negative assertion its teeth. `!stdout.contains("this CAIRNB3
/// medium")` passes against ANY wording change to the empty-plane note, so without a test that
/// finds that phrase where it belongs, a rewording would leave 17a green while proving nothing. A
/// clinic that never recorded a chart captures exactly this medium.
#[tokio::test]
async fn an_empty_clinical_plane_is_named_rather_than_reported_as_nothing() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let dir = tempfile::tempdir().unwrap();

    let (sk, kid) = provisioned_clinic(&c).await;
    let medium = capture(&c, &sk, &kid, dir.path()).await;
    wipe_to_a_fresh_dr_machine(&c).await;

    let out = restore_cli(&base, &dir.path().join("restored.key"), &medium, None);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);

    assert!(
        out.status.success(),
        "an empty clinical plane is a legitimate federation-only restore; stdout:\n{stdout}\n\
         stderr:\n{stderr}"
    );
    assert!(
        stdout.contains("note: this CAIRNB3 medium carries NO clinical records"),
        "the empty plane must be NAMED, not left as a summary that never mentions patients; \
         stdout:\n{stdout}"
    );
    assert!(
        !stdout.contains("predates the clinical plane"),
        "and never confused with a legacy medium, whose remedy differs; stdout:\n{stdout}"
    );
}

/// **Test 17b.** Mutation this kills: the unknown-plane note deleted from the restore summary —
/// the two records then sit on the medium unmentioned inside a clean-looking restore.
#[tokio::test]
async fn a_plane_this_build_cannot_route_is_noted_with_its_count_and_never_applied() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let dir = tempfile::tempdir().unwrap();
    let (op, code) = (an_op_passphrase(17), a_recovery_code(17));

    let (sk, kid) = provisioned_clinic(&c).await;
    let chart = author_sealed_clinical_event(&c, &sk, &kid).await;
    let medium = capture(&c, &sk, &kid, dir.path()).await;

    // Two VALIDLY SIGNED clinical events for the chart's own registered patient, never submitted
    // here — so if they were ever routed as clinical, the door would admit them.
    let unroutable: Vec<(String, Vec<u8>)> = (0..2)
        .map(|n| {
            let event_id = Uuid::now_v7().to_string();
            let hlc = Hlc {
                wall: 2,
                counter: n,
                node_origin: "newer-cairn".into(),
            };
            let (body, _dek) =
                sealed_assert_body(&kid, chart.patient, &event_id, "unroutable", hlc);
            (event_id, sign(&body, &sk).unwrap().signed_bytes)
        })
        .collect();
    rewrite_medium(&medium, |segments| {
        let records = unroutable
            .iter()
            .enumerate()
            .map(|(n, (_, bytes))| keyless_record(bytes.clone(), 10_000 + n as i64))
            .collect();
        // Tag 0x7f: no Cairn build has used it, so this build cannot know what it holds.
        let appended = chained_segment(segments, Plane::from_tag(0x7f), records);
        segments.push(appended);
    });
    write_export_beside(&c, &sk, &medium, &op, &code, ExportCustody::Carried).await;

    wipe_to_a_fresh_dr_machine(&c).await;
    let code_file = old_recovery_code_file(dir.path(), &code);
    // Exit status deliberately NOT asserted: #594 decides it (see this file's header).
    let out = restore_cli(
        &base,
        &dir.path().join("restored.key"),
        &medium,
        Some(&code_file),
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);

    assert_eq!(
        twin_of(&c, &chart.event_id).await.as_deref(),
        Some(chart.twin.as_str()),
        "the plane this build CAN read restores regardless — a newer plane is not damage; \
         stdout:\n{stdout}\nstderr:\n{stderr}"
    );
    assert!(
        stdout.contains("also carries 2 record(s) in a plane this build does not recognise"),
        "the operator must be told what was left behind, with its count; stdout:\n{stdout}"
    );
    // Neither applied NOR penned. An unroutable record offered to the door and refused would stay
    // out of `event_log` too, and the note above is computed from the plane counts rather than from
    // what was offered, so only the pen tells "never offered" apart from "offered and refused".
    for (event_id, bytes) in &unroutable {
        assert!(
            !in_event_log(&c, event_id).await,
            "a record in a plane this build cannot route must never be applied as clinical"
        );
        assert!(
            !in_pen(&c, bytes).await,
            "nor offered to the door and penned — it was never routed as clinical at all"
        );
    }
}
