//! Sign-in slice 3b-2a, Task 3 — an abandoned draft goes into the record UNSIGNED after the
//! site's quarantine (`cairn_node::note::commit`).
//!
//! The timer makes one NODE-signed note event per non-blank part, chained by `addendum-to` as a
//! Sign chains them. A named writer is a `drafted` contributor beside the node's `recorded`; a
//! part by a writer not known is the node's alone. These tests pin those sets, that "overdue" is
//! the database's decision (a draft not yet due writes nothing; two commits racing one draft
//! write it once), that an all-blank draft leaves nothing, that a writer with no key makes the
//! draft STUCK rather than "not known", that a Sign arriving after the timer is refused
//! `Committed`, and that the read grades the result "went in unsigned".
//!
//! "Overdue" is made by moving the draft's last keystroke back as the owner — never by sleeping.
//!
//! DB-gated on `$CAIRN_TEST_PG`, serialized via `db::test_serial_guard` taken BEFORE connecting.
//! Key material is minted at runtime (house rule 6).
mod common;

use cairn_event::generate_key;
use cairn_event::SigningKey;
use cairn_node::db;
use cairn_node::medication::AuthorParams;
use cairn_node::note::commit::{
    commit_overdue, commit_overdue_drafts, overdue_drafts, CommitOutcome,
};
use cairn_node::note::draft::{open_drafts, save_part, SaveOutcome};
use cairn_node::note::read::list_patient_notes;
use cairn_node::note::sign::{sign_note, SignNoteInput, SignRefusal};
use cairn_node::note::view::{NoteGrade, PartWriter};
use chrono::{DateTime, Utc};
use tokio_postgres::Client;
use uuid::Uuid;

const ORIGIN: &str = "note-commit-test-node";
const TERMINAL: &str = "ward-3-pc";

/// db/067's `cairn_note_drafts_due` arithmetic, as the test's oracle: the header's last keystroke
/// plus the quarantine. Only here — production reads `due` from the database, never computes it.
fn draft_due(last_keystroke: DateTime<Utc>, quarantine_minutes: i64) -> DateTime<Utc> {
    last_keystroke + chrono::Duration::minutes(quarantine_minutes)
}

/// Enrol `Nurse A` (`nurse-a`) as a vault member with a fresh key, as `note_sign_parts.rs`
/// does. Returns her key id.
async fn enrol_nurse_a(c: &Client) -> String {
    let (_sk, kid): (SigningKey, String) = generate_key().unwrap();
    c.execute(
        "SELECT enroll_actor('human', '{\"role\":\"clinician\",\"handle\":\"nurse-a\"}', $1)",
        &[&kid],
    )
    .await
    .unwrap();
    c.execute(
        "INSERT INTO vault_member (staff_id, display_name, role, actor_id, signing_key_id, \
                                   sealed, enrolled_by, contact_email) \
         SELECT 'nurse-a', 'Nurse A', 'nurse', actor_id, $1, '\\x00'::bytea, 'console', \
                'a@example.org' \
           FROM actor_event WHERE signing_key_id = $1 AND op = 'enroll' LIMIT 1",
        &[&kid],
    )
    .await
    .unwrap();
    kid
}

/// Nurse A leaves the vault (the owner deletes her member row): her key is gone.
async fn remove_nurse_a(c: &Client) {
    c.batch_execute(
        "DELETE FROM vault_attempt WHERE staff_id = 'nurse-a'; \
         DELETE FROM vault_member WHERE staff_id = 'nurse-a';",
    )
    .await
    .unwrap();
}

async fn save_as(
    c: &Client,
    draft: Uuid,
    chart: Uuid,
    writer: Option<&str>,
    text: &str,
    stated: Option<DateTime<Utc>>,
) {
    let saved = save_part(c, draft, chart, TERMINAL, writer, text, stated)
        .await
        .unwrap();
    assert_eq!(saved, SaveOutcome::Saved, "{writer:?} saves {text:?}");
}

/// Make `draft` as if it were typed `minutes` earlier: its header's and parts' recording times
/// all move back together (moving only the last keystroke could put it before the start, which
/// the floor refuses in a note).
async fn age(c: &Client, draft: Uuid, minutes: i32) {
    c.execute(
        "WITH h AS (UPDATE note_draft SET last_keystroke = last_keystroke - make_interval(mins => $2), \
                           recording_started = recording_started - make_interval(mins => $2) \
                     WHERE draft_id = $1::text::uuid) \
         UPDATE note_draft_part SET last_keystroke = last_keystroke - make_interval(mins => $2), \
                recording_started = recording_started - make_interval(mins => $2) \
          WHERE draft_id = $1::text::uuid",
        &[&draft.to_string(), &minutes],
    )
    .await
    .unwrap();
}

async fn note_count(c: &Client, patient: Uuid) -> i64 {
    c.query_one(
        "SELECT count(*) FROM clinical_note WHERE patient_id = $1::text::uuid",
        &[&patient.to_string()],
    )
    .await
    .unwrap()
    .get(0)
}

fn contributor(kid: &str, role: &str) -> serde_json::Value {
    serde_json::json!({"actor_id": kid, "role": role})
}

/// One committed event as stored: signer, contributors, refs, and the note's times.
struct Stored {
    signer: String,
    contributors: serde_json::Value,
    refs: serde_json::Value,
    t_effective: DateTime<Utc>,
    basis: String,
    recording_started: DateTime<Utc>,
    recording_ended: DateTime<Utc>,
}

async fn stored(c: &Client, event: Uuid) -> Stored {
    let r = c
        .query_one(
            "SELECT e.signer_key_id, e.contributors::text, n.refs::text, n.t_effective, \
                    n.effective_basis, n.recording_started, n.recording_ended \
               FROM event_log e JOIN clinical_note n ON n.event_id = e.event_id \
              WHERE e.event_id = $1::text::uuid",
            &[&event.to_string()],
        )
        .await
        .unwrap();
    Stored {
        signer: r.get(0),
        contributors: serde_json::from_str(&r.get::<_, String>(1)).unwrap(),
        refs: serde_json::from_str(&r.get::<_, String>(2)).unwrap(),
        t_effective: r.get(3),
        basis: r.get(4),
        recording_started: r.get(5),
        recording_ended: r.get(6),
    }
}

/// Unwrap a `Committed` outcome's event ids.
fn committed(out: CommitOutcome) -> Vec<Uuid> {
    match out {
        CommitOutcome::Committed(c) => {
            assert_eq!(c.event_id, c.value[0], "the spend names part 1's event");
            c.value
        }
        other => panic!("expected Committed, got {other:?}"),
    }
}

#[tokio::test]
async fn an_overdue_two_writer_draft_goes_in_as_two_node_signed_events() {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (node_sk, node_kid, _, b_kid, chart) = common::note_setup(&c).await;
    let a_kid = enrol_nurse_a(&c).await;
    let stated: DateTime<Utc> = "2026-11-02T17:00:00Z".parse().unwrap();
    let draft = Uuid::now_v7();
    save_as(
        &c,
        draft,
        chart,
        Some("nurse-a"),
        "obs stable",
        Some(stated),
    )
    .await;
    save_as(&c, draft, chart, Some("notewriter"), "plan: review", None).await;
    age(&c, draft, 181).await;
    let parts = open_drafts(&c, chart).await.unwrap().remove(0).parts;
    assert_eq!(overdue_drafts(&c, Some(chart)).await.unwrap(), vec![draft]);

    let ids = committed(
        commit_overdue(&mut c, &node_sk, draft, ORIGIN)
            .await
            .unwrap(),
    );

    let [first, second] = ids[..] else {
        panic!("two parts, two events: {ids:?}");
    };
    let one = stored(&c, first).await;
    let two = stored(&c, second).await;
    for s in [&one, &two] {
        assert_eq!(s.signer, node_kid, "the node signs a timer's commit");
    }
    assert_eq!(
        one.contributors,
        serde_json::json!([
            contributor(&a_kid, "drafted"),
            contributor(&node_kid, "recorded")
        ])
    );
    assert_eq!(
        two.contributors,
        serde_json::json!([
            contributor(&b_kid, "drafted"),
            contributor(&node_kid, "recorded")
        ])
    );
    assert_eq!(one.refs, serde_json::json!([]));
    assert_eq!(
        two.refs,
        serde_json::json!([{"rel": "addendum-to", "event": first.to_string()}])
    );
    for (s, p) in [(&one, &parts[0]), (&two, &parts[1])] {
        assert_eq!(s.recording_started, p.recording_started);
        assert_eq!(s.recording_ended, p.last_keystroke);
    }
    assert_eq!(
        (one.t_effective, one.basis.as_str()),
        (stated, "stated"),
        "the quarantine never touches the stated time"
    );
    assert!(open_drafts(&c, chart).await.unwrap().is_empty());
}

/// PR #981 review: a draft whose blank parts sit BETWEEN text parts (a writer cleared their own
/// tail, then another wrote on). The blank parts are skipped; the first event WRITTEN, not part 1,
/// is the note the spend names (or db/065's commit-time trigger would refuse every pass, and the
/// draft would sit overdue forever); and the addendum chain skips the blanks.
#[tokio::test]
async fn blank_parts_between_text_parts_are_skipped() {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (node_sk, _, _, b_kid, chart) = common::note_setup(&c).await;
    enrol_nurse_a(&c).await;
    let draft = Uuid::now_v7();
    // A blank part arises only from a writer clearing their OWN tail (a new writer's blank save
    // makes no part): so each blank here was typed, then cleared.
    save_as(&c, draft, chart, Some("nurse-a"), "typo", None).await;
    save_as(&c, draft, chart, Some("nurse-a"), " \n ", None).await;
    save_as(&c, draft, chart, Some("notewriter"), "plan: review", None).await;
    save_as(&c, draft, chart, Some("nurse-a"), "typo", None).await;
    save_as(&c, draft, chart, Some("nurse-a"), "\u{00A0}", None).await;
    save_as(&c, draft, chart, Some("notewriter"), "obs stable", None).await;
    assert_eq!(open_drafts(&c, chart).await.unwrap()[0].parts.len(), 4);
    age(&c, draft, 181).await;

    // `committed` also asserts the spend names the first event written.
    let ids = committed(
        commit_overdue(&mut c, &node_sk, draft, ORIGIN)
            .await
            .unwrap(),
    );

    let [first, second] = ids[..] else {
        panic!("two text parts, two events: {ids:?}");
    };
    let one = stored(&c, first).await;
    let two = stored(&c, second).await;
    assert_eq!(one.refs, serde_json::json!([]));
    assert_eq!(
        two.refs,
        serde_json::json!([{"rel": "addendum-to", "event": first.to_string()}])
    );
    for s in [&one, &two] {
        assert_eq!(s.contributors[0], contributor(&b_kid, "drafted"));
    }
    let mut texts: Vec<_> = list_patient_notes(&c, chart)
        .await
        .unwrap()
        .notes
        .into_iter()
        .map(|n| n.text)
        .collect();
    texts.sort();
    assert_eq!(texts, ["obs stable", "plan: review"]);
    assert!(open_drafts(&c, chart).await.unwrap().is_empty());
}

#[tokio::test]
async fn an_unknown_writers_part_is_the_node_alone() {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (node_sk, node_kid, _, _, chart) = common::note_setup(&c).await;
    let draft = Uuid::now_v7();
    save_as(&c, draft, chart, None, "found on the floor", None).await;
    age(&c, draft, 181).await;

    let ids = committed(
        commit_overdue(&mut c, &node_sk, draft, ORIGIN)
            .await
            .unwrap(),
    );

    assert_eq!(
        stored(&c, ids[0]).await.contributors,
        serde_json::json!([contributor(&node_kid, "recorded")])
    );
    let list = list_patient_notes(&c, chart).await.unwrap();
    assert_eq!(list.notes[0].grade, NoteGrade::WriterNotKnown);
}

#[tokio::test]
async fn a_draft_not_yet_due_is_not_due() {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (node_sk, _, _, _, chart) = common::note_setup(&c).await;
    let draft = Uuid::now_v7();
    save_as(&c, draft, chart, Some("notewriter"), "still typing", None).await;
    age(&c, draft, 179).await;

    let out = commit_overdue(&mut c, &node_sk, draft, ORIGIN)
        .await
        .unwrap();

    assert!(matches!(out, CommitOutcome::NothingWritten), "{out:?}");
    assert_eq!(note_count(&c, chart).await, 0);
    assert_eq!(open_drafts(&c, chart).await.unwrap().len(), 1, "kept");
}

#[tokio::test]
async fn an_all_blank_overdue_draft_leaves_nothing() {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (node_sk, _, _, _, chart) = common::note_setup(&c).await;
    let draft = Uuid::now_v7();
    save_as(&c, draft, chart, Some("notewriter"), "  \n ", None).await;
    age(&c, draft, 181).await;

    let out = commit_overdue(&mut c, &node_sk, draft, ORIGIN)
        .await
        .unwrap();

    assert!(matches!(out, CommitOutcome::NothingWritten), "{out:?}");
    assert_eq!(note_count(&c, chart).await, 0, "no note");
    assert!(open_drafts(&c, chart).await.unwrap().is_empty(), "no draft");
    let spent: i64 = c
        .query_one("SELECT count(*) FROM note_draft_spent", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(spent, 0, "and no spend record");
}

#[tokio::test]
async fn a_writer_with_no_key_is_stuck_and_kept() {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (node_sk, _, _, _, chart) = common::note_setup(&c).await;
    let a_kid = enrol_nurse_a(&c).await;
    let draft = Uuid::now_v7();
    save_as(&c, draft, chart, Some("notewriter"), "obs stable", None).await;
    save_as(&c, draft, chart, Some("nurse-a"), "fluids running", None).await;
    remove_nurse_a(&c).await;
    age(&c, draft, 181).await;

    let out = commit_overdue(&mut c, &node_sk, draft, ORIGIN)
        .await
        .unwrap();

    assert!(matches!(out, CommitOutcome::Stuck), "{out:?}");
    assert_eq!(note_count(&c, chart).await, 0, "part 1 rolled back too");
    let drafts = open_drafts(&c, chart).await.unwrap();
    let texts: Vec<_> = drafts[0]
        .parts
        .iter()
        .map(|p| p.narrative.as_str())
        .collect();
    assert_eq!(
        texts,
        ["obs stable", "fluids running"],
        "the draft is intact"
    );
    assert!(drafts[0].overdue, "and still shown overdue");
    assert!(
        drafts[0].stuck,
        "and read stuck: it will not go in by itself"
    );

    // ADR-0089 decision 8, as the STUCK words promise: once she is a vault member again, the next
    // pass puts the draft in, naming her key as the drafting writer. (Her actor stays enrolled; a
    // second enrol of the same person is refused by the registry, #152.)
    c.execute(
        "INSERT INTO vault_member (staff_id, display_name, role, actor_id, signing_key_id, \
                                   sealed, enrolled_by, contact_email) \
         SELECT 'nurse-a', 'Nurse A', 'nurse', actor_id, $1, '\\x00'::bytea, 'console', \
                'a@example.org' \
           FROM actor_event WHERE signing_key_id = $1 AND op = 'enroll' LIMIT 1",
        &[&a_kid],
    )
    .await
    .unwrap();
    let ids = committed(
        commit_overdue(&mut c, &node_sk, draft, ORIGIN)
            .await
            .unwrap(),
    );
    assert_eq!(ids.len(), 2, "both parts went in");
    assert_eq!(
        stored(&c, ids[1]).await.contributors[0],
        contributor(&a_kid, "drafted")
    );
    assert!(open_drafts(&c, chart).await.unwrap().is_empty());
}

#[tokio::test]
async fn one_stuck_draft_does_not_stop_the_others() {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (node_sk, _, _, _, chart) = common::note_setup(&c).await;
    enrol_nurse_a(&c).await;
    let stuck = Uuid::now_v7();
    save_as(&c, stuck, chart, Some("nurse-a"), "fluids running", None).await;
    age(&c, stuck, 200).await; // the older one, so the pass meets it first
    let fine = Uuid::now_v7();
    save_as(&c, fine, chart, Some("notewriter"), "chest clear", None).await;
    age(&c, fine, 181).await;
    remove_nurse_a(&c).await;

    let report = commit_overdue_drafts(&mut c, &node_sk, None, ORIGIN)
        .await
        .unwrap();

    assert_eq!(report.committed, 1, "{report:?}");
    assert_eq!(report.stuck, vec![stuck]);
    assert!(report.failed.is_empty(), "{report:?}");
    assert_eq!(note_count(&c, chart).await, 1);
}

/// Review Focus 2: two windows tick at once on one draft, on two connections. One door call wins
/// the advisory try-lock; the other either finds it held or finds the draft spent, answers zero
/// rows, and writes nothing.
#[tokio::test]
async fn two_commits_race_one_note() {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let mut other = db::connect(&base).await.unwrap();
    let (node_sk, _, _, _, chart) = common::note_setup(&c).await;
    enrol_nurse_a(&c).await;
    let draft = Uuid::now_v7();
    save_as(&c, draft, chart, Some("nurse-a"), "obs stable", None).await;
    save_as(&c, draft, chart, Some("notewriter"), "plan: review", None).await;
    age(&c, draft, 181).await;

    let (x, y) = tokio::join!(
        commit_overdue(&mut c, &node_sk, draft, ORIGIN),
        commit_overdue(&mut other, &node_sk, draft, ORIGIN)
    );

    let outs = [x.unwrap(), y.unwrap()];
    let wins = outs
        .iter()
        .filter(|o| matches!(o, CommitOutcome::Committed(_)))
        .count();
    let losses = outs
        .iter()
        .filter(|o| matches!(o, CommitOutcome::NothingWritten))
        .count();
    assert_eq!((wins, losses), (1, 1), "{outs:?}");
    assert_eq!(note_count(&c, chart).await, 2, "the two parts, once");
}

/// Final review M2: a draft whose lock someone else holds is being acted on (a save or a Sign
/// mid-flight), so it is not abandoned — the timer's door answers zero rows at once and writes
/// nothing, rather than making a chart read or a tick wait behind a clinician's press.
#[tokio::test]
async fn a_held_draft_lock_makes_the_commit_skip_it() {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let mut other = db::connect(&base).await.unwrap();
    let (node_sk, _, _, _, chart) = common::note_setup(&c).await;
    let draft = Uuid::now_v7();
    save_as(&c, draft, chart, Some("notewriter"), "obs stable", None).await;
    age(&c, draft, 181).await;

    // The lock every draft door takes (db/066), held by another session's open transaction.
    let holder = other.transaction().await.unwrap();
    holder
        .execute(
            "SELECT pg_advisory_xact_lock(hashtextextended('note_draft:' || $1::text::uuid::text, 0))",
            &[&draft.to_string()],
        )
        .await
        .unwrap();
    let skipped = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        commit_overdue(&mut c, &node_sk, draft, ORIGIN),
    )
    .await
    .expect("the timer never waits behind a held draft lock")
    .unwrap();
    assert!(
        matches!(skipped, CommitOutcome::NothingWritten),
        "{skipped:?}"
    );
    assert_eq!(note_count(&c, chart).await, 0, "nothing written");
    assert_eq!(
        open_drafts(&c, chart).await.unwrap().len(),
        1,
        "the draft is intact"
    );

    // Released: the next pass commits it.
    holder.rollback().await.unwrap();
    committed(
        commit_overdue(&mut c, &node_sk, draft, ORIGIN)
            .await
            .unwrap(),
    );
    assert_eq!(note_count(&c, chart).await, 1);
}

#[tokio::test]
async fn a_sign_after_the_timer_is_refused_committed() {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (node_sk, _, human_sk, human_kid, chart) = common::note_setup(&c).await;
    let draft = Uuid::now_v7();
    save_as(&c, draft, chart, Some("notewriter"), "chest clear", None).await;
    age(&c, draft, 181).await;
    committed(
        commit_overdue(&mut c, &node_sk, draft, ORIGIN)
            .await
            .unwrap(),
    );
    let author = AuthorParams {
        human_sk: &human_sk,
        human_kid: &human_kid,
    };
    let input = SignNoteInput {
        draft,
        patient: chart,
        terminal: TERMINAL,
        writer: Some("notewriter"),
        narrative: "chest clear, more",
        stated: None,
        node_origin: ORIGIN,
    };

    let out = sign_note(&mut c, &node_sk, &input, &author).await.unwrap();

    assert_eq!(out.err(), Some(SignRefusal::Committed));
    assert_eq!(note_count(&c, chart).await, 1, "nothing more was written");
}

#[tokio::test]
async fn the_read_grades_a_committed_part_asserted() {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (node_sk, _, _, _, chart) = common::note_setup(&c).await;
    enrol_nurse_a(&c).await;
    let draft = Uuid::now_v7();
    save_as(&c, draft, chart, Some("nurse-a"), "obs stable", None).await;
    age(&c, draft, 181).await;
    committed(
        commit_overdue(&mut c, &node_sk, draft, ORIGIN)
            .await
            .unwrap(),
    );

    let list = list_patient_notes(&c, chart).await.unwrap();

    assert_eq!(list.notes[0].grade, NoteGrade::WentInUnsigned);
    assert_eq!(list.notes[0].writer, PartWriter::Drafted("Nurse A".into()));
}

#[tokio::test]
async fn an_open_draft_reads_due_and_overdue() {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (_, _, _, _, chart) = common::note_setup(&c).await;
    c.execute("SELECT cairn_note_set_quarantine(30)", &[])
        .await
        .unwrap();
    let draft = Uuid::now_v7();
    save_as(&c, draft, chart, Some("notewriter"), "still typing", None).await;

    let before = open_drafts(&c, chart).await.unwrap().remove(0);
    assert_eq!(before.due, draft_due(before.last_keystroke, 30));
    assert!(!before.overdue);

    age(&c, draft, 31).await;
    let after = open_drafts(&c, chart).await.unwrap().remove(0);
    assert_eq!(after.due, draft_due(after.last_keystroke, 30));
    assert!(after.overdue);
    assert!(!after.stuck, "its writer is enrolled");
}

/// Task 3 review M1: `due` is worked from the draft HEADER's last keystroke — the input the
/// commit door and `overdue` read — never from its parts'. After a backward database clock step
/// db/066 can clamp the two apart; here the owner moves them apart by hand, and `due` and
/// `overdue` must still tell one story.
#[tokio::test]
async fn due_follows_the_header_not_the_parts() {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (_, _, _, _, chart) = common::note_setup(&c).await;
    c.execute("SELECT cairn_note_set_quarantine(30)", &[])
        .await
        .unwrap();
    let draft = Uuid::now_v7();
    save_as(&c, draft, chart, Some("notewriter"), "obs stable", None).await;
    // The header 60 minutes back, its part only 10: overdue by the header, not by the part.
    c.execute(
        "WITH h AS (UPDATE note_draft SET last_keystroke = last_keystroke - interval '60 min', \
                           recording_started = recording_started - interval '60 min' \
                     WHERE draft_id = $1::text::uuid) \
         UPDATE note_draft_part SET last_keystroke = last_keystroke - interval '10 min', \
                recording_started = recording_started - interval '10 min' \
          WHERE draft_id = $1::text::uuid",
        &[&draft.to_string()],
    )
    .await
    .unwrap();
    let header: DateTime<Utc> = c
        .query_one(
            "SELECT last_keystroke FROM note_draft WHERE draft_id = $1::text::uuid",
            &[&draft.to_string()],
        )
        .await
        .unwrap()
        .get(0);

    let read = open_drafts(&c, chart).await.unwrap().remove(0);

    assert_eq!(read.due, draft_due(header, 30));
    assert!(read.overdue, "the header says overdue");
    assert!(read.due < Utc::now(), "and so does the due time shown");
}
