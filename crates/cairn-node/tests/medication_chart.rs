//! Charting slice 1a, Task 4 — the node signs a charted line, reads its charting back, and
//! searches the drug list for the medication pick.
//!
//! * `chart_medication` signs a human-authored assertion carrying the charting keys; both the
//!   statement and the charting projection take it. A combination carries no safety claim.
//! * `assert_medication` (now a delegate of `chart_medication` with no charting) signs exactly
//!   the payload it signed before charting existed, and writes no charting row.
//! * `read_charting` reads a thread's charting row by medication_id.
//! * `search_medication_drugs` says "no drug list" (`None`) without drugref, offers drugs only
//!   with it, and never fails its caller when the drug list raises.
//!
//! drugref is the stand-in `tests/fixtures/drug_stub.sql`, loaded only inside a transaction that
//! is rolled back. DB-gated on `$CAIRN_TEST_PG` (a skipped run prints `skipped:`), serialized via
//! `db::test_serial_guard`.
use cairn_event::medication::{
    medication_assertion_body, render_medication_twin, ChartedAs, Charting, Component,
    MedicationAssertion, OverrideRef, SubstanceCoding, WarningOverride, ALLERGY_SOURCE,
};
use cairn_node::allergy::drug_facts::search_medication_drugs;
use cairn_node::db;
use cairn_node::medication::{
    assert_medication, build_assert_body, build_charted_body, chart_medication, read_charting,
    AssertMedicationInput, AuthorParams, ChartingRow,
};
use serde_json::Value;
use tokio_postgres::Client;
use uuid::Uuid;

mod common;

/// The drugref stand-in (see its header). Never load it outside a transaction that rolls back.
const STUB: &str = include_str!("fixtures/drug_stub.sql");
/// Any well-formed moiety uuid will do: the floor checks a coding's shape, never drugref.
const AMOX: &str = "0f8c4b1e-1b7a-5c2d-9a3e-2b6f7c8d9e01";

/// The serial guard and a connection with the schema loaded.
async fn rig() -> Option<(Client, Client)> {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return None;
    };
    let guard = db::test_serial_guard(&base).await.unwrap();
    let c = db::connect_and_load_schema(&base).await.unwrap();
    Some((guard, c))
}

fn input() -> AssertMedicationInput<'static> {
    AssertMedicationInput {
        term: "Augmentin Duo Forte",
        coding: None,
        formulation: Some("tablet"),
        dose_amount: Some("1"),
        dose_unit: Some("tablet"),
        sig: Some("1 tab bd po"),
        info_source: "clinician-observed",
        started: None,
        started_precision: None,
    }
}

fn components() -> [Component<'static>; 2] {
    [
        Component {
            term: "amoxicillin",
            coding: Some(SubstanceCoding {
                system: "drugref-moiety",
                code: AMOX,
                display: "amoxicillin",
            }),
        },
        Component {
            term: "clavulanic acid",
            coding: None,
        },
    ]
}

/// A started-here combination charted despite an allergy warning (`against`).
fn combination<'a>(parts: &'a [Component<'a>], against: &'a [OverrideRef<'a>]) -> Charting<'a> {
    Charting {
        charted_as: Some(ChartedAs::StartedHere),
        route: Some("po"),
        strength: Some("875/125 mg"),
        components: parts,
        warning_override: Some(WarningOverride {
            reason: "no alternative in the remote clinic",
            against,
        }),
    }
}

/// The node, a human author, and a registered chart.
struct Rig {
    node_sk: cairn_event::SigningKey,
    node_kid: String,
    human_sk: cairn_event::SigningKey,
    human_kid: String,
    patient: Uuid,
}

async fn chart(c: &Client) -> Rig {
    let (node_sk, node_kid, human_sk, human_kid) = common::medication_setup(c).await;
    c.batch_execute("TRUNCATE medication_charting")
        .await
        .unwrap();
    let patient = Uuid::now_v7();
    common::submit_registration(c, &node_sk, &node_kid, patient, 0).await;
    Rig {
        node_sk,
        node_kid,
        human_sk,
        human_kid,
        patient,
    }
}

/// (signer key id, clear payload) of one event.
async fn signed(c: &Client, event_id: Uuid) -> (String, Value) {
    let r = c
        .query_one(
            "SELECT e.signer_key_id, cairn_clear_payload(e)::text \
               FROM event_log e WHERE e.event_id = $1::text::uuid",
            &[&event_id.to_string()],
        )
        .await
        .unwrap();
    (r.get(0), serde_json::from_str(r.get::<_, &str>(1)).unwrap())
}

async fn count(c: &Client, sql: &str, med: Uuid) -> i64 {
    c.query_one(sql, &[&med.to_string()]).await.unwrap().get(0)
}

#[tokio::test]
async fn chart_medication_signs_and_projects() {
    let Some((_g, mut c)) = rig().await else {
        return;
    };
    let r = chart(&c).await;
    let allergy = Uuid::now_v7().to_string();
    let against = [OverrideRef {
        source: ALLERGY_SOURCE,
        reference: &allergy,
    }];
    let parts = components();
    let author = AuthorParams {
        human_sk: &r.human_sk,
        human_kid: &r.human_kid,
    };
    let done = chart_medication(
        &mut c,
        &r.node_sk,
        &r.node_kid,
        "test-node",
        r.patient,
        &input(),
        &combination(&parts, &against),
        Some(&author),
        None,
    )
    .await
    .expect("a charted combination signs");

    let (signer, payload) = signed(&c, done.event_id).await;
    assert_eq!(signer, r.human_kid, "the clinician signs (ADR-0053)");
    assert_eq!(payload["charted_as"], "started-here");
    assert_eq!(
        payload["warning_override"]["against"],
        serde_json::json!([{"source": "allergy", "ref": allergy}])
    );
    assert_eq!(payload["substance"]["components"][1]["coding"], Value::Null);
    assert!(
        payload.get("safety").is_none(),
        "a combination carries no safety claim (ADR-0063 withholding)"
    );
    let statement = "SELECT count(*) FROM medication_statement \
                      WHERE medication_id = $1::text::uuid";
    let charting = "SELECT count(*) FROM medication_charting \
                     WHERE medication_id = $1::text::uuid";
    assert_eq!(count(&c, statement, done.value).await, 1);
    assert_eq!(count(&c, charting, done.value).await, 1);
}

/// `assert_medication` signs the pre-charting payload: the clear payload on the log is exactly
/// what cairn-event builds with no charting at all, and no charting row is written.
#[tokio::test]
async fn assert_medication_is_unchanged() {
    let Some((_g, mut c)) = rig().await else {
        return;
    };
    let r = chart(&c).await;
    let i = input();
    let med = assert_medication(
        &mut c,
        &r.node_sk,
        &r.node_kid,
        "test-node",
        r.patient,
        &i,
        None,
        None,
    )
    .await
    .unwrap();
    let event: String = c
        .query_one(
            "SELECT event_id::text FROM event_log \
              WHERE event_type = 'clinical.medication.asserted' \
                AND patient_id = $1::text::uuid",
            &[&r.patient.to_string()],
        )
        .await
        .unwrap()
        .get(0);
    let (_, payload) = signed(&c, event.parse().unwrap()).await;
    let mid = med.to_string();
    let before = MedicationAssertion {
        medication_id: &mid,
        term: i.term,
        coding: i.coding,
        formulation: i.formulation,
        dose_amount: i.dose_amount,
        dose_unit: i.dose_unit,
        sig: i.sig,
        info_source: i.info_source,
        started: i.started,
        started_precision: i.started_precision,
        safety: None,
        charting: None,
    };
    assert_eq!(payload, medication_assertion_body(&before));
    let charting = "SELECT count(*) FROM medication_charting \
                     WHERE medication_id = $1::text::uuid";
    assert_eq!(count(&c, charting, med).await, 0);

    // And the pure builders agree: no charting is the old body, payload and twin alike.
    let (e, m) = (Uuid::now_v7(), med);
    let hlc = || cairn_event::Hlc {
        wall: 1,
        counter: 0,
        node_origin: "test-node".into(),
    };
    let old = build_assert_body(e, m, r.patient, &i, &r.node_kid, hlc(), None);
    let new = build_charted_body(
        e,
        m,
        r.patient,
        &i,
        &Charting::default(),
        &r.node_kid,
        hlc(),
        None,
    );
    assert_eq!(new.payload, medication_assertion_body(&before));
    assert_eq!(new.payload, old.payload);
    assert_eq!(new.plaintext_twin, Some(render_medication_twin(&before)));
}

#[tokio::test]
async fn read_charting_reads_by_thread() {
    let Some((_g, mut c)) = rig().await else {
        return;
    };
    let r = chart(&c).await;
    let allergy = Uuid::now_v7().to_string();
    let against = [OverrideRef {
        source: ALLERGY_SOURCE,
        reference: &allergy,
    }];
    let parts = components();
    let already = Charting {
        charted_as: Some(ChartedAs::PreExisting),
        route: Some("po"),
        strength: Some("500 mg"),
        ..Charting::default()
    };
    let mut threads = Vec::new();
    for charting in [combination(&parts, &against), already, Charting::default()] {
        let done = chart_medication(
            &mut c,
            &r.node_sk,
            &r.node_kid,
            "test-node",
            r.patient,
            &input(),
            &charting,
            None,
            None,
        )
        .await
        .unwrap();
        threads.push(done.value);
    }
    let (combo, single, plain) = (threads[0], threads[1], threads[2]);
    let got = read_charting(&c, &[combo, single, plain, Uuid::now_v7()])
        .await
        .unwrap();
    assert_eq!(
        got.len(),
        2,
        "an uncharted thread and an unknown id have no row"
    );
    assert_eq!(
        got[&combo],
        ChartingRow {
            charted_as: Some("started-here".into()),
            route: Some("po".into()),
            strength: Some("875/125 mg".into()),
            components: vec![
                (
                    "amoxicillin".into(),
                    Some(("drugref-moiety".into(), AMOX.into(), "amoxicillin".into()))
                ),
                ("clavulanic acid".into(), None),
            ],
            override_reason: Some("no alternative in the remote clinic".into()),
        }
    );
    assert_eq!(
        got[&single],
        ChartingRow {
            charted_as: Some("pre-existing".into()),
            route: Some("po".into()),
            strength: Some("500 mg".into()),
            components: vec![],
            override_reason: None,
        }
    );
}

#[tokio::test]
async fn search_medication_drugs_none_without_drugref() {
    let Some((_g, c)) = rig().await else {
        return;
    };
    let none: bool = c
        .query_one("SELECT to_regnamespace('drugref') IS NULL", &[])
        .await
        .unwrap()
        .get(0);
    assert!(none, "this test needs a database with no drugref schema");
    assert_eq!(
        search_medication_drugs(&c, "amoxicillin", 12)
            .await
            .unwrap(),
        None
    );
}

/// With drugref the pick offers drugs only, each coded under `drugref-moiety`.
#[tokio::test]
async fn search_medication_drugs_offers_moieties() {
    let Some((_g, mut c)) = rig().await else {
        return;
    };
    let tx = c.transaction().await.unwrap();
    tx.batch_execute(STUB).await.unwrap();
    let found = search_medication_drugs(&tx, "amox", 12)
        .await
        .unwrap()
        .expect("the stub is a drug list");
    let amox: String = tx
        .query_one("SELECT md5('stub moiety amoxicillin')::uuid::text", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        found
            .iter()
            .map(|o| (o.system.as_str(), o.code.as_str(), o.display.as_str()))
            .collect::<Vec<_>>(),
        [("drugref-moiety", amox.as_str(), "amoxicillin")]
    );
    tx.rollback().await.unwrap();
}

/// A drug list that raises (here: a role that may ask whether it is there, but not search it)
/// reads as no drug list: the line charts uncoded, "not checked" (Review focus 5).
#[tokio::test]
async fn search_medication_drugs_never_fails_the_caller() {
    let Some((_g, mut c)) = rig().await else {
        return;
    };
    let tx = c.transaction().await.unwrap();
    tx.batch_execute(STUB).await.unwrap();
    tx.batch_execute(
        "GRANT EXECUTE ON FUNCTION cairn_drug_available() TO cairn_agent; \
         SET LOCAL ROLE cairn_agent",
    )
    .await
    .unwrap();
    let who: String = tx
        .query_one("SELECT current_user::text", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(who, "cairn_agent", "the role switch really happened");
    let available: bool = tx
        .query_one("SELECT cairn_drug_available()", &[])
        .await
        .unwrap()
        .get(0);
    assert!(
        available,
        "the drug list is there; only the search is refused"
    );
    assert_eq!(
        search_medication_drugs(&tx, "amox", 12).await.unwrap(),
        None
    );
    tx.rollback().await.unwrap();
}
