#!/usr/bin/env bash
# Mutation harness for #619 (ADR-0073). THROWAWAY — committed only so the run's ledger in
# docs/superpowers/plans/2026-09-19-node-plane-substitution-guard-619.md is reproducible.
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

fail() { echo "HARNESS ERROR: $*" >&2; exit 2; }

# Route around the IDE's rust-analyzer, which holds the default target/ lock, unless the caller
# already set one.
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-/tmp/cairn-619-target}"

read -r PGHOST PGPORT <<<"$(scripts/pg-target.sh)" || fail "no PostgreSQL target"
U=$(whoami)
export CAIRN_TEST_PG="host=$PGHOST port=$PGPORT dbname=cairn_test user=$U"
export CAIRN_TEST_PG2="host=$PGHOST port=$PGPORT dbname=cairn_test2 user=$U"
export CAIRN_TEST_PG3="host=$PGHOST port=$PGPORT dbname=cairn_test3 user=$U"

require_clean() {
    git diff --quiet || fail "tree is dirty before a mutation — refusing to start (this is the
        positive control; a dirty tree means a previous revert did not land)"
}

# KNOWN_IDS — every mutation id this harness defines. Validated against the CLI arguments
# immediately below, before require_clean or any mutation runs.
KNOWN_IDS=(M1 M2 M3 M4 M5 M6 M7 M8 M9 M10)

# want <id> — true when no ids were given on the command line (run everything) or when <id> is
# one of the requested ids (per-id selection: `2026-09-19-619.sh M3 M7` runs only those). Wraps
# each run_mutation call site below rather than living inside run_mutation, so run_mutation's own
# infrastructure (kept from the #614/#615 harness) stays untouched. Padded with spaces on both
# sides so "M1" never matches inside "M10".
ARGS_COUNT=$#
ARGS=" $* "
want() {
    [ "$ARGS_COUNT" -eq 0 ] && return 0
    case "$ARGS" in
        *" $1 "*) return 0 ;;
        *) return 1 ;;
    esac
}

# A typo'd id (e.g. "M9x") must not silently shrink the run to whatever subset happened to
# match — that is indistinguishable from a correct, deliberate subset run and still exits 0.
# So every argument is checked against KNOWN_IDS right here, before require_clean touches
# anything or any mutation runs, and every bad one is named in one loud, non-zero-exit failure.
if [ "$ARGS_COUNT" -gt 0 ]; then
    bad=()
    for a in "$@"; do
        known=0
        for k in "${KNOWN_IDS[@]}"; do
            [ "$a" = "$k" ] && known=1 && break
        done
        [ "$known" -eq 0 ] && bad+=("$a")
    done
    [ "${#bad[@]}" -eq 0 ] || fail "unknown mutation id(s): ${bad[*]} — known ids are: ${KNOWN_IDS[*]}"
fi

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

    # Reverse-direction half of the positive control. Incident: M9's first `to` was a bare
    # `    RETURN v_eid;`, which already occurred twice elsewhere in db/007 — after the forward
    # swap it occurred THREE times, so the revert's `swap "$to" "$from"` refused as ambiguous and
    # the mutation was left applied on what the harness still called a clean tree. `swap`'s own
    # exactly-once check only ever looks at the FROM direction, so it cannot catch this. Refuse
    # here, before anything is touched, unless `to` is currently absent — the only way its
    # post-swap count is guaranteed to be exactly one, which is what the revert needs.
    local to_before
    to_before=$(python3 - "$file" "$to" <<'PY'
import sys
print(open(sys.argv[1]).read().count(sys.argv[2]))
PY
)
    [ "$to_before" = "0" ] || fail "$id: replacement text already occurs $to_before time(s) in
        $file — the revert would be ambiguous after the forward swap (this is M9's incident,
        guarded against structurally): ${to:0:60}..."

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
    # A COMPILE failure says "could not compile" or carries an error code (error[E0433]).
    # Cargo prints "error: test failed, to rerun pass ..." for an ordinary RUNTIME failure, so
    # matching a bare leading "error:" would misreport every runtime kill as a compiler one —
    # which is exactly what the first run of this harness did.
    if echo "$out" | grep -qE "could not compile|^error\[E[0-9]+\]"; then
        verdict="KILLED (compiler — says nothing about runtime)"
    fi
    printf '%-4s expected %-9s actual %s\n' "$id" "$expected" "$verdict"
    [ "$verdict" != "${expected}" ] && echo "     ^ DIVERGENCE — investigate; see the plan's ledger"
    # Kill evidence: the first panicked-at line and the line after it, indented, so the ledger
    # can confirm the kill failed at the assertion that names its claim.
    if [ "$verdict" = "KILLED" ]; then
        echo "$out" | grep -m1 -A1 "panicked at" | sed 's/^/       /'
    fi
    return 0
}

echo "=== #619 mutation run — $(date -u +%FT%TZ) ==="
require_clean

NODE_TEST=(cargo test -p cairn-node --test)

# M1 — the local door's guard deleted. The local rival tests must catch it.
if want M1; then
run_mutation M1 KILLED db/007_node_federation.sql \
    "    PERFORM cairn_refuse_substitution(v_found, v_ca, v_eid, 'submit_node_event');" \
    '    -- (mutation M1: guard deleted)' \
    "${NODE_TEST[@]}" node_plane_one_event_id_one_body
fi

# M2 — the admission gate's guard deleted. The remote rival tests must catch it.
if want M2; then
run_mutation M2 KILLED db/007_node_federation.sql \
    "    PERFORM cairn_refuse_substitution(v_found, v_ca, v_eid, 'apply_remote_node_event');" \
    '    -- (mutation M2: guard deleted)' \
    "${NODE_TEST[@]}" node_plane_one_event_id_one_body
fi

# M3 — M1's deletion again, measured against the CATALOGUE rule alone: the inventory must catch a
# door that stopped calling the helper without any behaviour test's help.
if want M3; then
run_mutation M3 KILLED db/007_node_federation.sql \
    "    PERFORM cairn_refuse_substitution(v_found, v_ca, v_eid, 'submit_node_event');" \
    '    -- (mutation M3: guard deleted)' \
    "${NODE_TEST[@]}" substitution_guard_covers_every_writer
fi

# M4 — the admission gate's guard hoisted ABOVE its IF/ELSE, where nothing is held yet: every
# clean apply is refused. The IDEMPOTENCE case must catch it (the rival cases would still pass).
if want M4; then
run_mutation M4 KILLED db/007_node_federation.sql \
    "    IF v_op = 'enroll' THEN
        -- The genesis must match an active, out-of-band-confirmed peer: its" \
    "    SELECT content_address INTO v_found FROM node_event WHERE node_event_id = v_eid;
    PERFORM cairn_refuse_substitution(v_found, v_ca, v_eid, 'apply_remote_node_event');
    IF v_op = 'enroll' THEN
        -- The genesis must match an active, out-of-band-confirmed peer: its" \
    "${NODE_TEST[@]}" node_plane_one_event_id_one_body
fi

# M5 — the local door's guard hoisted above its IF/ELSE.
if want M5; then
run_mutation M5 KILLED db/007_node_federation.sql \
    "    IF v_op = 'supersede' THEN
        IF v_payload ->> 'superseded_node_id_hex' IS NULL THEN
            RAISE EXCEPTION 'submit_node_event:" \
    "    SELECT content_address INTO v_found FROM node_event WHERE node_event_id = v_eid;
    PERFORM cairn_refuse_substitution(v_found, v_ca, v_eid, 'submit_node_event');
    IF v_op = 'supersede' THEN
        IF v_payload ->> 'superseded_node_id_hex' IS NULL THEN
            RAISE EXCEPTION 'submit_node_event:" \
    "${NODE_TEST[@]}" node_plane_one_event_id_one_body
fi

# M6 — the decision inverted: an idempotent re-offer becomes a substitution, a rival does not.
if want M6; then
run_mutation M6 KILLED crates/cairn-node/src/sync/substitution.rs \
    '    if held == offered {' \
    '    if held != offered {' \
    cargo test -p cairn-node --lib sync::substitution
fi

# M7 — the puller never asks: every P0001 is skipped again (#619's pull-path half undone). Written
# as `.filter(|_| false)` so every binding stays used and the COMPILER cannot be what kills it.
if want M7; then
run_mutation M7 KILLED crates/cairn-node/src/sync.rs \
    '                        &offered,
                    ) {' \
    '                        &offered,
                    ).filter(|_| false) {' \
    "${NODE_TEST[@]}" node_substitution_is_penned
fi

# M8 — the lookup blinded: nothing is ever held, so no substitution is ever found.
if want M8; then
run_mutation M8 KILLED crates/cairn-node/src/sync/substitution.rs \
    '    Ok(row.map(|r| r.get(0)))' \
    '    Ok(row.map(|r| r.get(0)).filter(|_: &Vec<u8>| false))' \
    "${NODE_TEST[@]}" node_substitution_is_penned
fi

# M9 — the one shared clock merge deleted. The moved pin (3 → 1) must be live. `to` carries a
# distinguishing comment (not a bare `RETURN v_eid;`, which already occurs twice elsewhere in
# this file and made the revert ambiguous — the incident the reverse-direction guard above now
# catches structurally).
if want M9; then
run_mutation M9 KILLED db/007_node_federation.sql \
    "    PERFORM cairn_node_hlc_merge((b -> 'hlc' ->> 'wall')::bigint,
                                 (b -> 'hlc' ->> 'counter')::int);
    RETURN v_eid;" \
    '    -- (mutation M9: shared clock merge deleted)
    RETURN v_eid;' \
    "${NODE_TEST[@]}" hlc_merge_helper
fi

# M10 — the lookup-failure FREEZE turned into a SKIP (the arm yields `None`, so the event is
# skipped-and-advanced instead of freezing the cursor). EXPECTED SURVIVOR, declared before the run:
# no test can make `held_content_address` fail inside the self-pull (a lock BLOCKS rather than
# fails, and the owner role bypasses grants). Bounded: a skipped substitution is re-offered on the
# next full sweep and penned then. Recorded so the gap is a stated residual, not a silent one.
if want M10; then
run_mutation M10 SURVIVED crates/cairn-node/src/sync.rs \
    '                            stats.frozen = Some(seq);
                            break;
                        }
                    };' \
    '                            None
                        }
                    };' \
    "${NODE_TEST[@]}" node_substitution_is_penned
fi

echo "=== run complete ==="
require_clean && echo "tree is clean: every revert landed"
