//! The possible-duplicate banner (repair path R5a, #680; design page "R5a — the banner,
//! designed 2026-10-06"): the section `med_list` carries, Review's admission, and the
//! "Different people" command. The banner's and the judgement's sentences are in `view.rs`
//! (golden-tested); only the commands' own read-failure and fixture-mode refusals are worded
//! here. Every DB rule is in `cairn_node::duplicate_review` (DB-tested there). This module only
//! orders the reads and applies the chart-command rules.
//!
//! LOCKING: `state.db` is a `tokio::sync::Mutex`, which is NOT re-entrant, and
//! `read_chart_of` / `chart_set_of` take it themselves. So [`duplicate_section`] reads in two
//! phases — everything else under one lock, then each entry's medications after releasing it —
//! and nothing here calls either helper while holding the lock (that would deadlock the window).
pub mod verdict;
pub mod view;

use crate::chart_set::{check_displayed_set, linked_members, member_line, MemberLine, CHANGED};
use crate::commands::read_chart_of;
use crate::funnel::view::{ErrorView, Retry};
use crate::link::chart_set_of;
use crate::link::judgement_view::{key_locked_for, refused, LinkReportView};
use crate::link::view::{ALREADY_IN_RECORD, NOT_ON_SCREEN, OTHER_CHANGED, THIS_CHANGED};
use crate::state::{AppState, Now};
use cairn_medication_view::ChartSet;
use cairn_node::db_diagnosis::operator_chain;
use cairn_node::duplicate_check::{
    chart_check_pending, classify, read_snapshot, STALLED_AFTER_SECS,
};
use cairn_node::duplicate_review::{self, DifferentPeople};
use uuid::Uuid;
use verdict::{different_people_error_view, PairResult};
use view::{
    check_lines, entry_view, fixture_section, medications_of, section_view, unread_set_section,
    ChartCheck, DuplicateSection, EntryFlags, ACCEPTED_NOT_OVERRULED, DIFFERENT_PEOPLE_BUTTON,
    MAX_SHOWN, NOTHING_OPEN, NOT_SHOWN_OR_RESOLVED,
};

/// The banner for the displayed record. Never fails: every failure is a worded line (an absent
/// banner must mean "checked, none open"). `members` are the record's member lines, already
/// read for the header — used only to name a pending member.
pub async fn duplicate_section(
    state: &AppState,
    opened: Uuid,
    charts: &ChartSet,
    members: &[MemberLine],
) -> DuplicateSection {
    let Some(db) = state.db.as_ref() else {
        return fixture_section();
    };
    // Phase 1, under ONE lock: the checks and the node's status, THEN the proposals and each
    // shown entry's identities. `found` is `(how many entries exist, each SHOWN entry WITH its
    // identity read)` — the two travel together as one tuple, so no later step can pair one
    // person's name with another person's medications by index.
    //
    // The ORDER is load-bearing (PR #788 review; pinned by `read_order_tests.rs`). The matcher
    // worker, from its own process, never deletes a notice before the proposals it caused are
    // committed — and either can land between any two of these autocommit reads. Read the proposals first and such a pair falls
    // between them: no entry, and a status of "nothing waiting" — a silent "checked, none open".
    // Read the check state first and "nothing waiting" proves every proposal was already in.
    let (found, checks, status) = {
        let db = db.lock().await;
        let mut checks = vec![];
        for chart in charts.members() {
            checks.push(ChartCheck {
                chart: *chart,
                pending: chart_check_pending(&db, *chart)
                    .await
                    .map_err(|e| operator_chain(&e)),
            });
        }
        let status = read_snapshot(&db)
            .await
            .map(|s| classify(&s, STALLED_AFTER_SECS))
            .map_err(|e| operator_chain(&e));
        let found = match duplicate_review::possible_duplicates(&*db, charts).await {
            Err(e) => Err(operator_chain(&e)),
            Ok(entries) => {
                let total = entries.len();
                let mut shown = vec![];
                for entry in entries.into_iter().take(MAX_SHOWN) {
                    let ids =
                        cairn_node::patient::person::chart_identities(&*db, &entry.other_record)
                            .await
                            .map(|ids| ids.iter().map(member_line).collect::<Vec<_>>())
                            .map_err(|e| operator_chain(&e));
                    shown.push((entry, ids));
                }
                Ok((total, shown))
            }
        };
        (found, checks, status)
    };
    // The lock is released here: `read_chart_of` below takes it again.
    // Phase 2: each shown entry's medications through the SAME read opening that chart gives
    // (§5.9 custody and sealing unchanged) — drawn only if read over the set the identities name.
    let entries = match found {
        Err(e) => Err(e),
        Ok((total, found)) => {
            let mut shown = vec![];
            for (entry, ids) in found {
                let meds = medications_of(
                    &entry.other_record,
                    read_chart_of(state, entry.review_chart).await,
                );
                shown.push(entry_view(
                    entry.review_chart,
                    EntryFlags {
                        vetoed: entry.vetoed,
                        accepted: entry.accepted,
                        disputed: entry.disputed,
                    },
                    ids,
                    meds,
                ));
            }
            Ok((shown, total))
        }
    };
    section_view(entries, check_lines(opened, &checks, members, status))
}

/// The banner ALONE, for the open chart whose medication list could not be read (#745 points 1
/// and 2; the webview's `redrawBannerAlone`, called from `main.js`'s `refresh` on a failed read).
///
/// The banner needs only the record's chart set, never its medication list, so a failed list
/// read must not take the duplicate warning down with it — and after a judgement, a failed
/// re-read must not leave the judged pair on screen with a live Review. This resolves the set
/// itself and then draws exactly what a chart open draws ([`duplicate_section`]); a set that
/// cannot be read is a worded banner ([`view::unread_set_section`]), never a hidden one.
/// Bound to the chart on screen (`AppState::displayed_patient`) like every chart command.
pub async fn banner_alone_impl(
    state: &AppState,
    patient_id: &str,
) -> Result<DuplicateSection, String> {
    let patient = state.displayed_patient(patient_id).await?;
    let charts = match record_charts(state, patient).await {
        Ok(charts) => charts,
        Err(e) => return Ok(unread_set_section(e)),
    };
    // As `med_list_impl` does: an unread member list only loses the pending member's name.
    let members = linked_members(state, &charts).await;
    Ok(duplicate_section(state, patient, &charts, members.as_deref().unwrap_or(&[])).await)
}

/// The chart set `patient` reads over, or why it could not be read (operator chain). Fixture mode
/// has no links, so a fixture chart is a record of one. Takes `state.db` itself — not held here.
async fn record_charts(state: &AppState, patient: Uuid) -> Result<ChartSet, String> {
    let Some(db) = state.db.as_ref() else {
        return Ok(ChartSet::single(patient));
    };
    let db = db.lock().await;
    cairn_node::patient::person::person_charts(&*db, patient)
        .await
        .map_err(|e| operator_chain(&e))
}

#[tauri::command]
pub async fn duplicate_banner(
    state: tauri::State<'_, AppState>,
    patient_id: String,
) -> Result<DuplicateSection, String> {
    banner_alone_impl(&state, &patient_id).await
}

/// Review's admission for the compare panel (`link::same_person::resolve_pair`): a chart a list
/// on screen showed (`AppState::shown`, unchanged), OR one an open proposal joins to `left`'s
/// record at this moment — the banner showed it. `shown` is deliberately NOT widened (design
/// "R5a"): a pair a colleague resolved a second ago is refused here, never silently compared.
/// Returns the name the list showed, or `None` for a banner admission (no list showed a name;
/// only fixture mode reads it, and fixture mode has no proposals to admit by).
pub(crate) async fn admit_other(
    state: &AppState,
    left: &ChartSet,
    other: Uuid,
) -> Result<Option<String>, ErrorView> {
    let shown = state
        .shown
        .lock()
        .await
        .get(&other)
        .map(|c| c.display_name.clone());
    if let Some(name) = shown {
        return Ok(Some(name));
    }
    let Some(db) = state.db.as_ref() else {
        return Err(refused(NOT_ON_SCREEN)); // fixture mode has no proposals
    };
    let right = chart_set_of(state, other).await?; // takes the lock itself — not held here
    let db = db.lock().await;
    let pairs = duplicate_review::open_pairs_between(&*db, left, &right)
        .await
        .map_err(|e| ErrorView {
            text: format!(
                "Could not read whether that chart is an open possible duplicate of this record \
                 — nothing was done: {}",
                operator_chain(&e)
            ),
            retry: Retry::Now,
        })?;
    if pairs.is_empty() {
        Err(refused(NOT_SHOWN_OR_RESOLVED))
    } else {
        Ok(None)
    }
}

/// "Different people — not the same person" (R5a; offered only on a banner's comparison).
///
/// The rules, IN THIS ORDER (as `link::same_person::link_impl`'s): the chart on screen; this
/// record's set is the one compared (`THIS_CHANGED`); the other chart id is well-formed
/// (`NOT_ON_SCREEN`, a window fault); the other chart is not already in this record
/// (`ALREADY_IN_RECORD` — never "already judged", which would claim a colleague's act); the other
/// record's set is the one compared (`OTHER_CHANGED` — the rule that keeps the signature to the
/// charts compared); fixture mode; the key. Then the node judges every pair still open between
/// the two records, read fresh — `NothingOpen` means a colleague got there first and nothing was
/// signed. Unlike Link there is no Review admission: that fresh open-pair read stands in for it.
///
/// Pinned by the tests below: every rule up to and including fixture mode (fixture mode is
/// where they can be reached). The locked key and `admit_other`'s live proposal branch sit past
/// fixture mode, and this crate has no DB-gated tests: they have NO automated or scripted
/// coverage at the window layer (RUNBOOK §11 exercises neither). The node functions they call
/// are DB-tested in cairn-node's `duplicate_review` suite; a window-level live-DB harness is
/// issue #738.
pub async fn different_people_impl(
    state: &AppState,
    patient_id: &str,
    charts: Vec<String>,
    other_id: &str,
    other_charts: Vec<String>,
) -> Result<LinkReportView, ErrorView> {
    let patient = state.displayed_patient(patient_id).await.map_err(refused)?;
    let left = check_displayed_set(&chart_set_of(state, patient).await?, &charts).map_err(|e| {
        if e == CHANGED {
            refused(THIS_CHANGED)
        } else {
            refused(e)
        }
    })?;
    let other: Uuid = other_id.parse().map_err(|_| refused(NOT_ON_SCREEN))?;
    if left.contains(&other) {
        return Err(refused(ALREADY_IN_RECORD));
    }
    let right =
        check_displayed_set(&chart_set_of(state, other).await?, &other_charts).map_err(|e| {
            if e == CHANGED {
                refused(OTHER_CHANGED)
            } else {
                refused(e)
            }
        })?;
    if state.is_mock() {
        return Err(refused(
            "fixture mode: this window is showing mock data and cannot write",
        ));
    }
    let (human_sk, human_kid) = state
        .live_key(Now::read())
        .await
        .ok_or_else(|| key_locked_for(DIFFERENT_PEOPLE_BUTTON))?;
    let mut db = state
        .db
        .as_ref()
        .ok_or_else(|| refused("no database connection"))?
        .lock()
        .await;
    let reviewer = cairn_node::chart_link::Reviewer {
        human_sk: &human_sk,
        human_kid: &human_kid,
    };
    let outcome = duplicate_review::record_different_people(
        &mut db,
        &left,
        &right,
        &reviewer,
        &state.node_origin,
    )
    .await
    .map_err(|e| ErrorView {
        text: format!(
            "Could not read whether this possible duplicate is still open — nothing was done: {}",
            operator_chain(&e)
        ),
        retry: Retry::Now,
    })?;
    different_people_view(outcome)
}

/// The window's reading of a "Different people" outcome. **Pure**, so the refusals are
/// testable without a database. `AcceptedAsSame` (#736): a human already accepted the pair as
/// the same person, so this judgement would silently overrule them; nothing was written.
fn different_people_view(outcome: DifferentPeople) -> Result<LinkReportView, ErrorView> {
    match outcome {
        DifferentPeople::NothingOpen => Err(refused(NOTHING_OPEN)),
        DifferentPeople::AcceptedAsSame => Err(refused(ACCEPTED_NOT_OVERRULED)),
        DifferentPeople::Judged(judged) => verdict::different_people_report(
            judged
                .into_iter()
                .map(|j| PairResult {
                    low: j.low,
                    high: j.high,
                    outcome: j
                        .outcome
                        .map(|o| o.effect)
                        .map_err(|e| different_people_error_view(&e)),
                })
                .collect(),
        ),
    }
}

#[tauri::command]
pub async fn record_different_people(
    state: tauri::State<'_, AppState>,
    patient_id: String,
    charts: Vec<String>,
    other_id: String,
    other_charts: Vec<String>,
) -> Result<LinkReportView, ErrorView> {
    different_people_impl(&state, &patient_id, charts, &other_id, other_charts).await
}

#[cfg(test)]
mod read_order_tests;

#[cfg(test)]
mod tests;
