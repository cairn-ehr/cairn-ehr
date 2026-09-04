#!/usr/bin/env bash
#
# Write the three texts GitHub will parse for a pull request, for
# scripts/check_closing_keywords.py to read.
#
# GitHub closes an issue from a closing keyword adjacent to a reference in any of
# them, so all three are collected:
#
#   pr-title.txt        the PR title — it becomes the merge commit's BODY, so it
#                       is parsed on the default branch like any commit message
#   pr-body.txt         the PR description
#   commit-messages.txt every commit in the PR, NUL-separated
#
# Inputs come from the environment, never from `${{ }}` interpolation in the
# calling workflow: title and body are attacker-controlled text and interpolation
# would splice them into the shell.
#
#   PR_TITLE   the pull request's title    (may be empty)
#   PR_BODY    the pull request's body     (may be empty)
#   BASE_SHA   fallback base for the commit range, used only when HEAD is not a
#              merge ref (see "Which commits" below)
#
# Usage:  scripts/collect_pr_text.sh [output-directory]   (default: .)
#
# Its own tests live in scripts/tests/collect_pr_text_test.sh.
set -euo pipefail

OUT_DIR="${1:-.}"
mkdir -p "$OUT_DIR"

# No trailing newline. A pull request may legitimately have no body, and
# `printf '%s\n' ""` writes one byte — which the checker then reports as
# "(1 characters)" rather than "(empty)". The difference between "read nothing"
# and "read clean text" is the whole point of the report's first line.
printf '%s' "${PR_TITLE:-}" > "$OUT_DIR/pr-title.txt"
printf '%s' "${PR_BODY:-}"  > "$OUT_DIR/pr-body.txt"

# Which commits.
#
# On a `pull_request` event, checkout leaves HEAD on the merge ref: a merge of
# this PR's head into the base tip GitHub merged against. So `HEAD^1` is that
# base tip, `HEAD^2` is the PR head, and `HEAD^1..HEAD^2` is exactly this PR's
# commits. It works for a FORK too, whose head SHA is not in this repository.
#
# BASE_SHA is deliberately NOT the first choice. It is the base tip stamped into
# the event payload, and it does not advance when the base branch alone moves —
# so `BASE_SHA..HEAD` sweeps in every commit merged to the base in between,
# reports their closes as this PR's, and can refuse this PR over a sentence
# already on the base branch that its author cannot rewrite.
if git rev-parse --verify --quiet HEAD^2 > /dev/null; then
  RANGE="$(git rev-parse HEAD^1)..$(git rev-parse HEAD^2)"
else
  RANGE="${BASE_SHA:?BASE_SHA is required when HEAD is not a merge ref}..HEAD"
fi
echo "Commit range: $RANGE"

# NUL between messages so the checker reads each commit on its own. Concatenated,
# the tail of one message becomes adjacent to the head of the next — an adjacency
# GitHub never sees, and a refusal no author can act on because no such sentence
# exists anywhere.
git log --format=%B%x00 "$RANGE" > "$OUT_DIR/commit-messages.txt"
