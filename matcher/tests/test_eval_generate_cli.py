"""Tests for the generator CLI edge (python -m cairn_matcher.eval.generate)."""

import json

from cairn_matcher.eval.generate import main
from cairn_matcher.eval.loader import load_dataset_file


def test_cli_writes_a_loadable_dataset_file(tmp_path):
    out = tmp_path / "synthetic.json"
    rc = main(["--entities", "20", "--seed", "9", "--out", str(out)])
    assert rc == 0
    ds = load_dataset_file(out)                 # must parse via the real loader
    assert len(ds.entities) == 20


def test_cli_is_deterministic_for_same_seed(tmp_path):
    a, b = tmp_path / "a.json", tmp_path / "b.json"
    main(["--entities", "15", "--seed", "5", "--out", str(a)])
    main(["--entities", "15", "--seed", "5", "--out", str(b)])
    assert a.read_text() == b.read_text()


def test_cli_writes_to_stdout_when_no_out(capsys):
    rc = main(["--entities", "3", "--seed", "1"])
    assert rc == 0
    payload = json.loads(capsys.readouterr().out)
    assert len(payload["entities"]) == 3
