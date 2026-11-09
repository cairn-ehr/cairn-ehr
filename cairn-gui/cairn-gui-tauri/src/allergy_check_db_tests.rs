//! DB-gated walks for `allergy_check.rs` (on `CAIRN_TEST_PG`, run locally — #905: CI never runs
//! these), on `supervised_note`'s fixture: the window's own login, a John Doe chart, and Dr Bea
//! in the vault.
//!
//! Two seams, both leaving nothing behind on the shared database:
//!   * **The drug list.** `drug_stub.sql` may only ever live inside a transaction that rolls back
//!     (its header). The window's commands run on their own connection, in their own snapshot,
//!     which cannot see another session's uncommitted schema. So the walks that need drugref call
//!     [`check_list`] — the very function `med_list_impl` hands its connection to — on the owner's
//!     rolled-back transaction, as the window's login (`SET LOCAL ROLE`). The walks with no drug
//!     list go through `med_list_impl` whole.
//!   * **A failed read** (Review Focus 4, D11). Grants cannot be subtracted from a role that
//!     inherits `cairn_window`, and the shared login must never be changed. So a throwaway role
//!     is given every grant the window holds but one, and the window's connection is swapped for
//!     an owner connection `SET ROLE` to it; it is dropped at the start and at the end.
use super::*;
use crate::allergies::{allergy_list_impl, allergy_record_impl, EntryInput};
use crate::commands::{med_list_impl, read_chart_of};
use crate::notes::review::db_tests::{credential, supervised_note};
use cairn_event::medication::SubstanceCoding;
use cairn_gui_tab_medications::MedListRowView;
use cairn_node::allergy::check::NOT_REVIEWED;
use cairn_node::allergy::read::list_patient_allergies;
use cairn_node::medication::{assert_medication, AssertMedicationInput};
use tokio_postgres::Client;

/// The drugref stand-in. Never load it outside a transaction that rolls back.
const STUB: &str = include_str!("../../../crates/cairn-node/tests/fixtures/drug_stub.sql");

/// The throwaway role of the failed-read seam.
const MINUS: &str = "cairn_test_window_minus";

/// A drug allergy, as the form sends it.
fn drug(term: &str) -> EntryInput {
    EntryInput {
        allergen_kind: Some("drug".into()),
        term: term.into(),
        kind: Some("allergy".into()),
        category: Some("anaphylaxis".into()),
        reaction_text: None,
        onset: None,
        source: Some("patient-stated".into()),
        corrects: None,
    }
}

/// A pick, as the form sends it.
fn picked(system: &str, code: impl ToString, display: &str) -> Option<CodingArg> {
    Some(CodingArg {
        system: system.into(),
        code: code.to_string(),
        display: display.into(),
    })
}

/// Sign `entry` coded `coding` on `chart` under Dr Bea's key.
async fn record(state: &AppState, chart: Uuid, entry: EntryInput, coding: Option<CodingArg>) {
    let patient = chart.to_string();
    let done = allergy_record_impl(
        state,
        &patient,
        vec![patient.clone()],
        entry,
        coding,
        credential("bea"),
    )
    .await
    .unwrap();
    assert_eq!(done.outcome, "Recorded.");
}

/// A current medication on `chart`, coded to the moiety `moiety` when given (shown as `term`).
async fn medication(owner: &mut Client, state: &AppState, chart: Uuid, term: &str, moiety: Uuid) {
    let sk = state.node_sk.as_ref().unwrap();
    let kid = hex::encode(sk.verifying_key().to_bytes());
    let code = moiety.to_string();
    let input = AssertMedicationInput {
        term,
        coding: Some(SubstanceCoding {
            system: MOIETY_SYSTEM,
            code: &code,
            display: term,
        }),
        formulation: None,
        dose_amount: None,
        dose_unit: None,
        sig: None,
        info_source: "patient",
        started: None,
        started_precision: None,
    };
    assert_medication(
        owner,
        sk,
        &kid,
        &state.node_origin,
        chart,
        &input,
        None,
        None,
    )
    .await
    .unwrap();
}

/// The stub's uuid for a label, computed as the stub computes it.
async fn stub_id(c: &Client, label: &str) -> Uuid {
    let t: String = c
        .query_one("SELECT md5($1)::uuid::text", &[&label])
        .await
        .unwrap()
        .get(0);
    t.parse().unwrap()
}

fn row<'a>(rows: &'a [MedListRowView], primary: &str) -> &'a MedListRowView {
    rows.iter()
        .find(|r| r.primary == primary)
        .unwrap_or_else(|| panic!("{primary} is on the list"))
}

/// Drop the failed-read seam's role and every grant it holds here, if it exists.
async fn drop_minus(owner: &Client) {
    owner
        .batch_execute(&format!(
            "DO $$ BEGIN IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = '{MINUS}') THEN \
               DROP OWNED BY {MINUS}; DROP ROLE {MINUS}; END IF; END $$"
        ))
        .await
        .unwrap();
}

/// The window's state, reconnected as a role holding every table, column, function and schema
/// grant the window's role holds (its own and inherited) EXCEPT on the object named `except`.
async fn window_without(owner: &Client, state: &mut AppState, except: &str) {
    drop_minus(owner).await;
    owner
        .batch_execute(&format!(
            "CREATE ROLE {MINUS} NOLOGIN;
             DO $$ DECLARE g record; BEGIN
               FOR g IN
                 WITH w AS (SELECT oid FROM pg_roles WHERE pg_has_role('cairn_window', oid, 'MEMBER'))
                 SELECT DISTINCT format('GRANT %s ON %s TO {MINUS}', a.privilege_type, c.oid::regclass) AS s
                   FROM pg_class c, aclexplode(c.relacl) a
                  WHERE a.grantee IN (SELECT oid FROM w) AND c.relkind IN ('r', 'v', 'm', 'p')
                    AND c.relname <> '{except}'
                 UNION
                 SELECT DISTINCT format('GRANT %s (%I) ON %s TO {MINUS}', a.privilege_type,
                                        t.attname, c.oid::regclass)
                   FROM pg_attribute t JOIN pg_class c ON c.oid = t.attrelid,
                        aclexplode(t.attacl) a
                  WHERE a.grantee IN (SELECT oid FROM w) AND c.relname <> '{except}'
                 UNION
                 SELECT DISTINCT format('GRANT EXECUTE ON FUNCTION %s TO {MINUS}', p.oid::regprocedure)
                   FROM pg_proc p, aclexplode(p.proacl) a
                  WHERE a.grantee IN (SELECT oid FROM w) AND a.privilege_type = 'EXECUTE'
                    AND p.proname <> '{except}'
                 UNION
                 SELECT DISTINCT format('GRANT USAGE ON SCHEMA %I TO {MINUS}', n.nspname)
                   FROM pg_namespace n, aclexplode(n.nspacl) a
                  WHERE a.grantee IN (SELECT oid FROM w) AND a.privilege_type = 'USAGE'
               LOOP EXECUTE g.s; END LOOP;
             END $$"
        ))
        .await
        .unwrap();
    let cs = std::env::var("CAIRN_TEST_PG").unwrap();
    let c = cairn_node::db::connect(&cs).await.unwrap();
    c.batch_execute(&format!("SET ROLE {MINUS}")).await.unwrap();
    state.db = Some(std::sync::Arc::new(cairn_gui_live::NodeConnection::fixed(
        c,
    )));
}

#[tokio::test]
async fn a_picked_group_is_signed_with_its_coding() {
    let Some((_guard, owner, state, chart, _)) = supervised_note().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let group = Uuid::from_u128(0x9E41_C111);
    // The webview may send any spelling of the uuid; it is signed canonically.
    let code = group.to_string().to_uppercase();
    record(
        &state,
        chart,
        drug("penicillins"),
        picked(GROUP_SYSTEM, code, "Penicillins (allergy group)"),
    )
    .await;
    let a = list_patient_allergies(&owner, chart).await.unwrap();
    assert_eq!(a.entries.len(), 1);
    let c = a.entries[0]
        .coding
        .as_ref()
        .expect("signed with its coding");
    assert_eq!(
        (c.system.as_str(), c.code.as_str(), c.display.as_str()),
        (
            GROUP_SYSTEM,
            group.to_string().as_str(),
            "Penicillins (allergy group)"
        )
    );
    assert_eq!(a.entries[0].term, "penicillins", "the term as typed");
}

#[tokio::test]
async fn a_bad_code_is_refused_before_signing() {
    let Some((_guard, owner, state, chart, _)) = supervised_note().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let patient = chart.to_string();
    let err = allergy_record_impl(
        &state,
        &patient,
        vec![patient.clone()],
        drug("amoxicillin"),
        picked(MOIETY_SYSTEM, "amoxicillin", "amoxicillin"),
        credential("bea"),
    )
    .await
    .unwrap_err();
    assert_eq!(err.text, BAD_CODE);
    let a = list_patient_allergies(&owner, chart).await.unwrap();
    assert!(a.entries.is_empty(), "nothing was signed");
}

/// Ruling T5-d: a pick sent on an `other` entry is refused before the key — no passphrase is
/// asked (none is held and none sent, yet the answer is the refusal) and nothing is signed.
#[tokio::test]
async fn a_pick_on_an_other_entry_is_refused_before_the_key() {
    let Some((_guard, owner, state, chart, _)) = supervised_note().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let patient = chart.to_string();
    let mut latex = drug("latex");
    latex.allergen_kind = Some("other".into());
    let err = allergy_record_impl(
        &state,
        &patient,
        vec![patient.clone()],
        latex,
        picked(MOIETY_SYSTEM, Uuid::from_u128(0x1A7E), "latex"),
        None,
    )
    .await
    .unwrap_err();
    assert_eq!(err.text, NOT_A_DRUG);
    let a = list_patient_allergies(&owner, chart).await.unwrap();
    assert!(a.entries.is_empty(), "nothing was signed");
}

/// No drug list here: an identical code is still a hit (D7), and a line coded to another drug
/// cannot be verified, so it carries the small tag (D8).
#[tokio::test]
async fn med_list_marks_a_line_against_a_coded_allergy() {
    let Some((_guard, mut owner, state, chart, _)) = supervised_note().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let (amox, metformin) = (Uuid::from_u128(0xA40C_0001), Uuid::from_u128(0x3E7F_0002));
    let coded = picked(MOIETY_SYSTEM, amox, "amoxicillin");
    record(&state, chart, drug("amoxicillin"), coded).await;
    // Shown by a brand name, so only the code can meet the allergy.
    medication(&mut owner, &state, chart, "Amoxil", amox).await;
    medication(&mut owner, &state, chart, "Diabex", metformin).await;

    let pane = med_list_impl(&state, &chart.to_string(), None)
        .await
        .unwrap();
    assert_eq!(pane.allergy_check_failed, None);
    let amoxil = row(&pane.list.rows, "Amoxil");
    assert_eq!(
        amoxil.allergy_marks,
        ["Allergy recorded: amoxicillin → anaphylaxis"]
    );
    assert!(!amoxil.allergy_unverified);
    let diabex = row(&pane.list.rows, "Diabex");
    assert!(diabex.allergy_marks.is_empty());
    assert!(diabex.allergy_unverified, "no drug list: not checked");
}

/// With the drug list: an `unplaced` answer leaves a line "not checked" (D8, ruling R1); a
/// `match` through the group marks one; the window's login can make the whole read.
#[tokio::test]
async fn med_list_flags_an_unplaced_line_not_checked() {
    let Some((_guard, mut owner, state, chart, _)) = supervised_note().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let fluclox = stub_id(&owner, "stub moiety flucloxacillin").await;
    let metformin = stub_id(&owner, "stub moiety metformin").await;
    let amox = stub_id(&owner, "stub moiety amoxicillin").await;
    let perindopril = stub_id(&owner, "stub moiety perindopril").await;
    let coded = |id| picked(MOIETY_SYSTEM, id, "drug");
    record(&state, chart, drug("flucloxacillin"), coded(fluclox)).await;
    record(&state, chart, drug("metformin"), coded(metformin)).await;
    medication(&mut owner, &state, chart, "Amoxil", amox).await;
    medication(&mut owner, &state, chart, "Coversyl", perindopril).await;
    // What med_list_impl reads, through the window's own reads.
    let (_, allergies, _) = crate::allergies::read_list(&state, chart).await.unwrap();
    let list = read_chart_of(&state, chart).await.unwrap();

    let tx = owner.transaction().await.unwrap();
    tx.batch_execute(STUB).await.unwrap();
    tx.batch_execute("SET LOCAL ROLE cairn_window_probe")
        .await
        .unwrap();
    let checks = check_list(Some(&tx), &allergies, &list).await;
    tx.rollback().await.unwrap();

    let line = |term: &str| {
        let r = list.rows.iter().find(|r| r.term == term).unwrap();
        checks.get(&r.group_id).cloned().unwrap_or_default()
    };
    assert_eq!(
        line("Amoxil").marks,
        [
            "Amoxil is in the same group as flucloxacillin (Penicillins (allergy group)); allergy recorded: \
          anaphylaxis"
        ]
    );
    assert!(!line("Amoxil").unverified);
    // flucloxacillin: no-match (verified); metformin: unplaced (not verified).
    assert_eq!(
        line("Coversyl"),
        LineCheck {
            unverified: true,
            ..LineCheck::default()
        }
    );
}

/// Ruling T5-a (T2R-b's mapping): a drug list the door's owner can only partly read makes the
/// facts door RAISE. That is "no drug list", never a failed pane: every line reads "not checked".
#[tokio::test]
async fn a_drug_list_that_raises_leaves_every_line_not_checked() {
    let Some((_guard, mut owner, state, chart, _)) = supervised_note().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let fluclox = stub_id(&owner, "stub moiety flucloxacillin").await;
    let amox = stub_id(&owner, "stub moiety amoxicillin").await;
    record(
        &state,
        chart,
        drug("flucloxacillin"),
        picked(MOIETY_SYSTEM, fluclox, "flucloxacillin"),
    )
    .await;
    medication(&mut owner, &state, chart, "Amoxil", amox).await;
    let (_, allergies, _) = crate::allergies::read_list(&state, chart).await.unwrap();
    let list = read_chart_of(&state, chart).await.unwrap();

    let tx = owner.transaction().await.unwrap();
    tx.batch_execute(STUB).await.unwrap();
    // As ruling P1: the doors handed to a throwaway owner (inside this rolled-back transaction)
    // that passes every availability check but cannot read the table `allergy_check` answers
    // from, so the door is "available" and then raises.
    tx.batch_execute(
        "DO $$
         DECLARE r text := 'cairn_drug_probe_' || pg_backend_pid(); f text;
         BEGIN
             EXECUTE format('CREATE ROLE %I NOLOGIN', r);
             FOREACH f IN ARRAY ARRAY['cairn_drug_available()', 'cairn_drug_facts(uuid[], uuid[])']
             LOOP
                 EXECUTE format('ALTER FUNCTION %s OWNER TO %I', f, r);
             END LOOP;
             EXECUTE format('GRANT USAGE ON SCHEMA drugref TO %I', r);
             EXECUTE format('GRANT SELECT ON drugref.substance_moiety, drugref.substance_class, \
                             drugref.allergy_group TO %I', r);
             EXECUTE format('GRANT EXECUTE ON FUNCTION drugref.allergy_check(uuid, uuid) TO %I', r);
         END $$;
         SET LOCAL ROLE cairn_window_probe",
    )
    .await
    .unwrap();
    let available: bool = tx
        .query_one("SELECT cairn_drug_available()", &[])
        .await
        .unwrap()
        .get(0);
    assert!(available, "control: the drug list reads as there");
    let checks = check_list(Some(&tx), &allergies, &list).await;
    assert!(
        tx.query_one("SELECT 1", &[]).await.is_err(),
        "the facts door raised (the transaction is aborted)"
    );
    tx.rollback().await.unwrap();
    assert_eq!(
        checks.get(&list.rows[0].group_id),
        Some(&LineCheck {
            unverified: true,
            ..LineCheck::default()
        }),
        "the group match is unknowable: not checked, never clean"
    );
}

/// Review Focus 4: the allergy read fails while the medication list is read. The list still
/// draws, with the worded line above it.
#[tokio::test]
async fn med_list_draws_when_the_allergy_read_fails() {
    let Some((_guard, mut owner, mut state, chart, _)) = supervised_note().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let amox = Uuid::from_u128(0xA40C_0003);
    let coded = picked(MOIETY_SYSTEM, amox, "amoxicillin");
    record(&state, chart, drug("amoxicillin"), coded).await;
    medication(&mut owner, &state, chart, "Amoxil", amox).await;
    let patient = chart.to_string();
    let control = med_list_impl(&state, &patient, None).await.unwrap();
    assert_eq!(control.list.rows[0].allergy_marks.len(), 1, "control");

    window_without(&owner, &mut state, "clinical_allergy").await;
    let pane = med_list_impl(&state, &patient, None).await;
    let why = crate::allergies::read_list(&state, chart).await.err();
    drop_minus(&owner).await;
    let why = why.expect("the seam fails the allergy read").text;
    assert!(
        why.contains("clinical_allergy"),
        "only that grant is missing: {why}"
    );
    let pane = pane.expect("the medication list still opens");
    assert_eq!(
        pane.allergy_check_failed.as_deref(),
        Some(ALLERGIES_NOT_READ)
    );
    let amoxil = row(&pane.list.rows, "Amoxil");
    assert!(amoxil.allergy_marks.is_empty() && !amoxil.allergy_unverified);
}

/// D11 and M7: a group-coded entry whose group is not reviewed says so on the expanded list —
/// with no drug list (the door answers nothing) and when the status read FAILS. A drug-coded
/// entry never does.
#[tokio::test]
async fn allergy_list_marks_an_unreviewed_group() {
    let Some((_guard, owner, mut state, chart, _)) = supervised_note().await else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let (group, moiety) = (Uuid::from_u128(0x9E41_C112), Uuid::from_u128(0x3E7F_0004));
    let pen = picked(GROUP_SYSTEM, group, "Penicillins (allergy group)");
    record(&state, chart, drug("penicillins"), pen).await;
    let met = picked(MOIETY_SYSTEM, moiety, "metformin");
    record(&state, chart, drug("metformin"), met).await;
    let patient = chart.to_string();
    let says = |pane: &crate::allergy_pane::AllergyPane, term: &str| {
        let line = pane
            .drug_allergies
            .iter()
            .find(|l| l.words.as_ref().unwrap().term == term)
            .unwrap();
        line.text.ends_with(NOT_REVIEWED)
    };
    let pane = allergy_list_impl(&state, &patient).await.unwrap();
    assert!(says(&pane, "penicillins"), "no drug list: not reviewed");
    assert!(!says(&pane, "metformin"));
    assert!(!pane.strip.contains(NOT_REVIEWED), "the strip stays terse");

    window_without(&owner, &mut state, "cairn_drug_group_status").await;
    let pane = allergy_list_impl(&state, &patient).await;
    drop_minus(&owner).await;
    let pane = pane.expect("a failed status read never fails the list");
    assert!(says(&pane, "penicillins"), "a failed read: not reviewed");
    assert!(!says(&pane, "metformin"));
}
