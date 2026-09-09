//! ADR-0026 slice D — the sealed local-state export (container shape).
//!
//! WHY THIS EXISTS: ADR-0026 point 3 requires a node's NON-EVENT, non-signing-key
//! material — the data-at-rest keystore (node-default DEKs + sealed-episode DEKs),
//! node config, and the draft/scratchpad store — to be exportable as an encrypted
//! bundle co-located with the cold-peer backup medium, so a dead disk does not lose
//! it. The signing key is DELIBERATELY EXCLUDED (point 4): a stolen, unsealed artifact
//! must yield read access, never a signing identity.
//!
//! SCOPE (slice D): this module builds the can't-retrofit SHAPE — the format, the
//! dual-recipient secret lifecycle (a long-lived local-state DEK dual-wrapped once at
//! provisioning), the container, and the restore path — with typed slots the clinical tier
//! fills later via additive evolution (principle 11). **Three of those slots are no longer
//! empty** — see the state of play below; this paragraph is left otherwise as written
//! because its own expiry is the lesson recorded under it. The genuine
//! day-one piece is `establish_lsk`: state accrued before the channel exists has no
//! durability path, so the channel must exist from `init`.
//!
//! # History of this header, because it is the lesson (#495)
//!
//! The scope paragraph above once ended *"the federation-node tier has no clinical surface
//! yet, so the bundle is EMPTY today"*. That was true when slice D was written and
//! **expired silently** when ADR-0052 made every clinical body born-sealed: the node began
//! holding real `event_dek` custody while these slots stayed empty, and because restore
//! mints a fresh signing seed (ADR-0026 decision 4) from which the X25519 unwrap secret was
//! then HKDF-derived (ADR-0052 decision 4), **every born-sealed body on a restored SOLO
//! node was unopenable**. A document whose stated precondition had expired, believed for
//! months. Read the paragraph above with that in mind before adding to it.
//!
//! # What is closed, and what is still open — state of play
//!
//! **CLOSED (#495 / ADR-0066 — see `docs/spec/decisions/`, ADR number 0066, "Identity dies
//! with the disk; custody must not").** The node's unwrap key is now an INDEPENDENT X25519
//! keypair (decision 1) living in its own `<key>.unwrap` keystore file, and it rides this
//! export beside the custody rows (decision 3): [`LocalState::unwrap_secret`] carries the
//! secret, [`LocalState::episode_deks`] carries the wrapped `event_dek` rows minus every
//! target in `erasure_shred_log` (decision 7). The producer is
//! [`crate::localstate_read::read_local_state`].
//!
//! Both halves of the KEY's journey are now closed: the export carries the secret (decision
//! 3) and [`apply_local_state`] installs it on arrival, re-sealed under the restored node's
//! own secrets, and registers its public half (decision 4).
//!
//! Read that narrowly, because the ADR's title — "identity dies with the disk; custody must
//! not" — is still not fully this system's behaviour. What survives a restore today is the
//! KEY. The custody ROWS ride along and are counted, but nothing inserts them — because
//! nothing yet RESTORES a clinical event for them to be custody of (#500, below). Neither
//! half is useful without the other; do not read this section as more than it says.
//!
//! **STILL OPEN, do not read this module as closing them:**
//!
//! - **Nothing restores a clinical event (#500).** Since DR slice 2c the backup medium DOES
//!   carry the clinical plane, with each record's wrapped DEK — but `restore` and
//!   `verify-backup` read the federation plane alone (`backup::node_plane_events`), so a
//!   restored node still gets a working key and nothing to open with it. Slice 2d.
//! - **The restore side lands the KEY, and the rows land with the events.**
//!   [`apply_local_state`] installs the recovered unwrap secret and registers its public half
//!   (ADR-0066 decision 4), so a restored node's custody IS the dead node's custody — #495's
//!   restore half, closed. The carried `episode_deks` are still not inserted HERE, and that is
//!   now a placement decision rather than a gap: since #554 slice 2d the restore unwraps each
//!   record's DEK in Rust and passes the PLAINTEXT to `apply_remote_event`, which re-wraps it
//!   through the one door that owns `event_dek`. Piping a carried, already-wrapped key
//!   straight into that door's `p_dek` would DOUBLE-WRAP every key in the clinic's record —
//!   rows that are present, well-formed, the right length, and unwrap to noise. The count is
//!   still reported, because the export's copy is the fallback when a medium's is missing.
//! - **Promise 2 has no subject.** [`LocalState::node_default_deks`] stays empty because no
//!   node-default data-at-rest keystore exists anywhere in the built system. That slot's
//!   emptiness is neither honoured nor violated; it names a tier that must exist first.
//! - **The actor registry travels AND is installed (#554 slice 2d).**
//!   [`crate::localstate_read::read_local_state`] fills [`LocalState::actor_registry`] from
//!   `actor_event` (db/004), and [`apply_local_state`] now hands the whole set to
//!   `restore_actor_registry` (db/052) — so a restored node can resolve the authors of its own
//!   history rather than refusing every clinical event it just inherited. Two counts are
//!   reported, deliberately apart: [`AppliedLocalState::actor_registry_carried`] (what the
//!   export held) and [`AppliedLocalState::actor_registry_restored`] (what THIS run inserted),
//!   which differ on a resumed restore.
//!
//!   **These rows are the one part of a restore that is not verify-on-apply.** They arrive
//!   authenticated by the `CAIRNL1` container's AEAD and nothing else — no per-row signature —
//!   while every clinical event around them is individually signature-verified by
//!   `apply_remote_event`. Accepted deliberately (ADR-0067): whoever holds the export AND its
//!   passphrase or recovery code already controls the restored node completely, so refusing
//!   here would cost the record and buy nothing. It is PRINTED to the operator at restore
//!   time, because a limitation living only in a design doc is one nobody finds.
//!
//! `crates/cairn-node/tests/dr_clinical_guarantee_gap.rs` holds the guards for all of the
//! above, and says of each whether it asserts a guarantee or pins a surviving defect.

use crate::seal::{
    self, aead_decrypt, aead_encrypt, normalize_recovery_code, rand_bytes, ArgonParams, Wrap,
};
use cairn_event::keys::Secret32;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
// `Zeroizing` wipes its wrapped bytes on drop (issue #54), matching the convention in `seal.rs`.
// Since #511 the fixed-size key material here is `Secret32`, which wipes itself — the LSK, minted
// and recovered alike (`seal::try_unwrap` returns a `Secret32`). What is still `Zeroizing` is the
// VARIABLE-length secret this crate handles and `Secret32` cannot cover: the decrypted bundle
// CBOR, a `Zeroizing<Vec<u8>>` that must not linger in freed heap either.
use zeroize::Zeroizing;

/// Magic for the `.lsk` sidecar (the dual-wrapped LSK). 8 bytes, like CAIRNK1/CAIRNB1.
const SIDECAR_MAGIC: &[u8] = b"CAIRNX1\n";
/// Magic for the export container (the sealed local-state bundle).
const CONTAINER_MAGIC: &[u8] = b"CAIRNL1\n";

#[derive(thiserror::Error, Debug)]
pub enum LocalStateError {
    /// The bytes are not a valid bundle / container / sidecar (bad magic or malformed body).
    #[error("decode: {0}")]
    Decode(String),
    /// A sealing/unsealing step failed (wrong secret, tamper, or entropy failure).
    /// Reachable from `establish_lsk`, `seal_local_state`, and their callers.
    #[error("seal: {0}")]
    Seal(String),
    // NOTE: no `Io` variant. The FORMAT functions in this module do no file I/O (reads
    // happen in `main.rs` via `anyhow`). The one function here that does touch the
    // filesystem — `CustodyKeyDestination::install`, which writes the inherited unwrap key —
    // returns `anyhow::Result` rather than this enum, because its failures are already typed
    // by `keystore::KeystoreError` and its caller only ever reports them to an operator.
    // Adding a variant here would be YAGNI; add one when a `LocalStateError`-returning
    // function needs to distinguish an I/O failure.
}

/// The highest bundle `version` this build understands. A bundle declaring a higher
/// version must be REFUSED, not partially applied — see [`from_cbor`].
pub const SUPPORTED_LOCAL_STATE_VERSION: u8 = 1;

/// The node-local material ADR-0026 point 3 exports. The leaf type is opaque `Vec<u8>` so
/// we reserve the SLOT SHAPE without committing to the clinical tier's internal schema (no
/// speculative generality).
///
/// **Which slots carry something, as of #495 / ADR-0066 / Task 11 (#500)** — read this before
/// writing a comment that calls the bundle empty, because two earlier comments here said
/// exactly that and both went stale (the module header has the history):
///
/// - [`Self::episode_deks`], [`Self::unwrap_secret`] and [`Self::actor_registry`] are
///   **FILLED** on a provisioned node by [`crate::localstate_read::read_local_state`]. The
///   first two are the custody that survives a dead disk; the registry is what a restored
///   node needs to be ALLOWED to apply that custody again (`actor_current` gates every
///   clinical apply door).
/// - [`Self::node_default_deks`], [`Self::config`] and [`Self::drafts`] are still empty,
///   legitimately: none of the three has a store anywhere in the built system to be filled
///   from. Their emptiness is "nothing exists yet", not "we forgot".
///
/// The signing key is DELIBERATELY ABSENT (ADR-0026 point 4): a stolen, unsealed export
/// must grant read access, never a signing identity. Do not add it here. An unwrap secret is
/// read access and so belongs; a signing seed is an identity and does not (ADR-0066
/// decision 2 restates this and keeps the boundary exactly where ADR-0026 drew it).
///
/// `serde(default)` on every content field makes this ADDITIVELY evolvable (principle 11):
/// a bundle written before a field existed still deserializes, with that field defaulted.
///
/// `deny_unknown_fields` is the OTHER half of that contract, and the fix for review finding
/// A7c: without it, a bundle written by a NEWER cairn-node carrying a content-bearing field
/// this build doesn't know (e.g. a future clinical-tier `episode_deks` variant) would have
/// that field SILENTLY DROPPED on read, and `is_empty()` would call the restore a success
/// while quietly discarding key material — the exact failure the format is "can't-retrofit"
/// to guard against. With this, an unknown field is a LOUD refusal instead. `default` (for
/// missing fields) and `deny_unknown_fields` (for extra fields) are orthogonal and compose:
/// an OLDER bundle still deserializes, a NEWER one is refused rather than silently lossy.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalState {
    /// Bundle format version (bump only on a NON-additive change, which we avoid).
    /// NOT `#[serde(default)]`: absence of a version is always a malformed bundle —
    /// we must refuse it rather than silently assume v1.
    version: u8,
    /// Node-default data-at-rest keys. Empty — and no store exists to fill them from
    /// (#495: promise 2 has no subject; see `dr_clinical_guarantee_gap.rs`).
    #[serde(default)]
    node_default_deks: Vec<Vec<u8>>,
    /// Sealed-episode DEKs — one [`EpisodeDek`] per surviving `event_dek` row, CBOR-encoded
    /// into this slot's opaque leaf type, minus every event named in `erasure_shred_log`
    /// (ADR-0026 point 6 / ADR-0066 decision 7). Each DEK travels **wrapped**, exactly as it
    /// sits in the database; [`Self::unwrap_secret`] is what opens them.
    #[serde(default)]
    episode_deks: Vec<Vec<u8>>,
    /// Node config blob. None today (no node config table exists yet).
    #[serde(default)]
    config: Option<Vec<u8>>,
    /// Draft / scratchpad store. Empty today (no draft store exists yet).
    #[serde(default)]
    drafts: Vec<Vec<u8>>,
    /// ADR-0066: this node's INDEPENDENT X25519 unwrap secret, so a restored node inherits
    /// custody of every body it also inherits. The signing key is still deliberately absent
    /// (ADR-0026 point 4): a stolen, unsealed export must yield READ access, never a
    /// signing identity — and an unwrap secret is exactly read access.
    #[serde(default)]
    unwrap_secret: Option<Secret32>,
    /// Task 11 (#500): the append-only actor registry (`actor_event`, db/004), one CBOR
    /// [`ActorRegistryRow`] per row, ordered by `seq`. It rides this export because it can
    /// ride nothing else: `actor_event` has no `signed_bytes` and replicates nowhere, while
    /// every clinical apply door gates on `actor_current` — without this slot a restored
    /// node holds keys and events it is never again ALLOWED to use (2a §3).
    ///
    /// Appended at the END of the struct, deliberately: field order is irrelevant to decode
    /// (CBOR here is a named map, not a tuple), so putting the new slot last keeps this diff
    /// a pure addition against every earlier field's declaration.
    ///
    /// No `skip_serializing_if`, on purpose, even for the empty case — #511's lesson,
    /// restated for a genuinely new slot rather than a type change: if an empty registry
    /// could vanish from the wire, "the registry silently never travelled" (a bug) would be
    /// byte-identical to "this node has no actors" (a fact), and an operator staring at a hex
    /// dump could never tell which one they are looking at. See
    /// `tests/localstate_wire_pins.rs`'s `the_empty_registry_encoding_is_pinned`.
    ///
    /// ⚠️ **Authenticated by the CONTAINER's AEAD only, never by a per-row signature.** Every
    /// OTHER slot in this bundle is either non-clinical (config, drafts) or itself the
    /// wrapped ciphertext of something signature-verified elsewhere (`episode_deks` opens
    /// bodies that were verified on apply). These rows are neither: they are the ONE part of
    /// a restore whose authenticity rests solely on "the container decrypted", not on
    /// verify-on-apply. That is acceptable ONLY because `apply_local_state` does not yet
    /// insert them (Task 11 is the write half; slice 2d is the apply half) — the caveat is
    /// recorded here so 2e's ADR is not the first place it is written down.
    #[serde(default)]
    actor_registry: Vec<Vec<u8>>,
}

impl LocalState {
    /// The bundle's ZERO VALUE — the honest answer for a node that holds nothing to carry
    /// (a freshly-`init`ed node, or one whose custody tables are empty).
    ///
    /// It is one of exactly TWO producers of a `LocalState`; the other is
    /// [`crate::localstate_read::read_local_state`], which reads real custody out of the
    /// database and is the one that must filter erased events. That pairing is pinned by
    /// `dr_clinical_guarantee_gap.rs`, because a THIRD producer skipping the filter is how
    /// an erased body's key would travel.
    pub fn empty() -> Self {
        LocalState {
            version: 1,
            node_default_deks: Vec::new(),
            episode_deks: Vec::new(),
            config: None,
            drafts: Vec::new(),
            unwrap_secret: None,
            actor_registry: Vec::new(),
        }
    }

    /// The OTHER producer, and the one that carries real custody: the bundle
    /// [`crate::localstate_read::read_local_state`] builds after filtering out every event
    /// named in `erasure_shred_log`. Named `..._and_registry` since Task 11 (#500), which
    /// widened it from two arguments to three — see the history note below.
    ///
    /// **Why this exists rather than a struct literal.** The fields above are PRIVATE and there
    /// are exactly TWO producers — this and [`Self::empty`] — because this struct's own doc has
    /// always warned that a third producer skipping that filter "is how an erased body's key
    /// would travel", and until #511 nothing prevented one. There is deliberately **no
    /// `set_episode_deks`** (and, since Task 11, no `set_actor_registry` either — a setter is
    /// exactly how a third producer would sneak in): the custody slot is the one the filter
    /// guards, so it can only be filled by a producer, and the only producer that fills it
    /// non-empty is this one. The mutators below touch slots the filter has nothing to say
    /// about.
    ///
    /// **What that does and does not promise, stated exactly.** This function does not itself
    /// filter — it takes the rows its caller hands it. What the private fields buy is that the
    /// *only in-tree filler* is [`crate::localstate_read::read_local_state`], which does filter,
    /// and that no code outside this module can assemble a `LocalState` around unfiltered rows
    /// without going through a producer a reviewer can see. It stays `pub` rather than
    /// `pub(crate)` because the golden-bytes wire pin (`tests/localstate_wire_pins.rs`) is a
    /// separate crate and must build a fully-populated bundle; that is a legitimate caller, and
    /// narrowing the visibility would only push it back to a test-only constructor with the same
    /// reach. An earlier version of this paragraph claimed the filter itself was the gate, which
    /// claimed more than the code delivers.
    ///
    /// **History: this was `from_custody(episode_deks, unwrap_secret)` before Task 11.** The
    /// rename, not a wrapper kept alongside it, is deliberate: the producer set stays closed at
    /// two, and a `from_custody` shim delegating to this one would not change that count (it
    /// contains no struct literal for the guard to see) but would still be a second NAME for
    /// filling the same slot — exactly the kind of quiet second door #511 closed. Every call
    /// site (including this crate's own tests) was updated instead.
    ///
    /// `actor_registry` is not filtered the way `episode_deks` is — there is no
    /// `erasure_shred_log`-shaped exclusion for actor-registry rows — but it is closed to this
    /// same single producer for the same structural reason: a setter or a second constructor is
    /// a door a reviewer has to keep re-checking forever, and a single producer is a door they
    /// check once.
    ///
    /// Pinned by `dr_clinical_guarantee_gap.rs`'s producer count — when it fails, ask whether
    /// the new producer filters, not whether the number should go up.
    pub fn from_custody_and_registry(
        episode_deks: Vec<Vec<u8>>,
        unwrap_secret: Option<Secret32>,
        actor_registry: Vec<Vec<u8>>,
    ) -> Self {
        LocalState {
            version: 1,
            node_default_deks: Vec::new(), // no node-default keystore exists yet (#495 promise 2)
            episode_deks,
            config: None,       // no node config table exists yet
            drafts: Vec::new(), // no draft store exists yet
            unwrap_secret,
            actor_registry,
        }
    }

    /// The bundle format version.
    pub fn version(&self) -> u8 {
        self.version
    }

    /// The reserved node-default data-at-rest key slot. Always empty today: no store exists
    /// to fill it from, which is #495's promise 2 having no subject rather than a defect.
    pub fn node_default_deks(&self) -> &[Vec<u8>] {
        &self.node_default_deks
    }

    /// The surviving wrapped custody rows, each a CBOR [`EpisodeDek`]. **Carried, not
    /// applied** — see [`apply_local_state`].
    pub fn episode_deks(&self) -> &[Vec<u8>] {
        &self.episode_deks
    }

    /// The node-config blob. `None` today: no node config table exists yet.
    pub fn config(&self) -> Option<&[u8]> {
        self.config.as_deref()
    }

    /// The draft/scratchpad store. Empty today: no draft store exists yet.
    pub fn drafts(&self) -> &[Vec<u8>] {
        &self.drafts
    }

    /// This node's independent X25519 unwrap secret, if the bundle carries one. `None` is a
    /// legitimate older export (pre-ADR-0066) and the caller must WARN, never treat it as a
    /// quiet success — see [`recovered_unwrap_secret`].
    pub fn unwrap_secret(&self) -> Option<&Secret32> {
        self.unwrap_secret.as_ref()
    }

    /// The surviving actor-registry rows, each a CBOR [`ActorRegistryRow`], ordered by `seq`
    /// (Task 11 / #500). **Carried, not applied**: nothing today inserts these into a
    /// restored node's `actor_event` — that is slice 2d. `is_empty()` on this slice means
    /// either "no export has run since this bundle predates the registry slot" or "this node
    /// really has no enrolled actors"; [`from_cbor`] cannot tell those apart from an old
    /// export alone, which is exactly why the empty encoding is pinned rather than skipped
    /// (see the field's own doc).
    ///
    /// ⚠️ Authenticated by the CONTAINER's AEAD only, never by a per-row signature — see the
    /// field's doc for the full caveat before trusting these rows the way a verified clinical
    /// event is trusted.
    pub fn actor_registry(&self) -> &[Vec<u8>] {
        &self.actor_registry
    }

    /// Set (or clear) the custody secret.
    ///
    /// Safe in a way `set_episode_deks` would not be: the `erasure_shred_log` filter is about
    /// which DEKs travel, and the secret is not a DEK. (For the full list of what may and may not
    /// be mutated, see [`Self::set_config`] — it is stated once, there.)
    pub fn set_unwrap_secret(&mut self, secret: Option<Secret32>) {
        self.unwrap_secret = secret;
    }

    /// Move the custody secret OUT of the bundle, leaving it without one.
    ///
    /// A move rather than a clone, deliberately: a caller lifting the node's custody key out
    /// of a decoded bundle should not silently leave a second live copy behind it.
    pub fn take_unwrap_secret(&mut self) -> Option<Secret32> {
        self.unwrap_secret.take()
    }

    /// Replace the draft slot. Same reasoning as [`Self::set_unwrap_secret`]'s: drafts are not
    /// custody rows, so the filter that guards `episode_deks` has nothing to say about them.
    pub fn set_drafts(&mut self, drafts: Vec<Vec<u8>>) {
        self.drafts = drafts;
    }

    /// Replace the node-config slot. Same reasoning again.
    ///
    /// **Where the line is, stated once for every mutator on this type.** There are setters for
    /// `unwrap_secret`, `drafts` and `config`, plus [`Self::take_unwrap_secret`] (four mutating
    /// methods in all), and deliberately NONE for `episode_deks`, `node_default_deks`,
    /// `actor_registry` or `version`. `episode_deks` is the slot the `erasure_shred_log`
    /// filter guards, so it may be filled only through [`Self::from_custody_and_registry`] —
    /// whose sole in-tree caller, [`crate::localstate_read::read_local_state`], is the code
    /// that applies that filter; `actor_registry` has no analogous filter but is closed to the
    /// same single producer for the same reason (Task 11 — a setter is how a third producer
    /// sneaks in); `node_default_deks` is reserved and wiped by this type's
    /// `Drop`; `version` is a format fact, not content. A future setter for any of those four
    /// is the change to argue about, not to make.
    ///
    /// **These four mutators have no production callers today** — every call site is in
    /// `crates/cairn-node/tests/`. They exist so the test crate can stage bundles that the now
    /// private fields no longer let it assemble directly. Said out loud rather than left to be
    /// inferred, because a `pub` mutator on the custody-bearing type reads like production API
    /// and is surface the producer-count guard cannot see; if one of them ever loses its last
    /// test caller, delete it rather than leaving an unreachable door into this struct.
    pub fn set_config(&mut self, config: Option<Vec<u8>>) {
        self.config = config;
    }

    /// True iff the bundle carries no content at all.
    ///
    /// ⚠️ **Not a validity check, and never a success condition.** Two earlier comments here
    /// framed emptiness first as "the only valid state at this tier" and then as "the state
    /// a node in #495 is stuck in"; both aged badly. What it means now is narrow and stable:
    /// this node had nothing to export. On a provisioned node holding born-sealed bodies
    /// that answer is FALSE, and must be — see [`Self::episode_deks`].
    ///
    /// Every content slot participates, [`Self::unwrap_secret`] included: a bundle carrying
    /// only the secret still carries key material, and treating it as empty would let
    /// [`apply_local_state`] wave it through as a no-op. [`Self::actor_registry`] joined this
    /// list in Task 11 for the same reason — a bundle carrying only registry rows still
    /// carries something a restore needs.
    pub fn is_empty(&self) -> bool {
        self.node_default_deks.is_empty()
            && self.episode_deks.is_empty()
            && self.config.is_none()
            && self.drafts.is_empty()
            && self.unwrap_secret.is_none()
            && self.actor_registry.is_empty()
    }
}

// `Debug` is DERIVED again (see the attribute on the struct above), and that is a
// STRENGTHENING, not a relaxation. It was derived until #495, hand-written from #495 to #511
// because `{:?}` on a populated bundle would otherwise have printed this node's custody key in
// full, and derived again once `unwrap_secret` became a `Secret32`, whose own `Debug` prints
// `Secret32(<redacted>)`. The redaction is now STRUCTURAL: it belongs to the secret rather than
// to this struct's formatter, so the next secret-bearing slot added here inherits it instead of
// re-earning it — which is exactly the failure mode #511 warned about for the `Drop` below.
// Presence is still visible (`Some(Secret32(<redacted>))` vs `None`), so a reader can tell
// "absent" from "hidden".

/// Wipe the slots that do not wipe themselves (issues #46/#54 — the convention this module
/// already follows for the LSK).
///
/// [`Self::unwrap_secret`] is a [`Secret32`] since #511, so its OWN `Drop` wipes it and this
/// impl no longer names it. That is the point of the type: wiping became structural.
///
/// [`Self::node_default_deks`] is the RESERVED, untyped slot — `Vec<Vec<u8>>`, with no producer
/// anywhere in the built system yet — so it is not covered structurally and is wiped here by
/// name. This loop is a no-op today and correct the day that slot is filled. #511's warning was
/// precisely that a `Drop` naming one field goes stale SILENTLY when a second is added, and no
/// test fails; keeping the impl and widening it is cheaper than re-discovering that.
///
/// LIMITS, so nobody reads this as more than it is: a `Vec` that was reallocated while being
/// built leaves its earlier buffer behind, and `serde` makes its own copies during encode and
/// decode. Wiping is a real reduction in exposure, not an erasure guarantee (#508).
impl Drop for LocalState {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        for key in self.node_default_deks.iter_mut() {
            key.zeroize();
        }
    }
}

/// One event's wrapped custody row, as it travels in [`LocalState::episode_deks`].
///
/// The slot's leaf type is opaque `Vec<u8>` by design (the container reserved the SLOT SHAPE
/// without committing to the clinical tier's schema), so each element is a small CBOR struct
/// rather than a format change: the container format is untouched by this type existing.
///
/// The DEK travels **wrapped**, exactly as it sits in `event_dek` — the export never holds
/// raw key material, and the separately-carried [`LocalState::unwrap_secret`] is what opens
/// it. `event_id` is the hyphenated UUID TEXT, matching how this crate carries every event
/// id (tokio-postgres's `uuid` feature is not enabled here).
///
/// `#[serde(deny_unknown_fields)]`, matching [`LocalState`]'s own contract one level up
/// (review finding I3 on Task 11 / #500): without it, a row written by a NEWER build
/// carrying a field this build doesn't know would have that field SILENTLY DROPPED on read
/// rather than loudly refused — the exact A7c failure `LocalState` guards against, one
/// struct down. Neither field gets `#[serde(default)]`: both are the row's own identity
/// (which event, which key) rather than optional content, so their absence should refuse —
/// same reasoning as `LocalState::version`, which is likewise never defaulted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EpisodeDek {
    pub event_id: String,
    pub dek_wrapped: Vec<u8>,
}

/// Serialize one custody row for the export slot. Pure.
pub fn episode_dek_to_cbor(d: &EpisodeDek) -> Vec<u8> {
    let mut out = Vec::new();
    ciborium::into_writer(d, &mut out).expect("CBOR serialization of EpisodeDek cannot fail");
    out
}

/// Parse one custody row from the export slot. Errors, never panics — a restore reading a
/// bit-rotted or foreign element must degrade honestly rather than abort a node mid-recovery.
///
/// **The length check is not decoration, and it is cheap only while it is early.** A
/// `dek_wrapped` that is not exactly [`cairn_event::seal::WRAPPED_DEK_LEN`] bytes can never
/// be opened — `unwrap_dek` refuses it — so a truncated or padded row is a custody row that
/// looks present and is permanently dead. Today that is inert: `apply_local_state` COUNTS
/// these rows without inserting them, because nothing yet restores a clinical event for them
/// to belong to (#500 — the medium carries them since slice 2c; the restore door does not
/// read them). **#500's restore half is what inserts them**, and after that a bad row is a chart entry
/// nobody can read or crypto-shred, discovered only when someone tries. Catching it at the
/// decode boundary costs one comparison and is the same reasoning
/// [`recovered_unwrap_secret`] applies one struct over — which is exactly where this check
/// was missing.
pub fn episode_dek_from_cbor(bytes: &[u8]) -> Result<EpisodeDek, LocalStateError> {
    let row: EpisodeDek =
        ciborium::from_reader(bytes).map_err(|e| LocalStateError::Decode(e.to_string()))?;
    if row.dek_wrapped.len() != cairn_event::seal::WRAPPED_DEK_LEN {
        return Err(LocalStateError::Decode(format!(
            "custody row for event {} carries a {}-byte wrapped DEK, not {} — it could never \
             be unwrapped, so admitting it would restore a chart entry nobody can read or \
             crypto-shred",
            row.event_id,
            row.dek_wrapped.len(),
            cairn_event::seal::WRAPPED_DEK_LEN
        )));
    }
    Ok(row)
}

/// One row of the append-only actor registry (`actor_event`, db/004), as it travels in
/// [`LocalState::actor_registry`] (Task 11 / #500).
///
/// Mirrors `actor_event`'s columns — `recorded_at` INCLUDED, since a fix round on this same
/// task corrected the earlier "leave it behind" call: issue #99 argues for ordering by `seq`
/// over `recorded_at` (two rows from one ceremony can share a `clock_timestamp()`), never
/// for DROPPING the column. `actor_current` orders by `(recorded_at, seq)` with
/// `recorded_at` PRIMARY (db/004), so an eventual restore-side insert (slice 2d) that
/// re-stamped `clock_timestamp()` instead would permanently lose the real enrollment/
/// revocation time — an audit fact, not a detail, and cheapest to carry now while no real
/// export yet exists to be missing it. `actor_id` and `superseded_by` travel as raw bytes —
/// the content-address the pinned-determinant set hashes to — and `pinned`/`recorded_at` as
/// their TEXT source, because this crate does not enable tokio-postgres's
/// `with-serde_json-1` / chrono features (the same idiom `matcher_actor.rs` documents for
/// `pinned`).
///
/// The leaf type is a real struct, not opaque bytes, so the restore side (slice 2d) can
/// decode a row without re-deriving its shape from the SQL — the same reason [`EpisodeDek`]
/// exists rather than leaving `episode_deks`'s element shape to be discovered later.
///
/// **Additive evolution, one level down (review finding I3 on Task 11 / #500).** This struct
/// sits directly beneath [`LocalState`], which is documented at length for exactly this
/// contract — `#[serde(deny_unknown_fields)]` refuses a row from a NEWER build carrying a
/// field this one doesn't know, rather than silently dropping it, and `#[serde(default)]`
/// on the fields that are genuinely optional CONTENT (a revoke row carries no `kind` or
/// `signing_key_id`) lets a row missing one of THOSE still decode. `actor_event_id`,
/// `actor_id`, `op`, `seq` and `recorded_at` stay un-defaulted: they are the row's identity,
/// not optional content, so their absence should refuse — same reasoning as
/// `LocalState::version`.
///
/// **`recorded_at` LOST its `#[serde(default)]` in #554 slice 2d, and the reason is worth
/// keeping.** It carried one, justified as degrading *"rather than refusing the whole row
/// over one audit field."* It is not an audit field. `actor_current` (db/004) resolves the
/// trust anchor with `ORDER BY ae.actor_id, ae.recorded_at DESC, ae.seq DESC` and compares
/// revocations with `(r.recorded_at, r.seq) >= (ae.recorded_at, ae.seq)`, so `recorded_at`
/// is the PRIMARY ordering key deciding **who may author** — `seq` is only the tiebreak. A
/// restored `enroll` whose timestamp defaulted to empty-or-now would outrank a genuine older
/// `revoke` and silently re-authorise a recalled actor, arriving through
/// `restore_actor_registry`, the door built to restore the registry. That was harmless only
/// while nothing INSTALLED these rows; slice 2d installs them — which is exactly what #554
/// item 4's decode-refusal test was asked to find. Pinned by
/// `an_actor_registry_row_without_recorded_at_is_refused` in this file's tests.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ActorRegistryRow {
    pub actor_event_id: String,
    pub actor_id: Vec<u8>,
    pub op: String,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub pinned: Option<String>,
    #[serde(default)]
    pub signing_key_id: Option<String>,
    #[serde(default)]
    pub superseded_by: Option<Vec<u8>>,
    pub seq: i64,
    /// Required — see the struct doc. A defaulted value here can re-authorise a
    /// recalled actor.
    pub recorded_at: String,
}

/// Shape a decoded registry set into the JSON array `restore_actor_registry` (db/052) parses.
/// **Pure**, so the encoding can be tested without a database — which matters because a
/// mis-encoded `actor_id` would be refused by the door mid-disaster, and the only way to see
/// that coming is to assert the encoding on its own.
///
/// `actor_id` and `superseded_by` travel as HEX, because the door decodes them through
/// `cairn_decode_hex_or_raise` — the one helper that names the field it could not read
/// instead of raising `invalid input syntax for type bytea` at an operator holding their
/// only copy of a registry.
///
/// `pinned` travels as a JSON **string** holding JSON source, matching how this crate carries
/// it everywhere else (it does not enable tokio-postgres's `with-serde_json-1` feature); the
/// door casts it back with `::JSONB`.
///
/// Built with `serde_json` rather than `format!`, deliberately: a `signing_key_id` or a
/// `pinned` blob containing a quote would otherwise produce a payload the door cannot parse,
/// and the failure would arrive as a JSON syntax error naming no row at all.
pub fn actor_registry_rows_to_json(rows: &[ActorRegistryRow]) -> String {
    let values: Vec<serde_json::Value> = rows
        .iter()
        .map(|r| {
            let mut o = serde_json::Map::new();
            o.insert("actor_event_id".into(), r.actor_event_id.clone().into());
            o.insert("actor_id".into(), hex::encode(&r.actor_id).into());
            o.insert("op".into(), r.op.clone().into());
            o.insert("seq".into(), r.seq.into());
            o.insert("recorded_at".into(), r.recorded_at.clone().into());
            // The optional columns are OMITTED when absent rather than sent as JSON null:
            // the door reads them with `->>`, which maps a JSON null to SQL NULL anyway, but
            // an omitted key makes "this row had no kind" and "this row had a null kind" the
            // same statement, which for `actor_event` they are.
            if let Some(kind) = &r.kind {
                o.insert("kind".into(), kind.clone().into());
            }
            if let Some(pinned) = &r.pinned {
                o.insert("pinned".into(), pinned.clone().into());
            }
            if let Some(key) = &r.signing_key_id {
                o.insert("signing_key_id".into(), key.clone().into());
            }
            if let Some(by) = &r.superseded_by {
                o.insert("superseded_by".into(), hex::encode(by).into());
            }
            serde_json::Value::Object(o)
        })
        .collect();
    serde_json::Value::Array(values).to_string()
}

/// Serialize one actor-registry row for the export slot. Pure.
pub fn actor_registry_row_to_cbor(r: &ActorRegistryRow) -> Vec<u8> {
    let mut out = Vec::new();
    ciborium::into_writer(r, &mut out).expect("CBOR serialization of ActorRegistryRow cannot fail");
    out
}

/// Parse one actor-registry row from the export slot. Errors, never panics — same reasoning
/// as [`episode_dek_from_cbor`]: a restore reading a bit-rotted or foreign element must
/// degrade honestly rather than abort mid-recovery. Unlike that function there is no
/// fixed-length invariant to check here — an actor-registry row has no field whose wrong
/// length makes it provably useless the way a mis-sized wrapped DEK is.
pub fn actor_registry_row_from_cbor(bytes: &[u8]) -> Result<ActorRegistryRow, LocalStateError> {
    ciborium::from_reader(bytes).map_err(|e| LocalStateError::Decode(e.to_string()))
}

/// Serialize a bundle to CBOR. Pure. (No magic header — the bundle is always carried
/// INSIDE a sealed container, which has its own magic; this is the plaintext that gets
/// encrypted.)
pub fn to_cbor(ls: &LocalState) -> Vec<u8> {
    let mut out = Vec::new();
    ciborium::into_writer(ls, &mut out).expect("CBOR serialization of LocalState cannot fail");
    out
}

/// Parse a bundle from CBOR. Errors (never panics) on a malformed body, an UNKNOWN field
/// (a newer-format bundle — `deny_unknown_fields`), or a `version` this build cannot fully
/// honour. All three refuse rather than silently drop content (review finding A7c).
pub fn from_cbor(bytes: &[u8]) -> Result<LocalState, LocalStateError> {
    let ls: LocalState =
        ciborium::from_reader(bytes).map_err(|e| LocalStateError::Decode(e.to_string()))?;
    if ls.version > SUPPORTED_LOCAL_STATE_VERSION {
        return Err(LocalStateError::Decode(format!(
            "local-state bundle version {} exceeds supported {} — this build may not understand \
             all of its content; upgrade cairn-node to restore it (refusing rather than dropping)",
            ls.version, SUPPORTED_LOCAL_STATE_VERSION
        )));
    }
    Ok(ls)
}

/// The dual-wraps of a long-lived local-state DEK (LSK), established ONCE at provisioning
/// (the can't-retrofit day-one piece). A random 32-byte LSK is wrapped under a KEK from the
/// operational passphrase AND a KEK from the recovery code; either secret recovers it.
/// This is the `.lsk` sidecar's payload. `Debug` is intentionally NOT derived (mirrors
/// `SealedKey`) so a stray `{:?}` cannot dump wrapped key material.
#[derive(Clone, Serialize, Deserialize)]
pub struct LskWraps {
    argon: ArgonParams,
    salt_op: [u8; 16],
    salt_rec: [u8; 16],
    wrap_op: Wrap,
    wrap_rec: Wrap,
}

/// A sealed local-state export: the stable LSK wraps PLUS this export's freshly-encrypted
/// bundle. Self-contained — an off-site restore needs only this (the recovery code unwraps
/// the LSK, which decrypts the payload). `Debug` deliberately not derived.
#[derive(Clone, Serialize, Deserialize)]
pub struct SealedLocalState {
    wraps: LskWraps,
    payload_nonce: [u8; 24],
    payload_ct: Vec<u8>,
}

impl SealedLocalState {
    /// Assemble a sealed export from its three parts.
    ///
    /// #511 rides-along 3, and worth stating precisely so nobody reads more into it than it
    /// gives: this does **not** make a mismatched pairing impossible — node A's wraps with
    /// node B's payload is still expressible, because nothing here can tell whose LSK sealed
    /// which ciphertext. What it removes is the ability to reach that state by poking one
    /// `pub` field of an otherwise-consistent value. A caller now has to name all three parts
    /// together, which is a deliberate, reviewable act rather than an edit that looks local.
    pub fn new(wraps: LskWraps, payload_nonce: [u8; 24], payload_ct: Vec<u8>) -> Self {
        SealedLocalState {
            wraps,
            payload_nonce,
            payload_ct,
        }
    }

    /// The LSK wraps this export was sealed under.
    pub fn wraps(&self) -> &LskWraps {
        &self.wraps
    }

    /// The AEAD nonce for the sealed payload.
    pub fn payload_nonce(&self) -> &[u8; 24] {
        &self.payload_nonce
    }

    /// The sealed bundle ciphertext.
    pub fn payload_ct(&self) -> &[u8] {
        &self.payload_ct
    }
}

/// Establish the long-lived local-state DEK and dual-wrap it. Called ONCE at provisioning
/// (`init`/`seal-key`/`establish-local-state-key`) when BOTH secrets are in hand. The LSK
/// itself is discarded after wrapping — every later export re-derives it from the op-pass.
/// Reuses `seal::wrap_dek` (the same audited Argon2id+AEAD wrap the signing key uses).
pub fn establish_lsk(op_pass: &str, recovery_code: &str) -> Result<LskWraps, LocalStateError> {
    let argon = ArgonParams::default();
    // The LSK is discarded after wrapping (every later export re-derives it from the
    // op-pass), so hold it in `Secret32` — it must not linger on the stack afterwards.
    let lsk =
        Secret32::from_bytes(rand_bytes::<32>().map_err(|e| LocalStateError::Seal(e.to_string()))?);
    let salt_op = rand_bytes::<16>().map_err(|e| LocalStateError::Seal(e.to_string()))?;
    let salt_rec = rand_bytes::<16>().map_err(|e| LocalStateError::Seal(e.to_string()))?;
    let wrap_op = seal::wrap_dek(&lsk, op_pass, &salt_op, &argon)
        .map_err(|e| LocalStateError::Seal(e.to_string()))?;
    // Normalize the recovery code so any spacing/case the human re-types still unseals.
    let wrap_rec = seal::wrap_dek(
        &lsk,
        &normalize_recovery_code(recovery_code),
        &salt_rec,
        &argon,
    )
    .map_err(|e| LocalStateError::Seal(e.to_string()))?;
    Ok(LskWraps {
        argon,
        salt_op,
        salt_rec,
        wrap_op,
        wrap_rec,
    })
}

/// Seal the current bundle for export: unwrap the LSK with the op-pass (the unattended,
/// runtime-available secret), then AEAD-encrypt the bundle under the LSK with a fresh nonce.
/// The wraps are carried through unchanged (stable across exports — ADR-0026 point 5).
/// Errors if the op-pass cannot unwrap the LSK (never seals under a wrong/garbage key).
pub fn seal_local_state(
    wraps: &LskWraps,
    op_pass: &str,
    bundle: &[u8],
) -> Result<SealedLocalState, LocalStateError> {
    let lsk = seal::try_unwrap(&wraps.wrap_op, op_pass, &wraps.salt_op, &wraps.argon).ok_or_else(
        || {
            LocalStateError::Seal(
                "operational passphrase did not unwrap the local-state key".into(),
            )
        },
    )?;
    let payload_nonce = rand_bytes::<24>().map_err(|e| LocalStateError::Seal(e.to_string()))?;
    let payload_ct = aead_encrypt(lsk.as_bytes(), &payload_nonce, bundle)
        .map_err(|_| LocalStateError::Seal("aead".into()))?;
    Ok(SealedLocalState {
        wraps: wraps.clone(),
        payload_nonce,
        payload_ct,
    })
}

/// Recover the bundle via the operational passphrase (re-export / self-verify path).
pub fn unseal_local_state_op(s: &SealedLocalState, op_pass: &str) -> Option<Zeroizing<Vec<u8>>> {
    let lsk = seal::try_unwrap(&s.wraps.wrap_op, op_pass, &s.wraps.salt_op, &s.wraps.argon)?;
    // Zeroizing at the SOURCE, not at each call site. This buffer is the decrypted
    // bundle CBOR, which since ADR-0066 decision 3 holds the node's raw X25519 custody
    // secret in the clear. Returning a bare `Vec<u8>` made wiping a discipline every
    // caller had to remember. (`seal::try_unwrap` itself returns a `Secret32` since #511, which
    // wipes itself; this wrapper is for the variable-length CBOR it decrypts, which cannot be
    // a `Secret32` — see the module header.)
    aead_decrypt(lsk.as_bytes(), &s.payload_nonce, &s.payload_ct).map(Zeroizing::new)
}

/// Recover the bundle via the recovery code (the disaster-recovery path — the only
/// guaranteed-available secret after total disk loss). The code is normalized first.
pub fn unseal_local_state_rec(
    s: &SealedLocalState,
    recovery_code: &str,
) -> Option<Zeroizing<Vec<u8>>> {
    let lsk = seal::try_unwrap(
        &s.wraps.wrap_rec,
        &normalize_recovery_code(recovery_code),
        &s.wraps.salt_rec,
        &s.wraps.argon,
    )?;
    // Zeroizing at the SOURCE, not at each call site. This buffer is the decrypted
    // bundle CBOR, which since ADR-0066 decision 3 holds the node's raw X25519 custody
    // secret in the clear. Returning a bare `Vec<u8>` made wiping a discipline every
    // caller had to remember. (`seal::try_unwrap` itself returns a `Secret32` since #511, which
    // wipes itself; this wrapper is for the variable-length CBOR it decrypts, which cannot be
    // a `Secret32` — see the module header.)
    aead_decrypt(lsk.as_bytes(), &s.payload_nonce, &s.payload_ct).map(Zeroizing::new)
}

/// Serialize a sealed export to magic-prefixed CBOR for the `CAIRNL1` sibling file. Pure.
pub fn serialize_container(s: &SealedLocalState) -> Vec<u8> {
    let mut out = CONTAINER_MAGIC.to_vec();
    ciborium::into_writer(s, &mut out).expect("CBOR serialization of SealedLocalState cannot fail");
    out
}

/// Parse a `CAIRNL1` container. Errors (never panics) on bad magic / malformed body.
pub fn parse_container(bytes: &[u8]) -> Result<SealedLocalState, LocalStateError> {
    let body = bytes
        .strip_prefix(CONTAINER_MAGIC)
        .ok_or_else(|| LocalStateError::Decode("missing CAIRNL1 magic".into()))?;
    ciborium::from_reader(body).map_err(|e| LocalStateError::Decode(e.to_string()))
}

/// Confirm a just-written `CAIRNL1` export reads back SOUND, not merely well-framed (#500
/// slice 2c Task 12 fix round 1, Minor 2). `written` is the buffer the caller just handed
/// `fsio::atomic_write`; `readback` is what a fresh `std::fs::read` of that same path
/// returned a moment later; `op_pass` is the SAME operator passphrase that just sealed it.
///
/// Two checks, cheapest first:
/// 1. **Byte-identical.** A torn write, a filesystem that lied about `atomic_write`'s
///    rename, or a disk that corrupts on read-back all show up here for free — no crypto
///    needed, and this alone catches every corruption a `diff` would catch.
/// 2. **Actually unseals.** `parse_container` succeeding only proves the bytes are
///    well-formed CBOR; a single bit flipped inside `payload_ct` still decodes as a valid
///    (if numerically different) `SealedLocalState` and would sail through a framing-only
///    check, failing only much later, at restore, when the operator can least afford it.
///    Unsealing under the op-pass that sealed it moments ago is a real exercise of the
///    exact operation `restore`/`status` will perform, so it catches the class check 1
///    cannot: a container whose `written` and `readback` bytes DO match, but whose
///    ciphertext was never actually recoverable in the first place (e.g. a caller error
///    upstream that sealed under the wrong secret).
///
/// Returns the parsed container on success so the caller need not re-parse.
pub fn confirm_export_readback(
    written: &[u8],
    readback: &[u8],
    op_pass: &str,
) -> Result<SealedLocalState, LocalStateError> {
    if readback != written {
        return Err(LocalStateError::Decode(
            "the export read back from disk is not byte-identical to what was written".into(),
        ));
    }
    let sealed = parse_container(readback)?;
    if unseal_local_state_op(&sealed, op_pass).is_none() {
        return Err(LocalStateError::Seal(
            "the export parses but does not unseal under the SAME op-pass that just sealed \
             it — its ciphertext is corrupt or was sealed under the wrong secret"
                .into(),
        ));
    }
    Ok(sealed)
}

/// Seal a bundle for export AND frame it as the on-disk `CAIRNL1` container, in one fallible
/// step. Combining the seal and the framing lets the `backup` caller treat the whole optional
/// export as a SINGLE degrade-on-error operation (warn + skip on failure, never abort backup).
/// Errors only if the op-pass cannot unwrap the LSK or AEAD fails — never frames a container
/// under a wrong/garbage key.
pub fn build_export_container(
    wraps: &LskWraps,
    op_pass: &str,
    bundle: &LocalState,
) -> Result<Vec<u8>, LocalStateError> {
    // `Zeroizing` is load-bearing here since #495, and it was not before. The bundle now
    // carries a RAW X25519 unwrap secret (`LocalState::unwrap_secret`), so this CBOR
    // plaintext is a full copy of it in the clear; without the wrapper it would be dropped
    // unwiped the moment `seal_local_state` returned, leaving the node's custody key
    // readable in freed heap for anything that later reads that memory (a core dump, a swap
    // file, a heap-spray). Same reasoning, and the same issues (#46/#54), that made
    // `seal::try_unwrap` return `Zeroizing` for the LSK.
    //
    // RESIDUAL, stated rather than implied: `to_cbor` builds its `Vec` by GROWING it, so any
    // reallocation during serialization frees an intermediate buffer that still holds part of
    // the secret and that nothing can reach to wipe. Wiping the final buffer is a real
    // reduction, not a guarantee — the guarantee needs a serializer writing into a
    // pre-sized zeroizing buffer. Tracked in #508.
    let plaintext = Zeroizing::new(to_cbor(bundle));
    let sealed = seal_local_state(wraps, op_pass, &plaintext)?;
    Ok(serialize_container(&sealed))
}

/// Serialize the LSK wraps to magic-prefixed CBOR for the `.lsk` sidecar. Pure.
pub fn serialize_sidecar(w: &LskWraps) -> Vec<u8> {
    let mut out = SIDECAR_MAGIC.to_vec();
    ciborium::into_writer(w, &mut out).expect("CBOR serialization of LskWraps cannot fail");
    out
}

/// Parse a `.lsk` sidecar. Errors on bad magic / malformed body.
pub fn parse_sidecar(bytes: &[u8]) -> Result<LskWraps, LocalStateError> {
    let body = bytes
        .strip_prefix(SIDECAR_MAGIC)
        .ok_or_else(|| LocalStateError::Decode("missing CAIRNX1 magic".into()))?;
    ciborium::from_reader(body).map_err(|e| LocalStateError::Decode(e.to_string()))
}

/// The export sibling for a backup medium: `<medium>.localstate` in the same directory,
/// so the operator carries ONE artifact off-site (ADR-0026 point 3 — "same artifact"). Pure.
pub fn localstate_path_for(medium: &Path) -> PathBuf {
    let mut name = medium
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_default();
    name.push(".localstate");
    medium.with_file_name(name)
}

/// The `.lsk` sidecar for a key file: `<key>.lsk`, sibling of the signing key. Pure.
pub fn lsk_sidecar_path_for(key: &Path) -> PathBuf {
    let mut name = key
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_default();
    name.push(".lsk");
    key.with_file_name(name)
}

/// The DB-reading producer, re-exported so every existing `localstate::read_local_state`
/// call site keeps resolving after the move.
///
/// It lives in [`crate::localstate_read`] rather than here because this module owns the
/// FORMAT (container, seal, slots) and was already past the project's 500-line file-size
/// GUIDELINE (a guideline, not a cap — `tests/patient_register_demographics.rs` records the
/// correction to the "house limit" phrasing); reading custody out of a database is a
/// different job with a different dependency.
pub use crate::localstate_read::read_local_state;

/// Where a restored node's INHERITED unwrap key is written, and at what at-rest posture.
///
/// WHY THIS IS AN ENUM RATHER THAN A PASSPHRASE PAIR. The custody key follows the SIGNING
/// key's posture, exactly as it does at `init` (see `keystore::write_unwrap_plaintext`'s doc
/// for the full argument): a node restored with `--insecure-plaintext` has no operator
/// passphrase and no recovery code in existence to seal anything under, and skipping the
/// custody key there would leave that node unable to open a single body it just inherited.
/// Modelling "which secrets exist" as a type rather than two `Option`s makes the impossible
/// combination — a recovery code but no passphrase — unrepresentable.
///
/// The lifetime is borrowed throughout: the caller owns the operator secrets (in `Zeroizing`,
/// so they are wiped when the restore ceremony ends) and this type never copies them.
///
/// **`Debug` is deliberately NOT derived**, the same rule [`LskWraps`] states for itself.
/// This type holds the restored node's operator passphrase AND its recovery code as plain
/// `&str`. A derived `Debug` would print both, in full, into any log line, panic message or
/// failing `assert_eq!` that happened to include a destination. If a future change needs one,
/// hand-write it and redact both fields, exactly as [`LocalState`] does for its secret.
pub enum CustodyKeyDestination<'a> {
    /// Sealed under the RESTORED node's own freshly-minted secrets — never the dead node's.
    /// The operator carries ONE passphrase and ONE recovery code forward from the restore
    /// ceremony (ADR-0066 decision 1); the dead disk's secrets are not part of the new node.
    Sealed {
        path: &'a Path,
        op_pass: &'a str,
        recovery_code: &'a str,
    },
    /// Written unsealed (mode 0600) — only on `restore --insecure-plaintext`, where no
    /// operator secret exists. Never for a real node: the escrow does not exist for a
    /// plaintext key, so key loss is record loss.
    Plaintext { path: &'a Path },
}

impl<'a> CustodyKeyDestination<'a> {
    /// The keystore file this destination writes.
    pub fn path(&self) -> &'a Path {
        match self {
            Self::Sealed { path, .. } | Self::Plaintext { path } => path,
        }
    }

    /// Write `secret` here and PROVE it reads back before returning.
    ///
    /// The read-after-write check is not ceremony, and it is the same one
    /// `keystore::generate_unwrap_sealed` performs for the same reason: the caller's very
    /// next act is to register this key's PUBLIC half in `node_unwrap_key`, which is a
    /// singleton whose registrar then refuses any different key forever. Registering a half
    /// whose secret is not actually recoverable from disk — a truncated write, a lying
    /// fsync, a serialization edge — would leave the restored node unable to open the custody
    /// it just inherited, with every surface reporting success. That is the ADR-0066 failure
    /// shape reintroduced one layer up, at the one moment there is no second copy to go back
    /// to.
    ///
    /// BOTH recipients are verified on the sealed path. A bundle that opens under the
    /// passphrase but not the recovery code is half an escrow, and a node cannot tell from
    /// the outside — the operator would discover it only on the *next* disaster. The cost is
    /// three Argon2 derivations (the agnostic loader tries the passphrase recipient first),
    /// paid once per restore ceremony, which is negligible beside the restore itself.
    pub fn install(&self, secret: &Secret32) -> anyhow::Result<()> {
        match self {
            Self::Sealed {
                path,
                op_pass,
                recovery_code,
            } => {
                crate::keystore::write_unwrap_sealed(path, secret, op_pass, recovery_code)?;
                verify_installed(path, Some(op_pass), secret, "operator passphrase")?;
                verify_installed(path, Some(recovery_code), secret, "recovery code")?;
            }
            Self::Plaintext { path } => {
                crate::keystore::write_unwrap_plaintext(path, secret)?;
                verify_installed(path, None, secret, "unsealed read")?;
            }
        }
        Ok(())
    }
}

/// One arm of [`CustodyKeyDestination::install`]'s read-after-write check: re-read the file
/// under `reader` and confirm the recovered bytes are the ones we meant to install.
fn verify_installed(
    path: &Path,
    reader: Option<&str>,
    expected: &Secret32,
    recipient: &str,
) -> anyhow::Result<()> {
    let readback = crate::keystore::load_unwrap_secret(path, reader).map_err(|e| {
        anyhow::anyhow!(
            "inherited unwrap key written to {} but unreadable via its {recipient}: {e} \
             — refusing to register a custody key this node cannot open",
            path.display()
        )
    })?;
    if readback != *expected {
        anyhow::bail!(
            "inherited unwrap key written to {} reads back DIFFERENT bytes via its \
             {recipient} — refusing to register a custody key this node cannot open",
            path.display()
        );
    }
    Ok(())
}

/// What [`apply_local_state`] actually did, so the caller can TELL THE OPERATOR.
///
/// This is a return value rather than a set of `println!`s inside the applier for one
/// reason: a count that nobody sees is the exact failure shape this whole slice exists to
/// correct (a document whose stated precondition had expired, believed for months). The
/// restore command prints every field; a test can assert on every field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppliedLocalState {
    /// The keystore file the inherited unwrap key was written to, if the bundle carried one.
    /// `None` means the export carried no key — a real degradation the caller must warn
    /// about, not a quiet success.
    ///
    /// PRIVATE, with two named constructors below, because `restore` branches on nothing but
    /// `is_some()` to choose between "custody inherited" and the loudest warning on the whole
    /// DR path. A public field made "claims an install that did not happen" a representable
    /// state in the one type whose falsehood silences that warning.
    unwrap_key_installed: Option<PathBuf>,
    /// How many wrapped custody rows the bundle carried. **Carried, not applied** — see
    /// [`apply_local_state`].
    episode_deks_carried: usize,
    /// How many actor-registry rows the bundle CARRIED (Task 11 / #500 fix round, review
    /// finding I2). Kept beside `actor_registry_restored` rather than replaced by it: the two
    /// numbers differ on a RESUMED restore, where the export carries the whole registry and
    /// this run installs only the remainder. Reporting the carried count alone would hide the
    /// resume; reporting the restored count alone would read as data loss.
    actor_registry_carried: usize,
    /// How many of those rows this run actually INSERTED (#554 slice 2d).
    ///
    /// Lower than `actor_registry_carried` means a resume completed a prior interrupted
    /// restore — the door is set-shaped and idempotent, so it reports what IT did rather than
    /// re-claiming the set. Zero with a non-zero carried count on a FRESH database would mean
    /// the registry was already fully present, which is the same resume case at its end.
    actor_registry_restored: usize,
}

impl AppliedLocalState {
    /// The good outcome: a key was installed at `path` AND registered. Only
    /// [`apply_local_state`] can honestly say this, which is why it is the only caller.
    fn custody_inherited(
        path: PathBuf,
        episode_deks_carried: usize,
        actor_registry_carried: usize,
        actor_registry_restored: usize,
    ) -> Self {
        Self {
            unwrap_key_installed: Some(path),
            episode_deks_carried,
            actor_registry_carried,
            actor_registry_restored,
        }
    }

    /// The degraded outcome: the bundle carried no unwrap key, so none was installed and none
    /// registered. Named rather than expressed as `None`, so the caller's warning branch is
    /// reached by a stated fact instead of an inferred one.
    fn no_custody_key(
        episode_deks_carried: usize,
        actor_registry_carried: usize,
        actor_registry_restored: usize,
    ) -> Self {
        Self {
            unwrap_key_installed: None,
            episode_deks_carried,
            actor_registry_carried,
            actor_registry_restored,
        }
    }

    /// The keystore file the inherited key was written to, or `None` if the bundle carried
    /// none.
    ///
    /// **Callers MUST warn on `None`.** It is not a quiet success: with no key installed and
    /// none registered, the restored node can neither open an inherited sealed body nor
    /// author a new one, and the operator's obvious next move (`establish-unwrap-key`) would
    /// foreclose the real key permanently on the singleton registrar.
    pub fn unwrap_key_installed(&self) -> Option<&Path> {
        self.unwrap_key_installed.as_deref()
    }

    /// How many custody rows travelled. Carried, not applied (#500).
    pub fn episode_deks_carried(&self) -> usize {
        self.episode_deks_carried
    }

    /// How many actor-registry rows travelled in the export.
    ///
    /// Since #554 slice 2d they are also INSTALLED — see [`Self::actor_registry_restored`]
    /// for how many this run actually inserted, and why the two numbers are kept apart.
    pub fn actor_registry_carried(&self) -> usize {
        self.actor_registry_carried
    }

    /// How many actor-registry rows this run INSERTED into `actor_event` (#554 slice 2d).
    ///
    /// Report this to the operator alongside [`Self::actor_registry_carried`], never instead
    /// of it: a resumed restore legitimately carries N and installs fewer, and either number
    /// shown alone tells a false story about what happened.
    pub fn actor_registry_restored(&self) -> usize {
        self.actor_registry_restored
    }
}

/// The recovered X25519 unwrap secret from a restored bundle, or `None` if it carries none.
///
/// **This function used to also REFUSE a wrong-length slot, and that refusal has not been
/// dropped — it MOVED, one layer earlier.** Since #511 the slot is a [`Secret32`], whose
/// hand-written `Deserialize` accepts exactly 32 elements and otherwise fails
/// [`from_cbor`] with a message naming the expected length. So a malformed secret can no
/// longer reach this function at all: the bundle does not parse.
///
/// That placement is strictly stronger, for the reason the old doc here already gave.
/// `apply_local_state` writes the keystore file BEFORE `cairn_register_unwrap_key` is
/// consulted, so the registrar cannot catch a malformed secret — by the time it would refuse,
/// the file is already on disk. Refusing at the parse boundary means nothing is written *or*
/// read out of the bundle first.
///
/// `None` remains a legitimate answer, and a different one from a refusal: an ABSENT secret is
/// an older export (written before ADR-0066), which the caller must WARN about rather than
/// treat as a success.
pub fn recovered_unwrap_secret(ls: &LocalState) -> Option<&Secret32> {
    ls.unwrap_secret.as_ref()
}

/// Prove the recovered secret is the key this bundle's custody is actually wrapped to —
/// the one check that distinguishes a *well-formed* 32 bytes from a *correct* 32 bytes
/// (review finding I4).
///
/// ⚠️ **READ THIS BEFORE CONCLUDING #511's NEWTYPES MADE THIS FUNCTION REDUNDANT.** They did
/// not, and the reason is worth having in front of you, because the argument that used to stand
/// here — "every key in this plane is a bare `[u8; 32]`, so the compiler cannot tell them
/// apart" — was made obsolete by that very slice and is exactly the kind of expired premise
/// that gets a live check deleted.
///
/// What the types now close: the ACCIDENTAL public-for-secret substitution.
/// `cairn_event::seal::unwrap_public` returns a `PublicKey32`, and no signature in the custody
/// plane accepts one where a [`Secret32`] belongs, so `install(&unwrap_public(&secret))` is a
/// compile error.
///
/// What they deliberately do NOT close, and why this trial-unwrap is still the only proof:
/// - **Secret-for-secret.** An unwrap secret, an Ed25519 signing seed and a DEK are all
///   `Secret32` (a stated design residual — see `cairn_event::keys`'s header). A bundle
///   carrying this node's SIGNING SEED in the custody slot is well-formed at every type.
/// - **Another node's key.** Perfectly valid 32 bytes, perfectly wrong for this record.
/// - **The wire.** `from_cbor` decodes an untyped CBOR array into a `Secret32`; a foreign or
///   hand-built bundle can put a public half in that slot and it deserializes without complaint,
///   because `Secret32::from_bytes` accepts any 32 bytes. The compiler never sees that path.
///
/// And [`recovered_unwrap_secret`] cannot help: since #511 it performs no check at all — the
/// length refusal moved into `Secret32`'s `Deserialize`, one layer earlier. The read-after-write
/// checks in [`CustodyKeyDestination::install`] do not close it either: they prove the file holds
/// *the bytes we wrote*, never *a key that opens anything*. So a wrong-but-well-formed key would
/// install cleanly, register cleanly, and be discovered only when someone tried to read a chart.
/// By then `node_unwrap_key` is a singleton holding the wrong key, and the right one is refused
/// forever.
///
/// The proof is cheap and total: unwrap one carried DEK. If the secret is the right one it
/// succeeds; if it is the public half, another node's key, or corrupted-but-32-bytes, the
/// AEAD tag fails. One X25519 plus one AEAD open, once per restore.
///
/// **When it cannot prove anything, it says so by doing nothing.** An export carrying no
/// custody rows — a node that never wrote a sealed body — offers nothing to test against.
/// That is a legitimate bundle and must restore, so this returns `Ok(())`. Manufacturing a
/// verdict from an absence of evidence is the failure principle 4 exists to forbid; the
/// caller reports the carried count either way, so the operator can see which case they are
/// in.
///
/// Pure, and called BEFORE the install, so a wrong key costs nothing. That ordering is now the
/// LAST one in this path that has to be got right by hand: the length refusal it used to share
/// with [`recovered_unwrap_secret`] moved into `Secret32`'s `Deserialize`, where a malformed
/// bundle simply does not parse. This check cannot move there — it needs a carried DEK to test
/// against — so it stays here, and it stays before the write.
pub fn secret_opens_the_carried_custody(ls: &LocalState, secret: &Secret32) -> anyhow::Result<()> {
    let Some(first) = ls.episode_deks.first() else {
        return Ok(());
    };
    let row = episode_dek_from_cbor(first).map_err(|e| {
        anyhow::anyhow!(
            "the export's first custody row could not be decoded ({e}) — refusing \
                         to install a key against a bundle this node cannot read"
        )
    })?;
    cairn_event::seal::unwrap_dek(&row.dek_wrapped, secret).map_err(|_| {
        anyhow::anyhow!(
            "the unwrap secret in this export does NOT open the custody it travels with \
             (tested against the row for event {}). The bundle is internally inconsistent: \
             its key and its custody rows come from different nodes, or the secret slot holds \
             something that is not this node's X25519 secret half — every key here is 32 raw \
             bytes, so a public half or a signing seed in that slot is well-formed and still \
             useless. Refusing before anything is written: installing it would register a \
             singleton custody key that opens none of this node's record, and the registrar \
             would then refuse the real key permanently (ADR-0066).",
            row.event_id
        )
    })?;
    Ok(())
}

/// Apply a restored local-state bundle into a fresh node — **ADR-0066 decision 4, the moment
/// a restored solo clinic regains custody of its own record.**
///
/// A restored node deliberately mints a FRESH signing identity (ADR-0026 decision 4), because
/// the dead node's signing seed was never backed up. So it must ADOPT the dead node's unwrap
/// secret rather than mint one: every `event_dek` row it is about to inherit is wrapped to
/// that key, and `node_unwrap_key` is a singleton whose registrar refuses a differing key —
/// mint here and the node can never open its own record, irreversibly.
///
/// What this does, in the order it must happen:
///
/// 1. **Refuse what it cannot apply**, before touching anything. `node_default_deks`,
///    `config` and `drafts` have no store anywhere in the built system; a bundle from a newer
///    node that carries them must fail loudly rather than have them silently dropped.
/// 2. **Validate the recovered secret** ([`recovered_unwrap_secret`]) — a malformed one is
///    refused before a byte is written, because the file write is not reversible here.
/// 3. **Install it** at `custody`, proving it reads back (see
///    [`CustodyKeyDestination::install`]).
/// 4. **Register its public half**, so the restored node's custody IS the dead node's
///    custody. Registration comes last of the three because it is the step that can never be
///    taken back.
///
/// # What is CARRIED but not APPLIED, and why that is honest rather than lossy
///
/// [`LocalState::episode_deks`] is counted and reported, not inserted. The rows belong to
/// clinical events, and while the backup medium HAS carried those since DR slice 2c, nothing
/// on this side reads them back yet: `restore` applies the federation plane only, so there is
/// still no restored clinical event here for these rows to be custody OF. The door that
/// changes that is slice 2d's (**#500** stays open until it lands), and it is the same door
/// that must insert this custody — building a second one here, ahead of it, would be waste.
/// The count travels back to the caller in [`AppliedLocalState::episode_deks_carried`] and the
/// restore command PRINTS it: an operator is told what came across and what is still owed,
/// rather than finding out on the next disaster.
///
/// [`LocalState::actor_registry`] gets the SAME treatment, for the SAME reason (Task 11's own
/// fix round, review finding I2): counted in [`AppliedLocalState::actor_registry_carried`] and
/// printed, never inserted — `actor_event` has no INSERT door here yet either, and that door
/// is slice 2d's, alongside the clinical-event apply it must precede.
///
/// The ordering note for whoever lands #500's restore half: custody must be registered BEFORE
/// clinical events apply, because the door wraps each event's DEK to the registered public
/// half. This function already does its half in that order; the *caller* currently runs it
/// after `finalize_identity`, which is fine only while no clinical event is APPLIED. The
/// medium has carried them since slice 2c, so the trigger for moving this call site up is the
/// slice that starts applying them, not the one that started capturing them.
pub async fn apply_local_state(
    db: &tokio_postgres::Client,
    ls: &LocalState,
    custody: &CustodyKeyDestination<'_>,
) -> anyhow::Result<AppliedLocalState> {
    // Step 1 — the refusal that must never soften into a silent accept. Named per slot, so
    // the operator learns WHAT was withheld rather than that "something" was.
    let mut unappliable: Vec<&str> = Vec::new();
    if !ls.node_default_deks.is_empty() {
        unappliable.push("node_default_deks");
    }
    if ls.config.is_some() {
        unappliable.push("config");
    }
    if !ls.drafts.is_empty() {
        unappliable.push("drafts");
    }
    if !unappliable.is_empty() {
        anyhow::bail!(
            "restored local-state bundle carries content this node version cannot apply \
             ({}); refusing to silently drop it. No store exists here for those slots — the \
             bundle was probably written by a newer cairn-node.",
            unappliable.join(", ")
        );
    }

    // Step 2 — validate before writing. See `recovered_unwrap_secret` for why the order is
    // load-bearing rather than stylistic. ONE check happens here now: that the secret (when the
    // bundle carries custody to test against) actually OPENS that custody — what separates
    // well-formed from correct, see `secret_opens_the_carried_custody`. The length check that
    // used to be its weaker first half moved into `Secret32`'s `Deserialize` in #511, so a
    // wrong-length secret never survives `from_cbor` to reach this point.
    let secret = recovered_unwrap_secret(ls);
    if let Some(secret) = secret {
        secret_opens_the_carried_custody(ls, secret)?;
    }

    // Steps 3 and 4 — install, then register. Never the other way round: a registered public
    // half whose secret is not on disk is unrecoverable (the singleton registrar refuses the
    // real key afterwards), whereas a written file with no registration is fixed by re-running.
    let unwrap_key_installed = match secret {
        Some(secret) => {
            custody.install(secret)?;
            let public = cairn_event::seal::unwrap_public(secret);
            db.execute(
                "SELECT cairn_register_unwrap_key($1)",
                &[&public.as_bytes().as_slice()],
            )
            .await
            // The registrar refuses a DIFFERING key (db/037), and that refusal arrives AFTER
            // the file is on disk — the ordering above is deliberate and right for the
            // ordinary case, but it means this particular failure leaves the node in a state
            // no raw Postgres string explains: a custody FILE holding the dead node's key
            // beside a REGISTRATION holding some other one. The reachable cause is specific
            // and worth naming, because the remedy follows from it: somebody ran
            // `establish-unwrap-key` against this database before restoring into it, which
            // registered a key derived from the NEW signing seed.
            .map_err(|e| {
                anyhow::anyhow!(
                    "the inherited custody key was written to {} but this database ALREADY \
                     REGISTERED a different one, and `node_unwrap_key` is a singleton whose \
                     registrar refuses a differing key ({e}). The likely cause is \
                     `cairn-node establish-unwrap-key` having been run against this database \
                     before the restore, which registers a key derived from the NEW signing \
                     seed — the registration is the wrong one, the file just written is the \
                     right one. Restore again from the SAME medium into a DIFFERENT, freshly \
                     created database and do NOT run `establish-unwrap-key` on it first; a \
                     second superseding identity is auditable and expected. The file at {} \
                     holds the dead node's real key: keep it.",
                    custody.path().display(),
                    custody.path().display()
                )
            })?;
            Some(custody.path().to_path_buf())
        }
        None => None,
    };

    // Step 5 — INSTALL the carried actor registry (#554 slice 2d).
    //
    // WHY IT IS HERE AND NOT LATER. Every clinical apply door gates on `actor_current`
    // (db/004), so without this a restored node refuses its own history and "restored" means
    // a node with no patients. It runs BEFORE the clinical plane is applied, which is the
    // whole reason design §3 reorders the restore ceremony to put `finalize_identity` last.
    //
    // WHY IT IS HERE AND NOT EARLIER: the unwrap key installs first (steps 3–4) because the
    // clinical apply that follows needs custody registered before it can wrap a single DEK.
    //
    // WHAT AUTHENTICATES THESE ROWS: the `CAIRNL1` container's AEAD, and nothing else — the
    // one part of a restore that is not verify-on-apply, accepted deliberately (ADR-0067) and
    // PRINTED to the operator by the caller, because a limitation that lives only in a design
    // doc is one nobody finds.
    //
    // A DECODE FAILURE REFUSES THE WHOLE SET rather than skipping the row. The registry is
    // not a bag of independent facts: a dropped `revoke` silently re-authorises a recalled
    // actor, so admitting "most of" a registry is strictly worse than admitting none of it
    // and telling the operator which row is unreadable.
    let mut registry: Vec<ActorRegistryRow> = Vec::with_capacity(ls.actor_registry.len());
    for (i, raw) in ls.actor_registry.iter().enumerate() {
        registry.push(actor_registry_row_from_cbor(raw).map_err(|e| {
            anyhow::anyhow!(
                "the restored local-state export carries an actor-registry row (#{i}) this \
                 build cannot decode ({e}) — refusing to install a PARTIAL registry. A \
                 registry is not a bag of independent facts: a dropped revoke silently \
                 re-authorises a recalled actor. Restore from an export this build can read.",
                i = i,
                e = e
            )
        })?);
    }
    let actor_registry_restored = if registry.is_empty() {
        0
    } else {
        let payload = actor_registry_rows_to_json(&registry);
        // `$1::text::jsonb`, not `$1::jsonb`: tokio-postgres infers a parameter's type from
        // its cast TARGET, and this crate does not enable the `with-serde_json-1` feature, so
        // the payload travels as TEXT and Postgres does the parse.
        let row = db
            .query_one(
                "SELECT restore_actor_registry($1::text::jsonb)",
                &[&payload],
            )
            .await
            .map_err(|e| {
                anyhow::anyhow!(
                    "the carried actor registry could not be restored ({e}). Until it is, \
                     this node cannot apply ANY clinical event — every apply door resolves \
                     its author through `actor_current`. The database is still un-enrolled, \
                     so the same medium and export can be restored again once the cause is \
                     fixed.",
                    e = crate::db_diagnosis::legible_db_error(&e)
                )
            })?;
        let inserted: i32 = row.get(0);
        usize::try_from(inserted).unwrap_or(0)
    };

    Ok(match unwrap_key_installed {
        Some(path) => AppliedLocalState::custody_inherited(
            path,
            ls.episode_deks.len(),
            ls.actor_registry.len(),
            actor_registry_restored,
        ),
        None => AppliedLocalState::no_custody_key(
            ls.episode_deks.len(),
            ls.actor_registry.len(),
            actor_registry_restored,
        ),
    })
}

/// The `status` local-state line. Pure (presence flags injected). Honest about BOTH the
/// day-one escrow (`.lsk` present) and whether an export has been written. Absent escrow is
/// the loud case — a node accruing real content without the channel would lose it on a dead disk.
pub fn describe_local_state(lsk_present: bool, export_present: bool) -> String {
    match (lsk_present, export_present) {
        (false, _) => {
            "no local-state escrow — run `cairn-node establish-local-state-key`".to_string()
        }
        (true, false) => {
            "escrow set (dual-recipient); no export yet — run `cairn-node backup`".to_string()
        }
        (true, true) => {
            "escrow set (dual-recipient); exported alongside the last backup".to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_bundle_cbor_roundtrips() {
        let ls = LocalState::empty();
        let bytes = to_cbor(&ls);
        let back = from_cbor(&bytes).expect("an empty bundle must roundtrip");
        assert_eq!(back, ls, "roundtrip must recover the exact bundle");
        assert!(
            back.is_empty(),
            "a fresh node's bundle has no content today"
        );
    }

    // -----------------------------------------------------------------------
    // THE SERDE CONTRACT OF THE TWO CARRIED-ROW TYPES (#554 item 4, slice 2d design §2.4).
    //
    // These structs were harmless while nothing INSTALLED their rows — a dropped field cost
    // a count, not a record. Slice 2d installs both, so the contract is now load-bearing and
    // it is tested rather than merely written down: #554 asked for the decode-refusal test
    // in as many words, and the finding it turned up is `recorded_at`.
    //
    // Both directions matter and they fail differently:
    //   * a MISSING required field must refuse (a silently-defaulted `recorded_at` outranks a
    //     genuine older revoke and re-authorises a recalled actor);
    //   * an UNKNOWN field must refuse (`deny_unknown_fields`) — a row written by a NEWER
    //     build carrying a field this one does not know would otherwise have it SILENTLY
    //     DROPPED, which is the A7c failure `LocalState` guards against, one struct down.
    // -----------------------------------------------------------------------

    /// Encode an arbitrary field map as CBOR, so a test can present a row this build's own
    /// `Serialize` could never produce — an older node's row missing a field, or a newer
    /// node's row carrying one. Round-tripping through the real struct cannot express either.
    fn cbor_map(fields: &[(&str, ciborium::value::Value)]) -> Vec<u8> {
        let map = ciborium::value::Value::Map(
            fields
                .iter()
                .map(|(k, v)| (ciborium::value::Value::Text((*k).to_string()), v.clone()))
                .collect(),
        );
        let mut out = Vec::new();
        ciborium::into_writer(&map, &mut out).unwrap();
        out
    }

    fn text(s: &str) -> ciborium::value::Value {
        ciborium::value::Value::Text(s.to_string())
    }

    /// `recorded_at` REFUSES rather than defaulting — design §2.4.
    ///
    /// It is not an audit field. `actor_current` (db/004) orders on
    /// `(recorded_at DESC, seq DESC)` with `recorded_at` PRIMARY and compares revocations
    /// with `(r.recorded_at, r.seq) >= (ae.recorded_at, ae.seq)`, so it is the key that
    /// decides WHO MAY AUTHOR. A row whose timestamp defaulted to empty-or-now would outrank
    /// a genuine older `revoke` and silently re-authorise a recalled actor — arriving through
    /// the door built to restore the registry.
    #[test]
    fn an_actor_registry_row_without_recorded_at_is_refused() {
        let complete = [
            (
                "actor_event_id",
                text("aaaaaaaa-0000-7000-8000-000000000001"),
            ),
            ("actor_id", ciborium::value::Value::Bytes(vec![1, 2, 3])),
            ("op", text("enroll")),
            ("seq", ciborium::value::Value::Integer(1.into())),
            ("recorded_at", text("2026-01-01 00:00:00+00")),
        ];
        actor_registry_row_from_cbor(&cbor_map(&complete))
            .expect("anti-vacuity: the complete row must decode");

        let missing: Vec<_> = complete
            .iter()
            .filter(|(k, _)| *k != "recorded_at")
            .cloned()
            .collect();
        assert!(
            actor_registry_row_from_cbor(&cbor_map(&missing)).is_err(),
            "a registry row with no recorded_at must REFUSE, never default — it is \
             actor_current's primary ordering key, not an audit detail"
        );
    }

    /// `deny_unknown_fields` is pinned on BOTH carried-row types.
    ///
    /// Deleting the attribute must redden something, which before this test it did not. The
    /// failure it prevents is silent: a row from a NEWER build carrying a field this one does
    /// not know would decode with that field DROPPED, so a restore would install a
    /// registry row or a custody row that is a lossy shadow of what was exported — and
    /// nothing anywhere would say so.
    #[test]
    fn both_carried_row_types_refuse_an_unknown_field() {
        let registry = [
            (
                "actor_event_id",
                text("aaaaaaaa-0000-7000-8000-000000000001"),
            ),
            ("actor_id", ciborium::value::Value::Bytes(vec![1, 2, 3])),
            ("op", text("enroll")),
            ("seq", ciborium::value::Value::Integer(1.into())),
            ("recorded_at", text("2026-01-01 00:00:00+00")),
            ("custody_scope", text("a field from a newer build")),
        ];
        assert!(
            actor_registry_row_from_cbor(&cbor_map(&registry)).is_err(),
            "an unknown field on a registry row must refuse, never be dropped"
        );

        let dek = [
            ("event_id", text("11111111-1111-7111-8111-111111111111")),
            (
                "dek_wrapped",
                ciborium::value::Value::Bytes(vec![0u8; cairn_event::seal::WRAPPED_DEK_LEN]),
            ),
            ("rewrap_epoch", ciborium::value::Value::Integer(2.into())),
        ];
        assert!(
            episode_dek_from_cbor(&cbor_map(&dek)).is_err(),
            "an unknown field on a custody row must refuse, never be dropped"
        );
    }

    /// An `EpisodeDek` missing a field is refused — neither field is optional CONTENT, both
    /// are the row's identity (which event, which key).
    #[test]
    fn an_episode_dek_missing_a_field_is_refused() {
        let wrapped = ciborium::value::Value::Bytes(vec![0u8; cairn_event::seal::WRAPPED_DEK_LEN]);
        episode_dek_from_cbor(&cbor_map(&[
            ("event_id", text("11111111-1111-7111-8111-111111111111")),
            ("dek_wrapped", wrapped.clone()),
        ]))
        .expect("anti-vacuity: the complete row must decode");

        assert!(
            episode_dek_from_cbor(&cbor_map(&[("dek_wrapped", wrapped)])).is_err(),
            "a custody row with no event_id names no event and must refuse"
        );
        assert!(
            episode_dek_from_cbor(&cbor_map(&[(
                "event_id",
                text("11111111-1111-7111-8111-111111111111")
            )]))
            .is_err(),
            "a custody row with no key must refuse"
        );
    }

    #[test]
    fn from_cbor_rejects_garbage() {
        assert!(from_cbor(b"not a bundle").is_err());
    }

    #[test]
    fn older_bundle_without_a_later_field_defaults_it() {
        // Additive evolution (principle 11): a bundle serialized by an OLDER node that
        // lacks a field this node knows about must still deserialize, with the missing
        // field defaulted. We simulate "older" by constructing a ciborium Value::Map
        // omitting later fields, then serializing it to CBOR — encode a map missing `drafts`.
        let mut partial = std::collections::BTreeMap::new();
        partial.insert(
            "version".to_string(),
            ciborium::value::Value::Integer(1.into()),
        );
        // Intentionally omit node_default_deks/episode_deks/config/drafts.
        let val = ciborium::value::Value::Map(
            partial
                .into_iter()
                .map(|(k, v)| (ciborium::value::Value::Text(k), v))
                .collect(),
        );
        let mut bytes = Vec::new();
        ciborium::into_writer(&val, &mut bytes).unwrap();
        let back = from_cbor(&bytes).expect("a bundle missing later fields must still parse");
        assert!(back.is_empty(), "omitted collections default to empty");
    }

    /// Helper: encode a top-level CBOR map from (key, Value) pairs, as a newer/older writer would.
    fn encode_map(entries: Vec<(&str, ciborium::value::Value)>) -> Vec<u8> {
        let val = ciborium::value::Value::Map(
            entries
                .into_iter()
                .map(|(k, v)| (ciborium::value::Value::Text(k.to_string()), v))
                .collect(),
        );
        let mut bytes = Vec::new();
        ciborium::into_writer(&val, &mut bytes).unwrap();
        bytes
    }

    #[test]
    fn newer_bundle_with_unknown_field_is_refused_not_silently_dropped() {
        // Review fix A7c: a NEWER cairn-node adds a content-bearing field this build does
        // not know. `deny_unknown_fields` must make that a LOUD refusal — never a silent
        // drop that reports the restore a success while discarding (e.g.) episode DEKs.
        let bytes = encode_map(vec![
            ("version", ciborium::value::Value::Integer(1.into())),
            // A field from the future carrying real content this build cannot represent.
            (
                "episode_wrapped_deks_v2",
                ciborium::value::Value::Bytes(vec![1, 2, 3]),
            ),
        ]);
        let err = from_cbor(&bytes).expect_err("an unknown field must be refused, not dropped");
        assert!(
            matches!(err, LocalStateError::Decode(_)),
            "unknown field -> Decode error"
        );
    }

    #[test]
    fn bundle_version_beyond_supported_is_refused() {
        // A bundle declaring a version this build cannot fully honour must be refused rather
        // than partially applied (the version-gate half of the A7c contract).
        let bytes = encode_map(vec![(
            "version",
            ciborium::value::Value::Integer(((SUPPORTED_LOCAL_STATE_VERSION + 1) as i32).into()),
        )]);
        assert!(
            from_cbor(&bytes).is_err(),
            "a too-new version must be refused"
        );
    }

    const OP: &str = "op-pass";
    const REC: &str = "AB12C-D34EF";

    #[test]
    fn lsk_seal_then_unseal_via_both_recipients() {
        let wraps = establish_lsk(OP, REC).unwrap();
        let bundle = to_cbor(&LocalState::empty());
        let sealed = seal_local_state(&wraps, OP, &bundle).unwrap();
        // Either secret recovers the same plaintext bundle.
        assert_eq!(
            unseal_local_state_op(&sealed, OP)
                .as_deref()
                .map(Vec::as_slice),
            Some(bundle.as_slice())
        );
        assert_eq!(
            unseal_local_state_rec(&sealed, REC)
                .as_deref()
                .map(Vec::as_slice),
            Some(bundle.as_slice()),
            "the recovery code (off-node escrow) must unseal — the disaster-recovery path"
        );
    }

    #[test]
    fn lsk_unseal_rejects_wrong_secret_and_tamper() {
        let wraps = establish_lsk(OP, REC).unwrap();
        let sealed = seal_local_state(&wraps, OP, &to_cbor(&LocalState::empty())).unwrap();
        assert_eq!(
            unseal_local_state_op(&sealed, "nope"),
            None,
            "wrong op-pass => None"
        );
        assert_eq!(
            unseal_local_state_rec(&sealed, "ZZZZZ"),
            None,
            "wrong recovery code => None"
        );
        // Flip a byte of the payload ciphertext: AEAD tag must fail.
        let mut t = sealed.clone();
        t.payload_ct[0] ^= 1;
        assert_eq!(
            unseal_local_state_op(&t, OP),
            None,
            "tampered payload must fail unseal"
        );
        // The LSK wrap is where the key actually lives on disk (the real storage-attacker
        // target): a flipped wrap ciphertext must fail the unwrap's AEAD tag, not silently
        // recover a corrupted key.
        let mut t2 = sealed.clone();
        t2.wraps.wrap_op.ct[0] ^= 1;
        assert_eq!(
            unseal_local_state_op(&t2, OP),
            None,
            "tampered op-wrap must fail unseal"
        );

        let mut t3 = sealed.clone();
        t3.wraps.wrap_rec.ct[0] ^= 1;
        assert_eq!(
            unseal_local_state_rec(&t3, REC),
            None,
            "tampered rec-wrap must fail unseal"
        );
    }

    #[test]
    fn seal_local_state_needs_the_op_pass_to_unwrap_the_lsk() {
        // seal_local_state unwraps the LSK with the op-pass; a wrong op-pass cannot
        // unwrap it, so sealing must fail rather than silently produce a bundle under a
        // wrong/garbage key.
        let wraps = establish_lsk(OP, REC).unwrap();
        assert!(seal_local_state(&wraps, "wrong-op", &to_cbor(&LocalState::empty())).is_err());
    }

    use std::path::Path;

    #[test]
    fn container_roundtrips_and_has_magic() {
        let wraps = establish_lsk(OP, REC).unwrap();
        let sealed = seal_local_state(&wraps, OP, b"x").unwrap();
        let bytes = serialize_container(&sealed);
        assert!(
            bytes.starts_with(b"CAIRNL1\n"),
            "export container must carry CAIRNL1 magic"
        );
        let back = parse_container(&bytes).unwrap();
        assert_eq!(
            unseal_local_state_rec(&back, REC)
                .as_deref()
                .map(Vec::as_slice),
            Some(b"x".as_slice())
        );
    }

    #[test]
    fn build_export_container_frames_a_sealed_bundle_and_rejects_a_wrong_op_pass() {
        // The `backup` arm calls this as ONE fallible step it degrades on (warn + skip) so a
        // missing/wrong passphrase never aborts an already-complete event backup.
        let wraps = establish_lsk(OP, REC).unwrap();
        let bytes = build_export_container(&wraps, OP, &LocalState::empty())
            .expect("the right op-pass must seal + frame the export");
        assert!(
            bytes.starts_with(b"CAIRNL1\n"),
            "the built export must carry the container magic"
        );
        // The off-node recovery code still unseals the framed container to the empty bundle.
        let parsed = parse_container(&bytes).unwrap();
        let plaintext = unseal_local_state_rec(&parsed, REC).expect("recovery code must unseal");
        assert!(from_cbor(&plaintext).unwrap().is_empty());
        // A wrong op-pass cannot unwrap the LSK, so building fails rather than emitting a
        // container under a wrong/garbage key — this Err is exactly what drives the warn+skip.
        assert!(
            build_export_container(&wraps, "wrong-op", &LocalState::empty()).is_err(),
            "a wrong op-pass must fail the build, not produce a bad container"
        );
    }

    #[test]
    fn sidecar_roundtrips_and_has_magic() {
        let wraps = establish_lsk(OP, REC).unwrap();
        let bytes = serialize_sidecar(&wraps);
        assert!(
            bytes.starts_with(b"CAIRNX1\n"),
            "lsk sidecar must carry CAIRNX1 magic"
        );
        let back = parse_sidecar(&bytes).unwrap();
        // The recovered wraps still unseal an export sealed under the originals.
        let sealed = seal_local_state(&back, OP, b"y").unwrap();
        assert_eq!(
            unseal_local_state_op(&sealed, OP)
                .as_deref()
                .map(Vec::as_slice),
            Some(b"y".as_slice())
        );
    }

    #[test]
    fn parse_rejects_wrong_or_missing_magic() {
        assert!(parse_container(b"nope").is_err());
        assert!(parse_sidecar(b"nope").is_err());
        // A container's bytes are not a valid sidecar and vice-versa (distinct magics).
        let wraps = establish_lsk(OP, REC).unwrap();
        let container = serialize_container(&seal_local_state(&wraps, OP, b"z").unwrap());
        assert!(
            parse_sidecar(&container).is_err(),
            "a container must not parse as a sidecar"
        );
        // ...and the reverse: the invariant is bidirectional (distinct 8-byte magics),
        // so a sidecar's bytes must not parse as a container either.
        let sidecar = serialize_sidecar(&wraps);
        assert!(
            parse_container(&sidecar).is_err(),
            "a sidecar must not parse as a container"
        );
    }

    #[test]
    fn paths_are_deterministic_siblings() {
        assert_eq!(
            localstate_path_for(Path::new("/mnt/backup/cairn.medium")),
            Path::new("/mnt/backup/cairn.medium.localstate")
        );
        assert_eq!(
            lsk_sidecar_path_for(Path::new("/var/lib/cairn/node.key")),
            Path::new("/var/lib/cairn/node.key.lsk")
        );
    }

    #[test]
    fn describe_local_state_is_honest_about_escrow_and_export() {
        assert!(describe_local_state(false, false).contains("no local-state escrow"));
        assert!(describe_local_state(true, false).contains("escrow set"));
        assert!(describe_local_state(true, false).contains("no export yet"));
        assert!(describe_local_state(true, true).contains("exported"));
    }

    #[test]
    fn re_export_keeps_wraps_stable_but_refreshes_the_payload() {
        // ADR-0026 point 5 / Approach 1: the LSK (and thus its dual-wraps) is long-lived
        // across exports — only the payload re-encrypts. So two seals over the SAME wraps
        // must carry byte-identical wrap_op/wrap_rec (the recovery code still unseals both)
        // but DIFFERENT payload ciphertext (fresh nonce), and each unseals to its own bundle.
        let wraps = establish_lsk(OP, REC).unwrap();
        let a = seal_local_state(&wraps, OP, b"bundle-A").unwrap();
        let b = seal_local_state(&wraps, OP, b"bundle-B").unwrap();
        assert_eq!(
            a.wraps.wrap_op.ct, b.wraps.wrap_op.ct,
            "LSK op-wrap is stable across exports"
        );
        assert_eq!(
            a.wraps.wrap_rec.ct, b.wraps.wrap_rec.ct,
            "LSK rec-wrap is stable across exports"
        );
        assert_ne!(
            a.payload_ct, b.payload_ct,
            "each export re-encrypts the payload (fresh nonce)"
        );
        assert_eq!(
            unseal_local_state_rec(&a, REC)
                .as_deref()
                .map(Vec::as_slice),
            Some(b"bundle-A".as_slice())
        );
        assert_eq!(
            unseal_local_state_rec(&b, REC)
                .as_deref()
                .map(Vec::as_slice),
            Some(b"bundle-B".as_slice())
        );
    }
}
