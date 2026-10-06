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
pub mod view;

use crate::chart_set::{check_displayed_set, member_line, MemberLine, CHANGED};
use crate::commands::read_chart_of;
use crate::funnel::view::{ErrorView, Retry};
use crate::link::chart_set_of;
use crate::link::view::{
    key_locked_for, refused, LinkReportView, ALREADY_IN_RECORD, NOT_ON_SCREEN, OTHER_CHANGED,
    THIS_CHANGED,
};
use crate::state::{AppState, Now};
use cairn_medication_view::ChartSet;
use cairn_node::db_diagnosis::operator_chain;
use cairn_node::duplicate_check::{
    chart_check_pending, classify, read_snapshot, STALLED_AFTER_SECS,
};
use cairn_node::duplicate_review::{self, DifferentPeople};
use uuid::Uuid;
use view::{
    check_lines, different_people_error_view, entry_view, fixture_section, medications_of,
    section_view, ChartCheck, DuplicateSection, PairResult, DIFFERENT_PEOPLE_BUTTON, MAX_SHOWN,
    NOTHING_OPEN, NOT_SHOWN_OR_RESOLVED,
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
    // Phase 1, under ONE lock: the proposals, each shown entry's identities, the checks.
    // `found` is `(how many entries exist, each SHOWN entry WITH its identity read)` — the two
    // travel together as one tuple, so no later step can pair one person's name with another
    // person's medications by index.
    let (found, checks, status) = {
        let db = db.lock().await;
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
                shown.push(entry_view(entry.review_chart, entry.vetoed, ids, meds));
            }
            Ok((shown, total))
        }
    };
    section_view(entries, check_lines(opened, &checks, members, status))
}

/// Review's admission for the compare panel (`link::resolve_pair`): a chart a list on screen
/// showed (`AppState::shown`, unchanged), OR one an open proposal joins to `left`'s record at
/// this moment — the banner showed it. `shown` is deliberately NOT widened (design "R5a"): a
/// pair a colleague resolved a second ago is refused here, never silently compared. Returns the
/// name the list showed, or `None` for a banner admission (no list showed a name; only fixture
/// mode reads it, and fixture mode has no proposals to admit by).
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
/// The rules, IN THIS ORDER (as `link::link_impl`'s): the chart on screen; this record's set is
/// the one compared (`THIS_CHANGED`); the other chart id is well-formed (`NOT_ON_SCREEN`, a
/// window fault); the other chart is not already in this record (`ALREADY_IN_RECORD` — never
/// "already judged", which would claim a colleague's act); the other record's set is the one
/// compared (`OTHER_CHANGED` — the rule that keeps the signature to the charts compared);
/// fixture mode; the key. Then the node judges every pair still open between the two records,
/// read fresh — `NothingOpen` means a colleague got there first and nothing was signed. Unlike
/// Link there is no Review admission: that fresh open-pair read stands in for it.
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
    match outcome {
        DifferentPeople::NothingOpen => Err(refused(NOTHING_OPEN)),
        DifferentPeople::Judged(judged) => view::different_people_report(
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
mod tests {
    use super::*;
    use cairn_patient_search::{Candidate, TrustState};

    fn fixture() -> Uuid {
        cairn_gui_data::mock::fixtures::FIXTURE_UUID
            .parse()
            .unwrap()
    }

    #[tokio::test]
    async fn fixture_mode_shows_no_entries_and_says_no_check_ran() {
        let state = AppState::mock(Some(fixture()));
        let s = duplicate_section(&state, fixture(), &ChartSet::single(fixture()), &[]).await;
        assert_eq!(s, view::fixture_section());
    }

    #[tokio::test]
    async fn a_shown_chart_is_admitted_by_its_list() {
        let state = AppState::mock(Some(fixture()));
        let other = Uuid::from_u128(2);
        state.shown.lock().await.insert(
            other,
            Candidate {
                patient_id: other,
                display_name: "Other Person".into(),
                age: None,
                trust: TrustState::Confirmed,
                last_activity: None,
                locale: None,
                photo_ref: None,
            },
        );
        let name = admit_other(&state, &ChartSet::single(fixture()), other)
            .await
            .unwrap();
        assert_eq!(name.as_deref(), Some("Other Person"));
    }

    /// Fixture mode has no proposals, so an unshown chart keeps today's refusal word for word.
    #[tokio::test]
    async fn in_fixture_mode_an_unshown_chart_keeps_the_not_on_screen_refusal() {
        let state = AppState::mock(Some(fixture()));
        let err = admit_other(&state, &ChartSet::single(fixture()), Uuid::from_u128(2))
            .await
            .unwrap_err();
        assert_eq!(err.text, crate::link::view::NOT_ON_SCREEN);
    }

    /// Final review M5: a chart already in this record is refused BEFORE Review's admission is
    /// asked — no list showed it here, so admitting first would refuse it as "not on screen"
    /// (and, live, spend a proposal read on a chart that can never be compared).
    #[tokio::test]
    async fn an_in_record_chart_is_refused_before_any_admission() {
        let state = AppState::mock(Some(fixture()));
        let p = fixture().to_string();
        let err = crate::link::compare_impl(&state, &p, vec![p.clone()], &p)
            .await
            .unwrap_err();
        assert_eq!(err.text, crate::link::view::ALREADY_IN_RECORD);
    }

    /// Final review M4c: a malformed other chart id is a window fault ("not on screen", as
    /// `link_impl` words it) — never "already judged", which would tell the clinician a colleague
    /// resolved something nobody touched.
    #[tokio::test]
    async fn a_malformed_other_chart_is_not_on_screen() {
        let state = AppState::mock(Some(fixture()));
        let p = fixture().to_string();
        let err = different_people_impl(&state, &p, vec![p.clone()], "not-a-uuid", vec![])
            .await
            .unwrap_err();
        assert_eq!(err.text, crate::link::view::NOT_ON_SCREEN);
    }

    #[tokio::test]
    async fn different_people_is_bound_to_the_chart_on_screen() {
        let state = AppState::mock(Some(fixture()));
        let err =
            different_people_impl(&state, &Uuid::from_u128(9).to_string(), vec![], "x", vec![])
                .await
                .unwrap_err();
        assert!(err.text.contains("not the chart"), "{}", err.text);
    }

    #[tokio::test]
    async fn different_people_refuses_a_changed_set() {
        let state = AppState::mock(Some(fixture()));
        let p = fixture().to_string();
        let err = different_people_impl(
            &state,
            &p,
            vec![p.clone(), Uuid::from_u128(7).to_string()],
            &Uuid::from_u128(2).to_string(),
            vec![],
        )
        .await
        .unwrap_err();
        assert_eq!(err.text, crate::link::view::THIS_CHANGED);
    }

    #[tokio::test]
    async fn fixture_mode_cannot_record_different_people() {
        let state = AppState::mock(Some(fixture()));
        let p = fixture().to_string();
        let other = Uuid::from_u128(2).to_string();
        let err = different_people_impl(&state, &p, vec![p.clone()], &other, vec![other.clone()])
            .await
            .unwrap_err();
        assert!(err.text.contains("fixture mode"), "{}", err.text);
    }

    /// Test review gap 1: the other record changed between the comparison and the press. This is
    /// the one rule that keeps the signature to the charts the clinician compared — so it comes
    /// BEFORE fixture mode, as in `link_impl`, where a fixture-mode test can reach it.
    #[tokio::test]
    async fn different_people_refuses_when_the_other_record_changed() {
        let state = AppState::mock(Some(fixture()));
        let p = fixture().to_string();
        let other = Uuid::from_u128(2);
        let err = different_people_impl(
            &state,
            &p,
            vec![p.clone()],
            &other.to_string(),
            vec![other.to_string(), Uuid::from_u128(4).to_string()],
        )
        .await
        .unwrap_err();
        assert_eq!(err.text, crate::link::view::OTHER_CHANGED);
    }

    /// Type review finding 3: a chart already in this record is "already in this record", as
    /// Link says it — never "already judged or resolved", which claims a colleague's act.
    #[tokio::test]
    async fn different_people_refuses_a_chart_already_in_this_record() {
        let state = AppState::mock(Some(fixture()));
        let p = fixture().to_string();
        let err = different_people_impl(&state, &p, vec![p.clone()], &p, vec![p.clone()])
            .await
            .unwrap_err();
        assert_eq!(err.text, crate::link::view::ALREADY_IN_RECORD);
    }

    /// duplicates.js is untyped: a Rust field rename would not break the build, it would draw
    /// an empty banner. Every field it reads must be one the backend sends.
    #[test]
    fn duplicates_js_reads_no_field_the_backend_does_not_send() {
        use crate::commands::tests::fields_read_in;
        let js = include_str!("../../src-ui/duplicates.js");
        let entry = view::entry_view(Uuid::from_u128(9), true, Ok(vec![]), Err("x".into()));
        let keys = |v: serde_json::Value| -> std::collections::BTreeSet<String> {
            v.as_object().unwrap().keys().cloned().collect()
        };
        for (binding, available) in [
            (
                "section",
                keys(serde_json::to_value(DuplicateSection::checked_none_open()).unwrap()),
            ),
            ("entry", keys(serde_json::to_value(&entry).unwrap())),
        ] {
            let read = fields_read_in(js, binding);
            assert!(
                !read.is_empty(),
                "duplicates.js no longer reads `{binding}` — rename it here"
            );
            for field in read {
                assert!(
                    available.contains(&field),
                    "duplicates.js reads `{binding}.{field}`, not sent"
                );
            }
        }
    }
}
