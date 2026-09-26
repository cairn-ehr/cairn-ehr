#!/usr/bin/env python3
"""The §1.2 / §5.11 measurement rig for `db/046`'s patient search (#637, #639).

House rule 7 owes every clinical-surface slice a falsifiable paper-parity figure, and the
search-before-create funnel's is stated in `db/046` itself: **5 s to find an existing
chart**, on Pi-class hardware, at the ~50,000-patient population pinned in spec §8.1.
§5.11 adds a second, harder limb — *"type a few chars and enter, no spinner"* — which is a
claim about the FLOOR, not the worst case.

#636 slice 1 measured both by hand, on a Pi reached over ssh, with a script that was never
committed. This rig is that script, made repeatable: the numbers in the #637 and #639 plans
can be re-derived rather than believed, and the next person to touch pass 3 can find out
what they cost before a reviewer does.

# What it measures, and what it deliberately does not

**The read path only.** Rows are inserted straight into the `patient_name` projection rather
than authored as signed events, because `cairn_search_candidates` reads nothing else and
authoring 50,000 signed events on a Pi would measure the WRITE path, which is not what is
budgeted here. This is the same caveat slice 1 recorded, kept visible rather than buried:
the figures are honest about search and say nothing about registration throughput.

**Timing is server-side.** Each query is run through `cairn_search_candidates` and timed
around the round trip from this process, so the figure includes the network hop to the
cluster. Run it ON the target machine (or over a link you are willing to call negligible)
when the number is going into a budget.

# Names

Real name distributions matter for the token counts, not for the timings — slice 1 proved
that by re-running against 50,378 real Australian names (965,260 distinct surnames) and
finding every timing within ~5% of a synthetic fixture. So `--name-pool` takes an optional
SQLite database with a table of names; without it the rig generates a deterministic
synthetic pool, which is enough for a before/after comparison and is what CI-shaped runs
should use.

# Usage

    # before/after on one machine, against an already-loaded schema
    python3 scripts/measure_patient_search.py --dbname cairn_test --rows 50000

    # with the maintainer's synthetic-population pool
    python3 scripts/measure_patient_search.py --rows 50000 \\
        --name-pool ~/src/SyntheticHealthData/synthetic_demographics.sqlite3

    python3 scripts/measure_patient_search.py --self-test   # pure functions, no database

Requires `psql` on PATH and a cluster whose schema is already loaded (any `cairn-node`
DB-gated test run loads it; see `scripts/pg-target.sh` for discovery).
"""

from __future__ import annotations

import argparse
import statistics
import subprocess
import sys
import time
import unicodedata
from dataclasses import dataclass

# The §1.2 budget `db/046` states, in milliseconds. Not a flag: a budget a run can talk
# itself out of is not a budget.
BUDGET_MS = 5000.0

# §5.11's other limb. There is no stated number for "no spinner", so this rig uses the
# conventional UI threshold at which a wait stops reading as instant. It is reported, never
# enforced — the number a slice must beat is a maintainer's call, not this script's.
SPINNER_MS = 1000.0

# The five gestures slice 1 measured, so a re-run is comparable row for row.
DEFAULT_QUERIES = [
    "fyodorowksi-eschenbacher",
    "mich",
    "smi",
    "李小",
    "wu",
]


@dataclass(frozen=True)
class Timing:
    """One query's result: what was typed, how long it took, and what came back."""

    query: str
    median_ms: float
    rows: int

    @property
    def found_a_chart(self) -> bool:
        """Did this gesture actually find anything?

        §1.2's budget is **5 s to FIND AN EXISTING CHART**, so a gesture that matched nothing
        did not measure the budget — it measured how fast pass 3 can scan and come back
        empty. That is a real number and a different one, and it must never be mistaken for
        the other.
        """
        return self.rows > 0

    @property
    def within_budget(self) -> bool:
        """Whether this gesture met §1.2 — which a gesture that found nothing cannot.

        The `found_a_chart` conjunct is the fix for a silent failure this rig shipped with:
        the verdict consulted `median_ms` alone, so a query matching zero rows returned in a
        few milliseconds, passed, AND became the `floor` figure quoted against §5.11's "no
        spinner" limb. The fastest row in the table was the one that did the least work.
        """
        return self.found_a_chart and self.median_ms <= BUDGET_MS


def median_ms(samples: list[float]) -> float:
    """Median of a sample list, in milliseconds, rounded to whole milliseconds.

    The median rather than the mean: a single scheduler hiccup on a 4-core Pi moves a mean
    and does not move a median, and the question being asked is what the clerk usually
    waits, not what the machine can be provoked into.
    """
    if not samples:
        raise ValueError("no samples to take a median of")
    return round(statistics.median(samples), 1)


def synthetic_names(count: int) -> list[str]:
    """A deterministic name pool, shaped to exercise every pass-3 path.

    Not a realistic population, and not trying to be — slice 1 established that timings are
    insensitive to name distribution. What it IS trying to be is exhaustive about SHAPES, so
    a change that only slows one path down still shows up: long compounds, short surnames,
    CJK tokens, and plain two-word Latin names, in fixed proportion.
    """
    shapes = [
        "Fyodorowksi-Eschenbacher Michael",
        "Smith John",
        "Wu Ling",
        "李小明",
        "Michaels Anne",
        "O'Brien-Smith Patricia",
    ]
    return [f"{shapes[i % len(shapes)]}{i}" for i in range(count)]


def pool_names(sqlite_path: str, count: int, spread: bool = False) -> list[str]:
    """Draw `count` names from a SQLite pool of real names.

    `spread=False` (this rig's default, kept so its published latency figures stay comparable —
    close, not exact, since blank names are now skipped) takes the FIRST `count` usable rows. That head is not representative of the pool: in the
    maintainer's pool the first 50,000 rows hold ~4x the table's share of its commonest
    surnames (Smith 0.72% vs 0.18%, review of PR #678). For a latency rig that is a pessimistic,
    stable choice — more namesakes, bigger candidate sets. A rig whose figures are ABOUT the
    name distribution (`measure_prompt_truncation.py`) passes `spread=True`: every k-th usable
    row across the whole table, deterministic and representative. It numbers the usable rows
    with `row_number()` rather than reading `rowid`, which a VIEW or a WITHOUT ROWID table (both
    of which discovery can pick) does not have.

    A row with a blank (empty or whitespace-only) given name or surname is never drawn: it
    would become a one-word "name" and quietly model a case the rig does not claim to.

    The pool's shape is discovered rather than assumed: the first table holding columns that
    look like a given name and a surname is used. A pool that does not match is a loud error
    — silently falling back to synthetic names would put a "real distribution" label on a
    figure that has none, which is the error #637's first measurement made about hardware.
    """
    import re
    import sqlite3

    def plain_identifier(name: str) -> str:
        """Refuse anything that is not a bare identifier before it reaches a query.

        The table and column names below are DISCOVERED from the pool file rather than fixed, so
        they are interpolated (SQLite takes no bind parameter for an identifier). The pool is the
        maintainer's own data file, not input — but a rig that only works on trusted input is one
        nobody can point at an unfamiliar pool, and the check is one line.
        """
        if not re.fullmatch(r"[A-Za-z_][A-Za-z0-9_]*", name):
            raise SystemExit(f"{sqlite_path}: refusing to query non-identifier name {name!r}")
        return name

    con = sqlite3.connect(sqlite_path)
    try:
        tables = [
            r[0]
            for r in con.execute(
                "SELECT name FROM sqlite_master WHERE type IN ('table','view')"
            )
        ]
        for table in tables:
            # SKIP a table this rig cannot safely name, rather than aborting the run on it.
            # `plain_identifier` raising here used to kill the whole measurement on the FIRST
            # oddly-named object in the file — before the loop ever reached the table holding
            # the names — which is the opposite of the docstring's stated intent.
            if not re.fullmatch(r"[A-Za-z_][A-Za-z0-9_]*", table):
                continue
            cols = [r[1].lower() for r in con.execute(f'PRAGMA table_info("{table}")')]
            given = next((c for c in cols if c in ("firstname", "given_name", "given", "first_name")), None)
            family = next((c for c in cols if c in ("surname", "family_name", "last_name", "lastname")), None)
            if given and family:
                table, given, family = (
                    plain_identifier(table),
                    plain_identifier(given),
                    plain_identifier(family),
                )
                usable = (
                    f'"{family}" IS NOT NULL AND trim("{family}") <> \'\' '
                    f'AND "{given}" IS NOT NULL AND trim("{given}") <> \'\''
                )
                stride = 1
                if spread:
                    # Every `stride`-th USABLE row: `available // count` numbered rows at a
                    # stride of `stride` yield at least `count`, so the draw cannot come up short.
                    (available,) = con.execute(
                        f'SELECT count(*) FROM "{table}" WHERE {usable}'
                    ).fetchone()
                    stride = max(1, available // count)
                    rows = con.execute(
                        f'SELECT g, f FROM (SELECT "{given}" AS g, "{family}" AS f, '
                        f"row_number() OVER () AS rn FROM \"{table}\" WHERE {usable}) "
                        f"WHERE (rn - 1) % ? = 0 LIMIT ?",
                        (stride, count),
                    ).fetchall()
                else:
                    rows = con.execute(
                        f'SELECT "{given}", "{family}" FROM "{table}" WHERE {usable} LIMIT ?',
                        (count,),
                    ).fetchall()
                # SAY WHICH TABLE WON. The schema is DISCOVERED, and more than one table can
                # satisfy the heuristic: the maintainer's own pool has `names` (6.5M rows) and
                # `person` (10 rows), both matching, resolved only by `sqlite_master` order.
                # A discovered-schema heuristic that does not report its choice is
                # undebuggable six months out.
                print(
                    f"pool: {sqlite_path} table {table!r} "
                    f"columns ({given}, {family}) -> {len(rows)} names, "
                    + (f"one usable row in every {stride}" if spread else "the first usable rows"),
                    flush=True,
                )
                # THE SHORT-DRAW GUARD, which `file_names` below has always had and this path
                # did not. `LIMIT ?` returns fewer rows than asked for without complaint, so
                # pointing the rig at a small or decoy table used to seed that many charts,
                # time five searches against them, print sub-50 ms figures and exit 0 — with
                # the table header still claiming the §1.2 budget was met at 50,000 rows.
                if len(rows) < count:
                    raise SystemExit(
                        f"{sqlite_path}: table {table!r} yielded {len(rows)} usable names, "
                        f"need {count}. Measuring a smaller population and labelling it "
                        f"{count} is the failure this rig exists to stop."
                    )
                return [f"{g} {f}".strip() for g, f in rows]
        raise SystemExit(
            f"{sqlite_path}: no table with a recognisable given-name + surname pair; "
            f"tables seen: {', '.join(tables[:20])}"
        )
    finally:
        con.close()


def file_names(path: str, count: int) -> list[str]:
    """Read names from a plain text file, one per line.

    This exists for the measurement that matters: the budget is stated for Pi-class hardware,
    the realistic name pool is a gigabyte of SQLite on the maintainer's workstation, and
    copying the pool to a Pi to draw 50,000 rows from it is a waste of an afternoon. Draw them
    where the pool lives, carry the text file, measure where the budget applies.
    """
    with open(path, encoding="utf-8") as fh:
        names = [line.strip() for line in fh if line.strip()]
    if len(names) < count:
        raise SystemExit(f"{path}: holds {len(names)} names, need {count}")
    return names[:count]


def seed_sql(names: list[str]) -> str:
    """The SQL that puts `names` into `patient_name`, one chart each.

    A single multi-row INSERT rather than one statement per name: 50,000 round trips to a Pi
    is minutes of measuring nothing. `uuidv7()` gives each row its own chart, which matters
    because pass 3 returns DISTINCT patient ids and a shared id would collapse the result
    set and flatter the timings.

    Returned as text rather than executed so `--self-test` can check the escaping without a
    database.
    """
    values = ",\n".join(
        "(uuidv7(), 'legal', {}, 'patient-stated', 1, 0, 0, 'measure', clock_timestamp())".format(
            quote_literal(unicodedata.normalize("NFC", n))
        )
        for n in names
    )
    return (
        "INSERT INTO patient_name (patient_id, use_key, value, provenance, provenance_rank, "
        "last_hlc_wall, last_hlc_count, asserted_origin, updated_at) VALUES\n" + values + ";"
    )


def quote_literal(value: str) -> str:
    """Postgres single-quoted literal. Doubling the quote is the whole rule."""
    return "'" + value.replace("'", "''") + "'"


def format_table(timings: list[Timing]) -> str:
    """The markdown table that goes into the plan's measurement section.

    The `Rows found` column is not decoration and must not be dropped when the table is
    pasted into a plan: it is the only thing distinguishing "this gesture met the budget"
    from "this gesture matched nothing and came back fast". A row that found nothing says so
    in the verdict column, in words, so a reader of the PLAIN TABLE cannot miss it even if
    they never see this file.
    """
    header = "| Search | Median | Rows found | Budget |\n|---|---|---|---|"

    def verdict(t: Timing) -> str:
        if not t.found_a_chart:
            return "**NO ROWS — not a measurement of finding a chart**"
        return "ok" if t.within_budget else "**OVER**"

    rows = [
        "| `{q}` | **{ms} ms** | {n} | {v} |".format(
            q=t.query, ms=t.median_ms, n=t.rows, v=verdict(t)
        )
        for t in timings
    ]

    # The floor is quoted against §5.11's "no spinner" limb, so it must be the cheapest
    # search that ACTUALLY FOUND A CHART. Taking the minimum over every timing let a
    # zero-row gesture — `李小` against a Latin-script name pool, in the run this rig was
    # written for — supply the headline figure for the harder of the two limbs.
    found = [t for t in timings if t.found_a_chart]
    if not found:
        note = (
            "\n\n⚠️ **No gesture in this set found a chart, so there is no floor to report.** "
            "Every §1.2 and §5.11 figure here would be a measurement of an empty search."
        )
    else:
        floor = min(t.median_ms for t in found)
        empty = len(timings) - len(found)
        caveat = (
            f" ({empty} gesture(s) found nothing and are excluded from the floor.)"
            if empty
            else ""
        )
        note = (
            f"\n\nFloor (cheapest search that found a chart): **{floor} ms** — "
            f"{'above' if floor > SPINNER_MS else 'within'} the ~{SPINNER_MS:.0f} ms "
            f"at which a wait stops reading as instant (§5.11).{caveat}"
        )
    return "\n".join([header, *rows]) + note


def psql(conn: list[str], sql: str) -> str:
    """Run one statement, returning stdout. Raises on a non-zero exit, loudly.

    `-X` (`--no-psqlrc`) is NOT optional here, and it is the whole reason this wrapper exists
    rather than an inline `subprocess.run`. Without it psql sources `~/.psqlrc` before every
    single `-c` — that is once per timed sample — and a psqlrc holding `SET work_mem`,
    `SET enable_seqscan = off` or `SET jit = off` silently re-plans the seq-scan-dominated
    query this rig exists to time. The result is a number that is entirely plausible and
    reproduces on no other machine. This rig's whole method is "same machine, same database,
    the function swapped in place between two runs"; a psqlrc is precisely the uncontrolled
    variable that method assumes away.
    """
    proc = subprocess.run(
        ["psql", "-X", *conn, "-v", "ON_ERROR_STOP=1", "-tAc", sql],
        capture_output=True,
        text=True,
    )
    if proc.returncode != 0:
        sys.stderr.write(proc.stderr)
        raise SystemExit(f"psql failed ({proc.returncode})")
    return proc.stdout.strip()


def scalar(conn: list[str], sql: str) -> str:
    """`psql` for a statement that MUST return exactly one value.

    Separate from `psql` because "the query succeeded and returned nothing" and "the query
    returned 0" are different facts that must not share a representation. The rig used to
    write `int(out or 0)`, which quietly turned the first into the second — and since a
    zero-row timing used to pass the budget and could become the reported floor, a count that
    was never actually read could have supplied the headline number.
    """
    out = psql(conn, sql)
    if not out:
        raise SystemExit(
            f"psql exited 0 but returned no output for: {sql}\n"
            "The value was never read, so nothing measured here can be trusted."
        )
    return out


def measure_sql(query: str) -> str:
    """The exact statement one sample times.

    Pulled out as a pure function so it can be asserted on without a cluster. A rig that
    silently times the WRONG query — a different function, a different pass, a scalar where
    the caller fetches rows — is the worst of the silent-wrong-measurement failures, because
    every number it produces is internally consistent.
    """
    return (
        "SELECT count(*) FROM cairn_search_candidates("
        f"ARRAY[{quote_literal(query)}]::text[], NULL, '[]'::jsonb)"
    )


def measure(conn: list[str], query: str, repeats: int) -> Timing:
    """Time one search, discarding a warm-up run.

    The warm-up is not politeness: the first call after a schema load pays for pages the
    buffer cache has not seen, and a clerk's second search of the shift is the one the budget
    is about.

    ⚠️ IT DOES NOT WARM A PLAN CACHE, and an earlier version of this docstring said it did.
    Every sample is a fresh `psql` process and therefore a fresh session, so no prepared plan
    survives any call: each sample re-parses and re-plans, warm-up included. What this rig
    measures is cold-plan, warm-buffer execution. The same fresh-process design also means
    every sample pays a process-spawn, connect and authentication constant, which `overhead`
    below measures rather than assumes.
    """
    sql = measure_sql(query)
    psql(conn, sql)  # warm-up, not recorded
    samples = []
    rows = 0
    for _ in range(repeats):
        start = time.perf_counter()
        out = scalar(conn, sql)
        samples.append((time.perf_counter() - start) * 1000.0)
        rows = int(out)
    return Timing(query=query, median_ms=median_ms(samples), rows=rows)


def count_population(conn: list[str]) -> str:
    """Rows in `patient_name` — the population every timing below is a measurement OF.

    `count(*)` over the whole table, deliberately, not over `asserted_origin = 'measure'`:
    the question is not "did my seed land" but "what is pass 3 actually scanning", and a row
    left behind by something else is scanned exactly like one of ours.
    """
    return scalar(conn, "SELECT count(*) FROM patient_name")


def overhead_ms(conn: list[str], repeats: int) -> float:
    """Per-sample cost that is not the query: fork, exec, connect, authenticate.

    Reported next to the timings rather than left in a plan's prose. Each sample above spawns
    a psql process and opens a new connection — on a 4-core Pi with SCRAM-SHA-256 that is
    thousands of PBKDF2 iterations before a byte of SQL is sent. It was measured by hand once,
    off-rig, at ~32 ms; since the post-#639 timings cluster within 14 ms of each other, a
    constant of that size is not a rounding error next to the finding, and it moves with the
    transport, the auth method and the machine. Measure it where the numbers are produced.
    """
    sql = "SELECT 1"
    psql(conn, sql)  # warm-up, same discipline as measure()
    samples = []
    for _ in range(repeats):
        start = time.perf_counter()
        scalar(conn, sql)
        samples.append((time.perf_counter() - start) * 1000.0)
    return median_ms(samples)


def self_test() -> int:
    """Exercise the pure functions. No database, no psql, no network."""
    assert median_ms([3.0, 1.0, 2.0]) == 2.0
    assert median_ms([1.0, 2.0]) == 1.5
    try:
        median_ms([])
    except ValueError:
        pass
    else:  # pragma: no cover - the assertion above is the test
        raise AssertionError("an empty sample list must raise")

    assert quote_literal("O'Brien") == "'O''Brien'"
    assert len(synthetic_names(10)) == 10
    assert len(set(synthetic_names(10))) == 10, "names must be distinct or charts collapse"

    sql = seed_sql(["O'Brien Ann", "李小明"])
    assert "'O''Brien Ann'" in sql, "a quote in a name must be escaped, not dropped"
    assert "李小明" in sql
    assert sql.count("uuidv7()") == 2, "one chart per name"

    table = format_table(
        [
            Timing("wu", 1479.0, 16),
            Timing("fitzherbert-brockholes", 6000.0, 3),
            Timing("李小", 15.0, 0),
        ]
    )
    assert "**OVER**" in table, "a search past the budget must be marked, not merely listed"
    assert "1479.0 ms" in table
    assert "NO ROWS" in table, "a search that found nothing must say so in the table itself"
    assert "**1479.0 ms**" in table.split("Floor")[1] or "1479.0" in table.split("Floor")[1], (
        "the floor must be the cheapest search THAT FOUND A CHART — a 15 ms empty search "
        "must not supply the figure quoted against §5.11"
    )
    assert measure_sql("wu") == (
        "SELECT count(*) FROM cairn_search_candidates("
        "ARRAY['wu']::text[], NULL, '[]'::jsonb)"
    ), "the rig must time cairn_search_candidates itself, not something adjacent to it"

    print("self-test: ok")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=5532)
    parser.add_argument(
        "--dbname",
        default="cairn_test",
        help=(
            "database to seed and measure. The default is the shared DB-gated test database, "
            "which every `cargo test` suite TRUNCATEs — point this somewhere else if anything "
            "else might run against the cluster during the measurement. A corpus that changes "
            "mid-run is caught and refused, not silently measured."
        ),
    )
    parser.add_argument("--user", default=None)
    parser.add_argument("--rows", type=int, default=50000, help="patient_name rows to seed")
    parser.add_argument("--repeats", type=int, default=5, help="timed runs per query (median)")
    parser.add_argument("--name-pool", default=None, help="SQLite database of real names")
    parser.add_argument(
        "--names-file", default=None, help="plain text file of names, one per line"
    )
    parser.add_argument(
        "--dump-names",
        default=None,
        help="write the drawn names to this file and exit, for carrying to the target machine",
    )
    parser.add_argument(
        "--queries",
        default=",".join(DEFAULT_QUERIES),
        help="comma-separated tokens to time; defaults to slice 1's five gestures",
    )
    parser.add_argument(
        "--keep",
        action="store_true",
        help="leave the seeded rows behind (for EXPLAIN by hand afterwards)",
    )
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()

    if args.self_test:
        return self_test()

    # An EMPTY --host means the local Unix socket, which is how a Debian-packaged cluster lets
    # the `postgres` system user in without a password. That is the shortest path to measuring
    # ON the target machine, which is the only place the budget means anything.
    conn = ["-p", str(args.port), "-d", args.dbname]
    if args.host:
        conn = ["-h", args.host, *conn]
    if args.user:
        conn += ["-U", args.user]

    if args.names_file:
        names = file_names(args.names_file, args.rows)
    elif args.name_pool:
        names = pool_names(args.name_pool, args.rows)
    else:
        names = synthetic_names(args.rows)

    if args.dump_names:
        with open(args.dump_names, "w", encoding="utf-8") as fh:
            fh.write("\n".join(names) + "\n")
        print(f"wrote {len(names)} names to {args.dump_names}")
        return 0

    print(f"seeding {len(names)} patient_name rows into {args.dbname}…", flush=True)
    psql(conn, "DELETE FROM patient_name WHERE asserted_origin = 'measure'")
    # Chunked so one statement does not grow past what a Pi will happily parse.
    for i in range(0, len(names), 5000):
        psql(conn, seed_sql(names[i : i + 5000]))

    # ASSERT THE POPULATION, do not merely print it. This used to be a bare `print` of
    # `count(*)` over the whole table, referenced by nothing — so a short seed, a
    # partially-failed chunk, or foreign rows left behind by an earlier `cargo test` all
    # produced a number nobody compared to anything, followed by a budget table that read
    # exactly the same as a good run. A budget measured against an unknown population is not
    # a budget. Checked in BOTH directions: too few rows understates the cost, too many
    # overstates it, and both are wrong.
    population = int(count_population(conn))
    if population != len(names):
        raise SystemExit(
            f"patient_name holds {population} rows, expected exactly {len(names)}. "
            "Either the seed did not complete, or rows from another run are still present "
            "(this rig only ever removes its own `asserted_origin = 'measure'` rows). "
            "Nothing measured against this table would be a measurement of the stated "
            "population."
        )
    print(f"patient_name holds {population} rows, as seeded\n", flush=True)

    per_sample_overhead = overhead_ms(conn, args.repeats)
    # `.strip()` each token: `--queries "mich, smi"` is the natural way to type a list, and an
    # unstripped " smi" is a DIFFERENT gesture — pass 3 compares against `lower(normalize(t,
    # NFC))` with no trim, so it would match nothing and (before the zero-row rule above) time
    # as the fastest row in the table.
    queries = [q.strip() for q in args.queries.split(",") if q.strip()]
    try:
        timings = [measure(conn, q, args.repeats) for q in queries]
    except BaseException:
        # Leave no corpus behind on a failed run. Without this, a psql error on sample 3 of 5
        # aborts with 50,000 rows still in a SHARED database: the next `cargo test` suite that
        # does not list `patient_name` in its own truncation set then runs against a polluted
        # projection, and the next rig run cleans up silently — so the residue is invisible to
        # whoever caused it.
        if not args.keep:
            psql(conn, "DELETE FROM patient_name WHERE asserted_origin = 'measure'")
        raise

    # RE-ASSERT THE POPULATION AFTER TIMING. `--dbname` defaults to `cairn_test`, the same
    # database every DB-gated suite TRUNCATEs and serialises on via `db::test_serial_guard`;
    # this rig takes no such lock. A `cargo test` starting mid-run empties `patient_name`, the
    # remaining samples return in milliseconds, and the table prints figures BETTER than the
    # truth — the most dangerous shape, because nothing about them invites suspicion. This
    # catches that, and every other way the corpus could move under the measurement.
    after = int(count_population(conn))
    if after != population:
        raise SystemExit(
            f"patient_name held {population} rows before timing and {after} after: the corpus "
            "changed UNDER the measurement, so these timings describe no known population. "
            "A concurrent `cargo test` against the same database is the usual cause — re-run "
            "with --dbname pointing somewhere nothing else writes."
        )

    print(format_table(timings))
    print(
        f"\nPer-sample overhead (process spawn + connect + auth, `SELECT 1` through the same "
        f"path): **{per_sample_overhead} ms**. Every figure above includes it."
    )

    if not args.keep:
        psql(conn, "DELETE FROM patient_name WHERE asserted_origin = 'measure'")

    return 0 if all(t.within_budget for t in timings) else 1


if __name__ == "__main__":
    raise SystemExit(main())
