//! Batch charting slice 1: the node's side of the per-writer medication draft list (db/078).
//!
//! The list is mutable, unsigned scratch space beside the record. This file READS it
//! ([`read_draft_lines`], [`writer_drafts`]) and signs ONE line out of it ([`chart_draft_line`]):
//! the line's event enters the record and its draft row leaves the list in the same transaction,
//! so a line is never in both places and never in neither. The window's list commands also write
//! through here ([`save_draft_line`], [`delete_draft_line`], [`take_draft_lines`]) and ask the
//! vault the two things the list's Sign checks its writer by ([`staff_id`], [`writer_key`]).
use super::assert::AssertMedicationInput;
use super::charting::chart_body;
use super::draft_versions_sign::{prepare_version, spend_row, submit, VersionToSign};
use super::sealed_submit::{prepare_sealed, AuthorParams};
use anyhow::Context;
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
    /// The clinician unticked a line the default tick would give (Task 6 review C1): their veto,
    /// stored so the line never comes back ticked. Cleared by the door when the words change or
    /// a hand tick is given.
    pub held: bool,
    /// The line's "From:" category (`patient-reported`, `clinician-observed`, `external-record`,
    /// `unknown`) and the free-text detail after it; `None` = not given.
    pub info_source: Option<String>,
    pub source_detail: Option<String>,
    /// Why the clinician is charting it, as typed.
    pub why: Option<String>,
    /// `line:<uuid>` (a draft line) or `med:<uuid>` (a charted medication) this line is a
    /// version of; `None` = not set aside. See `draft_versions`.
    pub set_aside_of: Option<String>,
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
                    hand_ticked, ticked_against::text, finished, saved_at::text, typed_drug, directions, held, \
                    info_source, source_detail, why, set_aside_of \
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
                held: r.try_get(13)?,
                info_source: r.try_get(14)?,
                source_detail: r.try_get(15)?,
                why: r.try_get(16)?,
                set_aside_of: r.try_get(17)?,
            })
        })
        .collect()
}

/// Save `line` as `writer`'s on `patient` (insert or update): the door's one word ('saved',
/// 'moved', 'elsewhere', 'no-chart', 'no-such-writer', 'not-yours', 'deleted'). `line.terminal_id`
/// is the terminal saving it; `seq` and `saved_at` are the door's own and are not sent. The door
/// decides what a tick keeps (db/078): send the refs of the moment every time.
pub async fn save_draft_line(
    client: &(impl GenericClient + Sync),
    patient: Uuid,
    writer: Option<&str>,
    line: &DraftLine,
) -> anyhow::Result<String> {
    let against: Vec<serde_json::Value> = line
        .ticked_against
        .iter()
        .map(|(source, r)| serde_json::json!({"source": source, "ref": r.to_string()}))
        .collect();
    let against = serde_json::Value::Array(against).to_string();
    let parts = line.parts.to_string();
    let row = client
        .query_one(
            "SELECT cairn_med_draft_line_save($1::text::uuid, $2::text::uuid, $3, $4, $5, $6, \
                    $7::text::jsonb, $8, $9, $10::text::jsonb, $11, $12, $13, $14, $15, $16, $17, $18)",
            &[
                &line.line_id.to_string(),
                &patient.to_string(),
                &line.terminal_id,
                &writer,
                &line.charted_as,
                &line.line_text,
                &parts,
                &line.reason,
                &line.hand_ticked,
                &against,
                &line.finished,
                &line.typed_drug,
                &line.directions,
                &line.held,
                &line.info_source,
                &line.source_detail,
                &line.why,
                &line.set_aside_of,
            ],
        )
        .await
        .context("saving the medication draft line")?;
    Ok(row.try_get(0)?)
}

/// Save the writer's "From:" for `patient` (slice 2a): the preset LABEL (db/078 holds it to the
/// six labels, ruling R6) and the free-text detail, replacing what was there. The door's word.
pub async fn save_draft_from(
    client: &(impl GenericClient + Sync),
    patient: Uuid,
    writer: Option<&str>,
    preset: Option<&str>,
    detail: Option<&str>,
) -> anyhow::Result<String> {
    let row = client
        .query_one(
            "SELECT cairn_med_draft_from_save($1::text::uuid, $2, $3, $4)",
            &[&patient.to_string(), &writer, &preset, &detail],
        )
        .await
        .context("saving the medication list's From:")?;
    Ok(row.try_get(0)?)
}

/// The writer's "From:" for `patient` as `(preset, detail)`, or `None` when never set.
pub async fn read_draft_from(
    client: &(impl GenericClient + Sync),
    patient: Uuid,
    writer: Option<&str>,
) -> anyhow::Result<Option<(Option<String>, Option<String>)>> {
    let row = client
        .query_opt(
            "SELECT preset, detail FROM cairn_med_draft_from($1::text::uuid, $2)",
            &[&patient.to_string(), &writer],
        )
        .await
        .context("reading the medication list's From:")?;
    row.map(|r| Ok((r.try_get(0)?, r.try_get(1)?))).transpose()
}

/// Delete one line the clinician removed, as last read at `saved_at`: the door's word ('deleted',
/// 'gone', 'moved', 'elsewhere', 'not-yours', 'changed'). No record is kept (design §9 Q3).
pub async fn delete_draft_line(
    client: &(impl GenericClient + Sync),
    line: Uuid,
    patient: Uuid,
    terminal: &str,
    writer: Option<&str>,
    saved_at: &str,
) -> anyhow::Result<String> {
    let row = client
        .query_one(
            "SELECT cairn_med_draft_line_delete($1::text::uuid, $2::text::uuid, $3, $4, $5::text::timestamptz)",
            &[&line.to_string(), &patient.to_string(), &terminal, &writer, &saved_at],
        )
        .await
        .context("deleting the medication draft line")?;
    Ok(row.try_get(0)?)
}

/// "Continue here": move the writer's whole list on `patient` to `terminal`; how many lines moved.
pub async fn take_draft_lines(
    client: &(impl GenericClient + Sync),
    patient: Uuid,
    writer: Option<&str>,
    terminal: &str,
) -> anyhow::Result<i32> {
    let row = client
        .query_one(
            "SELECT cairn_med_draft_take($1::text::uuid, $2, $3)",
            &[&patient.to_string(), &writer, &terminal],
        )
        .await
        .context("moving the medication draft list to this terminal")?;
    Ok(row.try_get(0)?)
}

/// `raw` normalised as the vault normalises a staff id (`cairn_vault_staff_id`): the rule lives in
/// SQL alone, never in a copy here (R7). `None` for a blank id.
pub async fn staff_id(
    client: &(impl GenericClient + Sync),
    raw: &str,
) -> anyhow::Result<Option<String>> {
    let row = client
        .query_one("SELECT cairn_vault_staff_id($1)", &[&raw])
        .await
        .context("normalising a staff id")?;
    Ok(row.try_get(0)?)
}

/// The CURRENT signing key id of vault member `staff` (`cairn_vault_member_key`), `None` for no
/// such member. The list's Sign compares the key it was handed with this one: by KEY, never by
/// staff id, since the key is what the signature proves (ruling R-B, `note::sign`).
pub async fn writer_key(
    client: &(impl GenericClient + Sync),
    staff: &str,
) -> anyhow::Result<Option<String>> {
    let row = client
        .query_one("SELECT cairn_vault_member_key($1)", &[&staff])
        .await
        .context("reading the writer's signing key")?;
    Ok(row.try_get(0)?)
}

/// Why [`chart_draft_line`] wrote nothing because the LIST moved under it: the spend answered
/// `changed`, `moved`, `gone`, … instead of `deleted`. Typed, so a caller can tell this ("read the
/// list again") from a failure of the node, without matching on words.
#[derive(Debug)]
pub struct SpendRefused(pub String);

impl std::fmt::Display for SpendRefused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "the draft line was not signed: the list answered {:?} (re-read the list)",
            self.0
        )
    }
}

impl std::error::Error for SpendRefused {}

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

/// Chart one draft line, and any versions set aside beside it: submit their events and delete
/// their draft rows in ONE transaction, so each line leaves the list exactly when it enters the
/// record. Every delete must answer `'deleted'`, else the transaction rolls back (the line moved,
/// went, or was SAVED AGAIN since the caller read it at `saved_at`: the caller re-reads).
///
/// FLOW: build and seal the companion (`chart_body` + `prepare_sealed`) and each version
/// (`prepare_version`, `alternative_to` = the companion's new `medication_id`), nothing written →
/// open a transaction → `submit_event` the companion FIRST, then each version (the floor wants the
/// companion projected and current) → spend every VERSION's row, then the companion's → commit.
///
/// The spends go the other way round (ruling R5): db/078's delete door, deleting a line, unmarks
/// and re-stamps every version still naming it, so spending the companion first would make each
/// version's spend answer `changed` and roll everything back. A lost commit is marked
/// outcome-unknown (#814) exactly as `seal_sign_submit`'s attested arm does.
///
/// Returns the companion's thread and the versions' ids; `event_id` is the companion's.
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
    versions: &[VersionToSign<'_>],
) -> anyhow::Result<crate::db_diagnosis::Committed<(Uuid, Vec<Uuid>)>> {
    let (body, medication_id) =
        chart_body(client, node_kid, node_origin, patient, input, charting).await?;
    let companion = prepare_sealed(&*client, node_sk, body, Some(author)).await?;
    let event_id = companion.event_id;
    let mut prepared_versions = Vec::with_capacity(versions.len());
    for v in versions {
        prepared_versions.push(
            prepare_version(
                client,
                node_sk,
                node_kid,
                node_origin,
                patient,
                v,
                medication_id,
                author,
            )
            .await?,
        );
    }

    let tx = client.transaction().await?;
    submit(&tx, &companion).await?;
    for (p, _) in &prepared_versions {
        submit(&tx, p).await?;
    }
    // Versions' rows first, the companion's last (ruling R5, above).
    for v in versions {
        spend_row(&tx, v.line, patient, terminal, writer, v.saved_at).await?;
    }
    spend_row(&tx, line, patient, terminal, writer, saved_at).await?;
    tx.commit()
        .await
        .map_err(|e| crate::db_diagnosis::commit_failure(event_id, e.into()))?;
    Ok(crate::db_diagnosis::Committed {
        value: (
            medication_id,
            prepared_versions.iter().map(|(_, id)| *id).collect(),
        ),
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
