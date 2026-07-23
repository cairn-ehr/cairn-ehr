use cairn_event::{event_address, short_fingerprint, sign, EventBody, Hlc, PairingBundle};
use cairn_node::{db, identity, keystore};

fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

#[tokio::test]
async fn admission_admits_trusted_peer_genesis_and_rejects_strangers() {
    let Some(base) = cs() else {
        eprintln!("skipped");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap(); // serialize shared-DB tests
    let a = db::connect_and_load_schema(&base).await.unwrap();
    // Re-runnable: truncate before provisioning.
    a.batch_execute("TRUNCATE node_event, local_node")
        .await
        .ok();

    let tmp = tempfile::tempdir().unwrap();
    let (sk_a, kid_a) = keystore::generate_plaintext(&tmp.path().join("a.key")).unwrap();
    identity::provision(&a, &sk_a, &kid_a, "A", "127.0.0.1:7800")
        .await
        .unwrap();

    // B's genesis (authored against B's own key), captured as signed bytes.
    let (sk_b, kid_b) = cairn_event::generate_key().unwrap();
    let body_b = EventBody {
        event_id: uuid::Uuid::now_v7().to_string(),
        patient_id: identity::NIL_PATIENT.into(),
        event_type: "node.enrolled".into(),
        schema_version: "node/1".into(),
        hlc: Hlc {
            wall: 0,
            counter: 0,
            node_origin: "B".into(),
        },
        t_effective: None,
        signer_key_id: kid_b.clone(),
        contributors: serde_json::json!([{"actor_id": kid_b, "role": "device"}]),
        payload: serde_json::json!({"display_name":"B","address":"127.0.0.1:7801"}),
        attachments: vec![],
        plaintext_twin: None,
        clock_grade: cairn_event::ClockGrade::SelfAsserted,
    };
    let signed_b = sign(&body_b, &sk_b).unwrap();
    let b_node_id = hex::encode(event_address(&signed_b.signed_bytes));

    // Before A peers with B, B's genesis is rejected (deny-all).
    let bytes = signed_b.signed_bytes.clone();
    let r = a
        .execute("SELECT apply_remote_node_event($1)", &[&bytes])
        .await;
    assert!(r.is_err(), "un-trusted genesis must be rejected");
    eprintln!("REJECT 1 (un-peered): {:?}", r.unwrap_err());

    // A pairs with B (records peer.added with B's real node_id + pubkey + fingerprint).
    let bundle = cairn_event::PairingBundle {
        node_id_hex: b_node_id.clone(),
        pubkey_hex: kid_b.clone(),
        address: "127.0.0.1:7801".into(),
        fingerprint: cairn_event::short_fingerprint(&kid_b).unwrap(),
        nonce: "n".into(),
        hlc: Hlc {
            wall: 0,
            counter: 0,
            node_origin: b_node_id.clone(),
        },
    };
    identity::author_peer(&a, &sk_a, &kid_a, "A", &bundle, Some("peer"))
        .await
        .unwrap();

    // Now B's genesis is admitted.
    let bytes = signed_b.signed_bytes.clone();
    a.execute("SELECT apply_remote_node_event($1)", &[&bytes])
        .await
        .unwrap();
    eprintln!("ADMIT: B genesis accepted after peering");

    // After unpeering B, a NEW B-authored peer event is rejected.
    identity::author_unpeer(&a, &sk_a, &kid_a, "A", &b_node_id)
        .await
        .unwrap();
    let body_b2 = EventBody {
        event_id: uuid::Uuid::now_v7().to_string(),
        event_type: "peer.added".into(),
        payload: serde_json::json!({"peer_node_id_hex":"aa","peer_pubkey":"bb","fingerprint":"X"}),
        ..body_b.clone()
    };
    let signed_b2 = sign(&body_b2, &sk_b).unwrap();
    let bytes = signed_b2.signed_bytes.clone();
    let r2 = a
        .execute("SELECT apply_remote_node_event($1)", &[&bytes])
        .await;
    assert!(r2.is_err(), "events from a revoked peer must be rejected");
    eprintln!("REJECT 2 (revoked peer): {:?}", r2.unwrap_err());
}

// REC1 — unknown signer: a peer.added event signed by a key that has no
// enrolled node entry must be rejected with "maps to no known node".
#[tokio::test]
async fn admission_rejects_peer_event_from_an_unknown_signer() {
    let Some(base) = cs() else {
        eprintln!("skipped");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap(); // serialize shared-DB tests
    let a = db::connect_and_load_schema(&base).await.unwrap();
    a.batch_execute("TRUNCATE node_event, local_node")
        .await
        .ok();

    // Provision node A so the DB is in a valid enrolled state.
    let tmp = tempfile::tempdir().unwrap();
    let (sk_a, kid_a) = keystore::generate_plaintext(&tmp.path().join("a.key")).unwrap();
    identity::provision(&a, &sk_a, &kid_a, "A", "127.0.0.1:7810")
        .await
        .unwrap();

    // Generate a key Z that is NEVER enrolled as a node.
    let (sk_z, kid_z) = cairn_event::generate_key().unwrap();

    // Build a peer.added body signed by sk_z; kid_z resolves to no known node.
    let body_z = EventBody {
        event_id: uuid::Uuid::now_v7().to_string(),
        patient_id: identity::NIL_PATIENT.into(),
        event_type: "peer.added".into(),
        schema_version: "node/1".into(),
        hlc: Hlc {
            wall: 0,
            counter: 0,
            node_origin: "Z".into(),
        },
        t_effective: None,
        signer_key_id: kid_z.clone(),
        contributors: serde_json::json!([{"actor_id": kid_z, "role": "device"}]),
        payload: serde_json::json!({
            "peer_node_id_hex": "aabbccdd".repeat(8), // 64 hex chars — arbitrary
            "peer_pubkey": kid_z,
            "fingerprint": "X"
        }),
        attachments: vec![],
        plaintext_twin: None,
        clock_grade: cairn_event::ClockGrade::SelfAsserted,
    };
    let signed_z = sign(&body_z, &sk_z).unwrap();
    let bytes = signed_z.signed_bytes.clone();

    let r = a
        .execute("SELECT apply_remote_node_event($1)", &[&bytes])
        .await;
    assert!(
        r.is_err(),
        "peer event from an unknown signer must be rejected (REC1)"
    );
    let pg_err = r.unwrap_err();
    let db_msg = pg_err
        .as_db_error()
        .map(|e| e.message())
        .unwrap_or("<no db message>");
    eprintln!("REJECT REC1 (unknown signer): {db_msg}");
    assert!(
        db_msg.contains("maps to no known node"),
        "expected 'maps to no known node' in error, got: {db_msg}"
    );
}

// REC2 — genesis key mismatch: A has peered with B recording a WRONG pubkey.
// When B's real genesis arrives, the admission predicate
// (peer_node_id = content_address AND peer_pubkey = signer_key_id) fails because
// the pinned pubkey does not match B's actual genesis signer.
#[tokio::test]
async fn admission_rejects_genesis_when_pinned_pubkey_mismatches_signer() {
    let Some(base) = cs() else {
        eprintln!("skipped");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap(); // serialize shared-DB tests
    let a = db::connect_and_load_schema(&base).await.unwrap();
    a.batch_execute("TRUNCATE node_event, local_node")
        .await
        .ok();

    // Provision node A.
    let tmp = tempfile::tempdir().unwrap();
    let (sk_a, kid_a) = keystore::generate_plaintext(&tmp.path().join("a.key")).unwrap();
    identity::provision(&a, &sk_a, &kid_a, "A", "127.0.0.1:7820")
        .await
        .unwrap();

    // Build B's REAL genesis signed by sk_b.
    let (sk_b, kid_b) = cairn_event::generate_key().unwrap();
    let body_b = EventBody {
        event_id: uuid::Uuid::now_v7().to_string(),
        patient_id: identity::NIL_PATIENT.into(),
        event_type: "node.enrolled".into(),
        schema_version: "node/1".into(),
        hlc: Hlc {
            wall: 0,
            counter: 0,
            node_origin: "B".into(),
        },
        t_effective: None,
        signer_key_id: kid_b.clone(),
        contributors: serde_json::json!([{"actor_id": kid_b, "role": "device"}]),
        payload: serde_json::json!({"display_name": "B", "address": "127.0.0.1:7821"}),
        attachments: vec![],
        plaintext_twin: None,
        clock_grade: cairn_event::ClockGrade::SelfAsserted,
    };
    let signed_b = sign(&body_b, &sk_b).unwrap();
    let b_node_id = hex::encode(event_address(&signed_b.signed_bytes));

    // Generate a DIFFERENT (wrong) key X — this will be pinned in A's trust_peer instead of kid_b.
    let (_sk_x, kid_x) = cairn_event::generate_key().unwrap();

    // A peers with B but records kid_x (the WRONG pubkey) instead of kid_b.
    let bundle_wrong = PairingBundle {
        node_id_hex: b_node_id.clone(),
        pubkey_hex: kid_x.clone(),
        address: "127.0.0.1:7821".into(),
        fingerprint: short_fingerprint(&kid_x).unwrap(),
        nonce: "n".into(),
        hlc: Hlc {
            wall: 0,
            counter: 0,
            node_origin: b_node_id.clone(),
        },
    };
    identity::author_peer(&a, &sk_a, &kid_a, "A", &bundle_wrong, Some("peer"))
        .await
        .unwrap();

    // B's real genesis now arrives: content_address = b_node_id (matches trust_peer.peer_node_id)
    // but signer_key_id = kid_b, while trust_peer.peer_pubkey = kid_x — mismatch → must reject.
    let bytes = signed_b.signed_bytes.clone();
    let r = a
        .execute("SELECT apply_remote_node_event($1)", &[&bytes])
        .await;
    assert!(
        r.is_err(),
        "genesis with mismatched pinned pubkey must be rejected (REC2)"
    );
    let pg_err = r.unwrap_err();
    let db_msg = pg_err
        .as_db_error()
        .map(|e| e.message())
        .unwrap_or("<no db message>");
    eprintln!("REJECT REC2 (genesis key mismatch): {db_msg}");
    assert!(
        db_msg.contains("un-trusted or mismatched"),
        "expected 'un-trusted or mismatched' in error, got: {db_msg}"
    );

    // Positive control: with the CORRECT key pinned, the same genesis IS admitted.
    // (Proves the test is not vacuously failing for an unrelated reason.)
    a.batch_execute("TRUNCATE node_event, local_node")
        .await
        .ok();
    let (sk_a2, kid_a2) = cairn_event::generate_key().unwrap();
    let body_a2 = EventBody {
        event_id: uuid::Uuid::now_v7().to_string(),
        patient_id: identity::NIL_PATIENT.into(),
        event_type: "node.enrolled".into(),
        schema_version: "node/1".into(),
        hlc: Hlc {
            wall: 0,
            counter: 0,
            node_origin: "A2".into(),
        },
        t_effective: None,
        signer_key_id: kid_a2.clone(),
        contributors: serde_json::json!([{"actor_id": kid_a2, "role": "device"}]),
        payload: serde_json::json!({"display_name": "A2", "address": "127.0.0.1:7822"}),
        attachments: vec![],
        plaintext_twin: None,
        clock_grade: cairn_event::ClockGrade::SelfAsserted,
    };
    let signed_a2 = sign(&body_a2, &sk_a2).unwrap();
    let a2_bytes = signed_a2.signed_bytes.clone();
    a.execute("SELECT submit_node_event($1)", &[&a2_bytes])
        .await
        .unwrap();

    let bundle_correct = PairingBundle {
        node_id_hex: b_node_id.clone(),
        pubkey_hex: kid_b.clone(), // correct key this time
        address: "127.0.0.1:7821".into(),
        fingerprint: short_fingerprint(&kid_b).unwrap(),
        nonce: "n2".into(),
        hlc: Hlc {
            wall: 0,
            counter: 0,
            node_origin: b_node_id.clone(),
        },
    };
    identity::author_peer(&a, &sk_a2, &kid_a2, "A2", &bundle_correct, Some("peer"))
        .await
        .unwrap();

    let bytes_ok = signed_b.signed_bytes.clone();
    a.execute("SELECT apply_remote_node_event($1)", &[&bytes_ok])
        .await
        .expect("positive control: genesis with correct pinned pubkey must be admitted");
    eprintln!("POSITIVE CONTROL REC2: genesis admitted when pinned pubkey matches signer");
}

// Issue #201 — `node.superseded` must REPLICATE through the peer-admission door.
// The submit door (db/007) and the restore door (db/009) both emit/apply it, but the
// apply-door op map omitted it, so a peer pulling a restored node's history refused
// the lineage event forever: the pull loop skip-and-sweeps it on every full sweep
// (busy-loop noise) and the node plane's set-union guarantee is permanently violated
// for that event. Lineage is a signed, attributable CLAIM (principle 2) consumed only
// by the advisory node_lineage view — node_current resolves keys from `enroll` rows
// only and trust_peer reads only `peer`/`revoke`, so a false supersede can hijack
// neither key resolution nor peer trust — which makes admitting it from an active
// peer exactly as trust-bounded as peer/revoke.
#[tokio::test]
async fn admission_admits_supersede_from_an_active_peer() {
    let Some(base) = cs() else {
        eprintln!("skipped");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap(); // serialize shared-DB tests
    let a = db::connect_and_load_schema(&base).await.unwrap();
    a.batch_execute("TRUNCATE node_event, local_node")
        .await
        .ok();

    // Provision A, then peer it with B and admit B's genesis (the setup a real
    // restored-node pull sits on: the restored node was re-admitted out-of-band).
    let tmp = tempfile::tempdir().unwrap();
    let (sk_a, kid_a) = keystore::generate_plaintext(&tmp.path().join("a.key")).unwrap();
    identity::provision(&a, &sk_a, &kid_a, "A", "127.0.0.1:7830")
        .await
        .unwrap();

    let (sk_b, kid_b) = cairn_event::generate_key().unwrap();
    let body_b = EventBody {
        event_id: uuid::Uuid::now_v7().to_string(),
        patient_id: identity::NIL_PATIENT.into(),
        event_type: "node.enrolled".into(),
        schema_version: "node/1".into(),
        hlc: Hlc {
            wall: 0,
            counter: 0,
            node_origin: "B".into(),
        },
        t_effective: None,
        signer_key_id: kid_b.clone(),
        contributors: serde_json::json!([{"actor_id": kid_b, "role": "device"}]),
        payload: serde_json::json!({"display_name": "B", "address": "127.0.0.1:7831"}),
        attachments: vec![],
        plaintext_twin: None,
        clock_grade: cairn_event::ClockGrade::SelfAsserted,
    };
    let signed_b = sign(&body_b, &sk_b).unwrap();
    let b_node_id = hex::encode(event_address(&signed_b.signed_bytes));
    let bundle = PairingBundle {
        node_id_hex: b_node_id.clone(),
        pubkey_hex: kid_b.clone(),
        address: "127.0.0.1:7831".into(),
        fingerprint: short_fingerprint(&kid_b).unwrap(),
        nonce: "n".into(),
        hlc: Hlc {
            wall: 0,
            counter: 0,
            node_origin: b_node_id.clone(),
        },
    };
    identity::author_peer(&a, &sk_a, &kid_a, "A", &bundle, Some("peer"))
        .await
        .unwrap();
    let bytes = signed_b.signed_bytes.clone();
    a.execute("SELECT apply_remote_node_event($1)", &[&bytes])
        .await
        .expect("setup: B's genesis must be admitted");

    // B — a restored node — records that it supersedes its dead predecessor D.
    // D's node-id needs no local enroll: the receiving peer may never have known
    // the dead node (neither submit nor restore checks subject existence either).
    let dead_node_id_hex = hex::encode(event_address(b"the dead predecessor node"));
    let body_sup = EventBody {
        event_id: uuid::Uuid::now_v7().to_string(),
        patient_id: identity::NIL_PATIENT.into(),
        event_type: "node.superseded".into(),
        schema_version: "node/1".into(),
        hlc: Hlc {
            wall: 0,
            counter: 0,
            node_origin: "B".into(),
        },
        t_effective: None,
        signer_key_id: kid_b.clone(),
        contributors: serde_json::json!([{"actor_id": kid_b, "role": "device"}]),
        payload: serde_json::json!({ "superseded_node_id_hex": dead_node_id_hex }),
        attachments: vec![],
        plaintext_twin: None,
        clock_grade: cairn_event::ClockGrade::SelfAsserted,
    };
    let signed_sup = sign(&body_sup, &sk_b).unwrap();
    let bytes = signed_sup.signed_bytes.clone();
    a.execute("SELECT apply_remote_node_event($1)", &[&bytes])
        .await
        .expect("supersede from an active peer must be admitted (#201)");

    // The lineage claim lands in the advisory view: dead node -> B.
    let row = a
        .query_one(
            "SELECT encode(superseded_node_id,'hex'), encode(new_node_id,'hex') FROM node_lineage",
            &[],
        )
        .await
        .unwrap();
    assert_eq!(row.get::<_, String>(0), dead_node_id_hex, "lineage subject");
    assert_eq!(row.get::<_, String>(1), b_node_id, "lineage author");

    // Set-union idempotence: a re-offered supersede (every full sweep re-ships the
    // whole log) applies as a no-op, never an error, and never a duplicate row.
    let bytes = signed_sup.signed_bytes.clone();
    a.execute("SELECT apply_remote_node_event($1)", &[&bytes])
        .await
        .expect("re-applying the same supersede must be idempotent");
    let n: i64 = a
        .query_one("SELECT count(*) FROM node_event WHERE op='supersede'", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(n, 1, "exactly one supersede row after re-apply");

    // Deny-all holds: a supersede signed by a key with no admitted enroll is refused
    // (the same author-resolution gate peer/revoke pass through).
    let (sk_z, kid_z) = cairn_event::generate_key().unwrap();
    let body_z = EventBody {
        event_id: uuid::Uuid::now_v7().to_string(),
        patient_id: identity::NIL_PATIENT.into(),
        event_type: "node.superseded".into(),
        schema_version: "node/1".into(),
        hlc: Hlc {
            wall: 0,
            counter: 0,
            node_origin: "Z".into(),
        },
        t_effective: None,
        signer_key_id: kid_z.clone(),
        contributors: serde_json::json!([{"actor_id": kid_z, "role": "device"}]),
        payload: serde_json::json!({ "superseded_node_id_hex": dead_node_id_hex }),
        attachments: vec![],
        plaintext_twin: None,
        clock_grade: cairn_event::ClockGrade::SelfAsserted,
    };
    let signed_z = sign(&body_z, &sk_z).unwrap();
    let bytes = signed_z.signed_bytes.clone();
    let r = a
        .execute("SELECT apply_remote_node_event($1)", &[&bytes])
        .await;
    assert!(
        r.is_err(),
        "supersede from an unknown signer must be refused"
    );
    let msg = r
        .unwrap_err()
        .as_db_error()
        .map(|e| e.message().to_string())
        .unwrap_or_default();
    assert!(
        msg.contains("maps to no known node"),
        "deny-all refusal must be the author-resolution gate, got: {msg}"
    );

    // Legible malformed guard: a trusted peer's supersede MISSING its payload field
    // is refused by name — never stored with a NULL/garbage subject.
    let body_bad = EventBody {
        event_id: uuid::Uuid::now_v7().to_string(),
        payload: serde_json::json!({}),
        ..body_sup.clone()
    };
    let signed_bad = sign(&body_bad, &sk_b).unwrap();
    let bytes = signed_bad.signed_bytes.clone();
    let r = a
        .execute("SELECT apply_remote_node_event($1)", &[&bytes])
        .await;
    assert!(
        r.is_err(),
        "supersede without its subject field must be refused"
    );
    let msg = r
        .unwrap_err()
        .as_db_error()
        .map(|e| e.message().to_string())
        .unwrap_or_default();
    assert!(
        msg.contains("superseded_node_id_hex"),
        "malformed refusal must name the missing field, got: {msg}"
    );
}
