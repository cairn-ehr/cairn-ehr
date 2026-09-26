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

With `--perturb dob` the query instead carries a MIS-TYPED date of birth (`perturb_dob`: day
and month swapped, or the year off by one) — the realistic imperfect duplicate, for which the
DOB pass no longer matches and ranking has only the name to go on.

# What it measures, and what it does not

**The candidate SET and its ORDER, not timings.** Rows go straight into the two projections
`cairn_search_candidates` reads (`patient_name`, and `patient_demographic`'s dob row), as
`measure_patient_search.py` does for names, so this says nothing about write throughput. Ids
are assigned in insertion order, so "id order" really is chart-creation order — the order
`search_patients` used before slice 2c. The population is shuffled first, so a sampled patient's
chart age is random.

With `--perturb name` the query carries a TYPO in the last name (`perturb_name`) — the commonest
real duplicate — and `--perturb both` a typo AND a mis-typed date of birth, so the chart is left
with only its given name to be found by. Recorded, not optimised: ADR-0075 hands what no prompt
can show to the §5.2 matcher and the link-repair path.

**Ranking is recomputed here, not read from `search_patients`.** `rank()` is the Python twin of
`cairn_patient_search::rank_candidates` (ADR-0075: passes, then name tokens matched, then DOB
near-miss, then id), and `rank_by_passes()` of slice 2c's order, so one run reports before and
after. Both are pinned by the self-test on the Rust unit tests' own examples. The pass counts
themselves come from the real `cairn_search_candidates`.

# Usage

    uv run --no-project python scripts/measure_prompt_truncation.py --self-test
    uv run --no-project python scripts/measure_prompt_truncation.py --dbname cairn_test \\
        --rows 50000 --name-pool ~/src/SyntheticHealthData/synthetic_demographics.sqlite3
    # the same, with each query's date of birth mis-typed:
    uv run --no-project python scripts/measure_prompt_truncation.py --dbname cairn_test \\
        --rows 50000 --perturb dob

Requires `psql` and a cluster with the schema loaded; reuses `measure_patient_search.py`'s
`psql`/`scalar`/`pool_names`/`quote_literal` helpers rather than a second copy of them.
"""

from __future__ import annotations

import argparse
import calendar
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

    Known drift, rig-only: Python's `str.isalnum` excludes combining vowel signs (Mn/Mc) that
    Rust's `char::is_alphanumeric` includes (e.g. Devanagari), so such names would tokenise
    differently here. The current name pool is Latin-script; revisit before using another.

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


# A candidate row: (id, passes matched, name tokens matched, DOB near-miss).
Row = tuple[str, int, int, bool]


def rank_by_passes(rows: list[Row]) -> list[str]:
    """Slice 2c's order, kept so one run reports before AND after: passes DESC, then id ASC."""
    return [r[0] for r in sorted(rows, key=lambda r: (-r[1], r[0]))]


def rank(rows: list[Row]) -> list[str]:
    """The Python twin of `cairn_patient_search::rank_candidates` (ADR-0075): passes DESC, then
    name tokens matched DESC, then DOB near-miss first, then id ASC."""
    return [r[0] for r in sorted(rows, key=lambda r: (-r[1], -r[2], not r[3], r[0]))]


def tokens_matched(query: list[str], stored_names: list[str]) -> int:
    """Twin of `cairn_patient_search::tokens_matched`: DISTINCT PLAIN (all-alphanumeric) query
    tokens found among the tokens of any stored name, each stored name tokenised by the query's
    own rule (`query_tokens`) after the NFC + lowercase normalisation Postgres applies in
    `search_rank.rs`. Plain only, so a hyphenated word's whole form and parts count once each."""
    stored: set[str] = set()
    for name in stored_names:
        stored.update(query_tokens(unicodedata.normalize("NFC", name).lower()))
    return sum(1 for t in set(query) if t.isalnum() and t in stored)


def parse_ymd(value: str) -> tuple[int, int, int] | None:
    """Twin of `cairn_patient_search::candidate::parse_ymd`: a full, REAL ISO date or None."""
    parts = value.split("-")
    if len(parts) != 3:
        return None
    try:
        y, m, d = (int(x) for x in parts)
    except ValueError:
        return None
    if not 1 <= m <= 12 or not 1 <= d <= calendar.monthrange(y, m)[1]:
        return None
    return y, m, d


def is_dob_near_miss(query: str, candidate: str) -> bool:
    """Twin of `cairn_patient_search::is_dob_near_miss`: day/month swapped, year +-1, or the
    year's last two digits transposed. Partial or impossible dates, and exact matches, never."""
    q, c = parse_ymd(query), parse_ymd(candidate)
    if q is None or c is None or q == c:
        return False
    (qy, qm, qd), (cy, cm, cd) = q, c
    same_day_month = (cm, cd) == (qm, qd)
    swapped = cy == qy and cm == qd and cd == qm
    transposed = (
        qy != cy and qy // 100 == cy // 100
        and (qy % 100) // 10 == cy % 10 and qy % 10 == (cy % 100) // 10
    )
    return swapped or (same_day_month and abs(cy - qy) == 1) or (same_day_month and transposed)


def perturb_dob_any(dob: str, rng: random.Random) -> str:
    """A date of birth that is simply WRONG — any other valid date, 1930-2025 — rather than one
    of the slips `is_dob_near_miss` knows. The control for the near-miss key: the `dob` arm uses
    exactly the slips that key rewards, so it cannot say what happens outside them.
    """
    while True:
        other = f"{rng.randint(1930, 2025):04d}-{rng.randint(1, 12):02d}-{rng.randint(1, 28):02d}"
        if other != dob:
            return other


def perturb_name(name: str, rng: random.Random) -> str:
    """The commonest real duplicate (maintainer, #671): a TYPO in a hard-to-spell name. One
    interior character of the last word becomes a different lowercase letter, length kept.

    `db/046` matches whole tokens, so the misspelt token no longer matches; the chart is still
    found through its OTHER keys (the given name, an exact DOB). The arms record how often it
    is shown; what no prompt can show — every token misspelt AND the date wrong — is the §5.2
    matcher's and the link-repair path's (ADR-0075 decision 2).
    """
    words = name.split()
    last = words[-1]
    if len(last) < 3:
        return name
    i = rng.randrange(1, len(last) - 1)
    replacement = rng.choice([ch for ch in "abcdefghijklmnopqrstuvwxyz" if ch != last[i].lower()])
    words[-1] = last[:i] + replacement + last[i + 1 :]
    return " ".join(words)


def perturb_dob(dob: str) -> str:
    """The date of birth a registrar mis-hears or mis-types: day and month swapped when that is
    still a different valid date, otherwise the year off by one.

    The exact-duplicate arm answers "is the chart shown when everything was typed right?". This
    arm answers the harder question the funnel exists for (PR #674 review #7): the existing chart
    now shares only the NAME pass, ties with everyone else sharing a name token, and falls back to
    chart-age order within that tie.
    """
    year, month, day = (int(x) for x in dob.split("-"))
    if day <= 12 and day != month:
        return f"{year:04d}-{day:02d}-{month:02d}"
    return f"{year + 1:04d}-{month:02d}-{day:02d}"


def position(order: list[str], pid: str) -> int:
    """1-based position of `pid` in `order`, or a sentinel past any cap when it is absent (the
    name-typo arm, where the search usually does not find the chart at all)."""
    return order.index(pid) + 1 if pid in order else 10**9


def summarise(results: list[dict], cap: int) -> dict[str, object]:
    """Reduce per-search results to the figures the result file reports.

    Each result is `{"self": id, "rows": [Row, ...]}` — the candidates one step-3 search
    returned, each with its passes, name tokens matched and DOB near-miss.
    """
    counts = [len(r["rows"]) for r in results]
    found = [r for r in results if r["self"] in {row[0] for row in r["rows"]}]
    by_id = [position(sorted(row[0] for row in r["rows"]), r["self"]) for r in results]
    by_passes = [position(rank_by_passes(r["rows"]), r["self"]) for r in results]
    ranked = [position(rank(r["rows"]), r["self"]) for r in results]
    strong = [sum(1 for row in r["rows"] if row[1] >= 2) for r in results]

    def median_found(positions: list[int]) -> object:
        inside = [p for p in positions if p < 10**9]
        return statistics.median(inside) if inside else None

    return {
        "searches": len(results),
        "candidates_median": statistics.median(counts),
        "candidates_p90": sorted(counts)[int(0.9 * (len(counts) - 1))],
        "candidates_max": max(counts),
        "truncated": sum(1 for c in counts if c > cap),
        "self_in_candidate_set": len(found),
        "self_in_cap_by_id": sum(1 for p in by_id if p <= cap),
        "self_in_cap_passes_only": sum(1 for p in by_passes if p <= cap),
        "self_in_cap_ranked": sum(1 for p in ranked if p <= cap),
        "self_rank_passes_only_median": median_found(by_passes),
        "self_rank_ranked_median": median_found(ranked),
        # Searches where MORE than `cap` candidates matched two or more passes.
        "strong_over_cap": sum(1 for x in strong if x > cap),
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
    # Twin of the 2c order (passes, then id) — kept to report before/after in one run.
    assert rank_by_passes([("1", 1, 0, False), ("3", 1, 0, False), ("2", 2, 0, False)]) == ["2", "1", "3"]
    # Twin of cairn_patient_search::rank (the Rust unit tests' own examples).
    assert tokens_matched(["john", "smith"], ["john smith"]) == 2
    assert tokens_matched(["john", "smith"], ["john brown"]) == 1
    assert tokens_matched(["john", "john"], ["john smith"]) == 1
    assert tokens_matched(["john"], []) == 0
    q = query_tokens("Mary-Jane Smith")
    assert tokens_matched(q, ["Mary Jane Smith"]) > tokens_matched(q, ["Mary-Jane Brown"])
    assert is_dob_near_miss("1980-03-07", "1980-07-03")
    assert is_dob_near_miss("1980-03-07", "1979-03-07")
    assert is_dob_near_miss("1967-05-20", "1976-05-20")
    assert not is_dob_near_miss("1967-05-20", "1977-05-20")
    assert not is_dob_near_miss("1980-03-07", "1980-03-07")
    assert not is_dob_near_miss("1980", "1981")
    assert not is_dob_near_miss("1980-02-30", "1980-30-02")
    assert not is_dob_near_miss("1980-03-07", "1982-03-07")
    assert rank([("1", 1, 2, True), ("2", 2, 0, False)]) == ["2", "1"]
    assert rank([("1", 1, 1, False), ("2", 1, 2, False)]) == ["2", "1"]
    assert rank([("1", 1, 2, False), ("2", 1, 2, True)]) == ["2", "1"]
    assert rank([("20", 1, 1, False), ("10", 1, 1, False)]) == ["10", "20"]
    # The name-typo arm: exactly one character of the LAST word changes, deterministically.
    t = perturb_name("John Smith", random.Random(1))
    assert t.split()[0] == "John" and t != "John Smith" and len(t) == len("John Smith"), t
    assert perturb_name("John Smith", random.Random(1)) == t, "deterministic under a seed"
    assert perturb_dob_any("1980-03-07", random.Random(1)) != "1980-03-07"
    s = summarise(
        [
            {"self": "x", "rows": [("a", 1, 2, False), ("b", 1, 0, False), ("x", 2, 1, False)]},
            {"self": "y", "rows": [("y", 2, 2, False)]},
        ],
        cap=2,
    )
    assert s["searches"] == 2
    assert s["truncated"] == 1, s
    assert s["self_in_cap_by_id"] == 1, s  # x is 3rd by id; y is 1st
    assert s["self_in_cap_passes_only"] == 2, s
    assert s["self_in_cap_ranked"] == 2, s
    assert s["self_in_candidate_set"] == 2, s
    lost = summarise([{"self": "z", "rows": [("a", 1, 1, False)]}], cap=2)
    assert lost["self_in_candidate_set"] == 0 and lost["self_in_cap_ranked"] == 0, lost
    assert s["strong_over_cap"] == 0, s
    sql = seed_sql([("00000000-0000-0000-0000-000000000001", "O'Brien Ann", "1980")])
    assert "'O''Brien Ann'" in sql[0] and "'1980'" in sql[1]
    assert "ARRAY['ann','brien','o''brien']" in batch_query_sql([("x", "O'Brien Ann", "1980")])
    # The perturbed-DOB arm: the realistic imperfect duplicate (PR #674 review #7).
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
        choices=["none", "dob", "dob-any", "name", "both", "both-any"],
        default="none",
        help="'dob': a mis-typed date of birth (perturb_dob); 'name': a typo in the last name "
        "(perturb_name); 'both': the two slips at once; '*-any': the date simply wrong "
        "(perturb_dob_any), the near-miss key's control",
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
    # Seeding sits INSIDE the try, so a failure part-way through it is cleaned up too rather than
    # leaving up to `--rows` rows under ORIGIN until the next run's up-front cleanup.
    try:
        # Chunked, as measure_patient_search.py does: one 50,000-row statement passed to `psql -c`
        # would exceed the OS argument-size limit (1 MiB on macOS).
        for i in range(0, len(people), 5000):
            for sql in seed_sql(people[i : i + 5000]):
                psql(conn, sql)
        count_sql = f"SELECT count(*) FROM patient_name WHERE asserted_origin = '{ORIGIN}'"
        before = scalar(conn, count_sql)
        samples = rng.sample(people, args.samples)
        if args.perturb == "dob":
            queried = [(pid, name, perturb_dob(dob)) for pid, name, dob in samples]
        elif args.perturb == "name":
            queried = [(pid, perturb_name(name, rng), dob) for pid, name, dob in samples]
        elif args.perturb == "both":
            queried = [(pid, perturb_name(name, rng), perturb_dob(dob)) for pid, name, dob in samples]
        elif args.perturb == "dob-any":
            queried = [(pid, name, perturb_dob_any(dob, rng)) for pid, name, dob in samples]
        elif args.perturb == "both-any":
            queried = [
                (pid, perturb_name(name, rng), perturb_dob_any(dob, rng)) for pid, name, dob in samples
            ]
        else:
            queried = samples
        out = psql(conn, batch_query_sql(queried))
        after = scalar(conn, count_sql)
        if before != after or int(before) != len(people):
            raise SystemExit(f"population changed mid-run ({before} -> {after}); refusing to report")
        # The ranking keys are computed from the population this rig seeded, exactly as
        # `search_rank.rs` computes them from `patient_name` / `patient_demographic`.
        name_of = {pid: name for pid, name, _ in people}
        dob_of = {pid: dob for pid, _, dob in people}
        rows: dict[int, list[Row]] = {i: [] for i in range(len(samples))}
        for line in out.splitlines():
            i, pid, passes = line.split("|")
            _, q_name, q_dob = queried[int(i)]
            rows[int(i)].append(
                (
                    pid,
                    int(passes),
                    tokens_matched(query_tokens(q_name), [name_of[pid]]),
                    is_dob_near_miss(q_dob, dob_of[pid]),
                )
            )
        results = [{"self": samples[i][0], "rows": rows[i]} for i in range(len(samples))]
        missing = [r for r in results if r["self"] not in {row[0] for row in r["rows"]}]
        # A typo'd name usually leaves the candidate set entirely: that is a RESULT of the name
        # arm (reported as self_in_candidate_set), and a rig defect for every other arm.
        if missing and args.perturb not in ("name", "both", "both-any"):
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
