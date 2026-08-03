//! Node-local gesture timing for the §1.2 paper-parity budget — aggregates ONLY.
//!
//! # Why this shape is the design, not a caveat
//!
//! Per-clinician gesture timings are a productivity-surveillance dataset. Captured the
//! obvious way — user, gesture, duration, timestamp — this table is exactly what a hostile
//! administrator or an acquiring vendor would use to rank clinicians by speed, inside a
//! node the clinician cannot audit. An anti-capture project must not ship that as a side
//! effect of measuring a benchmark. It is also a safety hazard in its own right: clinicians
//! who know they are timed rush the review step the sign-off exists to force.
//!
//! So there is no user id, no patient id, no per-sample row and no timestamp anywhere in
//! this module or its table. There is nothing to re-identify because the identifying
//! columns never exist. What survives is the only thing §1.2 actually needs: what a gesture
//! costs on THIS premise.
//!
//! The rows never sync and never touch the signed clinical event stream — the same category
//! rule the reference-shell design applies to UI preferences (principle 12).
use std::collections::HashMap;

/// A running estimate for one (gesture, list-size) cell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Aggregate {
    pub n: i64,
    pub p50_ms: i32,
    pub p95_ms: i32,
}

/// Fold one observed duration into the running estimate.
///
/// Uses online quantile estimation by stochastic gradient descent: for target quantile q,
/// the estimate moves up by `step * q` when the sample is above it and down by
/// `step * (1 - q)` when below, so it settles where a fraction q of samples fall below.
/// Chosen over an exact quantile because exactness needs the raw samples retained — and
/// retaining raw samples is precisely what this design refuses to do.
///
/// The step is proportional to the current estimate (with a floor of 1 ms) so convergence
/// does not depend on whether a gesture takes 50 ms or 50 s.
pub fn fold_sample(prev: Option<Aggregate>, duration_ms: i32) -> Aggregate {
    // A negative duration is a caller bug (a clock that stepped backwards, a subtraction
    // the wrong way round). Clamp rather than propagate: a negative estimate would violate
    // the table's own CHECK constraint, and this metric must never be able to fail the
    // clinical gesture it is measuring.
    let duration_ms = duration_ms.max(0);
    let Some(prev) = prev else {
        // The first sample is the best estimate of every quantile there is.
        return Aggregate {
            n: 1,
            p50_ms: duration_ms,
            p95_ms: duration_ms,
        };
    };
    Aggregate {
        n: prev.n.saturating_add(1),
        p50_ms: nudge(prev.p50_ms, duration_ms, 0.50),
        p95_ms: nudge(prev.p95_ms, duration_ms, 0.95),
    }
}

/// One SGD step of a quantile estimate toward a new sample. Never returns a negative.
fn nudge(estimate: i32, sample: i32, q: f64) -> i32 {
    let step = ((estimate as f64) / 20.0).max(1.0);
    let delta = if sample > estimate {
        step * q
    } else {
        -step * (1.0 - q)
    };
    ((estimate as f64) + delta).round().max(0.0) as i32
}

/// Which size bucket a list of `n` items falls in.
///
/// Coarse on purpose. A finer partition would let someone reconstruct a specific chart's
/// size from the aggregate table, which is the sort of leak this whole module is built to
/// avoid. Three buckets are enough to see whether cost scales with list length.
pub fn size_bucket(items: usize) -> &'static str {
    match items {
        0..=3 => "1-3",
        4..=8 => "4-8",
        _ => "9+",
    }
}

/// Fold one observed gesture into its aggregate cell.
///
/// Read-modify-write in one statement pair under the caller's connection. A lost update
/// under concurrency costs one sample out of thousands, which is immaterial to a running
/// estimate — and a lock here would let a metric stall a clinical gesture, which is not a
/// trade this tier is allowed to make.
pub async fn record_gesture(
    client: &(impl tokio_postgres::GenericClient + Sync),
    gesture_kind: &str,
    list_items: usize,
    duration_ms: i32,
) -> anyhow::Result<()> {
    let bucket = size_bucket(list_items);
    let existing = client
        .query_opt(
            "SELECT n, p50_ms, p95_ms FROM ui_gesture_timing \
             WHERE gesture_kind = $1 AND size_bucket = $2",
            &[&gesture_kind, &bucket],
        )
        .await?
        .map(|row| Aggregate {
            n: row.get("n"),
            p50_ms: row.get::<_, Option<i32>>("p50_ms").unwrap_or(0),
            p95_ms: row.get::<_, Option<i32>>("p95_ms").unwrap_or(0),
        });

    let next = fold_sample(existing, duration_ms);
    client
        .execute(
            "INSERT INTO ui_gesture_timing (gesture_kind, size_bucket, n, p50_ms, p95_ms) \
             VALUES ($1, $2, $3, $4, $5) \
             ON CONFLICT (gesture_kind, size_bucket) DO UPDATE \
             SET n = EXCLUDED.n, p50_ms = EXCLUDED.p50_ms, p95_ms = EXCLUDED.p95_ms",
            &[&gesture_kind, &bucket, &next.n, &next.p50_ms, &next.p95_ms],
        )
        .await?;
    Ok(())
}

/// Every aggregate cell, keyed by (gesture kind, size bucket). This is the whole reporting
/// surface — there is no per-sample read because there are no per-sample rows.
pub async fn read_aggregates(
    client: &(impl tokio_postgres::GenericClient + Sync),
) -> anyhow::Result<HashMap<(String, String), Aggregate>> {
    Ok(client
        .query(
            "SELECT gesture_kind, size_bucket, n, p50_ms, p95_ms FROM ui_gesture_timing",
            &[],
        )
        .await?
        .iter()
        .map(|row| {
            (
                (row.get("gesture_kind"), row.get("size_bucket")),
                Aggregate {
                    n: row.get("n"),
                    p50_ms: row.get::<_, Option<i32>>("p50_ms").unwrap_or(0),
                    p95_ms: row.get::<_, Option<i32>>("p95_ms").unwrap_or(0),
                },
            )
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_sample_seeds_both_estimates() {
        let a = fold_sample(None, 1_200);
        assert_eq!(a.n, 1);
        assert_eq!(a.p50_ms, 1_200);
        assert_eq!(a.p95_ms, 1_200);
    }

    #[test]
    fn a_constant_stream_converges_to_that_constant() {
        let mut a = None;
        for _ in 0..500 {
            a = Some(fold_sample(a, 1_000));
        }
        let a = a.unwrap();
        assert_eq!(a.n, 500);
        assert!(
            (a.p50_ms as i64 - 1_000).abs() <= 50,
            "p50 drifted: {}",
            a.p50_ms
        );
        assert!(
            (a.p95_ms as i64 - 1_000).abs() <= 50,
            "p95 drifted: {}",
            a.p95_ms
        );
    }

    /// The point of tracking p95 separately: on a stream where most gestures are fast and
    /// a few are slow, p95 must sit above p50 — that tail is what a budget has to cover.
    #[test]
    fn p95_sits_above_p50_on_a_skewed_stream() {
        let mut a = None;
        for i in 0..1_000 {
            let sample = if i % 10 == 0 { 5_000 } else { 1_000 };
            a = Some(fold_sample(a, sample));
        }
        let a = a.unwrap();
        assert!(a.p95_ms > a.p50_ms, "p50={} p95={}", a.p50_ms, a.p95_ms);
    }

    #[test]
    fn buckets_partition_list_sizes() {
        assert_eq!(size_bucket(0), "1-3");
        assert_eq!(size_bucket(1), "1-3");
        assert_eq!(size_bucket(3), "1-3");
        assert_eq!(size_bucket(4), "4-8");
        assert_eq!(size_bucket(8), "4-8");
        assert_eq!(size_bucket(9), "9+");
        assert_eq!(size_bucket(200), "9+");
    }

    /// The estimator must never emit a negative or absurd duration, whatever it is fed.
    #[test]
    fn estimates_stay_non_negative_on_a_zero_stream() {
        let mut a = None;
        for _ in 0..200 {
            a = Some(fold_sample(a, 0));
        }
        let a = a.unwrap();
        assert_eq!(a.p50_ms, 0);
        assert_eq!(a.p95_ms, 0);
    }

    /// A negative duration can only be a caller bug (a clock that went backwards, a
    /// subtraction in the wrong order). It must be clamped, never folded in as-is: a
    /// negative estimate would violate the table's own CHECK and take down the write of a
    /// metric that must never be able to fail a clinical gesture.
    #[test]
    fn a_negative_sample_is_clamped_rather_than_stored() {
        assert_eq!(fold_sample(None, -5).p50_ms, 0);
    }
}
