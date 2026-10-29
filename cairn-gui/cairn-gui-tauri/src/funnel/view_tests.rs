//! Goldens for every front-door sentence and payload in `funnel/view.rs` (moved out of
//! line, #793). The wording IS the safety content (see that file's header), so each test
//! pins a sentence. `sample_candidate` is shared: `funnel::rows` and `funnel::commands`
//! import it as `crate::funnel::view::tests::sample_candidate`.
use super::*;
use cairn_gui_funnel::trigger_state;

/// Named fields, so each call below says which count is which.
fn counts(shown: usize, withheld: usize, incomplete: bool) -> PromptCounts {
    PromptCounts {
        shown,
        shown_charts: shown,
        withheld,
        incomplete,
    }
}
use cairn_patient_search::{Age, TrustState};

#[test]
fn every_actor_standing_but_enrolled_has_its_own_sentence() {
    assert_eq!(standing_sentence(ActorStanding::Enrolled, "k"), None);
    let never = standing_sentence(ActorStanding::NeverEnrolled, "k").unwrap();
    let retired = standing_sentence(ActorStanding::Retired, "k").unwrap();
    let ambiguous = standing_sentence(ActorStanding::Ambiguous, "k").unwrap();
    assert!(never.contains("enroll-device-actor"), "{never}");
    // Retired must NOT offer enroll-device-actor as the fix (#152): the remedy is a key.
    assert!(retired.contains("NEW signing key"), "{retired}");
    assert!(ambiguous.contains("MORE THAN ONE"), "{ambiguous}");
    assert_ne!(never, retired);
    assert_ne!(retired, ambiguous);
}

/// The sentence must name the key the standing was computed for (#670).
#[test]
fn the_standing_sentence_names_the_probed_key() {
    let s = standing_sentence(ActorStanding::NeverEnrolled, "abc123").unwrap();
    assert!(s.contains("abc123"), "{s}");
}

#[test]
fn a_failed_search_never_reads_as_nothing_found() {
    let v = search_error_view(&DataError::Unavailable("connection closed".into()));
    assert!(
        v.text.contains("NOT"),
        "must say this is not a no-match: {}",
        v.text
    );
    assert!(v.text.contains("connection closed"));
    assert_eq!(v.retry, Retry::Now);
}

/// #880: a search can fail for an operator's reason too (the server refused the window's
/// reopen). It is still a FAILED search, never a no-match: the same warning, and no retry-now.
#[test]
fn a_search_that_needs_an_operator_still_never_reads_as_nothing_found() {
    let v = search_error_view(&DataError::NotProvisioned("refused [28P01]".into()));
    assert!(v.text.contains("NOT a \"no match\""), "{}", v.text);
    assert!(v.text.contains("refused [28P01]"), "{}", v.text);
    assert!(!v.text.contains("Try again"), "{}", v.text);
    assert_eq!(v.retry, Retry::AfterOperator);
}

/// #880: the operator's case is not always enrolment — a refused reconnect reaches this arm too,
/// so the sentence claims nothing about what the node may do, only what pressing again does.
#[test]
fn an_operator_case_registration_claims_nothing_about_the_node() {
    let p = register_error_view(&DataError::NotProvisioned("cause".into()), Restored::Kept);
    assert!(!p.text.contains("may not write"), "{}", p.text);
    assert!(p.text.contains("until an operator acts"), "{}", p.text);
    assert!(p.text.contains("Nothing was saved"), "{}", p.text);
}

#[test]
fn a_refusal_withholds_the_retry_and_an_outage_offers_it() {
    let r = register_error_view(&DataError::Refused("bad dob".into()), Restored::Kept);
    assert_eq!(r.retry, Retry::Never);
    assert!(r.text.contains("bad dob"));
    let u = register_error_view(&DataError::Unavailable("timeout".into()), Restored::Kept);
    assert_eq!(u.retry, Retry::Now);
    assert!(u.text.contains("Press Register again"), "{}", u.text);
    let p = register_error_view(&DataError::NotProvisioned("run x".into()), Restored::Kept);
    assert_eq!(p.retry, Retry::AfterOperator);
    assert!(p.text.contains("run x"));
}

#[test]
fn a_dropped_search_says_to_wait_for_the_new_one_not_to_press_register() {
    let v = register_error_view(
        &DataError::Unavailable("timeout".into()),
        Restored::SupersededAndDropped,
    );
    assert_eq!(v.retry, Retry::Never, "there is nothing to retry WITH");
    assert!(v.text.contains("new search"), "{}", v.text);
    assert!(!v.text.contains("Press Register again"), "{}", v.text);
}

#[test]
fn a_token_refusal_is_its_own_sentence_and_never_retried() {
    let v = token_error_view(TokenError::Absent);
    assert_eq!(v.text, TokenError::Absent.to_string());
    assert_eq!(v.retry, Retry::Never);
}

#[test]
fn retry_advice_crosses_to_the_webview_as_snake_case() {
    let json = serde_json::to_value(Retry::AfterOperator).unwrap();
    assert_eq!(json, serde_json::json!("after_operator"));
    // `funnel.js` compares against "now" to keep a search for the retry.
    assert_eq!(
        serde_json::to_value(Retry::Now).unwrap(),
        serde_json::json!("now")
    );
    // `link.js` and `unlink.js` compare against "never" (and "after_operator") to take the
    // judgement button away after a verdict — renamed, the button would stay live on one.
    assert_eq!(
        serde_json::to_value(Retry::Never).unwrap(),
        serde_json::json!("never")
    );
    // ...and `chart_stale.js`'s `behindTheRecord` reads `retry.after_reread` to re-read the chart
    // and take the button away when the screen is behind the record — renamed, the button would
    // stay live. Its value words the re-read (#811): "no", nothing was recorded (the record
    // changed); "unknown", a commit was interrupted (#713) and the clinician's act may have landed.
    assert_eq!(
        serde_json::to_value(Retry::AfterReread(Recorded::No)).unwrap(),
        serde_json::json!({ "after_reread": "no" })
    );
    assert_eq!(
        serde_json::to_value(Retry::AfterReread(Recorded::Unknown)).unwrap(),
        serde_json::json!({ "after_reread": "unknown" })
    );
}

#[test]
fn the_waiting_sentence_names_every_missing_part() {
    let s = waiting_sentence(&trigger_state("John", "")).unwrap();
    assert!(s.contains("1 of 2"), "{s}");
    assert!(s.contains("date of birth"), "{s}");
    assert!(
        s.contains("Register still searches"),
        "advisory, never a gate: {s}"
    );
    assert_eq!(waiting_sentence(&trigger_state("John Smith", "1980")), None);
}

/// PR #674 review #6: a screen-reader clerk must HEAR that possible matches appeared, and what
/// the Register button now means — the prompt is otherwise silent while they type.
#[test]
fn the_prompt_announces_how_many_matches_and_what_register_now_means() {
    let some = prompt_summary(&counts(3, 0, false));
    assert!(some.contains('3'), "{some}");
    assert!(some.contains("none of these"), "{some}");
    let none = prompt_summary(&counts(0, 0, false));
    assert!(none.contains("No existing chart matched"), "{none}");
    assert!(none.contains("record that search"), "{none}");
}

/// Principle 4 on the one sentence that licenses a new chart: a search that showed nobody
/// but did NOT finish must never be announced as "nobody matched" (PR #674 review).
#[test]
fn a_partial_search_that_shows_nobody_never_reads_as_no_match() {
    let s = prompt_summary(&counts(0, 0, true));
    assert!(!s.contains("No existing chart matched"), "{s}");
    assert!(s.contains("NOT"), "{s}");
    let b = browse_summary(0, 0, true);
    assert!(!b.contains("No existing chart matched"), "{b}");
    assert!(b.contains("NOT"), "{b}");
}

/// ADR-0075 decision 4: truncation is said quietly, as a count and a way to narrow —
/// not as "incomplete", which is reserved for a search that did not finish.
#[test]
fn a_cut_prompt_says_how_many_closest_of_how_many() {
    let s = prompt_summary(&counts(5, 98, false));
    assert!(s.contains("5 closest of 103"), "{s}");
    assert!(s.contains("type more to narrow"), "{s}");
    assert!(
        !s.contains("not complete"),
        "a cut list is not a partial search: {s}"
    );
    assert!(s.contains("none of these"), "{s}");
}

#[test]
fn a_cut_prompt_over_a_partial_search_says_both() {
    let s = prompt_summary(&counts(5, 3, true));
    assert!(s.contains("5 closest of 8"), "{s}");
    assert!(s.contains("not complete"), "{s}");
}

/// Final review #3: a prompt that showed nobody because everything was CUT must never say
/// "No existing chart matched" — that sentence licenses a new chart. Unreachable with
/// today's cap of five, and pinned so it stays safe if the cap ever changes.
#[test]
fn a_prompt_cut_to_nobody_never_reads_as_no_match() {
    let s = prompt_summary(&counts(0, 4, false));
    assert!(!s.contains("No existing chart matched"), "{s}");
    assert!(s.contains("4 existing patient(s) matched"), "{s}");
}

/// Exactly the cap and nothing cut: no "closest of" — the list IS every match.
#[test]
fn a_prompt_of_exactly_the_cap_does_not_say_it_was_cut() {
    let s = prompt_summary(&counts(5, 0, false));
    assert!(!s.contains("closest of"), "{s}");
    assert!(s.contains("none of these"), "{s}");
}

/// A partial search wins over a cut one when nobody was shown: the reader must hear that
/// the search did not finish, which is the stronger warning against "no match".
#[test]
fn a_partial_search_cut_to_nobody_says_it_did_not_finish() {
    let s = prompt_summary(&counts(0, 4, true));
    assert!(s.contains("did not finish"), "{s}");
    assert!(!s.contains("No existing chart matched"), "{s}");
}

/// Review focus 5: the one sentence that licenses a new chart is unchanged.
#[test]
fn an_empty_prompt_still_licenses_a_new_chart() {
    assert!(prompt_summary(&counts(0, 0, false)).contains("No existing chart matched"));
}

/// A partial list with rows says it is partial IN the announcement, not only in the
/// separate note — a screen-reader clerk hears the live region, and may hear only that.
#[test]
fn a_partial_list_with_rows_says_so_in_its_announcement() {
    assert!(prompt_summary(&counts(2, 0, true)).contains("not complete"));
    assert!(browse_summary(2, 2, true).contains("not complete"));
    assert!(!browse_summary(2, 2, false).contains("not complete"));
}

/// The browse announcement, from Rust like every other sentence on this screen.
#[test]
fn the_browse_announces_how_many_charts_it_found() {
    assert!(browse_summary(4, 4, false).contains('4'));
    assert!(browse_summary(0, 0, false).contains("No existing chart matched"));
}

/// R3 goldens: when every row is one chart the announcements are byte-identical to what
/// they were before the front door collapsed by person.
#[test]
fn single_chart_summaries_are_byte_identical_to_before_r3() {
    assert_eq!(browse_summary(3, 3, false), "3 existing chart(s) found.");
    assert_eq!(
        browse_summary(3, 3, true),
        "3 existing chart(s) found — the list is not complete."
    );
    assert_eq!(
        prompt_summary(&counts(5, 98, false)),
        "5 existing patient(s) might be this person — the 5 closest of 103 matches, \
         listed below; type more to narrow. Pressing Register now means none of these."
    );
    assert_eq!(
        prompt_summary(&counts(3, 0, true)),
        "3 existing patient(s) might be this person — listed below, but the list is not \
         complete (the reason follows). Pressing Register now means none of these."
    );
}

#[test]
fn a_linked_row_is_counted_as_one_person_and_its_charts_are_named() {
    assert_eq!(
        browse_summary(3, 4, false),
        "3 existing patient(s) found (4 charts)."
    );
    assert_eq!(
        browse_summary(3, 4, true),
        "3 existing patient(s) found (4 charts) — the list is not complete."
    );
    let s = prompt_summary(&PromptCounts {
        shown: 5,
        shown_charts: 6,
        withheld: 98,
        incomplete: false,
    });
    assert!(
        s.starts_with(
            "5 existing patient(s) (6 charts) might be this person — the 5 closest of 103"
        ),
        "{s}"
    );
}

#[test]
fn a_linked_row_in_a_partial_search_says_both_things() {
    let s = prompt_summary(&PromptCounts {
        shown: 3,
        shown_charts: 4,
        withheld: 0,
        incomplete: true,
    });
    assert!(s.contains("3 existing patient(s) (4 charts)"), "{s}");
    assert!(s.contains("the list is not complete"), "{s}");
}

#[test]
fn a_candidate_renders_its_age_and_trust() {
    let v = candidate_view(&sample_candidate());
    assert_eq!(v.age, "46 y");
    assert_eq!(v.trust, "confirmed");
    assert_eq!(v.patient_id, uuid::Uuid::from_u128(5).to_string());
}

#[test]
fn an_unknown_age_renders_as_absence_not_a_blank() {
    let mut c = sample_candidate();
    c.age = None;
    assert_eq!(candidate_view(&c).age, "age not recorded");
}

#[test]
fn a_registration_header_carries_what_was_typed_and_names_absence() {
    let id = uuid::Uuid::from_u128(9);
    let h = header_from_registration(id, " ", None);
    assert_eq!(h.name, "(no name recorded)");
    assert_eq!(h.born, "date of birth not recorded");
    let h = header_from_registration(id, "Mary Poppins", Some("1910"));
    assert_eq!(h.name, "Mary Poppins");
    assert_eq!(h.born, "born 1910");
    // What the node reports for an ordinary registration — never the identity-pending
    // (John-Doe) state, which a standard registration does not open (db/024).
    assert_eq!(h.trust, TrustState::Confirmed.as_str());
    assert_eq!(h.patient_id, id.to_string());
}

#[test]
fn a_picked_candidates_header_is_what_the_list_showed() {
    let h = header_from_candidate(&sample_candidate());
    assert_eq!(h.name, "Samantha Example");
    assert_eq!(h.born, "46 y");
    assert_eq!(h.trust, "confirmed");
}

#[test]
fn a_chart_opened_by_id_says_its_name_was_not_read() {
    let h = header_opened_by_id(uuid::Uuid::from_u128(9));
    assert!(h.name.contains("not read"), "{}", h.name);
}

/// A fully populated candidate, shared with the drift guard in `commands`.
pub(crate) fn sample_candidate() -> Candidate {
    Candidate {
        patient_id: uuid::Uuid::from_u128(5),
        display_name: "Samantha Example".into(),
        age: Some(Age {
            years: 46,
            basis: "dob".into(),
        }),
        trust: TrustState::Confirmed,
        last_activity: None,
        locale: None,
        photo_ref: None,
    }
}

/// A refused reopen's text as `cairn-gui-live` words it (#880), shared with the judgement tests.
/// The assert in each test that uses it keeps it honest: if the live wording moves, they fail.
pub(crate) const REFUSED_REOPEN: &str = "connection to the node was lost and could not be \
    reopened: password authentication failed [28P01]. The node answered and refused the \
    connection, so pressing again fails the same way until an operator acts.";

/// #885: a refused reopen already says its remedy, so the front door's two wrappers must not
/// say it again — each says "until an operator acts" exactly once.
#[test]
fn a_refused_reopen_s_remedy_is_said_once_at_the_front_door() {
    assert!(cairn_gui_live::says_operator_remedy(REFUSED_REOPEN));
    let refused = DataError::NotProvisioned(REFUSED_REOPEN.into());
    let register = register_error_view(&refused, Restored::Kept);
    let search = search_error_view(&refused);
    for v in [&register, &search] {
        assert_eq!(v.text.matches("operator").count(), 1, "{}", v.text);
        assert_eq!(v.retry, Retry::AfterOperator);
    }
    assert!(
        register.text.contains("Nothing was saved"),
        "{}",
        register.text
    );
    assert!(
        search.text.contains("NOT a \"no match\""),
        "{}",
        search.text
    );
}

/// #882: what the front door's provisioning line says, from the node's answer to "may this
/// node write?" — asked afresh by every `funnel_status`, never kept from launch.
#[test]
fn the_provisioning_line_says_the_standing_or_why_it_could_not_be_checked() {
    assert_eq!(
        provisioning_sentence(Ok(ActorStanding::Enrolled), "k"),
        None
    );
    assert_eq!(
        provisioning_sentence(Ok(ActorStanding::NeverEnrolled), "k"),
        standing_sentence(ActorStanding::NeverEnrolled, "k")
    );
    let unknown = provisioning_sentence(Err(DataError::Unavailable("reset".into())), "k").unwrap();
    assert_eq!(
        unknown,
        "Could not check whether this node may write (reset), so registering may be refused."
    );
    let none = provisioning_sentence(Err(DataError::NotFound), "k").unwrap();
    assert!(none.contains("(no answer)"), "{none}");
}
