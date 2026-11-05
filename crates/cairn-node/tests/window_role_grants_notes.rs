//! Sign-in slices 3a/3b-1 — the note pane's calls, run under the window's OWN login (`cairn_window`).
//!
//! Split out of `window_role_grants.rs` (which walks the rest of the window's day) to keep each
//! file under 500 lines; read that file's module doc for why these walks exist and what they
//! assert ("this call was not refused for want of a privilege", and then that it succeeded).
//! The shared checks — `assert_not_denied`, `expect_as_window`, `owner_and_window` — and the
//! note fixture `note_setup` live in `tests/common`.
//!
//! DB-gated on `$CAIRN_TEST_PG`, serialized via `db::test_serial_guard`. Key material is minted
//! at runtime.
mod common;

use cairn_node::chart_link::canonical_pair;
use cairn_node::db;
use cairn_node::medication::snapshot::in_snapshot;
use cairn_node::medication::AuthorParams;
use cairn_node::note::draft::{open_drafts, save_part, take_draft, SaveOutcome, TakeOutcome};
use cairn_node::note::read::{list_patient_notes, PatientNoteList};
use cairn_node::note::sign::{sign_note, SignNoteInput};
use cairn_node::note::view::{NoteGrade, PartWriter};
use common::{
    cs, expect_as_window, note_setup, owner_and_window, submit_link_event, submit_registration,
};
use tokio_postgres::Client;
use uuid::Uuid;

const ORIGIN: &str = "window-role-grants-notes";

/// The note pane's read exactly as `notes::note_list_impl` makes it: in one snapshot (R12).
async fn note_pane_read(c: &mut Client, chart: Uuid) -> anyhow::Result<PatientNoteList> {
    in_snapshot(c, async |tx| list_patient_notes(tx, chart).await).await
}

/// The note pane (slices 3a, 3b-1): save a part, list drafts, move one, read the who's-here
/// directory, sign (which spends the draft's parts), and read back — notes and an open draft —
/// over one chart, then over a linked set, undoubted and doubted, so withholding runs too.
#[tokio::test]
async fn the_windows_note_calls_run_under_its_own_login() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let (owner, mut window) = owner_and_window(&base).await;
    let (sk, kid, sk_h, kid_h, a) = note_setup(&owner).await;
    let author = AuthorParams {
        human_sk: &sk_h,
        human_kid: &kid_h,
    };
    let x = Uuid::now_v7();
    submit_registration(&owner, &sk, &kid, x, 1).await;
    for (chart, text) in [(a, "BP <90 [L] arm (R)"), (x, "note on X")] {
        let draft = Uuid::now_v7();
        // A NAMED writer, so the sign's key lookup (`cairn_vault_member_key`) runs as the window.
        let r = save_part(
            &window,
            draft,
            chart,
            "pc-window",
            Some("notewriter"),
            text,
            None,
        )
        .await;
        assert_eq!(
            expect_as_window("note::draft::save_part", r),
            SaveOutcome::Saved
        );
        let r = open_drafts(&window, chart).await;
        assert_eq!(expect_as_window("note::draft::open_drafts", r).len(), 1);
        let input = SignNoteInput {
            draft,
            patient: chart,
            terminal: "pc-window",
            writer: Some("notewriter"),
            narrative: text,
            stated: None,
            node_origin: ORIGIN,
        };
        let r = sign_note(&mut window, &sk, &input, &author).await;
        expect_as_window("note::sign::sign_note", r).expect("the note is signed");
    }
    // One draft left UNSIGNED on A, with a stated time, so the snapshot read below decodes a
    // draft row (every column, the stated time included) under the window's login.
    let stated = "2026-11-02T17:00:30Z".parse().unwrap();
    let open = Uuid::now_v7();
    let r = save_part(
        &window,
        open,
        a,
        "pc-window",
        Some("notewriter"),
        "still typing",
        Some(stated),
    )
    .await;
    assert_eq!(
        expect_as_window("note::draft::save_part (left open)", r),
        SaveOutcome::Saved
    );
    // The move door (db/066), then back, so the read below still sees it as this terminal's.
    for terminal in ["pc-elsewhere", "pc-window"] {
        let r = take_draft(&window, open, a, terminal).await;
        assert_eq!(
            expect_as_window("note::draft::take_draft", r),
            TakeOutcome::Taken
        );
    }
    let one = expect_as_window(
        "note::read::list_patient_notes (one chart)",
        note_pane_read(&mut window, a).await,
    );
    assert_eq!(one.notes.len(), 1, "A alone holds its own note");
    assert_eq!(one.notes[0].text, "BP <90 [L] arm (R)", "shown as typed");
    assert_eq!(one.drafts.len(), 1, "the open draft is read");
    let part = &one.drafts[0].parts[0];
    assert_eq!(part.narrative, "still typing");
    assert_eq!(part.stated_time, Some(stated));
    assert_eq!(
        part.writer_name.as_deref(),
        Some(common::NOTE_AUTHOR),
        "the writer's name is read through the parts door"
    );

    // The who's-here directory (db/066): the note fixture's clinician is offered.
    let staff = expect_as_window(
        "note::whos_here::directory",
        cairn_node::note::whos_here::directory(&window).await,
    );
    assert!(
        staff.iter().any(|s| s.staff_id == "notewriter"),
        "{staff:?}"
    );
    // 3a's three draft doors are gone (a door's shape is final; db/066 replaced them).
    for gone in [
        "cairn_note_draft_save(uuid,uuid,text,text,timestamptz)",
        "cairn_note_drafts(uuid)",
        "cairn_note_draft_spend(uuid,uuid)",
    ] {
        let still: bool = owner
            .query_one("SELECT to_regprocedure($1) IS NOT NULL", &[&gone])
            .await
            .unwrap()
            .get(0);
        assert!(!still, "{gone} must be gone");
    }

    // Linked: X's note joins the read; flagged in doubt, it is counted and withheld instead.
    submit_link_event(&owner, &sk, &kid, a, x, 10, true).await;
    let linked = expect_as_window(
        "note::read::list_patient_notes (linked)",
        note_pane_read(&mut window, a).await,
    );
    assert_eq!(linked.notes.len(), 2, "the linked chart's note is read");
    let (lo, hi) = canonical_pair(a, x);
    owner
        .execute(
            "INSERT INTO link_veto_flag (low, high, content_address) \
             SELECT low, high, content_address FROM patient_link \
             WHERE low = $1::text::uuid AND high = $2::text::uuid",
            &[&lo.to_string(), &hi.to_string()],
        )
        .await
        .unwrap();
    let doubted = expect_as_window(
        "note::read::list_patient_notes (doubted)",
        note_pane_read(&mut window, a).await,
    );
    assert_eq!(doubted.notes.len(), 1, "X's note is withheld");
    assert_eq!(doubted.withheld_on_linked, 1, "and counted");
}

/// The quarantine commit (slice 3b-2a) under the window's login: the policy read and the
/// overdue hint (through `open_drafts`' due words and `overdue_drafts`), and the commit door
/// driven to its deep path — an overdue draft actually committed. The console's setter is
/// refused here (42501): a site's quarantine is set at its own console only.
#[tokio::test]
async fn the_windows_quarantine_commit_runs_under_its_own_login() {
    use cairn_node::note::commit::{commit_overdue_drafts, overdue_drafts};
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let (owner, mut window) = owner_and_window(&base).await;
    let (sk, _, _, _, a) = note_setup(&owner).await;
    let draft = Uuid::now_v7();
    let r = save_part(
        &window,
        draft,
        a,
        "pc-window",
        Some("notewriter"),
        "left",
        None,
    )
    .await;
    assert_eq!(expect_as_window("save_part", r), SaveOutcome::Saved);
    owner
        .execute(
            "UPDATE note_draft SET last_keystroke = last_keystroke - interval '181 minutes' \
              WHERE draft_id = $1::text::uuid",
            &[&draft.to_string()],
        )
        .await
        .unwrap();

    let r = open_drafts(&window, a).await;
    let drafts = expect_as_window("open_drafts (due, overdue)", r);
    assert!(
        drafts[0].overdue,
        "cairn_note_quarantine_minutes + the hint ran"
    );
    let r = overdue_drafts(&window, Some(a)).await;
    assert_eq!(expect_as_window("overdue_drafts", r), vec![draft]);
    let r = commit_overdue_drafts(&mut window, &sk, Some(a), ORIGIN).await;
    let report = expect_as_window("commit_overdue_drafts", r);
    assert_eq!(report.committed, 1, "{report:?}");
    assert!(report.failed.is_empty(), "{report:?}");
    // The read of that note runs `cairn_note_key_is_human` under the window's own login: the
    // named writer's part grades "went in unsigned", never "writer not known".
    let r = note_pane_read(&mut window, a).await;
    let list = expect_as_window("note pane read (the human-key door)", r);
    let note = list
        .notes
        .iter()
        .find(|n| n.text == "left")
        .expect("the committed part");
    assert_eq!(note.grade, NoteGrade::WentInUnsigned);
    assert_eq!(note.writer, PartWriter::Drafted(common::NOTE_AUTHOR.into()));

    let refused = window
        .execute("SELECT cairn_note_set_quarantine(60)", &[])
        .await
        .expect_err("the window may not set the quarantine");
    assert_eq!(
        refused.code(),
        Some(&tokio_postgres::error::SqlState::INSUFFICIENT_PRIVILEGE)
    );
}

/// The audited discard (slice 3b-2b) under the window's login, driven to its deep path: the
/// save, db/069's door, the seal and the strict submit all run as `cairn_window`, and db/065's
/// commit-time trigger accepts the event. Then the note pane's read decodes the marker (the
/// projection, `cairn_t_effective` and `cairn_note_signer_name`) under the same login.
#[tokio::test]
async fn the_windows_discard_runs_under_its_own_login() {
    use cairn_node::note::discard::{discard_draft, DiscardOutcome};
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let (owner, mut window) = owner_and_window(&base).await;
    let (sk, _, sk_h, kid_h, a) = note_setup(&owner).await;
    let author = AuthorParams {
        human_sk: &sk_h,
        human_kid: &kid_h,
    };
    let draft = Uuid::now_v7();
    let input = SignNoteInput {
        draft,
        patient: a,
        terminal: "pc-window",
        writer: Some("notewriter"),
        narrative: "wrong patient",
        stated: Some("2026-11-04T09:00:00Z".parse().unwrap()),
        node_origin: ORIGIN,
    };
    let r = discard_draft(&mut window, &sk, &input, &author).await;
    let out = expect_as_window("note::discard::discard_draft", r);
    assert!(matches!(out, Ok(DiscardOutcome::Recorded(_))), "{out:?}");

    let r = note_pane_read(&mut window, a).await;
    let list = expect_as_window("note pane read (discard markers)", r);
    assert_eq!(list.discards.len(), 1);
    assert_eq!(
        list.discards[0].discarder.as_deref(),
        Some(common::NOTE_AUTHOR)
    );
    assert_eq!(list.discards[0].parts[0].text, "wrong patient");
    assert_eq!(list.discards[0].parts[0].stated, input.stated);
}
