#!/bin/bash
# Claude Code Stop hook — house rule 8: work that exists only on a local branch is invisible.
# When the session's branch carries commits beyond main that are not pushed, or has no PR,
# it blocks the stop ONCE PER COMMIT with a reminder. Claude may then push and open a
# (draft) PR, or say why not; either way the same commit is never nudged again, so a session
# that is deliberately mid-task is not nagged every turn.
# Silent on main, with nothing beyond main, and when gh cannot reach GitHub (offline is not
# "no PR"). Input: the hook JSON on stdin (fields used: stop_hook_active, cwd).
set -uo pipefail
input="$(cat)"
[ "$(jq -r '.stop_hook_active // false' <<<"$input")" = true ] && exit 0
cd "$(jq -r '.cwd // "."' <<<"$input")" 2>/dev/null || exit 0

branch="$(git symbolic-ref --short -q HEAD)" || exit 0
[ "$branch" = main ] && exit 0
[ "$(git rev-list --count origin/main..HEAD 2>/dev/null || echo 0)" -gt 0 ] || exit 0

head="$(git rev-parse HEAD)"
marker="$(git rev-parse --git-path cairn-stop-nudge)"
[ "$(cat "$marker" 2>/dev/null)" = "$branch $head" ] && exit 0

# Compare against origin/<branch>, not @{upstream}: a branch made with
# `git worktree add -b X origin/main` tracks origin/main, which would read as never pushed.
remote="refs/remotes/origin/$branch"
problem=""
if ! git rev-parse -q --verify "$remote" >/dev/null; then
  problem="branch '$branch' is not pushed"
elif [ "$(git rev-list --count "$remote..HEAD")" -gt 0 ]; then
  problem="branch '$branch' has $(git rev-list --count "$remote..HEAD") commit(s) not pushed"
elif ! err="$(gh pr view --json state 2>&1 >/dev/null)"; then
  case "$err" in *"no pull requests found"*) problem="branch '$branch' has no PR" ;; esac
fi
[ -n "$problem" ] || exit 0

echo "$branch $head" > "$marker"
jq -n --arg r "House rule 8: $problem. Before the session ends, push and open a PR (draft if \
incomplete), or tell the user why not. If you are mid-task and continuing, carry on — this \
reminder fires once per commit." '{"decision": "block", "reason": $r}'
