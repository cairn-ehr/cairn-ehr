"""Smoke tests for the weight-learning CLI (python -m cairn_matcher.eval.learn)."""

from cairn_matcher.eval.learn import main
from cairn_matcher.eval.model_io import read_model


def test_cli_runs_on_bundled_gold_and_prints_before_after(capsys):
    rc = main(["--folds", "5"])
    assert rc == 0
    out = capsys.readouterr().out
    assert "BEFORE" in out and "AFTER" in out


def test_cli_writes_a_loadable_artifact(tmp_path):
    path = tmp_path / "model.json"
    rc = main(["--folds", "5", "--out", str(path)])
    assert rc == 0
    model = read_model(path)  # reloads without error
    assert model.thresholds.auto > model.thresholds.review


def test_cli_reports_a_bad_dataset_path_gracefully(capsys):
    rc = main(["/no/such/dataset.json"])
    assert rc == 2
    assert "error" in capsys.readouterr().err.lower()


def test_cli_rejects_a_bad_knob_gracefully(capsys):
    rc = main(["--margin", "0"])
    assert rc == 2
    assert "error" in capsys.readouterr().err.lower()
