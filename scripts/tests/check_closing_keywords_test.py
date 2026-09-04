#!/usr/bin/env python3
"""Tests for scripts/check_closing_keywords.py — the closing-keyword guard.

Why this suite exists at all: the guard it tests was written *after* seven issues
were silently closed by prose that said the opposite — #101, #115, #434, #441,
#468, #500 and #534. Every one of those sentences is a fixture below, verbatim,
so the guard is proven against the real text that fooled GitHub rather than
against text invented to make it pass.

Two properties matter and they pull in opposite directions:

  1. `find_closing_references` must track **GitHub's** parser, not a stricter
     or looser one. Reporting a reference GitHub ignores trains a reader to
     ignore the report; missing one it acts on is how an issue disappears.
     Where the two are known to diverge, a test says so out loud rather than
     leaving the next reader to rediscover it.
  2. `contradictions` must fire on text that *says* the issue is not closed
     while handing GitHub a reference that closes it.

Standard library only, no pytest — run it directly, the way
`scripts/tests/techdebt-loop-test.sh` is run:

    python3 scripts/tests/check_closing_keywords_test.py
"""

from __future__ import annotations

import sys
import time
import unittest
from pathlib import Path

# The guard lives one directory up. Import it by path so this file runs from
# anywhere without the repo needing to be an installed package.
sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

import check_closing_keywords as guard  # noqa: E402  (path set above)


class FindsWhatGitHubActsOn(unittest.TestCase):
    """Property 1: the reference list mirrors GitHub's own closing parser."""

    def issues(self, text: str) -> list[int]:
        return [ref.issue for ref in guard.find_closing_references(text)]

    def test_plain_keyword_and_hash_reference_closes(self) -> None:
        self.assertEqual(self.issues("Closes #511. Opens #541."), [511])

    def test_every_keyword_form_is_recognised(self) -> None:
        for keyword in (
            "close",
            "closes",
            "closed",
            "fix",
            "fixes",
            "fixed",
            "resolve",
            "resolves",
            "resolved",
        ):
            with self.subTest(keyword=keyword):
                self.assertEqual(self.issues(f"This {keyword} #42 today"), [42])

    def test_keyword_is_case_insensitive(self) -> None:
        self.assertEqual(self.issues("FIXES #79"), [79])

    def test_a_colon_between_keyword_and_reference_still_closes(self) -> None:
        # This is the form that closed #534, #441 and #468.
        self.assertEqual(self.issues("Filed rather than fixed: #534"), [534])

    def test_a_markdown_link_reference_still_closes(self) -> None:
        # The form that closed #500: the reference is wrapped in a link.
        body = "It does not fix [#500](https://github.com/cairn-ehr/cairn-ehr/issues/500)."
        self.assertEqual(self.issues(body), [500])

    def test_a_bare_issue_url_still_closes(self) -> None:
        body = "resolves https://github.com/cairn-ehr/cairn-ehr/issues/177"
        self.assertEqual(self.issues(body), [177])

    def test_a_cross_repository_reference_still_closes(self) -> None:
        self.assertEqual(self.issues("closes cairn-ehr/cairn-ehr#12"), [12])

    def test_a_reference_split_across_lines_is_still_adjacent(self) -> None:
        # A markdown heading and the reference beneath it are adjacent tokens
        # once whitespace collapses, which is how GitHub reads them too.
        self.assertEqual(self.issues("## Filed rather than fixed\n\n#534 stays open"), [534])

    def test_a_parenthesised_keyword_still_closes(self) -> None:
        """`(closes #N)` is a close, and this repository's own history proves it.

        PR #42's title ended `(closes #38)`. That parenthesised phrase was the
        ONLY closing adjacency anywhere in the PR — its body says `issue #38`
        three times with no keyword beside it, and no branch commit carries one
        either. #38 closed at 21:33:06 on 2026-06-23, **one second** after PR #42
        merged at 21:33:05, and `closedByPullRequestsReferences` is empty (no
        body link, no manual close). Nine merged PR titles here use this shape.

        The guard was blind to all of them until the lookbehind stopped
        excluding a keyword that merely follows an open parenthesis.
        """
        self.assertEqual(self.issues("harden(cairn-node): real genesis HLC (closes #38)"), [38])
        self.assertEqual(self.issues("Rework the paging loop (fixes #123)."), [123])
        # The bracket variants always worked; the asymmetry is what gave the bug away.
        self.assertEqual(self.issues("[closes #123]"), [123])
        self.assertEqual(self.issues("{fixes #123}"), [123])

    def test_the_cross_block_adjacency_that_closed_449(self) -> None:
        """A markdown heading closes the FIRST reference in the block beneath it.

        PR #448's body carries `## Filed, not fixed` followed by #449 and then
        #450-#454. #449 closed at that merge (23:11:24) and was reopened 38
        minutes later; **#450-#454 all stayed open**. That split is the proof:
        GitHub bridges the block boundary, and adjacency reaches exactly one
        reference. It is also why `normalize` collapses every run of whitespace.
        """
        body = "## Filed, not fixed\n\n[#449](https://github.com/o/r/issues/449), #450, #451"
        self.assertEqual(self.issues(body), [449])

    # --- the other half of property 1: what GitHub does NOT act on ----------

    def test_the_conventional_commit_scope_does_not_close(self) -> None:
        """`fix(#500):` is this repo's commit convention and it is SAFE.

        Proof from the repo's own history: `fix(#288)` and `fix(#530)` commits
        are on `main` with both issues still open. The parenthesis breaks the
        adjacency GitHub requires. Reporting these would flood every PR in this
        repo with false alarms and make the whole report worthless.
        """
        self.assertEqual(self.issues("fix(#500): the medium carries no clinical event"), [])
        self.assertEqual(self.issues("feat(#511): the custody plane takes Secret32"), [])
        self.assertEqual(self.issues("fix(#405,#412): close two live leaks"), [])

    def test_a_keyword_not_adjacent_to_the_reference_does_not_close(self) -> None:
        self.assertEqual(self.issues("We fixed the paging bug. #530 remains open."), [])

    def test_a_non_keyword_verb_does_not_close(self) -> None:
        # GitHub's keyword list has no "fixing", "addresses" or "closing".
        self.assertEqual(self.issues("Without fixing #12, and addresses #13"), [])

    def test_a_bare_reference_does_not_close(self) -> None:
        self.assertEqual(self.issues("#500 stays open; see also #101."), [])

    def test_a_reference_inside_a_code_span_does_not_close(self) -> None:
        """GitHub does not linkify a reference inside `code`, so it cannot close it.

        This is the same reasoning as the emphasis markers in `SEPARATOR`, run
        the other way: GitHub parses the *rendered* body, and in the rendered
        body these are `<code>` elements, not issue references.
        """
        self.assertEqual(self.issues("closes `#123`"), [])

    def test_the_GH_dash_reference_form_is_a_KNOWN_divergence(self) -> None:
        """`GH-123` is a GitHub reference form this guard does not report.

        Recorded as a test rather than left to be rediscovered. It has never
        been used in this repository (zero occurrences across 1649 commit
        messages and 216 merged PR bodies), so supporting it would add a shape
        no reviewer here can check against real evidence.
        """
        self.assertEqual(self.issues("fixes GH-123"), [])

    def test_every_reference_is_reported_and_duplicates_collapse(self) -> None:
        text = "closes #99, closes #100, and closes #99 again"
        self.assertEqual(guard.issues_that_will_close(text), [99, 100])


class FlagsTextThatContradictsItself(unittest.TestCase):
    """Property 2: a reference that closes an issue the text says is not closed."""

    def reasons(self, text: str) -> list[str]:
        return [reason for _, reason in guard.contradictions(text)]

    def test_the_sentence_that_closed_500(self) -> None:
        body = (
            "**It does not fix [#500](https://github.com/cairn-ehr/cairn-ehr/issues/500).** "
            "After this merges the medium still carries no clinical event."
        )
        found = guard.contradictions(body)
        self.assertEqual([ref.issue for ref, _ in found], [500])
        self.assertIn("not", found[0][1])

    def test_the_sentence_that_closed_534(self) -> None:
        body = "Filed rather than fixed: #534 (a freeze stops content convergence)"
        found = guard.contradictions(body)
        self.assertEqual([ref.issue for ref, _ in found], [534])
        self.assertIn("rather than", found[0][1])

    def test_the_sentence_that_closed_101(self) -> None:
        """A *partial* close is a contradiction too — the qualifier follows the reference."""
        body = "It does close **[#101](https://github.com/cairn-ehr/cairn-ehr/issues/101) item 1**"
        found = guard.contradictions(body)
        self.assertEqual([ref.issue for ref, _ in found], [101])
        self.assertIn("item", found[0][1])

    def test_a_deliberate_close_is_not_flagged(self) -> None:
        self.assertEqual(self.reasons("Closes #511. Opens #541."), [])

    def test_a_close_next_to_an_unrelated_negation_is_not_flagged(self) -> None:
        # The negation must be near the keyword, not merely somewhere in the text.
        text = "The restore no longer mints a fresh seed. Closes #495."
        self.assertEqual(self.reasons(text), [])

    def test_a_negation_before_a_CONJUNCTION_belongs_to_the_other_clause(self) -> None:
        """Punctuation alone cannot find every clause boundary.

        Both of these are correct, complete closes whose negation belongs to the
        clause before the conjunction — and "no new dependency" is prose this
        repository writes constantly. A guard that refuses them teaches its
        author to switch it off.
        """
        for body in (
            "This adds no new dependency and closes #144",
            "The restore never mints a fresh seed and closes #495",
        ):
            with self.subTest(body=body):
                self.assertEqual(self.reasons(body), [], body)

    def test_a_denial_spanning_a_conjunction_is_STILL_refused(self) -> None:
        # The conjunction cut must not weaken a real denial: a sentence that
        # denies on both sides of the "and" carries its own negation after it.
        self.assertEqual(
            len(self.reasons("This does not fix the medium and so does not close #500")), 1
        )

    def test_stays_open_before_the_keyword_is_a_negation(self) -> None:
        self.assertEqual(len(self.reasons("#500 stays open; this does not close #500")), 1)

    def test_a_qualifier_far_after_the_reference_is_not_a_contradiction(self) -> None:
        text = "Closes #511. The custody plane is now typed, which took part of a day."
        self.assertEqual(self.reasons(text), [])

    def test_a_qualifier_belonging_to_the_NEXT_sentence_is_not_a_contradiction(self) -> None:
        """Real PR #493, which closed #480 correctly.

        "Closes #480. Partially addresses #490 (items 1–2…)" — the qualifier is
        about a different issue in the next sentence. Reading across the full
        stop is how a guard earns a reputation for crying wolf.
        """
        text = "Closes #489. Closes #482. Closes #480. Partially addresses #490 (items 1-2)."
        self.assertEqual(self.reasons(text), [])

    def test_a_qualifier_in_the_next_sentence_without_a_blank_line(self) -> None:
        # Real PR #271, which closed #200 correctly.
        text = "Closes #200. Third item of the Priority-6 design queue."
        self.assertEqual(self.reasons(text), [])

    def test_a_negation_belonging_to_a_parenthetical_aside_is_not_a_contradiction(self) -> None:
        """Real PR #256, which closed #154 correctly.

        The "never" is inside an em-dashed aside about the ordering contract, not
        a denial of the close that follows it.
        """
        text = (
            "an honest ordering contract (*content never waits, permissions always wait* "
            "— closes #154 structurally), and adjudication via supersede"
        )
        self.assertEqual(self.reasons(text), [])

    def test_a_parenthesised_partial_qualifier_is_a_contradiction(self) -> None:
        """The #101 defect wearing parentheses.

        `LINK_TARGET` exists to step over a markdown link target so that
        `[#101](url) item 1` reads the way a human reads it. It used to tolerate
        whitespace before the parenthesis, which let it swallow ANY following
        parenthetical — so the most natural way to write a partial close went
        unflagged while the unparenthesised form was refused.
        """
        for body in (
            "It does close #101 (item 1 only)",
            "Closes #480 (partially)",
            "Closes #101 (items 2-3 remain open)",
        ):
            with self.subTest(body=body):
                self.assertEqual(len(self.reasons(body)), 1, body)

    def test_an_em_dashed_partial_qualifier_is_a_contradiction(self) -> None:
        # An em dash after a reference introduces a qualifier; it does not end
        # the claim the way a full stop does.
        self.assertEqual(len(self.reasons("Closes #101 - item 1 only")), 1)
        self.assertEqual(len(self.reasons("Closes #101 \u2014 item 1 only")), 1)

    def test_a_universal_quantifier_makes_the_qualifier_mean_the_OPPOSITE(self) -> None:
        """Real PR #433, which closed #388 correctly.

        "Closes #388 (all four parts)" says *parts* and means the whole issue.
        It was found by re-running the guard over history after the forward
        window was widened to see parenthesised qualifiers — the widening is
        what exposed it, and this is the fixture that keeps it seen.
        """
        for body in (
            "Closes #388 (all four parts) - Closes #383 - Closes #421",
            "Closes #101 (both parts)",
            "Closes #101 (all items)",
        ):
            with self.subTest(body=body):
                self.assertEqual(self.reasons(body), [], body)

    def test_partly_is_a_partial_qualifier(self) -> None:
        # "partially" was covered and "partly" was not, in either word list.
        self.assertEqual(len(self.reasons("This partly fixes #101")), 1)
        self.assertEqual(len(self.reasons("Closes #101 partly")), 1)

    def test_a_word_that_merely_CONTAINS_a_negation_is_not_a_negation(self) -> None:
        """Word boundaries, which the qualifier half had and the negation half did not.

        Every one of these is a correct, complete close written in ordinary
        prose, and every one was refused: by "not" inside *notes*, *notable*,
        *annotation* and *denotes*, and by "in part" inside *in particular*. A
        guard that cries wolf on correct text gets switched off, and then it
        guards nothing.
        """
        for body in (
            "Release notes updated, fixes #500",
            "A notable simplification, fixes #500",
            "The annotation is now typed, closes #500",
            "In particular the paged pull, closes #500",
            "Denotes the sealed body, closes #500",
            "A minor note: closes #500",
        ):
            with self.subTest(body=body):
                self.assertEqual(self.reasons(body), [], body)

    def test_cannot_is_still_a_negation(self) -> None:
        """Guards the regression that word boundaries would otherwise introduce.

        "cannot fix #500" was refused only because "not" was matched as a bare
        substring of *cannot*. Adding boundaries removes that accident, so the
        word has to be named outright or the guard silently loses a real denial.
        """
        self.assertEqual(len(self.reasons("This cannot fix #500 yet")), 1)
        self.assertEqual(len(self.reasons("This can not fix #500 yet")), 1)

    def test_a_smart_apostrophe_contraction_is_still_a_negation(self) -> None:
        # U+2019 is what GitHub's own web editor produces, and PR bodies are
        # routinely drafted outside git. Both spellings must refuse.
        self.assertEqual(len(self.reasons("It doesn't fix #500")), 1)
        self.assertEqual(len(self.reasons("It doesn\u2019t fix #500")), 1)

    def test_a_phrase_with_no_closing_reference_is_never_flagged(self) -> None:
        # The correct rephrasing of every fixture above: keep the keyword away
        # from the reference. These must all pass cleanly.
        for safe in (
            "This does not address #500.",
            "#500 is not fixed by this slice.",
            "Filed rather than repaired: #534.",
            "It closes item 1 of #101 only.",
        ):
            with self.subTest(text=safe):
                self.assertEqual(self.reasons(safe), [])


class ReportsForAHuman(unittest.TestCase):
    """The rendered report and the exit status the CI job depends on."""

    def test_each_record_in_a_source_is_scanned_on_its_own(self) -> None:
        """Two commit messages must not be read as one run-on sentence.

        `git log --format=%B` concatenates messages with nothing between them,
        and `normalize` then collapses the newline, so the tail of one commit
        becomes adjacent to the head of the next — an adjacency GitHub never
        sees, and a hard refusal on a pull request where nothing is wrong. The
        workflow separates messages with a NUL and the report splits on it.
        """
        joined = (
            "refactor(pen): the stale row is documented rather than fixed"
            + guard.RECORD_SEPARATOR
            + "closes #530"
        )
        # Read as one run-on text the second commit's close inherits the first
        # commit's denial, which is a refusal no author can act on.
        self.assertEqual(len(guard.contradictions(joined)), 1)
        # Read as two records — the way GitHub reads them — it is a plain close.
        report = guard.build_report([("commit-messages.txt", joined)])
        self.assertEqual(report.exit_code, 0)
        self.assertIn("#530", report.text)

    def test_a_required_source_that_is_empty_REFUSES(self) -> None:
        """An empty commit list is broken plumbing, whatever the body says.

        The all-empty refusal cannot fire here: the body is present and clean,
        so without a per-source requirement the report is a confident all-clear
        over a scan that read no commit message at all. A pull request always
        carries at least one commit, so the CI job declares that source
        required and this is what enforces it.
        """
        report = guard.build_report(
            [("pr-body.txt", "Closes #511."), ("commit-messages.txt", "")],
            required=("commit-messages.txt",),
        )
        self.assertEqual(report.exit_code, 1)
        self.assertIn("commit-messages.txt", report.text)
        self.assertIn("REFUSED", report.text)

    def test_a_required_source_with_content_passes(self) -> None:
        report = guard.build_report(
            [("pr-body.txt", ""), ("commit-messages.txt", "a subject")],
            required=("commit-messages.txt",),
        )
        self.assertEqual(report.exit_code, 0)

    def test_a_clean_text_reports_ok_and_exits_zero(self) -> None:
        report = guard.build_report([("PR body", "Closes #511.")])
        self.assertEqual(report.exit_code, 0)
        self.assertIn("#511", report.text)

    def test_a_contradiction_exits_non_zero_and_names_the_source(self) -> None:
        report = guard.build_report(
            [
                ("PR body", "It does not fix #500."),
                ("commit messages", "fix(#500): a slice that closes nothing"),
            ]
        )
        self.assertEqual(report.exit_code, 1)
        self.assertIn("PR body", report.text)
        self.assertIn("#500", report.text)
        # The remedy must be in the output — a guard that only says "no" makes
        # the next author guess, and guessing is what produced the six.
        self.assertIn("does not address", report.text)

    def test_a_scan_with_nothing_in_it_REFUSES_rather_than_reporting_all_clear(self) -> None:
        """An empty scan is not a pass — it is a check that verified nothing.

        The plumbing that feeds this script is a `git log` range and a PR body
        written by a CI step. If either breaks (a shallow clone, a bad range), an
        all-empty scan prints exactly the same all-clear as a clean PR. This
        repository has been bitten by that shape before: an *empty backup medium*
        that sealed, verified and reported healthy (#500's own review wave).
        """
        report = guard.build_report([("pr-body.txt", ""), ("commit-messages.txt", "")])
        self.assertEqual(report.exit_code, 1)
        self.assertIn("NOTHING WAS SCANNED", report.text)

    def test_an_empty_pr_body_beside_real_commits_is_fine(self) -> None:
        # A PR may legitimately have no body; it can never have no commits.
        report = guard.build_report([("pr-body.txt", ""), ("commit-messages.txt", "a subject")])
        self.assertEqual(report.exit_code, 0)

    def test_the_report_says_how_much_of_each_source_it_read(self) -> None:
        report = guard.build_report([("pr-body.txt", "12345"), ("commit-messages.txt", "")])
        self.assertIn("pr-body.txt (5 characters)", report.text)
        self.assertIn("commit-messages.txt (empty)", report.text)

    def test_a_path_that_is_not_a_regular_file_raises_rather_than_reading_empty(self) -> None:
        """A directory must not become a silent empty source.

        `read_sources` tolerates a *missing* file so the script runs by hand over
        whichever inputs exist. A path that exists but is not a file is a
        different thing entirely — nobody meant it — and swallowing it produces
        an all-clear over nothing.
        """
        with self.assertRaises(IsADirectoryError):
            guard.read_sources([str(Path(__file__).resolve().parent)])

    def test_the_separator_does_not_backtrack_on_a_long_run_of_markers(self) -> None:
        """The separator must stay linear on input a fork can control.

        Two overlapping star classes made this quadratic: 6400 emphasis markers
        took roughly three-quarters of a second. The assertion is deliberately
        loose — it is guarding against a return to quadratic blow-up, not
        pinning a machine's speed.
        """
        started = time.perf_counter()
        guard.find_closing_references("closes " + "*" * 20000)
        self.assertLess(time.perf_counter() - started, 1.0)

    def test_the_report_lists_what_the_merge_will_close(self) -> None:
        report = guard.build_report([("PR body", "Closes #511 and closes #541.")])
        self.assertIn("511", report.text)
        self.assertIn("541", report.text)


if __name__ == "__main__":
    unittest.main(verbosity=2)
