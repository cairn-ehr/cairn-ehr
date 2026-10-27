//! #855 — two releases of ONE hold racing each other at the local door.
//!
//! ADR-0082 says a release of a hold that is no longer standing is refused at the local door,
//! as a STALE picture. Two interleavings used to slip past that sentence:
//!
//!   1. **The floor.** db/059 section 8 read `medication_withhold_release` under READ COMMITTED
//!      with no lock, so two local releases of the same hold in flight at once (two windows, or
//!      the CLI and a window) each missed the other's uncommitted row and BOTH were admitted — a
//!      release that released nothing. Section 8 now takes a per-hold advisory lock before it
//!      checks, so the second waits for the first, then sees it and is refused.
//!   2. **The orchestrator.** `release_medication_holds` asks `cairn_medication_hold_ended`
//!      before signing (so its refusal carries `RefusalScope::Stale`), then submits in a later
//!      statement. A colleague's release committing in between was refused by the floor as a
//!      bare `P0001`, so the window showed a refusal instead of re-reading (#840's "stale wins"
//!      did not fire). The orchestrator now re-asks the same predicate after a floor refusal.
//!
//! Each test opens the colleague's release on one connection and leaves it UNCOMMITTED, runs the
//! second release on another connection in a spawned task, waits until that task has either
//! finished (no lock: the unfixed floor) or is provably parked on a lock
//! (`pg_stat_activity`, never a guessed sleep), insists that the lock is section 8's advisory
//! one, and only then commits the first. Both outcomes are deterministic, so each assertion is
//! red before its fix and green after it.
//!
//! DB-gated on `$CAIRN_TEST_PG`, serialized via `db::test_serial_guard`. Key material is derived
//! at runtime.

use cairn_event::medication::HoldUntil;
use cairn_node::db;
use cairn_node::db_diagnosis::{refusal_scope, RefusalScope};
use cairn_node::medication::{
    build_release_body, release_medication_holds, withhold_medication, WithholdMedicationInput,
};
use tokio_postgres::Client;
use uuid::Uuid;

mod common;
#[path = "common/late_custody_kit.rs"]
mod late_custody_kit;
#[path = "common/medication_kit.rs"]
mod medication_kit;
use late_custody_kit::{second_connection, until_finished_or_parked};
use medication_kit::*;

async fn release_rows(c: &Client, med: Uuid) -> i64 {
    c.query_one(
        "SELECT count(*) FROM medication_withhold_release WHERE medication_id = $1::text::uuid",
        &[&med.to_string()],
    )
    .await
    .unwrap()
    .get(0)
}

/// An instant `secs` from the database's clock, floored to the second, as RFC 3339 UTC text.
async fn instant_from_now(c: &Client, secs: i64) -> String {
    c.query_one(
        "SELECT to_char((clock_timestamp() + make_interval(secs => $1::bigint)) \
                AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS\"Z\"')",
        &[&secs],
    )
    .await
    .unwrap()
    .get(0)
}

/// A hold "until reviewed" that started a minute ago, so every release below lands while it is
/// in force (not before its start, which would make the release a cancel).
async fn hold_started_a_minute_ago(
    c: &mut Client,
    sk: &cairn_event::SigningKey,
    kid: &str,
    patient: Uuid,
    med: Uuid,
) -> Uuid {
    let from = instant_from_now(c, -60).await;
    let input = WithholdMedicationInput {
        reason: "BP 85/50",
        from: Some(&from),
        until: HoldUntil::Reviewed,
    };
    withhold_medication(c, sk, kid, "test-node", patient, med, &input, None)
        .await
        .expect("the withhold is admitted")
}

/// Open the colleague's release of `wid` on `c`, stamped one second ago, and leave its
/// transaction OPEN: the caller commits it once the racing release is finished or parked.
async fn open_colleagues_release(
    c: &Client,
    sk: &cairn_event::SigningKey,
    kid: &str,
    patient: Uuid,
    med: Uuid,
    wid: Uuid,
) {
    let hlc = db::next_hlc(c, "test-node").await.unwrap();
    let mut body = build_release_body(Uuid::now_v7(), med, patient, &[wid], None, kid, hlc);
    body.t_effective = Some(instant_from_now(c, -1).await);
    c.batch_execute("BEGIN").await.unwrap();
    seal_and_submit(c, sk, body)
        .await
        .expect("the colleague's release is admitted");
}

/// Wait until `second` is parked, then insist it is parked on an ADVISORY lock — section 8's
/// per-hold lock — not merely on some lock. A row lock taken earlier would make the second
/// release wait and then be refused with or without that lock, and the test would prove nothing.
async fn parked_on_the_hold_lock<T>(
    observer: &Client,
    second_pid: i32,
    second: &tokio::task::JoinHandle<T>,
) {
    until_finished_or_parked(observer, second_pid, second).await;
    assert!(
        !second.is_finished(),
        "THE SECOND RELEASE MUST WAIT FOR THE FIRST (#855): it ran while the first was uncommitted"
    );
    let wait: Option<String> = observer
        .query_one(
            "SELECT wait_event FROM pg_stat_activity WHERE pid = $1",
            &[&second_pid],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        wait.as_deref(),
        Some("advisory"),
        "parked on the wrong lock"
    );
}

#[tokio::test]
async fn two_local_releases_of_one_hold_in_flight_admit_only_one() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let (patient, med) = chart_with_one_med(&mut c, &sk, &kid).await;
    let wid = hold_started_a_minute_ago(&mut c, &sk, &kid, patient, med).await;

    open_colleagues_release(&c, &sk, &kid, patient, med, wid).await;

    // The second release, through the same strict door from another client, stamped now.
    let (c2, pid2) = second_connection(&base).await;
    let hlc = db::next_hlc(&c2, "test-node").await.unwrap();
    let mut body = build_release_body(Uuid::now_v7(), med, patient, &[wid], None, &kid, hlc);
    body.t_effective = Some(instant_from_now(&c2, 0).await);
    let sk2 = sk.clone();
    let second = tokio::spawn(async move { seal_and_submit(&c2, &sk2, body).await });
    let (observer, _) = second_connection(&base).await;
    parked_on_the_hold_lock(&observer, pid2, &second).await;

    c.batch_execute("COMMIT").await.unwrap();
    let err = second.await.unwrap().expect_err(
        "THE SECOND RELEASE MUST WAIT FOR THE FIRST AND BE REFUSED (#855): it ran \
                     while the first was uncommitted and was admitted beside it",
    );
    let msg = db_msg(&err);
    assert!(
        msg.starts_with("medication release:") && msg.contains("already released"),
        "{msg}"
    );
    assert_eq!(
        release_rows(&c, med).await,
        1,
        "one release, not a second that released nothing"
    );
}

#[tokio::test]
async fn a_release_overtaken_after_its_precheck_is_refused_as_stale() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let (patient, med) = chart_with_one_med(&mut c, &sk, &kid).await;
    let wid = hold_started_a_minute_ago(&mut c, &sk, &kid, patient, med).await;

    // The colleague's release is in flight: the orchestrator's pre-check cannot see it, so it
    // signs and submits, and the floor parks on the hold's lock until the colleague commits.
    open_colleagues_release(&c, &sk, &kid, patient, med, wid).await;

    let (mut c2, pid2) = second_connection(&base).await;
    let (sk2, kid2) = (sk.clone(), kid.clone());
    let second = tokio::spawn(async move {
        release_medication_holds(
            &mut c2,
            &sk2,
            &kid2,
            "test-node",
            patient,
            med,
            &[wid],
            Some("BP recovered"),
            None,
        )
        .await
    });
    let (observer, _) = second_connection(&base).await;
    parked_on_the_hold_lock(&observer, pid2, &second).await;

    c.batch_execute("COMMIT").await.unwrap();
    let err = second
        .await
        .unwrap()
        .expect_err("a release overtaken by a colleague's is refused");
    assert_eq!(
        refusal_scope(&err),
        Some(RefusalScope::Stale),
        "THE FLOOR'S REFUSAL OF AN OVERTAKEN RELEASE MUST REACH THE CALLER AS STALE (#855), so \
         the window re-reads instead of showing a refusal: {err:#}"
    );
    let text = format!("{err:#}");
    assert!(
        text.contains(&wid.to_string()) && text.contains("already released"),
        "{text}"
    );
    assert!(text.contains("nothing was released"), "{text}");
    assert!(
        text.contains("the node said: medication release:"),
        "the floor's own refusal rides along, so a different fault is never hidden: {text}"
    );
    assert_eq!(release_rows(&c, med).await, 1);
}
