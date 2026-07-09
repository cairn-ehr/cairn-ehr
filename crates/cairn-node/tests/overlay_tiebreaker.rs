//! Deterministic HLC-overlay tiebreaker (#115): the shared `cairn_hlc_overlay_wins()`
//! predicate, and arrival-order-independent convergence of the five state overlays when
//! two DISTINCT events share an identical (wall, counter, origin) triple (a Byzantine /
//! broken-signer collision). Real Postgres, gated on `$CAIRN_TEST_PG`, serialized
//! cluster-wide via `db::test_serial_guard`.
use cairn_event::identity::{
    link_assertion_body, render_link_twin, render_unlink_twin, unlink_assertion_body, LinkAssertion,
};
use cairn_event::{event_address, generate_key, sign, EventBody, Hlc, SigningKey};
use cairn_node::db;
use tokio_postgres::Client;
use uuid::Uuid;

fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

/// Evaluate the pure predicate in the DB. `na`/`ca` are the content-address bytes (current
/// side nullable — the "no winner yet" case an overlay hits on its first insert).
#[allow(clippy::too_many_arguments)] // mirrors the same-shaped helpers in match_veto.rs / demographics_names.rs
async fn wins(
    c: &Client,
    nw: i64,
    nc: i32,
    no: &str,
    na: Vec<u8>,
    cw: Option<i64>,
    cc: Option<i32>,
    co: Option<&str>,
    ca: Option<Vec<u8>>,
) -> bool {
    c.query_one(
        "SELECT cairn_hlc_overlay_wins($1,$2,$3,$4,$5,$6,$7,$8)",
        &[&nw, &nc, &no, &na, &cw, &cc, &co, &ca],
    )
    .await
    .unwrap()
    .get(0)
}

#[tokio::test]
async fn overlay_predicate_is_a_deterministic_total_order() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();

    // Higher wall wins regardless of everything after it.
    assert!(
        wins(
            &c,
            2,
            0,
            "a",
            vec![1],
            Some(1),
            Some(9),
            Some("z"),
            Some(vec![9])
        )
        .await
    );
    // Lower wall loses regardless of everything after it.
    assert!(
        !wins(
            &c,
            1,
            9,
            "z",
            vec![9],
            Some(2),
            Some(0),
            Some("a"),
            Some(vec![1])
        )
        .await
    );
    // Equal (wall, counter, origin): the content_address breaks the tie deterministically.
    assert!(
        wins(
            &c,
            5,
            3,
            "peer",
            vec![2],
            Some(5),
            Some(3),
            Some("peer"),
            Some(vec![1])
        )
        .await
    );
    assert!(
        !wins(
            &c,
            5,
            3,
            "peer",
            vec![1],
            Some(5),
            Some(3),
            Some("peer"),
            Some(vec![2])
        )
        .await
    );
    // A full tie (same address too) is NOT a win — strict-greater, so an idempotent re-apply
    // never churns the row.
    assert!(
        !wins(
            &c,
            5,
            3,
            "peer",
            vec![7],
            Some(5),
            Some(3),
            Some("peer"),
            Some(vec![7])
        )
        .await
    );
    // No current winner yet (COALESCE path): a real event always beats the absent row.
    assert!(wins(&c, 0, 0, "", vec![0], None, None, None, None).await);
}

/// The Postgres RAISE text for a failed statement (Display renders only "db error"; the real
/// message lives in the DbError payload — project convention, see identity_linkage.rs).
/// Unused by this task's test (which only asserts convergence, not a specific rejection
/// message) but part of the shared harness later collision tests (#115 follow-on tasks) reuse.
#[allow(dead_code)]
fn db_msg(e: &tokio_postgres::Error) -> String {
    e.as_db_error()
        .map(|d| d.message().to_string())
        .unwrap_or_else(|| e.to_string())
}

/// Truncate every clinical + Group-A overlay table and enroll one agent signer + one human
/// attester (distinct keys — the suppressing repudiation in Task 5 needs a human token).
/// Overlay tables from later migrations are truncated behind `to_regclass` guards so setup()
/// stays correct even on a DB migrated only to an earlier stage (the identity_*.rs pattern).
async fn setup(c: &Client) -> (SigningKey, String, SigningKey, String) {
    c.batch_execute(
        "TRUNCATE event_log, actor_event, patient_chart, patient_identifier, \
         patient_demographic, patient_name, patient_link, person_member, \
         identity_projection_flag CASCADE",
    )
    .await
    .unwrap();
    c.batch_execute(
        "DO $$ BEGIN \
           IF to_regclass('public.chart_dispute') IS NOT NULL THEN TRUNCATE chart_dispute; END IF; \
           IF to_regclass('public.chart_identity_state') IS NOT NULL THEN TRUNCATE chart_identity_state; END IF; \
           IF to_regclass('public.name_repudiation') IS NOT NULL THEN TRUNCATE name_repudiation; END IF; \
         END $$;",
    ).await.unwrap();
    c.batch_execute("UPDATE hlc_state SET hlc_wall = 0, hlc_counter = 0")
        .await
        .unwrap();
    let (sk_a, kid_a) = generate_key().unwrap();
    let (sk_h, kid_h) = generate_key().unwrap();
    c.execute(
        "SELECT enroll_actor('agent', '{\"model\":\"tb-stub\",\"version\":\"1\",\"skill_epoch\":\"e\"}', $1)",
        &[&kid_a],
    ).await.unwrap();
    c.execute(
        "SELECT enroll_actor('human', '{\"role\":\"records-officer\"}', $1)",
        &[&kid_h],
    )
    .await
    .unwrap();
    (sk_a, kid_a, sk_h, kid_h)
}

/// Apply one validly-signed remote event through the in-DB clinical apply door (db/020),
/// which takes the wire HLC verbatim — the ONLY door that lets two events carry a colliding
/// (wall, counter, origin) triple, and the realistic foreign-node scenario.
async fn apply(c: &Client, signed: &[u8]) -> Result<u64, tokio_postgres::Error> {
    c.execute("SELECT apply_remote_event($1)", &[&signed.to_vec()])
        .await
}

/// Clean the event log + projections between the two arrival orders WITHOUT dropping the
/// actor enrollment (re-running setup() would mint new keys and un-enroll the pre-signed
/// events). hlc_state is reset so the local merge does not carry over.
async fn reset_between_orders(c: &Client) {
    c.batch_execute(
        "TRUNCATE event_log, patient_chart, patient_identifier, patient_demographic, \
         patient_name, patient_link, person_member, identity_projection_flag CASCADE",
    )
    .await
    .unwrap();
    c.batch_execute(
        "DO $$ BEGIN \
           IF to_regclass('public.chart_dispute') IS NOT NULL THEN TRUNCATE chart_dispute; END IF; \
           IF to_regclass('public.chart_identity_state') IS NOT NULL THEN TRUNCATE chart_identity_state; END IF; \
           IF to_regclass('public.name_repudiation') IS NOT NULL THEN TRUNCATE name_repudiation; END IF; \
         END $$;",
    ).await.unwrap();
    c.batch_execute("UPDATE hlc_state SET hlc_wall = 0, hlc_counter = 0")
        .await
        .unwrap();
}

/// A signed link (or unlink) for the SAME (a, b) pair at a chosen HLC triple. link vs unlink
/// changes the event_type (and event_id), so the two events differ in signed bytes ⇒ differ
/// in content_address ⇒ they collide on (wall, counter, origin) but never on the tiebreak.
fn link_event(kid: &str, a: Uuid, b: Uuid, link: bool, wall: i64, counter: i32) -> EventBody {
    let a_s = a.to_string();
    let b_s = b.to_string();
    let la = LinkAssertion {
        subject_a: &a_s,
        subject_b: &b_s,
        provenance: "tb:conv",
        confidence: None,
    };
    let (etype, payload, twin) = if link {
        (
            "identity.link.asserted",
            link_assertion_body(&la),
            render_link_twin(&la),
        )
    } else {
        (
            "identity.unlink.asserted",
            unlink_assertion_body(&la),
            render_unlink_twin(&la),
        )
    };
    EventBody {
        event_id: Uuid::now_v7().to_string(),
        patient_id: a_s.clone(),
        event_type: etype.into(),
        schema_version: "identity.link/1".into(),
        hlc: Hlc {
            wall,
            counter,
            node_origin: "peer".into(),
        },
        t_effective: None,
        signer_key_id: kid.into(),
        contributors: serde_json::json!([{"actor_id": kid, "role": "recorded"}]),
        payload,
        attachments: vec![],
        plaintext_twin: Some(twin),
    }
}

#[tokio::test]
async fn patient_link_converges_under_hlc_collision() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid, _sk_h, _kid_h) = setup(&c).await;

    let (a, b) = (Uuid::now_v7(), Uuid::now_v7());
    // Two distinct events at an IDENTICAL (wall, counter, origin): a link and an unlink of the
    // same pair. Winner MUST be the higher content_address, both arrival orders.
    let e_link = sign(&link_event(&kid, a, b, true, 5000, 7), &sk)
        .unwrap()
        .signed_bytes;
    let e_unlink = sign(&link_event(&kid, a, b, false, 5000, 7), &sk)
        .unwrap()
        .signed_bytes;
    let expect = if event_address(&e_unlink) > event_address(&e_link) {
        "unlink"
    } else {
        "link"
    };
    let (lo, hi) = if a < b { (a, b) } else { (b, a) };
    // tokio-postgres in this project has no uuid `ToSql` feature enabled (project
    // convention, see identity_linkage.rs::edge_state) — cast text params through uuid.
    let (lo_s, hi_s) = (lo.to_string(), hi.to_string());

    apply(&c, &e_link).await.expect("link applies");
    apply(&c, &e_unlink).await.expect("unlink applies");
    let state1: String = c
        .query_one(
            "SELECT state FROM patient_link WHERE low = $1::text::uuid AND high = $2::text::uuid",
            &[&lo_s, &hi_s],
        )
        .await
        .unwrap()
        .get(0);

    reset_between_orders(&c).await;
    apply(&c, &e_unlink).await.expect("unlink applies");
    apply(&c, &e_link).await.expect("link applies");
    let state2: String = c
        .query_one(
            "SELECT state FROM patient_link WHERE low = $1::text::uuid AND high = $2::text::uuid",
            &[&lo_s, &hi_s],
        )
        .await
        .unwrap()
        .get(0);

    assert_eq!(
        state1, state2,
        "arrival order must not change the winner (#115)"
    );
    assert_eq!(
        state1, expect,
        "winner is the higher content_address, deterministically"
    );
}
