//! Shared scaffolding for the cairn-node identity integration tests (#120).
//!
//! Every identity suite needs the same four things before it can assert anything: a
//! connection string from the environment, a clean database with one enrolled signer, a
//! way to sign-and-submit an event through the real `submit_event` door, and a way to read
//! the resulting projection rows back. Those helpers used to be copy-pasted into each
//! suite — `identity_identify.rs` was the third copy — and the copies had already drifted.
//! They live here once instead.
//!
//! **How Cargo sees this file.** Cargo compiles every top-level `tests/*.rs` as its own
//! test binary, but treats a SUBdirectory like `tests/common/` as an ordinary module
//! directory. So this is not a test binary; each suite pulls it in with `mod common;`.
//! That is the standard Rust idiom for shared integration-test code.
//!
//! **What belongs here.** Only helpers that are generic across identity suites. A suite's
//! own event builders (`submit_link`, `open_dispute`, `mark_pending`, …) stay in the suite,
//! where their canned clinical strings are readable next to the assertions that depend on
//! them. The line is: if two suites would write it identically, it goes here.
//!
//! `tests/identity_scaffolding_shared.rs` enforces adoption — but only of the helpers that
//! are specific to this cluster: `submit_signed`, `submit_patient_created`, `trust_of`,
//! `person_chart_trust`. It deliberately does NOT bind `cs` / `db_msg` / `setup`, which are
//! project-wide test idioms declared in dozens of this directory's files; see that file's
//! `REPO_WIDE` const for why, and #327 for unifying them. So a suite re-declaring its own
//! `setup` is caught by review, not by the guard.

// Each test binary uses only the subset of helpers it needs, so from the perspective of
// any single binary the rest are dead. Without this, every suite would warn about the
// helpers the OTHER suites use. Scoped to this module, so it never masks dead code in a
// test suite itself.
#![allow(dead_code)]

use cairn_event::{generate_key, sign, ClockGrade, EventBody, Hlc, SigningKey};
use tokio_postgres::Client;
use uuid::Uuid;

/// The test Postgres connection string, or `None` when the suite should self-skip.
///
/// Every DB-gated test opens with `let Some(base) = cs() else { return };` — absent
/// `$CAIRN_TEST_PG` the suite quietly passes, so a plain `cargo test` on a machine with no
/// database still works. Two things set it so the tests actually run: CI declares it as a
/// workflow-step `env:` entry (`.github/workflows/rust.yml`), and locally
/// `scripts/run-db-gated-tests.sh` bakes it in.
pub fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

/// The Postgres error message text for a failed statement.
///
/// `tokio_postgres::Error`'s `Display` renders only the literal "db error"; the actual
/// `RAISE EXCEPTION` message from the in-DB floor lives in the `DbError` payload. Every
/// floor-rejection assertion must therefore go through here, or it asserts against a
/// constant string and passes for the wrong reason.
pub fn db_msg(e: &tokio_postgres::Error) -> String {
    e.as_db_error()
        .map(|d| d.message().to_string())
        .unwrap_or_else(|| e.to_string())
}

/// Truncate the clinical tables, truncate the caller's identity-overlay tables, and enroll
/// one agent signer. Returns `(signing key, key id)`.
///
/// `extra_tables` names the projection/overlay tables this suite writes to — e.g.
/// `&["patient_link", "person_member"]` for linkage, `&["chart_dispute"]` for disputes.
/// They are truncated behind a `to_regclass` guard because each is created by a LATER
/// migration than the core clinical tables: the guard keeps one shared `setup()` correct
/// on a database migrated only partway, instead of erroring on a table that does not exist
/// yet. Hand-maintaining that `DO $$` block per copy is exactly the drift #120 is about,
/// so it is generated from the list here.
pub async fn setup(c: &Client, extra_tables: &[&str]) -> (SigningKey, String) {
    c.batch_execute(
        "TRUNCATE event_log, actor_event, patient_chart, patient_identifier, \
         patient_demographic CASCADE",
    )
    .await
    .unwrap();

    if !extra_tables.is_empty() {
        // Table names are interpolated (an identifier cannot be a bind parameter), so they
        // are asserted to be bare lower-case identifiers first. These are compile-time
        // literals from test code rather than input, but the check costs nothing and keeps
        // the generated SQL obviously safe to a reader.
        let mut sql = String::from("DO $$ BEGIN ");
        for t in extra_tables {
            assert!(
                !t.is_empty()
                    && t.bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_'),
                "extra table name must be a bare lower-case identifier, got {t:?}"
            );
            sql.push_str(&format!(
                "IF to_regclass('public.{t}') IS NOT NULL THEN TRUNCATE {t}; END IF; "
            ));
        }
        sql.push_str("END $$;");
        c.batch_execute(&sql).await.unwrap();
    }

    let (sk, kid) = generate_key().unwrap();
    c.execute(
        "SELECT enroll_actor('agent', '{\"model\":\"reg-stub\",\"version\":\"1\",\"skill_epoch\":\"e\"}', $1)",
        &[&kid],
    )
    .await
    .unwrap();
    (sk, kid)
}

/// The parts of an event that actually vary between identity test cases.
///
/// A named-field struct rather than a long positional argument list: at a call site
/// `event_type: "identity.dispute.asserted"` reads as itself, where the eighth positional
/// `&str` would not. Everything an identity test never varies — the UUIDv7 event id, the
/// single-contributor set, the absent `t_effective`, the empty attachments — is filled in
/// by [`submit_signed`] rather than repeated per call.
pub struct EventSpec<'a> {
    /// The chart this event is "about". Identity assertions use their subject's UUID.
    pub patient: Uuid,
    pub event_type: &'a str,
    pub schema_version: &'a str,
    /// The event body content, which becomes the DB `body`.
    pub payload: serde_json::Value,
    /// The §3.13 legibility twin. `Some` for types with a renderer (identity assertions,
    /// where the in-DB floor requires it); `None` for types where `submit_event` still
    /// derives the honest-degrade skeleton itself (db/015).
    pub plaintext_twin: Option<String>,
    /// HLC wall clock — higher is newer. How a test orders overlays deterministically
    /// without sleeping on a real clock.
    pub wall: i64,
}

/// Sign `spec` and submit it through the real `submit_event` door.
///
/// Returns the raw submit result — NOT unwrapped — because about a third of these tests
/// assert a *rejection* and match on [`db_msg`] of the error. A helper that unwrapped here
/// would make the floor's rejections untestable.
pub async fn submit_signed(
    c: &Client,
    sk: &SigningKey,
    kid: &str,
    spec: EventSpec<'_>,
) -> Result<u64, tokio_postgres::Error> {
    let body = EventBody {
        event_id: Uuid::now_v7().to_string(),
        patient_id: spec.patient.to_string(),
        event_type: spec.event_type.into(),
        schema_version: spec.schema_version.into(),
        hlc: Hlc {
            wall: spec.wall,
            counter: 0,
            node_origin: "n".into(),
        },
        t_effective: None,
        signer_key_id: kid.into(),
        contributors: serde_json::json!([{"actor_id": kid, "role": "recorded"}]),
        payload: spec.payload,
        attachments: vec![],
        plaintext_twin: spec.plaintext_twin,
        clock_grade: ClockGrade::SelfAsserted,
    };
    let signed = sign(&body, sk).unwrap();
    c.execute("SELECT submit_event($1)", &[&signed.signed_bytes])
        .await
}

/// Submit a minimal `patient.created` so a subject has a `patient_chart` row.
///
/// Several projections (`person_chart`, and the trust reads composed on top of it) are
/// chart reads: they list a subject only once its chart exists. A test that wants to
/// observe a subject through one of those must create the chart first. Unwrapped, because
/// this is always setup for the real assertion, never the thing under test.
pub async fn submit_patient_created(c: &Client, sk: &SigningKey, kid: &str, p: Uuid, wall: i64) {
    submit_signed(
        c,
        sk,
        kid,
        EventSpec {
            patient: p,
            event_type: "patient.created",
            schema_version: "patient/1",
            payload: serde_json::json!({"name": "T", "dob": "1990", "sex": "x"}),
            plaintext_twin: None, // non-demographic type → honest-degrade skeleton (db/015)
            wall,
        },
    )
    .await
    .expect("patient.created accepted");
}

/// The effective trust state `chart_trust` reports for a subject, or `None` (== confirmed).
///
/// `chart_trust` is the authoritative pre-sync safety signal: it answers for a chart
/// whether or not that chart has replicated anywhere.
pub async fn trust_of(c: &Client, subject: Uuid) -> Option<String> {
    let s_s = subject.to_string();
    c.query_opt(
        "SELECT trust_state FROM chart_trust WHERE patient_id = $1::text::uuid",
        &[&s_s],
    )
    .await
    .unwrap()
    .map(|r| r.get::<_, String>(0))
}

/// `person_chart_trust.trust_state` for a subject's chart row, or `None`.
///
/// The unified read composed on top of the `person_chart` linkage view — so unlike
/// [`trust_of`] it surfaces a subject only once that subject has a chart. Tests assert
/// against both to pin the difference.
pub async fn person_chart_trust(c: &Client, subject: Uuid) -> Option<String> {
    let s_s = subject.to_string();
    c.query_opt(
        "SELECT trust_state FROM person_chart_trust WHERE patient_id = $1::text::uuid",
        &[&s_s],
    )
    .await
    .unwrap()
    .map(|r| r.get::<_, String>(0))
}
