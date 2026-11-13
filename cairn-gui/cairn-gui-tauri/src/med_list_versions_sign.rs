//! Signing a set-aside version (batch charting slice 2a), moved VERBATIM out of
//! `med_list_versions.rs` to keep that file under 500 lines; it re-exports both, so callers are
//! unedited. [`version_to_sign`] rebuilds what a version signs from its STORED row;
//! [`sign_alone`] signs one beside an already-charted line, in its own transaction.
use crate::funnel::view::ErrorView;
use crate::med_chart::press;
use crate::med_line_sign::single_coding;
use crate::med_list::Judged;
use crate::med_list_versions::list_input;
use crate::state::AppState;
use cairn_event::medication::{Charting, MedicationAssertion};
use cairn_node::medication::draft_versions_sign::{set_aside_draft_line, VersionToSign};
use cairn_node::medication::AuthorParams;
use uuid::Uuid;

/// What a version signs, rebuilt from its STORED row: the drug fields exactly as a charted line
/// spells them ([`list_input`]), with its route and strength, but no start, no press, no
/// override and no safety claim (a version is never current). Its source is the line's own
/// (stamped by "From:" or chosen), else `unknown`: never the press's default, which names who
/// reported a CHARTED line and says nothing about a figure the clinician set aside (ADR-0102
/// item 4, principle 4). Pure.
pub(crate) fn version_to_sign(v: &Judged) -> Result<VersionToSign<'_>, ErrorView> {
    let (term, parts) = v.signable.as_ref().map_err(Clone::clone)?;
    let i = list_input(
        v,
        term,
        single_coding(parts),
        press(&v.line.charted_as)?,
        "",
    );
    Ok(VersionToSign {
        line: v.line.line_id,
        saved_at: &v.line.saved_at,
        drug: MedicationAssertion {
            medication_id: "",
            term: i.term,
            coding: i.coding,
            formulation: i.formulation,
            dose_amount: i.dose_amount,
            dose_unit: i.dose_unit,
            sig: i.sig,
            info_source: v.line.info_source.as_deref().unwrap_or("unknown"),
            started: None,
            started_precision: None,
            safety: None,
            charting: Some(Charting {
                route: v.read.route.as_deref(),
                strength: v.read.strength.as_deref(),
                ..Charting::default()
            }),
            source_note: i.source_note,
        },
    })
}

/// Sign one version beside the charted line `med`, alone: its event and its spend in one
/// transaction (`set_aside_draft_line`). The floor refuses a line stopped since; it rolls back.
/// `list` is the list's identity: `(chart, terminal, writer)`.
pub(crate) async fn sign_alone(
    state: &AppState,
    node_sk: &cairn_event::SigningKey,
    (patient, terminal, writer): (Uuid, &str, Option<&str>),
    v: &Judged,
    med: Uuid,
    author: &AuthorParams<'_>,
) -> anyhow::Result<()> {
    let version = version_to_sign(v).map_err(|e| anyhow::anyhow!("{}", e.text))?;
    let node_kid = hex::encode(node_sk.verifying_key().to_bytes());
    let mut db = state
        .db
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("no database connection"))?
        .lock()
        .await?;
    set_aside_draft_line(
        &mut db,
        node_sk,
        &node_kid,
        &state.node_origin,
        patient,
        terminal,
        writer,
        &version,
        med,
        author,
    )
    .await
    .map(|_| ())
    .map_err(|e| db.explain(e))
}
