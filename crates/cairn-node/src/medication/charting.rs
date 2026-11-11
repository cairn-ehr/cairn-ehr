//! Charting a medication (charting slice 1a): sign one line with its charting fields, and read
//! those fields back.
//!
//! [`chart_medication`] is the medication assertion's orchestrator; `assert_medication` is it
//! with no charting. The fields themselves (charted-as, route, strength, a combination's
//! components, the warning override) are cairn-event's [`Charting`], written into the signed
//! payload by [`build_charted_body`]. db/077's floor judges them and its projection
//! (`medication_charting`) keeps them per thread; [`read_charting`] reads that projection.
use super::assert::{build_charted_body, validate_term, AssertMedicationInput};
use anyhow::Context;
use cairn_event::medication::Charting;
use cairn_event::SigningKey;
use serde_json::Value;
use std::collections::HashMap;
use tokio_postgres::GenericClient;
use uuid::Uuid;

/// Record a medication line, with its charting fields. Mints and returns the thread's
/// `medication_id` beside the assert event's id (#828).
///
/// FLOW: refuse a blank term → mint the HLC and ids → look up the §5.9 safety class (a single
/// coding only) → build the clear body → seal, sign and submit through the strict door
/// (`seal_sign_submit`, which also applies `author` and runs `attest` atomically).
///
/// `author`: `Some` signs as that clinician (ADR-0053); `None` is device-additive. `attest`:
/// `Some` vouches for the new thread in the same transaction as the assert (slice 4) — kept
/// here so `assert_medication`'s attest path is this one path too.
#[allow(clippy::too_many_arguments)] // signer + node context + patient/input/charting/author/attest
pub async fn chart_medication(
    client: &mut tokio_postgres::Client,
    node_sk: &SigningKey,
    node_kid: &str,
    node_origin: &str,
    patient: Uuid,
    input: &AssertMedicationInput<'_>,
    charting: &Charting<'_>,
    author: Option<&crate::medication::AuthorParams<'_>>,
    attest: Option<&crate::medication::AttestParams<'_>>,
) -> anyhow::Result<crate::db_diagnosis::Committed<Uuid>> {
    validate_term(input.term)?;
    // Mint HLCs up front (self-committing; a rolled-back submit just leaves a gap —
    // the HLC is monotonic and gaps are allowed, exactly like identify_patient).
    let verb_hlc = crate::db::next_hlc(client, node_origin).await?;
    let event_id = Uuid::now_v7();
    let medication_id = Uuid::now_v7();
    // §5.9 part B (ADR-0063): the class is looked up on the CODING node, PRE-SEAL, because
    // it is a drug-knowledge lookup a reader cannot redo without a drug database — and
    // making the safety floor depend on holding one would defeat the floor (#294 /
    // ADR-0059 decision 4). An uncoded medication, or a coding this deployment's map has no
    // row for, yields None: both are honest absences and emit no signal at all, rather than
    // a content-free marker that would manufacture a warning from nothing.
    //
    // A COMBINATION yields None too: one class cannot speak for several drugs, and a claim
    // for one part would read as the whole line's. Absent is the withholding direction, as
    // for an uncoded line (ADR-0063). (The floor already refuses a combination beside a
    // single coding; this does not rely on it.)
    //
    // Held in a local `class` first because `PreciseSafety` BORROWS its two strings; the
    // owned pair must outlive the body construction below.
    //
    // A lookup that ERRORS is treated as "no class", never propagated: an advisory field
    // must never be able to fail a clinical write (ADR-0060 — the system may fail to record
    // an order, but it may never cancel one). Falling back is the WITHHOLDING direction, so
    // the worst case is a missing decoration on a recorded medication, never a lost
    // medication. See `crate::safety::advisory_or_withheld` for the full argument.
    let single_coding = input.coding.filter(|_| charting.components.is_empty());
    let class = match single_coding.as_ref() {
        Some(coding) => crate::safety::advisory_or_withheld(
            crate::safety::lookup_class(client, coding).await,
            None,
            "the safety class lookup for a medication assert",
        ),
        None => None,
    };
    let safety = class
        .as_ref()
        .map(|(class, severity)| cairn_event::safety::PreciseSafety { class, severity });
    let body = build_charted_body(
        event_id,
        medication_id,
        patient,
        input,
        charting,
        node_kid,
        verb_hlc,
        safety,
    );
    // ADR-0052 seal-at-write: the clear body is sealed, signed, and submitted through the
    // ONE strict door by seal_sign_submit — which also runs the atomic author-time
    // attestation when `attest` is Some (it vouches for the thread named in the body's
    // payload.medication_id), and rewrites the body to human-signed authorship when
    // `author` is Some (#204 / ADR-0053; the node still keeps DEK custody). It returns the
    // content event's id, the one its lost commit names; the thread id rides beside it.
    let recorded =
        crate::medication::sealed_submit::seal_sign_submit(client, node_sk, body, author, attest)
            .await?;
    Ok(crate::db_diagnosis::Committed {
        value: medication_id,
        event_id: recorded,
    })
}

/// One component of a charted combination: the term as typed, and its coding as
/// `(system, code, display)` when it was picked.
pub type ChartedComponent = (String, Option<(String, String, String)>);

/// A thread's charting, as the projection holds it. Every field is an honest `None`/empty when
/// it was not charted. `charted_as` stays text: a peer may speak a newer word (ADR-0056).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChartingRow {
    pub charted_as: Option<String>,
    pub route: Option<String>,
    pub strength: Option<String>,
    /// Empty when the line is not a combination.
    pub components: Vec<ChartedComponent>,
    pub override_reason: Option<String>,
}

/// The components of a signed combination (`substance.components`). Pure. Both doors refuse an
/// element that is not `{term, coding|null}`, so the fallbacks here (an empty term, a coding
/// read as unpicked) only keep one odd row from failing a whole list read.
fn components_of(v: &Value) -> Vec<ChartedComponent> {
    let text = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).map(str::to_owned);
    let Some(parts) = v.as_array() else {
        return Vec::new();
    };
    parts
        .iter()
        .map(|p| {
            let coding = p
                .get("coding")
                .and_then(|c| Some((text(c, "system")?, text(c, "code")?, text(c, "display")?)));
            (text(p, "term").unwrap_or_default(), coding)
        })
        .collect()
}

/// The charting rows of `medication_ids`, keyed by thread. A thread charted without any
/// charting field (or not known here) has no row and no key.
pub async fn read_charting(
    tx: &impl GenericClient,
    medication_ids: &[Uuid],
) -> anyhow::Result<HashMap<Uuid, ChartingRow>> {
    let ids: Vec<String> = medication_ids.iter().map(Uuid::to_string).collect();
    let rows = tx
        .query(
            "SELECT medication_id::text, charted_as, route, strength, components::text, \
                    override_reason \
               FROM medication_charting WHERE medication_id = ANY($1::text[]::uuid[])",
            &[&ids],
        )
        .await
        .context("reading the medication charting rows")?;
    rows.iter()
        .map(|r| {
            let components: Option<String> = r.try_get(4)?;
            let components = match components {
                Some(json) => components_of(&serde_json::from_str(&json)?),
                None => Vec::new(),
            };
            Ok((
                r.try_get::<_, String>(0)?.parse()?,
                ChartingRow {
                    charted_as: r.try_get(1)?,
                    route: r.try_get(2)?,
                    strength: r.try_get(3)?,
                    components,
                    override_reason: r.try_get(5)?,
                },
            ))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn components_read_picked_and_unpicked_parts() {
        let v = json!([
            {"term": "amoxicillin", "coding": {"system": "s", "code": "c", "display": "d"}},
            {"term": "clavulanic acid", "coding": null},
        ]);
        assert_eq!(
            components_of(&v),
            vec![
                (
                    "amoxicillin".to_string(),
                    Some(("s".into(), "c".into(), "d".into()))
                ),
                ("clavulanic acid".to_string(), None),
            ]
        );
        assert!(components_of(&json!(null)).is_empty());
    }
}
