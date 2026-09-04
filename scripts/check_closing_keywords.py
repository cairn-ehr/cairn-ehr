#!/usr/bin/env python3
"""Refuse a PR whose text hands GitHub a closing reference it says is NOT a close.

## The defect this exists to prevent

GitHub closes an issue when a merged pull-request body or commit message puts one
of nine keywords (`close`/`closes`/`closed`, `fix`/`fixes`/`fixed`,
`resolve`/`resolves`/`resolved`) immediately before a reference to it. The parser
sees only that adjacency: it does not read the sentence. So all three of these
close the issue, and every one of them was written to say the opposite:

    It does not fix #500                 -> #500 closed  (PR #526, 2026-09-01)
    It does close #101 item 1            -> #101 closed  (PR #533, 2026-09-03)
    Filed rather than fixed: #534        -> #534 closed  (PR #539, 2026-09-03)

Six issues in this repository were closed that way — #101, #434, #441, #468, #500
and #534 — including the tracking issue for the disaster-recovery work that was
under construction at the time. Nothing anywhere reported it. A known defect
vanishing from the tracker is exactly what house rule 5 ("never let a known defect
pass silently") exists to stop, so the guard is mechanical rather than a convention.

## What it does NOT flag, and why that matters

This repository's commit convention is `fix(#500): <subject>`. That is **safe** —
the parenthesis breaks the adjacency GitHub requires, proven by `fix(#288)` and
`fix(#530)` commits sitting on `main` with both issues still open. A guard that
flagged those would fire on nearly every commit here and be switched off within a
week, so `find_closing_references` deliberately mirrors GitHub's parser exactly:
what it lists is what a merge will really close, no more and no less.

## Two outputs

1. **The report** — every issue this merge will close, so an author can see an
   unintended one before merging rather than a month later.
2. **The refusal** — exit status 1 when a closing reference sits in text that says
   the issue is not (or is only partly) closed. The remedy is always the same and
   is printed with the finding: keep the keyword away from the reference.

Standard library only. Run over any number of text files:

    python3 scripts/check_closing_keywords.py pr-body.txt commit-messages.txt

Each file's name is used as the source label in the report. Its own tests live in
`scripts/tests/check_closing_keywords_test.py`.
"""

from __future__ import annotations

import argparse
import re
import sys
from dataclasses import dataclass
from pathlib import Path

# --------------------------------------------------------------------------
# The vocabulary. Keep these three lists together: they are the whole policy.
# --------------------------------------------------------------------------

# GitHub's closing keywords, verbatim. Do not add to this list to be "safe" — a
# keyword GitHub ignores produces a finding no rephrasing can be justified for.
KEYWORDS = r"close[sd]?|fix(?:es|ed)?|resolve[sd]?"

# Phrases that, standing in the same clause just before the keyword, mean the
# author is denying the close. Matched as substrings of a lower-cased window, so
# "doesn't" is covered by "n't".
NEGATIONS = (
    "not",
    "n't",
    "never",
    "nothing",
    "none",
    "no ",
    "without",
    "rather than",
    "instead of",
    "neither",
    "nor ",
    "partially",
    "in part",
    "stays open",
    "still open",
    "remains open",
)

# Words that, following the reference, mean only a PART of the issue is done.
# A partial close is still a close as far as GitHub is concerned, which is how
# #101 lost its two remaining items.
PARTIAL_QUALIFIERS = ("item", "items", "part", "parts", "half", "only", "partially")

# How far back and forward to look. Both windows are additionally cut at the
# nearest clause boundary, which is what keeps three real, correct PR bodies from
# being flagged: "…no longer mints a seed. Closes #495." (the "no" is in the
# previous sentence), "Closes #480. Partially addresses #490" (the qualifier is
# about a different issue), and "(*content never waits…* — closes #154)" (the
# negation is inside an em-dashed aside). A guard that cries wolf on correct text
# gets switched off, and then it guards nothing.
LOOKBEHIND_CHARS = 60
LOOKAHEAD_CHARS = 16

# What ends a clause: sentence punctuation, both dashes, and either parenthesis.
CLAUSE_BOUNDARIES = ".;!?—–()"

# A reference in any of the three shapes GitHub honours: `#42`, `owner/repo#42`,
# or a full issue URL. `\[?` and `\]?` let a markdown link's visible text match,
# which is the shape that closed #500.
REFERENCE = (
    r"(?:"
    r"\[?#(?P<short>\d+)\]?"
    r"|[\w.-]+/[\w.-]+#(?P<cross>\d+)"
    r"|https?://github\.com/[\w.-]+/[\w.-]+/issues/(?P<url>\d+)"
    r")"
)

# What may sit between the keyword and the reference: spaces, at most one colon,
# and markdown emphasis markers. The emphasis markers are not cosmetic tolerance —
# GitHub parses the *rendered* body, so `It does close **[#101](…) item 1**` closed
# #101 with two asterisks standing between the keyword and the reference. Note what
# is NOT in this set: `(`. That single omission is what keeps this repository's
# `fix(#500):` commit convention out of the results, where it belongs.
SEPARATOR = r"[ \t*_~`]*:?[ \t*_~`]*"

# The adjacency GitHub requires. `(?<![\w(])` before the keyword stops a keyword
# that is merely the tail of a longer word — or of `fix(` — from matching.
CLOSING_PATTERN = re.compile(
    r"(?<![\w(])(?P<keyword>" + KEYWORDS + r")\b" + SEPARATOR + REFERENCE,
    re.IGNORECASE,
)

# A markdown link target following the visible text, e.g. the `(https://…)` in
# `[#101](https://…)`. Skipped before looking for a partial qualifier so that
# "…#101](url) item 1" is read the way a human reads it.
LINK_TARGET = re.compile(r"^\s*\([^)]*\)")


@dataclass(frozen=True)
class Reference:
    """One closing reference GitHub will act on.

    `text` is the matched snippet as written, so a finding can quote the author's
    own words back at them rather than describing them.
    """

    issue: int
    keyword: str
    text: str
    start: int
    end: int


def normalize(text: str) -> str:
    """Collapse every run of whitespace to a single space.

    GitHub does not care whether a heading and the reference beneath it are on
    one line or two, and neither may we: `## Filed rather than fixed` followed by
    `#534` is the same adjacency as `fixed: #534` on one line.
    """
    return re.sub(r"\s+", " ", text)


def find_closing_references(text: str) -> list[Reference]:
    """Every reference in `text` that a merge would act on, in order of appearance.

    Pure: no I/O, no state. This is the function that must match GitHub's own
    behaviour — see the module docstring on why being stricter is not "safer".
    """
    flat = normalize(text)
    references: list[Reference] = []
    for match in CLOSING_PATTERN.finditer(flat):
        number = match.group("short") or match.group("cross") or match.group("url")
        references.append(
            Reference(
                issue=int(number),
                keyword=match.group("keyword"),
                text=match.group(0),
                start=match.start(),
                end=match.end(),
            )
        )
    return references


def issues_that_will_close(text: str) -> list[int]:
    """The distinct issue numbers a merge of `text` would close, first-seen order."""
    seen: list[int] = []
    for reference in find_closing_references(text):
        if reference.issue not in seen:
            seen.append(reference.issue)
    return seen


def last_boundary(window: str) -> int:
    """Index of the last clause boundary in `window`, or -1 if there is none."""
    return max((window.rfind(char) for char in CLAUSE_BOUNDARIES), default=-1)


def first_boundary(window: str) -> int:
    """Index of the first clause boundary in `window`, or -1 if there is none."""
    found = [index for index in (window.find(char) for char in CLAUSE_BOUNDARIES) if index >= 0]
    return min(found) if found else -1


def preceding_clause(flat_text: str, position: int) -> str:
    """The words just before `position`, cut at the nearest clause boundary.

    Why the cut: a negation only denies the claim it shares a clause with.
    """
    window = flat_text[max(0, position - LOOKBEHIND_CHARS) : position]
    boundary = last_boundary(window)
    return window[boundary + 1 :] if boundary >= 0 else window


def following_qualifier(flat_text: str, position: int) -> str:
    """The words just after a reference, up to the next clause boundary.

    Any markdown link target (`[#101](https://…)`) is stepped over first, so the
    qualifier is read the way a human reads the rendered sentence.
    """
    rest = flat_text[position:]
    link = LINK_TARGET.match(rest)
    if link:
        rest = rest[link.end() :]
    window = rest[:LOOKAHEAD_CHARS]
    boundary = first_boundary(window)
    return window[:boundary] if boundary >= 0 else window


def contradiction_reason(flat_text: str, reference: Reference) -> str | None:
    """Why this reference contradicts its own sentence, or None if it does not.

    Two shapes, both drawn from real closures in this repository:
      * a denial before the keyword  — "It does **not** fix #500"
      * a qualifier after it         — "It does close #101 **item** 1"
    """
    clause = preceding_clause(flat_text, reference.start).lower()
    for phrase in NEGATIONS:
        if phrase in clause:
            return f'negated by "{phrase.strip()}" in the same clause'

    trailer = following_qualifier(flat_text, reference.end).lower()
    for word in PARTIAL_QUALIFIERS:
        if re.search(rf"\b{word}\b", trailer):
            return f'qualified as partial by "{word}" right after the reference'
    return None


def contradictions(text: str) -> list[tuple[Reference, str]]:
    """Every closing reference in `text` whose own sentence denies it."""
    flat = normalize(text)
    found: list[tuple[Reference, str]] = []
    for reference in find_closing_references(text):
        reason = contradiction_reason(flat, reference)
        if reason is not None:
            found.append((reference, reason))
    return found


@dataclass(frozen=True)
class Report:
    """What to print, and what to exit with. Building it does no I/O."""

    text: str
    exit_code: int


REMEDY = (
    "Remedy: keep the keyword away from the reference — GitHub reads adjacency, not "
    'meaning. Write "this does not address #500", "#500 is not fixed by this slice", '
    '"filed rather than repaired: #534", or "closes item 1 of #101 only". The '
    "conventional-commit scope `fix(#500):` is already safe: the parenthesis breaks the "
    "adjacency, so it never closes anything."
)


def build_report(sources: list[tuple[str, str]]) -> Report:
    """Render the report for a list of `(label, text)` sources.

    Pure, so the whole output is testable without files or a CI runner.
    """
    lines: list[str] = ["## Closing-keyword guard", ""]

    will_close: list[tuple[str, Reference]] = []
    contradicted: list[tuple[str, Reference, str]] = []
    for label, text in sources:
        for reference in find_closing_references(text):
            will_close.append((label, reference))
        for reference, reason in contradictions(text):
            contradicted.append((label, reference, reason))

    if will_close:
        numbers = []
        for _, reference in will_close:
            if reference.issue not in numbers:
                numbers.append(reference.issue)
        lines.append("Merging this will close: " + ", ".join(f"#{n}" for n in numbers))
        lines.append("")
        for label, reference in will_close:
            lines.append(f'  - #{reference.issue} — {label}: "{reference.text}"')
        lines.append("")
        lines.append("If any of those should stay open, rephrase before merging.")
    else:
        lines.append("Merging this closes no issue.")
    lines.append("")

    if contradicted:
        lines.append("### REFUSED — a closing reference contradicts its own sentence")
        lines.append("")
        for label, reference, reason in contradicted:
            lines.append(
                f'  - #{reference.issue} would be CLOSED by {label}: "{reference.text}" '
                f"— {reason}"
            )
        lines.append("")
        lines.append(REMEDY)

    return Report(text="\n".join(lines) + "\n", exit_code=1 if contradicted else 0)


def read_sources(paths: list[str]) -> list[tuple[str, str]]:
    """Load each path as a `(label, text)` pair; a missing file is an empty source.

    A missing file is tolerated because the CI job writes an empty PR body to disk
    when a pull request has none, and an absent body is not a defect.
    """
    sources: list[tuple[str, str]] = []
    for path in paths:
        file = Path(path)
        text = file.read_text(encoding="utf-8", errors="replace") if file.is_file() else ""
        sources.append((file.name, text))
    return sources


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument(
        "paths",
        nargs="+",
        help="text files GitHub will parse (PR body, commit messages)",
    )
    args = parser.parse_args(argv)

    report = build_report(read_sources(args.paths))
    print(report.text)
    return report.exit_code


if __name__ == "__main__":
    sys.exit(main())
