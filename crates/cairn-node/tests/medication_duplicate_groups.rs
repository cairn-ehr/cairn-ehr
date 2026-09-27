//! `cairn_medication_duplicate_groups` (db/054)'s own SET semantics, case by case, calling
//! the SQL function directly. The source-level drift guard (`medication_dup_key_drift.rs`)
//! pins only its duplicate-key TEXT against db/033's; this file pins what it DOES. Its Rust
//! reader is `medication::read::read_reconciliation_flagged_groups`.
//!
//! DB-gated on $CAIRN_TEST_PG, serialized cluster-wide via `db::test_serial_guard`, same
//! conventions as `medication_read.rs` (`medication_setup` + `submit_registration` + the
//! medication orchestrators; a cross-chart group is built the same direct
//! `INSERT INTO medication_group_member` way `medication_read.rs`'s cross-patient tests
//! do). Linking (`person_member`/`patient_link`) is deliberately NOT exercised here — the
//! function takes an explicit chart array, not a linked person's set; that composition
//! (person_charts feeding this function) is pinned by `combined_read.rs`'s
//! `the_same_drug_on_two_linked_charts_is_flagged`.
mod common;

use cairn_event::SigningKey;
use cairn_node::db;
use cairn_node::medication::{
    assert_medication, cease_medication, AssertMedicationInput, CeaseMedicationInput,
    SubstanceCoding,
};
use common::{cs, medication_setup as setup, submit_registration};
use tokio_postgres::Client;
use uuid::Uuid;

/// Assert one uncoded medication and return its thread id (== its own group id until
/// reconciled). Mirrors `medication_read.rs`'s `assert_one`.
async fn assert_one(
    c: &mut Client,
    sk: &SigningKey,
    kid: &str,
    origin: &str,
    patient: Uuid,
    term: &str,
) -> Uuid {
    assert_medication(
        c,
        sk,
        kid,
        origin,
        patient,
        &AssertMedicationInput {
            term,
            coding: None,
            formulation: None,
            dose_amount: Some("500"),
            dose_unit: Some("mg"),
            sig: None,
            info_source: "patient",
            started: None,
            started_precision: None,
        },
        None,
        None,
    )
    .await
    .unwrap()
}

/// Assert one CODED medication (ADR-0059) and return its thread id. Used only by the
/// coded-vs-uncoded case (case 5).
async fn assert_coded(
    c: &mut Client,
    sk: &SigningKey,
    kid: &str,
    origin: &str,
    patient: Uuid,
    term: &str,
    code: &str,
) -> Uuid {
    assert_medication(
        c,
        sk,
        kid,
        origin,
        patient,
        &AssertMedicationInput {
            term,
            coding: Some(SubstanceCoding {
                system: "drugref-moiety",
                code,
                display: term,
            }),
            formulation: None,
            dose_amount: Some("500"),
            dose_unit: Some("mg"),
            sig: None,
            info_source: "patient",
            started: None,
            started_precision: None,
        },
        None,
        None,
    )
    .await
    .unwrap()
}

/// Fold two threads into ONE reconciled group, `group_id` set to the FIRST id given —
/// the same shape `cairn_recompute_medication_group` writes for a real reconciled pair,
/// and the same direct-insert idiom `medication_read.rs`'s cross-patient tests use to
/// reach a state the local door would otherwise refuse to construct directly (here it is
/// simpler: reconciling two threads on two DIFFERENT patients is exactly what db/033's
/// local door refuses, matching `medication_read.rs`'s own comment on why it inserts
/// directly rather than asserting an event).
async fn fold_into_one_group(c: &Client, group_id: Uuid, other: Uuid) {
    c.execute(
        "INSERT INTO medication_group_member (medication_id, group_id) VALUES \
         ($1::text::uuid, $1::text::uuid), ($2::text::uuid, $1::text::uuid)",
        &[&group_id.to_string(), &other.to_string()],
    )
    .await
    .unwrap();
}

/// Call `cairn_medication_duplicate_groups` directly with `charts`, returning the
/// flagged group ids it names (order not asserted — the function has no ORDER BY and
/// none is promised).
async fn duplicate_groups(c: &Client, charts: &[Uuid]) -> Vec<Uuid> {
    let chart_strs: Vec<String> = charts.iter().map(Uuid::to_string).collect();
    let rows = c
        .query(
            "SELECT g::text AS g FROM cairn_medication_duplicate_groups($1::text[]::uuid[]) g",
            &[&chart_strs],
        )
        .await
        .unwrap();
    rows.iter()
        .map(|r| r.get::<_, String>("g").parse().unwrap())
        .collect()
}

fn sorted(mut v: Vec<Uuid>) -> Vec<Uuid> {
    v.sort();
    v
}

/// Case 1: the same drug on two DIFFERENT charts, never reconciled — two distinct
/// groups sharing a dup_key. Queried with BOTH charts, the function must name both
/// group ids (the within-set property this whole slice exists for: a linked person's
/// combined list must catch a duplicate that no single chart's own reconciliation flag
/// would ever see, since `patient_medication_reconciliation_flag` groups by patient_id
/// alone). Queried with only ONE chart, it must return nothing — the positive control
/// that the flag really is about the SET, not some per-row property of either thread
/// alone (a bug that flagged unconditionally would still pass the [A,B] half of this
/// test but not this half).
#[tokio::test]
async fn case1_same_drug_two_charts_is_flagged_only_as_a_set() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid, _hsk, _hkid) = setup(&c).await;
    let a = Uuid::now_v7();
    let b = Uuid::now_v7();
    submit_registration(&c, &sk, &kid, a, 0).await;
    submit_registration(&c, &sk, &kid, b, 0).await;

    // Same drug, deliberately spelled with different case and trailing whitespace — the
    // dup_key normalises via `lower(btrim(term))`, so these must still collide.
    let thread_a = assert_one(&mut c, &sk, &kid, "origin-a", a, "metformin").await;
    let thread_b = assert_one(&mut c, &sk, &kid, "origin-a", b, "Metformin ").await;

    let with_both = sorted(duplicate_groups(&c, &[a, b]).await);
    assert_eq!(
        with_both,
        sorted(vec![thread_a, thread_b]),
        "both charts' groups must be named when the set spans both"
    );

    let with_a_alone = duplicate_groups(&c, &[a]).await;
    assert!(
        with_a_alone.is_empty(),
        "a single chart can never show a cross-chart duplicate on its own: {with_a_alone:?}"
    );
}

/// Case 2: the two threads from case 1, folded into ONE reconciled group — reconciling
/// IS the paper-parity resolution to a duplicate, so the flag must clear.
#[tokio::test]
async fn case2_reconciled_into_one_group_is_not_flagged() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid, _hsk, _hkid) = setup(&c).await;
    let a = Uuid::now_v7();
    let b = Uuid::now_v7();
    submit_registration(&c, &sk, &kid, a, 0).await;
    submit_registration(&c, &sk, &kid, b, 0).await;

    let thread_a = assert_one(&mut c, &sk, &kid, "origin-a", a, "metformin").await;
    let thread_b = assert_one(&mut c, &sk, &kid, "origin-a", b, "Metformin ").await;
    fold_into_one_group(&c, thread_a, thread_b).await;

    let flagged = duplicate_groups(&c, &[a, b]).await;
    assert!(
        flagged.is_empty(),
        "a reconciled pair is one group, not a duplicate: {flagged:?}"
    );
}

/// Case 3: one of the two threads ceased — a stopped drug is not a live duplicate. Only
/// active threads feed the dup_key computation (both here and in db/033's own view), the
/// same "struck line" convention `medication_read.rs` documents for display.
#[tokio::test]
async fn case3_one_ceased_is_not_flagged() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid, _hsk, _hkid) = setup(&c).await;
    let a = Uuid::now_v7();
    let b = Uuid::now_v7();
    submit_registration(&c, &sk, &kid, a, 0).await;
    submit_registration(&c, &sk, &kid, b, 0).await;

    let thread_a = assert_one(&mut c, &sk, &kid, "origin-a", a, "metformin").await;
    let _thread_b = assert_one(&mut c, &sk, &kid, "origin-a", b, "Metformin ").await;
    cease_medication(
        &mut c,
        &sk,
        &kid,
        "origin-a",
        a,
        thread_a,
        &CeaseMedicationInput {
            stopped: None,
            stopped_precision: None,
            reason: Some("no longer indicated"),
        },
        None,
        None,
    )
    .await
    .unwrap();

    let flagged = duplicate_groups(&c, &[a, b]).await;
    assert!(
        flagged.is_empty(),
        "one active + one ceased thread is not a live duplicate: {flagged:?}"
    );
}

/// Case 4: the SAME drug also lives on a chart C that is NOT in the queried array — the
/// function must answer only about the charts it was asked about. `p_charts` is an
/// explicit argument precisely so a caller controls the set; a function that scanned
/// every chart carrying the term regardless of `p_charts` would leak an unrelated
/// person's duplicate into this one's combined read.
#[tokio::test]
async fn case4_a_duplicate_outside_the_array_is_not_flagged() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid, _hsk, _hkid) = setup(&c).await;
    let a = Uuid::now_v7();
    let b = Uuid::now_v7();
    let outside = Uuid::now_v7();
    submit_registration(&c, &sk, &kid, a, 0).await;
    submit_registration(&c, &sk, &kid, b, 0).await;
    submit_registration(&c, &sk, &kid, outside, 0).await;

    // Only A holds the drug within [A, B]; the true duplicate partner is on `outside`,
    // which is never named in the query.
    assert_one(&mut c, &sk, &kid, "origin-a", a, "metformin").await;
    assert_one(&mut c, &sk, &kid, "origin-a", outside, "metformin").await;

    let flagged = duplicate_groups(&c, &[a, b]).await;
    assert!(
        flagged.is_empty(),
        "a duplicate partner outside p_charts must not be visible: {flagged:?}"
    );
}

/// Case 5: a CODED entry and a term-only entry for the SAME substance are NOT treated as
/// duplicates. This mirrors db/033's own documented gap (its comment: "the PAIR never a
/// bare code, and the coded<->uncoded case this deliberately does NOT close") —
/// deliberately asserting the CURRENT behaviour, not proposing it as correct. The
/// dup_key is either a `code:<system>|<code>` key or a `term:<normalised>` key; a coded
/// and an uncoded assert of the same drug produce two DIFFERENT keys and so never
/// collide, even though a human reading both lines would recognise them as the same
/// medication.
#[tokio::test]
async fn case5_coded_and_uncoded_same_substance_is_not_flagged() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    let (sk, kid, _hsk, _hkid) = setup(&c).await;
    let a = Uuid::now_v7();
    let b = Uuid::now_v7();
    submit_registration(&c, &sk, &kid, a, 0).await;
    submit_registration(&c, &sk, &kid, b, 0).await;

    // A drugref-moiety code is a UUIDv5 (the door's own floor check), not a free-text
    // string — same fixture value `medication_read.rs`'s coding-conflict test uses.
    const MOIETY_METFORMIN: &str = "3c7d9a52-4e18-5f60-8b21-6d4a0e9c7f33";
    assert_coded(
        &mut c,
        &sk,
        &kid,
        "origin-a",
        a,
        "metformin",
        MOIETY_METFORMIN,
    )
    .await;
    assert_one(&mut c, &sk, &kid, "origin-a", b, "metformin").await;

    let flagged = duplicate_groups(&c, &[a, b]).await;
    assert!(
        flagged.is_empty(),
        "coded vs uncoded is db/033's documented gap, not a new regression here: {flagged:?}"
    );
}
