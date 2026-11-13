//! The medication list's ONE Sign (batch charting slice 1, design §3.3): every ticked line is
//! charted under one key release, each line in its OWN transaction (ADR-0060 decision 7).
//!
//! Refusal order. Every refusal of the WHOLE Sign comes before the key, and is for a changed
//! picture only (the screen no longer shows what would be signed):
//!   1. the chart on screen and writable (`on_screen_and_writable`);
//!   2. the writer: a staff id typed in the strip must be the list's writer's
//!      ([`med_list_not_yours`] — the next person at the terminal never signs another's list);
//!   3. a fresh read (`med_list::fresh_read`) over the set on screen, in which every ticked id
//!      must be a line held by this terminal ([`MED_LIST_STALE`]);
//!   4. every ticked line's tick still holds on that read (`med_list_judge::tick_holds`): a
//!      default tick by `default_tick`, a hand tick while every warning found is one it was given
//!      against (one that arrived after the tick voids it). Any that fails loses its hand tick,
//!      and NOTHING is signed ([`med_list_changed`]); a second press signs the rest. A missing
//!      reason, or a reason against a check that could not run, is NOT a changed picture: the
//!      tick stands and that line alone is refused below (ruling R3).
//!
//! Then the key, ONCE (`signer_for_act`). For a named writer the key released must be the
//! writer's CURRENT key, by KEY (ruling R-B): a key held for someone else signs nothing. That key
//! is read BEFORE the release, so no read failure ever spends a passphrase.
//!
//! Then, in list order, each line: slice 1a's per-line checks (the press, the parts, the reason:
//! `NEEDS_A_REASON`, `CHECK_DID_NOT_RUN`),
//! a failure skipping that line only (ADR-0060 decision 1), and `chart_draft_line`, which writes
//! the event and spends the draft row in one transaction. A line whose commit got no answer may
//! have landed: it is reported NOT known and the loop STOPS, so the lines after it stay in the
//! list, ticked, each reported [`NOT_TRIED`], and the re-read list is the truth (Review Focus 4).
//!
//! The webview's word is never trusted: the lines signed are rebuilt from the stored rows, and
//! every tick, check and duplicate is re-derived here.
use crate::allergies::{failure_named, local_today, on_screen_and_writable};
use crate::chart_set::{check_displayed_set, ChartPane, ChartSetError};
use crate::commands::{med_list_impl, record_timing};
use crate::funnel::view::ErrorView;
use crate::link::judgement_view::refused;
use crate::med_chart::{override_before_the_key, press, UNKNOWN_LEAD};
use crate::med_chart_write::charted_input;
use crate::med_line_sign::{charting, components, override_refs, single_coding};
use crate::med_list::{
    claim_writer, fresh_read, judge, med_list_read_impl, tick_holds, Judged, MedListView,
};
use crate::notes::terminal_of;
use crate::signer::Credential;
use crate::state::{AppState, Now};
use crate::write_failure::{key_locked, node_cannot_write};
use cairn_event::medication::ChartedAs;
use cairn_node::medication::draft::{self, SpendRefused};
use cairn_node::medication::AuthorParams;
use std::time::Instant;
use uuid::Uuid;

/// The list changed since the screen was drawn (a ticked line is not in it, or not here).
pub const MED_LIST_STALE: &str =
    "The list changed since it was shown: nothing was signed. Look at it again";

/// Why a line was not charted when its spend found the list moved under it (it was saved again,
/// moved or removed since the read): nothing was written for it.
pub const LINE_MOVED: &str = "the line changed since the list was read; look at it again";

/// Why a line after one whose outcome is NOT known was not tried (new copy, review finding I3).
pub const NOT_TRIED: &str =
    "not tried: the line above may have been charted; look at the list again";

/// MED_LIST_CHANGED: the refusal when `n` ticks no longer hold (Global Constraints, pinned by test).
pub fn med_list_changed(n: usize) -> String {
    format!(
        "{n} ticked line(s) changed since they were ticked: nothing was signed. Look at them again"
    )
}

/// MED_LIST_NOT_YOURS: the refusal of a Sign by someone other than the list's writer (`name`).
pub fn med_list_not_yours(name: &str) -> String {
    format!("This list was written by {name}; only they can sign it")
}

/// What Sign answers. `changed`: nothing was signed, and `lines` (ids) lost their ticks. `done`:
/// the lines were tried — `charted` of `of` went in; `not_charted` is `(line id, reason)`;
/// `unknown` the line ids whose outcome is NOT known (at most one: the loop stops there);
/// `outcome` the words for all of it; `pane` the medication list and `list` this list, read again
/// (`None` when that read failed — the lines that went in are in the record either way).
// The `Done` variant is the larger by the pane it carries; one value per Sign, never stored.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MedListSigned {
    Changed {
        lines: Vec<String>,
        message: String,
    },
    Done {
        outcome: String,
        charted: usize,
        of: usize,
        not_charted: Vec<(String, String)>,
        unknown: Vec<String>,
        pane: Option<ChartPane>,
        list: Option<MedListView>,
    },
}

#[tauri::command]
pub async fn med_list_sign(
    state: tauri::State<'_, AppState>,
    patient_id: String,
    charts: Vec<String>,
    ticked: Vec<String>,
    credential: Option<Credential>,
) -> Result<MedListSigned, ErrorView> {
    med_list_sign_impl(&state, &patient_id, charts, &ticked, credential).await
}

/// Sign the ticked lines (refusal order: module doc).
pub async fn med_list_sign_impl(
    state: &AppState,
    patient_id: &str,
    charts: Vec<String>,
    ticked: &[String],
    credential: Option<Credential>,
) -> Result<MedListSigned, ErrorView> {
    let patient = on_screen_and_writable(state, patient_id, &charts).await?;
    let terminal = terminal_of(state)?.to_string();
    let (writer, name) = claim_writer(state).await;
    let name = name.unwrap_or_default();
    // 2. A typed staff id must be the writer's (a held key is checked by KEY after the release).
    if let (Some(w), Some(c)) = (&writer, credential.as_ref().filter(|c| !c.is_blank())) {
        let (theirs, ours) = (
            staff_id(state, &c.staff_id).await?,
            staff_id(state, w).await?,
        );
        writer_matches(theirs.as_deref(), ours.as_deref(), &name)?;
    }
    // 3. The fresh read, over the set on screen.
    let fresh = fresh_read(state, patient, writer.as_deref()).await?;
    check_displayed_set(&fresh.set, &charts).map_err(ChartSetError::into_view)?;
    let judged = judge(state, fresh, true, writer.is_some()).await;
    let chosen = ticked_lines(&judged, ticked, &terminal)?;
    // 4. Every tick still holds, or nothing is signed.
    let failing: Vec<&Judged> = chosen.iter().copied().filter(|j| !tick_holds(j)).collect();
    if !failing.is_empty() {
        untick(state, patient, writer.as_deref(), &failing).await;
        return Ok(MedListSigned::Changed {
            lines: failing.iter().map(|j| j.line.line_id.to_string()).collect(),
            message: med_list_changed(failing.len()),
        });
    }
    // Slice 1a's per-line checks, decided before the key so a Sign that could chart nothing never
    // asks for a passphrase; a failing line is skipped, the others sign.
    let verdicts: Vec<_> = chosen.iter().map(|j| (*j, line_verdict(j))).collect();
    let mut not_charted: Vec<(String, String)> = verdicts
        .iter()
        .filter_map(|(j, v)| {
            v.as_ref()
                .err()
                .map(|e| (j.line.line_id.to_string(), e.text.clone()))
        })
        .collect();
    let mut words: Vec<String> = verdicts
        .iter()
        .filter_map(|(j, v)| {
            v.as_ref()
                .err()
                .map(|e| not_charted_line(&j.line.line_text, &e.text))
        })
        .collect();
    let (mut charted, mut unknown) = (0, vec![]);
    let to_sign: Vec<_> = verdicts
        .iter()
        .filter_map(|(j, v)| v.as_ref().ok().map(|v| (*j, *v)))
        .collect();
    let started = Instant::now();
    if !to_sign.is_empty() {
        let node_sk = state
            .node_sk
            .as_ref()
            .ok_or_else(|| node_cannot_write("no node key"))?;
        // Read before the key, so a failed read never spends a passphrase; compared after it.
        let writer_kid = match &writer {
            Some(w) => Some(writer_key(state, w).await?),
            None => None,
        };
        let (human_sk, human_kid) = state
            .signer_for_act(credential, Now::read())
            .await
            .map_err(|r| key_locked(&r))?;
        // By KEY (ruling R-B): the only refusal after the key, and it writes nothing.
        if writer_kid.is_some_and(|kid| kid.as_deref() != Some(human_kid.as_str())) {
            return Err(refused(med_list_not_yours(&name)));
        }
        let author = AuthorParams {
            human_sk: &human_sk,
            human_kid: &human_kid,
        };
        let mut lines = to_sign.into_iter();
        while let Some((j, (charted_as, reason))) = lines.next() {
            let list = (patient, terminal.as_str(), writer.as_deref());
            let signed = sign_line(state, node_sk, list, j, charted_as, reason, &author).await;
            match signed.map_err(|e| line_failure(&e)) {
                Ok(()) => charted += 1,
                Err(LineFailure::Unknown(text)) => {
                    // It may have landed: never try the rest on a guess (Review Focus 4). Each
                    // line left is named, so none is silently missing from the outcome (I3).
                    unknown.push(j.line.line_id.to_string());
                    words.push(format!("{} — {text}", j.line.line_text));
                    for (rest, _) in lines.by_ref() {
                        words.push(not_charted_line(&rest.line.line_text, NOT_TRIED));
                        not_charted.push((rest.line.line_id.to_string(), NOT_TRIED.into()));
                    }
                }
                Err(LineFailure::NotCharted(text)) => {
                    words.push(not_charted_line(&j.line.line_text, &text));
                    not_charted.push((j.line.line_id.to_string(), text));
                }
            }
        }
    }
    let elapsed_ms = started.elapsed().as_millis().min(i32::MAX as u128) as i32;
    record_timing(state, "medication-chart", charted, elapsed_ms).await;
    let pane = med_list_impl(state, &patient.to_string(), Some(charts.clone()))
        .await
        .map_err(|e| eprintln!("the medication list was not read again after Sign: {e}"))
        .ok();
    let list = med_list_read_impl(state, patient_id, Some(&charts))
        .await
        .map_err(|e| eprintln!("the medication draft list was not read again: {}", e.text))
        .ok();
    Ok(MedListSigned::Done {
        outcome: outcome(charted, chosen.len(), &words),
        charted,
        of: chosen.len(),
        not_charted,
        unknown,
        pane,
        list,
    })
}

/// Sign one line that passed its checks: rebuild what it signs from the STORED row, then write
/// its event and spend its draft row in one transaction (`chart_draft_line`).
/// `list` is the list's identity: `(chart, terminal, writer)`.
async fn sign_line(
    state: &AppState,
    node_sk: &cairn_event::SigningKey,
    (patient, terminal, writer): (Uuid, &str, Option<&str>),
    j: &Judged,
    charted_as: ChartedAs,
    reason: Option<&str>,
    author: &AuthorParams<'_>,
) -> anyhow::Result<()> {
    let (term, parts) = j
        .signable
        .as_ref()
        .map_err(|e| anyhow::anyhow!("{}", e.text))?;
    let ids: Vec<String> = j.check.against.iter().map(Uuid::to_string).collect();
    let against = override_refs(&ids);
    let today = local_today().to_string();
    let components = components(parts);
    let input = charted_input(
        term,
        single_coding(parts),
        &j.read,
        &j.line.directions,
        charted_as,
        &today,
    );
    let charting = charting(charted_as, &j.read, &components, reason, &against);
    let node_kid = hex::encode(node_sk.verifying_key().to_bytes());
    let mut db = state
        .db
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("no database connection"))?
        .lock()
        .await?;
    draft::chart_draft_line(
        &mut db,
        node_sk,
        &node_kid,
        &state.node_origin,
        patient,
        terminal,
        writer,
        j.line.line_id,
        &j.line.saved_at,
        &input,
        &charting,
        author,
        &[], // Task 6 wires versions
    )
    .await
    .map(|_| ())
    .map_err(|e| db.explain(e))
}

/// How one line's sign failed, as the outcome says it.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum LineFailure {
    /// The commit got no answer: it may have landed (`failure_named`'s NOT-known words).
    Unknown(String),
    /// Nothing was written for it: why.
    NotCharted(String),
}

/// Classify a failed line. Pure.
pub(crate) fn line_failure(e: &anyhow::Error) -> LineFailure {
    if cairn_node::db_diagnosis::commit_outcome_unknown_event(e).is_some() {
        return LineFailure::Unknown(failure_named(e, UNKNOWN_LEAD).text);
    }
    if e.downcast_ref::<SpendRefused>().is_some() {
        return LineFailure::NotCharted(LINE_MOVED.into());
    }
    LineFailure::NotCharted(failure_named(e, UNKNOWN_LEAD).text)
}

/// The ticked ids as lines of `judged`, in LIST order, or [`MED_LIST_STALE`] when one is not a
/// line of this read held by `terminal` (the screen showed a list that is no longer there). Pure.
pub(crate) fn ticked_lines<'a>(
    judged: &'a [Judged],
    ticked: &[String],
    terminal: &str,
) -> Result<Vec<&'a Judged>, ErrorView> {
    let mut ids = Vec::with_capacity(ticked.len());
    for t in ticked {
        let id: Uuid = t.parse().map_err(|_| refused(MED_LIST_STALE))?;
        if !judged
            .iter()
            .any(|j| j.line.line_id == id && j.line.terminal_id == terminal)
        {
            return Err(refused(MED_LIST_STALE));
        }
        ids.push(id);
    }
    Ok(judged
        .iter()
        .filter(|j| ids.contains(&j.line.line_id))
        .collect())
}

/// Slice 1a's per-line checks on a holding line: its press, what it signs, and the reason a
/// Start line with an allergy mark needs. Pure.
fn line_verdict(j: &Judged) -> Result<(ChartedAs, Option<&str>), ErrorView> {
    let charted_as = press(&j.line.charted_as)?;
    j.signable.as_ref().map_err(Clone::clone)?;
    let reason = override_before_the_key(charted_as, &j.check, j.line.reason.as_deref())?;
    Ok((charted_as, reason))
}

/// A typed staff id against the list's writer, both as the vault normalised them. A typed id the
/// vault cannot normalise (blank) is no one's. Pure.
pub(crate) fn writer_matches(
    typed: Option<&str>,
    writer: Option<&str>,
    name: &str,
) -> Result<(), ErrorView> {
    match (typed, writer) {
        (Some(t), Some(w)) if t == w => Ok(()),
        _ => Err(refused(med_list_not_yours(name))),
    }
}

/// The outcome's words: "Charted {k} of {n}", then one line per line not charted (or not known).
/// Pure.
pub(crate) fn outcome(charted: usize, of: usize, lines: &[String]) -> String {
    std::iter::once(format!("Charted {charted} of {of}"))
        .chain(lines.iter().cloned())
        .collect::<Vec<_>>()
        .join("\n")
}

/// One line not charted, as the outcome names it. Pure.
pub(crate) fn not_charted_line(drug_line: &str, reason: &str) -> String {
    format!("{drug_line} — not charted: {reason}")
}

/// Clear the hand tick of each line in `lines` that has one, through the save door (the same
/// content with `hand_ticked = false`): a tick that no longer holds must not survive to the next
/// press. A default tick is derived, never stored, so there is nothing to clear. A failure is
/// logged: the next Sign re-derives the tick and refuses it again.
async fn untick(state: &AppState, patient: Uuid, writer: Option<&str>, lines: &[&Judged]) {
    let Some(db) = state.db.as_ref() else { return };
    let db = match db.lock().await {
        Ok(db) => db,
        Err(e) => return eprintln!("the changed lines' ticks were not cleared: {e}"),
    };
    for j in lines.iter().filter(|j| j.line.hand_ticked) {
        let mut line = j.line.clone();
        line.hand_ticked = false;
        line.ticked_against.clear();
        match draft::save_draft_line(&*db, patient, writer, &line).await {
            Ok(a) if a == "saved" => {}
            Ok(a) => eprintln!("a changed line's tick was not cleared: the list answered {a}"),
            Err(e) => eprintln!(
                "a changed line's tick was not cleared: {}",
                cairn_gui_live::cause_of(&db.explain(e))
            ),
        }
    }
}

/// `raw` as the vault normalises a staff id (the rule is SQL's alone, R7).
async fn staff_id(state: &AppState, raw: &str) -> Result<Option<String>, ErrorView> {
    let db = state
        .db
        .as_ref()
        .ok_or_else(|| node_cannot_write("no database connection"))?
        .lock()
        .await?;
    draft::staff_id(&*db, raw)
        .await
        .map_err(|e| crate::write_failure::node_failure(&db.explain(e)))
}

/// The writer's CURRENT vault key (`None`: no such member).
async fn writer_key(state: &AppState, writer: &str) -> Result<Option<String>, ErrorView> {
    let db = state
        .db
        .as_ref()
        .ok_or_else(|| node_cannot_write("no database connection"))?
        .lock()
        .await?;
    draft::writer_key(&*db, writer)
        .await
        .map_err(|e| crate::write_failure::node_failure(&db.explain(e)))
}
