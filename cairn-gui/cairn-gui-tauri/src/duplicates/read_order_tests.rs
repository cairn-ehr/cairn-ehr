//! The banner and the tray read the node's check state BEFORE the open pairs (PR #788 review).
//!
//! Why the order matters: the matcher worker is a separate process, and it never deletes a notice
//! before the proposals that notice caused are committed — in the same transaction for one
//! chart's check (`worker.check_chart`), in a later one for a bulk sweep (`worker.run_bulk`; see
//! `queue_db.clear_notices`). The window's reads are separate autocommit statements, which the
//! `state.db` lock does not shield from those commits. Read the pairs first and a proposal
//! committed between the two reads is in neither: the pairs read saw no pair, the status read sees
//! nothing waiting — the banner shows no entry and no "running" line, which reads as "checked,
//! none open" (#742's whole promise) while an open pair exists.
//!
//! Read the check state first and the window is closed: "nothing waiting" at that moment means
//! every proposal had already been committed, so the later pairs read sees it; "running" then
//! means the line shows, and a pair appearing beside it is a harmless overlap.
//!
//! No GUI test reaches a database (#738), so this reads the source, as `unlink_tests.rs` does.

use crate::source_guard::{position, rust_fn};

#[test]
fn the_banner_reads_the_check_state_before_the_pairs() {
    let banner = rust_fn(include_str!("mod.rs"), "duplicate_section");
    let pairs = position(banner, "possible_duplicates(");
    assert!(
        position(banner, "chart_check_pending(") < pairs,
        "each chart's own check is read before the pairs"
    );
    assert!(
        position(banner, "read_snapshot(") < pairs,
        "the node's status is read before the pairs"
    );
}

/// The tray's count has the same race: a count of none read before a proposal commits, then a
/// status of "nothing waiting" read after it, would say "none open" over an open pair.
#[test]
fn the_tray_reads_the_status_before_the_count() {
    let tray = rust_fn(include_str!("../worklist/mod.rs"), "tray_count_impl");
    assert!(position(tray, "read_snapshot(") < position(tray, "worklist_count("));
}
