//! ADR-0052 apply-door (db/020) sealed/lenient arm + the ADR-0049 false-fresh gate.
//!
//! The APPLY door is the sync seam's counterpart to the strict submit door, and its
//! obligations are the MIRROR IMAGE of submit's: where submit REFUSES (born-sealed
//! floor, missing DEK), apply ADMITS — set-union losslessness means a validly-signed
//! event a peer accepted can never be lost here (availability over consistency). The
//! four legs of that lenient contract:
//!
//!   - sealed + DEK  → the full floor runs on the CLEAR view, custody + shadow stored,
//!     projections fire through the shadow (exactly as submit);
//!   - sealed w/o DEK → ADMIT structurally only: the signed stub twin is stored, NO
//!     custody / shadow / projection — never a reject (the ADR-0051 lenient-apply
//!     precedent applied to the seal);
//!   - plaintext clinical → ADMIT leniently (foreign / pre-ADR-0052 legacy data);
//!     only the STRICT door enforces born-sealed;
//!   - shredded target → admit the row, WITHHOLD custody (anti-resurrection: set-union
//!     may re-deliver forever, custody never comes back).
//!
//! Plus the ADR-0049 FALSE-FRESH gate: this is the first task that populates custody on
//! the SYNC path, so the sealed-thread staleness hazard goes live. A partial-custody node
//! reads FEWER content events than the attester reviewed, yet could recompute a commitment
//! that coincidentally matches — reading a grown thread as FRESH. The staleness view now
//! forces stale whenever the node's readable content-event count is LESS than the
//! attestation's reviewed_count (never "fresh" on a thread it cannot fully reproduce).
//!
//! DB-gated on $CAIRN_TEST_PG, serialized cluster-wide via db::test_serial_guard
//! (shared-DB + TRUNCATE pattern, like seal_submit.rs / medication_remote_apply.rs). Key
//! material is derived at runtime (generate_key), never a literal (house rule 6).
use cairn_event::seal::{derive_unwrap_secret, seal_event_payload, seal_stub_twin, unwrap_public};
use cairn_event::{
    event_address, generate_key, sign, sign_attestation, EventBody, Hlc, SigningKey,
};
use cairn_node::db;
use tokio_postgres::Client;
use uuid::Uuid;
use zeroize::Zeroizing;

fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

/// A realistic HLC wall (ms since epoch, ≈ 2026-06-21), comfortably in the past so the
/// apply door's clock-drift clamp never fires. Mirrors medication_remote_apply.rs.
const WALL_2026: i64 = 1_782_000_000_000;

/// An HLC "as minted on the peer" — the events in this file arrive by sync.
fn peer_hlc(wall: i64) -> Hlc {
    Hlc {
        wall,
        counter: 0,
        node_origin: "peer".into(),
    }
}

/// Truncate the log + custody plane + medication projections, then enroll a fresh DEVICE
/// actor (the peer's authoring key) plus a fresh HUMAN actor (signs + attests) and
/// register THIS node's X25519 unwrap key (device-derived) so the door can wrap sealed
/// DEKs into custody. The custody tables have NO FK to event_log, so the CASCADE from
/// event_log does not reach them — they must be named explicitly. Returns
/// (device_sk, device_kid, human_sk, human_kid).
async fn setup(c: &Client) -> (SigningKey, String, SigningKey, String) {
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
           IF to_regclass('public.medication_dose_event') IS NOT NULL THEN TRUNCATE medication_dose_event; END IF; \
           IF to_regclass('public.medication_attestation') IS NOT NULL THEN TRUNCATE medication_attestation; END IF; \
         END $$;",
    )
    .await
    .unwrap();
    let (sk_d, kid_d) = generate_key().unwrap();
    let (sk_h, kid_h) = generate_key().unwrap();
    c.execute(
        "SELECT enroll_actor('device', '{\"role\":\"peer-ward-terminal\"}', $1)",
        &[&kid_d],
    )
    .await
    .unwrap();
    c.execute(
        "SELECT enroll_actor('human', '{\"role\":\"clinician\"}', $1)",
        &[&kid_h],
    )
    .await
    .unwrap();
    // The node's unwrap key is derived from the DEVICE key (a node has exactly one,
    // regardless of who signs individual events). Registering it is what lets the door
    // wrap a sealed event's DEK into custody at apply.
    let secret = derive_unwrap_secret(&sk_d.to_bytes());
    c.execute(
        "SELECT cairn_register_unwrap_key($1)",
        &[&unwrap_public(&secret).as_slice()],
    )
    .await
    .unwrap();
    (sk_d, kid_d, sk_h, kid_h)
}

/// Build a CLEAR medication-assert EventBody (mirror of medication/assert.rs's field
/// construction), then seal it: payload → container, twin → stub. Returns the sealed
/// body (ready to sign) and the per-event DEK. Copied verbatim from seal_submit.rs —
/// integration tests cannot share a helper module.
fn sealed_assert_body(node_kid: &str, patient: Uuid, hlc: Hlc) -> (EventBody, Zeroizing<[u8; 32]>) {
    let event_id = Uuid::now_v7().to_string();
    let medication_id = Uuid::now_v7().to_string();
    let payload = serde_json::json!({
        "medication_id": medication_id,
        "substance": {"term": "amoxicillin"},
        "info_source": "patient",
    });
    let twin = format!("amoxicillin — asserted for {patient}");
    let (container, dek) = seal_event_payload(&payload, &twin, &event_id).unwrap();
    let body = EventBody {
        event_id,
        patient_id: patient.to_string(),
        event_type: "clinical.medication.asserted".into(),
        schema_version: "clinical.medication/1".into(),
        hlc,
        t_effective: None,
        signer_key_id: node_kid.into(),
        contributors: serde_json::json!([{"actor_id": node_kid, "role": "recorded"}]),
        payload: container,
        attachments: vec![],
        plaintext_twin: Some(seal_stub_twin("clinical.medication.asserted")),
    };
    (body, dek)
}

/// The SAME clear medication body, but LEFT UNSEALED — a legacy / foreign plaintext
/// clinical body (payload is the clear medication payload, plaintext_twin is the clear
/// twin, no container). This is exactly what the STRICT door refuses and the LENIENT
/// apply door must admit (set-union losslessness for foreign data).
fn unsealed_assert_body(node_kid: &str, patient: Uuid, hlc: Hlc) -> EventBody {
    let event_id = Uuid::now_v7().to_string();
    let medication_id = Uuid::now_v7().to_string();
    let payload = serde_json::json!({
        "medication_id": medication_id,
        "substance": {"term": "amoxicillin"},
        "info_source": "patient",
    });
    let twin = format!("amoxicillin — asserted for {patient}");
    EventBody {
        event_id,
        patient_id: patient.to_string(),
        event_type: "clinical.medication.asserted".into(),
        schema_version: "clinical.medication/1".into(),
        hlc,
        t_effective: None,
        signer_key_id: node_kid.into(),
        contributors: serde_json::json!([{"actor_id": node_kid, "role": "recorded"}]),
        payload,
        attachments: vec![],
        plaintext_twin: Some(twin),
    }
}

/// An `erasure.shred.asserted` tombstone targeting `target`. Plaintext BY DESIGN (the
/// tombstone must outlive every key — it is NOT `clinical.%`, so it passes both doors),
/// with an authored twin (the type hard-requires one) and a plain `recorded` contributor
/// carrying NO responsibility, so it never trips the attestation gate. (The human-vouched
/// shred ceremony is the CLI's job — Task 11.)
fn shred_body(node_kid: &str, patient: Uuid, target: &str, hlc: Hlc) -> EventBody {
    let event_id = Uuid::now_v7().to_string();
    let payload = serde_json::json!({
        "target_event_id": target,
        "basis": "retention ceiling",
    });
    EventBody {
        event_id,
        patient_id: patient.to_string(),
        event_type: "erasure.shred.asserted".into(),
        schema_version: "erasure.shred/1".into(),
        hlc,
        t_effective: None,
        signer_key_id: node_kid.into(),
        contributors: serde_json::json!([{"actor_id": node_kid, "role": "recorded"}]),
        payload,
        attachments: vec![],
        plaintext_twin: Some(format!(
            "shredded medication assertion {target} — basis: retention ceiling"
        )),
    }
}

/// Count the medication_statement rows projected for `patient`.
async fn statement_count(c: &Client, patient: Uuid) -> i64 {
    c.query_one(
        "SELECT count(*) FROM medication_statement WHERE patient_id = $1::text::uuid",
        &[&patient.to_string()],
    )
    .await
    .unwrap()
    .get(0)
}

/// Count the custody rows (event_dek + event_clear) held for one event id.
async fn custody_count(c: &Client, event_id: &str) -> (i64, i64) {
    let dek: i64 = c
        .query_one(
            "SELECT count(*) FROM event_dek WHERE event_id = $1::text::uuid",
            &[&event_id],
        )
        .await
        .unwrap()
        .get(0);
    let clear: i64 = c
        .query_one(
            "SELECT count(*) FROM event_clear WHERE event_id = $1::text::uuid",
            &[&event_id],
        )
        .await
        .unwrap()
        .get(0);
    (dek, clear)
}

#[tokio::test]
async fn sealed_apply_with_dek_projects_like_submit() {
    // Leg 1: a sealed event delivered WITH its sidecar DEK runs the full floor on the
    // clear view, stores custody + shadow, and projects — indistinguishable from submit.
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_d, kid_d, _sk_h, _kid_h) = setup(&c).await;
    let patient = Uuid::now_v7();

    let (body, dek) = sealed_assert_body(&kid_d, patient, peer_hlc(WALL_2026));
    let event_id = body.event_id.clone();
    let signed = sign(&body, &sk_d).unwrap();
    c.execute(
        "SELECT apply_remote_event($1, NULL, NULL, $2)",
        &[&signed.signed_bytes, &dek.as_slice()],
    )
    .await
    .expect("a sealed event WITH its DEK applies through the sync door");

    // event_log holds the CIPHERTEXT container, the stub twin, and sealed = true.
    let (sealed, twin, body_text): (bool, String, String) = {
        let row = c
            .query_one(
                "SELECT sealed, plaintext_twin, body::text FROM event_log WHERE event_id = $1::text::uuid",
                &[&event_id],
            )
            .await
            .unwrap();
        (row.get(0), row.get(1), row.get(2))
    };
    assert!(sealed, "the applied row is marked sealed");
    assert!(
        twin.contains("twin under seal"),
        "the stored outer twin is the mechanical stub, got: {twin}"
    );
    assert!(
        !body_text.contains("amoxicillin"),
        "no cleartext leaks into event_log.body"
    );

    // event_clear + event_dek: the operational shadow and recoverable custody.
    let clear_term: String = c
        .query_one(
            "SELECT body -> 'substance' ->> 'term' FROM event_clear WHERE event_id = $1::text::uuid",
            &[&event_id],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(clear_term, "amoxicillin", "the clear payload is shadowed");
    let (dek_rows, clear_rows) = custody_count(&c, &event_id).await;
    assert_eq!(
        (dek_rows, clear_rows),
        (1, 1),
        "custody + shadow both stored"
    );

    // The projection fired THROUGH the shadow.
    assert_eq!(
        statement_count(&c, patient).await,
        1,
        "the sealed assert projected via the clear shadow, exactly like submit"
    );
}

#[tokio::test]
async fn sealed_apply_without_dek_admits_structurally_never_rejects() {
    // Leg 2: a sealed event delivered WITHOUT its DEK (not a custody holder, or a
    // byte-lazy pull) is ADMITTED on structural checks only — never rejected. The signed
    // stub twin is stored; no custody, no shadow, no projection. This is the ADR-0051
    // lenient-apply precedent applied to the seal.
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_d, kid_d, _sk_h, _kid_h) = setup(&c).await;
    let patient = Uuid::now_v7();

    let (body, _dek) = sealed_assert_body(&kid_d, patient, peer_hlc(WALL_2026));
    let event_id = body.event_id.clone();
    let signed = sign(&body, &sk_d).unwrap();
    // One-arg apply: the sealed body arrives, but no DEK travelled. (Read the returned
    // UUID as ::text — this crate's tokio-postgres has no uuid FromSql feature.)
    let returned: String = c
        .query_one(
            "SELECT apply_remote_event($1)::text",
            &[&signed.signed_bytes],
        )
        .await
        .expect("a sealed body without its DEK is ADMITTED, never refused")
        .get(0);
    assert_eq!(returned, event_id, "the door returns the event id");

    // The row landed, sealed, with the stub twin — but nothing custody-derived exists.
    let (sealed, twin): (bool, String) = {
        let row = c
            .query_one(
                "SELECT sealed, plaintext_twin FROM event_log WHERE event_id = $1::text::uuid",
                &[&event_id],
            )
            .await
            .expect("the event was admitted into the log");
        (row.get(0), row.get(1))
    };
    assert!(sealed, "the admitted row is sealed");
    assert!(
        twin.contains("twin under seal"),
        "the stored twin is the signed stub, got: {twin}"
    );
    let (dek_rows, clear_rows) = custody_count(&c, &event_id).await;
    assert_eq!(
        (dek_rows, clear_rows),
        (0, 0),
        "no custody and no shadow without the DEK"
    );
    assert_eq!(
        statement_count(&c, patient).await,
        0,
        "no projection without the clear shadow"
    );
}

#[tokio::test]
async fn plaintext_clinical_apply_is_admitted_leniently() {
    // Leg 3: the SAME unsealed clinical body the strict door REFUSES (proven in
    // seal_submit.rs) is ADMITTED at the sync door and projects — set-union losslessness
    // for foreign / pre-ADR-0052 data. Only the STRICT door enforces born-sealed.
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_d, kid_d, _sk_h, _kid_h) = setup(&c).await;
    let patient = Uuid::now_v7();

    let body = unsealed_assert_body(&kid_d, patient, peer_hlc(WALL_2026));
    let event_id = body.event_id.clone();
    let signed = sign(&body, &sk_d).unwrap();
    c.execute("SELECT apply_remote_event($1)", &[&signed.signed_bytes])
        .await
        .expect("a plaintext clinical body is admitted leniently at the sync door");

    let sealed: bool = c
        .query_one(
            "SELECT sealed FROM event_log WHERE event_id = $1::text::uuid",
            &[&event_id],
        )
        .await
        .expect("the foreign plaintext body was admitted")
        .get(0);
    assert!(!sealed, "an unsealed body is stored unsealed");
    assert_eq!(
        statement_count(&c, patient).await,
        1,
        "the foreign plaintext assert projects directly from its (unsealed) body"
    );
}

#[tokio::test]
async fn custody_is_never_granted_to_a_shredded_event() {
    // Leg 4 + anti-resurrection: submit a sealed event via the full local path, shred it,
    // then RE-apply the original WITH its DEK (set-union re-delivery). The row is
    // conflict-ignored and custody NEVER comes back — arrival-order independence half 1.
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_d, kid_d, _sk_h, _kid_h) = setup(&c).await;
    let patient = Uuid::now_v7();

    // 1. Author a sealed medication assert locally, with custody + projection.
    let (body, dek) = sealed_assert_body(&kid_d, patient, peer_hlc(WALL_2026));
    let event_id = body.event_id.clone();
    let signed = sign(&body, &sk_d).unwrap();
    c.execute(
        "SELECT submit_event($1, NULL, NULL, $2)",
        &[&signed.signed_bytes, &dek.as_slice()],
    )
    .await
    .expect("the sealed assert is authored locally");
    let (dek0, clear0) = custody_count(&c, &event_id).await;
    assert_eq!(
        (dek0, clear0),
        (1, 1),
        "custody + shadow exist before the shred"
    );
    assert_eq!(
        statement_count(&c, patient).await,
        1,
        "projected before the shred"
    );

    // 2. Author + apply an erasure.shred for it — scrubs custody, shadow, projection.
    let shred = shred_body(&kid_d, patient, &event_id, peer_hlc(WALL_2026 + 10));
    let shred_signed = sign(&shred, &sk_d).unwrap();
    c.execute(
        "SELECT apply_remote_event($1)",
        &[&shred_signed.signed_bytes],
    )
    .await
    .expect("the shred applies");
    let (dek1, clear1) = custody_count(&c, &event_id).await;
    assert_eq!(
        (dek1, clear1),
        (0, 0),
        "the shred scrubbed custody + shadow"
    );
    assert_eq!(
        statement_count(&c, patient).await,
        0,
        "the shred scrubbed the projection"
    );

    // 3. Set-union re-delivers the original sealed event WITH its DEK. It must NOT
    //    resurrect: the row is conflict-ignored and custody stays extinguished.
    c.execute(
        "SELECT apply_remote_event($1, NULL, NULL, $2)",
        &[&signed.signed_bytes, &dek.as_slice()],
    )
    .await
    .expect("re-delivery of a shredded event is admitted (set-union) but grants no custody");
    let (dek2, clear2) = custody_count(&c, &event_id).await;
    assert_eq!(
        (dek2, clear2),
        (0, 0),
        "custody never resurrects for a shredded event, whatever the arrival order"
    );
    assert_eq!(
        statement_count(&c, patient).await,
        0,
        "and the projection stays extinguished"
    );
}

#[tokio::test]
async fn partial_custody_thread_reads_stale_not_fresh() {
    // The ADR-0049 FALSE-FRESH gate, now that custody is reachable on the sync path.
    //
    // A peer's attestation vouched for a thread it reviewed at reviewed_count = 2. This
    // node applies one content event WITH custody (readable = 1) and the attestation WITH
    // custody, and — the adversarial part — the attestation's pinned reviewed_commitment
    // exactly matches the ONE event this node can read. So the natural commitment compare
    // reads FRESH (H({ca1}) == pinned), the precise false-fresh ADR-0049 forbids: the
    // thread the attester actually reviewed is larger than what this node can reproduce.
    // The count tripwire (readable 1 < reviewed 2) must force stale regardless.
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_d, kid_d, sk_h, kid_h) = setup(&c).await;
    let patient = Uuid::now_v7();

    // One content event, applied WITH custody → readable = 1.
    let (body, dek) = sealed_assert_body(&kid_d, patient, peer_hlc(WALL_2026));
    let medication_id: String = {
        // Recover the thread id from the clear shadow after apply (the payload is sealed).
        // Read it as text — this crate's tokio-postgres has no uuid FromSql feature.
        let signed = sign(&body, &sk_d).unwrap();
        let event_id = body.event_id.clone();
        c.execute(
            "SELECT apply_remote_event($1, NULL, NULL, $2)",
            &[&signed.signed_bytes, &dek.as_slice()],
        )
        .await
        .expect("the content event applies with custody");
        c.query_one(
            "SELECT body ->> 'medication_id' FROM event_clear WHERE event_id = $1::text::uuid",
            &[&event_id],
        )
        .await
        .unwrap()
        .get(0)
    };

    // The commitment this node CAN compute over its one readable event — H({ca1}).
    let commitment: Vec<u8> = c
        .query_one(
            "SELECT cairn_medication_thread_commitment($1::text::uuid)",
            &[&medication_id],
        )
        .await
        .unwrap()
        .get(0);

    // A peer's attestation pinning that exact commitment but reviewed_count = 2 (it
    // reviewed a thread this node cannot fully see). Human-signed, self-attested, sealed,
    // applied WITH its DEK + token so it projects into the overlay.
    let att_clear = serde_json::json!({
        "medication_id": medication_id,
        "reviewed_commitment": hex::encode(&commitment),
        "reviewed_count": 2,
    });
    let att_event_id = Uuid::now_v7().to_string();
    let att_twin = "reviewed and attested the medication thread".to_string();
    let (att_container, att_dek) =
        seal_event_payload(&att_clear, &att_twin, &att_event_id).unwrap();
    let att_body = EventBody {
        event_id: att_event_id.clone(),
        patient_id: patient.to_string(),
        event_type: "clinical.medication-attestation.asserted".into(),
        schema_version: "clinical.medication-attestation/1".into(),
        hlc: peer_hlc(WALL_2026 + 20),
        t_effective: None,
        signer_key_id: kid_h.clone(),
        contributors: serde_json::json!([
            {"actor_id": kid_h, "role": "attested", "responsibility": {"held_by": kid_h}}
        ]),
        payload: att_container,
        attachments: vec![],
        plaintext_twin: Some(seal_stub_twin("clinical.medication-attestation.asserted")),
    };
    let att_signed = sign(&att_body, &sk_h).unwrap();
    let ca = event_address(&att_signed.signed_bytes);
    let token = sign_attestation(&ca, &kid_h, "attested", &sk_h).unwrap();
    let attester_key = hex::decode(&kid_h).unwrap();
    c.execute(
        "SELECT apply_remote_event($1, $2, $3, $4)",
        &[
            &att_signed.signed_bytes,
            &token,
            &attester_key,
            &att_dek.as_slice(),
        ],
    )
    .await
    .expect("the sealed attestation applies with its DEK + token");

    // Without the gate, the commitment matches and this reads stale = FALSE (false-fresh).
    // With the gate, readable (1) < reviewed_count (2) forces stale = TRUE.
    let stale: bool = c
        .query_one(
            "SELECT stale FROM medication_thread_attestation WHERE medication_id = $1::text::uuid",
            &[&medication_id],
        )
        .await
        .expect("the vouch projects")
        .get(0);
    assert!(
        stale,
        "a partial-custody thread the node cannot fully reproduce must read stale, \
         never fresh — the false-fresh gate forces it (readable count < reviewed_count)"
    );
}
