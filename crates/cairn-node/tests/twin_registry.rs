//! #173 — the cairn_event_twin registry. DB-gated on $CAIRN_TEST_PG, serialized
//! cluster-wide via db::test_serial_guard (same idiom as medication.rs). Part 1 (this
//! file, Task 1): the registry table's validation trigger fails closed on a check_fn that
//! does not exist with the unified (text, jsonb) signature. Part 2 (Task 3) adds per-type
//! dispatch tests.
use cairn_node::db;

fn cs() -> Option<String> {
    std::env::var("CAIRN_TEST_PG").ok()
}

/// The real RAISE EXCEPTION text (tokio_postgres wraps DB errors as a generic "db error").
fn db_msg(e: &tokio_postgres::Error) -> String {
    e.as_db_error()
        .map(|d| d.message().to_string())
        .unwrap_or_else(|| e.to_string())
}

#[tokio::test]
async fn registry_trigger_rejects_missing_check_fn() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();

    // Defensive cleanup: clear any `test.*` residue an earlier interrupted run may have
    // leaked into the shared registry (rows are otherwise deleted per-insert below), so this
    // test and the count assertion in registry_is_seeded_with_the_expected_mapping stay
    // robust regardless of prior-run state.
    c.execute(
        "DELETE FROM cairn_event_twin_check WHERE event_type LIKE 'test.%'",
        &[],
    )
    .await
    .unwrap();

    // A registry row naming a function that does not exist is refused at insert time.
    let err = c
        .execute(
            "INSERT INTO cairn_event_twin_check (event_type, check_fn, twin_required_msg) \
             VALUES ('test.bogus.asserted', 'cairn_check_does_not_exist', 'x')",
            &[],
        )
        .await
        .expect_err("bogus check_fn must be rejected");
    assert!(
        db_msg(&err).contains("does not exist"),
        "unexpected: {}",
        db_msg(&err)
    );

    // A row naming an existing (text, jsonb) check fn is accepted, then cleaned up.
    c.execute(
        "INSERT INTO cairn_event_twin_check (event_type, check_fn, twin_required_msg) \
         VALUES ('test.ok.asserted', 'cairn_check_medication_assertion', 'x')",
        &[],
    )
    .await
    .expect("valid (text,jsonb) check fn must be accepted");
    c.execute(
        "DELETE FROM cairn_event_twin_check WHERE event_type = 'test.ok.asserted'",
        &[],
    )
    .await
    .unwrap();

    // A row with NULL check_fn (twin-required-only, no structural check) is accepted.
    c.execute(
        "INSERT INTO cairn_event_twin_check (event_type, check_fn, twin_required_msg) \
         VALUES ('test.nullfn.asserted', NULL, 'x')",
        &[],
    )
    .await
    .expect("NULL check_fn must be accepted");
    c.execute(
        "DELETE FROM cairn_event_twin_check WHERE event_type = 'test.nullfn.asserted'",
        &[],
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn registry_is_seeded_with_the_expected_mapping() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();

    // Robustness: ignore any `test.*` residue a prior interrupted run may have leaked, so the
    // count reflects only the migration-seeded rows.
    c.execute(
        "DELETE FROM cairn_event_twin_check WHERE event_type LIKE 'test.%'",
        &[],
    )
    .await
    .unwrap();

    // Assert the full 15-row mapping is present so a dropped registration is caught.
    let n: i64 = c
        .query_one("SELECT count(*) FROM cairn_event_twin_check", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(n, 15, "expected 15 seeded twin-check rows");

    // Lock the FULL registry contract. This table is now the single source of floor-wiring
    // truth, so assert every (event_type → check_fn, twin_required_msg) mapping byte-for-byte
    // rather than a count + one spot-check: a future slice that mis-points a check_fn or
    // mis-transcribes a twin_required_msg is caught here directly, not merely if the broad
    // behaviour suite happens to exercise that exact negative path. Strings are transcribed
    // verbatim from the seeding migrations (db/010–033).
    let mut expected: Vec<(&str, &str, &str)> = vec![
        (
            "demographic.identifier.asserted",
            "cairn_check_identifier_assertion",
            "demographic assertion requires a non-empty authored twin (§4.5)",
        ),
        (
            "demographic.field.asserted",
            "cairn_check_demographic_field",
            "demographic assertion requires a non-empty authored twin (§4.5)",
        ),
        (
            "identity.link.asserted",
            "cairn_check_link_assertion",
            "identity linkage assertion requires a non-empty authored twin (§5.7)",
        ),
        (
            "identity.unlink.asserted",
            "cairn_check_link_assertion",
            "identity linkage assertion requires a non-empty authored twin (§5.7)",
        ),
        (
            "identity.dispute.asserted",
            "cairn_check_dispute_assertion",
            "identity dispute assertion requires a non-empty authored twin (§5.7)",
        ),
        (
            "identity.dispute.resolved",
            "cairn_check_dispute_assertion",
            "identity dispute assertion requires a non-empty authored twin (§5.7)",
        ),
        (
            "identity.pending.asserted",
            "cairn_check_identity_state_assertion",
            "identity-state assertion requires a non-empty authored twin (§5.7)",
        ),
        (
            "identity.identify.asserted",
            "cairn_check_identity_state_assertion",
            "identity-state assertion requires a non-empty authored twin (§5.7)",
        ),
        (
            "identity.repudiate.asserted",
            "cairn_check_repudiation_assertion",
            "identity repudiation assertion requires a non-empty authored twin (§5.7)",
        ),
        (
            "clinical.medication.asserted",
            "cairn_check_medication_assertion",
            "medication assertion requires a non-empty authored twin (§3.13/§3.15)",
        ),
        (
            "clinical.medication-cessation.asserted",
            "cairn_check_medication_assertion",
            "medication assertion requires a non-empty authored twin (§3.13/§3.15)",
        ),
        (
            "clinical.medication-dose-change.asserted",
            "cairn_check_medication_dose",
            "medication dose assertion requires a non-empty authored twin (§3.13/§3.15)",
        ),
        (
            "clinical.medication-dose-correction.asserted",
            "cairn_check_medication_dose",
            "medication dose assertion requires a non-empty authored twin (§3.13/§3.15)",
        ),
        (
            "clinical.medication-reconciliation.asserted",
            "cairn_check_medication_reconciliation",
            "medication reconciliation requires a non-empty authored twin (§3.13/§3.15)",
        ),
        (
            "clinical.medication-separation.asserted",
            "cairn_check_medication_reconciliation",
            "medication reconciliation requires a non-empty authored twin (§3.13/§3.15)",
        ),
    ];
    expected.sort();

    // Sort BOTH sides in Rust (byte-lexicographic) so the comparison never depends on the
    // node's default TEXT collation for ORDER BY. get::<_, String> also asserts non-null:
    // all 15 seed rows carry both a check_fn and a twin_required_msg.
    let rows = c
        .query(
            "SELECT event_type, check_fn, twin_required_msg FROM cairn_event_twin_check",
            &[],
        )
        .await
        .unwrap();
    let mut actual: Vec<(String, String, String)> = rows
        .iter()
        .map(|r| {
            (
                r.get::<_, String>(0),
                r.get::<_, String>(1),
                r.get::<_, String>(2),
            )
        })
        .collect();
    actual.sort();
    let actual_ref: Vec<(&str, &str, &str)> = actual
        .iter()
        .map(|(et, cf, msg)| (et.as_str(), cf.as_str(), msg.as_str()))
        .collect();

    assert_eq!(
        actual_ref, expected,
        "registry mapping drifted from the verbatim seed contract"
    );
}

#[tokio::test]
async fn dispatch_runs_the_registered_structural_check() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();

    // Call the dispatcher directly with a structurally-invalid link body (empty payload →
    // no subjects). cairn_check_link_assertion must fire and RAISE — proof the registry
    // dispatched to a check, not the skeleton. (An authored twin is present, so a raise can
    // only come from the structural check running BEFORE the authored-twin return.)
    let body = r#"{"schema_version":"identity.link/1",
                   "patient_id":"00000000-0000-0000-0000-000000000001",
                   "plaintext_twin":"linked","payload":{}}"#;
    // NOTE: cast as $1::text::jsonb, not $1::jsonb — with a bare ::jsonb cast, Postgres's
    // parameter-type inference reports OID jsonb for $1, and tokio-postgres's `ToSql` for
    // `&str` only accepts TEXT/VARCHAR/NAME/UNKNOWN, so binding fails client-side with a
    // `WrongType` error *before* the query ever reaches the server. Because that client-side
    // error also satisfies `.expect_err(...)`, the bare-cast form is a false green: it never
    // proves dispatch reached the check. `$1::text::jsonb` matches the established codebase
    // idiom (see recall_epoch.rs) — parameter type resolves to text, cast to jsonb happens
    // server-side after binding.
    let err = c
        .query_one(
            "SELECT cairn_event_twin('identity.link.asserted', $1::text::jsonb)",
            &[&body],
        )
        .await
        .expect_err("an invalid link body must be refused by the dispatched check");
    let msg = db_msg(&err);
    assert!(!msg.is_empty());
    assert!(
        msg.contains("§5.7") || msg.contains("link assertion"),
        "expected a link-assertion structural-check message, got: {msg}"
    );
}

#[tokio::test]
async fn unregistered_type_gets_skeleton_no_raise() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();

    // A type with no registry row and no authored twin returns the mechanical skeleton and
    // does NOT raise (honest degradation, ADR-0039) — matches note.added behaviour today.
    let body = r#"{"schema_version":"note/1",
                   "patient_id":"00000000-0000-0000-0000-000000000001",
                   "payload":{"text":"hi"}}"#;
    // $1::text::jsonb — see the comment in dispatch_runs_the_registered_structural_check for
    // why a bare $1::jsonb cast fails client-side under tokio-postgres.
    let twin: String = c
        .query_one(
            "SELECT cairn_event_twin('note.added', $1::text::jsonb)",
            &[&body],
        )
        .await
        .expect("unregistered type must not raise")
        .get(0);
    assert!(twin.contains("[note.added]"));
}

#[tokio::test]
async fn registered_type_absent_twin_raises_required_msg() {
    let Some(base) = cs() else { return };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();

    // A STRUCTURALLY-VALID link body (distinct valid subjects + non-empty provenance) that
    // passes the dispatched floor check but carries NO authored twin. This isolates the
    // twin-REQUIRED path driven by the registry's twin_required_msg column: the structural
    // check passes, so the only remaining raise is the hard-require branch. Proves the
    // twin-required policy is sourced from the registry row (ADR-0039 hard-require), not from
    // any residual per-type dispatch code — a path the structural-check and skeleton tests
    // above do not exercise.
    let body = r#"{"schema_version":"identity.link/1",
                   "patient_id":"00000000-0000-0000-0000-000000000001",
                   "payload":{"subject_a":"00000000-0000-0000-0000-0000000000aa",
                              "subject_b":"00000000-0000-0000-0000-0000000000bb",
                              "provenance":"test"}}"#;
    // $1::text::jsonb — see dispatch_runs_the_registered_structural_check for why a bare
    // $1::jsonb cast false-greens under tokio-postgres.
    let err = c
        .query_one(
            "SELECT cairn_event_twin('identity.link.asserted', $1::text::jsonb)",
            &[&body],
        )
        .await
        .expect_err("a registered twin-required type with no authored twin must raise");
    let msg = db_msg(&err);
    assert!(
        msg.contains("requires a non-empty authored twin") && msg.contains("§5.7"),
        "expected the registry twin_required_msg for a link assertion, got: {msg}"
    );
}
