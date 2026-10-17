//! Guards on the webview's half of #797: every "the screen is behind the record" refusal
//! (`retry: "after_reread"`) re-reads the WHOLE chart, a re-read says what changed (Rust's
//! sentence), and a list that could not be re-read after such a refusal is marked out of date.
//!
//! The fixes live in `main.js`, `chart_stale.js`, `link.js`, `unlink.js` and `index.html`, which
//! have no test rig of their own (no npm or bundler: main.js's header, issue #332). So, like
//! `chart_view_webview_tests.rs`, each guard reads the script's source (`crate::source_guard`) and
//! pins the ORDER or the CHOICE the fix is. The behaviour was walked headlessly (a stub `invoke`
//! in a browser) when it was written — see the PR that added this file.
use crate::source_guard::{js_const, js_fn, position};

const MAIN_JS: &str = include_str!("../src-ui/main.js");
const CHART_STALE_JS: &str = include_str!("../src-ui/chart_stale.js");
const LINK_JS: &str = include_str!("../src-ui/link.js");
const UNLINK_JS: &str = include_str!("../src-ui/unlink.js");
const INDEX_HTML: &str = include_str!("../src-ui/index.html");

/// The arguments of every `refresh(…)` call in `body`, split at top-level commas (a lead such as
/// `el("outcome").textContent` carries its own parentheses).
fn refresh_calls(body: &str) -> Vec<Vec<String>> {
    let mut calls = Vec::new();
    for (at, _) in body.match_indices("refresh(") {
        // `function refresh(` is the declaration, not a call.
        if body[..at].ends_with("function ") {
            continue;
        }
        let mut args = vec![String::new()];
        let mut depth = 0;
        for c in body[at + "refresh(".len()..].chars() {
            match c {
                '(' => depth += 1,
                ')' if depth == 0 => break,
                ')' => depth -= 1,
                ',' if depth == 0 => {
                    args.push(String::new());
                    continue;
                }
                _ => {}
            }
            args.last_mut().expect("one argument at least").push(c);
        }
        calls.push(args.into_iter().map(|a| a.trim().to_string()).collect());
    }
    calls
}

/// Every re-read measures the change from the HELD set (`heldCharts`, chart_stale.js): the chart
/// the clinician holds — what the window last DREW, moved on by their own acts and by nothing else
/// (the maintainer's rule, 2026-10-17). Not a per-call argument: a baseline living in ONE call let
/// every OTHER re-read (a stale Stop's, "Read this chart again") compare against the pre-act set
/// and announce the clinician's own link back to them (fix round 1, Important 1).
#[test]
fn refresh_sends_the_set_on_screen() {
    assert!(MAIN_JS.contains("async function refresh(lead, staleIfFails) {"));
    let refresh = js_fn(MAIN_JS, "refresh");
    assert!(
        refresh.contains(
            "await invoke(\"med_list\", { patientId: patient, seenCharts: heldCharts });"
        ),
        "{refresh}"
    );
    // `renderedCharts` stays what every WRITE's set check sends; it is never the baseline.
    assert!(!refresh.contains("renderedCharts"), "{refresh}");
}

/// The held set follows what is DRAWN: `drawList` sets it beside `renderedCharts`; `clearChart`
/// (a chart change) and `showUnshownList` (nothing drawn) empty it. Only chart_stale.js assigns it.
#[test]
fn the_held_set_is_what_was_drawn() {
    let draw = js_fn(MAIN_JS, "drawList");
    assert!(
        position(draw, "renderedCharts = view.charts;")
            < position(draw, "holdCharts(view.charts);"),
        "{draw}"
    );
    for name in ["clearChart", "showUnshownList"] {
        let body = js_fn(MAIN_JS, name);
        assert!(body.contains("holdCharts(null);"), "{name}: {body}");
    }
    assert!(CHART_STALE_JS.contains("let heldCharts = null;"));
    assert!(js_fn(CHART_STALE_JS, "holdCharts").contains("heldCharts = charts;"));
    for (file, js) in [
        ("main.js", MAIN_JS),
        ("link.js", LINK_JS),
        ("unlink.js", UNLINK_JS),
    ] {
        assert!(
            !js.contains("heldCharts ="),
            "{file} assigns the held set itself"
        );
    }
}

/// The clinician's own recorded verdict moves the held set by ITS delta, the moment the report
/// lands and before any re-read is sent — so their own change is silent in EVERY later re-read,
/// while a colleague's still differs and is named. Both recorded paths hand over the charts the
/// act touched and `report.charts_after`; no re-read carries a baseline of its own.
#[test]
fn an_own_judgement_re_reads_against_the_set_it_left() {
    let send = js_fn(LINK_JS, "sendJudgement");
    let recorded = &send[..position(send, "catch (failure)")];
    let shown = js_fn(UNLINK_JS, "showUnlinkReport");
    for (body, held) in [
        (
            recorded,
            "holdOwnAct(sent.charts.concat(sent.otherCharts), report.charts_after);",
        ),
        (
            shown,
            "holdOwnAct(sent.charts.concat([sent.low, sent.high]), report.charts_after);",
        ),
    ] {
        let moved = position(body, held);
        let calls = refresh_calls(body);
        assert_eq!(calls.len(), 2, "both re-reads of a recorded verdict");
        assert!(moved < position(body, "await refresh("), "{body}");
        for args in calls {
            assert_eq!(args.len(), 1, "no per-call baseline: {args:?}");
        }
    }
    let own = js_fn(CHART_STALE_JS, "holdOwnAct");
    assert!(
        own.contains("heldCharts = applyOwnAct(heldCharts, touched, after, displayedPatient);"),
        "{own}"
    );
}

/// `applyOwnAct` is pure and anchored on the chart on screen: no held set or no recorded set
/// leaves it as it is; if the anchor ended in the act's result, the touched charts are replaced
/// by that result; if it ended on the OTHER side of the act (an unlink, answered elsewhere), by
/// what the act left there; if the act never touched it, the held set is unchanged — a foreign
/// change is never silenced.
#[test]
fn an_own_act_moves_the_held_set_by_its_own_delta_only() {
    let apply = js_fn(CHART_STALE_JS, "applyOwnAct");
    let empty = position(apply, "if (held === null || !after) return held;");
    let untouched = position(apply, "const untouched = without(held, touched);");
    let joined = position(
        apply,
        "if (after.includes(anchor)) return ordered(untouched.concat(after));",
    );
    let other = position(
        apply,
        "if (touched.includes(anchor)) return ordered(untouched.concat(without(touched, after)));",
    );
    let unchanged = position(apply, "\n  return held;");
    assert!(empty < untouched && untouched < joined && joined < other && other < unchanged);
    for impure in ["heldCharts", "displayedPatient", "el(", "say("] {
        assert!(!apply.contains(impure), "applyOwnAct reads {impure}");
    }
}

/// "Read again" and the change sentence are said only over an answer `mayDraw` ACCEPTED AND
/// drawn — after `render`, inside the success path, and only if the list drew (`render` catches a
/// drawing fault itself and `showUnshownList` nulls `renderedPatient`: "review it" over a list
/// that was taken down would be false). A failed read says "Could not read the chart", never
/// "read again". Not stale and no change: the outcome line is left alone.
#[test]
fn read_again_is_said_only_after_an_accepted_answer() {
    let refresh = js_fn(MAIN_JS, "refresh");
    let rendered = position(refresh, "render(answer, patient);");
    let said = position(
        refresh,
        "if (renderedPatient === patient) sayAfterReread(lead, staleIfFails, answer);",
    );
    let caught = position(refresh, "catch (e)");
    assert!(rendered < said && said < caught, "{refresh}");
    assert!(!refresh[caught..].contains("sayAfterReread"), "{refresh}");
    assert!(
        !refresh.contains("READ_AGAIN"),
        "said only through sayAfterReread"
    );

    let after = js_fn(CHART_STALE_JS, "sayAfterReread");
    let stale = position(after, "if (staleIfFails)");
    let again = position(after, "READ_AGAIN, pane.set_change");
    let changed = position(after, "else if (pane.set_change)");
    assert!(stale < again && again < changed, "{after}");
}

/// The outcome line is a run of sentences from different places — Rust's refusals end without a
/// full stop ("… — nothing was done"), and the node's reach the CLI, so they are not edited. The
/// JOINER closes each part that lacks terminal punctuation and capitalises the line, so it never
/// reads "…nothing was done The chart has been read again…" (fix round 1, Minor 2). The failed
/// re-read's line joins through it too.
#[test]
fn the_outcome_line_joins_whole_sentences() {
    let join = js_fn(CHART_STALE_JS, "sentences");
    assert!(
        join.contains("/[.!?]$/.test(part) ? part : part + \".\""),
        "{join}"
    );
    assert!(
        join.contains("line.charAt(0).toUpperCase() + line.slice(1)"),
        "{join}"
    );
    let refresh = js_fn(MAIN_JS, "refresh");
    let caught = &refresh[position(refresh, "catch (e)")..];
    assert!(
        caught.contains("say(sentences(lead, \"Could not read the chart: \" + e));"),
        "{caught}"
    );
}

/// A stale refusal's re-read that FAILS leaves the old list up (an honest near-truth, principle
/// 4) but marks it out of date. Only by the newest read (after `isLatestRead`), only over a drawn
/// list, and BEFORE the banner's own read is awaited: after that await a newer read may have drawn
/// a fresh list, which must not be marked.
#[test]
fn a_failed_stale_re_read_marks_the_list_out_of_date() {
    let refresh = js_fn(MAIN_JS, "refresh");
    let caught = &refresh[position(refresh, "catch (e)")..];
    let marked = position(
        caught,
        "if (staleIfFails && renderedPatient !== null) markListStale();",
    );
    assert!(
        position(caught, "if (!isLatestRead(patient, revision)) return;") < marked,
        "{refresh}"
    );
    assert!(
        marked < position(caught, "await redrawBannerAlone(patient);"),
        "{refresh}"
    );
}

/// `markListStale` warns in the chart's warnings — ABOVE the table, never on the outcome line —
/// and disables sign-off with a worded reason. It leaves the rows: every write over a stale list
/// is refused by the backend's set check anyway. A list that draws takes the warning down.
#[test]
fn mark_list_stale_disables_sign_off_and_warns_above_the_table() {
    let mark = js_fn(CHART_STALE_JS, "markListStale");
    assert!(
        mark.contains("setMessage(el(\"chart-stale\"), LIST_OUT_OF_DATE);"),
        "{mark}"
    );
    assert!(
        mark.contains("el(\"chart-warnings\").hidden = false;"),
        "{mark}"
    );
    assert!(mark.contains("button.disabled = true;"), "{mark}");
    assert!(
        mark.contains("button.textContent = \"Nothing to sign off — this list is out of date\";"),
        "{mark}"
    );
    for absent in ["say(", "med-rows", "clearDrawnList", "renderedPatient ="] {
        assert!(!mark.contains(absent), "{absent} in {mark}");
    }
    assert!(
        js_fn(MAIN_JS, "drawList").contains("setMessage(el(\"chart-stale\"), \"\");"),
        "a list that draws takes the out-of-date warning down"
    );

    let warnings = position(INDEX_HTML, "<section id=\"chart-warnings\"");
    let stale = position(INDEX_HTML, "<p id=\"chart-stale\" hidden></p>");
    let unshown = position(INDEX_HTML, "<p id=\"chart-unshown\"");
    let table = position(INDEX_HTML, "<table id=\"med-table\">");
    assert!(warnings < stale && stale < unshown && unshown < table);
}

/// Every handler a stale refusal reaches re-reads the WHOLE chart, flagged stale (so a failed
/// re-read marks the list): Compare, the review panel's Compare, a link-panel judgement, a
/// review-panel verdict, sign-off and cease. The flag is `behindTheRecord(failure)` — directly or
/// through `reread` — and only that helper spells `"after_reread"`.
#[test]
fn every_after_reread_handler_re_reads_the_whole_chart_stale() {
    let behind = js_fn(CHART_STALE_JS, "behindTheRecord");
    assert!(
        behind.contains("return Boolean(failure && failure.retry === \"after_reread\");"),
        "{behind}"
    );
    for (file, js, name) in [
        ("link.js", LINK_JS, "compare"),
        ("link.js", LINK_JS, "sendJudgement"),
        ("unlink.js", UNLINK_JS, "compareLinked"),
        ("unlink.js", UNLINK_JS, "showUnlinkFailure"),
        ("main.js", MAIN_JS, "signOff"),
        ("main.js", MAIN_JS, "cease"),
    ] {
        let body = js_fn(js, name);
        // The catch block alone, to its closing `  }` (signOff's and cease's success re-read
        // follows it); showUnlinkFailure IS the catch.
        let caught = match body.find("catch (failure)") {
            Some(at) => {
                let rest = &body[at..];
                &rest[..rest.find("\n  }").unwrap_or(rest.len())]
            }
            None => body,
        };
        let calls = refresh_calls(caught);
        assert!(!calls.is_empty(), "{file} {name} re-reads nothing: {body}");
        for args in calls {
            let flag = args.get(1).map(String::as_str);
            let derived = match flag {
                Some("behindTheRecord(failure)") => true,
                Some("reread") => body.contains("const reread = behindTheRecord(failure);"),
                _ => false,
            };
            assert!(derived, "{file} {name}: refresh{args:?}");
        }
    }
}

/// The two sentences this slice words itself, verbatim. The out-of-date one names the control
/// that is the way out — "Read this chart again" — rather than "reload", which this window has
/// no control for.
#[test]
fn the_list_out_of_date_sentence_names_the_control() {
    assert_eq!(
        js_const(CHART_STALE_JS, "READ_AGAIN"),
        "The chart has been read again — review it before you act on it."
    );
    // Single-quoted in the script (it quotes the control's name), so `js_const`, which reads
    // double-quoted pieces, cannot read it: the pieces between single quotes are joined here.
    let start = position(CHART_STALE_JS, "const LIST_OUT_OF_DATE =");
    let declaration = &CHART_STALE_JS[start..start + position(&CHART_STALE_JS[start..], ";\n")];
    let stale: String = declaration.split('\'').skip(1).step_by(2).collect();
    assert_eq!(
        stale,
        "This list is out of date: the record changed while it was on screen, and reading it \
         again failed. Press \"Read this chart again\"."
    );
}
