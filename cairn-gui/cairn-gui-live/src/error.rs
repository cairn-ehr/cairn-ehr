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
//! **Two kinds of floor decision are deliberately outside this rule**, and both are reported
//! as outages today:
//!
//! - a **constraint** violation (class `23`) or a **privilege** refusal (`42501`) is the floor
//!   deciding — principle 12 counts RLS and constraints as part of it — but carries its own
//!   SQLSTATE, not `P0001`. No path reachable from these two ports produces one today, which is
//!   why this stays a binary rule for now — and
//!   `a_constraint_or_privilege_refusal_is_not_yet_told_apart` pins the CLASSIFICATION so the
//!   gap is a value in every run. (It does not pin the reachability; nothing does.)
//! - a refusal raised **in Rust, before any statement reaches Postgres** — see below.
//!
//! Both belong to the same unanswered question — *the `false` half of `refusal_is_deliberate`
//! is not one thing* — which `cairn-sync` already had to answer for itself (`LocalDbFault`).
//! [#655](https://github.com/cairn-ehr/cairn-ehr/issues/655) carries it.
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
//! # What this rule does NOT cover
//!
//! A refusal `cairn-node` raises **in Rust, before any statement reaches Postgres** — the
//! date-of-birth shape check `register_patient` runs up front — is just as deterministic and
//! carries no SQLSTATE at all, so it reaches the clerk as an outage.
//! [#651](https://github.com/cairn-ehr/cairn-ehr/issues/651) has the argument and the two
//! candidate fixes; `tests/refusal_is_not_an_outage.rs` pins today's behaviour so the gap is
//! visible in every run rather than only in that issue.
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
/// for a one-line-per-event operator log and appends the bracketed SQLSTATE. The refusal a
/// fresh node actually produces reads `submit_event: signer 9f3c… is not an enrolled,
/// non-revoked actor [P0001]` — true, legible, and not a remedy. Slice 2c must not paste it
/// raw into a form; resolving that is
/// [#654](https://github.com/cairn-ehr/cairn-ehr/issues/654).
pub fn data_error_from(e: &anyhow::Error) -> DataError {
    let text = cairn_node::db_diagnosis::operator_chain(e);
    if refusal_is_deliberate(sqlstate_of(e)) {
        DataError::Refused(text)
    } else {
        DataError::Unavailable(text)
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
    /// **This test asserts today's behaviour, not the desired one** — the same treatment
    /// `a_rust_side_pre_flight_refusal_is_not_yet_told_apart` gives the Rust-side half. When
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
