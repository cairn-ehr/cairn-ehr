//! Batch charting slice 2a, final review: the webview half of a line's source after a save
//! (`med_list.js`'s `sendLine`, `med_list_versions.js`'s `takeStamp` and `sourceChosen`). Beside
//! `med_list_versions_webview_tests.rs` (same rig: the scripts' source, pinned) for its size cap.
//! Both behaviours were also walked with the functions run under a stubbed `invoke` (report).
use crate::med_list_webview_tests::LIST_JS;
use crate::source_guard::{js_fn, position};

const VERSIONS_JS: &str = include_str!("../src-ui/med_list_versions.js");

/// Ruling R23: the save that stamps "From:" on a new line answers the stamp, and the window puts
/// it on the line as soon as that save answers `saved` — so the line's next save carries the
/// source even when no read lands between the two (a read is not awaited). A source changed here
/// after the save was sent is the clinician's newer choice and is kept.
#[test]
fn a_stamp_reaches_the_line_with_its_save_answer() {
    let send = js_fn(LIST_JS, "sendLine");
    let saved = position(send, "if (door.answer === \"saved\") {");
    let took = position(send, "takeStamp(line, door, sent);");
    let refused = position(send, "} else {");
    assert!(saved < took && took < refused, "{send}");
    let take = js_fn(VERSIONS_JS, "takeStamp");
    for step in [
        "if (!door.info_source) return;",
        "if (own.info_source > sent.version || own.source_detail > sent.version) return;",
        "line.content.info_source = door.info_source;",
        "line.content.source_detail = door.source_detail;",
    ] {
        assert!(take.contains(step), "{step}: {take}");
    }
}

/// Review M3: "not set" picked on a line's own source clears it — category and detail, the box
/// emptied too — so a stale stamp can be taken off; the press's default then stands at Sign. A
/// preset sends its category and the detail (the free text, or the label when blank, as Rust's
/// `stamp`); free text typed under "not set" is a detail with no category, as before.
#[test]
fn not_set_clears_the_lines_own_source() {
    let chosen = js_fn(VERSIONS_JS, "sourceChosen");
    let cleared = position(chosen, "if (!label && picked) r.own.detail.value = \"\";");
    let read = position(chosen, "const text = r.own.detail.value.trim();");
    let sent = position(
        chosen,
        "ownChanged(line, { info_source: FROM_PRESETS[label] || null, source_detail: text || label || null });",
    );
    assert!(cleared < read && read < sent, "{chosen}");
    // Picking from the list is a pick ("not set" included); typing the detail is not.
    let own = js_fn(VERSIONS_JS, "versionControls");
    assert!(
        own.contains("preset.addEventListener(\"change\", () => sourceChosen(line, true));"),
        "{own}"
    );
    assert!(
        own.contains("detail.addEventListener(\"input\", () => sourceChosen(line, false));"),
        "{own}"
    );
}
