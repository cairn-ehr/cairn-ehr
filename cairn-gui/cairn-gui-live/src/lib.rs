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
//! [`LiveData`] holds the NODE's key because the node **signs** this act, and db/005's door
//! resolves that signature to an enrolled, non-revoked actor before admitting anything. It
//! does not hold the clinician's key: `register_patient` takes no human author, so a
//! registration is not a per-write human-authored clinical act in the ADR-0053 sense today.
//! When that changes, it changes in `cairn-node` first and this crate follows.
//!
//! **Sealing does not enter into it, and the distinction matters to anyone arriving from the
//! medication stream.** A registration is an unsealed identity event — `register_patient`
//! names neither a DEK nor custody, and `db/045`'s projection opens with `IF e.sealed THEN
//! RETURN;`. ADR-0052's born-sealed bodies and the custody question they raise belong to
//! clinical content, not to this path.
pub mod error;
mod funnel;
pub mod node;

use cairn_event::SigningKey;
use cairn_node::identity::Identity;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio_postgres::Client;

/// One node connection, plus the identity every write is signed under.
///
/// # Why `tokio::sync::Mutex` and not `std::sync::Mutex`
///
/// `register_patient` takes `&mut Client`, while `PatientRegistration::register` takes `&self`
/// and must return a `Send` future. A `std::sync::MutexGuard` is not `Send`, so holding one
/// across the `.await` inside `register` does not compile — and the mock's "compute before the
/// async block" trick is not available here, because the whole body is awaits. The port doc in
/// `cairn-gui-data` states this so it is not rediscovered by fighting the compiler.
///
/// # One connection, not a pool — and SHARED with the chart
///
/// The funnel is one clerk at one keyboard; there is no concurrency worth pooling for, and a
/// mutex makes the borrow rules explicit at every call site. The window's chart commands read
/// through the SAME connection ([`LiveData::sharing`]), so "which node am I" is answered once
/// per window rather than once per connection. If a later slice puts two windows
/// on one `LiveData`, the second one waits — which is correct rather than merely acceptable:
/// `register_patient` ticks this node's HLC, and two registrations interleaving inside one
/// connection is not a thing to be clever about.
pub struct LiveData {
    /// `Arc` so the window's chart commands can hold the same connection (see above).
    db: Arc<Mutex<Client>>,
    node_sk: SigningKey,
    /// Hex of `node_sk`'s verifying key. Derived ONCE, here, rather than per call, so the kid
    /// a registration is signed under cannot drift from the key that signed it.
    node_kid: String,
    /// This node's origin id — `Identity::node_id_hex`, and never one of its three siblings.
    /// [`LiveData::new`] takes the whole `Identity` so that is true by construction; see its
    /// doc for what a mis-picked field would do to causal order.
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
    /// # Why the whole `Identity`, and not a `node_origin: String`
    ///
    /// `Identity` carries FOUR `String` fields — `node_id_hex`, `pubkey_hex`, `fingerprint`,
    /// `address`. A `String` parameter accepts all four, and `""`, and compiles either way.
    /// Only one is right: `node_origin` is what `register_patient` feeds to `next_hlc`, so it
    /// becomes the HLC's origin — the third sort key of causal order (`db/001`) and the final
    /// tiebreaker between two concurrent demographic assertions (`db/011`). A mis-picked
    /// field therefore changes merge outcomes across the federation, silently, on events that
    /// are append-only and can only ever be overlaid. Taking the whole value and reading the
    /// field HERE makes the right answer the only available one.
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
    /// REFUSED** — db/005's *"signer … is not an enrolled, non-revoked actor"*, arriving as
    /// [`cairn_gui_data::port::DataError::Refused`] (#648). That sentence names a key id, not a
    /// remedy, so the window does not rely on it: it probes [`LiveData::standing`] at launch and
    /// asks [`LiveData::require_provisioned`] before each registration, which refuses as
    /// `NotProvisioned` naming the remedy (#654 option 2, #665; see `node.rs`).
    pub fn new(db: Client, node_sk: SigningKey, identity: &Identity) -> Self {
        Self::sharing(Arc::new(Mutex::new(db)), node_sk, identity)
    }

    /// As [`LiveData::new`], over a connection the caller also holds.
    ///
    /// The reference window uses this: its medication commands read through the same
    /// connection the funnel writes through, so one window describes one node. Everything
    /// [`LiveData::new`]'s doc says about `identity` and enrolment applies unchanged.
    pub fn sharing(db: Arc<Mutex<Client>>, node_sk: SigningKey, identity: &Identity) -> Self {
        let node_kid = hex::encode(node_sk.verifying_key().to_bytes());
        Self {
            db,
            node_sk,
            node_kid,
            node_origin: identity.node_id_hex.clone(),
        }
    }

    /// The connection this port uses, for a caller that must read through the same one.
    ///
    /// A production accessor, not a test convenience: tests read rows back on a connection
    /// of their own (`tests/common`'s `connect`), as that helper's doc asks.
    pub fn connection(&self) -> Arc<Mutex<Client>> {
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
    /// an `.await` in a command. Both hold today — `Mutex<Client>` is `Sync` because the mutex
    /// is, and `SigningKey` is plain data — but neither is written down anywhere, and the
    /// obvious "simplification" of swapping the tokio mutex for a `std` one takes `Send` away
    /// from the guard rather than from the struct, so the failure would land in 2c as a wall of
    /// lifetime errors inside a command body rather than here.
    ///
    /// A static assertion rather than a dependency on `static_assertions`: one function that is
    /// never called is cheaper than a crate, and the error message names the bound directly.
    #[test]
    fn live_data_can_be_tauri_managed_state() {
        fn assert_send_sync_static<T: Send + Sync + 'static>() {}
        assert_send_sync_static::<LiveData>();
    }
}
