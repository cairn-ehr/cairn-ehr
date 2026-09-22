//! The funnel's two ports, backed by this node's database.
//!
//! # What this crate is
//!
//! `cairn_gui_data::port` declares `PatientSearch` and `PatientRegistration`; `--mock`
//! implements them over fixtures. This crate implements them over a real
//! `tokio_postgres::Client`, by delegating to the `cairn-node` orchestrators that already own
//! the rules — `search_patients` (§5.8) and `register_patient` (§5.3, ADR-0061). It adds **no
//! clinical logic of its own**. There is exactly one decision in here, and it is the one in
//! [`error`]: whether a failure was a VERDICT about this call or an accident that befell it.
//!
//! # Why it is a separate crate
//!
//! See the comment at the top of `Cargo.toml`. In one line: `cairn-gui-data` must stay free of
//! a database driver, and `/crates` may never depend on `cairn-gui`.
//!
//! # The window still signs nothing (§9.6)
//!
//! [`LiveData`] holds the NODE's key, because the node seals bodies and holds custody
//! (ADR-0052). It does not hold the clinician's key: `register_patient` takes no human author,
//! so a registration is not a per-write human-authored clinical act in the ADR-0053 sense
//! today. When that changes, it changes in `cairn-node` first and this crate follows.
pub mod error;
mod funnel;

use cairn_event::SigningKey;
use tokio::sync::Mutex;
use tokio_postgres::Client;

/// One node connection, plus the identity every write is sealed under.
///
/// # Why `tokio::sync::Mutex` and not `std::sync::Mutex`
///
/// `register_patient` takes `&mut Client`, while `PatientRegistration::register` takes `&self`
/// and must return a `Send` future. A `std::sync::MutexGuard` is not `Send`, so holding one
/// across the `.await` inside `register` does not compile — and the mock's "compute before the
/// async block" trick is not available here, because the whole body is awaits. The port doc in
/// `cairn-gui-data` states this so it is not rediscovered by fighting the compiler.
///
/// # One connection, not a pool
///
/// The funnel is one clerk at one keyboard; there is no concurrency worth pooling for, and a
/// mutex makes the borrow rules explicit at every call site. If a later slice puts two windows
/// on one `LiveData`, the second one waits — which is correct rather than merely acceptable:
/// `register_patient` ticks this node's HLC, and two registrations interleaving inside one
/// connection is not a thing to be clever about.
pub struct LiveData {
    db: Mutex<Client>,
    node_sk: SigningKey,
    /// Hex of `node_sk`'s verifying key. Derived ONCE, here, rather than per call, so the kid
    /// a registration is sealed under cannot drift from the key that sealed it.
    node_kid: String,
    /// This node's origin id, as `cairn_node::identity::load_local` reports it.
    node_origin: String,
}

impl LiveData {
    /// Take ownership of a connection whose schema is already loaded.
    ///
    /// The caller connects (`cairn_node::db::connect_and_load_schema`) and reads the node
    /// identity (`cairn_node::identity::load_local`), exactly as the window's
    /// `build_live_state` already does. Passing those in rather than doing them here keeps the
    /// "which node am I" question answered in ONE place per window: a `LiveData` that
    /// connected on its own could end up describing a different node than the rest of the
    /// window, and nothing on screen would say so.
    ///
    /// # ⚠️ It does NOT enrol the signing key, and the CLI does
    ///
    /// `cairn-node patient-register` calls `ensure_registration_actor` first, which enrols an
    /// unknown signing key as a `device` actor with role `registration-desk`. This does not,
    /// deliberately: enrolling an actor is **provisioning**, and provisioning as a write-path
    /// side effect is the shape trap 2 forbids (ADR-0066 decision 6 made `ensure_unwrap_key`
    /// refuse rather than quietly provision, for the same reason). A GUI silently minting a
    /// `device` actor on somebody's node is worse than the CLI doing it, not better.
    ///
    /// **So on a node where `patient-register` has never been run, the first registration
    /// through this port is REFUSED** — db/005's *"signer … is not an enrolled, non-revoked
    /// actor"*. Since #648 that arrives as [`cairn_gui_data::port::DataError::Refused`]
    /// carrying the floor's own sentence, rather than as an outage inviting a pointless
    /// retry, which is the right failure; but the message names a key id, not a remedy.
    /// Resolving that asymmetry is
    /// [#654](https://github.com/cairn-ehr/cairn-ehr/issues/654), and it belongs to the slice
    /// that first puts this in front of a person.
    pub fn new(db: Client, node_sk: SigningKey, node_origin: String) -> Self {
        let node_kid = hex::encode(node_sk.verifying_key().to_bytes());
        Self {
            db: Mutex::new(db),
            node_sk,
            node_kid,
            node_origin,
        }
    }
}
