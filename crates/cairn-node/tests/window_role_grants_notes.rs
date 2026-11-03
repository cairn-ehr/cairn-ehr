//! Sign-in slice 3a — the note pane's calls, run under the window's OWN login (`cairn_window`).
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
use cairn_node::note::draft::{open_drafts, save_draft, SaveOutcome};
use cairn_node::note::read::{list_patient_notes, PatientNoteList};
use cairn_node::note::sign::{sign_note, SignNoteInput};
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

/// The note pane (slice 3a): save, list drafts, sign, and read back — notes and an open draft —
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
        let r = save_draft(&window, draft, chart, "pc-window", text, None).await;
        assert_eq!(
            expect_as_window("note::draft::save_draft", r),
            SaveOutcome::Saved
        );
        let r = open_drafts(&window, chart).await;
        assert_eq!(expect_as_window("note::draft::open_drafts", r).len(), 1);
        let input = SignNoteInput {
            draft,
            patient: chart,
            terminal: "pc-window",
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
    let r = save_draft(
        &window,
        Uuid::now_v7(),
        a,
        "pc-window",
        "still typing",
        Some(stated),
    )
    .await;
    assert_eq!(
        expect_as_window("note::draft::save_draft (left open)", r),
        SaveOutcome::Saved
    );
    let one = expect_as_window(
        "note::read::list_patient_notes (one chart)",
        note_pane_read(&mut window, a).await,
    );
    assert_eq!(one.notes.len(), 1, "A alone holds its own note");
    assert_eq!(one.notes[0].text, "BP <90 [L] arm (R)", "shown as typed");
    assert_eq!(one.drafts.len(), 1, "the open draft is read");
    assert_eq!(one.drafts[0].narrative, "still typing");
    assert_eq!(one.drafts[0].stated_time, Some(stated));

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
