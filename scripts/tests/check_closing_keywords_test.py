#!/usr/bin/env python3
"""Tests for scripts/check_closing_keywords.py — the closing-keyword guard.

Why this suite exists at all: the guard it tests was written *after* six issues
were silently closed by prose that said the opposite (#101, #434, #441, #468,
#500, #534). Every one of those sentences is a fixture below, verbatim, so the
guard is proven against the real text that fooled GitHub rather than against
text invented to make it pass.

Two properties matter and they pull in opposite directions:

  1. `find_closing_references` must mirror **GitHub's** parser, not a stricter
     or looser one. Reporting a reference GitHub ignores trains a reader to
     ignore the report; missing one it acts on is how an issue disappears.
  2. `contradictions` must fire on text that *says* the issue is not closed
     while handing GitHub a reference that closes it.

Standard library only, no pytest — run it directly, the way
`scripts/tests/techdebt-loop-test.sh` is run:

    python3 scripts/tests/check_closing_keywords_test.py
"""

from __future__ import annotations

import sys
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

    def test_empty_input_is_clean(self) -> None:
        report = guard.build_report([("PR body", "")])
        self.assertEqual(report.exit_code, 0)

    def test_the_report_lists_what_the_merge_will_close(self) -> None:
        report = guard.build_report([("PR body", "Closes #511 and closes #541.")])
        self.assertIn("511", report.text)
        self.assertIn("541", report.text)


if __name__ == "__main__":
    unittest.main(verbosity=2)
