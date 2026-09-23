#!/usr/bin/env python3
"""How often does the funnel's step-3 prompt truncate — and does ranking keep the duplicate on it?

# The question (funnel UI slice 2c, 2026-09-23)

The registration window's step-3 prompt shows at most `PROMPT_CAP` (5) candidates, and a new
chart's birth act permanently attests exactly those rows as displayed (ADR-0061). The design
assumed a full-name-plus-DOB search "returns few candidates by construction", and said: *if the
prompt is routinely incomplete, the cap is wrong and the design needs revisiting.*

`db/046` is a DISJUNCTION of three passes (identifier / exact DOB / any name token), so that
assumption deserved a measurement rather than an argument. This rig takes it, and it also
measures the fix slice 2c made — `search_patients` ranking candidates by passes matched instead
of by chart age — by asking one question per sampled patient:

    A clerk is about to register someone who is ALREADY on file, typing their full name and
    date of birth. Is that existing chart among the five the prompt shows?

Each sampled patient plays that existing chart; the query is built from its own stored name and
date of birth, exactly as the window builds one (`FormSnapshot::query`).

# What it measures, and what it does not

**The candidate SET and its ORDER, not timings.** Rows go straight into the two projections
`cairn_search_candidates` reads (`patient_name`, and `patient_demographic`'s dob row), as
`measure_patient_search.py` does for names, so this says nothing about write throughput. Ids
are assigned in insertion order, so "id order" really is chart-creation order — the order
`search_patients` used before slice 2c. The population is shuffled first, so a sampled patient's
chart age is random.

**Ranking is recomputed here, not read from `search_patients`.** `rank()` is the Python twin of
`rank_by_passes_matched` (crates/cairn-node/src/patient/search.rs), pinned against it by the
self-test on the same example the Rust unit test uses. The pass counts themselves come from the
real `cairn_search_candidates`.

# Usage

    uv run --no-project python scripts/measure_prompt_truncation.py --self-test
    uv run --no-project python scripts/measure_prompt_truncation.py --dbname cairn_test \\
        --rows 50000 --name-pool ~/src/SyntheticHealthData/synthetic_demographics.sqlite3

Requires `psql` and a cluster with the schema loaded; reuses `measure_patient_search.py`'s
`psql`/`scalar`/`pool_names` helpers rather than a second copy of them.
"""

from __future__ import annotations

import argparse
import json
import os
import random
import statistics
import sys
import unicodedata
import uuid

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from measure_patient_search import pool_names, psql, quote_literal, scalar  # noqa: E402

# The prompt's cap. Mirrors cairn_gui_funnel::PROMPT_CAP; a measurement of "the cap" must use
# the cap the window uses.
PROMPT_CAP = 5

# Rows this rig writes carry this origin, so cleanup can never touch anything else.
ORIGIN = "measure-prompt"

# A realistic-SHAPED synthetic population, for runs without the maintainer's pool: common given
# names and surnames with a skew, so shared tokens occur at believable rates. Deterministic.
GIVEN = [
    "James", "Mary", "John", "Patricia", "Robert", "Jennifer", "Michael", "Linda", "William",
    "Elizabeth", "David", "Barbara", "Richard", "Susan", "Joseph", "Jessica", "Thomas", "Sarah",
    "Charles", "Karen", "Wei", "Ling", "Mohammed", "Fatima", "Nguyen", "Anh", "Priya", "Raj",
    "Olivia", "Jack", "Noah", "Charlotte", "Liam", "Amelia", "Ava", "Oliver", "Isla", "Leo",
]
SURNAMES = [
    "Smith", "Jones", "Williams", "Brown", "Wilson", "Taylor", "Johnson", "White", "Martin",
    "Anderson", "Thompson", "Nguyen", "Thomas", "Walker", "Harris", "Lee", "Ryan", "Robinson",
    "Kelly", "King", "Davis", "Wright", "Evans", "Roberts", "Green", "Hall", "Wood", "Jackson",
    "Clarke", "Patel", "Khan", "Chen", "Wang", "Li", "Zhang", "Singh", "Kaur", "Tran", "Le",
    "Pham", "Murphy", "O'Brien", "Campbell", "Scott", "Mitchell", "Young", "Turner", "Baker",
]


def synthetic_population(count: int, rng: random.Random) -> list[str]:
    """`count` names drawn with a Zipf-like skew (weight 1/rank) from the lists above."""
    gw = [1 / (i + 1) for i in range(len(GIVEN))]
    sw = [1 / (i + 1) for i in range(len(SURNAMES))]
    return [f"{rng.choices(GIVEN, gw)[0]} {rng.choices(SURNAMES, sw)[0]}" for _ in range(count)]


def query_tokens(raw_name: str) -> list[str]:
    """The Python twin of `cairn_patient_search::SearchQuery::new`'s name tokeniser.

    Per whitespace-delimited word: the whole word with edge punctuation trimmed, lowercased;
    plus its alphanumeric parts longer than one character, lowercased. Sorted and deduplicated.
    """
    tokens: set[str] = set()
    for word in raw_name.split():
        start, end = 0, len(word)
        while start < end and not word[start].isalnum():
            start += 1
        while end > start and not word[end - 1].isalnum():
            end -= 1
        whole = word[start:end].lower()
        if whole:
            tokens.add(whole)
        part = ""
        for ch in word + " ":
            if ch.isalnum():
                part += ch
            else:
                if len(part) > 1:
                    tokens.add(part.lower())
                part = ""
    return sorted(tokens)


def rank(rows: list[tuple[str, int]]) -> list[str]:
    """The Python twin of `rank_by_passes_matched`: passes matched DESC, then id ASC."""
    return [pid for pid, _ in sorted(rows, key=lambda r: (-r[1], r[0]))]


def perturb_dob(dob: str) -> str:
    """The date of birth a registrar mis-hears or mis-types: day and month swapped when that is
    still a different valid date, otherwise the year off by one.

    The exact-duplicate arm answers "is the chart shown when everything was typed right?". This
    arm answers the harder question the funnel exists for (final review #7): the existing chart
    now shares only the NAME pass, ties with everyone else sharing a name token, and falls back to
    chart-age order within that tie.
    """
    year, month, day = (int(x) for x in dob.split("-"))
    if day <= 12 and day != month:
        return f"{year:04d}-{day:02d}-{month:02d}"
    return f"{year + 1:04d}-{month:02d}-{day:02d}"


def position(order: list[str], pid: str) -> int:
    """1-based position of `pid` in `order`."""
    return order.index(pid) + 1


def summarise(results: list[dict], cap: int) -> dict[str, object]:
    """Reduce per-search results to the figures the result file reports.

    Each result is `{"self": id, "rows": [(id, passes), ...]}` — the candidates one step-3
    search returned, with how many passes each matched.
    """
    counts = [len(r["rows"]) for r in results]
    by_id = [position(sorted(pid for pid, _ in r["rows"]), r["self"]) for r in results]
    ranked = [position(rank(r["rows"]), r["self"]) for r in results]
    strong = [sum(1 for _, p in r["rows"] if p >= 2) for r in results]
    return {
        "searches": len(results),
        "candidates_median": statistics.median(counts),
        "candidates_p90": sorted(counts)[int(0.9 * (len(counts) - 1))],
        "candidates_max": max(counts),
        "truncated": sum(1 for c in counts if c > cap),
        "self_in_cap_by_id": sum(1 for p in by_id if p <= cap),
        "self_in_cap_ranked": sum(1 for p in ranked if p <= cap),
        "self_rank_by_id_median": statistics.median(by_id),
        "self_rank_ranked_median": statistics.median(ranked),
        # Searches where MORE than `cap` candidates matched two or more passes: the prompt
        # truncates among strong matches too, and the self-match can then be cut on id order.
        "strong_over_cap": sum(1 for s in strong if s > cap),
    }


def seed_sql(people: list[tuple[str, str, str]]) -> list[str]:
    """INSERTs for (id, name, dob) triples: one `patient_name` and one dob row each."""
    names = ",\n".join(
        f"('{pid}', 'legal', {quote_literal(unicodedata.normalize('NFC', name))}, "
        f"'patient-stated', 1, 0, 0, '{ORIGIN}', clock_timestamp())"
        for pid, name, _ in people
    )
    dobs = ",\n".join(
        f"('{pid}', 'dob', '{dob}', 'patient-stated', 1, 0, 0, '{ORIGIN}')"
        for pid, _, dob in people
    )
    return [
        "INSERT INTO patient_name (patient_id, use_key, value, provenance, provenance_rank, "
        "last_hlc_wall, last_hlc_count, asserted_origin, updated_at) VALUES\n" + names + ";",
        "INSERT INTO patient_demographic (patient_id, field, value, provenance, provenance_rank, "
        "asserted_hlc_wall, asserted_hlc_count, asserted_origin) VALUES\n" + dobs + ";",
    ]


def batch_query_sql(samples: list[tuple[str, str, str]]) -> str:
    """ONE statement running every sampled step-3 search, returning (sample, id, passes) rows."""
    values = ",\n".join(
        "({i}, ARRAY[{toks}]::text[], '{dob}')".format(
            i=i, toks=",".join(quote_literal(t) for t in query_tokens(name)), dob=dob
        )
        for i, (_, name, dob) in enumerate(samples)
    )
    return (
        "SELECT s.i, c.patient_id::text, count(DISTINCT c.matched_pass) "
        "FROM (VALUES\n" + values + ") AS s(i, toks, dob) "
        "CROSS JOIN LATERAL cairn_search_candidates(s.toks, s.dob, '[]'::jsonb) c "
        "GROUP BY s.i, c.patient_id"
    )


def cleanup(conn: list[str]) -> None:
    psql(conn, f"DELETE FROM patient_name WHERE asserted_origin = '{ORIGIN}'")
    psql(conn, f"DELETE FROM patient_demographic WHERE asserted_origin = '{ORIGIN}'")


def self_test() -> int:
    """The pure functions. No database."""
    # Twin of SearchQuery::new: interior punctuation stays in the whole-word token.
    assert query_tokens("O'Brien-Smith, John") == ["brien", "john", "o'brien-smith", "smith"], (
        query_tokens("O'Brien-Smith, John")
    )
    assert query_tokens("  Wu   Ling ") == ["ling", "wu"]
    # Twin of rank_by_passes_matched, on the Rust unit test's own example.
    assert rank([("1", 1), ("3", 1), ("2", 2)]) == ["2", "1", "3"]
    assert rank([("b", 1), ("a", 1), ("c", 2)]) == ["c", "a", "b"]
    s = summarise(
        [
            {"self": "x", "rows": [("a", 1), ("b", 1), ("x", 2)]},
            {"self": "y", "rows": [("y", 2)]},
        ],
        cap=2,
    )
    assert s["searches"] == 2
    assert s["truncated"] == 1, s
    assert s["self_in_cap_by_id"] == 1, s  # x is 3rd by id; y is 1st
    assert s["self_in_cap_ranked"] == 2, s
    assert s["strong_over_cap"] == 0, s
    sql = seed_sql([("00000000-0000-0000-0000-000000000001", "O'Brien Ann", "1980")])
    assert "'O''Brien Ann'" in sql[0] and "'1980'" in sql[1]
    assert "ARRAY['ann','brien','o''brien']" in batch_query_sql([("x", "O'Brien Ann", "1980")])
    # The perturbed-DOB arm: the realistic imperfect duplicate (final review #7).
    assert perturb_dob("1980-03-07") == "1980-07-03", "day <= 12: swap day and month"
    assert perturb_dob("1980-03-03") == "1981-03-03", "day == month: a swap changes nothing"
    assert perturb_dob("1980-03-20") == "1981-03-20", "day > 12: the swap is not a date"
    print("self-test: ok")
    return 0


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--host", default="127.0.0.1")
    ap.add_argument("--port", type=int, default=5532)
    ap.add_argument("--dbname", default="cairn_test")
    ap.add_argument("--user", default=None)
    ap.add_argument("--rows", type=int, default=50000, help="population (spec §8.1: ~50,000)")
    ap.add_argument("--samples", type=int, default=500, help="step-3 searches to run")
    ap.add_argument("--name-pool", default=None, help="SQLite pool of real names")
    ap.add_argument("--seed", type=int, default=20260923)
    ap.add_argument(
        "--perturb",
        choices=["none", "dob"],
        default="none",
        help="'dob': query each sampled patient with a mis-typed date of birth (see perturb_dob)",
    )
    ap.add_argument("--self-test", action="store_true")
    args = ap.parse_args()
    if args.self_test:
        return self_test()

    conn = ["-p", str(args.port), "-d", args.dbname]
    if args.host:
        conn = ["-h", args.host, *conn]
    if args.user:
        conn += ["-U", args.user]

    rng = random.Random(args.seed)
    names = pool_names(args.name_pool, args.rows) if args.name_pool else synthetic_population(args.rows, rng)
    rng.shuffle(names)
    base = 0x0190_0000_0000_7000_8000_0000_0000_0000
    people = []
    for i, name in enumerate(names):
        # Ascending ids in insertion order: id order IS chart-creation order here.
        pid = str(uuid.UUID(int=base + i))
        year = rng.randint(1930, 2025)
        dob = f"{year:04d}-{rng.randint(1, 12):02d}-{rng.randint(1, 28):02d}"
        people.append((pid, name, dob))

    cleanup(conn)
    # Chunked, as measure_patient_search.py does: one 50,000-row statement passed to `psql -c`
    # would exceed the OS argument-size limit (1 MiB on macOS).
    for i in range(0, len(people), 5000):
        for sql in seed_sql(people[i : i + 5000]):
            psql(conn, sql)
    before = scalar(conn, f"SELECT count(*) FROM patient_name WHERE asserted_origin = '{ORIGIN}'")
    try:
        samples = rng.sample(people, args.samples)
        queried = (
            [(pid, name, perturb_dob(dob)) for pid, name, dob in samples]
            if args.perturb == "dob"
            else samples
        )
        out = psql(conn, batch_query_sql(queried))
        after = scalar(conn, f"SELECT count(*) FROM patient_name WHERE asserted_origin = '{ORIGIN}'")
        if before != after or int(before) != len(people):
            raise SystemExit(f"population changed mid-run ({before} -> {after}); refusing to report")
        rows: dict[int, list[tuple[str, int]]] = {i: [] for i in range(len(samples))}
        for line in out.splitlines():
            i, pid, passes = line.split("|")
            rows[int(i)].append((pid, int(passes)))
        results = [{"self": samples[i][0], "rows": rows[i]} for i in range(len(samples))]
        missing = [r for r in results if r["self"] not in {pid for pid, _ in r["rows"]}]
        if missing:
            raise SystemExit(f"{len(missing)} searches did not find their own chart; the rig is wrong")
        summary = summarise(results, PROMPT_CAP)
        summary.update(
            population=len(people),
            pool=args.name_pool or "synthetic (Zipf-skewed common names)",
            perturb=args.perturb,
            cap=PROMPT_CAP,
        )
        print(json.dumps(summary, indent=2))
    finally:
        cleanup(conn)
    return 0


if __name__ == "__main__":
    sys.exit(main())
