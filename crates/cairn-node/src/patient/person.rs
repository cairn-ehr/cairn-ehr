//! Which charts are this person (ADR-0076 decision 1) — the Rust face of db/054's
//! `cairn_person_charts`. Every combined read calls this, so every reader agrees on the set.
use cairn_medication_view::ChartSet;
use uuid::Uuid;

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
