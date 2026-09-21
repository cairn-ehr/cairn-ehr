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
import sys
import unicodedata
import tempfile

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
    over = m.format_table([m.Timing("x", m.BUDGET_MS + 1, 0)])
    under = m.format_table([m.Timing("x", m.BUDGET_MS - 1, 0)])
    assert "**OVER**" in over
    assert "**OVER**" not in under


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


def main() -> int:
    m = load_rig()
    tests = [v for k, v in sorted(globals().items()) if k.startswith("test_")]
    for t in tests:
        t(m)
        print(f"ok  {t.__name__}")
    print(f"\n{len(tests)} passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
