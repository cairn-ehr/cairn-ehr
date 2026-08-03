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
use cairn_medication_view::{sign_off_targets, MedicationStatus};
use std::collections::BTreeMap;
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
    /// signed — today, only cross-patient groups (issue #334), whose displayed dose may
    /// belong to the group's other patient. The caller MUST surface these: "signed off 11"
    /// over a chart of 12 outstanding lines is a false completeness claim, which is the
    /// same defect class as vouching for a list with a missing line. Empty in normal
    /// operation. See `cairn_medication_view::withheld_rows`.
    pub withheld: Vec<Uuid>,
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
}

/// Attest every thread on this patient's chart whose vouch is absent or stale, in one
/// transaction.
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
/// The one thing still refused is the target-set MISMATCH below, and it is a different
/// question: not "is this chart perfect?" but "is this the same chart the human reviewed?".
///
/// # Why the target set is read twice
///
/// HLCs must be minted BEFORE the transaction opens: `node_hlc_tick()` advances node state,
/// and minting inside a transaction that later aborts would roll the tick back. So the list
/// is read once outside the transaction (to size the HLC mint) and once inside it (to
/// decide what to sign), and the two computed target SETS must agree before anything is
/// written.
///
/// WHAT THIS DOES NOT GUARANTEE (issue #335). `client.transaction()` issues a plain BEGIN,
/// which runs at Postgres's default READ COMMITTED — a fresh snapshot PER STATEMENT, not
/// one snapshot for the whole transaction. `list_patient_medications` issues up to SEVEN
/// statements (the per-thread vouch read, three advisory-flag reads, the current/past list
/// reads, and the hazardous-group membership read), so even the in-transaction read alone
/// spans seven snapshots, and neither read is atomic with the other or with itself. The
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
pub async fn sign_off_medication_list(
    client: &mut tokio_postgres::Client,
    node_sk: &cairn_event::SigningKey,
    node_origin: &str,
    params: &AttestParams<'_>,
    patient: Uuid,
) -> anyhow::Result<SignOffOutcome> {
    // The node holds custody of every sealed body it writes, attestations included
    // (ADR-0052). Idempotent, and committed ahead of the transaction.
    crate::medication::sealed_submit::ensure_unwrap_key(client, node_sk).await?;

    let first_read = list_patient_medications(&*client, patient).await?;

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
        // The ONE admissible whole-gesture refusal (ADR-0060 decision 5). It does not ask
        // "is this chart perfect?" — that question is now always answered by reporting — but
        // "is this the same list the human reviewed?". Raised BEFORE any line commits, so
        // refusing here writes nothing and needs no rollback.
        //
        // Report WHAT changed, not just how many — two counts that happen to match (a
        // thread swapped for another) would otherwise read as a true but useless "3 vs 3".
        let added = format_thread_ids(actual.iter().filter(|t| !expected.contains(t)));
        let removed = format_thread_ids(expected.iter().filter(|t| !actual.contains(t)));
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
    let mut attested = Vec::with_capacity(actual.len());
    let mut event_ids = Vec::with_capacity(actual.len());
    let mut failed = Vec::new();
    for (thread, hlc) in actual.iter().zip(hlcs) {
        let tx = client.transaction().await?;
        match crate::medication::attest_thread_in_tx(&tx, params, patient, *thread, hlc).await {
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
    })
}

/// Render a set of uuids for a clinician-facing message: `"none"` when empty, otherwise a
/// comma-separated list. Pure and reusable rather than inlined at each call site, so the
/// mismatch diagnostic's two symmetric branches (added / removed) stay visibly identical
/// instead of risking silent drift between two hand-written formats.
fn format_thread_ids<'a>(ids: impl Iterator<Item = &'a Uuid>) -> String {
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
