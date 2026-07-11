//! §5.4 unidentified-registration ("John Doe") — the registration front door. Composes
//! primitives C4 and the demographics slices already built: mint a patient UUID, then
//! author TWO additive events through the existing 1-arg `submit_event` door —
//!   1. a **callsign name assertion** (`demographic.field.asserted`, `facets.use =
//!      "callsign"`) so the chart renders an obvious placeholder header
//!      (`Unknown-ED-<site>-<date>-<suffix>`) instead of a fake name; and
//!   2. the **C4 `identity.pending.asserted`** marking the chart *unconfirmed* (§5.4
//!      "identity-pending is an active workflow state").
//!
//! No new event types, no `db/` migration, no floor change: both events are additive and
//! flow through the door with the low ceremony of any recorded demographic assertion. The
//! callsign is a REAL name in `patient_name` (it is the display header); the advisory
//! matcher excludes it from its feature space by its `use` (see `matcher/pipeline/db.py`),
//! which is the correct layer for a matcher-feature decision (§5.2/§5.13, principle 12).
//!
//! Split: pure body assembly (unit-tested, no DB) + the async `register_john_doe`
//! orchestrator (one transaction, so the callsign name and the pending marker land
//! atomically — a chart is never half-registered).

use cairn_event::demographics::{name_assertion_body, render_name_twin};
use cairn_event::identity::{pending_assertion_body, render_pending_twin, PendingAssertion};
use cairn_event::john_doe::{callsign, CALLSIGN_USE};
use cairn_event::{sign, EventBody, Hlc, SigningKey};
use tokio_postgres::Client;
use uuid::Uuid;

/// schema_version for a demographic field assertion (mirrors `demographics_names.rs`).
const DEMOGRAPHIC_FIELD_SCHEMA_VERSION: &str = "demographic.field/1";
/// schema_version for the C4 identity-pending marker.
const PENDING_SCHEMA_VERSION: &str = "identity.pending.asserted/1";

/// The §4.1 provenance stamped on a system-generated callsign name. Value-open and
/// legible: the name was not stated by anyone — the system minted it because the patient
/// could not be identified. Distinct from a patient-stated or document-sourced name.
pub const CALLSIGN_PROVENANCE: &str = "system:john-doe-registration";

/// Number of trailing hex characters of the UUID used as the callsign's disambiguating
/// suffix. Eight hex = 32 bits of entropy: two John Does registered at the same site on the
/// same day collide on an identical callsign only if their UUID tails share all 32 bits
/// (~1 in 4.3 billion per pair). A collision is not merely cosmetic — two live unidentified
/// charts with an IDENTICAL worklist header is a wrong-chart hazard (paper-parity,
/// principle 3: paper "Unknown male bay 3" folders are physically distinct), so the suffix
/// is sized to make it negligible rather than treated as harmless. (Four hex / 16 bits
/// collided ~1 in 65 536 — enough to flake the coexistence test and, worse, to occasionally
/// print two identical bedside headers.)
const SUFFIX_HEX_LEN: usize = 8;

/// Derive the callsign's disambiguating suffix from the freshly-minted patient UUID: the
/// last `SUFFIX_HEX_LEN` hex characters of its simple form (lower-case, as the simple form
/// already is — `callsign`'s sanitizer lower-cases every part anyway, so this returns the
/// exact token that appears in the callsign). Partition-safe with ZERO coordination (the
/// UUID is already globally unique) — see design call 3. This is what keeps two John Does
/// registered at the same site on the same day distinct without a per-day counter that a
/// partition would race on.
pub fn suffix_from_uuid(patient_id: Uuid) -> String {
    let simple = patient_id.simple().to_string(); // 32 lower-hex chars, no dashes
    simple[simple.len() - SUFFIX_HEX_LEN..].to_string()
}

/// Assemble the callsign name `EventBody` (a `demographic.field.asserted` name whose
/// `use` marks it a placeholder). Pure: `event_id`, `hlc`, and the resolved `callsign`
/// string are supplied by the caller so the body is fully testable. The sole contributor
/// is the registering actor with role `recorded` (it recorded the placeholder) — additive,
/// so no attestation is demanded.
pub fn build_callsign_name_body(
    event_id: Uuid,
    patient_id: Uuid,
    callsign: &str,
    kid: &str,
    hlc: Hlc,
) -> EventBody {
    EventBody {
        event_id: event_id.to_string(),
        patient_id: patient_id.to_string(),
        event_type: "demographic.field.asserted".into(),
        schema_version: DEMOGRAPHIC_FIELD_SCHEMA_VERSION.into(),
        hlc,
        t_effective: None,
        signer_key_id: kid.into(),
        contributors: serde_json::json!([{"actor_id": kid, "role": "recorded"}]),
        payload: name_assertion_body(callsign, Some(CALLSIGN_USE), CALLSIGN_PROVENANCE),
        attachments: vec![],
        plaintext_twin: Some(render_name_twin(
            callsign,
            Some(CALLSIGN_USE),
            CALLSIGN_PROVENANCE,
        )),
    }
}

/// Assemble the C4 `identity.pending.asserted` `EventBody` marking the chart *unconfirmed*.
/// Pure. `basis` is the §4.1 value-open reason the chart is identity-pending (e.g.
/// "unconscious ED arrival, no ID"); the db/024 floor requires it non-empty.
pub fn build_pending_body(
    event_id: Uuid,
    patient_id: Uuid,
    basis: &str,
    kid: &str,
    hlc: Hlc,
) -> EventBody {
    let pid = patient_id.to_string();
    let a = PendingAssertion {
        subject: &pid,
        basis,
    };
    EventBody {
        event_id: event_id.to_string(),
        patient_id: pid.clone(), // an identity-state assertion is "about" its subject's chart
        event_type: "identity.pending.asserted".into(),
        schema_version: PENDING_SCHEMA_VERSION.into(),
        hlc,
        t_effective: None,
        signer_key_id: kid.into(),
        contributors: serde_json::json!([{"actor_id": kid, "role": "recorded"}]),
        payload: pending_assertion_body(&a),
        attachments: vec![],
        plaintext_twin: Some(render_pending_twin(&a)),
    }
}

/// Register an unidentified ("John Doe") patient: mint a UUID, derive a callsign, and
/// author the callsign name + the identity-pending marker in ONE transaction (atomic —
/// a chart is never half-registered). Returns the minted `(patient_id, callsign, node-local ordinal)`.
///
/// `class` is the care context (`ED`, `ward`, …), `site` the registering location, `date`
/// an already-formatted date string (the CLI edge owns the clock and format), and `basis`
/// the value-open reason the chart is identity-pending. Care can proceed against the
/// returned UUID immediately (§5.4 — "UUID minted immediately; care proceeds without delay").
#[allow(clippy::too_many_arguments)] // signer + node context + the four callsign/basis inputs
pub async fn register_john_doe(
    client: &mut Client,
    sk: &SigningKey,
    kid: &str,
    node_origin: &str,
    class: &str,
    site: &str,
    date: &str,
    basis: &str,
) -> anyhow::Result<(Uuid, String, i64)> {
    let patient_id = Uuid::now_v7();
    let suffix = suffix_from_uuid(patient_id);
    let call = callsign(class, site, date, &suffix);

    // Tick the HLC once per event (name @ h1 < pending @ h2 — strict order). These ticks
    // run outside the transaction below and self-commit; if the submit txn then rolls back
    // the clock has merely advanced by two with no matching events, which is fine — the HLC
    // is monotonic and gaps are allowed (the same tick-then-submit shape auto_apply uses).
    let h1 = crate::db::next_hlc(client, node_origin).await?;
    let h2 = crate::db::next_hlc(client, node_origin).await?;

    let name_body = build_callsign_name_body(Uuid::now_v7(), patient_id, &call, kid, h1);
    let pending_body = build_pending_body(Uuid::now_v7(), patient_id, basis, kid, h2);
    let name_signed = sign(&name_body, sk)?;
    let pending_signed = sign(&pending_body, sk)?;

    let tx = client.transaction().await?;
    tx.execute("SELECT submit_event($1)", &[&name_signed.signed_bytes])
        .await?;
    tx.execute("SELECT submit_event($1)", &[&pending_signed.signed_bytes])
        .await?;
    tx.commit().await?;

    // Read the node-local friendly ordinal for this registration (finisher 1). Queried
    // post-commit against the same client: the callsign row is durably present, and
    // post-commit avoids threading the read through the just-consumed transaction handle.
    // The VIEW partitions by node_origin, so this row's ordinal is its rank within THIS
    // node's John-Doe registrations — i.e. "this node's John Doe #N".
    //
    // INVARIANT (why `query_one` is exactly-one-row correct): each `register_john_doe`
    // authors EXACTLY ONE callsign name event per patient, and there is no path that
    // re-authors a callsign for an existing chart — so the VIEW holds one row per
    // patient_id. If a future feature ever re-callsigns a chart, `query_one` would see
    // two rows and error (loud, not silent — the registration itself has already
    // committed); revisit this read then (add `ORDER BY ordinal LIMIT 1` to pick the
    // registration ordinal) rather than letting a display read fail a real registration.
    let ordinal: i64 = client
        .query_one(
            "SELECT ordinal FROM john_doe_local_ordinal WHERE patient_id = $1::text::uuid",
            &[&patient_id.to_string()],
        )
        .await?
        .get("ordinal");

    Ok((patient_id, call, ordinal))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids() -> (Uuid, Uuid) {
        let eid = Uuid::parse_str("22222222-0000-0000-0000-000000000000").unwrap();
        let pid = Uuid::parse_str("00000000-0000-0000-0000-0000000000ab").unwrap();
        (eid, pid)
    }

    #[test]
    fn suffix_is_the_last_eight_uuid_hex_chars() {
        let pid = Uuid::parse_str("00000000-0000-0000-0000-0000dead00ab").unwrap();
        // simple form ends "...dead00ab" → last eight "dead00ab" (lower-case, as it appears
        // in the callsign — sanitize_part lower-cases every part, so no upper-casing here).
        // Eight hex = 32 bits: negligible same-site/same-day callsign-collision probability.
        assert_eq!(suffix_from_uuid(pid), "dead00ab");
    }

    #[test]
    fn callsign_name_body_is_a_placeholder_name_with_authored_twin() {
        let (eid, pid) = ids();
        let body = build_callsign_name_body(
            eid,
            pid,
            "Unknown-ed-s-2026-07-03-00ab",
            "kid",
            Hlc {
                wall: 5,
                counter: 0,
                node_origin: "n".into(),
            },
        );
        assert_eq!(body.event_type, "demographic.field.asserted");
        assert_eq!(body.patient_id, pid.to_string());
        assert_eq!(body.payload["field"], "name");
        assert_eq!(body.payload["value"], "Unknown-ed-s-2026-07-03-00ab");
        // The use facet is what the matcher excludes on — it MUST be present and == callsign.
        assert_eq!(body.payload["facets"]["use"], CALLSIGN_USE);
        assert_eq!(body.payload["provenance"], CALLSIGN_PROVENANCE);
        let twin = body.plaintext_twin.as_deref().unwrap();
        assert!(
            !twin.trim().is_empty(),
            "the demographic floor HARD-requires a non-empty twin"
        );
        assert!(twin.contains("Unknown-ed-s-2026-07-03-00ab"));
    }

    #[test]
    fn pending_body_marks_the_same_chart_identity_pending() {
        let (eid, pid) = ids();
        let body = build_pending_body(
            eid,
            pid,
            "unconscious ED arrival, no ID",
            "kid",
            Hlc {
                wall: 6,
                counter: 0,
                node_origin: "n".into(),
            },
        );
        assert_eq!(body.event_type, "identity.pending.asserted");
        assert_eq!(body.patient_id, pid.to_string());
        assert_eq!(body.payload["subject"], pid.to_string());
        assert_eq!(body.payload["basis"], "unconscious ED arrival, no ID");
        assert!(
            body.payload.get("method").is_none(),
            "a pending marker carries no method"
        );
        assert!(!body.plaintext_twin.as_deref().unwrap().trim().is_empty());
    }

    #[test]
    fn both_events_are_recorded_role_no_responsibility_so_no_attestation() {
        let (eid, pid) = ids();
        let hlc = Hlc {
            wall: 5,
            counter: 0,
            node_origin: "n".into(),
        };
        for body in [
            build_callsign_name_body(eid, pid, "Unknown-x", "kid", hlc.clone()),
            build_pending_body(eid, pid, "unconscious", "kid", hlc.clone()),
        ] {
            let c = &body.contributors[0];
            assert_eq!(c["role"], "recorded");
            assert!(
                c.get("responsibility").is_none(),
                "additive events demand no attestation"
            );
        }
    }
}
