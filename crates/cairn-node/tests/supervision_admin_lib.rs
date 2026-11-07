//! DB-gated: `cairn_node::supervision_admin`, the library over db/072's doors. The SQL matrix is
//! pinned by `supervision_doors.rs`; this pins that the LIBRARY reads the prior, signs the address
//! the door verifies, and reads the panel rows back.
mod common;
#[path = "common/vault_enroller_kit.rs"]
#[macro_use]
mod kit;

use cairn_event::SigningKey;
use cairn_node::supervision_admin::{act, console_act, staff as panel, SupervisionOp};
use kit::*;

#[test]
fn op_words_round_trip_and_unknown_words_are_refused() {
    use SupervisionOp::*;
    for (op, w) in [
        (LinkSupervisor, "link-supervisor"),
        (UnlinkSupervisor, "unlink-supervisor"),
        (RequireCosign, "require-cosign"),
        (LiftCosign, "lift-cosign"),
        (GrantAssigner, "grant-assigner"),
        (WithdrawAssigner, "withdraw-assigner"),
    ] {
        assert_eq!(op.as_sql(), w);
        assert_eq!(SupervisionOp::from_cli(w), Some(op));
    }
    assert_eq!(SupervisionOp::from_cli("Link-Supervisor"), None);
    assert_eq!(SupervisionOp::from_cli(""), None);
}

fn boss() -> SigningKey {
    SigningKey::from_bytes(&test_seed(BOSS))
}

#[tokio::test]
async fn act_links_and_lifts_through_the_door() {
    db_gated!();
    let (_g, c, _, _) = fresh().await;
    boot(&c).await;
    enrol(&c, "jsmith", "clinician", JSMITH, "boss", BOSS)
        .await
        .unwrap();
    enrol(&c, "bwong", "clinician", BWONG, "boss", BOSS)
        .await
        .unwrap();
    let su = boss();
    act(&c, SupervisionOp::RequireCosign, "bwong", None, "boss", &su)
        .await
        .unwrap();
    act(
        &c,
        SupervisionOp::LinkSupervisor,
        "bwong",
        Some("jsmith"),
        "boss",
        &su,
    )
    .await
    .unwrap();
    let rows = panel(&c).await.unwrap();
    let b = rows.iter().find(|r| r.staff_id == "bwong").unwrap();
    assert!(b.needs_cosigning);
    assert_eq!(b.supervisors, vec!["jsmith".to_string()]);
    act(&c, SupervisionOp::LiftCosign, "bwong", None, "boss", &su)
        .await
        .unwrap();
    // A refusal surfaces as an error, not a silent no-op: lifting again is a no-op the door refuses.
    let e = act(&c, SupervisionOp::LiftCosign, "bwong", None, "boss", &su)
        .await
        .unwrap_err();
    assert!(format!("{e:#}").contains("supervision act"), "{e:#}");
}

#[tokio::test]
async fn console_act_needs_no_token() {
    db_gated!();
    let (_g, c, _, _) = fresh().await;
    boot(&c).await;
    enrol(&c, "jsmith", "clinician", JSMITH, "boss", BOSS)
        .await
        .unwrap();
    console_act(&c, SupervisionOp::GrantAssigner, "jsmith", None)
        .await
        .unwrap();
    let rows = panel(&c).await.unwrap();
    assert!(
        rows.iter()
            .find(|r| r.staff_id == "jsmith")
            .unwrap()
            .assigner
    );
    let e = console_act(&c, SupervisionOp::GrantAssigner, "jsmith", None)
        .await
        .unwrap_err();
    assert!(
        format!("{e:#}").contains("console supervision act"),
        "{e:#}"
    );
}

#[tokio::test]
async fn staff_reads_the_panel_rows() {
    db_gated!();
    let (_g, c, _, _) = fresh().await;
    boot(&c).await;
    enrol(&c, "jsmith", "clinician", JSMITH, "boss", BOSS)
        .await
        .unwrap();
    let rows = panel(&c).await.unwrap();
    let j = rows.iter().find(|r| r.staff_id == "jsmith").unwrap();
    assert_eq!(j.role, "clinician");
    assert_eq!(j.display_name, display("jsmith"));
    assert!(!j.needs_cosigning && !j.assigner && j.supervisors.is_empty());
}
