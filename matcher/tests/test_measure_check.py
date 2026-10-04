"""R4 Task 7 fix F4: the measurement reports the population it actually seeded.

The generator builds entities in twos, so asking for an odd size seeds one record fewer; the
printed N must be the real record count, or a reader compares figures for populations that
were never measured.
"""

from cairn_matcher.eval import measure_check


def test_the_printed_size_is_the_seeded_record_count(pg_conn, capsys):
    assert measure_check.main(["--sizes", "21", "--sample", "3"]) == 0
    line = capsys.readouterr().out.strip()
    assert line.startswith("N=    20 "), line          # 21 asked, 20 records seeded
