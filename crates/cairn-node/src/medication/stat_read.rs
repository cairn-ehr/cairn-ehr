//! Charting slice 2: read stat orders back with their gives and strikes (db/082).
//!
//! One query per kind (orders, gives, strikes), joined in Rust on `medication_id` AND
//! `patient_id`: an orphan give, or a peer's row pointing at another chart, is never returned.
//! Names come from `version_settle_read::WHO` as slice 2b's read: the vouched attester else the
//! envelope signer, named only when that key is a human's, else `None` ("signer not known").
use super::version_settle_read::WHO;
use anyhow::Context;
use chrono::{DateTime, Utc};
use std::collections::HashMap;
use tokio_postgres::GenericClient;
use uuid::Uuid;

/// A given dose's "entered in error".
#[derive(Debug, Clone, PartialEq)]
pub struct Strike {
    pub who: Option<String>,
    pub at: DateTime<Utc>,
    pub why: Option<String>,
}

/// One given dose of a stat order.
#[derive(Debug, Clone, PartialEq)]
pub struct Give {
    pub event_id: Uuid,
    /// The time given (the envelope's `t_effective`).
    pub given_at: DateTime<Utc>,
    pub giver: Option<String>,
    /// The give names a stop it completes (a give that completes nothing leaves the order waiting).
    pub completes: bool,
    pub struck: Option<Strike>,
}

/// A stat order and what became of it.
#[derive(Debug, Clone, PartialEq)]
pub struct StatOrder {
    pub medication_id: Uuid,
    pub patient_id: Uuid,
    /// The order event's `t_recorded`.
    pub ordered_at: DateTime<Utc>,
    pub orderer: Option<String>,
    /// Oldest `given_at` first.
    pub gives: Vec<Give>,
}

impl StatOrder {
    /// Waiting to be given: no standing (unstruck) give that completes it.
    pub fn waiting(&self) -> bool {
        !self.gives.iter().any(|g| g.completes && g.struck.is_none())
    }
}

/// Every stat order over `charts` (`medication_charting.stat`), with its gives and their strikes.
pub async fn read_stat_orders(
    client: &(impl GenericClient + Sync),
    charts: &[Uuid],
) -> anyhow::Result<Vec<StatOrder>> {
    let charts: Vec<String> = charts.iter().map(Uuid::to_string).collect();
    let orders = client
        .query(
            &format!(
                "SELECT ch.medication_id::text, ch.patient_id::text, \
                        to_timestamp(e.hlc_wall / 1000.0), {WHO} \
                   FROM medication_charting ch \
                   JOIN medication_statement m \
                     ON m.medication_id = ch.medication_id AND m.patient_id = ch.patient_id \
                   JOIN event_log e ON e.content_address = ch.content_address \
                  WHERE ch.stat AND ch.patient_id::text = ANY($1) \
                  ORDER BY e.hlc_wall, ch.medication_id"
            ),
            &[&charts],
        )
        .await
        .context("reading the stat orders")?;
    let gives = client
        .query(
            &format!(
                "SELECT x.medication_id::text, x.patient_id::text, x.event_id::text, x.given_at, \
                        {WHO}, x.completes IS NOT NULL \
                   FROM medication_given x JOIN event_log e ON e.event_id = x.event_id \
                  WHERE x.patient_id::text = ANY($1) ORDER BY x.given_at, x.event_id"
            ),
            &[&charts],
        )
        .await
        .context("reading the stat orders' gives")?;
    let strikes = client
        .query(
            &format!(
                "SELECT x.patient_id::text, x.target::text, {WHO}, \
                        to_timestamp(e.hlc_wall / 1000.0), x.text \
                   FROM medication_given_retraction x JOIN event_log e ON e.event_id = x.event_id \
                  WHERE x.patient_id::text = ANY($1) ORDER BY x.hlc_wall, x.event_id"
            ),
            &[&charts],
        )
        .await
        .context("reading the stat orders' strikes")?;

    // (patient, given event) -> its earliest strike.
    let mut struck: HashMap<(Uuid, Uuid), Strike> = HashMap::new();
    for r in &strikes {
        let key = (
            r.try_get::<_, String>(0)?.parse()?,
            r.try_get::<_, String>(1)?.parse()?,
        );
        struck.entry(key).or_insert(Strike {
            who: r.try_get(2)?,
            at: r.try_get(3)?,
            why: r.try_get(4)?,
        });
    }
    let mut by_order: HashMap<(Uuid, Uuid), Vec<Give>> = HashMap::new();
    for r in &gives {
        let (med, patient): (Uuid, Uuid) = (
            r.try_get::<_, String>(0)?.parse()?,
            r.try_get::<_, String>(1)?.parse()?,
        );
        let event_id: Uuid = r.try_get::<_, String>(2)?.parse()?;
        by_order.entry((med, patient)).or_default().push(Give {
            event_id,
            given_at: r.try_get(3)?,
            giver: r.try_get(4)?,
            completes: r.try_get(5)?,
            struck: struck.get(&(patient, event_id)).cloned(),
        });
    }
    let mut out = Vec::with_capacity(orders.len());
    for r in &orders {
        let (medication_id, patient_id): (Uuid, Uuid) = (
            r.try_get::<_, String>(0)?.parse()?,
            r.try_get::<_, String>(1)?.parse()?,
        );
        out.push(StatOrder {
            medication_id,
            patient_id,
            ordered_at: r.try_get(2)?,
            orderer: r.try_get(3)?,
            gives: by_order
                .remove(&(medication_id, patient_id))
                .unwrap_or_default(),
        });
    }
    Ok(out)
}
