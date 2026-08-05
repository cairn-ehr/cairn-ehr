//! §5.3/§5.8 STANDARD patient registration — the funnel's create act. Composes the `cairn-event`
//! wire shape (`RegistrationAssertion`) with the `cairn-patient-search` read model
//! (`SearchAttestation`) `search::search_patients` produced, and authors the
//! `identity.registration.asserted` act PLUS the name/dob facts that make the chart findable
//! (#350 / Task 8b — see below), through `submit_event`.
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
//! authorship confidence is a GRADE, not a gate (§5.11) — `db/045_patient_registration.sql`'s
//! own header explains why a gate would block care documentation at 03:00 when a clerk's key
//! is not unlocked. The contributor set below is the same "the NODE recorded it" shape every
//! additive event in this codebase uses (see `john_doe::build_callsign_name_body`) — a human
//! registrar, when one signs, is added by the CALLER as a bearing role.
//!
//! # #350 / Task 8b — the funnel could not catch its own duplicate, and why
//!
//! Task 8 wired `patient-search`/`patient-register` together and found, by running the real
//! CLI end to end, that `register_patient` authored ONLY the registration act: it never wrote
//! to `patient_name`/`patient_demographic`, which is exactly what `search_patients` reads. So
//! "register Jane Smith" -> "search Jane Smith" found nothing, and a second registration
//! silently minted a duplicate chart. Filed as issue #350; this module now ALSO authors a
//! `demographic.field.asserted` name and/or dob event, in the SAME transaction as the
//! registration act (a chart is never half-registered — see `register_patient`'s doc for the
//! atomicity argument), and ONLY when actually supplied (principle 4 — a fabricated
//! placeholder would be a precise untruth, worse than the honest gap left by an
//! identifier-only registration). Provenance is `registrar-entered`, not `patient-stated` —
//! see `REGISTRATION_DEMOGRAPHIC_PROVENANCE`'s own doc for why. And `SearchQuery` cannot
//! supply the raw name a name assertion needs (it retains only normalised tokens) without
//! changing shape for one caller's convenience — see `register_patient`'s doc for why it
//! takes `name` as a separate parameter instead.
//!
//! Split, mirroring `john_doe.rs`: pure body assembly (`build_registration_body`/
//! `build_name_body`/`build_dob_body`, unit-tested, no DB) plus the async `register_patient`
//! orchestrator. And the one-home rule for the cross-crate conversion:
//!
//! `cairn_event::registration::SearchAttestationInput` takes borrowed primitives (`cairn-event`
//! is the wire core and must not depend on a read-model crate — see that module's own doc for
//! why). `cairn_patient_search::SearchAttestation` is the owned, already-derived read-model
//! value the search surface hands back. Converting between the two has EXACTLY ONE home: here,
//! inside `build_registration_body`. If that conversion existed in two places they could drift
//! apart silently, and a drift here means a registration swearing to candidates the clerk
//! never actually saw — the load-bearing round-trip test in `tests/patient_register.rs` is
//! what keeps this the only place it happens.

use cairn_event::demographics::{
    dob_assertion_body, name_assertion_body, render_dob_twin, render_name_twin,
};
use cairn_event::registration::{
    registration_assertion_body, render_registration_twin, RegistrationAssertion,
    RegistrationClass, SearchAttestationInput, SearchTerms, REGISTRATION_EVENT_TYPE,
    REGISTRATION_SCHEMA_VERSION,
};
use cairn_event::{sign, ClockGrade, EventBody, Hlc, SigningKey};
use cairn_patient_search::{CandidateList, SearchAttestation, SearchQuery};
use tokio_postgres::Client;
use uuid::Uuid;

/// schema_version for a `demographic.field.asserted` event (mirrors `john_doe.rs`'s own
/// private copy — no shared location exports one today).
const DEMOGRAPHIC_FIELD_SCHEMA_VERSION: &str = "demographic.field/1";

/// The §4.1 provenance stamped on the name/dob asserted at STANDARD registration. See this
/// module's own doc ("#350 / Task 8b") for the full reasoning; in short: `patient-stated`
/// would often be a precise untruth (the speaker at a registration desk is frequently a
/// third party — a parent, a carer), and `registrar-entered` is the honest description of
/// what actually happened — a clerk wrote down what they were told, unverified.
///
/// Deliberately NOT added to `cairn_provenance_rank` (db/011): that function's own doc
/// states its safe default for an unrecognised term is the LOWEST rank (0, below even
/// "inferred") so a newer/unknown term "can never DISPLACE a known-provenance value" and
/// degrades to "lowest, never highest" — exactly right for a brand-new term today; ranking
/// it explicitly is a separate decision for whoever next audits the whole ladder.
pub const REGISTRATION_DEMOGRAPHIC_PROVENANCE: &str = "registrar-entered";

/// The §4.2 dob precision this module always asserts: `SearchQuery.birth_date` (and the
/// `--birth-date` CLI flag behind it) is documented as a full ISO `YYYY-MM-DD`, so "day" is
/// the honest precision for every dob this module authors.
const REGISTRATION_DOB_PRECISION: &str = "day";

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

/// Assemble a §4.2 NAME `demographic.field.asserted` `EventBody` for the registration path.
/// Pure, mirroring `john_doe::build_callsign_name_body`'s shape. `value` is the RAW name text
/// as the clerk typed it — not `SearchQuery`'s normalised tokens (see the module doc's
/// "signature problem" section for why `register_patient` takes it as a separate parameter).
///
/// `use_` is left `None`: this is not asserted as any particular category (legal/alias/…),
/// just "the name given at registration" — no claim about legal status is being made. The
/// db/012 display-winner still picks it up as the display winner via its documented
/// fallback ("when no legal name exists, the newest name of ANY use wins"), which is exactly
/// right for a chart that, at the moment of registration, has no other name on file yet.
pub fn build_name_body(
    event_id: Uuid,
    patient_id: Uuid,
    value: &str,
    kid: &str,
    hlc: Hlc,
) -> EventBody {
    EventBody {
        event_id: event_id.to_string(),
        patient_id: patient_id.to_string(),
        event_type: "demographic.field.asserted".into(),
        schema_version: DEMOGRAPHIC_FIELD_SCHEMA_VERSION.into(),
        hlc,
        t_effective: None,
        signer_key_id: kid.into(),
        contributors: serde_json::json!([{"actor_id": kid, "role": "recorded"}]),
        payload: name_assertion_body(value, None, REGISTRATION_DEMOGRAPHIC_PROVENANCE),
        attachments: vec![],
        plaintext_twin: Some(render_name_twin(
            value,
            None,
            REGISTRATION_DEMOGRAPHIC_PROVENANCE,
        )),
        clock_grade: ClockGrade::SelfAsserted,
    }
}

/// Assemble a §4.2 DOB `demographic.field.asserted` `EventBody` for the registration path.
/// Pure, mirroring `build_name_body` above. `value` is the raw ISO `YYYY-MM-DD` string —
/// `SearchQuery.birth_date` already carries it untouched (tokenisation is a NAME-only
/// concern), so no separate raw-value plumbing was needed for this field, unlike the name.
pub fn build_dob_body(
    event_id: Uuid,
    patient_id: Uuid,
    value: &str,
    kid: &str,
    hlc: Hlc,
) -> EventBody {
    EventBody {
        event_id: event_id.to_string(),
        patient_id: patient_id.to_string(),
        event_type: "demographic.field.asserted".into(),
        schema_version: DEMOGRAPHIC_FIELD_SCHEMA_VERSION.into(),
        hlc,
        t_effective: None,
        signer_key_id: kid.into(),
        contributors: serde_json::json!([{"actor_id": kid, "role": "recorded"}]),
        payload: dob_assertion_body(
            value,
            REGISTRATION_DOB_PRECISION,
            None,
            REGISTRATION_DEMOGRAPHIC_PROVENANCE,
        ),
        attachments: vec![],
        plaintext_twin: Some(render_dob_twin(
            value,
            REGISTRATION_DOB_PRECISION,
            REGISTRATION_DEMOGRAPHIC_PROVENANCE,
        )),
        clock_grade: ClockGrade::SelfAsserted,
    }
}

/// Register a standard patient: mint a UUID, derive the §5.8 search attestation from the
/// query and candidate list the clerk actually saw, and author the
/// `identity.registration.asserted` act PLUS a name and/or dob `demographic.field.asserted`
/// event for whatever was actually supplied — ALL through the real `submit_event` door,
/// inside ONE transaction, mirroring `register_john_doe` (a chart is never half-registered;
/// see the module doc's "#350 / Task 8b" section for why that means "never
/// registered-but-unfindable").
///
/// `query`/`displayed` are exactly what `patient::search::search_patients` produced and the
/// clerk was shown — `SearchAttestation::from_displayed` derives the attestation FROM them
/// (see `cairn_patient_search::attestation`'s module doc for why it must be derived, never
/// independently constructed).
///
/// `name` is the RAW name text as typed (see the module doc's "signature problem" for why
/// `SearchQuery` cannot supply this) — `None`, or blank after trimming, when nothing was
/// typed (e.g. identifier-only). No name event is authored in that case (principle 4). The
/// dob needs no separate parameter — `query.birth_date` already carries the raw ISO string,
/// and the identical "only if actually supplied" rule applies to it.
///
/// Returns the minted `patient_id`; care can proceed against it immediately, as with
/// `register_john_doe`.
pub async fn register_patient(
    client: &mut Client,
    sk: &SigningKey,
    kid: &str,
    node_origin: &str,
    name: Option<&str>,
    query: &SearchQuery,
    displayed: &CandidateList,
) -> anyhow::Result<Uuid> {
    let patient_id = Uuid::now_v7();
    let attestation = SearchAttestation::from_displayed(query, displayed);

    // Principle 4 at the boundary: blank-after-trim counts as "nothing supplied", same as
    // `None` (an empty `--name` CLI default must not assert an empty-string name). Filtering
    // HERE, once, means everything below can treat `Some` as a genuine value.
    let name = name.map(str::trim).filter(|n| !n.is_empty());
    let birth_date = query
        .birth_date
        .as_deref()
        .map(str::trim)
        .filter(|d| !d.is_empty());

    // Tick the HLC once per event actually being authored, in submission order: the
    // registration act FIRST (the chart's birth act — #345 is expected to require the FIRST
    // event on any patient_id to be a registration, no carve-out here either), then name,
    // then dob. These self-commit outside the transaction below; a rollback merely leaves a
    // monotonic HLC gap, which is fine (the same shape `register_john_doe`/`auto_apply` use).
    let h_registration = crate::db::next_hlc(client, node_origin).await?;
    let h_name = match name {
        Some(_) => Some(crate::db::next_hlc(client, node_origin).await?),
        None => None,
    };
    let h_dob = match birth_date {
        Some(_) => Some(crate::db::next_hlc(client, node_origin).await?),
        None => None,
    };

    let registration_body = build_registration_body(
        Uuid::now_v7(),
        patient_id,
        RegistrationClass::Standard,
        None,
        Some(&attestation),
        kid,
        h_registration,
    );
    let registration_signed = sign(&registration_body, sk)?;

    // `.zip` pairs each supplied value with its already-ticked HLC (one is `Some` iff the
    // other is); `.transpose()?` signs iff present, propagating a real signing error.
    let name_signed = name
        .zip(h_name)
        .map(|(n, h)| sign(&build_name_body(Uuid::now_v7(), patient_id, n, kid, h), sk))
        .transpose()?;
    let dob_signed = birth_date
        .zip(h_dob)
        .map(|(d, h)| sign(&build_dob_body(Uuid::now_v7(), patient_id, d, kid, h), sk))
        .transpose()?;

    // ONE transaction for every event this call authors — the #350 fix: a registration with
    // no matching demographic facts is exactly the half-registered state to avoid.
    let tx = client.transaction().await?;
    tx.execute(
        "SELECT submit_event($1)",
        &[&registration_signed.signed_bytes],
    )
    .await?;
    if let Some(s) = &name_signed {
        tx.execute("SELECT submit_event($1)", &[&s.signed_bytes])
            .await?;
    }
    if let Some(s) = &dob_signed {
        tx.execute("SELECT submit_event($1)", &[&s.signed_bytes])
            .await?;
    }
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

    // --- #350 / Task 8b: the name and dob demographic assertions ---

    #[test]
    fn name_body_asserts_field_name_with_registrar_entered_provenance() {
        let pid = Uuid::from_u128(1);
        let body = build_name_body(
            Uuid::from_u128(2),
            pid,
            "O'Brien-Smith, Jane",
            "kid",
            hlc(1),
        );
        assert_eq!(body.event_type, "demographic.field.asserted");
        assert_eq!(body.patient_id, pid.to_string());
        assert_eq!(body.payload["field"], "name");
        assert_eq!(body.payload["value"], "O'Brien-Smith, Jane");
        // Literal, not the imported constant: comparing against the constant would make this
        // assertion vacuous against a change to the constant's OWN value.
        assert_eq!(
            body.payload["provenance"], "registrar-entered",
            "must NOT be patient-stated — see REGISTRATION_DEMOGRAPHIC_PROVENANCE's doc for why"
        );
        assert!(
            body.payload.get("facets").is_none(),
            "no use category is claimed for a registration-desk name"
        );
        let twin = body.plaintext_twin.as_deref().unwrap();
        assert!(
            !twin.trim().is_empty(),
            "the demographic floor HARD-requires a non-empty twin"
        );
        assert!(twin.contains("O'Brien-Smith, Jane"));
    }

    #[test]
    fn dob_body_asserts_field_dob_with_day_precision_and_registrar_entered_provenance() {
        let pid = Uuid::from_u128(1);
        let body = build_dob_body(Uuid::from_u128(2), pid, "1980-01-01", "kid", hlc(1));
        assert_eq!(body.event_type, "demographic.field.asserted");
        assert_eq!(body.patient_id, pid.to_string());
        assert_eq!(body.payload["field"], "dob");
        assert_eq!(body.payload["value"], "1980-01-01");
        assert_eq!(body.payload["provenance"], "registrar-entered"); // literal — see above
        assert_eq!(
            body.payload["facets"]["precision"], "day",
            "SearchQuery.birth_date is documented as a full ISO YYYY-MM-DD"
        );
        let twin = body.plaintext_twin.as_deref().unwrap();
        assert!(!twin.trim().is_empty());
        assert!(twin.contains("1980-01-01"));
    }

    #[test]
    fn name_and_dob_bodies_are_recorded_only_no_responsibility_claimed() {
        let pid = Uuid::from_u128(1);
        for body in [
            build_name_body(Uuid::from_u128(2), pid, "Jane", "kid", hlc(1)),
            build_dob_body(Uuid::from_u128(2), pid, "1980-01-01", "kid", hlc(1)),
        ] {
            let c = &body.contributors[0];
            assert_eq!(c["role"], "recorded");
            assert!(
                c.get("responsibility").is_none(),
                "additive events demand no attestation"
            );
        }
    }
}
