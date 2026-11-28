//! ADR-0115 — a patient is marked synthetic at registration, and no link crosses the line.
//!
//! Two things are pinned here, and they are tested together because they are not safe apart:
//! a mark with no rule about linking would let an invented chart be joined to a real person's.
//!
//! **The mark** (plan Task 1). It is a field of the registration event. `cairn_chart_synthetic`
//! is the ONE place SQL reads it, and it has three answers, never two: `synthetic`, `real`, and
//! `unknown` for a chart whose registration is not held here (a peer's clinical event
//! legitimately arrives before the registration that licenses it; principle 4).
//!
//! **The line** (plan Task 2). A link between a `synthetic` chart and a `real` one is:
//!   * REFUSED at the local door, whoever signs it. This is deliberately stronger than the
//!     §5.2 hard veto, which a human attestation lifts (`link_veto_floor.rs`): no person
//!     should be able to say an invented patient is a real one;
//!   * ADMITTED at the remote door (a node cannot refuse a peer's event without forking the
//!     event set) and flagged;
//!   * and, whatever was admitted, never part of a person's chart set: `cairn_person_charts`
//!     decides at READ time, so a registration that arrives after the link still cuts it.
//!
//! Real Postgres, gated on `$CAIRN_TEST_PG`, serialized via `db::test_serial_guard`.

use cairn_event::identity::{link_assertion_body, render_link_twin, LinkAssertion};
use cairn_event::registration::{
    registration_assertion_body, render_registration_twin, RegistrationAssertion,
    RegistrationClass, REGISTRATION_EVENT_TYPE, REGISTRATION_SCHEMA_VERSION,
};
use cairn_event::{
    event_address, generate_key, sign, sign_attestation, EventBody, Hlc, SigningKey,
};
use cairn_node::db;
use serde_json::Value;
use tokio_postgres::Client;
use uuid::Uuid;

fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

fn db_msg(e: &tokio_postgres::Error) -> String {
    e.as_db_error()
        .map(|d| d.message().to_string())
        .unwrap_or_else(|| e.to_string())
}

/// A clean slate, one enrolled agent (the recorder) and one enrolled human (the attester).
async fn setup(c: &Client) -> (SigningKey, String, SigningKey, String) {
    c.batch_execute(
        "TRUNCATE event_log, actor_event, patient_chart, patient_identifier, \
         patient_demographic, patient_name, patient_link, person_member, \
         identity_projection_flag, link_veto_flag CASCADE",
    )
    .await
    .unwrap();
    let (sk_a, kid_a) = generate_key().unwrap();
    let (sk_h, kid_h) = generate_key().unwrap();
    c.execute(
        "SELECT enroll_actor('agent', '{\"model\":\"syn-stub\",\"version\":\"1\",\"skill_epoch\":\"e\"}', $1)",
        &[&kid_a],
    ).await.unwrap();
    c.execute(
        "SELECT enroll_actor('human', '{\"role\":\"records-officer\",\"actor\":\"H\"}', $1)",
        &[&kid_h],
    )
    .await
    .unwrap();
    (sk_a, kid_a, sk_h, kid_h)
}

/// A signed registration of `patient`. `payload_edit` lets a test put something on the wire
/// that the Rust builder would never write (an explicit `false`, a string).
fn registration(
    sk: &SigningKey,
    kid: &str,
    patient: Uuid,
    synthetic: bool,
    wall: i64,
    payload_edit: impl FnOnce(&mut Value),
) -> Vec<u8> {
    // Unidentified, so no search attestation is needed: the mark is independent of the
    // class, and the search rules are `patient_registration.rs`'s business, not this file's.
    let a = RegistrationAssertion {
        class: RegistrationClass::Unidentified,
        synthetic,
        basis: Some("fixture"),
        search: None,
    };
    let mut payload = registration_assertion_body(&a);
    payload_edit(&mut payload);
    let body = EventBody {
        event_id: Uuid::now_v7().to_string(),
        patient_id: patient.to_string(),
        event_type: REGISTRATION_EVENT_TYPE.into(),
        schema_version: REGISTRATION_SCHEMA_VERSION.into(),
        hlc: Hlc {
            wall,
            counter: 0,
            node_origin: "peer".into(),
        },
        t_effective: None,
        signer_key_id: kid.into(),
        contributors: serde_json::json!([{"actor_id": kid, "role": "recorded"}]),
        payload,
        attachments: vec![],
        plaintext_twin: Some(render_registration_twin(&a)),
        clock_grade: cairn_event::ClockGrade::SelfAsserted,
        safety: None,
    };
    sign(&body, sk).unwrap().signed_bytes
}

/// Register `patient` through the local door and return its id.
async fn register(c: &Client, sk: &SigningKey, kid: &str, synthetic: bool) -> Uuid {
    let patient = Uuid::now_v7();
    let signed = registration(sk, kid, patient, synthetic, 1, |_| {});
    c.execute("SELECT submit_event($1)", &[&signed])
        .await
        .expect("a well-formed registration is accepted");
    patient
}

async fn chart_synthetic(c: &Client, patient: Uuid) -> String {
    c.query_one(
        "SELECT cairn_chart_synthetic($1::text::uuid)",
        &[&patient.to_string()],
    )
    .await
    .unwrap()
    .get(0)
}

/// A link event over (a, b). With `human`, the signer also holds responsibility, which is
/// the shape a human-attested link has (`link_veto_floor.rs`).
fn link(kid: &str, a: Uuid, b: Uuid, wall: i64, human: bool, sk: &SigningKey) -> Vec<u8> {
    let (a_s, b_s) = (a.to_string(), b.to_string());
    let la = LinkAssertion {
        subject_a: &a_s,
        subject_b: &b_s,
        provenance: "syn:test",
        confidence: None,
    };
    let contributors = if human {
        serde_json::json!([{"actor_id": kid, "role": "attested", "responsibility": {"held_by": kid}}])
    } else {
        serde_json::json!([{"actor_id": kid, "role": "recorded"}])
    };
    let body = EventBody {
        event_id: Uuid::now_v7().to_string(),
        patient_id: a_s.clone(),
        event_type: "identity.link.asserted".into(),
        schema_version: "identity.link/1".into(),
        hlc: Hlc {
            wall,
            counter: 0,
            node_origin: "peer".into(),
        },
        t_effective: None,
        signer_key_id: kid.into(),
        contributors,
        payload: link_assertion_body(&la),
        attachments: vec![],
        plaintext_twin: Some(render_link_twin(&la)),
        clock_grade: cairn_event::ClockGrade::SelfAsserted,
        safety: None,
    };
    sign(&body, sk).unwrap().signed_bytes
}

/// The charts `cairn_person_charts` reads as one person with `patient`.
async fn person_charts(c: &Client, patient: Uuid) -> Vec<String> {
    c.query(
        "SELECT x::text FROM cairn_person_charts($1::text::uuid) AS x ORDER BY 1",
        &[&patient.to_string()],
    )
    .await
    .unwrap()
    .iter()
    .map(|r| r.get(0))
    .collect()
}

async fn open(base: &str) -> Client {
    db::connect_and_load_schema(base).await.unwrap()
}

// ---------------------------------------------------------------------------------------
// The mark
// ---------------------------------------------------------------------------------------

#[tokio::test]
async fn a_chart_is_synthetic_real_or_unknown_here() {
    let Some(base) = cs() else { return };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = open(&base).await;
    let (sk, kid, _, _) = setup(&c).await;

    let invented = register(&c, &sk, &kid, true).await;
    let person = register(&c, &sk, &kid, false).await;
    let never_registered_here = Uuid::now_v7();

    assert_eq!(chart_synthetic(&c, invented).await, "synthetic");
    assert_eq!(chart_synthetic(&c, person).await, "real");
    // NOT "real": this node has no registration for it, so it does not know (principle 4).
    assert_eq!(chart_synthetic(&c, never_registered_here).await, "unknown");
}

/// "Real" has one spelling, which is the field's absence. A second spelling would let two
/// clients disagree about what they wrote, and the floor says so at the door.
#[tokio::test]
async fn the_local_door_refuses_any_spelling_of_the_mark_but_true() {
    let Some(base) = cs() else { return };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = open(&base).await;
    let (sk, kid, _, _) = setup(&c).await;

    for wrong in [
        serde_json::json!(false),
        serde_json::json!("yes"),
        serde_json::json!(1),
        Value::Null,
    ] {
        let patient = Uuid::now_v7();
        let shown = wrong.to_string();
        let signed = registration(&sk, &kid, patient, false, 1, |p| {
            p["synthetic"] = wrong;
        });
        let err = c
            .execute("SELECT submit_event($1)", &[&signed])
            .await
            .expect_err("a mark that is not JSON true must be refused locally");
        assert!(
            db_msg(&err).contains("synthetic"),
            "the refusal names the field, for {shown}: {}",
            db_msg(&err)
        );
        assert_eq!(
            chart_synthetic(&c, patient).await,
            "unknown",
            "nothing registered"
        );
    }
}

/// The remote door must not refuse what an older build already admitted (it knows nothing of
/// the field and takes any value: `registration_tolerates_an_unknown_field.rs`). So a
/// malformed mark is admitted there, and read in the VISIBLE direction: a registration that
/// says anything about being synthetic, other than an explicit `false`, is shown as synthetic.
#[tokio::test]
async fn the_remote_door_admits_a_malformed_mark_and_reads_it_as_marked() {
    let Some(base) = cs() else { return };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = open(&base).await;
    let (sk, kid, _, _) = setup(&c).await;

    let odd = Uuid::now_v7();
    let signed = registration(&sk, &kid, odd, false, 1, |p| {
        p["synthetic"] = serde_json::json!("yes");
    });
    c.execute("SELECT apply_remote_event($1)", &[&signed])
        .await
        .expect("the remote door admits it: a refusal would fork the event set");
    assert_eq!(chart_synthetic(&c, odd).await, "synthetic");

    let said_false = Uuid::now_v7();
    let signed = registration(&sk, &kid, said_false, false, 1, |p| {
        p["synthetic"] = serde_json::json!(false);
    });
    c.execute("SELECT apply_remote_event($1)", &[&signed])
        .await
        .expect("admitted");
    assert_eq!(chart_synthetic(&c, said_false).await, "real");
}

/// Two registrations of one chart that disagree. The EARLIEST is the chart's birth act for
/// every other fact (db/045 section 6), so it is for this one, in either arrival order; and
/// the disagreement is reported, never hidden.
#[tokio::test]
async fn two_registrations_that_disagree_read_the_earliest_and_are_reported() {
    let Some(base) = cs() else { return };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = open(&base).await;
    let (sk, kid, _, _) = setup(&c).await;

    for (first_applied_is_later, expect) in [(true, "synthetic"), (false, "synthetic")] {
        let chart = Uuid::now_v7();
        let born_synthetic = registration(&sk, &kid, chart, true, 10, |_| {});
        let later_says_real = registration(&sk, &kid, chart, false, 20, |_| {});
        let order = if first_applied_is_later {
            [&later_says_real, &born_synthetic]
        } else {
            [&born_synthetic, &later_says_real]
        };
        for signed in order {
            c.execute("SELECT apply_remote_event($1)", &[signed])
                .await
                .expect("both registrations are admitted");
        }
        assert_eq!(chart_synthetic(&c, chart).await, expect);
        let conflict: bool = c
            .query_one(
                "SELECT cairn_chart_synthetic_conflict($1::text::uuid)",
                &[&chart.to_string()],
            )
            .await
            .unwrap()
            .get(0);
        assert!(
            conflict,
            "a chart whose registrations disagree must be reportable"
        );
    }
}

/// An older build stores a marked registration without understanding it. When that node is
/// upgraded, reprojecting from the log must produce the mark: the fact was never lost.
#[tokio::test]
async fn a_reprojection_rebuilds_the_mark_from_the_log() {
    let Some(base) = cs() else { return };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = open(&base).await;
    let (sk, kid, _, _) = setup(&c).await;
    let invented = register(&c, &sk, &kid, true).await;

    // What an older build's projection holds: the row, without the mark.
    c.batch_execute("UPDATE patient_registration SET synthetic = false")
        .await
        .unwrap();
    assert_eq!(chart_synthetic(&c, invented).await, "real");

    // HEAL mode (p_rebuild = false): every event is applied again over the rows that are
    // there. It is what a connect runs after an upgrade (ADR-0057), so it is the path by
    // which the mark actually reaches an upgraded node, and it is why db/045's apply
    // function updates this one column on conflict instead of doing nothing.
    c.batch_execute("SELECT cairn_reproject('identity.registration.asserted', false, 'test')")
        .await
        .expect("reprojection runs");
    assert_eq!(chart_synthetic(&c, invented).await, "synthetic");
}

/// What the one-patient commands print first (`medication-list`, `patient-sensitivity`,
/// `patient-safety`, `duplicate-check`): they read the kind through `patient::kind`, and a
/// real chart gets no line at all.
#[tokio::test]
async fn the_one_patient_commands_read_the_kind_and_banner_all_but_a_real_chart() {
    use cairn_node::patient::kind::{chart_kind, kind_banner};
    use cairn_patient_search::PatientKind;
    let Some(base) = cs() else { return };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = open(&base).await;
    let (sk, kid, _, _) = setup(&c).await;
    let invented = register(&c, &sk, &kid, true).await;
    let person = register(&c, &sk, &kid, false).await;

    assert_eq!(
        chart_kind(&c, invented).await.unwrap(),
        PatientKind::Synthetic
    );
    assert_eq!(chart_kind(&c, person).await.unwrap(), PatientKind::Real);
    let stranger = chart_kind(&c, Uuid::now_v7()).await.unwrap();
    assert_eq!(stranger, PatientKind::Unknown);

    assert!(kind_banner(chart_kind(&c, invented).await.unwrap()).is_some());
    assert!(kind_banner(stranger).is_some(), "unknown is never silent");
    assert_eq!(kind_banner(chart_kind(&c, person).await.unwrap()), None);
}

/// What a clerk is shown. A search candidate carries the chart's kind, read through the one
/// function that knows, so every surface built on a candidate can mark it (plan Task 5).
#[tokio::test]
async fn a_search_candidate_says_what_kind_of_chart_it_is() {
    use cairn_patient_search::PatientKind;
    let Some(base) = cs() else { return };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = open(&base).await;
    let (sk, kid, _, _) = setup(&c).await;
    let invented = register(&c, &sk, &kid, true).await;
    let person = register(&c, &sk, &kid, false).await;
    let not_held_here = Uuid::now_v7();

    let shown = cairn_node::patient::candidate_read::candidates_by_id(
        &c,
        &[invented, person, not_held_here],
        "2026-11-28",
    )
    .await
    .unwrap();
    let kinds: Vec<PatientKind> = shown.iter().map(|x| x.kind).collect();
    assert_eq!(
        kinds,
        [
            PatientKind::Synthetic,
            PatientKind::Real,
            PatientKind::Unknown
        ],
        "a chart whose registration is not held here is Unknown, never Real"
    );
}

// ---------------------------------------------------------------------------------------
// The line
// ---------------------------------------------------------------------------------------

#[tokio::test]
async fn a_link_across_the_line_is_refused_locally_even_when_a_human_attests_it() {
    let Some(base) = cs() else { return };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = open(&base).await;
    let (sk_a, kid_a, sk_h, kid_h) = setup(&c).await;
    let invented = register(&c, &sk_a, &kid_a, true).await;
    let person = register(&c, &sk_a, &kid_a, false).await;

    // Un-attested, in both subject orders.
    for (x, y) in [(invented, person), (person, invented)] {
        let err = c
            .execute(
                "SELECT submit_event($1)",
                &[&link(&kid_a, x, y, 10, false, &sk_a)],
            )
            .await
            .expect_err("an agent may not link across the line");
        assert!(db_msg(&err).contains("synthetic"), "{}", db_msg(&err));
    }

    // Human-attested: the §5.2 veto would let this through. This must not.
    let signed = link(&kid_h, invented, person, 11, true, &sk_h);
    let token = sign_attestation(&event_address(&signed), &kid_h, "attested", &sk_h).unwrap();
    let vk = sk_h.verifying_key().to_bytes().to_vec();
    let err = c
        .execute("SELECT submit_event($1,$2,$3)", &[&signed, &token, &vk])
        .await
        .expect_err("no signature lifts this refusal");
    let msg = db_msg(&err);
    assert!(msg.contains("synthetic"), "{msg}");
    assert!(
        msg.contains("no signature"),
        "the refusal must say an attestation does not help, or the human will try again: {msg}"
    );

    assert_eq!(person_charts(&c, person).await, [person.to_string()]);
    assert_eq!(person_charts(&c, invented).await, [invented.to_string()]);
}

/// Duplicate-repair training needs two invented charts that can be linked. The lazy
/// implementation refuses anything synthetic; this is the test that it does not.
#[tokio::test]
async fn two_synthetic_charts_link_as_any_two_charts_do() {
    let Some(base) = cs() else { return };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = open(&base).await;
    let (sk, kid, _, _) = setup(&c).await;
    let a = register(&c, &sk, &kid, true).await;
    let b = register(&c, &sk, &kid, true).await;

    c.execute(
        "SELECT submit_event($1)",
        &[&link(&kid, a, b, 10, false, &sk)],
    )
    .await
    .expect("two synthetic charts may be linked");
    let mut both = [a.to_string(), b.to_string()];
    both.sort();
    assert_eq!(person_charts(&c, a).await, both);

    // And two real ones, unchanged by any of this.
    let p = register(&c, &sk, &kid, false).await;
    let q = register(&c, &sk, &kid, false).await;
    c.execute(
        "SELECT submit_event($1)",
        &[&link(&kid, p, q, 10, false, &sk)],
    )
    .await
    .expect("two real charts link as they always did");
}

#[tokio::test]
async fn a_peers_link_across_the_line_is_admitted_flagged_and_never_joined() {
    let Some(base) = cs() else { return };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = open(&base).await;
    let (sk, kid, _, _) = setup(&c).await;
    let invented = register(&c, &sk, &kid, true).await;
    let person = register(&c, &sk, &kid, false).await;

    c.execute(
        "SELECT apply_remote_event($1)",
        &[&link(&kid, invented, person, 10, false, &sk)],
    )
    .await
    .expect("the remote door admits it: this node cannot refuse a peer's event");

    let flags: i64 = c
        .query_one("SELECT count(*) FROM link_veto_flag", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        flags, 1,
        "a link across the line is on the worklist, never silent"
    );

    // The floor under both doors: whatever was admitted, the read never mixes them.
    assert_eq!(person_charts(&c, person).await, [person.to_string()]);
    assert_eq!(person_charts(&c, invented).await, [invented.to_string()]);
    let veto: bool = c
        .query_one(
            "SELECT cairn_has_hard_veto($1::text::uuid, $2::text::uuid)",
            &[&invented.to_string(), &person.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert!(
        veto,
        "every existing reader of the hard veto must refuse this pair too"
    );
}

/// The #220 shape. The link arrives while one chart's registration is not yet held, so no
/// door can know it crosses the line. The read must cut it once the registration arrives.
#[tokio::test]
async fn a_registration_that_arrives_after_the_link_still_cuts_it() {
    let Some(base) = cs() else { return };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = open(&base).await;
    let (sk, kid, _, _) = setup(&c).await;
    let person = register(&c, &sk, &kid, false).await;
    let not_yet_known = Uuid::now_v7();

    c.execute(
        "SELECT apply_remote_event($1)",
        &[&link(&kid, person, not_yet_known, 10, false, &sk)],
    )
    .await
    .expect("admitted: nothing is known against it yet");
    let mut both = [person.to_string(), not_yet_known.to_string()];
    both.sort();
    assert_eq!(
        person_charts(&c, person).await,
        both,
        "an unknown chart is not treated as across the line: a door must not act on what it does not know"
    );

    // Its registration arrives, and it is synthetic.
    let signed = registration(&sk, &kid, not_yet_known, true, 5, |_| {});
    c.execute("SELECT apply_remote_event($1)", &[&signed])
        .await
        .expect("the registration is admitted");

    assert_eq!(person_charts(&c, person).await, [person.to_string()]);
    assert_eq!(
        person_charts(&c, not_yet_known).await,
        [not_yet_known.to_string()]
    );
}
