//! Charting slice 3, Task 1 — the drug picks search drugref's moiety NAMES (db/076, ADR-0106).
//!
//! A clinician types the name they use: *adrenaline*, *lignocaine*, *acetaminophen*. drugref
//! (since its db/070 + db/071, drugref#231) lists every name a moiety answers to in the view
//! `drugref.moiety_name`: its display name, its UNII official names, its MeSH synonyms. Both pick
//! doors (`cairn_drug_search` for allergies, `cairn_drug_medication_search` for medication lines)
//! now search that view when it is there, and say which name matched (`matched`: a UNII name
//! lower-cased, a MeSH name in its own case; NULL when the display name matched), so the window
//! can draw "Adrenaline → epinephrine".
//!
//! What is pinned here:
//!   * an alias finds its drug and names itself; one moiety is listed once, display first; a
//!     name two moieties claim lists both (the pick never chooses between them); a drug found
//!     by its display name ranks before one found only by another name;
//!   * a drugref WITHOUT the view (any release before drugref db/070), or with one the door's
//!     owner cannot read or that has the wrong columns, answers exactly as before — never an
//!     error, never nothing; a view that GAINED a column (additive drugref evolution) still
//!     serves the names;
//!   * the text stays literal; the internal helper and the names check are not the window's;
//!     a replay converges on the new shapes.
//!
//! drugref is `tests/fixtures/drug_stub.sql` (the older drugref) plus, for the newer one,
//! `tests/fixtures/drug_names_stub.sql`, both loaded only inside a transaction this file rolls
//! back. DB-gated on `$CAIRN_TEST_PG` (a skipped run prints `skipped:`), serialized via
//! `db::test_serial_guard`.
use cairn_node::db;
use tokio_postgres::{Client, GenericClient, Transaction};

mod common;

const STUB: &str = include_str!("fixtures/drug_stub.sql");
const NAMES: &str = include_str!("fixtures/drug_names_stub.sql");

/// A connection with the schema loaded, and the serial guard held for the test's life.
async fn rig() -> Option<(Client, Client)> {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return None;
    };
    let guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    Some((guard, c))
}

/// Load the older drugref, and the names too when `names`.
async fn load(tx: &Transaction<'_>, names: bool) {
    tx.batch_execute(STUB).await.unwrap();
    if names {
        tx.batch_execute(NAMES).await.unwrap();
    }
}

/// The ALLERGY pick's drug rows for `text`, as "display|matched" (`-` for NULL), in door order.
async fn allergy_pick(c: &impl GenericClient, text: &str) -> Vec<String> {
    c.query(
        "SELECT display || '|' || coalesce(matched, '-') FROM cairn_drug_search($1, 20) \
          WHERE system = 'drugref-moiety'",
        &[&text],
    )
    .await
    .unwrap()
    .iter()
    .map(|r| r.get(0))
    .collect()
}

/// The MEDICATION pick's rows for `text`, as "display|matched", in door order.
async fn med_pick(c: &impl GenericClient, text: &str) -> Vec<String> {
    c.query(
        "SELECT display || '|' || coalesce(matched, '-') \
           FROM cairn_drug_medication_search($1, 20)",
        &[&text],
    )
    .await
    .unwrap()
    .iter()
    .map(|r| r.get(0))
    .collect()
}

/// Replay every migration, as an upgrade's connect does: clear the stamp, then connect again.
async fn replay(c: &Client) -> Client {
    db::force_schema_replay(c).await.unwrap();
    db::connect_and_load_schema(&common::cs().unwrap())
        .await
        .unwrap()
}

async fn names_available(c: &impl GenericClient) -> bool {
    c.query_one("SELECT cairn_drug_names_available()", &[])
        .await
        .unwrap()
        .get(0)
}

#[tokio::test]
async fn alias_names_find_the_drug_and_say_which_name_matched() {
    let Some((_g, mut c)) = rig().await else {
        return;
    };
    let tx = c.transaction().await.unwrap();
    load(&tx, true).await;
    assert!(names_available(&tx).await, "control: the view is readable");
    // MeSH's own capitals are kept (only UNII's are lower-cased, final review M2); the racemate's
    // name is the one "adren" STARTS, never the shorter "Racemic Adrenaline" (final review I1).
    let adren = [
        "epinephrine|Adrenaline",
        "epinephrine,(+/-)-|Adrenaline, Racemic Mixture",
    ];
    assert_eq!(allergy_pick(&tx, "adren").await, adren);
    assert_eq!(
        allergy_pick(&tx, "acetam").await,
        ["paracetamol|acetaminophen"],
        "UNII's capitals are lower-cased"
    );
    assert_eq!(allergy_pick(&tx, "ligno").await, ["lidocaine|Lignocaine"]);
    assert_eq!(med_pick(&tx, "adren").await, adren);
    assert_eq!(
        med_pick(&tx, "ACETAMINOPHEN").await,
        ["paracetamol|acetaminophen"]
    );
    tx.rollback().await.unwrap();
}

/// Review Focus 2: amoxicillin answers "amox" by its display name AND its synonym Amoxycillin.
#[tokio::test]
async fn one_moiety_is_listed_once_display_first() {
    let Some((_g, mut c)) = rig().await else {
        return;
    };
    let tx = c.transaction().await.unwrap();
    load(&tx, true).await;
    assert_eq!(allergy_pick(&tx, "amox").await, ["amoxicillin|-"]);
    assert_eq!(med_pick(&tx, "amox").await, ["amoxicillin|-"]);
    assert_eq!(
        med_pick(&tx, "amoxy").await,
        ["amoxicillin|Amoxycillin"],
        "only the synonym matches: it is named"
    );
    tx.rollback().await.unwrap();
}

/// "cef" finds cefuroxime axetil by its display name and both cephalexins only by another name.
/// The display-name match comes first in both doors, though its name is the longer (so only that
/// rank, not length or byte order, can put it there): the window shows few rows, so an alias must
/// never push a drug the clinician spelled out below them.
#[tokio::test]
async fn a_display_name_match_ranks_before_another_names_match() {
    let Some((_g, mut c)) = rig().await else {
        return;
    };
    let tx = c.transaction().await.unwrap();
    load(&tx, true).await;
    let cef = [
        "cefuroxime axetil|-",
        "cephalexin|Cefalexin",
        "cephalexin anhydrous|cefalexin",
    ];
    assert_eq!(allergy_pick(&tx, "cef").await, cef);
    assert_eq!(med_pick(&tx, "cef").await, cef);
    tx.rollback().await.unwrap();
}

/// Review Focus 1: "cefalexin" is a MeSH synonym of cephalexin AND the UNII official name of
/// cephalexin anhydrous. Both are listed; the door never picks one.
#[tokio::test]
async fn a_shared_name_lists_every_moiety_it_names() {
    let Some((_g, mut c)) = rig().await else {
        return;
    };
    let tx = c.transaction().await.unwrap();
    load(&tx, true).await;
    let both = ["cephalexin|Cefalexin", "cephalexin anhydrous|cefalexin"];
    assert_eq!(allergy_pick(&tx, "cefalexin").await, both);
    assert_eq!(med_pick(&tx, "cefalexin").await, both);
    tx.rollback().await.unwrap();
}

/// Review Focus 3: the older drugref — no view — answers as it always did.
#[tokio::test]
async fn without_names_the_doors_answer_as_before() {
    let Some((_g, mut c)) = rig().await else {
        return;
    };
    let tx = c.transaction().await.unwrap();
    load(&tx, false).await;
    assert!(!names_available(&tx).await, "no view: no names");
    assert_eq!(allergy_pick(&tx, "amox").await, ["amoxicillin|-"]);
    assert_eq!(med_pick(&tx, "amox").await, ["amoxicillin|-"]);
    assert!(allergy_pick(&tx, "adren").await.is_empty());
    assert!(med_pick(&tx, "adren").await.is_empty());
    tx.rollback().await.unwrap();
}

/// Review Focus 3: a view the owner cannot read, or one reshaped, is ignored — never a fault.
/// The test database's owner is a superuser, for whom every privilege check is true, so (as
/// drug_read_door.rs does) the doors are handed to a throwaway non-superuser role inside this
/// rolled-back transaction.
#[tokio::test]
async fn an_unreadable_or_reshaped_name_view_is_ignored() {
    let Some((_g, mut c)) = rig().await else {
        return;
    };
    let tx = c.transaction().await.unwrap();
    load(&tx, true).await;
    tx.batch_execute(
        "DO $$
         DECLARE r text := 'cairn_drug_probe_' || pg_backend_pid(); f text;
         BEGIN
             EXECUTE format('CREATE ROLE %I NOLOGIN', r);
             FOREACH f IN ARRAY ARRAY['cairn_drug_available()', 'cairn_drug_names_available()',
                                      'cairn_drug_moiety_matches(text)',
                                      'cairn_drug_search(text, integer)',
                                      'cairn_drug_medication_search(text, integer)'] LOOP
                 EXECUTE format('ALTER FUNCTION %s OWNER TO %I', f, r);
             END LOOP;
             EXECUTE format('GRANT USAGE ON SCHEMA drugref TO %I', r);
             EXECUTE format('GRANT SELECT ON ALL TABLES IN SCHEMA drugref TO %I', r);
             EXECUTE format('GRANT SELECT ON public.medication_coding, public.medication_charting TO %I', r);
             REVOKE EXECUTE ON FUNCTION drugref.allergy_check(uuid, uuid) FROM PUBLIC;
             EXECUTE format('GRANT EXECUTE ON FUNCTION drugref.allergy_check(uuid, uuid) TO %I', r);
         END $$",
    )
    .await
    .unwrap();
    assert_eq!(
        med_pick(&tx, "ligno").await,
        ["lidocaine|Lignocaine"],
        "control: the probe owner reads the view"
    );
    tx.batch_execute(
        "DO $$ BEGIN EXECUTE format('REVOKE SELECT ON drugref.moiety_name FROM %I', \
         'cairn_drug_probe_' || pg_backend_pid()); END $$",
    )
    .await
    .unwrap();
    assert_display_only(&tx, "an unreadable view").await;

    // Reshaped: the same view without name_kind (readable, so only the shape check stops it).
    tx.batch_execute(
        "DROP VIEW drugref.moiety_name;
         CREATE VIEW drugref.moiety_name AS
             SELECT moiety_uuid, display_name AS name FROM drugref.substance_moiety
             UNION ALL SELECT moiety_uuid, name FROM drugref.moiety_mesh_synonym;
         DO $$ BEGIN EXECUTE format('GRANT SELECT ON drugref.moiety_name TO %I', \
             'cairn_drug_probe_' || pg_backend_pid()); END $$;",
    )
    .await
    .unwrap();
    assert_display_only(&tx, "a reshaped view").await;
    tx.rollback().await.unwrap();
}

/// drugref evolves additively (principle 11): a view that gains a column still has the three
/// the door reads, so the names stay ON. A check tightened to the exact column set would switch
/// every node's names off, silently, on the next drugref release.
#[tokio::test]
async fn a_view_with_an_added_column_still_serves_the_names() {
    let Some((_g, mut c)) = rig().await else {
        return;
    };
    let tx = c.transaction().await.unwrap();
    load(&tx, true).await;
    tx.batch_execute(
        "CREATE OR REPLACE VIEW drugref.moiety_name AS
             SELECT moiety_uuid, display_name AS name, 'display'::text AS name_kind,
                    'en'::text AS lang
               FROM drugref.substance_moiety
             UNION ALL SELECT moiety_uuid, name, 'mesh_synonym'::text, 'en'::text
               FROM drugref.moiety_mesh_synonym;",
    )
    .await
    .unwrap();
    assert!(
        names_available(&tx).await,
        "an added column: names still on"
    );
    assert_eq!(allergy_pick(&tx, "ligno").await, ["lidocaine|Lignocaine"]);
    assert_eq!(med_pick(&tx, "ligno").await, ["lidocaine|Lignocaine"]);
    tx.rollback().await.unwrap();
}

/// The names are not there, and both doors still search display names.
async fn assert_display_only(tx: &Transaction<'_>, why: &str) {
    assert!(!names_available(tx).await, "{why}: no names");
    assert!(allergy_pick(tx, "adren").await.is_empty(), "{why}");
    assert!(med_pick(tx, "adren").await.is_empty(), "{why}");
    assert_eq!(allergy_pick(tx, "amox").await, ["amoxicillin|-"], "{why}");
    assert_eq!(med_pick(tx, "amox").await, ["amoxicillin|-"], "{why}");
}

#[tokio::test]
async fn the_alias_text_is_still_literal() {
    let Some((_g, mut c)) = rig().await else {
        return;
    };
    let tx = c.transaction().await.unwrap();
    load(&tx, true).await;
    for literal in ["%%%", "ad_en", "adr%", "ad\\ren"] {
        assert!(allergy_pick(&tx, literal).await.is_empty(), "{literal:?}");
        assert!(med_pick(&tx, literal).await.is_empty(), "{literal:?}");
    }
    tx.rollback().await.unwrap();
}

/// The helper and the names check are the doors' own; the window gets only the doors.
#[tokio::test]
async fn the_helper_is_not_the_windows() {
    let Some((_g, c)) = rig().await else {
        return;
    };
    for f in [
        "cairn_drug_moiety_matches(text)",
        "cairn_drug_names_available()",
    ] {
        let can: bool = c
            .query_one(
                "SELECT has_function_privilege('cairn_window', $1, 'EXECUTE')",
                &[&f],
            )
            .await
            .unwrap()
            .get(0);
        assert!(!can, "{f}: not the window's");
    }
}

/// A replay leaves both doors in the new shape (the guarded DROP drops nothing the second time).
#[tokio::test]
async fn a_replay_converges() {
    let Some((_g, c)) = rig().await else {
        return;
    };
    // force_schema_replay only clears the stamp: the replay is the NEXT connect's. Twice, so the
    // second replay runs over an already converged node.
    for _ in 0..2 {
        replay(&c).await;
    }
    for (f, shape) in [
        (
            "cairn_drug_search(text, integer)",
            "TABLE(system text, code text, display text, matched text)",
        ),
        (
            "cairn_drug_medication_search(text, integer)",
            "TABLE(code text, display text, uses bigint, matched text)",
        ),
    ] {
        let got: String = c
            .query_one("SELECT pg_get_function_result(to_regprocedure($1))", &[&f])
            .await
            .unwrap()
            .get(0);
        assert_eq!(got, shape, "{f}");
    }
}

/// Task 2: the node's own readers carry the matched name (by column NAME, not position).
#[tokio::test]
async fn the_node_reads_the_matched_name() {
    use cairn_node::allergy::drug_facts::{search_drugs, search_medication_drugs};
    let Some((_g, mut c)) = rig().await else {
        return;
    };
    let tx = c.transaction().await.unwrap();
    load(&tx, true).await;
    let allergy = search_drugs(&tx, "ligno").await.unwrap().unwrap();
    let med = search_medication_drugs(&tx, "ligno", 20)
        .await
        .unwrap()
        .unwrap();
    for (which, found) in [("allergy", allergy), ("medication", med)] {
        assert_eq!(found.len(), 1, "{which}");
        assert_eq!(found[0].display, "lidocaine", "{which}");
        assert_eq!(found[0].matched.as_deref(), Some("Lignocaine"), "{which}");
    }
    let amox = search_medication_drugs(&tx, "amox", 20)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(amox[0].matched, None, "a display match names no other name");
    tx.rollback().await.unwrap();
}

/// Final review M5: a node still holding either door's OLD shape (before `matched`) is moved to
/// the new one by a replay (each guarded DROP really drops, and the door is granted to the window
/// again).
/// Committed, because db/076 runs its own transaction; the replay restores the converged shape.
#[tokio::test]
async fn a_replay_moves_an_old_shape_to_the_new_one() {
    let Some((_g, c)) = rig().await else {
        return;
    };
    c.batch_execute(
        "DROP FUNCTION cairn_drug_search(text, integer);
         CREATE FUNCTION cairn_drug_search(p_text text, p_limit int)
         RETURNS TABLE(system text, code text, display text)
         LANGUAGE sql STABLE AS $$ SELECT NULL::text, NULL::text, NULL::text WHERE false $$;
         DROP FUNCTION cairn_drug_medication_search(text, integer);
         CREATE FUNCTION cairn_drug_medication_search(p_text text, p_limit int)
         RETURNS TABLE(code text, display text, uses bigint)
         LANGUAGE sql STABLE AS $$ SELECT NULL::text, NULL::text, 0::bigint WHERE false $$;",
    )
    .await
    .unwrap();
    let c = replay(&c).await;
    for (f, shape) in [
        (
            "cairn_drug_search(text, integer)",
            "TABLE(system text, code text, display text, matched text)",
        ),
        (
            "cairn_drug_medication_search(text, integer)",
            "TABLE(code text, display text, uses bigint, matched text)",
        ),
    ] {
        let row = c
            .query_one(
                "SELECT pg_get_function_result($1::text::regprocedure), \
                        has_function_privilege('cairn_window', $1, 'EXECUTE')",
                &[&f],
            )
            .await
            .unwrap();
        assert_eq!(row.get::<_, String>(0), shape, "{f}");
        assert!(
            row.get::<_, bool>(1),
            "{f}: the re-created door is the window's again"
        );
    }
}
