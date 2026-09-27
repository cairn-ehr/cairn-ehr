//! Which charts are this person (ADR-0076 decision 1) — the Rust face of db/054's
//! `cairn_person_charts`. Every combined read calls this, so every reader agrees on the set.
use std::collections::{HashMap, HashSet};
use tokio_postgres::GenericClient;
use uuid::Uuid;

use cairn_medication_view::ChartSet;

/// The chart set `patient` belongs to: every chart in its link component, or itself alone.
///
/// Errors on a database failure, and on an answer that does not contain `patient` itself.
/// That cannot happen today (the SQL always unions `patient` in), and is refused rather
/// than guessed around should the function ever change: a set missing the opened chart
/// would silently leave that chart's own drugs off its list.
pub async fn person_charts(
    client: &(impl tokio_postgres::GenericClient + Sync),
    patient: Uuid,
) -> anyhow::Result<ChartSet> {
    let rows = client
        .query(
            "SELECT c::text AS chart FROM cairn_person_charts($1::text::uuid) AS c",
            &[&patient.to_string()],
        )
        .await?;
    let charts: Result<Vec<Uuid>, uuid::Error> = rows
        .iter()
        .map(|r| r.get::<_, String>("chart").parse())
        .collect();
    ChartSet::new(charts?)
        .filter(|set| set.contains(&patient))
        .ok_or_else(|| {
            anyhow::anyhow!(
                "cairn_person_charts did not return {patient} itself; expected a set containing it"
            )
        })
}

/// One member chart's own identity line for the combined-read header: its display name,
/// dob and trust state, read independently of every other member.
///
/// ADR-0076 decision 1 is explicit that a linked chart set shows EACH member's own line,
/// never a winner picked across them — two duplicates are so often linked BECAUSE their
/// names or dates differ by exactly the typo that created the duplicate ("SMITH" vs
/// "SMYTHE", or a day/month transposition on the same dob) that collapsing them into one
/// displayed identity would erase the very evidence a clinician needs to tell the charts
/// apart, or to decide the link itself was wrong. So this type carries no merged/preferred
/// field at all — only per-chart facts, `Option`-typed wherever principle 4 says absence
/// must read as `None` rather than a fabricated or borrowed value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChartIdentity {
    pub patient_id: Uuid,
    /// Whether this node holds anything about the chart at all (a `patient_chart` row). A
    /// link can name a chart that has not reached this node — it synced ahead of that
    /// chart's registration, or the chart lies outside this node's sync scope — and then
    /// `name` and `birth_date` are absent because nothing ARRIVED, not because nothing was
    /// recorded. Renderers must say which.
    pub held: bool,
    pub name: Option<String>,
    pub birth_date: Option<String>,
    /// A `chart_trust` state, or `"unknown"` for a chart this node does not hold — see
    /// [`trust_of`].
    pub trust: String,
}

/// The trust state a member line shows, from whether the chart is held here and its
/// `chart_trust` row (if any).
///
/// A `chart_trust` row (db/024) is a positive claim — an open dispute, a pending identity,
/// a vetoed link — and is shown whenever there is one, held chart or not: a vetoed link to a
/// chart this node has never received still makes that chart `under-review`.
///
/// NO row means `confirmed` by db/023's `person_chart_trust` convention (it LEFT JOINs
/// `chart_trust` onto `patient_chart` and coalesces the gap) — the same convention
/// `patient/search.rs::read_trust_states` applies. But that default is true only of a chart
/// that EXISTS here, which is exactly why `person_chart_trust` starts from `patient_chart`.
/// For a chart this node does not hold, "no row" means nothing arrived, and claiming
/// `confirmed` would be a precise untruth (principle 4): it reads `unknown`. Pure, so the
/// rule is tested without a database.
pub fn trust_of(held: bool, row: Option<&str>) -> String {
    match (held, row) {
        (_, Some(state)) => state.to_string(),
        (true, None) => "confirmed".to_string(),
        (false, None) => "unknown".to_string(),
    }
}

/// Read one [`ChartIdentity`] per member of `charts`, in the set's own order.
///
/// FOUR SMALL QUERIES, JOINED IN RUST (the `read.rs`/`search.rs` style, not one join):
/// name, dob and trust each come from a different projection with its own absence
/// convention (a nameless chart has no `patient_name_current` row; a chart with no dob
/// asserted has no `patient_demographic` row; a `confirmed` chart has no `chart_trust`
/// row at all — the `person_chart_trust` default, db/023). A fourth, `patient_chart`, says
/// whether the chart is held here at all (see [`trust_of`]). Joining them in SQL would need
/// an outer join per projection to keep an absent row from dropping the whole line; four flat
/// `WHERE id = ANY(...)` reads plus a `HashMap` lookup in Rust is the shape a reviewer can
/// check against each view's own definition without untangling a multi-way outer join.
///
/// Generic over `GenericClient` so a caller can read inside an open transaction, matching
/// every other combined-read query in this crate.
pub async fn chart_identities<C: GenericClient + Sync>(
    client: &C,
    charts: &ChartSet,
) -> anyhow::Result<Vec<ChartIdentity>> {
    let members = charts.members();
    let id_strs: Vec<String> = members.iter().map(Uuid::to_string).collect();

    let held = read_held(client, &id_strs).await?;
    let names = read_names(client, &id_strs).await?;
    let dobs = read_dobs(client, &id_strs).await?;
    let trusts = read_trusts(client, &id_strs).await?;

    Ok(members
        .iter()
        .map(|id| ChartIdentity {
            patient_id: *id,
            held: held.contains(id),
            name: names.get(id).cloned(),
            birth_date: dobs.get(id).cloned(),
            trust: trust_of(held.contains(id), trusts.get(id).map(String::as_str)),
        })
        .collect())
}

/// The ids this node holds anything about: a `patient_chart` row, which the projection
/// dispatch creates for the first event about a chart of any type. The same relation
/// `person_chart_trust` is built from, which is what makes its `confirmed` default honest.
async fn read_held<C: GenericClient + Sync>(
    client: &C,
    id_strs: &[String],
) -> anyhow::Result<HashSet<Uuid>> {
    let sql = "SELECT patient_id::text AS patient_id FROM patient_chart \
               WHERE patient_id = ANY($1::text[]::uuid[])";
    let mut out = HashSet::new();
    for row in client.query(sql, &[&id_strs]).await? {
        out.insert(row.get::<_, String>("patient_id").parse()?);
    }
    Ok(out)
}

/// `patient_name_current.value` for each id that has one — the same display-winner view
/// `patient/search.rs::read_display_names` reads, but never falling back to a placeholder
/// here: a linked chart's own header line reports a genuine absence as `None`, it does not
/// invent "(name unavailable)" text the way a search-results row does.
async fn read_names<C: GenericClient + Sync>(
    client: &C,
    id_strs: &[String],
) -> anyhow::Result<HashMap<Uuid, String>> {
    let sql = "SELECT patient_id::text AS patient_id, value \
               FROM patient_name_current \
               WHERE patient_id = ANY($1::text[]::uuid[])";
    let mut out = HashMap::new();
    for row in client.query(sql, &[&id_strs]).await? {
        let id: Uuid = row.get::<_, String>("patient_id").parse()?;
        out.insert(id, row.get::<_, String>("value"));
    }
    Ok(out)
}

/// `patient_demographic.value` for `field = 'dob'`, carried VERBATIM — never reformatted.
/// The whole point of showing each member's own date rather than a merged one is that the
/// difference between two members' dates (a day/month transposition, say) is often the
/// very typo that created the duplicate; reformatting it here could paper over exactly
/// that difference.
async fn read_dobs<C: GenericClient + Sync>(
    client: &C,
    id_strs: &[String],
) -> anyhow::Result<HashMap<Uuid, String>> {
    let sql = "SELECT patient_id::text AS patient_id, value \
               FROM patient_demographic \
               WHERE field = 'dob' AND patient_id = ANY($1::text[]::uuid[])";
    let mut out = HashMap::new();
    for row in client.query(sql, &[&id_strs]).await? {
        let id: Uuid = row.get::<_, String>("patient_id").parse()?;
        out.insert(id, row.get::<_, String>("value"));
    }
    Ok(out)
}

/// `chart_trust.trust_state` for each id that has a row — what absence means is
/// [`trust_of`]'s call, because it depends on whether the chart is held here at all.
async fn read_trusts<C: GenericClient + Sync>(
    client: &C,
    id_strs: &[String],
) -> anyhow::Result<HashMap<Uuid, String>> {
    let sql = "SELECT patient_id::text AS patient_id, trust_state \
               FROM chart_trust \
               WHERE patient_id = ANY($1::text[]::uuid[])";
    let mut out = HashMap::new();
    for row in client.query(sql, &[&id_strs]).await? {
        let id: Uuid = row.get::<_, String>("patient_id").parse()?;
        out.insert(id, row.get::<_, String>("trust_state"));
    }
    Ok(out)
}

/// Pure tests for the member trust rule. The DB-backed behaviour (the four reads, and a link
/// to a chart this node does not hold) lives in `crates/cairn-node/tests/person_charts.rs`.
#[cfg(test)]
mod tests {
    use super::trust_of;

    #[test]
    fn a_held_chart_with_no_trust_row_is_confirmed() {
        assert_eq!(trust_of(true, None), "confirmed");
    }

    #[test]
    fn a_trust_row_is_shown_held_or_not() {
        assert_eq!(trust_of(true, Some("under-review")), "under-review");
        assert_eq!(trust_of(false, Some("under-review")), "under-review");
    }

    /// Principle 4: "no row" about a chart that never arrived is not evidence of anything.
    #[test]
    fn a_chart_not_held_with_no_trust_row_is_unknown_never_confirmed() {
        assert_eq!(trust_of(false, None), "unknown");
    }
}
