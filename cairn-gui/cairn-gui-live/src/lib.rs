//! The funnel's two ports, backed by this node's database.
//!
//! # What this crate is
//!
//! `cairn_gui_data::port` declares `PatientSearch` and `PatientRegistration`; `--mock`
//! implements them over fixtures. This crate implements them over a real
//! `tokio_postgres::Client`, by delegating to `cairn-node`, which owns the rules: the search
//! calls `search_patients` (§5.8), and a registration (§5.3, ADR-0061) goes through the node's
//! terminal service, a request the window submits and then polls
//! (`cairn_node::patient::register_request`, #909). It adds **no clinical logic of its own**.
//! There is exactly one decision in here, and it is the one in [`error`]: whether a failure was
//! a VERDICT about this call or an accident that befell it.
//!
//! # Why it is a separate crate
//!
//! See the comment at the top of `Cargo.toml`. In one line: `cairn-gui-data` must stay free of
//! a database driver, and `/crates` may never depend on `cairn-gui`.
//!
//! # The window signs nothing, and holds no node key (§9.6, #909)
//!
//! A registration is NODE-signed (`register_patient` takes no human author, so it is not a
//! per-write human-authored clinical act in the ADR-0053 sense today), and db/005's door resolves
//! that signature to an enrolled, non-revoked actor. Since #909 the window holds only the node's
//! PUBLIC id ([`LiveData::node_kid`]), never its secret key: `PatientRegistration::register` writes
//! a request to db/083's queue and the node's terminal service registers and signs it
//! (`cairn_node::patient::register_request`, the funnel's adapter in `funnel.rs`).
//!
//! **Sealing does not enter into it, and the distinction matters to anyone arriving from the
//! medication stream.** A registration is an unsealed identity event — `register_patient`
//! names neither a DEK nor custody, and `db/045`'s projection opens with `IF e.sealed THEN
//! RETURN;`. ADR-0052's born-sealed bodies and the custody question they raise belong to
//! clinical content, not to this path.
mod connection;
pub mod error;
mod funnel;
pub mod node;

use cairn_node::identity::Identity;
use std::sync::Arc;
use tokio_postgres::Client;

pub use connection::{
    cause_of, failed_reopen_cause, name_cut_in, says_operator_remedy, ConnectionGuard,
    ConnectionLost, CutOff, CutWatch, NodeConnection, Refusal,
};

/// One node connection, plus the node identity every write names.
///
/// # Why [`NodeConnection`]'s mutex is `tokio::sync::Mutex` and not `std::sync::Mutex`
///
/// The ports take `&self` and must return `Send` futures, and every call here holds the
/// connection's guard across an `.await` (a query, a submit, each poll of a registration
/// request). A `std::sync::MutexGuard` is not `Send`, so holding one across those awaits does
/// not compile — and the mock's "compute before the async block" trick is not available here,
/// because the whole body is awaits. The port doc in `cairn-gui-data` states this so it is not
/// rediscovered by fighting the compiler.
///
/// # One connection, not a pool — and SHARED with the chart
///
/// The funnel is one clerk at one keyboard; there is no concurrency worth pooling for, and a
/// mutex makes the borrow rules explicit at every call site. The window's chart commands read
/// through the SAME connection ([`LiveData::sharing`]), so "which node am I" is answered once
/// per window rather than once per connection. If a later slice puts two windows
/// on one `LiveData`, the second one waits for each call the first makes. A registration holds
/// the connection only per call (submit, each poll, expire), never across its poll sleeps, so it
/// does not shut the chart commands out for the seconds the node's terminal service may take
/// (only an `expire` waiting behind the service's own registration of the request holds it).
///
/// The window's connection is reopened when it is lost (#807, [`NodeConnection::reopening`]);
/// one made by [`LiveData::new`] is not ([`NodeConnection::fixed`]).
pub struct LiveData {
    /// `Arc` so the window's chart commands can hold the same connection (see above).
    db: Arc<NodeConnection>,
    /// Hex of the node's verifying key (`Identity::pubkey_hex`): the node's PUBLIC id, all the
    /// window holds of its key (#909). The secret key stays with the node's terminal service.
    node_kid: String,
}

impl LiveData {
    /// Take ownership of a connection whose schema is already loaded. It is never reopened
    /// ([`NodeConnection::fixed`]): the caller made it and may have set session state on it.
    ///
    /// The caller connects (`cairn_node::db::connect`), checks the schema stamp
    /// (`read_schema_stamp` + `window_schema_verdict`; the window never loads the
    /// schema, #898) and reads the node identity (`cairn_node::identity::load_local`), exactly as the window's
    /// `build_live_state` already does. Passing those in rather than doing them here keeps the
    /// "which node am I" question answered in ONE place per window: a `LiveData` that
    /// connected on its own could end up describing a different node than the rest of the
    /// window, and nothing on screen would say so.
    ///
    /// # Why the whole `Identity`, and not a `node_kid: String`
    ///
    /// `Identity` carries FOUR `String` fields — `node_id_hex`, `pubkey_hex`, `fingerprint`,
    /// `address`. A `String` parameter accepts all four, and `""`, and compiles either way.
    /// Only `pubkey_hex` is the node's kid, the key [`LiveData::standing`] and
    /// [`LiveData::require_provisioned`] ask about. Taking the whole value and reading the
    /// field HERE makes the right answer the only available one. (Until #909 this also took the
    /// HLC origin, `node_id_hex`, for `register_patient`; the node's terminal service stamps its
    /// own now.)
    ///
    /// # ⚠️ It does NOT enrol the signing key — AND NEITHER DOES THE CLI ANY MORE (#654)
    ///
    /// Enrolling an actor is **provisioning**, and provisioning as a write-path side effect is
    /// the shape trap 2 forbids (ADR-0066 decision 6 made `ensure_unwrap_key` refuse rather
    /// than quietly provision, for the same reason). This port never did it.
    ///
    /// Until #654 the CLI *did*: a private `ensure_registration_actor` enrolled an unknown
    /// signing key as a `device` actor on first use, at fifteen write subcommands. That made a
    /// node's behaviour depend on which surface touched it first. It is gone. `cairn-node init`
    /// provisions, `cairn-node enroll-device-actor` is the named remedy for a node that never
    /// ran `init`, and every write path on both surfaces now refuses.
    ///
    /// **So on a node that was never provisioned, a registration through this port is
    /// REFUSED** — db/005's *"signer … is not an enrolled, non-revoked actor"*, which the
    /// terminal service settles as the request's refusal and this port returns as
    /// [`cairn_gui_data::port::DataError::Refused`] (#648). That sentence names a key id, not a
    /// remedy, so the window does not rely on it: it probes [`LiveData::standing`] at launch and
    /// asks [`LiveData::require_provisioned`] before each registration, which refuses as
    /// `NotProvisioned` naming the remedy (#654 option 2, #665; see `node.rs`).
    pub fn new(db: Client, identity: &Identity) -> Self {
        Self::sharing(Arc::new(NodeConnection::fixed(db)), identity)
    }

    /// As [`LiveData::new`], over a connection the caller also holds.
    ///
    /// The reference window uses this: its medication commands read through the same
    /// connection the funnel writes through, so one window describes one node. Everything
    /// [`LiveData::new`]'s doc says about `identity` and enrolment applies unchanged.
    pub fn sharing(db: Arc<NodeConnection>, identity: &Identity) -> Self {
        Self {
            db,
            node_kid: identity.pubkey_hex.clone(),
        }
    }

    /// The connection this port uses, for a caller that must read through the same one.
    ///
    /// A production accessor, not a test convenience: tests read rows back on a connection
    /// of their own (`tests/common`'s `connect`), as that helper's doc asks.
    pub fn connection(&self) -> Arc<NodeConnection> {
        self.db.clone()
    }

    /// Hex of this node's verifying key — the key [`LiveData::standing`] asks about. The
    /// window's launch-probe sentence must name THIS key (#670: a standing carries no subject,
    /// so pairing it with the right key is the caller's job).
    pub fn node_kid(&self) -> &str {
        &self.node_kid
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A compile-time claim 2c depends on and nothing else here would catch.
    ///
    /// Tauri's `manage`/`State` require `Send + Sync + 'static`, and so does holding this across
    /// an `.await` in a command. Both hold today — `NodeConnection` is `Sync` because its one
    /// (tokio) mutex is, and the rest is plain data. The obvious "simplification" of
    /// swapping that mutex for a `std` one in `connection.rs` takes `Send` away from the guard
    /// rather than from the struct, so this test would stay green; the guard's own pin is
    /// `connection.rs`'s `the_guard_may_be_held_across_an_await`.
    ///
    /// A static assertion rather than a dependency on `static_assertions`: one function that is
    /// never called is cheaper than a crate, and the error message names the bound directly.
    #[test]
    fn live_data_can_be_tauri_managed_state() {
        fn assert_send_sync_static<T: Send + Sync + 'static>() {}
        assert_send_sync_static::<LiveData>();
    }
}
