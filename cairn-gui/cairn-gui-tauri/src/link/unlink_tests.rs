//! Tests for `link/unlink.rs`, in a sibling file like `unlink_view_tests.rs`.
use super::*;
use crate::source_guard::{js_fn, rust_fn};
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
    assert_eq!(err.retry, crate::funnel::view::Retry::AfterReread);
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
    assert_eq!(err.text, crate::chart_set::CHANGED);
    assert_eq!(err.retry, crate::funnel::view::Retry::AfterReread);
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
    assert_eq!(err.retry, crate::funnel::view::Retry::AfterReread);
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
    // Nothing the record did: a re-read cannot mend an id the window failed to send.
    assert_eq!(err.retry, crate::funnel::view::Retry::Never);
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
        unverified: Ok(vec![]),
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
    assert_eq!(err.retry, crate::funnel::view::Retry::AfterReread);
}

/// A failed read of the "unverified" notes is not a failed read of the links (PR #788 review):
/// the notes hold nothing, so every verdict and Compare still finds its pair.
#[test]
fn an_unread_note_list_never_blocks_a_verdict() {
    let (l, h) = (Uuid::from_u128(1), Uuid::from_u128(2));
    let read = Ok(RecordLinks {
        unverified: Err("boom".into()),
        ..links(vec![edge(1, 2)]).unwrap()
    });
    assert!(shown_pair(read, l, h, PairKind::Link, Act::Unlink).is_ok());
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
            unverified: Ok(vec![]),
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
        unverified: Ok(vec![]),
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
            unverified: Ok(vec![]),
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
    assert_eq!(err.retry, crate::funnel::view::Retry::Never);
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
    assert!(
        !line_still_shows(&pair(true), false),
        "a colleague confirmed it"
    );
    assert!(
        !line_still_shows(&pair(false), true),
        "an attested link no longer stands"
    );
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

/// The Outranked race (PR #777 review): a verdict is in flight, the clinician presses the SAME
/// pair's "Review…" again, and the first answer lands Outranked on the chart line ("chart"), not
/// on the panel. Outranked leaves the pair's standing judgement unchanged, so the node's
/// stale-pair check would pass a second press — a second attested judgement over the colleague's
/// that already outranked the first. Wherever ANY recorded answer lands, a newer comparison of
/// the same pair is disarmed, exactly as an unknown outcome (#713) already disarms it — not only
/// when `reload` is false: R5a's "Different people" answers `reload: true` even when every pair
/// was outranked, so `reload` cannot be the test (the fix review of PR #777). A newer comparison
/// over a record that DID change is stale anyway, so disarming it costs a re-Review, nothing more.
#[test]
fn a_recorded_verdict_disarms_a_newer_comparison_of_the_same_pair() {
    let js = include_str!("../../src-ui/unlink.js");
    let report = js_fn(js, "showUnlinkReport");
    assert!(report.contains("\n  disarmNewerReview(sent);"), "{report}");
    let failure = js_fn(js, "showUnlinkFailure");
    assert!(
        failure.contains("if (reread) disarmNewerReview(sent);"),
        "{failure}"
    );

    // R5a's banner panel has the same race, over both of its verdicts.
    let link = include_str!("../../src-ui/link.js");
    let send = js_fn(link, "sendJudgement");
    assert!(
        send.contains("\n    disarmNewerComparison(sent);\n"),
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

/// The window's half of the stale-pair guarantee (PR #777 review): the node's pair re-check
/// under the chart-link lock runs only inside the `_as_shown` entry points. The window has no
/// DB-backed tests (#738), so no GUI test reaches the node call, and the node's own tests
/// (`confirm_link.rs`, `unlink_as_shown.rs`) call the node directly. Reverting either verdict
/// to the plain `unlink_charts` / `link_charts` would silently bring back the overrule of a
/// colleague's mid-review judgement with every test green — so this reads the source.
#[test]
fn both_verdicts_sign_only_through_the_node_s_as_shown_checks() {
    let unlink = rust_fn(include_str!("unlink.rs"), "unlink_impl");
    assert!(
        unlink.contains("cairn_node::chart_link::unlink_charts_as_shown_from("),
        "{unlink}"
    );
    assert!(!unlink.contains("chart_link::unlink_charts("), "{unlink}");
    let confirm = rust_fn(include_str!("confirm.rs"), "confirm_link_impl");
    assert!(
        confirm.contains("cairn_node::chart_link::confirm_link_as_shown("),
        "{confirm}"
    );
    assert!(!confirm.contains("chart_link::link_charts("), "{confirm}");
}

/// The Compare's half of the pre-click check (the fix review of PR #777): `line_still_shows` is
/// pure and pinned above, and the webview's half is pinned by
/// `the_door_sends_the_attestation_its_line_showed` — but no GUI test reaches the call between
/// them (#738), so deleting it would leave every test green and the race open. It must run
/// before the panel reads which judgement stands, so no panel is drawn over a stale line.
#[test]
fn the_compare_refuses_a_stale_line_before_it_reads_the_pair() {
    let compare = rust_fn(include_str!("unlink.rs"), "compare_linked_impl");
    let guard = compare
        .find("if !line_still_shows(&edge.shown, attested) {")
        .unwrap_or_else(|| panic!("the Compare calls the pre-click check: {compare}"));
    let read = compare
        .find("standing_link(")
        .expect("the Compare reads the standing judgement");
    assert!(guard < read, "the check runs before the panel's read");
}

/// A Confirm read failure is worded like an Unlink's: it is about to write.
#[test]
fn an_unread_list_before_a_confirm_says_nothing_was_done() {
    let (l, h) = (Uuid::from_u128(1), Uuid::from_u128(2));
    let err = shown_pair(Err("boom".into()), l, h, PairKind::Link, Act::Confirm).unwrap_err();
    assert_eq!(err.retry, Retry::Now);
    assert!(err.text.contains("nothing was done"));
}

/// A pair judged since its list line was drawn is the screen being behind the record: the window
/// re-reads ([`stale`]), and the sentence names no remedy it cannot perform. The branch needs a
/// live links read to reach (no DB-gated harness at this layer, issue #738), so the source and
/// the exact copy are pinned instead.
#[test]
fn a_line_judged_since_it_was_drawn_is_stale_not_refused() {
    let compare = rust_fn(include_str!("unlink.rs"), "compare_linked_impl");
    assert!(compare.contains("stale(LINE_STALE)"));
    assert_eq!(
        LINE_STALE,
        "this pair has been judged since the list was drawn — nothing was done"
    );
}
