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
/// # The rule, in two parts
///
/// * a `tokio_postgres::Error` is rendered through [`legible_db_error`], never as its
///   kind — so a bare one (no wrapper) still says something useful;
/// * a layer is **dropped when the layer above already ends with it**, which is exactly
///   the `LocalDbFault` case and needs no knowledge of that type. A suffix test rather
///   than a type test keeps this honest for any future wrapper that renders its own cause.
///
/// Every layer is flattened with [`one_line`]: this is the other door into the same
/// one-line-per-event operator log `compose_db_diagnosis` already guards.
pub fn operator_chain(e: &anyhow::Error) -> String {
    let mut parts: Vec<String> = Vec::new();
    for cause in e.chain() {
        let rendered = match cause.downcast_ref::<tokio_postgres::Error>() {
            Some(pg) => legible_db_error(pg),
            None => one_line(&cause.to_string()),
        };
        // An empty layer would make the suffix test below vacuously true for everything
        // after it, so it is dropped outright rather than reasoned about.
        if rendered.is_empty() {
            continue;
        }
        if parts.last().is_some_and(|above| above.ends_with(&rendered)) {
            continue;
        }
        parts.push(rendered);
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

    /// A `LocalDbFault` must do BOTH jobs at once, and the second is the one that is easy
    /// to lose: it renders legibly (so an operator gets the SQLSTATE) *and* it keeps the
    /// `tokio_postgres::Error` reachable through `source()` (so `sync.rs`'s classifier can
    /// still tell a local database failure from a partition). `anyhow!("\u{2026}: {}", \u{2026})` does
    /// the first and silently destroys the second, which would reinstate issue #469's
    /// defect inside the fix for it.
    ///
    /// Exercised with no database and no network: `Config`'s own parser yields a real
    /// `tokio_postgres::Error` with a live `source()`.
    #[test]
    fn a_local_db_fault_renders_legibly_and_keeps_its_cause() {
        let pg = "host=localhost port=not-a-number"
            .parse::<tokio_postgres::Config>()
            .expect_err("a non-numeric port is not a parseable connection string");
        let fault = LocalDbFault::new("checkpointing sync cursor", pg);

        let text = fault.to_string();
        assert!(
            text.starts_with("checkpointing sync cursor: "),
            "the caller's context leads, so the line says WHAT was being done: {text}"
        );
        assert!(
            text.contains("port"),
            "\u{2026}and the rendered cause follows, naming the failure: {text}"
        );

        // The half classification depends on.
        let anyhow_err = anyhow::Error::from(fault);
        assert!(
            anyhow_err.chain().any(|c| c.is::<tokio_postgres::Error>()),
            "the cause must stay reachable through the chain, or every local database \
             failure is classified as a partition (#474 item 3): {anyhow_err:#}"
        );
        assert_ne!(format!("{anyhow_err}"), "db error", "{anyhow_err}");
    }

    /// A pure fixture: a real `tokio_postgres::Error` with a live `source()`, built with
    /// no database and no network. `Config`'s own parser is the only way to get one by
    /// hand, because `DbError` cannot be constructed outside the crate.
    fn a_real_pg_error() -> tokio_postgres::Error {
        "host=localhost port=not-a-number"
            .parse::<tokio_postgres::Config>()
            .expect_err("a non-numeric port is not a parseable connection string")
    }

    /// How many times does `needle` occur in `hay`? Used to pin the ONE thing the naive
    /// fix gets wrong.
    fn occurrences(hay: &str, needle: &str) -> usize {
        hay.matches(needle).count()
    }

    /// The defect this function exists for (PR #478 review, findings 1 and 2): a
    /// `.context()` layer over an ALREADY-RENDERED database failure hides the diagnosis,
    /// because `anyhow`'s non-alternate `Display` prints only the outermost message.
    ///
    /// This is the `serve_conn` shape — `db::connect` renders through `legible_db_error`
    /// and returns `anyhow`, then the caller adds its own layer.
    #[test]
    fn a_context_layer_over_a_rendered_failure_keeps_the_diagnosis() {
        let inner = anyhow::anyhow!("connecting to the database: no pg_hba.conf entry [28000]");
        let e = inner.context("serve: connecting to DB");

        assert_eq!(
            format!("{e}"),
            "serve: connecting to DB",
            "the premise: plain `Display` drops everything below the outermost layer"
        );
        let line = operator_chain(&e);
        assert!(
            line.contains("serve: connecting to DB") && line.contains("[28000]"),
            "both the layer AND the diagnosis must survive: {line}"
        );
    }

    /// The trap in the obvious fix. `{e:#}` renders the chain, but `LocalDbFault`'s own
    /// `Display` ALREADY embeds the rendered cause — so the alternate form prints the
    /// diagnosis twice, and for a server-side error the trailing copy is the literal
    /// `db error` this whole module exists to eliminate.
    #[test]
    fn a_local_db_fault_is_not_rendered_twice() {
        let e = anyhow::Error::from(LocalDbFault::new(
            "checkpointing sync cursor",
            a_real_pg_error(),
        ));

        let alt = format!("{e:#}");
        assert!(
            occurrences(&alt, "invalid value for option `port`") == 2,
            "the premise: the alternate form duplicates, which is why it is not the fix: {alt}"
        );

        let line = operator_chain(&e);
        assert_eq!(
            occurrences(&line, "invalid value for option `port`"),
            1,
            "the cause is rendered exactly once: {line}"
        );
        assert!(
            line.starts_with("checkpointing sync cursor: "),
            "…and the context still leads: {line}"
        );
    }

    /// Both at once: a caller's `.context()` stacked on a `LocalDbFault`. The classifier
    /// already survives this (`an_added_context_layer_leaves_the_cause_reachable`); the
    /// RENDERING has to as well, or the class is right and the line says nothing.
    #[test]
    fn a_context_layer_over_a_local_db_fault_renders_every_layer_once() {
        let e = anyhow::Error::from(LocalDbFault::new("reading sync cursor", a_real_pg_error()))
            .context("pull cycle 7");
        let line = operator_chain(&e);

        assert!(line.starts_with("pull cycle 7: "), "{line}");
        assert!(line.contains("reading sync cursor"), "{line}");
        assert_eq!(
            occurrences(&line, "invalid value for option `port`"),
            1,
            "still exactly once, with a layer above it: {line}"
        );
    }

    /// A transport failure is the other half of the `PARTITION` line: the errno lives in
    /// `source()`, and `.context()` + plain `Display` throws it away. Nothing here is a
    /// database error, so the function must not claim it is — it just walks the chain.
    #[test]
    fn a_transport_failure_keeps_its_errno() {
        let io = std::io::Error::new(std::io::ErrorKind::ConnectionRefused, "connection refused");
        let e = anyhow::Error::from(io).context("mTLS handshake (server pin)");
        let line = operator_chain(&e);

        assert!(
            line.contains("mTLS handshake (server pin)") && line.contains("connection refused"),
            "a partition line that names only the layer is #467 one kind over: {line}"
        );
    }

    /// A bare `tokio_postgres::Error` with no wrapper must still render legibly rather
    /// than as its kind — and must not render as the EMPTY string, which is what a naive
    /// "skip every postgres error" dedupe rule would produce.
    #[test]
    fn a_bare_postgres_error_still_renders() {
        let e = anyhow::Error::from(a_real_pg_error());
        let line = operator_chain(&e);

        assert_ne!(line, "db error", "{line}");
        assert!(!line.is_empty(), "a chain of one must still say something");
        assert!(
            line.contains("port"),
            "…and it must be the legible rendering: {line}"
        );
    }

    /// A multi-line cause cannot forge a second operator line, the same rule
    /// `compose_db_diagnosis` already obeys — this function is the other door into the
    /// same log.
    #[test]
    fn a_chain_never_forges_a_log_line() {
        let e = anyhow::anyhow!("first line\nFATAL: forged second line")
            .context("reading a response frame");
        let line = operator_chain(&e);

        assert!(
            !line.contains('\n'),
            "the whole point of a one-line log: {line}"
        );
        assert!(
            line.contains("forged second line"),
            "collapsed, never dropped: {line}"
        );
    }

    /// `.context()` on top of a `LocalDbFault` must not break the chain walk \u{2014} a caller is
    /// free to add its own layer, and the classifier has to survive it.
    #[test]
    fn an_added_context_layer_leaves_the_cause_reachable() {
        let pg = "host=localhost port=not-a-number"
            .parse::<tokio_postgres::Config>()
            .unwrap_err();
        let e = anyhow::Error::from(LocalDbFault::new("reading sync cursor", pg))
            .context("pull cycle 7");
        assert!(e.chain().any(|c| c.is::<tokio_postgres::Error>()), "{e:#}");
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
