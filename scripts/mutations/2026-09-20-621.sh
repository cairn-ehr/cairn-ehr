#!/usr/bin/env bash
# Mutation harness for #621 (ADR-0074). THROWAWAY — committed only so the run's ledger in
# docs/superpowers/plans/2026-09-20-node-pull-deterministic-refusal-621.md is reproducible.
#
# Adapted from scripts/mutations/2026-09-19-619.sh — its header below records why the harness has
# this shape, and nothing in that infrastructure changed except the RAN counter at the end, which
# this slice added after a mis-assembled copy ran ZERO mutations and still reported a clean tree.
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
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-/tmp/cairn-621-target}"

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
KNOWN_IDS=(M1 M2 M3 M4 M5 M6 M7 M8 M9 M10 M11 M12 M13)

# want <id> — true when no ids were given on the command line (run everything) or when <id> is
# one of the requested ids (per-id selection: `2026-09-20-621.sh M3 M7` runs only those). Wraps
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

# How many mutations actually RAN. A harness that executes none and then prints "tree is
# clean: every revert landed" is telling the truth about a run that never happened — which is
# exactly what a mis-assembled copy of this script did once. The tail refuses that silence.
RAN=0

# run_mutation <id> <expected> <file> <from> <to> <test-cmd...>
run_mutation() {
    RAN=$((RAN + 1))
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

echo "=== #621 mutation run — $(date -u +%FT%TZ) ==="
require_clean

NODE_TEST=(cargo test -p cairn-node --test)

# --- The doors (db/001, db/007, db/009): every malformed field must raise P0001. ---

# M1 — the admission gate's event_id guard reverts to the bare cast it replaced. This is THE
# defect #621 reported, reintroduced in its exact original shape.
if want M1; then
run_mutation M1 KILLED db/007_node_federation.sql \
    "    v_eid := cairn_uuid_or_raise('event_id', b ->> 'event_id', 'apply_remote_node_event');" \
    "    v_eid := (b ->> 'event_id')::uuid; -- (mutation M1)" \
    "${NODE_TEST[@]}" node_door_refusals_are_p0001
fi

# M2 — the same reversion, measured against the CATALOGUE guards ALONE: a door that stopped
# calling the helper must be caught with no behavioural test's help, because the next door will
# be added by someone who never opens the behaviour suite.
if want M2; then
run_mutation M2 KILLED db/007_node_federation.sql \
    "    v_eid := cairn_uuid_or_raise('event_id', b ->> 'event_id', 'apply_remote_node_event');" \
    "    v_eid := (b ->> 'event_id')::uuid; -- (mutation M2)" \
    "${NODE_TEST[@]}" node_door_input_guards
fi

# M3 — the LOCAL door's event_id guard reverts. The doors are guarded one by one, never as a set.
if want M3; then
run_mutation M3 KILLED db/007_node_federation.sql \
    "    v_eid    := cairn_uuid_or_raise('event_id', b ->> 'event_id', 'submit_node_event');" \
    "    v_eid    := (b ->> 'event_id')::uuid; -- (mutation M3)" \
    "${NODE_TEST[@]}" node_door_refusals_are_p0001
fi

# M4 — the RESTORE door's event_id guard reverts (db/009 is the third door, and the one a reader
# is most likely to forget: it aborts the whole restore either way, so only legibility changes).
if want M4; then
run_mutation M4 KILLED db/009_node_supersede_and_restore.sql \
    "    v_eid := cairn_uuid_or_raise('event_id', b ->> 'event_id', 'restore_node_event');" \
    "    v_eid := (b ->> 'event_id')::uuid; -- (mutation M4)" \
    "${NODE_TEST[@]}" node_door_refusals_are_p0001
fi

# M5 — the admission gate's clock guard deleted, so a negative HLC reaches node_event_hlc_nonneg
# and raises 23514 again.
if want M5; then
run_mutation M5 KILLED db/007_node_federation.sql \
    "    PERFORM cairn_hlc_nonneg_or_raise((b -> 'hlc' ->> 'wall')::bigint,
                                      (b -> 'hlc' ->> 'counter')::int, 'apply_remote_node_event');" \
    "    -- (mutation M5: clock guard deleted)" \
    "${NODE_TEST[@]}" node_door_refusals_are_p0001
fi

# M6 — the clock guard checks the WALL only. The half-guard is the plausible version of this
# code, and only the negative-COUNTER case can tell it from the whole one.
if want M6; then
run_mutation M6 KILLED db/001_envelope.sql \
    "    IF p_wall < 0 OR p_counter < 0 THEN" \
    "    IF p_wall < 0 THEN -- (mutation M6: counter unguarded)" \
    "${NODE_TEST[@]}" node_door_refusals_are_p0001 \
    a_negative_hlc_counter_is_refused_with_the_skip_and_advance_code
fi

# M7 — the role guard deleted from the admission gate, so an unknown role reaches the CHECK.
if want M7; then
run_mutation M7 KILLED db/007_node_federation.sql \
    "                cairn_node_role_or_raise(v_payload ->> 'role', 'apply_remote_node_event')," \
    "                v_payload ->> 'role', -- (mutation M7)" \
    "${NODE_TEST[@]}" node_door_refusals_are_p0001
fi

# M8 — the target_event_id guard reverts to its bare cast on the admission gate.
if want M8; then
run_mutation M8 KILLED db/007_node_federation.sql \
    "                CASE WHEN NULLIF(v_payload ->> 'target_event_id','') IS NULL THEN NULL
                     ELSE cairn_uuid_or_raise('target_event_id',
                            v_payload ->> 'target_event_id', 'apply_remote_node_event') END," \
    "                NULLIF(v_payload ->> 'target_event_id','')::uuid, -- (mutation M8)" \
    "${NODE_TEST[@]}" node_door_refusals_are_p0001
fi

# M9 — the UUID validator becomes NARROWER than the cast it replaces (canonical spellings only).
# Every refusal test still passes; only the positive control can see it, and what it would cost
# in production is the mirror of PR #623's finding 1: events the log can already hold, refused.
if want M9; then
run_mutation M9 KILLED db/001_envelope.sql \
    "    IF NOT pg_input_is_valid(p_value, 'uuid') THEN" \
    "    IF p_value !~ '^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}\$' THEN -- (mutation M9)" \
    "${NODE_TEST[@]}" node_door_refusals_are_p0001 \
    a_well_formed_event_still_applies_however_its_id_is_spelled
fi

# M10 — the role vocabulary re-inlined into the CHECK constraint. Behaviour is identical TODAY;
# what is lost is the single source, and with it the guarantee that widening the vocabulary
# cannot leave the door and the floor disagreeing.
if want M10; then
run_mutation M10 KILLED db/007_node_federation.sql \
    "    CHECK (role IS NULL OR role = ANY (cairn_node_roles()));" \
    "    CHECK (role IS NULL OR role IN ('upstream','downstream','peer')); -- (mutation M10)" \
    "${NODE_TEST[@]}" node_door_input_guards
fi

# --- The puller (cairn-node/src/sync.rs): which failures pen, which freeze. ---

# M11 — a dropped connection (no SQLSTATE) starts penning. A pen row would then claim the door
# refused bytes it never saw.
if want M11; then
run_mutation M11 KILLED crates/cairn-node/src/sync.rs \
    "        None => false," \
    "        None => true, // (mutation M11)" \
    "${NODE_TEST[@]}" node_pull_refusal_class
fi

# M12 — the 22 class (a cast on a peer-supplied field) is claimed as LOCAL, which is #621's own
# defect stated in Rust rather than in SQL.
if want M12; then
run_mutation M12 KILLED crates/cairn-node/src/sync.rs \
    '                "08"    // connection_exception' \
    '                "22" | "08"    // (mutation M12)' \
    "${NODE_TEST[@]}" node_pull_refusal_class
fi

# M13 — the new arm stops freezing when its pen could not be written, and advances instead. The
# refusal is then lost until the next full sweep with nothing holding it (the #111 review's A1).
if want M13; then
run_mutation M13 KILLED crates/cairn-node/src/sync.rs \
    "                        PenOutcome::Frozen => {
                            stats.frozen = Some(seq);
                            break;
                        }
                    }
                }
                // Any OTHER error on a verifiable event is THIS NODE'S OWN trouble:" \
    "                        PenOutcome::Frozen => {} // (mutation M13)
                    }
                }
                // Any OTHER error on a verifiable event is THIS NODE'S OWN trouble:" \
    "${NODE_TEST[@]}" node_pull_deterministic_refusal -- --test-threads=1
fi

echo "=== run complete: $RAN mutation(s) ran ==="
EXPECTED_RUNS=$ARGS_COUNT
[ "$ARGS_COUNT" -eq 0 ] && EXPECTED_RUNS=${#KNOWN_IDS[@]}
[ "$RAN" -eq "$EXPECTED_RUNS" ] || fail "expected $EXPECTED_RUNS mutation(s) to run, but $RAN did —
    a run that executes nothing and reports a clean tree is a lie about work never done"
require_clean && echo "tree is clean: every revert landed"
