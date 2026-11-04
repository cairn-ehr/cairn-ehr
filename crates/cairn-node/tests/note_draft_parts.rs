//! Sign-in slice 3b-1, Task 3 — the node's part-aware draft calls (`cairn_node::note::draft`)
//! and the staff directory (`cairn_node::note::whos_here`).
//!
//! db/066 made a draft a sequence of writers' parts and added a door that MOVES a draft to
//! another terminal. These tests drive the Rust calls the window makes, under the window's OWN
//! login (`cairn_window`), so they pin both the shape the window reads back and that every door
//! is reachable by that role: two writers read back as two parts, a moved draft answers `Moved`
//! at the terminal it left, and the who's-here directory never offers a superuser.
//!
//! DB-gated on `$CAIRN_TEST_PG`, serialized via `db::test_serial_guard` taken BEFORE connecting.
mod common;

use cairn_node::db;
use cairn_node::note::draft::{open_drafts, save_part, take_draft, SaveOutcome, TakeOutcome};
use cairn_node::note::whos_here::directory;
use common::{expect_as_window, note_setup, owner_and_window};
use tokio_postgres::Client;
use uuid::Uuid;

/// A second clinical member beside `note_setup`'s `notewriter`, and a superuser, written
/// directly as `note_setup` writes its member (the `sealed` blob is a placeholder nobody
/// releases; the key ids are derived, never literal).
async fn more_members(owner: &Client) {
    for (staff, name, role, k) in [
        ("nurse-b", "Nurse B", "nurse", 0xb2u8),
        ("site-super", "Site Super", "superuser", 0xc3u8),
    ] {
        let kid = hex::encode([k; 32]);
        owner
            .execute(
                "INSERT INTO vault_member (staff_id, display_name, role, actor_id, signing_key_id, \
                                           sealed, enrolled_by, contact_email) \
                 VALUES ($1, $2, $3, '\\x00'::bytea, $4, '\\x00'::bytea, 'console', 'x@example.org')",
                &[&staff, &name, &role, &kid],
            )
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn two_writers_read_back_as_two_parts() {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let (owner, window) = owner_and_window(&base).await;
    let (_, _, _, _, chart) = note_setup(&owner).await;
    more_members(&owner).await;
    let draft = Uuid::now_v7();

    for (writer, text) in [
        (Some("notewriter"), "obs stable"),
        (Some(" Nurse-B "), "fluids running"),
    ] {
        let r = save_part(&window, draft, chart, "pc-1", writer, text, None).await;
        assert_eq!(expect_as_window("save_part", r), SaveOutcome::Saved);
    }
    let r = save_part(&window, draft, chart, "pc-1", Some("nobody"), "x", None).await;
    assert_eq!(expect_as_window("save_part", r), SaveOutcome::NoSuchWriter);

    let drafts = expect_as_window("open_drafts", open_drafts(&window, chart).await);
    assert_eq!(drafts.len(), 1, "one draft, holding both parts");
    let d = &drafts[0];
    assert_eq!((d.draft_id, d.terminal_id.as_str()), (draft, "pc-1"));
    let parts: Vec<_> = d
        .parts
        .iter()
        .map(|p| {
            (
                p.seq,
                p.writer.as_deref(),
                p.writer_name.as_deref(),
                p.narrative.as_str(),
            )
        })
        .collect();
    assert_eq!(
        parts,
        [
            (
                1,
                Some("notewriter"),
                Some(common::NOTE_AUTHOR),
                "obs stable"
            ),
            (2, Some("nurse-b"), Some("Nurse B"), "fluids running"),
        ]
    );
    assert_eq!(d.recording_started, d.parts[0].recording_started);
    assert_eq!(d.last_keystroke, d.parts[1].last_keystroke);
}

#[tokio::test]
async fn a_take_then_the_old_terminal_saves_moved() {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let (owner, window) = owner_and_window(&base).await;
    let (_, _, _, _, chart) = note_setup(&owner).await;
    let draft = Uuid::now_v7();
    let r = save_part(&window, draft, chart, "pc-1", None, "begun at pc-1", None).await;
    assert_eq!(expect_as_window("save_part", r), SaveOutcome::Saved);

    let r = take_draft(&window, draft, chart, "pc-2").await;
    assert_eq!(expect_as_window("take_draft", r), TakeOutcome::Taken);
    let r = save_part(&window, draft, chart, "pc-1", None, "late", None).await;
    assert_eq!(expect_as_window("save_part (old)", r), SaveOutcome::Moved);
    let r = save_part(&window, draft, chart, "pc-2", None, "continued", None).await;
    assert_eq!(expect_as_window("save_part (new)", r), SaveOutcome::Saved);

    let r = take_draft(&window, draft, Uuid::now_v7(), "pc-3").await;
    assert_eq!(expect_as_window("take_draft", r), TakeOutcome::Elsewhere);
    let r = take_draft(&window, Uuid::now_v7(), chart, "pc-3").await;
    assert_eq!(expect_as_window("take_draft", r), TakeOutcome::Gone);
    let drafts = expect_as_window("open_drafts", open_drafts(&window, chart).await);
    assert_eq!(drafts[0].terminal_id, "pc-2");
    assert_eq!(drafts[0].parts[0].narrative, "continued");
}

#[tokio::test]
async fn the_directory_omits_superusers() {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let (owner, window) = owner_and_window(&base).await;
    note_setup(&owner).await;
    more_members(&owner).await;

    let staff = expect_as_window("whos_here::directory", directory(&window).await);
    let shown: Vec<_> = staff
        .iter()
        .map(|s| {
            (
                s.staff_id.as_str(),
                s.display_name.as_str(),
                s.role.as_str(),
            )
        })
        .collect();
    assert_eq!(
        shown,
        [
            ("notewriter", common::NOTE_AUTHOR, "clinician"),
            ("nurse-b", "Nurse B", "nurse"),
        ],
        "clinical members by name; never the superuser"
    );
}
