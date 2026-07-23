//! Integration coverage for `apply_remote_event` (db/020) — the in-DB clinical-plane
//! sync apply door (issue #91, review findings A2/A5b/M8/H4). ADR-0021 places the
//! enforcement floor BELOW the inter-node path; before db/020 the floor only guarded
//! local authors (`submit_event`) while replicated events were raw-INSERTed by the
//! daemon with owner privileges. These tests pin the door's contract:
//!
//!   * everything `submit_event` enforces holds at apply too (signature, enrollment,
//!     fail-closed classification, attestation gate, twin floor, t_effective rules,
//!     substitution guard) — ONE floor, two doors;
//!   * replication-appropriate deltas: idempotent re-apply is a silent no-op
//!     (set-union), the local HLC merges forward, and projection maintenance
//!     CLAMPS-AND-FLAGS instead of vetoing a validly-signed event peers accepted (A5b).
//!
//! Real Postgres, gated on `$CAIRN_TEST_PG`, serialized via `db::test_serial_guard`.
use cairn_event::identity::{link_assertion_body, render_link_twin, LinkAssertion};
use cairn_event::{
    event_address, generate_key, sign, sign_attestation, EventBody, Hlc, SigningKey,
};
use cairn_node::db;
use tokio_postgres::Client;
use uuid::Uuid;

fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

/// A realistic HLC wall (ms since epoch, ≈ 2026-06-21) so the t_effective ≤ t_recorded
/// ceiling tests compare against a sane "recorded" instant, not 1970.
const WALL_2026: i64 = 1_782_000_000_000;

/// The Postgres error message for a failed statement (Display renders only "db error";
/// the RAISE text lives in the DbError payload — project convention, see identity_linkage.rs).
fn db_msg(e: &tokio_postgres::Error) -> String {
    e.as_db_error()
        .map(|d| d.message().to_string())
        .unwrap_or_else(|| e.to_string())
}

/// Truncate the clinical tables and enroll one agent signer + one human attester.
async fn setup(c: &Client) -> (SigningKey, String, SigningKey, String) {
    c.batch_execute(
        "TRUNCATE event_log, actor_event, patient_chart, patient_identifier, \
         patient_demographic, patient_link, person_member, identity_projection_flag, \
         t_effective_ceiling_flag CASCADE",
    )
    .await
    .unwrap();
    c.batch_execute("UPDATE hlc_state SET hlc_wall = 0, hlc_counter = 0")
        .await
        .unwrap();
    let (sk_a, kid_a) = generate_key().unwrap();
    let (sk_h, kid_h) = generate_key().unwrap();
    c.execute(
        "SELECT enroll_actor('agent', '{\"model\":\"sync-peer-stub\",\"version\":\"1\",\"skill_epoch\":\"e\"}', $1)",
        &[&kid_a],
    ).await.unwrap();
    c.execute(
        "SELECT enroll_actor('human', '{\"role\":\"clinician\"}', $1)",
        &[&kid_h],
    )
    .await
    .unwrap();
    (sk_a, kid_a, sk_h, kid_h)
}

/// Build a signed note.added "arriving from a peer".
fn note(kid: &str, patient: Uuid, wall: i64, t_effective: Option<&str>) -> EventBody {
    EventBody {
        event_id: Uuid::now_v7().to_string(),
        patient_id: patient.to_string(),
        event_type: "note.added".into(),
        schema_version: "note/1".into(),
        hlc: Hlc {
            wall,
            counter: 0,
            node_origin: "peer".into(),
        },
        t_effective: t_effective.map(|s| s.to_string()),
        signer_key_id: kid.into(),
        contributors: serde_json::json!([{"actor_id": kid, "role": "recorded"}]),
        payload: serde_json::json!({"text": "arrived by sync"}),
        attachments: vec![],
        plaintext_twin: Some("Progress note: arrived by sync".into()),
        clock_grade: cairn_event::ClockGrade::SelfAsserted,
    }
}

async fn apply(c: &Client, signed: &[u8]) -> Result<u64, tokio_postgres::Error> {
    c.execute("SELECT apply_remote_event($1)", &[&signed.to_vec()])
        .await
}

async fn event_count(c: &Client, patient: Uuid) -> i64 {
    c.query_one(
        "SELECT count(*) FROM event_log WHERE patient_id = $1::text::uuid",
        &[&patient.to_string()],
    )
    .await
    .unwrap()
    .get(0)
}

// ---------------------------------------------------------------------------
// The floor holds at the apply door (A2).
// ---------------------------------------------------------------------------

#[tokio::test]
async fn malformed_bytes_are_refused() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    setup(&c).await;
    let err = apply(&c, b"\xde\xad\xbe\xef").await.unwrap_err();
    let m = db_msg(&err);
    assert!(
        m.contains("signature") || m.contains("verif"),
        "legible verify reject: {m}"
    );
}

#[tokio::test]
async fn unenrolled_signer_is_refused_at_apply() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    setup(&c).await;
    // A validly-signed event from a key NEVER enrolled in the local actor registry:
    // the exact bypass the raw-INSERT path allowed (review A2).
    let (sk_x, kid_x) = generate_key().unwrap();
    let p = Uuid::now_v7();
    let signed = sign(&note(&kid_x, p, WALL_2026, None), &sk_x).unwrap();
    let err = apply(&c, &signed.signed_bytes).await.unwrap_err();
    assert!(
        db_msg(&err).contains("enrolled"),
        "cites enrollment: {}",
        db_msg(&err)
    );
    assert_eq!(
        event_count(&c, p).await,
        0,
        "refused event must not be stored"
    );
}

#[tokio::test]
async fn unknown_event_type_is_refused_fail_closed() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid, _, _) = setup(&c).await;
    let p = Uuid::now_v7();
    let mut b = note(&kid, p, WALL_2026, None);
    b.event_type = "mystery.op".into();
    let signed = sign(&b, &sk).unwrap();
    let err = apply(&c, &signed.signed_bytes).await.unwrap_err();
    assert!(
        db_msg(&err).contains("fail closed"),
        "unknown type fails closed: {}",
        db_msg(&err)
    );
}

#[tokio::test]
async fn enrolled_signer_event_applies_projects_and_merges_hlc() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid, _, _) = setup(&c).await;
    let p = Uuid::now_v7();
    let mut b = note(&kid, p, WALL_2026, None);
    b.event_type = "patient.created".into();
    b.schema_version = "patient/1".into();
    b.payload = serde_json::json!({"name": "Synced Patient", "dob": "1980-01-01", "sex": "U"});
    let signed = sign(&b, &sk).unwrap();
    apply(&c, &signed.signed_bytes)
        .await
        .expect("valid remote event applies");

    assert_eq!(event_count(&c, p).await, 1);
    // Projection trigger ran (same trigger as the local door — one maintenance path).
    let name: String = c
        .query_one(
            "SELECT name FROM patient_chart WHERE patient_id = $1::text::uuid",
            &[&p.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(name, "Synced Patient");
    // The local clock never falls behind an event we accepted (HLC merge, A3 invariant).
    let wall: i64 = c
        .query_one("SELECT hlc_wall FROM hlc_state", &[])
        .await
        .unwrap()
        .get(0);
    assert!(wall >= WALL_2026, "hlc_state merged forward: {wall}");

    // Idempotent re-apply is a silent no-op (set-union), never an error.
    apply(&c, &signed.signed_bytes)
        .await
        .expect("re-apply of the same bytes is idempotent");
    assert_eq!(event_count(&c, p).await, 1, "no duplicate row on re-apply");
}

#[tokio::test]
async fn event_id_substitution_is_refused_at_apply() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid, _, _) = setup(&c).await;
    let p = Uuid::now_v7();
    let b1 = note(&kid, p, WALL_2026, None);
    let mut b2 = note(&kid, p, WALL_2026 + 1, None);
    b2.event_id = b1.event_id.clone(); // hostile/buggy reuse of an existing event_id
    apply(&c, &sign(&b1, &sk).unwrap().signed_bytes)
        .await
        .unwrap();
    let err = apply(&c, &sign(&b2, &sk).unwrap().signed_bytes)
        .await
        .unwrap_err();
    assert!(
        db_msg(&err).contains("substitution"),
        "substitution refused: {}",
        db_msg(&err)
    );
}

// ---------------------------------------------------------------------------
// The attestation gate holds for replicated suppressing events (A2's sharpest tooth).
// ---------------------------------------------------------------------------

#[tokio::test]
async fn unattested_suppress_is_refused_at_apply() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid, _, _) = setup(&c).await;
    let p = Uuid::now_v7();
    let target = note(&kid, p, WALL_2026, None);
    apply(&c, &sign(&target, &sk).unwrap().signed_bytes)
        .await
        .unwrap();

    let mut sup = note(&kid, p, WALL_2026 + 10, None);
    sup.event_type = "visibility.suppress".into();
    sup.payload = serde_json::json!({"target_event_id": target.event_id});
    let signed = sign(&sup, &sk).unwrap();
    // No attestation token travelled with it → refused, exactly as submit_event would.
    let err = apply(&c, &signed.signed_bytes).await.unwrap_err();
    assert!(
        db_msg(&err).contains("attestation"),
        "un-attested suppress refused: {}",
        db_msg(&err)
    );
    assert_eq!(event_count(&c, p).await, 1, "the suppress did not land");
}

#[tokio::test]
async fn attested_suppress_applies_and_stores_the_token() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid, sk_h, kid_h) = setup(&c).await;
    let p = Uuid::now_v7();
    let target = note(&kid, p, WALL_2026, None);
    apply(&c, &sign(&target, &sk).unwrap().signed_bytes)
        .await
        .unwrap();

    let mut sup = note(&kid, p, WALL_2026 + 10, None);
    sup.event_type = "visibility.suppress".into();
    sup.payload = serde_json::json!({"target_event_id": target.event_id});
    let signed = sign(&sup, &sk).unwrap();
    let ca = event_address(&signed.signed_bytes);
    let token = sign_attestation(&ca, &kid_h, "attested", &sk_h).unwrap();
    let hkey = hex::decode(&kid_h).unwrap();
    c.execute(
        "SELECT apply_remote_event($1, $2, $3)",
        &[&signed.signed_bytes, &token, &hkey],
    )
    .await
    .expect("attested suppress applies");

    // The responsibility proof is STORED so this node can re-serve it to its own
    // peers (the attestation must keep travelling with the event, M7 residual).
    let (att, akey): (Option<Vec<u8>>, Option<Vec<u8>>) =
        {
            let row = c.query_one(
            "SELECT attestation, attester_key FROM event_log WHERE event_id = $1::text::uuid",
            &[&sup.event_id],
        ).await.unwrap();
            (row.get(0), row.get(1))
        };
    assert_eq!(
        att.as_deref(),
        Some(&token[..]),
        "attestation token stored verbatim"
    );
    assert_eq!(akey.as_deref(), Some(&hkey[..]), "attester key stored");
}

#[tokio::test]
async fn suppress_targeting_unknown_event_is_refused() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid, sk_h, kid_h) = setup(&c).await;
    let p = Uuid::now_v7();
    let mut sup = note(&kid, p, WALL_2026, None);
    sup.event_type = "visibility.suppress".into();
    sup.payload = serde_json::json!({"target_event_id": Uuid::now_v7().to_string()});
    let signed = sign(&sup, &sk).unwrap();
    let ca = event_address(&signed.signed_bytes);
    let token = sign_attestation(&ca, &kid_h, "attested", &sk_h).unwrap();
    let hkey = hex::decode(&kid_h).unwrap();
    let err = c
        .execute(
            "SELECT apply_remote_event($1, $2, $3)",
            &[&signed.signed_bytes, &token, &hkey],
        )
        .await
        .expect_err("orphan suppress refused (retried once the target arrives)");
    assert!(
        db_msg(&err).contains("unknown event"),
        "cites the target: {}",
        db_msg(&err)
    );
}

// ---------------------------------------------------------------------------
// M8 symmetry: the same twin floor runs at both doors.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn twinless_demographic_is_refused_at_apply_like_submit() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid, _, _) = setup(&c).await;
    let p = Uuid::now_v7();
    // Structurally valid identifier assertion, authored twin DROPPED. submit_event
    // hard-rejects this (ADR-0034); before db/020 the sync path accepted it with a
    // locally-derived twin — the M8 asymmetry this test closes.
    let body = EventBody {
        event_id: Uuid::now_v7().to_string(),
        patient_id: p.to_string(),
        event_type: "demographic.identifier.asserted".into(),
        schema_version: "demographic.identifier/1".into(),
        hlc: Hlc {
            wall: WALL_2026,
            counter: 0,
            node_origin: "peer".into(),
        },
        t_effective: None,
        signer_key_id: kid.clone(),
        contributors: serde_json::json!([{"actor_id": kid, "role": "recorded"}]),
        payload: serde_json::json!({
            "value": "943 476 5919", "system": "nhs-number", "provenance": "document-verified",
            "normalized": "9434765919", "profile": "nhs-number@b3-abc", "use": "national-id"
        }),
        attachments: vec![],
        plaintext_twin: None,
        clock_grade: cairn_event::ClockGrade::SelfAsserted,
    };
    let signed = sign(&body, &sk).unwrap();
    let err = apply(&c, &signed.signed_bytes).await.unwrap_err();
    let m = db_msg(&err);
    assert!(
        m.contains("authored twin") || m.contains("§4.5"),
        "cites the twin floor: {m}"
    );
    assert_eq!(event_count(&c, p).await, 0);
}

#[tokio::test]
async fn twinless_note_derives_the_same_skeleton_at_both_doors() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid, _, _) = setup(&c).await;
    // The SAME twin-less body applied at the sync door vs submitted at the local door
    // must store the IDENTICAL derived twin (single-source floor renderer, M8): two
    // fresh events differing only in event_id, one through each door.
    let p = Uuid::now_v7();
    let mut b1 = note(&kid, p, WALL_2026, None);
    b1.plaintext_twin = None;
    let mut b2 = b1.clone();
    b2.event_id = Uuid::now_v7().to_string();
    apply(&c, &sign(&b1, &sk).unwrap().signed_bytes)
        .await
        .unwrap();
    c.execute(
        "SELECT submit_event($1)",
        &[&sign(&b2, &sk).unwrap().signed_bytes],
    )
    .await
    .unwrap();
    let rows = c
        .query(
            "SELECT plaintext_twin FROM event_log WHERE patient_id = $1::text::uuid",
            &[&p.to_string()],
        )
        .await
        .unwrap();
    assert_eq!(rows.len(), 2);
    let t1: String = rows[0].get(0);
    let t2: String = rows[1].get(0);
    assert_eq!(
        t1, t2,
        "both doors derive via the one in-DB skeleton renderer"
    );
}

// ---------------------------------------------------------------------------
// H4 + A3: the t_effective wire pin and the t_recorded ceiling, at both doors.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn offsetless_t_effective_is_refused_at_both_doors() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid, _, _) = setup(&c).await;
    let p = Uuid::now_v7();
    // No UTC offset: this string names a DIFFERENT instant on differently-configured
    // nodes (session TimeZone), so it is a wire-conformance failure (H4) — fail closed.
    let b = note(&kid, p, WALL_2026, Some("2026-06-20T10:00:00"));
    let signed = sign(&b, &sk).unwrap();
    let err = apply(&c, &signed.signed_bytes).await.unwrap_err();
    assert!(
        db_msg(&err).contains("offset"),
        "apply door cites the offset pin: {}",
        db_msg(&err)
    );
    let err = c
        .execute("SELECT submit_event($1)", &[&signed.signed_bytes])
        .await
        .unwrap_err();
    assert!(
        db_msg(&err).contains("offset"),
        "authoring door too: {}",
        db_msg(&err)
    );
    assert_eq!(event_count(&c, p).await, 0);
}

#[tokio::test]
async fn explicit_offset_t_effective_is_stored_timezone_independently() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid, _, _) = setup(&c).await;
    // A hostile-to-parsing session config: before the pin, the cast of an offset-less
    // string depended on these GUCs. With the explicit offset the stored instant is
    // identical on every node regardless of them.
    c.batch_execute("SET TimeZone = 'America/New_York'; SET DateStyle = 'German, DMY'")
        .await
        .unwrap();
    let p = Uuid::now_v7();
    let b = note(&kid, p, WALL_2026, Some("2026-06-20T10:00:00+02:00"));
    apply(&c, &sign(&b, &sk).unwrap().signed_bytes)
        .await
        .expect("explicit offset accepted");
    let same_instant: bool = c
        .query_one(
            "SELECT t_effective = '2026-06-20T08:00:00Z'::timestamptz \
         FROM event_log WHERE patient_id = $1::text::uuid",
            &[&p.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert!(
        same_instant,
        "stored t_effective is the offset-corrected UTC instant"
    );
    c.batch_execute("RESET TimeZone; RESET DateStyle")
        .await
        .unwrap();
}

#[tokio::test]
async fn forward_dated_t_effective_is_admitted_and_flagged_at_apply() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid, _, _) = setup(&c).await;
    let p = Uuid::now_v7();
    // t_effective AFTER the event's own t_recorded ceiling, self-asserted grade (this
    // fixture's grade — see `note`). Pre-ADR-0058 this unconditionally RAISEd here
    // (prima-facie falsification, ADR-0003 tier-1); the LENIENT remote door must now
    // NEVER reject on the ceiling (issue #216 F1/F2): a refusal of a verifiable event
    // freezes the puller's seq watermark and WEDGES clinical sync from one bad/forward
    // clock — see the do_pull regression in
    // crates/cairn-sync/tests/clinical_pull.rs::forward_dated_event_does_not_wedge_the_pull.
    // Admitted unchanged instead, and recorded as an advisory ceiling-flag row.
    let b = note(&kid, p, 1_000_000_000_000, Some("2026-06-20T10:00:00Z")); // recorded 2001
    let signed = sign(&b, &sk).unwrap();
    let ca = event_address(&signed.signed_bytes);
    apply(&c, &signed.signed_bytes).await.expect(
        "a forward-dated t_effective must be admitted, never rejected (ADR-0058 lenient door)",
    );
    assert_eq!(
        event_count(&c, p).await,
        1,
        "the admitted forward-dated event must be appended"
    );
    let flags: i64 = c
        .query_one(
            "SELECT count(*) FROM t_effective_ceiling_flag WHERE content_address = $1",
            &[&ca],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        flags, 1,
        "a forward-dated event must be recorded as an advisory ceiling flag, never rejected"
    );
}

// ---------------------------------------------------------------------------
// A5b: projection maintenance clamps-and-flags at apply, still vetoes local authoring.
// ---------------------------------------------------------------------------

fn link_event(kid: &str, a: Uuid, b: Uuid, wall: i64) -> EventBody {
    let a_s = a.to_string();
    let b_s = b.to_string();
    let la = LinkAssertion {
        subject_a: &a_s,
        subject_b: &b_s,
        provenance: "matcher:cfg@test",
        confidence: None,
    };
    EventBody {
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
        contributors: serde_json::json!([{"actor_id": kid, "role": "recorded"}]),
        payload: link_assertion_body(&la),
        attachments: vec![],
        plaintext_twin: Some(render_link_twin(&la)),
        clock_grade: cairn_event::ClockGrade::SelfAsserted,
    }
}

#[tokio::test]
async fn oversize_component_clamps_and_flags_at_apply_never_vetoes() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid, _, _) = setup(&c).await;
    c.batch_execute("SET cairn.max_component_size = 2")
        .await
        .unwrap();
    let (a, b, cc) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());

    // {A,B} = 2 ≤ cap: applies and projects normally.
    let l1 = link_event(&kid, a, b, WALL_2026);
    apply(&c, &sign(&l1, &sk).unwrap().signed_bytes)
        .await
        .expect("at-cap link applies");
    let n: i64 = c
        .query_one("SELECT count(*) FROM person_member", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(n, 2, "in-cap component projected");

    // {A,B,C} = 3 > cap arriving BY SYNC: peers already accepted this validly-signed
    // event, so a node-local projection cap must never refuse it (that would fork the
    // event set on a config difference — the A5b divergence). The event lands; the
    // projection recompute is SKIPPED (clamped) and the pathology is flagged loudly.
    let l2 = link_event(&kid, b, cc, WALL_2026 + 10);
    apply(&c, &sign(&l2, &sk).unwrap().signed_bytes)
        .await
        .expect("oversize component must not veto a replicated event (A5b)");
    let stored: i64 = c
        .query_one(
            "SELECT count(*) FROM event_log WHERE event_type = 'identity.link.asserted'",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(stored, 2, "the replicated link event IS in the log");
    let flags: i64 = c
        .query_one(
            "SELECT count(*) FROM identity_projection_flag WHERE observed_size > cap",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert!(
        flags >= 1,
        "the skipped recompute left a loud worklist flag"
    );

    // Local authoring keeps the fail-loud veto: the same pathology REFUSED at submit
    // (nothing accepted anywhere yet, so refusal loses no data).
    let (d, e_) = (Uuid::now_v7(), Uuid::now_v7());
    let l3 = link_event(&kid, d, e_, WALL_2026 + 20);
    c.execute(
        "SELECT submit_event($1)",
        &[&sign(&l3, &sk).unwrap().signed_bytes],
    )
    .await
    .expect("fresh 2-member link fine locally");
    let l4 = link_event(&kid, e_, cc, WALL_2026 + 30); // joins {D,E} to {A,B,C} → 5 > 2
    let err = c
        .execute(
            "SELECT submit_event($1)",
            &[&sign(&l4, &sk).unwrap().signed_bytes],
        )
        .await
        .unwrap_err();
    assert!(
        db_msg(&err).contains("exceeds max size"),
        "local authoring still vetoes fail-loud: {}",
        db_msg(&err)
    );
    c.batch_execute("RESET cairn.max_component_size")
        .await
        .unwrap();
}

// ---------------------------------------------------------------------------
// Size ceiling (A7a) at the apply door.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn oversized_event_is_refused_at_apply() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid, _, _) = setup(&c).await;
    let p = Uuid::now_v7();
    let mut b = note(&kid, p, WALL_2026, None);
    // Push the signed envelope past cairn_max_event_bytes() (8,000,000).
    b.payload = serde_json::json!({"text": "x".repeat(8_100_000)});
    let signed = sign(&b, &sk).unwrap();
    let err = apply(&c, &signed.signed_bytes).await.unwrap_err();
    assert!(
        db_msg(&err).contains("ceiling"),
        "cites the admission ceiling: {}",
        db_msg(&err)
    );
}
