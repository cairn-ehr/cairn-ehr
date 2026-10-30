use super::*;

// --- #651: a refusal raised in Rust is as much a verdict as one raised in the floor ---

/// The marker survives the `.context(…)` layers every orchestrator adds.
///
/// This is the whole reason it walks the chain rather than reading the outermost error:
/// `register_patient` wraps its failures in context naming the operation, so a check that
/// looked only at the top would answer `false` for every refusal a real call site makes.
#[test]
fn a_deliberate_refusal_is_recognised_through_a_context_chain() {
    let e = deliberate_refusal("birth date \"3/2/1980\" is not a recognised shape")
        .context("asserting the date of birth")
        .context("registering the patient");
    assert!(
        carries_refusal_marker(&e),
        "a verdict that stops being recognisable the moment an orchestrator adds context \
         is a verdict nobody can act on"
    );
}

/// An ordinary failure is NOT one, and this is the dangerous direction to get wrong.
///
/// Calling an outage a refusal tells a clerk to change a form that was never the problem.
#[test]
fn an_ordinary_failure_is_not_a_deliberate_refusal() {
    let e = anyhow::anyhow!("connection closed").context("registering the patient");
    assert!(!carries_refusal_marker(&e));
}

/// The refusal's own sentence reaches the operator rendering unchanged.
///
/// A refusal is legible on purpose, and its text is the only thing that tells the clerk
/// what to change (#648/#651). An earlier version of this line tagged that claim §9.6,
/// which is "The validated submit surface (the write path)" and says nothing about
/// clerk-facing legibility — the mis-attribution came in from `port.rs` (PR #661 review). A Rust-side refusal is held to the same standard, so
/// `operator_chain` must still carry its words.
#[test]
fn a_deliberate_refusals_own_sentence_survives_into_the_operator_rendering() {
    let e = deliberate_refusal("birth date \"3/2/1980\" is not a recognised shape")
        .context("registering the patient");
    assert!(operator_chain(&e).contains("not a recognised shape"));
}

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
    let e =
        anyhow::Error::from(LocalDbFault::new("reading sync cursor", pg)).context("pull cycle 7");
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
