//! The person menu's commands (sign-in slice 3c-3c): "what is waiting for me?", the picker's
//! counts, and "Open note". Thin adapters, as `notes.rs`'s are: each `#[tauri::command]` forwards
//! to a plain `*_impl(&AppState, …)` tested against `AppState::mock`. Every sentence is made in
//! `person_menu_view.rs`; every classification rule lives in `cairn_node::note::menu`.
//!
//! Three rules hold here:
//! * the menu belongs to the person who-is-here NAMES; "not asked" and "don't know" get no menu
//!   and no database read;
//! * a menu is a list on screen, so its rows' charts are admitted to `AppState::shown` and a chart
//!   opens only through `open_chart_impl` (the wrong-chart rule is unchanged);
//! * each command reads in ONE snapshot, so a count agrees with its own list; the bar and the
//!   picker are two reads, and agree only while nothing changes between them (ADR-0095 decision 6).
use crate::funnel::commands::open_chart_impl;
use crate::funnel::view::{ChartHeaderView, ErrorView};
use crate::person_menu_view::{
    counts_failed_view, counts_view, menu_failed_view, menu_view, PersonMenuView, StaffCountView,
    NO_LONGER_WAITS,
};
use crate::state::AppState;
use crate::whos_here::Claim;
use crate::write_failure::chart_read_failed;
use cairn_node::note::menu::PersonMenu;
use cairn_node::note::menu_read::{read_menus, MenuRead};
use cairn_patient_search::Candidate;
use chrono::Utc;
use std::collections::HashMap;
use uuid::Uuid;

/// The named person's staff id, or `None` for "not asked" / "don't know".
async fn named(state: &AppState) -> Option<String> {
    match state.whos_here.lock().await.claim() {
        Claim::Named { staff_id, .. } => Some(staff_id.clone()),
        Claim::NotAsked | Claim::Unknown => None,
    }
}

/// What one snapshot read yields: the read, the person's groups, and a candidate per row chart.
struct Fresh {
    /// The one instant this command classified and words by.
    now: chrono::DateTime<Utc>,
    read: MenuRead,
    menu: PersonMenu,
    patients: HashMap<Uuid, Candidate>,
}

/// Read `staff`'s menu and name every row's chart, in ONE snapshot; admit those charts to
/// `state.shown`. A staff id the node cannot place reads as an empty menu.
async fn fresh(
    state: &AppState,
    db: &cairn_gui_live::NodeConnection,
    staff: &str,
) -> Result<Fresh, ErrorView> {
    let now = Utc::now();
    let today = now.with_timezone(&chrono::Local).date_naive().to_string();
    let mut db = db.lock().await?;
    let (read, menu, found) = cairn_node::medication::snapshot::in_snapshot(&mut db, async |tx| {
        let read = read_menus(tx, Some(staff), now).await?;
        let menu = read.menu_for(staff, now).unwrap_or_default();
        let found =
            cairn_node::patient::candidate_read::candidates_by_id(tx, &menu.charts(), &today)
                .await?;
        Ok((read, menu, found))
    })
    .await
    .map_err(|e| {
        let cause = db.explain(e);
        eprintln!("waiting list: {cause:#}");
        menu_failed_view(chart_read_failed(cause))
    })?;
    drop(db);
    let patients: HashMap<Uuid, Candidate> = found.into_iter().map(|c| (c.patient_id, c)).collect();
    state.shown.lock().await.extend(patients.clone());
    Ok(Fresh {
        now,
        read,
        menu,
        patients,
    })
}

/// The bar's menu. `--mock` (no database): empty when somebody is named.
pub async fn person_menu_impl(state: &AppState) -> Result<PersonMenuView, ErrorView> {
    let Some(staff) = named(state).await else {
        return Ok(PersonMenuView::Nobody);
    };
    let tz = chrono::Local;
    let Some(db) = state.db.as_ref() else {
        let read = MenuRead {
            people: vec![],
            drafts: vec![],
            parts: vec![],
        };
        return Ok(menu_view(
            &PersonMenu::default(),
            &read,
            &HashMap::new(),
            &tz,
            Utc::now(),
        ));
    };
    let f = fresh(state, db, &staff).await?;
    Ok(menu_view(&f.menu, &f.read, &f.patients, &tz, f.now))
}

/// "Open note": re-read the menu; only a chart a row there still names is opened.
pub async fn person_menu_open_impl(
    state: &AppState,
    patient_id: &str,
) -> Result<ChartHeaderView, String> {
    let (Some(staff), Some(db)) = (named(state).await, state.db.as_ref()) else {
        return Err(NO_LONGER_WAITS.into());
    };
    let f = fresh(state, db, &staff).await.map_err(|e| e.text)?;
    if !f.menu.charts().iter().any(|c| c.to_string() == patient_id) {
        return Err(NO_LONGER_WAITS.into());
    }
    open_chart_impl(state, patient_id).await
}

/// The picker's counts: one read for everybody. `--mock` answers none.
pub async fn whos_here_counts_impl(state: &AppState) -> Result<Vec<StaffCountView>, ErrorView> {
    let Some(db) = state.db.as_ref() else {
        return Ok(vec![]);
    };
    let now = Utc::now();
    let mut db = db.lock().await?;
    let read = cairn_node::medication::snapshot::in_snapshot(&mut db, async |tx| {
        read_menus(tx, None, now).await
    })
    .await
    .map_err(|e| {
        let cause = db.explain(e);
        eprintln!("waiting counts: {cause:#}");
        counts_failed_view(chart_read_failed(cause))
    })?;
    Ok(counts_view(read.counts(now)))
}

/// The bar's menu, for the webview: forwards to [`person_menu_impl`].
#[tauri::command]
pub async fn person_menu(state: tauri::State<'_, AppState>) -> Result<PersonMenuView, ErrorView> {
    person_menu_impl(&state).await
}

/// "Open note", for the webview: forwards to [`person_menu_open_impl`].
#[tauri::command]
pub async fn person_menu_open(
    state: tauri::State<'_, AppState>,
    patient_id: String,
) -> Result<ChartHeaderView, String> {
    person_menu_open_impl(&state, &patient_id).await
}

/// The picker's counts, for the webview: forwards to [`whos_here_counts_impl`].
#[tauri::command]
pub async fn whos_here_counts(
    state: tauri::State<'_, AppState>,
) -> Result<Vec<StaffCountView>, ErrorView> {
    whos_here_counts_impl(&state).await
}

#[cfg(test)]
#[path = "person_menu_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "person_menu_db_tests.rs"]
mod db_tests;
