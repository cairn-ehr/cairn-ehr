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
    def within_budget(self) -> bool:
        return self.median_ms <= BUDGET_MS


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


def pool_names(sqlite_path: str, count: int) -> list[str]:
    """Draw `count` names from a SQLite pool of real names.

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
        for table in map(plain_identifier, tables):
            cols = [r[1].lower() for r in con.execute(f'PRAGMA table_info("{table}")')]
            given = next((c for c in cols if c in ("firstname", "given_name", "given", "first_name")), None)
            family = next((c for c in cols if c in ("surname", "family_name", "last_name", "lastname")), None)
            if given and family:
                given, family = plain_identifier(given), plain_identifier(family)
                rows = con.execute(
                    f'SELECT "{given}", "{family}" FROM "{table}" '
                    f"WHERE \"{family}\" IS NOT NULL AND \"{family}\" <> '' LIMIT ?",
                    (count,),
                ).fetchall()
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
    """The markdown table that goes into the plan's measurement section."""
    header = "| Search | Median | Rows found | Budget |\n|---|---|---|---|"
    rows = [
        "| `{q}` | **{ms} ms** | {n} | {verdict} |".format(
            q=t.query,
            ms=t.median_ms,
            n=t.rows,
            verdict="ok" if t.within_budget else "**OVER**",
        )
        for t in timings
    ]
    floor = min(t.median_ms for t in timings) if timings else 0.0
    note = (
        f"\n\nFloor (cheapest search of the set): **{floor} ms** — "
        f"{'above' if floor > SPINNER_MS else 'within'} the ~{SPINNER_MS:.0f} ms "
        "at which a wait stops reading as instant (§5.11)."
    )
    return "\n".join([header, *rows]) + note


def psql(conn: list[str], sql: str, quiet: bool = False) -> str:
    """Run one statement, returning stdout. Raises on a non-zero exit, loudly."""
    proc = subprocess.run(
        ["psql", *conn, "-v", "ON_ERROR_STOP=1", "-tAc", sql],
        capture_output=True,
        text=True,
    )
    if proc.returncode != 0:
        if not quiet:
            sys.stderr.write(proc.stderr)
        raise SystemExit(f"psql failed ({proc.returncode})")
    return proc.stdout.strip()


def measure(conn: list[str], query: str, repeats: int) -> Timing:
    """Time one search, discarding a warm-up run.

    The warm-up is not politeness: the first call after a schema load pays for plan caching
    and for pages the buffer cache has not seen, and a clerk's second search of the shift is
    the one the budget is about.
    """
    sql = (
        "SELECT count(*) FROM cairn_search_candidates("
        f"ARRAY[{quote_literal(query)}]::text[], NULL, '[]'::jsonb)"
    )
    psql(conn, sql)  # warm-up, not recorded
    samples = []
    rows = 0
    for _ in range(repeats):
        start = time.perf_counter()
        out = psql(conn, sql)
        samples.append((time.perf_counter() - start) * 1000.0)
        rows = int(out or 0)
    return Timing(query=query, median_ms=median_ms(samples), rows=rows)


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
            Timing("fitzherbert-brockholes", 6000.0, 0),
        ]
    )
    assert "**OVER**" in table, "a search past the budget must be marked, not merely listed"
    assert "1479.0 ms" in table
    assert "Floor" in table

    print("self-test: ok")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--host", default="127.0.0.1")
    parser.add_argument("--port", type=int, default=5532)
    parser.add_argument("--dbname", default="cairn_test")
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
    total = psql(conn, "SELECT count(*) FROM patient_name")
    print(f"patient_name now holds {total} rows\n", flush=True)

    timings = [measure(conn, q, args.repeats) for q in args.queries.split(",") if q]
    print(format_table(timings))

    if not args.keep:
        psql(conn, "DELETE FROM patient_name WHERE asserted_origin = 'measure'")

    return 0 if all(t.within_budget for t in timings) else 1


if __name__ == "__main__":
    raise SystemExit(main())
