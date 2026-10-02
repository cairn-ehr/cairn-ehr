//! Cairn's first clinical READ path (#288 med-list slice), read over a chart SET (ADR-0076).
//!
//! Everything before this slice authored events; nothing read clinical content back out
//! in Rust. This module maps the existing medication projections into the shared
//! `cairn_medication_view` model — and it is the ONLY such mapping: the CLI verbs read
//! through it today, and the med-list UI and the future native API (ADR-0023, Phase 8) are
//! expected to wrap this same function rather than re-derive the joins.
//!
//! WHAT "A CHART" MEANS HERE (ADR-0076 decision 1). Opening a chart reads every chart in
//! its link component — the charts `cairn_person_charts` (db/054) says are this person —
//! as ONE list. A chart that was never linked is a set of one and gains no header, source
//! label or line from the combined read (pinned by the golden test in
//! `tests/combined_read.rs`). The one change it does see is #334's fix below: a group it
//! shares with ANOTHER person's chart now shows on it, flagged and withheld, where it used to
//! vanish.
//!
//! SELECTED BY MEMBERSHIP, NOT BY DISPLAY OWNER (the #334 fix). A medication group is on
//! this list when ANY of its member threads lives on a chart in the set. It used to be
//! selected by the `patient_id` column of `patient_medication_current`/`_past`, but that
//! column is `medication_group_display`'s single DISTINCT ON winner — so a group whose
//! threads spanned two charts displayed on the winner's chart only and silently vanished
//! from the other. Finding a group through its own members cannot lose it that way.
//!
//! WHY SEVERAL SMALL QUERIES AND NOT ONE JOIN. Up to ten statements — the chart set, the
//! per-thread vouches, each group's charts, the charts a group reaches, two advisory flags,
//! whether the set holds a doubted link (skipped for a never-linked chart), the two list
//! views (one builder), and the membership of the groups the hazard flags name — answer
//! different questions over different grains (chart set, group, thread, worklist,
//! mis-reconciliation, cross-patient hazard). One join would need two levels of
//! aggregation and would be far harder for a reviewer to check against the view
//! definitions in db/031-034 and db/054. Plain queries plus an explicit assembly step in
//! Rust is the reviewer-legible shape §9 asks for, and each query is independently
//! checkable against its view.
//!
//! Generic over `GenericClient` so a caller can read through an open transaction as well
//! as a plain client. The sign-off orchestrator (`signoff.rs`) reads the list twice to
//! re-check it before writing. That re-read is a best-effort compare, NOT an isolation
//! guarantee: the connection runs at READ COMMITTED, so each of these statements takes a
//! fresh snapshot. See `signoff.rs` and issue #335 before relying on it for atomicity.
//!
//! UUID BINDING. `tokio-postgres` has no `ToSql`/`FromSql` impl for `uuid::Uuid` without the
//! `with-uuid-1` feature, which this crate deliberately does not enable (mirrors the
//! text-cast pattern already used throughout `cairn-node`, e.g. `medication/dose.rs`,
//! `medication/attestation.rs`, `auto_apply.rs`). So every UUID parameter is bound as text
//! and cast in SQL (`$1::text::uuid`, or `$1::text[]::uuid[]` for a list), and every UUID
//! column is cast back to text in the SELECT list and parsed on the Rust side.
use cairn_medication_view::{ChartSet, MedicationRow, MedicationStatus, MemberVouch, VouchState};
use std::collections::{BTreeMap, HashMap, HashSet};
use uuid::Uuid;

// The chart model, its repair instruction and its renderer moved to the shared pure crate
// when the med-list window became their second consumer (see `cairn_medication_view::chart`
// for why). Re-exported rather than relocated in every caller: these paths are the CLI's
// and the sign-off orchestrator's, and a mechanical rename across them would have buried
// the one change that matters in this commit.
pub use cairn_medication_view::{
    format_hazard_groups, PatientMedicationList, MISSING_GROUP_INSTRUCTION, SEPARATION_INSTRUCTION,
};

/// Read the medication list of the person `patient` is a chart of: current drugs AND
/// ceased ones, over every chart linked to `patient` (ADR-0076 decision 1).
///
/// The chart set comes from `person_charts` (db/054's `cairn_person_charts`), the one
/// answer every combined read shares; a never-linked chart is a set of one. The list
/// itself is `list_chart_set_medications` over that set — see it for what the list holds.
pub async fn list_patient_medications(
    client: &(impl tokio_postgres::GenericClient + Sync),
    patient: Uuid,
) -> anyhow::Result<PatientMedicationList> {
    let charts = crate::patient::person::person_charts(client, patient).await?;
    list_chart_set_medications(client, &charts).await
}

/// Read one medication list over a chart set that `person_charts` just answered.
///
/// PRIVATE ON PURPOSE. Its hazard rule (`is_wrong_chart_hazard`) is relative to the set it is
/// handed, so re-reading a STALE set — the set a clinician was shown before an unlink — would
/// read a group spanning the two now-separate charts as "inside the set" and drop its
/// cross-patient withholding. The right response to a changed set is ADR-0076 decision 3's:
/// refuse and reload (`signoff.rs::ensure_same_charts`), never re-read the old set. The only
/// caller is `list_patient_medications`, which always reads the CURRENT set.
///
/// Ceased rows are retained deliberately. A struck line stays visible on a paper drug
/// chart; dropping it here would lose that parity and would hide a drug the clinician may
/// need to see was recently stopped. They carry `MedicationStatus::Ceased` and are never
/// sign-off targets (`cairn_medication_view::sign_off_targets`).
///
/// THE ASSEMBLY, IN ORDER:
/// 1. `members` — every thread on a chart in the set, by the group it displays under.
/// 2. `groups` — the keys of `members`. THIS is the #334 fix: a group is found through its
///    own members, never through the list views' single display-winner `patient_id`.
/// 3. the rows of the two list views for exactly those groups.
/// 4. per group: the charts its threads sit on (`source_charts`) and whether it is a
///    wrong-chart hazard (`cross_patient`, `is_wrong_chart_hazard`) — reaching another chart
///    of the same person is not a hazard, reaching someone else's is, and so is spanning a
///    link the node doubts.
/// 5. the advisory flags, each scoped to the set, and whether the set holds a doubted link
///    (an input to the hazard rule in step 4, read just before it is applied).
async fn list_chart_set_medications(
    client: &(impl tokio_postgres::GenericClient + Sync),
    charts: &ChartSet,
) -> anyhow::Result<PatientMedicationList> {
    let members = read_member_vouches(client, charts).await?;
    let groups: Vec<Uuid> = sorted_unique(members.keys().copied());

    let group_charts = read_group_charts(client, &groups).await?;
    let reached = read_cross_patient_charts(client, &groups).await?;
    let reconciliation_flagged = read_reconciliation_flagged_groups(client, charts).await?;
    let coding_conflict = read_coding_conflict_groups(client, charts).await?;
    let doubted = set_has_doubted_link(client, charts).await?;

    // A group is a wrong-chart hazard per `is_wrong_chart_hazard`, over EVERY chart it
    // touches. Two sources name those charts: `source_charts` (statement-derived,
    // `medication_thread_group`) and `medication_group_cross_patient.patients`, which ALSO
    // sees a thread known only through an orphan cessation (db/033, PR #219 finding 3) — the
    // reason the latter is read at all. The rule runs over their union: either one naming a
    // chart is enough. Over-warn, never under-warn.
    let empty: Vec<Uuid> = Vec::new();
    let cross_patient: HashSet<Uuid> = groups
        .iter()
        .copied()
        .filter(|g| {
            let touched: Vec<Uuid> = group_charts
                .get(g)
                .unwrap_or(&empty)
                .iter()
                .chain(reached.get(g).unwrap_or(&empty))
                .copied()
                .collect();
            is_wrong_chart_hazard(charts, doubted, &touched)
        })
        .collect();

    // The two chart views, each mapped by the same `list_sql` builder (see its comment for
    // why one shared column list rather than two literals), bound to the member-found groups.
    let group_strs = uuid_strings(&groups);
    let mut rows = Vec::new();
    for (view, status) in [
        ("patient_medication_current", MedicationStatus::Active),
        ("patient_medication_past", MedicationStatus::Ceased),
    ] {
        for db_row in client.query(&list_sql(view), &[&group_strs]).await? {
            let group_id: Uuid = db_row.get::<_, String>("medication_id").parse()?;
            rows.push(MedicationRow {
                group_id,
                // The view's display owner (`medication_group_display`'s winner). For a
                // group spanning charts this can be any of them — `source_charts` below is
                // the field that says where the drug was recorded.
                display_chart: db_row.get::<_, String>("patient_id").parse()?,
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
                wrong_chart: Default::default(), // filled by Task 2
                // Every group here came from `members`, which reads the same
                // `medication_thread_group` view as `read_group_charts`, so an entry exists
                // unless a concurrent separation re-keyed the group between the two
                // statements (READ COMMITTED, see the module note) — in which case the
                // group's row is normally gone too and `groups_missing_from_chart` reports it.
                source_charts: group_charts.get(&group_id).cloned().unwrap_or_default(),
            });
        }
    }

    // DEDUPLICATE by group_id, keeping the first occurrence (issue #334). A group whose
    // member threads span two charts makes `medication_group_status` emit one row per
    // (group, patient), so the list views emit the SAME group once per chart it touches —
    // see `medication_group_cross_patient`'s view comment in db/033 for the mechanism. That
    // is now true of a group wholly INSIDE the set too (two linked charts of one person), and
    // selecting by membership returns every one of those rows. Without this dedup the group
    // would print once per chart: a duplicated drug line is a double-dose reading hazard on an
    // inpatient chart, not a cosmetic glitch. The duplicate rows are identical (every column
    // is a per-group value), so which one survives does not matter.
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

    let groups_missing_from_chart = missing_groups(groups.iter().copied(), &seen_groups);

    // The membership of every group this chart calls hazardous — the arguments to the
    // `medication-separate` remedy all three warnings name. Scoped to the hazardous groups
    // rather than fetched for the whole chart: in normal operation both sets are empty and
    // this costs no query at all (`read_group_member_threads` returns early), whereas
    // whole-chart membership would be a second O(all members) read per chart open for data
    // nothing displays (issue #336).
    let hazardous: Vec<Uuid> = sorted_unique(
        cross_patient
            .iter()
            .copied()
            .chain(groups_missing_from_chart.iter().copied()),
    );
    let separation_targets = read_group_member_threads(client, &hazardous).await?;

    Ok(PatientMedicationList {
        rows,
        groups_missing_from_chart,
        separation_targets,
        charts: charts.clone(),
    })
}

/// Whether a group touching `group_charts` reaches a chart OUTSIDE `set` — the first of the
/// two hazard tests in `is_wrong_chart_hazard`.
///
/// This is the core of the ADR-0076 meaning of "cross-patient". Before the combined read, any
/// group spanning two charts was a hazard, because two charts were two people. Once linked charts
/// read as one person, a group spanning two charts OF THE SAME PERSON is an ordinary
/// reconciled drug, and only a chart outside the set can put another person's dose on this
/// line. Pure, so the rule is tested without a database (see the tests below).
fn reaches_outside(set: &ChartSet, group_charts: &[Uuid]) -> bool {
    !set.contains_all(group_charts)
}

/// Whether a group touching `group_charts` must be withheld as a wrong-chart hazard — the
/// value of `MedicationRow::cross_patient`.
///
/// Two ways a line can carry another person's dose:
/// 1. the group reaches a chart OUTSIDE the set (`reaches_outside`), or
/// 2. the set itself holds a link this node DOUBTS (`set_has_doubted_link`: an un-attested
///    link its hard veto flagged, or trips now — db/054) and the group spans more than one
///    chart.
///
/// WHY (2). ADR-0076 decision 1 combines every standing link, including an un-attested
/// synced link the local veto doubts. Treating a group across such a pair as "inside the
/// set" would turn a line that was withheld before the combined read into a signable one,
/// whose displayed dose may be the other person's. The rule does not read the link graph to
/// find WHICH pair is doubted: any multi-chart group in such a set is withheld. That
/// over-warns in a rare case, and a human resolving the link (an attested link or unlink)
/// clears it. A one-chart line is never affected — its dose is its own chart's.
///
/// NOT COVERED: a member whose own identity is in question for another reason (an open
/// dispute, a pending John Doe) does not trigger (2). Those are claims about ONE chart, not
/// about whether two charts are one person, and each member's header line shows them. Pure,
/// so the rule is tested without a database.
fn is_wrong_chart_hazard(
    set: &ChartSet,
    set_has_doubted_link: bool,
    group_charts: &[Uuid],
) -> bool {
    let spans_charts = sorted_unique(group_charts.iter().copied()).len() > 1;
    reaches_outside(set, group_charts) || (set_has_doubted_link && spans_charts)
}

/// Groups with a member thread on a chart in the set but no row on the list — sorted.
///
/// A DEFENSIVE NET, not an expected state. Before the combined read this was how #334 was
/// caught: a cross-chart group displayed only on its display-winner's chart, so the other
/// chart's thread had nowhere to show. Rows are now selected by the very groups `members`
/// found, so by construction this is empty — unless a list view drops a group it should
/// emit (a projection defect), or a concurrent reconciliation/separation re-keys a group
/// between this read's statements. Either way the list is INCOMPLETE and every renderer must
/// say so; kept rather than deleted because "cannot happen" is exactly the claim a silent
/// omission of a drug would need to be wrong about only once.
fn missing_groups(member_groups: impl Iterator<Item = Uuid>, shown: &HashSet<Uuid>) -> Vec<Uuid> {
    sorted_unique(member_groups.filter(|g| !shown.contains(g)))
}

/// Collect ids into ascending, de-duplicated order — Rust's own `Uuid` ordering, so no
/// result this module returns depends on the database agreeing about order (ADR-0045).
fn sorted_unique(ids: impl Iterator<Item = Uuid>) -> Vec<Uuid> {
    let mut v: Vec<Uuid> = ids.collect();
    v.sort();
    v.dedup();
    v
}

/// Ids as the `text[]` this module binds (see the module's "UUID BINDING" note).
fn uuid_strings(ids: &[Uuid]) -> Vec<String> {
    ids.iter().map(Uuid::to_string).collect()
}

/// Parse a `text[]` column of uuids into a sorted, de-duplicated list.
fn parse_uuid_list(texts: Vec<String>) -> anyhow::Result<Vec<Uuid>> {
    let ids: Result<Vec<Uuid>, uuid::Error> = texts.iter().map(|t| t.parse()).collect();
    Ok(sorted_unique(ids?.into_iter()))
}

/// The chart query for one of the two list views, written ONCE.
///
/// `view` is ALWAYS one of the two compile-time literals in `list_chart_set_medications`'s
/// loop — never a runtime value, so this is not a SQL-injection surface (the groups are
/// still a bind parameter; only the relation name is interpolated, and identifiers cannot
/// be bound). Sharing one builder is what keeps the two queries from drifting into reading
/// different columns from the two views — a divergence no assertion would obviously catch.
///
/// Filtered by GROUP (`medication_id` is the view's group key), never by the view's
/// `patient_id`: that column is the display winner, and filtering on it is what lost a
/// cross-chart group from every chart but one (#334). The caller binds the groups its
/// members were found in.
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
         FROM {view} WHERE medication_id = ANY($1::text[]::uuid[])"
    )
}

/// Every member thread of the given groups, regardless of which chart each member
/// belongs to — the one place this read path deliberately looks past the chart set.
///
/// Reads `medication_group_member` directly rather than `medication_thread_group`: the
/// latter is set-scoped by the caller everywhere else, and scoping here would return
/// exactly the half of the membership the operator already has. A singleton (never
/// reconciled) thread has no `medication_group_member` row at all, which is why this is
/// called only for groups already known to be hazardous — a cross-patient group always has
/// two or more members. A group that somehow yields no rows simply gets no entry, and the
/// callers degrade to naming the group alone rather than inventing a member list.
async fn read_group_member_threads(
    client: &(impl tokio_postgres::GenericClient + Sync),
    groups: &[Uuid],
) -> anyhow::Result<BTreeMap<Uuid, Vec<Uuid>>> {
    let mut out: BTreeMap<Uuid, Vec<Uuid>> = BTreeMap::new();
    if groups.is_empty() {
        return Ok(out);
    }
    let sql = "SELECT gm.group_id::text AS group_id, gm.medication_id::text AS medication_id \
               FROM medication_group_member gm \
               WHERE gm.group_id = ANY($1::text[]::uuid[]) \
               ORDER BY gm.group_id, gm.medication_id";
    for row in client.query(sql, &[&uuid_strings(groups)]).await? {
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

/// Every locally-known thread on a chart in `charts`, grouped by the row it displays under,
/// carrying the ADR-0049 vouch it holds and the chart it lives on.
///
/// This is the query that DECIDES what is on the list: its group keys are the groups the
/// list shows (see `list_chart_set_medications`). Scoped by the thread's own chart
/// (`medication_thread_group.patient_id`, from its statement), so a thread belonging to a
/// chart outside the set is never a member of this list's line — it can only appear as a
/// `separation_targets` argument.
///
/// The LEFT JOIN is what makes an unattested thread readable at all: it produces a row
/// with a NULL attester, which maps to `VouchState::Absent`. `stale` is read, never
/// recomputed — db/034 derives it from the set-commitment compare.
async fn read_member_vouches(
    client: &(impl tokio_postgres::GenericClient + Sync),
    charts: &ChartSet,
) -> anyhow::Result<HashMap<Uuid, Vec<MemberVouch>>> {
    let sql = "SELECT g.group_id::text AS group_id, g.medication_id::text AS medication_id, \
               g.patient_id::text AS patient_id, a.attester_kid, a.stale \
               FROM medication_thread_group g \
               LEFT JOIN medication_thread_attestation a ON a.medication_id = g.medication_id \
               WHERE g.patient_id = ANY($1::text[]::uuid[])";
    let mut out: HashMap<Uuid, Vec<MemberVouch>> = HashMap::new();
    for row in client
        .query(sql, &[&uuid_strings(charts.members())])
        .await?
    {
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
        out.entry(group_id).or_default().push(MemberVouch {
            medication_id: row.get::<_, String>("medication_id").parse()?,
            vouch,
            // The chart the thread's own statement is on — what a combined sign-off must
            // attest it under (ADR-0076 decision 2), never the chart the list was opened on.
            patient_id: row.get::<_, String>("patient_id").parse()?,
        });
    }
    // Member order by thread id, in Rust rather than by SQL ORDER BY (ADR-0045).
    for members in out.values_mut() {
        members.sort_by_key(|m| m.medication_id);
    }
    Ok(out)
}

/// Each group's source charts: the charts owning at least one of its member threads
/// (`medication_thread_group`, statement-derived), sorted — `MedicationRow::source_charts`.
///
/// Deliberately NOT scoped to the set: a group reaching a chart outside it must say so on
/// the row (it is exactly the row a clinician needs to trace), and the hazard test in
/// `list_chart_set_medications` reads this too.
async fn read_group_charts(
    client: &(impl tokio_postgres::GenericClient + Sync),
    groups: &[Uuid],
) -> anyhow::Result<HashMap<Uuid, Vec<Uuid>>> {
    let sql = "SELECT group_id::text AS group_id, \
               array_agg(DISTINCT patient_id::text) AS charts \
               FROM medication_thread_group \
               WHERE group_id = ANY($1::text[]::uuid[]) \
               GROUP BY group_id";
    read_group_chart_lists(client, sql, groups).await
}

/// For each of `groups` that `medication_group_cross_patient` lists (threads on more than
/// one chart), every chart its threads belong to.
///
/// Read for the charts `read_group_charts` cannot see: the view derives a thread's chart
/// through `cairn_medication_thread_patient` — the statement, else an ORPHAN CESSATION (a
/// stop event that arrived before the statement it stops, db/033 PR #219 finding 3) — so a
/// group reaching another person only through such a thread is still caught. Whether a
/// listed group is a HAZARD is not this query's call: that is `is_wrong_chart_hazard` against
/// the set, because two charts of the same person are no longer two people (ADR-0076).
async fn read_cross_patient_charts(
    client: &(impl tokio_postgres::GenericClient + Sync),
    groups: &[Uuid],
) -> anyhow::Result<HashMap<Uuid, Vec<Uuid>>> {
    let sql = "SELECT group_id::text AS group_id, patients::text[] AS charts \
               FROM medication_group_cross_patient \
               WHERE group_id = ANY($1::text[]::uuid[])";
    read_group_chart_lists(client, sql, groups).await
}

/// Run a `(group_id text, charts text[])` query bound to `groups` and parse it into
/// group → sorted, de-duplicated charts. Shared by the two group-chart readers so they
/// parse identically.
async fn read_group_chart_lists(
    client: &(impl tokio_postgres::GenericClient + Sync),
    sql: &str,
    groups: &[Uuid],
) -> anyhow::Result<HashMap<Uuid, Vec<Uuid>>> {
    let mut out = HashMap::new();
    if groups.is_empty() {
        return Ok(out);
    }
    for row in client.query(sql, &[&uuid_strings(groups)]).await? {
        let group_id: Uuid = row.get::<_, String>("group_id").parse()?;
        out.insert(group_id, parse_uuid_list(row.get("charts"))?);
    }
    Ok(out)
}

/// Groups holding an un-reconciled duplicate ACROSS the set (db/054
/// `cairn_medication_duplicate_groups`): active threads on any chart in the set sharing a
/// duplicate key while displaying under more than one group.
///
/// Scoped to the SET rather than to one chart because the same drug recorded on two linked
/// charts is two groups on two patients — the per-patient view db/033 keeps
/// (`patient_medication_reconciliation_flag`) can never see that pair, and a combined list
/// would then show one drug twice with no flag: a double-dose reading hazard. For a set of
/// one the rule is the per-patient rule unchanged (the golden pins it).
async fn read_reconciliation_flagged_groups(
    client: &(impl tokio_postgres::GenericClient + Sync),
    charts: &ChartSet,
) -> anyhow::Result<HashSet<Uuid>> {
    let sql = "SELECT g::text AS group_id \
               FROM cairn_medication_duplicate_groups($1::text[]::uuid[]) AS g";
    read_group_set(client, sql, charts).await
}

/// Groups whose members carry two different drug anchors (ADR-0059 decision 5) — a
/// possible mis-reconciliation. The view is not patient-scoped, so it is joined through
/// `medication_thread_group` to scope it to the groups with a member on a chart in the set.
async fn read_coding_conflict_groups(
    client: &(impl tokio_postgres::GenericClient + Sync),
    charts: &ChartSet,
) -> anyhow::Result<HashSet<Uuid>> {
    let sql = "SELECT DISTINCT cc.group_id::text AS group_id \
               FROM medication_group_coding_conflict cc \
               JOIN medication_thread_group g ON g.group_id = cc.group_id \
               WHERE g.patient_id = ANY($1::text[]::uuid[])";
    read_group_set(client, sql, charts).await
}

/// Whether the set holds a link this node doubts (db/054 `cairn_chart_set_has_doubted_link`:
/// an un-attested standing link that db/018 flagged on arrival, or that trips the hard veto
/// now — see that function for why both) — the second input to `is_wrong_chart_hazard`.
///
/// A set of one cannot hold a link, so it is answered without a query: a never-linked chart
/// costs exactly the statements it cost before.
async fn set_has_doubted_link(
    client: &(impl tokio_postgres::GenericClient + Sync),
    charts: &ChartSet,
) -> anyhow::Result<bool> {
    if !charts.is_linked() {
        return Ok(false);
    }
    let sql = "SELECT cairn_chart_set_has_doubted_link($1::text[]::uuid[]) AS doubted";
    let row = client
        .query_one(sql, &[&uuid_strings(charts.members())])
        .await?;
    Ok(row.get("doubted"))
}

/// Run a one-column `group_id` query bound to the set's charts and collect the ids.
async fn read_group_set(
    client: &(impl tokio_postgres::GenericClient + Sync),
    sql: &str,
    charts: &ChartSet,
) -> anyhow::Result<HashSet<Uuid>> {
    let ids: Result<HashSet<Uuid>, uuid::Error> = client
        .query(sql, &[&uuid_strings(charts.members())])
        .await?
        .iter()
        .map(|r| r.get::<_, String>("group_id").parse())
        .collect();
    Ok(ids?)
}

/// Pure tests for the SQL builder and the two set rules (`reaches_outside`,
/// `missing_groups`). The chart model's own pure tests (the hazard-group renderer, the
/// repair instruction, the `--json` serializability of the whole struct) moved with it to
/// `cairn_medication_view::chart` — a test that stays behind when its subject moves is how
/// two copies of one rule start to drift. The DB-backed behaviour of this module lives in
/// `crates/cairn-node/tests/medication_read.rs` (one chart) and
/// `crates/cairn-node/tests/combined_read.rs` (a chart set, and the never-linked golden).
#[cfg(test)]
mod tests {
    use super::*;

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
        assert!(current.contains("WHERE medication_id = ANY($1::text[]::uuid[])"));
    }

    #[test]
    fn a_group_wholly_inside_the_set_is_not_a_hazard() {
        let set = ChartSet::new([Uuid::from_u128(1), Uuid::from_u128(2)]).unwrap();
        assert!(!reaches_outside(
            &set,
            &[Uuid::from_u128(2), Uuid::from_u128(1)]
        ));
    }

    #[test]
    fn a_group_reaching_one_chart_outside_is_a_hazard() {
        let set = ChartSet::new([Uuid::from_u128(1), Uuid::from_u128(2)]).unwrap();
        assert!(reaches_outside(
            &set,
            &[Uuid::from_u128(2), Uuid::from_u128(3)]
        ));
    }

    #[test]
    fn without_a_doubted_link_the_hazard_is_reaching_outside() {
        let set = ChartSet::new([Uuid::from_u128(1), Uuid::from_u128(2)]).unwrap();
        let inside = [Uuid::from_u128(1), Uuid::from_u128(2)];
        let outside = [Uuid::from_u128(2), Uuid::from_u128(3)];
        assert!(!is_wrong_chart_hazard(&set, false, &inside));
        assert!(is_wrong_chart_hazard(&set, false, &outside));
    }

    /// A set holding a link this node doubts: any group spanning more than one chart is
    /// withheld, even wholly inside the set — the node itself doubts the pair is one person.
    /// Over-warns when the doubted pair is not the pair the group spans (the rule does not
    /// read the link graph); that is the direction this module always errs in.
    #[test]
    fn with_a_doubted_link_a_group_spanning_two_charts_is_a_hazard() {
        let set = ChartSet::new([Uuid::from_u128(1), Uuid::from_u128(2)]).unwrap();
        assert!(is_wrong_chart_hazard(
            &set,
            true,
            &[Uuid::from_u128(1), Uuid::from_u128(2)]
        ));
    }

    /// A line on ONE chart shows that chart's own dose, doubted link or not: nothing on it can
    /// belong to the other member. Duplicates in the input (the two sources overlap) must not
    /// read as two charts.
    #[test]
    fn with_a_doubted_link_a_one_chart_group_is_not_a_hazard() {
        let set = ChartSet::new([Uuid::from_u128(1), Uuid::from_u128(2)]).unwrap();
        assert!(!is_wrong_chart_hazard(&set, true, &[Uuid::from_u128(2)]));
        assert!(!is_wrong_chart_hazard(
            &set,
            true,
            &[Uuid::from_u128(2), Uuid::from_u128(2)]
        ));
        assert!(!is_wrong_chart_hazard(&set, true, &[]));
    }

    #[test]
    fn a_group_with_members_but_no_row_is_missing() {
        let shown: HashSet<Uuid> = [Uuid::from_u128(1)].into();
        assert_eq!(
            missing_groups([Uuid::from_u128(2), Uuid::from_u128(1)].into_iter(), &shown),
            vec![Uuid::from_u128(2)]
        );
    }
}
