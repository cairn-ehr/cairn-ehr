//! Which charts are this person (ADR-0076 decision 1) — the Rust face of db/054's
//! `cairn_person_charts`. Every combined read calls this, so every reader agrees on the set.
use std::collections::HashMap;
use tokio_postgres::GenericClient;
use uuid::Uuid;

use cairn_medication_view::ChartSet;

/// The chart set `patient` belongs to: every chart in its link component, or itself alone.
///
/// Errors only on a database failure. An empty answer cannot happen (the SQL always
/// includes `patient` itself), and is reported as an error rather than guessed around
/// should the function ever be changed to return one.
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
    ChartSet::new(charts?).ok_or_else(|| {
        anyhow::anyhow!(
            "cairn_person_charts returned no chart for {patient}; expected at least itself"
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
    pub name: Option<String>,
    pub birth_date: Option<String>,
    pub trust: String,
}

/// Read one [`ChartIdentity`] per member of `charts`, in the set's own order.
///
/// THREE SMALL QUERIES, JOINED IN RUST (the `read.rs`/`search.rs` style, not one join):
/// name, dob and trust each come from a different projection with its own absence
/// convention (a nameless chart has no `patient_name_current` row; a chart with no dob
/// asserted has no `patient_demographic` row; a `confirmed` chart has no `chart_trust`
/// row at all — the `person_chart_trust` default). Joining them in SQL would need an outer
/// join per projection to keep an absent row from dropping the whole line; three flat
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

    let names = read_names(client, &id_strs).await?;
    let dobs = read_dobs(client, &id_strs).await?;
    let trusts = read_trusts(client, &id_strs).await?;

    Ok(members
        .iter()
        .map(|id| ChartIdentity {
            patient_id: *id,
            name: names.get(id).cloned(),
            birth_date: dobs.get(id).cloned(),
            // No row in `chart_trust` means `confirmed` by that view's own construction
            // (db/024) — the same convention `patient/search.rs::read_trust_states`
            // applies for the search-candidate list.
            trust: trusts
                .get(id)
                .cloned()
                .unwrap_or_else(|| "confirmed".to_string()),
        })
        .collect())
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

/// `chart_trust.trust_state` for each id that has a row — absence means `confirmed` (the
/// `person_chart_trust` convention, db/024), applied by the caller's `unwrap_or_else`.
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
