#!/bin/bash
# Tests for scripts/collect_pr_text.sh — the closing-keyword guard's input plumbing.
#
# Why this file exists: the guard's Python half is thoroughly tested, but every
# one of its inputs is produced by shell, and the shell is where the two failures
# that would matter most live — a commit range that includes commits the pull
# request did not author, and a body written in a way that reports "(1
# characters)" when it is empty. A checker fed the wrong text is not a control.
#
# Each test builds a throwaway git repository with the exact shape GitHub hands a
# `pull_request` event: a merge ref whose first parent is the CURRENT base tip,
# while BASE_SHA in the event payload is an OLDER one.
#
# Run: bash scripts/tests/collect_pr_text_test.sh
set -u

TESTS_RUN=0
TESTS_FAILED=0
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
COLLECT="$SCRIPT_DIR/../collect_pr_text.sh"

t_assert_eq() {
  TESTS_RUN=$((TESTS_RUN + 1))
  if [ "$2" = "$3" ]; then
    echo "ok   - $1"
  else
    TESTS_FAILED=$((TESTS_FAILED + 1))
    echo "FAIL - $1: expected [$2] got [$3]"
  fi
}

t_assert_absent() {
  TESTS_RUN=$((TESTS_RUN + 1))
  if ! grep -qF "$2" "$3"; then
    echo "ok   - $1"
  else
    TESTS_FAILED=$((TESTS_FAILED + 1))
    echo "FAIL - $1: [$2] should NOT appear in $3"
  fi
}

t_assert_present() {
  TESTS_RUN=$((TESTS_RUN + 1))
  if grep -qF "$2" "$3"; then
    echo "ok   - $1"
  else
    TESTS_FAILED=$((TESTS_FAILED + 1))
    echo "FAIL - $1: [$2] should appear in $3"
  fi
}

# build_pr_shaped_repo DIR — a repo in the shape a `pull_request` event presents.
#
#   base0  (BASE_SHA: the base tip stamped into the event payload)
#     |\
#     | feature   ("PR commit: does not address 999")
#     |
#   base1        ("someone else: Filed rather than fixed: 777")  <- merged since
#     |
#   HEAD = merge(base1, feature)   the merge ref GitHub recomputed
#
# Echoes BASE_SHA. The stale-payload gap between base0 and base1 is the whole
# point: it is what `BASE_SHA..HEAD` would wrongly sweep in.
build_pr_shaped_repo() {
  local dir="$1"
  git -C "$dir" init --quiet --initial-branch=main
  git -C "$dir" config user.email t@example.com
  git -C "$dir" config user.name Test
  git -C "$dir" commit --quiet --allow-empty -m "base0: the tip when the event fired"
  local base_sha
  base_sha="$(git -C "$dir" rev-parse HEAD)"

  git -C "$dir" checkout --quiet -b feature
  git -C "$dir" commit --quiet --allow-empty -m "PR commit: does not address 999"

  git -C "$dir" checkout --quiet main
  git -C "$dir" commit --quiet --allow-empty \
    -m "someone else's merged PR: Filed rather than fixed: 777"

  # The merge ref: first parent is the CURRENT base tip, second is the PR head.
  git -C "$dir" merge --quiet --no-ff -m "Merge feature into main" feature
  echo "$base_sha"
}

run_tests() {
  local tmp base_sha
  tmp="$(mktemp -d)"
  trap 'rm -rf "$tmp"' RETURN
  base_sha="$(build_pr_shaped_repo "$tmp")"

  (
    cd "$tmp" || exit 1
    PR_TITLE="feat: a title" PR_BODY="" BASE_SHA="$base_sha" \
      bash "$COLLECT" "$tmp/out" > "$tmp/stdout.txt" 2>&1
  )

  # The heart of it: a stranger's commit, merged to the base after the event was
  # stamped, must not be attributed to this pull request. Under BASE_SHA..HEAD it
  # would be — and its "Filed rather than fixed" would refuse this PR over text
  # its author cannot rewrite.
  t_assert_absent "a base-branch commit merged since BASE_SHA is not scanned" \
    "someone else" "$tmp/out/commit-messages.txt"
  t_assert_present "the PR's own commit IS scanned" \
    "PR commit" "$tmp/out/commit-messages.txt"

  # Proof the fixture is honest: the range this replaced really would have swept
  # the stranger's commit in. Without this the test above could pass vacuously.
  t_assert_eq "the OLD BASE_SHA..HEAD range really did include it" "1" \
    "$(git -C "$tmp" log --format=%B "$base_sha"..HEAD | grep -c 'someone else')"

  # Commit messages are NUL-separated so each is read on its own.
  t_assert_eq "commit messages are NUL-separated" "1" \
    "$(tr -dc '\0' < "$tmp/out/commit-messages.txt" | wc -c | tr -d ' ')"

  # An absent body must be zero bytes, so the report can say "(empty)" rather
  # than "(1 characters)" — the difference between "read nothing" and "read
  # clean text".
  t_assert_eq "an empty PR body is written as zero bytes" "0" \
    "$(wc -c < "$tmp/out/pr-body.txt" | tr -d ' ')"
  t_assert_eq "the PR title is captured" "feat: a title" \
    "$(cat "$tmp/out/pr-title.txt")"

  # A title or body holding shell metacharacters must survive verbatim: it
  # arrives as data through the environment, never as script text.
  (
    cd "$tmp" || exit 1
    PR_TITLE='$(touch /tmp/pwned) `id` "; rm -rf /"' PR_BODY="" BASE_SHA="$base_sha" \
      bash "$COLLECT" "$tmp/out2" > /dev/null 2>&1
  )
  t_assert_eq "a title full of shell metacharacters is stored verbatim" \
    '$(touch /tmp/pwned) `id` "; rm -rf /"' "$(cat "$tmp/out2/pr-title.txt")"
}

run_tests
echo
echo "$((TESTS_RUN - TESTS_FAILED))/$TESTS_RUN passed"
[ "$TESTS_FAILED" -eq 0 ]
