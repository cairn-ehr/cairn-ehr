//! The banner commands' fixture-mode tests (R5a): Review's admission, every "Different people"
//! refusal up to fixture mode, and the duplicates.js field-drift guard. Moved out of `mod.rs`
//! (#745) to keep it under 500 lines.
use super::*;
use cairn_patient_search::{Candidate, TrustState};

fn fixture() -> Uuid {
    cairn_gui_data::mock::fixtures::FIXTURE_UUID
        .parse()
        .unwrap()
}

#[tokio::test]
async fn fixture_mode_shows_no_entries_and_says_no_check_ran() {
    let state = AppState::mock(Some(fixture()));
    let s = duplicate_section(&state, fixture(), &ChartSet::single(fixture()), &[]).await;
    assert_eq!(s, view::fixture_section());
}

#[tokio::test]
async fn a_shown_chart_is_admitted_by_its_list() {
    let state = AppState::mock(Some(fixture()));
    let other = Uuid::from_u128(2);
    state.shown.lock().await.insert(
        other,
        Candidate {
            patient_id: other,
            display_name: "Other Person".into(),
            age: None,
            trust: TrustState::Confirmed,
            last_activity: None,
            locale: None,
            photo_ref: None,
        },
    );
    let name = admit_other(&state, &ChartSet::single(fixture()), other)
        .await
        .unwrap();
    assert_eq!(name.as_deref(), Some("Other Person"));
}

/// Fixture mode has no proposals, so an unshown chart keeps today's refusal word for word.
#[tokio::test]
async fn in_fixture_mode_an_unshown_chart_keeps_the_not_on_screen_refusal() {
    let state = AppState::mock(Some(fixture()));
    let err = admit_other(&state, &ChartSet::single(fixture()), Uuid::from_u128(2))
        .await
        .unwrap_err();
    assert_eq!(err.text, crate::link::view::NOT_ON_SCREEN);
}

/// Final review M5: a chart already in this record is refused BEFORE Review's admission is
/// asked — no list showed it here, so admitting first would refuse it as "not on screen"
/// (and, live, spend a proposal read on a chart that can never be compared).
#[tokio::test]
async fn an_in_record_chart_is_refused_before_any_admission() {
    let state = AppState::mock(Some(fixture()));
    let p = fixture().to_string();
    let err = crate::link::same_person::compare_impl(&state, &p, vec![p.clone()], &p)
        .await
        .unwrap_err();
    assert_eq!(err.text, crate::link::view::ALREADY_IN_RECORD);
}

/// Final review M4c: a malformed other chart id is a window fault ("not on screen", as
/// `link::same_person::link_impl` words it) — never "already judged", which would tell the
/// clinician a colleague resolved something nobody touched.
#[tokio::test]
async fn a_malformed_other_chart_is_not_on_screen() {
    let state = AppState::mock(Some(fixture()));
    let p = fixture().to_string();
    let err = different_people_impl(&state, &p, vec![p.clone()], "not-a-uuid", vec![])
        .await
        .unwrap_err();
    assert_eq!(err.text, crate::link::view::NOT_ON_SCREEN);
}

#[tokio::test]
async fn different_people_is_bound_to_the_chart_on_screen() {
    let state = AppState::mock(Some(fixture()));
    let err = different_people_impl(&state, &Uuid::from_u128(9).to_string(), vec![], "x", vec![])
        .await
        .unwrap_err();
    assert!(err.text.contains("not the chart"), "{}", err.text);
}

#[tokio::test]
async fn different_people_refuses_a_changed_set() {
    let state = AppState::mock(Some(fixture()));
    let p = fixture().to_string();
    let err = different_people_impl(
        &state,
        &p,
        vec![p.clone(), Uuid::from_u128(7).to_string()],
        &Uuid::from_u128(2).to_string(),
        vec![],
    )
    .await
    .unwrap_err();
    assert_eq!(err.text, crate::link::view::THIS_CHANGED);
    assert_eq!(err.retry, crate::funnel::view::Retry::AfterReread);
}

#[tokio::test]
async fn fixture_mode_cannot_record_different_people() {
    let state = AppState::mock(Some(fixture()));
    let p = fixture().to_string();
    let other = Uuid::from_u128(2).to_string();
    let err = different_people_impl(&state, &p, vec![p.clone()], &other, vec![other.clone()])
        .await
        .unwrap_err();
    assert!(err.text.contains("fixture mode"), "{}", err.text);
}

/// Test review gap 1: the other record changed between the comparison and the press. This is
/// the one rule that keeps the signature to the charts the clinician compared — so it comes
/// BEFORE fixture mode, as in `link::same_person::link_impl`, where a fixture-mode test can
/// reach it.
#[tokio::test]
async fn different_people_refuses_when_the_other_record_changed() {
    let state = AppState::mock(Some(fixture()));
    let p = fixture().to_string();
    let other = Uuid::from_u128(2);
    let err = different_people_impl(
        &state,
        &p,
        vec![p.clone()],
        &other.to_string(),
        vec![other.to_string(), Uuid::from_u128(4).to_string()],
    )
    .await
    .unwrap_err();
    assert_eq!(err.text, crate::link::view::OTHER_CHANGED);
    assert_eq!(err.retry, crate::funnel::view::Retry::Never);
}

/// Type review finding 3: a chart already in this record is "already in this record", as
/// Link says it — never "already judged or resolved", which claims a colleague's act.
#[tokio::test]
async fn different_people_refuses_a_chart_already_in_this_record() {
    let state = AppState::mock(Some(fixture()));
    let p = fixture().to_string();
    let err = different_people_impl(&state, &p, vec![p.clone()], &p, vec![p.clone()])
        .await
        .unwrap_err();
    assert_eq!(err.text, crate::link::view::ALREADY_IN_RECORD);
}

/// duplicates.js is untyped: a Rust field rename would not break the build, it would draw
/// an empty banner. Every field it reads must be one the backend sends.
#[test]
fn duplicates_js_reads_no_field_the_backend_does_not_send() {
    use crate::commands::tests::fields_read_in;
    let js = include_str!("../../src-ui/duplicates.js");
    let entry = view::entry_view(
        Uuid::from_u128(9),
        view::EntryFlags {
            vetoed: true,
            accepted: false,
            disputed: false,
        },
        Ok(vec![]),
        Err("x".into()),
    );
    let keys = |v: serde_json::Value| -> std::collections::BTreeSet<String> {
        v.as_object().unwrap().keys().cloned().collect()
    };
    for (binding, available) in [
        (
            "section",
            keys(serde_json::to_value(DuplicateSection::checked_none_open()).unwrap()),
        ),
        ("entry", keys(serde_json::to_value(&entry).unwrap())),
    ] {
        let read = fields_read_in(js, binding);
        assert!(
            !read.is_empty(),
            "duplicates.js no longer reads `{binding}` — rename it here"
        );
        for field in read {
            assert!(
                available.contains(&field),
                "duplicates.js reads `{binding}.{field}`, not sent"
            );
        }
    }
    // #737: losing either read draws a nameless heading or hides the entry's status.
    let entry_reads = fields_read_in(js, "entry");
    assert!(entry_reads.contains("heading") && entry_reads.contains("status"));
    // #745 point 5: the banner hides on Rust's `silent`, never on a JS-side conjunction.
    assert!(fields_read_in(js, "section").contains("silent"));
}

/// #736: "Different people" on an accepted pair is refused, wording and `Retry::Never`.
#[test]
fn accepted_as_same_maps_to_the_not_overruled_refusal() {
    let err = different_people_view(DifferentPeople::AcceptedAsSame).unwrap_err();
    assert_eq!(err.text, view::ACCEPTED_NOT_OVERRULED);
    assert_eq!(err.retry, Retry::Never);
}

/// #745 points 1–2: the banner ALONE, which the window reads when the chart's medication list
/// could not be (and after a decided refusal). Bound to the chart on screen like every chart
/// command: a request naming a chart no longer on screen is refused. The refusal is a worded
/// `ErrorView` (a decided `Retry::Never`), never a bare string — the window words a bare string
/// as "could not reach its backend", which would send the clinician chasing an outage that is
/// not there (PR #798 review).
#[tokio::test]
async fn the_banner_alone_is_bound_to_the_chart_on_screen() {
    let state = AppState::mock(Some(fixture()));
    let err = banner_alone_impl(&state, &Uuid::from_u128(9).to_string())
        .await
        .unwrap_err();
    assert!(err.text.contains("not the chart"), "{}", err.text);
    assert_eq!(err.retry, Retry::Never);
}

/// It is the SAME banner a chart open draws — the same reads, the same sentences — so a failed
/// list read changes only what is around it, never what it says.
#[tokio::test]
async fn the_banner_alone_is_the_banner_a_chart_open_draws() {
    let state = AppState::mock(Some(fixture()));
    let p = fixture().to_string();
    let alone = banner_alone_impl(&state, &p).await.unwrap();
    let pane = crate::commands::med_list_impl(&state, &p).await.unwrap();
    assert_eq!(alone, pane.duplicates);
}

/// When even the record's chart set cannot be read, the banner says so — an error line, never a
/// silent (hidden) banner, which would claim "checked, none open". **Pure.**
#[test]
fn an_unread_chart_set_is_a_worded_banner_never_a_silent_one() {
    let section = unread_set_section("connection reset".into());
    assert!(!section.is_silent());
    let error = section.error.expect("worded");
    assert!(error.contains("connection reset"), "{error}");
}

/// The banner alone is drawn through the chart open's OWN reads — the same section builder, the
/// same member names — so it cannot drift into a second banner. In fixture mode both sides are the
/// fixture section (the test above), so only the source can show it. A failed member-name read is
/// said (`note_unread_names`), never dropped (PR #798 review).
#[test]
fn the_banner_alone_draws_through_the_chart_opens_own_reads() {
    let body = crate::source_guard::rust_fn(include_str!("mod.rs"), "banner_alone_impl");
    for call in [
        "record_charts(state, patient)",
        "linked_members(state, &charts)",
        "duplicate_section(state, patient, &charts,",
        "note_unread_names(",
    ] {
        crate::source_guard::position(body, call);
    }
}

/// A banner read on its own while the record's member names could not be read: a check line may
/// then name a linked chart only by its number, so the banner says why — and with no check line
/// there is nothing to explain, and a silent section stays silent. **Pure.**
#[test]
fn unread_member_names_are_said_only_where_a_check_line_needs_them() {
    let with_line = DuplicateSection {
        check_lines: vec!["Linked chart 9: duplicate check not yet run".into()],
        ..DuplicateSection::checked_none_open()
    };
    let noted = view::note_unread_names(with_line, "connection reset");
    assert_eq!(noted.check_lines.len(), 2);
    assert!(
        noted.check_lines[1].contains("connection reset"),
        "{:?}",
        noted.check_lines
    );
    assert!(
        noted.check_lines[1].contains("names"),
        "{:?}",
        noted.check_lines
    );

    let quiet = view::note_unread_names(DuplicateSection::checked_none_open(), "connection reset");
    assert!(quiet.is_silent());
}
