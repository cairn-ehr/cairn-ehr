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
//! The two now take the same shape — `&tokio_postgres::Error` here, `&postgres::Error`
//! there, and they are the same type (`postgres` re-exports `tokio_postgres::error`). An
//! earlier version of this paragraph said `cairn-sync`'s took it BY VALUE "so it drops
//! straight into `.map_err(legible_db_error)`"; #479 changed that signature to a reference
//! so `LocalDbFault::new` could render without consuming, and rewrote all four point-free
//! call sites to `.map_err(|e| legible_db_error(&e))`. The sentence outlived the change and
//! was still telling readers to write code that no longer compiles (PR #486 review). What
//! must stay identical is the composed TEXT.
//!
//! (`cairn-sync` has ONE narrower renderer of the same idea left: `ApplyError::from`, which
//! keeps the SQLSTATE separately because the pull routing reads it, renders its no-`DbError`
//! arm through `legible_db_error`, and exposes `operator_text()` for the callers that have
//! established the failure was NOT a deliberate floor refusal — where the SQLSTATE is the
//! whole diagnosis, and where its own `Display` deliberately omits it.
//! `quarantine_event`'s private `legible()` — the second one this paragraph used to name —
//! is GONE: it flattened a server error into
//! a `String`, which destroyed the `source()` a chain-walking classifier reads and made a pen
//! write refused by this node's own database indistinguishable from the peer's data being at
//! fault (#490 item 2, #489 part 1). It uses `LocalDbFault` now, like everything else.)

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
/// `tokio_postgres::Error` implements `source()` but its `Display`
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

/// Render a whole `anyhow` error chain as ONE operator line, with each layer said once.
/// **Pure.**
///
/// # The defect this exists for (PR #478 review)
///
/// `anyhow::Error`'s plain `Display` prints **only the outermost message**. So a
/// `.context()` layer over an already-rendered database failure — `serve_conn`'s
/// `.context("serve: connecting to DB")` over `db::connect` — printed the layer and threw
/// the diagnosis away, which is #467's species surviving inside the sweep that closed it.
/// Two sites did this: the per-session `serve` line and `run`'s `PARTITION` line, the
/// latter losing the errno that separates *refused* from *timed out* from *no route*.
///
/// # Why not simply `{e:#}`
///
/// The alternate form walks the chain, but it **duplicates**: [`LocalDbFault`]'s own
/// `Display` already embeds `legible_db_error(source)`, so `{e:#}` appends the source's
/// `Display` again — and for a server-side error that trailing copy is the literal
/// `db error`. Rendering the fix's own log line as `… : db error` would have been a
/// remarkable way to close #474.
///
/// # The rule, in three parts
///
/// * a `tokio_postgres::Error` is rendered through [`legible_db_error`], never as its
///   kind — so a bare one (no wrapper) still says something useful;
/// * a layer is **dropped when the layer above already ends with it**, which is exactly
///   the `LocalDbFault` case and needs no knowledge of that type. A suffix test rather
///   than a type test keeps this honest for any future wrapper that renders its own cause;
/// * the walk **stops at the first `tokio_postgres::Error`**, because [`legible_db_error`]
///   has already consumed that error's whole source subtree. Without it the `Kind::Db` arm
///   printed the server's message twice on one line — the `{e:#}` defect this very header
///   rejects, and one the suffix rule cannot catch. The body comment at the `break` carries
///   the full account of why it survived so long; it is stated here because `cairn-sync`'s
///   twin points readers at this list for "the same three rules" and, until PR #486's
///   review, sent them to a list of two.
///
/// Every layer is flattened with `one_line` (private to this module): this is the other
/// door into the same one-line-per-event operator log `compose_db_diagnosis` already
/// guards.
pub fn operator_chain(e: &anyhow::Error) -> String {
    let mut parts: Vec<String> = Vec::new();
    for cause in e.chain() {
        let pg = cause.downcast_ref::<tokio_postgres::Error>();
        let rendered = match pg {
            Some(pg) => legible_db_error(pg),
            None => one_line(&cause.to_string()),
        };
        // An empty layer would make the suffix test below vacuously true for everything
        // after it, so it is dropped outright rather than reasoned about.
        if rendered.is_empty() {
            // Still stop if this was the database error — see the `break` below.
            if pg.is_some() {
                break;
            }
            continue;
        }
        if !parts.last().is_some_and(|above| above.ends_with(&rendered)) {
            parts.push(rendered);
        }
        if pg.is_some() {
            // STOP DESCENDING. `legible_db_error` has already consumed this error's whole
            // source subtree — the `Kind::Db` arm reads the `DbError`'s message, SQLSTATE,
            // DETAIL and HINT, and every other arm walks `source()` itself through
            // `kind_and_causes`. An `anyhow` chain is a single path, so everything below a
            // `tokio_postgres::Error` IS that subtree and has nothing left to add.
            //
            // Without this the `Kind::Db` arm printed the server's message TWICE on one
            // line, which is precisely the `{e:#}` defect this function's header rejects.
            // The suffix rule cannot catch it: `compose_db_diagnosis` renders
            // `message [SQLSTATE] — DETAIL — HINT` while `DbError`'s own `Display` is
            // `severity: message` + `DETAIL:` + `HINT:`, and neither ends with the other.
            //
            // It survived because the unit fixtures build their error from an unparseable
            // connection string — `Kind::ConfigParse`, whose rendering ends with its
            // cause's text, so the suffix rule fired and the dedupe LOOKED correct. The
            // arm every in-DB refusal actually takes had no coverage until
            // `a_server_error_is_rendered_once_through_the_whole_chain`, which needs a
            // live server because a `DbError` cannot be constructed by hand.
            break;
        }
    }
    parts.join(": ")
}

/// A database failure from **this node's own database**, rendered for a human without
/// throwing away the error underneath it.
///
/// # Why a type and not another `anyhow!`
///
/// `db.rs` renders its failures as `anyhow!("\u{2026}: {}", legible_db_error(&e))`, and that is
/// right there: nothing downstream of the schema loader reads the cause, so moving the
/// rendered string in and dropping the error costs nothing.
///
/// `sync.rs` is different. Its `run` loop must tell apart two failures that look identical
/// from the outside \u{2014} *this node's database failed* and *the peer did not answer* \u{2014} because
/// they send an operator to two different places, and calling the first one a partition
/// spends their attention on a healthy WAN while charging link downtime for a local write
/// failure (issue #474 item 3; issue #469 is the same defect in `cairn-sync`, where the fix
/// is a distinct error class). Classification walks the error chain, so the chain has to
/// still BE there \u{2014} and `anyhow!` empties it: the macro takes a formatted STRING, so the
/// `tokio_postgres::Error` is consumed by the `format!` and never becomes anyone's
/// `source()`.
///
/// So this type does both jobs at once: `Display` is the legible rendering an operator
/// reads, and `source()` is the original error a classifier can find. Replacing it with an
/// `anyhow!` looks like a tidy-up and silently reverts every local fault to `partition`.
///
/// # Why the rendering happens at construction
///
/// [`legible_db_error`] is called once, in [`LocalDbFault::new`], and the result is stored.
///
/// The saving is small and should not be overstated (PR #478 review, I6): only the
/// non-`DbError` arm walks a chain at all, that walk is hard-capped at 8 hops, and a
/// `LocalDbFault` is built at a failed QUERY, so `as_db_error()` \u{2014} a field access \u{2014} is the
/// dominant case. What rendering at construction really buys is that `Display` is total
/// and allocation-free: a `Display` that can do work is one that can behave differently on
/// an error path than on a happy one. Doing it at the site that knows the context is also
/// simply where the context is.
#[derive(Debug)]
pub struct LocalDbFault {
    /// What this node was trying to do, in the caller's own words \u{2014} `"checkpointing sync
    /// cursor"`. NEVER the connection string: it can carry a password.
    context: String,
    /// The [`legible_db_error`] rendering of `source`, computed once at construction.
    rendered: String,
    /// The original error, kept so [`std::error::Error::source`] can hand it back. This is
    /// the field the whole type exists for.
    source: tokio_postgres::Error,
}

impl LocalDbFault {
    /// Wrap a failed database call with what it was doing.
    ///
    /// `context` is a short phrase in the caller's own words. It must never contain a
    /// connection string, a password, or unbounded text from a peer \u{2014} it is spliced into a
    /// one-line operator log, and this project has already had to ESCAPE one such channel
    /// against line forgery (`custody_withheld`, #466 review \u{2014} which applies no length
    /// bound and says so in capitals; an earlier draft here called it \"bound\", PR #478
    /// review I5).
    pub fn new(context: &str, source: tokio_postgres::Error) -> Self {
        let rendered = legible_db_error(&source);
        Self {
            context: context.to_string(),
            rendered,
            source,
        }
    }
}

impl std::fmt::Display for LocalDbFault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.context, self.rendered)
    }
}

impl std::error::Error for LocalDbFault {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.source)
    }
}

// The refusal and commit-outcome markers (#810 split them out; their paths are unchanged).
mod refusal;
pub use refusal::*;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod refusal_scope_tests;
