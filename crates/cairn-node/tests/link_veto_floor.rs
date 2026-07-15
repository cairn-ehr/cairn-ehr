//! Issue #190 (2026-07-15 review, finding A2) — the db/016 hard-veto faces the DOOR.
//!
//! The only caller of `cairn_has_hard_veto` was the Rust auto-apply seam
//! (auto_apply.rs), so the ADR-0030 threat actor — a hostile-but-enrolled agent with
//! `cairn_agent` + raw SQL — could author `identity.link.asserted` for two patients
//! with a trustworthy-identifier/DOB clash straight through `submit_event`:
//! `person_member` merged the charts and no trust state was even flagged. That is the
//! "veto is L2-convention, not floor" gap principle 12 forbids.
//!
//! The floor contract (db/016 is advisory-forcing, never auto-rejecting a HUMAN):
//!   * an UN-ATTESTED link (agent/device — not the human decision the veto forces)
//!     that trips the hard veto is REFUSED at the local door;
//!   * a HUMAN-ATTESTED link passes — the attestation IS the human decision;
//!   * on the sync-apply path the event is admitted and projected (a node-local veto
//!     verdict must never fork the event set), but the pair lands on the
//!     `link_veto_flag` worklist and BOTH charts read `under-review` in `chart_trust`
//!     until a human resolves it (unlink, or a human-attested re-link).
//!
//! Real Postgres, gated on `$CAIRN_TEST_PG`, serialized via `db::test_serial_guard`.
use cairn_event::demographics::{dob_assertion_body, render_dob_twin};
use cairn_event::identity::{
    link_assertion_body, render_link_twin, render_unlink_twin, unlink_assertion_body, LinkAssertion,
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

fn db_msg(e: &tokio_postgres::Error) -> String {
    e.as_db_error()
        .map(|d| d.message().to_string())
        .unwrap_or_else(|| e.to_string())
}

/// Truncate the clinical tables; enroll one agent + one human. Returns their pairs.
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
         END $$;",
    )
    .await
    .unwrap();
    let (sk_a, kid_a) = generate_key().unwrap();
    let (sk_h, kid_h) = generate_key().unwrap();
    c.execute(
        "SELECT enroll_actor('agent', '{\"model\":\"lv-stub\",\"version\":\"1\",\"skill_epoch\":\"e\"}', $1)",
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

/// Submit one document-verified DOB assertion so the pair (a, b) trips the db/016
/// hard veto (both verified, same precision, different values).
async fn submit_dob(c: &Client, sk: &SigningKey, kid: &str, patient: Uuid, wall: i64, value: &str) {
    let body = EventBody {
        event_id: Uuid::now_v7().to_string(),
        patient_id: patient.to_string(),
        event_type: "demographic.field.asserted".into(),
        schema_version: "demographic.field/1".into(),
        hlc: Hlc {
            wall,
            counter: 0,
            node_origin: "n".into(),
        },
        t_effective: None,
        signer_key_id: kid.into(),
        contributors: serde_json::json!([{"actor_id": kid, "role": "recorded"}]),
        payload: dob_assertion_body(value, "day", Some("document"), "document-verified"),
        attachments: vec![],
        plaintext_twin: Some(render_dob_twin(value, "day", "document-verified")),
    };
    let signed = sign(&body, sk).unwrap();
    c.execute("SELECT submit_event($1)", &[&signed.signed_bytes])
        .await
        .expect("valid dob accepted");
}

/// Two fresh patients with a verified-DOB clash — a hard veto by construction.
async fn vetoed_pair(c: &Client, sk: &SigningKey, kid: &str) -> (Uuid, Uuid) {
    let a = Uuid::now_v7();
    let b = Uuid::now_v7();
    submit_dob(c, sk, kid, a, 1, "1980-07-15").await;
    submit_dob(c, sk, kid, b, 2, "1975-01-02").await;
    let vetoed: bool = c
        .query_one(
            "SELECT cairn_has_hard_veto($1::text::uuid, $2::text::uuid)",
            &[&a.to_string(), &b.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert!(
        vetoed,
        "test precondition: the pair must trip the hard veto"
    );
    (a, b)
}

/// A signed link/unlink body. `responsibility` adds the responsibility-bearing
/// contributor a human-attested link carries.
fn link_body(
    kid: &str,
    a: Uuid,
    b: Uuid,
    link: bool,
    wall: i64,
    responsibility: bool,
) -> EventBody {
    let a_s = a.to_string();
    let b_s = b.to_string();
    let la = LinkAssertion {
        subject_a: &a_s,
        subject_b: &b_s,
        provenance: "lv:test",
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
    let contributors = if responsibility {
        serde_json::json!([{"actor_id": kid, "role": "attested", "responsibility": "attested"}])
    } else {
        serde_json::json!([{"actor_id": kid, "role": "recorded"}])
    };
    EventBody {
        event_id: Uuid::now_v7().to_string(),
        patient_id: a_s.clone(),
        event_type: etype.into(),
        schema_version: "identity.link/1".into(),
        hlc: Hlc {
            wall,
            counter: 0,
            node_origin: "peer".into(),
        },
        t_effective: None,
        signer_key_id: kid.into(),
        contributors,
        payload,
        attachments: vec![],
        plaintext_twin: Some(twin),
    }
}

/// Are a and b in the SAME linkage component (a real merge)? A recompute over a
/// pure-unlink seed still writes a self-row (patient_id = person_id), so counting
/// person_member rows can't distinguish "merged" from "recomputed-but-separate" —
/// compare the two charts' person_id representatives instead.
async fn same_person(c: &Client, a: Uuid, b: Uuid) -> bool {
    let r: Option<bool> = c
        .query_one(
            "SELECT (SELECT person_id FROM person_member WHERE patient_id = $1::text::uuid)
                  = (SELECT person_id FROM person_member WHERE patient_id = $2::text::uuid)",
            &[&a.to_string(), &b.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    r.unwrap_or(false)
}

async fn trust_state(c: &Client, p: Uuid) -> Option<String> {
    c.query_opt(
        "SELECT trust_state FROM chart_trust WHERE patient_id = $1::text::uuid",
        &[&p.to_string()],
    )
    .await
    .unwrap()
    .map(|r| r.get(0))
}

#[tokio::test]
async fn agent_link_with_hard_veto_refused_at_local_door() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, _sk_h, _kid_h) = setup(&c).await;
    let (a, b) = vetoed_pair(&c, &sk_a, &kid_a).await;

    let body = link_body(&kid_a, a, b, true, 10, false);
    let signed = sign(&body, &sk_a).unwrap();
    let r = c
        .execute("SELECT submit_event($1)", &[&signed.signed_bytes])
        .await;
    assert!(
        r.is_err(),
        "an un-attested link that trips the hard veto must be refused at the local door"
    );
    let m = db_msg(&r.unwrap_err());
    assert!(
        m.contains("veto"),
        "the refusal must name the veto legibly, got: {m}"
    );

    // Nothing merged.
    let merged: i64 = c
        .query_one("SELECT count(*) FROM person_member", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(merged, 0, "the charts must not merge");
}

#[tokio::test]
async fn human_attested_link_with_hard_veto_still_passes() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, sk_h, kid_h) = setup(&c).await;
    let (a, b) = vetoed_pair(&c, &sk_a, &kid_a).await;

    // The human signs AND vouches: the veto forces a human decision — this IS it.
    let body = link_body(&kid_h, a, b, true, 10, true);
    let signed = sign(&body, &sk_h).unwrap();
    let ca = event_address(&signed.signed_bytes);
    let token = sign_attestation(&ca, &kid_h, "attested", &sk_h).unwrap();
    let vk = sk_h.verifying_key().to_bytes().to_vec();
    c.execute(
        "SELECT submit_event($1,$2,$3)",
        &[&signed.signed_bytes, &token, &vk],
    )
    .await
    .expect("a human-attested link must pass the veto floor (the human decided)");

    let merged: i64 = c
        .query_one("SELECT count(*) FROM person_member", &[])
        .await
        .unwrap()
        .get(0);
    assert!(merged > 0, "the human-attested link must project the merge");
}

#[tokio::test]
async fn remote_vetoed_link_admitted_flagged_and_under_review() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, _sk_h, _kid_h) = setup(&c).await;
    let (a, b) = vetoed_pair(&c, &sk_a, &kid_a).await;

    let body = link_body(&kid_a, a, b, true, 10, false);
    let signed = sign(&body, &sk_a).unwrap();
    c.execute("SELECT apply_remote_event($1)", &[&signed.signed_bytes])
        .await
        .expect("the apply door must admit the replicated link (converge, never fork)");

    // The link projects (set-union convergence)...
    let merged: i64 = c
        .query_one("SELECT count(*) FROM person_member", &[])
        .await
        .unwrap()
        .get(0);
    assert!(merged > 0, "sync must converge the link");

    // ...the pair is on the veto worklist...
    let flags: i64 = c
        .query_one("SELECT count(*) FROM link_veto_flag", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(flags, 1, "the vetoed link must be flagged, never silent");

    // ...and BOTH charts read under-review (never a silent merge).
    assert_eq!(trust_state(&c, a).await.as_deref(), Some("under-review"));
    assert_eq!(trust_state(&c, b).await.as_deref(), Some("under-review"));
}

#[tokio::test]
async fn unlink_clears_the_veto_flag() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, _sk_h, _kid_h) = setup(&c).await;
    let (a, b) = vetoed_pair(&c, &sk_a, &kid_a).await;

    let link = sign(&link_body(&kid_a, a, b, true, 10, false), &sk_a).unwrap();
    c.execute("SELECT apply_remote_event($1)", &[&link.signed_bytes])
        .await
        .unwrap();

    // The unlink is the never-erase reversal — the standing hazard is gone.
    let unlink = sign(&link_body(&kid_a, a, b, false, 11, false), &sk_a).unwrap();
    c.execute("SELECT apply_remote_event($1)", &[&unlink.signed_bytes])
        .await
        .expect("the unlink repair must always pass");

    let flags: i64 = c
        .query_one("SELECT count(*) FROM link_veto_flag", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(flags, 0, "an unlink must clear the standing veto flag");
    assert_eq!(
        trust_state(&c, a).await,
        None,
        "chart reads confirmed again"
    );
}

/// Finding 1 (this-PR review) — the flag lifecycle must key on the STANDING overlay
/// winner, not on the arriving event's verb. A backdated un-attested unlink that LOSES
/// the HLC overlay must NOT clear a still-standing vetoed merge: the charts are still
/// merged (the losing unlink changed no standing edge), so silently dropping the flag
/// would restore the exact silent hard-vetoed merge #190 exists to prevent — and it is
/// reachable by the ADR-0030 hostile enrolled writer with one cheap backdated event.
#[tokio::test]
async fn backdated_unlink_losing_overlay_keeps_the_flag() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, _sk_h, _kid_h) = setup(&c).await;
    let (a, b) = vetoed_pair(&c, &sk_a, &kid_a).await;

    // The vetoed link (wall 10) syncs in: merged, flagged, both charts under-review.
    let link = sign(&link_body(&kid_a, a, b, true, 10, false), &sk_a).unwrap();
    c.execute("SELECT apply_remote_event($1)", &[&link.signed_bytes])
        .await
        .unwrap();

    // A BACKDATED un-attested unlink (wall 9) arrives — it LOSES the overlay (9 < 10),
    // so the standing edge stays the merged link. The flag must survive.
    let unlink = sign(&link_body(&kid_a, a, b, false, 9, false), &sk_a).unwrap();
    c.execute("SELECT apply_remote_event($1)", &[&unlink.signed_bytes])
        .await
        .expect("a losing unlink still applies (it just doesn't win the overlay)");

    // The charts are STILL merged (the unlink changed no standing edge)...
    assert!(
        same_person(&c, a, b).await,
        "the losing unlink must not have split the charts"
    );

    // ...so the veto flag must NOT have been cleared (else: silent vetoed merge).
    let flags: i64 = c
        .query_one("SELECT count(*) FROM link_veto_flag", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        flags, 1,
        "a backdated unlink that loses the overlay must not clear the standing veto flag"
    );
    assert_eq!(trust_state(&c, a).await.as_deref(), Some("under-review"));
    assert_eq!(trust_state(&c, b).await.as_deref(), Some("under-review"));
}

/// Finding 1, converse — a stale un-attested vetoed link that LOSES to a standing unlink
/// must NOT raise a phantom flag. The pair is not merged (the link lost the overlay), so
/// flagging it would leave both charts under-review forever for a pair that isn't linked,
/// and make two honest nodes with the same event set disagree by arrival order.
#[tokio::test]
async fn stale_link_losing_to_standing_unlink_raises_no_phantom_flag() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, _sk_h, _kid_h) = setup(&c).await;
    let (a, b) = vetoed_pair(&c, &sk_a, &kid_a).await;

    // A standing unlink (wall 11) is the winner for the pair.
    let unlink = sign(&link_body(&kid_a, a, b, false, 11, false), &sk_a).unwrap();
    c.execute("SELECT apply_remote_event($1)", &[&unlink.signed_bytes])
        .await
        .unwrap();

    // A stale, lower-HLC vetoed link (wall 10) arrives afterwards — it LOSES the overlay.
    let link = sign(&link_body(&kid_a, a, b, true, 10, false), &sk_a).unwrap();
    c.execute("SELECT apply_remote_event($1)", &[&link.signed_bytes])
        .await
        .expect("a losing link still applies (it just doesn't win the overlay)");

    // Not merged, and NOT flagged (the standing winner is the unlink).
    assert!(
        !same_person(&c, a, b).await,
        "the losing link must not merge the charts"
    );
    let flags: i64 = c
        .query_one("SELECT count(*) FROM link_veto_flag", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        flags, 0,
        "a link that loses to a standing unlink must not raise a phantom veto flag"
    );
    assert_eq!(trust_state(&c, a).await, None, "chart reads confirmed");
}

#[tokio::test]
async fn human_attested_relink_clears_the_veto_flag() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _g = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_a, kid_a, sk_h, kid_h) = setup(&c).await;
    let (a, b) = vetoed_pair(&c, &sk_a, &kid_a).await;

    let link = sign(&link_body(&kid_a, a, b, true, 10, false), &sk_a).unwrap();
    c.execute("SELECT apply_remote_event($1)", &[&link.signed_bytes])
        .await
        .unwrap();

    // A human reviews the pair and CONFIRMS the link — the human decision supersedes
    // the flag (higher HLC wins the overlay; the vouch is the resolution).
    let body = link_body(&kid_h, a, b, true, 11, true);
    let signed = sign(&body, &sk_h).unwrap();
    let ca = event_address(&signed.signed_bytes);
    let token = sign_attestation(&ca, &kid_h, "attested", &sk_h).unwrap();
    let vk = sk_h.verifying_key().to_bytes().to_vec();
    c.execute(
        "SELECT submit_event($1,$2,$3)",
        &[&signed.signed_bytes, &token, &vk],
    )
    .await
    .expect("the human-attested confirmation must pass");

    let flags: i64 = c
        .query_one("SELECT count(*) FROM link_veto_flag", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        flags, 0,
        "a human-attested link for the pair resolves the veto flag"
    );
    assert_eq!(
        trust_state(&c, a).await,
        None,
        "chart reads confirmed again"
    );
}
