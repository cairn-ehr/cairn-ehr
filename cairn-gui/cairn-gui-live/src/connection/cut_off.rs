//! Saying that the WINDOW cut a command off (#893).
//!
//! When a hold runs past [`NodeConnection::ANSWER_WITHIN`](super::NodeConnection::ANSWER_WITHIN),
//! the watchdog aborts the connection task, and tokio-postgres hands the stuck query its bare
//! `Error::closed()` ("connection closed"): the same text as a server restart or a reset socket.
//! The library cannot be told why, so the marker is added afterwards, by the one party that
//! knows: the guard, still held when the error comes back. Every call site whose error reaches
//! the screen passes it through [`ConnectionGuard::explain`] (or a [`CutWatch`] taken from the
//! guard) before the guard drops; a failure the node already worded as text gets the cut named
//! by [`name_cut_in`].
//!
//! Only the TEXT changes. The marker is a context layer on top of the chain, and every
//! classifier (`data_error_from`, `commit_outcome_unknown_event`) walks the whole chain, so a
//! cut-off read stays an outage and a cut-off COMMIT stays "not known".
use super::ConnectionGuard;
use std::time::Duration;

/// A hold the window cut off because the command was not finished within `limit`. The whole
/// hold is timed, not one statement, so the text never says the node did not answer: a command
/// of many answered statements is cut too. It is a lowercase clause, so it reads inside a
/// `{e:#}` chain and as an outcome line's cause.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CutOff {
    limit: Duration,
}

impl std::fmt::Display for CutOff {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "the command was not finished within {} s, so this window cut the connection off",
            self.limit.as_secs_f64()
        )
    }
}

impl std::error::Error for CutOff {}

impl CutOff {
    /// For OTHER crates' tests only (the window words a cut without a database to cut).
    /// Production makes one in `CutWatch::cut_off` alone.
    #[doc(hidden)]
    pub fn for_tests(limit: Duration) -> Self {
        Self { limit }
    }
}

impl ConnectionGuard<'_> {
    /// Was this hold's client cut off by its watchdog? See [`CutWatch::cut_off`].
    pub fn cut_off(&self) -> Option<CutOff> {
        self.cut_watch().cut_off()
    }

    /// `e`, saying so if this hold was cut off (the module doc). The borrow makes this one
    /// safe: it can only be called while the guard is held (see [`CutWatch`] for why that
    /// matters).
    pub fn explain(&self, e: anyhow::Error) -> anyhow::Error {
        self.cut_watch().explain(e)
    }

    /// This hold's cut, as a value that does not borrow the guard: for a caller whose open
    /// transaction already borrows it mutably (`record_links`' snapshot read), or who hands it to
    /// a helper that takes a generic client (the worklist's doubt reads).
    pub fn cut_watch(&self) -> CutWatch {
        CutWatch {
            watched: self.watch.as_ref().map(|w| (w.cutter.clone(), w.limit)),
        }
    }
}

/// Whether one hold's client was cut off, read when asked. Read it before the guard drops: the
/// flag belongs to the CLIENT, not the hold, so once the guard is gone a LATER hold on the same
/// client could be cut, and this would then name a cut the error never had.
#[derive(Clone)]
pub struct CutWatch {
    /// The hold's client cutter and limit; `None` for a `fixed` connection, never cut.
    watched: Option<(super::Cutter, Duration)>,
}

impl CutWatch {
    /// Was the hold's client cut off by its watchdog? Any error a command gets from a cut client
    /// after the cut is the cut's: the client can answer nothing after it. (An error that came
    /// back an instant BEFORE the cut and is explained after it is said to be cut; only the text
    /// is wrong then, never the class.)
    pub fn cut_off(&self) -> Option<CutOff> {
        let (cutter, limit) = self.watched.as_ref()?;
        cutter.was_cut().then_some(CutOff { limit: *limit })
    }

    /// `e`, saying so if the hold was cut off.
    pub fn explain(&self, e: anyhow::Error) -> anyhow::Error {
        explained(e, self.cut_off())
    }
}

/// `e` with the cut on top when there was one. Pure.
fn explained(e: anyhow::Error, cut: Option<CutOff>) -> anyhow::Error {
    match cut {
        Some(cut) => e.context(cut),
        None => e,
    }
}

/// The cause an outcome line names for a failure: the cut if the window cut the command off
/// (its root cause is only "connection closed", which a node restart gives too), else the root
/// cause. Pure.
pub fn cause_of(e: &anyhow::Error) -> String {
    match e.downcast_ref::<CutOff>() {
        Some(cut) => cut.to_string(),
        None => e.root_cause().to_string(),
    }
}

/// The text tokio-postgres gives a query on a closed client (`Kind::Closed`, 0.7.18), so the end
/// of any failure worded from one. `answer_limit.rs` pins it against a real cut.
const CLOSED: &str = "connection closed";

/// A failure the node already worded as TEXT (a sign-off's failed line, a worklist side), with
/// the cut named if it is the cut's: the hold was cut off and the text ends in `connection
/// closed`. A bare `connection closed` becomes the cut's sentence, as [`cause_of`] gives it; a
/// longer chain keeps its words behind the cut. A text that ends any other way failed for its
/// own reason and is kept as it was. Pure.
pub fn name_cut_in(text: &str, cut: Option<CutOff>) -> String {
    match cut {
        Some(cut) if text == CLOSED => cut.to_string(),
        Some(cut) if text.ends_with(CLOSED) => format!("{cut}: {text}"),
        _ => text.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CUT: CutOff = CutOff {
        limit: Duration::from_secs(15),
    };

    #[test]
    fn a_cut_heads_the_chain_and_names_the_limit() {
        let e = explained(
            anyhow::anyhow!("connection closed").context("reading"),
            Some(CUT),
        );
        assert_eq!(
            format!("{e:#}"),
            "the command was not finished within 15 s, so this window cut the connection off: \
             reading: connection closed"
        );
    }

    #[test]
    fn no_cut_leaves_the_error_as_it_was() {
        let e = explained(anyhow::anyhow!("connection closed"), None);
        assert_eq!(format!("{e:#}"), "connection closed");
    }

    #[test]
    fn the_cause_is_the_cut_when_there_was_one_else_the_root() {
        let root = || anyhow::anyhow!("connection closed").context("saving");
        assert_eq!(cause_of(&explained(root(), Some(CUT))), CUT.to_string());
        assert_eq!(cause_of(&root()), "connection closed");
    }

    /// The hold is timed, not one statement: a command whose every statement was answered is
    /// cut too, so the sentence must not say the node never answered (#895 review).
    #[test]
    fn the_cut_says_the_command_ran_out_of_time_not_that_the_node_was_silent() {
        let text = CUT.to_string();
        assert_eq!(
            text,
            "the command was not finished within 15 s, so this window cut the connection off"
        );
        assert!(!text.contains("did not answer"), "{text}");
    }

    /// A failure that reaches the window as TEXT (a sign-off's failed line, a worklist side)
    /// names the cut only when it ends the way a cut client's does; its own words are kept.
    #[test]
    fn a_worded_failure_names_the_cut_only_if_it_ends_in_connection_closed() {
        let cut = Some(CUT);
        assert_eq!(name_cut_in("connection closed", cut), CUT.to_string());
        assert_eq!(
            name_cut_in("reading the record of chart X: connection closed", cut),
            format!("{CUT}: reading the record of chart X: connection closed")
        );
        let own = "signing: db error: ERROR: permission denied for table event_log";
        assert_eq!(name_cut_in(own, cut), own);
        assert_eq!(name_cut_in("connection closed", None), "connection closed");
    }

    #[test]
    fn a_limit_below_a_second_is_said_in_fractions() {
        let cut = CutOff {
            limit: Duration::from_millis(300),
        };
        assert!(cut.to_string().contains("within 0.3 s"), "{cut}");
    }
}
