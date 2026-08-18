//! §5.9 — how a chart report reads.
//!
//! PURE. No database, no I/O, no `tokio_postgres` import. Every honesty claim this surface
//! makes is a sentence — "this is not a clean bill of health", "this node may hold no
//! custody", "this list is not complete" — and a sentence that only exists inside a
//! `println!` in `main.rs` can be tested only by running the binary against a live cluster,
//! which is why nobody ever did. Keeping the wording here makes each claim a unit test.
//!
//! Precedent: `crate::safety::render_safety_line`, which is pure for the same reason.
use super::report::ChartReport;

/// Render one chart's §5.9 report as the lines an operator reads, in order.
///
/// The chart grade comes FIRST and keeps its exact wire shape — see the contract test. The
/// per-thread breakdown follows. Later tasks insert warning blocks between the two, which
/// is deliberate: a warning that appears forty thread-lines below the claim it qualifies is
/// a warning nobody reads.
///
/// Returns lines rather than printing them so the caller owns the I/O and the wording stays
/// testable. The chart label is a PARAMETER rather than something the caller splices in
/// afterwards: an earlier draft had `main.rs` rewrite the leading `chart:` token, which also
/// rewrites the custody-blind line ending `...stand on this chart:` and mangles it. Passing
/// the label costs one argument and cannot mis-target.
pub fn render_chart_report(chart: &str, r: &ChartReport) -> Vec<String> {
    let mut out = Vec::new();
    out.push(format!(
        "chart {}: {} (winning subject: {}{})",
        chart,
        r.chart_grade,
        r.chart_source,
        match &r.chart_content_address {
            Some(ca) => format!(", withdraws={ca}"),
            None => String::new(),
        }
    ));
    out.extend(render_threads(r));
    out.push(
        "(report only — nothing is withheld; enforcement needs custody narrowing, \
         #232 part C)"
            .to_string(),
    );
    out
}

/// The per-thread breakdown. Task 5 replaces the empty branch — today it reproduces
/// `main.rs`'s current wording exactly, so that replacement is visible as a test edit.
fn render_threads(r: &ChartReport) -> Vec<String> {
    if r.threads.is_empty() {
        return vec!["  no medication threads on this chart".to_string()];
    }
    r.threads
        .iter()
        .map(|t| {
            format!(
                "  thread {}: {} (winning subject: {}{})",
                t.thread_id,
                t.grade,
                t.source,
                match &t.content_address {
                    Some(ca) => format!(", withdraws={ca}"),
                    None => String::new(),
                }
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sensitivity::report::ThreadGrade;

    /// A chart with nothing wrong: one grade line, one thread line, the standing footer.
    fn healthy() -> ChartReport {
        ChartReport {
            chart_grade: "routine".into(),
            chart_source: "none".into(),
            chart_content_address: None,
            threads: vec![ThreadGrade {
                thread_id: uuid::Uuid::nil(),
                grade: "routine".into(),
                source: "none".into(),
                content_address: None,
            }],
        }
    }

    #[test]
    fn the_grade_line_keeps_its_documented_shape() {
        // `sensitivity-withdraw --withdraws` documents its argument as "the hex
        // content_address, as patient-sensitivity prints it". That is a CONTRACT: an
        // earlier draft of ChartReport dropped the address entirely and a hand exercise of
        // the CLI caught it. Pin the shape so the next refactor cannot quietly break it.
        let mut r = healthy();
        r.chart_grade = "sequestered".into();
        r.chart_source = "chart-wide".into();
        r.chart_content_address = Some("a3f".into());
        let lines = render_chart_report("C", &r);
        assert_eq!(
            lines[0],
            "chart C: sequestered (winning subject: chart-wide, withdraws=a3f)"
        );
    }

    #[test]
    fn a_chart_with_no_assertion_names_no_address() {
        let lines = render_chart_report("C", &healthy());
        assert_eq!(lines[0], "chart C: routine (winning subject: none)");
    }

    #[test]
    fn a_healthy_chart_raises_no_warning() {
        // The anti-vacuity test for every later task: if a warning ever appears on a chart
        // with nothing wrong, the operator learns to ignore warnings, and the surface has
        // made things worse than silence.
        let lines = render_chart_report("C", &healthy());
        assert!(
            !lines.iter().any(|l| l.contains('⚠')),
            "a healthy chart must print no warning: {lines:?}"
        );
    }
}
