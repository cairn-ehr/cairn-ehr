//! Charting slice 2, Task 6: the stat line under its drug line, "Given" and "Entered in error"
//! (`med_stat.js`, `chart_stat::attach_stat`).
//!
//! Two halves, as `med_versions_settle_webview_tests.rs`. What the line SAYS, whether it is
//! offered a button and whether the row reads "given" are Rust's (`attach_stat`), tested here on
//! Rust's own answer. What the webview DOES with that answer is pinned on the script's source (no
//! JS test rig: no npm, by design) — and VISIBILITY, not text: each element sits in a row the
//! script draws un-hidden, and no stylesheet rule hides it. The walk itself was also driven
//! headlessly with a stub `invoke`, asserting computed display and hidden ancestors (task report).
use crate::chart_stat::attach_stat;
use crate::commands::tests::fields_read_in;
use crate::med_chart_webview_tests::not_hidden_by_css;
use crate::source_guard::{js_const, js_fn, position};
use cairn_gui_tab_medications::{build_view, GiveView, MedListRowView, StatView};
use cairn_medication_view::fixtures::sample_chart;
use cairn_node::medication::stat_read::{Give, StatOrder, Strike};
use chrono::{Local, TimeZone, Utc};
use uuid::Uuid;

const STAT_JS: &str = include_str!("../src-ui/med_stat.js");
const MAIN_JS: &str = include_str!("../src-ui/main.js");
const INDEX_HTML: &str = include_str!("../src-ui/index.html");

/// The opened chart of `sample_chart` (every member thread lives on it).
fn opened() -> Uuid {
    Uuid::from_u128(1)
}

/// A stat order on thread `thread` of `chart`, charted by Dr Ann Lee, with `gives`.
fn order(thread: u128, chart: Uuid, gives: Vec<Give>) -> StatOrder {
    StatOrder {
        medication_id: Uuid::from_u128(thread),
        patient_id: chart,
        ordered_at: Utc.with_ymd_and_hms(2026, 11, 13, 22, 0, 0).unwrap(),
        orderer: Some("Dr Ann Lee".into()),
        gives,
    }
}

/// A give by RN Bo, after the order, that completed it.
fn give(id: u128, struck: Option<Strike>) -> Give {
    Give {
        event_id: Uuid::from_u128(id),
        given_at: Utc.with_ymd_and_hms(2026, 11, 13, 22, 20, 0).unwrap(),
        giver: Some("RN Bo".into()),
        completes: true,
        struck,
    }
}

/// `sample_chart`'s rows as the pane shows them, with `orders` attached (opened chart 1).
fn pane(orders: &[StatOrder]) -> Vec<MedListRowView> {
    let chart = sample_chart();
    let mut shown = build_view(&chart).rows;
    attach_stat(&mut shown, &chart.rows, orders, opened(), &Local::now());
    shown
}

/// The shown row whose member thread is `thread` (10 is a current line, 40 a stopped one).
fn line_of(shown: &[MedListRowView], thread: u128) -> &MedListRowView {
    let at = sample_chart()
        .rows
        .iter()
        .position(|r| {
            r.members
                .iter()
                .any(|m| m.medication_id.as_u128() == thread)
        })
        .expect("a line with that thread");
    &shown[at]
}

fn stat_of(shown: &[MedListRowView], thread: u128) -> &StatView {
    line_of(shown, thread).stat.as_ref().expect("a stat line")
}

/// No dialog stands anywhere in this script (principle 3).
fn no_dialog() {
    for banned in ["confirm(", "alert(", "prompt(", "<dialog", "showModal"] {
        assert!(!STAT_JS.contains(banned), "{banned}");
    }
}

/// The stat line is a row of its own right under the drug line, drawn un-hidden, and nothing the
/// clinician reads or presses in it is hidden by a stylesheet rule.
fn drawn_under_the_line() {
    let render = js_fn(MAIN_JS, "renderRow");
    let line = position(render, "const rows = [tr];");
    let hook = position(render, "rows.push(...statRows(row));");
    let flags = position(render, "for (const flag of row.flags) {");
    assert!(line < hook && hook < flags, "{render}");
    let rows = js_fn(STAT_JS, "statRows");
    for step in [
        "if (!stat) return [];",
        "tr.setAttribute(\"data-stat\", \"true\");",
        "td.colSpan = 5;",
        "td.append(cell(\"span\", stat.line, { class: \"med-stat-line\" }));",
        "for (const words of stat.struck) td.append(cell(\"span\", words, { class: \"med-stat-struck\" }));",
    ] {
        assert!(rows.contains(step), "{step}: {rows}");
    }
    assert!(!rows.contains(".hidden = true"), "{rows}");
    for class in [
        "tr[data-stat",
        ".med-stat-line",
        ".med-stat-struck",
        ".med-stat-act",
        ".med-stat-said",
        ".med-stat-row",
    ] {
        not_hidden_by_css(class);
    }
    // Loaded after every script whose functions it calls, before the boot.
    let stat = position(INDEX_HTML, "<script src=\"med_stat.js\"></script>");
    for before in ["med_list_sign.js", "med_versions_settle.js"] {
        assert!(
            position(INDEX_HTML, &format!("<script src=\"{before}\"></script>")) < stat,
            "{before}"
        );
    }
    assert!(stat < position(INDEX_HTML, "<script src=\"funnel.js\"></script>"));
}

/// A waiting stat order of the OPENED chart, on a current line, reads "not yet given" and offers
/// "Given", named for its drug, down while a Sign or a list act is in flight.
#[test]
fn a_waiting_stat_line_shows_given() {
    let shown = pane(&[order(10, opened(), vec![])]);
    let stat = stat_of(&shown, 10);
    assert!(stat.can_give && stat.gives.is_empty(), "{stat:?}");
    assert!(
        stat.line.starts_with("stat · ordered ")
            && stat.line.ends_with(" Dr Ann Lee · not yet given"),
        "{}",
        stat.line
    );
    assert_eq!(stat.medication_id, Uuid::from_u128(10).to_string());
    assert_eq!(line_of(&shown, 10).status_label, "current");
    assert!(
        line_of(&shown, 20).stat.is_none(),
        "never under another line"
    );
    assert_eq!(js_const(STAT_JS, "GIVE"), "Given");
    let rows = js_fn(STAT_JS, "statRows");
    assert!(
        rows.contains("if (stat.can_give) td.append(statButton(GIVE, row, (event) => openStat(\"give\", stat, null, row, event.target)));"),
        "{rows}"
    );
    let button = js_fn(STAT_JS, "statButton");
    for step in [
        "\"data-settle\": \"stat\",",
        "\"aria-label\": label + \": \" + row.primary,",
        "button.disabled = settleBlocked(\"stat\");",
    ] {
        assert!(button.contains(step), "{step}: {button}");
    }
    // `data-settle` puts it among the buttons `syncSettleButtons` re-decides on every list draw;
    // `settleBlocked("stat")` is down exactly while a Sign or a list act is in flight.
    let blocked = js_fn(
        include_str!("../src-ui/med_versions_settle.js"),
        "settleBlocked",
    );
    assert!(
        blocked.contains("if (medSigning) return true;"),
        "{blocked}"
    );
    drawn_under_the_line();
    no_dialog();
}

/// A stat order of a LINKED chart reads its line but offers no button: giving or striking it is
/// that chart's act (spec §2), and Rust refuses it anyway (`STAT_NOT_OPEN`, `GIVE_GONE`).
#[test]
fn a_linked_charts_stat_line_shows_no_button() {
    let other = Uuid::from_u128(2);
    let waiting = pane(&[order(10, other, vec![])]);
    assert!(!stat_of(&waiting, 10).can_give);
    let given = pane(&[order(40, other, vec![give(70, None)])]);
    let stat = stat_of(&given, 40);
    assert_eq!(
        stat.gives,
        [GiveView {
            event_id: Uuid::from_u128(70).to_string(),
            can_strike: false
        }]
    );
    let rows = js_fn(STAT_JS, "statRows");
    assert!(
        rows.contains("for (const give of stat.gives) {")
            && rows.contains("if (!give.can_strike) continue;"),
        "{rows}"
    );
}

/// "Given" opens ONE inline row: "Time given", a `datetime-local` pre-filled with now at the
/// press (minutes, as notes_time's `datetime_local` writes it), `max` now, its Sign and Cancel.
/// It signs nothing until that Sign, which sends the time with the strip's credential, closes the
/// row on success and draws the pane Rust read again.
#[test]
fn the_give_row_prefills_now_and_signs() {
    assert_eq!(js_const(STAT_JS, "GIVE_TIME_LABEL"), "Time given");
    assert_eq!(js_const(STAT_JS, "STAT_SIGN"), "Sign");
    let open = js_fn(STAT_JS, "openStat");
    assert!(!open.contains("invoke("), "opening signs nothing: {open}");
    assert!(
        open.contains("time: kind === \"give\" ? localMinute(new Date()) : \"\","),
        "{open}"
    );
    let minute = js_fn(STAT_JS, "localMinute");
    assert!(
        minute.contains("d.getHours()") && minute.contains("d.getMinutes()"),
        "{minute}"
    );
    assert!(
        !minute.contains("toISOString"),
        "local, never UTC: {minute}"
    );
    let row = js_fn(STAT_JS, "statRow");
    for step in [
        "{ type: \"datetime-local\", max: localMinute(new Date()) }",
        "const label = cell(\"label\", (give ? GIVE_TIME_LABEL : STRIKE_WHY_LABEL) + \" \");",
        "statButton(STAT_SIGN, row, () => pressStatSign(stat, field))",
        "form.append(label, sign, cancel);",
    ] {
        assert!(row.contains(step), "{step}: {row}");
    }
    let sign = js_fn(STAT_JS, "signStat");
    let sent = position(sign, "invoke(\"med_stat_give\", {");
    for arg in [
        "patientId: patient,",
        "charts: charts,",
        "medicationId: stat.medication_id,",
        "givenAt: words.time || null,",
        "credential: signerCredential(),",
    ] {
        assert!(sign.contains(arg), "{arg}: {sign}");
    }
    let closed = position(sign, "if (statOpen === words) statOpen = null;");
    let drawn = position(sign, "drawSignedTable(answer, patient);");
    assert!(sent < closed && closed < drawn, "{sign}");
    assert!(sign.contains("clearSigner(true);") && sign.contains("clearSigner();"));
    let press = js_fn(STAT_JS, "pressStatSign");
    for step in [
        "if (signInFlight() || settleBlocked(\"stat\")) return;",
        "medListSignInFlight = true;",
        "medListSigning = signStat(stat, words)",
    ] {
        assert!(press.contains(step), "{step}: {press}");
    }
    assert!(js_fn(MAIN_JS, "clearChart").contains("clearStat();"));
    no_dialog();
}

/// Review Focus 4: a partly typed time (a date, no hour) reaches JS as an EMPTY value with
/// `validity.badInput`, and Rust reads empty as NOW — so the press is refused here, in words
/// beside the line, and nothing is invoked. A partial typed before a redraw (`keep` stores `null`)
/// is refused the same way, never rebuilt as the pre-filled now.
#[test]
fn a_partly_typed_time_is_refused_and_signs_nothing() {
    let press = js_fn(STAT_JS, "pressStatSign");
    let kept = position(press, "keep(field);");
    let refused = position(
        press,
        "if (words.kind === \"give\" && words.time === null) return sayStat(stat.medication_id, PARTLY_TYPED_GIVEN);",
    );
    let signed = position(press, "medListSignInFlight = true;");
    assert!(kept < refused && refused < signed, "{press}");
    let keep = js_fn(STAT_JS, "keep");
    assert!(
        keep.contains("statOpen.time = field.validity.badInput ? null : field.value;"),
        "{keep}"
    );
    // Kept on every way a partial can be typed: a partly typed datetime-local fires no `input`.
    let row = js_fn(STAT_JS, "statRow");
    assert!(
        row.contains("for (const type of [\"input\", \"change\", \"keyup\", \"blur\"]) field.addEventListener(type, () => keep(field));"),
        "{row}"
    );
    assert_eq!(
        js_const(STAT_JS, "PARTLY_TYPED_GIVEN"),
        "the time given is only partly typed — finish it, or clear it if the dose was given now"
    );
}

/// A given line reads "given" (styled as a stopped line, `data-ceased`), its stat line names who
/// gave it and when, and each standing give of the opened chart offers "Entered in error". Once
/// struck, the line is current again: the struck give reads as its own line, and Given returns.
#[test]
fn a_given_row_reads_given_and_offers_entered_in_error() {
    let shown = pane(&[order(40, opened(), vec![give(70, None)])]);
    let line = line_of(&shown, 40);
    assert_eq!(line.status_label, "given");
    let stat = stat_of(&shown, 40);
    assert!(!stat.can_give, "a stopped line cannot be given");
    assert!(
        stat.line.contains(" · given ") && stat.line.ends_with(" RN Bo"),
        "{}",
        stat.line
    );
    assert_eq!(
        stat.gives,
        [GiveView {
            event_id: Uuid::from_u128(70).to_string(),
            can_strike: true
        }]
    );
    let struck = Strike {
        who: Some("RN Di".into()),
        at: Utc::now(),
        why: Some("wrong chart".into()),
    };
    let shown = pane(&[order(10, opened(), vec![give(71, Some(struck))])]);
    let stat = stat_of(&shown, 10);
    assert_eq!(line_of(&shown, 10).status_label, "current");
    assert!(stat.can_give && stat.gives.is_empty(), "{stat:?}");
    assert!(stat.line.ends_with("not yet given"), "{}", stat.line);
    assert_eq!(stat.struck.len(), 1);
    assert!(
        stat.struck[0].contains(" — entered in error by RN Di, "),
        "{:?}",
        stat.struck
    );
    assert!(
        stat.struck[0].ends_with(" — wrong chart"),
        "{:?}",
        stat.struck
    );
    assert!(js_fn(MAIN_JS, "renderRow").contains(
        "if (row.status_label === \"ceased\" || row.status_label === \"given\") tr.setAttribute(\"data-ceased\", \"true\");"
    ));
    assert_eq!(js_const(STAT_JS, "STRIKE"), "Entered in error");
    let rows = js_fn(STAT_JS, "statRows");
    assert!(
        rows.contains("td.append(statButton(STRIKE, row, (event) => openStat(\"strike\", stat, give, row, event.target)));"),
        "{rows}"
    );
    drawn_under_the_line();
}

/// "Entered in error" opens its row: "Why (optional)", its Sign and Cancel. The Sign sends the
/// give's event id and the why (blank: none) with the strip's credential.
#[test]
fn the_strike_row_signs_with_its_why() {
    assert_eq!(js_const(STAT_JS, "STRIKE_WHY_LABEL"), "Why (optional)");
    let row = js_fn(STAT_JS, "statRow");
    assert!(
        row.contains(": { type: \"text\", autocomplete: \"off\" }"),
        "{row}"
    );
    let open = js_fn(STAT_JS, "openStat");
    assert!(
        open.contains("eventId: give ? give.event_id : null,"),
        "{open}"
    );
    let sign = js_fn(STAT_JS, "signStat");
    let sent = position(sign, "invoke(\"med_stat_strike\", {");
    for arg in [
        "givenEvent: words.eventId,",
        "why: words.why.trim() || null,",
    ] {
        assert!(sign[sent..].contains(arg), "{arg}: {sign}");
    }
    no_dialog();
}

/// A refusal (STAT_NOT_OPEN, GIVE_GONE, GIVEN_LATER_THAN_NOW, a locked key) is said BESIDE the
/// line, kept across a redraw until the next press, never only in a status bar; the row keeps its
/// words. A list behind the record is read again. The stat read's own failure is said above the
/// list (`STAT_NOT_READ`), and taken down with it.
#[test]
fn a_refusal_is_worded_beside_the_line() {
    let sign = js_fn(STAT_JS, "signStat");
    let said = position(sign, "sayStat(stat.medication_id, failureText(failure));");
    let behind = position(
        sign,
        "if (behind) await refresh(failureText(failure), behind);",
    );
    let closed = position(sign, "if (statOpen === words) statOpen = null;");
    assert!(said < behind && behind < closed, "{sign}");
    let say = js_fn(STAT_JS, "sayStat");
    assert!(say.contains("statSaid.set(medicationId, text);"), "{say}");
    let rows = js_fn(STAT_JS, "statRows");
    for step in [
        "const said = cell(\"p\", statSaid.get(stat.medication_id) || \"\"",
        "said.hidden = !statSaid.has(stat.medication_id);",
        "td.append(said);",
    ] {
        assert!(rows.contains(step), "{step}: {rows}");
    }
    let shown = js_fn(STAT_JS, "statNotRead");
    assert!(
        shown.contains("el(\"versions-not-read\").after(p);"),
        "{shown}"
    );
    assert!(shown.contains("setMessage(p, text);"), "{shown}");
    assert!(js_fn(MAIN_JS, "drawList").contains("statNotRead(pane.stat_error);"));
    assert!(js_fn(MAIN_JS, "clearDrawnList").contains("statNotRead(\"\");"));
    not_hidden_by_css("#stat-not-read");
    no_dialog();
}

/// Every field the script reads off a row, its stat line and a give is one Rust sends.
#[test]
fn every_field_read_is_one_rust_sends() {
    let shown = pane(&[order(40, opened(), vec![give(70, None)])]);
    let row = serde_json::to_value(line_of(&shown, 40)).unwrap();
    for field in fields_read_in(STAT_JS, "row") {
        assert!(row.get(&field).is_some(), "row.{field} is not sent: {row}");
    }
    let stat = &row["stat"];
    for field in fields_read_in(STAT_JS, "stat") {
        assert!(
            stat.get(&field).is_some(),
            "stat.{field} is not sent: {stat}"
        );
    }
    let give = &stat["gives"][0];
    for field in fields_read_in(STAT_JS, "give") {
        assert!(
            give.get(&field).is_some(),
            "give.{field} is not sent: {give}"
        );
    }
}
