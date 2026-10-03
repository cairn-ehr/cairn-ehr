//! The "Same person as…" panel's own search (R2b-1): the front door's `browse`, minus this
//! record's own charts, with a summary that counts what the list SHOWS.
//!
//! Why not reuse `browse` as-is: the panel must leave out the charts already in this record
//! (there is nothing to compare them against), and `browse`'s summary counts every candidate.
//! Filtering in the webview left a status line such as "1 existing chart(s) found." over an
//! empty list when the clerk typed the open patient's own name — a sentence about a list that
//! is not on screen (principle 4), and a decision the webview is not supposed to make (PR #707
//! review). So the filter and the sentence both live here, pure and tested.
use crate::funnel::commands::{browse_impl, BrowseView};
use crate::funnel::rows::charts_suffix;
use crate::funnel::view::ErrorView;
use crate::state::AppState;
use cairn_gui_funnel::FormSnapshot;

/// The panel's summary line: how many OTHER people the list shows (and how many charts they
/// hold, when more), whether the search was partial, and — when any matched — that this record's
/// own charts were left out, so an empty list after typing the open patient's own name is
/// explained rather than read as "nobody".
///
/// `own_charts` counts CHARTS (every chart of every row left out), because that is what the clerk
/// recognises as "this record". With one chart per person the sentences are the pre-R3 ones
/// verbatim.
pub fn link_search_summary(
    other_people: usize,
    other_charts: usize,
    own_charts: usize,
    incomplete: bool,
) -> String {
    let base = if other_people == 0 {
        if incomplete {
            "The search did not finish, and showed no other chart — this is NOT a \"no match\"."
                .to_string()
        } else {
            "No other chart matched.".to_string()
        }
    } else {
        // "chart(s)" while each person is one chart; "patient(s) (C charts)" once rows are linked.
        let linked = other_charts != other_people;
        let noun = if linked { "patient(s)" } else { "chart(s)" };
        let charts = charts_suffix(other_people, other_charts);
        let partial = if incomplete {
            " — the list is not complete"
        } else {
            ""
        };
        format!("{other_people} other {noun} found{charts}{partial}.")
    };
    if own_charts == 0 {
        base
    } else {
        format!("{base} {own_charts} chart(s) of this record also matched and are not listed.")
    }
}

/// `view` with every ROW that holds any chart of this record (`in_record`, the ids the window
/// displays) removed, and its summary re-worded over what remains. A row is one person, so a
/// row holding one of this record's charts is this record whole — even a member the search itself
/// did not match is left out with it. Pure.
pub fn link_search_view(view: BrowseView, in_record: &[String]) -> BrowseView {
    let (own, others): (Vec<_>, Vec<_>) = view.people.into_iter().partition(|row| {
        row.members
            .iter()
            .any(|c| in_record.contains(&c.patient_id))
    });
    let own_charts: usize = own.iter().map(|r| r.members.len()).sum();
    let other_charts: usize = others.iter().map(|r| r.members.len()).sum();
    BrowseView {
        summary: link_search_summary(
            others.len(),
            other_charts,
            own_charts,
            view.incomplete_reason.is_some(),
        ),
        revision: view.revision,
        people: others,
        incomplete_reason: view.incomplete_reason,
    }
}

/// The panel's search. `charts` is the set the window displays for the open chart; a stale set
/// only changes which rows are left out, never what a Compare may act on — `compare_records`
/// re-checks everything itself.
#[tauri::command]
pub async fn link_search(
    state: tauri::State<'_, AppState>,
    form: FormSnapshot,
    charts: Vec<String>,
) -> Result<BrowseView, ErrorView> {
    Ok(link_search_view(browse_impl(&state, form).await?, &charts))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::funnel::rows::PersonRowView;
    use crate::funnel::view::CandidateView;

    fn candidate(id: &str) -> CandidateView {
        CandidateView {
            patient_id: id.into(),
            name: "N".into(),
            age: "46 y".into(),
            trust: "confirmed".into(),
        }
    }

    /// A browse answer made of person rows: each inner slice is one row's chart ids.
    fn browse_rows(rows: &[&[&str]], incomplete: bool) -> BrowseView {
        BrowseView {
            revision: 7,
            people: rows
                .iter()
                .map(|r| PersonRowView {
                    label: (r.len() > 1).then(|| "a label".to_string()),
                    members: r.iter().map(|i| candidate(i)).collect(),
                })
                .collect(),
            summary: "the front door's sentence".into(),
            incomplete_reason: incomplete.then(|| "timed out".to_string()),
        }
    }

    /// One chart per row, the shape every pre-R3 test used.
    fn browse(ids: &[&str], incomplete: bool) -> BrowseView {
        let rows: Vec<Vec<&str>> = ids.iter().map(|i| vec![*i]).collect();
        let rows: Vec<&[&str]> = rows.iter().map(|r| r.as_slice()).collect();
        browse_rows(&rows, incomplete)
    }

    fn first_ids(v: &BrowseView) -> Vec<&str> {
        v.people
            .iter()
            .map(|p| p.members[0].patient_id.as_str())
            .collect()
    }

    /// The case that motivated this module: the clerk types the open patient's own name, and
    /// the only match is this record's own chart. The list is empty and the line says why.
    #[test]
    fn only_this_records_own_chart_matched() {
        let v = link_search_view(browse(&["a"], false), &["a".into()]);
        assert!(v.people.is_empty());
        assert_eq!(
            v.summary,
            "No other chart matched. 1 chart(s) of this record also matched and are not listed."
        );
        assert_eq!(
            v.revision, 7,
            "the revision is echoed, so a late answer is dropped"
        );
    }

    #[test]
    fn the_count_is_of_the_rows_shown() {
        let v = link_search_view(browse(&["a", "b", "c"], false), &["a".into()]);
        assert_eq!(first_ids(&v), vec!["b", "c"]);
        assert!(
            v.summary.starts_with("2 other chart(s) found."),
            "counts the two rows on screen, not the three the search returned"
        );
    }

    #[test]
    fn a_plain_answer_says_nothing_about_this_record() {
        let v = link_search_view(browse(&["b"], false), &["a".into()]);
        assert_eq!(v.summary, "1 other chart(s) found.");
    }

    /// A partial answer never reads as a complete one — the front door's rule, kept.
    #[test]
    fn a_partial_answer_still_says_so() {
        let empty = link_search_view(browse(&[], true), &["a".into()]);
        assert!(empty.summary.contains("NOT a \"no match\""));
        let some = link_search_view(browse(&["b"], true), &["a".into()]);
        assert!(some.summary.contains("not complete"));
        assert!(some.incomplete_reason.is_some());
    }

    /// The open record is {a, b}; the search matched only `a` and returned the row [a, b]. The
    /// whole row is this record, so BOTH its charts are left out and counted as own.
    #[test]
    fn this_records_whole_row_is_left_out_even_its_unmatched_member() {
        let v = link_search_view(
            browse_rows(&[&["a", "b"], &["x"]], false),
            &["a".into(), "b".into()],
        );
        assert_eq!(v.people.len(), 1);
        assert_eq!(v.people[0].members[0].patient_id, "x");
        assert_eq!(
            v.summary,
            "1 other chart(s) found. 2 chart(s) of this record also matched and are not listed."
        );
    }

    /// `link.js` is untyped, so a Rust rename renders `undefined`. This guards ONLY the search
    /// answer (`found` in `runLinkSearch`); the comparison view's guard is the rest of #715.
    #[test]
    fn link_js_search_reads_no_field_the_backend_does_not_send() {
        use crate::commands::tests::fields_read_in;
        let js = include_str!("../../src-ui/link.js");
        let sent = serde_json::to_value(link_search_view(
            browse_rows(&[&["x", "y"]], false),
            &["a".into()],
        ))
        .unwrap();
        let available: Vec<String> = sent.as_object().unwrap().keys().cloned().collect();
        let read = fields_read_in(js, "found");
        assert!(
            read.contains("people") && read.contains("summary"),
            "{read:?}"
        );
        for field in read {
            assert!(
                available.contains(&field),
                "link.js reads `found.{field}`, which the backend does not send: {available:?}"
            );
        }
    }

    #[test]
    fn a_linked_other_person_is_one_row() {
        let v = link_search_view(browse_rows(&[&["x", "y"]], false), &["a".into()]);
        assert_eq!(v.people.len(), 1);
        assert_eq!(v.summary, "1 other patient(s) found (2 charts).");
    }

    #[test]
    fn a_linked_other_person_in_a_partial_search_says_so() {
        let v = link_search_view(browse_rows(&[&["x", "y"]], true), &["a".into()]);
        assert_eq!(
            v.summary,
            "1 other patient(s) found (2 charts) — the list is not complete."
        );
    }
}
