# matcher/tests/test_db_gate_actually_ran.py
"""#451 — the matcher's DB-gated suite cannot go silently green either.

# The hole, one language over

#442 closed this for the Rust suite (``crates/cairn-node/tests/db_gate_actually_ran.rs``).
The Python side kept it unchanged: the DB-gated files under ``matcher/tests/`` self-skip when
``CAIRN_TEST_PG`` is unset — most through ``conftest.py``'s ``pg_conn`` fixture, one through a
direct ``pytest.skip`` — and **pytest reports a skip as a pass**. The variable reaches the suite
from exactly one step-level ``env:`` block, ``pytest (matcher DB-gated integration suite)`` in
``.github/workflows/rust.yml``. A typo in that key, a step split, or a job copied without its
``env:`` skips the whole DB tier at once — and the run comes back *greener than usual* rather
than red, which is the worst possible signal: it looks like an improvement.

(No count of files is given on purpose: the first cut said "fifteen", inherited unchecked from
the Rust header, and it was fourteen by two different mechanisms. A number that rots on every
added suite buys nothing this sentence needs.)

# What is derived, and from what

The variable names come from the **matcher's own sources**, never from a list written here — a
guard defined over the list it guards is not a guard (#387). So a future ``CAIRN_TEST_PG4`` is
covered the moment the first test reads it, with no edit to this file.

The scan reads **code, not prose** (the #449 lesson, applied here from the start rather than
after the fact): a name counts only inside an ``os.environ.get("…")`` / ``os.getenv("…")`` /
``os.environ["…"]`` expression, and ``#`` comment lines are dropped first. The bait is already
in the tree — ``test_conftest_lifecycle.py`` discusses ``CAIRN_TEST_PG`` in a docstring, and a
prefix scan would have turned every such sentence into an environment requirement.

Three honest limits. A full expression quoted verbatim inside a *docstring* is still read as a
call site; so is one in a TRAILING ``#`` comment after code on the same line, since
``_without_comment_lines`` only blanks whole-line comments (the Rust guard has the same limit,
and the first cut of this file claimed "the same two" while stating a different two); and a name
assembled at runtime has no literal to read. The ``_GATE_VARS_TODAY`` floor is what notices the
scan going quiet.

# Polarity: it fails CLOSED

Binding on ``$CI`` was rejected for the reason #450 gives: nothing in this repo sets ``CI``, it
is inherited from the runner, and a scrubbed environment would silently disable the guard. This
binds by default and takes the **same opt-out variable as the Rust guard**, so there is one rule
and not two: ``CAIRN_ALLOW_DB_SKIP=1``.

``matcher.yml``'s ``lint-test`` job runs the pure suite *deliberately* without a database, so it
declares that opt-out in its own ``env:`` block — the fail-closed shape working as intended: the
run that means to skip the DB tier says so at the site that means it, and every other run is
bound.
"""

import os
import re
from pathlib import Path

import pytest

# Directory names never worth walking: virtualenvs, caches, build output.
_SKIP_DIRS = {".venv", "venv", "__pycache__", ".pytest_cache", ".ruff_cache", "target", ".git"}

# The `matcher/` tree: this file lives at matcher/tests/, so its grandparent is the root.
_MATCHER_ROOT = Path(__file__).resolve().parents[1]

# This file writes fixture expressions naming variables nothing reads; without the exclusion
# they would become requirements the environment has to satisfy.
_THIS_FILE = Path(__file__).resolve()

# How many gate variables the scan must find, as a LIVENESS check on the scan rather than a
# definition of the set. Today the matcher reads exactly one (`CAIRN_TEST_PG`, in
# tests/conftest.py and in src/cairn_matcher/eval/__main__.py). Without this floor, a scan that
# silently found nothing — a moved directory, a changed idiom — would pass for the same reason a
# correctly-configured run passes.
_GATE_VARS_TODAY = 1

# The opt-out, shared with the Rust guard so the repo has one rule. Deliberately NOT prefixed
# `CAIRN_TEST_`: it is not a gate variable, and prefixing it would make the scan demand that the
# opt-out itself be set.
_OPT_OUT = "CAIRN_ALLOW_DB_SKIP"

# `os.environ.get("NAME")` / `os.getenv("NAME")`, and the subscript form `os.environ["NAME"]`.
# Written as two expressions rather than one alternation so each stays readable and each names
# its own capture group. `monkeypatch.setenv(...)` matches neither, which is correct — a test
# MUTATING the environment is not a read of it.
_CALL_READ = re.compile(r"""os\.(?:environ\.get|getenv)\s*\(\s*(['"])(CAIRN_TEST_[A-Z0-9_]*)\1""")
_ITEM_READ = re.compile(r"""os\.environ\s*\[\s*(['"])(CAIRN_TEST_[A-Z0-9_]*)\1""")


def _without_comment_lines(text: str) -> str:
    """Blank out whole-line ``#`` comments, keeping line structure.

    Crude on purpose, and the same shape as the Rust guard's: a line whose first non-whitespace
    character is ``#`` is prose. Docstrings are NOT stripped — that limit is stated in the module
    docstring rather than papered over, because stripping them would need a real parser.
    """
    return "\n".join("" if line.lstrip().startswith("#") else line for line in text.splitlines())


def gate_var_names_in(text: str) -> set[str]:
    """Every ``CAIRN_TEST_*`` name read from the environment by ``text``.

    Pure function over source text, so the parser can be pinned by fixtures below rather than
    only by whatever the tree happens to contain on a given day.

    A trailing-underscore or bare-prefix name (``CAIRN_TEST_``) is rejected: the regex requires
    at least the prefix, and the emptiness check below drops the prefix-only case, so a prose
    mention of the family name cannot invent a variable.
    """
    scanned = _without_comment_lines(text)
    found = set()
    for pattern in (_CALL_READ, _ITEM_READ):
        for _quote, name in pattern.findall(scanned):
            if name != "CAIRN_TEST_":
                found.add(name)
    return found


def matcher_sources(root: Path | None = None) -> list[Path]:
    """Every ``.py`` file under ``root`` (default ``matcher/``), minus caches and virtualenvs.

    **Loud on an unreadable directory — which is the one thing ``rglob`` is not.** The first cut
    of this function used ``Path.rglob`` and claimed in this very docstring that it raised. It
    does not: CPython suppresses ``OSError`` inside the recursive selector, so an unreadable
    subdirectory yields a silently SHORT list and the guard still passes. That is precisely the
    #452 defect the three Rust walks were unified to remove, reproduced here while citing #452
    as the authority for the opposite — reassurance without earning it, in the file written to
    close exactly that. ``os.scandir`` raises; ``test_an_unreadable_directory_is_loud`` pins it.

    Symlinks are neither descended into nor collected (``follow_symlinks=False``), matching
    ``crates/cairn-node/tests/common/sources.rs``'s ``DirEntry::file_type`` semantics. A symlink
    to an ancestor would otherwise make this walk unbounded — a hang in a required check, which
    presents as flakiness rather than as a defect.

    ``root`` is a parameter rather than a hardcoded global so the tests below can drive it over
    a fixture tree without monkeypatching module state (house rule 4: pure and reusable beats
    clever). Production callers pass nothing.

    Sorted, so a failure names files in the same order on every machine.
    """
    out: list[Path] = []
    stack = [root if root is not None else _MATCHER_ROOT]
    while stack:
        # `os.scandir` raises on an unreadable directory instead of yielding nothing, and the
        # context manager closes the directory handle even when it does.
        with os.scandir(stack.pop()) as entries:
            for entry in entries:
                path = Path(entry.path)
                if entry.is_dir(follow_symlinks=False):
                    if entry.name not in _SKIP_DIRS:
                        stack.append(path)
                elif entry.is_file(follow_symlinks=False) and path.suffix == ".py":
                    out.append(path)
    return sorted(out)


def gate_vars_read_by_the_suite() -> tuple[set[str], int]:
    """The variables the matcher actually reads, and how many times this file excluded itself."""
    found: set[str] = set()
    self_exclusions = 0
    for path in matcher_sources():
        if path == _THIS_FILE:
            self_exclusions += 1
            continue
        found |= gate_var_names_in(path.read_text(encoding="utf-8"))
    return found, self_exclusions


def db_skip_is_allowed() -> bool:
    """May this run skip the database tier?

    **Only an explicit affirmative opts out.** An unrecognised value must not be read as
    permission, or ``CAIRN_ALLOW_DB_SKIP=please`` (or ``=false``, or ``=0``) silently restores
    the fail-open behaviour this guard exists to remove. Same rule, same spelling, as the Rust
    guard's ``db_skip_is_allowed``.
    """
    return os.environ.get(_OPT_OUT, "").strip().lower() in {"1", "true", "yes", "on"}


def is_usefully_set(name: str) -> bool:
    """Is ``name`` set to something a connection string could be built from?

    An *empty* value counts as missing. GitHub Actions resolves an undefined expression —
    ``CAIRN_TEST_PG: ${{ env.TYPO }}`` — to the empty string rather than to nothing, so the key
    is present, the read returns ``""``, and a naive ``is None`` check passes while the suite
    skips. That is the same species of typo this guard is about, one layer lower down.
    """
    return bool(os.environ.get(name, "").strip())


def test_the_db_gated_suite_actually_ran():
    """Every gate variable the matcher reads must be set, unless this run declared otherwise."""
    variables, self_exclusions = gate_vars_read_by_the_suite()

    assert self_exclusions == 1, (
        f"expected to exclude exactly this file from the scan, excluded {self_exclusions} — "
        "the path comparison has stopped identifying it, so its own fixture expressions are "
        "now feeding the requirement list (#451)."
    )

    assert len(variables) >= _GATE_VARS_TODAY, (
        f"the CAIRN_TEST_* scan found {len(variables)} variable(s), fewer than the "
        f"{_GATE_VARS_TODAY} this suite is known to read (#451). Either the scan has gone "
        "stale — a moved directory, or a reading idiom the os.environ matcher no longer "
        "recognises — in which case it would now pass without checking anything, or a gate "
        f"variable was deliberately retired, in which case lower the floor in the same "
        f"commit. Found: {sorted(variables)}"
    )

    if db_skip_is_allowed():
        return

    missing = sorted(v for v in variables if not is_usefully_set(v))
    assert not missing, (
        f"these DB-gate variables are unset or empty: {missing}\n\n"
        "The DB-gated tests under matcher/tests/ self-skip without them, AND PYTEST REPORTS A "
        "SKIP AS A PASS, so the whole integration suite would have looked green while proving "
        "nothing (#451).\n\n"
        "· In CI: set them in the step's `env:` block — see 'pytest (matcher DB-gated "
        "integration suite)' in .github/workflows/rust.yml.\n"
        "· Locally with PostgreSQL 18 + cairn_pgx: CAIRN_TEST_PG='host=... dbname=cairn_test' "
        "uv run --extra pipeline pytest\n"
        f"· Locally WITHOUT a database: export {_OPT_OUT}=1 to declare that this run skips the "
        "database tier (see CONTRIBUTING.md). The guard fails closed on purpose — an absent "
        "opt-out is not permission (#450)."
    )


# ─── Fixture tests: the parser is pinned over synthetic source, not over the tree ────────────
#
# ANTI-VACUITY. The scan above runs against whatever `matcher/` happens to contain, so on any
# given day it could pass while recognising very little. These pin the properties that matter
# over strings written here, so they fail if the parser regresses regardless of the real tree.


def test_prose_does_not_invent_a_gate_variable():
    """A name mentioned in a comment or a docstring sentence is not a read."""
    commentary = """
# TODO: CAIRN_TEST_PG4 once the fourth cluster lands.
    # see CAIRN_TEST_PG5
"""
    assert gate_var_names_in(commentary) == set()

    # The live bait already in the tree: test_conftest_lifecycle.py's docstring discusses the
    # old module-level shape. It names the function but has no string literal to read.
    prose = "The old shape was a module-level `CAIRN_TEST_PG6 = os.environ.get(...)`."
    assert gate_var_names_in(prose) == set()

    # A bare name in an ordinary string — a skip message, an error — is likewise inert.
    assert gate_var_names_in('pytest.skip("CAIRN_TEST_PG7 not set")') == set()

    # A COMMENTED-OUT EXPRESSION — the only case here that actually pins
    # `_without_comment_lines`. The three above contain no `os.environ` read shape at all, so
    # they are rejected by the regexes and would still pass with the comment stripper deleted
    # outright; the PR #456 review measured exactly that in the Rust twin. This one fails
    # without the stripper.
    commented_out = """
# dsn = os.environ.get("CAIRN_TEST_PG8")
    #third = os.environ['CAIRN_TEST_PG9']
"""
    assert gate_var_names_in(commented_out) == set(), (
        "a read that has been COMMENTED OUT is not a read — this is what the whole-line "
        "comment stripper is for (#449)"
    )


def test_a_real_read_is_found():
    """Every reading idiom the tree uses, plus the subscript form, is picked up."""
    source = """
        dsn = os.environ.get("CAIRN_TEST_PG")
        alt = os.getenv('CAIRN_TEST_PG2')
        third = os.environ["CAIRN_TEST_PG3"]
        spaced = os.environ.get( "CAIRN_TEST_PG9" )
    """
    assert gate_var_names_in(source) == {
        "CAIRN_TEST_PG",
        "CAIRN_TEST_PG2",
        "CAIRN_TEST_PG3",
        "CAIRN_TEST_PG9",
    }


def test_near_misses_are_rejected():
    """Non-gate variables, env MUTATION, and runtime-assembled names contribute nothing."""
    cases = [
        # Not a gate variable — including this guard's own opt-out, which must never become a
        # requirement.
        'os.environ.get("CAIRN_ALLOW_DB_SKIP")',
        'os.environ.get("CI")',
        # Setting or clearing is not reading. Several tests do exactly this.
        'monkeypatch.setenv("CAIRN_TEST_PG4", "host=example")',
        'monkeypatch.delenv("CAIRN_TEST_PG5", raising=False)',
        # The prefix alone names nothing.
        'os.environ.get("CAIRN_TEST_")',
        # No literal to read: assembled at runtime.
        'os.environ.get(f"CAIRN_TEST_PG{n}")',
        # A left-boundary near-miss: a DIFFERENT variable that merely contains the prefix.
        'os.environ.get("OLD_CAIRN_TEST_PG4")',
    ]
    for case in cases:
        assert gate_var_names_in(case) == set(), f"must contribute no variable: {case}"


def test_only_an_explicit_affirmative_opts_out(monkeypatch):
    """#450's polarity, pinned: an unrecognised value is not permission."""
    for yes in ["1", "true", "TRUE", " yes ", "on"]:
        monkeypatch.setenv(_OPT_OUT, yes)
        assert db_skip_is_allowed(), f"{yes!r} must opt out"

    for no in ["", "0", "false", "no", "off", "please", "  "]:
        monkeypatch.setenv(_OPT_OUT, no)
        assert not db_skip_is_allowed(), f"{no!r} must NOT be read as permission"

    monkeypatch.delenv(_OPT_OUT, raising=False)
    assert not db_skip_is_allowed(), "an absent opt-out must bind the guard"


def test_an_empty_value_counts_as_missing(monkeypatch):
    """The GitHub-Actions empty-string defence, pinned (#456 review).

    ``is_usefully_set`` is the only thing standing between a mistyped ``env:`` key and a silent
    skip, and it had no fixture: relaxing it to ``is not None`` passed everywhere, because CI
    sets the variable non-empty and nothing ever supplied an empty one.
    """
    name = "CAIRN_TEST_PG_FIXTURE"
    monkeypatch.setenv(name, "host=127.0.0.1 dbname=cairn_test")
    assert is_usefully_set(name)

    # `CAIRN_TEST_PG: ${{ env.TYPO }}` resolves to "" — the key is PRESENT and the read
    # succeeds, which is exactly how a typo skips the tier while looking configured.
    for empty in ["", "   ", "\t\n"]:
        monkeypatch.setenv(name, empty)
        assert not is_usefully_set(name), f"{empty!r} must count as missing"

    monkeypatch.delenv(name, raising=False)
    assert not is_usefully_set(name), "an unset variable is missing"


def test_an_unreadable_directory_is_loud(tmp_path):
    """A directory the walk cannot read must RAISE, not silently shorten the list (#452).

    The counterpart of the Rust walk's ``a_missing_root_is_loud``, and the test the ``rglob``
    implementation could not pass — it returned a short list and no exception, so a guard that
    examined fewer files than it thought reported the same green as one that examined them all.
    """
    if os.geteuid() == 0:
        pytest.skip("root ignores directory permissions, so there is nothing to be loud about")

    (tmp_path / "readable.py").write_text("x = 1\n", encoding="utf-8")
    blocked = tmp_path / "blocked"
    blocked.mkdir()
    (blocked / "hidden.py").write_text("y = 2\n", encoding="utf-8")
    blocked.chmod(0o000)
    try:
        with pytest.raises(OSError):
            matcher_sources(tmp_path)
    finally:
        # Restore before the fixture teardown, which would otherwise fail to remove the tree.
        blocked.chmod(0o755)


def test_a_missing_root_is_loud(tmp_path):
    """A root that does not exist raises rather than returning an empty list."""
    with pytest.raises(OSError):
        matcher_sources(tmp_path / "no-such-directory")


def test_symlinks_are_neither_followed_nor_collected(tmp_path):
    """The #452 symlink property, pinned in Python as it is in Rust.

    A symlinked DIRECTORY is not descended into and a symlinked FILE is not collected. The
    directory case is the load-bearing one: a symlink to an ancestor makes a following walk
    unbounded — a hang in a required check, which presents as flakiness rather than as a defect.

    Asserted through a symlink to a SIBLING rather than to an ancestor deliberately: a following
    walk then terminates and fails on a named marker file, instead of hanging CI to prove a
    point.
    """
    (tmp_path / "real.py").write_text("x = 1\n", encoding="utf-8")
    elsewhere = tmp_path / "elsewhere"
    elsewhere.mkdir()
    (elsewhere / "marker.py").write_text("y = 2\n", encoding="utf-8")

    walked = tmp_path / "walked"
    walked.mkdir()
    (walked / "link_to_dir").symlink_to(elsewhere, target_is_directory=True)
    (walked / "link_to_file.py").symlink_to(tmp_path / "real.py")

    found = {p.name for p in matcher_sources(walked)}
    assert found == set(), (
        f"a symlinked directory must not be descended into and a symlinked file must not be "
        f"collected; found {sorted(found)}"
    )


def test_the_scan_finds_the_matchers_real_reads():
    """Both real call sites in the tree are found — the derivation is not vacuous.

    Named rather than counted: a count cannot tell "found both" from "found one twice". The
    first cut made that argument and then did not follow it — it asserted the two FILES were
    walked and that one variable was somewhere in the union, which ``conftest.py`` alone
    satisfies, since both sites read the same name (#456 review). Each file is now parsed on
    its own.
    """
    by_name = {p.name: p for p in matcher_sources()}
    assert "conftest.py" in by_name, "the walk must reach matcher/tests/"
    assert "__main__.py" in by_name, "the walk must reach matcher/src/"

    for name in ("conftest.py", "__main__.py"):
        text = by_name[name].read_text(encoding="utf-8")
        assert "CAIRN_TEST_PG" in gate_var_names_in(text), (
            f"{name} reads CAIRN_TEST_PG and the parser must see it THERE — asserting only "
            "over the union lets one site cover for the other going silent"
        )

    variables, _ = gate_vars_read_by_the_suite()
    assert "CAIRN_TEST_PG" in variables
