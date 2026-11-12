//! Allergy slice 2, Task 3 — signing the two overlays (a de-label by challenge, an entered-in-error
//! withdrawal) and reading every overlay back, with each signer's key state. A correction is an
//! ordinary entry carrying a `correction-of` ref; it reads back with the line it replaces.
//! DB-gated on `$CAIRN_TEST_PG`, serialized via `db::test_serial_guard` taken BEFORE connecting.
use cairn_event::allergy::{
    AllergenKind, AllergyEntry, EntryKind, NoneScope, Onset, ReactionCategory, Source,
};
use cairn_event::allergy_overlay::{AllergyChallenge, AllergyRetraction};
use cairn_event::SigningKey;
use cairn_node::allergy::read::{list_patient_allergies, KeyState};
use cairn_node::allergy::sign::{
    sign_allergy, sign_allergy_challenge, sign_allergy_none, sign_allergy_retraction,
};
use cairn_node::db;
use cairn_node::medication::AuthorParams;
use cairn_node::shred::shred_event;
use tokio_postgres::Client;
use uuid::Uuid;

mod common;

fn entry<'a>(term: &'a str, corrects: Option<&'a str>) -> AllergyEntry<'a> {
    AllergyEntry {
        allergen_kind: AllergenKind::Drug,
        term,
        kind: EntryKind::Allergy,
        category: ReactionCategory::Anaphylaxis,
        reaction_text: None,
        onset: Onset::Year("2019".into()),
        source: Source::PatientStated,
        corrects,
        coding: None,
    }
}

/// The node's and the human's keys plus one registered chart (`note_setup`, so the signer is named).
struct Ward {
    c: Client,
    node_sk: SigningKey,
    node_kid: String,
    human_sk: SigningKey,
    human_kid: String,
    chart: Uuid,
}

impl Ward {
    async fn new(base: &str) -> Ward {
        let c = db::connect_and_load_schema(base).await.unwrap();
        let (node_sk, node_kid, human_sk, human_kid, chart) = common::note_setup(&c).await;
        Ward {
            c,
            node_sk,
            node_kid,
            human_sk,
            human_kid,
            chart,
        }
    }

    async fn sign_entry(&mut self, patient: Uuid, e: &AllergyEntry<'_>) -> Uuid {
        let author = AuthorParams {
            human_sk: &self.human_sk,
            human_kid: &self.human_kid,
        };
        sign_allergy(
            &mut self.c,
            &self.node_sk,
            &self.node_kid,
            "n",
            patient,
            e,
            &author,
        )
        .await
        .unwrap()
    }

    /// A challenge on `patient`'s chart: "tolerated amoxicillin in March 2022".
    async fn challenge(&mut self, patient: Uuid, target: Uuid) -> Uuid {
        let author = AuthorParams {
            human_sk: &self.human_sk,
            human_kid: &self.human_kid,
        };
        let t = target.to_string();
        let ch = AllergyChallenge {
            target: &t,
            tolerated: "amoxicillin",
            tolerated_coding: None,
            date: Onset::Month("2022-03".into()),
            text: Some("oral challenge"),
        };
        sign_allergy_challenge(
            &mut self.c,
            &self.node_sk,
            &self.node_kid,
            "n",
            patient,
            &ch,
            &author,
        )
        .await
        .unwrap()
    }

    async fn retract(&mut self, patient: Uuid, target: Uuid) -> Uuid {
        let author = AuthorParams {
            human_sk: &self.human_sk,
            human_kid: &self.human_kid,
        };
        let t = target.to_string();
        let r = AllergyRetraction {
            target: &t,
            text: Some("wrong patient"),
        };
        sign_allergy_retraction(
            &mut self.c,
            &self.node_sk,
            &self.node_kid,
            "n",
            patient,
            &r,
            &author,
        )
        .await
        .unwrap()
    }
}

/// The `(actor_id, role)` contributors of event `id`.
async fn roles(c: &Client, id: Uuid) -> Vec<(String, String)> {
    c.query(
        "SELECT x->>'actor_id', x->>'role' FROM event_log e, jsonb_array_elements(e.contributors) x \
         WHERE e.event_id = $1::text::uuid",
        &[&id.to_string()],
    )
    .await
    .unwrap()
    .iter()
    .map(|r| (r.get(0), r.get(1)))
    .collect()
}

#[tokio::test]
async fn signing_a_challenge_and_a_retraction_records_human_authorship() {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut w = Ward::new(&base).await;
    let a = w.chart;
    let e = w.sign_entry(a, &entry("penicillin", None)).await;
    let ch = w.challenge(a, e).await;
    let re = w.retract(a, e).await;
    for id in [ch, re] {
        let mut got = roles(&w.c, id).await;
        got.sort();
        let mut want = vec![
            (w.human_kid.clone(), "authored".to_string()),
            (w.node_kid.clone(), "recorded".to_string()),
        ];
        want.sort();
        assert_eq!(got, want, "exactly {{human authored, node recorded}}");
        let no_safety: bool =
            w.c.query_one(
                "SELECT safety IS NULL FROM event_log WHERE event_id = $1::text::uuid",
                &[&id.to_string()],
            )
            .await
            .unwrap()
            .get(0);
        assert!(no_safety, "no safety signal on an overlay");
    }
}

#[tokio::test]
async fn a_correction_reads_back_with_its_target() {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut w = Ward::new(&base).await;
    let a = w.chart;
    let wrong = w.sign_entry(a, &entry("penicilin", None)).await;
    let ws = wrong.to_string();
    let fixed = w.sign_entry(a, &entry("penicillin", Some(&ws))).await;
    let got = list_patient_allergies(&w.c, a).await.unwrap();
    let corrects = |id: Uuid| {
        got.entries
            .iter()
            .find(|r| r.event_id == id)
            .unwrap()
            .corrects
    };
    assert_eq!(corrects(fixed), Some(wrong));
    assert_eq!(corrects(wrong), None);
}

#[tokio::test]
async fn overlays_read_back_over_a_linked_set() {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut w = Ward::new(&base).await;
    let a = w.chart;
    let x = Uuid::now_v7();
    common::submit_registration(&w.c, &w.node_sk, &w.node_kid, x, 0).await;
    let ea = w.sign_entry(a, &entry("penicillin", None)).await;
    let ex = w.sign_entry(x, &entry("sulfa", None)).await;
    let ch = w.challenge(x, ex).await;
    let re = w.retract(a, ea).await;
    common::submit_link_event(&w.c, &w.node_sk, &w.node_kid, a, x, 10, true).await;

    let got = list_patient_allergies(&w.c, a).await.unwrap();
    assert_eq!(got.challenges.len(), 1);
    let c = &got.challenges[0];
    assert_eq!((c.event_id, c.chart, c.target), (ch, x, ex));
    assert_eq!(c.tolerated, "amoxicillin");
    assert_eq!(
        (c.date_precision.as_str(), c.date_value.as_deref()),
        ("month", Some("2022-03"))
    );
    assert_eq!(c.text.as_deref(), Some("oral challenge"));
    assert_eq!(c.signer.as_deref(), Some(common::NOTE_AUTHOR));
    assert_eq!(c.key, KeyState::Standing);
    assert!((chrono::Utc::now() - c.recorded_at).num_seconds().abs() < 300);

    assert_eq!(got.retractions.len(), 1);
    let r = &got.retractions[0];
    assert_eq!((r.event_id, r.chart, r.target), (re, a, ea));
    assert_eq!(r.text.as_deref(), Some("wrong patient"));
    assert_eq!(r.signer.as_deref(), Some(common::NOTE_AUTHOR));
    assert_eq!(r.key, KeyState::Standing);
    assert_eq!(got.unreadable, 0);
}

/// A stolen key is repudiated after it signed: every row it signed says so (the view decides what
/// a repudiated signature is worth; the read only reports it).
#[tokio::test]
async fn a_repudiated_signer_reads_as_repudiated() {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut w = Ward::new(&base).await;
    let a = w.chart;
    let e = w.sign_entry(a, &entry("penicillin", None)).await;
    let author = AuthorParams {
        human_sk: &w.human_sk,
        human_kid: &w.human_kid,
    };
    sign_allergy_none(
        &mut w.c,
        &w.node_sk,
        &w.node_kid,
        "n",
        a,
        NoneScope::Drug,
        &author,
    )
    .await
    .unwrap();
    w.challenge(a, e).await;
    w.retract(a, e).await;
    let before = list_patient_allergies(&w.c, a).await.unwrap();
    assert_eq!(before.entries[0].key, KeyState::Standing);
    assert_eq!(before.none[0].key, KeyState::Standing);

    // As note_repudiated_signer.rs does: rotate the actor onto a fresh key, then repudiate the old.
    let aid: Vec<u8> =
        w.c.query_one(
            "SELECT actor_id FROM actor_event WHERE signing_key_id = $1 AND op = 'enroll'",
            &[&w.human_kid],
        )
        .await
        .unwrap()
        .get(0);
    let (_sk, kid) = cairn_event::generate_key().unwrap();
    w.c.execute(
        "SELECT cairn_actor_append($1, 'rotate-key', $2, NULL, NULL)",
        &[&aid, &kid],
    )
    .await
    .unwrap();
    w.c.execute(
        "SELECT cairn_actor_append($1, 'repudiate', $2, 'console', NULL)",
        &[&aid, &w.human_kid],
    )
    .await
    .unwrap();

    let got = list_patient_allergies(&w.c, a).await.unwrap();
    assert_eq!(got.entries[0].key, KeyState::Repudiated);
    assert_eq!(got.none[0].key, KeyState::Repudiated);
    assert_eq!(got.challenges[0].key, KeyState::Repudiated);
    assert_eq!(got.retractions[0].key, KeyState::Repudiated);
}

/// An overlay this node holds but cannot read (shredded here; no custody reads the same) is
/// COUNTED, never silently absent — and its target entry still reads back: an overlay the node
/// cannot read must never take an allergy off the list.
#[tokio::test]
async fn an_unreadable_overlay_is_counted_and_its_target_stays_live() {
    let Some(base) = common::cs() else {
        eprintln!("skipped: set CAIRN_TEST_PG");
        return;
    };
    let _guard = db::test_serial_guard(&base).await.unwrap();
    let mut w = Ward::new(&base).await;
    let a = w.chart;
    let e = w.sign_entry(a, &entry("penicillin", None)).await;
    let ch = w.challenge(a, e).await;
    let re = w.retract(a, e).await;
    assert_eq!(list_patient_allergies(&w.c, a).await.unwrap().unreadable, 0);
    for id in [ch, re] {
        shred_event(
            &mut w.c,
            &w.node_sk,
            &w.node_kid,
            &w.node_kid,
            id,
            "retention ceiling",
            None,
        )
        .await
        .expect("the shred is admitted");
    }
    let got = list_patient_allergies(&w.c, a).await.unwrap();
    assert_eq!(got.unreadable, 2, "the shredded challenge and withdrawal");
    let entries: Vec<Uuid> = got.entries.iter().map(|r| r.event_id).collect();
    assert_eq!(entries, vec![e], "the target still reads back");
    assert!(got.challenges.is_empty());
    assert!(got.retractions.is_empty());
}
