//! §5.3/§5.8 STANDARD patient registration — the funnel's create act. Composes the
//! `cairn-event` wire shape (`RegistrationAssertion`) with the `cairn-patient-search` read
//! model (`SearchAttestation`) that `search::search_patients` already produced, and authors
//! ONE `identity.registration.asserted` event through the real `submit_event` door.
//!
//! # Why this module exists, and what it deliberately does NOT do
//!
//! §5.8 requires the act that mints a chart to record the search that preceded it — see
//! `cairn_event::registration`'s module doc for why that must be an ACT (not a side effect
//! of whatever event happens to carry the `patient_id` first) and why the attestation NAMES
//! candidates rather than counting them. This module is where that requirement is discharged
//! for the STANDARD path: a clerk typed a search, saw a `CandidateList`, and chose to create
//! anyway. `register_patient` therefore mints ONLY `RegistrationClass::Standard` — the §5.4
//! John Doe path (`john_doe::register_john_doe`) has nothing to search with and is registered
//! elsewhere, and the §5.6 pseudonymous path is likewise out of scope here.
//!
//! No human-author requirement is added here, and none should ever be added: spec §2.6 says
//! authorship confidence is a GRADE, not a gate (§5.11), and `db/045_patient_registration.sql`'s
//! own header explains why a gate would block care documentation at 03:00 when a clerk's key
//! is not unlocked — it would also push named patients through the John Doe path and leave no
//! forensic record in the case it fired. The contributor set below is the same "the NODE
//! recorded it" shape every additive event in this codebase uses (see
//! `john_doe::build_callsign_name_body`) — a human registrar, when one signs, is added by the
//! CALLER as a bearing role, and `db/005`'s unconditional `cairn_authorship_bound` (step 4b)
//! already makes naming one who did not authenticate unforgeable, with no rule from this file.
//!
//! # Split, mirroring `john_doe.rs`
//!
//! Pure body assembly (`build_registration_body`, unit-tested, no DB) plus the async
//! `register_patient` orchestrator (ticks the HLC, signs, submits inside ONE transaction — so
//! a chart is never half-registered). Today there is only one event to submit, so the
//! transaction is not load-bearing yet the way it is in `register_john_doe`'s two-event case
//! — it is kept anyway so a LATER second event added to this path is atomic by construction,
//! not by a future author remembering to wrap one in.
//!
//! # The one-home rule for the cross-crate conversion
//!
//! `cairn_event::registration::SearchAttestationInput` takes borrowed primitives (`cairn-event`
//! is the wire core and must not depend on a read-model crate — see that module's own doc for
//! why). `cairn_patient_search::SearchAttestation` is the owned, already-derived read-model
//! value the search surface hands back. Converting between the two has EXACTLY ONE home: here,
//! inside `build_registration_body`. If that conversion existed in two places they could drift
//! apart silently, and a drift here means a registration swearing to candidates the clerk
//! never actually saw — the load-bearing round-trip test in `tests/patient_register.rs` is
//! what keeps this the only place it happens.

use cairn_event::registration::{
    registration_assertion_body, render_registration_twin, RegistrationAssertion,
    RegistrationClass, SearchAttestationInput, SearchTerms, REGISTRATION_EVENT_TYPE,
    REGISTRATION_SCHEMA_VERSION,
};
use cairn_event::{sign, ClockGrade, EventBody, Hlc, SigningKey};
use cairn_patient_search::{CandidateList, SearchAttestation, SearchQuery};
use tokio_postgres::Client;
use uuid::Uuid;

/// Assemble the `identity.registration.asserted` `EventBody`. Pure: every input is supplied
/// by the caller (including the freshly-minted `patient_id` and the HLC-derived `event_id`),
/// so the whole wire shape is unit-testable with no clock, no database, and no key.
///
/// `attestation` is `Some` for a `Standard` registration (the db/045 floor requires it
/// present) and `None` for the non-standard classes (the floor requires it structurally
/// ABSENT — see `cairn_event::registration`'s own doc for why absence is not merely
/// optional). This function does not itself enforce that pairing: like the typed builder it
/// wraps, it deliberately PERMITS the illegal states too (twelfth founding principle — the
/// unbypassable enforcement point is the database, not one client's types). A caller that
/// gets the pairing backwards is refused at submit time, never silently admitted.
///
/// THE ONE HOME for the `SearchAttestation` -> `SearchAttestationInput` conversion (see the
/// module doc). `cairn-event`'s builder takes borrowed slices, so this borrows straight out
/// of `attestation` rather than cloning — the borrow only needs to live for this call, which
/// is all `registration_assertion_body`/`render_registration_twin` require.
pub fn build_registration_body(
    event_id: Uuid,
    patient_id: Uuid,
    class: RegistrationClass,
    basis: Option<&str>,
    attestation: Option<&SearchAttestation>,
    kid: &str,
    hlc: Hlc,
) -> EventBody {
    let search = attestation.map(|a| SearchAttestationInput {
        terms: SearchTerms {
            name_tokens: &a.query.name_tokens,
            birth_date: a.query.birth_date.as_deref(),
            identifiers: &a.query.identifiers,
        },
        // Borrowed straight out of `a.displayed` — no re-ordering, no re-deriving. Keeping
        // the SAME `Vec<Uuid>` (not, say, sorting it "for tidiness") is exactly what the
        // round-trip test in tests/patient_register.rs pins: the stored order must be the
        // display order, because that is the order the clerk actually read it in.
        displayed: &a.displayed,
        incomplete: a.incomplete,
    });
    let assertion = RegistrationAssertion {
        class,
        basis,
        search,
    };
    EventBody {
        event_id: event_id.to_string(),
        patient_id: patient_id.to_string(),
        event_type: REGISTRATION_EVENT_TYPE.into(),
        schema_version: REGISTRATION_SCHEMA_VERSION.into(),
        hlc,
        t_effective: None,
        signer_key_id: kid.into(),
        // "recorded", never "authored": the NODE recorded this registration. A human
        // registrar, when one signs, is added by the CALLER as a bearing role — see the
        // module doc for why no such requirement belongs in this builder.
        contributors: serde_json::json!([{"actor_id": kid, "role": "recorded"}]),
        payload: registration_assertion_body(&assertion),
        attachments: vec![],
        plaintext_twin: Some(render_registration_twin(&assertion)),
        clock_grade: ClockGrade::SelfAsserted,
    }
}

/// Register a standard patient: mint a UUID, derive the §5.8 search attestation from the
/// query and candidate list the clerk actually saw, and author ONE
/// `identity.registration.asserted` event through the real `submit_event` door — inside a
/// transaction, mirroring `register_john_doe` (a chart is never half-registered).
///
/// `query`/`displayed` are exactly what `patient::search::search_patients` produced and the
/// clerk was shown. `SearchAttestation::from_displayed` (cairn-patient-search) derives the
/// attestation FROM them rather than letting this function re-decide what was displayed —
/// see `cairn_patient_search::attestation`'s module doc ("the whole reason the crate
/// exists") for why the attestation must be derived, never independently constructed.
///
/// Returns the minted `patient_id`. Care can proceed against it immediately once this
/// returns — the same "UUID minted immediately" shape `register_john_doe` already gives.
pub async fn register_patient(
    client: &mut Client,
    sk: &SigningKey,
    kid: &str,
    node_origin: &str,
    query: &SearchQuery,
    displayed: &CandidateList,
) -> anyhow::Result<Uuid> {
    let patient_id = Uuid::now_v7();
    let attestation = SearchAttestation::from_displayed(query, displayed);

    // Tick the HLC before assembling the body it stamps. This tick self-commits outside the
    // transaction below; if that transaction then rolled back the clock would simply have
    // advanced with no matching event, which is fine — the HLC is monotonic and gaps are
    // allowed (the identical shape `register_john_doe` and `auto_apply` already use).
    let hlc = crate::db::next_hlc(client, node_origin).await?;
    let body = build_registration_body(
        Uuid::now_v7(),
        patient_id,
        RegistrationClass::Standard,
        None,
        Some(&attestation),
        kid,
        hlc,
    );
    let signed = sign(&body, sk)?;

    let tx = client.transaction().await?;
    tx.execute("SELECT submit_event($1)", &[&signed.signed_bytes])
        .await?;
    tx.commit().await?;

    Ok(patient_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_patient_search::{Candidate, TrustState};

    fn hlc(wall: i64) -> Hlc {
        Hlc {
            wall,
            counter: 0,
            node_origin: "n".into(),
        }
    }

    fn candidate(id: Uuid) -> Candidate {
        Candidate {
            patient_id: id,
            display_name: "Some One".into(),
            age: None,
            trust: TrustState::Confirmed,
            last_activity: None,
            locale: None,
            photo_ref: None,
        }
    }

    #[test]
    fn a_standard_body_carries_the_attestation_and_no_basis() {
        let pid = Uuid::from_u128(1);
        let displayed_ids = [Uuid::from_u128(10), Uuid::from_u128(11)];
        let list = CandidateList {
            candidates: displayed_ids.iter().map(|id| candidate(*id)).collect(),
            incomplete: false,
            incomplete_reason: None,
        };
        let query = SearchQuery::new("smith", None, &[]);
        let attestation = SearchAttestation::from_displayed(&query, &list);
        let body = build_registration_body(
            Uuid::from_u128(2),
            pid,
            RegistrationClass::Standard,
            None,
            Some(&attestation),
            "kid",
            hlc(1),
        );
        assert_eq!(body.event_type, "identity.registration.asserted");
        assert_eq!(body.patient_id, pid.to_string());
        assert_eq!(body.payload["class"], "standard");
        assert!(
            body.payload.get("basis").is_none(),
            "a standard registration carries no basis"
        );
        assert_eq!(
            body.payload["search"]["displayed"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        let twin = body.plaintext_twin.as_deref().unwrap();
        assert!(
            !twin.trim().is_empty(),
            "the demographic floor HARD-requires a non-empty twin"
        );
    }

    #[test]
    fn a_non_standard_body_carries_no_search_key() {
        // Pure-assembly proof that this builder permits the illegal pairing rather than
        // gatekeeping it itself — the twelfth founding principle applied. The DB floor
        // (db/045, exercised in tests/patient_registration.rs) is the actual enforcement
        // point; this test only pins what the BUILDER does with mismatched inputs.
        let body = build_registration_body(
            Uuid::from_u128(2),
            Uuid::from_u128(1),
            RegistrationClass::Unidentified,
            Some("unconscious, no ID"),
            None,
            "kid",
            hlc(1),
        );
        assert!(body.payload.get("search").is_none());
        assert_eq!(body.payload["basis"], "unconscious, no ID");
    }

    #[test]
    fn contributors_are_recorded_only_no_responsibility_claimed() {
        let body = build_registration_body(
            Uuid::from_u128(2),
            Uuid::from_u128(1),
            RegistrationClass::Standard,
            None,
            None,
            "kid",
            hlc(1),
        );
        let c = &body.contributors[0];
        assert_eq!(c["role"], "recorded");
        assert!(
            c.get("responsibility").is_none(),
            "no attestation is demanded of a recorded-only contributor"
        );
    }
}
