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
    let err = unlink_impl(&state, &Uuid::from_u128(9).to_string(), vec![], "1", "2")
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
    )
    .await
    .unwrap_err();
    assert_ne!(err.text, crate::link::view::THIS_CHANGED);
    assert!(err.text.contains("linked charts changed"));
}

/// Review Focus 3.
#[tokio::test]
async fn unlink_refuses_a_link_the_record_no_longer_has() {
    let state = AppState::mock(Some(fixture()));
    let (p, charts) = on_screen();
    let err = unlink_impl(&state, &p, charts, &p, &Uuid::from_u128(2).to_string())
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
    )
    .await
    .unwrap_err();
    assert_eq!(err.text, crate::link::unlink_view::LINK_GONE);
}

#[tokio::test]
async fn an_unparseable_link_is_refused_not_guessed() {
    let state = AppState::mock(Some(fixture()));
    let (p, charts) = on_screen();
    let err = unlink_impl(&state, &p, charts, "not-a-uuid", "2")
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

/// A kind the webview did not get from us is refused like a vanished link, never guessed.
#[tokio::test]
async fn compare_refuses_an_unknown_pair_kind() {
    let state = AppState::mock(Some(fixture()));
    let (p, charts) = on_screen();
    let err = compare_linked_impl(&state, &p, charts, "1", "2", "merged")
        .await
        .unwrap_err();
    assert_eq!(err.text, crate::link::unlink_view::LINK_GONE);
}

/// A Confirm read failure is worded like an Unlink's: it is about to write.
#[test]
fn an_unread_list_before_a_confirm_says_nothing_was_done() {
    let (l, h) = (Uuid::from_u128(1), Uuid::from_u128(2));
    let err = shown_pair(Err("boom".into()), l, h, PairKind::Link, Act::Confirm).unwrap_err();
    assert_eq!(err.retry, Retry::Now);
    assert!(err.text.contains("nothing was done"));
}
