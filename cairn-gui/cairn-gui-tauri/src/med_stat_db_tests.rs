//! DB-gated walks for charting slice 2 in the window (`med_stat.rs`, `chart_stat.rs`): a stat line
//! signs `stat: true` from both Sign paths; Given signs and the line leaves the current list;
//! Entered in error returns it; a stale press is refused BEFORE the key, writing nothing. Nested
//! under `med_list.rs` to share `med_list_db_tests.rs`'s fixture (Dr Bea named, a John Doe
//! chart). Run locally on `CAIRN_TEST_PG` (#905: CI never runs these).
use super::db_tests::{bea_list, done, pick, save, sign};
use super::*;
use crate::chart_stat::STAT_NOT_READ;
use crate::med_chart::{med_chart_impl, ChartArg};
use crate::med_stat::{med_stat_give_impl, med_stat_strike_impl, GIVE_GONE, STAT_NOT_OPEN};
use crate::notes::review::db_tests::credential;
use cairn_node::medication::stat_read::{read_stat_orders, StatOrder};
use tokio_postgres::Client;

/// A drugref moiety code for ceftriaxone in these walks (no drug list is installed).
const CEFTRIAXONE: u128 = 0xCEF7_0001;

/// The clear payloads of every medication assertion on `chart` (read as the owner).
async fn payloads(owner: &Client, chart: Uuid) -> Vec<serde_json::Value> {
    owner
        .query(
            "SELECT cairn_clear_payload(e)::text FROM event_log e \
              WHERE e.event_type = 'clinical.medication.asserted' \
                AND e.patient_id = $1::text::uuid ORDER BY e.hlc_wall",
            &[&chart.to_string()],
        )
        .await
        .unwrap()
        .iter()
        .map(|r| serde_json::from_str(r.get::<_, &str>(0)).unwrap())
        .collect()
}

/// How many times the vault was asked for a key: unchanged means "refused before the key".
async fn vault_attempts(owner: &Client) -> i64 {
    owner
        .query_one("SELECT count(*) FROM vault_attempt", &[])
        .await
        .unwrap()
        .get(0)
}

/// How many events of `event_type` the database holds.
async fn count(owner: &Client, event_type: &str) -> i64 {
    owner
        .query_one(
            "SELECT count(*) FROM event_log WHERE event_type = $1",
            &[&event_type],
        )
        .await
        .unwrap()
        .get(0)
}

/// Whether `medication_id` reads current (in `patient_medication_current`).
async fn current(owner: &Client, medication_id: Uuid) -> bool {
    owner
        .query_one(
            "SELECT EXISTS (SELECT 1 FROM patient_medication_current \
                             WHERE medication_id = $1::text::uuid)",
            &[&medication_id.to_string()],
        )
        .await
        .unwrap()
        .get(0)
}

/// Chart "ceftriaxone 1g iv stat" on `chart` through the list's Sign; its one stat order.
async fn stat_line(state: &AppState, owner: &Client, chart: Uuid) -> StatOrder {
    let a = save(
        state,
        chart,
        ("ceftriaxone", "1g iv stat"),
        pick("ceftriaxone", CEFTRIAXONE),
        true,
    )
    .await;
    let (outcome, charted, ..) = done(
        sign(state, chart, &[a.to_string()], credential("bea"))
            .await
            .unwrap(),
    );
    assert_eq!(charted, 1, "{outcome}");
    let mut orders = read_stat_orders(owner, &[chart]).await.unwrap();
    assert_eq!(orders.len(), 1, "{orders:?}");
    orders.remove(0)
}

/// Press Given on `medication_id` from `chart` (its own chart set `charts`), Bea's key.
async fn give(
    state: &AppState,
    chart: Uuid,
    charts: &[Uuid],
    medication_id: Uuid,
    given_at: Option<String>,
) -> Result<crate::med_chart_write::MedChartWrite, crate::funnel::view::ErrorView> {
    let charts = charts.iter().map(Uuid::to_string).collect();
    let mid = medication_id.to_string();
    med_stat_give_impl(
        state,
        &chart.to_string(),
        charts,
        &mid,
        given_at,
        credential("bea"),
    )
    .await
}

/// Press Entered in error on `given` from `chart`, Bea's key.
async fn strike(
    state: &AppState,
    chart: Uuid,
    given: Uuid,
    why: Option<&str>,
) -> Result<crate::med_chart_write::MedChartWrite, crate::funnel::view::ErrorView> {
    let patient = chart.to_string();
    let given = given.to_string();
    let why = why.map(str::to_owned);
    med_stat_strike_impl(
        state,
        &patient,
        vec![patient.clone()],
        &given,
        why,
        credential("bea"),
    )
    .await
}

/// Both Sign paths sign `stat: true` on a stat ORDER, and leave it off every other line.
#[tokio::test]
async fn a_charted_stat_line_signs_stat_true() {
    let Some((_guard, owner, state, chart)) = bea_list().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    // The list's Sign.
    stat_line(&state, &owner, chart).await;
    // The one-line Sign: a stat order, then a regimen.
    let patient = chart.to_string();
    for directions in ["1g iv stat", "1g iv daily"] {
        let line = ChartArg {
            parts: pick("ceftriaxone", CEFTRIAXONE),
            typed_drug: "ceftriaxone".into(),
            directions: directions.into(),
            charted_as: "started-here".into(),
            reason: None,
        };
        med_chart_impl(
            &state,
            &patient,
            vec![patient.clone()],
            line,
            credential("bea"),
        )
        .await
        .unwrap();
    }
    let stat: Vec<Option<bool>> = payloads(&owner, chart)
        .await
        .iter()
        .map(|p| p.get("stat").map(|s| s.as_bool().unwrap()))
        .collect();
    // Absent, never `false`, on the regimen (the wire is forever: today's bytes).
    assert_eq!(stat, [Some(true), Some(true), None]);
    assert_eq!(read_stat_orders(&owner, &[chart]).await.unwrap().len(), 2);
}

/// Given signs, the line leaves the current list (stopped by the give), the time given is the
/// one typed, and a second press on the same line is refused before the key.
#[tokio::test]
async fn give_signs_and_the_line_leaves_the_current_list() {
    let Some((_guard, owner, state, chart)) = bea_list().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let order = stat_line(&state, &owner, chart).await;
    let at = (chrono::Utc::now() - chrono::Duration::minutes(30))
        .with_timezone(&chrono::Local)
        .format("%Y-%m-%dT%H:%M")
        .to_string();
    let written = give(
        &state,
        chart,
        &[chart],
        order.medication_id,
        Some(at.clone()),
    )
    .await
    .unwrap();
    assert_eq!(written.outcome, "Given.");
    assert!(written.pane.is_some(), "the pane read again");
    assert!(
        !current(&owner, order.medication_id).await,
        "it left the list"
    );
    let after = read_stat_orders(&owner, &[chart]).await.unwrap();
    assert!(!after[0].waiting(), "{after:?}");
    let g = &after[0].gives[0];
    assert_eq!(g.giver.as_deref(), Some("Dr Bea"));
    assert_eq!(
        g.given_at,
        crate::notes::time::stated_time_from_local(&at).unwrap()
    );
    // A stale second press: refused, the vault not asked, nothing written.
    let before = vault_attempts(&owner).await;
    let again = give(&state, chart, &[chart], order.medication_id, None)
        .await
        .unwrap_err();
    assert_eq!(again.text, STAT_NOT_OPEN);
    assert_eq!(
        vault_attempts(&owner).await,
        before,
        "the vault was not asked"
    );
    assert_eq!(count(&owner, "clinical.medication-given.asserted").await, 1);
}

/// Review Focus 5: a line stopped since the pane was drawn is no longer waiting; Given is refused
/// before the key, and nothing is written.
#[tokio::test]
async fn give_on_a_line_stopped_since_is_refused_before_the_key() {
    let Some((_guard, owner, state, chart)) = bea_list().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let order = stat_line(&state, &owner, chart).await;
    let patient = chart.to_string();
    let pane = crate::commands::med_list_impl(&state, &patient, None)
        .await
        .unwrap();
    let group = pane.list.rows[0].group_id.clone();
    crate::commands::cease_impl(
        &state,
        &patient,
        vec![patient.clone()],
        &group,
        "no longer indicated",
        credential("bea"),
    )
    .await
    .unwrap();
    let before = vault_attempts(&owner).await;
    let refused = give(&state, chart, &[chart], order.medication_id, None)
        .await
        .unwrap_err();
    assert_eq!(refused.text, STAT_NOT_OPEN);
    assert_eq!(
        vault_attempts(&owner).await,
        before,
        "the vault was not asked"
    );
    assert_eq!(count(&owner, "clinical.medication-given.asserted").await, 0);
}

/// Spec §2: a linked chart's stat line is never given from this chart: refused, nothing written.
#[tokio::test]
async fn give_on_a_linked_charts_line_is_refused() {
    let Some((_guard, mut owner, state, chart)) = bea_list().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let node_sk = state.node_sk.as_ref().unwrap();
    let node_kid = hex::encode(node_sk.verifying_key().to_bytes());
    let (other, _, _) = cairn_node::john_doe::register_john_doe(
        &mut owner,
        node_sk,
        &node_kid,
        &state.node_origin,
        "ED",
        "test-site",
        "2026-11-14",
        "found wandering",
    )
    .await
    .unwrap();
    *state.chart.lock().await = Some(crate::funnel::window::OpenChart::by_id(other));
    let theirs = stat_line(&state, &owner, other).await;
    *state.chart.lock().await = Some(crate::funnel::window::OpenChart::by_id(chart));
    let (bea_sk, bea_kid) = state
        .signer_for_act(None, crate::state::Now::read())
        .await
        .unwrap();
    let reviewer = cairn_node::chart_link::Reviewer {
        human_sk: &bea_sk,
        human_kid: &bea_kid,
    };
    cairn_node::chart_link::link_charts(&mut owner, chart, other, &reviewer, &state.node_origin)
        .await
        .unwrap();
    let refused = give(&state, chart, &[chart, other], theirs.medication_id, None)
        .await
        .unwrap_err();
    assert_eq!(refused.text, STAT_NOT_OPEN);
    assert_eq!(count(&owner, "clinical.medication-given.asserted").await, 0);
    assert!(current(&owner, theirs.medication_id).await, "still waiting");
}

/// Entered in error on the give returns the line to the current list and says it is waiting to
/// be given again; the strike reads back with its why.
#[tokio::test]
async fn strike_returns_the_line_and_answers_waiting_again() {
    let Some((_guard, owner, state, chart)) = bea_list().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let order = stat_line(&state, &owner, chart).await;
    give(&state, chart, &[chart], order.medication_id, None)
        .await
        .unwrap();
    let given = read_stat_orders(&owner, &[chart]).await.unwrap()[0].gives[0].event_id;
    let written = strike(&state, chart, given, Some("wrong patient"))
        .await
        .unwrap();
    assert_eq!(
        written.outcome,
        "Entered in error: the stat dose is waiting to be given again."
    );
    assert!(written.pane.is_some(), "the pane read again");
    assert!(
        current(&owner, order.medication_id).await,
        "back on the list"
    );
    let after = read_stat_orders(&owner, &[chart]).await.unwrap();
    assert!(after[0].waiting(), "{after:?}");
    let struck = after[0].gives[0].struck.as_ref().expect("struck");
    assert_eq!(struck.why.as_deref(), Some("wrong patient"));
    assert_eq!(struck.who.as_deref(), Some("Dr Bea"));
}

/// Review Focus 5: a give struck since the pane was drawn (or never on this chart) is refused
/// before the key, and no second strike is written.
#[tokio::test]
async fn strike_on_a_given_struck_since_is_refused() {
    let Some((_guard, owner, state, chart)) = bea_list().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let order = stat_line(&state, &owner, chart).await;
    give(&state, chart, &[chart], order.medication_id, None)
        .await
        .unwrap();
    let given = read_stat_orders(&owner, &[chart]).await.unwrap()[0].gives[0].event_id;
    strike(&state, chart, given, None).await.unwrap();
    let before = vault_attempts(&owner).await;
    for target in [given, Uuid::now_v7()] {
        let refused = strike(&state, chart, target, None).await.unwrap_err();
        assert_eq!(refused.text, GIVE_GONE);
    }
    assert_eq!(
        vault_attempts(&owner).await,
        before,
        "the vault was not asked"
    );
    let strikes = count(&owner, "clinical.medication-given-retraction.asserted").await;
    assert_eq!(strikes, 1);
}

/// A failed stat read never hides the medication list (availability over consistency, as the
/// versions read): the list opens, a given line still reads stopped, and the pane says the stat
/// doses were not read, so "no stat line shown" is never mistaken for "none given".
#[tokio::test]
async fn a_failed_stat_read_still_opens_the_list() {
    use crate::allergy_check::db_tests::{drop_minus, window_without};
    let Some((_guard, owner, mut state, chart)) = bea_list().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    stat_line(&state, &owner, chart).await;
    let patient = chart.to_string();
    let control = crate::commands::med_list_impl(&state, &patient, None)
        .await
        .unwrap();
    assert_eq!(control.stat_error, None, "control");

    window_without(&owner, &mut state, "medication_given_retraction").await;
    let pane = crate::commands::med_list_impl(&state, &patient, None).await;
    drop_minus(&owner).await;
    let pane = pane.expect("the medication list still opens");
    assert_eq!(pane.stat_error.as_deref(), Some(STAT_NOT_READ));
    assert_eq!(pane.versions_error, None, "the versions read is its own");
    assert_eq!(pane.list.rows.len(), 1, "the current line is still shown");
}

/// A failed SAVEPOINT is the snapshot failing, not the stat read: it propagates (as
/// `chart_versions`), never a pane from a dead transaction with only [`STAT_NOT_READ`] above it.
#[tokio::test]
async fn a_failed_savepoint_propagates_and_is_not_stat_not_read() {
    let Some((_guard, mut owner, _state, chart)) = bea_list().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let tx = owner.transaction().await.unwrap();
    tx.batch_execute("SELECT 1/0")
        .await
        .expect_err("aborts the tx");
    let e = crate::chart_stat::read_stat(&tx, &[chart]).await;
    let e = e.expect_err("a SAVEPOINT in an aborted transaction propagates");
    assert!(format!("{e:#}").contains("aborted"), "{e:#}");
}

/// Bea's key as the node-level acts below sign it: what reaches the chart from ANOTHER terminal
/// or a peer, which this window would refuse to offer (a second give, a real stop on a given line).
async fn bea(state: &AppState) -> (cairn_event::SigningKey, String) {
    state
        .signer_for_act(None, crate::state::Now::read())
        .await
        .unwrap()
}

/// Two standing gives (given at once from two terminals): striking one leaves the other's stop
/// standing, so the line stays stopped and the strike must NOT say "waiting to be given again" —
/// that sentence would invite a real second dose.
#[tokio::test]
async fn striking_one_of_two_gives_does_not_say_waiting_again() {
    let Some((_guard, mut owner, state, chart)) = bea_list().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let order = stat_line(&state, &owner, chart).await;
    give(&state, chart, &[chart], order.medication_id, None)
        .await
        .unwrap();
    let first = read_stat_orders(&owner, &[chart]).await.unwrap()[0].gives[0].event_id;
    // The other terminal's give, signed before it saw this one.
    let (sk, kid) = bea(&state).await;
    let author = cairn_node::medication::AuthorParams {
        human_sk: &sk,
        human_kid: &kid,
    };
    let node_sk = state.node_sk.as_ref().unwrap();
    let node_kid = hex::encode(node_sk.verifying_key().to_bytes());
    cairn_node::medication::stat_give::give_stat(
        &mut owner,
        node_sk,
        &node_kid,
        &state.node_origin,
        chart,
        order.medication_id,
        chrono::Utc::now(),
        &author,
    )
    .await
    .unwrap();
    let written = strike(&state, chart, first, None).await.unwrap();
    assert_eq!(written.outcome, "Entered in error.");
    assert!(!current(&owner, order.medication_id).await, "still stopped");
    assert!(!read_stat_orders(&owner, &[chart]).await.unwrap()[0].waiting());
}

/// Review Focus 1: a real stop stands beside the give; striking the give leaves the line stopped,
/// so the strike says only "Entered in error.".
#[tokio::test]
async fn striking_a_give_beside_a_real_stop_does_not_say_waiting_again() {
    let Some((_guard, mut owner, state, chart)) = bea_list().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let order = stat_line(&state, &owner, chart).await;
    give(&state, chart, &[chart], order.medication_id, None)
        .await
        .unwrap();
    let given = read_stat_orders(&owner, &[chart]).await.unwrap()[0].gives[0].event_id;
    // Another terminal's real stop of the same line.
    let (sk, kid) = bea(&state).await;
    let author = cairn_node::medication::AuthorParams {
        human_sk: &sk,
        human_kid: &kid,
    };
    let node_sk = state.node_sk.as_ref().unwrap();
    let node_kid = hex::encode(node_sk.verifying_key().to_bytes());
    let stop = cairn_node::medication::CeaseMedicationInput {
        stopped: None,
        stopped_precision: None,
        reason: Some("no longer indicated"),
    };
    cairn_node::medication::cease_medication(
        &mut owner,
        node_sk,
        &node_kid,
        &state.node_origin,
        chart,
        order.medication_id,
        &stop,
        Some(&author),
        None,
    )
    .await
    .unwrap();
    let written = strike(&state, chart, given, None).await.unwrap();
    assert_eq!(written.outcome, "Entered in error.");
    assert!(
        !current(&owner, order.medication_id).await,
        "the real stop stands"
    );
}
