#!/usr/bin/env bash
# scripts/pg-target.sh — resolve the PostgreSQL cluster the DB-backed test rigs should use,
# and REFUSE one that cannot run this schema.
#
# WHY THIS EXISTS (2026-09-07). `scripts/run-db-sql-tests.sh` connected through the plain
# libpq defaults. On a machine with more than one cluster that is whichever one the default
# socket happens to reach — and on the maintainer's Mac that is a LEGACY PostgreSQL 16
# instance, not the PG18 rig. Nothing checked the server version, so the run did not say
# "wrong server": it failed 22 migrations deep, first on a stale extension and then, once
# that was updated, on `max(bytea)` — an aggregate PostgreSQL only gained in 17. Neither
# error names the actual problem. The session that hit it concluded the extension was stale
# on the rig (it was not; the rig was current the whole time) and wrote that into a pull
# request body before catching it.
#
# So this script answers two questions the callers used to assume:
#
#   1. WHICH cluster? Never a hardcoded port — `run-db-gated-tests.sh` used to default to
#      5532, which is true of exactly one machine in the world.
#   2. Is it new enough? `db/001_envelope.sql` states the floor; below it the schema cannot
#      load, and it must fail HERE, before a database is created, with the version in the
#      message.
#
# CONTRACT. On success prints one line, `<host> <port>`, to stdout and exits 0. On failure
# prints a diagnosis to stderr, nothing to stdout, and exits 1. Callers:
#
#     target="$(scripts/pg-target.sh)" || exit 1
#     read -r PGHOST PGPORT <<<"$target"
#
# EXPLICIT ALWAYS WINS. If PGPORT is set, that cluster is the only candidate and it is still
# floor-checked — an operator who names a server gets that server or a clear refusal, never
# a different one chosen quietly. CI sets PGPORT, so CI's behaviour is unchanged by the
# discovery below.
set -uo pipefail

# The schema's PostgreSQL floor. SECOND HOME of a number whose source of truth is the
# `-- Target: PostgreSQL >= NN` line in `db/001_envelope.sql` (a shell script cannot read a
# SQL comment reliably enough to depend on it at runtime). A repeated constant that nothing
# checks is how the twin-registry row counts drifted apart in #182, so the two are bound by
# `scripts/tests/pg_target_test.sh`, which fails if they ever disagree.
CAIRN_PG_FLOOR_MAJOR=18

# How long to wait on a candidate before calling it unreachable. Discovery probes clusters
# that may not be running; without a bound, one dead socket stalls the whole gate.
CAIRN_PG_PROBE_TIMEOUT="${CAIRN_PG_PROBE_TIMEOUT:-5}"

# ---------------------------------------------------------------------------
# The decision — PURE. Reads `<host> <port> <version>` triples on stdin, where version is a
# `server_version_num` (180001, 160013, …) or the literal `unreachable`. Writes the chosen
# `<host> <port>` to stdout, or a diagnosis to stderr.
#
# Split out and kept free of psql so the whole verdict policy is testable with NO PostgreSQL
# of any kind. That is not a convenience: a guard whose tests need the substrate it guards
# cannot be trusted to fail correctly when that substrate is the thing that is wrong — which
# is the only situation this guard exists for.
# ---------------------------------------------------------------------------
pg_target_decide() {
    local qualifying=() too_old=() unreachable=()
    local host port version major seen

    while read -r host port version; do
        [ -z "${host:-}" ] && continue

        if [ "$version" = "unreachable" ]; then
            unreachable+=("$host:$port")
            continue
        fi

        # `server_version_num` is MMmmpp: 180001 -> 18, 160013 -> 16.
        major=$(( version / 10000 ))
        if [ "$major" -lt "$CAIRN_PG_FLOOR_MAJOR" ]; then
            too_old+=("$host:$port (PostgreSQL $major)")
            continue
        fi

        # Discovery can legitimately find one cluster twice — as a running postmaster AND as
        # whatever the libpq default resolves to. Calling that "ambiguous" would refuse a rig
        # that is not ambiguous at all, so identical targets collapse.
        seen=0
        for existing in ${qualifying[@]+"${qualifying[@]}"}; do
            [ "$existing" = "$host $port" ] && seen=1 && break
        done
        [ "$seen" -eq 0 ] && qualifying+=("$host $port")
    done

    if [ "${#qualifying[@]}" -eq 1 ]; then
        printf '%s\n' "${qualifying[0]}"
        return 0
    fi

    if [ "${#qualifying[@]}" -gt 1 ]; then
        {
            echo "More than one PostgreSQL >= $CAIRN_PG_FLOOR_MAJOR cluster is reachable, and this"
            echo "script DROPS AND RECREATES a database on the one it picks — so it will not guess:"
            for q in "${qualifying[@]}"; do
                echo "  - ${q// /:}"
            done
            echo "Name the one you mean, e.g.  PGPORT=${qualifying[0]##* } scripts/run-db-sql-tests.sh"
        } >&2
        return 1
    fi

    # Nothing qualified. Report EVERY rejected candidate with the reason, because "too old"
    # and "not running" send an operator to opposite remedies and a message that merges them
    # sends half of them to the wrong one.
    {
        echo "No PostgreSQL >= $CAIRN_PG_FLOOR_MAJOR cluster could be reached."
        echo "The schema needs it: db/001_envelope.sql targets PostgreSQL >= $CAIRN_PG_FLOOR_MAJOR,"
        echo "and below that the load fails deep in the migration chain on a missing function"
        echo "(db/048 needs max(bytea), a PostgreSQL 17 aggregate) rather than on this line."
        if [ "${#too_old[@]}" -gt 0 ]; then
            echo
            echo "TOO OLD — reachable, but below the floor. Point at a newer cluster; upgrading"
            echo "these in place is not something this script should do for you:"
            for t in "${too_old[@]}"; do
                echo "  - $t"
            done
        fi
        if [ "${#unreachable[@]}" -gt 0 ]; then
            echo
            echo "UNREACHABLE — nothing answered. A different remedy: start the server, or"
            echo "correct the host/port. This is NOT the same as too old:"
            for u in "${unreachable[@]}"; do
                echo "  - $u"
            done
        fi
        echo
        echo "Set PGHOST/PGPORT to name the cluster explicitly if it was not discovered."
    } >&2
    return 1
}

# ---------------------------------------------------------------------------
# Probing — impure. Ask one candidate its version, or report it unreachable.
# ---------------------------------------------------------------------------
pg_target_probe() {
    local host="$1" port="$2" version
    # `-tA` so the answer is the bare number. Errors are discarded on purpose: the DISTINCTION
    # this script draws is answered/not-answered, and psql's connection diagnostics would
    # otherwise be interleaved into the candidate stream.
    version="$(PGHOST="$host" PGPORT="$port" PGCONNECT_TIMEOUT="$CAIRN_PG_PROBE_TIMEOUT" \
        psql -d postgres -tAc 'SHOW server_version_num' 2>/dev/null | tr -d '[:space:]')"
    if [ -z "$version" ]; then
        printf '%s %s unreachable\n' "$host" "$port"
    else
        printf '%s %s %s\n' "$host" "$port" "$version"
    fi
}

# ---------------------------------------------------------------------------
# Command-line parsing — PURE, and separate because getting it wrong is silent. Discovery
# reads the process table, and a postmaster's arguments are UNQUOTED there, so a data
# directory containing spaces (`~/Library/Application Support/...` — the Postgres.app
# default on macOS) cannot be recovered by stopping at the first space. The first version of
# this did exactly that, produced `/Users/h/Library/Application`, found no pid file and
# dropped the ONLY qualifying cluster while reporting that none existed.
# ---------------------------------------------------------------------------

# Which `ps` lines are postmasters. Two conditions, deliberately kept as two greps rather
# than folded into one clever expression: the first says "this invoked a postgres BINARY"
# (excluding `postgres: io worker 0` and the Postgres.app GUI helpers, which are not
# postmasters), the second says "it carries a -D flag".
#
# They must stay separate. Folding them into `postgres[[:space:]].*[[:space:]]-D` demands a
# space BEFORE `-D` that does not exist when `-D` is the first argument — which is precisely
# how Postgres.app launches, so that version selected nothing at all and let the resolver
# fall through to its default while appearing to have searched.
pg_target_postmaster_lines() {
    grep -E '(^|/)postgres(\.exe)?[[:space:]]' | grep -E '[[:space:]]-D[[:space:]]'
}

# The port a postmaster was started with, when it was started with one. Preferred over the
# pid file because it needs no filesystem access — but not universal: Debian/Ubuntu
# packaging leaves the port to a config file, which is why the datadir fallback exists.
pg_target_port_from_args() {
    printf '%s\n' "$1" | sed -nE 's/.*[[:space:]]-p[[:space:]]+([0-9]+).*/\1/p'
}

# The data directory, spaces and all. Everything after `-D ` up to the next ` -flag` or the
# end of the line — the only reading that survives an unquoted path with spaces in it.
pg_target_datadir_from_args() {
    printf '%s\n' "$1" | sed -nE 's/.*[[:space:]]-D[[:space:]]+(.*)/\1/p' | sed -E 's/[[:space:]]+-[[:alpha:]].*$//'
}

# ---------------------------------------------------------------------------
# Discovery — which clusters might there be? Never a hardcoded port list: the ports come
# from postmasters ACTUALLY RUNNING on this machine — from `-p` when they carry it, and
# otherwise from each data directory's `postmaster.pid` (line 4 is the port, a stable part
# of PostgreSQL's on-disk format across every supported version). That works the same on
# macOS and Linux and assumes nothing about how PostgreSQL was installed.
# ---------------------------------------------------------------------------
pg_target_candidates() {
    local host="${PGHOST:-127.0.0.1}"

    # EXPLICIT WINS. A named port is the only candidate — still floor-checked, never
    # silently swapped for another cluster.
    if [ -n "${PGPORT:-}" ]; then
        pg_target_probe "$host" "$PGPORT"
        return
    fi

    # Every running postmaster, from the process table. `-D <dir>` is how every supported
    # PostgreSQL is started, by every packaging we care about, so it identifies the lines.
    local postmasters
    postmasters="$(ps -axo args= 2>/dev/null | pg_target_postmaster_lines | sort -u)"

    local found=0 port dir pidfile
    local -a ports=()
    while IFS= read -r line; do
        [ -z "$line" ] && continue
        port="$(pg_target_port_from_args "$line")"
        if [ -z "$port" ]; then
            dir="$(pg_target_datadir_from_args "$line")"
            pidfile="$dir/postmaster.pid"
            [ -r "$pidfile" ] || continue
            port="$(sed -n '4p' "$pidfile" | tr -d '[:space:]')"
        fi
        case "$port" in
            ''|*[!0-9]*) continue ;;
        esac
        # One cluster can show several postmaster-shaped lines; probing each would be slow
        # and would report the same target repeatedly.
        local dup=0
        for p in ${ports[@]+"${ports[@]}"}; do
            [ "$p" = "$port" ] && dup=1 && break
        done
        [ "$dup" -eq 1 ] && continue
        ports+=("$port")
        pg_target_probe "$host" "$port"
        found=1
    done <<< "$postmasters"

    # Nothing discoverable — fall back to whatever libpq itself would do with no override,
    # so a remote server named purely by PGHOST, or a socket-only rig, still resolves. Its
    # port is reported as libpq's own default so the message can name something concrete.
    if [ "$found" -eq 0 ]; then
        pg_target_probe "$host" "${PGPORT:-5432}"
    fi
}

pg_target_main() {
    if [ "$#" -ne 0 ]; then
        echo "pg-target.sh takes no arguments (it reads PGHOST/PGPORT)" >&2
        return 2
    fi
    pg_target_candidates | pg_target_decide
}

# Only run when EXECUTED, not when sourced — `scripts/tests/pg_target_test.sh` sources this
# file to exercise `pg_target_decide` directly, and sourcing must not probe anything.
if [ "${BASH_SOURCE[0]}" = "$0" ]; then
    pg_target_main "$@"
fi
