//! Integration coverage for issue #109 — wiring the ADR-0040 legibility primitives
//! (`cairn_verify_error`, the twin-provenance `verifiable` column) into the doors.
//!
//! Real Postgres, gated on `$CAIRN_TEST_PG`, serialized cluster-wide via
//! `db::test_serial_guard`. Two properties:
//!
//!  1. Every signature-gating door still RAISEs on a malformed/unverifiable blob
//!     (the boolean floor is unchanged) but now attaches the legible rejection
//!     reason from `cairn_verify_error` as the exception DETAIL. Before this, a
//!     context mismatch (wire-format skew / pre-ADR-0040 blob) was indistinguishable
//!     from tampering at the SQL boundary — during a skew-induced write outage that
//!     misdirects the operator badly (principle 4, honest degradation).
//!
//!  2. `event_twin_provenance` exposes a `verifiable` column, so a row whose bytes no
//!     longer verify (e.g. a pre-ADR-0040 legacy row in an upgraded-in-place dev DB)
//!     is surfaced as unverifiable instead of being silently misclassified as
//!     "author omitted the twin" (`twin_authored = false`) — which would let a
//!     worklist re-derive skeletons over genuinely authored twins.
use cairn_node::db;
use tokio_postgres::Client;

fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

/// The four self-described bytes we use as a stand-in for "cannot verify". Any
/// blob that is not a valid COSE_Sign1/Ed25519 event drives `cairn_verify` false
/// and `cairn_verify_error` non-NULL — exactly the door path under test.
const BAD: &[u8] = b"\xde\xad\xbe\xef";

/// Call `door(BAD)`, require it to fail with a DB error, and return the
/// (primary message, DETAIL) pair so the caller can assert the boolean floor is
/// intact AND the legible reason now rides along as DETAIL.
async fn detail_of(c: &Client, door_sql: &str, door_name: &str) -> (String, String) {
    let bad = BAD.to_vec();
    let err = c
        .execute(door_sql, &[&bad])
        .await
        .expect_err(&format!("{door_name}: malformed blob must be rejected"));
    let db = err
        .as_db_error()
        .unwrap_or_else(|| panic!("{door_name}: expected a DB error, got {err}"));
    let message = db.message().to_string();
    let detail = db.detail().unwrap_or("").to_string();
    (message, detail)
}

#[tokio::test]
async fn every_door_surfaces_the_verify_error_as_detail() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    // restore_node_event fences on an already-enrolled node (`local_node`) BEFORE the
    // verify gate — clear it so a fresh-node restore actually reaches the signature
    // floor under test. The other four doors hit verify right after the size ceiling,
    // independent of node state.
    c.batch_execute("DELETE FROM local_node").await.unwrap();

    // All five signature-gating doors. Each takes a single leading bytea; the
    // clinical two have NULL-default trailing args, so a one-arg call is valid.
    // A tiny malformed blob passes the size ceiling and hits the verify gate
    // before any peer/genesis/enrollment logic, so no fixtures are needed.
    let doors = [
        ("submit_event", "SELECT submit_event($1)"),
        ("apply_remote_event", "SELECT apply_remote_event($1)"),
        ("submit_node_event", "SELECT submit_node_event($1)"),
        (
            "apply_remote_node_event",
            "SELECT apply_remote_node_event($1)",
        ),
        ("restore_node_event", "SELECT restore_node_event($1)"),
    ];

    for (name, sql) in doors {
        let (message, detail) = detail_of(&c, sql, name).await;
        assert!(
            message.contains("verification failed"),
            "{name}: primary message still cites the verify floor, got: {message}"
        );
        assert!(
            !detail.trim().is_empty(),
            "{name}: DETAIL must carry the legible cairn_verify_error reason, got empty"
        );
        // The generic placeholder must never be what surfaces — the whole point is a
        // real reason from the verify vocabulary.
        assert_ne!(
            detail.trim(),
            "unknown",
            "{name}: DETAIL fell back to the coalesce placeholder instead of a real reason"
        );
    }
}

#[tokio::test]
async fn twin_provenance_exposes_verifiable_and_distinguishes_unverifiable_rows() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    c.batch_execute("TRUNCATE event_log, actor_event, patient_chart, patient_identifier CASCADE")
        .await
        .unwrap();

    // Insert a row whose signed_bytes do NOT verify, directly (as the owning role —
    // the C5.4 raw-INSERT floor only blocks cairn_agent). This is the pre-ADR-0040
    // legacy-row scenario from the issue: bytes that once verified no longer do.
    // The UUIDs are minted in SQL (the dev-dep tokio-postgres has no uuid feature —
    // same reason twin_globalise.rs never binds a uuid); we tag the row with a unique
    // text `node_origin` marker and query the view back by it.
    let marker = "legacy-109";
    c.execute(
        "INSERT INTO event_log (event_id, patient_id, event_type, schema_version,
             hlc_wall, hlc_counter, node_origin, signed_bytes, content_address,
             body, contributors, signer_key_id, plaintext_twin)
         VALUES (gen_random_uuid(), gen_random_uuid(), 'note.added', 'note/1', 1, 0, $1, $2::bytea,
             '\\x1220'::bytea || digest($2::bytea, 'sha256'),
             '{}'::jsonb, '[]'::jsonb, 'k', 'a once-authored twin')",
        &[&marker, &BAD.to_vec()],
    )
    .await
    .expect("owner may seed a legacy row directly");

    let row = c
        .query_one(
            "SELECT ep.verifiable, ep.twin_authored
               FROM event_twin_provenance ep JOIN event_log el USING (event_id)
              WHERE el.node_origin = $1",
            &[&marker],
        )
        .await
        .expect("event_twin_provenance must expose a verifiable column");
    let verifiable: bool = row.get(0);
    let twin_authored: bool = row.get(1);

    assert!(
        !verifiable,
        "an unverifiable row is reported verifiable=false (surfaced, not hidden)"
    );
    // twin_authored is false here too — but ONLY the verifiable column tells the
    // worklist this is "no longer verifies", not "author omitted the twin".
    assert!(
        !twin_authored,
        "unverifiable bytes cannot be read as an authored twin"
    );
}
