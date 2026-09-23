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
//! - **Every CLI write path that authors as THIS NODE calls [`require_device_actor`] and
//!   refuses.** Nothing provisions the node's own device actor.
//!
//! ⚠️ **"Every CLI write path" is literal, and the gap is the reference window.** All fifteen
//! `require_device_actor` call sites are `Cmd::` arms in `main.rs`; `cairn-gui-live` calls it
//! nowhere. The GUI still *refuses* on an unprovisioned node — db/005 sees to that — but it
//! refuses in db/005's words, naming a key rather than this module's remedy. The
//! provisioning asymmetry #654 closed is genuinely closed on both surfaces; the
//! remedy-naming half is not, and is filed as
//! [#665](https://github.com/cairn-ehr/cairn-ehr/issues/665) (PR #661 review).
//!
//! ⚠️ **That last sentence is scoped, and the scope is load-bearing.** One path in this same
//! binary still enrols on first use: `matcher_actor::resolve_matcher_actor` mints a per-epoch
//! `agent` actor from `Cmd::ApplyAutoCandidates`, which is a write path. That is arguably its
//! own ceremony — a matcher epoch is an identity, not a node — but it IS the shape #654 is
//! named after, so it is named here rather than left for someone to discover and cite as
//! precedent for re-adding enrol-on-miss.
//! [#663](https://github.com/cairn-ehr/cairn-ehr/issues/663) carries the question.
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
/// **Four states, not two**, and the ones that bite are the two that look like `NeverEnrolled`
/// and are not. `actor_current` excludes **revoked** actors, so a key that *was* enrolled and
/// has since been revoked reads exactly like a key nobody ever enrolled — and the obvious remedy
/// for the second is a dead end for the first. See [`ActorStanding::Retired`] and
/// [`ActorStanding::Ambiguous`].
///
/// # ⚠️ WHAT A SUPERSEDED KEY CLASSIFIES AS IS UNDECIDED, BECAUSE db/004 CONTRADICTS ITSELF
///
/// This classification keys on **`signing_key_id`**, not on `actor_id`. So what a superseded key
/// reads depends entirely on whether a `supersede` row carries a key — and db/004 says both:
///
/// - around line 80 (`cairn_actor_id_key_conflict`): *"revoke and supersede rows carry no
///   `signing_key_id`"*;
/// - around line 112 (`cairn_key_actor_id_conflict`): *"`op IN ('enroll','supersede')` restricts
///   to the key-bearing ops"*.
///
/// `actor_event.signing_key_id` is a bare nullable `TEXT` with no per-`op` CHECK, so the schema
/// settles nothing. Working the branches through:
///
/// - **supersede carries NULL, or carries the NEW key** — the old key matches no `actor_current`
///   row, the `actor_event` EXISTS arm fires on its surviving `enroll` row, and it reads
///   [`ActorStanding::Retired`]. Fail-closed, but the remedy is wrong: `retired_actor_refusal`
///   tells the operator their node needs a NEW signing key, which is an identity-level decision
///   handed to somebody whose node is in a state the design considers normal.
/// - **supersede carries the OLD key** (or a determinant bump that keeps the same key) — the key
///   matches two view rows and reads [`ActorStanding::Ambiguous`], refusing every write on a
///   legitimately rotated node.
///
/// **An earlier version of this doc asserted, in capitals, that a superseded key would read
/// [`ActorStanding::Enrolled`] and keep authoring. There is no branch on which that happens**,
/// and the claim is dangerous in a specific way: the natural "fix" for the symptom it described
/// is to make this classifier consult `actor_id`/`superseded_by`, which is precisely how a
/// superseded key *would* get to keep authoring. Every branch above is fail-closed today.
///
/// Note also that a supersede row is **replayable today** even though nothing authors one:
/// db/052's `restore_actor_registry` accepts `op = 'supersede'` and deliberately bypasses
/// db/004's collision guards (it replays, it does not re-adjudicate). So this is not purely a
/// future concern — a restored node can carry supersede rows now.
///
/// Whoever writes the rotate-key door must settle the convention first
/// ([#666](https://github.com/cairn-ehr/cairn-ehr/issues/666)) and then revisit this function
/// ([#664](https://github.com/cairn-ehr/cairn-ehr/issues/664)). (PR #661 review, converged on by
/// three reviewers.)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActorStanding {
    /// Resolvable to a current actor. It may author.
    Enrolled,
    /// No row in the registry mentions this key at all. `enroll-device-actor` fixes it.
    NeverEnrolled,
    /// This key HAS registry history — enrolled once, then **revoked** — and is not current.
    ///
    /// A *superseded* key may also land here, depending on a db/004 convention that is not yet
    /// settled: see the enum doc and [#666](https://github.com/cairn-ehr/cairn-ehr/issues/666).
    /// That is why `retired_actor_refusal` says "revoked **or superseded**" — the message an
    /// operator reads must not claim a precision the classification does not have.
    ///
    /// **`enroll-device-actor` cannot fix this, and must not be offered.** db/004's
    /// `cairn_actor_id_key_conflict` refuses a fresh enroll onto an `actor_id` carrying prior
    /// revoke/supersede history, deliberately: a post-revoke enroll would outrank the revoke in
    /// `actor_current`'s `(recorded_at, seq)` order and silently **resurrect a retired actor**
    /// (#152). Refusing that is correct. Sending an operator there is not — they would meet an
    /// opaque `P0001` about actor-id collisions while following the remedy we just gave them.
    Retired,
    /// This key maps to **more than one** current actor — not a state anything may author under.
    ///
    /// `submit_event` resolves a signer by `signing_key_id` alone and sets `actor_id = NULL`
    /// for **every** event a dual-mapped key authors node-wide (db/005,
    /// `array_length(v_actor_ids, 1) = 1`) — silently and irreversibly destroying attribution.
    /// So the honest answer is to refuse the write, not to let it through unattributed.
    ///
    /// Unreachable through the only **adjudicating** enrol door, `enroll_actor`: db/004's
    /// `cairn_key_actor_id_conflict` is whole-history and fails closed, and
    /// `a_key_already_enrolled_under_another_kind_is_left_alone` pins the count at one.
    ///
    /// **But that is not the only door, and this state is reachable today.** db/052's
    /// `restore_actor_registry` replays a medium's `actor_event` rows and *deliberately*
    /// bypasses those collision guards — its own header says so, because re-adjudicating would
    /// refuse the node's own legitimate revoke and supersede rows. It validates shape, not
    /// consistency. So a restored node is the concrete place an operator meets this, and
    /// `ambiguous_actor_refusal` names it. db/004 additionally anticipates a **future
    /// actor-sync apply door** (ADR-0044 §3) that must mirror the check, and nothing in
    /// `cairn-node` would notice if it did not.
    ///
    /// Pinned by `a_key_mapping_to_two_current_actors_refuses_rather_than_unattributing`
    /// (PR #661 review: this branch had no test at all, and the obvious `EXISTS` simplification
    /// deletes it with every other test still green).
    Ambiguous,
}

/// Where does this key stand? One round trip, four answers.
///
/// ⚠️ **The standing carries no SUBJECT**, so pairing it with the right key is caller discipline
/// rather than a compiler fact: `device_actor_standing(db, kid_a)` returning `Retired` and
/// `retired_actor_refusal(kid_b)` compiles and would hand an operator an identity-level remedy for
/// the wrong key. Both call sites use one binding today.
/// [#670](https://github.com/cairn-ehr/cairn-ehr/issues/670) carries the fix (return the kid
/// beside the standing).
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
            // COUNTS the current rows rather than testing existence, because `Enrolled` has to
            // mean "resolves to exactly one actor" and not merely "appears in the view" — see
            // `ActorStanding::Ambiguous` for what two rows do to attribution.
            "SELECT CASE \
               WHEN c.n = 1 THEN 'enrolled' \
               WHEN c.n > 1 THEN 'ambiguous' \
               WHEN EXISTS(SELECT 1 FROM actor_event WHERE signing_key_id = $1) THEN 'retired' \
               ELSE 'never' END \
             FROM (SELECT count(*) AS n FROM actor_current WHERE signing_key_id = $1) c",
            &[&kid],
        )
        .await
        .context("checking where this node's key stands with the actor registry")?
        .get(0);
    Ok(match standing.as_str() {
        "enrolled" => ActorStanding::Enrolled,
        "retired" => ActorStanding::Retired,
        "never" => ActorStanding::NeverEnrolled,
        "ambiguous" => ActorStanding::Ambiguous,
        // ⚠️ FAIL CLOSED AND LOUD, not towards a guess. An earlier version of this arm fell
        // through to `NeverEnrolled` on the argument that the CASE is total so the arm is
        // unreachable, and that offering a command "that will simply refuse if it is wrong" is
        // the cheaper mistake. **That argument is falsified by this module's own
        // `ActorStanding::Retired`**: the command in question does not simply refuse, it refuses
        // with db/004's opaque resurrection P0001 — the exact dead end `Retired` exists to
        // close. So a fourth CASE label added later would silently reintroduce it. Unreachable
        // today; an error costs nothing and cannot mislead an operator.
        other => anyhow::bail!(
            "actor standing {other:?} is not one this build understands — the SQL classification \
             and this match have drifted apart, and guessing would hand the operator a remedy \
             that may be a dead end"
        ),
    })
}

/// The refusal for a key whose actor was retired. **Pure.**
///
/// Deliberately does **not** name `enroll-device-actor`: see [`ActorStanding::Retired`] for why
/// that command cannot help, and what the operator meets if they try.
pub fn retired_actor_refusal(kid: &str) -> anyhow::Error {
    crate::db_diagnosis::node_state_refusal(format!(
        "this node's signing key {kid} was enrolled as an actor and has since been revoked or \
         superseded, so it may not author clinical events. `cairn-node enroll-device-actor` \
         will NOT help and is not the remedy: re-enrolling a retired actor id is refused on \
         purpose, because it would silently resurrect the retired actor (issue #152). This \
         node needs a NEW signing key, enrolled afresh — which is a decision about who is \
         accountable for what this node writes, not a command to run blind."
    ))
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
        // Refuse rather than adding a THIRD row to a key that already maps to two.
        ActorStanding::Ambiguous => return Err(ambiguous_actor_refusal(kid)),
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
/// A [`crate::db_diagnosis::DeliberateRefusal`] (#651) at
/// [`crate::db_diagnosis::RefusalScope::NodeState`], because it is a verdict about the NODE
/// rather than about the form: the same key refuses identically no matter how many times the
/// call is retried unchanged, so offering a retry-now would be offering one that cannot work —
/// **but the clerk's form was never wrong, and once an operator has run the named command the
/// identical call succeeds.** That is the whole reason the scope exists: a surface must withhold
/// the retry-now and still keep a way forward. Contrast
/// [`crate::patient::register::dob_precision`], which is `Input` scope — nobody can make that
/// one succeed (PR #661 review).
///
/// It names the **command**, not only the key. `submit_event`'s own refusal — *"signer 9f3c… is
/// not an enrolled, non-revoked actor"* — is true, legible, and tells nobody what to do about
/// it; the precedent for naming the remedy is `submit_event`'s unwrap-key refusal, which names
/// `establish-unwrap-key`.
pub fn not_enrolled_refusal(kid: &str) -> anyhow::Error {
    crate::db_diagnosis::node_state_refusal(format!(
        "this node's signing key {kid} is not enrolled as an actor, so it may not author \
         clinical events. Enrolling is provisioning, not a side effect of writing: run \
         `cairn-node enroll-device-actor` once on this node. (A node created by `cairn-node \
         init` is already enrolled; a node restored without its actor registry is not.)"
    ))
}

/// The refusal for a key that maps to more than one current actor. **Pure.**
///
/// Refusing is the safe answer and letting it through is not: db/005 would accept the write and
/// stamp `actor_id = NULL` on it, and on every other event this key ever authors. An event whose
/// author cannot be named is worse than an event that was not written — the first is a silent,
/// permanent hole in the accountability record (principle 10), the second is a message on screen.
pub fn ambiguous_actor_refusal(kid: &str) -> anyhow::Error {
    crate::db_diagnosis::node_state_refusal(format!(
        "this node's signing key {kid} maps to MORE THAN ONE current actor, so nothing it \
         authors could be attributed to anyone (db/005 sets actor_id = NULL for every event a \
         dual-mapped key writes, node-wide and irreversibly). Refusing rather than writing an \
         unattributable clinical event. This should be unreachable through `enroll_actor`, so \
         it means something wrote `actor_event` directly or a new apply door skipped db/004's \
         `cairn_key_actor_id_conflict` — investigate before authoring anything on this node."
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
        ActorStanding::Ambiguous => Err(ambiguous_actor_refusal(kid)),
    }
}
