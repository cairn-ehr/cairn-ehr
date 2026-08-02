//! Whole-list medication sign-off — the record-layer half of #288.
//!
//! ADR-0049 attestation is per THREAD, so vouching for a chart means authoring one
//! attestation per thread that needs one. That is N cryptographic acts, but it must be
//! ONE human act: `attest_thread_in_tx` takes an already-unsealed key by reference, so one
//! unseal and one transaction cover all N. This module is what turns that permission into
//! a callable verb — and it lives in the node, not the UI, so the CLI has the same gesture
//! and the reference UI uses no privileged path (ADR-0021).
//!
//! The bundling shape (mint the HLCs, open one transaction, attest each thread, commit) is
//! the one `reconciliation.rs` already uses for exactly two threads, generalised to N.
use crate::medication::{
    read::{format_hazard_groups, list_patient_medications, SEPARATION_INSTRUCTION},
    AttestParams,
};
use cairn_medication_view::{sign_off_targets, MedicationStatus};
use std::collections::BTreeMap;
use uuid::Uuid;

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
}

/// Attest every thread on this patient's chart whose vouch is absent or stale, in one
/// transaction.
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

    // Refuse BEFORE minting anything when the node itself knows of medication content for
    // this patient that this chart cannot display (issue #334 — a reconciled group whose
    // member threads span two patients displays on only one of them). A clinician must
    // never vouch for a list the node knows is incomplete: `sign_off_medication_list`
    // returning Ok(empty) over a chart that is silently missing a real drug would be a
    // false "everything is accounted for" claim from the vouching human. Checked on this
    // FIRST, outside-transaction read — no HLC has been minted and no transaction opened
    // yet, so refusing here costs nothing to unwind.
    //
    // THIS IS AN OPEN DESIGN QUESTION, NOT A SETTLED ONE (issue #339). Refusing the whole
    // chart argues from WHOLE-LIST semantics ("you cannot vouch for what you were never
    // shown"), while `withheld` below argues from PER-LINE drug-chart semantics for the
    // present-but-untrustworthy case — and per-line is the model #288 actually adopted, in
    // which each signature is a claim about its own thread and is not falsified by a
    // different line being invisible. The cost is real: one invisible group blocks sign-off
    // for every other drug on the chart, which is slower than the paper chart §1.2
    // benchmarks against. Pinned by `medication_read.rs`'s
    // `an_incomplete_chart_refuses_sign_off_even_for_its_unrelated_drugs`, so resolving
    // #339 the other way fails a test rather than changing behaviour unnoticed.
    if !first_read.groups_missing_from_chart.is_empty() {
        anyhow::bail!(
            "{} medication group(s) with locally-known content for this patient do not \
             appear on this chart (issue #334) — most likely a cross-patient reconciliation \
             this chart cannot yet display correctly; signing off would falsely vouch for an \
             incomplete list, so nothing was signed. {SEPARATION_INSTRUCTION} Then sign off \
             again. Affected: {}.",
            first_read.groups_missing_from_chart.len(),
            format_hazard_groups(
                &first_read.groups_missing_from_chart,
                &first_read.separation_targets
            )
        );
    }

    // Lines that need a signature but are not safe to sign (cross-patient dose bleed,
    // issue #334). Withheld per LINE, never per chart: refusing the whole gesture over one
    // suspect line would make this slower than the paper chart it is benchmarked against
    // (§1.2), where a clinician signs the lines they can vouch for. Computed on the first
    // read so the count reported matches the list the refusal check just validated.
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
        // three apart (issues #331 and #338 review finding 2).
        return Ok(SignOffOutcome {
            attested: vec![],
            event_ids: vec![],
            total_rows: first_read.rows.len(),
            active_rows,
            withheld,
            separation_targets: first_read.separation_targets,
        });
    }

    // One HLC per attestation, minted up front and consumed in target order (which
    // `sign_off_targets` sorts, so the assignment is deterministic).
    let mut hlcs = Vec::with_capacity(expected.len());
    for _ in 0..expected.len() {
        hlcs.push(crate::db::next_hlc(client, node_origin).await?);
    }

    // SAFETY NOTE (untested, issue #333): this refusal branch has no test coverage today.
    // Forcing `actual != expected` needs a second connection to write a medication event
    // (or a competing sign-off) for this patient in the narrow window between the two
    // reads above — a race that needs a test-only injection seam this crate does not have
    // yet. Issue #333 tracks building that seam; it also covers the mid-gesture-rollback
    // case `medication_signoff.rs`'s `a_refused_attestation_signs_nothing_at_all` can't
    // reach for the same reason (see that test's doc comment).
    let tx = client.transaction().await?;
    let second_read = list_patient_medications(&tx, patient).await?;

    // Re-check completeness inside the transaction, not just outside it. A reconciliation
    // landing in the gap between the two reads can pull a group off this chart WITHOUT
    // changing the target set — if every thread on the vanished group was already vouched,
    // `actual == expected` still holds and the target compare below waves it through. The
    // signal is already computed by the read; discarding it here would leave the #334
    // defence with a hole exactly the width of that race.
    if !second_read.groups_missing_from_chart.is_empty() {
        anyhow::bail!(
            "this chart became incomplete while it was being signed: {} medication group(s) \
             with locally-known content for this patient no longer appear on it (issue \
             #334); nothing was signed — refresh the list and sign again. \
             {SEPARATION_INSTRUCTION} Affected: {}.",
            second_read.groups_missing_from_chart.len(),
            format_hazard_groups(
                &second_read.groups_missing_from_chart,
                &second_read.separation_targets
            )
        );
    }

    let actual = sign_off_targets(&second_read.rows);
    if actual != expected {
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

    let mut event_ids = Vec::with_capacity(actual.len());
    for (thread, hlc) in actual.iter().zip(hlcs) {
        event_ids.push(
            crate::medication::attest_thread_in_tx(&tx, params, patient, *thread, hlc).await?,
        );
    }
    tx.commit().await?;

    Ok(SignOffOutcome {
        attested: actual,
        event_ids,
        total_rows: first_read.rows.len(),
        active_rows,
        withheld,
        separation_targets: first_read.separation_targets,
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
