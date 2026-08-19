//! §5.9 — how a chart report reads.
//!
//! PURE. No database, no I/O, no `tokio_postgres` import. Every honesty claim this surface
//! makes is a sentence — "this is not a clean bill of health", "this node may hold no
//! custody", "this list is not complete" — and a sentence that only exists inside a
//! `println!` in `main.rs` can be tested only by running the binary against a live cluster,
//! which is why nobody ever did. Keeping the wording here makes each claim a unit test.
//!
//! Precedent: `crate::safety::render_safety_line`, which is pure for the same reason.
use super::readback::{SubjectResolution, TargetState, WithdrawOutcome};
use super::report::{
    ChartReport, DeferredSensitivityEvent, SafetyOverclaim, WithdrawalWorklistRow,
};
use super::winner::WinningSubject;

/// The prefix `cairn_patient_deferred_sensitivity` filters on. Named once, here, so the
/// footer that declares the block's limit cannot drift from the SQL that creates it.
const DEFERRED_PREFIX: &str = "sensitivity.%";

/// Peer-supplied text, made safe to put in a line-oriented report.
///
/// NOT cosmetic. `node_origin`, `event_type` and `grade` are unconstrained `TEXT` copied
/// VERBATIM from a peer's own self-asserted body — db/048 says of the grade vocabulary "a
/// future grade from an upgraded peer is ADMITTED verbatim", which is correct for the wire
/// (principle 11) and dangerous for a renderer. A newline in any of them lets a hostile
/// peer forge an entire line: an operator reading a fabricated `chart <id>: sequestered`
/// among the real ones would believe a chart is protected when it is not.
///
/// `{:?}` (Debug for `str`) escapes control characters and quotes while leaving printable
/// Unicode alone, so an ordinary value still reads naturally. It is the idiom `rationale`
/// already used — this extends it to every other field with the same provenance.
fn peer(s: &str) -> String {
    format!("{s:?}")
}

/// The `(winning subject: …)` clause both grade lines end with.
///
/// ONE function, because the chart line and the thread line must read identically — a
/// reader who learns the shape on one must be able to trust it on the other. Before
/// [`WinningSubject`] existed this was the same six-line `match` written twice, over a pair
/// of fields whose correlation was held only by a doc comment.
///
/// The address is printed UNQUOTED: `withdraws=<hex>` is a copy-paste CONTRACT into
/// `sensitivity-withdraw --withdraws`, and hex from `encode(…, 'hex')` carries no
/// line-forging risk to escape (unlike the peer-authored grade beside it).
///
/// That "it is hex" is not a hope: [`WinningSubject`]'s payload fields are private, so the
/// only producers are its two constructors, and every caller of those feeds them
/// `encode(…, 'hex')`. A new producer would have to be added inside `winner.rs`, next to
/// this reasoning. Likewise the phrase is `&'static str`, so neither half of this line can
/// become peer text without a type change.
fn winner_clause(w: &WinningSubject) -> String {
    match w.content_address() {
        Some(ca) => format!("(winning subject: {}, withdraws={ca})", w.phrase()),
        None => format!("(winning subject: {})", w.phrase()),
    }
}

/// Why a worklist row is on the worklist, in words. Pure and TOTAL — every input has an
/// output, including one this build has never seen.
///
/// The two reasons have DIFFERENT fixes, which is why they get different sentences:
/// `inert` means nobody this node can hold responsible stands behind the claim (the fix is
/// an accountable human re-asserting it), while `stranger-attested` means someone did stand
/// behind it, the withdrawal TOOK EFFECT, and the fix is a look at who is asserting here.
///
/// The catch-all points the reader AT the row rather than rendering an unknown reason as
/// though it were understood — the same discipline as `super::subject_kind_phrase`.
pub fn withdrawal_reason_explanation(reason: &str) -> &'static str {
    match reason {
        "inert" => {
            "no accountable human this node can hold responsible stands behind it (ADR-0064)"
        }
        // The caveat is part of the sentence, not a footnote: #415 is open and says this
        // measures the SIGNER's actor, not the accountable human's authorship. Every
        // clinical verb except attestation is node-signed, so a clinician who has worked
        // this chart all week still carries the node's actor_id on all of it and is
        // labelled a stranger. Telling an operator "no prior presence" as bare fact would
        // point them at the wrong person.
        "stranger-attested" => {
            "attested, and it TOOK EFFECT — but by an actor with no prior presence on this \
             chart. NOTE: this measures the signing key's actor, not authorship, so a \
             clinician who worked this chart through node-signed verbs can be mislabelled \
             (#415)"
        }
        _ => "an unrecognised reason from a newer schema on this node — read the row itself",
    }
}

/// The header for one reason-group of the withdrawal worklist.
///
/// ONE HEADER PER REASON, because the two arms have OPPOSITE effects and a shared header
/// was factually false for one of them. `sensitivity_withdrawal_worklist` is a union of two
/// disjoint arms (db/048 section 11): `verdict = 'unverified'` did not take effect, and
/// `verdict <> 'unverified'` DID — db/048: "as SALIENCE it blocks nothing and delays
/// nothing — the withdrawal has already taken effect."
///
/// An earlier draft counted both under "did NOT take effect". That was the single worst
/// line on this surface: the stranger-attested row is a COMPLETED, unaccountable removal of
/// protection, and the report told the operator it had not happened — while the grade line
/// directly above already showed the lowered grade.
pub fn withdrawal_group_header(reason: &str, n: usize) -> String {
    match reason {
        "inert" => format!(
            "⚠ {n} withdrawal(s) on this chart did NOT take effect — the grade above may \
             not be what someone intended"
        ),
        "stranger-attested" => format!(
            "⚠ {n} withdrawal(s) on this chart TOOK EFFECT with no accountable prior \
             presence here — protection was removed and the grade above already reflects it"
        ),
        _ => format!(
            "⚠ {n} withdrawal(s) on this chart are flagged for a reason this build does not \
             recognise — read the rows themselves"
        ),
    }
}

/// The withdrawal worklist, grouped by reason so each group can state its own effect.
///
/// Grouping is by FIRST APPEARANCE rather than by a hardcoded arm list: `reason` is an open
/// vocabulary (a future db/048 may add a third), and a build that has never seen a value
/// must still group and count it correctly rather than dropping it.
fn render_withdrawals(ws: &[WithdrawalWorklistRow]) -> Vec<String> {
    let mut reasons: Vec<&str> = Vec::new();
    for w in ws {
        if !reasons.contains(&w.reason.as_str()) {
            reasons.push(&w.reason);
        }
    }
    let mut out = Vec::new();
    for reason in reasons {
        let group: Vec<&WithdrawalWorklistRow> = ws.iter().filter(|w| w.reason == reason).collect();
        out.push(withdrawal_group_header(reason, group.len()));
        for w in group {
            out.push(format!(
                "    {:<18} withdraws={}  by actor={}  origin={}",
                w.reason,
                w.withdraws,
                w.responsible_actor_id
                    .as_deref()
                    .unwrap_or("(none this node can name)"),
                peer(&w.node_origin)
            ));
            out.push(format!("      rationale: {}", peer(&w.rationale)));
            out.push(format!(
                "      → {}",
                withdrawal_reason_explanation(&w.reason)
            ));
        }
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
            peer(&d.event_type),
            d.admitted_at,
            d.adjudication_error
                .as_deref()
                .map(peer)
                .unwrap_or_else(|| "(not yet re-adjudicated)".to_string())
        ));
    }
    out
}

/// The warning block for recorded safety overclaims.
///
/// The rungs are printed in a fixed `emitted=… licensed=…` order because the DIRECTION is
/// the whole meaning: emitted finer than licensed is over-disclosure. Reading them the
/// other way round would turn a disclosure incident into an over-cautious one.
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
            o.content_address,
            peer(&o.emitted_rung),
            peer(&o.licensed_rung)
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
///
/// `scope` names WHAT the standing grade was read over, because the caller resolves a
/// chart-wide assert and a thread-scoped assert against different subjects. Without it, a
/// thread-scoped `restricted` on a routine chart read back as "routine now stands", which
/// looks exactly like "your assertion did nothing" for an assertion that fully took effect.
///
/// `winning_subject` is `&'static str` because it is the one value here printed
/// UNESCAPED — `asserted` and `standing` both go through `peer`. Peer text arrives as
/// `String`, so the narrower type is what makes routing it into that slot impossible
/// rather than merely unlikely (#387; the same reasoning as `SubjectReading`'s field and
/// `WinningSubject::phrase`).
pub fn render_assert_readback(
    asserted: &str,
    standing: &str,
    winning_subject: &'static str,
    scope: &str,
) -> String {
    format!(
        "asserted {}; {} now stands on {} (winning subject: {})",
        peer(asserted),
        peer(standing),
        scope,
        winning_subject
    )
}

/// What an operator sees after withdrawing a grade: whether protection actually came off,
/// and — when it did not — why (#435).
///
/// THE TWO FACTS ARE PRINTED SEPARATELY AND NEVER MERGED. `sensitivity-withdraw` used to
/// print "withdrew ..." unconditionally, which reads as "protection removed" whatever
/// happened, and a withdrawal that lands inert is the headline subject of the whole §5.9
/// slice. But the obvious repair — one line saying whether it "took effect" — walks
/// straight into the defect the Slice 69 review found on the chart report: db/048's
/// worklist is a union of two arms with OPPOSITE meanings, and `stranger-attested` is a
/// COMPLETED removal of protection, not a failure.
///
/// So the effect is stated from the TARGET's standing (a set-membership test that cannot
/// be misread), and the worklist arm is reported beside it as its own, separately-worded
/// fact. A row on the worklist never changes the effect sentence — it adds one.
///
/// Returns lines rather than printing them, for the same reason as `render_chart_report`:
/// the caller owns the I/O and every honesty claim here stays a unit test.
pub fn render_withdraw_readback(o: &WithdrawOutcome) -> Vec<String> {
    let mut out = Vec::new();
    match &o.target {
        // Nothing about the target is knowable here, so no effect is claimed in EITHER
        // direction. This is a routine federated state, not an error: db/048 keeps no
        // foreign key from a withdrawal to its target precisely because set-union sync has
        // no ordering. Note this is NOT "it will take effect on arrival" — whether it
        // does depends on the accountability fact below, which may itself be `inert`.
        TargetState::NotHeldHere => out.push(
            "  ⚠ the assertion this withdrawal targets has NOT replicated to this node, so \
             what it graded — and whether this withdrawal moves it — cannot be determined \
             here"
                .to_string(),
        ),
        // ADR-0064's KNOWN GAP. The target is alive on its own chart, so this is neither
        // "removed" nor "still standing here" — it is an act that can never take effect
        // anywhere, and no arm of db/048's worklist names the condition.
        TargetState::OnAnotherChart => out.push(
            "  ⚠ the assertion this withdrawal names is recorded on a different chart, so \
             this withdrawal is permanently INERT: it can never remove that protection, \
             and no worklist arm reports the reason (ADR-0064, Known limitations). Check \
             --patient and the address, then re-issue"
                .to_string(),
        ),
        TargetState::Held {
            still_standing,
            subject,
        } => {
            if *still_standing {
                out.push(
                    "  ⚠ INERT — the withdrawn assertion STILL STANDS on this node: the \
                     claim landed and converges, but it removed no protection"
                        .to_string(),
                );
            } else {
                out.push(
                    "  ✓ the withdrawn assertion NO LONGER STANDS — protection was removed"
                        .to_string(),
                );
            }
            // What now stands over the target's OWN subject, never the chart-wide grade.
            // Resolving against the wrong subject is the bug the assert read-back had to
            // fix mid-review; it is also what stops "no longer stands" from reading as
            // "this subject is now open" when a SECOND assertion still grades it.
            match subject {
                SubjectResolution::Resolved(r) => out.push(format!(
                    "    {} {} stands on {} (winning subject: {})",
                    peer(&r.grade),
                    if *still_standing { "still" } else { "now" },
                    r.scope,
                    r.winning_subject
                )),
                SubjectResolution::Unrecognised(kind) => out.push(format!(
                    "    (its subject kind {} is one this build does not recognise, so what \
                     now stands over it cannot be resolved here)",
                    peer(kind)
                )),
            }
        }
    }
    // The accountability fact, ALWAYS its own line. `withdrawal_reason_explanation` is the
    // one place each arm's meaning is worded, so this verb and the chart report can never
    // describe the same arm differently.
    if let Some(reason) = &o.worklist_reason {
        out.push(format!(
            "  ⚠ on the withdrawal worklist as {}: {}",
            peer(reason),
            withdrawal_reason_explanation(reason)
        ));
    }
    out
}

/// Render one chart's §5.9 report as the lines an operator reads, in order.
///
/// The chart grade comes FIRST and keeps its exact wire shape — see the contract test. The
/// warning blocks sit between the grade and the per-thread breakdown, deliberately: a
/// warning that appears forty thread-lines below the claim it qualifies is a warning nobody
/// reads.
///
/// Returns lines rather than printing them so the caller owns the I/O and the wording stays
/// testable. The chart label is a PARAMETER rather than something the caller splices in
/// afterwards: an earlier draft had `main.rs` rewrite the leading `chart:` token, which also
/// rewrites the custody-blind line ending `...stand on this chart:` and mangles it. Passing
/// the label costs one argument and cannot mis-target.
pub fn render_chart_report(chart: &str, r: &ChartReport) -> Vec<String> {
    let mut out = Vec::new();
    out.push(format!(
        "chart {}: {} {}",
        chart,
        peer(&r.chart_grade),
        winner_clause(&r.chart_subject)
    ));
    out.extend(render_withdrawals(&r.withdrawals_needing_review));
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
    // The deferred block's own limit, declared for the same reason and printed on the same
    // terms. An event is deferred BECAUSE this node does not recognise its type, so
    // filtering that set by a prefix this build already knows cannot be complete — and the
    // safe default db/048 states for its own unknown types ("unknown must coarsen, never
    // expose... the safe default requires no one to remember to add it") runs the other way.
    // Widening the filter is #434; until then the limit is stated rather than implied.
    out.push(format!(
        "(the DEFERRED list above covers only event types matching '{DEFERRED_PREFIX}': a \
         confidentiality event this node cannot classify under another name is admitted, \
         unapplied and invisible here — run `cairn-node deferred` for the unfiltered set, \
         #434)"
    ));
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

/// The per-thread breakdown, plus the honest statement of what it leaves out.
///
/// The custody warning is driven by `sealed_medication_events_without_custody` — a MEASURED
/// count — and is emitted whatever the length of `threads`. The partial-custody case (some
/// DEKs held, not all) renders a plausible, silently truncated list and is the one this
/// block most needs to catch; no proxy over `threads`/`standing` can see it at all.
fn render_threads(r: &ChartReport) -> Vec<String> {
    let mut out = Vec::new();
    let unopenable = r.sealed_medication_events_without_custody;

    if !r.threads.is_empty() {
        out.extend(r.threads.iter().map(render_thread_line));
    } else if !r.standing.is_empty() {
        // NAMED, NEVER COUNTED. A bare count cannot separate "this node is custody-blind"
        // from "the chart is genuinely empty", which is the one question this branch exists
        // to answer — ADR-0061 settled the same shape for the registration funnel. Each row
        // also carries the content_address `sensitivity-withdraw --withdraws` consumes.
        out.push(format!(
            "⚠ this node projects no medication threads, but {} sensitivity assertion(s) \
             stand on this chart:",
            r.standing.len()
        ));
        for s in &r.standing {
            out.push(format!(
                "    {} ({}, subject {})  withdraws={}",
                peer(&s.grade),
                super::subject_kind_phrase(&s.subject_kind),
                s.subject_id,
                s.content_address
            ));
        }
    } else if unopenable == 0 {
        // The ONLY state in which absence can honestly be asserted: nothing projected,
        // nothing standing, and nothing sealed that this node cannot open.
        out.push(
            "  no medication threads and no standing sensitivity assertions on this chart"
                .to_string(),
        );
    } else {
        // Say what this NODE sees, never what the chart contains — the custody line below
        // supplies the reason.
        out.push("  this node projects no medication threads on this chart".to_string());
    }

    if unopenable > 0 {
        out.push(format!(
            "⚠ this node holds {unopenable} sealed medication event(s) on this chart it \
             cannot open (no DEK custody) — the thread list above is INCOMPLETE, whatever \
             its length (#383)"
        ));
    }
    out
}

/// One projected thread's line. Extracted so both branches of `render_threads` read the
/// same way and neither can drift from the other's wording.
fn render_thread_line(t: &super::ThreadGrade) -> String {
    format!(
        "  thread {}: {} {}",
        t.thread_id,
        peer(&t.grade),
        winner_clause(&t.subject)
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
            chart_subject: WinningSubject::None,
            threads: vec![ThreadGrade {
                thread_id: uuid::Uuid::nil(),
                grade: "routine".into(),
                subject: WinningSubject::None,
            }],
            withdrawals_needing_review: vec![],
            standing: vec![],
            deferred: vec![],
            overclaims: vec![],
            sealed_medication_events_without_custody: 0,
        }
    }

    #[test]
    fn the_thread_line_keeps_the_same_shape_as_the_chart_line() {
        // THE THREAD LINE WAS RENDERED BY NO TEST AT ALL until this one. Every fixture used
        // `WinningSubject::None`, so `(winning subject: …, withdraws=<hex>)` could be
        // deleted from `render_thread_line` outright — or lose its leading space — with the
        // whole suite green. That hex is the operator's ONLY route to withdrawing a
        // thread-scoped grade without raw SQL, i.e. exactly the contract
        // `the_grade_line_keeps_its_documented_shape` was written to protect one function
        // away on the chart line.
        let mut r = healthy();
        r.threads[0].grade = "restricted".into();
        r.threads[0].subject = WinningSubject::from_row("thread", Some("a3f".into()));
        let line = render_thread_line(&r.threads[0]);
        assert_eq!(
            line,
            format!(
                "  thread {}: \"restricted\" (winning subject: this thread, withdraws=a3f)",
                uuid::Uuid::nil()
            ),
            "byte-for-byte, including the two-space indent and the space before `(`"
        );
    }

    #[test]
    fn a_thread_with_no_winner_offers_no_withdraws_clause() {
        // The other arm, equally unpinned before: `healthy()`'s thread has no winner, so
        // the line must carry the phrase and NO `withdraws=`, or an operator would be
        // handed an address for an assertion that does not exist.
        let r = healthy();
        let line = render_thread_line(&r.threads[0]);
        assert_eq!(
            line,
            format!(
                "  thread {}: \"routine\" (winning subject: none)",
                uuid::Uuid::nil()
            )
        );
        assert!(!line.contains("withdraws="));
    }

    #[test]
    fn the_grade_line_keeps_its_documented_shape() {
        // `sensitivity-withdraw --withdraws` documents its argument as "the hex
        // content_address, as patient-sensitivity prints it". That is a CONTRACT: an
        // earlier draft of ChartReport dropped the address entirely and a hand exercise of
        // the CLI caught it. Pin the shape so the next refactor cannot quietly break it.
        let mut r = healthy();
        r.chart_grade = "sequestered".into();
        r.chart_subject = WinningSubject::from_row("patient", Some("a3f".into()));
        let lines = render_chart_report("C", &r);
        // The grade is Debug-quoted: it is unconstrained peer text (see `peer`). The part
        // that is a copy-paste CONTRACT — `withdraws=<hex>` — is unquoted and unchanged.
        assert_eq!(
            lines[0],
            "chart C: \"sequestered\" (winning subject: chart-wide, withdraws=a3f)"
        );
    }

    #[test]
    fn a_chart_with_no_assertion_names_no_address() {
        let lines = render_chart_report("C", &healthy());
        assert_eq!(lines[0], "chart C: \"routine\" (winning subject: none)");
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

    fn inert_withdrawal() -> WithdrawalWorklistRow {
        WithdrawalWorklistRow {
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
        r.withdrawals_needing_review = vec![inert_withdrawal()];
        let text = render_chart_report("C", &r).join("\n");
        assert!(text.contains("did NOT take effect"), "{text}");
        assert!(text.contains("inert"), "{text}");
        assert!(
            text.contains("consent withdrawn by patient"),
            "the rationale: {text}"
        );
        assert!(
            text.contains("beef"),
            "the accountable actor (#421): {text}"
        );
        assert!(text.contains("withdraws=a3f"), "the target address: {text}");
    }

    #[test]
    fn the_two_reasons_read_differently() {
        // 'inert' and 'stranger-attested' have DIFFERENT fixes — one needs an accountable
        // human, the other needs a look at who is asserting on this chart. A shared
        // sentence would hide that, which is the whole failure this surface exists to end.
        let mut a = healthy();
        a.withdrawals_needing_review = vec![inert_withdrawal()];
        let mut b = healthy();
        b.withdrawals_needing_review = vec![WithdrawalWorklistRow {
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
        assert!(
            text.contains("no medication threads and no standing"),
            "{text}"
        );
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
        // Genuinely custody-blind now MEANS something measured: three sealed medication
        // events this node cannot open. Under the old proxy this fixture asserted custody
        // blindness purely from "standing is non-empty", which was the bug.
        r.sealed_medication_events_without_custody = 3;
        r.standing = vec![StandingAssertion {
            content_address: "c0ffee".into(),
            subject_kind: "thread".into(),
            subject_id: uuid::Uuid::nil(),
            grade: "restricted".into(),
        }];
        let text = render_chart_report("C", &r).join("\n");
        assert!(text.contains("c0ffee"), "the address must be named: {text}");
        assert!(
            text.contains("restricted"),
            "the grade must be named: {text}"
        );
        assert!(
            text.contains("no DEK custody"),
            "the custody explanation: {text}"
        );
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
        assert!(
            text.contains("#414"),
            "the disclaimer must cite its issue: {text}"
        );
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
        let line = render_assert_readback("restricted", "sequestered", "chart-wide", "this chart");
        assert!(line.contains("restricted"), "{line}");
        assert!(line.contains("sequestered"), "{line}");
        assert!(line.contains("chart-wide"), "{line}");
    }

    #[test]
    fn the_read_back_is_still_two_facts_when_they_agree() {
        // No special case for agreement: a reader who learns the surface prints one grade
        // when they agree cannot then trust a single-grade line to mean agreement.
        let line = render_assert_readback("restricted", "restricted", "this thread", "that thread");
        assert!(line.matches("restricted").count() >= 2, "{line}");
    }
}

#[cfg(test)]
mod review_fixes {
    use super::*;
    use crate::sensitivity::report::{StandingAssertion, ThreadGrade};

    fn base() -> ChartReport {
        ChartReport {
            chart_grade: "routine".into(),
            chart_subject: WinningSubject::None,
            threads: vec![ThreadGrade {
                thread_id: uuid::Uuid::nil(),
                grade: "routine".into(),
                subject: WinningSubject::None,
            }],
            withdrawals_needing_review: vec![],
            standing: vec![],
            deferred: vec![],
            overclaims: vec![],
            sealed_medication_events_without_custody: 0,
        }
    }

    fn row(reason: &str) -> WithdrawalWorklistRow {
        WithdrawalWorklistRow {
            withdraws: "a3f".into(),
            reason: reason.into(),
            node_origin: "peer-b".into(),
            rationale: "consent withdrawn".into(),
            responsible_actor_id: Some("beef".into()),
        }
    }

    #[test]
    fn a_stranger_attested_withdrawal_is_never_reported_as_ineffective() {
        // db/048: "as SALIENCE it blocks nothing and delays nothing — the withdrawal has
        // ALREADY TAKEN EFFECT." Protection was removed. Telling the operator it did not
        // take effect is a precise untruth about the one row representing a completed,
        // unaccountable protection strip.
        let mut r = base();
        r.withdrawals_needing_review = vec![row("stranger-attested")];
        let text = render_chart_report("C", &r).join("\n");
        assert!(
            !text.contains("did NOT take effect"),
            "a stranger-attested withdrawal DID take effect: {text}"
        );
        assert!(text.contains("TOOK EFFECT"), "{text}");
    }

    #[test]
    fn the_two_reasons_get_opposite_effect_claims_not_merely_different_ones() {
        // Asserting the whole reports differ is too weak: `reason` is echoed verbatim in
        // every row line, so two reports differ even if both headers claim the same thing.
        // Assert on the pure header function instead.
        let inert = withdrawal_group_header("inert", 1);
        let stranger = withdrawal_group_header("stranger-attested", 1);
        assert!(inert.contains("did NOT take effect"), "{inert}");
        assert!(!stranger.contains("did NOT take effect"), "{stranger}");
    }

    #[test]
    fn both_reason_groups_render_with_their_own_header_and_every_row() {
        let mut r = base();
        r.withdrawals_needing_review = vec![row("inert"), row("inert"), row("stranger-attested")];
        let text = render_chart_report("C", &r).join("\n");
        assert!(text.contains("2 withdrawal(s)"), "the inert count: {text}");
        assert!(
            text.contains("1 withdrawal(s)"),
            "the stranger count: {text}"
        );
        // No row may be silently dropped: three rows, three rationale lines.
        assert_eq!(text.matches("rationale:").count(), 3, "{text}");
    }

    #[test]
    fn peer_supplied_text_cannot_forge_a_line() {
        // node_origin/event_type/grade are unconstrained TEXT copied VERBATIM from a peer's
        // self-asserted body (db/001, db/048 line 26 "ADMITTED verbatim"). A newline in any
        // of them would let a hostile peer fabricate a grade line in a line-oriented report.
        let mut r = base();
        r.withdrawals_needing_review = vec![WithdrawalWorklistRow {
            node_origin: "peer-b\nchart C: sequestered (winning subject: chart-wide)".into(),
            ..row("inert")
        }];
        for line in render_chart_report("C", &r) {
            assert!(
                !line.trim_start().starts_with("chart C: sequestered"),
                "peer text forged a line: {line}"
            );
        }
    }

    #[test]
    fn a_hostile_grade_cannot_forge_a_line() {
        let mut r = base();
        r.chart_grade = "routine\n⚠ 0 withdrawal(s) on this chart did NOT take effect".into();
        let lines = render_chart_report("C", &r);
        assert_eq!(
            lines[0].lines().count(),
            1,
            "grade broke the line: {lines:?}"
        );
    }

    #[test]
    fn the_deferred_block_declares_the_prefix_it_is_scoped_to() {
        // An event is deferred BECAUSE its type is unrecognised, so a prefix filter cannot
        // be complete. Printed even when the list is empty — the arm most exposed to a
        // future confidentiality type must not render as a reassuring blank.
        let text = render_chart_report("C", &base()).join("\n");
        assert!(text.contains("sensitivity.%"), "{text}");
    }

    #[test]
    fn a_custody_blind_chart_with_no_standing_assertion_still_refuses_to_claim_absence() {
        // #383's precise untruth survived here: grading is opt-in, so MOST custody-blind
        // charts carry no standing assertion at all and took this branch.
        let mut r = base();
        r.threads = vec![];
        r.sealed_medication_events_without_custody = 5;
        let text = render_chart_report("C", &r).join("\n");
        assert!(
            !text.contains("no medication threads and no standing"),
            "must not assert absence it cannot know: {text}"
        );
        assert!(text.contains("no DEK custody"), "{text}");
    }

    #[test]
    fn partial_custody_warns_even_though_some_threads_projected() {
        // The worst case: a node holding 3 of 8 DEKs renders a plausible, silently
        // truncated list. The old code early-returned on a non-empty threads vec.
        let mut r = base();
        r.sealed_medication_events_without_custody = 5;
        let text = render_chart_report("C", &r).join("\n");
        assert!(text.contains('⚠'), "partial custody must warn: {text}");
        assert!(text.contains("no DEK custody"), "{text}");
    }

    #[test]
    fn a_genuinely_empty_chart_with_full_custody_still_says_so_and_raises_no_warning() {
        let mut r = base();
        r.threads = vec![];
        let text = render_chart_report("C", &r).join("\n");
        assert!(text.contains("no medication threads"), "{text}");
        assert!(!text.contains('⚠'), "nothing is wrong here: {text}");
    }

    #[test]
    fn the_stranger_explanation_declares_the_415_signer_caveat() {
        let e = withdrawal_reason_explanation("stranger-attested");
        assert!(e.contains("#415"), "the signer-vs-author caveat: {e}");
    }

    #[test]
    fn a_standing_assertion_line_survives_a_hostile_grade() {
        let mut r = base();
        r.threads = vec![];
        r.standing = vec![StandingAssertion {
            content_address: "c0ffee".into(),
            subject_kind: "thread".into(),
            subject_id: uuid::Uuid::nil(),
            grade: "restricted\nchart C: routine".into(),
        }];
        for line in render_chart_report("C", &r) {
            assert!(!line.trim_start().starts_with("chart C: routine"), "{line}");
        }
    }
}

#[cfg(test)]
mod withdraw_readback {
    use super::*;
    use crate::sensitivity::readback::SubjectReading;

    // #435 — the withdraw read-back.
    //
    // A withdrawal that lands INERT is the headline subject of the whole §5.9 slice, and
    // the verb that authors one printed "withdrew ..." unconditionally: an operator had no
    // way to learn from that command that nothing had moved. These tests hold the four
    // operator stories apart, because the one thing this surface must never do is give two
    // opposite outcomes the same sentence — the Slice 69 review's lesson 5, one verb over.

    /// What now stands over the withdrawn assertion's OWN subject.
    fn reading(grade: &str) -> SubjectReading {
        SubjectReading {
            grade: grade.into(),
            winning_subject: "this thread",
            scope: "that thread",
        }
    }

    fn held(still_standing: bool, grade: &str) -> TargetState {
        TargetState::Held {
            still_standing,
            subject: SubjectResolution::Resolved(reading(grade)),
        }
    }

    #[test]
    fn a_clean_withdrawal_says_the_assertion_no_longer_stands() {
        let o = WithdrawOutcome {
            worklist_reason: None,
            target: held(false, "routine"),
        };
        let joined = render_withdraw_readback(&o).join("\n");
        assert!(
            joined.contains("NO LONGER STANDS"),
            "the direct effect must be stated: {joined}"
        );
        assert!(
            !joined.contains('⚠'),
            "an accountable withdrawal that took effect is not a warning: {joined}"
        );
        assert!(
            joined.contains("\"routine\""),
            "what now stands over that subject must be named: {joined}"
        );
    }

    #[test]
    fn an_inert_withdrawal_says_the_assertion_still_stands() {
        // The defect #435 exists for: the verb used to print a bare "withdrew ...", which
        // an operator reads as "protection removed" when nothing moved at all.
        let o = WithdrawOutcome {
            worklist_reason: Some("inert".into()),
            target: held(true, "restricted"),
        };
        let joined = render_withdraw_readback(&o).join("\n");
        assert!(
            joined.contains('⚠'),
            "an inert withdrawal must warn: {joined}"
        );
        assert!(
            joined.contains("STILL STANDS"),
            "the grade did not move, and the line must say so: {joined}"
        );
        assert!(
            joined.contains("ADR-0064"),
            "the reason belongs in the sentence, not a footnote: {joined}"
        );
        assert!(
            joined.contains("\"restricted\""),
            "the unmoved grade must be named: {joined}"
        );
    }

    #[test]
    fn a_stranger_attested_withdrawal_is_never_reported_as_ineffective() {
        // THE REGRESSION GUARD. `stranger-attested` means the withdrawal CLEARED the bar
        // and protection WAS removed; it is on the worklist as salience, not as failure.
        // The Slice 69 review found exactly this arm counted under "did NOT take effect"
        // on the chart report, while the grade line above already showed the lowered
        // grade. The same collapse must never appear on this verb.
        let o = WithdrawOutcome {
            worklist_reason: Some("stranger-attested".into()),
            target: held(false, "routine"),
        };
        let joined = render_withdraw_readback(&o).join("\n");
        assert!(
            joined.contains("NO LONGER STANDS"),
            "protection WAS removed — say so first: {joined}"
        );
        assert!(
            !joined.contains("STILL STANDS"),
            "the target is gone; claiming otherwise inverts the outcome: {joined}"
        );
        assert!(
            !joined.to_lowercase().contains("did not take effect"),
            "the completed-removal arm must never read as a failure: {joined}"
        );
        assert!(
            joined.contains('⚠'),
            "it is still a thing to look at — an unaccountable removal: {joined}"
        );
    }

    #[test]
    fn a_target_this_node_does_not_hold_claims_no_grade() {
        // db/048's `inert` arm merges two states its own comment separates: nobody
        // accountable stands behind the claim, and the target has not replicated here yet
        // (`target_content_address IS NULL`). Reading the worklist alone cannot tell them
        // apart, which is why the read-back looks the target up directly.
        let o = WithdrawOutcome {
            worklist_reason: Some("inert".into()),
            target: TargetState::NotHeldHere,
        };
        let joined = render_withdraw_readback(&o).join("\n");
        assert!(
            joined.contains("has NOT replicated"),
            "name the real reason nothing can be said: {joined}"
        );
        assert!(
            !joined.contains("STILL STANDS") && !joined.contains("NO LONGER STANDS"),
            "this node cannot know either way — asserting one is a precise untruth: {joined}"
        );
        assert!(
            joined.contains("ADR-0064"),
            "the accountability fact is still known and still true here: {joined}"
        );
    }

    #[test]
    fn a_target_recorded_on_another_chart_warns_and_claims_no_effect() {
        // ADR-0064's KNOWN GAP, made visible on the one surface that can see it cheaply: a
        // withdrawal mis-stamped with the wrong chart's patient_id names a real assertion
        // that lives elsewhere. `cairn_sensitivity_standing` is patient-scoped on both
        // sides — load-bearing, or a withdrawal on chart B could strip chart A — so the
        // target is absent from THIS chart's standing set while being perfectly alive on
        // its own. Reading that absence as "protection was removed" is a precise untruth,
        // and in the reassuring direction.
        let o = WithdrawOutcome {
            worklist_reason: None,
            target: TargetState::OnAnotherChart,
        };
        let joined = render_withdraw_readback(&o).join("\n");
        assert!(
            joined.contains('⚠'),
            "a permanently inert withdrawal must warn: {joined}"
        );
        assert!(
            !joined.contains("NO LONGER STANDS"),
            "the assertion is alive on its own chart — this must never read as removal: \
             {joined}"
        );
        assert!(
            joined.contains("different chart"),
            "name the actual condition, which no arm of the worklist does: {joined}"
        );
    }

    #[test]
    fn an_unrecognised_subject_kind_reports_the_effect_but_no_reading() {
        // A future peer's subject kind (ADR-0056: admitted, not understood). Whether the
        // target still stands is STILL exactly knowable — it is a set-membership test over
        // content_address, which needs no understanding of the kind — but what now stands
        // OVER that subject is not resolvable here, and guessing would be the precise
        // untruth principle 4 forbids.
        let o = WithdrawOutcome {
            worklist_reason: None,
            target: TargetState::Held {
                still_standing: false,
                subject: SubjectResolution::Unrecognised("episode".into()),
            },
        };
        let joined = render_withdraw_readback(&o).join("\n");
        assert!(
            joined.contains("NO LONGER STANDS"),
            "the direct effect is still known: {joined}"
        );
        assert!(
            joined.contains("episode"),
            "name the kind this build cannot resolve: {joined}"
        );
    }

    #[test]
    fn an_unrecognised_worklist_reason_is_still_surfaced() {
        // `reason` is open vocabulary — a future db/048 may add a third arm. A build that
        // has never seen it must still show the operator that the row exists rather than
        // dropping it, the same discipline `withdrawal_reason_explanation` follows.
        let o = WithdrawOutcome {
            worklist_reason: Some("quarantined-pending-review".into()),
            target: held(false, "routine"),
        };
        let joined = render_withdraw_readback(&o).join("\n");
        assert!(
            joined.contains("quarantined-pending-review"),
            "an unknown reason must be shown verbatim, never dropped: {joined}"
        );
    }

    #[test]
    fn a_hostile_grade_cannot_forge_a_line_in_the_readback() {
        // Same provenance argument as `peer`: the grade is unconstrained TEXT copied from
        // a peer's own body, and this renderer is line-oriented. A newline would otherwise
        // let a peer forge a whole outcome line.
        let o = WithdrawOutcome {
            worklist_reason: None,
            target: held(
                false,
                "routine\n⚠ INERT — the withdrawn assertion STILL STANDS",
            ),
        };
        for line in render_withdraw_readback(&o) {
            assert!(
                !line.trim_start().starts_with("⚠ INERT"),
                "a newline in peer text forged a line: {line}"
            );
        }
    }
}
