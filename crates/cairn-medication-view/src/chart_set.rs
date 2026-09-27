//! The set of charts a read covers — one chart, or every chart in a link component
//! (ADR-0076 decision 1).
//!
//! A chart command must name the set the clinician SAW and refuse when it changed
//! (decision 3). That comparison crosses an IPC hop and two database reads, so the set's
//! ORDER must never make two equal sets compare unequal: `new` sorts and deduplicates, and
//! it and `single` (one chart, trivially in order) are the only ways a `ChartSet` is built.
//! It is never empty — a read always covers at least the chart that was opened.
use serde::Serialize;
use uuid::Uuid;

/// Serialize only. Do NOT derive `Deserialize`: a plain derive would build a `ChartSet`
/// straight from the wire, skipping `new`'s sort and dedup, and `contains` (a binary search)
/// would then silently answer wrong. If one is ever needed, use
/// `#[serde(try_from = "Vec<Uuid>")]` through `new`. Today the window sends the displayed set
/// back as `Vec<String>` and parses it through `new` (`cairn-gui-tauri`'s
/// `check_displayed_set`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct ChartSet(Vec<Uuid>);

impl ChartSet {
    /// One chart that is linked to nothing — the pre-ADR-0076 case, and still the common one.
    pub fn single(chart: Uuid) -> Self {
        Self(vec![chart])
    }

    /// The set of `charts`, in canonical order. `None` when there are none: a read that
    /// covers no chart at all is a bug in the caller, not a state to render.
    pub fn new(charts: impl IntoIterator<Item = Uuid>) -> Option<Self> {
        let mut v: Vec<Uuid> = charts.into_iter().collect();
        v.sort();
        v.dedup();
        (!v.is_empty()).then_some(Self(v))
    }

    pub fn members(&self) -> &[Uuid] {
        &self.0
    }

    pub fn contains(&self, chart: &Uuid) -> bool {
        self.0.binary_search(chart).is_ok()
    }

    /// Whether every one of `charts` is in this set. The cross-patient test: a medication
    /// group is a hazard exactly when its charts are NOT all inside the set being read.
    pub fn contains_all(&self, charts: &[Uuid]) -> bool {
        charts.iter().all(|c| self.contains(c))
    }

    /// More than one chart: the header shows the linked members.
    pub fn is_linked(&self) -> bool {
        self.0.len() > 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    #[test]
    fn the_order_given_never_changes_the_set() {
        let a = ChartSet::new([u(3), u(1), u(2)]).unwrap();
        let b = ChartSet::new([u(2), u(3), u(1)]).unwrap();
        assert_eq!(a, b, "the same charts in another order are the same set");
        assert_eq!(a.members(), &[u(1), u(2), u(3)]);
    }

    #[test]
    fn a_repeated_chart_counts_once() {
        let s = ChartSet::new([u(1), u(1), u(2)]).unwrap();
        assert_eq!(s.members(), &[u(1), u(2)]);
    }

    #[test]
    fn an_empty_set_cannot_be_made() {
        assert!(ChartSet::new(std::iter::empty()).is_none());
    }

    #[test]
    fn a_single_chart_is_not_linked_and_two_are() {
        assert!(!ChartSet::single(u(1)).is_linked());
        assert!(ChartSet::new([u(1), u(2)]).unwrap().is_linked());
    }

    #[test]
    fn contains_all_is_a_subset_test() {
        let s = ChartSet::new([u(1), u(2)]).unwrap();
        assert!(s.contains_all(&[u(2), u(1)]));
        assert!(!s.contains_all(&[u(1), u(3)]));
        assert!(s.contains_all(&[]), "the empty list is inside every set");
    }

    #[test]
    fn it_serializes_as_a_plain_array_of_ids() {
        let s = ChartSet::new([u(2), u(1)]).unwrap();
        assert_eq!(
            serde_json::to_string(&s).unwrap(),
            format!("[\"{}\",\"{}\"]", u(1), u(2))
        );
    }
}
