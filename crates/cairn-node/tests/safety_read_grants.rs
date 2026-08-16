//! §5.9 part B (ADR-0063) — the GRANT floor under the safety read model (#405 part 1).
//!
//! # What this file is for
//!
//! `db/049` says the three read functions are the sanctioned way to read the safety
//! signal, "and a reader that reaches `event_log.safety` directly gets the UNCOARSENED
//! bytes". Until this slice that sentence was aspirational: `db/005` does
//! `GRANT SELECT ON event_log … TO cairn_agent`, **a table-level grant covers every column
//! added later**, so the runtime role could simply
//!
//! ```sql
//! SELECT safety FROM event_log WHERE event_id = …
//! ```
//!
//! and read the emitted rung and class raw — skipping section 7's re-coarsening entirely.
//! That defeats exactly the case read coarsening exists for: an honest peer emits
//! `precise` because the chart was routine on ITS node, while this node holds a
//! `restricted` grade (the grade is node-relative, ADR-0062 decision 9).
//!
//! Principle 12 says the floor must be unbypassable **in the database**, so the fix is a
//! privilege, not a convention: `event_log`'s table-level SELECT grant to `cairn_agent` is
//! replaced by an explicit column list that omits `safety`, and the two read functions that
//! must still reach the column became `SECURITY DEFINER`.
//!
//! # Why a whole file rather than three more tests in `safety_read.rs`
//!
//! That file is already 890 lines and #402 asks for it to be split along the three
//! properties its header names. These tests are about PRIVILEGE, not about the read
//! model's semantics, and they are the only tests in the repo that must keep working when
//! `event_log` gains a column — so they are easier to find here.
//!
//! Every test self-skips without `$CAIRN_TEST_PG` (`cs()` returns `None`), and cargo then
//! reports the suite as passing while running nothing — a green run that prints no test
//! names is a SKIP, not a pass.
mod common;
use common::{cs, setup};
use uuid::Uuid;

/// Postgres's `insufficient_privilege`. Asserted by SQLSTATE, never by message text: the
/// message is localised and reworded across major versions, the code is the contract.
const INSUFFICIENT_PRIVILEGE: &str = "42501";

/// Every `event_log` column `cairn_agent` is deliberately allowed to read, in `attnum`
/// order so a reader can diff it against `\d event_log` by eye.
///
/// **This list is the decision, and adding a column to `event_log` must move it.** A
/// column-level grant does not extend to columns added later, which is the fail-CLOSED
/// direction chosen on purpose: a new column is unreadable by the runtime role until
/// someone writes it down here and in `db/049`'s grant block, rather than becoming
/// world-readable by inheriting a table-level grant the way `safety` did (#405 part 1).
const GRANTED_COLUMNS: [&str; 23] = [
    "event_id",
    "patient_id",
    "event_type",
    "schema_version",
    "hlc_wall",
    "hlc_counter",
    "node_origin",
    "t_effective",
    "signed_bytes",
    "content_address",
    "body",
    "contributors",
    "signer_key_id",
    "plaintext_twin",
    "sealed",
    "dek_wrapped",
    "attachments",
    "recorded_at",
    "attestation",
    "attester_key",
    "actor_id",
    "clock_grade",
    "seq",
];

/// Every `event_log` column deliberately WITHHELD from `cairn_agent`.
///
/// `safety` is the first *clear*, grade-sensitive column on `event_log` — before it a raw
/// `SELECT` yielded ciphertext (`body` is sealed, ADR-0052) or envelope metadata — so this
/// side channel is new, and so is the need for a list at all.
const WITHHELD_COLUMNS: [&str; 1] = ["safety"];

/// Build one chart carrying: a `restricted` chart-wide grade and a `note.added` whose
/// emitted safety signal is the finest rung there is. Returns the event id.
///
/// The pairing is the whole point — emission says `precise`, the local grade licenses only
/// `existence`, so a correct reader must coarsen and a raw column read must not be
/// possible. Both fixtures go through `apply_remote_event` for the reason `safety_read.rs`
/// documents: the local door would refuse this shape, and "already on disk" is precisely
/// the situation the read model exists for.
async fn chart_with_an_overclaiming_note(
    c: &tokio_postgres::Client,
    sk: &cairn_event::SigningKey,
    kid: &str,
    patient: Uuid,
) -> Uuid {
    use cairn_event::sensitivity::{
        SubjectKind, SENSITIVITY_EVENT_TYPE, SENSITIVITY_SCHEMA_VERSION,
    };

    let a = cairn_event::sensitivity::SensitivityAssertion {
        subject_kind: SubjectKind::Patient,
        subject_id: patient,
        grade: "restricted",
        source: "human",
        rationale: Some("test fixture"),
    };
    let grade_body = cairn_event::EventBody {
        event_id: Uuid::now_v7().to_string(),
        patient_id: patient.to_string(),
        event_type: SENSITIVITY_EVENT_TYPE.into(),
        schema_version: SENSITIVITY_SCHEMA_VERSION.into(),
        hlc: cairn_event::Hlc {
            wall: 10,
            counter: 0,
            node_origin: "n1".into(),
        },
        t_effective: None,
        signer_key_id: kid.into(),
        contributors: serde_json::json!([{"actor_id": kid, "role": "recorded"}]),
        payload: cairn_event::sensitivity::sensitivity_assertion_body(&a),
        attachments: vec![],
        plaintext_twin: Some(cairn_event::sensitivity::render_sensitivity_twin(&a)),
        clock_grade: cairn_event::ClockGrade::SelfAsserted,
        safety: None,
    };
    let signed = cairn_event::sign(&grade_body, sk).expect("signs");
    c.execute("SELECT apply_remote_event($1)", &[&signed.signed_bytes])
        .await
        .expect("grade applied");

    let note = cairn_event::EventBody {
        event_id: Uuid::now_v7().to_string(),
        patient_id: patient.to_string(),
        event_type: "note.added".into(),
        schema_version: "note/1".into(),
        hlc: cairn_event::Hlc {
            wall: 20,
            counter: 0,
            node_origin: "n1".into(),
        },
        t_effective: None,
        signer_key_id: kid.into(),
        contributors: serde_json::json!([{"actor_id": kid, "role": "recorded"}]),
        payload: serde_json::json!({"text": "a note"}),
        attachments: vec![],
        plaintext_twin: Some("a note".into()),
        clock_grade: cairn_event::ClockGrade::SelfAsserted,
        safety: Some(
            serde_json::json!({"rung": "precise", "class": "rh-sensitizing", "severity": "high"}),
        ),
    };
    let id: Uuid = note.event_id.parse().expect("uuid");
    let signed = cairn_event::sign(&note, sk).expect("signs");
    c.execute("SELECT apply_remote_event($1)", &[&signed.signed_bytes])
        .await
        .expect("note admitted");
    id
}

/// The floor itself: the runtime role holds no SELECT privilege on `event_log.safety`,
/// and an actual raw read under that role is refused by Postgres.
///
/// Both halves are needed. `has_column_privilege` alone would pass against a database
/// where the grant block never ran (it reports the catalogue, and a missing role would be
/// an error rather than a false); the executed `SELECT` proves the privilege is the thing
/// standing in the way, at the exact SQL `db/049`'s header names.
#[tokio::test]
async fn cairn_agent_cannot_read_the_safety_column_raw() {
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();
    let (sk, kid) = setup(&c, &["sensitivity_assertion", "sensitivity_withdrawal"]).await;
    let p = Uuid::now_v7();
    let ev = chart_with_an_overclaiming_note(&c, &sk, &kid, p).await;

    let has: bool = c
        .query_one(
            "SELECT has_column_privilege('cairn_agent', 'event_log', 'safety', 'SELECT')",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert!(
        !has,
        "cairn_agent must hold no SELECT privilege on event_log.safety — a table-level \
         grant covers columns added later, which is how #405 part 1 happened"
    );

    c.batch_execute("SET ROLE cairn_agent").await.unwrap();
    let denied = c
        .query_one(
            "SELECT safety::text FROM event_log WHERE event_id = $1::text::uuid",
            &[&ev.to_string()],
        )
        .await
        .expect_err("the raw column read must be refused for cairn_agent");
    c.batch_execute("RESET ROLE").await.unwrap();
    assert_eq!(
        denied.as_db_error().map(|e| e.code().code()),
        Some(INSUFFICIENT_PRIVILEGE),
        "the refusal must be a privilege refusal, not some other error: {denied}"
    );
}

/// The other half of the same floor, and the one that makes it safe to ship: the
/// SANCTIONED read still works under the runtime role, and it returns the COARSENED
/// answer.
///
/// This is the strong pin (the `claim_authority.rs` distinction — a role-switched test
/// that never lands real data through the role pins only Postgres's executor-start ACL
/// check). Here the fixture carries a real signal and a real standing grade, so the
/// function bodies actually execute under `cairn_agent` against live rows: without
/// `SECURITY DEFINER` on both functions this fails with `42501` the moment either touches
/// `event_log.safety`.
#[tokio::test]
async fn the_sanctioned_read_still_works_as_cairn_agent_and_coarsens() {
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();
    let (sk, kid) = setup(&c, &["sensitivity_assertion", "sensitivity_withdrawal"]).await;
    let p = Uuid::now_v7();
    let ev = chart_with_an_overclaiming_note(&c, &sk, &kid, p).await;

    c.batch_execute("SET ROLE cairn_agent").await.unwrap();
    let row = c
        .query_one(
            "SELECT rung, class, severity FROM cairn_event_safety($1::text::uuid)",
            &[&ev.to_string()],
        )
        .await
        .expect("cairn_agent must be able to read the coarsened signal");
    let (rung, class, severity): (String, Option<String>, Option<String>) =
        (row.get(0), row.get(1), row.get(2));

    // The chart-wide report reads the same rows through the same functions; it needs the
    // privilege independently (its own WHERE touches `safety`), so a fix that only made
    // cairn_event_safety a definer would pass the assertion above and fail here.
    let chart = c
        .query(
            "SELECT rung, class FROM cairn_patient_safety($1::text::uuid)",
            &[&p.to_string()],
        )
        .await
        .expect("cairn_agent must be able to read the chart-wide report");
    c.batch_execute("RESET ROLE").await.unwrap();

    // 'restricted' ranks 20 → licensed rung 'existence'; the emitted 'precise' loses.
    assert_eq!(
        rung, "existence",
        "the local grade must win over the emission"
    );
    assert_eq!(class, None, "a class must never survive coarsening");
    assert_eq!(severity, None, "at 'existence' the severity goes too");
    assert_eq!(chart.len(), 1, "the note is the chart's only signal");
    assert_eq!(chart[0].get::<_, String>(0), "existence");
    assert_eq!(chart[0].get::<_, Option<String>>(1), None);
}

/// The ledger: every column of `event_log` is either deliberately granted to `cairn_agent`
/// or deliberately withheld, and this test names both sets.
///
/// It exists so a future `ALTER TABLE event_log ADD COLUMN` cannot quietly decide the
/// question. Adding a column makes this test fail with the column's name, and the person
/// adding it then chooses: grant it in `db/049`'s block and list it here, or withhold it
/// and say why. That is the same "force the decision at the moment of the change"
/// discipline as the twin-registry row-count pins.
#[tokio::test]
async fn every_event_log_column_is_a_deliberate_grant_decision() {
    let Some(base) = cs() else { return };
    let _guard = cairn_node::db::test_serial_guard(&base).await.unwrap();
    let c = cairn_node::db::connect_and_load_schema(&base)
        .await
        .unwrap();

    let rows = c
        .query(
            "SELECT attname::text,
                    has_column_privilege('cairn_agent', 'event_log', attname, 'SELECT')
               FROM pg_attribute
              WHERE attrelid = 'event_log'::regclass AND attnum > 0 AND NOT attisdropped
              ORDER BY attnum",
            &[],
        )
        .await
        .unwrap();

    let mut unaccounted: Vec<String> = Vec::new();
    for r in &rows {
        let (name, readable): (String, bool) = (r.get(0), r.get(1));
        let expected_readable = if WITHHELD_COLUMNS.contains(&name.as_str()) {
            false
        } else if GRANTED_COLUMNS.contains(&name.as_str()) {
            true
        } else {
            unaccounted.push(name.clone());
            continue;
        };
        assert_eq!(
            readable, expected_readable,
            "event_log.{name}: cairn_agent's SELECT privilege disagrees with this file's \
             declared decision (expected readable = {expected_readable})"
        );
    }
    assert!(
        unaccounted.is_empty(),
        "event_log gained {unaccounted:?} and no one decided whether cairn_agent may read \
         it. Grant it in db/049's column-grant block and add it to GRANTED_COLUMNS, or \
         withhold it and add it to WITHHELD_COLUMNS with a reason."
    );
}
