//! Sign-in slice 3c-1, Task 1 — the addendum draft (db/070) and the same-chart floor rule
//! (db/064 rule 6, authoring arm).
//!
//! WHY: "Add to this note" starts a draft that remembers WHICH signed note it adds to
//! (`note_draft.addendum_to`, set once when the header is made, never changed). These tests drive
//! db/070's two doors under the window's own login (`cairn_window`), the way the window will, and
//! pin their answer words: a target must be a note of THIS chart (`no-such-note` otherwise), a
//! blank first save makes no header, a draft begun as a new note can never become an addendum
//! (`elsewhere`), and a later plain save (db/066's door) keeps the target. Then the floor: a signed
//! note whose `addendum-to` names another chart's note, or an event that is not a note, is refused
//! at the strict door with the new message, while the REMOTE door keeps only its existence check
//! (#948: its bare refusal is what the sync puller pens and re-offers).
//!
//! DB-gated on `$CAIRN_TEST_PG`, serialized via `db::test_serial_guard` taken BEFORE connecting.
mod common;

use cairn_event::note::{
    note_payload, note_t_effective, render_note_twin, EscapedNarrative, NoteAssertion, StatedTime,
    NOTE_EVENT_TYPE, NOTE_SCHEMA_VERSION,
};
use cairn_event::{EventBody, SigningKey};
use cairn_node::db;
use cairn_node::medication::sealed_submit::seal_sign_submit;
use cairn_node::medication::{AssertMedicationInput, AuthorParams};
use common::{note_setup, owner_and_window, submit_registration};
use tokio_postgres::Client;
use uuid::Uuid;

/// The floor's new refusal, verbatim up to its `%` (the named event).
const NOT_A_NOTE_HERE: &str =
    "note assertion: an addendum-to target must be a note on this chart, got ";

/// The keys `note_setup` minted: the node's, and the human author's who signs every note here.
struct Keys {
    node_sk: SigningKey,
    node_kid: String,
    human_sk: SigningKey,
    human_kid: String,
}

/// A clear `clinical.note.asserted` body for `patient`, optionally `addendum-to` the event
/// `target`, device-shaped (`seal_sign_submit` rewrites it to human authorship). The same shape
/// as `note_floor.rs`'s `note_body`, with the times fixed.
fn note_body(
    event_id: Uuid,
    patient: Uuid,
    k: &Keys,
    hlc: cairn_event::Hlc,
    target: Option<&str>,
) -> EventBody {
    let a = NoteAssertion {
        narrative: EscapedNarrative::of_typed("chest clear"),
        stated: StatedTime::NotStated,
        recording_started: "2026-11-05T01:00:00Z",
        recording_ended: "2026-11-05T01:05:00Z",
        addendum_to: target,
    };
    EventBody {
        event_id: event_id.to_string(),
        patient_id: patient.to_string(),
        event_type: NOTE_EVENT_TYPE.into(),
        schema_version: NOTE_SCHEMA_VERSION.into(),
        hlc,
        t_effective: Some(note_t_effective(&a)),
        signer_key_id: k.node_kid.clone(),
        contributors: serde_json::json!([{"actor_id": k.node_kid, "role": "recorded"}]),
        payload: note_payload(&a),
        attachments: vec![],
        plaintext_twin: Some(render_note_twin(&a)),
        clock_grade: cairn_event::ClockGrade::SelfAsserted,
        safety: None,
    }
}

/// Seal, sign and submit a note on `patient` through the strict door (as the owner), optionally
/// `addendum-to` `target`. Returns the note's event id, or the door's refusal.
async fn signed_note(
    c: &mut Client,
    k: &Keys,
    patient: Uuid,
    target: Option<Uuid>,
) -> anyhow::Result<Uuid> {
    let hlc = db::next_hlc(c, &k.node_kid).await?;
    let author = AuthorParams {
        human_sk: &k.human_sk,
        human_kid: &k.human_kid,
    };
    let target = target.map(|t| t.to_string());
    let body = note_body(Uuid::now_v7(), patient, k, hlc, target.as_deref());
    seal_sign_submit(c, &k.node_sk, body, Some(&author), None).await
}

/// `note_setup` plus one signed note on its chart (the target every draft below adds to).
/// Returns `(owner, window, keys, chart, root note)`.
async fn chart_with_a_note(base: &str) -> (Client, Client, Keys, Uuid, Uuid) {
    let (mut owner, window) = owner_and_window(base).await;
    let (node_sk, node_kid, human_sk, human_kid, chart) = note_setup(&owner).await;
    let k = Keys {
        node_sk,
        node_kid,
        human_sk,
        human_kid,
    };
    let root = signed_note(&mut owner, &k, chart, None)
        .await
        .expect("the root note signs");
    (owner, window, k, chart, root)
}

/// A second registered chart, with a signed note of its own. Returns `(chart, its note)`.
async fn other_chart_with_a_note(owner: &mut Client, k: &Keys) -> (Uuid, Uuid) {
    let x = Uuid::now_v7();
    submit_registration(owner, &k.node_sk, &k.node_kid, x, 5).await;
    let note = signed_note(owner, k, x, None)
        .await
        .expect("X's note signs");
    (x, note)
}

/// db/070's addendum save door, called as the window would: terminal `pc-1`, writer
/// `notewriter`, no stated time.
async fn addendum_save(c: &Client, draft: Uuid, chart: Uuid, text: &str, target: Uuid) -> String {
    c.query_one(
        "SELECT cairn_note_draft_addendum_part_save($1::text::uuid, $2::text::uuid, 'pc-1', \
                'notewriter', $3, NULL, $4::text::uuid)",
        &[
            &draft.to_string(),
            &chart.to_string(),
            &text,
            &target.to_string(),
        ],
    )
    .await
    .unwrap_or_else(|e| panic!("the addendum door answers: {}", common::db_msg(&e)))
    .get(0)
}

/// db/066's plain save door, same terminal and writer.
async fn plain_save(c: &Client, draft: Uuid, chart: Uuid, text: &str) -> String {
    c.query_one(
        "SELECT cairn_note_draft_part_save($1::text::uuid, $2::text::uuid, 'pc-1', 'notewriter', $3, NULL)",
        &[&draft.to_string(), &chart.to_string(), &text],
    )
    .await
    .unwrap_or_else(|e| panic!("the part door answers: {}", common::db_msg(&e)))
    .get(0)
}

/// What the draft's header says it adds to (db/070's reader), as text; `None` = a new note's
/// draft, or no draft.
async fn addendum_to(c: &Client, draft: Uuid) -> Option<String> {
    c.query_one(
        "SELECT cairn_note_draft_addendum_to($1::text::uuid)::text",
        &[&draft.to_string()],
    )
    .await
    .unwrap_or_else(|e| panic!("the reader answers: {}", common::db_msg(&e)))
    .get(0)
}

/// The chart's open draft parts, through db/066's listing door.
async fn part_count(c: &Client, chart: Uuid) -> i64 {
    c.query_one(
        "SELECT count(*) FROM cairn_note_draft_parts($1::text::uuid)",
        &[&chart.to_string()],
    )
    .await
    .unwrap()
    .get(0)
}

#[tokio::test]
async fn an_addendum_draft_names_its_note() {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let (_owner, window, _k, chart, root) = chart_with_a_note(&base).await;
    let draft = Uuid::now_v7();
    assert_eq!(
        addendum_save(&window, draft, chart, "plan: review at 1600", root).await,
        "saved"
    );
    assert_eq!(addendum_to(&window, draft).await, Some(root.to_string()));
    assert_eq!(
        part_count(&window, chart).await,
        1,
        "the text is part 1, as db/066 writes it"
    );
}

#[tokio::test]
async fn a_blank_first_save_makes_no_header() {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let (_owner, window, _k, chart, root) = chart_with_a_note(&base).await;
    let draft = Uuid::now_v7();
    // A click into "Add to this note" and out again writes nothing, exactly as db/066's
    // empty-new-writer rule writes no part.
    assert_eq!(
        addendum_save(&window, draft, chart, " \u{a0}\n", root).await,
        "saved"
    );
    assert_eq!(
        addendum_to(&window, draft).await,
        None,
        "no header was made"
    );
    assert_eq!(part_count(&window, chart).await, 0, "and no part");
}

#[tokio::test]
async fn a_target_on_another_chart_is_no_such_note() {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let (mut owner, window, k, chart, _root) = chart_with_a_note(&base).await;
    let (_, xs_note) = other_chart_with_a_note(&mut owner, &k).await;
    let draft = Uuid::now_v7();
    assert_eq!(
        addendum_save(&window, draft, chart, "plan", xs_note).await,
        "no-such-note"
    );
    assert_eq!(
        addendum_to(&window, draft).await,
        None,
        "nothing was written"
    );
    assert_eq!(part_count(&window, chart).await, 0);
}

#[tokio::test]
async fn a_target_that_is_not_a_note_is_no_such_note() {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let (mut owner, window, k, chart, _root) = chart_with_a_note(&base).await;
    let med_event = medication_event(&mut owner, &k, chart).await;
    let draft = Uuid::now_v7();
    assert_eq!(
        addendum_save(&window, draft, chart, "plan", med_event).await,
        "no-such-note"
    );
    assert_eq!(
        addendum_to(&window, draft).await,
        None,
        "nothing was written"
    );
}

#[tokio::test]
async fn a_new_notes_draft_refuses_a_target_later() {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let (_owner, window, _k, chart, root) = chart_with_a_note(&base).await;
    let draft = Uuid::now_v7();
    assert_eq!(
        plain_save(&window, draft, chart, "a new note").await,
        "saved"
    );
    // The header said "a new note" when it was made; no later save may turn it into an addendum.
    assert_eq!(
        addendum_save(&window, draft, chart, "a new note, more", root).await,
        "elsewhere"
    );
    assert_eq!(addendum_to(&window, draft).await, None);
}

#[tokio::test]
async fn a_later_plain_save_keeps_the_target() {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let (_owner, window, _k, chart, root) = chart_with_a_note(&base).await;
    let draft = Uuid::now_v7();
    assert_eq!(
        addendum_save(&window, draft, chart, "plan", root).await,
        "saved"
    );
    assert_eq!(
        plain_save(&window, draft, chart, "plan: review at 1600").await,
        "saved"
    );
    assert_eq!(
        addendum_to(&window, draft).await,
        Some(root.to_string()),
        "no door updates it"
    );
}

/// A medication event on `chart`, through the real orchestrator; returns its EVENT id (the
/// orchestrator returns the thread's medication_id, so the event is read back by type).
async fn medication_event(owner: &mut Client, k: &Keys, chart: Uuid) -> Uuid {
    let author = AuthorParams {
        human_sk: &k.human_sk,
        human_kid: &k.human_kid,
    };
    let input = AssertMedicationInput {
        term: "paracetamol",
        coding: None,
        formulation: None,
        dose_amount: Some("1"),
        dose_unit: Some("g"),
        sig: None,
        info_source: "patient",
        started: None,
        started_precision: None,
        source_note: None,
    };
    cairn_node::medication::assert_medication(
        owner,
        &k.node_sk,
        &k.node_kid,
        "addendum-doors",
        chart,
        &input,
        Some(&author),
        None,
    )
    .await
    .expect("the medication is asserted");
    let id: String = owner
        .query_one(
            "SELECT event_id::text FROM event_log WHERE patient_id = $1::text::uuid \
               AND event_type = 'clinical.medication.asserted'",
            &[&chart.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    id.parse().unwrap()
}

#[tokio::test]
async fn the_floor_refuses_an_addendum_to_another_charts_note() {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let (mut owner, _window, k, chart, root) = chart_with_a_note(&base).await;
    // The passing control: an addendum to this chart's own note signs.
    signed_note(&mut owner, &k, chart, Some(root))
        .await
        .unwrap_or_else(|e| panic!("an addendum to this chart's note signs: {e:#}"));
    let (_, xs_note) = other_chart_with_a_note(&mut owner, &k).await;
    let e = signed_note(&mut owner, &k, chart, Some(xs_note))
        .await
        .expect_err("a raw-SQL client naming another chart's note is refused");
    let msg = format!("{e:#}");
    assert!(
        msg.contains(&format!("{NOT_A_NOTE_HERE}{xs_note}")),
        "{msg}"
    );
}

#[tokio::test]
async fn the_floor_refuses_an_addendum_to_a_medication_event() {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let (mut owner, _window, k, chart, _root) = chart_with_a_note(&base).await;
    let med_event = medication_event(&mut owner, &k, chart).await;
    let e = signed_note(&mut owner, &k, chart, Some(med_event))
        .await
        .expect_err("an addendum to a medication event is refused");
    let msg = format!("{e:#}");
    assert!(
        msg.contains(&format!("{NOT_A_NOTE_HERE}{med_event}")),
        "{msg}"
    );
}

/// The remote door is NOT tightened (#948): a peer's addendum whose target this node holds on
/// another chart is admitted, and only an unknown target is refused, with the bare `RAISE`
/// (`P0001`) the sync puller pens. Driven by calling the floor directly with the remote door's
/// marker raised, as db/tests/064 does: building a peer-signed, sealed note through
/// `apply_remote_event` would test the door's other steps, not this rule.
#[tokio::test]
async fn the_remote_door_still_admits_a_held_target_on_another_chart() {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let (mut owner, _window, k, chart, _root) = chart_with_a_note(&base).await;
    let (_, xs_note) = other_chart_with_a_note(&mut owner, &k).await;
    let hlc = db::next_hlc(&owner, &k.node_kid).await.unwrap();
    let body_for = |target: Uuid| {
        let t = target.to_string();
        serde_json::to_string(&note_body(Uuid::now_v7(), chart, &k, hlc.clone(), Some(&t))).unwrap()
    };
    let tx = owner.transaction().await.unwrap();
    tx.batch_execute("SET LOCAL cairn.remote_apply = 'on'")
        .await
        .unwrap();
    tx.execute(
        "SELECT cairn_check_note_assertion('clinical.note.asserted', $1::text::jsonb)",
        &[&body_for(xs_note)],
    )
    .await
    .unwrap_or_else(|e| {
        panic!(
            "the remote door admits a held target: {}",
            common::db_msg(&e)
        )
    });
    let e = tx
        .execute(
            "SELECT cairn_check_note_assertion('clinical.note.asserted', $1::text::jsonb)",
            &[&body_for(Uuid::now_v7())],
        )
        .await
        .expect_err("an unknown target is refused at the remote door");
    assert_eq!(
        e.code(),
        Some(&tokio_postgres::error::SqlState::RAISE_EXCEPTION)
    );
    assert!(
        common::db_msg(&e).contains("is not held on this node"),
        "{}",
        common::db_msg(&e)
    );
}
