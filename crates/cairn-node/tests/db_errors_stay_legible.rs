//! #467 stays fixed: no DB error in this crate may reach an operator as its own `Display`.
//!
//! # Why a source scan rather than more behavioural tests
//!
//! PR #472 rerouted every `tokio_postgres::Error` rendering in
//! `crates/cairn-node/src/db.rs` through `db_diagnosis::legible_db_error`, and two of them
//! are pinned behaviourally by `tests/db_diagnosis.rs` (the migration door and the connect
//! door). The rest are the same one-line composition, and writing that many more
//! near-identical DB-gated tests would buy little. (This paragraph used to hardcode the
//! counts, which this sweep's own new site immediately made wrong — PR #478 review, I9.)
//!
//! What that leaves unguarded is not any individual site but the **class**: the NEXT
//! site, written six months from now by someone who has never read #467, as the
//! `anyhow!("…: {e}")` that every other Rust codebase writes without thinking. It would be
//! correct-looking, would pass every test in the tree, and would put `db error` back in
//! front of an operator. The CI line that filed #467 —
//! `loading 031_medication: db error` — was exactly that shape.
//!
//! So this guard asserts a property of the FILE, which is the only thing that can catch a
//! site nobody has written yet.
//!
//! # What counts as an offender
//!
//! Any interpolation of a bare binding named `e`/`err`/`error` inside a string in a guarded
//! file — the `{e}` / `{err}` / `{error}` shapes. The legitimate form is `{}` fed by
//! `legible_db_error(&e)`, which is why the check is on the INTERPOLATION and not on the
//! word.
//!
//! **Comment lines are skipped**, and that is not a loophole: a comment renders nothing to
//! an operator, and the three files here explain the defect by NAMING the shape that caused
//! it. A guard that punished the most precise available description of the bug it protects
//! against would push every future writer toward vaguer prose — which is the opposite of
//! what #467 needs. (The skip is deliberately narrow: only a line whose first non-blank
//! characters are `//`. A trailing comment after code is still scanned, which errs in the
//! safe direction. The residual hole is a line INSIDE a multi-line string literal that
//! happens to begin with `//`; no such line exists in this crate, and SQL — the only
//! multi-line literal here — comments with `--`.)
//!
//! # Scope, stated so coverage is not confused with aspiration
//!
//! Five files in `cairn-node`: `db.rs` (the file #467 was filed against, and the one that
//! fails first on a fresh node), `safety.rs` (#473 — the clinical write path), `sync.rs`
//! (#474 — the daemon loop), and `auto_apply.rs` + `matcher_actor.rs` (#477 — the §5.7
//! identity auto-apply ceremony, which links two patient charts with no human in the loop).
//! The last two are one subsystem and were converted together: `auto_apply.rs` alone would
//! have left `resolve_failure_line` — the line that fires when an epoch's actor cannot be
//! resolved at all — rendering `resolve_matcher_actor`'s three unwrapped registry reads.
//!
//! That is **not** every production file in this crate that talks to the database: **28**
//! files under `crates/cairn-node/src/` execute SQL, so **23** sit outside `GUARDED`, and they
//! hold **89** postgres call sites between them — measured 2026-08-23, per-file table in
//! **#485**, which is where this residual is tracked. Not one of those 23 files contains a
//! single `LocalDbFault` or `legible_db_error`, so all 89 are raw.
//!
//! The figure this replaces — "~24 raw `?` sites" — was inherited from before `auto_apply.rs`
//! was converted and never re-measured. It understated the residual more than threefold, in the
//! one paragraph a maintainer sizes #485 from (PR #486 review), which is why what stands here
//! now is a count with a date and an issue behind it rather than a tilde.
//!
//! Those sites are ugly-but-not-silent rather than silent — a bare `?` preserves `source()`,
//! so `anyhow`'s chain printing still reaches the `DbError` — but they name no operation, so
//! an operator learns the cause and not what was being attempted.
//!
//! An earlier draft of this paragraph claimed the three files WERE the whole set. That
//! claim was false, contradicted by this repo's own HANDOVER, and worse than the honest
//! gap it replaced: a reader who believes the crate is covered never widens the guard
//! (PR #478 review, finding 3). Add a file to `GUARDED` when its sites are fixed — each
//! one converted is a durable ratchet.
//!
//! `cairn-sync`'s `main.rs` is deliberately NOT in `GUARDED`. It carries the twin renderer
//! and its loops were fixed alongside these (#475, #471, #479), but it is one 10,000-line
//! file mixing production code with its own test modules, and dozens of its `{e}`-shaped
//! sites render errors that are not database errors at all — hex decoding, serde, I/O, and
//! `ApplyError`, which is NOT legible by construction (its `None` arm is `e.to_string()`;
//! that is #480). A name-based scan over that file would be mostly false positives, and a
//! guard whose failures are usually noise is one people learn to silence. Splitting
//! `main.rs` is separate work (#402's shape).
//!
//! It is not left to its behavioural tests alone, though. #479's run-loop sites are pinned
//! BY COUNT AND BY SHAPE below, the same technique `sync.rs`'s `LocalDbFault` discipline
//! uses and for the same reason — this file reads by repo path, so a guard over another
//! crate's source belongs here rather than in a fourth copy of the machinery.
//!
//! # When a site here IS a false positive
//!
//! The predicate is name-based, because a source scan cannot type-check. A binding that
//! genuinely does not hold a database error — an `io::Error` from `accept`, a serde failure
//! — is resolved by NAMING it (`accept_err`), never by suppressing the check. The rename is
//! only acceptable when the new name says what the value IS: that leaves the source more
//! informative than `e` was, which is what makes it a fix rather than a dodge.
//!
//! **A rename is not, by itself, proof.** The first version of this widening renamed five
//! bindings in `sync.rs`, and two of them (`session_err`, `pull_err`) genuinely DID hold
//! database errors on some branches — so the guard reported green over two live instances
//! of the defect it had just been widened to catch (PR #478 review, findings 1, 2 and 9).
//! Both now render through `db_diagnosis::operator_chain`, which walks the whole `anyhow`
//! chain. Before renaming a binding, establish that every branch reaching it is
//! non-database; if any branch is, render the chain instead.
//!
//! The predicate cannot see two further shapes, stated so they are not mistaken for
//! coverage: the positional `format!("…: {}", e)` form, and any renamed binding at all.
//! That is why `sync.rs`'s `LocalDbFault` discipline is pinned by COUNT below rather than
//! by the interpolation scan.

#[path = "common/sources.rs"]
mod sources;

/// The files whose DB errors must stay legible. See the module doc for which files these
/// are and why `cairn-sync`'s `main.rs` is not among them.
const GUARDED: &[&str] = &[
    "crates/cairn-node/src/auto_apply.rs",
    "crates/cairn-node/src/db.rs",
    "crates/cairn-node/src/matcher_actor.rs",
    "crates/cairn-node/src/safety.rs",
    "crates/cairn-node/src/sync.rs",
];

/// How many wrapped postgres calls `auto_apply.rs` carries (#477).
///
/// Every postgres call in that file that PROPAGATES is wrapped. Two sit outside that
/// population, named here so a future audit reconciling this constant against `grep -c` need
/// not rediscover them: `db::next_hlc` returns `anyhow` rather than a `tokio_postgres::Error`
/// and does its own naming, and the best-effort `pg_advisory_unlock` is a `let _ =` that
/// deliberately swallows. An earlier draft said "every postgres call in that file propagates",
/// which the unlock contradicts — and the unlock is precisely the swallowed one, so the claim
/// omitted the case it most needed to name (PR #486 review). See #488 for the swallow itself.
///
/// The count is the same crude, effective instrument as [`SYNC_LOCAL_DB_FAULT_SITES`] next
/// door, and it is needed for the same reason: reverting a wrapper to a bare `?` compiles,
/// leaves the interpolation scan green (no `{e}` appears) and leaves the two
/// operator-line tests green (they build their own error and never assert that a
/// production site produces one).
const AUTO_APPLY_LOCAL_DB_FAULT_SITES: usize = 10;

/// The shape counted for `auto_apply.rs` — narrower than `sync.rs`'s, deliberately.
///
/// `sync.rs` counts the bare `LocalDbFault::new(` because one of its twelve sites spans
/// several lines and no single-line shape would match it. That works there because nothing
/// in `sync.rs` builds one outside production code. `auto_apply.rs`'s test module DOES —
/// `a_failed_apply_names_the_pair_and_the_diagnosis` constructs one to drive the operator
/// line — so the bare form counts eleven and would report a *drop* to ten as healthy the
/// day someone deletes that test. Counting the `.map_err` shape keeps the two populations
/// apart. Stated rather than left as an inconsistency between two adjacent guards.
const AUTO_APPLY_WRAPPED_CALL: &str = ".map_err(|e| LocalDbFault::new(";

/// How many wrapped postgres calls `matcher_actor.rs` carries (#477).
///
/// Three: the `actor_current` read, the `actor_event` enroll-history read, and the
/// `enroll_actor` write. `auto_apply.rs`'s ceremony calls into this file, so leaving them
/// bare would have kept `db error` on the very line #477 names first.
const MATCHER_ACTOR_LOCAL_DB_FAULT_SITES: usize = 3;

/// The `LocalDbFault` discipline across the auto-apply ceremony, pinned by count (#477).
///
/// What is lost by a revert is narrower than in `sync.rs` — this file has no partition
/// classifier reading the chain — but it is what #477 is about: the operator learns the
/// SQLSTATE and NOT which step of the §5.7 auto-apply ceremony met it. On a path that
/// links two patient charts with no human in the loop, and whose caller counts the failure
/// and continues, that is the difference between one missing grant and nine unrelated
/// causes.
#[test]
fn every_postgres_call_in_the_auto_apply_ceremony_names_what_it_was_doing() {
    let root = sources::repo_root();
    // `flattened_code`, never the raw text: a deleted wrapper left behind in the comment
    // that explains its deletion would otherwise keep this count at ten (PR #486 review).
    let text = flattened_code(
        &std::fs::read_to_string(root.join("crates/cairn-node/src/auto_apply.rs"))
            .expect("auto_apply.rs is in the tree"),
    );
    let found = text.matches(AUTO_APPLY_WRAPPED_CALL).count();

    let in_matcher_actor = flattened_code(
        &std::fs::read_to_string(root.join("crates/cairn-node/src/matcher_actor.rs"))
            .expect("matcher_actor.rs is in the tree"),
    )
    .matches(AUTO_APPLY_WRAPPED_CALL)
    .count();
    assert_eq!(
        in_matcher_actor, MATCHER_ACTOR_LOCAL_DB_FAULT_SITES,
        "matcher_actor.rs has {in_matcher_actor} wrapped postgres calls, expected \
         {MATCHER_ACTOR_LOCAL_DB_FAULT_SITES}. It is the other half of the same ceremony: \
         unwrapped, `auto-apply resolve epoch '…'` goes back to saying `db error` (#477)."
    );

    assert_eq!(
        found, AUTO_APPLY_LOCAL_DB_FAULT_SITES,
        "auto_apply.rs has {found} `{AUTO_APPLY_WRAPPED_CALL}` sites, expected \
         {AUTO_APPLY_LOCAL_DB_FAULT_SITES}. If you ADDED a postgres call, wrap it and bump \
         the constant. If this DROPPED, a call was reverted to a bare `?` — which leaves \
         the SQLSTATE reachable but no longer says WHICH step of the ceremony failed (#477)."
    );
}

/// Bindings that, interpolated raw, render a `tokio_postgres::Error` as its useless kind.
const RAW_ERROR_BINDINGS: &[&str] = &["e", "err", "error"];

/// Is this line a comment, and therefore incapable of rendering anything to an operator?
///
/// Only a line whose first non-blank characters are `//` — which covers `//`, `///` and
/// `//!`. A trailing comment after real code is deliberately NOT excluded: that line still
/// contains code, and erring toward scanning it is the safe direction for a guard.
///
/// This exists because the widening pass found the guard's only three "offenders" were
/// comments EXPLAINING the defect, quoting the exact shape that caused it. See the module
/// doc for why naming the shape is worth protecting.
fn is_a_comment_line(line: &str) -> bool {
    line.trim_start().starts_with("//")
}

/// The file's CODE — comments removed, all whitespace flattened to single spaces. **Pure.**
///
/// # Why the presence guards need this, and the interpolation scan does not
///
/// The scan below asks *does this line render an error badly?*, so a comment is harmless and
/// being over-eager on code is the safe direction. The three guards ABOVE ask the opposite
/// question — *is this site still here?* — and for that question a comment is not harmless,
/// it is a forgery. Delete `do_pull`'s `map_err` and leave the old line behind in a `//`
/// comment explaining what changed, and `contains`/`matches` over the raw text still says yes
/// over a live regression on the first statement of every pull cycle. This repo quotes exact
/// code shapes in prose constantly — this very file does it eight times in
/// [`SYNC_DAEMON_RENDERINGS`] — so that is house style, not a contrived bypass. A guard that
/// reports the same green over a reverted site is the #387 species (PR #486 review).
///
/// Trailing comments are truncated too, unlike [`is_a_comment_line`]'s deliberate leniency:
/// for a PRESENCE question the safe direction is inverted. Truncating at `//` can only remove
/// text, so it can only turn a match into a miss — a false RED, which a human reads. The
/// alternative — a trailing comment able to satisfy a shape — is a false green.
///
/// Whitespace is flattened so a shape can be written as one logical line whatever rustfmt did
/// with it. The trust-set entry below spans three source lines; without this it could only be
/// pinned by embedding its exact indentation, which would go red on reformatting alone.
fn flattened_code(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for line in text.lines() {
        if is_a_comment_line(line) {
            continue;
        }
        let code = line.split("//").next().unwrap_or(line);
        for word in code.split_whitespace() {
            out.push_str(word);
            out.push(' ');
        }
    }
    out
}

/// [`flattened_code`] keeps code, drops comments, and survives reflow.
#[test]
fn a_commented_out_site_cannot_stand_in_for_a_live_one() {
    // The bypass that passed all three presence guards before this existed: the site is
    // gone, its shape survives in the comment explaining why.
    let reverted = "    .query(&sql, &[])?\n    // was: .map_err(|e| LocalDbFault::boxed(\"listing the quarantine pen\", e))?";
    assert!(
        !flattened_code(reverted).contains(".map_err(|e| LocalDbFault::boxed("),
        "a whole-line comment must not satisfy a presence guard"
    );

    // …nor a trailing one, which `is_a_comment_line` deliberately does not exclude.
    assert!(
        !flattened_code("    let x = 1; // .map_err(|e| LocalDbFault::new(")
            .contains("LocalDbFault::new("),
        "a trailing comment must not satisfy a presence guard either"
    );

    // Real code survives, and a call rustfmt split across lines reads as one.
    let split = "            eprintln!(\n                \"lookup failed: {}\",\n                legible_db_error(&e)\n            );";
    assert!(
        flattened_code(split).contains(r#""lookup failed: {}", legible_db_error(&e)"#),
        "a reflowed call must still match a flat shape: {:?}",
        flattened_code(split)
    );

    // The doc-comment forms this repo uses for prose are all dropped.
    for prose in ["/// {e}", "//! `{e}`", "  // {e}"] {
        assert!(
            flattened_code(prose).trim().is_empty(),
            "{prose:?} is prose"
        );
    }
}

/// Does this line interpolate one of the raw error bindings into a string?
///
/// Deliberately simple and slightly over-eager on CODE: it is a guard, and a false positive
/// costs one `legible_db_error` call or one rename that names the error's kind, while a
/// false negative costs an operator their diagnosis. Pure, so the judgement is testable
/// without touching disk.
fn interpolates_a_raw_error(line: &str) -> bool {
    if is_a_comment_line(line) {
        return false;
    }
    RAW_ERROR_BINDINGS
        .iter()
        .any(|b| line.contains(&format!("{{{b}}}")))
}

/// Every `sync.rs` postgres call that PROPAGATES is wrapped in `LocalDbFault`.
///
/// Twelve is not a magic number — it is the count of postgres calls in `sync.rs` that
/// return their error to a caller. (The thirteenth, `SELECT apply_remote_node_event`, is
/// matched inline and never propagates, so it has no wrapper to lose.) Bump it
/// deliberately when a query is added or removed, exactly as `twin_registry.rs` and
/// `db/tests/034` are bumped.
const SYNC_LOCAL_DB_FAULT_SITES: usize = 12;

/// The `LocalDbFault` discipline in `sync.rs`, pinned by count (PR #478 review, finding 8).
///
/// # Why a count, when the file already states the rule in a doc comment
///
/// Reverting any one `map_err(|e| LocalDbFault::new(…))` to `.context(…)` compiles, leaves
/// the interpolation scan above green (no `{e}` appears), and leaves
/// `tests/pull_failure_class.rs` green (it builds its own `LocalDbFault` and never asserts
/// that a production site produces one). Yet it reinstates BOTH defects at once: the line
/// loses its SQLSTATE, and — because `pull_failure_class` walks the chain looking for a
/// `tokio_postgres::Error` that `.context()` leaves in place but `anyhow!` does not — the
/// discipline that keeps the chain intact stops being verifiable at all. That is issue
/// #474 item 3's machinery, unpinned.
///
/// A count is crude, and it is the only thing that catches site 13 written six months from
/// now — the same argument the interpolation scan above makes for itself.
#[test]
fn every_propagating_postgres_call_in_sync_is_wrapped_in_a_local_db_fault() {
    let root = sources::repo_root();
    // Comment-stripped for the same reason as the auto-apply count next door.
    let text = flattened_code(
        &std::fs::read_to_string(root.join("crates/cairn-node/src/sync.rs"))
            .expect("sync.rs is in the tree"),
    );
    let found = text.matches("LocalDbFault::new(").count();

    assert_eq!(
        found, SYNC_LOCAL_DB_FAULT_SITES,
        "sync.rs has {found} `LocalDbFault::new(` sites, expected \
         {SYNC_LOCAL_DB_FAULT_SITES}. If you ADDED a postgres call, wrap it and bump the \
         constant. If this DROPPED, a call was reverted to `.context()` — which loses the \
         SQLSTATE and unpins the chain the partition classifier reads (#474 item 3)."
    );
    assert!(
        !text.contains(".context(\"checkpointing"),
        "the cursor checkpoint is the canonical #474 item 3 site: it must never be a \
         `.context()`, whatever the count says"
    );
}

/// `cairn-sync`'s daemon-loop sites, pinned by the exact shape that renders them (#479).
///
/// Each entry is `(what it is, the shape that must still be there)`. **Every shape includes
/// the RENDERING CALL, not just the format string.** The first cut of the last two stopped at
/// the format string and its comma, so reverting those two sites to a bare `e` put `db error`
/// back on the byte tier AND on the serve trust-set lookup while this test still reported
/// green — measured, PR #486 review. A pin that survives the revert it names is the #387
/// species, and those two were it.
///
/// Shapes are matched against [`flattened_code`], so each is written as one logical line
/// whatever rustfmt did with it — the trust-set entry spans three source lines — and each is
/// deliberately long enough to be unambiguous: several include the `.map_err(|e| …)?`
/// wrapper, which no test in that file writes, so a test fixture using the same operation
/// phrase cannot stand in for a reverted production site.
const SYNC_DAEMON_RENDERINGS: &[(&str, &str)] = &[
    (
        "do_pull's sync_state upsert — the FIRST statement of a cycle, before any network \
         I/O, and the one every later cycle fails at once the database is gone",
        r#".map_err(|e| LocalDbFault::boxed("registering this peer in sync_state", e))?"#,
    ),
    (
        "do_pull's cursor read — the second pre-network statement",
        r#".map_err(|e| LocalDbFault::boxed("reading this peer's sync cursor", e))?"#,
    ),
    (
        "do_requeue's opening query — the statement PR #478 left behind when it converted \
         the three inside the loop (#471's own command)",
        r#".map_err(|e| LocalDbFault::boxed("listing the quarantine pen", e))?"#,
    ),
    (
        "the JSONL `pull_error` key, which bet_a.py reads",
        "line[\"pull_error\"] = serde_json::json!(operator_chain(e));",
    ),
    (
        "the operator's terminal line for a failed cycle",
        r#"": PULL FAILED: {}", operator_chain(e.as_ref())"#,
    ),
    (
        "the fingerprint failure arm, which had no `else` at all — BOTH surfaces, since a \
         first cut wrote only to stderr while `bet_a.py` reads the JSONL",
        "Err(e) => { record_fingerprint_failure(&mut line, e.as_ref()); \
         eprintln!(\"{}\", fingerprint_error_line(e.as_ref())); }",
    ),
    (
        "the byte tier's chunk insert",
        r#""blob_chunk insert failed: {}", legible_db_error(&e)"#,
    ),
    (
        "the serve trust-set lookup — the AUTHORIZATION path for an inbound peer",
        r#""cairn-sync serve: trust-set lookup for puller {kid} failed: {}", legible_db_error(&e)"#,
    ),
];

/// How many `.map_err(|e| LocalDbFault::boxed(` sites `cairn-sync`'s daemon carries.
///
/// **What this count does and does not catch.** It counts WRAPPERS, not postgres calls, so a
/// new call written without one leaves it at three and passes. This test's own doc below says
/// exactly that — "it protects the sites that were fixed, and it does NOT protect the next one
/// somebody writes" — and an earlier draft of this sentence claimed the opposite, two lines
/// from the sentence contradicting it (PR #486 review). What it DOES catch is a revert: drop a
/// wrapper and the count drops with it. Bumping it is the forced acknowledgement when a
/// wrapped site is added. The shapes above also catch a revert and name which site; the count
/// catches one whose shape was edited rather than deleted.
const SYNC_DAEMON_LOCAL_DB_FAULT_SITES: usize = 3;

/// `cairn-sync`'s run loop keeps rendering its database failures (#479).
///
/// # Why this guard lives in `cairn-node`'s test tree
///
/// It reads by repo path, exactly as the `sync.rs` count above does, so nothing about it
/// is crate-specific. The alternative — a fourth guard binary in `cairn-sync/tests/` —
/// would have needed a fourth copy of the comment-stripping and file-reading machinery,
/// which is the #452 species this repo has already paid for once.
///
/// # Why shapes and a count, rather than the interpolation scan
///
/// The module doc gives the reason `main.rs` is not in `GUARDED`: a name-based scan over
/// 10.1k lines mixing production and test code, most of whose `{e}` sites hold errors that
/// are not database errors, would be mostly noise. So the sites #479 names are pinned
/// individually. That is narrower than a scan and honest about being so: it protects the
/// sites that were fixed, and it does NOT protect the next one somebody writes.
///
/// Each of these reverts cleanly to something that compiles and leaves the behavioural
/// tests green — `record_pull_failure`'s test would still pass if `cmd_run`'s terminal
/// line went back to `{e}`, because the two lines are separate statements over the same
/// error.
#[test]
fn the_cairn_sync_run_loop_still_renders_its_database_failures() {
    let root = sources::repo_root();
    let path = root.join("crates/cairn-sync/src/main.rs");
    // Comment-stripped AND whitespace-flattened: the raw text let a `//` line stand in for
    // a deleted site, and the flattening is what lets each shape below be written as one
    // logical line regardless of how rustfmt wrapped it.
    let text = flattened_code(
        &std::fs::read_to_string(&path).expect("cairn-sync's main.rs is in the tree"),
    );

    let missing: Vec<&str> = SYNC_DAEMON_RENDERINGS
        .iter()
        .filter(|(_, shape)| !text.contains(shape))
        .map(|(what, _)| *what)
        .collect();

    assert!(
        missing.is_empty(),
        "these cairn-sync daemon sites no longer render their database failure: \
         {missing:#?}\n\nEach one reverted puts `db error` back in front of an operator — \
         and `cmd_run` builds its client ONCE outside the loop, so after the database goes \
         away the line repeats every cycle for the life of the process (#479). If a site \
         was legitimately moved or renamed, update the shape here in the same commit."
    );

    let wrapped = text.matches(".map_err(|e| LocalDbFault::boxed(").count();
    assert_eq!(
        wrapped, SYNC_DAEMON_LOCAL_DB_FAULT_SITES,
        "cairn-sync's daemon has {wrapped} `.map_err(|e| LocalDbFault::boxed(` sites, \
         expected {SYNC_DAEMON_LOCAL_DB_FAULT_SITES}. If you ADDED a propagating postgres \
         call, wrap it and bump the constant. If this DROPPED, a site was reverted to a \
         bare `?` — which loses the operation name AND, because `classify_pull_failure` \
         walks the chain for a `postgres::Error`, would be silently reinstated as a \
         `partition` if the wrapper were replaced by a String error instead (#479)."
    );
}

#[test]
fn no_db_error_in_a_guarded_file_reaches_an_operator_as_its_kind() {
    let root = sources::repo_root();
    let mut offenders: Vec<String> = Vec::new();

    for rel in GUARDED {
        let path = root.join(rel);
        let text = sources::read_source(&path);
        for (n, line) in text.lines().enumerate() {
            if interpolates_a_raw_error(line) {
                offenders.push(format!("{rel}:{} — {}", n + 1, line.trim()));
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "#467: a database failure must never reach an operator as `tokio_postgres::Error`'s \
         own Display — that is the literal string \"db error\" for a server-side failure, and \
         a bare kind name (\"error connecting to server\") for everything else. Wrap the \
         error in `db_diagnosis::legible_db_error(&e)` and interpolate THAT.\n\n{}",
        offenders.join("\n")
    );
}

/// The guard's own predicate, pinned — a guard whose judgement is wrong is worse than no
/// guard, because it reports the same green.
#[test]
fn the_predicate_catches_the_shape_that_filed_the_issue_and_spares_the_fix() {
    // The exact line from `db.rs` as it stood when #467 was filed.
    assert!(interpolates_a_raw_error(
        r#"            .map_err(|e| anyhow::anyhow!("loading {name}: {e}"))"#
    ));
    assert!(interpolates_a_raw_error(r#"eprintln!("failed: {err}")"#));
    assert!(interpolates_a_raw_error(r#"format!("{error}")"#));

    // The fix, which must not trip it.
    assert!(!interpolates_a_raw_error(
        r#"            .map_err(|e| anyhow::anyhow!("loading {name}: {}", legible_db_error(&e)))"#
    ));
    // A name that merely CONTAINS a guarded binding is not one.
    assert!(!interpolates_a_raw_error(
        r#"format!("{event}: {errors_seen}")"#
    ));

    // A comment cannot render anything to an operator, and all three files here explain
    // the defect by naming its shape. Every comment form must be spared.
    assert!(!interpolates_a_raw_error(
        r#"    // and `{e}` printed `db error` in its place."#
    ));
    assert!(!interpolates_a_raw_error(
        r#"/// flags `{e}` in this file, and its predicate is name-based."#
    ));
    assert!(!interpolates_a_raw_error(r#"//! `anyhow!("…: {e}")`"#));

    // …but a trailing comment does not launder the code beside it.
    assert!(interpolates_a_raw_error(
        r#"eprintln!("failed: {e}"); // TODO: make legible"#
    ));
}
