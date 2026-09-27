//! ADR-0076 R1, decisions 2 and 3: the whole-list sign-off over a chart SET.
//!
//! Once a chart reads as one list over every chart it is linked to, one sign-off gesture
//! covers lines recorded on several charts. Two rules keep that gesture a truthful signature:
//!
//! - **Each thread is attested under the chart it LIVES ON** (decision 2), never under the
//!   chart the list happened to be opened from. Attesting chart B's drug on chart A would
//!   put a responsibility-bearing clinical signature on the wrong chart — the defect this
//!   suite exists to keep out.
//! - **A changed chart set refuses the gesture** (decision 3). The clinician vouched for
//!   the list they SAW; if a link or unlink landed while it was on screen, the list they
//!   would be signing is not that list, and nothing may be signed on their behalf.
//!
//! Kept apart from `combined_read.rs` (the READ over a set) so each file stays one subject
//! and a reviewable length. DB-gated on $CAIRN_TEST_PG, serialized cluster-wide via
//! `db::test_serial_guard`, taken BEFORE connecting (see `medication_signoff.rs`'s header
//! for why that order matters for a gesture that reads nearly every medication view). Key
//! material is minted at runtime by `medication_setup` (house rule 6).
mod common;
use cairn_event::SigningKey;
use cairn_medication_view::ChartSet;
use cairn_node::db;
use cairn_node::medication::read::list_patient_medications;
use cairn_node::medication::signoff::sign_off_medication_list;
use cairn_node::medication::{assert_medication, AssertMedicationInput, AttestParams};
use common::{
    attestation_count, cs, medication_setup as setup, submit_link_event, submit_registration,
};
use tokio_postgres::Client;
use uuid::Uuid;

/// One registered chart (#345: the birth act precedes everything recorded about it).
/// A file-local copy of `combined_read.rs`'s helper of the same name — two suites, three
/// lines each, is below the bar for `common/`.
async fn chart(c: &Client, sk: &SigningKey, kid: &str) -> Uuid {
    let p = Uuid::now_v7();
    submit_registration(c, sk, kid, p, 0).await;
    p
}

/// Assert one uncoded active medication on `patient`; returns its thread id. Same shape
/// as `combined_read.rs`'s helper (every drug carries the same dose and differs only in its
/// name, which is all these tests are about).
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

/// The human vouch every gesture here is made with: no basis, no note.
fn params<'a>(hsk: &'a SigningKey, hkid: &'a str) -> AttestParams<'a> {
    AttestParams {
        human_sk: hsk,
        human_kid: hkid,
        basis: None,
        note: None,
    }
}

/// The chart an event was recorded under — read straight from the envelope, because
/// that column, not anything the orchestrator reports, is what the record will say.
async fn chart_of_event(c: &Client, event: Uuid) -> Uuid {
    let row = c
        .query_one(
            "SELECT patient_id::text AS p FROM event_log WHERE event_id = $1::text::uuid",
            &[&event.to_string()],
        )
        .await
        .unwrap();
    row.get::<_, String>("p").parse().unwrap()
}

/// Everything one test needs: the serial guard (held for the test's whole life — it is the
/// advisory lock on its own connection), a client on a clean medication database with no
/// links, and `medication_setup`'s node + human keys.
type Fixture = (Client, Client, (SigningKey, String, SigningKey, String));

/// The preamble every test shares. `None` without `CAIRN_TEST_PG`, and the caller then
/// returns early — a skip, which is NOT a pass (the message says so on stderr).
async fn open() -> Option<Fixture> {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return None;
    };
    let guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    c.batch_execute("TRUNCATE patient_link, person_member")
        .await
        .unwrap();
    let keys = setup(&c).await;
    Some((guard, c, keys))
}

/// THE DEFECT THIS TASK REMOVES. One gesture over a linked pair signs both charts' lines,
/// and each attestation is recorded on the chart its thread lives on — metformin's on A,
/// amlodipine's on B — even though the list was opened from A.
#[tokio::test]
async fn a_combined_sign_off_attests_each_thread_under_its_own_chart() {
    let Some((_guard, mut c, (sk, kid, hsk, hkid))) = open().await else {
        return;
    };
    let a = chart(&c, &sk, &kid).await;
    let b = chart(&c, &sk, &kid).await;
    let met = assert_one(&mut c, &sk, &kid, a, "metformin").await;
    let aml = assert_one(&mut c, &sk, &kid, b, "amlodipine").await;
    submit_link_event(&c, &sk, &kid, a, b, 10, true).await;
    let shown = list_patient_medications(&c, a).await.unwrap().charts;

    let out = sign_off_medication_list(
        &mut c,
        &sk,
        "origin-a",
        &params(&hsk, &hkid),
        a,
        Some(&shown),
    )
    .await
    .unwrap();

    assert!(out.failed.is_empty(), "{:?}", out.failed);
    assert_eq!(
        out.attested.len(),
        2,
        "one gesture covers both charts' lines"
    );
    for (thread, event) in out.attested.iter().zip(&out.event_ids) {
        let expected = if *thread == met {
            a
        } else {
            assert_eq!(*thread, aml);
            b
        };
        assert_eq!(
            chart_of_event(&c, *event).await,
            expected,
            "thread {thread} is attested under the chart it lives on, not the opened chart"
        );
    }
    assert_eq!(
        out.charts, shown,
        "the outcome names the set it signed across"
    );
}

/// A link landing while the list is on screen changes what the gesture would sign (B's
/// drugs join the list the clinician never saw), so it is refused and nothing is written.
#[tokio::test]
async fn a_sign_off_is_refused_when_the_chart_set_changed() {
    let Some((_guard, mut c, (sk, kid, hsk, hkid))) = open().await else {
        return;
    };
    let a = chart(&c, &sk, &kid).await;
    let b = chart(&c, &sk, &kid).await;
    let met = assert_one(&mut c, &sk, &kid, a, "metformin").await;
    let aml = assert_one(&mut c, &sk, &kid, b, "amlodipine").await;
    let shown = list_patient_medications(&c, a).await.unwrap().charts; // {a}: what was on screen
    submit_link_event(&c, &sk, &kid, a, b, 10, true).await; // lands while the list is shown

    let err = sign_off_medication_list(
        &mut c,
        &sk,
        "origin-a",
        &params(&hsk, &hkid),
        a,
        Some(&shown),
    )
    .await
    .unwrap_err();
    assert!(
        format!("{err:#}").contains("linked charts changed"),
        "{err:#}"
    );
    assert_eq!(attestation_count(&c, met).await, 0, "nothing was signed");
    assert_eq!(attestation_count(&c, aml).await, 0, "nothing was signed");
}

/// The other direction: an UNLINK landing while a combined list is on screen shrinks the
/// set. Signing now would vouch for a list missing lines the clinician saw — refused too.
#[tokio::test]
async fn a_sign_off_is_refused_when_an_unlink_shrank_the_set() {
    let Some((_guard, mut c, (sk, kid, hsk, hkid))) = open().await else {
        return;
    };
    let a = chart(&c, &sk, &kid).await;
    let b = chart(&c, &sk, &kid).await;
    let met = assert_one(&mut c, &sk, &kid, a, "metformin").await;
    submit_link_event(&c, &sk, &kid, a, b, 10, true).await;
    let shown = list_patient_medications(&c, a).await.unwrap().charts; // {a, b}
    assert!(
        shown.is_linked(),
        "precondition: the list was read over the pair"
    );
    submit_link_event(&c, &sk, &kid, a, b, 11, false).await;

    let err = sign_off_medication_list(
        &mut c,
        &sk,
        "origin-a",
        &params(&hsk, &hkid),
        a,
        Some(&shown),
    )
    .await
    .unwrap_err();
    assert!(
        format!("{err:#}").contains("linked charts changed"),
        "{err:#}"
    );
    assert_eq!(attestation_count(&c, met).await, 0, "nothing was signed");
}

/// The common case is untouched: a never-linked chart, signed with the set it was shown
/// (a set of one), signs exactly as it always did, under that chart.
#[tokio::test]
async fn an_unlinked_chart_signs_with_its_displayed_set_of_one() {
    let Some((_guard, mut c, (sk, kid, hsk, hkid))) = open().await else {
        return;
    };
    let a = chart(&c, &sk, &kid).await;
    let met = assert_one(&mut c, &sk, &kid, a, "metformin").await;

    let out = sign_off_medication_list(
        &mut c,
        &sk,
        "origin-a",
        &params(&hsk, &hkid),
        a,
        Some(&ChartSet::single(a)),
    )
    .await
    .unwrap();
    assert_eq!(out.attested, vec![met]);
    assert_eq!(chart_of_event(&c, out.event_ids[0]).await, a);
    assert_eq!(out.charts, ChartSet::single(a));
}

/// The CLI shows no list before signing, so it has no displayed set to compare and passes
/// `None`: it signs the set it finds, and the outcome says which set that was.
#[tokio::test]
async fn the_cli_form_signs_the_set_it_finds() {
    let Some((_guard, mut c, (sk, kid, hsk, hkid))) = open().await else {
        return;
    };
    let a = chart(&c, &sk, &kid).await;
    let b = chart(&c, &sk, &kid).await;
    assert_one(&mut c, &sk, &kid, a, "metformin").await;
    let aml = assert_one(&mut c, &sk, &kid, b, "amlodipine").await;
    submit_link_event(&c, &sk, &kid, a, b, 10, true).await;
    let out = sign_off_medication_list(&mut c, &sk, "origin-a", &params(&hsk, &hkid), a, None)
        .await
        .unwrap();
    assert_eq!(out.attested.len(), 2);
    assert_eq!(out.charts, ChartSet::new([a, b]).unwrap());
    let aml_event = out.event_ids[out.attested.iter().position(|t| *t == aml).unwrap()];
    assert_eq!(
        chart_of_event(&c, aml_event).await,
        b,
        "the CLI form attests under each thread's own chart too"
    );
}
