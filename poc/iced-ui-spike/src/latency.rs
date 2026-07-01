//! Pure latency statistics for Spike 0004 claim **L**.
//!
//! Why this module exists: the live GUI harness (`--latency`) only needs to
//! *collect* a vector of keystroke-to-paint timings; computing percentiles is a
//! pure, testable function that must not live tangled inside the event loop.
//! Separating it lets CI verify the summariser against known vectors
//! (claim **L1**) without a display, so the operator's Pi run (claim **L2**)
//! reduces to "collect samples, hand them to [`summarize`], compare p95 to the
//! paper-parity floor".
//!
//! All functions here are pure (explicit input → explicit output, no I/O, no
//! globals) per house rule 4 — favour small reusable functions over cleverness.

/// A summary of a batch of latency samples, in milliseconds.
///
/// Percentiles use the **nearest-rank** method on the ascending-sorted samples:
/// the p-th percentile is the value at rank `ceil(p/100 * n)` (1-indexed). This
/// is deterministic and interpolation-free, which makes the unit tests exact
/// rather than approximate — important when the result gates a paper-parity claim.
#[derive(Debug, Clone, PartialEq)]
pub struct Summary {
    /// Number of samples summarised.
    pub n: usize,
    /// Arithmetic mean (ms).
    pub mean_ms: f64,
    /// 50th percentile / median (ms).
    pub p50_ms: f64,
    /// 95th percentile (ms) — the figure compared to the floor in claim L2.
    pub p95_ms: f64,
    /// 99th percentile (ms).
    pub p99_ms: f64,
    /// Worst observed sample (ms).
    pub max_ms: f64,
}

/// Summarise a batch of latency samples (in ms).
///
/// Returns `None` for an empty batch — there is no honest percentile of zero
/// samples, and returning a zeroed struct would silently read as "instant",
/// which is exactly the kind of precise-untruth the fourth governing principle
/// forbids. Callers must handle the no-data case explicitly.
///
/// Non-finite samples (NaN/∞) are rejected by sorting them to the end and would
/// corrupt percentiles, so we filter them out first and treat an all-non-finite
/// batch as empty.
pub fn summarize(samples: &[f64]) -> Option<Summary> {
    // Drop non-finite samples: a stray NaN from a timer glitch must not poison
    // the whole summary (and NaN breaks total ordering).
    let mut sorted: Vec<f64> = samples.iter().copied().filter(|x| x.is_finite()).collect();
    if sorted.is_empty() {
        return None;
    }
    // f64 has no total Ord; we already removed NaN, so partial_cmp is total here.
    sorted.sort_by(|a, b| a.partial_cmp(b).expect("non-finite filtered above"));

    let n = sorted.len();
    let sum: f64 = sorted.iter().sum();
    Some(Summary {
        n,
        mean_ms: sum / n as f64,
        p50_ms: nearest_rank(&sorted, 50.0),
        p95_ms: nearest_rank(&sorted, 95.0),
        p99_ms: nearest_rank(&sorted, 99.0),
        max_ms: sorted[n - 1],
    })
}

/// Nearest-rank percentile of an **ascending-sorted, non-empty** slice.
///
/// rank = ceil(p/100 * n), clamped to [1, n]; the result is the value at that
/// 1-indexed rank. Kept private and tiny so the percentile definition lives in
/// exactly one place.
fn nearest_rank(sorted_asc: &[f64], p: f64) -> f64 {
    debug_assert!(!sorted_asc.is_empty());
    let n = sorted_asc.len();
    let rank = (p / 100.0 * n as f64).ceil() as usize;
    let idx = rank.clamp(1, n) - 1;
    sorted_asc[idx]
}

/// Does this summary clear the paper-parity interactive floor?
///
/// Claim **L2**'s threshold is expressed on p95 (keystroke-to-paint), so a
/// deployment/operator passes a single budget in and gets a verdict. Pure, so
/// the threshold lives with the data, not buried in the GUI.
pub fn within_paper_parity_floor(summary: &Summary, p95_budget_ms: f64) -> bool {
    summary.p95_ms <= p95_budget_ms
}

#[cfg(test)]
mod tests {
    use super::*;

    // Helper: assert two f64 are equal to the bit (our percentiles pick actual
    // samples, so equality is exact — no epsilon needed).
    fn eq(a: f64, b: f64) -> bool {
        a == b
    }

    #[test]
    fn empty_batch_is_none_not_zero() {
        // The fourth-principle point: no data must not masquerade as "0 ms".
        assert!(summarize(&[]).is_none());
    }

    #[test]
    fn all_non_finite_is_treated_as_empty() {
        assert!(summarize(&[f64::NAN, f64::INFINITY]).is_none());
    }

    #[test]
    fn single_sample_is_every_percentile() {
        let s = summarize(&[7.0]).unwrap();
        assert_eq!(s.n, 1);
        assert!(eq(s.p50_ms, 7.0));
        assert!(eq(s.p95_ms, 7.0));
        assert!(eq(s.p99_ms, 7.0));
        assert!(eq(s.max_ms, 7.0));
        assert!(eq(s.mean_ms, 7.0));
    }

    #[test]
    fn nearest_rank_on_one_to_ten() {
        // Canonical vector 1..=10 — nearest-rank gives exact, well-known answers.
        let v: Vec<f64> = (1..=10).map(|x| x as f64).collect();
        let s = summarize(&v).unwrap();
        assert_eq!(s.n, 10);
        assert!(eq(s.mean_ms, 5.5));
        assert!(eq(s.p50_ms, 5.0)); // ceil(0.50*10)=5 -> idx4 -> 5
        assert!(eq(s.p95_ms, 10.0)); // ceil(0.95*10)=10 -> idx9 -> 10
        assert!(eq(s.p99_ms, 10.0)); // ceil(0.99*10)=10 -> idx9 -> 10
        assert!(eq(s.max_ms, 10.0));
    }

    #[test]
    fn unsorted_input_is_handled() {
        // Summary must not depend on input order.
        let a = summarize(&[10.0, 1.0, 5.0, 3.0, 8.0]).unwrap();
        let b = summarize(&[1.0, 3.0, 5.0, 8.0, 10.0]).unwrap();
        assert_eq!(a, b);
        assert!(eq(a.p50_ms, 5.0)); // ceil(0.5*5)=3 -> idx2 -> 5
    }

    #[test]
    fn non_finite_samples_are_filtered_but_finite_kept() {
        let s = summarize(&[2.0, f64::NAN, 4.0, f64::INFINITY, 6.0]).unwrap();
        assert_eq!(s.n, 3); // only the three finite samples
        assert!(eq(s.max_ms, 6.0));
    }

    #[test]
    fn floor_check_compares_on_p95() {
        let s = summarize(&[10.0, 20.0, 30.0, 40.0, 200.0]).unwrap();
        // p95: ceil(0.95*5)=5 -> idx4 -> 200 (the spike outlier dominates p95)
        assert!(eq(s.p95_ms, 200.0));
        assert!(!within_paper_parity_floor(&s, 100.0));
        assert!(within_paper_parity_floor(&s, 250.0));
    }
}
