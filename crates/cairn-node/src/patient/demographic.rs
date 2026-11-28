//! The door for the demographics a registration does not record (#1211).
//!
//! # Why this exists
//!
//! The floor has understood a stated sex, a gender identity, a sex at birth, an address and
//! a name's use since db/011 to db/014. No command could write them: `patient-register`
//! records a name, a date of birth and identifiers, and `assert-observed-evidence` can only
//! say that a clinician LOOKED at the patient. A registrar could not record where a patient
//! lives, or what the patient says their sex is. On a paper front sheet those are two lines.
//!
//! # What it is
//!
//! One additive `demographic.field.asserted` event per fact, on an EXISTING chart, through
//! `submit_event`. Nothing is edited: a new address or a corrected sex is a further
//! assertion, and the projections (db/011, db/013, db/014) pick the one to display. The
//! sole contributor is the recording actor with role `recorded`, so no attestation is asked
//! for (as `evidence.rs` and `john_doe.rs`).
//!
//! # What it deliberately does not do
//!
//! * **It parses nothing.** A name and an address are recorded as typed. An address is its
//!   `display` line only: structured parts need a locale profile (ADR-0032) and none exists
//!   yet, so this slice writes the part of an address that is legible everywhere.
//! * **It closes no vocabulary.** Sex and gender are open strings (principle 4: intersex,
//!   indeterminate, unknown and declined must all be recordable).
//! * **It chooses no provenance.** The caller states where the facts came from. There is no
//!   good default: `patient-stated` would be a lie for an import, and `unknown` would throw
//!   away what a registrar at the desk knows.
//!
//! Split as `evidence.rs` is: pure body assembly (unit-tested, no database) and the async
//! [`assert_demographics`], one transaction so the facts land together or not at all.

use cairn_event::demographics::{
    address_assertion_body, administrative_sex_assertion_body, gender_identity_assertion_body,
    name_assertion_body, render_address_twin, render_administrative_sex_twin,
    render_gender_identity_twin, render_name_twin, render_sex_at_birth_twin,
    sex_at_birth_assertion_body, AddressAssertion,
};
use cairn_event::{sign, EventBody, Hlc, SigningKey};
use serde_json::Value;
use tokio_postgres::Client;
use uuid::Uuid;

/// The schema version of a `demographic.field.asserted` body (as `patient/register.rs`).
const DEMOGRAPHIC_FIELD_SCHEMA_VERSION: &str = "demographic.field/1";

/// One demographic fact to record. Every value is the text as given: nothing here is
/// validated against a list, because every list of sexes, genders or address forms is some
/// culture's list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DemographicFact {
    /// A further name, with what it is used for (`legal`, `maiden`, `alias`, …) if stated.
    Name { value: String, use_: Option<String> },
    /// The sex on documents and forms.
    AdministrativeSex { value: String },
    /// The sex recorded at birth.
    SexAtBirth { value: String },
    /// The gender the patient states.
    GenderIdentity { value: String },
    /// The whole address as one readable line, with its use (`residential`, `postal`, …).
    Address {
        display: String,
        use_: Option<String>,
    },
}

impl DemographicFact {
    /// The word the success line and a refusal use for this fact. **Pure.**
    pub fn label(&self) -> &'static str {
        match self {
            DemographicFact::Name { .. } => "name",
            DemographicFact::AdministrativeSex { .. } => "administrative sex",
            DemographicFact::SexAtBirth { .. } => "sex at birth",
            DemographicFact::GenderIdentity { .. } => "gender identity",
            DemographicFact::Address { .. } => "address",
        }
    }

    /// The text this fact asserts. **Pure.**
    fn text(&self) -> &str {
        match self {
            DemographicFact::Name { value, .. }
            | DemographicFact::AdministrativeSex { value }
            | DemographicFact::SexAtBirth { value }
            | DemographicFact::GenderIdentity { value } => value,
            DemographicFact::Address { display, .. } => display,
        }
    }
}

/// What a command line gave, before it is known to make sense: each `None` is a fact the
/// operator did not mention. Kept as a struct so the seven strings cannot be passed in the
/// wrong order (they are all `Option<String>`, and a swap would type-check).
#[derive(Debug, Default, Clone)]
pub struct Stated {
    pub name: Option<String>,
    pub name_use: Option<String>,
    pub administrative_sex: Option<String>,
    pub sex_at_birth: Option<String>,
    pub gender_identity: Option<String>,
    pub address: Option<String>,
    pub address_use: Option<String>,
}

/// Turn what was stated into facts, in a fixed order (name, sexes, gender, address).
/// **Pure.**
///
/// A use with nothing to describe is refused, not dropped: `--address-use postal` with no
/// `--address` is a slip (the address was forgotten), and recording nothing for it in
/// silence would let the operator believe it was filed.
pub fn facts_from(stated: Stated) -> Result<Vec<DemographicFact>, String> {
    if stated.name_use.is_some() && stated.name.is_none() {
        return Err("--name-use says what a name is used for; give the --name too".into());
    }
    if stated.address_use.is_some() && stated.address.is_none() {
        return Err(
            "--address-use says what an address is used for; give the --address too".into(),
        );
    }
    let mut facts = Vec::new();
    if let Some(value) = stated.name {
        facts.push(DemographicFact::Name {
            value,
            use_: stated.name_use,
        });
    }
    if let Some(value) = stated.administrative_sex {
        facts.push(DemographicFact::AdministrativeSex { value });
    }
    if let Some(value) = stated.sex_at_birth {
        facts.push(DemographicFact::SexAtBirth { value });
    }
    if let Some(value) = stated.gender_identity {
        facts.push(DemographicFact::GenderIdentity { value });
    }
    if let Some(display) = stated.address {
        facts.push(DemographicFact::Address {
            display,
            use_: stated.address_use,
        });
    }
    Ok(facts)
}

/// Why this set of facts cannot be recorded, or `None` if it can. **Pure.**
///
/// Checked before anything is signed, so a refusal costs no HLC tick and writes nothing.
/// The floor would refuse an empty value too (db/011); saying it here names WHICH fact was
/// empty, where the floor can only say that one was.
pub fn refusal(facts: &[DemographicFact], provenance: &str) -> Option<String> {
    if facts.is_empty() {
        return Some("nothing to record: give at least one fact".into());
    }
    if provenance.trim().is_empty() {
        return Some(
            "say where these facts came from (--provenance): patient-stated, \
             document-verified, third-party-stated, clinician-observed, imported or unknown"
                .into(),
        );
    }
    facts.iter().find(|f| f.text().trim().is_empty()).map(|f| {
        format!(
            "the {} is empty; leave it out if it is not known",
            f.label()
        )
    })
}

/// The payload and the legibility twin of one fact. **Pure.**
fn payload_and_twin(fact: &DemographicFact, provenance: &str) -> (Value, String) {
    match fact {
        DemographicFact::Name { value, use_ } => (
            name_assertion_body(value, use_.as_deref(), provenance),
            render_name_twin(value, use_.as_deref(), provenance),
        ),
        DemographicFact::AdministrativeSex { value } => (
            administrative_sex_assertion_body(value, provenance),
            render_administrative_sex_twin(value, provenance),
        ),
        DemographicFact::SexAtBirth { value } => (
            sex_at_birth_assertion_body(value, provenance),
            render_sex_at_birth_twin(value, provenance),
        ),
        DemographicFact::GenderIdentity { value } => (
            gender_identity_assertion_body(value, provenance),
            render_gender_identity_twin(value, provenance),
        ),
        DemographicFact::Address { display, use_ } => {
            let a = AddressAssertion {
                display,
                provenance,
                use_: use_.as_deref(),
                geo: None,
                structured: None,
            };
            (address_assertion_body(&a), render_address_twin(&a))
        }
    }
}

/// Assemble the `demographic.field.asserted` event for one fact. **Pure**: the id and the
/// clock are supplied, so the whole body is testable without a database.
pub fn build_demographic_event(
    event_id: Uuid,
    patient_id: Uuid,
    fact: &DemographicFact,
    provenance: &str,
    kid: &str,
    hlc: Hlc,
) -> EventBody {
    let (payload, twin) = payload_and_twin(fact, provenance);
    EventBody {
        event_id: event_id.to_string(),
        patient_id: patient_id.to_string(),
        event_type: "demographic.field.asserted".into(),
        schema_version: DEMOGRAPHIC_FIELD_SCHEMA_VERSION.into(),
        hlc,
        t_effective: None,
        signer_key_id: kid.into(),
        contributors: serde_json::json!([{"actor_id": kid, "role": "recorded"}]),
        payload,
        attachments: vec![],
        plaintext_twin: Some(twin),
        clock_grade: cairn_event::ClockGrade::SelfAsserted,
        safety: None,
    }
}

/// Record `facts` on `patient_id` in ONE transaction, all with the same `provenance`.
///
/// Returns the events' ids in the order the facts were given, for the CLI to print: a lost
/// commit names the first of them (#817), so an operator should have seen it before.
///
/// The chart must already be registered. That is not checked here: the floor's precedence
/// rule (db/005 step 8b, #345) refuses the first event on a chart unless it is the
/// registration, and a check of our own would be a second, weaker copy of that rule.
pub async fn assert_demographics(
    client: &mut Client,
    sk: &SigningKey,
    kid: &str,
    node_origin: &str,
    patient_id: Uuid,
    facts: &[DemographicFact],
    provenance: &str,
) -> anyhow::Result<Vec<Uuid>> {
    if let Some(why) = refusal(facts, provenance) {
        anyhow::bail!(why);
    }
    // Build + sign each event OUTSIDE the transaction (HLC ticks self-commit and may gap
    // safely). Each event's id is kept beside it: a lost commit names one of them (#817).
    let mut signed = Vec::new();
    for fact in facts {
        let h = crate::db::next_hlc(client, node_origin).await?;
        let id = Uuid::now_v7();
        let body = build_demographic_event(id, patient_id, fact, provenance, kid, h);
        signed.push((id, sign(&body, sk)?));
    }
    let tx = client.transaction().await?;
    for (_, s) in &signed {
        tx.execute("SELECT submit_event($1)", &[&s.signed_bytes])
            .await
            .map_err(|e| anyhow::anyhow!(crate::db_diagnosis::legible_db_error(&e)))?;
    }
    // `signed` is non-empty: `refusal` turned away an empty set above.
    tx.commit()
        .await
        .map_err(|e| crate::db_diagnosis::commit_failure(signed[0].0, e.into()))?;
    Ok(signed.into_iter().map(|(id, _)| id).collect())
}

/// The CLI's success lines: one per recorded fact, naming its kind and its event (#826).
/// **Pure.** `event_ids` is [`assert_demographics`]'s return value, in the same order.
pub fn recorded_lines(patient_id: Uuid, facts: &[DemographicFact], event_ids: &[Uuid]) -> String {
    facts
        .iter()
        .zip(event_ids)
        .map(|(fact, id)| format!("recorded {} for {patient_id}; event {id}", fact.label()))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hlc() -> Hlc {
        Hlc {
            wall: 1,
            counter: 0,
            node_origin: "n".into(),
        }
    }

    fn sex(v: &str) -> DemographicFact {
        DemographicFact::AdministrativeSex { value: v.into() }
    }

    #[test]
    fn an_address_is_recorded_as_its_display_line_with_no_invented_structure() {
        let fact = DemographicFact::Address {
            display: "197 Kurrajong Place, SOLDIERS HILL VIC 3350".into(),
            use_: Some("residential".into()),
        };
        let body = build_demographic_event(
            Uuid::now_v7(),
            Uuid::now_v7(),
            &fact,
            "imported",
            "k",
            hlc(),
        );
        assert_eq!(body.event_type, "demographic.field.asserted");
        assert_eq!(body.payload["field"], "address");
        assert_eq!(
            body.payload["value"],
            "197 Kurrajong Place, SOLDIERS HILL VIC 3350"
        );
        assert_eq!(body.payload["provenance"], "imported");
        assert_eq!(body.payload["facets"]["use"], "residential");
        // No locale profile exists yet (ADR-0032), so no structured parts are claimed.
        assert!(body.payload["facets"].get("structured").is_none());
        assert_eq!(
            body.plaintext_twin.as_deref(),
            Some("Address (residential): 197 Kurrajong Place, SOLDIERS HILL VIC 3350")
        );
    }

    #[test]
    fn each_fact_goes_to_its_own_field_and_carries_a_twin() {
        let cases = [
            (sex("X"), "administrative-sex"),
            (
                DemographicFact::SexAtBirth {
                    value: "unknown".into(),
                },
                "sex-at-birth",
            ),
            (
                DemographicFact::GenderIdentity {
                    value: "non-binary".into(),
                },
                "gender-identity",
            ),
            (
                DemographicFact::Name {
                    value: "田中 太郎".into(),
                    use_: Some("legal".into()),
                },
                "name",
            ),
        ];
        for (fact, field) in cases {
            let body = build_demographic_event(
                Uuid::now_v7(),
                Uuid::now_v7(),
                &fact,
                "patient-stated",
                "k",
                hlc(),
            );
            assert_eq!(body.payload["field"], field);
            assert_eq!(body.payload["value"], fact.text(), "recorded as given");
            let twin = body.plaintext_twin.expect("every event is legible");
            // The message is fixed text on purpose. Printing the twin here would write a sex or
            // a gender to the test log; CodeQL's cleartext-logging query said so (alert #101),
            // and although these values are invented, a test should not model the habit.
            assert!(
                twin.contains(fact.text()),
                "the twin must carry the value exactly as it was given"
            );
        }
    }

    #[test]
    fn the_recorder_is_the_only_contributor_and_bears_no_responsibility() {
        let body = build_demographic_event(
            Uuid::now_v7(),
            Uuid::now_v7(),
            &sex("F"),
            "patient-stated",
            "kid-1",
            hlc(),
        );
        assert_eq!(
            body.contributors,
            serde_json::json!([{"actor_id": "kid-1", "role": "recorded"}])
        );
        assert_eq!(body.signer_key_id, "kid-1");
    }

    #[test]
    fn an_empty_set_a_missing_provenance_and_an_empty_value_are_each_refused_by_name() {
        assert!(refusal(&[], "patient-stated")
            .unwrap()
            .contains("at least one"));
        assert!(refusal(&[sex("F")], "  ").unwrap().contains("--provenance"));
        let empty_address = DemographicFact::Address {
            display: " ".into(),
            use_: None,
        };
        let why = refusal(&[sex("F"), empty_address], "patient-stated").unwrap();
        assert!(why.contains("address"), "names the empty fact: {why}");
        assert_eq!(refusal(&[sex("F")], "patient-stated"), None);
        // An open vocabulary: a term the ladder does not know is the floor's to rank (it
        // ranks last, db/011), never this function's to refuse.
        assert_eq!(refusal(&[sex("F")], "registrar-entered"), None);
    }

    #[test]
    fn stated_flags_become_facts_and_a_use_without_its_value_is_refused() {
        let facts = facts_from(Stated {
            administrative_sex: Some("F".into()),
            address: Some("1 High St".into()),
            address_use: Some("postal".into()),
            ..Stated::default()
        })
        .unwrap();
        assert_eq!(
            facts,
            [
                sex("F"),
                DemographicFact::Address {
                    display: "1 High St".into(),
                    use_: Some("postal".into()),
                },
            ]
        );
        assert_eq!(facts_from(Stated::default()).unwrap(), []);
        let forgotten = facts_from(Stated {
            address_use: Some("postal".into()),
            ..Stated::default()
        });
        assert!(forgotten.unwrap_err().contains("--address"));
        let forgotten = facts_from(Stated {
            name_use: Some("legal".into()),
            ..Stated::default()
        });
        assert!(forgotten.unwrap_err().contains("--name"));
    }

    #[test]
    fn the_success_lines_name_each_fact_and_its_event() {
        let (p, a, b) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
        let facts = [
            sex("intersex"),
            DemographicFact::Address {
                display: "197 Kurrajong Place".into(),
                use_: Some("residential".into()),
            },
        ];
        let lines = recorded_lines(p, &facts, &[a, b]);
        assert_eq!(
            lines,
            format!(
                "recorded administrative sex for {p}; event {a}\n\
                 recorded address for {p}; event {b}"
            )
        );
        // What was typed never reaches a terminal or a log: only the kind of fact and two
        // record ids do. The CodeQL barrier row for this function rests on exactly this.
        for typed in ["intersex", "Kurrajong", "residential"] {
            assert!(
                !lines.contains(typed),
                "{typed} leaked into the success lines"
            );
        }
    }
}
