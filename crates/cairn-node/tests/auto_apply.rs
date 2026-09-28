//! Integration coverage for the §5.2/§5.7 C2b auto-apply seam. Real Postgres, gated on
//! $CAIRN_TEST_PG, serialized cluster-wide via db::test_serial_guard. No submit_event
//! change is exercised: C2b composes the db/018 identity floor + db/016 veto + db/004
//! actor registry, all reused unmodified.
use cairn_event::demographics::{
    identifier_assertion_body, render_identifier_twin, IdentifierAssertion,
};
use cairn_event::{generate_key, sign, EventBody, Hlc, SigningKey};
use cairn_node::auto_apply::{
    apply_auto_candidate, apply_auto_candidates, AutoOutcome, AutoSummary,
};
use cairn_node::chart_link::LinkVerb;
use cairn_node::db;
use cairn_node::matcher_actor::resolve_matcher_actor;
use tokio_postgres::Client;
use uuid::Uuid;

// The shared scaffolding, for `register_pair` alone: since #345 a chart must be registered
// before any event may be authored about it, and copying a fourth registration builder into
// this suite is exactly the drift #120/#327 exist to stop. `apply_proposal.rs` arranges its
// pairs identically, which is why the helper is in `common` and not here.
mod common;

fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

/// Truncate every table this seam (and its veto seeding) touches. patient_link /
/// person_member guarded by to_regclass so this stays correct as db/018 grows.
async fn reset(c: &Client) {
    c.batch_execute(
        "TRUNCATE event_log, actor_event, patient_chart, patient_identifier, \
         patient_demographic, match_proposal CASCADE",
    )
    .await
    .unwrap();
    c.batch_execute(
        "DO $$ BEGIN \
           IF to_regclass('public.patient_link')  IS NOT NULL THEN TRUNCATE patient_link;  END IF; \
           IF to_regclass('public.person_member') IS NOT NULL THEN TRUNCATE person_member; END IF; \
         END $$;",
    )
    .await
    .unwrap();
}

/// Seed one match_proposal for the canonical (low, high) pair with the given band/status.
async fn seed_proposal(c: &Client, low: Uuid, high: Uuid, band: &str, status: &str, version: &str) {
    let (low_s, high_s) = (low.to_string(), high.to_string());
    c.execute(
        "INSERT INTO match_proposal \
           (patient_low, patient_high, score_total, band, veto_findings, evidence, matcher_version, status) \
         VALUES ($1::text::uuid,$2::text::uuid, 9.10, $3, '[]'::jsonb, '[]'::jsonb, $4, $5)",
        &[&low_s, &high_s, &band.to_string(), &version.to_string(), &status.to_string()],
    )
    .await
    .unwrap();
}

fn canonical(a: Uuid, b: Uuid) -> (Uuid, Uuid) {
    if a < b {
        (a, b)
    } else {
        (b, a)
    }
}

/// Enroll a throwaway `agent` signer (skill_epoch 'seed', distinct from any matcher epoch)
/// used only to author the demographic identifier assertions that create a hard veto.
async fn enroll_seeder(c: &Client) -> (SigningKey, String) {
    let (sk, kid) = generate_key().unwrap();
    c.execute(
        "SELECT enroll_actor('agent', '{\"model\":\"seed\",\"version\":\"1\",\"skill_epoch\":\"seed\"}', $1)",
        &[&kid],
    )
    .await
    .unwrap();
    (sk, kid)
}

/// Submit one §4.4 identifier assertion for `patient` (mirrors tests/match_veto.rs).
async fn submit_identifier(
    c: &Client,
    sk: &SigningKey,
    kid: &str,
    patient: Uuid,
    wall: i64,
    a: &IdentifierAssertion<'_>,
) {
    let body = EventBody {
        event_id: Uuid::now_v7().to_string(),
        patient_id: patient.to_string(),
        event_type: "demographic.identifier.asserted".into(),
        schema_version: "demographic.identifier/1".into(),
        hlc: Hlc {
            wall,
            counter: 0,
            node_origin: "n".into(),
        },
        t_effective: None,
        signer_key_id: kid.into(),
        contributors: serde_json::json!([{"actor_id": kid, "role": "recorded"}]),
        payload: identifier_assertion_body(a),
        attachments: vec![],
        plaintext_twin: Some(render_identifier_twin(a)),
        clock_grade: cairn_event::ClockGrade::SelfAsserted,
        safety: None,
    };
    let signed = sign(&body, sk).unwrap();
    c.execute("SELECT submit_event($1)", &[&signed.signed_bytes])
        .await
        .expect("valid identifier accepted");
}

/// Create a hard veto between the pair: two SAME-system identifiers with DIFFERENT
/// normalized values (db/016 `identifier` hard_veto). After this,
/// `EXISTS(SELECT 1 FROM cairn_match_veto(a,b))` is TRUE.
async fn assert_identifier_clash(
    c: &Client,
    seeder_sk: &SigningKey,
    seeder_kid: &str,
    a: Uuid,
    b: Uuid,
) {
    let ia = |value: &'static str| IdentifierAssertion {
        value,
        system: "medicare-au",
        provenance: "patient-stated",
        normalized: Some(value),
        // The floor requires a named profile whenever a normalized form is present
        // (the materialised-key rule, §4.4). A stub tag satisfies it for the fixture.
        profile: Some("test-profile@stub"),
        use_: None,
    };
    submit_identifier(c, seeder_sk, seeder_kid, a, 1, &ia("9434765919")).await;
    submit_identifier(c, seeder_sk, seeder_kid, b, 2, &ia("5000000000")).await;
    let vetoed: bool = c
        .query_one(
            "SELECT EXISTS(SELECT 1 FROM cairn_match_veto($1::text::uuid, $2::text::uuid))",
            &[&a.to_string(), &b.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert!(
        vetoed,
        "identifier clash must produce a veto (test precondition)"
    );
}

// ---------------------------------------------------------------------------
// Task 3 — resolve_matcher_actor
// ---------------------------------------------------------------------------

#[tokio::test]
async fn resolve_enrolls_agent_once_and_reuses_it() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c: Client = db::connect_and_load_schema(&base).await.unwrap();
    reset(&c).await;
    let dir = tempfile::tempdir().unwrap();

    let (_sk1, kid1) = resolve_matcher_actor(&c, dir.path(), None, "0.3.0+aaa")
        .await
        .unwrap();
    let n1: i64 = c
        .query_one(
            "SELECT count(*) FROM actor_event WHERE signing_key_id=$1 AND kind='agent' AND op='enroll'",
            &[&kid1],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(n1, 1, "matcher agent enrolled on first sight");

    let (_sk2, kid2) = resolve_matcher_actor(&c, dir.path(), None, "0.3.0+aaa")
        .await
        .unwrap();
    assert_eq!(kid1, kid2, "same epoch reuses the same key");
    let n2: i64 = c
        .query_one(
            "SELECT count(*) FROM actor_event WHERE signing_key_id=$1 AND kind='agent' AND op='enroll'",
            &[&kid1],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(n2, 1, "no duplicate enroll on reuse");
}

#[tokio::test]
async fn distinct_epochs_get_distinct_actors_and_keys() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c: Client = db::connect_and_load_schema(&base).await.unwrap();
    reset(&c).await;
    let dir = tempfile::tempdir().unwrap();

    let (_a, kid_a) = resolve_matcher_actor(&c, dir.path(), None, "0.3.0+aaa")
        .await
        .unwrap();
    let (_b, kid_b) = resolve_matcher_actor(&c, dir.path(), None, "0.3.0+bbb")
        .await
        .unwrap();
    assert_ne!(kid_a, kid_b, "a fresh key per epoch");
    let n: i64 = c
        .query_one(
            "SELECT count(DISTINCT actor_id) FROM actor_event WHERE kind='agent' AND op='enroll'",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(n, 2);
}

/// A revoked (contamination-cascade-recalled) matcher epoch whose key file still sits in
/// the keystore must NOT be silently re-enrolled: the idempotency check keys on the
/// registry HISTORY (actor_event), not on actor_current (which lists only non-revoked
/// identities), so a resurrecting re-enroll can never re-authorise a recalled matcher.
#[tokio::test]
async fn revoked_matcher_epoch_is_refused_not_resurrected() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c: Client = db::connect_and_load_schema(&base).await.unwrap();
    reset(&c).await;
    let dir = tempfile::tempdir().unwrap();

    // Enroll the epoch once (its key is written to `dir`), then REVOKE it by appending a
    // revoke actor_event for the same actor_id (what a db/006 recall of a bad config does).
    let (_sk, kid) = resolve_matcher_actor(&c, dir.path(), None, "0.3.0+bad")
        .await
        .unwrap();
    c.execute(
        "INSERT INTO actor_event (actor_id, op, kind, signing_key_id) \
         SELECT actor_id, 'revoke', 'agent', signing_key_id FROM actor_event \
         WHERE signing_key_id=$1 AND op='enroll'",
        &[&kid],
    )
    .await
    .unwrap();
    let live: i64 = c
        .query_one(
            "SELECT count(*) FROM actor_current WHERE signing_key_id=$1",
            &[&kid],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(live, 0, "revoked epoch is no longer current (precondition)");

    // Resolving again (the key file is still on disk) must REFUSE — never re-enroll.
    let again = resolve_matcher_actor(&c, dir.path(), None, "0.3.0+bad").await;
    assert!(
        again.is_err(),
        "a revoked matcher epoch must not be re-enrolled/resurrected"
    );

    // No second enroll was appended: the revoke stands.
    let enrolls: i64 = c
        .query_one(
            "SELECT count(*) FROM actor_event WHERE signing_key_id=$1 AND op='enroll'",
            &[&kid],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(enrolls, 1, "no resurrecting re-enroll");
    let still_dead: i64 = c
        .query_one(
            "SELECT count(*) FROM actor_current WHERE signing_key_id=$1",
            &[&kid],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(still_dead, 0, "the recalled matcher stays recalled");
}

// ---------------------------------------------------------------------------
// Task 4 — apply_auto_candidate (single proposal)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn auto_candidate_becomes_unattested_link_and_projects_person() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c: Client = db::connect_and_load_schema(&base).await.unwrap();
    reset(&c).await;
    let dir = tempfile::tempdir().unwrap();
    let (low, high) = canonical(Uuid::now_v7(), Uuid::now_v7());
    let (seed_sk, seed_kid) = enroll_seeder(&c).await;
    common::register_pair(&c, &seed_sk, &seed_kid, low, high).await;
    seed_proposal(&c, low, high, "auto_candidate", "pending", "0.3.0+aaa").await;

    let (sk, kid) = resolve_matcher_actor(&c, dir.path(), None, "0.3.0+aaa")
        .await
        .unwrap();
    let out = apply_auto_candidate(
        &mut c,
        low,
        high,
        &sk,
        &kid,
        Hlc {
            wall: 100,
            counter: 0,
            node_origin: "testnode".into(),
        },
    )
    .await
    .unwrap();
    assert!(matches!(out, AutoOutcome::Applied(_)));

    // Exactly one link event, appended with NO attestation (attestation column NULL).
    let n_ev: i64 = c
        .query_one(
            "SELECT count(*) FROM event_log WHERE event_type='identity.link.asserted' AND attestation IS NULL",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(n_ev, 1, "one un-attested link event appended");

    // Standing edge + person projection (both patients -> min-UUID person).
    let (low_s, high_s) = (low.to_string(), high.to_string());
    let n_edge: i64 = c
        .query_one(
            "SELECT count(*) FROM patient_link WHERE low=$1::text::uuid AND high=$2::text::uuid AND state='link'",
            &[&low_s, &high_s],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(n_edge, 1);
    // Uuid has no FromSql in this crate's tokio-postgres; read UUIDs as text.
    let person: String = c
        .query_one(
            "SELECT person_id::text FROM person_member WHERE patient_id=$1::text::uuid",
            &[&low_s],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        person,
        low.min(high).to_string(),
        "person = min-UUID of the component"
    );

    // Proposal marked auto_applied with an applied_event_id.
    let r = c
        .query_one(
            "SELECT status, applied_event_id::text FROM match_proposal \
             WHERE patient_low=$1::text::uuid AND patient_high=$2::text::uuid",
            &[&low_s, &high_s],
        )
        .await
        .unwrap();
    let status: String = r.get(0);
    let applied: Option<String> = r.get(1);
    assert_eq!(status, "auto_applied");
    assert!(applied.is_some());
}

#[tokio::test]
async fn veto_appeared_since_propose_kicks_to_review_no_event() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c: Client = db::connect_and_load_schema(&base).await.unwrap();
    reset(&c).await;
    let dir = tempfile::tempdir().unwrap();
    let (low, high) = canonical(Uuid::now_v7(), Uuid::now_v7());
    let (seed_sk, seed_kid) = enroll_seeder(&c).await;
    common::register_pair(&c, &seed_sk, &seed_kid, low, high).await;
    seed_proposal(&c, low, high, "auto_candidate", "pending", "0.3.0+aaa").await;

    // A hard veto now exists (appeared after the propose-time band was computed).
    assert_identifier_clash(&c, &seed_sk, &seed_kid, low, high).await;

    let (sk, kid) = resolve_matcher_actor(&c, dir.path(), None, "0.3.0+aaa")
        .await
        .unwrap();
    let out = apply_auto_candidate(
        &mut c,
        low,
        high,
        &sk,
        &kid,
        Hlc {
            wall: 100,
            counter: 0,
            node_origin: "testnode".into(),
        },
    )
    .await
    .unwrap();
    assert!(matches!(out, AutoOutcome::VetoedToReview));

    let n_ev: i64 = c
        .query_one(
            "SELECT count(*) FROM event_log WHERE event_type='identity.link.asserted'",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(n_ev, 0, "no link event when a veto appeared");
    let status: String = c
        .query_one(
            "SELECT status FROM match_proposal WHERE patient_low=$1::text::uuid AND patient_high=$2::text::uuid",
            &[&low.to_string(), &high.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(status, "review", "since-vetoed proposal kicked to a human");
}

#[tokio::test]
async fn human_rejected_auto_candidate_is_skipped() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c: Client = db::connect_and_load_schema(&base).await.unwrap();
    reset(&c).await;
    let dir = tempfile::tempdir().unwrap();
    let (low, high) = canonical(Uuid::now_v7(), Uuid::now_v7());
    let (seed_sk, seed_kid) = enroll_seeder(&c).await;
    common::register_pair(&c, &seed_sk, &seed_kid, low, high).await;
    // A human already REJECTED this auto_candidate -> must NOT be auto-applied.
    seed_proposal(&c, low, high, "auto_candidate", "rejected", "0.3.0+aaa").await;

    let (sk, kid) = resolve_matcher_actor(&c, dir.path(), None, "0.3.0+aaa")
        .await
        .unwrap();
    let out = apply_auto_candidate(
        &mut c,
        low,
        high,
        &sk,
        &kid,
        Hlc {
            wall: 100,
            counter: 0,
            node_origin: "testnode".into(),
        },
    )
    .await
    .unwrap();
    assert!(matches!(out, AutoOutcome::Skipped(_)));
    let n_ev: i64 = c
        .query_one(
            "SELECT count(*) FROM event_log WHERE event_type='identity.link.asserted'",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(n_ev, 0);
    let status: String = c
        .query_one(
            "SELECT status FROM match_proposal WHERE patient_low=$1::text::uuid AND patient_high=$2::text::uuid",
            &[&low.to_string(), &high.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(status, "rejected", "a human's disposition is untouched");
}

/// ADR-0076 decision 5. Since R2a an UN-attested link loses
/// to a standing ATTESTED judgement on the same pair — so a matcher link submitted over a
/// human's "different people" (or "same person") changes nothing in `patient_link`. Before
/// the fix, auto-apply submitted it anyway and marked the proposal `auto_applied` with an
/// `applied_event_id` pointing at the LOSING event, and the summary counted it applied: a
/// precise untruth (principle 4) in the very table R5's worklist reads. The pair must be
/// skipped instead, writing nothing — no event, no status change.
///
/// The human's judgement lands through the REMOTE door (a peer's clinician made it), which
/// — unlike `unlink_charts` on this node — leaves the local proposal `pending`: exactly the
/// state in which auto-apply would otherwise act. Run for both verbs: an attested LINK
/// already standing makes a matcher link just as redundant.
#[tokio::test]
async fn a_pair_a_human_already_judged_is_skipped_and_nothing_is_written() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c: Client = db::connect_and_load_schema(&base).await.unwrap();
    for human_says_same in [false, true] {
        reset(&c).await;
        let dir = tempfile::tempdir().unwrap();
        let (low, high) = canonical(Uuid::now_v7(), Uuid::now_v7());
        let (seed_sk, seed_kid) = enroll_seeder(&c).await;
        common::register_pair(&c, &seed_sk, &seed_kid, low, high).await;
        seed_proposal(&c, low, high, "auto_candidate", "pending", "0.3.0+aaa").await;

        let (sk_h, kid_h) = common::enroll_human(&c).await;
        let verb = if human_says_same {
            LinkVerb::Link
        } else {
            LinkVerb::Unlink
        };
        let judged = common::link_assertion_event(&kid_h, low, high, verb, 50, 0, "peer", true);
        common::apply_remote_attested(&c, &sk_h, judged, &sk_h, &kid_h)
            .await
            .expect("the peer's attested judgement lands");
        let events_before: i64 = c
            .query_one("SELECT count(*) FROM event_log", &[])
            .await
            .unwrap()
            .get(0);

        let (sk, kid) = resolve_matcher_actor(&c, dir.path(), None, "0.3.0+aaa")
            .await
            .unwrap();
        let out = apply_auto_candidate(
            &mut c,
            low,
            high,
            &sk,
            &kid,
            Hlc {
                wall: 100,
                counter: 0,
                node_origin: "testnode".into(),
            },
        )
        .await
        .unwrap();
        // Its own outcome, not a generic skip: the proposal stays `pending` (#700), so the
        // batch driver must be able to count these apart from "absent"/"not pending".
        match out {
            AutoOutcome::AlreadyJudged { state } => assert_eq!(
                state,
                if human_says_same { "link" } else { "unlink" },
                "human_says_same={human_says_same}: names the standing judgement"
            ),
            _ => panic!("human_says_same={human_says_same}: must be AlreadyJudged"),
        }
        let events_after: i64 = c
            .query_one("SELECT count(*) FROM event_log", &[])
            .await
            .unwrap()
            .get(0);
        assert_eq!(
            events_after, events_before,
            "human_says_same={human_says_same}: no matcher event was written"
        );
        let r = c
            .query_one(
                "SELECT status, applied_event_id::text FROM match_proposal \
                 WHERE patient_low=$1::text::uuid AND patient_high=$2::text::uuid",
                &[&low.to_string(), &high.to_string()],
            )
            .await
            .unwrap();
        let (status, applied): (String, Option<String>) = (r.get(0), r.get(1));
        assert_eq!(
            (status.as_str(), applied),
            ("pending", None),
            "human_says_same={human_says_same}: the proposal is not marked auto_applied"
        );
        // The batch driver counts it in its own bucket, so the operator's summary shows a
        // pair that will stay pending rather than hiding it among benign skips.
        let s: AutoSummary = apply_auto_candidates(&mut c, dir.path(), None, "testnode")
            .await
            .unwrap();
        assert_eq!(
            (s.applied, s.skipped, s.human_judged, s.errored),
            (0, 0, 1, 0),
            "human_says_same={human_says_same}"
        );
    }
}

/// A pair a human has already judged is not sent back to a human because a veto appeared.
/// The human-judgement check runs BEFORE the veto re-check: a peer's clinician linked two
/// charts that now trip a hard veto here, and kicking the local proposal to `review` would
/// ask a clinician to decide what a clinician already decided.
#[tokio::test]
async fn a_vetoed_pair_a_human_already_judged_is_not_sent_back_to_review() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c: Client = db::connect_and_load_schema(&base).await.unwrap();
    reset(&c).await;
    let dir = tempfile::tempdir().unwrap();
    let (low, high) = canonical(Uuid::now_v7(), Uuid::now_v7());
    let (seed_sk, seed_kid) = enroll_seeder(&c).await;
    common::register_pair(&c, &seed_sk, &seed_kid, low, high).await;
    assert_identifier_clash(&c, &seed_sk, &seed_kid, low, high).await;
    seed_proposal(&c, low, high, "auto_candidate", "pending", "0.3.0+aaa").await;

    let (sk_h, kid_h) = common::enroll_human(&c).await;
    let judged =
        common::link_assertion_event(&kid_h, low, high, LinkVerb::Link, 50, 0, "peer", true);
    common::apply_remote_attested(&c, &sk_h, judged, &sk_h, &kid_h)
        .await
        .expect("the peer's attested link lands (a human may link over a veto)");

    let (sk, kid) = resolve_matcher_actor(&c, dir.path(), None, "0.3.0+aaa")
        .await
        .unwrap();
    let out = apply_auto_candidate(
        &mut c,
        low,
        high,
        &sk,
        &kid,
        Hlc {
            wall: 100,
            counter: 0,
            node_origin: "testnode".into(),
        },
    )
    .await
    .unwrap();
    assert!(
        matches!(out, AutoOutcome::AlreadyJudged { ref state } if state == "link"),
        "the standing human judgement answers first"
    );
    let status: String = c
        .query_one(
            "SELECT status FROM match_proposal \
             WHERE patient_low=$1::text::uuid AND patient_high=$2::text::uuid",
            &[&low.to_string(), &high.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(status, "pending", "not kicked to review");
}

/// A matcher link that LOSES the overlay once submitted is rolled back, not recorded as
/// applied. The step-2 read cannot see everything that outranks the link: an assertion
/// committed after it (a peer's judgement racing it — #700), or a standing assertion with a
/// later HLC than the one this run was handed. Deterministic stand-in for the race: a
/// peer's UN-attested unlink at a later HLC (so step 2, which looks only for attested
/// rows, lets the pair through). Submitting would change nothing in `patient_link`; marking
/// the proposal `auto_applied` with the LOSING event's id would be a precise untruth.
#[tokio::test]
async fn a_matcher_link_that_loses_the_overlay_is_rolled_back_not_marked_applied() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c: Client = db::connect_and_load_schema(&base).await.unwrap();
    reset(&c).await;
    let dir = tempfile::tempdir().unwrap();
    let (low, high) = canonical(Uuid::now_v7(), Uuid::now_v7());
    let (seed_sk, seed_kid) = enroll_seeder(&c).await;
    common::register_pair(&c, &seed_sk, &seed_kid, low, high).await;
    seed_proposal(&c, low, high, "auto_candidate", "pending", "0.3.0+aaa").await;

    let later_unlink = common::link_assertion_event(
        &seed_kid,
        low,
        high,
        LinkVerb::Unlink,
        500,
        0,
        "peer",
        false,
    );
    common::apply_remote_raw(&c, &seed_sk, later_unlink)
        .await
        .expect("the peer's un-attested unlink lands");
    let events_before: i64 = c
        .query_one("SELECT count(*) FROM event_log", &[])
        .await
        .unwrap()
        .get(0);

    let (sk, kid) = resolve_matcher_actor(&c, dir.path(), None, "0.3.0+aaa")
        .await
        .unwrap();
    let out = apply_auto_candidate(
        &mut c,
        low,
        high,
        &sk,
        &kid,
        Hlc {
            wall: 100,
            counter: 0,
            node_origin: "testnode".into(),
        },
    )
    .await
    .unwrap();
    match out {
        AutoOutcome::Skipped(why) => assert!(why.contains("outranked"), "names why: {why}"),
        _ => panic!("a link that did not take effect must not be reported Applied"),
    }
    let events_after: i64 = c
        .query_one("SELECT count(*) FROM event_log", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        events_after, events_before,
        "the losing link was rolled back"
    );
    let r = c
        .query_one(
            "SELECT status, applied_event_id::text FROM match_proposal \
             WHERE patient_low=$1::text::uuid AND patient_high=$2::text::uuid",
            &[&low.to_string(), &high.to_string()],
        )
        .await
        .unwrap();
    let (status, applied): (String, Option<String>) = (r.get(0), r.get(1));
    assert_eq!((status.as_str(), applied), ("pending", None));
}

// ---------------------------------------------------------------------------
// Task 5 — batch driver + idempotency + recall precision
// ---------------------------------------------------------------------------

#[tokio::test]
async fn batch_applies_all_pending_auto_candidates_across_epochs_and_is_idempotent() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c: Client = db::connect_and_load_schema(&base).await.unwrap();
    reset(&c).await;
    let dir = tempfile::tempdir().unwrap();

    let (l1, h1) = canonical(Uuid::now_v7(), Uuid::now_v7());
    let (l2, h2) = canonical(Uuid::now_v7(), Uuid::now_v7());
    let (seed_sk, seed_kid) = enroll_seeder(&c).await;
    common::register_pair(&c, &seed_sk, &seed_kid, l1, h1).await;
    common::register_pair(&c, &seed_sk, &seed_kid, l2, h2).await;
    seed_proposal(&c, l1, h1, "auto_candidate", "pending", "0.3.0+aaa").await;
    seed_proposal(&c, l2, h2, "auto_candidate", "pending", "0.3.0+bbb").await;
    // A review-band pair must be ignored by the driver — deliberately NOT registered either,
    // since nothing is ever authored about it.
    let (l3, h3) = canonical(Uuid::now_v7(), Uuid::now_v7());
    seed_proposal(&c, l3, h3, "review", "pending", "0.3.0+aaa").await;

    let s: AutoSummary = apply_auto_candidates(&mut c, dir.path(), None, "testnode")
        .await
        .unwrap();
    assert_eq!(s.applied, 2);
    let n_ev: i64 = c
        .query_one(
            "SELECT count(*) FROM event_log WHERE event_type='identity.link.asserted'",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        n_ev, 2,
        "both auto_candidate pairs linked; the review pair ignored"
    );

    // Idempotent: a second run applies nothing new.
    let s2 = apply_auto_candidates(&mut c, dir.path(), None, "testnode")
        .await
        .unwrap();
    assert_eq!(s2.applied, 0);
    let n_ev2: i64 = c
        .query_one(
            "SELECT count(*) FROM event_log WHERE event_type='identity.link.asserted'",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(n_ev2, 2, "no new events on re-run");
}

#[tokio::test]
async fn recall_over_the_matcher_epoch_selects_its_autolinks_precisely() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c: Client = db::connect_and_load_schema(&base).await.unwrap();
    reset(&c).await;
    let dir = tempfile::tempdir().unwrap();

    let (l1, h1) = canonical(Uuid::now_v7(), Uuid::now_v7());
    let (seed_sk, seed_kid) = enroll_seeder(&c).await;
    common::register_pair(&c, &seed_sk, &seed_kid, l1, h1).await;
    seed_proposal(&c, l1, h1, "auto_candidate", "pending", "0.3.0+aaa").await;
    apply_auto_candidates(&mut c, dir.path(), None, "testnode")
        .await
        .unwrap();

    // The matcher epoch's signing key (signer_key_id) is the recall key (db/006).
    let kid: String = c
        .query_one(
            "SELECT signing_key_id FROM actor_event \
             WHERE op='enroll' AND kind='agent' AND pinned->>'skill_epoch'='0.3.0+aaa'",
            &[],
        )
        .await
        .unwrap()
        .get(0);

    // A recall over THIS (key, epoch) selects the auto-link event, attributed 'pinned'.
    let hit: i64 = c
        .query_one(
            "SELECT count(*) FROM events_by_actor_epoch($1, '0.3.0+aaa') \
             WHERE event_type='identity.link.asserted'",
            &[&kid],
        )
        .await
        .unwrap()
        .get(0);
    assert!(
        hit >= 1,
        "contamination-cascade recall selects the epoch's auto-link"
    );

    // A recall over a DIFFERENT epoch (never registered for this key) selects nothing.
    let miss: i64 = c
        .query_one(
            "SELECT count(*) FROM events_by_actor_epoch($1, 'no-such-epoch')",
            &[&kid],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(miss, 0, "recall is precise to the epoch");
}
