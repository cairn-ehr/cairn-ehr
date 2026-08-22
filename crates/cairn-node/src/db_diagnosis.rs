//! Turn a PostgreSQL failure into something a human can act on (issue #467).
//!
//! # Why this module exists
//!
//! `tokio_postgres::Error`'s `Display` is the literal string **`db error`**. It is a bare
//! match on the error's *kind*, and it does not chain to the source that actually holds
//! the server's message, its `DETAIL`, its `HINT` and — the part an operator can search
//! for — its **SQLSTATE**. So every `format!("…: {e}")` over one of these renders eight
//! characters that name no cause:
//!
//! * `42P01` (relation does not exist — the schema never loaded here),
//! * `42501` (permission denied — a grant is missing),
//! * `40P01` (deadlock), `53300` (too many connections), `57014` (statement timeout),
//!
//! …are five completely different operator actions and one indistinguishable line. That
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
//! can) plus a thin extractor ([`legible_db_error`]) that pulls the four fields out of
//! `as_db_error()`.
//!
//! When there is no `DbError` to read — a refused socket, a dropped connection, a TLS
//! failure, an unparseable connection string — the extractor names the kind **and then
//! walks `source()`**. That second half is not optional, and the first version of this
//! module got it wrong (PR #472 review, finding 1): `Display` is a bare kind match for
//! *every* kind, not just `Kind::Db`, so `Kind::Connect` renders `error connecting to
//! server` whether the socket was refused, the host did not resolve, or the handshake
//! timed out — three different operator actions and one indistinguishable line, which is
//! this module's own opening paragraph one kind over. The cause is reachable, it is just
//! not in `Display`; `Connection refused (os error 61)` lives in `source()`.
//!
//! # The twin one crate over
//!
//! `cairn-sync`'s `main.rs` carries the same renderer, also called `legible_db_error`, and
//! composes the **byte-identical** shape on purpose: an operator grepping a node log and a
//! sync log must not have to learn two formats. The duplication is deliberate and small —
//! `cairn-sync` depends on `cairn-node` only as a *dev*-dependency, so its production code
//! cannot call this, and a new workspace crate to share thirty lines would cost more than
//! it saves. But it IS duplication, so `the_agreed_shape_is_exactly_this` here and
//! `the_twin_composes_the_agreed_shape` there assert the same strings: change the shape in
//! one and the other crate goes red rather than silently drifting.
//!
//! The two are not copy-paste-compatible, which is worth knowing before you "change it
//! there too": this one takes `&tokio_postgres::Error`, `cairn-sync`'s takes
//! `postgres::Error` BY VALUE so it drops straight into `.map_err(legible_db_error)`.
//! (They are the same type — `postgres` re-exports `tokio_postgres::error`.) What must
//! stay identical is the composed TEXT, not the signature.
//!
//! (`cairn-sync` also has two older, narrower renderers of the same idea: `ApplyError::from`,
//! which keeps the SQLSTATE separately because the pull routing reads it, and
//! `quarantine_event`'s local one.)

/// Flatten a server-supplied string onto ONE line. **Pure.**
///
/// Every rendering here ends up in a one-line-per-event operator log, and a PostgreSQL
/// `DETAIL`/`HINT` is routinely multi-line. A raw newline would let a server — or, through
/// the bounded peer prefixes this project's own doors splice into their `RAISE` messages, a
/// PEER — forge a whole log line. `cairn-sync`'s `unlearnable_report` escapes its cause for
/// exactly this reason; so does this, and collapsing rather than escaping keeps the line
/// readable (an operator reading `\n` in prose is worse off than one reading a long line).
///
/// Collapses, never drops: every character the server sent is still present.
fn one_line(s: &str) -> String {
    s.replace(['\n', '\r'], " ")
}

/// Compose the operator-facing text of a server-side failure. **Pure.**
///
/// The shape is `message [SQLSTATE] — DETAIL — HINT: hint`, and each part earns its place:
///
/// * **message** — what the server said, which for our own doors is the `RAISE` text;
/// * **SQLSTATE** in brackets — the only machine-stable part, and therefore the part an
///   operator can search a manual or an issue tracker for. Bracketed so it is greppable
///   without matching prose;
/// * **DETAIL** — where the reason actually lives on this project's doors (issue #109:
///   `cairn_verify_error` travels as DETAIL, and `message()` alone drops it);
/// * **HINT** — where POSTGRES puts the remedy (`No function matches the given name and
///   argument types.`, `Perhaps you meant to reference the column …`). Labelled, because
///   unlike DETAIL it is advice rather than fact and an operator should weigh it as such.
///   Dropping it — as the first version of this module did — threw away the most
///   actionable line the server sent, on exactly the `42883`-shaped failures the schema
///   loader hits (PR #472 review, finding 1).
///
/// `detail` and `hint` are `None` for the many failures that carry neither; each separator
/// travels with its own part, so an absent part leaves no dangling em dash.
pub fn compose_db_diagnosis(
    message: &str,
    sqlstate: &str,
    detail: Option<&str>,
    hint: Option<&str>,
) -> String {
    let message = one_line(message);
    let detail = detail
        .map(|d| format!(" \u{2014} {}", one_line(d)))
        .unwrap_or_default();
    let hint = hint
        .map(|h| format!(" \u{2014} HINT: {}", one_line(h)))
        .unwrap_or_default();
    format!("{message} [{sqlstate}]{detail}{hint}")
}

/// Name the kind of a non-server failure **and every cause beneath it**. **Pure.**
///
/// `tokio_postgres::Error` implements `source()` (`error/mod.rs:408`) but its `Display`
/// never consults it, so the kind alone is a category, not a diagnosis: `error connecting
/// to server` is the whole of what `Display` says about a refused socket, an unresolvable
/// host and a TLS timeout alike. Walking the chain is what turns that back into something
/// an operator can act on.
///
/// The hop limit is belt-and-braces for an error path: a cyclic source chain would
/// otherwise spin forever, and no diagnostic is worth hanging a node over.
fn kind_and_causes(e: &tokio_postgres::Error) -> String {
    let mut text = one_line(&e.to_string());
    let mut source = std::error::Error::source(e);
    for _ in 0..8 {
        match source {
            Some(cause) => {
                text.push_str(": ");
                text.push_str(&one_line(&cause.to_string()));
                source = cause.source();
            }
            None => break,
        }
    }
    text
}

/// Render a `tokio_postgres::Error` for a human instead of the two words it gives.
///
/// Takes a reference so a caller can keep the error (for its SQLSTATE, say) after asking
/// for its text — the node's callers wrap into `anyhow` and never need the original, but
/// borrowing costs nothing and forecloses nothing.
pub fn legible_db_error(e: &tokio_postgres::Error) -> String {
    match e.as_db_error() {
        Some(db) => compose_db_diagnosis(db.message(), db.code().code(), db.detail(), db.hint()),
        // Not a server error at all — a refused socket, a dropped connection, a TLS
        // failure, an unparseable connection string. The kind names the LAYER that failed;
        // the chain beneath it names the failure. Both, or this arm is #467 again.
        None => kind_and_causes(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// All four parts survive, and the SQLSTATE is bracketed so it can be grepped.
    #[test]
    fn compose_names_every_part_the_server_sent() {
        let text = compose_db_diagnosis(
            "relation \"event_log\" does not exist",
            "42P01",
            Some("cairn_verify_error: unknown key"),
            Some("Perhaps you meant to reference the table \"event_log_v2\"."),
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
        assert!(
            text.contains("HINT: Perhaps you meant"),
            "HINT is where POSTGRES puts the remedy, and dropping it threw away the \
             most actionable line the server sent (PR #472 review): {text}"
        );
    }

    /// Most failures carry neither DETAIL nor HINT. Each separator must travel with its
    /// own part, or a line ends in a dangling em dash that reads like truncation.
    #[test]
    fn compose_without_detail_or_hint_has_no_dangling_separator() {
        let text = compose_db_diagnosis("deadlock detected", "40P01", None, None);
        assert_eq!(text, "deadlock detected [40P01]");
        assert!(!text.contains('\u{2014}'), "no orphan separator: {text}");
    }

    /// HINT without DETAIL is the common shape for a PostgreSQL syntax/typo error, and
    /// it must not borrow DETAIL's separator or read as one.
    #[test]
    fn compose_carries_a_hint_that_arrives_without_a_detail() {
        let text = compose_db_diagnosis(
            "function no_such_function_here() does not exist",
            "42883",
            None,
            Some("No function matches the given name and argument types."),
        );
        assert_eq!(
            text,
            "function no_such_function_here() does not exist [42883] \u{2014} \
             HINT: No function matches the given name and argument types.",
            "{text}"
        );
    }

    /// **The regression this pins (PR #472 review, finding 1).**
    ///
    /// `tokio_postgres::Error`'s `Display` is a bare match on KIND for every kind, not
    /// just `Kind::Db` — it never chains to `source()`. So the fallback arm used to
    /// render `"invalid connection string"` and drop the one fact an operator needs:
    /// WHICH option was invalid. That is `"db error"` with a longer name, in the very
    /// arm this module was written to make legible.
    ///
    /// Exercised with no database and no network: `Config`'s own parser produces a
    /// `tokio_postgres::Error` carrying no `DbError` but a live `source()`.
    #[test]
    fn a_non_db_error_names_its_cause_not_just_its_kind() {
        let e = "host=localhost port=not-a-number"
            .parse::<tokio_postgres::Config>()
            .expect_err("a non-numeric port is not a parseable connection string");
        let text = legible_db_error(&e);

        assert_ne!(text, "db error", "the whole point: {text}");
        assert!(
            text.contains("invalid connection string"),
            "the kind still leads, because it says which LAYER failed: {text}"
        );
        assert!(
            text.contains("port"),
            "and the cause names WHICH option was wrong \u{2014} without it this arm is \
             the same silent failure one kind over: {text}"
        );
    }

    /// A server message can carry newlines (a multi-line DETAIL or HINT is ordinary),
    /// and every rendering here ends up in a one-line-per-event operator log. A raw
    /// newline would let a server \u{2014} or, through a bounded peer prefix in a door's
    /// own message, a PEER \u{2014} forge a whole log line. `unlearnable_report` one crate
    /// over already escapes for exactly this reason; so does this.
    #[test]
    fn a_diagnosis_never_forges_a_log_line() {
        let text = compose_db_diagnosis(
            "submit_event: refused\npull peer-a: 0 attachment reference(s) unlearnable",
            "P0001",
            Some("first\r\nsecond"),
            Some("third\rfourth"),
        );
        assert!(
            !text.contains('\n') && !text.contains('\r'),
            "no rendering may span two lines: {text:?}"
        );
        assert!(
            text.contains("submit_event: refused") && text.contains("second"),
            "collapsing must not DROP anything \u{2014} it only flattens: {text}"
        );
    }

    /// The composed shape is a contract shared with `cairn-sync` (see the module doc).
    /// This is the node half of the pair; `cairn-sync`'s `the_twin_composes_the_agreed_shape`
    /// asserts the identical string, so drift between the two goes red instead of being
    /// caught by a reviewer's eye.
    #[test]
    fn the_agreed_shape_is_exactly_this() {
        assert_eq!(
            compose_db_diagnosis("permission denied for table event_log", "42501", None, None),
            "permission denied for table event_log [42501]"
        );
        assert_eq!(
            compose_db_diagnosis("could not obtain lock", "55P03", Some("why"), Some("how")),
            "could not obtain lock [55P03] \u{2014} why \u{2014} HINT: how"
        );
    }
}
