//! Issue #216 / ADR-0058 — the grade-gated `t_effective` ceiling at the STRICT door
//! (db/005 `submit_event`). The born `clock_grade` (a mandatory `EventBody` field — a
//! conforming client can never omit it, a compile-time guarantee) tells the door how much
//! standing a clock has to call a forward-dated claim impossible: at unknown/self-asserted
//! (ranks 0-1 — the only grades any node currently mints) the ceiling is OPEN ABOVE, so a
//! forward `t_effective` is ADMITTED + FLAGGED, never rejected (principle 4 — a slow/dead
//! clock must not force fabrication). Only a credible high-grade clock can REJECT; no emit
//! path mints one this slice, so that arm is exercised here by synthesis.
//!
//! Real Postgres, gated on `$CAIRN_TEST_PG`, serialized cluster-wide via
//! `db::test_serial_guard` (shared-DB + TRUNCATE pattern — mirrors hlc_drift.rs). Keys are
//! derived at runtime via `generate_key()` (house rule 6), never a literal.
use cairn_event::{event_address, generate_key, sign, ClockGrade, EventBody, Hlc, SigningKey};
use cairn_node::db;
use tokio_postgres::Client;
use uuid::Uuid;

fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

/// A fixed, plausible HLC wall (ms since epoch — 2020-09-13T12:26:40Z) that every test in
/// this file measures its `t_effective` claims against. Fixed rather than "now" so the
/// paired ISO-8601 strings `iso8601_utc` produces are exact and the test is deterministic.
/// Comfortably in the real past, so it never trips the SEPARATE, unrelated local-door
/// clock-drift ceiling (issue #187, db/005 step 1a), which compares the wall against
/// `clock_timestamp()`, never against `t_effective`.
const WALL_MS: i64 = 1_600_000_000_000;

/// Civil (proleptic-Gregorian) date from days-since-epoch — Howard Hinnant's well-known
/// `civil_from_days` algorithm (public domain, exact for the whole `i64` range). We need
/// this small pure function because the crate carries no date/time-formatting dependency
/// (chrono/time) and `cairn_t_effective` (db/001) requires a literal ISO-8601 string —
/// this is the one bridge from "epoch ms" (what the HLC wall and our offsets are
/// expressed in) to that wire format.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

/// Render `ms_since_epoch` as the one ISO-8601 shape `cairn_t_effective` (db/001) accepts:
/// an explicit UTC "Z" offset. Whole-second precision only (every t_effective this suite
/// builds is a whole-second offset from `WALL_MS`).
fn iso8601_utc(ms_since_epoch: i64) -> String {
    let total_secs = ms_since_epoch.div_euclid(1000);
    let days = total_secs.div_euclid(86_400);
    let secs_of_day = total_secs.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    let hh = secs_of_day / 3600;
    let mm = (secs_of_day % 3600) / 60;
    let ss = secs_of_day % 60;
    format!("{y:04}-{m:02}-{d:02}T{hh:02}:{mm:02}:{ss:02}Z")
}

/// Truncate the tables this file's events touch (event_log for the submission itself,
/// patient_chart for note.added's projection, actor_event for the enrolled signer, and
/// t_effective_ceiling_flag so each test starts from a clean ceiling ledger), then enroll
/// one fresh agent signer. Mirrors hlc_drift.rs's `clinical_setup`.
async fn clinical_setup(c: &Client) -> (SigningKey, String) {
    c.batch_execute(
        "TRUNCATE event_log, actor_event, patient_chart, t_effective_ceiling_flag CASCADE",
    )
    .await
    .unwrap();
    let (sk, kid) = generate_key().unwrap();
    c.execute(
        "SELECT enroll_actor('agent', '{\"model\":\"sync-peer-stub\",\"version\":\"1\",\"skill_epoch\":\"e\"}', $1)",
        &[&kid],
    )
    .await
    .unwrap();
    (sk, kid)
}

/// Build a signed `note.added` `EventBody` whose `t_effective` sits `offset_secs` away
/// from the fixed `WALL_MS` ceiling, carrying `grade` as its born clock-confidence grade —
/// the one axis this suite drives.
fn forward_note(
    kid: &str,
    patient: Uuid,
    event_id: Uuid,
    grade: ClockGrade,
    offset_secs: i64,
) -> EventBody {
    EventBody {
        event_id: event_id.to_string(),
        patient_id: patient.to_string(),
        event_type: "note.added".into(),
        schema_version: "note/1".into(),
        hlc: Hlc {
            wall: WALL_MS,
            counter: 0,
            node_origin: "n".into(),
        },
        t_effective: Some(iso8601_utc(WALL_MS + offset_secs * 1000)),
        signer_key_id: kid.into(),
        contributors: serde_json::json!([{"actor_id": kid, "role": "recorded"}]),
        payload: serde_json::json!({"text": "ceiling probe"}),
        attachments: vec![],
        plaintext_twin: Some("Progress note: ceiling probe".into()),
        clock_grade: grade,
    }
}

/// Sign + submit a `forward_note` through the real strict door (`submit_event`) and return
/// the row identity `(client, event_id, content_address)` on success, or the raw
/// `tokio_postgres::Error` on refusal — the one shared implementation both
/// `submit_forward_effective` and `try_submit_forward_effective` wrap. `base` is the
/// `$CAIRN_TEST_PG` connection string; the caller holds its own `db::test_serial_guard`
/// for the test's full duration (mirrors every other DB-gated test file in this crate).
async fn try_submit_forward_effective(
    base: &str,
    grade: ClockGrade,
    offset_secs: i64,
) -> Result<(Client, Uuid, Vec<u8>), tokio_postgres::Error> {
    let c = db::connect_and_load_schema(base).await.unwrap();
    let (sk, kid) = clinical_setup(&c).await;

    let patient = Uuid::now_v7();
    let event_id = Uuid::now_v7();
    let body = forward_note(&kid, patient, event_id, grade, offset_secs);
    let signed = sign(&body, &sk).unwrap();
    let ca = event_address(&signed.signed_bytes);

    c.execute("SELECT submit_event($1)", &[&signed.signed_bytes])
        .await?;
    Ok((c, event_id, ca))
}

/// Convenience wrapper for the two admit-path tests: unwraps, since a submit failure there
/// is a test bug, not the behavior under test.
async fn submit_forward_effective(
    base: &str,
    grade: ClockGrade,
    offset_secs: i64,
) -> (Client, Uuid, Vec<u8>) {
    try_submit_forward_effective(base, grade, offset_secs)
        .await
        .expect("submit_event must admit this event (see the test body for the flag assertion)")
}

/// The Postgres error message text for a failed statement (tokio_postgres wraps a
/// DB-originated error as a generic "db error"; the real RAISE text is on the DbError).
fn db_msg(e: &tokio_postgres::Error) -> String {
    e.as_db_error()
        .map(|d| d.message().to_string())
        .unwrap_or_else(|| e.to_string())
}

async fn assert_event_present(c: &Client, event_id: Uuid) {
    let n: i64 = c
        .query_one(
            "SELECT count(*) FROM event_log WHERE event_id = $1::text::uuid",
            &[&event_id.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(n, 1, "the admitted event must be present in event_log");
}

/// The honest-slow-clock case (principle 4): a self-asserted clock has no standing to
/// call a forward-dated claim impossible, so it is admitted and recorded as an advisory
/// flag, never rejected.
#[tokio::test]
async fn self_asserted_forward_t_effective_is_admitted_and_flagged() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();

    let (client, event_id, ca) =
        submit_forward_effective(&base, ClockGrade::SelfAsserted, 3600).await; // hlc_wall + 1h
    assert_event_present(&client, event_id).await;
    let flags: i64 = client
        .query_one(
            "SELECT count(*) FROM t_effective_ceiling_flag WHERE content_address = $1",
            &[&ca],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        flags, 1,
        "self-asserted forward must be admitted + flagged, never rejected"
    );
}

/// Normal backdating (t_effective before hlc_wall) is the everyday case — clean, no
/// advisory flag.
#[tokio::test]
async fn clean_backdate_writes_no_flag() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();

    let (client, _event_id, ca) =
        submit_forward_effective(&base, ClockGrade::SelfAsserted, -3600).await; // hlc_wall - 1h
    let flags: i64 = client
        .query_one(
            "SELECT count(*) FROM t_effective_ceiling_flag WHERE content_address = $1",
            &[&ca],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(flags, 0, "a clean backdate must not be flagged");
}

/// The dormant reject arm, exercised by SYNTHESIS: a hardware-sourced clock genuinely
/// knows the time, so a t_effective far past its tight bound (W=5s) IS prima-facie
/// forward-dating. No emit path mints hardware-sourced this slice — the door still
/// classifies whatever grade the signed body carries.
#[tokio::test]
async fn high_grade_far_forward_is_rejected_at_the_strict_door() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();

    let err = try_submit_forward_effective(&base, ClockGrade::HardwareSourced, 3600) // +1h >> 5s
        .await
        .unwrap_err();
    let m = db_msg(&err);
    assert!(m.contains("grade-gated"), "got: {m}");
}
