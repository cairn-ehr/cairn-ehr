"""Smoke test: the package imports and exposes a version. Proves the uv project runs."""

import cairn_matcher


def test_package_exposes_version():
    assert isinstance(cairn_matcher.__version__, str)
    assert cairn_matcher.__version__


def test_the_package_metadata_and_the_module_carry_one_version():
    # matcher_version (the proposal pin, and the trigger for a worker's full re-check) reads
    # __version__; pyproject.toml is what the package is published and locked as. A bump that
    # moves one and not the other leaves the two disagreeing about which matcher ran (#775 review).
    import tomllib
    from pathlib import Path

    pyproject = tomllib.loads(
        (Path(__file__).resolve().parents[1] / "pyproject.toml").read_text(encoding="utf-8")
    )
    assert pyproject["project"]["version"] == cairn_matcher.__version__
