"""Every matcher module except the named few imports WITHOUT psycopg installed.

Why this exists: CI's `matcher` job runs the pure suite with no `pipeline` extra, so psycopg is
absent there — but every local run uses `uv run --extra pipeline pytest`, so psycopg is always
present on a developer's machine. A module that drags psycopg in at import time therefore passes
every local gate and fails only in CI, at test COLLECTION (R4, PR #724: `targeted.py` imported
three SQL strings from `pipeline/db.py`, and `worker` → `watch` → `cli` all inherited it).

The convention it pins (see `pipeline/db.py`'s docstring and `test_alias_pipeline.py`): `db.py`
is the only psycopg-touching module; everything else imports it lazily, inside the function that
needs a connection. This test does not depend on the environment: it runs a fresh interpreter
in which `import psycopg` is made to fail, then imports every module of the package.
"""

import json
import subprocess
import sys

# Modules that MAY need psycopg at import time, each with its reason. Keep this list short:
# a new entry means the pure CI suite can no longer collect any test that imports that module.
# Each entry must still really need psycopg (checked below) — when one stops, delete it.
NEEDS_PSYCOPG = {
    "cairn_matcher.pipeline.db": "the one psycopg-touching module, by design",
    "cairn_matcher.eval.measure_check": "a measurement tool that only runs against a database",
}

# Runs in the child interpreter. Setting sys.modules["psycopg"] = None makes every later
# `import psycopg` (and `from psycopg.rows import …`) raise ImportError, exactly as if the
# package were not installed. `__main__` modules are skipped: importing one runs its program.
_PROBE = """
import importlib, json, pkgutil, sys
sys.modules["psycopg"] = None
import cairn_matcher
failed = {}
for info in pkgutil.walk_packages(cairn_matcher.__path__, "cairn_matcher."):
    if info.name.endswith(".__main__"):
        continue
    try:
        importlib.import_module(info.name)
    except ImportError as e:
        failed[info.name] = str(e)
print(json.dumps(failed))
"""


def _modules_failing_without_psycopg() -> dict[str, str]:
    """Import every package module in a psycopg-less child; return {module: error}."""
    out = subprocess.run(
        [sys.executable, "-c", _PROBE], capture_output=True, text=True, check=True
    )
    return json.loads(out.stdout)


def test_only_the_named_modules_need_psycopg_to_import():
    failed = _modules_failing_without_psycopg()
    unexpected = {m: e for m, e in failed.items() if m not in NEEDS_PSYCOPG}
    assert not unexpected, (
        "these modules now import psycopg at import time, so CI's pure suite (no `pipeline` "
        f"extra) cannot collect any test importing them — import it lazily instead: {unexpected}"
    )


def test_every_allowed_module_still_needs_psycopg():
    failed = _modules_failing_without_psycopg()
    stale = sorted(set(NEEDS_PSYCOPG) - set(failed))
    assert not stale, f"no longer needs psycopg — delete it from NEEDS_PSYCOPG: {stale}"
