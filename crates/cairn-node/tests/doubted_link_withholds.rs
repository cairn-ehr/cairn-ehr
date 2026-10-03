//! #697 (b): while a chart set holds a DOUBTED link (db/054 `cairn_chart_set_has_doubted_link`:
//! an un-attested link db/018 flagged, or that trips the hard veto now, or a clinician's
//! attested unlink between two charts the set still joins), every medication line
//! not recorded only on the OPENED chart is withheld from sign-off, carrying the `doubted_link`
//! reason; the opened chart's own lines stay signable; humans judging the links lift it. Also
//! #701: the doubted-link test reads the stored `patient_link.attested`.
//!
//! Two tests moved here from `combined_read.rs` (R1), where the rule withheld only MULTI-chart
//! lines: `a_doubted_set_withholds_every_line_not_on_the_opened_chart` (1st in this file; it was
//! `a_group_across_a_vetoed_link_is_still_withheld` there) and
//! `a_group_across_a_link_the_veto_now_refuses_is_withheld` (4th, same name). The first one's
//! one-chart assertion is the line #697 (b) reverses.
//!
//! DB-gated on $CAIRN_TEST_PG, serialized via `db::test_serial_guard` taken BEFORE connecting.
//! Key material is minted at runtime (house rule 6). `chart`, `assert_one` and `group`
//! below are copies of `combined_read.rs`'s; the other helpers are new in this file.
mod common;
use cairn_event::demographics::{dob_assertion_body, render_dob_twin};
use cairn_event::SigningKey;
use cairn_medication_view::{
    sign_off_targets, withheld_group_ids, withheld_rows, MedicationRow, PatientMedicationList,
    WrongChartReasons,
};
use cairn_node::chart_link::{link_charts, unlink_charts, LinkEffect, Reviewer};
use cairn_node::db;
use cairn_node::medication::read::list_patient_medications;
use cairn_node::medication::{assert_medication, AssertMedicationInput};
use common::{
    cs, medication_setup as setup, submit_link_event, submit_registration, submit_signed, EventSpec,
};
use tokio_postgres::Client;
use uuid::Uuid;

const ORIGIN: &str = "r1b-test-node";
const DOUBTED: WrongChartReasons = WrongChartReasons {
    outside_set: false,
    doubted_link: true,
};

async fn chart(c: &Client, sk: &SigningKey, kid: &str) -> Uuid {
    let p = Uuid::now_v7();
    submit_registration(c, sk, kid, p, 0).await;
    p
}

async fn assert_one(c: &mut Client, sk: &SigningKey, kid: &str, patient: Uuid, term: &str) -> Uuid {
    assert_medication(
        c,
        sk,
        kid,
        "origin-a",
        patient,
        &AssertMedicationInput {
            term,
            coding: None,
            formulation: None,
            dose_amount: Some("500"),
            dose_unit: Some("mg"),
            sig: None,
            info_source: "patient",
            started: None,
            started_precision: None,
        },
        None,
        None,
    )
    .await
    .unwrap()
}

/// Fold two threads into one group (the peer-arrival shape; see `combined_read.rs`).
async fn group(c: &Client, first: Uuid, second: Uuid) {
    c.execute(
        "INSERT INTO medication_group_member (medication_id, group_id) VALUES \
         ($1::text::uuid, $1::text::uuid), ($2::text::uuid, $1::text::uuid)",
        &[&first.to_string(), &second.to_string()],
    )
    .await
    .unwrap();
}

/// Flag the standing a-b link as db/018 would for an un-attested link that trips the veto
/// (its lifecycle is pinned by `link_veto_floor.rs`; these tests are about the read).
async fn flag_link(c: &Client, a: Uuid, b: Uuid) {
    let (lo, hi) = (a.min(b), a.max(b));
    c.execute(
        "INSERT INTO link_veto_flag (low, high, content_address) \
         SELECT low, high, content_address FROM patient_link \
         WHERE low = $1::text::uuid AND high = $2::text::uuid",
        &[&lo.to_string(), &hi.to_string()],
    )
    .await
    .unwrap();
}

fn row_of(list: &PatientMedicationList, group: Uuid) -> &MedicationRow {
    list.rows
        .iter()
        .find(|r| r.group_id == group)
        .expect("the line is shown")
}

/// The read builds `cross_patient` and `wrong_chart` from one rule; pin that on every row.
fn assert_flag_agrees(list: &PatientMedicationList) {
    for r in &list.rows {
        assert_eq!(r.cross_patient, r.wrong_chart.any(), "row {}", r.group_id);
    }
}

/// db/054's answer for one record's charts, asked directly.
async fn set_doubted(c: &Client, charts: &[Uuid]) -> bool {
    let ids: Vec<String> = charts.iter().map(Uuid::to_string).collect();
    c.query_one(
        "SELECT cairn_chart_set_has_doubted_link($1::text[]::uuid[])",
        &[&ids],
    )
    .await
    .unwrap()
    .get(0)
}

fn sorted(mut v: Vec<Uuid>) -> Vec<Uuid> {
    v.sort();
    v
}

/// Moved from `combined_read.rs` and REVERSED in its one-chart half (#697 (b)). A-X linked
/// with a flagged link; a group shared by A and X, a line only on X, a line only on A.
#[tokio::test]
async fn a_doubted_set_withholds_every_line_not_on_the_opened_chart() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    c.batch_execute("TRUNCATE patient_link, person_member, link_veto_flag")
        .await
        .unwrap();
    let (sk, kid, _hsk, _hkid) = setup(&c).await;
    let a = chart(&c, &sk, &kid).await;
    let x = chart(&c, &sk, &kid).await;
    let ta = assert_one(&mut c, &sk, &kid, a, "warfarin").await;
    let tx = assert_one(&mut c, &sk, &kid, x, "warfarin").await;
    let only_x = assert_one(&mut c, &sk, &kid, x, "amlodipine").await;
    let only_a = assert_one(&mut c, &sk, &kid, a, "metformin").await;
    group(&c, ta, tx).await;
    submit_link_event(&c, &sk, &kid, a, x, 10, true).await;

    // Positive control: the link is NOT doubted yet, so every line is one person's.
    let before = list_patient_medications(&c, a).await.unwrap();
    assert_flag_agrees(&before);
    assert!(before.rows.iter().all(|r| !r.cross_patient));

    flag_link(&c, a, x).await;
    let list = list_patient_medications(&c, a).await.unwrap();
    assert_flag_agrees(&list);
    assert!(
        list.charts.is_linked(),
        "the doubted link still combines the read"
    );
    assert_eq!(
        row_of(&list, ta).wrong_chart,
        DOUBTED,
        "the shared line: doubted, not outside"
    );
    assert_eq!(
        row_of(&list, only_x).wrong_chart,
        DOUBTED,
        "X's one-chart line is withheld: signing it would vouch for a possible stranger's drug"
    );
    assert!(
        !row_of(&list, only_a).cross_patient,
        "A's own line is A's own"
    );
    assert!(
        list.separation_targets.contains_key(&ta),
        "the shared line carries the arguments to separate it"
    );
    assert_eq!(
        sign_off_targets(&list.rows),
        vec![only_a],
        "only the opened chart's own line is signed"
    );
    assert_eq!(
        withheld_group_ids(&withheld_rows(&list.rows)),
        sorted(vec![ta, only_x]),
        "both withheld lines are REPORTED, by group id"
    );
    assert!(withheld_rows(&list.rows)
        .iter()
        .all(|w| w.reasons == DOUBTED));
}

/// Guards the mirror case: opened from X, the rule mirrors - X's own line is signable, A's is not.
#[tokio::test]
async fn seen_from_the_other_chart_the_withholding_mirrors() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    c.batch_execute("TRUNCATE patient_link, person_member, link_veto_flag")
        .await
        .unwrap();
    let (sk, kid, _hsk, _hkid) = setup(&c).await;
    let a = chart(&c, &sk, &kid).await;
    let x = chart(&c, &sk, &kid).await;
    let only_x = assert_one(&mut c, &sk, &kid, x, "amlodipine").await;
    let only_a = assert_one(&mut c, &sk, &kid, a, "metformin").await;
    submit_link_event(&c, &sk, &kid, a, x, 10, true).await;
    flag_link(&c, a, x).await;

    let list = list_patient_medications(&c, x).await.unwrap();
    assert_flag_agrees(&list);
    assert_eq!(row_of(&list, only_a).wrong_chart, DOUBTED);
    assert!(!row_of(&list, only_x).cross_patient);
    assert_eq!(sign_off_targets(&list.rows), vec![only_x]);
}

/// The withholding is lifted by a human: an attested link outranks the machine's (ADR-0076
/// D5) and db/018 clears the flag, so the set holds no doubted link and every line is signable.
#[tokio::test]
async fn a_human_link_lifts_the_withholding() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    c.batch_execute("TRUNCATE patient_link, person_member, link_veto_flag")
        .await
        .unwrap();
    let (sk, kid, hsk, hkid) = setup(&c).await;
    let a = chart(&c, &sk, &kid).await;
    let x = chart(&c, &sk, &kid).await;
    let only_x = assert_one(&mut c, &sk, &kid, x, "amlodipine").await;
    let only_a = assert_one(&mut c, &sk, &kid, a, "metformin").await;
    submit_link_event(&c, &sk, &kid, a, x, 10, true).await;
    flag_link(&c, a, x).await;
    let held = list_patient_medications(&c, a).await.unwrap();
    assert_eq!(
        sign_off_targets(&held.rows),
        vec![only_a],
        "precondition: X's line is held"
    );

    let who = Reviewer {
        human_sk: &hsk,
        human_kid: &hkid,
    };
    link_charts(&mut c, a, x, &who, ORIGIN).await.unwrap();
    let list = list_patient_medications(&c, a).await.unwrap();
    assert_flag_agrees(&list);
    assert!(
        list.rows.iter().all(|r| !r.cross_patient),
        "nothing is withheld any more"
    );
    assert_eq!(sign_off_targets(&list.rows), sorted(vec![only_a, only_x]));
}

/// Issue #220's path: db/018 evaluates the hard veto only when a link ARRIVES, so a link
/// that synced ahead of the clashing demographics is never flagged. Here the un-attested link
/// is admitted while neither chart has a date of birth (nothing to veto), then two clashing
/// document-verified dates arrive. `link_veto_flag` stays empty — and the read must still
/// withhold a group spanning the pair, because db/054's doubted-link test re-evaluates the
/// veto at read time.
#[tokio::test]
async fn a_group_across_a_link_the_veto_now_refuses_is_withheld() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    c.batch_execute("TRUNCATE patient_link, person_member, link_veto_flag")
        .await
        .unwrap();
    let (sk, kid, _hsk, _hkid) = setup(&c).await;
    let a = chart(&c, &sk, &kid).await;
    let x = chart(&c, &sk, &kid).await;
    let ta = assert_one(&mut c, &sk, &kid, a, "warfarin").await;
    let tx = assert_one(&mut c, &sk, &kid, x, "warfarin").await;
    let only_x = assert_one(&mut c, &sk, &kid, x, "amlodipine").await;
    group(&c, ta, tx).await;
    submit_link_event(&c, &sk, &kid, a, x, 10, true).await;
    let before = list_patient_medications(&c, a).await.unwrap();
    assert!(
        before.rows.iter().all(|r| !r.cross_patient),
        "positive control: no clash yet, so the link is not doubted"
    );

    verified_dob(&c, &sk, &kid, a, "1980-07-15", 20).await;
    verified_dob(&c, &sk, &kid, x, "1975-01-02", 21).await;
    let flags: i64 = c
        .query_one("SELECT count(*) FROM link_veto_flag", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        flags, 0,
        "precondition: #220 — the late clash raised no flag"
    );

    let list = list_patient_medications(&c, a).await.unwrap();
    assert!(list.charts.is_linked());
    assert_flag_agrees(&list);
    assert!(
        row_of(&list, ta).cross_patient,
        "the veto trips NOW, so the link is doubted and the shared line is a hazard"
    );
    assert_eq!(
        row_of(&list, only_x).wrong_chart,
        DOUBTED,
        "#697 (b) on the read-time veto path too"
    );
    assert!(sign_off_targets(&list.rows).is_empty());
}

/// A document-verified date of birth — the trustworthy kind db/016's hard veto compares
/// (`link_veto_floor.rs`'s `submit_dob` shape). `wall` orders it after the link.
async fn verified_dob(
    c: &Client,
    sk: &SigningKey,
    kid: &str,
    patient: Uuid,
    value: &str,
    wall: i64,
) {
    submit_signed(
        c,
        sk,
        kid,
        EventSpec {
            patient,
            event_type: "demographic.field.asserted",
            schema_version: "demographic.field/1",
            payload: dob_assertion_body(value, "day", Some("document"), "document-verified"),
            plaintext_twin: Some(render_dob_twin(value, "day", "document-verified")),
            wall,
        },
    )
    .await
    .expect("dob accepted");
}

/// #701: the doubted-link test reads the STORED `patient_link.attested` (R2a's one definition,
/// evaluated when the winner was applied), not a second spelling re-derived through an
/// `event_log` join. The UPDATE below simulates nothing real: it makes the two spellings
/// DISAGREE, so the answer shows which one the function reads.
#[tokio::test]
async fn the_doubted_link_check_reads_the_stored_attested_column() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    c.batch_execute("TRUNCATE patient_link, person_member, link_veto_flag")
        .await
        .unwrap();
    let (sk, kid, _hsk, _hkid) = setup(&c).await;
    let a = chart(&c, &sk, &kid).await;
    let x = chart(&c, &sk, &kid).await;
    submit_link_event(&c, &sk, &kid, a, x, 10, true).await;
    verified_dob(&c, &sk, &kid, a, "1980-07-15", 20).await;
    verified_dob(&c, &sk, &kid, x, "1975-01-02", 21).await;
    let ids = vec![a.to_string(), x.to_string()];
    async fn doubted(c: &Client, ids: &[String]) -> bool {
        c.query_one(
            "SELECT cairn_chart_set_has_doubted_link($1::text[]::uuid[])",
            &[&ids],
        )
        .await
        .unwrap()
        .get::<_, bool>(0)
    }
    assert!(
        doubted(&c, &ids).await,
        "precondition: un-attested and the veto trips now"
    );

    let (lo, hi) = (a.min(x), a.max(x));
    c.execute(
        "UPDATE patient_link SET attested = TRUE \
         WHERE low = $1::text::uuid AND high = $2::text::uuid",
        &[&lo.to_string(), &hi.to_string()],
    )
    .await
    .unwrap();
    assert!(
        !doubted(&c, &ids).await,
        "the function must read pl.attested — a re-derivation would still say doubted"
    );
}

/// F1a, the A–C–X bridge (maintainer decision 2026-10-03): an ATTESTED unlink between two
/// charts that are BOTH still in the set is a clinician's record that the set holds two
/// people, so the set holds a doubted link. Here neither machine link trips the veto (no
/// dates of birth — a sparse chart such as a John Doe), so before the third case db/054 found
/// NO doubt: minutes after a human said "A and X are different people", X's line was
/// signable from A, through C. Lifted here by X leaving A's record; a human relinking A–X lifts
/// it too (`a_human_relink_lifts_the_bridge_doubt`).
#[tokio::test]
async fn an_attested_unlink_inside_the_set_is_a_doubted_link() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    c.batch_execute("TRUNCATE patient_link, person_member, link_veto_flag")
        .await
        .unwrap();
    let (sk, kid, hsk, hkid) = setup(&c).await;
    let a = chart(&c, &sk, &kid).await;
    let bridge = chart(&c, &sk, &kid).await;
    let x = chart(&c, &sk, &kid).await;
    let only_x = assert_one(&mut c, &sk, &kid, x, "amlodipine").await;
    let only_a = assert_one(&mut c, &sk, &kid, a, "metformin").await;
    // A line on the bridge chart: the one line whose state after the lift shows whether the
    // doubt really lifted (A's own line is signable either way).
    let only_bridge = assert_one(&mut c, &sk, &kid, bridge, "atorvastatin").await;
    submit_link_event(&c, &sk, &kid, a, bridge, 10, true).await;
    submit_link_event(&c, &sk, &kid, bridge, x, 11, true).await;

    // Positive control: two machine links, no clash, no human judgement — one person.
    let before = list_patient_medications(&c, a).await.unwrap();
    assert_flag_agrees(&before);
    assert_eq!(
        before.charts.members(),
        sorted(vec![a, bridge, x]).as_slice()
    );
    assert!(before.rows.iter().all(|r| !r.cross_patient));

    let who = Reviewer {
        human_sk: &hsk,
        human_kid: &hkid,
    };
    let out = unlink_charts(&mut c, a, x, Some(a), &who, ORIGIN)
        .await
        .unwrap();
    assert_eq!(
        out.effect,
        LinkEffect::StillJoined,
        "precondition: the bridge keeps A and X in one record"
    );
    let list = list_patient_medications(&c, a).await.unwrap();
    assert_flag_agrees(&list);
    assert_eq!(
        list.charts.members(),
        sorted(vec![a, bridge, x]).as_slice(),
        "still joined through the bridge chart"
    );
    assert_eq!(
        row_of(&list, only_x).wrong_chart,
        DOUBTED,
        "a human said X is another person: X's line must not be signed from A"
    );
    assert_eq!(
        row_of(&list, only_bridge).wrong_chart,
        DOUBTED,
        "every line not on the opened chart is held while the record holds a doubt"
    );
    assert_eq!(sign_off_targets(&list.rows), vec![only_a]);

    // The lift: unlink the bridge from X, so X leaves A's record and nothing is in doubt.
    // The A–X unlink still stands, with X now OUTSIDE the set: it must no longer count.
    let out = unlink_charts(&mut c, bridge, x, Some(a), &who, ORIGIN)
        .await
        .unwrap();
    assert_eq!(out.effect, LinkEffect::TookEffect);
    let lifted = list_patient_medications(&c, a).await.unwrap();
    assert_flag_agrees(&lifted);
    assert_eq!(lifted.charts.members(), sorted(vec![a, bridge]).as_slice());
    assert!(
        lifted.rows.iter().all(|r| !r.is_wrong_chart_hazard()),
        "nothing is withheld once X is out"
    );
    assert_eq!(
        sign_off_targets(&lifted.rows),
        sorted(vec![only_a, only_bridge]),
        "the bridge chart's line is signable again: an unlink reaching outside the set is no doubt"
    );
}

/// The other way out of the bridge doubt: a human relinks A–X. Both judgements are attested,
/// so the later one wins (ADR-0076 decision 5), the pair's standing state is a link again, and
/// nothing in the set is in doubt.
#[tokio::test]
async fn a_human_relink_lifts_the_bridge_doubt() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    c.batch_execute("TRUNCATE patient_link, person_member, link_veto_flag")
        .await
        .unwrap();
    let (sk, kid, hsk, hkid) = setup(&c).await;
    let a = chart(&c, &sk, &kid).await;
    let bridge = chart(&c, &sk, &kid).await;
    let x = chart(&c, &sk, &kid).await;
    let only_x = assert_one(&mut c, &sk, &kid, x, "amlodipine").await;
    let only_a = assert_one(&mut c, &sk, &kid, a, "metformin").await;
    submit_link_event(&c, &sk, &kid, a, bridge, 10, true).await;
    submit_link_event(&c, &sk, &kid, bridge, x, 11, true).await;
    let who = Reviewer {
        human_sk: &hsk,
        human_kid: &hkid,
    };
    let out = unlink_charts(&mut c, a, x, Some(a), &who, ORIGIN)
        .await
        .unwrap();
    assert_eq!(out.effect, LinkEffect::StillJoined, "precondition");
    let held = list_patient_medications(&c, a).await.unwrap();
    assert_eq!(
        row_of(&held, only_x).wrong_chart,
        DOUBTED,
        "precondition: the bridge doubt holds X's line"
    );

    link_charts(&mut c, a, x, &who, ORIGIN).await.unwrap();
    let list = list_patient_medications(&c, a).await.unwrap();
    assert_flag_agrees(&list);
    assert!(
        list.rows.iter().all(|r| !r.is_wrong_chart_hazard()),
        "the relink is the standing judgement: nothing is in doubt"
    );
    assert_eq!(sign_off_targets(&list.rows), sorted(vec![only_a, only_x]));
}

/// The deliberate under-warn (`medication::hazard`'s NOT COVERED note): an UN-attested unlink
/// between two charts a bridge still joins is not a doubt. Only a human's "not the same person"
/// is; counting a machine's or a peer agent's would let any unreviewed writer freeze sign-off.
#[tokio::test]
async fn an_unattested_unlink_inside_the_set_is_not_a_doubt() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    c.batch_execute("TRUNCATE patient_link, person_member, link_veto_flag")
        .await
        .unwrap();
    let (sk, kid, _hsk, _hkid) = setup(&c).await;
    let a = chart(&c, &sk, &kid).await;
    let bridge = chart(&c, &sk, &kid).await;
    let x = chart(&c, &sk, &kid).await;
    let only_x = assert_one(&mut c, &sk, &kid, x, "amlodipine").await;
    let only_a = assert_one(&mut c, &sk, &kid, a, "metformin").await;
    submit_link_event(&c, &sk, &kid, a, bridge, 10, true).await;
    submit_link_event(&c, &sk, &kid, bridge, x, 11, true).await;
    submit_link_event(&c, &sk, &kid, a, x, 12, true).await;
    submit_link_event(&c, &sk, &kid, a, x, 13, false).await;

    let list = list_patient_medications(&c, a).await.unwrap();
    assert_flag_agrees(&list);
    assert_eq!(
        list.charts.members(),
        sorted(vec![a, bridge, x]).as_slice(),
        "precondition: the bridge still joins A and X"
    );
    assert!(
        list.rows.iter().all(|r| !r.is_wrong_chart_hazard()),
        "a machine's unlink is no human judgement: nothing is held"
    );
    assert_eq!(sign_off_targets(&list.rows), sorted(vec![only_a, only_x]));
}

/// A doubt is a fact about ONE record. db/054 is handed one record's charts and every case
/// filters on them, so a doubted link in ANOTHER record on this node — of each of the three
/// kinds — must leave this record's lines signable. Without the `ANY(p_charts)` filters, one bad
/// synced link anywhere would freeze sign-off on every combined record.
#[tokio::test]
async fn a_doubt_in_another_record_leaves_this_record_alone() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    c.batch_execute("TRUNCATE patient_link, person_member, link_veto_flag")
        .await
        .unwrap();
    let (sk, kid, hsk, hkid) = setup(&c).await;

    // This record: A–X, an undoubted machine link.
    let a = chart(&c, &sk, &kid).await;
    let x = chart(&c, &sk, &kid).await;
    let only_x = assert_one(&mut c, &sk, &kid, x, "amlodipine").await;
    let only_a = assert_one(&mut c, &sk, &kid, a, "metformin").await;
    submit_link_event(&c, &sk, &kid, a, x, 10, true).await;

    // Case (a) elsewhere: B–Y, a flagged link.
    let b = chart(&c, &sk, &kid).await;
    let y = chart(&c, &sk, &kid).await;
    let only_y = assert_one(&mut c, &sk, &kid, y, "warfarin").await;
    submit_link_event(&c, &sk, &kid, b, y, 11, true).await;
    flag_link(&c, b, y).await;

    // Case (b) elsewhere: D–E, an un-attested link the veto trips now.
    let d = chart(&c, &sk, &kid).await;
    let e = chart(&c, &sk, &kid).await;
    submit_link_event(&c, &sk, &kid, d, e, 12, true).await;
    verified_dob(&c, &sk, &kid, d, "1980-07-15", 20).await;
    verified_dob(&c, &sk, &kid, e, "1975-01-02", 21).await;

    // Case (c) elsewhere: F–G–H, with an attested unlink F–H the bridge G still spans.
    let f = chart(&c, &sk, &kid).await;
    let g = chart(&c, &sk, &kid).await;
    let h = chart(&c, &sk, &kid).await;
    submit_link_event(&c, &sk, &kid, f, g, 13, true).await;
    submit_link_event(&c, &sk, &kid, g, h, 14, true).await;
    let who = Reviewer {
        human_sk: &hsk,
        human_kid: &hkid,
    };
    let out = unlink_charts(&mut c, f, h, Some(f), &who, ORIGIN)
        .await
        .unwrap();
    assert_eq!(out.effect, LinkEffect::StillJoined, "precondition");

    // Positive control: the other record IS doubted, so the doubt is real, just not ours.
    let other = list_patient_medications(&c, b).await.unwrap();
    assert_eq!(row_of(&other, only_y).wrong_chart, DOUBTED, "precondition");
    assert!(set_doubted(&c, &[d, e]).await, "precondition: case (b)");
    assert!(set_doubted(&c, &[f, g, h]).await, "precondition: case (c)");

    let list = list_patient_medications(&c, a).await.unwrap();
    assert_flag_agrees(&list);
    assert_eq!(list.charts.members(), sorted(vec![a, x]).as_slice());
    assert!(
        list.rows.iter().all(|r| !r.is_wrong_chart_hazard()),
        "another record's doubt must not hold this record's lines"
    );
    assert_eq!(sign_off_targets(&list.rows), sorted(vec![only_a, only_x]));
}

/// Both reasons on one row, THROUGH the database: `outside_set` is computed by the read from
/// the groups' charts, `doubted_link` by db/054 — the pure test in `hazard.rs` cannot show
/// that the read wires both. A–X is a flagged (doubted) link; O is an outsider linked to
/// nothing. A line spanning X and O reaches outside the set AND is not only on A; so does a
/// line spanning A and O (touching O, it is not "only on the opened chart").
#[tokio::test]
async fn a_line_can_be_withheld_for_both_reasons_at_once() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    c.batch_execute("TRUNCATE patient_link, person_member, link_veto_flag")
        .await
        .unwrap();
    let (sk, kid, _hsk, _hkid) = setup(&c).await;
    let a = chart(&c, &sk, &kid).await;
    let x = chart(&c, &sk, &kid).await;
    let outsider = chart(&c, &sk, &kid).await;
    let tx = assert_one(&mut c, &sk, &kid, x, "warfarin").await;
    let to = assert_one(&mut c, &sk, &kid, outsider, "warfarin").await;
    let ta = assert_one(&mut c, &sk, &kid, a, "digoxin").await;
    let ta_o = assert_one(&mut c, &sk, &kid, outsider, "digoxin").await;
    group(&c, tx, to).await;
    group(&c, ta, ta_o).await;
    submit_link_event(&c, &sk, &kid, a, x, 10, true).await;
    flag_link(&c, a, x).await;

    let list = list_patient_medications(&c, a).await.unwrap();
    assert_flag_agrees(&list);
    assert_eq!(list.charts.members(), sorted(vec![a, x]).as_slice());
    let both = WrongChartReasons {
        outside_set: true,
        doubted_link: true,
    };
    assert_eq!(row_of(&list, tx).wrong_chart, both, "X+O: both reasons");
    assert_eq!(
        row_of(&list, ta).wrong_chart,
        both,
        "A+O: outside the set, and touching O it is not only on A"
    );
    assert!(sign_off_targets(&list.rows).is_empty());
    assert!(withheld_rows(&list.rows).iter().all(|w| w.reasons == both));
}
