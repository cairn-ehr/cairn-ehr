//! The medication read model shared by the node's read path, the CLI, and the UI.
//!
//! WHY A SHARED CRATE. Two consumers must agree on one question — *which threads does a
//! sign-off gesture attest?* `cairn-node`'s orchestrator answers it to decide what to
//! sign; the UI answers it to tell the clinician what is about to be signed. If those
//! were two implementations, a divergence would put a green "signed" badge over a thread
//! nobody signed. So the model and the rule live here, and both sides depend on it.
//!
//! This crate is deliberately pure: no database driver, no GUI toolkit. That is what lets
//! the GUI tab crate test in milliseconds without Postgres in the build tree.
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Whether a displayed medication group is still being taken.
///
/// Ceased rows are RETAINED in the list, not filtered out: a struck line stays visible on
/// a paper drug chart, and dropping it would lose that parity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MedicationStatus {
    Active,
    Ceased,
}

/// The ADR-0049 sign-off state of ONE medication thread.
///
/// `by` is the attester's hex key id, as recorded in `medication_attestation.attester_kid`.
/// Staleness is NOT computed here — it is read from `medication_thread_attestation.stale`,
/// which the database derives from the set-commitment compare. A second implementation of
/// staleness would be a second answer to a safety question.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum VouchState {
    /// No attestation on this thread at all.
    Absent,
    /// A current vouch.
    Fresh { by: String },
    /// A vouch whose set-commitment no longer matches the thread's content.
    Stale { by: String },
}

impl VouchState {
    /// True when a sign-off gesture must (re-)vouch this thread.
    pub fn needs_signature(&self) -> bool {
        matches!(self, VouchState::Absent | VouchState::Stale { .. })
    }

    /// The attester's key id, when there is one.
    pub fn attester(&self) -> Option<&str> {
        match self {
            VouchState::Absent => None,
            VouchState::Fresh { by } | VouchState::Stale { by } => Some(by),
        }
    }
}

/// One member thread of a displayed row, with the vouch that thread carries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemberVouch {
    pub medication_id: Uuid,
    pub vouch: VouchState,
    /// The chart this thread lives on (ADR-0076 decision 2). A combined list's sign-off
    /// gesture attests each thread under the chart it actually belongs to, not under
    /// whichever chart the read happened to be opened from — so once a read can cover more
    /// than one linked chart, the attestation target must be carried on the member itself
    /// rather than assumed from the enclosing list.
    pub patient_id: Uuid,
}

/// WHY a line is withheld from sign-off as a wrong-chart hazard (#697). Three independent facts,
/// any of which is enough, and any combination of which can hold at once:
///
/// - `outside_set`: the group reaches a chart OUTSIDE the set the list was read over, so some
///   thread on this line is recorded on another person's chart (issue #334). Remedy: separate
///   the threads (`SEPARATION_INSTRUCTION`).
/// - `doubted_link`: the set holds a link this node DOUBTS (db/054: an un-attested link its
///   hard veto flagged, or trips now; or a clinician's attested unlink between two charts the
///   set still joins), and this line is not recorded only on the opened chart. A signature is
///   a claim about a person, and the node has positive evidence the other member may be
///   someone else. Remedy: a human judges the LINKS (`DOUBTED_LINK_INSTRUCTION`).
/// - `patient_conflict` (#836): an event on one of this line's threads names another person's
///   chart. It was admitted (a synced one, because refusing it would fork the event set; a
///   dose or correction written here before its thread's claim, because the local door cannot
///   compare against a chart nobody has named yet) and projected onto the thread: another
///   person's W can show on this drug, or a release written for someone else can end this
///   patient's W. It is found by db/031's `medication_patient_conflict_flag` or, when it
///   arrived before anything tied its thread to a chart, by db/059's
///   `medication_thread_stray_chart` view (#846). Remedy: `conflict_remedy(status)`
///   (a current line: check it with the patient, then stop it and record it again; a ceased
///   line: the stop itself may be the stray event).
///
/// Built by `cairn-node`'s `medication::hazard::wrong_chart_reasons`, the one rule.
///
/// Serialize only, like `MedicationRow`: nothing reads either back, and a deserializer would
/// be a second way to build a row, outside the one rule.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct WrongChartReasons {
    pub outside_set: bool,
    pub doubted_link: bool,
    pub patient_conflict: bool,
}

impl WrongChartReasons {
    /// Whether any reason holds.
    pub fn any(&self) -> bool {
        self.outside_set || self.doubted_link || self.patient_conflict
    }

    /// The reasons a renderer words for a line it knows is a hazard: these, or, when none is
    /// set, the outside-the-set reason (the only meaning the hazard flag had before #697).
    /// Renderers print one block per reason, so a hazard with an empty set would otherwise
    /// print nothing and vanish from the report. One fallback, used by `hazard_reasons` and by
    /// every renderer of a `WithheldLine`.
    pub fn worded(self) -> Self {
        if self.any() {
            self
        } else {
            WrongChartReasons {
                outside_set: true,
                ..Self::default()
            }
        }
    }
}

/// One displayed row = one medication GROUP.
///
/// A group is what `patient_medication_current` emits: reconciled duplicate threads
/// (ADR-0047) collapse into a single clinical statement. Attestation, however, is
/// per-THREAD, so the row carries its members and each member's vouch. That group/thread
/// asymmetry is the most defect-prone seam in this slice — see the tests in
/// `targeting.rs` and `crates/cairn-node/tests/medication_read.rs`.
///
/// Serialize only (the CLI's `--json`). Do NOT derive `Deserialize`: nothing reads a row back,
/// and a deserializer would build one whose hazard fields no rule set — the same reason
/// `ChartSet` has none.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MedicationRow {
    pub group_id: Uuid,
    /// The chart the GROUP displays under: the list view's single display winner
    /// (`medication_group_display`'s DISTINCT ON pick), carried through as-is.
    ///
    /// Since the combined read (ADR-0076) this is NOT "the chart this line is on". For a
    /// group spanning two linked charts it is whichever one won, and for a cross-patient
    /// group it can be a chart OUTSIDE the set the list was read over — someone else's.
    /// Nor is it where a signature goes: sign-off attests each thread under its own chart.
    /// For where the drug was recorded, read `source_charts`; for the chart a thread lives
    /// on (the attestation target), read `MemberVouch::patient_id`.
    ///
    /// Named `display_chart` in Rust so no later writer mistakes it for a write target (it
    /// was `patient_id`, which invites exactly that); serialized as `patient_id` so the
    /// `--json` output and the never-linked golden are unchanged.
    #[serde(rename = "patient_id")]
    pub display_chart: Uuid,
    /// The free-text term as asserted — may legitimately be vague ("little white pill").
    pub term: String,
    /// The ADR-0059 coded display name, when the drug has been coded.
    pub coding_display: Option<String>,
    pub formulation: Option<String>,
    pub dose_amount: Option<String>,
    pub dose_unit: Option<String>,
    pub sig: Option<String>,
    pub started_value: Option<String>,
    pub started_precision: Option<String>,
    pub status: MedicationStatus,
    pub members: Vec<MemberVouch>,
    /// This group shares a duplicate key with another un-reconciled group
    /// (`patient_medication_reconciliation_flag`). Advisory worklist, never auto-resolved.
    pub reconciliation_flagged: bool,
    /// Two different drug anchors inside one reconciled group
    /// (`medication_group_coding_conflict`) — a possible mis-reconciliation.
    pub coding_conflict: bool,
    /// This line is a wrong-chart hazard, never a sign-off target; `wrong_chart` says why
    /// (#697). Kept, rather than replaced by `wrong_chart`, because the `--json` output's
    /// readers check this key. The rule is `cairn-node`'s `medication::hazard::wrong_chart_reasons`.
    pub cross_patient: bool,
    /// Why `cross_patient` is set (#697): the reasons a renderer words, each with its own
    /// remedy. Read through `is_wrong_chart_hazard` or `hazard_reasons`, never directly: that is
    /// where a row whose flag and reasons disagree is resolved in the fail-safe direction.
    pub wrong_chart: WrongChartReasons,
    /// The charts owning at least one member thread of this group, sorted. The row names
    /// where the drug was recorded so a clinician reading a combined list — one read over
    /// several linked charts (ADR-0076) — can tell which chart a line came from, rather than
    /// having to infer it from which patient happened to be open.
    ///
    /// Empty only in a race: a concurrent reconciliation or separation re-keyed the group
    /// between two of the read's statements (READ COMMITTED, one snapshot per statement —
    /// see `cairn-node`'s `medication/read.rs`). Renderers name that state ("not read")
    /// rather than printing an empty label.
    pub source_charts: Vec<Uuid>,
    /// The holds on this line that are in force or not yet started, latest start first
    /// (ADR-0081). A hold does NOT change `status`: a withheld line is still on the current
    /// list, signable and ceasable (a third status would drop it out of sign-off). Always
    /// empty unless `status` is Active: cessation wins over any hold.
    pub holds: Vec<MedicationHold>,
    /// Why the line was ceased, when the cessation gave a reason (non-empty only). Always
    /// `None` unless `status` is Ceased. This is the reason of the stop whose date the line
    /// shows; a line stopped more than once has its other reasons in
    /// `other_cessation_reasons`.
    pub cessation_reason: Option<String>,
    /// Every OTHER distinct reason a stop of this line gave (any member thread), newest
    /// recorded first, never repeating `cessation_reason` (#857). A second stop is admitted on
    /// purpose (ADR-0082 decision 4), and the first one's reason may be the only record of an
    /// adverse reaction, so a renderer that shows `cessation_reason` must show these too —
    /// including when `cessation_reason` is `None` (a later stop that gave no reason). Always
    /// empty unless `status` is Ceased.
    pub other_cessation_reasons: Vec<String>,
}

/// One hold on a medication line (ADR-0081, #803): the paper drug chart's "W".
/// Instants are RFC 3339 UTC text from the DB, to the second (`2026-10-21T20:00:45Z`).
/// Serialize only, like `MedicationRow`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MedicationHold {
    pub withhold_event_id: Uuid,
    /// The thread the withhold named (a member of the line's group) — the thread a release
    /// must name too.
    pub medication_id: Uuid,
    pub reason: String,
    pub starts_at: String,
    /// The instant the hold ends, or the literal `"reviewed"` for an open hold — the wire's
    /// own spelling. Never null: a null would read as "end unknown", which an open hold is
    /// not (principle 4).
    pub until: String,
    /// Key id of the clinician who withheld it.
    pub withholder: String,
    /// Not yet started: listed, but the line is not withheld by it.
    pub pending: bool,
    /// When a release already recorded will end this hold, if that instant is still ahead
    /// (#835): a peer's clock ran fast, or a writer other than Cairn's release verb (which
    /// always stamps the real now) set it later. The hold still stands until then (listed, or
    /// in force once started), but every surface must say a release exists, or the record
    /// holds a fact nobody is shown. Same RFC 3339 UTC text as `starts_at`.
    pub release_pending: Option<String>,
}

impl MedicationHold {
    /// Whether the recorded release (#835) lands at or before this hold's start, so it cancels
    /// a hold that never begins, rather than ending one. Surfaces must say which: "from 08:00,
    /// release at 16:00" with the release first reads as a W from 08:00 to 16:00.
    pub fn release_cancels_it(&self) -> bool {
        // A hold in force has started: a release can only end it.
        // shortcut: compares the text, valid because both instants come from the read's one
        // fixed-width UTC format; parse them if either ever arrives in another form.
        self.pending
            && self
                .release_pending
                .as_deref()
                .is_some_and(|at| at <= self.starts_at.as_str())
    }
}

/// The sentence a stop says when `what` ("This drug", or the CLI's "This thread") was already
/// stopped before it reached the record (#843, #854), naming the earlier stop's reason (`None`
/// when none was recorded). Since #857 the list keeps every stop's reason (db/060), which is
/// what lets this sentence say so.
pub fn already_stopped_sentence(what: &str, earlier_reason: Option<&str>) -> String {
    let earlier = match earlier_reason {
        Some(reason) => format!("its reason: {reason}"),
        None => "no reason was recorded".to_string(),
    };
    format!(
        "{what} had already been stopped before this stop reached the record ({earlier}). \
         Your stop is recorded too, and the list keeps every stop's reason."
    )
}

impl MedicationRow {
    /// What a stop of this line says when the line, read BEFORE the stop was written, was
    /// already stopped (#843, #854; ADR-0082 decision 4): a colleague's stop landed first. The
    /// stop is still written, because the node admits a cease of a ceased line and its reason
    /// (an adverse reaction, say) may be the only record of it while no allergy stream exists.
    /// This sentence keeps "stopped" from being read as "this stop stopped it". `None` for a
    /// line that is not ceased. The window stops every thread of a line, so this LINE rule is
    /// its whole answer; the CLI stops one thread and also asks about that thread.
    pub fn already_stopped_note(&self) -> Option<String> {
        if self.status != MedicationStatus::Ceased {
            return None;
        }
        Some(already_stopped_sentence(
            "This drug",
            self.cessation_reason.as_deref(),
        ))
    }

    /// Whether any hold on this line was in force at the instant the row was READ (pending
    /// ones do not count). A kept row goes stale as holds start and end: re-read it.
    pub fn is_withheld(&self) -> bool {
        self.holds.iter().any(|h| !h.pending)
    }

    /// Whether this line must be withheld from sign-off as a wrong-chart hazard. Either signal
    /// is enough, so a builder that sets only one of the two fields still withholds the line
    /// (fail-safe; the read sets both from one rule).
    pub fn is_wrong_chart_hazard(&self) -> bool {
        self.cross_patient || self.wrong_chart.any()
    }

    /// Why this line is a wrong-chart hazard, or `None` when it is not one. A hazard with no
    /// recorded reason (a builder that predates #697) is worded as reaching outside the set
    /// (`WrongChartReasons::worded`).
    ///
    /// BLIND TO STATUS AND VOUCH. A ceased or already-signed hazard line returns `Some`, but
    /// sign-off withholds nothing from it: nothing on it needs a signature. Whether THIS
    /// gesture withholds a line is `targeting::withheld_reasons`, and anything that tells the
    /// reader a line "cannot be signed" or points at the withheld report must ask that one.
    /// (Named `withheld_because` until the PR #717 review found the CLI pointing every hazard
    /// row at a note printed only for withheld ones.)
    pub fn hazard_reasons(&self) -> Option<WrongChartReasons> {
        self.is_wrong_chart_hazard()
            .then(|| self.wrong_chart.worded())
    }

    /// The name the clinician actually sees: the coded display name when the drug has been
    /// coded, else the term exactly as asserted. Every renderer and the sort MUST use this, or
    /// the chart is ordered by a string the reader cannot see (a coded chart would sort under
    /// its invisible `term` while the clinician reads `coding_display` — e.g. "Lipitor" filed
    /// under "atorvastatin").
    pub fn display_name(&self) -> &str {
        self.coding_display.as_deref().unwrap_or(&self.term)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A row carrying just the two fields `display_name` chooses between. The rest are the
    /// neutral empty values — this is a unit test of one accessor, not of the read path.
    fn row(term: &str, coding_display: Option<&str>) -> MedicationRow {
        MedicationRow {
            group_id: Uuid::from_u128(1),
            display_chart: Uuid::from_u128(2),
            term: term.into(),
            coding_display: coding_display.map(Into::into),
            formulation: None,
            dose_amount: None,
            dose_unit: None,
            sig: None,
            started_value: None,
            started_precision: None,
            status: MedicationStatus::Active,
            members: vec![],
            reconciliation_flagged: false,
            coding_conflict: false,
            cross_patient: false,
            wrong_chart: WrongChartReasons::default(),
            source_charts: vec![],
            holds: vec![],
            cessation_reason: None,
            other_cessation_reasons: vec![],
        }
    }

    /// The `--json` contract for #803: exact key names, `until: "reviewed"` for an open hold, and
    /// (#835) `release_pending` as an instant or `null`, never absent.
    #[test]
    fn holds_and_cessation_reason_serialize_with_pinned_keys() {
        let mut r = row("x", None);
        r.cessation_reason = Some("rash".into());
        r.other_cessation_reasons = vec!["angioedema".into()];
        r.holds = vec![
            MedicationHold {
                withhold_event_id: Uuid::from_u128(1),
                medication_id: Uuid::from_u128(2),
                reason: "BP".into(),
                starts_at: "2026-10-21T20:00:00Z".into(),
                until: "2026-10-22T08:00:45Z".into(),
                withholder: "k1".into(),
                pending: false,
                release_pending: Some("2026-10-22T06:00:00Z".into()),
            },
            MedicationHold {
                withhold_event_id: Uuid::from_u128(3),
                medication_id: Uuid::from_u128(2),
                reason: "GP".into(),
                starts_at: "2026-10-21T21:00:00Z".into(),
                until: "reviewed".into(),
                withholder: "k2".into(),
                pending: true,
                release_pending: None,
            },
        ];
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(v["cessation_reason"], "rash");
        assert_eq!(
            v["other_cessation_reasons"],
            serde_json::json!(["angioedema"])
        );
        assert_eq!(
            v["holds"],
            serde_json::json!([
                {"withhold_event_id": Uuid::from_u128(1), "medication_id": Uuid::from_u128(2),
                 "reason": "BP", "starts_at": "2026-10-21T20:00:00Z",
                 "until": "2026-10-22T08:00:45Z", "withholder": "k1", "pending": false,
                 "release_pending": "2026-10-22T06:00:00Z"},
                {"withhold_event_id": Uuid::from_u128(3), "medication_id": Uuid::from_u128(2),
                 "reason": "GP", "starts_at": "2026-10-21T21:00:00Z",
                 "until": "reviewed", "withholder": "k2", "pending": true,
                 "release_pending": null}
            ])
        );
    }

    #[test]
    fn is_withheld_ignores_pending_holds() {
        let hold = |pending| MedicationHold {
            withhold_event_id: Uuid::from_u128(1),
            medication_id: Uuid::from_u128(2),
            reason: "r".into(),
            starts_at: "2030-01-01T00:00:00Z".into(),
            until: "reviewed".into(),
            withholder: "k".into(),
            pending,
            release_pending: None,
        };
        let mut r = row("x", None);
        assert!(!r.is_withheld());
        r.holds = vec![hold(true)];
        assert!(!r.is_withheld());
        r.holds.push(hold(false));
        assert!(r.is_withheld());
    }

    /// #854: the rule both surfaces share. An active line says nothing; a ceased one names the
    /// earlier reason and says the list keeps every stop's reason (true since #857, db/060).
    #[test]
    fn already_stopped_note_names_the_earlier_reason_and_what_the_list_keeps() {
        let mut r = row("x", None);
        assert_eq!(r.already_stopped_note(), None);
        r.status = MedicationStatus::Ceased;
        let none = r
            .already_stopped_note()
            .expect("a ceased line earns the note");
        assert!(none.contains("no reason was recorded"), "{none}");
        r.cessation_reason = Some("angioedema".into());
        let note = r.already_stopped_note().unwrap();
        assert!(note.contains("its reason: angioedema"), "{note}");
        assert!(
            note.contains("the list keeps every stop's reason"),
            "{note}"
        );
        assert!(!note.contains("only the latest"), "{note}");
    }

    /// #835 review: a release at or before a pending hold's start cancels it — the hold never
    /// begins — while one after its start ends it then.
    #[test]
    fn a_release_at_or_before_the_start_cancels_the_hold() {
        let hold = |release: Option<&str>| MedicationHold {
            withhold_event_id: Uuid::from_u128(1),
            medication_id: Uuid::from_u128(2),
            reason: "r".into(),
            starts_at: "2030-01-02T00:00:00Z".into(),
            until: "reviewed".into(),
            withholder: "k".into(),
            pending: true,
            release_pending: release.map(Into::into),
        };
        assert!(!hold(None).release_cancels_it());
        assert!(hold(Some("2030-01-01T23:59:59Z")).release_cancels_it());
        assert!(hold(Some("2030-01-02T00:00:00Z")).release_cancels_it());
        assert!(!hold(Some("2030-01-02T00:00:01Z")).release_cancels_it());
        let mut live = hold(Some("2030-01-01T23:59:59Z"));
        live.pending = false;
        assert!(
            !live.release_cancels_it(),
            "a hold in force is ended, never cancelled"
        );
    }

    #[test]
    fn an_uncoded_row_displays_its_asserted_term() {
        assert_eq!(
            row("little white pill", None).display_name(),
            "little white pill"
        );
    }

    #[test]
    fn a_coded_row_displays_its_coded_name() {
        assert_eq!(
            row("atorvastatin", Some("Lipitor 40 mg tablet")).display_name(),
            "Lipitor 40 mg tablet"
        );
    }

    /// The case the accessor exists to prevent: sorting a coded chart by the invisible
    /// `term` files "Lipitor" under "atorvastatin", so the clinician's eye lands nowhere
    /// near where the row actually is. These two rows sort in OPPOSITE orders by `term`
    /// and by `display_name`, so this test fails if any caller reverts to sorting on
    /// `term`.
    ///
    /// The fixture is chosen around a real property of the comparison the read path uses:
    /// it is a BYTE-order compare (deliberately, so the order cannot depend on the
    /// database's collation — ADR-0045), and in ASCII every capital letter sorts before
    /// every lowercase one. So a capitalised brand name like "Lipitor" lands ahead of a
    /// lowercase generic like "aspirin" regardless of letter. That is what makes these two
    /// orders diverge here, and it is also a real cognitive-load wart for the eventual
    /// chart UI (issue #337) — recorded rather than hidden behind a tidier fixture.
    #[test]
    fn sorting_by_display_name_differs_from_sorting_by_term() {
        let coded = row("atorvastatin", Some("Lipitor 40 mg tablet"));
        let plain = row("aspirin", None);
        assert!(plain.term < coded.term, "by term, aspirin comes first");
        assert!(
            coded.display_name() < plain.display_name(),
            "by displayed name, Lipitor comes first — the opposite order"
        );
    }

    /// A row the read marked a hazard but gave no reason (an older builder, a test fixture) is
    /// still a hazard, and is worded as the pre-#697 case: reaching outside the set.
    #[test]
    fn a_hazard_with_no_recorded_reason_is_worded_as_reaching_outside() {
        let mut r = row("warfarin", None);
        r.cross_patient = true;
        assert!(r.is_wrong_chart_hazard());
        assert_eq!(
            r.hazard_reasons(),
            Some(WrongChartReasons {
                outside_set: true,
                doubted_link: false,
                patient_conflict: false,
            })
        );
    }

    /// The converse disagreement — a reason with the flag down — fails safe: it is a hazard.
    #[test]
    fn a_reason_without_the_flag_is_still_a_hazard() {
        let mut r = row("warfarin", None);
        r.wrong_chart.doubted_link = true;
        assert!(r.is_wrong_chart_hazard());
        assert_eq!(
            r.hazard_reasons(),
            Some(WrongChartReasons {
                outside_set: false,
                doubted_link: true,
                patient_conflict: false,
            })
        );
    }

    #[test]
    fn an_ordinary_row_is_not_a_hazard() {
        let r = row("warfarin", None);
        assert!(!r.is_wrong_chart_hazard());
        assert_eq!(r.hazard_reasons(), None);
    }

    /// `hazard_reasons` is blind to status and vouch on purpose: a CEASED line in a doubted
    /// record is still not this patient's for certain. Whether sign-off withholds it is a
    /// separate, status-aware question (`targeting::withheld_reasons`) — the PR #717 review
    /// found a note gated on one and a pointer gated on the other.
    #[test]
    fn hazard_reasons_ignores_status() {
        let mut r = row("warfarin", None);
        r.status = MedicationStatus::Ceased;
        r.wrong_chart.doubted_link = true;
        assert_eq!(
            r.hazard_reasons(),
            Some(WrongChartReasons {
                outside_set: false,
                doubted_link: true,
                patient_conflict: false,
            })
        );
    }

    /// A line reported as withheld must carry a reason a renderer can word: the renderers
    /// print one block per reason, so an empty set would drop the line from the report.
    #[test]
    fn an_empty_reason_set_is_worded_as_reaching_outside() {
        assert_eq!(
            WrongChartReasons::default().worded(),
            WrongChartReasons {
                outside_set: true,
                doubted_link: false,
                patient_conflict: false,
            }
        );
        let doubted = WrongChartReasons {
            outside_set: false,
            doubted_link: true,
            patient_conflict: false,
        };
        assert_eq!(doubted.worded(), doubted);
    }
}
