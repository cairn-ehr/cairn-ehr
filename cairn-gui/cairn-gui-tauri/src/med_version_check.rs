//! Batch charting slice 2b in the window: "Checked: not current" (design §3.4), an IMMEDIATE
//! signed act on one set-aside version of the opened chart. A reviewing clinician records that a
//! version was looked at and is not what the patient takes now, saying where they checked
//! ("From:") and, optionally, why. It locks nothing: the version keeps both buttons.
//!
//! It runs `allergy_overlays.rs`'s steps in their order (`allergy_retract_impl`): the chart on
//! screen, its set and fixture mode (`on_screen_and_writable`); the version must still be on a
//! FRESH read of the OPENED chart (never a linked chart's, spec §2: [`VERSION_GONE`]) and not
//! restored since (`VERSION_RESTORED`, as the pane offers neither act on it); only then
//! the key, ONCE, the one event (`check_version`), and the medication pane read again. The check
//! row signs nothing until this command, and nothing here asks for confirmation.
use crate::allergies::{failure_named, on_screen_and_writable};
use crate::commands::med_list_impl;
use crate::funnel::view::ErrorView;
use crate::link::judgement_view::refused;
use crate::med_chart_write::MedChartWrite;
use crate::med_list_restore::{not_restored, VERSION_GONE};
use crate::med_list_versions::{given, stamp, FromView};
use crate::signer::Credential;
use crate::state::{AppState, Now};
use crate::write_failure::{key_locked, node_cannot_write, node_failure};
use cairn_event::medication::SourceNote;
use cairn_node::medication::version_settle::{check_version, VersionCheck};
use cairn_node::medication::version_settle_read::version_for_restore;
use cairn_node::medication::AuthorParams;
use uuid::Uuid;

/// Whose outcome is NOT known when the commit got no answer (`failure_named`).
const CHECK_LEAD: &str = "Whether this version was recorded as checked";

#[tauri::command]
#[allow(clippy::too_many_arguments)] // Tauri passes each argument the webview sends by name.
pub async fn med_version_check(
    state: tauri::State<'_, AppState>,
    patient_id: String,
    charts: Vec<String>,
    version_id: String,
    preset: Option<String>,
    detail: Option<String>,
    why: Option<String>,
    credential: Option<Credential>,
) -> Result<MedChartWrite, ErrorView> {
    med_version_check_impl(
        &state,
        &patient_id,
        charts,
        &version_id,
        preset,
        detail,
        why,
        credential,
    )
    .await
}

/// Check `version_id` (refusal order: module doc). `preset` and `detail` are "From:"'s, mapped as
/// the list maps them (`stamp`); with no preset the source is `unknown` and the detail is kept
/// as typed (ruling R1: the category is never guessed). A blank why is none.
#[allow(clippy::too_many_arguments)]
pub async fn med_version_check_impl(
    state: &AppState,
    patient_id: &str,
    charts: Vec<String>,
    version_id: &str,
    preset: Option<String>,
    detail: Option<String>,
    why: Option<String>,
    credential: Option<Credential>,
) -> Result<MedChartWrite, ErrorView> {
    let patient = on_screen_and_writable(state, patient_id, &charts).await?;
    let version: Uuid = version_id.parse().map_err(|_| refused(VERSION_GONE))?;
    {
        let db = state
            .db
            .as_ref()
            .ok_or_else(|| node_cannot_write("no database connection"))?
            .lock()
            .await?;
        let v = version_for_restore(&*db, patient, version)
            .await
            .map_err(|e| node_failure(&db.explain(e)))?
            .ok_or_else(|| refused(VERSION_GONE))?;
        not_restored(&v).map_err(refused)?;
    }
    let (info_source, source_detail) = check_source(preset, detail);
    let why = given(why);
    let source_note = note(source_detail.as_deref(), why.as_deref());
    let (human_sk, human_kid) = state
        .signer_for_act(credential, Now::read())
        .await
        .map_err(|r| key_locked(&r))?;
    let node_sk = state
        .node_sk
        .as_ref()
        .ok_or_else(|| node_cannot_write("no node key"))?;
    let node_kid = hex::encode(node_sk.verifying_key().to_bytes());
    let author = AuthorParams {
        human_sk: &human_sk,
        human_kid: &human_kid,
    };
    let check = VersionCheck {
        version_id: version,
        info_source,
        source_note,
    };
    let signed = {
        let mut db = state
            .db
            .as_ref()
            .ok_or_else(|| node_cannot_write("no database connection"))?
            .lock()
            .await?;
        let origin = state.node_origin.as_str();
        check_version(
            &mut db, node_sk, &node_kid, origin, patient, &check, &author,
        )
        .await
        .map_err(|e| db.explain(e))
    };
    signed.map_err(|e| failure_named(&e, CHECK_LEAD))?;
    let reread = med_list_impl(state, &patient.to_string(), Some(charts)).await;
    Ok(checked(reread))
}

/// "From:" for a check: `(info_source, source_detail)`. A known preset gives its category and
/// the detail (else the preset's label), as `stamp` gives a list line; anything else gives
/// `unknown` and the detail as typed (ruling R1). Pure.
pub(crate) fn check_source(
    preset: Option<String>,
    detail: Option<String>,
) -> (&'static str, Option<String>) {
    let from = FromView {
        preset: given(preset),
        detail: given(detail),
    };
    match stamp(&from) {
        Some((category, detail)) => (category, Some(detail)),
        None => ("unknown", from.detail),
    }
}

/// The check's source note: its detail and why, `None` when it has neither. Pure.
fn note<'a>(source_detail: Option<&'a str>, why: Option<&'a str>) -> Option<SourceNote<'a>> {
    (source_detail.is_some() || why.is_some()).then_some(SourceNote { source_detail, why })
}

/// What a check that LANDED answers: always an Ok (as `med_chart_write::charted`): the pane read
/// again, or a sentence saying it was signed and how to see it — never an error, which the window
/// would offer to retry and so record it twice. Pure.
pub(crate) fn checked(reread: Result<crate::chart_set::ChartPane, String>) -> MedChartWrite {
    match reread {
        Ok(pane) => MedChartWrite {
            outcome: "Checked: not current.".into(),
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Ruling R1: a known preset gives its category; with none, the source is `unknown` and the
    /// detail is kept as typed, never a category guessed from it.
    #[test]
    fn from_maps_a_preset_and_never_guesses_one() {
        assert_eq!(
            check_source(Some("Packets / Webster pack seen".into()), None),
            (
                "clinician-observed",
                Some("Packets / Webster pack seen".into())
            )
        );
        assert_eq!(
            check_source(None, Some("rang the pharmacy".into())),
            ("unknown", Some("rang the pharmacy".into()))
        );
        assert_eq!(check_source(None, Some("  ".into())), ("unknown", None));
    }

    /// A check that landed answers Ok even when the pane could not be read again: an error would
    /// offer a retry, and a retry records the check twice.
    #[test]
    fn a_landed_check_whose_reread_failed_still_answers_ok() {
        let said = checked(Err("the lock was lost".into()));
        assert!(said.pane.is_none());
        assert!(
            said.outcome.starts_with("Signed") && said.outcome.contains("the lock was lost"),
            "{}",
            said.outcome
        );
    }
}
