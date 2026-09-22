//! When the machine runs the registration search *unasked* — step 3 of the funnel.
//!
//! # The workflow this serves
//!
//! A clerk who found nothing by browsing moves to the registration form. Once that form holds
//! enough to identify a person, a search runs in the background over the completed data and,
//! if it finds likely matches, asks *could this be one of these existing patients?* The point
//! is keystroke economy: the cheap fragment lookup catches the common case, and the expensive
//! exact check happens once, automatically, on data the clerk has already typed for another
//! reason.
//!
//! # THIS RULE IS ADVISORY. IT IS NEVER A GATE.
//!
//! Read that twice before changing anything here, because the failure mode is silent and
//! clinical. The rule below decides whether the search *also* runs early — never whether it
//! runs at all, and never whether a chart may be created. A registration always carries a
//! search; [`crate::token::TokenStore::record`] mints a token for a form this module calls
//! `Waiting`, and deliberately does not consult this module at all.
//!
//! If it ever did, a mononymous patient — or one whose date of birth is genuinely unknown —
//! could not be registered without someone typing a second name or a date that nobody knows.
//! That is principle 4 inverted: a required field satisfiable only by fabrication. It is also
//! a paper-parity defect, because the paper desk has no such rule; a clerk writes the card
//! with what they were told. `a_mononymous_form_with_no_birth_date_still_gets_a_token` in
//! [`crate::token`] is the test that holds this open.
//!
//! # Why name TOKENS and not a given name and a surname
//!
//! The design said "a given name, a surname and a date of birth" until this slice was planned
//! against the code. That phrasing implies two separate fields, which is *one culture's name
//! model* — the cultural capture [ADR-0014](../../../docs/spec/decisions/0014-locale-pluggable-matcher-comparators.md)
//! forbids — and it fails outright for a mononymous patient, a patronymic, or Han name order.
//!
//! It also forced the raw name to be *reassembled* from two boxes before it could be
//! searched, and `cairn_node::patient::register::register_patient`'s own doc warns that its
//! `name` argument **must be the same typed string the `SearchQuery` was built from**, with
//! nothing in the types able to enforce it. Counting whitespace-separated tokens over ONE
//! free name field — exactly what the CLI's `--name` already is — carries the same
//! information with no name model, and lets one typed string feed both. The drift that doc
//! warns about stops being possible rather than being merely discouraged.

/// How many whitespace-separated name tokens the early search waits for.
///
/// Two, because one token is a fragment — it is precisely what the clerk typed in the browse
/// step and already found nothing for, so searching on it again unasked would cost a round
/// trip to re-learn what the screen already shows. Two tokens is the point at which the
/// search is asking a genuinely new question.
///
/// A named constant with a test pinning it, so a change is visible in a diff and has to be
/// argued for, rather than being a number buried in a comparison.
pub const MIN_NAME_TOKENS: usize = 2;

/// A part of the form the early search is still waiting on.
///
/// Carried so the screen can *say* what it is waiting for. A form that silently declines to
/// search leaves the clerk to guess whether it is thinking, broken, or satisfied — and
/// principle 4 is explicit that *not-yet-asked* is a state worth naming, distinct from
/// silence and distinct from a negative answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MissingPart {
    /// Fewer name tokens than [`MIN_NAME_TOKENS`]. `have` travels with `need` because
    /// "type more of the name" without saying how much more is a guessing game.
    NameTokens { have: usize, need: usize },
    /// No date of birth yet.
    BirthDate,
}

/// Whether the machine should search now, unasked — and if not, what it is waiting for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TriggerState {
    /// Enough is typed. Run the registration search in the background.
    Ready,
    /// Not yet. The parts are listed in field order, so the screen reports them all at once
    /// rather than revealing the second only after the first is satisfied.
    Waiting(Vec<MissingPart>),
}

/// Count the name tokens in raw field text.
///
/// `split_whitespace` is deliberate and load-bearing on two counts. It never yields an empty
/// token, so `"   "` counts as zero rather than one (a `split(' ')` would count three). And it
/// splits on whitespace ONLY, so `"O'Brien-Smith"` is one token — matching how
/// `cairn_patient_search::SearchQuery::new` treats one whitespace-delimited word as one whole
/// token, and how `db/046`'s pass 3 tokenises a stored name. Counting punctuation-separated
/// parts here would let a single hyphenated surname trip the trigger on its own, firing the
/// early search on half a person.
fn name_token_count(raw_name: &str) -> usize {
    raw_name.split_whitespace().count()
}

/// Should the machine search NOW, unasked?
///
/// Pure, and takes the raw field text rather than a parsed form, so the caller owns every
/// decision about what the fields mean. See the module doc: the answer is **advisory**.
pub fn trigger_state(raw_name: &str, birth_date: &str) -> TriggerState {
    let mut missing = Vec::new();

    let have = name_token_count(raw_name);
    if have < MIN_NAME_TOKENS {
        missing.push(MissingPart::NameTokens {
            have,
            need: MIN_NAME_TOKENS,
        });
    }
    // Blank-after-trim counts as "nothing supplied", the same rule `SearchQuery::new` and
    // `register_patient` both apply to a birth date. A field holding only spaces must never
    // read as an answer.
    if birth_date.trim().is_empty() {
        missing.push(MissingPart::BirthDate);
    }

    if missing.is_empty() {
        TriggerState::Ready
    } else {
        TriggerState::Waiting(missing)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_minimum_is_two_name_tokens() {
        // Pinned so a change is argued for in a diff, not made in passing.
        assert_eq!(MIN_NAME_TOKENS, 2);
    }

    #[test]
    fn two_name_tokens_and_a_birth_date_are_enough_to_search_unasked() {
        assert_eq!(
            trigger_state("Samantha Michaelowski", "1984-03-02"),
            TriggerState::Ready
        );
    }

    #[test]
    fn one_name_token_is_not_yet_enough_and_the_form_can_say_why() {
        // The `have` is load-bearing, not decoration: a form that says "type more of the
        // name" without saying how much more makes the clerk guess.
        assert_eq!(
            trigger_state("Michaelowski", "1984-03-02"),
            TriggerState::Waiting(vec![MissingPart::NameTokens { have: 1, need: 2 }])
        );
    }

    #[test]
    fn a_name_with_no_birth_date_is_waiting_on_the_birth_date_alone() {
        assert_eq!(
            trigger_state("Samantha Michaelowski", ""),
            TriggerState::Waiting(vec![MissingPart::BirthDate])
        );
    }

    #[test]
    fn a_birth_date_of_only_spaces_is_not_a_birth_date() {
        // Same blank-after-trim rule `SearchQuery::new` and `register_patient` both apply.
        // A field holding spaces — pasted, or left over from a clear — must not read as an
        // answer that satisfies the trigger.
        assert_eq!(
            trigger_state("Samantha Michaelowski", "   "),
            TriggerState::Waiting(vec![MissingPart::BirthDate])
        );
    }

    #[test]
    fn an_empty_form_is_waiting_on_both_parts_not_on_one() {
        // Both, in field order. Reporting only the first missing part makes the clerk
        // discover the second one only after satisfying the first — two waits where the
        // form could have asked once.
        assert_eq!(
            trigger_state("", ""),
            TriggerState::Waiting(vec![
                MissingPart::NameTokens { have: 0, need: 2 },
                MissingPart::BirthDate,
            ])
        );
    }

    #[test]
    fn whitespace_is_not_a_name_token() {
        // `split_whitespace` yields nothing for this; a `split(' ')` would yield three empty
        // strings and report `have: 3`, tripping the trigger on an empty field. Pinned so a
        // future switch cannot pass silently.
        assert_eq!(
            trigger_state("   ", "1984-03-02"),
            TriggerState::Waiting(vec![MissingPart::NameTokens { have: 0, need: 2 }])
        );
    }

    #[test]
    fn punctuation_does_not_split_a_name_into_more_tokens_than_the_clerk_typed() {
        // "O'Brien-Smith" is ONE token here, exactly as `SearchQuery::new` treats one
        // whitespace-delimited word as one whole token. If this counted alphanumeric parts,
        // a single hyphenated surname would satisfy MIN_NAME_TOKENS by itself and the early
        // search would fire on half a person.
        assert_eq!(
            trigger_state("O'Brien-Smith", "1984-03-02"),
            TriggerState::Waiting(vec![MissingPart::NameTokens { have: 1, need: 2 }])
        );
        // And the same surname with a given name beside it IS two tokens.
        assert_eq!(
            trigger_state("John O'Brien-Smith", "1984-03-02"),
            TriggerState::Ready
        );
    }

    #[test]
    fn a_name_in_a_non_latin_script_counts_its_tokens_the_same_way() {
        // Nothing in the rule names a script (ADR-0014). Two whitespace-separated tokens are
        // two tokens whether they are Latin, Han or Devanagari — and a Han name written
        // without a space is one token, which is the correct and honest answer: it is not
        // more identifying for being in another script.
        assert_eq!(
            trigger_state("阿明娜 李", "1984-03-02"),
            TriggerState::Ready
        );
        assert_eq!(
            trigger_state("李小明", "1984-03-02"),
            TriggerState::Waiting(vec![MissingPart::NameTokens { have: 1, need: 2 }])
        );
    }

    #[test]
    fn a_mononymous_patient_with_no_birth_date_never_becomes_ready() {
        // Principle 4, and the reason the module doc insists this rule is advisory. This is
        // NOT a refusal to register: see `token.rs`'s
        // `a_mononymous_form_with_no_birth_date_still_gets_a_token`, which mints a token for
        // exactly this form. The trigger only decides whether the search ALSO runs early.
        assert_eq!(
            trigger_state("Amina", ""),
            TriggerState::Waiting(vec![
                MissingPart::NameTokens { have: 1, need: 2 },
                MissingPart::BirthDate,
            ])
        );
    }
}
