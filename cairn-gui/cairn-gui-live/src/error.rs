//! One decision: was a failed call a VERDICT about it, or an accident that befell it?
//!
//! # The contract this rests on
//!
//! Every refusal in the in-DB floor is a bare `RAISE EXCEPTION`, which PostgreSQL assigns
//! SQLSTATE `P0001`. That is a **contract, not an accident of using `RAISE EXCEPTION`**:
//! `db/001_envelope.sql` states it in the comment above `cairn_decode_hex_or_raise` (#228)
//! and forbids `USING ERRCODE` on those refusals, because the node pull loop routes on it.
//! Anything else — a dropped connection, a lock timeout, a serialization failure, a full
//! disk — decided nothing at all.
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
//! [#652](https://github.com/cairn-ehr/cairn-ehr/issues/652)**, which also names #633 as the
//! guard that belongs in the same shared home. Until it is done: if you change the rule,
//! change all three. The drift costs a wrong verdict, not merely an inaccurate sentence.
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
pub fn sqlstate_of(e: &anyhow::Error) -> Option<String> {
    e.chain()
        .find_map(|cause| cause.downcast_ref::<tokio_postgres::Error>())
        .and_then(|pg| pg.as_db_error())
        .map(|db| db.code().code().to_string())
}

/// Map a `cairn-node` orchestrator's failure onto the port's error type.
///
/// The message is `cairn_node::db_diagnosis::operator_chain`'s rendering in both arms — one
/// line, the server's message rendered exactly once, every context layer kept.
/// `commands.rs`'s rule 1: return the underlying text, never a generic string. An in-DB floor
/// refusal is legible on purpose (§9.6), and the text is the only thing that tells the clerk
/// what to change.
pub fn data_error_from(e: &anyhow::Error) -> DataError {
    let text = cairn_node::db_diagnosis::operator_chain(e);
    if refusal_is_deliberate(sqlstate_of(e).as_deref()) {
        DataError::Refused(text)
    } else {
        DataError::Unavailable(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The contract, pinned as a value. `db/001_envelope.sql` forbids `USING ERRCODE` on the
    /// floor's refusals precisely so this one code identifies all of them.
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
