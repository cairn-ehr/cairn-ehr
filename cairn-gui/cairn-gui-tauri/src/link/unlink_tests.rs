//! Tests for `link/unlink.rs`, in a sibling file like `unlink_view_tests.rs`.
use super::*;
use crate::state::AppState;
use cairn_node::patient::edges::{LinkDoubt, RecordEdge};
use uuid::Uuid;

fn fixture() -> Uuid {
    cairn_gui_data::mock::fixtures::FIXTURE_UUID
        .parse()
        .unwrap()
}
fn on_screen() -> (String, Vec<String>) {
    (fixture().to_string(), vec![fixture().to_string()])
}

#[tokio::test]
async fn compare_linked_is_bound_to_the_chart_on_screen() {
    let state = AppState::mock(Some(fixture()));
    let err = compare_linked_impl(
        &state,
        &Uuid::from_u128(9).to_string(),
        vec![],
        "1",
        "2",
        "link",
        false,
    )
    .await
    .unwrap_err();
    assert!(
        err.text.contains("not the chart"),
        "must refuse a chart that is not on screen"
    );
}

#[tokio::test]
async fn unlink_is_bound_to_the_chart_on_screen() {
    let state = AppState::mock(Some(fixture()));
    let err = unlink_impl(
        &state,
        &Uuid::from_u128(9).to_string(),
        vec![],
        "1",
        "2",
        vec![],
    )
    .await
    .unwrap_err();
    assert!(
        err.text.contains("not the chart"),
        "must refuse a chart that is not on screen"
    );
}

#[tokio::test]
async fn unlink_refuses_a_changed_record() {
    let state = AppState::mock(Some(fixture()));
    let (p, _) = on_screen();
    let err = unlink_impl(
        &state,
        &p,
        vec![p.clone(), Uuid::from_u128(7).to_string()],
        &Uuid::from_u128(1).to_string(),
        &Uuid::from_u128(2).to_string(),
        vec![],
    )
    .await
    .unwrap_err();
    assert_eq!(err.text, crate::link::view::THIS_CHANGED);
}

#[tokio::test]
async fn compare_linked_refuses_a_changed_record_in_the_lists_wording() {
    let state = AppState::mock(Some(fixture()));
    let (p, _) = on_screen();
    let err = compare_linked_impl(
        &state,
        &p,
        vec![p.clone(), Uuid::from_u128(7).to_string()],
        &Uuid::from_u128(1).to_string(),
        &Uuid::from_u128(2).to_string(),
        "link",
        false,
    )
    .await
    .unwrap_err();
    assert_ne!(err.text, crate::link::view::THIS_CHANGED);
    assert!(err.text.contains("linked charts changed"));
}

/// A link a colleague's unlink (or this clinician's own) already removed: nothing to judge.
#[tokio::test]
async fn unlink_refuses_a_link_the_record_no_longer_has() {
    let state = AppState::mock(Some(fixture()));
    let (p, charts) = on_screen();
    let err = unlink_impl(
        &state,
        &p,
        charts,
        &p,
        &Uuid::from_u128(2).to_string(),
        vec![],
    )
    .await
    .unwrap_err();
    assert_eq!(err.text, crate::link::unlink_view::LINK_GONE);
}

/// Cannot tell the membership stage from compare's `db None` -> LINK_GONE; the pure
/// `shown_pair` tests pin the membership stage.
#[tokio::test]
async fn compare_linked_refuses_a_link_the_record_no_longer_has() {
    let state = AppState::mock(Some(fixture()));
    let (p, charts) = on_screen();
    let err = compare_linked_impl(
        &state,
        &p,
        charts,
        &p,
        &Uuid::from_u128(2).to_string(),
        "link",
        false,
    )
    .await
    .unwrap_err();
    assert_eq!(err.text, crate::link::unlink_view::LINK_GONE);
}

#[tokio::test]
async fn an_unparseable_link_is_refused_not_guessed() {
    let state = AppState::mock(Some(fixture()));
    let (p, charts) = on_screen();
    let err = unlink_impl(&state, &p, charts, "not-a-uuid", "2", vec![])
        .await
        .unwrap_err();
    assert_eq!(err.text, crate::link::unlink_view::LINK_GONE);
}

fn edge(a: u128, b: u128) -> RecordEdge {
    RecordEdge {
        low: Uuid::from_u128(a),
        high: Uuid::from_u128(b),
        attested: true,
        recorded_on: "2026-09-28".into(),
    }
}

/// A read with these standing links and no doubts.
fn links(edges: Vec<RecordEdge>) -> Result<RecordLinks, String> {
    Ok(RecordLinks {
        edges,
        doubts: vec![],
    })
}

#[test]
fn a_present_link_passes_however_the_pair_was_ordered() {
    let (l, h) = (Uuid::from_u128(1), Uuid::from_u128(2));
    assert!(shown_pair(links(vec![edge(1, 2)]), l, h, PairKind::Link, Act::Unlink).is_ok());
    let (l, h) = cairn_node::chart_link::canonical_pair(h, l);
    assert!(shown_pair(links(vec![edge(1, 2)]), l, h, PairKind::Link, Act::Unlink).is_ok());
}

#[test]
fn an_absent_link_is_gone() {
    let (l, h) = (Uuid::from_u128(1), Uuid::from_u128(3));
    let err = shown_pair(links(vec![edge(1, 2)]), l, h, PairKind::Link, Act::Unlink).unwrap_err();
    assert_eq!(err.text, LINK_GONE);
}

#[test]
fn an_unread_edge_list_is_retryable_and_not_a_verdict() {
    let (l, h) = (Uuid::from_u128(1), Uuid::from_u128(2));
    let err = shown_pair(Err("boom".into()), l, h, PairKind::Link, Act::Unlink).unwrap_err();
    assert_eq!(err.retry, Retry::Now);
    assert_ne!(err.text, LINK_GONE);
    assert!(err.text.contains("nothing was done"));
    let err = shown_pair(Err("boom".into()), l, h, PairKind::Link, Act::Compare).unwrap_err();
    assert_eq!(err.retry, Retry::Now);
    assert!(!err.text.contains("nothing was done"));
}

#[test]
fn a_separated_pair_must_still_be_separated() {
    let sep = |a, b| LinkDoubt {
        low: Uuid::from_u128(a),
        high: Uuid::from_u128(b),
        kind: DoubtKind::Separated,
        recorded_on: "2026-10-01".into(),
    };
    let links = || {
        Ok(RecordLinks {
            edges: vec![],
            doubts: vec![sep(1, 3)],
        })
    };
    let (l, h) = (Uuid::from_u128(1), Uuid::from_u128(3));
    assert!(shown_pair(links(), l, h, PairKind::Separated, Act::Confirm).is_ok());
    let err = shown_pair(links(), l, h, PairKind::Link, Act::Confirm).unwrap_err();
    assert_eq!(
        err.text, LINK_GONE,
        "a separated pair is not a standing link"
    );
}

#[test]
fn a_clash_link_carries_its_attestation_into_the_shown_pair() {
    let links = Ok(RecordLinks {
        edges: vec![RecordEdge {
            attested: false,
            ..edge(1, 2)
        }],
        doubts: vec![],
    });
    let s = shown_pair(
        links,
        Uuid::from_u128(1),
        Uuid::from_u128(2),
        PairKind::Link,
        Act::Compare,
    )
    .unwrap();
    assert!(!s.attested && s.kind == PairKind::Link);
}

/// The shown pair carries the date the panel names: the link's own for a link, the
/// separation's for a separated pair. A Clash doubt on a pair is NOT a separation.
#[test]
fn the_shown_pair_carries_its_own_date_and_a_clash_is_not_a_separation() {
    let clash = LinkDoubt {
        low: Uuid::from_u128(1),
        high: Uuid::from_u128(2),
        kind: DoubtKind::Clash,
        recorded_on: "2026-10-02".into(),
    };
    let (l, h) = (Uuid::from_u128(1), Uuid::from_u128(2));
    let read = || {
        Ok(RecordLinks {
            edges: vec![edge(1, 2)],
            doubts: vec![clash.clone()],
        })
    };
    let s = shown_pair(read(), l, h, PairKind::Link, Act::Compare).unwrap();
    assert_eq!(s.recorded_on, "2026-09-28");
    let err = shown_pair(read(), l, h, PairKind::Separated, Act::Compare).unwrap_err();
    assert_eq!(err.text, LINK_GONE);
}

/// A kind the webview did not get from us is refused like a vanished link, never guessed — and
/// FIRST, before any screen check reads anything. Real chart ids of a link the record does not
/// have, sent with a CHANGED chart set: were the kind checked after `resolve_edge`, the answer
/// would be the changed-set sentence instead.
#[tokio::test]
async fn compare_refuses_an_unknown_pair_kind() {
    let state = AppState::mock(Some(fixture()));
    let (p, _) = on_screen();
    let err = compare_linked_impl(
        &state,
        &p,
        vec![p.clone(), Uuid::from_u128(7).to_string()],
        &Uuid::from_u128(1).to_string(),
        &Uuid::from_u128(2).to_string(),
        "merged",
        false,
    )
    .await
    .unwrap_err();
    assert_eq!(err.text, crate::link::unlink_view::LINK_GONE);
}

/// The webview half of the Unlink's stale-pair check: `unlink.js` must send the standing
/// address back with `unlink_records`, or the node has nothing to re-check (the command would
/// refuse every Unlink as unread). `unlink.js` is untyped, so this reads its source.
#[test]
fn unlink_js_sends_the_standing_address_with_the_unlink() {
    let js = include_str!("../../src-ui/unlink.js");
    let start = js
        .find("invoke(\"unlink_records\"")
        .expect("unlink.js invokes unlink_records");
    let call = &js[start..start + js[start..].find("});").expect("the call's object closes")];
    assert!(call.contains("standing: sent.standing"), "{call}");
}

/// The pre-click race (PR #777 review): the list line says "linked without a clinician's
/// confirmation", a colleague's attested confirm then lands while a third chart keeps the set
/// unchanged, and the clinician clicks "Review this link…". The panel must not open over a pair
/// that no longer reads the way the line did — it would offer only Unlink, silently, over a
/// colleague's judgement the clinician never saw.
#[test]
fn a_line_drawn_before_the_pair_was_judged_is_stale() {
    let pair = |attested| ShownPair {
        kind: PairKind::Link,
        attested,
        recorded_on: "2026-10-01".into(),
    };
    assert!(line_still_shows(&pair(false), false));
    assert!(line_still_shows(&pair(true), true));
    assert!(!line_still_shows(&pair(true), false), "a colleague confirmed it");
    assert!(!line_still_shows(&pair(false), true), "an attested link no longer stands");
}

/// The webview half of the pre-click check: each line's door sends the attestation the line
/// showed, and the Compare carries it. `main.js`/`unlink.js` are untyped, so this reads them.
#[test]
fn the_door_sends_the_attestation_its_line_showed() {
    let main = include_str!("../../src-ui/main.js");
    assert!(
        main.contains(
            "compareLinked(recordLink.low, recordLink.high, recordLink.kind, recordLink.attested)"
        ),
        "main.js passes the line's attestation to compareLinked"
    );
    let js = include_str!("../../src-ui/unlink.js");
    let start = js
        .find("invoke(\"compare_linked\"")
        .expect("unlink.js invokes compare_linked");
    let call = &js[start..start + js[start..].find("});").expect("the call's object closes")];
    assert!(call.contains("attested"), "{call}");
}

/// The body of top-level JS function `name` in `src`: from its declaration to the first line
/// that is a lone `}` (the house formatting closes every top-level function that way).
fn js_function<'a>(src: &'a str, name: &str) -> &'a str {
    let start = src
        .find(&format!("function {name}("))
        .unwrap_or_else(|| panic!("{name} is declared"));
    let len = src[start..]
        .find("\n}\n")
        .unwrap_or_else(|| panic!("{name} closes"));
    &src[start..start + len]
}

/// The Outranked race (PR #777 review): a verdict is in flight, the clinician presses the SAME
/// pair's "Review…" again, and the first answer lands Outranked on the chart line ("chart"), not
/// on the panel. Outranked leaves the pair's standing judgement unchanged, so the node's
/// stale-pair check would pass a second press — a second attested judgement over the colleague's
/// that already outranked the first. Wherever an Outranked answer lands, a newer comparison of
/// the same pair is disarmed, exactly as an unknown outcome (#713) already disarms it.
#[test]
fn an_outranked_verdict_disarms_a_newer_comparison_of_the_same_pair() {
    let js = include_str!("../../src-ui/unlink.js");
    let report = js_function(js, "showUnlinkReport");
    assert!(
        report.contains("if (!report.reload) disarmNewerReview(sent);"),
        "{report}"
    );
    let failure = js_function(js, "showUnlinkFailure");
    assert!(
        failure.contains("if (reread) disarmNewerReview(sent);"),
        "{failure}"
    );

    // R5a's banner panel has the same race over its Link's Outranked answer.
    let link = include_str!("../../src-ui/link.js");
    let send = js_function(link, "sendJudgement");
    assert!(
        send.contains("if (!report.reload) disarmNewerComparison(sent);"),
        "{send}"
    );
    assert!(
        send.contains("if (reread) disarmNewerComparison(sent);"),
        "{send}"
    );
    // The webview's scripts share ONE global scope (plain <script> tags): a helper declared in
    // both files under one name would silently become the later file's.
    assert!(!js.contains("function disarmNewerComparison("));
    assert!(!link.contains("function disarmNewerReview("));
}

/// A Confirm read failure is worded like an Unlink's: it is about to write.
#[test]
fn an_unread_list_before_a_confirm_says_nothing_was_done() {
    let (l, h) = (Uuid::from_u128(1), Uuid::from_u128(2));
    let err = shown_pair(Err("boom".into()), l, h, PairKind::Link, Act::Confirm).unwrap_err();
    assert_eq!(err.retry, Retry::Now);
    assert!(err.text.contains("nothing was done"));
}
