#!/usr/bin/env python3
"""Pure-function tests for `scripts/measure_patient_search.py`.

The rig's job is to produce a number that goes into a paper-parity budget, so the parts that
can lie quietly — the median, the SQL escaping, the table's over-budget marking — are tested
without a database, in CI, next to `measure_dr_restore_test.py`.

What is deliberately NOT tested here: anything that needs a cluster. A test that silently
skips when Postgres is absent is how a rig rots; the DB-touching half is exercised by being
run, and its results are recorded in the plan that cites them.

    python3 scripts/tests/measure_patient_search_test.py
"""

from __future__ import annotations

import importlib.util
import pathlib
import sqlite3
import sys
import tempfile
import unicodedata

RIG = pathlib.Path(__file__).resolve().parents[1] / "measure_patient_search.py"


def load_rig():
    """Import the rig by path — it is a script, not an installed module."""
    spec = importlib.util.spec_from_file_location("measure_patient_search", RIG)
    module = importlib.util.module_from_spec(spec)
    # Registered before execution so `@dataclass` can resolve the module it is defined in.
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    return module


def test_self_test_passes(m) -> None:
    """The rig's own `--self-test` is the first assertion, so CI runs it too."""
    assert m.self_test() == 0


def test_a_name_with_a_quote_survives_seeding(m) -> None:
    """`O'Brien` is a name, not a syntax error, and dropping it would skew a distribution."""
    sql = m.seed_sql(["O'Brien Ann"])
    assert "'O''Brien Ann'" in sql


def test_seeding_gives_every_name_its_own_chart(m) -> None:
    """Pass 3 returns DISTINCT patient ids: sharing one id would collapse the result set and
    flatter every timing."""
    sql = m.seed_sql(["Ann Smith", "Bo Ng", "Cy Wu"])
    assert sql.count("uuidv7()") == 3


def test_names_are_stored_nfc_normalised(m) -> None:
    """Not because a stored value must be composed — the door writes the asserted string
    verbatim, decomposed or not, which is why `patient_search_equivalence.rs` seeds a
    decomposed chart on purpose — but because the fixture should vary only what is being
    measured. Pass 3 normalises on read either way, so a mixed-form pool would add a
    difference that is not the one the before/after comparison is about."""
    decomposed = unicodedata.normalize("NFD", "Jos\u00e9")
    assert "\u0301" in decomposed, "the fixture must really be decomposed"
    sql = m.seed_sql([decomposed])
    assert "Jos\u00e9" in sql
    assert "\u0301" not in sql


def test_a_search_past_the_budget_is_marked_not_merely_listed(m) -> None:
    over = m.format_table([m.Timing("x", m.BUDGET_MS + 1, 3)])
    under = m.format_table([m.Timing("x", m.BUDGET_MS - 1, 3)])
    assert "**OVER**" in over
    assert "**OVER**" not in under


def test_a_search_exactly_at_the_budget_is_within_it(m) -> None:
    """The boundary the `<=` in `within_budget` actually draws. Tested because every other
    case here is `BUDGET_MS ± 1`, which leaves the one value the comparison is about
    unpinned."""
    assert m.Timing("x", m.BUDGET_MS, 1).within_budget
    assert not m.Timing("x", m.BUDGET_MS + 0.1, 1).within_budget


def test_a_search_that_found_nothing_is_not_a_measurement_of_the_budget(m) -> None:
    """§1.2's budget is **5 s to FIND AN EXISTING CHART**. A gesture matching zero rows
    measures how fast pass 3 comes back empty — a different number, and a much smaller one.

    The rig shipped with `within_budget` consulting `median_ms` alone, so such a gesture
    passed AND could become the `floor` quoted against §5.11's harder "no spinner" limb. That
    is not hypothetical: the run this rig was written for timed `李小` (a CJK prefix) against
    a pool of real Australian names."""
    empty = m.Timing("李小", 15.0, 0)
    assert not empty.found_a_chart
    assert not empty.within_budget, "a search that found nothing cannot have met the budget"

    table = m.format_table([m.Timing("wu", 1479.0, 16), empty])
    assert "NO ROWS" in table, "the table itself must say so — plans paste the table, not this file"
    # The 15 ms empty search must not supply the floor.
    assert "1479.0 ms" in table.split("Floor")[1]
    assert "15.0 ms" not in table.split("Floor")[1]


def test_a_set_in_which_nothing_was_found_reports_no_floor_at_all(m) -> None:
    """The degenerate case: if no gesture found a chart there is no §5.11 figure to quote,
    and the rig must say that rather than quote the cheapest empty search."""
    table = m.format_table([m.Timing("zzz", 12.0, 0), m.Timing("qqq", 14.0, 0)])
    assert "no floor to report" in table


def test_the_rig_times_the_search_function_itself(m) -> None:
    """A rig that silently times the WRONG statement is the worst silent-wrong-measurement
    failure, because every number it produces is internally consistent."""
    sql = m.measure_sql("O'Brien")
    assert "cairn_search_candidates(" in sql
    assert "'O''Brien'" in sql, "the query token must be escaped, not dropped"
    assert "count(*)" in sql


def test_the_floor_is_reported_against_the_spinner_limb(m) -> None:
    """§5.11's limb is about the CHEAPEST search, not the worst — the distinction #637's
    headline missed and its own correction restored."""
    table = m.format_table([m.Timing("slow", 3000.0, 1), m.Timing("fast", 900.0, 1)])
    assert "900.0 ms" in table.split("Floor")[1]
    assert "within" in table.split("Floor")[1]

    table = m.format_table([m.Timing("slow", 3000.0, 1), m.Timing("fast", 1500.0, 1)])
    assert "above" in table.split("Floor")[1]


def test_a_names_file_shorter_than_the_run_is_a_loud_error(m) -> None:
    """Quietly measuring 900 rows and labelling it 50,000 is the failure mode this rig exists
    to stop — a number is worth nothing if the population under it is unverified."""
    with tempfile.NamedTemporaryFile("w", suffix=".txt", delete=False) as fh:
        fh.write("Ann Smith\nBo Ng\n")
        path = fh.name
    try:
        m.file_names(path, 2)  # exactly enough is fine
        try:
            m.file_names(path, 50)
        except SystemExit:
            pass
        else:
            raise AssertionError("a short names file must abort, not truncate the run")
    finally:
        pathlib.Path(path).unlink()


def _pool(tmpdir: str, rows: list[tuple[str | None, str]], table: str = "names") -> str:
    """A minimal SQLite name pool with the column names the rig's heuristic looks for."""
    path = str(pathlib.Path(tmpdir) / "pool.sqlite3")
    con = sqlite3.connect(path)
    con.execute(f'CREATE TABLE "{table}" (firstname TEXT, surname TEXT)')
    con.executemany(f'INSERT INTO "{table}" VALUES (?, ?)', rows)
    con.commit()
    con.close()
    return path


def test_a_name_pool_shorter_than_the_run_is_a_loud_error(m) -> None:
    """The same guard `file_names` has always carried, on the path that lacked it.

    `LIMIT ?` returns fewer rows than asked for without complaint, so this path used to seed
    whatever it got, time five searches against it, and print a budget verdict. It is not a
    theoretical concern: the maintainer's own 959 MB pool holds TWO tables matching the rig's
    given-name + surname heuristic — `names` (6.5M rows) and `person` (10) — resolved today
    only by `sqlite_master` order."""
    with tempfile.TemporaryDirectory() as tmp:
        path = _pool(tmp, [("Ann", "Smith"), ("Bo", "Ng")])
        assert len(m.pool_names(path, 2)) == 2  # exactly enough is fine
        try:
            m.pool_names(path, 50)
        except SystemExit:
            pass
        else:
            raise AssertionError("a short name pool must abort, not truncate the run")


def test_a_pool_row_with_no_given_name_never_becomes_a_patient_called_none(m) -> None:
    """`f"{g} {f}"` with a NULL given name renders the literal string `None`, injecting a
    bogus high-frequency token across the whole pool. The maintainer's current pool happens to
    store empty strings rather than NULLs, so this is latent there — which is exactly why it
    is pinned before the next pool arrives."""
    with tempfile.TemporaryDirectory() as tmp:
        path = _pool(tmp, [(None, "Smith"), ("Ann", "Ng"), ("", "Wu")])
        names = m.pool_names(path, 2)
        assert not any(n.startswith("None") for n in names), names


def test_a_pool_whose_other_tables_are_oddly_named_still_yields_names(m) -> None:
    """The identifier guard must SKIP a table it cannot safely name, not abort the run on it.
    Raising here killed the measurement before the loop ever reached the names table."""
    with tempfile.TemporaryDirectory() as tmp:
        path = _pool(tmp, [("Ann", "Smith"), ("Bo", "Ng")])
        con = sqlite3.connect(path)
        con.execute('CREATE TABLE "odd-name 2024" (firstname TEXT, surname TEXT)')
        con.commit()
        con.close()
        assert len(m.pool_names(path, 2)) == 2


def test_the_default_draw_is_the_head_unchanged(m) -> None:
    """The latency rig's published figures depend on WHICH rows it draws, so the default draw is
    pinned as it always was: the first usable rows, a blank given name kept as a one-word name.
    Review of PR #678 changed this by accident; this file's CI step caught it."""
    with tempfile.TemporaryDirectory() as tmp:
        path = _pool(tmp, [("", "Wu"), ("Ann", "Ng"), ("Bo", "Li"), ("Cy", "Ho")])
        assert m.pool_names(path, 2) == ["Wu", "Ann Ng"]


def test_a_spread_draw_skips_blank_names(m) -> None:
    """The prompt rig models full names; a blank given name or surname (empty or whitespace)
    would quietly model a one-word name it does not claim to (review of PR #678)."""
    with tempfile.TemporaryDirectory() as tmp:
        path = _pool(tmp, [("", "Wu"), ("  ", "Li"), ("Ann", " "), ("Ann", "Ng"), ("Bo", "Ho")])
        assert sorted(m.pool_names(path, 2, spread=True)) == ["Ann Ng", "Bo Ho"]


def test_a_spread_draw_reaches_across_the_whole_table(m) -> None:
    """The head of the maintainer's pool holds ~4x its share of common surnames, so a
    representative draw must come from the whole table, not its first rows (review of PR #678)."""
    with tempfile.TemporaryDirectory() as tmp:
        path = _pool(tmp, [(f"G{i}", f"S{i}") for i in range(100)])
        names = m.pool_names(path, 10, spread=True)
        assert len(names) == 10, names
        assert "G90 S90" in names, names


def test_a_spread_draw_works_on_a_view(m) -> None:
    """Discovery also picks VIEWs, which have no `rowid`; the spread draw must not read it."""
    with tempfile.TemporaryDirectory() as tmp:
        path = _pool(tmp, [(f"G{i}", f"S{i}") for i in range(20)], table="raw")
        con = sqlite3.connect(path)
        con.execute("DROP TABLE IF EXISTS names")
        con.execute('ALTER TABLE raw RENAME COLUMN firstname TO a')
        con.execute("CREATE VIEW people AS SELECT a AS firstname, surname FROM raw")
        con.commit()
        con.close()
        assert len(m.pool_names(path, 5, spread=True)) == 5


def main() -> int:
    m = load_rig()
    tests = [v for k, v in sorted(globals().items()) if k.startswith("test_")]
    # A runner that collects tests by NAME PREFIX must say how many it expected to find:
    # rename the prefix, or move these into a class, and an empty list prints "0 passed" and
    # exits 0 — a green CI step asserting nothing at all.
    if len(tests) < 12:
        raise AssertionError(
            f"collected only {len(tests)} tests; this file has more than that. The collector "
            "matches names starting `test_`, so a rename can silently empty it."
        )
    for t in tests:
        t(m)
        print(f"ok  {t.__name__}")
    print(f"\n{len(tests)} passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
