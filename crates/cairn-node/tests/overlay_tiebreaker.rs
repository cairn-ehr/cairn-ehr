//! Deterministic HLC-overlay tiebreaker (#115): the shared `cairn_hlc_overlay_wins()`
//! predicate, and arrival-order-independent convergence of the five state overlays when
//! two DISTINCT events share an identical (wall, counter, origin) triple (a Byzantine /
//! broken-signer collision). Real Postgres, gated on `$CAIRN_TEST_PG`, serialized
//! cluster-wide via `db::test_serial_guard`.
//!
//! Convention: tokio-postgres has no uuid `ToSql` feature enabled in this project (see
//! `identity_linkage.rs::edge_state`), so UUID lookup params are bound as text and cast in
//! SQL via `$n::text::uuid`. That's why the read-back queries below take `*.to_string()`.
use cairn_event::identity::{
    dispute_assertion_body, dispute_resolution_body, identify_assertion_body, link_assertion_body,
    pending_assertion_body, render_dispute_resolved_twin, render_dispute_twin,
    render_identify_twin, render_link_twin, render_pending_twin, render_repudiate_twin,
    render_unlink_twin, repudiation_assertion_body, unlink_assertion_body, DisputeAssertion,
    DisputeResolution, IdentifyAssertion, LinkAssertion, PendingAssertion, RepudiationAssertion,
};
use cairn_event::{
    event_address, generate_key, sign, sign_attestation, EventBody, Hlc, SigningKey,
};
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

/// All recorded collision rows for an overlay, as (addr_lo, addr_hi) byte pairs, ordered so the
/// vec is comparable across arrival orders. Empty when no Byzantine collision was detected.
async fn collision_rows(c: &Client, overlay: &str) -> Vec<(Vec<u8>, Vec<u8>)> {
    c.query(
        "SELECT addr_lo, addr_hi FROM hlc_collision_log WHERE overlay = $1 ORDER BY addr_lo, addr_hi",
        &[&overlay],
    )
    .await
    .unwrap()
    .iter()
    .map(|r| (r.get(0), r.get(1)))
    .collect()
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

/// Truncate every clinical + Group-A overlay table and enroll one agent signer + one human
/// attester (distinct keys — the suppressing repudiation in Task 5 needs a human token).
/// Overlay tables from later migrations are truncated behind `to_regclass` guards so setup()
/// stays correct even on a DB migrated only to an earlier stage (the identity_*.rs pattern).
async fn setup(c: &Client) -> (SigningKey, String, SigningKey, String) {
    c.batch_execute(
        "TRUNCATE event_log, actor_event, patient_chart, patient_identifier, \
         patient_demographic, patient_name, patient_link, person_member, \
         identity_projection_flag, hlc_collision_log CASCADE",
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
         patient_name, patient_link, person_member, identity_projection_flag, \
         hlc_collision_log CASCADE",
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

/// A signed dispute-open (or dispute-resolve) for the SAME dispute_id + subject at a chosen
/// HLC triple. open vs resolve changes the event_type (and event_id) ⇒ different signed
/// bytes ⇒ different content_address; they collide on (wall, counter, origin) only.
fn dispute_event(
    kid: &str,
    dispute_id: Uuid,
    subject: Uuid,
    open: bool,
    descriptive: &str,
    wall: i64,
    counter: i32,
) -> EventBody {
    let did = dispute_id.to_string();
    let subj = subject.to_string();
    let (etype, payload, twin, sver) = if open {
        let d = DisputeAssertion {
            dispute_id: &did,
            subject: &subj,
            reason: descriptive,
        };
        (
            "identity.dispute.asserted",
            dispute_assertion_body(&d),
            render_dispute_twin(&d),
            "identity.dispute.asserted/1",
        )
    } else {
        let d = DisputeResolution {
            dispute_id: &did,
            subject: &subj,
            resolution: descriptive,
        };
        (
            "identity.dispute.resolved",
            dispute_resolution_body(&d),
            render_dispute_resolved_twin(&d),
            "identity.dispute.resolved/1",
        )
    };
    EventBody {
        event_id: Uuid::now_v7().to_string(),
        patient_id: subj.clone(),
        event_type: etype.into(),
        schema_version: sver.into(),
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
async fn chart_dispute_converges_under_hlc_collision() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid, _sk_h, _kid_h) = setup(&c).await;

    let (did, subj) = (Uuid::now_v7(), Uuid::now_v7());
    let e_open = sign(
        &dispute_event(&kid, did, subj, true, "claims never here", 5000, 7),
        &sk,
    )
    .unwrap()
    .signed_bytes;
    let e_resolved = sign(
        &dispute_event(&kid, did, subj, false, "confirmed present", 5000, 7),
        &sk,
    )
    .unwrap()
    .signed_bytes;
    let expect = if event_address(&e_resolved) > event_address(&e_open) {
        "resolved"
    } else {
        "open"
    };

    apply(&c, &e_open).await.expect("open applies");
    apply(&c, &e_resolved).await.expect("resolve applies");
    let did_s = did.to_string();
    let s1: String = c
        .query_one(
            "SELECT state FROM chart_dispute WHERE dispute_id = $1::text::uuid",
            &[&did_s],
        )
        .await
        .unwrap()
        .get(0);

    reset_between_orders(&c).await;
    apply(&c, &e_resolved).await.expect("resolve applies");
    apply(&c, &e_open).await.expect("open applies");
    let s2: String = c
        .query_one(
            "SELECT state FROM chart_dispute WHERE dispute_id = $1::text::uuid",
            &[&did_s],
        )
        .await
        .unwrap()
        .get(0);

    assert_eq!(
        s1, s2,
        "a dispute must not settle open-vs-resolved by arrival order (#115)"
    );
    assert_eq!(
        s1, expect,
        "winner is the higher content_address, deterministically"
    );
}

/// A signed identity-pending (or identify) for the SAME subject at a chosen HLC triple.
/// pending vs identify changes the event_type (and event_id) ⇒ different content_address.
fn identity_state_event(
    kid: &str,
    subject: Uuid,
    identified: bool,
    descriptive: &str,
    wall: i64,
    counter: i32,
) -> EventBody {
    let subj = subject.to_string();
    let (etype, payload, twin, sver) = if identified {
        let a = IdentifyAssertion {
            subject: &subj,
            method: descriptive,
        };
        (
            "identity.identify.asserted",
            identify_assertion_body(&a),
            render_identify_twin(&a),
            "identity.identify.asserted/1",
        )
    } else {
        let a = PendingAssertion {
            subject: &subj,
            basis: descriptive,
        };
        (
            "identity.pending.asserted",
            pending_assertion_body(&a),
            render_pending_twin(&a),
            "identity.pending.asserted/1",
        )
    };
    EventBody {
        event_id: Uuid::now_v7().to_string(),
        patient_id: subj.clone(),
        event_type: etype.into(),
        schema_version: sver.into(),
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
async fn chart_identity_state_converges_under_hlc_collision() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid, _sk_h, _kid_h) = setup(&c).await;

    let subj = Uuid::now_v7();
    let e_pending = sign(
        &identity_state_event(&kid, subj, false, "unconscious ED arrival", 5000, 7),
        &sk,
    )
    .unwrap()
    .signed_bytes;
    let e_identified = sign(
        &identity_state_event(&kid, subj, true, "photo id matched", 5000, 7),
        &sk,
    )
    .unwrap()
    .signed_bytes;
    let expect = if event_address(&e_identified) > event_address(&e_pending) {
        "identified"
    } else {
        "pending"
    };
    let subj_s = subj.to_string();

    apply(&c, &e_pending).await.expect("pending applies");
    apply(&c, &e_identified).await.expect("identify applies");
    let s1: String = c
        .query_one(
            "SELECT state FROM chart_identity_state WHERE subject = $1::text::uuid",
            &[&subj_s],
        )
        .await
        .unwrap()
        .get(0);

    reset_between_orders(&c).await;
    apply(&c, &e_identified).await.expect("identify applies");
    apply(&c, &e_pending).await.expect("pending applies");
    let s2: String = c
        .query_one(
            "SELECT state FROM chart_identity_state WHERE subject = $1::text::uuid",
            &[&subj_s],
        )
        .await
        .unwrap()
        .get(0);

    assert_eq!(
        s1, s2,
        "identity-pending vs identified must not flip by arrival order (#115)"
    );
    assert_eq!(
        s1, expect,
        "winner is the higher content_address, deterministically"
    );
}

/// A signed repudiation of (subject, value). Two repudiations of the SAME (subject, value)
/// with different `reason` differ in signed bytes ⇒ different content_address, colliding on
/// (wall, counter, origin) only — the #115 case for the suppressing overlay.
fn repudiation_event(
    kid: &str,
    subject: Uuid,
    value: &str,
    reason: &str,
    wall: i64,
    counter: i32,
) -> EventBody {
    let subj = subject.to_string();
    let a = RepudiationAssertion {
        subject: &subj,
        value,
        reason,
    };
    EventBody {
        event_id: Uuid::now_v7().to_string(),
        patient_id: subj.clone(),
        event_type: "identity.repudiate.asserted".into(),
        schema_version: "identity.repudiate.asserted/1".into(),
        hlc: Hlc {
            wall,
            counter,
            node_origin: "peer".into(),
        },
        t_effective: None,
        signer_key_id: kid.into(),
        contributors: serde_json::json!([{"actor_id": kid, "role": "recorded"}]),
        payload: repudiation_assertion_body(&a),
        attachments: vec![],
        plaintext_twin: Some(render_repudiate_twin(&a)),
    }
}

/// Apply a validly-signed suppressing event through the attested apply door (db/020, 3-arg):
/// signed bytes + a human attestation token over its content_address + the attester's key.
async fn apply_attested(
    c: &Client,
    signed: &[u8],
    token: &[u8],
    hkey: &[u8],
) -> Result<u64, tokio_postgres::Error> {
    c.execute(
        "SELECT apply_remote_event($1, $2, $3)",
        &[&signed.to_vec(), &token.to_vec(), &hkey.to_vec()],
    )
    .await
}

#[tokio::test]
async fn name_repudiation_converges_under_hlc_collision() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid, sk_h, kid_h) = setup(&c).await;
    let hkey = hex::decode(&kid_h).unwrap();

    let subj = Uuid::now_v7();
    let value = "Fabricated Persona";
    // Two repudiations of the same struck name at an IDENTICAL HLC triple; the winning `reason`
    // must be the higher content_address either arrival order.
    let e1 = sign(
        &repudiation_event(&kid, subj, value, "reason-one", 5000, 7),
        &sk,
    )
    .unwrap()
    .signed_bytes;
    let e2 = sign(
        &repudiation_event(&kid, subj, value, "reason-two", 5000, 7),
        &sk,
    )
    .unwrap()
    .signed_bytes;
    let t1 = sign_attestation(&event_address(&e1), &kid_h, "attested", &sk_h).unwrap();
    let t2 = sign_attestation(&event_address(&e2), &kid_h, "attested", &sk_h).unwrap();
    let expect = if event_address(&e2) > event_address(&e1) {
        "reason-two"
    } else {
        "reason-one"
    };

    apply_attested(&c, &e1, &t1, &hkey)
        .await
        .expect("repudiation one applies");
    apply_attested(&c, &e2, &t2, &hkey)
        .await
        .expect("repudiation two applies");
    let subj_s = subj.to_string();
    let r1: String = c
        .query_one(
            "SELECT reason FROM name_repudiation WHERE subject = $1::text::uuid AND value = $2",
            &[&subj_s, &value],
        )
        .await
        .unwrap()
        .get(0);

    reset_between_orders(&c).await;
    apply_attested(&c, &e2, &t2, &hkey)
        .await
        .expect("repudiation two applies");
    apply_attested(&c, &e1, &t1, &hkey)
        .await
        .expect("repudiation one applies");
    let r2: String = c
        .query_one(
            "SELECT reason FROM name_repudiation WHERE subject = $1::text::uuid AND value = $2",
            &[&subj_s, &value],
        )
        .await
        .unwrap()
        .get(0);

    assert_eq!(
        r1, r2,
        "a repudiation must not settle by arrival order (#115)"
    );
    assert_eq!(
        r1, expect,
        "winner is the higher content_address, deterministically"
    );
}

/// A signed patient.amended carrying a `name` at a chosen HLC triple. Two amendments with
/// different names differ in content_address, colliding on (wall, counter, origin) only.
fn amended_event(kid: &str, patient: Uuid, name: &str, wall: i64, counter: i32) -> EventBody {
    EventBody {
        event_id: Uuid::now_v7().to_string(),
        patient_id: patient.to_string(),
        event_type: "patient.amended".into(),
        schema_version: "patient/1".into(),
        hlc: Hlc {
            wall,
            counter,
            node_origin: "peer".into(),
        },
        t_effective: None,
        signer_key_id: kid.into(),
        contributors: serde_json::json!([{"actor_id": kid, "role": "recorded"}]),
        payload: serde_json::json!({"name": name}),
        attachments: vec![],
        plaintext_twin: Some(format!("patient amended: {name}")),
    }
}

#[tokio::test]
async fn patient_chart_converges_under_hlc_collision() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid, _sk_h, _kid_h) = setup(&c).await;

    let p = Uuid::now_v7();
    let e_a = sign(&amended_event(&kid, p, "Alice A", 5000, 7), &sk)
        .unwrap()
        .signed_bytes;
    let e_b = sign(&amended_event(&kid, p, "Bob B", 5000, 7), &sk)
        .unwrap()
        .signed_bytes;
    let expect = if event_address(&e_b) > event_address(&e_a) {
        "Bob B"
    } else {
        "Alice A"
    };
    let p_s = p.to_string();

    apply(&c, &e_a).await.expect("amend A applies");
    apply(&c, &e_b).await.expect("amend B applies");
    let n1: String = c
        .query_one(
            "SELECT name FROM patient_chart WHERE patient_id = $1::text::uuid",
            &[&p_s],
        )
        .await
        .unwrap()
        .get(0);
    let sig1 = collision_rows(&c, "patient_chart").await;

    reset_between_orders(&c).await;
    apply(&c, &e_b).await.expect("amend B applies");
    apply(&c, &e_a).await.expect("amend A applies");
    let n2: String = c
        .query_one(
            "SELECT name FROM patient_chart WHERE patient_id = $1::text::uuid",
            &[&p_s],
        )
        .await
        .unwrap()
        .get(0);

    assert_eq!(
        n1, n2,
        "the demographic winner must not flip by arrival order (#115)"
    );
    assert_eq!(
        n1, expect,
        "winner is the higher content_address, deterministically"
    );

    // #157: the resolved collision is also SURFACED — exactly one advisory row, identical across
    // both arrival orders (the signal is itself a convergent set-union projection).
    let sig2 = collision_rows(&c, "patient_chart").await;
    assert_eq!(sig1.len(), 1, "one collision recorded, order 1");
    assert_eq!(sig2.len(), 1, "one collision recorded, order 2");
    assert_eq!(
        sig1, sig2,
        "the advisory signal converges across arrival order (#157)"
    );
}

#[tokio::test]
async fn distinct_triples_record_no_collision() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid, _sk_h, _kid_h) = setup(&c).await;

    // Two ordinary amendments at DIFFERENT HLC triples (wall 5000 then 5001) — a normal overlay,
    // never a Byzantine collision. No advisory row must be recorded.
    let p = Uuid::now_v7();
    let e_a = sign(&amended_event(&kid, p, "Alice A", 5000, 7), &sk)
        .unwrap()
        .signed_bytes;
    let e_b = sign(&amended_event(&kid, p, "Bob B", 5001, 0), &sk)
        .unwrap()
        .signed_bytes;
    apply(&c, &e_a).await.expect("amend A applies");
    apply(&c, &e_b).await.expect("amend B applies");

    assert!(
        collision_rows(&c, "patient_chart").await.is_empty(),
        "distinct HLC triples are normal overlay, not a Byzantine collision (#157)"
    );
}
