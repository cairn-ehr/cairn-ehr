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

/// A HOSTILE `identity.link.asserted` whose payload claims `sealed=true` AND carries a
/// MALFORMED top-level `subject_a` (a non-UUID string) — the exact shape a non-conformant
/// / compromised enrolled peer can mint. A conformant node never seals a non-clinical body
/// (the strict door refuses it), and a *well-formed* sealed container hides subject_a inside
/// `ct` (so `p ->> 'subject_a'` is NULL and casts to NULL harmlessly). This shape puts a
/// non-UUID at the TOP level, so patient_link_apply's DECLARE-block `(p ->> 'subject_a')::uuid`
/// cast would raise BEFORE the seal guard — aborting apply on a verifiable event and wedging
/// sync (code-review finding #1). Built by hand (not seal_event_payload) precisely to inject
/// the top-level garbage field; no DEK travels (the exploit's no-custody path).
fn malformed_sealed_link_body(node_kid: &str, patient: Uuid, hlc: Hlc) -> EventBody {
    let payload = serde_json::json!({
        "sealed": true,
        "subject_a": "not-a-uuid",
        "subject_b": "also-not-a-uuid",
        "provenance": "hostile-peer",
    });
    EventBody {
        event_id: Uuid::now_v7().to_string(),
        patient_id: patient.to_string(),
        event_type: "identity.link.asserted".into(),
        schema_version: "identity.link/1".into(),
        hlc,
        t_effective: None,
        signer_key_id: node_kid.into(),
        contributors: serde_json::json!([{"actor_id": node_kid, "role": "recorded"}]),
        payload,
        attachments: vec![],
        plaintext_twin: Some(seal_stub_twin("identity.link.asserted")),
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

#[tokio::test]
async fn shred_after_event_scrubs_everything_but_the_log_row() {
    // Task 9 (a): the OTHER direction from custody_is_never_granted_to_a_shredded_event
    // (which shreds BEFORE re-delivery). Here the shred arrives AFTER its target, via
    // the full local submit_event path (custody + shadow + projection all present
    // beforehand), and must scrub every DERIVED surface (custody, shadow, provenance-
    // precise projection) while leaving the append-only event_log rows — target AND
    // tombstone — untouched, including the target's signature, which still verifies
    // over the CIPHERTEXT it was signed over (ADR-0005: erasure redistributes key
    // custody, it never deletes the signed record).
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_d, kid_d, _sk_h, _kid_h) = setup(&c).await;
    let patient = Uuid::now_v7();

    // 1. Full sealed local submit — custody + shadow + projection all present.
    let (body, dek) = sealed_assert_body(&kid_d, patient, peer_hlc(WALL_2026));
    let event_id = body.event_id.clone();
    let signed = sign(&body, &sk_d).unwrap();
    c.execute(
        "SELECT submit_event($1, NULL, NULL, $2)",
        &[&signed.signed_bytes, &dek.as_slice()],
    )
    .await
    .expect("the sealed assert is authored locally");
    assert_eq!(
        custody_count(&c, &event_id).await,
        (1, 1),
        "custody + shadow exist before the shred"
    );
    assert_eq!(
        statement_count(&c, patient).await,
        1,
        "projected before the shred"
    );

    // 2. Shred it via the STRICT door (the target exists locally, so its existence
    //    check passes and the shred executes in the same submit).
    let shred = shred_body(&kid_d, patient, &event_id, peer_hlc(WALL_2026 + 10));
    let shred_event_id = shred.event_id.clone();
    let shred_signed = sign(&shred, &sk_d).unwrap();
    c.execute("SELECT submit_event($1)", &[&shred_signed.signed_bytes])
        .await
        .expect("the shred is admitted by the strict door (target exists)");

    // 3. event_dek / event_clear / medication_statement are all scrubbed.
    assert_eq!(
        custody_count(&c, &event_id).await,
        (0, 0),
        "the shred scrubs custody + shadow"
    );
    assert_eq!(
        statement_count(&c, patient).await,
        0,
        "the shred scrubs the content_address-precise projection"
    );

    // 4. erasure_shred_log carries the row, with its basis.
    let (logged_shred_id, basis): (String, String) = {
        let row = c
            .query_one(
                "SELECT shred_event_id::text, basis FROM erasure_shred_log \
                 WHERE target_event_id = $1::text::uuid",
                &[&event_id],
            )
            .await
            .expect("the shred log carries the target's row");
        (row.get(0), row.get(1))
    };
    assert_eq!(
        logged_shred_id, shred_event_id,
        "the log names the shredding event"
    );
    assert_eq!(basis, "retention ceiling", "the audited basis is recorded");

    // 5. event_log still holds BOTH rows — the target (untouched, sealed ciphertext)
    //    and the tombstone. Append-only: shredding never deletes a log row.
    let log_rows: i64 = c
        .query_one(
            "SELECT count(*) FROM event_log WHERE event_id IN ($1::text::uuid, $2::text::uuid)",
            &[&event_id, &shred_event_id],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        log_rows, 2,
        "both the target and the tombstone survive in event_log"
    );

    // 6. The signature over the target's CIPHERTEXT still verifies — shredding destroys
    //    custody (the DEK), never the signed bytes.
    let still_verifies: bool = c
        .query_one(
            "SELECT cairn_verify(signed_bytes) FROM event_log WHERE event_id = $1::text::uuid",
            &[&event_id],
        )
        .await
        .unwrap()
        .get(0);
    assert!(
        still_verifies,
        "the target row's signature over ciphertext survives the shred"
    );
}

#[tokio::test]
async fn shred_is_idempotent_under_replay() {
    // Task 9 (b): set-union re-delivery of the SAME shred event must be a silent no-op
    // at the apply door — the second apply returns the same uuid, raises no error, and
    // erasure_shred_log still holds exactly one row (the ON CONFLICT DO NOTHING in
    // cairn_execute_shred, db/037).
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_d, kid_d, _sk_h, _kid_h) = setup(&c).await;
    let patient = Uuid::now_v7();

    // Content event, authored locally with custody.
    let (body, dek) = sealed_assert_body(&kid_d, patient, peer_hlc(WALL_2026));
    let event_id = body.event_id.clone();
    let signed = sign(&body, &sk_d).unwrap();
    c.execute(
        "SELECT submit_event($1, NULL, NULL, $2)",
        &[&signed.signed_bytes, &dek.as_slice()],
    )
    .await
    .expect("the sealed assert is authored locally");

    // The shred, applied via the SYNC door (as it would arrive from a peer).
    let shred = shred_body(&kid_d, patient, &event_id, peer_hlc(WALL_2026 + 10));
    let shred_event_id = shred.event_id.clone();
    let shred_signed = sign(&shred, &sk_d).unwrap();

    let first: String = c
        .query_one(
            "SELECT apply_remote_event($1)::text",
            &[&shred_signed.signed_bytes],
        )
        .await
        .expect("the first apply of the shred succeeds")
        .get(0);
    assert_eq!(first, shred_event_id);

    let second: String = c
        .query_one(
            "SELECT apply_remote_event($1)::text",
            &[&shred_signed.signed_bytes],
        )
        .await
        .expect("re-delivery of the SAME shred event is a silent no-op, not an error")
        .get(0);
    assert_eq!(
        second, shred_event_id,
        "the replayed apply returns the same event id"
    );

    let log_rows: i64 = c
        .query_one(
            "SELECT count(*) FROM erasure_shred_log WHERE target_event_id = $1::text::uuid",
            &[&event_id],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(log_rows, 1, "erasure_shred_log still holds exactly one row");
}

#[tokio::test]
async fn schema_reload_rebuilds_the_shred_log() {
    // Task 9 (c): erasure_shred_log is derived state, rebuilt idempotently from the
    // append-only event_log tombstone on every schema load (db/037's trailing
    // INSERT ... SELECT ... ON CONFLICT DO NOTHING). Simulate a wiped projection-state
    // table, reconnect through connect_and_load_schema, and confirm the row comes back
    // from the log alone — "restore replays the shred log" (§3.8).
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
        "SELECT submit_event($1, NULL, NULL, $2)",
        &[&signed.signed_bytes, &dek.as_slice()],
    )
    .await
    .expect("the sealed assert is authored locally");

    let shred = shred_body(&kid_d, patient, &event_id, peer_hlc(WALL_2026 + 10));
    let shred_event_id = shred.event_id.clone();
    let shred_signed = sign(&shred, &sk_d).unwrap();
    c.execute("SELECT submit_event($1)", &[&shred_signed.signed_bytes])
        .await
        .expect("the shred is admitted");

    let before: i64 = c
        .query_one(
            "SELECT count(*) FROM erasure_shred_log WHERE target_event_id = $1::text::uuid",
            &[&event_id],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(before, 1, "the shred log row exists before the wipe");

    // Simulate a projection-state wipe (e.g. a fresh replica before its first replay).
    c.batch_execute("DELETE FROM erasure_shred_log")
        .await
        .unwrap();
    let wiped: i64 = c
        .query_one("SELECT count(*) FROM erasure_shred_log", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(wiped, 0, "the wipe removed the row");

    // Reload the schema — db/037's rebuild-from-tombstones INSERT SELECT re-derives
    // erasure_shred_log purely from event_log's tombstone row.
    let c2 = db::connect_and_load_schema(&base).await.unwrap();
    let (logged_shred_id, basis): (String, String) = {
        let row = c2
            .query_one(
                "SELECT shred_event_id::text, basis FROM erasure_shred_log \
                 WHERE target_event_id = $1::text::uuid",
                &[&event_id],
            )
            .await
            .expect("the schema reload rebuilds the row from the tombstone");
        (row.get(0), row.get(1))
    };
    assert_eq!(
        logged_shred_id, shred_event_id,
        "the rebuilt row names the shredding event"
    );
    assert_eq!(
        basis, "retention ceiling",
        "the audited basis survives the rebuild"
    );
}

#[tokio::test]
async fn sealed_apply_with_wrong_dek_admits_without_custody() {
    // Task-8-review Minor: a DEK that fails to open the sealed body (wrong key,
    // PRESENT but incorrect — distinct from the absent-DEK leg already pinned by
    // sealed_apply_without_dek_admits_structurally_never_rejects) is a transport
    // defect, never a refusal reason at the lenient sync door: cairn_unseal_body
    // returns NULL, the door WARNs and admits structurally with no custody. The wrong
    // key is DERIVED from the real, randomly-generated per-event DEK (one bit flipped),
    // never a literal (house rule 6).
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

    let mut wrong_dek: [u8; 32] = *dek;
    wrong_dek[0] ^= 0xFF;

    c.execute(
        "SELECT apply_remote_event($1, NULL, NULL, $2)",
        &[&signed.signed_bytes, &wrong_dek.as_slice()],
    )
    .await
    .expect("a wrong DEK is a transport defect, never a refusal reason");

    let (dek_rows, clear_rows) = custody_count(&c, &event_id).await;
    assert_eq!(
        (dek_rows, clear_rows),
        (0, 0),
        "no custody and no shadow when the presented DEK fails to open the body"
    );
    assert_eq!(
        statement_count(&c, patient).await,
        0,
        "no projection without the clear shadow"
    );
}

#[tokio::test]
async fn custody_is_withheld_when_shred_arrives_before_its_target() {
    // Task-8-review Minor: anti-resurrection HALF 2 (the mirror of
    // custody_is_never_granted_to_a_shredded_event, which pins half 1 — shred AFTER
    // re-delivery of content). A shred may arrive on the wire BEFORE its target —
    // erasure.shred.asserted is classified targets_other_author = false (db/037)
    // precisely so the apply door's generic target-existence gate does not block it.
    // The LATER-arriving content event must still be admitted (set-union losslessness)
    // but must NEVER be granted custody — arrival-order independence, either direction.
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_d, kid_d, _sk_h, _kid_h) = setup(&c).await;
    let patient = Uuid::now_v7();

    // The content event's id is minted before it is ever applied — exactly the shape a
    // real shred-before-content race takes on the wire.
    let (body, dek) = sealed_assert_body(&kid_d, patient, peer_hlc(WALL_2026));
    let event_id = body.event_id.clone();

    // 1. The shred arrives FIRST, targeting an event this node has never seen.
    let shred = shred_body(&kid_d, patient, &event_id, peer_hlc(WALL_2026 - 5));
    let shred_signed = sign(&shred, &sk_d).unwrap();
    c.execute(
        "SELECT apply_remote_event($1)",
        &[&shred_signed.signed_bytes],
    )
    .await
    .expect("the lenient apply door admits a shred whose target is not yet known");
    let pre_target_log: i64 = c
        .query_one(
            "SELECT count(*) FROM erasure_shred_log WHERE target_event_id = $1::text::uuid",
            &[&event_id],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        pre_target_log, 1,
        "the shred is recorded before its target ever lands"
    );

    // 2. The content event arrives SECOND, WITH its DEK — it would normally gain full
    //    custody, but the shred already named it.
    let signed = sign(&body, &sk_d).unwrap();
    c.execute(
        "SELECT apply_remote_event($1, NULL, NULL, $2)",
        &[&signed.signed_bytes, &dek.as_slice()],
    )
    .await
    .expect("the content event is admitted (set-union), even though it arrives shredded");

    let (dek_rows, clear_rows) = custody_count(&c, &event_id).await;
    assert_eq!(
        (dek_rows, clear_rows),
        (0, 0),
        "custody is withheld — the shred already named this event"
    );
    assert_eq!(
        statement_count(&c, patient).await,
        0,
        "no projection: no custody means no clear shadow to project through"
    );
}

/// A CLEAR demographic.field.asserted (dob) body — NON-clinical, plaintext by necessity.
fn demographic_body(node_kid: &str, patient: Uuid, hlc: Hlc) -> EventBody {
    EventBody {
        event_id: Uuid::now_v7().to_string(),
        patient_id: patient.to_string(),
        event_type: "demographic.field.asserted".into(),
        schema_version: "demographic.field/1".into(),
        hlc,
        t_effective: None,
        signer_key_id: node_kid.into(),
        contributors: serde_json::json!([{"actor_id": node_kid, "role": "recorded"}]),
        payload: serde_json::json!({
            "field": "dob",
            "value": "1980",
            "provenance": "document-verified",
            "facets": {"precision": "year"},
        }),
        attachments: vec![],
        plaintext_twin: Some(format!("dob 1980 — asserted for {patient}")),
    }
}

/// The SAME clear demographic body, but wrongly SEALED — the never-lawful shape ADR-0052 §2
/// forbids (only clinical.* is born-sealed). Returns the sealed body and its (unused) DEK.
fn sealed_demographic_body(
    node_kid: &str,
    patient: Uuid,
    hlc: Hlc,
) -> (EventBody, Zeroizing<[u8; 32]>) {
    let mut b = demographic_body(node_kid, patient, hlc);
    let payload = b.payload.clone();
    let twin = b.plaintext_twin.clone().unwrap();
    let (container, dek) = seal_event_payload(&payload, &twin, &b.event_id).unwrap();
    b.payload = container;
    b.plaintext_twin = Some(seal_stub_twin("demographic.field.asserted"));
    (b, dek)
}

#[tokio::test]
async fn sealed_non_clinical_admits_without_projection_or_wedge() {
    // The availability-floor wedge vector (ADR-0052 §2, final review, issue #10): a
    // validly-signed NON-clinical body carrying payload.sealed=true, WITHOUT a DEK — the
    // exact exploit shape. The strict door refuses it, but the LENIENT sync door must ADMIT
    // it (set-union losslessness) WITHOUT letting a NEW.body-reading projection detonate on
    // the ciphertext: a RAISE here would fail a VERIFIABLE event at apply, and do_pull
    // FREEZES its watermark on any verifiable event that fails to apply — WEDGING clinical
    // sync permanently. The fix makes every non-clinical projection seal-robust (returns
    // NULL on a sealed row): the sealed non-clinical row is admitted as harmless ciphertext
    // noise (no custody, no projection, no leak) and a later legit event still projects.
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_d, kid_d, _sk_h, _kid_h) = setup(&c).await;
    let patient = Uuid::now_v7();

    // Pre-fix this DETONATES patient_address_apply: on the ciphertext body `p ->> 'field'`
    // is NULL, so `fld <> 'address'` is NULL (NOT true) and the trigger falls through to
    // INSERT a NULL `display` into a NOT NULL PK column → RAISE → apply fails on a verifiable
    // event → freeze. Post-fix the projection guard returns NULL first, so apply succeeds.
    let (body, _dek) = sealed_demographic_body(&kid_d, patient, peer_hlc(WALL_2026));
    let event_id = body.event_id.clone();
    let signed = sign(&body, &sk_d).unwrap();
    let returned: String = c
        .query_one(
            "SELECT apply_remote_event($1)::text",
            &[&signed.signed_bytes],
        )
        .await
        .expect("a sealed NON-clinical body is ADMITTED, never detonates a projection")
        .get(0);
    assert_eq!(returned, event_id, "the door returns the event id");

    // The row landed, marked sealed — and NO projection row exists anywhere (ciphertext
    // projects nothing: no demographic field, no name, no address).
    let sealed: bool = c
        .query_one(
            "SELECT sealed FROM event_log WHERE event_id = $1::text::uuid",
            &[&event_id],
        )
        .await
        .expect("the event was admitted into the log")
        .get(0);
    assert!(
        sealed,
        "the sealed non-clinical row is admitted, marked sealed"
    );
    let projected: i64 = c
        .query_one(
            "SELECT (SELECT count(*) FROM patient_demographic WHERE patient_id = $1::text::uuid) \
                  + (SELECT count(*) FROM patient_name        WHERE patient_id = $1::text::uuid) \
                  + (SELECT count(*) FROM patient_address     WHERE patient_id = $1::text::uuid)",
            &[&patient.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        projected, 0,
        "a sealed non-clinical row projects NOTHING — no custody, no clear view, no leak"
    );

    // The watermark never froze: a subsequent LEGITIMATE (unsealed) demographic event still
    // applies and projects — proof the door was not wedged by the sealed ciphertext noise.
    let good = demographic_body(&kid_d, patient, peer_hlc(WALL_2026 + 1));
    let good_signed = sign(&good, &sk_d).unwrap();
    c.execute(
        "SELECT apply_remote_event($1)",
        &[&good_signed.signed_bytes],
    )
    .await
    .expect("a legitimate unsealed demographic still applies after the sealed noise");
    let dob: i64 = c
        .query_one(
            "SELECT count(*) FROM patient_demographic WHERE patient_id = $1::text::uuid AND field = 'dob'",
            &[&patient.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        dob, 1,
        "the legit unsealed demographic projected — the apply door was never wedged"
    );
}

#[tokio::test]
async fn sealed_identity_link_with_malformed_field_admits_without_wedge() {
    // db/018 patient_link_apply cast-before-guard wedge (code-review finding #1). The seal
    // guard `IF NEW.sealed THEN RETURN NULL` must fire BEFORE the DECLARE-block
    // `(p ->> 'subject_a')::uuid` casts. A hostile enrolled peer mints a sealed
    // identity.link whose payload carries a NON-UUID top-level subject_a; the lenient apply
    // door SKIPS the structural floor for a sealed no-custody row, so the garbage is never
    // validated and reaches the AFTER-INSERT trigger. Pre-fix the DECLARE cast raises
    // `invalid input syntax for type uuid` before the guard — aborting apply_remote_event on
    // a VERIFIABLE event, freezing do_pull's watermark, and WEDGING clinical sync forever
    // (the very availability-floor failure the seal-robustness work exists to prevent, but
    // db/018 was the one projection that casts a body field in DECLARE rather than after the
    // guard). Post-fix the guard returns NULL first: admit as harmless ciphertext noise.
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk_d, kid_d, _sk_h, _kid_h) = setup(&c).await;
    let patient = Uuid::now_v7();
    // setup() does not truncate the identity projection (no FK to event_log): clear it so the
    // "no edge projected" assertion below is exact regardless of sibling-test residue.
    c.execute("TRUNCATE patient_link CASCADE", &[])
        .await
        .unwrap();

    let body = malformed_sealed_link_body(&kid_d, patient, peer_hlc(WALL_2026));
    let event_id = body.event_id.clone();
    let signed = sign(&body, &sk_d).unwrap();

    let returned: String = c
        .query_one(
            "SELECT apply_remote_event($1)::text",
            &[&signed.signed_bytes],
        )
        .await
        .expect("a sealed identity-link with a malformed field is ADMITTED, never wedges apply")
        .get(0);
    assert_eq!(returned, event_id, "the door returns the event id");

    let sealed: bool = c
        .query_one(
            "SELECT sealed FROM event_log WHERE event_id = $1::text::uuid",
            &[&event_id],
        )
        .await
        .expect("the event was admitted into the log")
        .get(0);
    assert!(
        sealed,
        "the sealed identity-link row is admitted, marked sealed"
    );

    let links: i64 = c
        .query_one("SELECT count(*) FROM patient_link", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        links, 0,
        "a sealed identity-link projects no edge — ciphertext noise only, no wedge"
    );

    // The watermark never froze: a subsequent legit event still applies through the door.
    let good = demographic_body(&kid_d, patient, peer_hlc(WALL_2026 + 1));
    let good_signed = sign(&good, &sk_d).unwrap();
    c.execute(
        "SELECT apply_remote_event($1)",
        &[&good_signed.signed_bytes],
    )
    .await
    .expect("a legit event still applies after the sealed identity-link noise — no wedge");
}
