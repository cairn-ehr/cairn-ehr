//! ADR-0081 (#803) decision 7 + ADR-0005/0052 — where a withhold meets the pieces that
//! enumerate medication event types: sign-off currency (ADR-0049), shred, and thread
//! resolution.
//!
//! * A withhold or release is CONTENT on the thread, so it enters the sign-off commitment:
//!   withholding after a sign-off makes the line stale (the signer vouched for a line that
//!   was not on hold).
//! * EXPIRY writes no event, and the commitment takes no clock input, so a hold lapsing can
//!   never stale a sign-off. Pinned both by behaviour (a hold that lapses within seconds)
//!   and by a source check.
//! * A shred of a withhold scrubs its projection row (the reason is clinical plaintext).
//! * `cairn_event_thread` resolves a withhold and a release to their line.
//!
//! DB-gated on `$CAIRN_TEST_PG`, serialized via `db::test_serial_guard`.
use cairn_event::medication::HoldUntil;
use cairn_event::{generate_key, SigningKey};
use cairn_node::db;
use cairn_node::medication::{
    attest_medication_thread, release_medication_holds, withhold_medication, AttestParams,
    WithholdMedicationInput,
};
use cairn_node::shred::shred_event;
use tokio_postgres::Client;
use uuid::Uuid;

mod common;
#[path = "common/medication_kit.rs"]
mod medication_kit;
use medication_kit::*;

/// Enroll a human actor (the sign-off signer) and return its key.
async fn enroll_human(c: &Client) -> (SigningKey, String) {
    let (sk, kid) = generate_key().unwrap();
    c.execute(
        "SELECT enroll_actor('human', '{\"role\":\"clinician\"}', $1)",
        &[&kid],
    )
    .await
    .unwrap();
    (sk, kid)
}

/// Sign off the thread through the orchestrator (the human vouches for its current content).
async fn sign_off(
    c: &mut Client,
    node: &SigningKey,
    human: &(SigningKey, String),
    p: Uuid,
    med: Uuid,
) {
    let params = AttestParams {
        human_sk: &human.0,
        human_kid: &human.1,
        basis: None,
        note: None,
    };
    attest_medication_thread(c, node, "test-node", &params, p, med)
        .await
        .expect("the sign-off is admitted");
}

async fn stale(c: &Client, med: Uuid) -> bool {
    c.query_one(
        "SELECT stale FROM medication_thread_attestation WHERE medication_id = $1::text::uuid",
        &[&med.to_string()],
    )
    .await
    .unwrap()
    .get(0)
}

#[tokio::test]
async fn a_withhold_after_sign_off_makes_the_line_stale() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let human = enroll_human(&c).await;
    let (p, med) = chart_with_one_med(&mut c, &sk, &kid).await;

    sign_off(&mut c, &sk, &human, p, med).await;
    assert!(!stale(&c, med).await, "freshly signed off is not stale");

    withhold(&mut c, &sk, &kid, p, med, "BP 85/50").await;
    assert!(
        stale(&c, med).await,
        "a withhold after sign-off stales the line"
    );
}

#[tokio::test]
async fn a_release_after_sign_off_makes_the_line_stale() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let human = enroll_human(&c).await;
    let (p, med) = chart_with_one_med(&mut c, &sk, &kid).await;
    let w = withhold(&mut c, &sk, &kid, p, med, "BP 85/50").await;

    sign_off(&mut c, &sk, &human, p, med).await;
    assert!(!stale(&c, med).await);

    release_medication_holds(&mut c, &sk, &kid, "test-node", p, med, &[w], None, None)
        .await
        .expect("the release is admitted");
    assert!(
        stale(&c, med).await,
        "a release after sign-off stales the line"
    );
}

/// How many holds the fold returns for `med`'s line at the real clock.
async fn holds_now(c: &Client, med: Uuid) -> i64 {
    c.query_one(
        "SELECT count(*) FROM cairn_medication_group_holds(\
           ARRAY(SELECT group_id FROM medication_thread_group \
                  WHERE medication_id = $1::text::uuid), clock_timestamp())",
        &[&med.to_string()],
    )
    .await
    .unwrap()
    .get(0)
}

#[tokio::test]
async fn expiry_does_not_stale_a_sign_off() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let human = enroll_human(&c).await;
    let (p, med) = chart_with_one_med(&mut c, &sk, &kid).await;

    // A hold that lapses within seconds; sign off while it stands, then wait it out.
    // (clock_timestamp, not now(): now() is the transaction's start, on a pooled connection
    // possibly long past.)
    let until: String = c
        .query_one(
            "SELECT to_char((clock_timestamp() + interval '4 seconds') AT TIME ZONE 'UTC', \
                            'YYYY-MM-DD\"T\"HH24:MI:SS.MS\"Z\"')",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    let input = WithholdMedicationInput {
        reason: "NBM for theatre",
        from: None,
        until: HoldUntil::At(&until),
    };
    withhold_medication(&mut c, &sk, &kid, "test-node", p, med, &input, None)
        .await
        .expect("the timed withhold is admitted");
    sign_off(&mut c, &sk, &human, p, med).await;
    assert!(!stale(&c, med).await);

    // The behaviour: once `until` has passed the hold is gone from the fold, and the
    // sign-off is exactly as current as before.
    assert_eq!(holds_now(&c, med).await, 1, "the hold stands at sign-off");
    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    assert_eq!(holds_now(&c, med).await, 0, "the hold has lapsed");
    assert!(
        !stale(&c, med).await,
        "a lapsed hold does not stale the sign-off"
    );

    // And the structure behind it: expiry writes no event, so nothing in the commitment can
    // move when it passes — neither the commitment, the readable count nor the staleness
    // view reads a clock.
    let sources: Vec<String> = c
        .query(
            "SELECT pg_get_viewdef('medication_thread_attestation'::regclass) \
             UNION ALL SELECT prosrc FROM pg_proc \
              WHERE proname IN ('cairn_medication_thread_commitment', \
                                'cairn_medication_thread_readable_count')",
            &[],
        )
        .await
        .unwrap()
        .iter()
        .map(|r| r.get(0))
        .collect();
    assert_eq!(sources.len(), 3, "the view and both fns were found");
    for s in &sources {
        assert!(
            ![
                "now()",
                "clock_timestamp",
                "current_timestamp",
                "transaction_timestamp",
                "statement_timestamp",
                "localtimestamp",
            ]
            .iter()
            .any(|clock| s.to_lowercase().contains(clock)),
            "expiry must not be able to reach staleness: {s}"
        );
    }
}

#[tokio::test]
async fn a_plain_threads_commitment_is_the_hash_of_its_one_assert() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let (_p, med) = chart_with_one_med(&mut c, &sk, &kid).await;

    // The commitment is sha256 of the sorted concatenated content addresses; with one
    // content event that is sha256(that assert's content_address) — unchanged by db/059.
    let same: bool = c
        .query_one(
            "SELECT cairn_medication_thread_commitment($1::text::uuid) = \
                    (SELECT digest(content_address, 'sha256') FROM event_log \
                      WHERE event_type = 'clinical.medication.asserted' \
                        AND (cairn_clear_payload(event_log) ->> 'medication_id')::uuid = $1::text::uuid)",
            &[&med.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert!(same);
}

#[tokio::test]
async fn shredding_a_withhold_removes_its_row() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let (p, med) = chart_with_one_med(&mut c, &sk, &kid).await;
    let w = withhold(&mut c, &sk, &kid, p, med, "BP 85/50").await;
    let r = release_ids(&mut c, &sk, &kid, p, med, w).await;

    assert_eq!(rows_for(&c, "medication_withhold", w).await, 1);
    assert_eq!(rows_for(&c, "medication_withhold_release", r).await, 1);

    for target in [w, r] {
        shred_event(
            &mut c,
            &sk,
            &kid,
            "test-node",
            target,
            "retention ceiling",
            None,
        )
        .await
        .expect("the shred is admitted");
    }
    assert_eq!(
        rows_for(&c, "medication_withhold", w).await,
        0,
        "withhold row scrubbed"
    );
    assert_eq!(
        rows_for(&c, "medication_withhold_release", r).await,
        0,
        "release row scrubbed"
    );
}

/// Rows in a projection `table` for one event id.
async fn rows_for(c: &Client, table: &str, id: Uuid) -> i64 {
    let sql = format!("SELECT count(*) FROM {table} WHERE event_id = $1::text::uuid");
    c.query_one(&sql, &[&id.to_string()]).await.unwrap().get(0)
}

/// Release one withhold; returns the release event's id.
async fn release_ids(
    c: &mut Client,
    sk: &SigningKey,
    kid: &str,
    p: Uuid,
    med: Uuid,
    w: Uuid,
) -> Uuid {
    release_medication_holds(c, sk, kid, "test-node", p, med, &[w], None, None)
        .await
        .expect("the release is admitted")
}

#[tokio::test]
async fn cairn_event_thread_resolves_a_withhold_and_a_release() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid) = setup_node(&c).await;
    let (p, med) = chart_with_one_med(&mut c, &sk, &kid).await;
    let w = withhold(&mut c, &sk, &kid, p, med, "BP 85/50").await;
    let r = release_ids(&mut c, &sk, &kid, p, med, w).await;

    for event in [w, r] {
        let thread: Option<String> = c
            .query_one(
                "SELECT cairn_event_thread($1::text::uuid)::text",
                &[&event.to_string()],
            )
            .await
            .unwrap()
            .get(0);
        assert_eq!(
            thread,
            Some(med.to_string()),
            "event {event} resolves to its line"
        );
    }
}

#[tokio::test]
async fn only_the_six_type_thread_index_survives_a_schema_load() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let defs: Vec<String> = c
        .query(
            "SELECT indexname || ' ' || indexdef FROM pg_indexes \
             WHERE indexname LIKE 'event_log_medication_thread_idx%'",
            &[],
        )
        .await
        .unwrap()
        .iter()
        .map(|r| r.get(0))
        .collect();
    assert_eq!(defs.len(), 1, "exactly one thread index: {defs:?}");
    assert!(
        defs[0].starts_with("event_log_medication_thread_idx2 "),
        "{defs:?}"
    );
    assert!(
        defs[0].contains("medication-withhold.asserted")
            && defs[0].contains("medication-withhold-release.asserted"),
        "predicate covers the withhold types: {defs:?}"
    );
}
