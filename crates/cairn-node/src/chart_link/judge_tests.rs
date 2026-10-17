//! Tests for `chart_link/judge.rs`, in a sibling file (house rule 4: files under 500 lines).
use super::*;

#[test]
fn a_judgement_says_what_it_did_not_just_that_it_was_recorded() {
    use LinkEffect::*;
    // Outranked whenever the standing assertion says the opposite — whatever the rest of
    // the record reads.
    for verb in [LinkVerb::Link, LinkVerb::Unlink] {
        for joined in [false, true] {
            assert_eq!(link_effect(verb, false, joined), Outranked, "{verb:?}");
        }
    }
    assert_eq!(link_effect(LinkVerb::Link, true, true), TookEffect);
    assert_eq!(link_effect(LinkVerb::Unlink, true, false), TookEffect);
    assert_eq!(link_effect(LinkVerb::Unlink, true, true), StillJoined);
}

/// #797: the set a judgement reports as "the record it left" (`LinkOutcome::charts`, which the
/// window holds as its baseline) must be read AFTER the event applied — so it includes this
/// judgement — and BEFORE the commit, while the transaction still holds db/018's link lock, so no
/// colleague's link can land in between and be folded into the clinician's "own" baseline, never
/// to be announced. The order is a property of `judge`'s source, which no DB test can see break
/// (a read after the commit passes every single-writer test), so this guard reads the source.
#[test]
fn the_set_left_is_read_after_the_apply_and_before_the_commit() {
    let source = include_str!("judge.rs");
    // `judge`'s body alone: from its signature to the test module that closes the file.
    let start = source.find("\nasync fn judge(").expect("judge is declared");
    let end = start
        + source[start..]
            .find("\n#[cfg(test)]")
            .expect("the tests follow judge");
    let judge = &source[start..end];
    let at = |needle: &str| {
        let found = judge.find(needle);
        assert_eq!(
            judge.matches(needle).count(),
            1,
            "exactly one `{needle}` in judge"
        );
        found.expect("present")
    };
    let applied = at("let asserted = assert_link_in_tx(");
    let read = at("person_charts(&tx, record_of)");
    let committed = at("tx.commit()");
    assert!(
        applied < read,
        "the set is read after the judgement applied"
    );
    assert!(read < committed, "the set is read before the commit");
}
