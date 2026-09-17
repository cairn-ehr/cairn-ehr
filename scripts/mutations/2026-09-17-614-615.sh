#!/usr/bin/env bash
# Mutation harness for #614/#615 (ADR-0072). THROWAWAY — committed only so the run's ledger in
# docs/superpowers/plans/2026-09-17-restore-loses-no-record-silently-614-615.md is reproducible.
#
# WHY THIS HARNESS HAS THE SHAPE IT HAS. PR #594's first M2–M6 run was DISCARDED, for two defects
# that this script exists not to repeat:
#
#   1. The deletion mutations could not be reverted by swapping "" back — the empty string is not
#      a unique anchor — so the revert failed SILENTLY and each mutation ran on top of its
#      predecessor, reporting confident kills for mutations never cleanly applied.
#   2. One mutation's block anchor started BELOW its leading comment, so the revert restored the
#      code and orphaned the comment.
#
# So: every mutation is a full-string swap both ways, the tree must be CLEAN before each one, and
# the revert is VERIFIED rather than assumed. `git diff --quiet` is the whole positive control —
# if the harness cannot prove it returned the tree to HEAD, it stops.
#
# A mutation that the COMPILER kills is recorded as such and says nothing about runtime (#594's
# M9 lesson: an unused import killed by -D warnings is not evidence about the code under test).
set -uo pipefail
cd "$(dirname "$0")/../.."

U=$(whoami)
export CAIRN_TEST_PG="host=127.0.0.1 port=5532 dbname=cairn_test user=$U"
export CAIRN_TEST_PG2="host=127.0.0.1 port=5532 dbname=cairn_test2 user=$U"
export CAIRN_TEST_PG3="host=127.0.0.1 port=5532 dbname=cairn_test3 user=$U"

fail() { echo "HARNESS ERROR: $*" >&2; exit 2; }

require_clean() {
    git diff --quiet || fail "tree is dirty before a mutation — refusing to start (this is the
        positive control; a dirty tree means a previous revert did not land)"
}

# swap <file> <from> <to> — refuses unless <from> occurs EXACTLY once.
swap() {
    local file=$1 from=$2 to=$3
    local n
    n=$(python3 - "$file" "$from" <<'PY'
import sys
print(open(sys.argv[1]).read().count(sys.argv[2]))
PY
)
    [ "$n" = "1" ] || fail "anchor occurs $n times in $file (need exactly 1): ${from:0:60}..."
    python3 - "$file" "$from" "$to" <<'PY'
import sys
p, a, b = sys.argv[1], sys.argv[2], sys.argv[3]
s = open(p).read()
open(p, "w").write(s.replace(a, b, 1))
PY
}

# run_mutation <id> <expected> <file> <from> <to> <test-cmd...>
run_mutation() {
    local id=$1 expected=$2 file=$3 from=$4 to=$5; shift 5
    require_clean
    swap "$file" "$from" "$to"
    git diff --quiet && fail "$id: the mutation changed nothing — the anchor did not apply"

    local out rc
    out=$("$@" 2>&1); rc=$?

    # Revert, then PROVE it landed.
    swap "$file" "$to" "$from"
    git diff --quiet || fail "$id: REVERT DID NOT LAND — stopping before the next mutation runs
        on top of this one (the #594 failure). Fix by hand: git diff"

    local verdict="SURVIVED"
    [ "$rc" -ne 0 ] && verdict="KILLED"
    if echo "$out" | grep -qE "^error(\[|:)"; then
        verdict="KILLED (compiler — says nothing about runtime)"
    fi
    printf '%-4s expected %-9s actual %s\n' "$id" "$expected" "$verdict"
    [ "$verdict" != "${expected}" ] && echo "     ^ DIVERGENCE — investigate; see the plan's ledger"
    return 0
}

echo "=== #614/#615 mutation run — $(date -u +%FT%TZ) ==="
require_clean

NODE_TEST=(cargo test -p cairn-node --test)

# M1 — the #608 fail-open, reintroduced. The absent-row arm must catch it.
run_mutation M1 KILLED db/053_substitution_guard.sql \
    'IF p_found_ca IS DISTINCT FROM p_new_ca THEN' \
    'IF p_found_ca <> p_new_ca THEN' \
    "${NODE_TEST[@]}" substitution_guard

# M2 — #615 itself, undone. The node-plane attack test must catch it.
run_mutation M2 KILLED db/009_node_supersede_and_restore.sql \
    '    SELECT content_address INTO v_found FROM node_event WHERE node_event_id = v_eid;
    PERFORM cairn_refuse_substitution(v_found, v_ca, v_eid, '"'"'restore_node_event'"'"');' \
    '    -- (mutation M2: guard deleted)' \
    "${NODE_TEST[@]}" restore_one_node_event_id_one_body

# M3 — db/005's call deleted, db/020's left standing. The strict door's substitution arm
# lives in late_custody_reaches_the_chart.rs (it also pins that the guard precedes the
# late-custody call, ADR-0070 decision 1) — NOT in seal_submit, where a first guess put it.
run_mutation M3 KILLED db/005_submit.sql \
    '        PERFORM cairn_refuse_substitution(
            (SELECT content_address FROM event_log WHERE event_id = v_event_id),
            v_ca, v_event_id, '"'"'submit_event'"'"');' \
    '        NULL; -- (mutation M3: guard deleted)' \
    "${NODE_TEST[@]}" late_custody_reaches_the_chart

# M4 — db/020's call deleted, db/005's left standing.
run_mutation M4 KILLED db/020_apply_remote_event.sql \
    '        PERFORM cairn_refuse_substitution(
            (SELECT content_address FROM event_log WHERE event_id = v_event_id),
            v_ca, v_event_id, '"'"'apply_remote_event'"'"');' \
    '        NULL; -- (mutation M4: guard deleted)' \
    "${NODE_TEST[@]}" restore_one_event_id_one_body

# M5 — the #614 count hard-wired to 0. Already run by hand during Task 4; re-run for the ledger.
run_mutation M5 KILLED crates/cairn-node/src/restore/clinical.rs \
    '    report.deferred = deferred_count(db).await?;' \
    '    report.deferred = 0;
    let _ = deferred_count(db).await?;' \
    "${NODE_TEST[@]}" restore_reports_deferred_records

# M6 — the remedy stripped from the notice. NOT `deferred >= 0` (always true for a usize, so
# -D warnings kills it at COMPILE time and it says nothing about runtime — #594's M9 lesson).
# This one is runtime-observable and aims at the assertion that exists to keep the command there.
run_mutation M6 KILLED crates/cairn-node/src/restore/clinical.rs \
    ' List them with `cairn-node deferred`.' \
    '' \
    "${NODE_TEST[@]}" restore_reports_deferred_records

# M7 — db/009's guard moved ABOVE the INSERT branch, where v_found is always NULL. A clean
# restore must then refuse, so this is caught by the IDEMPOTENCE arm, not the attack arm.
run_mutation M7 KILLED db/009_node_supersede_and_restore.sql \
    '    IF v_op = '"'"'enroll'"'"' THEN' \
    '    SELECT content_address INTO v_found FROM node_event WHERE node_event_id = v_eid;
    PERFORM cairn_refuse_substitution(v_found, v_ca, v_eid, '"'"'restore_node_event'"'"');
    IF v_op = '"'"'enroll'"'"' THEN' \
    "${NODE_TEST[@]}" restore_one_node_event_id_one_body

echo "=== run complete ==="
require_clean && echo "tree is clean: every revert landed"
