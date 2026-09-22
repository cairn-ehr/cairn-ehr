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
    use anyhow::Context as _;
    let enrolled: bool = db
        .query_one(
            "SELECT EXISTS(SELECT 1 FROM actor_current WHERE signing_key_id = $1)",
            &[&kid],
        )
        .await
        .context("checking whether this node's key is an enrolled actor")?
        .get(0);
    Ok(enrolled)
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
    if device_actor_enrolled(db, kid).await? {
        return Ok(false);
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
    if device_actor_enrolled(db, kid).await? {
        return Ok(());
    }
    Err(not_enrolled_refusal(kid))
}
