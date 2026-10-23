//! The lines on ANOTHER chart that carry something recorded for this chart set (#848) —
//! `PatientMedicationList::elsewhere`. Read by `read.rs` after it has decided which lines it
//! shows; see `cairn_medication_view::elsewhere` for what such a line is and why it exists.
//!
//! WHERE THEY ARE FOUND. Both conflict records `read.rs`'s `read_conflict_charts` already reads,
//! seen from the other end:
//! - db/031's `medication_patient_conflict_flag`, naming a chart of the set as EITHER side. As
//!   `standing_patient` it is the chart the thread was on when a later-HLC assert moved it away
//!   (the #848 scenario); as `asserted_patient` it is this patient's claim (assert, stop,
//!   coding, W) or later event on another chart's thread.
//! - db/059's `medication_thread_stray_chart`, naming a chart of the set as `asserted_patient`:
//!   this patient's dose, correction or release on another chart's thread, whatever the order
//!   it arrived in. Its `standing_patient` is the thread's chart NOW, so a set chart there means
//!   the thread is this set's: either listed here, or claimed here by a stop, W or coding and on
//!   no list at all. Neither is elsewhere, so that side is not read.
//!
//! Every thread and line the list already shows is left out, so a line is said once: on the
//! list (its own `patient_conflict` hazard) or here, never both. And only a line SOME chart
//! lists (`medication_thread_group`, i.e. one whose assert has arrived) is said to be "on
//! another chart". A thread no chart lists — claimed only by a stop, W or coding, this
//! patient's or another chart's — is on NO list yet, and calling it "another chart's" would be
//! a precise untruth. So this patient's event on such a thread is said on NEITHER chart until
//! the thread's assert arrives (`a_thread_no_chart_lists_is_not_said_to_be_elsewhere` pins it).
//!
//! WHY THE LISTED LINES ARE CHECKED TWICE. `listed_groups` comes from the list's earlier member
//! read, a separate READ COMMITTED statement. A peer's assert that puts a line on a set chart
//! between the two reads would otherwise come back here as "on chart <this patient's own
//! chart>" and ask for a second line. So the query also drops, in its own snapshot, any line a
//! set chart lists. In that window the line shows in neither place for one read (a gap the
//! next read closes), never in a false one.
use anyhow::Context;
use cairn_medication_view::{ChartSet, ElsewhereLine, MedicationStatus};
use uuid::Uuid;

/// The lines elsewhere for `charts`, sorted by line id. `listed_threads` and `listed_groups` are
/// every thread and line the list shows (`read.rs`'s `members` and `groups`). Empty, at the cost
/// of one query, on a clean node.
///
/// The stray view is filtered by `asserted_patient`, which reaches the patient indexes db/059
/// gives its three source tables; the listed threads are dropped before the view's per-row
/// chart lookup, so the lookup runs only for this set's events on threads NOT on this list.
///
/// Public only so `tests/medication_rehomed_thread.rs` can drive it with a stale listed set (the
/// race the module doc describes); the list read is its one production caller.
pub async fn read_elsewhere(
    client: &(impl tokio_postgres::GenericClient + Sync),
    charts: &ChartSet,
    listed_threads: &[Uuid],
    listed_groups: &[Uuid],
) -> anyhow::Result<Vec<ElsewhereLine>> {
    // `name` follows `MedicationRow::display_name` (coded display, else the term). A line's
    // status comes from whichever list view emits it; `active` sorts first, should both.
    // LEFT JOIN + COALESCE: an unreconciled thread has no `medication_group_member` row and is
    // its own line (db/033), the same trap `read_conflict_charts` documents.
    let sql = "WITH hit AS ( \
                 SELECT medication_id FROM medication_patient_conflict_flag \
                  WHERE standing_patient = ANY($1::text[]::uuid[]) \
                     OR asserted_patient = ANY($1::text[]::uuid[]) \
                 UNION \
                 SELECT medication_id FROM medication_thread_stray_chart \
                  WHERE asserted_patient = ANY($1::text[]::uuid[]) \
                    AND medication_id <> ALL($2::text[]::uuid[])), \
               line AS ( \
                 SELECT DISTINCT COALESCE(gm.group_id, h.medication_id) AS group_id \
                   FROM hit h \
                   LEFT JOIN medication_group_member gm ON gm.medication_id = h.medication_id \
                  WHERE h.medication_id <> ALL($2::text[]::uuid[]) \
                    AND COALESCE(gm.group_id, h.medication_id) <> ALL($3::text[]::uuid[]) \
                    AND EXISTS (SELECT 1 FROM medication_thread_group t \
                                 WHERE t.group_id = COALESCE(gm.group_id, h.medication_id)) \
                    AND NOT EXISTS (SELECT 1 FROM medication_thread_group t \
                                     WHERE t.group_id = COALESCE(gm.group_id, h.medication_id) \
                                       AND t.patient_id = ANY($1::text[]::uuid[]))) \
               SELECT l.group_id::text AS group_id, \
                      COALESCE(v.coding_display, v.term) AS name, v.status, \
                      ARRAY(SELECT DISTINCT t.patient_id::text FROM medication_thread_group t \
                             WHERE t.group_id = l.group_id) AS charts \
                 FROM line l \
                 LEFT JOIN LATERAL ( \
                   SELECT term, coding_display, 'active' AS status \
                     FROM patient_medication_current WHERE medication_id = l.group_id \
                   UNION ALL \
                   SELECT term, coding_display, 'ceased' \
                     FROM patient_medication_past WHERE medication_id = l.group_id \
                   ORDER BY status LIMIT 1) v ON true";
    let strings = |ids: &[Uuid]| ids.iter().map(Uuid::to_string).collect::<Vec<_>>();
    let mut out = Vec::new();
    for row in client
        .query(
            sql,
            &[
                &strings(charts.members()),
                &strings(listed_threads),
                &strings(listed_groups),
            ],
        )
        .await
        .context("reading this chart set's medication lines on other charts (#848)")?
    {
        let status = match row.get::<_, Option<String>>("status").as_deref() {
            Some("active") => Some(MedicationStatus::Active),
            Some("ceased") => Some(MedicationStatus::Ceased),
            _ => None,
        };
        let mut line_charts = row
            .get::<_, Vec<String>>("charts")
            .iter()
            .map(|c| c.parse())
            .collect::<Result<Vec<Uuid>, _>>()
            .context("an elsewhere line's chart id is not a uuid")?;
        line_charts.sort();
        out.push(ElsewhereLine {
            group_id: row
                .get::<_, String>("group_id")
                .parse()
                .context("an elsewhere line's id is not a uuid")?,
            name: row.get("name"),
            status,
            charts: line_charts,
        });
    }
    // Sorted in Rust, not by SQL ORDER BY, for the same reason as every list here (ADR-0045).
    out.sort_by_key(|l| l.group_id);
    Ok(out)
}
