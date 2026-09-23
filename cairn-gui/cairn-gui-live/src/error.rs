//! One decision: was a failed call a VERDICT about it, or an accident that befell it?
//!
//! # The contract this rests on
//!
//! Every refusal the in-DB floor *raises* is a bare `RAISE EXCEPTION`, which PostgreSQL
//! assigns SQLSTATE `P0001`. That is a **contract, not an accident of using `RAISE
//! EXCEPTION`**, and it is stated per-door rather than tree-wide: `db/001_envelope.sql` says
//! it above `cairn_decode_hex_or_raise` for the node-plane doors (#228) and
//! `db/048_sensitivity_stream.sql` says it for the clinical apply door, each forbidding
//! `USING ERRCODE` because a pull loop routes on it. The guard that makes that per-door prose
//! true of every `db/*.sql` at once is
//! `floor_refusals_carry_no_errcode.rs` in `crates/cairn-node/tests/` (#633). Anything else —
//! a dropped connection, a lock timeout, a serialization failure, a full disk — decided
//! nothing at all.
//!
//! **One kind of floor decision is still deliberately outside this rule and reported as an
//! outage:** a **constraint** violation (class `23`) or a **privilege** refusal (`42501`) is the
//! floor deciding — principle 12 counts RLS and constraints as part of it — but carries its own
//! SQLSTATE, not `P0001`. No path reachable from these two ports produces one today, and
//! `a_constraint_or_privilege_refusal_is_not_yet_told_apart` pins the CLASSIFICATION so the gap
//! is a value in every run. (It does not pin the reachability; nothing does.) That is the
//! unanswered *`false` half* of the rule, which `cairn-sync` already had to answer for itself
//! (`LocalDbFault`): [#655](https://github.com/cairn-ehr/cairn-ehr/issues/655) carries it.
//!
//! **A refusal raised in Rust, before any statement reaches Postgres, USED to be the second
//! member of that list and no longer is** — see the two-discriminator section below (#651).
//!
//! Getting it backwards is a real defect in both directions. Calling an outage a refusal
//! tells a clerk to change a form that was never the problem; calling a refusal an outage
//! hands them a retry button for a verdict, and they press it.
//!
//! # ⚠️ A THIRD HOME FOR A RULE THAT SHOULD HAVE ONE
//!
//! `cairn-sync`'s `refusal_is_deliberate` and `cairn-node`'s
//! `restore::clinical::refusal_is_deliberate` are the other two, and the second one's own doc
//! already calls itself *"A SECOND HOME … Keep the two identical"*. This is the third, and it
//! is here rather than shared because consolidating them means changing `crates/`, which is a
//! different slice's blast radius. **Filed as
//! [#652](https://github.com/cairn-ehr/cairn-ehr/issues/652)**. Until it is done: if you change
//! the rule, change all three. The drift costs a wrong verdict, not merely an inaccurate
//! sentence.
//!
//! What #652 should ALSO absorb is #655 — the `false` half above. Three copies of a rule that is
//! only half right is the worse of the two problems.
//!
//! The three are not *quite* redundant, and the difference is worth knowing: the other two
//! take an already-extracted `Option<&str>`, because their callers hold a
//! `tokio_postgres::Error` directly. This crate's callers hold an `anyhow::Error` from a
//! `cairn-node` orchestrator, so it must dig the SQLSTATE out of a context chain first —
//! which is [`sqlstate_of`], and is the part that can silently stop working.
//!
//! # THE RULE HAS TWO DISCRIMINATORS, NOT ONE (#651, 2026-09-23)
//!
//! A verdict can be reached in two places, so [`data_error_from`] asks two questions and a
//! `true` from either one means *refused*:
//!
//! 1. **The SQLSTATE is `P0001`** — the floor raised it, under the no-`USING ERRCODE` contract
//!    described above.
//! 2. **`cairn_node::db_diagnosis::carries_refusal_marker` finds a marker on the chain** — the
//!    node refused in Rust, before any statement reached Postgres.
//!
//! The second exists because the date-of-birth shape check `register_patient` runs up front is
//! just as deterministic as anything the floor does — the same string refuses identically
//! forever — and carried no SQLSTATE at all, so the most deterministic failure on the
//! registration path reached the clerk as an outage. On a desk with no date widget that is the
//! *default* failure mode: `3/2/1980` searches fine (the trigger applies no date format check,
//! correctly — a registrar is often told only a year), finds nothing, then fails in Rust with a
//! retry button that can never work. `tests/refusal_is_not_an_outage.rs` proves both arms
//! against a real floor.
//!
//! **What is still NOT covered is the `false` half**, which is #655 above: a constraint
//! violation (class `23`) and a privilege refusal (`42501`) are floor *decisions* carrying their
//! own SQLSTATE, and they still land in `Unavailable`. Adding a second discriminator did not
//! make the first one right.
//!
//! (An earlier draft of this paragraph also listed `42P01`. That is a mis-scoping worth not
//! repeating: a never-loaded schema is an ENVIRONMENT fault, not a floor decision — `Unavailable`
//! is the right answer for it, and retrying after somebody loads the schema is exactly what
//! should happen. `cairn-node`'s own `db_diagnosis` module doc calls it *"the schema never loaded
//! here"* for the same reason. The pre-existing test below does exercise `42P01`, but only as one
//! of a list of codes that are not `P0001`.)
use cairn_gui_data::port::DataError;

/// The SQLSTATE PostgreSQL assigns to a bare `RAISE EXCEPTION` in PL/pgSQL.
const SQLSTATE_RAISE_EXCEPTION: &str = "P0001";

/// Did the floor DELIBERATELY refuse this call? **Pure.**
///
/// `None` — no SQLSTATE reached us at all — is never a verdict. See the module doc.
pub fn refusal_is_deliberate(sqlstate: Option<&str>) -> bool {
    sqlstate == Some(SQLSTATE_RAISE_EXCEPTION)
}

/// Dig the SQLSTATE out of a `cairn-node` orchestrator's error.
///
/// Walks the whole `anyhow` chain rather than reading only the outermost error, because every
/// orchestrator adds `.context("…")` layers naming the operation. The same walk
/// `cairn_node::db_diagnosis::operator_chain` performs, for the same reason.
///
/// **This returns `None` when a call site rendered its database error into a string** — e.g.
/// `anyhow!("submit: {}", legible_db_error(&e))` — because that destroys the source and the
/// `tokio_postgres::Error` is no longer in the chain to be found. The failure is silent and
/// one-directional: every refusal becomes an outage. Only a test against a real floor can
/// catch it, which is why `tests/refusal_is_not_an_outage.rs` exists and why it must never be
/// relaxed into asserting merely that the call failed.
///
/// The `and_then` is INSIDE the `find_map` on purpose. A `tokio_postgres::Error` that carries
/// no `DbError` (a client-side decode failure, say) must not end the walk: the layer that
/// matters may be deeper. Hoisting it out — `find_map(downcast).and_then(as_db_error)` — reads
/// the same and silently answers `None` for a chain whose verdict is one link further down.
///
/// Borrows rather than allocating: `SqlState::code` returns a `&str` tied to the `DbError`,
/// which lives as long as `e` does.
pub fn sqlstate_of(e: &anyhow::Error) -> Option<&str> {
    e.chain()
        .find_map(|cause| {
            cause
                .downcast_ref::<tokio_postgres::Error>()
                .and_then(|pg| pg.as_db_error())
        })
        .map(|db| db.code().code())
}

/// Map a `cairn-node` orchestrator's failure onto the port's error type.
///
/// The message is `cairn_node::db_diagnosis::operator_chain`'s rendering in both arms — one
/// line, the server's message rendered exactly once, every *distinct* context layer above the
/// database error kept. (It is not a verbatim transcript: `operator_chain` collapses a layer
/// whose text the layer above already ends with, and stops descending at the database error,
/// because `legible_db_error` has already consumed that error's own source subtree.)
///
/// `commands.rs`'s rule 1: return the underlying text, never a generic string. An in-DB floor
/// refusal is legible on purpose (§9.6), and the text is the only thing that tells the clerk
/// what to change.
///
/// ⚠️ **This is an OPERATOR rendering, not a clerk-facing sentence.** `operator_chain` exists
/// for a one-line-per-event operator log and appends the bracketed SQLSTATE. Slice 2c must not
/// paste it raw into a form.
///
/// **On an unprovisioned node the remedy-naming refusal is reached through
/// [`LiveData::require_provisioned`](crate::LiveData::require_provisioned)**, which the window
/// calls before a registration. The three `cairn_node::actor_enrolment` refusals are
/// `NodeState`-scoped and map here to [`DataError::NotProvisioned`]. A caller that skips that
/// pre-check still meets db/005's own `submit_event: signer 9f3c… is not an enrolled,
/// non-revoked actor [P0001]` — true, carrying no marker, and so classified `Refused` — which
/// is why the port suites can still use an unenrolled signer to reach the floor (#665).
pub fn data_error_from(e: &anyhow::Error) -> DataError {
    let text = cairn_node::db_diagnosis::operator_chain(e);
    // TWO discriminators, complementary rather than alternative, because a verdict can be
    // reached in two places. The floor's own refusals carry `P0001`; a refusal `cairn-node`
    // raised in Rust before any statement reached Postgres carries no SQLSTATE at all and is
    // MARKED instead (#651). Either one means the call was decided, not merely unlucky.
    //
    // The SCOPE question is asked FIRST, and only of the marked ones, because it is narrower:
    // a marked refusal at `NodeState` scope is still a verdict, but the way forward is an
    // operator command rather than an edit to the form, and `Refused` is rendered with no way
    // forward but the form. Asking it first keeps the two questions in the right order — "is
    // this a verdict" then "a verdict about what" — so a future third scope cannot silently
    // fall through to `Unavailable` (PR #661 review).
    match cairn_node::db_diagnosis::refusal_scope(e) {
        Some(cairn_node::db_diagnosis::RefusalScope::NodeState) => DataError::NotProvisioned(text),
        // `Input` scope, or no marker at all — fall through to the SQLSTATE question, which is
        // the only one that can speak for the floor's own refusals.
        Some(cairn_node::db_diagnosis::RefusalScope::Input) => DataError::Refused(text),
        None if refusal_is_deliberate(sqlstate_of(e)) => DataError::Refused(text),
        None => DataError::Unavailable(text),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The contract, pinned as a value. Every `db/*.sql` refusal is a bare `RAISE EXCEPTION`,
    /// which `floor_refusals_carry_no_errcode.rs` enforces tree-wide (#633), so this one code
    /// identifies all of them.
    #[test]
    fn a_bare_raise_exception_is_the_floors_verdict() {
        assert!(refusal_is_deliberate(Some("P0001")));
    }

    /// Everything else is an accident that befell the call, not a decision about it. These
    /// are the ones an operator actually meets: a serialization failure, a lock-not-available,
    /// disk full, admin shutdown, a dropped connection, and an internal error — the last of
    /// which is a pgrx function panicking on adversarial bytes, and is emphatically not a
    /// considered verdict.
    #[test]
    fn everything_else_is_an_accident_not_a_verdict() {
        for code in ["40001", "55P03", "53100", "57P01", "08006", "XX000"] {
            assert!(
                !refusal_is_deliberate(Some(code)),
                "{code} is not a floor verdict — treating it as one would tell the clerk to \
                 change a form that was never the problem"
            );
        }
    }

    /// The SECOND discriminator, as a pure test (#651).
    ///
    /// `data_error_from`'s marker arm is the one the PR that added it names as its load-bearing
    /// mutation, and until this test existed it was proved ONLY by a DB-gated suite — so the
    /// cheapest proof of the newest rule needed a database (PR #661 review). It does not.
    #[test]
    fn a_marked_refusal_is_a_verdict_even_with_no_database_in_sight() {
        // Built the way an orchestrator builds one: the marker at the bottom, operation context
        // layered above it, and no `tokio_postgres::Error` anywhere in the chain. (`context` is
        // an INHERENT method on `anyhow::Error`, so no trait import — one here is an unused
        // import, which CI's clippy denies.)
        let e = cairn_node::patient::register::dob_precision("3/2/1980")
            .expect_err("a malformed birth date refuses")
            .context("registering the patient");
        match data_error_from(&e) {
            DataError::Refused(text) => assert!(
                text.contains("not a recognised shape"),
                "the orchestrator's own sentence is what tells the clerk WHAT to change — got: \
                 {text}"
            ),
            other => panic!(
                "a deterministic Rust-side refusal must not reach the clerk as {other:?} — that \
                 is a retry button on a verdict, on the default failure mode of a desk with no \
                 date widget"
            ),
        }
    }

    /// A node-state verdict is NOT rendered as a dead end, and not as an outage either.
    ///
    /// The clerk's form was correct. Retrying the identical call is pointless, so `Unavailable`
    /// — which earns a retry-now button — would be a precise untruth. But the way forward
    /// exists and is one operator command away, so `Refused` — which slice 2c renders with no
    /// way forward but editing the form — strands them just as badly, on a form that was never
    /// the problem.
    ///
    /// **The mutation that kills this test:** collapse the scope arm in `data_error_from` back
    /// into the single `Refused` answer. Both refusals still classify as verdicts and every
    /// other test in both trees stays green — which is exactly how the two situations came to
    /// share one rendering in the first place (PR #661 review).
    #[test]
    fn a_node_state_verdict_is_neither_a_dead_end_nor_an_outage() {
        let e = cairn_node::actor_enrolment::not_enrolled_refusal("9f3c")
            .context("registering the patient");
        match data_error_from(&e) {
            DataError::NotProvisioned(text) => assert!(
                text.contains("enroll-device-actor"),
                "the payload must name the command that makes this same call succeed — got: \
                 {text}"
            ),
            other => panic!(
                "an unprovisioned node must not reach the clerk as {other:?}: `Refused` offers \
                 no way forward on a form that was correct, and `Unavailable` offers a \
                 retry-now that will fail identically until an operator acts"
            ),
        }
    }

    /// The two scopes are told apart, not merely both marked.
    ///
    /// Without this, a `refusal_scope` that answered `NodeState` for everything marked would
    /// pass the test above while sending a malformed date of birth to a rendering that implies
    /// an operator can fix it.
    #[test]
    fn an_input_verdict_and_a_node_state_verdict_do_not_render_the_same() {
        let input = cairn_node::patient::register::dob_precision("3/2/1980")
            .expect_err("a malformed birth date refuses");
        let node_state = cairn_node::actor_enrolment::not_enrolled_refusal("9f3c");
        assert!(matches!(data_error_from(&input), DataError::Refused(_)));
        assert!(matches!(
            data_error_from(&node_state),
            DataError::NotProvisioned(_)
        ));
    }

    /// THE CLASSES THIS RULE KNOWINGLY GETS WRONG, pinned so the gap is a value rather than a
    /// sentence in a doc.
    ///
    /// A constraint violation and a privilege refusal are the floor *deciding* — principle 12
    /// counts RLS and constraints as part of the floor — and both are as deterministic as a
    /// `RAISE`. They are classified as outages here because they carry their own SQLSTATE, so
    /// a clerk meeting one is offered a retry that can never work. Nothing reachable from
    /// these two ports produces one today; the sibling copy in `cairn-sync` already pins
    /// `23514` for the same reason.
    ///
    /// **This test asserts today's behaviour, not the desired one.** The Rust-side half got
    /// the same treatment until #651 fixed it, and
    /// `a_rust_side_pre_flight_refusal_is_refused_not_unavailable` is what that test became —
    /// which is the precedent for this one. When
    /// [#655](https://github.com/cairn-ehr/cairn-ehr/issues/655) resolves the split, this
    /// fails; invert it and delete this paragraph.
    #[test]
    fn a_constraint_or_privilege_refusal_is_not_yet_told_apart() {
        for code in ["23514", "23505", "23502", "23503", "42501", "42P01"] {
            assert!(
                !refusal_is_deliberate(Some(code)),
                "{code} classifies as an outage today (#655). If this now fails, the \
                 `false`-half split has landed — invert the assertion."
            );
        }
    }

    /// NO SQLSTATE AT ALL IS NEVER A VERDICT, and this is the arm that matters most.
    ///
    /// A dropped connection, a TLS reset, a client-side decode failure: the statement never
    /// reached a decision. Defaulting these to `Refused` would tell a clerk their registration
    /// was rejected by the safety floor when the network blinked — and, worse, would tell them
    /// not to retry the one thing that would have worked.
    #[test]
    fn no_sqlstate_is_never_a_verdict() {
        assert!(!refusal_is_deliberate(None));
    }

    /// An error carrying no `tokio_postgres::Error` anywhere in its chain has no SQLSTATE to
    /// read, so it must reach the caller as an outage. `anyhow!` builds exactly that shape.
    #[test]
    fn an_error_with_no_database_cause_is_an_outage() {
        let e = anyhow::anyhow!("the node key could not be read");
        assert_eq!(sqlstate_of(&e), None);
        let mapped = data_error_from(&e);
        let DataError::Unavailable(text) = &mapped else {
            panic!("expected an outage, got {mapped:?}");
        };
        assert!(
            text.contains("node key"),
            "the operator needs the real text, never a category label: {text}"
        );
    }

    /// The message must survive `anyhow`'s context layers, because that is how every
    /// `cairn-node` orchestrator reports: the outermost layer says which operation failed and
    /// an inner one says why. Dropping either half leaves the clerk with half a sentence.
    #[test]
    fn the_whole_context_chain_reaches_the_caller() {
        use anyhow::Context;
        let e = Err::<(), _>(anyhow::anyhow!("relation does not exist"))
            .context("registering the patient")
            .unwrap_err();
        let mapped = data_error_from(&e);
        let DataError::Unavailable(text) = &mapped else {
            panic!("expected an outage, got {mapped:?}");
        };
        assert!(text.contains("registering the patient"), "got: {text}");
        assert!(text.contains("relation does not exist"), "got: {text}");
    }
}
