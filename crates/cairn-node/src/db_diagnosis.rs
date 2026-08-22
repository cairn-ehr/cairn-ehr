//! Turn a PostgreSQL failure into something a human can act on (issue #467).
//!
//! # Why this module exists
//!
//! `tokio_postgres::Error`'s `Display` is the literal string **`db error`**. It is a bare
//! match on the error's *kind*, and it does not chain to the source that actually holds
//! the server's message, its `DETAIL` and — the part an operator can search for — its
//! **SQLSTATE**. So every `format!("…: {e}")` over one of these renders four characters
//! that name no cause:
//!
//! * `42P01` (relation does not exist — the schema never loaded here),
//! * `42501` (permission denied — a grant is missing),
//! * `40P01` (deadlock), `53300` (too many connections), `57014` (statement timeout),
//!
//! …are four completely different operator actions and one indistinguishable line. That
//! is not hypothetical: a required CI job failed with `loading 031_medication: db error`
//! and nothing more could be said about it, which is what filed #467.
//!
//! It is worse than a lost message, because `anyhow!("…: {e}")` also *discards the source*
//! — so `anyhow`'s own chain printing, which would otherwise have shown the `DbError`
//! underneath, has nothing left to show either. The wrapper that was meant to add context
//! subtracted the diagnosis.
//!
//! # Shape
//!
//! One **pure** composer ([`compose_db_diagnosis`], testable with no database at all,
//! because `tokio_postgres`'s `DbError` cannot be constructed by hand but its *rendering*
//! can) plus a thin extractor ([`legible_db_error`]) that pulls the three fields out of
//! `as_db_error()` and falls back to `Display` when there is no server error to read —
//! a dropped connection or an unparseable connection string, where the kind's own text
//! genuinely is the whole story.
//!
//! # The twin one crate over
//!
//! `cairn-sync`'s `main.rs` carries the same renderer, also called `legible_db_error`, and
//! composes the **byte-identical** shape on purpose: an operator grepping a node log and a
//! sync log must not have to learn two formats. The duplication is deliberate and small —
//! `cairn-sync` does not depend on `cairn-node`, and a new workspace crate to share twenty
//! lines would cost more than it saves — but it IS duplication, so if you change the shape
//! here, change it there. (`cairn-sync` also has two older, narrower renderers of the same
//! idea: `ApplyError::from`, which keeps the SQLSTATE separately because the pull routing
//! reads it, and `quarantine_event`'s local one.)

/// Compose the operator-facing text of a server-side failure. **Pure.**
///
/// The shape is `message [SQLSTATE] — DETAIL`, and each part earns its place:
///
/// * **message** — what the server said, which for our own doors is the `RAISE` text;
/// * **SQLSTATE** in brackets — the only machine-stable part, and therefore the part an
///   operator can search a manual or an issue tracker for. Bracketed so it is greppable
///   without matching prose;
/// * **DETAIL** — where the reason actually lives on this project's doors (issue #109:
///   `cairn_verify_error` travels as DETAIL, and `message()` alone drops it).
///
/// `detail` is `None` for the many failures that carry none; the separator goes with it,
/// so an absent DETAIL leaves no dangling em dash.
pub fn compose_db_diagnosis(message: &str, sqlstate: &str, detail: Option<&str>) -> String {
    let detail = detail.map(|d| format!(" — {d}")).unwrap_or_default();
    format!("{message} [{sqlstate}]{detail}")
}

/// Render a `tokio_postgres::Error` for a human instead of the two words it gives.
///
/// Takes a reference so a caller can keep the error (for its SQLSTATE, say) after asking
/// for its text — the node's callers wrap into `anyhow` and never need the original, but
/// borrowing costs nothing and forecloses nothing.
pub fn legible_db_error(e: &tokio_postgres::Error) -> String {
    match e.as_db_error() {
        Some(db) => compose_db_diagnosis(db.message(), db.code().code(), db.detail()),
        // Not a server error at all — a dropped connection, a TLS failure, an unparseable
        // connection string. `Display` names the kind, which for these IS the diagnosis.
        None => e.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// All three parts survive, and the SQLSTATE is bracketed so it can be grepped.
    #[test]
    fn compose_names_all_three_parts() {
        let text = compose_db_diagnosis(
            "relation \"event_log\" does not exist",
            "42P01",
            Some("cairn_verify_error: unknown key"),
        );
        assert!(
            text.contains("relation \"event_log\" does not exist"),
            "the server's own message is the headline: {text}"
        );
        assert!(
            text.contains("[42P01]"),
            "the SQLSTATE is the searchable part, and it is bracketed: {text}"
        );
        assert!(
            text.contains("cairn_verify_error: unknown key"),
            "DETAIL is where this project's doors put the reason (#109): {text}"
        );
    }

    /// Most failures carry no DETAIL. The separator must travel with it, or every line
    /// ends in a dangling em dash that reads like truncation.
    #[test]
    fn compose_without_detail_has_no_dangling_separator() {
        let text = compose_db_diagnosis("deadlock detected", "40P01", None);
        assert_eq!(text, "deadlock detected [40P01]");
        assert!(!text.contains('—'), "no orphan separator: {text}");
    }

    /// The fallback arm, exercised with no database and no network: an unparseable
    /// connection string fails in `Config`'s own parser and yields a
    /// `tokio_postgres::Error` that carries no `DbError`.
    ///
    /// The assertion that matters is the NEGATIVE one — whatever this renders, it must not
    /// be the useless string, and it must name the kind of thing that went wrong.
    #[test]
    fn a_non_db_error_falls_back_to_the_kind_and_is_not_db_error() {
        let e = "host=localhost port=not-a-number"
            .parse::<tokio_postgres::Config>()
            .expect_err("a non-numeric port is not a parseable connection string");
        let text = legible_db_error(&e);
        assert_ne!(text, "db error", "the whole point: {text}");
        assert!(
            !text.is_empty(),
            "the fallback still says something: {text}"
        );
    }
}
