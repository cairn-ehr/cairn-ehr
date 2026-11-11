#!/bin/bash
# Tests for the three local guards that turn remembered rules into mechanical ones:
#
#   .githooks/commit-msg            — runs scripts/check_closing_keywords.py on the message
#                                     BEFORE the commit exists (the slip lives in the commit
#                                     message, and fixing it afterwards needs a force-push,
#                                     which is deny-listed repo-wide).
#   .githooks/pre-commit            — when a Cargo.toml/Cargo.lock is staged, refuses a stale
#                                     lockfile in ANY of the three Cargo trees (only CI's
#                                     `--locked` GUI clippy noticed before).
#   scripts/hooks/claude-stop-unpushed-work.sh — nudges a Claude session, once per commit, when its
#                                     branch has work that is not pushed or has no PR (house
#                                     rule 8: finished-but-unpushed work cost a whole session).
#
# Every case runs in a throwaway git repository under a temp dir with a stubbed `gh`, so
# nothing here touches the real repository, the network, or GitHub.
#
# Run: bash scripts/tests/git_hooks_test.sh
set -u

TESTS_RUN=0
TESTS_FAILED=0
SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
COMMIT_MSG_HOOK="$REPO_ROOT/.githooks/commit-msg"
PRE_COMMIT_HOOK="$REPO_ROOT/.githooks/pre-commit"
STOP_HOOK="$REPO_ROOT/scripts/hooks/claude-stop-unpushed-work.sh"

t_ok() { TESTS_RUN=$((TESTS_RUN + 1)); echo "ok   - $1"; }
t_fail() { TESTS_RUN=$((TESTS_RUN + 1)); TESTS_FAILED=$((TESTS_FAILED + 1)); echo "FAIL - $1"; }
t_assert_eq() { if [ "$2" = "$3" ]; then t_ok "$1"; else t_fail "$1: expected [$2] got [$3]"; fi; }
t_assert_contains() {
  case "$3" in *"$2"*) t_ok "$1" ;; *) t_fail "$1: [$2] not found in [$3]" ;; esac
}

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

# A fresh repository whose scripts/ holds the real closing-keyword checker, as the hook expects.
new_repo() {
  local dir="$WORK/$1"
  git init -q -b main "$dir"
  git -C "$dir" config user.name t
  git -C "$dir" config user.email t@example.invalid
  mkdir -p "$dir/scripts"
  cp "$REPO_ROOT/scripts/check_closing_keywords.py" "$dir/scripts/"
  echo "$dir"
}

# ---- commit-msg --------------------------------------------------------------------------
R="$(new_repo msg)"
run_msg_hook() { printf '%s\n' "$1" > "$R/MSG"; (cd "$R" && "$COMMIT_MSG_HOOK" MSG >/dev/null 2>&1); echo $?; }

t_assert_eq "commit-msg: a plain message passes" 0 "$(run_msg_hook 'fix(#500): tighten the guard')"
t_assert_eq "commit-msg: a disclaimed close is refused" 1 \
  "$(run_msg_hook "$(printf 'docs: notes\n\nFiled rather than fixed: #534')")"
t_assert_eq "commit-msg: git's own comment lines are not message text" 0 \
  "$(run_msg_hook "$(printf 'docs: notes\n# It does not fix #500')")"

# ---- pre-commit (lockfile currency) ------------------------------------------------------
# A root package path-depending on a sibling crate: adding the dependency without refreshing
# Cargo.lock makes `cargo metadata --locked` fail, with no registry or network involved.
R="$(new_repo lock)"
(
  cd "$R" || exit 1
  cargo new -q --lib --vcs none dep
  printf '[package]\nname = "root"\nversion = "0.1.0"\nedition = "2021"\n[workspace]\n' > Cargo.toml
  mkdir src && : > src/lib.rs
  cargo generate-lockfile -q --offline
  git add -A && git commit -qm init --no-verify
)
run_pre_commit() { (cd "$R" && "$PRE_COMMIT_HOOK" >/dev/null 2>&1); echo $?; }

echo x > "$R/notes.txt" && git -C "$R" add notes.txt
t_assert_eq "pre-commit: no Cargo file staged -> skips" 0 "$(run_pre_commit)"
git -C "$R" add Cargo.toml
t_assert_eq "pre-commit: Cargo.toml staged, lock current -> passes" 0 "$(run_pre_commit)"
printf '[dependencies]\ndep = { path = "dep" }\n' >> "$R/Cargo.toml"
git -C "$R" add Cargo.toml
t_assert_eq "pre-commit: Cargo.toml staged, lock stale -> refused" 1 "$(run_pre_commit)"

# ---- Stop hook ---------------------------------------------------------------------------
# A stub `gh` whose answer to `gh pr view` is chosen by STUB_GH.
mkdir -p "$WORK/bin"
cat > "$WORK/bin/gh" <<'EOF'
#!/bin/bash
case "$STUB_GH" in
  pr) echo OPEN ;;
  nopr) echo 'no pull requests found for branch "x"' >&2; exit 1 ;;
  *) echo 'error connecting to api.github.com' >&2; exit 1 ;;
esac
EOF
chmod +x "$WORK/bin/gh"

R="$(new_repo stop)"
git init -q --bare "$WORK/origin.git"
(
  cd "$R" || exit 1
  git remote add origin "$WORK/origin.git"
  echo a > a && git add a && git commit -qm init --no-verify
  git push -q origin main
)
run_stop() { # $1 = stop_hook_active, $2 = gh mode
  printf '{"stop_hook_active": %s, "cwd": "%s"}' "$1" "$R" \
    | PATH="$WORK/bin:$PATH" STUB_GH="$2" "$STOP_HOOK" 2>&1
}

t_assert_eq "stop: on main -> silent" "" "$(run_stop false nopr)"
git -C "$R" checkout -q -b feature
t_assert_eq "stop: branch with nothing beyond main -> silent" "" "$(run_stop false nopr)"
echo b > "$R/b" && git -C "$R" add b && git -C "$R" commit -qm b --no-verify
OUT="$(run_stop false nopr)"
t_assert_contains "stop: unpushed branch -> blocks" '"decision": "block"' "$OUT"
t_assert_contains "stop: says the branch is not pushed" "not pushed" "$OUT"
t_assert_eq "stop: same commit nudges only once" "" "$(run_stop false nopr)"

echo c > "$R/c" && git -C "$R" add c && git -C "$R" commit -qm c --no-verify
t_assert_eq "stop: never re-blocks a turn it already continued" "" "$(run_stop true nopr)"

git -C "$R" push -q -u origin feature
echo d > "$R/d" && git -C "$R" add d && git -C "$R" commit -qm d --no-verify && git -C "$R" push -q
t_assert_eq "stop: pushed and a PR exists -> silent" "" "$(run_stop false pr)"
echo e > "$R/e" && git -C "$R" add e && git -C "$R" commit -qm e --no-verify && git -C "$R" push -q
t_assert_contains "stop: pushed but no PR -> blocks naming the PR" "no PR" "$(run_stop false nopr)"
echo f > "$R/f" && git -C "$R" add f && git -C "$R" commit -qm f --no-verify && git -C "$R" push -q
t_assert_eq "stop: gh unreachable (offline) is not 'no PR' -> silent" "" "$(run_stop false offline)"

# `git worktree add -b X origin/main` (and --track) leaves X's UPSTREAM at origin/main, so
# "pushed" must mean origin/X exists and holds HEAD — not that @{upstream} does.
git -C "$R" checkout -q --track -b feat2 origin/main
echo g > "$R/g" && git -C "$R" add g && git -C "$R" commit -qm g --no-verify
git -C "$R" push -q origin feat2
t_assert_eq "stop: pushed, upstream still origin/main, PR exists -> silent" "" "$(run_stop false pr)"

echo
echo "$TESTS_RUN tests, $TESTS_FAILED failed"
[ "$TESTS_FAILED" -eq 0 ]
