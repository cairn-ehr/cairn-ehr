//! Batch charting slice 2b, Task 2 (split from `medication_restore_floor.rs` for its size cap,
//! same rig and fixtures, `common/med_restore_kit.rs`): a restore replaces WHATEVER CURRENTLY
//! STANDS for the companion (ruling R3) — the companion itself, or an earlier restore's line —
//! followed through a chain of restores (ruling R4) — and `restores` records how a thread began:
//! only a new thread carries it, and its own projection keeps it whatever the arrival order
//! (ruling R5). Plus the idempotent re-submit, two restores in one transaction, and the two
//! paths the floor's comments claim: `SET CONSTRAINTS ALL IMMEDIATE`, and a version set aside
//! in the restore's own transaction.
//!
//! DB-gated on `$CAIRN_TEST_PG` (a skipped run prints `skipped:`), serialized via
//! `db::test_serial_guard`.
use cairn_event::medication::Charting;
use cairn_node::db;
use cairn_node::medication::build_charted_body;
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

/// Ruling R3: restoring another version of a companion already replaced by a restore must stop
/// that earlier restore's line, which is what now stands for the companion.
#[tokio::test]
async fn a_second_restore_stops_the_first_restores_line() {
    rig!(c, sk, kid);
    let (p, x, v1) = chart_with_version(&mut c, &sk, &kid).await;
    let v2 = version_beside(&c, &sk, &kid, p, x).await;
    let (l1, l1_med) = restore_body(&c, &kid, p, v1).await;
    let stop_x = stop_body(&c, &kid, p, x).await;
    submit_together(&mut c, &sk, vec![stop_x, l1])
        .await
        .expect("the first restore");
    // X is already stopped, but L1 stands for it: a restore of V2 that leaves L1 is refused.
    let (raw, _) = restore_body(&c, &kid, p, v2).await;
    let err = submit_together(&mut c, &sk, vec![raw])
        .await
        .expect_err("two current lines of one drug");
    assert!(err.contains("still stands for"), "{err}");
    let (l2, l2_med) = restore_body(&c, &kid, p, v2).await;
    let stop_l1 = stop_body(&c, &kid, p, l1_med).await;
    submit_together(&mut c, &sk, vec![l2, stop_l1])
        .await
        .expect("a restore that stops the line standing for the companion");
    assert_eq!(projected_restores(&c, l2_med).await, Some(v2));
}

/// Ruling R3, the same version restored twice: the second must stop the first's line.
#[tokio::test]
async fn a_second_restore_of_the_same_version_stops_the_first() {
    rig!(c, sk, kid);
    let (p, x, v1) = chart_with_version(&mut c, &sk, &kid).await;
    let (l1, l1_med) = restore_body(&c, &kid, p, v1).await;
    let stop_x = stop_body(&c, &kid, p, x).await;
    submit_together(&mut c, &sk, vec![stop_x, l1])
        .await
        .expect("the first restore");
    let (raw, _) = restore_body(&c, &kid, p, v1).await;
    let err = submit_together(&mut c, &sk, vec![raw])
        .await
        .expect_err("two current lines of one drug");
    assert!(err.contains("still stands for"), "{err}");
    let (again, _) = restore_body(&c, &kid, p, v1).await;
    let stop_l1 = stop_body(&c, &kid, p, l1_med).await;
    submit_together(&mut c, &sk, vec![stop_l1, again])
        .await
        .expect("restored again, the first restore's line stopped");
}

#[tokio::test]
async fn restores_on_an_existing_thread_is_refused_at_the_authoring_door() {
    rig!(c, sk, kid);
    let (p, x, version) = chart_with_version(&mut c, &sk, &kid).await;
    let other = add_med(&mut c, &sk, &kid, p).await;
    let line = restore_body_on(&c, &kid, p, version, other).await;
    let stop_x = stop_body(&c, &kid, p, x).await;
    let err = submit_together(&mut c, &sk, vec![stop_x, line])
        .await
        .expect_err("restores records how a thread began");
    assert!(
        err.contains("restores is carried only by a new line"),
        "{err}"
    );
}

/// Ruling R5: `restores` has its own projection, insert-or-nothing per event, so it does not
/// depend on arrival order. A NEWER re-assertion of the thread arriving first (over the sync
/// door, explicit HLCs) must not hide the OLDER restore that arrives after it.
#[tokio::test]
async fn a_restore_projects_whatever_order_it_arrives_in() {
    rig!(c, sk, kid);
    let p = Uuid::now_v7();
    let version = Uuid::now_v7();
    let wall = db::next_hlc(&c, "test-node").await.unwrap().wall + 1_000;
    let (mut restore, med) = restore_body(&c, &kid, p, version).await;
    restore.hlc = peer_hlc(wall);
    let route = Charting {
        route: Some("po"),
        ..Charting::default()
    };
    let newer = build_charted_body(
        Uuid::now_v7(),
        med,
        p,
        &sample_input(),
        &route,
        &kid,
        peer_hlc(wall + 1_000),
        None,
    );
    for body in [&newer, &restore] {
        apply_remote(&c, &sk, body)
            .await
            .map_err(|e| db_msg(&e))
            .expect("admitted");
    }
    assert_eq!(projected_restores(&c, med).await, Some(version));
}

/// Ruling R4: the line that stands for a companion is found through a CHAIN of restores, not
/// one hop. X has V1 and V2; restoring V1 gives L1; V3 is set aside beside L1, and restoring it
/// gives L3. Restoring V2 must then stop L3, the line that now stands for X.
#[tokio::test]
async fn a_chain_of_restores_is_followed_to_the_line_that_stands() {
    rig!(c, sk, kid);
    let (p, x, v1) = chart_with_version(&mut c, &sk, &kid).await;
    let v2 = version_beside(&c, &sk, &kid, p, x).await;
    let (l1, l1_med) = restore_body(&c, &kid, p, v1).await;
    let stop_x = stop_body(&c, &kid, p, x).await;
    submit_together(&mut c, &sk, vec![stop_x, l1])
        .await
        .expect("restore V1");
    let v3 = version_beside(&c, &sk, &kid, p, l1_med).await;
    let (l3, l3_med) = restore_body(&c, &kid, p, v3).await;
    let stop_l1 = stop_body(&c, &kid, p, l1_med).await;
    submit_together(&mut c, &sk, vec![stop_l1, l3])
        .await
        .expect("restore V3");
    let (raw, _) = restore_body(&c, &kid, p, v2).await;
    let err = submit_together(&mut c, &sk, vec![raw])
        .await
        .expect_err("L3 still stands for X: two current lines of one drug");
    assert!(err.contains("still stands for"), "{err}");
    let (l2, l2_med) = restore_body(&c, &kid, p, v2).await;
    let stop_l3 = stop_body(&c, &kid, p, l3_med).await;
    submit_together(&mut c, &sk, vec![l2, stop_l3])
        .await
        .expect("restore V2, stopping the line at the end of the chain");
    assert_eq!(projected_restores(&c, l2_med).await, Some(v2));
}

/// M1: db/005 promises an idempotent re-submit of the SAME event is a silent no-op. The
/// restore's reference rules must not turn it into a refusal (its thread now exists).
#[tokio::test]
async fn an_idempotent_resubmit_of_a_restore_is_a_no_op() {
    rig!(c, sk, kid);
    let (p, x, version) = chart_with_version(&mut c, &sk, &kid).await;
    let (line, _) = restore_body(&c, &kid, p, version).await;
    let line = seal_sign(line, &sk);
    let stop_x = seal_sign(stop_body(&c, &kid, p, x).await, &sk);
    submit_sealed(&mut c, &[stop_x, line.clone()], false)
        .await
        .expect("restored");
    submit_sealed(&mut c, &[line], false)
        .await
        .expect("the same bytes again: a silent no-op");
    assert_eq!(pending_rows(&c).await, 0);
}

/// M2: two restores of one companion in ONE transaction — restore L1, stop X, restore L2, stop
/// L1 — leave one current line, so they commit. A stopped restore stands for nothing.
#[tokio::test]
async fn two_restores_of_one_companion_in_one_transaction_commit_when_one_stands() {
    rig!(c, sk, kid);
    let (p, x, v1) = chart_with_version(&mut c, &sk, &kid).await;
    let v2 = version_beside(&c, &sk, &kid, p, x).await;
    let (l1, l1_med) = restore_body(&c, &kid, p, v1).await;
    let stop_x = stop_body(&c, &kid, p, x).await;
    let (l2, l2_med) = restore_body(&c, &kid, p, v2).await;
    let stop_l1 = stop_body(&c, &kid, p, l1_med).await;
    submit_together(&mut c, &sk, vec![l1, stop_x, l2, stop_l1])
        .await
        .expect("one current line at commit");
    assert_eq!(projected_restores(&c, l2_med).await, Some(v2));
}

#[tokio::test]
async fn set_constraints_immediate_needs_the_stop_first() {
    rig!(c, sk, kid);
    let (p, x, version) = chart_with_version(&mut c, &sk, &kid).await;
    let (line, _) = restore_body(&c, &kid, p, version).await;
    let stop_x = stop_body(&c, &kid, p, x).await;
    let err = submit_with(&mut c, &sk, vec![line, stop_x], true)
        .await
        .expect_err("immediate: the check runs as the line lands, before its stop");
    assert!(
        err.contains("a restore must stop the line it replaces"),
        "{err}"
    );
    let (line, _) = restore_body(&c, &kid, p, version).await;
    let stop_x = stop_body(&c, &kid, p, x).await;
    submit_with(&mut c, &sk, vec![stop_x, line], true)
        .await
        .expect("immediate, stop first: committed");
    assert_eq!(pending_rows(&c).await, 0);
}

/// The db/080 claim: a version submitted earlier in the same transaction counts.
#[tokio::test]
async fn a_version_in_the_same_transaction_as_its_restore_satisfies_the_floor() {
    rig!(c, sk, kid);
    let (p, x) = chart_with_one_med(&mut c, &sk, &kid).await;
    let version = Uuid::now_v7();
    let set_aside = version_body(&c, &kid, p, version, x, |_| {}).await;
    let stop_x = stop_body(&c, &kid, p, x).await;
    let (line, med) = restore_body(&c, &kid, p, version).await;
    submit_together(&mut c, &sk, vec![set_aside, stop_x, line])
        .await
        .expect("set aside, stopped and restored in one transaction");
    assert_eq!(projected_restores(&c, med).await, Some(version));
}
