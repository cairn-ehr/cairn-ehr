//! Cairn's first clinical READ path (#288 med-list slice).
//!
//! Everything before this slice authored events; nothing read clinical content back out
//! in Rust. This module maps the existing medication projections into the shared
//! `cairn_medication_view` model — and it is the ONLY such mapping: the med-list UI reads
//! through it today, and the future native API (ADR-0023, Phase 8) is expected to wrap
//! this same function rather than re-derive the joins.
//!
//! WHY THREE SMALL QUERIES AND NOT ONE JOIN. The list, the per-thread vouches, and the two
//! advisory flags answer three different questions over three different grains (group,
//! thread, worklist). One join would need two levels of aggregation and would be far
//! harder for a reviewer to check against the view definitions in db/031-034. Three plain
//! queries plus an explicit assembly step in Rust is the reviewer-legible shape §9 asks
//! for, and each query is independently checkable against its view.
//!
//! Generic over `GenericClient` so a caller can read through an open transaction — the
//! sign-off orchestrator (`signoff.rs`) relies on that to compute its targets in the same
//! snapshot it writes in.
//!
//! UUID BINDING. `tokio-postgres` has no `ToSql`/`FromSql` impl for `uuid::Uuid` without the
//! `with-uuid-1` feature, which this crate deliberately does not enable (mirrors the
//! text-cast pattern already used throughout `cairn-node`, e.g. `medication/dose.rs`,
//! `medication/attestation.rs`, `auto_apply.rs`). So every UUID parameter is bound as text
//! and cast in SQL (`$1::text::uuid`), and every UUID column is cast back to text in the
//! SELECT list and parsed on the Rust side.
use cairn_medication_view::{MedicationRow, MedicationStatus, MemberVouch, VouchState};
use std::collections::{HashMap, HashSet};
use uuid::Uuid;

/// Read one patient's medication list: current drugs AND ceased ones.
///
/// Ceased rows are retained deliberately. A struck line stays visible on a paper drug
/// chart; dropping it here would lose that parity and would hide a drug the clinician may
/// need to see was recently stopped. They carry `MedicationStatus::Ceased` and are never
/// sign-off targets (`cairn_medication_view::sign_off_targets`).
pub async fn list_patient_medications(
    client: &(impl tokio_postgres::GenericClient + Sync),
    patient: Uuid,
) -> anyhow::Result<Vec<MedicationRow>> {
    let members = read_member_vouches(client, patient).await?;
    let reconciliation_flagged = read_reconciliation_flagged_groups(client, patient).await?;
    let coding_conflict = read_coding_conflict_groups(client, patient).await?;

    // `patient_medication_current` and `_past` each keep their OWN column set stable
    // across migrations (the db/033 replay-safety constraint on `CREATE OR REPLACE VIEW`
    // — a widened view must stay append-only, or a live node's re-replay of an earlier
    // migration fails). The two views are NOT identical to each other — `_past` also
    // carries `stopped_value`/`stopped_precision`/`reason` — so this SELECT list is
    // deliberately the subset the two views genuinely share, which is what lets one
    // mapper serve both.
    let patient_s = patient.to_string();
    let mut rows = Vec::new();
    for (sql, status) in [
        (CURRENT_SQL, MedicationStatus::Active),
        (PAST_SQL, MedicationStatus::Ceased),
    ] {
        for db_row in client.query(sql, &[&patient_s]).await? {
            let group_id: Uuid = db_row.get::<_, String>("medication_id").parse()?;
            rows.push(MedicationRow {
                group_id,
                patient_id: db_row.get::<_, String>("patient_id").parse()?,
                term: db_row.get("term"),
                coding_display: db_row.get("coding_display"),
                formulation: db_row.get("formulation"),
                dose_amount: db_row.get("dose_amount"),
                dose_unit: db_row.get("dose_unit"),
                sig: db_row.get("sig"),
                started_value: db_row.get("started_value"),
                started_precision: db_row.get("started_precision"),
                status,
                members: members.get(&group_id).cloned().unwrap_or_default(),
                reconciliation_flagged: reconciliation_flagged.contains(&group_id),
                coding_conflict: coding_conflict.contains(&group_id),
            });
        }
    }
    // Stable display order: coded/asserted term, then the group id as the tiebreak. Sorted
    // in Rust rather than SQL so the order cannot depend on the database's collation
    // (ADR-0045 — a locale-dependent ORDER BY is a node-local property).
    rows.sort_by(|a, b| {
        a.term
            .as_bytes()
            .cmp(b.term.as_bytes())
            .then_with(|| a.group_id.cmp(&b.group_id))
    });
    Ok(rows)
}

const CURRENT_SQL: &str = "SELECT medication_id::text AS medication_id, \
     patient_id::text AS patient_id, term, formulation, dose_amount, \
     dose_unit, sig, started_value, started_precision, coding_display \
     FROM patient_medication_current WHERE patient_id = $1::text::uuid";

const PAST_SQL: &str = "SELECT medication_id::text AS medication_id, \
     patient_id::text AS patient_id, term, formulation, dose_amount, \
     dose_unit, sig, started_value, started_precision, coding_display \
     FROM patient_medication_past WHERE patient_id = $1::text::uuid";

/// Every locally-known thread for this patient, grouped by the row it displays under,
/// carrying the ADR-0049 vouch it holds.
///
/// The LEFT JOIN is what makes an unattested thread readable at all: it produces a row
/// with a NULL attester, which maps to `VouchState::Absent`. `stale` is read, never
/// recomputed — db/034 derives it from the set-commitment compare.
async fn read_member_vouches(
    client: &(impl tokio_postgres::GenericClient + Sync),
    patient: Uuid,
) -> anyhow::Result<HashMap<Uuid, Vec<MemberVouch>>> {
    let sql = "SELECT g.group_id::text AS group_id, g.medication_id::text AS medication_id, \
               a.attester_kid, a.stale \
               FROM medication_thread_group g \
               LEFT JOIN medication_thread_attestation a ON a.medication_id = g.medication_id \
               WHERE g.patient_id = $1::text::uuid \
               ORDER BY g.group_id, g.medication_id";
    let patient_s = patient.to_string();
    let mut out: HashMap<Uuid, Vec<MemberVouch>> = HashMap::new();
    for row in client.query(sql, &[&patient_s]).await? {
        let attester: Option<String> = row.get("attester_kid");
        let stale: Option<bool> = row.get("stale");
        // Principle 4 (acknowledged uncertainty): an uncertain staleness read must never
        // be silently upgraded to a confident "signed" one — that direction is unsafe,
        // because a stale vouch rendering as fresh is a signed claim the drug was
        // reviewed when it was not. So every arm is spelled explicitly rather than
        // falling through a wildcard toward Fresh.
        let vouch = match (attester, stale) {
            (Some(by), Some(true)) => VouchState::Stale { by },
            (Some(by), Some(false)) => VouchState::Fresh { by },
            // `stale` is a boolean expression on `medication_thread_attestation`
            // (db/034) that is never NULL for a row the LEFT JOIN actually matched — this
            // arm is unreachable today. It exists so that if that invariant ever breaks
            // (a future db/034 change, or a different join shape), an attested-but-
            // unknown-staleness thread fails SAFE by reading Stale (forces re-signature)
            // rather than silently reading Fresh.
            (Some(by), None) => VouchState::Stale { by },
            (None, _) => VouchState::Absent,
        };
        let group_id: Uuid = row.get::<_, String>("group_id").parse()?;
        let medication_id: Uuid = row.get::<_, String>("medication_id").parse()?;
        out.entry(group_id).or_default().push(MemberVouch {
            medication_id,
            vouch,
        });
    }
    Ok(out)
}

/// Groups touched by an un-reconciled-duplicate flag.
///
/// `patient_medication_reconciliation_flag` reports THREAD ids spanning more than one
/// group, so every group those threads display under is flagged — that is exactly the
/// pair (or set) the clinician is being asked to look at.
async fn read_reconciliation_flagged_groups(
    client: &(impl tokio_postgres::GenericClient + Sync),
    patient: Uuid,
) -> anyhow::Result<HashSet<Uuid>> {
    let sql = "SELECT DISTINCT g.group_id::text AS group_id \
               FROM patient_medication_reconciliation_flag f \
               CROSS JOIN LATERAL unnest(f.medication_ids) AS t(medication_id) \
               JOIN medication_thread_group g ON g.medication_id = t.medication_id \
               WHERE f.patient_id = $1::text::uuid";
    let patient_s = patient.to_string();
    let ids: Result<HashSet<Uuid>, uuid::Error> = client
        .query(sql, &[&patient_s])
        .await?
        .iter()
        .map(|r| r.get::<_, String>("group_id").parse())
        .collect();
    Ok(ids?)
}

/// Groups whose members carry two different drug anchors (ADR-0059 decision 5) — a
/// possible mis-reconciliation. The view is not patient-scoped, so it is joined through
/// `medication_thread_group` to scope it to this chart.
async fn read_coding_conflict_groups(
    client: &(impl tokio_postgres::GenericClient + Sync),
    patient: Uuid,
) -> anyhow::Result<HashSet<Uuid>> {
    let sql = "SELECT DISTINCT cc.group_id::text AS group_id \
               FROM medication_group_coding_conflict cc \
               JOIN medication_thread_group g ON g.group_id = cc.group_id \
               WHERE g.patient_id = $1::text::uuid";
    let patient_s = patient.to_string();
    let ids: Result<HashSet<Uuid>, uuid::Error> = client
        .query(sql, &[&patient_s])
        .await?
        .iter()
        .map(|r| r.get::<_, String>("group_id").parse())
        .collect();
    Ok(ids?)
}
