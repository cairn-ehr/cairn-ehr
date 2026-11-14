//! Charting slice 2, Task 4 — the node signs a give and a strike, and reads stat orders back.
//!
//! DB-gated on `$CAIRN_TEST_PG` (a skipped run prints `skipped:`), serialized via
//! `db::test_serial_guard` taken BEFORE connecting.
mod common;
#[path = "common/given_kit.rs"]
mod given_kit;
#[path = "common/medication_kit.rs"]
mod medication_kit;

use cairn_event::medication::{Charting, STAT_GIVEN_REASON};
use cairn_event::SigningKey;
use cairn_node::db;
use cairn_node::medication::stat_give::{give_stat, strike_given};
use cairn_node::medication::stat_read::read_stat_orders;
use cairn_node::medication::{chart_medication, AuthorParams};
use chrono::{Duration, Utc};
use common::{note_setup, submit_registration, NOTE_AUTHOR};
use tokio_postgres::Client;
use uuid::Uuid;

struct Rig {
    c: Client,
    nsk: SigningKey,
    nkid: String,
    hsk: SigningKey,
    hkid: String,
    chart: Uuid,
}

impl Rig {
    /// A line on `chart`, human-signed; a stat order when `stat`, else one with a route only.
    async fn order(&mut self, chart: Uuid, stat: bool, human: bool) -> Uuid {
        let charting = Charting {
            route: (!stat).then_some("po"),
            stat,
            ..Charting::default()
        };
        let author = AuthorParams {
            human_sk: &self.hsk,
            human_kid: &self.hkid,
        };
        chart_medication(
            &mut self.c,
            &self.nsk,
            &self.nkid,
            "test-node",
            chart,
            &medication_kit::sample_input(),
            &charting,
            human.then_some(&author),
            None,
        )
        .await
        .expect("the line signs")
        .value
    }

    async fn give(
        &mut self,
        chart: Uuid,
        med: Uuid,
        ago_min: i64,
    ) -> anyhow::Result<cairn_node::db_diagnosis::Committed<Uuid>> {
        let author = AuthorParams {
            human_sk: &self.hsk,
            human_kid: &self.hkid,
        };
        give_stat(
            &mut self.c,
            &self.nsk,
            &self.nkid,
            "test-node",
            chart,
            med,
            Utc::now() - Duration::minutes(ago_min),
            &author,
        )
        .await
    }

    async fn strike(
        &mut self,
        chart: Uuid,
        given: Uuid,
        text: Option<&str>,
    ) -> anyhow::Result<cairn_node::db_diagnosis::Committed<Uuid>> {
        let author = AuthorParams {
            human_sk: &self.hsk,
            human_kid: &self.hkid,
        };
        strike_given(
            &mut self.c,
            &self.nsk,
            &self.nkid,
            "test-node",
            chart,
            given,
            text,
            &author,
        )
        .await
    }
}

async fn rig(base: &str) -> Rig {
    let c = db::connect_and_load_schema(base).await.unwrap();
    let (nsk, nkid, hsk, hkid, chart) = note_setup(&c).await;
    c.batch_execute("TRUNCATE medication_charting, medication_given, medication_given_retraction")
        .await
        .unwrap();
    Rig {
        c,
        nsk,
        nkid,
        hsk,
        hkid,
        chart,
    }
}

async fn events(c: &Client) -> i64 {
    c.query_one("SELECT count(*) FROM event_log", &[])
        .await
        .unwrap()
        .get(0)
}

macro_rules! db_test {
    ($base:ident) => {
        let Some($base) = common::cs() else {
            eprintln!("skipped: set CAIRN_TEST_PG");
            return;
        };
        let _g = db::test_serial_guard(&$base).await.unwrap();
    };
}

#[tokio::test]
async fn give_stat_stops_the_line_and_records_the_given_in_one_transaction() {
    db_test!(base);
    let mut r = rig(&base).await;
    let med = r.order(r.chart, true, true).await;
    assert!(given_kit::is_current(&r.c, med).await);
    let before = events(&r.c).await;
    let g = r.give(r.chart, med, 5).await.expect("the give signs");
    assert_eq!(events(&r.c).await, before + 2, "a stop and a given");
    assert!(!given_kit::is_current(&r.c, med).await);
    let reason: Option<String> =
        r.c.query_one(
            "SELECT reason FROM medication_cessation WHERE medication_id = $1::text::uuid",
            &[&med.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(reason.as_deref(), Some(STAT_GIVEN_REASON));
    let completes: Option<String> =
        r.c.query_one(
            "SELECT completes::text FROM medication_given WHERE event_id = $1::text::uuid",
            &[&g.value.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert!(completes.is_some(), "the given completes its stop");
}

#[tokio::test]
async fn a_failed_given_rolls_back_the_stop_too() {
    db_test!(base);
    let mut r = rig(&base).await;
    let med = r.order(r.chart, false, true).await; // not a stat order: the given's floor refuses
    let before = events(&r.c).await;
    assert!(r.give(r.chart, med, 5).await.is_err());
    assert_eq!(events(&r.c).await, before, "nothing was written");
    assert!(
        given_kit::is_current(&r.c, med).await,
        "the stop rolled back"
    );
}

#[tokio::test]
async fn strike_given_returns_the_line_to_current() {
    db_test!(base);
    let mut r = rig(&base).await;
    let med = r.order(r.chart, true, true).await;
    let g = r.give(r.chart, med, 5).await.unwrap().value;
    assert!(!given_kit::is_current(&r.c, med).await);
    r.strike(r.chart, g, Some("  "))
        .await
        .expect("the strike signs");
    assert!(given_kit::is_current(&r.c, med).await);
}

#[tokio::test]
async fn strike_given_refuses_another_charts_given() {
    db_test!(base);
    let mut r = rig(&base).await;
    let med = r.order(r.chart, true, true).await;
    let g = r.give(r.chart, med, 5).await.unwrap().value;
    let other = Uuid::now_v7();
    submit_registration(&r.c, &r.nsk, &r.nkid, other, 0).await;
    let before = events(&r.c).await;
    assert!(r.strike(other, g, None).await.is_err());
    assert_eq!(events(&r.c).await, before, "no event was written");
    assert!(!given_kit::is_current(&r.c, med).await);
}

#[tokio::test]
async fn read_stat_orders_names_orderer_and_giver_in_time_order() {
    db_test!(base);
    let mut r = rig(&base).await;
    let med = r.order(r.chart, true, true).await;
    let late = r.give(r.chart, med, 5).await.unwrap().value;
    r.strike(r.chart, late, None).await.unwrap();
    let early = r.give(r.chart, med, 30).await.unwrap().value;
    let orders = read_stat_orders(&r.c, &[r.chart]).await.unwrap();
    assert_eq!(orders.len(), 1);
    let o = &orders[0];
    assert_eq!((o.medication_id, o.patient_id), (med, r.chart));
    assert_eq!(o.orderer.as_deref(), Some(NOTE_AUTHOR));
    let ids: Vec<Uuid> = o.gives.iter().map(|g| g.event_id).collect();
    assert_eq!(ids, [early, late], "oldest time given first");
    assert!(o
        .gives
        .iter()
        .all(|g| g.giver.as_deref() == Some(NOTE_AUTHOR)));
    assert!(o.gives.iter().all(|g| g.completes));
    // Another chart's read never sees it.
    assert!(read_stat_orders(&r.c, &[Uuid::now_v7()])
        .await
        .unwrap()
        .is_empty());
}

#[tokio::test]
async fn an_unvouched_signer_reads_as_none() {
    db_test!(base);
    let mut r = rig(&base).await;
    let med = r.order(r.chart, true, false).await; // signed by the device key alone
    let orders = read_stat_orders(&r.c, &[r.chart]).await.unwrap();
    assert_eq!(orders[0].medication_id, med);
    assert_eq!(orders[0].orderer, None);
}

#[tokio::test]
async fn a_struck_give_is_read_with_its_striker_and_why() {
    db_test!(base);
    let mut r = rig(&base).await;
    let med = r.order(r.chart, true, true).await;
    let g = r.give(r.chart, med, 5).await.unwrap().value;
    r.strike(r.chart, g, Some("wrong patient")).await.unwrap();
    let o = read_stat_orders(&r.c, &[r.chart]).await.unwrap().remove(0);
    let s = o.gives[0].struck.as_ref().expect("struck");
    assert_eq!(s.who.as_deref(), Some(NOTE_AUTHOR));
    assert_eq!(s.why.as_deref(), Some("wrong patient"));
    assert!(s.at >= o.ordered_at);
}

#[tokio::test]
async fn waiting_is_true_until_a_standing_give() {
    db_test!(base);
    let mut r = rig(&base).await;
    let med = r.order(r.chart, true, true).await;
    async fn read(r: &Rig) -> cairn_node::medication::stat_read::StatOrder {
        read_stat_orders(&r.c, &[r.chart]).await.unwrap().remove(0)
    }
    assert!(read(&r).await.waiting(), "ordered, not given");
    let g = r.give(r.chart, med, 5).await.unwrap().value;
    assert!(!read(&r).await.waiting(), "given");
    r.strike(r.chart, g, None).await.unwrap();
    assert!(read(&r).await.waiting(), "the give was struck");
}
