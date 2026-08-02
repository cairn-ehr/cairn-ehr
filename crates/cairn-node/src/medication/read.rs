//! Cairn's first clinical READ path (#288 med-list slice).
//!
//! Everything before this slice authored events; nothing read clinical content back out
//! in Rust. This module maps the existing medication projections into the shared
//! `cairn_medication_view` model — and it is the ONLY such mapping: the CLI verbs read
//! through it today, and the med-list UI (deferred to a later session on this branch) and
//! the future native API (ADR-0023, Phase 8) are expected to wrap this same function
//! rather than re-derive the joins.
//!
//! WHY SEVERAL SMALL QUERIES AND NOT ONE JOIN. Six query helpers issuing seven statements —
//! the current list, the past list, the per-thread vouches, three advisory flags, and the
//! membership of the groups those flags make hazardous — answer different questions over
//! different grains (group, thread, worklist, mis-reconciliation, cross-patient hazard).
//! One join would need two levels of aggregation and would be far harder for a reviewer to
//! check against the view definitions in db/031-034. Plain queries plus an explicit
//! assembly step in Rust is the reviewer-legible shape §9 asks for, and each query is
//! independently checkable against its view.
//!
//! Generic over `GenericClient` so a caller can read through an open transaction — the
//! sign-off orchestrator (`signoff.rs`) reads through its own transaction to re-check the
//! list before writing. That re-read is a best-effort compare, NOT an isolation guarantee:
//! the connection runs at READ COMMITTED, so each of these seven statements takes a fresh
//! snapshot. See `signoff.rs` and issue #335 before relying on it for atomicity.
//!
//! UUID BINDING. `tokio-postgres` has no `ToSql`/`FromSql` impl for `uuid::Uuid` without the
//! `with-uuid-1` feature, which this crate deliberately does not enable (mirrors the
//! text-cast pattern already used throughout `cairn-node`, e.g. `medication/dose.rs`,
//! `medication/attestation.rs`, `auto_apply.rs`). So every UUID parameter is bound as text
//! and cast in SQL (`$1::text::uuid`), and every UUID column is cast back to text in the
//! SELECT list and parsed on the Rust side.
use cairn_medication_view::{MedicationRow, MedicationStatus, MemberVouch, VouchState};
use serde::Serialize;
use std::collections::{BTreeMap, HashMap, HashSet};
use uuid::Uuid;

/// The one sentence that tells an operator how to clear a cross-patient group.
///
/// It is a const, not three hand-written copies, because it is quoted by three different
/// user-facing messages (the sign-off refusal, the withheld-line warning, and the chart's
/// per-row warning). A remedy that drifts between them is worse than no remedy: the
/// operator learns to distrust whichever one they read second.
///
/// WHY `medication-separate` AND WHY WITHOUT `--attest-as`. Separation is the repair
/// primitive for exactly this inconsistency and the db/033 door deliberately never blocks
/// it (unlike reconciliation, which refuses a cross-patient link at local author time). But
/// the verb takes a SINGLE `patient` argument that it stamps onto both threads' vouches
/// when `--attest-as` is given — and for a cross-patient group no single patient is right
/// for both, so attesting here would file a vouch under the wrong chart for one of them.
/// Device-additive separation carries no such claim, so that is what we tell them to run.
pub const SEPARATION_INSTRUCTION: &str =
    "Clear it with `medication-separate <patient> <thread_a> <thread_b>`, naming BOTH member \
     threads listed below — run it WITHOUT `--attest-as`, because the threads belong to \
     different patients and a vouch would record the wrong chart for one of them. Separation \
     is deliberately never blocked (db/033).";

/// A patient's chart, plus what the node knows is MISSING from it.
///
/// `rows` is what the clinician sees. `groups_missing_from_chart` is a safety signal that
/// exists BECAUSE a reconciled group can span more than one patient (issue #334): the
/// group then displays on only ONE patient's chart (see the module doc on
/// `read_cross_patient_groups`), so a patient whose thread was pulled into such a group
/// can have locally-known medication content the node simply cannot show here. Non-empty
/// means this chart is INCOMPLETE, not merely sparse — `sign_off_medication_list` refuses
/// to vouch for it for exactly that reason. Empty in normal operation.
///
/// WHAT IT DOES NOT CATCH. The signal is derived from `medication_thread_group`, which
/// db/033 drives from `medication_statement` alone. A thread known locally ONLY through an
/// orphan cessation — a stop event that arrived before the statement it stops, the
/// late-arrival case db/033 calls out — has no `medication_thread_group` row, so it
/// contributes nothing here and a group holding only such threads still escapes detection.
/// That thread is invisible to this read path with or without a group, so this is a
/// pre-existing limit of the projection rather than a gap this signal introduced; it is
/// recorded here so nobody reads `groups_missing_from_chart` as a total guarantee of
/// completeness. It is a guarantee about *displayable* content only.
///
/// `separation_targets` is what makes the other two ACTIONABLE — see its own comment.
#[derive(Debug, Clone, Serialize)]
pub struct PatientMedicationList {
    pub rows: Vec<MedicationRow>,
    pub groups_missing_from_chart: Vec<Uuid>,
    /// For each group this chart flags as a cross-patient hazard — whether it is displayed
    /// here (`MedicationRow::cross_patient`) or invisible here
    /// (`groups_missing_from_chart`) — the group's FULL member-thread list, including
    /// members belonging to OTHER patients. Sorted, and empty in normal operation.
    ///
    /// WHY THIS EXISTS (#338 review finding 1). Every message about a cross-patient group
    /// points the operator at `medication-separate`, which takes TWO THREAD IDS. Everything
    /// else this struct carries is scoped to one patient — `rows` shows only groups that
    /// display under this patient, and `read_member_vouches` filters members by
    /// `medication_thread_group.patient_id` — so the *other* patient's thread appears
    /// nowhere. Without this field the node names a remedy whose arguments it never shows,
    /// and the only way out of a sign-off refusal is raw SQL. The cross-patient member is
    /// deliberately the one piece of another chart's data this read path surfaces: it is a
    /// bare thread id with no clinical content attached, and it is the minimum needed to
    /// repair a wrong-chart link the node itself is complaining about.
    pub separation_targets: BTreeMap<Uuid, Vec<Uuid>>,
}

/// Read one patient's medication list: current drugs AND ceased ones.
///
/// Ceased rows are retained deliberately. A struck line stays visible on a paper drug
/// chart; dropping it here would lose that parity and would hide a drug the clinician may
/// need to see was recently stopped. They carry `MedicationStatus::Ceased` and are never
/// sign-off targets (`cairn_medication_view::sign_off_targets`).
pub async fn list_patient_medications(
    client: &(impl tokio_postgres::GenericClient + Sync),
    patient: Uuid,
) -> anyhow::Result<PatientMedicationList> {
    let members = read_member_vouches(client, patient).await?;
    let reconciliation_flagged = read_reconciliation_flagged_groups(client, patient).await?;
    let coding_conflict = read_coding_conflict_groups(client, patient).await?;
    let cross_patient = read_cross_patient_groups(client, patient).await?;

    // The two chart views, each mapped by the same `list_sql` builder (see its comment for
    // why one shared column list rather than two literals).
    let patient_s = patient.to_string();
    let mut rows = Vec::new();
    for (view, status) in [
        ("patient_medication_current", MedicationStatus::Active),
        ("patient_medication_past", MedicationStatus::Ceased),
    ] {
        for db_row in client.query(&list_sql(view), &[&patient_s]).await? {
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
                cross_patient: cross_patient.contains(&group_id),
            });
        }
    }

    // DEDUPLICATE by group_id, keeping the first occurrence (issue #334). A cross-patient
    // group (member threads spanning two patients) makes `medication_group_status` emit
    // TWO rows for the SAME group under the WINNING patient's id — see
    // `medication_group_cross_patient`'s view comment in db/033 for the mechanism. Without
    // this dedup, that group would print TWICE on the winner's chart: a duplicated drug
    // line is a double-dose reading hazard on an inpatient chart, not a cosmetic glitch.
    let mut seen_groups: HashSet<Uuid> = HashSet::new();
    rows.retain(|row| seen_groups.insert(row.group_id));

    // Stable display order: the name the clinician actually SEES (`display_name` — coded
    // display when coded, else the asserted term), then the group id as the tiebreak.
    // Sorting on the invisible `term` when a coded display exists would file a coded drug
    // under a string the reader never sees (e.g. "Lipitor" sorted under "atorvastatin") —
    // real cognitive-load cost against the §1.2 paper-parity benchmark. Sorted in Rust
    // rather than SQL so the order cannot depend on the database's collation (ADR-0045 —
    // a locale-dependent ORDER BY is a node-local property).
    rows.sort_by(|a, b| {
        a.display_name()
            .as_bytes()
            .cmp(b.display_name().as_bytes())
            .then_with(|| a.group_id.cmp(&b.group_id))
    });

    // Groups this patient has locally-known member threads in (per `members`, which is
    // scoped to this patient via `medication_thread_group.patient_id`) but that matched no
    // assembled row above. Sorted for determinism — the same reason the rows themselves
    // are sorted in Rust rather than left in database order.
    let mut groups_missing_from_chart: Vec<Uuid> = members
        .keys()
        .filter(|group_id| !seen_groups.contains(*group_id))
        .copied()
        .collect();
    groups_missing_from_chart.sort();

    // The membership of every group this chart calls hazardous — the arguments to the
    // `medication-separate` remedy all three warnings name. Scoped to the hazardous groups
    // rather than fetched for the whole chart: in normal operation both sets are empty and
    // this costs one query returning no rows, whereas whole-chart membership would be a
    // second O(all members) read per chart open for data nothing displays (issue #336).
    let hazardous: Vec<Uuid> = cross_patient
        .iter()
        .copied()
        .chain(groups_missing_from_chart.iter().copied())
        .collect::<HashSet<Uuid>>()
        .into_iter()
        .collect();
    let separation_targets = read_group_member_threads(client, &hazardous).await?;

    Ok(PatientMedicationList {
        rows,
        groups_missing_from_chart,
        separation_targets,
    })
}

/// Render hazardous groups as `group <id> (member threads: <a>, <b>)` — the shape EVERY
/// cross-patient message uses, written once.
///
/// The member threads are the whole point (#338 review finding 1): the remedy those
/// messages name (`medication-separate`, see [`SEPARATION_INSTRUCTION`]) takes two THREAD
/// ids, so a message printing only the group id sends the operator looking for arguments
/// the node never shows them. A group with no locally-known membership degrades honestly to
/// "unknown locally" rather than inventing a list — the same acknowledged-uncertainty
/// direction as the rest of this module.
///
/// Public because three call sites render it — the sign-off refusal, the CLI's
/// withheld-line warning, and the CLI's chart warnings. Three hand-written copies of one
/// repair instruction is how they drift.
pub fn format_hazard_groups(
    groups: &[Uuid],
    separation_targets: &BTreeMap<Uuid, Vec<Uuid>>,
) -> String {
    groups
        .iter()
        .map(|group| match separation_targets.get(group) {
            Some(members) if !members.is_empty() => {
                let rendered: Vec<String> = members.iter().map(|m| m.to_string()).collect();
                format!("group {group} (member threads: {})", rendered.join(", "))
            }
            _ => format!("group {group} (member threads: unknown locally)"),
        })
        .collect::<Vec<_>>()
        .join("; ")
}

/// The chart query for one of the two list views, written ONCE.
///
/// `view` is ALWAYS one of the two compile-time literals in `list_patient_medications`'s
/// loop — never a runtime value, so this is not a SQL-injection surface (the patient is
/// still a bind parameter; only the relation name is interpolated, and identifiers cannot
/// be bound). Sharing one builder is what keeps the two queries from drifting into reading
/// different columns from the two views — a divergence no assertion would obviously catch.
///
/// The column list is deliberately the subset `patient_medication_current` and
/// `patient_medication_past` genuinely SHARE: `_past` also carries
/// `stopped_value`/`stopped_precision`/`reason`, and each view must keep its own column set
/// stable across migrations (the db/033 replay-safety constraint on `CREATE OR REPLACE
/// VIEW` — a widened view must stay append-only, or a live node's re-replay of an earlier
/// migration fails).
fn list_sql(view: &str) -> String {
    format!(
        "SELECT medication_id::text AS medication_id, \
         patient_id::text AS patient_id, term, formulation, dose_amount, \
         dose_unit, sig, started_value, started_precision, coding_display \
         FROM {view} WHERE patient_id = $1::text::uuid"
    )
}

/// Every member thread of the given groups, regardless of which patient each member
/// belongs to — the one place this read path deliberately looks past the patient filter.
///
/// Reads `medication_group_member` directly rather than `medication_thread_group`: the
/// latter is patient-scoped by the caller everywhere else, and scoping here would return
/// exactly the half of the membership the operator already has. A singleton (never
/// reconciled) thread has no `medication_group_member` row at all, which is why this is
/// called only for groups already known to be cross-patient — those always have two or
/// more members. A group that somehow yields no rows simply gets no entry, and the callers
/// degrade to naming the group alone rather than inventing a member list.
async fn read_group_member_threads(
    client: &(impl tokio_postgres::GenericClient + Sync),
    groups: &[Uuid],
) -> anyhow::Result<BTreeMap<Uuid, Vec<Uuid>>> {
    let mut out: BTreeMap<Uuid, Vec<Uuid>> = BTreeMap::new();
    if groups.is_empty() {
        return Ok(out);
    }
    // Same text-bind-and-cast convention as every other UUID parameter here (see the
    // module's "UUID BINDING" note), lifted to an array: bind `text[]`, cast to `uuid[]`.
    let group_strs: Vec<String> = groups.iter().map(|g| g.to_string()).collect();
    let sql = "SELECT gm.group_id::text AS group_id, gm.medication_id::text AS medication_id \
               FROM medication_group_member gm \
               WHERE gm.group_id = ANY($1::text[]::uuid[]) \
               ORDER BY gm.group_id, gm.medication_id";
    for row in client.query(sql, &[&group_strs]).await? {
        let group_id: Uuid = row.get::<_, String>("group_id").parse()?;
        let medication_id: Uuid = row.get::<_, String>("medication_id").parse()?;
        out.entry(group_id).or_default().push(medication_id);
    }
    // The SQL ORDER BY is on the uuid columns; sorting again in Rust pins the order to
    // Rust's own Uuid ordering so callers (and the tests' `sorted()` expectations) cannot
    // depend on the database agreeing about uuid collation. Same reasoning as the row sort.
    for members in out.values_mut() {
        members.sort();
    }
    Ok(out)
}

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

/// Groups whose member threads span more than one patient (issue #334, db/033
/// `medication_group_cross_patient`) — a standing wrong-chart hazard. The reconcile door
/// only refuses this at LOCAL author time when BOTH patients are already known locally
/// (db/033 lines 260-279); it never refuses on the sync-apply path, so this state is
/// EXPECTED to arrive from a peer. `medication_group_display`'s DISTINCT ON always picks
/// ONE patient as the group's displayed owner, so the OTHER patient's chart shows no row
/// for this group at all — that silent absence is exactly what
/// `groups_missing_from_chart` (above) exists to catch. This query, like
/// `read_coding_conflict_groups`, is joined through `medication_thread_group` because the
/// underlying view carries no patient_id of its own.
async fn read_cross_patient_groups(
    client: &(impl tokio_postgres::GenericClient + Sync),
    patient: Uuid,
) -> anyhow::Result<HashSet<Uuid>> {
    let sql = "SELECT DISTINCT cp.group_id::text AS group_id \
               FROM medication_group_cross_patient cp \
               JOIN medication_thread_group g ON g.group_id = cp.group_id \
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

/// Pure tests for the pieces that need no database. The DB-backed behaviour of this module
/// lives in `crates/cairn-node/tests/medication_read.rs`; these pin the CLI-facing text and
/// the SQL builder, which the integration tests exercise only indirectly.
#[cfg(test)]
mod tests {
    use super::*;

    fn uid(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    /// The case the whole `separation_targets` field exists for: the rendered message must
    /// carry the THREAD ids, because those are what `medication-separate` takes.
    #[test]
    fn a_hazard_group_renders_its_member_threads() {
        let targets = BTreeMap::from([(uid(1), vec![uid(1), uid(2)])]);
        let rendered = format_hazard_groups(&[uid(1)], &targets);
        assert!(
            rendered.contains(&uid(1).to_string()) && rendered.contains(&uid(2).to_string()),
            "both member threads must appear: {rendered}"
        );
    }

    /// Acknowledged uncertainty (principle 4): a group whose membership this node cannot
    /// see says so, rather than rendering an empty list that reads as "no other threads".
    #[test]
    fn a_group_with_no_known_members_says_so() {
        let rendered = format_hazard_groups(&[uid(7)], &BTreeMap::new());
        assert!(
            rendered.contains("unknown locally"),
            "must not imply the group is a singleton: {rendered}"
        );
        assert!(rendered.contains(&uid(7).to_string()));
    }

    /// An empty membership vector is the same uncertainty as an absent key — neither may
    /// render as a confident empty list.
    #[test]
    fn an_empty_member_list_degrades_like_a_missing_one() {
        let targets = BTreeMap::from([(uid(3), vec![])]);
        assert!(format_hazard_groups(&[uid(3)], &targets).contains("unknown locally"));
    }

    #[test]
    fn several_hazard_groups_render_together() {
        let targets = BTreeMap::from([
            (uid(1), vec![uid(1), uid(2)]),
            (uid(5), vec![uid(5), uid(6)]),
        ]);
        let rendered = format_hazard_groups(&[uid(1), uid(5)], &targets);
        assert_eq!(rendered.matches("group ").count(), 2, "{rendered}");
    }

    #[test]
    fn no_hazard_groups_render_to_nothing() {
        assert_eq!(format_hazard_groups(&[], &BTreeMap::new()), "");
    }

    /// The two chart views must be read through the SAME column list — that is the whole
    /// reason `list_sql` exists rather than two literals.
    #[test]
    fn both_chart_views_are_read_with_the_same_columns() {
        let current = list_sql("patient_medication_current");
        let past = list_sql("patient_medication_past");
        assert_eq!(
            current.replace("patient_medication_current", "V"),
            past.replace("patient_medication_past", "V"),
            "the two list queries must differ ONLY in the view they read"
        );
        assert!(current.contains("WHERE patient_id = $1::text::uuid"));
    }

    /// `medication-list --json` serializes this struct WHOLE, and a `BTreeMap` keyed by
    /// `Uuid` is the one field that could fail at runtime rather than at compile time:
    /// serde_json only accepts map keys that serialize as strings. Nothing else exercises
    /// that path until an operator hits `--json` on a chart with a cross-patient group —
    /// i.e. exactly when they are least able to afford a serializer error.
    #[test]
    fn the_whole_list_serializes_to_json_including_its_uuid_keyed_map() {
        let list = PatientMedicationList {
            rows: vec![],
            groups_missing_from_chart: vec![uid(1)],
            separation_targets: BTreeMap::from([(uid(1), vec![uid(1), uid(2)])]),
        };
        let json = serde_json::to_string(&list).expect("the read model must serialize");
        assert!(json.contains(&uid(2).to_string()), "{json}");
        assert!(json.contains("separation_targets"), "{json}");
    }

    /// The repair instruction must actually name the verb and its two-thread shape — a
    /// message that says "separate it" without saying how is what finding 1 was about.
    #[test]
    fn the_separation_instruction_names_the_verb_and_both_arguments() {
        assert!(SEPARATION_INSTRUCTION.contains("medication-separate"));
        assert!(SEPARATION_INSTRUCTION.contains("thread_a"));
        assert!(SEPARATION_INSTRUCTION.contains("thread_b"));
        // Attesting a cross-patient separation would file a vouch under the wrong chart
        // for one of the two threads; the instruction must warn against it.
        assert!(SEPARATION_INSTRUCTION.contains("--attest-as"));
    }
}
