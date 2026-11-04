//! The note pane's words for what a save, a take and a sign answered (split from `notes.rs`, which
//! runs the commands): the views the webview reads, and every sentence the clinician reads, made
//! by pure functions so their wording is pinned without a database or a webview
//! (`notes_tests.rs`).
use crate::funnel::view::ErrorView;
use crate::link::judgement_view::{refused, stale};
use cairn_node::note::draft::{SaveOutcome, TakeOutcome};
use cairn_node::note::sign::SignRefusal;

/// What a draft save did. `saved: false` always carries the sentence why nothing was written.
#[derive(Debug, PartialEq, Eq, serde::Serialize)]
pub struct SaveView {
    pub saved: bool,
    /// The draft was taken to another terminal: the webview keeps what it had not saved as a NEW
    /// draft of its own, so nothing typed is lost.
    pub moved: bool,
    /// The draft was signed. Unless this window's own Sign may have signed it, the webview keeps
    /// what it had not saved as a new draft, as for `moved` (final review I1).
    pub spent: bool,
    /// The quarantine timer put the draft into the record unsigned (slice 3b-2a). Never this
    /// window's own Sign, so the webview always keeps what it had not saved as a new draft.
    pub committed: bool,
    /// The draft was discarded (slice 3b-2b). Forks with its own words (`DISCARDED_ELSEWHERE`):
    /// no note exists, so they never say "signed". The webview keeps what it had not saved as a
    /// new draft.
    pub discarded: bool,
    /// Why nothing was written. For `moved` and `committed`, the same words as `fork.kept`.
    pub sentence: Option<String>,
    /// `moved`, `spent`, `committed` or `discarded`: the words for keeping what the draft lacks as a new draft.
    pub fork: Option<ForkWords>,
    /// `spent` or `discarded` only: the words for keeping what was typed AFTER this window's own
    /// press (Sign note or Discard draft) whose outcome was not known. The press took only the
    /// text it carried; the webview keeps the rest as a new draft (PR #984 review).
    pub pressed_fork: Option<ForkWords>,
}

/// What the webview says when it keeps the text a moved, signed-elsewhere, timer-committed or
/// discarded draft lacks. Only the webview knows which of the three happened to the text on
/// screen, so all three travel.
#[derive(Debug, PartialEq, Eq, serde::Serialize)]
pub struct ForkWords {
    /// The text typed after the last answered save is kept: the usual branch.
    pub kept: String,
    /// Nothing on screen was unsaved: nothing is kept, and the editor closes.
    pub nothing_new: String,
    /// The text above the last save was edited, so all of it is kept (and the saved part is in
    /// the other note too).
    pub everything: String,
}

/// What a "Continue here" did. `taken: false` always carries the sentence why nothing moved.
#[derive(Debug, PartialEq, Eq, serde::Serialize)]
pub struct TakeView {
    pub taken: bool,
    pub sentence: Option<String>,
}

/// The sentence a save, take or sign answers when the draft was discarded (slice 3b-2b).
pub(crate) const DISCARDED: &str = "This draft was discarded; it is no longer open.";
/// … when a save finds its draft discarded in another window: the text typed since the last save
/// is in no draft, so it is kept. Never "signed": a discard leaves no note.
pub(crate) const DISCARDED_ELSEWHERE: &str =
    "This draft was discarded in another window. What you typed \
     since the last save is kept here as a new draft.";
/// … the same, when nothing on screen was unsaved: nothing is kept here.
pub(crate) const DISCARDED_ELSEWHERE_NOTHING_NEW: &str =
    "This draft was discarded in another window. \
     Everything typed here was saved into it first, and the record keeps it only as a discarded \
     draft, never as a note.";
/// … the same, when the text that was saved has since been edited: everything is kept.
pub(crate) const DISCARDED_ELSEWHERE_EVERYTHING: &str =
    "This draft was discarded in another window. \
     Everything on this screen is kept here as a new draft.";
/// … when a save read after this window's own Sign note, whose outcome was not known, finds the
/// draft signed: what was typed after the press is in no note, so it is kept. True whichever
/// window signed it, so it never says which.
const SIGNED_AFTER_PRESS: &str =
    "This note was signed. What you typed after pressing Sign note is kept here as a new draft.";
/// … the same, when nothing but blank space was typed after the press: nothing is kept.
const SIGNED_AFTER_PRESS_NOTHING_NEW: &str = "This note was signed; it is no longer open.";
/// … the same, when the text the press carried was edited after it: all of it is kept.
const SIGNED_AFTER_PRESS_EVERYTHING: &str = "This note was signed. The text here was changed \
     after you pressed Sign note, so all of it is kept here as a new draft.";
/// … the same three, for a Discard draft whose outcome was not known.
const DISCARDED_AFTER_PRESS: &str = "This draft was discarded. What you typed after pressing \
     Discard draft is kept here as a new draft.";
/// … nothing but blank space typed after the press.
const DISCARDED_AFTER_PRESS_NOTHING_NEW: &str = DISCARDED;
/// … the text the press carried edited after it.
const DISCARDED_AFTER_PRESS_EVERYTHING: &str = "This draft was discarded. The text here was \
     changed after you pressed Discard draft, so all of it is kept here as a new draft.";
/// The sentence a save or sign answers when the draft was already signed.
pub(crate) const SPENT: &str = "this note has already been signed";
/// … when a SAVE's draft is another chart's (another terminal's is `MOVED`, #958).
const ELSEWHERE: &str = "this draft belongs to another chart — nothing was saved";
/// … the same, for a SIGN: nothing was signed, and the editor still holds the text.
pub(crate) const SIGN_ELSEWHERE: &str =
    "this draft belongs to another chart — nothing was signed; your text is still here";
/// … when a SIGN finds its draft taken to another terminal (Task 5 review M1): not "another
/// chart" — the clinician is told where the note went.
pub(crate) const SIGN_MOVED: &str =
    "This note is now being continued at another terminal — nothing was \
     signed; your text is still here.";
/// … when the writer a save names is not an enrolled member (any more).
const NO_SUCH_WRITER: &str =
    "the name at this terminal is not an enrolled member — say who's here again";
/// … when a save finds its draft taken to another terminal.
const MOVED: &str = "This note is now being continued at another terminal. What you typed since \
     the last save is kept here as a new draft.";
/// … the same, when nothing on screen was unsaved: nothing is kept here.
const MOVED_NOTHING_NEW: &str = "This note is now being continued at another terminal. \
     Everything typed here was saved into it before it moved.";
/// … the same, when the text that was saved has since been edited: everything is kept.
const MOVED_EVERYTHING: &str = "This note is now being continued at another terminal. \
     Everything on this screen is kept here as a new draft; what was saved before the move is \
     also in that note.";
/// … when a save finds its draft signed in another window (final review I1): the text typed since
/// the last save is in no note, so it is kept.
const SIGNED_ELSEWHERE: &str = "This note was signed in another window. What you typed since the \
     last save is kept here as a new draft.";
/// … the same, when nothing on screen was unsaved: nothing is kept here.
const SIGNED_ELSEWHERE_NOTHING_NEW: &str = "This note was signed in another window. Everything \
     typed here was saved into it first.";
/// … the same, when the text that was saved has since been edited: everything is kept.
const SIGNED_ELSEWHERE_EVERYTHING: &str = "This note was signed in another window. Everything on \
     this screen is kept here as a new draft; what was saved before is also in that note.";
/// … when a save finds its draft put into the record unsigned by the quarantine timer (slice
/// 3b-2a): the text typed since the last save is in no note, so it is kept.
const COMMITTED: &str = "This note went into the record unsigned after it was left untouched. \
     What you typed since the last save is kept here as a new draft.";
/// … the same, when nothing on screen was unsaved: nothing is kept here.
const COMMITTED_NOTHING_NEW: &str = "This note went into the record unsigned after it was left \
     untouched. Everything typed here was saved into it first.";
/// … the same, when the text that was saved has since been edited: everything is kept.
const COMMITTED_EVERYTHING: &str = "This note went into the record unsigned after it was left \
     untouched. Everything on this screen is kept here as a new draft; what was saved before is \
     also in that note.";
/// … when a TAKE finds the draft already put in by the timer.
const TAKE_COMMITTED: &str = "this note has already gone into the record unsigned";
/// … when a SIGN finds it: nothing was signed. Not "your text is still here", as for `SPENT`: the
/// save that follows always forks, and closes the editor when nothing was unsaved (review I2).
pub(crate) const SIGN_COMMITTED: &str =
    "this note had already gone into the record unsigned — nothing was signed";
/// … when a TAKE's draft is another chart's (final review item 6): a take saves nothing, and a
/// draft at another terminal is what a take moves, so ELSEWHERE's words are not true of it.
const TAKE_ELSEWHERE: &str = "this draft belongs to another chart — nothing was moved";
/// … when a take finds no such draft (never saved, or discarded).
pub(crate) const GONE: &str = "this draft is no longer open";
/// … the same, for a SIGN: the on-screen writer, or the writer of a part above, is no longer
/// enrolled. Nothing was signed, and the editor still holds the text.
pub(crate) const SIGN_NO_SUCH_WRITER: &str =
    "a writer of this note is not enrolled on this node — nothing \
     was signed; your text is still here";
/// … when no event names the patient on this node.
pub(crate) const NO_CHART: &str = "this patient has no chart on this node";
/// … when nothing but whitespace was typed.
const EMPTY: &str = "there is nothing to sign — the note is empty";

/// A save's answer as the webview reads it. Pure.
pub(crate) fn save_view(outcome: SaveOutcome) -> SaveView {
    let sentence = match outcome {
        SaveOutcome::Saved => None,
        SaveOutcome::Spent => Some(SPENT),
        SaveOutcome::Moved => Some(MOVED),
        SaveOutcome::Elsewhere => Some(ELSEWHERE),
        SaveOutcome::NoChart => Some(NO_CHART),
        SaveOutcome::NoSuchWriter => Some(NO_SUCH_WRITER),
        SaveOutcome::Committed => Some(COMMITTED),
        SaveOutcome::Discarded => Some(DISCARDED),
    };
    let fork = match outcome {
        SaveOutcome::Moved => Some([MOVED, MOVED_NOTHING_NEW, MOVED_EVERYTHING]),
        SaveOutcome::Spent => Some([
            SIGNED_ELSEWHERE,
            SIGNED_ELSEWHERE_NOTHING_NEW,
            SIGNED_ELSEWHERE_EVERYTHING,
        ]),
        SaveOutcome::Discarded => Some([
            DISCARDED_ELSEWHERE,
            DISCARDED_ELSEWHERE_NOTHING_NEW,
            DISCARDED_ELSEWHERE_EVERYTHING,
        ]),
        SaveOutcome::Committed => Some([COMMITTED, COMMITTED_NOTHING_NEW, COMMITTED_EVERYTHING]),
        // Spelled out, no `_` (#412): a new outcome must decide whether its text forks.
        SaveOutcome::Saved
        | SaveOutcome::Elsewhere
        | SaveOutcome::NoChart
        | SaveOutcome::NoSuchWriter => None,
    };
    // Only a spend or a discard can be this window's own press (Sign note, Discard draft).
    let pressed_fork = match outcome {
        SaveOutcome::Spent => Some([
            SIGNED_AFTER_PRESS,
            SIGNED_AFTER_PRESS_NOTHING_NEW,
            SIGNED_AFTER_PRESS_EVERYTHING,
        ]),
        SaveOutcome::Discarded => Some([
            DISCARDED_AFTER_PRESS,
            DISCARDED_AFTER_PRESS_NOTHING_NEW,
            DISCARDED_AFTER_PRESS_EVERYTHING,
        ]),
        SaveOutcome::Saved
        | SaveOutcome::Moved
        | SaveOutcome::Elsewhere
        | SaveOutcome::NoChart
        | SaveOutcome::NoSuchWriter
        | SaveOutcome::Committed => None,
    };
    let words = |[kept, nothing_new, everything]: [&str; 3]| ForkWords {
        kept: kept.into(),
        nothing_new: nothing_new.into(),
        everything: everything.into(),
    };
    SaveView {
        saved: sentence.is_none(),
        moved: outcome == SaveOutcome::Moved,
        spent: outcome == SaveOutcome::Spent,
        committed: outcome == SaveOutcome::Committed,
        discarded: outcome == SaveOutcome::Discarded,
        sentence: sentence.map(str::to_string),
        fork: fork.map(words),
        pressed_fork: pressed_fork.map(words),
    }
}

/// A take's answer as the webview reads it. Pure.
pub(crate) fn take_view(outcome: TakeOutcome) -> TakeView {
    let sentence = match outcome {
        TakeOutcome::Taken => None,
        TakeOutcome::Spent => Some(SPENT),
        TakeOutcome::Elsewhere => Some(TAKE_ELSEWHERE),
        TakeOutcome::Gone => Some(GONE),
        TakeOutcome::Committed => Some(TAKE_COMMITTED),
        TakeOutcome::Discarded => Some(DISCARDED),
    };
    TakeView {
        taken: sentence.is_none(),
        sentence: sentence.map(str::to_string),
    }
}

/// A sign's refusal as the webview reads it. Pure. Only "already signed" and "already gone in
/// unsigned" ask for a re-read: the pane on screen is behind the record, and the re-read shows
/// the note. The others are verdicts a re-read cannot mend.
pub(crate) fn sign_refusal_view(refusal: SignRefusal) -> ErrorView {
    match refusal {
        SignRefusal::Empty => refused(EMPTY),
        SignRefusal::Spent => stale(SPENT),
        SignRefusal::Elsewhere => refused(SIGN_ELSEWHERE),
        SignRefusal::Moved => refused(SIGN_MOVED),
        SignRefusal::NoChart => refused(NO_CHART),
        SignRefusal::NoSuchWriter => refused(SIGN_NO_SUCH_WRITER),
        SignRefusal::Committed => stale(SIGN_COMMITTED),
        SignRefusal::Discarded => stale(DISCARDED),
    }
}
