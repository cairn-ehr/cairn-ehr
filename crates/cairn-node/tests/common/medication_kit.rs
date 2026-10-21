//! The medication-suite fixture: the node, the chart and the injection helpers that
//! `medication_withhold.rs`, `medication_withhold_fold.rs` and `medication_withhold_clock.rs`
//! need (#803).
//!
//! Include it with `#[path = "common/medication_kit.rs"] mod medication_kit;`. The including
//! binary must ALSO declare `mod common;` (charts are registered through
//! `common::submit_registration`, #345). `tests/medication.rs` keeps its own older copies.
#![allow(dead_code)] // each including suite uses a different subset

use crate::common;
use cairn_event::medication::HoldUntil;
use cairn_event::{generate_key, sign, EventBody, Hlc, SigningKey};
use cairn_node::medication::{
    assert_medication, cease_medication, withhold_medication, AssertMedicationInput,
    CeaseMedicationInput, WithholdMedicationInput,
};
use tokio_postgres::Client;
use uuid::Uuid;

pub fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

/// The Postgres RAISE text (a DB error's `Display` is just "db error").
pub fn db_msg(e: &tokio_postgres::Error) -> String {
    e.as_db_error()
        .map(|d| d.message().to_string())
        .unwrap_or_else(|| e.to_string())
}

/// Seal a CLEAR clinical body like the node write path, sign it, and submit it through the
/// strict door — for the one test that must hand-shape a payload the orchestrator would
/// never build. Copied from medication.rs (the DEK is generated, never a literal).
pub async fn seal_and_submit(
    c: &Client,
    sk: &SigningKey,
    mut body: EventBody,
) -> Result<u64, tokio_postgres::Error> {
    let twin = body
        .plaintext_twin
        .take()
        .expect("a clinical body carries its clear twin");
    let (container, dek) =
        cairn_event::seal::seal_event_payload(&body.payload, &twin, &body.event_id)
            .expect("seal the clear payload+twin");
    body.payload = container;
    body.plaintext_twin = Some(cairn_event::seal::seal_stub_twin(&body.event_type));
    let signed = sign(&body, sk).expect("sign the sealed body");
    c.execute(
        "SELECT submit_event($1, NULL, NULL, $2)",
        &[&signed.signed_bytes, &dek.as_bytes().as_slice()],
    )
    .await
}

/// Truncate the log, the medication projections this suite reads, and the custody plane;
/// enroll a device actor and provision its unwrap key (see medication.rs's `setup_node`
/// for why each step is needed).
pub async fn setup_node(c: &Client) -> (SigningKey, String) {
    c.batch_execute(
        "TRUNCATE event_log, actor_event, patient_chart, \
         node_unwrap_key, event_dek, event_clear, erasure_shred_log CASCADE",
    )
    .await
    .unwrap();
    c.batch_execute(
        "DO $$ BEGIN \
           IF to_regclass('public.medication_statement') IS NOT NULL THEN TRUNCATE medication_statement; END IF; \
           IF to_regclass('public.medication_cessation') IS NOT NULL THEN TRUNCATE medication_cessation; END IF; \
           IF to_regclass('public.medication_coding') IS NOT NULL THEN TRUNCATE medication_coding; END IF; \
           IF to_regclass('public.medication_withhold') IS NOT NULL THEN TRUNCATE medication_withhold; END IF; \
           IF to_regclass('public.medication_withhold_release') IS NOT NULL THEN TRUNCATE medication_withhold_release; END IF; \
         END $$;",
    )
    .await
    .unwrap();
    let (sk, kid) = generate_key().unwrap();
    c.execute(
        "SELECT enroll_actor('device', '{\"role\":\"registration-desk\"}', $1)",
        &[&kid],
    )
    .await
    .unwrap();
    let unwrap = cairn_event::seal::generate_unwrap_secret().unwrap();
    c.execute(
        "SELECT cairn_register_unwrap_key($1)",
        &[&cairn_event::seal::unwrap_public(&unwrap)
            .as_bytes()
            .as_slice()],
    )
    .await
    .unwrap();
    (sk, kid)
}

pub fn sample_input() -> AssertMedicationInput<'static> {
    AssertMedicationInput {
        term: "metoprolol",
        coding: None,
        formulation: Some("tablet"),
        dose_amount: Some("25"),
        dose_unit: Some("mg"),
        sig: Some("one BD"),
        info_source: "patient-reported",
        started: Some("2024"),
        started_precision: Some("year"),
    }
}

/// A hold "until reviewed", starting now — the commonest bedside W.
pub fn until_reviewed(reason: &str) -> WithholdMedicationInput<'_> {
    WithholdMedicationInput {
        reason,
        from: None,
        until: HoldUntil::Reviewed,
    }
}

/// Assert one more medication on an existing chart; returns its thread id.
pub async fn add_med(c: &mut Client, sk: &SigningKey, kid: &str, patient: Uuid) -> Uuid {
    assert_medication(
        c,
        sk,
        kid,
        "test-node",
        patient,
        &sample_input(),
        None,
        None,
    )
    .await
    .unwrap()
}

/// A fresh registered chart with one asserted medication. Returns (patient, thread).
pub async fn chart_with_one_med(c: &mut Client, sk: &SigningKey, kid: &str) -> (Uuid, Uuid) {
    let patient = Uuid::now_v7();
    common::submit_registration(c, sk, kid, patient, 0).await;
    let med = add_med(c, sk, kid, patient).await;
    (patient, med)
}

/// Cease a thread (optionally with a stop date) through the orchestrator.
pub async fn cease(
    c: &mut Client,
    sk: &SigningKey,
    kid: &str,
    p: Uuid,
    med: Uuid,
    stopped: Option<&str>,
) {
    let input = CeaseMedicationInput {
        stopped,
        stopped_precision: stopped.map(|_| "year"),
        reason: None,
    };
    cease_medication(c, sk, kid, "test-node", p, med, &input, None, None)
        .await
        .unwrap();
}

/// Withhold through the orchestrator ("until reviewed"); returns the withhold's event id.
pub async fn withhold(
    c: &mut Client,
    sk: &SigningKey,
    kid: &str,
    p: Uuid,
    med: Uuid,
    reason: &str,
) -> Uuid {
    withhold_medication(
        c,
        sk,
        kid,
        "test-node",
        p,
        med,
        &until_reviewed(reason),
        None,
    )
    .await
    .expect("the withhold is admitted")
}

/// A peer-minted HLC at `wall` (ms) — for events that arrive through `apply_remote_event`.
pub fn peer_hlc(wall: i64) -> Hlc {
    Hlc {
        wall,
        counter: 0,
        node_origin: "peer".into(),
    }
}

/// Give a hand-built withhold or release the `t_effective` an orchestrator would stamp: here,
/// the body's own HLC wall as an instant (never after it, so no ceiling anomaly). The floor
/// refuses a withhold without `from`, and any release, that carries none (db/059 — the HLC
/// wall is no stand-in), so a test that builds one by hand must say when it took effect.
/// Not for a hold that must be in force at the REAL now: earlier tests in a binary push the
/// shared HLC ahead of the physical clock, so this instant can be a moment in the future.
pub async fn effective_at_wall(c: &Client, mut body: EventBody) -> EventBody {
    let at: String = c
        .query_one(
            "SELECT to_char(to_timestamp($1::bigint / 1000.0) AT TIME ZONE 'UTC',                             'YYYY-MM-DD\"T\"HH24:MI:SS.MS\"Z\"')",
            &[&body.hlc.wall],
        )
        .await
        .unwrap()
        .get(0);
    body.t_effective = Some(at);
    body
}

/// Sign a CLEAR body and apply it through the sync door (`apply_remote_event`).
pub async fn apply_remote(
    c: &Client,
    sk: &SigningKey,
    body: &EventBody,
) -> Result<u64, tokio_postgres::Error> {
    c.execute(
        "SELECT apply_remote_event($1)",
        &[&sign(body, sk).unwrap().signed_bytes],
    )
    .await
}
