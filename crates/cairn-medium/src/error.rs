//! The one error type for this crate — but NOT one opaque variant for every failure.
//!
//! WHY THE VARIANTS ARE SHAPED THIS WAY (#500 slice 2a review). This type used to carry a
//! single `Decode(String)` that absorbed every read failure. That collapsed situations whose
//! operator remedies are OPPOSITE:
//!
//! | situation                          | remedy                                          |
//! |------------------------------------|-------------------------------------------------|
//! | written by a newer Cairn           | upgrade this node. **The medium is fine.**       |
//! | physically damaged                 | do not append; fetch another copy.               |
//! | not a backup medium at all         | you picked the wrong file.                       |
//!
//! A restore tool that (reasonably) maps one opaque `Decode` to "this medium is damaged,
//! restore from another copy" is WRONG for the first row, and an operator following that
//! advice discards a perfectly good medium mid-disaster. `record::take_record`'s message even
//! said "upgrade this node before reading it" — while handing the caller a variant that could
//! not express it. Naming the three is the difference between an error a program can act on
//! and one it can only print.

#[derive(thiserror::Error, Debug)]
pub enum BackupError {
    /// Never constructed inside this crate — it does no I/O at all (see the crate docs).
    /// It exists so a CALLER that does (reading a medium off disk, writing one) can use this
    /// one error type end to end via `?` rather than wrapping it in a second enum.
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    /// These bytes are not a Cairn backup medium at all — no recognised magic header. The
    /// operator pointed at the wrong file; nothing is damaged.
    #[error("not a backup medium: {0}")]
    NotAMedium(String),

    /// The medium is STRUCTURALLY INTACT but was written by a newer Cairn that uses a
    /// feature this build does not implement (an unknown record flag bit, say).
    ///
    /// **Never treat this as damage.** Do not re-run a backup over this medium and do not
    /// append to it — this build cannot see everything already on it, so an append would
    /// write against an incomplete picture. Upgrade the node and read it again.
    #[error("unsupported by this build: {0}")]
    UnsupportedByThisBuild(String),

    /// The medium is damaged: a length prefix beyond its cap, a truncated frame, a malformed
    /// section body, a non-UTF-8 field. Distinct from a TORN TAIL, which is not an error at
    /// all — an interrupted append surfaces as `Ok(None)` from `segment::take_section` and as
    /// `MediumV3::truncated_tail`, because its remedy ("run the backup again") is the
    /// opposite of this one's ("do not append; fetch another copy").
    #[error("damaged medium: {0}")]
    Damaged(String),

    /// A caller asked this crate to WRITE something that violates a hard format bound — a
    /// chunk over the frame cap, a section over the section cap, an empty segment. Fires
    /// BEFORE any bytes are written, on the writer's own data, so a caller can never produce
    /// a medium that could not be read back. Not a property of any medium on disk.
    #[error("refused to encode: {0}")]
    Encode(String),
}
