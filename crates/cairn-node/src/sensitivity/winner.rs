//! §5.9 — which assertion produced a displayed grade, and how to withdraw it.
//!
//! PURE. No database, no I/O. Split out of `report.rs` because it is a value type with its
//! own invariant to hold, and holding that invariant is the whole reason it exists — see
//! [`WinningSubject`]. Keeping it beside the DB reads that happen to build it would bury
//! the one function (`from_row`) that a future reader must not "simplify".
use super::subject_kind_phrase;

/// Which assertion produced a displayed grade, and how to withdraw it (#387).
///
/// # Why a sum type, and not the two correlated fields it replaces
///
/// `ChartReport` used to carry `chart_source: String` beside `chart_content_address:
/// Option<String>`, with a doc comment stating that the second is `None` exactly when the
/// first reads `"none"`. The invariant was real and provable from the SQL — and enforced by
/// nothing. Principle 12 does not shelter these types: they are read-model DTOs built
/// in-process from a query this same module wrote, so there is no wire, no peer, and no
/// floor beneath a struct that never reaches the database. The place to make the state
/// unrepresentable is here.
///
/// # What it actually prevents (ADR-0062 erratum E6)
///
/// **`content_address IS NOT NULL` is the "did anything win" test — never
/// `subject_kind <> 'none'`.** `none` is a legal OPEN-VOCABULARY value a peer may send as a
/// real subject kind, and db/048's catch-all arm reports `'coarsened'` rather than echoing
/// it, so the two collided once already. A consumer keying on the phrase would see an
/// assertion that genuinely won, whose declared kind happens to be `none`, and report that
/// nothing applies — silently dropping the `content_address` an operator needs to feed
/// `sensitivity-withdraw --withdraws`, on the one surface that exists to make withdrawal
/// possible without raw SQL.
///
/// [`WinningSubject::from_row`] is the ONE place that decision is taken, and it reads the
/// address. There is no longer a second place to get it wrong.
#[derive(Debug)]
pub enum WinningSubject {
    /// Nothing applies to this subject: the grade is whatever the absence of any assertion
    /// produces, and there is nothing to withdraw.
    None,
    /// An assertion won. `content_address` is hex, and goes straight into
    /// `sensitivity-withdraw --withdraws`.
    Assertion {
        phrase: &'static str,
        content_address: String,
    },
}

impl WinningSubject {
    /// Build from one `cairn_effective_sensitivity` row. **Keys on the address, never the
    /// phrase** — see the type doc; that is the whole point of routing every construction
    /// through here.
    pub fn from_row(subject_kind: &str, content_address: Option<String>) -> Self {
        match content_address {
            Some(ca) => WinningSubject::Assertion {
                phrase: subject_kind_phrase(subject_kind),
                content_address: ca,
            },
            None => WinningSubject::None,
        }
    }

    /// The phrase a human reads: "chart-wide" | "this thread" | "this event" | "none" (or
    /// the coarsening/unrecognised phrases — see [`subject_kind_phrase`]).
    ///
    /// `&'static str`, not `String`: the producer already returns one, and the old code
    /// immediately `.to_string()`d it for no reason.
    pub fn phrase(&self) -> &'static str {
        match self {
            WinningSubject::None => subject_kind_phrase("none"),
            WinningSubject::Assertion { phrase, .. } => phrase,
        }
    }

    /// The hex `content_address` to withdraw, when there is one.
    pub fn content_address(&self) -> Option<&str> {
        match self {
            WinningSubject::None => None,
            WinningSubject::Assertion {
                content_address, ..
            } => Some(content_address),
        }
    }
}

#[cfg(test)]
mod tests {
    //! #387 item 3 — the correlated pair becomes a sum type, and in doing so makes
    //! ADR-0062's erratum E6 structurally unrepeatable.
    use super::*;

    #[test]
    fn an_assertion_that_won_carries_both_its_phrase_and_its_address() {
        let w = WinningSubject::from_row("patient", Some("a3f".into()));
        assert_eq!(w.phrase(), "chart-wide");
        assert_eq!(w.content_address(), Some("a3f"));
    }

    #[test]
    fn nothing_winning_carries_no_address_to_offer() {
        let w = WinningSubject::from_row("none", None);
        assert_eq!(w.phrase(), "none");
        assert_eq!(w.content_address(), None);
    }

    #[test]
    fn the_winner_is_decided_by_the_address_never_by_the_phrase() {
        // ADR-0062 ERRATUM E6, MADE STRUCTURAL. `content_address IS NOT NULL` is the "did
        // anything win" test; `subject_kind <> 'none'` is NOT, because `none` is a legal
        // OPEN-VOCABULARY value a peer can send as a real subject kind, and db/048's
        // catch-all arm reports `'coarsened'` rather than echoing it. A build that keyed on
        // the phrase would look at a winning assertion whose declared kind happens to be
        // "none" and report that nothing applies — dropping the address an operator needs
        // to withdraw it, on the surface that exists to make withdrawal possible.
        //
        // With the pair fused into one constructor there is no longer a place to make that
        // mistake: `from_row` reads the address and nothing else.
        let w = WinningSubject::from_row("none", Some("c0ffee".into()));
        assert_eq!(
            w.content_address(),
            Some("c0ffee"),
            "an assertion DID win — its odd declared kind must not erase it"
        );
        assert!(
            matches!(w, WinningSubject::Assertion { .. }),
            "keyed on the address, not the phrase"
        );
    }

    #[test]
    fn an_unrecognised_subject_kind_still_yields_a_usable_address() {
        // The other half: a FUTURE peer's kind. The phrase degrades honestly (chart-wide
        // reading, per `subject_kind_phrase`) while the address stays intact, so the
        // assertion remains withdrawable through the CLI by an operator on a build that
        // does not understand what it grades.
        let w = WinningSubject::from_row("episode", Some("dead".into()));
        assert_eq!(w.content_address(), Some("dead"));
        assert!(w.phrase().contains("unrecognised") || w.phrase().contains("chart-wide"));
    }
}
