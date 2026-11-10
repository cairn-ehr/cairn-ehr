//! Charting slice 1a, Task 3 — the medication pick's drug search (`cairn_drug_medication_search`,
//! in the drug read door, db/076 §5).
//!
//! The allergy pick (`cairn_drug_search`) offers drugs AND allergy groups. A medication line is
//! one drug, never a group, so the medication pick offers drugs only, and ranks them by how often
//! THIS node has charted each one (a coded line in `medication_coding`, or a coded component of
//! a combination in `medication_charting.components`), then by shorter name. Like every door it
//! answers nothing — never an error — when drugref is absent.
//!
//! drugref is the stand-in `tests/fixtures/drug_stub.sql`, loaded only inside a transaction this
//! file rolls back, so no other test on the shared database ever sees a `drugref` schema. The
//! "charted lines" are projection rows written inside the same transaction: the count is what is
//! pinned here, and the projection that writes them is pinned in medication_charting_projection.rs.
//!
//! DB-gated on `$CAIRN_TEST_PG` (a skipped run prints `skipped:`), serialized via
//! `db::test_serial_guard`.
use cairn_node::db;
use tokio_postgres::{Client, GenericClient};

mod common;

/// The drugref stand-in (see its header). Never load it outside a transaction that rolls back.
const STUB: &str = include_str!("fixtures/drug_stub.sql");
const DOOR: &str = "cairn_drug_medication_search(text, integer)";

async fn rig() -> Option<(Client, Client)> {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return None;
    };
    let guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    Some((guard, c))
}

/// The door's answer for `text`, as "display:uses" in the door's order.
async fn search(c: &impl GenericClient, text: &str) -> Vec<String> {
    c.query(
        "SELECT display || ':' || uses FROM cairn_drug_medication_search($1, 20)",
        &[&text],
    )
    .await
    .unwrap()
    .iter()
    .map(|r| r.get(0))
    .collect()
}

#[tokio::test]
async fn the_search_lists_drugs_only_ranked_by_use() {
    let Some((_g, mut c)) = rig().await else {
        return;
    };
    let tx = c.transaction().await.unwrap();
    tx.batch_execute(STUB).await.unwrap();
    // amoxapine is SHORTER than amoxicillin, so only the use count can put amoxicillin first.
    // The group's name matches "amox" too, so "no group" is a real exclusion, not a non-match.
    tx.batch_execute(
        "INSERT INTO drugref.substance_moiety (moiety_uuid, display_name) \
           VALUES (md5('stub moiety amoxapine')::uuid, 'amoxapine'); \
         INSERT INTO drugref.substance_class (class_uuid, class_name, concept_type) \
           VALUES (md5('stub class Amoxicillins')::uuid, 'Amoxicillins (allergy group)', 'AG'); \
         INSERT INTO drugref.allergy_group (class_uuid, review_status) \
           VALUES (md5('stub class Amoxicillins')::uuid, 'structural-seed'); \
         TRUNCATE medication_coding, medication_charting; \
         INSERT INTO medication_coding (medication_id, patient_id, coding_system, coding_code, \
                coding_display, hlc_wall, hlc_counter, origin, content_address) \
           VALUES (gen_random_uuid(), gen_random_uuid(), 'drugref-moiety', \
                   md5('stub moiety amoxicillin')::uuid::text, 'amoxicillin', 1, 0, 't', '\\x01'); \
         INSERT INTO medication_charting (medication_id, patient_id, components, \
                hlc_wall, hlc_counter, origin, content_address) \
           VALUES (gen_random_uuid(), gen_random_uuid(), jsonb_build_array( \
                   jsonb_build_object('term', 'amoxicillin', 'coding', jsonb_build_object( \
                       'system', 'drugref-moiety', \
                       'code', md5('stub moiety amoxicillin')::uuid::text, \
                       'display', 'amoxicillin')), \
                   jsonb_build_object('term', 'clavulanic acid', 'coding', NULL)), \
                   1, 0, 't', '\\x02');",
    )
    .await
    .unwrap();
    assert_eq!(
        search(&tx, "amox").await,
        ["amoxicillin:2", "amoxapine:0"],
        "drugs only, the two charted amoxicillin lines first, then shorter names"
    );
    let code: String = tx
        .query_one(
            "SELECT code FROM cairn_drug_medication_search('amoxicillin', 20)",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    let want: String = tx
        .query_one("SELECT md5('stub moiety amoxicillin')::uuid::text", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        code, want,
        "the code is the moiety uuid the line will carry"
    );
    assert!(
        search(&tx, "am").await.is_empty(),
        "fewer than 3 characters: nothing"
    );
    assert!(
        search(&tx, "a%x").await.is_empty(),
        "the text matches literally"
    );
    assert!(
        search(&tx, "moxi").await.is_empty(),
        "a word start, never mid-word"
    );
    tx.rollback().await.unwrap();
}

#[tokio::test]
async fn the_search_is_empty_without_drugref() {
    let Some((_g, c)) = rig().await else { return };
    let none: bool = c
        .query_one("SELECT to_regnamespace('drugref') IS NULL", &[])
        .await
        .unwrap()
        .get(0);
    assert!(none, "this test needs a database with no drugref schema");
    assert!(search(&c, "amoxicillin").await.is_empty());
    // Only the window asks; the door is a definer with the path pinned, as db/076's are.
    let (public, window, definer, path): (bool, bool, bool, Vec<String>) = c
        .query_one(
            "SELECT has_function_privilege('public', $1::text, 'EXECUTE'), \
                    has_function_privilege('cairn_window', $1::text, 'EXECUTE'), \
                    p.prosecdef, p.proconfig \
               FROM pg_proc p WHERE p.oid = $1::text::regprocedure",
            &[&DOOR],
        )
        .await
        .map(|r| (r.get(0), r.get(1), r.get(2), r.get(3)))
        .unwrap();
    assert!(!public, "PUBLIC must not execute {DOOR}");
    assert!(window, "the window must execute {DOOR}");
    assert!(definer, "{DOOR} is SECURITY DEFINER");
    assert_eq!(path, ["search_path=pg_catalog, pg_temp"]);
    // The door's first name is dropped on replay: no orphan definer outside the prefix guard.
    let orphan: bool = c
        .query_one(
            "SELECT to_regprocedure('cairn_medication_drug_search(text, integer)') IS NOT NULL",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert!(!orphan, "the old door name must not survive a replay");
}
