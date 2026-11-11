//! Batch charting slice 1: the node's side of the per-writer medication draft list (db/078).
//!
//! The list is mutable, unsigned scratch space beside the record. This file READS it
//! ([`read_draft_lines`], [`writer_drafts`]) and signs ONE line out of it ([`chart_draft_line`]):
//! the line's event enters the record and its draft row leaves the list in the same transaction,
//! so a line is never in both places and never in neither.
use super::assert::AssertMedicationInput;
use super::charting::chart_body;
use super::sealed_submit::{prepare_sealed, AuthorParams, Prepared};
use anyhow::{bail, Context};
use cairn_event::medication::Charting;
use cairn_event::SigningKey;
use chrono::{DateTime, Utc};
use tokio_postgres::GenericClient;
use uuid::Uuid;

/// One saved line, as `cairn_med_draft_lines` returns it.
#[derive(Debug, Clone, PartialEq)]
pub struct DraftLine {
    pub line_id: Uuid,
    pub terminal_id: String,
    pub seq: i32,
    pub charted_as: String,
    pub line_text: String,
    pub parts: serde_json::Value,
    pub reason: Option<String>,
    pub hand_ticked: bool,
    /// The warnings the clinician ticked against: `(source, ref)`.
    pub ticked_against: Vec<(String, Uuid)>,
    pub finished: bool,
    /// When the row was last saved, as the database's own text (microseconds and zone intact).
    /// Hand it back to [`chart_draft_line`] so the spend only takes the content that was read.
    pub saved_at: String,
    /// The drug as the clinician typed it, and the directions after it: the window splits the
    /// line, and Sign rebuilds what it signs from these stored halves, never from the webview.
    pub typed_drug: String,
    pub directions: String,
}

/// `[{"source": "...", "ref": "<uuid>"}]` to pairs, or `None` when ANY element is malformed.
/// Pure. The floor (db/078) refuses a malformed tick on new rows, so this only meets old rows.
/// Callers treat `None` as "no usable tick": fewer refs can only make `hand_tick_holds` (which
/// needs every fresh warning listed) fail, so the line shows unticked and wants a fresh tick.
/// That fails closed, and one line's scratch data never blocks the whole list.
fn parse_ticked_against(v: &serde_json::Value) -> Option<Vec<(String, Uuid)>> {
    v.as_array()?
        .iter()
        .map(|i| {
            let source = i.get("source")?.as_str()?;
            let r = i.get("ref")?.as_str()?.parse::<Uuid>().ok()?;
            Some((source.to_owned(), r))
        })
        .collect()
}

/// The writer's saved lines on one chart, in list order.
pub async fn read_draft_lines(
    client: &(impl GenericClient + Sync),
    patient: Uuid,
    writer: Option<&str>,
) -> anyhow::Result<Vec<DraftLine>> {
    // tokio-postgres here has no uuid/json codecs: ids and jsonb cross as text.
    let rows = client
        .query(
            "SELECT line_id::text, terminal_id, seq, charted_as, line_text, parts::text, reason, \
                    hand_ticked, ticked_against::text, finished, saved_at::text, typed_drug, directions \
               FROM cairn_med_draft_lines($1::text::uuid, $2)",
            &[&patient.to_string(), &writer],
        )
        .await
        .context("reading the medication draft list")?;
    rows.iter()
        .map(|r| {
            let against: serde_json::Value = serde_json::from_str(&r.try_get::<_, String>(8)?)?;
            // A malformed tick reads as no tick at all (see `parse_ticked_against`).
            let (hand_ticked, ticked_against) = match parse_ticked_against(&against) {
                Some(t) => (r.try_get(7)?, t),
                None => (false, Vec::new()),
            };
            Ok(DraftLine {
                line_id: r.try_get::<_, String>(0)?.parse()?,
                terminal_id: r.try_get(1)?,
                seq: r.try_get(2)?,
                charted_as: r.try_get(3)?,
                line_text: r.try_get(4)?,
                parts: serde_json::from_str(&r.try_get::<_, String>(5)?)?,
                reason: r.try_get(6)?,
                hand_ticked,
                ticked_against,
                finished: r.try_get(9)?,
                saved_at: r.try_get(10)?,
                typed_drug: r.try_get(11)?,
                directions: r.try_get(12)?,
            })
        })
        .collect()
}

/// Who has unsigned lines where: `(writer, patient, lines, newest)`.
pub async fn writer_drafts(
    client: &(impl GenericClient + Sync),
) -> anyhow::Result<Vec<(String, Uuid, i64, DateTime<Utc>)>> {
    let rows = client
        .query(
            "SELECT writer, patient_id::text, lines, newest FROM cairn_med_drafts_by_writer()",
            &[],
        )
        .await
        .context("reading the drafts by writer")?;
    rows.iter()
        .map(|r| {
            Ok((
                r.try_get(0)?,
                r.try_get::<_, String>(1)?.parse()?,
                r.try_get(2)?,
                r.try_get(3)?,
            ))
        })
        .collect()
}

/// Chart one draft line: submit its event and delete its draft row in ONE transaction, so the
/// line leaves the list exactly when it enters the record. The delete must answer `'deleted'`,
/// else the transaction rolls back (the line moved, went, or was SAVED AGAIN since the caller
/// read it at `saved_at`: the caller re-reads).
///
/// FLOW: build and seal the body (`chart_body` + `prepare_sealed`, nothing written) → open a
/// transaction → `submit_event` → delete the draft row → commit. A lost commit is marked
/// outcome-unknown (#814) exactly as `seal_sign_submit`'s attested arm does.
#[allow(clippy::too_many_arguments)] // signer + node context + the line's identity + its content
pub async fn chart_draft_line(
    client: &mut tokio_postgres::Client,
    node_sk: &SigningKey,
    node_kid: &str,
    node_origin: &str,
    patient: Uuid,
    terminal: &str,
    writer: Option<&str>,
    line: Uuid,
    saved_at: &str,
    input: &AssertMedicationInput<'_>,
    charting: &Charting<'_>,
    author: &AuthorParams<'_>,
) -> anyhow::Result<crate::db_diagnosis::Committed<Uuid>> {
    let (body, medication_id) =
        chart_body(client, node_kid, node_origin, patient, input, charting).await?;
    let Prepared {
        event_id,
        signed_bytes,
        dek,
    } = prepare_sealed(&*client, node_sk, body, Some(author)).await?;

    let tx = client.transaction().await?;
    tx.execute(
        "SELECT submit_event($1, NULL, NULL, $2)",
        &[&signed_bytes, &dek.as_bytes().as_slice()],
    )
    .await?;
    let answer: String = tx
        .query_one(
            "SELECT cairn_med_draft_line_delete($1::text::uuid, $2::text::uuid, $3, $4, $5::text::timestamptz)",
            &[&line.to_string(), &patient.to_string(), &terminal, &writer, &saved_at],
        )
        .await
        .context("spending the draft line (nothing was written)")?
        .get(0);
    if answer != "deleted" {
        // Returning drops `tx`, which rolls back: nothing is written, and the clear error stays.
        bail!("the draft line was not signed: the list answered {answer:?} (re-read the list)");
    }
    tx.commit()
        .await
        .map_err(|e| crate::db_diagnosis::commit_failure(event_id, e.into()))?;
    Ok(crate::db_diagnosis::Committed {
        value: medication_id,
        event_id,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn ticked_against_parses_and_a_malformed_one_is_no_tick() {
        let id = Uuid::now_v7();
        let ok = parse_ticked_against(&json!([{"source": "allergy", "ref": id.to_string()}]));
        assert_eq!(ok, Some(vec![("allergy".to_string(), id)]));
        assert_eq!(parse_ticked_against(&json!([])), Some(vec![]));
        assert_eq!(
            parse_ticked_against(&json!([{"source": "allergy", "ref": "r1"}])),
            None
        );
        assert_eq!(
            parse_ticked_against(&json!([{"ref": id.to_string()}])),
            None
        );
        assert_eq!(parse_ticked_against(&json!({})), None);
    }
}
