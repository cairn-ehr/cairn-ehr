//! Sign-in slice 3a, Task 2 — the `clinical.note.asserted` stream end to end below the window:
//! a note built by `cairn_event::note` is sealed, signed by a HUMAN author (ADR-0053), admitted
//! by the strict door's floor (`db/064`'s `cairn_check_note_assertion`), and projected in the
//! clear into `clinical_note`; a shred then scrubs that projection row (db/037).
//!
//! `db/tests/064_clinical_note_test.sql` pins the floor rule by rule; this file proves the real
//! pipeline produces a body that floor admits, and that the projection holds the signed
//! narrative byte for byte (the escaped text — the window unescapes on display, ruling R4).
//!
//! DB-gated on `$CAIRN_TEST_PG`, serialized via `db::test_serial_guard`.
use cairn_event::note::{
    escape_note_markup, note_payload, note_t_effective, render_note_twin, NoteAssertion,
    StatedTime, NOTE_EVENT_TYPE, NOTE_SCHEMA_VERSION,
};
use cairn_event::{EventBody, SigningKey};
use cairn_node::db;
use cairn_node::medication::sealed_submit::seal_sign_submit;
use cairn_node::medication::AuthorParams;
use cairn_node::shred::shred_event;
use tokio_postgres::Client;
use uuid::Uuid;

mod common;

/// Assemble a clear `clinical.note.asserted` body, device-shaped (`seal_sign_submit` rewrites
/// it to human authorship). Modelled on `medication::assert::build_assert_body`.
fn note_body(
    event_id: Uuid,
    patient: Uuid,
    node_kid: &str,
    hlc: cairn_event::Hlc,
    a: &NoteAssertion,
) -> EventBody {
    EventBody {
        event_id: event_id.to_string(),
        patient_id: patient.to_string(),
        event_type: NOTE_EVENT_TYPE.into(),
        schema_version: NOTE_SCHEMA_VERSION.into(),
        hlc,
        t_effective: Some(note_t_effective(a)),
        signer_key_id: node_kid.into(),
        contributors: serde_json::json!([{"actor_id": node_kid, "role": "recorded"}]),
        payload: note_payload(a),
        attachments: vec![],
        plaintext_twin: Some(render_note_twin(a)),
        clock_grade: cairn_event::ClockGrade::SelfAsserted,
        safety: None,
    }
}

/// The node's signing key and key id, as `common::medication_setup` enrolled them.
struct Node {
    sk: SigningKey,
    kid: String,
}

/// A registered chart with one human-authored note on it. Returns the node (to sign a later
/// shred), the note's event id, and the escaped narrative that was signed.
async fn chart_with_one_note(c: &mut Client, typed: &str) -> (Node, Uuid, String) {
    let (node_sk, node_kid, human_sk, human_kid) = common::medication_setup(c).await;
    let patient = Uuid::now_v7();
    // #345: a chart's first event is its registration.
    common::submit_registration(c, &node_sk, &node_kid, patient, 0).await;

    let narrative = escape_note_markup(typed);
    let a = NoteAssertion {
        narrative: &narrative,
        stated: StatedTime::NotStated,
        recording_started: "2026-01-02T03:04:05Z",
        recording_ended: "2026-01-02T03:09:05Z",
    };
    let hlc = db::next_hlc(c, &node_kid).await.unwrap();
    let event_id = Uuid::now_v7();
    let author = AuthorParams {
        human_sk: &human_sk,
        human_kid: &human_kid,
    };
    let recorded = seal_sign_submit(
        c,
        &node_sk,
        note_body(event_id, patient, &node_kid, hlc, &a),
        Some(&author),
        None,
    )
    .await
    .unwrap_or_else(|e| panic!("the strict door admits a note the window would write: {e:#}"));
    assert_eq!(recorded, event_id);
    let node = Node {
        sk: node_sk,
        kid: node_kid,
    };
    (node, event_id, narrative)
}

/// Rows in `clinical_note` for one note.
async fn note_rows(c: &Client, id: Uuid) -> i64 {
    c.query_one(
        "SELECT count(*) FROM clinical_note WHERE event_id = $1::text::uuid",
        &[&id.to_string()],
    )
    .await
    .unwrap()
    .get(0)
}

#[tokio::test]
async fn a_sealed_note_submits_and_projects_its_clear_text() {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    // Text a clinician really types that looks like markup: it must sign, not be refused.
    let typed = "BP <90 on arrival, [L] arm (R)\nsee [ref]: discharge letter\nends with \\";
    let (_, id, narrative) = chart_with_one_note(&mut c, typed).await;

    let rows = c
        .query(
            "SELECT narrative, effective_basis, t_effective = recording_started, refs::text \
               FROM clinical_note WHERE event_id = $1::text::uuid",
            &[&id.to_string()],
        )
        .await
        .unwrap();
    assert_eq!(rows.len(), 1, "one projected row per note");
    assert_eq!(
        rows[0].get::<_, String>(0),
        narrative,
        "the signed narrative, byte for byte"
    );
    assert_eq!(rows[0].get::<_, String>(1), "recording-started");
    assert!(
        rows[0].get::<_, bool>(2),
        "no stated time: t_effective is recording start"
    );
    assert_eq!(rows[0].get::<_, String>(3), "[]");

    // The event itself is sealed: its stored body is ciphertext, not the narrative.
    let sealed: bool = c
        .query_one(
            "SELECT body::text NOT LIKE '%arrival%' FROM event_log WHERE event_id = $1::text::uuid",
            &[&id.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert!(sealed, "the note is born sealed (ADR-0052)");
}

#[tokio::test]
async fn a_shred_scrubs_the_note_projection() {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (node, id, _) = chart_with_one_note(&mut c, "pt distressed, disclosed history").await;
    assert_eq!(note_rows(&c, id).await, 1);

    shred_event(
        &mut c,
        &node.sk,
        &node.kid,
        &node.kid,
        id,
        "retention ceiling",
        None,
    )
    .await
    .expect("the shred is admitted");
    assert_eq!(
        note_rows(&c, id).await,
        0,
        "the narrative is clinical plaintext: the shred scrubs its row"
    );
}

/// Every string of length 0..=5 over `\ < ( ] : a` (9,331 of them): the small alphabet whose
/// every interleaving `cairn_event::note`'s exhaustive test walks.
fn small_alphabet_strings() -> Vec<String> {
    let alphabet = ['\\', '<', '(', ']', ':', 'a'];
    let mut all = Vec::new();
    let mut level = vec![String::new()];
    for _ in 0..=5 {
        let next = level
            .iter()
            .flat_map(|s| alphabet.iter().map(move |c| format!("{s}{c}")))
            .collect();
        all.append(&mut level);
        level = next;
    }
    all
}

/// Final review M6: the Rust escape's inertness test judges "live markup" by its OWN definition,
/// so this ties it to the REAL floor. Every escaped string of the exhaustive set goes through
/// db/064's `cairn_check_note_markup` (the markup rules, alone), in a few batched queries, and
/// none may be refused; what comes back out of PostgreSQL must unescape to the typed string. A
/// future floor rule the escape does not satisfy fails here, not at a clinician's Sign.
#[tokio::test]
async fn every_escaped_string_of_the_small_alphabet_passes_the_real_floor() {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let typed = small_alphabet_strings();
    assert_eq!(typed.len(), 9_331);

    for chunk in typed.chunks(2_000) {
        let escaped: Vec<String> = chunk.iter().map(|s| escape_note_markup(s)).collect();
        // The void check in FROM runs once per string; a refusal aborts the batch with its sentence.
        let rows = c
            .query(
                "SELECT t FROM unnest($1::text[]) WITH ORDINALITY AS u(t, i), \
                        LATERAL cairn_check_note_markup(u.t) ORDER BY u.i",
                &[&escaped],
            )
            .await
            .unwrap_or_else(|e| panic!("the floor refused an escaped string: {e:?}"));
        assert_eq!(rows.len(), chunk.len(), "one row back per string");
        for (row, s) in rows.iter().zip(chunk) {
            assert_eq!(
                cairn_event::note::unescape_note_markup(&row.get::<_, String>(0)),
                *s,
                "round trip through the floor"
            );
        }
    }
}
