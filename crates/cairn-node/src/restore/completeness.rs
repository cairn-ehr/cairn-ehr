//! #594 / ADR-0071 — **did the restore bring everything back, and how does it say so?**
//!
//! ## Why this module exists
//!
//! `cairn-node restore` is the command a solo clinic reaches for after it has already lost its
//! disk. It is also, increasingly, a command run by a **cron-driven drill** with no human at the
//! terminal (ADR-0069 gave it a non-interactive path precisely so a clinic could rehearse). A
//! script reading such a run has exactly one channel it can rely on: the **exit status**. Warnings
//! on stderr are dropped by half the cron wrappers in existence, and the stdout summary needs a
//! parser.
//!
//! Before #594 that one channel could not tell these two runs apart:
//!
//! - every record on the medium is now in the log — a clean recovery; and
//! - three nights of charts sat past a broken chain link, were never offered to the apply door,
//!   and **no retry of any command will ever reach them**.
//!
//! Both exited **0**. That is #500's own signature — a restore that reads "restored" to a clinic
//! which then believes it has its charts back — reappearing in the mechanism built to prevent it.
//!
//! ## What this module decides
//!
//! One question, over five scalars: *is any record the medium carried still not usable in this
//! node's log?* **"Usable", not merely "present"** — the one *cause below* where the distinction
//! bites is [`Unrestored::penned`](crate::restore::completeness::Unrestored::penned), whose doc
//! names it: a record the door admitted but whose custody did not land is a row in `event_log`
//! wrapping a sealed body nobody can open, and a restore that reported that as recovered would be
//! making #500's claim one layer down.
//!
//! ⚠️ **"Usable" is bounded BY the five causes; it is not an open-ended promise.** A record this
//! build cannot CLASSIFY is equally unusable — admitted uninterpreted, counted `applied`, yielding
//! no projection and no chart until the node is upgraded — and it exits **0**, because no cause
//! below covers it. That gap is real and is filed as
//! [#614](https://github.com/cairn-ehr/cairn-ehr/issues/614); read this question as *"does any of
//! the five hold?"*, never as *"is this recovery whole?"*. If so the run is
//! **INCOMPLETE** ([`EXIT_INCOMPLETE`](crate::restore::completeness::EXIT_INCOMPLETE),
//! the status `cairn-sync requeue` has
//! used since #578) and [`Unrestored::notice`](crate::restore::completeness::Unrestored::notice) says
//! which of the five causes hold, with each one's
//! remedy. If not, the run says nothing and exits 0.
//!
//! It is deliberately **pure** — no database, no I/O, no `main.rs` locals — so the rule can be read
//! and falsified in one screen without rehearsing a disaster to run it against
//! (`tests/restore_exit_vocabulary.rs` does exactly that). `main.rs` fills in the five fields from
//! its own variables and does nothing else with the answer but print it and exit.
//!
//! ## What INCOMPLETE is NOT
//!
//! **It is not FAILED.** Exit **1** stays reserved for a run that was *blocked*: a recovery code
//! that could not be **read at all** (no `--old-recovery-code-file` and no tty), a bundle this
//! build cannot **decode**, recovered key material that could not be **installed**, a database
//! fault, an interrupted ceremony.
//!
//! ⚠️ **Two different mechanisms produce that 1, and only one of them is a "precedence".** A
//! database fault or a clinical apply that dies is an `?` far earlier in the arm — it never reaches
//! either verdict, and it loses the summary this block works so hard to preserve (that loss is
//! [#616](https://github.com/cairn-ehr/cairn-ehr/issues/616), not a property anyone chose). The
//! *held* failure, `local_state_failure`, is the one `main.rs` deliberately checks FIRST at the
//! verdict site, and that check is what makes FAILED outrank INCOMPLETE.
//!
//! ⚠️ **A wrong recovery code and a corrupt export are NOT exit 1.** `apply_local_state_export`
//! returns `Ok(None)` for both — *an honest degradation already reported to the operator* — and the
//! run **usually** lands on **3**, because without the export there is usually no actor registry
//! either and the clinical plane is never offered. **Usually, not always**: on a medium carrying no
//! clinical records at all there is nothing to leave behind, every field below is zero, and such a
//! run exits **0** having installed no custody key — correct by this module's own rule, and filed
//! as [#613](https://github.com/cairn-ehr/cairn-ehr/issues/613) because it is still not what a
//! drill wrapper reading 0 believes. The line is not *"did the export work?"* but *"could this
//! command do what it set out to do?"*: an unreadable prompt means the ceremony never ran; a wrong
//! code means it ran and fell short. The distinction is between *"the restore did everything it safely
//! could and work remains"* and *"the restore could not do what it set out to do"*, and it is the
//! same line `requeue` draws (see `cairn_sync::requeue::EXIT_INCOMPLETE`'s doc).
//!
//! **It is not a refusal.** [ADR-0068](../../../../docs/spec/decisions/0068-provenance-warns-never-gates-on-the-restore-path.md)
//! decision 1 — *refusing converts a partial loss into a total one* — is about **gating**, and
//! nothing here gates. Every record this build is entitled to apply has already been applied by the
//! time the verdict is computed. A non-zero status taken after that refuses nothing.

/// The exit status of a restore that finished its ceremony but did not finish the recovery.
///
/// **There is a SECOND definition of this number, `cairn_sync::requeue::EXIT_INCOMPLETE`, and only
/// a test holds the two equal.** `cairn-sync` cannot alias THIS constant: it is a binary-only crate
/// whose `cairn-node` dependency is a **dev**-dependency, so importing it in production code would
/// pull this whole crate into that binary's build graph for an integer. The guard is
/// `cairn_sync::requeue::tests::exit_incomplete_matches_cairn_nodes_restore`, which fails naming
/// the other constant. **Change one and you must change both.**
///
/// ⚠️ **That is a reason the ALIAS cannot go here, not a proof no shared home exists — and the
/// difference matters, because the next reviewer will check.** Both crates depend, in production,
/// on `cairn-event` and `cairn-keystore`, so a single definition in either needs no new crate and
/// no new dependency edge. It is **rejected on §9 blast-radius grounds**: `cairn-event` is the
/// safety-critical core the project has committed to keeping small and reviewable, and a CLI exit
/// status is not an event concept. The two-test arrangement is a deliberate trade, not a
/// workaround — and it is genuinely two-sided: this side pins that the constants AGREE,
/// `restore_exit_vocabulary.rs` pins that the value is **3**, so they cannot both drift to 7
/// together.
///
/// A **third** definition exists and is deliberately NOT bound to these:
/// `cairn-sync/tests/common/dead_node.rs`'s own `EXIT_INCOMPLETE`. That one is an independent
/// oracle — a suite that read the number back out of the code under test could not catch the code
/// changing it — so it is meant to be edited by hand, and to go red when it is not. Do not "fix" it
/// into an import. (Its own doc still says it "has to be" a copy because the module is
/// unimportable; since #594 that is half-stale — `cairn-node` is a dev-dependency there, so it
/// *could* import this, and staying a copy is now the choice this paragraph describes.)
///
/// **The bare `3`s in the test suites are not part of that inventory.** The CLI tests assert
/// `out.status.code() == Some(3)` as a literal on purpose: a test that imported the constant would
/// still pass if the constant changed, which is the whole failure the third copy above guards
/// against. They are oracles, not definitions.
///
/// Distinct from `1`, which is a run that FAILED, and from `2`, which is a bad flag.
pub const EXIT_INCOMPLETE: i32 = 3;

/// Everything a finished `restore` left behind — one field per way a record can fail to reach the
/// log, each with a different remedy and a different degree of recoverability.
///
/// All five are `Default`-zero, so a clean restore is `Unrestored::default()`. `main.rs` builds one
/// from its own locals at the tail of the `Cmd::Restore` arm, *after* the whole summary has printed.
///
/// **The field set is closed at five, and one near-miss is deliberately absent.** A penned row that
/// an operator has already **acked** is not a sixth cause: `ClinicalRestoreReport::penned()` already
/// counts it, and the summary carries its own NOTE explaining that `requeue` skips it. Adding an
/// `acked` field would double-count the same rows and let a restore that penned nothing report
/// INCOMPLETE. (Pinned by `restore_exit_vocabulary.rs::the_cause_list_is_exactly_five`, which fails
/// to compile if a sixth field appears — so a new cause gets a deliberate decision, not a silent
/// widening.)
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Unrestored {
    /// Clinical records past this medium's last verified chain link. They are still ON the medium
    /// but were never offered to the apply door (a segment hanging from an unverified predecessor
    /// could have been spliced in whole — 2a invariant 5), and **nothing recovers them**: they
    /// cannot be requeued, because they were never penned.
    pub past_chain_break: usize,
    /// Records under a plane tag this build does not recognise — a newer Cairn's. Still on the
    /// medium, and recoverable: upgrade the node and restore again.
    pub unknown_plane: usize,
    /// The medium's tail was torn. Unlike the other four this carries no count, because there is
    /// none to carry: a torn tail is equally consistent with a truncated copy, so how much is
    /// missing is unknown (principle 4 — `backup::torn_tail_notice`'s doc argues it at length).
    /// The intact prefix HAS been restored; what came after it is not on this copy.
    pub torn_tail: bool,
    /// Clinical records held in the quarantine pen with their custody. The most recoverable of the
    /// five: `cairn-sync requeue` completes the restore without redoing it.
    ///
    /// ⚠️ **One of the four refusal causes pens a record that IS in the log.**
    /// `RefusalCause::CustodyDidNotLand` fires *after* the door returned `Ok` — db/020's two
    /// lenient arms admit the event and withhold its custody — so the body is stored and no key
    /// opens it. It belongs here anyway: a sealed body without its DEK has not "come back" in any
    /// sense a clinic would recognise, and `requeue` is still its remedy. This is why the module
    /// doc's question is *usable in the log*, not *present in it*.
    ///
    /// ⚠️ **This count INCLUDES rows an operator has already acked, and `requeue` skips those.**
    /// `ClinicalRestoreReport::penned_but_acked` is a strict subset of `penned()` (both are
    /// incremented by the same `pen()` call), and `do_requeue` deliberately does not put an acked
    /// row through the door — `RequeueCounts::is_incomplete` excludes `skipped_acked`, so requeue
    /// exits **0** having released none of them. A resumed restore over a pen whose rows were all
    /// acked therefore reports 3 with no command that clears it. Counting them is still right —
    /// those records genuinely are not in the record — but [`Unrestored::notice`] must not promise
    /// `requeue` for them without the caveat, and it does not.
    pub penned: usize,
    /// No actor registry reached this node, so the apply door would have refused every clinical
    /// record as authored by an unenrolled signer and the plane was never offered at all. The
    /// remedy is NOT `requeue` — `finalize_identity` has already closed the registry door
    /// permanently — but a second restore into a freshly created database (#554 finding 4).
    pub no_registry: bool,
}

impl Unrestored {
    /// True when every record the medium carried is in this node's log.
    ///
    /// The whole INCOMPLETE decision, in one place: `main.rs` adds no second condition of its own
    /// to it, and there is no sixth cause hiding at the call site.
    ///
    /// ⚠️ **A `false` here is NOT "therefore exit 3" — FAILED outranks it.** `main.rs` returns
    /// `local_state_failure` *before* it ever builds this struct, so a blocked ceremony that also
    /// left records behind exits **1**. That is not an exception to the rule; it is the precedence
    /// this whole slice establishes (see the module doc above), and it is pinned end to end by
    /// `restore_cli_surface.rs::without_the_flag_a_piped_restore_still_inherits_no_custody` — the
    /// one run that carries both verdicts at once.
    pub fn is_complete(&self) -> bool {
        self.past_chain_break == 0
            && self.unknown_plane == 0
            && !self.torn_tail
            && self.penned == 0
            && !self.no_registry
    }

    /// The operator's verdict: `None` when the restore is complete, else every cause that holds.
    ///
    /// **Why it names every cause and not just the first.** These are not alternatives — a medium
    /// can be torn *and* carry an unroutable plane *and* pen what it did offer — and each one has a
    /// different remedy. A verdict that stopped at the first would send an operator to fix one
    /// thing and leave believing they were done.
    ///
    /// **Why each line repeats a remedy the summary already printed.** Same discipline as the
    /// torn-tail note and the untrusted note above it: an operator reading only the last screen of
    /// a long restore — or a cron log that kept stderr and dropped stdout — must still be told what
    /// to do next. Duplication is cheap; a remedy discovered at an empty pen is not.
    ///
    /// **Why the text states the number as well as the word.** Two different readers: the human
    /// needs "INCOMPLETE, not FAILED" (their charts ARE back, minus what is named), and whoever
    /// later debugs the cron wrapper needs to know which status produced this line.
    pub fn notice(&self) -> Option<String> {
        if self.is_complete() {
            return None;
        }
        let mut out = format!(
            "restore: INCOMPLETE (exit {EXIT_INCOMPLETE}) — this node IS restored and everything \
             above it applied is in the log, but the following did NOT come back. This is not a \
             failed restore; it is an incomplete recovery, and each cause below has its own remedy."
        );
        // Ordered by how little the operator can do about it: the two that no retry reaches come
        // first, so the causes that matter most are not buried under the ones a command fixes.
        if self.torn_tail {
            out.push_str(
                "\n  · The medium was TORN. Only its intact, verified prefix was restored; whatever \
                 followed is not on this copy and cannot be recovered from it. If the source node \
                 is still reachable, compare its `backup-status.json` before assuming nothing more \
                 is missing.",
            );
        }
        if self.past_chain_break > 0 {
            out.push_str(&format!(
                "\n  · {} clinical record(s) are past this medium's last verified chain link and \
                 were NEVER offered to the apply door. They are still on the medium, but no retry \
                 of this or any other command reaches them — a segment hanging from an unverified \
                 predecessor could have been spliced in whole. Find another copy of this backup.",
                self.past_chain_break
            ));
        }
        if self.no_registry {
            out.push_str(
                "\n  · The clinical plane was NOT restored at all: this node has no actor registry, \
                 so every record would have been refused as authored by an unenrolled signer. \
                 `cairn-sync requeue` will NOT fix this — `finalize_identity` has already closed \
                 the registry door on this database. Recover the local-state export and RESTORE \
                 AGAIN, from this same medium, into a FRESHLY CREATED database. A second \
                 superseding identity is auditable and expected.",
            );
        }
        if self.unknown_plane > 0 {
            out.push_str(&format!(
                "\n  · {} record(s) are in a plane this build cannot route. They are untouched on \
                 the medium: upgrade this node and restore again from it.",
                self.unknown_plane
            ));
        }
        if self.penned > 0 {
            out.push_str(&format!(
                "\n  · {} clinical record(s) are HELD in the quarantine pen with their custody. \
                 Inspect them with `cairn-sync quarantine`; once the cause is fixed, `cairn-sync \
                 requeue` completes the restore without redoing it — EXCEPT for any row already \
                 ACKED, which `requeue` deliberately skips (it is a recorded human decision that \
                 those bytes never enter the record). If requeue reports nothing left to do while \
                 this count stands, that is why: un-ack them first if the decision no longer holds.",
                self.penned
            ));
        }
        Some(out)
    }
}
