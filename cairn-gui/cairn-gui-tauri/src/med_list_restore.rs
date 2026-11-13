//! Batch charting slice 2b in the window: "Use this version instead" (design §3.3).
//!
//! A reviewing clinician who believes a SET-ASIDE version over the line it was set aside beside
//! (its companion) restores it. A restore is never an edit of the version: it is a stop of what
//! stands for the companion plus a NEW line (ADR-0102 Consequences). So the press
//! ([`med_list_restore_open`]) only adds a list line, pre-filled from the version
//! ([`restore_prefill`]) and carrying `restores` (db/078's restore door); the clinician may edit
//! it like any line, and the list's one Sign charts it. Nothing is signed by the press.
//!
//! At Sign (`med_list_sign.rs`), from the STORED row on the fresh read, never the webview's word:
//! - [`read_versions`] reads each restore line's version on the OPENED chart (never a linked
//!   chart's, spec §2), with what currently stands for its companion (`standing`, db/080's
//!   lineage function, rulings R3 + R4);
//! - [`restore_to_sign`] turns that into the stops chart_draft_line signs in the line's own
//!   transaction, or refuses the line alone ([`RESTORE_VERSION_GONE`], Review Focus 4);
//! - [`restore_source`]: a restore line with no source of its own signs `unknown`.
//!
//! On the list, a restore line is judged a duplicate against the chart WITHOUT the lines it will
//! stop ([`besides`], Review Focus 2), but still against every other line, list lines included
//! (two restores of one companion duplicate each other, Review Focus 3).
use crate::allergy_check::CodingArg;
use crate::funnel::view::ErrorView;
use crate::link::judgement_view::refused;
use crate::med_chart::PartArg;
use crate::med_list::FIXTURE;
use crate::med_list::{claim_writer, med_list_read_impl, Judged, ListLineView, MedListView};
use crate::med_list_judge::current_lines;
use crate::notes::terminal_of;
use crate::state::AppState;
use crate::write_failure::{node_cannot_write, node_failure};
use cairn_medication_view::MedicationRow;
use cairn_node::medication::draft::{self, DraftLine, RestoreContent};
use cairn_node::medication::line::read_directions;
use cairn_node::medication::version_settle::RestoreToSign;
use cairn_node::medication::version_settle_read::version_for_restore;
pub(crate) use cairn_node::medication::version_settle_read::VersionForRestore;
use std::collections::BTreeMap;
use tokio_postgres::GenericClient;
use uuid::Uuid;

/// The press or the check named a version that is not on the opened chart's read (gone since the
/// pane was drawn, or a linked chart's: spec §2 offers neither).
pub const VERSION_GONE: &str = "That version is no longer on this chart; read it again";

/// A restore line's per-line Sign refusal: its version is no longer on the opened chart.
pub const RESTORE_VERSION_GONE: &str = "the version it restores can no longer be read";

/// The list is held by another terminal: the pane's own words for it (`med_list.js`'s
/// `MED_LIST_ELSEWHERE`), followed by that terminal.
pub const LIST_ELSEWHERE: &str = "These lines are open on another terminal: ";

/// The opened chart's versions the list's restore lines name, by version id (a version not on
/// it is absent).
pub(crate) type Versions = BTreeMap<Uuid, VersionForRestore>;

/// What the press answers: the list read again, and the new line, for the editor to open on.
#[derive(Debug, serde::Serialize)]
pub struct RestoreOpened {
    pub list: MedListView,
    pub line_id: String,
}

#[tauri::command]
pub async fn med_list_restore_open(
    state: tauri::State<'_, AppState>,
    patient_id: String,
    charts: Vec<String>,
    version_id: String,
) -> Result<RestoreOpened, ErrorView> {
    med_list_restore_open_impl(&state, &patient_id, charts, &version_id).await
}

/// "Use this version instead": add a restore line of `version_id` to who's here's list on the
/// chart on screen, then read the list over `charts`. Refused, with no row written: fixture mode,
/// a version not on the OPENED chart ([`VERSION_GONE`]), a list held on another terminal.
pub async fn med_list_restore_open_impl(
    state: &AppState,
    patient_id: &str,
    charts: Vec<String>,
    version_id: &str,
) -> Result<RestoreOpened, ErrorView> {
    let patient = state.displayed_patient(patient_id).await.map_err(refused)?;
    if state.is_mock() {
        return Err(refused(FIXTURE));
    }
    let terminal = terminal_of(state)?;
    let version: Uuid = version_id.parse().map_err(|_| refused(VERSION_GONE))?;
    let (writer, _) = claim_writer(state).await;
    let line = Uuid::now_v7();
    {
        let db = state
            .db
            .as_ref()
            .ok_or_else(|| node_cannot_write("no database connection"))?
            .lock()
            .await?;
        let failed = |e: anyhow::Error| node_failure(&db.explain(e));
        let lines = draft::read_draft_lines(&*db, patient, writer.as_deref())
            .await
            .map_err(failed)?;
        if let Some(l) = lines.iter().find(|l| l.terminal_id != terminal) {
            return Err(refused(format!("{LIST_ELSEWHERE}{}", l.terminal_id)));
        }
        let v = version_for_restore(&*db, patient, version)
            .await
            .map_err(failed)?
            .ok_or_else(|| refused(VERSION_GONE))?;
        let (typed_drug, directions, line_text, parts) = restore_prefill(&v);
        let content = RestoreContent {
            charted_as: restore_charted_as(&v),
            line_text: &line_text,
            typed_drug: &typed_drug,
            directions: &directions,
            parts: &parts,
        };
        let answer = draft::add_restore_line(
            &*db,
            line,
            patient,
            terminal,
            writer.as_deref(),
            version,
            &content,
        )
        .await
        .map_err(failed)?;
        match answer.as_str() {
            "saved" => {}
            "no-such-version" => return Err(refused(VERSION_GONE)),
            other => return Err(refused(format!("The line was not added: {other}"))),
        }
    }
    Ok(RestoreOpened {
        list: med_list_read_impl(state, patient_id, Some(&charts)).await?,
        line_id: line.to_string(),
    })
}

/// The restore line's prefill `(typed_drug, directions, line_text, parts)`, read back by Sign
/// through `read_directions` as the version's dose, route and sig. The version's `sig` is its
/// directions as typed (`charted_input`), which in this window already carry the dose and route,
/// so it is taken whole; only what it does not say is put before it, from the version's own
/// fields (a peer's version may keep them apart). Doubling them would read as two doses, which
/// `read_directions` refuses to choose between. The drug is the version's term alone: the
/// window splits a line at the drug, so a strength belongs to the directions, never the term.
/// Pure.
pub(crate) fn restore_prefill(
    v: &VersionForRestore,
) -> (String, String, String, serde_json::Value) {
    let sig = v.sig.as_deref().unwrap_or("").trim();
    let said = read_directions(sig);
    let dose = match (&v.dose_amount, &v.dose_unit, &said.dose_amount) {
        (Some(amount), unit, None) => Some(match unit {
            Some(unit) => format!("{amount} {unit}"),
            None => amount.clone(),
        }),
        _ => None,
    };
    let route = v.route.clone().filter(|_| said.route.is_none());
    let directions = [dose, route, Some(sig.to_string())]
        .into_iter()
        .flatten()
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    let typed_drug = v.term.clone();
    let line_text = format!("{typed_drug} {directions}").trim().to_string();
    let part = PartArg {
        term: v.term.clone(),
        coding: v.coding.as_ref().map(|(system, code, display)| CodingArg {
            system: system.clone(),
            code: code.clone(),
            display: display.clone(),
        }),
    };
    let parts = serde_json::json!([part]);
    (typed_drug, directions, line_text, parts)
}

/// The press the restored line takes: the companion's, `pre-existing` for a companion charted
/// before slice 1a (the maintainer's ruling). The reviewer may change it. Pure.
pub(crate) fn restore_charted_as(v: &VersionForRestore) -> &str {
    v.companion_charted_as.as_deref().unwrap_or("pre-existing")
}

/// The reason each stop of a restore signs. Pure.
pub(crate) fn replaced_by(source: &str) -> String {
    format!("replaced by the {source} version")
}

/// The mark a restore line carries on the list. Pure.
pub(crate) fn restores_words(source: &str) -> String {
    format!("restores the {source} version")
}

/// The version `line` restores, if it is one of `versions` (the opened chart's fresh read).
pub(crate) fn version_of<'a>(
    line: &DraftLine,
    versions: &'a Versions,
) -> Option<&'a VersionForRestore> {
    versions.get(&line.restores?)
}

/// What a line's Sign adds for its restore (rulings R3 + R4): `Ok(None)` for an ordinary line;
/// `Ok(Some)` stopping each line that stands for the companion on the fresh read
/// (`version.standing`, none when nothing does), each with `reason`; [`RESTORE_VERSION_GONE`]
/// when the version is not on the opened chart's read. Pure.
pub(crate) fn restore_to_sign<'a>(
    line: &DraftLine,
    version: Option<&'a VersionForRestore>,
    reason: &'a str,
) -> Result<Option<RestoreToSign<'a>>, &'static str> {
    let Some(version_id) = line.restores else {
        return Ok(None);
    };
    let v = version.ok_or(RESTORE_VERSION_GONE)?;
    Ok(Some(RestoreToSign {
        version_id,
        stop: v.standing.iter().map(|id| (*id, reason)).collect(),
    }))
}

/// The source a list line signs, given `signed` (`list_input`'s: its own, else the press's
/// default): a restore line naming none signs `unknown` — the press says who reported a line
/// being charted, nothing about who reported the version (the maintainer's ruling). Pure.
pub(crate) fn restore_source<'a>(line: &DraftLine, signed: &'a str) -> &'a str {
    match (line.restores, &line.info_source) {
        (Some(_), None) => "unknown",
        _ => signed,
    }
}

/// Every version the list's `lines` restore, read on `patient` (the OPENED chart), inside the
/// list's snapshot.
pub(crate) async fn read_versions(
    client: &(impl GenericClient + Sync),
    patient: Uuid,
    lines: &[DraftLine],
) -> anyhow::Result<Versions> {
    let mut out = Versions::new();
    for id in lines.iter().filter_map(|l| l.restores) {
        if let Some(v) = version_for_restore(client, patient, id).await? {
            out.insert(id, v);
        }
    }
    Ok(out)
}

/// For each restore line of `lines` whose version reads, the chart's current lines it is judged
/// a duplicate against (`current_lines`' shape): all but the rows made wholly of the lines it
/// will stop (Review Focus 2). A row that also holds a thread it will NOT stop stays: that thread
/// is still current after Sign, so it is still a duplicate (a false duplicate costs a hand tick;
/// a missed one could chart the drug twice). Pure.
pub(crate) fn besides(
    rows: &[MedicationRow],
    lines: &[DraftLine],
    versions: &Versions,
) -> BTreeMap<Uuid, Vec<(String, String)>> {
    lines
        .iter()
        .filter_map(|l| {
            let standing = &version_of(l, versions)?.standing;
            let stops = |r: &&MedicationRow| {
                !r.members.is_empty()
                    && r.members
                        .iter()
                        .all(|m| standing.contains(&m.medication_id))
            };
            let kept: Vec<MedicationRow> = rows.iter().filter(|r| !stops(r)).cloned().collect();
            Some((l.line_id, current_lines(&kept)))
        })
        .collect()
}

/// Mark each restore line of `lines` (the pane's, in `judged`'s order) with the version it
/// restores ([`restores_words`]); a version no longer readable leaves no mark (Sign refuses it).
pub(crate) fn mark_restores(lines: &mut [ListLineView], judged: &[Judged], versions: &Versions) {
    for (view, j) in lines.iter_mut().zip(judged) {
        view.restores_words = version_of(&j.line, versions).map(|v| restores_words(&v.source));
    }
}

#[cfg(test)]
#[path = "med_list_restore_tests.rs"]
mod tests;
