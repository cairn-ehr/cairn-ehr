//! Batch charting slice 2b, Task 5 — the node settles a set-aside version.
//!
//! A RESTORE is the stop of every line that stands for the companion (ruling R3) plus a new line
//! carrying `restores`, in ONE transaction with the spend of its draft row; a CHECK is one event
//! of its own. `read_settlements` reads both back for display, and `version_for_restore` is the
//! fresh read Sign builds a restore from (what stands = db/080's lineage function, ruling R4).
//!
//! DB-gated on `$CAIRN_TEST_PG` (a skipped run prints `skipped:`), serialized via
//! `db::test_serial_guard` taken BEFORE connecting.
mod common;
#[macro_use]
#[path = "common/version_settle_kit.rs"]
mod version_settle_kit;

use cairn_event::medication::{ChartedAs, Charting, SourceNote};
use cairn_node::db;
use cairn_node::medication::draft::{read_draft_lines, SpendRefused};
use cairn_node::medication::version_settle::{check_version, RestoreToSign, VersionCheck};
use cairn_node::medication::version_settle_read::{
    read_settlements, version_for_restore, Settlement,
};
use cairn_node::medication::AuthorParams;
use common::{submit_registration, NOTE_AUTHOR};
use uuid::Uuid;
use version_settle_kit::*;

#[tokio::test]
async fn a_restore_stops_its_companion_and_charts_the_line_in_one_transaction() {
    db_test!(base);
    let mut r = rig(&base).await;
    let x = charted(&mut r).await;
    let v = version_beside(&mut r, x).await;
    assert_eq!(standing(&r, v).await, [x]);
    let before = count(&r.c, "event_log").await;
    let done = restore(&mut r, v, &[x]).await.expect("the restore signs");
    let line = done.value.0;
    assert_eq!(
        count(&r.c, "event_log").await,
        before + 2,
        "a stop and a line"
    );
    assert_eq!(
        stop_reason(&r.c, x).await,
        Some(Some("replaced by the GP letter version".into()))
    );
    let restores: String =
        r.c.query_one(
            "SELECT version_id::text FROM medication_restore WHERE medication_id = $1::text::uuid",
            &[&line.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(restores, v.to_string());
    assert!(read_draft_lines(&r.c, r.chart, Some("notewriter"))
        .await
        .unwrap()
        .is_empty());
    assert_eq!(standing(&r, v).await, [line], "one current line");
}

#[tokio::test]
async fn a_restore_of_a_stopped_companions_version_emits_no_stop() {
    db_test!(base);
    let mut r = rig(&base).await;
    let x = charted(&mut r).await;
    let v = version_beside(&mut r, x).await;
    cease(&mut r, x).await;
    let read = version_for_restore(&r.c, r.chart, v)
        .await
        .unwrap()
        .unwrap();
    assert!(!read.companion_current);
    assert!(read.standing.is_empty());
    let before = count(&r.c, "event_log").await;
    let done = restore(&mut r, v, &read.standing).await.unwrap();
    assert_eq!(count(&r.c, "event_log").await, before + 1, "the line only");
    assert_eq!(standing(&r, v).await, [done.value.0]);
}

/// Ruling R3: X has versions V1 and V2. Restoring V1 stops X; restoring V2 next must stop V1's
/// line, which now stands for X.
#[tokio::test]
async fn a_second_restore_stops_the_first_restores_line() {
    db_test!(base);
    let mut r = rig(&base).await;
    let x = charted(&mut r).await;
    let (v1, v2) = (
        version_beside(&mut r, x).await,
        version_beside(&mut r, x).await,
    );
    let l1 = restore(&mut r, v1, &[x]).await.unwrap().value.0;
    let read = version_for_restore(&r.c, r.chart, v2)
        .await
        .unwrap()
        .unwrap();
    assert!(!read.companion_current);
    assert_eq!(read.standing, [l1]);
    assert_eq!(
        read.standing_restored,
        [l1],
        "what stands is itself a restore"
    );
    let l2 = restore(&mut r, v2, &read.standing).await.unwrap().value.0;
    assert!(
        stop_reason(&r.c, l1).await.is_some(),
        "the first restore's line is stopped"
    );
    assert_eq!(standing(&r, v2).await, [l2]);
}

#[tokio::test]
async fn a_failed_spend_rolls_back_the_stop_too() {
    db_test!(base);
    let mut r = rig(&base).await;
    let x = charted(&mut r).await;
    let v = version_beside(&mut r, x).await;
    let (line, _) = restore_line(&r, v).await;
    let before = count(&r.c, "event_log").await;
    let rs = RestoreToSign {
        version_id: v,
        stop: vec![(x, "replaced by the GP letter version")],
    };
    let err = sign(
        &mut r,
        line,
        "2000-01-01 00:00:00+00",
        &Charting::default(),
        Some(&rs),
    )
    .await
    .unwrap_err();
    assert!(err.downcast_ref::<SpendRefused>().is_some(), "{err:#}");
    assert_eq!(count(&r.c, "event_log").await, before, "nothing written");
    assert_eq!(
        stop_reason(&r.c, x).await,
        None,
        "the companion is not stopped"
    );
    assert_eq!(count(&r.c, "medication_restore").await, 0);
    assert_eq!(standing(&r, v).await, [x]);
}

/// Review Focus 5: someone else stops the companion between Sign's fresh read and the commit.
/// db/031 ADMITS a second cessation of a stopped thread (its floor checks only the
/// medication_id; the projection keeps one row per thread, HLC-winner), so the restore's own stop
/// goes in beside the other, the line goes in, and exactly one line stands.
#[tokio::test]
async fn a_companion_stopped_since_the_read_still_leaves_one_current_line() {
    db_test!(base);
    let mut r = rig(&base).await;
    let x = charted(&mut r).await;
    let v = version_beside(&mut r, x).await;
    let read = version_for_restore(&r.c, r.chart, v)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(read.standing, [x]);
    cease(&mut r, x).await; // the colleague's stop, after the read
    let line = restore(&mut r, v, &read.standing)
        .await
        .expect("a second stop of a stopped line is admitted")
        .value
        .0;
    let stops: i64 =
        r.c.query_one(
            "SELECT count(*) FROM medication_cessation WHERE medication_id = $1::text::uuid",
            &[&x.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(stops, 1, "the chart shows one stop of the companion");
    assert_eq!(standing(&r, v).await, [line], "never two current lines");
}

#[tokio::test]
async fn check_version_signs_one_event() {
    db_test!(base);
    let mut r = rig(&base).await;
    let x = charted(&mut r).await;
    let v = version_beside(&mut r, x).await;
    let before = count(&r.c, "event_log").await;
    let check = VersionCheck {
        version_id: v,
        info_source: "external-record",
        source_note: Some(SourceNote {
            source_detail: Some("pharmacy"),
            why: Some("dispensed 40 mg"),
        }),
    };
    let done = {
        let chart = r.chart;
        sign_check(&mut r, chart, &check)
    }
    .await
    .expect("the check signs");
    assert_eq!(count(&r.c, "event_log").await, before + 1);
    let row =
        r.c.query_one(
            "SELECT event_id::text, version_id::text, info_source, source_detail, why \
               FROM medication_set_aside_checked",
            &[],
        )
        .await
        .unwrap();
    assert_eq!(row.get::<_, String>(0), done.event_id.to_string());
    assert_eq!(row.get::<_, String>(1), v.to_string());
    assert_eq!(row.get::<_, String>(2), "external-record");
    assert_eq!(row.get::<_, Option<String>>(3).as_deref(), Some("pharmacy"));
    assert_eq!(
        row.get::<_, Option<String>>(4).as_deref(),
        Some("dispensed 40 mg")
    );
}

#[tokio::test]
async fn check_version_refuses_another_charts_version() {
    db_test!(base);
    let mut r = rig(&base).await;
    let x = charted(&mut r).await;
    let v = version_beside(&mut r, x).await;
    let other = Uuid::now_v7();
    submit_registration(&r.c, &r.nsk, &r.nkid, other, 0).await;
    let before = count(&r.c, "event_log").await;
    let check = VersionCheck {
        version_id: v,
        info_source: "external-record",
        source_note: None,
    };
    let err = sign_check(&mut r, other, &check).await.unwrap_err();
    assert!(
        format!("{err:#}").contains("is not a set-aside version on this chart"),
        "{err:#}"
    );
    assert_eq!(count(&r.c, "event_log").await, before);
    assert_eq!(count(&r.c, "medication_set_aside_checked").await, 0);
}

#[tokio::test]
async fn read_settlements_names_the_signer_and_the_replaced_line() {
    db_test!(base);
    let mut r = rig(&base).await;
    let x = charted(&mut r).await;
    let v = version_beside(&mut r, x).await;
    let check = VersionCheck {
        version_id: v,
        info_source: "external-record",
        source_note: Some(SourceNote {
            source_detail: Some("pharmacy"),
            why: Some("dispensed 40 mg"),
        }),
    };
    {
        let chart = r.chart;
        sign_check(&mut r, chart, &check)
    }
    .await
    .unwrap();
    let line = restore(&mut r, v, &[x]).await.unwrap().value.0;
    // Another chart's settlements are never read with this one's.
    let rows = read_settlements(&r.c, &[r.chart]).await.unwrap();
    assert_eq!(rows.len(), 2, "{rows:?}");
    assert!(rows[0].at <= rows[1].at, "oldest first");
    assert_eq!(
        rows[0].settlement,
        Settlement::Checked {
            source: "pharmacy".into(),
            why: Some("dispensed 40 mg".into()),
        }
    );
    assert_eq!(
        rows[1].settlement,
        Settlement::Restored {
            line,
            replaced_line: "Lipitor 40 mg (clinician-observed)".into(),
        }
    );
    for row in &rows {
        assert_eq!(row.version_id, v);
        assert_eq!(row.patient_id, r.chart);
        assert_eq!(row.who.as_deref(), Some(NOTE_AUTHOR));
    }
    assert!(read_settlements(&r.c, &[Uuid::now_v7()])
        .await
        .unwrap()
        .is_empty());
}

/// A check whose signer is not a person (here the node's own device key as the "author") is
/// never credited to a name: `who` is None, and the window says "signer not known".
#[tokio::test]
async fn an_unvouched_signer_reads_as_none() {
    db_test!(base);
    let mut r = rig(&base).await;
    let x = charted(&mut r).await;
    let v = version_beside(&mut r, x).await;
    let check = VersionCheck {
        version_id: v,
        info_source: "external-record",
        source_note: None,
    };
    let device = AuthorParams {
        human_sk: &r.nsk,
        human_kid: &r.nkid,
    };
    check_version(
        &mut r.c,
        &r.nsk,
        &r.nkid,
        "test-node",
        r.chart,
        &check,
        &device,
    )
    .await
    .unwrap();
    let rows = read_settlements(&r.c, &[r.chart]).await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].who, None);
}

#[tokio::test]
async fn version_for_restore_carries_the_companions_charted_as_and_currency() {
    db_test!(base);
    let mut r = rig(&base).await;
    let x = charted_as(
        &mut r,
        &Charting {
            charted_as: Some(ChartedAs::PreExisting),
            ..Charting::default()
        },
    )
    .await;
    let v = version_beside(&mut r, x).await;
    let read = version_for_restore(&r.c, r.chart, v)
        .await
        .unwrap()
        .expect("on this chart");
    assert_eq!(read.alternative_to, x);
    assert!(read.companion_current);
    assert_eq!(read.standing, [x]);
    assert!(
        read.standing_restored.is_empty(),
        "the original line is no restore"
    );
    assert_eq!(read.companion_charted_as.as_deref(), Some("pre-existing"));
    assert_eq!(read.source, "GP letter");
    assert_eq!(read.term, "Lipitor");
    assert_eq!(read.dose_amount.as_deref(), Some("25"));
    assert_eq!(read.dose_unit.as_deref(), Some("mg"));
    assert_eq!(read.coding, None);
    // Not on another chart, and not at all for an unknown version.
    assert!(version_for_restore(&r.c, Uuid::now_v7(), v)
        .await
        .unwrap()
        .is_none());
    assert!(version_for_restore(&r.c, r.chart, Uuid::now_v7())
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn chart_draft_line_with_no_restore_is_unchanged() {
    db_test!(base);
    let mut r = rig(&base).await;
    let line = Uuid::now_v7();
    let at = save(&r, line, None).await;
    let before = count(&r.c, "event_log").await;
    let done = sign(&mut r, line, &at, &Charting::default(), None)
        .await
        .unwrap();
    assert!(done.value.1.is_empty());
    assert_eq!(count(&r.c, "event_log").await, before + 1);
    assert_eq!(count(&r.c, "medication_restore").await, 0);
    assert_eq!(count(&r.c, "medication_cessation").await, 0);
}

/// One source of truth for `restores`: a caller that sets it on `charting` AND passes a restore
/// is refused before anything is written.
#[tokio::test]
async fn a_restore_with_its_own_charting_restores_is_refused() {
    db_test!(base);
    let mut r = rig(&base).await;
    let x = charted(&mut r).await;
    let v = version_beside(&mut r, x).await;
    let (line, at) = restore_line(&r, v).await;
    let before = count(&r.c, "event_log").await;
    let vs = v.to_string();
    let rs = RestoreToSign {
        version_id: v,
        stop: vec![(x, "replaced")],
    };
    let charting = Charting {
        restores: Some(&vs),
        ..Charting::default()
    };
    assert!(sign(&mut r, line, &at, &charting, Some(&rs)).await.is_err());
    assert_eq!(count(&r.c, "event_log").await, before);
}

/// Ruling R10: the lineage is ROOTED. X charted; L1 restored v1 (X stopped); v3 set aside beside
/// L1 while an older v2 is still beside X. A restores v2 (stopping L1) in one transaction; B then
/// restores v3 in another, stopping L1 again but not A. B's companion is L1, whose DOWNWARD
/// closure alone ({L1, B}) misses A, its sibling under X: B must fail at commit, and then
/// succeed once it stops A, leaving one current line.
#[tokio::test]
async fn a_restore_of_a_version_set_aside_beside_a_restore_line_is_checked_against_the_whole_lineage(
) {
    db_test!(base);
    let mut r = rig(&base).await;
    let x = charted(&mut r).await;
    let (v1, v2) = (
        version_beside(&mut r, x).await,
        version_beside(&mut r, x).await,
    );
    let l1 = restore(&mut r, v1, &[x]).await.unwrap().value.0;
    let v3 = version_beside(&mut r, l1).await;
    let a = restore(&mut r, v2, &[l1]).await.unwrap().value.0;
    let refused = restore(&mut r, v3, &[l1]).await;
    assert!(
        format!("{:#}", refused.as_ref().unwrap_err()).contains("still stands"),
        "{refused:?}"
    );
    assert_eq!(
        standing(&r, v3).await,
        [a],
        "the whole lineage, read from L1"
    );
    let b = restore(&mut r, v3, &[a]).await.unwrap().value.0;
    assert_eq!(standing(&r, v2).await, [b], "one current line");
}
