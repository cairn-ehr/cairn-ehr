#!/usr/bin/env bash
# scripts/codeql-alerts.sh — read-only CodeQL alert triage, in ONE allowlistable shape.
#
# WHY THIS EXISTS (2026-09-01, issue #527). `gh api` is DENIED repo-wide, in both
# .claude/settings.json and ~/.claude/settings.json, and that deny is correct: `gh api`
# is a general HTTP client that can POST and DELETE, and permission rules are PREFIX
# matches on the command string, so they cannot separate a read from a write —
# `gh api -X DELETE repos/…` and `gh api repos/…` share the prefix `gh api `, because
# the method flag comes BEFORE the path. Deny also beats allow categorically (proof: the
# repo already allows `Bash(gh:*)`, and `gh api` is still refused), so no narrow allow
# rule can carve out an exception.
#
# The cost of that — correct — boundary was that CodeQL findings became untriageable
# from a session: the alert text lives only in line-level PR review comments and the
# code-scanning API, both API-only, and default setup publishes no SARIF artifact. So
# every CodeQL finding, real or false positive, cost a human round-trip to the Security
# tab (see #527).
#
# The fix is the one scripts/run-db-gated-tests.sh already uses for the same class of
# problem: permission rules match the command string SUBMITTED, not the commands a
# script runs internally. Baking one read-only query into a script gives it a single
# allowlistable shape — `Bash(scripts/codeql-alerts.sh)` — while leaving the `gh api`
# deny completely intact for everything else.
#
# CLOSED SURFACE, deliberately. Each of these is what keeps the allow rule narrow:
#   - takes NO arguments, and the allow rule carries no trailing wildcard, so nothing
#     can be appended to widen it;
#   - the repository is HARD-CODED below — it cannot be pointed at another repo;
#   - it issues GET only. There is no method flag anywhere in this file and no way for
#     a caller to supply one.
#
# The residual, stated plainly: anyone who can edit this file changes what the
# allowlisted command does. That is true of every script-shaped allowlist entry the repo
# already has; it is why the no-arguments rule matters, and why this script is committed
# rather than kept out of tree — what it does is reviewable in the diff.
set -euo pipefail

cd "$(dirname "$0")/.."   # repo root, same convention as the sibling scripts

# Closed surface: no arguments. See the header.
if [ "$#" -ne 0 ]; then
    echo "codeql-alerts.sh takes no arguments (closed surface — see the header)" >&2
    exit 2
fi

# Hard-coded on purpose: a derived value (git remote, `gh repo view`) would be a way to
# aim this at a different repository without editing the file.
REPO="cairn-ehr/cairn-ehr"

echo "== open CodeQL alerts for ${REPO}"

# `|| true` on the pipeline head: a repo with zero alerts, or a token without
# security-events scope, must print an honest message rather than tripping `set -e` and
# leaving the caller to guess which of the two happened.
alerts="$(gh api "repos/${REPO}/code-scanning/alerts?state=open&per_page=100" \
    --paginate 2>/dev/null || true)"

if [ -z "$alerts" ]; then
    echo "   (none returned — either there are no open alerts, or this token lacks the"
    echo "    security-events scope; check with: gh auth status)"
    exit 0
fi

count="$(printf '%s' "$alerts" | jq -s 'add | length')"
if [ "$count" = "0" ]; then
    echo "   none — the alert gate should be green"
    exit 0
fi

echo "   ${count} open"
echo

printf '%s' "$alerts" | jq -s -r '
  add
  | sort_by(.rule.security_severity_level // "zzz", .rule.id)
  | .[]
  | "  #\(.number)  [\(.rule.security_severity_level // .rule.severity // "?")] \(.rule.id)\n"
  + "      \(.most_recent_instance.location.path // "?"):"
  + "\(.most_recent_instance.location.start_line // 0)\n"
  + "      \(.rule.description // .rule.name // "")\n"
  + "      \(.most_recent_instance.message.text // "(no message)")\n"
  + "      ref: \(.most_recent_instance.ref // "?")\n"
  + "      \(.html_url)\n"
'

echo "To dismiss a false positive, use the Security tab — this script is read-only."
