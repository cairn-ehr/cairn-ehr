//! ADR-0076 R1: a linked chart reads as ONE list over its chart set, each row naming its
//! source chart; a group is found through its members (the #334 fix); a group reaching a
//! chart OUTSIDE the set is still a hazard; the same drug on two linked charts is flagged.
//!
//! DB-gated on $CAIRN_TEST_PG, serialized cluster-wide via `db::test_serial_guard`, taken
//! BEFORE connecting (see `medication_read.rs`'s header for why that order is load-bearing
//! for this read in particular). Key material is minted at runtime (house rule 6).
mod common;
use cairn_event::SigningKey;
use cairn_medication_view::{sign_off_targets, withheld_rows, ChartSet};
use cairn_node::db;
use cairn_node::medication::read::list_patient_medications;
use cairn_node::medication::{
    assert_medication, attest_medication_thread, cease_medication, reconcile_medications,
    AssertMedicationInput, AttestParams, CeaseMedicationInput, ReconcileInput, SubstanceCoding,
};
use common::{cs, medication_setup as setup, submit_link_event, submit_registration};
use serde_json::Value;
use std::collections::HashMap;
use tokio_postgres::Client;
use uuid::Uuid;

/// One registered chart (#345: the birth act precedes everything recorded about it).
async fn chart(c: &Client, sk: &SigningKey, kid: &str) -> Uuid {
    let p = Uuid::now_v7();
    submit_registration(c, sk, kid, p, 0).await;
    p
}

/// Assert one active medication on `patient`, optionally coded (ADR-0059); returns its
/// thread id. The one place this suite builds an `AssertMedicationInput`, so every drug in
/// it carries the same 500 mg dose and differs only in what the test is about.
async fn assert_drug(
    c: &mut Client,
    sk: &SigningKey,
    kid: &str,
    patient: Uuid,
    term: &str,
    coding: Option<SubstanceCoding<'_>>,
) -> Uuid {
    assert_medication(
        c,
        sk,
        kid,
        "origin-a",
        patient,
        &AssertMedicationInput {
            term,
            coding,
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

/// Assert one uncoded active medication on `patient`; returns its thread id. Same shape as
/// `medication_read.rs`'s helper of the same name (kept file-local: two suites, one line of
/// difference each, is below the bar for `common/`).
async fn assert_one(c: &mut Client, sk: &SigningKey, kid: &str, patient: Uuid, term: &str) -> Uuid {
    assert_drug(c, sk, kid, patient, term, None).await
}

/// Fold two threads into one group with `first` as the group id — the peer-arrival shape
/// `medication_read.rs`'s #334 tests use (the local door refuses a cross-chart reconcile, and
/// never refuses on the sync-apply path, so this state legitimately arrives from a peer).
async fn group(c: &Client, first: Uuid, second: Uuid) {
    c.execute(
        "INSERT INTO medication_group_member (medication_id, group_id) VALUES \
         ($1::text::uuid, $1::text::uuid), ($2::text::uuid, $1::text::uuid)",
        &[&first.to_string(), &second.to_string()],
    )
    .await
    .unwrap();
}

/// Two ids in ascending order. The golden names a pair's threads by their ORDER rather than
/// by which was minted first, so the literal cannot depend on two `now_v7()` calls in the
/// same millisecond happening to sort the way they were minted.
fn lo_hi(x: Uuid, y: Uuid) -> (Uuid, Uuid) {
    (x.min(y), x.max(y))
}

/// Replace, anywhere in `v`, every string that `roles` names by its role — so a list read
/// over ids minted fresh on every run can be compared against ONE literal. Object keys are
/// renamed too (`separation_targets` is keyed by group id).
fn name_roles(v: &mut Value, roles: &HashMap<String, String>) {
    match v {
        Value::String(s) => {
            if let Some(role) = roles.get(s.as_str()) {
                *s = role.clone();
            }
        }
        Value::Array(items) => items.iter_mut().for_each(|i| name_roles(i, roles)),
        Value::Object(map) => {
            let entries: Vec<(String, Value)> = std::mem::take(map).into_iter().collect();
            for (key, mut value) in entries {
                name_roles(&mut value, roles);
                map.insert(roles.get(&key).cloned().unwrap_or(key), value);
            }
        }
        _ => {}
    }
}

/// Strip the fields Task 3 of R1 ADDED (`charts`, each row's `source_charts`, each member's
/// `patient_id`). The golden pins everything that existed BEFORE the combined read, so the
/// new fields — whose values the combined read is entitled to derive differently — are not
/// part of it; they are pinned by the tests that are about them.
fn strip_new_fields(list: &mut Value) {
    list.as_object_mut().unwrap().remove("charts");
    for row in list["rows"].as_array_mut().unwrap() {
        row.as_object_mut().unwrap().remove("source_charts");
        for member in row["members"].as_array_mut().unwrap() {
            member.as_object_mut().unwrap().remove("patient_id");
        }
    }
}

/// THE GOLDEN (ADR-0076 R1, plan constraint "a single never-linked chart reads exactly as
/// before"). Captured against the read as it stood BEFORE the combined read replaced it
/// (the post-Task-3 tree of the R1 plan), and required to pass unchanged after.
///
/// One never-linked chart holding one of each thing the read distinguishes: an attested
/// active drug (a Fresh vouch), a ceased drug (the past view), a reconciled same-chart pair
/// (one row, two members — through the real reconcile orchestrator, which the local door
/// permits within one chart), a coded drug (sorted under its coded display name), and an
/// un-reconciled duplicate pair (both flagged). Every advisory flag appears at least once
/// false, and `reconciliation_flagged` once true — the query behind it is one this slice
/// replaces. Ids are minted per run, so each is replaced by its role name before the compare.
#[tokio::test]
async fn a_never_linked_chart_reads_exactly_as_before() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    c.batch_execute("TRUNCATE patient_link, person_member")
        .await
        .unwrap();
    let (sk, kid, hsk, hkid) = setup(&c).await;
    let p = chart(&c, &sk, &kid).await;

    let active = assert_one(&mut c, &sk, &kid, p, "amlodipine").await;
    let params = AttestParams {
        human_sk: &hsk,
        human_kid: &hkid,
        basis: None,
        note: None,
    };
    attest_medication_thread(&mut c, &sk, "origin-a", &params, p, active)
        .await
        .unwrap();

    let ceased = assert_one(&mut c, &sk, &kid, p, "penicillin").await;
    cease_medication(
        &mut c,
        &sk,
        &kid,
        "origin-a",
        p,
        ceased,
        &CeaseMedicationInput {
            stopped: None,
            stopped_precision: None,
            reason: Some("rash"),
        },
        None,
        None,
    )
    .await
    .unwrap();

    let x = assert_one(&mut c, &sk, &kid, p, "metformin").await;
    let y = assert_one(&mut c, &sk, &kid, p, "Metformin XR").await;
    reconcile_medications(
        &mut c,
        &sk,
        &kid,
        "origin-a",
        p,
        x,
        y,
        &ReconcileInput {
            provenance: "clinician-judgment",
            reason: None,
        },
        None,
        None,
    )
    .await
    .unwrap();
    let (pair_lo, pair_hi) = lo_hi(x, y);
    // The group takes its display term from the member whose id IS the group id (db/033's
    // `medication_group_display` tiebreak), so the golden's "metformin" depends on `x`
    // sorting first. `now_v7` is monotonic within a process; say so rather than rely on it.
    assert_eq!(pair_lo, x, "the first-minted thread sorts first");

    const MOIETY_ATORVASTATIN: &str = "0f8c4b1e-1b7a-5c2d-9a3e-2b6f7c8d9e01";
    let coded = assert_drug(
        &mut c,
        &sk,
        &kid,
        p,
        "atorvastatin 20",
        Some(SubstanceCoding {
            system: "drugref-moiety",
            code: MOIETY_ATORVASTATIN,
            display: "Lipitor",
        }),
    )
    .await;

    let d1 = assert_one(&mut c, &sk, &kid, p, "aspirin").await;
    let d2 = assert_one(&mut c, &sk, &kid, p, "aspirin").await;
    let (dup_lo, dup_hi) = lo_hi(d1, d2);

    let roles: HashMap<String, String> = [
        (p.to_string(), "<chart>"),
        (active.to_string(), "<active>"),
        (ceased.to_string(), "<ceased>"),
        (pair_lo.to_string(), "<pair-lo>"),
        (pair_hi.to_string(), "<pair-hi>"),
        (coded.to_string(), "<coded>"),
        (dup_lo.to_string(), "<dup-lo>"),
        (dup_hi.to_string(), "<dup-hi>"),
        (hkid.clone(), "<human-kid>"),
    ]
    .into_iter()
    .map(|(k, v)| (k, v.to_string()))
    .collect();

    let mut got = serde_json::to_value(list_patient_medications(&c, p).await.unwrap()).unwrap();
    strip_new_fields(&mut got);
    name_roles(&mut got, &roles);

    // Captured 2026-09-27 from the single-chart read (post-Task-3 tree, before the combined
    // read replaced it) — see the doc comment. Laid out one field-family per line for review;
    // compared as parsed JSON, so key order inside an object is immaterial while ROW order
    // (the display sort) is pinned.
    const GOLDEN: &str = r#"{
  "groups_missing_from_chart": [],
  "separation_targets": {},
  "rows": [
    {
      "group_id": "<coded>", "patient_id": "<chart>", "status": "Active", "term": "atorvastatin 20", "coding_display": "Lipitor",
      "dose_amount": "500", "dose_unit": "mg", "formulation": null, "sig": null, "started_value": null, "started_precision": null,
      "reconciliation_flagged": false, "coding_conflict": false, "cross_patient": false,
      "members": [{"medication_id": "<coded>", "vouch": "Absent"}]
    },
    {
      "group_id": "<active>", "patient_id": "<chart>", "status": "Active", "term": "amlodipine", "coding_display": null,
      "dose_amount": "500", "dose_unit": "mg", "formulation": null, "sig": null, "started_value": null, "started_precision": null,
      "reconciliation_flagged": false, "coding_conflict": false, "cross_patient": false,
      "members": [{"medication_id": "<active>", "vouch": {"Fresh": {"by": "<human-kid>"}}}]
    },
    {
      "group_id": "<dup-lo>", "patient_id": "<chart>", "status": "Active", "term": "aspirin", "coding_display": null,
      "dose_amount": "500", "dose_unit": "mg", "formulation": null, "sig": null, "started_value": null, "started_precision": null,
      "reconciliation_flagged": true, "coding_conflict": false, "cross_patient": false,
      "members": [{"medication_id": "<dup-lo>", "vouch": "Absent"}]
    },
    {
      "group_id": "<dup-hi>", "patient_id": "<chart>", "status": "Active", "term": "aspirin", "coding_display": null,
      "dose_amount": "500", "dose_unit": "mg", "formulation": null, "sig": null, "started_value": null, "started_precision": null,
      "reconciliation_flagged": true, "coding_conflict": false, "cross_patient": false,
      "members": [{"medication_id": "<dup-hi>", "vouch": "Absent"}]
    },
    {
      "group_id": "<pair-lo>", "patient_id": "<chart>", "status": "Active", "term": "metformin", "coding_display": null,
      "dose_amount": "500", "dose_unit": "mg", "formulation": null, "sig": null, "started_value": null, "started_precision": null,
      "reconciliation_flagged": false, "coding_conflict": false, "cross_patient": false,
      "members": [{"medication_id": "<pair-lo>", "vouch": "Absent"}, {"medication_id": "<pair-hi>", "vouch": "Absent"}]
    },
    {
      "group_id": "<ceased>", "patient_id": "<chart>", "status": "Ceased", "term": "penicillin", "coding_display": null,
      "dose_amount": "500", "dose_unit": "mg", "formulation": null, "sig": null, "started_value": null, "started_precision": null,
      "reconciliation_flagged": false, "coding_conflict": false, "cross_patient": false,
      "members": [{"medication_id": "<ceased>", "vouch": "Absent"}]
    }
  ]
}"#;
    let want: Value = serde_json::from_str(GOLDEN).unwrap();
    assert_eq!(
        got,
        want,
        "a never-linked chart must read exactly as it did before the combined read:\n{}",
        serde_json::to_string_pretty(&got).unwrap()
    );
}

/// The headline of R1: two linked charts read as ONE list from either side, and each row
/// says which chart it was recorded on — a clinician reading a combined list must never have
/// to infer a drug's source from which chart happened to be opened.
#[tokio::test]
async fn a_linked_pair_reads_as_one_list_with_source_labels() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    c.batch_execute("TRUNCATE patient_link, person_member")
        .await
        .unwrap();
    let (sk, kid, _hsk, _hkid) = setup(&c).await;
    let a = chart(&c, &sk, &kid).await;
    let b = chart(&c, &sk, &kid).await;
    let met = assert_one(&mut c, &sk, &kid, a, "metformin").await;
    let aml = assert_one(&mut c, &sk, &kid, b, "amlodipine").await;
    submit_link_event(&c, &sk, &kid, a, b, 10, true).await;

    for opened in [a, b] {
        let list = list_patient_medications(&c, opened).await.unwrap();
        assert_eq!(
            list.charts,
            ChartSet::new([a, b]).unwrap(),
            "opened {opened}"
        );
        assert_eq!(list.rows.len(), 2, "both charts' drugs, from either side");
        let row = |g: Uuid| list.rows.iter().find(|r| r.group_id == g).unwrap();
        assert_eq!(row(met).source_charts, vec![a]);
        assert_eq!(row(aml).source_charts, vec![b]);
        assert_eq!(
            row(met).members[0].patient_id,
            a,
            "each thread names its own chart"
        );
        assert_eq!(row(aml).members[0].patient_id, b);
        assert!(
            list.rows.iter().all(|r| !r.cross_patient),
            "nothing reaches outside the set"
        );
    }
}

/// A reconciled group whose threads sit on two charts of the SAME person: one line (never the
/// view's two `(group, patient)` rows), naming both source charts, each member naming its own
/// chart — and signable, because both charts are this person, so it is no wrong-chart hazard.
#[tokio::test]
async fn a_group_inside_the_set_shows_once_and_is_signable() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    c.batch_execute("TRUNCATE patient_link, person_member")
        .await
        .unwrap();
    let (sk, kid, _hsk, _hkid) = setup(&c).await;
    let a = chart(&c, &sk, &kid).await;
    let b = chart(&c, &sk, &kid).await;
    let ta = assert_one(&mut c, &sk, &kid, a, "metformin").await;
    let tb = assert_one(&mut c, &sk, &kid, b, "metformin").await;
    group(&c, ta, tb).await;
    submit_link_event(&c, &sk, &kid, a, b, 10, true).await;

    let list = list_patient_medications(&c, b).await.unwrap();
    assert_eq!(
        list.rows.len(),
        1,
        "one drug, one line — never the view's two (group, patient) rows"
    );
    let row = &list.rows[0];
    assert_eq!(row.source_charts, {
        let mut v = vec![a, b];
        v.sort();
        v
    });
    assert!(
        !row.cross_patient,
        "both charts are this person: not a wrong-chart hazard"
    );
    assert!(
        !row.reconciliation_flagged,
        "one group is not a duplicate of itself"
    );
    let owner = |t: Uuid| {
        row.members
            .iter()
            .find(|m| m.medication_id == t)
            .unwrap()
            .patient_id
    };
    assert_eq!(owner(ta), a);
    assert_eq!(owner(tb), b);
    let mut both = vec![ta, tb];
    both.sort();
    assert_eq!(
        sign_off_targets(&list.rows),
        both,
        "a group inside the set is signable"
    );
    assert!(list.groups_missing_from_chart.is_empty());
    assert!(
        list.separation_targets.is_empty(),
        "nothing hazardous, nothing to separate"
    );
}

/// Review focus 3: A–B linked, and a reconciled group spanning B and an UNLINKED chart C.
/// Membership in the set is not enough to make a group safe — it must lie WHOLLY inside it.
/// And #334's other half: C's own chart shows the group too, where it used to vanish.
#[tokio::test]
async fn a_group_reaching_outside_the_set_is_still_a_hazard() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    c.batch_execute("TRUNCATE patient_link, person_member")
        .await
        .unwrap();
    let (sk, kid, _hsk, _hkid) = setup(&c).await;
    let a = chart(&c, &sk, &kid).await;
    let b = chart(&c, &sk, &kid).await;
    let outsider = chart(&c, &sk, &kid).await;
    let tb = assert_one(&mut c, &sk, &kid, b, "warfarin").await;
    let to = assert_one(&mut c, &sk, &kid, outsider, "warfarin").await;
    group(&c, tb, to).await;
    submit_link_event(&c, &sk, &kid, a, b, 10, true).await; // the outsider is NOT linked

    let list = list_patient_medications(&c, a).await.unwrap();
    assert_eq!(list.rows.len(), 1);
    assert!(
        list.rows[0].cross_patient,
        "the group reaches a chart that is not this person"
    );
    assert_eq!(
        withheld_rows(&list.rows),
        vec![tb],
        "and is withheld from sign-off"
    );
    assert!(
        list.separation_targets.contains_key(&tb),
        "with the arguments to separate it"
    );
    assert_eq!(
        list.rows[0]
            .members
            .iter()
            .map(|m| m.medication_id)
            .collect::<Vec<_>>(),
        vec![tb],
        "the outsider's thread is a separation argument, never a member of this person's line"
    );

    // #334's other half: the outsider's own chart SHOWS the group too (it used to vanish).
    let theirs = list_patient_medications(&c, outsider).await.unwrap();
    assert_eq!(theirs.rows.len(), 1);
    assert!(theirs.rows[0].cross_patient);
    assert!(theirs.groups_missing_from_chart.is_empty());
}

/// A link this node's own hard veto flagged (db/018 `link_veto_flag`: an un-attested link that
/// trips `cairn_has_hard_veto`, admitted on the sync path) still combines the read — ADR-0076
/// decision 1 follows every standing link, and the member lines show `under-review`. But it
/// must NOT make a group spanning the pair signable. Before the combined read such a group was
/// cross-patient and withheld; treating it as "inside the set" would let a clinician vouch A's
/// thread under a line whose displayed dose may be X's — the node itself believes X may be
/// someone else. So while the set holds a vetoed pair, a group spanning more than one chart
/// stays a hazard; a line on ONE chart is untouched (its dose is its own chart's).
///
/// The flag is set directly rather than by engineering a veto-tripping pair: its lifecycle is
/// db/018's and is pinned by `link_veto_floor.rs`; this test is about the read's response.
#[tokio::test]
async fn a_group_across_a_vetoed_link_is_still_withheld() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    c.batch_execute("TRUNCATE patient_link, person_member, link_veto_flag")
        .await
        .unwrap();
    let (sk, kid, _hsk, _hkid) = setup(&c).await;
    let a = chart(&c, &sk, &kid).await;
    let x = chart(&c, &sk, &kid).await;
    let ta = assert_one(&mut c, &sk, &kid, a, "warfarin").await;
    let tx = assert_one(&mut c, &sk, &kid, x, "warfarin").await;
    let only_x = assert_one(&mut c, &sk, &kid, x, "amlodipine").await;
    group(&c, ta, tx).await;
    submit_link_event(&c, &sk, &kid, a, x, 10, true).await;

    // Positive control: with the link standing and NOT flagged, the pair is one person and
    // the shared group is an ordinary reconciled drug.
    let before = list_patient_medications(&c, a).await.unwrap();
    assert!(before.rows.iter().all(|r| !r.cross_patient));

    let (lo, hi) = lo_hi(a, x);
    c.execute(
        "INSERT INTO link_veto_flag (low, high, content_address) \
         SELECT low, high, content_address FROM patient_link \
         WHERE low = $1::text::uuid AND high = $2::text::uuid",
        &[&lo.to_string(), &hi.to_string()],
    )
    .await
    .unwrap();

    let list = list_patient_medications(&c, a).await.unwrap();
    assert!(
        list.charts.is_linked(),
        "the vetoed link still combines the read"
    );
    let shared = list.rows.iter().find(|r| r.group_id == ta).unwrap();
    assert!(
        shared.cross_patient,
        "a group spanning a vetoed pair is a wrong-chart hazard"
    );
    assert_eq!(
        withheld_rows(&list.rows),
        vec![ta],
        "the line (reported by its group id) is withheld from sign-off"
    );
    assert!(
        list.separation_targets.contains_key(&ta),
        "with the arguments to separate it"
    );
    let single = list.rows.iter().find(|r| r.group_id == only_x).unwrap();
    assert!(
        !single.cross_patient,
        "a line on ONE chart shows that chart's own dose: not a wrong-chart hazard"
    );
    assert_eq!(
        sign_off_targets(&list.rows),
        vec![only_x],
        "neither thread of the shared group is signed; the one-chart line still is"
    );
}

/// The orphan-cessation half of the hazard rule, through the combined read. A thread known
/// only through a stop event that arrived before its statement (db/033 PR #219 finding 3)
/// is invisible to `medication_thread_group`, so only `medication_group_cross_patient` can
/// say the group reaches another chart. Deleting that half of the rule would look like
/// redundancy cleanup and leave every other test green — this one pins it, and then pins
/// that the SET rule applies on this path too: once the two charts are linked, it is not a
/// hazard.
#[tokio::test]
async fn a_group_reaching_outside_only_through_an_orphan_cessation_is_a_hazard() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    c.batch_execute("TRUNCATE patient_link, person_member, link_veto_flag")
        .await
        .unwrap();
    let (sk, kid, _hsk, _hkid) = setup(&c).await;
    let a = chart(&c, &sk, &kid).await;
    let other = chart(&c, &sk, &kid).await;
    let ta = assert_one(&mut c, &sk, &kid, a, "digoxin").await;
    // A thread id no statement ever named, stopped on `other`: accepted offline-first, and
    // known locally ONLY through this cessation (the `medication_patient_consistency.rs`
    // finding-3 shape).
    let orphan = Uuid::now_v7();
    cease_medication(
        &mut c,
        &sk,
        &kid,
        "origin-a",
        other,
        orphan,
        &CeaseMedicationInput {
            stopped: Some("2025"),
            stopped_precision: Some("year"),
            reason: Some("stopped elsewhere"),
        },
        None,
        None,
    )
    .await
    .expect("an orphan cessation is accepted offline-first");
    group(&c, ta, orphan).await;

    let list = list_patient_medications(&c, a).await.unwrap();
    let row = list.rows.iter().find(|r| r.group_id == ta).unwrap();
    assert_eq!(
        row.source_charts,
        vec![a],
        "the orphan's chart is invisible to the statement-derived source list"
    );
    assert!(
        row.cross_patient,
        "yet the group reaches another chart, through the cessation alone"
    );
    assert_eq!(withheld_rows(&list.rows), vec![ta]);

    submit_link_event(&c, &sk, &kid, a, other, 10, true).await;
    let linked = list_patient_medications(&c, a).await.unwrap();
    let row = linked.rows.iter().find(|r| r.group_id == ta).unwrap();
    assert!(
        !row.cross_patient,
        "once linked, the other chart is this person: no hazard on this path either"
    );
}

/// Review focus 4: the same drug recorded on two charts that turn out to be one person must
/// never read as two quiet lines — that is a double-dose reading hazard. The positive control
/// (nothing flagged before the link) proves the flag is about the SET, not about the drug.
#[tokio::test]
async fn the_same_drug_on_two_linked_charts_is_flagged() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    c.batch_execute("TRUNCATE patient_link, person_member")
        .await
        .unwrap();
    let (sk, kid, _hsk, _hkid) = setup(&c).await;
    let a = chart(&c, &sk, &kid).await;
    let b = chart(&c, &sk, &kid).await;
    assert_one(&mut c, &sk, &kid, a, "metformin").await;
    assert_one(&mut c, &sk, &kid, b, "Metformin ").await; // the dup_key lowers and trims

    // Positive control: before the link each chart holds ONE metformin and nothing is flagged,
    // so the flag below is about the SET, not about metformin.
    let alone = list_patient_medications(&c, a).await.unwrap();
    assert!(alone.rows.iter().all(|r| !r.reconciliation_flagged));

    submit_link_event(&c, &sk, &kid, a, b, 10, true).await;
    let list = list_patient_medications(&c, a).await.unwrap();
    assert_eq!(
        list.rows.len(),
        2,
        "two recordings, two lines — until reconciled"
    );
    assert!(
        list.rows.iter().all(|r| r.reconciliation_flagged),
        "the same drug on two linked charts must never show twice unflagged"
    );
}

/// Review focus 2: the set follows the STANDING links only — an unlink splits the read again,
/// and the other chart's drug leaves this list with it.
#[tokio::test]
async fn an_unlink_splits_the_read_again() {
    let Some(base) = cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut c = db::connect_and_load_schema(&base).await.unwrap();
    c.batch_execute("TRUNCATE patient_link, person_member")
        .await
        .unwrap();
    let (sk, kid, _hsk, _hkid) = setup(&c).await;
    let a = chart(&c, &sk, &kid).await;
    let b = chart(&c, &sk, &kid).await;
    assert_one(&mut c, &sk, &kid, a, "metformin").await;
    assert_one(&mut c, &sk, &kid, b, "amlodipine").await;
    submit_link_event(&c, &sk, &kid, a, b, 10, true).await;
    submit_link_event(&c, &sk, &kid, a, b, 11, false).await;
    let list = list_patient_medications(&c, a).await.unwrap();
    assert_eq!(list.charts, ChartSet::single(a));
    assert_eq!(list.rows.len(), 1);
}
