//! The pair's STANDING `patient_link` row, the assertion that currently wins db/018's overlay, and
//! the one read of it ([`standing_link`]).
//!
//! Every human judgement path asks this read one of two questions: "did my event take effect?"
//! (db/018 ADMITS an assertion that loses the overlay, so a successful submit does not prove it),
//! or "is the pair still the one the panel showed?" (R6's `RecordsShown::pair`). The matcher's
//! auto-apply asks the first one too.
//!
//! Lives apart from `chart_link.rs` (house rule 4: files under 500 lines); re-exported by the
//! parent, so `chart_link::standing_link` and `chart_link::StandingLink` keep their paths.

use crate::db_diagnosis::{node_state_refusal, LocalDbFault};
use uuid::Uuid;

/// The pair's STANDING `patient_link` row — the assertion that currently wins the overlay
/// (db/018) — as far as a caller deciding "did my event take effect?" needs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StandingLink {
    /// The winning event's content address.
    pub content_address: Vec<u8>,
    /// `link` or `unlink`.
    pub state: String,
    /// Whether the winner is a human's vouched judgement (db/018's one definition, stored).
    pub attested: bool,
}

impl StandingLink {
    /// Is the event with this content address the one standing? The check both judgement
    /// paths make right after submitting: db/018 ADMITS an assertion that loses the
    /// overlay (it is a real event, and replicates), so "submitted" is not "took effect".
    pub fn is(&self, content_address: &[u8]) -> bool {
        self.content_address == content_address
    }
}

/// Read the standing winner for the canonical pair, inside the caller's transaction (so it
/// sees the caller's own just-submitted event). `None`: no assertion about the pair exists.
///
/// **A row with no content address is refused, never read as a judgement (#787).** A database
/// older than #115 gained `content_address` through db/018's `ADD COLUMN IF NOT EXISTS`, which
/// leaves it nullable, so such a pair's row can still carry a NULL. The address is what names
/// the winning EVENT: R6's panel signs a verdict only while it is the one shown, and auto-apply
/// asks "is it mine?". A row without one cannot answer either question truthfully, so this read
/// refuses it and names the operator act that refills the column ([`legacy_address_refusal`]).
/// The read used to `Row::get` the column, which PANICS on a NULL: the review panel stayed blank,
/// and a verdict lost its transaction mid-way.
pub async fn standing_link(
    client: &(impl tokio_postgres::GenericClient + Sync),
    low: Uuid,
    high: Uuid,
) -> anyhow::Result<Option<StandingLink>> {
    let row = client
        .query_opt(
            "SELECT content_address, state, attested FROM patient_link \
             WHERE low = $1::text::uuid AND high = $2::text::uuid",
            &[&low.to_string(), &high.to_string()],
        )
        .await
        .map_err(|e| LocalDbFault::new("reading the pair's standing link", e))?;
    let Some(r) = row else {
        return Ok(None);
    };
    let Some(content_address) = r.get::<_, Option<Vec<u8>>>(0) else {
        return Err(node_state_refusal(legacy_address_refusal(low, high)));
    };
    Ok(Some(StandingLink {
        content_address,
        state: r.get(1),
        attested: r.get(2),
    }))
}

/// The words for a standing row that predates #115 and has no content address. **Pure**, so the
/// sentence is pinned apart from the database.
///
/// THE REMEDY, LIGHTEST FIRST. A HEAL of the identity projections (`reproject --prefix
/// identity.`, no `--rebuild`) replays every link/unlink through db/018's heal-safe applier
/// without deleting anything. Replaying the winner's own event refills the address, because
/// `cairn_hlc_overlay_wins` (db/002) reads a NULL current address as the empty one, which any
/// real address beats. A full `--rebuild` is named only as the fallback: it TRUNCATEs every
/// projection table and replays the whole log in one transaction, so chart reads on the node
/// wait for it. (`--prefix identity. --rebuild` is refused outright: it would truncate
/// `patient_chart`, which registrations also feed.) Both need an owner-privileged connection:
/// `cairn_reproject` is not granted to the runtime role. `tests/standing_link_legacy_address.rs`
/// runs both commands named here.
pub fn legacy_address_refusal(low: Uuid, high: Uuid) -> String {
    format!(
        "what stands between charts {low} and {high} has no recorded content address (a row \
         written before #115), so it cannot be compared — nothing was done; an operator must \
         replay the identity projections over an owner-privileged connection: `cairn-node \
         reproject --prefix identity.` (if this refusal persists, `cairn-node reproject \
         --rebuild`, which holds chart reads until it finishes)"
    )
}
