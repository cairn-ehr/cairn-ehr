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
//! stop ([`current_besides_stops`], Review Focus 2), but still against every other line, list
//! lines included (two moiety-coded restores of one drug duplicate each other, Review Focus 3).
use crate::allergy_check::CodingArg;
use crate::funnel::view::ErrorView;
use crate::link::judgement_view::refused;
use crate::med_chart::PartArg;
use crate::med_list::FIXTURE;
use crate::med_list::{
    claim_writer, med_list_read_impl, Judged, ListLineView, MedListView, WarningRef,
};
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
use std::collections::{BTreeMap, BTreeSet};
use tokio_postgres::GenericClient;
use uuid::Uuid;

/// The press or the check named a version that is not on the opened chart's read (gone since the
/// pane was drawn, or a linked chart's: spec §2 offers neither).
pub const VERSION_GONE: &str = "That version is no longer on this chart; read it again";

/// The press or the check named a version restored since the pane was drawn: the pane offers
/// neither settle act on a restored version (`can_settle`), so a stale pane's press is refused.
pub const VERSION_RESTORED: &str = "That version was restored since; read the chart again";

/// The source of a restore line's marker ref in its `against` / `ticked_against`: the version it
/// restores ([`restore_seen`]).
pub(crate) const RESTORE_OF: &str = "restore-of";

/// The source of each ref naming a line that stood for the companion when the line was drawn.
pub(crate) const RESTORE_STANDS: &str = "restore-stands";

/// A restore line's per-line Sign refusal: its version is no longer on the opened chart.
pub const RESTORE_VERSION_GONE: &str = "the version it restores can no longer be read";

/// The list is held by another terminal: the pane's own words for it (`med_list.js`'s
/// `MED_LIST_ELSEWHERE`), followed by that terminal.
pub const LIST_ELSEWHERE: &str = "These lines are open on another terminal: ";

/// Ruling R9 (a): a ticked restore line whose lineage an earlier ticked restore line replaces
/// (the same companion, or a line both would stop: [`restore_holds`], ruling R10).
/// Its words say what signing it later does (ruling R13).
pub const SAME_LINE_RESTORED: &str =
    "another restore of the same line is signed first; signed after it, this one replaces it";

/// The opened chart's versions the list's restore lines name, by version id (a version not on
/// it is absent).
pub(crate) type Versions = BTreeMap<Uuid, VersionForRestore>;

/// What the press answers once the line was added: the list read again, and the new line, for
/// the editor to open on; or, when the list could not be read again, no list and `said`.
#[derive(Debug, serde::Serialize)]
pub struct RestoreOpened {
    pub list: Option<MedListView>,
    pub line_id: String,
    /// Why there is no list: the line was added, the read after it failed.
    pub said: Option<String>,
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
        not_restored(&v).map_err(refused)?;
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
        if answer != "saved" {
            return Err(refused(not_added(&answer)));
        }
    }
    let reread = med_list_read_impl(state, patient_id, Some(&charts)).await;
    Ok(restore_opened(reread, line))
}

/// What an add that LANDED answers: always an Ok (as `med_version_check::checked`) — the list
/// read again, or words saying the line was added and how to see it. Never an error: the window
/// would offer the press again, and a second press adds a second line (a fresh line id). Pure.
pub(crate) fn restore_opened(reread: Result<MedListView, ErrorView>, line: Uuid) -> RestoreOpened {
    let line_id = line.to_string();
    match reread {
        Ok(list) => RestoreOpened {
            list: Some(list),
            line_id,
            said: None,
        },
        Err(e) => RestoreOpened {
            list: None,
            line_id,
            said: Some(format!(
                "The line was added, but the list could not be read again ({}). \
                 Use “Read this chart again”.",
                e.text
            )),
        },
    }
}

/// A press (restore or check) on a version restored since the pane was drawn is refused
/// ([`VERSION_RESTORED`]), as the pane would not offer it. Pure.
pub(crate) fn not_restored(v: &VersionForRestore) -> Result<(), &'static str> {
    match v.restored {
        true => Err(VERSION_RESTORED),
        false => Ok(()),
    }
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

/// The press the restored line takes: `started-here` when the companion was started here, else
/// `pre-existing` — for a companion charted before slice 1a (the maintainer's ruling), and for
/// `unknown` or any word a peer sent (ruling R11), since a list line takes only the two presses
/// (db/078's CHECK). The line keeps it: no list control changes a press. Pure.
pub(crate) fn restore_charted_as(v: &VersionForRestore) -> &'static str {
    match v.companion_charted_as.as_deref() {
        Some("started-here") => "started-here",
        _ => "pre-existing",
    }
}

/// The reason each stop of a restore signs. Pure.
pub(crate) fn replaced_by(source: &str) -> String {
    format!("replaced by the {source} version")
}

/// The mark a restore line carries on the list, saying when its line was stopped since the press
/// (nothing stands for its companion, ruling R12). Pure.
pub(crate) fn restores_words(v: &VersionForRestore) -> String {
    let since = match v.standing.is_empty() {
        true => " — its line has since been stopped",
        false => "",
    };
    format!("restores the {} version{since}", v.source)
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
pub(crate) fn current_besides_stops(
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
/// restores ([`restores_words`]); one whose version no longer reads says so
/// ([`RESTORE_VERSION_GONE`]) before Sign refuses it. Its `against` gains what a hand tick on it
/// vouches for ([`restore_seen`]).
pub(crate) fn mark_restores(lines: &mut [ListLineView], judged: &[Judged], versions: &Versions) {
    for (view, j) in lines.iter_mut().zip(judged) {
        view.against.extend(restore_seen(&j.line, versions));
        view.restores_words = j
            .line
            .restores
            .map(|_| match version_of(&j.line, versions) {
                Some(v) => restores_words(v),
                None => RESTORE_VERSION_GONE.to_string(),
            });
    }
}

/// The restore door's refusal as the clinician reads it, never its raw word. Pure.
pub(crate) fn not_added(answer: &str) -> String {
    let why = match answer {
        "no-such-version" => return VERSION_GONE.into(),
        "elsewhere" | "moved" => "the list is open on another terminal; read it again",
        "no-such-writer" => "the person named is not a clinical member here",
        "deleted" => "that line was removed; press again",
        "no-chart" => "this chart is not on this node",
        _ => "the list refused it; read it again",
    };
    format!("The line was not added: {why}")
}

/// Ruling R9 (a): a ticked line's restore, judged before the key in Sign order. `Ok` for an
/// ordinary line or a restore that may sign; [`RESTORE_VERSION_GONE`] when its version is not on
/// the opened chart; [`SAME_LINE_RESTORED`] when an `earlier` restore line about to sign
/// replaces the same lineage (its stops were read before that one charts, so the floor would
/// refuse it at commit in its own words). Same lineage (ruling R10): the same companion — two
/// restores of a stopped companion both stop nothing — or a line both would stop (a version set
/// aside beside a restore line has that line as its companion, under the same original). Pure.
pub(crate) fn restore_holds<'a>(
    line: &DraftLine,
    earlier: impl IntoIterator<Item = &'a DraftLine>,
    versions: &Versions,
) -> Result<(), &'static str> {
    if line.restores.is_none() {
        return Ok(());
    }
    let v = version_of(line, versions).ok_or(RESTORE_VERSION_GONE)?;
    let same = |e: &DraftLine| {
        version_of(e, versions).is_some_and(|w| {
            w.alternative_to == v.alternative_to
                || w.standing.iter().any(|s| v.standing.contains(s))
        })
    };
    match earlier.into_iter().any(same) {
        true => Err(SAME_LINE_RESTORED),
        false => Ok(()),
    }
}

/// Ruling R9 (b): whether `line` restores a version whose companion was already replaced by a
/// restore line (one of `standing` carries a restore). Pure.
pub(crate) fn replaces_a_restore(line: &DraftLine, versions: &Versions) -> bool {
    version_of(line, versions).is_some_and(|v| !v.standing_restored.is_empty())
}

/// Ruling R12: whether `line` restores a version whose line was stopped since the press (nothing
/// stands for its companion on the fresh read), so its Sign would restart a stopped drug. Pure.
pub(crate) fn restores_a_stopped_line(line: &DraftLine, versions: &Versions) -> bool {
    version_of(line, versions).is_some_and(|v| v.standing.is_empty())
}

/// Whether a restore line is never default-ticked on this read: it replaces a restore (R9b),
/// restores a stopped line (R12), or its version no longer reads (Sign refuses it,
/// [`RESTORE_VERSION_GONE`]). Pure.
pub(crate) fn restore_withholds_default_tick(line: &DraftLine, versions: &Versions) -> bool {
    line.restores.is_some()
        && (version_of(line, versions).is_none()
            || replaces_a_restore(line, versions)
            || restores_a_stopped_line(line, versions))
}

/// What a restore line's hand tick vouches for, drawn into its `against` and sent back as `seen`
/// when ticked (as its allergy marks are): the version it restores ([`RESTORE_OF`]), then each
/// line standing for its companion as drawn ([`RESTORE_STANDS`]). Empty for an ordinary line or
/// one whose version no longer reads. Pure.
pub(crate) fn restore_seen(line: &DraftLine, versions: &Versions) -> Vec<WarningRef> {
    let (Some(id), Some(v)) = (line.restores, version_of(line, versions)) else {
        return vec![];
    };
    let r = |source: &str, id: &Uuid| WarningRef {
        source: source.into(),
        reference: id.to_string(),
    };
    std::iter::once(r(RESTORE_OF, &id))
        .chain(v.standing.iter().map(|s| r(RESTORE_STANDS, s)))
        .collect()
}

/// The restore refs of a hand tick's `seen`, as the door stores them (`ticked_against`). Allergy
/// refs are `vouched`'s; a ref that is not a uuid vouches for nothing. Pure.
pub(crate) fn restore_vouched(seen: &[WarningRef]) -> Vec<(String, Uuid)> {
    seen.iter()
        .filter(|w| w.source == RESTORE_OF || w.source == RESTORE_STANDS)
        .filter_map(|w| Some((w.source.clone(), w.reference.parse().ok()?)))
        .collect()
}

/// Whether a HAND tick on `line` still holds for its restore (R9b, R12, the chart-held rule): it
/// vouched for this version and for exactly the lines standing for its companion on the fresh
/// read. A colleague's stop or restore since the tick changes that set, so the tick fails and
/// Sign hands the line back unticked, never signing a stop or restart its clinician did not see.
/// A tick given after the "since been stopped" mark vouched for the empty set: a deliberate
/// restart. An ordinary line holds here, and so does one whose version is gone: Sign refuses
/// that line alone ([`RESTORE_VERSION_GONE`], decision 18), never the whole Sign. Pure.
pub(crate) fn restore_tick_holds(line: &DraftLine, versions: &Versions) -> bool {
    let Some(id) = line.restores else {
        return true;
    };
    let Some(v) = version_of(line, versions) else {
        return true;
    };
    let vouched = &line.ticked_against;
    let marked = vouched.iter().any(|(s, r)| s == RESTORE_OF && *r == id);
    let stood: BTreeSet<Uuid> = vouched
        .iter()
        .filter(|(s, _)| s == RESTORE_STANDS)
        .map(|(_, r)| *r)
        .collect();
    marked && stood == v.standing.iter().copied().collect()
}

#[cfg(test)]
#[path = "med_list_restore_tests.rs"]
mod tests;
