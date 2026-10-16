//! The repair path's gestures on a combined record (ADR-0076).
//!
//! "Same person as…" (R2b-1) compares two records side by side and links them:
//! `same_person.rs`. Its sibling, the review panel opened by "Review this link…" / "Review this
//! judgement…" (R2b-2, R6 — undo ONE link of a combined record, or confirm it), lives in
//! `unlink.rs` and `confirm.rs` with its own rule order and its own panel; `record_links.rs` reads
//! the "How these charts are linked" list it opens from. The wording is pure and tested on its
//! own: `view.rs` (the comparison), `judgement_view.rs` (what a judgement did, or why it failed),
//! `unlink_view.rs` / `confirm_view.rs` (the review panel). `search.rs` is the in-chart search.
//!
//! `lock_hint_tests.rs` pins the verdict buttons' lock hint against script load order (#782).
//!
//! This file is the module map plus two helpers the gestures share: `chart_set_of` (which charts
//! a record combines, read live) and `as_text` (a read's error as legible text).
pub mod confirm;
pub mod confirm_view;
pub mod judgement_view;
pub mod record_links;
pub mod same_person;
pub mod search;
pub mod unlink;
pub mod unlink_view;
pub mod view;

#[cfg(test)]
mod lock_hint_tests;

use crate::funnel::view::ErrorView;
use crate::state::AppState;
use cairn_medication_view::ChartSet;
use uuid::Uuid;

/// A read's error as the text a comparison part carries (the operator chain, legible).
/// A plain generic fn, not a closure: it is used at several different `T`s.
pub(crate) fn as_text<T>(r: anyhow::Result<T>) -> Result<T, String> {
    r.map_err(|e| cairn_node::db_diagnosis::operator_chain(&e))
}

/// The record `patient` belongs to: its link component live, itself alone in fixture mode
/// (fixture charts are never linked). A failure here is a READ that failed — nothing was
/// judged — so it is worded as one, retryable, never as a link outcome.
pub(crate) async fn chart_set_of(state: &AppState, patient: Uuid) -> Result<ChartSet, ErrorView> {
    let Some(db) = state.db.as_ref() else {
        return Ok(ChartSet::single(patient));
    };
    let db = db.lock().await;
    cairn_node::patient::person::person_charts(&*db, patient)
        .await
        .map_err(|e| ErrorView {
            text: format!(
                "Could not read which charts this record combines — nothing was done: {}",
                cairn_node::db_diagnosis::operator_chain(&e)
            ),
            retry: crate::funnel::view::Retry::Now,
        })
}
