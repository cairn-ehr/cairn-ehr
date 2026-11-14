//! Tests for `chart_stat::attach_stat` beyond the webview's (`med_stat_webview_tests.rs`).
use super::attach_stat;
use cairn_gui_tab_medications::{build_view, MedListRowView};
use cairn_medication_view::fixtures::sample_chart;
use cairn_medication_view::{MedicationStatus, PatientMedicationList};
use cairn_node::medication::stat_read::{Give, StatOrder};
use chrono::{Local, TimeZone, Utc};
use uuid::Uuid;

/// A stat order on thread `thread` of the opened chart 1, with one standing give when `given`.
fn order(thread: u128, given: bool) -> StatOrder {
    StatOrder {
        medication_id: Uuid::from_u128(thread),
        patient_id: Uuid::from_u128(1),
        ordered_at: Utc.with_ymd_and_hms(2026, 11, 13, 22, 0, 0).unwrap(),
        orderer: Some("Dr Ann Lee".into()),
        gives: given
            .then(|| Give {
                event_id: Uuid::from_u128(900 + thread),
                given_at: Utc.with_ymd_and_hms(2026, 11, 13, 22, 20, 0).unwrap(),
                giver: Some("RN Bo".into()),
                completes: true,
                struck: None,
            })
            .into_iter()
            .collect(),
    }
}

/// The reconciled row of `chart` (threads 50 and 51), as shown with `orders` attached.
fn reconciled_row(chart: &PatientMedicationList, orders: &[StatOrder]) -> MedListRowView {
    let mut shown = build_view(chart).rows;
    attach_stat(
        &mut shown,
        &chart.rows,
        orders,
        Uuid::from_u128(1),
        &Local::now(),
    );
    let at = chart
        .rows
        .iter()
        .position(|r| r.members.len() == 2 && r.members[0].medication_id.as_u128() == 50)
        .expect("the reconciled row");
    shown.swap_remove(at)
}

/// Two stat orders under ONE row (reconciled duplicates, ADR-0047: the same stat order charted at
/// two terminals), the WAITING one read first. The row shows the GIVEN one and offers no Given:
/// Rust re-derives "waiting" per thread, so a Given on the waiting thread would be accepted and
/// the dose given twice.
#[test]
fn two_stat_orders_under_one_row_show_the_given_one_and_offer_no_given() {
    let orders = [order(50, false), order(51, true)];
    let chart = sample_chart();
    let row = reconciled_row(&chart, &orders);
    let stat = row.stat.expect("a stat line");
    assert_eq!(stat.medication_id, Uuid::from_u128(51).to_string());
    assert!(!stat.can_give, "{stat:?}");
    assert!(
        stat.line.contains(" · given ") && stat.line.ends_with(" RN Bo"),
        "{}",
        stat.line
    );
    assert_eq!(stat.gives.len(), 1);
    // Stopped by its give: the row reads "given".
    let mut stopped = sample_chart();
    for r in &mut stopped.rows {
        if r.members.iter().any(|m| m.medication_id.as_u128() == 51) {
            r.status = MedicationStatus::Ceased;
        }
    }
    assert_eq!(reconciled_row(&stopped, &orders).status_label, "given");
    // Neither given: the first read, offered Given on a current row.
    let both_waiting = reconciled_row(&chart, &[order(50, false), order(51, false)]);
    let stat = both_waiting.stat.expect("a stat line");
    assert_eq!(stat.medication_id, Uuid::from_u128(50).to_string());
    assert!(stat.can_give);
}

/// Two standing gives (given at once from two terminals) each offer "Entered in error"; a
/// screen-reader user must tell the two buttons apart, so each give carries its own words (time
/// given and giver, `give_words`), which `med_stat.js` puts in its button's accessible name.
#[test]
fn two_standing_gives_strike_buttons_are_named_apart() {
    let mut o = order(10, true);
    let mut later = o.gives[0].clone();
    later.event_id = Uuid::from_u128(971);
    later.given_at += chrono::TimeDelta::minutes(10);
    later.giver = Some("RN Cy".into());
    o.gives.push(later);
    let mut shown = build_view(&sample_chart()).rows;
    attach_stat(
        &mut shown,
        &sample_chart().rows,
        &[o],
        Uuid::from_u128(1),
        &Local::now(),
    );
    let stat = shown
        .iter()
        .find_map(|r| r.stat.as_ref())
        .expect("a stat line");
    let words: Vec<_> = stat.gives.iter().map(|g| g.words.as_str()).collect();
    assert!(
        words[0].starts_with("given ") && words[0].ends_with(" RN Bo"),
        "{words:?}"
    );
    assert!(
        words[1].ends_with(" RN Cy") && words[0] != words[1],
        "{words:?}"
    );
}
