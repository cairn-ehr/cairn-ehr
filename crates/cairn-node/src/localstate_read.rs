//! ADR-0066 / #495 — the producer that fills the sealed local-state export.
//!
//! WHY A SEPARATE FILE: `localstate.rs` owns the FORMAT (container, seal, slots) and is
//! already past the project's 500-line file-size GUIDELINE — a guideline, not a cap; the
//! repo does not enforce a file-length limit, and `tests/patient_register_demographics.rs`
//! records the correction to the "house limit" phrasing. This file owns the one thing needing a
//! database — reading custody out of it — so neither grows the other. The public name
//! stays `localstate::read_local_state` (re-exported there), so no call site moved.
//!
//! WHAT IT MUST NEVER DO: export a shredded event's DEK. ADR-0026 point 6 requires a
//! restore to honour an erasure the node already executed, and this export is the artifact
//! that crosses the restore boundary — ADR-0066 decision 7 puts it plainly: a key that
//! never crosses cannot be resurrected by a restore, which is stronger than replaying the
//! shred log afterwards and does not depend on replay ordering.
//!
//! Be honest about how much of that the filter below carries. It is a LAST LINE, not the
//! only one: `cairn_execute_shred` (db/037) already DELETES the custody row when a shred
//! executes, and `apply_remote_event` (db/020) already refuses to create one for a target
//! already in `erasure_shred_log`. So on a healthy node the `NOT EXISTS` clause selects
//! nothing extra. It is kept because the failure it prevents — an erased body's key
//! resurrected on a restored node — is irreversible, and because the two upstream defences
//! are in a different codebase layer (SQL) that this file cannot see change.

use crate::localstate::{episode_dek_to_cbor, EpisodeDek, LocalState};

/// Read this node's exportable local state.
///
/// `unwrap_secret` is the node's independent X25519 secret, loaded from the `<key>.unwrap`
/// keystore file by the caller (it is not in the database and never will be — a DB backup
/// that could reconstruct a DEK would defeat the whole custody plane). `None` means the
/// caller could not load it; the export is still built, carrying custody rows that a restore
/// will not be able to open, and the caller must WARN. That degradation is deliberate: the
/// export is optional and the event medium is the load-bearing copy, so a missing passphrase
/// on an unattended backup run must never abort the backup — but an operator has to be told,
/// or they will discover it only during a restore.
///
/// # What each slot gets, and why the empty ones are empty
///
/// * `episode_deks` — one CBOR [`EpisodeDek`] per surviving `event_dek` row. The DEK is
///   copied **wrapped**, byte for byte as the database holds it; this function never
///   unwraps anything, so no raw key material passes through it.
/// * `unwrap_secret` — the caller's secret, if it had one.
/// * `node_default_deks`, `config`, `drafts` — empty, and legitimately: no node-default
///   keystore, node-config table, or draft store exists anywhere in the built system yet.
///   That is "nothing to read", not "not implemented".
///
/// # Memory footprint, stated because it is unbounded by construction
///
/// Every surviving `event_dek` row is materialised into one in-memory `LocalState`, which
/// `build_export_container` then CBOR-encodes and encrypts in a single shot — so peak
/// residency is roughly three copies of the whole custody set. At the scale a node holds
/// today (one wrapped 32-byte-ish DEK per sealed body) that is trivially fine, and a
/// streaming/chunked export would be speculative generality now. It is written down because
/// nothing here bounds it: the day a node holds millions of sealed bodies, this is the line
/// that has to change, and a reader should not have to rediscover that from a memory spike.
pub async fn read_local_state(
    db: &tokio_postgres::Client,
    unwrap_secret: Option<&[u8; 32]>,
) -> anyhow::Result<LocalState> {
    use anyhow::Context;

    // `event_id::text`: this crate does not enable tokio-postgres's `with-uuid-1` feature,
    // so a UUID column cannot be decoded directly — cast in SQL and carry it as a String,
    // which is the repo-wide read idiom and is also what `EpisodeDek` stores.
    //
    // ORDER BY makes the export DETERMINISTIC: two runs over the same custody produce
    // byte-identical `episode_deks`, which keeps a diff of two export bundles meaningful.
    let rows = db
        .query(
            "SELECT d.event_id::text AS event_id, d.dek_wrapped \
             FROM event_dek d \
             WHERE NOT EXISTS ( \
                 SELECT 1 FROM erasure_shred_log s WHERE s.target_event_id = d.event_id \
             ) \
             ORDER BY d.event_id",
            &[],
        )
        .await
        .context("reading event_dek custody for the local-state export")?;

    let episode_deks = rows
        .iter()
        .map(|r| {
            episode_dek_to_cbor(&EpisodeDek {
                event_id: r.get::<_, String>("event_id"),
                dek_wrapped: r.get::<_, Vec<u8>>("dek_wrapped"),
            })
        })
        .collect();

    Ok(LocalState {
        version: 1,
        node_default_deks: Vec::new(), // no node-default keystore exists yet (#495 promise 2)
        episode_deks,
        config: None,       // no node config table exists yet
        drafts: Vec::new(), // no draft store exists yet
        unwrap_secret: unwrap_secret.map(|s| s.to_vec()),
    })
}
