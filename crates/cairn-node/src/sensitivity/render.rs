//! §5.9 — how a chart report reads.
//!
//! PURE. No database, no I/O, no `tokio_postgres` import. Every honesty claim this surface
//! makes is a sentence — "this is not a clean bill of health", "this node may hold no
//! custody", "this list is not complete" — and a sentence that only exists inside a
//! `println!` in `main.rs` can be tested only by running the binary against a live cluster,
//! which is why nobody ever did. Keeping the wording here makes each claim a unit test.
//!
//! Precedent: `crate::safety::render_safety_line`, which is pure for the same reason.
use super::report::{
    ChartReport, DeferredSensitivityEvent, IneffectiveWithdrawal, SafetyOverclaim,
};

/// Why a worklist row is on the worklist, in words. Pure and TOTAL — every input has an
/// output, including one this build has never seen.
///
/// The two reasons have DIFFERENT fixes, which is why they get different sentences rather
/// than a shared "did not take effect": `inert` means nobody this node can hold responsible
/// stands behind the claim (the fix is an accountable human re-asserting it), while
/// `stranger-attested` means someone did stand behind it but has no prior presence on this
/// chart (the fix is a look at who is asserting on this chart at all).
///
/// The catch-all points the reader AT the row rather than rendering an unknown reason as
/// though it were understood — the same discipline as `super::subject_kind_phrase`.
pub fn withdrawal_reason_explanation(reason: &str) -> &'static str {
    match reason {
        "inert" => {
            "no accountable human this node can hold responsible stands behind it (ADR-0064)"
        }
        "stranger-attested" => "attested, but by an actor with no prior presence on this chart",
        _ => "an unrecognised reason from a newer node — read the row itself",
    }
}

/// The warning block for withdrawals that landed and changed nothing. Empty when there are
/// none, so a healthy chart stays silent.
fn render_ineffective_withdrawals(ws: &[IneffectiveWithdrawal]) -> Vec<String> {
    if ws.is_empty() {
        return Vec::new();
    }
    let mut out = vec![format!(
        "⚠ {} withdrawal(s) on this chart did NOT take effect — the grade above may not be \
         what someone intended",
        ws.len()
    )];
    for w in ws {
        out.push(format!(
            "    {:<18} withdraws={}  by actor={}  origin={}",
            w.reason,
            w.withdraws,
            w.responsible_actor_id
                .as_deref()
                .unwrap_or("(none this node can name)"),
            w.node_origin
        ));
        out.push(format!("      rationale: {:?}", w.rationale));
        out.push(format!("      → {}", withdrawal_reason_explanation(&w.reason)));
    }
    out
}

/// The warning block for sensitivity events this node holds but cannot apply.
fn render_deferred(ds: &[DeferredSensitivityEvent]) -> Vec<String> {
    if ds.is_empty() {
        return Vec::new();
    }
    let mut out = vec![format!(
        "⚠ {} sensitivity event(s) on this chart are DEFERRED — admitted, powerless, not \
         applied to any grade above",
        ds.len()
    )];
    for d in ds {
        out.push(format!(
            "    {}  {}  {}  {}",
            d.event_id,
            d.event_type,
            d.admitted_at,
            d.adjudication_error
                .as_deref()
                .unwrap_or("(not yet re-adjudicated)")
        ));
    }
    out
}

/// The warning block for recorded safety overclaims.
fn render_overclaims(os: &[SafetyOverclaim]) -> Vec<String> {
    if os.is_empty() {
        return Vec::new();
    }
    let mut out = vec![format!(
        "⚠ {} safety overclaim(s) recorded on this chart — a rung finer than the grade \
         licensed was published, and a published byte cannot be clawed back",
        os.len()
    )];
    for o in os {
        out.push(format!(
            "    event={}  emitted={}  licensed={}",
            o.content_address, o.emitted_rung, o.licensed_rung
        ));
    }
    out
}

/// What an operator sees after asserting a grade: what they asked for, and what now stands.
///
/// TWO FACTS, ALWAYS. Printing only the standing grade would render a thread-scoped
/// `restricted` under a chart-wide `sequestered` as bare "sequestered", which reads as a
/// silent upgrade of the operator's own act. Printing only the asserted grade would claim
/// an effect that may not have occurred. There is deliberately no shortened form for the
/// agreeing case: a reader who learns that one grade means agreement can no longer read a
/// one-grade line as anything.
pub fn render_assert_readback(asserted: &str, standing: &str, winning_subject: &str) -> String {
    format!(
        "asserted {asserted}; {standing} now stands on this chart (winning subject: \
         {winning_subject})"
    )
}

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
    out.extend(render_ineffective_withdrawals(&r.ineffective_withdrawals));
    out.extend(render_deferred(&r.deferred));
    out.extend(render_overclaims(&r.overclaims));
    out.extend(render_threads(r));
    // DECLARED, NOT IMPLIED. ADR-0064's Known limitations: a withdrawal mis-stamped with
    // another chart's patient_id and left unverified finds nothing in
    // cairn_sensitivity_standing on any read, ever, so it falls out of the worklist's own
    // inert arm. Printed even when the list is EMPTY — that is the case where silence is
    // most convincing and most wrong.
    out.push(
        "(this list is not complete: a withdrawal mis-stamped with another chart's \
         patient_id and left unverified is permanently inert AND invisible here — \
         ADR-0064, Known limitations)"
            .to_string(),
    );
    // Same posture main.rs already takes for an empty safety_class_map: an empty result
    // must never read as "checked, nothing found" (principle 4).
    out.push(
        "(an empty overclaim list is NOT a clean bill: the ledger's completeness rests on \
         a RAISE WARNING nothing consumes — #414)"
            .to_string(),
    );
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
    if !r.threads.is_empty() {
        return r.threads.iter().map(render_thread_line).collect();
    }
    // NOTHING PROJECTED. Two very different states, and the old wording collapsed them
    // into one precise untruth: "no medication threads on this chart" (#383).
    if r.standing.is_empty() {
        return vec![
            "  no medication threads and no standing sensitivity assertions on this chart"
                .to_string(),
        ];
    }
    // NAMED, NEVER COUNTED. A bare count cannot separate "this node is custody-blind" from
    // "the chart is genuinely empty", which is the one question this branch exists to
    // answer — ADR-0061 settled the same shape for the registration funnel. Each row also
    // carries the content_address `sensitivity-withdraw --withdraws` consumes.
    let mut out = vec![format!(
        "⚠ this node projects no medication threads, but {} sensitivity assertion(s) stand \
         on this chart:",
        r.standing.len()
    )];
    for s in &r.standing {
        out.push(format!(
            "    {} ({}, subject {})  withdraws={}",
            s.grade,
            super::subject_kind_phrase(&s.subject_kind),
            s.subject_id,
            s.content_address
        ));
    }
    out.push(
        "  → this node may hold no DEK custody, so the threads these assertions grade may \
         exist and be invisible here (#383)"
            .to_string(),
    );
    out
}

/// One projected thread's line. Extracted so both branches of `render_threads` read the
/// same way and neither can drift from the other's wording.
fn render_thread_line(t: &super::ThreadGrade) -> String {
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sensitivity::report::{StandingAssertion, ThreadGrade};

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
            ineffective_withdrawals: vec![],
            standing: vec![],
            deferred: vec![],
            overclaims: vec![],
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

    fn inert_withdrawal() -> IneffectiveWithdrawal {
        IneffectiveWithdrawal {
            withdraws: "a3f".into(),
            reason: "inert".into(),
            node_origin: "peer-b".into(),
            rationale: "consent withdrawn by patient 2026-08-12".into(),
            responsible_actor_id: Some("beef".into()),
        }
    }

    #[test]
    fn an_inert_withdrawal_names_its_reason_rationale_and_actor() {
        // THE §1.2 BUDGET, as a unit test: "why did this withdrawal not take effect?"
        // answered without raw SQL. Everything the operator needs must be in these lines.
        let mut r = healthy();
        r.ineffective_withdrawals = vec![inert_withdrawal()];
        let text = render_chart_report("C", &r).join("\n");
        assert!(text.contains("did NOT take effect"), "{text}");
        assert!(text.contains("inert"), "{text}");
        assert!(text.contains("consent withdrawn by patient"), "the rationale: {text}");
        assert!(text.contains("beef"), "the accountable actor (#421): {text}");
        assert!(text.contains("withdraws=a3f"), "the target address: {text}");
    }

    #[test]
    fn the_two_reasons_read_differently() {
        // 'inert' and 'stranger-attested' have DIFFERENT fixes — one needs an accountable
        // human, the other needs a look at who is asserting on this chart. A shared
        // sentence would hide that, which is the whole failure this surface exists to end.
        let mut a = healthy();
        a.ineffective_withdrawals = vec![inert_withdrawal()];
        let mut b = healthy();
        b.ineffective_withdrawals = vec![IneffectiveWithdrawal {
            reason: "stranger-attested".into(),
            ..inert_withdrawal()
        }];
        assert_ne!(
            render_chart_report("C", &a).join("\n"),
            render_chart_report("C", &b).join("\n")
        );
    }

    #[test]
    fn an_unrecognised_reason_is_shown_not_swallowed() {
        // Open vocabulary: a future db/048 may add a reason this build has never seen.
        // Mirrors subject_kind_phrase's total mapping — the catch-all must point the
        // reader AT the row, never silently render it as if it were understood.
        let phrase = withdrawal_reason_explanation("some-future-reason");
        assert!(
            phrase.contains("unrecognised"),
            "an unknown reason must say so: {phrase}"
        );
    }

    #[test]
    fn the_footer_declares_the_invisible_withdrawal_even_with_none_listed() {
        // ADR-0064 Known limitations: a cross-chart mis-targeted withdrawal that stays
        // unverified is permanently inert AND permanently invisible — it falls out of the
        // worklist's inert arm. A surface listing "the withdrawals that did not take
        // effect" while silent about that is a comment asserting a guarantee the code does
        // not provide, which is the defect class this whole slice is about. Asserted on a
        // report with an EMPTY list, because that is the case where silence is most
        // convincing and most wrong.
        let text = render_chart_report("C", &healthy()).join("\n");
        assert!(text.contains("not complete"), "{text}");
        assert!(text.contains("ADR-0064"), "{text}");
    }

    #[test]
    fn an_empty_chart_says_both_things_are_empty() {
        let mut r = healthy();
        r.threads = vec![];
        let text = render_chart_report("C", &r).join("\n");
        assert!(text.contains("no medication threads and no standing"), "{text}");
    }

    #[test]
    fn a_custody_blind_chart_names_each_standing_assertion_and_never_merely_counts() {
        // #383 / #388 part 3. Both issues proposed a COUNT. This diverges from both:
        // ADR-0061 settled the shape — "2 standing assertions, 0 threads" cannot tell an
        // operator whether this node is custody-blind or the chart is genuinely empty,
        // which is the one question the line exists to answer. A named row also carries the
        // content_address that `sensitivity-withdraw --withdraws` consumes.
        let mut r = healthy();
        r.threads = vec![];
        r.standing = vec![StandingAssertion {
            content_address: "c0ffee".into(),
            subject_kind: "thread".into(),
            subject_id: uuid::Uuid::nil(),
            grade: "restricted".into(),
        }];
        let text = render_chart_report("C", &r).join("\n");
        assert!(text.contains("c0ffee"), "the address must be named: {text}");
        assert!(text.contains("restricted"), "the grade must be named: {text}");
        assert!(text.contains("no DEK custody"), "the custody explanation: {text}");
        assert!(
            !text.contains("no medication threads on this chart"),
            "the old precise untruth must be gone: {text}"
        );
    }

    #[test]
    fn a_deferred_sensitivity_event_is_reported_as_powerless() {
        // db/043 records adjudication_error and leaves the event deferred. A sensitivity
        // assertion admitted by a pre-db/048 node (ADR-0056 admit-and-defer — a DESIGNED
        // state, given "no lockstep fleet upgrade") projects nothing and therefore reads
        // 'routine'. Nothing in the §5.9 read path consulted event_deferred, so a grade
        // this node is FAILING TO APPLY was invisible.
        let mut r = healthy();
        r.deferred = vec![DeferredSensitivityEvent {
            event_id: uuid::Uuid::nil(),
            event_type: "sensitivity.grade.asserted".into(),
            admitted_at: "2026-08-18 09:00:00+00".into(),
            adjudication_error: None,
        }];
        let text = render_chart_report("C", &r).join("\n");
        assert!(text.contains("DEFERRED"), "{text}");
        assert!(text.contains("powerless"), "{text}");
        assert!(
            text.contains("not yet re-adjudicated"),
            "the null-error wording: {text}"
        );
    }

    #[test]
    fn an_overclaim_names_both_rungs() {
        let mut r = healthy();
        r.overclaims = vec![SafetyOverclaim {
            content_address: "dead".into(),
            emitted_rung: "precise".into(),
            licensed_rung: "existence".into(),
        }];
        let text = render_chart_report("C", &r).join("\n");
        assert!(text.contains("overclaim"), "{text}");
        assert!(text.contains("precise"), "{text}");
        assert!(text.contains("existence"), "{text}");
        assert!(text.contains("dead"), "the event must be nameable: {text}");
    }

    #[test]
    fn an_empty_overclaim_ledger_is_never_a_clean_bill() {
        // #414: the ledger's completeness rests on a RAISE WARNING nothing consumes, so an
        // empty ledger is indistinguishable from a broken one. Same shape as
        // safety_class_map shipping empty, where main.rs already refuses to say "no safety
        // signals" — an empty result must never read as "checked, nothing found"
        // (principle 4: an imprecise near-truth beats a precise untruth).
        let text = render_chart_report("C", &healthy()).join("\n");
        assert!(text.contains("#414"), "the disclaimer must cite its issue: {text}");
        assert!(
            !text.contains("no overclaims"),
            "an empty ledger must not read as a clean bill: {text}"
        );
    }

    #[test]
    fn the_read_back_reports_the_asserted_and_the_standing_grade_as_two_facts() {
        // A thread-scoped 'restricted' asserted while a chart-wide 'sequestered' stands
        // reads back as 'sequestered' — correct, and indistinguishable from "your assertion
        // was silently upgraded" if only one grade is printed. Both, always, with the
        // winning subject, so the operator can see WHY they differ.
        let line = render_assert_readback("restricted", "sequestered", "chart-wide");
        assert!(line.contains("restricted"), "{line}");
        assert!(line.contains("sequestered"), "{line}");
        assert!(line.contains("chart-wide"), "{line}");
    }

    #[test]
    fn the_read_back_is_still_two_facts_when_they_agree() {
        // No special case for agreement: a reader who learns the surface prints one grade
        // when they agree cannot then trust a single-grade line to mean agreement.
        let line = render_assert_readback("restricted", "restricted", "this thread");
        assert!(line.matches("restricted").count() >= 2, "{line}");
    }
}
