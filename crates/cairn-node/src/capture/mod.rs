//! #500 slice 2c — read one page of either plane's event set, map a row to the record the
//! medium carries, and (Task 7) run the loop that decides which events reach the medium.
//!
//! The read half is deliberately thin: `read_clinical_page` and `read_node_page` are one
//! `SELECT` apiece, and `to_medium_record` is a pure, field-by-field copy. Two things a
//! reader cannot get from the code alone:
//!
//! 1. **`dek_wrapped` never carries a plaintext key.** It is copied VERBATIM — already
//!    wrapped to THIS node's own unwrap public key, which is the key a restored node
//!    inherits (ADR-0066). There is nothing to translate here and nothing to decrypt; the
//!    mapping is a byte-for-byte copy, never an unwrap-then-rewrap.
//! 2. **The crypto-shred filter is not this module's job.** `db/051`'s
//!    `cairn_clinical_page` already selects through `event_custody_surviving` internally,
//!    so a shredded body's `dek_wrapped` arrives here already `NULL` — this code never
//!    sees the shred log and never decides anything about it. Re-deriving that filter here
//!    would be a second, driftable spelling of a safety predicate that must have exactly
//!    one home; `shred_predicate_has_one_home` enforces that this file stays a CALLER,
//!    never a second definition.
//!
//! [`capture_plane`] — the loop on top of these, and the safety-critical part of the
//! module — lives in the private `plane` submodule (`capture/plane.rs`), re-exported below
//! so the public path `capture::capture_plane` is unchanged. That file's header says why it
//! is a separate one: the two halves have different reasons to change (this one tracks
//! db/051's shape; that one tracks the capture policy). It is deliberately NOT an intra-doc
//! link — `plane` is private, and linking public docs at a private item is a hard rustdoc
//! error under CI's `RUSTDOCFLAGS=-D warnings`.

use tokio_postgres::Client;

use cairn_medium::MediumRecord;

mod plane;
pub use plane::{capture_plane, PlaneCapture, MAX_GAP_PROBES_PER_CAPTURE};

/// One row exactly as db/051's `cairn_clinical_page` hands it over — or, for the
/// federation plane, as `read_node_page`'s NULL-padded mirror of the same shape.
///
/// A thin, honest mirror of the function's `RETURNS TABLE`, deliberately NOT
/// `MediumRecord` itself: keeping the DB shape and the medium shape as two distinct types
/// means each can evolve independently, and the mapping between them (`to_medium_record`)
/// stays one small, reviewable function instead of being smeared across every call site.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClinicalRow {
    /// This plane's local sequence number for the row — the paging cursor Task 7's loop
    /// advances `after_seq` to.
    pub seq: i64,
    /// The event's COSE_Sign1 bytes, verbatim.
    pub signed_bytes: Vec<u8>,
    /// The human attestation token, when the database holds one for this row. Always
    /// `None` on the federation plane (see `read_node_page`).
    pub attestation: Option<Vec<u8>>,
    /// The attester's public key, when an attestation accompanies this row.
    pub attester_key: Option<Vec<u8>>,
    /// This event's DEK, already wrapped to this node's own unwrap public key. `None`
    /// means no custody travels: the event is unsealed, this node holds no DEK for it, or
    /// — on the clinical plane only — the body has been shredded (db/051's filter, not
    /// this module's).
    pub dek_wrapped: Option<Vec<u8>>,
}

/// Read one page of the CLINICAL plane via db/051's `cairn_clinical_page(after_seq,
/// page_limit)`. `after_seq` is an EXCLUSIVE cursor (rows with `seq > after_seq`);
/// `page_limit` bounds how many rows come back. The shred filter has already run inside
/// the database function by the time this reads the result — see the module doc.
pub async fn read_clinical_page(
    db: &Client,
    after_seq: i64,
    page_limit: i64,
) -> anyhow::Result<Vec<ClinicalRow>> {
    use anyhow::Context;
    let rows = db
        .query(
            "SELECT seq, signed_bytes, attestation, attester_key, dek_wrapped \
             FROM cairn_clinical_page($1, $2)",
            &[&after_seq, &page_limit],
        )
        .await
        .context("reading one clinical page from cairn_clinical_page")?;
    Ok(rows
        .iter()
        .map(|r| ClinicalRow {
            seq: r.get("seq"),
            signed_bytes: r.get("signed_bytes"),
            attestation: r.get("attestation"),
            attester_key: r.get("attester_key"),
            dek_wrapped: r.get("dek_wrapped"),
        })
        .collect())
}

/// Read one page of the FEDERATION plane (`node_event`), in the same [`ClinicalRow`]
/// shape `read_clinical_page` returns, so Task 7's paging loop can walk either plane with
/// one piece of code rather than two.
///
/// The three trailing `NULL`s are HONEST, not lazy. The federation plane — enroll, peer,
/// revoke, supersede — carries no human attestation token and no sealed-body custody, and
/// never will: a node-federation event is never encrypted and never requires a human
/// attester, so there is no value these three columns could hold. `NULL` says exactly
/// that ("this concept does not apply here"), which is a different fact from "a value was
/// expected but did not arrive" — the distinction `MediumRecord`'s own doc insists on
/// (`None` vs. `Some(vec![])`) applies at the source, not only at the medium.
pub async fn read_node_page(
    db: &Client,
    after_seq: i64,
    page_limit: i64,
) -> anyhow::Result<Vec<ClinicalRow>> {
    use anyhow::Context;
    let rows = db
        .query(
            "SELECT seq, signed_bytes, NULL::bytea, NULL::bytea, NULL::bytea \
             FROM node_event WHERE seq > $1 ORDER BY seq LIMIT $2",
            &[&after_seq, &page_limit],
        )
        .await
        .context("reading one federation-plane page from node_event")?;
    Ok(rows
        .iter()
        .map(|r| ClinicalRow {
            seq: r.get(0),
            signed_bytes: r.get(1),
            attestation: r.get(2),
            attester_key: r.get(3),
            dek_wrapped: r.get(4),
        })
        .collect())
}

/// Map one row to the record the medium carries. PURE — the whole DB-shape-to-medium-shape
/// decision lives in this one function, so a reviewer can hold the entire mapping in their
/// head rather than trusting that several call sites agree.
///
/// `dek_wrapped` is copied VERBATIM (see the module doc: it is already wrapped to this
/// node's unwrap key, so there is nothing to translate and no plaintext key material
/// passes through this path). `attestation` and `attester_key` are copied the same way,
/// preserving `None` (no token travelled) as distinct from `Some(vec![])` (an empty token
/// travelled) — collapsing that distinction is exactly the mistake `MediumRecord`'s own
/// doc warns against, because the clinical apply door reacts to the two differently.
pub fn to_medium_record(row: &ClinicalRow) -> MediumRecord {
    MediumRecord {
        signed_bytes: row.signed_bytes.clone(),
        attestation: row.attestation.clone(),
        attester_key: row.attester_key.clone(),
        dek_wrapped: row.dek_wrapped.clone(),
        source_seq: row.seq,
    }
}
