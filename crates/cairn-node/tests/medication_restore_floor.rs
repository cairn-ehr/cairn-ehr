//! Batch charting slice 2b, Task 2 — a RESTORE: "use this version instead" (spec §3.3, db/080 §1).
//!
//! A restore is two events in ONE transaction: a cessation of the companion line, and a NEW
//! `clinical.medication.asserted` thread carrying `restores` = the set-aside version's id. The
//! floor makes the pairing hold even for a raw-SQL client: at COMMIT (a deferred check, because
//! the two events may arrive in either order) nothing else may still stand for the companion —
//! neither the companion nor an earlier restore's line (ruling R3, `medication_restore_replaces.rs`)
//! — or nothing of the transaction lands. Otherwise the chart would show two current lines of the
//! same drug.
//!
//! Door rules (ADR-0056): the SHAPE of `restores` (a uuid string, or JSON null = absent) refuses
//! at BOTH doors; "it names a version on this chart" and the commit-time pairing apply at the
//! AUTHORING door only, so a peer's restore whose version has not synced yet never wedges sync.
//!
//! DB-gated on `$CAIRN_TEST_PG` (a skipped run prints `skipped:`), serialized via
//! `db::test_serial_guard`.
use cairn_node::db;
use serde_json::{json, Value};
use tokio_postgres::Client;
use uuid::Uuid;

mod common;
#[path = "common/medication_kit.rs"]
mod medication_kit;
use medication_kit::*;
#[macro_use]
#[path = "common/set_aside_kit.rs"]
mod set_aside_kit;
use set_aside_kit::*;
#[path = "common/med_restore_kit.rs"]
mod med_restore_kit;
use med_restore_kit::*;

/// Call the medication-assertion floor directly on `body` at one door, in a throwaway
/// transaction (the assertion twin of set_aside_kit's `floor`).
async fn assertion_floor(c: &mut Client, body: &Value, remote: bool) -> Result<(), String> {
    let tx = c.transaction().await.unwrap();
    tx.execute(
        "SELECT set_config('cairn.remote_apply', $1, true)",
        &[&if remote { "on" } else { "" }],
    )
    .await
    .unwrap();
    let r = tx
        .execute(
            "SELECT cairn_check_medication_assertion('clinical.medication.asserted', $1::text::jsonb)",
            &[&body.to_string()],
        )
        .await;
    tx.rollback().await.unwrap();
    r.map(|_| ()).map_err(|e| db_msg(&e))
}

#[tokio::test]
async fn a_restore_with_its_companions_stop_commits_and_projects_restores() {
    rig!(c, sk, kid);
    let (p, med, version) = chart_with_version(&mut c, &sk, &kid).await;
    let (line, new_med) = restore_body(&c, &kid, p, version).await;
    let stop = stop_body(&c, &kid, p, med).await;
    // The line BEFORE the stop: the check is deferred to COMMIT, so either order holds.
    submit_together(&mut c, &sk, vec![line, stop])
        .await
        .expect("a restore that stops its companion commits");
    assert_eq!(projected_restores(&c, new_med).await, Some(version));
}

#[tokio::test]
async fn a_restore_without_the_companions_stop_fails_at_commit() {
    rig!(c, sk, kid);
    let (p, med, version) = chart_with_version(&mut c, &sk, &kid).await;
    let (line, new_med) = restore_body(&c, &kid, p, version).await;
    let line_id = line.event_id.clone();
    let err = submit_together(&mut c, &sk, vec![line])
        .await
        .expect_err("a restore that leaves its companion current is refused");
    assert!(
        err.contains("a restore must stop the line it replaces in the same transaction"),
        "{err}"
    );
    // Nothing of the transaction lands: not the event, not its projection, not a pending row.
    let logged: i64 = c
        .query_one(
            "SELECT count(*) FROM event_log WHERE event_id = $1::text::uuid",
            &[&line_id],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(logged, 0);
    assert_eq!(projected_restores(&c, new_med).await, None);
    assert_eq!(pending_rows(&c).await, 0);
    let ceased = "SELECT count(*) FROM medication_cessation WHERE medication_id = $1::text::uuid";
    assert_eq!(
        count(&c, ceased, med).await,
        0,
        "the companion is still current"
    );
}

#[tokio::test]
async fn a_restore_of_an_already_stopped_companions_version_needs_no_stop() {
    rig!(c, sk, kid);
    let (p, med, version) = chart_with_version(&mut c, &sk, &kid).await;
    cease(&mut c, &sk, &kid, p, med, None).await;
    let (line, new_med) = restore_body(&c, &kid, p, version).await;
    submit_together(&mut c, &sk, vec![line])
        .await
        .expect("a companion stopped before needs no second stop");
    assert_eq!(projected_restores(&c, new_med).await, Some(version));
}

#[tokio::test]
async fn a_restore_naming_another_charts_version_or_none_is_refused_at_the_authoring_door() {
    rig!(c, sk, kid);
    let (_a, _med, version) = chart_with_version(&mut c, &sk, &kid).await;
    let b = Uuid::now_v7();
    common::submit_registration(&c, &sk, &kid, b, 0).await;
    for named in [version, Uuid::now_v7()] {
        let (line, _) = restore_body(&c, &kid, b, named).await;
        let err = submit_together(&mut c, &sk, vec![line])
            .await
            .expect_err("a restore must name a set-aside version of its own chart");
        assert!(
            err.contains("restores") && err.contains("not a set-aside version on this chart"),
            "{err}"
        );
    }
    assert_eq!(pending_rows(&c).await, 0);
}

#[tokio::test]
async fn the_remote_door_admits_an_orphan_restore() {
    rig!(c, sk, kid);
    let p = Uuid::now_v7();
    let unknown = Uuid::now_v7();
    let wall = db::next_hlc(&c, "test-node").await.unwrap().wall + 1_000;
    let (mut line, new_med) = restore_body(&c, &kid, p, unknown).await;
    line.hlc = peer_hlc(wall);
    apply_remote(&c, &sk, &line)
        .await
        .map_err(|e| db_msg(&e))
        .expect("the sync door admits a restore whose version has not arrived");
    assert_eq!(projected_restores(&c, new_med).await, Some(unknown));
    assert_eq!(
        pending_rows(&c).await,
        0,
        "no pairing is owed by a peer's event"
    );
}

/// A bare assertion body for the direct floor calls, with `restores` set to `r`.
fn assertion_with_restores(r: Value) -> Value {
    json!({"patient_id": Uuid::now_v7().to_string(),
           "payload": {"medication_id": Uuid::now_v7().to_string(),
                       "substance": {"term": "atorvastatin"},
                       "info_source": "patient-reported",
                       "restores": r}})
}

#[tokio::test]
async fn a_non_uuid_restores_is_refused_at_both_doors() {
    rig!(c, sk, kid);
    let _ = (&sk, &kid);
    for bad in [json!("not-a-uuid"), json!(42), json!({"version_id": "x"})] {
        for remote in [false, true] {
            let err = assertion_floor(&mut c, &assertion_with_restores(bad.clone()), remote)
                .await
                .expect_err("a non-uuid restores is refused");
            assert!(err.contains("restores must be a uuid string"), "{err}");
        }
    }
    // JSON null is ABSENT at both doors (a peer that emits nulls for absent optionals).
    for remote in [false, true] {
        assertion_floor(&mut c, &assertion_with_restores(Value::Null), remote)
            .await
            .expect("a JSON null restores reads as absent");
    }
}

#[tokio::test]
async fn a_set_aside_carrying_restores_is_refused_at_both_doors() {
    rig!(c, sk, kid);
    let p = Uuid::now_v7();
    let body = version_body(&c, &kid, p, Uuid::now_v7(), Uuid::now_v7(), |_| {}).await;
    for value in [json!(Uuid::now_v7().to_string()), Value::Null] {
        let mut payload = body.payload.clone();
        payload["restores"] = value;
        for remote in [false, true] {
            let err = floor(&mut c, &floor_body(payload.clone()), remote)
                .await
                .expect_err("a set-aside version restores nothing");
            assert!(err.contains("restores must be absent"), "{err}");
        }
    }
    let _ = &sk;
}

#[tokio::test]
async fn the_pending_table_is_empty_after_commit() {
    rig!(c, sk, kid);
    let (p, med, version) = chart_with_version(&mut c, &sk, &kid).await;
    let stop = stop_body(&c, &kid, p, med).await;
    let (line, _) = restore_body(&c, &kid, p, version).await;
    submit_together(&mut c, &sk, vec![stop, line])
        .await
        .expect("committed");
    assert_eq!(
        pending_rows(&c).await,
        0,
        "the pairing is settled and forgotten"
    );
}

#[tokio::test]
async fn a_reprojection_rebuilds_restores() {
    rig!(c, sk, kid);
    let (p, med, version) = chart_with_version(&mut c, &sk, &kid).await;
    let stop = stop_body(&c, &kid, p, med).await;
    let (line, new_med) = restore_body(&c, &kid, p, version).await;
    submit_together(&mut c, &sk, vec![stop, line])
        .await
        .expect("committed");
    c.batch_execute("TRUNCATE medication_restore")
        .await
        .unwrap();
    assert_eq!(projected_restores(&c, new_med).await, None);
    // A re-fold re-runs the appliers only, in the lenient posture: it rebuilds the column and
    // owes no pairing (no pending row is written, so none can fail a later commit).
    c.execute(
        "SELECT count(*) FROM cairn_reproject('clinical.medication.asserted', false, 'test')",
        &[],
    )
    .await
    .unwrap();
    assert_eq!(projected_restores(&c, new_med).await, Some(version));
    assert_eq!(pending_rows(&c).await, 0);
}

#[tokio::test]
async fn a_shred_removes_the_restore_row() {
    rig!(c, sk, kid);
    let (p, med, version) = chart_with_version(&mut c, &sk, &kid).await;
    let stop = stop_body(&c, &kid, p, med).await;
    let (line, new_med) = restore_body(&c, &kid, p, version).await;
    let ev: Uuid = line.event_id.parse().unwrap();
    submit_together(&mut c, &sk, vec![stop, line])
        .await
        .expect("committed");
    assert_eq!(projected_restores(&c, new_med).await, Some(version));
    cairn_node::shred::shred_event(
        &mut c,
        &sk,
        &kid,
        "test-node",
        ev,
        "retention ceiling",
        None,
    )
    .await
    .expect("shred");
    assert_eq!(
        projected_restores(&c, new_med).await,
        None,
        "scrubbed by content address"
    );
}

/// ADR-0103 decision 6: the COMMIT check never reads `cairn.remote_apply`. The strict door's ''
/// is gone by COMMIT, and the setting then reads whatever the transaction set last — so a check
/// that read it would let a restore with no stop through once anything left it 'on'. Two ways
/// to leave it 'on' after the restore is submitted: set it by hand, and run a reprojection
/// (db/039 sets it for its lenient re-fold). Either way the pending row, not the setting, says
/// "this restore came through the authoring door", and the COMMIT still refuses.
#[tokio::test]
async fn the_commit_check_ignores_remote_apply_set_after_the_restore() {
    rig!(c, sk, kid);
    let (p, _med, version) = chart_with_version(&mut c, &sk, &kid).await;
    for reproject in [false, true] {
        let (line, _) = restore_body(&c, &kid, p, version).await;
        let (bytes, dek) = seal_sign(line, &sk);
        let tx = c.transaction().await.unwrap();
        tx.execute("SELECT submit_event($1, NULL, NULL, $2)", &[&bytes, &dek])
            .await
            .map_err(|e| db_msg(&e))
            .expect("the restore with no stop is admitted at submit (the check is deferred)");
        if reproject {
            tx.execute(
                "SELECT count(*) FROM cairn_reproject('clinical.medication.asserted', false, 'test')",
                &[],
            )
            .await
            .unwrap();
        } else {
            tx.execute("SELECT set_config('cairn.remote_apply', 'on', true)", &[])
                .await
                .unwrap();
        }
        // The premise: at COMMIT the setting really does read 'on'.
        let setting: String = tx
            .query_one("SELECT current_setting('cairn.remote_apply', true)", &[])
            .await
            .unwrap()
            .get(0);
        assert_eq!(setting, "on", "reproject = {reproject}");
        let err = tx
            .commit()
            .await
            .map_err(|e| db_msg(&e))
            .expect_err("a later remote_apply = 'on' does not excuse the missing stop");
        assert!(
            err.contains("a restore must stop the line it replaces in the same transaction"),
            "reproject = {reproject}: {err}"
        );
    }
    assert_eq!(pending_rows(&c).await, 0);
}

/// `cairn_medication_restore_lineage` on (`companion`, `patient`), as text, sorted.
async fn lineage(
    c: &impl tokio_postgres::GenericClient,
    companion: Uuid,
    patient: Uuid,
) -> Vec<String> {
    let mut ids: Vec<String> = c
        .query(
            "SELECT l::text FROM cairn_medication_restore_lineage($1::text::uuid, $2::text::uuid) AS l",
            &[&companion.to_string(), &patient.to_string()],
        )
        .await
        .map_err(|e| db_msg(&e))
        .expect("the lineage walk ends")
        .into_iter()
        .map(|r| r.get(0))
        .collect();
    ids.sort();
    ids
}

/// The lineage walks use UNION, so a cycle — which no authoring door can build, but which forged
/// or peer-crafted projection rows could — still ends instead of hanging every restore on the
/// chart. Forged as owner, in a transaction rolled back after: lines A and B each restore a
/// version set aside beside the other (a two-cycle through BOTH the upward and the downward
/// walk), and line X restores a version set aside beside itself. A 2-second statement timeout
/// turns a runaway walk into a failure instead of a hung suite.
#[tokio::test]
async fn a_forged_restore_cycle_still_ends() {
    rig!(c, sk, kid);
    let _ = (&sk, &kid);
    let (p, a, b, x) = (
        Uuid::now_v7(),
        Uuid::now_v7(),
        Uuid::now_v7(),
        Uuid::now_v7(),
    );
    let tx = c.transaction().await.unwrap();
    tx.batch_execute("SET LOCAL statement_timeout = '2s'")
        .await
        .unwrap();
    // (the version's companion, the line that restores it)
    for (beside, restored_by) in [(a, b), (b, a), (x, x)] {
        let version = Uuid::now_v7().to_string();
        tx.execute(
            "INSERT INTO medication_set_aside (content_address, event_id, patient_id, version_id, \
                 alternative_to, term, info_source, t_recorded) \
             VALUES (uuid_send(gen_random_uuid()), gen_random_uuid(), $1::text::uuid, \
                 $2::text::uuid, $3::text::uuid, 'atorvastatin', 'patient-reported', now())",
            &[&p.to_string(), &version, &beside.to_string()],
        )
        .await
        .unwrap();
        tx.execute(
            "INSERT INTO medication_restore (content_address, event_id, patient_id, medication_id, \
                 version_id) \
             VALUES (uuid_send(gen_random_uuid()), gen_random_uuid(), $1::text::uuid, \
                 $2::text::uuid, $3::text::uuid)",
            &[&p.to_string(), &restored_by.to_string(), &version],
        )
        .await
        .unwrap();
    }
    let mut both = vec![a.to_string(), b.to_string()];
    both.sort();
    assert_eq!(lineage(&tx, a, p).await, both);
    assert_eq!(lineage(&tx, b, p).await, both);
    assert_eq!(lineage(&tx, x, p).await, vec![x.to_string()]);
    tx.rollback().await.unwrap();
}

/// A peer may send two versions sharing one version_id beside two different lines (db/079's
/// "version_id already exists" is an authoring-door rule). A restore of that version then owes
/// a stop of EACH companion: the pending table keys on (event, companion), one row per
/// companion. Stopping only one leaves the other current beside the restore, so COMMIT refuses;
/// stopping both commits.
#[tokio::test]
async fn a_version_id_shared_by_two_companions_owes_a_stop_of_each() {
    rig!(c, sk, kid);
    let (p, med, version) = chart_with_version(&mut c, &sk, &kid).await;
    let other = add_med(&mut c, &sk, &kid, p).await;
    let wall = db::next_hlc(&c, "test-node").await.unwrap().wall + 1_000;
    let mut twin = version_body(&c, &kid, p, version, other, |_| {}).await;
    twin.hlc = peer_hlc(wall);
    apply_remote(&c, &sk, &twin)
        .await
        .map_err(|e| db_msg(&e))
        .expect("the sync door admits a peer's version reusing the version_id");
    let rows = "SELECT count(*) FROM medication_set_aside WHERE version_id = $1::text::uuid";
    assert_eq!(
        count(&c, rows, version).await,
        2,
        "two companions, one version_id"
    );

    let (line, _) = restore_body(&c, &kid, p, version).await;
    let stop = stop_body(&c, &kid, p, med).await;
    let err = submit_together(&mut c, &sk, vec![line, stop])
        .await
        .expect_err("the second companion is still current");
    assert!(
        err.contains("a restore must stop the line it replaces in the same transaction")
            && err.contains(&format!("still stands for line {other}")),
        "{err}"
    );
    assert_eq!(pending_rows(&c).await, 0);

    let (line, new_med) = restore_body(&c, &kid, p, version).await;
    let events = vec![
        line,
        stop_body(&c, &kid, p, med).await,
        stop_body(&c, &kid, p, other).await,
    ];
    submit_together(&mut c, &sk, events)
        .await
        .expect("a restore that stops both companions commits");
    assert_eq!(projected_restores(&c, new_med).await, Some(version));
}
