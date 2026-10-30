//! Which failures are VERDICTS and which are accidents (#651, #797): the refusal markers a
//! surface reads to decide whether to offer a retry, a re-read or only an edit, and the
//! commit-outcome-unknown marker that tells a surface a write MAY have landed (#713, #815).
//!
//! Split out of `db_diagnosis.rs` (#810), which keeps the rendering half; the parent re-exports
//! every item here, so callers still name them `db_diagnosis::…`.
use super::operator_chain;

/// WHAT a [`DeliberateRefusal`] is a verdict about — and therefore whether a surface may offer
/// any way forward other than editing the form.
///
/// # Why one bit was not enough
///
/// `DeliberateRefusal` began as a single bit: *this node decided this, and deciding it again
/// will decide the same*. That bit is what separates a verdict from an accident, and it is
/// correct. But it collapsed two situations a clerk experiences completely differently, and the
/// constructor's own precondition — *"nothing about the environment took part in the decision;
/// a retry must be pointless by construction"* — was true of only one of them:
///
/// - [`RefusalScope::Input`] — `dob_precision("3/2/1980")`. Pure. No act by anybody makes that
///   call succeed. The only way forward is to change what was typed.
/// - [`RefusalScope::NodeState`] — `not_enrolled_refusal`. Decided *after* a database round trip
///   over mutable, append-only registry state. Retrying the identical call is still pointless,
///   so it is genuinely a refusal and not an outage — but it is pointless *until an operator
///   runs a named command*, at which point the same call succeeds. The form was never wrong.
///
/// Rendering the second like the first is the dead-end-remedy shape this subsystem exists to
/// close: it hands the clerk a sentence naming a command they cannot run, and no way to try
/// again once somebody has. Rendering it like an *outage* is the other error — it offers a
/// retry-now on a node that will refuse identically until provisioned, and hides the remedy.
/// The scope is what lets a surface be right in both directions (PR #661 review).
///
/// **Adding a scope must never subtract the bit.** [`carries_refusal_marker`] answers *"is this
/// a verdict"* and stays independent of this enum, so a future third scope cannot silently turn
/// a verdict back into an accident. Pinned by `every_scope_is_still_a_refusal`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefusalScope {
    /// A verdict about the **input**: pointless to retry forever, by anybody. A record that
    /// changed under the caller is NOT this: it is [`RefusalScope::Stale`].
    Input,
    /// A verdict about **this node's state**: pointless to retry unchanged, but the identical call
    /// succeeds once that state changes — by an operator act (the enrolment refusals, which name
    /// the command) or by sync delivering what is missing (`chart_link`'s "not held here"
    /// refusal, which no operator can hasten and which names no command).
    NodeState,
    /// A verdict about the caller's PICTURE: the record changed since the caller read it. The
    /// same call is refused until the caller re-reads; re-reading is the whole remedy.
    Stale,
}

/// A refusal this node raised **deliberately, in Rust** — almost always before any statement
/// reached Postgres; the few decided by a read (`chart_link`'s held-chart and record checks) say
/// so at their call sites.
///
/// # The question this answers
///
/// Every caller that renders a failure to a human has to decide one thing: was this a *verdict*
/// about the call, or an *accident* that befell it? Getting it backwards does real harm in both
/// directions — calling an outage a refusal tells a clerk to change a form that was never the
/// problem, and calling a refusal an outage hands them a retry button for a verdict, which they
/// will press.
///
/// For refusals the in-DB floor raises there is already an answer: a bare `RAISE EXCEPTION` is
/// SQLSTATE `P0001`, which `db/001_envelope.sql` states is a contract and
/// `crates/cairn-node/tests/floor_refusals_carry_no_errcode.rs` enforces across every
/// `db/*.sql` (#633).
///
/// That answer is unavailable for a refusal raised **above** the database.
/// [`crate::patient::register::dob_precision`] validates the shape of a date of birth up front,
/// deliberately, so a malformed one refuses the whole call with zero side effects — no HLC tick,
/// no partial chart (#350). That refusal is every bit as deterministic as the floor's: the same
/// string refuses identically forever. But it carried no SQLSTATE anywhere in its chain, so it
/// was **indistinguishable from a dropped connection** — and the clerk most likely to meet it is
/// the one on a desk with no date widget who typed `3/2/1980`, searched fine, found nothing, and
/// is then offered a retry that can never work. See
/// [#651](https://github.com/cairn-ehr/cairn-ehr/issues/651).
///
/// # Why a type and not a convention
///
/// The alternative #651 weighed was a marker layer or a sentinel string on the chain. A type
/// with a private field, constructible only through `deliberate_refusal` (crate-private, so the
/// link is deliberately plain text — see that function for why the mint is narrow), means
/// *"is this a
/// verdict"* is answered by the compiler rather than by a convention — and a convention is
/// precisely what #648 was trying to get away from.
///
/// # What it is NOT
///
/// It is not a claim that a failure carrying it is *safe*, and it is not a second error
/// vocabulary competing with [`LocalDbFault`](super::LocalDbFault). It is one bit — *this node
/// decided this, and deciding it again will decide the same* — travelling beside a message
/// already written for a human to read.
#[derive(Debug)]
pub struct DeliberateRefusal {
    /// The operator/clerk-facing sentence. Private: the only way to build one is
    /// `deliberate_refusal` or `node_state_refusal` (both crate-private), so the marker cannot
    /// be attached to a message by accident, and cannot be attached at all from outside this
    /// crate.
    message: String,
    /// What the verdict is about. Private for the same reason as `message`, and read from
    /// outside through [`refusal_scope`] — a caller may branch on the scope but may not mint
    /// one, so "which kind of verdict is this" stays a claim only this crate makes.
    scope: RefusalScope,
}

impl std::fmt::Display for DeliberateRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for DeliberateRefusal {}

/// Build a deterministic refusal as an `anyhow::Error`, ready to be `?`'d and contextualised.
///
/// **`pub(crate)` on purpose.** Declaring that a failure is a VERDICT — that no retry can ever
/// change it — is a claim only this crate is positioned to make, because only this crate knows
/// which of its own pre-flight checks are deterministic. Keeping the mint in-crate turns "who
/// may declare a verdict" into a compiler question rather than a doc question. Widening it is
/// easy and should be a deliberate, argued act (PR #661 review).
///
/// Use this **only** for a verdict about the INPUT: the same inputs refuse the same way
/// forever, and nothing about the environment — a connection, a lock, a disk, a clock — took
/// part in the decision. **A retry must be pointless by construction, for everybody.** If a
/// retry might work, this is the wrong constructor and an ordinary error is the honest answer;
/// marking a transient failure as a verdict tells the clerk to change something that was
/// already correct.
///
/// If the refusal is decided by how the node is *provisioned* rather than by what was typed —
/// so that an operator act makes the identical call succeed — use [`node_state_refusal`]
/// instead. Both are verdicts; only the way forward differs. See [`RefusalScope`].
///
/// A judgement made FROM a record that has since changed is not input: it is
/// [`stale_picture_refusal`]'s.
pub(crate) fn deliberate_refusal(message: impl Into<String>) -> anyhow::Error {
    anyhow::Error::new(DeliberateRefusal {
        message: message.into(),
        scope: RefusalScope::Input,
    })
}

/// Build a refusal that is a verdict about **this node's state**.
///
/// Same marker, same *"retrying this unchanged is pointless"* guarantee, different way forward:
/// once the node's state changes, the identical call succeeds. So a surface may — must — keep a
/// way to try again, which it must NOT do for [`deliberate_refusal`].
///
/// `pub(crate)` for the reason [`deliberate_refusal`] gives: only this crate knows which of its
/// own checks are decided by node state rather than by luck. The current call sites, each minted
/// after a read whose *answer* stays the same until the node's state changes (PR #661 review):
/// - the three [`crate::actor_enrolment`] refusals and `chart_link`'s enrolled-human pre-check —
///   changed by an operator act, which the enrolment messages name;
/// - `chart_link`'s held-chart pre-check (R2b-1) — changed by sync delivering the chart or the
///   record joining the pair; no operator is involved and no command is named (PR #707 review);
/// - `chart_link::admit`'s open-chart `NotHeld` arm (R2b-2) — the chart judged from has no
///   `patient_chart` row here yet; changed the same way (PR #711 review);
/// - `chart_link::standing_link`'s legacy-address arm (#787) — a pre-#115 `patient_link` row
///   with no content address; changed by an operator's `cairn-node reproject --prefix identity.`
///   (a heal; a whole `--rebuild` as the fallback), which the message names.
pub(crate) fn node_state_refusal(message: impl Into<String>) -> anyhow::Error {
    anyhow::Error::new(DeliberateRefusal {
        message: message.into(),
        scope: RefusalScope::NodeState,
    })
}

/// Build a refusal that is a verdict about the caller's **picture** of the record: what the
/// caller acted on has since changed (a colleague's link landed), so this call is refused until
/// they re-read. Unlike [`deliberate_refusal`] the input was not wrong, and unlike
/// [`node_state_refusal`] no operator act or sync helps: only a fresh read does, which is why a
/// surface can route this scope to "re-read the chart" without matching on the sentence.
///
/// The sentence says only what happened (the CLI shows it too); the remedy is the scope's job.
pub(crate) fn stale_picture_refusal(message: impl Into<String>) -> anyhow::Error {
    anyhow::Error::new(DeliberateRefusal {
        message: message.into(),
        scope: RefusalScope::Stale,
    })
}

/// What a deliberate refusal on this chain is a verdict **about**, if it is one at all.
///
/// `None` means no marker anywhere on the chain — *"not one of these"*, exactly as
/// [`carries_refusal_marker`] returning `false` does. It does **not** mean "not a refusal": the
/// floor's own `P0001` refusals carry no marker, so a caller must still ask the SQLSTATE
/// question. The two discriminators are complementary; see [`carries_refusal_marker`].
///
/// Walks the whole chain, and returns the **innermost** marker's scope — the one the refusal was
/// minted with, not one a later layer could imply — because `.context(…)` layers add operation
/// names above it and never a second marker.
pub fn refusal_scope(e: &anyhow::Error) -> Option<RefusalScope> {
    e.chain()
        .filter_map(|cause| cause.downcast_ref::<DeliberateRefusal>())
        .last()
        .map(|refusal| refusal.scope)
}

/// The line the CLI prints after a [`RefusalScope::Stale`] refusal (#797 review).
///
/// A stale refusal's sentence says only what happened ("… — nothing was done"): its remedy is the
/// SCOPE's job, because the window acts on it itself (it re-reads the whole chart). The CLI has no
/// such reflex, and before #797 it read the remedy out of the sentence — so it now adds one line of
/// its own, rather than leave an operator at "nothing was signed" with no next step.
pub const CLI_STALE_REMEDY: &str =
    "The record changed after this command read it. Read it again, and run the command once more \
     if it still applies.";

/// The CLI's own remedy line for this failure, if it needs one: [`CLI_STALE_REMEDY`] for a
/// [`RefusalScope::Stale`] refusal, the check to run for a [`CommitOutcomeUnknown`] (#822),
/// `None` for everything else (whose sentences already say what to do, or are accidents with
/// nothing to add). Pure, so `main.rs`'s printer stays one line.
///
/// The unknown-outcome line exists because the marker asks the operator to "check whether it was
/// recorded", and before #822 the CLI had no command to check with: only raw psql. It names
/// `cairn-node event`, which says whether this node holds that event. `--conn` is written
/// before the subcommand because it is a top-level flag, not a global one. The line only says what
/// a found event means: what "not found" means (only "not yet", while a cut-off session may still
/// be committing) is `cairn-node event`'s own answer to give, at the moment it is true. It claims
/// nothing about the command's OTHER writes: a command that committed earlier steps says what
/// those did in its own sentence.
pub fn cli_remedy(e: &anyhow::Error) -> Option<String> {
    if refusal_scope(e) == Some(RefusalScope::Stale) {
        return Some(CLI_STALE_REMEDY.to_string());
    }
    commit_outcome_unknown_event(e).map(|id| {
        format!(
            "To check, run `cairn-node --conn <the same> event {id}`: it says whether this \
             node holds that event. If it does, the write took effect: do not run the command \
             again."
        )
    })
}

/// Did this node deliberately refuse, above the database?
///
/// # Named for the DISCRIMINATOR, not the conclusion — and that matters here
///
/// **Four** sibling items are called some spelling of *deliberate refusal*, and they mean the
/// opposite thing about SQLSTATE. All four test for **`P0001`**:
///
/// - `crate::restore::clinical::refusal_is_deliberate` — **in this very crate**, and therefore
///   the one most easily mistaken for this function;
/// - `cairn-sync`'s `refusal_is_deliberate` and its separate `is_deliberate_refusal` — two
///   distinct items, not one;
/// - `cairn-gui-live::error::refusal_is_deliberate`, which is in the **`cairn-gui` workspace**,
///   not this one.
///
/// This function is true exactly when there is **no SQLSTATE at all**. A reader grepping the
/// obvious name finds four items and no way to tell which is which, so this one says what it
/// inspects: a marker on the chain. (PR #661 review — an earlier version of this paragraph
/// said "two" and "three", omitted the sibling in this same crate, and called `cairn-gui-live`
/// part of "this workspace". #652/#655 carry the consolidation this list argues for.)
///
/// Walks the **whole** `anyhow` chain, for the same reason [`operator_chain`] and
/// `cairn-gui-live`'s `sqlstate_of` do: every orchestrator adds `.context("…")` layers naming
/// the operation, so a check that read only the outermost error would answer `false` for every
/// refusal a real call site produces.
///
/// A `false` here means only *"not one of these"*. A caller must still ask the SQLSTATE question
/// about the floor's own refusals — the two discriminators are complementary, not alternatives,
/// and `cairn-gui-live`'s `data_error_from` consults both.
pub fn carries_refusal_marker(e: &anyhow::Error) -> bool {
    e.chain()
        .any(|cause| cause.downcast_ref::<DeliberateRefusal>().is_some())
}

/// A write whose COMMIT was interrupted, so whether it landed is unknown (#713).
///
/// A connection lost DURING `COMMIT` is the one failure after which the database may or may not
/// hold the event ([`commit_failure`] decides which failures count: also an autocommit statement's,
/// and a FATAL or PANIC answer). It is neither a verdict (nothing was decided about the call) nor an ordinary
/// outage (an outage wrote nothing, so the same act may simply be tried again). Treating it as an
/// outage is the #713 hazard: a judgement that DID commit — say as `Outranked`, so the record
/// still reads as before — invites a second press, and that second event, stamped above a
/// colleague's, silently overrules them. The only safe way forward is to RE-READ before acting
/// again, and this marker is what lets a surface know to do that instead of offering a retry.
///
/// Distinct from [`DeliberateRefusal`] on purpose: [`carries_refusal_marker`] and
/// [`refusal_scope`] answer `false`/`None` for it, so no surface can mistake an unknown outcome
/// for a decided one. Ask [`commit_outcome_unknown_event`].
#[derive(Debug)]
pub struct CommitOutcomeUnknown {
    /// The event whose commit was interrupted — what the operator looks for.
    event_id: uuid::Uuid,
    /// The commit's own failure, kept as this error's `source()` so a chain walk finds the
    /// marker FIRST and still reaches what went wrong (`operator_chain` renders both).
    cause: anyhow::Error,
}

impl std::fmt::Display for CommitOutcomeUnknown {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "commit outcome unknown for event {} — check whether it was recorded before \
             retrying, or it may be recorded twice",
            self.event_id
        )
    }
}

impl std::error::Error for CommitOutcomeUnknown {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(self.cause.as_ref())
    }
}

/// Wrap the failure of a COMMIT that may have landed, naming the event (#713). `cause` is the
/// commit's own error, kept BELOW the marker so `operator_chain` still renders what went wrong.
///
/// `pub`, unlike the refusal mints: this marker claims LESS than an ordinary error, not more —
/// "nobody knows yet" — so a caller outside this crate that attaches it can only make a surface
/// more cautious (re-read before acting), never hide a remedy behind a false verdict.
pub fn commit_outcome_unknown(event_id: uuid::Uuid, cause: anyhow::Error) -> anyhow::Error {
    anyhow::Error::new(CommitOutcomeUnknown { event_id, cause })
}

/// The failure of a write's COMMIT (`fault`, which carries the commit's `tokio_postgres::Error`),
/// marked [`commit_outcome_unknown`] unless the server answered it with an ordinary ERROR
/// (#713, #814).
///
/// A server that answered with an ERROR-severity `DbError` (somewhere on the chain, so a
/// SQLSTATE) rolled the transaction back: nothing was written, and `fault` is returned untouched,
/// an ordinary failure. Anything else may have landed, so it is marked, and a surface re-reads
/// before offering the act again instead of calling it "failed": a commit with no answer (the
/// connection lost mid-flight), and a FATAL or PANIC answer, which can follow a commit that
/// landed (a PANIC while flushing the commit's WAL is replayed by crash recovery).
///
/// The rule the clinical and identity writes go through (the medication writes, the judgement,
/// a registration, and since #817 identify, a John-Doe chart, evidence, a proposal's link, an
/// auto-applied link, a sensitivity grade, a shred), so the question is asked the same way
/// everywhere. An autocommit statement counts: it is its own commit.
/// `tests/db_errors_stay_legible.rs` lists every file that commits, or writes an event in
/// autocommit, as marked or, with its reason, not: writes a re-run cannot double (a restore's
/// fenced commit, the node-plane and actor-enrolment events), writes with no event, and the
/// sync harness's load generator.
///
/// Errs towards "unknown": a call that failed before it was ever sent (a dead connection), or a
/// FATAL that arrived before the commit began, is marked too. That claims LESS than the truth,
/// never more.
pub fn commit_failure(event_id: uuid::Uuid, fault: anyhow::Error) -> anyhow::Error {
    use tokio_postgres::error::Severity;
    let answered = fault.chain().any(|cause| {
        cause
            .downcast_ref::<tokio_postgres::Error>()
            .and_then(|pg| pg.as_db_error())
            .is_some_and(|db| db.parsed_severity() == Some(Severity::Error))
    });
    if answered {
        fault
    } else {
        commit_outcome_unknown(event_id, fault)
    }
}

/// The floor's own sentence, if this chain is a floor refusal: a bare `RAISE EXCEPTION`, SQLSTATE
/// `P0001` (db/001's contract, `tests/floor_refusals_carry_no_errcode.rs`). The server sends that
/// only as an ANSWERED error, so `Some` also means nothing was written; an unknown commit outcome
/// (see [`commit_failure`]) is never one. Walks the whole chain, because callers add `.context(…)`
/// layers above the database error. **Pure.**
///
/// The restore's `refusal_is_deliberate` asks the same question of an already-extracted SQLSTATE,
/// as a mirror of `cairn-sync`'s across a crate wall; this one starts from the error itself.
pub(crate) fn floor_refusal_message(e: &anyhow::Error) -> Option<&str> {
    e.chain()
        .filter_map(|cause| cause.downcast_ref::<tokio_postgres::Error>()?.as_db_error())
        .find(|db| db.code() == &tokio_postgres::error::SqlState::RAISE_EXCEPTION)
        .map(|db| db.message())
}

/// What a write returned (`value`), plus the id of the event a lost commit of that same write
/// would have named instead through [`commit_failure`] (#828). A success line that prints
/// `event_id` therefore names the same KIND of id a "commit outcome unknown" names, so the
/// operator meets one vocabulary either way and `cairn-node event <id>` checks both.
#[derive(Debug)]
pub struct Committed<T> {
    pub value: T,
    pub event_id: uuid::Uuid,
}

/// The event whose commit outcome is unknown, if this chain carries the [`CommitOutcomeUnknown`]
/// marker anywhere. Walks the whole chain, like [`refusal_scope`], because callers add
/// `.context(…)` layers above it.
pub fn commit_outcome_unknown_event(e: &anyhow::Error) -> Option<uuid::Uuid> {
    e.chain()
        .find_map(|cause| cause.downcast_ref::<CommitOutcomeUnknown>())
        .map(|marker| marker.event_id)
}

/// The failure behind a [`CommitOutcomeUnknown`] marker, rendered as [`operator_chain`] renders
/// it but WITHOUT the marker's own sentence ("check whether it was recorded … or it may be
/// recorded twice") and without any `.context(…)` layer added ABOVE the marker — only what the
/// marker wraps. For a caller that words the outcome itself: auto-apply knows a re-run cannot
/// double its link (#823 review), so printing the marker's warning beside its "running it again
/// is safe" would contradict it. `None` when the chain carries no marker.
pub fn commit_outcome_unknown_cause(e: &anyhow::Error) -> Option<String> {
    e.chain()
        .find_map(|cause| cause.downcast_ref::<CommitOutcomeUnknown>())
        .map(|marker| operator_chain(&marker.cause))
}
