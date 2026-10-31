//! Saying that the WINDOW cut a command off (#893).
//!
//! When a hold runs past [`NodeConnection::ANSWER_WITHIN`](super::NodeConnection::ANSWER_WITHIN),
//! the watchdog aborts the connection task, and tokio-postgres hands the stuck query its bare
//! `Error::closed()` ("connection closed"): the same text as a server restart or a reset socket.
//! The library cannot be told why, so the marker is added afterwards, by the one party that
//! knows: the guard, still held when the error comes back. A call site passes its error through
//! [`ConnectionGuard::explain`] before the guard drops.
//!
//! Only the TEXT changes. The marker is a context layer on top of the chain, and every
//! classifier (`data_error_from`, `commit_outcome_unknown_event`) walks the whole chain, so a
//! cut-off read stays an outage and a cut-off COMMIT stays "not known".
use super::ConnectionGuard;
use std::time::Duration;

/// A hold the window cut off because the node did not answer within `limit`. Its text is a
/// lowercase clause, so it reads at the head of a `{e:#}` chain and as an outcome line's cause.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CutOff {
    limit: Duration,
}

impl std::fmt::Display for CutOff {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "the node did not answer within {} s, so this window cut the connection off",
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

    /// `e`, saying so if this hold was cut off (the module doc). Call it while the guard is
    /// still held: once it drops, nothing can tell.
    pub fn explain(&self, e: anyhow::Error) -> anyhow::Error {
        self.cut_watch().explain(e)
    }

    /// This hold's cut, as a value that does not borrow the guard: for a caller whose open
    /// transaction already borrows it mutably (`record_links`' snapshot read).
    pub fn cut_watch(&self) -> CutWatch {
        CutWatch {
            watched: self.watch.as_ref().map(|w| (w.cutter.clone(), w.limit)),
        }
    }
}

/// Whether one hold was cut off, read when asked. Taken from its guard while the guard is held.
#[derive(Clone)]
pub struct CutWatch {
    /// The hold's client cutter and limit; `None` for a `fixed` connection, never cut.
    watched: Option<(super::Cutter, Duration)>,
}

impl CutWatch {
    /// Was the hold's client cut off by its watchdog? Any error a command gets from a cut client
    /// is the cut's: the client can answer nothing after it.
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

/// The cause an outcome line names for a failure: the cut if the window cut the command off,
/// else the root cause (for a cut, that is only "connection closed"). Pure.
pub fn cause_of(e: &anyhow::Error) -> String {
    match e.downcast_ref::<CutOff>() {
        Some(cut) => cut.to_string(),
        None => e.root_cause().to_string(),
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
            "the node did not answer within 15 s, so this window cut the connection off: \
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

    #[test]
    fn a_limit_below_a_second_is_said_in_fractions() {
        let cut = CutOff {
            limit: Duration::from_millis(300),
        };
        assert!(cut.to_string().contains("within 0.3 s"), "{cut}");
    }
}
