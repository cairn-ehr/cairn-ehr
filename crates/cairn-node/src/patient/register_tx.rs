//! [`register_patient_in_tx`]: STANDARD registration on a transaction the CALLER owns (#909).
//!
//! The terminal service claims a registration request, re-runs the search, registers, and
//! settles the request in ONE transaction, so the registration cannot open or commit its own.
//! This is the body of `register::register_patient_returning_event` moved here verbatim (that
//! file is already past its size cap); it ticks every HLC and submits every event on the
//! caller's transaction and NEVER commits. Nothing here assumes it is the outermost
//! transaction, so a caller may run it on a savepoint and roll just that back.
//! Every precondition and rule of `register::register_patient` applies unchanged, above all
//! that `name` is the very string `query` was built from.

use super::register::{
    build_dob_body, build_identifier_body, build_name_body, build_registration_body, dob_precision,
    supplied_identifiers,
};
use crate::db_diagnosis::Committed;
use cairn_event::registration::RegistrationClass;
use cairn_event::{sign, SigningKey};
use cairn_patient_search::{CandidateList, SearchAttestation, SearchQuery};
use uuid::Uuid;

/// Register a chart on `tx` without committing it; the caller commits (or rolls back).
/// `kid` is the hex of the node's verifying key. Returns the new chart and its registration
/// event id.
pub async fn register_patient_in_tx(
    tx: &tokio_postgres::Transaction<'_>,
    sk: &SigningKey,
    kid: &str,
    node_origin: &str,
    name: Option<&str>,
    query: &SearchQuery,
    displayed: &CandidateList,
) -> anyhow::Result<Committed<Uuid>> {
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

    // Derive + validate the dob precision from its SHAPE up front — before ticking ANY HLC
    // or authoring the registration act (review round 1, #350, Important 1). A malformed
    // shape must refuse the WHOLE call with zero side effects, never partially proceed (tick
    // a clock, author a registration) and only then discover dob's turn fails. Bundling the
    // value with its derived precision here means every later use of `birth_date` already
    // carries an honest, validated precision — never re-derived, never re-guessed.
    let birth_date: Option<(&str, &str)> = birth_date
        .map(|d| dob_precision(d).map(|p| (d, p)))
        .transpose()?;

    // The identifiers to assert, decided BEFORE any HLC tick (same discipline as the dob
    // precision above) so the number of ticks matches the number of events exactly. See
    // `supplied_identifiers` for the blank/duplicate rules and for why the surviving values
    // are asserted TRIMMED (final review N3 — both sides trim, so a later search on the
    // same identifier matches bit-for-bit).
    let identifiers = supplied_identifiers(&query.identifiers);

    // Tick the HLC once per event actually being authored, in submission order: the
    // registration act FIRST (the chart's birth act — #345 is expected to require the FIRST
    // event on any patient_id to be a registration, no carve-out here either), then name,
    // then dob, then one per identifier. They run INSIDE the caller's transaction, as
    // `note::commit` does, so a rollback takes the ticks with it.
    let h_registration = crate::db::next_hlc(tx, node_origin).await?;
    let h_name = match name {
        Some(_) => Some(crate::db::next_hlc(tx, node_origin).await?),
        None => None,
    };
    let h_dob = match birth_date {
        Some(_) => Some(crate::db::next_hlc(tx, node_origin).await?),
        None => None,
    };
    let mut h_identifiers = Vec::with_capacity(identifiers.len());
    for _ in &identifiers {
        h_identifiers.push(crate::db::next_hlc(tx, node_origin).await?);
    }

    // Named, not inline: an interrupted commit below reports THIS event as the one whose
    // outcome is unknown (#815).
    let registration_event_id = Uuid::now_v7();
    let registration_body = build_registration_body(
        registration_event_id,
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
        .map(|((d, precision), h)| {
            sign(
                &build_dob_body(Uuid::now_v7(), patient_id, d, precision, kid, h),
                sk,
            )
        })
        .transpose()?;
    // One signed identifier event per supplied pair, paired with its already-ticked HLC.
    // `zip` is length-safe by construction (`h_identifiers` was built by iterating the same
    // vec), and `collect::<Result<_,_>>()` propagates a real signing error rather than
    // silently authoring a short list.
    let identifier_signed = identifiers
        .iter()
        .zip(h_identifiers)
        .map(|((system, value), h)| {
            sign(
                &build_identifier_body(Uuid::now_v7(), patient_id, system, value, kid, h),
                sk,
            )
        })
        .collect::<Result<Vec<_>, _>>()?;

    // ONE transaction for every event this call authors — the #350 fix: a registration with
    // no matching demographic facts is exactly the half-registered state to avoid. The
    // identifiers (final review, C1) are part of that same rule: a registration that searched
    // on an MRN and then did not record it is unfindable on the highest-precision pass.
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
    for s in &identifier_signed {
        tx.execute("SELECT submit_event($1)", &[&s.signed_bytes])
            .await?;
    }

    Ok(Committed {
        value: patient_id,
        event_id: registration_event_id,
    })
}
