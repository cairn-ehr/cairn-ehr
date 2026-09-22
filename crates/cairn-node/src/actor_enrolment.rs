//! The one enrolment rule: a node's signing key becomes an authoring actor by **provisioning**,
//! never as a side effect of a write.
//!
//! # Why this module exists at all
//!
//! Until [#654](https://github.com/cairn-ehr/cairn-ehr/issues/654), `main.rs` carried a private
//! `ensure_registration_actor`, called by **fifteen** write subcommands, which enrolled the
//! node's key as a `device` actor on first use. The reference window's `cairn-gui-live`
//! deliberately did **not**, because provisioning on a write path is the shape ADR-0066
//! decision 6 forbids for the unwrap key — `ensure_unwrap_key`/`submit_event` were made to
//! *refuse* rather than quietly provision (trap 2).
//!
//! The result was an asymmetry in which a node's behaviour depended on **which surface touched
//! it first**: the CLI provisioned silently, and the window's very first registration refused
//! with a message naming a key id rather than a remedy.
//!
//! One rule now, and it has three parts:
//!
//! - **`cairn-node init` enrols**, so an initialised node costs its operator no new act.
//! - **`cairn-node enroll-device-actor` is the named remedy** for a node that was not — most
//!   obviously a node restored without its actor registry, since a restore never runs `init`.
//! - **Every write path calls [`require_device_actor`] and refuses.** Nothing provisions.
//!
//! # What an enrolled device actor is, and is not
//!
//! It is the headless-node/CLI convenience: the node's own key, enrolled as a `device` with
//! role `registration-desk`, so an unattended node can author. **A real clinical UI attaches
//! the operating clerk's *human* actor instead** — that is principle 10's separable
//! accountability, and this device-key path does not stand in for it.

/// The role recorded in a device actor's pinned determinants.
///
/// Unchanged from the pre-#654 spelling on purpose. It is written into a **signed** actor
/// event, so renaming it would change bytes on the wire in order to make a command name read
/// better — which is never a trade this project takes.
const DEVICE_ACTOR_ROLE: &str = "registration-desk";

/// Where this signing key stands with the actor registry.
///
/// **Three states, not two**, and the third is the one that bites. `actor_current` excludes
/// revoked and superseded-away actors, so a key that *was* enrolled and has since been retired
/// reads exactly like a key nobody ever enrolled — and the obvious remedy for the second is a
/// dead end for the first. See [`ActorStanding::Retired`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActorStanding {
    /// Resolvable to a current actor. It may author.
    Enrolled,
    /// No row in the registry mentions this key at all. `enroll-device-actor` fixes it.
    NeverEnrolled,
    /// This key HAS registry history — enrolled once, then revoked or superseded away — and is
    /// not current.
    ///
    /// **`enroll-device-actor` cannot fix this, and must not be offered.** db/004's
    /// `cairn_actor_id_key_conflict` refuses a fresh enroll onto an `actor_id` carrying prior
    /// revoke/supersede history, deliberately: a post-revoke enroll would outrank the revoke in
    /// `actor_current`'s `(recorded_at, seq)` order and silently **resurrect a retired actor**
    /// (#152). Refusing that is correct. Sending an operator there is not — they would meet an
    /// opaque `P0001` about actor-id collisions while following the remedy we just gave them.
    Retired,
}

/// Where does this key stand? One round trip, three answers.
///
/// The `actor_event` arm keys on `signing_key_id`, which only `enroll`/`supersede` rows carry —
/// a `revoke` row has a NULL key by design (db/004) — so it answers *"was this key ever
/// enrolled?"* rather than *"was it ever revoked?"*, which is the question that distinguishes
/// `Retired` from `NeverEnrolled` once `actor_current` has already said "not now".
pub async fn device_actor_standing(
    db: &tokio_postgres::Client,
    kid: &str,
) -> anyhow::Result<ActorStanding> {
    use anyhow::Context as _;
    let standing: String = db
        .query_one(
            "SELECT CASE \
               WHEN EXISTS(SELECT 1 FROM actor_current WHERE signing_key_id = $1) THEN 'enrolled' \
               WHEN EXISTS(SELECT 1 FROM actor_event   WHERE signing_key_id = $1) THEN 'retired' \
               ELSE 'never' END",
            &[&kid],
        )
        .await
        .context("checking where this node's key stands with the actor registry")?
        .get(0);
    Ok(match standing.as_str() {
        "enrolled" => ActorStanding::Enrolled,
        "retired" => ActorStanding::Retired,
        // Fail towards the state with a remedy. The CASE above is total, so this arm is
        // unreachable; treating an impossible answer as "never" offers a command that will
        // simply refuse if it is wrong, which is the cheaper of the two mistakes.
        _ => ActorStanding::NeverEnrolled,
    })
}

/// The refusal for a key whose actor was retired. **Pure.**
///
/// Deliberately does **not** name `enroll-device-actor`: see [`ActorStanding::Retired`] for why
/// that command cannot help, and what the operator meets if they try.
pub fn retired_actor_refusal(kid: &str) -> anyhow::Error {
    crate::db_diagnosis::deliberate_refusal(format!(
        "this node's signing key {kid} was enrolled as an actor and has since been revoked or \
         superseded, so it may not author clinical events. `cairn-node enroll-device-actor` \
         will NOT help and is not the remedy: re-enrolling a retired actor id is refused on \
         purpose, because it would silently resurrect the retired actor (issue #152). This \
         node needs a NEW signing key, enrolled afresh — which is a decision about who is \
         accountable for what this node writes, not a command to run blind."
    ))
}

/// Is this signing key already resolvable to an authoring actor?
///
/// # ⚠️ KIND-AGNOSTIC, AND THAT IS NOT AN OVERSIGHT
///
/// `submit_event` resolves a signer to an actor purely by `signing_key_id` — kind matters only
/// for attestation — and if one key maps to **more than one** `actor_current` row it sets
/// `actor_id = NULL` for **every** event that key authors node-wide (db/005,
/// `array_length(v_actor_ids, 1) = 1`), silently and irreversibly degrading attribution.
///
/// A kind-scoped `AND kind = 'device'` guard would happily add a second actor to a key already
/// enrolled as (say) a matcher `agent` or a `human`, tripping exactly that dual-mapping. Keying
/// on `signing_key_id` alone means a key already usable for authoring is left untouched — never
/// split into two actors.
///
/// Public because slice 2c's window probes it **at launch**, which is the same discipline
/// `build_live_state` already follows by loading the node key up front rather than discovering
/// at sign-off that it can never seal anything.
pub async fn device_actor_enrolled(db: &tokio_postgres::Client, kid: &str) -> anyhow::Result<bool> {
    // Delegates rather than running its own `EXISTS`, so there is ONE definition of "enrolled"
    // and the kind-agnostic argument above cannot be true of one query and false of the other.
    Ok(device_actor_standing(db, kid).await? == ActorStanding::Enrolled)
}

/// Provision this key as a `device` actor. **Idempotent.** Returns whether it actually enrolled.
///
/// An **owner ceremony**: the runtime `cairn_agent` role deliberately cannot call
/// `enroll_actor`, so this runs as the operator — from `init` or from `enroll-device-actor` —
/// and never from a write path.
///
/// The `bool` is not decoration. `enroll-device-actor` prints a different sentence for *"done"*
/// than for *"there was nothing to do"*, and an operator following a crash remedy needs to know
/// which one happened.
pub async fn enroll_device_actor(db: &tokio_postgres::Client, kid: &str) -> anyhow::Result<bool> {
    use anyhow::Context as _;
    match device_actor_standing(db, kid).await? {
        ActorStanding::Enrolled => return Ok(false),
        // Refuse HERE, in words an operator can act on, rather than letting db/004's
        // `cairn_actor_id_key_conflict` raise its actor-id-collision message at somebody who
        // only did what a previous refusal told them to (PR #661 review).
        ActorStanding::Retired => return Err(retired_actor_refusal(kid)),
        ActorStanding::NeverEnrolled => {}
    }
    let pinned = serde_json::json!({ "role": DEVICE_ACTOR_ROLE, "node_key": kid }).to_string();
    db.execute(
        "SELECT enroll_actor('device', $1::text::jsonb, $2)",
        &[&pinned, &kid],
    )
    .await
    .context("enrolling this node's key as a device actor")?;
    Ok(true)
}

/// The refusal a write path gives on an unprovisioned node. **Pure.**
///
/// A [`crate::db_diagnosis::DeliberateRefusal`] (#651), because it is a verdict: the same key
/// refuses identically until somebody enrols it, so a caller offering a retry would be offering
/// one that can never work.
///
/// It names the **command**, not only the key. `submit_event`'s own refusal — *"signer 9f3c… is
/// not an enrolled, non-revoked actor"* — is true, legible, and tells nobody what to do about
/// it; the precedent for naming the remedy is `submit_event`'s unwrap-key refusal, which names
/// `establish-unwrap-key`.
pub fn not_enrolled_refusal(kid: &str) -> anyhow::Error {
    crate::db_diagnosis::deliberate_refusal(format!(
        "this node's signing key {kid} is not enrolled as an actor, so it may not author \
         clinical events. Enrolling is provisioning, not a side effect of writing: run \
         `cairn-node enroll-device-actor` once on this node. (A node created by `cairn-node \
         init` is already enrolled; a node restored without its actor registry is not.)"
    ))
}

/// Require an enrolled actor before a write, refusing if there is none. **Never provisions.**
///
/// The whole point of this function is the thing it does *not* do. A check that enrolled on
/// miss would be `ensure_registration_actor` under a new name, and the asymmetry #654 closed
/// would be back the moment anyone added a sixteenth write command.
pub async fn require_device_actor(db: &tokio_postgres::Client, kid: &str) -> anyhow::Result<()> {
    match device_actor_standing(db, kid).await? {
        ActorStanding::Enrolled => Ok(()),
        ActorStanding::NeverEnrolled => Err(not_enrolled_refusal(kid)),
        // A DIFFERENT sentence, because the remedy is different and the obvious one is a dead
        // end. See `ActorStanding::Retired`.
        ActorStanding::Retired => Err(retired_actor_refusal(kid)),
    }
}
