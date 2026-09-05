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
//! already in `erasure_shred_log`. So on a healthy node the filter selects nothing extra.
//!
//! The filter itself used to be a `NOT EXISTS` clause written out here — one of two
//! hand-written spellings of "a shredded body's key must not travel" (the other lived in
//! cairn-sync's serve door). Slice 2c's db/051 gave the predicate its one home:
//! `event_custody_surviving`, a `security_invoker` view every caller now selects from
//! instead of re-deriving. This file is a CALLER, not the definition — but the reason a
//! last-line defence still belongs here, in Rust, is unchanged: the failure it prevents
//! (an erased body's key resurrected on a restored node) is irreversible, and the two
//! upstream defences above are in a different codebase layer (SQL) that this file cannot
//! see change out from under it.

use crate::localstate::{
    actor_registry_row_to_cbor, episode_dek_to_cbor, ActorRegistryRow, EpisodeDek, LocalState,
};
use cairn_event::keys::Secret32;

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
/// * `actor_registry` — one CBOR [`ActorRegistryRow`] per `actor_event` row (Task 11 / #500),
///   ordered by `seq`. See the query's own comment below for why it has to ride this export
///   at all, and for the caveat on how much trust these rows are owed.
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
    unwrap_secret: Option<&Secret32>,
) -> anyhow::Result<LocalState> {
    use anyhow::Context;

    // `event_id::text`: this crate does not enable tokio-postgres's `with-uuid-1` feature,
    // so a UUID column cannot be decoded directly — cast in SQL and carry it as a String,
    // which is the repo-wide read idiom and is also what `EpisodeDek` stores.
    //
    // `event_custody_surviving` (db/051) is the ONE definition of "not shredded" — this
    // query no longer re-derives it. ORDER BY makes the export DETERMINISTIC: two runs over
    // the same custody produce byte-identical `episode_deks`, which keeps a diff of two
    // export bundles meaningful.
    let rows = db
        .query(
            "SELECT c.event_id::text AS event_id, c.dek_wrapped \
             FROM event_custody_surviving c \
             ORDER BY c.event_id",
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

    // The actor registry rides the export because it can ride nothing else: actor_event has
    // no signed_bytes and replicates nowhere, while every clinical apply door gates on
    // actor_current. Without it a restored node refuses its own history (2a §3).
    //
    // ⚠️ These rows arrive authenticated by the CONTAINER's AEAD, not by per-row signatures
    // — the one part of a restore that is not verify-on-apply. 2e's ADR owes that caveat;
    // do not let this comment be the only place it is written down.
    //
    // `actor_event_id::text` and `pinned::text`: same idiom as `event_id::text` above, and
    // for the same underlying reason — `pinned` is JSONB, and this crate does not enable
    // tokio-postgres's `with-serde_json-1` feature, so a bare `jsonb` column has no `FromSql`
    // impl to land in (see `matcher_actor.rs`'s note on the identical idiom for writes).
    // Casting to `text` on the database side and carrying it as a `String` sidesteps that
    // entirely. ORDER BY seq, never recorded_at: two rows from one enrollment ceremony can
    // share a `clock_timestamp()` (issue #99), and seq is the monotonic tiebreak db/004 adds
    // for exactly this reason.
    let registry_rows = db
        .query(
            "SELECT actor_event_id::text AS actor_event_id, actor_id, op, kind, \
                    pinned::text AS pinned, signing_key_id, superseded_by, seq \
             FROM actor_event ORDER BY seq",
            &[],
        )
        .await
        .context("reading the actor registry for the local-state export")?;

    let actor_registry = registry_rows
        .iter()
        .map(|r| {
            actor_registry_row_to_cbor(&ActorRegistryRow {
                actor_event_id: r.get::<_, String>("actor_event_id"),
                actor_id: r.get::<_, Vec<u8>>("actor_id"),
                op: r.get::<_, String>("op"),
                kind: r.get::<_, Option<String>>("kind"),
                pinned: r.get::<_, Option<String>>("pinned"),
                signing_key_id: r.get::<_, Option<String>>("signing_key_id"),
                superseded_by: r.get::<_, Option<Vec<u8>>>("superseded_by"),
                seq: r.get::<_, i64>("seq"),
            })
        })
        .collect();

    // `from_custody_and_registry` rather than a struct literal: it is one of only TWO
    // producers of a `LocalState` (the other is `empty()`), and keeping the set closed is
    // what stops a third one appearing that skips the `erasure_shred_log` filter above — the
    // failure this file's header calls out by name (#511 rides-along 1).
    Ok(LocalState::from_custody_and_registry(
        episode_deks,
        unwrap_secret.cloned(),
        actor_registry,
    ))
}
