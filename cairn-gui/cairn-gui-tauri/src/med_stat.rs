//! Charting slice 2 in the window: a stat dose, ordered then given (design §3.6). On paper a stat
//! order is a line in the "stat" box; the nurse who gives it initials it with the time, and the
//! order is done. Here:
//!
//! - **Sign** signs `stat: true` on a line that is a stat ORDER ([`is_stat_order`], called by
//!   `med_line_sign::charting`, so the one-line Sign and the list's Sign can never disagree).
//! - **Given** ([`med_stat_give`]) signs the give with the time given ("Time given", now when
//!   left empty); the node signs the order's stop with it, so the line leaves the current list.
//! - **Entered in error** ([`med_stat_strike`]) strikes a give; the stop it made is voided, so
//!   the order is waiting to be given again.
//!
//! Both commands run `med_version_check.rs`'s steps in its order: the chart on screen, its set
//! and fixture mode (`on_screen_and_writable`); the press's own input (the time given); a FRESH
//! read of the OPENED chart's stat orders only (never a linked chart's, spec §2), because the
//! webview's picture may be stale; only then the key, ONCE, the act, and the pane read again. A
//! stale press is refused before the key and writes nothing. Nothing here asks for confirmation.
use crate::allergies::{failure_named, on_screen_and_writable};
use crate::commands::med_list_impl;
use crate::funnel::view::ErrorView;
use crate::link::judgement_view::refused;
use crate::med_chart_write::MedChartWrite;
use crate::med_list_versions::given;
use crate::notes::time::{stated_time_from_local, year_the_record_holds};
use crate::signer::Credential;
use crate::state::{AppState, Now};
use crate::write_failure::{key_locked, node_cannot_write, node_failure};
use cairn_event::medication::ChartedAs;
use cairn_node::medication::line::Directions;
use cairn_node::medication::stat_give::{give_stat, strike_given};
use cairn_node::medication::stat_read::read_stat_orders;
use cairn_node::medication::AuthorParams;
use chrono::{DateTime, Utc};
use uuid::Uuid;

/// Given on a line that is not a waiting stat order of the opened chart: stopped, given or set
/// aside since the pane was drawn, a linked chart's, or never a stat order.
pub const STAT_NOT_OPEN: &str =
    "That stat dose is no longer waiting to be given; read the chart again";
/// A time given later than now: a dose cannot have been given in the future.
pub const GIVEN_LATER_THAN_NOW: &str = "The time given cannot be later than now";
/// Entered in error on a give not on the opened chart, or struck since the pane was drawn.
pub const GIVE_GONE: &str =
    "That given dose is no longer on this chart, or is already struck; read it again";

/// Whose outcome is NOT known when the commit got no answer (`failure_named`).
const GIVE_LEAD: &str = "Whether this stat dose was recorded as given";
const STRIKE_LEAD: &str = "Whether this given dose was recorded as entered in error";

/// A stat ORDER: started here ("Start"), its directions read `stat`, with no frequency and not
/// prn. "1g iv stat then daily" is a regimen with a first dose and "prn stat" is "when needed":
/// neither may leave the list at its first give. An "Already taking" line orders nothing. Pure.
pub(crate) fn is_stat_order(charted_as: ChartedAs, d: &Directions) -> bool {
    charted_as == ChartedAs::StartedHere && d.stat && d.frequency.is_none() && !d.prn
}

/// The time given, from the row's "Time given" field (`datetime-local` text, this machine's
/// zone): empty or absent is `now`. A time later than `now` is [`GIVEN_LATER_THAN_NOW`]; an
/// earlier one is kept, even one before the order was charted (§10 Q3: shown, never refused). A
/// partly typed time ("2026-11-14", no hour) is refused in the parser's own words, never read as
/// midnight. Pure.
pub(crate) fn given_time(text: Option<&str>, now: DateTime<Utc>) -> Result<DateTime<Utc>, String> {
    let Some(s) = text.filter(|s| !s.is_empty()) else {
        return Ok(now);
    };
    let t = stated_time_from_local(s)?;
    // A year the wire cannot hold would be refused by the floor AFTER the key: said here.
    year_the_record_holds(s, t)?;
    if t > now {
        return Err(GIVEN_LATER_THAN_NOW.into());
    }
    Ok(t)
}

/// What a strike that landed says: "waiting again" only when, read AFTER the strike, the order
/// is waiting and its line is current (`waiting_here`); otherwise only "Entered in error.". Pure.
pub(crate) fn struck_outcome(waiting_again: bool) -> &'static str {
    if waiting_again {
        "Entered in error: the stat dose is waiting to be given again."
    } else {
        "Entered in error."
    }
}

/// What an act that LANDED answers (a give, a strike, a version checked): always an Ok: the pane
/// read again, or a sentence saying it was signed and how to see it — never an error, which would
/// read as "not recorded" and be offered for retry. For a give or a strike the retry would only be
/// refused as stale (`STAT_NOT_OPEN`), leaving the clinician believing a dose they gave went
/// unrecorded; a retried check would record it twice (`med_version_check::checked`). Pure.
pub(crate) fn landed(
    outcome: &str,
    reread: Result<crate::chart_set::ChartPane, String>,
) -> MedChartWrite {
    match reread {
        Ok(pane) => MedChartWrite {
            outcome: outcome.into(),
            pane: Some(pane),
        },
        Err(e) => MedChartWrite {
            outcome: format!(
                "Signed — but the medication list could not be read again ({e}). \
                 Use “Read this chart again”."
            ),
            pane: None,
        },
    }
}

#[tauri::command]
pub async fn med_stat_give(
    state: tauri::State<'_, AppState>,
    patient_id: String,
    charts: Vec<String>,
    medication_id: String,
    given_at: Option<String>,
    credential: Option<Credential>,
) -> Result<MedChartWrite, ErrorView> {
    med_stat_give_impl(
        &state,
        &patient_id,
        charts,
        &medication_id,
        given_at,
        credential,
    )
    .await
}

/// Give the stat order `medication_id` at `given_at` (refusal order: module doc).
pub async fn med_stat_give_impl(
    state: &AppState,
    patient_id: &str,
    charts: Vec<String>,
    medication_id: &str,
    given_at: Option<String>,
    credential: Option<Credential>,
) -> Result<MedChartWrite, ErrorView> {
    let patient = on_screen_and_writable(state, patient_id, &charts).await?;
    let medication: Uuid = medication_id.parse().map_err(|_| refused(STAT_NOT_OPEN))?;
    let given_at = given_time(given_at.as_deref(), Utc::now()).map_err(refused)?;
    {
        let db = state
            .db
            .as_ref()
            .ok_or_else(|| node_cannot_write("no database connection"))?
            .lock()
            .await?;
        let open = waiting_here(&db, patient, medication)
            .await
            .map_err(|e| node_failure(&db.explain(e)))?;
        if !open {
            return Err(refused(STAT_NOT_OPEN));
        }
    }
    let (human_sk, human_kid) = state
        .signer_for_act(credential, Now::read())
        .await
        .map_err(|r| key_locked(&r))?;
    let author = AuthorParams {
        human_sk: &human_sk,
        human_kid: &human_kid,
    };
    let node_sk = state
        .node_sk
        .as_ref()
        .ok_or_else(|| node_cannot_write("no node key"))?;
    let node_kid = hex::encode(node_sk.verifying_key().to_bytes());
    let signed = {
        let mut db = state
            .db
            .as_ref()
            .ok_or_else(|| node_cannot_write("no database connection"))?
            .lock()
            .await?;
        let origin = state.node_origin.as_str();
        give_stat(
            &mut db, node_sk, &node_kid, origin, patient, medication, given_at, &author,
        )
        .await
        .map_err(|e| db.explain(e))
    };
    signed.map_err(|e| failure_named(&e, GIVE_LEAD))?;
    let reread = med_list_impl(state, &patient.to_string(), Some(charts)).await;
    Ok(landed("Given.", reread))
}

/// Whether `medication` is, on a fresh read, a stat order OF `patient` (the opened chart only)
/// still waiting to be given and still on the current list. A real stop since the draw leaves
/// the order "waiting" (no give) but no longer current, so both are asked.
async fn waiting_here(
    db: &tokio_postgres::Client,
    patient: Uuid,
    medication: Uuid,
) -> anyhow::Result<bool> {
    let orders = read_stat_orders(db, &[patient]).await?;
    if !orders
        .iter()
        .any(|o| o.medication_id == medication && o.waiting())
    {
        return Ok(false);
    }
    let row = db
        .query_one(
            "SELECT EXISTS (SELECT 1 FROM patient_medication_current \
                             WHERE medication_id = $1::text::uuid AND patient_id = $2::text::uuid)",
            &[&medication.to_string(), &patient.to_string()],
        )
        .await?;
    Ok(row.try_get(0)?)
}

#[tauri::command]
pub async fn med_stat_strike(
    state: tauri::State<'_, AppState>,
    patient_id: String,
    charts: Vec<String>,
    given_event: String,
    why: Option<String>,
    credential: Option<Credential>,
) -> Result<MedChartWrite, ErrorView> {
    med_stat_strike_impl(&state, &patient_id, charts, &given_event, why, credential).await
}

/// Strike the give `given_event` ("Entered in error", refusal order: module doc). A blank why
/// is none.
pub async fn med_stat_strike_impl(
    state: &AppState,
    patient_id: &str,
    charts: Vec<String>,
    given_event: &str,
    why: Option<String>,
    credential: Option<Credential>,
) -> Result<MedChartWrite, ErrorView> {
    let patient = on_screen_and_writable(state, patient_id, &charts).await?;
    let given_id: Uuid = given_event.parse().map_err(|_| refused(GIVE_GONE))?;
    // The order the standing give belongs to: read again after the strike (`struck_outcome`).
    let medication = {
        let db = state
            .db
            .as_ref()
            .ok_or_else(|| node_cannot_write("no database connection"))?
            .lock()
            .await?;
        let orders = read_stat_orders(&*db, &[patient])
            .await
            .map_err(|e| node_failure(&db.explain(e)))?;
        orders
            .iter()
            .find(|o| {
                o.gives
                    .iter()
                    .any(|g| g.event_id == given_id && g.struck.is_none())
            })
            .map(|o| o.medication_id)
            .ok_or_else(|| refused(GIVE_GONE))?
    };
    let why = given(why);
    let (human_sk, human_kid) = state
        .signer_for_act(credential, Now::read())
        .await
        .map_err(|r| key_locked(&r))?;
    let author = AuthorParams {
        human_sk: &human_sk,
        human_kid: &human_kid,
    };
    let node_sk = state
        .node_sk
        .as_ref()
        .ok_or_else(|| node_cannot_write("no node key"))?;
    let node_kid = hex::encode(node_sk.verifying_key().to_bytes());
    let signed = {
        let mut db = state
            .db
            .as_ref()
            .ok_or_else(|| node_cannot_write("no database connection"))?
            .lock()
            .await?;
        let origin = state.node_origin.as_str();
        let struck = strike_given(
            &mut db,
            node_sk,
            &node_kid,
            origin,
            patient,
            given_id,
            why.as_deref(),
            &author,
        )
        .await
        .map_err(|e| db.explain(e));
        // The words come from the state AFTER the strike, never from the struck give alone: a
        // second standing give (two terminals, two offline peers) or a real stop keeps the line
        // stopped, and a false "waiting to be given again" invites a second dose. A failed read
        // here says only "Entered in error." — never an Err, since the strike has landed.
        match struck {
            Ok(_) => Ok(waiting_here(&db, patient, medication)
                .await
                .unwrap_or_else(|e| {
                    eprintln!("the stat order was not read again after the strike: {e:#}");
                    false
                })),
            Err(e) => Err(e),
        }
    };
    let waiting_again = signed.map_err(|e| failure_named(&e, STRIKE_LEAD))?;
    let reread = med_list_impl(state, &patient.to_string(), Some(charts)).await;
    Ok(landed(struck_outcome(waiting_again), reread))
}

#[cfg(test)]
#[path = "med_stat_tests.rs"]
mod tests;
