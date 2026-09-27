//! Repair path R2a — `link_charts` / `unlink_charts`: a human's judgement on two charts,
//! authored as an ATTESTED identity event, resolving any open match_proposal for the pair
//! in the same transaction (ADR-0076 decisions 4 and 5; #681's orchestration).
//!
//! Real Postgres, gated on `$CAIRN_TEST_PG`, serialized via `db::test_serial_guard`.
use cairn_event::{generate_key, SigningKey};
use cairn_node::chart_link::{link_charts, unlink_charts, Reviewer};
use cairn_node::db;
use tokio_postgres::Client;
use uuid::Uuid;

mod common;
use common::{apply_remote_raw, link_assertion_event, register_pair, vetoed_pair};

fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

const ORIGIN: &str = "r2a-test-node";

/// Clean identity + proposal state; enroll an agent (to register charts and play the
/// machine) and a human reviewer.
async fn setup(c: &Client) -> (SigningKey, String, SigningKey, String) {
    c.batch_execute(
        "TRUNCATE event_log, actor_event, patient_chart, patient_identifier, \
         patient_demographic, patient_link, person_member, identity_projection_flag CASCADE",
    )
    .await
    .unwrap();
    c.batch_execute(
        "DO $$ BEGIN \
           IF to_regclass('public.chart_dispute') IS NOT NULL THEN TRUNCATE chart_dispute; END IF; \
           IF to_regclass('public.chart_identity_state') IS NOT NULL THEN TRUNCATE chart_identity_state; END IF; \
           IF to_regclass('public.link_veto_flag') IS NOT NULL THEN TRUNCATE link_veto_flag; END IF; \
           TRUNCATE match_proposal; \
         END $$;",
    )
    .await
    .unwrap();
    let (sk_a, kid_a) = generate_key().unwrap();
    let (sk_h, kid_h) = generate_key().unwrap();
    c.execute(
        "SELECT enroll_actor('agent', '{\"model\":\"cl-stub\",\"version\":\"1\",\"skill_epoch\":\"e\"}', $1)",
        &[&kid_a],
    )
    .await
    .unwrap();
    c.execute(
        "SELECT enroll_actor('human', '{\"role\":\"records-officer\",\"actor\":\"CL\"}', $1)",
        &[&kid_h],
    )
    .await
    .unwrap();
    (sk_a, kid_a, sk_h, kid_h)
}

async fn standing(c: &Client, a: Uuid, b: Uuid) -> Option<(String, bool)> {
    let (lo, hi) = if a < b { (a, b) } else { (b, a) };
    c.query_opt(
        "SELECT state, attested FROM patient_link WHERE low = $1::text::uuid AND high = $2::text::uuid",
        &[&lo.to_string(), &hi.to_string()],
    )
    .await
    .unwrap()
    .map(|r| (r.get(0), r.get(1)))
}

async fn seed_proposal(c: &Client, a: Uuid, b: Uuid, status: &str) {
    let (lo, hi) = if a < b { (a, b) } else { (b, a) };
    c.execute(
        "INSERT INTO match_proposal \
           (patient_low, patient_high, score_total, band, veto_findings, evidence, matcher_version, status) \
         VALUES ($1::text::uuid, $2::text::uuid, 0.91, 'review', '[]'::jsonb, '[]'::jsonb, 'cfg@test', $3)",
        &[&lo.to_string(), &hi.to_string(), &status.to_string()],
    )
    .await
    .unwrap();
}

async fn proposal(c: &Client, a: Uuid, b: Uuid) -> (String, Option<String>) {
    let (lo, hi) = if a < b { (a, b) } else { (b, a) };
    let r = c
        .query_one(
            "SELECT status, applied_event_id::text FROM match_proposal \
             WHERE patient_low = $1::text::uuid AND patient_high = $2::text::uuid",
            &[&lo.to_string(), &hi.to_string()],
        )
        .await
        .unwrap();
    (r.get(0), r.get(1))
}

#[tokio::test]
async fn a_link_is_attested_and_the_two_charts_read_as_one_set() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, sk_h, kid_h) = setup(&c).await;
    let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
    register_pair(&c, &sk_a, &kid_a, a, b).await;

    let who = Reviewer {
        human_sk: &sk_h,
        human_kid: &kid_h,
    };
    let out = link_charts(&mut c, b, a, &who, ORIGIN).await.expect("link");

    assert_eq!(standing(&c, a, b).await, Some(("link".into(), true)));
    assert_eq!(
        out.charts.members().len(),
        2,
        "the returned set is the combined chart"
    );
    assert!(!out.proposal_resolved, "no proposal existed");
    let attester: Option<Vec<u8>> = c
        .query_one(
            "SELECT attester_key FROM event_log WHERE event_id = $1::text::uuid",
            &[&out.event_id.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        attester,
        Some(sk_h.verifying_key().to_bytes().to_vec()),
        "the human vouched"
    );
}

#[tokio::test]
async fn different_people_is_an_unlink_on_a_pair_never_linked() {
    // ADR-0076 decision 4.
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, sk_h, kid_h) = setup(&c).await;
    let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
    register_pair(&c, &sk_a, &kid_a, a, b).await;

    let who = Reviewer {
        human_sk: &sk_h,
        human_kid: &kid_h,
    };
    let out = unlink_charts(&mut c, a, b, &who, ORIGIN)
        .await
        .expect("unlink");
    assert_eq!(standing(&c, a, b).await, Some(("unlink".into(), true)));
    assert_eq!(out.charts.members(), &[a], "a stays a chart of its own");
}

#[tokio::test]
async fn an_unlink_after_a_link_splits_the_set_again() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, sk_h, kid_h) = setup(&c).await;
    let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
    register_pair(&c, &sk_a, &kid_a, a, b).await;

    let who = Reviewer {
        human_sk: &sk_h,
        human_kid: &kid_h,
    };
    link_charts(&mut c, a, b, &who, ORIGIN).await.unwrap();
    let out = unlink_charts(&mut c, a, b, &who, ORIGIN).await.unwrap();
    assert_eq!(out.charts.members().len(), 1);
}

#[tokio::test]
async fn a_later_machine_link_from_a_peer_does_not_undo_the_reviewers_unlink() {
    // D5 end to end: the reviewer says "different people"; a peer's matcher then links the
    // pair with a later clock. The charts stay two.
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, sk_h, kid_h) = setup(&c).await;
    let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
    register_pair(&c, &sk_a, &kid_a, a, b).await;
    let who = Reviewer {
        human_sk: &sk_h,
        human_kid: &kid_h,
    };
    unlink_charts(&mut c, a, b, &who, ORIGIN).await.unwrap();

    let wall: i64 = c
        .query_one("SELECT max(hlc_wall) FROM patient_link", &[])
        .await
        .unwrap()
        .get(0);
    let later = link_assertion_event(&kid_a, a, b, true, wall + 1_000, 0, "peer-matcher", false);
    apply_remote_raw(&c, &sk_a, later)
        .await
        .expect("the peer's link is admitted (set-union)");

    assert_eq!(standing(&c, a, b).await, Some(("unlink".into(), true)));
}

#[tokio::test]
async fn a_hard_vetoed_pair_can_still_be_linked_by_a_human_and_is_not_flagged() {
    // §5.13: a veto forces a human decision, never an automatic refusal.
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, sk_h, kid_h) = setup(&c).await;
    let (a, b) = vetoed_pair(&c, &sk_a, &kid_a).await;
    let who = Reviewer {
        human_sk: &sk_h,
        human_kid: &kid_h,
    };
    link_charts(&mut c, a, b, &who, ORIGIN)
        .await
        .expect("a human may link a vetoed pair");
    let flags: i64 = c
        .query_one("SELECT count(*) FROM link_veto_flag", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        flags, 0,
        "an attested link is the human decision the veto forces"
    );
}

#[tokio::test]
async fn a_human_link_resolves_a_doubted_machine_link() {
    // The doubted-link state R1 withholds from sign-off (db/054) is lifted by the human.
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, sk_h, kid_h) = setup(&c).await;
    let (a, b) = vetoed_pair(&c, &sk_a, &kid_a).await;
    let machine = link_assertion_event(&kid_a, a, b, true, 50, 0, "peer-matcher", false);
    apply_remote_raw(&c, &sk_a, machine).await.unwrap();
    // A plain async fn rather than a closure: a `|c: &Client| async move { .. }` closure's
    // returned future borrows `c` for the closure's OWN elided lifetime, which the compiler
    // cannot prove outlives the future — a function item's lifetimes are named per-call
    // instead, so this sidesteps the issue rather than fighting it with explicit HRTB syntax.
    async fn doubted(c: &Client, a: Uuid, b: Uuid) -> bool {
        let ids = vec![a.to_string(), b.to_string()];
        c.query_one(
            "SELECT cairn_chart_set_has_doubted_link($1::text[]::uuid[])",
            &[&ids],
        )
        .await
        .unwrap()
        .get(0)
    }
    assert!(
        doubted(&c, a, b).await,
        "precondition: the machine link is doubted"
    );

    let who = Reviewer {
        human_sk: &sk_h,
        human_kid: &kid_h,
    };
    link_charts(&mut c, a, b, &who, ORIGIN).await.unwrap();
    assert!(
        !doubted(&c, a, b).await,
        "the human's attested link lifts the doubt"
    );
}

#[tokio::test]
async fn an_open_proposal_moves_with_the_judgement_in_the_same_transaction() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, sk_h, kid_h) = setup(&c).await;
    let who = Reviewer {
        human_sk: &sk_h,
        human_kid: &kid_h,
    };

    for open in ["pending", "accepted", "review"] {
        let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
        register_pair(&c, &sk_a, &kid_a, a, b).await;
        seed_proposal(&c, a, b, open).await;
        let out = link_charts(&mut c, a, b, &who, ORIGIN).await.unwrap();
        assert!(out.proposal_resolved, "{open} is an open proposal");
        assert_eq!(
            proposal(&c, a, b).await,
            ("applied".into(), Some(out.event_id.to_string()))
        );

        let (x, y) = (Uuid::now_v7(), Uuid::now_v7());
        register_pair(&c, &sk_a, &kid_a, x, y).await;
        seed_proposal(&c, x, y, open).await;
        let out = unlink_charts(&mut c, y, x, &who, ORIGIN).await.unwrap();
        assert!(out.proposal_resolved);
        assert_eq!(
            proposal(&c, x, y).await,
            ("rejected".into(), None),
            "a rejection names no applied event (db/019's invariant)"
        );
    }
}

#[tokio::test]
async fn a_closed_proposal_is_left_exactly_as_it_was() {
    // Review Focus 5.
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, sk_h, kid_h) = setup(&c).await;
    let who = Reviewer {
        human_sk: &sk_h,
        human_kid: &kid_h,
    };
    for closed in ["applied", "auto_applied", "rejected", "retracted"] {
        let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
        register_pair(&c, &sk_a, &kid_a, a, b).await;
        seed_proposal(&c, a, b, closed).await;
        let before = proposal(&c, a, b).await;
        let out = unlink_charts(&mut c, a, b, &who, ORIGIN).await.unwrap();
        assert!(!out.proposal_resolved, "{closed} is not an open proposal");
        assert_eq!(proposal(&c, a, b).await, before);
    }
}

#[tokio::test]
async fn a_non_human_key_is_refused_and_nothing_moves() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, _sk_h, _kid_h) = setup(&c).await;
    let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
    register_pair(&c, &sk_a, &kid_a, a, b).await;
    seed_proposal(&c, a, b, "pending").await;

    let agent = Reviewer {
        human_sk: &sk_a,
        human_kid: &kid_a,
    };
    let err = link_charts(&mut c, a, b, &agent, ORIGIN)
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("not an enrolled human"), "names why: {err}");
    assert_eq!(standing(&c, a, b).await, None, "no event landed");
    assert_eq!(
        proposal(&c, a, b).await.0,
        "pending",
        "the proposal did not move"
    );
}

#[tokio::test]
async fn a_chart_this_node_has_never_seen_is_refused_before_signing() {
    // Review Focus 4.
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, sk_h, kid_h) = setup(&c).await;
    let a = Uuid::now_v7();
    common::submit_registration(&c, &sk_a, &kid_a, a, 1).await;
    let stranger = Uuid::now_v7();

    let who = Reviewer {
        human_sk: &sk_h,
        human_kid: &kid_h,
    };
    for verb_is_link in [true, false] {
        let r = if verb_is_link {
            link_charts(&mut c, a, stranger, &who, ORIGIN).await
        } else {
            unlink_charts(&mut c, stranger, a, &who, ORIGIN).await
        };
        let err = r.unwrap_err().to_string();
        assert!(
            err.contains(&stranger.to_string()),
            "names the unknown chart: {err}"
        );
    }
    let n: i64 = c
        .query_one(
            "SELECT count(*) FROM event_log WHERE event_type LIKE 'identity.%link.asserted'",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(n, 0);
}

#[tokio::test]
async fn a_chart_cannot_be_linked_to_itself() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, sk_h, kid_h) = setup(&c).await;
    let a = Uuid::now_v7();
    common::submit_registration(&c, &sk_a, &kid_a, a, 1).await;
    let who = Reviewer {
        human_sk: &sk_h,
        human_kid: &kid_h,
    };
    let err = link_charts(&mut c, a, a, &who, ORIGIN)
        .await
        .unwrap_err()
        .to_string();
    assert!(err.contains("same chart"), "{err}");
}
