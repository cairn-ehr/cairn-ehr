//! §5.9 — which assertion produced a displayed grade, and how to withdraw it.
//!
//! PURE. No database, no I/O. Split out of `report.rs` because it is a value type with its
//! own invariant to hold, and holding that invariant is the whole reason it exists — see
//! [`WinningSubject`]. Keeping it beside the DB reads that happen to build it would bury
//! the two constructors that a future reader must not "simplify".
use super::subject_kind_phrase;

/// The payload of a winning assertion: the phrase a human reads, and the address they
/// withdraw.
///
/// **Both fields are private, and that is the entire mechanism.** They can only be filled
/// by [`WinningSubject::from_row`] or [`WinningSubject::coarsened`], both of which live in
/// this module and take the did-anything-win decision correctly. An earlier version made
/// them public and claimed in a doc comment that `from_row` was "the ONE place" — it was
/// not; any caller could write `Assertion { phrase, content_address }` and pair a phrase
/// with an address that disagrees with it, which is the exact defect the type exists to
/// prevent. A claim about construction has to be enforced by visibility or it is a wish.
///
/// Privacy also makes [`WinningSubject::content_address`]'s **hex** guarantee real rather
/// than documentary: `render::winner_clause` prints that value UNESCAPED on the strength
/// of it being hex, and now the only producers are the two constructors below, both fed by
/// `encode(…, 'hex')`.
#[derive(Debug)]
pub struct Winner {
    phrase: &'static str,
    content_address: String,
}

/// Which assertion produced a displayed grade, and how to withdraw it (#387).
///
/// # Why a sum type, and not the two correlated fields it replaces
///
/// `ChartReport` used to carry `chart_source: String` beside `chart_content_address:
/// Option<String>`, with a doc comment stating that the second is `None` exactly when the
/// first reads `"none"`. The invariant was real and provable from the SQL — and enforced by
/// nothing. Principle 12 does not shelter these types: they are read-model DTOs built
/// in-process from a query `report.rs` wrote, so there is no wire, no peer, and no floor
/// beneath a struct that never reaches the database. The place to make the state
/// unrepresentable is here.
///
/// # What it actually prevents (ADR-0062 erratum E6)
///
/// **`content_address IS NOT NULL` is the "did anything win" test — never
/// `subject_kind <> 'none'`.** `none` is a legal OPEN-VOCABULARY value a peer may send as a
/// real subject kind, and the two collided once already. A consumer keying on the phrase
/// would see an assertion that genuinely won, whose declared kind happens to be `none`, and
/// report that nothing applies — silently dropping the `content_address` an operator needs
/// to feed `sensitivity-withdraw --withdraws`, on the one surface that exists to make
/// withdrawal possible without raw SQL.
///
/// E6 already fixed the producing end: db/048's catch-all arm now reports `'coarsened'`
/// rather than echoing a peer's raw `subject_kind`, so the SQL cannot hand this build a
/// `("none", Some(addr))` row today. This type is therefore a **floor under a future
/// producer**, not a live bug being patched — which is why its guard test constructs that
/// shape by hand.
#[derive(Debug)]
pub enum WinningSubject {
    /// Nothing applies to this subject: the grade is whatever the absence of any assertion
    /// produces, and there is nothing to withdraw.
    None,
    /// An assertion won.
    Assertion(Winner),
}

impl WinningSubject {
    /// Build from one `cairn_effective_sensitivity` row. **Keys on the address, never the
    /// phrase** — see the type doc; that is the whole point of routing construction here.
    ///
    /// `content_address` is `Option` because on THIS query it genuinely answers the
    /// question: db/048 leaves it NULL exactly when the `LEFT JOIN LATERAL` found no
    /// winner. A caller that already knows an assertion won must use [`Self::coarsened`]
    /// instead, so that "nothing won" can never be inferred from the wrong signal.
    pub fn from_row(subject_kind: &str, content_address: Option<String>) -> Self {
        match content_address {
            Some(ca) => WinningSubject::Assertion(Winner {
                phrase: subject_kind_phrase(subject_kind),
                content_address: ca,
            }),
            None => WinningSubject::None,
        }
    }

    /// Build for a standing assertion that won with **no registration event to anchor it**
    /// (`report.rs`'s no-local-registration fallback).
    ///
    /// Separate from [`Self::from_row`], and the address is NOT optional, because that
    /// caller is in a categorically different position: "did anything win" was already
    /// answered by the row existing at all, and `cairn_sensitivity_standing` selects a
    /// `BYTEA PRIMARY KEY` (`sensitivity_assertion.content_address`, db/048 §6), so there is
    /// no NULL to interpret. Pinned by symbol rather than by line: the line number this used
    /// to carry was already stale, having moved when db/048 gained two comment blocks.
    ///
    /// Routing it through `from_row`'s `Option` — as the first version of this did — made
    /// the coarsening phrase a CONSEQUENCE of a column's nullability in another file. If
    /// that query were ever widened (a `LEFT JOIN`, a new arm), the report would silently
    /// print `"sequestered" (winning subject: none)`: a real standing grade next to the
    /// phrase meaning nothing applies, on the path that exists precisely because
    /// "answering 'routine' here is the disclosure direction". A non-optional parameter
    /// turns that into a `tokio_postgres` type error instead.
    pub fn coarsened(content_address: String) -> Self {
        WinningSubject::Assertion(Winner {
            phrase: subject_kind_phrase("coarsened"),
            content_address,
        })
    }

    /// The phrase a human reads: "chart-wide" | "this thread" | "this event" | "none" (or
    /// the coarsening/unrecognised phrases — see [`subject_kind_phrase`]).
    ///
    /// `&'static str`, not `String`, and this is a CONFIDENTIALITY property rather than an
    /// allocation saving: `render::render_assert_readback` prints this value UNESCAPED,
    /// unlike the peer-authored grade beside it. Peer text off `tokio-postgres` is always
    /// `String`, so a `&'static str` here makes routing peer text into that unescaped
    /// position type-impossible. Do not "simplify" it back to `String`.
    pub fn phrase(&self) -> &'static str {
        match self {
            WinningSubject::None => subject_kind_phrase("none"),
            WinningSubject::Assertion(w) => w.phrase,
        }
    }

    /// The hex `content_address` to withdraw, when there is one.
    pub fn content_address(&self) -> Option<&str> {
        match self {
            WinningSubject::None => None,
            WinningSubject::Assertion(w) => Some(&w.content_address),
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
        // OPEN-VOCABULARY value a peer can send as a real subject kind. A build that keyed
        // on the phrase would look at a winning assertion whose declared kind happens to be
        // "none" and report that nothing applies — dropping the address an operator needs
        // to withdraw it, on the surface that exists to make withdrawal possible.
        //
        // Constructed by hand because db/048's catch-all arm no longer EMITS this shape
        // (E6 replaced the echo with 'coarsened'). This is the floor under a future
        // producer, not a reproduction of a live bug.
        let w = WinningSubject::from_row("none", Some("c0ffee".into()));
        assert_eq!(
            w.content_address(),
            Some("c0ffee"),
            "an assertion DID win — its odd declared kind must not erase it"
        );
        assert!(
            matches!(w, WinningSubject::Assertion(_)),
            "keyed on the address, not the phrase"
        );
    }

    #[test]
    fn an_unrecognised_subject_kind_still_yields_a_usable_address() {
        // The other half: a FUTURE peer's kind. The phrase degrades honestly while the
        // address stays intact, so the assertion remains withdrawable through the CLI by an
        // operator on a build that does not understand what it grades.
        //
        // Asserted against `subject_kind_phrase` EXACTLY. The first version of this test
        // said `contains("unrecognised") || contains("chart-wide")`, which the honest
        // phrase satisfies on both counts — and so does a bare "chart-wide". It could not
        // tell an honestly-degraded reading from a confident, wrongly-targeted one, which
        // is the only distinction it existed to draw.
        let w = WinningSubject::from_row("episode", Some("dead".into()));
        assert_eq!(w.content_address(), Some("dead"));
        assert_eq!(w.phrase(), subject_kind_phrase("episode"));
        assert_ne!(
            w.phrase(),
            subject_kind_phrase("patient"),
            "a kind this build cannot read must never render as a confident chart-wide grade"
        );
    }

    #[test]
    fn a_coarsened_winner_keeps_its_phrase_and_its_address() {
        // The no-local-registration fallback. Its phrase must NOT be reachable from the
        // "nothing applies" path: an operator reading "none" while a standing sequestered
        // grade is in force is the disclosure direction.
        let w = WinningSubject::coarsened("beef".into());
        assert_eq!(w.content_address(), Some("beef"));
        assert_eq!(w.phrase(), subject_kind_phrase("coarsened"));
        assert_ne!(w.phrase(), subject_kind_phrase("none"));
    }
}
