//! `cairn-node event <id>`: does this node hold that event? (#822)
//!
//! Every write whose COMMIT got no ordinary answer is marked
//! [`crate::db_diagnosis::CommitOutcomeUnknown`], which tells the operator to check whether the
//! event was recorded before running the command again — a re-run after a commit that DID land
//! records it twice (a second chart, a second link). Before #822 the CLI had no way to make that
//! check; it took raw psql against `event_log`. The paper equivalent is "look in the chart",
//! one step, so the CLI's remedy line (`db_diagnosis::cli_remedy`) names this command.
//!
//! It reads only the clear envelope columns (type, chart, recorded time): never the body, so a
//! sealed event can be looked up without opening it, and never `safety` (which the runtime role
//! is deliberately not granted, db/049).

use crate::db_diagnosis::LocalDbFault;
use tokio_postgres::Client;
use uuid::Uuid;

/// What this node holds for one event: enough to recognise it, nothing from its body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordedEvent {
    /// The event's type, e.g. `identity.registration.asserted`.
    pub event_type: String,
    /// The chart the event is filed in (`event_log.patient_id`).
    pub chart: Uuid,
    /// When this node recorded it: the HLC's physical part (`t_recorded`), as a timestamp.
    pub recorded_at: String,
}

/// The event `id`, if this node holds it. `Ok(None)` is an answer — "not recorded here" — not a
/// failure; only a database fault is an error.
pub async fn recorded_event(client: &Client, id: Uuid) -> anyhow::Result<Option<RecordedEvent>> {
    let row = client
        .query_opt(
            "SELECT event_type, patient_id::text, to_timestamp(hlc_wall / 1000.0)::text \
             FROM event_log WHERE event_id = $1::text::uuid",
            &[&id.to_string()],
        )
        .await
        .map_err(|e| LocalDbFault::new("looking the event up in event_log", e))?;
    row.map(|r| {
        Ok(RecordedEvent {
            event_type: r.get(0),
            chart: r.get::<_, String>(1).parse()?,
            recorded_at: r.get(2),
        })
    })
    .transpose()
}

/// The operator's sentence for one lookup. **Pure.**
///
/// It says what this node holds and nothing more: an absent event is "not recorded on this node",
/// not "never written" — the operator asked about THIS node, where the interrupted command ran.
pub fn describe(id: Uuid, found: Option<&RecordedEvent>) -> String {
    match found {
        Some(ev) => format!(
            "event {id} IS recorded on this node: {}, in chart {}, recorded {}",
            ev.event_type, ev.chart, ev.recorded_at
        ),
        None => format!("event {id} is NOT recorded on this node"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_found_event_is_named_with_its_type_chart_and_time() {
        let (id, chart) = (Uuid::from_u128(1), Uuid::from_u128(2));
        let ev = RecordedEvent {
            event_type: "identity.link.asserted".into(),
            chart,
            recorded_at: "2026-10-19 08:00:00+00".into(),
        };
        let text = describe(id, Some(&ev));
        assert!(text.contains(&format!("event {id} IS recorded")), "{text}");
        assert!(text.contains("identity.link.asserted"), "{text}");
        assert!(text.contains(&chart.to_string()), "{text}");
        assert!(text.contains("2026-10-19 08:00:00+00"), "{text}");
    }

    #[test]
    fn an_absent_event_says_not_recorded_on_this_node() {
        let id = Uuid::from_u128(3);
        assert_eq!(
            describe(id, None),
            format!("event {id} is NOT recorded on this node")
        );
    }
}
