//! The pure half of `cairn-sync requeue` — the release rule, outcome accounting, and the operator's
//! words.
//!
//! **Why this module exists at all.** `do_requeue` used to make every decision inline, and one of
//! them was wrong in a way no test could see: it deleted a pen row whenever the apply door returned
//! `Ok`, without ever asking whether the custody it was carrying had actually landed. On a restored
//! solo node that pen row is the last copy of the event's DEK, so the command the pen's own remedy
//! text points an operator at destroyed the key it was holding, printed nothing a monitor could
//! read, and exited 0 (issue #578).
//!
//! Every decision that can be made from values alone lives here, as a pure function with a unit
//! test that needs no database — which matters, because the behaviour these serve is otherwise
//! reachable only through a DB-gated suite that drives the shipped binary. `do_requeue` in
//! `main.rs` asks the database the questions ([`CustodyState`] after the door, whether an unwrap
//! key is registered) and hands the answers to [`open_pen_key`], [`keyed_row_verdict`] and
//! [`door_withheld_cause`].
//!
//! **THE ONE RULE, stated once, here.**
//!
//! > A pen row that carries a wrapped DEK is released only when custody for its event is SETTLED.
//!
//! Settled means held, shredded, or plaintext — see [`CustodyState::settled_as`]. The rule is
//! deliberately uniform across every way custody can fail to land ([`CustodyGap`]), because they
//! are not distinguishable *as recoverability* at the moment of the decision. Most tempting to
//! carve out is [`WrappedDekFault::DidNotOpen`] — "this key is not ours, so its row is worthless".
//! That claim cannot be made from here: *did not open with the key we have right now* is not *not
//! ours*. The operator may be holding the right `<key>.unwrap` on a USB stick they have not plugged
//! in, which is the #495 shape this whole disaster-recovery path exists to survive.
//!
//! The same rule is enforced one layer down, in the database, by `cairn_release_pen_row`
//! (`db/052_restore_doors.sql`): this module decides so it can EXPLAIN every retention; the door
//! is the floor under the decision, and the whole of it for `pull`'s auto-release.
//!
//! **What "released" promises.** That custody for the row's event is SETTLED — which is not the
//! same as "a key landed": a [`Released::Shredded`] row's key is destroyed on purpose and a
//! [`Released::NothingToOpen`] row never had one. When a key does land on an event already in the
//! log without it, the apply door projects that event itself (ADR-0070), so there is no heal step
//! for this command to report. The ONE exception is an event still carrying an `event_deferred`
//! marker: the door lands its key and deliberately leaves its chart to re-adjudication, which
//! projects it through the same dispatch. `cairn-node deferred` is where that state is visible.
//!
//! **How a row that truly is unopenable ever leaves the pen**, since the rule alone would hold it
//! forever: `db/021`'s `acked` flag, which it describes as *"a recorded human decision, never an
//! automatic one"*. `do_requeue` does not put an acked row through the door at all. That is NOT
//! what `do_pull` does with one — pull re-offers acked bytes and releases the row if the floor now
//! admits them; its `skipped_acked` counts acked rows the door REFUSED again. The difference is
//! argued at pull's release site in `main.rs`: nothing forces a requeue, so applying a row a human
//! excluded would be this node's own choice to override that decision.

use cairn_event::keys::Secret32;
use cairn_event::seal::WRAPPED_DEK_LEN;

/// The exit status of a requeue that finished its loop but did not finish the recovery.
///
/// Distinct from `1`, which is a run that FAILED (an interrupted loop, a database fault), and from
/// `2`, which is a bad flag. A run that retained rows or left rows the door still refuses has done
/// everything it safely could — and a script must still be able to see that the pen is not empty.
///
/// **KEPT EQUAL TO `cairn_node::restore::completeness::EXIT_INCOMPLETE` BY A TEST (#594,
/// ADR-0071).** `cairn-node restore` ruled the same way about the same state and, until #594, had
/// only exit 1 to say it with — its message apologising in words for a vocabulary the command did
/// not have. It has the vocabulary now, and the two commands are used together: `restore` fills the
/// pen and `requeue` empties it, so a script driving one recovery reads this number from both.
///
/// It is a second definition rather than an alias of `cairn-node`'s because `cairn-sync` is a
/// **binary-only** crate whose `cairn-node` dependency is a **dev**-dependency: importing it in
/// production code would pull the whole node crate into this binary's build graph for an integer.
/// `exit_incomplete_matches_cairn_nodes_restore` below closes the gap where the dependency already
/// exists — if either number moves, that test fails and names the other.
///
/// ⚠️ That rules out the alias, not a shared home: both crates depend in production on
/// `cairn-event` and `cairn-keystore`, so one definition in either would need no new crate and no
/// new edge. Rejected on **§9 blast-radius** grounds — `cairn-event` is the safety-critical core
/// kept deliberately small, and a CLI exit status is not an event concept. See
/// `cairn_node::restore::completeness::EXIT_INCOMPLETE`'s doc for the full argument.
pub const EXIT_INCOMPLETE: i32 = 3;

/// The custody state of an event, as `cairn_custody_state` (`db/052`) names it.
///
/// A named state rather than the boolean `cairn_custody_landed` returns, because `requeue` does
/// more with the answer than decide: it counts a shredded release apart from a recovered one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CustodyState {
    /// No event with this content address is in this node's log.
    Absent,
    /// Erasure is logged for the event: its key was destroyed on purpose (ADR-0005).
    Shredded,
    /// This node holds a wrapped DEK for the event.
    Held,
    /// The event is not sealed, so there is nothing a DEK could open.
    Plaintext,
    /// A sealed event holding neither a DEK nor a shred — admitted, and unopenable here.
    Withheld,
}

impl CustodyState {
    /// Parse the word `cairn_custody_state` returns. `None` for anything else — a schema newer than
    /// this binary, which the caller must report rather than guess about.
    pub fn parse(word: &str) -> Option<Self> {
        match word {
            "absent" => Some(Self::Absent),
            "shredded" => Some(Self::Shredded),
            "held" => Some(Self::Held),
            "plaintext" => Some(Self::Plaintext),
            "withheld" => Some(Self::Withheld),
            _ => None,
        }
    }

    /// Is custody SETTLED — safe to stop holding a key for this event — and if so, which kind of
    /// release does it license? `None` when it is not settled.
    ///
    /// THE settled list, in Rust: [`keyed_row_verdict`] decides with nothing else. It must agree
    /// with `cairn_custody_landed`'s `IN ('held', 'shredded', 'plaintext')` in `db/052`; the unit
    /// test below pins this side and `db/tests/052_restore_doors_test.sql` pins that one.
    pub fn settled_as(self) -> Option<Released> {
        match self {
            Self::Held => Some(Released::WithCustody),
            Self::Shredded => Some(Released::Shredded),
            Self::Plaintext => Some(Released::NothingToOpen),
            Self::Withheld | Self::Absent => None,
        }
    }
}

/// Why a wrapped DEK sitting in the pen would not open.
///
/// The two arms call for **different operator actions**, which is the whole reason the distinction
/// is kept (issue #581): one says the stored bytes are the wrong size, the other that they did not
/// open. Before this existed, both were reported as the second — a precise untruth where an
/// imprecise near-truth was available for free (principle 4 inverted).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WrappedDekFault {
    /// The stored blob is not the right **size**, so it never reached decryption at all.
    ///
    /// `cairn_event::seal::unwrap_dek` length-checks before it does anything else. A
    /// `sync_quarantine.dek_wrapped` of any other length was written truncated or over-long — a pen
    /// write defect, not a foreign medium. Telling an operator to go and find the right key would
    /// send them after a key that would not have helped. (Damage that KEEPS the length — flipped
    /// bits — cannot be told apart from a wrong key here, and lands in [`Self::DidNotOpen`].)
    Damaged,
    /// Correctly sized, and it did not open with the key this node is holding.
    ///
    /// The medium came from another node, this node has not been given its own custody key yet, or
    /// the blob is corrupt without having changed length. **The code does not claim to know which**,
    /// and the message says so.
    DidNotOpen,
}

/// Classify an unwrap failure **structurally**, never by matching the error's text.
///
/// `WRAPPED_DEK_LEN` is public, and the length check inside `unwrap_dek` is the first thing it
/// does, so the caller can ask the same question the same way. That keeps this a pure function of
/// the bytes and leaves no string to drift when `cairn-event`'s wording changes.
///
/// Called only when an unwrap has already failed ([`open_pen_key`] is the one caller): a blob of the
/// right length that *did* open is never classified.
pub fn classify_wrapped_dek(wrapped: &[u8]) -> WrappedDekFault {
    if wrapped.len() == WRAPPED_DEK_LEN {
        WrappedDekFault::DidNotOpen
    } else {
        WrappedDekFault::Damaged
    }
}

/// Why the key a pen row carries could not be opened BEFORE the door was reached.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PenKeyFault {
    /// This node could not resolve its own custody key at all, so nothing was even attempted.
    /// `cmd_requeue` has already printed the reason; this is the per-row consequence.
    KeyUnresolved,
    /// A key was available and the row's wrapped DEK still would not open.
    Dek(WrappedDekFault),
}

/// Open the wrapped DEK a pen row carries, with the custody key this run resolved (if any).
///
/// The PLAINTEXT DEK is what the apply door wants — it feeds `p_dek` into `cairn_wrap_dek`, so
/// passing the wrapped copy through would double-wrap it into a key that unwraps to noise. Pure
/// (crypto, no IO), so every way it can fail is unit-tested here.
pub fn open_pen_key(wrapped: &[u8], secret: Option<&Secret32>) -> Result<Secret32, PenKeyFault> {
    let secret = secret.ok_or(PenKeyFault::KeyUnresolved)?;
    // The error itself is not carried: `classify_wrapped_dek` re-asks its one decidable question
    // structurally, and the rest of a decryption failure's text says nothing an operator can act on.
    cairn_event::seal::unwrap_dek(wrapped, secret)
        .map_err(|_| PenKeyFault::Dek(classify_wrapped_dek(wrapped)))
}

/// Why the custody a keyed pen row was carrying did not reach `event_dek`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CustodyGap {
    /// The key never opened, so the door was handed no DEK and admitted the event without one.
    PenKey(PenKeyFault),
    /// The DEK opened, but this node has NO unwrap key registered (`node_unwrap_key` is empty), so
    /// the door could not store custody for it. This is the #578 chain proper.
    ///
    /// Not a certainty about the body: `db/020` checks that the DEK opens the body (step 7) BEFORE
    /// it looks for a registered key (step 9), and both withhold custody the same way. So a DEK that
    /// ALSO does not open the body lands here until a key is registered, and shows up as
    /// [`CustodyGap::DekDidNotOpenBody`] on the next run. The row is kept either way.
    NoKeyRegistered,
    /// The DEK opened with this node's key and an unwrap key IS registered, but the DEK did not
    /// open this event's sealed body (`db/020` step 7).
    DekDidNotOpenBody,
}

/// What became of a keyed pen row's own DEK before the door. Keyless rows never get here — they have
/// no custody to lose and release as they always did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PenKey {
    /// The DEK opened and its plaintext went to the door.
    Opened,
    /// It did not, and the door was handed no DEK.
    Unopened(PenKeyFault),
}

/// How a released keyed row's custody stood when it was let go — counted apart, because each says
/// something different about whether a record came back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Released {
    /// This node holds the key. The record's body opens.
    WithCustody,
    /// The event is shredded: the key was destroyed on purpose, and the pen's copy goes with the row.
    Shredded,
    /// The event is plaintext: the DEK riding beside it could open nothing.
    NothingToOpen,
}

/// The verdict on a keyed pen row whose event the door has just admitted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Custody is settled: the row may go.
    Release(Released),
    /// Custody is not settled, for a cause already known from the pen stage.
    Retain(CustodyGap),
    /// Custody is not settled although the DEK opened: the door withheld it, on one of TWO arms
    /// whose cause turns on whether `node_unwrap_key` holds a row. Resolve with
    /// [`door_withheld_cause`] — the one case that needs one more question of the database, asked
    /// only when it is needed.
    RetainDoorWithheld,
}

/// Decide a keyed pen row from its pen-stage outcome and its custody state AFTER the door.
///
/// THE RULE AT THE TOP OF THIS MODULE, as a table:
///
/// | after the door | pen key | verdict |
/// |---|---|---|
/// | held | any | release, with custody |
/// | shredded | any | release, shredded |
/// | plaintext | any | release, nothing to open |
/// | withheld / absent | unopened (fault) | retain, that fault |
/// | withheld / absent | opened | retain — the door withheld it |
///
/// A key that did not open is still released when custody is settled by other means — a peer's
/// pull landed it first, say. Nothing is lost then, and holding the row would be a stale copy.
pub fn keyed_row_verdict(pen: PenKey, after: CustodyState) -> Verdict {
    match (after.settled_as(), pen) {
        (Some(how), _) => Verdict::Release(how),
        (None, PenKey::Unopened(fault)) => Verdict::Retain(CustodyGap::PenKey(fault)),
        (None, PenKey::Opened) => Verdict::RetainDoorWithheld,
    }
}

/// Which of the door's two withholding arms took a DEK that opened.
///
/// `db/020` withholds a DEK that opened on step 7 (it does not open this event's body) or step 9
/// (no unwrap key registered to re-wrap it to). A registered key rules step 9 out, so the cause is
/// step 7. No registration cannot rule step 7 out — it runs first — so `NoKeyRegistered` can be
/// followed, once a key is registered, by `DekDidNotOpenBody`; see that variant. The two remedies
/// are opposite enough that the line must not guess past what registration tells it: one is a key
/// ceremony that, run carelessly on a restored node, forecloses the real key forever.
pub fn door_withheld_cause(unwrap_key_registered: bool) -> CustodyGap {
    if unwrap_key_registered {
        CustodyGap::DekDidNotOpenBody
    } else {
        CustodyGap::NoKeyRegistered
    }
}

/// Where the custody key this run holds came from, as `unwrap_key::resolve` chose it.
///
/// `resolve` uses the `<key>.unwrap` file when one is there and falls back to deriving a key from the
/// signing seed when none is. The message for [`CustodyGap::NoKeyRegistered`] tells an operator to
/// register EXACTLY the key that opened the DEK, and HOW differs between the two: a file can be put
/// where `cairn-node` looks for it, a derived key cannot — `establish-unwrap-key` adopts it only
/// when run with the same signing key and no file in place. A single sentence for both was a remedy
/// that could not be followed in the derived case (PR #582 review).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeySource {
    /// The unwrap-key file at this path (`--unwrap-key`, or the `<key>.unwrap` sibling).
    File { unwrap_key_path: String },
    /// No file at `unwrap_key_path`; derived from the signing key at `key_path`.
    Derived {
        key_path: String,
        unwrap_key_path: String,
    },
}

impl KeySource {
    /// Which source `resolve` used: the file when it is present, the derivation otherwise.
    pub fn of(unwrap_key_path: &str, file_present: bool, key_path: &str) -> Self {
        if file_present {
            Self::File {
                unwrap_key_path: unwrap_key_path.to_string(),
            }
        } else {
            Self::Derived {
                key_path: key_path.to_string(),
                unwrap_key_path: unwrap_key_path.to_string(),
            }
        }
    }

    /// The key, in words an operator can check.
    pub fn describe(&self) -> String {
        match self {
            Self::File { unwrap_key_path } => format!("the unwrap-key file {unwrap_key_path}"),
            Self::Derived {
                key_path,
                unwrap_key_path,
            } => format!(
                "a key derived from the signing key {key_path} (there is no unwrap-key file at \
                 {unwrap_key_path})"
            ),
        }
    }

    /// How to register exactly this key with `cairn-node establish-unwrap-key`.
    pub fn registration_remedy(&self) -> &'static str {
        match self {
            Self::File { .. } => {
                "put that file at `<key>.unwrap` beside cairn-node's own --key (if it is not already \
                 there) and run `cairn-node establish-unwrap-key`, which loads and registers an \
                 existing key file and never replaces it"
            }
            Self::Derived { .. } => {
                "run `cairn-node establish-unwrap-key` with cairn-node's --key set to that same \
                 signing key and no unwrap-key file beside it: with nothing registered it adopts \
                 exactly the key derived from that signing key. cairn-node's key file is not the \
                 same format as cairn-sync's hex seed (#515), so check it is the same KEY, not the \
                 same filename"
            }
        }
    }
}

/// The custody key a requeue run holds, and where it came from.
pub struct HeldKey<'a> {
    pub secret: &'a Secret32,
    pub source: KeySource,
}

/// The first sixteen hex characters of a digest — enough to name a row in a log line.
fn prefix(digest_hex: &str) -> &str {
    &digest_hex[..digest_hex.len().min(16)]
}

/// The operator's `UPDATE … WHERE content_digest = …` clause, typed out in full.
///
/// A prefix will not do here: `content_digest` is `bytea`, so an operator who pasted the sixteen
/// characters from the log line would match no row and get `UPDATE 0` — an ack that silently did
/// not happen.
fn digest_literal(digest_hex: &str) -> String {
    format!("'\\x{digest_hex}'")
}

/// One line telling the operator a keyed row was **kept** rather than released, and what to do.
///
/// `key_source` is [`HeldKey::source`], and is read only by [`CustodyGap::NoKeyRegistered`] — the
/// one cause whose remedy is "register the key that opened it", which is only safe to follow if the
/// line says which key that was, and how to register that one.
pub fn custody_retained_message(
    digest: &[u8],
    gap: CustodyGap,
    key_source: Option<&KeySource>,
) -> String {
    let hex = hex::encode(digest);
    let cause = match gap {
        CustodyGap::PenKey(PenKeyFault::KeyUnresolved) => {
            "this node's custody key could not be resolved, so the penned key was never opened"
                .to_string()
        }
        CustodyGap::PenKey(PenKeyFault::Dek(WrappedDekFault::Damaged)) => {
            // Deliberately does NOT mention other nodes, even to rule them out. This arm exists
            // because the old message sent operators hunting for the right key when the bytes
            // themselves were the wrong size; a sentence that says "not another node's key" still
            // puts that idea in front of someone reading at 3am. Say what IS wrong, and stop.
            "the penned wrapped DEK is the WRONG LENGTH — the pen row itself is damaged (a \
             truncated or over-long write), so no key could have opened it"
                .to_string()
        }
        CustodyGap::PenKey(PenKeyFault::Dek(WrappedDekFault::DidNotOpen)) => {
            "the penned wrapped DEK did not open with the custody key this node is holding — the \
             medium came from another node, this node's own key has not been established yet, or \
             the stored blob is corrupt"
                .to_string()
        }
        CustodyGap::NoKeyRegistered => format!(
            "the penned DEK opened with {}, but this node has NO unwrap key registered, so the \
             apply door could not store custody for it. Register exactly that key: {}. Do NOT run \
             `establish-unwrap-key` against any other key — on a restored node, with no key file in \
             place and the NEW signing key, it would derive a new key, and the singleton registrar \
             would then refuse the real key permanently",
            key_source.map_or_else(
                || "the custody key this run resolved".to_string(),
                KeySource::describe
            ),
            key_source.map_or(
                "`cairn-node establish-unwrap-key` loads and registers an existing `<key>.unwrap` \
                 file beside cairn-node's --key and never replaces it",
                KeySource::registration_remedy
            )
        ),
        CustodyGap::DekDidNotOpenBody => {
            "the penned DEK opened with this node's registered custody key but did not open this \
             event's sealed body — either the pen row pairs this event with another event's key, \
             or the body is in a sealed format this node's cairn_pgx cannot read (upgrade it \
             first)"
                .to_string()
        }
    };
    // The remedy differs for ONE cause. A wrong-length blob is not a key problem, so "run requeue
    // again" would be a false promise there: nothing this node can do to that row will open it.
    let remedy = match gap {
        CustodyGap::PenKey(PenKeyFault::Dek(WrappedDekFault::Damaged)) => {
            "Running requeue again will not help: this record's key has to come from another copy \
             — a peer that holds its custody (once a pull has landed it, the next requeue releases \
             this row)."
        }
        _ => "Fix the cause and run `cairn-sync requeue` again.",
    };
    format!(
        "requeue: {} KEPT in the pen — {cause}. Its event is in the log, but its body cannot be \
         opened here and this pen row may hold the only copy of its key, so the row is NOT \
         released. {remedy} To stop trying instead — accepting that this record stays unreadable \
         on this node — ack it: UPDATE sync_quarantine SET acked = TRUE WHERE content_digest = {};",
        prefix(&hex),
        digest_literal(&hex)
    )
}

/// One line for a row skipped because a human already licensed its exclusion (issue #581).
///
/// It must not be silent in either direction. An operator who acked rows during a flood, fixed the
/// cause and then ran `requeue` needs to be told why those rows did not come back, and needs the
/// way to put them back in play. It does not claim the event is absent from the log: a row acked
/// after an earlier run retained it for custody has an event that is already there.
pub fn acked_skip_message(digest: &[u8]) -> String {
    let hex = hex::encode(digest);
    format!(
        "requeue: {} SKIPPED — a human acked this row (db/021: a recorded decision to exclude it), \
         so requeue does not put it through the door. To put it back in play, clear the flag and \
         run requeue again: UPDATE sync_quarantine SET acked = FALSE WHERE content_digest = {};",
        prefix(&hex),
        digest_literal(&hex)
    )
}

/// One line for a keyed row released although its OWN penned key would not open.
///
/// Custody was settled by other means — a peer's pull landed it first, or the event was shredded —
/// so nothing is lost. But a pen row whose key was the wrong length is evidence of a pen write
/// defect, and it would otherwise vanish with a plain "released" line.
pub fn released_despite_pen_key_note(digest: &[u8], fault: WrappedDekFault) -> String {
    let what = match fault {
        WrappedDekFault::Damaged => "was the WRONG LENGTH (a damaged pen row)",
        WrappedDekFault::DidNotOpen => "did not open with this node's custody key",
    };
    format!(
        "requeue: {} released — custody for its event was already settled here, although this \
         row's own penned key {what}. Nothing was lost.",
        prefix(&hex::encode(digest))
    )
}

/// One line for a row this run judged releasable that `cairn_release_pen_row` nevertheless KEPT.
///
/// The database floor refuses to delete a pen row carrying a wrapped DEK whose custody has not
/// landed. `do_requeue` asks the same question first, so the two should never disagree — but if
/// custody changed between the two checks, or a future edit makes them disagree, the row is still
/// held with its key, and the operator must be told that, not that it vanished.
pub fn guard_refused_release_message(digest: &[u8]) -> String {
    format!(
        "requeue: {} KEPT in the pen — the database's release guard (cairn_release_pen_row) refused \
         it: the row carries a wrapped DEK whose custody has not landed, although this run's own \
         check had judged it releasable. Custody changed during the run, or the two checks \
         disagree (worth reporting). The row and its key are held; run `cairn-sync requeue` again.",
        prefix(&hex::encode(digest))
    )
}

/// Every outcome one `requeue` run reached, counted.
///
/// **Five fields PARTITION the rows examined** — `released`, `custody_retained`, `skipped_acked`,
/// `still_quarantined`, `vanished`; see [`RequeueCounts::accounted_for`]. **Two are SUBSETS of
/// `released`**, not further outcomes: `released_with_custody` and `released_shredded` say how a
/// released keyed row's custody stood (#579's question: of the rows that left the pen, how many
/// carried a key that landed).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct RequeueCounts {
    /// Rows that left the pen: the event is in the log and the row is gone.
    pub released: usize,
    /// Of those, keyed rows whose custody this node now holds.
    pub released_with_custody: usize,
    /// Of those, keyed rows over a SHREDDED event — no key exists, on purpose.
    pub released_shredded: usize,
    /// Keyed rows kept because custody for their event is not settled (the rule at the top).
    pub custody_retained: usize,
    /// Rows a human had already acked, left untouched.
    pub skipped_acked: usize,
    /// Rows the apply door still refuses: not admitted, annotated, kept.
    pub still_quarantined: usize,
    /// Rows that left the pen between the listing and their turn (a concurrent pull, a second
    /// requeue, an operator DELETE).
    pub vanished: usize,
}

impl RequeueCounts {
    /// The rows this run reached a verdict on. On a COMPLETE run this equals `examined`.
    ///
    /// Destructured with every field NAMED, so a field added later fails to compile here until
    /// someone decides whether it partitions or is a subset — rather than silently falling out of
    /// the sum. The two subsets are named and discarded: adding a subset to its own superset
    /// would double-count, the "second spelling of a count" defect the slice-2d measurement rig
    /// deleted a field to avoid.
    pub fn accounted_for(&self) -> usize {
        let Self {
            released,
            released_with_custody: _,
            released_shredded: _,
            custody_retained,
            skipped_acked,
            still_quarantined,
            vanished,
        } = *self;
        released + custody_retained + skipped_acked + still_quarantined + vanished
    }

    /// Did this run leave work for a human? True when rows are still held (for custody, or refused
    /// by the door) — the states [`EXIT_INCOMPLETE`] reports. Acked and vanished rows are not work
    /// left: one is a human's decision, the other is gone.
    pub fn is_incomplete(&self) -> bool {
        self.custody_retained > 0 || self.still_quarantined > 0
    }

    /// The `--metrics` object, built in one place so a successful run and an interrupted one can
    /// never disagree about what a requeue reports (ADR-0060 decision 2).
    ///
    /// `references_unlearnable` is passed in rather than counted here because it is `null` — never
    /// `0` — when the #465 report did not run, and only the caller knows that. The custody counts
    /// take no such treatment: under this module's rule the code always looks, so a `0` in
    /// `released_with_custody` truthfully means "no released row carried a key this node now holds".
    pub fn metrics(
        &self,
        examined: usize,
        references_unlearnable: serde_json::Value,
    ) -> serde_json::Value {
        let Self {
            released,
            released_with_custody,
            released_shredded,
            custody_retained,
            skipped_acked,
            still_quarantined,
            vanished,
        } = *self;
        serde_json::json!({
            "op": "requeue",
            // examined == accounted_for() on a COMPLETE run. An interrupted one is short by the
            // rows it never reached PLUS ONE — the row it stopped on, which was reached and is
            // deliberately counted nowhere because the failure is what left its outcome
            // undecided. The interruption message states both in words; reconciling the numbers
            // alone would mislead by exactly that one.
            "examined": examined,
            "released": released,
            "released_with_custody": released_with_custody,
            "released_shredded": released_shredded,
            "custody_retained": custody_retained,
            "skipped_acked": skipped_acked,
            "still_quarantined": still_quarantined,
            "vanished": vanished,
            "references_unlearnable": references_unlearnable
        })
    }

    /// The one human line summarising a run — every count the metrics object carries, so the channel
    /// an operator reads and the channel a monitor parses cannot tell two different stories.
    pub fn summary_line(&self, examined: usize) -> String {
        let Self {
            released,
            released_with_custody,
            released_shredded,
            custody_retained,
            skipped_acked,
            still_quarantined,
            vanished,
        } = *self;
        format!(
            "requeue: {examined} examined — {released} released ({released_with_custody} with \
             custody, {released_shredded} shredded), {custody_retained} kept for custody, \
             {skipped_acked} skipped (acked), {still_quarantined} still quarantined, {vanished} \
             vanished"
        )
    }

    /// The stderr notice for an [`is_incomplete`](Self::is_incomplete) run, naming what is left.
    /// `None` for a complete one.
    pub fn incomplete_notice(&self) -> Option<String> {
        if !self.is_incomplete() {
            return None;
        }
        Some(format!(
            "requeue: INCOMPLETE (exit {EXIT_INCOMPLETE}) — {} row(s) still held in the pen ({} kept \
             for custody, {} still refused by the apply door). Each is named on its own line above, \
             with its remedy.",
            self.custody_retained + self.still_quarantined,
            self.custody_retained,
            self.still_quarantined,
        ))
    }
}

/// What `do_requeue` returns on a run that reached the bottom of its loop.
///
/// The counts as a TYPED value, not only as JSON, so `cmd_requeue` decides its exit status and
/// writes its summary from fields the compiler checks rather than from string keys a typo would
/// silently turn into `null`.
#[derive(Debug, Clone, PartialEq)]
pub struct RequeueReport {
    pub examined: usize,
    pub counts: RequeueCounts,
    /// `null` when the #465 report could not run; see [`RequeueCounts::metrics`].
    pub references_unlearnable: serde_json::Value,
}

impl RequeueReport {
    /// The `--metrics` object for this run.
    pub fn metrics(&self) -> serde_json::Value {
        self.counts
            .metrics(self.examined, self.references_unlearnable.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cairn_event::seal::{unwrap_public, wrap_dek_for};

    /// A test custody key, derived at runtime (house rule 6a) from a `lineage` byte — named for
    /// what it does in the fixture, not for a cryptographic role it does not play (rule 6b).
    fn test_key(lineage: u8) -> Secret32 {
        Secret32::from_bytes(std::array::from_fn(|i| {
            (i as u8).wrapping_mul(lineage).wrapping_add(7)
        }))
    }

    const ALL_STATES: [CustodyState; 5] = [
        CustodyState::Absent,
        CustodyState::Shredded,
        CustodyState::Held,
        CustodyState::Plaintext,
        CustodyState::Withheld,
    ];

    /// Every word the database can return parses, and nothing else does.
    #[test]
    fn every_custody_state_word_parses_and_an_unknown_one_does_not() {
        assert_eq!(CustodyState::parse("absent"), Some(CustodyState::Absent));
        assert_eq!(
            CustodyState::parse("shredded"),
            Some(CustodyState::Shredded)
        );
        assert_eq!(CustodyState::parse("held"), Some(CustodyState::Held));
        assert_eq!(
            CustodyState::parse("plaintext"),
            Some(CustodyState::Plaintext)
        );
        assert_eq!(
            CustodyState::parse("withheld"),
            Some(CustodyState::Withheld)
        );
        assert_eq!(
            CustodyState::parse("rotated"),
            None,
            "a state a newer schema added must be reported, never guessed at"
        );
    }

    /// The settled list matches `cairn_custody_landed`'s `IN ('held', 'shredded', 'plaintext')`.
    #[test]
    fn exactly_held_shredded_and_plaintext_are_settled() {
        assert_eq!(CustodyState::Held.settled_as(), Some(Released::WithCustody));
        assert_eq!(
            CustodyState::Shredded.settled_as(),
            Some(Released::Shredded)
        );
        assert_eq!(
            CustodyState::Plaintext.settled_as(),
            Some(Released::NothingToOpen)
        );
        assert_eq!(CustodyState::Withheld.settled_as(), None);
        assert_eq!(CustodyState::Absent.settled_as(), None);
    }

    /// A blob of the exact wrapped-DEK length that will not open is a KEY problem.
    #[test]
    fn a_correctly_sized_blob_that_will_not_open_is_a_key_problem() {
        let wrapped = vec![0u8; WRAPPED_DEK_LEN];
        assert_eq!(classify_wrapped_dek(&wrapped), WrappedDekFault::DidNotOpen);
    }

    /// Anything else is a DAMAGED row, whichever side of the length it falls.
    #[test]
    fn any_other_length_is_a_damaged_row() {
        for len in [0, 1, WRAPPED_DEK_LEN - 1, WRAPPED_DEK_LEN + 1, 4096] {
            assert_eq!(
                classify_wrapped_dek(&vec![0u8; len]),
                WrappedDekFault::Damaged,
                "length {len} must classify as a damaged pen row"
            );
        }
    }

    /// The classifier agrees with the function whose failure it is explaining.
    ///
    /// ANTI-VACUITY: the classifier's whole claim is that `unwrap_dek` rejects a wrong-length blob
    /// *before* it attempts decryption, and that a right-length one gets past that check. Both
    /// halves are asserted against `cairn-event` itself, so removing or reordering that early
    /// return fails this test instead of silently turning "damaged" into "wrong key" in an
    /// operator's log. (A change to `WRAPPED_DEK_LEN` alone would NOT: both blobs are built from it.)
    #[test]
    fn the_classifier_agrees_with_the_unwrap_it_explains() {
        let secret = test_key(3);
        let truncated: Vec<u8> = (0..WRAPPED_DEK_LEN - 8).map(|i| i as u8).collect();
        let right_size: Vec<u8> = (0..WRAPPED_DEK_LEN).map(|i| i as u8).collect();

        let truncated_err = cairn_event::seal::unwrap_dek(&truncated, &secret)
            .expect_err("a short blob cannot open");
        assert!(
            truncated_err.to_string().contains("malformed"),
            "cairn-event no longer length-checks first; this classifier's premise is gone: \
             {truncated_err}"
        );
        assert_eq!(classify_wrapped_dek(&truncated), WrappedDekFault::Damaged);

        let right_size_err = cairn_event::seal::unwrap_dek(&right_size, &secret)
            .expect_err("arbitrary bytes of the right length must not open");
        assert!(
            !right_size_err.to_string().contains("malformed"),
            "a right-length blob must get PAST the length check, or DidNotOpen is unreachable: \
             {right_size_err}"
        );
        assert_eq!(
            classify_wrapped_dek(&right_size),
            WrappedDekFault::DidNotOpen
        );
    }

    /// Every way the pen stage can go, against real crypto.
    #[test]
    fn opening_a_pen_key_names_every_way_it_can_fail() {
        let node = test_key(5);
        let stranger = test_key(11);
        let dek = test_key(13);
        let wrapped = wrap_dek_for(&dek, &unwrap_public(&node)).expect("wrap for this node");

        let opened = open_pen_key(&wrapped, Some(&node)).expect("this node's own wrap opens");
        assert_eq!(
            opened.as_bytes(),
            dek.as_bytes(),
            "and yields the plaintext DEK"
        );

        assert_eq!(
            open_pen_key(&wrapped, None).err(),
            Some(PenKeyFault::KeyUnresolved),
            "no key resolved: nothing was attempted"
        );
        assert_eq!(
            open_pen_key(&wrapped, Some(&stranger)).err(),
            Some(PenKeyFault::Dek(WrappedDekFault::DidNotOpen))
        );
        assert_eq!(
            open_pen_key(&wrapped[..wrapped.len() - 4], Some(&node)).err(),
            Some(PenKeyFault::Dek(WrappedDekFault::Damaged))
        );
    }

    /// THE RULE, exhaustively: every pen outcome against every custody state.
    ///
    /// Written as an independent table rather than by re-deriving the verdict, so a wrong arm in
    /// `keyed_row_verdict` disagrees with a line a reviewer can read.
    #[test]
    fn a_keyed_row_is_released_exactly_when_custody_is_settled() {
        let unopened = [
            PenKey::Unopened(PenKeyFault::KeyUnresolved),
            PenKey::Unopened(PenKeyFault::Dek(WrappedDekFault::Damaged)),
            PenKey::Unopened(PenKeyFault::Dek(WrappedDekFault::DidNotOpen)),
        ];
        for pen in unopened.iter().copied().chain([PenKey::Opened]) {
            for after in ALL_STATES {
                let expected = match (after, pen) {
                    (CustodyState::Held, _) => Verdict::Release(Released::WithCustody),
                    (CustodyState::Shredded, _) => Verdict::Release(Released::Shredded),
                    (CustodyState::Plaintext, _) => Verdict::Release(Released::NothingToOpen),
                    (CustodyState::Withheld | CustodyState::Absent, PenKey::Unopened(f)) => {
                        Verdict::Retain(CustodyGap::PenKey(f))
                    }
                    (CustodyState::Withheld | CustodyState::Absent, PenKey::Opened) => {
                        Verdict::RetainDoorWithheld
                    }
                };
                assert_eq!(
                    keyed_row_verdict(pen, after),
                    expected,
                    "{pen:?} / {after:?}"
                );
            }
        }
    }

    /// The door's two withholding arms are told apart by registration alone.
    #[test]
    fn a_withheld_dek_that_opened_is_blamed_on_the_right_arm() {
        assert_eq!(door_withheld_cause(false), CustodyGap::NoKeyRegistered);
        assert_eq!(door_withheld_cause(true), CustodyGap::DekDidNotOpenBody);
    }

    /// Each retention cause produces its OWN sentence, and every one keeps the row and names it.
    ///
    /// The distinctness assertion is the point: two causes that rendered the same sentence would pass
    /// a "does it mention the digest" test while telling every operator the same wrong thing.
    #[test]
    fn each_retention_cause_names_its_own_remedy() {
        let digest: Vec<u8> = (0..34u8).collect();
        let gaps = [
            CustodyGap::PenKey(PenKeyFault::KeyUnresolved),
            CustodyGap::PenKey(PenKeyFault::Dek(WrappedDekFault::Damaged)),
            CustodyGap::PenKey(PenKeyFault::Dek(WrappedDekFault::DidNotOpen)),
            CustodyGap::NoKeyRegistered,
            CustodyGap::DekDidNotOpenBody,
        ];
        let messages: Vec<String> = gaps
            .iter()
            .map(|g| {
                custody_retained_message(&digest, *g, Some(&KeySource::of("/k.unwrap", true, "/k")))
            })
            .collect();
        let hex = hex::encode(&digest);
        for (gap, m) in gaps.iter().zip(&messages) {
            assert!(m.contains(&hex[..16]), "{gap:?} must name the row");
            assert!(
                m.contains("KEPT in the pen"),
                "{gap:?} must say it kept the row"
            );
            assert!(
                m.contains(&format!("'\\x{hex}'")),
                "{gap:?}: the ack remedy must carry the FULL digest, or it matches no row: {m}"
            );
        }
        for (i, a) in messages.iter().enumerate() {
            for b in messages.iter().skip(i + 1) {
                assert_ne!(a, b, "two retention causes rendered the same sentence");
            }
        }
    }

    /// A damaged row must NOT be reported as somebody else's key, nor promised a rerun that cannot
    /// help — the #581 finding and its review follow-up, pinned.
    #[test]
    fn a_damaged_row_is_not_a_foreign_key_and_is_not_promised_a_rerun() {
        let damaged = custody_retained_message(
            b"d",
            CustodyGap::PenKey(PenKeyFault::Dek(WrappedDekFault::Damaged)),
            None,
        );
        assert!(damaged.contains("WRONG LENGTH"));
        assert!(
            !damaged.contains("another node"),
            "a truncated pen row must not send the operator looking for another node's key"
        );
        assert!(
            !damaged.contains("run `cairn-sync requeue` again"),
            "no rerun opens a wrong-length blob; promising one is a false remedy: {damaged}"
        );
        let other =
            custody_retained_message(b"d", CustodyGap::PenKey(PenKeyFault::KeyUnresolved), None);
        assert!(other.contains("run `cairn-sync requeue` again"));
    }

    /// The no-key-registered line names the key that opened the DEK and warns against the one
    /// careless step that would foreclose the real key forever (#578 review, critical 2).
    #[test]
    fn the_no_key_registered_line_names_the_key_and_the_hazard() {
        let file = KeySource::of("/media/usb/node.key.unwrap", true, "/n/node.key");
        let m = custody_retained_message(b"d", CustodyGap::NoKeyRegistered, Some(&file));
        assert!(m.contains("/media/usb/node.key.unwrap"), "{m}");
        assert!(m.contains("establish-unwrap-key"), "{m}");
        assert!(
            m.contains("Do NOT run"),
            "cairn-node itself warns that this command, run with no key file in place on a restored \
             node, forecloses the real key permanently; a remedy that omits that is a trap: {m}"
        );
        // The two sources get DIFFERENT remedies: a derived key cannot be "put in place" as a file,
        // and telling the operator to do so is advice nobody can follow.
        let derived = KeySource::of("/n/node.key.unwrap", false, "/n/node.key");
        let d = custody_retained_message(b"d", CustodyGap::NoKeyRegistered, Some(&derived));
        assert!(
            d.contains("derived from the signing key /n/node.key"),
            "{d}"
        );
        assert!(
            d.contains("same signing key"),
            "the derived remedy says how: {d}"
        );
        assert!(
            !m.contains("same signing key") && m.contains("put that file"),
            "{m}"
        );
        let body = custody_retained_message(b"d", CustodyGap::DekDidNotOpenBody, None);
        assert!(
            !body.contains("establish-unwrap-key"),
            "a key IS registered on this arm; sending the operator to register one is the \
             single-cause claim the review found: {body}"
        );
    }

    /// The key source is described as what `unwrap_key::resolve` actually used.
    #[test]
    fn the_key_source_says_file_or_derived() {
        let file = KeySource::of("/n/node.key.unwrap", true, "/n/node.key").describe();
        assert!(file.contains("/n/node.key.unwrap") && !file.contains("derived"));
        let derived = KeySource::of("/n/node.key.unwrap", false, "/n/node.key").describe();
        assert!(
            derived.contains("derived") && derived.contains("/n/node.key "),
            "{derived}"
        );
    }

    /// The acked skip tells the operator what happened and the way back in, typed out in full.
    #[test]
    fn the_acked_skip_names_the_way_back_in() {
        let digest: Vec<u8> = (0..34u8).collect();
        let m = acked_skip_message(&digest);
        assert!(m.contains("SKIPPED"));
        assert!(
            m.contains(&format!(
                "acked = FALSE WHERE content_digest = '\\x{}'",
                hex::encode(&digest)
            )),
            "a skipped row is only honest if the operator is told how to unskip it: {m}"
        );
        assert!(
            !m.contains("never enter the record"),
            "an acked row's event may already be in the log (a row retained for custody, then acked)"
        );
    }

    /// When the database's release guard refuses a row this run judged releasable, the line says
    /// the DATABASE kept it — not that it vanished, which is what a bare FALSE first read as.
    #[test]
    fn a_guard_refusal_is_reported_as_kept_by_the_database() {
        let digest: Vec<u8> = (0..34u8).collect();
        let m = guard_refused_release_message(&digest);
        assert!(m.contains(&hex::encode(&digest)[..16]), "{m}");
        assert!(m.contains("KEPT in the pen"), "{m}");
        assert!(
            m.contains("cairn_release_pen_row"),
            "names the floor that refused: {m}"
        );
        assert!(!m.contains("gone"), "the row is NOT gone: {m}");
    }

    fn sample_counts() -> RequeueCounts {
        // Every field DISTINCT and non-zero, so a line that dropped one or rendered two from the
        // same field cannot pass by coincidence.
        RequeueCounts {
            released: 11,
            released_with_custody: 7,
            released_shredded: 3,
            custody_retained: 5,
            skipped_acked: 13,
            still_quarantined: 17,
            vanished: 19,
        }
    }

    /// The five partitioning outcomes add up, and the two subsets do not join them.
    #[test]
    fn the_outcomes_partition_and_the_subsets_stay_out() {
        assert_eq!(sample_counts().accounted_for(), 11 + 5 + 13 + 17 + 19);
    }

    /// Every field reaches the metrics object, because a count a monitor cannot see is a count that
    /// does not exist (#579's whole complaint).
    #[test]
    fn every_count_reaches_the_metrics_object() {
        let m = sample_counts().metrics(65, serde_json::Value::Null);
        assert_eq!(m["op"], "requeue");
        assert_eq!(m["examined"], 65);
        assert_eq!(m["released"], 11);
        assert_eq!(m["released_with_custody"], 7);
        assert_eq!(m["released_shredded"], 3);
        assert!(
            m.get("reproject_owed").is_none(),
            "retired by ADR-0070: the door projects a late key"
        );
        assert_eq!(m["custody_retained"], 5);
        assert_eq!(m["skipped_acked"], 13);
        assert_eq!(m["still_quarantined"], 17);
        assert_eq!(m["vanished"], 19);
        assert!(m["references_unlearnable"].is_null());
    }

    /// The human line carries every count too — including `vanished`, which the first version of
    /// this line dropped while its comment claimed it named them all (#578 review).
    #[test]
    fn the_summary_line_names_every_count() {
        let line = sample_counts().summary_line(65);
        for needle in [
            "65 examined",
            "11 released",
            "7 with custody",
            "3 shredded",
            "5 kept for custody",
            "13 skipped (acked)",
            "17 still quarantined",
            "19 vanished",
        ] {
            assert!(line.contains(needle), "missing {needle:?}: {line}");
        }
        assert!(!line.contains("reproject"), "{line}");
    }

    /// A shredded release and a recovered one do not serialize alike — #579's failure, one notch
    /// finer: three destroyed keys must not read as three recovered charts.
    #[test]
    fn a_shredded_release_does_not_look_like_a_recovery() {
        let recovered = RequeueCounts {
            released: 3,
            released_with_custody: 3,
            ..Default::default()
        };
        let shredded = RequeueCounts {
            released: 3,
            released_shredded: 3,
            ..Default::default()
        };
        let lost = RequeueCounts {
            custody_retained: 3,
            ..Default::default()
        };
        let render = |c: RequeueCounts| c.metrics(3, serde_json::Value::Null).to_string();
        assert_ne!(render(recovered), render(shredded));
        assert_ne!(render(recovered), render(lost));
    }

    /// Work left for a human is INCOMPLETE; a human's decision and a row already gone are not.
    #[test]
    fn a_run_is_incomplete_exactly_when_it_leaves_work() {
        assert!(!RequeueCounts::default().is_incomplete());
        let complete = RequeueCounts {
            released: 4,
            released_with_custody: 2,
            released_shredded: 1,
            skipped_acked: 2,
            vanished: 1,
            ..Default::default()
        };
        assert!(!complete.is_incomplete(), "{complete:?}");
        assert!(complete.incomplete_notice().is_none());

        for left in [
            RequeueCounts {
                custody_retained: 1,
                ..Default::default()
            },
            RequeueCounts {
                still_quarantined: 1,
                ..Default::default()
            },
        ] {
            assert!(left.is_incomplete(), "{left:?}");
            let notice = left.incomplete_notice().expect("an incomplete run says so");
            assert!(notice.contains("INCOMPLETE"), "{notice}");
        }
    }

    /// #594 / ADR-0071: the two commands of one recovery must speak one exit vocabulary.
    ///
    /// `restore` fills the quarantine pen and exits 3; `requeue` empties it and exits 3 for what
    /// it could not release. A cron wrapper driving a disaster drill reads the number from both,
    /// and it has no way to learn that they disagree except by recovering the wrong thing.
    ///
    /// This is a TEST rather than a compile-time alias because `cairn-node` is a dev-dependency
    /// here (see [`EXIT_INCOMPLETE`]'s own doc) — so this is the earliest point in the build where
    /// both numbers are visible at once. It is cheap, it runs in every `cargo test`, and it fails
    /// naming the other constant.
    #[test]
    fn exit_incomplete_matches_cairn_nodes_restore() {
        assert_eq!(
            EXIT_INCOMPLETE,
            cairn_node::restore::completeness::EXIT_INCOMPLETE,
            "requeue and restore must exit with the SAME status for the same state — a recovery \
             is driven by both commands, and a script cannot be asked to learn two vocabularies \
             for it. Change both, or neither."
        );
    }
}
