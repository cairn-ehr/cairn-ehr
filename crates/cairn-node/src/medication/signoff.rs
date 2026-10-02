//! Whole-list medication sign-off — the record-layer half of #288.
//!
//! ADR-0049 attestation is per THREAD, so vouching for a chart means authoring one
//! attestation per thread that needs one. That is N cryptographic acts, but it must be
//! ONE human act: `attest_thread_in_tx` takes an already-unsealed key by reference, so a
//! single unseal and a single review cover all N. This module is what turns that permission
//! into a callable verb — and it lives in the node, not the UI, so the CLI has the same
//! gesture and the reference UI uses no privileged path (ADR-0021).
//!
//! WHAT MAKES THE GESTURE ONE THING is the unseal and the review, NOT a shared database
//! transaction. Each attestation commits in its OWN transaction (ADR-0060): these are N
//! independent clinical acts, and a failure on one must not un-write the others. An earlier
//! version bundled all N into one transaction and had exactly that defect.
use crate::medication::{read::list_patient_medications, AttestParams};
use cairn_medication_view::{sign_off_targets, ChartSet, MedicationRow, MedicationStatus};
use std::collections::{BTreeMap, HashMap};
use uuid::Uuid;

/// One line the gesture attempted but could not complete.
///
/// A failed line is NOT a failed gesture (ADR-0060): it is excluded and reported, and every
/// other line commits regardless. `error` carries the real reason, rendered from the full
/// `anyhow` chain, because "one line failed" without saying which or why is a report the
/// operator cannot act on (ADR-0060 decision 4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailedLine {
    pub medication_id: Uuid,
    pub error: String,
}

/// What one sign-off gesture did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignOffOutcome {
    /// The thread ids that were vouched, in the order they were attested.
    pub attested: Vec<Uuid>,
    /// The attestation event ids, positionally matching `attested`.
    pub event_ids: Vec<Uuid>,
    /// How many rows the chart held at the FIRST read, before targeting narrowed that down
    /// to what actually needed a signature. Lets a caller distinguish "there is nothing on
    /// this chart at all" from "everything on this chart already carries a current
    /// signature" — both produce an empty `attested`, but they are very different clinical
    /// states (issue #331: the first has no "reviewed, nothing to record" act to log yet).
    pub total_rows: usize,
    /// How many of `total_rows` were ACTIVE (not ceased) at the first read.
    ///
    /// `total_rows` alone is not enough to keep a caller honest (#338 review finding 2). A
    /// chart holding nothing but a ceased, never-signed drug reports `total_rows == 1` and
    /// an empty `attested` — and a caller that concludes "every drug already carries a
    /// current signature" from that pair states a plain falsehood: that drug carries no
    /// signature at all, it is simply a struck line that is never re-signed. This field is
    /// what separates "nothing here CURRENTLY NEEDS a signature" from "every current drug
    /// HAS one", so only the second is ever said out loud.
    pub active_rows: usize,
    /// Displayed lines (GROUP ids) that still need a signature but were deliberately NOT
    /// signed — a group reaching a chart outside the set (issue #334) or, while the set holds
    /// a doubted link, any group not recorded only on the opened chart (#697 (b)): in both
    /// the displayed dose may belong to another patient. The caller MUST surface these: "signed off 11"
    /// over a chart of 12 outstanding lines is a false completeness claim, which is the
    /// same defect class as vouching for a list with a missing line. Empty in normal
    /// operation. Each line carries its reasons (#697), so a renderer words each with its own
    /// remedy. See `cairn_medication_view::withheld_rows`.
    pub withheld: Vec<cairn_medication_view::WithheldLine>,
    /// Each hazardous group's FULL member-thread list — the arguments to the
    /// `medication-separate` remedy the caller is told to run. Carried through verbatim
    /// from `PatientMedicationList::separation_targets`, so it is a SUPERSET of `withheld`:
    /// it also covers cross-patient groups that needed no signature and were therefore
    /// never withheld. Look up the groups you are reporting; do not iterate it as if it
    /// were the withheld set. See the `PatientMedicationList` field for why naming a group
    /// without its threads is not enough to act on (#338 review finding 1).
    pub separation_targets: BTreeMap<Uuid, Vec<Uuid>>,
    /// Groups whose locally-known content this chart could not display at all (issue
    /// #334), carried through from `PatientMedicationList::groups_missing_from_chart`.
    ///
    /// This does NOT block the gesture (#339) — see `sign_off_medication_list`. It is the
    /// other half of the bargain: sign every line you can show, and say plainly which ones
    /// you could not. The caller MUST surface this, because an empty or partial `attested`
    /// over a silently incomplete chart is exactly the false "all accounted for" claim the
    /// #334 defence exists to prevent. Union of both reads, so a group that vanished
    /// mid-gesture is reported too. Empty in normal operation.
    pub groups_missing_from_chart: Vec<Uuid>,
    /// Lines this gesture tried to sign and could not — each with the reason.
    ///
    /// A failed line never blocks another (ADR-0060): each attestation commits in its OWN
    /// transaction, so a failure here rolls back that line alone. Callers MUST surface this;
    /// the CLI additionally exits non-zero, because unlike `withheld` and
    /// `groups_missing_from_chart` — which are reported, actionable, normal-operation states
    /// — a failed line is an attempted write that errored. Empty in normal operation.
    pub failed: Vec<FailedLine>,
    /// The chart set this gesture read and signed across (ADR-0076): the opened chart plus
    /// every chart linked to it, or just the opened chart when it is linked to nothing.
    ///
    /// Each line in `attested` was recorded under the chart IN this set that its thread
    /// lives on — not necessarily the chart the gesture was opened from — so a caller
    /// reporting a combined sign-off must name the set, not only the opened chart. It is the
    /// first read's set, and a gesture that got as far as a second read was refused unless
    /// that read agreed.
    pub charts: ChartSet,
}

/// Attest every thread on this patient's medication list whose vouch is absent or stale —
/// one human gesture, one transaction PER LINE (see below).
///
/// # A combined list is signed chart by chart (ADR-0076 decisions 2 and 3)
///
/// The list is read over the opened chart's whole chart SET — every chart linked to it —
/// so one gesture can cover lines recorded on several charts. Two rules keep that gesture
/// a truthful signature:
///
/// - **Each thread is attested under the chart it lives on** (`MemberVouch::patient_id`,
///   gathered by `thread_charts`), NEVER under `patient`, the chart the list was opened
///   from. An attestation is a responsibility-bearing clinical signature recorded on a
///   chart; putting chart B's drug on chart A is a wrong-chart write, and the two charts
///   can be unlinked again tomorrow (identity is a claim, never a fact). A target whose
///   chart cannot be read is reported as a `FailedLine`, never guessed.
/// - **A changed chart set refuses the whole gesture**, and nothing is ever WRITTEN once it
///   does. The clinician vouched for the list they SAW; a link that landed while it was on
///   screen adds another chart's drugs they never reviewed, and an unlink removes lines
///   they did. `displayed` is the set that was on screen, compared against the first read —
///   that half runs before any HLC is minted. The first read is ALSO compared against the
///   second, catching a link landing between them; that half runs AFTER minting (HLCs are
///   minted once, up front, to size the mint before the transaction opens — see below), so
///   a refusal there burns the minted HLCs rather than pre-empting them. Either way nothing
///   is written: the per-line transactions that would write an attestation do not open
///   until after both compares have passed.
///
/// `displayed: None` skips ONLY the on-screen compare, and exists for the CLI verb, which
/// shows no list before signing and so has no displayed set to hold the gesture to — it
/// signs the set it finds, reported in `SignOffOutcome::charts`. `None` is NOT the default
/// for a surface that shows a list: there, passing `None` would silently sign a set the
/// clinician may never have seen, which is exactly the substitution decision 3 forbids.
/// Such a surface passes the `PatientMedicationList::charts` it rendered.
///
/// # A defect on one line never invalidates another (ADR-0060, #339)
///
/// This function does NOT refuse over an incomplete or partly-untrustworthy chart. It signs
/// every line it can show and stand behind, and REPORTS the rest — `withheld` for lines
/// present but untrustworthy (cross-patient dose bleed), `groups_missing_from_chart` for
/// content the chart could not display at all, `failed` for lines whose write errored. All
/// three must be surfaced by the caller; signing what it can must never become silence
/// about what it cannot.
///
/// The rule reaches the **transaction layer**, not just the targeting logic: each line
/// commits separately, so a rollback damages only the line that caused it.
///
/// The clinician's ruling that settled this: *"there is no reason to refuse the whole chart
/// if one single line is not visible or not trustworthy. What matters is that all visible
/// lines in the chart must be signed … or presented as unsigned in the UI."* The paper
/// counterpart is a drug written up but missing a signature — that prompts the nurse to
/// chase the signature before acting on THAT drug; it does not void the chart.
///
/// The worked case, which is why this is a safety property rather than a convenience:
/// 1 L normal saline over 4 h, signed, plus a 100 mL minibag with 10 mmol potassium, not
/// signed. The saline must still be giveable. A system that voids the chart because the
/// potassium line is unsigned — or invalid, or invisible — withholds fluid from a patient
/// over a defect in a different line. Partial orders carry weight.
///
/// The whole gesture is refused only when it is not the list the human reviewed: the
/// chart SET changed (the two ADR-0076 decision 3 compares, `ensure_same_charts`), or the
/// target set changed between the two reads (the MISMATCH below). That is a different
/// question: not "is this chart perfect?" but "is this the same chart the human reviewed?".
///
/// # Why the target set is read twice
///
/// HLCs must be minted BEFORE the transaction opens: `node_hlc_tick()` advances node state,
/// and minting inside a transaction that later aborts would roll the tick back. So the list
/// is read once before the mint (to size it) and once after (to decide what to sign) — both
/// on the client, outside any transaction, since each line then commits in its own — and the
/// two computed target SETS must agree before anything is written.
///
/// WHAT THIS DOES NOT GUARANTEE (issue #335). The connection runs at Postgres's default
/// READ COMMITTED — a fresh snapshot PER STATEMENT. `list_patient_medications` issues up to
/// TEN statements (the chart-set read, the per-thread vouch read, the two group-chart reads,
/// two advisory-flag reads, the vetoed-link read, the current/past list reads, and the
/// hazardous-group membership read — see `read.rs`), so even one read alone spans up to ten
/// snapshots, and
/// neither read is atomic with the other or with itself. The
/// `actual != expected` compare below is therefore a best-effort check, not an isolation
/// guarantee: it catches a race that happens to move the computed TARGET SET between the
/// two reads, but a narrower race, or one that leaves the target set unchanged while still
/// mutating what gets signed, could slip through undetected. Issue #335 tracks the
/// isolation-level decision (e.g. upgrading to REPEATABLE READ) and binding the compare to
/// the human's actual on-screen review window rather than just the gap between these two
/// reads.
///
/// If the two target sets disagree — a medication arrived, or someone else signed a
/// thread, in the milliseconds between — the gesture is REFUSED rather than silently
/// adjusted. That is the clinically correct answer: the clinician vouched for the list
/// they were looking at, and signing a different list on their behalf would be exactly the
/// silent substitution the "never silently refresh on screen" rule exists to prevent. The
/// caller refreshes and the clinician signs again.
///
/// `_node_sk` — DELIBERATELY UNUSED, deliberately KEPT, for the reason spelled out on
/// `attestation::attest_medication_thread`: ADR-0066 decision 6 removed its only use
/// (registering the node's unwrap key), nothing here signs with the node key, and dropping
/// it would silently stop `medication-sign-off` asking for the node's passphrase — an
/// operator-facing ceremony change that is not this refactor's to make.
pub async fn sign_off_medication_list(
    client: &mut tokio_postgres::Client,
    _node_sk: &cairn_event::SigningKey,
    node_origin: &str,
    params: &AttestParams<'_>,
    patient: Uuid,
    displayed: Option<&ChartSet>,
) -> anyhow::Result<SignOffOutcome> {
    // The node holds custody of every sealed body it writes, attestations included
    // (ADR-0052). Verified ahead of the transaction so an unprovisioned node is refused
    // before any signature is minted (ADR-0066 decision 6).
    crate::medication::sealed_submit::ensure_unwrap_key(client).await?;

    let first_read = list_patient_medications(&*client, patient).await?;

    // ADR-0076 decision 3, the on-screen half: is this the chart set the clinician saw?
    // Checked FIRST — before the empty-list early return and before any HLC is minted — so
    // a refused gesture has advanced no node state and reported nothing about a set nobody
    // was shown.
    if let Some(shown) = displayed {
        ensure_same_charts(shown, &first_read.charts, "while this list was on screen")?;
    }

    // Lines that need a signature but are not safe to sign (cross-patient dose bleed,
    // issue #334). Withheld per LINE, never per chart — see the #339 note on this
    // function: nothing wrong with one line may block another.
    let withheld = cairn_medication_view::withheld_rows(&first_read.rows);
    let active_rows = first_read
        .rows
        .iter()
        .filter(|row| row.status == MedicationStatus::Active)
        .count();

    let expected = sign_off_targets(&first_read.rows);
    if expected.is_empty() {
        // Nothing to vouch for. NOT an error: an empty, ceased-only or fully-vouched chart
        // is a legitimate state — `total_rows` and `active_rows` let the caller tell the
        // three apart (issues #331 and #338 review finding 2). An INCOMPLETE chart also
        // lands here rather than erroring (#339); `groups_missing_from_chart` is what the
        // caller must say out loud.
        return Ok(SignOffOutcome {
            attested: vec![],
            event_ids: vec![],
            total_rows: first_read.rows.len(),
            active_rows,
            withheld,
            separation_targets: first_read.separation_targets,
            groups_missing_from_chart: first_read.groups_missing_from_chart,
            failed: vec![],
            charts: first_read.charts,
        });
    }

    // One HLC per attestation, minted up front and consumed in target order (which
    // `sign_off_targets` sorts, so the assignment is deterministic). A line that later
    // fails simply burns its HLC — the counter is monotonic and nothing requires reuse.
    let mut hlcs = Vec::with_capacity(expected.len());
    for _ in 0..expected.len() {
        hlcs.push(crate::db::next_hlc(client, node_origin).await?);
    }

    // The second read, on the client directly rather than inside a transaction. Wrapping it
    // in one bought no isolation — READ COMMITTED takes a fresh snapshot per statement
    // either way (issue #335) — and it can no longer share a transaction with the writes,
    // because the writes are now per line (see below).
    //
    // SAFETY NOTE (untested, issue #333): the mismatch refusal below still has no coverage.
    // Forcing `actual != expected` needs a second connection writing a medication event for
    // this patient in the narrow window between the two reads.
    let second_read = list_patient_medications(&*client, patient).await?;

    // ADR-0076 decision 3, the between-reads half: a link or unlink landing in the gap
    // changes whose drugs are on the list. Checked before the target compare because it is
    // the more specific diagnosis — such a change usually moves the targets too, and "the
    // linked charts changed" tells the clinician WHY the list is different. Same best-effort
    // caveat as the target compare below (READ COMMITTED, issue #335), and the same #333
    // coverage gap: forcing it needs a second connection writing a link in that window.
    ensure_same_charts(
        &first_read.charts,
        &second_read.charts,
        "while it was being signed",
    )?;

    // Report the UNION of what either read found missing. A reconciliation landing in the
    // gap can pull a group off this chart WITHOUT changing the target set — if every thread
    // on the vanished group was already vouched, `actual == expected` still holds and the
    // compare below waves it through. Unioning means a group missing at EITHER moment is
    // reported: over-report incompleteness, never under-report it (ADR-0060 decision 3).
    let groups_missing_from_chart = union_sorted(
        &first_read.groups_missing_from_chart,
        &second_read.groups_missing_from_chart,
    );

    let actual = sign_off_targets(&second_read.rows);
    if actual != expected {
        // The target-set half of the whole-gesture refusal (ADR-0060 decision 5; the chart-
        // set half is `ensure_same_charts`, ADR-0076 decision 3). It does not ask
        // "is this chart perfect?" — that question is now always answered by reporting — but
        // "is this the same list the human reviewed?". Raised BEFORE any line commits, so
        // refusing here writes nothing and needs no rollback.
        //
        // Report WHAT changed, not just how many — two counts that happen to match (a
        // thread swapped for another) would otherwise read as a true but useless "3 vs 3".
        let added = format_ids(actual.iter().filter(|t| !expected.contains(t)));
        let removed = format_ids(expected.iter().filter(|t| !actual.contains(t)));
        anyhow::bail!(
            "the medication list changed while it was being signed (thread(s) added: {added}; \
             removed: {removed}); nothing was signed — refresh the list and sign again so the \
             vouch covers what was actually reviewed"
        );
    }

    // ONE TRANSACTION PER LINE (ADR-0060, transaction scope must match clinical atomicity).
    //
    // These N attestations are N INDEPENDENT clinical acts that happen to share one human
    // gesture — the gesture is one because one unseal and one review cover them all, NOT
    // because they share a database transaction. Bundling them into a single transaction
    // meant a failure on any one line un-wrote every other line's signature: the saline
    // rolled back because the potassium minibag could not be vouched. That is precisely the
    // collateral damage ADR-0060 forbids, so each line now commits on its own and a failure
    // is confined to the line that caused it.
    //
    // What is NOT split: a single clinical act that spans two threads (a reconciliation and
    // its two attestations, `reconciliation.rs`) stays atomic — you cannot half-link two
    // drugs. The unit is the clinical line, not the statement count.
    //
    // THE CHART EACH LINE IS SIGNED ON (ADR-0076 decision 2) comes from the SECOND read —
    // the same rows `actual` was computed from — so a target and its chart always describe
    // one moment. `patient` (the opened chart) is deliberately not consulted here at all.
    let charts_of = thread_charts(&second_read.rows);
    let mut attested = Vec::with_capacity(actual.len());
    let mut event_ids = Vec::with_capacity(actual.len());
    let mut failed = Vec::new();
    for (thread, hlc) in actual.iter().zip(hlcs) {
        let chart = match chart_of_thread(&charts_of, *thread) {
            Ok(chart) => chart,
            Err(line) => {
                // Unreachable by construction (every target comes from a member of these
                // same rows), and handled anyway: the only alternative to reporting it is
                // guessing a chart, and the obvious guess — the opened one — is the
                // wrong-chart write this function exists to prevent. Its HLC is burned.
                failed.push(line);
                continue;
            }
        };
        let tx = client.transaction().await?;
        match crate::medication::attest_thread_in_tx(&tx, params, chart, *thread, hlc).await {
            Ok(event_id) => {
                tx.commit().await?;
                attested.push(*thread);
                event_ids.push(event_id);
            }
            Err(e) => {
                // Roll back THIS line only. The error is kept as text rather than
                // propagated: propagating it would abort the remaining lines, which is the
                // behaviour this loop exists to remove.
                tx.rollback().await?;
                failed.push(FailedLine {
                    medication_id: *thread,
                    error: format!("{e:#}"),
                });
            }
        }
    }

    // The second read's hazard membership wins: it is the state at write time, and a group
    // that only became hazardous during the gesture must still carry its repair arguments.
    // Merged rather than replaced so a group seen only on the FIRST read (one that vanished
    // mid-gesture, and is in the union above) keeps the membership we managed to read.
    let mut separation_targets = first_read.separation_targets;
    separation_targets.extend(second_read.separation_targets);

    Ok(SignOffOutcome {
        attested,
        event_ids,
        total_rows: first_read.rows.len(),
        active_rows,
        withheld,
        separation_targets,
        groups_missing_from_chart,
        failed,
        charts: first_read.charts,
    })
}

/// Render a set of uuids (threads or charts) for a clinician-facing message: `"none"` when
/// empty, otherwise a comma-separated list. Pure and reusable rather than inlined at each
/// call site, so the mismatch diagnostics' symmetric halves (added / removed, shown / now)
/// stay visibly identical instead of risking silent drift between hand-written formats.
fn format_ids<'a>(ids: impl Iterator<Item = &'a Uuid>) -> String {
    let rendered: Vec<String> = ids.map(|id| id.to_string()).collect();
    if rendered.is_empty() {
        "none".to_string()
    } else {
        rendered.join(", ")
    }
}

/// The sorted, deduplicated union of two uuid lists.
///
/// Used for the incompleteness signal across the two reads, where the safe direction is to
/// over-report: a group missing at EITHER moment is a group the clinician was not shown,
/// and dropping it because the other read happened not to see it would put the silence back
/// that #334 exists to break.
fn union_sorted(a: &[Uuid], b: &[Uuid]) -> Vec<Uuid> {
    let mut out: Vec<Uuid> = a.iter().chain(b.iter()).copied().collect();
    out.sort();
    out.dedup();
    out
}

/// Every thread on the list → the chart that thread lives on (ADR-0076 decision 2).
///
/// Read from each row's members (`MemberVouch::patient_id`, the thread's own statement's
/// chart), NOT from `MedicationRow::display_chart`: that field is only the chart the GROUP
/// displays under — the view's display winner — which for a group spanning two linked
/// charts is one of them, and would put the other chart's thread on the wrong chart.
///
/// Pure, so the rule "a thread signs on its own chart" is tested without a database.
fn thread_charts(rows: &[MedicationRow]) -> HashMap<Uuid, Uuid> {
    rows.iter()
        .flat_map(|row| row.members.iter())
        .map(|member| (member.medication_id, member.patient_id))
        .collect()
}

/// The chart `thread` must be attested under, or the `FailedLine` explaining why it
/// cannot be. There is deliberately no fallback: a missing entry is reported, never
/// answered with the opened chart (see `sign_off_medication_list`).
fn chart_of_thread(charts_of: &HashMap<Uuid, Uuid>, thread: Uuid) -> Result<Uuid, FailedLine> {
    charts_of.get(&thread).copied().ok_or_else(|| FailedLine {
        medication_id: thread,
        error: "its chart could not be read (the thread is not a member of any line on this \
                list), so it was not signed rather than signed on a guessed chart"
            .to_string(),
    })
}

/// Refuse the gesture when the chart set moved (ADR-0076 decision 3): `Ok` exactly when
/// `before == now`, otherwise the clinician-facing refusal naming both sets.
///
/// Shared by both compares — the displayed set against the first read, and the first read
/// against the second — so the two refusals say the same thing in the same words; `window`
/// is the only part that differs ("while this list was on screen" / "while it was being
/// signed"). Sets compare by value: `ChartSet` is sorted and deduplicated on construction,
/// so two reads of one link component are equal whatever order the database returned.
fn ensure_same_charts(before: &ChartSet, now: &ChartSet, window: &str) -> anyhow::Result<()> {
    if before == now {
        return Ok(());
    }
    anyhow::bail!(
        "the linked charts changed {window} (shown: {}; now: {}); nothing was signed — \
         reload the chart and sign again",
        format_ids(before.members().iter()),
        format_ids(now.members().iter()),
    )
}

/// Pure tests for the set rules this module adds. The DB-backed behaviour of the gesture
/// lives in `crates/cairn-node/tests/medication_signoff.rs` (one chart) and
/// `crates/cairn-node/tests/combined_signoff.rs` (a chart set, ADR-0076).
#[cfg(test)]
mod tests {
    use super::*;
    use cairn_medication_view::{MemberVouch, VouchState};

    fn uid(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    /// One active, uncoded row displaying under `display_chart` with the given members,
    /// each `(thread, chart it lives on)`. Only the fields `thread_charts` reads matter.
    fn row(group: u128, display_chart: u128, members: &[(u128, u128)]) -> MedicationRow {
        MedicationRow {
            group_id: uid(group),
            display_chart: uid(display_chart),
            term: "metformin".into(),
            coding_display: None,
            formulation: None,
            dose_amount: None,
            dose_unit: None,
            sig: None,
            started_value: None,
            started_precision: None,
            status: MedicationStatus::Active,
            members: members
                .iter()
                .map(|&(thread, chart)| MemberVouch {
                    medication_id: uid(thread),
                    vouch: VouchState::Absent,
                    patient_id: uid(chart),
                })
                .collect(),
            reconciliation_flagged: false,
            coding_conflict: false,
            cross_patient: false,
            wrong_chart: Default::default(),
            source_charts: vec![],
        }
    }

    /// The decision-2 rule at its sharpest: a reconciled group spanning two linked charts
    /// displays under ONE of them (chart 1 here), yet its thread on chart 2 must sign on
    /// chart 2. Reading the row's display chart would get exactly that thread wrong.
    #[test]
    fn each_thread_maps_to_its_own_chart_not_the_display_chart() {
        let rows = [row(10, 1, &[(10, 1), (11, 2)]), row(20, 2, &[(20, 2)])];
        let map = thread_charts(&rows);
        assert_eq!(map.len(), 3);
        assert_eq!(map[&uid(10)], uid(1));
        assert_eq!(map[&uid(11)], uid(2), "the member's chart, not the row's");
        assert_eq!(map[&uid(20)], uid(2));
    }

    #[test]
    fn a_thread_with_no_chart_is_a_failed_line_never_a_guess() {
        let map = thread_charts(&[row(10, 1, &[(10, 1)])]);
        assert_eq!(chart_of_thread(&map, uid(10)), Ok(uid(1)));
        let line = chart_of_thread(&map, uid(99)).unwrap_err();
        assert_eq!(line.medication_id, uid(99));
        assert!(
            line.error.contains("its chart could not be read"),
            "{}",
            line.error
        );
    }

    #[test]
    fn an_unchanged_set_passes_whatever_order_it_was_built_in() {
        let shown = ChartSet::new([uid(2), uid(1)]).unwrap();
        let now = ChartSet::new([uid(1), uid(2), uid(1)]).unwrap();
        assert!(ensure_same_charts(&shown, &now, "while this list was on screen").is_ok());
    }

    #[test]
    fn a_changed_set_is_refused_naming_both_sets() {
        let shown = ChartSet::single(uid(1));
        let now = ChartSet::new([uid(1), uid(2)]).unwrap();
        let msg = format!(
            "{:#}",
            ensure_same_charts(&shown, &now, "while this list was on screen").unwrap_err()
        );
        assert_eq!(
            msg,
            format!(
                "the linked charts changed while this list was on screen (shown: {one}; now: \
                 {one}, {two}); nothing was signed — reload the chart and sign again",
                one = uid(1),
                two = uid(2)
            )
        );
    }
}
