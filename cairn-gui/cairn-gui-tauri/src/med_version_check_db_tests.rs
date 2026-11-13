//! DB-gated walks for batch charting slice 2b's "Checked: not current" (`med_version_check.rs`):
//! one immediate signed act on a set-aside version of the opened chart, its "From:" mapped as the
//! list's is, the pane read again. Nested under `med_list_restore_db_tests.rs` to share its
//! fixture (a charted line beside a set-aside version). Run locally on `CAIRN_TEST_PG` (#905).
use super::super::db_tests::bea_list;
use super::charted_with_versions;
use crate::med_version_check::med_version_check_impl;
use crate::notes::review::db_tests::credential;
use cairn_node::medication::version_settle_read::{read_settlements, Settlement};
use tokio_postgres::Client;
use uuid::Uuid;

/// The checks on `chart` as `(info_source, source_detail, why)` (read as the owner).
async fn checks(owner: &Client, chart: Uuid) -> Vec<(String, Option<String>, Option<String>)> {
    owner
        .query(
            "SELECT info_source, source_detail, why FROM medication_set_aside_checked \
              WHERE patient_id = $1::text::uuid",
            &[&chart.to_string()],
        )
        .await
        .unwrap()
        .iter()
        .map(|r| (r.get(0), r.get(1), r.get(2)))
        .collect()
}

/// A check names its "From:" (the preset's category, its label as the detail) and why, signs one
/// event, and answers with the pane read again; the check reads back under the version.
#[tokio::test]
async fn a_check_signs_and_reads_back() {
    let Some((_guard, owner, state, chart)) = bea_list().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let (_, versions) = charted_with_versions(&state, &owner, chart, &["25mg tab bd"]).await;
    let patient = chart.to_string();
    let written = med_version_check_impl(
        &state,
        &patient,
        vec![patient.clone()],
        &versions[0].to_string(),
        Some("GP letter".into()),
        None,
        Some("old packet".into()),
        credential("bea"),
    )
    .await
    .unwrap();
    assert!(written.pane.is_some(), "{}", written.outcome);
    assert_eq!(
        checks(&owner, chart).await,
        [(
            "external-record".to_string(),
            Some("GP letter".to_string()),
            Some("old packet".to_string())
        )]
    );
    let settled = read_settlements(&owner, &[chart]).await.unwrap();
    assert_eq!(settled.len(), 1);
    assert_eq!(settled[0].version_id, versions[0]);
    assert_eq!(
        settled[0].settlement,
        Settlement::Checked {
            source: "GP letter".into(),
            why: Some("old packet".into()),
        }
    );
}

/// Ruling R1: with no preset, the check signs `unknown`, the typed detail kept as it was typed;
/// the category is never guessed from it.
#[tokio::test]
async fn a_check_with_no_preset_signs_unknown() {
    let Some((_guard, owner, state, chart)) = bea_list().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let (_, versions) = charted_with_versions(&state, &owner, chart, &["25mg tab bd"]).await;
    let patient = chart.to_string();
    for detail in [Some("pharmacy printout"), None] {
        med_version_check_impl(
            &state,
            &patient,
            vec![patient.clone()],
            &versions[0].to_string(),
            None,
            detail.map(Into::into),
            None,
            credential("bea"),
        )
        .await
        .unwrap();
    }
    let mut got = checks(&owner, chart).await;
    got.sort();
    assert_eq!(
        got,
        [
            ("unknown".to_string(), None, None),
            (
                "unknown".to_string(),
                Some("pharmacy printout".to_string()),
                None
            ),
        ]
    );
}
