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

Seven issues in this repository were closed that way — #101, #115, #434, #441,
#468, #500 and #534 — including the tracking issue for the disaster-recovery work
that was under construction at the time, and #115, which sat wrongly closed for
eight weeks. Nothing anywhere reported it. A known defect vanishing from the
tracker is exactly what house rule 5 ("never let a known defect pass silently")
exists to stop, so the guard is mechanical rather than a convention.

## What it does NOT flag, and why that matters

This repository's commit convention is `fix(#500): <subject>`. That is **safe** —
the parenthesis breaks the adjacency GitHub requires, proven by `fix(#288)` and
`fix(#530)` commits sitting on `main` with both issues still open. A guard that
flagged those would fire on nearly every commit here and be switched off within a
week, so `find_closing_references` tracks GitHub's parser rather than being
"safely" stricter than it.

It is not a perfect mirror, and the places it is known to differ are named here
and pinned by tests rather than left to be rediscovered:

  * a reference inside a code span (`` closes `#123` ``) is NOT reported —
    GitHub does not linkify inside `<code>`, so it does not close;
  * `GH-123`, a GitHub reference form, is NOT reported — it has never been used
    in this repository, so supporting it would add a shape no reviewer here can
    check against real evidence;
  * the pull-request TITLE is text GitHub parses (it becomes the merge commit's
    body), so the CI job passes it in as its own source — see the workflow.

## Two outputs

1. **The report** — every issue this merge will close, so an author can see an
   unintended one before merging rather than a month later.
2. **The refusal** — exit status 1 when a closing reference sits in text that says
   the issue is not (or is only partly) closed. The remedy is always the same and
   is printed with the finding: keep the keyword away from the reference.

Standard library only. Run over any number of text files:

    python3 scripts/check_closing_keywords.py --require commit-messages.txt \
        pr-title.txt pr-body.txt commit-messages.txt

Each file's name is used as the source label in the report. `--require` names a
source that cannot legitimately be empty, so broken plumbing refuses instead of
reporting an all-clear over text it never read.

The CI job writes those three files with `scripts/collect_pr_text.sh`. Tests:
`scripts/tests/check_closing_keywords_test.py` for this file,
`scripts/tests/collect_pr_text_test.sh` for the plumbing that feeds it.
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
# author is denying the close.
#
# Every one is matched on WORD BOUNDARIES. That is not tidiness: as bare
# substrings, "not" is inside *notes*, *notable*, *annotation* and *denotes*, and
# "in part" is inside *in particular* — so "Release notes updated, fixes #500", a
# correct and complete close, was refused. A guard that cries wolf on correct
# text gets switched off, and then it guards nothing.
#
# `cannot` and `can not` have to be named outright: they used to be caught only
# by accident, as substrings of the bare "not", and boundaries remove that
# accident along with the false alarms.
NEGATIONS = (
    r"\bnot\b",
    r"\bcannot\b",
    r"\bcan not\b",
    r"n't\b",  # doesn't, don't, didn't — `normalize` folds the smart apostrophe first
    r"\bnever\b",
    r"\bnothing\b",
    r"\bnone\b",
    r"\bno\b",
    r"\bwithout\b",
    r"\brather than\b",
    r"\binstead of\b",
    r"\bneither\b",
    r"\bnor\b",
    r"\bpartially\b",
    r"\bpartly\b",
    r"\bin part\b",
    r"\bstays open\b",
    r"\bstill open\b",
    r"\bremains open\b",
)

# Words that, following the reference, mean only a PART of the issue is done.
# A partial close is still a close as far as GitHub is concerned, which is how
# #101 lost its two remaining items.
PARTIAL_QUALIFIERS = (
    "item",
    "items",
    "part",
    "parts",
    "half",
    "only",
    "partially",
    "partly",
)

# Words that turn a partial qualifier into its opposite. "Closes #388 (all four
# parts)" — PR #433, a correct and complete close — says *parts* and means the
# whole issue. Without this the widened forward window flags it, and a guard that
# cries wolf on correct text gets switched off.
TRAILER_UNIVERSALS = ("all", "both", "every", "entire", "in full")

# How far back and forward to look. Each window is then cut at a boundary, and
# the two use DIFFERENT boundary sets (below) because a mark that ends a clause
# behind a reference often introduces one in front of it.
#
# The cutting is what keeps real, correct PR bodies from being flagged:
# "…no longer mints a seed. Closes #495." (the "no" is in the previous sentence),
# "Closes #480. Partially addresses #490" (the qualifier is about a different
# issue), and "(*content never waits…* — closes #154)" (the negation is inside an
# em-dashed aside). A guard that cries wolf on correct text gets switched off,
# and then it guards nothing.
LOOKBEHIND_CHARS = 60
LOOKAHEAD_CHARS = 16

# What ends a clause when looking BACKWARDS for a negation: sentence punctuation,
# both dashes, and either parenthesis. The parentheses and dashes are what keep a
# negation inside an aside — "(*content never waits…* — closes #154)", PR #256, a
# correct close — from being read as a denial of the close beside it.
CLAUSE_BOUNDARIES = ".;!?—–()"

# What ends the window when looking FORWARDS for a partial qualifier: only the
# marks that end a claim outright. A dash or an opening parenthesis after a
# reference does the opposite of ending it — it introduces the qualifier, as in
# "It does close #101 (item 1 only)". Reusing CLAUSE_BOUNDARIES here made every
# parenthesised and em-dashed partial close invisible.
TRAILING_BOUNDARIES = ".;!?"

# Coordinating conjunctions also end a clause when looking backwards, and
# punctuation alone cannot find them: "This adds no new dependency and closes
# #144" is a correct, complete close whose "no" belongs to the clause before the
# "and". They are matched as words, after the punctuation cut.
#
# This does not weaken a real denial that spans a conjunction, because such a
# sentence carries its own negation on both sides: "does not fix the medium and
# so does not close #500" still refuses on the second "not".
CLAUSE_CONJUNCTIONS = (" and ", " but ", " so ", " then ", " yet ")

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

# What may sit between the keyword and the reference: spaces, colons, and markdown
# emphasis markers. The emphasis markers are not cosmetic tolerance — GitHub parses
# the *rendered* body, so `It does close **[#101](…) item 1**` closed #101 with two
# asterisks standing between the keyword and the reference.
#
# Two characters are deliberately NOT in this set:
#   `(` — its omission is the ONLY thing keeping this repository's `fix(#500):`
#         commit convention out of the results, where it belongs. Do not add it,
#         and do not assume anything else is protecting that convention.
#   `` ` `` — GitHub parses the rendered body, and in the rendered body
#         ``closes `#123` `` is a `<code>` element, not a reference; it closes
#         nothing, so reporting it would be a false alarm.
#
# One class rather than two overlapping ones: the previous
# `[ \t*_~`]*:?[ \t*_~`]*` was ambiguous and backtracked quadratically on a long
# run of markers (6400 characters took three-quarters of a second), on input a
# fork can control.
SEPARATOR = r"[ \t*_~:]*"

# The adjacency GitHub requires. `(?<!\w)` before the keyword stops a keyword that
# is merely the tail of a longer word (`prefix #12`, `unfixes #12`) from matching.
#
# It says nothing about `fix(#500):` — a lookbehind inspects the character BEFORE
# the keyword, which there is a space. That convention is protected by `(` being
# absent from SEPARATOR, four lines up. This lookbehind used to exclude `(` as
# well, on the strength of that confusion, and the cost was total blindness to
# `(closes #N)` — the form that closed #38 one second after PR #42 merged, and
# the form nine merged pull-request titles in this repository use.
CLOSING_PATTERN = re.compile(
    r"(?<!\w)(?P<keyword>" + KEYWORDS + r")\b" + SEPARATOR + REFERENCE,
    re.IGNORECASE,
)

# A markdown link target following the visible text, e.g. the `(https://…)` in
# `[#101](https://…)`. Skipped before looking for a partial qualifier so that
# "…#101](url) item 1" is read the way a human reads it.
#
# The anchor is `^\(` and NOT `^\s*\(`: a real link target has no space before it.
# Tolerating one let this swallow any following parenthetical, so
# "It does close #101 (item 1 only)" — the most natural way to write a partial
# close — sailed through while the unparenthesised form was refused.
LINK_TARGET = re.compile(r"^\([^)]*\)")

# How the CI job separates one commit message from the next inside a single file.
# Without it `git log --format=%B` concatenates messages with nothing between
# them and `normalize` collapses the newline, so the tail of one commit becomes
# adjacent to the head of the next — an adjacency GitHub never sees, and a hard
# refusal on a pull request where nothing is wrong.
RECORD_SEPARATOR = "\0"


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
    """Fold the smart apostrophe, then collapse every run of whitespace to a space.

    **Whitespace.** GitHub does not care whether a heading and the reference
    beneath it are on one line or two, and neither may we. This repository has
    the proof: PR #448's body carries `## Filed, not fixed` and then #449
    followed by #450-#454. #449 closed at that merge and was reopened 38 minutes
    later; **#450-#454 all stayed open**. Only the first reference after the
    keyword closed — which is exactly this adjacency, read across a blank line.

    **The apostrophe.** U+2019 is what GitHub's own web editor produces, and pull
    request bodies are routinely drafted outside git. Folding it to ASCII lets
    one `n't` pattern cover both spellings of "doesn't". The substitution is one
    character for one character, so every offset into the result still lines up.
    """
    return re.sub(r"\s+", " ", text.replace("\u2019", "'"))


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


def first_boundary(window: str, boundaries: str = TRAILING_BOUNDARIES) -> int:
    """Index of the first boundary character in `window`, or -1 if there is none."""
    found = [index for index in (window.find(char) for char in boundaries) if index >= 0]
    return min(found) if found else -1


def preceding_clause(flat_text: str, position: int) -> str:
    """The words just before `position`, cut back to the start of their clause.

    Two cuts, because a clause ends at either kind of boundary: first at the
    nearest punctuation mark, then at the nearest coordinating conjunction.

    Why cut at all: a negation only denies the claim it shares a clause with.
    "The restore no longer mints a seed. Closes #495." and "This adds no new
    dependency and closes #144" are both complete, correct closes whose "no"
    belongs to the words before the boundary.
    """
    window = flat_text[max(0, position - LOOKBEHIND_CHARS) : position]
    boundary = last_boundary(window)
    clause = window[boundary + 1 :] if boundary >= 0 else window
    cut = max(clause.lower().rfind(word) for word in CLAUSE_CONJUNCTIONS)
    return clause[cut + 1 :] if cut >= 0 else clause


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
    for pattern in NEGATIONS:
        found = re.search(pattern, clause)
        if found:
            # Quote what was actually matched, not the pattern: a finding that
            # says `negated by "\bnot\b"` makes the author decode a regex.
            return f'negated by "{found.group(0)}" in the same clause'

    trailer = following_qualifier(flat_text, reference.end).lower()
    if any(re.search(rf"\b{word}\b", trailer) for word in TRAILER_UNIVERSALS):
        return None
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


def records(text: str) -> list[str]:
    """Split one source into the independent texts GitHub will parse.

    A file of commit messages holds many messages; GitHub reads each on its own.
    Scanning the concatenation instead lets the tail of one message form an
    adjacency — or supply a negation — for the head of the next, which is a
    refusal no author can act on because no such sentence exists.
    """
    return text.split(RECORD_SEPARATOR)


def build_report(
    sources: list[tuple[str, str]],
    required: tuple[str, ...] = (),
) -> Report:
    """Render the report for a list of `(label, text)` sources.

    `required` names the labels that must carry text. A pull request always has
    at least one commit message, so an empty `commit-messages.txt` is broken
    plumbing — but the all-empty refusal below cannot catch it while the body is
    present and clean, and the result is a confident all-clear over a scan that
    read no commit at all. The caller states which sources it knows cannot
    legitimately be empty; the script cannot know that on its own.

    Pure, so the whole output is testable without files or a CI runner.
    """
    lines: list[str] = ["## Closing-keyword guard", ""]

    # What was actually read, stated first. Without this an all-clear from broken
    # plumbing (a shallow clone, a bad `git log` range) is indistinguishable from
    # an all-clear from a clean PR — the shape that let an EMPTY backup medium
    # seal, verify and report healthy (#500's review wave). A check that verified
    # nothing must say so, and must not pass.
    sizes = ", ".join(
        f"{label} ({len(text)} characters)" if text else f"{label} (empty)"
        for label, text in sources
    )
    lines.append(f"Scanned: {sizes or 'nothing at all'}")
    lines.append("")

    missing = [label for label, text in sources if label in required and not text.strip()]
    if missing:
        lines.append("### REFUSED — A REQUIRED SOURCE WAS EMPTY")
        lines.append("")
        lines.append(
            "Empty: " + ", ".join(missing) + ". These sources cannot legitimately "
            "be empty — a pull request always has at least one commit message — so "
            "this is the plumbing that feeds the check, not the text it checks. "
            "The rest of the scan established nothing about them."
        )
        return Report(text="\n".join(lines) + "\n", exit_code=1)

    if not any(text.strip() for _, text in sources):
        lines.append("### REFUSED — NOTHING WAS SCANNED")
        lines.append("")
        lines.append(
            "Every source was empty, so this check established nothing. A pull request "
            "always has at least one commit message: an empty scan means the plumbing "
            "that feeds this script is broken (a shallow clone, or a base..head range "
            "that resolved to nothing), not that the text is clean."
        )
        return Report(text="\n".join(lines) + "\n", exit_code=1)

    will_close: list[tuple[str, Reference]] = []
    contradicted: list[tuple[str, Reference, str]] = []
    for label, text in sources:
        for record in records(text):
            for reference in find_closing_references(record):
                will_close.append((label, reference))
            for reference, reason in contradictions(record):
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

    A missing file is tolerated so the script can be run by hand over whichever
    inputs you happen to have. The CI job always writes all of its files, so an
    absent one there means the plumbing changed — which `--require` catches,
    because a required source that is absent reads as empty.

    A path that exists but is NOT a regular file (a directory, most plausibly)
    raises instead of quietly becoming an empty source: that is never a shape
    anyone intended, and swallowing it produces an all-clear over nothing.
    """
    sources: list[tuple[str, str]] = []
    for path in paths:
        file = Path(path)
        if file.exists() and not file.is_file():
            raise IsADirectoryError(f"{path} exists but is not a regular file")
        text = file.read_text(encoding="utf-8", errors="replace") if file.is_file() else ""
        sources.append((file.name, text))
    return sources


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument(
        "paths",
        nargs="+",
        help="text files GitHub will parse (PR title, PR body, commit messages)",
    )
    parser.add_argument(
        "--require",
        action="append",
        default=[],
        metavar="FILENAME",
        help=(
            "a source that must carry text; refuse if it is empty. Use it for "
            "inputs that cannot legitimately be empty, such as commit messages."
        ),
    )
    args = parser.parse_args(argv)

    report = build_report(read_sources(args.paths), required=tuple(args.require))
    print(report.text)
    return report.exit_code


if __name__ == "__main__":
    sys.exit(main())
