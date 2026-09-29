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
use crate::funnel::view::ErrorView;
use crate::state::AppState;
use cairn_gui_funnel::FormSnapshot;

/// The panel's summary line: how many OTHER charts the list shows, whether the search was
/// partial, and — when any matched — that this record's own charts were left out, so an empty
/// list after typing the open patient's own name is explained rather than read as "nobody".
pub fn link_search_summary(other: usize, own: usize, incomplete: bool) -> String {
    let base = match (other, incomplete) {
        (0, false) => "No other chart matched.".to_string(),
        (0, true) => "The search did not finish, and showed no other chart — this is NOT a \
                      \"no match\"."
            .to_string(),
        (n, false) => format!("{n} other chart(s) found."),
        (n, true) => format!("{n} other chart(s) found — the list is not complete."),
    };
    if own == 0 {
        base
    } else {
        format!("{base} {own} chart(s) of this record also matched and are not listed.")
    }
}

/// `view` with every chart of this record (`in_record`, the ids the window displays) removed,
/// and its summary re-worded over what remains. Pure.
pub fn link_search_view(view: BrowseView, in_record: &[String]) -> BrowseView {
    let total = view.candidates.len();
    let candidates: Vec<_> = view
        .candidates
        .into_iter()
        .filter(|c| !in_record.contains(&c.patient_id))
        .collect();
    let own = total - candidates.len();
    BrowseView {
        summary: link_search_summary(candidates.len(), own, view.incomplete_reason.is_some()),
        revision: view.revision,
        candidates,
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
    use crate::funnel::view::CandidateView;

    fn candidate(id: &str) -> CandidateView {
        CandidateView {
            patient_id: id.into(),
            name: "N".into(),
            age: "46 y".into(),
            trust: "confirmed".into(),
        }
    }

    fn browse(ids: &[&str], incomplete: bool) -> BrowseView {
        BrowseView {
            revision: 7,
            candidates: ids.iter().map(|i| candidate(i)).collect(),
            summary: "the front door's sentence".into(),
            incomplete_reason: incomplete.then(|| "timed out".to_string()),
        }
    }

    /// The case that motivated this module: the clerk types the open patient's own name, and
    /// the only match is this record's own chart. The list is empty and the line says why.
    #[test]
    fn only_this_records_own_chart_matched() {
        let v = link_search_view(browse(&["a"], false), &["a".into()]);
        assert!(v.candidates.is_empty());
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
        let ids: Vec<&str> = v.candidates.iter().map(|c| c.patient_id.as_str()).collect();
        assert_eq!(ids, vec!["b", "c"]);
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
}
