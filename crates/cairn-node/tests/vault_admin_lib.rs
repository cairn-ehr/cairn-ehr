//! DB-gated: `cairn_node::vault_admin`, the library over db/063's doors (re-key, change of
//! passphrase, the console's superuser re-key). The SQL is pinned by `vault_rekey.rs` /
//! `vault_enrollers.rs`; this pins that the LIBRARY signs what the doors verify.
use cairn_node::db;
use cairn_node::vault::{
    bootstrap_superuser, enrol_member, release_for_act, NewMember, Purpose, Release,
};
use cairn_node::vault_admin::{
    change_passphrase, enrollers, judge_key, rekey_member, rekey_superuser_from_console,
    replaced_notice, set_enroller, unconfirmed, Rekey, Reseal,
};
use tokio_postgres::Client;

mod common;

fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

/// Serialised connection with an EMPTY vault; registry rows are kept (they are append-only).
async fn fresh() -> (Client, Client) {
    let base = cs().expect("guarded by caller");
    let guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    c.batch_execute(
        "TRUNCATE vault_attempt, vault_member, vault_terminal, actor_event, event_log CASCADE",
    )
    .await
    .unwrap();
    (guard, c)
}

async fn window(owner: &Client) -> Client {
    db::connect(&common::window_login(owner, &cs().expect("guarded by caller")).await)
        .await
        .unwrap()
}

fn member<'a>(staff_id: &'a str, role: &'a str, passphrase: &'a str) -> NewMember<'a> {
    NewMember {
        staff_id,
        display_name: "Test Person",
        role,
        phone: None,
        email: Some("t@example.org"),
        passphrase,
        hint: None,
    }
}

fn rekey<'a>(staff_id: &'a str, role: &'a str, passphrase: &'a str) -> Rekey<'a> {
    Rekey {
        staff_id,
        role,
        display_name: None,
        phone: None,
        email: None,
        passphrase,
        hint: None,
    }
}

async fn released(
    w: &Client,
    staff: &str,
    pass: &str,
    p: Purpose,
) -> (cairn_event::SigningKey, String) {
    match release_for_act(w, staff, pass, p).await.unwrap() {
        Release::Released { key, kid } => (key, kid),
        other => panic!("expected a release, got {other:?}"),
    }
}

/// Bootstrap `boss` and enrol `jsmith` (a clinician) under `old`; returns boss's signer.
async fn world(c: &Client, old: &str) -> cairn_event::SigningKey {
    bootstrap_superuser(c, &member("boss", "superuser", "boss pass one"))
        .await
        .unwrap();
    let (su, _) = released(&window(c).await, "boss", "boss pass one", Purpose::Enrol).await;
    enrol_member(c, &member("jsmith", "clinician", old), "boss", &su)
        .await
        .unwrap();
    su
}

#[tokio::test]
async fn a_rekeyed_member_signs_with_the_new_passphrase() {
    let Some(_) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let (_g, c) = fresh().await;
    let su = world(&c, "old pass phrase").await;
    let w = window(&c).await;
    let kid = rekey_member(
        &w,
        &rekey("jsmith", "clinician", "new pass phrase"),
        "boss",
        &su,
    )
    .await
    .unwrap();
    let (_, got) = released(&w, "jsmith", "new pass phrase", Purpose::Clinical).await;
    assert_eq!(got, kid);
    // The old passphrase now meets the replaced key and is told so.
    assert!(matches!(
        release_for_act(&w, "jsmith", "old pass phrase", Purpose::Clinical)
            .await
            .unwrap(),
        Release::Replaced { .. }
    ));
    // The superuser's re-key confirmed the key at once, so the worklist is empty; the notice
    // still names the approver for the 14-day window.
    assert!(unconfirmed(&w).await.unwrap().is_empty());
    let n = replaced_notice(&w, "jsmith")
        .await
        .unwrap()
        .expect("a notice");
    assert_eq!(n.approver, "Test Person");
}

#[tokio::test]
async fn change_passphrase_keeps_the_key() {
    let Some(_) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let (_g, c) = fresh().await;
    world(&c, "old pass phrase").await;
    let w = window(&c).await;
    let (_, before) = released(&w, "jsmith", "old pass phrase", Purpose::Clinical).await;
    let r = change_passphrase(
        &w,
        "jsmith",
        "old pass phrase",
        "new pass phrase",
        Some("h"),
    )
    .await
    .unwrap();
    assert!(matches!(r, Reseal::Done), "{r:?}");
    let (_, after) = released(&w, "jsmith", "new pass phrase", Purpose::Clinical).await;
    assert_eq!(before, after);
    assert!(matches!(
        release_for_act(&w, "jsmith", "old pass phrase", Purpose::Clinical)
            .await
            .unwrap(),
        Release::Wrong { .. }
    ));
}

#[tokio::test]
async fn a_wrong_current_passphrase_changes_nothing() {
    let Some(_) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let (_g, c) = fresh().await;
    world(&c, "old pass phrase").await;
    let before: Vec<u8> = c
        .query_one(
            "SELECT sealed FROM vault_member WHERE staff_id = 'jsmith'",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    let w = window(&c).await;
    let r = change_passphrase(&w, "jsmith", "not it", "new pass phrase", None)
        .await
        .unwrap();
    assert!(matches!(r, Reseal::Refused(Release::Wrong { .. })), "{r:?}");
    let after: Vec<u8> = c
        .query_one(
            "SELECT sealed FROM vault_member WHERE staff_id = 'jsmith'",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(before, after);
}

#[tokio::test]
async fn the_console_rekeys_a_restored_superuser() {
    let Some(_) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let (_g, c) = fresh().await;
    bootstrap_superuser(&c, &member("boss", "superuser", "boss pass one"))
        .await
        .unwrap();
    // A restore: the registry survives, the vault does not.
    c.batch_execute("TRUNCATE vault_attempt, vault_member CASCADE")
        .await
        .unwrap();
    let mut r = rekey("boss", "superuser", "boss pass two");
    r.display_name = Some("The Boss");
    r.email = Some("b@example.org");
    let kid = rekey_superuser_from_console(&c, &r).await.unwrap();
    let (_, got) = released(&window(&c).await, "boss", "boss pass two", Purpose::Enrol).await;
    assert_eq!(got, kid);
}

#[tokio::test]
async fn grants_and_judgements_round_trip() {
    let Some(_) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let (_g, c) = fresh().await;
    let su = world(&c, "old pass phrase").await;
    let w = window(&c).await;
    set_enroller(&w, "jsmith", true, "boss", &su).await.unwrap();
    let rows = enrollers(&w).await.unwrap();
    assert!(rows.iter().any(|e| e.staff_id == "jsmith" && e.enroller));
    // An enroller's re-key of someone is unconfirmed until the superuser confirms it.
    enrol_member(&c, &member("nurse", "clinician", "nurse pass"), "boss", &su)
        .await
        .unwrap();
    let (jkey, _) = released(&w, "jsmith", "old pass phrase", Purpose::Enrol).await;
    let kid = rekey_member(
        &w,
        &rekey("nurse", "clinician", "nurse pass 2"),
        "jsmith",
        &jkey,
    )
    .await
    .unwrap();
    let todo = unconfirmed(&w).await.unwrap();
    assert!(todo
        .iter()
        .any(|u| u.kid == kid && u.approver_staff_id == "jsmith"));
    judge_key(&w, &kid, true, "boss", &su).await.unwrap();
    assert!(unconfirmed(&w).await.unwrap().iter().all(|u| u.kid != kid));
}
